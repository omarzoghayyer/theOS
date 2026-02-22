// dht/mod.rs — theOS Distributed Hash Table
// Peer discovery without any central server
// Every theOS device participates in routing
// Based on Kademlia DHT — same algorithm as BitTorrent

pub mod node;
pub mod routing;
pub mod message;

pub use node::{DhtNode, NodeId};
pub use routing::RoutingTable;
pub use message::DhtMessage;

use std::net::SocketAddr;

/// The main DHT instance — one per device
pub struct TheOsDht {
    pub node:    DhtNode,
    pub routing: RoutingTable,
}

impl TheOsDht {
    pub fn new(identity_key: &[u8; 32]) -> Self {
        // Node ID is derived from identity key — same key, same ID always
        let node_id = NodeId::from_identity(identity_key);
        let node    = DhtNode::new(node_id.clone());
        let routing = RoutingTable::new(node_id);

        println!("[dht] node ID: {}", node.id.short());
        Self { node, routing }
    }

    /// Announce your presence to the DHT network
    /// Called on boot and whenever your satellite IP changes
    pub fn announce(&mut self, addr: SocketAddr) {
        self.node.addr = Some(addr);
        println!("[dht] announced at: {}", addr);
    }

    /// Look up where a contact's device is right now
    /// Returns their current satellite IP if found
    pub fn find_peer(&self, identity_key: &[u8; 32]) -> Option<SocketAddr> {
        let target = NodeId::from_identity(identity_key);
        self.routing.find_closest(&target)
            .and_then(|node| node.addr)
    }

    /// Add a bootstrap node — first contact when joining the network
    pub fn add_bootstrap(&mut self, addr: SocketAddr) {
        println!("[dht] bootstrap node: {}", addr);
        // In production: ping the bootstrap node, get initial routing table
        let _ = addr;
    }

    /// Update routing table when we hear from another node
    pub fn heard_from(&mut self, node_id: NodeId, addr: SocketAddr) {
        self.routing.update(node_id, addr);
    }
}
