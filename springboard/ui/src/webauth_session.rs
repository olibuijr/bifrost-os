//! Web sign-in sessions: the request an app makes to sign a user in through a web page, and the callback
//! the page hands back. The model checks the start URL and the callback, holds the session state, and routes
//! a redirect to the one running session that asked for it, so another app that registered the same URL
//! scheme never receives it. Pure: no page is opened here. The shell drives the page and reports the redirect.

/// How the page reports completion: a custom URL scheme (`myapp://...`), or an https URL with a fixed host and path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Callback {
    CustomScheme(String),
    Https { host: String, path: String },
}

/// Why a session ended without a callback.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Failure {
    /// The user closed the page, or the app cancelled the session.
    Canceled,
    /// No place to show the page was given when the session started.
    NoPresentationContext,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum State {
    Idle,
    Running,
    /// The callback URL the page handed back, as received.
    Finished(String),
    Failed(Failure),
}

/// One sign-in request: the page to open, how it calls back, and the options that shape the browser session.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WebAuthSession {
    url: String,
    callback: Callback,
    ephemeral: bool,
    headers: Vec<(String, String)>,
    state: State,
}

impl WebAuthSession {
    pub fn new(url: &str, callback: Callback) -> Self {
        WebAuthSession { url: url.to_string(), callback, ephemeral: false, headers: Vec::new(), state: State::Idle }
    }

    pub fn url(&self) -> &str {
        &self.url
    }

    pub fn callback(&self) -> &Callback {
        &self.callback
    }

    /// Asks the browser for a private session that keeps no cookies or history from earlier sign-ins.
    pub fn set_prefers_ephemeral(&mut self, on: bool) {
        self.ephemeral = on;
    }

    pub fn prefers_ephemeral(&self) -> bool {
        self.ephemeral
    }

    /// Sets a header sent with the first request. A name replaces an earlier one, ignoring ASCII case.
    pub fn set_header(&mut self, name: &str, value: &str) {
        match self.headers.iter().position(|(n, _)| n.eq_ignore_ascii_case(name)) {
            Some(i) => self.headers[i].1 = value.to_string(),
            None => self.headers.push((name.to_string(), value.to_string())),
        }
    }

    pub fn headers(&self) -> &[(String, String)] {
        &self.headers
    }

    pub fn state(&self) -> &State {
        &self.state
    }

    /// Whether `start` would succeed: the session is idle, the start URL is http or https with a host, and the
    /// callback is well formed.
    pub fn can_start(&self) -> bool {
        self.state == State::Idle && valid_start_url(&self.url) && valid_callback(&self.callback)
    }

    /// Starts the session. `has_context` says whether the app gave a place to show the page. Returns false and
    /// leaves the state alone when the session cannot start; a missing place fails the session.
    pub fn start(&mut self, has_context: bool) -> bool {
        if !self.can_start() {
            return false;
        }
        if !has_context {
            self.state = State::Failed(Failure::NoPresentationContext);
            return false;
        }
        self.state = State::Running;
        true
    }

    /// Cancels a running session. A session that is not running is left as it is.
    pub fn cancel(&mut self) {
        if self.state == State::Running {
            self.state = State::Failed(Failure::Canceled);
        }
    }

    /// Whether a redirect is the callback this running session waits for.
    pub fn accepts(&self, redirect: &str) -> bool {
        self.state == State::Running && callback_matches(&self.callback, redirect)
    }

    /// Finishes the session with a redirect it accepts. Returns false for any other URL.
    pub fn complete(&mut self, redirect: &str) -> bool {
        if !self.accepts(redirect) {
            return false;
        }
        self.state = State::Finished(redirect.to_string());
        true
    }
}

/// Holds sessions that are open at once and routes each redirect to the oldest running session that accepts it.
#[derive(Debug)]
pub struct SessionRouter {
    next_id: u64,
    sessions: Vec<(u64, WebAuthSession)>,
}

impl SessionRouter {
    pub fn new() -> Self {
        SessionRouter { next_id: 1, sessions: Vec::new() }
    }

    /// Adds a session and returns its id. Ids are never reused.
    pub fn add(&mut self, session: WebAuthSession) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        self.sessions.push((id, session));
        id
    }

    pub fn get(&self, id: u64) -> Option<&WebAuthSession> {
        self.sessions.iter().find(|(i, _)| *i == id).map(|(_, s)| s)
    }

    /// Routes a redirect. Returns the id of the session it completed, or None when no running session accepts it.
    pub fn deliver(&mut self, redirect: &str) -> Option<u64> {
        let index = self.sessions.iter().position(|(_, s)| s.accepts(redirect))?;
        let id = self.sessions[index].0;
        self.sessions[index].1.complete(redirect);
        Some(id)
    }

    /// Removes a session and returns it, if it was known.
    pub fn remove(&mut self, id: u64) -> Option<WebAuthSession> {
        let index = self.sessions.iter().position(|(i, _)| *i == id)?;
        Some(self.sessions.remove(index).1)
    }

    /// How many sessions are running now.
    pub fn running(&self) -> usize {
        self.sessions.iter().filter(|(_, s)| *s.state() == State::Running).count()
    }
}

/// The scheme of a URL (lowercased): the text before the first colon, when it is a valid scheme name.
pub fn scheme_of(url: &str) -> Option<String> {
    let colon = url.find(':')?;
    let scheme = &url[..colon];
    let mut chars = scheme.chars();
    let first = chars.next()?;
    if !first.is_ascii_alphabetic() {
        return None;
    }
    if !chars.all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '-' || c == '.') {
        return None;
    }
    Some(scheme.to_ascii_lowercase())
}

/// The host (lowercased, without port or user information) and the path of a URL with an authority.
/// The query and fragment are dropped. A URL with no path has the path "/".
pub fn host_and_path(url: &str) -> Option<(String, String)> {
    let rest = url.split_once("://")?.1;
    let end = rest.find(|c: char| c == '?' || c == '#').unwrap_or(rest.len());
    let rest = &rest[..end];
    let (authority, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };
    let host_port = authority.rsplit('@').next().unwrap_or("");
    let host = host_port.split(':').next().unwrap_or("");
    Some((host.to_ascii_lowercase(), path.to_string()))
}

fn valid_start_url(url: &str) -> bool {
    match scheme_of(url).as_deref() {
        Some("http") | Some("https") => {}
        _ => return false,
    }
    matches!(host_and_path(url), Some((host, _)) if !host.is_empty())
}

fn valid_callback(callback: &Callback) -> bool {
    match callback {
        Callback::CustomScheme(s) => scheme_of(&format!("{s}:")).is_some(),
        Callback::Https { host, path } => !host.is_empty() && path.starts_with('/'),
    }
}

fn callback_matches(callback: &Callback, redirect: &str) -> bool {
    match callback {
        Callback::CustomScheme(s) => scheme_of(redirect).as_deref() == Some(s.to_ascii_lowercase().as_str()),
        Callback::Https { host, path } => {
            scheme_of(redirect).as_deref() == Some("https")
                && host_and_path(redirect) == Some((host.to_ascii_lowercase(), path.clone()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn custom(s: &str) -> Callback {
        Callback::CustomScheme(s.to_string())
    }

    #[test]
    fn scheme_is_lowercased_and_must_be_a_valid_name() {
        assert_eq!(scheme_of("MyApp://cb?x=1"), Some("myapp".to_string()));
        assert_eq!(scheme_of("no-colon-here"), None);
        assert_eq!(scheme_of("1bad:x"), None);
        assert_eq!(scheme_of("a b:x"), None);
    }

    #[test]
    fn host_and_path_drop_port_userinfo_query_and_fragment() {
        assert_eq!(
            host_and_path("https://Login.Example.com:8443/oauth/cb?x=1#frag"),
            Some(("login.example.com".to_string(), "/oauth/cb".to_string()))
        );
        assert_eq!(host_and_path("https://user@idp.test"), Some(("idp.test".to_string(), "/".to_string())));
        assert_eq!(host_and_path("mailto-no-authority"), None);
    }

    #[test]
    fn start_url_and_callback_must_be_usable() {
        assert!(WebAuthSession::new("https://idp.example/auth", custom("myapp")).can_start());
        assert!(!WebAuthSession::new("ftp://idp.example/auth", custom("myapp")).can_start());
        assert!(!WebAuthSession::new("https://idp.example/auth", custom("1bad")).can_start());
        assert!(!WebAuthSession::new("https:///auth", custom("myapp")).can_start());
        let bad_path = Callback::Https { host: "idp.example".to_string(), path: "cb".to_string() };
        assert!(!WebAuthSession::new("https://idp.example/auth", bad_path).can_start());
    }

    #[test]
    fn start_needs_a_presentation_context_and_only_runs_once() {
        let mut s = WebAuthSession::new("https://idp.example/auth", custom("myapp"));
        assert!(!s.start(false));
        assert_eq!(s.state(), &State::Failed(Failure::NoPresentationContext));
        let mut t = WebAuthSession::new("https://idp.example/auth", custom("myapp"));
        assert!(t.start(true));
        assert_eq!(t.state(), &State::Running);
        assert!(!t.start(true));
        assert_eq!(t.state(), &State::Running);
    }

    #[test]
    fn custom_scheme_callback_finishes_with_the_redirect() {
        let mut s = WebAuthSession::new("https://idp.example/auth", custom("myapp"));
        s.start(true);
        assert!(!s.complete("otherapp://done?code=1"));
        assert!(s.complete("MYAPP://done?code=1"));
        assert_eq!(s.state(), &State::Finished("MYAPP://done?code=1".to_string()));
    }

    #[test]
    fn https_callback_needs_host_path_and_scheme() {
        let cb = Callback::Https { host: "Idp.Example".to_string(), path: "/cb".to_string() };
        let mut s = WebAuthSession::new("https://idp.example/auth", cb);
        s.start(true);
        assert!(!s.accepts("https://idp.example/cb2"));
        assert!(!s.accepts("http://idp.example/cb"));
        assert!(s.accepts("https://idp.example/cb?code=2"));
    }

    #[test]
    fn cancel_fails_a_running_session_and_ignores_later_redirects() {
        let mut s = WebAuthSession::new("https://idp.example/auth", custom("myapp"));
        s.cancel();
        assert_eq!(s.state(), &State::Idle);
        s.start(true);
        s.cancel();
        assert_eq!(s.state(), &State::Failed(Failure::Canceled));
        assert!(!s.complete("myapp://done"));
    }

    #[test]
    fn router_sends_each_redirect_to_the_oldest_running_match() {
        let mut router = SessionRouter::new();
        let mut first = WebAuthSession::new("https://idp.example/a", custom("myapp"));
        let mut second = WebAuthSession::new("https://idp.example/b", custom("myapp"));
        first.start(true);
        second.start(true);
        let a = router.add(first);
        let b = router.add(second);
        assert_eq!((a, b), (1, 2));
        assert_eq!(router.running(), 2);
        assert_eq!(router.deliver("myapp://done"), Some(a));
        assert_eq!(router.get(b).map(|s| s.state().clone()), Some(State::Running));
        assert_eq!(router.deliver("myapp://again"), Some(b));
        assert_eq!(router.deliver("myapp://none"), None);
        assert_eq!(router.running(), 0);
        assert!(router.remove(a).is_some());
        assert!(router.remove(a).is_none());
    }

    #[test]
    fn headers_replace_by_name_ignoring_case() {
        let mut s = WebAuthSession::new("https://idp.example/auth", custom("myapp"));
        s.set_header("Accept", "text/html");
        s.set_header("accept", "application/json");
        s.set_header("X-Client", "bifrost");
        assert_eq!(
            s.headers(),
            &[
                ("Accept".to_string(), "application/json".to_string()),
                ("X-Client".to_string(), "bifrost".to_string()),
            ]
        );
    }
}
