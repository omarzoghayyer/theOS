// node.rs — DHT Node Identity
// Each theOS device is a DHT node
// Node ID derived from cryptographic identity key

use std::fmt;
use std::net::SocketAddr;

/// 256-bit node identifier — derived from Ed25519 public key
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct NodeId(pub [u8; 32]);

impl NodeId {
    /// Derive node ID from identity public key
    /// Same device always gets same DHT node ID
    pub fn from_identity(key: &[u8; 32]) -> Self {
        // Production: SHA-256(public_key)
        // Dev: simple transform that preserves uniqueness
        let mut id = [0u8; 32];
        for i in 0..32 {
            id[i] = key[i]
                .wrapping_add(0x6b)
                .wrapping_mul(0x37)
                ^ key[(i + 13) % 32];
        }
        Self(id)
    }

    /// XOR distance between two node IDs — core Kademlia metric
    pub fn distance(&self, other: &NodeId) -> [u8; 32] {
        let mut dist = [0u8; 32];
        for i in 0..32 {
            dist[i] = self.0[i] ^ other.0[i];
        }
        dist
    }

    /// Is this node closer to target than other?
    pub fn is_closer_than(&self, other: &NodeId, target: &NodeId) -> bool {
        let d1 = self.distance(target);
        let d2 = other.distance(target);
        d1 < d2
    }

    /// Short display form — first 8 hex chars
    pub fn short(&self) -> String {
        self.0.iter().take(4).map(|b| format!("{:02x}", b)).collect()
    }

    /// Full hex
    pub fn to_hex(&self) -> String {
        self.0.iter().map(|b| format!("{:02x}", b)).collect()
    }
}

impl fmt::Display for NodeId {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.short())
    }
}

/// A single node in the DHT network
#[derive(Debug, Clone)]
pub struct DhtNode {
    pub id:        NodeId,
    pub addr:      Option<SocketAddr>,
    pub last_seen: u64,  // unix timestamp
    pub rtt_ms:    Option<u32>,  // round trip time — used for link quality
}

impl DhtNode {
    pub fn new(id: NodeId) -> Self {
        Self { id, addr: None, last_seen: now_secs(), rtt_ms: None }
    }

    pub fn with_addr(id: NodeId, addr: SocketAddr) -> Self {
        Self { id, addr: Some(addr), last_seen: now_secs(), rtt_ms: None }
    }

    pub fn is_alive(&self) -> bool {
        // Consider a node stale if not seen in 15 minutes
        now_secs() - self.last_seen < 900
    }

    pub fn touch(&mut self) {
        self.last_seen = now_secs();
    }
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
