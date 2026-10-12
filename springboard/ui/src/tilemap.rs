//! Tile map model: a grid of columns and rows, each cell holding one tile group from a tile set, or nothing. It is the
//! counterpart of the 2D scene framework's tile map node and tile set. It converts between grid cells and node
//! positions, and reads the neighbours of a cell so an auto-tiling rule can pick the matching edge or corner piece.
//!
//! Pure: no images and no drawing. The renderer maps a group name to its art. Rows count upwards from the bottom, as
//! the scene does. The anchor sets which point of the map sits at the node's position: (0, 0) is the bottom-left
//! corner and (0.5, 0.5) the centre.

/// The named tile groups a map can use. A group index is its position in the set.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct TileSet {
    names: Vec<String>,
}

impl TileSet {
    pub fn new() -> TileSet {
        TileSet { names: Vec::new() }
    }

    /// Adds a group and returns its index.
    pub fn add_group(&mut self, name: &str) -> usize {
        self.names.push(name.to_string());
        self.names.len() - 1
    }

    pub fn group_name(&self, index: usize) -> Option<&str> {
        self.names.get(index).map(|s| s.as_str())
    }

    pub fn group_count(&self) -> usize {
        self.names.len()
    }
}

/// A grid of cells. Each cell is `None` or the index of a group in the tile set.
#[derive(Clone, Debug, PartialEq)]
pub struct TileMap {
    pub tile_set: TileSet,
    pub columns: usize,
    pub rows: usize,
    /// Size of one cell, in points.
    pub tile_width: f32,
    pub tile_height: f32,
    pub anchor: (f32, f32),
    cells: Vec<Option<usize>>,
}

impl TileMap {
    /// A map of `columns` by `rows` cells, all set to `fill` (`None` leaves them empty).
    pub fn new(
        tile_set: TileSet,
        columns: usize,
        rows: usize,
        tile_width: f32,
        tile_height: f32,
        fill: Option<usize>,
    ) -> TileMap {
        TileMap {
            tile_set,
            columns,
            rows,
            tile_width,
            tile_height,
            anchor: (0.0, 0.0),
            cells: vec![fill; columns * rows],
        }
    }

    /// Overall size of the map in points.
    pub fn map_size(&self) -> (f32, f32) {
        (self.columns as f32 * self.tile_width, self.rows as f32 * self.tile_height)
    }

    fn index(&self, column: usize, row: usize) -> Option<usize> {
        if column < self.columns && row < self.rows {
            Some(row * self.columns + column)
        } else {
            None
        }
    }

    /// Sets the group of one cell (`None` clears it). Returns false if the cell is outside the map or the group is
    /// not in the tile set.
    pub fn set_tile(&mut self, column: usize, row: usize, group: Option<usize>) -> bool {
        if let Some(g) = group {
            if g >= self.tile_set.group_count() {
                return false;
            }
        }
        match self.index(column, row) {
            Some(i) => {
                self.cells[i] = group;
                true
            }
            None => false,
        }
    }

    /// The group of one cell, or `None` if the cell is empty or outside the map.
    pub fn tile_group(&self, column: usize, row: usize) -> Option<usize> {
        self.index(column, row).and_then(|i| self.cells[i])
    }

    /// Centre of a cell in map coordinates, before the anchor is applied.
    pub fn center_of_tile(&self, column: usize, row: usize) -> (f32, f32) {
        (
            (column as f32 + 0.5) * self.tile_width,
            (row as f32 + 0.5) * self.tile_height,
        )
    }

    /// The column under an x position in the node's space, or `None` outside the map.
    pub fn column_index_from_x(&self, x: f32) -> Option<usize> {
        let local = x + self.anchor.0 * self.map_size().0;
        cell_index(local, self.tile_width, self.columns)
    }

    /// The row under a y position in the node's space, or `None` outside the map.
    pub fn row_index_from_y(&self, y: f32) -> Option<usize> {
        let local = y + self.anchor.1 * self.map_size().1;
        cell_index(local, self.tile_height, self.rows)
    }

    /// Bit mask of the four neighbours of a cell that hold the same group: north 1, east 2, south 4, west 8. A
    /// neighbour outside the map does not match. An empty cell or a cell outside the map has mask 0.
    pub fn neighbour_mask(&self, column: usize, row: usize) -> u8 {
        let group = match self.tile_group(column, row) {
            Some(g) => g,
            None => return 0,
        };
        let mut mask = 0u8;
        if row + 1 < self.rows && self.tile_group(column, row + 1) == Some(group) {
            mask |= 1;
        }
        if column + 1 < self.columns && self.tile_group(column + 1, row) == Some(group) {
            mask |= 2;
        }
        if row > 0 && self.tile_group(column, row - 1) == Some(group) {
            mask |= 4;
        }
        if column > 0 && self.tile_group(column - 1, row) == Some(group) {
            mask |= 8;
        }
        mask
    }
}

/// Index of the cell that holds `local` (distance from the map's bottom or left edge), or `None` outside `0..count`.
fn cell_index(local: f32, size: f32, count: usize) -> Option<usize> {
    if size <= 0.0 || local < 0.0 {
        return None;
    }
    let i = (local / size).floor() as usize;
    if i < count {
        Some(i)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set_with(names: &[&str]) -> TileSet {
        let mut s = TileSet::new();
        for n in names {
            s.add_group(n);
        }
        s
    }

    #[test]
    fn tile_set_hands_out_indices_in_order() {
        let mut s = TileSet::new();
        assert_eq!(s.add_group("grass"), 0);
        assert_eq!(s.add_group("water"), 1);
        assert_eq!(s.group_name(1), Some("water"));
        assert_eq!(s.group_name(5), None);
        assert_eq!(s.group_count(), 2);
    }

    #[test]
    fn map_size_and_cell_centres_follow_the_tile_size() {
        let map = TileMap::new(set_with(&["a"]), 4, 3, 16.0, 8.0, None);
        assert_eq!(map.map_size(), (64.0, 24.0));
        assert_eq!(map.center_of_tile(0, 0), (8.0, 4.0));
        assert_eq!(map.center_of_tile(3, 2), (56.0, 20.0));
    }

    #[test]
    fn set_tile_rejects_cells_and_groups_that_do_not_exist() {
        let mut map = TileMap::new(set_with(&["grass", "water"]), 4, 3, 16.0, 8.0, None);
        assert!(map.set_tile(1, 1, Some(0)));
        assert_eq!(map.tile_group(1, 1), Some(0));
        assert_eq!(map.tile_group(0, 0), None);
        assert!(!map.set_tile(4, 0, Some(0))); // column 4 is outside 0..4
        assert!(!map.set_tile(0, 0, Some(5))); // group 5 is not in the set
        assert!(map.set_tile(1, 1, None)); // clearing a cell is allowed
        assert_eq!(map.tile_group(1, 1), None);
    }

    #[test]
    fn position_maps_to_the_cell_under_it_with_a_bottom_left_anchor() {
        let map = TileMap::new(set_with(&["a"]), 4, 3, 16.0, 8.0, None);
        assert_eq!(map.column_index_from_x(0.0), Some(0));
        assert_eq!(map.column_index_from_x(15.9), Some(0));
        assert_eq!(map.column_index_from_x(16.0), Some(1));
        assert_eq!(map.column_index_from_x(63.9), Some(3));
        assert_eq!(map.column_index_from_x(64.0), None);
        assert_eq!(map.column_index_from_x(-0.1), None);
        assert_eq!(map.row_index_from_y(23.9), Some(2));
        assert_eq!(map.row_index_from_y(24.0), None);
    }

    #[test]
    fn centre_anchor_shifts_the_map_by_half_its_size() {
        let mut map = TileMap::new(set_with(&["a"]), 4, 3, 16.0, 8.0, None);
        map.anchor = (0.5, 0.5); // the map spans -32..32 in x and -12..12 in y
        assert_eq!(map.column_index_from_x(-32.0), Some(0));
        assert_eq!(map.column_index_from_x(0.0), Some(2));
        assert_eq!(map.column_index_from_x(-33.0), None);
        assert_eq!(map.row_index_from_y(-12.0), Some(0));
        assert_eq!(map.row_index_from_y(12.0), None);
    }

    #[test]
    fn neighbour_mask_counts_only_matching_neighbours_inside_the_map() {
        let mut map = TileMap::new(set_with(&["a"]), 3, 3, 1.0, 1.0, Some(0));
        // Every cell is group 0: the centre has all four neighbours, a corner has two.
        assert_eq!(map.neighbour_mask(1, 1), 1 | 2 | 4 | 8);
        assert_eq!(map.neighbour_mask(0, 0), 1 | 2);
        // Clearing the cell north of the centre removes its bit only.
        map.set_tile(1, 2, None);
        assert_eq!(map.neighbour_mask(1, 1), 2 | 4 | 8);
        // An empty cell has no mask.
        map.set_tile(2, 2, None);
        assert_eq!(map.neighbour_mask(2, 2), 0);
    }
}
