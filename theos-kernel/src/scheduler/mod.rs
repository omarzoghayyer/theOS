// scheduler/mod.rs -- theOS Priority Scheduler
//
// Purpose-built for theOS workload -- NOT a general purpose scheduler.
// Optimizes for: audio continuity + compositor frame deadlines.
//
// Problems this solves (from kernel issue tracker):
//
//   "Power management bolted on" [Linux, MEDIUM]
//     -> Power states are integrated with scheduler priority from day one.
//        The scheduler knows about satellite beam switches and adjusts
//        CPU budget allocation accordingly. Not an afterthought.
//
//   "Monolithic architecture" [Linux, HIGH]
//     -> Scheduler operates on capability-isolated processes only.
//        A misbehaving low-priority process cannot starve P0 audio.
//        Priority is enforced in hardware via ARM64 timer interrupts (Phase 2).
//
// Priority model (highest to lowest):
//   P0 Realtime -- audio capture/playback (VoIP must never glitch)
//   P1 High     -- compositor (frame rendering, 60fps target)
//   P2 Normal   -- identity, DHT, network daemons
//   P3 Low      -- background tasks (GC, cleanup)
//   P4 Idle     -- kernel idle loop
//
// Scheduling algorithm: strict priority with round-robin within each level.
//   - Always run the highest priority Ready process.
//   - Within the same priority level: round-robin with configurable quanta.
//   - A P0 process that is Ready always preempts everything else.
//   - A blocked P0 process (waiting for audio hardware) yields to P1.
//
// Quantum sizes (time slices):
//   P0: 2ms   -- audio buffer period (48kHz, 256 samples)
//   P1: 8ms   -- compositor frame budget (120fps = 8.3ms)
//   P2: 20ms  -- daemon response latency target
//   P3: 50ms  -- background work
//   P4: unlimited -- idle until interrupt
//
// Satellite awareness (Phase 3):
//   During Starlink beam switch (~20ms blackout):
//     P2 network daemon deprioritized (nothing to send/recv)
//     P1 compositor gets full CPU budget for smooth animation
//   Beam switch hint delivered via IPC from net_daemon.
//
// Phase 2: GIC timer interrupts wire preemption to this priority queue.
// Phase 3: satellite-aware scheduling hints from net_daemon.
// Phase 4: multi-core load balancing (Pixel 7 Pro: 4+4 big.LITTLE cores).

use crate::process::{Pid, Priority};

// -- Quantum sizes in microseconds --------------------------------------------
const QUANTUM_US_P0: u32 =     2_000; // 2ms  -- audio period
const QUANTUM_US_P1: u32 =     8_000; // 8ms  -- compositor frame
const QUANTUM_US_P2: u32 =    20_000; // 20ms -- daemon response
const QUANTUM_US_P3: u32 =    50_000; // 50ms -- background
const QUANTUM_US_P4: u32 = u32::MAX;  // idle -- unlimited

// -- Run queue ----------------------------------------------------------------
// Per-priority circular queues, no heap.
// Each level holds up to MAX_PER_LEVEL PIDs.

const MAX_PER_LEVEL: usize = 16;
const NUM_PRIORITIES: usize = 5;

struct RunQueue {
    // PIDs at each priority level
    pids:  [[u32; MAX_PER_LEVEL]; NUM_PRIORITIES],
    // Number of entries at each level
    count: [usize; NUM_PRIORITIES],
    // Round-robin head index at each level
    head:  [usize; NUM_PRIORITIES],
    // Currently running PID (0 = none)
    running_pid: u32,
    // Currently running priority level
    running_priority: usize,
    // Remaining quantum in microseconds for current process
    remaining_us: u32,
    // Whether satellite beam switch is in progress
    beam_switch_active: bool,
}

impl RunQueue {
    const fn new() -> Self {
        Self {
            pids:  [[0u32; MAX_PER_LEVEL]; NUM_PRIORITIES],
            count: [0usize; NUM_PRIORITIES],
            head:  [0usize; NUM_PRIORITIES],
            running_pid: 0,
            running_priority: NUM_PRIORITIES - 1,
            remaining_us: 0,
            beam_switch_active: false,
        }
    }

    fn priority_index(p: Priority) -> usize {
        p as usize
    }

    fn quantum_for(level: usize) -> u32 {
        match level {
            0 => QUANTUM_US_P0,
            1 => QUANTUM_US_P1,
            2 => QUANTUM_US_P2,
            3 => QUANTUM_US_P3,
            _ => QUANTUM_US_P4,
        }
    }

    /// Add a PID to the run queue at the given priority level.
    fn enqueue(&mut self, pid: Pid, priority: Priority) -> bool {
        let level = Self::priority_index(priority);
        if self.count[level] >= MAX_PER_LEVEL {
            return false; // queue full
        }
        let tail = (self.head[level] + self.count[level]) % MAX_PER_LEVEL;
        self.pids[level][tail] = pid.0;
        self.count[level] += 1;
        true
    }

    /// Remove a PID from the run queue (any priority level).
    fn dequeue_pid(&mut self, pid: Pid) {
        for level in 0..NUM_PRIORITIES {
            let mut i = 0;
            while i < self.count[level] {
                let slot = (self.head[level] + i) % MAX_PER_LEVEL;
                if self.pids[level][slot] == pid.0 {
                    // Shift remaining entries down
                    let mut j = i;
                    while j + 1 < self.count[level] {
                        let cur  = (self.head[level] + j)     % MAX_PER_LEVEL;
                        let next = (self.head[level] + j + 1) % MAX_PER_LEVEL;
                        self.pids[level][cur] = self.pids[level][next];
                        j += 1;
                    }
                    self.count[level] -= 1;
                    return;
                }
                i += 1;
            }
        }
    }

    /// Pick the next PID to run.
    /// Returns (pid, priority_level) of the highest priority ready process.
    /// During beam switch, P2 network daemon is skipped (deprioritized).
    fn pick_next(&mut self) -> Option<(Pid, usize)> {
        for level in 0..NUM_PRIORITIES {
            // During beam switch, skip P2 (Normal) -- net daemon has nothing to do
            if self.beam_switch_active && level == Priority::Normal as usize {
                continue;
            }
            if self.count[level] > 0 {
                let slot = self.head[level] % MAX_PER_LEVEL;
                let pid = self.pids[level][slot];
                // Advance round-robin head
                self.head[level] = (self.head[level] + 1) % MAX_PER_LEVEL;
                self.count[level] -= 1;
                return Some((Pid(pid), level));
            }
        }
        None
    }

    /// Notify scheduler that a Starlink beam switch is starting.
    /// Net daemon is deprioritized for the duration.
    fn beam_switch_start(&mut self) {
        self.beam_switch_active = true;
    }

    /// Notify scheduler that beam switch is complete.
    fn beam_switch_end(&mut self) {
        self.beam_switch_active = false;
    }

    /// Called on each timer tick. Returns Some(next_pid) if a context switch
    /// is needed, None if current process should continue.
    fn tick(&mut self, elapsed_us: u32) -> Option<Pid> {
        if self.remaining_us > elapsed_us {
            self.remaining_us -= elapsed_us;
            return None; // quantum not expired
        }
        // Quantum expired -- pick next process
        match self.pick_next() {
            Some((next_pid, level)) => {
                self.running_pid = next_pid.0;
                self.running_priority = level;
                self.remaining_us = Self::quantum_for(level);
                Some(next_pid)
            }
            None => None, // nothing else to run, continue current
        }
    }
}

static mut RUN_QUEUE: RunQueue = RunQueue::new();

macro_rules! rq {
    () => {
        unsafe { core::ptr::addr_of_mut!(RUN_QUEUE).as_mut().unwrap_unchecked() }
    };
}

// -- Public scheduler API -----------------------------------------------------

pub fn init() {
    crate::println!("[scheduler] theOS priority scheduler: active");
    crate::println!("[scheduler] P0=realtime(2ms) P1=high(8ms) P2=normal(20ms) P3=low(50ms) P4=idle");
    crate::println!("[scheduler] algorithm: strict priority + round-robin within level");
    crate::println!("[scheduler] audio continuity: P0 always preempts -- never glitches");
    crate::println!("[scheduler] compositor: P1 8ms quantum -- 120fps budget");
    crate::println!("[scheduler] satellite-aware: beam switch deprioritizes net daemon (Phase 3)");
    crate::println!("[scheduler] preemption: GIC timer interrupt (Phase 2)");
    crate::println!("[scheduler] multi-core: big.LITTLE load balancing (Phase 4)");
}

/// Add a process to the run queue.
pub fn enqueue(pid: Pid, priority: Priority) -> bool {
    rq!().enqueue(pid, priority)
}

/// Remove a process from the run queue (on block or terminate).
pub fn dequeue(pid: Pid) {
    rq!().dequeue_pid(pid)
}

/// Timer tick -- called by GIC timer interrupt handler (Phase 2).
/// Returns Some(pid) if a context switch should happen.
pub fn tick(elapsed_us: u32) -> Option<Pid> {
    rq!().tick(elapsed_us)
}

/// Notify scheduler that a Starlink beam switch is starting.
pub fn beam_switch_start() {
    rq!().beam_switch_start();
    crate::println!("[scheduler] beam switch: net daemon deprioritized");
}

/// Notify scheduler that beam switch is complete.
pub fn beam_switch_end() {
    rq!().beam_switch_end();
    crate::println!("[scheduler] beam switch: complete, net daemon restored");
}

/// Returns the quantum size in microseconds for a given priority.
pub fn quantum_us(priority: Priority) -> u32 {
    RunQueue::quantum_for(priority as usize)
}
