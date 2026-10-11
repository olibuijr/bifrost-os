//! Grid navigation: a rectangle of cells, some blocked, and the shortest route between two cells (A* search).
//! Coordinates are integers and a grid may start at any origin. Diagonal steps are optional and never cut
//! a blocked corner. Costs are integers (10 per straight step, 14 per diagonal), so the search needs no floats.

use std::cmp::Reverse;
use std::collections::BinaryHeap;

const STRAIGHT: u32 = 10;
const DIAGONAL: u32 = 14;

/// A cell in grid coordinates. Negative values are fine.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Cell {
    pub x: i32,
    pub y: i32,
}

/// A rectangular grid of passable and blocked cells.
#[derive(Clone, Debug)]
pub struct GridGraph {
    origin: Cell,
    width: i32,
    height: i32,
    diagonals: bool,
    /// Row-major, one flag per cell: true when the cell is blocked.
    blocked: Vec<bool>,
}

impl GridGraph {
    /// A `width` by `height` grid whose lowest corner is `origin`. Negative sizes become 0.
    pub fn new(origin: Cell, width: i32, height: i32, diagonals: bool) -> Self {
        let width = width.max(0);
        let height = height.max(0);
        Self {
            origin,
            width,
            height,
            diagonals,
            blocked: vec![false; (width as usize) * (height as usize)],
        }
    }

    pub fn origin(&self) -> Cell {
        self.origin
    }

    pub fn width(&self) -> i32 {
        self.width
    }

    pub fn height(&self) -> i32 {
        self.height
    }

    pub fn diagonals_allowed(&self) -> bool {
        self.diagonals
    }

    /// Whether the cell lies inside the grid, blocked or not.
    pub fn contains(&self, cell: Cell) -> bool {
        self.slot(cell).is_some()
    }

    /// Whether the cell lies inside the grid and is not blocked.
    pub fn is_open(&self, cell: Cell) -> bool {
        self.open_slot(cell).is_some()
    }

    /// Blocks or unblocks a cell. Returns false when the cell is outside the grid.
    pub fn set_blocked(&mut self, cell: Cell, blocked: bool) -> bool {
        match self.slot(cell) {
            Some(i) => {
                self.blocked[i] = blocked;
                true
            }
            None => false,
        }
    }

    /// The cheapest route from `start` to `goal`, both ends included. None when either end is blocked or
    /// outside the grid, or when no route exists.
    pub fn find_path(&self, start: Cell, goal: Cell) -> Option<Vec<Cell>> {
        let from = self.open_slot(start)?;
        let to = self.open_slot(goal)?;
        let mut best = vec![u32::MAX; self.blocked.len()];
        let mut came_from: Vec<Option<usize>> = vec![None; self.blocked.len()];
        let mut open = BinaryHeap::new();
        best[from] = 0;
        open.push(Reverse((self.estimate(start, goal), 0, from)));
        while let Some(Reverse((_, cost, cur))) = open.pop() {
            if cur == to {
                let mut path = vec![self.cell_at(cur)];
                let mut at = cur;
                while let Some(prev) = came_from[at] {
                    path.push(self.cell_at(prev));
                    at = prev;
                }
                path.reverse();
                return Some(path);
            }
            // A cheaper entry for this cell was already expanded; this one is stale.
            if cost > best[cur] {
                continue;
            }
            for (next, step) in self.neighbours(self.cell_at(cur)) {
                let next_cost = cost + step;
                if next_cost < best[next] {
                    best[next] = next_cost;
                    came_from[next] = Some(cur);
                    let guess = next_cost + self.estimate(self.cell_at(next), goal);
                    open.push(Reverse((guess, next_cost, next)));
                }
            }
        }
        None
    }

    /// Index of a cell in the row-major vector, if the cell is inside the grid.
    fn slot(&self, cell: Cell) -> Option<usize> {
        let dx = i64::from(cell.x) - i64::from(self.origin.x);
        let dy = i64::from(cell.y) - i64::from(self.origin.y);
        if dx < 0 || dy < 0 || dx >= i64::from(self.width) || dy >= i64::from(self.height) {
            return None;
        }
        Some(dy as usize * self.width as usize + dx as usize)
    }

    fn open_slot(&self, cell: Cell) -> Option<usize> {
        self.slot(cell).filter(|&i| !self.blocked[i])
    }

    /// The cell at a row-major index. Only called with indices that `slot` returned.
    fn cell_at(&self, slot: usize) -> Cell {
        let w = self.width as usize;
        Cell {
            x: self.origin.x + (slot % w) as i32,
            y: self.origin.y + (slot / w) as i32,
        }
    }

    /// Lower bound on the cost from `a` to `b`, ignoring walls. It never overestimates, so A* stays optimal.
    fn estimate(&self, a: Cell, b: Cell) -> u32 {
        let dx = (i64::from(a.x) - i64::from(b.x)).unsigned_abs() as u32;
        let dy = (i64::from(a.y) - i64::from(b.y)).unsigned_abs() as u32;
        let short = dx.min(dy);
        let long = dx.max(dy);
        if self.diagonals {
            short * DIAGONAL + (long - short) * STRAIGHT
        } else {
            (dx + dy) * STRAIGHT
        }
    }

    /// Open neighbours of a cell, as (index, step cost). A diagonal step needs both cells beside it open.
    fn neighbours(&self, c: Cell) -> Vec<(usize, u32)> {
        let mut out = Vec::with_capacity(8);
        for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
            let n = Cell {
                x: c.x.saturating_add(dx),
                y: c.y.saturating_add(dy),
            };
            if let Some(i) = self.open_slot(n) {
                out.push((i, STRAIGHT));
            }
        }
        if self.diagonals {
            for (dx, dy) in [(1, 1), (1, -1), (-1, 1), (-1, -1)] {
                let beside_x = Cell {
                    x: c.x.saturating_add(dx),
                    y: c.y,
                };
                let beside_y = Cell {
                    x: c.x,
                    y: c.y.saturating_add(dy),
                };
                if !(self.is_open(beside_x) && self.is_open(beside_y)) {
                    continue;
                }
                let n = Cell {
                    x: c.x.saturating_add(dx),
                    y: c.y.saturating_add(dy),
                };
                if let Some(i) = self.open_slot(n) {
                    out.push((i, DIAGONAL));
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cell(x: i32, y: i32) -> Cell {
        Cell { x, y }
    }

    /// Every step moves to a neighbouring cell and every cell on the path is open.
    fn is_walkable(grid: &GridGraph, path: &[Cell]) -> bool {
        path.iter().all(|&c| grid.is_open(c))
            && path.windows(2).all(|w| {
                let dx = (w[1].x - w[0].x).abs();
                let dy = (w[1].y - w[0].y).abs();
                dx <= 1 && dy <= 1 && (dx, dy) != (0, 0)
            })
    }

    #[test]
    fn a_straight_route_on_an_open_grid_visits_each_cell_on_the_way() {
        let g = GridGraph::new(cell(0, 0), 5, 1, false);
        let path = g.find_path(cell(0, 0), cell(3, 0)).unwrap();
        assert_eq!(path, vec![cell(0, 0), cell(1, 0), cell(2, 0), cell(3, 0)]);
    }

    #[test]
    fn a_wall_forces_a_detour_through_the_gap() {
        // Column x = 1 is blocked at y = 0 and y = 1; the only gap is at y = 2.
        let mut g = GridGraph::new(cell(0, 0), 3, 3, false);
        assert!(g.set_blocked(cell(1, 0), true));
        assert!(g.set_blocked(cell(1, 1), true));
        let path = g.find_path(cell(0, 0), cell(2, 0)).unwrap();
        assert!(path.contains(&cell(1, 2)), "must pass the gap: {path:?}");
        assert_eq!(path.len(), 7);
        assert!(is_walkable(&g, &path));
    }

    #[test]
    fn no_route_and_blocked_or_outside_ends_give_none() {
        let mut g = GridGraph::new(cell(0, 0), 3, 3, true);
        for y in 0..3 {
            g.set_blocked(cell(1, y), true);
        }
        assert_eq!(g.find_path(cell(0, 0), cell(2, 2)), None, "the wall splits the grid");
        assert_eq!(g.find_path(cell(1, 1), cell(0, 0)), None, "a blocked start");
        assert_eq!(g.find_path(cell(0, 0), cell(9, 9)), None, "a goal outside the grid");
        assert_eq!(g.find_path(cell(0, 0), cell(0, 0)), Some(vec![cell(0, 0)]));
    }

    #[test]
    fn diagonal_steps_shorten_the_route_when_they_are_allowed() {
        let diagonal = GridGraph::new(cell(0, 0), 5, 5, true);
        assert_eq!(diagonal.find_path(cell(0, 0), cell(2, 2)).unwrap().len(), 3);
        let plain = GridGraph::new(cell(0, 0), 5, 5, false);
        assert_eq!(plain.find_path(cell(0, 0), cell(2, 2)).unwrap().len(), 5);
    }

    #[test]
    fn a_diagonal_step_never_cuts_a_blocked_corner() {
        let mut g = GridGraph::new(cell(0, 0), 3, 3, true);
        g.set_blocked(cell(1, 0), true);
        let path = g.find_path(cell(0, 0), cell(1, 1)).unwrap();
        assert_eq!(path, vec![cell(0, 0), cell(0, 1), cell(1, 1)]);
    }

    #[test]
    fn negative_origins_and_cells_outside_the_grid_are_handled() {
        let mut g = GridGraph::new(cell(-2, -2), 3, 3, false);
        assert!(g.contains(cell(-2, -2)) && g.contains(cell(0, 0)));
        assert!(!g.contains(cell(1, 0)) && !g.contains(cell(-3, 0)));
        assert!(!g.set_blocked(cell(5, 5), true));
        assert!(g.set_blocked(cell(0, 0), true));
        let path = g.find_path(cell(-2, 0), cell(0, -2)).unwrap();
        assert!(is_walkable(&g, &path));
        assert_eq!(path.len(), 5, "a shortest route around the blocked corner: {path:?}");
        assert_eq!(path.first(), Some(&cell(-2, 0)));
        assert_eq!(path.last(), Some(&cell(0, -2)));
    }
}
