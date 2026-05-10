// message.rs — DHT Wire Protocol
// Messages exchanged between theOS nodes
// Minimal, efficient, encrypted

use super::node::NodeId;
use std::net::SocketAddr;

/// Message types in the theOS DHT protocol
#[derive(Debug, Clone)]
pub enum DhtMessage {
    /// "I'm here at this address" — broadcast on boot
    Announce {
        node_id:  NodeId,
        addr:     SocketAddr,
        version:  u8,
    },

    /// "Where is this node?" — lookup request
    FindNode {
        requester: NodeId,
        target:    NodeId,
        token:     u32,    // random nonce to match responses
    },

    /// Response to FindNode — here are the closest nodes I know
    FoundNodes {
        token:     u32,
        nodes:     Vec<(NodeId, SocketAddr)>,
    },

    /// "I want to call this identity key" — pre-call routing
    CallRoute {
        caller_key:   [u8; 32],  // Ed25519 public key
        target_key:   [u8; 32],  // Ed25519 public key
        session_id:   u64,
        offer_addr:   SocketAddr,
    },

    /// Ping — keepalive
    Ping { node_id: NodeId },

    /// Pong — response to ping
    Pong { node_id: NodeId, rtt_ms: u32 },
}

impl DhtMessage {
    /// Serialize to bytes for transmission over UDP
    pub fn to_bytes(&self) -> Vec<u8> {
        // Production: use MessagePack or Protocol Buffers
        // Dev: simple tag + JSON-like encoding
        match self {
            Self::Ping { node_id } => {
                let mut v = vec![0x01];
                v.extend_from_slice(&node_id.0);
                v
            }
            Self::Pong { node_id, rtt_ms } => {
                let mut v = vec![0x02];
                v.extend_from_slice(&node_id.0);
                v.extend_from_slice(&rtt_ms.to_le_bytes());
                v
            }
            Self::Announce { node_id, addr, version } => {
                let mut v = vec![0x03, *version];
                v.extend_from_slice(&node_id.0);
                v.extend_from_slice(addr.to_string().as_bytes());
                v
            }
            Self::FindNode { requester, target, token } => {
                let mut v = vec![0x04];
                v.extend_from_slice(&requester.0);
                v.extend_from_slice(&target.0);
                v.extend_from_slice(&token.to_le_bytes());
                v
            }
            _ => vec![0xFF], // placeholder for other types
        }
    }

    /// Message type tag for routing
    pub fn tag(&self) -> u8 {
        match self {
            Self::Ping { .. }       => 0x01,
            Self::Pong { .. }       => 0x02,
            Self::Announce { .. }   => 0x03,
            Self::FindNode { .. }   => 0x04,
            Self::FoundNodes { .. } => 0x05,
            Self::CallRoute { .. }  => 0x06,
        }
    }
}
