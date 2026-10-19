//! File > Calendar Colour: a sheet that lists the colours a calendar can be given (`overlay::PALETTE`, each one of the
//! system colours by name) with a swatch and a check mark on the calendar's colour. A tap picks a row, Done (or Return)
//! gives the calendar that colour, Cancel (or Escape) keeps the old one. The colour is kept with the calendars
//! (`extras.rs`) and used by the calendar list's dot and by the calendar's events in every view.
use super::*;
use bifrost_app::controls::{Node, Rect, Role};

/// The first row's top and one row's height (points), as in the event form.
const TOP: f32 = 108.0;
const ROW: f32 = 44.0;
/// The swatch's diameter and the left edge of the names (points).
const SWATCH: f32 = 16.0;
const NAME_X: f32 = 68.0;

/// The open colour sheet: the calendar being coloured and the palette colour picked so far.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ColourSheet {
    pub cal: u32,
    pub pick: u8,
}

impl Calendar {
    /// File > Calendar Colour > a calendar: the sheet opens on that calendar's colour. False for a calendar that does not
    /// exist and while another sheet or the event form is open.
    pub fn open_colour(&mut self, cal: u32) -> bool {
        if !self.settings_enabled() || !self.extras.exists(cal) {
            return false;
        }
        self.colour_sheet = Some(ColourSheet { cal, pick: self.extras.colour_index(cal) });
        self.search_focused = false;
        self.error = None;
        self.notice = None;
        self.dirty = true;
        self.focus(None);
        true
    }

    /// The calendar and the colour the sheet shows, while it is open.
    pub fn colour_sheet(&self) -> Option<ColourSheet> {
        self.colour_sheet
    }

    /// The sheet's rows as (palette colour, top pt, height pt): one per colour in the palette.
    pub(super) fn colour_rows(&self) -> Vec<(u8, f32, f32)> {
        if self.colour_sheet.is_none() {
            return vec![];
        }
        (0..overlay::PALETTE.len() as u8).map(|i| (i, TOP + ROW * i as f32, ROW)).collect()
    }

    /// The row under window-column pixel `(x, y)`.
    pub(super) fn hit_colour(&self, ui: &Ui, _x: f32, y: f32) -> Option<Target> {
        let yp = y / ui.s;
        self.colour_rows().into_iter().find(|r| (r.1..r.1 + r.2).contains(&yp)).map(|r| Target::ColourPick(r.0))
    }

    /// A tap on a row: that colour is the pick.
    pub(super) fn colour_pick(&mut self, colour: u8) {
        if let Some(sheet) = self.colour_sheet.as_mut() {
            if colour < extras::PALETTE_LEN {
                sheet.pick = colour;
            }
        }
    }

    /// Up and Down move the pick through the palette without wrapping.
    pub(super) fn colour_step(&mut self, by: i32) {
        if let Some(sheet) = self.colour_sheet.as_mut() {
            let top = extras::PALETTE_LEN as i32 - 1;
            sheet.pick = (sheet.pick as i32 + by).clamp(0, top) as u8;
        }
    }

    /// Done: the calendar gets the picked colour and the sheet closes.
    pub(super) fn colour_done(&mut self) {
        let Some(sheet) = self.colour_sheet.take() else { return };
        if self.extras.set_colour(sheet.cal, sheet.pick) {
            self.save_extras();
            self.notice = Some(format!("{} is now {}", self.extras.name_of(sheet.cal), overlay::palette_name(sheet.pick)));
        }
        self.dirty = true;
    }

    /// The sheet: the calendar's name, then a row per colour with its swatch, its name and a check mark on the pick.
    pub(super) fn draw_colour(&mut self, s: &mut Surface, ui: &Ui) {
        let Some(sheet) = self.colour_sheet else { return };
        let w = self.w as f32;
        let rows = self.colour_rows();
        let (first, last) = (rows[0], rows[rows.len() - 1]);
        s.fill_rrect(ui.pt(SPACE_XL), ui.pt(first.1), w - ui.pt(SPACE_HUGE), ui.pt(last.1 + last.2 - first.1), ui.pt(CARD_R), CARD_BG.now());
        for r in &rows[1..] {
            s.fill_rrect(ui.pt(36.0), ui.pt(r.1), w - ui.pt(56.0), ui.pt(0.5).max(1.0), 0.0, SEPARATOR_C.now());
        }
        let name = self.extras.name_of(sheet.cal);
        self.centre(s, ui, &name, MAC_TITLE3_PT, Weight::Semibold, LABEL.now(), w / 2.0, ui.pt(84.0));
        for (i, top, h) in rows {
            let mid = ui.pt(top + h / 2.0);
            let d = ui.pt(SWATCH);
            s.fill_rrect(ui.pt(36.0), mid - d / 2.0, d, d, d / 2.0, overlay::palette_colour(i));
            self.text(s, ui, overlay::palette_name(i), MAC_BODY_PT, LABEL.now(), ui.pt(NAME_X), mid);
            if i == sheet.pick {
                self.right(s, ui, "\u{2713}", colors::LINK.now(), w - ui.pt(36.0), mid);
            }
        }
    }

    /// The sheet's rows for assistive technology: one button per colour, the picked one marked.
    pub(super) fn colour_nodes(&self, ui: &Ui) -> Vec<Node> {
        let Some(sheet) = self.colour_sheet else { return vec![] };
        let k = ui.pt(1.0);
        let (x0, wp) = (self.x0 / k, self.w as f32 / k);
        self.colour_rows()
            .into_iter()
            .map(|(i, top, h)| Node {
                id: 240 + i as u64,
                role: Role::Button,
                label: overlay::palette_name(i).into(),
                value: if i == sheet.pick { "selected".into() } else { String::new() },
                bounds: Rect { x: x0 + 36.0, y: top, w: wp - 72.0, h },
                enabled: true,
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_palette_has_no_red_and_every_colour_has_a_name_and_a_distinct_place() {
        assert!(!overlay::PALETTE.contains(&"Red"), "red is the today marker and the destructive colour");
        assert_eq!(overlay::PALETTE.len(), extras::PALETTE_LEN as usize);
        let mut names = overlay::PALETTE.to_vec();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), overlay::PALETTE.len());
        assert_eq!(overlay::palette_name(overlay::BLUE), "Blue");
        assert_eq!(overlay::palette_name(200), "Green", "a number past the palette is the first colour");
    }

    #[test]
    fn the_first_six_colours_keep_the_numbers_calendars_were_saved_with() {
        // the calendars saved before the palette grew used the order of the holidays' and subscriptions' cycle
        assert_eq!(&overlay::PALETTE[..6], &["Green", "Orange", "Purple", "Pink", "Teal", "Indigo"]);
        for slot in 0..6usize {
            assert_eq!(overlay::colour(slot), overlay::palette_colour(slot as u8));
        }
    }
}
