//! Input routing. Backends turn device events into these calls; tests call them directly.
//!
//! Also here: the shell's pointer, trackpad, keyboard and cursor preferences (read from `settings.ini`), their
//! mapping to libinput and xkb settings, pointer gestures, pointer constraints (locked pointers) and relative
//! pointer motion. The parsing and mapping functions are pure and host-tested; see `docs/graphics/g2-wm-input.md`.

use std::path::{Path, PathBuf};

use smithay::{
    backend::input::{
        Axis, AxisSource, ButtonState, Event, GestureBeginEvent, GestureEndEvent, GesturePinchUpdateEvent,
        GestureSwipeUpdateEvent, InputBackend, InputEvent, KeyState, KeyboardKeyEvent, PointerAxisEvent, PointerButtonEvent,
        PointerMotionEvent,
    },
    delegate_pointer_constraints, delegate_pointer_gestures, delegate_relative_pointer,
    input::{
        keyboard::{Error as KeyboardError, FilterResult, Keycode, KeyboardHandle, XkbConfig},
        pointer::{
            AxisFrame, ButtonEvent, GestureHoldBeginEvent, GestureHoldEndEvent, GesturePinchBeginEvent, GesturePinchEndEvent,
            GesturePinchUpdateEvent as PinchUpdate, GestureSwipeBeginEvent, GestureSwipeEndEvent,
            GestureSwipeUpdateEvent as SwipeUpdate, MotionEvent, PointerHandle, RelativeMotionEvent,
        },
        Seat,
    },
    reexports::wayland_server::{protocol::wl_surface::WlSurface, DisplayHandle},
    utils::{Logical, Point, Serial, SERIAL_COUNTER},
    wayland::{
        pointer_constraints::{
            with_pointer_constraint, PointerConstraint, PointerConstraintsHandler, PointerConstraintsState,
        },
        pointer_gestures::PointerGesturesState,
        relative_pointer::RelativePointerManagerState,
    },
};

use crate::state::WmState;

/// Where the shell keeps `settings.ini`: the same rule as the shell's `data_dir` (the root account uses
/// `/var/lib/springboard`, a user the XDG data directory). `BIFROST_SETTINGS` overrides the file.
pub fn prefs_path() -> PathBuf {
    if let Some(p) = std::env::var_os("BIFROST_SETTINGS") {
        return PathBuf::from(p);
    }
    let root = std::env::var("USER").map(|u| u == "root").unwrap_or(false);
    let xdg = std::env::var("XDG_DATA_HOME").ok().filter(|s| !s.is_empty());
    let home = std::env::var("HOME").ok().filter(|s| !s.is_empty());
    let dir = match (root, xdg, home) {
        (false, Some(x), _) => format!("{}/springboard", x.trim_end_matches('/')),
        (false, None, Some(h)) => format!("{}/.local/share/springboard", h.trim_end_matches('/')),
        _ => "/var/lib/springboard".to_string(),
    };
    PathBuf::from(dir).join("settings.ini")
}

// ---------------------------------------------------------------------------------------------------------------
// Preferences
// ---------------------------------------------------------------------------------------------------------------

/// The pointer acceleration profile.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Accel {
    Adaptive,
    Flat,
}

/// The input and cursor preferences the window server reads from `settings.ini`. Missing or invalid values fall
/// back to the defaults in [`InputPrefs::default`].
///
/// Keys: `pointer_accel` (`adaptive` | `flat`), `pointer_speed` (-1.0 to 1.0), `natural_scroll`,
/// `tap_to_click`, `two_finger_secondary_click`, `key_repeat_delay_ms` (100 to 2000), `key_repeat_rate`
/// (1 to 100 per second), `keyboard_layout` (an xkb layout such as `us` or `is`), `cursor_size` (16 to 96 px) and
/// `cursor_theme` (a directory name under `icons/`).
#[derive(Clone, Debug, PartialEq)]
pub struct InputPrefs {
    pub accel: Accel,
    pub speed: f64,
    pub natural_scroll: bool,
    pub tap_to_click: bool,
    pub two_finger_secondary: bool,
    pub repeat_delay_ms: i32,
    pub repeat_rate: i32,
    pub layout: String,
    pub cursor_size: u32,
    pub cursor_theme: String,
}

impl Default for InputPrefs {
    fn default() -> Self {
        InputPrefs {
            accel: Accel::Adaptive,
            speed: 0.0,
            natural_scroll: false,
            tap_to_click: true,
            two_finger_secondary: true,
            repeat_delay_ms: 300,
            repeat_rate: 30,
            layout: "us".into(),
            cursor_size: 24,
            cursor_theme: "default".into(),
        }
    }
}

/// `1`, `on`, `true`, `yes` are on; `0`, `off`, `false`, `no` are off; anything else is unknown.
pub fn parse_flag(value: &str) -> Option<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "on" | "true" | "yes" => Some(true),
        "0" | "off" | "false" | "no" => Some(false),
        _ => None,
    }
}

/// A keyboard layout name: two or three lowercase ASCII letters (`us`, `is`, `gb`, `de`).
pub fn valid_layout(value: &str) -> bool {
    (2..=3).contains(&value.len()) && value.bytes().all(|b| b.is_ascii_lowercase())
}

/// A cursor theme directory name: 1 to 64 characters, no path separators or spaces.
pub fn valid_theme(value: &str) -> bool {
    (1..=64).contains(&value.len()) && !value.contains(['/', ' ', '\t', '\\']) && value != "." && value != ".."
}

impl InputPrefs {
    /// Reads the preferences through `get` (a settings lookup). Each key that is missing or invalid keeps its default.
    pub fn from_lookup<'a>(get: impl Fn(&str) -> Option<&'a str>) -> Self {
        let d = InputPrefs::default();
        let flag = |key: &str, default: bool| get(key).and_then(parse_flag).unwrap_or(default);
        InputPrefs {
            accel: match get("pointer_accel").map(|v| v.trim().to_ascii_lowercase()) {
                Some(v) if v == "flat" => Accel::Flat,
                Some(v) if v == "adaptive" => Accel::Adaptive,
                _ => d.accel,
            },
            speed: get("pointer_speed")
                .and_then(|v| v.trim().parse::<f64>().ok())
                .filter(|v| v.is_finite())
                .map_or(d.speed, |v| v.clamp(-1.0, 1.0)),
            natural_scroll: flag("natural_scroll", d.natural_scroll),
            tap_to_click: flag("tap_to_click", d.tap_to_click),
            two_finger_secondary: flag("two_finger_secondary_click", d.two_finger_secondary),
            repeat_delay_ms: get("key_repeat_delay_ms")
                .and_then(|v| v.trim().parse::<i32>().ok())
                .map_or(d.repeat_delay_ms, |v| v.clamp(100, 2000)),
            repeat_rate: get("key_repeat_rate").and_then(|v| v.trim().parse::<i32>().ok()).map_or(d.repeat_rate, |v| v.clamp(1, 100)),
            layout: get("keyboard_layout").map(str::trim).filter(|v| valid_layout(v)).map_or(d.layout, str::to_string),
            cursor_size: get("cursor_size")
                .and_then(|v| v.trim().parse::<u32>().ok())
                .map_or(d.cursor_size, |v| v.clamp(16, 96)),
            cursor_theme: get("cursor_theme").map(str::trim).filter(|v| valid_theme(v)).map_or(d.cursor_theme, str::to_string),
        }
    }

    /// Reads `key=value` lines (the format of `settings.ini`; `#` starts a comment line). A later line wins.
    pub fn parse(text: &str) -> Self {
        let pairs: Vec<(&str, &str)> = text
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .filter_map(|l| l.split_once('='))
            .map(|(k, v)| (k.trim(), v.trim()))
            .collect();
        InputPrefs::from_lookup(|key| pairs.iter().rev().find(|(k, _)| *k == key).map(|(_, v)| *v))
    }

    /// The preferences in the file at `path`; a missing or unreadable file gives the defaults.
    pub fn load(path: &Path) -> Self {
        std::fs::read_to_string(path).map(|t| Self::parse(&t)).unwrap_or_default()
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Devices (libinput)
// ---------------------------------------------------------------------------------------------------------------

/// What a libinput device can be configured for.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DeviceCaps {
    /// it has an acceleration profile and a speed (every pointer, most touchpads)
    pub accel: bool,
    pub natural_scroll: bool,
    /// it has tap-to-click (touchpads)
    pub tap: bool,
    /// it can click by finger count (the two-finger secondary click)
    pub clickfinger: bool,
}

/// The settings to write to one device. `None` leaves the device as libinput set it.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct DevicePlan {
    /// `Some(true)` is the flat profile, `Some(false)` the adaptive one
    pub accel_flat: Option<bool>,
    pub speed: Option<f64>,
    pub natural_scroll: Option<bool>,
    pub tap: Option<bool>,
    pub clickfinger: Option<bool>,
}

/// Maps the preferences onto one device's capabilities: a setting is written only where the device supports it.
pub fn plan_device(prefs: &InputPrefs, caps: DeviceCaps) -> DevicePlan {
    DevicePlan {
        accel_flat: caps.accel.then_some(prefs.accel == Accel::Flat),
        speed: caps.accel.then_some(prefs.speed),
        natural_scroll: caps.natural_scroll.then_some(prefs.natural_scroll),
        tap: caps.tap.then_some(prefs.tap_to_click),
        clickfinger: caps.clickfinger.then_some(prefs.two_finger_secondary),
    }
}

/// Writes the preferences to a libinput device (called on `DeviceAdded`). A setting the device refuses is logged
/// and skipped; the rest still apply.
#[cfg(feature = "drm")]
pub fn configure_device(device: &mut smithay::reexports::input::Device, prefs: &InputPrefs) {
    use smithay::reexports::input::{AccelProfile, ClickMethod, DeviceConfigError};
    let caps = DeviceCaps {
        accel: device.config_accel_is_available(),
        natural_scroll: device.config_scroll_has_natural_scroll(),
        tap: device.config_tap_finger_count() > 0,
        clickfinger: device.config_click_methods().contains(&ClickMethod::Clickfinger),
    };
    let plan = plan_device(prefs, caps);
    let name = device.name().to_string();
    let report = |what: &str, r: Result<(), DeviceConfigError>| {
        if let Err(e) = r {
            eprintln!("bifrost-wm: {name}: {what}: {e:?}");
        }
    };
    if let Some(flat) = plan.accel_flat {
        report("accel profile", device.config_accel_set_profile(if flat { AccelProfile::Flat } else { AccelProfile::Adaptive }));
    }
    if let Some(speed) = plan.speed {
        report("accel speed", device.config_accel_set_speed(speed));
    }
    if let Some(on) = plan.natural_scroll {
        report("natural scroll", device.config_scroll_set_natural_scroll_enabled(on));
    }
    if let Some(on) = plan.tap {
        report("tap to click", device.config_tap_set_enabled(on));
    }
    if let Some(on) = plan.clickfinger {
        let method = if on { ClickMethod::Clickfinger } else { ClickMethod::ButtonAreas };
        report("click method", device.config_click_set_method(method));
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Keyboard and cursor
// ---------------------------------------------------------------------------------------------------------------

/// Creates the seat's keyboard with the layout and repeat rate from the preferences. The keyboard sends the repeat
/// rate to clients as `wl_keyboard.repeat_info`.
pub fn add_keyboard(seat: &mut Seat<WmState>, prefs: &InputPrefs) -> Result<KeyboardHandle<WmState>, KeyboardError> {
    match seat.add_keyboard(XkbConfig { layout: &prefs.layout, ..Default::default() }, prefs.repeat_delay_ms, prefs.repeat_rate) {
        Ok(k) => Ok(k),
        Err(e) => {
            // a layout the xkb data does not have (`valid_layout` only checks the shape) must not abort the session (WM-9)
            let fallback = InputPrefs::default().layout;
            eprintln!("bifrost-wm: keyboard layout {:?} rejected ({e:?}), using {fallback:?}", prefs.layout);
            seat.add_keyboard(XkbConfig { layout: &fallback, ..Default::default() }, prefs.repeat_delay_ms, prefs.repeat_rate)
        }
    }
}

/// The xcursor search directories for a theme: `~/.icons`, then `icons/` under each XDG data directory.
/// `data_dirs` is the colon-separated `XDG_DATA_DIRS` (empty gives the standard `/usr/local/share:/usr/share`).
pub fn xcursor_dirs(home: Option<&str>, data_dirs: &str, theme: &str) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(h) = home.filter(|h| !h.is_empty()) {
        dirs.push(PathBuf::from(h).join(".icons").join(theme).join("cursors"));
    }
    let data = if data_dirs.trim().is_empty() { "/usr/local/share:/usr/share" } else { data_dirs };
    for d in data.split(':').filter(|d| !d.is_empty()) {
        dirs.push(PathBuf::from(d).join("icons").join(theme).join("cursors"));
    }
    dirs
}

/// The environment that XCursor clients (and XWayland) read the theme and size from.
pub fn cursor_env(prefs: &InputPrefs) -> [(&'static str, String); 2] {
    [("XCURSOR_THEME", prefs.cursor_theme.clone()), ("XCURSOR_SIZE", prefs.cursor_size.to_string())]
}

// ---------------------------------------------------------------------------------------------------------------
// Pointer gestures: begin and end must pair per gesture kind
// ---------------------------------------------------------------------------------------------------------------

/// The three pointer gestures of `zwp_pointer_gestures_v1`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Gesture {
    Swipe = 0,
    Pinch = 1,
    Hold = 2,
}

/// Keeps the gesture protocol well formed: a `begin` is forwarded only when that kind is not already running, an
/// `update` or `end` only while it is. Each kind runs on its own, so a swipe and a pinch can be active together.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GestureGate {
    active: [bool; 3],
}

impl GestureGate {
    /// Starts `g`; false when it is already running (the begin is dropped).
    pub fn begin(&mut self, g: Gesture) -> bool {
        let slot = &mut self.active[g as usize];
        !std::mem::replace(slot, true)
    }

    /// Whether `g` is running, so an update may be forwarded.
    pub fn running(&self, g: Gesture) -> bool {
        self.active[g as usize]
    }

    /// Ends `g`; false when it was not running (the end is dropped).
    pub fn end(&mut self, g: Gesture) -> bool {
        std::mem::replace(&mut self.active[g as usize], false)
    }

    /// Every running gesture, cleared. Used when the pointer focus changes: the ends are sent to the old client.
    pub fn cancel_all(&mut self) -> Vec<Gesture> {
        [Gesture::Swipe, Gesture::Pinch, Gesture::Hold].into_iter().filter(|g| std::mem::take(&mut self.active[*g as usize])).collect()
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Pointer constraints: a locked pointer does not move; relative motion still reaches the client
// ---------------------------------------------------------------------------------------------------------------

/// Whether a constraint (`(active, locked)` from [`constraint_state`]) holds the pointer in place.
pub fn motion_locked(state: Option<(bool, bool)>) -> bool {
    matches!(state, Some((true, true)))
}

/// `(active, locked)` of the constraint the pointer has on `surface`, if any.
fn constraint_state(surface: &WlSurface, pointer: &PointerHandle<WmState>) -> Option<(bool, bool)> {
    with_pointer_constraint(surface, pointer, |c| c.map(|c| (c.is_active(), matches!(*c, PointerConstraint::Locked(_)))))
}

impl WmState {
    /// Keyboard, pointer and gesture events from a backend. Pointer motion also goes out as relative motion.
    pub fn process_input_event<I: InputBackend>(&mut self, event: InputEvent<I>) {
        // any input is activity for ext-idle-notify (the shell dims and locks on idle)
        self.dirty = true;
        let seat = self.seat.clone();
        self.idle_notifier_state.notify_activity(&seat);
        match event {
            InputEvent::Keyboard { event } => self.key(event.key_code(), event.state(), event.time_msec()),
            InputEvent::PointerMotion { event } => {
                // real pointer motion shows the pointer image again after a touch (touch.rs)
                self.last_input_touch = false;
                let delta = event.delta();
                self.pointer_relative(delta, event.delta_unaccel(), event.time());
                let pos = self.pointer_location() + delta;
                self.pointer_motion(pos, event.time_msec());
            }
            InputEvent::PointerMotionAbsolute { event } => {
                self.last_input_touch = false;
                // the device range goes through the output transform, as a touch does (touch_position)
                let norm = crate::touch::normalized::<I, _>(&event);
                if let Some(pos) = self.touch_position(norm) {
                    self.pointer_motion(pos, event.time_msec());
                }
            }
            InputEvent::PointerButton { event } => {
                self.pointer_button(event.button_code(), event.state() == ButtonState::Pressed, event.time_msec())
            }
            InputEvent::PointerAxis { event } => {
                let source = event.source();
                let amount = |axis| event.amount(axis).unwrap_or_else(|| event.amount_v120(axis).unwrap_or(0.0) * 15.0 / 120.0);
                let mut frame = AxisFrame::new(event.time_msec()).source(source);
                for axis in [Axis::Horizontal, Axis::Vertical] {
                    let v = amount(axis);
                    if v != 0.0 {
                        frame = frame.value(axis, v);
                        if let Some(d) = event.amount_v120(axis) {
                            frame = frame.v120(axis, d as i32);
                        }
                    } else if source == AxisSource::Finger && event.amount(axis) == Some(0.0) {
                        frame = frame.stop(axis);
                    }
                }
                let pointer = self.seat.get_pointer().expect("pointer");
                pointer.axis(self, frame);
                pointer.frame(self);
            }
            InputEvent::GestureSwipeBegin { event } => self.gesture_begin(Gesture::Swipe, event.fingers(), event.time_msec()),
            InputEvent::GestureSwipeUpdate { event } => {
                self.gesture_swipe_update(Point::from((event.delta_x(), event.delta_y())), event.time_msec())
            }
            InputEvent::GestureSwipeEnd { event } => self.gesture_end(Gesture::Swipe, event.cancelled(), event.time_msec()),
            InputEvent::GesturePinchBegin { event } => self.gesture_begin(Gesture::Pinch, event.fingers(), event.time_msec()),
            InputEvent::GesturePinchUpdate { event } => self.gesture_pinch_update(
                Point::from((event.delta_x(), event.delta_y())),
                event.scale(),
                event.rotation(),
                event.time_msec(),
            ),
            InputEvent::GesturePinchEnd { event } => self.gesture_end(Gesture::Pinch, event.cancelled(), event.time_msec()),
            InputEvent::GestureHoldBegin { event } => self.gesture_begin(Gesture::Hold, event.fingers(), event.time_msec()),
            InputEvent::GestureHoldEnd { event } => self.gesture_end(Gesture::Hold, event.cancelled(), event.time_msec()),
            // the touchscreen: wl_touch to the surface under each finger, a pointer for clients without it (touch.rs)
            event @ (InputEvent::TouchDown { .. }
            | InputEvent::TouchMotion { .. }
            | InputEvent::TouchUp { .. }
            | InputEvent::TouchCancel { .. }
            | InputEvent::TouchFrame { .. }) => self.process_touch_event(event),
            _ => {}
        }
    }

    pub fn pointer_location(&self) -> Point<f64, Logical> {
        self.seat.get_pointer().map(|p| p.current_location()).unwrap_or_default()
    }

    /// Sends a relative motion to the focused surface's relative pointers. `utime` is microseconds, as libinput
    /// reports it.
    pub fn pointer_relative(&mut self, delta: Point<f64, Logical>, delta_unaccel: Point<f64, Logical>, utime: u64) {
        let pointer = self.seat.get_pointer().expect("pointer");
        let focus = self.surface_under(self.pointer_location());
        pointer.relative_motion(self, focus, &RelativeMotionEvent { delta, delta_unaccel, utime });
        pointer.frame(self);
    }

    /// Moves the pointer to `pos`, kept on the output. A pointer locked by the focused surface does not move.
    pub fn pointer_motion(&mut self, pos: Point<f64, Logical>, time: u32) {
        let pointer = self.seat.get_pointer().expect("pointer");
        if pointer.current_focus().is_some_and(|s| motion_locked(constraint_state(&s, &pointer))) {
            return;
        }
        let pos = match self.output().and_then(|o| self.space.output_geometry(&o)) {
            Some(g) => (
                pos.x.clamp(g.loc.x as f64, (g.loc.x + g.size.w - 1) as f64),
                pos.y.clamp(g.loc.y as f64, (g.loc.y + g.size.h - 1) as f64),
            )
                .into(),
            None => pos,
        };
        let under = self.surface_under(pos);
        update_constraint_focus(&pointer, under.as_ref().map(|u| &u.0));
        pointer.motion(self, under, &MotionEvent { location: pos, serial: SERIAL_COUNTER.next_serial(), time });
        pointer.frame(self);
    }

    /// A press on a window raises and focuses it (on the desktop it clears focus) before the client sees it.
    pub fn pointer_button(&mut self, button: u32, pressed: bool, time: u32) {
        let pointer = self.seat.get_pointer().expect("pointer");
        let serial = SERIAL_COUNTER.next_serial();
        if pressed && !pointer.is_grabbed() {
            self.focus_at(pointer.current_location(), serial);
        }
        let state = if pressed { ButtonState::Pressed } else { ButtonState::Released };
        pointer.button(self, &ButtonEvent { button, state, serial, time });
        pointer.frame(self);
    }

    /// A press or touch at `pos` raises and focuses the window there; on the desktop it clears focus.
    pub fn focus_at(&mut self, pos: Point<f64, Logical>, serial: Serial) {
        if let Some(output) = self.output() {
            let map = smithay::desktop::layer_map_for_output(&output);
            let has_exclusive = map.layers().any(|l| {
                smithay::wayland::compositor::with_states(l.wl_surface(), |states| {
                    states.cached_state.get::<smithay::wayland::shell::wlr_layer::LayerSurfaceCachedState>().current().keyboard_interactivity == smithay::wayland::shell::wlr_layer::KeyboardInteractivity::Exclusive
                })
            });
            if has_exclusive {
                return;
            }
        }
        let hit = self.wm.at(pos.x as i32, pos.y as i32);
        if hit.is_some() || self.surface_under(pos).is_none() {
            self.focus(hit, serial);
        }
    }

    /// The pointer leaves its surface at `pos` (the end of an emulated touch pointer).
    pub fn pointer_leave(&mut self, pos: Point<f64, Logical>, time: u32) {
        let pointer = self.seat.get_pointer().expect("pointer");
        update_constraint_focus(&pointer, None);
        pointer.motion(self, None, &MotionEvent { location: pos, serial: SERIAL_COUNTER.next_serial(), time });
        pointer.frame(self);
    }

    pub fn key(&mut self, code: Keycode, state: KeyState, time: u32) {
        let keyboard = self.seat.get_keyboard().expect("keyboard");
        // compositor chords are taken before any client sees them; the action runs on the press only
        let bound = keyboard.input::<Option<crate::keybindings::Action>, _>(
            self,
            code,
            state,
            SERIAL_COUNTER.next_serial(),
            time,
            |_, mods, handle| {
                match crate::keybindings::action(handle.modified_sym().raw(), crate::keybindings::Mods::from(mods)) {
                    Some(a) => FilterResult::Intercept((state == KeyState::Pressed).then_some(a)),
                    None => FilterResult::Forward,
                }
            },
        );
        if let Some(Some(action)) = bound {
            self.run_action(action);
        }
    }

    fn gesture_begin(&mut self, g: Gesture, fingers: u32, time: u32) {
        if !self.gestures.begin(g) {
            return;
        }
        let pointer = self.seat.get_pointer().expect("pointer");
        let serial = SERIAL_COUNTER.next_serial();
        match g {
            Gesture::Swipe => pointer.gesture_swipe_begin(self, &GestureSwipeBeginEvent { serial, time, fingers }),
            Gesture::Pinch => pointer.gesture_pinch_begin(self, &GesturePinchBeginEvent { serial, time, fingers }),
            Gesture::Hold => pointer.gesture_hold_begin(self, &GestureHoldBeginEvent { serial, time, fingers }),
        }
        pointer.frame(self);
    }

    fn gesture_swipe_update(&mut self, delta: Point<f64, Logical>, time: u32) {
        if !self.gestures.running(Gesture::Swipe) {
            return;
        }
        let pointer = self.seat.get_pointer().expect("pointer");
        pointer.gesture_swipe_update(self, &SwipeUpdate { time, delta });
        pointer.frame(self);
    }

    fn gesture_pinch_update(&mut self, delta: Point<f64, Logical>, scale: f64, rotation: f64, time: u32) {
        if !self.gestures.running(Gesture::Pinch) {
            return;
        }
        let pointer = self.seat.get_pointer().expect("pointer");
        pointer.gesture_pinch_update(self, &PinchUpdate { time, delta, scale, rotation });
        pointer.frame(self);
    }

    fn gesture_end(&mut self, g: Gesture, cancelled: bool, time: u32) {
        if !self.gestures.end(g) {
            return;
        }
        let pointer = self.seat.get_pointer().expect("pointer");
        let serial = SERIAL_COUNTER.next_serial();
        match g {
            Gesture::Swipe => pointer.gesture_swipe_end(self, &GestureSwipeEndEvent { serial, time, cancelled }),
            Gesture::Pinch => pointer.gesture_pinch_end(self, &GesturePinchEndEvent { serial, time, cancelled }),
            Gesture::Hold => pointer.gesture_hold_end(self, &GestureHoldEndEvent { serial, time, cancelled }),
        }
        pointer.frame(self);
    }
}

/// Activates the pointer constraints of the surface that gets focus and deactivates those of the one that loses it.
fn update_constraint_focus(pointer: &PointerHandle<WmState>, new: Option<&WlSurface>) {
    let old = pointer.current_focus();
    if old.as_ref() == new {
        return;
    }
    if let Some(old) = old {
        with_pointer_constraint(&old, pointer, |c| {
            if let Some(c) = c {
                if c.is_active() {
                    c.deactivate();
                }
            }
        });
    }
    if let Some(new) = new {
        with_pointer_constraint(new, pointer, |c| {
            if let Some(c) = c {
                if !c.is_active() {
                    c.activate();
                }
            }
        });
    }
}

/// Creates the globals this module serves: pointer gestures, pointer constraints and relative pointer. The objects
/// are not kept: the globals live as long as the display.
pub fn register_protocols(dh: &DisplayHandle) {
    let _ = PointerGesturesState::new::<WmState>(dh);
    let _ = PointerConstraintsState::new::<WmState>(dh);
    let _ = RelativePointerManagerState::new::<WmState>(dh);
}

impl PointerConstraintsHandler for WmState {
    fn new_constraint(&mut self, surface: &WlSurface, pointer: &PointerHandle<Self>) {
        // a constraint created while the pointer is already on its surface starts active
        if pointer.current_focus().as_ref() == Some(surface) {
            with_pointer_constraint(surface, pointer, |c| {
                if let Some(c) = c {
                    if !c.is_active() {
                        c.activate();
                    }
                }
            });
        }
    }

    fn cursor_position_hint(&mut self, surface: &WlSurface, pointer: &PointerHandle<Self>, location: Point<f64, Logical>) {
        // a locked surface asks for the pointer to be left at `location` (surface-local) when the lock ends
        if !motion_locked(constraint_state(surface, pointer)) {
            return;
        }
        let Some((focus, origin)) = self.surface_under(pointer.current_location()) else { return };
        if &focus != surface {
            return;
        }
        let time = self.start.elapsed().as_millis() as u32;
        let location = origin + location;
        pointer.motion(self, Some((focus, origin)), &MotionEvent { location, serial: SERIAL_COUNTER.next_serial(), time });
        pointer.frame(self);
    }
}

delegate_pointer_constraints!(WmState);
delegate_relative_pointer!(WmState);
delegate_pointer_gestures!(WmState);

#[cfg(test)]
mod tests {
    use super::*;

    fn lookup<'a>(pairs: &'a [(&'static str, &'static str)]) -> impl Fn(&str) -> Option<&'static str> + 'a {
        move |key| pairs.iter().find(|(k, _)| *k == key).map(|(_, v)| *v)
    }

    #[test]
    fn defaults_match_the_shell_and_the_old_seat() {
        let d = InputPrefs::default();
        assert_eq!(InputPrefs::from_lookup(|_| None), d);
        assert_eq!((d.repeat_delay_ms, d.repeat_rate), (300, 30), "the seat's values before prefs");
        assert!(d.tap_to_click && d.two_finger_secondary && !d.natural_scroll);
        assert_eq!((d.cursor_size, d.cursor_theme.as_str(), d.layout.as_str()), (24, "default", "us"));
    }

    #[test]
    fn flags_accept_the_usual_spellings_and_reject_the_rest() {
        assert_eq!(parse_flag("1"), Some(true));
        assert_eq!(parse_flag(" On "), Some(true));
        assert_eq!(parse_flag("false"), Some(false));
        assert_eq!(parse_flag("NO"), Some(false));
        assert_eq!(parse_flag("maybe"), None);
        assert_eq!(parse_flag(""), None);
    }

    #[test]
    fn numbers_are_clamped_and_bad_values_keep_the_default() {
        let p = InputPrefs::from_lookup(lookup(&[("pointer_speed", "3.5"), ("key_repeat_delay_ms", "5"), ("key_repeat_rate", "500")]));
        assert_eq!(p.speed, 1.0);
        assert_eq!(p.repeat_delay_ms, 100);
        assert_eq!(p.repeat_rate, 100);
        let p = InputPrefs::from_lookup(lookup(&[("pointer_speed", "-7"), ("cursor_size", "4000"), ("key_repeat_delay_ms", "soon")]));
        assert_eq!(p.speed, -1.0);
        assert_eq!(p.cursor_size, 96);
        assert_eq!(p.repeat_delay_ms, 300, "not a number: the default");
        let p = InputPrefs::from_lookup(lookup(&[("pointer_speed", "NaN")]));
        assert_eq!(p.speed, 0.0, "NaN is not a speed");
    }

    #[test]
    fn layouts_and_themes_are_validated() {
        assert!(valid_layout("is") && valid_layout("gb") && !valid_layout("U S") && !valid_layout("english"));
        let p = InputPrefs::from_lookup(lookup(&[("keyboard_layout", "is")]));
        assert_eq!(p.layout, "is");
        let p = InputPrefs::from_lookup(lookup(&[("keyboard_layout", "../etc")]));
        assert_eq!(p.layout, "us");
        assert!(valid_theme("Adwaita") && !valid_theme("../x") && !valid_theme("a/b") && !valid_theme(""));
        let p = InputPrefs::from_lookup(lookup(&[("cursor_theme", "Bibata-Modern")]));
        assert_eq!(p.cursor_theme, "Bibata-Modern");
    }

    #[test]
    fn the_prefs_file_is_key_value_lines_and_the_last_line_wins() {
        let text = "# shell\nnatural_scroll=1\npointer_accel = flat\nkey_repeat_rate=40\nkey_repeat_rate=25\ngarbage line\n";
        let p = InputPrefs::parse(text);
        assert!(p.natural_scroll);
        assert_eq!(p.accel, Accel::Flat);
        assert_eq!(p.repeat_rate, 25);
        assert_eq!(InputPrefs::parse(""), InputPrefs::default());
    }

    #[test]
    fn an_adaptive_touchpad_with_all_features_gets_every_setting() {
        let prefs = InputPrefs { accel: Accel::Flat, speed: 0.25, natural_scroll: true, ..InputPrefs::default() };
        let caps = DeviceCaps { accel: true, natural_scroll: true, tap: true, clickfinger: true };
        assert_eq!(
            plan_device(&prefs, caps),
            DevicePlan { accel_flat: Some(true), speed: Some(0.25), natural_scroll: Some(true), tap: Some(true), clickfinger: Some(true) }
        );
    }

    #[test]
    fn a_device_without_a_feature_is_left_alone() {
        let prefs = InputPrefs { natural_scroll: true, tap_to_click: false, ..InputPrefs::default() };
        let mouse = DeviceCaps { accel: true, ..DeviceCaps::default() };
        let plan = plan_device(&prefs, mouse);
        assert_eq!(plan.natural_scroll, None, "a mouse with no natural scroll is not written");
        assert_eq!(plan.tap, None);
        assert_eq!(plan.accel_flat, Some(false), "the adaptive profile is written to a mouse");
        assert_eq!(plan_device(&prefs, DeviceCaps::default()), DevicePlan::default(), "a switch or a tablet gets nothing");
    }

    #[test]
    fn xcursor_dirs_search_home_then_each_data_dir() {
        let dirs = xcursor_dirs(Some("/home/u"), "/opt/share:/usr/share", "Adwaita");
        assert_eq!(
            dirs,
            vec![
                PathBuf::from("/home/u/.icons/Adwaita/cursors"),
                PathBuf::from("/opt/share/icons/Adwaita/cursors"),
                PathBuf::from("/usr/share/icons/Adwaita/cursors"),
            ]
        );
        let standard = xcursor_dirs(None, "", "default");
        assert_eq!(standard.len(), 2, "no home and no XDG_DATA_DIRS gives the standard two");
        assert_eq!(standard[0], PathBuf::from("/usr/local/share/icons/default/cursors"));
    }

    #[test]
    fn the_cursor_theme_and_size_reach_clients_as_xcursor_env() {
        let p = InputPrefs { cursor_size: 32, cursor_theme: "Bibata".into(), ..InputPrefs::default() };
        assert_eq!(cursor_env(&p), [("XCURSOR_THEME", "Bibata".to_string()), ("XCURSOR_SIZE", "32".to_string())]);
    }

    #[test]
    fn a_gesture_begin_pairs_with_one_end_per_kind() {
        let mut g = GestureGate::default();
        assert!(!g.running(Gesture::Swipe), "an update before a begin is dropped");
        assert!(g.begin(Gesture::Swipe));
        assert!(!g.begin(Gesture::Swipe), "a second begin of the same kind is dropped");
        assert!(g.begin(Gesture::Pinch), "a pinch runs next to a swipe");
        assert!(g.running(Gesture::Swipe) && g.running(Gesture::Pinch) && !g.running(Gesture::Hold));
        assert!(g.end(Gesture::Swipe));
        assert!(!g.end(Gesture::Swipe), "an end without a begin is dropped");
        assert!(g.running(Gesture::Pinch));
    }

    #[test]
    fn a_focus_change_ends_every_running_gesture_once() {
        let mut g = GestureGate::default();
        g.begin(Gesture::Hold);
        g.begin(Gesture::Swipe);
        assert_eq!(g.cancel_all(), vec![Gesture::Swipe, Gesture::Hold]);
        assert!(g.cancel_all().is_empty());
        assert!(g.begin(Gesture::Hold), "after the cancel a new hold can start");
    }

    #[test]
    fn a_locked_pointer_stays_only_while_the_lock_is_active() {
        assert!(motion_locked(Some((true, true))), "an active lock holds the pointer");
        assert!(!motion_locked(Some((false, true))), "a lock that is not active does not");
        assert!(!motion_locked(Some((true, false))), "a confinement lets the pointer move");
        assert!(!motion_locked(None), "no constraint on this surface");
    }
}
