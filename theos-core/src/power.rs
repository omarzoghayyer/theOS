// power.rs -- theOS Power Management
//
// Manages device power states for a voice-first satellite OS.
//
// Design constraints:
//   - "Hey OS" wake word must work at any battery level above 5%
//   - ADSP always-on mic uses ~5mW -- acceptable at all states
//   - 5% battery reserve (0.05) is never consumed by apps
//   - Deep sleep cuts all non-ADSP power to extend standby
//
// State machine:
//   Active <-> Dim <-> Sleep <-> DeepSleep
//   Any state -> ChargingActive (on charger connect)
//   Any state -> CriticalLow (battery < 0.05)
//
// Security assumptions (flag for audit):
//   - Battery level is read from sysfs -- not authenticated.
//     A compromised sysfs could fake battery level.
//   - Wake lock is advisory -- no kernel enforcement in this module.

use std::time::{Duration, Instant};

// -- Constants ----------------------------------------------------------------

/// Minimum battery reserve. Never consumed by apps.
/// Below this level, only emergency calls are permitted.
pub const BATTERY_RESERVE: f32 = 0.05;

/// Battery level below which device enters CriticalLow state.
pub const CRITICAL_BATTERY: f32 = 0.05;

/// Battery level below which device refuses non-emergency actions.
pub const LOW_BATTERY: f32 = 0.10;

/// Seconds of inactivity before screen dims.
pub const DIM_TIMEOUT_SECS: u64 = 30;

/// Seconds of inactivity before screen sleeps.
pub const SLEEP_TIMEOUT_SECS: u64 = 60;

/// Seconds of inactivity before deep sleep.
pub const DEEP_SLEEP_TIMEOUT_SECS: u64 = 300;

// -- PowerState ---------------------------------------------------------------

/// Device power state.
/// Transitions driven by user activity, timers, and battery level.
#[derive(Debug, Clone, PartialEq)]
pub enum PowerState {
    /// Screen on, fully interactive.
    Active,
    /// Screen dimmed, touch still responsive.
    Dim,
    /// Screen off, ADSP mic on, wake word listening.
    Sleep,
    /// All non-ADSP subsystems off. Maximum battery savings.
    /// Wake word still detected via ADSP interrupt.
    DeepSleep,
    /// Plugged in and charging. Screen stays on.
    ChargingActive,
    /// Battery below 5% reserve. Emergency mode only.
    CriticalLow,
}

impl PowerState {
    pub fn label(&self) -> &str {
        match self {
            PowerState::Active         => "active",
            PowerState::Dim            => "dim",
            PowerState::Sleep          => "sleep",
            PowerState::DeepSleep      => "deep_sleep",
            PowerState::ChargingActive => "charging_active",
            PowerState::CriticalLow    => "critical_low",
        }
    }

    pub fn is_screen_on(&self) -> bool {
        matches!(self, PowerState::Active | PowerState::Dim | PowerState::ChargingActive)
    }

    pub fn is_wake_word_active(&self) -> bool {
        // Wake word always active except CriticalLow
        !matches!(self, PowerState::CriticalLow)
    }

    pub fn allows_satellite_tx(&self) -> bool {
        // No satellite TX in deep sleep -- radio is off
        !matches!(self, PowerState::DeepSleep | PowerState::CriticalLow)
    }

    pub fn allows_app_actions(&self) -> bool {
        matches!(self, PowerState::Active | PowerState::ChargingActive)
    }
}

// -- BatteryStatus ------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct BatteryStatus {
    /// 0.0 to 1.0
    pub level:      f32,
    pub charging:   bool,
    pub temperature_c: f32,
}

impl BatteryStatus {
    pub fn new(level: f32, charging: bool) -> Self {
        Self { level: level.clamp(0.0, 1.0), charging, temperature_c: 25.0 }
    }

    pub fn full() -> Self      { Self::new(1.0, false) }
    pub fn charging() -> Self  { Self::new(0.5, true) }
    pub fn critical() -> Self  { Self::new(0.04, false) }
    pub fn low() -> Self       { Self::new(0.09, false) }

    pub fn is_critical(&self) -> bool { self.level < CRITICAL_BATTERY }
    pub fn is_low(&self)      -> bool { self.level < LOW_BATTERY }
    pub fn above_reserve(&self) -> bool { self.level >= BATTERY_RESERVE }

    pub fn percent(&self) -> u8 { (self.level * 100.0) as u8 }

    pub fn is_overheating(&self) -> bool { self.temperature_c > 45.0 }
}

// -- WakeLock -----------------------------------------------------------------

/// Prevent the device from sleeping while held.
/// Dropped automatically when it goes out of scope.
#[derive(Debug)]
pub struct WakeLock {
    pub reason: String,
    acquired_at: Instant,
}

impl WakeLock {
    pub fn new(reason: &str) -> Self {
        Self { reason: reason.to_string(), acquired_at: Instant::now() }
    }

    pub fn held_for(&self) -> Duration {
        self.acquired_at.elapsed()
    }
}

// -- PowerManager -------------------------------------------------------------

/// Manages device power state transitions.
/// Called by the compositor frame loop and system event handlers.
pub struct PowerManager {
    pub state:        PowerState,
    pub battery:      BatteryStatus,
    last_activity:    Instant,
    wake_locks:       Vec<WakeLock>,
    pub sleep_count:  u64,
    pub wake_count:   u64,
}

impl PowerManager {
    pub fn new() -> Self {
        println!("[power] initialized -- state:active battery:100%");
        Self {
            state:         PowerState::Active,
            battery:       BatteryStatus::full(),
            last_activity: Instant::now(),
            wake_locks:    Vec::new(),
            sleep_count:   0,
            wake_count:    0,
        }
    }

    /// Record user activity (touch, voice, key press).
    /// Resets inactivity timer and wakes screen if sleeping.
    pub fn on_activity(&mut self) {
        self.last_activity = Instant::now();
        match self.state {
            PowerState::Sleep | PowerState::DeepSleep => {
                self.transition_to(PowerState::Active);
                self.wake_count += 1;
            }
            PowerState::Dim => {
                self.transition_to(PowerState::Active);
            }
            _ => {}
        }
    }

    /// Called when wake word "Hey OS" is detected by ADSP.
    /// Wakes screen from any sleep state.
    pub fn on_wake_word(&mut self) {
        println!("[power] wake word detected -- waking");
        self.wake_count += 1;
        match &self.state {
            PowerState::Sleep | PowerState::DeepSleep | PowerState::Dim => {
                self.transition_to(PowerState::Active);
            }
            PowerState::CriticalLow => {
                // Wake word detected but battery critical --
                // allow emergency interface only
                println!("[power] wake word in critical low -- emergency mode only");
            }
            _ => {}
        }
        self.last_activity = Instant::now();
    }

    /// Update battery status. Called periodically from sysfs reader.
    pub fn on_battery_update(&mut self, status: BatteryStatus) {
        let was_charging = self.battery.charging;
        self.battery = status;

        if self.battery.is_critical() {
            if self.state != PowerState::CriticalLow {
                println!("[power] CRITICAL: battery {}% -- emergency mode", self.battery.percent());
                self.transition_to(PowerState::CriticalLow);
            }
        } else if self.battery.charging && !was_charging {
            println!("[power] charger connected -- {}%", self.battery.percent());
            self.transition_to(PowerState::ChargingActive);
        } else if !self.battery.charging && was_charging {
            println!("[power] charger disconnected -- {}%", self.battery.percent());
            self.transition_to(PowerState::Active);
            self.last_activity = Instant::now();
        }
    }

    /// Tick called every frame. Drives inactivity timeouts.
    /// Returns true if state changed (compositor should re-render).
    pub fn tick(&mut self) -> bool {
        // Don't apply timeouts in these states
        match self.state {
            PowerState::ChargingActive |
            PowerState::CriticalLow    |
            PowerState::DeepSleep      => return false,
            _ => {}
        }

        // Wake locks prevent sleep
        if !self.wake_locks.is_empty() { return false; }

        let idle = self.last_activity.elapsed().as_secs();
        let prev = self.state.clone();

        if idle >= DEEP_SLEEP_TIMEOUT_SECS && self.state == PowerState::Sleep {
            self.transition_to(PowerState::DeepSleep);
            self.sleep_count += 1;
        } else if idle >= SLEEP_TIMEOUT_SECS && self.state == PowerState::Dim {
            self.transition_to(PowerState::Sleep);
            self.sleep_count += 1;
        } else if idle >= DIM_TIMEOUT_SECS && self.state == PowerState::Active {
            self.transition_to(PowerState::Dim);
        }

        self.state != prev
    }

    /// Acquire a wake lock. Device won't sleep while any wake lock is held.
    pub fn acquire_wake_lock(&mut self, reason: &str) -> usize {
        let lock = WakeLock::new(reason);
        self.wake_locks.push(lock);
        println!("[power] wake lock acquired: {} (total: {})", reason, self.wake_locks.len());
        self.wake_locks.len() - 1
    }

    /// Release a wake lock by index.
    pub fn release_wake_lock(&mut self, index: usize) {
        if index < self.wake_locks.len() {
            let lock = self.wake_locks.remove(index);
            println!("[power] wake lock released: {} held for {:?}", lock.reason, lock.held_for());
        }
    }

    pub fn wake_lock_count(&self) -> usize { self.wake_locks.len() }

    /// Force immediate sleep (e.g. power button press).
    pub fn force_sleep(&mut self) {
        println!("[power] force sleep");
        self.sleep_count += 1;
        self.transition_to(PowerState::Sleep);
    }

    /// Force immediate deep sleep.
    pub fn force_deep_sleep(&mut self) {
        println!("[power] force deep sleep");
        self.sleep_count += 1;
        self.transition_to(PowerState::DeepSleep);
    }

    /// Can the device transmit over satellite right now?
    pub fn can_transmit(&self) -> bool {
        self.state.allows_satellite_tx() && self.battery.above_reserve()
    }

    /// Can the device run app actions right now?
    pub fn can_run_apps(&self) -> bool {
        self.state.allows_app_actions() && !self.battery.is_critical()
    }

    fn transition_to(&mut self, new_state: PowerState) {
        if self.state != new_state {
            println!("[power] {} -> {}", self.state.label(), new_state.label());
            self.state = new_state;
        }
    }
}

impl Default for PowerManager {
    fn default() -> Self { Self::new() }
}

// -- Tests --------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn pm() -> PowerManager { PowerManager::new() }

    // PowerState

    #[test]
    fn test_active_screen_on() { assert!(PowerState::Active.is_screen_on()); }
    #[test]
    fn test_dim_screen_on() { assert!(PowerState::Dim.is_screen_on()); }
    #[test]
    fn test_sleep_screen_off() { assert!(!PowerState::Sleep.is_screen_on()); }
    #[test]
    fn test_deep_sleep_screen_off() { assert!(!PowerState::DeepSleep.is_screen_on()); }
    #[test]
    fn test_wake_word_active_in_sleep() { assert!(PowerState::Sleep.is_wake_word_active()); }
    #[test]
    fn test_wake_word_inactive_in_critical() { assert!(!PowerState::CriticalLow.is_wake_word_active()); }
    #[test]
    fn test_no_satellite_tx_in_deep_sleep() { assert!(!PowerState::DeepSleep.allows_satellite_tx()); }
    #[test]
    fn test_satellite_tx_in_active() { assert!(PowerState::Active.allows_satellite_tx()); }
    #[test]
    fn test_app_actions_in_active() { assert!(PowerState::Active.allows_app_actions()); }
    #[test]
    fn test_no_app_actions_in_sleep() { assert!(!PowerState::Sleep.allows_app_actions()); }
    #[test]
    fn test_state_labels() {
        assert_eq!(PowerState::Active.label(), "active");
        assert_eq!(PowerState::DeepSleep.label(), "deep_sleep");
        assert_eq!(PowerState::CriticalLow.label(), "critical_low");
    }

    // BatteryStatus

    #[test]
    fn test_battery_full() { assert_eq!(BatteryStatus::full().percent(), 100); }
    #[test]
    fn test_battery_critical() { assert!(BatteryStatus::critical().is_critical()); }
    #[test]
    fn test_battery_low() { assert!(BatteryStatus::low().is_low()); }
    #[test]
    fn test_battery_above_reserve() { assert!(BatteryStatus::full().above_reserve()); }
    #[test]
    fn test_battery_clamp() { assert_eq!(BatteryStatus::new(1.5, false).level, 1.0); }
    #[test]
    fn test_battery_overheat() {
        let mut b = BatteryStatus::full();
        b.temperature_c = 50.0;
        assert!(b.is_overheating());
    }

    // PowerManager

    #[test]
    fn test_initial_state_active() { assert_eq!(pm().state, PowerState::Active); }

    #[test]
    fn test_on_activity_resets_timer() {
        let mut p = pm();
        p.on_activity();
        assert_eq!(p.state, PowerState::Active);
    }

    #[test]
    fn test_wake_word_wakes_from_sleep() {
        let mut p = pm();
        p.force_sleep();
        assert_eq!(p.state, PowerState::Sleep);
        p.on_wake_word();
        assert_eq!(p.state, PowerState::Active);
    }

    #[test]
    fn test_wake_word_wakes_from_deep_sleep() {
        let mut p = pm();
        p.force_deep_sleep();
        p.on_wake_word();
        assert_eq!(p.state, PowerState::Active);
    }

    #[test]
    fn test_critical_battery_transitions() {
        let mut p = pm();
        p.on_battery_update(BatteryStatus::critical());
        assert_eq!(p.state, PowerState::CriticalLow);
    }

    #[test]
    fn test_charger_connect_transitions() {
        let mut p = pm();
        p.on_battery_update(BatteryStatus::charging());
        assert_eq!(p.state, PowerState::ChargingActive);
    }

    #[test]
    fn test_charger_disconnect_transitions() {
        let mut p = pm();
        p.on_battery_update(BatteryStatus::charging());
        assert_eq!(p.state, PowerState::ChargingActive);
        p.on_battery_update(BatteryStatus::new(0.5, false));
        assert_eq!(p.state, PowerState::Active);
    }

    #[test]
    fn test_force_sleep() {
        let mut p = pm();
        p.force_sleep();
        assert_eq!(p.state, PowerState::Sleep);
        assert_eq!(p.sleep_count, 1);
    }

    #[test]
    fn test_force_deep_sleep() {
        let mut p = pm();
        p.force_deep_sleep();
        assert_eq!(p.state, PowerState::DeepSleep);
    }

    #[test]
    fn test_wake_lock_prevents_sleep() {
        let mut p = pm();
        let _idx = p.acquire_wake_lock("voip_call");
        assert_eq!(p.wake_lock_count(), 1);
        // Tick should not sleep while lock held
        p.tick();
        assert_eq!(p.state, PowerState::Active);
    }

    #[test]
    fn test_wake_lock_release() {
        let mut p = pm();
        let idx = p.acquire_wake_lock("test");
        p.release_wake_lock(idx);
        assert_eq!(p.wake_lock_count(), 0);
    }

    #[test]
    fn test_can_transmit_when_active() {
        let mut p = pm();
        assert!(p.can_transmit());
    }

    #[test]
    fn test_cannot_transmit_in_deep_sleep() {
        let mut p = pm();
        p.force_deep_sleep();
        assert!(!p.can_transmit());
    }

    #[test]
    fn test_cannot_transmit_below_reserve() {
        let mut p = pm();
        p.on_battery_update(BatteryStatus::new(0.03, false));
        assert!(!p.can_transmit());
    }

    #[test]
    fn test_can_run_apps_when_active() { assert!(pm().can_run_apps()); }

    #[test]
    fn test_cannot_run_apps_critical() {
        let mut p = pm();
        p.on_battery_update(BatteryStatus::critical());
        assert!(!p.can_run_apps());
    }

    #[test]
    fn test_battery_reserve_constant() {
        assert_eq!(BATTERY_RESERVE, 0.05);
    }

    #[test]
    fn test_wake_count_increments() {
        let mut p = pm();
        p.force_sleep();
        p.on_wake_word();
        assert_eq!(p.wake_count, 1);
    }
}
