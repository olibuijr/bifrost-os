//! Navigation policy for web content: which requests a page may make, and whether a response is shown or saved.
//! The three decisions (allow, cancel, download) are the ones the web content API's navigation delegate returns, for
//! each request before it starts and for each response when its headers arrive. These are pure rules over the URL,
//! the frame, the status and the content type; the engine calls them and acts on the result.

/// What to do with a navigation request or response.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Policy {
    Allow,
    Cancel,
    /// Save the content as a file instead of showing it.
    Download,
}

/// Rules for one web view: schemes a page may load, and hosts that are refused (with their subdomains).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NavigationPolicy {
    /// Lowercase schemes a navigation may use.
    pub schemes: Vec<String>,
    /// Lowercase hosts refused, each also refusing its subdomains.
    pub blocked_hosts: Vec<String>,
}

/// A request a page is about to make.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NavigationAction<'a> {
    pub url: &'a str,
    /// False for a request from a frame inside the page.
    pub main_frame: bool,
    /// The link carries the `download` attribute, so the user asked for a file.
    pub download: bool,
}

/// The response to a request, before its body is shown.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NavigationResponse<'a> {
    pub url: &'a str,
    pub status: u16,
    pub mime: &'a str,
    pub content_disposition: Option<&'a str>,
}

impl NavigationPolicy {
    /// The web defaults: http, https, about, blob and data, with no host refused.
    pub fn web() -> Self {
        Self {
            schemes: ["http", "https", "about", "blob", "data"].iter().map(|s| s.to_string()).collect(),
            blocked_hosts: Vec::new(),
        }
    }

    /// Refuses `host` and its subdomains from now on. Case is ignored.
    pub fn block_host(&mut self, host: &str) {
        let host = host.trim().to_ascii_lowercase();
        if !host.is_empty() && !self.blocked_hosts.contains(&host) {
            self.blocked_hosts.push(host);
        }
    }

    /// Decision for a request: cancelled for a scheme or host outside the rules, saved when the link asks for a file.
    pub fn decide_action(&self, action: &NavigationAction) -> Policy {
        match scheme_of(action.url) {
            Some(scheme) if self.schemes.iter().any(|s| *s == scheme) => {}
            _ => return Policy::Cancel,
        }
        if host_of(action.url).is_some_and(|host| self.is_blocked(&host)) {
            return Policy::Cancel;
        }
        if action.download {
            return Policy::Download;
        }
        Policy::Allow
    }

    /// Decision for a response: cancelled for a refused URL or a status that must not show content (204, 205),
    /// saved for an attachment or a content type the page cannot show, and allowed otherwise.
    pub fn decide_response(&self, response: &NavigationResponse) -> Policy {
        match scheme_of(response.url) {
            Some(scheme) if self.schemes.iter().any(|s| *s == scheme) => {}
            _ => return Policy::Cancel,
        }
        if host_of(response.url).is_some_and(|host| self.is_blocked(&host)) {
            return Policy::Cancel;
        }
        if response.status == 204 || response.status == 205 {
            return Policy::Cancel;
        }
        let attachment = response
            .content_disposition
            .and_then(|d| d.split(';').next())
            .is_some_and(|kind| kind.trim().eq_ignore_ascii_case("attachment"));
        if attachment || !shows_inline(response.mime) {
            return Policy::Download;
        }
        Policy::Allow
    }

    fn is_blocked(&self, host: &str) -> bool {
        self.blocked_hosts.iter().any(|b| host == b.as_str() || host.strip_suffix(b.as_str()).is_some_and(|p| p.ends_with('.')))
    }
}

/// True for content a page shows itself: text, images, audio, video, PDF, XML, JSON, and content whose type is not
/// stated (the engine then sniffs it).
fn shows_inline(mime: &str) -> bool {
    let kind = mime.split(';').next().unwrap_or("").trim().to_ascii_lowercase();
    kind.is_empty()
        || kind.starts_with("text/")
        || kind.starts_with("image/")
        || kind.starts_with("audio/")
        || kind.starts_with("video/")
        || matches!(kind.as_str(), "application/xhtml+xml" | "application/xml" | "application/json" | "application/pdf")
}

/// The lowercase scheme of `url` (`https` in `https://x`), or `None` when the text does not start with a scheme.
fn scheme_of(url: &str) -> Option<String> {
    let (scheme, _) = url.split_once(':')?;
    let mut chars = scheme.chars();
    let first_ok = chars.next().is_some_and(|c| c.is_ascii_alphabetic());
    let rest_ok = chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'));
    (first_ok && rest_ok).then(|| scheme.to_ascii_lowercase())
}

/// The lowercase host of an authority URL (`example.com` in `https://user@example.com:8443/p`), or `None`.
fn host_of(url: &str) -> Option<String> {
    let (_, rest) = url.split_once("://")?;
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host_port = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    let host = if let Some(bracketed) = host_port.strip_prefix('[') {
        bracketed.split_once(']').map_or(bracketed, |(h, _)| h)
    } else {
        match host_port.rsplit_once(':') {
            Some((h, port)) if port.chars().all(|c| c.is_ascii_digit()) => h,
            _ => host_port,
        }
    };
    (!host.is_empty()).then(|| host.to_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn action(url: &str) -> NavigationAction<'_> {
        NavigationAction { url, main_frame: true, download: false }
    }

    fn response<'a>(url: &'a str, status: u16, mime: &'a str, disposition: Option<&'a str>) -> NavigationResponse<'a> {
        NavigationResponse { url, status, mime, content_disposition: disposition }
    }

    #[test]
    fn only_listed_schemes_may_load() {
        let policy = NavigationPolicy::web();
        assert_eq!(policy.decide_action(&action("https://example.test/")), Policy::Allow);
        assert_eq!(policy.decide_action(&action("HTTP://example.test/")), Policy::Allow);
        assert_eq!(policy.decide_action(&action("file:///etc/passwd")), Policy::Cancel);
        assert_eq!(policy.decide_action(&action("javascript:alert(1)")), Policy::Cancel);
        assert_eq!(policy.decide_action(&action("not a url")), Policy::Cancel);
        assert_eq!(policy.decide_action(&action("1http://example.test/")), Policy::Cancel);
    }

    #[test]
    fn a_blocked_host_refuses_its_subdomains_but_not_similar_names() {
        let mut policy = NavigationPolicy::web();
        policy.block_host("Tracker.Test");
        assert_eq!(policy.decide_action(&action("https://tracker.test/x")), Policy::Cancel);
        assert_eq!(policy.decide_action(&action("https://a.b.tracker.test/x")), Policy::Cancel);
        assert_eq!(policy.decide_action(&action("https://nottracker.test/")), Policy::Allow);
        assert_eq!(policy.decide_action(&action("https://tracker.test.example/")), Policy::Allow);
    }

    #[test]
    fn host_matching_ignores_userinfo_port_and_case() {
        let mut policy = NavigationPolicy::web();
        policy.block_host("example.com");
        assert_eq!(policy.decide_action(&action("https://user:pw@Example.com:8443/path?q=1")), Policy::Cancel);
        assert_eq!(policy.decide_action(&action("https://[::1]:8080/")), Policy::Allow);
        assert_eq!(policy.decide_action(&action("https://example.com.evil.test/")), Policy::Allow);
    }

    #[test]
    fn a_download_link_asks_for_a_file() {
        let policy = NavigationPolicy::web();
        let link = NavigationAction { url: "https://example.test/report.pdf", main_frame: true, download: true };
        assert_eq!(policy.decide_action(&link), Policy::Download);
        let frame = NavigationAction { url: "https://example.test/frame", main_frame: false, download: false };
        assert_eq!(policy.decide_action(&frame), Policy::Allow);
    }

    #[test]
    fn responses_are_shown_or_saved_by_type_and_disposition() {
        let policy = NavigationPolicy::web();
        let url = "https://example.test/x";
        assert_eq!(policy.decide_response(&response(url, 200, "text/html; charset=utf-8", None)), Policy::Allow);
        assert_eq!(policy.decide_response(&response(url, 200, "application/octet-stream", None)), Policy::Download);
        assert_eq!(policy.decide_response(&response(url, 200, "text/html", Some("attachment; filename=a.html"))), Policy::Download);
        assert_eq!(policy.decide_response(&response(url, 200, "text/html", Some("inline"))), Policy::Allow);
        assert_eq!(policy.decide_response(&response(url, 204, "text/html", None)), Policy::Cancel);
        assert_eq!(policy.decide_response(&response("file:///etc/hosts", 200, "text/plain", None)), Policy::Cancel);
    }

    #[test]
    fn a_blocked_host_cancels_responses_too() {
        let mut policy = NavigationPolicy::web();
        policy.block_host("ads.test");
        let blocked = response("https://cdn.ads.test/a.js", 200, "application/javascript", None);
        assert_eq!(policy.decide_response(&blocked), Policy::Cancel);
    }
}
