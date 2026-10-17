//! Chapter lists of a file. An MP4 file carries either a Nero `chpl` list in `moov/udta`, or a QuickTime text track
//! that another track names through `tref/chap`; the text track's samples are length-prefixed titles at the times of
//! the sample table. ID3 chapters (`CHAP` frames) are read by `id3`, which uses this module's [`Chapter`].

use super::mp4::{child, children, sample_table};
use super::source::Source;

/// One chapter: its span in milliseconds, its title and an optional encoded picture (JPEG or PNG).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Chapter {
    pub start_ms: u64,
    /// The end in milliseconds; `None` when the chapter runs to the end of the file.
    pub end_ms: Option<u64>,
    pub title: Option<String>,
    pub image: Option<Vec<u8>>,
}

/// Fills each missing end from the start of the chapter after it (lists that give only starts).
pub(crate) fn fill_ends(list: &mut [Chapter]) {
    let starts: Vec<u64> = list.iter().map(|c| c.start_ms).collect();
    for (i, c) in list.iter_mut().enumerate() {
        if c.end_ms.is_none() {
            c.end_ms = starts.get(i + 1).copied();
        }
    }
}

/// The chapters of an MP4 file's movie box payload: the Nero list when there is one, else the QuickTime chapter track.
/// Samples are read through `src` only when a text track is needed; a read that fails ends the list early.
pub fn from_moov(moov: &[u8], src: &mut dyn Source) -> Vec<Chapter> {
    if let Some(list) = child(moov, b"udta").and_then(|u| child(u, b"chpl")) {
        let chapters = nero(list);
        if !chapters.is_empty() {
            return chapters;
        }
    }
    text_track(moov, src)
}

/// The chapters of a Nero `chpl` box body: version, flags, a reserved word when the version is not 0, the chapter count
/// (one byte), then per chapter a start time in 100 ns units (8 bytes), a title length (one byte) and the UTF-8 title.
pub fn nero(body: &[u8]) -> Vec<Chapter> {
    let mut out = Vec::new();
    let Some(&version) = body.first() else { return out };
    let mut p = 4;
    if version != 0 {
        p += 4;
    }
    let Some(&count) = body.get(p) else { return out };
    p += 1;
    for _ in 0..count {
        let Some(t) = body.get(p..p + 8) else { break };
        let start = t.iter().fold(0u64, |a, &x| a << 8 | x as u64);
        let Some(&len) = body.get(p + 8) else { break };
        let title = body.get(p + 9..p + 9 + len as usize).map(|s| String::from_utf8_lossy(s).trim().to_string()).filter(|s| !s.is_empty());
        out.push(Chapter { start_ms: start / 10_000, end_ms: None, title, image: None });
        p += 9 + len as usize;
    }
    fill_ends(&mut out);
    out
}

/// The chapters of the QuickTime text track that a track names in `tref/chap`.
fn text_track(moov: &[u8], src: &mut dyn Source) -> Vec<Chapter> {
    let traks: Vec<&[u8]> = children(moov).filter(|(k, _)| k == b"trak").map(|(_, b)| b).collect();
    let Some(refs) = traks.iter().find_map(|t| child(t, b"tref").and_then(|r| child(r, b"chap"))) else { return Vec::new() };
    let Some(track) = refs.get(..4).map(be_u32) else { return Vec::new() };
    let Some(trak) = traks.iter().copied().find(|t| track_id(t) == Some(track)) else { return Vec::new() };
    let Some(stbl) = child(trak, b"mdia").and_then(|m| child(m, b"minf")).and_then(|m| child(m, b"stbl")) else { return Vec::new() };
    let Some(timescale) = timescale(trak) else { return Vec::new() };
    let Ok(samples) = sample_table(stbl) else { return Vec::new() };
    let mut out = Vec::new();
    for s in samples {
        if s.size > 1 << 16 {
            continue;
        }
        let mut buf = vec![0u8; s.size as usize];
        if src.read_at(s.offset, &mut buf).is_err() {
            break;
        }
        let start_ms = s.dts.saturating_mul(1000) / timescale.max(1) as u64;
        out.push(Chapter { start_ms, end_ms: None, title: sample_title(&buf), image: None });
    }
    fill_ends(&mut out);
    out
}

/// A text sample: a 16-bit length, then the title in UTF-8 or (with a byte order mark) UTF-16 big-endian.
fn sample_title(b: &[u8]) -> Option<String> {
    let len = u16::from_be_bytes([*b.first()?, *b.get(1)?]) as usize;
    let raw = b.get(2..(2 + len).min(b.len()))?;
    let text = match raw {
        [0xFE, 0xFF, body @ ..] => {
            let units: Vec<u16> = body.chunks_exact(2).map(|c| u16::from_be_bytes([c[0], c[1]])).collect();
            String::from_utf16_lossy(&units)
        }
        _ => String::from_utf8_lossy(raw).into_owned(),
    };
    let text = text.trim().to_string();
    (!text.is_empty()).then_some(text)
}

fn be_u32(b: &[u8]) -> u32 {
    b.iter().fold(0u32, |a, &x| a << 8 | x as u32)
}

/// The track id of a `trak` (`tkhd`, version 0 or 1).
fn track_id(trak: &[u8]) -> Option<u32> {
    let tkhd = child(trak, b"tkhd")?;
    let at = if tkhd.first() == Some(&1) { 20 } else { 12 };
    Some(be_u32(tkhd.get(at..at + 4)?))
}

/// The media timescale of a `trak` (`mdhd`, version 0 or 1).
fn timescale(trak: &[u8]) -> Option<u32> {
    let mdhd = child(child(trak, b"mdia")?, b"mdhd")?;
    let at = if mdhd.first() == Some(&1) { 20 } else { 12 };
    Some(be_u32(mdhd.get(at..at + 4)?))
}

#[cfg(test)]
mod tests {
    use super::super::source::MemSource;
    use super::*;

    fn full(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut b = ((body.len() + 8) as u32).to_be_bytes().to_vec();
        b.extend_from_slice(kind);
        b.extend_from_slice(body);
        b
    }

    #[test]
    fn nero_chpl_reads_starts_titles_and_ends() {
        // version 1 (reserved word present), count 2
        let mut body = vec![1, 0, 0, 0, 0, 0, 0, 0, 2];
        body.extend(0u64.to_be_bytes());
        body.push(3);
        body.extend(b"One");
        body.extend(30_000_000u64.to_be_bytes()); // 3 s in 100 ns units
        body.push(0); // untitled
        let list = nero(&body);
        assert_eq!(list.len(), 2);
        assert_eq!(list[0], Chapter { start_ms: 0, end_ms: Some(3000), title: Some("One".into()), image: None });
        assert_eq!(list[1], Chapter { start_ms: 3000, end_ms: None, title: None, image: None });
        assert!(nero(&[0, 0, 0, 0, 5]).is_empty(), "a count past the body ends the list");
    }

    /// The stbl of the text track: two samples ("Intr" as UTF-8, "Ok" as UTF-16 BE) at dts 0 and 1000 ms, one chunk
    /// starting at `data_at`.
    fn text_stbl(data_at: u32, second_len: u32) -> Vec<u8> {
        let mut stsz = vec![0u8; 4];
        stsz.extend(0u32.to_be_bytes());
        stsz.extend(2u32.to_be_bytes());
        stsz.extend(6u32.to_be_bytes());
        stsz.extend(second_len.to_be_bytes());
        let mut stts = vec![0u8; 4];
        stts.extend(1u32.to_be_bytes());
        stts.extend(2u32.to_be_bytes());
        stts.extend(1000u32.to_be_bytes());
        let mut stco = vec![0u8; 4];
        stco.extend(1u32.to_be_bytes());
        stco.extend(data_at.to_be_bytes());
        let stsc = [0u8, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 2, 0, 0, 0, 1];
        [full(b"stsz", &stsz), full(b"stts", &stts), full(b"stco", &stco), full(b"stsc", &stsc)].concat()
    }

    /// A trak with the given id and timescale 1000 around `stbl`, optionally naming `chap_ref` in tref/chap.
    fn trak(id: u32, stbl: &[u8], chap_ref: Option<u32>) -> Vec<u8> {
        let mut tkhd = vec![0u8; 12];
        tkhd.extend(id.to_be_bytes());
        let mut mdhd = vec![0u8; 12];
        mdhd.extend(1000u32.to_be_bytes());
        mdhd.extend([0u8; 8]);
        let mdia = full(b"mdia", &[full(b"mdhd", &mdhd), full(b"minf", &full(b"stbl", stbl))].concat());
        let mut parts = vec![full(b"tkhd", &tkhd), mdia];
        if let Some(r) = chap_ref {
            parts.push(full(b"tref", &full(b"chap", &r.to_be_bytes())));
        }
        full(b"trak", &parts.concat())
    }

    #[test]
    fn text_track_chapters_come_from_the_tref_chap_track() {
        let s2: Vec<u8> = [0xFE, 0xFF].into_iter().chain("Ok".encode_utf16().flat_map(|u| u.to_be_bytes())).collect();
        let mut data = 4u16.to_be_bytes().to_vec();
        data.extend(b"Intr");
        data.extend((s2.len() as u16).to_be_bytes());
        data.extend(&s2);
        // the stco offset is the data's position: right after the moov, past the mdat header. The stco entry is a
        // fixed-size field, so the moov length does not depend on its value.
        let moov_len = full(b"moov", &[trak(1, &text_stbl(0, s2.len() as u32 + 2), Some(2)), trak(2, &text_stbl(0, s2.len() as u32 + 2), None)].concat()).len();
        let at = moov_len as u32 + 8;
        let moov = full(b"moov", &[trak(1, &text_stbl(0, s2.len() as u32 + 2), Some(2)), trak(2, &text_stbl(at, s2.len() as u32 + 2), None)].concat());
        assert_eq!(moov.len(), moov_len);
        let mut file = moov.clone();
        file.extend(full(b"mdat", &data));
        let mut src = MemSource::new(file);
        let list = from_moov(&moov[8..], &mut src);
        assert_eq!(list.len(), 2);
        assert_eq!(list[0], Chapter { start_ms: 0, end_ms: Some(1000), title: Some("Intr".into()), image: None });
        assert_eq!(list[1].title.as_deref(), Some("Ok"));
        assert_eq!(list[1].end_ms, None);
    }

    #[test]
    fn a_nero_list_wins_over_a_text_track() {
        let chpl = full(b"chpl", &[1, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 1, b'A']);
        let moov = full(b"moov", &full(b"udta", &chpl));
        let mut src = MemSource::new(Vec::new());
        let list = from_moov(&moov[8..], &mut src);
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].title.as_deref(), Some("A"));
    }
}
