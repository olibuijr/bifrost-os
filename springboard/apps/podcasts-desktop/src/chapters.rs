//! Chapters kept in a separate JSON file (the Podcasting 2.0 `podcast:chapters` link). Pure functions: the file is
//! fetched by the app.

use crate::feed::{Chapter, Episode};
use bifrost_core::json::{self, Json};

/// How many fetched chapter lists stay in memory.
pub const KEPT: usize = 16;

/// Reads a chapters file: `{"chapters":[{"startTime":12.5,"title":"Intro"}]}`. Times are seconds. Chapters with
/// `"toc": false` are not meant for a chapter list and are left out, as are entries without a start time. The result is
/// in start order; an unreadable file gives an empty list.
pub fn parse(raw: &str) -> Vec<Chapter> {
    let Ok(doc) = json::parse(raw.trim_start_matches('\u{feff}')) else { return Vec::new() };
    let mut out: Vec<Chapter> = doc
        .get("chapters")
        .and_then(Json::as_array)
        .unwrap_or_default()
        .iter()
        .filter(|c| c.get("toc") != Some(&Json::Bool(false)))
        .filter_map(|c| {
            let seconds = c.get("startTime").and_then(Json::number).filter(|s| s.is_finite() && *s >= 0.0)?;
            Some(Chapter { start_ms: (seconds * 1000.0).round() as u64, title: c.str_field("title").unwrap_or_default() })
        })
        .collect();
    out.sort_by_key(|c| c.start_ms);
    out
}

/// Whether the chapters file of `episode` should be fetched now: the feed names one, the feed itself lists no chapters
/// and it has not been asked for yet (`asked` holds the episode addresses already requested).
pub fn wanted(episode: &Episode, asked: &[String]) -> bool {
    !episode.chapters_url.is_empty() && episode.chapters.is_empty() && !asked.contains(&episode.url)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chapters_are_read_in_start_order_with_seconds_as_milliseconds() {
        let raw = r#"{"version":"1.2.0","chapters":[{"startTime":90,"title":"Later"},{"startTime":0,"title":"Intro"},{"startTime":12.5,"title":"Mid"}]}"#;
        assert_eq!(
            parse(raw),
            [
                Chapter { start_ms: 0, title: "Intro".into() },
                Chapter { start_ms: 12_500, title: "Mid".into() },
                Chapter { start_ms: 90_000, title: "Later".into() },
            ]
        );
    }

    #[test]
    fn hidden_and_untimed_entries_are_skipped() {
        let raw = r#"{"chapters":[{"startTime":0,"title":"Shown"},{"startTime":5,"title":"Hidden","toc":false},{"title":"No time"},{"startTime":-3,"title":"Negative"},{"startTime":9}]}"#;
        assert_eq!(parse(raw), [Chapter { start_ms: 0, title: "Shown".into() }, Chapter { start_ms: 9_000, title: String::new() }]);
    }

    #[test]
    fn bad_input_gives_no_chapters() {
        assert!(parse("").is_empty());
        assert!(parse("not json").is_empty());
        assert!(parse(r#"{"chapters":"x"}"#).is_empty());
        assert!(parse(r#"{"other":[]}"#).is_empty());
        assert_eq!(parse("\u{feff}{\"chapters\":[{\"startTime\":1,\"title\":\"A\"}]}").len(), 1);
    }

    #[test]
    fn only_episodes_with_a_link_and_no_inline_chapters_are_wanted() {
        let mut ep = Episode { url: "https://x.test/1.mp3".into(), ..Episode::default() };
        assert!(!wanted(&ep, &[]));
        ep.chapters_url = "https://x.test/c.json".into();
        assert!(wanted(&ep, &[]));
        assert!(!wanted(&ep, &["https://x.test/1.mp3".to_string()]));
        ep.chapters.push(Chapter { start_ms: 0, title: "A".into() });
        assert!(!wanted(&ep, &[]));
    }
}
