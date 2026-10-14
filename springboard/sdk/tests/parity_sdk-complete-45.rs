//! Parity sdk-complete-45: the App Intents catalog. The declared intents are checked as one set, and the lookups the
//! shell and the control socket use are tested without a running app.

use bifrost_app::intent_catalog::{Catalog, CatalogError};
use bifrost_app::intents::{Intent, Param, ParamType};

fn note_intent(id: &str) -> Intent {
    Intent::new(id, "Add Note", "Adds a note").param(Param::new("text", "Text", ParamType::String))
}

#[test]
fn a_catalog_keeps_the_declared_order_and_finds_intents_by_identifier() {
    let catalog = Catalog::new(vec![note_intent("add-note"), Intent::new("open-list", "Open List", "")]).unwrap();
    assert_eq!(catalog.len(), 2);
    assert!(!catalog.is_empty());
    assert_eq!(catalog.ids(), ["add-note", "open-list"]);
    assert_eq!(catalog.get("open-list").map(|i| i.title.as_str()), Some("Open List"));
    assert!(catalog.get("Add-Note").is_none(), "identifiers match exactly");
    assert!(catalog.get("missing").is_none());
}

#[test]
fn an_empty_list_is_a_valid_empty_catalog() {
    let catalog = Catalog::new(Vec::new()).unwrap();
    assert!(catalog.is_empty());
    assert_eq!(catalog.len(), 0);
    assert!(catalog.shortcuts().is_empty());
}

#[test]
fn an_identifier_must_be_a_method_token_of_lowercase_letters_digits_hyphens_and_underscores() {
    assert!(Catalog::new(vec![Intent::new("note_2-add", "Add", "")]).is_ok());
    assert_eq!(Catalog::new(vec![Intent::new("", "Add", "")]), Err(CatalogError::BadId(String::new())));
    assert_eq!(Catalog::new(vec![Intent::new("Add", "Add", "")]), Err(CatalogError::BadId("Add".into())));
    assert_eq!(Catalog::new(vec![Intent::new("add note", "Add", "")]), Err(CatalogError::BadId("add note".into())));
    assert_eq!(Catalog::new(vec![Intent::new("add.note", "Add", "")]), Err(CatalogError::BadId("add.note".into())));
}

#[test]
fn two_intents_with_one_identifier_are_refused() {
    let err = Catalog::new(vec![note_intent("add-note"), Intent::new("add-note", "Other", "")]).unwrap_err();
    assert_eq!(err, CatalogError::Duplicate("add-note".into()));
}

#[test]
fn an_intent_needs_a_title() {
    let err = Catalog::new(vec![Intent::new("ping", "   ", "")]).unwrap_err();
    assert_eq!(err, CatalogError::NoTitle("ping".into()));
}

#[test]
fn a_shortcut_with_a_required_parameter_is_refused_and_an_optional_one_is_allowed() {
    let required = Intent::new("add-note", "Add Note", "").param(Param::new("text", "Text", ParamType::String)).shortcut();
    assert_eq!(Catalog::new(vec![required]), Err(CatalogError::ShortcutNeedsParameter("add-note".into())));
    let optional = Intent::new("open-list", "Open List", "").param(Param::new("filter", "Filter", ParamType::String).optional()).shortcut();
    assert!(Catalog::new(vec![optional]).is_ok());
}

#[test]
fn shortcuts_are_the_intents_search_lists_in_declaration_order() {
    let catalog = Catalog::new(vec![
        Intent::new("ping", "Ping", ""),
        Intent::new("open-list", "Open List", "").shortcut(),
        Intent::new("show-totals", "Totals", "").shortcut(),
    ])
    .unwrap();
    let ids: Vec<&str> = catalog.shortcuts().iter().map(|i| i.id.as_str()).collect();
    assert_eq!(ids, ["open-list", "show-totals"]);
}

#[test]
fn the_first_problem_in_the_list_is_the_one_reported() {
    let err = Catalog::new(vec![Intent::new("Bad", "Bad", ""), Intent::new("add-note", "", "")]).unwrap_err();
    assert_eq!(err, CatalogError::BadId("Bad".into()));
}
