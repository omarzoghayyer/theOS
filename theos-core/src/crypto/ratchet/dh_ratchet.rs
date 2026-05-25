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

use super::dh::{DhKeyPair, DhPublic, kdf_rk};
use super::symmetric::{ChainKey, MessageKey};

/// Header sent alongside each message so the receiver can ratchet correctly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RatchetHeader {
    /// Sender's current ratchet public key.
    pub ratchet_pub: DhPublic,
    /// Index of this message within the sender's current sending chain.
    pub msg_index: u32,
}

pub struct DhRatchet {
    root_key: [u8; 32],
    self_kp: DhKeyPair,
    peer_pub: Option<DhPublic>,
    send_chain: Option<ChainKey>,
    recv_chain: Option<ChainKey>,
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
        }
    }

    /// Responder side. Called by the party whose published prekey was used.
    /// Knows the shared root key and holds the matching ratchet keypair, but
    /// has NO sending chain yet and does not know the peer's ratchet pubkey
    /// until the first message arrives.
    pub fn new_responder(root_key: [u8; 32], self_kp: DhKeyPair) -> Self {
        Self {
            root_key,
            self_kp,
            peer_pub: None,
            send_chain: None,
            recv_chain: None,
        }
    }

    /// Our current ratchet public key (goes in the header we send).
    pub fn ratchet_public(&self) -> DhPublic {
        self.self_kp.public_bytes()
    }

    /// Produce the next sending message key and the header to attach.
    ///
    /// Responder special-case: if we have no sending chain yet (never sent,
    /// and haven't received either), we cannot send. In normal flow the
    /// responder only sends after receiving, which creates the chain.
    pub fn next_sending_key(&mut self) -> Option<(MessageKey, RatchetHeader)> {
        let chain = self.send_chain.as_mut()?;
        let (mk, idx) = chain.next_message_key();
        Some((
            mk,
            RatchetHeader { ratchet_pub: self.ratchet_public(), msg_index: idx },
        ))
    }

    /// Process an incoming header and produce the message key to decrypt with.
    ///
    /// If the header carries a NEW peer ratchet pubkey (direction change), we
    /// perform a DH ratchet step: derive a new receiving chain from the peer's
    /// new key, then rotate our own keypair and derive a new SENDING chain so
    /// our next reply heals the ratchet.
    ///
    /// NOTE (step-5 scope): this assumes in-order delivery within a chain. A
    /// message arriving out of order across a rotation is not yet handled;
    /// skipped-key storage is the persistence step.
    pub fn next_receiving_key(&mut self, header: &RatchetHeader) -> MessageKey {
        let is_new_peer_key = match self.peer_pub {
            Some(p) => p != header.ratchet_pub,
            None => true,
        };

        if is_new_peer_key {
            self.dh_ratchet_step(header.ratchet_pub);
        }

        let chain = self
            .recv_chain
            .as_mut()
            .expect("recv chain exists after dh_ratchet_step");
        let (mk, _idx) = chain.next_message_key();
        mk
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
        let dh_send = self.self_kp.diffie_hellman(&peer_new_pub);
        let (root_after_send, send_ck) = kdf_rk(&root_after_recv, &dh_send);
        self.send_chain = Some(ChainKey::new(send_ck));

        self.root_key = root_after_send;
        self.peer_pub = Some(peer_new_pub);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Build a matched initiator/responder pair sharing a root key, the way
    // X3DH will wire them in step 4. Responder's keypair is what the initiator
    // points its first DH at.
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
        let bk = bob.next_receiving_key(&hdr);
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
        let bk0 = bob.next_receiving_key(&h0);
        assert_eq!(ak0.as_bytes(), bk0.as_bytes());

        // Bob now has a sending chain (created during his receive step).
        let (bk_reply, h_reply) = bob.next_sending_key().expect("bob replies");
        let ak_reply = alice.next_receiving_key(&h_reply);
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

        let b0 = bob.next_receiving_key(&h0);
        let b1 = bob.next_receiving_key(&h1);
        let b2 = bob.next_receiving_key(&h2);
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
            assert_eq!(ak.as_bytes(), bob.next_receiving_key(&ah).as_bytes());
            let (bk, bh) = bob.next_sending_key().unwrap();
            assert_eq!(bk.as_bytes(), alice.next_receiving_key(&bh).as_bytes());
        }
    }

    #[test]
    fn ratchet_key_rotates_on_direction_change() {
        let (mut alice, mut bob) = pair();
        let (_a0, h0) = alice.next_sending_key().unwrap();
        bob.next_receiving_key(&h0);
        let (_b0, hb) = bob.next_sending_key().unwrap();
        // Bob's ratchet key (after his receive step rotated it) differs from
        // the key Alice used.
        assert_ne!(h0.ratchet_pub, hb.ratchet_pub);
    }
}
