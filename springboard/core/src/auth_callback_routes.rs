//! Callback routes for web authentication sessions (parity row: single sign-on in a web browser app).
//!
//! A web authentication session finishes when the service redirects to a callback scheme such as `myapp://`. The shell
//! has to decide which app that redirect belongs to. This module is that decision as a pure table: an app registers
//! one scheme, the shell asks which app owns a redirect URL, and a callback is delivered only to the app that owns its
//! scheme. Reserved web and browser schemes can never be registered, so a page cannot take over `https` redirects.
//!
//! Pure model: no socket, no window and no IPC. The shell keeps one [`CallbackRoutes`] and calls it from the place that
//! receives redirects. The browser app does not act as an SSO broker for other apps (see the matrix row); this table is
//! the shared part that a broker would need.

use crate::auth_session::scheme_of;
use std::fmt;

/// Schemes that belong to the web platform or to the browser itself. No app may register them as a callback.
const RESERVED: [&str; 8] = ["http", "https", "file", "about", "data", "javascript", "blob", "chrome"];

/// Why a callback route could not be registered or used.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RouteError {
    /// The scheme text is not a valid URL scheme name.
    InvalidScheme,
    /// The scheme is reserved for the web platform or the browser.
    ReservedScheme,
    /// The scheme already belongs to another app.
    SchemeTaken,
    /// No app owns the scheme of the redirect URL.
    NoRoute,
    /// The redirect URL belongs to an app other than the one that asked for it.
    NotRequestingApp,
}

impl fmt::Display for RouteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            RouteError::InvalidScheme => "the callback scheme is not a valid URL scheme name",
            RouteError::ReservedScheme => "the callback scheme is reserved for the web or the browser",
            RouteError::SchemeTaken => "the callback scheme already belongs to another app",
            RouteError::NoRoute => "no app owns the callback scheme",
            RouteError::NotRequestingApp => "the callback belongs to an app that did not start the session",
        };
        f.write_str(text)
    }
}

/// One scheme owned by one app. The scheme is stored in lower case.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Route {
    pub scheme: String,
    pub app_id: String,
}

/// The table of callback schemes. Each scheme has at most one owner.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CallbackRoutes {
    routes: Vec<Route>,
}

impl CallbackRoutes {
    /// An empty table.
    pub fn new() -> CallbackRoutes {
        CallbackRoutes::default()
    }

    /// Gives `scheme` to `app_id`. Registering the same scheme for the same app again succeeds and changes nothing.
    pub fn register(&mut self, scheme: &str, app_id: &str) -> Result<(), RouteError> {
        let scheme = normalized(scheme)?;
        if RESERVED.contains(&scheme.as_str()) {
            return Err(RouteError::ReservedScheme);
        }
        match self.routes.iter().find(|r| r.scheme == scheme) {
            Some(existing) if existing.app_id == app_id => Ok(()),
            Some(_) => Err(RouteError::SchemeTaken),
            None => {
                self.routes.push(Route { scheme, app_id: app_id.to_string() });
                Ok(())
            }
        }
    }

    /// Removes every scheme that `app_id` owns, for example when the app quits. Returns how many were removed.
    pub fn unregister_app(&mut self, app_id: &str) -> usize {
        let before = self.routes.len();
        self.routes.retain(|r| r.app_id != app_id);
        before - self.routes.len()
    }

    /// The app that owns the scheme of `url`.
    pub fn owner_of(&self, url: &str) -> Result<&str, RouteError> {
        let scheme = scheme_of(url).ok_or(RouteError::NoRoute)?.to_ascii_lowercase();
        self.routes
            .iter()
            .find(|r| r.scheme == scheme)
            .map(|r| r.app_id.as_str())
            .ok_or(RouteError::NoRoute)
    }

    /// Delivers a redirect to `requesting_app`. Succeeds only when that app owns the scheme of `url`, so a callback can
    /// reach only the app that started the session.
    pub fn deliver<'a>(&self, url: &'a str, requesting_app: &str) -> Result<&'a str, RouteError> {
        match self.owner_of(url)? {
            owner if owner == requesting_app => Ok(url),
            _ => Err(RouteError::NotRequestingApp),
        }
    }

    /// The routes in registration order.
    pub fn routes(&self) -> &[Route] {
        &self.routes
    }
}

/// Checks and lower-cases a scheme name given by an app.
fn normalized(scheme: &str) -> Result<String, RouteError> {
    match scheme_of(&format!("{scheme}:")) {
        Some(valid) if valid.len() == scheme.len() => Ok(valid.to_ascii_lowercase()),
        _ => Err(RouteError::InvalidScheme),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_registered_scheme_routes_to_its_app() {
        let mut routes = CallbackRoutes::new();
        routes.register("MyApp", "is.example.myapp").unwrap();
        assert_eq!(routes.owner_of("myapp://done?code=1"), Ok("is.example.myapp"));
        assert_eq!(routes.routes()[0].scheme, "myapp");
    }

    #[test]
    fn reserved_and_malformed_schemes_are_refused() {
        let mut routes = CallbackRoutes::new();
        assert_eq!(routes.register("https", "a"), Err(RouteError::ReservedScheme));
        assert_eq!(routes.register("Chrome", "a"), Err(RouteError::ReservedScheme));
        assert_eq!(routes.register("1bad", "a"), Err(RouteError::InvalidScheme));
        assert_eq!(routes.register("my app", "a"), Err(RouteError::InvalidScheme));
        assert_eq!(routes.register("", "a"), Err(RouteError::InvalidScheme));
        assert!(routes.routes().is_empty());
    }

    #[test]
    fn a_scheme_belongs_to_one_app_until_it_is_removed() {
        let mut routes = CallbackRoutes::new();
        routes.register("shared", "app.one").unwrap();
        assert_eq!(routes.register("shared", "app.one"), Ok(()));
        assert_eq!(routes.register("shared", "app.two"), Err(RouteError::SchemeTaken));
        assert_eq!(routes.unregister_app("app.one"), 1);
        assert_eq!(routes.register("shared", "app.two"), Ok(()));
    }

    #[test]
    fn a_callback_is_delivered_only_to_the_requesting_app() {
        let mut routes = CallbackRoutes::new();
        routes.register("myapp", "app.one").unwrap();
        let url = "myapp://cb?code=7";
        assert_eq!(routes.deliver(url, "app.one"), Ok(url));
        assert_eq!(routes.deliver(url, "app.two"), Err(RouteError::NotRequestingApp));
        assert_eq!(routes.deliver("otherapp://x", "app.one"), Err(RouteError::NoRoute));
        assert_eq!(routes.owner_of("no-colon-here"), Err(RouteError::NoRoute));
    }
}
