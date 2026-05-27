// crypto/ratchet/dh_ratchet.rs — DH ratchet (Double Ratchet, step 3)
//
// The protocol core. Combines the root key with per-turn X25519 rotation to
// provide POST-COMPROMISE SECURITY ("self-healing"): once a fresh DH step runs
// that an attacker did not observe, previously-leaked keys stop helping them.
//
// State: a root key + a sending ChainKey + a receiving ChainKey, plus our
// current ratchet keypair and the peer's current ratchet public key.
//
// Turn-taking: same-direction messages only advance the cheap symmetric chain.
// A direction change triggers a DH ratchet step (new keypair, DH, kdf_rk).
//
// ROLE ASYMMETRY (the easy-to-get-wrong part):
//   - Initiator (ran X3DH) knows peer's initial ratchet pubkey up front and
//     immediately steps to create a sending chain.
//   - Responder starts with no sending chain; it is created on first receive.
//
// SCOPE: this implements correct STEADY-STATE turn-taking. Out-of-order
// delivery that spans a DH rotation (skipped keys across chains) is step 5
// and is explicitly NOT handled here — see next_receiving_key's note.
use std::collections::HashMap;

use super::dh::{DhKeyPair, DhPublic, kdf_rk};
use super::symmetric::{ChainKey, MessageKey};

/// Max message keys retained for out-of-order delivery (satellite-tuned).
const MAX_SKIP: u32 = 2000;
/// Max forward jump in a single message before rejecting (DoS bound).
const MAX_JUMP: u32 = 2000;

/// Receive-path errors. Rejection is a real outcome, not a panic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RatchetError {
    /// Skip count or stored-key limit exceeded (likely a malicious index).
    SkipLimitExceeded,
    /// The key for this message is unavailable (in the past, never stored).
    MessageKeyGone,
}

/// Header sent alongside each message so the receiver can ratchet correctly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RatchetHeader {
    /// Sender's current ratchet public key.
    pub ratchet_pub: DhPublic,
    /// Index of this message within the sender's current sending chain.
    pub msg_index: u32,
    /// Length of the sender's previous sending chain (Signal's "pn").
    /// Used by receiver to stash keys across a DH ratchet rotation.
    pub pn: u32,
}

pub struct DhRatchet {
    root_key: [u8; 32],
    self_kp: DhKeyPair,
    peer_pub: Option<DhPublic>,
    send_chain: Option<ChainKey>,
    recv_chain: Option<ChainKey>,
    /// Skipped message keys for out-of-order delivery: (ratchet_pub, index) -> key.
    skipped: HashMap<(DhPublic, u32), MessageKey>,
    /// Number of messages sent on the previous sending chain (Signal's "pn"),
    /// stamped into headers so the receiver can stash across a rotation.
    prev_send_count: u32,
}

impl DhRatchet {
    /// Initiator side. Called by the party that ran X3DH and therefore already
    /// knows the shared root key AND the peer's initial ratchet public key.
    /// Immediately performs a DH step to establish the sending chain.
    pub fn new_initiator(root_key: [u8; 32], peer_pub: DhPublic) -> Self {
        let self_kp = DhKeyPair::generate();
        let dh = self_kp.diffie_hellman(&peer_pub);
        let (new_root, send_ck) = kdf_rk(&root_key, &dh);
        Self {
            root_key: new_root,
            self_kp,
            peer_pub: Some(peer_pub),
            send_chain: Some(ChainKey::new(send_ck)),
            recv_chain: None,
            skipped: HashMap::new(),
            prev_send_count: 0,
        }
    }

    /// Responder side. Called by the party that ran X3DH and received the
    /// initiator's first message (carrying their ratchet key). Waits for the
    /// first receive to create the sending chain.
    pub fn new_responder(root_key: [u8; 32], self_kp: DhKeyPair) -> Self {
        Self {
            root_key,
            self_kp,
            peer_pub: None,
            send_chain: None,
            recv_chain: None,
            skipped: HashMap::new(),
            prev_send_count: 0,
        }
    }

    /// The public key the receiver should use to key the next DH ratchet step.
    pub fn ratchet_public(&self) -> DhPublic {
        self.self_kp.public_bytes()
    }

    pub fn next_sending_key(&mut self) -> Option<(MessageKey, RatchetHeader)> {
        let chain = self.send_chain.as_mut()?;
        let (mk, idx) = chain.next_message_key();
        Some((
            mk,
            RatchetHeader { ratchet_pub: self.ratchet_public(), msg_index: idx, pn: self.prev_send_count },
        ))
    }

    pub fn next_receiving_key(&mut self, header: &RatchetHeader) -> Result<MessageKey, RatchetError> {
        // 1. Check skipped store first (out-of-order arrival of already-stashed key)
        if let Some(mk) = self.skipped.remove(&(header.ratchet_pub, header.msg_index)) {
            return Ok(mk);
        }

        // 2. Handle new peer key (DH ratchet rotation) with stashing of old chain
        let is_new_peer_key = match self.peer_pub {
            Some(p) => p != header.ratchet_pub,
            None => true,
        };
        if is_new_peer_key {
            // Before rotating, stash remaining messages from old recv chain up to pn.
            // (Only if we have an old chain; responder starts with None.)
            if self.recv_chain.is_some() {
                self.advance_and_stash(header.pn)?;
            }
            self.dh_ratchet_step(header.ratchet_pub);
        }

        // 3. Get current receiving chain and check bounds
        let current_idx = {
            let chain = self
                .recv_chain
                .as_ref()
                .ok_or(RatchetError::MessageKeyGone)?;
            chain.index()
        };

        // 4. Handle out-of-order within same chain (before accessing chain mutably)
        if header.msg_index > current_idx {
            let gap = header.msg_index - current_idx;
            if gap > MAX_JUMP {
                return Err(RatchetError::SkipLimitExceeded);
            }
            self.advance_and_stash(header.msg_index)?;
        } else if header.msg_index < current_idx {
            return Err(RatchetError::MessageKeyGone);
        }

        // 5. Get the target message key
        let chain = self
            .recv_chain
            .as_mut()
            .ok_or(RatchetError::MessageKeyGone)?;
        let (mk, _idx) = chain.next_message_key();
        Ok(mk)
    }

    /// One DH ratchet step on receiving a new peer ratchet key:
    ///   1. DH(our current key, peer new key) -> kdf_rk -> new recv chain
    ///   2. rotate our keypair
    ///   3. DH(our new key, peer new key) -> kdf_rk -> new send chain
    /// After this, replying advances a chain the attacker has never seen.
    fn dh_ratchet_step(&mut self, peer_new_pub: DhPublic) {
        // Step 1: receiving chain from our current key + peer's new key.
        let dh_recv = self.self_kp.diffie_hellman(&peer_new_pub);
        let (root_after_recv, recv_ck) = kdf_rk(&self.root_key, &dh_recv);
        self.recv_chain = Some(ChainKey::new(recv_ck));
        // Step 2: rotate our ratchet keypair.
        self.self_kp = DhKeyPair::generate();
        // Step 3: sending chain from our new key + peer's new key.
        // Before rotating, capture the old sending chain's message count for pn.
        if let Some(old_chain) = &self.send_chain {
            self.prev_send_count = old_chain.index();
        }
        let dh_send = self.self_kp.diffie_hellman(&peer_new_pub);
        let (root_after_send, send_ck) = kdf_rk(&root_after_recv, &dh_send);
        self.send_chain = Some(ChainKey::new(send_ck));
        self.root_key = root_after_send;
        self.peer_pub = Some(peer_new_pub);
    }

    /// Advance the current receiving chain up to `until`, storing each skipped key.
    /// Used both for same-chain skip-forward and for stashing before DH rotation.
    fn advance_and_stash(&mut self, until: u32) -> Result<(), RatchetError> {
        let peer_pub = self.peer_pub.ok_or(RatchetError::MessageKeyGone)?;
        let chain = self.recv_chain.as_mut().ok_or(RatchetError::MessageKeyGone)?;

        let current = chain.index();
        if until <= current {
            return Ok(());
        }

        let gap = until - current;
        if gap > MAX_SKIP {
            return Err(RatchetError::SkipLimitExceeded);
        }

        for _ in 0..gap {
            let (mk, idx) = chain.next_message_key();
            if self.skipped.len() >= MAX_SKIP as usize {
                return Err(RatchetError::SkipLimitExceeded);
            }
            self.skipped.insert((peer_pub, idx), mk);
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pair() -> (DhRatchet, DhRatchet) {
        let root = [0x42u8; 32];
        let responder_kp = DhKeyPair::generate();
        let responder_pub = responder_kp.public_bytes();
        let initiator = DhRatchet::new_initiator(root, responder_pub);
        let responder = DhRatchet::new_responder(root, responder_kp);
        (initiator, responder)
    }

    #[test]
    fn initiator_can_send_first() {
        let (mut alice, mut bob) = pair();
        let (ak, hdr) = alice.next_sending_key().expect("alice sends");
        let bk = bob.next_receiving_key(&hdr).unwrap();
        assert_eq!(ak.as_bytes(), bk.as_bytes());
    }

    #[test]
    fn responder_cannot_send_before_receiving() {
        let (_alice, mut bob) = pair();
        assert!(bob.next_sending_key().is_none());
    }

    #[test]
    fn full_round_trip_heals() {
        let (mut alice, mut bob) = pair();
        // Alice -> Bob
        let (ak0, h0) = alice.next_sending_key().unwrap();
        let bk0 = bob.next_receiving_key(&h0).unwrap();
        assert_eq!(ak0.as_bytes(), bk0.as_bytes());
        // Bob now has a sending chain (created during his receive step).
        let (bk_reply, h_reply) = bob.next_sending_key().expect("bob replies");
        let ak_reply = alice.next_receiving_key(&h_reply).unwrap();
        assert_eq!(bk_reply.as_bytes(), ak_reply.as_bytes());
    }

    #[test]
    fn multiple_sends_before_reply() {
        let (mut alice, mut bob) = pair();
        // Alice sends three in a row; only the symmetric chain advances.
        let (a0, h0) = alice.next_sending_key().unwrap();
        let (a1, h1) = alice.next_sending_key().unwrap();
        let (a2, h2) = alice.next_sending_key().unwrap();
        assert_eq!(h0.ratchet_pub, h1.ratchet_pub); // same ratchet key
        assert_eq!(h1.ratchet_pub, h2.ratchet_pub);
        assert_eq!((h0.msg_index, h1.msg_index, h2.msg_index), (0, 1, 2));
        let b0 = bob.next_receiving_key(&h0).unwrap();
        let b1 = bob.next_receiving_key(&h1).unwrap();
        let b2 = bob.next_receiving_key(&h2).unwrap();
        assert_eq!(a0.as_bytes(), b0.as_bytes());
        assert_eq!(a1.as_bytes(), b1.as_bytes());
        assert_eq!(a2.as_bytes(), b2.as_bytes());
    }

    #[test]
    fn sustained_conversation_stays_in_sync() {
        // Alternating turns for several rounds; keys must match every time.
        let (mut alice, mut bob) = pair();
        for _ in 0..4 {
            let (ak, ah) = alice.next_sending_key().unwrap();
            assert_eq!(ak.as_bytes(), bob.next_receiving_key(&ah).unwrap().as_bytes());
            let (bk, bh) = bob.next_sending_key().unwrap();
            assert_eq!(bk.as_bytes(), alice.next_receiving_key(&bh).unwrap().as_bytes());
        }
    }

    #[test]
    fn ratchet_key_rotates_on_direction_change() {
        let (mut alice, mut bob) = pair();
        let (_a0, h0) = alice.next_sending_key().unwrap();
        bob.next_receiving_key(&h0).unwrap();
        let (_b0, hb) = bob.next_sending_key().unwrap();
        // Bob's ratchet key (after his receive step rotated it) differs from
        // the key Alice used.
        assert_ne!(h0.ratchet_pub, hb.ratchet_pub);
    }

    #[test]
    fn out_of_order_within_chain() {
        let (mut alice, mut bob) = pair();
        // Alice sends three messages
        let (a0, h0) = alice.next_sending_key().unwrap();
        let (a1, h1) = alice.next_sending_key().unwrap();
        let (a2, h2) = alice.next_sending_key().unwrap();
        
        // Bob receives out of order: 2, 0, 1
        let b2 = bob.next_receiving_key(&h2).unwrap();
        let b0 = bob.next_receiving_key(&h0).unwrap();
        let b1 = bob.next_receiving_key(&h1).unwrap();
        
        assert_eq!(a0.as_bytes(), b0.as_bytes());
        assert_eq!(a1.as_bytes(), b1.as_bytes());
        assert_eq!(a2.as_bytes(), b2.as_bytes());
    }

    #[test]
    fn dos_rejection_max_jump() {
        let (mut alice, mut bob) = pair();
        // Alice sends a normal first message so Bob establishes a recv chain.
        let (_a0, h0) = alice.next_sending_key().unwrap();
        bob.next_receiving_key(&h0).unwrap();

        // Craft a header on the same chain claiming an impossibly large index.
        let (_a1, mut evil_hdr) = alice.next_sending_key().unwrap();
        evil_hdr.msg_index = MAX_JUMP + 5;

        // Bob receives it — the jump exceeds MAX_JUMP, so reject.
        let result = bob.next_receiving_key(&evil_hdr);
        assert_eq!(result, Err(RatchetError::SkipLimitExceeded));
    }

    #[test]
    fn dos_rejection_store_full() {
        let (mut alice, mut bob) = pair();
        // Alice sends a normal first message so Bob establishes a recv chain at index 1.
        let (_a0, h0) = alice.next_sending_key().unwrap();
        bob.next_receiving_key(&h0).unwrap();

        // Craft a header whose index forces a skip-gap strictly greater than the
        // bound (gap > MAX_SKIP). Bob's chain is at 1, so an index of MAX_SKIP + 2
        // means a gap of MAX_SKIP + 1 — over the limit, must be rejected.
        let (_a1, mut far_hdr) = alice.next_sending_key().unwrap();
        far_hdr.msg_index = MAX_SKIP + 2;

        let result = bob.next_receiving_key(&far_hdr);
        assert_eq!(result, Err(RatchetError::SkipLimitExceeded));
    }
}
