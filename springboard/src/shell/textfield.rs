//! A single-line text field: placeholder, caret with insertion point,
//! arrow-key and tap caret placement, secure (bulleted) entry and a caret
//! that blinks on the frame clock. Touch editing as on the bifrost: a long
//! press places the caret and offers Select / Select All / Paste, a double
//! tap selects a word, the selection's lollipop handles drag, and the menu
//! cuts, copies and pastes through the shell clipboard.

use super::edit::{self, Action, EditTarget};
use super::editmenu::EditMenu;
use super::keyboard::Out;
use super::ui::{Ui, Weight, WHITE};
use super::wifi_page::BLUE;
use super::Event;
use crate::gfx::{pack, rgba, Image, Surface};
use crate::scene::STATUS_PT;

/// Caret on/off period in ms (bifrost blinks at roughly 1 Hz).
const BLINK_MS: f64 = 500.0;
const BULLET: char = '•';
/// A press this long without moving becomes a long press.
pub const LONG_PRESS_MS: f64 = 400.0;
/// Two taps this close in time and space select a word.
const DOUBLE_TAP_MS: f64 = 350.0;
const DOUBLE_TAP_PT: f32 = 20.0;
/// Finger travel that cancels a long press.
const SLOP_PT: f32 = 10.0;
/// The selection handles: a bar beside the selection with a dot at its end.
const DOT_PT: f32 = 10.0;
const BAR_PT: f32 = 2.0;
/// How far from a handle's dot a finger still grabs it.
const HANDLE_HIT_PT: f32 = 22.0;
/// The selection tint (the bifrost's selection blue) and its 20% highlight.
pub const SELECT: u32 = pack(70, 147, 255, 255);
const HIGHLIGHT_ALPHA: u8 = 51;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Handle {
    Start,
    End,
}

#[derive(Clone, Copy, Debug)]
struct Press {
    x: f32,
    y: f32,
    t: f64,
    moved: bool,
    long: bool,
    handle: Option<Handle>,
    /// screen x of the text's left edge when the press began
    origin: f32,
}

/// The px box of the text area, as handed to `draw`.
pub type Frame = (f32, f32, f32, f32);

pub struct TextField {
    text: String,
    /// insertion point (and the moving end of a selection) as a char index into `text`
    caret: usize,
    /// the fixed end of the selection while one exists
    anchor: Option<usize>,
    max: usize,
    secure: bool,
    pub focused: bool,
    size_pt: f32,
    placeholder: Image,
    rendered: Option<Image>,
    /// px from the text's left edge to the caret
    caret_px: f32,
    /// the composing (marked) range in chars, while an input method has text in progress; drawn underlined
    marked: Option<(usize, usize)>,
    /// when the caret was last reset to visible (an edit or a focus)
    blink_from: f64,
    press: Option<Press>,
    /// a finger holding the menu
    menu_touch: bool,
    /// the last single tap: (x, t), for double-tap detection
    last_tap: Option<(f32, f64)>,
    /// after a double tap: where and when, so a third tap selects everything
    word_tap: Option<(f32, f64)>,
    menu: Option<EditMenu>,
    /// the text and caret before the last edit, for Ctrl+Z
    undo: Option<(String, usize)>,
    /// what Undo took away, for Ctrl+Shift+Z; any new edit drops it
    redo: Option<(String, usize)>,
}

impl TextField {
    /// `placeholder` shows while empty; `max` caps the length in chars.
    pub fn new(ui: &Ui, placeholder: &str, size_pt: f32, secure: bool, max: usize) -> Self {
        TextField {
            text: String::new(),
            caret: 0,
            anchor: None,
            max,
            secure,
            focused: true,
            size_pt,
            placeholder: ui.text(placeholder, size_pt, Weight::Regular, WHITE),
            rendered: None,
            caret_px: 0.0,
            marked: None,
            blink_from: 0.0,
            press: None,
            menu_touch: false,
            last_tap: None,
            word_tap: None,
            menu: None,
            undo: None,
            redo: None,
        }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn caret(&self) -> usize {
        self.caret
    }

    /// True for a password field: its text is never shown, copied or handed to anything.
    pub fn is_secure(&self) -> bool {
        self.secure
    }

    /// The field as an accessibility text element: caret, selection and length in characters. A secure field is marked
    /// `secure`, so the bridge exposes bullets and a length, never the text.
    pub fn ax_text(&self, id: u64) -> bifrost_ui::ax::AxText {
        let (a, b) = self.selection();
        let n = self.len() as u32;
        bifrost_ui::ax::AxText { id, caret: self.caret as u32, sel_start: a as u32, sel_end: b as u32, vis_start: 0, vis_end: n, secure: self.secure, len: n }
    }

    pub fn len(&self) -> usize {
        self.text.chars().count()
    }

    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    /// The editing menu while it shows.
    pub fn menu(&self) -> Option<&EditMenu> {
        self.menu.as_ref()
    }

    /// Replaces the contents, caret at the end.
    pub fn set_text(&mut self, ui: &Ui, text: &str) {
        self.text = text.chars().take(self.max).collect();
        self.caret = self.len();
        self.anchor = None;
        self.menu = None;
        self.marked = None;
        self.render(ui);
    }

    /// The composing (marked) range in chars, when an input method has text in progress.
    pub fn marked_range(&self) -> Option<(usize, usize)> {
        self.marked
    }

    /// An input method's composition (`SetMarkedText`): `text` replaces the marked range, or the selection when none is
    /// open, and is marked for underlining. The caret goes to `sel_start` (a byte offset into `text`, as the protocol
    /// sends it), or to the end when that is past the text. The text is cut at the field's limit.
    pub fn set_marked(&mut self, ui: &Ui, text: &str, sel_start: usize) {
        self.menu = None;
        let (start, end) = self.marked.unwrap_or_else(|| self.selection());
        let before: String = self.text.chars().take(start).collect();
        let after: String = self.text.chars().skip(end).collect();
        let room = self.max.saturating_sub(before.chars().count() + after.chars().count());
        let mid: String = text.chars().take(room).collect();
        let mid_len = mid.chars().count();
        self.text = format!("{before}{mid}{after}");
        self.marked = Some((start, start + mid_len));
        let within = text.get(..sel_start.min(text.len())).map_or(mid_len, |p| p.chars().count().min(mid_len));
        self.caret = start + within;
        self.anchor = None;
        self.render(ui);
    }

    /// The input method finished its composition. `commit` keeps the marked text; otherwise it is removed and the caret
    /// falls back to where the composition began.
    pub fn unmark(&mut self, ui: &Ui, commit: bool) {
        let Some((start, end)) = self.marked.take() else { return };
        if !commit {
            self.text = self.text.chars().enumerate().filter(|(i, _)| *i < start || *i >= end).map(|(_, c)| c).collect();
            self.caret = if self.caret >= end { self.caret - (end - start) } else { self.caret.min(start) };
        }
        self.anchor = None;
        self.render(ui);
    }

    /// What is drawn: bullets in secure mode, the text otherwise.
    fn shown(&self) -> String {
        if self.secure {
            std::iter::repeat_n(BULLET, self.len()).collect()
        } else {
            self.text.clone()
        }
    }

    fn byte_at(&self, chars: usize) -> usize {
        self.text.char_indices().nth(chars).map_or(self.text.len(), |(b, _)| b)
    }

    /// Applies a key; true when return was pressed. `t` restarts the blink
    /// so the caret is visible right after each edit. Typing replaces a
    /// selection, backspace deletes it, arrows collapse it.
    pub fn edit(&mut self, ui: &Ui, out: &Out, t: f64) -> bool {
        self.blink_from = t;
        self.menu = None;
        // a key the input method did not take ends the composition: the marked text stays as plain text
        self.marked = None;
        let (start, end) = self.selection();
        if matches!(out, Out::Text(_) | Out::Backspace) {
            self.snapshot();
        }
        match out {
            Out::Text(s) => self.replace(ui, (start, end), s),
            Out::Backspace => {
                if start < end {
                    self.replace(ui, (start, end), "");
                } else if self.caret > 0 {
                    self.replace(ui, (edit::prev_boundary(&self.text, self.caret), self.caret), "");
                }
            }
            Out::Named("Left") => self.caret = if start < end { start } else { edit::prev_boundary(&self.text, self.caret) },
            Out::Named("Right") => self.caret = if start < end { end } else { edit::next_boundary(&self.text, self.caret) },
            // Home, End, Option and Cmd with the arrows, Shift with all of them: the hardware keyboard's movement keys
            Out::Named(name @ ("Home" | "End" | "WordLeft" | "WordRight")) => self.caret = self.move_target(name),
            Out::Named(name @ ("ShiftHome" | "ShiftEnd" | "ShiftWordLeft" | "ShiftWordRight" | "ShiftLeft" | "ShiftRight")) => {
                let from = self.anchor.unwrap_or(self.caret);
                self.caret = self.move_target(name);
                self.anchor = Some(from);
            }
            // Delete (forward), Option+Backspace and Option+Delete (a word), Cmd+Backspace (everything before the caret)
            Out::Named(name @ ("DeleteForward" | "DeleteWord" | "DeleteWordForward" | "DeleteLine")) => {
                let range = if start < end {
                    (start, end)
                } else {
                    match *name {
                        "DeleteForward" => (self.caret, edit::next_boundary(&self.text, self.caret)),
                        "DeleteWord" => (self.move_target("WordLeft"), self.caret),
                        "DeleteWordForward" => (self.caret, self.move_target("WordRight")),
                        _ => (0, self.caret),
                    }
                };
                if range.0 < range.1 {
                    self.edit_with_undo(ui, |f, ui| f.replace(ui, range, ""));
                }
            }
            Out::Enter => return true,
            // hardware-keyboard shortcuts (Ctrl/Cmd + C, X, V, A, Z, Shift+Z)
            // a secure field never puts its text on the clipboard: Copy and Cut do nothing there, as in the touch edit menu
            Out::Named("Copy" | "Cut") if self.secure => {}
            Out::Named("Copy") => edit::apply(Action::Copy, self, ui),
            Out::Named("Cut") => self.edit_with_undo(ui, |f, ui| edit::apply(Action::Cut, f, ui)),
            Out::Named("Paste") => self.edit_with_undo(ui, |f, ui| edit::apply(Action::Paste, f, ui)),
            Out::Named("SelectAll") => edit::apply(Action::SelectAll, self, ui),
            Out::Named("Undo") => self.swap_history(false),
            Out::Named("Redo") => self.swap_history(true),
            Out::Hide | Out::Named(_) => {}
        }
        // Select All and Copy leave the selection standing for the next shortcut, and so do the keys that extend it
        if !matches!(out, Out::Named("SelectAll" | "Copy" | "ShiftHome" | "ShiftEnd" | "ShiftWordLeft" | "ShiftWordRight" | "ShiftLeft" | "ShiftRight")) {
            self.anchor = None;
        }
        self.render(ui);
        false
    }

    /// The char index a movement key takes the caret to: the ends of the text, the previous or next word (a secure field is
    /// one word, so its text gives nothing away), or the previous or next character.
    fn move_target(&self, name: &str) -> usize {
        let n = self.len();
        match name {
            "Home" | "ShiftHome" => 0,
            "End" | "ShiftEnd" => n,
            "WordLeft" | "ShiftWordLeft" if self.secure => 0,
            "WordRight" | "ShiftWordRight" if self.secure => n,
            "WordLeft" | "ShiftWordLeft" => edit::word_start_before(&self.text, self.caret),
            "WordRight" | "ShiftWordRight" => edit::word_end_after(&self.text, self.caret),
            "ShiftLeft" => edit::prev_boundary(&self.text, self.caret),
            "ShiftRight" => edit::next_boundary(&self.text, self.caret),
            _ => self.caret,
        }
    }

    /// Undo (or, with `redo`, Redo): restores the saved text and keeps the
    /// current one for the opposite step.
    fn swap_history(&mut self, redo: bool) {
        let (from, to) = if redo { (&mut self.redo, &mut self.undo) } else { (&mut self.undo, &mut self.redo) };
        if let Some((text, caret)) = from.take() {
            *to = Some((std::mem::replace(&mut self.text, text), self.caret));
            self.caret = caret.min(self.len());
        }
    }

    /// Remembers the text and caret so Ctrl+Z can restore them. One step:
    /// a run of typing in one word is a single step, as on the bifrost.
    fn snapshot(&mut self) {
        let boundary = self.text.chars().last().map_or(true, char::is_whitespace);
        self.redo = None;
        if self.undo.is_none() || boundary {
            self.undo = Some((self.text.clone(), self.caret));
        }
    }

    fn edit_with_undo(&mut self, ui: &Ui, f: impl FnOnce(&mut Self, &Ui)) {
        self.undo = Some((self.text.clone(), self.caret));
        self.redo = None;
        f(self, ui);
    }

    fn render(&mut self, ui: &Ui) {
        let shown = self.shown();
        self.caret_px = self.x_of(ui, self.caret);
        self.rendered = (!shown.is_empty()).then(|| ui.text(&shown, self.size_pt, Weight::Regular, WHITE));
    }

    /// Px from the text's left edge to the boundary before char `i`,
    /// measured as `ui.text` draws it.
    fn x_of(&self, ui: &Ui, i: usize) -> f32 {
        let prefix: String = self.shown().chars().take(i).collect();
        ui.measure(&prefix, self.size_pt, Weight::Regular)
    }

    /// The character boundary nearest `x` px from the text's left edge.
    fn index_at(&self, ui: &Ui, x: f32) -> usize {
        let mut best = (0usize, f32::MAX);
        for i in 0..=self.len() {
            let d = (self.x_of(ui, i) - x).abs();
            if d < best.1 {
                best = (i, d);
            }
        }
        best.0
    }

    /// The char whose box holds `x` px from the text's left edge (the last
    /// char past the end).
    fn char_at(&self, ui: &Ui, x: f32) -> usize {
        (0..self.len()).find(|&i| x < self.x_of(ui, i + 1)).unwrap_or(self.len().saturating_sub(1))
    }

    /// Moves the caret to the character boundary nearest `x` px from the
    /// text's left edge (as tapped inside the field).
    pub fn tap(&mut self, ui: &Ui, x: f32, t: f64) {
        self.focused = true;
        self.blink_from = t;
        self.anchor = None;
        self.caret = self.index_at(ui, x);
        self.caret_px = self.x_of(ui, self.caret);
    }

    /// Whether the caret is in its visible half of the blink at `t`.
    pub fn caret_on(&self, t: f64) -> bool {
        (((t - self.blink_from) / BLINK_MS).floor() as i64) % 2 == 0
    }

    /// When the caret next toggles, for frame scheduling; None when unfocused.
    pub fn next_blink(&self, t: f64) -> Option<f64> {
        self.focused.then(|| self.blink_from + (((t - self.blink_from) / BLINK_MS).floor() + 1.0) * BLINK_MS)
    }

    /// When the field next needs `tick`: the caret blink or a pending long press.
    pub fn next_wake(&self, t: f64) -> Option<f64> {
        let press = self.press.filter(|p| !p.moved && !p.long && p.handle.is_none()).map(|p| p.t + crate::shell::accessibility::long_press_ms(LONG_PRESS_MS));
        match (self.next_blink(t), press) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }

    fn caret_w(&self, ui: &Ui) -> f32 {
        ui.pt(2.0)
    }

    fn line_h(&self, ui: &Ui) -> f32 {
        ui.pt(self.size_pt * 1.25)
    }

    /// How far the text is scrolled left so the caret stays inside `w`.
    fn scroll(&self, ui: &Ui, w: f32) -> f32 {
        (self.caret_px - (w - self.caret_w(ui)).max(1.0)).max(0.0)
    }

    /// The selection's highlight box on screen, if there is a selection.
    fn highlight(&self, ui: &Ui, (x, y, w, h): Frame) -> Option<Frame> {
        let (start, end) = self.selection();
        if start >= end {
            return None;
        }
        let scroll = self.scroll(ui, w);
        let (x0, x1) = (self.x_of(ui, start) - scroll, self.x_of(ui, end) - scroll);
        let lh = self.line_h(ui);
        Some((x + x0, y + h / 2.0 - lh / 2.0, x1 - x0, lh))
    }

    /// The handles' dot centres: start above the left edge, end below the right.
    fn dots(&self, ui: &Ui, frame: Frame) -> Option<[(f32, f32); 2]> {
        let (hx, hy, hw, hh) = self.highlight(ui, frame)?;
        let (bar, dot) = (ui.pt(BAR_PT), ui.pt(DOT_PT));
        Some([(hx - bar / 2.0, hy - dot / 2.0), (hx + hw + bar / 2.0, hy + hh + dot / 2.0)])
    }

    fn handle_at(&self, ui: &Ui, frame: Frame, x: f32, y: f32) -> Option<Handle> {
        let dots = self.dots(ui, frame)?;
        let r = ui.pt(HANDLE_HIT_PT);
        let near = |(cx, cy): (f32, f32)| (x - cx).hypot(y - cy) <= r;
        // the nearer dot wins when both are in reach
        let d = |(cx, cy): (f32, f32)| (x - cx).hypot(y - cy);
        match (near(dots[0]), near(dots[1])) {
            (true, true) => Some(if d(dots[0]) <= d(dots[1]) { Handle::Start } else { Handle::End }),
            (true, false) => Some(Handle::Start),
            (false, true) => Some(Handle::End),
            _ => None,
        }
    }

    /// Drags one end of the selection to the boundary under `x`, keeping at
    /// least one char selected and the ends from crossing.
    fn drag_handle(&mut self, ui: &Ui, (fx, _, fw, _): Frame, handle: Handle, x: f32) {
        let (start, end) = self.selection();
        let i = self.index_at(ui, x - fx + self.scroll(ui, fw));
        let (start, end) = match handle {
            Handle::Start => (i.min(end - 1), end),
            Handle::End => (start, i.max(start + 1)),
        };
        self.anchor = Some(start);
        self.caret = end;
        self.caret_px = self.x_of(ui, self.caret);
    }

    fn show_menu(&mut self, ui: &Ui, frame: Frame, screen_w: f32) {
        let actions = edit::actions(self, self.secure);
        if actions.is_empty() {
            self.menu = None;
            return;
        }
        let dot = ui.pt(DOT_PT);
        let anchor = match self.highlight(ui, frame) {
            Some((x, y, w, h)) => (x, y - dot, w, h + 2.0 * dot),
            None => {
                let (fx, fy, fw, fh) = frame;
                let lh = self.line_h(ui);
                (fx + self.caret_px - self.scroll(ui, fw), fy + fh / 2.0 - lh / 2.0, self.caret_w(ui), lh)
            }
        };
        self.menu = Some(EditMenu::new(ui, &actions, anchor, screen_w, ui.pt(STATUS_PT)));
    }

    /// A touch on the screen while the field sits at `frame` (the box given
    /// to `draw`) on a screen `screen_w` wide. True when the field took it:
    /// a press inside it, on a handle or on the menu. A press anywhere else
    /// drops the selection and menu and is left to the caller.
    pub fn touch(&mut self, ui: &Ui, ev: Event, frame: Frame, screen_w: f32, t: f64) -> bool {
        let (fx, fy, fw, fh) = frame;
        match ev {
            Event::Down(x, y) => {
                if let Some(m) = &mut self.menu {
                    if m.contains(ui, x, y) {
                        m.press(x, y);
                        self.menu_touch = true;
                        return true;
                    }
                }
                self.menu = None;
                let handle = self.handle_at(ui, frame, x, y);
                if handle.is_none() && !(x >= fx && x < fx + fw && y >= fy && y < fy + fh) {
                    self.anchor = None;
                    return false;
                }
                let origin = fx - self.scroll(ui, fw);
                self.press = Some(Press { x, y, t, moved: false, long: false, handle, origin });
                true
            }
            Event::Move(x, y) => {
                if self.menu_touch {
                    return true;
                }
                let Some(mut p) = self.press else { return false };
                if let Some(h) = p.handle {
                    self.drag_handle(ui, frame, h, x);
                    return true;
                }
                if !p.moved && (x - p.x).hypot(y - p.y) > ui.pt(SLOP_PT) {
                    p.moved = true;
                }
                if p.long {
                    // after a long press the caret follows the finger
                    self.tap(ui, x - fx + self.scroll(ui, fw), t);
                }
                self.press = Some(p);
                true
            }
            Event::Up(x, y) => {
                if self.menu_touch {
                    self.menu_touch = false;
                    if let Some(action) = self.menu.as_mut().and_then(|m| m.release(x, y)) {
                        self.menu = None;
                        self.act(ui, action, frame, screen_w, t);
                    }
                    return true;
                }
                let Some(p) = self.press.take() else { return false };
                if p.handle.is_some() || p.long {
                    self.show_menu(ui, frame, screen_w);
                    return true;
                }
                if p.moved {
                    return true;
                }
                let x_rel = x - fx + self.scroll(ui, fw);
                let double = self.last_tap.is_some_and(|(lx, lt)| t - lt <= DOUBLE_TAP_MS && (x - lx).abs() <= ui.pt(DOUBLE_TAP_PT));
                let triple = self.word_tap.is_some_and(|(lx, lt)| t - lt <= DOUBLE_TAP_MS && (x - lx).abs() <= ui.pt(DOUBLE_TAP_PT));
                if triple && !self.secure && !self.is_empty() {
                    self.word_tap = None;
                    self.focused = true;
                    self.select(ui, (0, self.len()));
                    self.show_menu(ui, frame, screen_w);
                } else if double && !self.secure && !self.is_empty() {
                    self.last_tap = None;
                    self.word_tap = Some((x, t));
                    self.focused = true;
                    self.select(ui, edit::word_at(&self.text, self.char_at(ui, x_rel)));
                    self.show_menu(ui, frame, screen_w);
                } else {
                    self.tap(ui, x_rel, t);
                    self.last_tap = Some((x, t));
                }
                true
            }
            Event::Power | Event::Wake | Event::VolumeUp | Event::VolumeDown | Event::Screenshot => false,
        }
    }

    /// Runs a menu action; Select and Select All leave the menu up for the
    /// new selection.
    fn act(&mut self, ui: &Ui, action: Action, frame: Frame, screen_w: f32, t: f64) {
        self.blink_from = t;
        edit::apply(action, self, ui);
        if matches!(action, Action::Select | Action::SelectAll) {
            self.show_menu(ui, frame, screen_w);
        }
    }

    /// Fires a pending long press at `t`: the caret moves under the finger
    /// and the field takes focus; the menu follows on release. True when
    /// the screen changed.
    pub fn tick(&mut self, ui: &Ui, t: f64) -> bool {
        let Some(mut p) = self.press else { return false };
        if p.moved || p.long || p.handle.is_some() || t - p.t < crate::shell::accessibility::long_press_ms(LONG_PRESS_MS) {
            return false;
        }
        p.long = true;
        self.press = Some(p);
        self.tap(ui, p.x - p.origin, t);
        true
    }

    /// The field's selection, or `(caret, caret)`.
    pub fn selection(&self) -> (usize, usize) {
        EditTarget::selection(self)
    }

    /// Draws text (or placeholder), the selection with its handles, the
    /// caret and the editing menu, vertically centred in `frame`. The text
    /// scrolls so the caret stays inside the width.
    pub fn draw(&self, s: &mut Surface, ui: &Ui, frame: Frame, t: f64) {
        let (x, y, w, h) = frame;
        let mid = y + h / 2.0;
        let caret_w = self.caret_w(ui);
        let avail = (w - caret_w).max(1.0);
        let scroll = self.scroll(ui, w);
        let mut caret_x = x + self.caret_px - scroll;
        match &self.rendered {
            Some(img) => {
                let visible = img.crop(scroll as usize, 0, (avail.ceil() as usize).min(img.w.saturating_sub(scroll as usize)), img.h);
                // rendered white, drawn in the label colour so it follows Light and Dark (NativeTextField labelColor)
                s.blit_colored(&visible, x as i32, (mid - img.h as f32 / 2.0) as i32, super::ui::LABEL.now(), 255);
            }
            None => {
                s.blit_colored(&self.placeholder, x as i32, (mid - self.placeholder.h as f32 / 2.0) as i32, super::ui::LABEL_2.now(), 255);
                caret_x = x;
            }
        }
        // composing text is underlined across the marked range, as an input method marks it
        if let (Some((a, b)), Some(_)) = (self.marked, &self.rendered) {
            let (x0, x1) = ((x + self.x_of(ui, a) - scroll).max(x), (x + self.x_of(ui, b) - scroll).min(x + w));
            if x1 > x0 {
                s.fill_rrect(x0, mid + self.line_h(ui) / 2.0 - ui.pt(2.0), x1 - x0, ui.pt(1.0), 0.0, super::ui::LABEL.now());
            }
        }
        if let Some((hx, hy, hw, hh)) = self.highlight(ui, frame) {
            let (hx0, hx1) = (hx.max(x), (hx + hw).min(x + w));
            if hx1 > hx0 {
                s.fill_rrect(hx0, hy, hx1 - hx0, hh, 0.0, rgba(70, 147, 255, HIGHLIGHT_ALPHA));
            }
            let (bar, dot) = (ui.pt(BAR_PT), ui.pt(DOT_PT));
            let [start, end] = self.dots(ui, frame).unwrap_or_default();
            s.fill_rrect(hx - bar, hy - dot / 2.0, bar, hh + dot / 2.0, 0.0, SELECT);
            s.fill_rrect(hx + hw, hy, bar, hh + dot / 2.0, 0.0, SELECT);
            for (cx, cy) in [start, end] {
                s.fill_rrect(cx - dot / 2.0, cy - dot / 2.0, dot, dot, dot / 2.0, SELECT);
            }
        } else if self.focused && self.caret_on(t) {
            let ch = self.line_h(ui);
            s.fill_rrect(caret_x, mid - ch / 2.0, caret_w, ch, caret_w / 2.0, BLUE);
        }
        if self.press.is_some_and(|p| p.handle.is_some() || (p.long && p.moved)) && !self.secure {
            let shown: Vec<char> = self.shown().chars().collect();
            let (a, b) = (self.caret.saturating_sub(8), (self.caret + 8).min(shown.len()));
            let pre: String = shown[a..self.caret.min(shown.len())].iter().collect();
            let post: String = shown[self.caret.min(shown.len())..b].iter().collect();
            draw_loupe(s, ui, caret_x, y - ui.pt(LOUPE_GAP_PT), &pre, &post, self.size_pt);
        }
        if let Some(m) = &self.menu {
            m.draw(s, ui);
        }
    }
}

/// The magnifier's size and its distance above the line being edited.
const LOUPE_W_PT: f32 = 96.0;
const LOUPE_H_PT: f32 = 40.0;
const LOUPE_GAP_PT: f32 = 44.0;
/// How much larger the loupe shows the text around the caret.
const LOUPE_ZOOM: f32 = 1.4;

/// The bifrost loupe: a capsule above the finger showing the text around the
/// caret enlarged, with the caret at its centre. `cx` is the caret's px x,
/// `top` the capsule's top; `pre`/`post` are the text before and after it.
pub fn draw_loupe(s: &mut Surface, ui: &Ui, cx: f32, top: f32, pre: &str, post: &str, pt: f32) {
    let (w, h) = (ui.pt(LOUPE_W_PT), ui.pt(LOUPE_H_PT));
    let x = (cx - w / 2.0).clamp(0.0, (s.w as f32 - w).max(0.0));
    let y = top.max(0.0);
    let hair = ui.pt(1.0);
    s.fill_rrect(x - hair, y - hair, w + 2.0 * hair, h + 2.0 * hair, h / 2.0, super::ui::SEPARATOR);
    s.fill_rrect(x, y, w, h, h / 2.0, super::ui::BLACK);
    let mid = x + w / 2.0;
    let big = pt * LOUPE_ZOOM;
    if !pre.is_empty() {
        let img = ui.text(pre, big, Weight::Regular, WHITE);
        let sx = img.w.saturating_sub((w / 2.0) as usize);
        let vis = img.crop(sx, 0, img.w - sx, img.h);
        s.blit(&vis, (mid - vis.w as f32) as i32, (y + (h - img.h as f32) / 2.0) as i32, 255, 255);
    }
    if !post.is_empty() {
        let img = ui.text(post, big, Weight::Regular, WHITE);
        let vis = img.crop(0, 0, img.w.min((w / 2.0) as usize), img.h);
        s.blit(&vis, mid as i32, (y + (h - img.h as f32) / 2.0) as i32, 255, 255);
    }
    let ch = h * 0.7;
    s.fill_rrect(mid - ui.pt(1.0), y + (h - ch) / 2.0, ui.pt(2.0), ch, ui.pt(1.0), SELECT);
}

impl EditTarget for TextField {
    fn text(&self) -> &str {
        &self.text
    }

    fn selection(&self) -> (usize, usize) {
        match self.anchor {
            Some(a) if a != self.caret => (a.min(self.caret), a.max(self.caret)),
            _ => (self.caret, self.caret),
        }
    }

    fn select(&mut self, ui: &Ui, (a, b): (usize, usize)) {
        let n = self.len();
        let (start, end) = (a.min(b).min(n), a.max(b).min(n));
        self.anchor = (start < end).then_some(start);
        self.caret = end;
        self.caret_px = self.x_of(ui, self.caret);
    }

    fn replace(&mut self, ui: &Ui, (a, b): (usize, usize), with: &str) {
        let n = self.len();
        let (start, end) = (a.min(b).min(n), a.max(b).min(n));
        let room = (self.max + (end - start)).saturating_sub(n);
        // a single-line field has no line breaks or tabs: they become spaces
        let ins: String = with.replace("\r\n", " ").chars().map(|c| if matches!(c, '\n' | '\r' | '\t') { ' ' } else { c }).take(room).collect();
        let (ba, bb) = (self.byte_at(start), self.byte_at(end));
        self.text.replace_range(ba..bb, &ins);
        self.caret = start + ins.chars().count();
        self.anchor = None;
        self.render(ui);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shell::clipboard;

    fn ui() -> Ui {
        crate::shell::tests::ui()
    }

    fn field(ui: &Ui, secure: bool) -> TextField {
        TextField::new(ui, "Password", 17.0, secure, 10)
    }

    #[test]
    fn inserts_at_the_caret_and_arrows_move_it() {
        let ui = ui();
        let mut f = field(&ui, false);
        assert!(!f.edit(&ui, &Out::Text("ac".into()), 0.0));
        f.edit(&ui, &Out::Named("Left"), 1.0);
        f.edit(&ui, &Out::Text("b".into()), 2.0);
        assert_eq!(f.text(), "abc");
        assert_eq!(f.caret(), 2);
        f.edit(&ui, &Out::Named("Right"), 3.0);
        f.edit(&ui, &Out::Named("Right"), 3.0);
        assert_eq!(f.caret(), 3, "clamped at the end");
        f.edit(&ui, &Out::Backspace, 4.0);
        assert_eq!(f.text(), "ab");
        f.edit(&ui, &Out::Named("Left"), 5.0);
        f.edit(&ui, &Out::Named("Left"), 5.0);
        f.edit(&ui, &Out::Named("Left"), 5.0);
        assert_eq!(f.caret(), 0);
        f.edit(&ui, &Out::Backspace, 6.0);
        assert_eq!(f.text(), "ab", "backspace at the start is a no-op");
        assert!(f.edit(&ui, &Out::Enter, 7.0), "return submits");
        f.edit(&ui, &Out::Named("Tab"), 8.0);
        f.edit(&ui, &Out::Hide, 8.0);
        assert_eq!(f.text(), "ab");
    }

    #[test]
    fn composing_text_is_marked_then_kept_or_removed() {
        let ui = ui();
        let mut f = field(&ui, false);
        f.edit(&ui, &Out::Text("ab".into()), 0.0);
        // "にほ" is 6 bytes; a caret at byte 3 sits after "に" (one char)
        f.set_marked(&ui, "にほ", 3);
        assert_eq!(f.text(), "abにほ");
        assert_eq!(f.marked_range(), Some((2, 4)));
        assert_eq!(f.caret(), 3);
        // the next composition replaces the marked range, not the text before it
        f.set_marked(&ui, "にほん", 9);
        assert_eq!(f.text(), "abにほん");
        assert_eq!(f.marked_range(), Some((2, 5)));
        assert_eq!(f.caret(), 5, "caret at the end when sel_start is the end");
        f.unmark(&ui, true);
        assert_eq!(f.marked_range(), None);
        assert_eq!(f.text(), "abにほん", "a committed composition stays");
        assert_eq!(f.caret(), 5);
    }

    #[test]
    fn a_cancelled_composition_is_removed_and_the_caret_falls_back() {
        let ui = ui();
        let mut f = field(&ui, false);
        f.edit(&ui, &Out::Text("ab".into()), 0.0);
        // sel_start 1 is after "x", so the caret is one char into the marked text
        f.set_marked(&ui, "xy", 1);
        assert_eq!(f.text(), "abxy");
        assert_eq!(f.marked_range(), Some((2, 4)));
        assert_eq!(f.caret(), 3);
        f.unmark(&ui, false);
        assert_eq!(f.text(), "ab");
        assert_eq!(f.marked_range(), None);
        assert_eq!(f.caret(), 2, "the caret goes back to where the composition began");
    }

    #[test]
    fn a_typed_key_ends_the_composition_and_composing_is_cut_at_the_limit() {
        let ui = ui();
        let mut f = field(&ui, false);
        f.set_marked(&ui, "abc", 3);
        assert_eq!(f.marked_range(), Some((0, 3)));
        f.edit(&ui, &Out::Text("d".into()), 1.0);
        assert_eq!(f.marked_range(), None, "the next key commits what was marked");
        assert_eq!(f.text(), "abcd");
        // the field holds 10 chars at most
        f.set_marked(&ui, "zzzzzzzzzzzzzz", 14);
        assert_eq!(f.text(), "abcdzzzzzz");
        assert_eq!(f.marked_range(), Some((4, 10)));
    }

    #[test]
    fn respects_the_limit_and_multibyte_text() {
        let ui = ui();
        let mut f = field(&ui, false);
        f.edit(&ui, &Out::Text("éðþæö".into()), 0.0);
        f.edit(&ui, &Out::Named("Left"), 0.0);
        f.edit(&ui, &Out::Backspace, 0.0);
        assert_eq!(f.text(), "éðþö");
        f.edit(&ui, &Out::Text("x".repeat(20)), 0.0);
        assert_eq!(f.len(), 10);
        f.set_text(&ui, "hello");
        assert_eq!((f.text(), f.caret()), ("hello", 5));
    }

    #[test]
    fn hardware_shortcuts_select_all_cut_paste_and_undo() {
        let ui = ui();
        let mut f = field(&ui, false);
        f.edit(&ui, &Out::Text("hello".into()), 0.0);
        f.edit(&ui, &Out::Named("SelectAll"), 1.0);
        f.edit(&ui, &Out::Named("Cut"), 2.0);
        assert_eq!(f.text(), "");
        f.edit(&ui, &Out::Named("Paste"), 3.0);
        assert_eq!(f.text(), "hello");
        f.edit(&ui, &Out::Named("Undo"), 4.0);
        assert_eq!(f.text(), "", "undo reverts the paste");
        f.edit(&ui, &Out::Text("ab".into()), 5.0);
        f.edit(&ui, &Out::Text("c".into()), 5.0);
        f.edit(&ui, &Out::Named("Undo"), 6.0);
        assert_eq!(f.text(), "", "a run of typing is one undo step");
    }

    #[test]
    fn redo_replays_what_undo_took_away() {
        let ui = ui();
        let mut f = field(&ui, false);
        f.edit(&ui, &Out::Text("hello".into()), 0.0);
        f.edit(&ui, &Out::Named("Undo"), 1.0);
        assert_eq!(f.text(), "");
        f.edit(&ui, &Out::Named("Redo"), 2.0);
        assert_eq!((f.text(), f.caret()), ("hello", 5));
        f.edit(&ui, &Out::Named("Undo"), 3.0);
        f.edit(&ui, &Out::Text("x".into()), 4.0);
        f.edit(&ui, &Out::Named("Redo"), 5.0);
        assert_eq!(f.text(), "x", "a new edit drops the redo step");
    }

    #[test]
    fn backspace_and_arrows_step_over_whole_graphemes() {
        let ui = ui();
        let mut f = field(&ui, false);
        f.edit(&ui, &Out::Text("a\u{1F1EE}\u{1F1F8}\u{1F44D}\u{1F3FD}".into()), 0.0);
        f.edit(&ui, &Out::Backspace, 1.0);
        assert_eq!(f.text(), "a\u{1F1EE}\u{1F1F8}", "the skin-toned thumb goes at once");
        f.edit(&ui, &Out::Named("Left"), 2.0);
        assert_eq!(f.caret(), 1, "a flag is one caret step");
        f.edit(&ui, &Out::Named("Right"), 3.0);
        assert_eq!(f.caret(), 3);
        f.edit(&ui, &Out::Backspace, 4.0);
        assert_eq!(f.text(), "a");
    }

    #[test]
    fn pasted_line_breaks_and_tabs_become_spaces() {
        let ui = ui();
        let mut f = field(&ui, false);
        f.replace(&ui, (0, 0), "a\r\nb\nc\td");
        assert_eq!(f.text(), "a b c d");
    }

    #[test]
    fn selected_text_follows_the_caret_in_the_shell_field() {
        let ui = ui();
        let mut f = field(&ui, false);
        f.set_text(&ui, "hello");
        f.select(&ui, (1, 4));
        let a = f.ax_text(7);
        assert_eq!((a.id, a.caret, a.selected_range(), a.len), (7, 4, (1, 4), 5));
        assert_eq!(a.selected_text(f.text()), "ell");
    }

    #[test]
    fn password_fields_never_expose_text() {
        let ui = ui();
        let mut f = field(&ui, true);
        f.set_text(&ui, "abc");
        f.select(&ui, (0, 3));
        let a = f.ax_text(1);
        assert!(a.secure);
        assert_eq!((a.len, a.selected_text(f.text())), (3, String::new()));
    }

    #[test]
    fn secure_mode_draws_bullets_and_caret_follows() {
        let ui = ui();
        let mut f = field(&ui, true);
        f.edit(&ui, &Out::Text("abc".into()), 0.0);
        assert_eq!(f.shown(), "•••");
        let full = f.caret_px;
        f.edit(&ui, &Out::Named("Left"), 0.0);
        assert!(f.caret_px < full && f.caret_px > 0.0);
        let mut buf = vec![0u32; 300 * 60];
        let mut s = Surface { w: 300, h: 60, stride: 300, px: &mut buf, y0: 0 };
        f.draw(&mut s, &ui, (10.0, 0.0, 280.0, 60.0), 0.0);
        assert!(buf.contains(&BLUE), "caret visible right after an edit");
        assert!(buf.contains(&WHITE), "bullets drawn");
    }

    #[test]
    fn caret_blinks_on_a_half_second_clock() {
        let ui = ui();
        let mut f = field(&ui, false);
        f.edit(&ui, &Out::Text("a".into()), 1000.0);
        assert!(f.caret_on(1000.0) && f.caret_on(1499.0));
        assert!(!f.caret_on(1500.0));
        assert!(f.caret_on(2000.0));
        assert_eq!(f.next_blink(1200.0), Some(1500.0));
        f.focused = false;
        assert_eq!(f.next_blink(1200.0), None);
    }

    #[test]
    fn tap_places_the_caret_at_the_nearest_boundary() {
        let ui = ui();
        let mut f = field(&ui, false);
        f.edit(&ui, &Out::Text("hello".into()), 0.0);
        let two = ui.measure("he", 17.0, Weight::Regular);
        f.tap(&ui, two + 1.0, 0.0);
        assert_eq!(f.caret(), 2);
        f.tap(&ui, -5.0, 0.0);
        assert_eq!(f.caret(), 0);
        f.tap(&ui, 9999.0, 0.0);
        assert_eq!(f.caret(), 5);
        f.edit(&ui, &Out::Text("X".into()), 0.0);
        assert_eq!(f.text(), "helloX");
    }

    #[test]
    fn empty_field_shows_the_placeholder() {
        let ui = ui();
        let f = field(&ui, false);
        let mut buf = vec![0u32; 300 * 60];
        let mut s = Surface { w: 300, h: 60, stride: 300, px: &mut buf, y0: 0 };
        f.draw(&mut s, &ui, (10.0, 0.0, 280.0, 60.0), 0.0);
        assert!(buf.contains(&crate::shell::ui::LABEL_2.now()));
    }

    const FRAME: Frame = (100.0, 500.0, 600.0, 70.0);
    const W: f32 = 1200.0;

    /// A field with room for a sentence.
    fn wide(ui: &Ui) -> TextField {
        TextField::new(ui, "Name", 17.0, false, 40)
    }

    /// The hardware keyboard's movement and delete keys in a shell field: Home and End, words, Shift to extend, the word and line
    /// deletes and the forward delete.
    #[test]
    fn home_end_word_keys_shift_selection_and_the_word_deletes_edit_a_shell_field() {
        let ui = ui();
        let mut f = wide(&ui);
        let key = |f: &mut TextField, name: &'static str| {
            f.edit(&ui, &Out::Named(name), 0.0);
        };
        f.set_text(&ui, "hello big world");
        assert_eq!(f.caret(), 15);
        key(&mut f, "Home");
        assert_eq!((f.caret(), f.selection()), (0, (0, 0)));
        key(&mut f, "End");
        assert_eq!(f.caret(), 15);
        key(&mut f, "WordLeft");
        assert_eq!(f.caret(), 10, "Option+Left goes to the start of the word before the caret");
        key(&mut f, "WordLeft");
        assert_eq!(f.caret(), 6);
        key(&mut f, "WordRight");
        assert_eq!(f.caret(), 9, "Option+Right goes to the end of the next word");
        // Shift extends from where the caret was, whichever key moves it
        key(&mut f, "ShiftWordRight");
        assert_eq!(f.selection(), (9, 15));
        key(&mut f, "ShiftHome");
        assert_eq!(f.selection(), (0, 9), "the anchor stays at 9 and the caret goes to the start");
        key(&mut f, "ShiftRight");
        assert_eq!(f.selection(), (1, 9));
        key(&mut f, "ShiftLeft");
        key(&mut f, "ShiftLeft");
        assert_eq!(f.selection(), (0, 9), "stopped at the start");
        key(&mut f, "ShiftEnd");
        assert_eq!(f.selection(), (9, 15));
        // a plain move ends the selection
        key(&mut f, "Home");
        assert_eq!(f.selection(), (0, 0));
        // deleting: the selection first, else a character, a word or the line before the caret
        f.set_text(&ui, "hello big world");
        key(&mut f, "DeleteWord");
        assert_eq!(f.text(), "hello big ", "Option+Backspace takes the last word");
        key(&mut f, "DeleteLine");
        assert_eq!((f.text(), f.caret()), ("", 0), "Cmd+Backspace takes everything before the caret");
        f.set_text(&ui, "hello big world");
        key(&mut f, "Home");
        key(&mut f, "DeleteForward");
        assert_eq!(f.text(), "ello big world", "Delete takes the character after the caret");
        key(&mut f, "DeleteWordForward");
        assert_eq!(f.text(), " big world", "Option+Delete takes the word after the caret");
        key(&mut f, "End");
        key(&mut f, "DeleteForward");
        assert_eq!(f.text(), " big world", "nothing after the caret");
        f.select(&ui, (1, 4));
        key(&mut f, "DeleteWord");
        assert_eq!(f.text(), "  world", "with a selection every delete key deletes the selection");
        // the delete is one undo step
        f.edit(&ui, &Out::Named("Undo"), 1.0);
        assert_eq!(f.text(), " big world");
    }

    #[test]
    fn a_secure_field_moves_by_word_to_its_ends_and_never_shows_its_word_breaks() {
        let ui = ui();
        let mut f = TextField::new(&ui, "Password", 17.0, true, 40);
        f.set_text(&ui, "two words");
        f.edit(&ui, &Out::Named("WordLeft"), 0.0);
        assert_eq!(f.caret(), 0, "the whole password is one word");
        f.edit(&ui, &Out::Named("WordRight"), 0.0);
        assert_eq!(f.caret(), 9);
    }

    fn tap_at(f: &mut TextField, ui: &Ui, x: f32, t: f64) -> bool {
        let a = f.touch(ui, Event::Down(x, 535.0), FRAME, W, t);
        let b = f.touch(ui, Event::Up(x, 535.0), FRAME, W, t + 40.0);
        a && b
    }

    fn long_press(f: &mut TextField, ui: &Ui, x: f32, t: f64) {
        assert!(f.touch(ui, Event::Down(x, 535.0), FRAME, W, t));
        let blink = f.next_blink(t).unwrap_or(f64::MAX);
        assert_eq!(f.next_wake(t), Some((t + LONG_PRESS_MS).min(blink)));
        assert!(!f.tick(ui, t + LONG_PRESS_MS - 1.0));
        assert!(f.tick(ui, t + LONG_PRESS_MS));
        assert!(f.touch(ui, Event::Up(x, 535.0), FRAME, W, t + LONG_PRESS_MS + 50.0));
    }

    fn x_at(f: &TextField, ui: &Ui, i: usize) -> f32 {
        FRAME.0 + f.x_of(ui, i)
    }

    #[test]
    fn selection_edits_replace_delete_and_collapse() {
        let ui = ui();
        let mut f = wide(&ui);
        f.set_text(&ui, "hello wörld");
        f.select(&ui, (0, 5));
        assert_eq!(f.selection(), (0, 5));
        f.edit(&ui, &Out::Named("Right"), 0.0);
        assert_eq!((f.selection(), f.caret()), ((5, 5), 5), "right collapses to the end");
        f.select(&ui, (8, 6));
        f.edit(&ui, &Out::Named("Left"), 0.0);
        assert_eq!(f.caret(), 6, "ends are ordered; left collapses to the start");
        f.select(&ui, (0, 5));
        f.edit(&ui, &Out::Text("Y".into()), 0.0);
        assert_eq!((f.text(), f.caret()), ("Y wörld", 1));
        f.select(&ui, (2, 7));
        f.edit(&ui, &Out::Backspace, 0.0);
        assert_eq!((f.text(), f.selection()), ("Y ", (2, 2)));
        f.select(&ui, (0, 99));
        assert_eq!(f.selection(), (0, 2), "clamped to the text");
        f.replace(&ui, (0, 2), "x".repeat(50).as_str());
        assert_eq!(f.len(), 40, "the limit still holds after a replace");
    }

    #[test]
    fn double_tap_selects_a_word_and_shows_cut_copy() {
        let _g = clipboard::tests::exclusive();
        let ui = ui();
        let mut f = wide(&ui);
        f.set_text(&ui, "góðan dag");
        let x = x_at(&f, &ui, 2);
        assert!(tap_at(&mut f, &ui, x, 1000.0));
        assert_eq!((f.selection(), f.caret()), ((2, 2), 2));
        assert!(f.menu().is_none());
        assert!(tap_at(&mut f, &ui, x + 3.0, 1200.0));
        assert_eq!(f.selection(), (0, 5), "the Icelandic word");
        let menu = f.menu().expect("menu over the selection");
        let (_, my, _, mh) = menu.rect();
        assert!(my + mh < FRAME.1, "above the field");
        assert_eq!(menu.hit(menu.rect().0 + 5.0, my + 5.0), Some(Action::Cut));
        assert!(!menu.contains(&ui, 0.0, 0.0));
        // Copy from the menu, then paste at the end over nothing
        let (mx, my, mw, mh) = menu.rect();
        let copy_x = mx + mw * 0.5;
        assert!(f.touch(&ui, Event::Down(copy_x, my + mh / 2.0), FRAME, W, 2000.0));
        assert!(f.touch(&ui, Event::Up(copy_x, my + mh / 2.0), FRAME, W, 2040.0));
        assert_eq!(clipboard::text().as_deref(), Some("góðan"));
        assert!(f.menu().is_none());
        assert_eq!(f.selection(), (0, 5), "copy keeps the selection");
        // a tap elsewhere drops the selection
        let x9 = x_at(&f, &ui, 9);
        assert!(tap_at(&mut f, &ui, x9, 3000.0));
        assert_eq!(f.selection(), (9, 9));
        // double tap on a space selects just the space
        let sx = x_at(&f, &ui, 5) + 2.0;
        tap_at(&mut f, &ui, sx, 4000.0);
        tap_at(&mut f, &ui, sx, 4100.0);
        assert_eq!(f.selection(), (5, 6));
        // two taps too far apart in time are two single taps
        tap_at(&mut f, &ui, x, 6000.0);
        tap_at(&mut f, &ui, x, 7000.0);
        assert_eq!(f.selection(), (2, 2));
    }

    #[test]
    fn long_press_places_the_caret_and_offers_select_and_paste() {
        let _g = clipboard::tests::exclusive();
        let ui = ui();
        let mut f = wide(&ui);
        f.set_text(&ui, "hello");
        f.focused = false;
        let x = x_at(&f, &ui, 3);
        long_press(&mut f, &ui, x, 1000.0);
        assert!(f.focused, "a long press focuses the field");
        assert_eq!(f.caret(), 3);
        let menu = f.menu().expect("caret menu");
        let (mx, my, mw, mh) = menu.rect();
        assert_eq!(menu.hit(mx + 5.0, my + 5.0), Some(Action::Select));
        assert_eq!(menu.hit(mx + mw - 5.0, my + 5.0), Some(Action::SelectAll), "no Paste on an empty clipboard");
        // Select picks the word and keeps a menu up
        assert!(f.touch(&ui, Event::Down(mx + 5.0, my + mh / 2.0), FRAME, W, 2000.0));
        assert!(f.touch(&ui, Event::Up(mx + 5.0, my + mh / 2.0), FRAME, W, 2040.0));
        assert_eq!(f.selection(), (0, 5));
        assert_eq!(f.menu().and_then(|m| m.hit(m.rect().0 + 5.0, m.rect().1 + 5.0)), Some(Action::Cut));
        // Cut empties the field and fills the clipboard; the next long press offers Paste
        let (mx, my, _, mh) = f.menu().unwrap().rect();
        f.touch(&ui, Event::Down(mx + 5.0, my + mh / 2.0), FRAME, W, 3000.0);
        f.touch(&ui, Event::Up(mx + 5.0, my + mh / 2.0), FRAME, W, 3040.0);
        assert_eq!((f.text(), clipboard::text().as_deref()), ("", Some("hello")));
        long_press(&mut f, &ui, x, 4000.0);
        let menu = f.menu().unwrap();
        let (mx, my, mw, mh) = menu.rect();
        assert_eq!(menu.hit(mx + mw - 5.0, my + 5.0), Some(Action::Paste));
        assert_eq!(menu.hit(mx + 5.0, my + 5.0), Some(Action::Paste), "nothing to select in an empty field");
        f.touch(&ui, Event::Down(mx + 5.0, my + mh / 2.0), FRAME, W, 5000.0);
        f.touch(&ui, Event::Up(mx + 5.0, my + mh / 2.0), FRAME, W, 5040.0);
        assert_eq!((f.text(), f.caret()), ("hello", 5));
        assert!(f.menu().is_none());
    }

    #[test]
    fn moving_cancels_the_long_press_and_a_press_outside_clears() {
        let ui = ui();
        let mut f = wide(&ui);
        f.set_text(&ui, "hello");
        let x = x_at(&f, &ui, 1);
        assert!(f.touch(&ui, Event::Down(x, 535.0), FRAME, W, 0.0));
        assert!(f.touch(&ui, Event::Move(x + 40.0, 535.0), FRAME, W, 100.0));
        assert!(!f.tick(&ui, 500.0), "moved: no long press");
        assert_eq!(f.next_wake(500.0), f.next_blink(500.0));
        assert!(f.touch(&ui, Event::Up(x + 40.0, 535.0), FRAME, W, 600.0));
        assert!(f.menu().is_none());
        f.select(&ui, (0, 5));
        assert!(!f.touch(&ui, Event::Down(50.0, 50.0), FRAME, W, 1000.0), "outside is not ours");
        assert_eq!(f.selection(), (5, 5), "but it drops the selection");
        assert!(!f.touch(&ui, Event::Up(50.0, 50.0), FRAME, W, 1040.0));
    }

    #[test]
    fn handles_drag_the_selection_ends_without_crossing() {
        let _g = clipboard::tests::exclusive();
        let ui = ui();
        let mut f = wide(&ui);
        f.set_text(&ui, "hello wörld");
        f.select(&ui, (6, 11));
        let [(sx, sy), (_, ey)] = f.dots(&ui, FRAME).unwrap();
        assert!(sy < FRAME.1 + 35.0 && ey > sy, "start dot above, end dot below");
        assert!(f.touch(&ui, Event::Down(sx + 4.0, sy - 4.0), FRAME, W, 0.0), "grabbed the start handle");
        f.touch(&ui, Event::Move(x_at(&f, &ui, 2), sy), FRAME, W, 50.0);
        assert_eq!(f.selection(), (2, 11));
        f.touch(&ui, Event::Move(x_at(&f, &ui, 11) + 50.0, sy), FRAME, W, 80.0);
        assert_eq!(f.selection(), (10, 11), "the start never passes the end");
        assert!(f.touch(&ui, Event::Up(x_at(&f, &ui, 11) + 50.0, sy), FRAME, W, 100.0));
        assert!(f.menu().is_some(), "the menu returns after the drag");
        let [_, (ex, ey)] = f.dots(&ui, FRAME).unwrap();
        assert!(f.touch(&ui, Event::Down(ex, ey + 3.0), FRAME, W, 200.0));
        f.touch(&ui, Event::Move(x_at(&f, &ui, 0) - 30.0, ey), FRAME, W, 250.0);
        assert_eq!(f.selection(), (10, 11), "the end never passes the start");
        f.touch(&ui, Event::Up(x_at(&f, &ui, 0), ey), FRAME, W, 300.0);
        let _ = ex;
        let mut buf = vec![0u32; 1200 * 700];
        f.draw(&mut Surface { w: 1200, h: 700, stride: 1200, px: &mut buf, y0: 0 }, &ui, FRAME, 0.0);
        assert!(buf.contains(&SELECT), "handles drawn");
        assert!(!buf.contains(&BLUE), "no caret while selecting");
        assert!(buf.contains(&super::super::editmenu::BG), "menu drawn");
    }

    #[test]
    fn secure_fields_only_paste() {
        let _g = clipboard::tests::exclusive();
        let ui = ui();
        let mut f = field(&ui, true);
        f.set_text(&ui, "secret");
        let x = x_at(&f, &ui, 2);
        tap_at(&mut f, &ui, x, 0.0);
        tap_at(&mut f, &ui, x, 100.0);
        assert_eq!(f.selection(), (2, 2), "no word selection in a password");
        long_press(&mut f, &ui, x, 1000.0);
        assert!(f.menu().is_none(), "nothing to paste");
        clipboard::set_text("pw");
        long_press(&mut f, &ui, x, 3000.0);
        let menu = f.menu().unwrap();
        let (mx, my, mw, _) = menu.rect();
        assert_eq!(menu.hit(mx + 5.0, my + 5.0), Some(Action::Paste));
        assert_eq!(menu.hit(mx + mw - 5.0, my + 5.0), Some(Action::Paste), "Paste is the only item");
    }

    #[test]
    fn secure_fields_never_copy_or_cut_from_the_keyboard() {
        let _g = clipboard::tests::exclusive();
        let ui = ui();
        let mut f = field(&ui, true);
        f.set_text(&ui, "secret");
        clipboard::set_text("before");
        f.edit(&ui, &Out::Named("SelectAll"), 0.0);
        f.edit(&ui, &Out::Named("Copy"), 0.0);
        assert_eq!(clipboard::text().as_deref(), Some("before"), "Copy left the clipboard alone");
        f.edit(&ui, &Out::Named("Cut"), 0.0);
        assert_eq!(clipboard::text().as_deref(), Some("before"), "Cut left the clipboard alone");
        assert_eq!(f.text(), "secret", "Cut did not remove the text");
        let mut plain = field(&ui, false);
        plain.set_text(&ui, "abc");
        plain.edit(&ui, &Out::Named("SelectAll"), 0.0);
        plain.edit(&ui, &Out::Named("Copy"), 0.0);
        assert_eq!(clipboard::text().as_deref(), Some("abc"), "a plain field still copies");
    }
}
