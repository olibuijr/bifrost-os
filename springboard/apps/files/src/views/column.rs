//! Column view (Files's browser view): the folders above the open one, then its rows.
use crate::*;
use springboard::shell::ui::SELECTION_INACTIVE;

/// Columns view: the width of one column, the row pitch, and the folders shown above the open one.
pub(crate) const COLUMN_W: f32 = 200.;

pub(crate) const ROW_H: f32 = 28.;

/// How a column row is drawn. Keyboard focus lives in the open folder's column (the last one), so only its selected
/// rows sit on the accent plate, with white text and icon. The row that leads to the next column in a folder above it
/// is also selected, but on the neutral inactive plate with the normal text, as in the reference.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RowState {
    Plain,
    /// Selected in the focused column: accent plate, white text and icon.
    Active,
    /// Selected in a column without focus: neutral plate, normal text.
    Inactive,
}

impl RowState {
    /// The state of a row from whether it is selected and whether its column has keyboard focus.
    pub(crate) fn of(selected: bool, focused_column: bool) -> RowState {
        match (selected, focused_column) {
            (false, _) => RowState::Plain,
            (true, true) => RowState::Active,
            (true, false) => RowState::Inactive,
        }
    }
    /// The plate behind the row, if it has one.
    pub(crate) fn plate(self) -> Option<u32> {
        match self {
            RowState::Plain => None,
            RowState::Active => Some(SELECTION.now()),
            RowState::Inactive => Some(SELECTION_INACTIVE.now()),
        }
    }
    /// The name's colour: white on the accent plate, the scheme's label otherwise.
    pub(crate) fn ink(self) -> u32 {
        if self == RowState::Active { WHITE } else { LABEL.now() }
    }
    /// The folder chevron's colour: white on the accent plate, secondary grey otherwise.
    pub(crate) fn chevron(self) -> u32 {
        if self == RowState::Active { WHITE } else { LABEL_2.now() }
    }
}

/// One row of a column: icon, name, and a chevron for folders. The plate behind a selected row is drawn by the caller.
pub(crate) fn column_row(s: &mut Surface, ui: &Ui, e: &Entry, x0: f32, y: f32, cw: f32, state: RowState) {
    let icon = (ui.pt(x0 + 8.), ui.pt(y + 5.), ui.pt(SPACE_XL), ui.pt(SPACE_XL));
    if state == RowState::Active {
        applications::entry_icon_on_accent(s, icon, e);
    } else {
        applications::entry_icon(s, icon, e);
    }
    text(s, ui, &e.name, 13., state.ink(), x0 + 32., y + 6., cw - 56.);
    if e.is_dir {
        text(s, ui, "›", 16., state.chevron(), x0 + cw - 16., y + 3., 12.);
    }
}

impl Files {
    /// How many folders above the open one fit as columns beside it.
    pub(crate) fn ancestor_shown(&self) -> usize {
        let fit = ((self.w - (self.side() + 16.)) / self.vo.col_w).floor().max(1.) as usize;
        self.ancestors.len().min(fit - 1)
    }
    /// The rows of the `index`th ancestor column, with hidden files left out as the list is.
    pub(crate) fn ancestor_rows(&self, index: usize) -> Vec<&Entry> {
        self.ancestors[index].1.iter().filter(|e| self.show_hidden || !e.name.starts_with('.')).collect()
    }
    /// The folder a click in an ancestor column opens, when that row is a folder.
    pub(crate) fn ancestor_hit(&self, x: f32, y: f32) -> Option<PathBuf> {
        if !self.columns || x < (self.side() + 16.) || y < self.rows_top() || y > self.bottom() {
            return None;
        }
        let n = self.ancestor_shown();
        let col = ((x - (self.side() + 16.)) / self.vo.col_w).floor() as usize;
        if col >= n {
            return None;
        }
        let row = ((y - self.rows_top()) / ROW_H).floor() as usize;
        let index = self.ancestors.len() - n + col;
        self.ancestor_rows(index).get(row).filter(|e| e.is_dir).map(|e| e.path.clone())
    }
    /// The columns view: the folders above the open one, then the open folder's rows.
    pub(crate) fn paint_columns(&self, ui: &Ui, s: &mut Surface, visible: &[usize]) {
        let w = self.w;
        let n = self.ancestor_shown();
        let first = self.ancestors.len() - n;
        let bottom = self.bottom();
        for c in 0..n {
            let x0 = (self.side() + 16.) + c as f32 * self.vo.col_w;
            rect(s, ui, x0, self.body_top(), 1., bottom - self.body_top(), SEPARATOR_C.now());
            // the row leading to the next column is highlighted
            let next = if c + 1 < n { &self.ancestors[first + c + 1].0 } else { &self.path };
            for (k, e) in self.ancestor_rows(first + c).into_iter().enumerate() {
                let y = self.rows_top() + k as f32 * ROW_H;
                if y + ROW_H > bottom {
                    break;
                }
                // a folder above the open one has no keyboard focus: its row toward the next column is the inactive plate
                let state = RowState::of(e.path == *next, false);
                if let Some(plate) = state.plate() {
                    rect(s, ui, x0, y, self.vo.col_w, ROW_H, plate);
                }
                column_row(s, ui, e, x0, y, self.vo.col_w, state);
            }
        }
        let x0 = (self.side() + 16.) + n as f32 * self.vo.col_w;
        rect(s, ui, x0, self.body_top(), 1., bottom - self.body_top(), SEPARATOR_C.now());
        for (k, &i) in visible.iter().enumerate() {
            let y = self.rows_top() + k as f32 * ROW_H;
            if y + ROW_H > bottom {
                break;
            }
            let e = &self.rows[i];
            // the open folder's column has the keyboard focus: its selection is the accent plate
            let state = RowState::of(self.selection.contains(&i), true);
            if let Some(plate) = state.plate() {
                rect(s, ui, x0, y, w - x0, ROW_H, plate);
            }
            // the item being renamed shows its name field in place of the name
            if !self.paint_column_name_field(ui, s, e, x0, y) {
                column_row(s, ui, e, x0, y, self.vo.col_w, state);
            }
            self.paint_badges(ui, s, &e.path, x0 + 8. + SPACE_XL - 2., y + 13.);
        }
        self.paint_column_preview(ui, s, x0 + self.vo.col_w);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn font_ui() -> Ui {
        let font = springboard::font::Font::parse(Box::leak(std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/SFUI.ttf")).unwrap().into_boxed_slice())).unwrap();
        Ui::new(font, 1.25)
    }

    #[test]
    fn only_the_focused_columns_selection_is_the_accent_plate_with_white_text() {
        assert_eq!(RowState::of(false, true), RowState::Plain);
        assert_eq!(RowState::of(false, false), RowState::Plain);
        assert_eq!(RowState::of(true, true), RowState::Active);
        assert_eq!(RowState::of(true, false), RowState::Inactive);
        assert_eq!(RowState::Plain.plate(), None);
        assert_eq!(RowState::Active.plate(), Some(SELECTION.now()));
        assert_eq!(RowState::Inactive.plate(), Some(SELECTION_INACTIVE.now()));
        assert_eq!(RowState::Active.ink(), WHITE, "white on the accent plate");
        assert_eq!(RowState::Active.chevron(), WHITE);
        assert_eq!(RowState::Inactive.ink(), LABEL.now(), "the normal text on the grey plate");
        assert_eq!(RowState::Plain.ink(), LABEL.now());
        assert_eq!(RowState::Inactive.chevron(), LABEL_2.now());
        // the inactive plate is neither the accent nor the text colour, in both schemes
        for (plate, accent, label) in [(SELECTION_INACTIVE.dark, SELECTION.dark, LABEL.dark), (SELECTION_INACTIVE.light, SELECTION.light, LABEL.light)] {
            assert_ne!(plate, accent);
            assert_ne!(plate, label);
        }
    }

    #[test]
    fn a_path_row_is_grey_and_the_open_columns_selection_is_the_accent() {
        let ui = font_ui();
        let mut files = crate::tests::fixture(vec![crate::tests::folder("inside"), crate::tests::folder("other")]);
        files.columns = true;
        files.grid = false;
        files.path = PathBuf::from("/fixture/a");
        files.ancestors = vec![(PathBuf::from("/fixture"), vec![crate::tests::folder("a"), crate::tests::folder("b")])];
        files.selection = vec![1];
        assert_eq!(files.ancestor_shown(), 1, "one folder above the open one");
        let (w, h) = (1200usize, 800usize);
        let mut px = vec![0u32; w * h];
        let visible = files.visible();
        files.paint_columns(&ui, &mut Surface { w, h, stride: w, px: &mut px, y0: 0 }, &visible);
        // a pixel inside a row, right of the name and left of the chevron
        let at = |column: f32, row: usize| {
            let x = files.side() + 16. + column * files.vo.col_w + 120.;
            let y = files.rows_top() + row as f32 * ROW_H + 3.;
            px[ui.pt(y) as usize * w + ui.pt(x) as usize]
        };
        assert_eq!(at(0., 0), SELECTION_INACTIVE.now(), "the folder the path goes through: grey plate");
        assert_eq!(at(0., 1), 0, "the other folder above has no plate");
        assert_eq!(at(1., 1), SELECTION.now(), "the selected row of the focused column: accent plate");
        assert_eq!(at(1., 0), 0, "an unselected row has no plate");
    }
}
