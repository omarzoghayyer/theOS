// ai_shell.rs -- theOS AI Shell
//
// The primary user-facing surface. Replaces the traditional home screen
// as the first thing the user sees after unlock.
//
// Intent routing is hardcoded for the demo. Claude API replaces this in Phase 5.

use crate::render::{Surface, TouchState};
use smithay::backend::renderer::gles::GlesFrame;
use smithay::utils::{Physical, Size};

#[derive(Debug, Clone, PartialEq)]
pub enum Intent {
    Call { contact: String },
    ShowConnection,
    ShowMessages,
    ShowContacts,
    ShowIdentity,
    ShowSettings,
    OpenApp { name: String },
    Unknown { raw: String },
}

impl Intent {
    pub fn parse(input: &str) -> Self {
        let s = input.trim().to_lowercase();

        if s.starts_with("call ") || s.starts_with("ring ") || s.starts_with("dial ") {
            let contact = s
                .trim_start_matches("call ")
                .trim_start_matches("ring ")
                .trim_start_matches("dial ")
                .trim()
                .to_string();
            return Intent::Call { contact };
        }

        if s.contains("connection") || s.contains("network") || s.contains("satellite")
            || s.contains("signal") || s.contains("status")
        {
            return Intent::ShowConnection;
        }

        if s.contains("message") || s.contains("chat") || s.contains("inbox") {
            return Intent::ShowMessages;
        }

        if s.contains("identity") || s.contains("qr") || s.contains("my key")
            || s.contains("who am i")
        {
            return Intent::ShowIdentity;
        }

        if s.contains("contact") || s.contains("people") || s.contains("who") {
            return Intent::ShowContacts;
        }

        if s.contains("setting") || s.contains("config") || s.contains("preference") {
            return Intent::ShowSettings;
        }

        if s.starts_with("open ") {
            let name = s.trim_start_matches("open ").trim().to_string();
            return Intent::OpenApp { name };
        }

        Intent::Unknown { raw: input.trim().to_string() }
    }

    pub fn response(&self, sys: &AiShellState) -> String {
        match self {
            Intent::Call { contact } =>
                format!("Calling {}...", contact),
            Intent::ShowConnection =>
                format!(
                    "Satellite: {} - {}ms - {}% signal\nDHT peers: {} active\nIdentity: @{}",
                    sys.satellite_link, sys.latency_ms, sys.signal_quality,
                    sys.dht_peers, sys.handle,
                ),
            Intent::ShowMessages =>
                "Opening your messages.".to_string(),
            Intent::ShowContacts =>
                format!("{} contacts connected.", sys.contact_count),
            Intent::ShowIdentity =>
                format!(
                    "Your handle: @{}\nPublic key: {}...\nTap to show QR code.",
                    sys.handle,
                    &sys.pubkey_hex[..16.min(sys.pubkey_hex.len())],
                ),
            Intent::ShowSettings =>
                "Opening settings.".to_string(),
            Intent::OpenApp { name } =>
                format!("Opening {} in privacy sandbox...", name),
            Intent::Unknown { raw } =>
                format!(
                    "I'm not sure how to help with \"{}\". Try: call, messages, connection status, or open <app>.",
                    raw
                ),
        }
    }

    pub fn navigates_to(&self) -> Option<&'static str> {
        match self {
            Intent::Call { .. }  => Some("call"),
            Intent::ShowMessages => Some("messages"),
            Intent::ShowSettings => Some("settings"),
            _                    => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct AiShellState {
    pub handle:         String,
    pub pubkey_hex:     String,
    pub satellite_link: String,
    pub latency_ms:     u32,
    pub signal_quality: u8,
    pub dht_peers:      usize,
    pub contact_count:  usize,
    pub battery_pct:    u8,
    pub charging:       bool,
}

impl Default for AiShellState {
    fn default() -> Self {
        Self {
            handle:         "you".to_string(),
            pubkey_hex:     "0000000000000000000000000000000000000000000000000000000000000000".to_string(),
            satellite_link: "Starlink".to_string(),
            latency_ms:     0,
            signal_quality: 0,
            dht_peers:      0,
            contact_count:  0,
            battery_pct:    100,
            charging:       false,
        }
    }
}

#[derive(Debug, Clone)]
pub struct AiMessage {
    pub text:      String,
    pub from_user: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum InputMode {
    Idle,
    Typing,
}

#[derive(Debug, Clone, PartialEq)]
pub enum AiShellNav {
    None,
    GoToCall { contact: String },
    GoToMessages,
    GoToSettings,
    GoToTraditionalHome,
}

pub struct AiShell {
    pub messages:      Vec<AiMessage>,
    pub input_buf:     String,
    pub input_mode:    InputMode,
    pub sys:           AiShellState,
    pub scroll_offset: i32,
}

impl AiShell {
    pub fn new() -> Self {
        let mut s = Self {
            messages:      Vec::new(),
            input_buf:     String::new(),
            input_mode:    InputMode::Idle,
            sys:           AiShellState::default(),
            scroll_offset: 0,
        };
        s.push_ai("What do you want to do?".to_string());
        s
    }

    pub fn update_state(&mut self, state: AiShellState) {
        self.sys = state;
    }

    fn push_user(&mut self, text: String) {
        self.messages.push(AiMessage { text, from_user: true });
        self.trim_history();
    }

    fn push_ai(&mut self, text: String) {
        self.messages.push(AiMessage { text, from_user: false });
        self.trim_history();
    }

    fn trim_history(&mut self) {
        if self.messages.len() > 30 {
            self.messages.remove(0);
        }
    }

    pub fn submit_input(&mut self) -> AiShellNav {
        let raw = self.input_buf.trim().to_string();
        if raw.is_empty() {
            self.input_mode = InputMode::Idle;
            return AiShellNav::None;
        }

        self.push_user(raw.clone());
        self.input_buf.clear();
        self.input_mode = InputMode::Idle;

        let intent = Intent::parse(&raw);
        let reply  = intent.response(&self.sys);
        let nav    = intent.navigates_to();

        self.push_ai(reply);

        match nav {
            Some("call") => {
                let contact = match &intent {
                    Intent::Call { contact } => contact.clone(),
                    _ => String::new(),
                };
                AiShellNav::GoToCall { contact }
            }
            Some("messages") => AiShellNav::GoToMessages,
            Some("settings") => AiShellNav::GoToSettings,
            _ => AiShellNav::None,
        }
    }

    pub fn type_char(&mut self, ch: char) {
        if self.input_mode == InputMode::Typing {
            self.input_buf.push(ch);
        }
    }

    pub fn backspace(&mut self) {
        if self.input_mode == InputMode::Typing {
            self.input_buf.pop();
        }
    }
}

impl Surface for AiShell {
    fn name(&self) -> &str { "ai_shell" }

    fn draw(&self, _frame: &mut GlesFrame, _size: Size<i32, Physical>) {
        println!(
            "[ai_shell] render -- {} messages, mode: {:?}, buf: '{}'",
            self.messages.len(), self.input_mode, self.input_buf,
        );
    }

    fn handle_touch(&mut self, _x: f64, y: f64, state: TouchState) {
        if state != TouchState::Down { return; }
        let screen_h = 2280.0_f64;
        if y > screen_h * 0.85 {
            self.input_mode = InputMode::Typing;
            println!("[ai_shell] input activated");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn default_state() -> AiShellState { AiShellState::default() }

    #[test]
    fn test_intent_call() {
        assert_eq!(Intent::parse("call Sarah"), Intent::Call { contact: "sarah".to_string() });
    }

    #[test]
    fn test_intent_call_ring_prefix() {
        assert_eq!(Intent::parse("ring Marcus"), Intent::Call { contact: "marcus".to_string() });
    }

    #[test]
    fn test_intent_call_dial_prefix() {
        assert_eq!(Intent::parse("dial 555"), Intent::Call { contact: "555".to_string() });
    }

    #[test]
    fn test_intent_connection() {
        assert_eq!(Intent::parse("show connection"), Intent::ShowConnection);
        assert_eq!(Intent::parse("satellite status"), Intent::ShowConnection);
        assert_eq!(Intent::parse("network"), Intent::ShowConnection);
        assert_eq!(Intent::parse("signal"), Intent::ShowConnection);
    }

    #[test]
    fn test_intent_messages() {
        assert_eq!(Intent::parse("messages"), Intent::ShowMessages);
        assert_eq!(Intent::parse("show chat"), Intent::ShowMessages);
    }

    #[test]
    fn test_intent_contacts() {
        assert_eq!(Intent::parse("contacts"), Intent::ShowContacts);
        assert_eq!(Intent::parse("show people"), Intent::ShowContacts);
    }

    #[test]
    fn test_intent_identity() {
        assert_eq!(Intent::parse("show identity"), Intent::ShowIdentity);
        assert_eq!(Intent::parse("show qr"), Intent::ShowIdentity);
        assert_eq!(Intent::parse("who am i"), Intent::ShowIdentity);
    }

    #[test]
    fn test_intent_settings() {
        assert_eq!(Intent::parse("settings"), Intent::ShowSettings);
        assert_eq!(Intent::parse("open config"), Intent::ShowSettings);
    }

    #[test]
    fn test_intent_open_app() {
        assert_eq!(Intent::parse("open instagram"), Intent::OpenApp { name: "instagram".to_string() });
        assert_eq!(Intent::parse("open spotify"),   Intent::OpenApp { name: "spotify".to_string() });
    }

    #[test]
    fn test_intent_unknown() {
        assert_eq!(
            Intent::parse("do something weird"),
            Intent::Unknown { raw: "do something weird".to_string() }
        );
    }

    #[test]
    fn test_connection_response_contains_satellite() {
        let mut s = default_state();
        s.satellite_link = "Starlink".to_string();
        s.latency_ms     = 47;
        s.signal_quality = 82;
        s.dht_peers      = 12;
        s.handle         = "omar".to_string();
        let resp = Intent::ShowConnection.response(&s);
        assert!(resp.contains("Starlink"));
        assert!(resp.contains("47ms"));
        assert!(resp.contains("82%"));
        assert!(resp.contains("12 active"));
        assert!(resp.contains("@omar"));
    }

    #[test]
    fn test_identity_response_contains_handle() {
        let mut s = default_state();
        s.handle     = "omar".to_string();
        s.pubkey_hex = "abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890".to_string();
        let resp = Intent::ShowIdentity.response(&s);
        assert!(resp.contains("@omar"));
        assert!(resp.contains("abcdef123456789"));
    }

    #[test]
    fn test_call_response() {
        let resp = Intent::Call { contact: "sarah".to_string() }.response(&default_state());
        assert!(resp.contains("sarah"));
    }

    #[test]
    fn test_open_app_response() {
        let resp = Intent::OpenApp { name: "instagram".to_string() }.response(&default_state());
        assert!(resp.contains("instagram"));
        assert!(resp.contains("privacy sandbox"));
    }

    #[test]
    fn test_unknown_response_echoes_input() {
        let resp = Intent::Unknown { raw: "do something weird".to_string() }.response(&default_state());
        assert!(resp.contains("do something weird"));
    }

    #[test]
    fn test_call_navigates_to_call() {
        assert_eq!(Intent::Call { contact: "sarah".to_string() }.navigates_to(), Some("call"));
    }

    #[test]
    fn test_messages_navigates_to_messages() {
        assert_eq!(Intent::ShowMessages.navigates_to(), Some("messages"));
    }

    #[test]
    fn test_connection_does_not_navigate() {
        assert_eq!(Intent::ShowConnection.navigates_to(), None);
    }

    #[test]
    fn test_identity_does_not_navigate() {
        assert_eq!(Intent::ShowIdentity.navigates_to(), None);
    }

    #[test]
    fn test_greeting_on_new() {
        let shell = AiShell::new();
        assert_eq!(shell.messages.len(), 1);
        assert!(!shell.messages[0].from_user);
        assert!(shell.messages[0].text.contains("What do you"));
    }

    #[test]
    fn test_submit_adds_user_and_ai_messages() {
        let mut shell    = AiShell::new();
        shell.input_buf  = "show connection".to_string();
        shell.input_mode = InputMode::Typing;
        shell.submit_input();
        assert_eq!(shell.messages.len(), 3);
        assert!(shell.messages[1].from_user);
        assert!(!shell.messages[2].from_user);
    }

    #[test]
    fn test_submit_empty_does_nothing() {
        let mut shell    = AiShell::new();
        shell.input_buf  = "   ".to_string();
        shell.input_mode = InputMode::Typing;
        shell.submit_input();
        assert_eq!(shell.messages.len(), 1);
    }

    #[test]
    fn test_submit_call_returns_call_nav() {
        let mut shell    = AiShell::new();
        shell.input_buf  = "call sarah".to_string();
        shell.input_mode = InputMode::Typing;
        let nav = shell.submit_input();
        assert_eq!(nav, AiShellNav::GoToCall { contact: "sarah".to_string() });
    }

    #[test]
    fn test_submit_connection_returns_no_nav() {
        let mut shell    = AiShell::new();
        shell.input_buf  = "show connection".to_string();
        shell.input_mode = InputMode::Typing;
        let nav = shell.submit_input();
        assert_eq!(nav, AiShellNav::None);
    }

    #[test]
    fn test_history_trimmed_at_30() {
        let mut shell = AiShell::new();
        for i in 0..35 {
            shell.input_buf  = format!("message {}", i);
            shell.input_mode = InputMode::Typing;
            shell.submit_input();
        }
        assert!(shell.messages.len() <= 30);
    }

    #[test]
    fn test_type_char_appends() {
        let mut shell    = AiShell::new();
        shell.input_mode = InputMode::Typing;
        shell.type_char('h');
        shell.type_char('i');
        assert_eq!(shell.input_buf, "hi");
    }

    #[test]
    fn test_backspace_removes_char() {
        let mut shell    = AiShell::new();
        shell.input_mode = InputMode::Typing;
        shell.input_buf  = "hello".to_string();
        shell.backspace();
        assert_eq!(shell.input_buf, "hell");
    }

    #[test]
    fn test_type_char_ignored_when_idle() {
        let mut shell = AiShell::new();
        shell.type_char('x');
        assert_eq!(shell.input_buf, "");
    }

    #[test]
    fn test_update_state() {
        let mut shell   = AiShell::new();
        let mut state   = AiShellState::default();
        state.handle    = "omar".to_string();
        state.dht_peers = 7;
        shell.update_state(state);
        assert_eq!(shell.sys.handle, "omar");
        assert_eq!(shell.sys.dht_peers, 7);
    }
}

// Claude API integration (added for AI-Native OS)
// Gracefully falls back to heuristic parsing if unavailable

#[cfg(feature = "claude")]
mod claude_integration {
    use std::env;
    
    pub fn try_init_claude() -> Option<String> {
        env::var("ANTHROPIC_API_KEY").ok()
    }
}

// Claude API integration (added for AI-Native OS)
// Gracefully falls back to heuristic parsing if unavailable

#[cfg(feature = "claude")]
mod claude_integration {
    use std::env;
    
    pub fn try_init_claude() -> Option<String> {
        env::var("ANTHROPIC_API_KEY").ok()
    }
}
