//! The screen reader: a keyboard cursor over the page of the front Settings window.
//!
//! Settings > Accessibility > Vision > Screen Reader turns it on. Ctrl+Alt+Right and Ctrl+Alt+Left move the cursor to
//! the next or previous enabled row of the visible page, wrapping at the ends. Each move speaks the row (its name, its kind
//! and its value) through the announcement queue, so the banner shows it and the AT-SPI bridge speaks it, and a focus ring
//! is drawn around the row. The model (`Reader::step`, `speech`, `page_elements`) is pure and tested on the host; the `Shell`
//! methods at the bottom are the glue.
//!
//! Not walked yet: the sidebar of a split Settings window, rows scrolled out of view (the cursor does not scroll the page),
//! the menu bar, the Dock and app windows (an app's own controls are drawn by the app, so a ring from the shell would sit
//! under its surface). See docs/parity/wire5-screen-reader.md.

use super::AccessibilitySettings;
use crate::gfx::Surface;
use crate::shell::axmap::item_role;
use crate::shell::desk::full_size;
use crate::shell::list::Spec;
use crate::shell::ui::LINK;
use crate::shell::{AppId, Effect, Shell};
use bifrost_proto::AnnouncePriority;
use bifrost_ui::ax::{Element, Map};
use bifrost_ui::prefs_layout::m::SELECTION_RADIUS;
use bifrost_ui::Rect;

/// The focus ring's stroke, in points: the same width the SDK's focus ring draws.
const RING_PT: f32 = 2.0;

/// The cursor and the frame bookkeeping the damage check needs.
#[derive(Debug, Default)]
pub struct Reader {
    /// the address of the element the cursor is on
    cursor: Option<String>,
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
        self.pending = true;
        let live: Vec<&'a Element> = els.iter().filter(|e| e.enabled).collect();
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

    /// The element the cursor is on, looked up by address, so the cursor survives the page being rebuilt.
    pub fn focused<'a>(&self, els: &'a [Element]) -> Option<&'a Element> {
        let a = self.cursor.as_deref()?;
        els.iter().find(|e| e.address == a)
    }

    /// The address the cursor rests on.
    pub fn cursor(&self) -> Option<&str> {
        self.cursor.as_deref()
    }

    /// Whether the next frame must repaint the area under the ring: a move is pending, or the last frame drew one.
    pub fn busy(&self) -> bool {
        self.pending || self.drawn
    }
}

/// What the reader says for an element: its name, its kind, then its value, and "unavailable" when it cannot be used.
/// `Bold Text, switch, on` or `Display Zoom, button, Standard`.
pub fn speech(e: &Element) -> String {
    let kind = match e.role.as_str() {
        "page" => "button",
        "toggle" => "switch",
        "text" | "row" => "",
        other => other,
    };
    let mut parts: Vec<&str> = vec![e.name.as_str()];
    if !kind.is_empty() {
        parts.push(kind);
    }
    if !e.value.is_empty() {
        parts.push(e.value.as_str());
    }
    if !e.enabled {
        parts.push("unavailable");
    }
    parts.join(", ")
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

    /// The rows of the front window's page and the window-local origin of its content, when the front window is Settings.
    fn reader_elements(&self) -> Option<(Vec<Element>, (f32, f32))> {
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

    /// Ctrl+Alt+Right or Left: moves the screen reader's cursor and speaks the row it lands on.
    pub(in crate::shell) fn reader_step(&mut self, forward: bool, _t: f64) -> Vec<Effect> {
        let els = self.reader_elements().map(|(els, _)| els).unwrap_or_default();
        if let Some(e) = self.reader.step(&els, forward) {
            self.announce.post(&speech(e), AnnouncePriority::High);
        }
        self.dirty = true;
        Vec::new()
    }

    /// Whether the frame must repaint everything under the focus ring.
    pub(in crate::shell) fn reader_damage(&self) -> bool {
        self.reader.busy()
    }

    /// Draws the focus ring around the cursor's row when the screen reader is on and the front window is Settings.
    pub(in crate::shell) fn draw_reader(&mut self, s: &mut Surface) {
        let mut drawn = false;
        if self.screen_reader_on() {
            if let Some((els, (ox, oy))) = self.reader_elements() {
                if let Some(b) = self.reader.focused(&els).and_then(|e| e.bounds) {
                    let t = self.ui.pt(RING_PT);
                    let r = self.ui.pt(SELECTION_RADIUS);
                    s.stroke_rrect(ox + b.x - t, oy + b.y - t, b.w + 2.0 * t, b.h + 2.0 * t, r + t, t, LINK);
                    drawn = true;
                }
            }
        }
        self.reader.pending = false;
        self.reader.drawn = drawn;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn speech_says_name_kind_and_value() {
        assert_eq!(speech(&el("a", "toggle", "Bold Text", "on", true)), "Bold Text, switch, on");
        assert_eq!(speech(&el("a", "page", "Display Zoom", "Standard", true)), "Display Zoom, button, Standard");
        assert_eq!(speech(&el("a", "text", "Message", "Hello", true)), "Message, Hello");
        assert_eq!(speech(&el("a", "page", "Wi-Fi", "", false)), "Wi-Fi, button, unavailable");
    }
}
