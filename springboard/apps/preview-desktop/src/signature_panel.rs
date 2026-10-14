//! The signature picker: the saved signatures listed down the right side of the window, each drawn in its own cell. A
//! click on a cell places that signature on the shown page. This module is the geometry and the fitting of a signature
//! into a cell; the window draws the strokes it returns and runs the insert a click returns.

use crate::export_panel::Metrics;
use crate::signature::{self, Signature};

/// One saved signature's cell in the panel, in window pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cell {
    pub index: usize,
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

/// The cells of `count` saved signatures, stacked from the top of the panel under its padding. Each cell is one button
/// row high, with the gap between cells.
pub fn cells(m: &Metrics, count: usize) -> Vec<Cell> {
    let w = (m.width - 2 * m.pad).max(1);
    (0..count)
        .map(|index| Cell { index, x: m.left + m.pad, y: m.top + m.pad + index as i32 * (m.row + m.gap), w, h: m.row })
        .collect()
}

/// The saved signature under a window point, if any.
pub fn hit(m: &Metrics, count: usize, x: f32, y: f32) -> Option<usize> {
    cells(m, count).into_iter().find(|c| x >= c.x as f32 && x < (c.x + c.w) as f32 && y >= c.y as f32 && y < (c.y + c.h) as f32).map(|c| c.index)
}

/// The width and height a signature of `aspect` (height over width) takes in a `box_w` by `box_h` box: as large as
/// fits, keeping its shape. A bad aspect is taken as square.
pub fn fit(aspect: f32, box_w: f32, box_h: f32) -> (f32, f32) {
    let aspect = if aspect.is_finite() && aspect > 0.0 { aspect } else { 1.0 };
    let (w, h) = (box_w.max(1.0), box_h.max(1.0));
    if w * aspect <= h {
        (w, w * aspect)
    } else {
        (h / aspect, h)
    }
}

/// The strokes of `sig` drawn into `cell`: fitted with a margin of a quarter of the cell's height and centred.
pub fn strokes_in(sig: &Signature, cell: &Cell) -> Vec<Vec<(f32, f32)>> {
    let margin = cell.h as f32 / 4.0;
    let (w, h) = fit(sig.aspect, cell.w as f32 - 2.0 * margin, cell.h as f32 - 2.0 * margin);
    let x = cell.x as f32 + (cell.w as f32 - w) / 2.0;
    let y = cell.y as f32 + (cell.h as f32 - h) / 2.0;
    signature::place(sig, x, y, w, h)
}

#[cfg(test)]
mod tests {
    use super::*;

    const METRICS: Metrics = Metrics { left: 600, top: 48, width: 200, pad: 8, row: 40, heading: 16, gap: 4 };

    fn square() -> Signature {
        Signature { strokes: vec![vec![(0.0, 0.0), (1.0, 1.0)]], aspect: 1.0 }
    }

    #[test]
    fn cells_stack_from_the_top_with_the_gap_between_them() {
        let result = cells(&METRICS, 3);
        assert_eq!(result.len(), 3);
        assert_eq!((result[0].x, result[0].y, result[0].w, result[0].h), (608, 56, 184, 40));
        assert_eq!(result[1].y, 56 + 44);
        assert_eq!(result[2].index, 2);
        assert!(cells(&METRICS, 0).is_empty());
    }

    #[test]
    fn a_point_picks_the_signature_cell_it_is_in() {
        assert_eq!(hit(&METRICS, 3, 620.0, 60.0), Some(0));
        assert_eq!(hit(&METRICS, 3, 620.0, 60.0 + 44.0), Some(1));
        assert_eq!(hit(&METRICS, 3, 620.0, 60.0 + 3.0 * 44.0), None, "below the last cell");
        assert_eq!(hit(&METRICS, 3, 10.0, 60.0), None, "outside the panel");
    }

    #[test]
    fn a_signature_is_fitted_keeping_its_shape() {
        // a wide box is limited by its height, a tall one by its width
        assert_eq!(fit(0.5, 100.0, 100.0), (100.0, 50.0));
        assert_eq!(fit(2.0, 100.0, 100.0), (50.0, 100.0));
        assert_eq!(fit(f32::NAN, 40.0, 80.0), (40.0, 40.0), "a bad aspect is square");
    }

    #[test]
    fn strokes_sit_inside_their_cell_with_a_margin() {
        let cell = Cell { index: 0, x: 100, y: 50, w: 200, h: 40 };
        let strokes = strokes_in(&square(), &cell);
        let points = &strokes[0];
        for &(x, y) in points {
            assert!(x >= cell.x as f32 && x <= (cell.x + cell.w) as f32, "{x}");
            assert!(y >= cell.y as f32 + 10.0 - 0.001 && y <= (cell.y + cell.h) as f32 - 10.0 + 0.001, "{y}");
        }
    }
}
