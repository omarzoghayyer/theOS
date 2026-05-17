// ipc/mod.rs -- Inter-Process Communication
//
// Fast synchronous IPC between kernel processes.
// Message format mirrors IpcMessage from theos-core/src/ipc_protocol.rs
// ensuring the protocol is consistent from kernel to compositor.
//
// Design: synchronous rendezvous IPC (seL4-inspired)
//   Sender blocks until receiver accepts message.
//   No kernel-managed message queues -- simpler, faster, more secure.
//   Zero-copy for large payloads (capability-based shared memory).
//
// Phase 1 (current): stub
// Phase 2: IpcEndpoint, send/recv primitives
// Phase 3: capability passing in IPC messages
// Phase 4: zero-copy large message transfer

/// Initialize the IPC subsystem.
pub fn init() {
    crate::println!("[ipc] synchronous rendezvous IPC: stub (Phase 1)");
    crate::println!("[ipc] endpoint model: Phase 2");
    crate::println!("[ipc] capability passing: Phase 3");
}
