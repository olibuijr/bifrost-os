//! Lyrics for the player sheet: reads time-synced `.lrc` text (and plain text), finds the line to highlight at a
//! playback position, and finds the lyrics file that sits beside a song. Pure std.
#![allow(dead_code)] // the Lyrics panel in the player sheet wires this in next

use std::path::{Path, PathBuf};

/// One lyric line. `at_ms` is None for plain, unsynced text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Line {
    pub at_ms: Option<u64>,
    pub text: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Lyrics {
    pub lines: Vec<Line>,
    /// from `[ti:]`, `[ar:]`, `[al:]` tags
    pub title: String,
    pub artist: String,
    pub album: String,
}

/// Parses `[mm:ss.xx]` (or `[mm:ss]`, `[mm:ss.xxx]`) into milliseconds.
fn stamp(tag: &str) -> Option<u64> {
    let (m, rest) = tag.split_once(':')?;
    let minutes: u64 = m.trim().parse().ok()?;
    let (s, frac) = match rest.split_once(['.', ':']) {
        Some((s, f)) => (s, f),
        None => (rest, ""),
    };
    let seconds: u64 = s.trim().parse().ok()?;
    if seconds >= 60 {
        return None;
    }
    let ms = if frac.is_empty() {
        0
    } else {
        let digits: String = frac.chars().take(3).collect();
        let v: u64 = digits.parse().ok()?;
        v * 10u64.pow(3 - digits.len() as u32)
    };
    Some(minutes * 60_000 + seconds * 1000 + ms)
}

/// Reads lyrics text. A line may carry several time tags (`[00:10.00][01:20.00]chorus`); tags are expanded into one
/// line each and sorted by time. Text with no time tags at all becomes plain lines.
pub fn parse(text: &str) -> Lyrics {
    let mut out = Lyrics::default();
    let mut synced: Vec<Line> = vec![];
    let mut plain: Vec<Line> = vec![];
    for raw in text.lines() {
        let mut rest = raw.trim_start_matches('\u{feff}').trim();
        let mut stamps: Vec<u64> = vec![];
        let mut meta = false;
        while rest.starts_with('[') {
            let Some(end) = rest.find(']') else { break };
            let tag = &rest[1..end];
            if let Some(ms) = stamp(tag) {
                stamps.push(ms);
            } else if let Some((k, v)) = tag.split_once(':') {
                match k.trim() {
                    "ti" => out.title = v.trim().to_string(),
                    "ar" => out.artist = v.trim().to_string(),
                    "al" => out.album = v.trim().to_string(),
                    _ => {}
                }
                meta = true;
            }
            rest = rest[end + 1..].trim_start();
        }
        if meta && stamps.is_empty() {
            continue;
        }
        if stamps.is_empty() {
            plain.push(Line { at_ms: None, text: rest.to_string() });
        } else {
            for ms in stamps {
                synced.push(Line { at_ms: Some(ms), text: rest.to_string() });
            }
        }
    }
    synced.sort_by_key(|l| l.at_ms);
    out.lines = if synced.is_empty() { plain } else { synced };
    out
}

impl Lyrics {
    pub fn is_synced(&self) -> bool {
        self.lines.iter().any(|l| l.at_ms.is_some())
    }

    /// The index of the line to highlight at `pos_ms`: the last line whose time has passed. None before the first
    /// line, and always None for unsynced text.
    pub fn current(&self, pos_ms: u64) -> Option<usize> {
        if !self.is_synced() {
            return None;
        }
        let n = self.lines.partition_point(|l| l.at_ms.is_some_and(|t| t <= pos_ms));
        n.checked_sub(1)
    }

    /// The time to seek to when a line is tapped.
    pub fn seek_for(&self, index: usize) -> Option<u64> {
        self.lines.get(index).and_then(|l| l.at_ms)
    }
}

/// Where lyrics for `song` may live: the same name with `.lrc`, then `.txt`, in the song's folder.
pub fn candidates(song: &Path) -> Vec<PathBuf> {
    ["lrc", "txt"].iter().map(|e| song.with_extension(e)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "[ti:Song]\n[ar:Band]\n[al:Disc]\n[00:12.50]first\n[00:05.00]zero\n[01:00.00][00:30.25]chorus\n\n";

    #[test]
    fn stamps() {
        assert_eq!(stamp("00:12.50"), Some(12_500));
        assert_eq!(stamp("01:02"), Some(62_000));
        assert_eq!(stamp("00:01.5"), Some(1_500));
        assert_eq!(stamp("00:01.123"), Some(1_123));
        assert_eq!(stamp("00:61.00"), None);
        assert_eq!(stamp("ti:Song"), None);
    }

    #[test]
    fn synced_text_is_sorted_and_tags_expand() {
        let l = parse(SAMPLE);
        assert_eq!((l.title.as_str(), l.artist.as_str(), l.album.as_str()), ("Song", "Band", "Disc"));
        let times: Vec<Option<u64>> = l.lines.iter().map(|x| x.at_ms).collect();
        assert_eq!(times, vec![Some(5_000), Some(12_500), Some(30_250), Some(60_000), None].into_iter().take(4).collect::<Vec<_>>());
        assert_eq!(l.lines[2].text, "chorus");
        assert_eq!(l.lines[3].text, "chorus");
        assert!(l.is_synced());
    }

    #[test]
    fn current_line_follows_the_position() {
        let l = parse(SAMPLE);
        assert_eq!(l.current(0), None);
        assert_eq!(l.current(5_000), Some(0));
        assert_eq!(l.current(20_000), Some(1));
        assert_eq!(l.current(999_999), Some(3));
        assert_eq!(l.seek_for(1), Some(12_500));
    }

    #[test]
    fn plain_text_has_no_highlight() {
        let l = parse("one\ntwo\n");
        assert!(!l.is_synced());
        assert_eq!(l.lines.len(), 2);
        assert_eq!(l.current(5_000), None);
        assert_eq!(l.seek_for(0), None);
    }

    #[test]
    fn candidate_files() {
        assert_eq!(
            candidates(Path::new("/m/a/b.mp3")),
            vec![PathBuf::from("/m/a/b.lrc"), PathBuf::from("/m/a/b.txt")]
        );
    }
}
