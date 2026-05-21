// ipc/mod.rs -- Synchronous Rendezvous IPC
//
// seL4-inspired kernel IPC: sender blocks until receiver accepts.
// No kernel-managed message queues -- zero buffering, zero hidden state.
//
// Capability-gated: IpcSend to send, IpcRecv to receive.
// Every operation capability-checked -- no ambient IPC access.

pub mod channel;

use channel::{IpcError, KernelMessage};
use crate::process::Pid;
use crate::process::capability::Capability;

// -- Well-known endpoint IDs --------------------------------------------------
pub const EP_COMPOSITOR:   u32 = 0;
pub const EP_VOIP_DAEMON:  u32 = 1;
pub const EP_NET_DAEMON:   u32 = 2;
pub const EP_ID_DAEMON:    u32 = 3;
pub const EP_POWER_DAEMON: u32 = 4;

// -- Message tags -------------------------------------------------------------
pub const TAG_PING:                 u32 = 1;
pub const TAG_PONG:                 u32 = 2;
pub const TAG_START_CALL:           u32 = 10;
pub const TAG_HANGUP_CALL:          u32 = 11;
pub const TAG_MUTE_AUDIO:           u32 = 12;
pub const TAG_CALL_STATE:           u32 = 13;
pub const TAG_AUDIO_LEVEL:          u32 = 14;
pub const TAG_GET_NETWORK_STATUS:   u32 = 20;
pub const TAG_NETWORK_STATUS:       u32 = 21;
pub const TAG_IP_CHANGED:           u32 = 22;
pub const TAG_LOOKUP_CONTACT:       u32 = 30;
pub const TAG_CONTACT_INFO:         u32 = 31;
pub const TAG_GET_MY_IDENTITY:      u32 = 32;
pub const TAG_IDENTITY_INFO:        u32 = 33;
pub const TAG_BATTERY_UPDATE:       u32 = 40;
pub const TAG_ACQUIRE_WAKE_LOCK:    u32 = 41;
pub const TAG_RELEASE_WAKE_LOCK:    u32 = 42;
pub const TAG_SHOW_NOTIFICATION:    u32 = 50;
pub const TAG_DISMISS_NOTIFICATION: u32 = 51;
pub const TAG_ERROR:                u32 = 99;

// -- Endpoint table -----------------------------------------------------------
// Flat arrays instead of structs-of-arrays to avoid large static init.

const MAX_ENDPOINTS: usize = 32;

// Endpoint exists flag
static mut EP_ACTIVE: [bool; MAX_ENDPOINTS] = [false; MAX_ENDPOINTS];
// Endpoint owner PID
static mut EP_OWNER: [u32; MAX_ENDPOINTS] = [0u32; MAX_ENDPOINTS];
// Pending state: 0=empty 1=sender_waiting 2=receiver_waiting
static mut EP_STATE: [u8; MAX_ENDPOINTS] = [0u8; MAX_ENDPOINTS];
// Pending sender PID
static mut EP_SENDER: [u32; MAX_ENDPOINTS] = [0u32; MAX_ENDPOINTS];
// Pending message tag
static mut EP_TAG: [u32; MAX_ENDPOINTS] = [0u32; MAX_ENDPOINTS];
// Pending message length
static mut EP_LEN: [u32; MAX_ENDPOINTS] = [0u32; MAX_ENDPOINTS];
// Pending message payload - 32 endpoints x 128 bytes
static mut EP_PAYLOAD: [[u8; 128]; MAX_ENDPOINTS] = [[0u8; 128]; MAX_ENDPOINTS];

const STATE_EMPTY:           u8 = 0;
const STATE_SENDER_WAITING:  u8 = 1;
const STATE_RECEIVER_WAITING: u8 = 2;

// -- Public kernel IPC API ----------------------------------------------------

pub fn init() {
    crate::println!("[ipc] init entry");

    // Create well-known endpoints
    let kernel_pid = Pid(0);
    let endpoints: &[(u32, &str)] = &[
        (EP_COMPOSITOR,   "compositor"),
        (EP_VOIP_DAEMON,  "voip_daemon"),
        (EP_NET_DAEMON,   "net_daemon"),
        (EP_ID_DAEMON,    "id_daemon"),
        (EP_POWER_DAEMON, "power_daemon"),
    ];

    for &(id, name) in endpoints {
        create_endpoint(id, kernel_pid);
        crate::println!("[ipc] endpoint {}: {}", id, name);
    }

    crate::println!("[ipc] synchronous rendezvous IPC: active");
    crate::println!("[ipc] endpoints: {}/{}",  endpoints.len(), MAX_ENDPOINTS);
    crate::println!("[ipc] capability enforcement: active");
    crate::println!("[ipc] no message queues: rendezvous only");
}

pub fn create_endpoint(id: u32, owner: Pid) {
    if id as usize >= MAX_ENDPOINTS { return; }
    unsafe {
        EP_ACTIVE[id as usize] = true;
        EP_OWNER[id as usize] = owner.0;
        EP_STATE[id as usize] = STATE_EMPTY;
    }
}

pub fn assign_owner(endpoint_id: u32, new_owner: Pid) -> Result<(), IpcError> {
    let idx = endpoint_id as usize;
    if idx >= MAX_ENDPOINTS { return Err(IpcError::InvalidEndpoint); }
    unsafe {
        if !EP_ACTIVE[idx] { return Err(IpcError::InvalidEndpoint); }
        EP_OWNER[idx] = new_owner.0;
    }
    Ok(())
}

pub fn endpoint_exists(id: u32) -> bool {
    if id as usize >= MAX_ENDPOINTS { return false; }
    unsafe { EP_ACTIVE[id as usize] }
}

/// Send a message to an endpoint.
/// Returns Ok(true) = immediate rendezvous, Ok(false) = sender should block.
pub fn send(sender: Pid, endpoint_id: u32, msg: KernelMessage) -> Result<bool, IpcError> {
    if !crate::process::check_capability(sender, &Capability::IpcSend { endpoint_id }) {
        return Err(IpcError::NotPermitted);
    }
    let idx = endpoint_id as usize;
    if idx >= MAX_ENDPOINTS { return Err(IpcError::InvalidEndpoint); }
    unsafe {
        if !EP_ACTIVE[idx] { return Err(IpcError::InvalidEndpoint); }
        match EP_STATE[idx] {
            STATE_SENDER_WAITING => Err(IpcError::EndpointFull),
            STATE_RECEIVER_WAITING => {
                // Receiver waiting -- immediate rendezvous
                EP_TAG[idx] = msg.tag;
                EP_LEN[idx] = msg.len;
                EP_PAYLOAD[idx][..msg.len as usize]
                    .copy_from_slice(&msg.payload[..msg.len as usize]);
                EP_SENDER[idx] = sender.0;
                EP_STATE[idx] = STATE_SENDER_WAITING;
                Ok(true)
            }
            _ => {
                // No receiver yet -- store and block sender
                EP_TAG[idx] = msg.tag;
                EP_LEN[idx] = msg.len;
                EP_PAYLOAD[idx][..msg.len as usize]
                    .copy_from_slice(&msg.payload[..msg.len as usize]);
                EP_SENDER[idx] = sender.0;
                EP_STATE[idx] = STATE_SENDER_WAITING;
                Ok(false)
            }
        }
    }
}

/// Receive a message from an endpoint.
pub fn recv(receiver: Pid, endpoint_id: u32) -> Result<(Pid, KernelMessage), IpcError> {
    if !crate::process::check_capability(receiver, &Capability::IpcRecv { endpoint_id }) {
        return Err(IpcError::NotPermitted);
    }
    let idx = endpoint_id as usize;
    if idx >= MAX_ENDPOINTS { return Err(IpcError::InvalidEndpoint); }
    unsafe {
        if !EP_ACTIVE[idx] { return Err(IpcError::InvalidEndpoint); }
        match EP_STATE[idx] {
            STATE_SENDER_WAITING => {
                // Sender waiting -- transfer message
                let sender_pid = Pid(EP_SENDER[idx]);
                let mut payload = [0u8; 128];
                let len = EP_LEN[idx] as usize;
                payload[..len].copy_from_slice(&EP_PAYLOAD[idx][..len]);
                let msg = KernelMessage {
                    tag: EP_TAG[idx],
                    len: EP_LEN[idx],
                    payload,
                };
                EP_STATE[idx] = STATE_EMPTY;
                Ok((sender_pid, msg))
            }
            _ => {
                // No sender -- mark receiver waiting
                EP_STATE[idx] = STATE_RECEIVER_WAITING;
                Err(IpcError::NoMessage)
            }
        }
    }
}
