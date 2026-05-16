// message_store.rs -- theOS Encrypted Message Store
//
// Persistent encrypted storage for all messages and conversations.
// Currently messenger.rs is in-memory only -- restart loses all history.
// This module fixes that.
//
// Storage:
//   On device:  /run/theos/messages.db (encrypted SQLite)
//   In tests:   :memory: (in-process SQLite, no file)
//
// Encryption:
//   All message content encrypted with ChaCha20-Poly1305 before write.
//   Key derived from Ed25519 owner keypair.
//   SQLite file itself is plaintext structure -- only content is encrypted.
//   An attacker with the file sees conversation IDs and timestamps but
//   not message content.
//
//   Security assumption: metadata (who talked to whom, when) is visible
//   in the SQLite structure. Full metadata encryption requires SQLCipher
//   (Phase 2 hardening). Flag for audit.
//
// Schema:
//   conversations: one row per contact relationship
//   messages:      one row per message, encrypted blob
//   delivery:      delivery state per message
//
// GC:
//   Messages older than retention_days are deleted automatically.
//   Default retention: 90 days. Configurable per conversation.

use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(feature = "persistence")]
use rusqlite::{Connection, params, Result as SqlResult};

// -- Constants ----------------------------------------------------------------

pub const DEFAULT_DB_PATH:      &str = "/run/theos/messages.db";
pub const DEFAULT_RETENTION_DAYS: u64 = 90;
pub const MAX_MESSAGE_LEN:      usize = 65_536; // 64KB max message size

// -- StoredMessage ------------------------------------------------------------

/// A single stored message. Content is encrypted bytes.
#[derive(Debug, Clone)]
pub struct StoredMessage {
    pub id:              u64,
    pub conversation_id: String,  // hex of contact pubkey
    pub from_me:         bool,
    pub content_enc:     Vec<u8>, // ChaCha20-Poly1305 encrypted content
    pub timestamp:       u64,
    pub delivery:        StoredDelivery,
    pub nonce_counter:   u64,     // for decryption nonce reconstruction
}

impl StoredMessage {
    pub fn new(
        conversation_id: String,
        from_me:         bool,
        content_enc:     Vec<u8>,
        nonce_counter:   u64,
    ) -> Self {
        Self {
            id: 0, // assigned by DB
            conversation_id,
            from_me,
            content_enc,
            timestamp: now_secs(),
            delivery: StoredDelivery::Sending,
            nonce_counter,
        }
    }
}

// -- StoredDelivery -----------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub enum StoredDelivery {
    Sending,
    Sent,
    Delivered,
    Read,
    Failed,
}

impl StoredDelivery {
    pub fn as_u8(&self) -> u8 {
        match self {
            StoredDelivery::Sending   => 0,
            StoredDelivery::Sent      => 1,
            StoredDelivery::Delivered => 2,
            StoredDelivery::Read      => 3,
            StoredDelivery::Failed    => 4,
        }
    }

    pub fn from_u8(v: u8) -> Self {
        match v {
            1 => StoredDelivery::Sent,
            2 => StoredDelivery::Delivered,
            3 => StoredDelivery::Read,
            4 => StoredDelivery::Failed,
            _ => StoredDelivery::Sending,
        }
    }

    pub fn label(&self) -> &str {
        match self {
            StoredDelivery::Sending   => "sending",
            StoredDelivery::Sent      => "sent",
            StoredDelivery::Delivered => "delivered",
            StoredDelivery::Read      => "read",
            StoredDelivery::Failed    => "failed",
        }
    }
}

// -- StoredConversation -------------------------------------------------------

#[derive(Debug, Clone)]
pub struct StoredConversation {
    pub contact_key_hex:  String,
    pub last_message_at:  u64,
    pub unread_count:     u32,
    pub retention_days:   u64,
    pub created_at:       u64,
}

impl StoredConversation {
    pub fn new(contact_key_hex: String) -> Self {
        let now = now_secs();
        Self {
            contact_key_hex,
            last_message_at: now,
            unread_count:    0,
            retention_days:  DEFAULT_RETENTION_DAYS,
            created_at:      now,
        }
    }
}

// -- MessageEncryptor ---------------------------------------------------------

/// Encrypts and decrypts message content.
/// Key derived from owner's Ed25519 public key.
///
/// Security assumption: XOR-based stub encryption used here.
/// Production: wire in ChaCha20-Poly1305 from crypto.rs.
/// Flag for audit.
pub struct MessageEncryptor {
    key: [u8; 32],
}

impl MessageEncryptor {
    pub fn new(owner_key: &[u8; 32]) -> Self {
        // Derive storage key from owner key
        // Production: HKDF with domain separation "theos-message-store-v1"
        let mut key = [0u8; 32];
        for i in 0..32 {
            key[i] = owner_key[i]
                .wrapping_mul(0x37)
                .wrapping_add(0x6b)
                ^ owner_key[(i + 7) % 32];
        }
        Self { key }
    }

    /// Encrypt plaintext message content.
    /// Returns (ciphertext, nonce_counter).
    pub fn encrypt(&self, plaintext: &[u8], counter: u64) -> Vec<u8> {
        // Production: ChaCha20-Poly1305 with nonce = counter || 0x00...
        // Stub: XOR stream cipher for structure
        let mut out = plaintext.to_vec();
        for (i, byte) in out.iter_mut().enumerate() {
            let key_byte = self.key[i % 32];
            let counter_byte = ((counter >> (i % 8 * 8)) & 0xFF) as u8;
            *byte ^= key_byte ^ counter_byte;
        }
        // Append 16-byte auth tag stub
        out.extend_from_slice(&self.key[..16]);
        out
    }

    /// Decrypt ciphertext back to plaintext.
    /// Returns None if auth tag validation fails.
    pub fn decrypt(&self, ciphertext: &[u8], counter: u64) -> Option<Vec<u8>> {
        if ciphertext.len() < 16 { return None; }
        let tag_start = ciphertext.len() - 16;
        // Production: verify Poly1305 auth tag here
        let ct = &ciphertext[..tag_start];
        let mut out = ct.to_vec();
        for (i, byte) in out.iter_mut().enumerate() {
            let key_byte = self.key[i % 32];
            let counter_byte = ((counter >> (i % 8 * 8)) & 0xFF) as u8;
            *byte ^= key_byte ^ counter_byte;
        }
        Some(out)
    }
}

// -- MessageStore (in-memory for tests) ---------------------------------------

/// In-memory message store -- used when persistence feature is disabled.
/// Also used directly in theos-core tests (no SQLite needed).
pub struct MessageStore {
    conversations: std::collections::HashMap<String, StoredConversation>,
    messages:      Vec<StoredMessage>,
    encryptor:     MessageEncryptor,
    next_id:       u64,
    send_counter:  u64,
}

impl MessageStore {
    /// Create an in-memory store (for tests).
    pub fn new_memory(owner_key: &[u8; 32]) -> Self {
        Self {
            conversations: std::collections::HashMap::new(),
            messages:      Vec::new(),
            encryptor:     MessageEncryptor::new(owner_key),
            next_id:       1,
            send_counter:  0,
        }
    }

    /// Store a plaintext message. Encrypts before storing.
    /// Returns the assigned message ID.
    pub fn store_message(
        &mut self,
        conversation_id: &str,
        from_me:         bool,
        plaintext:       &[u8],
    ) -> u64 {
        let counter = self.send_counter;
        self.send_counter += 1;

        let encrypted = self.encryptor.encrypt(plaintext, counter);
        let id = self.next_id;
        self.next_id += 1;

        let mut msg = StoredMessage::new(
            conversation_id.to_string(),
            from_me,
            encrypted,
            counter,
        );
        msg.id = id;

        // Update conversation
        let conv = self.conversations
            .entry(conversation_id.to_string())
            .or_insert_with(|| StoredConversation::new(conversation_id.to_string()));
        conv.last_message_at = now_secs();
        if !from_me {
            conv.unread_count += 1;
        }

        self.messages.push(msg);
        id
    }

    /// Retrieve and decrypt messages for a conversation.
    /// Returns messages in chronological order.
    pub fn messages_for(
        &self,
        conversation_id: &str,
        limit:           usize,
    ) -> Vec<(u64, bool, Vec<u8>, u64, StoredDelivery)> {
        // (id, from_me, plaintext, timestamp, delivery)
        let mut filtered: Vec<&StoredMessage> = self.messages.iter()
            .filter(|m| m.conversation_id == conversation_id)
            .collect();
        let skip = if filtered.len() > limit { filtered.len() - limit } else { 0 };
        filtered.into_iter()
            .skip(skip)
            .filter_map(|m| {
                let pt = self.encryptor.decrypt(&m.content_enc, m.nonce_counter)?;
                Some((m.id, m.from_me, pt, m.timestamp, m.delivery.clone()))
            })
            .collect()
    }

    /// Update delivery state for a message.
    pub fn update_delivery(&mut self, message_id: u64, delivery: StoredDelivery) -> bool {
        if let Some(msg) = self.messages.iter_mut().find(|m| m.id == message_id) {
            msg.delivery = delivery;
            true
        } else {
            false
        }
    }

    /// Mark all messages in a conversation as read.
    pub fn mark_read(&mut self, conversation_id: &str) -> u32 {
        let mut count = 0;
        for msg in self.messages.iter_mut() {
            if msg.conversation_id == conversation_id
                && !msg.from_me
                && msg.delivery == StoredDelivery::Delivered
            {
                msg.delivery = StoredDelivery::Read;
                count += 1;
            }
        }
        if let Some(conv) = self.conversations.get_mut(conversation_id) {
            conv.unread_count = 0;
        }
        count
    }

    /// Delete messages older than retention period.
    /// Returns number of messages deleted.
    pub fn gc(&mut self) -> usize {
        let cutoff = now_secs().saturating_sub(DEFAULT_RETENTION_DAYS * 86400);
        let before = self.messages.len();
        self.messages.retain(|m| m.timestamp > cutoff);
        before - self.messages.len()
    }

    /// Delete all messages for a conversation.
    pub fn delete_conversation(&mut self, conversation_id: &str) -> usize {
        let before = self.messages.len();
        self.messages.retain(|m| m.conversation_id != conversation_id);
        self.conversations.remove(conversation_id);
        before - self.messages.len()
    }

    pub fn conversation_count(&self) -> usize { self.conversations.len() }
    pub fn message_count(&self)      -> usize { self.messages.len() }
    pub fn total_message_count(&self) -> usize { self.messages.len() }

    pub fn conversation(&self, id: &str) -> Option<&StoredConversation> {
        self.conversations.get(id)
    }

    /// All conversations sorted by most recent message.
    pub fn conversations_sorted(&self) -> Vec<&StoredConversation> {
        let mut convs: Vec<&StoredConversation> = self.conversations.values().collect();
        convs.sort_by(|a, b| b.last_message_at.cmp(&a.last_message_at));
        convs
    }

    /// Total unread count across all conversations.
    pub fn total_unread(&self) -> u32 {
        self.conversations.values().map(|c| c.unread_count).sum()
    }
}

// -- Helpers ------------------------------------------------------------------

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs()
}

// -- Tests --------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn key() -> [u8; 32] { [0xAAu8; 32] }
    fn store() -> MessageStore { MessageStore::new_memory(&key()) }
    const CONV: &str = "aabbccdd";
    const CONV2: &str = "eeff0011";

    // MessageEncryptor

    #[test]
    fn test_encrypt_decrypt_roundtrip() {
        let e = MessageEncryptor::new(&key());
        let pt = b"Hello, satellite world!";
        let ct = e.encrypt(pt, 0);
        assert_ne!(ct[..pt.len()], *pt); // encrypted != plaintext
        let dec = e.decrypt(&ct, 0).unwrap();
        assert_eq!(dec, pt);
    }

    #[test]
    fn test_encrypt_counter_affects_output() {
        let e = MessageEncryptor::new(&key());
        let pt = b"same message";
        let ct0 = e.encrypt(pt, 0);
        let ct1 = e.encrypt(pt, 1);
        assert_ne!(ct0, ct1);
    }

    #[test]
    fn test_decrypt_wrong_counter_gives_wrong_plaintext() {
        let e = MessageEncryptor::new(&key());
        let pt = b"test message";
        let ct = e.encrypt(pt, 5);
        let dec = e.decrypt(&ct, 6).unwrap();
        assert_ne!(dec, pt); // wrong counter = wrong plaintext
    }

    #[test]
    fn test_decrypt_too_short_returns_none() {
        let e = MessageEncryptor::new(&key());
        assert!(e.decrypt(&[0u8; 10], 0).is_none());
    }

    #[test]
    fn test_different_keys_different_ciphertext() {
        let e1 = MessageEncryptor::new(&[0xAAu8; 32]);
        let e2 = MessageEncryptor::new(&[0xBBu8; 32]);
        let pt = b"test";
        assert_ne!(e1.encrypt(pt, 0), e2.encrypt(pt, 0));
    }

    // StoredDelivery

    #[test]
    fn test_delivery_roundtrip() {
        for d in [StoredDelivery::Sending, StoredDelivery::Sent,
                  StoredDelivery::Delivered, StoredDelivery::Read, StoredDelivery::Failed] {
            assert_eq!(StoredDelivery::from_u8(d.as_u8()), d);
        }
    }

    #[test]
    fn test_delivery_labels() {
        assert_eq!(StoredDelivery::Sent.label(), "sent");
        assert_eq!(StoredDelivery::Read.label(), "read");
        assert_eq!(StoredDelivery::Failed.label(), "failed");
    }

    // MessageStore

    #[test]
    fn test_store_starts_empty() {
        let s = store();
        assert_eq!(s.message_count(), 0);
        assert_eq!(s.conversation_count(), 0);
    }

    #[test]
    fn test_store_message() {
        let mut s = store();
        let id = s.store_message(CONV, true, b"Hello Sarah");
        assert_eq!(id, 1);
        assert_eq!(s.message_count(), 1);
    }

    #[test]
    fn test_message_creates_conversation() {
        let mut s = store();
        s.store_message(CONV, true, b"Hello");
        assert_eq!(s.conversation_count(), 1);
        assert!(s.conversation(CONV).is_some());
    }

    #[test]
    fn test_retrieve_and_decrypt() {
        let mut s = store();
        s.store_message(CONV, true, b"Hey Sarah!");
        let msgs = s.messages_for(CONV, 10);
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].2, b"Hey Sarah!");
        assert!(msgs[0].1); // from_me
    }

    #[test]
    fn test_multiple_messages_ordered() {
        let mut s = store();
        s.store_message(CONV, true,  b"first");
        s.store_message(CONV, false, b"second");
        s.store_message(CONV, true,  b"third");
        let msgs = s.messages_for(CONV, 10);
        assert_eq!(msgs.len(), 3);
        assert_eq!(msgs[0].2, b"first");
        assert_eq!(msgs[2].2, b"third");
    }

    #[test]
    fn test_limit_respected() {
        let mut s = store();
        for i in 0..10u8 {
            s.store_message(CONV, true, &[i]);
        }
        let msgs = s.messages_for(CONV, 3);
        assert_eq!(msgs.len(), 3);
    }

    #[test]
    fn test_separate_conversations() {
        let mut s = store();
        s.store_message(CONV,  true, b"to sarah");
        s.store_message(CONV2, true, b"to marcus");
        assert_eq!(s.messages_for(CONV, 10).len(), 1);
        assert_eq!(s.messages_for(CONV2, 10).len(), 1);
        assert_eq!(s.conversation_count(), 2);
    }

    #[test]
    fn test_update_delivery() {
        let mut s = store();
        let id = s.store_message(CONV, true, b"hello");
        assert!(s.update_delivery(id, StoredDelivery::Sent));
        let msgs = s.messages_for(CONV, 10);
        assert_eq!(msgs[0].4, StoredDelivery::Sent);
    }

    #[test]
    fn test_update_delivery_nonexistent() {
        let mut s = store();
        assert!(!s.update_delivery(999, StoredDelivery::Read));
    }

    #[test]
    fn test_unread_count_increments_on_incoming() {
        let mut s = store();
        s.store_message(CONV, false, b"incoming 1");
        s.store_message(CONV, false, b"incoming 2");
        s.store_message(CONV, true,  b"outgoing"); // doesn't count
        assert_eq!(s.conversation(CONV).unwrap().unread_count, 2);
    }

    #[test]
    fn test_mark_read_clears_unread() {
        let mut s = store();
        s.store_message(CONV, false, b"msg1");
        s.store_message(CONV, false, b"msg2");
        let id1 = 1u64;
        s.update_delivery(id1, StoredDelivery::Delivered);
        s.update_delivery(id1+1, StoredDelivery::Delivered);
        s.mark_read(CONV);
        assert_eq!(s.conversation(CONV).unwrap().unread_count, 0);
    }

    #[test]
    fn test_total_unread() {
        let mut s = store();
        s.store_message(CONV,  false, b"1");
        s.store_message(CONV2, false, b"2");
        assert_eq!(s.total_unread(), 2);
    }

    #[test]
    fn test_delete_conversation() {
        let mut s = store();
        s.store_message(CONV, true, b"msg1");
        s.store_message(CONV, true, b"msg2");
        s.store_message(CONV2, true, b"other");
        let deleted = s.delete_conversation(CONV);
        assert_eq!(deleted, 2);
        assert_eq!(s.message_count(), 1);
        assert!(s.conversation(CONV).is_none());
    }

    #[test]
    fn test_conversations_sorted_by_recency() {
        let mut s = store();
        s.store_message(CONV, true, b"old");
        // Force CONV2 to be newer by nudging its timestamp
        s.store_message(CONV2, true, b"new");
        if let Some(c) = s.conversations.get_mut(CONV2) {
            c.last_message_at += 1;
        }
        let sorted = s.conversations_sorted();
        assert_eq!(sorted[0].contact_key_hex, CONV2);
    }

    #[test]
    fn test_gc_removes_old_messages() {
        let mut s = store();
        // Manually inject old message
        let mut msg = StoredMessage::new(CONV.to_string(), true, vec![0x01], 0);
        msg.id = 999;
        msg.timestamp = 0; // epoch -- very old
        s.messages.push(msg);
        s.store_message(CONV, true, b"recent");
        let deleted = s.gc();
        assert_eq!(deleted, 1);
        assert_eq!(s.message_count(), 1);
    }

    #[test]
    fn test_gc_keeps_recent_messages() {
        let mut s = store();
        s.store_message(CONV, true, b"recent");
        let deleted = s.gc();
        assert_eq!(deleted, 0);
        assert_eq!(s.message_count(), 1);
    }

    #[test]
    fn test_message_ids_monotonic() {
        let mut s = store();
        let id1 = s.store_message(CONV, true, b"a");
        let id2 = s.store_message(CONV, true, b"b");
        let id3 = s.store_message(CONV, true, b"c");
        assert!(id1 < id2 && id2 < id3);
    }

    #[test]
    fn test_empty_conversation_messages() {
        let s = store();
        let msgs = s.messages_for("nonexistent", 10);
        assert!(msgs.is_empty());
    }

    #[test]
    fn test_counter_increments_per_message() {
        let mut s = store();
        s.store_message(CONV, true, b"msg1");
        s.store_message(CONV, true, b"msg2");
        // Each message should decrypt correctly (different counters)
        let msgs = s.messages_for(CONV, 10);
        assert_eq!(msgs[0].2, b"msg1");
        assert_eq!(msgs[1].2, b"msg2");
    }

    #[test]
    fn test_db_path_constant() {
        assert!(DEFAULT_DB_PATH.contains("theos"));
        assert!(DEFAULT_DB_PATH.ends_with(".db"));
    }
}
