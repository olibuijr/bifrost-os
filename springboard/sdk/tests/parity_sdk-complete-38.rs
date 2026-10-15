//! Parity sdk-complete-38: headless tests for background activity criteria, searchable entities and shortcut phrases.
//! Time is passed in as seconds, and nothing here touches an index, a scheduler or a device.

use bifrost_app::activity_schedule::{ActivityCriteria, ActivitySchedule, Decision};
use bifrost_app::intents::Intent;
use bifrost_app::quicksearch::{CSSearchableItemAttributeSet, IndexError};
use bifrost_app::searchable_entity::{searchable_items, SearchableEntity};

#[test]
fn headless_criteria_need_a_positive_interval_and_a_tolerance_within_it() {
    assert!(ActivityCriteria::new(0, 0).is_none());
    assert!(ActivityCriteria::new(60, 61).is_none());
    let c = ActivityCriteria::new(60, 10).expect("a valid interval and tolerance");
    assert_eq!((c.interval_secs(), c.tolerance_secs(), c.repeats()), (60, 10, false));
    assert!(c.repeating().repeats());
}

#[test]
fn headless_a_new_activity_is_due_at_once_with_its_tolerance_as_the_deadline() {
    let s = ActivitySchedule::new(ActivityCriteria::new(600, 30).unwrap());
    assert_eq!(s.last_run(), None);
    assert_eq!(s.decide(100), Decision::Run { deadline: 130 });
}

#[test]
fn headless_a_one_shot_activity_runs_once_and_then_finishes() {
    let mut s = ActivitySchedule::new(ActivityCriteria::new(60, 0).unwrap());
    let mut runs = 0;
    assert!(s.run_if_due(100, || runs += 1));
    assert!(!s.run_if_due(500, || runs += 1));
    assert_eq!(s.decide(500), Decision::Finished);
    assert_eq!(runs, 1);
    assert_eq!(s.last_run(), Some(100));
}

#[test]
fn headless_a_repeating_activity_waits_for_its_interval_then_runs_within_its_tolerance() {
    let mut s = ActivitySchedule::new(ActivityCriteria::new(60, 10).unwrap().repeating());
    assert!(s.run_if_due(100, || {}));
    assert_eq!(s.decide(130), Decision::Wait { until: 160 });
    assert_eq!(s.decide(159), Decision::Wait { until: 160 });
    assert_eq!(s.decide(165), Decision::Run { deadline: 170 });
}

#[test]
fn headless_a_late_run_reports_a_deadline_that_is_not_in_the_past() {
    let mut s = ActivitySchedule::new(ActivityCriteria::new(60, 10).unwrap().repeating());
    assert!(s.run_if_due(100, || {}));
    assert_eq!(s.decide(200), Decision::Run { deadline: 200 });
}

#[test]
fn headless_work_does_not_run_while_a_repeating_activity_waits() {
    let mut s = ActivitySchedule::new(ActivityCriteria::new(60, 0).unwrap().repeating());
    let mut runs = 0;
    assert!(s.run_if_due(0, || runs += 1));
    assert!(!s.run_if_due(59, || runs += 1));
    assert!(s.run_if_due(60, || runs += 1));
    assert_eq!(runs, 2);
    assert_eq!(s.last_run(), Some(60));
}

struct Note {
    id: &'static str,
    title: &'static str,
    expires: Option<i64>,
}

impl SearchableEntity for Note {
    fn entity_identifier(&self) -> String {
        if self.id.is_empty() { String::new() } else { format!("note.{}", self.id) }
    }

    fn domain_identifier(&self) -> String {
        "notes".into()
    }

    fn attribute_set(&self) -> CSSearchableItemAttributeSet {
        CSSearchableItemAttributeSet::new(self.title).keywords(&["draft"])
    }

    fn expires_at(&self) -> Option<i64> {
        self.expires
    }
}

#[test]
fn headless_an_entity_becomes_a_searchable_item_with_its_identifiers_and_title() {
    let item = Note { id: "n1", title: "Milk", expires: None }.searchable_item();
    assert_eq!(item.unique_identifier, "note.n1");
    assert_eq!(item.domain_identifier, "notes");
    assert_eq!(item.attribute_set.title, "Milk");
    assert_eq!(item.attribute_set.keywords, vec!["draft".to_string()]);
    assert_eq!(item.expiration_date, None);
}

#[test]
fn headless_an_entity_with_an_expiry_keeps_it_on_the_item() {
    let item = Note { id: "n2", title: "Bread", expires: Some(1_700_000_000) }.searchable_item();
    assert_eq!(item.expiration_date, Some(1_700_000_000));
    assert!(item.is_expired(1_700_000_000));
    assert!(!item.is_expired(1_699_999_999));
}

#[test]
fn headless_a_batch_of_entities_keeps_its_order() {
    let notes = [Note { id: "a", title: "A", expires: None }, Note { id: "b", title: "B", expires: None }];
    let ids: Vec<String> = searchable_items(&notes).into_iter().map(|i| i.unique_identifier).collect();
    assert_eq!(ids, vec!["note.a".to_string(), "note.b".to_string()]);
}

#[test]
fn headless_an_entity_without_an_identifier_fails_the_index_check() {
    let item = Note { id: "", title: "Untitled", expires: None }.searchable_item();
    assert!(matches!(IndexError::check(&item), Err(IndexError::EmptyIdentifier)));
    let valid = Note { id: "ok", title: "Fine", expires: None }.searchable_item();
    assert_eq!(IndexError::check(&valid), Ok(()));
}

#[test]
fn headless_shortcut_phrases_are_trimmed_and_unique() {
    let intent = Intent::new("start", "Start", "").shortcut().phrase("  Start timer ").phrase("").phrase("Start timer");
    assert_eq!(intent.phrases, vec!["Start timer".to_string()]);
}

#[test]
fn headless_shortcut_phrases_are_written_to_the_manifest_and_the_intent_json() {
    let intent = Intent::new("start", "Start", "").shortcut().phrase("Start timer").phrase("Begin");
    let toml = intent.manifest_toml();
    assert!(toml.contains("phrases = [\"Start timer\", \"Begin\"]"), "{toml}");
    assert!(intent.to_json().get("phrases").is_some());
}

#[test]
fn headless_an_intent_without_phrases_keeps_the_manifest_it_always_had() {
    let toml = Intent::new("stop", "Stop", "").shortcut().manifest_toml();
    assert!(!toml.contains("phrases"), "{toml}");
}

#[test]
fn headless_quotes_in_a_phrase_are_written_as_single_quotes() {
    let toml = Intent::new("go", "Go", "").shortcut().phrase("Say \"go\"").manifest_toml();
    assert!(toml.contains("phrases = [\"Say 'go'\"]"), "{toml}");
}
