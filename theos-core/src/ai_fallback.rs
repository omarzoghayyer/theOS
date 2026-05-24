// ai_fallback.rs — Graceful degradation for AI intent parsing
//
// If Claude API unavailable (network, rate limit, timeout), fall back to
// local heuristic parser. User never notices — system degrades silently.

#[derive(Debug, Clone, PartialEq)]
pub enum Intent {
    Call { contact: String },
    Message { contact: String, text: String },
    CallStatus,
    MessageStatus,
    Unknown { raw: String },
}

impl Intent {
    /// Parse intent from raw user input using heuristic rules.
    /// Used when Claude API is unavailable.
    pub fn from_heuristic(input: &str) -> Self {
        let lower = input.to_lowercase();

        // Pattern: "call X"
        if lower.starts_with("call ") {
            let contact = input[5..].trim().to_string();
            return Intent::Call { contact };
        }

        // Pattern: "message X" or "text X"
        if lower.starts_with("message ") || lower.starts_with("text ") {
            let parts: Vec<&str> = input.split_whitespace().collect();
            if parts.len() >= 2 {
                let contact = parts[1].to_string();
                let text = parts[2..].join(" ");
                return Intent::Message { contact, text };
            }
        }

        // Pattern: "call status" or "message status"
        if lower.contains("status") {
            if lower.contains("call") {
                return Intent::CallStatus;
            } else if lower.contains("message") {
                return Intent::MessageStatus;
            }
        }

        // Fallback: unknown
        Intent::Unknown {
            raw: input.to_string(),
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Intent::Call { .. } => "call",
            Intent::Message { .. } => "message",
            Intent::CallStatus => "call_status",
            Intent::MessageStatus => "message_status",
            Intent::Unknown { .. } => "unknown",
        }
    }
}

pub struct FallbackIntentParser;

impl FallbackIntentParser {
    pub fn parse(input: &str) -> Intent {
        Intent::from_heuristic(input)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_call_intent() {
        let intent = Intent::from_heuristic("call alice");
        assert_eq!(intent, Intent::Call { contact: "alice".to_string() });
    }

    #[test]
    fn test_message_intent() {
        let intent = Intent::from_heuristic("message bob hello there");
        assert_eq!(
            intent,
            Intent::Message {
                contact: "bob".to_string(),
                text: "hello there".to_string()
            }
        );
    }

    #[test]
    fn test_call_status() {
        let intent = Intent::from_heuristic("what's my call status");
        assert_eq!(intent, Intent::CallStatus);
    }

    #[test]
    fn test_unknown() {
        let intent = Intent::from_heuristic("random words here");
        assert!(matches!(intent, Intent::Unknown { .. }));
    }
}
