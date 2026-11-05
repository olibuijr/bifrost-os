//! A form sheet: labelled text fields, choices and lists in a sheet that hangs from the title bar, with an OK and a
//! Cancel button. The Rename, Erase, Restore, RAID and disk image sheets are forms. This module is the model, the
//! layout and the hit tests, all in points and all pure, so the drawing code (`draw.rs`) and the touch code read the
//! same rectangles and the logic is tested on the host. Typing goes to the focused text field; a choice is a
//! segmented control; a list is a column of rows that are on or off (one at a time when it is `single`).

use bifrost_app::controls::sheet::{self, ButtonRole, Hit, Sheet, SheetButton, SHEET_PAD_PT};
use bifrost_app::controls::{Rect, TextField};
use crate::partition::Segment;

/// The form sheet's buttons.
pub const FORM_CANCEL: u32 = 1;
pub const FORM_OK: u32 = 2;

/// The sheet's width, the width of the label column, and the height of a text field row (points).
pub const WIDTH_PT: f32 = 460.0;
pub const LABEL_PT: f32 = 110.0;
pub const ROW_PT: f32 = 32.0;
/// The height of a text field or a segmented control.
pub const FIELD_H_PT: f32 = 24.0;
/// One row of a list, and the most rows a list shows.
pub const LIST_ROW_PT: f32 = 22.0;
pub const LIST_MAX: usize = 10;
/// The height of one note line, and the room the panel's title takes above the first row (points).
pub const NOTE_PT: f32 = 15.0;
const TITLE_PT: f32 = 26.0;
/// Panel height that is not content: the padding, the title, the gap above the buttons and the buttons.
const CHROME_PT: f32 = SHEET_PAD_PT * 2.0 + TITLE_PT + 12.0 + sheet::BUTTON_H_PT;

/// What a row asks for.
#[derive(Clone, Debug)]
pub enum Control {
    /// A single-line text field. Typing past `limit` bytes is cut.
    Text { field: TextField, placeholder: String, limit: usize },
    /// One of several options, as a segmented control.
    Choice { options: Vec<String>, selected: usize },
    /// Rows that are on or off. With `single` a pick turns the others off.
    List { rows: Vec<String>, on: Vec<bool>, single: bool },
    /// A bar of proportional segments, such as a partition layout. It only shows; a press on it does nothing.
    Bar { segments: Vec<Segment> },
}

/// A label and its control.
#[derive(Clone, Debug)]
pub struct Row {
    pub label: String,
    pub control: Control,
}

/// What a point on the form is over.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FormHit {
    Nothing,
    Button(u32),
    /// The text field of this row.
    Text(usize),
    /// An option of a choice, or a row of a list: (row, index).
    Pick(usize, usize),
}

/// The rectangles of one row, in points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RowGeo {
    pub label: Rect,
    pub control: Rect,
}

/// The rectangles of the whole form.
#[derive(Clone, Debug, PartialEq)]
pub struct Geo {
    pub panel: Rect,
    pub rows: Vec<RowGeo>,
    /// The top of the first note line.
    pub notes_y: f32,
}

/// The form.
#[derive(Clone, Debug)]
pub struct Form {
    pub title: String,
    pub rows: Vec<Row>,
    /// The row whose text field takes typing.
    pub focus: Option<usize>,
    /// The button Tab reached after the last text field (a `FORM_*` id); the sheet draws its focus ring and Space presses
    /// it. While it is set, typing goes nowhere.
    pub button_focus: Option<u32>,
    /// Lines under the rows, in footnote size.
    pub notes: Vec<String>,
    /// A refusal, shown in red under the notes.
    pub error: Option<String>,
    pub ok_title: String,
    /// The OK button is drawn as a destructive action.
    pub destructive: bool,
}

impl Form {
    pub fn new(title: &str, ok_title: &str, destructive: bool) -> Form {
        Form { title: title.to_string(), rows: Vec::new(), focus: None, button_focus: None, notes: Vec::new(), error: None, ok_title: ok_title.to_string(), destructive }
    }

    /// Adds a text field. The first one takes the focus.
    pub fn text(mut self, label: &str, initial: &str, placeholder: &str, limit: usize) -> Form {
        let mut field = TextField::new();
        field.push_str(initial);
        if self.focus.is_none() {
            self.focus = Some(self.rows.len());
        }
        self.rows.push(Row { label: label.to_string(), control: Control::Text { field, placeholder: placeholder.to_string(), limit } });
        self
    }

    /// Adds a choice. `selected` is cut to the last option.
    pub fn choice(mut self, label: &str, options: &[&str], selected: usize) -> Form {
        let selected = selected.min(options.len().saturating_sub(1));
        self.rows.push(Row { label: label.to_string(), control: Control::Choice { options: options.iter().map(|o| o.to_string()).collect(), selected } });
        self
    }

    /// Adds a list of at most `LIST_MAX` rows, all off except the first of a `single` list.
    pub fn list(mut self, label: &str, rows: Vec<String>, single: bool) -> Form {
        let rows: Vec<String> = rows.into_iter().take(LIST_MAX).collect();
        let on = (0..rows.len()).map(|i| single && i == 0).collect();
        self.rows.push(Row { label: label.to_string(), control: Control::List { rows, on, single } });
        self
    }

    /// Adds a bar of proportional segments.
    pub fn bar(mut self, label: &str, segments: Vec<Segment>) -> Form {
        self.rows.push(Row { label: label.to_string(), control: Control::Bar { segments } });
        self
    }

    /// Replaces the segments of the bar in `row`. True when the row is a bar.
    pub fn set_bar(&mut self, row: usize, segments: Vec<Segment>) -> bool {
        match self.rows.get_mut(row).map(|r| &mut r.control) {
            Some(Control::Bar { segments: shown }) => {
                *shown = segments;
                true
            }
            _ => false,
        }
    }

    pub fn note(mut self, line: &str) -> Form {
        self.notes.push(line.to_string());
        self
    }

    /// The height of the controls of row `i`, including the gap under it.
    fn row_h(&self, i: usize) -> f32 {
        match &self.rows[i].control {
            Control::List { rows, .. } => rows.len() as f32 * LIST_ROW_PT + 10.0,
            _ => ROW_PT,
        }
    }

    /// The height of everything between the title and the buttons.
    pub fn content_h(&self) -> f32 {
        let rows: f32 = (0..self.rows.len()).map(|i| self.row_h(i)).sum();
        let lines = self.notes.len() + usize::from(self.error.is_some());
        rows + if lines == 0 { 0.0 } else { 4.0 + lines as f32 * NOTE_PT }
    }

    /// The sheet that frames the form: its size follows the rows, and OK is the first (rightmost) button.
    pub fn sheet(&self) -> Sheet {
        let role = if self.destructive { ButtonRole::Destructive } else { ButtonRole::Default };
        let buttons = vec![SheetButton::new(FORM_OK, &self.ok_title, role), SheetButton::new(FORM_CANCEL, "Cancel", ButtonRole::Cancel)];
        Sheet::new(&self.title, (WIDTH_PT, self.content_h() + CHROME_PT), buttons).with_focus(self.button_focus)
    }

    /// The rectangles for a sheet hung from the title bar (`titlebar_h` tall) of `parent`.
    pub fn geometry(&self, parent: Rect, titlebar_h: f32) -> Geo {
        let panel = self.sheet().layout(parent, titlebar_h, 1.0).panel;
        let x = panel.x + SHEET_PAD_PT;
        let w = (panel.w - SHEET_PAD_PT * 2.0).max(0.0);
        let mut y = panel.y + SHEET_PAD_PT + TITLE_PT;
        let mut rows = Vec::with_capacity(self.rows.len());
        for (i, row) in self.rows.iter().enumerate() {
            let h = self.row_h(i);
            let label = Rect { x, y, w: LABEL_PT, h: ROW_PT };
            let control = match &row.control {
                Control::List { rows, .. } => Rect { x: x + LABEL_PT, y: y + 4.0, w: (w - LABEL_PT).max(0.0), h: rows.len() as f32 * LIST_ROW_PT },
                _ => Rect { x: x + LABEL_PT, y: y + (ROW_PT - FIELD_H_PT) / 2.0, w: (w - LABEL_PT).max(0.0), h: FIELD_H_PT },
            };
            rows.push(RowGeo { label, control });
            y += h;
        }
        Geo { panel, rows, notes_y: y + 4.0 }
    }

    /// What the point is over. A point on the dimmed window or the panel's padding is `Nothing`.
    pub fn hit(&self, parent: Rect, titlebar_h: f32, x: f32, y: f32) -> FormHit {
        match self.sheet().hit(parent, titlebar_h, 1.0, x, y) {
            Hit::Button(id) => return FormHit::Button(id),
            Hit::Panel => {}
            _ => return FormHit::Nothing,
        }
        let geo = self.geometry(parent, titlebar_h);
        for (i, (row, g)) in self.rows.iter().zip(&geo.rows).enumerate() {
            if !g.control.contains(x, y) {
                continue;
            }
            return match &row.control {
                Control::Text { .. } => FormHit::Text(i),
                Control::Choice { options, .. } => sheet::segment_hit(g.control, options.len(), x, y).map_or(FormHit::Nothing, |k| FormHit::Pick(i, k)),
                Control::Bar { .. } => FormHit::Nothing,
                Control::List { rows, .. } => {
                    let k = ((y - g.control.y) / LIST_ROW_PT) as usize;
                    if k < rows.len() {
                        FormHit::Pick(i, k)
                    } else {
                        FormHit::Nothing
                    }
                }
            };
        }
        FormHit::Nothing
    }

    /// Whether the form has a text field to type in, so the keyboard should be up.
    pub fn takes_text(&self) -> bool {
        self.focus.is_some() && self.button_focus.is_none()
    }

    /// Gives the focus to the text field of `row`. False when the row has none.
    pub fn focus_row(&mut self, row: usize) -> bool {
        if matches!(self.rows.get(row).map(|r| &r.control), Some(Control::Text { .. })) {
            self.focus = Some(row);
            true
        } else {
            false
        }
    }

    /// Moves the focus to the next text field, wrapping round. False when there is only one or none.
    pub fn focus_next(&mut self) -> bool {
        let texts: Vec<usize> = (0..self.rows.len()).filter(|&i| matches!(self.rows[i].control, Control::Text { .. })).collect();
        let Some(current) = self.focus else { return false };
        if texts.len() < 2 {
            return false;
        }
        let at = texts.iter().position(|&i| i == current).unwrap_or(0);
        self.focus = Some(texts[(at + 1) % texts.len()]);
        true
    }

    /// Tab: the next text field; after the last one the focus ring walks the buttons (in the order they are drawn), and
    /// after the last button it returns to the first text field. A form with no text field has only the buttons.
    pub fn tab(&mut self) -> bool {
        let sheet = self.sheet();
        let texts: Vec<usize> = (0..self.rows.len()).filter(|&i| matches!(self.rows[i].control, Control::Text { .. })).collect();
        let first = sheet.next_focus(None, false);
        if let Some(b) = self.button_focus {
            let next = sheet.next_focus(Some(b), false);
            if next == first && !texts.is_empty() {
                self.button_focus = None;
                self.focus = texts.first().copied();
            } else {
                self.button_focus = next;
            }
            return true;
        }
        if self.focus.is_some() && self.focus != texts.last().copied() {
            return self.focus_next();
        }
        self.button_focus = first;
        first.is_some()
    }

    /// Typed text goes to the focused field. True when the form changed.
    pub fn type_text(&mut self, text: &str) -> bool {
        if self.button_focus.is_some() {
            return false;
        }
        let Some(row) = self.focus else { return false };
        let Some(Control::Text { field, limit, .. }) = self.rows.get_mut(row).map(|r| &mut r.control) else { return false };
        let before = field.len();
        field.push_str(text);
        while field.len() > *limit {
            field.pop();
        }
        self.error = None;
        field.len() != before
    }

    /// Backspace in the focused field. True when the form changed.
    pub fn backspace(&mut self) -> bool {
        if self.button_focus.is_some() {
            return false;
        }
        let Some(row) = self.focus else { return false };
        let Some(Control::Text { field, .. }) = self.rows.get_mut(row).map(|r| &mut r.control) else { return false };
        let before = field.len();
        field.pop();
        self.error = None;
        field.len() != before
    }

    /// A pick of option or row `index` in `row`. True when the form changed.
    pub fn pick(&mut self, row: usize, index: usize) -> bool {
        let Some(r) = self.rows.get_mut(row) else { return false };
        self.error = None;
        match &mut r.control {
            Control::Choice { options, selected } if index < options.len() => {
                let changed = *selected != index;
                *selected = index;
                changed
            }
            Control::List { on, single, .. } if index < on.len() => {
                if *single {
                    on.iter_mut().enumerate().for_each(|(i, v)| *v = i == index);
                } else {
                    on[index] = !on[index];
                }
                true
            }
            _ => false,
        }
    }

    /// The text of the field in `row`, trimmed. Empty for a row that is not a text field.
    pub fn value(&self, row: usize) -> String {
        match self.rows.get(row).map(|r| &r.control) {
            Some(Control::Text { field, .. }) => field.trim().to_string(),
            _ => String::new(),
        }
    }

    /// The selected option of the choice in `row`.
    pub fn selected(&self, row: usize) -> usize {
        match self.rows.get(row).map(|r| &r.control) {
            Some(Control::Choice { selected, .. }) => *selected,
            _ => 0,
        }
    }

    /// The rows of the list in `row` that are on.
    pub fn picked(&self, row: usize) -> Vec<usize> {
        match self.rows.get(row).map(|r| &r.control) {
            Some(Control::List { on, .. }) => on.iter().enumerate().filter(|(_, v)| **v).map(|(i, _)| i).collect(),
            _ => Vec::new(),
        }
    }

    pub fn set_error(&mut self, sentence: &str) {
        self.error = Some(sentence.to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window() -> Rect {
        Rect { x: 0.0, y: 0.0, w: 1200.0, h: 780.0 }
    }

    fn bar() -> f32 {
        bifrost_app::layout::UNIFIED_BAR_PT
    }

    fn sample() -> Form {
        Form::new("Erase sdb?", "Erase", true)
            .text("Name", "Games", "Untitled", 8)
            .choice("Format", &["ext4", "exFAT", "FAT32"], 1)
            .text("Other", "", "", 8)
            .list("Source", vec!["sdb1".into(), "sdc1".into(), "sdd1".into()], true)
            .note("Everything on it is erased.")
    }

    #[test]
    fn the_first_text_field_takes_the_focus_and_typing_is_cut_at_the_limit() {
        let mut f = sample();
        assert_eq!(f.focus, Some(0));
        assert!(f.takes_text());
        assert!(f.type_text("12345"));
        assert_eq!(f.value(0), "Games123");
        assert!(!f.type_text("9"), "the field is full");
        assert!(f.backspace());
        assert_eq!(f.value(0), "Games12");
    }

    #[test]
    fn tab_moves_between_text_fields_and_a_click_focuses_one() {
        let mut f = sample();
        assert!(f.focus_next());
        assert_eq!(f.focus, Some(2));
        assert!(f.focus_next());
        assert_eq!(f.focus, Some(0));
        assert!(!f.focus_row(1), "a choice takes no typing");
        assert!(f.focus_row(2));
        let single = Form::new("t", "OK", false).text("Only", "", "", 4);
        assert!(!single.clone().focus_next());
        assert!(!Form::new("t", "OK", false).takes_text());
    }

    #[test]
    fn a_choice_selects_one_option_and_clamps_its_start() {
        let mut f = sample();
        assert_eq!(f.selected(1), 1);
        assert!(f.pick(1, 2));
        assert!(!f.pick(1, 2));
        assert!(!f.pick(1, 9));
        assert_eq!(f.selected(1), 2);
        assert_eq!(Form::new("t", "OK", false).choice("c", &["a", "b"], 7).selected(0), 1);
    }

    #[test]
    fn a_single_list_keeps_one_row_on_and_a_multi_list_toggles() {
        let mut f = sample();
        assert_eq!(f.picked(3), vec![0]);
        assert!(f.pick(3, 2));
        assert_eq!(f.picked(3), vec![2]);
        let mut multi = Form::new("t", "OK", false).list("Members", vec!["a".into(), "b".into(), "c".into()], false);
        assert!(multi.picked(0).is_empty());
        multi.pick(0, 0);
        multi.pick(0, 2);
        assert_eq!(multi.picked(0), vec![0, 2]);
        multi.pick(0, 0);
        assert_eq!(multi.picked(0), vec![2]);
    }

    #[test]
    fn a_list_shows_at_most_ten_rows() {
        let rows: Vec<String> = (0..25).map(|i| format!("sd{i}")).collect();
        let f = Form::new("t", "OK", false).list("Members", rows, false);
        match &f.rows[0].control {
            Control::List { rows, on, .. } => assert!(rows.len() == LIST_MAX && on.len() == LIST_MAX),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn the_sheet_grows_with_its_rows_and_notes() {
        let plain = Form::new("t", "OK", false).text("a", "", "", 4);
        let with_note = plain.clone().note("one").note("two");
        assert!(with_note.sheet().size.1 > plain.sheet().size.1);
        let mut red = plain.clone();
        red.set_error("It does not fit.");
        assert!(red.sheet().size.1 > plain.sheet().size.1);
        assert_eq!(plain.sheet().size.0, WIDTH_PT);
        let ids: Vec<u32> = sample().sheet().buttons.iter().map(|b| b.id).collect();
        assert_eq!(ids, vec![FORM_OK, FORM_CANCEL]);
    }

    #[test]
    fn rows_are_stacked_below_the_title_inside_the_panel() {
        let f = sample();
        let geo = f.geometry(window(), bar());
        assert_eq!(geo.rows.len(), 4);
        for pair in geo.rows.windows(2) {
            assert!(pair[1].label.y > pair[0].label.y);
        }
        for g in &geo.rows {
            assert!(g.control.x >= geo.panel.x && g.control.x + g.control.w <= geo.panel.x + geo.panel.w + 0.5);
            assert!(g.control.y >= geo.panel.y && g.control.y + g.control.h <= geo.panel.y + geo.panel.h);
        }
        assert!(geo.notes_y < geo.panel.y + geo.panel.h);
    }

    fn centre(r: Rect) -> (f32, f32) {
        (r.x + r.w / 2.0, r.y + r.h / 2.0)
    }

    #[test]
    fn a_point_over_a_control_hits_that_control() {
        let f = sample();
        let geo = f.geometry(window(), bar());
        let (x, y) = centre(geo.rows[0].control);
        assert_eq!(f.hit(window(), bar(), x, y), FormHit::Text(0));
        let seg = sheet::segments(geo.rows[1].control, 3);
        let (x, y) = centre(seg[2]);
        assert_eq!(f.hit(window(), bar(), x, y), FormHit::Pick(1, 2));
        let list = geo.rows[3].control;
        assert_eq!(f.hit(window(), bar(), list.x + 20.0, list.y + LIST_ROW_PT * 1.5), FormHit::Pick(3, 1));
        assert_eq!(f.hit(window(), bar(), list.x + 20.0, list.y + LIST_ROW_PT * 3.5), FormHit::Nothing);
        let ok = f.sheet().layout(window(), bar(), 1.0).buttons.iter().find(|(id, _)| *id == FORM_OK).map(|(_, r)| *r).expect("ok button");
        let (x, y) = centre(ok);
        assert_eq!(f.hit(window(), bar(), x, y), FormHit::Button(FORM_OK));
    }

    #[test]
    fn the_dimmed_window_and_the_labels_hit_nothing() {
        let f = sample();
        assert_eq!(f.hit(window(), bar(), 5.0, 400.0), FormHit::Nothing);
        let geo = f.geometry(window(), bar());
        let (x, y) = centre(geo.rows[0].label);
        assert_eq!(f.hit(window(), bar(), x, y), FormHit::Nothing);
    }

    #[test]
    fn a_bar_row_shows_segments_and_takes_no_press() {
        let seg = |f: f32| Segment { fraction: f, caption: String::new(), free: false };
        let mut f = Form::new("t", "OK", false).bar("Layout", vec![seg(0.5)]).text("Sizes", "", "", 8);
        assert_eq!(f.focus, Some(1), "the bar is not a text field");
        assert!(f.set_bar(0, vec![seg(0.25), seg(0.25)]));
        assert!(!f.set_bar(1, Vec::new()), "a text row is not a bar");
        match &f.rows[0].control {
            Control::Bar { segments } => assert_eq!(segments.len(), 2),
            other => panic!("{other:?}"),
        }
        let geo = f.geometry(window(), bar());
        let (x, y) = centre(geo.rows[0].control);
        assert_eq!(f.hit(window(), bar(), x, y), FormHit::Nothing);
        assert!(!f.pick(0, 0));
    }

    #[test]
    fn editing_clears_the_refusal() {
        let mut f = sample();
        f.set_error("The name is not valid.");
        assert!(f.error.is_some());
        f.type_text("x");
        assert!(f.error.is_none());
    }
}
