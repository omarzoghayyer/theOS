// conversation_screen.rs — Messaging UI surface
//
// Displays a thread of messages with one contact.
// Features: message bubbles, delivery indicators, input field, send on swipe.
//
// MVP scope: Display messages, input, send. Defer: pagination, real-time updates.

use crate::render::{Surface, TouchState};
use smithay::backend::renderer::gles::GlesFrame;
use smithay::utils::{Physical, Size};
use std::collections::VecDeque;

// -- Message UI Model --------------------------------------------------------

#[derive(Debug, Clone)]
pub struct MessageBubble {
    pub id: u64,
    pub text: String,
    pub from_me: bool,
    pub delivery_state: DeliveryState,
    pub timestamp: u64,
    pub appeared_at: f64, // animation timestamp
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DeliveryState {
    Sending,
    Sent,
    Delivered,
    Read,
    Failed,
}

impl DeliveryState {
    pub fn label(&self) -> &'static str {
        match self {
            DeliveryState::Sending => "⏳",
            DeliveryState::Sent => "✓",
            DeliveryState::Delivered => "✓✓",
            DeliveryState::Read => "✓✓",
            DeliveryState::Failed => "✗",
        }
    }

    pub fn is_final(&self) -> bool {
        matches!(
            self,
            DeliveryState::Delivered | DeliveryState::Read | DeliveryState::Failed
        )
    }
}

// -- Conversation Screen State ------------------------------------------------

pub struct ConversationScreen {
    pub contact_name: String,
    pub contact_key: String,
    pub messages: VecDeque<MessageBubble>,
    pub input_buf: String,
    pub is_typing: bool,
    pub last_message_id: u64,
}

impl ConversationScreen {
    pub fn new(contact_name: String, contact_key: String) -> Self {
        Self {
            contact_name,
            contact_key,
            messages: VecDeque::new(),
            input_buf: String::new(),
            is_typing: false,
            last_message_id: 0,
        }
    }

    pub fn add_message(
        &mut self,
        text: String,
        from_me: bool,
        delivery_state: DeliveryState,
        timestamp: u64,
    ) {
        self.last_message_id += 1;
        let bubble = MessageBubble {
            id: self.last_message_id,
            text,
            from_me,
            delivery_state,
            timestamp,
            appeared_at: now_secs_f64(),
        };

        // Keep last 50 messages (MVP: no pagination)
        if self.messages.len() >= 50 {
            self.messages.pop_front();
        }

        self.messages.push_back(bubble);
    }

    pub fn type_char(&mut self, ch: char) {
        if self.input_buf.len() < 1000 {
            self.input_buf.push(ch);
            self.is_typing = true;
        }
    }

    pub fn backspace(&mut self) {
        self.input_buf.pop();
        self.is_typing = !self.input_buf.is_empty();
    }

    pub fn clear_input(&mut self) {
        self.input_buf.clear();
        self.is_typing = false;
    }

    pub fn input_is_empty(&self) -> bool {
        self.input_buf.trim().is_empty()
    }

    pub fn submit_message(&mut self) -> Option<String> {
        if self.input_is_empty() {
            return None;
        }

        let msg = self.input_buf.trim().to_string();
        self.add_message(msg.clone(), true, DeliveryState::Sending, now_secs());
        self.clear_input();
        Some(msg)
    }

    pub fn update_delivery_state(&mut self, message_id: u64, new_state: DeliveryState) {
        if let Some(msg) = self.messages.iter_mut().find(|m| m.id == message_id) {
            msg.delivery_state = new_state;
        }
    }

    pub fn unread_count(&self) -> usize {
        self.messages
            .iter()
            .filter(|m| !m.from_me && m.delivery_state != DeliveryState::Read)
            .count()
    }
}

// -- Surface Implementation ---------------------------------------------------

impl Surface for ConversationScreen {
    fn name(&self) -> &str {
        "conversation"
    }

    fn draw(&self, frame: &mut GlesFrame, _size: Size<i32, Physical>) {
        // Delegate to rendering module
        crate::conversation_render::render(frame, self);
    }

    fn handle_touch(&mut self, x: f64, y: f64, state: TouchState) {
        if state != TouchState::Down {
            return;
        }

        let screen_h = 2400.0;
        let input_top = screen_h * 0.85;

        // Tap in input field area
        if y > input_top {
            self.is_typing = true;
        }
    }
}

// -- Utilities ----------------------------------------------------------------

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn now_secs_f64() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_conversation() {
        let conv = ConversationScreen::new("alice".to_string(), "key123".to_string());
        assert_eq!(conv.contact_name, "alice");
        assert!(conv.messages.is_empty());
    }

    #[test]
    fn test_add_message() {
        let mut conv = ConversationScreen::new("alice".to_string(), "key123".to_string());
        conv.add_message(
            "hello".to_string(),
            true,
            DeliveryState::Sent,
            now_secs(),
        );
        assert_eq!(conv.messages.len(), 1);
        assert_eq!(conv.messages[0].text, "hello");
    }

    #[test]
    fn test_type_char() {
        let mut conv = ConversationScreen::new("alice".to_string(), "key123".to_string());
        conv.type_char('h');
        conv.type_char('i');
        assert_eq!(conv.input_buf, "hi");
    }

    #[test]
    fn test_backspace() {
        let mut conv = ConversationScreen::new("alice".to_string(), "key123".to_string());
        conv.input_buf = "hello".to_string();
        conv.backspace();
        assert_eq!(conv.input_buf, "hell");
    }

    #[test]
    fn test_submit_message() {
        let mut conv = ConversationScreen::new("alice".to_string(), "key123".to_string());
        conv.input_buf = "hello alice".to_string();
        let result = conv.submit_message();
        assert_eq!(result, Some("hello alice".to_string()));
        assert!(conv.input_buf.is_empty());
        assert_eq!(conv.messages.len(), 1);
    }

    #[test]
    fn test_delivery_state_labels() {
        assert_eq!(DeliveryState::Sending.label(), "⏳");
        assert_eq!(DeliveryState::Sent.label(), "✓");
        assert_eq!(DeliveryState::Delivered.label(), "✓✓");
    }

    #[test]
    fn test_max_50_messages() {
        let mut conv = ConversationScreen::new("alice".to_string(), "key123".to_string());
        for i in 0..60 {
            conv.add_message(
                format!("msg {}", i),
                true,
                DeliveryState::Sent,
                now_secs(),
            );
        }
        assert!(conv.messages.len() <= 50);
    }

    #[test]
    fn test_unread_count() {
        let mut conv = ConversationScreen::new("alice".to_string(), "key123".to_string());
        conv.add_message(
            "from alice".to_string(),
            false,
            DeliveryState::Delivered,
            now_secs(),
        );
        conv.add_message(
            "from me".to_string(),
            true,
            DeliveryState::Sent,
            now_secs(),
        );
        assert_eq!(conv.unread_count(), 1);
    }
}
