//! Show search and the Top Charts against two open, keyless podcast directories: fyyd (searched by title) and
//! gpodder.net (searched across title, author and description, and the source of the Top Charts). Pure functions: the
//! app fetches each address through its `Fetch`, parses each answer with the parser its `Source` names, then merges
//! and ranks the hits with `merge`. No account, key or identifier is sent; the only thing either directory learns is
//! the term typed.

use bifrost_core::json;

/// One show a directory found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hit {
    pub title: String,
    pub author: String,
    /// The feed address that Follow subscribes to.
    pub feed: String,
}

/// Which directory an answer came from, so it is read with the right parser.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    Fyyd,
    Gpodder,
}

/// How many shows the Top Charts list.
pub const CHART_COUNT: usize = 50;

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

/// The fyyd title search for `term`, or None when the term is blank.
pub fn url(term: &str) -> Option<String> {
    let term = term.trim();
    (!term.is_empty()).then(|| format!("https://api.fyyd.de/0.2/search/podcast?title={}&count=25", percent_encode(term)))
}

/// Every request a search for `term` makes, with the directory each answer is read as; empty for a blank term. The
/// fyyd search covers titles only, so gpodder.net's search adds the shows a term names by author or publisher.
pub fn requests(term: &str) -> Vec<(Source, String)> {
    let Some(fyyd) = url(term) else { return Vec::new() };
    vec![(Source::Fyyd, fyyd), (Source::Gpodder, format!("https://gpodder.net/search.json?q={}", percent_encode(term.trim())))]
}

/// The Top Charts request: gpodder.net's most subscribed shows.
pub fn charts_url() -> String {
    format!("https://gpodder.net/toplist/{CHART_COUNT}.json")
}

/// The shows in a fyyd answer; entries without a feed address are dropped.
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

/// The shows in a gpodder.net answer (a search or the toplist: an array of podcasts); entries without a feed address
/// are dropped. Authors often carry a mail address in parentheses, which is cut off.
pub fn parse_gpodder(body: &str) -> Vec<Hit> {
    let Ok(doc) = json::parse(body) else { return Vec::new() };
    let Some(list) = doc.as_array() else { return Vec::new() };
    list.iter()
        .filter_map(|item| {
            let feed = item.str_field("url").filter(|f| !f.is_empty())?;
            let author = item.str_field("author").unwrap_or_default();
            let author = author.split(" (").next().unwrap_or_default().trim().to_string();
            Some(Hit { title: item.str_field("title").map(|t| t.trim().to_string()).filter(|t| !t.is_empty()).unwrap_or_else(|| feed.clone()), author, feed })
        })
        .collect()
}

/// Reads an answer from `source`.
pub fn parse_from(source: Source, body: &str) -> Vec<Hit> {
    match source {
        Source::Fyyd => parse(body),
        Source::Gpodder => parse_gpodder(body),
    }
}

/// The words of `s`, lower-cased, for matching.
fn words(s: &str) -> Vec<String> {
    s.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).map(str::to_lowercase).collect()
}

/// How well `hit` answers `term`: 0 is an exact title, then a title with every word, an author or publisher with
/// every word, a title or author with some of the words; None when no word of the term is in either.
fn relevance(term: &[String], hit: &Hit) -> Option<u8> {
    let title = words(&hit.title);
    let author = words(&hit.author);
    let all = |field: &[String]| term.iter().all(|w| field.contains(w));
    let some = |field: &[String]| term.iter().any(|w| field.contains(w));
    if title == term {
        Some(0)
    } else if all(&title) {
        Some(1)
    } else if all(&author) {
        Some(2)
    } else if some(&title) || some(&author) {
        Some(3)
    } else {
        None
    }
}

/// The answers to a search for `term`, merged: one hit per feed, those that name the term in their title or author
/// first (best match first, each directory's own order kept within a rank), and hits that match no word of the term
/// dropped. When nothing matches a word (a misspelt term), the directories' own answers are kept in their order.
pub fn merge(term: &str, answers: Vec<Vec<Hit>>) -> Vec<Hit> {
    let mut seen = std::collections::HashSet::new();
    let unique: Vec<Hit> = answers.into_iter().flatten().filter(|h| seen.insert(h.feed.trim_end_matches('/').to_lowercase())).collect();
    let term = words(term);
    let mut ranked: Vec<(u8, usize, Hit)> = unique.iter().enumerate().filter_map(|(i, h)| relevance(&term, h).map(|r| (r, i, h.clone()))).collect();
    if ranked.is_empty() {
        return unique;
    }
    ranked.sort_by_key(|(r, i, _)| (*r, *i));
    ranked.into_iter().map(|(_, _, h)| h).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hit(title: &str, author: &str, feed: &str) -> Hit {
        Hit { title: title.into(), author: author.into(), feed: feed.into() }
    }

    #[test]
    fn terms_are_percent_encoded() {
        assert_eq!(percent_encode("a b&c/ö"), "a%20b%26c%2F%C3%B6");
        assert_eq!(percent_encode("Az-9_.~"), "Az-9_.~");
        assert_eq!(url("  "), None);
        assert_eq!(url("rust news").unwrap(), "https://api.fyyd.de/0.2/search/podcast?title=rust%20news&count=25");
    }

    #[test]
    fn a_search_asks_both_directories_and_a_blank_term_asks_none() {
        assert!(requests("   ").is_empty());
        let asked = requests(" Relay FM ");
        assert_eq!(asked.len(), 2);
        assert_eq!(asked[0], (Source::Fyyd, "https://api.fyyd.de/0.2/search/podcast?title=Relay%20FM&count=25".to_string()));
        assert_eq!(asked[1], (Source::Gpodder, "https://gpodder.net/search.json?q=Relay%20FM".to_string()));
        assert!(asked.iter().all(|(_, u)| u.starts_with("https://") && !u.contains("key")), "keyless, over TLS");
        assert_eq!(charts_url(), "https://gpodder.net/toplist/50.json");
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

    #[test]
    fn gpodder_answers_read_the_feed_title_and_author_without_the_mail_address() {
        let body = r#"[{"url":"https://www.relay.fm/master/feed","title":"Relay FM Master Feed","author":"Relay FM","subscribers":9},
            {"url":"http://leo.am/podcasts/twit","title":"This Week in Tech (Audio)","author":"Leo Laporte (distro@twit.tv)"},
            {"title":"No feed"}]"#;
        let hits = parse_gpodder(body);
        assert_eq!(hits, [hit("Relay FM Master Feed", "Relay FM", "https://www.relay.fm/master/feed"), hit("This Week in Tech (Audio)", "Leo Laporte", "http://leo.am/podcasts/twit")]);
        assert!(parse_gpodder("{}").is_empty() && parse_gpodder("nope").is_empty());
        assert_eq!(parse_from(Source::Gpodder, body).len(), 2);
        assert!(parse_from(Source::Fyyd, body).is_empty(), "each answer is read by its own parser");
    }

    #[test]
    fn a_publisher_name_finds_its_shows_and_unrelated_hits_are_dropped() {
        // fyyd's title search answers "Relay FM" with an unrelated show; gpodder.net finds the network's shows by author
        let fyyd = vec![hit("Ruhe will Leben", "", "https://anchor.fm/s/9b63650/podcast/rss")];
        let gpodder = vec![hit("B-Sides", "Relay FM", "https://www.relay.fm/b-sides/feed"), hit("Relay FM Master Feed", "Relay FM", "https://www.relay.fm/master/feed")];
        let merged = merge("Relay FM", vec![fyyd, gpodder]);
        let titles: Vec<&str> = merged.iter().map(|h| h.title.as_str()).collect();
        assert_eq!(titles, ["Relay FM Master Feed", "B-Sides"], "the title match first, then the author match");
    }

    #[test]
    fn an_exact_title_beats_more_popular_partial_matches_and_a_feed_is_listed_once() {
        let gpodder = vec![
            hit("This Week in Tech (Audio)", "Leo Laporte", "http://leo.am/podcasts/twit"),
            hit("What The Tech Podcast", "", "https://wtt.test/rss"),
            hit("Accidental Tech Podcast", "Marco Arment", "https://atp.fm/rss"),
        ];
        let fyyd = vec![hit("Accidental Tech Podcast", "Marco Arment", "https://atp.fm/rss/")];
        let merged = merge("accidental tech podcast", vec![fyyd, gpodder]);
        assert_eq!(merged[0].title, "Accidental Tech Podcast");
        assert_eq!(merged.iter().filter(|h| h.title == "Accidental Tech Podcast").count(), 1, "the same feed once");
        assert_eq!(merged.len(), 3, "partial matches stay, after the exact one");
    }

    #[test]
    fn a_term_nothing_matches_keeps_the_directories_answers() {
        let answers = vec![vec![hit("Alpha", "", "https://a.test")], vec![hit("Beta", "", "https://b.test")]];
        let merged = merge("zzqx", answers);
        assert_eq!(merged.iter().map(|h| h.title.as_str()).collect::<Vec<_>>(), ["Alpha", "Beta"]);
    }
}
