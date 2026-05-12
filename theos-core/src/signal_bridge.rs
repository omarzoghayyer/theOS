// signal_bridge.rs -- theOS Signal Protocol Bridge
//
// Enables theOS users to exchange messages with Signal contacts
// without a phone number on the theOS side.
//
// Architecture:
//   theOS uses Ed25519 keypairs as identity (no phone numbers).
//   Signal uses phone numbers + registration IDs.
//
//   The bridge maps theOS identity -> Signal protocol identity by:
//     1. Deriving a Signal registration ID from the Ed25519 pubkey
//     2. Generating X3DH PreKey bundles from Ed25519 key material
//     3. Running Double Ratchet sessions for Signal-compatible encryption
//     4. Routing messages through a companion channel
//
// Protocol distinction (transparent to the user):
//   theOS <-> theOS:  ChaCha20-Poly1305 + Ed25519 (our stack)
//   theOS <-> Signal: Double Ratchet + X3DH (Signal protocol)
//
// Security assumptions (flag for audit):
//   - Registration ID derived from Ed25519 pubkey via SHA-256 truncation.
//   - PreKey material derived from Ed25519 key via SHA-256 (demo only).
//   - Production requires real X3DH with Curve25519 DH operations.
//   - Double Ratchet state must be persisted across sessions.

use crate::identity::keypair::IdentityKey;
use sha2::{Sha256, Digest};
use std::collections::HashMap;

const SIGNED_PREKEY_ID:    u32 = 1;
const REGISTRATION_ID_MAX: u32 = 16380;

// -- Registration ID ----------------------------------------------------------

pub fn derive_registration_id(pubkey: &IdentityKey) -> u32 {
    let mut hasher = Sha256::new();
    hasher.update(&pubkey.0);
    hasher.update(b"theos-signal-regid-v1");
    let hash = hasher.finalize();
    let raw = u16::from_le_bytes([hash[0], hash[1]]) as u32;
    (raw % REGISTRATION_ID_MAX) + 1
}

// -- PreKeyBundle -------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct PreKeyBundle {
    pub registration_id:   u32,
    pub device_id:         u32,
    pub prekey_id:         u32,
    pub prekey_public:     [u8; 32],
    pub signed_prekey_id:  u32,
    pub signed_prekey:     [u8; 32],
    pub signed_prekey_sig: Vec<u8>,
    pub identity_key:      IdentityKey,
}

impl PreKeyBundle {
    /// Demo derivation -- production requires real X3DH with private key.
    /// Security assumption: NOT secure for production. Flag for audit.
    pub fn new_demo(identity: &IdentityKey, prekey_id: u32) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(&identity.0);
        hasher.update(b"theos-prekey-demo");
        hasher.update(&prekey_id.to_le_bytes());
        let prekey_hash = hasher.finalize();
        let mut prekey_public = [0u8; 32];
        prekey_public.copy_from_slice(&prekey_hash[..32]);

        let mut hasher2 = Sha256::new();
        hasher2.update(&identity.0);
        hasher2.update(b"theos-signed-prekey-demo");
        let signed_hash = hasher2.finalize();
        let mut signed_public = [0u8; 32];
        signed_public.copy_from_slice(&signed_hash[..32]);

        Self {
            registration_id:   derive_registration_id(identity),
            device_id:         1,
            prekey_id,
            prekey_public,
            signed_prekey_id:  SIGNED_PREKEY_ID,
            signed_prekey:     signed_public,
            signed_prekey_sig: vec![0u8; 64],
            identity_key:      identity.clone(),
        }
    }
}

// -- Double Ratchet state -----------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub enum RatchetState {
    Uninitialized,
    Active {
        send_chain_key:   [u8; 32],
        recv_chain_key:   [u8; 32],
        send_message_num: u32,
        recv_message_num: u32,
        prev_send_count:  u32,
    },
    Closed,
}

#[derive(Debug, Clone)]
pub struct SignalSession {
    pub contact_key:     IdentityKey,
    pub state:           RatchetState,
    pub skipped_keys:    HashMap<u32, [u8; 32]>,
    pub created_at:      u64,
    pub last_message_at: Option<u64>,
}

impl SignalSession {
    pub fn new(contact_key: IdentityKey) -> Self {
        Self {
            contact_key,
            state:           RatchetState::Uninitialized,
            skipped_keys:    HashMap::new(),
            created_at:      now_secs(),
            last_message_at: None,
        }
    }

    pub fn is_active(&self) -> bool {
        matches!(self.state, RatchetState::Active { .. })
    }

    /// Initialize session from a PreKeyBundle.
    /// Security assumption: demo derivation only. Production needs real X3DH.
    pub fn initialize_from_bundle(&mut self, bundle: &PreKeyBundle, my_key: &IdentityKey) {
        let mut hasher = Sha256::new();
        hasher.update(&my_key.0);
        hasher.update(&bundle.identity_key.0);
        hasher.update(b"theos-x3dh-demo-send");
        let send_hash = hasher.finalize();
        let mut send_chain = [0u8; 32];
        send_chain.copy_from_slice(&send_hash[..32]);

        let mut hasher2 = Sha256::new();
        hasher2.update(&bundle.identity_key.0);
        hasher2.update(&my_key.0);
        hasher2.update(b"theos-x3dh-demo-recv");
        let recv_hash = hasher2.finalize();
        let mut recv_chain = [0u8; 32];
        recv_chain.copy_from_slice(&recv_hash[..32]);

        self.state = RatchetState::Active {
            send_chain_key:   send_chain,
            recv_chain_key:   recv_chain,
            send_message_num: 0,
            recv_message_num: 0,
            prev_send_count:  0,
        };

        println!("[signal_bridge] session initialized with {}", &bundle.identity_key.to_hex()[..8]);
    }

    pub fn next_send_key(&mut self) -> Option<([u8; 32], u32)> {
        match &mut self.state {
            RatchetState::Active { send_chain_key, send_message_num, .. } => {
                let mut hasher = Sha256::new();
                hasher.update(send_chain_key.as_ref());
                hasher.update(b"theos-msg-key");
                let msg_key_hash = hasher.finalize();
                let mut msg_key = [0u8; 32];
                msg_key.copy_from_slice(&msg_key_hash[..32]);

                let mut hasher2 = Sha256::new();
                hasher2.update(send_chain_key.as_ref());
                hasher2.update(b"theos-chain-advance");
                let next_hash = hasher2.finalize();
                send_chain_key.copy_from_slice(&next_hash[..32]);

                let num = *send_message_num;
                *send_message_num += 1;
                self.last_message_at = Some(now_secs());

                Some((msg_key, num))
            }
            _ => None,
        }
    }
}

// -- Protocol routing ---------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub enum MessageProtocol {
    TheOsNative,
    Signal,
}

pub struct SignalBridge {
    pub my_identity:     IdentityKey,
    pub registration_id: u32,
    sessions:            HashMap<String, SignalSession>,
    signal_contacts:     HashMap<String, bool>,
}

impl SignalBridge {
    pub fn new(identity: IdentityKey) -> Self {
        let reg_id = derive_registration_id(&identity);
        println!(
            "[signal_bridge] initialized -- registration_id: {} identity: {}",
            reg_id, &identity.to_hex()[..8]
        );
        Self {
            registration_id: reg_id,
            my_identity:     identity,
            sessions:        HashMap::new(),
            signal_contacts: HashMap::new(),
        }
    }

    pub fn register_signal_contact(&mut self, key: &IdentityKey) {
        self.signal_contacts.insert(key.to_hex(), true);
        println!("[signal_bridge] registered Signal contact: {}", &key.to_hex()[..8]);
    }

    pub fn is_signal_contact(&self, key: &IdentityKey) -> bool {
        self.signal_contacts.get(&key.to_hex()).copied().unwrap_or(false)
    }

    pub fn protocol_for(&self, key: &IdentityKey) -> MessageProtocol {
        if self.is_signal_contact(key) { MessageProtocol::Signal }
        else { MessageProtocol::TheOsNative }
    }

    pub fn establish_session(&mut self, bundle: &PreKeyBundle) {
        let key_hex = bundle.identity_key.to_hex();
        let mut session = SignalSession::new(bundle.identity_key.clone());
        session.initialize_from_bundle(bundle, &self.my_identity);
        self.sessions.insert(key_hex, session);
    }

    pub fn session_mut(&mut self, key: &IdentityKey) -> Option<&mut SignalSession> {
        self.sessions.get_mut(&key.to_hex())
    }

    pub fn session(&self, key: &IdentityKey) -> Option<&SignalSession> {
        self.sessions.get(&key.to_hex())
    }

    pub fn session_count(&self) -> usize { self.sessions.len() }
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

    fn key() -> IdentityKey { KeyPair::generate().public }

    #[test]
    fn test_registration_id_in_range() {
        let k  = key();
        let id = derive_registration_id(&k);
        assert!(id >= 1 && id <= 16380);
    }

    #[test]
    fn test_registration_id_deterministic() {
        let k   = key();
        let id1 = derive_registration_id(&k);
        let id2 = derive_registration_id(&k);
        assert_eq!(id1, id2);
    }

    #[test]
    fn test_registration_id_valid_per_key() {
        let k1 = key();
        let k2 = key();
        assert!(derive_registration_id(&k1) >= 1);
        assert!(derive_registration_id(&k2) >= 1);
    }

    #[test]
    fn test_prekey_bundle_creation() {
        let k      = key();
        let bundle = PreKeyBundle::new_demo(&k, 1);
        assert_eq!(bundle.device_id, 1);
        assert_eq!(bundle.signed_prekey_id, SIGNED_PREKEY_ID);
        assert_eq!(bundle.identity_key, k);
        assert!(bundle.registration_id >= 1 && bundle.registration_id <= 16380);
    }

    #[test]
    fn test_prekey_bundle_deterministic() {
        let k  = key();
        let b1 = PreKeyBundle::new_demo(&k, 1);
        let b2 = PreKeyBundle::new_demo(&k, 1);
        assert_eq!(b1.prekey_public, b2.prekey_public);
        assert_eq!(b1.signed_prekey, b2.signed_prekey);
    }

    #[test]
    fn test_prekey_bundle_different_ids_differ() {
        let k  = key();
        let b1 = PreKeyBundle::new_demo(&k, 1);
        let b2 = PreKeyBundle::new_demo(&k, 2);
        assert_ne!(b1.prekey_public, b2.prekey_public);
    }

    #[test]
    fn test_session_starts_uninitialized() {
        let session = SignalSession::new(key());
        assert!(!session.is_active());
        assert_eq!(session.state, RatchetState::Uninitialized);
    }

    #[test]
    fn test_session_initializes_from_bundle() {
        let alice  = key();
        let bob    = key();
        let bundle = PreKeyBundle::new_demo(&bob, 1);
        let mut session = SignalSession::new(bob.clone());
        session.initialize_from_bundle(&bundle, &alice);
        assert!(session.is_active());
    }

    #[test]
    fn test_session_produces_send_key() {
        let alice  = key();
        let bob    = key();
        let bundle = PreKeyBundle::new_demo(&bob, 1);
        let mut session = SignalSession::new(bob.clone());
        session.initialize_from_bundle(&bundle, &alice);
        let (msg_key, num) = session.next_send_key().unwrap();
        assert_eq!(num, 0);
        assert_ne!(msg_key, [0u8; 32]);
    }

    #[test]
    fn test_session_keys_advance() {
        let alice  = key();
        let bob    = key();
        let bundle = PreKeyBundle::new_demo(&bob, 1);
        let mut session = SignalSession::new(bob.clone());
        session.initialize_from_bundle(&bundle, &alice);
        let (k1, n1) = session.next_send_key().unwrap();
        let (k2, n2) = session.next_send_key().unwrap();
        assert_ne!(k1, k2);
        assert_eq!(n1, 0);
        assert_eq!(n2, 1);
    }

    #[test]
    fn test_session_no_key_when_uninitialized() {
        let mut session = SignalSession::new(key());
        assert!(session.next_send_key().is_none());
    }

    #[test]
    fn test_bridge_creation() {
        let k      = key();
        let bridge = SignalBridge::new(k.clone());
        assert_eq!(bridge.registration_id, derive_registration_id(&k));
        assert_eq!(bridge.session_count(), 0);
    }

    #[test]
    fn test_bridge_protocol_routing() {
        let my_key     = key();
        let signal_key = key();
        let theos_key  = key();
        let mut bridge = SignalBridge::new(my_key);
        bridge.register_signal_contact(&signal_key);
        assert_eq!(bridge.protocol_for(&signal_key), MessageProtocol::Signal);
        assert_eq!(bridge.protocol_for(&theos_key),  MessageProtocol::TheOsNative);
    }

    #[test]
    fn test_bridge_establish_session() {
        let my_key  = key();
        let bob_key = key();
        let bundle  = PreKeyBundle::new_demo(&bob_key, 1);
        let mut bridge = SignalBridge::new(my_key);
        bridge.establish_session(&bundle);
        assert_eq!(bridge.session_count(), 1);
        assert!(bridge.session(&bob_key).unwrap().is_active());
    }

    #[test]
    fn test_bridge_session_message_flow() {
        let alice_key = key();
        let bob_key   = key();
        let bundle    = PreKeyBundle::new_demo(&bob_key, 1);
        let mut bridge = SignalBridge::new(alice_key);
        bridge.establish_session(&bundle);
        let session = bridge.session_mut(&bob_key).unwrap();
        let (msg_key, num) = session.next_send_key().unwrap();
        assert_eq!(num, 0);
        assert_ne!(msg_key, [0u8; 32]);
    }

    #[test]
    fn test_is_signal_contact() {
        let my_key     = key();
        let signal_key = key();
        let mut bridge = SignalBridge::new(my_key);
        assert!(!bridge.is_signal_contact(&signal_key));
        bridge.register_signal_contact(&signal_key);
        assert!(bridge.is_signal_contact(&signal_key));
    }

    #[test]
    fn test_both_sides_active_after_init() {
        let alice_key    = key();
        let bob_key      = key();
        let alice_bundle = PreKeyBundle::new_demo(&alice_key, 1);
        let bob_bundle   = PreKeyBundle::new_demo(&bob_key, 1);

        let mut alice_session = SignalSession::new(bob_key.clone());
        alice_session.initialize_from_bundle(&bob_bundle, &alice_key);

        let mut bob_session = SignalSession::new(alice_key.clone());
        bob_session.initialize_from_bundle(&alice_bundle, &bob_key);

        assert!(alice_session.is_active());
        assert!(bob_session.is_active());
    }
}
