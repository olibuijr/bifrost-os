//! Subtitle tracks for the video player: SubRip (`.srt`) and WebVTT (`.vtt`) cues parsed from text, and the cue on
//! screen at a playback time. Pure: nothing here reads a file. `sidecar_candidates` only names the files next to a
//! movie, and the owner reads the first one that exists.
//!
//! Both formats share the block layout (cues separated by blank lines, a timing line with `-->`, then the text), so one
//! parser reads both. A cue identifier or SubRip index before the timing line is ignored, as are the WEBVTT header,
//! NOTE and STYLE blocks (they have no timing line). Inline markup such as `<i>` or `<c.yellow>` is removed.

use std::path::{Path, PathBuf};

/// One line of dialogue on screen from `start_ms` (inclusive) to `end_ms` (exclusive).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cue {
    pub start_ms: u64,
    pub end_ms: u64,
    pub text: String,
}

/// The cues of one subtitle file, sorted by start time.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Track {
    cues: Vec<Cue>,
}

impl Track {
    /// Parses SubRip or WebVTT text. Cues with no text, an unreadable timing line or an end before the start are
    /// skipped. A byte order mark and CRLF line ends are accepted.
    pub fn parse(text: &str) -> Track {
        let text = text.trim_start_matches('\u{feff}').replace("\r\n", "\n").replace('\r', "\n");
        let mut cues = Vec::new();
        for block in text.split("\n\n") {
            let lines: Vec<&str> = block.lines().filter(|l| !l.trim().is_empty()).collect();
            let Some((arrow, (start_ms, end_ms))) = lines.iter().enumerate().find_map(|(i, l)| timing(l).map(|t| (i, t))) else {
                continue;
            };
            if end_ms <= start_ms {
                continue;
            }
            let body: Vec<String> = lines[arrow + 1..].iter().map(|l| strip_tags(l)).filter(|l| !l.trim().is_empty()).collect();
            if body.is_empty() {
                continue;
            }
            cues.push(Cue { start_ms, end_ms, text: body.join("\n") });
        }
        cues.sort_by_key(|c| c.start_ms);
        Track { cues }
    }

    pub fn cues(&self) -> &[Cue] {
        &self.cues
    }

    pub fn is_empty(&self) -> bool {
        self.cues.is_empty()
    }

    /// The first cue on screen at `ms` (its start at or before `ms`, its end after).
    pub fn cue_at(&self, ms: u64) -> Option<&Cue> {
        self.cues.iter().find(|c| c.start_ms <= ms && ms < c.end_ms)
    }

    /// The text on screen at `ms`, if a cue is showing.
    pub fn text_at(&self, ms: u64) -> Option<&str> {
        self.cue_at(ms).map(|c| c.text.as_str())
    }
}

/// The subtitle files that may sit next to a movie, in the order the owner should try them: the same name with the
/// `.srt` extension, then `.vtt`.
pub fn sidecar_candidates(movie: &Path) -> Vec<PathBuf> {
    ["srt", "vtt"].iter().map(|ext| movie.with_extension(ext)).collect()
}

/// The `start --> end` line of a cue, as milliseconds. The end may carry WebVTT settings after it (`line:90%`), which are
/// dropped.
fn timing(line: &str) -> Option<(u64, u64)> {
    let (start, rest) = line.split_once("-->")?;
    let end = rest.split_whitespace().next()?;
    Some((parse_time(start.trim())?, parse_time(end)?))
}

/// A SubRip or WebVTT timestamp: `hh:mm:ss,mmm`, `hh:mm:ss.mmm`, or `mm:ss.mmm` (the hours are optional). The fraction
/// may have one to three digits; a missing fraction is zero.
pub fn parse_time(s: &str) -> Option<u64> {
    let (clock, frac) = match s.rsplit_once(['.', ',']) {
        Some((c, f)) => (c, f),
        None => (s, ""),
    };
    if frac.is_empty() && s.contains(['.', ',']) {
        return None;
    }
    let parts: Vec<&str> = clock.split(':').collect();
    let (h, m, sec) = match parts.as_slice() {
        [m, s] => (0, *m, *s),
        [h, m, s] => (h.parse::<u64>().ok()?, *m, *s),
        _ => return None,
    };
    let (m, sec): (u64, u64) = (m.parse().ok()?, sec.parse().ok()?);
    if m >= 60 || sec >= 60 {
        return None;
    }
    let ms = match frac.len() {
        0 => 0,
        1..=3 if frac.bytes().all(|b| b.is_ascii_digit()) => frac.parse::<u64>().ok()? * 10u64.pow(3 - frac.len() as u32),
        _ => return None,
    };
    Some(((h * 60 + m) * 60 + sec) * 1000 + ms)
}

/// The line without markup: anything between `<` and `>` is dropped.
fn strip_tags(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut in_tag = false;
    for ch in line.chars() {
        match ch {
            '<' => in_tag = true,
            '>' if in_tag => in_tag = false,
            _ if !in_tag => out.push(ch),
            _ => {}
        }
    }
    out.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SRT: &str = "1\n00:00:01,000 --> 00:00:03,500\nHello there.\n\n2\n00:00:04,000 --> 00:00:05,000\nLine one\nLine two\n";

    #[test]
    fn parses_srt_cues_in_order() {
        let t = Track::parse(SRT);
        assert_eq!(t.cues().len(), 2);
        assert_eq!(t.cues()[0], Cue { start_ms: 1000, end_ms: 3500, text: "Hello there.".into() });
        assert_eq!(t.cues()[1].text, "Line one\nLine two");
    }

    #[test]
    fn parses_webvtt_with_header_identifiers_and_settings() {
        let vtt = "\u{feff}WEBVTT\n\nNOTE a comment\nover two lines\n\nintro\n00:02.000 --> 00:04.250 line:90% align:start\n<i>Quiet</i> please\n\n01:00:00.000 --> 01:00:02.000\nLate\n";
        let t = Track::parse(vtt);
        assert_eq!(t.cues().len(), 2);
        assert_eq!(t.cues()[0], Cue { start_ms: 2000, end_ms: 4250, text: "Quiet please".into() });
        assert_eq!(t.cues()[1].start_ms, 3_600_000);
    }

    #[test]
    fn accepts_crlf_and_skips_broken_cues() {
        let text = "1\r\n00:00:01,000 --> 00:00:02,000\r\nOk\r\n\r\n2\r\n00:00:05,000 --> 00:00:04,000\r\nBackwards\r\n\r\n3\r\nnot a time --> later\r\nBad\r\n\r\n4\r\n00:00:06,000 --> 00:00:07,000\r\n\r\n";
        let t = Track::parse(text);
        assert_eq!(t.cues().len(), 1);
        assert_eq!(t.cues()[0].text, "Ok");
    }

    #[test]
    fn cue_lookup_includes_start_and_excludes_end() {
        let t = Track::parse(SRT);
        assert_eq!(t.text_at(999), None);
        assert_eq!(t.text_at(1000), Some("Hello there."));
        assert_eq!(t.text_at(3499), Some("Hello there."));
        assert_eq!(t.text_at(3500), None);
        assert_eq!(t.text_at(4500), Some("Line one\nLine two"));
        assert!(Track::parse("").is_empty());
    }

    #[test]
    fn timestamps_take_one_to_three_fraction_digits_and_optional_hours() {
        assert_eq!(parse_time("00:00:01,5"), Some(1500));
        assert_eq!(parse_time("01:02:03.004"), Some(3_723_004));
        assert_eq!(parse_time("02:30"), Some(150_000));
        assert_eq!(parse_time("00:00:01"), Some(1000));
        assert_eq!(parse_time("00:00:01.1234"), None);
        assert_eq!(parse_time("00:61:00.000"), None);
        assert_eq!(parse_time("00:00:01."), None);
        assert_eq!(parse_time("soon"), None);
    }

    #[test]
    fn sidecars_are_named_after_the_movie() {
        let c = sidecar_candidates(Path::new("/movies/Trip.mp4"));
        assert_eq!(c, vec![PathBuf::from("/movies/Trip.srt"), PathBuf::from("/movies/Trip.vtt")]);
    }
}
