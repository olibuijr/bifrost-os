//! Sign-in for apps: the web sign-in session, the authorization request controller, the one-time code finder and the
//! credential identity store. Each one is the pure model that the shell and the SDK share; this module is the
//! app-facing path to them.
//!
//! * [`WebSession`] runs a sign-in page in its own session and finishes when a redirect to the app's callback scheme
//!   arrives. [`PresentationContext`] names the open window it is shown over.
//! * [`AuthorizationController`] holds the password or passkey requests a sign-in provider made, runs them once and
//!   ends with one [`AuthorizationResult`], a failure or a cancel.
//! * [`find_code`] finds a verification code in the text of a message, for a field that offers it for autofill.
//! * [`IdentityStore`] keeps the credential identities an app offers to the system for autofill.
//!
//! The web session and the controller do not open a sheet or call a provider themselves. The shell or the caller
//! reports the outcome. Nothing here reaches the network.
//!
//! ```ignore
//! use bifrost_app::sign_in::{PresentationContext, WebSession};
//! let mut session = WebSession::new("https://id.example.org/login", "example-app");
//! session.start(&PresentationContext::over(window_id), &open_windows)?;
//! // when the shell delivers the redirect:
//! let url = session.complete("example-app://callback?code=abc")?;
//! ```

pub use bifrost_core::auth_session::{scheme_of, ControlSize, PresentationContext, SessionError, State, WebSession};
pub use bifrost_ui::authorization_request::{
    AuthorizationController, AuthorizationResult, ControllerError, Phase, RequestKind,
};
pub use bifrost_ui::credential_identity::{
    CredentialIdentity, IdentifierType, IdentityKind, IdentityStore, IdentityTypes, ServiceIdentifier, StoreError,
};
pub use bifrost_ui::one_time_code::{find_code, FoundCode};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn web_session_finishes_with_its_callback() {
        let windows = [7u32];
        let mut session = WebSession::new("https://id.example.org/login", "example-app");
        assert!(session.can_start(&PresentationContext::over(7), &windows));
        session.start(&PresentationContext::over(7), &windows).unwrap();
        assert_eq!(session.state(), &State::Running);
        let url = session.complete("example-app://callback?code=abc").unwrap();
        assert_eq!(url, "example-app://callback?code=abc");
        assert_eq!(session.outcome(), Some(Ok(url)));
    }

    #[test]
    fn web_session_rejects_a_foreign_callback_and_a_missing_window() {
        let mut session = WebSession::new("https://id.example.org/login", "example-app");
        let none = PresentationContext { anchor: None };
        assert_eq!(session.start(&none, &[7]), Err(SessionError::PresentationContextNotProvided));
        assert_eq!(session.start(&PresentationContext::over(9), &[7]), Err(SessionError::PresentationContextInvalid));
        session.start(&PresentationContext::over(7), &[7]).unwrap();
        assert_eq!(session.complete("other-app://callback"), Err(SessionError::CallbackMismatch));
        assert!(session.cancel());
        assert_eq!(session.outcome(), Some(Err(SessionError::CanceledLogin)));
    }

    #[test]
    fn controller_completes_with_a_requested_kind_only() {
        let mut controller = AuthorizationController::new(&[RequestKind::Password, RequestKind::Password]).unwrap();
        assert_eq!(controller.requests(), &[RequestKind::Password]);
        controller.perform().unwrap();
        let answer = AuthorizationResult::Passkey { credential_id: vec![1], user_id: vec![2] };
        assert_eq!(controller.complete(answer), Err(ControllerError::ResultNotRequested(RequestKind::Passkey)));
        let password = AuthorizationResult::Password { user: "alice".into(), password: "hunter2".into() };
        controller.complete(password).unwrap();
        assert!(matches!(controller.phase(), Phase::Completed(_)));
    }

    #[test]
    fn controller_without_requests_is_refused() {
        assert_eq!(AuthorizationController::new(&[]).err(), Some(ControllerError::NoRequests));
    }

    #[test]
    fn password_result_debug_hides_the_password() {
        let answer = AuthorizationResult::Password { user: "alice".into(), password: "hunter2".into() };
        assert!(!format!("{answer:?}").contains("hunter2"));
    }

    #[test]
    fn finds_a_code_next_to_its_keyword() {
        let text = "Your verification code is 482913. Do not share it.";
        let found = find_code(text).unwrap();
        assert_eq!(found.code, "482913");
        assert_eq!(&text[found.start..found.end], "482913");
        assert!(find_code("Order 482913 shipped").is_none());
    }

    #[test]
    fn scheme_is_read_from_a_redirect() {
        assert_eq!(scheme_of("example-app://callback"), Some("example-app"));
        assert_eq!(scheme_of("no scheme here"), None);
    }
}
