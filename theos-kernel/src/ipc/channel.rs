// ipc/channel.rs -- IPC Endpoint and Message Types
//
// KernelMessage: fixed-size, no heap, copyable.
// IpcEndpoint: single pending slot -- rendezvous semantics.
//
// Security properties:
//   - No message can be delivered without a matching receiver.
//   - No sender identity can be forged -- Pid is set by the kernel.
//   - Payload is opaque bytes -- the kernel does not interpret content.
//   - Double-send to a busy endpoint is rejected, not queued.

use crate::process::Pid;

/// Maximum inline payload size.
/// 128 bytes covers all theOS IPC messages when serialized compactly.
/// Larger transfers use shared memory via Mmio capability (Phase 3).
pub const MAX_PAYLOAD: usize = 128;

/// A kernel IPC message.
/// Fixed-size, no heap, safe to copy across context boundaries.
#[derive(Clone, Copy)]
pub struct KernelMessage {
    /// Message type tag (matches TAG_* constants in ipc/mod.rs)
    pub tag: u32,
    /// Valid bytes in payload (0..=MAX_PAYLOAD)
    pub len: u32,
    /// Inline payload -- upper layers parse the format
    pub payload: [u8; MAX_PAYLOAD],
}

impl KernelMessage {
    /// Create a new message with a tag and payload slice.
    /// Payload is truncated to MAX_PAYLOAD silently -- callers must
    /// ensure their payload fits. For larger data use Phase 3 shared memory.
    pub fn new(tag: u32, data: &[u8]) -> Self {
        let len = if data.len() > MAX_PAYLOAD { MAX_PAYLOAD } else { data.len() };
        let mut payload = [0u8; MAX_PAYLOAD];
        payload[..len].copy_from_slice(&data[..len]);
        Self { tag, len: len as u32, payload }
    }

    /// Create a tag-only message with no payload (e.g. HangupCall).
    pub fn tag_only(tag: u32) -> Self {
        Self { tag, len: 0, payload: [0u8; MAX_PAYLOAD] }
    }

    /// Return the valid payload slice.
    pub fn data(&self) -> &[u8] {
        &self.payload[..self.len as usize]
    }
}

/// IPC operation error codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IpcError {
    /// Caller does not hold the required capability.
    NotPermitted,
    /// Endpoint ID is out of range or does not exist.
    InvalidEndpoint,
    /// Endpoint already has a pending message (sender must wait).
    EndpointFull,
    /// No message is pending on this endpoint (receiver must wait).
    NoMessage,
    /// Endpoint already exists (create called twice for same ID).
    AlreadyExists,
}

/// State of a pending rendezvous.
#[derive(Clone, Copy, PartialEq, Eq)]
enum PendingState {
    /// No pending message.
    Empty,
    /// A sender is blocked waiting for a receiver.
    WaitingForReceiver,
    /// A receiver is blocked waiting for a sender.
    WaitingForSender,
}

/// A single IPC endpoint.
///
/// Rendezvous model:
///   - One pending slot: either a blocked sender or a blocked receiver, never both.
///   - When sender arrives and receiver is waiting: immediate transfer, both unblock.
///   - When receiver arrives and sender is waiting: immediate transfer, both unblock.
///   - When sender arrives and no receiver: message stored, sender blocks.
///   - When receiver arrives and no sender: receiver blocks (NoMessage returned).
#[derive(Clone, Copy)]
pub struct IpcEndpoint {
    /// Endpoint ID
    pub id: u32,
    /// Process that owns this endpoint (has IpcRecv rights by default)
    pub owner: Pid,
    /// Pending state
    state: PendingState,
    /// Pending message (valid when state == WaitingForReceiver)
    pending_msg: KernelMessage,
    /// PID of the blocked sender (valid when state == WaitingForReceiver)
    pending_sender: Pid,
}

impl IpcEndpoint {
    pub fn new(id: u32, owner: Pid) -> Self {
        Self {
            id,
            owner,
            state: PendingState::Empty,
            pending_msg: KernelMessage::tag_only(0),
            pending_sender: Pid(0),
        }
    }

    /// Attempt to send a message.
    ///
    /// Returns Ok(true)  -- receiver was waiting, immediate rendezvous.
    /// Returns Ok(false) -- message stored, sender should block.
    /// Returns Err(EndpointFull) -- another sender is already blocked here.
    pub fn send(&mut self, sender: Pid, msg: KernelMessage) -> Result<bool, IpcError> {
        match self.state {
            PendingState::WaitingForReceiver => {
                // Another sender is already blocked -- reject.
                // theOS has one sender per endpoint at a time by design.
                Err(IpcError::EndpointFull)
            }
            PendingState::WaitingForSender => {
                // Receiver is already waiting -- immediate rendezvous.
                // Store message for recv() to pick up, clear waiting state.
                self.pending_msg = msg;
                self.pending_sender = sender;
                self.state = PendingState::WaitingForReceiver;
                // Caller must unblock the receiver via process::unblock()
                Ok(true)
            }
            PendingState::Empty => {
                // No receiver yet -- store message and mark sender as blocked.
                self.pending_msg = msg;
                self.pending_sender = sender;
                self.state = PendingState::WaitingForReceiver;
                // Caller must block the sender via process::block()
                Ok(false)
            }
        }
    }

    /// Attempt to receive a message.
    ///
    /// Returns Ok((sender_pid, msg)) -- message received.
    /// Returns Err(NoMessage) -- no sender waiting, receiver should block.
    pub fn recv(&mut self, _receiver: Pid) -> Result<(Pid, KernelMessage), IpcError> {
        match self.state {
            PendingState::WaitingForReceiver => {
                // Sender is blocked and message is ready -- transfer now.
                let sender = self.pending_sender;
                let msg = self.pending_msg;
                self.state = PendingState::Empty;
                // Caller must unblock the sender via process::unblock()
                Ok((sender, msg))
            }
            PendingState::WaitingForSender => {
                // Already waiting for a sender -- shouldn't happen with
                // single-receiver model, but handle gracefully.
                Err(IpcError::NoMessage)
            }
            PendingState::Empty => {
                // No sender yet -- mark receiver as waiting.
                self.state = PendingState::WaitingForSender;
                Err(IpcError::NoMessage)
            }
        }
    }

    /// Whether this endpoint has a pending message ready for pickup.
    pub fn has_pending(&self) -> bool {
        self.state == PendingState::WaitingForReceiver
    }

    /// Whether a receiver is blocked waiting on this endpoint.
    pub fn has_waiting_receiver(&self) -> bool {
        self.state == PendingState::WaitingForSender
    }
}
