//! Empty Trash: the confirmation sheet for Recently Deleted (the background menu's Empty Trash, shown only in the bin). Return
//! or the red Empty Trash button deletes every item for good as a job on the worker, like Move to Trash; Escape or Cancel
//! closes the sheet. While the sheet is open it takes every key and touch, so the folder behind does not change.
use crate::*;
use bifrost_app::kit::ui::DESTRUCTIVE;

/// The sheet's height, its button size and the gap from its right edge (points).
const SHEET_H: f32 = 124.;
const BUTTON_W: f32 = 104.;
const BUTTON_H: f32 = 26.;
const PAD: f32 = 20.;
const BUTTON_GAP: f32 = 8.;
/// Where a touch on the sheet landed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Hit {
    Empty,
    Cancel,
    /// On the sheet but on no button: swallowed.
    Card,
}

/// The sheet's rectangle `(x, y, w)` in points, centred over the content area like the other sheets.
pub(crate) fn sheet_frame(window_w: f32, side: f32) -> (f32, f32, f32) {
    let w = (window_w - side - 32.).clamp(240., 440.);
    (side + (window_w - side - w) / 2., 72., w)
}

/// The two buttons' rectangles `(cancel, empty)` for a sheet frame `(x, y, w)`.
pub(crate) fn button_frames(x: f32, y: f32, w: f32) -> ((f32, f32, f32, f32), (f32, f32, f32, f32)) {
    let by = y + SHEET_H - PAD - BUTTON_H;
    let empty_x = x + w - PAD - BUTTON_W;
    let cancel_x = empty_x - BUTTON_GAP - BUTTON_W;
    ((cancel_x, by, BUTTON_W, BUTTON_H), (empty_x, by, BUTTON_W, BUTTON_H))
}

/// True when the point is inside the rectangle `(x, y, w, h)`.
fn inside(r: (f32, f32, f32, f32), x: f32, y: f32) -> bool {
    x >= r.0 && x < r.0 + r.2 && y >= r.1 && y < r.1 + r.3
}

/// Which part of the sheet a point (in points) is on.
pub(crate) fn hit(window_w: f32, side: f32, x: f32, y: f32) -> Option<Hit> {
    let (fx, fy, fw) = sheet_frame(window_w, side);
    if !(x >= fx && x < fx + fw && y >= fy && y < fy + SHEET_H) {
        return None;
    }
    let (cancel, empty) = button_frames(fx, fy, fw);
    Some(if inside(empty, x, y) {
        Hit::Empty
    } else if inside(cancel, x, y) {
        Hit::Cancel
    } else {
        Hit::Card
    })
}

impl Files {
    /// File > Empty Trash in Recently Deleted: asks first. An empty bin or a running job has nothing to ask about.
    pub(crate) fn ask_empty_trash(&mut self) {
        if !self.in_bin() {
            return;
        }
        if self.work.is_some() {
            self.notice = Some("Wait for the current job to finish".into());
            return;
        }
        self.fx.empty_trash = true;
        self.revision += 1;
    }

    /// Deletes the bin's items for good, as a job. Closes the sheet.
    pub(crate) fn empty_trash_now(&mut self) {
        self.fx.empty_trash = false;
        // the job itself is `ops::empty_trash` (also behind the tests); the sheet only asks first
        self.empty_trash();
        self.revision += 1;
    }

    /// Keys and touches while the sheet is open. Returns false when no sheet is open.
    pub(crate) fn empty_trash_input(&mut self, ctx: &Ctx, input: &Input) -> bool {
        if !self.fx.empty_trash {
            return false;
        }
        match input {
            Input::Key(KeyInput::Code(13)) => self.empty_trash_now(),
            Input::Key(KeyInput::Code(27)) => {
                self.fx.empty_trash = false;
                self.revision += 1;
            }
            Input::Touch { kind: TouchKind::Up, x, y, .. } => {
                let (x, y) = (x / ctx.ui.s, y / ctx.ui.s);
                match hit(self.w, self.side(), x, y) {
                    Some(Hit::Empty) => self.empty_trash_now(),
                    Some(Hit::Cancel) => {
                        self.fx.empty_trash = false;
                        self.revision += 1;
                    }
                    _ => {}
                }
            }
            _ => {}
        }
        true
    }

    /// The Empty Trash sheet, drawn over the content area with the other sheets.
    pub(crate) fn paint_empty_trash(&self, ui: &Ui, s: &mut Surface) {
        if !self.fx.empty_trash {
            return;
        }
        let (x, y, w) = sheet_frame(self.w, self.side());
        rect(s, ui, x, y, w, SHEET_H, CARD_BG.now());
        text(s, ui, "Empty the Trash?", MAC_TITLE3_PT, LABEL.now(), x + PAD, y + 16., w - 2. * PAD);
        text(s, ui, "The items in the Trash will be deleted for good. This can't be undone.", bifrost_app::kit::ui::MAC_SUBHEAD_PT, LABEL_2.now(), x + PAD, y + 46., w - 2. * PAD);
        let (cancel, empty) = button_frames(x, y, w);
        rect(s, ui, cancel.0, cancel.1, cancel.2, cancel.3, SEPARATOR_C.now());
        text(s, ui, "Cancel", bifrost_app::kit::ui::MAC_SUBHEAD_PT, LABEL.now(), cancel.0 + 12., cancel.1 + 5., cancel.2 - 24.);
        rect(s, ui, empty.0, empty.1, empty.2, empty.3, DESTRUCTIVE);
        text(s, ui, "Empty Trash", bifrost_app::kit::ui::MAC_SUBHEAD_PT, WHITE, empty.0 + 12., empty.1 + 5., empty.2 - 24.);
        text(s, ui, "Return empties the Trash. Escape cancels.", bifrost_app::kit::ui::MAC_SUBHEAD_PT, LABEL_2.now(), x + PAD, y + SHEET_H - 26. - 40., w - 2. * PAD);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sheet_is_centred_and_fits_the_window() {
        let (x, y, w) = sheet_frame(1000., 200.);
        assert_eq!(w, 440.);
        assert!((x - (200. + (1000. - 200. - 440.) / 2.)).abs() < 1e-3);
        assert_eq!(y, 72.);
        // a narrow window clamps to the minimum width
        assert_eq!(sheet_frame(300., 200.).2, 240.);
    }

    #[test]
    fn buttons_sit_inside_the_card_and_do_not_overlap() {
        let (x, y, w) = sheet_frame(1000., 200.);
        let (cancel, empty) = button_frames(x, y, w);
        assert!(cancel.0 >= x && empty.0 + empty.2 <= x + w, "both buttons are on the card");
        assert!(cancel.0 + cancel.2 < empty.0, "Cancel ends before Empty Trash starts");
        assert!(empty.1 + empty.3 <= y + SHEET_H);
    }

    #[test]
    fn touches_map_to_the_buttons() {
        let (side, win) = (200., 1000.);
        let (x, y, w) = sheet_frame(win, side);
        let (cancel, empty) = button_frames(x, y, w);
        assert_eq!(hit(win, side, empty.0 + 2., empty.1 + 2.), Some(Hit::Empty));
        assert_eq!(hit(win, side, cancel.0 + 2., cancel.1 + 2.), Some(Hit::Cancel));
        assert_eq!(hit(win, side, x + 2., y + 2.), Some(Hit::Card));
        assert_eq!(hit(win, side, x - 5., y + 2.), None, "outside the sheet is not on it");
        assert_eq!(hit(win, side, x + 2., y + SHEET_H + 1.), None);
    }

    #[test]
    fn the_trash_sheet_starts_closed() {
        assert!(!FileManagerState::default().empty_trash);
    }
}
