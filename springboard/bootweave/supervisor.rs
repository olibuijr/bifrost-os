//! BifrOSt's boot-to-desktop handoff supervisor (the `bootweave` binary, `/usr/bin/bifrost-bootweave`).
//!
//! The UEFI loader paints the boot frame and hands its progress record over; this process keeps the same frame and
//! its progress bar on screen in the inherited framebuffer until the window server takes the display (no modeset,
//! no DRM master: the bar's rows are written to the framebuffer device, which shows what the display already scans).
//! It is the one owner of the session's lifecycle, and after `desktop-faded` it writes the boot timing record
//! (`EFI/BIFROST/boot-timing`, the smoothed durations the next boot's bar uses).
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
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use bootweave_frame::render::PixelOrder;
use bootweave_frame::timing::Handoff;
use bootweave_frame::{Appearance, BootTiming, Progress, Stage};
use std::sync::Mutex;

const STATE_DIR: &str = "/run/bifrost/bootweave";
/// Consecutive quick session failures in this boot (a file, so it survives the service's restarts).
const FAILURES: &str = "/run/bifrost/bootweave/failures";
const LOG: &str = "/var/log/bifrost-bootweave.log";
const SESSION: &str = "/usr/lib/bifrost/session";
const RUNTIME: &str = "/run/bifrost/user";
/// The loader's progress hand-off: the `BifrostBootProgress` EFI variable (efivarfs: 4 attribute bytes, then the
/// 32-byte `timing::Handoff` with this boot's measured `loader_ms` at offset 24).
const HANDOFF: &str = "/sys/firmware/efi/efivars/BifrostBootProgress-d1f0b07c-5b2e-4a8d-9c3f-6e4a2b7d9f10";
/// The bar position as the window server continues it (contract C3): a `timing::Handoff` record, replaced atomically.
const PROGRESS: &str = "/run/bifrost/bootweave/progress";
/// The mark every boot's measurement starts from: the first contract mark, which `run` writes.
const START_MARK: &str = springboard::bootsplash::CONTRACT_MARKS[0];
/// The shell's settings (the Appearance choice lives here as `appearance_mode=`).
const SETTINGS: &str = "/var/lib/springboard/settings.ini";
/// The ESP's boot timing record (`EFI/BIFROST/boot-timing`), mounted at /boot on installed systems.
const TIMING_PATH: &str = "/boot/EFI/BIFROST/boot-timing";
/// The inherited framebuffer (simpledrm or the GPU driver's fbdev emulation) and its sysfs description.
const FB_DEV: &str = "/dev/fb0";
const FB_SYSFS: &str = "/sys/class/graphics/fb0";
/// Frame continuity hook (task 01): when `READBACK_REQUEST` exists (a persistent file, created on the machine under
/// test), the framebuffer is copied before the first draw (`inherited`) and after it (`bootweave`) in the format
/// `tools/boot-frames.py` reads: `<stage>.json` (width, height, stride in pixels, order) and `<stage>.raw`.
const READBACK_REQUEST: &str = "/etc/bifrost/boot-readback";
const READBACK_DIR: &str = "/run/bifrost/boot-frames";
/// How often the bar moves, and how long it may run if no window server ever takes the display.
const BAR_TICK: Duration = Duration::from_millis(33);
const BAR_CEILING: Duration = Duration::from_secs(60);
/// How long the record waits for `desktop-faded` before giving up on this boot.
const RECORD_WAIT: Duration = Duration::from_secs(120);
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

/// The one appearance source for every boot screen: the user's Settings choice, else the boot timing record the
/// loader also reads, else Dark. The loader, Bootweave and the shell all end up on this value.
pub fn boot_appearance() -> Appearance {
    fs::read_to_string(SETTINGS)
        .ok()
        .and_then(|text| appearance_from_settings(&text))
        .or_else(|| read_record(TIMING_PATH).map(|r| r.appearance))
        .unwrap_or_default()
}

/// Renders `screen` at the panel's size and presents it. The display stays open while the caller holds it.
fn present(display: &mut Display, screen: Screen) {
    let (w, h) = display.size();
    let frame = Frame { width: w as u32, height: h as u32, appearance: boot_appearance(), progress: None, screen };
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
        Next::Screen => {
            stop_bar();
            start_failed()
        }
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
    /// Kernel start (time zero) to Bootweave start: the `bootweave-start` mark.
    pub kernel_ms: u32,
    /// Bootweave start to desktop-ready: the `desktop-ready` mark, or `desktop-faded` when that is all there is.
    pub userspace_ms: u32,
}

/// Measures this boot from the marks. `None` when the start or the end is missing, or the end is before the start.
pub fn measure(marks: &[(String, f64)], loader_ms: Option<u32>) -> Option<Measured> {
    let start = mark_ms(marks, START_MARK)?;
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

/// The readback's description in the checker's format (`tools/boot-frames.py`, `check_meta`).
pub fn readback_meta(width: u32, height: u32, stride_px: usize, order: PixelOrder) -> String {
    let order = if order == PixelOrder::Rgbx { "rgbx" } else { "bgrx" };
    format!("{{\"width\": {width}, \"height\": {height}, \"stride\": {stride_px}, \"order\": \"{order}\"}}\n")
}

/// The pixel order from fbdev's channel offsets (bits): red at 0 and blue at 16 is `RedGreenBlueReserved`; anything
/// else is treated as the usual blue-green-red.
pub fn order_from(red_offset: u32, blue_offset: u32) -> PixelOrder {
    if red_offset == 0 && blue_offset == 16 { PixelOrder::Rgbx } else { PixelOrder::Bgrx }
}

/// The loader's hand-off from the efivarfs file image: 4 attribute bytes, then the 32-byte record.
pub fn decode_efivar(bytes: &[u8]) -> Option<Handoff> {
    Handoff::decode(bytes.get(4..)?)
}

/// The appearance the user chose in Settings (`appearance_mode=` in `settings.ini`). `auto` and anything unknown
/// return `None`: the timing record's value stands.
pub fn appearance_from_settings(text: &str) -> Option<Appearance> {
    let value = text.lines().find_map(|l| l.trim().strip_prefix("appearance_mode="))?;
    match value.trim() {
        "light" => Some(Appearance::Light),
        "dark" => Some(Appearance::Dark),
        _ => None,
    }
}

/// Whether the bar thread keeps drawing: not stopped (the window server has the display, or a failure screen does)
/// and inside the ceiling.
pub fn bar_runs(stopped: bool, elapsed: Duration) -> bool {
    !stopped && elapsed < BAR_CEILING
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

/// A boot timing record read from `path`; `None` when missing or not a valid record.
fn read_record(path: &str) -> Option<BootTiming> {
    fs::read(path).ok().and_then(|bytes| BootTiming::decode(&bytes))
}

/// Replaces `path` through a temporary file in the same directory (a reader sees the old or the new record). No
/// fsync: this is `/run`, and the bar moves many times a second.
fn publish(path: &str, bytes: &[u8]) -> io::Result<()> {
    let tmp = format!("{path}.tmp");
    fs::write(&tmp, bytes)?;
    fs::rename(&tmp, path)
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

// ---------------------------------------------------------------------------------------------------------------
// The bar in the inherited framebuffer
// ---------------------------------------------------------------------------------------------------------------

/// The bar on the inherited framebuffer. Each tick renders the bar into a full-size buffer and writes only the bar's
/// rows to `/dev/fb0`, so the boot frame's other pixels stay as the loader (or the previous stage) left them.
struct Bar {
    progress: Progress,
    loader_ms: u32,
    order: PixelOrder,
    /// The last bar position published to `PROGRESS`, in thousandths.
    published: Option<u32>,
    width: u32,
    height: u32,
    /// Bytes per row of the framebuffer.
    fb_stride: usize,
    buf: Vec<u32>,
    fb: File,
}

impl Bar {
    fn open(timing: BootTiming, loader_ms: u32) -> io::Result<Bar> {
        let text = fs::read_to_string(format!("{FB_SYSFS}/virtual_size"))?;
        let (width, height) = parse_virtual_size(&text).ok_or_else(|| io::Error::other("fb0 has no size"))?;
        let bpp = read_u32(&format!("{FB_SYSFS}/bits_per_pixel")).unwrap_or(32);
        if bpp != 32 {
            return Err(io::Error::other(format!("fb0 is {bpp} bits per pixel")));
        }
        let row_bytes = width as usize * 4;
        let fb_stride = read_u32(&format!("{FB_SYSFS}/stride")).map_or(row_bytes, |s| (s as usize).max(row_bytes));
        let fb = OpenOptions::new().read(true).write(true).open(FB_DEV)?;
        let order = fb_order(&fb);
        let mut bar = Bar {
            progress: seed(timing),
            loader_ms,
            order,
            published: None,
            width,
            height,
            fb_stride,
            buf: vec![0; width as usize * height as usize],
            fb,
        };
        bar.draw_frame()?;
        Ok(bar)
    }

    /// Draws the whole boot frame once (the mark alone, no bar) through the shared renderer, so the panel shows the
    /// same pixels as the loader's frame and as the start-failed screen without its buttons.
    fn draw_frame(&mut self) -> io::Result<()> {
        let frame = Frame { width: self.width, height: self.height, appearance: self.progress.timing.appearance, progress: None, screen: Screen::Boot };
        render(&frame, &mut self.buf, self.width as usize, self.order);
        let mut row = Vec::with_capacity(self.width as usize * 4);
        for r in 0..self.height as usize {
            row.clear();
            for px in &self.buf[r * self.width as usize..(r + 1) * self.width as usize] {
                row.extend_from_slice(&px.to_le_bytes());
            }
            self.fb.seek(SeekFrom::Start(r as u64 * self.fb_stride as u64))?;
            self.fb.write_all(&row)?;
        }
        Ok(())
    }

    /// Moves the bar to where this boot is now (`boot_ms` since kernel start), and publishes the position when it
    /// moved (contract C3).
    fn tick(&mut self, boot_ms: f64) -> io::Result<()> {
        if !self.progress.visible() {
            return Ok(());
        }
        let elapsed = elapsed_ms(self.loader_ms, boot_ms);
        let shown = self.progress.at(elapsed);
        let permille = (shown * 1000.0).round().clamp(0.0, 1000.0) as u32;
        if self.published != Some(permille) {
            let record = Handoff { timing: self.progress.timing, loader_ms: self.loader_ms, shown_permille: permille };
            if publish(PROGRESS, &record.encode()).is_ok() {
                self.published = Some(permille);
            }
        }
        // the boot frame has no bar: only the honest position is published
        Ok(())
    }

    /// The frame continuity hook: copies the whole framebuffer as `stage` when a readback was requested.
    fn readback(&mut self, stage: &str) -> io::Result<()> {
        if !Path::new(READBACK_REQUEST).exists() {
            return Ok(());
        }
        let mut bytes = vec![0u8; self.fb_stride * self.height as usize];
        self.fb.seek(SeekFrom::Start(0))?;
        self.fb.read_exact(&mut bytes)?;
        fs::create_dir_all(READBACK_DIR)?;
        write_atomic(&format!("{READBACK_DIR}/{stage}.raw"), &bytes)?;
        let meta = readback_meta(self.width, self.height, self.fb_stride / 4, self.order);
        write_atomic(&format!("{READBACK_DIR}/{stage}.json"), meta.as_bytes())
    }
}

/// The framebuffer's pixel order from `FBIOGET_VSCREENINFO` (red and blue channel offsets); blue-green-red when the
/// query fails.
fn fb_order(fb: &File) -> PixelOrder {
    use std::os::fd::AsRawFd;
    extern "C" {
        fn ioctl(fd: i32, request: u64, ...) -> i32;
    }
    const FBIOGET_VSCREENINFO: u64 = 0x4600;
    // struct fb_var_screeninfo is 40 u32 words; red {offset, length, msb_right} starts at word 8, blue at word 14.
    let mut info = [0u32; 40];
    // SAFETY: `info` is a writable buffer of 160 bytes, the size of `struct fb_var_screeninfo`, which the ioctl fills.
    let rc = unsafe { ioctl(fb.as_raw_fd(), FBIOGET_VSCREENINFO, info.as_mut_ptr()) };
    if rc != 0 { PixelOrder::Bgrx } else { order_from(info[8], info[14]) }
}

/// Set when the bar must stop drawing: the window server has the display, or a failure screen is about to take it.
static BAR_STOP: AtomicBool = AtomicBool::new(false);
/// The bar thread, so `finish` can wait for its last write before a failure screen is drawn.
static BAR_THREAD: Mutex<Option<thread::JoinHandle<()>>> = Mutex::new(None);

/// Stops the bar and waits for it (at most one tick) so nothing draws over the screen that follows.
fn stop_bar() {
    BAR_STOP.store(true, Ordering::Relaxed);
    let handle = BAR_THREAD.lock().ok().and_then(|mut slot| slot.take());
    if let Some(handle) = handle {
        let _ = handle.join();
    }
}

/// Keeps the bar moving until `BAR_STOP` is set, the ceiling passes, or the framebuffer cannot be drawn; takes the
/// frame continuity readbacks around the first draw.
fn bar_loop(timing: BootTiming, loader_ms: u32) {
    let mut bar = match Bar::open(timing, loader_ms) {
        Ok(bar) => bar,
        Err(error) => {
            eprintln!("bifrost-bootweave: bar not drawn: {error}");
            mark("display-handoff-start");
            return;
        }
    };
    if let Err(error) = bar.readback("inherited") {
        eprintln!("bifrost-bootweave: readback failed: {error}");
    }
    let started = Instant::now();
    let mut first = true;
    while bar_runs(BAR_STOP.load(Ordering::Relaxed), started.elapsed()) {
        if let Err(error) = bar.tick(springboard::bootsplash::boottime_ms()) {
            eprintln!("bifrost-bootweave: bar stopped: {error}");
            break;
        }
        if first {
            first = false;
            // the display handoff starts when the first bar write lands, after the bootweave start mark
            mark("display-handoff-start");
            if let Err(error) = bar.readback("bootweave") {
                eprintln!("bifrost-bootweave: readback failed: {error}");
            }
        }
        thread::sleep(BAR_TICK);
    }
    if first {
        mark("display-handoff-start");
    }
}

// ---------------------------------------------------------------------------------------------------------------
// After the boot
// ---------------------------------------------------------------------------------------------------------------

/// Waits for `desktop-faded`, then folds this boot into the timing record.
fn record_when_faded(prev: Option<BootTiming>, handoff: Option<Handoff>, appearance: Appearance) {
    let started = Instant::now();
    loop {
        let marks = parse_marks(&fs::read_to_string(springboard::bootsplash::MARKS).unwrap_or_default());
        if mark_ms(&marks, "desktop-faded").is_some() {
            if let Some(measured) = measure(&marks, handoff.map(|h| h.loader_ms)) {
                let record = next_record(prev, measured, appearance);
                if let Err(error) = write_atomic(TIMING_PATH, &record.encode()) {
                    eprintln!("bifrost-bootweave: cannot write {TIMING_PATH}: {error}");
                }
            }
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
    // without the window server binary there is no fade coming either
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
    mark(START_MARK);
    let records = pstore::crash_records(Path::new(pstore::DIR));
    if !records.is_empty() {
        crash_notice(&records);
    }
    let prev = read_record(TIMING_PATH);
    let handoff = fs::read(HANDOFF).ok().and_then(|bytes| decode_efivar(&bytes));
    let mut timing = pick_timing(handoff.map(|h| h.timing), prev);
    // the user's Settings choice wins over the record the loader carried
    let chosen = fs::read_to_string(SETTINGS).ok().and_then(|text| appearance_from_settings(&text));
    if let Some(appearance) = chosen {
        timing.appearance = appearance;
    }
    let loader_ms = handoff.map_or(timing.loader_ms, |h| h.loader_ms);
    // BAR_STOP is set once the window server has the display (its socket exists): the bar never draws over it.
    let handle = thread::spawn(move || bar_loop(timing, loader_ms));
    if let Ok(mut slot) = BAR_THREAD.lock() {
        *slot = Some(handle);
    }
    let appearance = timing.appearance;
    thread::spawn(move || record_when_faded(prev, handoff, appearance));
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
                BAR_STOP.store(true, Ordering::Relaxed);
                mark("compositor-ready");
            }
            if ready.exists() {
                BAR_STOP.store(true, Ordering::Relaxed);
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
        let parsed = marks("10.0 bootweave-start\n20.5 desktop-faded\n30.0 bootweave-start\nbad line\n-4 x\n");
        assert_eq!(parsed, vec![("bootweave-start".to_string(), 10.0), ("desktop-faded".to_string(), 20.5)]);
        assert_eq!(mark_ms(&parsed, "desktop-faded"), Some(20.5));
        assert_eq!(mark_ms(&parsed, "desktop-ready"), None);
    }

    #[test]
    fn measure_splits_kernel_from_userspace_at_bootweave_start() {
        // kernel: 0 to 412.3 ms (rounds to 412); userspace: 412.3 to 2600.0 ms (2187.7, rounds to 2188)
        let m = measure(&marks("412.3 bootweave-start\n2600.0 desktop-ready\n2900.0 desktop-faded\n"), Some(300)).unwrap();
        assert_eq!(m, Measured { loader_ms: Some(300), kernel_ms: 412, userspace_ms: 2188 });
    }

    #[test]
    fn measure_falls_back_to_faded_and_rejects_incomplete_marks() {
        // no desktop-ready: the end is desktop-faded, 3000.0 - 412.3 = 2587.7 rounds to 2588
        let m = measure(&marks("412.3 bootweave-start\n3000.0 desktop-faded\n"), None).unwrap();
        assert_eq!(m, Measured { loader_ms: None, kernel_ms: 412, userspace_ms: 2588 });
        assert_eq!(measure(&marks("412.3 bootweave-start\n"), None), None, "no end mark");
        assert_eq!(measure(&marks("3000.0 desktop-faded\n"), None), None, "no start mark");
        assert_eq!(measure(&marks("500.0 bootweave-start\n400.0 desktop-faded\n"), None), None, "end before start");
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
    fn the_start_mark_is_the_one_the_supervisor_writes() {
        assert_eq!(START_MARK, "bootweave-start");
        assert!(springboard::bootsplash::CONTRACT_MARKS.contains(&START_MARK));
        assert!(measure(&marks("412.3 bootweave-start\n2600.0 desktop-ready\n"), None).is_some());
    }

    #[test]
    fn the_loader_hand_off_decodes_from_an_efivarfs_image() {
        let record = Handoff { timing: BootTiming::DEFAULT, loader_ms: 240, shown_permille: 63 };
        let mut image = vec![7u8, 0, 0, 0];
        image.extend_from_slice(&record.encode());
        assert_eq!(image.len(), 36);
        assert_eq!(decode_efivar(&image), Some(record));
        assert_eq!(decode_efivar(&image[4..]), None, "without the attribute bytes the length is wrong");
        assert_eq!(decode_efivar(&[1, 2]), None);
    }

    #[test]
    fn the_settings_appearance_wins_and_auto_defers() {
        assert_eq!(appearance_from_settings("a=1\nappearance_mode=light\n"), Some(Appearance::Light));
        assert_eq!(appearance_from_settings("appearance_mode=dark\n"), Some(Appearance::Dark));
        assert_eq!(appearance_from_settings("appearance_mode=auto\n"), None);
        assert_eq!(appearance_from_settings("wallpaper=x\n"), None);
    }

    #[test]
    fn the_pixel_order_follows_the_channel_offsets() {
        assert_eq!(order_from(16, 0), PixelOrder::Bgrx);
        assert_eq!(order_from(0, 16), PixelOrder::Rgbx);
        assert_eq!(order_from(0, 0), PixelOrder::Bgrx, "unknown stays the usual order");
    }

    #[test]
    fn the_bar_gate_stops_on_the_flag_or_the_ceiling() {
        assert!(bar_runs(false, Duration::from_secs(1)));
        assert!(!bar_runs(true, Duration::from_secs(1)), "a failure screen stops it at once");
        assert!(!bar_runs(false, BAR_CEILING));
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
        assert_eq!(
            readback_meta(1920, 1080, 1920, PixelOrder::Bgrx),
            "{\"width\": 1920, \"height\": 1080, \"stride\": 1920, \"order\": \"bgrx\"}\n"
        );
        assert!(readback_meta(8, 2, 8, PixelOrder::Rgbx).contains("\"order\": \"rgbx\""));
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
