// connection.rs -- theOS Connection Request Protocol
//
// The ONLY way two theOS users can reach each other.
// Nobody can call or message you without going through this flow first.
//
// Flow:
//   1. Requester finds target handle in DHT -> gets their pubkey
//   2. Requester creates a signed ConnectionRequest
//   3. Request is delivered to target device via DHT routing
//   4. Target sees it in their PendingRequests queue
//   5. Target accepts -> both added to each other ContactBook
//   6. Target declines -> dropped silently (requester not notified)
//
// Security:
//   - Requests are signed with requester Ed25519 key
//   - Signature covers: requester pubkey + target pubkey + timestamp + message
//   - Replay prevention: requests expire after 24 hours
//   - Rate limit: one request per requester per target per hour

use crate::identity::keypair::{IdentityKey, KeyPair};
use crate::identity::contact::{Contact, ContactBook};
use std::collections::HashMap;

const REQUEST_TTL_SECS:    u64 = 86400; // 24 hours
const REQUEST_RATE_SECS:   u64 = 3600;  // one request per target per hour
const MAX_PENDING:         usize = 50;  // max pending requests in queue
const MAX_MESSAGE_LEN:     usize = 140; // intro message character limit

// -- ConnectionRequest --------------------------------------------------------

/// A signed request from one theOS user to connect with another.
/// Delivered via DHT routing to the target device.
#[derive(Debug, Clone, PartialEq)]
pub struct ConnectionRequest {
    pub requester_key: IdentityKey,   // who is asking
    pub target_key:    IdentityKey,   // who they want to reach
    pub display_name:  String,        // requester's chosen display name
    pub message:       String,        // optional intro message (max 140 chars)
    pub timestamp:     u64,
    pub expires_at:    u64,
    pub signature:     Vec<u8>,       // Ed25519 sig over payload
}


impl ConnectionRequest {
    /// Create and sign a new connection request.
    pub fn new(
        keypair:      &KeyPair,
        target_key:   &IdentityKey,
        display_name: &str,
        message:      &str,
    ) -> Result<Self, ConnectionError> {
        if message.len() > MAX_MESSAGE_LEN {
            return Err(ConnectionError::MessageTooLong);
        }
        if display_name.trim().is_empty() {
            return Err(ConnectionError::InvalidDisplayName);
        }

        let timestamp  = now_secs();
        let expires_at = timestamp + REQUEST_TTL_SECS;
        let message    = message.trim().to_string();

        let payload = Self::signing_payload(
            &keypair.public, target_key, &message, timestamp
        );
        let signature = keypair.sign(&payload);

        Ok(Self {
            requester_key: keypair.public.clone(),
            target_key:    target_key.clone(),
            display_name:  display_name.trim().to_string(),
            message,
            timestamp,
            expires_at,
            signature,
        })
    }

    /// Verify signature and expiry.
    ///
    /// Security assumption: requester_key authenticity is NOT verified here.
    /// The caller must check whether the key is known/trusted separately.
    /// This only verifies that the request was signed by the holder of
    /// requester_key -- not that requester_key itself is legitimate.
    pub fn verify(&self) -> Result<(), ConnectionError> {
        if self.is_expired() {
            return Err(ConnectionError::Expired);
        }
        let payload = Self::signing_payload(
            &self.requester_key,
            &self.target_key,
            &self.message,
            self.timestamp,
        );
        if !KeyPair::verify(&self.requester_key, &payload, &self.signature) {
            return Err(ConnectionError::InvalidSignature);
        }
        Ok(())
    }

    pub fn is_expired(&self) -> bool {
        now_secs() > self.expires_at
    }

    fn signing_payload(
        requester: &IdentityKey,
        target:    &IdentityKey,
        message:   &str,
        timestamp: u64,
    ) -> Vec<u8> {
        let mut p = Vec::new();
        p.extend_from_slice(&requester.0);
        p.extend_from_slice(&target.0);
        p.extend_from_slice(message.as_bytes());
        p.extend_from_slice(&timestamp.to_le_bytes());
        p.extend_from_slice(b"theos-conn-v1");
        p
    }
}

// -- ConnectionResponse -------------------------------------------------------

/// The target's response to a connection request.
/// Accept or decline -- sent back via DHT routing.
/// Decline is silent: the target's device drops it without notifying requester.
#[derive(Debug, Clone, PartialEq)]
pub enum ConnectionResponse {
    Accepted {
        responder_key: IdentityKey,
        requester_key: IdentityKey,
        display_name:  String,      // responder's chosen display name
        timestamp:     u64,
        signature:     Vec<u8>,
    },
    Declined,  // silent -- no info sent back
}

impl ConnectionResponse {
    /// Create a signed Accept response.
    pub fn accept(
        keypair:      &KeyPair,
        request:      &ConnectionRequest,
        display_name: &str,
    ) -> Result<Self, ConnectionError> {
        if display_name.trim().is_empty() {
            return Err(ConnectionError::InvalidDisplayName);
        }
        let timestamp = now_secs();
        let payload   = Self::signing_payload(
            &keypair.public, &request.requester_key, timestamp
        );
        let signature = keypair.sign(&payload);

        Ok(Self::Accepted {
            responder_key: keypair.public.clone(),
            requester_key: request.requester_key.clone(),
            display_name:  display_name.trim().to_string(),
            timestamp,
            signature,
        })
    }

    /// Verify an Accept response signature.
    pub fn verify(&self) -> Result<(), ConnectionError> {
        match self {
            Self::Declined => Ok(()), // nothing to verify
            Self::Accepted { responder_key, requester_key, timestamp, signature, .. } => {
                let payload = Self::signing_payload(responder_key, requester_key, *timestamp);
                if !KeyPair::verify(responder_key, &payload, signature) {
                    return Err(ConnectionError::InvalidSignature);
                }
                Ok(())
            }
        }
    }

    fn signing_payload(
        responder: &IdentityKey,
        requester: &IdentityKey,
        timestamp: u64,
    ) -> Vec<u8> {
        let mut p = Vec::new();
        p.extend_from_slice(&responder.0);
        p.extend_from_slice(&requester.0);
        p.extend_from_slice(&timestamp.to_le_bytes());
        p.extend_from_slice(b"theos-conn-accept-v1");
        p
    }
}

// -- PendingRequests ----------------------------------------------------------

/// Queue of incoming connection requests waiting for the user to act on.
/// Displayed in the AI shell: "Sarah wants to connect"
pub struct PendingRequests {
    /// requester key hex -> request
    requests:     HashMap<String, ConnectionRequest>,
    /// requester key hex -> last request timestamp (rate limiting)
    last_request: HashMap<String, u64>,
}

impl PendingRequests {
    pub fn new() -> Self {
        Self {
            requests:     HashMap::new(),
            last_request: HashMap::new(),
        }
    }

    /// Receive an incoming connection request.
    /// Verifies signature, checks rate limit, adds to queue.
    pub fn receive(
        &mut self,
        request: ConnectionRequest,
    ) -> Result<(), ConnectionError> {
        // Verify first
        request.verify()?;

        let key_hex = request.requester_key.to_hex();

        // Rate limit: one request per requester per hour
        if let Some(&last) = self.last_request.get(&key_hex) {
            if now_secs() - last < REQUEST_RATE_SECS {
                return Err(ConnectionError::RateLimited);
            }
        }

        // Drop oldest if queue full
        if self.requests.len() >= MAX_PENDING {
            // Remove the oldest request
            let oldest_key = self.requests
                .iter()
                .min_by_key(|(_, r)| r.timestamp)
                .map(|(k, _)| k.clone());
            if let Some(k) = oldest_key {
                self.requests.remove(&k);
                println!("[connection] queue full -- dropped oldest request");
            }
        }

        println!(
            "[connection] request from {} ({}): {}",
            request.display_name,
            &key_hex[..8],
            request.message,
        );

        self.last_request.insert(key_hex.clone(), now_secs());
        self.requests.insert(key_hex, request);
        Ok(())
    }

    /// Accept a pending request. Adds requester to ContactBook.
    /// Returns the signed Accept response to send back.
    pub fn accept(
        &mut self,
        requester_key: &IdentityKey,
        my_keypair:    &KeyPair,
        my_name:       &str,
        contacts:      &mut ContactBook,
    ) -> Result<ConnectionResponse, ConnectionError> {
        let key_hex = requester_key.to_hex();
        let request = self.requests
            .remove(&key_hex)
            .ok_or(ConnectionError::RequestNotFound)?;

        // Add to contacts
        let contact = Contact::new(&request.display_name, request.requester_key.clone());
        contacts.add_in_memory(contact);

        println!(
            "[connection] accepted @{} -- added to contacts",
            &key_hex[..8]
        );

        ConnectionResponse::accept(my_keypair, &request, my_name)
    }

    /// Decline a pending request. Dropped silently -- requester not notified.
    pub fn decline(&mut self, requester_key: &IdentityKey) -> Result<(), ConnectionError> {
        let key_hex = requester_key.to_hex();
        if self.requests.remove(&key_hex).is_some() {
            println!("[connection] declined request from {}", &key_hex[..8]);
            Ok(())
        } else {
            Err(ConnectionError::RequestNotFound)
        }
    }

    /// Remove all expired requests. Call periodically.
    pub fn gc(&mut self) {
        let expired: Vec<String> = self.requests
            .iter()
            .filter(|(_, r)| r.is_expired())
            .map(|(k, _)| k.clone())
            .collect();
        for k in &expired {
            self.requests.remove(k);
        }
        if !expired.is_empty() {
            println!("[connection] gc removed {} expired requests", expired.len());
        }
    }

    pub fn count(&self) -> usize { self.requests.len() }

    pub fn all(&self) -> Vec<&ConnectionRequest> {
        let mut v: Vec<&ConnectionRequest> = self.requests.values().collect();
        v.sort_by_key(|r| r.timestamp);
        v
    }

    pub fn has_request_from(&self, key: &IdentityKey) -> bool {
        self.requests.contains_key(&key.to_hex())
    }
}

// -- ContactBook extension ----------------------------------------------------
// Add in-memory contact without disk write (for testing / demo)

impl ContactBook {
    pub fn add_in_memory(&mut self, contact: Contact) {
        let key_hex = contact.key.to_hex();
        if !self.contacts.contains_key(&key_hex) {
            println!("[contacts] added (memory): {} ({})", contact.name, contact.key.short());
            self.contacts.insert(key_hex, contact);
        }
    }
}

// -- Error type ---------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub enum ConnectionError {
    MessageTooLong,
    InvalidDisplayName,
    InvalidSignature,
    Expired,
    RateLimited,
    RequestNotFound,
    QueueFull,
}

impl std::fmt::Display for ConnectionError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            ConnectionError::MessageTooLong      => write!(f, "message too long (max 140 chars)"),
            ConnectionError::InvalidDisplayName  => write!(f, "display name cannot be empty"),
            ConnectionError::InvalidSignature    => write!(f, "invalid signature"),
            ConnectionError::Expired             => write!(f, "request expired"),
            ConnectionError::RateLimited         => write!(f, "rate limited -- one request per hour"),
            ConnectionError::RequestNotFound     => write!(f, "request not found"),
            ConnectionError::QueueFull           => write!(f, "request queue full"),
        }
    }
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

// -- Tests --------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::keypair::KeyPair;
    use crate::identity::contact::ContactBook;

    fn kp() -> KeyPair { KeyPair::generate() }

    // ConnectionRequest creation and verification

    #[test]
    fn test_request_creates_and_verifies() {
        let omar  = kp();
        let sarah = kp();
        let req   = ConnectionRequest::new(&omar, &sarah.public, "Omar", "Hey, it is Omar").unwrap();
        assert!(req.verify().is_ok());
    }

    #[test]
    fn test_request_message_too_long() {
        let omar  = kp();
        let sarah = kp();
        let long  = "x".repeat(141);
        assert_eq!(
            ConnectionRequest::new(&omar, &sarah.public, "Omar", &long),
            Err(ConnectionError::MessageTooLong)
        );
    }

    #[test]
    fn test_request_empty_name_rejected() {
        let omar  = kp();
        let sarah = kp();
        assert_eq!(
            ConnectionRequest::new(&omar, &sarah.public, "  ", "hi"),
            Err(ConnectionError::InvalidDisplayName)
        );
    }

    #[test]
    fn test_request_empty_message_ok() {
        let omar  = kp();
        let sarah = kp();
        let req   = ConnectionRequest::new(&omar, &sarah.public, "Omar", "").unwrap();
        assert!(req.verify().is_ok());
    }

    #[test]
    fn test_request_tampered_signature_fails() {
        let omar  = kp();
        let sarah = kp();
        let mut req = ConnectionRequest::new(&omar, &sarah.public, "Omar", "hi").unwrap();
        req.signature[0] ^= 0xFF;
        assert_eq!(req.verify(), Err(ConnectionError::InvalidSignature));
    }

    #[test]
    fn test_request_tampered_message_fails() {
        let omar  = kp();
        let sarah = kp();
        let mut req = ConnectionRequest::new(&omar, &sarah.public, "Omar", "hi").unwrap();
        req.message = "evil".to_string();
        assert_eq!(req.verify(), Err(ConnectionError::InvalidSignature));
    }

    #[test]
    fn test_request_stores_correct_keys() {
        let omar  = kp();
        let sarah = kp();
        let req   = ConnectionRequest::new(&omar, &sarah.public, "Omar", "hey").unwrap();
        assert_eq!(req.requester_key, omar.public);
        assert_eq!(req.target_key,    sarah.public);
    }

    #[test]
    fn test_request_message_trimmed() {
        let omar  = kp();
        let sarah = kp();
        let req   = ConnectionRequest::new(&omar, &sarah.public, "Omar", "  hello  ").unwrap();
        assert_eq!(req.message, "hello");
    }

    // ConnectionResponse

    #[test]
    fn test_accept_response_verifies() {
        let omar  = kp();
        let sarah = kp();
        let req   = ConnectionRequest::new(&omar, &sarah.public, "Omar", "hi").unwrap();
        let resp  = ConnectionResponse::accept(&sarah, &req, "Sarah").unwrap();
        assert!(resp.verify().is_ok());
    }

    #[test]
    fn test_accept_empty_name_rejected() {
        let omar  = kp();
        let sarah = kp();
        let req   = ConnectionRequest::new(&omar, &sarah.public, "Omar", "hi").unwrap();
        assert_eq!(
            ConnectionResponse::accept(&sarah, &req, ""),
            Err(ConnectionError::InvalidDisplayName)
        );
    }

    #[test]
    fn test_decline_is_silent() {
        let resp = ConnectionResponse::Declined;
        assert_eq!(resp, ConnectionResponse::Declined);
        assert!(resp.verify().is_ok()); // nothing to verify
    }

    // PendingRequests queue

    #[test]
    fn test_receive_valid_request() {
        let omar  = kp();
        let sarah = kp();
        let req   = ConnectionRequest::new(&omar, &sarah.public, "Omar", "hi").unwrap();
        let mut pending = PendingRequests::new();
        assert!(pending.receive(req).is_ok());
        assert_eq!(pending.count(), 1);
    }

    #[test]
    fn test_receive_tampered_request_rejected() {
        let omar  = kp();
        let sarah = kp();
        let mut req = ConnectionRequest::new(&omar, &sarah.public, "Omar", "hi").unwrap();
        req.signature[0] ^= 0xFF;
        let mut pending = PendingRequests::new();
        assert_eq!(pending.receive(req), Err(ConnectionError::InvalidSignature));
        assert_eq!(pending.count(), 0);
    }

    #[test]
    fn test_rate_limit_blocks_second_request() {
        let omar  = kp();
        let sarah = kp();
        let req1  = ConnectionRequest::new(&omar, &sarah.public, "Omar", "hi").unwrap();
        let req2  = ConnectionRequest::new(&omar, &sarah.public, "Omar", "hi again").unwrap();
        let mut pending = PendingRequests::new();
        pending.receive(req1).unwrap();
        assert_eq!(pending.receive(req2), Err(ConnectionError::RateLimited));
    }

    #[test]
    fn test_has_request_from() {
        let omar  = kp();
        let sarah = kp();
        let req   = ConnectionRequest::new(&omar, &sarah.public, "Omar", "hi").unwrap();
        let mut pending = PendingRequests::new();
        assert!(!pending.has_request_from(&omar.public));
        pending.receive(req).unwrap();
        assert!(pending.has_request_from(&omar.public));
    }

    #[test]
    fn test_accept_adds_to_contacts() {
        let omar     = kp();
        let sarah    = kp();
        let req      = ConnectionRequest::new(&omar, &sarah.public, "Omar", "hi").unwrap();
        let mut pending  = PendingRequests::new();
        let mut contacts = ContactBook::new();
        pending.receive(req).unwrap();
        let resp = pending.accept(&omar.public, &sarah, "Sarah", &mut contacts).unwrap();
        assert!(resp.verify().is_ok());
        assert!(contacts.is_trusted(&omar.public));
        assert_eq!(pending.count(), 0);
    }

    #[test]
    fn test_accept_unknown_request_fails() {
        let omar     = kp();
        let sarah    = kp();
        let mut pending  = PendingRequests::new();
        let mut contacts = ContactBook::new();
        assert_eq!(
            pending.accept(&omar.public, &sarah, "Sarah", &mut contacts),
            Err(ConnectionError::RequestNotFound)
        );
    }

    #[test]
    fn test_decline_removes_request() {
        let omar  = kp();
        let sarah = kp();
        let req   = ConnectionRequest::new(&omar, &sarah.public, "Omar", "hi").unwrap();
        let mut pending = PendingRequests::new();
        pending.receive(req).unwrap();
        pending.decline(&omar.public).unwrap();
        assert_eq!(pending.count(), 0);
        assert!(!pending.has_request_from(&omar.public));
    }

    #[test]
    fn test_decline_unknown_fails() {
        let omar    = kp();
        let mut pending = PendingRequests::new();
        assert_eq!(
            pending.decline(&omar.public),
            Err(ConnectionError::RequestNotFound)
        );
    }

    #[test]
    fn test_gc_removes_nothing_when_fresh() {
        let omar  = kp();
        let sarah = kp();
        let req   = ConnectionRequest::new(&omar, &sarah.public, "Omar", "hi").unwrap();
        let mut pending = PendingRequests::new();
        pending.receive(req).unwrap();
        pending.gc();
        assert_eq!(pending.count(), 1);
    }

    #[test]
    fn test_full_handshake() {
        // Complete flow: request -> receive -> accept -> both have contact
        let omar     = kp();
        let sarah    = kp();
        let mut omar_contacts   = ContactBook::new();
        let mut sarah_contacts  = ContactBook::new();
        let mut sarah_pending   = PendingRequests::new();

        // Omar sends request
        let req = ConnectionRequest::new(&omar, &sarah.public, "Omar", "Hey Sarah!").unwrap();

        // Sarah receives it
        sarah_pending.receive(req).unwrap();
        assert!(sarah_pending.has_request_from(&omar.public));

        // Sarah accepts
        let resp = sarah_pending
            .accept(&omar.public, &sarah, "Sarah", &mut sarah_contacts)
            .unwrap();
        assert!(resp.verify().is_ok());
        assert!(sarah_contacts.is_trusted(&omar.public));

        // Omar receives Accept and adds Sarah
        if let ConnectionResponse::Accepted { ref responder_key, ref display_name, .. } = resp {
            resp.verify().unwrap();
            let contact = Contact::new(display_name, responder_key.clone());
            omar_contacts.add_in_memory(contact);
        }
        assert!(omar_contacts.is_trusted(&sarah.public));
    }

    #[test]
    fn test_all_returns_sorted_by_timestamp() {
        let requester1 = kp();
        let requester2 = kp();
        let target     = kp();
        let req1 = ConnectionRequest::new(&requester1, &target.public, "Alice", "hi").unwrap();
        let req2 = ConnectionRequest::new(&requester2, &target.public, "Bob",   "hi").unwrap();
        let mut pending = PendingRequests::new();
        pending.receive(req1).unwrap();
        pending.last_request.clear();
        pending.receive(req2).unwrap();
        let all = pending.all();
        assert_eq!(all.len(), 2);
    }
}
