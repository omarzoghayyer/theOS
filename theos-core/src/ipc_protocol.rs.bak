// ipc_protocol.rs -- theOS IPC Protocol
//
// Wire format for all compositor <-> daemon communication.
// Pure Rust, no async, no OS dependencies -- fully testable on Windows.
//
// Wire format:
//   [4 bytes: message length LE u32] [N bytes: JSON payload]
//
// Using JSON for v1 -- human readable, easy to debug over satellite.
// Binary msgpack is a Phase 2 optimization if JSON overhead matters.
//
// Message routing:
//   Compositor -> VoipDaemon:   StartCall, HangupCall, MuteAudio
//   VoipDaemon -> Compositor:   CallState, AudioLevel
//   Compositor -> NetDaemon:    GetNetworkStatus
//   NetDaemon  -> Compositor:   NetworkStatus, IpChanged
//   Compositor -> IdDaemon:     LookupContact, GetMyIdentity
//   IdDaemon   -> Compositor:   ContactInfo, IdentityInfo
//   Any        -> Any:          Ping, Pong, Error
//
// Security assumptions (flag for audit):
//   - Unix socket permissions restrict access to same-user processes.
//   - No authentication between daemons -- trust is implicit via socket ACL.
//   - Messages are not encrypted -- they stay on-device via Unix sockets.
//     An attacker with local access can read the socket. Acceptable for v1.
//     Flag: add Ed25519 message signing between daemons in Phase 2.

use std::io::{self, Read, Write};

// -- IpcMessage ---------------------------------------------------------------

/// All messages that can flow between compositor and daemons.
/// Tagged with source/destination daemon for routing.
#[derive(Debug, Clone, PartialEq)]
pub enum IpcMessage {
    // -- Connectivity --
    /// Keepalive. Any daemon -> any daemon.
    Ping { sequence: u32 },
    /// Ping response.
    Pong { sequence: u32 },

    // -- VoIP --
    /// Compositor -> VoipDaemon: initiate a call.
    StartCall { contact_key_hex: String },
    /// Compositor -> VoipDaemon: hang up active call.
    HangupCall,
    /// Compositor -> VoipDaemon: mute/unmute microphone.
    MuteAudio { muted: bool },
    /// VoipDaemon -> Compositor: call state changed.
    CallState { state: CallState, contact_key_hex: Option<String> },
    /// VoipDaemon -> Compositor: audio level for visualization.
    AudioLevel { level: f32 }, // 0.0 - 1.0

    // -- Network --
    /// Compositor -> NetDaemon: request current network status.
    GetNetworkStatus,
    /// NetDaemon -> Compositor: current network status.
    NetworkStatus {
        link:       NetworkLink,
        latency_ms: u32,
        quality:    u8,  // 0-100
        ip:         String,
    },
    /// NetDaemon -> Compositor: IP changed (beam switch).
    IpChanged { old_ip: String, new_ip: String },

    // -- Identity --
    /// Compositor -> IdDaemon: look up a contact by pubkey hex.
    LookupContact { key_hex: String },
    /// IdDaemon -> Compositor: contact info result.
    ContactInfo {
        key_hex:  String,
        handle:   Option<String>,
        found:    bool,
    },
    /// Compositor -> IdDaemon: get our own identity.
    GetMyIdentity,
    /// IdDaemon -> Compositor: our own pubkey and handle.
    IdentityInfo { key_hex: String, handle: Option<String> },

    // -- Power --
    /// PowerDaemon -> Compositor: battery update.
    BatteryUpdate { percent: u8, charging: bool, temperature_c: f32 },
    /// Compositor -> PowerDaemon: acquire wake lock.
    AcquireWakeLock { reason: String },
    /// Compositor -> PowerDaemon: release wake lock.
    ReleaseWakeLock { reason: String },

    // -- Notifications --
    /// Any daemon -> Compositor: show a notification overlay.
    ShowNotification { title: String, body: String, priority: u8 },
    /// Compositor -> Any: dismiss active notification.
    DismissNotification,

    // -- Errors --
    /// Any -> Any: error response.
    Error { code: u32, message: String },
    /// Any -> Any: unknown message type received.
    Unknown { raw: String },
}

#[derive(Debug, Clone, PartialEq)]
pub enum CallState {
    Idle,
    Calling,
    Ringing,
    Active,
    Ended,
    Failed,
}

impl CallState {
    pub fn label(&self) -> &str {
        match self {
            CallState::Idle    => "idle",
            CallState::Calling => "calling",
            CallState::Ringing => "ringing",
            CallState::Active  => "active",
            CallState::Ended   => "ended",
            CallState::Failed  => "failed",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum NetworkLink {
    Starlink,
    Wifi,
    Ethernet,
    None,
}

impl NetworkLink {
    pub fn label(&self) -> &str {
        match self {
            NetworkLink::Starlink  => "starlink",
            NetworkLink::Wifi      => "wifi",
            NetworkLink::Ethernet  => "ethernet",
            NetworkLink::None      => "none",
        }
    }
}

// -- Serialization ------------------------------------------------------------
//
// Manual JSON serialization -- no serde dependency in theos-core.
// Simple enough for the message set. Extend as messages are added.

impl IpcMessage {
    /// Serialize to JSON bytes.
    pub fn to_json(&self) -> Vec<u8> {
        let s = match self {
            IpcMessage::Ping { sequence } =>
                format!(r#"{{"type":"Ping","sequence":{}}}"#, sequence),
            IpcMessage::Pong { sequence } =>
                format!(r#"{{"type":"Pong","sequence":{}}}"#, sequence),
            IpcMessage::StartCall { contact_key_hex } =>
                format!(r#"{{"type":"StartCall","contact_key_hex":"{}"}}"#, contact_key_hex),
            IpcMessage::HangupCall =>
                r#"{"type":"HangupCall"}"#.to_string(),
            IpcMessage::MuteAudio { muted } =>
                format!(r#"{{"type":"MuteAudio","muted":{}}}"#, muted),
            IpcMessage::CallState { state, contact_key_hex } => {
                let key = contact_key_hex.as_deref().unwrap_or("");
                format!(r#"{{"type":"CallState","state":"{}","contact_key_hex":"{}"}}"#,
                    state.label(), key)
            }
            IpcMessage::AudioLevel { level } =>
                format!(r#"{{"type":"AudioLevel","level":{:.3}}}"#, level),
            IpcMessage::GetNetworkStatus =>
                r#"{"type":"GetNetworkStatus"}"#.to_string(),
            IpcMessage::NetworkStatus { link, latency_ms, quality, ip } =>
                format!(r#"{{"type":"NetworkStatus","link":"{}","latency_ms":{},"quality":{},"ip":"{}"}}"#,
                    link.label(), latency_ms, quality, ip),
            IpcMessage::IpChanged { old_ip, new_ip } =>
                format!(r#"{{"type":"IpChanged","old_ip":"{}","new_ip":"{}"}}"#, old_ip, new_ip),
            IpcMessage::LookupContact { key_hex } =>
                format!(r#"{{"type":"LookupContact","key_hex":"{}"}}"#, key_hex),
            IpcMessage::ContactInfo { key_hex, handle, found } => {
                let h = handle.as_deref().unwrap_or("");
                format!(r#"{{"type":"ContactInfo","key_hex":"{}","handle":"{}","found":{}}}"#,
                    key_hex, h, found)
            }
            IpcMessage::GetMyIdentity =>
                r#"{"type":"GetMyIdentity"}"#.to_string(),
            IpcMessage::IdentityInfo { key_hex, handle } => {
                let h = handle.as_deref().unwrap_or("");
                format!(r#"{{"type":"IdentityInfo","key_hex":"{}","handle":"{}"}}"#, key_hex, h)
            }
            IpcMessage::BatteryUpdate { percent, charging, temperature_c } =>
                format!(r#"{{"type":"BatteryUpdate","percent":{},"charging":{},"temperature_c":{:.1}}}"#,
                    percent, charging, temperature_c),
            IpcMessage::AcquireWakeLock { reason } =>
                format!(r#"{{"type":"AcquireWakeLock","reason":"{}"}}"#, reason),
            IpcMessage::ReleaseWakeLock { reason } =>
                format!(r#"{{"type":"ReleaseWakeLock","reason":"{}"}}"#, reason),
            IpcMessage::ShowNotification { title, body, priority } =>
                format!(r#"{{"type":"ShowNotification","title":"{}","body":"{}","priority":{}}}"#,
                    title, body, priority),
            IpcMessage::DismissNotification =>
                r#"{"type":"DismissNotification"}"#.to_string(),
            IpcMessage::Error { code, message } =>
                format!(r#"{{"type":"Error","code":{},"message":"{}"}}"#, code, message),
            IpcMessage::Unknown { raw } =>
                format!(r#"{{"type":"Unknown","raw":"{}"}}"#, raw),
        };
        s.into_bytes()
    }

    /// Deserialize from JSON bytes.
    /// Returns IpcMessage::Unknown if parsing fails.
    pub fn from_json(bytes: &[u8]) -> Self {
        let s = match std::str::from_utf8(bytes) {
            Ok(s)  => s,
            Err(_) => return IpcMessage::Unknown { raw: "<invalid utf8>".to_string() },
        };

        // Extract "type" field
        let msg_type = extract_str_field(s, "type")
            .unwrap_or_default();

        match msg_type.as_str() {
            "Ping"       => IpcMessage::Ping { sequence: extract_u32(s, "sequence").unwrap_or(0) },
            "Pong"       => IpcMessage::Pong { sequence: extract_u32(s, "sequence").unwrap_or(0) },
            "StartCall"  => IpcMessage::StartCall {
                contact_key_hex: extract_str_field(s, "contact_key_hex").unwrap_or_default()
            },
            "HangupCall" => IpcMessage::HangupCall,
            "MuteAudio"  => IpcMessage::MuteAudio {
                muted: extract_bool(s, "muted").unwrap_or(false)
            },
            "CallState"  => IpcMessage::CallState {
                state: match extract_str_field(s, "state").as_deref() {
                    Some("calling") => CallState::Calling,
                    Some("ringing") => CallState::Ringing,
                    Some("active")  => CallState::Active,
                    Some("ended")   => CallState::Ended,
                    Some("failed")  => CallState::Failed,
                    _               => CallState::Idle,
                },
                contact_key_hex: extract_str_field(s, "contact_key_hex")
                    .filter(|s| !s.is_empty()),
            },
            "AudioLevel" => IpcMessage::AudioLevel {
                level: extract_f32(s, "level").unwrap_or(0.0)
            },
            "GetNetworkStatus" => IpcMessage::GetNetworkStatus,
            "NetworkStatus" => IpcMessage::NetworkStatus {
                link: match extract_str_field(s, "link").as_deref() {
                    Some("starlink") => NetworkLink::Starlink,
                    Some("wifi")     => NetworkLink::Wifi,
                    Some("ethernet") => NetworkLink::Ethernet,
                    _                => NetworkLink::None,
                },
                latency_ms: extract_u32(s, "latency_ms").unwrap_or(0),
                quality:    extract_u32(s, "quality").unwrap_or(0) as u8,
                ip:         extract_str_field(s, "ip").unwrap_or_default(),
            },
            "IpChanged" => IpcMessage::IpChanged {
                old_ip: extract_str_field(s, "old_ip").unwrap_or_default(),
                new_ip: extract_str_field(s, "new_ip").unwrap_or_default(),
            },
            "LookupContact" => IpcMessage::LookupContact {
                key_hex: extract_str_field(s, "key_hex").unwrap_or_default()
            },
            "ContactInfo" => IpcMessage::ContactInfo {
                key_hex: extract_str_field(s, "key_hex").unwrap_or_default(),
                handle:  extract_str_field(s, "handle").filter(|s| !s.is_empty()),
                found:   extract_bool(s, "found").unwrap_or(false),
            },
            "GetMyIdentity" => IpcMessage::GetMyIdentity,
            "IdentityInfo"  => IpcMessage::IdentityInfo {
                key_hex: extract_str_field(s, "key_hex").unwrap_or_default(),
                handle:  extract_str_field(s, "handle").filter(|s| !s.is_empty()),
            },
            "BatteryUpdate" => IpcMessage::BatteryUpdate {
                percent:       extract_u32(s, "percent").unwrap_or(0) as u8,
                charging:      extract_bool(s, "charging").unwrap_or(false),
                temperature_c: extract_f32(s, "temperature_c").unwrap_or(25.0),
            },
            "AcquireWakeLock" => IpcMessage::AcquireWakeLock {
                reason: extract_str_field(s, "reason").unwrap_or_default()
            },
            "ReleaseWakeLock" => IpcMessage::ReleaseWakeLock {
                reason: extract_str_field(s, "reason").unwrap_or_default()
            },
            "ShowNotification" => IpcMessage::ShowNotification {
                title:    extract_str_field(s, "title").unwrap_or_default(),
                body:     extract_str_field(s, "body").unwrap_or_default(),
                priority: extract_u32(s, "priority").unwrap_or(0) as u8,
            },
            "DismissNotification" => IpcMessage::DismissNotification,
            "Error" => IpcMessage::Error {
                code:    extract_u32(s, "code").unwrap_or(0),
                message: extract_str_field(s, "message").unwrap_or_default(),
            },
            _ => IpcMessage::Unknown { raw: s.to_string() },
        }
    }
}

// -- IpcFrame -----------------------------------------------------------------

/// Wire frame: [4-byte LE length][payload bytes]
/// Length is the payload length only, not including the 4-byte header.
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
    /// Returns (message, bytes_consumed) or None if not enough data.
    pub fn decode(buf: &[u8]) -> Option<(IpcMessage, usize)> {
        if buf.len() < 4 { return None; }
        let len = u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]) as usize;
        if buf.len() < 4 + len { return None; }
        let payload = &buf[4..4 + len];
        let msg = IpcMessage::from_json(payload);
        Some((msg, 4 + len))
    }

    /// Write a framed message to any Write impl.
    pub fn write(w: &mut impl Write, msg: &IpcMessage) -> io::Result<()> {
        let frame = Self::encode(msg);
        w.write_all(&frame)
    }

    /// Read a framed message from any Read impl.
    pub fn read(r: &mut impl Read) -> io::Result<IpcMessage> {
        let mut len_buf = [0u8; 4];
        r.read_exact(&mut len_buf)?;
        let len = u32::from_le_bytes(len_buf) as usize;
        if len > 1_048_576 {
            // 1MB max message size -- reject oversized frames
            return Err(io::Error::new(io::ErrorKind::InvalidData, "frame too large"));
        }
        let mut payload = vec![0u8; len];
        r.read_exact(&mut payload)?;
        Ok(IpcMessage::from_json(&payload))
    }
}

// -- JSON helpers -------------------------------------------------------------

fn extract_str_field(json: &str, key: &str) -> Option<String> {
    let pattern = format!("\"{}\":\"", key);
    let start = json.find(&pattern)? + pattern.len();
    let end = json[start..].find('"')? + start;
    Some(json[start..end].to_string())
}

fn extract_u32(json: &str, key: &str) -> Option<u32> {
    let pattern = format!("\"{}\":", key);
    let start = json.find(&pattern)? + pattern.len();
    let rest = json[start..].trim_start();
    let end = rest.find(|c: char| !c.is_ascii_digit()).unwrap_or(rest.len());
    rest[..end].parse().ok()
}

fn extract_f32(json: &str, key: &str) -> Option<f32> {
    let pattern = format!("\"{}\":", key);
    let start = json.find(&pattern)? + pattern.len();
    let rest = json[start..].trim_start();
    let end = rest.find(|c: char| !c.is_ascii_digit() && c != '.' && c != '-')
        .unwrap_or(rest.len());
    rest[..end].parse().ok()
}

fn extract_bool(json: &str, key: &str) -> Option<bool> {
    let pattern = format!("\"{}\":", key);
    let start = json.find(&pattern)? + pattern.len();
    let rest = json[start..].trim_start();
    if rest.starts_with("true")  { return Some(true); }
    if rest.starts_with("false") { return Some(false); }
    None
}

// -- Tests --------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // IpcMessage serialization roundtrips

    #[test]
    fn test_ping_roundtrip() {
        let msg = IpcMessage::Ping { sequence: 42 };
        let json = msg.to_json();
        let decoded = IpcMessage::from_json(&json);
        assert_eq!(decoded, IpcMessage::Ping { sequence: 42 });
    }

    #[test]
    fn test_pong_roundtrip() {
        let msg = IpcMessage::Pong { sequence: 7 };
        assert_eq!(IpcMessage::from_json(&msg.to_json()), msg);
    }

    #[test]
    fn test_start_call_roundtrip() {
        let msg = IpcMessage::StartCall { contact_key_hex: "aabbccdd".to_string() };
        assert_eq!(IpcMessage::from_json(&msg.to_json()), msg);
    }

    #[test]
    fn test_hangup_roundtrip() {
        assert_eq!(IpcMessage::from_json(&IpcMessage::HangupCall.to_json()), IpcMessage::HangupCall);
    }

    #[test]
    fn test_mute_audio_roundtrip() {
        let msg = IpcMessage::MuteAudio { muted: true };
        assert_eq!(IpcMessage::from_json(&msg.to_json()), msg);
    }

    #[test]
    fn test_call_state_active_roundtrip() {
        let msg = IpcMessage::CallState {
            state: CallState::Active,
            contact_key_hex: Some("aabbccdd".to_string()),
        };
        assert_eq!(IpcMessage::from_json(&msg.to_json()), msg);
    }

    #[test]
    fn test_call_state_no_contact_roundtrip() {
        let msg = IpcMessage::CallState {
            state: CallState::Ended,
            contact_key_hex: None,
        };
        let decoded = IpcMessage::from_json(&msg.to_json());
        assert_eq!(decoded, msg);
    }

    #[test]
    fn test_audio_level_roundtrip() {
        let msg = IpcMessage::AudioLevel { level: 0.75 };
        if let IpcMessage::AudioLevel { level } = IpcMessage::from_json(&msg.to_json()) {
            assert!((level - 0.75).abs() < 0.001);
        } else { panic!("wrong variant"); }
    }

    #[test]
    fn test_network_status_roundtrip() {
        let msg = IpcMessage::NetworkStatus {
            link:       NetworkLink::Starlink,
            latency_ms: 580,
            quality:    72,
            ip:         "100.64.0.1".to_string(),
        };
        assert_eq!(IpcMessage::from_json(&msg.to_json()), msg);
    }

    #[test]
    fn test_ip_changed_roundtrip() {
        let msg = IpcMessage::IpChanged {
            old_ip: "100.64.0.1".to_string(),
            new_ip: "100.64.0.2".to_string(),
        };
        assert_eq!(IpcMessage::from_json(&msg.to_json()), msg);
    }

    #[test]
    fn test_lookup_contact_roundtrip() {
        let msg = IpcMessage::LookupContact { key_hex: "deadbeef".to_string() };
        assert_eq!(IpcMessage::from_json(&msg.to_json()), msg);
    }

    #[test]
    fn test_contact_info_found_roundtrip() {
        let msg = IpcMessage::ContactInfo {
            key_hex: "deadbeef".to_string(),
            handle:  Some("@omar".to_string()),
            found:   true,
        };
        assert_eq!(IpcMessage::from_json(&msg.to_json()), msg);
    }

    #[test]
    fn test_contact_info_not_found_roundtrip() {
        let msg = IpcMessage::ContactInfo {
            key_hex: "deadbeef".to_string(),
            handle:  None,
            found:   false,
        };
        assert_eq!(IpcMessage::from_json(&msg.to_json()), msg);
    }

    #[test]
    fn test_get_my_identity_roundtrip() {
        assert_eq!(IpcMessage::from_json(&IpcMessage::GetMyIdentity.to_json()), IpcMessage::GetMyIdentity);
    }

    #[test]
    fn test_identity_info_roundtrip() {
        let msg = IpcMessage::IdentityInfo {
            key_hex: "aabbccdd".to_string(),
            handle:  Some("@omar".to_string()),
        };
        assert_eq!(IpcMessage::from_json(&msg.to_json()), msg);
    }

    #[test]
    fn test_battery_update_roundtrip() {
        let msg = IpcMessage::BatteryUpdate { percent: 85, charging: true, temperature_c: 32.5 };
        if let IpcMessage::BatteryUpdate { percent, charging, temperature_c } =
            IpcMessage::from_json(&msg.to_json())
        {
            assert_eq!(percent, 85);
            assert!(charging);
            assert!((temperature_c - 32.5).abs() < 0.1);
        } else { panic!("wrong variant"); }
    }

    #[test]
    fn test_acquire_wake_lock_roundtrip() {
        let msg = IpcMessage::AcquireWakeLock { reason: "voip_call".to_string() };
        assert_eq!(IpcMessage::from_json(&msg.to_json()), msg);
    }

    #[test]
    fn test_release_wake_lock_roundtrip() {
        let msg = IpcMessage::ReleaseWakeLock { reason: "voip_call".to_string() };
        assert_eq!(IpcMessage::from_json(&msg.to_json()), msg);
    }

    #[test]
    fn test_show_notification_roundtrip() {
        let msg = IpcMessage::ShowNotification {
            title:    "Incoming call".to_string(),
            body:     "Marcus calling".to_string(),
            priority: 10,
        };
        assert_eq!(IpcMessage::from_json(&msg.to_json()), msg);
    }

    #[test]
    fn test_dismiss_notification_roundtrip() {
        assert_eq!(
            IpcMessage::from_json(&IpcMessage::DismissNotification.to_json()),
            IpcMessage::DismissNotification
        );
    }

    #[test]
    fn test_error_roundtrip() {
        let msg = IpcMessage::Error { code: 404, message: "not found".to_string() };
        assert_eq!(IpcMessage::from_json(&msg.to_json()), msg);
    }

    #[test]
    fn test_unknown_message() {
        let decoded = IpcMessage::from_json(b"{\"type\":\"WeirdNewMessage\"}");
        assert!(matches!(decoded, IpcMessage::Unknown { .. }));
    }

    #[test]
    fn test_invalid_utf8() {
        let decoded = IpcMessage::from_json(&[0xFF, 0xFE]);
        assert!(matches!(decoded, IpcMessage::Unknown { .. }));
    }

    // IpcFrame encode/decode

    #[test]
    fn test_frame_encode_decode_roundtrip() {
        let msg = IpcMessage::Ping { sequence: 99 };
        let frame = IpcFrame::encode(&msg);
        let (decoded, consumed) = IpcFrame::decode(&frame).unwrap();
        assert_eq!(decoded, msg);
        assert_eq!(consumed, frame.len());
    }

    #[test]
    fn test_frame_header_is_4_bytes() {
        let msg = IpcMessage::HangupCall;
        let frame = IpcFrame::encode(&msg);
        let payload_len = msg.to_json().len();
        assert_eq!(frame.len(), 4 + payload_len);
    }

    #[test]
    fn test_frame_decode_incomplete_header() {
        assert!(IpcFrame::decode(&[0u8; 3]).is_none());
    }

    #[test]
    fn test_frame_decode_incomplete_payload() {
        let msg = IpcMessage::Ping { sequence: 1 };
        let frame = IpcFrame::encode(&msg);
        // Give only header, no payload
        assert!(IpcFrame::decode(&frame[..4]).is_none());
    }

    #[test]
    fn test_frame_multiple_messages_in_buffer() {
        let m1 = IpcMessage::Ping { sequence: 1 };
        let m2 = IpcMessage::Pong { sequence: 1 };
        let mut buf = IpcFrame::encode(&m1);
        buf.extend(IpcFrame::encode(&m2));

        let (d1, n1) = IpcFrame::decode(&buf).unwrap();
        let (d2, n2) = IpcFrame::decode(&buf[n1..]).unwrap();
        assert_eq!(d1, m1);
        assert_eq!(d2, m2);
        assert_eq!(n1 + n2, buf.len());
    }

    #[test]
    fn test_frame_write_read_roundtrip() {
        let msg = IpcMessage::NetworkStatus {
            link:       NetworkLink::Starlink,
            latency_ms: 620,
            quality:    68,
            ip:         "100.64.0.5".to_string(),
        };
        let mut buf = Vec::new();
        IpcFrame::write(&mut buf, &msg).unwrap();
        let decoded = IpcFrame::read(&mut buf.as_slice()).unwrap();
        assert_eq!(decoded, msg);
    }

    // CallState / NetworkLink labels

    #[test]
    fn test_call_state_labels() {
        assert_eq!(CallState::Idle.label(),    "idle");
        assert_eq!(CallState::Active.label(),  "active");
        assert_eq!(CallState::Failed.label(),  "failed");
    }

    #[test]
    fn test_network_link_labels() {
        assert_eq!(NetworkLink::Starlink.label(), "starlink");
        assert_eq!(NetworkLink::None.label(),     "none");
    }

    #[test]
    fn test_get_network_status_roundtrip() {
        assert_eq!(
            IpcMessage::from_json(&IpcMessage::GetNetworkStatus.to_json()),
            IpcMessage::GetNetworkStatus
        );
    }
}
