// message_store_tests.rs — Encryption roundtrip tests
//
// Validates ChaCha20-Poly1305 integrity without depending on full MessageStore

#[cfg(test)]
mod encryption_tests {
    use crate::crypto::{ChaChaKey, chacha_encrypt, chacha_decrypt};

    #[test]
    fn test_round_trip_encryption() {
        let plaintext = b"Hello, encrypted world!";
        let key = ChaChaKey::from_seed(&[1u8; 32]);
        let nonce = [0u8; 12];

        let ciphertext = chacha_encrypt(plaintext, &key, &nonce).expect("encrypt failed");
        let decrypted = chacha_decrypt(&ciphertext, &key, &nonce).expect("decrypt failed");

        assert_eq!(plaintext, decrypted.as_slice());
    }

    #[test]
    fn test_different_keys_different_ciphertexts() {
        let plaintext = b"Same message";
        let key1 = ChaChaKey::from_seed(&[1u8; 32]);
        let key2 = ChaChaKey::from_seed(&[2u8; 32]);
        let nonce = [0u8; 12];

        let ct1 = chacha_encrypt(plaintext, &key1, &nonce).expect("ct1");
        let ct2 = chacha_encrypt(plaintext, &key2, &nonce).expect("ct2");

        assert_ne!(ct1, ct2);
    }

    #[test]
    fn test_empty_plaintext() {
        let plaintext = b"";
        let key = ChaChaKey::from_seed(&[5u8; 32]);
        let nonce = [0u8; 12];

        let ciphertext = chacha_encrypt(plaintext, &key, &nonce).expect("encrypt empty");
        let decrypted = chacha_decrypt(&ciphertext, &key, &nonce).expect("decrypt empty");

        assert_eq!(plaintext, decrypted.as_slice());
    }

    #[test]
    fn test_tampering_detection() {
        let plaintext = b"Original message";
        let key = ChaChaKey::from_seed(&[3u8; 32]);
        let nonce = [0u8; 12];

        let mut ciphertext = chacha_encrypt(plaintext, &key, &nonce).expect("encrypt");

        // Tamper with ciphertext
        if !ciphertext.is_empty() {
            ciphertext[0] ^= 0xFF;
        }

        // Should fail due to Poly1305 tag verification
        let result = chacha_decrypt(&ciphertext, &key, &nonce);
        assert!(result.is_err(), "Should reject tampered ciphertext");
    }

    #[test]
    fn test_key_derivation_consistency() {
        let seed = &[42u8; 32];
        let key1 = ChaChaKey::from_seed(seed);
        let key2 = ChaChaKey::from_seed(seed);

        assert_eq!(key1.as_bytes(), key2.as_bytes());
    }
}
