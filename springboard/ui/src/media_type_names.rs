//! Localized names of media types and media subtypes (`MTCopyLocalizedNameForMediaType`,
//! `MTCopyLocalizedNameForMediaSubType`). A media type is a four-character code (audio, video, text, subtitles,
//! closed captions); a subtype is a codec code inside one media type. Pure lookup tables: the type names come in
//! English and Icelandic, the codec names are English in every language. A code that is not in the table has no
//! name, and the caller shows the raw code instead.
//!
//! Gaps: only two languages, no name for a code the table does not list, no user-set names, and the codec names are
//! not translated. The codes are the ones the shell's own container readers produce and the common video codecs.

/// The language a name is looked up in, from a language tag such as "en", "en_US" or "is_IS".
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Language {
    English,
    Icelandic,
}

impl Language {
    /// The language of a tag: the primary subtag decides ("is_IS" is Icelandic). Anything else is English.
    pub fn from_tag(tag: &str) -> Self {
        let primary = tag.split(|c: char| c == '_' || c == '-').next().unwrap_or("");
        if primary.eq_ignore_ascii_case("is") {
            Language::Icelandic
        } else {
            Language::English
        }
    }
}

/// Media type codes and their names. Order is not significant.
const TYPE_NAMES: &[(&str, &str, &str)] = &[
    // (code, English, Icelandic)
    ("soun", "Audio", "Hljóð"),
    ("vide", "Video", "Myndband"),
    ("text", "Text", "Texti"),
    ("subt", "Subtitles", "Skjátexti"),
    ("clcp", "Closed captions", "Lokaður texti"),
];

/// Subtype codes as (media type code, subtype code, English name). The codec names are the same in every language.
const SUBTYPE_NAMES: &[(&str, &str, &str)] = &[
    ("soun", "mp4a", "AAC"),
    ("soun", "mp3 ", "MP3"),
    ("soun", "lpcm", "Linear PCM"),
    ("soun", "flac", "FLAC"),
    ("soun", "opus", "Opus"),
    ("soun", "ac-3", "Dolby Digital"),
    ("soun", "ec-3", "Dolby Digital Plus"),
    ("vide", "avc1", "H.264"),
    ("vide", "hvc1", "HEVC"),
    ("vide", "vp09", "VP9"),
    ("vide", "av01", "AV1"),
    ("vide", "mp4v", "MPEG-4 Visual"),
    ("subt", "wvtt", "WebVTT"),
];

/// Whether a code is exactly four bytes, so a short code such as "mp3" never matches the padded "mp3 ".
fn is_code(code: &str) -> bool {
    code.len() == 4
}

/// The name of a media type in a language, such as "Audio" or "Hljóð". None for an unknown or malformed code.
pub fn type_name(code: &str, language: &str) -> Option<&'static str> {
    if !is_code(code) {
        return None;
    }
    let lang = Language::from_tag(language);
    TYPE_NAMES.iter().find(|(c, _, _)| *c == code).map(|(_, en, is)| match lang {
        Language::English => *en,
        Language::Icelandic => *is,
    })
}

/// The name of a subtype (codec) inside a media type, such as `("vide", "avc1")` gives "H.264". None when the
/// subtype is not listed under that media type, so "mp4a" under video has no name.
pub fn subtype_name(media: &str, subtype: &str) -> Option<&'static str> {
    if !is_code(media) || !is_code(subtype) {
        return None;
    }
    SUBTYPE_NAMES.iter().find(|(m, s, _)| *m == media && *s == subtype).map(|(_, _, name)| *name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn type_names_follow_the_language_tag() {
        assert_eq!(type_name("soun", "en"), Some("Audio"));
        assert_eq!(type_name("soun", "en_US"), Some("Audio"));
        assert_eq!(type_name("soun", "is_IS"), Some("Hljóð"));
        assert_eq!(type_name("vide", "is"), Some("Myndband"));
        // a tag we have no table for falls back to English
        assert_eq!(type_name("clcp", "de"), Some("Closed captions"));
    }

    #[test]
    fn unknown_or_malformed_type_codes_have_no_name() {
        assert_eq!(type_name("xxxx", "en"), None);
        // three letters is not the four-byte code, even when its first three bytes match
        assert_eq!(type_name("sou", "en"), None);
        assert_eq!(type_name("soundz", "en"), None);
    }

    #[test]
    fn subtype_names_are_scoped_to_their_media_type() {
        assert_eq!(subtype_name("vide", "avc1"), Some("H.264"));
        assert_eq!(subtype_name("soun", "mp4a"), Some("AAC"));
        // AAC is an audio subtype, so asking for it under video gives nothing
        assert_eq!(subtype_name("vide", "mp4a"), None);
    }

    #[test]
    fn padded_codes_must_match_exactly() {
        // the MP3 code is four bytes with a trailing space; the trimmed code is not a match
        assert_eq!(subtype_name("soun", "mp3 "), Some("MP3"));
        assert_eq!(subtype_name("soun", "mp3"), None);
    }

    #[test]
    fn language_tags_are_read_by_primary_subtag() {
        assert_eq!(Language::from_tag("is-IS"), Language::Icelandic);
        assert_eq!(Language::from_tag("IS"), Language::Icelandic);
        assert_eq!(Language::from_tag("island"), Language::English);
        assert_eq!(Language::from_tag(""), Language::English);
    }
}
