//! Show search against an open podcast directory (fyyd): the request address and the answer. Pure functions: the
//! app fetches the address through its `Fetch` and applies the hits.

use bifrost_core::json;

/// One show the directory found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hit {
    pub title: String,
    pub author: String,
    /// The feed address that Follow subscribes to.
    pub feed: String,
}

/// Percent-encodes `s` for a query value: unreserved characters stay, everything else is %XX per UTF-8 byte.
pub fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// The search request for `term`, or None when the term is blank.
pub fn url(term: &str) -> Option<String> {
    let term = term.trim();
    (!term.is_empty()).then(|| format!("https://api.fyyd.de/0.2/search/podcast?title={}&count=25", percent_encode(term)))
}

/// The shows in an answer; entries without a feed address are dropped.
pub fn parse(body: &str) -> Vec<Hit> {
    let Ok(doc) = json::parse(body) else { return Vec::new() };
    let Some(list) = doc.get("data").and_then(|d| d.as_array()) else { return Vec::new() };
    list.iter()
        .filter_map(|item| {
            let feed = item.str_field("xmlURL").filter(|f| !f.is_empty())?;
            Some(Hit { title: item.str_field("title").unwrap_or_else(|| feed.clone()), author: item.str_field("author").unwrap_or_default(), feed })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terms_are_percent_encoded() {
        assert_eq!(percent_encode("a b&c/ö"), "a%20b%26c%2F%C3%B6");
        assert_eq!(percent_encode("Az-9_.~"), "Az-9_.~");
        assert_eq!(url("  "), None);
        assert_eq!(url("rust news").unwrap(), "https://api.fyyd.de/0.2/search/podcast?title=rust%20news&count=25");
    }

    #[test]
    fn hits_need_a_feed_address() {
        let body = r#"{"status":1,"data":[{"title":"A","author":"Ann","xmlURL":"https://a.test/rss"},{"title":"No feed"},{"xmlURL":"https://b.test/rss"}]}"#;
        let hits = parse(body);
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0], Hit { title: "A".into(), author: "Ann".into(), feed: "https://a.test/rss".into() });
        assert_eq!(hits[1].title, "https://b.test/rss");
        assert!(parse("not json").is_empty());
        assert!(parse("{}").is_empty());
    }
}
