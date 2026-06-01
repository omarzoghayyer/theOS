// crypto/ratchet/dh.rs — X25519 Diffie-Hellman + HKDF key-derivation primitives
//
// Foundation layer for the Double Ratchet. Provides:
//   - DhKeyPair: storable X25519 keypair (ephemeral + identity-derived)
//   - diffie_hellman(): raw DH shared-secret computation
//   - kdf_rk(): root-key ratchet KDF   (advances the DH ratchet)
//   - kdf_ck(): chain-key ratchet KDF  (advances the symmetric ratchet)
//
// NO protocol/state-machine logic lives here. Total functions only, so it can
// be validated with known-answer tests before any ratchet state exists.
//
// Security notes (flag for audit):
//   - X25519 identity key is HKDF-derived from the Ed25519 seed under a distinct
//     domain tag. Separate output, never the Ed25519 scalar reused.
//   - HKDF-SHA256 throughout; domain-separated info strings per use.
//   - DhKeyPair holds a StaticSecret because the ratchet retains a private key
//     across multiple DH ops, unlike the consume-on-use EphemeralSecret.

use hkdf::Hkdf;
use sha2::Sha256;
use x25519_dalek::{PublicKey, StaticSecret};

/// A 32-byte X25519 public key in raw byte form (wire-friendly).
pub type DhPublic = [u8; 32];

/// An X25519 keypair used by the ratchet.
pub struct DhKeyPair {
    secret: StaticSecret,
    public: PublicKey,
}

impl DhKeyPair {
    /// Generate a fresh random X25519 keypair (ephemeral ratchet keys).
    pub fn generate() -> Self {
        let secret = StaticSecret::random_from_rng(&mut rand::rngs::OsRng);
        let public = PublicKey::from(&secret);
        Self { secret, public }
    }

    /// Deterministically derive the X25519 IDENTITY keypair from the Ed25519 seed.
    /// Same seed -> same key, via HKDF under a distinct domain tag. Separate
    /// output from the Ed25519 signing key; the two never share a secret value.
    pub fn from_identity_seed(seed: &[u8; 32]) -> Self {
        let hk = Hkdf::<Sha256>::new(None, seed);
        let mut okm = [0u8; 32];
        hk.expand(b"theos-x25519-identity-v1", &mut okm)
            .expect("32 is a valid HKDF-SHA256 output length");
        let secret = StaticSecret::from(okm); // applies X25519 clamping
        let public = PublicKey::from(&secret);
        Self { secret, public }
    }

    /// Reconstruct from stored raw private-key bytes.
    pub fn from_private_bytes(bytes: [u8; 32]) -> Self {
        let secret = StaticSecret::from(bytes);
        let public = PublicKey::from(&secret);
        Self { secret, public }
    }

    /// Public key as raw bytes (wire / HandleAnnouncement).
    pub fn public_bytes(&self) -> DhPublic {
        *self.public.as_bytes()
    }

    /// Raw private-key bytes, for sealed storage only.
    pub fn private_bytes(&self) -> [u8; 32] {
        self.secret.to_bytes()
    }

    /// X25519 shared secret between our private key and their public.
    pub fn diffie_hellman(&self, their_public: &DhPublic) -> [u8; 32] {
        let their_pk = PublicKey::from(*their_public);
        *self.secret.diffie_hellman(&their_pk).as_bytes()
    }
}

impl Clone for DhKeyPair {
    fn clone(&self) -> Self {
        // Rebuild from raw private bytes — avoids depending on StaticSecret: Clone
        // across x25519-dalek versions. Same key in, identical keypair out.
        Self::from_private_bytes(self.private_bytes())
    }
}

/// Root-key KDF ("KDF_RK"). Current root key + DH output -> (new_root, chain_key).
/// Used each DH-ratchet step (per round-trip).
pub fn kdf_rk(root_key: &[u8; 32], dh_out: &[u8; 32]) -> ([u8; 32], [u8; 32]) {
    let hk = Hkdf::<Sha256>::new(Some(root_key), dh_out);
    let mut okm = [0u8; 64];
    hk.expand(b"theos-ratchet-rk-v1", &mut okm)
        .expect("64 is a valid HKDF-SHA256 output length");
    let mut new_root = [0u8; 32];
    let mut chain_key = [0u8; 32];
    new_root.copy_from_slice(&okm[..32]);
    chain_key.copy_from_slice(&okm[32..]);
    (new_root, chain_key)
}

/// Chain-key KDF ("KDF_CK"). Advances chain key -> (next_chain_key, message_key).
/// Used per message (symmetric ratchet).
pub fn kdf_ck(chain_key: &[u8; 32]) -> ([u8; 32], [u8; 32]) {
    let hk = Hkdf::<Sha256>::new(Some(chain_key), b"");
    let mut okm = [0u8; 64];
    hk.expand(b"theos-ratchet-ck-v1", &mut okm)
        .expect("64 is a valid HKDF-SHA256 output length");
    let mut next_chain = [0u8; 32];
    let mut message_key = [0u8; 32];
    next_chain.copy_from_slice(&okm[..32]);
    message_key.copy_from_slice(&okm[32..]);
    (next_chain, message_key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dh_is_symmetric() {
        let alice = DhKeyPair::generate();
        let bob = DhKeyPair::generate();
        let ab = alice.diffie_hellman(&bob.public_bytes());
        let ba = bob.diffie_hellman(&alice.public_bytes());
        assert_eq!(ab, ba);
    }

    #[test]
    fn identity_derivation_is_deterministic() {
        let seed = [7u8; 32];
        let k1 = DhKeyPair::from_identity_seed(&seed);
        let k2 = DhKeyPair::from_identity_seed(&seed);
        assert_eq!(k1.public_bytes(), k2.public_bytes());
        assert_eq!(k1.private_bytes(), k2.private_bytes());
    }

    #[test]
    fn different_seeds_give_different_identity_keys() {
        let a = DhKeyPair::from_identity_seed(&[1u8; 32]);
        let b = DhKeyPair::from_identity_seed(&[2u8; 32]);
        assert_ne!(a.public_bytes(), b.public_bytes());
    }

    #[test]
    fn x25519_key_differs_from_ed25519_seed() {
        let seed = [42u8; 32];
        let k = DhKeyPair::from_identity_seed(&seed);
        assert_ne!(k.private_bytes(), seed);
    }

    #[test]
    fn from_private_bytes_roundtrips_public() {
        let kp = DhKeyPair::generate();
        let priv_bytes = kp.private_bytes();
        let restored = DhKeyPair::from_private_bytes(priv_bytes);
        assert_eq!(kp.public_bytes(), restored.public_bytes());
    }

    #[test]
    fn kdf_rk_is_deterministic_and_splits() {
        let rk = [3u8; 32];
        let dh = [9u8; 32];
        let (r1, c1) = kdf_rk(&rk, &dh);
        let (r2, c2) = kdf_rk(&rk, &dh);
        assert_eq!(r1, r2);
        assert_eq!(c1, c2);
        assert_ne!(r1, c1);
        assert_ne!(r1, rk);
    }

    #[test]
    fn kdf_rk_changes_with_dh_input() {
        let rk = [3u8; 32];
        let (ra, _) = kdf_rk(&rk, &[1u8; 32]);
        let (rb, _) = kdf_rk(&rk, &[2u8; 32]);
        assert_ne!(ra, rb);
    }

    #[test]
    fn kdf_ck_advances_and_differs() {
        let ck = [5u8; 32];
        let (next, msg) = kdf_ck(&ck);
        let (next2, msg2) = kdf_ck(&ck);
        assert_eq!(next, next2);
        assert_eq!(msg, msg2);
        assert_ne!(next, msg);
        assert_ne!(next, ck);
        assert_ne!(msg, ck);
    }

    #[test]
    fn kdf_ck_chain_diverges_over_steps() {
        let ck0 = [5u8; 32];
        let (ck1, m0) = kdf_ck(&ck0);
        let (_ck2, m1) = kdf_ck(&ck1);
        assert_ne!(m0, m1);
    }
}
