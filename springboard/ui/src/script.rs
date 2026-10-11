//! Script detection: which writing system each character belongs to, and the runs of one script in a string.
//! The scripts are the standard script tags of the Natural Language framework's script type (`NLScript`): the
//! set is the same, names are the same lower camel case tags. Han characters are reported as `Han`. Telling
//! simplified from traditional Chinese needs a lexicon, which this module does not carry, so it is not guessed.
//! Common characters (digits, spaces, punctuation, symbols) carry no script. They stay inside the run they sit in.

use std::ops::Range;

/// A writing system, one per standard script tag.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Script {
    Arabic,
    Armenian,
    Bengali,
    CanadianAboriginalSyllabics,
    Cherokee,
    Cyrillic,
    Devanagari,
    Ethiopic,
    Georgian,
    Greek,
    Gujarati,
    Gurmukhi,
    /// Han ideographs (Chinese characters), without the simplified or traditional split.
    Han,
    Hebrew,
    /// Hiragana and katakana. Kanji are `Han`.
    Japanese,
    Kannada,
    Khmer,
    Korean,
    Lao,
    Latin,
    Malayalam,
    Mongolian,
    Myanmar,
    Oriya,
    Sinhala,
    Tamil,
    Telugu,
    Thai,
    Tibetan,
}

/// Every script, in tag order.
pub const ALL: &[Script] = &[
    Script::Arabic,
    Script::Armenian,
    Script::Bengali,
    Script::CanadianAboriginalSyllabics,
    Script::Cherokee,
    Script::Cyrillic,
    Script::Devanagari,
    Script::Ethiopic,
    Script::Georgian,
    Script::Greek,
    Script::Gujarati,
    Script::Gurmukhi,
    Script::Han,
    Script::Hebrew,
    Script::Japanese,
    Script::Kannada,
    Script::Khmer,
    Script::Korean,
    Script::Lao,
    Script::Latin,
    Script::Malayalam,
    Script::Mongolian,
    Script::Myanmar,
    Script::Oriya,
    Script::Sinhala,
    Script::Tamil,
    Script::Telugu,
    Script::Thai,
    Script::Tibetan,
];

impl Script {
    /// The stable tag used in stored data, such as `latin` or `canadianAboriginalSyllabics`.
    pub fn tag(self) -> &'static str {
        match self {
            Script::Arabic => "arabic",
            Script::Armenian => "armenian",
            Script::Bengali => "bengali",
            Script::CanadianAboriginalSyllabics => "canadianAboriginalSyllabics",
            Script::Cherokee => "cherokee",
            Script::Cyrillic => "cyrillic",
            Script::Devanagari => "devanagari",
            Script::Ethiopic => "ethiopic",
            Script::Georgian => "georgian",
            Script::Greek => "greek",
            Script::Gujarati => "gujarati",
            Script::Gurmukhi => "gurmukhi",
            Script::Han => "han",
            Script::Hebrew => "hebrew",
            Script::Japanese => "japanese",
            Script::Kannada => "kannada",
            Script::Khmer => "khmer",
            Script::Korean => "korean",
            Script::Lao => "lao",
            Script::Latin => "latin",
            Script::Malayalam => "malayalam",
            Script::Mongolian => "mongolian",
            Script::Myanmar => "myanmar",
            Script::Oriya => "oriya",
            Script::Sinhala => "sinhala",
            Script::Tamil => "tamil",
            Script::Telugu => "telugu",
            Script::Thai => "thai",
            Script::Tibetan => "tibetan",
        }
    }

    /// The script with this tag, if any.
    pub fn from_tag(tag: &str) -> Option<Script> {
        ALL.iter().copied().find(|s| s.tag() == tag)
    }
}

/// The script of one character, or `None` for common characters (digits, spaces, punctuation, symbols).
pub fn script_of(c: char) -> Option<Script> {
    let u = c as u32;
    let s = match u {
        0x41..=0x5A
        | 0x61..=0x7A
        | 0xAA
        | 0xBA
        | 0xC0..=0xD6
        | 0xD8..=0xF6
        | 0xF8..=0x24F
        | 0x1E00..=0x1EFF
        | 0x2C60..=0x2C7F
        | 0xA720..=0xA7FF
        | 0xFF21..=0xFF3A
        | 0xFF41..=0xFF5A => Script::Latin,
        0x370..=0x3FF | 0x1F00..=0x1FFF => Script::Greek,
        0x400..=0x52F | 0x2DE0..=0x2DFF | 0xA640..=0xA69F => Script::Cyrillic,
        0x531..=0x58F => Script::Armenian,
        0x591..=0x5FF | 0xFB1D..=0xFB4F => Script::Hebrew,
        0x600..=0x6FF | 0x750..=0x77F | 0x8A0..=0x8FF | 0xFB50..=0xFDFF | 0xFE70..=0xFEFF => Script::Arabic,
        0x900..=0x97F | 0xA8E0..=0xA8FF => Script::Devanagari,
        0x980..=0x9FF => Script::Bengali,
        0xA00..=0xA7F => Script::Gurmukhi,
        0xA80..=0xAFF => Script::Gujarati,
        0xB00..=0xB7F => Script::Oriya,
        0xB80..=0xBFF => Script::Tamil,
        0xC00..=0xC7F => Script::Telugu,
        0xC80..=0xCFF => Script::Kannada,
        0xD00..=0xD7F => Script::Malayalam,
        0xD80..=0xDFF => Script::Sinhala,
        0xE01..=0xE7F => Script::Thai,
        0xE80..=0xEFF => Script::Lao,
        0xF00..=0xFFF => Script::Tibetan,
        0x1000..=0x109F | 0xAA60..=0xAA7F => Script::Myanmar,
        0x10A0..=0x10FF | 0x2D00..=0x2D2F => Script::Georgian,
        0x1200..=0x139F | 0x2D80..=0x2DDF | 0xAB00..=0xAB2F => Script::Ethiopic,
        0x13A0..=0x13FF | 0xAB70..=0xABBF => Script::Cherokee,
        0x1400..=0x167F | 0x18B0..=0x18FF => Script::CanadianAboriginalSyllabics,
        0x1780..=0x17FF | 0x19E0..=0x19FF => Script::Khmer,
        0x1800..=0x18AF | 0x11660..=0x1167F => Script::Mongolian,
        0x1100..=0x11FF | 0x3130..=0x318F | 0xA960..=0xA97F | 0xAC00..=0xD7FF => Script::Korean,
        0x3041..=0x3096 | 0x309D..=0x309F | 0x30A1..=0x30FA | 0x30FC..=0x30FF | 0xFF66..=0xFF9D => Script::Japanese,
        0x2E80..=0x2FDF
        | 0x3005
        | 0x3007
        | 0x3021..=0x3029
        | 0x3400..=0x4DBF
        | 0x4E00..=0x9FFF
        | 0xF900..=0xFAFF
        | 0x20000..=0x2FA1F => Script::Han,
        _ => return None,
    };
    Some(s)
}

/// A stretch of text written in one script. `range` is a byte range into the source, on char boundaries.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScriptRun {
    pub script: Script,
    pub range: Range<usize>,
}

/// The runs of text in `text`, in order. Adjacent characters of one script form one run. A common character
/// extends the run before it. Text before the first scripted character belongs to no run.
pub fn runs(text: &str) -> Vec<ScriptRun> {
    let mut out: Vec<ScriptRun> = Vec::new();
    for (i, c) in text.char_indices() {
        let end = i + c.len_utf8();
        match script_of(c) {
            Some(s) => match out.last_mut() {
                Some(r) if r.script == s => r.range.end = end,
                _ => out.push(ScriptRun { script: s, range: i..end }),
            },
            None => {
                if let Some(r) = out.last_mut() {
                    r.range.end = end;
                }
            }
        }
    }
    out
}

/// The script that the most characters of `text` belong to. A tie goes to the script that appears first.
/// `None` when no character has a script.
pub fn dominant(text: &str) -> Option<Script> {
    let mut counts: Vec<(Script, usize)> = Vec::new();
    for s in text.chars().filter_map(script_of) {
        match counts.iter_mut().find(|(k, _)| *k == s) {
            Some(entry) => entry.1 += 1,
            None => counts.push((s, 1)),
        }
    }
    let mut best: Option<(Script, usize)> = None;
    for (s, n) in counts {
        if best.map_or(true, |(_, b)| n > b) {
            best = Some((s, n));
        }
    }
    best.map(|(s, _)| s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn letters_map_to_their_scripts() {
        assert_eq!(script_of('a'), Some(Script::Latin));
        assert_eq!(script_of('ð'), Some(Script::Latin));
        assert_eq!(script_of('П'), Some(Script::Cyrillic));
        assert_eq!(script_of('α'), Some(Script::Greek));
        assert_eq!(script_of('ש'), Some(Script::Hebrew));
        assert_eq!(script_of('م'), Some(Script::Arabic));
        assert_eq!(script_of('ก'), Some(Script::Thai));
        assert_eq!(script_of('한'), Some(Script::Korean));
        assert_eq!(script_of('か'), Some(Script::Japanese));
        assert_eq!(script_of('漢'), Some(Script::Han));
    }

    #[test]
    fn common_characters_have_no_script() {
        for c in ['1', ' ', '.', '-', '€', '×', '÷', '・'] {
            assert_eq!(script_of(c), None, "{c:?} should be common");
        }
    }

    #[test]
    fn runs_split_at_script_changes_with_byte_ranges() {
        let text = "Hello Привет 2026!";
        let got = runs(text);
        let scripts: Vec<Script> = got.iter().map(|r| r.script).collect();
        assert_eq!(scripts, vec![Script::Latin, Script::Cyrillic]);
        // The space and digits after "Hello" stay with the Latin run; the trailing "!" with Cyrillic.
        assert_eq!(&text[got[0].range.clone()], "Hello ");
        assert_eq!(&text[got[1].range.clone()], "Привет 2026!");
    }

    #[test]
    fn leading_common_text_belongs_to_no_run() {
        let text = "  12 漢字";
        let got = runs(text);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].script, Script::Han);
        assert_eq!(&text[got[0].range.clone()], "漢字");
    }

    #[test]
    fn dominant_counts_characters_and_breaks_ties_by_first_seen() {
        // the larger count wins whatever comes first
        assert_eq!(dominant("abc Привет"), Some(Script::Cyrillic));
        // a tie goes to the script seen first
        assert_eq!(dominant("abc жзи"), Some(Script::Latin));
        assert_eq!(dominant("жзи abc"), Some(Script::Cyrillic));
        assert_eq!(dominant("ab ЖЖ"), Some(Script::Latin));
        assert_eq!(dominant("123 !?"), None);
    }

    #[test]
    fn tags_round_trip() {
        for s in ALL {
            assert_eq!(Script::from_tag(s.tag()), Some(*s));
        }
        assert_eq!(Script::from_tag("canadianAboriginalSyllabics"), Some(Script::CanadianAboriginalSyllabics));
        assert_eq!(Script::from_tag("klingon"), None);
        assert_eq!(ALL.len(), 29);
    }
}
