//! Subtitle tracks for the video player: SubRip (`.srt`) and WebVTT (`.vtt`) cues parsed from text, and the cue on
//! screen at a playback time. Pure: nothing here reads a file. `sidecar_candidates` only names the files next to a
//! movie, and `sidecars_in` picks the subtitle files (with a label each) out of the names in the movie's folder; the
//! owner reads the folder and the files.
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

/// A subtitle file in the movie's folder and the label the subtitle picker shows for it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sidecar {
    pub file_name: String,
    pub label: String,
}

/// The language part of `file_name` when it is a subtitle file of `movie`: `Some(None)` for `name.srt` or `name.vtt`,
/// `Some(Some("en"))` for `name.en.srt`, and `None` for any other file. The extension may be in any case. A language is
/// a code of 2 to 8 letters, digits, `-` or `_` (`en`, `pt-BR`); the movie's own extension (`name.mp4.srt`) is not one.
fn sidecar_language<'a>(movie: &Path, file_name: &'a str) -> Option<Option<&'a str>> {
    let stem = movie.file_stem()?.to_str()?;
    let rest = file_name.strip_prefix(stem)?.strip_prefix('.')?;
    let (language, ext) = match rest.rsplit_once('.') {
        Some((language, ext)) => (Some(language), ext),
        None => (None, rest),
    };
    if !ext.eq_ignore_ascii_case("srt") && !ext.eq_ignore_ascii_case("vtt") {
        return None;
    }
    let own_ext = movie.extension().and_then(|e| e.to_str()).unwrap_or("");
    match language {
        None => Some(None),
        Some(l) if l.eq_ignore_ascii_case(own_ext) => Some(None),
        Some(l) if (2..=8).contains(&l.len()) && l.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') => Some(Some(l)),
        Some(_) => None,
    }
}

/// The subtitle files among `names`, the file names in the movie's folder, sorted by name. The label is the language code
/// (`en`), or "Subtitles" for a file with no language; two files that would share a label get their extension added
/// ("Subtitles (SRT)" and "Subtitles (VTT)"). The owner reads the folder and the files.
pub fn sidecars_in(movie: &Path, names: &[String]) -> Vec<Sidecar> {
    let mut found: Vec<(String, String)> = names
        .iter()
        .filter_map(|n| sidecar_language(movie, n).map(|l| (n.clone(), l.unwrap_or("Subtitles").to_string())))
        .collect();
    found.sort();
    let labels: Vec<String> = found.iter().map(|(_, l)| l.clone()).collect();
    found
        .into_iter()
        .map(|(file_name, label)| {
            let label = if labels.iter().filter(|l| **l == label).count() > 1 {
                let ext = Path::new(&file_name).extension().and_then(|e| e.to_str()).unwrap_or("").to_uppercase();
                format!("{label} ({ext})")
            } else {
                label
            };
            Sidecar { file_name, label }
        })
        .collect()
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

/// The subtitle picker of the video player: "Off" first, then the subtitle files the owner found next to the movie
/// (pass only the files that exist, from `sidecar_candidates`). Pure: choosing a row only records it, and the owner
/// loads the chosen file with `Track::parse`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Picker {
    tracks: Vec<PathBuf>,
    /// the index into `tracks` of the chosen file; `None` is Off
    selected: Option<usize>,
}

impl Picker {
    /// A picker over these subtitle files, with Off chosen.
    pub fn new(tracks: Vec<PathBuf>) -> Picker {
        Picker { tracks, selected: None }
    }

    /// The rows in picker order: the label of each and whether it is the chosen one. Off is the first row.
    pub fn rows(&self) -> Vec<(String, bool)> {
        let mut rows = vec![("Off".to_string(), self.selected.is_none())];
        for (i, path) in self.tracks.iter().enumerate() {
            let label = path.file_stem().and_then(|s| s.to_str()).unwrap_or("Subtitles").to_string();
            rows.push((label, self.selected == Some(i)));
        }
        rows
    }

    /// Chooses row `row` of `rows()` (0 is Off). Returns false when the row does not exist.
    pub fn choose(&mut self, row: usize) -> bool {
        if row == 0 {
            self.selected = None;
            true
        } else if row <= self.tracks.len() {
            self.selected = Some(row - 1);
            true
        } else {
            false
        }
    }

    /// The subtitle file chosen, or `None` for Off.
    pub fn selected(&self) -> Option<&Path> {
        self.selected.and_then(|i| self.tracks.get(i)).map(PathBuf::as_path)
    }

    /// Moves the choice to the next row, wrapping from the last file back to Off.
    pub fn cycle(&mut self) {
        let next = match self.selected {
            None if self.tracks.is_empty() => None,
            None => Some(0),
            Some(i) if i + 1 < self.tracks.len() => Some(i + 1),
            Some(_) => None,
        };
        self.selected = next;
    }
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

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|n| n.to_string()).collect()
    }

    #[test]
    fn language_sidecars_are_found_by_name_and_labelled() {
        let movie = Path::new("/movies/clip.mp4");
        let found = sidecars_in(movie, &names(&["clip.mp4", "clip.is.srt", "clip.en.VTT", "other.srt", "clipboard.srt", "clip.txt"]));
        assert_eq!(
            found,
            vec![
                Sidecar { file_name: "clip.en.VTT".into(), label: "en".into() },
                Sidecar { file_name: "clip.is.srt".into(), label: "is".into() },
            ]
        );
    }

    #[test]
    fn a_file_without_a_language_is_just_subtitles_and_two_of_them_get_their_extension() {
        let movie = Path::new("clip.mp4");
        let one = sidecars_in(movie, &names(&["clip.srt"]));
        assert_eq!(one, vec![Sidecar { file_name: "clip.srt".into(), label: "Subtitles".into() }]);
        let two = sidecars_in(movie, &names(&["clip.vtt", "clip.srt"]));
        let labels: Vec<&str> = two.iter().map(|s| s.label.as_str()).collect();
        assert_eq!(labels, vec!["Subtitles (SRT)", "Subtitles (VTT)"]);
    }

    #[test]
    fn the_movies_own_extension_and_long_words_are_not_languages() {
        let movie = Path::new("/m/clip.mp4");
        // name.mp4.srt is the movie's own subtitle file; a word longer than a language code is not a sidecar
        let found = sidecars_in(movie, &names(&["clip.mp4.srt", "clip.toolonglang.srt", "clip.x.srt"]));
        assert_eq!(found, vec![Sidecar { file_name: "clip.mp4.srt".into(), label: "Subtitles".into() }]);
    }

    #[test]
    fn the_picker_starts_off_and_lists_each_file_by_its_name() {
        let p = Picker::new(vec![PathBuf::from("/movies/Trip.srt"), PathBuf::from("/movies/Trip.vtt")]);
        assert_eq!(p.rows(), vec![("Off".to_string(), true), ("Trip".to_string(), false), ("Trip".to_string(), false)]);
        assert_eq!(p.selected(), None);
    }

    #[test]
    fn choosing_a_row_selects_that_file_and_row_zero_is_off() {
        let mut p = Picker::new(vec![PathBuf::from("/movies/a.srt"), PathBuf::from("/movies/b.vtt")]);
        assert!(p.choose(2));
        assert_eq!(p.selected(), Some(Path::new("/movies/b.vtt")));
        assert_eq!(p.rows()[2], ("b".to_string(), true));
        assert!(p.choose(0));
        assert_eq!(p.selected(), None);
        assert!(!p.choose(3), "row 3 does not exist");
        assert_eq!(p.selected(), None);
    }

    #[test]
    fn cycling_goes_off_then_each_file_then_off_again() {
        let mut p = Picker::new(vec![PathBuf::from("/movies/a.srt"), PathBuf::from("/movies/b.vtt")]);
        p.cycle();
        assert_eq!(p.selected(), Some(Path::new("/movies/a.srt")));
        p.cycle();
        assert_eq!(p.selected(), Some(Path::new("/movies/b.vtt")));
        p.cycle();
        assert_eq!(p.selected(), None);
    }

    #[test]
    fn cycling_with_no_files_stays_off() {
        let mut p = Picker::new(Vec::new());
        p.cycle();
        assert_eq!(p.selected(), None);
        assert_eq!(p.rows(), vec![("Off".to_string(), true)]);
    }
}
