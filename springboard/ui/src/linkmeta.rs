//! Rich links as pure models: link metadata read from a page's head, a metadata provider state machine (start,
//! cancel, timeout, complete), and a link view that shows a placeholder for a URL or a card for loaded metadata.
//!
//! No network happens here. The caller fetches the page bytes, hands them to [`LinkMetadata::from_html`], and draws
//! the [`Card`] from [`LinkView::card`]. Time is passed in as seconds, so timeouts are deterministic in tests.
//!
//! Gaps: no image, icon or video download (the caller fetches [`LinkMetadata::subresources`]), no redirect following
//! (the caller passes the final URL), no HTML entity beyond the common named and numeric ones, and no share-sheet
//! rich preview.

/// Why a link could not be validated, fetched or presented.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LinkError {
    /// The text is not an absolute http or https URL with a host and no whitespace.
    InvalidUrl,
    /// The scheme is not http or https.
    UnsupportedScheme,
    /// The fetch policy has a timeout that is zero, negative, not finite, or above [`MAX_TIMEOUT_SECS`].
    BadPolicy,
    /// The page did not answer within the policy's timeout.
    Timeout,
    /// The fetch was cancelled before it completed.
    Cancelled,
    /// The page has no title, description, icon, image or video that could be presented.
    NoMetadata,
}

/// Longest timeout a fetch policy may ask for, in seconds.
pub const MAX_TIMEOUT_SECS: f64 = 120.0;
/// Timeout used when the caller does not choose one, in seconds.
pub const DEFAULT_TIMEOUT_SECS: f64 = 30.0;

/// How a metadata fetch runs: how long it may take, and whether the images and video it names are fetched too.
#[derive(Clone, Debug, PartialEq)]
pub struct FetchPolicy {
    pub timeout_secs: f64,
    pub fetch_subresources: bool,
}

impl Default for FetchPolicy {
    fn default() -> Self {
        FetchPolicy { timeout_secs: DEFAULT_TIMEOUT_SECS, fetch_subresources: true }
    }
}

impl FetchPolicy {
    /// Checks the timeout: positive, finite and at most [`MAX_TIMEOUT_SECS`].
    pub fn validate(&self) -> Result<(), LinkError> {
        if self.timeout_secs.is_finite() && self.timeout_secs > 0.0 && self.timeout_secs <= MAX_TIMEOUT_SECS {
            Ok(())
        } else {
            Err(LinkError::BadPolicy)
        }
    }
}

/// The host of an URL without scheme, user information, port, path, query or fragment.
fn host_part(url: &str) -> &str {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    let authority = rest.split(|c| c == '/' || c == '?' || c == '#').next().unwrap_or("");
    let host = authority.rsplit('@').next().unwrap_or("");
    host.split(':').next().unwrap_or("")
}

/// Checks that `text` is an absolute http or https URL with a host and no whitespace, and returns it trimmed.
pub fn validate_url(text: &str) -> Result<String, LinkError> {
    let t = text.trim();
    if t.is_empty() || t.chars().any(char::is_whitespace) {
        return Err(LinkError::InvalidUrl);
    }
    let (scheme, _) = t.split_once("://").ok_or(LinkError::InvalidUrl)?;
    if !scheme.eq_ignore_ascii_case("http") && !scheme.eq_ignore_ascii_case("https") {
        return Err(LinkError::UnsupportedScheme);
    }
    if host_part(t).is_empty() {
        return Err(LinkError::InvalidUrl);
    }
    Ok(t.to_string())
}

/// Resolves a reference found in a page against the page's URL. Absolute http and https URLs stay as they are,
/// `//host/x` takes the page's scheme, `/x` the page's origin, and anything else the page's directory. Other
/// schemes (`data:`, `javascript:`) and invalid results give `None`. Dot segments are not normalised.
pub fn resolve(base: &str, href: &str) -> Option<String> {
    let h = href.trim();
    if h.is_empty() {
        return None;
    }
    if h.contains("://") {
        return validate_url(h).ok();
    }
    let (scheme, rest) = base.split_once("://")?;
    if let Some(r) = h.strip_prefix("//") {
        return validate_url(&format!("{scheme}://{r}")).ok();
    }
    if let Some(i) = h.find(':') {
        let before = &h[..i];
        if !before.is_empty() && before.chars().all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '-' || c == '.') {
            return None;
        }
    }
    let auth_end = rest.find(|c| c == '/' || c == '?' || c == '#').unwrap_or(rest.len());
    let origin = format!("{scheme}://{}", &rest[..auth_end]);
    if h.starts_with('/') {
        return validate_url(&format!("{origin}{h}")).ok();
    }
    let path = rest[auth_end..].split(|c| c == '?' || c == '#').next().unwrap_or("");
    let dir = match path.rfind('/') {
        Some(i) => &path[..=i],
        None => "/",
    };
    validate_url(&format!("{origin}{dir}{h}")).ok()
}

/// Decodes the entities a page uses in attribute values and titles.
fn decode(s: &str) -> String {
    s.replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&#x27;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

/// Collapses runs of whitespace to one space and trims the ends.
fn collapse(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The attributes of one tag, names lower-cased and values decoded. Quoted, unquoted and bare attributes are read.
fn attrs(tag: &str) -> Vec<(String, String)> {
    let cs: Vec<char> = tag.chars().collect();
    let mut out = Vec::new();
    let mut i = 1;
    while i < cs.len() && !cs[i].is_whitespace() && cs[i] != '>' && cs[i] != '/' {
        i += 1;
    }
    loop {
        while i < cs.len() && (cs[i].is_whitespace() || cs[i] == '/') {
            i += 1;
        }
        if i >= cs.len() || cs[i] == '>' {
            break;
        }
        let start = i;
        while i < cs.len() && !cs[i].is_whitespace() && cs[i] != '=' && cs[i] != '>' {
            i += 1;
        }
        let name: String = cs[start..i].iter().collect::<String>().to_ascii_lowercase();
        while i < cs.len() && cs[i].is_whitespace() {
            i += 1;
        }
        let mut value = String::new();
        if i < cs.len() && cs[i] == '=' {
            i += 1;
            while i < cs.len() && cs[i].is_whitespace() {
                i += 1;
            }
            if i < cs.len() && (cs[i] == '"' || cs[i] == '\'') {
                let q = cs[i];
                i += 1;
                let s = i;
                while i < cs.len() && cs[i] != q {
                    i += 1;
                }
                value = cs[s..i].iter().collect();
                i += 1;
            } else {
                let s = i;
                while i < cs.len() && !cs[i].is_whitespace() && cs[i] != '>' {
                    i += 1;
                }
                value = cs[s..i].iter().collect();
            }
        }
        if !name.is_empty() {
            out.push((name, decode(&value)));
        }
    }
    out
}

/// Every `<name ...>` tag before the body, as source slices. Matching ignores ASCII case.
fn tags<'a>(html: &'a str, name: &str) -> Vec<&'a str> {
    let lower = html.to_ascii_lowercase();
    let end = lower.find("<body").unwrap_or(lower.len());
    let needle = format!("<{name}");
    let mut out = Vec::new();
    let mut from = 0;
    while from < end {
        let Some(i) = lower[from..end].find(&needle) else { break };
        let at = from + i;
        let close = match html[at..].find('>') {
            Some(e) => at + e + 1,
            None => html.len(),
        };
        out.push(&html[at..close]);
        from = close;
    }
    out
}

/// The text of the first `<title>` before the body, decoded and collapsed.
fn title_text(html: &str) -> Option<String> {
    let lower = html.to_ascii_lowercase();
    let start = lower.find("<title")?;
    let open = start + lower[start..].find('>')? + 1;
    let close = open + lower[open..].find("</title")?;
    Some(collapse(&decode(&html[open..close])))
}

/// Metadata about one URL, as read from the page's head: the title, description, site name, icon, image and video.
/// `url` is the final address after redirects; `original_url` is the one that was asked for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkMetadata {
    pub original_url: String,
    pub url: String,
    pub title: String,
    pub site_name: String,
    pub description: String,
    pub icon: Option<String>,
    pub image: Option<String>,
    pub video: Option<String>,
}

impl LinkMetadata {
    /// Reads the metadata of a page. Titles come from `og:title`, then `twitter:title`, then `<title>`; an empty
    /// title falls back to the host. Images come from `og:image` and its variants, videos from `og:video`, and the
    /// icon from a `<link rel>` that names an icon (a non-touch icon is preferred). Relative references are
    /// resolved against the final URL. Fails with [`LinkError::NoMetadata`] when the page offers nothing to show.
    pub fn from_html(original_url: &str, final_url: &str, html: &str) -> Result<LinkMetadata, LinkError> {
        let original = validate_url(original_url)?;
        let url = validate_url(final_url)?;
        let metas: Vec<(String, String)> = tags(html, "meta")
            .into_iter()
            .filter_map(|t| {
                let a = attrs(t);
                let get = |n: &str| a.iter().find(|(k, _)| k.as_str() == n).map(|(_, v)| v.clone());
                let key = get("property").or_else(|| get("name"))?.trim().to_ascii_lowercase();
                let content = get("content")?.trim().to_string();
                if content.is_empty() {
                    None
                } else {
                    Some((key, content))
                }
            })
            .collect();
        let icons: Vec<(String, String)> = tags(html, "link")
            .into_iter()
            .filter_map(|t| {
                let a = attrs(t);
                let get = |n: &str| a.iter().find(|(k, _)| k.as_str() == n).map(|(_, v)| v.clone());
                let rel = get("rel")?.to_ascii_lowercase();
                let href = get("href")?.trim().to_string();
                if rel.contains("icon") && !href.is_empty() {
                    Some((rel, href))
                } else {
                    None
                }
            })
            .collect();
        let pick = |keys: &[&str]| {
            keys.iter()
                .find_map(|k| metas.iter().find(|(m, _)| m.as_str() == *k).map(|(_, v)| v.clone()))
        };
        let og_title = pick(&["og:title", "twitter:title"]);
        let title_tag = title_text(html).filter(|t| !t.is_empty());
        let description = pick(&["og:description", "twitter:description", "description"]);
        let site_name = pick(&["og:site_name"]).map(|s| collapse(&s)).unwrap_or_default();
        let image = pick(&["og:image:secure_url", "og:image", "og:image:url", "twitter:image", "twitter:image:src"])
            .and_then(|v| resolve(&url, &v));
        let video = pick(&["og:video:secure_url", "og:video:url", "og:video"]).and_then(|v| resolve(&url, &v));
        let icon_href = icons
            .iter()
            .find(|(rel, _)| !rel.contains("touch"))
            .or_else(|| icons.iter().find(|(rel, _)| rel.contains("touch")))
            .map(|(_, h)| h.clone());
        let icon = icon_href.and_then(|h| resolve(&url, &h));
        let found = og_title.is_some()
            || title_tag.is_some()
            || description.is_some()
            || image.is_some()
            || video.is_some()
            || icon.is_some();
        if !found {
            return Err(LinkError::NoMetadata);
        }
        let title = og_title
            .or(title_tag)
            .map(|t| collapse(&t))
            .filter(|t| !t.is_empty())
            .unwrap_or_else(|| host_part(&url).to_string());
        Ok(LinkMetadata {
            original_url: original,
            url,
            title,
            site_name,
            description: description.map(|d| collapse(&d)).unwrap_or_default(),
            icon,
            image,
            video,
        })
    }

    /// The icon, image and video addresses the caller should fetch. Empty when the policy does not fetch subresources.
    pub fn subresources(&self, policy: &FetchPolicy) -> Vec<&str> {
        if !policy.fetch_subresources {
            return Vec::new();
        }
        let mut out = Vec::new();
        for u in [&self.icon, &self.image, &self.video] {
            if let Some(s) = u.as_deref() {
                out.push(s);
            }
        }
        out
    }
}

/// Where a metadata fetch is.
#[derive(Clone, Debug, PartialEq)]
pub enum FetchState {
    Idle,
    Fetching { url: String, started_at: f64, policy: FetchPolicy },
    Done(LinkMetadata),
    Failed(LinkError),
}

/// Runs one metadata fetch at a time. The caller starts it, performs the request, and reports the answer with
/// [`MetadataProvider::complete`] or [`MetadataProvider::fail`]. A late answer after a cancel or a timeout is ignored.
#[derive(Clone, Debug, PartialEq)]
pub struct MetadataProvider {
    state: FetchState,
}

impl Default for MetadataProvider {
    fn default() -> Self {
        MetadataProvider { state: FetchState::Idle }
    }
}

impl MetadataProvider {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn state(&self) -> &FetchState {
        &self.state
    }

    /// Starts a fetch of `text` at time `now` in seconds. A fetch already running is replaced. An invalid URL or
    /// policy sets the state to failed and returns that error.
    pub fn start(&mut self, text: &str, policy: FetchPolicy, now: f64) -> Result<(), LinkError> {
        let checked = validate_url(text).and_then(|u| policy.validate().map(|_| u));
        match checked {
            Ok(url) => {
                self.state = FetchState::Fetching { url, started_at: now, policy };
                Ok(())
            }
            Err(e) => {
                self.state = FetchState::Failed(e.clone());
                Err(e)
            }
        }
    }

    /// Cancels a running fetch. Nothing happens when no fetch is running.
    pub fn cancel(&mut self) {
        if matches!(self.state, FetchState::Fetching { .. }) {
            self.state = FetchState::Failed(LinkError::Cancelled);
        }
    }

    /// Fails a running fetch whose transport reported `err`. Returns false when no fetch is running.
    pub fn fail(&mut self, err: LinkError) -> bool {
        if matches!(self.state, FetchState::Fetching { .. }) {
            self.state = FetchState::Failed(err);
            true
        } else {
            false
        }
    }

    /// Fails a running fetch whose timeout has passed at time `now`. Returns true when it timed out.
    pub fn tick(&mut self, now: f64) -> bool {
        let expired = match &self.state {
            FetchState::Fetching { started_at, policy, .. } => now - *started_at >= policy.timeout_secs,
            _ => false,
        };
        if expired {
            self.state = FetchState::Failed(LinkError::Timeout);
        }
        expired
    }

    /// Takes the page that answered the running fetch at time `now`. Returns false, and changes nothing, when no
    /// fetch is running or the answer came after the timeout.
    pub fn complete(&mut self, final_url: &str, html: &str, now: f64) -> bool {
        let original = match &self.state {
            FetchState::Fetching { url, .. } => url.clone(),
            _ => return false,
        };
        if self.tick(now) {
            return false;
        }
        self.state = match LinkMetadata::from_html(&original, final_url, html) {
            Ok(m) => FetchState::Done(m),
            Err(e) => FetchState::Failed(e),
        };
        true
    }
}

/// What a link card draws: the title, a subtitle (site name or host), the icon and image addresses, whether a video
/// is available, and whether this is the placeholder shown before metadata arrives.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Card {
    pub title: String,
    pub subtitle: String,
    pub icon: Option<String>,
    pub image: Option<String>,
    pub has_video: bool,
    pub placeholder: bool,
}

/// A rich link: a placeholder for a URL that has no metadata yet, or the loaded metadata.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LinkView {
    Placeholder { url: String },
    Loaded(LinkMetadata),
}

impl LinkView {
    /// A placeholder for `text`, which must be a valid URL.
    pub fn placeholder(text: &str) -> Result<LinkView, LinkError> {
        Ok(LinkView::Placeholder { url: validate_url(text)? })
    }

    pub fn loaded(metadata: LinkMetadata) -> LinkView {
        LinkView::Loaded(metadata)
    }

    /// The card for the current state.
    pub fn card(&self) -> Card {
        match self {
            LinkView::Placeholder { url } => Card {
                title: host_part(url).to_string(),
                subtitle: host_part(url).to_string(),
                icon: None,
                image: None,
                has_video: false,
                placeholder: true,
            },
            LinkView::Loaded(m) => Card {
                title: m.title.clone(),
                subtitle: if m.site_name.is_empty() { host_part(&m.url).to_string() } else { m.site_name.clone() },
                icon: m.icon.clone(),
                image: m.image.clone(),
                has_video: m.video.is_some(),
                placeholder: false,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAGE: &str = "<html><head><title> Fallback  title </title>\
        <meta property='og:title' content='Fréttir &amp; más'>\
        <meta name=description content=\"Short summary\">\
        <meta property=og:site_name content=News>\
        <meta property=\"og:image\" content=\"/media/cover.jpg?w=600&amp;h=400\">\
        <meta property=\"og:video:url\" content=\"https://cdn.example.is/clip.mp4\">\
        <link rel=\"touch-icon\" href=\"/touch.png\">\
        <link rel=\"shortcut icon\" href=\"icons/fav.ico\">\
        </head><body><meta property=\"og:title\" content=\"ignored\"></body></html>";

    #[test]
    fn reads_head_metadata_and_resolves_references_against_the_final_url() {
        let m = LinkMetadata::from_html("https://short.is/a", "https://news.example.is/posts/1.html", PAGE).unwrap();
        assert_eq!(m.original_url, "https://short.is/a");
        assert_eq!(m.url, "https://news.example.is/posts/1.html");
        assert_eq!(m.title, "Fréttir & más");
        assert_eq!(m.site_name, "News");
        assert_eq!(m.description, "Short summary");
        assert_eq!(m.image.as_deref(), Some("https://news.example.is/media/cover.jpg?w=600&h=400"));
        assert_eq!(m.video.as_deref(), Some("https://cdn.example.is/clip.mp4"));
        // a non-touch icon is preferred over the touch icon, and resolves against the directory
        assert_eq!(m.icon.as_deref(), Some("https://news.example.is/posts/icons/fav.ico"));
    }

    #[test]
    fn title_falls_back_from_open_graph_to_twitter_to_title_tag_to_host() {
        let tw = "<title>Tag</title><meta name='twitter:title' content='Card title'>";
        assert_eq!(LinkMetadata::from_html("https://a.is/", "https://a.is/", tw).unwrap().title, "Card title");
        let tag = "<title>\n  Tag   line </title><meta property='og:description' content='d'>";
        assert_eq!(LinkMetadata::from_html("https://a.is/", "https://a.is/", tag).unwrap().title, "Tag line");
        let bare = "<meta property='og:image' content='https://a.is/x.png'>";
        let m = LinkMetadata::from_html("https://a.is/", "https://www.a.is:8443/p", bare).unwrap();
        assert_eq!(m.title, "www.a.is");
        assert_eq!(m.image.as_deref(), Some("https://a.is/x.png"));
    }

    #[test]
    fn a_page_with_nothing_to_show_is_no_metadata() {
        assert_eq!(LinkMetadata::from_html("https://a.is/", "https://a.is/", "<p>hi</p>"), Err(LinkError::NoMetadata));
        // a meta tag in the body is not read: only the head counts
        let body = "<body><meta property='og:title' content='x'></body>";
        assert_eq!(LinkMetadata::from_html("https://a.is/", "https://a.is/", body), Err(LinkError::NoMetadata));
    }

    #[test]
    fn validates_urls_and_rejects_other_schemes_and_whitespace() {
        assert_eq!(validate_url("  https://bifrost.olibuijr.com/x "), Ok("https://bifrost.olibuijr.com/x".to_string()));
        assert_eq!(validate_url("ftp://a.is/"), Err(LinkError::UnsupportedScheme));
        assert_eq!(validate_url("javascript:alert(1)"), Err(LinkError::InvalidUrl));
        assert_eq!(validate_url("https://a b.is/"), Err(LinkError::InvalidUrl));
        assert_eq!(validate_url("https:///path"), Err(LinkError::InvalidUrl));
        assert_eq!(validate_url(""), Err(LinkError::InvalidUrl));
    }

    #[test]
    fn resolves_protocol_relative_root_relative_and_other_schemes() {
        let base = "https://a.is/dir/page.html?x=1";
        assert_eq!(resolve(base, "//cdn.a.is/i.png").as_deref(), Some("https://cdn.a.is/i.png"));
        assert_eq!(resolve(base, "/i.png").as_deref(), Some("https://a.is/i.png"));
        assert_eq!(resolve(base, "i.png").as_deref(), Some("https://a.is/dir/i.png"));
        assert_eq!(resolve("https://a.is", "i.png").as_deref(), Some("https://a.is/i.png"));
        assert_eq!(resolve(base, "data:image/png;base64,AAAA"), None);
        assert_eq!(resolve(base, "   "), None);
    }

    #[test]
    fn provider_runs_one_fetch_and_takes_the_first_answer() {
        let mut p = MetadataProvider::new();
        p.start("https://a.is/x", FetchPolicy::default(), 10.0).unwrap();
        assert!(matches!(p.state(), FetchState::Fetching { started_at, .. } if *started_at == 10.0));
        assert!(p.complete("https://a.is/y", "<title>Y</title>", 12.0));
        match p.state() {
            FetchState::Done(m) => assert_eq!((m.url.as_str(), m.title.as_str()), ("https://a.is/y", "Y")),
            other => panic!("expected Done, got {other:?}"),
        }
        // a second answer with no fetch running is ignored
        assert!(!p.complete("https://a.is/z", "<title>Z</title>", 13.0));
    }

    #[test]
    fn cancel_and_timeout_fail_the_fetch_and_late_answers_are_ignored() {
        let mut p = MetadataProvider::new();
        p.start("https://a.is/x", FetchPolicy { timeout_secs: 5.0, fetch_subresources: false }, 0.0).unwrap();
        assert!(!p.tick(4.9));
        assert!(p.tick(5.0));
        assert_eq!(p.state(), &FetchState::Failed(LinkError::Timeout));
        assert!(!p.complete("https://a.is/x", "<title>late</title>", 6.0));

        p.start("https://a.is/x", FetchPolicy::default(), 0.0).unwrap();
        p.cancel();
        assert_eq!(p.state(), &FetchState::Failed(LinkError::Cancelled));
        assert!(!p.complete("https://a.is/x", "<title>late</title>", 1.0));
        assert!(!p.fail(LinkError::Timeout), "nothing is running after a cancel");
    }

    #[test]
    fn bad_input_fails_the_provider_with_a_typed_error() {
        let mut p = MetadataProvider::new();
        assert_eq!(p.start("mailto:a@a.is", FetchPolicy::default(), 0.0), Err(LinkError::InvalidUrl));
        assert_eq!(p.state(), &FetchState::Failed(LinkError::InvalidUrl));
        let bad = FetchPolicy { timeout_secs: 0.0, fetch_subresources: true };
        assert_eq!(p.start("https://a.is/", bad, 0.0), Err(LinkError::BadPolicy));
        let huge = FetchPolicy { timeout_secs: MAX_TIMEOUT_SECS + 1.0, fetch_subresources: true };
        assert_eq!(huge.validate(), Err(LinkError::BadPolicy));
        assert!(FetchPolicy::default().validate().is_ok());
    }

    #[test]
    fn subresources_follow_the_policy_and_the_link_view_draws_placeholder_then_card() {
        let m = LinkMetadata::from_html("https://a.is/", "https://news.example.is/p", PAGE).unwrap();
        let on = m.subresources(&FetchPolicy::default());
        assert_eq!(on.len(), 3);
        assert!(m.subresources(&FetchPolicy { timeout_secs: 5.0, fetch_subresources: false }).is_empty());

        let ph = LinkView::placeholder("https://news.example.is/p?q=1").unwrap().card();
        assert!(ph.placeholder && ph.title == "news.example.is" && !ph.has_video && ph.image.is_none());

        let card = LinkView::loaded(m).card();
        assert!(!card.placeholder);
        assert_eq!(card.subtitle, "News");
        assert!(card.has_video);
        assert_eq!(card.title, "Fréttir & más");
    }
}
