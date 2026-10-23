//! Event editors in windows of their own (`bifrost_app::app_windows`). A double-click on an event opens its editor in a
//! window of the app; the main window keeps the grid.
//!
//! The editor is one set of state on `Calendar` (its draft, the three fields, the scroll and the touch state). A window
//! does not get a second copy of the calendar: each window keeps its own [`Pane`], and `Calendar::with_pane` swaps the
//! pane's state in while the window is drawn or sent input, runs the editor code the main window uses, and swaps it back.
//! The store, the events and the rest of the calendar are shared, so a save in a window is the same save as in the main
//! window. The main window sees no editor while a pane holds one (its `editor` is None when swapped back).
//!
//! Touches in a window come in points; the calendar lays out in pixels, so `window_input` in `main.rs` scales them.

use super::{Calendar, Draft, Target};
use springboard::deildu::scroll::Scroller;
use springboard::gfx::Surface;
use springboard::shell::textfield::TextField;
use springboard::shell::ui::Ui;

use bifrost_app::kit::ui::{MAC_BODY_PT, MAC_TITLE3_PT};

/// The size of an editor window, in points (the same size as a contact card window).
pub const EDITOR_WIDTH: f32 = 480.0;
pub const EDITOR_HEIGHT: f32 = 560.0;

/// Two taps on the same event within this many milliseconds are a double-click.
pub const DOUBLE_TAP_MS: f64 = 400.0;

/// The title of an editor window: the event's title, or "Event" for an event with none.
pub fn window_title(title: &str) -> String {
    let title = title.trim();
    if title.is_empty() { "Event".to_string() } else { title.to_string() }
}

/// The title, location and notes fields of an editor, as the editor lays them out (the same as the main window's).
pub(super) fn new_fields(ui: &Ui) -> [TextField; 3] {
    [("Title", MAC_TITLE3_PT), ("Location", MAC_BODY_PT), ("Notes", MAC_BODY_PT)].map(|(p, pt)| {
        let mut f = TextField::new(ui, p, pt, false, 160);
        f.focused = false;
        f
    })
}

/// One editor window: the editor state it holds while the window is not drawn, and the window's own geometry.
pub struct Pane {
    /// The shell's id of the window (`WindowId.0`).
    pub(super) window: u32,
    editor: Option<Draft>,
    fields: [TextField; 3],
    scroll: Scroller,
    pressed: Option<Target>,
    start: Option<(f32, f32)>,
    scrolling: bool,
    field_touch: bool,
    saved_scroll: f32,
    /// The window's geometry in pixels, with no sidebar (`Calendar::layout_columns` reads `full_w` and `side_hidden`).
    w: i32,
    h: i32,
    full_w: i32,
    side_hidden: bool,
}

impl Pane {
    fn new(ui: &Ui, window: u32) -> Self {
        let w = ui.pt(EDITOR_WIDTH) as i32;
        let h = ui.pt(EDITOR_HEIGHT) as i32;
        Pane {
            window,
            editor: None,
            fields: new_fields(ui),
            scroll: Scroller::new(),
            pressed: None,
            start: None,
            scrolling: false,
            field_touch: false,
            saved_scroll: 0.0,
            w,
            h,
            full_w: w,
            side_hidden: true,
        }
    }

    /// Exchanges the editor state of the pane and of the calendar. Called twice around each use, so it is its own inverse.
    fn swap_with(&mut self, c: &mut Calendar) {
        std::mem::swap(&mut self.editor, &mut c.editor);
        std::mem::swap(&mut self.fields, &mut c.fields);
        std::mem::swap(&mut self.scroll, &mut c.scroll);
        std::mem::swap(&mut self.pressed, &mut c.pressed);
        std::mem::swap(&mut self.start, &mut c.start);
        std::mem::swap(&mut self.scrolling, &mut c.scrolling);
        std::mem::swap(&mut self.field_touch, &mut c.field_touch);
        std::mem::swap(&mut self.saved_scroll, &mut c.saved_scroll);
        std::mem::swap(&mut self.w, &mut c.w);
        std::mem::swap(&mut self.h, &mut c.h);
        std::mem::swap(&mut self.full_w, &mut c.full_w);
        std::mem::swap(&mut self.side_hidden, &mut c.side_hidden);
    }
}

impl Calendar {
    /// The event a double-click asked to open in a window, once (the app opens the window).
    pub fn take_open_request(&mut self) -> Option<u64> {
        self.open_request.take()
    }

    /// The title of a saved event, for its window's title.
    pub fn event_title(&self, id: u64) -> Option<String> {
        self.events.iter().find(|e| e.id == id).map(|e| e.title.clone())
    }

    /// True when an editor window already shows event `id`, so a second double-click does not open another copy of it.
    pub fn editing_in_window(&self, id: u64) -> bool {
        self.panes.iter().any(|p| p.editor.as_ref().is_some_and(|d| d.event.id == id))
    }

    /// Records a tap on event `id` at `t` ms. The first tap selects the event; a second tap on it within
    /// [`DOUBLE_TAP_MS`] asks the app to open its editor in a window.
    pub fn tap_event(&mut self, id: u64, t: f64) {
        self.dirty = true;
        let double = self.last_tap.is_some_and(|(last, at)| last == id && t - at <= DOUBLE_TAP_MS);
        self.last_tap = if double { None } else { Some((id, t)) };
        if double {
            self.open_request = Some(id);
        } else {
            self.picked = Some(id);
        }
    }

    /// Opens the editor of event `id` in the window `window` (already opened by the app). False when the event is not
    /// saved, and then the window is not needed.
    pub fn open_pane(&mut self, ui: &Ui, window: u32, id: u64) -> bool {
        if !self.events.iter().any(|e| e.id == id) {
            return false;
        }
        let mut pane = Pane::new(ui, window);
        pane.swap_with(self);
        self.layout_columns(ui);
        self.open(ui, Some(id));
        let opened = self.editor.is_some();
        pane.swap_with(self);
        if opened {
            self.panes.push(pane);
        }
        opened
    }

    /// The event selected with a click (or last opened or saved), when it still exists.
    pub(super) fn picked_event(&self) -> Option<u64> {
        self.picked.filter(|id| self.events.iter().any(|e| e.id == *id))
    }

    /// Deletes the selected event with the Undo bar (Event > Delete Event, Backspace and Delete with no form open). An
    /// editor window on that event goes with it: its window is queued for the app to close (`take_closed_windows`).
    pub(super) fn delete_picked(&mut self, t: f64) -> bool {
        if self.editor.is_some() {
            return false;
        }
        if self.storage_blocked {
            self.error = Some("Saved events could not be read. Delete is unavailable.".into());
            return false;
        }
        let Some(id) = self.picked_event() else { return false };
        let Some(removed) = self.events.iter().find(|e| e.id == id).cloned() else { return false };
        let mut events = self.events.clone();
        events.retain(|e| e.id != id);
        match super::store::save(&self.path, &events) {
            Ok(()) => {
                self.events = events;
                self.changes.post();
                // the deleted event's extras wait with Undo, as when it is deleted from its form
                self.undo_extra = self.extras.events.remove(&id);
                self.save_extras();
                self.error = None;
                self.notice = None;
                self.undo = Some((removed, t + super::UNDO_MS));
                self.picked = None;
                self.last_tap = None;
                let gone: Vec<u32> = self.panes.iter().filter(|p| p.editor.as_ref().is_some_and(|d| d.event.id == id)).map(|p| p.window).collect();
                self.panes.retain(|p| !gone.contains(&p.window));
                self.closed_windows.extend(gone);
                self.dirty = true;
                true
            }
            Err(e) => {
                eprintln!("springboard: calendar: {e}");
                self.error = Some("Could not save. The event is still here.".into());
                false
            }
        }
    }

    /// The editor windows to close because their event was deleted elsewhere; taken once.
    pub fn take_closed_windows(&mut self) -> Vec<u32> {
        std::mem::take(&mut self.closed_windows)
    }

    /// Forgets the editor of a window that was closed (by the shell or by the app). Its unsaved edits are dropped.
    pub fn drop_pane(&mut self, window: u32) {
        self.panes.retain(|p| p.window != window);
    }

    /// Runs `f` with the editor of `window` in place, and returns its result with whether the editor is still open. The
    /// window is forgotten once its editor closes (Save, Delete, Cancel). None when no editor window has `window`.
    pub fn with_pane<R>(&mut self, window: u32, f: impl FnOnce(&mut Calendar) -> R) -> Option<(R, bool)> {
        let at = self.panes.iter().position(|p| p.window == window)?;
        let mut pane = self.panes.remove(at);
        pane.swap_with(self);
        let result = f(self);
        pane.swap_with(self);
        let open = pane.editor.is_some();
        if open {
            let at = at.min(self.panes.len());
            self.panes.insert(at, pane);
        }
        Some((result, open))
    }

    /// The scroll wheel over the editor window `window`, at `(x, y)` with travel `dy`, all in window points (as
    /// `App::window_pointer` delivers them; the editor lays out in pixels). True when the form scrolled; false when it did
    /// not, or no editor window has `window`.
    pub fn wheel_pane(&mut self, ui: &Ui, window: u32, x: f32, y: f32, dy: f32) -> bool {
        let k = ui.s;
        self.with_pane(window, |c| c.wheel(ui, x * k, y * k, dy * k)).is_some_and(|(moved, _)| moved)
    }

    /// Draws the editor of `window` at the size of the surface `s` (pixels), with the editor's toolbar. Returns false when
    /// no editor window has `window`.
    pub fn draw_pane(&mut self, window: u32, s: &mut Surface, ui: &Ui, t: f64) -> bool {
        let (w, h) = (s.w as i32, s.h as i32);
        self.with_pane(window, |c| {
            c.full_w = w;
            c.h = h;
            c.side_hidden = true;
            c.layout_columns(ui);
            c.background(s);
            c.draw_editor(s, ui, t);
            // an opaque band: the window material would show the main window's title through it (E2E W2-17)
            c.draw_chrome_opaque(s, ui, true);
            c.dirty = false;
        })
        .is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_untitled_event_gets_the_plain_window_title() {
        assert_eq!(window_title("  Lunch  "), "Lunch");
        assert_eq!(window_title("   "), "Event");
    }

    #[test]
    fn two_quick_taps_on_one_event_are_a_double_click() {
        // the window is opened by the app; here only the tap bookkeeping is checked, through the same rule the code uses
        let double = |last: Option<(u64, f64)>, id: u64, t: f64| last.is_some_and(|(l, at)| l == id && t - at <= DOUBLE_TAP_MS);
        assert!(double(Some((7, 1000.0)), 7, 1300.0));
        assert!(!double(Some((7, 1000.0)), 7, 1500.0), "too slow for a double-click");
        assert!(!double(Some((7, 1000.0)), 8, 1100.0), "a tap on another event is a new first tap");
        assert!(!double(None, 7, 0.0));
    }

    #[test]
    fn the_editor_window_is_the_size_of_a_card_window() {
        assert_eq!((EDITOR_WIDTH, EDITOR_HEIGHT), (480.0, 560.0));
    }
}
