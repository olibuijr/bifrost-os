//! Episode transcripts: WebVTT and SubRip files become timed cues; any other text becomes untimed paragraphs.
//! Pure functions: the file is fetched by the app.

use crate::feed::clock_ms;

/// One line of a transcript. `start_ms` is None for plain text with no timing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cue {
    pub start_ms: Option<u64>,
    pub text: String,
}

/// Removes `<...>` markup (voice and style tags in cue text).
fn strip_tags(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            '<' => in_tag = true,
            '>' if in_tag => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    out
}

/// Reads a transcript. A `-->` timing line starts a timed cue; text with none is split at blank lines.
pub fn parse(raw: &str) -> Vec<Cue> {
    let raw = raw.trim_start_matches('\u{feff}').replace("\r\n", "\n");
    let timed = raw.lines().any(|l| l.contains("-->"));
    let mut cues = Vec::new();
    for block in raw.split("\n\n") {
        let lines: Vec<&str> = block.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
        if lines.is_empty() {
            continue;
        }
        if !timed {
            cues.push(Cue { start_ms: None, text: lines.join(" ") });
            continue;
        }
        let Some(at) = lines.iter().position(|l| l.contains("-->")) else { continue };
        let start = lines[at].split("-->").next().and_then(clock_ms);
        let text = strip_tags(&lines[at + 1..].join(" ")).trim().to_string();
        if start.is_some() && !text.is_empty() {
            cues.push(Cue { start_ms: start, text });
        }
    }
    cues
}

/// The cue being spoken at `position_ms`: the last timed cue that has started. None before the first cue and for
/// untimed text.
pub fn active(cues: &[Cue], position_ms: u64) -> Option<usize> {
    cues.iter().rposition(|c| c.start_ms.is_some_and(|s| s <= position_ms))
}

/// Where playback jumps when line `cue` of the transcript is tapped: its start time. None for untimed text.
pub fn start_of(cues: &[Cue], cue: usize) -> Option<u64> {
    cues.get(cue)?.start_ms
}

/// The wrapped line under a point `y` pixels below the top of the lines (scrolling already added), for lines
/// `line_h` pixels high. None above the first line and below the last of `count`.
pub fn line_at(y: f32, line_h: f32, count: usize) -> Option<usize> {
    if y < 0.0 || line_h <= 0.0 {
        return None;
    }
    let line = (y / line_h).floor() as usize;
    (line < count).then_some(line)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn webvtt_cues_are_timed_and_stripped() {
        let vtt = "WEBVTT\n\n1\n00:00:01.000 --> 00:00:03.000\n<v Ann>Hello\nthere</v>\n\n00:01:05.500 --> 00:01:07.000\nSecond\n";
        let cues = parse(vtt);
        assert_eq!(cues, [Cue { start_ms: Some(1_000), text: "Hello there".into() }, Cue { start_ms: Some(65_500), text: "Second".into() }]);
    }

    #[test]
    fn subrip_uses_commas_and_crlf() {
        let srt = "1\r\n00:00:02,500 --> 00:00:04,000\r\nHi\r\n\r\n2\r\n00:00:05,000 --> 00:00:06,000\r\nBye\r\n";
        let cues = parse(srt);
        assert_eq!(cues.iter().map(|c| c.start_ms).collect::<Vec<_>>(), [Some(2_500), Some(5_000)]);
    }

    #[test]
    fn plain_text_becomes_untimed_paragraphs() {
        let cues = parse("First para\nstill first\n\nSecond para\n");
        assert_eq!(cues, [Cue { start_ms: None, text: "First para still first".into() }, Cue { start_ms: None, text: "Second para".into() }]);
        assert_eq!(active(&cues, 99_999), None);
    }

    #[test]
    fn the_active_cue_is_the_last_one_started() {
        let cues = parse("00:00:01.000 --> 00:00:02.000\na\n\n00:00:10.000 --> 00:00:12.000\nb\n");
        assert_eq!(active(&cues, 500), None);
        assert_eq!(active(&cues, 1_000), Some(0));
        assert_eq!(active(&cues, 9_999), Some(0));
        assert_eq!(active(&cues, 10_000), Some(1));
    }

    #[test]
    fn a_tapped_cue_seeks_to_its_start_and_untimed_text_does_not() {
        let cues = parse("00:00:01.000 --> 00:00:02.000\na\n\n00:00:10.500 --> 00:00:12.000\nb\n");
        assert_eq!(start_of(&cues, 0), Some(1_000));
        assert_eq!(start_of(&cues, 1), Some(10_500));
        assert_eq!(start_of(&cues, 2), None);
        assert_eq!(start_of(&parse("Just text"), 0), None);
    }

    #[test]
    fn a_point_maps_to_the_wrapped_line_under_it() {
        assert_eq!(line_at(0.0, 20.0, 3), Some(0));
        assert_eq!(line_at(19.9, 20.0, 3), Some(0));
        assert_eq!(line_at(20.0, 20.0, 3), Some(1));
        assert_eq!(line_at(59.9, 20.0, 3), Some(2));
        assert_eq!(line_at(60.0, 20.0, 3), None);
        assert_eq!(line_at(-1.0, 20.0, 3), None);
        assert_eq!(line_at(5.0, 0.0, 3), None);
    }
}
