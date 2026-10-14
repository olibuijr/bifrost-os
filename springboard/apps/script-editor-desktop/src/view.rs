//! Where each part of the window sits, in points. Pure geometry: the drawing and the touch handling both read it, and
//! the tests cover it without a canvas.
use bifrost_app::controls::prefs_layout::m;

/// Height of the top band, which holds the Run and Stop buttons and the document name.
pub const BAR_H: f32 = 52.0;
/// Height of one editor line.
pub const LINE_H: f32 = 18.0;
/// Width of the line-number gutter.
pub const GUTTER_W: f32 = 44.0;
/// Space between the gutter and the first character.
pub const TEXT_PAD: f32 = 8.0;
/// Height of the status bar at the foot of the window.
pub const STATUS_H: f32 = 26.0;
/// Height of the output pane's heading.
pub const OUTPUT_HEAD_H: f32 = 24.0;
/// Height of one output line.
pub const OUTPUT_LINE_H: f32 = 16.0;
/// Width of a Run or Stop button.
pub const BUTTON_W: f32 = 64.0;
/// Height of a Run or Stop button.
pub const BUTTON_H: f32 = 32.0;
/// Corner radius of the buttons (flat corners, at most 6 points).
pub const BUTTON_RADIUS: f32 = m::ICON_RADIUS;
/// Share of the height above the status bar given to the editor; the output pane takes the rest.
pub const EDITOR_SHARE: f32 = 0.6;

/// A rectangle in points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct R {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl R {
    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && x < self.x + self.w && y >= self.y && y < self.y + self.h
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Button {
    Run,
    Stop,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Layout {
    pub width: f32,
    pub height: f32,
    pub run: R,
    pub stop: R,
    pub editor: R,
    pub output: R,
    pub status: R,
}

impl Layout {
    /// The button under the point, if any.
    pub fn button_at(&self, x: f32, y: f32) -> Option<Button> {
        if self.run.contains(x, y) {
            Some(Button::Run)
        } else if self.stop.contains(x, y) {
            Some(Button::Stop)
        } else {
            None
        }
    }

    /// Lines of text the editor shows at once.
    pub fn editor_lines(&self) -> usize {
        ((self.editor.h - 2.0 * TEXT_PAD) / LINE_H).floor().max(1.0) as usize
    }

    /// Lines of output the pane shows at once (below its heading).
    pub fn output_lines(&self) -> usize {
        ((self.output.h - OUTPUT_HEAD_H) / OUTPUT_LINE_H).floor().max(0.0) as usize
    }

    /// The top-left corner of the text for the line shown `shown_row` lines below the first shown line, in points.
    pub fn text_origin(&self, shown_row: usize) -> (f32, f32) {
        (self.editor.x + GUTTER_W + TEXT_PAD, self.editor.y + TEXT_PAD + shown_row as f32 * LINE_H)
    }
}

/// The window's layout for a window `width` by `height` points. `lead` is the width the traffic lights take at the
/// left of the top band; the buttons start after it.
pub fn layout(width: f32, height: f32, lead: f32) -> Layout {
    let width = width.max(0.0);
    let height = height.max(0.0);
    let button_y = (BAR_H - BUTTON_H) / 2.0;
    let run = R { x: lead + TEXT_PAD, y: button_y, w: BUTTON_W, h: BUTTON_H };
    let stop = R { x: run.x + run.w + TEXT_PAD, y: button_y, w: BUTTON_W, h: BUTTON_H };
    let status_y = (height - STATUS_H).max(BAR_H);
    let body_h = status_y - BAR_H;
    let editor_h = body_h * EDITOR_SHARE;
    let editor = R { x: 0.0, y: BAR_H, w: width, h: editor_h };
    let output_y = BAR_H + editor_h;
    let output = R { x: 0.0, y: output_y, w: width, h: status_y - output_y };
    let status = R { x: 0.0, y: status_y, w: width, h: (height - status_y).max(0.0) };
    Layout { width, height, run, stop, editor, output, status }
}

/// The first line the editor shows so that the caret's line `row` is in view, given the current first line
/// `scroll` and the number of lines that fit (`visible`).
pub fn scroll_to_row(scroll: usize, row: usize, visible: usize) -> usize {
    let visible = visible.max(1);
    if row < scroll {
        row
    } else if row >= scroll + visible {
        row + 1 - visible
    } else {
        scroll
    }
}

/// The first output line shown, so that the newest lines are the ones in view.
pub fn output_tail_start(total: usize, visible: usize) -> usize {
    total.saturating_sub(visible)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_buttons_sit_after_the_traffic_lights() {
        let l = layout(800.0, 600.0, 90.0);
        assert!(l.run.x >= 90.0);
        assert!(l.stop.x > l.run.x + l.run.w);
        assert_eq!(l.button_at(l.run.x + 1.0, l.run.y + 1.0), Some(Button::Run));
        assert_eq!(l.button_at(l.stop.x + 1.0, l.stop.y + 1.0), Some(Button::Stop));
        assert_eq!(l.button_at(10.0, 10.0), None);
    }

    #[test]
    fn the_panes_stack_between_the_bar_and_the_status_bar() {
        let l = layout(800.0, 600.0, 90.0);
        assert_eq!(l.editor.y, BAR_H);
        assert_eq!(l.output.y, l.editor.y + l.editor.h);
        assert_eq!(l.status.y, l.output.y + l.output.h);
        assert!((l.status.y + l.status.h - 600.0).abs() < 1e-3);
        assert!(l.editor.h > l.output.h);
    }

    #[test]
    fn the_editor_fits_whole_lines() {
        let l = layout(800.0, 600.0, 90.0);
        let n = l.editor_lines();
        assert!(n >= 1);
        assert!(2.0 * TEXT_PAD + n as f32 * LINE_H <= l.editor.h + 1e-3);
        assert!(l.output_lines() >= 1);
    }

    #[test]
    fn a_tiny_window_still_has_positive_panes() {
        let l = layout(10.0, 10.0, 90.0);
        assert!(l.editor.h >= 0.0 && l.output.h >= 0.0);
        assert!(l.editor_lines() >= 1);
        assert_eq!(l.output_lines(), 0);
    }

    #[test]
    fn scrolling_keeps_the_caret_row_in_view() {
        assert_eq!(scroll_to_row(0, 3, 10), 0);
        assert_eq!(scroll_to_row(5, 2, 10), 2);
        assert_eq!(scroll_to_row(0, 12, 10), 3);
        assert_eq!(scroll_to_row(4, 6, 10), 4);
        assert_eq!(scroll_to_row(0, 5, 0), 5);
    }

    #[test]
    fn the_output_tail_starts_so_the_newest_line_shows() {
        assert_eq!(output_tail_start(3, 10), 0);
        assert_eq!(output_tail_start(30, 10), 20);
    }
}
