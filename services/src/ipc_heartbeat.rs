// ipc_heartbeat.rs — Keepalive heartbeat mechanism
//
// Sends periodic Ping messages to detect dead connections early.
// If 3 heartbeats fail, the connection is considered dead and closed.
//
// Implements Google SRE best practice: "Ping early, fail fast"

use std::time::Duration;
use tokio::time::sleep;
use theos_core::ipc_protocol::IpcMessage;
use crate::ipc_robustness::{ConnectionState, config};

/// Heartbeat task — runs for the lifetime of a connection
/// Sends periodic Ping messages and monitors for timeouts
pub async fn run_heartbeat(
    conn_state: &std::sync::Arc<tokio::sync::Mutex<ConnectionState>>,
    sequence_counter: &std::sync::Arc<tokio::sync::Mutex<u32>>,
) {
    let mut interval = tokio::time::interval(config::HEARTBEAT_INTERVAL);

    loop {
        interval.tick().await;

        let mut state = conn_state.lock().await;

        // Check if connection is already dead
        if state.is_dead() {
            eprintln!(
                "[heartbeat] connection dead: {} failed heartbeats",
                state.failed_heartbeats
            );
            return;
        }

        // Check if idle (no activity in 5 minutes)
        if state.is_idle() {
            eprintln!("[heartbeat] connection idle for {:?}, closing", config::IDLE_TIMEOUT);
            return;
        }

        // Prepare next heartbeat
        let seq = {
            let mut counter = sequence_counter.lock().await;
            *counter += 1;
            *counter
        };

        state.record_heartbeat_sent();
        eprintln!("[heartbeat] ping #{}", seq);
    }
}

/// Generate a heartbeat Ping message
pub fn make_heartbeat_ping(sequence: u32) -> IpcMessage {
    IpcMessage::Ping { sequence }
}

/// Check if a message is a heartbeat response
pub fn is_heartbeat_pong(msg: &IpcMessage) -> bool {
    matches!(msg, IpcMessage::Pong { .. })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_make_heartbeat_ping() {
        let ping = make_heartbeat_ping(42);
        assert!(matches!(ping, IpcMessage::Ping { sequence: 42 }));
    }

    #[test]
    fn test_is_heartbeat_pong() {
        let pong = IpcMessage::Pong { sequence: 42 };
        assert!(is_heartbeat_pong(&pong));

        let ping = IpcMessage::Ping { sequence: 42 };
        assert!(!is_heartbeat_pong(&ping));
    }
}
