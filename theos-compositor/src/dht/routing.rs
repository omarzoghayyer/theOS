// routing.rs — Kademlia Routing Table
// Organizes known peers into k-buckets by XOR distance
// Each bucket holds up to K=20 nodes at a given distance range

use super::node::{DhtNode, NodeId};
use std::net::SocketAddr;

const K: usize = 20;        // bucket size — standard Kademlia value
const BITS: usize = 256;    // node ID size in bits

/// A single k-bucket — nodes at a specific distance range
#[derive(Debug, Clone)]
struct KBucket {
    nodes: Vec<DhtNode>,
}

impl KBucket {
    fn new() -> Self { Self { nodes: Vec::new() } }

    fn add(&mut self, node: DhtNode) {
        // If already exists, move to end (most recently seen)
        if let Some(pos) = self.nodes.iter().position(|n| n.id == node.id) {
            self.nodes.remove(pos);
            self.nodes.push(node);
            return;
        }
        // If bucket not full, add
        if self.nodes.len() < K {
            self.nodes.push(node);
            return;
        }
        // Bucket full — remove oldest stale node if any
        if let Some(pos) = self.nodes.iter().position(|n| !n.is_alive()) {
            self.nodes.remove(pos);
            self.nodes.push(node);
        }
        // Otherwise drop — bucket is full of live nodes
    }

    fn find_closest(&self, target: &NodeId, count: usize) -> Vec<&DhtNode> {
        let mut sorted: Vec<&DhtNode> = self.nodes.iter()
            .filter(|n| n.addr.is_some())
            .collect();
        sorted.sort_by(|a, b| {
            let da = a.id.distance(target);
            let db = b.id.distance(target);
            da.cmp(&db)
        });
        sorted.into_iter().take(count).collect()
    }
}

/// The full routing table — 256 k-buckets
pub struct RoutingTable {
    pub own_id:  NodeId,
    buckets:     Vec<KBucket>,
    total_nodes: usize,
}

impl RoutingTable {
    pub fn new(own_id: NodeId) -> Self {
        Self {
            own_id,
            buckets: (0..BITS).map(|_| KBucket::new()).collect(),
            total_nodes: 0,
        }
    }

    /// Add or update a node in the routing table
    pub fn update(&mut self, id: NodeId, addr: SocketAddr) {
        if id == self.own_id { return; } // don't add self
        let bucket_idx = self.bucket_index(&id);
        let node = DhtNode::with_addr(id, addr);
        let was_empty = self.buckets[bucket_idx].nodes.is_empty();
        self.buckets[bucket_idx].add(node);
        if was_empty { self.total_nodes += 1; }
        println!("[dht] routing table: {} nodes", self.total_nodes);
    }

    /// Find the closest node to a target ID
    pub fn find_closest(&self, target: &NodeId) -> Option<DhtNode> {
        let mut best: Vec<&DhtNode> = Vec::new();

        for bucket in &self.buckets {
            best.extend(bucket.find_closest(target, 3));
        }

        best.sort_by(|a, b| {
            let da = a.id.distance(target);
            let db = b.id.distance(target);
            da.cmp(&db)
        });

        best.first().map(|n| (*n).clone())
    }

    /// Find K closest nodes to a target
    pub fn find_k_closest(&self, target: &NodeId) -> Vec<DhtNode> {
        let mut all: Vec<&DhtNode> = self.buckets.iter()
            .flat_map(|b| b.nodes.iter())
            .filter(|n| n.addr.is_some())
            .collect();

        all.sort_by(|a, b| {
            let da = a.id.distance(target);
            let db = b.id.distance(target);
            da.cmp(&db)
        });

        all.into_iter().take(K).cloned().collect()
    }

    /// Which bucket does this node ID belong in?
    fn bucket_index(&self, id: &NodeId) -> usize {
        let dist = self.own_id.distance(id);
        // Find the first differing bit
        for (byte_idx, byte) in dist.iter().enumerate() {
            if *byte != 0 {
                for bit in (0..8).rev() {
                    if byte & (1 << bit) != 0 {
                        return (byte_idx * 8 + (7 - bit)).min(BITS - 1);
                    }
                }
            }
        }
        0
    }

    pub fn node_count(&self) -> usize {
        self.buckets.iter().map(|b| b.nodes.len()).sum()
    }
}
