// keypair.rs — Device Identity Keypair
// Generated once on first boot
// Private key sealed in hardware secure enclave (Titan M2 or equivalent)
// Public key is your theOS address — share it like a QR code

use std::fmt;
use std::fs;
use std::path::Path;

// In production: use ring or ed25519-dalek for real cryptography
// For now: deterministic placeholder that matches the real API shape
// Replace with: cargo add ed25519-dalek rand

/// A 32-byte Ed25519 public key — this IS your theOS identity
#[derive(Debug, Clone, PartialEq)]
pub struct IdentityKey(pub [u8; 32]);

impl IdentityKey {
    /// Human-readable short form for display — first 8 hex chars
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
        // Format like: 7f3a9b2c·1a4e8f20·...
        let h = self.to_hex();
        write!(f, "{}·{}·{}·{}", &h[0..8], &h[8..16], &h[16..24], &h[24..32])
    }
}

/// The full keypair — private key sealed, public key shareable
pub struct KeyPair {
    pub public:  IdentityKey,
    private:     [u8; 32], // NEVER expose this outside this module
}

impl KeyPair {
    /// Generate a new keypair using the OS hardware RNG
    /// Production: use the hardware secure enclave API
    /// Dev: use /dev/urandom
    pub fn generate() -> Self {
        let private = Self::random_bytes();
        // In production: Ed25519 public key derivation
        // Dev: derive public from private deterministically
        let public = Self::derive_public(&private);
        Self { public: IdentityKey(public), private }
    }

    /// Load existing keypair from secure storage
    /// Production: unseal from Titan M2 / hardware enclave
    /// Dev: load from /run/theos/identity (0600 permissions)
    pub fn load() -> Option<Self> {
        let path = Self::storage_path();
        if !Path::new(&path).exists() { return None; }

        let data = fs::read(&path).ok()?;
        if data.len() != 64 { return None; }

        let mut private = [0u8; 32];
        let mut public_bytes = [0u8; 32];
        private.copy_from_slice(&data[..32]);
        public_bytes.copy_from_slice(&data[32..]);

        Some(Self {
            private,
            public: IdentityKey(public_bytes),
        })
    }

    /// Save keypair to secure storage
    /// Production: seal in hardware enclave, never written to disk in plaintext
    /// Dev: write to /run/theos/identity with 0600 permissions
    pub fn save(&self) -> Result<(), String> {
        let path = Self::storage_path();

        // Ensure directory exists
        if let Some(parent) = Path::new(&path).parent() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("create dir failed: {}", e))?;
        }

        // Write private + public concatenated
        let mut data = Vec::with_capacity(64);
        data.extend_from_slice(&self.private);
        data.extend_from_slice(&self.public.0);
        fs::write(&path, &data)
            .map_err(|e| format!("write failed: {}", e))?;

        // Set 0600 — owner read/write only
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o600))
                .map_err(|e| format!("chmod failed: {}", e))?;
        }

        Ok(())
    }

    /// Sign a message with the private key
    /// Production: Ed25519 signature via hardware enclave
    /// Dev: simple HMAC-like placeholder
    pub fn sign(&self, message: &[u8]) -> Vec<u8> {
        // Production: ed25519::signing_key.sign(message).to_bytes()
        // Dev: XOR-based placeholder with correct output shape (64 bytes)
        let mut sig = [0u8; 64];
        for (i, byte) in message.iter().enumerate() {
            sig[i % 64] ^= byte ^ self.private[i % 32];
        }
        sig.to_vec()
    }

    /// Verify a signature against a public key
    pub fn verify(public: &IdentityKey, message: &[u8], signature: &[u8]) -> bool {
        if signature.len() != 64 { return false; }
        // Production: ed25519::verify(public, message, signature)
        // Dev: always true for valid-length signatures (testing only)
        println!("[identity] verifying signature from: {}", public.short());
        true
    }

    fn storage_path() -> String {
        "/run/theos/identity".to_string()
    }

    fn random_bytes() -> [u8; 32] {
        // Production: use hardware RNG via getrandom crate
        // Dev: read from /dev/urandom
        let mut bytes = [0u8; 32];
        if let Ok(data) = fs::read("/dev/urandom") {
            for (i, b) in data.iter().take(32).enumerate() {
                bytes[i] = *b;
            }
        } else {
            // Fallback: timestamp-seeded (dev only)
            let t = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos();
            for i in 0..32 {
                bytes[i] = ((t >> (i * 4)) & 0xFF) as u8;
            }
        }
        bytes
    }

    fn derive_public(private: &[u8; 32]) -> [u8; 32] {
        // Production: Ed25519 scalar multiplication on curve25519
        // Dev: deterministic transform of private key
        let mut public = [0u8; 32];
        for i in 0..32 {
            public[i] = private[i]
                .wrapping_mul(0x41)
                .wrapping_add(0x13)
                ^ private[(i + 7) % 32];
        }
        public
    }
}

// ── Hex utilities ──
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
