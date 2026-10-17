//! HTTP Live Streaming media playlists (RFC 8216 section 4.3.3): the text an HLS stream starts from, listing the
//! segments to fetch in order. This module parses that text and resolves each segment's address against the playlist's
//! own address. [`load_media`] reads a stream's playlist (choosing a variant from a master playlist) and [`Segments`]
//! joins its segments into one byte stream for the demuxer, fetching each segment when the reader reaches it. Fetching
//! goes through a caller's function (the shell's HTTP client via [`client_fetch`]), so the logic is tested offline.
//! A master playlist is refused by [`parse`] with [`Error::Master`]; [`parse_master`] reads it.

use crate::deildu::http::Client;
use std::collections::VecDeque;
use std::fmt;
use std::io::{self, Cursor, Read};
use std::sync::Arc;

/// Fetches one address and returns its body. Errors name the failure.
pub type Fetch = dyn FnMut(&str) -> io::Result<Vec<u8>> + Send;

/// Why a stream could not be loaded.
#[derive(Debug)]
pub enum LoadError {
    /// A playlist or segment could not be fetched.
    Fetch(io::Error),
    /// The text fetched is not a usable playlist.
    Playlist(Error),
}

impl fmt::Display for LoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LoadError::Fetch(e) => write!(f, "fetch failed: {e}"),
            LoadError::Playlist(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for LoadError {}

/// Reads the media playlist of a stream at `url`. When `url` is a master playlist, the variant
/// [`select_variant`] picks for `max_bandwidth` is fetched and its media playlist read. Returns the address the media
/// playlist came from (its segment addresses are relative to it) and the playlist.
pub fn load_media(get: &mut Fetch, url: &str, max_bandwidth: u64) -> Result<(String, Playlist), LoadError> {
    let text = fetched_text(get, url)?;
    match parse(&text) {
        Ok(playlist) => Ok((url.to_string(), playlist)),
        Err(Error::Master) => {
            let master = parse_master(&text).map_err(LoadError::Playlist)?;
            let variant = select_variant(&master, max_bandwidth).ok_or(LoadError::Playlist(Error::Master))?;
            let media_url = resolve(url, &variant.uri);
            let playlist = parse(&fetched_text(get, &media_url)?).map_err(LoadError::Playlist)?;
            Ok((media_url, playlist))
        }
        Err(e) => Err(LoadError::Playlist(e)),
    }
}

fn fetched_text(get: &mut Fetch, url: &str) -> Result<String, LoadError> {
    get(url).map(|body| String::from_utf8_lossy(&body).into_owned()).map_err(LoadError::Fetch)
}

/// The segments of a media playlist as one byte stream. Each segment is fetched when the reader reaches it, so
/// playback starts after the first segment rather than after the whole list. A failed fetch is an `io::Error` from
/// `read`.
pub struct Segments {
    urls: VecDeque<String>,
    get: Box<Fetch>,
    current: Cursor<Vec<u8>>,
}

impl Segments {
    /// The segments of `playlist`, whose addresses are resolved against `playlist_url`.
    pub fn new(playlist_url: &str, playlist: &Playlist, get: Box<Fetch>) -> Segments {
        let urls = playlist.segments.iter().map(|s| resolve(playlist_url, &s.uri)).collect();
        Segments { urls, get, current: Cursor::new(Vec::new()) }
    }

    /// The segment addresses not yet fetched.
    pub fn remaining(&self) -> usize {
        self.urls.len()
    }
}

impl Read for Segments {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        loop {
            let n = self.current.read(buf)?;
            if n > 0 || buf.is_empty() {
                return Ok(n);
            }
            let Some(url) = self.urls.pop_front() else { return Ok(0) };
            self.current = Cursor::new((self.get)(&url)?);
        }
    }
}

/// A fetch function over the shell's HTTP client. A status outside 2xx is an error.
pub fn client_fetch(client: Arc<Client>, limit: usize) -> Box<Fetch> {
    Box::new(move |url: &str| {
        let r = client.get(url, None, limit)?;
        if (200..300).contains(&r.status) {
            Ok(r.body)
        } else {
            Err(io::Error::other(format!("server answered {}", r.status)))
        }
    })
}

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

/// One variant stream of a master playlist (`#EXT-X-STREAM-INF` and the address on the line after it).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Variant {
    /// The variant's address as written in the playlist (relative or absolute).
    pub uri: String,
    /// `BANDWIDTH` in bits per second.
    pub bandwidth: u64,
    /// `RESOLUTION` as (width, height), when the playlist gives one.
    pub resolution: Option<(u32, u32)>,
}

/// A parsed master playlist: the variant streams to choose from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Master {
    pub variants: Vec<Variant>,
}

/// Parses the text of a master playlist. A variant without a `BANDWIDTH` attribute is refused with
/// [`Error::BadSegment`] carrying its index; a text that lists no variants is refused with [`Error::Master`].
pub fn parse_master(text: &str) -> Result<Master, Error> {
    let mut lines = text.lines().map(str::trim);
    if lines.next() != Some("#EXTM3U") {
        return Err(Error::NotAPlaylist);
    }
    let mut variants: Vec<Variant> = Vec::new();
    let mut pending: Option<(u64, Option<(u32, u32)>)> = None;
    for line in lines {
        if line.is_empty() {
            continue;
        }
        if let Some(attrs) = line.strip_prefix("#EXT-X-STREAM-INF:") {
            let bandwidth = attr(attrs, "BANDWIDTH").and_then(|v| v.parse::<u64>().ok());
            let resolution = attr(attrs, "RESOLUTION").and_then(|v| {
                let (w, h) = v.split_once('x')?;
                Some((w.parse().ok()?, h.parse().ok()?))
            });
            pending = Some((bandwidth.ok_or(Error::BadSegment(variants.len()))?, resolution));
        } else if !line.starts_with('#') {
            let (bandwidth, resolution) = pending.take().ok_or(Error::BadSegment(variants.len()))?;
            variants.push(Variant { uri: line.to_string(), bandwidth, resolution });
        }
    }
    if variants.is_empty() {
        return Err(Error::Master);
    }
    Ok(Master { variants })
}

/// Reads one attribute from an attribute list such as `BANDWIDTH=800000,RESOLUTION=640x360`. Quoted values lose
/// their quotes. The name must match exactly, so `BANDWIDTH` never matches `AVERAGE-BANDWIDTH`.
fn attr<'a>(list: &'a str, name: &str) -> Option<&'a str> {
    let mut rest = list;
    while !rest.is_empty() {
        let (key, after) = rest.split_once('=')?;
        let (value, next) = if let Some(quoted) = after.strip_prefix('"') {
            let end = quoted.find('"')?;
            (&quoted[..end], quoted[end + 1..].trim_start_matches(','))
        } else {
            match after.find(',') {
                Some(i) => (&after[..i], &after[i + 1..]),
                None => (after, ""),
            }
        };
        if key.trim() == name {
            return Some(value);
        }
        rest = next;
    }
    None
}

/// Chooses the variant to play: the highest-bandwidth variant at or below `max_bandwidth`, or the lowest-bandwidth
/// variant when none fits. Returns `None` only when the master has no variants.
pub fn select_variant(master: &Master, max_bandwidth: u64) -> Option<&Variant> {
    let fitting = master.variants.iter().filter(|v| v.bandwidth <= max_bandwidth).max_by_key(|v| v.bandwidth);
    fitting.or_else(|| master.variants.iter().min_by_key(|v| v.bandwidth))
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

    const MASTER: &str = "#EXTM3U\n\
#EXT-X-STREAM-INF:BANDWIDTH=800000,AVERAGE-BANDWIDTH=700000,RESOLUTION=640x360\nlow/index.m3u8\n\
#EXT-X-STREAM-INF:BANDWIDTH=2800000,RESOLUTION=1280x720,CODECS=\"avc1.4d401f,mp4a.40.2\"\nhigh/index.m3u8\n\
#EXT-X-STREAM-INF:BANDWIDTH=1400000\nmid.m3u8\n";

    #[test]
    fn a_master_playlist_lists_its_variants() {
        let m = parse_master(MASTER).unwrap();
        assert_eq!(m.variants.len(), 3);
        assert_eq!(m.variants[0], Variant { uri: "low/index.m3u8".into(), bandwidth: 800_000, resolution: Some((640, 360)) });
        assert_eq!(m.variants[1].bandwidth, 2_800_000, "the quoted CODECS value does not split the list");
        assert_eq!(m.variants[1].resolution, Some((1280, 720)));
        assert_eq!(m.variants[2].resolution, None);
    }

    #[test]
    fn the_variant_is_the_best_one_under_the_limit() {
        let m = parse_master(MASTER).unwrap();
        assert_eq!(select_variant(&m, 1_500_000).unwrap().uri, "mid.m3u8");
        assert_eq!(select_variant(&m, 10_000_000).unwrap().uri, "high/index.m3u8");
    }

    #[test]
    fn the_lowest_variant_is_chosen_when_none_fits() {
        let m = parse_master(MASTER).unwrap();
        assert_eq!(select_variant(&m, 100).unwrap().uri, "low/index.m3u8");
    }

    #[test]
    fn a_media_playlist_is_not_a_master_playlist() {
        assert_eq!(parse_master("#EXTM3U\n#EXT-X-TARGETDURATION:4\n#EXTINF:4,\na.ts\n"), Err(Error::Master));
    }

    #[test]
    fn a_variant_without_bandwidth_is_refused() {
        let text = "#EXTM3U\n#EXT-X-STREAM-INF:RESOLUTION=640x360\nlow.m3u8\n";
        assert_eq!(parse_master(text), Err(Error::BadSegment(0)));
    }

    /// A fake server: a map from address to body. An unknown address is a 404 error.
    fn server(files: Vec<(&'static str, &'static str)>) -> Box<Fetch> {
        let files: std::collections::HashMap<&'static str, &'static str> = files.into_iter().collect();
        Box::new(move |url: &str| match files.get(url) {
            Some(body) => Ok(body.as_bytes().to_vec()),
            None => Err(io::Error::other(format!("server answered 404 to {url}"))),
        })
    }

    const MEDIA: &str = "#EXTM3U\n#EXT-X-TARGETDURATION:4\n#EXTINF:4,\nseg0.ts\n#EXTINF:4,\nseg1.ts\n#EXT-X-ENDLIST\n";

    #[test]
    fn a_media_playlist_loads_directly() {
        let mut get = server(vec![("https://cdn.example/a/index.m3u8", MEDIA)]);
        let (url, p) = load_media(&mut *get, "https://cdn.example/a/index.m3u8", 0).unwrap();
        assert_eq!(url, "https://cdn.example/a/index.m3u8");
        assert_eq!(p.segments.len(), 2);
    }

    #[test]
    fn a_master_playlist_loads_the_chosen_variant() {
        let master = "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=800000\nlow/index.m3u8\n#EXT-X-STREAM-INF:BANDWIDTH=2800000\nhigh/index.m3u8\n";
        let mut get = server(vec![("https://cdn.example/m.m3u8", master), ("https://cdn.example/low/index.m3u8", MEDIA)]);
        let (url, p) = load_media(&mut *get, "https://cdn.example/m.m3u8", 1_000_000).unwrap();
        assert_eq!(url, "https://cdn.example/low/index.m3u8");
        assert_eq!(p.segments[0].uri, "seg0.ts");
    }

    #[test]
    fn a_failed_playlist_fetch_is_reported() {
        let mut get = server(vec![]);
        assert!(matches!(load_media(&mut *get, "https://cdn.example/x.m3u8", 0), Err(LoadError::Fetch(_))));
    }

    #[test]
    fn segments_are_joined_in_order_and_fetched_as_they_are_read() {
        let fetched = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let log = fetched.clone();
        let mut files = server(vec![("https://cdn.example/a/seg0.ts", "AAAA"), ("https://cdn.example/a/seg1.ts", "BB")]);
        let get: Box<Fetch> = Box::new(move |url: &str| {
            log.lock().unwrap().push(url.to_string());
            files(url)
        });
        let p = parse(MEDIA).unwrap();
        let mut segs = Segments::new("https://cdn.example/a/index.m3u8", &p, get);
        assert_eq!(segs.remaining(), 2);
        let mut first = [0u8; 2];
        assert_eq!(segs.read(&mut first).unwrap(), 2);
        assert_eq!(&first, b"AA");
        assert_eq!(fetched.lock().unwrap().len(), 1, "the second segment waits until it is needed");
        let mut rest = String::new();
        segs.read_to_string(&mut rest).unwrap();
        assert_eq!(rest, "AABB");
        assert_eq!(segs.remaining(), 0);
    }

    #[test]
    fn a_failed_segment_is_an_error_from_read() {
        let mut files = server(vec![("https://cdn.example/a/seg0.ts", "AAAA")]);
        let get: Box<Fetch> = Box::new(move |url: &str| files(url));
        let p = parse(MEDIA).unwrap();
        let mut segs = Segments::new("https://cdn.example/a/index.m3u8", &p, get);
        let mut all = Vec::new();
        assert!(segs.read_to_end(&mut all).is_err());
        assert_eq!(all, b"AAAA");
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
