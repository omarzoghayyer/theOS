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
    /// Fixed session id for at-rest storage. Storage is one logical session
    /// per device; per-message uniqueness comes from the counter (nonce).
    const STORE_SESSION_ID: u64 = 0x7468_656f_735f_6d73; // "theos_ms"

    /// Derive the storage key from the owner key via HKDF-SHA256 with domain
    /// separation. Replaces the previous placeholder byte-mangle.
    pub fn new(owner_key: &[u8; 32]) -> Self {
        use hkdf::Hkdf;
        use sha2::Sha256;
        let hk = Hkdf::<Sha256>::new(None, owner_key);
        let mut key = [0u8; 32];
        hk.expand(b"theos-message-store-v1", &mut key)
            .expect("32 is a valid HKDF-SHA256 output length");
        Self { key }
    }

    /// Encrypt message content with real ChaCha20-Poly1305 (via crypto::encrypt).
    /// `counter` must be unique per message under this key (nonce uniqueness).
    pub fn encrypt(&self, plaintext: &[u8], counter: u64) -> Vec<u8> {
        // crypto::encrypt only errors on AEAD internal failure, which does not
        // occur for in-memory plaintext; a panic here would be a library bug.
        crate::crypto::encrypt(&self.key, counter, Self::STORE_SESSION_ID, plaintext)
            .expect("ChaCha20-Poly1305 encryption of in-memory plaintext cannot fail")
    }

    /// Decrypt and AUTHENTICATE. Returns None if the Poly1305 tag fails —
    /// i.e. tampered ciphertext, wrong key, or wrong counter.
    pub fn decrypt(&self, ciphertext: &[u8], counter: u64) -> Option<Vec<u8>> {
        crate::crypto::decrypt(&self.key, counter, Self::STORE_SESSION_ID, ciphertext).ok()
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
        let filtered: Vec<&StoredMessage> = self.messages.iter()
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

// -- PersistentMessageStore ---------------------------------------------------
//
// SQLite-backed message store for production use on device.
// Uses rusqlite with bundled SQLite (no system SQLite needed).
//
// Schema:
//   conversations(contact_key_hex TEXT PK, last_message_at INT,
//                 unread_count INT, retention_days INT, created_at INT)
//   messages(id INT PK, conversation_id TEXT, from_me INT,
//            content_enc BLOB, timestamp INT, delivery INT, nonce_counter INT)
//
// All content_enc blobs are ChaCha20-Poly1305 encrypted before insert.
// Decrypted on read. SQLite file is plaintext metadata only.

#[cfg(feature = "persistence")]
pub mod persistent {
    use super::*;
    use rusqlite::{Connection, params};

    pub struct PersistentMessageStore {
        conn:         Connection,
        encryptor:    MessageEncryptor,
        send_counter: u64,
    }

    impl PersistentMessageStore {
        /// Open or create the message database at the given path.
        /// Use ":memory:" for tests.
        pub fn open(path: &str, owner_key: &[u8; 32]) -> Result<Self, String> {
            let conn = Connection::open(path)
                .map_err(|e| format!("db open failed: {}", e))?;

            let store = Self {
                conn,
                encryptor:    MessageEncryptor::new(owner_key),
                send_counter: 0,
            };

            store.init_schema()?;
            Ok(store)
        }

        fn init_schema(&self) -> Result<(), String> {
            self.conn.execute_batch("
                CREATE TABLE IF NOT EXISTS conversations (
                    contact_key_hex TEXT PRIMARY KEY,
                    last_message_at INTEGER NOT NULL,
                    unread_count    INTEGER NOT NULL DEFAULT 0,
                    retention_days  INTEGER NOT NULL DEFAULT 90,
                    created_at      INTEGER NOT NULL
                );
                CREATE TABLE IF NOT EXISTS messages (
                    id              INTEGER PRIMARY KEY AUTOINCREMENT,
                    conversation_id TEXT    NOT NULL,
                    from_me         INTEGER NOT NULL,
                    content_enc     BLOB    NOT NULL,
                    timestamp       INTEGER NOT NULL,
                    delivery        INTEGER NOT NULL DEFAULT 0,
                    nonce_counter   INTEGER NOT NULL,
                    FOREIGN KEY (conversation_id)
                        REFERENCES conversations(contact_key_hex)
                );
                CREATE INDEX IF NOT EXISTS idx_messages_conv
                    ON messages(conversation_id, timestamp);
            ").map_err(|e| format!("schema init failed: {}", e))
        }

        /// Store an encrypted message. Returns assigned message ID.
        pub fn store_message(
            &mut self,
            conversation_id: &str,
            from_me:         bool,
            plaintext:       &[u8],
        ) -> Result<u64, String> {
            let counter   = self.send_counter;
            self.send_counter += 1;
            let encrypted = self.encryptor.encrypt(plaintext, counter);
            let now       = now_secs();
            let from_me_i = if from_me { 1i64 } else { 0i64 };

            // Upsert conversation
            self.conn.execute(
                "INSERT INTO conversations(contact_key_hex, last_message_at, unread_count, created_at)
                 VALUES(?1, ?2, ?3, ?4)
                 ON CONFLICT(contact_key_hex) DO UPDATE SET
                     last_message_at = ?2,
                     unread_count = unread_count + ?3",
                params![
                    conversation_id,
                    now as i64,
                    if from_me { 0i64 } else { 1i64 },
                    now as i64,
                ],
            ).map_err(|e| format!("upsert conv failed: {}", e))?;

            // Insert message
            self.conn.execute(
                "INSERT INTO messages(conversation_id, from_me, content_enc, timestamp, delivery, nonce_counter)
                 VALUES(?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    conversation_id,
                    from_me_i,
                    encrypted,
                    now as i64,
                    0i64, // Sending
                    counter as i64,
                ],
            ).map_err(|e| format!("insert message failed: {}", e))?;

            Ok(self.conn.last_insert_rowid() as u64)
        }

        /// Retrieve and decrypt messages for a conversation (newest last).
        pub fn messages_for(
            &self,
            conversation_id: &str,
            limit:           usize,
        ) -> Result<Vec<(u64, bool, Vec<u8>, u64, StoredDelivery)>, String> {
            let mut stmt = self.conn.prepare(
                "SELECT id, from_me, content_enc, timestamp, delivery, nonce_counter
                 FROM messages
                 WHERE conversation_id = ?1
                 ORDER BY timestamp ASC, id ASC
                 LIMIT ?2"
            ).map_err(|e| format!("prepare failed: {}", e))?;

            let rows = stmt.query_map(
                params![conversation_id, limit as i64],
                |row| Ok((
                    row.get::<_, i64>(0)? as u64,
                    row.get::<_, i64>(1)? != 0,
                    row.get::<_, Vec<u8>>(2)?,
                    row.get::<_, i64>(3)? as u64,
                    StoredDelivery::from_u8(row.get::<_, i64>(4)? as u8),
                    row.get::<_, i64>(5)? as u64,
                )),
            ).map_err(|e| format!("query failed: {}", e))?;

            let mut result = Vec::new();
            for row in rows {
                let (id, from_me, enc, ts, delivery, counter) =
                    row.map_err(|e| format!("row error: {}", e))?;
                if let Some(pt) = self.encryptor.decrypt(&enc, counter) {
                    result.push((id, from_me, pt, ts, delivery));
                }
            }
            Ok(result)
        }

        /// Update delivery state for a message.
        pub fn update_delivery(&self, message_id: u64, delivery: StoredDelivery) -> Result<bool, String> {
            let rows = self.conn.execute(
                "UPDATE messages SET delivery = ?1 WHERE id = ?2",
                params![delivery.as_u8() as i64, message_id as i64],
            ).map_err(|e| format!("update failed: {}", e))?;
            Ok(rows > 0)
        }

        /// Mark all delivered messages in a conversation as read.
        pub fn mark_read(&self, conversation_id: &str) -> Result<u32, String> {
            let rows = self.conn.execute(
                "UPDATE messages SET delivery = ?1
                 WHERE conversation_id = ?2 AND from_me = 0 AND delivery = ?3",
                params![
                    StoredDelivery::Read.as_u8() as i64,
                    conversation_id,
                    StoredDelivery::Delivered.as_u8() as i64,
                ],
            ).map_err(|e| format!("mark_read failed: {}", e))?;

            self.conn.execute(
                "UPDATE conversations SET unread_count = 0 WHERE contact_key_hex = ?1",
                params![conversation_id],
            ).map_err(|e| format!("reset unread failed: {}", e))?;

            Ok(rows as u32)
        }

        /// Delete messages older than retention period.
        pub fn gc(&self) -> Result<usize, String> {
            let cutoff = now_secs().saturating_sub(DEFAULT_RETENTION_DAYS * 86400) as i64;
            let rows = self.conn.execute(
                "DELETE FROM messages WHERE timestamp < ?1",
                params![cutoff],
            ).map_err(|e| format!("gc failed: {}", e))?;
            Ok(rows)
        }

        /// Delete all messages for a conversation.
        pub fn delete_conversation(&self, conversation_id: &str) -> Result<usize, String> {
            let rows = self.conn.execute(
                "DELETE FROM messages WHERE conversation_id = ?1",
                params![conversation_id],
            ).map_err(|e| format!("delete failed: {}", e))?;
            self.conn.execute(
                "DELETE FROM conversations WHERE contact_key_hex = ?1",
                params![conversation_id],
            ).map_err(|e| format!("delete conv failed: {}", e))?;
            Ok(rows)
        }

        pub fn message_count(&self) -> usize {
            self.conn.query_row(
                "SELECT COUNT(*) FROM messages", [], |r| r.get::<_, i64>(0)
            ).unwrap_or(0) as usize
        }

        pub fn conversation_count(&self) -> usize {
            self.conn.query_row(
                "SELECT COUNT(*) FROM conversations", [], |r| r.get::<_, i64>(0)
            ).unwrap_or(0) as usize
        }

        pub fn total_unread(&self) -> u32 {
            self.conn.query_row(
                "SELECT COALESCE(SUM(unread_count), 0) FROM conversations",
                [], |r| r.get::<_, i64>(0)
            ).unwrap_or(0) as u32
        }
    }

    // -- Persistence tests ----------------------------------------------------

    #[cfg(test)]
    mod tests {
        use super::*;

        fn owner() -> [u8; 32] { [0xAAu8; 32] }
        fn mem_store() -> PersistentMessageStore {
            PersistentMessageStore::open(":memory:", &owner()).unwrap()
        }
        const CONV: &str = "aabbccdd";
        const CONV2: &str = "eeff0011";

        #[test]
        fn test_persistent_store_opens() {
            assert!(PersistentMessageStore::open(":memory:", &owner()).is_ok());
        }

        #[test]
        fn test_persistent_store_message() {
            let mut s = mem_store();
            let id = s.store_message(CONV, true, b"Hello Sarah").unwrap();
            assert!(id > 0);
            assert_eq!(s.message_count(), 1);
        }

        #[test]
        fn test_persistent_retrieve_decrypt() {
            let mut s = mem_store();
            s.store_message(CONV, true, b"Hey!").unwrap();
            let msgs = s.messages_for(CONV, 10).unwrap();
            assert_eq!(msgs.len(), 1);
            assert_eq!(msgs[0].2, b"Hey!");
            assert!(msgs[0].1); // from_me
        }

        #[test]
        fn test_persistent_multiple_messages() {
            let mut s = mem_store();
            s.store_message(CONV, true,  b"first").unwrap();
            s.store_message(CONV, false, b"second").unwrap();
            s.store_message(CONV, true,  b"third").unwrap();
            let msgs = s.messages_for(CONV, 10).unwrap();
            assert_eq!(msgs.len(), 3);
            assert_eq!(msgs[0].2, b"first");
            assert_eq!(msgs[2].2, b"third");
        }

        #[test]
        fn test_persistent_separate_conversations() {
            let mut s = mem_store();
            s.store_message(CONV,  true, b"to sarah").unwrap();
            s.store_message(CONV2, true, b"to marcus").unwrap();
            assert_eq!(s.messages_for(CONV,  10).unwrap().len(), 1);
            assert_eq!(s.messages_for(CONV2, 10).unwrap().len(), 1);
            assert_eq!(s.conversation_count(), 2);
        }

        #[test]
        fn test_persistent_update_delivery() {
            let mut s = mem_store();
            let id = s.store_message(CONV, true, b"hello").unwrap();
            assert!(s.update_delivery(id, StoredDelivery::Sent).unwrap());
            let msgs = s.messages_for(CONV, 10).unwrap();
            assert_eq!(msgs[0].4, StoredDelivery::Sent);
        }

        #[test]
        fn test_persistent_mark_read() {
            let mut s = mem_store();
            let id = s.store_message(CONV, false, b"incoming").unwrap();
            s.update_delivery(id, StoredDelivery::Delivered).unwrap();
            s.mark_read(CONV).unwrap();
            let msgs = s.messages_for(CONV, 10).unwrap();
            assert_eq!(msgs[0].4, StoredDelivery::Read);
        }

        #[test]
        fn test_persistent_delete_conversation() {
            let mut s = mem_store();
            s.store_message(CONV,  true, b"msg1").unwrap();
            s.store_message(CONV,  true, b"msg2").unwrap();
            s.store_message(CONV2, true, b"other").unwrap();
            let deleted = s.delete_conversation(CONV).unwrap();
            assert_eq!(deleted, 2);
            assert_eq!(s.message_count(), 1);
        }

        #[test]
        fn test_persistent_gc() {
            let s = mem_store();
            // GC on empty store should succeed
            let deleted = s.gc().unwrap();
            assert_eq!(deleted, 0);
        }

        #[test]
        fn test_persistent_total_unread() {
            let mut s = mem_store();
            s.store_message(CONV,  false, b"1").unwrap();
            s.store_message(CONV2, false, b"2").unwrap();
            assert_eq!(s.total_unread(), 2);
        }

        #[test]
        fn test_persistent_limit() {
            let mut s = mem_store();
            for i in 0..10u8 {
                s.store_message(CONV, true, &[i]).unwrap();
            }
            let msgs = s.messages_for(CONV, 3).unwrap();
            assert_eq!(msgs.len(), 3);
        }

        #[test]
        fn test_persistent_encryption_roundtrip() {
            let mut s = mem_store();
            let secret = b"super secret message content";
            s.store_message(CONV, true, secret).unwrap();
            let msgs = s.messages_for(CONV, 1).unwrap();
            assert_eq!(msgs[0].2, secret);
        }

        #[test]
        fn test_persistent_ids_autoincrement() {
            let mut s = mem_store();
            let id1 = s.store_message(CONV, true, b"a").unwrap();
            let id2 = s.store_message(CONV, true, b"b").unwrap();
            assert!(id1 < id2);
        }
    }
}
