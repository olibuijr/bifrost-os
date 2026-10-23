//! The screen reader: a keyboard cursor over what is on screen. It walks, in order: the menu bar titles, the rows of an open
//! menu, then the front window's content. The front window is a Settings page (its rows, from the page spec) or an app
//! window (the elements the app declared, through `add_app_tree`). Built-in apps other than Settings are not walked yet.
//!
//! Settings > Accessibility > Vision > Screen Reader turns it on. Keys (see `hwkeys.rs`):
//! - Ctrl+Alt+Right and Ctrl+Alt+Left: the next or previous element of the current rotor, wrapping at the ends.
//! - Ctrl+Alt+Down and Ctrl+Alt+Up: the next or previous rotor (All, Controls, Fields, Lists, Menu items, Text, Progress);
//!   the new rotor's name is spoken.
//! - Ctrl+Alt+Space: presses the element under the cursor (opens a menu title, runs a menu row, taps a Settings row, or
//!   presses an app element through the app's own tap path).
//!
//! Every move speaks the element as `kind, name, value` (`button, Close`) through the announcement queue, so the banner shows
//! it and the AT-SPI bridge speaks it, and a focus ring is drawn around it. The model (`Reader`, `speech`, `page_elements`)
//! is pure and tested on the host; the `Shell` methods at the bottom are the glue. Not walked: the Dock (its icons move with
//! magnification and are not laid out as a list yet), rows scrolled out of view, and
//! the built-in apps. See docs/parity/wire6-a11y-roles.md.

use super::earcons::{Earcon, ReaderEvent};
use super::tree::{add_app_tree, AxState};
use super::walk::{self, SETTINGS, SETTINGS_SIDEBAR};
#[cfg(feature = "desktop-wayland")]
use super::walk::{BAR, BAR_ROW};
use super::AccessibilitySettings;
use crate::gfx::Surface;
use crate::shell::axmap::item_role;
use crate::shell::desk::full_size;
#[cfg(feature = "desktop-wayland")]
use crate::shell::desktop_bar::menu_spans;
use crate::shell::list::{Accessory, Spec};
use crate::shell::ui::LINK;
use crate::shell::{AppId, Effect, Event, Shell};
use bifrost_proto::AnnouncePriority;
use bifrost_ui::ax::{Element, Map};
use bifrost_ui::prefs_layout::m::SELECTION_RADIUS;
use bifrost_ui::Rect;

/// The focus ring's stroke, in points: the same width the SDK's focus ring draws.
const RING_PT: f32 = 2.0;

/// The cursor, the rotor and the frame bookkeeping the damage check needs.
#[derive(Debug, Default)]
pub struct Reader {
    /// the address of the element the cursor is on
    cursor: Option<String>,
    /// the rotor index: 0 is All, `i > 0` walks `walk::rotor_name(i)`
    rotor: usize,
    /// a move happened since the last frame
    pending: bool,
    /// the last frame drew a ring, so the frame that removes it repaints the area
    drawn: bool,
    /// the sound option is off: no earcon is queued (false, the default, keeps them on)
    silent: bool,
    /// earcons asked for since the shell last took them
    earcons: Vec<Earcon>,
    /// the screen reader mode seen by the last `note_mode` (None before the first, so a start is not a toggle)
    mode: Option<bool>,
    /// Switch Scanning: the time the highlight steps next (None while scanning is off or has only just started)
    scan_at: Option<f64>,
}

impl Reader {
    /// Moves the cursor to the next (`forward`) or previous enabled element, wrapping at the ends. With no cursor yet, forward
    /// is the first enabled element and backward the last. Returns the element the cursor now rests on, or None (and forgets
    /// the cursor) when no element can be read.
    pub fn step<'a>(&mut self, els: &'a [Element], forward: bool) -> Option<&'a Element> {
        self.step_in(els, 0, forward)
    }

    /// `step`, over the elements of rotor `rotor` only (`walk::in_rotor`). The cursor keeps its place in the page order.
    pub fn step_in<'a>(&mut self, els: &'a [Element], rotor: usize, forward: bool) -> Option<&'a Element> {
        self.pending = true;
        let live: Vec<&'a Element> = els.iter().filter(|e| e.enabled && walk::in_rotor(e, rotor)).collect();
        if live.is_empty() {
            self.cursor = None;
            self.cue(ReaderEvent::Failed);
            return None;
        }
        let n = live.len();
        let at = self.cursor.as_ref().and_then(|a| live.iter().position(|e| &e.address == a));
        let next = match (at, forward) {
            (None, true) => 0,
            (None, false) => n - 1,
            (Some(p), true) => (p + 1) % n,
            (Some(p), false) => (p + n - 1) % n,
        };
        let wrapped = at.is_some_and(|p| if forward { next < p } else { next > p });
        self.cue(if wrapped { ReaderEvent::Wrapped } else { ReaderEvent::FocusMoved });
        let e = live[next];
        self.cursor = Some(e.address.clone());
        Some(e)
    }

    /// Moves to the next (`forward`) or previous rotor, wrapping, and returns the rotor it lands on.
    pub fn cycle_rotor(&mut self, forward: bool) -> usize {
        let n = walk::rotor_count();
        self.cue(ReaderEvent::RotorChanged);
        self.rotor = if forward { (self.rotor + 1) % n } else { (self.rotor + n - 1) % n };
        self.rotor
    }

    /// Turns the earcons on or off (the reader's sound option).
    pub fn set_sounds(&mut self, on: bool) {
        self.silent = !on;
        if !on {
            self.earcons.clear();
        }
    }

    /// Asks for the earcon of `event`, unless sounds are off.
    pub fn cue(&mut self, event: ReaderEvent) {
        if !self.silent {
            self.earcons.push(Earcon::for_event(event));
        }
    }

    /// The earcons asked for since the last call, in order; the shell's sound player plays them.
    pub fn take_earcons(&mut self) -> Vec<Earcon> {
        std::mem::take(&mut self.earcons)
    }

    /// Records whether the screen reader is on this frame. A change after the first call queues the mode cue.
    pub fn note_mode(&mut self, on: bool) {
        match self.mode {
            Some(prev) if prev == on => {}
            Some(_) => {
                self.mode = Some(on);
                self.cue(ReaderEvent::ModeToggled(on));
            }
            None => self.mode = Some(on),
        }
    }

    /// Switch Scanning: whether the highlight steps on now. With `on` false the timer is dropped; the first call after that
    /// starts it (and does not step), then every `interval_ms` of `now` gives one step.
    pub fn scan_due(&mut self, on: bool, now: f64, interval_ms: f64) -> bool {
        if !on {
            self.scan_at = None;
            return false;
        }
        match self.scan_at {
            None => {
                self.scan_at = Some(now + interval_ms);
                false
            }
            Some(at) if now >= at => {
                self.scan_at = Some(now + interval_ms);
                true
            }
            Some(_) => false,
        }
    }

    /// The rotor index the cursor walks.
    pub fn rotor(&self) -> usize {
        self.rotor
    }

    /// The element the cursor is on, looked up by address, so the cursor survives the page being rebuilt.
    pub fn focused<'a>(&self, els: &'a [Element]) -> Option<&'a Element> {
        let a = self.cursor.as_deref()?;
        els.iter().find(|e| e.address == a)
    }

    /// The address the cursor rests on.
    pub fn cursor(&self) -> Option<&str> {
        self.cursor.as_deref()
    }

    /// Whether the next frame must repaint everything under the focus ring: a move is pending, or the last frame drew one.
    pub fn busy(&self) -> bool {
        self.pending || self.drawn
    }
}

/// The rows of a Settings page as the screen reader sees them, in page order, with the roles and values the element map
/// gives them. `rect` gives a row's rectangle in window-local points.
pub fn page_elements(spec: &Spec, rect: &dyn Fn(usize, usize) -> Option<Rect>) -> Vec<Element> {
    let mut map = Map::new();
    for (g, group) in spec.groups.iter().enumerate() {
        for (i, item) in group.items.iter().enumerate() {
            let (mut role, mut value) = item_role(item);
            // a trailing pop-up button is read as one (its chosen label is the value), not as a list choice
            if item.accessory == Accessory::Popup {
                role = "popup";
                if let Some(p) = item.popup.as_ref().and_then(|p| p.choices.get(p.selected)) {
                    value = p.clone();
                }
            }
            // a segmented control is read with the chosen segment as its value
            if item.accessory == Accessory::Segmented {
                if let Some(p) = item.segments.as_ref().and_then(|p| p.choices.get(p.selected)) {
                    value = p.clone();
                }
            }
            let enabled = item.tappable() || role == "text";
            map.add("settings", role, &item.label, &value, enabled, rect(g, i), "");
        }
    }
    map.elements().to_vec()
}

impl Shell {
    /// Whether the screen reader is on (Settings > Accessibility > Vision > Screen Reader).
    pub(in crate::shell) fn screen_reader_on(&self) -> bool {
        self.settings.as_ref().map_or_else(|| AccessibilitySettings::load(&self.prefs).screen_reader, |st| st.accessibility().screen_reader)
    }

    /// Whether Switch Control is on (Settings > Accessibility > Touch > Switch Control).
    pub(in crate::shell) fn switch_control_on(&self) -> bool {
        self.settings.as_ref().map_or_else(|| AccessibilitySettings::load(&self.prefs).switch_control, |st| st.accessibility().switch_control)
    }

    /// Whether the keyboard cursor and its ring are in use: the screen reader is on, or Switch Control steps through the screen
    /// with the same cursor (and selects with the same press), without the speech.
    pub(in crate::shell) fn reader_active(&self) -> bool {
        self.screen_reader_on() || self.switch_control_on()
    }

    /// The rows of the front Settings page (screen pixels, before the window origin is added) and the screen origin of the
    /// page pane's content, when the front window is Settings. In the split layout the pane starts right of the sidebar, so a
    /// row's window-local position is the origin plus its pane position.
    fn settings_page(&self) -> Option<(Vec<Element>, (f32, f32))> {
        let desk = self.desk.as_ref()?;
        let front = desk.front()?;
        if front.app != AppId::Settings {
            return None;
        }
        let st = self.settings.as_ref()?;
        let list = st.list()?;
        let ui = &self.ui;
        let title = if full_size(front.app) { 0 } else { self.desk_metrics().title };
        let origin = (front.r.x as f32 + st.pane_x(), (front.r.y + title) as f32);
        let els = page_elements(list.spec(), &|g, i| {
            let (x, y, w, h) = list.row_rect(ui, g, i);
            Some(Rect { x, y, w, h })
        });
        Some((els, origin))
    }

    /// The window origin (screen pixels) Settings rows are placed from: the window's content, below a title bar that is not
    /// full-size.
    fn settings_origin(&self) -> Option<(f32, f32)> {
        let front = self.desk.as_ref()?.front()?;
        let title = if full_size(front.app) { 0 } else { self.desk_metrics().title };
        Some((front.r.x as f32, (front.r.y + title) as f32))
    }

    /// The rows of the split layout's sidebar (screen pixels), when the front window is Settings. Their addresses sit under
    /// `settings-sidebar/`, so a sidebar row and a page row of the same name stay apart.
    fn settings_sidebar(&self) -> Vec<Element> {
        let (Some((ox, oy)), Some(front)) = (self.settings_origin(), self.desk.as_ref().and_then(|d| d.front())) else { return Vec::new() };
        if front.app != AppId::Settings {
            return Vec::new();
        }
        let Some(list) = self.settings.as_ref().and_then(|st| st.sidebar_list()) else { return Vec::new() };
        let ui = &self.ui;
        let mut els = page_elements(list.spec(), &|g, i| {
            let (x, y, w, h) = list.row_rect(ui, g, i);
            Some(Rect { x: x + ox, y: y + oy, w, h })
        });
        for e in &mut els {
            e.address = e.address.replacen("settings/", "settings-sidebar/", 1);
            e.handle = SETTINGS_SIDEBAR.to_string();
        }
        els
    }

    /// The elements an app window declared, with bounds in screen pixels: `origin` is the window's body and a point is
    /// `ui.pt(1.0)` pixels, as `ax_press` taps them.
    fn app_walk(&self, slot: usize, origin: (f32, f32)) -> Vec<Element> {
        let Some(Some(host)) = self.ext.get(slot) else { return Vec::new() };
        let scope = format!("app/{}", host.id);
        let mut map = Map::new();
        let mut st = AxState::default();
        add_app_tree(&mut map, &scope, &host.controls, &host.controls_tree, &mut st);
        let scale = self.ui.pt(1.0);
        let (ox, oy) = origin;
        map.elements()
            .iter()
            .filter(|e| e.address.starts_with(&scope))
            .map(|e| {
                let mut e = e.clone();
                e.bounds = e.bounds.map(|b| Rect { x: ox + b.x * scale, y: oy + b.y * scale, w: b.w * scale, h: b.h * scale });
                e
            })
            .collect()
    }

    /// Everything the cursor can reach now, in order, with bounds in screen pixels. Empty when there is no desktop.
    pub(in crate::shell) fn reader_walk(&self) -> Vec<Element> {
        let mut out = Vec::new();
        let Some(desk) = self.desk.as_ref() else { return out };
        // the menu bar exists only in the Wayland desktop build
        #[cfg(feature = "desktop-wayland")]
        {
        let bar = self.desktop_bar_height() as f32;
        for (_, x, w, label) in self.menu_buttons() {
            let slug = bifrost_ui::ax::slug(&label);
            out.push(walk::menu_title(&slug, &label, Rect { x, y: 0.0, w, h: bar }));
        }
        if let (Some(menu), Some((left, top, w, _))) = (self.desktop_menu, self.desktop_popup_rect()) {
            let rows = self.menu_rows(menu);
            let (spans, _) = menu_spans(&rows);
            let mut index = 0;
            for (row, (t0, h)) in rows.iter().zip(spans.iter()) {
                if row.text.is_empty() {
                    continue; // a separator
                }
                let b = Rect { x: left as f32, y: top as f32 + self.ui.pt(*t0), w: w as f32, h: self.ui.pt(*h) };
                out.push(walk::menu_row(index, &row.text, row.action.is_some(), b));
                index += 1;
            }
        }
        }
        // every window of the front app is a window of that app for the reader, its extra windows included
        if let Some(front) = desk.front() {
            let app = front.app.parent();
            for w in desk.wins.iter().filter(|w| w.app.parent() == app && !w.minimized) {
                let name = self.window_menu_name(w.app);
                let bounds = Rect { x: w.r.x as f32, y: w.r.y as f32, w: w.r.w as f32, h: w.r.h as f32 };
                let id = crate::shell::desk::win_id(w.app).unwrap_or(0);
                out.push(walk::window(id, &name, bounds));
            }
        }
        if let Some(front) = desk.front() {
            match front.app {
                AppId::Settings => {
                    out.extend(self.settings_sidebar());
                    if let Some((els, (ox, oy))) = self.settings_page() {
                        for mut e in els {
                            e.bounds = e.bounds.map(|b| Rect { x: b.x + ox, y: b.y + oy, ..b });
                            e.handle = SETTINGS.to_string();
                            out.push(e);
                        }
                    }
                }
                AppId::External(slot) => {
                    let m = self.desk_metrics();
                    let body = front.body(&m);
                    out.extend(self.app_walk(slot as usize, (body.x as f32, body.y as f32)));
                }
                _ => {}
            }
        }
        out
    }

    /// Whether Switch Scanning is on (Settings > Accessibility > Switch Scanning).
    pub(in crate::shell) fn switch_scan_on(&self) -> bool {
        self.settings.as_ref().map_or_else(|| AccessibilitySettings::load(&self.prefs).switch_scan, |st| st.accessibility().switch_scan)
    }

    /// The scan clock, once per tick: while Switch Scanning is on and the screen is in use, the highlight steps to the next
    /// element every `SWITCH_SCAN_MS`, speaking it as a keyboard move does. A press of Ctrl+Alt+Space picks it.
    pub(in crate::shell) fn switch_scan_tick(&mut self, t: f64) {
        let on = self.switch_scan_on() && self.screen_on && !self.is_locked();
        if self.reader.scan_due(on, t, super::SWITCH_SCAN_MS) {
            self.reader_step(true, t);
        }
    }

    /// Ctrl+Alt+Right or Left: moves the screen reader's cursor within the current rotor and speaks the element it lands on.
    pub(in crate::shell) fn reader_step(&mut self, forward: bool, _t: f64) -> Vec<Effect> {
        let els = self.reader_walk();
        let rotor = self.reader.rotor();
        if let Some(e) = self.reader.step_in(&els, rotor, forward) {
            let said = walk::speech(e);
            self.braille.show(&said);
            self.announce.post(&said, AnnouncePriority::High);
        }
        self.dirty = true;
        Vec::new()
    }

    /// Ctrl+Alt+Down (true) or Up (false): picks the next or previous rotor and speaks its name.
    pub(in crate::shell) fn reader_rotor(&mut self, forward: bool, _t: f64) -> Vec<Effect> {
        let rotor = self.reader.cycle_rotor(forward);
        self.braille.show(walk::rotor_name(rotor));
        self.announce.post(walk::rotor_name(rotor), AnnouncePriority::High);
        self.dirty = true;
        Vec::new()
    }

    /// Ctrl+Alt+Space: presses the element under the cursor. A problem is spoken, not raised.
    pub(in crate::shell) fn reader_activate(&mut self, t: f64) -> Vec<Effect> {
        let els = self.reader_walk();
        let Some(el) = self.reader.focused(&els).cloned() else { return Vec::new() };
        if !el.enabled {
            self.reader.cue(ReaderEvent::Failed);
            self.announce.post(&walk::speech(&el), AnnouncePriority::High);
            return Vec::new();
        }
        let fx = self.press_walked(&el, t);
        self.reader.cue(ReaderEvent::Activated);
        self.dirty = true;
        fx
    }

    /// Presses one element of `reader_walk` the way a tap on it would: opens a menu title, runs a menu row, taps a Settings
    /// row, or presses an app element through the app's own tap path. The screen reader's cursor and Voice Control's numbered
    /// labels both press through here. A problem is spoken, not raised.
    pub(in crate::shell) fn press_walked(&mut self, el: &Element, t: f64) -> Vec<Effect> {
        let centre = el.bounds.map(|b| (b.x + b.w / 2.0, b.y + b.h / 2.0));
        match (el.handle.as_str(), centre) {
            #[cfg(feature = "desktop-wayland")]
            (BAR, Some((x, y))) => self.desktop_bar_event(Event::Down(x, y), t).unwrap_or_default(),
            #[cfg(feature = "desktop-wayland")]
            (BAR_ROW, Some((x, y))) => {
                let mut fx = self.desktop_bar_event(Event::Down(x, y), t).unwrap_or_default();
                fx.extend(self.desktop_bar_event(Event::Up(x, y), t + 0.04).unwrap_or_default());
                fx
            }
            (SETTINGS_SIDEBAR, Some((x, y))) => {
                let (ox, oy) = self.settings_origin().unwrap_or((0.0, 0.0));
                let ui = &self.ui;
                match self.settings.as_mut() {
                    Some(st) => {
                        let mut fx = st.touch(ui, &mut self.prefs, Event::Down(x - ox, y - oy), t);
                        fx.extend(st.touch(ui, &mut self.prefs, Event::Up(x - ox, y - oy), t + 0.01));
                        fx
                    }
                    None => Vec::new(),
                }
            }
            (SETTINGS, Some((x, y))) => {
                // the touch is window-local, sidebar included: the page pane's own offset is the list's to remove
                let (ox, oy) = self.settings_origin().unwrap_or((0.0, 0.0));
                let ui = &self.ui;
                match self.settings.as_mut() {
                    Some(st) => {
                        let mut fx = st.touch(ui, &mut self.prefs, Event::Down(x - ox, y - oy), t);
                        fx.extend(st.touch(ui, &mut self.prefs, Event::Up(x - ox, y - oy), t + 0.01));
                        fx
                    }
                    None => Vec::new(),
                }
            }
            _ if el.address.starts_with("app/") => match self.ax_press(&el.address, t) {
                Ok((_, fx)) => fx,
                Err(e) => {
                    self.announce.post(&e, AnnouncePriority::High);
                    Vec::new()
                }
            },
            _ => Vec::new(),
        }
    }

    /// Whether the frame must repaint everything under the focus ring.
    pub(in crate::shell) fn reader_damage(&self) -> bool {
        self.reader.busy()
    }

    /// Draws the focus ring around the cursor's element when the screen reader is on.
    pub(in crate::shell) fn draw_reader(&mut self, s: &mut Surface) {
        let mut drawn = false;
        let mut ring_layer = None;
        if self.reader_active() || self.switch_scan_on() {
            let els = self.reader_walk();
            if let Some(b) = self.reader.focused(&els).and_then(|e| e.bounds) {
                let t = self.ui.pt(RING_PT);
                let r = self.ui.pt(SELECTION_RADIUS);
                let ring = ring_rect(b.x, b.y, b.w, b.h, t);
                if self.overlays_on_desk() && !self.is_locked() {
                    // on the desktop the ring is a surface of its own: drawn into the shell's frame it was under the
                    // windows and the menu bar, around exactly the elements it speaks (W5-19)
                    let (x0, y0) = (ring.0.floor(), ring.1.floor());
                    let (iw, ih) = ((ring.0 + ring.2 - x0).ceil().max(1.0) as usize + 1, (ring.1 + ring.3 - y0).ceil().max(1.0) as usize + 1);
                    let mut image = crate::gfx::Image::new(iw, ih);
                    let mut ls = Surface { w: iw, h: ih, stride: iw, px: &mut image.px, y0: 0 };
                    ls.stroke_rrect(ring.0 - x0, ring.1 - y0, ring.2, ring.3, r + t, t, LINK);
                    ring_layer = Some(crate::shell::capture_flow::Layer { x: x0 as i32, y: y0 as i32, image, input: Some([0, 0, 0, 0]) });
                } else {
                    s.stroke_rrect(ring.0, ring.1, ring.2, ring.3, r + t, t, LINK);
                }
                drawn = true;
            }
        }
        self.a11y_layers[1] = ring_layer;
        self.reader.pending = false;
        self.reader.drawn = drawn;
    }
}

/// The focus ring's rectangle (x, y, w, h) around an element's bounds, `t` px out on every side.
pub fn ring_rect(x: f32, y: f32, w: f32, h: f32, t: f32) -> (f32, f32, f32, f32) {
    (x - t, y - t, w + 2.0 * t, h + 2.0 * t)
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::walk::speech;

    fn el(addr: &str, role: &str, name: &str, value: &str, enabled: bool) -> Element {
        Element { address: addr.into(), name: name.into(), role: role.into(), value: value.into(), enabled, bounds: None, handle: String::new() }
    }

    fn page() -> Vec<Element> {
        vec![
            el("settings/a", "page", "A", "", true),
            el("settings/b", "text", "B", "", false),
            el("settings/c", "toggle", "C", "on", true),
            el("settings/d", "page", "D", "", true),
        ]
    }

    #[test]
    fn scanning_steps_on_the_interval_and_starts_over_when_switched_off() {
        let mut r = Reader::default();
        assert!(!r.scan_due(false, 0.0, 1500.0), "off never steps");
        assert!(!r.scan_due(true, 100.0, 1500.0), "the first call only starts the timer");
        assert!(!r.scan_due(true, 1599.0, 1500.0), "the interval is not up");
        assert!(r.scan_due(true, 1600.0, 1500.0), "the interval is up");
        assert!(!r.scan_due(true, 2000.0, 1500.0), "the next step is a full interval later");
        assert!(r.scan_due(true, 3100.0, 1500.0));
        assert!(!r.scan_due(false, 3200.0, 1500.0));
        assert!(!r.scan_due(true, 9000.0, 1500.0), "switching back on starts the timer again instead of stepping at once");
    }

    #[test]
    fn steps_forward_and_back_over_enabled_rows_and_wraps() {
        let els = page();
        let mut r = Reader::default();
        let name = |x: Option<&Element>| x.map(|e| e.name.clone());
        // enabled rows in order: A, C, D (B is disabled)
        assert_eq!(name(r.step(&els, true)), Some("A".to_string()), "the first step from nothing is the first row");
        assert_eq!(name(r.step(&els, true)), Some("C".to_string()), "the disabled row is skipped");
        assert_eq!(name(r.step(&els, true)), Some("D".to_string()));
        assert_eq!(name(r.step(&els, true)), Some("A".to_string()), "forward past the last wraps to the first");
        assert_eq!(name(r.step(&els, false)), Some("D".to_string()), "back from the first wraps to the last");
        assert_eq!(r.cursor(), Some("settings/d"));
    }

    #[test]
    fn a_rotor_steps_only_over_its_roles() {
        let els = page();
        let mut r = Reader::default();
        let name = |x: Option<&Element>| x.map(|e| e.name.clone());
        // Controls (rotor 1): buttons, toggles and sliders, so A, C, D
        assert_eq!(name(r.step_in(&els, 1, true)), Some("A".to_string()));
        assert_eq!(name(r.step_in(&els, 1, true)), Some("C".to_string()));
        // Text (rotor 5): only B, which is disabled, so nothing can be read
        assert!(r.step_in(&els, 5, true).is_none());
        assert_eq!(r.cursor(), None, "an empty rotor forgets the cursor");
    }

    #[test]
    fn rotors_cycle_both_ways_and_wrap() {
        let mut r = Reader::default();
        assert_eq!(r.rotor(), 0, "the first rotor is All");
        assert_eq!(r.cycle_rotor(true), 1);
        assert_eq!(r.cycle_rotor(false), 0);
        assert_eq!(r.cycle_rotor(false), 6, "backward from All wraps to the last rotor");
        assert_eq!(r.cycle_rotor(true), 0, "forward from the last wraps to All");
    }

    #[test]
    fn a_page_with_nothing_to_read_leaves_no_cursor() {
        let mut r = Reader::default();
        assert!(r.step(&[], true).is_none());
        assert_eq!(r.cursor(), None);
        let only_disabled = vec![el("x", "text", "X", "", false)];
        assert!(r.step(&only_disabled, false).is_none());
        assert!(r.busy(), "a move, even an empty one, repaints the frame");
    }

    #[test]
    fn the_ring_surrounds_the_element_by_its_stroke() {
        assert_eq!(ring_rect(10.0, 20.0, 100.0, 30.0, 3.0), (7.0, 17.0, 106.0, 36.0));
    }

    #[test]
    fn the_cursor_follows_its_address_when_the_page_changes() {
        let mut els = page();
        let mut r = Reader::default();
        r.step(&els, true);
        r.step(&els, true);
        els.insert(0, el("settings/new", "page", "New", "", true));
        assert_eq!(r.focused(&els).map(|e| e.name.as_str()), Some("C"));
    }

    #[test]
    fn a_move_queues_a_focus_earcon_and_a_wrap_queues_the_wrap_earcon() {
        let els = page();
        let mut r = Reader::default();
        r.step(&els, true);
        assert_eq!(r.take_earcons(), vec![Earcon::Focus]);
        r.step(&els, true);
        r.step(&els, true);
        r.take_earcons();
        r.step(&els, true);
        assert_eq!(r.take_earcons(), vec![Earcon::Wrap], "forward past the last wraps");
        r.step(&els, false);
        assert_eq!(r.take_earcons(), vec![Earcon::Wrap], "back from the first wraps");
        assert!(r.take_earcons().is_empty(), "taking drains the queue");
    }

    #[test]
    fn nothing_to_read_and_rotor_changes_queue_their_earcons() {
        let mut r = Reader::default();
        r.step(&[], true);
        r.cycle_rotor(true);
        assert_eq!(r.take_earcons(), vec![Earcon::Error, Earcon::PageTurn]);
    }

    #[test]
    fn no_earcon_is_queued_when_sounds_are_off() {
        let els = page();
        let mut r = Reader::default();
        r.set_sounds(false);
        r.step(&els, true);
        r.cue(ReaderEvent::Failed);
        assert!(r.take_earcons().is_empty());
        r.set_sounds(true);
        r.step(&els, true);
        assert_eq!(r.take_earcons(), vec![Earcon::Focus]);
    }

    #[test]
    fn switching_the_reader_on_or_off_queues_its_mode_cue_but_starting_does_not() {
        let mut r = Reader::default();
        r.note_mode(false);
        assert!(r.take_earcons().is_empty(), "the first state is not a change");
        r.note_mode(true);
        assert_eq!(r.take_earcons(), vec![Earcon::ModeOn]);
        r.note_mode(true);
        assert!(r.take_earcons().is_empty(), "the same mode again is not a change");
        r.note_mode(false);
        assert_eq!(r.take_earcons(), vec![Earcon::ModeOff]);
    }

    #[test]
    fn a_pop_up_row_is_read_as_a_pop_up_button_with_its_choice() {
        use crate::shell::list::{Group, Item, Spec};
        use crate::shell::pages::{Action, Pick, PageId};
        let spec = Spec {
            id: PageId::Accessibility,
            title: "Accessibility".into(),
            large: false,
            groups: vec![Group::new(vec![
                Item::new("Speech").popup(vec!["System Default".into(), "Icelandic".into()], 1, Action::Pick(Pick::SpeechVoice(3))),
                Item::new("Display Zoom").link(PageId::DisplayZoom),
            ])],
        };
        let els = page_elements(&spec, &|_: usize, _: usize| None::<Rect>);
        assert_eq!(els[0].role, "popup");
        assert_eq!(speech(&els[0]), "pop-up button, Speech, Icelandic", "the chosen label is the value");
        assert!(els[0].enabled, "a pop-up row opens its menu, so it can be pressed");
        assert_eq!(els[1].role, "page", "a row that opens a page keeps its button role");
    }

    #[test]
    fn speech_says_kind_name_and_value() {
        assert_eq!(speech(&el("a", "toggle", "Bold Text", "on", true)), "switch, Bold Text, on");
        assert_eq!(speech(&el("a", "page", "Display Zoom", "Standard", true)), "button, Display Zoom, Standard");
        assert_eq!(speech(&el("a", "text", "Message", "Hello", true)), "Message, Hello");
        assert_eq!(speech(&el("a", "page", "Wi-Fi", "", false)), "button, Wi-Fi, unavailable");
    }
}
