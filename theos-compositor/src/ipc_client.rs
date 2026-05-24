// ipc_client.rs -- Canonical IPC client for theOS Compositor
//
// Connects to the daemon via Unix socket and uses the canonical
// theos-core::ipc_protocol (with proper 4-byte length-prefix framing).
//
// This replaces the old hand-rolled IPC that was drifting.

use tokio::net::UnixStream;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::time;
use theos_core::ipc_protocol::{IpcMessage, IpcFrame};

const SOCKET_PATH: &str = "/tmp/theos-daemon.sock";

/// IPC client for communicating with the daemon.
/// Handles framing, serialization, and async I/O.
pub struct IpcClient {
    stream: UnixStream,
    read_buf: Vec<u8>,
}

impl IpcClient {
    /// Connect to the daemon via Unix socket.
    pub async fn connect() -> Result<Self, String> {
        // Retry logic: the daemon might not be ready yet
        for attempt in 0..10 {
            match UnixStream::connect(SOCKET_PATH).await {
                Ok(stream) => {
                    println!("[ipc_client] connected to daemon");
                    return Ok(Self {
                        stream,
                        read_buf: Vec::new(),
                    });
                }
                Err(e) if attempt < 9 => {
                    eprintln!("[ipc_client] connection attempt {} failed: {}, retrying...", attempt + 1, e);
                    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
                }
                Err(e) => return Err(format!("failed to connect after 10 attempts: {}", e)),
            }
        }
        Err("connection failed".to_string())
    }

    /// Send a message to the daemon (frames it and sends).
    pub async fn send(&mut self, msg: &IpcMessage) -> Result<(), String> {
        let frame = IpcFrame::encode(msg);
        self.stream.write_all(&frame).await.map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Receive a message from the daemon (reads framed message).
    pub async fn recv(&mut self) -> Result<IpcMessage, String> {
        let mut chunk = [0u8; 4096];
        loop {
            // Try to decode a complete frame from the buffer
            if let Some((msg, consumed)) = IpcFrame::decode(&self.read_buf) {
                self.read_buf.drain(..consumed);
                return Ok(msg);
            }

            // Need more data
            let n = self.stream.read(&mut chunk).await.map_err(|e| e.to_string())?;
            if n == 0 {
                return Err("daemon closed connection".to_string());
            }
            self.read_buf.extend_from_slice(&chunk[..n]);
        }
    }

    /// High-level: start a call to a contact.
    pub async fn start_call(&mut self, contact_key_hex: &str) -> Result<IpcMessage, String> {
        println!("[ipc_client] starting call to {}", contact_key_hex);
        self.send(&IpcMessage::StartCall {
            contact_key_hex: contact_key_hex.to_string(),
        }).await.map_err(|e| e.to_string())?;

        // Wait for the daemon's response (should be CallState)
        let reply = self.recv().await.map_err(|e| e.to_string())?;
        println!("[ipc_client] daemon replied: {:?}", reply);
        Ok(reply)
    }

    /// High-level: hang up the current call.
    pub async fn hangup(&mut self) -> Result<(), String> {
        println!("[ipc_client] hanging up");
        self.send(&IpcMessage::HangupCall).await.map_err(|e| e.to_string())?;
        Ok(())
    }

    /// High-level: ping the daemon (for connectivity check).
    pub async fn ping(&mut self, sequence: u32) -> Result<u32, String> {
        self.send(&IpcMessage::Ping { sequence }).await.map_err(|e| e.to_string())?;
        match self.recv().await.map_err(|e| e.to_string())? {
            IpcMessage::Pong { sequence: seq } => Ok(seq),
            _ => Err("expected Pong".into()),
        }
    }
}
