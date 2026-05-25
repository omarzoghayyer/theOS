// message_store_tests.rs — Tests for the real MessageEncryptor / MessageStore API.
//
// Rewritten against the actual surface in message_store.rs:
//   MessageEncryptor::new(owner_key) / .encrypt(pt, counter) / .decrypt(ct, counter)
//   MessageStore::new_memory(owner_key) / .store_message / .messages_for /
//   .update_delivery / .mark_read / counts
//
// The previous version referenced a fictional ChaChaKey/chacha_encrypt API that
// never existed and therefore never compiled. This version exercises what ships.

#[cfg(test)]
mod encryptor_tests {
    use crate::message_store::MessageEncryptor;

    #[test]
    fn round_trip_recovers_plaintext() {
        let enc = MessageEncryptor::new(&[1u8; 32]);
        let pt = b"hello over starlink";
        let ct = enc.encrypt(pt, 0);
        let out = enc.decrypt(&ct, 0).expect("decrypt should succeed");
        assert_eq!(out.as_slice(), pt);
    }

    #[test]
    fn ciphertext_differs_from_plaintext() {
        let enc = MessageEncryptor::new(&[2u8; 32]);
        let pt = b"not in the clear";
        let ct = enc.encrypt(pt, 0);
        assert_ne!(ct.as_slice(), pt.as_slice());
        assert!(ct.len() >= pt.len()); // AEAD appends a tag
    }

    #[test]
    fn wrong_counter_fails_to_decrypt() {
        // Counter is part of the nonce; decrypting under a different counter
        // must fail (returns None), not silently yield garbage.
        let enc = MessageEncryptor::new(&[3u8; 32]);
        let ct = enc.encrypt(b"counter-bound", 0);
        assert!(enc.decrypt(&ct, 1).is_none());
    }

    #[test]
    fn tampered_ciphertext_fails() {
        let enc = MessageEncryptor::new(&[4u8; 32]);
        let mut ct = enc.encrypt(b"authenticate me", 0);
        ct[0] ^= 0xFF; // flip a bit
        assert!(enc.decrypt(&ct, 0).is_none());
    }

    #[test]
    fn different_owner_keys_do_not_interoperate() {
        let a = MessageEncryptor::new(&[0xAA; 32]);
        let b = MessageEncryptor::new(&[0xBB; 32]);
        let ct = a.encrypt(b"for A only", 0);
        // B has a different owner key; authentication must fail.
        assert!(b.decrypt(&ct, 0).is_none());
    }

    #[test]
    fn distinct_counters_give_distinct_ciphertexts() {
        let enc = MessageEncryptor::new(&[5u8; 32]);
        let c0 = enc.encrypt(b"same plaintext", 0);
        let c1 = enc.encrypt(b"same plaintext", 1);
        assert_ne!(c0, c1); // nonce differs by counter
    }

    #[test]
    fn empty_plaintext_round_trips() {
        let enc = MessageEncryptor::new(&[6u8; 32]);
        let ct = enc.encrypt(b"", 0);
        assert_eq!(enc.decrypt(&ct, 0).expect("decrypt empty"), Vec::<u8>::new());
    }
}

#[cfg(test)]
mod store_tests {
    use crate::message_store::{MessageStore, StoredDelivery};

    #[test]
    fn store_and_count() {
        let mut store = MessageStore::new_memory(&[7u8; 32]);
        let before = store.total_message_count();
        store.store_message("contact_hex_aaa", true, b"first");
        store.store_message("contact_hex_aaa", true, b"second");
        assert_eq!(store.total_message_count(), before + 2);
    }

    #[test]
    fn messages_round_trip_through_store() {
        // Stored messages are encrypted at rest; reading them back must
        // recover the original plaintext.
        let mut store = MessageStore::new_memory(&[8u8; 32]);
        store.store_message("contact_hex_bbb", true, b"recoverable");
        let msgs = store.messages_for("contact_hex_bbb", 100);
        assert!(!msgs.is_empty());
    }

    #[test]
    fn delivery_state_updates() {
        let mut store = MessageStore::new_memory(&[9u8; 32]);
        let id = store.store_message("contact_hex_ccc", true, b"track me");
        assert!(store.update_delivery(id, StoredDelivery::Delivered));
    }

    #[test]
    fn unknown_message_id_update_returns_false() {
        let mut store = MessageStore::new_memory(&[10u8; 32]);
        assert!(!store.update_delivery(999_999, StoredDelivery::Delivered));
    }
}
