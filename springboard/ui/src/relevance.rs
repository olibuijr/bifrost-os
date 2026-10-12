//! Relevance ranking for a stack of glanceable content: the relevance hint of a timeline entry, and the order of a stack.
//! An item is eligible while `now` is inside its window: from its date for `duration` seconds. Eligible items come
//! first, highest score first. Equal scores, and items outside their window, keep the order they were given in.
//! Dates are seconds since 1970-01-01 as `f64`, as in `timeline`.
//!
//! Time clues (`RelevantContext.date(_:)`, `date(_:kind:)`, `date(interval:kind:)`, `date(range:kind:)` and
//! `date(from:to:)`) tell the stack when an item is relevant. A clue is a window in time plus a `DateKind` hint that
//! moves its score up or down. The official pages give the hint names but not the amounts, so the offsets here are
//! this model's choice. A point clue has no end in the official API; here it stays active for `POINT_CLUE_SECONDS`.

/// Seconds a point clue stays active after its date. The official API gives no duration, so this is a local choice.
pub const POINT_CLUE_SECONDS: f64 = 3600.0;

/// The hint a time clue carries (`RelevantContext.DateKind`): default, informational or scheduled.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DateKind {
    /// Default priority: no change to the score.
    Default,
    /// Shows content and needs no action: a slightly lower score.
    Informational,
    /// Shows important content or needs action: a higher score.
    Scheduled,
}

impl DateKind {
    /// The amount added to the item's score for this hint.
    pub fn score_offset(self) -> f32 {
        match self {
            DateKind::Default => 0.0,
            DateKind::Informational => -0.5,
            DateKind::Scheduled => 1.0,
        }
    }
}

/// Why a time clue was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClueError {
    /// A bound is NaN or infinite.
    NotFinite,
    /// The end comes before the start, or an open interval has no length.
    EmptyWindow,
}

/// A time-based relevance clue: the window it is active in, and its hint.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TimeClue {
    start: f64,
    end: f64,
    /// A closed window is active at its end too (a date range); an open one is not (an interval).
    closed: bool,
    kind: DateKind,
}

impl TimeClue {
    /// Relevant at one date. The clue is active from `date` for `POINT_CLUE_SECONDS`.
    pub fn at(date: f64, kind: DateKind) -> Result<Self, ClueError> {
        check_finite(date)?;
        Ok(TimeClue { start: date, end: date + POINT_CLUE_SECONDS, closed: false, kind })
    }

    /// Relevant for an interval. The start is included and the end is not.
    pub fn interval(start: f64, end: f64, kind: DateKind) -> Result<Self, ClueError> {
        let (start, end) = check_bounds(start, end, false)?;
        Ok(TimeClue { start, end, closed: false, kind })
    }

    /// Relevant for a known date range. Both ends are included. A single instant (start equal to end) is allowed.
    pub fn range(start: f64, end: f64, kind: DateKind) -> Result<Self, ClueError> {
        let (start, end) = check_bounds(start, end, true)?;
        Ok(TimeClue { start, end, closed: true, kind })
    }

    /// Relevant between two dates, with the default hint. The start is included and the end is not.
    pub fn between(from: f64, to: f64) -> Result<Self, ClueError> {
        Self::interval(from, to, DateKind::Default)
    }

    pub fn kind(&self) -> DateKind {
        self.kind
    }

    /// The first and last instants of the window, in seconds since 1970-01-01.
    pub fn window(&self) -> (f64, f64) {
        (self.start, self.end)
    }

    /// Whether the clue is active at `now`.
    pub fn is_active(&self, now: f64) -> bool {
        now >= self.start && (now < self.end || (self.closed && now == self.end))
    }

    /// The score the clue adds at `now`: `base` plus the hint's offset, or None outside the window.
    pub fn score_at(&self, base: f32, now: f64) -> Option<f32> {
        self.is_active(now).then(|| base + self.kind.score_offset())
    }
}

fn check_finite(v: f64) -> Result<(), ClueError> {
    if v.is_finite() {
        Ok(())
    } else {
        Err(ClueError::NotFinite)
    }
}

/// Checks both bounds. An open window needs `end > start`; a closed one needs `end >= start`.
fn check_bounds(start: f64, end: f64, closed: bool) -> Result<(f64, f64), ClueError> {
    check_finite(start)?;
    check_finite(end)?;
    let empty = if closed { end < start } else { end <= start };
    if empty {
        return Err(ClueError::EmptyWindow);
    }
    Ok((start, end))
}

#[cfg(test)]
mod time_clue_tests {
    use super::*;

    #[test]
    fn a_point_clue_is_active_for_one_hour_from_its_date() {
        let c = TimeClue::at(1000.0, DateKind::Default).unwrap();
        assert_eq!(c.window(), (1000.0, 1000.0 + POINT_CLUE_SECONDS));
        assert!(!c.is_active(999.0));
        assert!(c.is_active(1000.0));
        assert!(c.is_active(4599.0));
        assert!(!c.is_active(4600.0), "open at the end of the hour");
    }

    #[test]
    fn an_interval_includes_its_start_and_excludes_its_end() {
        let c = TimeClue::interval(100.0, 200.0, DateKind::Default).unwrap();
        assert!(c.is_active(100.0));
        assert!(c.is_active(199.5));
        assert!(!c.is_active(200.0));
    }

    #[test]
    fn a_range_includes_both_ends() {
        let c = TimeClue::range(100.0, 200.0, DateKind::Default).unwrap();
        assert!(c.is_active(200.0), "closed at the end");
        assert!(!c.is_active(200.5));
        let instant = TimeClue::range(50.0, 50.0, DateKind::Default).unwrap();
        assert!(instant.is_active(50.0), "a single instant is a valid closed range");
        assert!(!instant.is_active(50.1));
    }

    #[test]
    fn empty_windows_and_non_finite_bounds_are_refused() {
        assert_eq!(TimeClue::interval(100.0, 100.0, DateKind::Default), Err(ClueError::EmptyWindow));
        assert_eq!(TimeClue::interval(200.0, 100.0, DateKind::Default), Err(ClueError::EmptyWindow));
        assert_eq!(TimeClue::range(200.0, 100.0, DateKind::Default), Err(ClueError::EmptyWindow));
        assert_eq!(TimeClue::between(f64::NAN, 10.0), Err(ClueError::NotFinite));
        assert_eq!(TimeClue::at(f64::INFINITY, DateKind::Scheduled), Err(ClueError::NotFinite));
    }

    #[test]
    fn the_hint_moves_the_score_up_or_down_only_inside_the_window() {
        let base = 2.0_f32;
        let scheduled = TimeClue::interval(0.0, 10.0, DateKind::Scheduled).unwrap();
        let informational = TimeClue::interval(0.0, 10.0, DateKind::Informational).unwrap();
        let default = TimeClue::between(0.0, 10.0).unwrap();
        assert_eq!(scheduled.score_at(base, 5.0), Some(3.0));
        assert_eq!(informational.score_at(base, 5.0), Some(1.5));
        assert_eq!(default.score_at(base, 5.0), Some(2.0));
        assert_eq!(scheduled.score_at(base, 10.0), None);
        assert_eq!(default.kind(), DateKind::Default);
    }
}

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
