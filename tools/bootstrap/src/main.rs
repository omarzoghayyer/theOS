// main.rs -- theOS DHT Bootstrap Server
//
// First contact point for two theOS devices finding each other.
// Listens on UDP, maintains active node list, responds to Kademlia messages.
//
// Deployment: Railway.app -- single binary, zero dependencies, zero database.
// All state in-memory. Nodes expire after 15 minutes of silence.
//
// Protocol (theOS DHT wire format):
//   0x01 Ping     -> 0x02 Pong
//   0x03 Announce -> stores node, returns closest peers
//   0x04 FindNode -> returns closest known nodes
//
// Security:
//   - Bootstrap is public by design (no auth needed)
//   - Rate limiting: 10 requests per IP per second
//   - Node table capped at 10,000 entries
//   - Only NodeId + SocketAddr + timestamp stored -- no message content

use std::collections::HashMap;
use std::net::{SocketAddr, UdpSocket};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const MAX_NODES:        usize = 10_000;
const NODE_TTL_SECS:    u64   = 900;
const MAX_RETURNED:     usize = 20;
const RATE_LIMIT_RPS:   u32   = 10;
const GC_INTERVAL_SECS: u64   = 60;
const NODE_ID_LEN:      usize = 32;

// -- NodeRecord ---------------------------------------------------------------

#[derive(Clone)]
struct NodeRecord {
    id:        [u8; 32],
    addr:      SocketAddr,
    last_seen: u64,
}

impl NodeRecord {
    fn is_alive(&self) -> bool {
        now_secs().saturating_sub(self.last_seen) < NODE_TTL_SECS
    }

    fn xor_distance(&self, target: &[u8; 32]) -> [u8; 32] {
        let mut d = [0u8; 32];
        for i in 0..32 { d[i] = self.id[i] ^ target[i]; }
        d
    }
}

// -- RateLimiter --------------------------------------------------------------

struct RateLimiter {
    buckets: HashMap<std::net::IpAddr, (u32, Instant)>,
}

impl RateLimiter {
    fn new() -> Self { Self { buckets: HashMap::new() } }

    fn allow(&mut self, ip: std::net::IpAddr) -> bool {
        let now = Instant::now();
        let entry = self.buckets.entry(ip).or_insert((0, now));
        if now.duration_since(entry.1) >= Duration::from_secs(1) {
            *entry = (1, now);
            true
        } else if entry.0 < RATE_LIMIT_RPS {
            entry.0 += 1;
            true
        } else {
            false
        }
    }

    fn gc(&mut self) {
        let now = Instant::now();
        self.buckets.retain(|_, (_, t)| now.duration_since(*t) < Duration::from_secs(10));
    }
}

// -- NodeTable ----------------------------------------------------------------

struct NodeTable {
    nodes: HashMap<[u8; 32], NodeRecord>,
}

impl NodeTable {
    fn new() -> Self { Self { nodes: HashMap::new() } }

    fn upsert(&mut self, id: [u8; 32], addr: SocketAddr) {
        if self.nodes.len() >= MAX_NODES {
            let oldest = self.nodes.values()
                .min_by_key(|n| n.last_seen)
                .map(|n| n.id);
            if let Some(k) = oldest { self.nodes.remove(&k); }
        }
        self.nodes.insert(id, NodeRecord { id, addr, last_seen: now_secs() });
    }

    fn closest(&self, target: &[u8; 32], exclude: &[u8; 32]) -> Vec<NodeRecord> {
        let mut alive: Vec<NodeRecord> = self.nodes.values()
            .filter(|n| n.is_alive() && &n.id != exclude)
            .cloned()
            .collect();
        alive.sort_by_key(|n| n.xor_distance(target));
        alive.into_iter().take(MAX_RETURNED).collect()
    }

    fn gc(&mut self) -> usize {
        let before = self.nodes.len();
        self.nodes.retain(|_, n| n.is_alive());
        before - self.nodes.len()
    }

    fn count(&self) -> usize { self.nodes.len() }
}

// -- Encoding -----------------------------------------------------------------

fn parse_node_id(buf: &[u8], offset: usize) -> Option<[u8; 32]> {
    if buf.len() < offset + NODE_ID_LEN { return None; }
    let mut id = [0u8; 32];
    id.copy_from_slice(&buf[offset..offset + NODE_ID_LEN]);
    Some(id)
}

fn encode_addr(addr: SocketAddr) -> Vec<u8> {
    match addr {
        SocketAddr::V4(a) => {
            let mut v = vec![0x04];
            v.extend_from_slice(&a.ip().octets());
            v.extend_from_slice(&a.port().to_le_bytes());
            v
        }
        SocketAddr::V6(a) => {
            let mut v = vec![0x06];
            v.extend_from_slice(&a.ip().octets());
            v.extend_from_slice(&a.port().to_le_bytes());
            v
        }
    }
}

fn encode_found_nodes(token: u32, nodes: &[NodeRecord]) -> Vec<u8> {
    let mut v = vec![0x05];
    v.extend_from_slice(&token.to_le_bytes());
    v.push(nodes.len() as u8);
    for n in nodes {
        v.extend_from_slice(&n.id);
        v.extend_from_slice(&encode_addr(n.addr));
    }
    v
}

fn encode_pong(server_id: &[u8; 32]) -> Vec<u8> {
    let mut v = vec![0x02];
    v.extend_from_slice(server_id);
    v.extend_from_slice(&0u32.to_le_bytes());
    v
}

// -- Stats --------------------------------------------------------------------

#[derive(Debug, Default)]
struct Stats {
    received:     u64,
    pings:        u64,
    announces:    u64,
    find_nodes:   u64,
    rate_limited: u64,
    unknown:      u64,
}

impl Stats {
    fn reset(&mut self) { *self = Self::default(); }
}

// -- Helpers ------------------------------------------------------------------

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs()
}

fn hex8(id: &[u8; 32]) -> String {
    id.iter().take(4).map(|b| format!("{:02x}", b)).collect()
}

fn maybe_gc(
    table:    &mut NodeTable,
    limiter:  &mut RateLimiter,
    last_gc:  &mut u64,
    stats:    &mut Stats,
) {
    let now = now_secs();
    if now - *last_gc >= GC_INTERVAL_SECS {
        let evicted = table.gc();
        limiter.gc();
        *last_gc = now;
        println!(
            "[bootstrap] gc evicted={} alive={} pings={} announces={} find_nodes={} rate_limited={}",
            evicted, table.count(),
            stats.pings, stats.announces, stats.find_nodes, stats.rate_limited
        );
        stats.reset();
    }
}

// -- Main ---------------------------------------------------------------------

fn main() {
    let port = std::env::var("PORT")
        .ok()
        .and_then(|p| p.parse::<u16>().ok())
        .unwrap_or(7680);

    let bind = format!("0.0.0.0:{}", port);
    let socket = UdpSocket::bind(&bind).unwrap_or_else(|e| {
        eprintln!("[bootstrap] bind failed on {}: {}", bind, e);
        std::process::exit(1);
    });

    socket.set_read_timeout(Some(Duration::from_secs(1))).ok();

    println!("[bootstrap] theOS DHT bootstrap server v0.1");
    println!("[bootstrap] listening on udp:{}", bind);
    println!("[bootstrap] max_nodes={} ttl={}s rate_limit={}/s", MAX_NODES, NODE_TTL_SECS, RATE_LIMIT_RPS);

    let server_id = [0xB0u8; 32]; // fixed bootstrap node ID
    let mut table   = NodeTable::new();
    let mut limiter = RateLimiter::new();
    let mut buf     = [0u8; 4096];
    let mut last_gc = now_secs();
    let mut stats   = Stats::default();

    loop {
        let (len, src) = match socket.recv_from(&mut buf) {
            Ok(r) => r,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock
                   || e.kind() == std::io::ErrorKind::TimedOut => {
                maybe_gc(&mut table, &mut limiter, &mut last_gc, &mut stats);
                continue;
            }
            Err(e) => { eprintln!("[bootstrap] recv: {}", e); continue; }
        };

        let msg = &buf[..len];
        if msg.is_empty() { continue; }

        stats.received += 1;

        if !limiter.allow(src.ip()) {
            stats.rate_limited += 1;
            continue;
        }

        match msg[0] {

            // Ping -> Pong + upsert
            0x01 => {
                if let Some(node_id) = parse_node_id(msg, 1) {
                    socket.send_to(&encode_pong(&server_id), src).ok();
                    table.upsert(node_id, src);
                    stats.pings += 1;
                    println!("[bootstrap] ping from {} id:{} nodes:{}", src, hex8(&node_id), table.count());
                }
            }

            // Announce -> upsert + return closest
            0x03 => {
                if let Some(node_id) = parse_node_id(msg, 2) {
                    table.upsert(node_id, src);
                    let closest = table.closest(&node_id, &node_id);
                    socket.send_to(&encode_found_nodes(0, &closest), src).ok();
                    stats.announces += 1;
                    println!("[bootstrap] announce from {} id:{} nodes:{} returned:{}",
                        src, hex8(&node_id), table.count(), closest.len());
                }
            }

            // FindNode -> return closest to target
            0x04 => {
                if msg.len() < 1 + NODE_ID_LEN * 2 + 4 { continue; }
                let requester = match parse_node_id(msg, 1) { Some(x) => x, None => continue };
                let target    = match parse_node_id(msg, 1 + NODE_ID_LEN) { Some(x) => x, None => continue };
                let token_bytes: [u8; 4] = msg[1 + NODE_ID_LEN*2..1 + NODE_ID_LEN*2 + 4]
                    .try_into().unwrap_or([0;4]);
                let token = u32::from_le_bytes(token_bytes);
                table.upsert(requester, src);
                let closest = table.closest(&target, &requester);
                socket.send_to(&encode_found_nodes(token, &closest), src).ok();
                stats.find_nodes += 1;
                println!("[bootstrap] find_node from {} target:{} returned:{}",
                    src, hex8(&target), closest.len());
            }

            _ => { stats.unknown += 1; }
        }

        maybe_gc(&mut table, &mut limiter, &mut last_gc, &mut stats);
    }
}
