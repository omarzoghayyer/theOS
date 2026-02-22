// exchange.rs — Contact Exchange Protocol
// How two theOS users share their public keys
// Options: QR code, NFC tap, or a short invite code
// This is the ONLY way to add someone — there is no public directory

use super::keypair::IdentityKey;
use super::contact::{Contact, ContactBook};

/// A contact exchange payload — what gets encoded in the QR code
#[derive(Debug, Clone)]
pub struct ExchangePayload {
    pub public_key: IdentityKey,
    pub display_name: String,
    pub version: u8,
}

impl ExchangePayload {
    pub fn new(key: IdentityKey, name: &str) -> Self {
        Self { public_key: key, display_name: name.to_string(), version: 1 }
    }

    /// Encode to URI for QR code
    /// Format: theos://contact/v1/<hex_key>/<name>
    pub fn to_qr_uri(&self) -> String {
        format!(
            "theos://contact/v1/{}/{}",
            self.public_key.to_hex(),
            urlencoded(&self.display_name)
        )
    }

    /// Encode to a short 12-character invite code
    /// Easier to share verbally or over text
    /// Format: first 12 chars of public key hex
    pub fn to_invite_code(&self) -> String {
        self.public_key.to_hex()[..12].to_uppercase()
    }

    /// Parse from QR URI
    pub fn from_qr_uri(uri: &str) -> Result<Self, String> {
        // theos://contact/v1/<hex>/<name>
        let prefix = "theos://contact/v1/";
        if !uri.starts_with(prefix) {
            return Err("not a theOS contact URI".to_string());
        }
        let rest = &uri[prefix.len()..];
        let parts: Vec<&str> = rest.splitn(2, '/').collect();
        if parts.len() != 2 {
            return Err("malformed URI".to_string());
        }
        let key = IdentityKey::from_hex(parts[0])?;
        let name = urldecoded(parts[1]);
        Ok(Self { public_key: key, display_name: name, version: 1 })
    }
}

pub struct ContactExchange;

impl ContactExchange {
    /// Generate the QR code URI for sharing YOUR identity
    pub fn my_qr_uri(my_key: &IdentityKey, my_name: &str) -> String {
        ExchangePayload::new(my_key.clone(), my_name).to_qr_uri()
    }

    /// Process a scanned QR code — add to contacts
    pub fn accept_qr(
        uri: &str,
        name_override: Option<&str>,
        contacts: &mut ContactBook,
    ) -> Result<Contact, String> {
        let payload = ExchangePayload::from_qr_uri(uri)?;
        let name = name_override.unwrap_or(&payload.display_name).to_string();
        let contact = Contact::new(&name, payload.public_key);
        contacts.add(contact.clone())?;
        println!("[exchange] contact added via QR: {}", contact.name);
        Ok(contact)
    }

    /// Generate a short invite code for verbal sharing
    pub fn invite_code(key: &IdentityKey) -> String {
        key.to_hex()[..12].to_uppercase()
    }

    /// Validate an incoming call — is this key in contacts?
    pub fn validate_incoming(
        caller_key: &IdentityKey,
        contacts: &ContactBook,
    ) -> Result<&'static str, &'static str> {
        if contacts.is_trusted(caller_key) {
            println!("[exchange] incoming call accepted from: {}", caller_key.short());
            Ok("trusted")
        } else {
            println!("[exchange] incoming call REJECTED — unknown key: {}", caller_key.short());
            Err("unknown — call dropped silently")
        }
    }
}

fn urlencoded(s: &str) -> String {
    s.chars().map(|c| match c {
        'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' => c.to_string(),
        ' ' => "%20".to_string(),
        _ => format!("%{:02X}", c as u32),
    }).collect()
}

fn urldecoded(s: &str) -> String {
    let mut result = String::new();
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '%' {
            let h1 = chars.next().unwrap_or('0');
            let h2 = chars.next().unwrap_or('0');
            if let Ok(b) = u8::from_str_radix(&format!("{}{}", h1, h2), 16) {
                result.push(b as char);
            }
        } else if c == '+' {
            result.push(' ');
        } else {
            result.push(c);
        }
    }
    result
}
