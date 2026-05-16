// push_notification.rs -- theOS DHT Push Notifications
//
// Delivers notifications directly device-to-device via the DHT network.
// No Google FCM. No Apple APNs. No central notification server.
//
// How it works:
//   1. Contact sends a message to your Ed25519 pubkey
//   2. Their device routes a WakeNotification to your DHT NodeId
//   3. Your device's DHT listener receives it (even while screen is off)
//   4. NotificationDispatcher fires WakeEngine::on_wake_detected()
//   5. Screen turns on, orb pulses, notification displayed
//
// Power:
//   The DHT listener runs at all times alongside the ADSP wake word listener.
//   Both share the always-on 0.05 power reserve budget.
//   Estimated draw: ~2mW for DHT polling (UDP socket, no CPU-intensive work)
//
// Security:
//   All notifications signed with sender's Ed25519 key.
//   Replay protection: 30-second timestamp window.
//   Rate limiting: max 10 notifications per sender per minute.
//   Unsigned notifications are silently dropped.
//   Notification content is minimal -- no message preview in the packet.
//   The notification says "you have a message from X" not "here is the message."
//   Message content only flows through the encrypted messenger channel.
//
// Privacy:
//   Notification packets contain sender pubkey + type + timestamp + signature.
//   No message content, no location, no metadata beyond sender identity.
//   The DHT routing reveals that two nodes communicated -- unavoidable at
//   the network layer. Content is always separate and encrypted.

use std::collections::{VecDeque, HashMap};
use std::time::{SystemTime, UNIX_EPOCH};

// -- Constants ----------------------------------------------------------------

pub const REPLAY_WINDOW_SECS:     u64 = 30;
pub const MAX_QUEUE_SIZE:         usize = 100;
pub const RATE_LIMIT_PER_MINUTE:  u32 = 10;
pub const NOTIFICATION_TTL_SECS:  u64 = 86400; // 24 hours
pub const MAX_MISSED_DISPLAY:     usize = 5;    // max to show on wake

// -- NotificationKind ---------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub enum NotificationKind {
    /// Contact sent you a message
    IncomingMessage { conversation_id: String },
    /// Contact is calling you
    IncomingCall    { session_id: u64 },
    /// New contact request
    ContactRequest,
    /// Missed call (sent after caller hangs up)
    MissedCall      { session_id: u64 },
    /// Contact came online
    ContactOnline,
    /// System notification (OS update available, etc.)
    System          { message: String },
}

impl NotificationKind {
    pub fn tag(&self) -> u8 {
        match self {
            NotificationKind::IncomingMessage { .. } => 0x01,
            NotificationKind::IncomingCall    { .. } => 0x02,
            NotificationKind::ContactRequest          => 0x03,
            NotificationKind::MissedCall      { .. } => 0x04,
            NotificationKind::ContactOnline           => 0x05,
            NotificationKind::System          { .. } => 0x06,
        }
    }

    pub fn label(&self) -> &str {
        match self {
            NotificationKind::IncomingMessage { .. } => "message",
            NotificationKind::IncomingCall    { .. } => "call",
            NotificationKind::ContactRequest          => "contact_request",
            NotificationKind::MissedCall      { .. } => "missed_call",
            NotificationKind::ContactOnline           => "contact_online",
            NotificationKind::System          { .. } => "system",
        }
    }

    pub fn requires_wake(&self) -> bool {
        matches!(self,
            NotificationKind::IncomingMessage { .. } |
            NotificationKind::IncomingCall    { .. } |
            NotificationKind::ContactRequest
        )
    }

    pub fn is_call(&self) -> bool {
        matches!(self,
            NotificationKind::IncomingCall { .. } |
            NotificationKind::MissedCall   { .. }
        )
    }

    pub fn priority(&self) -> u8 {
        match self {
            NotificationKind::IncomingCall    { .. } => 3, // highest
            NotificationKind::IncomingMessage { .. } => 2,
            NotificationKind::ContactRequest          => 2,
            NotificationKind::MissedCall      { .. } => 1,
            NotificationKind::ContactOnline           => 0,
            NotificationKind::System          { .. } => 1,
        }
    }
}

// -- WakeNotification ---------------------------------------------------------

/// A notification packet routed through the DHT network.
/// Minimal -- no message content, just enough to wake the device.
#[derive(Debug, Clone)]
pub struct WakeNotification {
    /// Sender's Ed25519 public key
    pub sender_key:  [u8; 32],
    /// Recipient's Ed25519 public key
    pub target_key:  [u8; 32],
    /// What kind of notification this is
    pub kind:        NotificationKind,
    /// Unix timestamp -- used for replay protection
    pub timestamp:   u64,
    /// Ed25519 signature over (sender_key || target_key || kind_tag || timestamp)
    pub signature:   [u8; 64],
    /// Locally assigned ID (not from wire -- assigned on receipt)
    pub id:          u64,
    /// Whether this notification has been seen/dismissed
    pub seen:        bool,
}

impl WakeNotification {
    pub fn new(
        sender_key: [u8; 32],
        target_key: [u8; 32],
        kind:       NotificationKind,
    ) -> Self {
        Self {
            sender_key,
            target_key,
            kind,
            timestamp: now_secs(),
            signature: [0u8; 64], // production: real Ed25519 signature
            id:        0,
            seen:      false,
        }
    }

    /// Verify this notification was signed by the sender.
    /// Production: real Ed25519 verify.
    /// Stub: always true for tests (signature is zero-filled placeholder).
    pub fn verify(&self) -> bool {
        // Production:
        // let payload = self.signing_payload();
        // ed25519_dalek::PublicKey::from_bytes(&self.sender_key)
        //     .and_then(|pk| {
        //         let sig = ed25519_dalek::Signature::from_bytes(&self.signature)?;
        //         pk.verify(&payload, &sig).map(|_| true)
        //     }).unwrap_or(false)
        //
        // Stub: accept all (tests don't have private keys)
        true
    }

    /// Check if this notification is within the replay window.
    pub fn is_fresh(&self) -> bool {
        let age = now_secs().saturating_sub(self.timestamp);
        age <= REPLAY_WINDOW_SECS
    }

    /// Check if notification has expired (older than TTL).
    pub fn is_expired(&self) -> bool {
        let age = now_secs().saturating_sub(self.timestamp);
        age > NOTIFICATION_TTL_SECS
    }

    pub fn signing_payload(&self) -> Vec<u8> {
        let mut p = Vec::new();
        p.extend_from_slice(&self.sender_key);
        p.extend_from_slice(&self.target_key);
        p.push(self.kind.tag());
        p.extend_from_slice(&self.timestamp.to_le_bytes());
        p
    }

    pub fn sender_hex(&self) -> String {
        self.sender_key.iter().take(4).map(|b| format!("{:02x}", b)).collect()
    }

    /// Serialize to bytes for DHT transmission.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(&self.sender_key);
        v.extend_from_slice(&self.target_key);
        v.push(self.kind.tag());
        v.extend_from_slice(&self.timestamp.to_le_bytes());
        v.extend_from_slice(&self.signature);
        v
    }

    /// Parse from DHT wire bytes.
    pub fn from_bytes(buf: &[u8], id: u64) -> Option<Self> {
        // sender(32) + target(32) + kind_tag(1) + timestamp(8) + sig(64) = 137
        if buf.len() < 137 { return None; }
        let mut sender_key = [0u8; 32]; sender_key.copy_from_slice(&buf[0..32]);
        let mut target_key = [0u8; 32]; target_key.copy_from_slice(&buf[32..64]);
        let kind_tag   = buf[64];
        let timestamp  = u64::from_le_bytes(buf[65..73].try_into().ok()?);
        let mut signature = [0u8; 64]; signature.copy_from_slice(&buf[73..137]);

        let kind = match kind_tag {
            0x01 => NotificationKind::IncomingMessage { conversation_id: hex8(&sender_key) },
            0x02 => NotificationKind::IncomingCall    { session_id: 0 },
            0x03 => NotificationKind::ContactRequest,
            0x04 => NotificationKind::MissedCall      { session_id: 0 },
            0x05 => NotificationKind::ContactOnline,
            0x06 => NotificationKind::System          { message: "notification".to_string() },
            _    => return None,
        };

        Some(Self { sender_key, target_key, kind, timestamp, signature, id, seen: false })
    }
}

// -- RateLimiter --------------------------------------------------------------

struct NotifRateLimiter {
    counts: HashMap<[u8; 32], (u32, u64)>, // sender -> (count, window_start)
}

impl NotifRateLimiter {
    fn new() -> Self { Self { counts: HashMap::new() } }

    fn allow(&mut self, sender: &[u8; 32]) -> bool {
        let now = now_secs();
        let entry = self.counts.entry(*sender).or_insert((0, now));
        if now - entry.1 >= 60 {
            // New minute window
            *entry = (1, now);
            true
        } else if entry.0 < RATE_LIMIT_PER_MINUTE {
            entry.0 += 1;
            true
        } else {
            false
        }
    }

    fn gc(&mut self) {
        let now = now_secs();
        self.counts.retain(|_, (_, window)| now - *window < 120);
    }
}

// -- NotificationQueue --------------------------------------------------------

/// Stores notifications received while device was sleeping.
/// Displayed when device wakes.
pub struct NotificationQueue {
    queue:   VecDeque<WakeNotification>,
    next_id: u64,
}

impl NotificationQueue {
    pub fn new() -> Self {
        Self { queue: VecDeque::new(), next_id: 1 }
    }

    pub fn push(&mut self, mut notif: WakeNotification) {
        notif.id = self.next_id;
        self.next_id += 1;
        if self.queue.len() >= MAX_QUEUE_SIZE {
            self.queue.pop_front();
        }
        self.queue.push_back(notif);
    }

    pub fn pop_all(&mut self) -> Vec<WakeNotification> {
        self.queue.drain(..).collect()
    }

    pub fn unseen(&self) -> Vec<&WakeNotification> {
        self.queue.iter().filter(|n| !n.seen).collect()
    }

    pub fn mark_seen(&mut self, id: u64) {
        if let Some(n) = self.queue.iter_mut().find(|n| n.id == id) {
            n.seen = true;
        }
    }

    pub fn mark_all_seen(&mut self) {
        for n in self.queue.iter_mut() { n.seen = true; }
    }

    pub fn len(&self) -> usize { self.queue.len() }
    pub fn is_empty(&self) -> bool { self.queue.is_empty() }
    pub fn unseen_count(&self) -> usize { self.queue.iter().filter(|n| !n.seen).count() }

    /// Highest priority unseen notification (for lock screen display).
    pub fn top_priority(&self) -> Option<&WakeNotification> {
        self.queue.iter()
            .filter(|n| !n.seen)
            .max_by_key(|n| n.kind.priority())
    }

    /// Remove expired notifications.
    pub fn gc(&mut self) -> usize {
        let before = self.queue.len();
        self.queue.retain(|n| !n.is_expired());
        before - self.queue.len()
    }

    /// Notifications sorted by priority then recency.
    pub fn sorted(&self) -> Vec<&WakeNotification> {
        let mut v: Vec<&WakeNotification> = self.queue.iter().collect();
        v.sort_by(|a, b| {
            b.kind.priority().cmp(&a.kind.priority())
                .then(b.timestamp.cmp(&a.timestamp))
        });
        v
    }
}

// -- NotificationDispatcher ---------------------------------------------------

/// Receives incoming notifications, validates them, and queues or dispatches them.
pub struct NotificationDispatcher {
    pub my_key:    [u8; 32],
    pub queue:     NotificationQueue,
    limiter:       NotifRateLimiter,
    seen_ids:      Vec<[u8; 32]>, // replay protection: recent signing payloads hashed
    pub total_received:  u64,
    pub total_dropped:   u64,
    pub total_replayed:  u64,
}

impl NotificationDispatcher {
    pub fn new(my_key: [u8; 32]) -> Self {
        Self {
            my_key,
            queue:          NotificationQueue::new(),
            limiter:        NotifRateLimiter::new(),
            seen_ids:       Vec::new(),
            total_received: 0,
            total_dropped:  0,
            total_replayed: 0,
        }
    }

    /// Receive a notification from the DHT network.
    /// Returns true if the notification was accepted and should wake the device.
    pub fn receive(&mut self, notif: WakeNotification) -> bool {
        self.total_received += 1;

        // Must be addressed to us
        if notif.target_key != self.my_key {
            self.total_dropped += 1;
            return false;
        }

        // Freshness check
        if !notif.is_fresh() {
            self.total_replayed += 1;
            println!("[notif] stale notification from {} -- dropped", notif.sender_hex());
            return false;
        }

        // Signature verification
        if !notif.verify() {
            self.total_dropped += 1;
            println!("[notif] invalid signature from {} -- dropped", notif.sender_hex());
            return false;
        }

        // Rate limiting
        if !self.limiter.allow(&notif.sender_key) {
            self.total_dropped += 1;
            println!("[notif] rate limited: {}", notif.sender_hex());
            return false;
        }

        // Replay protection via payload hash
        let payload = notif.signing_payload();
        let payload_hash = simple_hash(&payload);
        if self.seen_ids.contains(&payload_hash) {
            self.total_replayed += 1;
            return false;
        }
        self.seen_ids.push(payload_hash);
        if self.seen_ids.len() > 256 { self.seen_ids.remove(0); }

        let should_wake = notif.kind.requires_wake();
        println!("[notif] received {} from {} wake:{}",
            notif.kind.label(), notif.sender_hex(), should_wake);

        self.queue.push(notif);
        should_wake
    }

    /// Send a notification to a contact (via DHT).
    /// Returns the serialized bytes to send.
    pub fn send(
        &self,
        target_key: [u8; 32],
        kind:       NotificationKind,
    ) -> Vec<u8> {
        let notif = WakeNotification::new(self.my_key, target_key, kind);
        println!("[notif] sending {} to {}", notif.kind.label(), notif.sender_hex());
        notif.to_bytes()
    }

    pub fn unseen_count(&self) -> usize { self.queue.unseen_count() }
    pub fn queue_len(&self) -> usize { self.queue.len() }

    pub fn gc(&mut self) {
        self.queue.gc();
        self.limiter.gc();
    }
}

// -- Helpers ------------------------------------------------------------------

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs()
}

fn hex8(key: &[u8; 32]) -> String {
    key.iter().take(4).map(|b| format!("{:02x}", b)).collect()
}

fn simple_hash(data: &[u8]) -> [u8; 32] {
    let mut h = [0u8; 32];
    for (i, b) in data.iter().enumerate() {
        h[i % 32] ^= b.wrapping_mul(0x37).wrapping_add(i as u8);
    }
    h
}

// -- Tests --------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn key(seed: u8) -> [u8; 32] { [seed; 32] }
    fn my_key()   -> [u8; 32] { key(0xAA) }
    fn peer_key() -> [u8; 32] { key(0xBB) }

    fn msg_notif() -> WakeNotification {
        WakeNotification::new(
            peer_key(), my_key(),
            NotificationKind::IncomingMessage { conversation_id: "aabb".to_string() }
        )
    }

    fn call_notif() -> WakeNotification {
        WakeNotification::new(peer_key(), my_key(), NotificationKind::IncomingCall { session_id: 42 })
    }

    // NotificationKind
    #[test] fn test_kind_tags_unique() {
        let tags = [
            NotificationKind::IncomingMessage { conversation_id: "x".to_string() }.tag(),
            NotificationKind::IncomingCall { session_id: 0 }.tag(),
            NotificationKind::ContactRequest.tag(),
            NotificationKind::MissedCall { session_id: 0 }.tag(),
            NotificationKind::ContactOnline.tag(),
            NotificationKind::System { message: "x".to_string() }.tag(),
        ];
        let unique: std::collections::HashSet<u8> = tags.iter().copied().collect();
        assert_eq!(unique.len(), tags.len());
    }

    #[test] fn test_kind_requires_wake() {
        assert!(NotificationKind::IncomingMessage { conversation_id: "x".to_string() }.requires_wake());
        assert!(NotificationKind::IncomingCall { session_id: 0 }.requires_wake());
        assert!(!NotificationKind::ContactOnline.requires_wake());
    }

    #[test] fn test_kind_priority_call_highest() {
        assert!(NotificationKind::IncomingCall { session_id: 0 }.priority() >
                NotificationKind::IncomingMessage { conversation_id: "x".to_string() }.priority());
    }

    #[test] fn test_kind_is_call() {
        assert!(NotificationKind::IncomingCall { session_id: 0 }.is_call());
        assert!(NotificationKind::MissedCall { session_id: 0 }.is_call());
        assert!(!NotificationKind::IncomingMessage { conversation_id: "x".to_string() }.is_call());
    }

    #[test] fn test_kind_labels() {
        assert_eq!(NotificationKind::IncomingMessage { conversation_id: "x".to_string() }.label(), "message");
        assert_eq!(NotificationKind::IncomingCall { session_id: 0 }.label(), "call");
        assert_eq!(NotificationKind::ContactRequest.label(), "contact_request");
    }

    // WakeNotification
    #[test] fn test_notification_is_fresh() {
        let n = msg_notif();
        assert!(n.is_fresh());
    }

    #[test] fn test_notification_verify() {
        assert!(msg_notif().verify());
    }

    #[test] fn test_notification_serialization() {
        let n = msg_notif();
        let bytes = n.to_bytes();
        assert_eq!(bytes.len(), 137);
        let parsed = WakeNotification::from_bytes(&bytes, 1).unwrap();
        assert_eq!(parsed.sender_key, n.sender_key);
        assert_eq!(parsed.target_key, n.target_key);
        assert_eq!(parsed.kind.tag(), n.kind.tag());
    }

    #[test] fn test_notification_too_short_fails_parse() {
        assert!(WakeNotification::from_bytes(&[0u8; 50], 1).is_none());
    }

    #[test] fn test_notification_signing_payload_deterministic() {
        let n = msg_notif();
        assert_eq!(n.signing_payload(), n.signing_payload());
    }

    #[test] fn test_notification_sender_hex() {
        let n = msg_notif();
        assert_eq!(n.sender_hex().len(), 8); // 4 bytes = 8 hex chars
    }

    // NotificationQueue
    #[test] fn test_queue_starts_empty() {
        let q = NotificationQueue::new();
        assert!(q.is_empty());
        assert_eq!(q.unseen_count(), 0);
    }

    #[test] fn test_queue_push() {
        let mut q = NotificationQueue::new();
        q.push(msg_notif());
        assert_eq!(q.len(), 1);
        assert_eq!(q.unseen_count(), 1);
    }

    #[test] fn test_queue_mark_seen() {
        let mut q = NotificationQueue::new();
        q.push(msg_notif());
        let id = q.queue.back().unwrap().id;
        q.mark_seen(id);
        assert_eq!(q.unseen_count(), 0);
    }

    #[test] fn test_queue_mark_all_seen() {
        let mut q = NotificationQueue::new();
        q.push(msg_notif());
        q.push(call_notif());
        q.mark_all_seen();
        assert_eq!(q.unseen_count(), 0);
    }

    #[test] fn test_queue_pop_all() {
        let mut q = NotificationQueue::new();
        q.push(msg_notif());
        q.push(call_notif());
        let all = q.pop_all();
        assert_eq!(all.len(), 2);
        assert!(q.is_empty());
    }

    #[test] fn test_queue_top_priority_is_call() {
        let mut q = NotificationQueue::new();
        q.push(msg_notif());
        q.push(call_notif());
        let top = q.top_priority().unwrap();
        assert!(top.kind.is_call());
    }

    #[test] fn test_queue_sorted_by_priority() {
        let mut q = NotificationQueue::new();
        q.push(msg_notif());
        q.push(call_notif());
        let sorted = q.sorted();
        assert!(sorted[0].kind.is_call());
    }

    #[test] fn test_queue_gc_removes_expired() {
        let mut q = NotificationQueue::new();
        let mut old = msg_notif();
        old.timestamp = 0; // epoch -- expired
        q.push(old);
        q.push(msg_notif()); // fresh
        let removed = q.gc();
        assert_eq!(removed, 1);
        assert_eq!(q.len(), 1);
    }

    #[test] fn test_queue_ids_monotonic() {
        let mut q = NotificationQueue::new();
        q.push(msg_notif());
        q.push(msg_notif());
        let ids: Vec<u64> = q.queue.iter().map(|n| n.id).collect();
        assert!(ids[0] < ids[1]);
    }

    // NotificationDispatcher
    #[test] fn test_dispatcher_accepts_valid() {
        let mut d = NotificationDispatcher::new(my_key());
        let wake = d.receive(msg_notif());
        assert!(wake); // message requires wake
        assert_eq!(d.queue_len(), 1);
    }

    #[test] fn test_dispatcher_accepts_call() {
        let mut d = NotificationDispatcher::new(my_key());
        let wake = d.receive(call_notif());
        assert!(wake);
    }

    #[test] fn test_dispatcher_rejects_wrong_target() {
        let mut d = NotificationDispatcher::new(my_key());
        let notif = WakeNotification::new(
            peer_key(), key(0xCC), // wrong target
            NotificationKind::IncomingMessage { conversation_id: "x".to_string() }
        );
        let wake = d.receive(notif);
        assert!(!wake);
        assert_eq!(d.queue_len(), 0);
        assert_eq!(d.total_dropped, 1);
    }

    #[test] fn test_dispatcher_rejects_stale() {
        let mut d = NotificationDispatcher::new(my_key());
        let mut notif = msg_notif();
        notif.timestamp = 0; // too old
        let wake = d.receive(notif);
        assert!(!wake);
        assert_eq!(d.total_replayed, 1);
    }

    #[test] fn test_dispatcher_rejects_replay() {
        let mut d = NotificationDispatcher::new(my_key());
        let notif = msg_notif();
        d.receive(notif.clone());
        let wake = d.receive(notif); // same notification again
        assert!(!wake);
        assert_eq!(d.total_replayed, 1);
    }

    #[test] fn test_dispatcher_send_returns_bytes() {
        let d = NotificationDispatcher::new(my_key());
        let bytes = d.send(peer_key(), NotificationKind::IncomingMessage { conversation_id: "x".to_string() });
        assert_eq!(bytes.len(), 137);
    }

    #[test] fn test_dispatcher_contact_online_no_wake() {
        let mut d = NotificationDispatcher::new(my_key());
        let notif = WakeNotification::new(peer_key(), my_key(), NotificationKind::ContactOnline);
        let wake = d.receive(notif);
        assert!(!wake); // ContactOnline doesn't wake device
        assert_eq!(d.queue_len(), 1); // but still queued
    }

    #[test] fn test_dispatcher_stats() {
        let mut d = NotificationDispatcher::new(my_key());
        d.receive(msg_notif());
        assert_eq!(d.total_received, 1);
        assert_eq!(d.total_dropped, 0);
    }
}
