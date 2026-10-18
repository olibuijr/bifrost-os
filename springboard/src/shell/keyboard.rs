//! Client of the on-screen keyboard, which is a system app (`is.olibuijr.keyboard`, `provides = ["keyboard"]`).
//!
//! No keyboard code lives in the shell or in any app. Every view that wants typing (the Terminal, Browser, Settings
//! search, a text field in an app) owns a `Keyboard` *client*: it knows whether the keyboard is shown, animates the
//! slide, forwards touches and blits the keyboard app's frame. The one keyboard process is shared by all clients
//! through a per-thread service: the client that was shown last owns it. What the user types comes back as `Out`
//! from `tick`, exactly as before the move; the layout, predictions and emoji are the app's (`apps/keyboard`).
//! Wire protocol: `ToApp::KeyboardConfig` / `ToApp::Touch` in, `ToShell::KeyboardOut` back (docs/keyboard-system-app.md).

use super::apphost::{self, AppHost, Pumped};
use super::kb_capslock::KbPrefs;
use super::kb_clicks::Click;
use super::ui::Ui;
use crate::gfx::{pack, Surface, View};
use bifrost_proto::{kbflags, Configure, KeyboardConfig, KeyboardLayout, KeyboardOut, ReturnKey, ToShell, TouchKind};
use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

pub const BG: u32 = pack(43, 43, 45, 255);

/// How long the slide in or out takes.
const SLIDE_MS: f64 = 250.0;
/// Pixel rows of the keyboard app's buffer relative to its width: room for the predictive bar and terminal row.
const BUFFER_ASPECT: f32 = 0.7;
/// Wait this long before restarting a keyboard process that exited.
const RESPAWN: Duration = Duration::from_secs(2);

/// Whether prose keyboards (`new_prose`: notes, reminders) start with smart typing on. Search, password and terminal
/// keyboards never do. The shell binary sets it.
static PREDICTIVE_DEFAULT: AtomicBool = AtomicBool::new(false);

pub fn set_predictive_default(on: bool) {
    PREDICTIVE_DEFAULT.store(on, Ordering::Relaxed);
}

/// What typing produces, for the focused field.
#[derive(Clone, Debug, PartialEq)]
pub enum Out {
    Text(String),
    Backspace,
    Enter,
    Hide,
    /// terminal keys: "Escape", "Tab", "Paste", "Up", "Down", "Left", "Right"
    Named(&'static str),
}

/// Composition the keyboard app reports besides typed text (candidate text of an IME): the shell forwards it to the
/// focused field as `ToApp::SetMarkedText` / `UnmarkText`.
#[derive(Clone, Debug, PartialEq)]
pub enum Composition {
    Preedit { text: String, sel_start: u32, sel_len: u32 },
    Unmark { commit: bool },
}

/// Docked, split or floating.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Mode {
    #[default]
    Docked,
    /// two thumb clusters with a gap in the middle
    Split,
    /// narrow, slid sideways by dragging the dock handle
    Floating,
}

fn layout_of(mode: Mode) -> KeyboardLayout {
    match mode {
        Mode::Docked => KeyboardLayout::Docked,
        Mode::Split => KeyboardLayout::Split,
        Mode::Floating => KeyboardLayout::Floating,
    }
}

fn static_name(name: &str) -> Option<&'static str> {
    ["Escape", "Tab", "Paste", "Up", "Down", "Left", "Right", "More", "Ctrl"].into_iter().find(|n| *n == name)
}

// ---- app backend -------------------------------------------------------------------------------------------------
// An app process (sandboxed, no keyboard of its own) uses the same `Keyboard` type: `set_shown` becomes a request to the
// shell (`ToShell::WantsKeyboard`, sent by the SDK runtime), the shell draws the keyboard over the app and handles its
// touches, typed text arrives as ordinary key input, and the height is the `safe_bottom` of the app's configuration.

thread_local! {
    static APP_MODE: Cell<bool> = const { Cell::new(false) };
    static APP_INSET: Cell<f32> = const { Cell::new(0.0) };
    static APP_WANT: Cell<Option<bool>> = const { Cell::new(None) };
}

/// Called by the SDK runtime in an app process: keyboards become requests to the shell.
pub fn use_app_backend() {
    APP_MODE.with(|m| m.set(true));
}

/// The keyboard height the shell reserved for the app (`Configure::safe_bottom`), px.
pub fn set_app_inset(px: f32) {
    APP_INSET.with(|i| i.set(px.max(0.0)));
}

/// The app's latest wish to show (true) or hide (false) the keyboard, once.
pub fn take_app_request() -> Option<bool> {
    APP_WANT.with(|w| w.take())
}

fn app_mode() -> bool {
    APP_MODE.with(|m| m.get())
}

/// The one keyboard process and what it has said, shared by every client on this thread.
struct Service {
    host: Option<AppHost>,
    /// the client that owns the keyboard now (the one shown last)
    owner: u64,
    /// geometry and configuration the process was last given
    sent: Option<(u32, u32, f32, KeyboardConfig)>,
    /// typed output not yet taken by the owner
    outs: VecDeque<Out>,
    clicks: Vec<Click>,
    /// composition reported by the keyboard, in order, not yet taken by the shell
    compositions: VecDeque<Composition>,
    /// the keyboard's own height in px, as it last reported
    height: f32,
    /// a frame arrived that no client has drawn yet
    dirty: bool,
    died_at: Option<Instant>,
    /// the provider, looked up at most every two seconds
    provider: Option<(Instant, Option<(String, std::path::PathBuf)>)>,
    /// tests: run this executable as the provider instead of looking one up
    #[cfg(any(test, feature = "testkit"))]
    provider_override: Option<(String, std::path::PathBuf)>,
    /// tests: there is no keyboard process; a stand-in types what `centre` + `up` ask for (on by default in tests)
    #[cfg(any(test, feature = "testkit"))]
    fake: bool,
}

thread_local! {
    static SERVICE: RefCell<Service> = const { RefCell::new(Service {
        host: None, owner: 0, sent: None, outs: VecDeque::new(), clicks: Vec::new(), compositions: VecDeque::new(), height: 0.0, dirty: false, died_at: None, provider: None,
        #[cfg(any(test, feature = "testkit"))]
        provider_override: None,
        #[cfg(any(test, feature = "testkit"))] fake: true,
    }) };
    static NEXT_ID: Cell<u64> = const { Cell::new(1) };
}

/// The installed system package that provides the keyboard and has something to run: (id, executable).
fn find_provider() -> Option<(String, std::path::PathBuf)> {
    super::store::install::installed_in(&apphost::root())
        .into_iter()
        .find(|a| a.provides.iter().any(|p| p == "keyboard") && a.exec_path.is_file())
        .map(|a| (a.id, a.exec_path))
}

impl Service {
    fn running(&self) -> bool {
        #[cfg(any(test, feature = "testkit"))]
        if self.fake {
            return true;
        }
        self.host.as_ref().is_some_and(|h| h.running())
    }

    fn provider(&mut self) -> Option<(String, std::path::PathBuf)> {
        #[cfg(any(test, feature = "testkit"))]
        if self.provider_override.is_some() {
            return self.provider_override.clone();
        }
        if self.provider.as_ref().map_or(true, |(at, _)| at.elapsed() > Duration::from_secs(2)) {
            self.provider = Some((Instant::now(), find_provider()));
        }
        self.provider.as_ref().and_then(|(_, p)| p.clone())
    }

    /// Starts (or restarts, when the package was updated or it died) the keyboard process.
    fn ensure(&mut self, w: u32, h: u32, scale: f32) {
        #[cfg(any(test, feature = "testkit"))]
        if self.fake {
            return;
        }
        let Some((id, exec)) = self.provider() else { return };
        if let Some(host) = self.host.as_mut().filter(|h| h.running()) {
            if !host.is_outdated(&exec) {
                return;
            }
            host.kill();
            self.host = None;
            self.sent = None;
        }
        if self.died_at.is_some_and(|t| t.elapsed() < RESPAWN) {
            return;
        }
        let data = apphost::root().join("appdata").join(&id);
        let cfg = Configure { w, h, scale, safe_top: 0.0, safe_bottom: 0.0, dark: true, text_scale: 1.0, reduce_motion: false };
        match AppHost::spawn(&id, &exec, &data, &apphost::root(), cfg) {
            Ok(host) => {
                self.host = Some(host);
                self.sent = None;
            }
            Err(e) => {
                eprintln!("springboard: cannot start {id}: {e}");
                self.died_at = Some(Instant::now());
            }
        }
    }

    /// Sends the geometry and field configuration when they changed.
    fn configure(&mut self, w: u32, h: u32, scale: f32, config: KeyboardConfig) {
        if self.sent == Some((w, h, scale, config)) {
            return;
        }
        let geometry_changed = self.sent.map_or(true, |(sw, sh, ss, _)| (sw, sh, ss) != (w, h, scale));
        self.sent = Some((w, h, scale, config));
        if let Some(host) = self.host.as_mut() {
            if geometry_changed {
                host.configure(Configure { w, h, scale, safe_top: 0.0, safe_bottom: 0.0, dark: true, text_scale: 1.0, reduce_motion: false });
            }
            host.send(&bifrost_proto::ToApp::KeyboardConfig(config));
        }
    }

    fn pump(&mut self) {
        #[cfg(any(test, feature = "testkit"))]
        if self.fake {
            return;
        }
        let Some(host) = self.host.as_mut() else { return };
        let Pumped { frame, requests, died } = host.pump();
        if frame {
            self.dirty = true;
        }
        for r in requests {
            let ToShell::KeyboardOut(out) = r else { continue };
            match out {
                KeyboardOut::Text(t) => self.outs.push_back(Out::Text(t)),
                KeyboardOut::Backspace => self.outs.push_back(Out::Backspace),
                KeyboardOut::Enter => self.outs.push_back(Out::Enter),
                KeyboardOut::Hide => self.outs.push_back(Out::Hide),
                KeyboardOut::Named(n) => {
                    if let Some(n) = static_name(&n) {
                        self.outs.push_back(Out::Named(n));
                    }
                }
                KeyboardOut::Height(h) => {
                    self.height = h as f32;
                    self.dirty = true;
                }
                KeyboardOut::Language(i) => {
                    if let Some(l) = super::kb_layouts::Layout::ALL.get(i as usize) {
                        super::kb_layouts::set_active(*l);
                    }
                }
                KeyboardOut::Preedit { text, sel_start, sel_len } => {
                    // a composition no field consumes must not pile up
                    if self.compositions.len() < 64 {
                        self.compositions.push_back(Composition::Preedit { text, sel_start, sel_len });
                    }
                }
                KeyboardOut::Unmark { commit } => {
                    if self.compositions.len() < 64 {
                        self.compositions.push_back(Composition::Unmark { commit });
                    }
                }
                KeyboardOut::Click(k) => self.clicks.push(match k {
                    1 => Click::Delete,
                    2 => Click::Modifier,
                    _ => Click::Key,
                }),
            }
        }
        if died || !host.running() {
            self.host = None;
            self.sent = None;
            self.died_at = Some(Instant::now());
        }
    }
}

/// Called once per shell tick: reads what the keyboard process sent.
pub fn pump() {
    SERVICE.with(|s| s.borrow_mut().pump());
}

/// Composition the keyboard app sent since the last call (candidate text, commit, cancel).
pub fn take_composition() -> Vec<Composition> {
    SERVICE.with(|s| {
        let mut s = s.borrow_mut();
        s.pump();
        s.compositions.drain(..).collect()
    })
}

/// The keyboard process's socket, for the shell's poll set (its frames and keys wake the loop).
pub fn poll_fd() -> Option<std::ffi::c_int> {
    SERVICE.with(|s| s.borrow().host.as_ref().filter(|h| h.running()).map(|h| h.fd() as std::ffi::c_int))
}

/// Tests: stand in for the keyboard process. `height` is its reported height in px.
#[cfg(any(test, feature = "testkit"))]
pub fn fake_process(height: f32) {
    SERVICE.with(|s| {
        let mut s = s.borrow_mut();
        s.fake = true;
        s.height = height;
    });
}

/// Tests: drop the stand-in and run this executable as the keyboard process (end-to-end tests of the real app).
#[cfg(any(test, feature = "testkit"))]
pub fn use_process_for_test(id: &str, exec: std::path::PathBuf) {
    SERVICE.with(|s| {
        let mut s = s.borrow_mut();
        s.fake = false;
        s.provider_override = Some((id.to_string(), exec));
    });
}

/// Tests: what the user typed on the keyboard.
#[cfg(any(test, feature = "testkit"))]
pub fn fake_typed(out: Out) {
    SERVICE.with(|s| s.borrow_mut().outs.push_back(out));
}

/// Tests: a key click the keyboard reported.
#[cfg(any(test, feature = "testkit"))]
pub fn fake_click(c: Click) {
    SERVICE.with(|s| s.borrow_mut().clicks.push(c));
}

#[cfg(any(test, feature = "testkit"))]
thread_local! {
    /// Labels handed out by `Keyboard::centre`, so the fake process can tell which key a lifted finger was on.
    static FAKE_KEYS: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

static DISABLED: AtomicBool = AtomicBool::new(false);

/// Turns the on-screen keyboard off for the whole session: no view shows one, none reserves room for one, and the
/// keyboard process is never started. A desktop has a hardware keyboard.
pub fn set_disabled(off: bool) {
    DISABLED.store(off, Ordering::Relaxed);
}

pub fn disabled() -> bool {
    DISABLED.load(Ordering::Relaxed)
}

/// A view's handle on the on-screen keyboard.
pub struct Keyboard {
    id: u64,
    pub w: f32,
    s: f32,
    accessory: bool,
    predictive: bool,
    auto_caps: bool,
    kbp: KbPrefs,
    mode: Mode,
    return_key: ReturnKey,
    shown: bool,
    /// (start, showing) of the slide in progress
    slide: Option<(f64, bool)>,
    /// a finger is down on the keyboard
    touching: bool,
}

impl Keyboard {
    /// `w` is the screen width in px, `s` px per point; `accessory` adds the terminal row. Starts shown, without
    /// animation.
    pub fn new(w: f32, s: f32, accessory: bool) -> Self {
        Self::make(w, s, accessory, true)
    }

    /// The shell's keyboard: the accessory row, starting off screen until a text field raises it (`set_shown`).
    pub fn new_hidden_with_accessory(w: f32, s: f32) -> Self {
        Self::make(w, s, true, false)
    }

    fn make(w: f32, s: f32, accessory: bool, shown: bool) -> Self {
        let id = NEXT_ID.with(|n| {
            let id = n.get();
            n.set(id + 1);
            id
        });
        let mut k = Keyboard {
            id,
            w,
            s,
            accessory,
            predictive: false,
            auto_caps: false,
            kbp: KbPrefs::default(),
            mode: Mode::Docked,
            return_key: ReturnKey::Return,
            shown,
            slide: None,
            touching: false,
        };
        k.claim(false);
        k
    }

    /// A keyboard without the accessory row that starts off screen, to be slid in with `set_shown` when a field is
    /// tapped.
    pub fn new_hidden(w: f32, s: f32) -> Self {
        Self::make(w, s, false, false)
    }

    /// Like `new_hidden`, for free text: smart typing follows the default set by `set_predictive_default`.
    pub fn new_prose(w: f32, s: f32) -> Self {
        let mut k = Keyboard::new_hidden(w, s);
        k.predictive = PREDICTIVE_DEFAULT.load(Ordering::Relaxed);
        k
    }

    /// Starts hidden and slides in: for a keyboard that appears with a sheet.
    pub fn appear(&mut self, t: f64) {
        self.shown = false;
        self.set_shown(true, t);
    }

    fn config(&self) -> KeyboardConfig {
        let mut flags = 0;
        for (on, bit) in [
            (self.predictive, kbflags::PREDICTIVE),
            (self.auto_caps, kbflags::AUTO_CAPS),
            (self.accessory, kbflags::ACCESSORY),
            (self.shown, kbflags::SHOWN),
            (self.kbp.enable_caps_lock, kbflags::CAPS_LOCK),
            (self.kbp.period_shortcut, kbflags::PERIOD_SHORTCUT),
        ] {
            if on {
                flags |= bit;
            }
        }
        let (languages, language) = super::kb_layouts::state();
        KeyboardConfig { layout: layout_of(self.mode), return_key: self.return_key, flags, languages, language }
    }

    fn buffer(&self) -> (u32, u32) {
        (self.w.max(1.0) as u32, (self.w * BUFFER_ASPECT).max(1.0) as u32)
    }

    /// Makes this client the keyboard's owner and tells the process what it should look like.
    fn claim(&mut self, take_over: bool) {
        if app_mode() {
            return;
        }
        if disabled() {
            return;
        }
        let (w, h) = self.buffer();
        let (s, config) = (self.s, self.config());
        let id = self.id;
        SERVICE.with(|svc| {
            let mut svc = svc.borrow_mut();
            // creating a client does not take the keyboard from the one in use; showing it does
            if self.shown && (take_over || svc.owner == 0) {
                svc.owner = id;
            }
            // the process starts when the keyboard in use is shown, also one created shown (the shell's, which the lock
            // screen raises at once); a hidden view never starts it, and a desktop never shows one
            if self.shown && svc.owner == id {
                svc.ensure(w, h, s);
            }
            if svc.owner == id {
                svc.configure(w, h, s, config);
            }
        });
    }

    fn owns(&self) -> bool {
        SERVICE.with(|s| s.borrow().owner == self.id)
    }

    /// The screen width changed (rotation, resizable window). The keys are laid out again only when the width moved.
    pub fn resize(&mut self, w: f32) {
        if w != self.w {
            self.w = w;
            self.claim(false);
        }
    }

    /// The width the keys are laid out for.
    pub fn width(&self) -> f32 {
        self.w
    }

    /// Display Zoom: lays the keys out at `s` px per point.
    pub fn set_scale(&mut self, s: f32) {
        if s != self.s {
            self.s = s;
            self.claim(false);
        }
    }

    /// Docks, splits or floats the keyboard.
    pub fn set_mode(&mut self, mode: Mode) {
        if mode != self.mode {
            self.mode = mode;
            self.claim(false);
        }
    }

    pub fn mode(&self) -> Mode {
        self.mode
    }

    /// Turns the prediction bar, autocorrect, double-space period and word learning on or off; the bar changes the
    /// keyboard's height.
    pub fn set_predictive(&mut self, on: bool) {
        if on != self.predictive {
            self.predictive = on;
            self.claim(false);
        }
    }

    pub fn predictive(&self) -> bool {
        self.predictive
    }

    /// The focused field's return key label (`NativeReturnKeyType` through `ReturnKeyType::basic`).
    pub fn set_return_key(&mut self, key: ReturnKey) {
        if key != self.return_key {
            self.return_key = key;
            self.claim(false);
        }
    }

    pub fn return_key(&self) -> ReturnKey {
        self.return_key
    }

    /// Auto-Capitalisation (Settings > General > Keyboard).
    pub fn set_auto_caps(&mut self, on: bool) {
        if on != self.auto_caps {
            self.auto_caps = on;
            self.claim(false);
        }
    }

    /// Settings > General > Keyboard: Enable Caps Lock and the "." Shortcut.
    pub fn set_kb_prefs(&mut self, p: KbPrefs) {
        if p != self.kbp {
            self.kbp = p;
            self.claim(false);
        }
    }

    /// Text size changed: the process re-renders its labels when it is configured again.
    pub fn clear_labels(&mut self) {}

    /// Slides the keyboard in or out over a quarter second.
    pub fn set_shown(&mut self, shown: bool, t: f64) {
        if disabled() {
            self.shown = false;
            return;
        }
        if shown != self.shown {
            self.shown = shown;
            self.touching = false;
            if app_mode() {
                // the shell animates and draws it
                self.slide = None;
                APP_WANT.with(|w| w.set(Some(shown)));
                return;
            }
            self.slide = Some((t, shown));
            self.claim(true);
        }
    }

    /// Where the keyboard rests (ignoring the slide in progress).
    pub fn shown(&self) -> bool {
        self.shown && !disabled()
    }

    /// The keyboard's height in px: what its process reported, or an estimate until it has. Zero when no keyboard
    /// package is installed, so nothing reserves room for a keyboard that cannot appear.
    pub fn h(&self) -> f32 {
        if disabled() {
            return 0.0;
        }
        if app_mode() {
            return APP_INSET.with(|i| i.get());
        }
        SERVICE.with(|s| {
            let mut s = s.borrow_mut();
            if !s.running() && s.provider().is_none() {
                return 0.0;
            }
            if s.height > 0.0 {
                return s.height;
            }
            let kx = (self.w / self.s / 768.0).max(1.0);
            let ky = kx.min(4.0 / 3.0);
            (258.0 + if self.predictive { 55.0 } else { 0.0 } + if self.accessory { 50.0 } else { 0.0 }) * self.s * ky
        })
    }

    pub fn animating(&self) -> bool {
        if app_mode() || disabled() {
            return false;
        }
        self.slide.is_some() || (self.shown && self.owns() && SERVICE.with(|s| s.borrow().dirty))
    }

    /// How far below its resting place the keyboard is drawn at `t`: 0 when shown, its height when hidden, in
    /// between while sliding (ease-out).
    pub fn offset(&self, t: f64) -> f32 {
        let h = self.h();
        match self.slide {
            Some((start, showing)) => {
                let p = ((t - start) / SLIDE_MS).clamp(0.0, 1.0) as f32;
                let e = 1.0 - (1.0 - p).powi(3);
                if showing {
                    h * (1.0 - e)
                } else {
                    h * e
                }
            }
            None if self.shown => 0.0,
            None => h,
        }
    }

    fn touch(&mut self, kind: TouchKind, x: f32, y: f32, t: f64) {
        if app_mode() || !self.owns() {
            return;
        }
        SERVICE.with(|s| {
            if let Some(host) = s.borrow_mut().host.as_mut() {
                host.touch(kind, 0, x, y, t as u32);
            }
        });
    }

    /// A finger went down at (x, y) relative to the keyboard's top-left.
    pub fn down(&mut self, x: f32, y: f32, t: f64) {
        self.touching = true;
        self.touch(TouchKind::Down, x, y, t);
    }

    pub fn moved(&mut self, x: f32, y: f32) {
        self.touch(TouchKind::Move, x, y, 0.0);
    }

    /// The finger lifted. What it typed arrives with the next `tick` (the keyboard is another process).
    pub fn up(&mut self, x: f32, y: f32, t: f64) -> Option<Out> {
        self.touching = false;
        #[cfg(any(test, feature = "testkit"))]
        if SERVICE.with(|s| s.borrow().fake) {
            return self.fake_up(x);
        }
        self.touch(TouchKind::Up, x, y, t);
        None
    }

    /// Tests: the position of the key labelled `name` (the fake process has no layout, so each label gets a slot).
    #[cfg(any(test, feature = "testkit"))]
    pub fn centre(&self, name: &str) -> Option<(f32, f32)> {
        let idx = FAKE_KEYS.with(|k| {
            let mut k = k.borrow_mut();
            k.iter().position(|n| n == name).unwrap_or_else(|| {
                k.push(name.to_string());
                k.len() - 1
            })
        });
        Some((4.0 + 8.0 * idx as f32, self.h() / 2.0))
    }

    /// Tests: lifting a finger over a key types what the real keyboard would.
    #[cfg(any(test, feature = "testkit"))]
    fn fake_up(&mut self, x: f32) -> Option<Out> {
        let idx = ((x - 4.0) / 8.0).round().max(0.0) as usize;
        let name = FAKE_KEYS.with(|k| k.borrow().get(idx).cloned())?;
        match name.as_str() {
            "return" => Some(Out::Enter),
            "hide" => Some(Out::Hide),
            "⌫" | "delete" => Some(Out::Backspace),
            n if n.chars().count() == 1 || n == "space" => Some(Out::Text(if n == "space" { " ".into() } else { n.into() })),
            n => static_name(match n.to_lowercase().as_str() {
                "esc" | "escape" => "Escape",
                "tab" => "Tab",
                "paste" => "Paste",
                "up" => "Up",
                "down" => "Down",
                "left" => "Left",
                "right" => "Right",
                other => other,
            })
            .map(Out::Named),
        }
    }

    /// The keys typed since the last call, when this client owns the keyboard.
    pub fn tick(&mut self, t: f64) -> Vec<Out> {
        if app_mode() {
            return Vec::new();
        }
        if self.slide.is_some_and(|(start, _)| t - start >= SLIDE_MS) {
            self.slide = None;
        }
        SERVICE.with(|s| {
            let mut s = s.borrow_mut();
            s.pump();
            if s.owner == self.id && self.shown {
                s.outs.drain(..).collect()
            } else {
                Vec::new()
            }
        })
    }

    /// Key click sounds queued since the last call.
    pub fn take_clicks(&mut self) -> Vec<Click> {
        if !self.owns() {
            return Vec::new();
        }
        SERVICE.with(|s| std::mem::take(&mut s.borrow_mut().clicks))
    }

    /// Draws the keyboard's frame with its top at `oy` (plus the slide offset).
    pub fn draw(&mut self, s: &mut Surface, _ui: &Ui, oy: f32, t: f64) {
        if app_mode() {
            return;
        }
        let y = (oy + self.offset(t)) as i32;
        let h = self.h().ceil() as usize;
        SERVICE.with(|svc| {
            let mut svc = svc.borrow_mut();
            if svc.owner == self.id {
                svc.dirty = false;
            }
            let frame = svc.host.as_ref().and_then(|h| h.frame());
            match frame {
                Some((px, fw, fh)) if svc.owner == self.id && fw > 0 && fh > 0 => {
                    let rows = h.min(fh);
                    s.copy(View { w: fw, h: rows, px: &px[..fw * rows] }, 0, y);
                    if rows < h {
                        s.fill_rect(0, (y + rows as i32).max(0) as usize, self.w as usize, h - rows, BG);
                    }
                }
                _ => s.fill_rect(0, y.max(0) as usize, self.w as usize, h, BG),
            }
        });
    }
}

impl Drop for Keyboard {
    fn drop(&mut self) {
        let id = self.id;
        SERVICE.with(|s| {
            if let Ok(mut s) = s.try_borrow_mut() {
                if s.owner == id {
                    s.owner = 0;
                }
            }
        });
    }
}

#[cfg(test)]
mod resize_tests {
    use super::*;

    #[test]
    fn resize_takes_the_new_width_and_keeps_the_same_one() {
        let mut k = Keyboard::new_hidden(1000.0, 1.0);
        k.resize(600.0);
        assert_eq!(k.width(), 600.0);
        k.resize(600.0);
        assert_eq!(k.width(), 600.0);
    }
}

#[cfg(test)]
mod disabled_tests {
    use super::*;

    #[test]
    fn a_disabled_keyboard_never_shows_or_reserves_room() {
        set_disabled(true);
        let mut k = Keyboard::new_hidden(1000.0, 1.0);
        k.set_shown(true, 0.0);
        let (shown, h, spawned) = (k.shown(), k.h(), poll_fd().is_some());
        set_disabled(false);
        assert!(!shown && h == 0.0 && !spawned);
    }
}
