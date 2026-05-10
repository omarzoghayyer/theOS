// contact.rs — Contact Book
// Stores trusted public keys with user-assigned names
// This is your entire address book — no phone numbers anywhere

use super::keypair::IdentityKey;
use std::collections::HashMap;
use std::fs;

const CONTACTS_PATH: &str = "/run/theos/contacts.json";

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
    contacts: HashMap<String, Contact>, // key hex → contact
}

impl ContactBook {
    pub fn new() -> Self {
        Self { contacts: HashMap::new() }
    }

    /// Load contact book from storage
    pub fn load() -> Self {
        let mut book = Self::new();
        if let Ok(data) = fs::read_to_string(CONTACTS_PATH) {
            // Production: decrypt with hardware key first
            // Dev: parse simple line format "hex:name:timestamp"
            for line in data.lines() {
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
        }
        println!("[contacts] loaded {} contacts", book.contacts.len());
        book
    }

    /// Save contact book to storage
    pub fn save(&self) -> Result<(), String> {
        if let Some(parent) = std::path::Path::new(CONTACTS_PATH).parent() {
            fs::create_dir_all(parent).ok();
        }
        // Production: encrypt with hardware key before writing
        let data: String = self.contacts.values()
            .map(|c| format!("{}:{}:{}", c.key.to_hex(), c.added_at, c.name))
            .collect::<Vec<_>>()
            .join("\n");
        fs::write(CONTACTS_PATH, data)
            .map_err(|e| format!("save failed: {}", e))?;
        Ok(())
    }

    /// Add a new contact — this is the ONLY way someone can reach you
    pub fn add(&mut self, contact: Contact) -> Result<(), String> {
        let key_hex = contact.key.to_hex();
        if self.contacts.contains_key(&key_hex) {
            return Err(format!("contact {} already exists", contact.name));
        }
        println!("[contacts] added: {} ({})", contact.name, contact.key.short());
        self.contacts.insert(key_hex, contact);
        self.save()
    }

    /// Remove a contact — they can no longer reach you
    pub fn remove(&mut self, key: &IdentityKey) -> Result<(), String> {
        let key_hex = key.to_hex();
        if let Some(c) = self.contacts.remove(&key_hex) {
            println!("[contacts] removed: {}", c.name);
            self.save()?;
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
