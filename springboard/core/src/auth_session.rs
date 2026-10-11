//! Authentication session model (task theme-05): the presentation context an authentication request is shown in, the
//! size of the biometric state icon, and the lifecycle of a web authentication session (start, cancel, and the
//! callback URL that completes it). Pure model: no window, no browser and no socket. The shell hands it the ids of the
//! windows that are open, and the app drives it through `start`, `cancel` and `complete`.

use std::fmt;

/// Why an authentication session could not start or did not finish.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionError {
    /// The user cancelled the login in the browser.
    CanceledLogin,
    /// The session was started without a presentation context.
    PresentationContextNotProvided,
    /// The presentation context names a window that is not open.
    PresentationContextInvalid,
    /// `complete` was called on a session that is not running.
    NotRunning,
    /// The redirect URL does not use the callback scheme of this session.
    CallbackMismatch,
}

impl fmt::Display for SessionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            SessionError::CanceledLogin => "the login was canceled",
            SessionError::PresentationContextNotProvided => "no presentation context was provided",
            SessionError::PresentationContextInvalid => "the presentation context is not an open window",
            SessionError::NotRunning => "the session is not running",
            SessionError::CallbackMismatch => "the callback URL does not use the session's scheme",
        };
        f.write_str(text)
    }
}

/// The window an authentication request is presented over. `anchor` is the id of an open window; `None` means the
/// app did not provide one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PresentationContext {
    pub anchor: Option<u32>,
}

impl PresentationContext {
    /// A context anchored to window `id`.
    pub fn over(id: u32) -> PresentationContext {
        PresentationContext { anchor: Some(id) }
    }

    /// Checks the context against the open window ids and returns the anchor window.
    pub fn validate(&self, open_windows: &[u32]) -> Result<u32, SessionError> {
        match self.anchor {
            None => Err(SessionError::PresentationContextNotProvided),
            Some(id) if open_windows.contains(&id) => Ok(id),
            Some(_) => Err(SessionError::PresentationContextInvalid),
        }
    }
}

/// The size of the biometric state icon. BifrOSt's own pixel values; the platform does not publish them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ControlSize {
    Mini,
    Small,
    Regular,
    Large,
    ExtraLarge,
}

impl ControlSize {
    /// Side length of the icon in points.
    pub fn points(self) -> u32 {
        match self {
            ControlSize::Mini => 16,
            ControlSize::Small => 20,
            ControlSize::Regular => 24,
            ControlSize::Large => 32,
            ControlSize::ExtraLarge => 40,
        }
    }
}

/// Where a web authentication session is in its life.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum State {
    Idle,
    Running,
    /// Finished with the callback URL the service redirected to.
    Finished(String),
    Cancelled,
}

/// A web authentication session: opens `url` and finishes when a redirect to `callback_scheme` arrives.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WebSession {
    pub url: String,
    pub callback_scheme: String,
    /// Ask the browser for a session that keeps no login state.
    pub prefers_ephemeral: bool,
    state: State,
}

impl WebSession {
    pub fn new(url: &str, callback_scheme: &str) -> WebSession {
        WebSession {
            url: url.to_string(),
            callback_scheme: callback_scheme.to_string(),
            prefers_ephemeral: false,
            state: State::Idle,
        }
    }

    pub fn state(&self) -> &State {
        &self.state
    }

    /// True when the session is idle, the URL is http or https, and the context names an open window.
    pub fn can_start(&self, context: &PresentationContext, open_windows: &[u32]) -> bool {
        self.state == State::Idle && is_web_url(&self.url) && context.validate(open_windows).is_ok()
    }

    /// Starts the session over the context's window.
    pub fn start(&mut self, context: &PresentationContext, open_windows: &[u32]) -> Result<(), SessionError> {
        if self.state != State::Idle {
            return Err(SessionError::NotRunning);
        }
        context.validate(open_windows)?;
        self.state = State::Running;
        Ok(())
    }

    /// The user cancels the login. Returns false when the session is not running.
    pub fn cancel(&mut self) -> bool {
        if self.state == State::Running {
            self.state = State::Cancelled;
            true
        } else {
            false
        }
    }

    /// Completes the session with a redirect URL. Only a redirect that uses the session's callback scheme counts.
    pub fn complete(&mut self, redirect: &str) -> Result<String, SessionError> {
        if self.state != State::Running {
            return Err(SessionError::NotRunning);
        }
        if scheme_of(redirect) != Some(self.callback_scheme.as_str()) {
            return Err(SessionError::CallbackMismatch);
        }
        self.state = State::Finished(redirect.to_string());
        Ok(redirect.to_string())
    }

    /// The completion the app receives: the callback URL, or the cancel error. `None` while the session has not ended.
    pub fn outcome(&self) -> Option<Result<String, SessionError>> {
        match &self.state {
            State::Finished(url) => Some(Ok(url.clone())),
            State::Cancelled => Some(Err(SessionError::CanceledLogin)),
            _ => None,
        }
    }
}

/// The scheme of a URL: the text before the first `:`, lower-cased, when it is a valid scheme name.
pub fn scheme_of(url: &str) -> Option<&str> {
    let end = url.find(':')?;
    let scheme = &url[..end];
    let mut chars = scheme.chars();
    let first = chars.next()?;
    if !first.is_ascii_alphabetic() || !chars.all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '-' || c == '.') {
        return None;
    }
    Some(scheme)
}

fn is_web_url(url: &str) -> bool {
    matches!(scheme_of(url), Some("http") | Some("https"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn context_needs_an_open_window() {
        assert_eq!(PresentationContext { anchor: None }.validate(&[1, 2]), Err(SessionError::PresentationContextNotProvided));
        assert_eq!(PresentationContext::over(3).validate(&[1, 2]), Err(SessionError::PresentationContextInvalid));
        assert_eq!(PresentationContext::over(2).validate(&[1, 2]), Ok(2));
    }

    #[test]
    fn control_sizes_grow_in_order() {
        let sizes = [ControlSize::Mini, ControlSize::Small, ControlSize::Regular, ControlSize::Large, ControlSize::ExtraLarge];
        let points: Vec<u32> = sizes.iter().map(|s| s.points()).collect();
        assert_eq!(points, vec![16, 20, 24, 32, 40]);
        assert!(points.windows(2).all(|w| w[0] < w[1]));
    }

    #[test]
    fn scheme_is_read_only_when_it_is_a_valid_name() {
        assert_eq!(scheme_of("myapp://done?code=1"), Some("myapp"));
        assert_eq!(scheme_of("https://example.com/a"), Some("https"));
        assert_eq!(scheme_of("1bad://x"), None);
        assert_eq!(scheme_of("no-colon-here"), None);
    }

    #[test]
    fn session_starts_only_for_web_urls_over_an_open_window() {
        let open = [7];
        let good = WebSession::new("https://id.example.com/auth", "myapp");
        assert!(good.can_start(&PresentationContext::over(7), &open));
        assert!(!good.can_start(&PresentationContext::over(8), &open));
        let file = WebSession::new("file:///etc/passwd", "myapp");
        assert!(!file.can_start(&PresentationContext::over(7), &open));
    }

    #[test]
    fn redirect_with_the_callback_scheme_finishes_the_session() {
        let mut s = WebSession::new("https://id.example.com/auth", "myapp");
        s.start(&PresentationContext::over(7), &[7]).unwrap();
        assert_eq!(s.state(), &State::Running);
        assert_eq!(s.complete("other://x"), Err(SessionError::CallbackMismatch));
        assert_eq!(s.complete("myapp://done?token=t"), Ok("myapp://done?token=t".to_string()));
        assert_eq!(s.outcome(), Some(Ok("myapp://done?token=t".to_string())));
        // A finished session cannot start or complete again.
        assert_eq!(s.start(&PresentationContext::over(7), &[7]), Err(SessionError::NotRunning));
        assert_eq!(s.complete("myapp://again"), Err(SessionError::NotRunning));
    }

    #[test]
    fn cancel_reports_canceled_login() {
        let mut s = WebSession::new("https://id.example.com/auth", "myapp");
        assert!(!s.cancel());
        assert_eq!(s.outcome(), None);
        s.start(&PresentationContext::over(7), &[7]).unwrap();
        assert!(s.cancel());
        assert_eq!(s.outcome(), Some(Err(SessionError::CanceledLogin)));
        assert!(!s.cancel());
    }

    #[test]
    fn start_with_a_bad_context_leaves_the_session_idle() {
        let mut s = WebSession::new("https://id.example.com/auth", "myapp");
        assert_eq!(s.start(&PresentationContext { anchor: None }, &[7]), Err(SessionError::PresentationContextNotProvided));
        assert_eq!(s.state(), &State::Idle);
    }
}
