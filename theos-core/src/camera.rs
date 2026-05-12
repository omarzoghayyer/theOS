// camera.rs -- theOS Camera Module
//
// Manages photo capture, encrypted storage, and AI-driven camera intents.
//
// Architecture:
//   The AI is the camera interface. Users say "take a photo" or
//   "take a photo and send it to Sarah" -- the AI parses the intent
//   and drives this module directly. No camera app needed.
//
// Hardware layer:
//   On device: V4L2 via /dev/video0 (OnePlus 6 main camera, SDM845 ISP)
//   In tests:  stub captures -- same interface, no hardware needed
//
// Storage:
//   All photos encrypted with the user Ed25519 keypair at rest.
//   Path format: /run/theos/photos/<timestamp>_<short_id>.enc
//   Photos shared only via theOS Messenger to accepted contacts.
//
// Security assumptions (flag for audit):
//   - Encrypted storage path tied to Ed25519 pubkey.
//     If keypair is lost, photos are unrecoverable.
//   - Photo signing uses Ed25519 -- proves photo came from this device.
//   - No cloud upload ever. Photos stay on device or are sent
//     directly to contacts via theOS Messenger E2E encrypted.

use crate::identity::keypair::{IdentityKey, KeyPair};
use sha2::{Sha256, Digest};
use std::collections::VecDeque;

const MAX_RECENT_CAPTURES: usize = 100;
const PHOTO_BASE_PATH: &str = "/run/theos/photos";

// -- CameraState --------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub enum CameraState {
    Unavailable,   // hardware not present or driver not loaded
    Idle,          // ready to capture
    Capturing,     // shutter open / sensor reading
    Processing,    // ISP processing, encoding
    Saved,         // photo written to encrypted storage
    Error(String), // capture failed
}

impl CameraState {
    pub fn is_ready(&self) -> bool {
        matches!(self, CameraState::Idle)
    }

    pub fn label(&self) -> &str {
        match self {
            CameraState::Unavailable   => "unavailable",
            CameraState::Idle          => "ready",
            CameraState::Capturing     => "capturing",
            CameraState::Processing    => "processing",
            CameraState::Saved         => "saved",
            CameraState::Error(_)      => "error",
        }
    }
}

// -- CameraLens ---------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub enum CameraLens {
    Main,      // rear main camera (Sony IMX519 on OnePlus 6)
    Telephoto, // rear secondary
    Front,     // selfie camera
}

impl CameraLens {
    pub fn v4l2_device(&self) -> &str {
        match self {
            CameraLens::Main      => "/dev/video0",
            CameraLens::Telephoto => "/dev/video1",
            CameraLens::Front     => "/dev/video2",
        }
    }
}

// -- CameraCapture ------------------------------------------------------------

/// A single captured photo record.
/// The actual image bytes live in encrypted storage at `storage_path`.
/// This struct is the metadata index entry.
#[derive(Debug, Clone)]
pub struct CameraCapture {
    pub id:           String,      // unique capture ID (hex short hash)
    pub timestamp:    u64,         // unix seconds
    pub lens:         CameraLens,
    pub storage_path: String,      // path to encrypted image file
    pub width:        u32,
    pub height:       u32,
    pub signature:    Vec<u8>,     // Ed25519 sig over (id + timestamp + path)
    pub sent_to:      Vec<String>, // contact key hexes this was sent to
}

impl CameraCapture {
    /// Create a new capture record.
    /// In production: keypair.sign() produces a real Ed25519 signature.
    pub fn new(
        lens:      CameraLens,
        width:     u32,
        height:    u32,
        owner_key: &IdentityKey,
        keypair:   &KeyPair,
    ) -> Self {
        let timestamp = now_secs();
        let id        = derive_capture_id(owner_key, timestamp);
        let path      = format!("{}/{}_{}.enc", PHOTO_BASE_PATH, timestamp, &id[..8]);

        // Sign the capture metadata to prove device origin
        let payload = signing_payload(&id, timestamp, &path);
        let signature = keypair.sign(&payload);

        Self {
            id,
            timestamp,
            lens,
            storage_path: path,
            width,
            height,
            signature,
            sent_to: Vec::new(),
        }
    }

    /// Verify this capture was signed by the given identity.
    pub fn verify(&self, owner_key: &IdentityKey) -> bool {
        let payload = signing_payload(&self.id, self.timestamp, &self.storage_path);
        KeyPair::verify(owner_key, &payload, &self.signature)
    }

    /// Mark this capture as sent to a contact.
    pub fn mark_sent_to(&mut self, contact_key_hex: String) {
        if !self.sent_to.contains(&contact_key_hex) {
            self.sent_to.push(contact_key_hex);
        }
    }

    pub fn was_sent_to(&self, contact_key_hex: &str) -> bool {
        self.sent_to.contains(&contact_key_hex.to_string())
    }
}

static CAPTURE_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn derive_capture_id(owner: &IdentityKey, timestamp: u64) -> String {
    let counter = CAPTURE_COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let mut hasher = Sha256::new();
    hasher.update(&owner.0);
    hasher.update(&timestamp.to_le_bytes());
    hasher.update(&counter.to_le_bytes());
    hasher.update(b"theos-capture-id-v1");
    hasher.finalize().iter().map(|b| format!("{:02x}", b)).collect()
}

fn signing_payload(id: &str, timestamp: u64, path: &str) -> Vec<u8> {
    let mut p = Vec::new();
    p.extend_from_slice(id.as_bytes());
    p.extend_from_slice(&timestamp.to_le_bytes());
    p.extend_from_slice(path.as_bytes());
    p.extend_from_slice(b"theos-photo-v1");
    p
}

// -- CameraIntent -------------------------------------------------------------

/// What the AI passes to the camera module after parsing user input.
#[derive(Debug, Clone, PartialEq)]
pub enum CameraIntent {
    /// Just take a photo, save it
    Capture { lens: CameraLens },
    /// Take a photo and send it to a contact
    CaptureAndSend { lens: CameraLens, contact_name: String },
    /// Switch to front camera (selfie)
    Selfie,
    /// Open photo gallery
    Gallery,
    /// Unknown camera intent
    Unknown { raw: String },
}

impl CameraIntent {
    /// Parse user text into a camera intent.
    pub fn parse(input: &str) -> Self {
        let s = input.trim().to_lowercase();

        // Selfie variants
        if s.contains("selfie") || s.contains("front camera") || s.contains("front-facing") {
            return CameraIntent::Selfie;
        }

        // Gallery
        if s.contains("gallery") || s.contains("photos") || s.contains("album") {
            return CameraIntent::Gallery;
        }

        // Capture and send
        if (s.contains("take") || s.contains("photo") || s.contains("picture") || s.contains("shot"))
            && (s.contains("send") || s.contains("share"))
        {
            // Extract contact name after "send to" or "share with"
            let contact = extract_contact(&s);
            return CameraIntent::CaptureAndSend {
                lens:         CameraLens::Main,
                contact_name: contact,
            };
        }

        // Plain capture
        if s.contains("take") || s.contains("photo") || s.contains("picture")
            || s.contains("camera") || s.contains("shoot") || s.contains("snap")
        {
            let lens = if s.contains("telephoto") || s.contains("zoom") {
                CameraLens::Telephoto
            } else {
                CameraLens::Main
            };
            return CameraIntent::Capture { lens };
        }

        CameraIntent::Unknown { raw: input.trim().to_string() }
    }

    /// AI response text for this intent.
    pub fn response(&self) -> String {
        match self {
            CameraIntent::Capture { lens } =>
                format!("Opening {} camera...", lens_name(lens)),
            CameraIntent::CaptureAndSend { contact_name, .. } =>
                format!("Taking a photo to send to {}...", contact_name),
            CameraIntent::Selfie =>
                "Opening front camera...".to_string(),
            CameraIntent::Gallery =>
                "Opening your photos...".to_string(),
            CameraIntent::Unknown { raw } =>
                format!("Not sure what you mean by \"{}\". Try: take a photo, selfie, or open gallery.", raw),
        }
    }
}

fn lens_name(lens: &CameraLens) -> &str {
    match lens {
        CameraLens::Main      => "main",
        CameraLens::Telephoto => "telephoto",
        CameraLens::Front     => "front",
    }
}

fn extract_contact(s: &str) -> String {
    // Look for "send to <name>" or "share with <name>"
    for prefix in &["send to ", "share with ", "send it to ", "share it with "] {
        if let Some(pos) = s.find(prefix) {
            let rest = &s[pos + prefix.len()..];
            // Take until end or punctuation
            let name: String = rest.chars()
                .take_while(|c| c.is_alphabetic() || *c == ' ')
                .collect();
            let name = name.trim().to_string();
            if !name.is_empty() {
                return name;
            }
        }
    }
    "contact".to_string()
}

// -- CameraSession ------------------------------------------------------------

/// A camera session -- tracks state across a capture sequence.
pub struct CameraSession {
    pub state:    CameraState,
    pub lens:     CameraLens,
    pub captures: VecDeque<CameraCapture>,
    owner_key:    IdentityKey,
}

impl CameraSession {
    pub fn new(owner_key: IdentityKey) -> Self {
        Self {
            state:    CameraState::Idle,
            lens:     CameraLens::Main,
            captures: VecDeque::new(),
            owner_key,
        }
    }

    pub fn switch_lens(&mut self, lens: CameraLens) {
        if self.state.is_ready() {
            println!("[camera] switching to {:?}", lens);
            self.lens = lens;
        }
    }

    /// Simulate a capture (demo mode -- no hardware).
    /// Production: opens V4L2 device, reads frame, ISP processes, writes enc file.
    pub fn capture_demo(&mut self, keypair: &KeyPair) -> Option<&CameraCapture> {
        if !self.state.is_ready() {
            println!("[camera] not ready -- state: {}", self.state.label());
            return None;
        }

        self.state = CameraState::Capturing;
        println!("[camera] capturing from {:?}", self.lens);

        self.state = CameraState::Processing;
        println!("[camera] processing...");

        // Create capture record
        let capture = CameraCapture::new(
            self.lens.clone(),
            4032, 3024, // OnePlus 6 main camera resolution
            &self.owner_key,
            keypair,
        );

        println!("[camera] saved: {} -> {}", &capture.id[..8], capture.storage_path);

        if self.captures.len() >= MAX_RECENT_CAPTURES {
            self.captures.pop_front();
        }
        self.captures.push_back(capture);
        self.state = CameraState::Saved;

        self.captures.back()
    }

    /// Reset to idle after a capture.
    pub fn reset(&mut self) {
        if matches!(self.state, CameraState::Saved | CameraState::Error(_)) {
            self.state = CameraState::Idle;
        }
    }

    pub fn capture_count(&self) -> usize { self.captures.len() }

    pub fn latest(&self) -> Option<&CameraCapture> { self.captures.back() }
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

// -- Tests --------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::keypair::KeyPair;

    fn kp() -> KeyPair { KeyPair::generate() }

    // CameraState

    #[test]
    fn test_idle_is_ready() {
        assert!(CameraState::Idle.is_ready());
    }

    #[test]
    fn test_other_states_not_ready() {
        assert!(!CameraState::Unavailable.is_ready());
        assert!(!CameraState::Capturing.is_ready());
        assert!(!CameraState::Processing.is_ready());
        assert!(!CameraState::Saved.is_ready());
        assert!(!CameraState::Error("oops".to_string()).is_ready());
    }

    #[test]
    fn test_state_labels() {
        assert_eq!(CameraState::Idle.label(),        "ready");
        assert_eq!(CameraState::Capturing.label(),   "capturing");
        assert_eq!(CameraState::Processing.label(),  "processing");
        assert_eq!(CameraState::Saved.label(),       "saved");
        assert_eq!(CameraState::Unavailable.label(), "unavailable");
    }

    // CameraIntent parsing

    #[test]
    fn test_intent_take_photo() {
        assert_eq!(
            CameraIntent::parse("take a photo"),
            CameraIntent::Capture { lens: CameraLens::Main }
        );
    }

    #[test]
    fn test_intent_picture() {
        assert_eq!(
            CameraIntent::parse("take a picture"),
            CameraIntent::Capture { lens: CameraLens::Main }
        );
    }

    #[test]
    fn test_intent_camera() {
        assert_eq!(
            CameraIntent::parse("open camera"),
            CameraIntent::Capture { lens: CameraLens::Main }
        );
    }

    #[test]
    fn test_intent_snap() {
        assert_eq!(
            CameraIntent::parse("snap a shot"),
            CameraIntent::Capture { lens: CameraLens::Main }
        );
    }

    #[test]
    fn test_intent_telephoto() {
        assert_eq!(
            CameraIntent::parse("take a photo with zoom"),
            CameraIntent::Capture { lens: CameraLens::Telephoto }
        );
    }

    #[test]
    fn test_intent_selfie() {
        assert_eq!(CameraIntent::parse("take a selfie"), CameraIntent::Selfie);
        assert_eq!(CameraIntent::parse("front camera"),  CameraIntent::Selfie);
    }

    #[test]
    fn test_intent_gallery() {
        assert_eq!(CameraIntent::parse("open gallery"), CameraIntent::Gallery);
        assert_eq!(CameraIntent::parse("show my photos"), CameraIntent::Gallery);
    }

    #[test]
    fn test_intent_capture_and_send() {
        let intent = CameraIntent::parse("take a photo and send to Sarah");
        assert!(matches!(intent, CameraIntent::CaptureAndSend { .. }));
        if let CameraIntent::CaptureAndSend { contact_name, .. } = intent {
            assert_eq!(contact_name, "sarah");
        }
    }

    #[test]
    fn test_intent_share_with() {
        let intent = CameraIntent::parse("take a picture and share with Marcus");
        assert!(matches!(intent, CameraIntent::CaptureAndSend { .. }));
        if let CameraIntent::CaptureAndSend { contact_name, .. } = intent {
            assert_eq!(contact_name, "marcus");
        }
    }

    #[test]
    fn test_intent_unknown() {
        assert_eq!(
            CameraIntent::parse("do something weird"),
            CameraIntent::Unknown { raw: "do something weird".to_string() }
        );
    }

    #[test]
    fn test_intent_responses_non_empty() {
        let kp = kp();
        let key = kp.public.clone();
        let intents = vec![
            CameraIntent::Capture { lens: CameraLens::Main },
            CameraIntent::CaptureAndSend { lens: CameraLens::Main, contact_name: "Sarah".to_string() },
            CameraIntent::Selfie,
            CameraIntent::Gallery,
            CameraIntent::Unknown { raw: "weird".to_string() },
        ];
        for intent in intents {
            assert!(!intent.response().is_empty());
        }
    }

    // CameraCapture

    #[test]
    fn test_capture_creation() {
        let kp      = kp();
        let capture = CameraCapture::new(CameraLens::Main, 4032, 3024, &kp.public, &kp);
        assert!(!capture.id.is_empty());
        assert!(capture.storage_path.contains("/run/theos/photos"));
        assert_eq!(capture.width,  4032);
        assert_eq!(capture.height, 3024);
        assert_eq!(capture.sent_to.len(), 0);
    }

    #[test]
    fn test_capture_signature_verifies() {
        let kp      = kp();
        let capture = CameraCapture::new(CameraLens::Main, 4032, 3024, &kp.public, &kp);
        assert!(capture.verify(&kp.public));
    }

    #[test]
    fn test_capture_wrong_key_fails_verify() {
        let kp1     = kp();
        let kp2     = kp();
        let capture = CameraCapture::new(CameraLens::Main, 4032, 3024, &kp1.public, &kp1);
        assert!(!capture.verify(&kp2.public));
    }

    #[test]
    fn test_capture_mark_sent() {
        let kp      = kp();
        let mut cap = CameraCapture::new(CameraLens::Main, 4032, 3024, &kp.public, &kp);
        assert!(!cap.was_sent_to("abc123"));
        cap.mark_sent_to("abc123".to_string());
        assert!(cap.was_sent_to("abc123"));
    }

    #[test]
    fn test_capture_no_duplicate_sent() {
        let kp      = kp();
        let mut cap = CameraCapture::new(CameraLens::Main, 4032, 3024, &kp.public, &kp);
        cap.mark_sent_to("abc123".to_string());
        cap.mark_sent_to("abc123".to_string());
        assert_eq!(cap.sent_to.len(), 1);
    }

    // CameraSession

    #[test]
    fn test_session_starts_idle() {
        let kp      = kp();
        let session = CameraSession::new(kp.public.clone());
        assert_eq!(session.state, CameraState::Idle);
        assert_eq!(session.capture_count(), 0);
    }

    #[test]
    fn test_session_capture_demo() {
        let kp          = kp();
        let mut session = CameraSession::new(kp.public.clone());
        let capture     = session.capture_demo(&kp);
        assert!(capture.is_some());
        assert_eq!(session.state, CameraState::Saved);
        assert_eq!(session.capture_count(), 1);
    }

    #[test]
    fn test_session_reset_after_capture() {
        let kp          = kp();
        let mut session = CameraSession::new(kp.public.clone());
        session.capture_demo(&kp);
        session.reset();
        assert_eq!(session.state, CameraState::Idle);
    }

    #[test]
    fn test_session_multiple_captures() {
        let kp          = kp();
        let mut session = CameraSession::new(kp.public.clone());
        session.capture_demo(&kp);
        session.reset();
        session.capture_demo(&kp);
        assert_eq!(session.capture_count(), 2);
    }

    #[test]
    fn test_session_switch_lens() {
        let kp          = kp();
        let mut session = CameraSession::new(kp.public.clone());
        session.switch_lens(CameraLens::Front);
        assert_eq!(session.lens, CameraLens::Front);
    }

    #[test]
    fn test_session_cannot_capture_when_not_idle() {
        let kp          = kp();
        let mut session = CameraSession::new(kp.public.clone());
        session.state   = CameraState::Capturing;
        let result      = session.capture_demo(&kp);
        assert!(result.is_none());
    }

    #[test]
    fn test_session_latest_returns_most_recent() {
        let kp          = kp();
        let mut session = CameraSession::new(kp.public.clone());
        session.capture_demo(&kp);
        let first_id    = session.latest().unwrap().id.clone();
        session.reset();
        session.capture_demo(&kp);
        let second_id   = session.latest().unwrap().id.clone();
        assert_ne!(first_id, second_id);
    }

    #[test]
    fn test_capture_id_deterministic_structure() {
        let kp  = kp();
        let cap = CameraCapture::new(CameraLens::Main, 4032, 3024, &kp.public, &kp);
        // ID should be 64 hex chars (SHA-256)
        assert_eq!(cap.id.len(), 64);
        assert!(cap.id.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn test_v4l2_device_paths() {
        assert_eq!(CameraLens::Main.v4l2_device(),      "/dev/video0");
        assert_eq!(CameraLens::Telephoto.v4l2_device(), "/dev/video1");
        assert_eq!(CameraLens::Front.v4l2_device(),     "/dev/video2");
    }
}
