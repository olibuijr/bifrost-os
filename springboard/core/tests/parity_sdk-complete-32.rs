//! Parity sdk-complete-32: headless tests for single sign-on completion (`bifrost_core::auth_callback_routes` and
//! `bifrost_core::auth_session`). Pure host tests: no window, no browser and no socket.

use bifrost_core::auth_callback_routes::{complete_session, CallbackRoutes, CompletionError, RouteError};
use bifrost_core::auth_session::{PresentationContext, SessionError, State, WebSession};

const APP: &str = "is.example.myapp";

fn routes() -> CallbackRoutes {
    let mut routes = CallbackRoutes::new();
    routes.register("myapp", APP).unwrap();
    routes.register("otherapp", APP).unwrap();
    routes
}

fn running_session(scheme: &str) -> WebSession {
    let mut session = WebSession::new("https://login.example.com/authorize", scheme);
    session.start(&PresentationContext::over(1), &[1]).unwrap();
    session
}

#[test]
fn a_redirect_routed_to_the_requesting_app_completes_its_session() {
    let mut session = running_session("myapp");
    let done = complete_session(&routes(), APP, &mut session, "myapp://cb?code=7");
    assert_eq!(done, Ok("myapp://cb?code=7".to_string()));
    assert_eq!(session.outcome(), Some(Ok("myapp://cb?code=7".to_string())));
}

#[test]
fn a_redirect_for_another_app_is_refused_and_the_session_keeps_running() {
    let mut session = running_session("myapp");
    let refused = complete_session(&routes(), "is.example.other", &mut session, "myapp://cb");
    assert_eq!(refused, Err(CompletionError::Route(RouteError::NotRequestingApp)));
    assert_eq!(session.state(), &State::Running, "the real callback can still arrive");
}

#[test]
fn a_redirect_with_a_scheme_other_than_the_session_is_refused_by_the_session() {
    let mut session = running_session("myapp");
    let refused = complete_session(&routes(), APP, &mut session, "otherapp://cb");
    assert_eq!(refused, Err(CompletionError::Session(SessionError::CallbackMismatch)));
    assert_eq!(session.state(), &State::Running);
}

#[test]
fn a_redirect_to_an_unrouted_scheme_is_refused_before_the_session_sees_it() {
    let mut session = running_session("myapp");
    let refused = complete_session(&routes(), APP, &mut session, "nobody://cb");
    assert_eq!(refused, Err(CompletionError::Route(RouteError::NoRoute)));
}

#[test]
fn the_callback_scheme_matches_in_any_letter_case() {
    let mut session = running_session("MyApp");
    assert_eq!(session.complete("myapp://cb"), Ok("myapp://cb".to_string()));
    assert_eq!(session.state(), &State::Finished("myapp://cb".to_string()));
}

#[test]
fn a_cancelled_session_refuses_a_late_redirect() {
    let mut session = running_session("myapp");
    assert!(session.cancel());
    let late = complete_session(&routes(), APP, &mut session, "myapp://cb");
    assert_eq!(late, Err(CompletionError::Session(SessionError::NotRunning)));
    assert_eq!(session.outcome(), Some(Err(SessionError::CanceledLogin)));
}

#[test]
fn a_completion_error_names_the_reason_in_words() {
    let route = CompletionError::Route(RouteError::NotRequestingApp);
    assert_eq!(route.to_string(), "the callback belongs to an app that did not start the session");
    let session = CompletionError::Session(SessionError::CallbackMismatch);
    assert_eq!(session.to_string(), "the callback URL does not use the session's scheme");
}
