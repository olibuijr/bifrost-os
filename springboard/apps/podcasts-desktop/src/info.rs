//! The episode info line of the Now Playing sheet: the media type, the codec, the length and the release date of the
//! episode. The type and codec names come from the localized media-name tables in bifrost-ui (`media_type_names`);
//! the codec is read from the episode file's extension, since the app has no container reader of its own. Pure
//! functions: nothing here touches the audio service.

use bifrost_app::controls::media_type_names::{subtype_name, type_name};
use springboard::shell::music::nowplaying::clock;

/// The four-character audio codec code for an episode's file extension, as the name table lists it. The extension
/// is read from the URL's path, so a query string does not hide it. None for an extension the table does not
/// cover, and for a URL with no extension (such as a feed address that only has a dot in its host).
pub fn codec_code(url: &str) -> Option<&'static str> {
    let path = url.split(['?', '#']).next().unwrap_or("");
    let (_, ext) = path.rsplit_once('.')?;
    if ext.contains('/') {
        return None;
    }
    match ext.to_ascii_lowercase().as_str() {
        "mp3" => Some("mp3 "),
        "m4a" | "aac" => Some("mp4a"),
        "opus" => Some("opus"),
        "flac" => Some("flac"),
        "wav" => Some("lpcm"),
        _ => None,
    }
}

/// The codec's name for an episode URL, such as "MP3" for `.mp3`. None when the extension has no codec code.
pub fn codec_name(url: &str) -> Option<&'static str> {
    subtype_name("soun", codec_code(url)?)
}

/// The info line for an episode, such as "Audio, MP3, 45:25, 05 Oct 2026". `duration_ms` is the length the audio
/// service reports (zero while not known); `feed_length` is the length the feed gave, shown when the service has
/// none. Empty parts are left out.
pub fn summary(url: &str, duration_ms: f64, feed_length: &str, released: &str) -> String {
    let mut parts: Vec<String> = Vec::new();
    parts.extend(type_name("soun", "en").map(str::to_string));
    parts.extend(codec_name(url).map(str::to_string));
    let length = if duration_ms.is_finite() && duration_ms > 0.0 { clock(duration_ms) } else { feed_length.trim().to_string() };
    if !length.is_empty() {
        parts.push(length);
    }
    let released = released.trim();
    if !released.is_empty() {
        parts.push(released.to_string());
    }
    parts.join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_come_from_the_extension_ignoring_case_and_query() {
        assert_eq!(codec_code("https://cdn.example.com/ep1.mp3"), Some("mp3 "));
        assert_eq!(codec_code("https://cdn.example.com/ep1.M4A?token=abc.def"), Some("mp4a"));
        assert_eq!(codec_code("https://cdn.example.com/ep1.flac#t=3"), Some("flac"));
        assert_eq!(codec_code("https://cdn.example.com/ep1.wav"), Some("lpcm"));
    }

    #[test]
    fn unknown_or_missing_extensions_have_no_code() {
        assert_eq!(codec_code("https://cdn.example.com/ep1.ogg"), None);
        // the only dot is in the host, so there is no extension
        assert_eq!(codec_code("https://example.com/feed"), None);
        assert_eq!(codec_code("https://cdn.example.com/episode"), None);
    }

    #[test]
    fn codec_names_come_from_the_shared_table() {
        assert_eq!(codec_name("https://cdn.example.com/ep1.mp3"), Some("MP3"));
        assert_eq!(codec_name("https://cdn.example.com/ep1.m4a"), Some("AAC"));
        assert_eq!(codec_name("https://cdn.example.com/ep1.ogg"), None);
    }

    #[test]
    fn the_summary_uses_the_service_length_then_the_feed_length() {
        // 2725 s is 45 minutes 25 seconds
        assert_eq!(summary("https://cdn.example.com/ep1.mp3", 2_725_000.0, "", "05 Oct 2026"), "Audio, MP3, 45:25, 05 Oct 2026");
        // no length from the service yet: the feed's own text is shown
        assert_eq!(summary("https://cdn.example.com/ep1.mp3", 0.0, "1:02:05", "05 Oct 2026"), "Audio, MP3, 1:02:05, 05 Oct 2026");
    }

    #[test]
    fn the_summary_leaves_out_parts_it_does_not_have() {
        assert_eq!(summary("https://cdn.example.com/ep1.ogg", f64::NAN, "", ""), "Audio");
        assert_eq!(summary("https://cdn.example.com/ep1.ogg", -5.0, "  12:00 ", "  "), "Audio, 12:00");
    }
}
