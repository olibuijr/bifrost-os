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
    /// One row of the list view: disclosure triangle, icon, name, date, size and kind.
    pub(crate) fn paint_list_row(&self, ui: &Ui, s: &mut Surface, n: usize, i: usize, prev: Option<usize>, bottom: f32) {
        let w = self.w;
        let e = &self.rows[i];
        let name_field = self.naming.as_ref().filter(|(p, _)| *p == e.path).map(|(_, f)| f);
        let selected = self.selection.contains(&i);
        let y = self.rows_top() + n as f32 * 32. - self.scroll;
        if y < self.rows_top() || y + 32. > bottom || ui.pt(y+32.) <= s.y0 as f32 || ui.pt(y) >= s.h as f32 {
            return;
        }
        let indent = self.depth_of(i) as f32 * INDENT;
        if n % 2 == 1 {
            rect(s, ui, self.side(), y, w - self.side(), 32., CARD_BG.now())
        }
        if self.dragging && self.drop_target.as_ref() == Some(&e.path) {
            rect(s, ui, self.side(), y, w - self.side(), 32., TINT)
        }
        if selected {
            rect(s, ui, self.side(), y, w - self.side(), 32., SELECTION.now())
        }
        // Group By: a caption where a new group starts
        let group = self.vo.group;
        let caption = (group != Group::None)
            .then(|| group.key(e))
            .filter(|k| prev.is_none_or(|p| group.key(&self.rows[p]) != *k));
        let name_dy = if caption.is_some() { 11. } else { 8. };
        if let Some((_, label)) = caption {
            rect(s, ui, self.side(), y, w - self.side(), 1., SEPARATOR_C.now());
            text(s, ui, &label, 10., if selected { WHITE } else { LABEL_2.now() }, self.side() + 48., y + 1., 200.);
        }
        if e.is_dir {
            let open = self.vo.expanded.contains(&e.path);
            triangle(s, ui, self.side() + 4. + indent, y + 12., if open { 1 } else { 0 }, if selected { WHITE } else { LABEL_2.now() });
        }
        icons::entry(
            s,
            (ui.pt(self.side() + 18. + indent), ui.pt(y + 7.), ui.pt(SPACE_XL), ui.pt(SPACE_XL)),
            e.kind,
        );
        tag_dots(s, ui, &self.tag_index, &e.path, w - 452., y + 11.);
        self.paint_badges(ui, s, &e.path, self.side() + 18. + indent + SPACE_XL - 2., y + 14.);
        let name_w = (w - 480. - 48. - indent - self.side()).max(40.);
        if let Some(field) = name_field {
            bifrost_app::widgets::text_field(s, ui, field, Rect { x: (self.side() + 46. + indent), y: y + 3., w: name_w + 10., h: 26. }, "", true);
        } else {
            cached_text(s, ui, &self.row_text, &e.name, self.vo.text + 1., ink(selected), self.side() + 48. + indent, y + name_dy, name_w);
        }
        let small = (self.vo.text).max(10.);
        let secondary = if selected { WHITE } else { LABEL_2.now() };
        let row_text = &self.row_text;
        cached_text(s, ui, row_text, &date_text(e.mtime), small, secondary, w - 430., y + 8., 135.);
        cached_text(s, ui, row_text, &if e.is_dir { "—".into() } else { fs::size_text(e.size) }, small, secondary, w - 290., y + 8., 105.);
        cached_text(s, ui, row_text, e.kind.label(), small, secondary, w - 165., y + 8., 145.);
    }
}
