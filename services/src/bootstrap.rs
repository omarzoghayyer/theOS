// bootstrap.rs -- theOS DHT Bootstrap Server
//
// The first node a new device contacts when joining the network.
// Helps devices discover each other without any central directory.
//
// Design:
//   - UDP listener on port 7700 (configurable via THEOS_PORT env var)
//   - Maintains a routing table of known nodes (in-memory, no DB needed)
//   - Responds to Ping, Announce, FindNode messages
//   - Gossips known nodes back to requesters
//   - Periodic GC removes stale nodes (not seen in 15 minutes)
//   - No authentication -- nodes authenticate each other via Ed25519
//   - Stateless by design: if server restarts, nodes re-announce themselves
//
// Deployment: Railway.app (free tier, no credit card for prototype)
//   Set env var: THEOS_PORT=7700
//   The server address is hardcoded in theos-core as the bootstrap node.
//
// Security properties:
//   - Server never stores private keys or encrypted content
//   - Server only stores (NodeId, SocketAddr, timestamp) tuples
//   - A compromised bootstrap server can lie about node locations
//     but cannot decrypt communications (end-to-end encrypted via ChaCha20)
//   - Devices verify each other via Ed25519 -- bootstrap is untrusted
//   - This is the standard Kademlia threat model

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::UdpSocket;
use tokio::sync::RwLock;
use tokio::time::{interval, Duration};

// Message tags matching theos-core/src/dht/message.rs
const MSG_PING:      u8 = 0x01;
const MSG_PONG:      u8 = 0x02;
const MSG_ANNOUNCE:  u8 = 0x03;
const MSG_FIND_NODE: u8 = 0x04;
const MSG_FOUND_NODES: u8 = 0x05;

const NODE_ID_LEN: usize = 32;
const MAX_NODES:   usize = 10_000;
const NODE_TTL_SECS: u64 = 900; // 15 minutes
const K: usize = 20; // Kademlia K -- nodes returned per FindNode

/// A node entry in the bootstrap server's routing table
#[derive(Clone)]
struct NodeEntry {
    id:        [u8; 32],
    addr:      SocketAddr,
    last_seen: u64,
}

impl NodeEntry {
    fn is_alive(&self) -> bool {
        now_secs() - self.last_seen < NODE_TTL_SECS
    }
}

/// The bootstrap server state -- shared across all async tasks
struct BootstrapState {
    /// node_id_hex -> NodeEntry
    nodes: HashMap<String, NodeEntry>,
    /// Stats
    total_pings:     u64,
    total_announces: u64,
    total_lookups:   u64,
}

impl BootstrapState {
    fn new() -> Self {
        Self {
            nodes:           HashMap::new(),
            total_pings:     0,
            total_announces: 0,
            total_lookups:   0,
        }
    }

    fn upsert(&mut self, id: [u8; 32], addr: SocketAddr) {
        if self.nodes.len() >= MAX_NODES {
            // Evict oldest stale node to make room
            let stale_key = self.nodes.iter()
                .filter(|(_, v)| !v.is_alive())
                .map(|(k, v)| (k.clone(), v.last_seen))
                .min_by_key(|(_, ts)| *ts)
                .map(|(k, _)| k);
            if let Some(k) = stale_key {
                self.nodes.remove(&k);
            } else {
                return; // table full, all nodes live -- drop this one
            }
        }
        let key = hex(&id);
        self.nodes.insert(key, NodeEntry { id, addr, last_seen: now_secs() });
    }

    /// Find K closest nodes to a target ID using XOR distance
    fn find_closest(&self, target: &[u8; 32]) -> Vec<NodeEntry> {
        let mut live: Vec<&NodeEntry> = self.nodes.values()
            .filter(|n| n.is_alive())
            .collect();

        live.sort_by(|a, b| {
            let da = xor_dist(&a.id, target);
            let db = xor_dist(&b.id, target);
            da.cmp(&db)
        });

        live.into_iter().take(K).cloned().collect()
    }

    fn gc(&mut self) -> usize {
        let before = self.nodes.len();
        self.nodes.retain(|_, v| v.is_alive());
        before - self.nodes.len()
    }

    fn live_count(&self) -> usize {
        self.nodes.values().filter(|n| n.is_alive()).count()
    }
}

pub async fn run() {
    let port = std::env::var("THEOS_PORT")
        .ok()
        .and_then(|p| p.parse::<u16>().ok())
        .unwrap_or(7700);

    let bind_addr = format!("0.0.0.0:{}", port);
    let socket = UdpSocket::bind(&bind_addr).await
        .expect("failed to bind UDP socket");

    println!("[bootstrap] theOS DHT Bootstrap Server");
    println!("[bootstrap] listening on UDP {}", bind_addr);
    println!("[bootstrap] max nodes: {}  node TTL: {}s", MAX_NODES, NODE_TTL_SECS);
    println!("[bootstrap] K={} (nodes returned per FindNode)", K);
    println!("[bootstrap] ready -- waiting for nodes");

    let socket = Arc::new(socket);
    let state  = Arc::new(RwLock::new(BootstrapState::new()));

    // Spawn GC + stats task
    {
        let state = state.clone();
        tokio::spawn(async move {
            let mut tick = interval(Duration::from_secs(60));
            loop {
                tick.tick().await;
                let mut s = state.write().await;
                let removed = s.gc();
                println!(
                    "[bootstrap] gc: removed={} live={} pings={} announces={} lookups={}",
                    removed, s.live_count(),
                    s.total_pings, s.total_announces, s.total_lookups
                );
            }
        });
    }

    // Main UDP receive loop
    let mut buf = [0u8; 2048];
    loop {
        let (len, src) = match socket.recv_from(&mut buf).await {
            Ok(r)  => r,
            Err(e) => { eprintln!("[bootstrap] recv error: {}", e); continue; }
        };

        if len == 0 { continue; }

        let msg = &buf[..len];
        let tag = msg[0];

        match tag {
            MSG_PING => handle_ping(&socket, &state, msg, src).await,
            MSG_ANNOUNCE => handle_announce(&socket, &state, msg, src).await,
            MSG_FIND_NODE => handle_find_node(&socket, &state, msg, src).await,
            _ => {
                eprintln!("[bootstrap] unknown message tag {:#x} from {}", tag, src);
            }
        }
    }
}

/// Ping: [0x01][32 bytes node_id]
/// Response: Pong [0x02][32 bytes node_id]
async fn handle_ping(
    socket: &UdpSocket,
    state:  &RwLock<BootstrapState>,
    msg:    &[u8],
    src:    SocketAddr,
) {
    if msg.len() < 1 + NODE_ID_LEN { return; }

    let mut node_id = [0u8; 32];
    node_id.copy_from_slice(&msg[1..1 + NODE_ID_LEN]);

    {
        let mut s = state.write().await;
        s.upsert(node_id, src);
        s.total_pings += 1;
    }

    // Pong: [0x02][our_zero_id -- bootstrap has no identity]
    let mut pong = [0u8; 1 + NODE_ID_LEN];
    pong[0] = MSG_PONG;
    // Bootstrap server node ID = all zeros (it's not a real DHT participant)

    let _ = socket.send_to(&pong, src).await;
}

/// Announce: [0x03][version:1][node_id:32][addr_str...]
/// Response: Pong (ack)
async fn handle_announce(
    socket: &UdpSocket,
    state:  &RwLock<BootstrapState>,
    msg:    &[u8],
    src:    SocketAddr,
) {
    // Minimum: tag(1) + version(1) + node_id(32) = 34 bytes
    if msg.len() < 34 { return; }

    let mut node_id = [0u8; 32];
    node_id.copy_from_slice(&msg[2..34]);

    // Use the source address as the canonical address
    // (avoids NAT issues where the announced address is wrong)
    {
        let mut s = state.write().await;
        s.upsert(node_id, src);
        s.total_announces += 1;
        let live = s.live_count();
        println!("[bootstrap] announce from {} node={} live_nodes={}",
            src, &hex(&node_id)[..8], live);
    }

    // Ack with pong
    let mut pong = [0u8; 1 + NODE_ID_LEN];
    pong[0] = MSG_PONG;
    let _ = socket.send_to(&pong, src).await;
}

/// FindNode: [0x04][requester_id:32][target_id:32][token:4]
/// Response: FoundNodes [0x05][token:4][count:1][(id:32 + addr_str + \0)*]
async fn handle_find_node(
    socket: &UdpSocket,
    state:  &RwLock<BootstrapState>,
    msg:    &[u8],
    src:    SocketAddr,
) {
    // Minimum: tag(1) + requester(32) + target(32) + token(4) = 69 bytes
    if msg.len() < 69 { return; }

    let mut requester_id = [0u8; 32];
    let mut target_id    = [0u8; 32];
    requester_id.copy_from_slice(&msg[1..33]);
    target_id.copy_from_slice(&msg[33..65]);
    let token = &msg[65..69];

    // Update routing table with requester
    {
        let mut s = state.write().await;
        s.upsert(requester_id, src);
        s.total_lookups += 1;
    }

    let closest = {
        let s = state.read().await;
        s.find_closest(&target_id)
    };

    // Build response: [0x05][token:4][count:1][(node_id:32)(addr_bytes)(0x00)]*
    let mut resp = Vec::with_capacity(256);
    resp.push(MSG_FOUND_NODES);
    resp.extend_from_slice(token);
    resp.push(closest.len() as u8);

    for node in &closest {
        resp.extend_from_slice(&node.id);
        let addr_str = node.addr.to_string();
        resp.extend_from_slice(addr_str.as_bytes());
        resp.push(0x00); // null terminator for addr string
    }

    let _ = socket.send_to(&resp, src).await;
}

// -- Helpers ------------------------------------------------------------------

fn xor_dist(a: &[u8; 32], b: &[u8; 32]) -> [u8; 32] {
    let mut d = [0u8; 32];
    for i in 0..32 { d[i] = a[i] ^ b[i]; }
    d
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
