//! Note windows: a note opened in a window of its own (a double-click on a row, or Note > Open in Separate Window).
//!
//! Each window is bound to one note by id. The window ids come from the SDK (`Ctx::open_window`), which keeps the app's
//! registry; this model only records which note each window shows, whether it floats (Window > Float on Top, through
//! `windows::set_window_level`) and which window was touched last, so the Window menu knows the focused window.
//!
//! The window is read-only for now: keyboard input is not routed to extra windows yet, so the window draws the note's
//! text and takes no typing. A window whose note is deleted keeps its id and draws only the background.

#![cfg_attr(not(test), allow(dead_code))]

use bifrost_app::app_windows::{WindowId, WindowSpec};
use bifrost_app::kit::ui::{CONTENT_BG, MAC_LARGE_TITLE_PT};
use super::settings::Settings;
use super::{ink, YELLOW};
use springboard::gfx::Surface;
use springboard::shell::textarea::TextArea;
use springboard::shell::ui::Ui;

/// The size of a note window, in points.
pub const WINDOW_W: f32 = 520.0;
pub const WINDOW_H: f32 = 600.0;
/// The inset of the text from the window edges, in points.
const INSET_PT: f32 = 20.0;
/// The top of the text, in points.
const TOP_PT: f32 = 20.0;
const TITLE_PT: f32 = MAC_LARGE_TITLE_PT;

/// The title of a note window: the note's title, or "New Note" for an empty note.
pub fn window_title(title: &str) -> String {
    let title = title.trim();
    if title.is_empty() { "New Note".to_string() } else { title.to_string() }
}

/// What to ask the SDK for when a note opens in its own window.
pub fn window_spec(title: &str) -> WindowSpec {
    WindowSpec::new(&window_title(title), WINDOW_W, WINDOW_H)
}

/// One open note window: the note it shows, whether it floats, and the text layout cached for drawing.
pub struct NoteWindow {
    window: WindowId,
    note: String,
    floating: bool,
    /// the laid-out text, and the width, ink and text size it was laid out for (`layout_key`)
    area: Option<(TextArea, [u32; 3])>,
}

/// The note windows, in the order they were opened.
#[derive(Default)]
pub struct NoteWindows {
    windows: Vec<NoteWindow>,
    focused: Option<WindowId>,
}

impl NoteWindows {
    pub fn new() -> Self {
        Self::default()
    }

    /// Records that `window` (opened with `window_spec`) shows `note`. A window that already shows a note is rebound.
    pub fn bind(&mut self, window: WindowId, note: &str) {
        self.windows.retain(|w| w.window != window);
        self.windows.push(NoteWindow { window, note: note.to_string(), floating: false, area: None });
    }

    /// Forgets a note window (the shell or the app closed it). Returns the note it showed.
    pub fn unbind(&mut self, window: WindowId) -> Option<String> {
        if self.focused == Some(window) {
            self.focused = None;
        }
        let index = self.windows.iter().position(|w| w.window == window)?;
        Some(self.windows.remove(index).note)
    }

    /// The window that shows a note, if one is open.
    pub fn window_of(&self, note: &str) -> Option<WindowId> {
        self.windows.iter().find(|w| w.note == note).map(|w| w.window)
    }

    /// The note a window shows. `None` for the main window and for a window that is not a note window.
    pub fn note_of(&self, window: WindowId) -> Option<&str> {
        self.windows.iter().find(|w| w.window == window).map(|w| w.note.as_str())
    }

    /// Makes a note window the focused one (the last one touched). A window that is not a note window is ignored.
    pub fn focus(&mut self, window: WindowId) {
        if self.windows.iter().any(|w| w.window == window) {
            self.focused = Some(window);
        }
    }

    /// The focused note window, if it is still open.
    pub fn focused(&self) -> Option<WindowId> {
        self.focused.filter(|f| self.windows.iter().any(|w| w.window == *f))
    }

    /// Whether a note window floats above the normal windows. False for a window that is not open.
    pub fn is_floating(&self, window: WindowId) -> bool {
        self.windows.iter().any(|w| w.window == window && w.floating)
    }

    /// Flips Float on Top for a note window. Returns the new state, or `None` for a window that is not open.
    pub fn toggle_floating(&mut self, window: WindowId) -> Option<bool> {
        let entry = self.windows.iter_mut().find(|w| w.window == window)?;
        entry.floating = !entry.floating;
        Some(entry.floating)
    }

    /// How many note windows are open.
    pub fn len(&self) -> usize {
        self.windows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.windows.is_empty()
    }
}

/// Draws a note window: the note's text (Markdown, as in the editor) at the window's size, with the Notes settings
/// (text size, dark background). `text` is the note's text, `None` when the note no longer exists (the window then
/// draws only the background).
pub fn draw(window: WindowId, windows: &mut NoteWindows, s: &mut Surface, ui: &Ui, text: Option<&str>, settings: Settings) {
    let dark = settings.dark_background;
    let background = if dark { CONTENT_BG.dark } else { CONTENT_BG.now() };
    s.fill_rect(0, 0, s.w as usize, s.h as usize, background);
    let Some(text) = text else { return };
    let Some(entry) = windows.windows.iter_mut().find(|w| w.window == window) else { return };
    let width = (s.w as f32 - 2.0 * ui.pt(INSET_PT)).max(1.0);
    let look = ink(dark);
    let body = settings.text_size.body_pt();
    let key = [width.to_bits(), look, body.to_bits()];
    let stale = match &entry.area {
        Some((area, k)) => area.text() != text || *k != key,
        None => true,
    };
    if stale {
        let mut area = TextArea::new(ui, width, body, TITLE_PT, look, YELLOW);
        area.set_detect_links(true);
        area.set_markdown(ui, true);
        area.set_text(ui, text);
        entry.area = Some((area, key));
    }
    if let Some((area, _)) = &mut entry.area {
        area.draw(s, ui, ui.pt(INSET_PT), ui.pt(TOP_PT), 0.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_window_spec_is_titled_with_the_trimmed_note_title_at_the_window_size() {
        let spec = window_spec("  Shopping  ");
        assert_eq!(spec.title, "Shopping");
        assert_eq!((spec.width, spec.height), (WINDOW_W, WINDOW_H));
    }

    #[test]
    fn an_untitled_note_window_gets_the_plain_title() {
        assert_eq!(window_title("   "), "New Note");
    }

    #[test]
    fn a_bound_note_is_found_by_note_and_by_window() {
        let mut w = NoteWindows::new();
        w.bind(WindowId(1), "a");
        w.bind(WindowId(2), "b");
        assert_eq!(w.window_of("b"), Some(WindowId(2)));
        assert_eq!(w.note_of(WindowId(1)), Some("a"));
        assert_eq!(w.note_of(WindowId::MAIN), None);
        assert_eq!(w.window_of("c"), None);
        assert_eq!(w.len(), 2);
    }

    #[test]
    fn unbinding_forgets_the_window_once_and_clears_its_focus() {
        let mut w = NoteWindows::new();
        w.bind(WindowId(1), "a");
        w.focus(WindowId(1));
        assert_eq!(w.unbind(WindowId(1)), Some("a".to_string()));
        assert_eq!(w.unbind(WindowId(1)), None);
        assert_eq!(w.focused(), None);
        assert!(w.is_empty());
    }

    #[test]
    fn focus_only_takes_a_note_window_that_is_open() {
        let mut w = NoteWindows::new();
        w.focus(WindowId(5));
        assert_eq!(w.focused(), None);
        w.bind(WindowId(5), "a");
        w.focus(WindowId(5));
        assert_eq!(w.focused(), Some(WindowId(5)));
    }

    #[test]
    fn float_on_top_flips_one_window_and_refuses_an_unknown_one() {
        let mut w = NoteWindows::new();
        w.bind(WindowId(1), "a");
        w.bind(WindowId(2), "b");
        assert_eq!(w.toggle_floating(WindowId(1)), Some(true));
        assert!(w.is_floating(WindowId(1)));
        assert!(!w.is_floating(WindowId(2)));
        assert_eq!(w.toggle_floating(WindowId(1)), Some(false));
        assert_eq!(w.toggle_floating(WindowId(9)), None);
    }

    #[test]
    fn rebinding_a_window_replaces_its_note_and_keeps_one_entry() {
        let mut w = NoteWindows::new();
        w.bind(WindowId(1), "a");
        w.bind(WindowId(1), "b");
        assert_eq!(w.len(), 1);
        assert_eq!(w.note_of(WindowId(1)), Some("b"));
    }
}
