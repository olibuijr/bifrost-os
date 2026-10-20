//! The pinned clock and seed for snapshot tests, shared by the shell crate and the SDK (`bifrost_app::testing`
//! re-exports it). The compliance harness sets [`ENV_PIN`] to `1` before it spawns an app binary, so anything that
//! reads the wall clock through [`pinned`] draws the same frame on every run.
//!
//! The app binaries the harness spawns are ordinary release builds, so the pin cannot be compiled out of them; it
//! is read from the environment once per process and cached, and a process pins itself only through [`pin_process`].

use std::sync::atomic::{AtomicU8, Ordering};

/// Environment variable that pins the clock and the random seed.
pub const ENV_PIN: &str = "BIFROST_TEST_PIN";
/// The pinned wall clock: 2025-10-10 12:00:00 UTC, in Unix seconds.
pub const PINNED_UNIX_SECS: i64 = 1_760_097_600;
/// The pinned random seed (non-zero, so xorshift generators work).
pub const PINNED_SEED: u64 = 0x9E37_79B9_7F4A_7C15;

/// Cached answer: 0 = not read yet, 1 = not pinned, 2 = pinned.
static STATE: AtomicU8 = AtomicU8::new(0);

/// True when the clock and seed are pinned (`BIFROST_TEST_PIN=1`). The environment is read on the first call only.
pub fn pinned() -> bool {
    match STATE.load(Ordering::Relaxed) {
        1 => false,
        2 => true,
        _ => {
            let on = std::env::var(ENV_PIN).is_ok_and(|v| v == "1");
            STATE.store(if on { 2 } else { 1 }, Ordering::Relaxed);
            on
        }
    }
}

/// Pins this process and, through the environment, the app binaries it spawns. Call it before spawning.
pub fn pin_process() {
    std::env::set_var(ENV_PIN, "1");
    STATE.store(2, Ordering::Relaxed);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pin_is_cached_and_pin_process_turns_it_on() {
        // One test owns the process-wide state, so parallel tests cannot race on it.
        std::env::remove_var(ENV_PIN);
        STATE.store(0, Ordering::Relaxed);
        assert!(!pinned());
        // A later change to the environment does not flip a cached answer.
        std::env::set_var(ENV_PIN, "1");
        assert!(!pinned());
        pin_process();
        assert!(pinned());
        assert_eq!(std::env::var(ENV_PIN).as_deref(), Ok("1"));
        std::env::remove_var(ENV_PIN);
        STATE.store(0, Ordering::Relaxed);
    }
}
