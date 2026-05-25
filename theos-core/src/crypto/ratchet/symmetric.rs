// crypto/ratchet/symmetric.rs — Symmetric-key ratchet (Double Ratchet, step 2)
//
// Provides FORWARD SECRECY within a session. Wraps kdf_ck: holds a chain key,
// and each message key derivation advances the chain one step and discards the
// previous chain key. Because kdf_ck is one-way, compromising the current chain
// key reveals NO past message keys — they are unrecoverable.
//
// This layer produces only a 32-byte message key (+ its index). It does NOT
// encrypt — the existing crypto::encrypt / CryptoSession path consumes the key.
// Out-of-order delivery (skipped keys) is handled in a later step; the message
// index laid down here is what makes that possible.
//
// Security notes (flag for audit):
//   - MessageKey zeroizes on drop (cold-boot / RAM-dump mitigation).
//   - ChainKey advances forward only; no rewind. Old chain keys are dropped.

use zeroize::{Zeroize, ZeroizeOnDrop};

use super::dh::kdf_ck;

/// A single-message AEAD key. Zeroized on drop — never lingers in freed memory.
/// Deref to the raw 32 bytes for the existing crypto::encrypt path.
#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub struct MessageKey([u8; 32]);

impl MessageKey {
    /// Borrow the raw key bytes (e.g. to pass into crypto::encrypt).
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

// Deliberately NOT implementing Debug/Display — secret material must not be
// printable. (zeroize-derived types don't get Debug for free anyway.)

/// A symmetric-ratchet sending or receiving chain.
///
/// Holds the current chain key and the index of the NEXT message it will
/// produce. Advancing consumes the old chain key (overwritten in place).
pub struct ChainKey {
    key: [u8; 32],
    index: u32,
}

impl ChainKey {
    /// Start a chain from an initial 32-byte chain key (as produced by kdf_rk).
    pub fn new(initial: [u8; 32]) -> Self {
        Self { key: initial, index: 0 }
    }

    /// The index of the next message key this chain will produce.
    pub fn index(&self) -> u32 {
        self.index
    }

    /// Advance the chain one step: derive this message's key, roll the chain
    /// key forward, and discard the old one. Returns (message_key, message_index).
    ///
    /// Forward secrecy: after this returns, the chain key that produced earlier
    /// message keys no longer exists in memory.
    pub fn next_message_key(&mut self) -> (MessageKey, u32) {
        let (next_chain, msg_key) = kdf_ck(&self.key);
        // Overwrite the old chain key in place, then store the advanced one.
        self.key.zeroize();
        self.key = next_chain;
        let idx = self.index;
        self.index += 1;
        (MessageKey(msg_key), idx)
    }
}

impl Drop for ChainKey {
    fn drop(&mut self) {
        self.key.zeroize();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_index_increments() {
        let mut ck = ChainKey::new([1u8; 32]);
        let (_k0, i0) = ck.next_message_key();
        let (_k1, i1) = ck.next_message_key();
        let (_k2, i2) = ck.next_message_key();
        assert_eq!((i0, i1, i2), (0, 1, 2));
        assert_eq!(ck.index(), 3);
    }

    #[test]
    fn consecutive_message_keys_differ() {
        let mut ck = ChainKey::new([2u8; 32]);
        let (k0, _) = ck.next_message_key();
        let (k1, _) = ck.next_message_key();
        assert_ne!(k0.as_bytes(), k1.as_bytes());
    }

    #[test]
    fn two_chains_same_seed_produce_same_keys() {
        // Sender and receiver chains starting from the same chain key must
        // produce identical message keys at each index (this is how decryption
        // recovers the key).
        let seed = [9u8; 32];
        let mut send = ChainKey::new(seed);
        let mut recv = ChainKey::new(seed);
        for _ in 0..5 {
            let (sk, si) = send.next_message_key();
            let (rk, ri) = recv.next_message_key();
            assert_eq!(si, ri);
            assert_eq!(sk.as_bytes(), rk.as_bytes());
        }
    }

    #[test]
    fn different_seeds_diverge_immediately() {
        let mut a = ChainKey::new([1u8; 32]);
        let mut b = ChainKey::new([2u8; 32]);
        let (ka, _) = a.next_message_key();
        let (kb, _) = b.next_message_key();
        assert_ne!(ka.as_bytes(), kb.as_bytes());
    }

    #[test]
    fn message_key_is_clonable_for_use() {
        // as_bytes borrows; clone yields an independent zeroizing copy.
        let mut ck = ChainKey::new([7u8; 32]);
        let (k, _) = ck.next_message_key();
        let copy = k.clone();
        assert_eq!(k.as_bytes(), copy.as_bytes());
    }
}
