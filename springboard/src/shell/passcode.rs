//! The passcode: a salted, stretched SHA-256 of a numeric code kept in
//! `DIR/passcode.ini` together with the failed-attempt counter and the
//! lockout deadline (so a restart does not reset the back-off), the bifrost
//! keypad, the Settings flow that sets, changes and turns it off, and the
//! lock-screen prompt. The code itself is never logged or stored.

use super::ui::{centred, Ui, Weight, WHITE};
use super::{Anim, Drawn, Effect, Event, Shell};
use crate::gfx::{rgba, Surface};
use super::sha256::{digest as sha256, hex, unhex};
use super::clock::tz::unix_now;
use std::io;
use std::path::{Path, PathBuf};

/// Digits of the default code.
pub const DEFAULT_LEN: usize = 6;
/// Custom numeric codes run from this many digits ...
pub const MIN_LEN: usize = 4;
/// ... to this many.
pub const MAX_LEN: usize = 16;
/// Wrong codes in a row before the first lockout.
pub const FREE_TRIES: u32 = 5;
const STRETCH: u32 = 20_000;

/// The stored form of `code`: SHA-256 of salt and code, then stretched by
/// re-hashing with the salt so a stolen file is slow to brute-force.
pub fn digest(salt: &[u8], code: &str) -> [u8; 32] {
    let mut first = salt.to_vec();
    first.extend_from_slice(code.as_bytes());
    let mut h = sha256(&first);
    for _ in 0..STRETCH {
        let mut next = h.to_vec();
        next.extend_from_slice(salt);
        h = sha256(&next);
    }
    h
}

/// Compares in time independent of where the inputs differ.
pub fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn random_salt() -> [u8; 16] {
    use std::io::Read;
    let mut salt = [0u8; 16];
    if std::fs::File::open("/dev/urandom").and_then(|mut f| f.read_exact(&mut salt)).is_ok() {
        return salt;
    }
    // no entropy device: the clock and the address of a local still differ per call
    let seed = format!("{:?}{:p}", std::time::SystemTime::now(), &salt);
    salt.copy_from_slice(&sha256(seed.as_bytes())[..16]);
    salt
}

// -- the stored passcode -------------------------------------------------------

/// Seconds locked out after `fails` wrong codes in a row: none for the
/// first four, then 1 minute, 5, 15 and 60 minutes.
pub fn lockout_secs(fails: u32) -> u64 {
    match fails {
        0..=4 => 0,
        5 => 60,
        6 => 300,
        7 => 900,
        _ => 3600,
    }
}

/// What entering a code did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    Ok,
    /// wrong; `locked` is the lockout (seconds) it started, 0 for none
    Wrong { fails: u32, locked: u64 },
    /// not tried: still locked out for this many seconds
    Locked(u64),
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Store {
    path: Option<PathBuf>,
    salt: Vec<u8>,
    hash: Vec<u8>,
    len: usize,
    fails: u32,
    until: i64,
}

impl Store {
    /// Loads `path` (missing or damaged means no passcode) and saves there from now on.
    pub fn load(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        let mut s = std::fs::read_to_string(&path).map(|t| Self::parse(&t)).unwrap_or_default();
        s.path = Some(path);
        s
    }

    /// An in-memory store that never touches the disk.
    pub fn memory() -> Self {
        Store::default()
    }

    fn parse(text: &str) -> Self {
        let mut s = Store::default();
        for (k, v) in text.lines().filter_map(|l| l.split_once('=')) {
            match k.trim() {
                "salt" => s.salt = unhex(v.trim()).unwrap_or_default(),
                "hash" => s.hash = unhex(v.trim()).unwrap_or_default(),
                "len" => s.len = v.trim().parse().unwrap_or(0),
                "fails" => s.fails = v.trim().parse().unwrap_or(0),
                "until" => s.until = v.trim().parse().unwrap_or(0),
                _ => {}
            }
        }
        // a half-written file counts as no passcode rather than a locked-out bifrost
        if s.hash.len() != 32 || s.salt.is_empty() || !(MIN_LEN..=MAX_LEN).contains(&s.len) {
            return Store::default();
        }
        s
    }

    pub fn enabled(&self) -> bool {
        !self.hash.is_empty()
    }

    /// Digits of the stored code (0 when off).
    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        !self.enabled()
    }

    /// Whether `code` is acceptable as a new code: digits only, 4 to 16 of them.
    pub fn valid(code: &str) -> bool {
        (MIN_LEN..=MAX_LEN).contains(&code.len()) && code.bytes().all(|b| b.is_ascii_digit())
    }

    /// Turns the passcode on with `code` (a fresh salt), clearing the back-off.
    pub fn set(&mut self, code: &str) -> io::Result<()> {
        if !Self::valid(code) {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "passcode must be 4 to 16 digits"));
        }
        self.salt = random_salt().to_vec();
        self.hash = digest(&self.salt, code).to_vec();
        self.len = code.len();
        self.fails = 0;
        self.until = 0;
        self.save()
    }

    /// Turns the passcode off.
    pub fn clear(&mut self) -> io::Result<()> {
        *self = Store { path: self.path.take(), ..Store::default() };
        match &self.path {
            Some(p) => match std::fs::remove_file(p) {
                Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
                _ => Ok(()),
            },
            None => Ok(()),
        }
    }

    /// Seconds still locked out at Unix time `now`. A deadline further away
    /// than the current penalty (the clock was set back) is cut to it.
    pub fn locked_for(&self, now: i64) -> u64 {
        if self.until <= now {
            return 0;
        }
        ((self.until - now) as u64).min(lockout_secs(self.fails))
    }

    /// Checks `code`; a wrong one counts, persists and may start a lockout.
    pub fn verify(&mut self, code: &str, now: i64) -> Verdict {
        if !self.enabled() {
            return Verdict::Ok;
        }
        let left = self.locked_for(now);
        if left > 0 {
            return Verdict::Locked(left);
        }
        if ct_eq(&digest(&self.salt, code), &self.hash) {
            if self.fails != 0 || self.until != 0 {
                self.fails = 0;
                self.until = 0;
                let _ = self.save();
            }
            return Verdict::Ok;
        }
        self.fails += 1;
        let locked = lockout_secs(self.fails);
        self.until = if locked > 0 { now + locked as i64 } else { 0 };
        let _ = self.save();
        Verdict::Wrong { fails: self.fails, locked }
    }

    fn save(&self) -> io::Result<()> {
        let Some(path) = &self.path else { return Ok(()) };
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let mut tmp = path.file_name().map(|n| n.to_os_string()).unwrap_or_default();
        tmp.push(".tmp");
        let tmp = path.with_file_name(tmp);
        let text = format!("salt={}\nhash={}\nlen={}\nfails={}\nuntil={}\n", hex(&self.salt), hex(&self.hash), self.len, self.fails, self.until);
        let mut f = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&tmp)?;
        f.write_all(text.as_bytes())?;
        f.sync_all()?;
        std::fs::rename(&tmp, path)
    }
}

/// "Try again in 1 minute" / "in 15 minutes" / "in 45 seconds".
pub fn retry_text(secs: u64) -> String {
    if secs >= 60 {
        let m = secs.div_ceil(60);
        format!("Try again in {m} minute{}", if m == 1 { "" } else { "s" })
    } else {
        format!("Try again in {secs} second{}", if secs == 1 { "" } else { "s" })
    }
}

// -- the keypad ----------------------------------------------------------------

/// What a keypad touch produced.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pad {
    Digit(u8),
    Delete,
    Cancel,
    /// the Done button of a custom-length code
    Done,
}

const KEY_PT: f32 = 78.0;
const GAP_PT: f32 = 22.0;
const LETTERS: [&str; 10] = ["", "", "ABC", "DEF", "GHI", "JKL", "MNO", "PQRS", "TUV", "WXYZ"];
pub(super) const SHAKE_MS: f64 = 450.0;

/// The bifrost numeric keypad with the title and the entry dots above it.
pub struct Keypad {
    pub title: String,
    pub message: String,
    /// shows a Done button (custom-length code) and keeps the dots to what was typed
    pub custom: bool,
    /// dots to show while nothing is typed (the code length)
    pub dots: usize,
    digits: String,
    pressed: Option<Pad>,
    shake: Option<f64>,
}

impl Keypad {
    pub fn new(title: &str, dots: usize, custom: bool) -> Self {
        Keypad { title: title.into(), message: String::new(), custom, dots, digits: String::new(), pressed: None, shake: None }
    }

    pub fn digits(&self) -> &str {
        &self.digits
    }

    pub fn clear(&mut self) {
        self.digits.clear();
    }

    /// Appends a digit; false when already at `max`.
    pub fn push(&mut self, d: u8, max: usize) -> bool {
        if self.digits.len() >= max {
            return false;
        }
        self.digits.push((b'0' + d) as char);
        true
    }

    pub fn pop(&mut self) {
        self.digits.pop();
    }

    /// Starts the wrong-code shake at `t` (ms) and empties the entry.
    pub fn reject(&mut self, t: f64) {
        self.shake = Some(t);
        self.digits.clear();
    }

    /// Whether the shake still needs frames.
    pub fn animating(&self, t: f64) -> bool {
        self.shake.is_some_and(|s| t - s < SHAKE_MS)
    }

    /// The key at (x, y) px, with its rectangle, for a screen `w` x `h`.
    fn keys(&self, ui: &Ui, w: i32, h: i32) -> Vec<(Pad, (f32, f32, f32, f32))> {
        let (k, g) = (ui.pt(KEY_PT), ui.pt(GAP_PT));
        let gw = 3.0 * k + 2.0 * g;
        let x0 = (w as f32 - gw) / 2.0;
        let y0 = self.keys_top(ui, h);
        let mut out = vec![];
        for d in 0..10u8 {
            let (col, row) = if d == 0 { (1, 3) } else { ((d as i32 - 1) % 3, (d as i32 - 1) / 3) };
            out.push((Pad::Digit(d), (x0 + col as f32 * (k + g), y0 + row as f32 * (k + g), k, k)));
        }
        let by = y0 + 4.0 * k + 3.0 * g + ui.pt(20.0);
        let (bw, bh) = (ui.pt(110.0), ui.pt(40.0));
        out.push((Pad::Cancel, (x0 - ui.pt(4.0), by, bw, bh)));
        let right = if self.custom && self.digits.len() >= MIN_LEN {
            Pad::Done
        } else {
            Pad::Delete
        };
        out.push((right, (x0 + gw - bw + ui.pt(4.0), by, bw, bh)));
        out
    }

    /// Top of the key grid: the block of title, dots and keys sits centred.
    fn keys_top(&self, ui: &Ui, h: i32) -> f32 {
        let block = ui.pt(30.0 + 24.0 + 18.0 + 46.0) + 4.0 * ui.pt(KEY_PT) + 3.0 * ui.pt(GAP_PT) + ui.pt(20.0 + 40.0);
        ((h as f32 - block) / 2.0).max(ui.pt(16.0)) + ui.pt(30.0 + 24.0 + 18.0 + 46.0)
    }

    /// The key under a point.
    pub fn key_at(&self, ui: &Ui, w: i32, h: i32, x: f32, y: f32) -> Option<Pad> {
        self.keys(ui, w, h).into_iter().find(|&(_, (rx, ry, rw, rh))| x >= rx && x < rx + rw && y >= ry && y < ry + rh).map(|(p, _)| p)
    }

    /// Centre of a key, for tests.
    pub fn centre(&self, ui: &Ui, w: i32, h: i32, pad: Pad) -> Option<(f32, f32)> {
        self.keys(ui, w, h).into_iter().find(|&(p, _)| p == pad).map(|(_, (x, y, kw, kh))| (x + kw / 2.0, y + kh / 2.0))
    }

    /// Highlights on touch down and reports the key lifted on; `None` for
    /// moves, presses that slid off the key and everything else.
    pub fn touch(&mut self, ui: &Ui, w: i32, h: i32, ev: Event) -> Option<Pad> {
        match ev {
            Event::Down(x, y) => {
                self.pressed = self.key_at(ui, w, h, x, y);
                None
            }
            Event::Move(x, y) => {
                if self.pressed.is_some() && self.key_at(ui, w, h, x, y) != self.pressed {
                    self.pressed = None;
                }
                None
            }
            Event::Up(x, y) => {
                let down = self.pressed.take();
                down.filter(|&p| self.key_at(ui, w, h, x, y) == Some(p))
            }
            _ => None,
        }
    }

    /// Whether the Delete or Done button shows (Delete only while there is something to delete).
    fn shows(&self, pad: Pad) -> bool {
        match pad {
            Pad::Delete => !self.digits.is_empty(),
            _ => true,
        }
    }

    /// Draws over `backdrop` (opaque black in Settings, a veil on the lock screen).
    pub fn draw(&self, s: &mut Surface, ui: &Ui, w: i32, h: i32, backdrop: u32, t: f64) {
        s.fill_rrect(0.0, 0.0, w as f32, h as f32, 0.0, backdrop);
        let cx = w / 2;
        let top = self.keys_top(ui, h) - ui.pt(30.0 + 24.0 + 18.0 + 46.0);
        let title = ui.text(&self.title, 20.0, Weight::Medium, WHITE);
        centred(s, &title, cx, top as i32);
        if !self.message.is_empty() {
            let m = ui.text(&self.message, 14.0, Weight::Regular, rgba(255, 255, 255, 190));
            centred(s, &m, cx, (top + ui.pt(30.0)) as i32);
        }
        // entry dots: squares, shaken sideways after a wrong code
        let n = if self.custom { self.digits.len().max(1) } else { self.dots.max(self.digits.len()) };
        let (dot, gap) = (ui.pt(12.0), ui.pt(16.0));
        let row_w = n as f32 * dot + (n as f32 - 1.0) * gap;
        let shake = self.shake.map_or(0.0, |s0| {
            let p = ((t - s0) / SHAKE_MS).clamp(0.0, 1.0) as f32;
            (p * 6.0 * std::f32::consts::PI).sin() * (1.0 - p) * ui.pt(14.0)
        });
        let dy = top + ui.pt(30.0 + 24.0 + 18.0);
        for i in 0..n {
            let x = cx as f32 - row_w / 2.0 + i as f32 * (dot + gap) + shake;
            let on = i < self.digits.len();
            // bifrost dots: a white ring when empty, filled once typed
            s.fill_rrect(x, dy, dot, dot, dot / 2.0, WHITE);
            if !on {
                s.fill_rrect(x + 1.5, dy + 1.5, dot - 3.0, dot - 3.0, (dot - 3.0) / 2.0, rgba(40, 40, 40, 255));
            }
        }
        for (pad, (x, y, kw, kh)) in self.keys(ui, w, h) {
            if !self.shows(pad) {
                continue;
            }
            match pad {
                Pad::Digit(d) => {
                    let down = self.pressed == Some(pad);
                    s.fill_rrect(x, y, kw, kh, kw / 2.0, if down { rgba(255, 255, 255, 200) } else { rgba(255, 255, 255, 38) });
                    let ink = if down { rgba(0, 0, 0, 255) } else { WHITE };
                    let num = ui.text(&d.to_string(), 34.0, Weight::Regular, ink);
                    let letters = LETTERS[d as usize];
                    let block = num.h as f32 + if letters.is_empty() { 0.0 } else { ui.pt(10.0) };
                    centred(s, &num, (x + kw / 2.0) as i32, (y + (kh - block) / 2.0) as i32);
                    if !letters.is_empty() {
                        let l = ui.text(letters, 10.0, Weight::Semibold, ink);
                        centred(s, &l, (x + kw / 2.0) as i32, (y + (kh - block) / 2.0 + num.h as f32 - ui.pt(4.0)) as i32);
                    }
                }
                other => {
                    let label = match other {
                        Pad::Cancel => "Cancel",
                        Pad::Done => "Done",
                        _ => "Delete",
                    };
                    let text = ui.text(label, 17.0, Weight::Regular, if self.pressed == Some(other) { rgba(255, 255, 255, 140) } else { WHITE });
                    centred(s, &text, (x + kw / 2.0) as i32, (y + (kh - text.h as f32) / 2.0) as i32);
                }
            }
        }
    }
}

// -- Settings: set, change, turn off ---------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Purpose {
    TurnOn,
    Change,
    TurnOff,
    /// only confirms the current code; the store is not changed (Settings > Passwords reveal)
    Reveal,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Step {
    Current,
    New,
    Verify,
}

/// What a submitted code did to the flow.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FlowOut {
    /// show this title (and message) for the next step; the entry is empty
    Next,
    /// the same step again, with this message
    Retry(String),
    /// finished: the store was changed
    Done,
    /// the store could not be written
    Failed,
}

/// The state machine behind Settings > Passcode: confirm the current code
/// when there is one, ask for the new one twice, then store it.
pub struct Flow {
    pub purpose: Purpose,
    step: Step,
    first: String,
    pub custom: bool,
}

impl Flow {
    pub fn new(purpose: Purpose, custom: bool) -> Self {
        let step = if purpose == Purpose::TurnOn { Step::New } else { Step::Current };
        Flow { purpose, step, first: String::new(), custom }
    }

    pub fn title(&self) -> &'static str {
        match self.step {
            Step::Current => "Enter your passcode",
            Step::New if self.purpose == Purpose::Change => "Enter your new passcode",
            Step::New => "Enter a passcode",
            Step::Verify => "Re-enter your passcode",
        }
    }

    /// Digits that submit by themselves at this step (`None`: the Done button does).
    pub fn auto_len(&self, store: &Store) -> Option<usize> {
        match self.step {
            Step::Current => Some(store.len()),
            _ if self.custom => None,
            _ => Some(DEFAULT_LEN),
        }
    }

    /// Longest entry the keypad accepts at this step.
    pub fn max_len(&self, store: &Store) -> usize {
        match self.step {
            Step::Current => store.len(),
            _ if self.custom => MAX_LEN,
            _ => DEFAULT_LEN,
        }
    }

    pub fn submit(&mut self, store: &mut Store, code: &str, now: i64) -> FlowOut {
        match self.step {
            Step::Current => match store.verify(code, now) {
                Verdict::Ok if self.purpose == Purpose::TurnOff => {
                    if store.clear().is_ok() {
                        FlowOut::Done
                    } else {
                        FlowOut::Failed
                    }
                }
                Verdict::Ok if self.purpose == Purpose::Reveal => FlowOut::Done,
                Verdict::Ok => {
                    self.step = Step::New;
                    FlowOut::Next
                }
                Verdict::Wrong { locked: 0, fails } => FlowOut::Retry(format!("{} failed passcode attempt{}", fails, if fails == 1 { "" } else { "s" })),
                Verdict::Wrong { locked, .. } | Verdict::Locked(locked) => FlowOut::Retry(retry_text(locked)),
            },
            Step::New => {
                if !Store::valid(code) {
                    return FlowOut::Retry(format!("Use at least {MIN_LEN} digits"));
                }
                self.first = code.to_string();
                self.step = Step::Verify;
                FlowOut::Next
            }
            Step::Verify => {
                if !ct_eq(self.first.as_bytes(), code.as_bytes()) {
                    self.first.clear();
                    self.step = Step::New;
                    return FlowOut::Retry("Passcodes did not match. Try again.".into());
                }
                let ok = store.set(code).is_ok();
                self.first.clear();
                if ok {
                    FlowOut::Done
                } else {
                    FlowOut::Failed
                }
            }
        }
    }
}

/// The Settings overlay: a flow driving a keypad on a black screen.
pub struct Overlay {
    pub flow: Flow,
    pub pad: Keypad,
    store: Store,
    w: i32,
    h: i32,
}

/// What the overlay's touch means for Settings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OverlayOut {
    Open,
    /// closed without a change
    Cancelled,
    /// closed after the store changed
    Done,
}

impl Overlay {
    pub fn new(path: &Path, purpose: Purpose, custom: bool, w: i32, h: i32) -> Self {
        let store = Store::load(path);
        let flow = Flow::new(purpose, custom);
        let mut pad = Keypad::new(flow.title(), flow.auto_len(&store).unwrap_or(DEFAULT_LEN), flow.auto_len(&store).is_none());
        pad.message = String::new();
        Overlay { flow, pad, store, w, h }
    }

    fn sync(&mut self) {
        self.pad.title = self.flow.title().into();
        self.pad.dots = self.flow.auto_len(&self.store).unwrap_or(DEFAULT_LEN);
        self.pad.custom = self.flow.auto_len(&self.store).is_none();
    }

    fn submit(&mut self, t: f64) -> OverlayOut {
        let code = self.pad.digits().to_string();
        let out = self.flow.submit(&mut self.store, &code, unix_now());
        self.pad.clear();
        match out {
            FlowOut::Next => {
                self.pad.message.clear();
                self.sync();
            }
            FlowOut::Retry(m) => {
                self.pad.message = m;
                self.pad.reject(t);
                self.sync();
            }
            FlowOut::Done => return OverlayOut::Done,
            FlowOut::Failed => {
                self.pad.message = "Could not save the passcode".into();
                self.pad.reject(t);
            }
        }
        OverlayOut::Open
    }

    pub fn touch(&mut self, ui: &Ui, ev: Event, t: f64) -> OverlayOut {
        let Some(key) = self.pad.touch(ui, self.w, self.h, ev) else { return OverlayOut::Open };
        match key {
            Pad::Cancel => return OverlayOut::Cancelled,
            Pad::Delete => self.pad.pop(),
            Pad::Done => return self.submit(t),
            Pad::Digit(d) => {
                if self.pad.push(d, self.flow.max_len(&self.store)) && self.flow.auto_len(&self.store) == Some(self.pad.digits().len()) {
                    return self.submit(t);
                }
            }
        }
        OverlayOut::Open
    }

    pub fn animating(&self, t: f64) -> bool {
        self.pad.animating(t)
    }

    pub fn draw(&self, s: &mut Surface, ui: &Ui, t: f64) {
        self.pad.draw(s, ui, self.w, self.h, rgba(0, 0, 0, 255), t);
    }
}

// -- the lock screen prompt ------------------------------------------------------

/// What a touch on the lock-screen prompt meant.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LockOut {
    Open,
    Cancelled,
    Unlocked,
}

/// The "Enter Passcode" prompt shown after the unlock swipe.
pub struct Lock {
    pub pad: Keypad,
    store: Store,
    w: i32,
    h: i32,
    last_msg: String,
}

impl Lock {
    pub fn new(path: &Path, w: i32, h: i32) -> Self {
        let store = Store::load(path);
        let pad = Keypad::new("Enter Passcode", store.len().max(MIN_LEN), false);
        let mut lock = Lock { pad, store, w, h, last_msg: String::new() };
        lock.refresh(unix_now());
        lock
    }

    /// Whether `path` holds an enabled passcode (read fresh, so Settings changes count at once).
    pub fn required(path: &Path) -> bool {
        Store::load(path).enabled()
    }

    /// Updates the lockout message; true when it changed (a frame is due).
    pub fn refresh(&mut self, now: i64) -> bool {
        let left = self.store.locked_for(now);
        let msg = if left > 0 { retry_text(left) } else { String::new() };
        if msg == self.last_msg {
            return false;
        }
        self.pad.message = msg.clone();
        self.last_msg = msg;
        true
    }

    /// Whether a lockout countdown is on screen (needs a tick per second).
    pub fn locked(&self, now: i64) -> bool {
        self.store.locked_for(now) > 0
    }

    pub fn animating(&self, t: f64) -> bool {
        self.pad.animating(t)
    }

    pub fn touch(&mut self, ui: &Ui, ev: Event, t: f64) -> LockOut {
        let Some(key) = self.pad.touch(ui, self.w, self.h, ev) else { return LockOut::Open };
        let now = unix_now();
        match key {
            Pad::Cancel => return LockOut::Cancelled,
            Pad::Delete => self.pad.pop(),
            Pad::Done => {}
            Pad::Digit(d) => {
                if self.store.locked_for(now) > 0 {
                    return LockOut::Open;
                }
                if self.pad.push(d, self.store.len()) && self.pad.digits().len() == self.store.len() {
                    let code = self.pad.digits().to_string();
                    return match self.store.verify(&code, now) {
                        Verdict::Ok => LockOut::Unlocked,
                        _ => {
                            self.pad.reject(t);
                            self.last_msg.clear();
                            self.refresh(now);
                            if self.pad.message.is_empty() {
                                self.pad.message = "Wrong passcode".into();
                            }
                            LockOut::Open
                        }
                    };
                }
            }
        }
        LockOut::Open
    }

    pub fn draw(&self, s: &mut Surface, ui: &Ui, t: f64) {
        self.pad.draw(s, ui, self.w, self.h, rgba(0, 0, 0, 175), t);
    }
}

// -- the shell's side of the lock-screen prompt ------------------------------------

impl Shell {
    pub(super) fn passcode_path(&self) -> PathBuf {
        self.data_dir.join("passcode.ini")
    }

    /// Starts the unlock slide from `from` (a fraction of the screen), or
    /// asks for the passcode first when one is set ("Require: Immediately").
    pub(super) fn begin_unlock(&mut self, from: f32, t: f64) {
        let path = self.passcode_path();
        if Lock::required(&path) {
            self.lock_drag = None;
            self.lock_pad = Some(Lock::new(&path, self.w, self.h));
            self.dirty = true;
        } else {
            self.anim = Some((Anim::Unlock { from, to: 1.0 }, t));
        }
    }

    /// Routes a touch to the prompt while it is open.
    pub(super) fn lock_pad_touch(&mut self, ev: Event, t: f64) -> Vec<Effect> {
        let Some(pad) = &mut self.lock_pad else { return vec![] };
        let out = pad.touch(&self.ui, ev, t);
        self.dirty = true;
        match out {
            LockOut::Open => {}
            LockOut::Cancelled => {
                self.lock_pad = None;
                self.notify_cancel_open();
                self.drawn = Drawn::Nothing;
            }
            LockOut::Unlocked => {
                self.lock_pad = None;
                self.anim = Some((Anim::Unlock { from: 0.0, to: 1.0 }, t));
            }
        }
        vec![]
    }

    /// Redraws the prompt for the lockout countdown and the wrong-code shake.
    pub(super) fn lock_pad_tick(&mut self, t: f64) {
        if let Some(pad) = &mut self.lock_pad {
            if pad.refresh(unix_now()) | pad.animating(t) {
                self.dirty = true;
            }
        }
    }

    /// When the prompt next needs a frame: now while shaking, each second while locked out.
    pub(super) fn lock_pad_wake(&self, t: f64) -> Option<f64> {
        let pad = self.lock_pad.as_ref()?;
        if pad.animating(t) {
            Some(t)
        } else if pad.locked(unix_now()) {
            Some(t + 1000.0)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/target/test-passcode")).join(std::process::id().to_string());
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join(name);
        let _ = std::fs::remove_file(&p);
        p
    }

    #[test]
    fn digest_depends_on_salt_and_code() {
        assert_eq!(digest(b"s1", "123456"), digest(b"s1", "123456"));
        assert_ne!(digest(b"s1", "123456"), digest(b"s2", "123456"));
        assert_ne!(digest(b"s1", "123456"), digest(b"s1", "123457"));
    }

    #[test]
    fn constant_time_compare() {
        assert!(ct_eq(b"abc", b"abc"));
        assert!(!ct_eq(b"abc", b"abd"));
        assert!(!ct_eq(b"abc", b"ab"));
        assert!(ct_eq(b"", b""));
    }

    #[test]
    fn backoff_escalates() {
        assert_eq!([0, 4, 5, 6, 7, 8, 20].map(lockout_secs), [0, 0, 60, 300, 900, 3600, 3600]);
        assert_eq!(retry_text(60), "Try again in 1 minute");
        assert_eq!(retry_text(300), "Try again in 5 minutes");
        assert_eq!(retry_text(61), "Try again in 2 minutes");
        assert_eq!(retry_text(1), "Try again in 1 second");
    }

    #[test]
    fn five_wrong_codes_lock_and_the_lock_survives_a_restart() {
        let path = scratch("lock.ini");
        let mut s = Store::load(&path);
        assert!(!s.enabled());
        s.set("123456").unwrap();
        let now = 1_000_000;
        for k in 1..5 {
            assert_eq!(s.verify("000000", now), Verdict::Wrong { fails: k, locked: 0 });
        }
        assert_eq!(s.verify("000000", now), Verdict::Wrong { fails: 5, locked: 60 });
        assert_eq!(s.verify("123456", now + 10), Verdict::Locked(50), "even the right code waits");
        let mut again = Store::load(&path);
        assert_eq!(again.locked_for(now + 10), 50, "persisted across restarts");
        assert_eq!(again.verify("123456", now + 61), Verdict::Ok);
        assert_eq!(Store::load(&path).locked_for(now + 61), 0);
        // the sixth failure in a row costs five minutes
        let mut s = Store::load(&path);
        for _ in 0..5 {
            s.verify("1", now);
        }
        assert_eq!(s.verify("1", now + 61), Verdict::Wrong { fails: 6, locked: 300 });
    }

    #[test]
    fn a_clock_set_back_does_not_extend_the_lockout() {
        let mut s = Store::memory();
        s.set("123456").unwrap();
        for _ in 0..5 {
            s.verify("1", 1_000_000);
        }
        assert_eq!(s.locked_for(1_000_000 - 86_400), 60);
    }

    #[test]
    fn the_file_never_holds_the_code_and_is_private() {
        use std::os::unix::fs::PermissionsExt;
        let path = scratch("secret.ini");
        let mut s = Store::load(&path);
        s.set("482913").unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(!text.contains("482913"));
        assert!(text.contains("salt=") && text.contains("hash=") && text.contains("len=6"));
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        let two = scratch("secret2.ini");
        let mut t = Store::load(&two);
        t.set("482913").unwrap();
        assert_ne!(std::fs::read_to_string(&two).unwrap(), text, "fresh salt per set");
    }

    #[test]
    fn damaged_files_mean_no_passcode_and_clear_removes_the_file() {
        let path = scratch("bad.ini");
        std::fs::write(&path, "salt=00\nhash=12\nlen=6\n").unwrap();
        assert!(!Store::load(&path).enabled());
        let mut s = Store::load(&path);
        s.set("1234").unwrap();
        assert_eq!(s.len(), 4);
        assert!(Store::set(&mut s, "12a4").is_err() && Store::set(&mut s, "123").is_err() && !Store::valid(&"1".repeat(17)));
        s.clear().unwrap();
        assert!(!path.exists() && !s.enabled());
        assert_eq!(s.verify("anything", 0), Verdict::Ok, "no passcode accepts anything");
    }

    #[test]
    fn flow_sets_changes_and_turns_off() {
        let mut s = Store::memory();
        let mut f = Flow::new(Purpose::TurnOn, false);
        assert_eq!(f.title(), "Enter a passcode");
        assert_eq!(f.auto_len(&s), Some(6));
        assert_eq!(f.submit(&mut s, "111111", 0), FlowOut::Next);
        assert_eq!(f.title(), "Re-enter your passcode");
        assert!(matches!(f.submit(&mut s, "222222", 0), FlowOut::Retry(_)));
        assert_eq!(f.title(), "Enter a passcode", "a mismatch starts over");
        assert_eq!(f.submit(&mut s, "111111", 0), FlowOut::Next);
        assert_eq!(f.submit(&mut s, "111111", 0), FlowOut::Done);
        assert!(s.enabled());

        let mut c = Flow::new(Purpose::Change, true);
        assert_eq!(c.title(), "Enter your passcode");
        assert!(matches!(c.submit(&mut s, "999999", 0), FlowOut::Retry(m) if m.contains("1 failed")));
        assert_eq!(c.submit(&mut s, "111111", 0), FlowOut::Next);
        assert_eq!(c.title(), "Enter your new passcode");
        assert_eq!(c.auto_len(&s), None, "custom codes end with Done");
        assert!(matches!(c.submit(&mut s, "12", 0), FlowOut::Retry(_)));
        assert_eq!(c.submit(&mut s, "7654321", 0), FlowOut::Next);
        assert_eq!(c.submit(&mut s, "7654321", 0), FlowOut::Done);
        assert_eq!(s.len(), 7);

        let mut off = Flow::new(Purpose::TurnOff, false);
        assert_eq!(off.max_len(&s), 7);
        assert!(matches!(off.submit(&mut s, "1", 0), FlowOut::Retry(_)));
        assert_eq!(off.submit(&mut s, "7654321", 0), FlowOut::Done);
        assert!(!s.enabled());
    }

    #[test]
    fn flow_reports_the_lockout() {
        let mut s = Store::memory();
        s.set("123456").unwrap();
        let mut f = Flow::new(Purpose::TurnOff, false);
        for _ in 0..4 {
            f.submit(&mut s, "000000", 0);
        }
        assert!(matches!(f.submit(&mut s, "000000", 0), FlowOut::Retry(m) if m == "Try again in 1 minute"));
        assert!(matches!(f.submit(&mut s, "123456", 5), FlowOut::Retry(_)));
        assert!(s.enabled());
    }

    #[test]
    fn keypad_types_deletes_and_taps_the_keys() {
        let ui = crate::shell::tests::ui();
        let mut k = Keypad::new("t", 6, false);
        let (x, y) = k.centre(&ui, 1200, 1920, Pad::Digit(5)).unwrap();
        assert_eq!(k.touch(&ui, 1200, 1920, Event::Down(x, y)), None);
        assert_eq!(k.touch(&ui, 1200, 1920, Event::Up(x, y)), Some(Pad::Digit(5)));
        let (x0, y0) = k.centre(&ui, 1200, 1920, Pad::Digit(0)).unwrap();
        assert!(y0 > y, "zero sits under the grid");
        k.touch(&ui, 1200, 1920, Event::Down(x, y));
        assert_eq!(k.touch(&ui, 1200, 1920, Event::Up(x0, y0)), None, "lifting on another key is no tap");
        assert!(k.push(1, 3) && k.push(2, 3) && k.push(3, 3) && !k.push(4, 3));
        assert_eq!(k.digits(), "123");
        k.pop();
        assert_eq!(k.digits(), "12");
        assert_eq!(k.key_at(&ui, 1200, 1920, 1.0, 1.0), None);
        assert!(k.centre(&ui, 1200, 1920, Pad::Delete).is_some());
        k.custom = true;
        k.push(3, 16);
        k.push(4, 16);
        assert!(k.centre(&ui, 1200, 1920, Pad::Done).is_some(), "Done appears once the code is long enough");
        k.reject(10.0);
        assert!(k.animating(100.0) && !k.animating(10_000.0) && k.digits().is_empty());
        let mut buf = vec![0u32; 1200 * 1920];
        k.draw(&mut Surface { w: 1200, h: 1920, stride: 1200, px: &mut buf, y0: 0 }, &ui, 1200, 1920, rgba(0, 0, 0, 255), 50.0);
    }

    #[test]
    fn lock_prompt_unlocks_on_the_right_code_and_shakes_on_a_wrong_one() {
        let ui = crate::shell::tests::ui();
        let path = scratch("prompt.ini");
        Store::load(&path).set("135790").unwrap();
        assert!(Lock::required(&path));
        let mut lock = Lock::new(&path, 1200, 1920);
        let press = |lock: &mut Lock, digits: &str| {
            let mut out = LockOut::Open;
            for c in digits.bytes() {
                let (x, y) = lock.pad.centre(&ui, 1200, 1920, Pad::Digit(c - b'0')).unwrap();
                lock.touch(&ui, Event::Down(x, y), 1.0);
                out = lock.touch(&ui, Event::Up(x, y), 1.0);
            }
            out
        };
        assert_eq!(press(&mut lock, "000000"), LockOut::Open);
        assert!(lock.animating(100.0) && lock.pad.digits().is_empty());
        assert_eq!(press(&mut lock, "135790"), LockOut::Unlocked);
        let none = scratch("none.ini");
        assert!(!Lock::required(&none));
        let (cx, cy) = lock.pad.centre(&ui, 1200, 1920, Pad::Cancel).unwrap();
        lock.touch(&ui, Event::Down(cx, cy), 1.0);
        assert_eq!(lock.touch(&ui, Event::Up(cx, cy), 1.0), LockOut::Cancelled);
    }

    #[test]
    fn lock_prompt_ignores_digits_while_locked_out() {
        let ui = crate::shell::tests::ui();
        let path = scratch("locked.ini");
        let mut s = Store::load(&path);
        s.set("135790").unwrap();
        for _ in 0..5 {
            s.verify("1", unix_now());
        }
        let mut lock = Lock::new(&path, 1200, 1920);
        assert!(lock.locked(unix_now()));
        assert!(lock.pad.message.starts_with("Try again in"));
        let (x, y) = lock.pad.centre(&ui, 1200, 1920, Pad::Digit(1)).unwrap();
        lock.touch(&ui, Event::Down(x, y), 1.0);
        lock.touch(&ui, Event::Up(x, y), 1.0);
        assert!(lock.pad.digits().is_empty());
        assert!(!lock.refresh(unix_now()), "unchanged text needs no frame");
    }

    #[test]
    fn overlay_walks_through_turning_the_passcode_on() {
        let ui = crate::shell::tests::ui();
        let path = scratch("overlay.ini");
        let mut o = Overlay::new(&path, Purpose::TurnOn, false, 1200, 1920);
        let type_code = |o: &mut Overlay, code: &str| {
            let mut out = OverlayOut::Open;
            for c in code.bytes() {
                let (x, y) = o.pad.centre(&ui, 1200, 1920, Pad::Digit(c - b'0')).unwrap();
                o.touch(&ui, Event::Down(x, y), 1.0);
                out = o.touch(&ui, Event::Up(x, y), 1.0);
            }
            out
        };
        assert_eq!(type_code(&mut o, "246810"), OverlayOut::Open);
        assert_eq!(o.pad.title, "Re-enter your passcode");
        assert_eq!(type_code(&mut o, "246810"), OverlayOut::Done);
        assert!(Store::load(&path).enabled());
    }
}
