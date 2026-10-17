//! Link metadata (`LPLinkMetadata` and `LPMetadataProvider` parity, see `docs/parity/link-presentation.md`).
//!
//! An app shows a web link with its title, icon and picture. This module does the parts that do not need the network:
//! - [`checked_url`] accepts only `http` and `https` addresses with a host.
//! - [`parse`] reads a page's `<title>`, Open Graph (`og:title`, `og:image`), Twitter card image, and its icon links
//!   from the HTML. Relative addresses are resolved against the page address by [`resolve`]. Anything that is not a
//!   web address (`data:`, `javascript:`) is dropped.
//! - [`LinkFetch`] is the state of one fetch: pending, done with metadata, or failed. It has a timeout (clamped to
//!   [`MIN_TIMEOUT_MS`]..=[`MAX_TIMEOUT_MS`]), it can be cancelled, and it accepts only HTML bodies. The caller does the
//!   HTTP request with `URLSession` and reports the result; this type never opens a connection.
//!
//! Not mirrored: the rich link view (`LPLinkView`), which is UI in the shared kit, and the share sheet's rich preview.
//! Those take a [`LinkMetadata`] as data.

use std::fmt;

/// What the app shows for one link. Every field but `url` may be missing on a page that does not declare it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LinkMetadata {
    pub url: String,
    pub title: Option<String>,
    pub icon: Option<String>,
    pub image: Option<String>,
}

/// `LPError`: why a metadata fetch produced nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinkError {
    InvalidUrl,
    NotHtml,
    Timeout,
    Cancelled,
    Network,
}

impl fmt::Display for LinkError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            LinkError::InvalidUrl => "the address is not a web address",
            LinkError::NotHtml => "the page is not HTML",
            LinkError::Timeout => "the page took too long to answer",
            LinkError::Cancelled => "the fetch was cancelled",
            LinkError::Network => "the page could not be loaded",
        })
    }
}

pub const DEFAULT_TIMEOUT_MS: u64 = 10_000;
pub const MIN_TIMEOUT_MS: u64 = 1_000;
pub const MAX_TIMEOUT_MS: u64 = 60_000;

/// Options for one fetch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FetchOptions {
    pub timeout_ms: u64,
}

impl Default for FetchOptions {
    fn default() -> Self {
        FetchOptions { timeout_ms: DEFAULT_TIMEOUT_MS }
    }
}

/// Returns the address when it is an `http` or `https` address with a host and no spaces.
pub fn checked_url(url: &str) -> Result<String, LinkError> {
    let url = url.trim();
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .ok_or(LinkError::InvalidUrl)?;
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    if host.is_empty() || url.chars().any(char::is_whitespace) {
        return Err(LinkError::InvalidUrl);
    }
    Ok(url.to_string())
}

/// Resolves `reference` against the page address `base`. Returns `None` for an empty reference, for a non-web
/// scheme, and for a base that is not a web address. Dot segments are not removed.
pub fn resolve(base: &str, reference: &str) -> Option<String> {
    let r = reference.trim();
    if r.is_empty() {
        return None;
    }
    if r.starts_with("https://") || r.starts_with("http://") {
        return Some(r.to_string());
    }
    if let Some(c) = r.find(':') {
        if !r[..c].contains(['/', '?', '#']) {
            return None;
        }
    }
    let scheme_end = base.find("://")?;
    let scheme = &base[..scheme_end];
    if scheme != "http" && scheme != "https" {
        return None;
    }
    if let Some(rest) = r.strip_prefix("//") {
        return Some(format!("{scheme}://{rest}"));
    }
    let after = &base[scheme_end + 3..];
    let host_end = after.find(['/', '?', '#']).unwrap_or(after.len());
    let origin = format!("{scheme}://{}", &after[..host_end]);
    if r.starts_with('/') {
        return Some(format!("{origin}{r}"));
    }
    let path = after[host_end..].split(['?', '#']).next().unwrap_or("");
    let dir = match path.rfind('/') {
        Some(i) => &path[..=i],
        None => "/",
    };
    Some(format!("{origin}{dir}{r}"))
}

/// Reads the metadata a page declares. `page_url` is the address the HTML came from.
pub fn parse(html: &str, page_url: &str) -> LinkMetadata {
    let title = meta_content(html, "og:title")
        .or_else(|| element_text(html, "title"))
        .filter(|t| !t.is_empty());
    let image = meta_content(html, "og:image")
        .or_else(|| meta_content(html, "twitter:image"))
        .and_then(|u| resolve(page_url, &u));
    let icon = link_href(html, "icon")
        .or_else(|| link_href(html, "shortcut icon"))
        .or_else(|| link_href(html, "apple-touch-icon"))
        .and_then(|u| resolve(page_url, &u));
    LinkMetadata { url: page_url.to_string(), title, icon, image }
}

/// The `content` of the first `<meta>` whose `property` or `name` is `key`, with entities decoded and spaces trimmed.
fn meta_content(html: &str, key: &str) -> Option<String> {
    tags(html, "meta").into_iter().find_map(|tag| {
        let name = attr(tag, "property").or_else(|| attr(tag, "name"))?;
        if !name.eq_ignore_ascii_case(key) {
            return None;
        }
        let content = attr(tag, "content")?.trim().to_string();
        (!content.is_empty()).then_some(content)
    })
}

/// The `href` of the first `<link>` whose `rel` is `rel`.
fn link_href(html: &str, rel: &str) -> Option<String> {
    tags(html, "link").into_iter().find_map(|tag| {
        let found = attr(tag, "rel")?;
        if !found.split_ascii_whitespace().collect::<Vec<_>>().join(" ").eq_ignore_ascii_case(rel) {
            return None;
        }
        attr(tag, "href").filter(|h| !h.trim().is_empty())
    })
}

/// The collapsed text of the first `<name>` element.
fn element_text(html: &str, name: &str) -> Option<String> {
    let lower = html.to_ascii_lowercase();
    let open = lower.find(&format!("<{name}"))?;
    let body = open + lower[open..].find('>')? + 1;
    let close = body + lower[body..].find(&format!("</{name}"))?;
    let text = entities(&html[body..close]);
    let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
    (!collapsed.is_empty()).then_some(collapsed)
}

/// Every `<name ...>` start tag in the page, from `<` to `>`.
fn tags<'a>(html: &'a str, name: &str) -> Vec<&'a str> {
    let lower = html.to_ascii_lowercase();
    let needle = format!("<{name}");
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(i) = lower[from..].find(&needle) {
        let at = from + i;
        let end = html[at..].find('>').map_or(html.len(), |e| at + e);
        let tag = &html[at..end];
        if tag[needle.len()..].starts_with(|c: char| c.is_ascii_whitespace() || c == '/') {
            out.push(tag);
        }
        from = end;
    }
    out
}

/// The value of attribute `name` in a start tag. Accepts double, single and unquoted values; the name matches only
/// as a whole attribute, so `data-content` does not match `content`.
fn attr(tag: &str, name: &str) -> Option<String> {
    let lower = tag.to_ascii_lowercase();
    let mut from = 0;
    while let Some(i) = lower[from..].find(name) {
        let at = from + i;
        let after = at + name.len();
        let whole = at == 0 || lower.as_bytes()[at - 1].is_ascii_whitespace();
        let rest = tag[after..].trim_start();
        if whole && rest.starts_with('=') {
            let value = rest[1..].trim_start();
            let quote = value.chars().next()?;
            if quote == '"' || quote == '\'' {
                let body = &value[1..];
                return body.find(quote).map(|e| entities(&body[..e]));
            }
            let end = value.find(|c: char| c.is_whitespace() || c == '>').unwrap_or(value.len());
            return Some(entities(&value[..end]));
        }
        from = after;
    }
    None
}

/// Decodes the entities a page uses in titles and attribute values.
fn entities(s: &str) -> String {
    s.replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

/// The state of one metadata fetch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FetchState {
    Pending,
    Done(LinkMetadata),
    Failed(LinkError),
}

/// One metadata fetch. The caller starts it, performs the request, and reports the result with [`LinkFetch::finish_html`]
/// or [`LinkFetch::fail`]. Once the state is `Done` or `Failed`, later reports are ignored.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkFetch {
    url: String,
    timeout_ms: u64,
    state: FetchState,
}

impl LinkFetch {
    /// Starts a fetch for `url`. Fails when the address is not a web address. The timeout is clamped to the allowed range.
    pub fn start(url: &str, options: FetchOptions) -> Result<LinkFetch, LinkError> {
        let url = checked_url(url)?;
        Ok(LinkFetch {
            url,
            timeout_ms: options.timeout_ms.clamp(MIN_TIMEOUT_MS, MAX_TIMEOUT_MS),
            state: FetchState::Pending,
        })
    }

    pub fn url(&self) -> &str {
        &self.url
    }

    pub fn timeout_ms(&self) -> u64 {
        self.timeout_ms
    }

    pub fn state(&self) -> &FetchState {
        &self.state
    }

    pub fn is_pending(&self) -> bool {
        self.state == FetchState::Pending
    }

    /// Cancels a pending fetch. Returns false when it had already finished.
    pub fn cancel(&mut self) -> bool {
        self.fail(LinkError::Cancelled)
    }

    /// Fails a pending fetch once `elapsed_ms` reaches the timeout. Returns true when it fails now.
    pub fn check_timeout(&mut self, elapsed_ms: u64) -> bool {
        if self.is_pending() && elapsed_ms >= self.timeout_ms {
            return self.fail(LinkError::Timeout);
        }
        false
    }

    /// Reports a network failure for a pending fetch. Returns false when it had already finished.
    pub fn fail(&mut self, error: LinkError) -> bool {
        if !self.is_pending() {
            return false;
        }
        self.state = FetchState::Failed(error);
        true
    }

    /// Reports the body of a finished request. A body whose content type is not HTML fails with `NotHtml`; an empty
    /// content type is taken as HTML. Returns false when the fetch had already finished.
    pub fn finish_html(&mut self, body: &str, content_type: &str) -> bool {
        if !self.is_pending() {
            return false;
        }
        let ct = content_type.to_ascii_lowercase();
        if !ct.is_empty() && !ct.contains("html") {
            self.state = FetchState::Failed(LinkError::NotHtml);
            return true;
        }
        self.state = FetchState::Done(parse(body, &self.url));
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAGE: &str = "https://www.visir.is/news/a?x=1";

    #[test]
    fn parse_reads_og_title_image_and_icon() {
        let html = r#"<head><title>Ignored</title>
            <meta property="og:title" content="Fréttir &amp; meira">
            <meta property="og:image" content="https://cdn.example.is/a_1600x900.jpg" />
            <link rel="icon" href="/favicon.ico"></head>"#;
        let m = parse(html, PAGE);
        assert_eq!(m.url, PAGE);
        assert_eq!(m.title.as_deref(), Some("Fréttir & meira"));
        assert_eq!(m.image.as_deref(), Some("https://cdn.example.is/a_1600x900.jpg"));
        assert_eq!(m.icon.as_deref(), Some("https://www.visir.is/favicon.ico"));
    }

    #[test]
    fn parse_falls_back_to_the_title_element_and_collapses_spaces() {
        let m = parse("<title>\n  Hello\n   world </title>", PAGE);
        assert_eq!(m.title.as_deref(), Some("Hello world"));
        assert_eq!(parse("<title>   </title>", PAGE).title, None);
        assert_eq!(parse("<p>no head</p>", PAGE), LinkMetadata { url: PAGE.into(), ..Default::default() });
    }

    #[test]
    fn parse_takes_the_twitter_image_when_there_is_no_og_image() {
        let m = parse(r#"<meta name="twitter:image" content="/card.png">"#, PAGE);
        assert_eq!(m.image.as_deref(), Some("https://www.visir.is/card.png"));
    }

    #[test]
    fn parse_prefers_the_icon_link_then_the_touch_icon() {
        let html = r#"<link rel="apple-touch-icon" href="/touch.png"><link rel="shortcut icon" href="/s.ico">"#;
        assert_eq!(parse(html, PAGE).icon.as_deref(), Some("https://www.visir.is/s.ico"));
        let touch = r#"<link rel="apple-touch-icon" href="/touch.png">"#;
        assert_eq!(parse(touch, PAGE).icon.as_deref(), Some("https://www.visir.is/touch.png"));
    }

    #[test]
    fn parse_drops_non_web_picture_and_icon_addresses() {
        let html = r#"<meta property="og:image" content="data:image/png;base64,AAAA">
            <link rel="icon" href="javascript:alert(1)">"#;
        let m = parse(html, PAGE);
        assert_eq!(m.image, None);
        assert_eq!(m.icon, None);
    }

    #[test]
    fn attr_matches_whole_names_and_all_quote_styles() {
        assert_eq!(attr(r#"<meta data-content="no" content='yes'>"#, "content").as_deref(), Some("yes"));
        assert_eq!(attr("<meta content=bare>", "content").as_deref(), Some("bare"));
        assert_eq!(attr("<meta property=\"og:x\">", "content"), None);
    }

    #[test]
    fn resolve_handles_root_scheme_and_path_relative_references() {
        assert_eq!(resolve(PAGE, "/a.png").as_deref(), Some("https://www.visir.is/a.png"));
        assert_eq!(resolve(PAGE, "//img.example.is/b.png").as_deref(), Some("https://img.example.is/b.png"));
        assert_eq!(resolve(PAGE, "c.png").as_deref(), Some("https://www.visir.is/news/c.png"));
        assert_eq!(resolve("https://example.is", "d.png").as_deref(), Some("https://example.is/d.png"));
        assert_eq!(resolve(PAGE, "https://other.is/e.png").as_deref(), Some("https://other.is/e.png"));
        assert_eq!(resolve(PAGE, "  "), None);
        assert_eq!(resolve(PAGE, "mailto:a@b.is"), None);
        assert_eq!(resolve("ftp://example.is/x", "y.png"), None);
    }

    #[test]
    fn checked_url_accepts_only_web_addresses_with_a_host() {
        assert_eq!(checked_url(" https://ruv.is/x ").unwrap(), "https://ruv.is/x");
        assert_eq!(checked_url("ftp://ruv.is"), Err(LinkError::InvalidUrl));
        assert_eq!(checked_url("https://"), Err(LinkError::InvalidUrl));
        assert_eq!(checked_url("https:///path"), Err(LinkError::InvalidUrl));
        assert_eq!(checked_url("https://ruv.is/a b"), Err(LinkError::InvalidUrl));
    }

    #[test]
    fn a_fetch_finishes_with_the_page_metadata() {
        let mut f = LinkFetch::start(PAGE, FetchOptions::default()).unwrap();
        assert!(f.is_pending());
        assert!(f.finish_html(r#"<title>T</title>"#, "text/html; charset=utf-8"));
        match f.state() {
            FetchState::Done(m) => assert_eq!(m.title.as_deref(), Some("T")),
            other => panic!("expected Done, got {other:?}"),
        }
        assert!(!f.cancel(), "a finished fetch cannot be cancelled");
    }

    #[test]
    fn a_fetch_refuses_a_body_that_is_not_html() {
        let mut f = LinkFetch::start(PAGE, FetchOptions::default()).unwrap();
        assert!(f.finish_html("%PDF", "application/pdf"));
        assert_eq!(f.state(), &FetchState::Failed(LinkError::NotHtml));
        assert!(!f.finish_html("<title>late</title>", "text/html"));
    }

    #[test]
    fn a_fetch_can_be_cancelled_or_time_out() {
        let mut f = LinkFetch::start(PAGE, FetchOptions::default()).unwrap();
        assert!(f.cancel());
        assert_eq!(f.state(), &FetchState::Failed(LinkError::Cancelled));

        let mut slow = LinkFetch::start(PAGE, FetchOptions { timeout_ms: 2_000 }).unwrap();
        assert!(!slow.check_timeout(1_999));
        assert!(slow.check_timeout(2_000));
        assert_eq!(slow.state(), &FetchState::Failed(LinkError::Timeout));
    }

    #[test]
    fn the_timeout_is_clamped_to_the_allowed_range() {
        assert_eq!(LinkFetch::start(PAGE, FetchOptions { timeout_ms: 1 }).unwrap().timeout_ms(), MIN_TIMEOUT_MS);
        assert_eq!(LinkFetch::start(PAGE, FetchOptions { timeout_ms: 999_999 }).unwrap().timeout_ms(), MAX_TIMEOUT_MS);
        assert_eq!(LinkFetch::start(PAGE, FetchOptions::default()).unwrap().timeout_ms(), DEFAULT_TIMEOUT_MS);
    }

    #[test]
    fn a_fetch_of_a_bad_address_does_not_start() {
        assert_eq!(LinkFetch::start("notaurl", FetchOptions::default()).unwrap_err(), LinkError::InvalidUrl);
    }
}
