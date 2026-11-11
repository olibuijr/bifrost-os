//! List view: one row per item with name, date modified, size and kind; sortable headers and inline folders.
use crate::*;

/// Indent of a nested row, per level.
const INDENT: f32 = 16.;

impl Files {
    /// The sortable columns and the x where each starts (pt), left to right.
    pub(crate) fn list_columns(&self) -> Vec<(Sort, f32)> {
        vec![(Sort::Name, self.side() + 40.), (Sort::Date, self.w - 430.), (Sort::Size, self.w - 290.), (Sort::Kind, self.w - 165.)]
    }
    /// The list view's column header band. The sorted column is bold-marked with an arrow; a click on a header sorts.
    pub(crate) fn paint_list_header(&self, ui: &Ui, s: &mut Surface) {
        let w = self.w;
        rect(s, ui, self.side(), self.body_top(), w - self.side(), 34., CARD_BG.now());
        rect(s, ui, self.side(), self.body_top() + 33., w - self.side(), 1., SEPARATOR_C.now());
        for (by, x) in self.list_columns() {
            let label = if by == Sort::Date { "Date Modified" } else { by.label() };
            let active = by == self.sort;
            let color = if active { LABEL.now() } else { LABEL_2.now() };
            text(s, ui, label, MAC_CALLOUT_PT, color, x, self.body_top() + 10., if by == Sort::Name { w - 580. } else { 110. });
            if active {
                let tw = ui.measure(label, MAC_CALLOUT_PT, Weight::Regular) / ui.pt(1.);
                // arrow up for A to Z / smallest first, down for newest / largest first
                let natural_up = matches!(by, Sort::Name | Sort::Kind);
                let up = natural_up == self.vo.asc;
                triangle(s, ui, x + tw.min(100.) + 6., self.body_top() + 13., if up { 2 } else { 1 }, color);
            }
        }
    }
    /// One row of the list view: disclosure triangle, icon, name, date, size and kind. A row that reaches past the rows'
    /// area (under the header, below `bottom`) is still drawn: `paint` hands in that area as the surface, which cuts it.
    pub(crate) fn paint_list_row(&self, ui: &Ui, s: &mut Surface, n: usize, i: usize, prev: Option<usize>, bottom: f32) {
        let w = self.w;
        let e = &self.rows[i];
        let name_field = self.naming.as_ref().filter(|(p, _)| *p == e.path).map(|(_, f)| f);
        let selected = self.selection.contains(&i);
        let y = self.rows_top() + n as f32 * 32. - self.scroll;
        if y + 32. <= self.rows_top() || y >= bottom || ui.pt(y+32.) <= s.y0 as f32 || ui.pt(y) >= s.h as f32 {
            return;
        }
        let indent = self.depth_of(i) as f32 * INDENT;
        if n % 2 == 1 {
            rect(s, ui, self.side(), y, w - self.side(), 32., CARD_BG.now())
        }
        if self.dragging && self.drop_target.as_ref() == Some(&e.path) {
            rect(s, ui, self.side(), y, w - self.side(), 32., accent_now())
        }
        if selected {
            rect(s, ui, self.side(), y, w - self.side(), 32., self.selection_plate())
        }
        // Group By: a caption where a new group starts
        let group = self.vo.group;
        let caption = (group != Group::None)
            .then(|| group.key(e))
            .filter(|k| prev.is_none_or(|p| group.key(&self.rows[p]) != *k));
        let name_dy = if caption.is_some() { 11. } else { 8. };
        if let Some((_, label)) = caption {
            rect(s, ui, self.side(), y, w - self.side(), 1., SEPARATOR_C.now());
            text(s, ui, &label, 10., self.detail_ink(selected), self.side() + 48., y + 1., 200.);
        }
        if e.is_dir {
            let open = self.vo.expanded.contains(&e.path);
            triangle(s, ui, self.side() + 4. + indent, y + 12., if open { 1 } else { 0 }, self.detail_ink(selected));
        }
        applications::entry_icon(
            s,
            (ui.pt(self.side() + 18. + indent), ui.pt(y + 7.), ui.pt(SPACE_XL), ui.pt(SPACE_XL)),
            e,
        );
        tag_dots(s, ui, &self.tag_index, &e.path, w - 452., y + 11.);
        self.paint_badges(ui, s, &e.path, self.side() + 18. + indent + SPACE_XL - 2., y + 14.);
        let name_w = (w - 480. - 48. - indent - self.side()).max(40.);
        if let Some(field) = name_field {
            bifrost_app::widgets::text_field(s, ui, field, Rect { x: (self.side() + 46. + indent), y: y + 3., w: name_w + 10., h: 26. }, "", true);
        } else {
            cached_text(s, ui, &self.row_text, &e.name, self.vo.text + 1., self.name_ink(selected), self.side() + 48. + indent, y + name_dy, name_w);
        }
        let small = (self.vo.text).max(10.);
        let secondary = self.detail_ink(selected);
        let row_text = &self.row_text;
        cached_text(s, ui, row_text, &date_text(e.mtime), small, secondary, w - 430., y + 8., 135.);
        cached_text(s, ui, row_text, &if e.is_dir { "—".into() } else { fs::size_text(e.size) }, small, secondary, w - 290., y + 8., 105.);
        cached_text(s, ui, row_text, kind_text(e), small, secondary, w - 165., y + 8., 145.);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use springboard::shell::ui::SELECTION_INACTIVE;

    /// E2E8 R05b: a Files window behind another one drew its selected row in the accent colour, like the key window.
    #[test]
    fn a_window_that_is_not_key_draws_its_selection_unemphasised() {
        let font = springboard::font::Font::parse(Box::leak(std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/SFUI.ttf")).unwrap().into_boxed_slice())).unwrap();
        let ui = Ui::new(font, 1.0);
        let mut files = crate::tests::fixture((0..5).map(|i| crate::tests::folder(&format!("f{i:02}"))).collect());
        files.select_only(1);
        let (w, h) = (1200usize, 800usize);
        let mut px = vec![0u32; w * h];
        let mut plate = |files: &mut Files, x: f32, y: f32| {
            px.fill(0);
            files.paint(&ui, &mut Surface { w, h, stride: w, px: &mut px, y0: 0 });
            px[y as usize * w + x as usize]
        };
        assert_ne!(SELECTION_INACTIVE.now(), SELECTION.now());
        // the list view: a point of the second row right of its Kind text
        files.grid = false;
        let list = (files.w - 12., files.rows_top() + 48.);
        assert_eq!(plate(&mut files, list.0, list.1), SELECTION.now(), "the key window: the accent plate");
        let before = files.revision;
        files.set_key(false);
        assert!(files.fx.inactive && files.revision != before, "the change repaints the window");
        assert_eq!(plate(&mut files, list.0, list.1), SELECTION_INACTIVE.now(), "not key: the unemphasised plate");
        assert_eq!(files.name_ink(true), LABEL.now(), "under the normal text");
        assert_eq!(files.detail_ink(true), LABEL_2.now());
        // the columns view: a point of the second row between its name and its chevron
        files.columns = true;
        let (x0, x1) = files.open_column();
        let column = (x1 - 30., files.rows_top() + 1.5 * ROW_H);
        assert!(column.0 > x0);
        assert_eq!(plate(&mut files, column.0, column.1), SELECTION_INACTIVE.now(), "the columns view too");
        let seen = files.revision;
        files.set_key(false);
        assert_eq!(files.revision, seen, "told the same again, nothing is repainted");
        files.set_key(true);
        assert!(!files.fx.inactive);
        assert_eq!(plate(&mut files, column.0, column.1), SELECTION.now(), "key again: the accent plate");
        assert_eq!(files.name_ink(true), WHITE);
        assert_eq!(files.detail_ink(true), WHITE);
        assert_eq!(files.name_ink(false), LABEL.now());
    }

    #[test]
    fn rows_that_only_partly_fit_are_drawn_cut_at_the_header_and_at_the_bottom() {
        let font = springboard::font::Font::parse(Box::leak(std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/SFUI.ttf")).unwrap().into_boxed_slice())).unwrap();
        let ui = Ui::new(font, 1.0);
        let mut files = crate::tests::fixture((0..40).map(|i| crate::tests::folder(&format!("f{i:02}"))).collect());
        files.grid = false;
        // ten rows high and scrolled by half a row: row 0 is half under the header, row 10 half below the view
        files.h = files.rows_top() + 10. * 32.;
        files.scroll = 16.;
        let (top, bottom) = (files.rows_top(), files.bottom());
        assert_eq!(bottom, files.h, "no footer: the view ends with the window");
        let (w, h) = (1200usize, 800usize);
        let mut px = vec![0u32; w * h];
        // a column of pixels right of the Kind text and left of the scroll knob
        let x = (files.w - 12.) as usize;
        let mut painted = |files: &mut Files, row: usize| {
            files.select_only(row);
            px.fill(0);
            files.paint(&ui, &mut Surface { w, h, stride: w, px: &mut px, y0: 0 });
            (HEADER_H as usize..h).filter(|y| px[y * w + x] == SELECTION.now()).collect::<Vec<usize>>()
        };
        let first = painted(&mut files, 0);
        assert!(!first.is_empty(), "the row cut by the header is drawn: no empty band under the header");
        assert_eq!(*first.first().unwrap() as f32, top, "from the header's bottom edge");
        assert_eq!(*first.last().unwrap() as f32, top + 15., "its lower half");
        let last = painted(&mut files, 10);
        assert!(!last.is_empty(), "the row that partly fits at the bottom is drawn, not left out");
        assert_eq!(*last.first().unwrap() as f32, bottom - 16.);
        assert!((*last.last().unwrap() as f32) < bottom, "and it is cut at the bottom of the view");
        // a row wholly below the view is not drawn
        assert!(painted(&mut files, 11).is_empty());
    }
}
