//! What a widget may reach outside its own drawing: the user's location, and the network hosts it may contact.
//! Pure model of the widget access rules. A widget has no access until the user grants it, and grants are kept
//! per widget identifier. A network grant lists host names; a host matches itself and its subdomains, so a grant
//! for `example.com` covers `api.example.com` but not `notexample.com`. Nothing here fetches anything or reads a
//! location: the shell asks this model before it does either.

use std::collections::{BTreeSet, HashMap};

/// The two accesses a widget can be granted.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Capability {
    Location,
    Network,
}

/// Why a host name was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HostError {
    /// The text holds no host name at all.
    Empty,
    /// The text is not a plain host name (a label is empty, too long, or has a character outside letters,
    /// digits and hyphens; ports and user information are refused too).
    Malformed(String),
}

/// Longest host name in bytes, without the trailing dot.
pub const MAX_HOST_LEN: usize = 253;

/// Reduces text to a bare host name: a scheme, a path, a query and a fragment are dropped, letters are lower
/// cased and a trailing dot is removed. Returns an error for text that is not a plain host name.
pub fn normalize_host(raw: &str) -> Result<String, HostError> {
    let mut text = raw.trim().to_ascii_lowercase();
    if let Some(i) = text.find("://") {
        text = text[i + 3..].to_string();
    }
    let host = text.split(|c: char| c == '/' || c == '?' || c == '#').next().unwrap_or("");
    let host = host.trim_end_matches('.');
    if host.is_empty() {
        return Err(HostError::Empty);
    }
    let plain = host.len() <= MAX_HOST_LEN
        && host.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        });
    if plain {
        Ok(host.to_string())
    } else {
        Err(HostError::Malformed(host.to_string()))
    }
}

/// True when `host` is `allowed` or one of its subdomains. Both names must already be normalized.
pub fn host_matches(host: &str, allowed: &str) -> bool {
    host == allowed || host.ends_with(&format!(".{allowed}"))
}

/// What one widget has been granted.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Grant {
    pub location: bool,
    /// Normalized host names the widget may contact.
    pub hosts: BTreeSet<String>,
}

/// The grants of every widget, keyed by widget bundle identifier.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Access {
    grants: HashMap<String, Grant>,
}

impl Access {
    /// The grant held by a widget. A widget that was never granted anything holds the empty grant.
    pub fn grant_of(&self, bundle_id: &str) -> Grant {
        self.grants.get(bundle_id).cloned().unwrap_or_default()
    }

    /// Sets the location grant. Switching it off keeps the network hosts.
    pub fn set_location(&mut self, bundle_id: &str, allowed: bool) {
        self.grants.entry(bundle_id.to_string()).or_default().location = allowed;
    }

    /// Adds a host to the widget's network grant and returns the normalized name it was stored under.
    pub fn allow_host(&mut self, bundle_id: &str, raw: &str) -> Result<String, HostError> {
        let host = normalize_host(raw)?;
        self.grants.entry(bundle_id.to_string()).or_default().hosts.insert(host.clone());
        Ok(host)
    }

    /// Removes a host from the widget's network grant. Returns true when it was there.
    pub fn revoke_host(&mut self, bundle_id: &str, raw: &str) -> bool {
        let Ok(host) = normalize_host(raw) else {
            return false;
        };
        self.grants.get_mut(bundle_id).map(|g| g.hosts.remove(&host)).unwrap_or(false)
    }

    /// True when the widget may read the location.
    pub fn may_locate(&self, bundle_id: &str) -> bool {
        self.grants.get(bundle_id).map(|g| g.location).unwrap_or(false)
    }

    /// True when the widget may contact `target`, a host name or a URL. A target that is not a plain host, and a
    /// widget with no network grant, are both refused.
    pub fn may_contact(&self, bundle_id: &str, target: &str) -> bool {
        let Ok(host) = normalize_host(target) else {
            return false;
        };
        match self.grants.get(bundle_id) {
            Some(g) => g.hosts.iter().any(|allowed| host_matches(&host, allowed)),
            None => false,
        }
    }

    /// Drops every grant of a widget whose app was removed.
    pub fn forget(&mut self, bundle_id: &str) {
        self.grants.remove(bundle_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_url_is_reduced_to_its_host() {
        assert_eq!(normalize_host("https://API.Example.com/v1/forecast?x=1#top").as_deref(), Ok("api.example.com"));
        assert_eq!(normalize_host("example.com.").as_deref(), Ok("example.com"));
        assert_eq!(normalize_host("   "), Err(HostError::Empty));
    }

    #[test]
    fn text_that_is_not_a_plain_host_is_refused() {
        assert!(matches!(normalize_host("example.com:8080"), Err(HostError::Malformed(_))));
        assert!(matches!(normalize_host("user@example.com"), Err(HostError::Malformed(_))));
        assert!(matches!(normalize_host("-bad.example.com"), Err(HostError::Malformed(_))));
        assert!(matches!(normalize_host("a..b"), Err(HostError::Malformed(_))));
        let long = "a".repeat(64);
        assert!(matches!(normalize_host(&long), Err(HostError::Malformed(_))));
    }

    #[test]
    fn a_host_matches_itself_and_its_subdomains_only() {
        assert!(host_matches("example.com", "example.com"));
        assert!(host_matches("api.example.com", "example.com"));
        assert!(!host_matches("notexample.com", "example.com"));
        assert!(!host_matches("example.com.evil.net", "example.com"));
    }

    #[test]
    fn a_widget_has_no_access_until_it_is_granted() {
        let access = Access::default();
        assert!(!access.may_locate("is.example.weather"));
        assert!(!access.may_contact("is.example.weather", "https://api.example.com/"));
        assert_eq!(access.grant_of("is.example.weather"), Grant::default());
    }

    #[test]
    fn a_network_grant_covers_the_host_and_its_subdomains() {
        let mut access = Access::default();
        let stored = access.allow_host("is.example.weather", "https://api.open-meteo.com/v1").unwrap();
        assert_eq!(stored, "api.open-meteo.com");
        assert!(access.may_contact("is.example.weather", "api.open-meteo.com"));
        assert!(access.may_contact("is.example.weather", "https://ws.api.open-meteo.com/x"));
        assert!(!access.may_contact("is.example.weather", "open-meteo.com"));
        assert!(!access.may_contact("is.example.other", "api.open-meteo.com"));
    }

    #[test]
    fn revoking_a_host_stops_its_traffic() {
        let mut access = Access::default();
        access.allow_host("is.example.weather", "example.com").unwrap();
        assert!(access.revoke_host("is.example.weather", "https://example.com/"));
        assert!(!access.may_contact("is.example.weather", "example.com"));
        assert!(!access.revoke_host("is.example.weather", "example.com"));
    }

    #[test]
    fn location_is_granted_and_withdrawn_separately_from_the_network() {
        let mut access = Access::default();
        access.allow_host("is.example.weather", "example.com").unwrap();
        access.set_location("is.example.weather", true);
        assert!(access.may_locate("is.example.weather"));
        access.set_location("is.example.weather", false);
        assert!(!access.may_locate("is.example.weather"));
        assert!(access.may_contact("is.example.weather", "example.com"));
    }

    #[test]
    fn forgetting_a_widget_drops_every_grant() {
        let mut access = Access::default();
        access.set_location("is.example.weather", true);
        access.allow_host("is.example.weather", "example.com").unwrap();
        access.forget("is.example.weather");
        assert_eq!(access.grant_of("is.example.weather"), Grant::default());
    }
}
