//! Relevance ranking for a stack of glanceable content: the relevance hint of a timeline entry, and the order of a stack.
//! An item is eligible while `now` is inside its window: from its date for `duration` seconds. Eligible items come
//! first, highest score first. Equal scores, and items outside their window, keep the order they were given in.
//! Dates are seconds since 1970-01-01 as `f64`, as in `timeline`.

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Relevance {
    /// Importance compared with the other items of the stack. Higher comes first.
    pub score: f32,
    /// Seconds after the item's date during which it is eligible for the front.
    pub duration: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Item {
    pub date: f64,
    /// None means the item never rotates to the front.
    pub relevance: Option<Relevance>,
}

/// The score the item counts for at `now`, or None outside its window. The window is closed at its start and open
/// at its end.
pub fn score_at(item: &Item, now: f64) -> Option<f32> {
    let r = item.relevance?;
    (now >= item.date && now < item.date + r.duration).then_some(r.score)
}

/// Indices of `items` in display order at `now`.
pub fn rank(items: &[Item], now: f64) -> Vec<usize> {
    let mut order: Vec<(usize, Option<f32>)> = items.iter().enumerate().map(|(i, it)| (i, score_at(it, now))).collect();
    // `sort_by` is stable, so ties and ineligible items keep their given order.
    order.sort_by(|a, b| match (a.1, b.1) {
        (Some(x), Some(y)) => y.total_cmp(&x),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => std::cmp::Ordering::Equal,
    });
    order.into_iter().map(|(i, _)| i).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(date: f64, score: f32, duration: f64) -> Item {
        Item { date, relevance: Some(Relevance { score, duration }) }
    }

    #[test]
    fn eligible_items_come_first_highest_score_first() {
        let items = [item(0.0, 1.0, 100.0), item(0.0, 3.0, 100.0), item(0.0, 2.0, 100.0)];
        assert_eq!(rank(&items, 10.0), vec![1, 2, 0]);
    }

    #[test]
    fn equal_scores_keep_the_given_order() {
        let items = [item(0.0, 2.0, 100.0), item(0.0, 2.0, 100.0), item(0.0, 2.0, 100.0)];
        assert_eq!(rank(&items, 10.0), vec![0, 1, 2]);
    }

    #[test]
    fn an_item_is_eligible_from_its_date_until_its_window_closes() {
        let it = item(100.0, 5.0, 50.0);
        assert_eq!(score_at(&it, 99.9), None, "not yet");
        assert_eq!(score_at(&it, 100.0), Some(5.0), "closed at the start");
        assert_eq!(score_at(&it, 149.9), Some(5.0));
        assert_eq!(score_at(&it, 150.0), None, "open at the end");
    }

    #[test]
    fn items_outside_their_window_fall_behind_in_their_given_order() {
        let items = [item(0.0, 9.0, 10.0), item(0.0, 1.0, 100.0), item(500.0, 8.0, 10.0), item(0.0, 4.0, 100.0)];
        // At t = 50: index 0 has expired, index 2 is not due yet; 1 and 3 are eligible.
        assert_eq!(rank(&items, 50.0), vec![3, 1, 0, 2]);
    }

    #[test]
    fn an_item_without_relevance_never_rotates_to_the_front() {
        let items = [Item { date: 0.0, relevance: None }, item(0.0, -1.0, 100.0)];
        assert_eq!(score_at(&items[0], 10.0), None);
        assert_eq!(rank(&items, 10.0), vec![1, 0], "a negative score that is in its window still beats no relevance");
    }

    #[test]
    fn an_empty_stack_ranks_to_nothing() {
        assert!(rank(&[], 0.0).is_empty());
    }
}
