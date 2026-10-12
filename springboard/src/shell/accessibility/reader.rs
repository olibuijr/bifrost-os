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
//! magnification and are not laid out as a list yet), a Settings sidebar in a split window, rows scrolled out of view, and
//! the built-in apps. See docs/parity/wire6-a11y-roles.md.

use super::tree::{add_app_tree, AxState};
use super::walk::{self, BAR, BAR_ROW, SETTINGS};
use super::AccessibilitySettings;
use crate::gfx::Surface;
use crate::shell::axmap::item_role;
use crate::shell::desk::full_size;
use crate::shell::desktop_bar::menu_spans;
use crate::shell::list::Spec;
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
        let e = live[next];
        self.cursor = Some(e.address.clone());
        Some(e)
    }

    /// Moves to the next (`forward`) or previous rotor, wrapping, and returns the rotor it lands on.
    pub fn cycle_rotor(&mut self, forward: bool) -> usize {
        let n = walk::rotor_count();
        self.rotor = if forward { (self.rotor + 1) % n } else { (self.rotor + n - 1) % n };
        self.rotor
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
            let (role, value) = item_role(item);
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

    /// The rows of the front Settings page (window-local points) and the window-local origin of its content, when the front
    /// window is Settings.
    fn settings_page(&self) -> Option<(Vec<Element>, (f32, f32))> {
        let desk = self.desk.as_ref()?;
        let front = desk.front()?;
        if front.app != AppId::Settings {
            return None;
        }
        let list = self.settings.as_ref()?.list()?;
        let ui = &self.ui;
        let title = if full_size(front.app) { 0 } else { self.desk_metrics().title };
        let origin = (front.r.x as f32, (front.r.y + title) as f32);
        let els = page_elements(list.spec(), &|g, i| {
            let (x, y, w, h) = list.row_rect(ui, g, i);
            Some(Rect { x, y, w, h })
        });
        Some((els, origin))
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
        if let Some(front) = desk.front() {
            match front.app {
                AppId::Settings => {
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

    /// Ctrl+Alt+Right or Left: moves the screen reader's cursor within the current rotor and speaks the element it lands on.
    pub(in crate::shell) fn reader_step(&mut self, forward: bool, _t: f64) -> Vec<Effect> {
        let els = self.reader_walk();
        let rotor = self.reader.rotor();
        if let Some(e) = self.reader.step_in(&els, rotor, forward) {
            self.announce.post(&walk::speech(e), AnnouncePriority::High);
        }
        self.dirty = true;
        Vec::new()
    }

    /// Ctrl+Alt+Down (true) or Up (false): picks the next or previous rotor and speaks its name.
    pub(in crate::shell) fn reader_rotor(&mut self, forward: bool, _t: f64) -> Vec<Effect> {
        let rotor = self.reader.cycle_rotor(forward);
        self.announce.post(walk::rotor_name(rotor), AnnouncePriority::High);
        self.dirty = true;
        Vec::new()
    }

    /// Ctrl+Alt+Space: presses the element under the cursor. A problem is spoken, not raised.
    pub(in crate::shell) fn reader_activate(&mut self, t: f64) -> Vec<Effect> {
        let els = self.reader_walk();
        let Some(el) = self.reader.focused(&els).cloned() else { return Vec::new() };
        if !el.enabled {
            self.announce.post(&walk::speech(&el), AnnouncePriority::High);
            return Vec::new();
        }
        let centre = el.bounds.map(|b| (b.x + b.w / 2.0, b.y + b.h / 2.0));
        let fx = match (el.handle.as_str(), centre) {
            #[cfg(feature = "desktop-wayland")]
            (BAR, Some((x, y))) => self.desktop_bar_event(Event::Down(x, y), t).unwrap_or_default(),
            #[cfg(feature = "desktop-wayland")]
            (BAR_ROW, Some((x, y))) => {
                let mut fx = self.desktop_bar_event(Event::Down(x, y), t).unwrap_or_default();
                fx.extend(self.desktop_bar_event(Event::Up(x, y), t + 0.04).unwrap_or_default());
                fx
            }
            (SETTINGS, Some((x, y))) => {
                let (ox, oy) = self.settings_page().map_or((0.0, 0.0), |(_, o)| o);
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
        };
        self.dirty = true;
        fx
    }

    /// Whether the frame must repaint everything under the focus ring.
    pub(in crate::shell) fn reader_damage(&self) -> bool {
        self.reader.busy()
    }

    /// Draws the focus ring around the cursor's element when the screen reader is on.
    pub(in crate::shell) fn draw_reader(&mut self, s: &mut Surface) {
        let mut drawn = false;
        if self.screen_reader_on() {
            let els = self.reader_walk();
            if let Some(b) = self.reader.focused(&els).and_then(|e| e.bounds) {
                let t = self.ui.pt(RING_PT);
                let r = self.ui.pt(SELECTION_RADIUS);
                s.stroke_rrect(b.x - t, b.y - t, b.w + 2.0 * t, b.h + 2.0 * t, r + t, t, LINK);
                drawn = true;
            }
        }
        self.reader.pending = false;
        self.reader.drawn = drawn;
    }
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
    fn the_cursor_follows_its_address_when_the_page_changes() {
        let mut els = page();
        let mut r = Reader::default();
        r.step(&els, true);
        r.step(&els, true);
        els.insert(0, el("settings/new", "page", "New", "", true));
        assert_eq!(r.focused(&els).map(|e| e.name.as_str()), Some("C"));
    }

    #[test]
    fn speech_says_kind_name_and_value() {
        assert_eq!(speech(&el("a", "toggle", "Bold Text", "on", true)), "switch, Bold Text, on");
        assert_eq!(speech(&el("a", "page", "Display Zoom", "Standard", true)), "button, Display Zoom, Standard");
        assert_eq!(speech(&el("a", "text", "Message", "Hello", true)), "Message, Hello");
        assert_eq!(speech(&el("a", "page", "Wi-Fi", "", false)), "button, Wi-Fi, unavailable");
    }
}
