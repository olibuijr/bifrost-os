//! Test-only helpers shared by the whole crate.
//!
//! Some state is process-wide: the `SPRINGBOARD_WAYLAND` environment variable, the appearance fade in
//! `bifrost_render::appearance`, the glass flags. Tests run in parallel threads, so a test that sets or changes any of it
//! must hold `global_lock()` for its whole body, or another test reads the value mid-change.
//!
//! Heavy read-only fixtures (the UI fonts, solid full-screen wallpapers) are made once per process here and shared, so
//! the suite's memory does not grow with the number of tests that build a shell.

use std::sync::{Mutex, MutexGuard};

static GLOBAL: Mutex<()> = Mutex::new(());

/// Holds the crate-wide lock for process-global state until the guard is dropped.
///
/// Poison-tolerant: a test that panicked while holding the lock does not make every test after it fail.
pub fn global_lock() -> MutexGuard<'static, ()> {
    GLOBAL.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// A file under `springboard/assets`, read once per process and shared by every test (read only).
///
/// Tests used to `Box::leak` a fresh copy of the UI fonts for each shell or `Ui` they built: about 1 MB per call that is
/// never freed, over 800 calls in the lib suite.
#[cfg(any(test, feature = "testkit"))]
pub fn asset(name: &'static str) -> &'static [u8] {
    use std::collections::HashMap;
    use std::sync::OnceLock;
    static FILES: OnceLock<Mutex<HashMap<&'static str, &'static [u8]>>> = OnceLock::new();
    let mut files = FILES.get_or_init(Default::default).lock().unwrap_or_else(|p| p.into_inner());
    files.entry(name).or_insert_with(|| {
        let path = format!("{}/assets/{name}", env!("CARGO_MANIFEST_DIR"));
        Box::leak(std::fs::read(&path).unwrap_or_else(|e| panic!("{path}: {e}")).into_boxed_slice())
    })
}

/// The system UI font (`assets/SFUI.ttf`), shared.
#[cfg(any(test, feature = "testkit"))]
pub fn sfui() -> &'static [u8] {
    asset("SFUI.ttf")
}

/// The monospaced UI font (`assets/SFUIMono.ttf`), shared.
#[cfg(any(test, feature = "testkit"))]
pub fn sfui_mono() -> &'static [u8] {
    asset("SFUIMono.ttf")
}

/// A `w` x `h` buffer of one packed colour, made once per (size, colour) and shared by every test (read only).
///
/// A test shell's wallpaper is a full-screen `&'static [u32]`; leaking a new one per shell cost 9.2 MB at 1200 x 1920
/// for each of the 400-odd shells the suite builds. The set of sizes and colours tests use is small and fixed.
#[cfg(any(test, feature = "testkit"))]
pub fn solid(w: usize, h: usize, px: u32) -> &'static [u32] {
    static BUFS: Mutex<Vec<((usize, usize, u32), &'static [u32])>> = Mutex::new(Vec::new());
    let mut bufs = BUFS.lock().unwrap_or_else(|p| p.into_inner());
    if let Some(&(_, b)) = bufs.iter().find(|(k, _)| *k == (w, h, px)) {
        return b;
    }
    let b: &'static [u32] = Box::leak(vec![px; w * h].into_boxed_slice());
    bufs.push(((w, h, px), b));
    b
}

#[cfg(test)]
mod tests {
    #[test]
    fn shared_fixtures_are_made_once() {
        assert!(std::ptr::eq(super::sfui(), super::sfui()));
        assert!(!super::sfui_mono().is_empty());
        let a = super::solid(4, 3, 7);
        assert_eq!(a.len(), 12);
        assert!(a.iter().all(|&p| p == 7));
        assert!(std::ptr::eq(a, super::solid(4, 3, 7)));
        assert!(!std::ptr::eq(a, super::solid(4, 3, 8)), "another colour is another buffer");
    }
}
