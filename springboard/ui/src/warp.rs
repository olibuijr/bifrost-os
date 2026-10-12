//! Grid warps: a lattice of source and destination vertices that bends content from one shape to another.
//! A grid of `columns` by `rows` cells has `(columns + 1) * (rows + 1)` vertices, stored row by row.
//! Source positions are in normalized content space (0 to 1 on each axis); destination positions are in
//! target space. The source lattice must be axis-aligned: column x values and row y values strictly increase.
//! The mapping inside a cell is bilinear, so a point maps to the bilinear blend of its cell's four destinations.

/// A warp grid (a grid-based deformation of content).
#[derive(Clone, Debug, PartialEq)]
pub struct WarpGrid {
    columns: usize,
    rows: usize,
    source: Vec<(f32, f32)>,
    destination: Vec<(f32, f32)>,
}

impl WarpGrid {
    /// An identity grid of `columns` by `rows` cells, with source and destination both on the unit square.
    /// Returns None when either count is zero.
    pub fn new(columns: usize, rows: usize) -> Option<Self> {
        if columns == 0 || rows == 0 {
            return None;
        }
        let mut source = Vec::with_capacity((columns + 1) * (rows + 1));
        for r in 0..=rows {
            for c in 0..=columns {
                source.push((c as f32 / columns as f32, r as f32 / rows as f32));
            }
        }
        Some(WarpGrid { columns, rows, destination: source.clone(), source })
    }

    /// A grid with explicit vertices, row by row. Returns None when a vertex count does not match, a count is
    /// zero, or the source lattice is not strictly increasing in x along a row and in y down a column.
    pub fn with_positions(
        columns: usize,
        rows: usize,
        source: Vec<(f32, f32)>,
        destination: Vec<(f32, f32)>,
    ) -> Option<Self> {
        if columns == 0 || rows == 0 {
            return None;
        }
        let count = (columns + 1) * (rows + 1);
        if source.len() != count || destination.len() != count {
            return None;
        }
        let grid = WarpGrid { columns, rows, source, destination };
        for c in 0..columns {
            if grid.src(c + 1, 0).0 <= grid.src(c, 0).0 {
                return None;
            }
        }
        for r in 0..rows {
            if grid.src(0, r + 1).1 <= grid.src(0, r).1 {
                return None;
            }
        }
        Some(grid)
    }

    fn index(&self, c: usize, r: usize) -> usize {
        r * (self.columns + 1) + c
    }

    fn src(&self, c: usize, r: usize) -> (f32, f32) {
        self.source[self.index(c, r)]
    }

    fn dst(&self, c: usize, r: usize) -> (f32, f32) {
        self.destination[self.index(c, r)]
    }

    pub fn columns(&self) -> usize {
        self.columns
    }

    pub fn rows(&self) -> usize {
        self.rows
    }

    pub fn vertex_count(&self) -> usize {
        (self.columns + 1) * (self.rows + 1)
    }

    /// The source position of vertex (`c`, `r`), where `c` runs 0..=columns and `r` runs 0..=rows.
    pub fn source_at(&self, c: usize, r: usize) -> Option<(f32, f32)> {
        if c > self.columns || r > self.rows {
            return None;
        }
        Some(self.src(c, r))
    }

    /// The destination position of vertex (`c`, `r`).
    pub fn destination_at(&self, c: usize, r: usize) -> Option<(f32, f32)> {
        if c > self.columns || r > self.rows {
            return None;
        }
        Some(self.dst(c, r))
    }

    /// A copy of this grid with new destination positions. None when the count does not match.
    pub fn with_destination(&self, destination: Vec<(f32, f32)>) -> Option<Self> {
        if destination.len() != self.vertex_count() {
            return None;
        }
        Some(WarpGrid { columns: self.columns, rows: self.rows, source: self.source.clone(), destination })
    }

    /// Maps a point in normalized source space to target space. None when the point lies outside the
    /// source lattice.
    pub fn map(&self, p: (f32, f32)) -> Option<(f32, f32)> {
        let (x0, y0) = self.src(0, 0);
        let (x1, y1) = self.src(self.columns, self.rows);
        if p.0 < x0 || p.0 > x1 || p.1 < y0 || p.1 > y1 {
            return None;
        }
        // The last column and row whose lower edge is at or below the point.
        let c = (0..self.columns).rev().find(|&c| self.src(c, 0).0 <= p.0).unwrap_or(0);
        let r = (0..self.rows).rev().find(|&r| self.src(0, r).1 <= p.1).unwrap_or(0);
        let (sx0, sy0) = self.src(c, r);
        let (sx1, sy1) = self.src(c + 1, r + 1);
        let u = (p.0 - sx0) / (sx1 - sx0);
        let v = (p.1 - sy0) / (sy1 - sy0);
        let d00 = self.dst(c, r);
        let d10 = self.dst(c + 1, r);
        let d01 = self.dst(c, r + 1);
        let d11 = self.dst(c + 1, r + 1);
        let top = (d00.0 + (d10.0 - d00.0) * u, d00.1 + (d10.1 - d00.1) * u);
        let bottom = (d01.0 + (d11.0 - d01.0) * u, d01.1 + (d11.1 - d01.1) * u);
        Some((top.0 + (bottom.0 - top.0) * v, top.1 + (bottom.1 - top.1) * v))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EPS: f32 = 1e-4;

    fn close2(a: (f32, f32), b: (f32, f32)) -> bool {
        (a.0 - b.0).abs() < EPS && (a.1 - b.1).abs() < EPS
    }

    fn unit_square_source() -> Vec<(f32, f32)> {
        vec![(0.0, 0.0), (1.0, 0.0), (0.0, 1.0), (1.0, 1.0)]
    }

    #[test]
    fn vertex_count_is_cells_plus_one_each_way() {
        let g = WarpGrid::new(3, 2).unwrap();
        assert_eq!(g.vertex_count(), 12);
        assert_eq!(g.columns(), 3);
        assert_eq!(g.rows(), 2);
        assert!(WarpGrid::new(0, 2).is_none());
    }

    #[test]
    fn identity_grid_maps_points_to_themselves() {
        let g = WarpGrid::new(2, 2).unwrap();
        assert!(close2(g.map((0.3, 0.8)).unwrap(), (0.3, 0.8)));
        assert!(close2(g.map((1.0, 0.0)).unwrap(), (1.0, 0.0)));
    }

    #[test]
    fn one_cell_scaled_by_two() {
        let g = WarpGrid::with_positions(
            1,
            1,
            unit_square_source(),
            vec![(0.0, 0.0), (2.0, 0.0), (0.0, 2.0), (2.0, 2.0)],
        )
        .unwrap();
        assert!(close2(g.map((0.5, 0.5)).unwrap(), (1.0, 1.0)));
        assert!(close2(g.map((1.0, 1.0)).unwrap(), (2.0, 2.0)));
    }

    #[test]
    fn moved_corner_pulls_the_interior() {
        let g = WarpGrid::with_positions(
            1,
            1,
            unit_square_source(),
            vec![(0.0, 0.0), (1.0, 0.0), (0.0, 1.0), (3.0, 1.0)],
        )
        .unwrap();
        // top = (0.5, 0), bottom = (1.5, 1); midpoint of those is (1.0, 0.5).
        assert!(close2(g.map((0.5, 0.5)).unwrap(), (1.0, 0.5)));
        assert!(close2(g.map((1.0, 1.0)).unwrap(), (3.0, 1.0)));
    }

    #[test]
    fn point_outside_the_source_lattice_is_none() {
        let g = WarpGrid::new(2, 2).unwrap();
        assert!(g.map((1.5, 0.5)).is_none());
        assert!(g.map((0.5, -0.1)).is_none());
    }

    #[test]
    fn second_column_uses_its_own_cell() {
        let source = vec![(0.0, 0.0), (0.5, 0.0), (1.0, 0.0), (0.0, 1.0), (0.5, 1.0), (1.0, 1.0)];
        let g = WarpGrid::with_positions(2, 1, source.clone(), source).unwrap();
        // The point lies in column 1 with u = 0.5 and v = 0.25; an identity grid returns it unchanged.
        assert!(close2(g.map((0.75, 0.25)).unwrap(), (0.75, 0.25)));
    }

    #[test]
    fn invalid_grids_are_rejected() {
        assert!(WarpGrid::with_positions(1, 1, vec![(0.0, 0.0), (1.0, 0.0), (0.0, 1.0)], unit_square_source()).is_none());
        let folded = vec![(0.0, 0.0), (0.0, 0.0), (0.0, 1.0), (0.0, 1.0)];
        assert!(WarpGrid::with_positions(1, 1, folded, unit_square_source()).is_none());
        let g = WarpGrid::new(1, 1).unwrap();
        assert!(g.with_destination(vec![(0.0, 0.0)]).is_none());
    }
}
