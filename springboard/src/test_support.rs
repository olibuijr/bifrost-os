//! Test-only helpers shared by the whole crate.
//!
//! Some state is process-wide: the `SPRINGBOARD_WAYLAND` environment variable, the appearance fade in
//! `bifrost_render::appearance`, the glass flags. Tests run in parallel threads, so a test that sets or changes any of it
//! must hold `global_lock()` for its whole body, or another test reads the value mid-change.

use std::sync::{Mutex, MutexGuard};

static GLOBAL: Mutex<()> = Mutex::new(());

/// Holds the crate-wide lock for process-global state until the guard is dropped.
///
/// Poison-tolerant: a test that panicked while holding the lock does not make every test after it fail.
pub fn global_lock() -> MutexGuard<'static, ()> {
    GLOBAL.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}
