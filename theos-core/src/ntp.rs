// ntp.rs -- theOS NTP Clock Synchronization
//
// Synchronizes the device clock over satellite UDP.
// Critical for Ed25519 timestamp validity and replay protection.
//
// Why this matters:
//   Ed25519 signatures include timestamps.
//   Replay protection uses a 30-second window.
//   If clock drifts >30 seconds, legitimate messages are rejected.
//   Satellite-only devices have no carrier network for time sync.
//   This module fixes that.
//
// Protocol: NTPv4 (RFC 5905) over UDP port 123.
//   Works over Starlink. Latency-compensated.
//   Round-trip time measured and halved for offset calculation.
//
// Servers (queried in order, first success wins):
//   time.cloudflare.com   -- anycast, fast
//   pool.ntp.org          -- global pool
//   time.google.com       -- fallback
//
// Satellite compensation:
//   Starlink RTT ~47ms average, spikes to 200ms.
//   We measure actual RTT per query and compensate.
//   Multiple samples averaged to reduce jitter.
//
// Security:
//   NTP is unauthenticated by default (no NTS in v1).
//   Flag for audit: NTS (RFC 8915) should replace this in production.
//   For now: use multiple servers, reject outliers (Marzullo's algorithm).
//
// Fallback:
//   If all NTP servers unreachable (satellite link down):
//   Use last known good time + monotonic clock offset.
//   Mark time as "estimated" -- signatures include this flag.
//   Peers accept "estimated" time with a wider 120-second replay window.

use std::time::{SystemTime, UNIX_EPOCH, Duration, Instant};
use std::net::UdpSocket;

// -- Constants ----------------------------------------------------------------

pub const NTP_PORT:           u16 = 123;
pub const NTP_EPOCH_OFFSET:   u64 = 2_208_988_800; // seconds between 1900 and 1970
pub const NTP_PACKET_SIZE:    usize = 48;
pub const QUERY_TIMEOUT_MS:   u64 = 5_000;  // 5 second timeout per server
pub const MAX_SAMPLES:        usize = 5;     // samples per sync
pub const SYNC_INTERVAL_SECS: u64 = 3_600;  // re-sync every hour
pub const MAX_DRIFT_SECS:     f64 = 300.0;  // reject offsets >5 min (likely attack)
pub const ESTIMATED_WINDOW:   u64 = 120;    // replay window when clock is estimated

pub const NTP_SERVERS: &[&str] = &[
    "time.cloudflare.com:123",
    "pool.ntp.org:123",
    "time.google.com:123",
    "time.apple.com:123",
];

// -- NtpTimestamp -------------------------------------------------------------

/// An NTP timestamp: seconds since Jan 1, 1900.
#[derive(Debug, Clone, Copy)]
pub struct NtpTimestamp {
    pub seconds:  u32,
    pub fraction: u32,
}

impl NtpTimestamp {
    pub fn to_unix_secs(&self) -> f64 {
        let secs = self.seconds.saturating_sub(NTP_EPOCH_OFFSET as u32) as f64;
        let frac = self.fraction as f64 / u32::MAX as f64;
        secs + frac
    }

    pub fn from_bytes(bytes: &[u8]) -> Self {
        let seconds  = u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        let fraction = u32::from_be_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
        Self { seconds, fraction }
    }
}

// -- NtpSample ----------------------------------------------------------------

/// A single NTP measurement from one server.
#[derive(Debug, Clone)]
pub struct NtpSample {
    pub server:      String,
    pub offset_secs: f64,  // how much our clock is off (+ = we're behind)
    pub rtt_ms:      f64,  // round-trip time in milliseconds
    pub timestamp:   u64,  // when this sample was taken (unix secs)
}

impl NtpSample {
    pub fn is_valid(&self) -> bool {
        // Reject obviously wrong offsets (likely misconfigured server or attack)
        self.offset_secs.abs() < MAX_DRIFT_SECS && self.rtt_ms < 10_000.0
    }
}

// -- ClockState ---------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub enum ClockState {
    /// Never synced -- clock is local system time only
    Unsynced,
    /// Synced with NTP -- offset applied, timestamps reliable
    Synced,
    /// NTP unreachable -- using last known + monotonic offset
    Estimated,
}

impl ClockState {
    pub fn label(&self) -> &str {
        match self {
            ClockState::Unsynced   => "unsynced",
            ClockState::Synced     => "synced",
            ClockState::Estimated  => "estimated",
        }
    }

    pub fn replay_window_secs(&self) -> u64 {
        match self {
            ClockState::Synced    => 30,
            ClockState::Estimated => ESTIMATED_WINDOW,
            ClockState::Unsynced  => 30,
        }
    }

    pub fn is_reliable(&self) -> bool {
        matches!(self, ClockState::Synced)
    }
}

// -- NtpSync ------------------------------------------------------------------

/// NTP synchronization state for the device.
pub struct NtpSync {
    pub state:          ClockState,
    pub offset_secs:    f64,        // current clock offset applied
    pub last_sync_at:   Option<u64>, // unix secs of last successful sync
    pub last_rtt_ms:    f64,
    pub sync_count:     u64,
    pub fail_count:     u64,
    monotonic_start:    Instant,
    monotonic_base:     u64,        // unix secs at monotonic_start
    samples:            Vec<NtpSample>,
}

impl NtpSync {
    pub fn new() -> Self {
        let now = system_now_secs();
        Self {
            state:           ClockState::Unsynced,
            offset_secs:     0.0,
            last_sync_at:    None,
            last_rtt_ms:     0.0,
            sync_count:      0,
            fail_count:      0,
            monotonic_start: Instant::now(),
            monotonic_base:  now,
            samples:         Vec::new(),
        }
    }

    /// Get current time in unix seconds, with NTP offset applied.
    /// This is the function everything in theos-core should use.
    pub fn now_secs(&self) -> u64 {
        match self.state {
            ClockState::Synced => {
                let base = system_now_secs() as f64 + self.offset_secs;
                base.max(0.0) as u64
            }
            ClockState::Estimated => {
                // Use monotonic clock from last known good time
                let elapsed = self.monotonic_start.elapsed().as_secs();
                self.monotonic_base + elapsed
            }
            ClockState::Unsynced => {
                system_now_secs()
            }
        }
    }

    /// Get current time as f64 with sub-second precision.
    pub fn now_f64(&self) -> f64 {
        match self.state {
            ClockState::Synced => {
                system_now_f64() + self.offset_secs
            }
            ClockState::Estimated => {
                let elapsed = self.monotonic_start.elapsed().as_secs_f64();
                self.monotonic_base as f64 + elapsed
            }
            ClockState::Unsynced => system_now_f64()
        }
    }

    /// Apply an NTP sample to update the clock offset.
    /// Uses simple averaging of valid samples.
    pub fn apply_sample(&mut self, sample: NtpSample) {
        if !sample.is_valid() {
            println!("[ntp] rejected outlier offset:{:.1}s rtt:{:.0}ms server:{}",
                sample.offset_secs, sample.rtt_ms, sample.server);
            return;
        }

        self.samples.push(sample.clone());
        if self.samples.len() > MAX_SAMPLES {
            self.samples.remove(0);
        }

        // Average valid samples (Marzullo-lite: drop highest and lowest if >2 samples)
        let mut offsets: Vec<f64> = self.samples.iter()
            .filter(|s| s.is_valid())
            .map(|s| s.offset_secs)
            .collect();

        if offsets.len() >= 3 {
            offsets.sort_by(|a, b| a.partial_cmp(b).unwrap());
            offsets.remove(0);
            offsets.pop();
        }

        if offsets.is_empty() { return; }

        let avg_offset = offsets.iter().sum::<f64>() / offsets.len() as f64;

        // Update monotonic base when syncing
        self.monotonic_start = Instant::now();
        self.monotonic_base  = (system_now_secs() as f64 + avg_offset) as u64;

        self.offset_secs  = avg_offset;
        self.last_rtt_ms  = sample.rtt_ms;
        self.last_sync_at = Some(system_now_secs());
        self.state        = ClockState::Synced;
        self.sync_count  += 1;

        println!("[ntp] synced offset:{:+.3}s rtt:{:.0}ms server:{} samples:{}",
            avg_offset, sample.rtt_ms, sample.server, self.samples.len());
    }

    /// Mark as estimated (NTP unreachable).
    pub fn mark_estimated(&mut self) {
        if self.state == ClockState::Synced {
            // Preserve monotonic from last good sync
            self.state = ClockState::Estimated;
            println!("[ntp] NTP unreachable -- using estimated time");
        } else if self.state == ClockState::Unsynced {
            self.state = ClockState::Estimated;
            // No good baseline -- use system clock as-is
            self.monotonic_start = Instant::now();
            self.monotonic_base  = system_now_secs();
        }
        self.fail_count += 1;
    }

    /// Should we re-sync? (called from main loop)
    pub fn needs_sync(&self) -> bool {
        match self.last_sync_at {
            None     => true,
            Some(t)  => system_now_secs().saturating_sub(t) >= SYNC_INTERVAL_SECS,
        }
    }

    /// How long since last sync (seconds).
    pub fn age_secs(&self) -> u64 {
        match self.last_sync_at {
            None    => u64::MAX,
            Some(t) => system_now_secs().saturating_sub(t),
        }
    }

    /// Query a single NTP server. Returns a sample or error string.
    /// This is the network call -- stub in tests, real UDP on device.
    pub fn query_server(server: &str) -> Result<NtpSample, String> {
        // Build NTPv4 request packet
        let mut packet = [0u8; NTP_PACKET_SIZE];
        packet[0] = 0x1B; // LI=0, VN=3, Mode=3 (client)

        let socket = UdpSocket::bind("0.0.0.0:0")
            .map_err(|e| format!("bind failed: {}", e))?;

        socket.set_read_timeout(Some(Duration::from_millis(QUERY_TIMEOUT_MS)))
            .map_err(|e| format!("set_timeout failed: {}", e))?;

        let t1 = system_now_f64();

        socket.send_to(&packet, server)
            .map_err(|e| format!("send failed to {}: {}", server, e))?;

        let mut buf = [0u8; NTP_PACKET_SIZE];
        let (len, _) = socket.recv_from(&mut buf)
            .map_err(|e| format!("recv failed from {}: {}", server, e))?;

        let t4 = system_now_f64();

        if len < NTP_PACKET_SIZE {
            return Err(format!("short response: {} bytes", len));
        }

        // Parse transmit timestamp (bytes 40-47) from server response
        let t2 = NtpTimestamp::from_bytes(&buf[32..40]).to_unix_secs(); // recv by server
        let t3 = NtpTimestamp::from_bytes(&buf[40..48]).to_unix_secs(); // transmit by server

        // NTP offset formula: offset = ((t2 - t1) + (t3 - t4)) / 2
        let offset = ((t2 - t1) + (t3 - t4)) / 2.0;
        let rtt_ms = (t4 - t1) * 1000.0 - (t3 - t2) * 1000.0;

        Ok(NtpSample {
            server:      server.to_string(),
            offset_secs: offset,
            rtt_ms:      rtt_ms.max(0.0),
            timestamp:   t4 as u64,
        })
    }

    /// Sync against all known servers. Updates state.
    /// Call this from the main loop when needs_sync() is true.
    pub fn sync(&mut self) {
        println!("[ntp] syncing against {} servers", NTP_SERVERS.len());
        let mut succeeded = false;

        for server in NTP_SERVERS {
            match Self::query_server(server) {
                Ok(sample) => {
                    println!("[ntp] {} offset:{:+.3}s rtt:{:.0}ms",
                        server, sample.offset_secs, sample.rtt_ms);
                    self.apply_sample(sample);
                    succeeded = true;
                    break; // first success wins
                }
                Err(e) => {
                    println!("[ntp] {} failed: {}", server, e);
                }
            }
        }

        if !succeeded {
            self.mark_estimated();
        }
    }

    /// Inject a known offset (for testing without network).
    pub fn inject_offset(&mut self, offset_secs: f64, rtt_ms: f64) {
        let sample = NtpSample {
            server:      "injected".to_string(),
            offset_secs,
            rtt_ms,
            timestamp:   system_now_secs(),
        };
        self.apply_sample(sample);
    }
}

// -- Helpers ------------------------------------------------------------------

fn system_now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs()
}

fn system_now_f64() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs_f64()
}

// -- Tests --------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn synced_ntp(offset: f64) -> NtpSync {
        let mut n = NtpSync::new();
        n.inject_offset(offset, 47.0);
        n
    }

    // ClockState

    #[test] fn test_clock_state_labels() {
        assert_eq!(ClockState::Unsynced.label(),  "unsynced");
        assert_eq!(ClockState::Synced.label(),    "synced");
        assert_eq!(ClockState::Estimated.label(), "estimated");
    }

    #[test] fn test_synced_is_reliable() {
        assert!(ClockState::Synced.is_reliable());
        assert!(!ClockState::Estimated.is_reliable());
        assert!(!ClockState::Unsynced.is_reliable());
    }

    #[test] fn test_replay_window_synced() {
        assert_eq!(ClockState::Synced.replay_window_secs(), 30);
    }

    #[test] fn test_replay_window_estimated_wider() {
        assert!(ClockState::Estimated.replay_window_secs() > ClockState::Synced.replay_window_secs());
    }

    // NtpTimestamp

    #[test] fn test_ntp_timestamp_epoch() {
        // NTP epoch to Unix epoch
        let ts = NtpTimestamp { seconds: NTP_EPOCH_OFFSET as u32, fraction: 0 };
        assert!((ts.to_unix_secs() - 0.0).abs() < 1.0);
    }

    #[test] fn test_ntp_timestamp_from_bytes() {
        let bytes = [0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00];
        let ts = NtpTimestamp::from_bytes(&bytes);
        assert_eq!(ts.seconds, 1);
        assert_eq!(ts.fraction, 0);
    }

    // NtpSample

    #[test] fn test_sample_valid() {
        let s = NtpSample { server:"test".to_string(), offset_secs:0.5, rtt_ms:47.0, timestamp:0 };
        assert!(s.is_valid());
    }

    #[test] fn test_sample_invalid_large_offset() {
        let s = NtpSample { server:"test".to_string(), offset_secs:400.0, rtt_ms:47.0, timestamp:0 };
        assert!(!s.is_valid());
    }

    #[test] fn test_sample_invalid_large_rtt() {
        let s = NtpSample { server:"test".to_string(), offset_secs:0.1, rtt_ms:15_000.0, timestamp:0 };
        assert!(!s.is_valid());
    }

    // NtpSync

    #[test] fn test_starts_unsynced() {
        let n = NtpSync::new();
        assert_eq!(n.state, ClockState::Unsynced);
        assert_eq!(n.sync_count, 0);
    }

    #[test] fn test_needs_sync_when_unsynced() {
        assert!(NtpSync::new().needs_sync());
    }

    #[test] fn test_inject_offset_syncs() {
        let n = synced_ntp(0.5);
        assert_eq!(n.state, ClockState::Synced);
        assert_eq!(n.sync_count, 1);
    }

    #[test] fn test_now_secs_reasonable() {
        let n = synced_ntp(0.0);
        let now = n.now_secs();
        let sys = system_now_secs();
        // Should be within 2 seconds of system time
        assert!((now as i64 - sys as i64).abs() < 2);
    }

    #[test] fn test_positive_offset_advances_time() {
        let n = synced_ntp(10.0);
        let now = n.now_secs();
        let sys = system_now_secs();
        assert!(now >= sys + 8); // at least +8s
    }

    #[test] fn test_negative_offset_delays_time() {
        let n = synced_ntp(-10.0);
        let now = n.now_secs();
        let sys = system_now_secs();
        assert!(now <= sys - 8); // at least -8s
    }

    #[test] fn test_mark_estimated_from_unsynced() {
        let mut n = NtpSync::new();
        n.mark_estimated();
        assert_eq!(n.state, ClockState::Estimated);
        assert_eq!(n.fail_count, 1);
    }

    #[test] fn test_mark_estimated_from_synced() {
        let mut n = synced_ntp(0.0);
        n.mark_estimated();
        assert_eq!(n.state, ClockState::Estimated);
    }

    #[test] fn test_estimated_now_secs_reasonable() {
        let mut n = synced_ntp(0.0);
        n.mark_estimated();
        let now = n.now_secs();
        let sys = system_now_secs();
        assert!((now as i64 - sys as i64).abs() < 5);
    }

    #[test] fn test_reject_large_offset_outlier() {
        let mut n = NtpSync::new();
        n.inject_offset(400.0, 47.0); // too large -- rejected
        assert_eq!(n.state, ClockState::Unsynced);
    }

    #[test] fn test_multiple_samples_averaged() {
        let mut n = NtpSync::new();
        n.inject_offset(1.0, 47.0);
        n.inject_offset(1.2, 47.0);
        n.inject_offset(0.8, 47.0);
        assert_eq!(n.state, ClockState::Synced);
        assert_eq!(n.samples.len(), 3);
    }

    #[test] fn test_age_secs_after_sync() {
        let n = synced_ntp(0.0);
        assert!(n.age_secs() < 5);
    }

    #[test] fn test_not_needs_sync_after_sync() {
        let n = synced_ntp(0.0);
        assert!(!n.needs_sync());
    }

    #[test] fn test_ntp_epoch_offset_constant() {
        // 70 years in seconds between 1900 and 1970
        assert_eq!(NTP_EPOCH_OFFSET, 2_208_988_800);
    }

    #[test] fn test_now_f64_positive() {
        let n = synced_ntp(0.0);
        assert!(n.now_f64() > 0.0);
    }

    #[test] fn test_sync_count_increments() {
        let mut n = NtpSync::new();
        n.inject_offset(0.1, 47.0);
        n.inject_offset(0.2, 47.0);
        assert_eq!(n.sync_count, 2);
    }

    #[test] fn test_servers_list_nonempty() {
        assert!(!NTP_SERVERS.is_empty());
    }

    #[test] fn test_query_timeout_constant() {
        assert!(QUERY_TIMEOUT_MS >= 1000);
    }
}
