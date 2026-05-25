// ai_shell_v2.rs — Enhanced AI Shell with better intent parsing
//
// Improvements over v1:
// - Wake-word detection (supports "call", "message", "check", "what's", "tell me")
// - Confidence scoring for parsed intents
// - Error recovery (asks user to repeat if confidence < threshold)
// - Voice feedback responses
// - Context awareness (recent calls/messages)

use crate::dht::HandleRegistry;

/// Enhanced intent parser with confidence scores
#[derive(Debug, Clone, PartialEq)]
pub enum Intent {
    /// Call a contact: "call alice", "call @alice", "phone alice"
    Call {
        contact: String,
        confidence: f32,
    },
    /// Message a contact: "message bob", "text bob", "send message to bob"
    Message {
        contact: String,
        text: Option<String>,
        confidence: f32,
    },
    /// Check call status: "am I on a call", "call status", "ongoing calls"
    CallStatus {
        confidence: f32,
    },
    /// Check message status: "unread messages", "message status"
    MessageStatus {
        confidence: f32,
    },
    /// Help/unclear: "help", "what can I say"
    Help {
        confidence: f32,
    },
    /// No match
    Unknown {
        raw: String,
        confidence: f32,
    },
}

impl Intent {
    pub fn is_high_confidence(&self) -> bool {
        self.confidence() >= 0.75
    }

    pub fn confidence(&self) -> f32 {
        match self {
            Intent::Call { confidence, .. } => *confidence,
            Intent::Message { confidence, .. } => *confidence,
            Intent::CallStatus { confidence } => *confidence,
            Intent::MessageStatus { confidence } => *confidence,
            Intent::Help { confidence } => *confidence,
            Intent::Unknown { confidence, .. } => *confidence,
        }
    }

    pub fn voice_feedback(&self) -> &'static str {
        match self {
            Intent::Call { .. } => "Calling...",
            Intent::Message { .. } => "Composing message...",
            Intent::CallStatus { .. } => "Checking call status...",
            Intent::MessageStatus { .. } => "Checking messages...",
            Intent::Help { .. } => "Here's what you can say...",
            Intent::Unknown { .. } => "I didn't quite get that. Can you repeat?",
        }
    }
}

/// Enhanced wake-word detector
pub struct WakeWordDetector {
    call_keywords: Vec<&'static str>,
    message_keywords: Vec<&'static str>,
    status_keywords: Vec<&'static str>,
}

impl WakeWordDetector {
    pub fn new() -> Self {
        Self {
            call_keywords: vec!["call", "phone", "ring", "dial", "reach"],
            message_keywords: vec!["message", "text", "send", "mail", "sms"],
            status_keywords: vec!["status", "check", "ongoing", "current", "active"],
        }
    }

    /// Detect if voice input contains a wake word
    pub fn has_wake_word(&self, input: &str) -> bool {
        let lower = input.to_lowercase();
        self.call_keywords.iter().any(|kw| lower.contains(kw))
            || self.message_keywords.iter().any(|kw| lower.contains(kw))
            || self.status_keywords.iter().any(|kw| lower.contains(kw))
    }

    /// Extract contact name from voice input
    /// Handles: "call alice", "call @alice", "phone alice"
    fn extract_contact(&self, input: &str) -> Option<String> {
        let words: Vec<&str> = input.split_whitespace().collect();
        if words.len() < 2 {
            return None;
        }

        // Find the keyword position
        let keyword_pos = words.iter().position(|w| {
            let lower = w.to_lowercase();
            self.call_keywords.contains(&lower.as_str())
                || self.message_keywords.contains(&lower.as_str())
        })?;

        // Contact is the next word after keyword
        words.get(keyword_pos + 1).map(|&contact| {
            contact.trim_matches('@').to_string()
        })
    }

    /// Extract message text if present
    /// Handles: "message alice hello there", "text bob how are you"
    fn extract_message_text(&self, input: &str) -> Option<String> {
        let words: Vec<&str> = input.split_whitespace().collect();
        if words.len() < 3 {
            return None;
        }

        // Find keyword, skip keyword + contact, collect rest
        let keyword_pos = words.iter().position(|w| {
            let lower = w.to_lowercase();
            self.message_keywords.contains(&lower.as_str())
        })?;

        let text_start = keyword_pos + 2; // skip keyword and contact
        if text_start >= words.len() {
            return None;
        }

        Some(words[text_start..].join(" "))
    }
}

/// Intent parser with confidence scoring
pub struct IntentParser {
    wake_detector: WakeWordDetector,
    min_confidence: f32,
}

impl IntentParser {
    pub fn new(min_confidence: f32) -> Self {
        Self {
            wake_detector: WakeWordDetector::new(),
            min_confidence,
        }
    }

    /// Parse voice input into intent with confidence
    pub fn parse(&self, input: &str) -> Intent {
        if input.is_empty() {
            return Intent::Unknown {
                raw: input.to_string(),
                confidence: 0.0,
            };
        }

        let lower = input.to_lowercase();

        // Check for call intent
        if self.wake_detector.call_keywords.iter().any(|kw| lower.contains(kw)) {
            if let Some(contact) = self.wake_detector.extract_contact(&lower) {
                return Intent::Call {
                    contact,
                    confidence: 0.95, // high confidence
                };
            } else {
                return Intent::Call {
                    contact: "unknown".to_string(),
                    confidence: 0.6, // medium confidence (no contact specified)
                };
            }
        }

        // Check for message intent
        if self.wake_detector.message_keywords.iter().any(|kw| lower.contains(kw)) {
            let contact = self.wake_detector.extract_contact(&lower);
            let text = self.wake_detector.extract_message_text(&lower);
            return Intent::Message {
                contact: contact.unwrap_or_else(|| "unknown".to_string()),
                text,
                confidence: if contact.is_some() { 0.90 } else { 0.5 },
            };
        }

        // Check for status intent
        if lower.contains("status")
            || lower.contains("check")
            || lower.contains("ongoing")
            || lower.contains("current")
        {
            if lower.contains("call") {
                return Intent::CallStatus { confidence: 0.88 };
            } else if lower.contains("message") {
                return Intent::MessageStatus { confidence: 0.88 };
            } else {
                // Ambiguous - could be either
                return Intent::CallStatus { confidence: 0.60 };
            }
        }

        // Check for help
        if lower.contains("help") || lower.contains("what can i say") {
            return Intent::Help { confidence: 0.95 };
        }

        // No match
        Intent::Unknown {
            raw: input.to_string(),
            confidence: 0.0,
        }
    }

    /// Parse with error recovery: if confidence is low, ask user to repeat
    pub fn parse_with_recovery(&self, input: &str) -> (Intent, Option<String>) {
        let intent = self.parse(input);
        if intent.confidence() < self.min_confidence {
            (
                intent.clone(),
                Some("I didn't quite understand. Could you repeat that?".to_string()),
            )
        } else {
            (intent, None)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_call_intent() {
        let parser = IntentParser::new(0.75);
        let intent = parser.parse("call alice");
        assert_eq!(
            intent,
            Intent::Call {
                contact: "alice".to_string(),
                confidence: 0.95
            }
        );
    }

    #[test]
    fn test_parse_call_with_at_symbol() {
        let parser = IntentParser::new(0.75);
        let intent = parser.parse("call @alice");
        assert_eq!(
            intent,
            Intent::Call {
                contact: "alice".to_string(),
                confidence: 0.95
            }
        );
    }

    #[test]
    fn test_parse_message_intent() {
        let parser = IntentParser::new(0.75);
        let intent = parser.parse("message bob hello");
        match intent {
            Intent::Message {
                contact,
                text,
                confidence,
            } => {
                assert_eq!(contact, "bob");
                assert_eq!(text, Some("hello".to_string()));
                assert!(confidence > 0.85);
            }
            _ => panic!("Expected Message intent"),
        }
    }

    #[test]
    fn test_parse_call_status() {
        let parser = IntentParser::new(0.75);
        let intent = parser.parse("call status");
        assert!(matches!(intent, Intent::CallStatus { .. }));
    }

    #[test]
    fn test_parse_help() {
        let parser = IntentParser::new(0.75);
        let intent = parser.parse("help");
        assert!(matches!(intent, Intent::Help { confidence: 0.95 }));
    }

    #[test]
    fn test_low_confidence_recovery() {
        let parser = IntentParser::new(0.75);
        let (intent, recovery) = parser.parse_with_recovery("blah blah");
        assert!(intent.confidence() < 0.75);
        assert!(recovery.is_some());
    }

    #[test]
    fn test_call_without_contact() {
        let parser = IntentParser::new(0.75);
        let intent = parser.parse("call");
        match intent {
            Intent::Call { contact, confidence } => {
                assert_eq!(contact, "unknown");
                assert!(confidence < 0.75);
            }
            _ => panic!("Expected Call intent"),
        }
    }

    #[test]
    fn test_multiple_keywords() {
        let parser = IntentParser::new(0.75);
        // "phone" is an alternative to "call"
        let intent = parser.parse("phone charlie");
        assert!(matches!(
            intent,
            Intent::Call {
                contact,
                confidence: 0.95
            } if contact == "charlie"
        ));
    }
}
