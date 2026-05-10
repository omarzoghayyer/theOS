// assistant.rs — theOS AI Assistant Surface
use crate::render::{Surface, TouchState};
use smithay::backend::renderer::gles::GlesFrame;
use smithay::utils::{Physical, Size};

#[derive(Debug, Clone)]
pub struct Message { pub text: String, pub from_user: bool }

pub struct Assistant {
    pub messages: Vec<Message>,
    pub listening: bool,
}

impl Assistant {
    pub fn new() -> Self {
        Self {
            messages: vec![Message {
                text: "Hi, I'm theOS AI. I can help with calls, satellite status, and network management.".to_string(),
                from_user: false,
            }],
            listening: false,
        }
    }

    pub fn add_message(&mut self, text: String, from_user: bool) {
        self.messages.push(Message { text, from_user });
        if self.messages.len() > 50 { self.messages.remove(0); }
    }
}

impl Surface for Assistant {
    fn name(&self) -> &str { "assistant" }
    fn draw(&self, _frame: &mut GlesFrame, _size: Size<i32, Physical>) {
        println!("[assistant] render — {} messages, listening: {}",
            self.messages.len(), self.listening);
    }
    fn handle_touch(&mut self, x: f64, y: f64, state: TouchState) {
        if state != TouchState::Down { return; }
        if y > 2400.0 * 0.85 {
            self.listening = !self.listening;
            println!("[assistant] listening: {}", self.listening);
        }
    }
}
