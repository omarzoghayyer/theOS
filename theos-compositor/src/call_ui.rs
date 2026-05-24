// call_ui.rs -- theOS Identity-Based Call Surface
//
// The active-call screen. Shown when calling or in a call with a contact.
//
// theOS principle: calls are placed to a CONTACT IDENTITY (Ed25519 key /
// handle), never a phone number. There is no number to dial. This surface
// reflects that -- it shows who you're talking to by handle and key
// fingerprint, the encryption status, and the satellite link quality.

use crate::render::{Surface, TouchState};
use smithay::backend::renderer::gles::GlesFrame;
use smithay::utils::{Physical, Size};

// -- Call lifecycle (UI view) -------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub enum CallUiState {
    Connecting,
    Ringing,
    Active { started_at: u64 },
    Incoming,
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
    Excellent,
    Good,
    Fair,
    Poor,
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
    pub handle:       Option<String>,
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

    pub fn from_key_hex(handle: Option<String>, key_hex: &str) -> Self {
        let fp = key_hex.chars().take(8).collect::<String>();
        Self::new(handle, fp)
    }

    pub fn display_name(&self) -> String {
        match &self.handle {
            Some(h) => h.clone(),
            None    => format!("key:{}", self.key_fp),
        }
    }

    pub fn link_quality(&self) -> LinkQuality {
        LinkQuality::from_latency(self.latency_ms)
    }

    pub fn on_ringing(&mut self) {
        if matches!(self.state, CallUiState::Connecting) {
            self.state = CallUiState::Ringing;
        }
    }

    pub fn on_answered(&mut self, now: u64) {
        self.state = CallUiState::Active { started_at: now };
    }

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

    pub fn duration_secs(&self, now: u64) -> u64 {
        match &self.state {
            CallUiState::Active { started_at } => now.saturating_sub(*started_at),
            _ => 0,
        }
    }
}

impl Surface for CallSurface {
    fn name(&self) -> &str { "call" }

    fn draw(&self, frame: &mut GlesFrame, _size: Size<i32, Physical>) {
        let now = now_secs();
        crate::call_ui_render::render(frame, self, now);
    }

    fn handle_touch(&mut self, x: f64, _y: f64, state: TouchState) {
        if state != TouchState::Down { return; }

        let screen_h = 2400.0;
        let screen_w = 1080.0;
        let control_top = screen_h * 0.80;

        if _y < control_top { return; }

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
    fn test_toggle_mute() {
        let mut s = surface();
        assert!(!s.muted);
        assert!(s.toggle_mute());
        assert!(s.muted);
    }

    #[test]
    fn test_toggle_speaker() {
        let mut s = surface();
        assert!(!s.speaker_on);
        assert!(s.toggle_speaker());
        assert!(s.speaker_on);
    }

    #[test]
    fn test_format_duration_under_minute() {
        assert_eq!(format_duration(42), "00:42");
    }

    #[test]
    fn test_link_quality_excellent() {
        assert_eq!(LinkQuality::from_latency(400), LinkQuality::Excellent);
    }
}
