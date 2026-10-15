//! Web site to app associations: a site names the app that handles its paths, and a link goes to the enabled app
//! whose path prefix is longest. Pure model. The caller reads the association data and lists the installed apps;
//! this module only answers which app a link belongs to. Nothing is fetched here.

use crate::json::{self, Json};

#[derive(Clone, Debug, PartialEq, Eq)]
struct Claim {
    host: String,
    app: String,
    prefix: String,
    enabled: bool,
}

#[derive(Debug, Default)]
pub struct AppLinks {
    claims: Vec<Claim>,
}

impl AppLinks {
    pub fn new() -> Self {
        Self::default()
    }

    /// Claims the path prefixes of `host` for `app`. An empty list claims the whole site. Prefixes that do not start
    /// with `/` are ignored. A new claim for the same host and app replaces the earlier one.
    pub fn claim(&mut self, host: &str, app: &str, prefixes: &[&str]) {
        let host = host.trim().to_ascii_lowercase();
        self.claims.retain(|c| !(c.host == host && c.app == app));
        let mut paths: Vec<String> =
            prefixes.iter().filter(|p| p.starts_with('/')).map(|p| p.to_string()).collect();
        if paths.is_empty() {
            paths.push("/".to_string());
        }
        for prefix in paths {
            self.claims.push(Claim { host: host.clone(), app: app.to_string(), prefix, enabled: true });
        }
    }

    /// Claims the paths that a site association document gives to the app with id `app_id`, as `claim` does for
    /// `app`. The document is the JSON a site publishes: each `applinks.details` entry names an app id (`appID`, or
    /// `appIDs` as a list) and may list `paths`. A path ending in `*` is a prefix, and a path without `*` is taken as
    /// a prefix too, bounded at a segment. Exclusion paths (`NOT ...`) cannot be held by this model, so the whole
    /// document is refused. A refused or unreadable document leaves the earlier claims as they were.
    pub fn claim_from_association(&mut self, host: &str, app: &str, app_id: &str, text: &str) -> Result<(), String> {
        let prefixes = association_paths(text, app_id)?;
        let refs: Vec<&str> = prefixes.iter().map(String::as_str).collect();
        self.claim(host, app, &refs);
        Ok(())
    }

    /// Turns an app's link handling on or off without dropping its claims.
    pub fn set_enabled(&mut self, app: &str, enabled: bool) {
        for claim in self.claims.iter_mut().filter(|c| c.app == app) {
            claim.enabled = enabled;
        }
    }

    /// Removes every claim of `app`.
    pub fn unclaim(&mut self, app: &str) {
        self.claims.retain(|c| c.app != app);
    }

    /// The app that should open `url`, if one claims it. Only http and https addresses resolve. The query and the
    /// fragment are ignored, and the port is ignored. A claim matches its exact host, not its subdomains. Among the
    /// matching enabled claims the longest path prefix wins, and the first one registered wins a tie.
    pub fn resolve(&self, url: &str) -> Option<&str> {
        let (host, path) = split_address(url)?;
        let host = host.to_ascii_lowercase();
        let mut best: Option<&Claim> = None;
        for claim in &self.claims {
            if !claim.enabled || claim.host != host || !prefix_matches(&claim.prefix, path) {
                continue;
            }
            if best.is_none_or( |b| claim.prefix.len() > b.prefix.len()) {
                best = Some(claim);
            }
        }
        best.map(|c| c.app.as_str())
    }
}

/// The host (without user info or port) and the path of an http or https address. The path defaults to `/`.
pub fn split_address(url: &str) -> Option<(&str, &str)> {
    let rest = url.strip_prefix("https://").or_else(|| url.strip_prefix("http://"))?;
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let (authority, tail) = rest.split_at(end);
    // User info comes before the last `@`; without this `https://good.example:x@evil.example/` would read as `good.example`.
    let host_port = authority.rsplit('@').next().unwrap_or("");
    let host = host_port.split(':').next().unwrap_or("");
    if host.is_empty() {
        return None;
    }
    let path_end = tail.find(['?', '#']).unwrap_or(tail.len());
    let path = &tail[..path_end];
    Some((host, if path.is_empty() { "/" } else { path }))
}

/// The path prefixes that the association document gives to `app_id`. A detail without `paths` gives none, which
/// `claim` reads as the whole site.
fn association_paths(text: &str, app_id: &str) -> Result<Vec<String>, String> {
    let doc = json::parse(text).map_err(|e| format!("association is not JSON: {e}"))?;
    let details = doc
        .get("applinks")
        .and_then(|links| links.get("details"))
        .and_then(Json::as_array)
        .ok_or("association has no applinks.details array")?;
    let detail = details
        .iter()
        .find(|d| detail_names_app(d, app_id))
        .ok_or_else(|| format!("no association detail names app id {app_id}"))?;
    let Some(paths) = detail.get("paths") else {
        return Ok(Vec::new());
    };
    let mut prefixes = Vec::new();
    for item in paths.as_array().ok_or("association paths must be an array")? {
        let path = item.as_str().ok_or("an association path must be a string")?;
        if path.starts_with("NOT ") {
            return Err(format!("exclusion path is not supported: {path}"));
        }
        prefixes.push(path.strip_suffix('*').unwrap_or(path).to_string());
    }
    Ok(prefixes)
}

fn detail_names_app(detail: &Json, app_id: &str) -> bool {
    detail.str_field("appID").is_some_and(|id| id == app_id)
        || detail
            .get("appIDs")
            .and_then(Json::as_array)
            .is_some_and(|ids| ids.iter().any(|id| id.as_str() == Some(app_id)))
}

/// The prefix covers the path when the path starts with it and the prefix ends at a segment boundary.
fn prefix_matches(prefix: &str, path: &str) -> bool {
    if !path.starts_with(prefix) {
        return false;
    }
    prefix.ends_with('/') || path.len() == prefix.len() || path.as_bytes()[prefix.len()] == b'/'
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_link_goes_to_the_app_that_claims_its_host() {
        let mut links = AppLinks::new();
        links.claim("Shop.Example.com", "shop-app", &[]);
        assert_eq!(links.resolve("https://shop.example.com/any/page?x=1#top"), Some("shop-app"));
        assert_eq!(links.resolve("https://sub.shop.example.com/"), None);
        assert_eq!(links.resolve("https://example.com/"), None);
    }

    #[test]
    fn the_longest_path_prefix_wins_and_respects_segments() {
        let mut links = AppLinks::new();
        links.claim("example.com", "docs", &["/docs"]);
        links.claim("example.com", "docs-plus", &["/docs/guide"]);
        assert_eq!(links.resolve("https://example.com/docs/guide/intro"), Some("docs-plus"));
        assert_eq!(links.resolve("https://example.com/docs/api"), Some("docs"));
        assert_eq!(links.resolve("https://example.com/docsy"), None);
    }

    #[test]
    fn a_disabled_app_is_skipped_and_its_claims_are_kept() {
        let mut links = AppLinks::new();
        links.claim("example.com", "notes", &["/n"]);
        links.claim("example.com", "web", &[]);
        links.set_enabled("notes", false);
        assert_eq!(links.resolve("https://example.com/n/1"), Some("web"));
        links.set_enabled("notes", true);
        assert_eq!(links.resolve("https://example.com/n/1"), Some("notes"));
        links.unclaim("notes");
        assert_eq!(links.resolve("https://example.com/n/1"), Some("web"));
    }

    #[test]
    fn only_web_addresses_resolve_and_the_port_is_ignored() {
        let mut links = AppLinks::new();
        links.claim("example.com", "web", &[]);
        assert_eq!(links.resolve("https://example.com:8443/a"), Some("web"));
        assert_eq!(links.resolve("http://example.com"), Some("web"));
        assert_eq!(links.resolve("ftp://example.com/a"), None);
        assert_eq!(links.resolve("https:///a"), None);
        assert_eq!(links.resolve("example.com/a"), None);
    }

    #[test]
    fn user_info_does_not_change_the_host() {
        let mut links = AppLinks::new();
        links.claim("example.com", "web", &[]);
        assert_eq!(links.resolve("https://example.com:x@evil.example/a"), None);
        assert_eq!(links.resolve("https://evil.example@example.com/a"), Some("web"));
        assert_eq!(split_address("https://u:p@Example.com:8443/a?b#c"), Some(("Example.com", "/a")));
    }

    #[test]
    fn a_new_claim_for_the_same_host_and_app_replaces_the_earlier_paths() {
        let mut links = AppLinks::new();
        links.claim("example.com", "web", &["/a", "/b"]);
        links.claim("example.com", "web", &["/c"]);
        assert_eq!(links.resolve("https://example.com/a"), None);
        assert_eq!(links.resolve("https://example.com/c/x"), Some("web"));
    }
}
