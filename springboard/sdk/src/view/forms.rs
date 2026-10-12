//! More control views: Checkbox, Stepper, ProgressIndicator, TextField, PopUpButton and the grouped Form / Section.
//! Same rules as `controls.rs`: named colours from render ui.rs, `MAC_*_PT` text sizes, `ui.pt()` for every length.
use super::controls::{draw_text, focus_ring, is_activate, node};
use super::{blit, TextInputInfo, View, ViewBase, ViewEvent};
use crate::kit::clipboard;
use crate::kit::ui::{Weight, CARD_BG, CONTROL_PRESSED, FIELD_BG, HAIRLINE_C, LABEL, LABEL_2, LABEL_3, SELECTION, SEPARATOR_C, TINT, TRACK_C, WHITE};
use crate::kit::{Event, Ui};
use bifrost_render::canvas::{Canvas, RectF};
use crate::layout::ROW_PT;
use crate::proto::{InputTraits, KeyInput, TextState, ToApp, MAX_SURROUNDING};
use crate::widgets::{self, MENU_PAD_PT, MENU_ROW_PT, MENU_SEPARATOR_PT};
use bifrost_render::ui::{MAC_BODY_PT, MAC_FOOTNOTE_PT, MAC_GROUP_HEADER_PT, SPACE_L, SPACE_M, SPACE_S, SPACE_XL, SPACE_XXS};
use bifrost_ui::menu::{Menu, MenuItem};
use bifrost_ui::prefs_layout::m;
use bifrost_ui::keys::{Key, KeyCombo};
use bifrost_ui::text::{binding, EditCommand, TextBuffer, TextEditor};
use bifrost_ui::{values, Command, Node, Rect, Role};
use std::f32::consts::PI;

/// `NativeButton` checkbox box edge, in points.
const CHECKBOX_PT: f32 = 14.0;
const CHECKBOX_RADIUS_PT: f32 = 4.0;
/// Dot diameter of the hand-drawn check mark.
const CHECK_DOT_PT: f32 = 1.8;
/// `NativeStepper` size, in points.
const STEPPER_W_PT: f32 = 15.0;
const STEPPER_H_PT: f32 = 22.0;
/// Bar progress indicator thickness and spinner diameter, in points.
const BAR_H_PT: f32 = 6.0;
const SPINNER_PT: f32 = 16.0;
const SPINNER_DOT_PT: f32 = 2.4;
const SPINNER_SPOKES: usize = 8;
/// Seconds for one sweep of an indeterminate bar and one turn of the spinner.
const BAR_PERIOD_S: f64 = 1.5;
const SPINNER_PERIOD_S: f64 = 0.8;
const FIELD_RADIUS_PT: f32 = 6.0;

pub(super) fn parse_bool(v: &str) -> Option<bool> {
    match v.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "on" | "checked" | "yes" => Some(true),
        "0" | "false" | "off" | "unchecked" | "no" => Some(false),
        _ => None,
    }
}

fn dot(s: &mut dyn Canvas, ui: &Ui, cx: f32, cy: f32, d: f32, color: u32) {
    s.fill_rrect(RectF::new(ui.pt(cx - d / 2.0), ui.pt(cy - d / 2.0), ui.pt(d), ui.pt(d)), ui.pt(d / 2.0), color);
}

/// A triangle of three one-pixel-step rows, apex up or down, centred on `(cx, cy)` (pixels).
fn chevron(s: &mut dyn Canvas, ui: &Ui, cx: f32, cy: f32, up: bool, color: u32) {
    let row = ui.pt(1.0).max(1.0);
    for k in 0..3 {
        let half = (k as f32 + 0.5) * row;
        let y = if up { cy - 1.5 * row + k as f32 * row } else { cy + 1.5 * row - (k as f32 + 1.0) * row };
        s.fill_rrect(RectF::new(cx - half, y, half * 2.0, row), 0.0, color);
    }
}

/// A checkbox (`NativeButton` checkbox style); sends `action` when toggled.
pub struct Checkbox {
    base: ViewBase,
    label: String,
    action: String,
    checked: bool,
    pressed: bool,
}

impl Checkbox {
    pub fn new(label: &str, checked: bool, action: &str) -> Self {
        Checkbox { base: ViewBase::default(), label: label.into(), action: action.into(), checked, pressed: false }
    }
    pub fn checked(&self) -> bool {
        self.checked
    }
    pub fn set_checked(&mut self, on: bool) {
        if self.checked != on {
            self.checked = on;
            self.base.dirty = true;
        }
    }
    fn toggle(&mut self) {
        self.set_checked(!self.checked);
        let a = self.action.clone();
        self.base.send_action(&a);
    }
}

impl View for Checkbox {
    crate::view_base!();
    fn intrinsic(&self, ui: &Ui) -> (f32, f32) {
        (CHECKBOX_PT + SPACE_S + ui.measure(&self.label, MAC_BODY_PT, Weight::Regular) / ui.pt(1.0), ROW_PT)
    }
    fn accepts_first_responder(&self) -> bool {
        true
    }
    fn draw(&mut self, s: &mut dyn Canvas, ui: &Ui) {
        let f = self.base.frame;
        let b = Rect { x: f.x, y: f.y + (f.h - CHECKBOX_PT) / 2.0, w: CHECKBOX_PT, h: CHECKBOX_PT };
        let fill = if self.checked && self.base.enabled { TINT } else if self.pressed { CONTROL_PRESSED.now() } else { FIELD_BG.now() };
        s.fill_rrect(RectF::new(ui.pt(b.x), ui.pt(b.y), ui.pt(b.w), ui.pt(b.h)), ui.pt(CHECKBOX_RADIUS_PT), fill);
        if !self.checked {
            s.stroke_rrect(RectF::new(ui.pt(b.x), ui.pt(b.y), ui.pt(b.w), ui.pt(b.h)), ui.pt(CHECKBOX_RADIUS_PT), ui.pt(1.0).max(1.0), HAIRLINE_C.now());
        } else {
            // check mark: short stroke down-right, long stroke up-right, as a row of dots in box-local points
            let pts = [(3.5, 7.5), (4.5, 8.5), (5.5, 9.5), (6.5, 8.5), (7.5, 7.0), (8.5, 5.5), (9.5, 4.2), (10.5, 3.2)];
            for (px, py) in pts {
                dot(s, ui, b.x + px, b.y + py, CHECK_DOT_PT, WHITE);
            }
        }
        let c = if self.base.enabled { LABEL } else { LABEL_3 };
        let lx = f.x + CHECKBOX_PT + SPACE_S;
        draw_text(s, ui, Rect { x: lx, w: (f.right() - lx).max(0.0), ..f }, &self.label, MAC_BODY_PT, Weight::Regular, c.now(), false);
        if self.base.focused {
            focus_ring(s, ui, b, CHECKBOX_RADIUS_PT);
        }
    }
    fn handle(&mut self, ev: &ViewEvent) -> bool {
        if !self.base.enabled {
            return false;
        }
        match ev {
            ViewEvent::Pointer(Event::Down(..)) => self.pressed = true,
            ViewEvent::Pointer(Event::Move(..)) => {}
            ViewEvent::Pointer(Event::Up(x, y)) => {
                if self.pressed && self.base.frame.contains(*x, *y) {
                    self.toggle();
                }
                self.pressed = false;
            }
            ViewEvent::Cancel => self.pressed = false,
            k if is_activate(k) => self.toggle(),
            _ => return false,
        }
        self.base.dirty = true;
        true
    }
    fn ax_node(&self) -> Option<Node> {
        node(self, Role::Checkbox, &self.label, if self.checked { "checked" } else { "unchecked" })
    }
    fn ax_checked(&self) -> Option<bool> {
        Some(self.checked)
    }
    fn ax_set_value(&mut self, value: &str) -> bool {
        match parse_bool(value) {
            Some(b) => {
                if b != self.checked {
                    self.toggle();
                }
                true
            }
            None => false,
        }
    }
}

/// A numeric stepper (`NativeStepper`) over `values::Stepper`; sends `action` on every change. Up / Down arrow keys nudge.
pub struct Stepper {
    base: ViewBase,
    label: String,
    action: String,
    pub model: values::Stepper,
}

impl Stepper {
    pub fn new(label: &str, model: values::Stepper, action: &str) -> Self {
        Stepper { base: ViewBase::default(), label: label.into(), action: action.into(), model }
    }
    pub fn value(&self) -> f32 {
        self.model.value
    }
    fn changed(&mut self, c: bool) {
        if c {
            self.base.dirty = true;
            let a = self.action.clone();
            self.base.send_action(&a);
        }
    }
}

impl View for Stepper {
    crate::view_base!();
    fn intrinsic(&self, _ui: &Ui) -> (f32, f32) {
        (STEPPER_W_PT, STEPPER_H_PT)
    }
    fn accepts_first_responder(&self) -> bool {
        true
    }
    fn draw(&mut self, s: &mut dyn Canvas, ui: &Ui) {
        let f = self.base.frame;
        s.fill_rrect(RectF::new(ui.pt(f.x), ui.pt(f.y), ui.pt(f.w), ui.pt(f.h)), ui.pt(FIELD_RADIUS_PT), FIELD_BG.now());
        s.fill_rect(RectF::new(ui.pt(f.x + SPACE_XXS).trunc(), ui.pt(f.y + f.h / 2.0).trunc(), ui.pt(f.w - 2.0 * SPACE_XXS).max(1.0).trunc(), 1.0), SEPARATOR_C.now());
        let c = if self.base.enabled { LABEL.now() } else { LABEL_3.now() };
        chevron(s, ui, ui.pt(f.x + f.w / 2.0), ui.pt(f.y + f.h / 4.0), true, c);
        chevron(s, ui, ui.pt(f.x + f.w / 2.0), ui.pt(f.y + 3.0 * f.h / 4.0), false, c);
        if self.base.focused {
            focus_ring(s, ui, f, FIELD_RADIUS_PT);
        }
    }
    fn handle(&mut self, ev: &ViewEvent) -> bool {
        if !self.base.enabled {
            return false;
        }
        match ev {
            ViewEvent::Pointer(Event::Down(x, y)) => {
                if let Some(part) = values::Stepper::part_at(self.base.frame, *x, *y) {
                    let c = self.model.press(part);
                    self.changed(c);
                }
            }
            ViewEvent::Pointer(_) | ViewEvent::Cancel => {}
            ViewEvent::Key(KeyInput::Code(c @ (38 | 40))) => {
                let c = self.model.nudge(if *c == 38 { 1 } else { -1 });
                self.changed(c);
            }
            _ => return false,
        }
        true
    }
    fn ax_node(&self) -> Option<Node> {
        node(self, Role::Slider, &self.label, &format!("{}", self.model.value))
    }
    fn ax_set_value(&mut self, value: &str) -> bool {
        match value.trim().parse::<f32>() {
            Ok(v) => {
                let v = v.clamp(self.model.min, self.model.max);
                let c = v != self.model.value;
                self.model.value = v;
                self.changed(c);
                true
            }
            Err(_) => false,
        }
    }
}

/// `NativeProgressIndicator.Style`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProgressStyle {
    Bar,
    Spinning,
}

/// A progress bar or spinner (`NativeProgressIndicator`). An indeterminate bar and the spinner animate from the time given
/// to [`Window::tick`](super::Window::tick) while `animating` (`startAnimation` / `stopAnimation`).
pub struct ProgressIndicator {
    base: ViewBase,
    pub style: ProgressStyle,
    pub model: values::Progress,
    animating: bool,
    t: f64,
}

impl ProgressIndicator {
    pub fn bar(fraction: f32) -> Self {
        ProgressIndicator { base: ViewBase::default(), style: ProgressStyle::Bar, model: values::Progress::determinate(fraction), animating: false, t: 0.0 }
    }
    pub fn indeterminate_bar() -> Self {
        ProgressIndicator { model: values::Progress::indeterminate(), animating: true, ..Self::bar(0.0) }
    }
    pub fn spinner() -> Self {
        ProgressIndicator { style: ProgressStyle::Spinning, model: values::Progress::indeterminate(), animating: true, ..Self::bar(0.0) }
    }
    pub fn set_fraction(&mut self, f: f32) {
        self.model.set(f);
        self.base.dirty = true;
    }
    pub fn start_animation(&mut self) {
        self.animating = true;
    }
    pub fn stop_animation(&mut self) {
        self.animating = false;
        self.base.dirty = true;
    }
    pub fn is_animating(&self) -> bool {
        self.animating
    }
    /// The spinner spoke that is brightest at the current time.
    pub fn head_spoke(&self) -> usize {
        (((self.t / SPINNER_PERIOD_S).fract() * SPINNER_SPOKES as f64) as usize) % SPINNER_SPOKES
    }
}

impl View for ProgressIndicator {
    crate::view_base!();
    fn intrinsic(&self, _ui: &Ui) -> (f32, f32) {
        match self.style {
            ProgressStyle::Bar => (0.0, BAR_H_PT),
            ProgressStyle::Spinning => (SPINNER_PT, SPINNER_PT),
        }
    }
    fn tick(&mut self, t: f64) {
        if self.animating && self.model.fraction().is_none() && (self.t - t).abs() > f64::EPSILON {
            self.t = t;
            self.base.dirty = true;
        }
    }
    fn draw(&mut self, s: &mut dyn Canvas, ui: &Ui) {
        let f = self.base.frame;
        match self.style {
            ProgressStyle::Bar => {
                let track = Rect { x: f.x, y: f.y + (f.h - BAR_H_PT) / 2.0, w: f.w, h: BAR_H_PT };
                s.fill_rrect(RectF::new(ui.pt(track.x), ui.pt(track.y), ui.pt(track.w), ui.pt(track.h)), ui.pt(BAR_H_PT / 2.0), TRACK_C.now());
                let fill = self.model.filled(track, self.t, BAR_PERIOD_S);
                if fill.w > 0.0 {
                    s.fill_rrect(RectF::new(ui.pt(fill.x), ui.pt(fill.y), ui.pt(fill.w), ui.pt(fill.h)), ui.pt(BAR_H_PT / 2.0), TINT);
                }
            }
            ProgressStyle::Spinning => {
                let (cx, cy) = (f.x + f.w / 2.0, f.y + f.h / 2.0);
                let r = (f.w.min(f.h) - SPINNER_DOT_PT) / 2.0;
                let head = self.head_spoke();
                for i in 0..SPINNER_SPOKES {
                    let a = i as f32 * 2.0 * PI / SPINNER_SPOKES as f32 - PI / 2.0;
                    let age = (head + SPINNER_SPOKES - i) % SPINNER_SPOKES;
                    let c = match age {
                        0 => LABEL.now(),
                        1 | 2 => LABEL_2.now(),
                        _ => LABEL_3.now(),
                    };
                    dot(s, ui, cx + r * a.cos(), cy + r * a.sin(), SPINNER_DOT_PT, c);
                }
            }
        }
    }
    fn ax_node(&self) -> Option<Node> {
        node(self, Role::ProgressIndicator, "Progress", &self.model.label())
    }
}

/// Horizontal text inset of a [`TextField`], in points.
const FIELD_INSET_PT: f32 = 12.0;
/// How many edits a field remembers.
const FIELD_UNDO_LEVELS: usize = 100;

/// evdev code of a letter key to the letter (the chords the editing bindings use).
fn evdev_letter(key: u32) -> Option<char> {
    const ROWS: [(u32, &str); 3] = [(16, "qwertyuiop"), (30, "asdfghjkl"), (44, "zxcvbnm")];
    ROWS.iter().find_map(|(start, chars)| key.checked_sub(*start).and_then(|i| chars.chars().nth(i as usize)))
}

/// The `KeyCombo` a chord means, for `text::binding`.
fn chord_combo(key: u32, mods: u32) -> Option<KeyCombo> {
    use crate::proto::{CHORD_ALT, CHORD_CTRL, CHORD_META, CHORD_SHIFT};
    Some(KeyCombo { cmd: mods & CHORD_META != 0, ctrl: mods & CHORD_CTRL != 0, alt: mods & CHORD_ALT != 0, shift: mods & CHORD_SHIFT != 0, key: Key::Char(evdev_letter(key)?) })
}

/// A single-line text field (`NativeTextField` editable) over `bifrost_ui::text::TextEditor`: caret and selection placed by
/// pointer (click, drag), the Golden Gate and emacs editing bindings (`text::binding`), grapheme-wise movement, and the
/// responder actions `cut copy paste selectAll undo redo`. The field keeps its own undo history (like
/// `NativeTextView.undoManager`); the window's undo manager is reached only when the field has nothing to undo. It is an
/// `NativeTextInputClient`: while first responder it reports its text, caret rectangle and `traits` (see
/// `ViewApp::sync_text_input`), draws marked text underlined, and takes `SetMarkedText`, `UnmarkText` and
/// `DeleteSurrounding` from an IME or the on-screen keyboard. Sends `action` after every edit.
pub struct TextField {
    base: ViewBase,
    placeholder: String,
    action: String,
    pub editor: TextEditor,
    /// `NativeTextInputTraits` reported to the shell (return key label, keyboard kind, flags `tiflags::*`)
    pub traits: InputTraits,
    /// set by `draw`: first visible byte, and every character boundary from there to the last visible one with its x in
    /// points from the text origin. Pointer hit-testing and the caret rectangle read it.
    start: usize,
    marks: Vec<(usize, f32)>,
    /// a drag selection is in progress
    dragging: bool,
    /// `accessibilityTitle` and `accessibilityHelp`
    ax_title: String,
    ax_help: String,
}

impl TextField {
    pub fn new(placeholder: &str, action: &str) -> Self {
        TextField {
            base: ViewBase::default(),
            placeholder: placeholder.into(),
            action: action.into(),
            editor: TextEditor::new(FIELD_UNDO_LEVELS),
            traits: InputTraits::default(),
            start: 0,
            marks: Vec::new(),
            dragging: false,
            ax_title: String::new(),
            ax_help: String::new(),
        }
    }
    /// `accessibilityTitle`: the visible title of the field (the placeholder stays the `accessibilityLabel`).
    pub fn with_ax_title(mut self, title: &str) -> Self {
        self.ax_title = title.into();
        self
    }
    /// `accessibilityHelp`: what the field is for, spoken after its label.
    pub fn with_ax_help(mut self, help: &str) -> Self {
        self.ax_help = help.into();
        self
    }
    /// The text model: selection, marked range, surrounding text.
    pub fn model(&self) -> &TextBuffer {
        &self.editor.buf
    }
    pub fn text(&self) -> &str {
        self.editor.buf.text()
    }
    /// `NativeReturnKeyType`, `NativeKeyboardType` and the `tiflags::*` the shell's keyboard should use for this field.
    pub fn with_traits(mut self, traits: InputTraits) -> Self {
        self.traits = traits;
        self
    }
    /// Replaces the contents programmatically (forgets undo history); the caret goes to the end.
    pub fn set_text(&mut self, text: &str) {
        self.editor = TextEditor::new(FIELD_UNDO_LEVELS);
        self.editor.buf.set_text(text);
        self.start = 0;
        self.base.dirty = true;
    }
    fn edited(&mut self, before: &str) {
        self.base.dirty = true;
        if self.text() != before {
            let a = self.action.clone();
            self.base.send_action(&a);
        }
    }
    /// A single-line field takes no line breaks.
    fn single_line(t: &str) -> String {
        t.chars().filter(|c| !matches!(c, '\n' | '\r')).collect()
    }
    /// x in points from the text origin of byte `b` (nearest known boundary at or before it).
    fn x_of(&self, b: usize) -> f32 {
        self.marks.iter().rev().find(|(m, _)| *m <= b).map(|(_, x)| *x).unwrap_or(0.0)
    }
    /// The byte boundary nearest to `x` points from the text origin.
    fn byte_at(&self, x: f32) -> usize {
        let mut best = (self.start.min(self.text().len()), f32::MAX);
        for &(b, mx) in &self.marks {
            let d = (mx - x).abs();
            if d < best.1 {
                best = (b, d);
            }
        }
        best.0
    }
    /// Runs an editing command; true (the key is consumed).
    fn run(&mut self, cmd: EditCommand, extend: bool) -> bool {
        let before = self.text().to_string();
        self.editor.perform(cmd, extend);
        self.edited(&before);
        true
    }
    fn clipboard_command(action: &str) -> Option<Command> {
        Some(match action {
            "cut" => Command::Cut,
            "copy" => Command::Copy,
            "paste" => Command::Paste,
            "selectAll" => Command::SelectAll,
            "undo" => Command::Undo,
            "redo" => Command::Redo,
            _ => return None,
        })
    }
    fn handle_key(&mut self, k: &KeyInput) -> bool {
        if let Some((key, mods)) = k.as_chord() {
            let Some((cmd, ext)) = chord_combo(key, mods).and_then(binding) else { return false };
            return match cmd {
                EditCommand::Undo => self.perform("undo"),
                EditCommand::Redo => self.perform("redo"),
                c => self.run(c, ext),
            };
        }
        match k {
            KeyInput::Text(t) => {
                let before = self.text().to_string();
                let t = Self::single_line(t);
                if !t.is_empty() {
                    self.editor.insert_text(&t);
                }
                self.edited(&before);
                true
            }
            KeyInput::Code(8) => self.run(EditCommand::DeleteBackward, false),
            KeyInput::Code(46) => self.run(EditCommand::DeleteForward, false),
            KeyInput::Code(37) => self.run(EditCommand::MoveLeft, false),
            KeyInput::Code(39) => self.run(EditCommand::MoveRight, false),
            KeyInput::Code(36) => self.run(EditCommand::MoveLineStart, false),
            KeyInput::Code(35) => self.run(EditCommand::MoveLineEnd, false),
            _ => false,
        }
    }
}

impl View for TextField {
    crate::view_base!();
    fn intrinsic(&self, _ui: &Ui) -> (f32, f32) {
        (0.0, ROW_PT)
    }
    fn accepts_first_responder(&self) -> bool {
        true
    }
    fn draw(&mut self, s: &mut dyn Canvas, ui: &Ui) {
        let Rect { x, y, w, h } = self.base.frame;
        let focused = self.base.focused;
        bifrost_ui::ax::record(Role::TextField, &self.placeholder, self.text(), self.base.frame, true);
        s.fill_rrect(RectF::new(ui.pt(x), ui.pt(y), ui.pt(w), ui.pt(h)), ui.pt(FIELD_RADIUS_PT), if focused { TINT } else { FIELD_BG.now() });
        if focused {
            s.fill_rrect(RectF::new(ui.pt(x + 1.), ui.pt(y + 1.), ui.pt(w - 2.), ui.pt(h - 2.)), ui.pt(FIELD_RADIUS_PT - 1.), FIELD_BG.now());
        }
        widgets::outline_if_borders(s, ui, self.base.frame, FIELD_RADIUS_PT);
        let room = ui.pt((w - 2. * FIELD_INSET_PT).max(0.));
        let buf = &self.editor.buf;
        let value = buf.text();
        let caret = buf.caret();
        // scroll: keep the caret visible without cutting a UTF-8 sequence
        let mut start = bifrost_ui::text::floor_boundary(value, self.start.min(value.len()));
        if caret < start {
            start = caret;
        }
        while start < caret && ui.measure(&value[start..caret], MAC_BODY_PT, Weight::Regular) > room {
            start = bifrost_ui::text::next_boundary(value, start);
        }
        // boundaries from `start` while they fit (always up to the caret)
        let mut marks = vec![(start, 0.0f32)];
        let mut b = start;
        while b < value.len() {
            let n = bifrost_ui::text::next_boundary(value, b);
            let px = ui.measure(&value[start..n], MAC_BODY_PT, Weight::Regular);
            if px > room && b >= caret {
                break;
            }
            marks.push((n, px / ui.s));
            b = n;
        }
        let end = b;
        let (left, top) = (ui.pt(x + FIELD_INSET_PT), ui.pt(y + (h - 16.) / 2.));
        let px = |m: &Vec<(usize, f32)>, at: usize| ui.pt(m.iter().rev().find(|(k, _)| *k <= at).map(|(_, v)| *v).unwrap_or(0.0));
        let sel = buf.selection();
        let marked = buf.marked_range();
        if focused && !sel.is_empty() {
            let (a, e) = (sel.start.clamp(start, end), sel.end.clamp(start, end));
            let (sx, ex) = (px(&marks, a), px(&marks, e));
            if ex > sx {
                s.fill_rrect(RectF::new(left + sx, top, ex - sx, ui.pt(17.)), 0., SELECTION.now());
            }
        }
        if value.is_empty() {
            let im = ui.text(&self.placeholder, MAC_BODY_PT, Weight::Regular, LABEL_2.now());
            blit(s, &im, left as i32, top as i32);
        } else {
            // segments split at the selection and marked-range edges: selected text is white, marked text underlined
            let mut cuts = vec![start, end];
            for c in [sel.start, sel.end].into_iter().chain(marked.iter().flat_map(|m| [m.start, m.end])) {
                if c > start && c < end {
                    cuts.push(c);
                }
            }
            cuts.sort_unstable();
            cuts.dedup();
            for pair in cuts.windows(2) {
                let (a, e) = (pair[0], pair[1]);
                let selected = focused && !sel.is_empty() && a >= sel.start && e <= sel.end;
                let color = if selected { WHITE } else { LABEL.now() };
                let im = ui.text(&value[a..e], MAC_BODY_PT, Weight::Regular, color);
                let ax = left + px(&marks, a);
                blit(s, &im, ax as i32, top as i32);
                if marked.as_ref().is_some_and(|m| a >= m.start && e <= m.end) {
                    // the clause the IME has selected gets a heavier line, as in the native toolkit
                    let thick = if !sel.is_empty() && a >= sel.start && e <= sel.end { 2.0 } else { 1.0 };
                    s.fill_rrect(RectF::new(ax, top + ui.pt(16.), px(&marks, e) - px(&marks, a), ui.pt(thick)), 0., color);
                }
            }
        }
        if focused && sel.is_empty() {
            s.fill_rrect(RectF::new(left + px(&marks, caret), top, ui.pt(1.), ui.pt(17.)), 0., TINT);
        }
        self.start = start;
        self.marks = marks;
    }
    fn handle(&mut self, ev: &ViewEvent) -> bool {
        if !self.base.enabled {
            return false;
        }
        match ev {
            ViewEvent::Pointer(Event::Down(px, _)) => {
                let before = self.text().to_string();
                if self.editor.buf.has_marked_text() {
                    self.editor.unmark_text(true);
                }
                let at = self.byte_at(px - self.base.frame.x - FIELD_INSET_PT);
                self.editor.buf.set_selection(at, at);
                self.dragging = true;
                self.edited(&before);
                true
            }
            ViewEvent::Pointer(Event::Move(px, _)) => {
                if self.dragging {
                    let at = self.byte_at(px - self.base.frame.x - FIELD_INSET_PT);
                    let anchor = self.editor.buf.anchor();
                    self.editor.buf.set_selection(anchor, at);
                }
                self.base.dirty = true;
                true
            }
            ViewEvent::Pointer(_) | ViewEvent::Cancel => {
                self.dragging = false;
                self.base.dirty = true;
                true
            }
            ViewEvent::Key(k) => self.handle_key(k),
        }
    }
    fn perform(&mut self, action: &str) -> bool {
        let Some(cmd) = Self::clipboard_command(action) else { return false };
        if self.validate(action) != Some(true) {
            return false;
        }
        let before = self.text().to_string();
        match cmd {
            Command::Cut => {
                clipboard::set_text(self.editor.buf.selected_text());
                self.editor.perform(EditCommand::DeleteBackward, false);
            }
            Command::Copy => clipboard::set_text(self.editor.buf.selected_text()),
            Command::Paste => {
                let clip = Self::single_line(&clipboard::text().unwrap_or_default());
                self.editor.insert_text(&clip);
            }
            Command::SelectAll => {
                self.editor.perform(EditCommand::SelectAll, false);
            }
            Command::Undo => {
                self.editor.undo();
            }
            Command::Redo => {
                self.editor.redo();
            }
        }
        self.edited(&before);
        true
    }
    fn validate(&self, action: &str) -> Option<bool> {
        let buf = &self.editor.buf;
        match action {
            "cut" => Some(buf.has_selection() && !buf.has_marked_text()),
            "copy" => Some(buf.has_selection()),
            "paste" => Some(clipboard::text().is_some_and(|t| !Self::single_line(&t).is_empty())),
            "selectAll" => Some(!buf.is_empty()),
            // nothing to undo here: defer to the window's undo manager
            "undo" => self.editor.can_undo().then_some(true),
            "redo" => self.editor.can_redo().then_some(true),
            _ => None,
        }
    }
    fn ax_node(&self) -> Option<Node> {
        node(self, Role::TextField, &self.placeholder, self.text())
    }
    /// The insertion point, the selection and the part of the text scrolled into view.
    fn ax_text(&self) -> Option<bifrost_ui::ax::AxText> {
        let buf = &self.editor.buf;
        let start = self.start.min(buf.len());
        let end = self.marks.last().map_or(buf.len(), |(b, _)| *b).max(start);
        Some(bifrost_ui::ax::text_ax(self.id(), buf.text(), buf.anchor(), buf.caret(), (start, end), false))
    }
    fn ax_meta(&self) -> Option<(String, String)> {
        (!self.ax_title.is_empty() || !self.ax_help.is_empty()).then(|| (self.ax_title.clone(), self.ax_help.clone()))
    }
    fn ax_set_value(&mut self, value: &str) -> bool {
        let before = self.text().to_string();
        self.editor.perform(EditCommand::SelectAll, false);
        let value = Self::single_line(value);
        if value.is_empty() {
            self.editor.perform(EditCommand::DeleteBackward, false);
        } else {
            self.editor.insert_text(&value);
        }
        self.edited(&before);
        true
    }
    fn text_input_info(&self) -> Option<TextInputInfo> {
        if !self.base.focused {
            return None;
        }
        let sur = self.editor.buf.surrounding(MAX_SURROUNDING);
        let state = TextState {
            text: sur.text.to_string(),
            cursor: sur.cursor as u32,
            anchor: sur.anchor as u32,
            marked: sur.marked.map(|(a, b)| (a as u32, b as u32)),
        };
        let f = self.base.frame;
        let caret = (f.x + FIELD_INSET_PT + self.x_of(self.editor.buf.caret()), f.y + (f.h - 16.) / 2., 1.0, 17.0);
        Some(TextInputInfo { state, caret, traits: self.traits })
    }
    fn apply_text_input(&mut self, ev: &ToApp) -> bool {
        if !self.base.enabled {
            return false;
        }
        let before = self.text().to_string();
        match ev {
            ToApp::SetMarkedText { text, sel_start, sel_len } => {
                self.editor.set_marked_text(text, *sel_start as usize, *sel_len as usize, None);
            }
            ToApp::UnmarkText { commit } => {
                self.editor.unmark_text(*commit);
            }
            ToApp::DeleteSurrounding { before: b, after } => {
                self.editor.delete_surrounding(*b as usize, *after as usize);
            }
            _ => return false,
        }
        self.edited(&before);
        true
    }
}

/// A pop-up button (`NativePopUpButton`) with its menu (`bifrost_ui::menu::Menu`, drawn by `widgets::menu` as an overlay
/// above the tree). Sends `action` when another item is chosen; read it with `selected()`.
pub struct PopUpButton {
    base: ViewBase,
    items: Vec<String>,
    selected: usize,
    action: String,
    open: bool,
    /// the menu just closed: the area it covered still has to be repainted
    stale: bool,
    menu: Menu,
}

impl PopUpButton {
    pub fn new(items: &[&str], selected: usize, action: &str) -> Self {
        let items: Vec<String> = items.iter().map(|i| i.to_string()).collect();
        let selected = selected.min(items.len().saturating_sub(1));
        PopUpButton { base: ViewBase::default(), items, selected, action: action.into(), open: false, stale: false, menu: Menu::default() }
    }
    pub fn selected(&self) -> usize {
        self.selected
    }
    pub fn selected_title(&self) -> &str {
        self.items.get(self.selected).map(String::as_str).unwrap_or("")
    }
    pub fn is_open(&self) -> bool {
        self.open
    }
    /// The open menu's rectangle in points: below the button, as wide, standard menu metrics.
    pub fn menu_rect(&self) -> Rect {
        let f = self.base.frame;
        let (_, h) = self.menu.rows(MENU_ROW_PT, MENU_SEPARATOR_PT, MENU_PAD_PT);
        Rect { x: f.x, y: f.bottom() + SPACE_XXS, w: f.w, h }
    }
    pub fn open_menu(&mut self) {
        if self.items.is_empty() || self.open {
            return;
        }
        self.menu = Menu::popup(self.items.iter().enumerate().map(|(i, t)| MenuItem::new(i as u32 + 1, t)).collect(), self.selected);
        self.open = true;
        self.base.dirty = true;
    }
    pub fn close_menu(&mut self) {
        if self.open {
            self.open = false;
            self.stale = true;
            self.base.dirty = true;
        }
    }
    fn choose(&mut self, i: usize) {
        if i != self.selected && i < self.items.len() {
            self.selected = i;
            let a = self.action.clone();
            self.base.send_action(&a);
        }
        self.close_menu();
    }
}

impl View for PopUpButton {
    crate::view_base!();
    fn intrinsic(&self, ui: &Ui) -> (f32, f32) {
        let w = self.items.iter().map(|t| ui.measure(t, MAC_BODY_PT, Weight::Regular) / ui.pt(1.0)).fold(0.0, f32::max);
        (w + SPACE_XL + SPACE_L, MENU_ROW_PT)
    }
    fn accepts_first_responder(&self) -> bool {
        true
    }
    fn did_resign_first_responder(&mut self) {
        self.close_menu();
    }
    fn hit_test(&self, x: f32, y: f32) -> bool {
        self.base.frame.contains(x, y) || (self.open && self.menu_rect().contains(x, y))
    }
    fn dirty_rect(&self) -> Rect {
        if self.open || self.stale { self.base.frame.union(self.menu_rect()) } else { self.base.frame }
    }
    fn draw(&mut self, s: &mut dyn Canvas, ui: &Ui) {
        self.stale = false;
        widgets::popup_button(s, ui, self.base.frame, self.selected_title(), self.open, self.base.enabled);
        if self.base.focused && !self.open {
            focus_ring(s, ui, self.base.frame, FIELD_RADIUS_PT);
        }
    }
    fn draw_overlay(&mut self, s: &mut dyn Canvas, ui: &Ui) {
        if self.open {
            let r = self.menu_rect();
            widgets::menu(s, ui, (r.x, r.y), r.w, &self.menu);
        }
    }
    fn handle(&mut self, ev: &ViewEvent) -> bool {
        if !self.base.enabled {
            return false;
        }
        match ev {
            ViewEvent::Pointer(Event::Down(x, y)) => {
                if self.open {
                    let r = self.menu_rect();
                    if r.contains(*x, *y) {
                        if let Some(i) = self.menu.hit(*y - r.y, MENU_ROW_PT, MENU_SEPARATOR_PT, MENU_PAD_PT) {
                            self.choose(i);
                            return true;
                        }
                    }
                    self.close_menu();
                } else {
                    self.open_menu();
                }
            }
            ViewEvent::Pointer(_) | ViewEvent::Cancel => {}
            ViewEvent::Key(KeyInput::Code(27)) if self.open => self.close_menu(),
            ViewEvent::Key(KeyInput::Code(9)) if self.open => {
                self.close_menu();
                return false;
            }
            ViewEvent::Key(KeyInput::Code(c @ (38 | 40))) if self.open => {
                self.menu.step(if *c == 38 { -1 } else { 1 });
                self.base.dirty = true;
            }
            ViewEvent::Key(KeyInput::Code(40)) => self.open_menu(),
            k if is_activate(k) => {
                if self.open {
                    match self.menu.activate() {
                        Some(id) => self.choose(id as usize - 1),
                        None => self.close_menu(),
                    }
                } else {
                    self.open_menu();
                }
            }
            _ => return false,
        }
        true
    }
    fn ax_node(&self) -> Option<Node> {
        node(self, Role::Button, self.selected_title(), if self.open { "open" } else { "" })
    }
    fn ax_set_value(&mut self, value: &str) -> bool {
        match self.items.iter().position(|t| t == value) {
            Some(i) => {
                self.choose(i);
                true
            }
            None => false,
        }
    }
}

/// An inset grouped card (`List` with `.insetGrouped`): rows in a rounded card (13 pt radius, `m::CARD_RADIUS`) with
/// hairline separators, an optional footnote header above and footer below. Opaque: a dirty row repaints the card.
pub struct Section {
    base: ViewBase,
    header: Option<String>,
    footer: Option<String>,
    rows: Vec<Box<dyn View>>,
    row_h: Vec<f32>,
}

impl Section {
    pub fn new(rows: Vec<Box<dyn View>>) -> Self {
        Section { base: ViewBase::default(), header: None, footer: None, rows, row_h: Vec::new() }
    }
    pub fn header(mut self, text: &str) -> Self {
        self.header = Some(text.into());
        self
    }
    pub fn footer(mut self, text: &str) -> Self {
        self.footer = Some(text.into());
        self
    }
    fn header_h(&self) -> f32 {
        if self.header.is_some() { MAC_GROUP_HEADER_PT + SPACE_S } else { 0.0 }
    }
    fn footer_h(&self) -> f32 {
        if self.footer.is_some() { MAC_FOOTNOTE_PT + SPACE_S } else { 0.0 }
    }
    fn heights(&self, ui: &Ui) -> Vec<f32> {
        self.rows.iter().map(|r| r.intrinsic(ui).1.max(ROW_PT)).collect()
    }
    /// The card's rectangle (rows only) inside the section frame.
    pub fn card(&self) -> Rect {
        let f = self.base.frame;
        let h: f32 = self.row_h.iter().sum();
        Rect { x: f.x, y: f.y + self.header_h(), w: f.w, h }
    }
}

impl View for Section {
    crate::view_base!();
    fn children(&self) -> &[Box<dyn View>] {
        &self.rows
    }
    fn children_mut(&mut self) -> &mut [Box<dyn View>] {
        &mut self.rows
    }
    fn opaque_backdrop(&self) -> bool {
        true
    }
    fn intrinsic(&self, ui: &Ui) -> (f32, f32) {
        (0.0, self.header_h() + self.heights(ui).iter().sum::<f32>() + self.footer_h())
    }
    fn layout(&mut self, ui: &Ui, bounds: Rect) {
        self.row_h = self.heights(ui);
        let h = self.header_h() + self.row_h.iter().sum::<f32>() + self.footer_h();
        self.set_frame(Rect { h, ..bounds });
        let mut y = bounds.y + self.header_h();
        let heights = self.row_h.clone();
        for (r, rh) in self.rows.iter_mut().zip(heights) {
            r.layout(ui, Rect { x: bounds.x + SPACE_M, y, w: (bounds.w - 2.0 * SPACE_M).max(0.0), h: rh });
            y += rh;
        }
    }
    fn draw(&mut self, s: &mut dyn Canvas, ui: &Ui) {
        let f = self.base.frame;
        let c = self.card();
        if let Some(h) = &self.header {
            let r = Rect { x: f.x + SPACE_M, y: f.y, w: f.w - 2.0 * SPACE_M, h: self.header_h() };
            draw_text(s, ui, r, h, MAC_GROUP_HEADER_PT, Weight::Regular, LABEL_2.now(), false);
        }
        s.fill_rrect(RectF::new(ui.pt(c.x), ui.pt(c.y), ui.pt(c.w), ui.pt(c.h)), ui.pt(m::CARD_RADIUS), CARD_BG.now());
        let mut y = c.y;
        for rh in self.row_h.iter().take(self.row_h.len().saturating_sub(1)) {
            y += rh;
            s.fill_rect(RectF::new(ui.pt(c.x + SPACE_M).trunc(), ui.pt(y).trunc(), ui.pt(c.w - 2.0 * SPACE_M).max(0.0).trunc(), 1.0), SEPARATOR_C.now());
        }
        if let Some(t) = &self.footer {
            let r = Rect { x: f.x + SPACE_M, y: c.bottom(), w: f.w - 2.0 * SPACE_M, h: self.footer_h() };
            draw_text(s, ui, r, t, MAC_FOOTNOTE_PT, Weight::Regular, LABEL_2.now(), false);
        }
    }
}

/// A form: grouped [`Section`]s stacked with a gap, inset by the detail margin (`m::DETAIL_MARGIN`). No scrolling.
pub struct Form {
    base: ViewBase,
    sections: Vec<Box<dyn View>>,
}

impl Form {
    pub fn new(sections: Vec<Section>) -> Self {
        Form { base: ViewBase::default(), sections: sections.into_iter().map(|s| Box::new(s) as Box<dyn View>).collect() }
    }
}

impl View for Form {
    crate::view_base!();
    fn children(&self) -> &[Box<dyn View>] {
        &self.sections
    }
    fn children_mut(&mut self) -> &mut [Box<dyn View>] {
        &mut self.sections
    }
    fn intrinsic(&self, ui: &Ui) -> (f32, f32) {
        let n = self.sections.len().saturating_sub(1) as f32;
        (0.0, self.sections.iter().map(|s| s.intrinsic(ui).1).sum::<f32>() + n * SPACE_XL)
    }
    fn layout(&mut self, ui: &Ui, bounds: Rect) {
        self.set_frame(bounds);
        let mut y = bounds.y;
        for s in self.sections.iter_mut() {
            let h = s.intrinsic(ui).1;
            s.layout(ui, Rect { x: bounds.x + m::DETAIL_MARGIN, y, w: (bounds.w - 2.0 * m::DETAIL_MARGIN).max(0.0), h });
            y += h + SPACE_XL;
        }
    }
}
