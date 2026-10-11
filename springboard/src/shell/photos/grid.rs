//! Layout of each photo album's grid: a header row per month section, then
//! rows of square cells separated by hairline gaps, in content coordinates
//! (y = 0 at the top of the scrolling content).

use super::library::Section;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RowKind {
    /// the section's title
    Header(usize),
    /// `count` cells starting at photo `first`
    Cells { first: usize, count: usize },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Row {
    pub y: f32,
    pub h: f32,
    pub kind: RowKind,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Grid {
    pub cols: usize,
    /// cell edge in px
    pub cell: f32,
    pub gap: f32,
    rows: Vec<Row>,
    /// top-left of every photo's cell, by photo index
    cells: Vec<(f32, f32)>,
    /// total content height
    pub height: f32,
}

impl Grid {
    /// Lays `sections` out `cols` wide in `w` px with `gap` px between
    /// cells and `header_h` px for each title row.
    pub fn build(sections: &[Section], cols: usize, w: f32, gap: f32, header_h: f32) -> Grid {
        let cols = cols.max(1);
        let cell = ((w - (cols - 1) as f32 * gap) / cols as f32).floor();
        let mut rows = Vec::new();
        let n = sections.iter().map(|s| s.start + s.len).max().unwrap_or(0);
        let mut cells = vec![(0.0, 0.0); n];
        let mut y = 0.0;
        for (si, s) in sections.iter().enumerate() {
            rows.push(Row { y, h: header_h, kind: RowKind::Header(si) });
            y += header_h;
            let mut first = s.start;
            while first < s.start + s.len {
                let count = cols.min(s.start + s.len - first);
                rows.push(Row { y, h: cell, kind: RowKind::Cells { first, count } });
                for c in 0..count {
                    cells[first + c] = (c as f32 * (cell + gap), y);
                }
                y += cell + gap;
                first += count;
            }
        }
        Grid { cols, cell, gap, rows, cells, height: y }
    }

    pub fn rows(&self) -> &[Row] {
        &self.rows
    }

    /// The cell of photo `i` as (x, y, w, h).
    pub fn cell_rect(&self, i: usize) -> (f32, f32, f32, f32) {
        let (x, y) = self.cells[i];
        (x, y, self.cell, self.cell)
    }

    /// The photo whose cell holds content point (x, y).
    pub fn hit(&self, x: f32, y: f32) -> Option<usize> {
        let row = self.rows.get(self.rows.partition_point(|r| r.y + r.h <= y)).filter(|r| y >= r.y)?;
        let RowKind::Cells { first, count } = row.kind else { return None };
        let col = (x / (self.cell + self.gap)).floor();
        if col < 0.0 || x - col * (self.cell + self.gap) >= self.cell {
            return None;
        }
        let col = col as usize;
        (col < count).then_some(first + col)
    }

    /// The rows intersecting content rows `y0..y1` (rows are in y order, so two binary searches).
    fn rows_between(&self, y0: f32, y1: f32) -> &[Row] {
        let start = self.rows.partition_point(|r| r.y + r.h <= y0);
        let end = self.rows.partition_point(|r| r.y < y1).max(start);
        &self.rows[start..end]
    }

    /// Photos whose cells intersect content rows `y0..y1`.
    pub fn visible(&self, y0: f32, y1: f32) -> Vec<usize> {
        self.rows_between(y0, y1)
            .iter()
            .filter_map(|r| match r.kind {
                RowKind::Cells { first, count } => Some(first..first + count),
                RowKind::Header(_) => None,
            })
            .flatten()
            .collect()
    }

    /// Section headers inside content rows `y0..y1`, as (section, row y).
    pub fn headers(&self, y0: f32, y1: f32) -> Vec<(usize, f32)> {
        self.rows_between(y0, y1)
            .iter()
            .filter_map(|r| match r.kind {
                RowKind::Header(s) => Some((s, r.y)),
                RowKind::Cells { .. } => None,
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sections() -> Vec<Section> {
        vec![Section { title: "September 2026".into(), start: 0, len: 7 }, Section { title: "October 2026".into(), start: 7, len: 2 }]
    }

    #[test]
    fn rows_alternate_headers_and_full_rows() {
        let g = Grid::build(&sections(), 5, 1200.0, 2.0, 60.0);
        assert_eq!(g.cell, 238.0);
        let kinds: Vec<RowKind> = g.rows().iter().map(|r| r.kind).collect();
        assert_eq!(kinds, [RowKind::Header(0), RowKind::Cells { first: 0, count: 5 }, RowKind::Cells { first: 5, count: 2 }, RowKind::Header(1), RowKind::Cells { first: 7, count: 2 }]);
        assert_eq!(g.cell_rect(0), (0.0, 60.0, 238.0, 238.0));
        assert_eq!(g.cell_rect(6), (240.0, 300.0, 238.0, 238.0));
        assert_eq!(g.cell_rect(7).1, 60.0 + 240.0 * 2.0 + 60.0);
        assert_eq!(g.height, 60.0 + 240.0 * 2.0 + 60.0 + 240.0);
        let three = Grid::build(&sections(), 3, 1200.0, 2.0, 60.0);
        assert_eq!(three.cell, 398.0);
        assert_eq!(three.rows().len(), 2 + 3 + 1);
        assert_eq!(Grid::build(&[], 5, 1200.0, 2.0, 60.0).height, 0.0);
    }

    #[test]
    fn hit_testing_lands_on_cells_not_gaps_or_headers() {
        let g = Grid::build(&sections(), 5, 1200.0, 2.0, 60.0);
        assert_eq!(g.hit(10.0, 10.0), None, "header");
        assert_eq!(g.hit(10.0, 70.0), Some(0));
        assert_eq!(g.hit(239.0, 70.0), None, "in the gap");
        assert_eq!(g.hit(241.0, 70.0), Some(1));
        assert_eq!(g.hit(241.0, 310.0), Some(6));
        assert_eq!(g.hit(700.0, 310.0), None, "empty slot in the short row");
        assert_eq!(g.hit(-1.0, 70.0), None);
        assert_eq!(g.hit(10.0, 5000.0), None, "below the content");
    }

    #[test]
    fn visible_range_and_headers_follow_the_viewport() {
        let g = Grid::build(&sections(), 5, 1200.0, 2.0, 60.0);
        assert_eq!(g.visible(0.0, 100.0), (0..5).collect::<Vec<_>>());
        assert_eq!(g.visible(300.0, 700.0), (5..9).collect::<Vec<_>>());
        assert_eq!(g.headers(0.0, 100.0), [(0, 0.0)]);
        assert_eq!(g.headers(0.0, 10_000.0).len(), 2);
        assert!(g.visible(10_000.0, 20_000.0).is_empty());
        assert!(g.visible(500.0, 100.0).is_empty(), "an inverted range is empty");
        assert_eq!(g.visible(0.0, 60.0), Vec::<usize>::new(), "only the header row");
    }
}
