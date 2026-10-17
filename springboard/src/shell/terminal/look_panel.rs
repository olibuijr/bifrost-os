//! Profile Editor: the ANSI palette wells, the channels of the selected well, Bold as Bright, Allow Blink, the character
//! and line spacing and the cursor style. Every change goes to each tab and is saved in prefs (`terminal.look.*`).
use super::*;
use crate::term::look::{ansi_name, clamp_spacing, SPACING_MAX, SPACING_MIN, SPACING_STEP};
use bifrost_ui::values::Stepper;

/// Rows of the editor, top to bottom.
pub const RESET: usize = 0;
/// Rows 1..=16 are the ANSI wells, black first.
pub const WELL_FIRST: usize = 1;
pub const CHANNEL_R: usize = 17;
pub const CHANNEL_G: usize = 18;
pub const CHANNEL_B: usize = 19;
pub const BOLD_BRIGHT: usize = 20;
pub const ALLOW_BLINK: usize = 21;
pub const CHAR_SPACING: usize = 22;
pub const LINE_SPACING: usize = 23;
pub const CURSOR: usize = 24;
pub const ROWS: usize = 25;

/// Whether a row is one of the steppers (its right side holds the − and + buttons).
fn stepper_row(row: usize) -> bool {
    matches!(row, CHANNEL_R | CHANNEL_G | CHANNEL_B | CHAR_SPACING | LINE_SPACING)
}

/// The direction a tap on a stepper row asks for: + at the right edge, − just left of it, 0 elsewhere.
pub fn stepper_dir(row: usize, lx: f32, w: f32) -> i32 {
    if !stepper_row(row) {
        0
    } else if lx >= w - 48.0 {
        1
    } else if lx >= w - 96.0 {
        -1
    } else {
        0
    }
}

impl Shell {
    /// Opens the editor from More > Profile Editor.
    pub(super) fn terminal_open_look(&mut self, t: f64) {
        self.terminal_ui.panel = Panel::Look;
        self.terminal_ui.scroll = 0.0;
        self.terminal_ui.message.clear();
        self.set_keyboard(false, t);
    }

    /// Applies row `row` for a tap (`dir` 0) or a stepper press (`dir` ±1), then saves and applies the look.
    pub(super) fn terminal_look_row(&mut self, row: usize, dir: i32) {
        let theme = self.terminal_ui.settings.theme;
        let well = self.terminal_ui.look_well.min(15);
        match row {
            RESET => self.terminal_ui.look.ansi = None,
            WELL_FIRST..=16 => {
                // a tap selects the well; the channel steppers then edit it
                self.terminal_ui.look_well = row - WELL_FIRST;
                self.terminal_ui.message = format!("Editing {}", ansi_name(row - WELL_FIRST));
                self.dirty = true;
                return;
            }
            CHANNEL_R | CHANNEL_G | CHANNEL_B if dir != 0 => {
                let mut ansi = self.terminal_ui.look.palette(theme);
                let (r, g, b) = ansi[well];
                let channel = |v: u8| (v as f32 + dir as f32 * 8.0).clamp(0.0, 255.0).round() as u8;
                ansi[well] = match row {
                    CHANNEL_R => (channel(r), g, b),
                    CHANNEL_G => (r, channel(g), b),
                    _ => (r, g, channel(b)),
                };
                self.terminal_ui.look.ansi = Some(ansi);
            }
            BOLD_BRIGHT => self.terminal_ui.look.bold_bright = !self.terminal_ui.look.bold_bright,
            ALLOW_BLINK => self.terminal_ui.look.allow_blink = !self.terminal_ui.look.allow_blink,
            CHAR_SPACING | LINE_SPACING if dir != 0 => {
                let current = if row == CHAR_SPACING { self.terminal_ui.look.char_spacing } else { self.terminal_ui.look.line_spacing };
                // the kit stepper does the arithmetic and the clamp; the value then snaps to the step
                let mut stepper = Stepper::new(current, SPACING_MIN, SPACING_MAX, SPACING_STEP);
                stepper.nudge(dir);
                let value = clamp_spacing(stepper.value);
                if row == CHAR_SPACING {
                    self.terminal_ui.look.char_spacing = value;
                } else {
                    self.terminal_ui.look.line_spacing = value;
                }
            }
            CURSOR => {
                self.terminal_ui.cursor = self.terminal_ui.cursor.next();
                self.terminal_cursor_changed();
            }
            _ => return,
        }
        if row != CURSOR {
            self.terminal_look_changed();
        }
    }

    /// Sends the look to every tab, and saves it.
    fn terminal_look_changed(&mut self) {
        let look = self.terminal_ui.look;
        for term in self.term.iter_mut() {
            term.set_look(look);
        }
        self.terminal_ui.message = match look.saved().into_iter().try_for_each(|(k, v)| self.prefs.set(k, &v)) {
            Ok(()) => "Profile saved".into(),
            Err(e) => format!("Could not save profile: {e}"),
        };
        self.dirty = true;
    }

    /// A tap in the editor: `row` from the panel's list (pt rows below the title), `lx` and `w` in pt.
    pub(super) fn terminal_look_tap(&mut self, row: usize, lx: f32, w: f32) {
        if row >= ROWS {
            return;
        }
        let dir = stepper_dir(row, lx, w);
        self.terminal_look_row(row, dir);
    }

    /// Draws the editor rows in the panel body (`px`, `pw` and the body rows are px).
    pub(super) fn terminal_draw_look(&self, s: &mut Surface, ui: &Ui, px: f32, pw: f32, content_top: f32, content_bottom: f32) {
        let look = self.terminal_ui.look;
        let theme = self.terminal_ui.settings.theme;
        let palette = look.palette(theme);
        let well = self.terminal_ui.look_well.min(15);
        let mut list = s.rows(content_top as usize, content_bottom as usize);
        for row in 0..ROWS {
            let y = content_top + ui.pt(row as f32 * 44.0) - self.terminal_ui.scroll;
            if y + ui.pt(44.0) < content_top || y >= content_bottom {
                continue;
            }
            let (label, detail): (String, String) = match row {
                RESET => ("Reset palette to theme".into(), if look.ansi.is_none() { "Theme".into() } else { String::new() }),
                WELL_FIRST..=16 => (ansi_name(row - WELL_FIRST).into(), String::new()),
                CHANNEL_R => (format!("Red of {}", ansi_name(well)), format!("{}", palette[well].0)),
                CHANNEL_G => (format!("Green of {}", ansi_name(well)), format!("{}", palette[well].1)),
                CHANNEL_B => (format!("Blue of {}", ansi_name(well)), format!("{}", palette[well].2)),
                BOLD_BRIGHT => ("Bold as bright".into(), String::from(if look.bold_bright { "On" } else { "Off" })),
                ALLOW_BLINK => ("Allow blink".into(), String::from(if look.allow_blink { "On" } else { "Off" })),
                CHAR_SPACING => ("Character spacing".into(), format!("{:.1}", look.char_spacing)),
                LINE_SPACING => ("Line spacing".into(), format!("{:.1}", look.line_spacing)),
                _ => ("Cursor style".into(), self.terminal_ui.cursor.label().into()),
            };
            text(&mut list, ui, &label, px + ui.pt(16.0), y, pw - ui.pt(150.0), WHITE);
            if stepper_row(row) {
                text(&mut list, ui, &detail, px + pw - ui.pt(150.0), y, ui.pt(44.0), SECONDARY);
                let minus = (px + pw - ui.pt(96.0), y + ui.pt(6.0), ui.pt(44.0), ui.pt(32.0));
                let plus = (px + pw - ui.pt(48.0), y + ui.pt(6.0), ui.pt(40.0), ui.pt(32.0));
                button(&mut list, ui, "−", minus, false);
                button(&mut list, ui, "+", plus, false);
            } else if (WELL_FIRST..=16).contains(&row) {
                let (r, g, b) = palette[row - WELL_FIRST];
                let (sx, sy) = (px + pw - ui.pt(44.0), y + ui.pt(10.0));
                if row - WELL_FIRST == well {
                    list.fill_rrect(sx - ui.pt(3.0), sy - ui.pt(3.0), ui.pt(34.0), ui.pt(24.0), ui.pt(5.0), ACCENT);
                }
                list.fill_rrect(sx, sy, ui.pt(28.0), ui.pt(18.0), ui.pt(4.0), pack(r, g, b, 255));
            } else {
                text(&mut list, ui, &detail, px + pw - ui.pt(150.0), y, ui.pt(120.0), SECONDARY);
            }
            list.fill_rrect(px + ui.pt(16.0), y + ui.pt(44.0) - 1.0, pw - ui.pt(32.0), 1.0, 0.0, BADGE);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steppers_hit_the_minus_and_plus_at_the_right_edge() {
        assert_eq!(stepper_dir(CHAR_SPACING, 300.0, 320.0), 1);
        assert_eq!(stepper_dir(CHAR_SPACING, 280.0, 320.0), -1);
        assert_eq!(stepper_dir(CHAR_SPACING, 200.0, 320.0), 0);
        assert_eq!(stepper_dir(BOLD_BRIGHT, 300.0, 320.0), 0, "a toggle row has no steppers");
        assert_eq!(stepper_dir(WELL_FIRST, 300.0, 320.0), 0, "a well row has no steppers");
    }

    #[test]
    fn spacing_steps_within_the_range() {
        let mut sh = crate::shell::tests::terminal(&["/bin/sh", "-c", "read x"]);
        sh.terminal_look_row(CHAR_SPACING, 1);
        assert_eq!(sh.terminal_ui.look.char_spacing, 1.1);
        for _ in 0..30 {
            sh.terminal_look_row(LINE_SPACING, 1);
        }
        assert_eq!(sh.terminal_ui.look.line_spacing, SPACING_MAX);
        sh.terminal_look_row(LINE_SPACING, -1);
        assert_eq!(sh.terminal_ui.look.line_spacing, 1.9);
    }

    #[test]
    fn wells_select_then_channels_edit_only_that_colour() {
        let mut sh = crate::shell::tests::terminal(&["/bin/sh", "-c", "read x"]);
        let before = sh.terminal_ui.look.palette(sh.terminal_ui.settings.theme);
        sh.terminal_look_row(WELL_FIRST + 2, 0);
        assert_eq!(sh.terminal_ui.look_well, 2);
        sh.terminal_look_row(CHANNEL_R, 1);
        let after = sh.terminal_ui.look.palette(sh.terminal_ui.settings.theme);
        assert_eq!(after[2].0, before[2].0.saturating_add(8));
        assert_eq!(after[2].1, before[2].1);
        assert!(sh.terminal_ui.look.ansi.is_some());
        sh.terminal_look_row(RESET, 0);
        assert_eq!(sh.terminal_ui.look.ansi, None);
    }

    #[test]
    fn bold_as_bright_toggles_and_saves() {
        let mut sh = crate::shell::tests::terminal(&["/bin/sh", "-c", "read x"]);
        assert!(sh.terminal_ui.look.bold_bright);
        sh.terminal_look_row(BOLD_BRIGHT, 0);
        assert!(!sh.terminal_ui.look.bold_bright);
        assert_eq!(sh.terminal_ui.message, "Profile saved");
    }
}
