// ipc_listener.rs -- Daemon-side IPC endpoint for the compositor.
//
// The compositor connects over a Unix socket and speaks the canonical
// theos-core IpcMessage protocol with 4-byte length-prefix framing
// (IpcFrame). This is the seam where "call sarah" in the UI reaches the
// daemon's DHT resolver and VoIP engine.
//
// Why canonical protocol + framing:
//   - One IpcMessage type shared by compositor and daemon (no drift)
//   - Length-prefix framing handles partial reads / batched messages on a
//     stream correctly (newline-delimited + fixed buffers do not)
//   - The vocabulary already covers the full call lifecycle (StartCall,
//     CallState, LookupContact, ContactInfo), so call signaling and contact
//     lookup extend without a new protocol
//
// Flow for a call:
//   compositor -> StartCall { contact_key_hex }   (here key_hex is a handle
//                 or key; we resolve it to a peer address via DhtClient)
//   daemon     -> CallState { Calling }            (ack: looking up peer)
//   daemon     -> CallState { Failed } or proceeds to VoIP session setup
//
// For contact lookup (no call yet):
//   compositor -> LookupContact { key_hex }
//   daemon     -> ContactInfo { key_hex, handle, found }

use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::Mutex;

use theos_core::ipc_protocol::{IpcMessage, IpcFrame, CallState};

use crate::dht_client::DhtClient;

const SOCKET_PATH: &str = "/tmp/theos-daemon.sock";

/// Start the Unix-socket IPC listener. Runs for the life of the daemon.
/// The DhtClient is shared so resolution state (routing table) persists
/// across connections.
pub async fn serve(dht: Arc<Mutex<DhtClient>>) -> std::io::Result<()> {
    // Remove any stale socket from a previous run.
    let _ = std::fs::remove_file(SOCKET_PATH);

    let listener = UnixListener::bind(SOCKET_PATH)?;
    println!("[ipc] listening on {}", SOCKET_PATH);

    loop {
        match listener.accept().await {
            Ok((stream, _addr)) => {
                let dht = dht.clone();
                tokio::spawn(async move {
                    if let Err(e) = handle_conn(stream, dht).await {
                        println!("[ipc] connection closed: {}", e);
                    }
                });
            }
            Err(e) => {
                println!("[ipc] accept error: {}", e);
            }
        }
    }
}

/// Handle one compositor connection: read framed messages, dispatch, reply.
async fn handle_conn(
    mut stream: UnixStream,
    dht: Arc<Mutex<DhtClient>>,
) -> std::io::Result<()> {
    println!("[ipc] compositor connected");
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];

    loop {
        // Drain any complete frames already buffered.
        while let Some((msg, consumed)) = IpcFrame::decode(&buf) {
            buf.drain(..consumed);
            if let Some(reply) = dispatch(msg, &dht).await {
                let frame = IpcFrame::encode(&reply);
                stream.write_all(&frame).await?;
            }
        }

        // Read more bytes.
        let n = stream.read(&mut chunk).await?;
        if n == 0 {
            return Ok(()); // peer closed
        }
        buf.extend_from_slice(&chunk[..n]);
    }
}

/// Map an incoming compositor message to a daemon action + optional reply.
async fn dispatch(
    msg: IpcMessage,
    dht: &Arc<Mutex<DhtClient>>,
) -> Option<IpcMessage> {
    match msg {
        IpcMessage::StartCall { contact_key_hex } => {
            println!("[ipc] StartCall -> resolving '{}'", contact_key_hex);
            // Resolve the contact (handle or key) to a peer address.
            let mut client = dht.lock().await;
            match client.resolve_contact(&contact_key_hex).await {
                Ok(addr) => {
                    println!("[ipc] resolved '{}' -> {}", contact_key_hex, addr);
                    // Ack: we found the peer and are placing the call.
                    Some(IpcMessage::CallState {
                        state: "calling".to_string(),
                        contact_key_hex: Some(contact_key_hex),
                    })
                    // (VoIP session setup with `addr` happens next, once the
                    //  audio path is wired -- Phase 3 hardware.)
                }
                Err(e) => {
                    println!("[ipc] resolve failed: {}", e);
                    Some(IpcMessage::CallState {
                        state: "failed".to_string(),
                        contact_key_hex: Some(contact_key_hex),
                    })
                }
            }
        }

        IpcMessage::LookupContact { key_hex } => {
            println!("[ipc] LookupContact '{}'", key_hex);
            let mut client = dht.lock().await;
            match client.resolve_contact(&key_hex).await {
                Ok(_addr) => Some(IpcMessage::ContactInfo {
                    key_hex,
                    handle: None,
                    found: true,
                }),
                Err(_) => Some(IpcMessage::ContactInfo {
                    key_hex,
                    handle: None,
                    found: false,
                }),
            }
        }

        IpcMessage::HangupCall => {
            println!("[ipc] HangupCall");
            Some(IpcMessage::CallState {
                state: "ended".to_string(),
                contact_key_hex: None,
            })
        }

        IpcMessage::Ping { sequence } => {
            Some(IpcMessage::Pong { sequence })
        }

        other => {
            println!("[ipc] unhandled message: {:?}", other);
            None
        }
    }
}
