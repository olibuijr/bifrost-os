//! Parity os-gaps-04: headless tests for the app usage ranker behind the launch suggestions. No clock is read: every
//! launch and query carries its own time in seconds.

use springboard::shell::app_usage::{AppUsage, HALF_LIFE_S};

#[test]
fn an_app_never_launched_has_no_score_and_is_not_suggested() {
    let usage = AppUsage::default();
    assert_eq!(usage.score("notes", 0.0), 0.0);
    assert!(usage.suggest(0.0, 5).is_empty());
}

#[test]
fn one_launch_scores_one_at_the_time_it_happened() {
    let mut usage = AppUsage::default();
    usage.record_launch("notes", 100.0);
    assert_eq!(usage.score("notes", 100.0), 1.0);
}

#[test]
fn a_score_halves_each_half_life_after_the_launch() {
    let mut usage = AppUsage::default();
    usage.record_launch("notes", 0.0);
    assert!((usage.score("notes", HALF_LIFE_S) - 0.5).abs() < 1e-12);
    assert!((usage.score("notes", 2.0 * HALF_LIFE_S) - 0.25).abs() < 1e-12);
}

#[test]
fn a_later_launch_adds_one_to_the_score_decayed_up_to_that_launch() {
    let mut usage = AppUsage::default();
    usage.record_launch("notes", 0.0);
    usage.record_launch("notes", HALF_LIFE_S);
    assert!((usage.score("notes", HALF_LIFE_S) - 1.5).abs() < 1e-12);
}

#[test]
fn a_query_before_the_last_launch_does_not_raise_the_score() {
    let mut usage = AppUsage::default();
    usage.record_launch("notes", 100.0);
    assert_eq!(usage.score("notes", 50.0), 1.0);
}

#[test]
fn suggestions_list_the_most_launched_app_first() {
    let mut usage = AppUsage::default();
    usage.record_launch("mail", 0.0);
    for t in 0..3u32 {
        usage.record_launch("notes", f64::from(t));
    }
    assert_eq!(usage.suggest(3.0, 5), vec!["notes", "mail"]);
}

#[test]
fn a_recent_habit_outranks_an_older_one_with_more_launches() {
    let mut usage = AppUsage::default();
    for _ in 0..5 {
        usage.record_launch("old", 0.0);
    }
    usage.record_launch("new", 10.0 * HALF_LIFE_S);
    assert_eq!(usage.suggest(10.0 * HALF_LIFE_S, 1), vec!["new"]);
}

#[test]
fn suggestions_stop_at_the_limit_and_equal_scores_keep_first_launch_order() {
    let mut usage = AppUsage::default();
    usage.record_launch("a", 0.0);
    usage.record_launch("b", 0.0);
    usage.record_launch("c", 0.0);
    assert_eq!(usage.suggest(0.0, 2), vec!["a", "b"]);
    assert!(usage.suggest(0.0, 0).is_empty());
}
