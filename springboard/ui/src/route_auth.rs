//! Per-app authorization of third-party media routes, and the active state of each route. An app may only activate a
//! route the user has authorized. Invalidating an authorization also deactivates the route. The model tracks route
//! identifiers only; it never opens a connection.

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RouteError {
    /// The route has no authorization, so it cannot be activated.
    Unauthorized,
}

#[derive(Clone, Debug, Default)]
pub struct RouteAuth {
    authorized: Vec<String>,
    active: Vec<String>,
    changed: bool,
}

impl RouteAuth {
    pub fn new() -> Self {
        Self::default()
    }

    /// Records the user's authorization of a route. Returns false when the identifier is empty or already authorized.
    pub fn authorize(&mut self, route: &str) -> bool {
        let route = route.trim();
        if route.is_empty() || self.is_authorized(route) {
            return false;
        }
        self.authorized.push(route.to_string());
        self.changed = true;
        true
    }

    pub fn is_authorized(&self, route: &str) -> bool {
        self.authorized.iter().any(|r| r == route)
    }

    /// Authorized routes in the order they were authorized.
    pub fn authorized_routes(&self) -> &[String] {
        &self.authorized
    }

    /// Sets the active state of a route. Activating needs an authorization; deactivating always succeeds.
    pub fn set_active(&mut self, route: &str, active: bool) -> Result<(), RouteError> {
        if !active {
            self.active.retain(|r| r != route);
            return Ok(());
        }
        if !self.is_authorized(route) {
            return Err(RouteError::Unauthorized);
        }
        if !self.is_active(route) {
            self.active.push(route.to_string());
        }
        Ok(())
    }

    pub fn is_active(&self, route: &str) -> bool {
        self.active.iter().any(|r| r == route)
    }

    /// Revokes the authorization and deactivates the route. Returns false when the route was not authorized.
    pub fn invalidate(&mut self, route: &str) -> bool {
        let before = self.authorized.len();
        self.authorized.retain(|r| r != route);
        self.active.retain(|r| r != route);
        let removed = self.authorized.len() != before;
        if removed {
            self.changed = true;
        }
        removed
    }

    /// True once after the list of authorized routes changed. Reading it clears the flag.
    pub fn take_changed(&mut self) -> bool {
        std::mem::take(&mut self.changed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn authorize_rejects_empty_and_repeated_routes() {
        let mut a = RouteAuth::new();
        assert!(a.authorize(" tv-1 "));
        assert!(!a.authorize("tv-1"));
        assert!(!a.authorize("   "));
        assert_eq!(a.authorized_routes(), &["tv-1".to_string()]);
    }

    #[test]
    fn activation_needs_an_authorization() {
        let mut a = RouteAuth::new();
        assert_eq!(a.set_active("speaker", true), Err(RouteError::Unauthorized));
        assert!(!a.is_active("speaker"));
        a.authorize("speaker");
        assert_eq!(a.set_active("speaker", true), Ok(()));
        assert!(a.is_active("speaker"));
    }

    #[test]
    fn deactivating_an_unknown_route_always_succeeds() {
        let mut a = RouteAuth::new();
        assert_eq!(a.set_active("nobody", false), Ok(()));
        assert!(!a.is_active("nobody"));
    }

    #[test]
    fn activating_twice_needs_one_deactivation() {
        let mut a = RouteAuth::new();
        a.authorize("tv");
        a.set_active("tv", true).unwrap();
        a.set_active("tv", true).unwrap();
        a.set_active("tv", false).unwrap();
        assert!(!a.is_active("tv"));
    }

    #[test]
    fn invalidate_revokes_and_deactivates_only_that_route() {
        let mut a = RouteAuth::new();
        a.authorize("tv");
        a.authorize("hub");
        a.set_active("tv", true).unwrap();
        a.set_active("hub", true).unwrap();
        assert!(a.invalidate("tv"));
        assert!(!a.is_authorized("tv"));
        assert!(!a.is_active("tv"));
        assert!(a.is_active("hub"));
        assert!(!a.invalidate("tv"), "a second revoke finds nothing");
    }

    #[test]
    fn take_changed_reports_each_change_once() {
        let mut a = RouteAuth::new();
        assert!(!a.take_changed());
        a.authorize("tv");
        assert!(a.take_changed());
        assert!(!a.take_changed());
        assert!(!a.invalidate("missing"), "nothing removed, so no change");
        assert!(!a.take_changed());
        assert!(a.invalidate("tv"));
        assert!(a.take_changed());
    }
}
