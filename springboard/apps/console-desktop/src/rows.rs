//! Row geometry when messages are expanded. Every row is `row` points tall; an expanded row is taller by the extra
//! lines its message needs. Rows are named by their index in the shown list. Pure arithmetic: the window measures the
//! text and hands in the extra heights.

/// One expanded row: its index in the shown list and how much taller than a plain row it is (points).
pub type Extra = (usize, f32);

/// The expanded rows of the shown list, sorted by index, over rows of `row` points.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Rows {
    row: f32,
    extras: Vec<Extra>,
}

impl Rows {
    /// `extras` may come in any order; entries with no extra height are dropped, and an index named twice counts once.
    pub fn new(row: f32, mut extras: Vec<Extra>) -> Rows {
        extras.retain(|e| e.1 > 0.0);
        extras.sort_by_key(|e| e.0);
        extras.dedup_by_key(|e| e.0);
        Rows { row, extras }
    }

    /// The extra height of the rows above row `index`.
    fn above(&self, index: usize) -> f32 {
        self.extras.iter().take_while(|e| e.0 < index).map(|e| e.1).sum()
    }

    /// The top of row `index`, from the top of the first row.
    pub fn top_of(&self, index: usize) -> f32 {
        index as f32 * self.row + self.above(index)
    }

    /// The height of row `index`.
    pub fn height_of(&self, index: usize) -> f32 {
        self.row + self.extras.iter().find(|e| e.0 == index).map_or(0.0, |e| e.1)
    }

    /// The height of `count` rows together.
    pub fn content_height(&self, count: usize) -> f32 {
        count as f32 * self.row + self.above(count)
    }

    /// The index of the row that holds `y`, measured from the top of the first row. May be one past the last row.
    pub fn locate(&self, y: f32) -> usize {
        if y <= 0.0 {
            return 0;
        }
        let mut gained = 0.0;
        for &(i, extra) in &self.extras {
            let top = i as f32 * self.row + gained;
            if y < top {
                break;
            }
            if y < top + self.row + extra {
                return i;
            }
            gained += extra;
        }
        ((y - gained) / self.row).floor() as usize
    }

    /// The scroll offset that brings row `index` into a view `view` points tall, from the current `offset`: unchanged
    /// when the row is already whole in view, else the smallest move that shows it (a row taller than the view lines
    /// up with its top).
    pub fn reveal(&self, index: usize, offset: f32, view: f32) -> f32 {
        let top = self.top_of(index);
        let bottom = top + self.height_of(index);
        if top < offset || bottom - top > view {
            top
        } else if bottom > offset + view {
            bottom - view
        } else {
            offset
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_rows_are_evenly_spaced() {
        let r = Rows::new(10.0, Vec::new());
        assert_eq!(r.top_of(3), 30.0);
        assert_eq!(r.height_of(3), 10.0);
        assert_eq!(r.content_height(5), 50.0);
        assert_eq!(r.locate(0.0), 0);
        assert_eq!(r.locate(29.9), 2);
        assert_eq!(r.locate(30.0), 3);
        assert_eq!(r.locate(-4.0), 0);
    }

    #[test]
    fn an_expanded_row_pushes_the_rows_below_it() {
        let r = Rows::new(10.0, vec![(2, 30.0)]);
        assert_eq!(r.top_of(2), 20.0);
        assert_eq!(r.height_of(2), 40.0);
        assert_eq!(r.top_of(3), 60.0);
        assert_eq!(r.content_height(4), 70.0);
        assert_eq!(r.content_height(2), 20.0);
    }

    #[test]
    fn locate_finds_rows_around_an_expanded_one() {
        let r = Rows::new(10.0, vec![(2, 30.0)]);
        assert_eq!(r.locate(19.9), 1);
        assert_eq!(r.locate(20.0), 2);
        assert_eq!(r.locate(59.9), 2, "the whole expanded row answers to row 2");
        assert_eq!(r.locate(60.0), 3);
        assert_eq!(r.locate(70.0), 4);
        for i in 0..8 {
            assert_eq!(r.locate(r.top_of(i)), i, "the top of row {i} is row {i}");
        }
    }

    #[test]
    fn several_expanded_rows_add_up_in_any_order() {
        let r = Rows::new(10.0, vec![(5, 10.0), (1, 20.0), (5, 99.0), (3, 0.0)]);
        assert_eq!(r.top_of(2), 40.0, "row 1 is 30 tall");
        assert_eq!(r.top_of(6), 90.0);
        assert_eq!(r.height_of(3), 10.0, "an extra of zero is no extra");
        for i in 0..10 {
            assert_eq!(r.locate(r.top_of(i) + r.height_of(i) - 0.1), i);
        }
    }

    #[test]
    fn reveal_moves_the_least_that_shows_the_row() {
        let r = Rows::new(10.0, vec![(4, 20.0)]);
        assert_eq!(r.reveal(1, 0.0, 50.0), 0.0, "already in view");
        assert_eq!(r.reveal(2, 0.0, 50.0), 0.0);
        assert_eq!(r.reveal(4, 0.0, 50.0), 20.0, "scroll down until its bottom (70) meets the view's");
        assert_eq!(r.reveal(0, 30.0, 50.0), 0.0, "scroll up to its top");
        assert_eq!(r.reveal(4, 0.0, 25.0), 40.0, "a row taller than the view lines up with its top");
    }
}
