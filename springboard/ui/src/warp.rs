//! Grid warp geometry: a grid of vertices, each with a source point and a destination point. A point inside the source
//! grid is moved to where the destination grid puts it, so content can be bent (a wave, a page curl, a squeezed panel).
//! It is the counterpart of the 2D scene framework's warp geometry grid, and it animates like that one: `interpolate`
//! moves the destination vertices from one grid to another.
//!
//! Pure: no clock and no drawing. The source is an axis-aligned rectangle, from the first vertex to the last, cut into
//! `columns` by `rows` cells. Vertices are stored row by row, `columns + 1` per row over `rows + 1` rows, so vertex
//! `row * (columns + 1) + column` is at that column and row. Mapping finds a point's cell from the source bounds with
//! uniform spacing, then interpolates bilinearly between the four destination corners of that cell. Points outside the
//! source are clamped to its edge. Only the source's corners are used, so the spacing of the source vertices is not
//! read back: keep the source on a regular grid.
//!
//! Units: whatever the caller uses for points. The default source is the unit square.

use crate::physics::Vec2;

/// A warp grid. Build one with `new` (the identity over the unit square) or `with_positions`.
#[derive(Clone, Debug, PartialEq)]
pub struct WarpGrid {
    columns: usize,
    rows: usize,
    source: Vec<Vec2>,
    destination: Vec<Vec2>,
}

impl WarpGrid {
    /// An identity grid over the unit square. Zero columns or rows become one.
    pub fn new(columns: usize, rows: usize) -> WarpGrid {
        let columns = columns.max(1);
        let rows = rows.max(1);
        let source = unit_grid(columns, rows);
        WarpGrid {
            columns,
            rows,
            destination: source.clone(),
            source,
        }
    }

    /// A grid with explicit vertices. Returns None when a list does not have `(columns + 1) * (rows + 1)` entries,
    /// or when the source has no width or no height.
    pub fn with_positions(columns: usize, rows: usize, source: Vec<Vec2>, destination: Vec<Vec2>) -> Option<WarpGrid> {
        if columns == 0 || rows == 0 {
            return None;
        }
        let count = (columns + 1) * (rows + 1);
        if source.len() != count || destination.len() != count {
            return None;
        }
        let grid = WarpGrid {
            columns,
            rows,
            source,
            destination,
        };
        grid.bounds()?;
        Some(grid)
    }

    pub fn columns(&self) -> usize {
        self.columns
    }

    pub fn rows(&self) -> usize {
        self.rows
    }

    /// The number of vertices, `(columns + 1) * (rows + 1)`.
    pub fn vertex_count(&self) -> usize {
        (self.columns + 1) * (self.rows + 1)
    }

    pub fn source_position(&self, index: usize) -> Option<Vec2> {
        self.source.get(index).copied()
    }

    pub fn destination_position(&self, index: usize) -> Option<Vec2> {
        self.destination.get(index).copied()
    }

    /// A copy with new destination vertices. None when the count is wrong.
    pub fn replacing_destination(&self, positions: Vec<Vec2>) -> Option<WarpGrid> {
        if positions.len() != self.vertex_count() {
            return None;
        }
        Some(WarpGrid {
            destination: positions,
            ..self.clone()
        })
    }

    /// A copy with new source vertices. None when the count is wrong or the new source has no width or height.
    pub fn replacing_source(&self, positions: Vec<Vec2>) -> Option<WarpGrid> {
        WarpGrid::with_positions(self.columns, self.rows, positions, self.destination.clone())
    }

    /// Moves a point from the source grid to the destination grid.
    pub fn map(&self, p: Vec2) -> Vec2 {
        let (x0, x1, y0, y1) = match self.bounds() {
            Some(b) => b,
            None => return p,
        };
        let u = (((p.x - x0) / (x1 - x0)).max(0.0).min(1.0)) * self.columns as f32;
        let v = (((p.y - y0) / (y1 - y0)).max(0.0).min(1.0)) * self.rows as f32;
        let col = (u.floor() as usize).min(self.columns - 1);
        let row = (v.floor() as usize).min(self.rows - 1);
        let fu = u - col as f32;
        let fv = v - row as f32;
        let stride = self.columns + 1;
        let d00 = self.destination[row * stride + col];
        let d10 = self.destination[row * stride + col + 1];
        let d01 = self.destination[(row + 1) * stride + col];
        let d11 = self.destination[(row + 1) * stride + col + 1];
        let top = lerp(d00, d10, fu);
        let bottom = lerp(d01, d11, fu);
        lerp(top, bottom, fv)
    }

    /// The source rectangle as (x0, x1, y0, y1), from the first and last vertices.
    fn bounds(&self) -> Option<(f32, f32, f32, f32)> {
        let first = *self.source.first()?;
        let last = *self.source.last()?;
        if last.x <= first.x || last.y <= first.y {
            return None;
        }
        Some((first.x, last.x, first.y, last.y))
    }
}

/// The destination vertices of `to` interpolated from those of `from` by `t` (clamped to 0..=1), with the source of
/// `from`. Both grids must have the same number of columns and rows; otherwise None.
pub fn interpolate(from: &WarpGrid, to: &WarpGrid, t: f32) -> Option<WarpGrid> {
    if from.columns != to.columns || from.rows != to.rows {
        return None;
    }
    let t = t.max(0.0).min(1.0);
    let destination = from
        .destination
        .iter()
        .zip(to.destination.iter())
        .map(|(a, b)| lerp(*a, *b, t))
        .collect();
    Some(WarpGrid {
        columns: from.columns,
        rows: from.rows,
        source: from.source.clone(),
        destination,
    })
}

fn unit_grid(columns: usize, rows: usize) -> Vec<Vec2> {
    let mut out = Vec::with_capacity((columns + 1) * (rows + 1));
    for r in 0..=rows {
        for c in 0..=columns {
            out.push(Vec2::new(c as f32 / columns as f32, r as f32 / rows as f32));
        }
    }
    out
}

fn lerp(a: Vec2, b: Vec2, t: f32) -> Vec2 {
    a.plus(b.minus(a).scaled(t))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-5
    }

    fn close_v(a: Vec2, b: Vec2) -> bool {
        close(a.x, b.x) && close(a.y, b.y)
    }

    #[test]
    fn a_one_by_one_grid_has_four_vertices_over_the_unit_square() {
        let g = WarpGrid::new(1, 1);
        assert_eq!(g.vertex_count(), 4);
        assert_eq!(g.source_position(0), Some(Vec2::new(0.0, 0.0)));
        assert_eq!(g.source_position(3), Some(Vec2::new(1.0, 1.0)));
        assert_eq!(g.destination_position(4), None);
    }

    #[test]
    fn the_identity_grid_maps_points_to_themselves() {
        let g = WarpGrid::new(3, 2);
        assert!(close_v(g.map(Vec2::new(0.25, 0.75)), Vec2::new(0.25, 0.75)));
        assert!(close_v(g.map(Vec2::new(1.0, 0.0)), Vec2::new(1.0, 0.0)));
    }

    #[test]
    fn moving_one_corner_bends_the_cell_bilinearly() {
        // Corner (1, 1) moves to (2, 2). At the centre: top = lerp((0,0),(1,0),.5) = (.5,0),
        // bottom = lerp((0,1),(2,2),.5) = (1,1.5), result = lerp((.5,0),(1,1.5),.5) = (.75,.75).
        let g = WarpGrid::new(1, 1);
        let g = g
            .replacing_destination(vec![
                Vec2::new(0.0, 0.0),
                Vec2::new(1.0, 0.0),
                Vec2::new(0.0, 1.0),
                Vec2::new(2.0, 2.0),
            ])
            .unwrap();
        assert!(close_v(g.map(Vec2::new(0.5, 0.5)), Vec2::new(0.75, 0.75)));
        assert!(close_v(g.map(Vec2::new(1.0, 1.0)), Vec2::new(2.0, 2.0)));
    }

    #[test]
    fn a_moved_interior_vertex_only_changes_the_four_cells_around_it() {
        // 2 by 2 grid. The centre vertex is index 1 * 3 + 1 = 4. Moved to (0.9, 0.9), the cell at the origin maps
        // (0.25, 0.25) to (0.35, 0.35): top = (.25, 0), bottom = lerp((0,.5),(.9,.9),.5) = (.45,.7), result (.35,.35).
        let mut dest = unit_grid(2, 2);
        dest[4] = Vec2::new(0.9, 0.9);
        let g = WarpGrid::with_positions(2, 2, unit_grid(2, 2), dest).unwrap();
        assert!(close_v(g.map(Vec2::new(0.25, 0.25)), Vec2::new(0.35, 0.35)));
        // The far corner cell is not touched by the move at (0.9, 0.9), so the origin corner stays put.
        assert!(close_v(g.map(Vec2::new(0.0, 0.0)), Vec2::new(0.0, 0.0)));
    }

    #[test]
    fn with_positions_rejects_a_wrong_vertex_count() {
        let three = vec![Vec2::new(0.0, 0.0); 3];
        let four = unit_grid(1, 1);
        assert!(WarpGrid::with_positions(1, 1, three, four).is_none());
        assert!(WarpGrid::with_positions(0, 1, unit_grid(1, 1), unit_grid(1, 1)).is_none());
    }

    #[test]
    fn replacing_the_destination_keeps_the_source() {
        let g = WarpGrid::new(1, 1);
        let moved = g.replacing_destination(vec![Vec2::new(5.0, 5.0); 4]).unwrap();
        assert_eq!(moved.source_position(3), Some(Vec2::new(1.0, 1.0)));
        assert_eq!(moved.destination_position(3), Some(Vec2::new(5.0, 5.0)));
        assert!(g.replacing_destination(vec![Vec2::ZERO; 2]).is_none());
    }

    #[test]
    fn interpolating_halfway_moves_each_vertex_halfway() {
        // Identity to a grid shifted right by 0.2. Halfway the origin vertex is at (0.1, 0), and the point (0, 0) maps there.
        let from = WarpGrid::new(1, 1);
        let shifted: Vec<Vec2> = unit_grid(1, 1).iter().map(|v| Vec2::new(v.x + 0.2, v.y)).collect();
        let to = from.replacing_destination(shifted).unwrap();
        let mid = interpolate(&from, &to, 0.5).unwrap();
        assert!(close_v(mid.map(Vec2::new(0.0, 0.0)), Vec2::new(0.1, 0.0)));
        assert!(interpolate(&from, &WarpGrid::new(2, 1), 0.5).is_none());
    }
}
