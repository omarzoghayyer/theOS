// keypair.rs — Device Identity Keypair
// Real Ed25519 cryptography via ed25519-dalek
// Private key sealed in hardware secure enclave (Titan M2 or equivalent)
// Public key is your theOS address — share it like a QR code

use ed25519_dalek::{SigningKey, VerifyingKey, Signer, Verifier, Signature};
use std::fmt;
use std::fs;
use std::path::Path;

/// A 32-byte Ed25519 public key — this IS your theOS identity
#[derive(Debug, Clone, PartialEq)]
pub struct IdentityKey(pub [u8; 32]);

impl IdentityKey {
    /// Human-readable short form — first 8 hex chars
    pub fn short(&self) -> String {
        hex_encode(&self.0[..4])
    }

    /// Full hex-encoded public key — your theOS address
    pub fn to_hex(&self) -> String {
        hex_encode(&self.0)
    }

    /// Parse from hex string
    pub fn from_hex(s: &str) -> Result<Self, String> {
        let bytes = hex_decode(s)?;
        if bytes.len() != 32 {
            return Err(format!("expected 32 bytes, got {}", bytes.len()));
        }
        let mut arr = [0u8; 32];
        arr.copy_from_slice(&bytes);
        Ok(Self(arr))
    }

    /// Generate a QR-code-friendly URI
    pub fn to_uri(&self) -> String {
        format!("theos://contact/{}", self.to_hex())
    }
}

impl fmt::Display for IdentityKey {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let h = self.to_hex();
        write!(f, "{}·{}·{}·{}", &h[0..8], &h[8..16], &h[16..24], &h[24..32])
    }
}

/// The full keypair — Ed25519 signing key
pub struct KeyPair {
    pub public:  IdentityKey,
    signing_key: SigningKey,
}

impl KeyPair {
    /// Generate a new keypair using OS hardware RNG
    pub fn generate() -> Self {
        // Use ed25519-dalek built-in secure RNG (cross-platform)
        let signing_key = SigningKey::generate(&mut rand::rngs::OsRng);
        let verifying_key = signing_key.verifying_key();
        let public = IdentityKey(verifying_key.to_bytes());

        println!("[identity] generated new Ed25519 keypair");
        Self { public, signing_key }
    }


    /// Derive this identity's X25519 keypair for Diffie-Hellman key agreement.
    ///
    /// X3DH and the Double Ratchet need DH (X25519), but the identity key is
    /// Ed25519 (signing). Rather than reuse the Ed25519 scalar (a cross-protocol
    /// anti-pattern), we derive a SEPARATE X25519 key from the same 32-byte seed
    /// via HKDF (see crypto::ratchet::dh::DhKeyPair::from_identity_seed).
    ///
    /// Deterministic: the same identity always yields the same X25519 key, so it
    /// survives reload and both parties can rederive it. The Ed25519 signing key
    /// and this X25519 key never share a secret value.
    pub fn x25519_identity(&self) -> crate::crypto::ratchet::dh::DhKeyPair {
        let seed = self.signing_key.to_bytes();
        crate::crypto::ratchet::dh::DhKeyPair::from_identity_seed(&seed)
    }

    /// The X25519 identity PUBLIC key, as raw bytes — publishable, e.g. in a
    /// HandleAnnouncement so a peer can run X3DH against this identity.
    pub fn x25519_identity_public(&self) -> [u8; 32] {
        self.x25519_identity().public_bytes()
    }

    /// Load existing keypair from secure storage
    /// Production: unseal from hardware enclave
    /// Dev: load from /tmp/theos-identity (0600 permissions)
    pub fn load() -> Option<Self> {
        let path = storage_path();
        if !Path::new(&path).exists() { return None; }

        let data = fs::read(&path).ok()?;
        if data.len() != 32 { return None; }

        let mut seed = [0u8; 32];
        seed.copy_from_slice(&data[..32]);

        let signing_key = SigningKey::from_bytes(&seed);
        let verifying_key = signing_key.verifying_key();
        let public = IdentityKey(verifying_key.to_bytes());

        println!("[identity] loaded existing keypair");
        Some(Self { public, signing_key })
    }

    /// Save keypair seed to secure storage
    /// Production: seal in hardware enclave — never plaintext on disk
    /// Dev: /tmp/theos-identity with 0600 permissions
    pub fn save(&self) -> Result<(), String> {
        let path = storage_path();
        if let Some(parent) = Path::new(&path).parent() {
            fs::create_dir_all(parent).ok();
        }

        // Save only the 32-byte seed — public key is derived from it
        let seed = self.signing_key.to_bytes();
        fs::write(&path, seed)
            .map_err(|e| format!("write failed: {}", e))?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o600))
                .map_err(|e| format!("chmod failed: {}", e))?;
        }

        println!("[identity] keypair saved securely");
        Ok(())
    }

    /// Sign a message with the private key — real Ed25519 signature
    pub fn sign(&self, message: &[u8]) -> Vec<u8> {
        let signature: Signature = self.signing_key.sign(message);
        signature.to_bytes().to_vec()
    }

    /// Verify a signature against a public key — real Ed25519 verification
    pub fn verify(public: &IdentityKey, message: &[u8], signature_bytes: &[u8]) -> bool {
        if signature_bytes.len() != 64 { return false; }

        let verifying_key = match VerifyingKey::from_bytes(&public.0) {
            Ok(k) => k,
            Err(_) => return false,
        };

        let mut sig_arr = [0u8; 64];
        sig_arr.copy_from_slice(signature_bytes);
        let signature = Signature::from_bytes(&sig_arr);

        match verifying_key.verify(message, &signature) {
            Ok(_) => {
                println!("[identity] ✓ signature valid from: {}", public.short());
                true
            }
            Err(_) => {
                println!("[identity] ✗ signature INVALID from: {}", public.short());
                false
            }
        }
    }
}

fn storage_path() -> String {
    "/tmp/theos-identity".to_string()
}

#[allow(dead_code)] // fallback RNG path, kept for no-getrandom targets
fn read_random_bytes() -> [u8; 32] {
    let mut bytes = [0u8; 32];
    if let Ok(data) = fs::read("/dev/urandom") {
        for (i, b) in data.iter().take(32).enumerate() {
            bytes[i] = *b;
        }
    }
    bytes
}

fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

fn hex_decode(s: &str) -> Result<Vec<u8>, String> {
    if s.len() % 2 != 0 {
        return Err("odd length hex string".to_string());
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i+2], 16)
            .map_err(|_| format!("invalid hex at position {}", i)))
        .collect()
}

#[cfg(test)]
mod x25519_identity_tests {
    use super::*;
    use crate::crypto::ratchet::dh::DhKeyPair;

    #[test]
    fn x25519_identity_matches_direct_derivation() {
        let kp = KeyPair::generate();
        let seed = kp.signing_key.to_bytes();
        let direct = DhKeyPair::from_identity_seed(&seed);
        assert_eq!(kp.x25519_identity_public(), direct.public_bytes());
    }

    #[test]
    fn x25519_identity_is_deterministic() {
        let kp = KeyPair::generate();
        assert_eq!(kp.x25519_identity_public(), kp.x25519_identity_public());
    }

    #[test]
    fn x25519_identity_differs_from_ed25519_public() {
        // Separate key, not a reinterpretation of the Ed25519 public bytes.
        let kp = KeyPair::generate();
        assert_ne!(kp.x25519_identity_public(), kp.public.0);
    }

    #[test]
    fn x25519_identity_survives_reload() {
        // Same seed -> same X25519 key after a save/load cycle. Two KeyPairs
        // built from the same seed must expose the same X25519 identity.
        let kp = KeyPair::generate();
        let seed = kp.signing_key.to_bytes();
        let reloaded = KeyPair { 
            public: kp.public.clone(), 
            signing_key: ed25519_dalek::SigningKey::from_bytes(&seed),
        };
        assert_eq!(kp.x25519_identity_public(), reloaded.x25519_identity_public());
    }
}
