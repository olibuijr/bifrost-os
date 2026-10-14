//! Parity sdk-complete-12: headless tests of the universal link association model (`bifrost_core::app_links`).
//! Every check is pure: association text in, resolved app names out. Nothing is fetched and no shell runs.

use bifrost_core::app_links::AppLinks;

const ASSOCIATION: &str = r#"{
  "applinks": {
    "apps": [],
    "details": [
      {"appID": "TEAM1.com.example.notes", "paths": ["/notes/*", "/n"]},
      {"appIDs": ["TEAM1.com.example.shop", "TEAM1.com.example.shop-beta"], "paths": ["/shop*"]},
      {"appID": "TEAM1.com.example.web"}
    ]
  }
}"#;

#[test]
fn an_association_claims_the_paths_of_the_app_that_names_its_id() {
    let mut links = AppLinks::new();
    links.claim_from_association("example.com", "notes", "TEAM1.com.example.notes", ASSOCIATION).unwrap();
    assert_eq!(links.resolve("https://example.com/notes/1"), Some("notes"));
    assert_eq!(links.resolve("https://example.com/n"), Some("notes"));
    assert_eq!(links.resolve("https://example.com/shop"), None, "a path of another app is not claimed");
}

#[test]
fn a_wildcard_suffix_is_a_prefix_and_an_app_id_list_is_searched() {
    let mut links = AppLinks::new();
    links.claim_from_association("example.com", "shop", "TEAM1.com.example.shop-beta", ASSOCIATION).unwrap();
    assert_eq!(links.resolve("https://example.com/shop/cart"), Some("shop"));
    assert_eq!(links.resolve("https://example.com/shopping/cart"), None, "a wildcard prefix still ends at a segment");
    assert_eq!(links.resolve("https://example.com/notes/1"), None);
}

#[test]
fn a_detail_without_paths_claims_the_whole_site() {
    let mut links = AppLinks::new();
    links.claim_from_association("example.com", "web", "TEAM1.com.example.web", ASSOCIATION).unwrap();
    assert_eq!(links.resolve("https://example.com/anything/at/all"), Some("web"));
}

#[test]
fn an_app_id_missing_from_the_association_claims_nothing_and_reports_it() {
    let mut links = AppLinks::new();
    let result = links.claim_from_association("example.com", "other", "TEAM1.com.example.other", ASSOCIATION);
    assert!(result.is_err(), "an app id with no detail is an error, not a silent empty claim");
    assert_eq!(links.resolve("https://example.com/notes/1"), None);
}

#[test]
fn an_exclusion_path_is_refused_because_the_model_cannot_hold_it() {
    let text = r#"{"applinks":{"details":[{"appID":"T.app","paths":["/a/*","NOT /a/private/*"]}]}}"#;
    let mut links = AppLinks::new();
    let result = links.claim_from_association("example.com", "app", "T.app", text);
    assert!(result.unwrap_err().contains("NOT"), "the error names the unsupported exclusion");
    assert_eq!(links.resolve("https://example.com/a/1"), None, "a refused association claims nothing");
}

#[test]
fn malformed_association_text_is_an_error_naming_the_missing_part() {
    let mut links = AppLinks::new();
    assert!(links.claim_from_association("example.com", "x", "T.x", "not json").is_err());
    let no_applinks = r#"{"webcredentials":{}}"#;
    let err = links.claim_from_association("example.com", "x", "T.x", no_applinks).unwrap_err();
    assert!(err.contains("applinks"), "the error names the missing applinks object");
}

#[test]
fn a_reclaim_from_association_replaces_the_earlier_claim_of_the_app() {
    let mut links = AppLinks::new();
    links.claim_from_association("example.com", "notes", "TEAM1.com.example.notes", ASSOCIATION).unwrap();
    let refused = r#"{"applinks":{"details":[{"appID":"TEAM1.com.example.notes","paths":["NOT /notes/x"]}]}}"#;
    assert!(links.claim_from_association("example.com", "notes", "TEAM1.com.example.notes", refused).is_err());
    assert_eq!(links.resolve("https://example.com/notes/1"), Some("notes"), "a refused reclaim keeps the earlier claim");
    links.claim_from_association("example.com", "notes", "TEAM1.com.example.notes", r#"{"applinks":{"details":[{"appID":"T.n","paths":["/only/*"]}]}}"#).unwrap_err();
    assert_eq!(links.resolve("https://example.com/notes/1"), Some("notes"));
}
