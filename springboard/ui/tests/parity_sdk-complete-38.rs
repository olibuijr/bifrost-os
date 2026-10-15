//! Parity sdk-complete-38: headless tests for the fetch predicates (`IN`, `BETWEEN`, prefix match) and the skip and
//! position command models. Every test works on in-memory records and plain values.

use bifrost_ui::fetch::{fetch, FetchRequest, Field, Predicate, Record};
use bifrost_ui::remote_command::{seek_target, CommandError, SkipIntervalCommand};

struct Item {
    name: &'static str,
    rank: i64,
    done: bool,
}

impl Record for Item {
    fn field(&self, key: &str) -> Option<Field> {
        match key {
            "name" => Some(Field::Text(self.name.into())),
            "rank" => Some(Field::Int(self.rank)),
            "done" => Some(Field::Bool(self.done)),
            _ => None,
        }
    }
}

fn items() -> Vec<Item> {
    vec![
        Item { name: "Birch", rank: 2, done: false },
        Item { name: "alder", rank: 5, done: true },
        Item { name: "Cedar", rank: 2, done: true },
        Item { name: "Aspen", rank: 9, done: false },
    ]
}

fn names(request: FetchRequest) -> Vec<&'static str> {
    let all = items();
    fetch(&all, &request).iter().map(|i| i.name).collect()
}

#[test]
fn headless_one_of_selects_records_whose_field_is_in_the_set() {
    let p = Predicate::one_of("rank", vec![Field::Int(2), Field::Int(9)]);
    assert_eq!(names(FetchRequest::new().filter(p)), ["Birch", "Cedar", "Aspen"]);
}

#[test]
fn headless_an_empty_set_matches_nothing() {
    let p = Predicate::one_of("rank", Vec::new());
    assert!(names(FetchRequest::new().filter(p)).is_empty());
}

#[test]
fn headless_between_includes_both_bounds() {
    let p = Predicate::between("rank", Field::Int(2), Field::Int(5));
    assert_eq!(names(FetchRequest::new().filter(p)), ["Birch", "alder", "Cedar"]);
}

#[test]
fn headless_between_compares_text_with_the_text_order() {
    // "alder", "birch" and "aspen" sort between "a" and "c"; "cedar" sorts after "c" and is left out.
    let p = Predicate::between("name", Field::Text("a".into()), Field::Text("c".into()));
    assert_eq!(names(FetchRequest::new().filter(p)), ["Birch", "alder", "Aspen"]);
}

#[test]
fn headless_a_missing_field_never_matches_in_or_between() {
    let one_of = Predicate::one_of("colour", vec![Field::Null]);
    let between = Predicate::between("colour", Field::Int(0), Field::Int(9));
    assert!(names(FetchRequest::new().filter(one_of)).is_empty());
    assert!(names(FetchRequest::new().filter(between)).is_empty());
}

#[test]
fn headless_starts_with_ignores_case_and_needs_text() {
    assert_eq!(names(FetchRequest::new().filter(Predicate::starts_with("name", "ce"))), ["Cedar"]);
    assert_eq!(names(FetchRequest::new().filter(Predicate::starts_with("name", "AL"))), ["alder"]);
    assert!(names(FetchRequest::new().filter(Predicate::starts_with("rank", "2"))).is_empty());
}

#[test]
fn headless_new_predicates_combine_with_the_boolean_operators() {
    let p = Predicate::between("rank", Field::Int(2), Field::Int(5)).and(Predicate::starts_with("name", "b").negate());
    assert_eq!(names(FetchRequest::new().filter(p)), ["alder", "Cedar"]);
}

#[test]
fn headless_a_skip_interval_needs_a_finite_positive_length() {
    assert!(SkipIntervalCommand::new(0.0).is_none());
    assert!(SkipIntervalCommand::new(-5.0).is_none());
    assert!(SkipIntervalCommand::new(f64::NAN).is_none());
    let skip = SkipIntervalCommand::new(15.0).expect("a positive interval");
    assert_eq!(skip.interval_secs(), 15.0);
    assert!(skip.is_enabled());
}

#[test]
fn headless_a_forward_skip_stops_at_the_end_of_the_item() {
    let skip = SkipIntervalCommand::new(15.0).unwrap();
    assert_eq!(skip.request(true, 10_000, 30_000), Ok(25_000));
    assert_eq!(skip.request(true, 20_000, 30_000), Ok(30_000));
}

#[test]
fn headless_a_backward_skip_stops_at_zero() {
    let skip = SkipIntervalCommand::new(15.0).unwrap();
    assert_eq!(skip.request(false, 40_000, 60_000), Ok(25_000));
    assert_eq!(skip.request(false, 10_000, 60_000), Ok(0));
}

#[test]
fn headless_a_forward_skip_of_unknown_length_is_not_clamped() {
    let skip = SkipIntervalCommand::new(15.0).unwrap();
    assert_eq!(skip.request(true, 10_000, 0), Ok(25_000));
}

#[test]
fn headless_a_disabled_skip_refuses_and_keeps_its_interval() {
    let mut skip = SkipIntervalCommand::new(15.0).unwrap();
    skip.set_enabled(false);
    assert_eq!(skip.request(true, 0, 60_000), Err(CommandError::Disabled));
    assert_eq!(skip.interval_secs(), 15.0);
    skip.set_enabled(true);
    assert_eq!(skip.request(true, 0, 60_000), Ok(15_000));
}

#[test]
fn headless_a_position_request_is_clamped_to_the_item() {
    assert_eq!(seek_target(45_000, 30_000), 30_000);
    assert_eq!(seek_target(12_000, 30_000), 12_000);
    assert_eq!(seek_target(12_000, 0), 12_000, "an unknown length keeps the requested position");
}
