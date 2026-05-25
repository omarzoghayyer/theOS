// kernel_ipc_tests.rs -- Host-side tests for kernel IPC primitives

pub const MAX_PAYLOAD: usize = 128;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Pid(pub u32);

#[derive(Clone, Copy, Debug)]
pub struct KernelMessage {
    pub tag:     u32,
    pub len:     u32,
    pub payload: [u8; MAX_PAYLOAD],
}

impl PartialEq for KernelMessage {
    fn eq(&self, other: &Self) -> bool {
        self.tag == other.tag
            && self.len == other.len
            && self.payload[..self.len as usize] == other.payload[..other.len as usize]
    }
}

impl KernelMessage {
    pub fn new(tag: u32, data: &[u8]) -> Self {
        let len = data.len().min(MAX_PAYLOAD);
        let mut payload = [0u8; MAX_PAYLOAD];
        payload[..len].copy_from_slice(&data[..len]);
        Self { tag, len: len as u32, payload }
    }

    pub fn tag_only(tag: u32) -> Self {
        Self { tag, len: 0, payload: [0u8; MAX_PAYLOAD] }
    }

    pub fn data(&self) -> &[u8] {
        &self.payload[..self.len as usize]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IpcError {
    NotPermitted,
    InvalidEndpoint,
    EndpointFull,
    NoMessage,
    AlreadyExists,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PendingState { Empty, WaitingForReceiver, WaitingForSender }

pub struct IpcEndpoint {
    pub id:         u32,
    pub owner:      Pid,
    state:          PendingState,
    pending_msg:    KernelMessage,
    pending_sender: Pid,
}

impl IpcEndpoint {
    pub fn new(id: u32, owner: Pid) -> Self {
        Self {
            id, owner,
            state:          PendingState::Empty,
            pending_msg:    KernelMessage::tag_only(0),
            pending_sender: Pid(0),
        }
    }

    pub fn send(&mut self, sender: Pid, msg: KernelMessage) -> Result<bool, IpcError> {
        match self.state {
            PendingState::WaitingForReceiver => Err(IpcError::EndpointFull),
            PendingState::WaitingForSender => {
                self.pending_msg    = msg;
                self.pending_sender = sender;
                self.state = PendingState::WaitingForReceiver;
                Ok(true)
            }
            PendingState::Empty => {
                self.pending_msg    = msg;
                self.pending_sender = sender;
                self.state = PendingState::WaitingForReceiver;
                Ok(false)
            }
        }
    }

    pub fn recv(&mut self, _receiver: Pid) -> Result<(Pid, KernelMessage), IpcError> {
        match self.state {
            PendingState::WaitingForReceiver => {
                let sender = self.pending_sender;
                let msg    = self.pending_msg;
                self.state = PendingState::Empty;
                Ok((sender, msg))
            }
            PendingState::WaitingForSender => Err(IpcError::NoMessage),
            PendingState::Empty => {
                self.state = PendingState::WaitingForSender;
                Err(IpcError::NoMessage)
            }
        }
    }

    pub fn has_pending(&self)          -> bool { self.state == PendingState::WaitingForReceiver }
    pub fn has_waiting_receiver(&self) -> bool { self.state == PendingState::WaitingForSender }
    pub fn is_empty(&self)             -> bool { self.state == PendingState::Empty }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ep() -> IpcEndpoint { IpcEndpoint::new(0, Pid(1)) }
    fn msg(tag: u32) -> KernelMessage { KernelMessage::tag_only(tag) }
    fn msg_data(tag: u32, data: &[u8]) -> KernelMessage { KernelMessage::new(tag, data) }

    #[test]
    fn test_message_tag_only() {
        let m = msg(42);
        assert_eq!(m.tag, 42);
        assert_eq!(m.len, 0);
        assert_eq!(m.data(), &[] as &[u8]);
    }

    #[test]
    fn test_message_with_payload() {
        let m = msg_data(1, &[0xAA, 0xBB, 0xCC]);
        assert_eq!(m.tag, 1);
        assert_eq!(m.len, 3);
        assert_eq!(m.data(), &[0xAA, 0xBB, 0xCC]);
    }

    #[test]
    fn test_message_payload_truncated_to_max() {
        let big = vec![0x55u8; MAX_PAYLOAD + 100];
        let m = msg_data(1, &big);
        assert_eq!(m.len as usize, MAX_PAYLOAD);
    }

    #[test]
    fn test_message_empty_payload() {
        let m = msg_data(5, &[]);
        assert_eq!(m.len, 0);
        assert_eq!(m.data(), &[] as &[u8]);
    }

    #[test]
    fn test_message_full_payload() {
        let data: Vec<u8> = (0..MAX_PAYLOAD as u8).collect();
        let m = msg_data(7, &data);
        assert_eq!(m.len as usize, MAX_PAYLOAD);
        assert_eq!(m.data(), data.as_slice());
    }

    #[test]
    fn test_message_copy() {
        let m1 = msg_data(1, &[1, 2, 3]);
        let m2 = m1;
        assert_eq!(m2.tag, 1);
        assert_eq!(m2.data(), &[1, 2, 3]);
    }

    #[test]
    fn test_endpoint_starts_empty() {
        let ep = ep();
        assert!(ep.is_empty());
        assert!(!ep.has_pending());
        assert!(!ep.has_waiting_receiver());
    }

    #[test]
    fn test_endpoint_id_and_owner() {
        let ep = IpcEndpoint::new(5, Pid(99));
        assert_eq!(ep.id, 5);
        assert_eq!(ep.owner, Pid(99));
    }

    #[test]
    fn test_send_no_receiver_returns_false() {
        let mut ep = ep();
        assert_eq!(ep.send(Pid(2), msg(1)), Ok(false));
    }

    #[test]
    fn test_send_no_receiver_sets_pending() {
        let mut ep = ep();
        ep.send(Pid(2), msg(1)).unwrap();
        assert!(ep.has_pending());
        assert!(!ep.has_waiting_receiver());
    }

    #[test]
    fn test_double_send_returns_endpoint_full() {
        let mut ep = ep();
        ep.send(Pid(2), msg(1)).unwrap();
        assert_eq!(ep.send(Pid(3), msg(2)), Err(IpcError::EndpointFull));
    }

    #[test]
    fn test_recv_no_sender_returns_no_message() {
        let mut ep = ep();
        assert_eq!(ep.recv(Pid(1)), Err(IpcError::NoMessage));
    }

    #[test]
    fn test_recv_no_sender_sets_waiting_receiver() {
        let mut ep = ep();
        ep.recv(Pid(1)).unwrap_err();
        assert!(ep.has_waiting_receiver());
        assert!(!ep.has_pending());
    }

    #[test]
    fn test_double_recv_returns_no_message() {
        let mut ep = ep();
        ep.recv(Pid(1)).unwrap_err();
        assert_eq!(ep.recv(Pid(1)), Err(IpcError::NoMessage));
    }

    #[test]
    fn test_sender_first_receiver_gets_message() {
        let mut ep = ep();
        ep.send(Pid(2), msg_data(10, &[1, 2, 3])).unwrap();
        let (sender_pid, received) = ep.recv(Pid(1)).unwrap();
        assert_eq!(sender_pid, Pid(2));
        assert_eq!(received.tag, 10);
        assert_eq!(received.data(), &[1, 2, 3]);
    }

    #[test]
    fn test_sender_first_endpoint_empty_after_recv() {
        let mut ep = ep();
        ep.send(Pid(2), msg(1)).unwrap();
        ep.recv(Pid(1)).unwrap();
        assert!(ep.is_empty());
    }

    #[test]
    fn test_receiver_first_send_returns_true() {
        let mut ep = ep();
        ep.recv(Pid(1)).unwrap_err();
        assert_eq!(ep.send(Pid(2), msg(5)), Ok(true));
    }

    #[test]
    fn test_receiver_first_message_available_after_send() {
        let mut ep = ep();
        ep.recv(Pid(1)).unwrap_err();
        ep.send(Pid(2), msg_data(5, &[0xDE, 0xAD])).unwrap();
        assert!(ep.has_pending());
        let (sender, m) = ep.recv(Pid(1)).unwrap();
        assert_eq!(sender, Pid(2));
        assert_eq!(m.data(), &[0xDE, 0xAD]);
    }

    #[test]
    fn test_receiver_first_endpoint_empty_after_recv() {
        let mut ep = ep();
        ep.recv(Pid(1)).unwrap_err();
        ep.send(Pid(2), msg(1)).unwrap();
        ep.recv(Pid(1)).unwrap();
        assert!(ep.is_empty());
    }

    #[test]
    fn test_multiple_messages_in_sequence() {
        let mut ep = ep();
        for i in 0..10u32 {
            ep.send(Pid(2), msg(i)).unwrap();
            let (_, m) = ep.recv(Pid(1)).unwrap();
            assert_eq!(m.tag, i);
            assert!(ep.is_empty());
        }
    }

    #[test]
    fn test_sender_pid_preserved() {
        let mut ep = ep();
        ep.send(Pid(42), msg(1)).unwrap();
        let (sender, _) = ep.recv(Pid(1)).unwrap();
        assert_eq!(sender, Pid(42));
    }

    #[test]
    fn test_message_payload_preserved_through_rendezvous() {
        let mut ep = ep();
        let data: Vec<u8> = (0..64u8).collect();
        ep.send(Pid(2), msg_data(99, &data)).unwrap();
        let (_, m) = ep.recv(Pid(1)).unwrap();
        assert_eq!(m.tag, 99);
        assert_eq!(m.data(), data.as_slice());
    }

    #[test]
    fn test_reuse_after_rendezvous() {
        let mut ep = ep();
        ep.send(Pid(2), msg(1)).unwrap();
        ep.recv(Pid(1)).unwrap();
        assert!(ep.is_empty());
        ep.send(Pid(3), msg(2)).unwrap();
        let (s, m) = ep.recv(Pid(1)).unwrap();
        assert_eq!(s, Pid(3));
        assert_eq!(m.tag, 2);
    }

    #[test]
    fn test_state_empty_to_pending_on_send() {
        let mut ep = ep();
        assert!(ep.is_empty());
        ep.send(Pid(2), msg(1)).unwrap();
        assert!(ep.has_pending());
    }

    #[test]
    fn test_state_empty_to_waiting_on_recv() {
        let mut ep = ep();
        ep.recv(Pid(1)).unwrap_err();
        assert!(ep.has_waiting_receiver());
    }

    #[test]
    fn test_state_returns_to_empty_after_transfer() {
        let mut ep = ep();
        ep.send(Pid(2), msg(1)).unwrap();
        ep.recv(Pid(1)).unwrap();
        assert!(ep.is_empty());
    }
}
