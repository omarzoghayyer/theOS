// call_ui.rs -- theOS Identity-Based Call Surface
//
// The active-call screen. Shown when calling or in a call with a contact.
//
// theOS principle: calls are placed to a CONTACT IDENTITY (Ed25519 key /
// handle), never a phone number. There is no number to dial. This surface
// reflects that -- it shows who you're talking to by handle and key
// fingerprint, the encryption status, and the satellite link quality.
//
// Visual design:
//   - Deep space dark background (matches orb/messenger)
//   - Large contact avatar circle, centered upper third
//   - Contact handle (@sarah) + truncated key fingerprint below
//   - Call state line: "Connecting...", "Ringing...", "00:42", "Call ended"
//   - Encryption pill: "Ed25519 + ChaCha20" -- the whole point of theOS
//   - Satellite link quality indicator
//   - Control row (lower third): mute, end call, speaker
//
// State mirrors services/voip.rs CallState but UI-focused (no SocketAddr --
// the surface doesn't need network details, only what to display).

use crate::render::{Surface, TouchState};
use smithay::backend::renderer::gles::GlesFrame;
use smithay::utils::{Physical, Size};

// -- Call lifecycle (UI view) -------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub enum CallUiState {
    /// Placing the call -- looking up contact in DHT, establishing session.
    Connecting,
    /// Remote device is ringing -- waiting for answer.
    Ringing,
    /// Call is live -- encrypted audio flowing. started_at = unix secs.
    Active { started_at: u64 },
    /// Incoming call -- waiting for the user to accept or decline.
    Incoming,
    /// Call ended -- terminal state before returning to orb.
    Ended,
}

impl CallUiState {
    pub fn label(&self) -> &'static str {
        match self {
            CallUiState::Connecting => "connecting",
            CallUiState::Ringing    => "ringing",
            CallUiState::Active {..} => "active",
            CallUiState::Incoming   => "incoming",
            CallUiState::Ended      => "ended",
        }
    }

    pub fn is_active(&self) -> bool {
        matches!(self, CallUiState::Active {..})
    }

    pub fn is_terminal(&self) -> bool {
        matches!(self, CallUiState::Ended)
    }

    /// Human-readable status line for the UI.
    pub fn status_line(&self, now: u64) -> String {
        match self {
            CallUiState::Connecting => "Connecting...".to_string(),
            CallUiState::Ringing    => "Ringing...".to_string(),
            CallUiState::Incoming   => "Incoming call".to_string(),
            CallUiState::Ended      => "Call ended".to_string(),
            CallUiState::Active { started_at } => {
                let secs = now.saturating_sub(*started_at);
                format_duration(secs)
            }
        }
    }
}

/// Format call duration as MM:SS (or HH:MM:SS past an hour).
pub fn format_duration(secs: u64) -> String {
    let h = secs / 3600;
    let m = (secs % 3600) / 60;
    let s = secs % 60;
    if h > 0 {
        format!("{:02}:{:02}:{:02}", h, m, s)
    } else {
        format!("{:02}:{:02}", m, s)
    }
}

// -- Satellite link quality ---------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LinkQuality {
    Excellent, // < 600ms latency
    Good,      // 600-900ms
    Fair,      // 900-1500ms
    Poor,      // > 1500ms or beam switching
}

impl LinkQuality {
    pub fn from_latency(latency_ms: u32) -> Self {
        match latency_ms {
            0..=599    => LinkQuality::Excellent,
            600..=899  => LinkQuality::Good,
            900..=1499 => LinkQuality::Fair,
            _          => LinkQuality::Poor,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            LinkQuality::Excellent => "excellent",
            LinkQuality::Good      => "good",
            LinkQuality::Fair      => "fair",
            LinkQuality::Poor      => "poor",
        }
    }

    /// Number of signal bars (0-4) for the UI indicator.
    pub fn bars(&self) -> u8 {
        match self {
            LinkQuality::Excellent => 4,
            LinkQuality::Good      => 3,
            LinkQuality::Fair      => 2,
            LinkQuality::Poor      => 1,
        }
    }
}

// -- CallSurface --------------------------------------------------------------

pub struct CallSurface {
    pub state:        CallUiState,
    /// Contact handle, e.g. "@sarah" (None if calling a raw key)
    pub handle:       Option<String>,
    /// Truncated key fingerprint for display, e.g. "a1b2c3d4"
    pub key_fp:       String,
    pub muted:        bool,
    pub speaker_on:   bool,
    pub latency_ms:   u32,
}

impl CallSurface {
    pub fn new(handle: Option<String>, key_fp: String) -> Self {
        Self {
            state:      CallUiState::Connecting,
            handle,
            key_fp,
            muted:      false,
            speaker_on: false,
            latency_ms: 0,
        }
    }

    /// Build a call surface from a contact key hex string.
    /// Derives the display fingerprint (first 8 hex chars).
    pub fn from_key_hex(handle: Option<String>, key_hex: &str) -> Self {
        let fp = key_hex.chars().take(8).collect::<String>();
        Self::new(handle, fp)
    }

    /// Display name: handle if known, otherwise key fingerprint.
    pub fn display_name(&self) -> String {
        match &self.handle {
            Some(h) => h.clone(),
            None    => format!("key:{}", self.key_fp),
        }
    }

    pub fn link_quality(&self) -> LinkQuality {
        LinkQuality::from_latency(self.latency_ms)
    }

    // -- State transitions --

    /// Remote device started ringing.
    pub fn on_ringing(&mut self) {
        if matches!(self.state, CallUiState::Connecting) {
            self.state = CallUiState::Ringing;
        }
    }

    /// Call was answered -- audio now flowing.
    pub fn on_answered(&mut self, now: u64) {
        self.state = CallUiState::Active { started_at: now };
    }

    /// Call ended (by either party).
    pub fn on_ended(&mut self) {
        self.state = CallUiState::Ended;
    }

    pub fn toggle_mute(&mut self) -> bool {
        self.muted = !self.muted;
        self.muted
    }

    pub fn toggle_speaker(&mut self) -> bool {
        self.speaker_on = !self.speaker_on;
        self.speaker_on
    }

    pub fn update_latency(&mut self, latency_ms: u32) {
        self.latency_ms = latency_ms;
    }

    /// Call duration in seconds (0 if not active).
    pub fn duration_secs(&self, now: u64) -> u64 {
        match &self.state {
            CallUiState::Active { started_at } => now.saturating_sub(*started_at),
            _ => 0,
        }
    }
}

impl Surface for CallSurface {
    fn name(&self) -> &str { "call" }

    fn draw(&self, _frame: &mut GlesFrame, _size: Size<i32, Physical>) {
        let now = now_secs();
        println!(
            "[call] render -- {} state:{} status:'{}' enc:Ed25519+ChaCha20 link:{} muted:{} spkr:{}",
            self.display_name(),
            self.state.label(),
            self.state.status_line(now),
            self.link_quality().label(),
            self.muted,
            self.speaker_on,
        );
    }

    fn handle_touch(&mut self, x: f64, y: f64, state: TouchState) {
        if state != TouchState::Down { return; }

        // Control row lives in the lower fifth of the screen.
        // Three controls: mute (left), end call (center), speaker (right).
        let screen_h = 2400.0;
        let screen_w = 1080.0;
        let control_top = screen_h * 0.80;

        if y < control_top { return; }

        let third = screen_w / 3.0;
        if x < third {
            let muted = self.toggle_mute();
            println!("[call] mute -> {}", muted);
        } else if x < third * 2.0 {
            self.on_ended();
            println!("[call] end call");
        } else {
            let spkr = self.toggle_speaker();
            println!("[call] speaker -> {}", spkr);
        }
    }
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

    fn surface() -> CallSurface {
        CallSurface::from_key_hex(Some("@sarah".to_string()), "a1b2c3d4e5f6")
    }

    #[test]
    fn test_new_starts_connecting() {
        let s = surface();
        assert_eq!(s.state, CallUiState::Connecting);
    }

    #[test]
    fn test_from_key_hex_truncates_fingerprint() {
        let s = CallSurface::from_key_hex(None, "a1b2c3d4e5f6a7b8");
        assert_eq!(s.key_fp, "a1b2c3d4");
    }

    #[test]
    fn test_display_name_prefers_handle() {
        let s = surface();
        assert_eq!(s.display_name(), "@sarah");
    }

    #[test]
    fn test_display_name_falls_back_to_key() {
        let s = CallSurface::from_key_hex(None, "a1b2c3d4e5f6");
        assert_eq!(s.display_name(), "key:a1b2c3d4");
    }

    #[test]
    fn test_connecting_to_ringing() {
        let mut s = surface();
        s.on_ringing();
        assert_eq!(s.state, CallUiState::Ringing);
    }

    #[test]
    fn test_ringing_to_active() {
        let mut s = surface();
        s.on_ringing();
        s.on_answered(1000);
        assert_eq!(s.state, CallUiState::Active { started_at: 1000 });
        assert!(s.state.is_active());
    }

    #[test]
    fn test_active_to_ended() {
        let mut s = surface();
        s.on_answered(1000);
        s.on_ended();
        assert_eq!(s.state, CallUiState::Ended);
        assert!(s.state.is_terminal());
    }

    #[test]
    fn test_on_answered_from_connecting_directly() {
        // Answering should work even if we skipped the ringing display
        let mut s = surface();
        s.on_answered(500);
        assert!(s.state.is_active());
    }

    #[test]
    fn test_ringing_only_from_connecting() {
        let mut s = surface();
        s.on_answered(1000); // now active
        s.on_ringing();      // should NOT revert to ringing
        assert!(s.state.is_active());
    }

    #[test]
    fn test_toggle_mute() {
        let mut s = surface();
        assert!(!s.muted);
        assert!(s.toggle_mute());
        assert!(s.muted);
        assert!(!s.toggle_mute());
        assert!(!s.muted);
    }

    #[test]
    fn test_toggle_speaker() {
        let mut s = surface();
        assert!(!s.speaker_on);
        assert!(s.toggle_speaker());
        assert!(s.speaker_on);
    }

    #[test]
    fn test_duration_zero_when_not_active() {
        let s = surface();
        assert_eq!(s.duration_secs(9999), 0);
    }

    #[test]
    fn test_duration_counts_from_start() {
        let mut s = surface();
        s.on_answered(1000);
        assert_eq!(s.duration_secs(1042), 42);
    }

    #[test]
    fn test_duration_saturates_no_underflow() {
        let mut s = surface();
        s.on_answered(1000);
        // now < started_at should not panic
        assert_eq!(s.duration_secs(500), 0);
    }

    // -- Status line --

    #[test]
    fn test_status_line_connecting() {
        let s = surface();
        assert_eq!(s.state.status_line(0), "Connecting...");
    }

    #[test]
    fn test_status_line_ringing() {
        let mut s = surface();
        s.on_ringing();
        assert_eq!(s.state.status_line(0), "Ringing...");
    }

    #[test]
    fn test_status_line_active_shows_duration() {
        let mut s = surface();
        s.on_answered(1000);
        assert_eq!(s.state.status_line(1065), "01:05");
    }

    #[test]
    fn test_status_line_ended() {
        let mut s = surface();
        s.on_ended();
        assert_eq!(s.state.status_line(0), "Call ended");
    }

    // -- Duration formatting --

    #[test]
    fn test_format_duration_under_minute() {
        assert_eq!(format_duration(42), "00:42");
    }

    #[test]
    fn test_format_duration_minutes() {
        assert_eq!(format_duration(125), "02:05");
    }

    #[test]
    fn test_format_duration_over_hour() {
        assert_eq!(format_duration(3725), "01:02:05");
    }

    #[test]
    fn test_format_duration_zero() {
        assert_eq!(format_duration(0), "00:00");
    }

    // -- Link quality --

    #[test]
    fn test_link_quality_excellent() {
        assert_eq!(LinkQuality::from_latency(400), LinkQuality::Excellent);
        assert_eq!(LinkQuality::from_latency(400).bars(), 4);
    }

    #[test]
    fn test_link_quality_good() {
        assert_eq!(LinkQuality::from_latency(700), LinkQuality::Good);
        assert_eq!(LinkQuality::from_latency(700).bars(), 3);
    }

    #[test]
    fn test_link_quality_fair() {
        assert_eq!(LinkQuality::from_latency(1200), LinkQuality::Fair);
        assert_eq!(LinkQuality::from_latency(1200).bars(), 2);
    }

    #[test]
    fn test_link_quality_poor() {
        assert_eq!(LinkQuality::from_latency(2000), LinkQuality::Poor);
        assert_eq!(LinkQuality::from_latency(2000).bars(), 1);
    }

    #[test]
    fn test_link_quality_from_surface() {
        let mut s = surface();
        s.update_latency(550);
        assert_eq!(s.link_quality(), LinkQuality::Excellent);
    }

    #[test]
    fn test_incoming_state() {
        let mut s = surface();
        s.state = CallUiState::Incoming;
        assert_eq!(s.state.status_line(0), "Incoming call");
        assert!(!s.state.is_active());
        assert!(!s.state.is_terminal());
    }
}
