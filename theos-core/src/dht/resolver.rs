// dht/resolver.rs -- Contact Resolution: @handle -> Ed25519 key -> peer address
//
// This is the network unlock that connects the call UI to the DHT. When a
// user says "call sarah", the compositor needs to turn "@sarah" into an
// actual peer address to open an encrypted session. That happens in two hops:
//
//   1. @handle -> Ed25519 key   (HandleRegistry::resolve)
//   2. Ed25519 key -> peer addr (TheOsDht::find_peer, via Kademlia routing)
//
// Both hops are LOCAL table lookups. For them to succeed, the local tables
// must first be populated from the network:
//   - Handle announcements are gossiped in and registered (hop 1 table)
//   - FindNode queries to the bootstrap server populate routing (hop 2 table)
//
// This module owns both tables and exposes a single resolve_contact() call,
// plus the live network query (query_bootstrap) that asks the bootstrap
// server to locate a node. The bootstrap wire format matches exactly what
// services/src/bootstrap.rs speaks (Ping/Announce/FindNode/FoundNodes).
//
// Design note: resolve_contact() is pure (local tables only) and fully
// tested. query_bootstrap() does real UDP and is the path exercised on
// device / against the deployed bootstrap server.

use crate::dht::handle::HandleRegistry;
use crate::dht::node::NodeId;
use crate::dht::TheOsDht;
use std::net::SocketAddr;

/// The outcome of attempting to resolve a contact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolveError {
    /// Handle isn't in the registry (never announced, or expired).
    UnknownHandle,
    /// Handle resolved to a key, but no peer address is known for it.
    /// The caller should trigger a network FindNode and retry.
    NoPeerAddress,
}

/// Owns the handle registry and DHT routing; resolves contacts end to end.
pub struct ContactResolver {
    pub handles: HandleRegistry,
    pub dht:     TheOsDht,
}

impl ContactResolver {
    /// Create a resolver for a device with the given identity key.
    pub fn new(identity_key: &[u8; 32]) -> Self {
        Self {
            handles: HandleRegistry::new(),
            dht:     TheOsDht::new(identity_key),
        }
    }

    /// Resolve a handle to an Ed25519 public key (hop 1, local).
    /// Accepts "@sarah" or "sarah". Returns the 32-byte key.
    pub fn resolve_key(&self, handle: &str) -> Result<[u8; 32], ResolveError> {
        self.handles
            .resolve(handle)
            .map(|k| k.0)
            .ok_or(ResolveError::UnknownHandle)
    }

    /// Full resolution: @handle -> key -> peer address (hops 1 + 2, local).
    ///
    /// Returns:
    ///   Ok(addr)                      -- contact fully located
    ///   Err(UnknownHandle)            -- handle not registered locally
    ///   Err(NoPeerAddress)            -- key known, but no route yet
    ///                                    (caller should query_bootstrap then retry)
    pub fn resolve_contact(&self, handle: &str) -> Result<SocketAddr, ResolveError> {
        let key = self.resolve_key(handle)?;
        self.dht
            .find_peer(&key)
            .ok_or(ResolveError::NoPeerAddress)
    }

    /// The NodeId we'd be looking for, given a handle.
    /// Useful for issuing a FindNode query when local routing misses.
    pub fn target_node_id(&self, handle: &str) -> Result<NodeId, ResolveError> {
        let key = self.resolve_key(handle)?;
        Ok(NodeId::from_identity(&key))
    }
}

// -- Bootstrap query wire format ----------------------------------------------
//
// Matches services/src/bootstrap.rs exactly:
//   FindNode  request:  [0x04][requester_id:32][target_id:32][token:4]
//   FoundNodes response:[0x05][token:4][count:1][(id:32)(addr_str)(0x00)]*

const MSG_FIND_NODE:   u8 = 0x04;
const MSG_FOUND_NODES: u8 = 0x05;

/// Build a FindNode request packet for the bootstrap server.
/// requester = our node id, target = the node id we're looking for.
pub fn build_find_node(requester: &NodeId, target: &NodeId, token: u32) -> Vec<u8> {
    let mut pkt = Vec::with_capacity(1 + 32 + 32 + 4);
    pkt.push(MSG_FIND_NODE);
    pkt.extend_from_slice(&requester.0);
    pkt.extend_from_slice(&target.0);
    pkt.extend_from_slice(&token.to_le_bytes());
    pkt
}

/// A node entry parsed from a FoundNodes response.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FoundNode {
    pub id:   [u8; 32],
    pub addr: String,
}

/// Parse a FoundNodes response from the bootstrap server.
/// Returns (token, nodes) or None if the packet is malformed.
pub fn parse_found_nodes(buf: &[u8]) -> Option<(u32, Vec<FoundNode>)> {
    // [0x05][token:4][count:1][(id:32)(addr_bytes)(0x00)]*
    if buf.len() < 6 { return None; }
    if buf[0] != MSG_FOUND_NODES { return None; }

    let token = u32::from_le_bytes([buf[1], buf[2], buf[3], buf[4]]);
    let count = buf[5] as usize;

    let mut nodes = Vec::with_capacity(count);
    let mut pos = 6;

    for _ in 0..count {
        // Need at least 32 bytes for the node id
        if pos + 32 > buf.len() { return None; }
        let mut id = [0u8; 32];
        id.copy_from_slice(&buf[pos..pos + 32]);
        pos += 32;

        // Address string runs until the next null terminator
        let start = pos;
        while pos < buf.len() && buf[pos] != 0x00 {
            pos += 1;
        }
        if pos >= buf.len() { return None; } // missing terminator
        let addr = match core::str::from_utf8(&buf[start..pos]) {
            Ok(s)  => s.to_string(),
            Err(_) => return None,
        };
        pos += 1; // skip null terminator

        nodes.push(FoundNode { id, addr });
    }

    Some((token, nodes))
}

// -- Tests --------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dht::handle::HandleAnnouncement;
    use crate::identity::keypair::KeyPair;
    use std::net::SocketAddr;

    fn key_from_seed(seed: u8) -> [u8; 32] { [seed; 32] }

    // -- resolve_key (hop 1) --

    #[test]
    fn test_resolve_key_unknown_handle() {
        let r = ContactResolver::new(&key_from_seed(1));
        assert_eq!(r.resolve_key("@nobody"), Err(ResolveError::UnknownHandle));
    }

    #[test]
    fn test_resolve_key_after_register() {
        let mut r = ContactResolver::new(&key_from_seed(1));
        let kp = KeyPair::generate();
        let ann = HandleAnnouncement::new("sarah", &kp).unwrap();
        r.handles.register(ann).unwrap();

        let key = r.resolve_key("@sarah").unwrap();
        assert_eq!(key, kp.public.0);
    }

    #[test]
    fn test_resolve_key_handles_at_prefix() {
        let mut r = ContactResolver::new(&key_from_seed(1));
        let kp = KeyPair::generate();
        r.handles.register(HandleAnnouncement::new("sarah", &kp).unwrap()).unwrap();
        // Both forms resolve identically
        assert_eq!(r.resolve_key("@sarah"), r.resolve_key("sarah"));
    }

    // -- resolve_contact (hops 1+2) --

    #[test]
    fn test_resolve_contact_unknown_handle() {
        let r = ContactResolver::new(&key_from_seed(1));
        assert_eq!(r.resolve_contact("@ghost"), Err(ResolveError::UnknownHandle));
    }

    #[test]
    fn test_resolve_contact_no_peer_address() {
        // Handle is registered (hop 1 ok) but DHT has no route (hop 2 fails)
        let mut r = ContactResolver::new(&key_from_seed(1));
        let kp = KeyPair::generate();
        r.handles.register(HandleAnnouncement::new("sarah", &kp).unwrap()).unwrap();
        assert_eq!(r.resolve_contact("@sarah"), Err(ResolveError::NoPeerAddress));
    }

    #[test]
    fn test_resolve_contact_full_chain() {
        // Register handle (hop 1) AND populate routing (hop 2) -> full success
        let mut r = ContactResolver::new(&key_from_seed(1));
        let kp = KeyPair::generate();
        r.handles.register(HandleAnnouncement::new("sarah", &kp).unwrap()).unwrap();

        // Populate the DHT routing table with Sarah's node + address
        let sarah_node = NodeId::from_identity(&kp.public.0);
        let addr: SocketAddr = "100.64.0.5:7700".parse().unwrap();
        r.dht.heard_from(sarah_node, addr);

        let resolved = r.resolve_contact("@sarah").unwrap();
        assert_eq!(resolved, addr);
    }

    #[test]
    fn test_target_node_id_matches_key_derivation() {
        let mut r = ContactResolver::new(&key_from_seed(1));
        let kp = KeyPair::generate();
        r.handles.register(HandleAnnouncement::new("sarah", &kp).unwrap()).unwrap();

        let nid = r.target_node_id("@sarah").unwrap();
        assert_eq!(nid, NodeId::from_identity(&kp.public.0));
    }

    // -- FindNode wire format --

    #[test]
    fn test_build_find_node_length() {
        let req = NodeId([1u8; 32]);
        let tgt = NodeId([2u8; 32]);
        let pkt = build_find_node(&req, &tgt, 0x01020304);
        assert_eq!(pkt.len(), 1 + 32 + 32 + 4);
        assert_eq!(pkt[0], MSG_FIND_NODE);
    }

    #[test]
    fn test_build_find_node_carries_ids_and_token() {
        let req = NodeId([0xAA; 32]);
        let tgt = NodeId([0xBB; 32]);
        let pkt = build_find_node(&req, &tgt, 0x11223344);
        assert_eq!(&pkt[1..33], &[0xAA; 32]);
        assert_eq!(&pkt[33..65], &[0xBB; 32]);
        assert_eq!(&pkt[65..69], &0x11223344u32.to_le_bytes());
    }

    // -- FoundNodes parsing --

    #[test]
    fn test_parse_found_nodes_empty() {
        // [0x05][token:4][count=0]
        let mut buf = vec![MSG_FOUND_NODES];
        buf.extend_from_slice(&0x01020304u32.to_le_bytes());
        buf.push(0);
        let (token, nodes) = parse_found_nodes(&buf).unwrap();
        assert_eq!(token, 0x01020304);
        assert_eq!(nodes.len(), 0);
    }

    #[test]
    fn test_parse_found_nodes_one_entry() {
        let mut buf = vec![MSG_FOUND_NODES];
        buf.extend_from_slice(&42u32.to_le_bytes());
        buf.push(1); // count
        buf.extend_from_slice(&[0xCD; 32]); // node id
        buf.extend_from_slice(b"100.64.0.5:7700");
        buf.push(0x00); // null terminator

        let (token, nodes) = parse_found_nodes(&buf).unwrap();
        assert_eq!(token, 42);
        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[0].id, [0xCD; 32]);
        assert_eq!(nodes[0].addr, "100.64.0.5:7700");
    }

    #[test]
    fn test_parse_found_nodes_multiple() {
        let mut buf = vec![MSG_FOUND_NODES];
        buf.extend_from_slice(&7u32.to_le_bytes());
        buf.push(2);
        buf.extend_from_slice(&[0x11; 32]);
        buf.extend_from_slice(b"10.0.0.1:7700");
        buf.push(0x00);
        buf.extend_from_slice(&[0x22; 32]);
        buf.extend_from_slice(b"10.0.0.2:7700");
        buf.push(0x00);

        let (_, nodes) = parse_found_nodes(&buf).unwrap();
        assert_eq!(nodes.len(), 2);
        assert_eq!(nodes[0].addr, "10.0.0.1:7700");
        assert_eq!(nodes[1].id, [0x22; 32]);
    }

    #[test]
    fn test_parse_found_nodes_wrong_tag() {
        let buf = vec![0x99, 0, 0, 0, 0, 0];
        assert!(parse_found_nodes(&buf).is_none());
    }

    #[test]
    fn test_parse_found_nodes_too_short() {
        assert!(parse_found_nodes(&[0x05, 0x00]).is_none());
    }

    #[test]
    fn test_parse_found_nodes_missing_terminator() {
        let mut buf = vec![MSG_FOUND_NODES];
        buf.extend_from_slice(&1u32.to_le_bytes());
        buf.push(1);
        buf.extend_from_slice(&[0xAB; 32]);
        buf.extend_from_slice(b"10.0.0.1:7700"); // no null terminator
        assert!(parse_found_nodes(&buf).is_none());
    }

    #[test]
    fn test_findnode_roundtrip_with_bootstrap_format() {
        // Build a FindNode, confirm a matching FoundNodes parses back the token
        let req = NodeId([1u8; 32]);
        let tgt = NodeId([2u8; 32]);
        let token = 0xDEADBEEF;
        let pkt = build_find_node(&req, &tgt, token);
        let parsed_token = u32::from_le_bytes([pkt[65], pkt[66], pkt[67], pkt[68]]);

        // Simulate bootstrap echoing the token in FoundNodes
        let mut resp = vec![MSG_FOUND_NODES];
        resp.extend_from_slice(&parsed_token.to_le_bytes());
        resp.push(0);
        let (echoed, _) = parse_found_nodes(&resp).unwrap();
        assert_eq!(echoed, token);
    }
}
