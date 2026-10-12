//! BifrOSt's boot-to-desktop handoff supervisor (the `bootweave` binary, `/usr/bin/bifrost-bootweave`).
//!
//! The UEFI loader paints the boot frame and hands its progress record over; this process keeps the same frame and
//! its progress bar on screen in the inherited framebuffer until the window server takes the display (no modeset,
//! no DRM master: the bar's rows are written to the framebuffer device, which shows what the display already scans).
//! It is the one owner of the session's lifecycle, and after `desktop-faded` it writes the boot timing record
//! (`EFI/BIFROST/boot-timing`, the smoothed durations the next boot's bar uses) and the module list the minimal
//! initramfs is built from (`EFI/BIFROST/boot-modules`).
//!
//! Failures are shown in the boot frame, never as text (docs/boot-architecture.md, "Text-free on every path"):
//! - a kernel panic record in pstore on this boot shows "Your computer restarted because of a problem" (`crash_notice`);
//! - a session that cannot start (three quick failures in a row) shows "BifrOSt could not start" with Restart and
//!   Recovery (`start_failed`), instead of the restart loop of the service.
//!
//! Owners: boot task 06 (progress, `docs/boot/06-bootweave-progress.md`) and boot task 11 (failure screens and pstore;
//! the loader's rollback screen is in `bootweave/efi`).

use bootweave_frame::{render, Frame, Screen};
use springboard::bare::input::{Device, Event};
use springboard::bare::kms::Display;
use springboard::gfx::Damage;
use springboard::pstore;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use bootweave_frame::render::{bar_rect, render_bar, PixelOrder};
use bootweave_frame::{Appearance, BootTiming, Progress, Stage};

const STATE_DIR: &str = "/run/bifrost/bootweave";
/// Consecutive quick session failures in this boot (a file, so it survives the service's restarts).
const FAILURES: &str = "/run/bifrost/bootweave/failures";
const LOG: &str = "/var/log/bifrost-bootweave.log";
const SESSION: &str = "/usr/lib/bifrost/session";
const RUNTIME: &str = "/run/bifrost/user";
/// The loader's progress hand-off: the encoded `BootTiming` the loader used for this boot (its `loader_ms` is this
/// boot's measured loader time).
const HANDOFF: &str = "/run/bifrost/boot-progress";
/// The ESP's boot timing record (`EFI/BIFROST/boot-timing`), mounted at /boot on installed systems.
const TIMING_PATH: &str = "/boot/EFI/BIFROST/boot-timing";
/// The modules this machine loaded, for the minimal initramfs (task 04).
const MODULES_PATH: &str = "/boot/EFI/BIFROST/boot-modules";
const PROC_MODULES: &str = "/proc/modules";
/// The inherited framebuffer (simpledrm or the GPU driver's fbdev emulation) and its sysfs description.
const FB_DEV: &str = "/dev/fb0";
const FB_SYSFS: &str = "/sys/class/graphics/fb0";
/// Frame continuity hook (task 01): when `REQUEST` exists at handover, the framebuffer is copied to `OUT.bgrx` with its
/// geometry in `OUT.txt`.
const READBACK_REQUEST: &str = "/run/bifrost/bootweave/readback-request";
const READBACK_OUT: &str = "/run/bifrost/bootweave/readback";
/// How often the bar moves, and how long it may run if no window server ever takes the display.
const BAR_TICK: Duration = Duration::from_millis(33);
const BAR_CEILING: Duration = Duration::from_secs(60);
/// How long the record waits for `desktop-faded` before giving up on this boot.
const RECORD_WAIT: Duration = Duration::from_secs(120);
/// The boot frame's appearance until the timing record can say (task 06 stores the user's choice there).
const APPEARANCE: Appearance = Appearance::Dark;
/// How long the crash notice stays on the panel before the session takes it over.
const CRASH_NOTICE_MS: u64 = 3000;
/// How long to wait for a display to open before giving up on a screen.
const DISPLAY_WAIT_MS: u64 = 5000;

/// Quick failures before the start-failure screen replaces the restart loop.
pub const FAILS_BEFORE_SCREEN: u32 = 3;
/// A session that ran this long had started: its later exit is a crash, not a start failure.
pub const STARTED_MS: u128 = 20_000;

/// What follows a session exit.
#[derive(Debug, PartialEq, Eq)]
pub enum Next {
    /// A clean exit: the supervisor exits 0 and the service stays down.
    Done,
    /// A failure short of the screen: exit non-zero and let systemd restart the service.
    Retry,
    /// The session cannot start: show the start-failure screen and hold it.
    Screen,
}

/// Decides what follows a session exit, given the consecutive start failures so far. Returns the new count and the
/// next step. A clean exit resets the count; a session that ran for `STARTED_MS` or more counts as started, so its
/// crash resets the count too and it simply restarts.
pub fn after_exit(count: u32, clean: bool, ran_ms: u128) -> (u32, Next) {
    if clean {
        return (0, Next::Done);
    }
    if ran_ms >= STARTED_MS {
        return (0, Next::Retry);
    }
    let count = count.saturating_add(1);
    if count >= FAILS_BEFORE_SCREEN {
        (count, Next::Screen)
    } else {
        (count, Next::Retry)
    }
}

/// The failure count from the state file's text (anything unreadable counts as none).
pub fn parse_count(text: &str) -> u32 {
    text.trim().parse().unwrap_or(0)
}

// Linux key codes the failure screen uses (linux/input-event-codes.h).
const KEY_TAB: u16 = 15;
const KEY_ENTER: u16 = 28;
const KEY_SPACE: u16 = 57;
const KEY_KPENTER: u16 = 96;
const KEY_LEFT: u16 = 105;
const KEY_RIGHT: u16 = 106;
/// The start-failure screen's buttons, in order: Restart is first and selected at the start.
const BUTTONS: u32 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Previous,
    Next,
    Activate,
}

/// The failure screen's meaning of a key press, `None` for keys it does not use.
pub fn key_of(code: u16) -> Option<Key> {
    match code {
        KEY_LEFT => Some(Key::Previous),
        KEY_RIGHT | KEY_TAB => Some(Key::Next),
        KEY_ENTER | KEY_KPENTER | KEY_SPACE => Some(Key::Activate),
        _ => None,
    }
}

/// The selected button after `key`. Moving stops at the ends (no wrap); activating keeps the selection.
pub fn step(selected: u32, key: Key) -> u32 {
    match key {
        Key::Previous => selected.saturating_sub(1),
        Key::Next => (selected + 1).min(BUTTONS - 1),
        Key::Activate => selected,
    }
}

/// The button a selection index names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Button {
    Restart,
    Recovery,
}

pub fn button(selected: u32) -> Button {
    if selected == 0 { Button::Restart } else { Button::Recovery }
}

const COMPOSITOR_CONF: &str = "/etc/bifrost/compositor";

const WINDOW_SERVER: &str = "/usr/bin/bifrost-wm";
// The shutdown handshake with bifrost-wm (springboard/wm/src/boot.rs, "Symmetric shutdown"). The request is written
// here by root; the faded marker is written by the window server, which runs as the account, so it lives in RUNTIME.
const SHUTDOWN_REQUEST: &str = "/run/bifrost/bootweave/shutdown";
const SHUTDOWN_FADED: &str = "/run/bifrost/user/shutdown-faded";
/// How long `shutdown` waits for the fade. `springboard.service` has a 2 s stop budget (`TimeoutStopSec`), and the
/// session's own stop comes after this, so the wait stays well inside it.
const SHUTDOWN_WAIT_MS: u64 = 1200;
/// The unit's stop budget in seconds, the value `packaging/arch/springboard.service` must carry.
#[cfg(test)]
const STOP_BUDGET_S: u64 = 2;

fn timestamp_ms() -> u128 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis()
}

fn mark(event: &str) {
    if !event.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_') {
        return;
    }
    springboard::bootsplash::mark(event);
    // Journal copy for `tools/boot-bench.py`: a shutdown-start mark is read back from the previous boot's journal,
    // because /run/bifrost/boot-marks is gone after the reboot.
    eprintln!("boot-mark {event} {} {:.1}", timestamp_ms(), springboard::bootsplash::boottime_ms());
    let _ = fs::create_dir_all(STATE_DIR);
    let line = format!("{} {}\n", timestamp_ms(), event);
    let _ = fs::write(format!("{STATE_DIR}/last"), &line);
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(LOG) {
        let _ = file.write_all(line.as_bytes());
    }
}

fn read_count() -> u32 {
    fs::read_to_string(FAILURES).map(|t| parse_count(&t)).unwrap_or(0)
}

fn write_count(count: u32) {
    let _ = fs::create_dir_all(STATE_DIR);
    let _ = fs::write(FAILURES, format!("{count}\n"));
}

fn park() -> ! {
    loop {
        thread::sleep(Duration::from_secs(3600));
    }
}

/// Renders `screen` at the panel's size and presents it. The display stays open while the caller holds it.
fn present(display: &mut Display, screen: Screen) {
    let (w, h) = display.size();
    let frame = Frame { width: w as u32, height: h as u32, appearance: APPEARANCE, progress: None, screen };
    let mut pixels = vec![0u32; w * h];
    render(&frame, &mut pixels, w, PixelOrder::Bgrx);
    let mut damage = Damage::new(w, h);
    damage.all();
    display.present(&pixels, &damage);
}

/// "Your computer restarted because of a problem": one screen for the pstore crash records of this boot, then the
/// records are removed. Without a display the records stay for the next boot.
fn crash_notice(records: &[PathBuf]) {
    let Ok(mut display) = Display::open_best(DISPLAY_WAIT_MS) else {
        eprintln!("bifrost-bootweave: no display for the crash notice; keeping {} record(s)", records.len());
        return;
    };
    present(&mut display, Screen::RestartedAfterProblem);
    mark("crash-notice-shown");
    thread::sleep(Duration::from_millis(CRASH_NOTICE_MS));
    pstore::clear(records);
    // the display closes here, so the session can take the panel
}

fn open_inputs() -> Vec<Device> {
    (0..32).filter_map(|i| Device::open(&format!("/dev/input/event{i}")).ok()).collect()
}

fn restart() {
    mark("restart-requested");
    if let Err(error) = Command::new("systemctl").arg("reboot").status() {
        eprintln!("bifrost-bootweave: cannot restart: {error}");
    }
}

/// "BifrOSt could not start": Restart and Recovery, held until one is chosen. Never returns.
fn start_failed() -> ! {
    mark("start-failed");
    let Ok(mut display) = Display::open_best(DISPLAY_WAIT_MS) else {
        // no panel to draw on: hold the service rather than restart it in a loop
        park();
    };
    let mut selected = 0u32;
    present(&mut display, Screen::StartFailed { selected });
    mark("start-failed-shown");
    let mut inputs = open_inputs();
    loop {
        let mut events = Vec::new();
        for device in inputs.iter_mut() {
            device.read(&mut events);
        }
        let mut changed = false;
        for event in events {
            let Event::Key { code, down: true } = event else { continue };
            let Some(key) = key_of(code) else { continue };
            if key == Key::Activate {
                match button(selected) {
                    Button::Restart => restart(),
                    // TODO(boot-11): Recovery needs the recovery environment, which does not exist yet.
                    Button::Recovery => mark("recovery-requested"),
                }
            } else {
                let next = step(selected, key);
                changed |= next != selected;
                selected = next;
            }
        }
        if changed {
            present(&mut display, Screen::StartFailed { selected });
        }
        thread::sleep(Duration::from_millis(30));
    }
}

/// Applies the failure policy to a session exit (`status` None when it could not be spawned) and exits, shows the
/// start-failure screen, or lets systemd restart the service.
fn finish(status: Option<ExitStatus>, ran: Duration) -> ! {
    let clean = status.is_some_and(|s| s.success());
    let (count, next) = after_exit(read_count(), clean, ran.as_millis());
    write_count(count);
    match next {
        Next::Done => std::process::exit(0),
        Next::Retry => std::process::exit(status.and_then(|s| s.code()).filter(|c| *c != 0).unwrap_or(1)),
        Next::Screen => start_failed(),
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Pure logic (host-tested)
// ---------------------------------------------------------------------------------------------------------------

/// The boot marks file (`ms event` per line, milliseconds since the kernel started) as (event, ms) pairs. The first
/// mark of an event wins, as `bootsplash::mark` does; malformed lines are skipped.
pub fn parse_marks(text: &str) -> Vec<(String, f64)> {
    let mut out: Vec<(String, f64)> = Vec::new();
    for line in text.lines() {
        let mut fields = line.split_whitespace();
        let (Some(ms), Some(event)) = (fields.next(), fields.next()) else { continue };
        let Ok(ms) = ms.parse::<f64>() else { continue };
        if !ms.is_finite() || ms < 0.0 || out.iter().any(|(seen, _)| seen == event) {
            continue;
        }
        out.push((event.to_string(), ms));
    }
    out
}

/// The time of `event` in the marks, if it has been reported.
pub fn mark_ms(marks: &[(String, f64)], event: &str) -> Option<f64> {
    marks.iter().find(|(seen, _)| seen == event).map(|(_, ms)| *ms)
}

/// One boot's measured stage durations (ms).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Measured {
    /// The loader's time for this boot, from the hand-off; `None` when the loader did not report one.
    pub loader_ms: Option<u32>,
    /// Kernel start (time zero) to Bootweave start: the `supervisor-start` mark.
    pub kernel_ms: u32,
    /// Bootweave start to desktop-ready: the `desktop-ready` mark, or `desktop-faded` when that is all there is.
    pub userspace_ms: u32,
}

/// Measures this boot from the marks. `None` when the start or the end is missing, or the end is before the start.
pub fn measure(marks: &[(String, f64)], loader_ms: Option<u32>) -> Option<Measured> {
    let start = mark_ms(marks, "supervisor-start")?;
    let end = mark_ms(marks, "desktop-ready").or_else(|| mark_ms(marks, "desktop-faded"))?;
    if end < start {
        return None;
    }
    Some(Measured { loader_ms, kernel_ms: start.round() as u32, userspace_ms: (end - start).round() as u32 })
}

/// The record after this boot: the previous record (the defaults when there is none) with this boot folded in as one
/// sample. A missing loader time keeps the previous record's loader average.
pub fn next_record(prev: Option<BootTiming>, measured: Measured, appearance: Appearance) -> BootTiming {
    let mut timing = prev.unwrap_or(BootTiming::DEFAULT);
    let loader = measured.loader_ms.unwrap_or(timing.loader_ms);
    timing.add_sample(loader, measured.kernel_ms, measured.userspace_ms);
    timing.appearance = appearance;
    timing
}

/// The timing this boot runs with: the loader's hand-off, else the ESP record, else the defaults. The hand-off's
/// appearance is the one the user chose at the loader, so it wins too.
pub fn pick_timing(handoff: Option<BootTiming>, prev: Option<BootTiming>) -> BootTiming {
    handoff.or(prev).unwrap_or(BootTiming::DEFAULT)
}

/// The module names in `/proc/modules`, sorted and unique.
pub fn module_list(proc_modules: &str) -> Vec<String> {
    let mut names: Vec<String> = proc_modules
        .lines()
        .filter_map(|line| line.split_whitespace().next())
        .map(str::to_string)
        .collect();
    names.sort();
    names.dedup();
    names
}

/// The module list file: one name per line, with a trailing newline (empty for no modules).
pub fn module_text(names: &[String]) -> String {
    names.iter().map(|name| format!("{name}\n")).collect()
}

/// Milliseconds since loader start: the loader's time plus the boot clock (which starts at kernel start).
pub fn elapsed_ms(loader_ms: u32, boot_ms: f64) -> u32 {
    (loader_ms as f64 + boot_ms.max(0.0)).round() as u32
}

/// `"W,H"` from fbdev's `virtual_size`, both non-zero.
pub fn parse_virtual_size(text: &str) -> Option<(u32, u32)> {
    let (w, h) = text.trim().split_once(',')?;
    let (w, h): (u32, u32) = (w.trim().parse().ok()?, h.trim().parse().ok()?);
    (w > 0 && h > 0).then_some((w, h))
}

/// The readback's description: width, height, stride (in pixels), and the byte order of the `.bgrx` file.
pub fn readback_header(width: u32, height: u32, stride_px: usize) -> String {
    format!("{width} {height} {stride_px} bgrx\n")
}

/// The progress bar for a boot seeded from the loader: the bar stands where the loader left it (its checkpoint) and
/// the loader's stage counts as done, so the bar continues from there instead of jumping.
pub fn seed(timing: BootTiming) -> Progress {
    let mut progress = Progress::new(timing);
    progress.shown = progress.checkpoint(Stage::Loader);
    progress.stage_done(Stage::Loader);
    // Bootweave starts when the kernel stage ends, so that checkpoint may now be passed.
    progress.stage_done(Stage::Kernel);
    progress
}

// ---------------------------------------------------------------------------------------------------------------
// Files
// ---------------------------------------------------------------------------------------------------------------

/// A boot timing record (or the loader's hand-off) read from `path`; `None` when missing or not a valid record.
fn read_record(path: &str) -> Option<BootTiming> {
    fs::read(path).ok().and_then(|bytes| BootTiming::decode(&bytes))
}

/// Writes `bytes` to `path` through a temporary file, so a power cut leaves the old file or the new one.
fn write_atomic(path: &str, bytes: &[u8]) -> io::Result<()> {
    let target = Path::new(path);
    if let Some(dir) = target.parent() {
        fs::create_dir_all(dir)?;
    }
    let tmp = format!("{path}.tmp");
    let mut file = File::create(&tmp)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    drop(file);
    fs::rename(&tmp, target)
}

fn read_u32(path: &str) -> Option<u32> {
    fs::read_to_string(path).ok()?.trim().parse().ok()
}

/// Writes the module list when this boot's modules differ from the file (ESP writes only when needed).
fn write_module_list() {
    let Ok(text) = fs::read_to_string(PROC_MODULES) else { return };
    let wanted = module_text(&module_list(&text));
    if fs::read_to_string(MODULES_PATH).ok().as_deref() == Some(wanted.as_str()) {
        return;
    }
    if let Err(error) = write_atomic(MODULES_PATH, wanted.as_bytes()) {
        eprintln!("bifrost-bootweave: cannot write {MODULES_PATH}: {error}");
    }
}

// ---------------------------------------------------------------------------------------------------------------
// The bar in the inherited framebuffer
// ---------------------------------------------------------------------------------------------------------------

/// The bar on the inherited framebuffer. Each tick renders the bar into a full-size buffer and writes only the bar's
/// rows to `/dev/fb0`, so the boot frame's other pixels stay as the loader (or the previous stage) left them.
struct Bar {
    progress: Progress,
    loader_ms: u32,
    width: u32,
    height: u32,
    /// Bytes per row of the framebuffer.
    fb_stride: usize,
    buf: Vec<u32>,
    fb: File,
}

impl Bar {
    fn open(timing: BootTiming) -> io::Result<Bar> {
        let text = fs::read_to_string(format!("{FB_SYSFS}/virtual_size"))?;
        let (width, height) = parse_virtual_size(&text).ok_or_else(|| io::Error::other("fb0 has no size"))?;
        let bpp = read_u32(&format!("{FB_SYSFS}/bits_per_pixel")).unwrap_or(32);
        if bpp != 32 {
            return Err(io::Error::other(format!("fb0 is {bpp} bits per pixel")));
        }
        let row_bytes = width as usize * 4;
        let fb_stride = read_u32(&format!("{FB_SYSFS}/stride")).map_or(row_bytes, |s| (s as usize).max(row_bytes));
        let fb = OpenOptions::new().read(true).write(true).open(FB_DEV)?;
        Ok(Bar {
            progress: seed(timing),
            loader_ms: timing.loader_ms,
            width,
            height,
            fb_stride,
            buf: vec![0; width as usize * height as usize],
            fb,
        })
    }

    /// Moves the bar to where this boot is now (`boot_ms` since kernel start).
    fn tick(&mut self, boot_ms: f64) -> io::Result<()> {
        if !self.progress.visible() {
            return Ok(());
        }
        let shown = self.progress.at(elapsed_ms(self.loader_ms, boot_ms));
        let appearance = self.progress.timing.appearance;
        render_bar(self.width, self.height, shown, appearance, &mut self.buf, self.width as usize, PixelOrder::Bgrx);
        let (x, y, w, h) = bar_rect(self.width, self.height);
        let mut row = Vec::with_capacity(w as usize * 4);
        for r in y..y + h {
            row.clear();
            let start = r as usize * self.width as usize + x as usize;
            for px in &self.buf[start..start + w as usize] {
                row.extend_from_slice(&px.to_le_bytes());
            }
            self.fb.seek(SeekFrom::Start(r as u64 * self.fb_stride as u64 + x as u64 * 4))?;
            self.fb.write_all(&row)?;
        }
        Ok(())
    }

    /// The frame continuity hook: copies the whole framebuffer when a readback was requested.
    fn readback(&mut self) -> io::Result<()> {
        if !Path::new(READBACK_REQUEST).exists() {
            return Ok(());
        }
        let mut bytes = vec![0u8; self.fb_stride * self.height as usize];
        self.fb.seek(SeekFrom::Start(0))?;
        self.fb.read_exact(&mut bytes)?;
        write_atomic(&format!("{READBACK_OUT}.bgrx"), &bytes)?;
        let header = readback_header(self.width, self.height, self.fb_stride / 4);
        write_atomic(&format!("{READBACK_OUT}.txt"), header.as_bytes())
    }
}

/// Keeps the bar moving until `stop` is set (the window server has the display), the ceiling passes, or the
/// framebuffer cannot be drawn; then runs the readback hook.
fn bar_loop(stop: Arc<AtomicBool>, timing: BootTiming) {
    let mut bar = match Bar::open(timing) {
        Ok(bar) => bar,
        Err(error) => {
            eprintln!("bifrost-bootweave: bar not drawn: {error}");
            return;
        }
    };
    let started = Instant::now();
    while !stop.load(Ordering::Relaxed) && started.elapsed() < BAR_CEILING {
        if let Err(error) = bar.tick(springboard::bootsplash::boottime_ms()) {
            eprintln!("bifrost-bootweave: bar stopped: {error}");
            break;
        }
        thread::sleep(BAR_TICK);
    }
    if let Err(error) = bar.readback() {
        eprintln!("bifrost-bootweave: readback failed: {error}");
    }
}

// ---------------------------------------------------------------------------------------------------------------
// After the boot
// ---------------------------------------------------------------------------------------------------------------

/// Waits for `desktop-faded`, then folds this boot into the timing record and refreshes the module list.
fn record_when_faded(prev: Option<BootTiming>, handoff: Option<BootTiming>) {
    let started = Instant::now();
    loop {
        let marks = parse_marks(&fs::read_to_string(springboard::bootsplash::MARKS).unwrap_or_default());
        if mark_ms(&marks, "desktop-faded").is_some() {
            if let Some(measured) = measure(&marks, handoff.map(|h| h.loader_ms)) {
                let record = next_record(prev, measured, pick_timing(handoff, prev).appearance);
                if let Err(error) = write_atomic(TIMING_PATH, &record.encode()) {
                    eprintln!("bifrost-bootweave: cannot write {TIMING_PATH}: {error}");
                }
            }
            write_module_list();
            return;
        }
        if started.elapsed() > RECORD_WAIT {
            return;
        }
        thread::sleep(Duration::from_millis(250));
    }
}

// ---------------------------------------------------------------------------------------------------------------
// The supervisor
// ---------------------------------------------------------------------------------------------------------------

/// What `shutdown` does, decided from the configured window server.
#[derive(Debug, PartialEq, Eq)]
enum ShutdownPlan {
    /// bifrost-wm draws the boot frame over the desktop: request the fade and wait for it.
    Fade,
    /// Another window server (or none) has no fade: return at once, the stop goes on.
    Skip,
}

/// The same choice the session script makes (`packaging/arch/wayland/session`): only the bifrost-wm name fades.
fn shutdown_plan(compositor_conf: &str) -> ShutdownPlan {
    if compositor_conf.trim() == "bifrost-wm" { ShutdownPlan::Fade } else { ShutdownPlan::Skip }
}

/// Removes the handshake files of an earlier run (`/run` is cleared on reboot, a restart of the unit is not).
fn clear_shutdown_state() {
    let _ = fs::remove_file(SHUTDOWN_REQUEST);
    let _ = fs::remove_file(SHUTDOWN_FADED);
}

/// `ExecStop=` of springboard.service: asks the window server to fade the desktop back to the boot frame, then waits
/// (at most `SHUTDOWN_WAIT_MS`) until it has. The frame stays up until power-off; the stop of the session follows.
/// Always exits 0, so a missing or stuck window server never blocks the stop.
fn shutdown() {
    // the shutdown milestone first, whichever window server runs (tools/boot-bench.py reads it on the next boot)
    mark("shutdown-start");
    let conf = fs::read_to_string(COMPOSITOR_CONF).unwrap_or_default();
    // the session falls back to labwc when the binary is missing, so no fade is coming either
    if shutdown_plan(&conf) == ShutdownPlan::Skip || !Path::new(WINDOW_SERVER).exists() {
        return;
    }
    clear_shutdown_state();
    let _ = fs::create_dir_all(STATE_DIR);
    if fs::write(SHUTDOWN_REQUEST, b"1\n").is_err() {
        return;
    }
    mark("shutdown-request");
    let started = Instant::now();
    while !Path::new(SHUTDOWN_FADED).exists() {
        if started.elapsed() >= Duration::from_millis(SHUTDOWN_WAIT_MS) {
            mark("shutdown-fade-timeout");
            return;
        }
        thread::sleep(Duration::from_millis(20));
    }
}

fn run() -> ! {
    clear_shutdown_state();
    mark("bootweave-start");
    mark("display-handoff-start");
    let records = pstore::crash_records(Path::new(pstore::DIR));
    if !records.is_empty() {
        crash_notice(&records);
    }
    let prev = read_record(TIMING_PATH);
    let handoff = read_record(HANDOFF);
    let timing = pick_timing(handoff, prev);
    // Set once the window server has the display (its socket exists): the bar stops and never draws over it.
    let handed_over = Arc::new(AtomicBool::new(false));
    {
        let stop = handed_over.clone();
        thread::spawn(move || bar_loop(stop, timing));
    }
    thread::spawn(move || record_when_faded(prev, handoff));
    let started = Instant::now();
    let mut child = match Command::new(SESSION).env("BIFROST_BOOTWEAVE", "1").spawn() {
        Ok(child) => child,
        Err(error) => {
            mark("session-spawn-failed");
            eprintln!("bifrost-bootweave: cannot start session: {error}");
            finish(None, started.elapsed());
        }
    };
    // Marks come from a short-lived watcher that ends once both milestones are seen; after the
    // first frame this process only blocks in wait().
    thread::spawn(move || {
        let ready = Path::new(RUNTIME).join("shell-ready");
        let wayland = Path::new(RUNTIME).join("wayland-0");
        let mut compositor_seen = false;
        loop {
            if !compositor_seen && wayland.exists() {
                compositor_seen = true;
                handed_over.store(true, Ordering::Relaxed);
                mark("compositor-ready");
            }
            if ready.exists() {
                handed_over.store(true, Ordering::Relaxed);
                if !compositor_seen { mark("compositor-ready"); }
                mark("desktop-first-frame");
                // Provisional until the window server's three-part gate (docs/boot-architecture.md, "Reveal once"):
                // today the shell's first ready frame is the desktop.
                mark("desktop-ready");
                // reaching the desktop ends this boot's run of start failures
                write_count(0);
                return;
            }
            thread::sleep(Duration::from_millis(20));
        }
    });
    match child.wait() {
        Ok(status) => {
            mark("session-exit");
            finish(Some(status), started.elapsed());
        }
        Err(error) => {
            eprintln!("bifrost-bootweave: wait failed: {error}");
            std::process::exit(1);
        }
    }
}

fn main() {
    match std::env::args().nth(1).as_deref() {
        Some("run") => run(),
        Some("mark") => {
            if let Some(event) = std::env::args().nth(2) {
                mark(&event);
            }
        }
        Some("shutdown") => shutdown(),
        _ => {
            eprintln!("usage: bifrost-bootweave run|mark EVENT|shutdown");
            std::process::exit(2);
        }
    }
}

#[cfg(test)]
mod failure_tests {
    use super::*;

    #[test]
    fn a_clean_exit_ends_the_policy_and_resets_the_count() {
        assert_eq!(after_exit(2, true, 500), (0, Next::Done));
    }

    #[test]
    fn quick_failures_retry_until_the_third_then_show_the_screen() {
        assert_eq!(after_exit(0, false, 500), (1, Next::Retry));
        assert_eq!(after_exit(1, false, 500), (2, Next::Retry));
        assert_eq!(after_exit(2, false, 500), (3, Next::Screen));
        assert_eq!(after_exit(3, false, 500), (4, Next::Screen), "held on the screen, never back to retry");
    }

    #[test]
    fn a_session_that_ran_long_enough_restarts_without_counting() {
        assert_eq!(after_exit(2, false, 19_999), (3, Next::Screen), "just under STARTED_MS still counts");
        assert_eq!(after_exit(2, false, 20_000), (0, Next::Retry), "STARTED_MS counts as started");
        assert_eq!(after_exit(0, false, 90_000), (0, Next::Retry));
    }

    #[test]
    fn a_spawn_failure_is_a_quick_failure() {
        assert_eq!(after_exit(0, false, 0), (1, Next::Retry));
    }

    #[test]
    fn the_count_file_reads_back_or_counts_as_none() {
        assert_eq!(parse_count("2\n"), 2);
        assert_eq!(parse_count(""), 0);
        assert_eq!(parse_count("two"), 0);
        assert_eq!(parse_count("-1"), 0);
    }

    #[test]
    fn keys_map_to_the_failure_screen_actions() {
        assert_eq!(key_of(KEY_LEFT), Some(Key::Previous));
        assert_eq!(key_of(KEY_RIGHT), Some(Key::Next));
        assert_eq!(key_of(KEY_TAB), Some(Key::Next));
        assert_eq!(key_of(KEY_ENTER), Some(Key::Activate));
        assert_eq!(key_of(KEY_KPENTER), Some(Key::Activate));
        assert_eq!(key_of(30), None, "the A key does nothing here");
    }

    #[test]
    fn selection_moves_between_the_two_buttons_without_wrapping() {
        assert_eq!(step(0, Key::Next), 1);
        assert_eq!(step(1, Key::Next), 1, "Recovery is the last button");
        assert_eq!(step(1, Key::Previous), 0);
        assert_eq!(step(0, Key::Previous), 0, "Restart is the first button");
        assert_eq!(step(1, Key::Activate), 1, "activating keeps the selection");
    }

    #[test]
    fn the_first_button_is_restart() {
        assert_eq!(button(0), Button::Restart);
        assert_eq!(button(1), Button::Recovery);
    }
}

#[cfg(test)]
mod progress_tests {
    use super::*;

    fn marks(text: &str) -> Vec<(String, f64)> {
        parse_marks(text)
    }

    #[test]
    fn marks_keep_the_first_of_each_event_and_skip_junk() {
        let parsed = marks("10.0 supervisor-start\n20.5 desktop-faded\n30.0 supervisor-start\nbad line\n-4 x\n");
        assert_eq!(parsed, vec![("supervisor-start".to_string(), 10.0), ("desktop-faded".to_string(), 20.5)]);
        assert_eq!(mark_ms(&parsed, "desktop-faded"), Some(20.5));
        assert_eq!(mark_ms(&parsed, "desktop-ready"), None);
    }

    #[test]
    fn measure_splits_kernel_from_userspace_at_bootweave_start() {
        // kernel: 0 to 412.3 ms (rounds to 412); userspace: 412.3 to 2600.0 ms (2187.7, rounds to 2188)
        let m = measure(&marks("412.3 supervisor-start\n2600.0 desktop-ready\n2900.0 desktop-faded\n"), Some(300)).unwrap();
        assert_eq!(m, Measured { loader_ms: Some(300), kernel_ms: 412, userspace_ms: 2188 });
    }

    #[test]
    fn measure_falls_back_to_faded_and_rejects_incomplete_marks() {
        // no desktop-ready: the end is desktop-faded, 3000.0 - 412.3 = 2587.7 rounds to 2588
        let m = measure(&marks("412.3 supervisor-start\n3000.0 desktop-faded\n"), None).unwrap();
        assert_eq!(m, Measured { loader_ms: None, kernel_ms: 412, userspace_ms: 2588 });
        assert_eq!(measure(&marks("412.3 supervisor-start\n"), None), None, "no end mark");
        assert_eq!(measure(&marks("3000.0 desktop-faded\n"), None), None, "no start mark");
        assert_eq!(measure(&marks("500.0 supervisor-start\n400.0 desktop-faded\n"), None), None, "end before start");
    }

    #[test]
    fn the_next_record_folds_this_boot_in_and_takes_the_chosen_appearance() {
        let measured = Measured { loader_ms: Some(300), kernel_ms: 412, userspace_ms: 2188 };
        // no previous record: this boot is the only sample, so the record is this boot's measurement
        let first = next_record(None, measured, Appearance::Light);
        assert_eq!(first.samples, 1);
        assert_eq!(first.kernel_ms, 412);
        assert_eq!(first.userspace_ms, 2188);
        assert_eq!(first.appearance, Appearance::Light);
        // a previous record of 5 boots stays at 5 samples (the smoothing window)
        let prev = BootTiming { loader_ms: 280, kernel_ms: 400, userspace_ms: 2000, samples: 5, appearance: Appearance::Dark };
        let next = next_record(Some(prev), measured, Appearance::Dark);
        assert_eq!(next.samples, 5);
        // no loader measurement keeps the previous loader average
        let no_loader = Measured { loader_ms: None, ..measured };
        assert_eq!(next_record(Some(prev), no_loader, Appearance::Dark).loader_ms, 280);
    }

    #[test]
    fn the_timing_for_this_boot_prefers_the_loader_hand_off_then_the_record_then_the_defaults() {
        let handoff = BootTiming { loader_ms: 250, kernel_ms: 1400, userspace_ms: 2100, samples: 3, appearance: Appearance::Light };
        let prev = BootTiming { loader_ms: 300, kernel_ms: 1500, userspace_ms: 2000, samples: 1, appearance: Appearance::Dark };
        assert_eq!(pick_timing(Some(handoff), Some(prev)), handoff);
        assert_eq!(pick_timing(None, Some(prev)), prev);
        assert_eq!(pick_timing(None, None), BootTiming::DEFAULT);
        assert_eq!(BootTiming::DEFAULT.total_ms(), 3800, "defaults: 300 + 1500 + 2000 ms");
    }

    #[test]
    fn the_module_list_is_sorted_unique_and_one_per_line() {
        let proc = "i915 3305472 2 - Live 0x0\nbtusb 80 0 - Live 0x0\ni915 3305472 2 - Live 0x0\n\nxhci_hcd 1 0 - Live 0\n";
        let names = module_list(proc);
        assert_eq!(names, vec!["btusb".to_string(), "i915".to_string(), "xhci_hcd".to_string()]);
        assert_eq!(module_text(&names), "btusb\ni915\nxhci_hcd\n");
        assert_eq!(module_text(&[]), "", "no modules: an empty file");
    }

    #[test]
    fn the_elapsed_clock_adds_the_loader_time_and_never_goes_negative() {
        assert_eq!(elapsed_ms(300, 1234.6), 1535);
        assert_eq!(elapsed_ms(0, -5.0), 0);
        assert_eq!(elapsed_ms(250, 0.4), 250);
    }

    #[test]
    fn the_framebuffer_size_and_the_readback_header_are_exact() {
        assert_eq!(parse_virtual_size("1920,1080\n"), Some((1920, 1080)));
        assert_eq!(parse_virtual_size("1920x1080"), None, "fbdev uses a comma");
        assert_eq!(parse_virtual_size("0,0"), None);
        assert_eq!(readback_header(1920, 1080, 1920), "1920 1080 1920 bgrx\n");
    }

    /// The shipped unit, read at test time so the tests check the file that gets installed.
    const UNIT: &str = include_str!("../../packaging/arch/springboard.service");

    fn unit_value(key: &str) -> Option<&str> {
        UNIT.lines().map(str::trim).find_map(|line| line.strip_prefix(key)?.strip_prefix('='))
    }

    #[test]
    fn only_the_window_server_that_draws_the_boot_frame_fades_at_shutdown() {
        assert_eq!(shutdown_plan("bifrost-wm\n"), ShutdownPlan::Fade);
        assert_eq!(shutdown_plan("bifrost-wm"), ShutdownPlan::Fade);
        assert_eq!(shutdown_plan("labwc\n"), ShutdownPlan::Skip);
        assert_eq!(shutdown_plan(""), ShutdownPlan::Skip);
        assert_eq!(shutdown_plan("bifrost-wm; rm -rf /"), ShutdownPlan::Skip);
    }

    #[test]
    fn the_handshake_files_are_where_their_writers_can_write() {
        assert!(SHUTDOWN_REQUEST.starts_with(STATE_DIR));
        assert!(SHUTDOWN_FADED.starts_with(RUNTIME));
        assert_ne!(SHUTDOWN_REQUEST, SHUTDOWN_FADED);
    }

    #[test]
    fn the_supervisor_and_the_window_server_agree_on_the_handshake_paths() {
        // bifrost-wm reads the request and writes the marker at these exact paths (springboard/wm/src/boot.rs)
        assert_eq!(SHUTDOWN_REQUEST, "/run/bifrost/bootweave/shutdown");
        assert_eq!(SHUTDOWN_FADED, "/run/bifrost/user/shutdown-faded");
    }

    #[test]
    fn the_window_server_binary_is_the_one_the_session_starts() {
        assert_eq!(WINDOW_SERVER, "/usr/bin/bifrost-wm");
    }

    #[test]
    fn the_fade_wait_fits_inside_the_unit_stop_budget() {
        assert_eq!(STOP_BUDGET_S, 2, "the 2 s budget of the shutdown contract");
        assert!(SHUTDOWN_WAIT_MS < STOP_BUDGET_S * 1000, "wait {SHUTDOWN_WAIT_MS} ms");
        // the fade itself (450 ms) fits in the wait with room for the frame to be presented
        assert!(SHUTDOWN_WAIT_MS >= 450 * 2);
    }

    #[test]
    fn the_unit_asks_for_the_fade_before_it_stops_the_session() {
        // `shutdown` writes the shutdown-start mark itself, so the unit needs one stop command
        // `shutdown` writes the shutdown-start mark itself, so the unit needs one stop command
        assert_eq!(unit_value("ExecStop"), Some("/usr/bin/bifrost-bootweave shutdown"));
        assert_eq!(UNIT.lines().filter(|l| l.trim_start().starts_with("ExecStop=")).count(), 1);
        assert_eq!(UNIT.lines().filter(|l| l.trim_start().starts_with("ExecStop=")).count(), 1);
        assert_eq!(unit_value("TimeoutStopSec"), Some("2s"));
    }

    #[test]
    fn the_unit_no_longer_parks_the_console_on_vt_12() {
        assert!(!UNIT.contains("ExecStopPost"), "the VT 12 parking is removed");
        assert!(!UNIT.contains("chvt 12"));
    }

    #[test]
    fn the_unit_keeps_its_shutdown_ordering() {
        // shutdown.target still stops this unit, and the unit still conflicts with it
        assert!(UNIT.lines().any(|l| l.trim() == "Before=shutdown.target"));
        assert!(UNIT.lines().any(|l| l.trim() == "Conflicts=shutdown.target"));
    }
}
