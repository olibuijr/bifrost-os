//! Parity sdk-rest-03: headless tests for the photo project model (sections of assets, applied as change lists).
//! Nothing here opens a photo library, a display or an extension host.

use bifrost_ui::photo_project::{Change, Error, Project, MAX_SECTIONS, MAX_TEXT};

fn project() -> Project {
    Project::new("book-1", "Summer book").unwrap()
}

fn add_section(id: &str, title: &str) -> Change {
    Change::AddSection { id: id.into(), title: title.into() }
}

fn with_two_sections() -> Project {
    let mut p = project();
    p.apply(&[add_section("a", "Arrival"), add_section("b", "Departure")]).unwrap();
    p
}

#[test]
fn a_new_project_has_an_identifier_a_title_and_no_sections() {
    let p = project();
    assert_eq!(p.id(), "book-1");
    assert_eq!(p.title(), "Summer book");
    assert!(p.sections().is_empty());
}

#[test]
fn a_project_trims_its_identifier_and_title() {
    let p = Project::new("  book-2 ", " Winter ").unwrap();
    assert_eq!(p.id(), "book-2");
    assert_eq!(p.title(), "Winter");
}

#[test]
fn an_empty_or_overlong_title_is_rejected_with_the_offending_text() {
    assert_eq!(Project::new("x", "   ").unwrap_err(), Error::InvalidText("   ".into()));
    let long = "t".repeat(MAX_TEXT + 1);
    assert_eq!(Project::new("x", &long).unwrap_err(), Error::InvalidText(long));
}

#[test]
fn sections_keep_the_order_they_were_added_in() {
    let p = with_two_sections();
    let ids: Vec<&str> = p.sections().iter().map(|s| s.id()).collect();
    assert_eq!(ids, ["a", "b"]);
}

#[test]
fn adding_an_asset_puts_it_in_the_section_in_order() {
    let mut p = with_two_sections();
    p.apply(&[Change::AddAssets { section: "a".into(), assets: vec!["img-2".into(), "img-1".into()] }]).unwrap();
    assert_eq!(p.section("a").unwrap().assets(), ["img-2", "img-1"]);
    assert!(p.section("b").unwrap().assets().is_empty());
}

#[test]
fn an_asset_already_in_the_section_is_a_duplicate_error() {
    let mut p = with_two_sections();
    p.apply(&[Change::AddAssets { section: "a".into(), assets: vec!["img-1".into()] }]).unwrap();
    let err = p.apply(&[Change::AddAssets { section: "a".into(), assets: vec!["img-1".into()] }]).unwrap_err();
    assert_eq!(err, Error::DuplicateAsset { section: "a".into(), asset: "img-1".into() });
}

#[test]
fn one_asset_may_sit_in_two_sections() {
    let mut p = with_two_sections();
    p.apply(&[
        Change::AddAssets { section: "a".into(), assets: vec!["img-1".into()] },
        Change::AddAssets { section: "b".into(), assets: vec!["img-1".into()] },
    ])
    .unwrap();
    assert!(p.section("a").unwrap().contains("img-1"));
    assert!(p.section("b").unwrap().contains("img-1"));
}

#[test]
fn removing_an_asset_the_section_does_not_hold_is_an_error() {
    let mut p = with_two_sections();
    let err = p.apply(&[Change::RemoveAssets { section: "b".into(), assets: vec!["img-9".into()] }]).unwrap_err();
    assert_eq!(err, Error::UnknownAsset { section: "b".into(), asset: "img-9".into() });
}

#[test]
fn a_change_list_that_fails_part_way_leaves_the_project_unchanged() {
    let mut p = with_two_sections();
    let before = p.clone();
    let result = p.apply(&[
        Change::RenameSection { id: "a".into(), title: "Renamed".into() },
        Change::RemoveSection { id: "missing".into() },
    ]);
    assert_eq!(result, Err(Error::UnknownSection("missing".into())));
    assert_eq!(p, before, "the rename before the failing change is not kept");
}

#[test]
fn a_duplicate_section_identifier_is_rejected() {
    let mut p = with_two_sections();
    let err = p.apply(&[add_section("a", "Again")]).unwrap_err();
    assert_eq!(err, Error::DuplicateSection("a".into()));
}

#[test]
fn renaming_a_section_changes_only_its_title() {
    let mut p = with_two_sections();
    p.apply(&[Change::RenameSection { id: "b".into(), title: "Return".into() }]).unwrap();
    assert_eq!(p.section("b").unwrap().title(), "Return");
    assert_eq!(p.section("a").unwrap().title(), "Arrival");
}

#[test]
fn removing_a_section_drops_its_assets_and_keeps_the_others() {
    let mut p = with_two_sections();
    p.apply(&[
        Change::AddAssets { section: "a".into(), assets: vec!["img-1".into()] },
        Change::RemoveSection { id: "a".into() },
    ])
    .unwrap();
    assert!(p.section("a").is_none());
    assert_eq!(p.sections().len(), 1);
}

#[test]
fn moving_a_section_places_it_at_the_given_position() {
    let mut p = with_two_sections();
    p.apply(&[add_section("c", "Stay")]).unwrap();
    p.apply(&[Change::MoveSection { id: "c".into(), to: 0 }]).unwrap();
    let ids: Vec<&str> = p.sections().iter().map(|s| s.id()).collect();
    assert_eq!(ids, ["c", "a", "b"]);
}

#[test]
fn moving_a_section_past_the_last_position_is_an_error() {
    let mut p = with_two_sections();
    let err = p.apply(&[Change::MoveSection { id: "a".into(), to: 2 }]).unwrap_err();
    assert_eq!(err, Error::PositionOutOfRange { position: 2, count: 2 });
}

#[test]
fn a_project_holds_at_most_the_section_limit() {
    let mut p = project();
    let changes: Vec<Change> = (0..MAX_SECTIONS).map(|i| add_section(&format!("s{i}"), "Part")).collect();
    p.apply(&changes).unwrap();
    assert_eq!(p.apply(&[add_section("one-more", "Part")]), Err(Error::TooManySections));
    assert_eq!(p.sections().len(), MAX_SECTIONS);
}

#[test]
fn errors_name_the_section_or_asset_they_refer_to() {
    let mut p = with_two_sections();
    let err = p.apply(&[Change::AddAssets { section: "zz".into(), assets: vec!["img-1".into()] }]).unwrap_err();
    assert_eq!(err.to_string(), "no section named zz");
}
