//! Text tables (`NSTextTable`, `NSTextTableBlock`): a grid of blocks laid out in a container width. Columns take an
//! absolute width, a percentage of the table, or an equal share of what is left. A row is as tall as its tallest
//! cell. A block may span rows and columns.

use crate::Rect;
use std::ops::Range;

/// How wide a column is (`NSTextBlock.setValue:type:forDimension:` for the width).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ColumnWidth {
    /// Points.
    Absolute(f32),
    /// Percent of the table width (0 to 100).
    Percentage(f32),
    /// An equal share of the width the other columns leave.
    Auto,
}

/// One cell (`NSTextTableBlock`).
#[derive(Clone, Debug, PartialEq)]
pub struct TextTableBlock {
    pub row: usize,
    pub column: usize,
    pub row_span: usize,
    pub column_span: usize,
    /// Space between the cell edge and its content, on every side.
    pub padding: f32,
    /// The byte range of the text the cell holds.
    pub content: Range<usize>,
}

impl TextTableBlock {
    pub fn new(row: usize, column: usize, content: Range<usize>) -> Self {
        TextTableBlock { row, column, row_span: 1, column_span: 1, padding: 0.0, content }
    }
}

/// A cell placed in the table: the block index and its rectangle in the table's coordinates.
#[derive(Clone, Debug, PartialEq)]
pub struct PlacedBlock {
    pub block: usize,
    pub rect: Rect,
}

/// The laid out table.
#[derive(Clone, Debug, PartialEq)]
pub struct TableLayout {
    pub cells: Vec<PlacedBlock>,
    pub column_widths: Vec<f32>,
    pub row_heights: Vec<f32>,
    pub height: f32,
}

/// A table (`NSTextTable`).
#[derive(Clone, Debug, PartialEq)]
pub struct TextTable {
    pub columns: Vec<ColumnWidth>,
    pub blocks: Vec<TextTableBlock>,
}

impl TextTable {
    /// A table of `columns` columns, each `Auto`.
    pub fn new(columns: usize) -> Self {
        TextTable { columns: vec![ColumnWidth::Auto; columns.max(1)], blocks: Vec::new() }
    }

    /// `numberOfColumns`.
    pub fn number_of_columns(&self) -> usize {
        self.columns.len()
    }

    /// The number of rows the blocks reach.
    pub fn number_of_rows(&self) -> usize {
        self.blocks.iter().map(|b| b.row + b.row_span.max(1)).max().unwrap_or(0)
    }

    fn fixed_width(c: ColumnWidth, width: f32) -> f32 {
        match c {
            ColumnWidth::Absolute(w) => w,
            ColumnWidth::Percentage(p) => width * p / 100.0,
            ColumnWidth::Auto => 0.0,
        }
    }

    /// The widths of the columns in a table `width` wide.
    pub fn column_widths(&self, width: f32) -> Vec<f32> {
        let fixed: f32 = self.columns.iter().map(|c| Self::fixed_width(*c, width)).sum();
        let autos = self.columns.iter().filter(|c| **c == ColumnWidth::Auto).count();
        let share = if autos == 0 { 0.0 } else { (width - fixed).max(0.0) / autos as f32 };
        self.columns
            .iter()
            .map(|c| if *c == ColumnWidth::Auto { share } else { Self::fixed_width(*c, width) })
            .collect()
    }

    fn block_width(widths: &[f32], b: &TextTableBlock) -> f32 {
        let start = b.column.min(widths.len());
        let end = (b.column + b.column_span.max(1)).min(widths.len());
        widths[start..end].iter().sum()
    }

    /// Lays the table out in `width` points. `content_height(block, content_width)` gives the height of a block's
    /// text when laid out `content_width` wide (cell width minus padding). Only blocks that span one row set a row's
    /// height; a taller spanning block grows its last row.
    pub fn layout(&self, width: f32, content_height: &dyn Fn(usize, f32) -> f32) -> TableLayout {
        let widths = self.column_widths(width);
        let rows = self.number_of_rows();
        let cell_height = |i: usize, b: &TextTableBlock| -> f32 {
            let inner = (Self::block_width(&widths, b) - 2.0 * b.padding).max(0.0);
            content_height(i, inner) + 2.0 * b.padding
        };
        let mut heights = vec![0.0f32; rows];
        for (i, b) in self.blocks.iter().enumerate() {
            if b.row_span.max(1) == 1 {
                heights[b.row] = heights[b.row].max(cell_height(i, b));
            }
        }
        for (i, b) in self.blocks.iter().enumerate() {
            if b.row_span.max(1) > 1 {
                let last = (b.row + b.row_span).min(rows) - 1;
                let have: f32 = heights[b.row..=last].iter().sum();
                let need = cell_height(i, b);
                if need > have {
                    heights[last] += need - have;
                }
            }
        }
        let mut cells = Vec::with_capacity(self.blocks.len());
        for (i, b) in self.blocks.iter().enumerate() {
            let x: f32 = widths[..b.column.min(widths.len())].iter().sum();
            let y: f32 = heights[..b.row].iter().sum();
            let end = (b.row + b.row_span.max(1)).min(rows);
            let h: f32 = heights[b.row..end].iter().sum();
            cells.push(PlacedBlock { block: i, rect: Rect { x, y, w: Self::block_width(&widths, b), h } });
        }
        let height = heights.iter().sum();
        TableLayout { cells, column_widths: widths, row_heights: heights, height }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn two_by_two() -> TextTable {
        let mut t = TextTable::new(2);
        for r in 0..2 {
            for c in 0..2 {
                let mut b = TextTableBlock::new(r, c, 0..0);
                b.padding = 2.0;
                t.blocks.push(b);
            }
        }
        t
    }

    #[test]
    fn auto_columns_share_the_width_and_rows_take_the_tallest_cell() {
        let t = two_by_two();
        // Block 1 (row 0, column 1) holds 30 points of text; the others 10.
        let l = t.layout(200.0, &|i, _| if i == 1 { 30.0 } else { 10.0 });
        assert_eq!(l.column_widths, vec![100.0, 100.0]);
        assert_eq!(l.row_heights, vec![34.0, 14.0]);
        assert_eq!(l.height, 48.0);
        assert_eq!(l.cells[3].rect, Rect { x: 100.0, y: 34.0, w: 100.0, h: 14.0 });
        assert_eq!(l.cells[0].rect, Rect { x: 0.0, y: 0.0, w: 100.0, h: 34.0 });
    }

    #[test]
    fn percentage_and_absolute_columns_leave_the_rest_to_auto() {
        let mut t = TextTable::new(3);
        t.columns = vec![ColumnWidth::Percentage(25.0), ColumnWidth::Absolute(50.0), ColumnWidth::Auto];
        assert_eq!(t.column_widths(200.0), vec![50.0, 50.0, 100.0]);
    }

    #[test]
    fn content_width_excludes_the_padding() {
        let t = two_by_two();
        let seen = std::cell::RefCell::new(Vec::new());
        t.layout(200.0, &|_, w| {
            seen.borrow_mut().push(w);
            0.0
        });
        assert_eq!(seen.borrow()[0], 96.0);
    }

    #[test]
    fn a_column_span_covers_its_columns() {
        let mut t = TextTable::new(2);
        let mut b = TextTableBlock::new(0, 0, 0..0);
        b.column_span = 2;
        t.blocks.push(b);
        let l = t.layout(200.0, &|_, _| 10.0);
        assert_eq!(l.cells[0].rect.w, 200.0);
    }
}
