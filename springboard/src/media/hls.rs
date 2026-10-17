//! HTTP Live Streaming media playlists (RFC 8216 section 4.3.3): the text an HLS stream starts from, listing the
//! segments to fetch in order. This module parses that text and resolves each segment's address against the playlist's
//! own address. It fetches nothing and decodes nothing; the caller downloads the segments and hands them to the demuxer.
//! A master playlist (a list of variant streams) is refused with [`Error::Master`]; choose a variant and fetch its
//! media playlist instead.

use std::fmt;

/// One media segment of a playlist.
#[derive(Clone, Debug, PartialEq)]
pub struct Segment {
    /// The segment's address as written in the playlist (relative or absolute).
    pub uri: String,
    /// Its length in seconds, from the `#EXTINF` tag before it.
    pub duration_s: f64,
    /// `true` when an `#EXT-X-DISCONTINUITY` tag precedes it (the timestamps restart).
    pub discontinuity: bool,
}

/// A parsed media playlist.
#[derive(Clone, Debug, PartialEq)]
pub struct Playlist {
    /// `#EXT-X-TARGETDURATION`, the longest segment in whole seconds.
    pub target_duration: u32,
    /// `#EXT-X-MEDIA-SEQUENCE`, the number of the first segment (0 when absent).
    pub media_sequence: u64,
    /// `true` when `#EXT-X-ENDLIST` closes the playlist, so it is video on demand; `false` for a live playlist that
    /// must be reloaded.
    pub ended: bool,
    pub segments: Vec<Segment>,
}

/// Why a playlist was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// The text does not start with `#EXTM3U`.
    NotAPlaylist,
    /// The text lists variant streams (`#EXT-X-STREAM-INF`), so it is a master playlist.
    Master,
    /// `#EXT-X-TARGETDURATION` is missing or not a number.
    NoTargetDuration,
    /// A segment has no `#EXTINF` duration before its address, or the duration is not a number.
    BadSegment(usize),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::NotAPlaylist => write!(f, "not an HLS playlist"),
            Error::Master => write!(f, "master playlist: choose a variant"),
            Error::NoTargetDuration => write!(f, "playlist has no target duration"),
            Error::BadSegment(n) => write!(f, "segment {n} has no duration"),
        }
    }
}

impl std::error::Error for Error {}

/// Parses the text of a media playlist.
pub fn parse(text: &str) -> Result<Playlist, Error> {
    let mut lines = text.lines().map(str::trim);
    if lines.next() != Some("#EXTM3U") {
        return Err(Error::NotAPlaylist);
    }
    let mut target_duration = None;
    let mut media_sequence = 0;
    let mut ended = false;
    let mut segments: Vec<Segment> = Vec::new();
    let mut pending: Option<f64> = None;
    let mut discontinuity = false;
    for line in lines {
        if line.is_empty() {
            continue;
        }
        if line.starts_with('#') {
            if line.starts_with("#EXT-X-STREAM-INF") {
                return Err(Error::Master);
            } else if let Some(v) = line.strip_prefix("#EXT-X-TARGETDURATION:") {
                target_duration = v.trim().parse::<u32>().ok();
            } else if let Some(v) = line.strip_prefix("#EXT-X-MEDIA-SEQUENCE:") {
                media_sequence = v.trim().parse().unwrap_or(0);
            } else if let Some(v) = line.strip_prefix("#EXTINF:") {
                let secs = v.split(',').next().unwrap_or("");
                pending = Some(secs.trim().parse::<f64>().map_err(|_| Error::BadSegment(segments.len()))?);
            } else if line == "#EXT-X-DISCONTINUITY" {
                discontinuity = true;
            } else if line == "#EXT-X-ENDLIST" {
                ended = true;
            }
        } else {
            let duration_s = pending.take().ok_or(Error::BadSegment(segments.len()))?;
            segments.push(Segment { uri: line.to_string(), duration_s, discontinuity: std::mem::take(&mut discontinuity) });
        }
    }
    let target_duration = target_duration.ok_or(Error::NoTargetDuration)?;
    Ok(Playlist { target_duration, media_sequence, ended, segments })
}

/// Resolves a segment address against the address of the playlist it came from. Absolute `http://` and `https://`
/// addresses are kept as they are; a path starting with `/` keeps the playlist's origin; anything else is relative to
/// the playlist's directory.
pub fn resolve(playlist_url: &str, uri: &str) -> String {
    if uri.starts_with("http://") || uri.starts_with("https://") {
        return uri.to_string();
    }
    let Some(scheme_end) = playlist_url.find("://").map(|i| i + 3) else {
        return uri.to_string();
    };
    if uri.starts_with('/') {
        let origin_end = playlist_url[scheme_end..].find('/').map_or(playlist_url.len(), |i| scheme_end + i);
        return format!("{}{}", &playlist_url[..origin_end], uri);
    }
    let dir_end = playlist_url.rfind('/').filter(|&i| i >= scheme_end).map_or(playlist_url.len(), |i| i + 1);
    let dir = if dir_end == playlist_url.len() && !playlist_url.ends_with('/') {
        format!("{playlist_url}/")
    } else {
        playlist_url[..dir_end].to_string()
    };
    format!("{dir}{uri}")
}

#[cfg(test)]
mod tests {
    use super::*;

    const VOD: &str = "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:10\n#EXT-X-MEDIA-SEQUENCE:0\n\
#EXTINF:9.009,\nseg0.ts\n#EXTINF:9.5,title\nseg1.ts\n#EXT-X-DISCONTINUITY\n#EXTINF:4,\nhttps://cdn.example/seg2.ts\n#EXT-X-ENDLIST\n";

    #[test]
    fn a_vod_playlist_lists_its_segments_in_order() {
        let p = parse(VOD).unwrap();
        assert_eq!(p.target_duration, 10);
        assert_eq!(p.media_sequence, 0);
        assert!(p.ended);
        assert_eq!(p.segments.len(), 3);
        assert_eq!(p.segments[0], Segment { uri: "seg0.ts".into(), duration_s: 9.009, discontinuity: false });
        assert_eq!(p.segments[1].duration_s, 9.5);
        assert!(p.segments[2].discontinuity, "the discontinuity marks the segment after it");
        assert!(!p.segments[1].discontinuity);
    }

    #[test]
    fn a_live_playlist_is_not_ended_and_keeps_its_sequence_number() {
        let p = parse("#EXTM3U\n#EXT-X-TARGETDURATION:6\n#EXT-X-MEDIA-SEQUENCE:812\n#EXTINF:6,\na.ts\n").unwrap();
        assert!(!p.ended);
        assert_eq!(p.media_sequence, 812);
    }

    #[test]
    fn a_master_playlist_is_refused() {
        let text = "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=800000\nlow.m3u8\n";
        assert_eq!(parse(text), Err(Error::Master));
    }

    #[test]
    fn text_that_is_not_a_playlist_is_refused() {
        assert_eq!(parse("<html>"), Err(Error::NotAPlaylist));
    }

    #[test]
    fn a_missing_target_duration_is_refused() {
        assert_eq!(parse("#EXTM3U\n#EXTINF:4,\na.ts\n"), Err(Error::NoTargetDuration));
    }

    #[test]
    fn a_segment_without_a_duration_is_refused_with_its_index() {
        let text = "#EXTM3U\n#EXT-X-TARGETDURATION:4\n#EXTINF:4,\na.ts\nb.ts\n";
        assert_eq!(parse(text), Err(Error::BadSegment(1)));
    }

    #[test]
    fn relative_addresses_resolve_against_the_playlist_directory() {
        let base = "https://cdn.example/live/720p/index.m3u8";
        assert_eq!(resolve(base, "seg1.ts"), "https://cdn.example/live/720p/seg1.ts");
        assert_eq!(resolve(base, "/other/seg.ts"), "https://cdn.example/other/seg.ts");
        assert_eq!(resolve(base, "https://elsewhere/x.ts"), "https://elsewhere/x.ts");
        assert_eq!(resolve("https://cdn.example", "a.ts"), "https://cdn.example/a.ts");
    }
}
