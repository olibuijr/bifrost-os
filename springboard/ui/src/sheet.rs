//! Sheets, alert sheets and the small controls that sit in them (`NativeWindow.beginSheet`, `NativeAlert`,
//! `NativeSegmentedControl`, `NativeStepper`): renderer-independent layout, hit-testing and key mapping.
//! A sheet hangs from the title bar of its parent window, dims and blocks it, maps Return to the default
//! button and Escape to the cancel button. `progress` (0..1) is the slide, driven by the caller from
//! `motion::Kind::SheetSlide` (with Reduce Motion the duration is 0, so progress is 1 at once).
use crate::geometry::Edge;
use crate::keys::Key;
use crate::Rect;

/// Role of a sheet button; decides the key it answers to and its look.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ButtonRole {
    /// Answers Return; filled with the accent.
    Default,
    /// Answers Escape.
    Cancel,
    Normal,
    /// Like `Normal`, drawn in the destructive colour.
    Destructive,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SheetButton {
    pub id: u32,
    pub title: String,
    pub role: ButtonRole,
}

impl SheetButton {
    pub fn new(id: u32, title: &str, role: ButtonRole) -> Self {
        SheetButton { id, title: title.to_string(), role }
    }
}

/// Button height and gap in points (regular push button).
pub const BUTTON_H_PT: f32 = 22.0;
pub const BUTTON_GAP_PT: f32 = 12.0;
pub const SHEET_PAD_PT: f32 = 20.0;
pub const SHEET_RADIUS_PT: f32 = 10.0;
/// Alert sheet width in points.
pub const ALERT_W_PT: f32 = 260.0;
/// An alert sheet stacks its buttons, full width, as the Golden Gate alert sheet does (reference
/// `goldengate_refimages/27-Golden-Gate-Mail-Warning.png`, measured at 2x): 28 pt capsules 6 pt apart, inset 16 pt
/// from the panel's sides and bottom, the default button on top and Cancel at the bottom.
pub const ALERT_BUTTON_H_PT: f32 = 28.0;
pub const ALERT_BUTTON_GAP_PT: f32 = 6.0;
pub const ALERT_BUTTON_INSET_PT: f32 = 16.0;
const CHARS_PER_LINE: usize = 34;
/// The alert title (headline, semibold) fits fewer characters on a line than the message.
const TITLE_CHARS_PER_LINE: usize = 28;
/// Line advance of a wrapped alert title and of a message line, in points.
pub const ALERT_TITLE_LINE_PT: f32 = 17.0;
pub const ALERT_MESSAGE_LINE_PT: f32 = 15.0;

/// How many lines `text` takes when word-wrapped at `per_line` characters: each `\n` starts a paragraph (an empty
/// one is a line), a word longer than a line is broken across lines. Only the first estimate of an alert's height:
/// wide glyphs wrap to more lines than a character count says, so the renderer measures the real wrap and records it
/// (`record_alert_height`), and the alert is sized from that measurement from then on.
pub fn wrapped_lines(text: &str, per_line: usize) -> usize {
    let per_line = per_line.max(1);
    text.split('\n')
        .map(|para| {
            let (mut lines, mut len) = (1usize, 0usize);
            for word in para.split_whitespace() {
                let n = word.chars().count();
                if len == 0 {
                    len = n;
                } else if len + 1 + n <= per_line {
                    len += 1 + n;
                } else {
                    lines += 1;
                    len = n;
                }
                while len > per_line {
                    lines += 1;
                    len -= per_line;
                }
            }
            lines
        })
        .sum()
}

/// The panel height of an alert whose title takes `title_lines` lines and whose message `message_lines` lines (0 for
/// no message). The renderer draws the title from the top padding at `ALERT_TITLE_LINE_PT` a line, the message 26 pt
/// below the title's first line at `ALERT_MESSAGE_LINE_PT` a line, and keeps 16 pt above the buttons.
pub fn alert_height(title_lines: usize, message_lines: usize) -> f32 {
    let title_h = 18.0 + title_lines.saturating_sub(1) as f32 * ALERT_TITLE_LINE_PT;
    let msg_h = if message_lines == 0 { 0.0 } else { message_lines as f32 * ALERT_MESSAGE_LINE_PT + 8.0 };
    SHEET_PAD_PT * 2.0 + title_h + msg_h + 16.0 + BUTTON_H_PT
}

/// The panel height of an alert sheet with `buttons` stacked buttons (`alert_height` holds one 22 pt row and the
/// bottom padding; the stack and its 16 pt inset replace them). No buttons: `alert_height`.
pub fn alert_sheet_height(title_lines: usize, message_lines: usize, buttons: usize) -> f32 {
    let text = alert_height(title_lines, message_lines);
    if buttons == 0 {
        return text;
    }
    text - SHEET_PAD_PT - BUTTON_H_PT + ALERT_BUTTON_INSET_PT + stacked_buttons_h(buttons)
}

/// The height of `n` stacked alert buttons and the gaps between them.
pub fn stacked_buttons_h(n: usize) -> f32 {
    n as f32 * ALERT_BUTTON_H_PT + n.saturating_sub(1) as f32 * ALERT_BUTTON_GAP_PT
}

/// What a sheet's panel is drawn as (`widgets::sheet`): the clip, which starts at the title bar's bottom edge (the
/// sheet slides out from under the title bar and its top edge is attached to it, with no gap), and the rounded body,
/// which reaches `SHEET_RADIUS_PT` above the clip so its top corners are cut off square and only the bottom corners
/// are rounded. `None` while nothing of the panel is below the title bar.
pub fn attached_body(panel: Rect, parent: Rect, titlebar_h: f32) -> Option<(Rect, Rect)> {
    let top = panel.y.max(parent.y + titlebar_h);
    if panel.bottom() <= top || panel.w <= 0.0 {
        return None;
    }
    let clip = Rect { x: panel.x, y: top, w: panel.w, h: panel.bottom() - top };
    let body = Rect { x: panel.x, y: panel.y - SHEET_RADIUS_PT, w: panel.w, h: panel.h + SHEET_RADIUS_PT };
    Some((clip, body))
}

/// How many measured alert heights are kept (one per alert text and width; an app shows one or two at a time).
const MEASURED_KEEP: usize = 8;

thread_local! {
    /// Alert heights the renderer measured (`record_alert_height`): title, message, width bits, height.
    static MEASURED: std::cell::RefCell<Vec<(String, String, u32, f32)>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// The renderer wrapped an alert of `title` and `message` at panel width `width_pt` with real glyph widths and needs
/// `height_pt` (`alert_sheet_height` of its line counts and buttons). From then on `Sheet::alert` and `Sheet::layout` size that alert
/// to it, so the panel, the drawn text and the button hit areas always agree, also for a sheet built again each frame.
pub fn record_alert_height(title: &str, message: &str, width_pt: f32, height_pt: f32) {
    MEASURED.with(|m| {
        let mut m = m.borrow_mut();
        let bits = width_pt.to_bits();
        if let Some(i) = m.iter().position(|(t, msg, w, _)| *w == bits && t == title && msg == message) {
            let mut entry = m.remove(i);
            entry.3 = height_pt;
            m.push(entry);
        } else {
            if m.len() >= MEASURED_KEEP {
                m.remove(0);
            }
            m.push((title.to_string(), message.to_string(), bits, height_pt));
        }
    });
}

/// The height `record_alert_height` recorded for this alert text at `width_pt`, if it was drawn.
pub fn measured_alert_height(title: &str, message: &str, width_pt: f32) -> Option<f32> {
    let bits = width_pt.to_bits();
    MEASURED.with(|m| m.borrow().iter().rev().find(|(t, msg, w, _)| *w == bits && t == title && msg == message).map(|e| e.3))
}

#[derive(Clone, Debug, PartialEq)]
pub struct Sheet {
    pub title: String,
    pub message: String,
    pub buttons: Vec<SheetButton>,
    /// Panel size in points; for an alert it is computed from the text.
    pub size: (f32, f32),
    pub alert: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SheetLayout {
    /// The dimmed, blocking area: the whole parent window.
    pub scrim: Rect,
    pub panel: Rect,
    /// A free-form sheet's buttons right-aligned in a row, the first button the rightmost. An alert's buttons stacked
    /// full width from top to bottom: the default button, the others in order, then Cancel (`stacked_order`).
    pub buttons: Vec<(u32, Rect)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
    Button(u32),
    Panel,
    /// The dimmed parent: the click is swallowed.
    Blocked,
    /// Outside the parent window altogether.
    Outside,
}

impl Sheet {
    /// A free-form sheet of the given size with a row of buttons at the bottom.
    pub fn new(title: &str, size: (f32, f32), buttons: Vec<SheetButton>) -> Self {
        Sheet { title: title.to_string(), message: String::new(), buttons, size, alert: false }
    }

    /// An alert sheet: title, informative text and buttons; height follows the text. Both the title and the message
    /// wrap: a long title such as a whole sentence is never cut off. The height is the one the renderer measured for
    /// this text with real glyph widths (`record_alert_height`) once the alert has been drawn, and until then an
    /// estimate from character counts (`wrapped_lines`); `layout` and `hit` always use the measured height.
    pub fn alert(title: &str, message: &str, buttons: Vec<SheetButton>) -> Self {
        let message_lines = if message.is_empty() { 0 } else { wrapped_lines(message, CHARS_PER_LINE) };
        let estimate = alert_sheet_height(wrapped_lines(title, TITLE_CHARS_PER_LINE), message_lines, buttons.len());
        let h = measured_alert_height(title, message, ALERT_W_PT).unwrap_or(estimate);
        Sheet { title: title.to_string(), message: message.to_string(), buttons, size: (ALERT_W_PT, h), alert: true }
    }

    /// The panel height before it is clamped to the parent: for an alert the renderer measured, that measurement (so
    /// the buttons are hit where they were drawn), else `size.1`.
    pub fn height(&self) -> f32 {
        if self.alert {
            measured_alert_height(&self.title, &self.message, self.size.0).unwrap_or(self.size.1)
        } else {
            self.size.1
        }
    }

    pub fn default_button(&self) -> Option<u32> {
        self.buttons.iter().find(|b| b.role == ButtonRole::Default).map(|b| b.id)
    }

    pub fn cancel_button(&self) -> Option<u32> {
        self.buttons.iter().find(|b| b.role == ButtonRole::Cancel).map(|b| b.id)
    }

    /// Return activates the default button, Escape the cancel button (a sheet with none ignores Escape). The sheet
    /// consumes every key while shown (`true` in the second field), so nothing reaches the blocked parent.
    pub fn key(&self, key: Key) -> (Option<u32>, bool) {
        match key {
            Key::Return => (self.default_button(), true),
            Key::Escape => (self.cancel_button(), true),
            _ => (None, true),
        }
    }

    /// Layout inside `parent` (the window rectangle, points), whose title bar is `titlebar_h` tall. At `progress` 0 the
    /// panel is fully above the title bar's bottom edge; at 1 it hangs from it, centred horizontally.
    pub fn layout(&self, parent: Rect, titlebar_h: f32, progress: f32) -> SheetLayout {
        let p = progress.clamp(0.0, 1.0);
        let w = self.size.0.min(parent.w);
        let h = self.height().min((parent.h - titlebar_h).max(0.0));
        let rest_y = parent.y + titlebar_h;
        let panel = Rect { x: parent.x + (parent.w - w) / 2.0, y: rest_y - h * (1.0 - p), w, h };
        if self.alert {
            let order = stacked_order(&self.buttons);
            let bw = (w - ALERT_BUTTON_INSET_PT * 2.0).max(0.0);
            let mut y = panel.bottom() - ALERT_BUTTON_INSET_PT - stacked_buttons_h(order.len());
            let buttons = order
                .into_iter()
                .map(|b| {
                    let r = Rect { x: panel.x + ALERT_BUTTON_INSET_PT, y, w: bw, h: ALERT_BUTTON_H_PT };
                    y += ALERT_BUTTON_H_PT + ALERT_BUTTON_GAP_PT;
                    (b.id, r)
                })
                .collect();
            return SheetLayout { scrim: parent, panel, buttons };
        }
        let mut x = panel.right() - SHEET_PAD_PT;
        let y = panel.bottom() - SHEET_PAD_PT - BUTTON_H_PT;
        let mut buttons = Vec::new();
        for b in &self.buttons {
            let bw = (b.title.chars().count() as f32 * 7.0 + 28.0).max(68.0);
            x -= bw;
            buttons.push((b.id, Rect { x, y, w: bw, h: BUTTON_H_PT }));
            x -= BUTTON_GAP_PT;
        }
        SheetLayout { scrim: parent, panel, buttons }
    }

    pub fn hit(&self, parent: Rect, titlebar_h: f32, progress: f32, x: f32, y: f32) -> Hit {
        if !parent.contains(x, y) {
            return Hit::Outside;
        }
        let l = self.layout(parent, titlebar_h, progress);
        if let Some((id, _)) = l.buttons.iter().find(|(_, r)| r.contains(x, y)) {
            return Hit::Button(*id);
        }
        if l.panel.contains(x, y) {
            Hit::Panel
        } else {
            Hit::Blocked
        }
    }
}

/// An alert's buttons from top to bottom: the default button, the others in their order, Cancel last.
pub fn stacked_order(buttons: &[SheetButton]) -> Vec<&SheetButton> {
    let rank = |b: &SheetButton| match b.role {
        ButtonRole::Default => 0,
        ButtonRole::Cancel => 2,
        _ => 1,
    };
    let mut order: Vec<&SheetButton> = buttons.iter().collect();
    order.sort_by_key(|b| rank(*b));
    order
}

/// Segmented control metrics, in points (regular size).
pub const SEGMENT_H_PT: f32 = 24.0;
pub const SEGMENT_MIN_W_PT: f32 = 48.0;

/// Segment rectangles of `n` equal segments inside `bounds` (height is the control height, centred).
pub fn segments(bounds: Rect, n: usize) -> Vec<Rect> {
    if n == 0 {
        return Vec::new();
    }
    let h = SEGMENT_H_PT;
    let y = bounds.y + (bounds.h - h) / 2.0;
    let w = (bounds.w / n as f32).max(SEGMENT_MIN_W_PT);
    (0..n).map(|i| Rect { x: bounds.x + w * i as f32, y, w, h }).collect()
}

pub fn segment_hit(bounds: Rect, n: usize, x: f32, y: f32) -> Option<usize> {
    segments(bounds, n).iter().position(|r| r.contains(x, y))
}

/// Left/Right move the selection by one, without wrapping.
pub fn segment_key(selected: usize, n: usize, key: Key) -> usize {
    match key {
        Key::Left => selected.saturating_sub(1),
        Key::Right => (selected + 1).min(n.saturating_sub(1)),
        _ => selected,
    }
}

/// Stepper metrics in points: 13 wide, 22 tall, two halves.
pub const STEPPER_W_PT: f32 = 13.0;
pub const STEPPER_H_PT: f32 = 22.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StepperPart {
    Up,
    Down,
}

/// The stepper's rectangle with its top-left at `origin`.
pub fn stepper_rect(origin: (f32, f32)) -> Rect {
    Rect { x: origin.0, y: origin.1, w: STEPPER_W_PT, h: STEPPER_H_PT }
}

pub fn stepper_hit(origin: (f32, f32), x: f32, y: f32) -> Option<StepperPart> {
    let r = stepper_rect(origin);
    if !r.contains(x, y) {
        return None;
    }
    Some(if y < r.y + r.h / 2.0 { StepperPart::Up } else { StepperPart::Down })
}

/// Applies one step to `value` within `min..=max`; returns the new value.
pub fn stepper_apply(value: f64, part: StepperPart, step: f64, min: f64, max: f64) -> f64 {
    let v = match part {
        StepperPart::Up => value + step,
        StepperPart::Down => value - step,
    };
    v.clamp(min, max)
}

/// Keyboard focus ring: 3 pt wide, drawn outside the control, radius grows by the same amount.
pub const FOCUS_RING_PT: f32 = 3.0;

/// The ring's outer rectangle and radius for a control of `bounds` and `radius`.
pub fn focus_ring_rect(bounds: Rect, radius: f32) -> (Rect, f32) {
    let g = FOCUS_RING_PT;
    (Rect { x: bounds.x - g, y: bounds.y - g, w: bounds.w + g * 2.0, h: bounds.h + g * 2.0 }, radius + g)
}

/// The anchor edge a popover's arrow touches, given the edge of the anchor the panel opened on.
pub fn arrow_edge(panel_edge: Edge) -> Edge {
    panel_edge.opposite()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn alert() -> Sheet {
        Sheet::alert("Delete file?", "This cannot be undone.", vec![SheetButton::new(1, "Delete", ButtonRole::Default), SheetButton::new(2, "Cancel", ButtonRole::Cancel)])
    }

    #[test]
    fn return_maps_to_default_and_escape_to_cancel_and_all_keys_are_consumed() {
        let s = alert();
        assert_eq!(s.key(Key::Return), (Some(1), true));
        assert_eq!(s.key(Key::Escape), (Some(2), true));
        assert_eq!(s.key(Key::Char('a')), (None, true));
        let only_default = Sheet::new("t", (300.0, 100.0), vec![SheetButton::new(9, "OK", ButtonRole::Default)]);
        assert_eq!(only_default.key(Key::Escape), (None, true));
    }

    #[test]
    fn the_panel_slides_down_from_the_title_bar_and_stays_centred() {
        let s = alert();
        let parent = Rect { x: 100.0, y: 50.0, w: 600.0, h: 400.0 };
        let (a, b) = (s.layout(parent, 28.0, 0.0), s.layout(parent, 28.0, 1.0));
        assert_eq!(a.panel.bottom(), 78.0);
        assert_eq!(b.panel.y, 78.0);
        assert_eq!(b.panel.x + b.panel.w / 2.0, 400.0);
        assert_eq!(s.layout(parent, 28.0, 7.0), b, "progress clamps");
    }

    #[test]
    fn the_sheet_blocks_the_parent_and_hits_buttons() {
        let s = alert();
        let parent = Rect { x: 0.0, y: 0.0, w: 600.0, h: 400.0 };
        let l = s.layout(parent, 28.0, 1.0);
        let (id, r) = l.buttons[0];
        assert_eq!(id, 1);
        assert_eq!(l.buttons[1].1.y, r.bottom() + ALERT_BUTTON_GAP_PT, "an alert's second button sits below the first");
        assert_eq!(s.hit(parent, 28.0, 1.0, r.x + 2.0, r.y + 2.0), Hit::Button(1));
        assert_eq!(s.hit(parent, 28.0, 1.0, 10.0, 300.0), Hit::Blocked);
        assert_eq!(s.hit(parent, 28.0, 1.0, l.panel.x + 5.0, l.panel.y + 5.0), Hit::Panel);
        assert_eq!(s.hit(parent, 28.0, 1.0, 900.0, 10.0), Hit::Outside);
    }

    #[test]
    fn alert_height_follows_the_message_and_fits_small_parents() {
        let short = Sheet::alert("T", "x", vec![]);
        let long = Sheet::alert("T", &"word ".repeat(40), vec![]);
        assert!(long.size.1 > short.size.1);
        let tiny = Rect { x: 0.0, y: 0.0, w: 200.0, h: 100.0 };
        let l = long.layout(tiny, 28.0, 1.0);
        assert!(l.panel.w <= 200.0 && l.panel.h <= 72.0);
    }

    #[test]
    fn a_long_alert_title_wraps_and_the_panel_grows_for_every_line() {
        assert_eq!(wrapped_lines("", 10), 1);
        assert_eq!(wrapped_lines("one two", 10), 1);
        assert_eq!(wrapped_lines("one two three", 10), 2, "words that do not fit start a new line");
        assert_eq!(wrapped_lines("a\n\nb", 10), 3, "each paragraph, even an empty one, is a line");
        assert_eq!(wrapped_lines(&"x".repeat(25), 10), 3, "a word longer than a line breaks across lines");
        // the Notes delete alert puts a whole sentence in the title: it must not be cut to one line (E2E N9)
        let short = Sheet::alert("Delete?", "", vec![]);
        let long = Sheet::alert("This note will be deleted immediately. You can\u{2019}t undo this action.", "", vec![]);
        let n = wrapped_lines(&long.title, TITLE_CHARS_PER_LINE);
        assert!(n >= 3, "the sentence takes {n} lines");
        assert_eq!(long.size.1 - short.size.1, (n - 1) as f32 * ALERT_TITLE_LINE_PT);
    }

    #[test]
    fn a_measured_alert_height_sizes_the_panel_and_moves_the_buttons_with_it() {
        let buttons = || vec![SheetButton::new(1, "OK", ButtonRole::Default)];
        let (title, message) = ("Wide", "WWWW MMMM WWWW MMMM WWWW MMMM WWWW");
        let parent = Rect { x: 0.0, y: 0.0, w: 800.0, h: 600.0 };
        let estimated = Sheet::alert(title, message, buttons());
        assert_eq!(estimated.size.1, alert_sheet_height(1, 1, 1), "34 characters are one line by count");
        // the renderer wrapped the message to three lines with real glyph widths
        let measured = alert_sheet_height(1, 3, 1);
        record_alert_height(title, message, ALERT_W_PT, measured);
        let l = estimated.layout(parent, 28.0, 1.0);
        assert_eq!(l.panel.h, measured, "a sheet built before the measurement takes it too");
        assert_eq!(l.buttons[0].1.y, l.panel.bottom() - ALERT_BUTTON_INSET_PT - ALERT_BUTTON_H_PT);
        assert_eq!(estimated.hit(parent, 28.0, 1.0, l.buttons[0].1.x + 2.0, l.buttons[0].1.y + 2.0), Hit::Button(1));
        let rebuilt = Sheet::alert(title, message, buttons());
        assert_eq!(rebuilt.size.1, measured, "a sheet built again each frame starts from the measurement");
        assert_eq!(rebuilt.layout(parent, 28.0, 1.0), l);
        // another width is another measurement; free-form sheets keep their own size
        assert_eq!(measured_alert_height(title, message, 400.0), None);
        let free = Sheet::new(title, (300.0, 120.0), buttons());
        assert_eq!(free.height(), 120.0);
    }

    /// E2E11: the save alert floated with rounded top corners and a gap under the title bar, its buttons in a row.
    #[test]
    fn an_alert_sheet_is_attached_to_the_title_bar_with_full_width_stacked_buttons() {
        let s = Sheet::alert(
            "Do you want to save the changes?",
            "Your changes will be lost if you don\u{2019}t save them.",
            vec![SheetButton::new(1, "Save", ButtonRole::Default), SheetButton::new(2, "Cancel", ButtonRole::Cancel), SheetButton::new(3, "Don\u{2019}t Save", ButtonRole::Destructive)],
        );
        let parent = Rect { x: 40.0, y: 30.0, w: 700.0, h: 500.0 };
        let bar = 52.0;
        let l = s.layout(parent, bar, 1.0);
        assert_eq!(l.panel.y, parent.y + bar, "the sheet's top is the title bar's bottom: no gap");
        let (clip, body) = attached_body(l.panel, parent, bar).expect("a shown sheet is drawn");
        assert_eq!(clip.y, parent.y + bar, "nothing is drawn over the title bar");
        assert_eq!((clip.x, clip.w, clip.bottom()), (l.panel.x, l.panel.w, l.panel.bottom()));
        assert!(body.y + SHEET_RADIUS_PT <= clip.y, "the rounding lies above the clip, so the top corners are square");
        assert_eq!(body.bottom(), l.panel.bottom(), "the bottom corners stay rounded");
        // Save on top, Don't Save, Cancel at the bottom, each as wide as the panel less its insets
        let ids: Vec<u32> = l.buttons.iter().map(|(id, _)| *id).collect();
        assert_eq!(ids, [1, 3, 2]);
        for (_, r) in &l.buttons {
            assert_eq!((r.x, r.w, r.h), (l.panel.x + ALERT_BUTTON_INSET_PT, l.panel.w - ALERT_BUTTON_INSET_PT * 2.0, ALERT_BUTTON_H_PT));
        }
        assert_eq!(l.buttons[2].1.bottom(), l.panel.bottom() - ALERT_BUTTON_INSET_PT);
        assert_eq!(l.buttons[1].1.y - l.buttons[0].1.bottom(), ALERT_BUTTON_GAP_PT);
        // sliding in, only what is below the title bar is drawn; fully up, nothing
        let half = s.layout(parent, bar, 0.5);
        let (clip, _) = attached_body(half.panel, parent, bar).unwrap();
        assert_eq!(clip.y, parent.y + bar);
        assert!(clip.h < half.panel.h);
        assert_eq!(attached_body(s.layout(parent, bar, 0.0).panel, parent, bar), None);
        // a free-form sheet keeps its row of buttons
        let free = Sheet::new("t", (300.0, 120.0), vec![SheetButton::new(1, "OK", ButtonRole::Default), SheetButton::new(2, "Cancel", ButtonRole::Cancel)]);
        let fl = free.layout(parent, bar, 1.0);
        assert!(fl.buttons[1].1.right() < fl.buttons[0].1.x);
    }

    #[test]
    fn segments_tile_the_bounds_and_arrow_keys_clamp() {
        let b = Rect { x: 10.0, y: 10.0, w: 300.0, h: 24.0 };
        let s = segments(b, 3);
        assert_eq!(s.len(), 3);
        assert_eq!(s[1].x, 110.0);
        assert_eq!(segment_hit(b, 3, 215.0, 20.0), Some(2));
        assert_eq!(segment_hit(b, 3, 5.0, 20.0), None);
        assert_eq!(segment_key(0, 3, Key::Left), 0);
        assert_eq!(segment_key(2, 3, Key::Right), 2);
        assert_eq!(segment_key(1, 3, Key::Right), 2);
        assert!(segments(b, 0).is_empty());
    }

    #[test]
    fn stepper_halves_step_and_clamp() {
        let o = (0.0, 0.0);
        assert_eq!(stepper_hit(o, 5.0, 3.0), Some(StepperPart::Up));
        assert_eq!(stepper_hit(o, 5.0, 18.0), Some(StepperPart::Down));
        assert_eq!(stepper_hit(o, 20.0, 3.0), None);
        assert_eq!(stepper_apply(9.0, StepperPart::Up, 2.0, 0.0, 10.0), 10.0);
        assert_eq!(stepper_apply(1.0, StepperPart::Down, 2.0, 0.0, 10.0), 0.0);
    }

    #[test]
    fn the_focus_ring_surrounds_the_control() {
        let (r, rad) = focus_ring_rect(Rect { x: 10.0, y: 10.0, w: 100.0, h: 22.0 }, 6.0);
        assert_eq!((r.x, r.w, rad), (7.0, 106.0, 9.0));
        assert_eq!(arrow_edge(Edge::Top), Edge::Bottom);
    }
}
