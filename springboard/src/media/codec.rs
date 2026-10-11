//! Which codec / container a file or a live stream is: the one place that
//! looks at content types, magic bytes and file extensions.

use super::aacstream::{adts_frame, loas_frame, Framing, LatmState, Parsed};
use super::id3;
use std::path::Path;

/// What the audio bytes are.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Mp3,
    /// AAC in an MP4 container
    M4a,
    /// bare AAC with ADTS headers
    Adts,
    /// AAC as LATM in a LOAS sync stream (HE-AAC radio)
    Latm,
    Flac,
    /// Ogg (Vorbis, or FLAC in Ogg)
    Ogg,
    Wav,
}

impl Kind {
    /// The framing of a bare-AAC kind.
    pub fn aac_framing(self) -> Option<Framing> {
        match self {
            Kind::Adts => Some(Framing::Adts),
            Kind::Latm => Some(Framing::Loas),
            _ => None,
        }
    }
}

/// The kind by file extension; None for files the Music app does not play.
pub fn from_extension(path: &Path) -> Option<Kind> {
    match path.extension()?.to_str()?.to_ascii_lowercase().as_str() {
        "mp3" => Some(Kind::Mp3),
        "m4a" | "mp4" | "m4b" => Some(Kind::M4a),
        "aac" | "adts" => Some(Kind::Adts),
        "loas" | "latm" => Some(Kind::Latm),
        "flac" => Some(Kind::Flac),
        "ogg" | "oga" => Some(Kind::Ogg),
        "wav" | "wave" => Some(Kind::Wav),
        _ => None,
    }
}

/// The kind a media type names (parameters and case ignored). `audio/aacp`
/// and friends are "some AAC"; `Adts` stands for that family here and
/// `sniff` tells ADTS from LOAS by looking at the bytes.
fn from_content_type(content_type: &str) -> Option<Kind> {
    let essence = content_type.split(';').next()?.trim().to_ascii_lowercase();
    match essence.as_str() {
        "audio/mpeg" | "audio/mp3" | "audio/mpeg3" | "audio/x-mpeg" | "audio/x-mp3" => Some(Kind::Mp3),
        "audio/aac" | "audio/aacp" | "audio/x-aac" | "audio/x-aacp" | "audio/vnd.dlna.adts" => Some(Kind::Adts),
        "audio/mp4a-latm" | "audio/x-mp4a-latm" | "audio/mpeg4-generic" => Some(Kind::Latm),
        "audio/mp4" | "audio/x-m4a" | "audio/m4a" | "video/mp4" => Some(Kind::M4a),
        "audio/ogg" | "application/ogg" | "audio/vorbis" | "audio/x-vorbis" | "audio/x-ogg" => Some(Kind::Ogg),
        "audio/flac" | "audio/x-flac" => Some(Kind::Flac),
        "audio/wav" | "audio/x-wav" | "audio/wave" | "audio/vnd.wave" => Some(Kind::Wav),
        _ => None,
    }
}

/// A bare-AAC stream at an unknown offset: the first position in `head`
/// where a chain of valid frames starts decides ADTS or LOAS.
fn find_aac(head: &[u8]) -> Option<Kind> {
    (0..head.len().saturating_sub(8)).find_map(|i| {
        let rest = &head[i..];
        match rest[0] {
            0xFF if matches!(adts_frame(rest, false), Parsed::Frame(_)) => Some(Kind::Adts),
            0x56 if matches!(loas_frame(rest, false, &mut LatmState::default()), Parsed::Frame(_)) => Some(Kind::Latm),
            _ => None,
        }
    })
}

/// The container by its magic bytes, for a stream or file that starts at its
/// first byte.
fn from_magic(head: &[u8]) -> Option<Kind> {
    match head {
        [b'f', b'L', b'a', b'C', ..] => Some(Kind::Flac),
        [b'O', b'g', b'g', b'S', ..] => Some(Kind::Ogg),
        [b'R', b'I', b'F', b'F', _, _, _, _, b'W', b'A', b'V', b'E', ..] => Some(Kind::Wav),
        [_, _, _, _, b'f', b't', b'y', b'p', ..] => Some(Kind::M4a),
        [0x56, b1, ..] if b1 & 0xE0 == 0xE0 => Some(Kind::Latm),
        // 12 sync bits, then MPEG version (not the reserved 01) and the layer: 00 is ADTS
        [0xFF, b1, ..] if b1 & 0xF0 == 0xF0 && b1 & 0x06 == 0 => Some(Kind::Adts),
        [0xFF, b1, ..] if b1 & 0xE0 == 0xE0 && b1 & 0x18 != 0x08 && b1 & 0x06 != 0 => Some(Kind::Mp3),
        _ => None,
    }
}

/// What `head` (the first bytes of a file or of a live stream, which may
/// start mid-frame) and the HTTP content type say the audio is. None when
/// neither says. Magic bytes outrank the content type, since stations
/// mislabel (`audio/aacp` carries both ADTS and LOAS, `audio/mpeg` gets
/// AAC); an ID3v2 tag is looked through.
pub fn sniff(content_type: Option<&str>, head: &[u8]) -> Option<Kind> {
    let tag = id3::tag_len(head);
    if tag > 0 {
        return match head.get(tag..) {
            Some(rest) if rest.len() >= 4 => sniff(content_type, rest),
            // the audio starts past what we have: a tagged file is MP3 unless the type says otherwise
            _ => content_type.and_then(from_content_type).or(Some(Kind::Mp3)),
        };
    }
    if let Some(kind) = from_magic(head) {
        // a stream that merely starts on a frame still gets the frame chain checked below
        if !matches!(kind, Kind::Mp3 | Kind::Adts | Kind::Latm) {
            return Some(kind);
        }
    }
    let by_type = content_type.and_then(from_content_type);
    match by_type {
        Some(Kind::Mp3) => {
            // a live "mp3" stream may really be AAC; mid-stream joins are fine, the chain decides
            find_aac(head).or(Some(Kind::Mp3))
        }
        Some(Kind::Adts | Kind::Latm) => find_aac(head).or(by_type),
        Some(other) => Some(other),
        None => find_aac(head).or_else(|| from_magic(head).filter(|k| *k == Kind::Mp3)),
    }
}

#[cfg(test)]
mod tests {
    use super::super::aacstream::tests::{adts, loas, lc_bits, raw_frames};
    use super::*;

    #[test]
    fn extensions_map_to_kinds() {
        for (name, kind) in [("a.MP3", Kind::Mp3), ("a.m4a", Kind::M4a), ("a.aac", Kind::Adts), ("a.flac", Kind::Flac), ("a.ogg", Kind::Ogg), ("a.oga", Kind::Ogg), ("a.wav", Kind::Wav)] {
            assert_eq!(from_extension(Path::new(name)), Some(kind), "{name}");
        }
        assert_eq!(from_extension(Path::new("a.txt")), None);
        assert_eq!(from_extension(Path::new("noext")), None);
    }

    #[test]
    fn magic_bytes_pick_the_container() {
        assert_eq!(sniff(None, b"fLaC\0\0\0\"rest"), Some(Kind::Flac));
        assert_eq!(sniff(Some("audio/mpeg"), b"OggS\0\x02rest"), Some(Kind::Ogg), "magic outranks a wrong type");
        assert_eq!(sniff(None, b"RIFF\x24\0\0\0WAVEfmt "), Some(Kind::Wav));
        assert_eq!(sniff(None, b"\0\0\0\x20ftypM4A "), Some(Kind::M4a));
        assert_eq!(sniff(None, b"<html>"), None);
    }

    #[test]
    fn content_types_pick_the_codec_when_the_bytes_are_unhelpful() {
        assert_eq!(sniff(Some("audio/mpeg"), &[1, 2, 3, 4]), Some(Kind::Mp3));
        assert_eq!(sniff(Some("Audio/MPEG; charset=x"), &[1, 2, 3, 4]), Some(Kind::Mp3));
        assert_eq!(sniff(Some("audio/ogg"), &[1, 2, 3, 4]), Some(Kind::Ogg));
        assert_eq!(sniff(Some("audio/flac"), &[1, 2, 3, 4]), Some(Kind::Flac));
        assert_eq!(sniff(Some("audio/aacp"), &[1, 2, 3, 4]), Some(Kind::Adts));
        assert_eq!(sniff(Some("audio/mp4a-latm"), &[1, 2, 3, 4]), Some(Kind::Latm));
        assert_eq!(sniff(Some("text/html"), b"<html>"), None);
    }

    #[test]
    fn aac_streams_are_told_apart_even_when_joined_mid_frame_and_mislabelled() {
        let (_, frames) = raw_frames();
        let a = adts(&frames[..8]);
        let mut l = Vec::new();
        for f in &frames[..8] {
            l.extend(loas(f, Some(&lc_bits)));
        }
        assert_eq!(sniff(None, &a), Some(Kind::Adts));
        assert_eq!(sniff(None, &l), Some(Kind::Latm));
        assert_eq!(sniff(Some("audio/aacp"), &a[37..]), Some(Kind::Adts), "mid-stream join");
        assert_eq!(sniff(Some("audio/aacp"), &l[37..]), Some(Kind::Latm), "aacp carrying LOAS");
        assert_eq!(sniff(Some("audio/mpeg"), &a[11..]), Some(Kind::Adts), "AAC labelled MP3");
        assert_eq!(sniff(Some("audio/aac"), &l), Some(Kind::Latm));
    }

    #[test]
    fn id3_tags_are_looked_through() {
        let mut data = b"ID3\x03\0\0\0\0\0\x04abcd".to_vec();
        data.extend(b"fLaC....");
        assert_eq!(sniff(None, &data), Some(Kind::Flac));
        let tag_only = b"ID3\x03\0\0\0\0\x10\0".to_vec();
        assert_eq!(sniff(None, &tag_only), Some(Kind::Mp3));
    }
}
