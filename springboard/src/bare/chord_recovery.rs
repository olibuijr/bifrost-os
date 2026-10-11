//! Hardware recovery chord: Ctrl + Alt + Backspace held for 3 s on any keyboard makes the shell
//! exit with `DESKTOP_EXIT` (non-zero), and the systemd unit starts it again, whatever the UI is
//! doing; see `spawn`. Nothing is drawn, since the screen may be frozen.
//!
//! The hold timer is pure. `spawn` feeds it from a thread that sleeps in poll() on the keyboards'
//! evdev nodes (read-only, no grab), an inotify on /dev/input (hot-plug) and a wake fd, and then
//! reads the key state with EVIOCGKEY, which does not depend on the possibly hung main loop.
//! No key, no wakeup. The timeout is only set while the chord is down (the 3 s hold) or while the
//! UI loop looks hung (`heartbeat::ui_stale`; then every SAMPLE_MS). Where another process holds
//! an exclusive grab, our nodes see no events: the shell's own input reader calls `note_key`, and
//! the heartbeat watcher calls `poke` when the UI stalls.

use super::{crashlog, heartbeat, sys, takeover};
use std::ffi::{c_char, c_int, c_uint};
use std::sync::atomic::{AtomicI32, Ordering};

/// The chord is a deliberate three-key press, so a short hold does.
pub const DESKTOP_HOLD_MS: f64 = 3_000.0;
/// Exit status of the desktop chord: non-zero, so `Restart=on-failure` brings the shell back.
pub const DESKTOP_EXIT: i32 = 71;
const KEY_BACKSPACE: u16 = 14;
const KEY_LEFTCTRL: u16 = 29;
const KEY_LEFTALT: u16 = 56;
const KEY_RIGHTCTRL: u16 = 97;
const KEY_RIGHTALT: u16 = 100;
/// Sampling period while the UI loop looks hung.
const SAMPLE_MS: i32 = 200;
const EV_KEY: u16 = 1;
const EFD_NONBLOCK: c_int = 0o4000;
const EFD_CLOEXEC: c_int = 0o2000000;
const IN_ATTRIB: u32 = 0x4;
const IN_CREATE: u32 = 0x100;
const IN_DELETE: u32 = 0x200;

extern "C" {
    fn eventfd(init: c_uint, flags: c_int) -> c_int;
    fn inotify_init1(flags: c_int) -> c_int;
    fn inotify_add_watch(fd: c_int, path: *const c_char, mask: u32) -> c_int;
}

/// The wake eventfd (-1 before `spawn`).
static WAKE_FD: AtomicI32 = AtomicI32::new(-1);

/// Whether Ctrl + Alt + Backspace (either side's Ctrl and Alt) is down in an EVIOCGKEY bitmap.
pub fn desktop_chord_down(bits: &[u8]) -> bool {
    let held = |a: u16, b: u16| takeover::key_held(bits, a) || takeover::key_held(bits, b);
    held(KEY_LEFTCTRL, KEY_RIGHTCTRL) && held(KEY_LEFTALT, KEY_RIGHTALT) && takeover::key_held(bits, KEY_BACKSPACE)
}

/// Whether `code` is one of the chord's keys.
pub fn is_chord_key(code: u16) -> bool {
    matches!(code, KEY_BACKSPACE | KEY_LEFTCTRL | KEY_LEFTALT | KEY_RIGHTCTRL | KEY_RIGHTALT)
}

/// Whether any chord key is down in an EVIOCGKEY bitmap.
pub fn any_chord_key(bits: &[u8]) -> bool {
    [KEY_BACKSPACE, KEY_LEFTCTRL, KEY_LEFTALT, KEY_RIGHTCTRL, KEY_RIGHTALT].iter().any(|&k| takeover::key_held(bits, k))
}

/// Wakes the watcher thread with one write; a no-op before `spawn`.
pub fn poke() {
    let fd = WAKE_FD.load(Ordering::Relaxed);
    if fd >= 0 {
        let one = 1u64.to_ne_bytes();
        // SAFETY: `one` is 8 readable bytes, the size an eventfd write takes.
        unsafe { sys::write(fd, one.as_ptr() as *const _, 8) };
    }
}

/// The shell's input reader reports a key press: wakes the watcher when it is a chord key.
pub fn note_key(code: u16) {
    if is_chord_key(code) {
        poke();
    }
}

/// Hold timer of the desktop chord.
#[derive(Debug, Default)]
pub struct DesktopHold {
    since: Option<f64>,
}

impl DesktopHold {
    /// Feeds whether the chord is down now; true once it has been for DESKTOP_HOLD_MS.
    pub fn feed(&mut self, down: bool, now_ms: f64) -> bool {
        if !down {
            self.since = None;
            return false;
        }
        now_ms - *self.since.get_or_insert(now_ms) >= DESKTOP_HOLD_MS
    }

    /// Milliseconds (rounded up) until the hold completes; None while the chord is not down.
    pub fn remaining_ms(&self, now_ms: f64) -> Option<i32> {
        self.since.map(|s| (DESKTOP_HOLD_MS - (now_ms - s)).max(0.0).ceil() as i32)
    }
}

/// poll() timeout: the hold's remainder while the chord is down, SAMPLE_MS while the UI looks
/// hung, otherwise -1 (sleep until an event).
pub fn wait_timeout_ms(remaining: Option<i32>, ui_stale: bool) -> i32 {
    match (remaining, ui_stale) {
        (Some(r), true) => r.min(SAMPLE_MS),
        (Some(r), false) => r,
        (None, true) => SAMPLE_MS,
        (None, false) => -1,
    }
}

/// The keyboard nodes (they report EV_KEY with a Backspace key), open read-only and non-blocking.
fn open_keyboards() -> Vec<c_int> {
    let mut fds = vec![];
    let Ok(rd) = std::fs::read_dir("/dev/input") else { return fds };
    for e in rd.flatten() {
        let path = e.path();
        if !path.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("event")) {
            continue;
        }
        let Some(fd) = path.to_str().and_then(|p| sys::open_dev(p, sys::O_RDONLY | sys::O_NONBLOCK).ok()) else { continue };
        let mut caps = [0u8; 96];
        // EVIOCGBIT(EV_KEY, len) = _IOC(_IOC_READ, 'E', 0x20 + EV_KEY, len)
        let ok = unsafe { sys::ioctl_ptr(fd, sys::ioc(2, b'E', 0x20 + EV_KEY as u8, caps.len()), caps.as_mut_ptr()) }.is_ok();
        if ok && takeover::key_held(&caps, KEY_BACKSPACE) {
            fds.push(fd);
        } else {
            // SAFETY: `fd` was opened above and is not used afterwards.
            unsafe { sys::close(fd) };
        }
    }
    fds
}

/// The union of the key states of the open keyboards (EVIOCGKEY needs no grab).
fn all_keys(fds: &[c_int]) -> [u8; 96] {
    let mut all = [0u8; 96];
    for &fd in fds {
        let mut bits = [0u8; 96];
        // EVIOCGKEY(len) = _IOC(_IOC_READ, 'E', 0x18, len)
        if unsafe { sys::ioctl_ptr(fd, sys::ioc(2, b'E', 0x18, bits.len()), bits.as_mut_ptr()) }.is_ok() {
            all.iter_mut().zip(bits).for_each(|(a, b)| *a |= b);
        }
    }
    all
}

/// Reads and drops what is queued on `fd`; false when the node is gone or failed.
fn drain(fd: c_int) -> bool {
    let mut buf = [0u8; 24 * 64];
    loop {
        let n = unsafe { sys::read(fd, buf.as_mut_ptr() as *mut _, buf.len()) };
        if n > 0 {
            continue;
        }
        return n < 0 && std::io::Error::last_os_error().kind() == std::io::ErrorKind::WouldBlock;
    }
}

fn close_all(fds: &mut Vec<c_int>) {
    for fd in fds.drain(..) {
        // SAFETY: each fd was opened by `open_keyboards` and is dropped from the list here.
        unsafe { sys::close(fd) };
    }
}

/// Starts the watcher thread: Ctrl + Alt + Backspace for 3 s exits with DESKTOP_EXIT and systemd
/// restarts the shell, whatever the UI is doing. It sleeps in poll() until a key event, a
/// hot-plug or a poke (see the module doc).
pub fn spawn(dir: &str) {
    let dir = dir.to_string();
    // SAFETY: plain syscalls with constant arguments; a failure gives -1, which poll ignores.
    let wake = unsafe { eventfd(0, EFD_NONBLOCK | EFD_CLOEXEC) };
    WAKE_FD.store(wake, Ordering::Relaxed);
    std::thread::Builder::new()
        .name("recovery-chord".into())
        .spawn(move || {
            let ino = unsafe { inotify_init1(EFD_NONBLOCK | EFD_CLOEXEC) };
            if ino >= 0 {
                unsafe { inotify_add_watch(ino, c"/dev/input".as_ptr(), IN_CREATE | IN_DELETE | IN_ATTRIB) };
            }
            let mut hold = DesktopHold::default();
            let mut devs = open_keyboards();
            let (mut ready, mut stale) = (Vec::new(), false);
            loop {
                let timeout = wait_timeout_ms(hold.remaining_ms(sys::now_ms()), stale);
                let mut fds = vec![wake, ino];
                fds.extend(&devs);
                sys::wait_into(&fds, timeout, &mut ready);
                let mut rescan = ready[1];
                if ready[0] {
                    drain(wake);
                }
                if ready[1] {
                    let mut buf = [0u8; 4096];
                    while unsafe { sys::read(ino, buf.as_mut_ptr() as *mut _, buf.len()) } > 0 {}
                }
                for (i, &fd) in devs.iter().enumerate() {
                    if ready[i + 2] && !drain(fd) {
                        rescan = true;
                    }
                }
                if rescan {
                    close_all(&mut devs);
                    devs = open_keyboards();
                }
                let now = sys::now_ms();
                stale = heartbeat::ui_stale(now);
                if hold.feed(desktop_chord_down(&all_keys(&devs)), now) {
                    takeover::log(&dir, "Ctrl + Alt + Backspace held 3s: exiting for a restart");
                    crashlog::write_last_exit(&dir, DESKTOP_EXIT, "recovery chord");
                    std::process::exit(DESKTOP_EXIT);
                }
            }
        })
        .ok();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn desktop_chord_needs_ctrl_alt_and_backspace() {
        fn set(bits: &mut [u8; 96], c: u16, on: bool) {
            if on {
                bits[c as usize / 8] |= 1 << (c % 8);
            } else {
                bits[c as usize / 8] &= !(1 << (c % 8));
            }
        }
        let mut bits = [0u8; 96];
        set(&mut bits, KEY_RIGHTCTRL, true);
        set(&mut bits, KEY_LEFTALT, true);
        assert!(!desktop_chord_down(&bits));
        set(&mut bits, KEY_BACKSPACE, true);
        assert!(desktop_chord_down(&bits));
        set(&mut bits, KEY_LEFTALT, false);
        assert!(!desktop_chord_down(&bits));
    }

    #[test]
    fn desktop_hold_fires_after_three_seconds_and_resets() {
        let mut h = DesktopHold::default();
        assert!(!h.feed(true, 0.0));
        assert!(!h.feed(true, 2_999.0));
        assert!(h.feed(true, 3_000.0));
        assert!(!h.feed(false, 3_100.0));
        assert!(!h.feed(true, 3_200.0));
    }

    #[test]
    fn wait_timeout_sleeps_unless_holding_or_hung() {
        assert_eq!(wait_timeout_ms(None, false), -1);
        assert_eq!(wait_timeout_ms(None, true), SAMPLE_MS);
        assert_eq!(wait_timeout_ms(Some(2_400), false), 2_400);
        assert_eq!(wait_timeout_ms(Some(50), true), 50);
    }

    #[test]
    fn hold_remaining_counts_down_only_while_down() {
        let mut h = DesktopHold::default();
        assert_eq!(h.remaining_ms(0.0), None);
        h.feed(true, 100.0);
        assert_eq!(h.remaining_ms(100.0), Some(3_000));
        // rounded up: the poll must not wake before the hold is due
        assert_eq!(h.remaining_ms(1_100.5), Some(2_000));
        assert_eq!(h.remaining_ms(9_000.0), Some(0));
        h.feed(false, 200.0);
        assert_eq!(h.remaining_ms(200.0), None);
    }

    #[test]
    fn chord_keys_are_ctrl_alt_backspace_only() {
        assert!(is_chord_key(KEY_RIGHTALT) && is_chord_key(KEY_BACKSPACE));
        assert!(!is_chord_key(30));
        let mut bits = [0u8; 96];
        assert!(!any_chord_key(&bits));
        bits[KEY_LEFTCTRL as usize / 8] |= 1 << (KEY_LEFTCTRL % 8);
        assert!(any_chord_key(&bits));
    }
}
