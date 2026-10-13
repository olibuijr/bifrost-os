//! Lyrics from a `.lrc` file beside the song (`Song.mp3` and `Song.lrc`). A file with time tags (`[01:23.45]text`)
//! gives synced lyrics, where the line for the playing position can be found; a file without tags gives plain lyrics
//! that are only listed. Pure parsing, no UI.

use std::path::Path;

/// A lyrics file bigger than this is not read.
const MAX_LYRICS_BYTES: u64 = 512 * 1024;

/// One line of lyrics and when it is sung.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Line {
    /// milliseconds from the start of the song; 0 in plain lyrics
    pub at_ms: u64,
    pub text: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Lyrics {
    lines: Vec<Line>,
    synced: bool,
}

/// The `.lrc` file that goes with the song at `song`.
pub fn sidecar(song: &Path) -> std::path::PathBuf {
    song.with_extension("lrc")
}

/// "mm:ss", "mm:ss.xx" or "mm:ss.xxx" as milliseconds; None for anything else (a metadata tag such as "ar:Name").
fn parse_time(tag: &str) -> Option<u64> {
    let (min, rest) = tag.split_once(':')?;
    let (sec, frac) = match rest.split_once(['.', ':']) {
        Some((s, f)) => (s, f),
        None => (rest, ""),
    };
    let digits = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_digit());
    if !digits(min) || !digits(sec) || !(frac.is_empty() || digits(frac)) {
        return None;
    }
    let frac_ms: u64 = if frac.is_empty() { 0 } else { format!("{:0<3}", &frac[..frac.len().min(3)]).parse().ok()? };
    let minutes = min.parse::<u64>().ok()?.checked_mul(60_000)?;
    let seconds = sec.parse::<u64>().ok()?.checked_mul(1000)?;
    minutes.checked_add(seconds)?.checked_add(frac_ms)
}

/// Removes enhanced-format word times (`<00:12.00>`) from a line of text.
fn strip_word_times(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut skipping = false;
    for c in text.chars() {
        match c {
            '<' => skipping = true,
            '>' if skipping => skipping = false,
            _ if !skipping => out.push(c),
            _ => {}
        }
    }
    out.trim().to_string()
}

impl Lyrics {
    /// Reads the text of a lyrics file.
    pub fn parse(text: &str) -> Lyrics {
        let mut timed: Vec<Line> = Vec::new();
        let mut plain: Vec<Line> = Vec::new();
        let mut offset: i64 = 0;
        for raw in text.trim_start_matches('\u{feff}').lines() {
            let mut rest = raw.trim();
            let mut times: Vec<u64> = Vec::new();
            let mut meta = false;
            while let Some(after) = rest.strip_prefix('[') {
                let Some((tag, tail)) = after.split_once(']') else { break };
                match parse_time(tag) {
                    Some(t) => times.push(t),
                    None => {
                        meta = true;
                        if let Some(v) = tag.strip_prefix("offset:") {
                            offset = v.trim().parse().unwrap_or(0);
                        }
                    }
                }
                rest = tail.trim_start();
            }
            if meta && times.is_empty() {
                continue;
            }
            let line = strip_word_times(rest);
            if times.is_empty() {
                if !line.is_empty() {
                    plain.push(Line { at_ms: 0, text: line });
                }
            } else {
                timed.extend(times.into_iter().map(|t| Line { at_ms: t, text: line.clone() }));
            }
        }
        if timed.is_empty() {
            return Lyrics { lines: plain, synced: false };
        }
        // a positive offset makes the lyrics come earlier
        for l in &mut timed {
            l.at_ms = (l.at_ms as i64 - offset).max(0) as u64;
        }
        timed.sort_by_key(|l| l.at_ms);
        Lyrics { lines: timed, synced: true }
    }

    /// The lyrics beside the song at `song`; None when there is no `.lrc` file, it is too big, or it holds no lines.
    pub fn load(song: &Path) -> Option<Lyrics> {
        let file = sidecar(song);
        if std::fs::metadata(&file).ok()?.len() > MAX_LYRICS_BYTES {
            return None;
        }
        let lyrics = Lyrics::parse(&std::fs::read_to_string(&file).ok()?);
        (!lyrics.lines.is_empty()).then_some(lyrics)
    }

    pub fn lines(&self) -> &[Line] {
        &self.lines
    }

    /// Whether the lines carry times, so one of them can be the current line.
    pub fn is_synced(&self) -> bool {
        self.synced
    }

    /// The line being sung at `position_ms`: the last line whose time has come. None before the first line, and for
    /// plain lyrics.
    pub fn current(&self, position_ms: f64) -> Option<usize> {
        if !self.synced {
            return None;
        }
        let at = position_ms.max(0.0) as u64;
        self.lines.partition_point(|l| l.at_ms <= at).checked_sub(1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times_read_in_each_common_spelling() {
        assert_eq!(parse_time("01:23.45"), Some(83_450));
        assert_eq!(parse_time("00:05"), Some(5_000));
        assert_eq!(parse_time("02:00.5"), Some(120_500));
        assert_eq!(parse_time("00:01.123456"), Some(1_123), "only milliseconds are kept");
        assert_eq!(parse_time("00:01:25"), Some(1_250), "a colon before the hundredths");
        assert_eq!(parse_time("ar:Abba"), None);
        assert_eq!(parse_time("offset:+500"), None);
        assert_eq!(parse_time("1:"), None);
        assert_eq!(parse_time(":30"), None);
    }

    #[test]
    fn synced_lyrics_are_ordered_and_the_current_line_follows_the_position() {
        let l = Lyrics::parse("[ar:Someone]\n[ti:Song]\n[00:10.00]second\n[00:02.50]first\n[00:20.00]third\n");
        assert!(l.is_synced());
        assert_eq!(l.lines().iter().map(|x| x.text.as_str()).collect::<Vec<_>>(), ["first", "second", "third"]);
        assert_eq!(l.current(0.0), None, "before the first line");
        assert_eq!(l.current(2_500.0), Some(0));
        assert_eq!(l.current(9_999.0), Some(0));
        assert_eq!(l.current(10_000.0), Some(1));
        assert_eq!(l.current(500_000.0), Some(2));
    }

    #[test]
    fn one_line_can_carry_several_times() {
        let l = Lyrics::parse("[00:05.00][00:35.00]chorus\n[00:20.00]verse\n");
        assert_eq!(l.lines().iter().map(|x| (x.at_ms, x.text.as_str())).collect::<Vec<_>>(), [(5_000, "chorus"), (20_000, "verse"), (35_000, "chorus")]);
    }

    #[test]
    fn a_timed_empty_line_is_kept_as_a_gap() {
        let l = Lyrics::parse("[00:01.00]hello\n[00:05.00]\n[00:09.00]again\n");
        assert_eq!(l.lines().len(), 3);
        assert_eq!(l.lines()[1].text, "");
        assert_eq!(l.current(6_000.0), Some(1));
    }

    #[test]
    fn plain_lyrics_are_listed_without_a_current_line() {
        let l = Lyrics::parse("first line\n\nsecond line\r\n[ar:Name]\n");
        assert!(!l.is_synced());
        assert_eq!(l.lines().iter().map(|x| x.text.as_str()).collect::<Vec<_>>(), ["first line", "second line"]);
        assert_eq!(l.current(10_000.0), None);
    }

    #[test]
    fn the_offset_tag_shifts_every_line() {
        let l = Lyrics::parse("[offset:+500]\n[00:01.00]a\n[00:00.20]b\n");
        assert_eq!(l.lines().iter().map(|x| x.at_ms).collect::<Vec<_>>(), [0, 500], "earlier by 500 ms, never below zero");
        let l = Lyrics::parse("[offset:-1000]\n[00:01.00]a\n");
        assert_eq!(l.lines()[0].at_ms, 2_000);
    }

    #[test]
    fn word_times_and_a_byte_order_mark_are_dropped() {
        let l = Lyrics::parse("\u{feff}[00:01.00]<00:01.00>hel<00:01.40>lo <00:01.80>world\n");
        assert_eq!(l.lines()[0].text, "hello world");
        assert_eq!(l.lines()[0].at_ms, 1_000);
    }

    #[test]
    fn nothing_readable_gives_empty_lyrics() {
        let l = Lyrics::parse("[ar:Name]\n[ti:Song]\n\n");
        assert!(l.lines().is_empty());
        assert_eq!(l.current(1_000.0), None);
    }

    #[test]
    fn the_sidecar_sits_beside_the_song_and_loads() {
        assert_eq!(sidecar(Path::new("/m/Artist/Song.mp3")), Path::new("/m/Artist/Song.lrc"));
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("target/test-lyrics");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let song = dir.join("a.mp3");
        assert!(Lyrics::load(&song).is_none(), "no file");
        std::fs::write(dir.join("a.lrc"), "[00:01.00]one\n").unwrap();
        assert_eq!(Lyrics::load(&song).unwrap().lines().len(), 1);
        std::fs::write(dir.join("a.lrc"), "[ar:Name]\n").unwrap();
        assert!(Lyrics::load(&song).is_none(), "no lines");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
