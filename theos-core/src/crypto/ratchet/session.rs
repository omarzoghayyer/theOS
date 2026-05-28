// crypto/ratchet/session.rs — ratchet-backed message session (text path)
//
// The seam between plaintext and ciphertext for TEXT messages. Owns a
// DhRatchet and turns it into a simple encrypt/decrypt API:
//
//   alice.encrypt_outgoing(b"hi")  -> (RatchetHeader, ciphertext)
//   bob.decrypt_incoming(&header, &ciphertext) -> b"hi"
//
// DISTINCT from crypto::CryptoSession, which is the VoIP path: that type uses
// one STATIC key per call (correct for ephemeral RTP audio) and has no forward
// secrecy. This type gives every message a fresh key from the Double Ratchet,
// providing forward secrecy and post-compromise security for durable text.
//
// AEAD nonce convention: each MessageKey from the ratchet is used EXACTLY ONCE,
// so the (counter, session_id) nonce inputs to crypto::encrypt are both fixed
// at 0. Uniqueness comes from the key itself, not the nonce — the textbook
// convention for per-message keys. Reusing a (key, nonce) pair is structurally
// impossible here because the ratchet never emits the same key twice.

use super::dh_ratchet::{DhRatchet, RatchetError, RatchetHeader, RatchetSerdeError};
use crate::crypto::{self, CryptoError};

/// Errors from a ratchet session. Two distinct failure domains:
/// the ratchet (key unavailable / skip limit) and the AEAD (decrypt failed).
#[derive(Debug, Clone, PartialEq)]
pub enum SessionError {
    /// The ratchet could not produce the key (out of bounds, key gone).
    Ratchet(RatchetError),
    /// AEAD encryption or decryption failed (e.g. tampered ciphertext).
    Crypto(CryptoError),
}

impl From<RatchetError> for SessionError {
    fn from(e: RatchetError) -> Self { SessionError::Ratchet(e) }
}
impl From<CryptoError> for SessionError {
    fn from(e: CryptoError) -> Self { SessionError::Crypto(e) }
}

/// Fixed nonce inputs for the per-message-key AEAD (see module note).
const RATCHET_AEAD_COUNTER: u64 = 0;
const RATCHET_AEAD_SESSION_ID: u64 = 0;

/// A ratchet-backed text-message session with one peer.
pub struct RatchetSession {
    ratchet: DhRatchet,
}

impl RatchetSession {
    /// Wrap a DhRatchet produced by X3DH (x3dh_initiate / x3dh_respond both
    /// return a wired ratchet). No distinction is needed at this layer between
    /// initiator and responder — the ratchet already carries that asymmetry.
    pub fn new(ratchet: DhRatchet) -> Self {
        Self { ratchet }
    }

    /// Encrypt a plaintext message. Returns the header to transmit alongside
    /// the ciphertext; the receiver feeds it back into `decrypt_incoming`.
    ///
    /// Returns Ratchet error only if this side has no sending chain yet (a
    /// responder that has never received) — in normal flow the chain exists.
    pub fn encrypt_outgoing(
        &mut self,
        plaintext: &[u8],
    ) -> Result<(RatchetHeader, Vec<u8>), SessionError> {
        let (msg_key, header) = self
            .ratchet
            .next_sending_key()
            .ok_or(SessionError::Ratchet(RatchetError::MessageKeyGone))?;
        let ciphertext = crypto::encrypt(
            msg_key.as_bytes(),
            RATCHET_AEAD_COUNTER,
            RATCHET_AEAD_SESSION_ID,
            plaintext,
        )?;
        Ok((header, ciphertext))
    }

    /// Decrypt an incoming message using its header. Advances the receiving
    /// ratchet (handling rotation, out-of-order, and skipped keys internally).
    pub fn decrypt_incoming(
        &mut self,
        header: &RatchetHeader,
        ciphertext: &[u8],
    ) -> Result<Vec<u8>, SessionError> {
        let msg_key = self.ratchet.next_receiving_key(header)?;
        let plaintext = crypto::decrypt(
            msg_key.as_bytes(),
            RATCHET_AEAD_COUNTER,
            RATCHET_AEAD_SESSION_ID,
            ciphertext,
        )?;
        Ok(plaintext)
    }

    /// Serialize the underlying ratchet state for persistence. Output contains
    /// secret material; the caller must encrypt at rest.
    pub fn serialize(&self) -> Vec<u8> {
        self.ratchet.serialize()
    }

    /// Restore a session from serialized ratchet state.
    pub fn deserialize(bytes: &[u8]) -> Result<Self, RatchetSerdeError> {
        Ok(Self { ratchet: DhRatchet::deserialize(bytes)? })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::x3dh::handshake::{x3dh_initiate, x3dh_respond};
    use crate::crypto::x3dh::prekey::SignedPrekey;
    use crate::identity::keypair::KeyPair;

    // Wire up a real Alice/Bob pair via X3DH, return their two sessions.
    fn session_pair() -> (RatchetSession, RatchetSession) {
        use crate::crypto::ratchet::dh::DhKeyPair;

        let alice = KeyPair::generate();
        let bob = KeyPair::generate();

        // Build Bob's published bundle and reconstruct his prekey keypair,
        // mirroring the handshake module's own bob_setup helper.
        let sp = SignedPrekey::new(&bob);
        let bundle = sp.bundle(bob.x25519_identity_public());
        let bob_prekey_kp = DhKeyPair::from_private_bytes(sp.keypair().private_bytes());

        let (alice_ratchet, init_msg) =
            x3dh_initiate(&alice, &bundle, &bob.public).unwrap();
        let bob_ratchet = x3dh_respond(&bob, bob_prekey_kp, &init_msg);

        (RatchetSession::new(alice_ratchet), RatchetSession::new(bob_ratchet))
    }

    #[test]
    fn single_message_round_trips() {
        let (mut alice, mut bob) = session_pair();
        let (hdr, ct) = alice.encrypt_outgoing(b"hello bob").unwrap();
        let pt = bob.decrypt_incoming(&hdr, &ct).unwrap();
        assert_eq!(pt, b"hello bob");
    }

    #[test]
    fn ciphertext_is_not_plaintext() {
        let (mut alice, mut _bob) = session_pair();
        let (_hdr, ct) = alice.encrypt_outgoing(b"secret").unwrap();
        assert_ne!(&ct[..], b"secret");
    }

    #[test]
    fn multiple_messages_in_order() {
        let (mut alice, mut bob) = session_pair();
        for text in [b"one".as_slice(), b"two", b"three"] {
            let (hdr, ct) = alice.encrypt_outgoing(text).unwrap();
            let pt = bob.decrypt_incoming(&hdr, &ct).unwrap();
            assert_eq!(pt, text);
        }
    }

    #[test]
    fn full_duplex_conversation() {
        let (mut alice, mut bob) = session_pair();
        // Alice -> Bob
        let (h1, c1) = alice.encrypt_outgoing(b"hi bob").unwrap();
        assert_eq!(bob.decrypt_incoming(&h1, &c1).unwrap(), b"hi bob");
        // Bob -> Alice (reply, triggers rotation)
        let (h2, c2) = bob.encrypt_outgoing(b"hi alice").unwrap();
        assert_eq!(alice.decrypt_incoming(&h2, &c2).unwrap(), b"hi alice");
        // Alice -> Bob again
        let (h3, c3) = alice.encrypt_outgoing(b"how are you").unwrap();
        assert_eq!(bob.decrypt_incoming(&h3, &c3).unwrap(), b"how are you");
    }

    #[test]
    fn out_of_order_delivery() {
        let (mut alice, mut bob) = session_pair();
        let (h0, c0) = alice.encrypt_outgoing(b"msg0").unwrap();
        let (h1, c1) = alice.encrypt_outgoing(b"msg1").unwrap();
        let (h2, c2) = alice.encrypt_outgoing(b"msg2").unwrap();
        // Bob receives 2, 0, 1
        assert_eq!(bob.decrypt_incoming(&h2, &c2).unwrap(), b"msg2");
        assert_eq!(bob.decrypt_incoming(&h0, &c0).unwrap(), b"msg0");
        assert_eq!(bob.decrypt_incoming(&h1, &c1).unwrap(), b"msg1");
    }

    #[test]
    fn tampered_ciphertext_fails() {
        let (mut alice, mut bob) = session_pair();
        let (hdr, mut ct) = alice.encrypt_outgoing(b"authentic").unwrap();
        ct[0] ^= 0xff; // flip a byte
        let result = bob.decrypt_incoming(&hdr, &ct);
        assert_eq!(result, Err(SessionError::Crypto(CryptoError::DecryptFailed)));
    }

    #[test]
    fn session_persists_mid_conversation() {
        let (mut alice, mut bob) = session_pair();
        // Exchange a couple messages.
        let (h1, c1) = alice.encrypt_outgoing(b"first").unwrap();
        bob.decrypt_incoming(&h1, &c1).unwrap();

        // Persist Bob, restore him.
        let bob_bytes = bob.serialize();
        let mut bob_restored = RatchetSession::deserialize(&bob_bytes).unwrap();

        // Restored Bob continues the conversation seamlessly.
        let (h2, c2) = alice.encrypt_outgoing(b"second").unwrap();
        assert_eq!(bob_restored.decrypt_incoming(&h2, &c2).unwrap(), b"second");
    }
}
