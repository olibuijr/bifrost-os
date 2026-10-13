//! Shared-login authentication session, shaped like the Safari Services session API (`SFAuthenticationSession`): an
//! app gives a start URL and a callback URL scheme, starts the session, and receives exactly one completion with either
//! the callback URL the page handed back or an error. The state machine lives in [`crate::webauth_session`]; this module
//! is the Safari-shaped surface over it and the single-completion rule. Pure: the shell opens the page and reports the
//! redirect. The Apple session is deprecated in favour of the web authentication session, so this model exists for apps
//! that still call the older name.

use crate::webauth_session::{scheme_of, Callback, Failure, State, WebAuthSession};

/// Why a session could not be created or started.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// The start URL is not http or https with a host, or the callback scheme is not a valid URL scheme name.
    InvalidRequest,
    /// The session was already started.
    AlreadyStarted,
    /// The app gave no place to show the page when it started.
    PresentationContextUnavailable,
}

/// The one completion a session reports.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Completion {
    /// The page redirected to the callback scheme; the URL is passed on as received.
    Callback(String),
    /// The person closed the page before a callback arrived.
    CanceledLogin,
    /// The session could not show its page.
    PresentationContextUnavailable,
}

#[derive(Debug)]
pub struct AuthSession {
    inner: WebAuthSession,
    reported: bool,
}

impl AuthSession {
    /// A session that opens `url` and waits for a redirect to `callback_scheme` (for example `myapp`).
    /// Fails when the URL or the scheme is not valid for a session.
    pub fn new(url: &str, callback_scheme: &str) -> Result<Self, Error> {
        let scheme = scheme_of(&format!("{callback_scheme}:x")).ok_or(Error::InvalidRequest)?;
        let inner = WebAuthSession::new(url, Callback::CustomScheme(scheme));
        if !inner.can_start() {
            return Err(Error::InvalidRequest);
        }
        Ok(AuthSession { inner, reported: false })
    }

    pub fn url(&self) -> &str {
        self.inner.url()
    }

    /// The callback scheme, lowercased as the redirect matching uses it.
    pub fn callback_scheme(&self) -> &str {
        match self.inner.callback() {
            Callback::CustomScheme(s) => s.as_str(),
            Callback::Https { .. } => "",
        }
    }

    /// Whether the session uses a private browsing context that keeps no shared cookies.
    pub fn set_prefers_ephemeral(&mut self, on: bool) {
        self.inner.set_prefers_ephemeral(on);
    }

    pub fn prefers_ephemeral(&self) -> bool {
        self.inner.prefers_ephemeral()
    }

    /// Starts the session. `has_context` says whether the app gave a place to show the page. A failure leaves the
    /// session unstarted or failed, and the completion is then reported by [`AuthSession::report`].
    pub fn start(&mut self, has_context: bool) -> Result<(), Error> {
        if *self.inner.state() == State::Running {
            return Err(Error::AlreadyStarted);
        }
        if self.inner.start(has_context) {
            return Ok(());
        }
        match self.inner.state() {
            State::Failed(Failure::NoPresentationContext) => Err(Error::PresentationContextUnavailable),
            _ => Err(Error::InvalidRequest),
        }
    }

    /// Cancels a running session, as the person dismissing the page would.
    pub fn cancel(&mut self) {
        self.inner.cancel();
    }

    /// Hands a redirect to the session. Returns true when the redirect is the callback this session waits for.
    pub fn deliver(&mut self, redirect: &str) -> bool {
        self.inner.complete(redirect)
    }

    /// The completion, once. Returns it the first time the session has ended and `None` afterwards, so the owner's
    /// handler runs exactly once. `None` while the session is still running or was never started.
    pub fn report(&mut self) -> Option<Completion> {
        if self.reported {
            return None;
        }
        let completion = match self.inner.state() {
            State::Finished(url) => Completion::Callback(url.clone()),
            State::Failed(Failure::Canceled) => Completion::CanceledLogin,
            State::Failed(Failure::NoPresentationContext) => Completion::PresentationContextUnavailable,
            State::Idle | State::Running => return None,
        };
        self.reported = true;
        Some(completion)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const START: &str = "https://idp.example/auth";

    #[test]
    fn rejects_a_start_url_that_is_not_web() {
        assert_eq!(AuthSession::new("ftp://idp.example/auth", "myapp").unwrap_err(), Error::InvalidRequest);
        assert_eq!(AuthSession::new("not a url", "myapp").unwrap_err(), Error::InvalidRequest);
    }

    #[test]
    fn rejects_an_invalid_callback_scheme() {
        assert_eq!(AuthSession::new(START, "").unwrap_err(), Error::InvalidRequest);
        assert_eq!(AuthSession::new(START, "1bad").unwrap_err(), Error::InvalidRequest);
    }

    #[test]
    fn callback_scheme_is_stored_lowercased() {
        let s = AuthSession::new(START, "MyApp").unwrap();
        assert_eq!(s.callback_scheme(), "myapp");
        assert_eq!(s.url(), START);
    }

    #[test]
    fn start_twice_is_refused() {
        let mut s = AuthSession::new(START, "myapp").unwrap();
        assert_eq!(s.start(true), Ok(()));
        assert_eq!(s.start(true), Err(Error::AlreadyStarted));
    }

    #[test]
    fn start_without_a_context_reports_presentation_failure() {
        let mut s = AuthSession::new(START, "myapp").unwrap();
        assert_eq!(s.start(false), Err(Error::PresentationContextUnavailable));
        assert_eq!(s.report(), Some(Completion::PresentationContextUnavailable));
    }

    #[test]
    fn callback_completes_once_with_the_redirect() {
        let mut s = AuthSession::new(START, "myapp").unwrap();
        assert_eq!(s.report(), None);
        s.start(true).unwrap();
        assert_eq!(s.report(), None);
        assert!(!s.deliver("otherapp://done?code=1"));
        assert!(s.deliver("MYAPP://done?code=1"));
        assert_eq!(s.report(), Some(Completion::Callback("MYAPP://done?code=1".to_string())));
        assert_eq!(s.report(), None);
    }

    #[test]
    fn cancel_reports_canceled_login_once() {
        let mut s = AuthSession::new(START, "myapp").unwrap();
        s.start(true).unwrap();
        s.cancel();
        assert_eq!(s.report(), Some(Completion::CanceledLogin));
        assert_eq!(s.report(), None);
        assert!(!s.deliver("myapp://late"));
    }

    #[test]
    fn ephemeral_flag_round_trips() {
        let mut s = AuthSession::new(START, "myapp").unwrap();
        assert!(!s.prefers_ephemeral());
        s.set_prefers_ephemeral(true);
        assert!(s.prefers_ephemeral());
    }
}
