// crypto.rs -- theOS ChaCha20-Poly1305 Encryption
//
// Replaces the previous placeholder (no encryption) with real AEAD encryption.
//
// Design:
//   - ChaCha20-Poly1305 for all message and RTP payload encryption
//   - Session keys derived from both parties' Ed25519 public keys + session ID
//     via SHA-256. This means both sides independently derive the same key
//     with no key exchange round-trip needed.
//   - Nonces are 12 bytes: 8-byte counter + 4-byte session tag.
//     Counter MUST be incremented for every message. Nonce reuse breaks
//     AEAD security -- this is flagged explicitly in every encrypt call.
//
// Security assumptions (flag for audit):
//   - Ed25519 public keys are already authenticated via the identity layer.
//     The session key derivation trusts that both pubkeys are genuine.
//   - Nonce counter is in-memory only. On crash/restart a new session_id
//     MUST be generated to avoid nonce reuse.
//   - This module does NOT implement forward secrecy. That requires a
//     Diffie-Hellman exchange (X25519). Flagged for Phase 6 hardening.

use chacha20poly1305::{
    ChaCha20Poly1305, Key, Nonce,
    aead::{Aead, AeadCore, KeyInit, OsRng},
};
use sha2::{Sha256, Digest};

// ── Session key derivation ────────────────────────────────────────────────────

/// Derive a 32-byte session key from two Ed25519 public keys and a session ID.
///
/// Both sides call this independently and get the same key because:
///   - Keys are sorted before hashing (order-independent)
///   - session_id is agreed during the INVITE handshake
///
/// Security assumption: pubkeys are authenticated by the identity layer.
/// This is NOT a Diffie-Hellman exchange -- no forward secrecy.
pub fn derive_session_key(
    my_pubkey:    &[u8; 32],
    their_pubkey: &[u8; 32],
    session_id:   u64,
) -> [u8; 32] {
    let mut hasher = Sha256::new();

    // Sort keys so both sides derive the same hash regardless of who calls first
    // Security assumption: XOR comparison is sufficient for ordering here
    // since we only need consistency, not cryptographic ordering.
    if my_pubkey <= their_pubkey {
        hasher.update(my_pubkey);
        hasher.update(their_pubkey);
    } else {
        hasher.update(their_pubkey);
        hasher.update(my_pubkey);
    }

    hasher.update(session_id.to_le_bytes());
    hasher.update(b"theos-session-v1"); // domain separation

    hasher.finalize().into()
}

// ── Nonce construction ────────────────────────────────────────────────────────

/// Build a 12-byte ChaCha20-Poly1305 nonce from a message counter.
///
/// Layout: [counter: 8 bytes LE] [session_tag: 4 bytes LE]
/// session_tag is the low 4 bytes of session_id -- ties nonce to this session.
///
/// CRITICAL: counter MUST be unique per message within a session.
/// Nonce reuse with the same key breaks AEAD and leaks plaintext.
pub fn build_nonce(counter: u64, session_id: u64) -> [u8; 12] {
    let mut nonce = [0u8; 12];
    nonce[..8].copy_from_slice(&counter.to_le_bytes());
    nonce[8..].copy_from_slice(&(session_id as u32).to_le_bytes());
    nonce
}

// ── Encrypt ───────────────────────────────────────────────────────────────────

/// Encrypt plaintext with ChaCha20-Poly1305.
///
/// Returns ciphertext with Poly1305 authentication tag appended (16 bytes).
/// Total output length = plaintext.len() + 16.
///
/// CRITICAL: never reuse (key, counter) pair. Increment counter after every call.
pub fn encrypt(
    key:       &[u8; 32],
    counter:   u64,
    session_id: u64,
    plaintext: &[u8],
) -> Result<Vec<u8>, CryptoError> {
    let cipher    = ChaCha20Poly1305::new(Key::from_slice(key));
    let nonce_arr = build_nonce(counter, session_id);
    let nonce     = Nonce::from_slice(&nonce_arr);

    cipher.encrypt(nonce, plaintext)
        .map_err(|_| CryptoError::EncryptFailed)
}

// ── Decrypt ───────────────────────────────────────────────────────────────────

/// Decrypt and authenticate ciphertext with ChaCha20-Poly1305.
///
/// Returns plaintext on success. Returns Err if authentication fails --
/// the packet was tampered with or the wrong key/nonce was used.
/// NEVER use output of a failed decrypt.
pub fn decrypt(
    key:        &[u8; 32],
    counter:    u64,
    session_id: u64,
    ciphertext: &[u8],
) -> Result<Vec<u8>, CryptoError> {
    let cipher    = ChaCha20Poly1305::new(Key::from_slice(key));
    let nonce_arr = build_nonce(counter, session_id);
    let nonce     = Nonce::from_slice(&nonce_arr);

    cipher.decrypt(nonce, ciphertext)
        .map_err(|_| CryptoError::DecryptFailed)
}

// ── Error type ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub enum CryptoError {
    EncryptFailed,
    DecryptFailed,
}

impl std::fmt::Display for CryptoError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            CryptoError::EncryptFailed => write!(f, "encryption failed"),
            CryptoError::DecryptFailed => write!(f, "decryption failed -- bad key, nonce, or tampered ciphertext"),
        }
    }
}

// ── Session ───────────────────────────────────────────────────────────────────

/// A live encrypted session between two theOS devices.
///
/// Owns the session key and tracks the send counter.
/// Receive counter is tracked per-sender by the caller to handle
/// out-of-order RTP packets.
pub struct CryptoSession {
    key:          [u8; 32],
    session_id:   u64,
    send_counter: u64,
}

impl CryptoSession {
    /// Create a new session. Both sides call this with the same arguments
    /// and get the same key via derive_session_key.
    pub fn new(
        my_pubkey:    &[u8; 32],
        their_pubkey: &[u8; 32],
        session_id:   u64,
    ) -> Self {
        let key = derive_session_key(my_pubkey, their_pubkey, session_id);
        println!(
            "[crypto] session established -- id:{} key:{}...",
            session_id,
            hex_short(&key),
        );
        Self { key, session_id, send_counter: 0 }
    }

    /// Encrypt an outgoing packet. Increments send counter automatically.
    /// CRITICAL: send_counter must never wrap. At u64::MAX packets, rotate session.
    pub fn encrypt_packet(&mut self, plaintext: &[u8]) -> Result<EncryptedPacket, CryptoError> {
        let counter = self.send_counter;
        let ciphertext = encrypt(&self.key, counter, self.session_id, plaintext)?;
        self.send_counter += 1;
        Ok(EncryptedPacket { ciphertext, counter })
    }

    /// Decrypt an incoming packet using the counter from the packet header.
    pub fn decrypt_packet(&self, packet: &EncryptedPacket) -> Result<Vec<u8>, CryptoError> {
        decrypt(&self.key, packet.counter, self.session_id, &packet.ciphertext)
    }

    pub fn session_id(&self) -> u64 { self.session_id }
    pub fn send_counter(&self) -> u64 { self.send_counter }
}

/// An encrypted packet ready for transmission.
/// counter is sent in the packet header so the receiver can build the same nonce.
#[derive(Debug, Clone)]
pub struct EncryptedPacket {
    pub ciphertext: Vec<u8>,
    pub counter:    u64,
}

fn hex_short(bytes: &[u8]) -> String {
    bytes[..4].iter().map(|b| format!("{:02x}", b)).collect()
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn alice() -> [u8; 32] { [0xAAu8; 32] }
    fn bob()   -> [u8; 32] { [0xBBu8; 32] }

    // Session key derivation

    #[test]
    fn test_session_key_deterministic() {
        let k1 = derive_session_key(&alice(), &bob(), 42);
        let k2 = derive_session_key(&alice(), &bob(), 42);
        assert_eq!(k1, k2);
    }

    #[test]
    fn test_session_key_symmetric() {
        // Both sides derive same key regardless of argument order
        let k1 = derive_session_key(&alice(), &bob(), 42);
        let k2 = derive_session_key(&bob(), &alice(), 42);
        assert_eq!(k1, k2);
    }

    #[test]
    fn test_session_key_differs_by_session_id() {
        let k1 = derive_session_key(&alice(), &bob(), 1);
        let k2 = derive_session_key(&alice(), &bob(), 2);
        assert_ne!(k1, k2);
    }

    #[test]
    fn test_session_key_differs_by_peer() {
        let carol = [0xCCu8; 32];
        let k1 = derive_session_key(&alice(), &bob(), 1);
        let k2 = derive_session_key(&alice(), &carol, 1);
        assert_ne!(k1, k2);
    }

    #[test]
    fn test_session_key_is_32_bytes() {
        let k = derive_session_key(&alice(), &bob(), 0);
        assert_eq!(k.len(), 32);
    }

    // Nonce construction

    #[test]
    fn test_nonce_is_12_bytes() {
        let n = build_nonce(0, 0);
        assert_eq!(n.len(), 12);
    }

    #[test]
    fn test_nonce_differs_by_counter() {
        let n1 = build_nonce(0, 42);
        let n2 = build_nonce(1, 42);
        assert_ne!(n1, n2);
    }

    #[test]
    fn test_nonce_differs_by_session() {
        let n1 = build_nonce(0, 1);
        let n2 = build_nonce(0, 2);
        assert_ne!(n1, n2);
    }

    // Encrypt / decrypt roundtrip

    #[test]
    fn test_encrypt_decrypt_roundtrip() {
        let key = derive_session_key(&alice(), &bob(), 1);
        let plaintext = b"hello theOS";
        let ciphertext = encrypt(&key, 0, 1, plaintext).unwrap();
        let recovered  = decrypt(&key, 0, 1, &ciphertext).unwrap();
        assert_eq!(recovered, plaintext);
    }

    #[test]
    fn test_ciphertext_longer_than_plaintext() {
        let key = derive_session_key(&alice(), &bob(), 1);
        let plaintext = b"test";
        let ciphertext = encrypt(&key, 0, 1, plaintext).unwrap();
        // Poly1305 tag adds 16 bytes
        assert_eq!(ciphertext.len(), plaintext.len() + 16);
    }

    #[test]
    fn test_ciphertext_differs_from_plaintext() {
        let key = derive_session_key(&alice(), &bob(), 1);
        let plaintext = b"secret message";
        let ciphertext = encrypt(&key, 0, 1, plaintext).unwrap();
        assert_ne!(&ciphertext[..plaintext.len()], plaintext);
    }

    #[test]
    fn test_wrong_key_fails_decrypt() {
        let key1 = derive_session_key(&alice(), &bob(), 1);
        let key2 = derive_session_key(&alice(), &bob(), 2); // different session
        let ciphertext = encrypt(&key1, 0, 1, b"secret").unwrap();
        assert_eq!(decrypt(&key2, 0, 1, &ciphertext), Err(CryptoError::DecryptFailed));
    }

    #[test]
    fn test_wrong_counter_fails_decrypt() {
        let key = derive_session_key(&alice(), &bob(), 1);
        let ciphertext = encrypt(&key, 0, 1, b"secret").unwrap();
        assert_eq!(decrypt(&key, 1, 1, &ciphertext), Err(CryptoError::DecryptFailed));
    }

    #[test]
    fn test_tampered_ciphertext_fails_decrypt() {
        let key = derive_session_key(&alice(), &bob(), 1);
        let mut ciphertext = encrypt(&key, 0, 1, b"secret").unwrap();
        ciphertext[0] ^= 0xFF; // flip bits
        assert_eq!(decrypt(&key, 0, 1, &ciphertext), Err(CryptoError::DecryptFailed));
    }

    #[test]
    fn test_empty_plaintext() {
        let key = derive_session_key(&alice(), &bob(), 1);
        let ciphertext = encrypt(&key, 0, 1, b"").unwrap();
        let recovered  = decrypt(&key, 0, 1, &ciphertext).unwrap();
        assert_eq!(recovered, b"");
    }

    #[test]
    fn test_large_payload() {
        let key = derive_session_key(&alice(), &bob(), 1);
        let plaintext = vec![0xABu8; 1400]; // typical max RTP payload
        let ciphertext = encrypt(&key, 0, 1, &plaintext).unwrap();
        let recovered  = decrypt(&key, 0, 1, &ciphertext).unwrap();
        assert_eq!(recovered, plaintext);
    }

    // CryptoSession

    #[test]
    fn test_session_encrypt_decrypt() {
        let mut alice_session = CryptoSession::new(&alice(), &bob(), 99);
        let bob_session       = CryptoSession::new(&bob(), &alice(), 99);

        let packet    = alice_session.encrypt_packet(b"hello bob").unwrap();
        let recovered = bob_session.decrypt_packet(&packet).unwrap();
        assert_eq!(recovered, b"hello bob");
    }

    #[test]
    fn test_session_counter_increments() {
        let mut session = CryptoSession::new(&alice(), &bob(), 1);
        session.encrypt_packet(b"msg1").unwrap();
        session.encrypt_packet(b"msg2").unwrap();
        assert_eq!(session.send_counter(), 2);
    }

    #[test]
    fn test_session_packets_have_unique_ciphertext() {
        let mut session = CryptoSession::new(&alice(), &bob(), 1);
        let p1 = session.encrypt_packet(b"same plaintext").unwrap();
        let p2 = session.encrypt_packet(b"same plaintext").unwrap();
        // Different counters -> different nonces -> different ciphertext
        assert_ne!(p1.ciphertext, p2.ciphertext);
        assert_ne!(p1.counter, p2.counter);
    }

    #[test]
    fn test_session_both_sides_same_key() {
        let alice_session = CryptoSession::new(&alice(), &bob(), 7);
        let bob_session   = CryptoSession::new(&bob(), &alice(), 7);
        assert_eq!(alice_session.key, bob_session.key);
    }
}
