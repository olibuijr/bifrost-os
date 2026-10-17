//! Tile maps (SpriteKit's SKTileMapNode) as a pure model: a grid of columns and rows, each cell either empty or holding a
//! tile group id. The origin is the bottom left corner of the map, as in the scene, so row 0 is the bottom row and y grows
//! upward. Coordinates are in the map's own units; a node's transform places the map in the scene.
//!
//! Auto-tiling is a lookup: [`TileMap::neighbour_mask`] reports which of the four sides hold the same group, and a tile
//! set maps that four-bit mask to the tile to draw. Drawing the tiles is the renderer's work.

/// A tile group id. Zero is reserved for an empty cell.
pub type Group = u32;

/// A map position: column and row, both counted from the bottom left corner.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cell {
    pub column: u32,
    pub row: u32,
}

/// Why a map or a cell is refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TileError {
    /// Columns, rows or the cell size is zero or not finite.
    BadSize,
    /// The cell lies outside the map.
    OutOfBounds,
}

/// The four sides of a cell, as bits of a neighbour mask.
pub const NORTH: u8 = 1;
pub const EAST: u8 = 2;
pub const SOUTH: u8 = 4;
pub const WEST: u8 = 8;

/// A grid of tile groups.
#[derive(Clone, Debug, PartialEq)]
pub struct TileMap {
    columns: u32,
    rows: u32,
    cell_width: f64,
    cell_height: f64,
    cells: Vec<Group>,
}

impl TileMap {
    /// An empty map of `columns` by `rows` cells, each `cell_width` by `cell_height` units.
    pub fn new(columns: u32, rows: u32, cell_width: f64, cell_height: f64) -> Result<TileMap, TileError> {
        let sizes_ok = cell_width.is_finite() && cell_height.is_finite() && cell_width > 0.0 && cell_height > 0.0;
        if columns == 0 || rows == 0 || !sizes_ok {
            return Err(TileError::BadSize);
        }
        let count = columns as usize * rows as usize;
        Ok(TileMap { columns, rows, cell_width, cell_height, cells: vec![0; count] })
    }

    pub fn columns(&self) -> u32 {
        self.columns
    }

    pub fn rows(&self) -> u32 {
        self.rows
    }

    /// The map's size in its own units.
    pub fn size(&self) -> (f64, f64) {
        (self.columns as f64 * self.cell_width, self.rows as f64 * self.cell_height)
    }

    fn index(&self, cell: Cell) -> Result<usize, TileError> {
        if cell.column >= self.columns || cell.row >= self.rows {
            return Err(TileError::OutOfBounds);
        }
        Ok(cell.row as usize * self.columns as usize + cell.column as usize)
    }

    /// The group in a cell, or zero when the cell is empty.
    pub fn group(&self, cell: Cell) -> Result<Group, TileError> {
        Ok(self.cells[self.index(cell)?])
    }

    /// Puts a group in a cell. Group zero clears it.
    pub fn set(&mut self, cell: Cell, group: Group) -> Result<(), TileError> {
        let i = self.index(cell)?;
        self.cells[i] = group;
        Ok(())
    }

    /// Fills a rectangle of cells, given by its lower left cell and its size, with one group. The rectangle is clipped
    /// to the map and the number of cells written is returned.
    pub fn fill(&mut self, lower_left: Cell, width: u32, height: u32, group: Group) -> usize {
        let mut written = 0;
        for row in lower_left.row..lower_left.row.saturating_add(height).min(self.rows) {
            for column in lower_left.column..lower_left.column.saturating_add(width).min(self.columns) {
                let i = row as usize * self.columns as usize + column as usize;
                self.cells[i] = group;
                written += 1;
            }
        }
        written
    }

    /// The cell under a point in map units, or `None` when the point is outside the map.
    pub fn cell_at(&self, x: f64, y: f64) -> Option<Cell> {
        if !(x.is_finite() && y.is_finite()) || x < 0.0 || y < 0.0 {
            return None;
        }
        let column = (x / self.cell_width).floor();
        let row = (y / self.cell_height).floor();
        if column >= self.columns as f64 || row >= self.rows as f64 {
            return None;
        }
        Some(Cell { column: column as u32, row: row as u32 })
    }

    /// The centre of a cell in map units.
    pub fn center(&self, cell: Cell) -> Result<(f64, f64), TileError> {
        self.index(cell)?;
        Ok((
            (cell.column as f64 + 0.5) * self.cell_width,
            (cell.row as f64 + 0.5) * self.cell_height,
        ))
    }

    /// Which sides of a non-empty cell hold the same group. A side off the map counts as different. An empty cell gives
    /// zero.
    pub fn neighbour_mask(&self, cell: Cell) -> Result<u8, TileError> {
        let group = self.group(cell)?;
        if group == 0 {
            return Ok(0);
        }
        let same = |c: u32, r: u32| self.group(Cell { column: c, row: r }).map(|g| g == group).unwrap_or(false);
        let mut mask = 0;
        if cell.row + 1 < self.rows && same(cell.column, cell.row + 1) {
            mask |= NORTH;
        }
        if cell.column + 1 < self.columns && same(cell.column + 1, cell.row) {
            mask |= EAST;
        }
        if cell.row > 0 && same(cell.column, cell.row - 1) {
            mask |= SOUTH;
        }
        if cell.column > 0 && same(cell.column - 1, cell.row) {
            mask |= WEST;
        }
        Ok(mask)
    }

    /// The cells whose squares overlap a rectangle in map units, clipped to the map. Used to draw only the visible part.
    pub fn cells_in(&self, x: f64, y: f64, width: f64, height: f64) -> Vec<Cell> {
        if !(x.is_finite() && y.is_finite() && width.is_finite() && height.is_finite()) {
            return Vec::new();
        }
        let (map_w, map_h) = self.size();
        let left = x.max(0.0);
        let bottom = y.max(0.0);
        let right = (x + width).min(map_w);
        let top = (y + height).min(map_h);
        if right <= left || top <= bottom {
            return Vec::new();
        }
        let c0 = (left / self.cell_width).floor() as u32;
        let c1 = ((right / self.cell_width).ceil() as u32).min(self.columns);
        let r0 = (bottom / self.cell_height).floor() as u32;
        let r1 = ((top / self.cell_height).ceil() as u32).min(self.rows);
        let mut out = Vec::new();
        for row in r0..r1 {
            for column in c0..c1 {
                out.push(Cell { column, row });
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cell(column: u32, row: u32) -> Cell {
        Cell { column, row }
    }

    #[test]
    fn a_map_needs_cells_and_positive_sizes() {
        assert_eq!(TileMap::new(0, 4, 16.0, 16.0), Err(TileError::BadSize));
        assert_eq!(TileMap::new(4, 0, 16.0, 16.0), Err(TileError::BadSize));
        assert_eq!(TileMap::new(4, 4, 0.0, 16.0), Err(TileError::BadSize));
        assert_eq!(TileMap::new(4, 4, f64::NAN, 16.0), Err(TileError::BadSize));
        assert!(TileMap::new(4, 4, 16.0, 16.0).is_ok());
    }

    #[test]
    fn new_maps_are_empty_and_report_their_size() {
        let m = TileMap::new(3, 2, 16.0, 8.0).unwrap();
        assert_eq!(m.group(cell(2, 1)), Ok(0));
        assert_eq!(m.size(), (48.0, 16.0));
    }

    #[test]
    fn set_and_read_a_cell_and_refuse_outside_it() {
        let mut m = TileMap::new(3, 2, 16.0, 16.0).unwrap();
        m.set(cell(1, 1), 7).unwrap();
        assert_eq!(m.group(cell(1, 1)), Ok(7));
        assert_eq!(m.set(cell(3, 0), 1), Err(TileError::OutOfBounds));
        assert_eq!(m.group(cell(0, 2)), Err(TileError::OutOfBounds));
        m.set(cell(1, 1), 0).unwrap();
        assert_eq!(m.group(cell(1, 1)), Ok(0), "group zero clears the cell");
    }

    #[test]
    fn fill_clips_to_the_map_and_counts_written_cells() {
        let mut m = TileMap::new(4, 3, 1.0, 1.0).unwrap();
        assert_eq!(m.fill(cell(2, 1), 5, 5, 9), 4);
        assert_eq!(m.group(cell(3, 2)), Ok(9));
        assert_eq!(m.group(cell(1, 1)), Ok(0));
    }

    #[test]
    fn a_point_maps_to_the_cell_below_it_from_the_bottom_left() {
        let m = TileMap::new(4, 3, 10.0, 20.0).unwrap();
        assert_eq!(m.cell_at(0.0, 0.0), Some(cell(0, 0)));
        assert_eq!(m.cell_at(25.0, 45.0), Some(cell(2, 2)));
        assert_eq!(m.cell_at(40.0, 0.0), None, "the right edge is outside");
        assert_eq!(m.cell_at(-1.0, 5.0), None);
        assert_eq!(m.cell_at(f64::NAN, 5.0), None);
    }

    #[test]
    fn the_centre_of_a_cell_is_in_the_middle_of_its_square() {
        let m = TileMap::new(4, 3, 10.0, 20.0).unwrap();
        assert_eq!(m.center(cell(1, 2)), Ok((15.0, 50.0)));
        assert_eq!(m.center(cell(4, 0)), Err(TileError::OutOfBounds));
    }

    #[test]
    fn neighbour_mask_reports_matching_sides_only() {
        let mut m = TileMap::new(3, 3, 1.0, 1.0).unwrap();
        m.fill(cell(0, 0), 3, 3, 5);
        m.set(cell(1, 1), 6).unwrap();
        assert_eq!(m.neighbour_mask(cell(1, 1)), Ok(0), "a different group matches nothing");
        assert_eq!(m.neighbour_mask(cell(0, 0)), Ok(NORTH | EAST), "the bottom left corner has two neighbours");
        assert_eq!(m.neighbour_mask(cell(2, 0)), Ok(NORTH | WEST));
        assert_eq!(m.neighbour_mask(cell(1, 2)), Ok(EAST | WEST), "the cell below is another group");
    }

    #[test]
    fn an_empty_cell_has_no_neighbour_mask() {
        let m = TileMap::new(2, 2, 1.0, 1.0).unwrap();
        assert_eq!(m.neighbour_mask(cell(0, 0)), Ok(0));
    }

    #[test]
    fn cells_in_a_rectangle_are_clipped_to_the_map() {
        let m = TileMap::new(4, 4, 10.0, 10.0).unwrap();
        let visible = m.cells_in(15.0, 0.0, 20.0, 15.0);
        assert_eq!(visible.len(), 6, "columns 1 to 3, rows 0 and 1");
        assert!(visible.contains(&cell(1, 0)) && visible.contains(&cell(3, 1)));
        assert!(m.cells_in(100.0, 100.0, 5.0, 5.0).is_empty());
        assert_eq!(m.cells_in(-5.0, -5.0, 1000.0, 1000.0).len(), 16);
    }
}
