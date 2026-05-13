// wake_word.rs -- theOS Wake Word & Voice Command Engine
//
// Always-on voice interface. No buttons. No visible input.
// The user speaks; the OS listens and acts.
//
// Wake word: "Hey OS" (and variants)
// On device: tiny keyword model on ADSP (~5mW, fastrpc bridge)
// In tests:  stub -- inject wake events directly
//
// Security: audio never leaves the device. ADSP model is on-chip only.
// Audio is never recorded -- only transcribed text is kept in history.

use std::collections::VecDeque;
use std::time::Instant;

pub const WAKE_WORD: &str = "hey os";
pub const WAKE_WORD_VARIANTS: &[&str] = &[
    "hey os", "hey o.s.", "hey the os", "wake up", "hey theos",
];
pub const MAX_COMMAND_LEN:      usize = 500;
pub const LISTEN_TIMEOUT_SECS:  u64   = 8;
pub const COMMAND_HISTORY_LEN:  usize = 50;

// -- WakeState ----------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub enum WakeState {
    Sleeping,
    Listening  { started_at: u64 },
    Processing { command: String },
    Responding,
}

impl WakeState {
    pub fn label(&self) -> &str {
        match self {
            WakeState::Sleeping        => "sleeping",
            WakeState::Listening {..}  => "listening",
            WakeState::Processing {..} => "processing",
            WakeState::Responding      => "responding",
        }
    }
    pub fn is_sleeping(&self)   -> bool { matches!(self, WakeState::Sleeping) }
    pub fn is_listening(&self)  -> bool { matches!(self, WakeState::Listening {..}) }
    pub fn is_processing(&self) -> bool { matches!(self, WakeState::Processing {..}) }
    pub fn is_responding(&self) -> bool { matches!(self, WakeState::Responding) }
    pub fn is_active(&self)     -> bool { !self.is_sleeping() }
}

// -- CommandRecord ------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct CommandRecord {
    pub raw:       String,
    pub stripped:  String,
    pub timestamp: u64,
    pub executed:  bool,
}

impl CommandRecord {
    pub fn new(raw: String, timestamp: u64) -> Self {
        let stripped = strip_wake_word(&raw);
        Self { raw, stripped, timestamp, executed: false }
    }
}

// -- WakeEngine ---------------------------------------------------------------

pub struct WakeEngine {
    pub state:       WakeState,
    pub history:     VecDeque<CommandRecord>,
    pub total_wakes: u64,
    pub false_wakes: u64,
    listen_start:    Option<Instant>,
}

impl WakeEngine {
    pub fn new() -> Self {
        Self {
            state:        WakeState::Sleeping,
            history:      VecDeque::new(),
            total_wakes:  0,
            false_wakes:  0,
            listen_start: None,
        }
    }

    pub fn on_wake_detected(&mut self) {
        if self.state.is_sleeping() {
            self.total_wakes += 1;
            self.state = WakeState::Listening { started_at: now_secs() };
            self.listen_start = Some(Instant::now());
            println!("[wake] detected -- listening (#{}) ", self.total_wakes);
        }
    }

    pub fn on_command_received(&mut self, raw: String) {
        if self.state.is_listening() {
            let record  = CommandRecord::new(raw.clone(), now_secs());
            let command = record.stripped.clone();
            if command.is_empty() {
                println!("[wake] empty command -- sleeping");
                self.false_wakes += 1;
                self.state = WakeState::Sleeping;
                return;
            }
            println!("[wake] \"{}\" -> \"{}\"", raw, command);
            self.history.push_back(record);
            if self.history.len() > COMMAND_HISTORY_LEN {
                self.history.pop_front();
            }
            self.state = WakeState::Processing { command };
            self.listen_start = None;
        }
    }

    pub fn on_command_executed(&mut self) {
        if let WakeState::Processing { .. } = &self.state {
            if let Some(rec) = self.history.back_mut() { rec.executed = true; }
            self.state = WakeState::Responding;
        }
    }

    pub fn on_response_complete(&mut self) {
        if self.state.is_responding() {
            self.state = WakeState::Sleeping;
            println!("[wake] response complete -- sleeping");
        }
    }

    pub fn tick(&mut self) {
        if let WakeState::Listening { .. } = &self.state {
            if let Some(start) = self.listen_start {
                if start.elapsed().as_secs() >= LISTEN_TIMEOUT_SECS {
                    println!("[wake] timeout -- sleeping");
                    self.false_wakes += 1;
                    self.state = WakeState::Sleeping;
                    self.listen_start = None;
                }
            }
        }
    }

    pub fn current_command(&self) -> Option<&str> {
        if let WakeState::Processing { command } = &self.state {
            Some(command)
        } else {
            None
        }
    }

    pub fn force_sleep(&mut self) {
        self.state = WakeState::Sleeping;
        self.listen_start = None;
    }

    pub fn recent_commands(&self, n: usize) -> Vec<&CommandRecord> {
        self.history.iter().rev().take(n).collect()
    }

    pub fn command_count(&self) -> usize { self.history.len() }
}

// -- Wake word stripping ------------------------------------------------------

pub fn strip_wake_word(raw: &str) -> String {
    let lower = raw.trim().to_lowercase();
    for variant in WAKE_WORD_VARIANTS {
        if lower.starts_with(variant) {
            let rest = &raw[variant.len()..];
            let stripped = rest.trim_start_matches(|c: char| {
                c == ',' || c == '.' || c == ':' || c.is_whitespace()
            });
            return stripped.trim().to_string();
        }
    }
    raw.trim().to_string()
}

pub fn contains_wake_word(input: &str) -> bool {
    let lower = input.to_lowercase();
    WAKE_WORD_VARIANTS.iter().any(|w| lower.contains(w))
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

    fn engine() -> WakeEngine { WakeEngine::new() }

    #[test] fn test_starts_sleeping() { assert!(engine().state.is_sleeping()); }
    #[test] fn test_labels() {
        assert_eq!(WakeState::Sleeping.label(), "sleeping");
        assert_eq!(WakeState::Listening { started_at: 0 }.label(), "listening");
        assert_eq!(WakeState::Processing { command: "x".to_string() }.label(), "processing");
        assert_eq!(WakeState::Responding.label(), "responding");
    }
    #[test] fn test_sleeping_not_active() { assert!(!WakeState::Sleeping.is_active()); }
    #[test] fn test_listening_is_active() { assert!(WakeState::Listening { started_at: 0 }.is_active()); }
    #[test] fn test_wake_transitions_to_listening() {
        let mut e = engine(); e.on_wake_detected(); assert!(e.state.is_listening());
    }
    #[test] fn test_wake_increments_counter() {
        let mut e = engine(); e.on_wake_detected(); assert_eq!(e.total_wakes, 1);
    }
    #[test] fn test_double_wake_ignored() {
        let mut e = engine(); e.on_wake_detected(); e.on_wake_detected(); assert_eq!(e.total_wakes, 1);
    }
    #[test] fn test_command_to_processing() {
        let mut e = engine(); e.on_wake_detected();
        e.on_command_received("hey os message sarah".to_string());
        assert!(e.state.is_processing());
    }
    #[test] fn test_command_stripped() {
        let mut e = engine(); e.on_wake_detected();
        e.on_command_received("hey os message sarah".to_string());
        assert_eq!(e.current_command(), Some("message sarah"));
    }
    #[test] fn test_empty_command_sleeps() {
        let mut e = engine(); e.on_wake_detected();
        e.on_command_received("hey os".to_string());
        assert!(e.state.is_sleeping()); assert_eq!(e.false_wakes, 1);
    }
    #[test] fn test_executed_to_responding() {
        let mut e = engine(); e.on_wake_detected();
        e.on_command_received("call marcus".to_string());
        e.on_command_executed(); assert!(e.state.is_responding());
    }
    #[test] fn test_response_complete_to_sleep() {
        let mut e = engine(); e.on_wake_detected();
        e.on_command_received("call marcus".to_string());
        e.on_command_executed(); e.on_response_complete();
        assert!(e.state.is_sleeping());
    }
    #[test] fn test_history_records() {
        let mut e = engine(); e.on_wake_detected();
        e.on_command_received("hey os message sarah".to_string());
        assert_eq!(e.command_count(), 1);
        assert_eq!(e.history.back().unwrap().stripped, "message sarah");
    }
    #[test] fn test_force_sleep() {
        let mut e = engine(); e.on_wake_detected(); e.force_sleep();
        assert!(e.state.is_sleeping());
    }
    #[test] fn test_recent_commands() {
        let mut e = engine();
        for cmd in &["call marcus", "message sarah", "satellite status"] {
            e.on_wake_detected();
            e.on_command_received(cmd.to_string());
            e.on_command_executed();
            e.on_response_complete();
        }
        let recent = e.recent_commands(2);
        assert_eq!(recent.len(), 2);
        assert_eq!(recent[0].stripped, "satellite status");
    }
    #[test] fn test_strip_hey_os() {
        assert_eq!(strip_wake_word("hey os message sarah"), "message sarah");
    }
    #[test] fn test_strip_with_comma() {
        assert_eq!(strip_wake_word("hey os, call marcus"), "call marcus");
    }
    #[test] fn test_strip_wake_up() {
        assert_eq!(strip_wake_word("wake up"), "");
    }
    #[test] fn test_strip_passthrough() {
        assert_eq!(strip_wake_word("message sarah"), "message sarah");
    }
    #[test] fn test_strip_hey_theos() {
        assert_eq!(strip_wake_word("hey theos open instagram"), "open instagram");
    }
    #[test] fn test_strip_case_insensitive() {
        assert_eq!(strip_wake_word("HEY OS call marcus"), "call marcus");
    }
    #[test] fn test_contains_wake_word_true() {
        assert!(contains_wake_word("hey os what time is it"));
    }
    #[test] fn test_contains_wake_word_false() {
        assert!(!contains_wake_word("what time is it"));
    }
    #[test] fn test_full_flow() {
        let mut e = engine();
        assert!(e.state.is_sleeping());
        e.on_wake_detected();
        assert!(e.state.is_listening());
        e.on_command_received("hey os, satellite status".to_string());
        assert_eq!(e.current_command(), Some("satellite status"));
        e.on_command_executed();
        assert!(e.state.is_responding());
        e.on_response_complete();
        assert!(e.state.is_sleeping());
        assert_eq!(e.total_wakes, 1);
        assert_eq!(e.false_wakes, 0);
    }
}
