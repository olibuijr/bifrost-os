//! Idle rules for the screen and the system: auto-lock after inactivity, and system sleep after the screen has been off.
//!
//! The shell's idle monitor and power service decide when these take effect, and apps cannot change them. The models
//! here let an app show the same values, validate a stored preference and check a given idle time, so the rules are
//! testable without a display or a power service. Values mirror the shell's constants.

/// Auto-lock delay in milliseconds used while Low Power Mode is on. Auto-lock is never longer than this then.
pub const LOW_POWER_LOCK_MS: u32 = 30_000;
/// Screen-off time in milliseconds before the system first attempts to suspend.
pub const SUSPEND_AFTER_MS: f64 = 30_000.0;
/// Preference key for the system sleep delay, in minutes. The value `never` (or no value) means the system never sleeps.
pub const SYSTEM_SLEEP_PREF: &str = "system_sleep_minutes";

/// The auto-lock delay in milliseconds that applies, given the stored delay and the Low Power Mode state.
/// 0 means the screen never locks on its own. With Low Power Mode on, a zero delay becomes the Low Power delay and any
/// longer delay is shortened to it.
pub fn effective_auto_lock_ms(stored_ms: u32, low_power: bool) -> u32 {
    match (low_power, stored_ms) {
        (true, 0) => LOW_POWER_LOCK_MS,
        (true, ms) => ms.min(LOW_POWER_LOCK_MS),
        (false, ms) => ms,
    }
}

/// Whether the screen locks after `idle_ms` of inactivity at the effective delay `lock_ms`.
pub fn should_auto_lock(idle_ms: u64, lock_ms: u32) -> bool {
    lock_ms > 0 && idle_ms >= u64::from(lock_ms)
}

/// The system sleep delay in milliseconds for a stored preference in minutes. None means never: a missing, unparsable,
/// zero or negative value, or a non-finite one.
pub fn system_sleep_after_ms(pref: Option<&str>) -> Option<f64> {
    let minutes: f64 = pref?.trim().parse().ok()?;
    (minutes > 0.0 && minutes.is_finite()).then_some(minutes * 60_000.0)
}

/// The stored value for a system sleep delay: the minutes, or `never` for 0.
pub fn system_sleep_pref_value(minutes: u32) -> String {
    if minutes == 0 {
        "never".to_string()
    } else {
        minutes.to_string()
    }
}
