//! Parity sdk-complete-35: headless tests for the security result codes (`bifrost_app::security_status`) and the
//! transport policy for plain http (`bifrost_app::transport_policy`). Pure functions only: no network, no keychain
//! service, no clock.

use bifrost_app::keychain::Error;
use bifrost_app::security_status::{category, is_success, meaning, Category, KNOWN_CODES};
use bifrost_app::transport_policy::{PolicyError, TransportPolicy};

// --- security_status: result codes -----------------------------------------------------------------------------------

#[test]
fn keychain_errors_report_codes_that_the_table_describes() {
    let cases = [
        (Error::DuplicateItem, -25299, Category::Duplicate, "the item already exists"),
        (Error::ItemNotFound, -25300, Category::Missing, "the item was not found"),
        (Error::NotPermitted, -25291, Category::Access, "the service is not available"),
        (Error::Param, -50, Category::Parameter, "invalid parameter"),
        (Error::Io(String::new()), -36, Category::Storage, "I/O error"),
    ];
    for (error, code, expected, text) in cases {
        assert_eq!(error.code(), code);
        assert_eq!(category(code), expected, "category of {code}");
        assert_eq!(meaning(code), Some(text));
    }
}

#[test]
fn a_refused_interaction_and_a_missing_entitlement_are_access_errors() {
    assert_eq!(category(-25308), Category::Access);
    assert_eq!(category(-25315), Category::Access);
    assert_eq!(category(-25293), Category::Access);
    assert_eq!(category(-34018), Category::Access);
    assert_eq!(meaning(-34018), Some("the app lacks the required entitlement"));
}

#[test]
fn a_missing_item_attribute_or_keychain_is_a_missing_category() {
    assert_eq!(category(-25303), Category::Missing);
    assert_eq!(category(-25307), Category::Missing);
    assert_eq!(category(-25294), Category::Missing);
}

#[test]
fn only_zero_is_success() {
    assert!(is_success(0));
    assert!(!is_success(-25300));
    assert!(!is_success(-1));
    assert_eq!(category(0), Category::Success);
    assert_eq!(meaning(0), Some("success"));
}

#[test]
fn a_code_the_platform_does_not_report_has_no_meaning() {
    assert_eq!(meaning(-1), None);
    assert_eq!(category(-1), Category::Unknown);
    assert_eq!(category(12345), Category::Unknown);
}

#[test]
fn every_listed_code_appears_once_with_a_text_and_a_category_that_round_trips() {
    for (i, (code, text, cat)) in KNOWN_CODES.iter().enumerate() {
        assert!(!text.is_empty());
        assert_eq!(category(*code), *cat);
        assert_eq!(meaning(*code), Some(*text));
        assert!(
            KNOWN_CODES[i + 1..].iter().all(|(other, _, _)| other != code),
            "code {code} is listed twice"
        );
    }
}

// --- transport_policy: plain http and redirects ----------------------------------------------------------------------

#[test]
fn the_default_policy_sends_https_and_refuses_plain_http() {
    let policy = TransportPolicy::secure_only();
    assert_eq!(policy.check_request("https://bifrost.example/ota/latest.json"), Ok(()));
    assert_eq!(
        policy.check_request("http://bifrost.example/ota/latest.json"),
        Err(PolicyError::InsecureRequest("http://bifrost.example/ota/latest.json".to_string()))
    );
}

#[test]
fn an_excepted_host_may_use_plain_http_and_no_other_host_may() {
    let policy = TransportPolicy::secure_only().allow_insecure_host("intranet.example", false);
    assert_eq!(policy.check_request("http://intranet.example:8080/status"), Ok(()));
    assert!(policy.check_request("http://other.intranet.example/status").is_err());
    assert!(policy.check_request("http://intranet.example.evil/status").is_err());
}

#[test]
fn an_exception_with_subdomains_covers_hosts_under_it_only() {
    let policy = TransportPolicy::secure_only().allow_insecure_host("intranet.example", true);
    assert_eq!(policy.check_request("http://a.b.intranet.example/x"), Ok(()));
    assert_eq!(policy.check_request("http://intranet.example/x"), Ok(()));
    assert!(policy.check_request("http://intranet.example.evil/x").is_err());
    assert!(policy.check_request("http://notintranet.example/x").is_err());
}

#[test]
fn host_names_match_without_regard_to_case() {
    let policy = TransportPolicy::secure_only().allow_insecure_host("Intranet.Example", false);
    assert_eq!(policy.check_request("http://INTRANET.example/x"), Ok(()));
}

#[test]
fn a_malformed_address_is_a_bad_url_under_every_policy() {
    let policy = TransportPolicy::secure_only().allow_insecure_host("bifrost.example", true);
    assert_eq!(policy.check_request("bifrost.example/x"), Err(PolicyError::BadURL("bifrost.example/x".to_string())));
    assert_eq!(policy.check_request("ftp://bifrost.example/x"), Err(PolicyError::BadURL("ftp://bifrost.example/x".to_string())));
}

#[test]
fn a_redirect_from_https_to_plain_http_is_refused_unless_the_host_is_excepted() {
    let policy = TransportPolicy::secure_only();
    assert_eq!(
        policy.check_redirect("https://bifrost.example/a", "http://bifrost.example/b"),
        Err(PolicyError::RedirectDowngrade {
            from: "https://bifrost.example/a".to_string(),
            to: "http://bifrost.example/b".to_string(),
        })
    );
    let excepted = policy.allow_insecure_host("legacy.example", false);
    assert_eq!(excepted.check_redirect("https://bifrost.example/a", "http://legacy.example/b"), Ok(()));
    assert!(excepted.check_redirect("https://bifrost.example/a", "http://other.example/b").is_err());
}

#[test]
fn a_redirect_that_keeps_https_or_goes_up_to_https_passes() {
    let policy = TransportPolicy::secure_only();
    assert_eq!(policy.check_redirect("https://bifrost.example/a", "https://cdn.example/b"), Ok(()));
    assert_eq!(policy.check_redirect("http://bifrost.example/a", "https://bifrost.example/b"), Ok(()));
    assert_eq!(policy.check_redirect("http://bifrost.example/a", "http://bifrost.example/b"), Ok(()));
}

#[test]
fn a_redirect_to_a_malformed_address_is_a_bad_url() {
    let policy = TransportPolicy::secure_only();
    assert_eq!(
        policy.check_redirect("https://bifrost.example/a", "not a url"),
        Err(PolicyError::BadURL("not a url".to_string()))
    );
}

#[test]
fn policy_errors_describe_the_refused_address() {
    let text = PolicyError::InsecureRequest("http://a.example/".to_string()).to_string();
    assert_eq!(text, "Plain http is not allowed for http://a.example/");
}
