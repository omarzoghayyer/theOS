// messenger.rs -- theOS Messenger Data Model
//
// Conversations, messages, delivery states, typing indicators.
// Pure logic -- no rendering. Tested on any platform.
//
// Design:
//   - Each conversation is keyed by the contact's IdentityKey
//   - Messages are E2E encrypted via CryptoSession (crypto.rs)
//   - Delivery states: Sending -> Sent -> Delivered -> Read
//   - Typing indicators expire after 5 seconds
//   - Conversation list sorted by last message timestamp

use crate::identity::keypair::IdentityKey;
use std::collections::HashMap;

const MAX_MESSAGES_PER_CONV: usize = 500;
const TYPING_TIMEOUT_SECS:   u64   = 5;
const MAX_MESSAGE_LEN:       usize = 4096;

// -- DeliveryState ------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub enum DeliveryState {
    Sending,    // in outbound queue, not yet ACKed by DHT
    Sent,       // delivered to DHT network
    Delivered,  // ACKed by recipient device
    Read,       // recipient opened the conversation
    Failed,     // delivery failed after retries
}

impl DeliveryState {
    pub fn label(&self) -> &'static str {
        match self {
            DeliveryState::Sending   => "sending",
            DeliveryState::Sent      => "sent",
            DeliveryState::Delivered => "delivered",
            DeliveryState::Read      => "read",
            DeliveryState::Failed    => "failed",
        }
    }

    /// Icon character for UI rendering
    pub fn icon(&self) -> &'static str {
        match self {
            DeliveryState::Sending   => "o",   // hollow circle -- clock
            DeliveryState::Sent      => "v",   // single check
            DeliveryState::Delivered => "vv",  // double check
            DeliveryState::Read      => "VV",  // double check filled (blue)
            DeliveryState::Failed    => "!",   // exclamation
        }
    }
}

// -- Message ------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub enum MessageKind {
    Text,
    // Future: Image, Audio, File, Reaction
}

#[derive(Debug, Clone, PartialEq)]
pub struct Message {
    pub id:        u64,           // monotonic ID within conversation
    pub text:      String,
    pub from_me:   bool,          // true = sent by us, false = received
    pub timestamp: u64,           // unix seconds
    pub state:     DeliveryState, // only meaningful when from_me = true
    pub kind:      MessageKind,
    pub edited:    bool,
}

impl Message {
    pub fn outgoing(id: u64, text: String) -> Result<Self, MessengerError> {
        if text.trim().is_empty() {
            return Err(MessengerError::EmptyMessage);
        }
        if text.len() > MAX_MESSAGE_LEN {
            return Err(MessengerError::MessageTooLong);
        }
        Ok(Self {
            id,
            text: text.trim().to_string(),
            from_me:   true,
            timestamp: now_secs(),
            state:     DeliveryState::Sending,
            kind:      MessageKind::Text,
            edited:    false,
        })
    }

    pub fn incoming(id: u64, text: String) -> Result<Self, MessengerError> {
        if text.trim().is_empty() {
            return Err(MessengerError::EmptyMessage);
        }
        if text.len() > MAX_MESSAGE_LEN {
            return Err(MessengerError::MessageTooLong);
        }
        Ok(Self {
            id,
            text: text.trim().to_string(),
            from_me:   false,
            timestamp: now_secs(),
            state:     DeliveryState::Delivered, // incoming = already delivered
            kind:      MessageKind::Text,
            edited:    false,
        })
    }
}

// -- Conversation -------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct Conversation {
    pub contact_key:    IdentityKey,
    pub contact_name:   String,
    pub messages:       Vec<Message>,
    pub unread_count:   usize,
    pub typing:         bool,         // is the other person typing?
    pub typing_since:   Option<u64>,  // when typing started
    next_id:            u64,
}

impl Conversation {
    pub fn new(contact_key: IdentityKey, contact_name: String) -> Self {
        Self {
            contact_key,
            contact_name,
            messages:     Vec::new(),
            unread_count: 0,
            typing:       false,
            typing_since: None,
            next_id:      1,
        }
    }

    /// Send a message from us. Returns message ID.
    pub fn send(&mut self, text: String) -> Result<u64, MessengerError> {
        let id  = self.next_id;
        let msg = Message::outgoing(id, text)?;
        self.next_id += 1;
        self.messages.push(msg);
        self.trim();
        Ok(id)
    }

    /// Receive an incoming message.
    pub fn receive(&mut self, text: String) -> Result<u64, MessengerError> {
        let id  = self.next_id;
        let msg = Message::incoming(id, text)?;
        self.next_id += 1;
        self.unread_count += 1;
        self.typing = false; // receiving a message clears typing indicator
        self.typing_since = None;
        self.messages.push(msg);
        self.trim();
        Ok(id)
    }

    /// Update delivery state of a sent message.
    pub fn update_delivery(&mut self, id: u64, state: DeliveryState) -> Result<(), MessengerError> {
        let msg = self.messages
            .iter_mut()
            .find(|m| m.id == id && m.from_me)
            .ok_or(MessengerError::MessageNotFound)?;
        msg.state = state;
        Ok(())
    }

    /// Mark all messages as read. Clears unread count.
    pub fn mark_read(&mut self) {
        self.unread_count = 0;
        // Mark all delivered outgoing messages as read
        for msg in self.messages.iter_mut() {
            if msg.from_me && msg.state == DeliveryState::Delivered {
                msg.state = DeliveryState::Read;
            }
        }
    }

    /// Set typing indicator. Expires after TYPING_TIMEOUT_SECS.
    pub fn set_typing(&mut self, is_typing: bool) {
        self.typing       = is_typing;
        self.typing_since = if is_typing { Some(now_secs()) } else { None };
    }

    /// Check and expire typing indicator if too old.
    pub fn tick_typing(&mut self) {
        if let Some(since) = self.typing_since {
            if now_secs() - since > TYPING_TIMEOUT_SECS {
                self.typing       = false;
                self.typing_since = None;
            }
        }
    }

    /// Preview text for conversation list.
    pub fn preview(&self) -> &str {
        self.messages
            .last()
            .map(|m| m.text.as_str())
            .unwrap_or("")
    }

    /// Timestamp of last message.
    pub fn last_timestamp(&self) -> u64 {
        self.messages
            .last()
            .map(|m| m.timestamp)
            .unwrap_or(0)
    }

    pub fn message_count(&self) -> usize { self.messages.len() }

    fn trim(&mut self) {
        if self.messages.len() > MAX_MESSAGES_PER_CONV {
            self.messages.remove(0);
        }
    }
}

// -- ConversationList ---------------------------------------------------------

/// All conversations on the device, sorted by recency.
pub struct ConversationList {
    convs: HashMap<String, Conversation>, // contact key hex -> conversation
}

impl ConversationList {
    pub fn new() -> Self {
        Self { convs: HashMap::new() }
    }

    /// Get or create a conversation with a contact.
    pub fn get_or_create(
        &mut self,
        key:  IdentityKey,
        name: &str,
    ) -> &mut Conversation {
        let hex = key.to_hex();
        self.convs
            .entry(hex)
            .or_insert_with(|| Conversation::new(key, name.to_string()))
    }

    /// Get a conversation by contact key.
    pub fn get(&self, key: &IdentityKey) -> Option<&Conversation> {
        self.convs.get(&key.to_hex())
    }

    pub fn get_mut(&mut self, key: &IdentityKey) -> Option<&mut Conversation> {
        self.convs.get_mut(&key.to_hex())
    }

    /// All conversations sorted by most recent message first.
    pub fn sorted(&self) -> Vec<&Conversation> {
        let mut v: Vec<&Conversation> = self.convs.values().collect();
        v.sort_by(|a, b| b.last_timestamp().cmp(&a.last_timestamp()));
        v
    }

    /// Total unread messages across all conversations.
    pub fn total_unread(&self) -> usize {
        self.convs.values().map(|c| c.unread_count).sum()
    }

    pub fn count(&self) -> usize { self.convs.len() }
}

// -- AnimationState -----------------------------------------------------------
// Drives bubble slide-in and typing dot animations.
// Stored per conversation, consumed by the render layer.

#[derive(Debug, Clone)]
pub struct BubbleAnimation {
    pub message_id:  u64,
    pub started_at:  f64,   // seconds since epoch (f64 for sub-second precision)
    pub duration_ms: u32,   // total animation duration
}

impl BubbleAnimation {
    pub fn new(message_id: u64, now: f64) -> Self {
        Self {
            message_id,
            started_at:  now,
            duration_ms: 280, // 280ms slide-in
        }
    }

    /// Progress from 0.0 (start) to 1.0 (complete).
    pub fn progress(&self, now: f64) -> f64 {
        let elapsed = (now - self.started_at) * 1000.0; // ms
        (elapsed / self.duration_ms as f64).clamp(0.0, 1.0)
    }

    pub fn is_complete(&self, now: f64) -> bool {
        self.progress(now) >= 1.0
    }

    /// Ease-out cubic: fast start, gentle landing.
    pub fn eased_progress(&self, now: f64) -> f64 {
        let t = self.progress(now);
        1.0 - (1.0 - t).powi(3)
    }
}

/// Typing indicator animation -- three pulsing dots.
#[derive(Debug, Clone)]
pub struct TypingAnimation {
    pub started_at: f64,
}

impl TypingAnimation {
    pub fn new(now: f64) -> Self {
        Self { started_at: now }
    }

    /// Phase of dot N (0, 1, 2) at time `now`. Returns 0.0..=1.0.
    /// Dots pulse 200ms apart for a wave effect.
    pub fn dot_phase(&self, dot: u8, now: f64) -> f64 {
        let offset    = dot as f64 * 0.2; // 200ms stagger
        let cycle     = 0.8;              // 800ms full cycle
        let t         = ((now - self.started_at - offset) % cycle) / cycle;
        // Sine wave: 0 = bottom, 1 = top
        (t * std::f64::consts::TAU).sin() * 0.5 + 0.5
    }
}

// -- Error type ---------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub enum MessengerError {
    EmptyMessage,
    MessageTooLong,
    MessageNotFound,
    ConversationNotFound,
}

impl std::fmt::Display for MessengerError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            MessengerError::EmptyMessage         => write!(f, "message cannot be empty"),
            MessengerError::MessageTooLong       => write!(f, "message too long (max 4096 chars)"),
            MessengerError::MessageNotFound      => write!(f, "message not found"),
            MessengerError::ConversationNotFound => write!(f, "conversation not found"),
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
    use crate::identity::keypair::KeyPair;

    fn key() -> IdentityKey { KeyPair::generate().public }

    // DeliveryState

    #[test]
    fn test_delivery_state_labels() {
        assert_eq!(DeliveryState::Sending.label(),   "sending");
        assert_eq!(DeliveryState::Sent.label(),      "sent");
        assert_eq!(DeliveryState::Delivered.label(), "delivered");
        assert_eq!(DeliveryState::Read.label(),      "read");
        assert_eq!(DeliveryState::Failed.label(),    "failed");
    }

    #[test]
    fn test_delivery_state_icons() {
        assert_eq!(DeliveryState::Sent.icon(),      "v");
        assert_eq!(DeliveryState::Delivered.icon(), "vv");
        assert_eq!(DeliveryState::Read.icon(),      "VV");
    }

    // Message

    #[test]
    fn test_outgoing_message_created() {
        let msg = Message::outgoing(1, "hello".to_string()).unwrap();
        assert!(msg.from_me);
        assert_eq!(msg.state, DeliveryState::Sending);
        assert_eq!(msg.text, "hello");
    }

    #[test]
    fn test_incoming_message_created() {
        let msg = Message::incoming(1, "hello".to_string()).unwrap();
        assert!(!msg.from_me);
        assert_eq!(msg.state, DeliveryState::Delivered);
    }

    #[test]
    fn test_empty_message_rejected() {
        assert_eq!(Message::outgoing(1, "".to_string()), Err(MessengerError::EmptyMessage));
        assert_eq!(Message::outgoing(1, "   ".to_string()), Err(MessengerError::EmptyMessage));
    }

    #[test]
    fn test_message_too_long_rejected() {
        let long = "x".repeat(4097);
        assert_eq!(Message::outgoing(1, long), Err(MessengerError::MessageTooLong));
    }

    #[test]
    fn test_message_text_trimmed() {
        let msg = Message::outgoing(1, "  hello  ".to_string()).unwrap();
        assert_eq!(msg.text, "hello");
    }

    // Conversation

    #[test]
    fn test_send_message() {
        let mut conv = Conversation::new(key(), "Sarah".to_string());
        let id = conv.send("hi".to_string()).unwrap();
        assert_eq!(id, 1);
        assert_eq!(conv.message_count(), 1);
    }

    #[test]
    fn test_receive_message_increments_unread() {
        let mut conv = Conversation::new(key(), "Sarah".to_string());
        conv.receive("hi".to_string()).unwrap();
        assert_eq!(conv.unread_count, 1);
        conv.receive("hey".to_string()).unwrap();
        assert_eq!(conv.unread_count, 2);
    }

    #[test]
    fn test_mark_read_clears_unread() {
        let mut conv = Conversation::new(key(), "Sarah".to_string());
        conv.receive("hi".to_string()).unwrap();
        conv.receive("hey".to_string()).unwrap();
        conv.mark_read();
        assert_eq!(conv.unread_count, 0);
    }

    #[test]
    fn test_mark_read_upgrades_delivered_to_read() {
        let mut conv = Conversation::new(key(), "Sarah".to_string());
        let id = conv.send("hi".to_string()).unwrap();
        conv.update_delivery(id, DeliveryState::Delivered).unwrap();
        conv.mark_read();
        let msg = conv.messages.iter().find(|m| m.id == id).unwrap();
        assert_eq!(msg.state, DeliveryState::Read);
    }

    #[test]
    fn test_update_delivery_state() {
        let mut conv = Conversation::new(key(), "Sarah".to_string());
        let id = conv.send("hi".to_string()).unwrap();
        conv.update_delivery(id, DeliveryState::Sent).unwrap();
        let msg = conv.messages.iter().find(|m| m.id == id).unwrap();
        assert_eq!(msg.state, DeliveryState::Sent);
    }

    #[test]
    fn test_update_delivery_unknown_id_fails() {
        let mut conv = Conversation::new(key(), "Sarah".to_string());
        assert_eq!(
            conv.update_delivery(99, DeliveryState::Sent),
            Err(MessengerError::MessageNotFound)
        );
    }

    #[test]
    fn test_ids_increment() {
        let mut conv = Conversation::new(key(), "Sarah".to_string());
        let id1 = conv.send("hi".to_string()).unwrap();
        let id2 = conv.send("hey".to_string()).unwrap();
        assert_eq!(id1, 1);
        assert_eq!(id2, 2);
    }

    #[test]
    fn test_preview_returns_last_message() {
        let mut conv = Conversation::new(key(), "Sarah".to_string());
        conv.send("first".to_string()).unwrap();
        conv.send("last".to_string()).unwrap();
        assert_eq!(conv.preview(), "last");
    }

    #[test]
    fn test_preview_empty_when_no_messages() {
        let conv = Conversation::new(key(), "Sarah".to_string());
        assert_eq!(conv.preview(), "");
    }

    #[test]
    fn test_typing_indicator_set() {
        let mut conv = Conversation::new(key(), "Sarah".to_string());
        assert!(!conv.typing);
        conv.set_typing(true);
        assert!(conv.typing);
        conv.set_typing(false);
        assert!(!conv.typing);
    }

    #[test]
    fn test_receive_clears_typing() {
        let mut conv = Conversation::new(key(), "Sarah".to_string());
        conv.set_typing(true);
        conv.receive("hi".to_string()).unwrap();
        assert!(!conv.typing);
    }

    // ConversationList

    #[test]
    fn test_get_or_create() {
        let mut list = ConversationList::new();
        let k = key();
        list.get_or_create(k.clone(), "Sarah");
        assert_eq!(list.count(), 1);
        list.get_or_create(k.clone(), "Sarah"); // same key -- no duplicate
        assert_eq!(list.count(), 1);
    }

    #[test]
    fn test_total_unread() {
        let mut list = ConversationList::new();
        let k1 = key();
        let k2 = key();
        list.get_or_create(k1.clone(), "Sarah").receive("hi".to_string()).unwrap();
        list.get_or_create(k2.clone(), "Marcus").receive("hey".to_string()).unwrap();
        list.get_or_create(k2.clone(), "Marcus").receive("yo".to_string()).unwrap();
        assert_eq!(list.total_unread(), 3);
    }

    #[test]
    fn test_sorted_by_recency() {
        let mut list = ConversationList::new();
        let k1 = key();
        let k2 = key();
        list.get_or_create(k1.clone(), "Sarah").send("hi".to_string()).unwrap();
        list.get_or_create(k2.clone(), "Marcus").send("hey".to_string()).unwrap();
        let sorted = list.sorted();
        assert_eq!(sorted.len(), 2);
        // Most recent first
        assert!(sorted[0].last_timestamp() >= sorted[1].last_timestamp());
    }

    // BubbleAnimation

    #[test]
    fn test_bubble_animation_progress() {
        let anim = BubbleAnimation::new(1, 0.0);
        assert_eq!(anim.progress(0.0), 0.0);
        assert!((anim.progress(0.14) - 0.5).abs() < 0.01); // ~halfway at 140ms
        assert_eq!(anim.progress(0.28), 1.0); // complete at 280ms
    }

    #[test]
    fn test_bubble_animation_clamps() {
        let anim = BubbleAnimation::new(1, 0.0);
        assert_eq!(anim.progress(100.0), 1.0); // never exceeds 1.0
    }

    #[test]
    fn test_bubble_animation_complete() {
        let anim = BubbleAnimation::new(1, 0.0);
        assert!(!anim.is_complete(0.1));
        assert!(anim.is_complete(0.3));
    }

    #[test]
    fn test_eased_progress_starts_fast() {
        let anim = BubbleAnimation::new(1, 0.0);
        // Ease-out: progress at 25% time should be > 25% eased
        let raw    = anim.progress(0.07);
        let eased  = anim.eased_progress(0.07);
        assert!(eased > raw); // ease-out runs ahead of linear
    }

    // TypingAnimation

    #[test]
    fn test_typing_dots_have_phase_offset() {
        let anim = TypingAnimation::new(0.0);
        let p0 = anim.dot_phase(0, 0.5);
        let p1 = anim.dot_phase(1, 0.5);
        let p2 = anim.dot_phase(2, 0.5);
        // All three dots should have different phases
        assert!(p0 != p1 || p1 != p2);
    }

    #[test]
    fn test_typing_dot_phase_in_range() {
        let anim = TypingAnimation::new(0.0);
        for t in [0.0, 0.2, 0.5, 0.8, 1.0, 2.0] {
            for dot in 0..3 {
                let p = anim.dot_phase(dot, t);
                assert!(p >= 0.0 && p <= 1.0, "phase out of range: {}", p);
            }
        }
    }
}
