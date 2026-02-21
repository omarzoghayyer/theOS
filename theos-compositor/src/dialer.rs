// dialer.rs — theOS Native Dialer
use crate::render::{Surface, TouchState};
use smithay::backend::renderer::gles::GlesFrame;
use smithay::utils::{Physical, Size};

#[derive(Debug, Clone, PartialEq)]
pub enum DialerState { Idle, Dialing, InCall { peer: String }, Incoming { peer: String } }

pub struct Dialer {
    pub state: DialerState,
    pub number_buffer: String,
}

impl Dialer {
    pub fn new() -> Self {
        Self { state: DialerState::Idle, number_buffer: String::new() }
    }

    pub fn key_press(&mut self, key: char) {
        if self.number_buffer.len() < 15 { self.number_buffer.push(key); }
    }

    pub fn backspace(&mut self) { self.number_buffer.pop(); }

    pub fn display_number(&self) -> String {
        let n = &self.number_buffer;
        match n.len() {
            0 => String::new(),
            1..=3 => n.clone(),
            4..=6 => format!("{} {}", &n[..3], &n[3..]),
            _ => format!("{} {} {}", &n[..3], &n[3..6], &n[6..]),
        }
    }

    pub fn keypad() -> &'static [(char, &'static str)] {
        &[('1',""),('2',"ABC"),('3',"DEF"),
          ('4',"GHI"),('5',"JKL"),('6',"MNO"),
          ('7',"PQRS"),('8',"TUV"),('9',"WXYZ"),
          ('*',""),('0',"+"),('#',"")]
    }
}

impl Surface for Dialer {
    fn name(&self) -> &str { "dialer" }
    fn draw(&self, _frame: &mut GlesFrame, _size: Size<i32, Physical>) {
        println!("[dialer] render — number: '{}'", self.display_number());
    }
    fn handle_touch(&mut self, x: f64, y: f64, state: TouchState) {
        if state != TouchState::Down { return; }
        let keypad_top = 2400.0 * 0.45;
        let key_w = 1080.0 / 3.0;
        let key_h = (2400.0 * 0.40) / 4.0;
        if y > keypad_top {
            let col = (x / key_w) as usize;
            let row = ((y - keypad_top) / key_h) as usize;
            let idx = row * 3 + col;
            if let Some(&(k, _)) = Self::keypad().get(idx) {
                self.key_press(k);
            }
        }
    }
}
