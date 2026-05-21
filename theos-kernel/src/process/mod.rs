// process/mod.rs -- Capability-Based Process Model
//
// Every process in theOS has a CapabilitySet.
// A capability is an unforgeable token granting access to exactly one resource.
// No process has ambient authority.
//
// Problems this solves (from kernel issue tracker):
//
//   "Privilege escalation -- ambient authority" [Linux+XNU, HIGH]
//     -> No process can escalate. There is nothing to escalate to.
//        A process with no IPC send capability cannot send IPC.
//        A process with no MMIO capability cannot touch hardware.
//        The kernel enforces this on every operation.
//
//   "Monolithic architecture -- driver bug = kernel crash" [Linux, HIGH]
//     -> Phase 4: each driver runs as an isolated process with only
//        the MMIO capability for its specific hardware range.
//        A driver crash cannot affect the kernel or other processes.
//        This module lays the foundation for that isolation.
//
// Process states:
//   Ready   -- runnable, waiting for CPU time
//   Running -- currently executing on a CPU core
//   Blocked -- waiting for an IPC message or event
//   Zombie  -- terminated, waiting for cleanup
//
// Process table: fixed-size array, no heap.
//   MAX_PROCESSES = 64. More than enough for theOS's workload:
//   compositor + audio + identity + DHT + network + a few daemons.

pub mod capability;
pub mod context;

use capability::{Capability, CapabilitySet};
use context::{CpuContext, context_switch};

/// Maximum simultaneous processes.
const MAX_PROCESSES: usize = 64;

/// Kernel stack size per process: 16KB (order-2 buddy allocation).
const KERNEL_STACK_ORDER: usize = 2;
const KERNEL_STACK_SIZE: u64 = (1 << KERNEL_STACK_ORDER) * crate::memory::buddy::PAGE_SIZE;

/// Process identifier. Never reused within a boot session.
/// Wrapping is not a concern -- u32 gives 4 billion process IDs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pid(pub u32);

/// Scheduler priority level.
/// Lower number = higher priority (P0 preempts everything).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Priority {
    Realtime = 0,  // P0: audio capture/playback -- must never glitch
    High     = 1,  // P1: compositor -- frame deadlines
    Normal   = 2,  // P2: identity, DHT, network daemons
    Low      = 3,  // P3: background work (GC, cleanup)
    Idle     = 4,  // P4: kernel idle loop
}

/// Process lifecycle state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProcessState {
    Ready,    // Runnable, waiting for CPU
    Running,  // Currently executing
    Blocked,  // Waiting for IPC or event
    Zombie,   // Terminated, pending cleanup
}

/// A kernel process descriptor.
/// All state for one process in the system.
pub struct Process {
    /// Unique process identifier
    pub pid: Pid,
    /// Human-readable name (debug/logging only, not security-relevant)
    pub name: &'static str,
    /// Current lifecycle state
    pub state: ProcessState,
    /// Scheduler priority
    pub priority: Priority,
    /// Saved CPU context (valid when state != Running)
    pub context: CpuContext,
    /// Capability set -- the only authority this process holds
    pub capabilities: CapabilitySet,
    /// Physical address of this process's kernel stack base
    pub stack_base: u64,
}

impl Process {
    /// Create a new process descriptor.
    /// Stack must be allocated by the caller from the buddy allocator.
    pub fn new(
        pid: Pid,
        name: &'static str,
        priority: Priority,
        entry: fn() -> !,
        stack_base: u64,
        capabilities: CapabilitySet,
    ) -> Self {
        let stack_top = stack_base + KERNEL_STACK_SIZE;
        Self {
            pid,
            name,
            state: ProcessState::Ready,
            priority,
            context: CpuContext::prepare(entry, stack_top),
            capabilities,
            stack_base,
        }
    }

    /// Grant an additional capability to this process.
    /// Returns false if the capability set is full (MAX_CAPS reached).
    pub fn grant(&mut self, cap: Capability) -> bool {
        self.capabilities.grant(cap)
    }

    /// Revoke a capability from this process.
    pub fn revoke(&mut self, cap: &Capability) -> bool {
        self.capabilities.revoke(cap)
    }

    /// Check whether this process can send to an IPC endpoint.
    pub fn can_send(&self, endpoint_id: u32) -> bool {
        self.capabilities.can_send(endpoint_id)
    }

    /// Check whether this process can receive on an IPC endpoint.
    pub fn can_recv(&self, endpoint_id: u32) -> bool {
        self.capabilities.can_recv(endpoint_id)
    }

    /// Check whether this process can access an MMIO address.
    pub fn can_mmio(&self, addr: u64) -> bool {
        self.capabilities.can_mmio(addr)
    }
}

/// The kernel process table.
/// Fixed-size, no heap. Processes are stored by slot index.
struct ProcessTable {
    slots: [Option<Process>; MAX_PROCESSES],
    count: usize,
    next_pid: u32,
    /// Index of the currently running process (None = kernel init thread)
    running: Option<usize>,
}

impl ProcessTable {
    const fn new() -> Self {
        // Option<Process> is not Copy so we can't use array repeat syntax.
        // This macro expands to 64 None values.
        Self {
            slots: [
                None, None, None, None, None, None, None, None,
                None, None, None, None, None, None, None, None,
                None, None, None, None, None, None, None, None,
                None, None, None, None, None, None, None, None,
                None, None, None, None, None, None, None, None,
                None, None, None, None, None, None, None, None,
                None, None, None, None, None, None, None, None,
                None, None, None, None, None, None, None, None,
            ],
            count: 0,
            next_pid: 1,
            running: None,
        }
    }

    /// Allocate a PID and find a free slot.
    /// Returns the slot index, or None if the table is full.
    fn allocate_slot(&mut self) -> Option<(usize, Pid)> {
        if self.count >= MAX_PROCESSES {
            return None;
        }
        for i in 0..MAX_PROCESSES {
            if self.slots[i].is_none() {
                let pid = Pid(self.next_pid);
                self.next_pid += 1;
                self.count += 1;
                return Some((i, pid));
            }
        }
        None
    }

    /// Spawn a new process.
    /// Allocates a kernel stack from the buddy allocator.
    /// Returns the PID of the new process, or None on OOM / table full.
    fn spawn(
        &mut self,
        name: &'static str,
        priority: Priority,
        entry: fn() -> !,
        capabilities: CapabilitySet,
    ) -> Option<Pid> {
        let (slot, pid) = self.allocate_slot()?;

        // Allocate kernel stack from physical memory
        let stack_base = crate::memory::alloc(KERNEL_STACK_ORDER)?;

        let process = Process::new(pid, name, priority, entry, stack_base, capabilities);
        self.slots[slot] = Some(process);

        crate::println!("[process] spawned '{}' pid={} priority={:?} stack={:#x}",
            name, pid.0, priority, stack_base);

        Some(pid)
    }

    /// Terminate a process and free its resources.
    fn terminate(&mut self, pid: Pid) {
        for i in 0..MAX_PROCESSES {
            if let Some(ref p) = self.slots[i] {
                if p.pid == pid {
                    let stack_base = p.stack_base;
                    self.slots[i] = None;
                    self.count -= 1;
                    // Return stack pages to the buddy allocator
                    crate::memory::free(stack_base, KERNEL_STACK_ORDER);
                    crate::println!("[process] terminated pid={}", pid.0);
                    return;
                }
            }
        }
    }

    /// Find a slot index by PID.
    fn find(&self, pid: Pid) -> Option<usize> {
        for i in 0..MAX_PROCESSES {
            if let Some(ref p) = self.slots[i] {
                if p.pid == pid { return Some(i); }
            }
        }
        None
    }

    /// Find the highest-priority Ready process.
    /// Used by the scheduler to pick the next thread to run.
    pub fn next_ready(&self) -> Option<usize> {
        let mut best: Option<(usize, Priority)> = None;
        for i in 0..MAX_PROCESSES {
            if let Some(ref p) = self.slots[i] {
                if p.state == ProcessState::Ready {
                    match best {
                        None => best = Some((i, p.priority)),
                        Some((_, ref bp)) if p.priority < *bp => {
                            best = Some((i, p.priority));
                        }
                        _ => {}
                    }
                }
            }
        }
        best.map(|(i, _)| i)
    }
}

/// Global process table -- all kernel process state lives here.
static mut PROCESS_TABLE: ProcessTable = ProcessTable::new();

macro_rules! table {
    () => {
        unsafe { core::ptr::addr_of_mut!(PROCESS_TABLE).as_mut().unwrap_unchecked() }
    };
}

// --- Public kernel API ---

/// Initialize the process subsystem.
/// Called once during boot from kernel_main().
pub fn init() {
    crate::println!("[process] capability-based process model: active");
    crate::println!("[process] max processes: {}", MAX_PROCESSES);
    crate::println!("[process] kernel stack per process: {}KB", KERNEL_STACK_SIZE / 1024);
    crate::println!("[process] privilege escalation: impossible by construction");
    crate::println!("[process] driver isolation: Phase 4 (per-process MMIO capabilities)");
}

/// Spawn a new kernel process.
/// `capabilities` defines the exact authority this process holds -- nothing more.
pub fn spawn(
    name: &'static str,
    priority: Priority,
    entry: fn() -> !,
    capabilities: CapabilitySet,
) -> Option<Pid> {
    table!().spawn(name, priority, entry, capabilities)
}

/// Terminate a process by PID and free all its resources.
pub fn terminate(pid: Pid) {
    table!().terminate(pid)
}

/// Find the next Ready process to run (highest priority).
/// Called by the scheduler.
pub fn next_ready() -> Option<Pid> {
    let t = table!();
    t.next_ready().and_then(|i| t.slots[i].as_ref().map(|p| p.pid))
}

/// Perform a context switch from `from_pid` to `to_pid`.
///
/// Saves the current process's CPU state and restores the next process's.
/// After this returns, we are executing in `to_pid`'s context.
///
/// # Safety
/// Both PIDs must be valid and present in the process table.
/// The caller (scheduler) is responsible for updating process states.
/// Interrupts must be disabled by the caller.
pub unsafe fn switch(from_pid: Pid, to_pid: Pid) {
    let t = table!();

    let from_idx = match t.find(from_pid) {
        Some(i) => i,
        None => {
            crate::println!("[process] switch: from pid={} not found", from_pid.0);
            return;
        }
    };
    let to_idx = match t.find(to_pid) {
        Some(i) => i,
        None => {
            crate::println!("[process] switch: to pid={} not found", to_pid.0);
            return;
        }
    };

    if from_idx == to_idx { return; } // No-op: switching to self

    // Update states
    if let Some(ref mut p) = t.slots[from_idx] {
        p.state = ProcessState::Ready;
    }
    if let Some(ref mut p) = t.slots[to_idx] {
        p.state = ProcessState::Running;
    }

    // Get raw pointers to both contexts -- must not hold references across the switch
    let from_ctx = match t.slots[from_idx] {
        Some(ref mut p) => &mut p.context as *mut CpuContext,
        None => return,
    };
    let to_ctx = match t.slots[to_idx] {
        Some(ref p) => &p.context as *const CpuContext,
        None => return,
    };

    t.running = Some(to_idx);

    // The actual CPU register save/restore
    unsafe { context_switch(from_ctx, to_ctx); }
}

/// Mark a process as Blocked (waiting for IPC or an event).
pub fn block(pid: Pid) {
    let t = table!();
    if let Some(i) = t.find(pid) {
        if let Some(ref mut p) = t.slots[i] {
            p.state = ProcessState::Blocked;
        }
    }
}

/// Mark a blocked process as Ready again.
pub fn unblock(pid: Pid) {
    let t = table!();
    if let Some(i) = t.find(pid) {
        if let Some(ref mut p) = t.slots[i] {
            if p.state == ProcessState::Blocked {
                p.state = ProcessState::Ready;
            }
        }
    }
}

/// Check whether a process holds a capability before granting resource access.
/// This is the central enforcement point -- every kernel resource access goes here.
pub fn check_capability(pid: Pid, cap: &Capability) -> bool {
    let t = table!();
    if let Some(i) = t.find(pid) {
        if let Some(ref p) = t.slots[i] {
            return p.capabilities.has(cap);
        }
    }
    false
}
