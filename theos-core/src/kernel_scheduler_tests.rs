// kernel_scheduler_tests.rs -- Host-side tests for kernel scheduler
//
// Mirrors the logic in theos-kernel/src/scheduler/mod.rs
// Runs on x86_64 -- validates priority queue, round-robin, quantum,
// beam switch deprioritization, and enqueue/dequeue correctness.

const MAX_PER_LEVEL: usize = 16;
const NUM_PRIORITIES: usize = 5;

const QUANTUM_US_P0: u32 =     2_000;
const QUANTUM_US_P1: u32 =     8_000;
const QUANTUM_US_P2: u32 =    20_000;
const QUANTUM_US_P3: u32 =    50_000;
const QUANTUM_US_P4: u32 = u32::MAX;

#[derive(Clone, Copy, PartialEq, Eq, Debug, PartialOrd, Ord)]
pub enum Priority {
    Realtime = 0,
    High     = 1,
    Normal   = 2,
    Low      = 3,
    Idle     = 4,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Pid(pub u32);

pub struct RunQueue {
    pids:               [[u32; MAX_PER_LEVEL]; NUM_PRIORITIES],
    count:              [usize; NUM_PRIORITIES],
    head:               [usize; NUM_PRIORITIES],
    pub running_pid:    u32,
    remaining_us:       u32,
    beam_switch_active: bool,
}

impl RunQueue {
    pub fn new() -> Self {
        Self {
            pids:               [[0u32; MAX_PER_LEVEL]; NUM_PRIORITIES],
            count:              [0; NUM_PRIORITIES],
            head:               [0; NUM_PRIORITIES],
            running_pid:        0,
            remaining_us:       0,
            beam_switch_active: false,
        }
    }

    pub fn quantum_for(level: usize) -> u32 {
        match level {
            0 => QUANTUM_US_P0,
            1 => QUANTUM_US_P1,
            2 => QUANTUM_US_P2,
            3 => QUANTUM_US_P3,
            _ => QUANTUM_US_P4,
        }
    }

    pub fn enqueue(&mut self, pid: Pid, priority: Priority) -> bool {
        let level = priority as usize;
        if self.count[level] >= MAX_PER_LEVEL { return false; }
        let tail = (self.head[level] + self.count[level]) % MAX_PER_LEVEL;
        self.pids[level][tail] = pid.0;
        self.count[level] += 1;
        true
    }

    pub fn dequeue_pid(&mut self, pid: Pid) {
        for level in 0..NUM_PRIORITIES {
            let mut i = 0;
            while i < self.count[level] {
                let slot = (self.head[level] + i) % MAX_PER_LEVEL;
                if self.pids[level][slot] == pid.0 {
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

    pub fn pick_next(&mut self) -> Option<(Pid, usize)> {
        for level in 0..NUM_PRIORITIES {
            if self.beam_switch_active && level == Priority::Normal as usize {
                continue;
            }
            if self.count[level] > 0 {
                let slot = self.head[level] % MAX_PER_LEVEL;
                let pid  = self.pids[level][slot];
                self.head[level]  = (self.head[level] + 1) % MAX_PER_LEVEL;
                self.count[level] -= 1;
                return Some((Pid(pid), level));
            }
        }
        None
    }

    pub fn tick(&mut self, elapsed_us: u32) -> Option<Pid> {
        if self.remaining_us > elapsed_us {
            self.remaining_us -= elapsed_us;
            return None;
        }
        match self.pick_next() {
            Some((pid, level)) => {
                self.running_pid  = pid.0;
                self.remaining_us = Self::quantum_for(level);
                Some(pid)
            }
            None => None,
        }
    }

    pub fn beam_switch_start(&mut self) { self.beam_switch_active = true; }
    pub fn beam_switch_end(&mut self)   { self.beam_switch_active = false; }
    pub fn total_queued(&self) -> usize { self.count.iter().sum() }
    pub fn queued_at(&self, p: Priority) -> usize { self.count[p as usize] }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rq() -> RunQueue { RunQueue::new() }

    // -- Quantum sizes --------------------------------------------------------

    #[test]
    fn test_quantum_p0_realtime() {
        assert_eq!(RunQueue::quantum_for(0), 2_000);
    }

    #[test]
    fn test_quantum_p1_high() {
        assert_eq!(RunQueue::quantum_for(1), 8_000);
    }

    #[test]
    fn test_quantum_p2_normal() {
        assert_eq!(RunQueue::quantum_for(2), 20_000);
    }

    #[test]
    fn test_quantum_p3_low() {
        assert_eq!(RunQueue::quantum_for(3), 50_000);
    }

    #[test]
    fn test_quantum_p4_idle_unlimited() {
        assert_eq!(RunQueue::quantum_for(4), u32::MAX);
    }

    #[test]
    fn test_p0_quantum_less_than_p1() {
        assert!(RunQueue::quantum_for(0) < RunQueue::quantum_for(1));
    }

    // -- Enqueue / dequeue ----------------------------------------------------

    #[test]
    fn test_enqueue_succeeds() {
        let mut rq = rq();
        assert!(rq.enqueue(Pid(1), Priority::High));
    }

    #[test]
    fn test_enqueue_increments_count() {
        let mut rq = rq();
        rq.enqueue(Pid(1), Priority::Normal);
        assert_eq!(rq.queued_at(Priority::Normal), 1);
    }

    #[test]
    fn test_enqueue_multiple_priorities() {
        let mut rq = rq();
        rq.enqueue(Pid(1), Priority::Realtime);
        rq.enqueue(Pid(2), Priority::High);
        rq.enqueue(Pid(3), Priority::Normal);
        assert_eq!(rq.total_queued(), 3);
    }

    #[test]
    fn test_enqueue_full_returns_false() {
        let mut rq = rq();
        for i in 0..MAX_PER_LEVEL {
            rq.enqueue(Pid(i as u32), Priority::Low);
        }
        assert!(!rq.enqueue(Pid(99), Priority::Low));
    }

    #[test]
    fn test_dequeue_removes_pid() {
        let mut rq = rq();
        rq.enqueue(Pid(1), Priority::Normal);
        rq.dequeue_pid(Pid(1));
        assert_eq!(rq.queued_at(Priority::Normal), 0);
    }

    #[test]
    fn test_dequeue_nonexistent_no_panic() {
        let mut rq = rq();
        rq.dequeue_pid(Pid(999)); // should not panic
    }

    #[test]
    fn test_dequeue_middle_pid() {
        let mut rq = rq();
        rq.enqueue(Pid(1), Priority::Normal);
        rq.enqueue(Pid(2), Priority::Normal);
        rq.enqueue(Pid(3), Priority::Normal);
        rq.dequeue_pid(Pid(2));
        assert_eq!(rq.queued_at(Priority::Normal), 2);
    }

    // -- Priority ordering ----------------------------------------------------

    #[test]
    fn test_p0_scheduled_before_p1() {
        let mut rq = rq();
        rq.enqueue(Pid(10), Priority::High);
        rq.enqueue(Pid(1),  Priority::Realtime);
        let (pid, level) = rq.pick_next().unwrap();
        assert_eq!(pid, Pid(1));
        assert_eq!(level, 0);
    }

    #[test]
    fn test_p1_scheduled_before_p2() {
        let mut rq = rq();
        rq.enqueue(Pid(20), Priority::Normal);
        rq.enqueue(Pid(10), Priority::High);
        let (pid, _) = rq.pick_next().unwrap();
        assert_eq!(pid, Pid(10));
    }

    #[test]
    fn test_p2_scheduled_before_p3() {
        let mut rq = rq();
        rq.enqueue(Pid(30), Priority::Low);
        rq.enqueue(Pid(20), Priority::Normal);
        let (pid, _) = rq.pick_next().unwrap();
        assert_eq!(pid, Pid(20));
    }

    #[test]
    fn test_p4_scheduled_last() {
        let mut rq = rq();
        rq.enqueue(Pid(40), Priority::Idle);
        rq.enqueue(Pid(30), Priority::Low);
        let (pid, _) = rq.pick_next().unwrap();
        assert_eq!(pid, Pid(30));
    }

    #[test]
    fn test_empty_queue_returns_none() {
        let mut rq = rq();
        assert!(rq.pick_next().is_none());
    }

    // -- Round-robin within priority level ------------------------------------

    #[test]
    fn test_round_robin_within_level() {
        let mut rq = rq();
        rq.enqueue(Pid(1), Priority::Normal);
        rq.enqueue(Pid(2), Priority::Normal);
        rq.enqueue(Pid(3), Priority::Normal);
        let (p1, _) = rq.pick_next().unwrap();
        let (p2, _) = rq.pick_next().unwrap();
        let (p3, _) = rq.pick_next().unwrap();
        // All three should be scheduled, in FIFO order
        let scheduled = vec![p1.0, p2.0, p3.0];
        assert!(scheduled.contains(&1));
        assert!(scheduled.contains(&2));
        assert!(scheduled.contains(&3));
    }

    #[test]
    fn test_fifo_order_within_level() {
        let mut rq = rq();
        rq.enqueue(Pid(1), Priority::High);
        rq.enqueue(Pid(2), Priority::High);
        let (first, _)  = rq.pick_next().unwrap();
        let (second, _) = rq.pick_next().unwrap();
        assert_eq!(first,  Pid(1));
        assert_eq!(second, Pid(2));
    }

    // -- Tick / quantum expiry ------------------------------------------------

    #[test]
    fn test_tick_no_switch_within_quantum() {
        let mut rq = rq();
        rq.enqueue(Pid(1), Priority::Normal);
        rq.tick(1000); // first tick picks a process
        let result = rq.tick(1000); // still within quantum
        assert!(result.is_none());
    }

    #[test]
    fn test_tick_switches_on_quantum_expiry() {
        let mut rq = rq();
        rq.enqueue(Pid(1), Priority::Normal);
        rq.enqueue(Pid(2), Priority::Normal);
        rq.tick(1000); // picks Pid(1), quantum = 20ms
        // Exhaust quantum
        let result = rq.tick(20_000);
        assert!(result.is_some());
    }

    #[test]
    fn test_tick_returns_none_when_empty() {
        let mut rq = rq();
        let result = rq.tick(1000);
        assert!(result.is_none());
    }

    #[test]
    fn test_tick_sets_running_pid() {
        let mut rq = rq();
        rq.enqueue(Pid(7), Priority::High);
        rq.tick(1000);
        assert_eq!(rq.running_pid, 7);
    }

    // -- Beam switch ----------------------------------------------------------

    #[test]
    fn test_beam_switch_skips_normal_priority() {
        let mut rq = rq();
        rq.enqueue(Pid(20), Priority::Normal);  // net daemon
        rq.enqueue(Pid(10), Priority::High);    // compositor
        rq.beam_switch_start();
        let (pid, _) = rq.pick_next().unwrap();
        // Normal (P2) skipped -- compositor (P1) scheduled instead
        assert_eq!(pid, Pid(10));
    }

    #[test]
    fn test_beam_switch_end_restores_normal() {
        let mut rq = rq();
        rq.enqueue(Pid(20), Priority::Normal);
        rq.beam_switch_start();
        rq.beam_switch_end();
        let (pid, _) = rq.pick_next().unwrap();
        assert_eq!(pid, Pid(20));
    }

    #[test]
    fn test_beam_switch_does_not_skip_realtime() {
        let mut rq = rq();
        rq.enqueue(Pid(1), Priority::Realtime);
        rq.beam_switch_start();
        let (pid, _) = rq.pick_next().unwrap();
        assert_eq!(pid, Pid(1)); // P0 always runs
    }

    #[test]
    fn test_beam_switch_does_not_skip_high() {
        let mut rq = rq();
        rq.enqueue(Pid(10), Priority::High);
        rq.beam_switch_start();
        let (pid, _) = rq.pick_next().unwrap();
        assert_eq!(pid, Pid(10)); // P1 compositor always runs
    }

    #[test]
    fn test_beam_switch_only_normal_p2_skipped() {
        let mut rq = rq();
        rq.enqueue(Pid(20), Priority::Normal); // skipped
        rq.enqueue(Pid(30), Priority::Low);    // not skipped
        rq.beam_switch_start();
        let (pid, _) = rq.pick_next().unwrap();
        assert_eq!(pid, Pid(30)); // P3 runs, P2 skipped
    }

    // -- Priority enum ordering -----------------------------------------------

    #[test]
    fn test_priority_ordering() {
        assert!(Priority::Realtime < Priority::High);
        assert!(Priority::High     < Priority::Normal);
        assert!(Priority::Normal   < Priority::Low);
        assert!(Priority::Low      < Priority::Idle);
    }

    #[test]
    fn test_priority_values() {
        assert_eq!(Priority::Realtime as usize, 0);
        assert_eq!(Priority::High     as usize, 1);
        assert_eq!(Priority::Normal   as usize, 2);
        assert_eq!(Priority::Low      as usize, 3);
        assert_eq!(Priority::Idle     as usize, 4);
    }
}
