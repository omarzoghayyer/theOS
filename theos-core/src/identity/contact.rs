// contact.rs — Contact Book
// Stores trusted public keys with user-assigned names
// This is your entire address book — no phone numbers anywhere
//
// File I/O lives in the daemon (services/src/identity_manager.rs); this
// module provides serialize/deserialize primitives only. Phase 2 of the
// theos-core std → no_std bridge.

use super::keypair::IdentityKey;
use std::collections::HashMap;

/// A single trusted contact
#[derive(Debug, Clone)]
pub struct Contact {
    pub name:       String,
    pub key:        IdentityKey,
    pub added_at:   u64,        // unix timestamp
    pub last_seen:  Option<u64>,
    pub trusted:    bool,
}

impl Contact {
    pub fn new(name: &str, key: IdentityKey) -> Self {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        Self {
            name: name.to_string(),
            key,
            added_at: now,
            last_seen: None,
            trusted: true,
        }
    }
}

/// The full contact book — stored encrypted on device
pub struct ContactBook {
    pub contacts: HashMap<String, Contact>, // key hex → contact
}

impl ContactBook {
    pub fn new() -> Self {
        Self { contacts: HashMap::new() }
    }

    /// Serialize the contact book to bytes for persistence.
    ///
    /// The daemon is responsible for writing these bytes to disk and
    /// encrypting them at rest. This library does no file I/O.
    ///
    /// Format: one contact per line, `key_hex:added_at:name`.
    pub fn to_bytes(&self) -> Vec<u8> {
        let data: String = self.contacts.values()
            .map(|c| format!("{}:{}:{}", c.key.to_hex(), c.added_at, c.name))
            .collect::<Vec<_>>()
            .join("\n");
        data.into_bytes()
    }

    /// Deserialize a contact book from bytes.
    ///
    /// Mirrors the lenient parse behavior of the previous file-loading
    /// path: malformed lines are silently skipped rather than failing
    /// the whole load. Invalid UTF-8 returns an empty book.
    pub fn from_bytes(data: &[u8]) -> Self {
        let mut book = Self::new();
        let text = match core::str::from_utf8(data) {
            Ok(s) => s,
            Err(_) => return book,
        };
        for line in text.lines() {
            let parts: Vec<&str> = line.splitn(3, ':').collect();
            if parts.len() == 3 {
                if let Ok(key) = IdentityKey::from_hex(parts[0]) {
                    let contact = Contact {
                        name:      parts[2].to_string(),
                        key:       key.clone(),
                        added_at:  parts[1].parse().unwrap_or(0),
                        last_seen: None,
                        trusted:   true,
                    };
                    book.contacts.insert(parts[0].to_string(), contact);
                }
            }
        }
        book
    }

    /// Add a new contact — this is the ONLY way someone can reach you
    pub fn add(&mut self, contact: Contact) -> Result<(), String> {
        let key_hex = contact.key.to_hex();
        if self.contacts.contains_key(&key_hex) {
            return Err(format!("contact {} already exists", contact.name));
        }
        println!("[contacts] added: {} ({})", contact.name, contact.key.short());
        self.contacts.insert(key_hex, contact);
        Ok(())
    }

    /// Remove a contact — they can no longer reach you
    pub fn remove(&mut self, key: &IdentityKey) -> Result<(), String> {
        let key_hex = key.to_hex();
        if let Some(c) = self.contacts.remove(&key_hex) {
            println!("[contacts] removed: {}", c.name);
        }
        Ok(())
    }

    /// Check if a public key is in your contacts
    /// This is called on EVERY incoming call/message
    /// If false — the connection is dropped silently
    pub fn is_trusted(&self, key: &IdentityKey) -> bool {
        self.contacts.contains_key(&key.to_hex())
    }

    /// Look up a contact by their public key
    pub fn find(&self, key: &IdentityKey) -> Option<&Contact> {
        self.contacts.get(&key.to_hex())
    }

    /// All contacts — for display in the UI
    pub fn all(&self) -> Vec<&Contact> {
        let mut contacts: Vec<&Contact> = self.contacts.values().collect();
        contacts.sort_by(|a, b| a.name.cmp(&b.name));
        contacts
    }

    pub fn count(&self) -> usize {
        self.contacts.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::keypair::KeyPair;

    fn fixture(name: &str) -> Contact {
        let kp = KeyPair::generate();
        Contact::new(name, kp.public.clone())
    }

    #[test]
    fn to_bytes_from_bytes_round_trip_preserves_contacts() {
        let mut book = ContactBook::new();
        book.add(fixture("alice")).unwrap();
        book.add(fixture("bob")).unwrap();
        book.add(fixture("carol")).unwrap();

        let bytes = book.to_bytes();
        let restored = ContactBook::from_bytes(&bytes);

        assert_eq!(restored.count(), 3);
        for (k, v) in &book.contacts {
            let r = restored.contacts.get(k).expect("contact missing after round trip");
            assert_eq!(r.name,     v.name);
            assert_eq!(r.added_at, v.added_at);
            assert_eq!(r.key.to_hex(), v.key.to_hex());
        }
    }

    #[test]
    fn from_bytes_empty_returns_empty_book() {
        let book = ContactBook::from_bytes(&[]);
        assert_eq!(book.count(), 0);
    }

    #[test]
    fn from_bytes_skips_malformed_lines() {
        let mut book = ContactBook::new();
        book.add(fixture("alice")).unwrap();
        let mut bytes = book.to_bytes();
        // Append garbage that should be silently skipped
        bytes.extend_from_slice(b"\nnot:a:valid:contact:line:with:too:many:parts");
        bytes.extend_from_slice(b"\nno_colons_at_all");
        bytes.extend_from_slice(b"\nzz_bad_hex:0:partial");

        let restored = ContactBook::from_bytes(&bytes);
        assert_eq!(restored.count(), 1, "only the valid line should survive");
    }
}
