//! ID3v2 tag reader (versions 2.2, 2.3 and 2.4) for the Music library:
//! title, artist, album, album artist, track number, year and the cover
//! picture, with unsynchronisation undone at the tag (2.3) or frame (2.4)
//! level. Everything else is skipped without being decoded.

use super::audiofile::Tags;
use super::chapters::Chapter;

/// The byte size of the ID3v2 tag at the start of `bytes` (header
/// included, footer included for 2.4), or 0 when there is none; the audio
/// stream starts there.
pub fn tag_len(bytes: &[u8]) -> usize {
    let Some(h) = Header::parse(bytes) else { return 0 };
    10 + h.size + if h.footer { 10 } else { 0 }
}

/// Parses the ID3v2 tag at the start of `bytes`, if any.
pub fn parse(bytes: &[u8]) -> Option<Tags> {
    let h = Header::parse(bytes)?;
    let body = bytes.get(10..10 + h.size).unwrap_or(&bytes[10.min(bytes.len())..]);
    let body = if h.unsync && h.version <= 3 { resync(body) } else { body.to_vec() };
    let mut p = 0;
    if h.extended {
        // 2.3: the size excludes its own 4 bytes; 2.4: syncsafe and inclusive
        let size = if h.version == 4 { syncsafe(&body) } else { be32(&body) + 4 };
        p += size.min(body.len());
    }
    let mut tags = Tags::default();
    let mut chapters = Vec::new();
    let mut tocs = Vec::new();
    while let Some(frame) = Frame::read(&body, &mut p, h) {
        if &frame.id == b"CHAP" {
            chapters.extend(chap(&frame.data, h));
        } else if &frame.id == b"CTOC" {
            tocs.extend(ctoc(&frame.data));
        } else {
            apply(&mut tags, frame);
        }
    }
    tags.chapters = order_chapters(chapters, &tocs);
    Some(tags)
}

/// A CHAP frame: the element id (a NUL-terminated string), the start and end times in milliseconds, two byte
/// offsets (unused here, 0xFFFFFFFF when absent) and sub-frames with the chapter's title (TIT2) and picture (APIC).
fn chap(d: &[u8], h: Header) -> Option<(String, Chapter)> {
    let nul = d.iter().position(|&b| b == 0)?;
    let id = String::from_utf8_lossy(&d[..nul]).into_owned();
    let times = d.get(nul + 1..nul + 17)?;
    let start = be32(&times[0..4]) as u64;
    let end = be32(&times[4..8]) as u64;
    let subs = d.get(nul + 17..).unwrap_or(&[]);
    let (mut title, mut image) = (None, None);
    let mut p = 0;
    while let Some(f) = Frame::read(subs, &mut p, h) {
        match &f.id {
            b"TIT2" if title.is_none() => title = decode_text(&f.data),
            b"APIC" if image.is_none() => image = picture(&f.data, false),
            _ => {}
        }
    }
    Some((id, Chapter { start_ms: start, end_ms: Some(end), title, image }))
}

/// A CTOC frame: the element id (not kept), the flags (bit 1 marks a top-level table of contents), the child element ids in
/// their play order. Its sub-frames are not needed.
fn ctoc(d: &[u8]) -> Option<Toc> {
    let nul = d.iter().position(|&b| b == 0)?;
    let flags = *d.get(nul + 1)?;
    let count = *d.get(nul + 2)? as usize;
    let mut p = nul + 3;
    let mut children = Vec::with_capacity(count);
    for _ in 0..count {
        let rest = d.get(p..)?;
        let n = rest.iter().position(|&b| b == 0)?;
        children.push(String::from_utf8_lossy(&rest[..n]).into_owned());
        p += n + 1;
    }
    Some(Toc { top_level: flags & 0x02 != 0, children })
}

/// A parsed CTOC frame.
struct Toc {
    top_level: bool,
    children: Vec<String>,
}

/// The chapters in play order: the children of the first top-level table of contents, in its order, with any chapter
/// it does not list following by start time. Without a table of contents the chapters sort by start time.
fn order_chapters(mut chapters: Vec<(String, Chapter)>, tocs: &[Toc]) -> Vec<Chapter> {
    chapters.sort_by_key(|(_, c)| c.start_ms);
    let mut ordered = Vec::with_capacity(chapters.len());
    if let Some(toc) = tocs.iter().find(|t| t.top_level) {
        for id in &toc.children {
            if let Some(i) = chapters.iter().position(|(known, _)| known == id) {
                ordered.push(chapters.remove(i));
            }
        }
    }
    ordered.extend(chapters);
    ordered.into_iter().map(|(_, c)| c).collect()
}

#[derive(Clone, Copy)]
struct Header {
    version: u8,
    size: usize,
    unsync: bool,
    extended: bool,
    footer: bool,
}

impl Header {
    fn parse(b: &[u8]) -> Option<Header> {
        if b.len() < 10 || &b[..3] != b"ID3" || !(2..=4).contains(&b[3]) || b[6..10].iter().any(|&x| x & 0x80 != 0) {
            return None;
        }
        let flags = b[5];
        Some(Header { version: b[3], size: syncsafe(&b[6..]), unsync: flags & 0x80 != 0, extended: b[3] >= 3 && flags & 0x40 != 0, footer: b[3] == 4 && flags & 0x10 != 0 })
    }
}

struct Frame<'a> {
    id: [u8; 4],
    data: std::borrow::Cow<'a, [u8]>,
}

impl<'a> Frame<'a> {
    /// The frame at `*p`, advancing past it; None at padding or the end.
    fn read(body: &'a [u8], p: &mut usize, h: Header) -> Option<Frame<'a>> {
        let (id_len, size_len, flag_len) = if h.version == 2 { (3, 3, 0) } else { (4, 4, 2) };
        let head = body.get(*p..*p + id_len + size_len + flag_len)?;
        if head[0] == 0 {
            return None;
        }
        let mut id = [b' '; 4];
        id[..id_len].copy_from_slice(&head[..id_len]);
        let size = match h.version {
            2 => (head[3] as usize) << 16 | (head[4] as usize) << 8 | head[5] as usize,
            3 => be32(&head[4..]),
            _ => syncsafe(&head[4..]),
        };
        let flags = if flag_len == 2 { u16::from_be_bytes([head[8], head[9]]) } else { 0 };
        let start = *p + id_len + size_len + flag_len;
        let raw = body.get(start..start + size)?;
        *p = start + size;
        let (compressed, encrypted, grouped, unsync, data_len) = match h.version {
            3 => (flags & 0x80 != 0, flags & 0x40 != 0, flags & 0x20 != 0, false, false),
            4 => (flags & 0x08 != 0, flags & 0x04 != 0, flags & 0x40 != 0, flags & 0x02 != 0 || h.unsync, flags & 0x01 != 0),
            _ => (false, false, false, false, false),
        };
        if compressed || encrypted {
            return Some(Frame { id: *b"    ", data: std::borrow::Cow::Borrowed(&[]) });
        }
        let mut skip = 0;
        if grouped {
            skip += 1;
        }
        if data_len {
            skip += 4;
        }
        if h.version == 3 && compressed {
            skip += 4;
        }
        let raw = raw.get(skip..).unwrap_or(&[]);
        let data = if unsync { std::borrow::Cow::Owned(resync(raw)) } else { std::borrow::Cow::Borrowed(raw) };
        Some(Frame { id, data })
    }
}

fn apply(tags: &mut Tags, f: Frame) {
    let text = || decode_text(&f.data);
    match &f.id {
        b"TIT2" | b"TT2 " => tags.title = text(),
        b"TPE1" | b"TP1 " => tags.artist = text(),
        b"TALB" | b"TAL " => tags.album = text(),
        b"TPE2" | b"TP2 " => tags.album_artist = text(),
        b"TRCK" | b"TRK " => tags.track = text().and_then(|t| t.split('/').next()?.trim().parse().ok()),
        b"TYER" | b"TDRC" | b"TYE " => tags.year = text().and_then(|t| t.get(..4)?.parse().ok()),
        b"APIC" | b"PIC " => {
            if tags.cover.is_none() {
                tags.cover = picture(&f.data, f.id[3] == b' ');
            }
        }
        b"TXXX" | b"TXX " => {
            if tags.replaygain_db.is_none() {
                tags.replaygain_db = user_text(&f.data, "REPLAYGAIN_TRACK_GAIN").and_then(|v| super::audiofile::parse_replaygain_db(&v));
            }
        }
        b"USLT" | b"ULT " => {
            if tags.lyrics.is_none() {
                tags.lyrics = lyrics(&f.data);
            }
        }
        _ => {}
    }
}

/// The picture bytes of an APIC (or 2.2 PIC) frame body.
fn picture(d: &[u8], v22: bool) -> Option<Vec<u8>> {
    let enc = *d.first()?;
    let mut p = 1;
    if v22 {
        p += 3;
    } else {
        p += d.get(p..)?.iter().position(|&b| b == 0)? + 1;
    }
    p += 1; // picture type
    p += terminator(d.get(p..)?, enc)?;
    let pic = d.get(p..)?;
    (!pic.is_empty()).then(|| pic.to_vec())
}

/// The lyrics text of a USLT (2.2 ULT) frame body: encoding, three language bytes, the content descriptor
/// (terminated in the frame's encoding), then the lyrics. The text is decoded with the frame's own encoding.
fn lyrics(d: &[u8]) -> Option<String> {
    let enc = *d.first()?;
    let rest = d.get(4..)?;
    let skip = terminator(rest, enc)?;
    let body = rest.get(skip..)?;
    let mut text = vec![enc];
    text.extend_from_slice(body);
    decode_text(&text).filter(|s| !s.trim().is_empty())
}

/// The value of a TXXX (2.2 TXX) user text frame whose description is `want` (case aside). The frame body is the
/// encoding byte, the description (terminated in the frame's encoding), then the value.
fn user_text(d: &[u8], want: &str) -> Option<String> {
    let enc = *d.first()?;
    let rest = d.get(1..)?;
    let skip = terminator(rest, enc)?;
    let desc = decode_text(&[&[enc][..], &rest[..skip]].concat())?;
    if !desc.eq_ignore_ascii_case(want) {
        return None;
    }
    decode_text(&[&[enc][..], rest.get(skip..)?].concat())
}

/// Length of the string at the start of `d` including its terminator, for
/// the given encoding (two NULs for UTF-16); the whole slice if unterminated.
fn terminator(d: &[u8], enc: u8) -> Option<usize> {
    if enc == 1 || enc == 2 {
        let i = d.chunks_exact(2).position(|c| c == [0, 0]).map(|i| i * 2 + 2);
        Some(i.unwrap_or(d.len()))
    } else {
        Some(d.iter().position(|&b| b == 0).map_or(d.len(), |i| i + 1))
    }
}

/// A text frame body (encoding byte then the string); only the first of a
/// 2.4 NUL-separated list is kept.
fn decode_text(d: &[u8]) -> Option<String> {
    let (&enc, rest) = d.split_first()?;
    let s = match enc {
        0 => rest.iter().take_while(|&&b| b != 0).map(|&b| b as char).collect(),
        3 => String::from_utf8_lossy(rest.split(|&b| b == 0).next().unwrap_or(&[])).into_owned(),
        1 | 2 => {
            let (be, body) = match rest {
                [0xFF, 0xFE, body @ ..] => (false, body),
                [0xFE, 0xFF, body @ ..] => (true, body),
                body => (enc == 2, body),
            };
            let units: Vec<u16> = body.chunks_exact(2).map(|c| if be { u16::from_be_bytes([c[0], c[1]]) } else { u16::from_le_bytes([c[0], c[1]]) }).take_while(|&u| u != 0).collect();
            String::from_utf16_lossy(&units)
        }
        _ => return None,
    };
    let s = s.trim().to_string();
    (!s.is_empty()).then_some(s)
}

/// Undoes unsynchronisation: every `FF 00` becomes `FF`.
fn resync(d: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(d.len());
    let mut i = 0;
    while i < d.len() {
        out.push(d[i]);
        if d[i] == 0xFF && d.get(i + 1) == Some(&0) {
            i += 1;
        }
        i += 1;
    }
    out
}

/// Zero bytes left after the frames, so a later edit can grow the tag in place.
const PADDING: usize = 512;

/// The ID3v2.4 tag that stores `tags`: title, artist, album, album artist, track, year, lyrics (USLT) and cover (APIC),
/// UTF-8 text, then padding. Fields that are `None` are left out. The audio is not included.
pub fn build(tags: &Tags) -> Vec<u8> {
    build_with(tags, &[])
}

/// `build` with `kept` (whole frames, already in the v2.4 layout) placed after the frames `Tags` writes.
fn build_with(tags: &Tags, kept: &[u8]) -> Vec<u8> {
    let mut frames = Vec::new();
    let texts = [(b"TIT2", &tags.title), (b"TPE1", &tags.artist), (b"TALB", &tags.album), (b"TPE2", &tags.album_artist)];
    for (id, value) in texts {
        if let Some(text) = value {
            push_frame(&mut frames, id, &text_body(text));
        }
    }
    if let Some(track) = tags.track {
        push_frame(&mut frames, b"TRCK", &text_body(&track.to_string()));
    }
    if let Some(year) = tags.year {
        push_frame(&mut frames, b"TDRC", &text_body(&format!("{year:04}")));
    }
    if let Some(lyrics) = &tags.lyrics {
        // encoding UTF-8, language, empty content descriptor, then the text
        let mut body = vec![3];
        body.extend(b"eng\0");
        body.extend(lyrics.as_bytes());
        push_frame(&mut frames, b"USLT", &body);
    }
    if let Some(cover) = &tags.cover {
        let mime: &[u8] = if cover.starts_with(&[0x89, b'P', b'N', b'G']) { b"image/png" } else { b"image/jpeg" };
        // encoding, MIME type, picture type 3 (front cover), empty description, then the picture
        let mut body = vec![3];
        body.extend(mime);
        body.extend([0, 3, 0]);
        body.extend(cover);
        push_frame(&mut frames, b"APIC", &body);
    }
    frames.extend_from_slice(kept);
    let mut out = b"ID3\x04\x00\x00".to_vec();
    out.extend(syncsafe_bytes(frames.len() + PADDING));
    out.extend(frames);
    out.resize(out.len() + PADDING, 0);
    out
}

/// The file with its ID3v2 tag replaced: the fields `Tags` knows are written, every other frame of the old tag
/// (genre, comments, chapters, ratings and the rest) is copied over unchanged, and the audio frames and anything after
/// them (an ID3v1 tag) are kept. None when the old tag cannot be copied safely (version 2.2, or a compressed or
/// encrypted frame), so a caller refuses the write rather than lose data.
pub fn replace_tag(file: &[u8], tags: &Tags) -> Option<Vec<u8>> {
    let kept = kept_frames(file, tags)?;
    let audio = &file[tag_len(file).min(file.len())..];
    let mut out = build_with(tags, &kept);
    out.extend_from_slice(audio);
    Some(out)
}

/// The frames of the old tag that `build` does not write, re-encoded as v2.4 frames with their bodies unchanged.
fn kept_frames(file: &[u8], tags: &Tags) -> Option<Vec<u8>> {
    let Some(h) = Header::parse(file) else { return Some(Vec::new()) };
    if h.version == 2 {
        return None;
    }
    let body = file.get(10..10 + h.size)?;
    let body = if h.unsync && h.version <= 3 { resync(body) } else { body.to_vec() };
    let mut p = 0;
    if h.extended {
        let size = if h.version == 4 { syncsafe(&body) } else { be32(&body) + 4 };
        p += size.min(body.len());
    }
    // the text frames `build` always owns; the year frames of v2.3 give way to TDRC when a year is written
    let owned: &[&[u8; 4]] = &[b"TIT2", b"TPE1", b"TALB", b"TPE2", b"TRCK", b"TDRC"];
    let mut out = Vec::new();
    while let Some(frame) = Frame::read(&body, &mut p, h) {
        let id = &frame.id;
        if id == b"    " {
            // a compressed or encrypted frame: it cannot be copied without its flags
            return None;
        }
        let replaced = owned.contains(&id)
            || (tags.year.is_some() && (id == b"TYER" || id == b"TDAT" || id == b"TIME"))
            || (tags.lyrics.is_some() && id == b"USLT")
            || (tags.cover.is_some() && id == b"APIC");
        if !replaced {
            push_frame(&mut out, id, &frame.data);
        }
    }
    Some(out)
}

fn push_frame(out: &mut Vec<u8>, id: &[u8; 4], body: &[u8]) {
    out.extend_from_slice(id);
    out.extend(syncsafe_bytes(body.len()));
    out.extend([0, 0]);
    out.extend_from_slice(body);
}

fn text_body(text: &str) -> Vec<u8> {
    let mut body = vec![3];
    body.extend(text.as_bytes());
    body
}

fn syncsafe_bytes(n: usize) -> [u8; 4] {
    let n = n as u32;
    [(n >> 21) as u8 & 0x7f, (n >> 14) as u8 & 0x7f, (n >> 7) as u8 & 0x7f, n as u8 & 0x7f]
}

fn syncsafe(b: &[u8]) -> usize {
    b.iter().take(4).fold(0, |a, &x| a << 7 | (x & 0x7f) as usize)
}

fn be32(b: &[u8]) -> usize {
    b.iter().take(4).fold(0, |a, &x| a << 8 | x as usize)
}

#[cfg(test)]
mod tests {
    use super::*;

    const V23: &[u8] = include_bytes!("testdata/tone_id3v23.mp3");
    const V24: &[u8] = include_bytes!("testdata/tone_id3v24.mp3");

    fn frame(id: &[u8; 4], body: &[u8], v4: bool, flags: u16) -> Vec<u8> {
        let mut f = id.to_vec();
        let n = body.len() as u32;
        if v4 {
            f.extend([(n >> 21) as u8 & 0x7f, (n >> 14) as u8 & 0x7f, (n >> 7) as u8 & 0x7f, n as u8 & 0x7f]);
        } else {
            f.extend(n.to_be_bytes());
        }
        f.extend(flags.to_be_bytes());
        f.extend_from_slice(body);
        f
    }

    fn tag(version: u8, flags: u8, body: &[u8]) -> Vec<u8> {
        let mut t = b"ID3".to_vec();
        let n = body.len() as u32;
        t.extend([version, 0, flags, (n >> 21) as u8 & 0x7f, (n >> 14) as u8 & 0x7f, (n >> 7) as u8 & 0x7f, n as u8 & 0x7f]);
        t.extend_from_slice(body);
        t
    }

    #[test]
    fn reads_ffmpegs_v23_tag_with_cover_and_track() {
        let t = parse(V23).unwrap();
        assert_eq!(t.title.as_deref(), Some("Tone Song"));
        assert_eq!(t.artist.as_deref(), Some("Fixture Artist"));
        assert_eq!(t.album.as_deref(), Some("Test Album"));
        assert_eq!(t.track, Some(3));
        let cover = t.cover.unwrap();
        assert_eq!(&cover[..2], &[0xFF, 0xD8], "jpeg cover");
        assert_eq!(tag_len(V23), 476);
        assert_eq!(&V23[476..478], &[0xFF, 0xFB], "audio starts right after the tag");
    }

    #[test]
    fn reads_ffmpegs_v24_utf8_tag() {
        let t = parse(V24).unwrap();
        assert_eq!(t.title.as_deref(), Some("Höfði"));
        assert_eq!(t.artist.as_deref(), Some("Bjórk"));
        assert_eq!(t.album.as_deref(), Some("Album Two"));
        assert!(t.cover.is_none() && t.track.is_none());
        assert_eq!(tag_len(V24), 102);
    }

    #[test]
    fn txxx_replaygain_is_read_from_its_description_and_value() {
        let mut body = vec![0u8];
        body.extend(b"replaygain_track_gain\0-6.54 dB");
        let t = parse(&tag(3, 0, &frame(b"TXXX", &body, false, 0))).unwrap();
        assert_eq!(t.replaygain_db, Some(-6.54));
        let mut other = vec![0u8];
        other.extend(b"REPLAYGAIN_ALBUM_GAIN\0-2.00 dB");
        assert_eq!(parse(&tag(3, 0, &frame(b"TXXX", &other, false, 0))).unwrap().replaygain_db, None);
    }

    #[test]
    fn no_tag_means_nothing() {
        assert!(parse(b"\xFF\xFB\x90\x00").is_none());
        assert_eq!(tag_len(b"\xFF\xFB"), 0);
        assert!(parse(b"ID3").is_none());
        assert!(parse(&tag(5, 0, &[])).is_none(), "unknown version");
    }

    #[test]
    fn v24_unsynchronised_frame_is_resynced_and_the_data_length_skipped() {
        // "A\u{ff}B" in ISO-8859-1 becomes A FF 00 B on the wire
        let body = [0u8, b'A', 0xFF, 0x00, b'B'];
        let mut f = vec![0, 0, 0, 3]; // data length indicator (syncsafe 3)
        f.extend(body);
        let t = parse(&tag(4, 0, &frame(b"TIT2", &f, true, 0x0003))).unwrap();
        assert_eq!(t.title.as_deref(), Some("AÿB"));
        // the same with the tag-level flag and no frame flag
        let t = parse(&tag(4, 0x80, &frame(b"TIT2", &body, true, 0))).unwrap();
        assert_eq!(t.title.as_deref(), Some("AÿB"));
    }

    #[test]
    fn v23_tag_level_unsync_and_extended_header() {
        let mut body = vec![0, 0, 0, 6, 0, 0, 0, 0, 0, 0]; // ext header: size 6 + 6 bytes
        body.extend(frame(b"TPE1", &[0, b'X', 0xFF, b'Y'], false, 0));
        // 2.3 unsynchronises the assembled tag: a NUL after every FF
        let unsynced: Vec<u8> = body.iter().flat_map(|&b| if b == 0xFF { vec![0xFF, 0x00] } else { vec![b] }).collect();
        let t = parse(&tag(3, 0xC0, &unsynced)).unwrap();
        assert_eq!(t.artist.as_deref(), Some("XÿY"));
    }

    #[test]
    fn utf16_with_and_without_bom_and_null_separated_lists() {
        let le: Vec<u8> = [1u8, 0xFF, 0xFE].into_iter().chain("Þór".encode_utf16().flat_map(|u| u.to_le_bytes())).chain([0, 0]).collect();
        let t = parse(&tag(3, 0, &frame(b"TALB", &le, false, 0))).unwrap();
        assert_eq!(t.album.as_deref(), Some("Þór"));
        let be: Vec<u8> = [2u8].into_iter().chain("Ab".encode_utf16().flat_map(|u| u.to_be_bytes())).collect();
        let t = parse(&tag(4, 0, &frame(b"TALB", &be, true, 0))).unwrap();
        assert_eq!(t.album.as_deref(), Some("Ab"));
        let t = parse(&tag(4, 0, &frame(b"TPE1", b"\x03One\0Two", true, 0))).unwrap();
        assert_eq!(t.artist.as_deref(), Some("One"));
        let t = parse(&tag(4, 0, &frame(b"TRCK", b"\x03 7/12", true, 0))).unwrap();
        assert_eq!(t.track, Some(7));
        let t = parse(&tag(4, 0, &frame(b"TDRC", b"\x032019-05-01", true, 0))).unwrap();
        assert_eq!(t.year, Some(2019));
    }

    #[test]
    fn apic_with_utf16_description_and_v22_pic() {
        let mut apic = vec![1u8];
        apic.extend(b"image/png\0");
        apic.push(3);
        apic.extend([0xFF, 0xFE, b'c', 0, 0, 0]);
        apic.extend(b"\x89PNG!");
        let t = parse(&tag(3, 0, &frame(b"APIC", &apic, false, 0))).unwrap();
        assert_eq!(t.cover.as_deref(), Some(&b"\x89PNG!"[..]));
        let mut body = b"PIC".to_vec();
        body.extend([0, 0, 10]);
        body.extend([0u8]);
        body.extend(b"JPG");
        body.push(0);
        body.push(0);
        body.extend(b"\xFF\xD8\xFF\xE0");
        let t = parse(&tag(2, 0, &body)).unwrap();
        assert_eq!(t.cover.as_deref(), Some(&b"\xFF\xD8\xFF\xE0"[..]));
    }

    #[test]
    fn uslt_lyrics_skip_the_language_and_descriptor() {
        let mut body = vec![0u8];
        body.extend(b"eng");
        body.push(0); // empty descriptor
        body.extend(b"la\nla");
        let t = parse(&tag(3, 0, &frame(b"USLT", &body, false, 0))).unwrap();
        assert_eq!(t.lyrics.as_deref(), Some("la\nla"));
        let mut utf16 = vec![1u8];
        utf16.extend(b"eng");
        utf16.extend([0xFF, 0xFE, 0, 0]);
        utf16.extend("Hi".encode_utf16().flat_map(|u| u.to_le_bytes()));
        let t = parse(&tag(4, 0, &frame(b"USLT", &utf16, true, 0))).unwrap();
        assert_eq!(t.lyrics.as_deref(), Some("Hi"));
    }

    #[test]
    fn compressed_or_truncated_frames_are_skipped() {
        let mut body = frame(b"TIT2", &[0, b'Z'], false, 0x0080);
        body.extend(frame(b"TALB", &[0, b'Q'], false, 0));
        body.extend(b"TPE1\0\0\xFF\xFF\0\0");
        let t = parse(&tag(3, 0, &body)).unwrap();
        assert_eq!(t.title, None, "compressed frame ignored");
        assert_eq!(t.album.as_deref(), Some("Q"));
        assert_eq!(t.artist, None, "a frame running past the tag ends parsing");
    }

    #[test]
    fn chap_and_ctoc_frames_give_ordered_chapters_with_titles_and_pictures() {
        // CHAP "c2" 4000-9000 ms with a TIT2 sub-frame; CHAP "c1" 0-4000 ms with a PNG APIC; the CTOC lists c2 first
        let mut title = vec![3u8];
        title.extend(b"Second");
        let mut apic = vec![3u8];
        apic.extend(b"image/png\0\x03\0");
        apic.extend(b"\x89PNG!");
        let chap = |id: &[u8], start: u32, end: u32, subs: &[u8]| {
            let mut b = id.to_vec();
            b.push(0);
            b.extend(start.to_be_bytes());
            b.extend(end.to_be_bytes());
            b.extend([0xFF; 8]);
            b.extend_from_slice(subs);
            b
        };
        let mut toc = b"toc\0".to_vec();
        toc.push(0x03); // top-level, ordered
        toc.push(2);
        toc.extend(b"c2\0c1\0");
        let mut body = frame(b"CTOC", &toc, true, 0);
        body.extend(frame(b"CHAP", &chap(b"c2", 4000, 9000, &frame(b"TIT2", &title, true, 0)), true, 0));
        body.extend(frame(b"CHAP", &chap(b"c1", 0, 4000, &frame(b"APIC", &apic, true, 0)), true, 0));
        let t = parse(&tag(4, 0, &body)).unwrap();
        assert_eq!(t.chapters.len(), 2);
        assert_eq!(t.chapters[0], Chapter { start_ms: 4000, end_ms: Some(9000), title: Some("Second".into()), image: None });
        assert_eq!(t.chapters[1].start_ms, 0);
        assert_eq!(t.chapters[1].image.as_deref(), Some(&b"\x89PNG!"[..]));
        // without a table of contents the chapters sort by start time
        let t = parse(&tag(4, 0, &frame(b"CHAP", &chap(b"x", 5, 6, &[]), true, 0))).unwrap();
        assert_eq!(t.chapters, vec![Chapter { start_ms: 5, end_ms: Some(6), title: None, image: None }]);
    }

    #[test]
    fn written_tag_round_trips_and_keeps_the_audio_bytes() {
        let tags = Tags {
            title: Some("Höfði Round".into()),
            artist: Some("Band".into()),
            album: Some("Record".into()),
            album_artist: Some("Various".into()),
            track: Some(4),
            year: Some(2020),
            cover: Some(b"\x89PNG-cover".to_vec()),
            lyrics: Some("line one\nline two".into()),
            chapters: Vec::new(),
            replaygain_db: None,
        };
        let out = replace_tag(V24, &tags).unwrap();
        assert_eq!(parse(&out).unwrap(), tags, "write, then read back");
        // the audio (everything after the old tag) is byte-identical and follows the new tag
        assert_eq!(&out[tag_len(&out)..], &V24[tag_len(V24)..]);
        // replacing again keeps a single tag
        let again = replace_tag(&out, &Tags { title: Some("Again".into()), ..Tags::default() }).unwrap();
        assert_eq!(parse(&again).unwrap().title.as_deref(), Some("Again"));
        assert_eq!(&again[tag_len(&again)..], &V24[tag_len(V24)..]);
        // the padding is inside the tag, not after it
        assert_eq!(tag_len(&build(&tags)), build(&tags).len());
    }

    #[test]
    fn resync_only_touches_ff_00_pairs() {
        assert_eq!(resync(&[0xFF, 0x00, 0xFF, 0x00, 0x01, 0xFF]), [0xFF, 0xFF, 0x01, 0xFF]);
        assert_eq!(resync(&[0xFF, 0xE0]), [0xFF, 0xE0]);
    }

    #[test]
    fn a_tag_edit_keeps_the_frames_it_does_not_write() {
        // an old v2.4 tag with a title and a genre (TCON) the writer does not model
        let mut frames = Vec::new();
        push_frame(&mut frames, b"TIT2", &text_body("Old"));
        push_frame(&mut frames, b"TCON", &text_body("Jazz"));
        let mut file = b"ID3\x04\x00\x00".to_vec();
        file.extend(syncsafe_bytes(frames.len()));
        file.extend(&frames);
        file.extend([0xff, 0xfb, 0x90, 0x00]);
        let out = replace_tag(&file, &Tags { title: Some("New".into()), ..Tags::default() }).unwrap();
        let tags = parse(&out).unwrap();
        assert_eq!(tags.title.as_deref(), Some("New"));
        assert!(out.windows(4).any(|w| w == b"TCON"), "the genre frame is kept");
        assert_eq!(out.windows(3).filter(|w| *w == b"Old").count(), 0, "the old title is replaced");
        assert!(out.ends_with(&[0xff, 0xfb, 0x90, 0x00]));
    }

    #[test]
    fn a_tag_with_a_v22_header_is_not_rewritten() {
        let mut file = b"ID3\x02\x00\x00\x00\x00\x00\x00".to_vec();
        file.extend([0xff, 0xfb]);
        assert!(replace_tag(&file, &Tags::default()).is_none());
    }
}
