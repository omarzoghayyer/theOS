// ipc_protocol.rs -- Canonical IPC protocol for theOS daemon <-> compositor/UI.
// 
// Uses Serde for robust JSON serialization/deserialization that handles
// whitespace and formatting variations correctly.
//
// Wire format: [4-byte LE length][JSON payload]
// Length is the payload length only.

use serde::{Serialize, Deserialize};
use std::io::{Read, Write};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum IpcMessage {
    // -- Connectivity --
    #[serde(rename = "Ping")]
    Ping { sequence: u32 },
    #[serde(rename = "Pong")]
    Pong { sequence: u32 },

    // -- VoIP --
    #[serde(rename = "StartCall")]
    StartCall { contact_key_hex: String },
    #[serde(rename = "HangupCall")]
    HangupCall,
    #[serde(rename = "MuteAudio")]
    MuteAudio { muted: bool },
    #[serde(rename = "CallState")]
    CallState { 
        state: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        contact_key_hex: Option<String>,
    },
    #[serde(rename = "AudioLevel")]
    AudioLevel { level: f32 },

    // -- Network --
    #[serde(rename = "GetNetworkStatus")]
    GetNetworkStatus,
    #[serde(rename = "NetworkStatus")]
    NetworkStatus { link: String },

    // -- Notifications --
    #[serde(rename = "SendNotification")]
    SendNotification {
        title: String,
        body: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        priority: Option<u32>,
    },
    #[serde(rename = "DismissNotification")]
    DismissNotification,

    // -- Errors --
    #[serde(rename = "Error")]
    Error { code: u32, message: String },

    // -- Contact lookup --
    #[serde(rename = "LookupContact")]
    LookupContact { key_hex: String },
    #[serde(rename = "ContactInfo")]
    ContactInfo { 
        key_hex: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        handle: Option<String>,
        found: bool,
    },

    // -- Fallback --
    #[serde(rename = "Unknown")]
    Unknown { raw: String },
}

impl IpcMessage {
    pub fn to_json(&self) -> Vec<u8> {
        match serde_json::to_vec(self) {
            Ok(bytes) => bytes,
            Err(e) => {
                eprintln!("[ipc] serialization error: {}", e);
                format!(r#"{{"type":"Error","code":500,"message":"serialization failed"}}"#).into_bytes()
            }
        }
    }

    pub fn from_json(bytes: &[u8]) -> Self {
        match std::str::from_utf8(bytes) {
            Ok(s) => {
                match serde_json::from_str::<IpcMessage>(s) {
                    Ok(msg) => msg,
                    Err(e) => {
                        eprintln!("[ipc] parse error: {}", e);
                        IpcMessage::Unknown { raw: s.to_string() }
                    }
                }
            }
            Err(_) => IpcMessage::Unknown { 
                raw: "<invalid utf8>".to_string() 
            },
        }
    }
}

pub struct IpcFrame;
impl IpcFrame {
    /// Encode a message into a length-prefixed frame.
    pub fn encode(msg: &IpcMessage) -> Vec<u8> {
        let payload = msg.to_json();
        let len = payload.len() as u32;
        let mut frame = Vec::with_capacity(4 + payload.len());
        frame.extend_from_slice(&len.to_le_bytes());
        frame.extend_from_slice(&payload);
        frame
    }

    /// Decode a message from a length-prefixed frame.
    pub fn decode(buf: &[u8]) -> Option<(IpcMessage, usize)> {
        if buf.len() < 4 { 
            return None; 
        }
        let len = u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]) as usize;
        if buf.len() < 4 + len { 
            return None; 
        }
        let payload = &buf[4..4 + len];
        let msg = IpcMessage::from_json(payload);
        Some((msg, 4 + len))
    }

    pub fn write(w: &mut impl Write, msg: &IpcMessage) -> std::io::Result<()> {
        let frame = Self::encode(msg);
        w.write_all(&frame)
    }

    pub fn read(r: &mut impl Read) -> std::io::Result<IpcMessage> {
        let mut len_buf = [0u8; 4];
        r.read_exact(&mut len_buf)?;
        let len = u32::from_le_bytes(len_buf) as usize;
        let mut payload = vec![0u8; len];
        r.read_exact(&mut payload)?;
        Ok(IpcMessage::from_json(&payload))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_startcall_compact() {
        let msg = IpcMessage::StartCall { contact_key_hex: "@alice".to_string() };
        let json = msg.to_json();
        let parsed = IpcMessage::from_json(&json);
        match parsed {
            IpcMessage::StartCall { contact_key_hex } => assert_eq!(contact_key_hex, "@alice"),
            _ => panic!("parsed wrong variant"),
        }
    }

    #[test]
    fn test_startcall_with_spaces() {
        // Test that we can parse JSON with spaces
        let json_with_spaces = r#"{"type": "StartCall", "contact_key_hex": "@bob"}"#;
        let parsed = IpcMessage::from_json(json_with_spaces.as_bytes());
        match parsed {
            IpcMessage::StartCall { contact_key_hex } => assert_eq!(contact_key_hex, "@bob"),
            _ => panic!("parsed wrong variant"),
        }
    }

    #[test]
    fn test_frame_roundtrip() {
        let msg = IpcMessage::StartCall { contact_key_hex: "@charlie".to_string() };
        let frame = IpcFrame::encode(&msg);
        let (parsed, consumed) = IpcFrame::decode(&frame).unwrap();
        assert_eq!(consumed, frame.len());
        match parsed {
            IpcMessage::StartCall { contact_key_hex } => assert_eq!(contact_key_hex, "@charlie"),
            _ => panic!("parsed wrong variant"),
        }
    }
}

// -- Call state enum --
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CallState {
    #[serde(rename = "idle")]
    Idle,
    #[serde(rename = "calling")]
    Calling,
    #[serde(rename = "ringing")]
    Ringing,
    #[serde(rename = "active")]
    Active,
    #[serde(rename = "ended")]
    Ended,
    #[serde(rename = "failed")]
    Failed,
}

impl CallState {
    pub fn as_str(&self) -> &'static str {
        match self {
            CallState::Idle => "idle",
            CallState::Calling => "calling",
            CallState::Ringing => "ringing",
            CallState::Active => "active",
            CallState::Ended => "ended",
            CallState::Failed => "failed",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "calling" => CallState::Calling,
            "ringing" => CallState::Ringing,
            "active" => CallState::Active,
            "ended" => CallState::Ended,
            "failed" => CallState::Failed,
            _ => CallState::Idle,
        }
    }
}
