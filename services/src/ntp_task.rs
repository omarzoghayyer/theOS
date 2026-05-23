// ntp_task.rs -- NTP clock sync background task for the daemon.
//
// theOS has no carrier and no cellular time source, so the clock is synced
// over the network via NTP. Reliable time matters for security: the replay
// protection window in the VoIP/crypto layer tightens to 30s when the clock
// is Synced and widens when it's only Estimated. A drifting clock weakens
// replay protection, so we keep the clock fresh.
//
// Design:
//   - NtpSync lives in theos-core (std, real UDP NTPv4 client)
//   - sync() is blocking, so it runs inside tokio::task::spawn_blocking
//   - Shared state via Arc<Mutex<NtpSync>> so other daemon components
//     (VoIP replay window, message timestamps) can read reliable time
//   - Syncs once on startup, then re-checks every 60s and re-syncs when
//     NtpSync::needs_sync() reports the clock is stale
//
// On a real device this runs once the satellite link is up. In dev it
// syncs against the public NTP pool defined in theos-core.

use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::time::{interval, Duration};
use theos_core::ntp::NtpSync;

/// Shared, reliable clock for the whole daemon.
pub type SharedClock = Arc<Mutex<NtpSync>>;

/// Create a fresh shared clock (Unsynced until first sync completes).
pub fn new_clock() -> SharedClock {
    Arc::new(Mutex::new(NtpSync::new()))
}

/// Spawn the background NTP sync task.
/// Returns immediately; the task runs for the life of the daemon.
pub fn spawn(clock: SharedClock) {
    tokio::spawn(async move {
        // Initial sync on startup.
        run_sync(&clock).await;

        // Periodic re-sync loop.
        let mut tick = interval(Duration::from_secs(60));
        loop {
            tick.tick().await;
            let needs = {
                let c = clock.lock().await;
                c.needs_sync()
            };
            if needs {
                run_sync(&clock).await;
            }
        }
    });
}

/// Run one sync cycle. The blocking NTP I/O happens on a blocking thread
/// so the async runtime is never stalled.
async fn run_sync(clock: &SharedClock) {
    // Take the current state out, sync on a blocking thread, put it back.
    // NtpSync isn't Send-friendly to hold across .await while locked for a
    // blocking call, so we clone the minimal state via a fresh sync object:
    // simplest correct approach is to lock inside spawn_blocking via Arc.
    let clock = clock.clone();
    let result = tokio::task::spawn_blocking(move || {
        // We block here on a dedicated thread -- safe for synchronous UDP.
        let mut guard = clock.blocking_lock();
        guard.sync();
        // Copy primitives out before the guard drops (ClockState is Copy).
        let state  = guard.state;
        let offset = guard.offset_secs;
        let rtt    = guard.last_rtt_ms;
        let age    = guard.age_secs();
        (state, offset, rtt, age)
    })
    .await;

    match result {
        Ok((state, offset, rtt, age)) => {
            println!(
                "[ntp] clock: {} offset:{:+.3}s rtt:{:.0}ms age:{}s reliable:{}",
                state.label(), offset, rtt, age, state.is_reliable()
            );
        }
        Err(e) => {
            println!("[ntp] sync task join error: {}", e);
        }
    }
}
