//! Note windows: a note opened in a window of its own (a double-click on a row, or Note > Open in Separate Window).
//!
//! Each window is bound to one note by id. The window ids come from the SDK (`Ctx::open_window`), which keeps the app's
//! registry; this model only records which note each window shows, whether it floats (Window > Float on Top, through
//! `windows::set_window_level`) and which window was touched last, so the Window menu knows the focused window.
//!
//! The window edits its note as the main editor does on the desktop: the text area runs in pointer mode (a click places
//! the caret, a drag, double and triple click and Shift-click select), the keys the shell sends to the key window type
//! and run the editing bindings, and Cut, Copy, Paste, Select All, Undo and Redo act on it (`Notes::window_*`). Every
//! change goes back to the note, which the autosave writes. A window whose note is deleted keeps its id and draws only
//! the background.
//!
//! A note longer than its window scrolls: the wheel moves it (`wheel`), and typing, the editing keys, Paste and Undo
//! keep the caret in view. The caret blinks in the key window only (`draw` with `key`, `next_blink`), and an input
//! method's marked text composes in the window being typed in (`text_input`).

#![cfg_attr(not(test), allow(dead_code))]

use bifrost_app::app_windows::{WindowId, WindowSpec};
use bifrost_app::kit::ui::{CONTENT_BG, MAC_LARGE_TITLE_PT};
use super::settings::Settings;
use super::{ink, YELLOW};
use springboard::gfx::Surface;
use springboard::shell::keyboard::Out;
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

/// Where a note window's text starts, in px from the window's top-left corner, before scrolling (the app turns the
/// window points of its touches and wheel into px).
pub fn text_origin(ui: &Ui) -> (f32, f32) {
    (ui.pt(INSET_PT), ui.pt(TOP_PT))
}

/// Room kept between the caret and the window's top and bottom edges when the text scrolls to show it, in points.
const CARET_PAD_PT: f32 = 12.0;

/// The mouse button in a note window: pressed, moved while pressed, released.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Press {
    Down,
    Move,
    Up,
}

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
    /// how far the text is scrolled up, in px
    scroll: f32,
    /// the window's height in px at the last frame (0 before the first), which bounds the scroll
    view_h: f32,
    /// the window holds the caret (a click or an edit in it); it shows and blinks while the window is key
    editing: bool,
}

impl NoteWindow {
    /// How far the text can scroll: its height with the top and bottom margins, past the window's height.
    fn max_scroll(&self, ui: &Ui) -> f32 {
        let Some((area, _)) = &self.area else { return 0.0 };
        (ui.pt(TOP_PT) * 2.0 + area.height() - self.view_h).max(0.0)
    }

    /// Scrolls so the caret is inside the window, with `CARET_PAD_PT` to spare. Nothing moves before the first frame.
    fn reveal_caret(&mut self, ui: &Ui) {
        if self.view_h <= 0.0 {
            return;
        }
        let Some((area, _)) = &self.area else { return };
        let (_, cy, _, ch) = area.caret_rect(ui);
        let first = area.caret_line() == 0;
        let (top, pad) = (ui.pt(TOP_PT) + cy, ui.pt(CARET_PAD_PT));
        if top + ch + pad - self.scroll > self.view_h {
            self.scroll = top + ch + pad - self.view_h;
        } else if top - pad < self.scroll {
            // the first line brings the top margin back with it
            self.scroll = if first { 0.0 } else { (top - pad).max(0.0) };
        }
        self.scroll = self.scroll.clamp(0.0, self.max_scroll(ui));
    }
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
        self.windows.push(NoteWindow { window, note: note.to_string(), floating: false, area: None, scroll: 0.0, view_h: 0.0, editing: false });
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

    fn area(&self, window: WindowId) -> Option<&TextArea> {
        self.windows.iter().find(|w| w.window == window).and_then(|w| w.area.as_ref().map(|(a, _)| a))
    }

    fn area_mut(&mut self, window: WindowId) -> Option<&mut TextArea> {
        self.windows.iter_mut().find(|w| w.window == window).and_then(|w| w.area.as_mut().map(|(a, _)| a))
    }

    fn entry_mut(&mut self, window: WindowId) -> Option<&mut NoteWindow> {
        self.windows.iter_mut().find(|w| w.window == window)
    }

    /// Runs an edit on the text area of `window`, which takes the caret, then scrolls the caret into view. `None` for a
    /// window that is not laid out.
    fn editing_area<R>(&mut self, ui: &Ui, window: WindowId, f: impl FnOnce(&mut TextArea) -> R) -> Option<R> {
        let entry = self.entry_mut(window)?;
        let (area, _) = entry.area.as_mut()?;
        area.focused = true;
        let out = f(area);
        entry.editing = true;
        entry.reveal_caret(ui);
        Some(out)
    }

    /// How far a note window's text is scrolled up, in px (0 for a window that is not open).
    pub fn scroll(&self, window: WindowId) -> f32 {
        self.windows.iter().find(|w| w.window == window).map_or(0.0, |w| w.scroll)
    }

    /// The scroll wheel over `window`: `dy` px moves the text (positive scrolls toward the top), within the text's
    /// height. True when it moved.
    pub fn wheel(&mut self, ui: &Ui, window: WindowId, dy: f32) -> bool {
        let Some(entry) = self.entry_mut(window) else { return false };
        let before = entry.scroll;
        entry.scroll = (before - dy).clamp(0.0, entry.max_scroll(ui));
        entry.scroll != before
    }

    /// When the caret of a note window next blinks: the window that holds the caret while it is key (the last frame
    /// drew it focused). `None` when no caret is showing.
    pub fn next_blink(&self, t: f64) -> Option<f64> {
        self.windows.iter().filter_map(|w| w.area.as_ref().and_then(|(a, _)| a.next_blink(t))).reduce(f64::min)
    }

    /// An input method's composition in `window` (`ToApp::SetMarkedText`, `UnmarkText`, `DeleteSurrounding`): the
    /// marked text composes at the window's caret, underlined, and the caret stays in view. `None` when the window
    /// takes no such input (not laid out, or another message); `Some(true)` when the text changed.
    pub fn text_input(&mut self, ui: &Ui, window: WindowId, ev: &bifrost_app::proto::ToApp, t: f64) -> Option<bool> {
        use bifrost_app::proto::ToApp;
        match ev {
            ToApp::SetMarkedText { text, sel_start, sel_len } => self.editing_area(ui, window, |a| {
                let before = a.text().to_string();
                a.set_marked_text(ui, text, *sel_start as usize, *sel_len as usize, t);
                a.text() != before
            }),
            ToApp::UnmarkText { commit } => self.editing_area(ui, window, |a| {
                let before = a.text().to_string();
                a.unmark_text(ui, *commit, t) && a.text() != before
            }),
            ToApp::DeleteSurrounding { before, after } => self.editing_area(ui, window, |a| a.delete_surrounding(ui, *before as usize, *after as usize, t)),
            _ => None,
        }
    }

    /// The marked text of `window` while an input method composes there (char range).
    pub fn marked_range(&self, window: WindowId) -> Option<std::ops::Range<usize>> {
        self.area(window)?.marked_range()
    }

    /// The Edit menu's Undo and Redo titles for `window` ("Undo Typing"); empty for a window that is not laid out.
    pub fn undo_titles(&self, window: WindowId) -> (String, String) {
        self.area(window).map(TextArea::undo_titles).unwrap_or_default()
    }

    /// The text a note window holds now (its edits included); `None` before its first frame laid the text out.
    pub fn text(&self, window: WindowId) -> Option<&str> {
        self.area(window).map(TextArea::text)
    }

    /// The mouse in `window` at window px `(x, y)` at `t` ms: a press places the caret (a double click selects the word,
    /// a triple click the paragraph, `extend` (Shift) moves the selection's free end), a drag selects, the release ends
    /// it. The press takes the caret from the other note windows. True when the window needs a new frame.
    pub fn pointer(&mut self, ui: &Ui, window: WindowId, press: Press, x: f32, y: f32, t: f64, extend: bool) -> bool {
        let (ox, oy) = text_origin(ui);
        let oy = oy - self.scroll(window);
        match press {
            Press::Down => {
                for w in self.windows.iter_mut().filter(|w| w.window != window) {
                    w.editing = false;
                    if let Some((a, _)) = &mut w.area {
                        a.focused = false;
                    }
                }
                let Some(entry) = self.entry_mut(window) else { return false };
                let Some((a, _)) = entry.area.as_mut() else { return false };
                a.press(ui, x - ox, y - oy, t, extend);
                entry.editing = true;
                true
            }
            Press::Move => self.area_mut(window).is_some_and(|a| a.drag_to(x - ox, y - oy)),
            Press::Up => self.area_mut(window).is_some_and(|a| {
                let was = a.pressed();
                a.release();
                was
            }),
        }
    }

    /// A hardware-keyboard combination in `window` (`TextArea::key_combo`: moves, Shift selections, word and line
    /// deletes, Cmd+A, Cmd+Z). `None` when it is not an editing key there; `Some(true)` when the text changed.
    pub fn key_combo(&mut self, ui: &Ui, window: WindowId, combo: bifrost_app::controls::keys::KeyCombo, t: f64) -> Option<bool> {
        self.editing_area(ui, window, |a| a.key_combo(ui, combo, t))?
    }

    /// Typing and the plain editing keys in `window`. True when the text changed.
    pub fn edit(&mut self, ui: &Ui, window: WindowId, out: &Out, t: f64) -> bool {
        self.editing_area(ui, window, |a| a.edit(ui, out, t)).unwrap_or(false)
    }

    /// Edit > Select All in `window`. False for a window that is not laid out.
    pub fn select_all(&mut self, window: WindowId) -> bool {
        let Some(entry) = self.entry_mut(window) else { return false };
        let Some((a, _)) = entry.area.as_mut() else { return false };
        a.select_all();
        a.focused = true;
        entry.editing = true;
        true
    }

    /// The selected text (Markdown, as the note keeps it) of `window`; `None` when nothing is selected.
    pub fn selected_text(&self, window: WindowId) -> Option<String> {
        let a = self.area(window)?;
        a.selection().filter(|r| r.end > r.start)?;
        Some(a.selected_text())
    }

    /// Replaces the selection of `window` (or inserts at its caret) with `text`: Paste, and Cut with an empty `text`.
    pub fn replace_selection(&mut self, ui: &Ui, window: WindowId, text: &str, t: f64) -> bool {
        self.editing_area(ui, window, |a| a.replace_selection(ui, text, t)).is_some()
    }

    /// Edit > Undo (or Redo, with `redo`) in `window`. True when the text changed.
    pub fn undo(&mut self, ui: &Ui, window: WindowId, redo: bool, t: f64) -> bool {
        self.editing_area(ui, window, |a| if redo { a.redo(ui, t) } else { a.undo(ui, t) }).unwrap_or(false)
    }

    /// Whether Undo and Redo have a step in `window`.
    pub fn can_undo_redo(&self, window: WindowId) -> (bool, bool) {
        self.area(window).map_or((false, false), |a| (a.can_undo(), a.can_redo()))
    }

    /// How many note windows are open.
    pub fn len(&self) -> usize {
        self.windows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.windows.is_empty()
    }
}

/// Draws a note window: the note's text (Markdown, as in the editor) at the window's size and scroll, with the Notes
/// settings (text size, dark background). `text` is the note's text, `None` when the note no longer exists (the window
/// then draws only the background). `is_key` says the window has the keys: only then does its caret show and blink.
#[allow(clippy::too_many_arguments)]
pub fn draw(window: WindowId, windows: &mut NoteWindows, s: &mut Surface, ui: &Ui, text: Option<&str>, settings: Settings, is_key: bool, t: f64) {
    let dark = settings.dark_background;
    let background = if dark { CONTENT_BG.dark } else { CONTENT_BG.now() };
    s.fill_rect(0, 0, s.w as usize, s.h as usize, background);
    let Some(text) = text else { return };
    let Some(entry) = windows.windows.iter_mut().find(|w| w.window == window) else { return };
    let width = (s.w as f32 - 2.0 * ui.pt(INSET_PT)).max(1.0);
    let look = ink(dark);
    let body = settings.text_size.body_pt();
    let key = [width.to_bits(), look, body.to_bits()];
    // the text area keeps its caret, selection and undo across frames: a new width relays it out, a change made
    // elsewhere (the main editor) replaces its text, and only a new ink or text size builds it again
    match &mut entry.area {
        Some((area, k)) if k[1..] == key[1..] => {
            if k[0] != key[0] {
                area.set_width(ui, width);
                k[0] = key[0];
            }
            if area.text() != text {
                area.set_text(ui, text);
            }
        }
        _ => {
            let mut area = TextArea::new(ui, width, body, TITLE_PT, look, YELLOW);
            area.set_detect_links(true);
            area.set_markdown(ui, true);
            // a note window is a desktop window: the mouse selects, there are no touch handles
            area.set_pointer_mode(true);
            area.set_text(ui, text);
            entry.area = Some((area, key));
        }
    }
    // a new height or a shorter text may leave the scroll past the end
    entry.view_h = s.h as f32;
    entry.scroll = entry.scroll.clamp(0.0, entry.max_scroll(ui));
    let (editing, scroll) = (entry.editing, entry.scroll);
    if let Some((area, _)) = &mut entry.area {
        // the caret shows in the key window only, and only once a click or an edit put it there
        area.focused = editing && is_key;
        area.draw(s, ui, ui.pt(INSET_PT), ui.pt(TOP_PT) - scroll, t);
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

    fn laid_out(text: &str) -> (NoteWindows, Ui) {
        let ui = crate::test_ui();
        let mut w = NoteWindows::new();
        w.bind(WindowId(1), "a");
        let mut buf = vec![0u32; 400 * 300];
        let mut s = Surface { w: 400, h: 300, stride: 400, px: &mut buf, y0: 0 };
        draw(WindowId(1), &mut w, &mut s, &ui, Some(text), Settings::default(), true, 0.0);
        (w, ui)
    }

    /// The pixel (window px) in the middle of the line before char `c`.
    fn char_px(w: &mut NoteWindows, ui: &Ui, c: usize) -> (f32, f32) {
        let a = w.area_mut(WindowId(1)).unwrap();
        let keep = a.caret();
        a.set_caret(c);
        let (x, y, _, h) = a.caret_rect(ui);
        a.set_caret(keep);
        let (ox, oy) = text_origin(ui);
        (ox + x, oy + y + h / 2.0)
    }

    #[test]
    fn a_note_window_selects_with_the_mouse_and_edits_its_text() {
        let (mut w, ui) = laid_out("Title\nthe quick brown fox");
        let win = WindowId(1);
        assert_eq!(w.text(win), Some("Title\nthe quick brown fox"));
        let (x0, y0) = char_px(&mut w, &ui, 10);
        let (x1, y1) = char_px(&mut w, &ui, 15);
        assert!(w.pointer(&ui, win, Press::Down, x0, y0, 1000.0, false));
        assert!(w.pointer(&ui, win, Press::Move, x1, y1, 1010.0, false));
        assert!(w.pointer(&ui, win, Press::Up, x1, y1, 1020.0, false));
        assert_eq!(w.selected_text(win).as_deref(), Some("quick"), "a drag selects");
        // Paste over the selection, then Undo puts the word back
        assert!(w.replace_selection(&ui, win, "slow", 2000.0));
        assert_eq!(w.text(win), Some("Title\nthe slow brown fox"));
        assert_eq!(w.can_undo_redo(win), (true, false));
        assert!(w.undo(&ui, win, false, 2100.0));
        assert_eq!(w.text(win), Some("Title\nthe quick brown fox"));
        // typing goes in at the caret; Select All takes everything
        assert!(w.select_all(win));
        assert_eq!(w.selected_text(win).as_deref(), Some("Title\nthe quick brown fox"));
        assert!(w.edit(&ui, win, &Out::Text("x".into()), 3000.0));
        assert_eq!(w.text(win), Some("x"));
        assert_eq!(w.selected_text(win), None);
        // a frame with the same text keeps the window's state; a change made elsewhere replaces the text
        let mut buf = vec![0u32; 400 * 300];
        let mut s = Surface { w: 400, h: 300, stride: 400, px: &mut buf, y0: 0 };
        draw(win, &mut w, &mut s, &ui, Some("x"), Settings::default(), true, 3100.0);
        assert_eq!(w.can_undo_redo(win).0, true, "the undo history survived the frame");
        draw(win, &mut w, &mut s, &ui, Some("from the editor"), Settings::default(), true, 3200.0);
        assert_eq!(w.text(win), Some("from the editor"));
    }

    #[test]
    fn the_keyboard_bindings_reach_a_note_window() {
        let (mut w, ui) = laid_out("one two three");
        let win = WindowId(1);
        let combo = |key: u32, shift: bool, alt: bool, meta: bool| bifrost_app::controls::keys::KeyCombo::from_evdev(key, false, shift, alt, meta).unwrap();
        // Cmd+Left to the start, then Shift+Option+Right takes the first word
        assert_eq!(w.key_combo(&ui, win, combo(105, false, false, true), 1000.0), Some(false));
        assert_eq!(w.key_combo(&ui, win, combo(106, true, true, false), 1100.0), Some(false));
        assert_eq!(w.selected_text(win).as_deref(), Some("one"));
        assert_eq!(w.key_combo(&ui, WindowId(9), combo(106, true, true, false), 1200.0), None, "an unknown window takes no keys");
    }

    /// A long note laid out in a 400 x 300 px window, with a frame drawn so the window knows its height.
    fn long_note() -> (NoteWindows, Ui) {
        let text: String = std::iter::once("Title".to_string()).chain((1..=60).map(|i| format!("line {i}"))).collect::<Vec<_>>().join("\n");
        laid_out(&text)
    }

    #[test]
    fn the_wheel_scrolls_a_long_note_within_its_text() {
        let (mut w, ui) = long_note();
        let win = WindowId(1);
        assert_eq!(w.scroll(win), 0.0);
        assert!(!w.wheel(&ui, win, 50.0), "the top does not scroll further up");
        assert!(w.wheel(&ui, win, -120.0));
        assert_eq!(w.scroll(win), 120.0);
        assert!(w.wheel(&ui, win, -1.0e6));
        let max = w.scroll(win);
        assert!(max > 120.0 && !w.wheel(&ui, win, -10.0), "the end stops the wheel");
        assert!(!w.wheel(&ui, WindowId(9), -10.0), "an unknown window does not scroll");
        // a click on a scrolled window lands on the text under the pointer
        w.wheel(&ui, win, 1.0e6);
        w.wheel(&ui, win, -200.0);
        let (x, y) = char_px(&mut w, &ui, 6);
        assert!(w.pointer(&ui, win, Press::Down, x, y, 1000.0, false));
        w.pointer(&ui, win, Press::Up, x, y, 1010.0, false);
        let a = w.area(win).unwrap();
        let (_, cy, _, _) = a.caret_rect(&ui);
        assert!(cy >= 200.0 - ui.pt(TOP_PT), "the caret went to a line scrolled into view, not the first line: {cy}");
    }

    #[test]
    fn typing_keeps_the_caret_in_view() {
        let (mut w, ui) = long_note();
        let win = WindowId(1);
        // Cmd+Down to the end of the text, then type: the window scrolls down to the caret
        let end = bifrost_app::controls::keys::KeyCombo::from_evdev(108, false, false, false, true).unwrap();
        w.key_combo(&ui, win, end, 1000.0);
        assert!(w.edit(&ui, win, &Out::Text("!".into()), 1100.0));
        let a = w.area(win).unwrap();
        let (_, cy, _, ch) = a.caret_rect(&ui);
        let bottom = ui.pt(TOP_PT) + cy + ch - w.scroll(win);
        assert!(w.scroll(win) > 0.0 && bottom <= 300.0, "the caret is inside the window: {bottom}");
        // Cmd+Up back to the start scrolls to the top
        let start = bifrost_app::controls::keys::KeyCombo::from_evdev(103, false, false, false, true).unwrap();
        w.key_combo(&ui, win, start, 1200.0);
        assert_eq!(w.scroll(win), 0.0);
    }

    #[test]
    fn the_caret_blinks_only_in_the_key_window_that_holds_it() {
        let (mut w, ui) = laid_out("Title\nbody");
        let win = WindowId(1);
        assert_eq!(w.next_blink(100.0), None, "no caret before a click or an edit");
        assert!(w.edit(&ui, win, &Out::Text("x".into()), 1000.0));
        assert_eq!(w.next_blink(1100.0), Some(1500.0));
        let mut buf = vec![0u32; 400 * 300];
        let mut s = Surface { w: 400, h: 300, stride: 400, px: &mut buf, y0: 0 };
        draw(win, &mut w, &mut s, &ui, Some("Title\nbodyx"), Settings::default(), false, 1200.0);
        assert_eq!(w.next_blink(1200.0), None, "a window that is not key shows no caret");
        draw(win, &mut w, &mut s, &ui, Some("Title\nbodyx"), Settings::default(), true, 1300.0);
        assert_eq!(w.next_blink(1300.0), Some(1500.0), "key again, the caret blinks on");
    }

    #[test]
    fn marked_text_composes_in_the_note_window() {
        use bifrost_app::proto::ToApp;
        let (mut w, ui) = laid_out("Title\nk");
        let win = WindowId(1);
        let marked = ToApp::SetMarkedText { text: "´".into(), sel_start: 2, sel_len: 0 };
        assert_eq!(w.text_input(&ui, win, &marked, 1000.0), Some(true));
        assert_eq!(w.text(win), Some("Title\nk´"));
        assert_eq!(w.marked_range(win), Some(7..8));
        let composed = ToApp::SetMarkedText { text: "á".into(), sel_start: 2, sel_len: 0 };
        assert_eq!(w.text_input(&ui, win, &composed, 1100.0), Some(true));
        assert_eq!(w.text_input(&ui, win, &ToApp::UnmarkText { commit: true }, 1200.0), Some(false), "the commit keeps the text as it is");
        assert_eq!(w.text(win), Some("Title\nká"));
        assert_eq!(w.marked_range(win), None);
        assert_eq!(w.undo_titles(win).0, "Undo Typing", "the Edit menu names the window's own step");
        assert_eq!(w.text_input(&ui, WindowId(9), &ToApp::UnmarkText { commit: true }, 1300.0), None);
        assert_eq!(w.text_input(&ui, win, &ToApp::AccentHold { letter: 'a' }, 1300.0), None, "no accent list here");
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
