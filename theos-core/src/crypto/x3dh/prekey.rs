// crypto/x3dh/prekey.rs — Signed prekeys for X3DH session establishment.
//
// A signed prekey is a medium-term X25519 keypair that an identity generates
// and signs with its Ed25519 identity key. Publishing it lets a peer start an
// encrypted session asynchronously. The Ed25519 signature BINDS the prekey to
// the identity, rejecting an impostor who substitutes a prekey of their own.

use crate::crypto::ratchet::dh::{DhKeyPair, DhPublic};
use crate::identity::keypair::{IdentityKey, KeyPair};

const PREKEY_SIG_DOMAIN: &[u8] = b"theos-signed-prekey-v1";

fn prekey_signing_payload(prekey_public: &DhPublic) -> Vec<u8> {
    let mut payload = Vec::with_capacity(PREKEY_SIG_DOMAIN.len() + 32);
    payload.extend_from_slice(PREKEY_SIG_DOMAIN);
    payload.extend_from_slice(prekey_public);
    payload
}

/// The owner's PRIVATE signed prekey: X25519 keypair + Ed25519 signature.
pub struct SignedPrekey {
    keypair: DhKeyPair,
    signature: Vec<u8>,
}

impl SignedPrekey {
    pub fn new(identity: &KeyPair) -> Self {
        let keypair = DhKeyPair::generate();
        let payload = prekey_signing_payload(&keypair.public_bytes());
        let signature = identity.sign(&payload);
        Self { keypair, signature }
    }

    pub fn keypair(&self) -> &DhKeyPair {
        &self.keypair
    }

    pub fn bundle(&self, identity_x25519: DhPublic) -> PrekeyBundle {
        PrekeyBundle {
            identity_x25519,
            prekey_public: self.keypair.public_bytes(),
            signature: self.signature.clone(),
        }
    }
}

/// The PUBLIC half a peer needs to start a session.
#[derive(Debug, Clone, PartialEq)]
pub struct PrekeyBundle {
    pub identity_x25519: DhPublic,
    pub prekey_public: DhPublic,
    pub signature: Vec<u8>,
}

impl PrekeyBundle {
    /// Verify the prekey signature against the owner's Ed25519 identity key.
    /// False if tampered or checked against the wrong identity (impostor).
    pub fn verify(&self, identity_ed25519: &IdentityKey) -> bool {
        let payload = prekey_signing_payload(&self.prekey_public);
        KeyPair::verify(identity_ed25519, &payload, &self.signature)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_bundle_verifies() {
        let identity = KeyPair::generate();
        let sp = SignedPrekey::new(&identity);
        let bundle = sp.bundle(identity.x25519_identity_public());
        assert!(bundle.verify(&identity.public));
    }

    #[test]
    fn tampered_prekey_fails() {
        let identity = KeyPair::generate();
        let sp = SignedPrekey::new(&identity);
        let mut bundle = sp.bundle(identity.x25519_identity_public());
        bundle.prekey_public[0] ^= 0xFF;
        assert!(!bundle.verify(&identity.public));
    }

    #[test]
    fn wrong_identity_fails() {
        let owner = KeyPair::generate();
        let impostor = KeyPair::generate();
        let sp = SignedPrekey::new(&owner);
        let bundle = sp.bundle(owner.x25519_identity_public());
        assert!(!bundle.verify(&impostor.public));
    }

    #[test]
    fn tampered_signature_fails() {
        let identity = KeyPair::generate();
        let sp = SignedPrekey::new(&identity);
        let mut bundle = sp.bundle(identity.x25519_identity_public());
        let last = bundle.signature.len() - 1;
        bundle.signature[last] ^= 0x01;
        assert!(!bundle.verify(&identity.public));
    }

    #[test]
    fn bundle_carries_correct_prekey() {
        let identity = KeyPair::generate();
        let sp = SignedPrekey::new(&identity);
        let bundle = sp.bundle(identity.x25519_identity_public());
        assert_eq!(bundle.prekey_public, sp.keypair().public_bytes());
    }
}
