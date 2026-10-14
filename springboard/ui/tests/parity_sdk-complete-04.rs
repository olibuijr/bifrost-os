//! Parity sdk-complete-04: the web sign-in session model, headless. A session is driven through its states here; no
//! page is opened and no browser profile is read, so the shared-login part that needs the browser stays untested.

use bifrost_ui::web_auth::{AuthSession, Completion, Error};
use bifrost_ui::webauth_session::{host_and_path, scheme_of, Callback, Failure, SessionRouter, State, WebAuthSession};

const START: &str = "https://idp.example/auth";

fn custom(scheme: &str) -> Callback {
    Callback::CustomScheme(scheme.to_string())
}

#[test]
fn a_session_needs_a_web_start_url_and_a_usable_callback_scheme() {
    assert_eq!(AuthSession::new("ftp://idp.example/auth", "myapp").unwrap_err(), Error::InvalidRequest);
    assert_eq!(AuthSession::new("javascript:alert(1)", "myapp").unwrap_err(), Error::InvalidRequest);
    assert_eq!(AuthSession::new(START, "1bad").unwrap_err(), Error::InvalidRequest);
    assert!(AuthSession::new(START, "myapp").is_ok());
}

#[test]
fn the_callback_scheme_is_stored_lowercased() {
    let s = AuthSession::new(START, "MyApp").unwrap();
    assert_eq!(s.callback_scheme(), "myapp");
    assert_eq!(s.url(), START);
    assert!(!s.prefers_ephemeral(), "a session shares cookies unless the app asks for a private one");
}

#[test]
fn the_ephemeral_preference_round_trips() {
    let mut s = AuthSession::new(START, "myapp").unwrap();
    s.set_prefers_ephemeral(true);
    assert!(s.prefers_ephemeral());
    s.set_prefers_ephemeral(false);
    assert!(!s.prefers_ephemeral());
}

#[test]
fn a_started_session_refuses_a_second_start() {
    let mut s = AuthSession::new(START, "myapp").unwrap();
    assert_eq!(s.start(true), Ok(()));
    assert_eq!(s.start(true), Err(Error::AlreadyStarted));
    assert_eq!(s.report(), None, "a running session has no completion yet");
}

#[test]
fn a_session_without_a_presentation_context_reports_that_once() {
    let mut s = AuthSession::new(START, "myapp").unwrap();
    assert_eq!(s.start(false), Err(Error::PresentationContextUnavailable));
    assert_eq!(s.report(), Some(Completion::PresentationContextUnavailable));
    assert_eq!(s.report(), None, "the owner's handler runs once");
}

#[test]
fn the_first_matching_redirect_completes_the_session_with_its_url() {
    let mut s = AuthSession::new(START, "myapp").unwrap();
    s.start(true).unwrap();
    assert!(!s.deliver("otherapp://done"), "another scheme is not this session's callback");
    assert_eq!(s.report(), None);
    assert!(s.deliver("myapp://done?code=42"));
    assert_eq!(s.report(), Some(Completion::Callback("myapp://done?code=42".into())));
    assert_eq!(s.report(), None);
    assert!(!s.deliver("myapp://again"), "a finished session takes no second redirect");
}

#[test]
fn cancelling_a_running_session_reports_a_canceled_login() {
    let mut s = AuthSession::new(START, "myapp").unwrap();
    s.start(true).unwrap();
    s.cancel();
    assert!(!s.deliver("myapp://late"), "a redirect after cancel is ignored");
    assert_eq!(s.report(), Some(Completion::CanceledLogin));
    assert_eq!(s.report(), None);
}

#[test]
fn cancelling_before_the_start_changes_nothing() {
    let mut s = AuthSession::new(START, "myapp").unwrap();
    s.cancel();
    assert_eq!(s.report(), None, "an idle session has no completion");
    assert_eq!(s.start(true), Ok(()), "the session can still start");
}

#[test]
fn a_redirect_sets_the_finished_state_and_keeps_its_url() {
    let mut s = WebAuthSession::new(START, custom("myapp"));
    assert_eq!(s.state(), &State::Idle);
    assert!(s.start(true));
    assert!(s.complete("MYAPP://done"), "the scheme is matched without case");
    assert_eq!(s.state(), &State::Finished("MYAPP://done".into()));
}

#[test]
fn an_https_callback_needs_its_host_and_path() {
    let callback = Callback::Https { host: "idp.example".into(), path: "/done".into() };
    let mut s = WebAuthSession::new(START, callback);
    assert!(s.start(true));
    assert!(!s.accepts("http://idp.example/done"), "the callback must be https");
    assert!(!s.accepts("https://idp.example/other"), "the path must match");
    assert!(!s.accepts("https://other.example/done"), "the host must match");
    assert!(s.accepts("https://IDP.example:443/done?x=1"), "host case, port and query do not count");
}

#[test]
fn a_start_url_with_no_host_or_a_non_web_scheme_cannot_start() {
    assert!(!WebAuthSession::new("http://:80/x", custom("myapp")).can_start());
    assert!(!WebAuthSession::new("ftp://idp.example/x", custom("myapp")).can_start());
    let mut s = WebAuthSession::new("ftp://idp.example/x", custom("myapp"));
    assert!(!s.start(true));
    assert_eq!(s.state(), &State::Idle, "a refused start leaves the session idle");
}

#[test]
fn a_cancelled_failure_is_the_canceled_kind() {
    let mut s = WebAuthSession::new(START, custom("myapp"));
    assert!(s.start(true));
    s.cancel();
    assert_eq!(s.state(), &State::Failed(Failure::Canceled));
}

#[test]
fn headers_keep_one_value_per_name_ignoring_case() {
    let mut s = WebAuthSession::new(START, custom("myapp"));
    s.set_header("Accept-Language", "is");
    s.set_header("accept-language", "en");
    s.set_header("X-Client", "bifrost");
    assert_eq!(
        s.headers(),
        &[("Accept-Language".to_string(), "en".to_string()), ("X-Client".to_string(), "bifrost".to_string())]
    );
}

#[test]
fn a_router_sends_each_redirect_to_the_oldest_running_session_that_accepts_it() {
    let mut first = WebAuthSession::new(START, custom("myapp"));
    assert!(first.start(true));
    let mut second = WebAuthSession::new(START, custom("myapp"));
    assert!(second.start(true));
    let mut router = SessionRouter::new();
    let a = router.add(first);
    let b = router.add(second);
    assert_eq!(router.running(), 2);
    assert_eq!(router.deliver("myapp://one"), Some(a));
    assert_eq!(router.running(), 1);
    assert_eq!(router.deliver("myapp://two"), Some(b));
    assert_eq!(router.deliver("myapp://three"), None, "no running session is left to take it");
    assert_eq!(router.get(a).unwrap().state(), &State::Finished("myapp://one".into()));
    assert_eq!(router.get(b).unwrap().state(), &State::Finished("myapp://two".into()));
}

#[test]
fn a_router_ignores_a_redirect_for_another_scheme() {
    let mut only = WebAuthSession::new(START, custom("myapp"));
    assert!(only.start(true));
    let mut router = SessionRouter::new();
    let id = router.add(only);
    assert_eq!(router.deliver("otherapp://done"), None);
    assert_eq!(router.running(), 1);
    assert!(router.remove(id).is_some());
    assert!(router.remove(id).is_none(), "a removed session cannot be removed twice");
    assert!(router.get(id).is_none());
}

#[test]
fn scheme_and_host_of_a_url_are_read_the_same_way_everywhere() {
    assert_eq!(scheme_of("MyApp://x"), Some("myapp".to_string()));
    assert_eq!(scheme_of("9x://y"), None);
    assert_eq!(scheme_of("no-colon"), None);
    assert_eq!(host_and_path("https://user@Host.Example:8443/a/b?q#f"), Some(("host.example".to_string(), "/a/b".to_string())));
    assert_eq!(host_and_path("https://example.com?x=1"), Some(("example.com".to_string(), "/".to_string())));
    assert_eq!(host_and_path("mailto:someone"), None);
}
