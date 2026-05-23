// NOTE: resolve_contact/query_bootstrap/resolver_mut are exercised by the
// integration tests and will be driven by the compositor IPC loop ("call
// sarah" -> resolve). Flagged unused in plain builds until that IPC wire
// lands; intentional, not dead.
#![allow(dead_code)]

// dht_client.rs -- Live DHT client for the daemon.
//
// This is the last wire between the ContactResolver (pure logic in
// theos-core) and the actual network. It owns:
//   - a ContactResolver (handle registry + Kademlia routing tables)
//   - a tokio UDP socket
//   - the bootstrap server address
//
// Resolution flow (resolve_contact):
//   1. Try local resolution (@handle -> key -> addr) -- instant if known.
//   2. On NoPeerAddress: send a FindNode to the bootstrap server over UDP,
//      parse the FoundNodes response, feed the nodes into routing.
//   3. Retry local resolution -- now succeeds if the bootstrap knew the peer.
//
// The wire format (build_find_node / parse_found_nodes) is shared with
// theos-core::dht::resolver and matches the bootstrap server byte-for-byte.
//
// Bootstrap address comes from THEOS_BOOTSTRAP env var, defaulting to the
// local dev server. On device this points at the deployed Railway address.

use std::net::SocketAddr;
use std::time::Duration;
use tokio::net::UdpSocket;
use tokio::time::timeout;

use theos_core::dht::resolver::{
    ContactResolver, ResolveError, build_find_node, parse_found_nodes,
};
use theos_core::dht::node::NodeId;

const QUERY_TIMEOUT_MS: u64 = 3000;

pub struct DhtClient {
    resolver:  ContactResolver,
    socket:    UdpSocket,
    bootstrap: String,
    my_node:   NodeId,
    token:     u32,
}

impl DhtClient {
    /// Bind a UDP socket and prepare the client for a device identity.
    pub async fn new(my_key: &[u8; 32]) -> std::io::Result<Self> {
        let socket = UdpSocket::bind("0.0.0.0:0").await?;
        let bootstrap = std::env::var("THEOS_BOOTSTRAP")
            .unwrap_or_else(|_| "127.0.0.1:7700".to_string());
        let my_node = NodeId::from_identity(my_key);

        println!("[dht] client bound on {}", socket.local_addr()?);
        println!("[dht] bootstrap server: {}", bootstrap);

        Ok(Self {
            resolver: ContactResolver::new(my_key),
            socket,
            bootstrap,
            my_node,
            token: 1,
        })
    }

    /// Access the resolver for registering handles / seeding routing.
    pub fn resolver_mut(&mut self) -> &mut ContactResolver {
        &mut self.resolver
    }

    /// Resolve a contact end to end, querying the bootstrap server on a miss.
    ///
    /// Returns the peer SocketAddr, or an error string for the caller to
    /// surface (e.g. to the call UI as "couldn't reach sarah").
    pub async fn resolve_contact(&mut self, handle: &str) -> Result<SocketAddr, String> {
        // 1. Fast path: local resolution.
        match self.resolver.resolve_contact(handle) {
            Ok(addr) => {
                println!("[dht] resolved {} locally -> {}", handle, addr);
                return Ok(addr);
            }
            Err(ResolveError::UnknownHandle) => {
                return Err(format!("unknown handle: {}", handle));
            }
            Err(ResolveError::NoPeerAddress) => {
                // Fall through to network query.
                println!("[dht] {} known but no route -- querying bootstrap", handle);
            }
        }

        // 2. We know the target NodeId (handle resolved to a key); ask bootstrap.
        let target = self.resolver.target_node_id(handle)
            .map_err(|_| format!("unknown handle: {}", handle))?;

        self.query_bootstrap(&target).await?;

        // 3. Retry local resolution now that routing may be populated.
        self.resolver.resolve_contact(handle)
            .map_err(|_| format!("{} not found on network", handle))
    }

    /// Send a FindNode to the bootstrap server and feed results into routing.
    async fn query_bootstrap(&mut self, target: &NodeId) -> Result<(), String> {
        let token = self.token;
        self.token = self.token.wrapping_add(1);

        let pkt = build_find_node(&self.my_node, target, token);

        self.socket.send_to(&pkt, &self.bootstrap).await
            .map_err(|e| format!("findnode send failed: {}", e))?;

        let mut buf = [0u8; 2048];
        let recv = timeout(
            Duration::from_millis(QUERY_TIMEOUT_MS),
            self.socket.recv_from(&mut buf),
        ).await;

        let (len, _src) = match recv {
            Ok(Ok((len, src))) => (len, src),
            Ok(Err(e)) => return Err(format!("findnode recv failed: {}", e)),
            Err(_)     => return Err("bootstrap query timed out".to_string()),
        };

        let (resp_token, nodes) = parse_found_nodes(&buf[..len])
            .ok_or_else(|| "malformed FoundNodes response".to_string())?;

        if resp_token != token {
            return Err(format!("token mismatch: sent {} got {}", token, resp_token));
        }

        println!("[dht] bootstrap returned {} node(s)", nodes.len());

        // Feed each returned node into the routing table.
        for n in nodes {
            if let Ok(addr) = n.addr.parse::<SocketAddr>() {
                let node_id = NodeId(n.id);
                self.resolver.dht.heard_from(node_id, addr);
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use theos_core::dht::handle::HandleAnnouncement;
    use theos_core::identity::keypair::KeyPair;

    // Full network path: register @sarah locally, announce Sarah's node to a
    // bootstrap server, then resolve @sarah end-to-end over UDP.
    #[tokio::test]
    async fn test_resolve_over_bootstrap() {
        // --- Start a bootstrap server on an ephemeral port ---
        let bs_sock = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let bs_addr = bs_sock.local_addr().unwrap();
        std::env::set_var("THEOS_BOOTSTRAP", bs_addr.to_string());

        // Sarah's identity + node + (pretend) peer address.
        let sarah_kp   = KeyPair::generate();
        let sarah_node = NodeId::from_identity(&sarah_kp.public.0);
        let sarah_addr = "100.64.0.5:7700";

        // Minimal bootstrap responder: answer one FindNode with Sarah's node.
        let sarah_node_bytes = sarah_node.0;
        tokio::spawn(async move {
            let mut buf = [0u8; 2048];
            if let Ok((len, src)) = bs_sock.recv_from(&mut buf).await {
                // Expect FindNode [0x04][req:32][tgt:32][token:4]
                if len >= 69 && buf[0] == 0x04 {
                    let token = &buf[65..69];
                    // Build FoundNodes [0x05][token:4][count:1][id:32][addr\0]
                    let mut resp = vec![0x05u8];
                    resp.extend_from_slice(token);
                    resp.push(1);
                    resp.extend_from_slice(&sarah_node_bytes);
                    resp.extend_from_slice(sarah_addr.as_bytes());
                    resp.push(0x00);
                    let _ = bs_sock.send_to(&resp, src).await;
                }
            }
        });

        // --- Client: register @sarah (hop 1 known), routing empty (hop 2 misses) ---
        let my_key = [0xAAu8; 32];
        let mut client = DhtClient::new(&my_key).await.unwrap();
        let ann = HandleAnnouncement::new("sarah", &sarah_kp).unwrap();
        client.resolver_mut().handles.register(ann).unwrap();

        // --- Resolve over the network ---
        let resolved = client.resolve_contact("@sarah").await;
        assert!(resolved.is_ok(), "resolve failed: {:?}", resolved);
        assert_eq!(resolved.unwrap().to_string(), sarah_addr);
    }

    #[tokio::test]
    async fn test_resolve_unknown_handle_fails_fast() {
        std::env::set_var("THEOS_BOOTSTRAP", "127.0.0.1:1"); // unused
        let mut client = DhtClient::new(&[0xBBu8; 32]).await.unwrap();
        let r = client.resolve_contact("@ghost").await;
        assert!(r.is_err());
        assert!(r.unwrap_err().contains("unknown handle"));
    }
}
