//! Reordering the saved places: the list move used by the sidebar drag and the Location menu, and the index the
//! selected place takes after a move. Pure list logic, so the screen and the menu share one rule.

/// Moves the item at `from` to position `to` (both must be in range), keeping the others in order. Returns false and
/// leaves the list alone when either index is out of range or they are equal.
pub fn move_item<T>(items: &mut [T], from: usize, to: usize) -> bool {
    if from == to || from >= items.len() || to >= items.len() {
        return false;
    }
    if from < to {
        items[from..=to].rotate_left(1);
    } else {
        items[to..=from].rotate_right(1);
    }
    true
}

/// Where the item that was at `index` ends up after `move_item(items, from, to)`.
pub fn follow_move(index: usize, from: usize, to: usize) -> usize {
    if index == from {
        to
    } else if from < to && index > from && index <= to {
        index - 1
    } else if to < from && index >= to && index < from {
        index + 1
    } else {
        index
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_item_moves_down_and_the_others_shift_up() {
        let mut v = vec!["a", "b", "c", "d"];
        assert!(move_item(&mut v, 0, 2));
        assert_eq!(v, vec!["b", "c", "a", "d"]);
    }

    #[test]
    fn an_item_moves_up_and_the_others_shift_down() {
        let mut v = vec!["a", "b", "c", "d"];
        assert!(move_item(&mut v, 3, 1));
        assert_eq!(v, vec!["a", "d", "b", "c"]);
    }

    #[test]
    fn a_move_to_the_same_place_or_out_of_range_changes_nothing() {
        let mut v = vec![1, 2, 3];
        assert!(!move_item(&mut v, 1, 1));
        assert!(!move_item(&mut v, 3, 0));
        assert!(!move_item(&mut v, 0, 3));
        assert_eq!(v, vec![1, 2, 3]);
    }

    #[test]
    fn the_selection_follows_its_place_through_a_move() {
        // move item 0 to 2 in [a, b, c, d]: b, c, a, d; the selected item "a" (0) goes to 2
        assert_eq!(follow_move(0, 0, 2), 2);
        // the items it jumped over shift up by one
        assert_eq!(follow_move(1, 0, 2), 0);
        assert_eq!(follow_move(2, 0, 2), 1);
        // items after the range stay put
        assert_eq!(follow_move(3, 0, 2), 3);
        // moving up: [a, b, c, d], move 3 to 1 gives a, d, b, c; "c" (2) moves to 3 and "b" (1) to 2
        assert_eq!(follow_move(3, 3, 1), 1);
        assert_eq!(follow_move(2, 3, 1), 3);
        assert_eq!(follow_move(1, 3, 1), 2);
        assert_eq!(follow_move(0, 3, 1), 0);
    }

    #[test]
    fn follow_move_agrees_with_move_item() {
        for from in 0..4 {
            for to in 0..4 {
                let mut v: Vec<usize> = (0..4).collect();
                move_item(&mut v, from, to);
                for index in 0..4 {
                    assert_eq!(v[follow_move(index, from, to)], index, "from {from} to {to}");
                }
            }
        }
    }
}
