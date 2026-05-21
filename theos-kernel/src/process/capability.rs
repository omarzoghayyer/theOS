// process/capability.rs -- Unforgeable Capability Tokens
//
// A capability is the ONLY way a process can access any resource.
// No ambient authority exists in theOS -- a process cannot access
// anything it was not explicitly granted at creation time.
//
// This eliminates the privilege escalation attack class by construction:
// there is nothing to escalate to if it was never granted.
//
// Linux problem solved: CVE class "privilege escalation via ambient authority"
// is structurally impossible here. Not patched -- prevented.
//
// Capability types mirror the theOS resource model:
//   Memory    -- access to a physical memory region
//   Uart      -- write to the serial console (debug builds only)
//   Irq       -- receive interrupts from a hardware line
//   IpcSend   -- send messages to a specific IPC endpoint
//   IpcRecv   -- receive messages on a specific IPC endpoint
//   Mmio      -- access a specific MMIO register range (drivers only)
//
// Each capability carries the minimum information needed to exercise it.
// No capability grants more than one resource.
// Capabilities are not forgeable -- created only by the kernel.

/// A single capability token.
/// The kernel constructs these; no process can fabricate one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Capability {
    /// Access to a physical memory region [base, base+size)
    Memory { base: u64, size: u64 },

    /// Write access to the UART (kernel debug output only)
    Uart,

    /// Receive a specific hardware interrupt line
    Irq { line: u32 },

    /// Send messages to a specific IPC endpoint
    IpcSend { endpoint_id: u32 },

    /// Receive messages on a specific IPC endpoint
    IpcRecv { endpoint_id: u32 },

    /// Direct MMIO access to a register range (driver processes only)
    /// Granted only by the kernel at driver spawn time.
    Mmio { base: u64, size: u64 },
}

/// Maximum capabilities a single process can hold.
/// 32 is generous for theOS's constrained process model.
const MAX_CAPS: usize = 32;

/// The full set of capabilities held by one process.
/// Fixed-size, no heap required.
#[derive(Clone, Copy)]
pub struct CapabilitySet {
    caps: [Option<Capability>; MAX_CAPS],
    count: usize,
}

impl CapabilitySet {
    /// Create an empty capability set (no authority whatsoever).
    pub const fn empty() -> Self {
        Self {
            caps: [None; MAX_CAPS],
            count: 0,
        }
    }

    /// Grant a capability. Returns false if the set is full.
    /// Only the kernel calls this -- no process can grant itself capabilities.
    pub fn grant(&mut self, cap: Capability) -> bool {
        if self.count >= MAX_CAPS {
            return false;
        }
        // Deduplicate -- no point holding the same capability twice
        for i in 0..self.count {
            if self.caps[i] == Some(cap) {
                return true; // already held
            }
        }
        self.caps[self.count] = Some(cap);
        self.count += 1;
        true
    }

    /// Revoke a capability. Returns true if it was held.
    /// Used when a process is downgraded or terminated.
    pub fn revoke(&mut self, cap: &Capability) -> bool {
        for i in 0..self.count {
            if self.caps[i].as_ref() == Some(cap) {
                // Swap with last and shrink -- O(1), order irrelevant
                self.count -= 1;
                self.caps[i] = self.caps[self.count];
                self.caps[self.count] = None;
                return true;
            }
        }
        false
    }

    /// Check whether this set holds a specific capability.
    pub fn has(&self, cap: &Capability) -> bool {
        for i in 0..self.count {
            if self.caps[i].as_ref() == Some(cap) {
                return true;
            }
        }
        false
    }

    /// Check whether this set holds send rights to an endpoint.
    pub fn can_send(&self, endpoint_id: u32) -> bool {
        self.has(&Capability::IpcSend { endpoint_id })
    }

    /// Check whether this set holds receive rights on an endpoint.
    pub fn can_recv(&self, endpoint_id: u32) -> bool {
        self.has(&Capability::IpcRecv { endpoint_id })
    }

    /// Check whether this set holds MMIO access to an address.
    pub fn can_mmio(&self, addr: u64) -> bool {
        for i in 0..self.count {
            if let Some(Capability::Mmio { base, size }) = self.caps[i] {
                if addr >= base && addr < base + size {
                    return true;
                }
            }
        }
        false
    }

    /// Number of capabilities currently held.
    pub fn count(&self) -> usize {
        self.count
    }
}
