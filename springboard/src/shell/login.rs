//! The login window shown on the lock screen (the reference's login window: goldengate_refimages Login-Window): the
//! clock stays at the top, the account's avatar, its full name and a capsule password field with a dot mask and a
//! return arrow sit low in the middle, and a power button at the right of the status icons opens Sleep, Restart…
//! and Shut Down… (the last two ask first, in a sheet). The password is checked by
//! the root helper (`login.authenticate` over `privd.sock`) on a worker thread; this file never blocks the UI
//! loop and never logs or stores the password beyond the field buffer, which is taken on submit.
//!
//! Layout derives from the screen size and `ui.pt()`; colours are `Pair`s resolved with `.now()`.

use super::keyboard::Out;
use super::ui::{centred, Ui, Weight};
use super::{Effect, Event};
use crate::gfx::{rgba, Surface};
use bifrost_render::appearance::Pair;
use std::sync::mpsc::{channel, Receiver, TryRecvError};
use std::sync::Mutex;
use std::sync::Arc;

/// Overwrites a password buffer with zeros before it is cleared, so it does not linger in freed memory.
fn wipe(s: &mut String) {
    // SAFETY: writing zero bytes keeps the string valid UTF-8.
    for b in unsafe { s.as_bytes_mut() } {
        unsafe { std::ptr::write_volatile(b, 0) };
    }
    std::sync::atomic::compiler_fence(std::sync::atomic::Ordering::SeqCst);
    s.clear();
}

/// The wait (s) the window imposes after `wrong` wrong passwords in a row: none before `BACKOFF_AFTER`, then 30 s,
/// doubling with each further miss up to `BACKOFF_MAX_SECS`.
pub fn local_backoff(wrong: u32) -> u64 {
    if wrong < BACKOFF_AFTER {
        return 0;
    }
    let steps = (wrong - BACKOFF_AFTER).min(8);
    (30u64 << steps).min(BACKOFF_MAX_SECS)
}

/// Wrong-password shake length (same curve as the old keypad).
pub const SHAKE_MS: f64 = super::passcode::SHAKE_MS;
/// Longest password the field takes.
pub const MAX_LEN: usize = 256;
/// How often an unavailable login helper is asked again.
const RETRY_MS: f64 = 5000.0;
/// Wrong passwords in a row before the password hint is shown.
pub const HINT_AFTER: u32 = 3;
/// Wrong passwords in a row before the window itself waits between attempts (privd keeps its own, longer lockout;
/// this one holds even when the helper answers without a retry time).
pub const BACKOFF_AFTER: u32 = 5;
/// The longest wait the window imposes between attempts.
pub const BACKOFF_MAX_SECS: u64 = 300;
/// The power button's size in the status bar.
const POWER_PT: f32 = 22.0;
/// How long the account group takes to move for the on-screen keyboard.
pub const LIFT_MS: f64 = 250.0;
/// Where the account group sits, as a fraction of the screen height (the avatar's top edge).
const GROUP_TOP: f32 = 0.62;

/// Whether the shell runs in the touch (tablet) form, where the login window raises the on-screen keyboard. The form
/// comes from the shared form-factor model (`BIFROST_FORM`, then `/etc/bifrost/form`, default desktop).
pub fn tablet_form() -> bool {
    bifrost_ui::form_factor::FormFactor::current().is_tablet()
}

/// How far the status icons move left while the login window shows its power button.
pub fn status_inset(ui: &Ui) -> i32 {
    (ui.pt(10.0) + ui.pt(POWER_PT) + ui.pt(8.0)) as i32
}

/// The power symbol (a ring open at the top and a bar through the gap), white with antialiased alpha, `d` px square.
fn power_glyph(d: usize) -> crate::gfx::Image {
    let mut img = crate::gfx::Image::new(d, d);
    let c = d as f32 / 2.0;
    let (r, half) = (d as f32 * 0.36, (d as f32 * 0.055).max(0.75));
    for y in 0..d {
        for x in 0..d {
            let (px, py) = (x as f32 + 0.5 - c, y as f32 + 0.5 - c);
            let ring = (px.hypot(py) - r).abs() - half;
            // open at the top: no ring within 40 degrees either side of straight up
            let ring = if py < 0.0 && px.abs() < -py * 0.84 { f32::MAX } else { ring };
            let bar_y = (py + r * 0.55).abs().max(0.0);
            let bar = if py <= 0.0 && py >= -r - half { px.abs() - half } else { px.abs().max(bar_y - r * 0.55) - half };
            let a = (0.5 - ring.min(bar)).clamp(0.0, 1.0);
            let a = (a * 255.0) as u32;
            img.px[y * d + x] = a << 24 | a << 16 | a << 8 | a;
        }
    }
    img
}

fn scrim() -> u32 {
    // a light veil only: the reference keeps the wallpaper and the clock clear behind the login window
    Pair::new(rgba(0, 0, 0, 36), rgba(0, 0, 0, 24)).now()
}
fn ink() -> u32 {
    Pair::new(rgba(255, 255, 255, 255), rgba(20, 20, 22, 255)).now()
}
fn ink_2() -> u32 {
    Pair::new(rgba(255, 255, 255, 170), rgba(20, 20, 22, 150)).now()
}
fn field() -> u32 {
    Pair::new(rgba(255, 255, 255, 46), rgba(255, 255, 255, 170)).now()
}
fn sheet() -> u32 {
    Pair::new(rgba(44, 44, 46, 245), rgba(246, 246, 246, 245)).now()
}
fn avatar() -> u32 {
    Pair::new(rgba(99, 99, 102, 255), rgba(174, 174, 178, 255)).now()
}
fn bad() -> u32 {
    Pair::new(rgba(255, 105, 97, 255), rgba(215, 0, 21, 255)).now()
}

/// Longest lock message kept; longer text is cut at a character boundary.
pub const MESSAGE_MAX: usize = 200;

/// The message shown on the locked screen (Settings > Lock Screen writes it): one `message=...` line in the
/// `lock-message` file of the shell's data directory. Control characters are dropped, blank text means no message.
pub fn parse_lock_message(text: &str) -> Option<String> {
    let raw = text.lines().find_map(|l| l.strip_prefix("message="))?;
    let clean: String = raw.chars().filter(|c| !c.is_control()).collect();
    let clean = clean.trim();
    if clean.is_empty() {
        return None;
    }
    Some(clean.chars().take(MESSAGE_MAX).collect())
}

/// The lock message from the first data directory that has one (system file first, then the user's).
pub fn load_lock_message() -> Option<String> {
    let mut paths = vec![std::path::PathBuf::from("/var/lib/springboard/lock-message")];
    if let Some(x) = std::env::var_os("XDG_DATA_HOME") {
        paths.push(std::path::PathBuf::from(x).join("springboard/lock-message"));
    }
    if let Some(h) = std::env::var_os("HOME") {
        paths.push(std::path::PathBuf::from(h).join(".local/share/springboard/lock-message"));
    }
    paths.iter().find_map(|p| std::fs::read_to_string(p).ok().and_then(|t| parse_lock_message(&t)))
}

/// What the root helper answered to `login.authenticate`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reply {
    Ok(String),
    Wrong { retry_secs: u64 },
    LockedOut { retry_secs: u64 },
    NoPassword,
    /// the helper could not be reached
    Unavailable,
}

/// The helper's account answer, for the avatar and name.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AccountInfo {
    pub full_name: String,
    pub username: String,
    pub has_password: bool,
    pub locked_out_secs: u64,
    /// the account's password hint; shown under the message after a few wrong passwords
    pub password_hint: String,
}

/// The privd side of the login window; tests stub it.
pub trait Backend: Send + Sync {
    fn authenticate(&self, password: &str) -> Reply;
    fn account(&self) -> Option<AccountInfo>;
}

/// Message line under the field.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Msg {
    None,
    Wrong,
    /// name and password mode: the typed name is not this computer's account
    NameMismatch,
    LockedOut,
    Unavailable,
    NoPassword,
}

/// What a key or touch asked of the shell.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Act {
    None,
    Sleep,
    Restart,
    ShutDown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Sheet {
    Restart,
    ShutDown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Hit {
    Submit,
    /// name and password mode: the name field
    Name,
    /// the password field outside the return arrow
    Field,
    /// the user picture
    Avatar,
    Power,
    Sleep,
    Restart,
    ShutDown,
    Confirm,
    Cancel,
}

type Rect = (f32, f32, f32, f32);

pub struct Login {
    pub name: String,
    pub username: String,
    password: String,
    pub msg: Msg,
    pub caps: bool,
    shake: Option<f64>,
    until_ms: f64,
    /// behind a Mutex so the shell (drawn on several threads) stays Sync
    pending: Option<Mutex<Receiver<Reply>>>,
    /// while the helper is unavailable: the account query in flight on a worker, and when to ask again
    probe: Option<Mutex<Receiver<Option<AccountInfo>>>>,
    retry_at: f64,
    sheet: Option<Sheet>,
    /// the power button's menu (Sleep, Restart…, Shut Down…) is open
    power_open: bool,
    /// the user's message for the locked screen, drawn low on the screen
    pub lock_message: Option<String>,
    /// Users & Groups > Login Options > Name and Password: no picture; a name field above the password field
    pub name_mode: bool,
    /// the account's password hint (empty when none is set)
    pub hint: String,
    /// wrong passwords since the last success; the hint shows from `HINT_AFTER`
    wrong_count: u32,
    /// name mode: the name typed so far
    typed_name: String,
    /// name mode: keys go to the name field until Return moves them on to the password
    name_focus: bool,
    /// the touch form: tapping the password field raises the on-screen keyboard
    pub touch_keyboard: bool,
    /// the computer's accounts (full name, this session's account) when there is more than one, listed low on the
    /// screen as the reference's login window lists them; empty for a single account
    pub accounts: Vec<(String, bool)>,
    /// the on-screen keyboard is up for this window
    kb_up: bool,
    /// a raise or hide the shell has not carried out yet
    kb_request: Option<bool>,
    /// how far (px) the account group is moved up now
    lift: f32,
    /// the move in progress: start time, from, to
    lift_anim: Option<(f64, f32, f32)>,
    backend: Arc<dyn Backend>,
    w: i32,
    h: i32,
}

impl Login {
    pub fn new(backend: Arc<dyn Backend>, fallback_name: &str, w: i32, h: i32, t: f64) -> Self {
        let info = backend.account();
        let mut l = Login {
            name: fallback_name.to_string(),
            username: String::new(),
            password: String::new(),
            msg: Msg::None,
            caps: false,
            shake: None,
            until_ms: 0.0,
            pending: None,
            probe: None,
            retry_at: t + RETRY_MS,
            sheet: None,
            power_open: false,
            lock_message: None,
            name_mode: false,
            hint: String::new(),
            wrong_count: 0,
            typed_name: String::new(),
            name_focus: false,
            touch_keyboard: tablet_form(),
            accounts: Vec::new(),
            kb_up: false,
            kb_request: None,
            lift: 0.0,
            lift_anim: None,
            backend,
            w,
            h,
        };
        l.account_info(info, t);
        l
    }

    fn account_info(&mut self, info: Option<AccountInfo>, t: f64) {
        match info {
            None => self.msg = Msg::Unavailable,
            Some(i) => {
                if self.msg == Msg::Unavailable {
                    self.msg = Msg::None;
                }
                if !i.full_name.is_empty() {
                    self.name = i.full_name;
                }
                self.username = i.username;
                self.hint = i.password_hint;
                if !i.has_password {
                    self.msg = Msg::NoPassword;
                } else if i.locked_out_secs > 0 {
                    self.lock_for(i.locked_out_secs, t);
                }
            }
        }
    }

    /// While the helper is unavailable, asks again every `RETRY_MS` on a worker (a call may block for its 10 s
    /// timeout), so a restarted privd brings the window back without a re-lock. True when something changed.
    fn retry(&mut self, t: f64) -> bool {
        if let Some(rx) = &self.probe {
            let got = rx.lock().map(|rx| rx.try_recv()).unwrap_or(Err(TryRecvError::Disconnected));
            return match got {
                Err(TryRecvError::Empty) => false,
                Ok(info) => {
                    self.probe = None;
                    self.account_info(info, t);
                    true
                }
                Err(TryRecvError::Disconnected) => {
                    self.probe = None;
                    false
                }
            };
        }
        if self.msg == Msg::Unavailable && self.pending.is_none() && t >= self.retry_at {
            self.retry_at = t + RETRY_MS;
            let (tx, rx) = channel();
            let backend = self.backend.clone();
            std::thread::spawn(move || {
                let _ = tx.send(backend.account());
            });
            self.probe = Some(Mutex::new(rx));
        }
        false
    }

    fn lock_for(&mut self, secs: u64, t: f64) {
        self.until_ms = t + secs as f64 * 1000.0;
        self.msg = Msg::LockedOut;
    }

    pub fn resize(&mut self, w: i32, h: i32) {
        self.w = w;
        self.h = h;
    }

    /// Dots in the field.
    pub fn len(&self) -> usize {
        self.password.chars().count()
    }

    pub fn is_empty(&self) -> bool {
        self.password.is_empty()
    }

    pub fn busy(&self) -> bool {
        self.pending.is_some()
    }

    pub fn locked(&self, t: f64) -> bool {
        self.msg == Msg::LockedOut && t < self.until_ms
    }

    /// Whole seconds left of a lockout (0 when none).
    pub fn secs_left(&self, t: f64) -> u64 {
        if self.locked(t) {
            ((self.until_ms - t) / 1000.0).ceil() as u64
        } else {
            0
        }
    }

    pub fn shaking(&self, t: f64) -> bool {
        self.shake.is_some_and(|s| t - s < SHAKE_MS)
    }

    /// Sideways shake in -1..=1 (multiply by a distance).
    pub fn shake_phase(&self, t: f64) -> f32 {
        self.shake.map_or(0.0, |s0| {
            let p = ((t - s0) / SHAKE_MS).clamp(0.0, 1.0) as f32;
            (p * 6.0 * std::f32::consts::PI).sin() * (1.0 - p)
        })
    }

    /// The text of the message line.
    pub fn message(&self, t: f64) -> String {
        match &self.msg {
            Msg::None => String::new(),
            Msg::Wrong => "Incorrect password".into(),
            Msg::NameMismatch => "Unknown account name".into(),
            Msg::LockedOut => {
                let s = self.secs_left(t);
                if s == 0 {
                    String::new()
                } else if s >= 60 {
                    let m = s.div_ceil(60);
                    format!("Try again in {m} minute{}", if m == 1 { "" } else { "s" })
                } else {
                    format!("Try again in {s} second{}", if s == 1 { "" } else { "s" })
                }
            }
            Msg::Unavailable => "Login service unavailable".into(),
            Msg::NoPassword => "No password is set for this account".into(),
        }
    }

    /// The password hint line, once the wrong passwords reach `HINT_AFTER` and the account has a hint.
    pub fn hint_line(&self) -> Option<String> {
        if self.wrong_count < HINT_AFTER || self.hint.is_empty() || self.msg == Msg::Unavailable {
            return None;
        }
        Some(format!("Hint: {}", self.hint))
    }

    /// Up to two letters for the avatar disc.
    pub fn initials(&self) -> String {
        let words: Vec<char> = self.name.split_whitespace().filter_map(|w| w.chars().next()).collect();
        let pick = match words.as_slice() {
            [] => vec![],
            [a] => vec![*a],
            [a, .., z] => vec![*a, *z],
        };
        pick.into_iter().flat_map(char::to_uppercase).collect()
    }

    /// The accounts row at the bottom: `users` is (user name, full name) of every person's account, `me` this
    /// session's user name. One account shows no list (the avatar above already is that account).
    pub fn set_accounts(&mut self, users: &[(String, String)], me: &str) {
        self.accounts = if users.len() > 1 {
            users.iter().map(|(name, full)| (if full.is_empty() { name.clone() } else { full.clone() }, name == me)).collect()
        } else {
            Vec::new()
        };
    }

    /// The account list's rows (full name, signed in, avatar rect), stacked under the password field.
    fn account_rows(&self, ui: &Ui) -> Vec<(&str, bool, Rect)> {
        let (d, gap) = (ui.pt(30.0), ui.pt(12.0));
        // under the field, past the room its Caps Lock, message and hint lines take
        let (_, fy, _, fh) = self.field(ui);
        let top = fy + fh + ui.pt(72.0);
        let x = self.w as f32 / 2.0 - ui.pt(80.0);
        self.accounts.iter().enumerate().map(|(i, (name, me))| (name.as_str(), *me, (x, top + i as f32 * (d + gap), d, d))).collect()
    }

    /// Edits the field: text, backspace and Escape (clear). Return submits.
    pub fn key(&mut self, out: &Out, t: f64) -> Act {
        if self.sheet.is_some() {
            match out {
                Out::Enter => return self.confirm(),
                Out::Named("Escape") | Out::Hide => self.sheet = None,
                _ => {}
            }
            return Act::None;
        }
        if self.power_open && matches!(out, Out::Named("Escape") | Out::Hide) {
            self.power_open = false;
            return Act::None;
        }
        if matches!(out, Out::Hide) && self.kb_up {
            self.request_keyboard(false);
            return Act::None;
        }
        if self.pending.is_some() {
            return Act::None;
        }
        if self.name_mode && self.name_focus {
            match out {
                Out::Text(s) => {
                    if self.locked(t) || self.msg == Msg::Unavailable {
                        return Act::None;
                    }
                    if matches!(self.msg, Msg::Wrong | Msg::NameMismatch) {
                        self.msg = Msg::None;
                    }
                    for c in s.chars().filter(|c| !c.is_control()) {
                        if self.typed_name.chars().count() < MAX_LEN {
                            self.typed_name.push(c);
                        }
                    }
                }
                Out::Backspace => {
                    self.typed_name.pop();
                }
                Out::Named("Escape") => self.typed_name.clear(),
                Out::Enter => {
                    if !self.typed_name.trim().is_empty() {
                        self.name_focus = false;
                    }
                }
                _ => {}
            }
            return Act::None;
        }
        match out {
            Out::Text(s) => {
                if self.locked(t) || self.msg == Msg::Unavailable {
                    return Act::None;
                }
                if self.msg == Msg::Wrong {
                    self.msg = Msg::None;
                }
                for c in s.chars().filter(|c| !c.is_control()) {
                    if self.len() < MAX_LEN {
                        self.password.push(c);
                    }
                }
            }
            Out::Backspace => {
                self.password.pop();
            }
            Out::Named("Escape") => wipe(&mut self.password),
            Out::Enter => self.submit(t),
            _ => {}
        }
        Act::None
    }

    // -- the on-screen keyboard -----------------------------------------------------

    /// The keyboard is up for this window (its keys type into the focused field).
    pub fn keyboard_up(&self) -> bool {
        self.kb_up
    }

    /// Asks for the keyboard to go up or down. Only the touch form raises it; hiding always works.
    fn request_keyboard(&mut self, on: bool) {
        if on && !self.touch_keyboard {
            return;
        }
        if self.kb_up != on {
            self.kb_up = on;
            self.kb_request = Some(on);
        }
    }

    /// The shell could not raise the keyboard (no keyboard package, a hardware keyboard, the form): keys keep going to
    /// the field from the hardware.
    fn keyboard_refused(&mut self) {
        self.kb_up = false;
    }

    /// The raise or hide the shell has still to carry out.
    pub fn take_keyboard_request(&mut self) -> Option<bool> {
        self.kb_request.take()
    }

    /// Where the keyboard's top edge rests (`None` when it is down): the account group moves up so the field, the
    /// message lines under it and the user picture stay above it.
    pub fn set_keyboard_top(&mut self, ui: &Ui, top: Option<f32>, t: f64) {
        let to = top.map_or(0.0, |k| self.lift_for(ui, k));
        let end = self.lift_anim.map_or(self.lift, |(_, _, to)| to);
        if (to - end).abs() > 0.5 {
            self.lift_anim = Some((t, self.lift, to));
        }
    }

    /// The move (px) that puts the group above a keyboard whose top edge is at `kb_top`, without pushing its top edge
    /// into the status bar.
    fn lift_for(&self, ui: &Ui, kb_top: f32) -> f32 {
        let (top, d) = (self.h as f32 * GROUP_TOP, ui.pt(72.0));
        let (fh, gap) = (ui.pt(30.0), ui.pt(10.0));
        let field_top = top + d + ui.pt(48.0);
        // room under the field for the caps, error and hint lines
        let bottom = field_top + fh + ui.pt(60.0);
        let highest = if self.name_mode { field_top - fh - gap } else { top };
        let need = (bottom - kb_top).max(0.0);
        let room = (highest - ui.pt(super::scene::STATUS_PT) - ui.pt(12.0)).max(0.0);
        need.min(room)
    }

    /// Moves the account group along its animation. True while it moves.
    fn tick_lift(&mut self, t: f64) -> bool {
        let Some((t0, from, to)) = self.lift_anim else { return false };
        let p = ((t - t0) / LIFT_MS).clamp(0.0, 1.0) as f32;
        let e = 1.0 - (1.0 - p).powi(3);
        self.lift = from + (to - from) * e;
        if p >= 1.0 {
            self.lift = to;
            self.lift_anim = None;
        }
        true
    }

    /// A tap on the password field or the user picture: the keys go to the password and the keyboard comes up.
    fn focus_password(&mut self) {
        self.name_focus = false;
        self.request_keyboard(true);
    }

    /// Starts the check on a worker thread. An empty field only submits for an account without a password.
    pub fn submit(&mut self, t: f64) {
        if self.pending.is_some() || self.locked(t) || self.msg == Msg::Unavailable {
            return;
        }
        if self.name_mode {
            // the name is checked here, before the password reaches the helper, so a wrong name costs no attempt
            if !self.name_matches() {
                wipe(&mut self.password);
                self.name_focus = true;
                self.shake = Some(t);
                self.msg = Msg::NameMismatch;
                return;
            }
        }
        if self.password.is_empty() && self.msg != Msg::NoPassword {
            return;
        }
        let pw = std::mem::take(&mut self.password);
        let backend = self.backend.clone();
        let (tx, rx) = channel();
        self.pending = Some(Mutex::new(rx));
        std::thread::spawn(move || {
            let mut pw = pw;
            let r = backend.authenticate(&pw);
            wipe(&mut pw);
            let _ = tx.send(r);
        });
    }

    /// Takes a finished reply, if any. `Some(token)` on success. Call every tick.
    pub fn poll(&mut self, t: f64) -> Option<String> {
        let got = self.pending.as_ref()?.lock().map(|rx| rx.try_recv()).unwrap_or(Err(TryRecvError::Disconnected));
        let r = match got {
            Ok(r) => r,
            Err(TryRecvError::Empty) => return None,
            Err(TryRecvError::Disconnected) => Reply::Unavailable,
        };
        self.pending = None;
        self.apply(r, t)
    }

    pub fn apply(&mut self, r: Reply, t: f64) -> Option<String> {
        match r {
            Reply::Ok(token) => {
                self.msg = Msg::None;
                self.wrong_count = 0;
                return Some(token);
            }
            Reply::Wrong { retry_secs } => {
                // the field was emptied on submit; a wrong password shakes it and counts towards the hint and the wait
                self.shake = Some(t);
                self.wrong_count += 1;
                self.msg = Msg::Wrong;
                let wait = retry_secs.max(local_backoff(self.wrong_count));
                if wait > 0 {
                    self.lock_for(wait, t);
                }
            }
            Reply::LockedOut { retry_secs } => self.lock_for(retry_secs.max(1), t),
            Reply::NoPassword => self.msg = Msg::NoPassword,
            Reply::Unavailable => self.msg = Msg::Unavailable,
        }
        None
    }

    /// Clears an expired lockout; true while the window still needs frames.
    pub fn tick(&mut self, t: f64) -> bool {
        let moving = self.tick_lift(t);
        if self.retry(t) || moving {
            return true;
        }
        if self.msg == Msg::LockedOut && t >= self.until_ms {
            self.msg = Msg::None;
            return true;
        }
        self.shaking(t) || self.locked(t) || self.pending.is_some()
    }

    /// When the window next needs a frame.
    pub fn wake(&self, t: f64) -> Option<f64> {
        if self.shaking(t) || self.pending.is_some() || self.lift_anim.is_some() {
            Some(t)
        } else if self.locked(t) {
            Some(t + 1000.0)
        } else if self.probe.is_some() {
            Some(t + 100.0)
        } else if self.msg == Msg::Unavailable {
            Some(self.retry_at.max(t))
        } else {
            None
        }
    }

    fn confirm(&mut self) -> Act {
        match self.sheet.take() {
            Some(Sheet::Restart) => Act::Restart,
            Some(Sheet::ShutDown) => Act::ShutDown,
            None => Act::None,
        }
    }

    // -- geometry ---------------------------------------------------------------

    /// The account group (avatar, name, field) sits low in the middle, as in the reference; the clock keeps the top.
    fn avatar_rect(&self, ui: &Ui) -> Rect {
        let d = ui.pt(72.0);
        (self.w as f32 / 2.0 - d / 2.0, self.h as f32 * GROUP_TOP - self.lift, d, d)
    }

    fn field(&self, ui: &Ui) -> Rect {
        let (_, top, _, d) = self.avatar_rect(ui);
        let (fw, fh) = (ui.pt(220.0), ui.pt(30.0));
        (self.w as f32 / 2.0 - fw / 2.0, top + d + ui.pt(48.0), fw, fh)
    }

    /// Name mode: the name field, one row above the password field.
    fn name_field(&self, ui: &Ui) -> Rect {
        let (fx, fy, fw, fh) = self.field(ui);
        (fx, fy - fh - ui.pt(10.0), fw, fh)
    }

    /// Name mode: the typed name is this account (case-insensitive, outer spaces ignored).
    fn name_matches(&self) -> bool {
        !self.typed_name.trim().is_empty() && self.username.eq_ignore_ascii_case(self.typed_name.trim())
    }

    fn arrow(&self, ui: &Ui) -> Rect {
        let (x, y, w, h) = self.field(ui);
        let d = h - ui.pt(8.0);
        (x + w - d - ui.pt(4.0), y + ui.pt(4.0), d, d)
    }

    /// The power button, rightmost in the status bar (the status icons move left by `status_inset`).
    fn power_button(&self, ui: &Ui) -> Rect {
        let d = ui.pt(POWER_PT);
        (self.w as f32 - ui.pt(10.0) - d, (ui.pt(super::scene::STATUS_PT) / 2.0 - d / 2.0).max(ui.pt(3.0)), d, d)
    }

    /// The menu under the power button, while it is open.
    fn power_menu(&self, ui: &Ui) -> Rect {
        let (bx, by, bw, bh) = self.power_button(ui);
        let (mw, pad) = (ui.pt(170.0), ui.pt(5.0));
        (bx + bw - mw, by + bh + ui.pt(6.0), mw, 3.0 * ui.pt(24.0) + 2.0 * pad)
    }

    fn bottom(&self, ui: &Ui) -> Vec<(Hit, &'static str, Rect)> {
        if !self.power_open {
            return vec![];
        }
        let (x, y, w, _) = self.power_menu(ui);
        let (rh, pad) = (ui.pt(24.0), ui.pt(5.0));
        let r = |i: f32| (x + pad, y + pad + i * rh, w - 2.0 * pad, rh);
        vec![(Hit::Sleep, "Sleep", r(0.0)), (Hit::Restart, "Restart\u{2026}", r(1.0)), (Hit::ShutDown, "Shut Down\u{2026}", r(2.0))]
    }

    fn sheet_rect(&self, ui: &Ui) -> Rect {
        let (sw, sh) = (ui.pt(300.0), ui.pt(120.0));
        (self.w as f32 / 2.0 - sw / 2.0, self.h as f32 / 2.0 - sh / 2.0, sw, sh)
    }

    fn sheet_buttons(&self, ui: &Ui) -> [(Hit, Rect); 2] {
        let (x, y, w, h) = self.sheet_rect(ui);
        let (bw, bh, m) = (ui.pt(120.0), ui.pt(28.0), ui.pt(16.0));
        [(Hit::Cancel, (x + m, y + h - m - bh, bw, bh)), (Hit::Confirm, (x + w - m - bw, y + h - m - bh, bw, bh))]
    }

    fn hit(&self, ui: &Ui, x: f32, y: f32) -> Option<Hit> {
        let inside = |(rx, ry, rw, rh): Rect| x >= rx && x < rx + rw && y >= ry && y < ry + rh;
        if self.sheet.is_some() {
            return self.sheet_buttons(ui).into_iter().find(|(_, r)| inside(*r)).map(|(h, _)| h);
        }
        if inside(self.power_button(ui)) {
            return Some(Hit::Power);
        }
        if let Some(h) = self.bottom(ui).into_iter().find(|(_, _, r)| inside(*r)).map(|(h, _, _)| h) {
            return Some(h);
        }
        if self.name_mode && inside(self.name_field(ui)) {
            return Some(Hit::Name);
        }
        if inside(self.arrow(ui)) {
            return Some(Hit::Submit);
        }
        if inside(self.field(ui)) {
            return Some(Hit::Field);
        }
        if !self.name_mode && inside(self.avatar_rect(ui)) {
            return Some(Hit::Avatar);
        }
        None
    }

    /// Touch and pointer: the arrow submits, the fields take the keys (and raise the keyboard in the touch form), the
    /// bottom row acts, a sheet confirms or cancels.
    pub fn touch(&mut self, ui: &Ui, ev: Event, t: f64) -> Act {
        let Event::Up(x, y) = ev else { return Act::None };
        let hit = self.hit(ui, x, y);
        // any click closes the power menu; a click on the button itself toggles it
        let was_open = std::mem::take(&mut self.power_open);
        match hit {
            Some(Hit::Power) => self.power_open = !was_open,
            Some(Hit::Submit) => self.submit(t),
            Some(Hit::Name) => {
                self.name_focus = true;
                self.request_keyboard(true);
            }
            // the pointer form keeps its behaviour: the keys go where they went before
            Some(Hit::Field | Hit::Avatar) if self.touch_keyboard => self.focus_password(),
            Some(Hit::Field | Hit::Avatar) => {}
            Some(Hit::Sleep) => return Act::Sleep,
            Some(Hit::Restart) => {
                self.sheet = Some(Sheet::Restart);
                self.request_keyboard(false);
            }
            Some(Hit::ShutDown) => {
                self.sheet = Some(Sheet::ShutDown);
                self.request_keyboard(false);
            }
            Some(Hit::Confirm) => return self.confirm(),
            Some(Hit::Cancel) => self.sheet = None,
            None => {}
        }
        Act::None
    }

    // -- drawing ----------------------------------------------------------------

    pub fn draw(&self, s: &mut Surface, ui: &Ui, t: f64) {
        let (w, h) = (self.w as f32, self.h as f32);
        s.fill_rrect(0.0, 0.0, w, h, 0.0, scrim());
        let cx = self.w / 2;
        let shake = self.shake_phase(t) * ui.pt(14.0);
        if !self.name_mode {
            let (_, top, d, _) = self.avatar_rect(ui);
            s.fill_rrect(cx as f32 - d / 2.0, top, d, d, d / 2.0, avatar());
            let ini = ui.text(&self.initials(), 28.0, Weight::Medium, rgba(255, 255, 255, 255));
            centred(s, &ini, cx, (top + d / 2.0 - ini.h as f32 / 2.0) as i32);
            // over the wallpaper the name is white in both appearances, as the reference draws it
            let name = ui.text(&self.name, 17.0, Weight::Semibold, rgba(255, 255, 255, 255));
            centred(s, &name, cx, (top + d + ui.pt(12.0)) as i32);
        } else {
            let (nx, ny, nw, nh) = self.name_field(ui);
            s.fill_rrect(nx + shake, ny, nw, nh, nh / 2.0, field());
            let (text, col) = if self.typed_name.is_empty() { ("Name", ink_2()) } else { (self.typed_name.as_str(), ink()) };
            let nt = ui.text(text, 13.0, Weight::Regular, col);
            s.blit(&nt, (nx + shake + ui.pt(14.0)) as i32, (ny + (nh - nt.h as f32) / 2.0) as i32, 255, 255);
        }
        let (fx, fy, fw, fh) = self.field(ui);
        let fxs = fx + shake;
        s.fill_rrect(fxs, fy, fw, fh, fh / 2.0, field());
        let (dot, gap) = (ui.pt(8.0), ui.pt(5.0));
        let room = fw - fh - ui.pt(14.0);
        let fit = ((room + gap) / (dot + gap)).floor().max(1.0) as usize;
        let n = self.len();
        if n == 0 {
            let ph = ui.text("Enter Password", 13.0, Weight::Regular, ink_2());
            s.blit(&ph, (fxs + ui.pt(14.0)) as i32, (fy + (fh - ph.h as f32) / 2.0) as i32, 255, 255);
        } else {
            for i in 0..n.min(fit) {
                let x = fxs + ui.pt(14.0) + i as f32 * (dot + gap);
                s.fill_rrect(x, fy + (fh - dot) / 2.0, dot, dot, dot / 2.0, ink());
            }
        }
        // the return arrow, lit once there is something to send
        let (ax, ay, aw, ah) = self.arrow(ui);
        let live = (n > 0 || self.msg == Msg::NoPassword) && !self.busy();
        s.fill_rrect(ax + shake, ay, aw, ah, aw / 2.0, if live { rgba(255, 255, 255, 235) } else { rgba(255, 255, 255, 60) });
        let arrow = ui.text("\u{2192}", 15.0, Weight::Semibold, rgba(40, 40, 42, 255));
        centred(s, &arrow, (ax + aw / 2.0 + shake) as i32, (ay + (ah - arrow.h as f32) / 2.0) as i32);
        let mut line = fy + fh + ui.pt(12.0);
        if self.caps {
            let c = ui.text("Caps Lock is on", 12.0, Weight::Regular, ink_2());
            centred(s, &c, cx, line as i32);
            line += ui.pt(20.0);
        }
        let m = self.message(t);
        if !m.is_empty() {
            let col = if self.msg == Msg::Wrong { bad() } else { ink_2() };
            let mi = ui.text(&m, 12.0, Weight::Regular, col);
            centred(s, &mi, cx, line as i32);
        }
        if let Some(h) = self.hint_line() {
            line += ui.pt(20.0);
            let hi = ui.text(&h, 12.0, Weight::Regular, ink_2());
            centred(s, &hi, cx, line as i32);
        }
        for (full, me, (ax, ay, ad, _)) in self.account_rows(ui) {
            s.fill_rrect(ax, ay, ad, ad, ad / 2.0, avatar());
            let words: Vec<char> = full.split_whitespace().filter_map(|w| w.chars().next()).collect();
            let ini: String = match words.as_slice() {
                [] => String::new(),
                [a] => a.to_uppercase().collect(),
                [a, .., z] => a.to_uppercase().chain(z.to_uppercase()).collect(),
            };
            let it = ui.text(&ini, 12.0, Weight::Medium, rgba(255, 255, 255, 255));
            centred(s, &it, (ax + ad / 2.0) as i32, (ay + (ad - it.h as f32) / 2.0) as i32);
            if me {
                // the signed-in account carries a check badge at its lower right, as the reference draws it
                let b = ui.pt(11.0);
                let (bx, by) = (ax + ad - b * 0.8, ay + ad - b * 0.8);
                s.fill_rrect(bx, by, b, b, b / 2.0, super::ui::LINK);
                let ck = ui.text("\u{2713}", 8.0, Weight::Bold, rgba(255, 255, 255, 255));
                centred(s, &ck, (bx + b / 2.0) as i32, (by + (b - ck.h as f32) / 2.0) as i32);
            }
            let nt = ui.text(full, 13.0, Weight::Semibold, rgba(255, 255, 255, 255));
            s.blit(&nt, (ax + ad + ui.pt(10.0)) as i32, (ay + (ad - nt.h as f32) / 2.0) as i32, 255, 255);
        }
        if let Some(m) = &self.lock_message {
            let mi = ui.text(m, 13.0, Weight::Regular, ink_2());
            centred(s, &mi, cx, (h - ui.pt(64.0) - mi.h as f32) as i32);
        }
        let (px, py, pd, _) = self.power_button(ui);
        if self.power_open {
            s.fill_rrect(px - ui.pt(3.0), py - ui.pt(3.0), pd + ui.pt(6.0), pd + ui.pt(6.0), ui.pt(6.0), rgba(255, 255, 255, 60));
        }
        s.blit_colored(&power_glyph(pd as usize), px as i32, py as i32, rgba(255, 255, 255, 255), 255);
        if self.power_open {
            let (mx, my, mw, mh) = self.power_menu(ui);
            s.fill_rrect(mx, my, mw, mh, ui.pt(10.0), sheet());
            for (_, label, (x, y, _, bh)) in self.bottom(ui) {
                let l = ui.text(label, 13.0, Weight::Regular, super::ui::LABEL.now());
                s.blit(&l, (x + ui.pt(9.0)) as i32, (y + (bh - l.h as f32) / 2.0) as i32, 255, 255);
            }
        }
        if let Some(which) = self.sheet {
            s.fill_rrect(0.0, 0.0, w, h, 0.0, rgba(0, 0, 0, 90));
            let (x, y, sw, sh) = self.sheet_rect(ui);
            s.fill_rrect(x, y, sw, sh, ui.pt(13.0), sheet());
            let (title, ok) = match which {
                Sheet::Restart => ("Are you sure you want to restart?", "Restart"),
                Sheet::ShutDown => ("Are you sure you want to shut down?", "Shut Down"),
            };
            let ti = ui.text(title, 13.0, Weight::Semibold, super::ui::LABEL.now());
            centred(s, &ti, (x + sw / 2.0) as i32, (y + ui.pt(24.0)) as i32);
            for (hit, (bx, by, bw, bh)) in self.sheet_buttons(ui) {
                let primary = hit == Hit::Confirm;
                s.fill_rrect(bx, by, bw, bh, ui.pt(6.0), if primary { super::ui::LINK } else { rgba(128, 128, 128, 70) });
                let ink = if primary { rgba(255, 255, 255, 255) } else { super::ui::LABEL.now() };
                let lab = ui.text(if primary { ok } else { "Cancel" }, 13.0, Weight::Regular, ink);
                centred(s, &lab, (bx + bw / 2.0) as i32, (by + (bh - lab.h as f32) / 2.0) as i32);
            }
        }
    }
}

impl Drop for Login {
    fn drop(&mut self) {
        wipe(&mut self.password);
    }
}

// -- the real backend: privd over its socket ------------------------------------------

/// Talks the privd frame protocol: 4-byte big-endian length, then JSON.
pub struct Privd {
    path: std::path::PathBuf,
}

impl Privd {
    /// Set by the session script. Fails closed: when the variable is missing on a machine that has the BifrOSt
    /// session (`/usr/lib/bifrost/session`) or a privd socket, the default socket is used, so a misconfigured
    /// session shows "Login service unavailable" and stays locked instead of falling back to the old PIN path.
    /// `None` only for host previews and machines without BifrOSt.
    pub fn from_env() -> Option<Self> {
        Self::resolve(std::env::var_os("SPRINGBOARD_PRIVD"), std::path::Path::new("/usr/lib/bifrost/session").exists() || std::path::Path::new(crate::privd::SOCK).exists())
    }

    /// `from_env` with its inputs given, for tests.
    pub fn resolve(env: Option<std::ffi::OsString>, installed: bool) -> Option<Self> {
        match env {
            Some(p) => Some(Privd { path: p.into() }),
            None if installed => Some(Privd { path: crate::privd::SOCK.into() }),
            None => None,
        }
    }

    fn call(&self, method: &str, params: &str) -> Option<String> {
        use std::io::{Read, Write};
        let mut s = std::os::unix::net::UnixStream::connect(&self.path).ok()?;
        let to = Some(std::time::Duration::from_secs(10));
        s.set_read_timeout(to).ok()?;
        s.set_write_timeout(to).ok()?;
        let body = format!("{{\"id\":1,\"method\":\"{method}\",\"params\":{params}}}");
        s.write_all(&(body.len() as u32).to_be_bytes()).ok()?;
        s.write_all(body.as_bytes()).ok()?;
        let mut len = [0u8; 4];
        s.read_exact(&mut len).ok()?;
        let n = u32::from_be_bytes(len) as usize;
        if n > 64 * 1024 {
            return None;
        }
        let mut buf = vec![0u8; n];
        s.read_exact(&mut buf).ok()?;
        String::from_utf8(buf).ok()
    }

    /// Fire-and-forget verbs (`session.lock`, `power.restart`, ...) from a worker so the UI never waits.
    pub fn notify(method: &'static str) {
        if let Some(p) = Privd::from_env() {
            std::thread::spawn(move || {
                let _ = p.call(method, "{}");
            });
        }
    }
}

fn json_str(text: &str, key: &str) -> Option<String> {
    let pat = format!("\"{key}\":\"");
    let i = text.find(&pat)? + pat.len();
    let mut out = String::new();
    let mut chars = text[i..].chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => return Some(out),
            '\\' => match chars.next()? {
                'n' => out.push('\n'),
                't' => out.push('\t'),
                'u' => {
                    let hex: String = chars.by_ref().take(4).collect();
                    out.push(char::from_u32(u32::from_str_radix(&hex, 16).ok()?)?);
                }
                o => out.push(o),
            },
            c => out.push(c),
        }
    }
    None
}

fn json_num(text: &str, key: &str) -> Option<u64> {
    let pat = format!("\"{key}\":");
    let i = text.find(&pat)? + pat.len();
    let digits: String = text[i..].trim_start().chars().take_while(char::is_ascii_digit).collect();
    digits.parse().ok()
}

fn json_bool(text: &str, key: &str) -> Option<bool> {
    let pat = format!("\"{key}\":");
    let rest = text[text.find(&pat)? + pat.len()..].trim_start();
    Some(rest.starts_with("true"))
}

fn json_escape(s: &str) -> String {
    let mut o = String::with_capacity(s.len() + 2);
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            c if (c as u32) < 0x20 => o.push_str(&format!("\\u{:04x}", c as u32)),
            c => o.push(c),
        }
    }
    o
}

/// Parses a `login.authenticate` reply frame. Anything unparseable is `Unavailable`, which never unlocks.
pub fn parse_reply(text: &str) -> Reply {
    use bifrost_core::json::{self, Json};
    let Ok(j) = json::parse(text) else { return Reply::Unavailable };
    if j.get("ok") == Some(&Json::Bool(true)) {
        return j.get("result").and_then(|r| r.str_field("token")).map_or(Reply::Unavailable, Reply::Ok);
    }
    let e = j.get("error");
    let retry = e.and_then(|e| e.get("retry_secs")).and_then(Json::as_u64).unwrap_or(0);
    match e.and_then(|e| e.str_field("code")).as_deref() {
        Some("wrong_password") => Reply::Wrong { retry_secs: retry },
        Some("locked_out") => Reply::LockedOut { retry_secs: retry },
        Some("no_password") => Reply::NoPassword,
        _ => Reply::Unavailable,
    }
}

impl Backend for Privd {
    fn authenticate(&self, password: &str) -> Reply {
        let params = format!("{{\"password\":\"{}\"}}", json_escape(password));
        self.call("login.authenticate", &params).map_or(Reply::Unavailable, |t| parse_reply(&t))
    }

    fn account(&self) -> Option<AccountInfo> {
        let t = self.call("login.account", "{}")?;
        if json_bool(&t, "ok") != Some(true) {
            return None;
        }
        Some(AccountInfo {
            full_name: json_str(&t, "full_name").unwrap_or_default(),
            username: json_str(&t, "username").unwrap_or_default(),
            has_password: json_bool(&t, "has_password").unwrap_or(true),
            locked_out_secs: json_num(&t, "locked_out_secs").unwrap_or(0),
            password_hint: json_str(&t, "password_hint").unwrap_or_default(),
        })
    }
}

/// The effect for a bottom-row choice (Sleep keeps the session; Restart and Shut Down go through privd first).
pub fn power_effect(act: &Act) -> Option<Effect> {
    match act {
        Act::Restart => Some(Effect::Power(super::power_menu::PowerAction::Restart)),
        Act::ShutDown => Some(Effect::Power(super::power_menu::PowerAction::Off)),
        Act::Sleep => Some(Effect::SleepSystem),
        Act::None => None,
    }
}

/// The account the initramfs logs in (task 10, "Unlock is login"): written to this file by the `bifrost-unlock` hook
/// when the loader unlocked the encrypted root, so the desktop starts without a second login prompt.
pub const AUTOLOGIN: &str = "/run/bifrost/autologin";

/// The account name in an autologin file: one line of letters, digits, `-`, `_` or `.`, at most 64 bytes.
pub fn autologin_name(text: &str) -> Option<String> {
    let name = text.trim();
    let ok = !name.is_empty() && name.len() <= 64 && name.bytes().all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b));
    ok.then(|| name.to_string())
}

/// Reads and consumes the autologin file once, at shell start. `None` when the machine was not unlocked by the loader.
pub fn take_autologin() -> Option<String> {
    let text = std::fs::read_to_string(AUTOLOGIN).ok()?;
    let _ = std::fs::remove_file(AUTOLOGIN);
    autologin_name(&text)
}

// -- the shell's side of the login window ------------------------------------------------

impl super::Shell {
    /// True while the login window owns the lock screen.
    pub(super) fn login_open(&self) -> bool {
        self.login.is_some()
    }

    /// Opens the login window over the lock screen when a root helper is configured (once per lock).
    pub(super) fn login_ensure(&mut self, t: f64) {
        if self.login.is_some() || self.screen != super::Screen::Lock || self.anim.is_some() || self.session_token.is_some() {
            return;
        }
        self.login_open_window(t);
    }

    /// Builds the login window: the lock message, the name-and-password option, the account list and the Caps Lock
    /// state. Both ways in (a tick and a lock or key that cannot wait for one) open the same window. A backend that
    /// was missing when the shell started (privd's socket came up later) is looked up again here, so a lock screen on
    /// an installed system always has a working field; without one (host previews) nothing opens.
    fn login_open_window(&mut self, t: f64) {
        #[cfg(not(test))]
        if self.login_backend.is_none() {
            self.login_backend = Privd::from_env().map(|p| Arc::new(p) as Arc<dyn Backend>);
        }
        let Some(b) = self.login_backend.clone() else {
            // no login service: the lock screen has no field. Said once per lock (the next tick and every key look the
            // service up again), so a lock that cannot be opened is in the journal instead of failing silently (E2E N0)
            if !self.login_missing_logged {
                self.login_missing_logged = true;
                eprintln!("springboard: login window not opened: no login service (SPRINGBOARD_PRIVD unset, no privd socket); retrying on the next key");
            }
            return;
        };
        self.login_missing_logged = false;
        let me = super::account::load();
        let mut l = Login::new(b, &me.full_name, self.w, self.h, t);
        l.lock_message = load_lock_message();
        l.name_mode = self.options().login_name_password;
        l.name_focus = l.name_mode;
        l.caps = self.key_mods.caps_lock();
        if !l.name_mode {
            let read = |p: &str| std::fs::read_to_string(p).unwrap_or_default();
            let view = super::users_page::View::from_text(&read("/etc/passwd"), &read("/etc/group"), &me.username);
            let users: Vec<(String, String)> = view.users.into_iter().map(|u| (u.name, u.full_name)).collect();
            l.set_accounts(&users, &me.username);
        }
        self.login = Some(l);
        self.dirty = true;
    }

    /// Caps Lock changed (every key event passes the held modifiers): the login window's hint follows it.
    pub(super) fn login_caps(&mut self, on: bool) {
        if let Some(l) = self.login.as_mut().filter(|l| l.caps != on) {
            l.caps = on;
            self.dirty = true;
        }
    }

    /// Brings the login window up at once (input that arrives before the next tick must not reach the old unlock):
    /// on the lock screen, with a backend, before a token.
    pub(super) fn login_show(&mut self, t: f64) {
        self.lock_drag = None;
        if self.login.is_some() || self.screen != super::Screen::Lock || self.session_token.is_some() {
            return;
        }
        self.login_open_window(t);
    }

    pub(super) fn login_touch(&mut self, ev: Event, t: f64) -> Vec<Effect> {
        if let Some(fx) = self.login_keyboard_touch(ev, t) {
            return fx;
        }
        let Some(l) = self.login.as_mut() else { return vec![] };
        let act = l.touch(&self.ui, ev, t);
        self.dirty = true;
        self.login_sync_keyboard(t);
        self.login_act(act)
    }

    pub(super) fn login_key(&mut self, out: &Out, t: f64) -> Vec<Effect> {
        let Some(l) = self.login.as_mut() else { return vec![] };
        let act = l.key(out, t);
        self.dirty = true;
        self.login_sync_keyboard(t);
        self.login_act(act)
    }

    /// The on-screen keyboard is up for the login window, so its keys belong to the window.
    pub(super) fn login_owns_keyboard(&self) -> bool {
        self.screen == super::Screen::Lock && self.login.as_ref().is_some_and(Login::keyboard_up)
    }

    /// Touches that land on the keyboard go to it; what it types goes into the login field. `None` when the touch is
    /// not the keyboard's.
    fn login_keyboard_touch(&mut self, ev: Event, t: f64) -> Option<Vec<Effect>> {
        let up = self.login_owns_keyboard() && self.keyboard.shown();
        let top = self.kb_top();
        match ev {
            Event::Down(x, y) if up && y >= top => {
                self.kb_touch = true;
                self.keyboard.down(x, y - top, t);
            }
            Event::Move(x, y) if self.kb_touch => self.keyboard.moved(x, y - top),
            Event::Up(x, y) if self.kb_touch => {
                self.kb_touch = false;
                self.dirty = true;
                return Some(match self.keyboard.up(x, y - top, t) {
                    Some(out) => self.login_key(&out, t),
                    None => vec![],
                });
            }
            _ => return None,
        }
        self.dirty = true;
        Some(vec![])
    }

    /// Carries out the window's request to raise or hide the keyboard and tells it where the keyboard rests, so the
    /// account group stays above it.
    fn login_sync_keyboard(&mut self, t: f64) {
        if let Some(on) = self.login.as_mut().and_then(Login::take_keyboard_request) {
            if on {
                self.keyboard.set_return_key(bifrost_proto::ReturnKey::Go);
            }
            self.set_keyboard(on, t);
            if on && !self.keyboard.shown() {
                self.keyboard.set_return_key(bifrost_proto::ReturnKey::Return);
                if let Some(l) = self.login.as_mut() {
                    l.keyboard_refused();
                }
            }
        }
        let up = self.login_owns_keyboard() && self.keyboard.shown() && self.keyboard.h() > 0.0;
        let top = up.then(|| self.kb_top());
        if let Some(l) = self.login.as_mut() {
            l.set_keyboard_top(&self.ui, top, t);
        }
    }

    /// The login window goes away (unlocked or locked again): a keyboard it raised goes down with it.
    fn login_close(&mut self, t: f64) {
        let raised = self.login.take().is_some_and(|l| l.keyboard_up());
        if raised {
            self.set_keyboard(false, t);
            self.keyboard.set_return_key(bifrost_proto::ReturnKey::Return);
        }
    }

    fn login_act(&mut self, act: Act) -> Vec<Effect> {
        let unavailable = self.login.as_ref().is_some_and(|l| l.msg == Msg::Unavailable);
        match act {
            Act::None => vec![],
            Act::Sleep => power_effect(&act).into_iter().collect(),
            Act::Restart | Act::ShutDown if unavailable => power_effect(&act).into_iter().collect(),
            Act::Restart => {
                Privd::notify("power.restart");
                vec![]
            }
            Act::ShutDown => {
                Privd::notify("power.shutdown");
                vec![]
            }
        }
    }

    /// Per tick: opens the window, collects the helper's reply and starts the unlock on success.
    pub(super) fn login_tick(&mut self, t: f64) {
        self.login_ensure(t);
        let Some(l) = self.login.as_mut() else { return };
        if let Some(token) = l.poll(t) {
            self.login_close(t);
            self.session_token = Some(token);
            self.anim = Some((super::Anim::Unlock { from: 0.0, to: 1.0 }, t));
            self.dirty = true;
        } else if l.tick(t) {
            self.dirty = true;
        }
        self.login_sync_keyboard(t);
    }

    pub(super) fn login_wake(&self, t: f64) -> Option<f64> {
        let own = self.login_owns_keyboard() && self.keyboard.animating();
        self.login.as_ref()?.wake(t).or(own.then_some(t))
    }

    /// A lock (menu, idle, lid): the session token is dropped and privd forgets the unlock.
    pub(super) fn login_reset(&mut self) {
        self.login_close(self.last_input);
        self.login_missing_logged = false;
        if self.session_token.take().is_some() {
            Privd::notify("session.lock");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Stub(Reply);
    impl Backend for Stub {
        fn authenticate(&self, _: &str) -> Reply {
            self.0.clone()
        }
        fn account(&self) -> Option<AccountInfo> {
            Some(AccountInfo { full_name: "Ada Lovelace".into(), username: "ada".into(), has_password: true, locked_out_secs: 0, password_hint: String::new() })
        }
    }

    fn login(r: Reply) -> Login {
        Login::new(Arc::new(Stub(r)), "Owner", 1200, 800, 0.0)
    }

    #[test]
    fn resize_takes_the_rotated_surface_size() {
        let mut l = login(Reply::Unavailable);
        l.resize(800, 1200);
        assert_eq!((l.w, l.h), (800, 1200));
        l.resize(800, 1200);
        assert_eq!((l.w, l.h), (800, 1200), "the same size again changes nothing");
    }

    fn wait(l: &mut Login, t: f64) -> Option<String> {
        for _ in 0..400 {
            if !l.busy() {
                break;
            }
            if let Some(tok) = l.poll(t) {
                return Some(tok);
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        None
    }

    fn name_mode(r: Reply) -> Login {
        let mut l = login(r);
        l.name_mode = true;
        l.name_focus = true;
        l
    }

    #[test]
    fn name_and_password_mode_checks_the_name_before_the_password() {
        let mut l = name_mode(Reply::Ok("tok".into()));
        l.key(&Out::Text("Ada ".into()), 0.0);
        l.key(&Out::Enter, 0.0);
        assert!(!l.name_focus, "return moves on to the password");
        l.key(&Out::Text("pw".into()), 0.0);
        l.key(&Out::Enter, 0.0);
        assert!(l.busy(), "the matching name sends the password");
        assert_eq!(wait(&mut l, 0.0), Some("tok".to_string()));
    }

    #[test]
    fn a_wrong_name_is_refused_without_an_attempt() {
        let mut l = name_mode(Reply::Ok("tok".into()));
        l.key(&Out::Text("bob".into()), 0.0);
        l.key(&Out::Enter, 0.0);
        l.key(&Out::Text("pw".into()), 0.0);
        l.key(&Out::Enter, 0.0);
        assert!(!l.busy(), "no helper call for another name");
        assert_eq!(l.msg, Msg::NameMismatch);
        assert!(l.is_empty(), "the password is cleared");
        assert!(l.name_focus, "the name field takes the keys again");
        assert_eq!(l.message(0.0), "Unknown account name");
    }

    #[test]
    fn the_name_match_ignores_case_and_outer_spaces_and_an_empty_name_never_matches() {
        let mut l = name_mode(Reply::Ok("tok".into()));
        assert!(!l.name_matches());
        l.key(&Out::Text("  ADA ".into()), 0.0);
        assert!(l.name_matches());
        l.key(&Out::Named("Escape"), 0.0);
        assert!(l.typed_name.is_empty(), "Escape clears the focused name field");
    }

    #[test]
    fn field_edits() {
        let mut l = login(Reply::Unavailable);
        l.key(&Out::Text("ab".into()), 0.0);
        l.key(&Out::Text("c\n".into()), 0.0);
        assert_eq!(l.len(), 3, "control characters are dropped");
        l.key(&Out::Backspace, 0.0);
        assert_eq!(l.len(), 2);
        l.key(&Out::Named("Escape"), 0.0);
        assert!(l.is_empty());
        l.key(&Out::Backspace, 0.0);
        assert!(l.is_empty(), "backspace on empty is harmless");
        assert_eq!(l.initials(), "AL");
    }

    #[test]
    fn right_password_returns_token_and_empties_field() {
        let mut l = login(Reply::Ok("tok".into()));
        l.key(&Out::Text("pw".into()), 0.0);
        l.key(&Out::Enter, 0.0);
        assert!(l.is_empty(), "the buffer is taken on submit");
        assert_eq!(wait(&mut l, 0.0).as_deref(), Some("tok"));
    }

    #[test]
    fn empty_field_does_not_submit() {
        let mut l = login(Reply::Ok("tok".into()));
        l.key(&Out::Enter, 0.0);
        assert!(!l.busy());
    }

    #[test]
    fn wrong_password_shakes_and_counts_down() {
        let mut l = login(Reply::Wrong { retry_secs: 60 });
        l.key(&Out::Text("x".into()), 100.0);
        l.key(&Out::Enter, 100.0);
        assert!(wait(&mut l, 100.0).is_none());
        assert!(l.shaking(200.0));
        assert!(!l.shaking(100.0 + SHAKE_MS + 1.0), "the shake ends after SHAKE_MS");
        assert_eq!(l.message(100.0), "Try again in 1 minute");
        assert!(l.locked(30_000.0));
        assert_eq!(l.secs_left(31_000.0), 30);
    }

    #[test]
    fn lockout_blocks_typing_until_it_expires() {
        let mut l = login(Reply::Unavailable);
        l.apply(Reply::LockedOut { retry_secs: 30 }, 0.0);
        l.key(&Out::Text("x".into()), 1000.0);
        assert!(l.is_empty());
        assert_eq!(l.message(1000.0), "Try again in 29 seconds");
        assert!(l.tick(31_000.0) && l.msg == Msg::None);
        l.key(&Out::Text("x".into()), 31_000.0);
        assert_eq!(l.len(), 1);
    }

    #[test]
    fn plain_wrong_shows_incorrect_until_typing() {
        let mut l = login(Reply::Unavailable);
        l.apply(Reply::Wrong { retry_secs: 0 }, 0.0);
        assert_eq!(l.message(0.0), "Incorrect password");
        l.key(&Out::Text("a".into()), 10.0);
        assert_eq!(l.message(10.0), "");
    }

    /// Down until `up` is set, then a normal account.
    struct Flaky(std::sync::atomic::AtomicBool);
    impl Backend for Flaky {
        fn authenticate(&self, _: &str) -> Reply { Reply::Unavailable }
        fn account(&self) -> Option<AccountInfo> {
            self.0.load(std::sync::atomic::Ordering::SeqCst).then(|| AccountInfo { full_name: "Owner".into(), username: "owner".into(), has_password: true, locked_out_secs: 0, password_hint: String::new() })
        }
    }

    #[test]
    fn an_unavailable_helper_is_asked_again_and_the_window_recovers() {
        let b = Arc::new(Flaky(std::sync::atomic::AtomicBool::new(false)));
        let mut l = Login::new(b.clone(), "Owner", 1200, 800, 0.0);
        assert_eq!(l.msg, Msg::Unavailable);
        assert_eq!(l.wake(0.0), Some(RETRY_MS), "it schedules the next try");
        b.0.store(true, std::sync::atomic::Ordering::SeqCst);
        let mut t = RETRY_MS;
        while l.msg == Msg::Unavailable && t < RETRY_MS + 2000.0 {
            l.tick(t);
            std::thread::sleep(std::time::Duration::from_millis(2));
            t += 10.0;
        }
        assert_eq!(l.msg, Msg::None, "privd is back: the window takes a password again");
    }

    #[test]
    fn unavailable_service_still_allows_power_buttons() {
        let mut l = login(Reply::Unavailable);
        l.apply(Reply::Unavailable, 0.0);
        assert_eq!(l.message(0.0), "Login service unavailable");
        l.key(&Out::Text("a".into()), 0.0);
        assert!(l.is_empty());
        let ui = super::super::tests::ui();
        assert!(l.bottom(&ui).is_empty(), "the power choices live in the power button's menu");
        let (px, py, pw, ph) = l.power_button(&ui);
        l.touch(&ui, Event::Up(px + pw / 2.0, py + ph / 2.0), 0.0);
        let (x, y, w, h) = l.bottom(&ui)[1].2;
        assert_eq!(l.touch(&ui, Event::Up(x + w / 2.0, y + h / 2.0), 0.0), Act::None);
        assert_eq!(l.sheet, Some(Sheet::Restart));
        assert!(!l.power_open, "choosing closes the menu");
        assert_eq!(l.key(&Out::Enter, 0.0), Act::Restart);
    }

    #[test]
    fn the_power_menu_opens_from_the_status_bar_and_closes_on_any_click_or_escape() {
        let mut l = login(Reply::Unavailable);
        let ui = super::super::tests::ui();
        let (px, py, pw, ph) = l.power_button(&ui);
        assert!(px + pw <= l.w as f32 && py >= 0.0, "inside the top-right corner");
        l.touch(&ui, Event::Up(px + pw / 2.0, py + ph / 2.0), 0.0);
        assert_eq!(l.bottom(&ui).iter().map(|b| b.1).collect::<Vec<_>>(), ["Sleep", "Restart\u{2026}", "Shut Down\u{2026}"]);
        l.touch(&ui, Event::Up(1.0, l.h as f32 - 1.0), 0.0);
        assert!(!l.power_open, "a click elsewhere closes it");
        l.touch(&ui, Event::Up(px + pw / 2.0, py + ph / 2.0), 0.0);
        l.key(&Out::Named("Escape"), 0.0);
        assert!(!l.power_open, "Escape closes it");
        let (sx, sy, sw, sh) = { l.power_open = true; l.bottom(&ui)[0].2 };
        assert_eq!(l.touch(&ui, Event::Up(sx + sw / 2.0, sy + sh / 2.0), 0.0), Act::Sleep);
    }

    #[test]
    fn the_password_hint_shows_after_three_wrong_passwords_and_only_when_set() {
        let mut l = login(Reply::Wrong { retry_secs: 0 });
        l.hint = "first pet".into();
        for _ in 0..HINT_AFTER - 1 {
            l.apply(Reply::Wrong { retry_secs: 0 }, 0.0);
        }
        assert_eq!(l.hint_line(), None, "no line before the third wrong password");
        l.apply(Reply::Wrong { retry_secs: 0 }, 0.0);
        assert_eq!(l.hint_line().as_deref(), Some("Hint: first pet"));
        l.apply(Reply::Ok("t".into()), 0.0);
        assert_eq!(l.hint_line(), None, "a success clears the count");
        let mut none = login(Reply::Wrong { retry_secs: 0 });
        none.wrong_count = 9;
        assert_eq!(none.hint_line(), None, "no hint is set");
    }

    #[test]
    fn a_wrong_password_clears_the_field_and_repeated_misses_make_the_window_wait() {
        let mut l = login(Reply::Wrong { retry_secs: 0 });
        l.key(&Out::Text("wrongpass".into()), 0.0);
        assert_eq!(l.len(), 9, "typing shows one dot per character");
        l.key(&Out::Enter, 0.0);
        assert!(wait(&mut l, 0.0).is_none(), "a wrong password never unlocks");
        assert!(l.is_empty(), "the field is cleared");
        assert!(l.shaking(10.0) && l.msg == Msg::Wrong);
        assert!(!l.locked(10.0), "one miss does not make the window wait");
        for _ in 1..BACKOFF_AFTER {
            l.apply(Reply::Wrong { retry_secs: 0 }, 1000.0);
        }
        assert!(l.locked(1000.0 + 29_000.0), "the fifth miss in a row waits 30 s even when privd gave no retry time");
        l.key(&Out::Text("x".into()), 2000.0);
        assert!(l.is_empty(), "typing is refused while the window waits");
        assert!(!l.locked(1000.0 + 31_000.0));
    }

    #[test]
    fn the_local_backoff_doubles_and_is_capped() {
        assert_eq!(local_backoff(0), 0);
        assert_eq!(local_backoff(BACKOFF_AFTER - 1), 0);
        assert_eq!(local_backoff(BACKOFF_AFTER), 30);
        assert_eq!(local_backoff(BACKOFF_AFTER + 1), 60);
        assert_eq!(local_backoff(BACKOFF_AFTER + 3), 240);
        assert_eq!(local_backoff(BACKOFF_AFTER + 4), BACKOFF_MAX_SECS);
        assert_eq!(local_backoff(u32::MAX), BACKOFF_MAX_SECS);
    }

    #[test]
    fn the_account_list_shows_only_for_several_accounts_and_marks_this_one() {
        let mut l = login(Reply::NoPassword);
        l.set_accounts(&[("owner".into(), "Owner".into())], "owner");
        assert!(l.accounts.is_empty(), "one account: the avatar above is the only one");
        l.set_accounts(&[("kid".into(), String::new()), ("owner".into(), "Owner Name".into())], "owner");
        assert_eq!(l.accounts, vec![("kid".to_string(), false), ("Owner Name".to_string(), true)]);
        let ui = super::super::tests::ui();
        let rows = l.account_rows(&ui);
        assert_eq!(rows.len(), 2);
        let (_, _, (_, y0, d, _)) = rows[0];
        let (_, _, (_, y1, _, _)) = rows[1];
        assert!(y1 > y0 + d, "the rows stack downwards");
        let (fx, fy, _, fh) = l.field(&ui);
        assert!(y0 > fy + fh, "the list sits below the password field");
        let _ = fx;
    }

    #[test]
    fn the_caps_lock_hint_is_drawn_from_the_flag() {
        let mut l = login(Reply::NoPassword);
        assert!(!l.caps);
        l.caps = true;
        let ui = super::super::tests::ui();
        let mut px = vec![0u32; 1200 * 800];
        let mut s = Surface { w: 1200, h: 800, stride: 1200, px: &mut px, y0: 0 };
        l.draw(&mut s, &ui, 0.0);
    }

    #[test]
    fn reply_parsing() {
        assert_eq!(parse_reply(r#"{"id":1,"ok":true,"result":{"token":"abc"}}"#), Reply::Ok("abc".into()));
        assert_eq!(parse_reply(r#"{"id":1,"ok":false,"error":{"code":"wrong_password","fails":2,"retry_secs":60}}"#), Reply::Wrong { retry_secs: 60 });
        assert_eq!(parse_reply(r#"{"ok":false,"error":{"code":"locked_out","retry_secs":9}}"#), Reply::LockedOut { retry_secs: 9 });
        assert_eq!(parse_reply("garbage"), Reply::Unavailable);
        // a token without ok:true, or ok:true without a token, never unlocks
        assert_eq!(parse_reply(r#"{"ok":false,"result":{"token":"abc"}}"#), Reply::Unavailable);
        assert_eq!(parse_reply(r#"{"ok":true,"result":{}}"#), Reply::Unavailable);
    }

    #[test]
    fn a_missing_variable_on_an_installed_system_still_gets_the_login_window() {
        assert!(Privd::resolve(None, true).is_some());
        assert!(Privd::resolve(None, false).is_none());
        assert_eq!(Privd::resolve(Some("/x/p.sock".into()), false).unwrap().path, std::path::PathBuf::from("/x/p.sock"));
    }

    #[test]
    fn autologin_accepts_one_account_name_line() {
        assert_eq!(autologin_name("olibuijr\n"), Some("olibuijr".to_string()));
        assert_eq!(autologin_name("  guest.1-a_b  "), Some("guest.1-a_b".to_string()));
    }

    #[test]
    fn autologin_rejects_empty_hostile_and_long_names() {
        assert_eq!(autologin_name(""), None);
        assert_eq!(autologin_name("\n"), None);
        assert_eq!(autologin_name("a b"), None);
        assert_eq!(autologin_name("../root"), None);
        assert_eq!(autologin_name("x\ny"), None);
        assert_eq!(autologin_name(&"a".repeat(65)), None);
        assert_eq!(autologin_name(&"a".repeat(64)).map(|n| n.len()), Some(64));
    }

    #[test]
    fn lock_message_takes_one_clean_line() {
        assert_eq!(parse_lock_message("message=Back at 5\n"), Some("Back at 5".to_string()));
        assert_eq!(parse_lock_message("message=  \n"), None);
        assert_eq!(parse_lock_message("other=x"), None);
        assert_eq!(parse_lock_message("message=a\u{7}b").as_deref(), Some("ab"));
        assert_eq!(parse_lock_message(&format!("message={}", "x".repeat(300))).map(|m| m.chars().count()), Some(MESSAGE_MAX));
    }

    // -- touch: the on-screen keyboard -------------------------------------------------------

    /// A portrait window in the touch form.
    fn touch_login(r: Reply) -> Login {
        let mut l = Login::new(Arc::new(Stub(r)), "Owner", 1200, 1920, 0.0);
        l.touch_keyboard = true;
        l
    }

    fn tap(l: &mut Login, ui: &Ui, (x, y): (f32, f32), t: f64) -> Act {
        l.touch(ui, Event::Up(x, y), t)
    }

    /// A point on the password field, left of the return arrow.
    fn field_point(l: &Login, ui: &Ui) -> (f32, f32) {
        let (x, y, w, h) = l.field(ui);
        (x + w * 0.25, y + h / 2.0)
    }

    fn avatar_point(l: &Login, ui: &Ui) -> (f32, f32) {
        let (x, y, w, h) = l.avatar_rect(ui);
        (x + w / 2.0, y + h / 2.0)
    }

    #[test]
    fn tapping_the_field_or_the_picture_raises_the_keyboard_in_the_touch_form() {
        let ui = super::super::tests::ui();
        let mut l = touch_login(Reply::Unavailable);
        assert!(!l.keyboard_up());
        let p = field_point(&l, &ui);
        assert_eq!(tap(&mut l, &ui, p, 0.0), Act::None);
        assert!(l.keyboard_up());
        assert_eq!(l.take_keyboard_request(), Some(true));
        assert_eq!(l.take_keyboard_request(), None, "a request is taken once");
        let mut l = touch_login(Reply::Unavailable);
        let p = avatar_point(&l, &ui);
        tap(&mut l, &ui, p, 0.0);
        assert_eq!(l.take_keyboard_request(), Some(true), "the picture takes the same tap");
    }

    #[test]
    fn a_click_does_not_raise_the_keyboard_in_the_desktop_form() {
        let ui = super::super::tests::ui();
        let mut l = touch_login(Reply::Unavailable);
        l.touch_keyboard = false;
        let p = field_point(&l, &ui);
        tap(&mut l, &ui, p, 0.0);
        assert!(!l.keyboard_up());
        assert_eq!(l.take_keyboard_request(), None);
    }

    #[test]
    fn tapping_the_name_field_raises_the_keyboard_and_the_password_field_takes_the_focus_back() {
        let ui = super::super::tests::ui();
        let mut l = touch_login(Reply::Unavailable);
        l.name_mode = true;
        l.name_focus = false;
        let (x, y, w, h) = l.name_field(&ui);
        tap(&mut l, &ui, (x + w / 2.0, y + h / 2.0), 0.0);
        assert!(l.name_focus && l.keyboard_up());
        let p = field_point(&l, &ui);
        tap(&mut l, &ui, p, 1.0);
        assert!(!l.name_focus, "keys go to the password again");
    }

    #[test]
    fn a_click_in_the_desktop_form_leaves_the_key_focus_alone() {
        let ui = super::super::tests::ui();
        let mut l = touch_login(Reply::Unavailable);
        l.touch_keyboard = false;
        l.name_mode = true;
        l.name_focus = true;
        let p = field_point(&l, &ui);
        tap(&mut l, &ui, p, 0.0);
        assert!(l.name_focus, "a pointer click on the password field changes nothing in the desktop form");
        assert!(!l.keyboard_up());
    }

    #[test]
    fn a_refused_keyboard_is_not_counted_as_up() {
        let ui = super::super::tests::ui();
        let mut l = touch_login(Reply::Unavailable);
        let p = field_point(&l, &ui);
        tap(&mut l, &ui, p, 0.0);
        assert_eq!(l.take_keyboard_request(), Some(true));
        l.keyboard_refused();
        assert!(!l.keyboard_up());
        tap(&mut l, &ui, p, 1.0);
        assert_eq!(l.take_keyboard_request(), Some(true), "the next tap asks again");
    }

    #[test]
    fn keyboard_keys_type_masked_backspace_deletes_and_return_submits() {
        let ui = super::super::tests::ui();
        let mut l = touch_login(Reply::Ok("tok".into()));
        let p = field_point(&l, &ui);
        tap(&mut l, &ui, p, 0.0);
        for k in ["p", "a", "s", "s"] {
            l.key(&Out::Text(k.into()), 0.0);
        }
        assert_eq!(l.len(), 4, "one dot per key; the text itself is never drawn");
        l.key(&Out::Backspace, 0.0);
        assert_eq!(l.len(), 3);
        l.key(&Out::Enter, 0.0);
        assert!(l.busy(), "return submits as the Enter key does");
        assert!(l.is_empty(), "the buffer is taken on submit");
        assert_eq!(wait(&mut l, 0.0), Some("tok".to_string()));
    }

    #[test]
    fn return_on_an_empty_field_does_not_submit_and_hide_lowers_the_keyboard() {
        let ui = super::super::tests::ui();
        let mut l = touch_login(Reply::Ok("tok".into()));
        let p = field_point(&l, &ui);
        tap(&mut l, &ui, p, 0.0);
        l.take_keyboard_request();
        l.key(&Out::Enter, 0.0);
        assert!(!l.busy());
        assert_eq!(l.key(&Out::Hide, 0.0), Act::None);
        assert!(!l.keyboard_up());
        assert_eq!(l.take_keyboard_request(), Some(false));
    }

    #[test]
    fn the_return_arrow_still_submits_with_the_keyboard_up() {
        let ui = super::super::tests::ui();
        let mut l = touch_login(Reply::Ok("tok".into()));
        let p = field_point(&l, &ui);
        tap(&mut l, &ui, p, 0.0);
        l.key(&Out::Text("pw".into()), 0.0);
        let (x, y, w, h) = l.arrow(&ui);
        tap(&mut l, &ui, (x + w / 2.0, y + h / 2.0), 1.0);
        assert!(l.busy());
        assert_eq!(wait(&mut l, 1.0), Some("tok".to_string()));
    }

    #[test]
    fn the_group_moves_above_the_keyboard_and_back() {
        let ui = super::super::tests::ui();
        let mut l = touch_login(Reply::Unavailable);
        let before = l.field(&ui);
        let kb_top = l.h as f32 * 0.7;
        assert!(before.1 + before.3 > kb_top, "the field starts under this keyboard");
        l.set_keyboard_top(&ui, Some(kb_top), 100.0);
        assert!(l.wake(100.0).is_some(), "the move needs frames");
        assert!(l.tick(100.0 + LIFT_MS / 2.0));
        let mid = l.field(&ui).1;
        assert!(mid < before.1, "on its way up");
        assert!(l.tick(100.0 + LIFT_MS));
        assert!(!l.tick(100.0 + LIFT_MS + 10.0), "settled");
        assert_eq!(l.wake(500.0), None);
        let (fx, fy, fw, fh) = l.field(&ui);
        assert!(fy + fh + ui.pt(60.0) <= kb_top + 1.0, "field and the lines under it clear the keyboard");
        let (_, ay, _, ad) = l.avatar_rect(&ui);
        assert!(ay + ad < fy, "the picture moved with the field");
        assert!(ay > ui.pt(super::super::scene::STATUS_PT), "and stays below the status bar");
        // a touch lands on the moved field, not on the old place
        assert_eq!(l.hit(&ui, fx + fw * 0.25, fy + fh / 2.0), Some(Hit::Field));
        assert_eq!(l.hit(&ui, fx + fw * 0.25, before.1 + before.3 / 2.0), None);
        l.set_keyboard_top(&ui, None, 1000.0);
        l.tick(1000.0 + LIFT_MS);
        assert_eq!(l.field(&ui), before, "back in place once the keyboard is down");
    }

    #[test]
    fn the_move_never_pushes_the_picture_into_the_status_bar() {
        let ui = super::super::tests::ui();
        let mut l = touch_login(Reply::Unavailable);
        // an absurdly tall keyboard
        l.set_keyboard_top(&ui, Some(100.0), 0.0);
        l.tick(LIFT_MS);
        let (_, ay, _, _) = l.avatar_rect(&ui);
        assert!(ay >= ui.pt(super::super::scene::STATUS_PT), "top edge {ay}");
    }

    #[test]
    fn a_short_keyboard_leaves_the_group_where_it_is() {
        let ui = super::super::tests::ui();
        let mut l = touch_login(Reply::Unavailable);
        let before = l.field(&ui);
        l.set_keyboard_top(&ui, Some(l.h as f32 - 10.0), 0.0);
        l.tick(LIFT_MS);
        assert_eq!(l.field(&ui), before);
    }

    #[test]
    fn opening_a_restart_sheet_lowers_the_keyboard() {
        let ui = super::super::tests::ui();
        let mut l = touch_login(Reply::Unavailable);
        let p = field_point(&l, &ui);
        tap(&mut l, &ui, p, 0.0);
        l.take_keyboard_request();
        l.power_open = true;
        let (x, y, w, h) = l.bottom(&ui)[1].2;
        tap(&mut l, &ui, (x + w / 2.0, y + h / 2.0), 1.0);
        assert_eq!(l.sheet, Some(Sheet::Restart));
        assert!(!l.keyboard_up());
        assert_eq!(l.take_keyboard_request(), Some(false));
    }
}
