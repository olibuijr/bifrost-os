//! Cloth simulation: a rectangular sheet of points joined by springs that hold their length, stepped with Verlet
//! integration. Used for hanging and fluttering fabric in scenes and effects (a banner, a curtain, a flag).
//!
//! The sheet starts flat in the XY plane with row 0 on top: point (column `c`, row `r`) sits at
//! `origin + (c * spacing, -r * spacing, 0)`. Three kinds of link keep the shape: structural links between
//! neighbours (they resist stretching), shear links on the diagonals (they resist skewing) and bend links two points
//! apart (they resist folding). Pinned points never move on their own; the caller may move them to drag the cloth.
//!
//! Pure: no clock and no thread; the caller steps the sheet with [`Cloth::step`]. Units are metres and seconds with
//! +Y up. Each step moves the free points by their last displacement (scaled by `damping`) plus gravity and wind,
//! relaxes every link `iterations` times, then keeps the points above the ground plane when there is one.

use crate::linalg::Vector3;

/// Why a cloth could not be made.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClothError {
    /// A sheet needs at least two columns and two rows.
    TooSmall,
    /// The spacing between points must be a positive, finite number.
    BadSpacing,
}

/// One link between two points and the length it holds.
#[derive(Clone, Copy, Debug)]
struct Link {
    a: usize,
    b: usize,
    rest: f32,
    /// structural links are the ones whose stretch is reported
    structural: bool,
}

/// A sheet of cloth.
#[derive(Clone, Debug)]
pub struct Cloth {
    cols: usize,
    rows: usize,
    pos: Vec<Vector3>,
    prev: Vec<Vector3>,
    pinned: Vec<bool>,
    links: Vec<Link>,
    /// acceleration applied to every free point, metres per second squared (default: 9.8 downwards)
    pub gravity: Vector3,
    /// acceleration from moving air, added to gravity (default: none)
    pub wind: Vector3,
    /// the share of the last displacement a point keeps each step, 0 to 1 (default 0.99)
    pub damping: f32,
    /// how many times each step relaxes the links (default 8)
    pub iterations: usize,
    /// the height of the ground plane; points are kept above it (default: none)
    pub ground: Option<f32>,
}

impl Cloth {
    /// A flat `cols` x `rows` sheet whose top-left point is at `origin`.
    pub fn new(cols: usize, rows: usize, spacing: f32, origin: Vector3) -> Result<Cloth, ClothError> {
        if cols < 2 || rows < 2 {
            return Err(ClothError::TooSmall);
        }
        if !(spacing.is_finite() && spacing > 0.0) {
            return Err(ClothError::BadSpacing);
        }
        let mut pos = Vec::with_capacity(cols * rows);
        for r in 0..rows {
            for c in 0..cols {
                pos.push(origin + Vector3::new(c as f32 * spacing, -(r as f32) * spacing, 0.0));
            }
        }
        let diagonal = spacing * std::f32::consts::SQRT_2;
        let mut links = Vec::new();
        let at = |c: usize, r: usize| r * cols + c;
        for r in 0..rows {
            for c in 0..cols {
                if c + 1 < cols {
                    links.push(Link { a: at(c, r), b: at(c + 1, r), rest: spacing, structural: true });
                }
                if r + 1 < rows {
                    links.push(Link { a: at(c, r), b: at(c, r + 1), rest: spacing, structural: true });
                }
                if c + 1 < cols && r + 1 < rows {
                    links.push(Link { a: at(c, r), b: at(c + 1, r + 1), rest: diagonal, structural: false });
                    links.push(Link { a: at(c + 1, r), b: at(c, r + 1), rest: diagonal, structural: false });
                }
                if c + 2 < cols {
                    links.push(Link { a: at(c, r), b: at(c + 2, r), rest: spacing * 2.0, structural: false });
                }
                if r + 2 < rows {
                    links.push(Link { a: at(c, r), b: at(c, r + 2), rest: spacing * 2.0, structural: false });
                }
            }
        }
        Ok(Cloth {
            cols,
            rows,
            prev: pos.clone(),
            pos,
            pinned: vec![false; cols * rows],
            links,
            gravity: Vector3::new(0.0, -9.8, 0.0),
            wind: Vector3::default(),
            damping: 0.99,
            iterations: 8,
            ground: None,
        })
    }

    /// The number of columns.
    pub fn cols(&self) -> usize {
        self.cols
    }

    /// The number of rows.
    pub fn rows(&self) -> usize {
        self.rows
    }

    fn index(&self, col: usize, row: usize) -> Option<usize> {
        (col < self.cols && row < self.rows).then_some(row * self.cols + col)
    }

    /// Pins a point in place. Returns false when the point is outside the sheet.
    pub fn pin(&mut self, col: usize, row: usize) -> bool {
        match self.index(col, row) {
            Some(i) => {
                self.pinned[i] = true;
                true
            }
            None => false,
        }
    }

    /// Lets a pinned point move again. Returns false when the point is outside the sheet.
    pub fn unpin(&mut self, col: usize, row: usize) -> bool {
        match self.index(col, row) {
            Some(i) => {
                self.pinned[i] = false;
                true
            }
            None => false,
        }
    }

    /// Whether a point is pinned.
    pub fn is_pinned(&self, col: usize, row: usize) -> bool {
        self.index(col, row).is_some_and(|i| self.pinned[i])
    }

    /// Where a point is now.
    pub fn position(&self, col: usize, row: usize) -> Option<Vector3> {
        self.index(col, row).map(|i| self.pos[i])
    }

    /// Moves a point to `to` and gives it no speed, to drag a pinned corner or place a free point.
    pub fn set_position(&mut self, col: usize, row: usize, to: Vector3) -> bool {
        match self.index(col, row) {
            Some(i) => {
                self.pos[i] = to;
                self.prev[i] = to;
                true
            }
            None => false,
        }
    }

    /// Every point, row by row.
    pub fn points(&self) -> &[Vector3] {
        &self.pos
    }

    /// The triangles that cover the sheet, as indices into [`Cloth::points`]: two per cell, wound the same way.
    pub fn triangles(&self) -> Vec<[u32; 3]> {
        let mut out = Vec::with_capacity((self.cols - 1) * (self.rows - 1) * 2);
        for r in 0..self.rows - 1 {
            for c in 0..self.cols - 1 {
                let a = (r * self.cols + c) as u32;
                let b = a + 1;
                let d = ((r + 1) * self.cols + c) as u32;
                let e = d + 1;
                out.push([a, d, b]);
                out.push([b, d, e]);
            }
        }
        out
    }

    /// The most any neighbour link is stretched, as its length over its rest length (1.0 is relaxed).
    pub fn max_stretch(&self) -> f32 {
        self.links
            .iter()
            .filter(|l| l.structural)
            .map(|l| (self.pos[l.b] - self.pos[l.a]).length() / l.rest)
            .fold(0.0, f32::max)
    }

    /// Advances the sheet by `dt` seconds. A zero, negative or non-finite step changes nothing.
    pub fn step(&mut self, dt: f32) {
        if !(dt.is_finite() && dt > 0.0) {
            return;
        }
        let accel = (self.gravity + self.wind) * (dt * dt);
        let keep = self.damping.clamp(0.0, 1.0);
        for i in 0..self.pos.len() {
            if self.pinned[i] {
                self.prev[i] = self.pos[i];
                continue;
            }
            let now = self.pos[i];
            self.pos[i] = now + (now - self.prev[i]) * keep + accel;
            self.prev[i] = now;
        }
        for _ in 0..self.iterations {
            for k in 0..self.links.len() {
                let Link { a, b, rest, .. } = self.links[k];
                let delta = self.pos[b] - self.pos[a];
                let dist = delta.length();
                if dist < 1e-9 {
                    continue;
                }
                let fix = delta * ((dist - rest) / dist);
                match (self.pinned[a], self.pinned[b]) {
                    (true, true) => {}
                    (true, false) => self.pos[b] = self.pos[b] - fix,
                    (false, true) => self.pos[a] = self.pos[a] + fix,
                    (false, false) => {
                        self.pos[a] = self.pos[a] + fix * 0.5;
                        self.pos[b] = self.pos[b] - fix * 0.5;
                    }
                }
            }
        }
        if let Some(floor) = self.ground {
            for i in 0..self.pos.len() {
                if !self.pinned[i] && self.pos[i].y < floor {
                    self.pos[i].y = floor;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn banner() -> Cloth {
        let mut cloth = Cloth::new(6, 6, 0.1, Vector3::new(0.0, 2.0, 0.0)).unwrap();
        for c in 0..6 {
            cloth.pin(c, 0);
        }
        cloth
    }

    #[test]
    fn a_sheet_needs_two_columns_two_rows_and_a_real_spacing() {
        assert_eq!(Cloth::new(1, 4, 0.1, Vector3::default()).unwrap_err(), ClothError::TooSmall);
        assert_eq!(Cloth::new(4, 1, 0.1, Vector3::default()).unwrap_err(), ClothError::TooSmall);
        assert_eq!(Cloth::new(4, 4, 0.0, Vector3::default()).unwrap_err(), ClothError::BadSpacing);
        assert_eq!(Cloth::new(4, 4, f32::NAN, Vector3::default()).unwrap_err(), ClothError::BadSpacing);
    }

    #[test]
    fn the_sheet_starts_flat_with_row_zero_on_top() {
        let cloth = Cloth::new(3, 3, 0.5, Vector3::new(1.0, 4.0, 2.0)).unwrap();
        assert_eq!(cloth.position(0, 0), Some(Vector3::new(1.0, 4.0, 2.0)));
        assert_eq!(cloth.position(2, 1), Some(Vector3::new(2.0, 3.5, 2.0)));
        assert_eq!(cloth.position(3, 0), None);
        assert_eq!(cloth.points().len(), 9);
        assert!((cloth.max_stretch() - 1.0).abs() < 1e-6);
    }

    #[test]
    fn a_pinned_row_holds_while_the_rest_hangs_below_it() {
        let mut cloth = banner();
        for _ in 0..300 {
            cloth.step(1.0 / 60.0);
        }
        for c in 0..6 {
            assert_eq!(cloth.position(c, 0).unwrap().y, 2.0);
        }
        let bottom = cloth.position(2, 5).unwrap();
        assert!(bottom.y < 2.0 - 0.3, "bottom edge at {}", bottom.y);
        assert!(cloth.points().iter().all(|p| p.x.is_finite() && p.y.is_finite() && p.z.is_finite()));
    }

    #[test]
    fn the_links_keep_the_cloth_from_stretching_far() {
        let mut cloth = banner();
        for _ in 0..300 {
            cloth.step(1.0 / 60.0);
        }
        assert!(cloth.max_stretch() < 1.5, "stretch {}", cloth.max_stretch());
    }

    #[test]
    fn without_gravity_or_wind_the_cloth_stays_put() {
        let mut cloth = banner();
        cloth.gravity = Vector3::default();
        let before: Vec<Vector3> = cloth.points().to_vec();
        for _ in 0..30 {
            cloth.step(1.0 / 60.0);
        }
        for (a, b) in before.iter().zip(cloth.points()) {
            assert!((*a - *b).length() < 1e-4);
        }
    }

    #[test]
    fn wind_pushes_the_free_points_along_the_wind() {
        let mut cloth = banner();
        cloth.wind = Vector3::new(0.0, 0.0, 6.0);
        for _ in 0..120 {
            cloth.step(1.0 / 60.0);
        }
        assert!(cloth.position(3, 5).unwrap().z > 0.05);
        assert_eq!(cloth.position(3, 0).unwrap().z, 0.0);
    }

    #[test]
    fn a_dropped_sheet_lands_on_the_ground_and_stays_above_it() {
        let mut cloth = Cloth::new(5, 5, 0.1, Vector3::new(0.0, 1.0, 0.0)).unwrap();
        cloth.ground = Some(0.0);
        for _ in 0..600 {
            cloth.step(1.0 / 60.0);
        }
        assert!(cloth.points().iter().all(|p| p.y >= 0.0));
        assert!(cloth.points().iter().any(|p| p.y == 0.0));
    }

    #[test]
    fn a_step_of_zero_negative_or_not_a_number_changes_nothing() {
        let mut cloth = banner();
        let before: Vec<Vector3> = cloth.points().to_vec();
        cloth.step(0.0);
        cloth.step(-1.0);
        cloth.step(f32::NAN);
        assert_eq!(cloth.points(), before.as_slice());
    }

    #[test]
    fn a_pin_can_be_moved_to_drag_the_cloth_and_released() {
        let mut cloth = banner();
        assert!(cloth.is_pinned(0, 0));
        assert!(cloth.set_position(0, 0, Vector3::new(-0.2, 2.0, 0.0)));
        assert_eq!(cloth.position(0, 0), Some(Vector3::new(-0.2, 2.0, 0.0)));
        assert!(cloth.unpin(0, 0));
        assert!(!cloth.is_pinned(0, 0));
        assert!(!cloth.pin(9, 9));
        assert!(!cloth.set_position(9, 0, Vector3::default()));
    }

    #[test]
    fn the_mesh_has_two_triangles_per_cell_and_valid_indices() {
        let cloth = Cloth::new(4, 3, 0.1, Vector3::default()).unwrap();
        let tris = cloth.triangles();
        assert_eq!(tris.len(), 2 * 3 * 2);
        assert!(tris.iter().flatten().all(|&i| (i as usize) < cloth.points().len()));
    }
}
