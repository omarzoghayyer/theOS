// crypto/x3dh/handshake.rs — X3DH session establishment (3-DH variant).
//
// Produces the shared root key that seeds the Double Ratchet, computed so both
// parties derive it independently without transmitting a secret.
//
// Three DHs (one-time prekeys deferred, so no DH4):
//   DH1 = DH(IK_initiator, SPK_responder)   authenticates initiator
//   DH2 = DH(EK_initiator, IK_responder)    authenticates responder
//   DH3 = DH(EK_initiator, SPK_responder)   forward secrecy
//   SK  = HKDF(DH1 || DH2 || DH3)
//
// The responder computes the mirror of each DH (own-private x peer-public) and
// gets the identical SK. Concatenation order is fixed and identical on both
// sides — a swap would silently desync the root keys.
//
// Public API returns a ready-to-use DhRatchet. The raw shared-secret derivation
// is a private fn so it can be tested directly for the both-sides-match property.

use hkdf::Hkdf;
use sha2::Sha256;

use crate::crypto::ratchet::dh::{DhKeyPair, DhPublic};
use crate::crypto::ratchet::dh_ratchet::DhRatchet;
use crate::crypto::x3dh::prekey::PrekeyBundle;
use crate::identity::keypair::KeyPair;

const X3DH_KDF_INFO: &[u8] = b"theos-x3dh-v1";

#[derive(Debug, Clone, PartialEq)]
pub enum X3dhError {
    /// The prekey bundle's signature did not verify against the claimed identity.
    BadPrekeySignature,
}

/// X3DH initial message: what the initiator sends so the responder can derive
/// the same shared secret. Public values only.
#[derive(Debug, Clone, PartialEq)]
pub struct InitialMessage {
    /// Initiator's X25519 identity public key.
    pub initiator_identity: DhPublic,
    /// Initiator's ephemeral public key (fresh per session).
    pub initiator_ephemeral: DhPublic,
}

/// Derive the raw 32-byte shared secret from the three DH outputs.
/// Order of concatenation is fixed; both sides must produce identical input.
fn derive_shared_secret(dh1: &[u8; 32], dh2: &[u8; 32], dh3: &[u8; 32]) -> [u8; 32] {
    let mut ikm = Vec::with_capacity(96);
    ikm.extend_from_slice(dh1);
    ikm.extend_from_slice(dh2);
    ikm.extend_from_slice(dh3);
    let hk = Hkdf::<Sha256>::new(None, &ikm);
    let mut sk = [0u8; 32];
    hk.expand(X3DH_KDF_INFO, &mut sk)
        .expect("32 is a valid HKDF-SHA256 output length");
    sk
}

/// Initiator (Alice): start a session against the responder's published bundle.
///
/// Verifies the bundle signature (rejecting an impostor's prekey), computes the
/// three DHs, derives the root key, and returns a wired initiator DhRatchet plus
/// the InitialMessage to send to the responder.
///
/// `initiator_identity` is Alice's full KeyPair (Ed25519); its X25519 identity
/// key is derived internally. `peer_bundle` is Bob's published PrekeyBundle.
pub fn x3dh_initiate(
    initiator_identity: &KeyPair,
    peer_bundle: &PrekeyBundle,
    peer_identity_ed25519: &crate::identity::keypair::IdentityKey,
) -> Result<(DhRatchet, InitialMessage), X3dhError> {
    // Reject an impostor: the prekey must be signed by the claimed identity.
    if !peer_bundle.verify(peer_identity_ed25519) {
        return Err(X3dhError::BadPrekeySignature);
    }

    let ik_a = initiator_identity.x25519_identity(); // Alice's identity DH key
    let ek_a = DhKeyPair::generate();                // Alice's ephemeral

    let spk_b = &peer_bundle.prekey_public;          // Bob's signed prekey (pub)
    let ik_b = &peer_bundle.identity_x25519;         // Bob's identity (pub)

    let dh1 = ik_a.diffie_hellman(spk_b);            // IK_a x SPK_b
    let dh2 = ek_a.diffie_hellman(ik_b);             // EK_a x IK_b
    let dh3 = ek_a.diffie_hellman(spk_b);            // EK_a x SPK_b
    let sk = derive_shared_secret(&dh1, &dh2, &dh3);

    // Bob's signed prekey is the initial peer ratchet key.
    let ratchet = DhRatchet::new_initiator(sk, *spk_b);

    let msg = InitialMessage {
        initiator_identity: ik_a.public_bytes(),
        initiator_ephemeral: ek_a.public_bytes(),
    };
    Ok((ratchet, msg))
}

/// Responder (Bob): derive the same session from the initiator's InitialMessage.
///
/// `responder_prekey` is Bob's signed-prekey DhKeyPair (the private half he kept
/// from SignedPrekey). `responder_identity` is Bob's full KeyPair.
pub fn x3dh_respond(
    responder_identity: &KeyPair,
    responder_prekey: DhKeyPair,
    msg: &InitialMessage,
) -> DhRatchet {
    let ik_b = responder_identity.x25519_identity(); // Bob's identity DH key
    let spk_b = &responder_prekey;                   // Bob's signed prekey (priv)

    let ik_a = &msg.initiator_identity;              // Alice's identity (pub)
    let ek_a = &msg.initiator_ephemeral;             // Alice's ephemeral (pub)

    // Mirror of the initiator's DHs (own-private x peer-public):
    let dh1 = spk_b.diffie_hellman(ik_a);            // SPK_b x IK_a  == DH1
    let dh2 = ik_b.diffie_hellman(ek_a);             // IK_b  x EK_a  == DH2
    let dh3 = spk_b.diffie_hellman(ek_a);            // SPK_b x EK_a  == DH3
    let sk = derive_shared_secret(&dh1, &dh2, &dh3);

    // Bob holds the signed-prekey keypair as his initial ratchet key.
    DhRatchet::new_responder(sk, responder_prekey)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::x3dh::prekey::SignedPrekey;

    // Build Bob's published bundle + keep his prekey keypair for responding.
    fn bob_setup(bob: &KeyPair) -> (PrekeyBundle, DhKeyPair) {
        let sp = SignedPrekey::new(bob);
        let bundle = sp.bundle(bob.x25519_identity_public());
        // Reconstruct Bob's prekey keypair for the responder side.
        let prekey_kp = DhKeyPair::from_private_bytes(sp.keypair().private_bytes());
        (bundle, prekey_kp)
    }

    #[test]
    fn both_sides_derive_identical_root_key() {
        // The keystone property: Alice's SK == Bob's SK. Tested by running both
        // sides and confirming a message Alice sends decrypts on Bob's ratchet
        // (which only works if the root keys matched).
        let alice = KeyPair::generate();
        let bob = KeyPair::generate();
        let (bundle, bob_prekey) = bob_setup(&bob);

        let (mut alice_ratchet, msg) =
            x3dh_initiate(&alice, &bundle, &bob.public).expect("initiate");
        let mut bob_ratchet = x3dh_respond(&bob, bob_prekey, &msg);

        let (ak, hdr) = alice_ratchet.next_sending_key().expect("alice sends");
        let bk = bob_ratchet.next_receiving_key(&hdr).unwrap();
        assert_eq!(ak.as_bytes(), bk.as_bytes());
    }

    #[test]
    fn impostor_prekey_is_rejected() {
        // Bob's bundle, but verified against the WRONG identity -> rejected.
        let alice = KeyPair::generate();
        let bob = KeyPair::generate();
        let impostor = KeyPair::generate();
        let (bundle, _bob_prekey) = bob_setup(&bob);

        let result = x3dh_initiate(&alice, &bundle, &impostor.public);
        assert!(matches!(result, Err(X3dhError::BadPrekeySignature)));
    }

    #[test]
    fn full_conversation_after_x3dh() {
        // End-to-end: establish via X3DH, then exchange several turns.
        let alice = KeyPair::generate();
        let bob = KeyPair::generate();
        let (bundle, bob_prekey) = bob_setup(&bob);

        let (mut a, msg) = x3dh_initiate(&alice, &bundle, &bob.public).expect("init");
        let mut b = x3dh_respond(&bob, bob_prekey, &msg);

        // Alice -> Bob
        let (ak, ah) = a.next_sending_key().unwrap();
        assert_eq!(ak.as_bytes(), b.next_receiving_key(&ah).unwrap().as_bytes());
        // Bob -> Alice (ratchet heals)
        let (bk, bh) = b.next_sending_key().unwrap();
        assert_eq!(bk.as_bytes(), a.next_receiving_key(&bh).unwrap().as_bytes());
        // Alice -> Bob again
        let (ak2, ah2) = a.next_sending_key().unwrap();
        assert_eq!(ak2.as_bytes(), b.next_receiving_key(&ah2).unwrap().as_bytes());
    }

    #[test]
    fn different_sessions_differ() {
        // Two independent sessions must not produce the same first key.
        let alice = KeyPair::generate();
        let bob = KeyPair::generate();
        let (bundle, bob_prekey) = bob_setup(&bob);
        let (mut a1, m1) = x3dh_initiate(&alice, &bundle, &bob.public).unwrap();
        let mut b1 = x3dh_respond(&bob, bob_prekey, &m1);
        let (k1, h1) = a1.next_sending_key().unwrap();
        let _ = b1.next_receiving_key(&h1).unwrap();

        let (bundle2, bob_prekey2) = bob_setup(&bob);
        let (mut a2, m2) = x3dh_initiate(&alice, &bundle2, &bob.public).unwrap();
        let mut b2 = x3dh_respond(&bob, bob_prekey2, &m2);
        let (k2, h2) = a2.next_sending_key().unwrap();
        let _ = b2.next_receiving_key(&h2).unwrap();

        assert_ne!(k1.as_bytes(), k2.as_bytes());
    }
}
