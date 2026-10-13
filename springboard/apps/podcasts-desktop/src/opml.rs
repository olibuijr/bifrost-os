//! OPML subscription lists: the feeds a user follows, as a file other podcast apps read and write.

use crate::feed::attribute;

/// The feeds (address, title) in an OPML document, each address once, in file order.
pub fn parse(text: &str) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    let mut rest = text;
    while let Some(i) = rest.find("<outline") {
        let Some(end) = rest[i..].find('>') else { break };
        let tag = &rest[i..=i + end];
        rest = &rest[i + end + 1..];
        // `attribute` looks for "<outline " with a space: a tag like `<outline/>` has no attributes
        let Some(url) = ["xmlUrl", "xmlurl", "xmlURL"].iter().find_map(|a| attribute(tag, "outline", a)).filter(|u| !u.is_empty()) else { continue };
        if out.iter().any(|(u, _)| *u == url) {
            continue;
        }
        let title = ["text", "title"].iter().find_map(|a| attribute(tag, "outline", a)).filter(|t| !t.is_empty()).unwrap_or_else(|| url.clone());
        out.push((url, title));
    }
    out
}

/// The feeds of an imported list that are not followed yet: addresses normalised (a bare host gets a scheme), titles
/// kept, repeats and entries that are not addresses dropped.
pub fn new_feeds(parsed: &[(String, String)], known: &[&str]) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    for (url, title) in parsed {
        let Some(url) = crate::feed::normalize(url) else { continue };
        if known.contains(&url.as_str()) || out.iter().any(|(u, _)| *u == url) {
            continue;
        }
        let title = if title.is_empty() { url.clone() } else { title.clone() };
        out.push((url, title));
    }
    out
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// An OPML 2.0 document listing `feeds` (address, title).
pub fn write(feeds: &[(String, String)]) -> String {
    let mut out = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<opml version=\"2.0\">\n<head><title>Podcast Subscriptions</title></head>\n<body>\n");
    for (url, title) in feeds {
        out.push_str(&format!("<outline type=\"rss\" text=\"{0}\" title=\"{0}\" xmlUrl=\"{1}\"/>\n", escape(title), escape(url)));
    }
    out.push_str("</body>\n</opml>\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn feeds_round_trip_through_a_document() {
        let feeds = vec![("https://a.test/rss?x=1&y=2".to_string(), "A & B".to_string()), ("https://b.test/rss".to_string(), "B".to_string())];
        let text = write(&feeds);
        assert!(text.contains("&amp;y=2"));
        assert_eq!(parse(&text), feeds);
    }

    #[test]
    fn only_new_feeds_are_imported() {
        let parsed = vec![
            ("https://a.test/rss".to_string(), "A".to_string()),
            ("b.test/rss".to_string(), String::new()),
            ("https://b.test/rss".to_string(), "B again".to_string()),
            ("not a feed".to_string(), "Bad".to_string()),
            ("https://c.test/rss".to_string(), "C".to_string()),
        ];
        let got = new_feeds(&parsed, &["https://c.test/rss"]);
        assert_eq!(got, [("https://a.test/rss".to_string(), "A".to_string()), ("https://b.test/rss".to_string(), "https://b.test/rss".to_string())]);
    }

    #[test]
    fn nested_folders_duplicates_and_missing_addresses_are_handled() {
        let text = r#"<opml><body><outline text="Folder"><outline text="One" xmlUrl="https://1.test/f"/><outline text="Dup" xmlUrl="https://1.test/f"/></outline>
<outline text="Two" xmlurl='https://2.test/f'/><outline title="Only title" xmlUrl="https://3.test/f"/><outline text="No feed"/></body></opml>"#;
        let got = parse(text);
        assert_eq!(got, [("https://1.test/f".into(), "One".into()), ("https://2.test/f".into(), "Two".into()), ("https://3.test/f".into(), "Only title".into())]);
    }
}
