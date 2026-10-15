//! Tab order: where each tab lands when one tab moves along the tab bar. The active tab and a split pair
//! keep their sessions, so their indexes are remapped with the same rule.

/// The index session `i` has once the session at `from` is removed and inserted at `to`.
pub fn moved_index(i: usize, from: usize, to: usize) -> usize {
    if i == from {
        to
    } else if from < to && i > from && i <= to {
        i - 1
    } else if from > to && i >= to && i < from {
        i + 1
    } else {
        i
    }
}

/// The position one step forward or back from `at` among `len` tabs; None at either end of the bar.
pub fn neighbour(len: usize, at: usize, forward: bool) -> Option<usize> {
    if at >= len {
        return None;
    }
    if forward {
        (at + 1 < len).then_some(at + 1)
    } else {
        at.checked_sub(1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_moved_tab_takes_the_target_index() {
        assert_eq!(moved_index(0, 0, 2), 2);
        assert_eq!(moved_index(2, 2, 0), 0);
    }

    #[test]
    fn tabs_between_the_ends_shift_by_one() {
        assert_eq!(moved_index(1, 0, 2), 0);
        assert_eq!(moved_index(2, 0, 2), 1);
        assert_eq!(moved_index(0, 2, 0), 1);
        assert_eq!(moved_index(1, 2, 0), 2);
    }

    #[test]
    fn tabs_outside_the_move_keep_their_index() {
        assert_eq!(moved_index(3, 0, 2), 3);
        assert_eq!(moved_index(0, 1, 2), 0);
    }

    #[test]
    fn neighbour_stops_at_both_ends() {
        assert_eq!(neighbour(3, 0, true), Some(1));
        assert_eq!(neighbour(3, 1, false), Some(0));
        assert_eq!(neighbour(3, 2, true), None);
        assert_eq!(neighbour(3, 0, false), None);
        assert_eq!(neighbour(0, 0, true), None);
    }
}
