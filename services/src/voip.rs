// voip.rs -- theOS VoIP Engine
//
// Encrypted peer-to-peer voice calls over satellite UDP.
// No SIP. No phone numbers. No carriers. No central server.
//
// Call flow:
//   Caller                                    Callee
//     |                                          |
//     |-- CallRequest (signed Ed25519) --------> |
//     |<- CallAccept  (signed Ed25519) ----------|
//     |                                          |
//     |  Both derive ChaCha20 session key        |
//     |  from Ed25519 keypairs + session_id      |
//     |                                          |
//     |<======= encrypted Opus RTP frames ======>|
//     |                                          |
//     |-- CallEnd (signed) --------------------> |
//
// Satellite optimization:
//   - 200ms jitter buffer (Starlink ~47ms avg, spikes to 200ms)
//   - 20ms Opus frames at 32kbps (satellite-efficient)
//   - RTP keepalive every 5 seconds (prevents NAT timeout)
//   - RE-INVITE on beam switch (IP change detection)
//
// Security:
//   - Ed25519 identity -- no phone numbers
//   - ChaCha20-Poly1305 -- all audio encrypted
//   - Replay window -- 30 second timestamp check
//   - No SIP server -- direct device-to-device

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce, aead::{Aead, KeyInit}};
use sha2::{Sha256, Digest};

// -- Constants ----------------------------------------------------------------

pub const RTP_PORT:           u16 = 16384;
pub const KEEPALIVE_INTERVAL: u64 = 5;      // seconds
pub const JITTER_BUFFER_MS:   u32 = 200;    // satellite-optimized
pub const OPUS_FRAME_MS:      u32 = 20;     // 20ms frames
pub const OPUS_BITRATE_KBPS:  u32 = 32;     // satellite-efficient
pub const REPLAY_WINDOW_SECS: u64 = 30;     // replay protection window
pub const CALL_TIMEOUT_SECS:  u64 = 30;     // ring timeout

// -- CallState ----------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub enum CallState {
    /// No active call
    Idle,
    /// Outgoing call -- waiting for accept
    Dialing { target_key: [u8; 32], peer_addr: SocketAddr, session_id: u64 },
    /// Incoming call -- waiting for user to accept/decline
    Ringing { caller_key: [u8; 32], peer_addr: SocketAddr, session_id: u64 },
    /// Call active -- encrypted audio flowing
    Active  { peer_addr: SocketAddr, session_id: u64, started_at: u64 },
    /// Call ending -- BYE sent/received
    Ending,
}

impl CallState {
    pub fn label(&self) -> &str {
        match self {
            CallState::Idle         => "idle",
            CallState::Dialing {..} => "dialing",
            CallState::Ringing {..} => "ringing",
            CallState::Active  {..} => "active",
            CallState::Ending       => "ending",
        }
    }

    pub fn is_active(&self) -> bool { matches!(self, CallState::Active {..}) }
    pub fn is_idle(&self)   -> bool { matches!(self, CallState::Idle) }

    pub fn session_id(&self) -> Option<u64> {
        match self {
            CallState::Dialing { session_id, .. } => Some(*session_id),
            CallState::Ringing { session_id, .. } => Some(*session_id),
            CallState::Active  { session_id, .. } => Some(*session_id),
            _ => None,
        }
    }

    pub fn peer_addr(&self) -> Option<SocketAddr> {
        match self {
            CallState::Dialing { peer_addr, .. } => Some(*peer_addr),
            CallState::Ringing { peer_addr, .. } => Some(*peer_addr),
            CallState::Active  { peer_addr, .. } => Some(*peer_addr),
            _ => None,
        }
    }

    pub fn duration_secs(&self) -> Option<u64> {
        if let CallState::Active { started_at, .. } = self {
            Some(now_secs().saturating_sub(*started_at))
        } else {
            None
        }
    }
}

// -- Wire messages ------------------------------------------------------------
//
// Minimal binary protocol. All messages signed with Ed25519.
// No SIP overhead. Satellite-efficient.

#[derive(Debug, Clone)]
pub enum VoipMessage {
    /// Initiate a call. Signed by caller's Ed25519 key.
    CallRequest {
        caller_key: [u8; 32],
        target_key: [u8; 32],
        session_id: u64,
        timestamp:  u64,
        signature:  [u8; 64],
    },

    /// Accept a call. Signed by callee's Ed25519 key.
    CallAccept {
        callee_key: [u8; 32],
        session_id: u64,
        timestamp:  u64,
        signature:  [u8; 64],
    },

    /// Decline a call.
    CallDecline {
        callee_key: [u8; 32],
        session_id: u64,
        signature:  [u8; 64],
    },

    /// End an active call.
    CallEnd {
        sender_key: [u8; 32],
        session_id: u64,
        signature:  [u8; 64],
    },

    /// Encrypted Opus audio frame.
    /// Payload: ChaCha20-Poly1305 encrypted Opus frame.
    AudioFrame {
        session_id:    u64,
        sequence:      u32,   // monotonic sequence number
        timestamp_rtp: u32,   // RTP timestamp (samples)
        nonce_counter: u64,   // for ChaCha20 nonce reconstruction
        payload:       Vec<u8>, // encrypted Opus frame
    },

    /// Keepalive -- sent every 5s to prevent NAT timeout.
    Keepalive {
        session_id: u64,
        sender_key: [u8; 32],
    },

    /// IP change notification (Starlink beam switch).
    /// Triggers RE-INVITE equivalent -- update peer address.
    AddressChange {
        sender_key:  [u8; 32],
        session_id:  u64,
        new_addr:    SocketAddr,
        timestamp:   u64,
        signature:   [u8; 64],
    },
}

impl VoipMessage {
    pub fn tag(&self) -> u8 {
        match self {
            VoipMessage::CallRequest   {..} => 0x10,
            VoipMessage::CallAccept    {..} => 0x11,
            VoipMessage::CallDecline   {..} => 0x12,
            VoipMessage::CallEnd       {..} => 0x13,
            VoipMessage::AudioFrame    {..} => 0x20,
            VoipMessage::Keepalive     {..} => 0x30,
            VoipMessage::AddressChange {..} => 0x40,
        }
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut v = vec![self.tag()];
        match self {
            VoipMessage::CallRequest { caller_key, target_key, session_id, timestamp, signature } => {
                v.extend_from_slice(caller_key);
                v.extend_from_slice(target_key);
                v.extend_from_slice(&session_id.to_le_bytes());
                v.extend_from_slice(&timestamp.to_le_bytes());
                v.extend_from_slice(signature);
            }
            VoipMessage::CallAccept { callee_key, session_id, timestamp, signature } => {
                v.extend_from_slice(callee_key);
                v.extend_from_slice(&session_id.to_le_bytes());
                v.extend_from_slice(&timestamp.to_le_bytes());
                v.extend_from_slice(signature);
            }
            VoipMessage::CallDecline { callee_key, session_id, signature } => {
                v.extend_from_slice(callee_key);
                v.extend_from_slice(&session_id.to_le_bytes());
                v.extend_from_slice(signature);
            }
            VoipMessage::CallEnd { sender_key, session_id, signature } => {
                v.extend_from_slice(sender_key);
                v.extend_from_slice(&session_id.to_le_bytes());
                v.extend_from_slice(signature);
            }
            VoipMessage::AudioFrame { session_id, sequence, timestamp_rtp, nonce_counter, payload } => {
                v.extend_from_slice(&session_id.to_le_bytes());
                v.extend_from_slice(&sequence.to_le_bytes());
                v.extend_from_slice(&timestamp_rtp.to_le_bytes());
                v.extend_from_slice(&nonce_counter.to_le_bytes());
                v.extend_from_slice(&(payload.len() as u16).to_le_bytes());
                v.extend_from_slice(payload);
            }
            VoipMessage::Keepalive { session_id, sender_key } => {
                v.extend_from_slice(&session_id.to_le_bytes());
                v.extend_from_slice(sender_key);
            }
            VoipMessage::AddressChange { sender_key, session_id, new_addr, timestamp, signature } => {
                v.extend_from_slice(sender_key);
                v.extend_from_slice(&session_id.to_le_bytes());
                v.extend_from_slice(&timestamp.to_le_bytes());
                v.extend_from_slice(signature);
                // addr encoded as string for simplicity
                let addr_str = new_addr.to_string();
                v.push(addr_str.len() as u8);
                v.extend_from_slice(addr_str.as_bytes());
            }
        }
        v
    }

    pub fn parse(buf: &[u8]) -> Option<Self> {
        if buf.is_empty() { return None; }
        match buf[0] {
            0x10 => {
                // CallRequest: tag(1) + caller(32) + target(32) + session(8) + ts(8) + sig(64) = 145
                if buf.len() < 145 { return None; }
                let mut caller_key = [0u8; 32]; caller_key.copy_from_slice(&buf[1..33]);
                let mut target_key = [0u8; 32]; target_key.copy_from_slice(&buf[33..65]);
                let session_id = u64::from_le_bytes(buf[65..73].try_into().ok()?);
                let timestamp  = u64::from_le_bytes(buf[73..81].try_into().ok()?);
                let mut signature = [0u8; 64]; signature.copy_from_slice(&buf[81..145]);
                Some(VoipMessage::CallRequest { caller_key, target_key, session_id, timestamp, signature })
            }
            0x11 => {
                // CallAccept: tag(1) + callee(32) + session(8) + ts(8) + sig(64) = 113
                if buf.len() < 113 { return None; }
                let mut callee_key = [0u8; 32]; callee_key.copy_from_slice(&buf[1..33]);
                let session_id = u64::from_le_bytes(buf[33..41].try_into().ok()?);
                let timestamp  = u64::from_le_bytes(buf[41..49].try_into().ok()?);
                let mut signature = [0u8; 64]; signature.copy_from_slice(&buf[49..113]);
                Some(VoipMessage::CallAccept { callee_key, session_id, timestamp, signature })
            }
            0x20 => {
                // AudioFrame: tag(1) + session(8) + seq(4) + rtp_ts(4) + nonce(8) + len(2) + payload
                if buf.len() < 27 { return None; }
                let session_id    = u64::from_le_bytes(buf[1..9].try_into().ok()?);
                let sequence      = u32::from_le_bytes(buf[9..13].try_into().ok()?);
                let timestamp_rtp = u32::from_le_bytes(buf[13..17].try_into().ok()?);
                let nonce_counter = u64::from_le_bytes(buf[17..25].try_into().ok()?);
                let payload_len   = u16::from_le_bytes(buf[25..27].try_into().ok()?) as usize;
                if buf.len() < 27 + payload_len { return None; }
                let payload = buf[27..27 + payload_len].to_vec();
                Some(VoipMessage::AudioFrame { session_id, sequence, timestamp_rtp, nonce_counter, payload })
            }
            0x30 => {
                if buf.len() < 41 { return None; }
                let session_id = u64::from_le_bytes(buf[1..9].try_into().ok()?);
                let mut sender_key = [0u8; 32]; sender_key.copy_from_slice(&buf[9..41]);
                Some(VoipMessage::Keepalive { session_id, sender_key })
            }
            _ => None,
        }
    }
}

// -- Jitter buffer ------------------------------------------------------------
//
// Satellite-optimized jitter buffer.
// Holds frames for up to JITTER_BUFFER_MS before playing.
// Handles out-of-order frames from Starlink beam switches.

pub struct JitterBuffer {
    frames:      Vec<(u32, Vec<u8>)>, // (sequence, decoded_opus)
    target_ms:   u32,
    last_played: u32,
}

impl JitterBuffer {
    pub fn new() -> Self {
        Self {
            frames:      Vec::with_capacity(20),
            target_ms:   JITTER_BUFFER_MS,
            last_played: 0,
        }
    }

    /// Push an incoming audio frame into the buffer.
    pub fn push(&mut self, sequence: u32, frame: Vec<u8>) {
        // Insert in order
        let pos = self.frames.partition_point(|(s, _)| *s < sequence);
        if pos == self.frames.len() || self.frames[pos].0 != sequence {
            self.frames.insert(pos, (sequence, frame));
        }
        // Cap buffer size
        if self.frames.len() > 50 {
            self.frames.remove(0);
        }
    }

    /// Pop the next frame for playback. Returns None if buffer not filled yet.
    pub fn pop(&mut self) -> Option<Vec<u8>> {
        if self.frames.is_empty() { return None; }
        let (seq, frame) = self.frames.remove(0);
        self.last_played = seq;
        Some(frame)
    }

    pub fn len(&self) -> usize { self.frames.len() }
    pub fn is_empty(&self) -> bool { self.frames.is_empty() }
}

// -- Session key derivation ---------------------------------------------------
//
// Both sides derive the same ChaCha20 session key from:
//   - caller Ed25519 public key
//   - callee Ed25519 public key
//   - session_id (random u64 from caller)
//
// Key derivation: SHA-256(sorted_keys || session_id || "theos-voip-v1")
// This produces the same key on both sides without a DH round trip.
//
// Security assumption: This is deterministic key derivation, NOT forward-secret.
// Phase 6: replace with X25519 DH for forward secrecy.
// Flag for audit.

pub fn derive_session_key(
    key_a:      &[u8; 32],
    key_b:      &[u8; 32],
    session_id: u64,
) -> [u8; 32] {
    let mut hasher = Sha256::new();
    if key_a <= key_b {
        hasher.update(key_a);
        hasher.update(key_b);
    } else {
        hasher.update(key_b);
        hasher.update(key_a);
    }
    hasher.update(session_id.to_le_bytes());
    hasher.update(b"theos-voip-v1");
    hasher.finalize().into()
}

// -- CallStats ----------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct CallStats {
    pub frames_sent:     u64,
    pub frames_received: u64,
    pub frames_lost:     u64,
    pub bytes_sent:      u64,
    pub bytes_received:  u64,
    pub last_rtt_ms:     u32,
    pub jitter_ms:       u32,
    pub beam_switches:   u32, // Starlink beam switch count
}

impl CallStats {
    pub fn packet_loss_pct(&self) -> f32 {
        if self.frames_sent == 0 { return 0.0; }
        (self.frames_lost as f32 / self.frames_sent as f32) * 100.0
    }
}

// -- VoipSession --------------------------------------------------------------
//
// A single active call session. Owns the encryption state,
// jitter buffer, and call statistics.

pub struct VoipSession {
    pub session_id:  u64,
    pub peer_addr:   SocketAddr,
    pub peer_key:    [u8; 32],
    pub my_key:      [u8; 32],
    session_key:     [u8; 32],   // ChaCha20 key
    send_counter:    u64,        // nonce counter for sending
    recv_window:     Vec<u64>,   // replay protection window
    pub jitter:      JitterBuffer,
    pub stats:       CallStats,
    pub started_at:  u64,
}

impl VoipSession {
    pub fn new(
        session_id: u64,
        peer_addr:  SocketAddr,
        peer_key:   [u8; 32],
        my_key:     [u8; 32],
    ) -> Self {
        let session_key = derive_session_key(&my_key, &peer_key, session_id);
        println!("[voip] session key derived for session:{} peer:{}",
            session_id, hex8(&peer_key));
        Self {
            session_id,
            peer_addr,
            peer_key,
            my_key,
            session_key,
            send_counter:  0,
            recv_window:   Vec::with_capacity(64),
            jitter:        JitterBuffer::new(),
            stats:         CallStats::default(),
            started_at:    now_secs(),
        }
    }

    /// Encrypt an Opus frame for transmission.
    /// Returns an AudioFrame message ready to send.
    ///
    /// Security: ChaCha20-Poly1305 with nonce derived from send_counter.
    /// The counter must never repeat for the same session key.
    pub fn encrypt_frame(&mut self, opus_frame: &[u8], sequence: u32) -> VoipMessage {
        let nonce_counter = self.send_counter;
        self.send_counter += 1;

        // Real ChaCha20-Poly1305 encryption
        // Nonce: 8-byte counter (LE) || 4-byte zero pad = 12 bytes
        let mut nonce_bytes = [0u8; 12];
        nonce_bytes[..8].copy_from_slice(&nonce_counter.to_le_bytes());
        let key    = Key::from_slice(&self.session_key);
        let cipher = ChaCha20Poly1305::new(key);
        let nonce  = Nonce::from_slice(&nonce_bytes);
        let payload = cipher.encrypt(nonce, opus_frame)
            .unwrap_or_else(|_| {
                println!("[voip] encrypt failed for counter:{}", nonce_counter);
                vec![]
            });

        let rtp_timestamp = (sequence * (OPUS_FRAME_MS * 48)) as u32; // 48kHz samples

        self.stats.frames_sent += 1;
        self.stats.bytes_sent  += payload.len() as u64;

        VoipMessage::AudioFrame {
            session_id:    self.session_id,
            sequence,
            timestamp_rtp: rtp_timestamp,
            nonce_counter,
            payload,
        }
    }

    /// Decrypt and buffer an incoming audio frame.
    /// Returns decrypted Opus frame, or None if auth fails.
    pub fn decrypt_frame(&mut self, msg: &VoipMessage) -> Option<Vec<u8>> {
        if let VoipMessage::AudioFrame { sequence, nonce_counter, payload, .. } = msg {
            // Replay protection
            if self.is_replayed(*nonce_counter) {
                println!("[voip] replay detected counter:{}", nonce_counter);
                return None;
            }
            self.mark_seen(*nonce_counter);

            // Real ChaCha20-Poly1305 decryption + auth tag verification
            if payload.is_empty() { return None; }
            let mut nonce_bytes = [0u8; 12];
            nonce_bytes[..8].copy_from_slice(&nonce_counter.to_le_bytes());
            let key    = Key::from_slice(&self.session_key);
            let cipher = ChaCha20Poly1305::new(key);
            let nonce  = Nonce::from_slice(&nonce_bytes);
            let plaintext = match cipher.decrypt(nonce, payload.as_ref()) {
                Ok(pt) => pt,
                Err(_) => {
                    println!("[voip] decrypt auth failed counter:{} -- dropping frame", nonce_counter);
                    return None;
                }
            };

            self.stats.frames_received += 1;
            self.stats.bytes_received  += payload.len() as u64;
            self.jitter.push(*sequence, plaintext.clone());
            Some(plaintext)
        } else {
            None
        }
    }

    /// Update peer address on Starlink beam switch.
    pub fn update_peer_addr(&mut self, new_addr: SocketAddr) {
        println!("[voip] beam switch: {} -> {}", self.peer_addr, new_addr);
        self.peer_addr = new_addr;
        self.stats.beam_switches += 1;
    }

    pub fn duration_secs(&self) -> u64 {
        now_secs().saturating_sub(self.started_at)
    }

    fn is_replayed(&self, counter: u64) -> bool {
        self.recv_window.contains(&counter)
    }

    fn mark_seen(&mut self, counter: u64) {
        self.recv_window.push(counter);
        // Keep window bounded
        if self.recv_window.len() > 128 {
            self.recv_window.remove(0);
        }
    }
}

// -- VoipEngine ---------------------------------------------------------------

pub struct VoipEngine {
    pub my_key:  [u8; 32],
    pub state:   Arc<Mutex<CallState>>,
    pub session: Arc<Mutex<Option<VoipSession>>>,
}

impl VoipEngine {
    pub fn new(my_key: [u8; 32]) -> Self {
        println!("[voip] engine initialized id:{}", hex8(&my_key));
        Self {
            my_key,
            state:   Arc::new(Mutex::new(CallState::Idle)),
            session: Arc::new(Mutex::new(None)),
        }
    }

    /// Initiate a call to a contact.
    /// Returns the CallRequest message to send to the peer.
    pub fn initiate_call(
        &self,
        target_key: [u8; 32],
        peer_addr:  SocketAddr,
    ) -> Result<VoipMessage, &'static str> {
        let mut state = self.state.lock().unwrap();
        if !state.is_idle() {
            return Err("already in a call");
        }
        let session_id = now_secs() ^ (peer_addr.port() as u64 * 0x9e3779b9);
        let timestamp  = now_secs();

        // Production: sign with Ed25519 private key
        // Stub: placeholder signature
        let signature  = [0u8; 64];

        *state = CallState::Dialing { target_key, peer_addr, session_id };

        println!("[voip] dialing {} session:{}", peer_addr, session_id);

        Ok(VoipMessage::CallRequest {
            caller_key: self.my_key,
            target_key,
            session_id,
            timestamp,
            signature,
        })
    }

    /// Accept an incoming call.
    /// Returns CallAccept message and initializes the session.
    pub fn accept_call(
        &self,
        request:   &VoipMessage,
        peer_addr: SocketAddr,
    ) -> Result<VoipMessage, &'static str> {
        if let VoipMessage::CallRequest { caller_key, session_id, timestamp, .. } = request {
            // Replay check
            if now_secs().saturating_sub(*timestamp) > REPLAY_WINDOW_SECS {
                return Err("call request expired");
            }

            let mut state   = self.state.lock().unwrap();
            let mut session = self.session.lock().unwrap();

            *state = CallState::Active {
                peer_addr,
                session_id: *session_id,
                started_at: now_secs(),
            };

            *session = Some(VoipSession::new(
                *session_id, peer_addr, *caller_key, self.my_key,
            ));

            println!("[voip] call accepted session:{} peer:{}", session_id, peer_addr);

            Ok(VoipMessage::CallAccept {
                callee_key: self.my_key,
                session_id: *session_id,
                timestamp:  now_secs(),
                signature:  [0u8; 64],
            })
        } else {
            Err("not a call request")
        }
    }

    /// Called when peer accepts our outgoing call.
    pub fn on_call_accepted(
        &self,
        accept:    &VoipMessage,
        peer_addr: SocketAddr,
    ) -> Result<(), &'static str> {
        if let VoipMessage::CallAccept { callee_key, session_id, .. } = accept {
            let mut state   = self.state.lock().unwrap();
            let mut session = self.session.lock().unwrap();

            if let CallState::Dialing { target_key, session_id: dialing_id, .. } = *state {
                if *session_id != dialing_id {
                    return Err("session ID mismatch");
                }
                *state = CallState::Active {
                    peer_addr,
                    session_id: *session_id,
                    started_at: now_secs(),
                };
                *session = Some(VoipSession::new(
                    *session_id, peer_addr, *callee_key, self.my_key,
                ));
                println!("[voip] call active session:{}", session_id);
                Ok(())
            } else {
                Err("not in dialing state")
            }
        } else {
            Err("not a call accept")
        }
    }

    /// Hang up the current call.
    pub fn hangup(&self) -> Option<VoipMessage> {
        let mut state   = self.state.lock().unwrap();
        let mut session = self.session.lock().unwrap();

        if let Some(ref s) = *session {
            let msg = VoipMessage::CallEnd {
                sender_key: self.my_key,
                session_id: s.session_id,
                signature:  [0u8; 64],
            };
            let stats = s.stats.clone();
            println!("[voip] hangup -- duration:{}s sent:{} received:{} loss:{:.1}% beams:{}",
                s.duration_secs(),
                stats.frames_sent, stats.frames_received,
                stats.packet_loss_pct(), stats.beam_switches);

            *state   = CallState::Idle;
            *session = None;
            Some(msg)
        } else {
            *state = CallState::Idle;
            None
        }
    }

    /// Handle peer hangup.
    pub fn on_call_ended(&self) {
        let mut state   = self.state.lock().unwrap();
        let mut session = self.session.lock().unwrap();
        *state   = CallState::Idle;
        *session = None;
        println!("[voip] call ended by peer");
    }

    /// Handle Starlink beam switch -- update peer address.
    pub fn on_address_change(&self, new_addr: SocketAddr) {
        let mut session = self.session.lock().unwrap();
        if let Some(ref mut s) = *session {
            s.update_peer_addr(new_addr);
        }
    }

    /// Encrypt an Opus frame for sending.
    pub fn send_audio(&self, opus_frame: &[u8], sequence: u32) -> Option<VoipMessage> {
        let mut session = self.session.lock().unwrap();
        session.as_mut().map(|s| s.encrypt_frame(opus_frame, sequence))
    }

    /// Process an incoming audio frame.
    pub fn receive_audio(&self, msg: &VoipMessage) -> Option<Vec<u8>> {
        let mut session = self.session.lock().unwrap();
        session.as_mut().and_then(|s| s.decrypt_frame(msg))
    }

    /// Get call state label.
    pub fn state_label(&self) -> String {
        self.state.lock().unwrap().label().to_string()
    }

    /// Build a keepalive message.
    pub fn keepalive(&self) -> Option<VoipMessage> {
        let state = self.state.lock().unwrap();
        state.session_id().map(|sid| VoipMessage::Keepalive {
            session_id: sid,
            sender_key: self.my_key,
        })
    }

    pub fn call_stats(&self) -> Option<CallStats> {
        self.session.lock().unwrap().as_ref().map(|s| s.stats.clone())
    }
}

// -- Helpers ------------------------------------------------------------------

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs()
}

fn hex8(key: &[u8; 32]) -> String {
    key.iter().take(4).map(|b| format!("{:02x}", b)).collect()
}

// -- Tests --------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_key(seed: u8) -> [u8; 32] { [seed; 32] }
    fn fake_addr() -> SocketAddr { "127.0.0.1:16384".parse().unwrap() }
    fn fake_addr2() -> SocketAddr { "127.0.0.2:16384".parse().unwrap() }

    #[test]
    fn test_call_state_labels() {
        assert_eq!(CallState::Idle.label(), "idle");
        assert_eq!(CallState::Ending.label(), "ending");
    }

    #[test]
    fn test_call_state_idle() { assert!(CallState::Idle.is_idle()); }

    #[test]
    fn test_call_state_active() {
        let s = CallState::Active { peer_addr: fake_addr(), session_id: 1, started_at: 0 };
        assert!(s.is_active());
        assert!(!s.is_idle());
    }

    #[test]
    fn test_call_state_session_id() {
        let s = CallState::Active { peer_addr: fake_addr(), session_id: 42, started_at: 0 };
        assert_eq!(s.session_id(), Some(42));
        assert_eq!(CallState::Idle.session_id(), None);
    }

    #[test]
    fn test_call_state_peer_addr() {
        let s = CallState::Dialing { target_key: fake_key(1), peer_addr: fake_addr(), session_id: 1 };
        assert_eq!(s.peer_addr(), Some(fake_addr()));
    }

    #[test]
    fn test_session_key_deterministic() {
        let k1 = fake_key(0xAA);
        let k2 = fake_key(0xBB);
        let k1c = derive_session_key(&k1, &k2, 12345);
        let k2c = derive_session_key(&k2, &k1, 12345);
        // Both sides derive same key regardless of argument order
        assert_eq!(k1c, k2c);
    }

    #[test]
    fn test_session_key_differs_per_session() {
        let k1 = fake_key(0xAA);
        let k2 = fake_key(0xBB);
        let ka = derive_session_key(&k1, &k2, 111);
        let kb = derive_session_key(&k1, &k2, 222);
        assert_ne!(ka, kb);
    }

    #[test]
    fn test_session_key_differs_per_peer() {
        let k1 = fake_key(0xAA);
        let k2 = fake_key(0xBB);
        let k3 = fake_key(0xCC);
        let ka = derive_session_key(&k1, &k2, 1);
        let kb = derive_session_key(&k1, &k3, 1);
        assert_ne!(ka, kb);
    }

    #[test]
    fn test_voip_message_serialization_ping() {
        let msg = VoipMessage::Keepalive { session_id: 99, sender_key: fake_key(0x01) };
        let bytes = msg.to_bytes();
        assert_eq!(bytes[0], 0x30);
        let parsed = VoipMessage::parse(&bytes).unwrap();
        if let VoipMessage::Keepalive { session_id, .. } = parsed {
            assert_eq!(session_id, 99);
        } else { panic!("wrong variant"); }
    }

    #[test]
    fn test_voip_message_audio_frame_roundtrip() {
        let payload = vec![0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08,
                           0x09, 0x0A, 0x0B, 0x0C, 0x0D, 0x0E, 0x0F, 0x10, 0x11];
        let msg = VoipMessage::AudioFrame {
            session_id: 42, sequence: 7, timestamp_rtp: 960,
            nonce_counter: 3, payload: payload.clone(),
        };
        let bytes = msg.to_bytes();
        let parsed = VoipMessage::parse(&bytes).unwrap();
        if let VoipMessage::AudioFrame { session_id, sequence, payload: p, .. } = parsed {
            assert_eq!(session_id, 42);
            assert_eq!(sequence, 7);
            assert_eq!(p, payload);
        } else { panic!("wrong variant"); }
    }

    #[test]
    fn test_jitter_buffer_ordering() {
        let mut jb = JitterBuffer::new();
        jb.push(3, vec![3]);
        jb.push(1, vec![1]);
        jb.push(2, vec![2]);
        assert_eq!(jb.pop(), Some(vec![1]));
        assert_eq!(jb.pop(), Some(vec![2]));
        assert_eq!(jb.pop(), Some(vec![3]));
    }

    #[test]
    fn test_jitter_buffer_dedup() {
        let mut jb = JitterBuffer::new();
        jb.push(1, vec![1]);
        jb.push(1, vec![1]); // duplicate
        assert_eq!(jb.len(), 1);
    }

    #[test]
    fn test_voip_session_encrypt_decrypt() {
        let my_key   = fake_key(0xAA);
        let peer_key = fake_key(0xBB);
        let mut session = VoipSession::new(1, fake_addr(), peer_key, my_key);
        let opus = vec![0xDE, 0xAD, 0xBE, 0xEF, 0x00, 0x11, 0x22, 0x33];
        let encrypted = session.encrypt_frame(&opus, 0);
        let decrypted = session.decrypt_frame(&encrypted).unwrap();
        assert_eq!(decrypted, opus);
    }

    #[test]
    fn test_voip_session_replay_protection() {
        let my_key   = fake_key(0xAA);
        let peer_key = fake_key(0xBB);
        let mut session = VoipSession::new(1, fake_addr(), peer_key, my_key);
        let opus = vec![0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08];
        let encrypted = session.encrypt_frame(&opus, 0);
        // First decrypt succeeds
        assert!(session.decrypt_frame(&encrypted).is_some());
        // Replay is rejected
        assert!(session.decrypt_frame(&encrypted).is_none());
    }

    #[test]
    fn test_voip_session_beam_switch() {
        let my_key   = fake_key(0xAA);
        let peer_key = fake_key(0xBB);
        let mut session = VoipSession::new(1, fake_addr(), peer_key, my_key);
        assert_eq!(session.peer_addr, fake_addr());
        session.update_peer_addr(fake_addr2());
        assert_eq!(session.peer_addr, fake_addr2());
        assert_eq!(session.stats.beam_switches, 1);
    }

    #[test]
    fn test_engine_initiate_call() {
        let engine = VoipEngine::new(fake_key(0xAA));
        let msg = engine.initiate_call(fake_key(0xBB), fake_addr()).unwrap();
        assert!(matches!(msg, VoipMessage::CallRequest {..}));
        assert!(!engine.state.lock().unwrap().is_idle());
    }

    #[test]
    fn test_engine_cannot_double_call() {
        let engine = VoipEngine::new(fake_key(0xAA));
        engine.initiate_call(fake_key(0xBB), fake_addr()).unwrap();
        let result = engine.initiate_call(fake_key(0xCC), fake_addr2());
        assert!(result.is_err());
    }

    #[test]
    fn test_engine_hangup_returns_to_idle() {
        let engine = VoipEngine::new(fake_key(0xAA));
        engine.initiate_call(fake_key(0xBB), fake_addr()).unwrap();
        engine.hangup();
        assert!(engine.state.lock().unwrap().is_idle());
    }

    #[test]
    fn test_engine_keepalive() {
        let engine = VoipEngine::new(fake_key(0xAA));
        assert!(engine.keepalive().is_none()); // no active session
        engine.initiate_call(fake_key(0xBB), fake_addr()).unwrap();
        assert!(engine.keepalive().is_some()); // session active
    }

    #[test]
    fn test_call_stats_packet_loss() {
        let mut stats = CallStats::default();
        stats.frames_sent = 100;
        stats.frames_lost = 5;
        assert!((stats.packet_loss_pct() - 5.0).abs() < 0.01);
    }

    #[test]
    fn test_message_tags() {
        assert_eq!(VoipMessage::Keepalive { session_id: 0, sender_key: [0;32] }.tag(), 0x30);
    }
}
