//! evdev decoding: multitouch protocol B (first contact only) and keys.

use super::pointer_finger::{Mouse, PointerFinger};
use super::pointer_prefs::{self, Secondary};
use super::sys::*;
use std::ffi::c_int;
use std::io;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Event {
    Down { x: f32, y: f32, t: f64 },
    Move { x: f32, y: f32, t: f64 },
    Up { x: f32, y: f32, t: f64 },
    Hover{x:f32,y:f32,t:f64},
    Context{x:f32,y:f32,t:f64},
    Key { code: u16, down: bool },
    /// A switch (jack insert and the like): the code and whether it is on.
    Switch { code: u16, on: bool },
    /// One finger of a multi-touch screen: the contact's id (unique among the fingers down), its phase and its
    /// position in panel pixels. Every finger produces these. The first finger of a touch sequence also produces
    /// the single-pointer `Down` / `Move` / `Up` events above, so the shell's pointer-style handling keeps working.
    Contact { id: i32, phase: TouchPhase, x: f32, y: f32, t: f64 },
}

/// Where a touch contact is in its life.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TouchPhase {
    Down,
    Move,
    Up,
    /// The system took the contact over (a system gesture, a lost grab): no tap or fling should follow.
    Cancel,
}

pub const KEY_VOLUMEDOWN: u16 = 114;
pub const KEY_VOLUMEUP: u16 = 115;
pub const KEY_POWER: u16 = 116;
pub const KEY_WAKEUP: u16 = 143;
pub use crate::media::route::{SW_HEADPHONE_INSERT, SW_MICROPHONE_INSERT};

const EV_SYN: u16 = 0;
const EV_KEY: u16 = 1;
const EV_ABS: u16 = 3;
const EV_SW: u16 = 5;
const EV_REL: u16 = 2;
const ABS_X: u16 = 0x00;
const ABS_Y: u16 = 0x01;
const ABS_MT_SLOT: u16 = 0x2f;
const ABS_MT_POSITION_X: u16 = 0x35;
const ABS_MT_POSITION_Y: u16 = 0x36;
const ABS_MT_TRACKING_ID: u16 = 0x39;
const BTN_TOUCH: u16 = 0x14a;
pub const EVENT_SIZE: usize = 24; // struct input_event on 64-bit

/// One `struct input_event` record.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RawEvent {
    pub sec: i64,
    pub usec: i64,
    pub ty: u16,
    pub code: u16,
    pub value: i32,
}

impl RawEvent {
    /// Decodes one record; None when `rec` is not a whole one.
    pub fn parse(rec: &[u8]) -> Option<RawEvent> {
        let rec: &[u8; EVENT_SIZE] = rec.try_into().ok()?;
        Some(RawEvent {
            sec: i64::from_ne_bytes(rec[0..8].try_into().ok()?),
            usec: i64::from_ne_bytes(rec[8..16].try_into().ok()?),
            ty: u16::from_ne_bytes([rec[16], rec[17]]),
            code: u16::from_ne_bytes([rec[18], rec[19]]),
            value: i32::from_ne_bytes(rec[20..24].try_into().ok()?),
        })
    }
}

/// Turns raw evdev records into touch/key events; tracks slot 0 only.
#[derive(Default)]
pub struct Decoder {
    /// device range -> panel pixels; None (the bifrost) passes raw values through
    map: Option<AbsMap>,
    /// also follow single-touch reports (ABS_X/Y + BTN_TOUCH), for desktop touchscreens
    single: bool,
    slot: i32,
    x: f32,
    y: f32,
    down: bool,
    pending_down: Option<bool>,
    moved: bool,
}

impl Decoder {
    /// A decoder for a desktop touchscreen: scaled to the panel, single-touch reports included.
    pub fn scaled(map: AbsMap) -> Self {
        Decoder { map: Some(map), single: true, ..Default::default() }
    }

    fn set_x(&mut self, v: i32) {
        self.x = self.map.map_or(v as f32, |m| m.x.apply(v));
        self.moved = true;
    }

    fn set_y(&mut self, v: i32) {
        self.y = self.map.map_or(v as f32, |m| m.y.apply(v));
        self.moved = true;
    }

    /// Feeds one record (type, code, value) at time `t` ms.
    pub fn feed(&mut self, ty: u16, code: u16, value: i32, t: f64, out: &mut Vec<Event>) {
        match (ty, code) {
            (EV_ABS, ABS_MT_SLOT) => self.slot = value,
            _ if ty == EV_ABS && self.slot != 0 => {}
            (EV_ABS, ABS_MT_TRACKING_ID) => self.pending_down = Some(value >= 0),
            (EV_ABS, ABS_MT_POSITION_X) => self.set_x(value),
            (EV_ABS, ABS_MT_POSITION_Y) => self.set_y(value),
            (EV_ABS, ABS_X) if self.single => self.set_x(value),
            (EV_ABS, ABS_Y) if self.single => self.set_y(value),
            (EV_KEY, BTN_TOUCH) => {
                if self.single {
                    self.pending_down = Some(value != 0);
                }
            }
            (EV_SW, _) => out.push(Event::Switch { code, on: value != 0 }),
            (EV_KEY, _) if value != 2 => out.push(Event::Key { code, down: value == 1 }),
            (EV_SYN, 0) => {
                let (x, y) = (self.x, self.y);
                match self.pending_down.take() {
                    Some(true) if !self.down => {
                        self.down = true;
                        out.push(Event::Down { x, y, t });
                    }
                    Some(false) if self.down => {
                        self.down = false;
                        out.push(Event::Up { x, y, t });
                    }
                    _ if self.down && self.moved => out.push(Event::Move { x, y, t }),
                    _ => {}
                }
                self.moved = false;
            }
            _ => {}
        }
    }
}

pub struct Device {
    pub fd: c_int,
    decoder: Decoder,
}

impl Device {
    /// Opens and exclusively grabs an input device.
    pub fn open(path: &str) -> io::Result<Self> {
        let fd = open_dev(path, O_RDONLY | O_NONBLOCK)?;
        let mut one: c_int = 1;
        let _ = unsafe { ioctl_ptr(fd, iow::<c_int>(b'E', 0x90), &mut one as *mut c_int as *mut ()) };
        // EVIOCSCLOCKID: stamp events with CLOCK_MONOTONIC, the clock `now_ms` reads
        let mut mono: c_int = 1;
        let _ = unsafe { ioctl_ptr(fd, iow::<c_int>(b'E', 0xa0), &mut mono as *mut c_int as *mut ()) };
        Ok(Device { fd, decoder: Decoder::default() })
    }

    /// A device with no hardware behind it (emulator, headless): the fd is -1, which
    /// the poll loop ignores; input arrives through the control socket instead.
    pub fn virtual_device() -> Self {
        Device { fd: -1, decoder: Decoder::default() }
    }

    /// Finds /dev/input/eventN by its kernel name.
    pub fn by_name(name: &str) -> Option<String> {
        (0..32)
            .map(|i| (i, std::fs::read_to_string(format!("/sys/class/input/event{i}/device/name"))))
            .find_map(|(i, n)| n.ok().filter(|n| n.trim() == name).map(|_| format!("/dev/input/event{i}")))
    }

    /// The first input device whose kernel name contains any of `hints`
    /// (lowercase), e.g. the headset jack.
    pub fn by_hint(hints: &[&str]) -> Option<String> {
        (0..32)
            .map(|i| (i, std::fs::read_to_string(format!("/sys/class/input/event{i}/device/name"))))
            .find_map(|(i, n)| n.ok().filter(|n| hints.iter().any(|h| n.to_lowercase().contains(h))).map(|_| format!("/dev/input/event{i}")))
    }

    /// The current switch bitmap (EVIOCGSW); 0 when the ioctl fails.
    pub fn switches(&self) -> u32 {
        let mut bits: u32 = 0;
        let _ = unsafe { ioctl_ptr(self.fd, ioc(2, b'E', 0x1b, std::mem::size_of::<u32>()), &mut bits) };
        bits
    }

    pub fn read(&mut self, out: &mut Vec<Event>) {
        let mut buf = [0u8; EVENT_SIZE * 64];
        loop {
            let n = unsafe { read(self.fd, buf.as_mut_ptr() as *mut _, buf.len()) };
            if n <= 0 {
                return;
            }
            for ev in buf[..n as usize].chunks_exact(EVENT_SIZE).filter_map(RawEvent::parse) {
                // the kernel's sample time keeps velocity right when events queue behind a frame;
                // a stamp from another clock (or a zero one from injection) falls back to now
                let now = now_ms();
                let stamp = ev.sec as f64 * 1e3 + ev.usec as f64 / 1e3;
                let t = if stamp <= now + 1.0 && now - stamp < 1000.0 { stamp } else { now };
                self.decoder.feed(ev.ty, ev.code, ev.value, t, out);
            }
        }
    }
}

/// Encodes one input_event record (zero timestamp; the kernel stamps it).
pub fn record(ty: u16, code: u16, value: i32) -> [u8; EVENT_SIZE] {
    let mut r = [0u8; EVENT_SIZE];
    r[16..18].copy_from_slice(&ty.to_ne_bytes());
    r[18..20].copy_from_slice(&code.to_ne_bytes());
    r[20..24].copy_from_slice(&value.to_ne_bytes());
    r
}

/// Writes a one-finger swipe (or tap when from == to) into an evdev node;
/// readers, including a grabbing one, see it as real touch input.
pub fn inject_swipe(path: &str, from: (f32, f32), to: (f32, f32), ms: f32) -> io::Result<()> {
    use std::io::Write;
    let mut f = std::fs::OpenOptions::new().write(true).open(path)?;
    let steps = ((ms / 16.0) as i32).max(1);
    let frame = |f: &mut std::fs::File, recs: &[[u8; EVENT_SIZE]]| f.write_all(&recs.concat());
    frame(
        &mut f,
        &[
            record(EV_ABS, ABS_MT_TRACKING_ID, 1),
            record(EV_ABS, ABS_MT_POSITION_X, from.0 as i32),
            record(EV_ABS, ABS_MT_POSITION_Y, from.1 as i32),
            record(EV_KEY, BTN_TOUCH, 1),
            record(EV_SYN, 0, 0),
        ],
    )?;
    for i in 1..=steps {
        let k = i as f32 / steps as f32;
        std::thread::sleep(std::time::Duration::from_millis(16));
        let (x, y) = (from.0 + (to.0 - from.0) * k, from.1 + (to.1 - from.1) * k);
        frame(&mut f, &[record(EV_ABS, ABS_MT_POSITION_X, x as i32), record(EV_ABS, ABS_MT_POSITION_Y, y as i32), record(EV_SYN, 0, 0)])?;
    }
    frame(&mut f, &[record(EV_ABS, ABS_MT_TRACKING_ID, -1), record(EV_KEY, BTN_TOUCH, 0), record(EV_SYN, 0, 0)])
}

// ---- desktop profile: capability-based discovery, several devices, pointer emulation ----

const REL_X: u16 = 0x00;
const REL_Y: u16 = 0x01;
const REL_HWHEEL: u16 = 0x06;
const REL_WHEEL: u16 = 0x08;
const BTN_LEFT: u16 = 0x110;
const BTN_RIGHT: u16 = 0x111;
const KEY_LEFTSHIFT: u16 = 42;
const KEY_RIGHTSHIFT: u16 = 54;
const BTN_TOOL_FINGER: u16 = 0x145;
const BTN_TOOL_DOUBLETAP: u16 = 0x14d;
/// a touchpad touch counts as a tap when it lifts within this many ms and moves less than `TAP_TRAVEL` px
const TAP_MS: f64 = 250.0;
const TAP_TRAVEL: f32 = 8.0;
const SW_LID: u16 = 0x00;
const INPUT_PROP_DIRECT: u16 = 0x01;
/// pointer speed of an absolute touchpad, relative to its span covering the panel
const TOUCHPAD_GAIN: f32 = 1.5;
/// Fallback poll when inotify on /dev/input is unavailable.
const RESCAN_MS: f64 = 2000.0;
const IN_NONBLOCK: c_int = 0o4000;
const IN_WATCH_MASK: u32 = 0x4 | 0x100 | 0x200; // ATTRIB | CREATE | DELETE
extern "C" {
    fn inotify_init1(flags: c_int) -> c_int;
    fn inotify_add_watch(fd: c_int, path: *const std::ffi::c_char, mask: u32) -> c_int;
}

/// Bit `n` of an evdev capability bitmap (little-endian bytes, as EVIOCGBIT returns them).
pub fn has_bit(bitmap: &[u8], n: u16) -> bool {
    bitmap.get(n as usize / 8).is_some_and(|b| b >> (n % 8) & 1 == 1)
}

/// One axis' range as EVIOCGABS reports it (`struct input_absinfo`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AbsInfo {
    pub min: i32,
    pub max: i32,
}

impl AbsInfo {
    pub const SIZE: usize = 24;

    /// Reads value, minimum, maximum, fuzz, flat, resolution; None when `rec` is not a whole one.
    pub fn parse(rec: &[u8]) -> Option<AbsInfo> {
        let rec: &[u8; Self::SIZE] = rec.try_into().ok()?;
        let word = |i: usize| i32::from_ne_bytes([rec[i * 4], rec[i * 4 + 1], rec[i * 4 + 2], rec[i * 4 + 3]]);
        Some(AbsInfo { min: word(1), max: word(2) })
    }
}

/// A device axis scaled onto `0..out` panel pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Axis {
    pub range: AbsInfo,
    pub out: f32,
}

impl Axis {
    pub fn apply(&self, v: i32) -> f32 {
        let span = (self.range.max - self.range.min).max(1) as f32;
        ((v - self.range.min) as f32 / span).clamp(0.0, 1.0) * (self.out - 1.0).max(0.0)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AbsMap {
    pub x: Axis,
    pub y: Axis,
}

/// The capability bitmaps of one device (EVIOCGBIT per type, EVIOCGPROP).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Caps {
    pub props: Vec<u8>,
    pub key: Vec<u8>,
    pub rel: Vec<u8>,
    pub abs: Vec<u8>,
    pub sw: Vec<u8>,
}

/// What a device is for. A device can be several (a keyboard that is also a power button).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Roles {
    /// direct-input absolute device: a touchscreen
    pub touchscreen: bool,
    /// mouse, absolute touchpad or bifrost: moves the pointer
    pub pointer: bool,
    /// an absolute pointer that moves it by finger deltas
    pub touchpad: bool,
    pub keyboard: bool,
    pub power: bool,
    pub lid: bool,
}

/// KEY_Q..KEY_P, KEY_A..KEY_L, KEY_Z..KEY_M: the letter keys, which no non-keyboard has all of.
const LETTER_KEYS: [std::ops::RangeInclusive<u16>; 3] = [16..=25, 30..=38, 44..=50];

impl Caps {
    fn abs_pair(&self, x: u16, y: u16) -> bool {
        has_bit(&self.abs, x) && has_bit(&self.abs, y)
    }

    pub fn roles(&self) -> Roles {
        let direct = has_bit(&self.props, INPUT_PROP_DIRECT);
        let mt = self.abs_pair(ABS_MT_POSITION_X, ABS_MT_POSITION_Y);
        let st = self.abs_pair(ABS_X, ABS_Y);
        let touch_key = has_bit(&self.key, BTN_TOUCH);
        let click = has_bit(&self.key, BTN_LEFT);
        let touchscreen = direct && (mt || st && touch_key);
        let relative = has_bit(&self.rel, REL_X) && has_bit(&self.rel, REL_Y) && click;
        let finger_pad = !direct && st && has_bit(&self.key, BTN_TOOL_FINGER);
        let absolute = !direct && (st || mt) && (click || touch_key);
        Roles {
            touchscreen,
            pointer: !touchscreen && (relative || absolute),
            touchpad: !touchscreen && !relative && finger_pad,
            keyboard: LETTER_KEYS.iter().all(|r| r.clone().all(|k| has_bit(&self.key, k))),
            power: has_bit(&self.key, KEY_POWER),
            lid: has_bit(&self.sw, SW_LID),
        }
    }
}

impl Roles {
    /// Power and lid devices stay shared with logind (it owns suspend and shutdown).
    fn grabbable(&self) -> bool {
        !self.power && !self.lid
    }
}

/// The shell's pointer: where it is on the panel. Relative devices integrate into it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pointer {
    pub x: f32,
    pub y: f32,
    panel: (f32, f32),
}

impl Pointer {
    pub fn new(w: f32, h: f32) -> Self {
        Pointer { x: w / 2.0, y: h / 2.0, panel: (w, h) }
    }

    fn move_by(&mut self, dx: f32, dy: f32) {
        self.x = (self.x + dx).clamp(0.0, (self.panel.0 - 1.0).max(0.0));
        self.y = (self.y + dy).clamp(0.0, (self.panel.1 - 1.0).max(0.0));
    }
}

/// Turns a mouse, absolute pointer or touchpad into touch events at the pointer through a
/// `PointerFinger`: the left button (or tap on a bifrost) is the finger, the right button a held
/// finger (long press), the wheel an eased drag.
#[derive(Default)]
pub struct PointerDecoder {
    map: Option<AbsMap>,
    touchpad: bool,
    last: (Option<i32>, Option<i32>),
    dx: f32,
    dy: f32,
    wheel: i32,
    hwheel: i32,
    pending: Option<bool>,
    pending_right: Option<bool>,
    moved: bool,
    shift: bool,
    finger: Option<PointerFinger>,
    /// finger travel since the touch began, in panel px (tap to click)
    travel: f32,
    /// when the current touchpad touch began, in ms
    tap_started: Option<f64>,
    /// a physical button went down during this touch, so it is not a tap
    tap_clicked: bool,
    /// a tap's press is pending; its release follows in the same sync
    tap_release: bool,
    /// two fingers are down on the touchpad (BTN_TOOL_DOUBLETAP)
    two_fingers: bool,
    /// the touchpad press in progress is a secondary click
    click_right: bool,
}

impl PointerDecoder {
    /// `map` places an absolute device on the panel; None for a mouse.
    pub fn new(map: Option<AbsMap>, touchpad: bool) -> Self {
        PointerDecoder { map, touchpad, ..Default::default() }
    }

    /// Whether a shift key is held (it turns the wheel sideways).
    pub fn set_shift(&mut self, on: bool) {
        self.shift = on;
    }

    /// The left button is held.
    pub fn is_down(&self) -> bool {
        self.finger.as_ref().is_some_and(PointerFinger::is_down)
    }

    /// Advances the finger's wheel ease and right-click hold up to `t`.
    pub fn tick(&mut self, t: f64, out: &mut Vec<Event>) {
        if let Some(f) = &mut self.finger {
            f.tick(t, out);
        }
    }

    /// When `tick` next has work (ms).
    pub fn deadline(&self) -> Option<f64> {
        self.finger.as_ref().and_then(PointerFinger::deadline)
    }

    pub fn feed(&mut self, ptr: &mut Pointer, ty: u16, code: u16, value: i32, t: f64, out: &mut Vec<Event>) {
        match (ty, code) {
            (EV_REL, REL_X) => self.dx += value as f32,
            (EV_REL, REL_Y) => self.dy += value as f32,
            (EV_REL, REL_WHEEL) => self.wheel += value,
            (EV_REL, REL_HWHEEL) => self.hwheel += value,
            (EV_ABS, ABS_X | ABS_Y) => self.abs(ptr, code == ABS_X, value),
            (EV_KEY, BTN_TOUCH) if self.touchpad => self.touch(value == 1, t),
            (EV_KEY, BTN_TOUCH) if value != 2 => self.button(ptr, true, value == 1),
            (EV_KEY, BTN_TOOL_DOUBLETAP) => self.two_fingers = value != 0,
            (EV_KEY, BTN_LEFT | BTN_RIGHT) if value != 2 => self.button(ptr, code == BTN_LEFT, value == 1),
            (EV_SYN, 0) => self.sync(ptr, t, out),
            _ => {}
        }
    }

    /// A touchpad touch began or lifted. A quick touch with little travel and no button counts as a tap
    /// when tap to click is on; the tap's press and release go out in the next sync.
    fn touch(&mut self, down: bool, t: f64) {
        self.last = (None, None);
        if down {
            self.tap_started = Some(t);
            self.travel = 0.0;
            self.tap_clicked = false;
            return;
        }
        let started = self.tap_started.take();
        let tap = pointer_prefs::get().tap_to_click && !self.tap_clicked && self.travel < TAP_TRAVEL && started.is_some_and(|s| t - s <= TAP_MS);
        if tap {
            self.pending = Some(true);
            self.tap_release = true;
        }
    }

    /// A button changed. On a touchpad the left button is the secondary click when the setting says so
    /// (a press in the bottom-right corner, or with two fingers down). The mouse's buttons trade places when
    /// Swap Buttons is on.
    fn button(&mut self, ptr: &Pointer, left: bool, down: bool) {
        let cfg = pointer_prefs::get();
        let right = if self.touchpad && left {
            if down {
                self.tap_clicked = true;
                self.click_right = match cfg.secondary {
                    Secondary::TwoFingers => self.two_fingers,
                    Secondary::Corner => ptr.x >= ptr.panel.0 * 0.8 && ptr.y >= ptr.panel.1 * 0.8,
                };
            }
            self.click_right
        } else {
            !left
        };
        let right = right != (cfg.swap_buttons && !self.touchpad);
        if right {
            self.pending_right = Some(down);
        } else {
            self.pending = Some(down);
        }
    }

    fn abs(&mut self, ptr: &mut Pointer, is_x: bool, v: i32) {
        let Some(map) = self.map else { return };
        if !self.touchpad {
            if is_x {
                ptr.x = map.x.apply(v);
            } else {
                ptr.y = map.y.apply(v);
            }
            self.moved = true;
            return;
        }
        // a touchpad: finger travel, relative to where the finger was
        let (slot, axis) = if is_x { (&mut self.last.0, map.x) } else { (&mut self.last.1, map.y) };
        let span = (axis.range.max - axis.range.min).max(1) as f32;
        let d = slot.replace(v).map_or(0.0, |p| (v - p) as f32 * axis.out / span * TOUCHPAD_GAIN);
        self.travel += d.abs();
        if is_x {
            self.dx += d;
        } else {
            self.dy += d;
        }
    }

    fn sync(&mut self, ptr: &mut Pointer, t: f64, out: &mut Vec<Event>) {
        let cfg = pointer_prefs::get();
        if self.dx != 0.0 || self.dy != 0.0 {
            let k = if self.touchpad { cfg.trackpad_speed } else { cfg.mouse_speed };
            ptr.move_by(self.dx * k, self.dy * k);
            self.moved = true;
        }
        let at = (ptr.x, ptr.y);
        let shift = self.shift;
        let finger = self.finger.get_or_insert_with(|| PointerFinger::new(ptr.panel.0, ptr.panel.1));
        if let Some(down) = self.pending.take() {
            finger.feed(t, at, shift, Mouse::Left(down), out);
        } else if self.moved {
            finger.feed(t, at, shift, Mouse::Moved, out);
        }
        if std::mem::take(&mut self.tap_release) {
            finger.feed(t, at, shift, Mouse::Left(false), out);
        }
        if let Some(down) = self.pending_right.take() {
            finger.feed(t, at, shift, Mouse::Right(down), out);
        }
        if self.wheel != 0 || self.hwheel != 0 {
            let v = pointer_prefs::scroll_step(self.wheel, cfg.mouse_scroll, cfg.natural_scroll);
            let h = pointer_prefs::scroll_step(self.hwheel, cfg.mouse_scroll, cfg.natural_scroll);
            finger.feed(t, at, shift, Mouse::Wheel { h, v }, out);
        }
        (self.dx, self.dy, self.wheel, self.hwheel, self.moved) = (0.0, 0.0, 0, 0, false);
    }
}

enum Source {
    Touch(Decoder),
    Pointer(PointerDecoder),
    /// keyboard, power button, lid switch
    Keys(Decoder),
}

struct Slot {
    path: String,
    fd: c_int,
    source: Source,
}

/// Every input device of a desktop, found by capability and kept current as devices come and go.
pub struct InputSet {
    panel: (f32, f32),
    /// grab the devices that are ours alone (never power buttons or lids)
    exclusive: bool,
    slots: Vec<Slot>,
    /// paths already probed and not wanted, so a scan does not reopen them
    ignored: Vec<String>,
    next_scan: f64,
    /// inotify fd on /dev/input (-1: not started, or unavailable and the timer rescans)
    watch: c_int,
    pointer: Pointer,
    /// shift keys held on any keyboard, for the wheel
    shift: u8,
}

fn ioctl_bits(fd: c_int, ev: u16, bytes: usize) -> Vec<u8> {
    let mut buf = vec![0u8; bytes];
    let _ = unsafe { ioctl_ptr(fd, ioc(2, b'E', 0x20 + ev as u8, bytes), buf.as_mut_ptr()) };
    buf
}

fn read_caps(fd: c_int) -> Caps {
    let mut props = vec![0u8; 4];
    let _ = unsafe { ioctl_ptr(fd, ioc(2, b'E', 0x09, 4), props.as_mut_ptr()) };
    Caps { props, key: ioctl_bits(fd, EV_KEY, 96), rel: ioctl_bits(fd, EV_REL, 2), abs: ioctl_bits(fd, EV_ABS, 8), sw: ioctl_bits(fd, EV_SW, 2) }
}

fn read_abs(fd: c_int, axis: u16) -> Option<AbsInfo> {
    let mut buf = [0u8; AbsInfo::SIZE];
    unsafe { ioctl_ptr(fd, ioc(2, b'E', 0x40 + axis as u8, AbsInfo::SIZE), buf.as_mut_ptr()) }.ok()?;
    AbsInfo::parse(&buf)
}

/// The panel mapping of an absolute device, from its multitouch axes or else its single-touch ones.
fn read_map(fd: c_int, caps: &Caps, panel: (f32, f32)) -> Option<AbsMap> {
    let (xa, ya) = if has_bit(&caps.abs, ABS_MT_POSITION_X) { (ABS_MT_POSITION_X, ABS_MT_POSITION_Y) } else { (ABS_X, ABS_Y) };
    Some(AbsMap { x: Axis { range: read_abs(fd, xa)?, out: panel.0 }, y: Axis { range: read_abs(fd, ya)?, out: panel.1 } })
}

impl InputSet {
    /// An empty set that never scans (bifrost and emulator: their devices are opened by name).
    pub fn none() -> Self {
        Self::desktop(0.0, 0.0, false)
    }

    /// A desktop set for a panel of `w` x `h`; `exclusive` grabs keyboards, pointers and touchscreens.
    pub fn desktop(w: f32, h: f32, exclusive: bool) -> Self {
        InputSet { panel: (w, h), exclusive, slots: vec![], ignored: vec![], next_scan: 0.0, watch: -1, pointer: Pointer::new(w, h), shift: 0 }
    }

    /// Where the pointer is on the panel, for a cursor to be drawn there.
    pub fn pointer(&self) -> (f32, f32) {
        (self.pointer.x, self.pointer.y)
    }

    /// A pointing device is present, so a cursor is worth drawing.
    pub fn has_pointer(&self) -> bool {
        self.slots.iter().any(|s| matches!(s.source, Source::Pointer(_)))
    }

    /// A pointer's left button is held.
    pub fn is_down(&self) -> bool {
        self.slots.iter().any(|s| matches!(&s.source, Source::Pointer(d) if d.is_down()))
    }

    /// Advances the pointers' wheel eases and right-click holds up to `t`.
    pub fn tick(&mut self, t: f64, out: &mut Vec<Event>) {
        for s in &mut self.slots {
            if let Source::Pointer(d) = &mut s.source {
                d.tick(t, out);
            }
        }
    }

    /// When `tick` next has work (ms), for the poll timeout.
    pub fn deadline(&self) -> Option<f64> {
        self.slots.iter().filter_map(|s| if let Source::Pointer(d) = &s.source { d.deadline() } else { None }).reduce(f64::min)
    }

    /// The fds to poll.
    pub fn fds(&self) -> impl Iterator<Item = c_int> + '_ {
        self.slots.iter().map(|s| s.fd).chain((self.watch >= 0).then_some(self.watch))
    }

    /// When the next rescan is due (ms), for the poll timeout; None for the empty set.
    pub fn next_scan(&self) -> Option<f64> {
        (self.panel.0 > 0.0 && self.watch < 0).then_some(self.next_scan)
    }

    /// Rescans /dev/input when due: opens new devices and drops vanished ones.
    pub fn rescan_if_due(&mut self, now: f64) {
        if self.panel.0 <= 0.0 {
            return;
        }
        if self.watch >= 0 {
            // hot-plug is event-driven: rescan only when inotify reported a change
            let mut buf = [0u8; 1024];
            let mut changed = false;
            while unsafe { read(self.watch, buf.as_mut_ptr() as *mut _, buf.len()) } > 0 {
                changed = true;
            }
            if changed {
                self.rescan();
            }
            return;
        }
        if now < self.next_scan {
            return;
        }
        self.next_scan = now + RESCAN_MS;
        if self.slots.is_empty() && self.ignored.is_empty() {
            // the one start-up scan arms the watch; without inotify the timer keeps rescanning
            let fd = unsafe { inotify_init1(IN_NONBLOCK | 0o2000000) };
            if fd >= 0 {
                if unsafe { inotify_add_watch(fd, c"/dev/input".as_ptr(), IN_WATCH_MASK) } >= 0 {
                    self.watch = fd;
                } else {
                    unsafe { close(fd) };
                }
            }
        }
        self.rescan();
    }

    fn rescan(&mut self) {
        let mut present: Vec<String> = std::fs::read_dir("/dev/input")
            .into_iter()
            .flatten()
            .filter_map(|e| e.ok()?.file_name().to_str().filter(|n| n.starts_with("event")).map(|n| format!("/dev/input/{n}")))
            .collect();
        present.sort();
        self.ignored.retain(|p| present.contains(p));
        self.slots.retain(|s| {
            let keep = present.contains(&s.path);
            if !keep {
                unsafe { close(s.fd) };
            }
            keep
        });
        for path in present {
            if self.slots.iter().any(|s| s.path == path) || self.ignored.contains(&path) {
                continue;
            }
            match self.open(&path) {
                Some(slot) => self.slots.push(slot),
                None => self.ignored.push(path),
            }
        }
    }

    fn open(&self, path: &str) -> Option<Slot> {
        let fd = open_dev(path, O_RDONLY | O_NONBLOCK).ok()?;
        let caps = read_caps(fd);
        let roles = caps.roles();
        let map = read_map(fd, &caps, self.panel);
        let source = match (roles, map) {
            (r, Some(map)) if r.touchscreen => Source::Touch(Decoder::scaled(map)),
            (r, map) if r.pointer => Source::Pointer(PointerDecoder::new(map, r.touchpad)),
            (r, _) if r.keyboard || r.power || r.lid => Source::Keys(Decoder::default()),
            _ => {
                unsafe { close(fd) };
                return None;
            }
        };
        if self.exclusive && roles.grabbable() {
            let mut one: c_int = 1;
            let _ = unsafe { ioctl_ptr(fd, iow::<c_int>(b'E', 0x90), &mut one as *mut c_int as *mut ()) };
        }
        // stamp events with CLOCK_MONOTONIC, the clock `now_ms` reads
        let mut mono: c_int = 1;
        let _ = unsafe { ioctl_ptr(fd, iow::<c_int>(b'E', 0xa0), &mut mono as *mut c_int as *mut ()) };
        Some(Slot { path: path.to_string(), fd, source })
    }

    /// Drains every device into `out`; a device that was unplugged is closed and forgotten.
    pub fn read(&mut self, out: &mut Vec<Event>) {
        let pointer = &mut self.pointer;
        let mut shift = self.shift;
        self.slots.retain_mut(|slot| {
            let mut buf = [0u8; EVENT_SIZE * 64];
            loop {
                let n = unsafe { read(slot.fd, buf.as_mut_ptr() as *mut _, buf.len()) };
                if n < 0 && io::Error::last_os_error().kind() != io::ErrorKind::WouldBlock {
                    unsafe { close(slot.fd) };
                    return false;
                }
                if n <= 0 {
                    return true;
                }
                for ev in buf[..n as usize].chunks_exact(EVENT_SIZE).filter_map(RawEvent::parse) {
                    let t = event_time(&ev);
                    if ev.ty == 1 && ev.value == 1 {
                        super::chord_recovery::note_key(ev.code);
                    }
                    match &mut slot.source {
                        Source::Touch(d) | Source::Keys(d) => {
                            let from = out.len();
                            d.feed(ev.ty, ev.code, ev.value, t, out);
                            for e in &out[from..] {
                                if let Event::Key { code: KEY_LEFTSHIFT | KEY_RIGHTSHIFT, down } = *e {
                                    shift = if down { shift.saturating_add(1) } else { shift.saturating_sub(1) };
                                }
                            }
                        }
                        Source::Pointer(d) => {
                            d.set_shift(shift > 0);
                            d.feed(pointer, ev.ty, ev.code, ev.value, t, out);
                        }
                    }
                }
            }
        });
        self.shift = shift;
    }
}

impl Drop for InputSet {
    fn drop(&mut self) {
        for s in &self.slots {
            unsafe { close(s.fd) };
        }
    }
}

/// The kernel's sample time keeps velocity right when events queue behind a frame;
/// a stamp from another clock (or a zero one from injection) falls back to now.
fn event_time(ev: &RawEvent) -> f64 {
    let now = now_ms();
    let stamp = ev.sec as f64 * 1e3 + ev.usec as f64 / 1e3;
    if stamp <= now + 1.0 && now - stamp < 1000.0 {
        stamp
    } else {
        now
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A capability bitmap with the given bits set, `len` bytes long.
    fn bits(len: usize, set: &[u16]) -> Vec<u8> {
        let mut b = vec![0u8; len];
        for &n in set {
            b[n as usize / 8] |= 1 << (n % 8);
        }
        b
    }

    fn letters() -> Vec<u16> {
        LETTER_KEYS.iter().flat_map(|r| r.clone()).collect()
    }

    fn panel_map() -> AbsMap {
        AbsMap { x: Axis { range: AbsInfo { min: 0, max: 4095 }, out: 1000.0 }, y: Axis { range: AbsInfo { min: 0, max: 2047 }, out: 500.0 } }
    }

    #[test]
    fn has_bit_reads_little_endian_bitmaps() {
        let b = [0b0000_0101, 0b1000_0000];
        assert!(has_bit(&b, 0) && !has_bit(&b, 1) && has_bit(&b, 2) && has_bit(&b, 15));
        assert!(!has_bit(&b, 16), "past the end is absent");
    }

    #[test]
    fn absinfo_parses_min_and_max() {
        let mut rec = [0u8; AbsInfo::SIZE];
        rec[4..8].copy_from_slice(&(-5i32).to_ne_bytes());
        rec[8..12].copy_from_slice(&1919i32.to_ne_bytes());
        assert_eq!(AbsInfo::parse(&rec), Some(AbsInfo { min: -5, max: 1919 }));
        assert_eq!(AbsInfo::parse(&rec[..23]), None);
    }

    #[test]
    fn axis_scales_onto_the_panel_and_clamps() {
        let a = Axis { range: AbsInfo { min: 100, max: 1100 }, out: 801.0 };
        assert_eq!(a.apply(100), 0.0);
        assert_eq!(a.apply(1100), 800.0);
        assert_eq!(a.apply(600), 400.0);
        assert_eq!(a.apply(5000), 800.0);
        assert_eq!(a.apply(-9), 0.0);
    }

    #[test]
    fn a_direct_multitouch_device_is_a_touchscreen() {
        let caps = Caps { props: bits(4, &[INPUT_PROP_DIRECT]), key: bits(96, &[BTN_TOUCH]), abs: bits(8, &[ABS_MT_POSITION_X, ABS_MT_POSITION_Y]), ..Default::default() };
        let r = caps.roles();
        assert!(r.touchscreen && !r.pointer && !r.keyboard);
    }

    #[test]
    fn a_direct_single_touch_device_needs_btn_touch() {
        let abs = bits(8, &[ABS_X, ABS_Y]);
        let with = Caps { props: bits(4, &[INPUT_PROP_DIRECT]), key: bits(96, &[BTN_TOUCH]), abs: abs.clone(), ..Default::default() };
        let without = Caps { props: bits(4, &[INPUT_PROP_DIRECT]), abs, ..Default::default() };
        assert!(with.roles().touchscreen);
        assert!(!without.roles().touchscreen);
    }

    #[test]
    fn a_mouse_is_a_pointer_not_a_touchpad() {
        let caps = Caps { key: bits(96, &[BTN_LEFT]), rel: bits(2, &[REL_X, REL_Y, REL_WHEEL]), ..Default::default() };
        let r = caps.roles();
        assert!(r.pointer && !r.touchpad && !r.touchscreen);
    }

    #[test]
    fn an_indirect_absolute_finger_device_is_a_touchpad_pointer() {
        let caps = Caps { key: bits(96, &[BTN_LEFT, BTN_TOUCH, BTN_TOOL_FINGER]), abs: bits(8, &[ABS_X, ABS_Y, ABS_MT_POSITION_X, ABS_MT_POSITION_Y]), ..Default::default() };
        let r = caps.roles();
        assert!(r.pointer && r.touchpad && !r.touchscreen);
    }

    #[test]
    fn a_pen_bifrost_is_an_absolute_pointer() {
        let caps = Caps { key: bits(96, &[BTN_TOUCH]), abs: bits(8, &[ABS_X, ABS_Y]), ..Default::default() };
        let r = caps.roles();
        assert!(r.pointer && !r.touchpad);
    }

    #[test]
    fn a_keyboard_has_every_letter_key() {
        let full = Caps { key: bits(96, &letters()), ..Default::default() };
        assert!(full.roles().keyboard);
        let mut some = letters();
        some.pop();
        assert!(!Caps { key: bits(96, &some), ..Default::default() }.roles().keyboard);
        // a media-key remote is not a keyboard
        assert!(!Caps { key: bits(96, &[KEY_VOLUMEUP, KEY_VOLUMEDOWN]), ..Default::default() }.roles().keyboard);
    }

    #[test]
    fn power_button_and_lid_are_found_and_never_grabbed() {
        let power = Caps { key: bits(96, &[KEY_POWER]), ..Default::default() }.roles();
        let lid = Caps { sw: bits(2, &[SW_LID]), ..Default::default() }.roles();
        assert!(power.power && !power.grabbable());
        assert!(lid.lid && !lid.grabbable());
        assert!(Caps { key: bits(96, &letters()), ..Default::default() }.roles().grabbable());
    }

    #[test]
    fn an_unrelated_device_has_no_role() {
        assert_eq!(Caps::default().roles(), Roles::default());
    }

    fn mouse(events: &[(u16, u16, i32)]) -> (Vec<Event>, Pointer) {
        let mut d = PointerDecoder::new(None, false);
        let mut p = Pointer::new(100.0, 50.0);
        let mut out = vec![];
        for &(ty, c, v) in events {
            d.feed(&mut p, ty, c, v, 0.0, &mut out);
        }
        (out, p)
    }

    #[test]
    fn relative_motion_integrates_and_clamps_to_the_panel() {
        let (out, p) = mouse(&[(EV_REL, REL_X, 10), (EV_REL, REL_Y, -5), (EV_SYN, 0, 0)]);
        assert!(out.is_empty(), "motion alone draws nothing and sends no touch");
        assert_eq!((p.x, p.y), (60.0, 20.0));
        let (_, p) = mouse(&[(EV_REL, REL_X, 9999), (EV_REL, REL_Y, -9999), (EV_SYN, 0, 0)]);
        assert_eq!((p.x, p.y), (99.0, 0.0));
    }

    #[test]
    fn mouse_button_press_drag_release_is_a_touch() {
        let (out, _) = mouse(&[
            (EV_KEY, BTN_LEFT, 1),
            (EV_SYN, 0, 0),
            (EV_REL, REL_X, 4),
            (EV_SYN, 0, 0),
            (EV_KEY, BTN_LEFT, 0),
            (EV_SYN, 0, 0),
        ]);
        assert_eq!(
            out,
            vec![Event::Down { x: 50.0, y: 25.0, t: 0.0 }, Event::Move { x: 54.0, y: 25.0, t: 0.0 }, Event::Up { x: 54.0, y: 25.0, t: 0.0 }]
        );
    }

    #[test]
    fn the_wheel_drags_a_finger_at_the_pointer_and_lifts_when_idle() {
        let mut d = PointerDecoder::new(None, false);
        let mut p = Pointer::new(100.0, 50.0);
        let mut out = vec![];
        d.feed(&mut p, EV_REL, REL_WHEEL, 1, 0.0, &mut out);
        d.feed(&mut p, EV_SYN, 0, 0, 0.0, &mut out);
        assert_eq!(out, vec![Event::Down { x: 50.0, y: 25.0, t: 0.0 }]);
        d.tick(48.0, &mut out);
        assert!(matches!(out.last(), Some(Event::Move { y, .. }) if *y > 25.0), "wheel up drags down");
        d.tick(200.0, &mut out);
        assert!(matches!(out.last(), Some(Event::Up { .. })));
        assert_eq!(d.deadline(), None);
    }

    #[test]
    fn the_right_button_holds_the_finger_for_a_long_press() {
        let mut d = PointerDecoder::new(None, false);
        let mut p = Pointer::new(100.0, 50.0);
        let mut out = vec![];
        d.feed(&mut p, EV_KEY, BTN_RIGHT, 1, 0.0, &mut out);
        d.feed(&mut p, EV_SYN, 0, 0, 0.0, &mut out);
        d.feed(&mut p, EV_KEY, BTN_RIGHT, 0, 10.0, &mut out);
        d.feed(&mut p, EV_SYN, 0, 0, 10.0, &mut out);
        assert_eq!(out.len(), 1, "released early: the finger stays down");
        assert!(d.deadline().is_some());
        d.tick(1000.0, &mut out);
        assert!(matches!(out.last(), Some(Event::Up { .. })));
    }

    #[test]
    fn an_absolute_pointer_lands_where_it_points() {
        let mut d = PointerDecoder::new(Some(panel_map()), false);
        let mut p = Pointer::new(1000.0, 500.0);
        let mut out = vec![];
        for (c, v) in [(ABS_X, 4095), (ABS_Y, 0)] {
            d.feed(&mut p, EV_ABS, c, v, 0.0, &mut out);
        }
        d.feed(&mut p, EV_KEY, BTN_TOUCH, 1, 0.0, &mut out);
        d.feed(&mut p, EV_SYN, 0, 0, 0.0, &mut out);
        assert_eq!(out, vec![Event::Down { x: 999.0, y: 0.0, t: 0.0 }]);
    }

    #[test]
    fn a_touchpad_moves_by_finger_travel_not_position() {
        let mut d = PointerDecoder::new(Some(panel_map()), true);
        let mut p = Pointer::new(1000.0, 500.0);
        let mut out = vec![];
        fn frame(d: &mut PointerDecoder, p: &mut Pointer, out: &mut Vec<Event>, x: i32) {
            d.feed(p, EV_ABS, ABS_X, x, 0.0, out);
            d.feed(p, EV_SYN, 0, 0, 0.0, out);
        }
        frame(&mut d, &mut p, &mut out, 1000);
        assert_eq!(p.x, 500.0, "first contact only anchors");
        frame(&mut d, &mut p, &mut out, 1000 + 410);
        assert!((p.x - (500.0 + 410.0 * 1000.0 / 4095.0 * TOUCHPAD_GAIN)).abs() < 0.5);
        let x = p.x;
        d.feed(&mut p, EV_KEY, BTN_TOUCH, 0, 0.0, &mut out);
        frame(&mut d, &mut p, &mut out, 3000);
        assert_eq!(p.x, x, "a new touch anchors again");
    }

    #[test]
    fn a_scaled_decoder_maps_multitouch_and_single_touch_to_the_panel() {
        let mut d = Decoder::scaled(panel_map());
        let mut out = vec![];
        for (ty, c, v) in [(EV_ABS, ABS_MT_TRACKING_ID, 3), (EV_ABS, ABS_MT_POSITION_X, 4095), (EV_ABS, ABS_MT_POSITION_Y, 2047), (EV_KEY, BTN_TOUCH, 1), (EV_SYN, 0, 0)] {
            d.feed(ty, c, v, 1.0, &mut out);
        }
        assert_eq!(out, vec![Event::Down { x: 999.0, y: 499.0, t: 1.0 }]);
        // a single-touch-only screen: BTN_TOUCH alone starts and ends the contact
        let mut d = Decoder::scaled(panel_map());
        let mut out = vec![];
        for (ty, c, v) in [(EV_ABS, ABS_X, 0), (EV_ABS, ABS_Y, 0), (EV_KEY, BTN_TOUCH, 1), (EV_SYN, 0, 0), (EV_KEY, BTN_TOUCH, 0), (EV_SYN, 0, 0)] {
            d.feed(ty, c, v, 2.0, &mut out);
        }
        assert_eq!(out, vec![Event::Down { x: 0.0, y: 0.0, t: 2.0 }, Event::Up { x: 0.0, y: 0.0, t: 2.0 }]);
    }

    #[test]
    fn the_bifrost_decoder_still_passes_raw_values_and_ignores_btn_touch() {
        let ev = run(&[(EV_ABS, ABS_X, 7), (EV_KEY, BTN_TOUCH, 1), (EV_SYN, 0, 0)]);
        assert!(ev.is_empty());
    }

    #[test]
    fn an_empty_set_never_scans_and_has_no_fds() {
        let mut s = InputSet::none();
        assert_eq!(s.next_scan(), None);
        s.rescan_if_due(1e9);
        assert_eq!(s.fds().count(), 0);
        let mut out = vec![];
        s.read(&mut out);
        assert!(out.is_empty());
    }

    #[test]
    fn virtual_device_never_reports() {
        let mut d = Device::virtual_device();
        assert_eq!(d.fd, -1);
        assert_eq!(d.switches(), 0);
        let mut out = vec![];
        d.read(&mut out);
        assert!(out.is_empty());
    }

    fn run(records: &[(u16, u16, i32)]) -> Vec<Event> {
        let mut d = Decoder::default();
        let mut out = vec![];
        for (i, &(t, c, v)) in records.iter().enumerate() {
            d.feed(t, c, v, i as f64, &mut out);
        }
        out
    }

    #[test]
    fn tap_produces_down_then_up() {
        let ev = run(&[
            (EV_ABS, ABS_MT_TRACKING_ID, 7),
            (EV_ABS, ABS_MT_POSITION_X, 100),
            (EV_ABS, ABS_MT_POSITION_Y, 200),
            (EV_KEY, BTN_TOUCH, 1),
            (EV_SYN, 0, 0),
            (EV_ABS, ABS_MT_TRACKING_ID, -1),
            (EV_KEY, BTN_TOUCH, 0),
            (EV_SYN, 0, 0),
        ]);
        assert_eq!(ev.len(), 2);
        assert!(matches!(ev[0], Event::Down { x, y, .. } if x == 100.0 && y == 200.0));
        assert!(matches!(ev[1], Event::Up { x, y, .. } if x == 100.0 && y == 200.0));
    }

    #[test]
    fn drag_reports_moves_and_ignores_second_finger() {
        let ev = run(&[
            (EV_ABS, ABS_MT_TRACKING_ID, 1),
            (EV_ABS, ABS_MT_POSITION_X, 10),
            (EV_ABS, ABS_MT_POSITION_Y, 10),
            (EV_SYN, 0, 0),
            (EV_ABS, ABS_MT_POSITION_X, 30),
            (EV_SYN, 0, 0),
            (EV_ABS, ABS_MT_SLOT, 1),
            (EV_ABS, ABS_MT_TRACKING_ID, 2),
            (EV_ABS, ABS_MT_POSITION_X, 900),
            (EV_SYN, 0, 0),
            (EV_ABS, ABS_MT_SLOT, 0),
            (EV_ABS, ABS_MT_POSITION_Y, 50),
            (EV_SYN, 0, 0),
        ]);
        assert_eq!(ev.len(), 3);
        assert!(matches!(ev[1], Event::Move { x, y, .. } if x == 30.0 && y == 10.0));
        assert!(matches!(ev[2], Event::Move { x, y, .. } if x == 30.0 && y == 50.0));
    }

    #[test]
    fn injected_records_decode_like_real_ones() {
        let mut d = Decoder::default();
        let mut out = vec![];
        for r in [record(EV_ABS, ABS_MT_TRACKING_ID, 1), record(EV_ABS, ABS_MT_POSITION_X, 5), record(EV_ABS, ABS_MT_POSITION_Y, 6), record(EV_SYN, 0, 0)] {
            let ev = RawEvent::parse(&r).unwrap();
            d.feed(ev.ty, ev.code, ev.value, 0.0, &mut out);
        }
        assert!(matches!(out[0], Event::Down { x, y, .. } if x == 5.0 && y == 6.0));
    }

    #[test]
    fn a_record_parses_whole_or_not_at_all() {
        let r = record(EV_KEY, KEY_POWER, 1);
        assert_eq!(RawEvent::parse(&r), Some(RawEvent { sec: 0, usec: 0, ty: EV_KEY, code: KEY_POWER, value: 1 }));
        assert_eq!(RawEvent::parse(&r[..EVENT_SIZE - 1]), None);
        assert_eq!(RawEvent::parse(&[]), None);
    }

    #[test]
    fn switch_records_become_switch_events() {
        let ev = run(&[(EV_SW, SW_HEADPHONE_INSERT, 1), (EV_SYN, 0, 0), (EV_SW, SW_HEADPHONE_INSERT, 0)]);
        assert_eq!(ev, vec![Event::Switch { code: SW_HEADPHONE_INSERT, on: true }, Event::Switch { code: SW_HEADPHONE_INSERT, on: false }]);
    }

    #[test]
    fn keys_skip_autorepeat() {
        let ev = run(&[(EV_KEY, KEY_POWER, 1), (EV_KEY, KEY_POWER, 2), (EV_KEY, KEY_POWER, 0)]);
        assert_eq!(ev, vec![Event::Key { code: KEY_POWER, down: true }, Event::Key { code: KEY_POWER, down: false }]);
    }
}
