//! Addresses: turning what was typed into a URL or a search, resolving
//! links against the page they are on, and the built-in `about:` pages
//! (start page, bookmarks, history) which are plain HTML rendered by the
//! same engine as any site.

use super::store::Library;
use url::Url;

pub const HOME: &str = "about:home";
pub const BOOKMARKS: &str = "about:bookmarks";
pub const HISTORY: &str = "about:history";
const SEARCH: &str = "https://duckduckgo.com/html/?q=";

pub fn is_about(url: &str) -> bool {
    url.starts_with("about:")
}

/// The address as shown in the bar when it is not being edited.
pub fn display_url(url: &str) -> String {
    let u = url.strip_prefix("https://").or_else(|| url.strip_prefix("http://")).unwrap_or(url);
    let u = u.strip_prefix("www.").unwrap_or(u);
    u.strip_suffix('/').unwrap_or(u).to_string()
}

pub fn percent_encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

pub fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let (mut out, mut i) = (Vec::with_capacity(b.len()), 0);
    while i < b.len() {
        match b[i] {
            b'+' => out.push(b' '),
            b'%' if b.get(i + 1..i + 3).is_some_and(|h| h.iter().all(u8::is_ascii_hexdigit)) => {
                out.push(u8::from_str_radix(&s[i + 1..i + 3], 16).unwrap_or(b'?'));
                i += 2;
            }
            c => out.push(c),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

pub fn search_url(query: &str) -> String {
    format!("{SEARCH}{}", percent_encode(query.trim()))
}

fn looks_like_host(host: &str) -> bool {
    let host = match host.rsplit_once(':') {
        Some((h, port)) if !port.is_empty() && port.bytes().all(|b| b.is_ascii_digit()) => h,
        _ => host,
    };
    if host.eq_ignore_ascii_case("localhost") {
        return true;
    }
    let labels: Vec<&str> = host.split('.').collect();
    if labels.len() < 2 || labels.iter().any(|l| l.is_empty() || !l.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')) {
        return false;
    }
    let is_ip = labels.len() == 4 && labels.iter().all(|l| l.parse::<u8>().is_ok());
    is_ip || labels.last().is_some_and(|tld| tld.len() >= 2 && tld.bytes().all(|b| b.is_ascii_alphabetic()))
}

/// What the address bar does with `text`: a web address is opened, anything
/// else is searched for. None for an empty field.
pub fn normalize_input(text: &str) -> Option<String> {
    let t = text.trim();
    if t.is_empty() {
        return None;
    }
    let lower = t.to_ascii_lowercase();
    if lower.starts_with("http://") || lower.starts_with("https://") || lower.starts_with("about:") {
        return Some(t.to_string());
    }
    if !t.contains(char::is_whitespace) {
        let host = t.split(['/', '?', '#']).next().unwrap_or(t);
        if looks_like_host(host) {
            let name = host.split(':').next().unwrap_or(host);
            let local = name.eq_ignore_ascii_case("localhost") || name.starts_with("192.168.") || name.starts_with("10.") || name.ends_with(".local");
            return Some(format!("{}://{t}", if local { "http" } else { "https" }));
        }
    }
    Some(search_url(t))
}

/// `href` resolved against `base`; None unless the result is a web or `about:` address.
pub fn resolve(base: &str, href: &str) -> Option<String> {
    let href = href.trim();
    if is_about(href) {
        return Some(href.to_string());
    }
    let mut u = match Url::parse(base) {
        Ok(b) => b.join(href).ok()?,
        Err(_) => Url::parse(href).ok()?,
    };
    u.set_fragment(None);
    matches!(u.scheme(), "http" | "https").then(|| u.to_string())
}

pub fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// A change an `about:` link asks for.
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    RemoveBookmark(String),
    RemoveHistory(String),
    ClearHistory,
}

impl Action {
    /// The page to show once the action is done.
    pub fn next(&self) -> &'static str {
        match self {
            Action::RemoveBookmark(_) => BOOKMARKS,
            Action::RemoveHistory(_) | Action::ClearHistory => HISTORY,
        }
    }
}

pub fn action(url: &str) -> Option<Action> {
    let rest = url.strip_prefix("about:")?;
    let (name, arg) = rest.split_once("?u=").map_or((rest, ""), |(n, a)| (n, a));
    match name {
        "remove-bookmark" => Some(Action::RemoveBookmark(percent_decode(arg))),
        "remove-history" => Some(Action::RemoveHistory(percent_decode(arg))),
        "clear-history" => Some(Action::ClearHistory),
        _ => None,
    }
}

fn nav_links() -> &'static str {
    "<p><a href=\"about:home\">Start</a> &middot; <a href=\"about:bookmarks\">Bookmarks</a> &middot; <a href=\"about:history\">History</a></p>"
}

/// The HTML of a built-in page; None for an unknown `about:` address.
pub fn about_page(url: &str, lib: &Library) -> Option<(String, String)> {
    let name = url.strip_prefix("about:")?;
    let list = |items: &mut dyn Iterator<Item = (String, String, Option<String>)>| -> String {
        let mut s = String::from("<ul>");
        for (title, href, remove) in items {
            let label = if title.trim().is_empty() { href.clone() } else { title };
            s.push_str(&format!("<li><a href=\"{}\">{}</a>", esc(&href), esc(&label)));
            if let Some(r) = remove {
                s.push_str(&format!(" <a href=\"{r}\">remove</a>"));
            }
            s.push_str("</li>");
        }
        s.push_str("</ul>");
        s
    };
    let (title, body) = match name {
        "blank" => ("New Tab".to_string(), String::new()),
        "home" => {
            let marks = if lib.bookmarks.is_empty() {
                "<p>No bookmarks yet. Tap the star in the toolbar to add one.</p>".to_string()
            } else {
                list(&mut lib.bookmarks.iter().take(12).map(|b| (b.title.clone(), b.url.clone(), None)))
            };
            let recent = if lib.history.is_empty() { "<p>Pages you visit show up here.</p>".to_string() } else { list(&mut lib.history.iter().take(10).map(|v| (v.title.clone(), v.url.clone(), None))) };
            ("Start".to_string(), format!("<h1>Browser</h1><p>Type a web address or a search in the bar above.</p><h2>Bookmarks</h2>{marks}<h2>Recently visited</h2>{recent}{}", nav_links()))
        }
        "bookmarks" => {
            let items = if lib.bookmarks.is_empty() {
                "<p>No bookmarks yet.</p>".to_string()
            } else {
                list(&mut lib.bookmarks.iter().map(|b| (b.title.clone(), b.url.clone(), Some(format!("about:remove-bookmark?u={}", percent_encode(&b.url))))))
            };
            ("Bookmarks".to_string(), format!("<h1>Bookmarks</h1>{items}{}", nav_links()))
        }
        "history" => {
            let items = if lib.history.is_empty() {
                "<p>No history.</p>".to_string()
            } else {
                let rows = list(&mut lib.history.iter().take(100).map(|v| (v.title.clone(), v.url.clone(), Some(format!("about:remove-history?u={}", percent_encode(&v.url))))));
                format!("<p><a href=\"about:clear-history\">Clear history</a></p>{rows}")
            };
            ("History".to_string(), format!("<h1>History</h1>{items}{}", nav_links()))
        }
        _ => return None,
    };
    Some((title.clone(), format!("<html><head><title>{}</title></head><body>{body}</body></html>", esc(&title))))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_text_becomes_an_address_or_a_search() {
        assert_eq!(normalize_input("https://a.test/x").as_deref(), Some("https://a.test/x"));
        assert_eq!(normalize_input("example.com").as_deref(), Some("https://example.com"));
        assert_eq!(normalize_input("  example.com/a b ").as_deref(), Some("https://duckduckgo.com/html/?q=example.com%2Fa+b"));
        assert_eq!(normalize_input("localhost:8080/x").as_deref(), Some("http://localhost:8080/x"));
        assert_eq!(normalize_input("192.168.1.12:8152").as_deref(), Some("http://192.168.1.12:8152"));
        assert_eq!(normalize_input("rust lifetimes").as_deref(), Some("https://duckduckgo.com/html/?q=rust+lifetimes"));
        assert_eq!(normalize_input("cats").as_deref(), Some("https://duckduckgo.com/html/?q=cats"));
        assert_eq!(normalize_input("about:bookmarks").as_deref(), Some("about:bookmarks"));
        assert_eq!(normalize_input("   "), None);
    }

    #[test]
    fn searches_are_percent_encoded() {
        assert_eq!(search_url("a&b=c \u{fe}"), "https://duckduckgo.com/html/?q=a%26b%3Dc+%C3%BE");
        assert_eq!(percent_decode("a%26b+c%C3%BE%zz%2"), "a&b c\u{fe}%zz%2");
    }

    #[test]
    fn links_resolve_against_the_page() {
        assert_eq!(resolve("https://a.test/dir/page", "../x?y=1#f").as_deref(), Some("https://a.test/x?y=1"));
        assert_eq!(resolve("https://a.test/dir/page", "//b.test/z").as_deref(), Some("https://b.test/z"));
        assert_eq!(resolve("https://a.test/", "mailto:x@y"), None);
        assert_eq!(resolve("https://a.test/", "javascript:go()"), None);
        assert_eq!(resolve("about:home", "https://b.test/").as_deref(), Some("https://b.test/"));
        assert_eq!(resolve("https://a.test/", "about:history").as_deref(), Some("about:history"));
    }

    #[test]
    fn addresses_display_without_noise() {
        assert_eq!(display_url("https://www.example.com/"), "example.com");
        assert_eq!(display_url("http://a.test/x/"), "a.test/x");
        assert_eq!(display_url("about:home"), "about:home");
    }

    #[test]
    fn built_in_pages_list_bookmarks_and_history_escaped() {
        let mut lib = Library::memory();
        lib.toggle_bookmark("Fish & <Chips>", "https://a.test/?q=1&r=2");
        lib.visit("https://b.test/", "B", 5);
        let (title, home) = about_page(HOME, &lib).unwrap();
        assert_eq!(title, "Start");
        assert!(home.contains("Fish &amp; &lt;Chips&gt;") && home.contains("href=\"https://a.test/?q=1&amp;r=2\""));
        assert!(home.contains("https://b.test/"));
        let (_, marks) = about_page(BOOKMARKS, &lib).unwrap();
        assert!(marks.contains("about:remove-bookmark?u=https%3A%2F%2Fa.test%2F%3Fq%3D1%26r%3D2"));
        let (_, hist) = about_page(HISTORY, &lib).unwrap();
        assert!(hist.contains("about:clear-history") && hist.contains("about:remove-history?u="));
        assert!(about_page("about:nope", &lib).is_none());
        assert_eq!(about_page("about:blank", &lib).unwrap().0, "New Tab");
    }

    #[test]
    fn about_actions_parse_and_choose_the_page_that_follows() {
        let a = action("about:remove-bookmark?u=https%3A%2F%2Fa.test%2F").unwrap();
        assert_eq!(a, Action::RemoveBookmark("https://a.test/".into()));
        assert_eq!(a.next(), BOOKMARKS);
        assert_eq!(action("about:clear-history"), Some(Action::ClearHistory));
        assert_eq!(action("about:home"), None);
        assert_eq!(Action::ClearHistory.next(), HISTORY);
    }

    #[test]
    fn built_in_page_html_parses_into_links() {
        use crate::shell::browser::html;
        let mut lib = Library::memory();
        lib.toggle_bookmark("A", "https://a.test/");
        let (_, h) = about_page(BOOKMARKS, &lib).unwrap();
        let dom = html::parse(&h);
        let mut links = vec![];
        html::find_all(&dom, "a", &mut links);
        assert!(links.iter().any(|l| l.attr("href") == Some("https://a.test/")));
    }
}
