//! Column view (Files's browser view): the folders above the open one, then its rows.
use crate::*;

/// Columns view: the width of one column, the row pitch, and the folders shown above the open one.
pub(crate) const COLUMN_W: f32 = 200.;

pub(crate) const ROW_H: f32 = 28.;

/// One row of a column: icon, name, and a chevron for folders.
pub(crate) fn column_row(s: &mut Surface, ui: &Ui, e: &Entry, x0: f32, y: f32, cw: f32) {
    icons::entry(s, (ui.pt(x0 + 8.), ui.pt(y + 5.), ui.pt(SPACE_XL), ui.pt(SPACE_XL)), e.kind);
    text(s, ui, &e.name, 13., LABEL.now(), x0 + 32., y + 6., cw - 56.);
    if e.is_dir {
        text(s, ui, "›", 16., LABEL_2.now(), x0 + cw - 16., y + 3., 12.);
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
                if e.path == *next {
                    rect(s, ui, x0, y, self.vo.col_w, ROW_H, SELECTION.now());
                }
                column_row(s, ui, e, x0, y, self.vo.col_w);
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
            if self.selection.contains(&i) {
                rect(s, ui, x0, y, w - x0, ROW_H, SELECTION.now());
            }
            // the item being renamed shows its name field in place of the name
            if !self.paint_column_name_field(ui, s, e, x0, y) {
                column_row(s, ui, e, x0, y, self.vo.col_w);
            }
            self.paint_badges(ui, s, &e.path, x0 + 8. + SPACE_XL - 2., y + 13.);
        }
        self.paint_column_preview(ui, s, x0 + self.vo.col_w);
    }
}
