//! iTunes metadata (`moov/udta/meta/ilst`) of an M4A file: title, artist,
//! album, album artist, track number, year and the cover picture.

use super::audiofile::{parse_replaygain_db, Tags};
use super::mp4::{child, children, header};
use super::source::Source;
use super::Error;

/// Reads the tags of the MP4 at `src` (the whole `moov` is read once).
pub fn read(src: &mut dyn Source) -> Result<Tags, Error> {
    let mut pos = 0;
    loop {
        let mut head = [0u8; 16];
        src.read_at(pos, &mut head)?;
        let (kind, hl, size) = header(&head).ok_or(Error::Invalid("box header"))?;
        if size < hl as u64 || size > (64 << 20) {
            return Err(Error::Invalid("box size"));
        }
        if &kind == b"moov" {
            let mut moov = vec![0u8; size as usize];
            src.read_at(pos, &mut moov)?;
            let body = &moov[hl..];
            let mut tags = parse_moov(body);
            tags.chapters = super::chapters::from_moov(body, src);
            return Ok(tags);
        }
        if size == 0 {
            return Err(Error::Invalid("no moov box"));
        }
        pos += size;
    }
}

/// The tags in a `moov` payload (empty when it has no `ilst`).
pub fn parse_moov(moov: &[u8]) -> Tags {
    let mut tags = Tags::default();
    let Some(ilst) = child(moov, b"udta").and_then(|u| child(u, b"meta")).and_then(|m| m.get(4..)).and_then(|m| child(m, b"ilst")) else { return tags };
    for (kind, item) in children(ilst) {
        let Some(data) = child(item, b"data").filter(|d| d.len() >= 8) else { continue };
        let (ty, payload) = (u32::from_be_bytes([data[0], data[1], data[2], data[3]]) & 0xff_ffff, &data[8..]);
        let text = || (ty == 1).then(|| String::from_utf8_lossy(payload).trim().to_string()).filter(|s| !s.is_empty());
        match &kind {
            b"\xa9nam" => tags.title = text(),
            b"\xa9ART" => tags.artist = text(),
            b"\xa9alb" => tags.album = text(),
            b"aART" => tags.album_artist = text(),
            b"\xa9day" => tags.year = text().and_then(|t| t.get(..4)?.parse().ok()),
            b"\xa9lyr" => tags.lyrics = text(),
            b"trkn" => tags.track = payload.get(2..4).map(|b| u16::from_be_bytes([b[0], b[1]]) as u32).filter(|&n| n > 0),
            b"----" => {
                if tags.replaygain_db.is_none() {
                    tags.replaygain_db = freeform_replaygain(item);
                }
            }
            b"covr" => {
                if tags.cover.is_none() && (ty == 13 || ty == 14 || ty == 0) && !payload.is_empty() {
                    tags.cover = Some(payload.to_vec());
                }
            }
            _ => {}
        }
    }
    tags
}

/// The file with its movie box's `udta/meta/ilst` replaced by `tags`. Other boxes in `udta` and `meta` (a chapter list,
/// the handler) are kept. The movie box may grow or shrink, so every chunk offset (`stco`, `co64`) that points past its
/// old end moves by the same amount; the samples stay where they were. Nothing else in the file changes.
pub fn rewrite(file: &[u8], tags: &Tags) -> Result<Vec<u8>, Error> {
    let (at, len, hl) = find_moov(file)?;
    let mut payload = set_ilst(&file[at + hl..at + len], tags);
    let delta = (8 + payload.len()) as i64 - len as i64;
    if delta != 0 {
        payload = patch_offsets(&payload, delta, (at + len) as u64)?;
    }
    if payload.len() + 8 > u32::MAX as usize {
        return Err(Error::Invalid("moov too large"));
    }
    let mut out = Vec::with_capacity(file.len() + delta.max(0) as usize);
    out.extend_from_slice(&file[..at]);
    out.extend(rebox(b"moov", &payload));
    out.extend_from_slice(&file[at + len..]);
    Ok(out)
}

/// The top-level `moov` box: its offset, its total size and its header length.
fn find_moov(file: &[u8]) -> Result<(usize, usize, usize), Error> {
    let mut pos = 0usize;
    while pos < file.len() {
        let (kind, hl, size) = header(&file[pos..]).ok_or(Error::Invalid("box header"))?;
        let size = if size == 0 { (file.len() - pos) as u64 } else { size };
        if size < hl as u64 || size > (file.len() - pos) as u64 {
            return Err(Error::Invalid("box size"));
        }
        if &kind == b"moov" {
            return Ok((pos, size as usize, hl));
        }
        pos += size as usize;
    }
    Err(Error::Invalid("no moov box"))
}

/// The movie payload with `udta/meta/ilst` replaced. A `meta` without a handler gets one, as iTunes-style tags need.
fn set_ilst(moov: &[u8], tags: &Tags) -> Vec<u8> {
    let ilst = rebox(b"ilst", &items(tags));
    let udta = child(moov, b"udta").unwrap_or(&[]);
    let meta = match child(udta, b"meta") {
        Some(m) if m.len() >= 4 => {
            let mut body = m[..4].to_vec();
            body.extend(replace_child(&m[4..], b"ilst", &ilst));
            body
        }
        _ => {
            let mut body = vec![0u8; 4];
            body.extend(handler());
            body.extend(&ilst);
            body
        }
    };
    let udta = replace_child(udta, b"meta", &rebox(b"meta", &meta));
    replace_child(moov, b"udta", &rebox(b"udta", &udta))
}

/// The `hdlr` of an iTunes-style metadata box: handler type `mdir`, no name.
fn handler() -> Vec<u8> {
    let mut body = vec![0u8; 8];
    body.extend(b"mdir");
    body.extend([0u8; 13]); // 12 reserved bytes and the empty name's terminator
    rebox(b"hdlr", &body)
}

/// The item boxes for the tags that are set.
fn items(tags: &Tags) -> Vec<u8> {
    let mut out = Vec::new();
    let texts: [(&[u8; 4], &Option<String>); 5] = [(b"\xa9nam", &tags.title), (b"\xa9ART", &tags.artist), (b"\xa9alb", &tags.album), (b"aART", &tags.album_artist), (b"\xa9lyr", &tags.lyrics)];
    for (kind, value) in texts {
        if let Some(text) = value {
            out.extend(data_item(kind, 1, text.as_bytes()));
        }
    }
    if let Some(track) = tags.track {
        // reserved, track number, total (unused), reserved
        let mut payload = vec![0, 0];
        payload.extend((track as u16).to_be_bytes());
        payload.extend([0u8; 4]);
        out.extend(data_item(b"trkn", 0, &payload));
    }
    if let Some(year) = tags.year {
        out.extend(data_item(b"\xa9day", 1, format!("{year:04}").as_bytes()));
    }
    if let Some(cover) = &tags.cover {
        let ty = if cover.starts_with(&[0x89, b'P', b'N', b'G']) { 14 } else { 13 };
        out.extend(data_item(b"covr", ty, cover));
    }
    out
}

/// One item box: the key, then a `data` box holding the type indicator, the locale (zero) and the payload.
fn data_item(kind: &[u8; 4], ty: u32, payload: &[u8]) -> Vec<u8> {
    let mut data = ty.to_be_bytes().to_vec();
    data.extend([0u8; 4]);
    data.extend_from_slice(payload);
    rebox(kind, &rebox(b"data", &data))
}

/// A box with an 8-byte header around `body`.
fn rebox(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
    let mut b = ((body.len() + 8) as u32).to_be_bytes().to_vec();
    b.extend_from_slice(kind);
    b.extend_from_slice(body);
    b
}

/// The child boxes of a container payload as (type, header length, whole box), and the bytes after the last whole box.
fn split_boxes(data: &[u8]) -> (Vec<([u8; 4], usize, &[u8])>, &[u8]) {
    let mut out = Vec::new();
    let mut pos = 0;
    while pos < data.len() {
        let Some((kind, hl, size)) = header(&data[pos..]) else { break };
        let size = if size == 0 { (data.len() - pos) as u64 } else { size };
        if size < hl as u64 || size > (data.len() - pos) as u64 {
            break;
        }
        let end = pos + size as usize;
        out.push((kind, hl, &data[pos..end]));
        pos = end;
    }
    (out, &data[pos..])
}

/// The container payload with its first child of `kind` replaced by `new` (or `new` appended when there is none).
/// Every other child is copied byte for byte.
fn replace_child(data: &[u8], kind: &[u8; 4], new: &[u8]) -> Vec<u8> {
    let (boxes, tail) = split_boxes(data);
    let mut out = Vec::with_capacity(data.len() + new.len());
    let mut done = false;
    for (k, _, raw) in boxes {
        if !done && &k == kind {
            out.extend_from_slice(new);
            done = true;
        } else {
            out.extend_from_slice(raw);
        }
    }
    out.extend_from_slice(tail);
    if !done {
        out.extend_from_slice(new);
    }
    out
}

/// The container payload with every chunk offset at or past `at` moved by `delta`. Walks the sample tables of each
/// track (`trak/mdia/minf/stbl`); `stco` holds 32-bit offsets and `co64` 64-bit ones.
fn patch_offsets(data: &[u8], delta: i64, at: u64) -> Result<Vec<u8>, Error> {
    let (boxes, tail) = split_boxes(data);
    let mut out = Vec::with_capacity(data.len());
    for (kind, hl, raw) in boxes {
        match &kind {
            b"trak" | b"mdia" | b"minf" | b"stbl" => out.extend(rebox(&kind, &patch_offsets(&raw[hl..], delta, at)?)),
            b"stco" => out.extend(rebox(&kind, &patch_table(&raw[hl..], false, delta, at)?)),
            b"co64" => out.extend(rebox(&kind, &patch_table(&raw[hl..], true, delta, at)?)),
            _ => out.extend_from_slice(raw),
        }
    }
    out.extend_from_slice(tail);
    Ok(out)
}

/// A chunk offset table body (version and flags, entry count, entries): the entries at or past `at` move by `delta`.
fn patch_table(body: &[u8], wide: bool, delta: i64, at: u64) -> Result<Vec<u8>, Error> {
    let mut b = body.to_vec();
    if b.len() < 8 {
        return Ok(b);
    }
    let count = b[4..8].iter().fold(0usize, |a, &x| a << 8 | x as usize);
    let w = if wide { 8 } else { 4 };
    let mut p = 8;
    for _ in 0..count {
        if p + w > b.len() {
            break;
        }
        let v = b[p..p + w].iter().fold(0u64, |a, &x| a << 8 | x as u64);
        if v >= at {
            let moved = v as i64 + delta;
            if moved < 0 || (!wide && moved > u32::MAX as i64) {
                return Err(Error::Invalid("chunk offset out of range"));
            }
            b[p..p + w].copy_from_slice(&(moved as u64).to_be_bytes()[8 - w..]);
        }
        p += w;
    }
    Ok(b)
}

/// The gain of a freeform (`----`) item whose name is `REPLAYGAIN_TRACK_GAIN`: its `name` atom is a version and flags
/// word then the name, its `data` atom holds the text (type 1).
fn freeform_replaygain(item: &[u8]) -> Option<f32> {
    let name = child(item, b"name")?.get(4..)?;
    if !String::from_utf8_lossy(name).eq_ignore_ascii_case("REPLAYGAIN_TRACK_GAIN") {
        return None;
    }
    let data = child(item, b"data").filter(|d| d.len() >= 8)?;
    let ty = u32::from_be_bytes([data[0], data[1], data[2], data[3]]) & 0xff_ffff;
    (ty == 1).then(|| parse_replaygain_db(&String::from_utf8_lossy(&data[8..]))).flatten()
}

#[cfg(test)]
mod tests {
    use super::super::source::MemSource;
    use super::*;

    const M4A: &[u8] = include_bytes!("testdata/tone.m4a");

    fn full(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut b = ((body.len() + 8) as u32).to_be_bytes().to_vec();
        b.extend_from_slice(kind);
        b.extend_from_slice(body);
        b
    }

    fn item(kind: &[u8; 4], ty: u32, payload: &[u8]) -> Vec<u8> {
        let mut d = ty.to_be_bytes().to_vec();
        d.extend([0, 0, 0, 0]);
        d.extend_from_slice(payload);
        full(kind, &full(b"data", &d))
    }

    #[test]
    fn reads_ffmpegs_tags_and_cover() {
        let mut src = MemSource::new(M4A.to_vec());
        let t = read(&mut src).unwrap();
        assert_eq!(t.title.as_deref(), Some("Tone M4A"));
        assert_eq!(t.artist.as_deref(), Some("Fixture Artist"));
        assert_eq!(t.album.as_deref(), Some("Test Album"));
        assert_eq!(t.album_artist.as_deref(), Some("Various"));
        assert_eq!(t.track, Some(2));
        assert_eq!(&t.cover.unwrap()[..2], &[0xFF, 0xD8]);
    }

    #[test]
    fn freeform_replaygain_item_is_read() {
        let mut name = vec![0; 4];
        name.extend(b"REPLAYGAIN_TRACK_GAIN");
        let mut data = vec![0, 0, 0, 1, 0, 0, 0, 0];
        data.extend(b"-3.50 dB");
        let ilst = full(b"----", &[full(b"name", &name), full(b"data", &data)].concat());
        let mut meta = vec![0, 0, 0, 0];
        meta.extend(full(b"ilst", &ilst));
        let moov = full(b"udta", &full(b"meta", &meta));
        assert_eq!(parse_moov(&moov).replaygain_db, Some(-3.5));
    }

    #[test]
    fn synthetic_ilst_with_year_png_cover_and_missing_pieces() {
        let mut ilst = item(b"\xa9nam", 1, b"  Name ");
        ilst.extend(item(b"\xa9day", 1, b"2001-02-03"));
        ilst.extend(item(b"covr", 14, b"\x89PNG"));
        ilst.extend(item(b"covr", 13, b"\xFF\xD8"));
        ilst.extend(item(b"trkn", 0, &[0, 0, 0, 5, 0, 12]));
        ilst.extend(full(b"free", &[0; 4]));
        let mut meta = vec![0, 0, 0, 0];
        meta.extend(full(b"ilst", &ilst));
        let moov = full(b"udta", &full(b"meta", &meta));
        let t = parse_moov(&moov);
        assert_eq!(t.title.as_deref(), Some("Name"));
        assert_eq!(t.year, Some(2001));
        assert_eq!(t.cover.as_deref(), Some(&b"\x89PNG"[..]), "the first cover wins");
        assert_eq!(t.track, Some(5));
        assert!(t.artist.is_none());
        assert_eq!(parse_moov(&full(b"mvhd", &[0; 100])), Tags::default());
        let mut src = MemSource::new(vec![0u8; 8]);
        assert!(read(&mut src).is_err());
    }

    /// A file with `moov` first (an old title, a chapter list in `udta`, one track whose `stco` and `co64` point at
    /// the sample after the moov) and then the `mdat` holding `sample`. `at` is the offset the tables point to.
    fn file_with_moov_first(at: u32, sample: &[u8]) -> Vec<u8> {
        let mut stco = vec![0u8; 4];
        stco.extend(1u32.to_be_bytes());
        stco.extend(at.to_be_bytes());
        let mut co64 = vec![0u8; 4];
        co64.extend(1u32.to_be_bytes());
        co64.extend((at as u64).to_be_bytes());
        let stbl = [full(b"stco", &stco), full(b"co64", &co64)].concat();
        let trak = full(b"trak", &full(b"mdia", &full(b"minf", &full(b"stbl", &stbl))));
        let mut meta = vec![0u8; 4];
        meta.extend(full(b"hdlr", &[0u8; 25]));
        meta.extend(full(b"ilst", &item(b"\xa9nam", 1, b"old")));
        let chpl = full(b"chpl", &[0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 1, b'A']);
        let udta = full(b"udta", &[chpl, full(b"meta", &meta)].concat());
        let moov = full(b"moov", &[trak, udta].concat());
        let mut file = moov;
        file.extend(full(b"mdat", sample));
        file
    }

    #[test]
    fn rewrite_round_trips_tags_and_moves_chunk_offsets_when_the_moov_grows() {
        let sample = b"SAMPLE-BYTES";
        let len = file_with_moov_first(0, sample).len() - sample.len() - 8;
        let file = file_with_moov_first(len as u32 + 8, sample);
        let tags = Tags {
            title: Some("A much longer title, so the item list grows".into()),
            artist: Some("Band".into()),
            track: Some(3),
            year: Some(2011),
            cover: Some(b"\x89PNG-art".to_vec()),
            lyrics: Some("la la".into()),
            ..Tags::default()
        };
        let out = rewrite(&file, &tags).unwrap();
        let (at, size, hl) = find_moov(&out).unwrap();
        assert!(size > len, "the movie box grew");
        assert_eq!(at, 0);
        let payload = &out[at + hl..at + size];
        assert_eq!(parse_moov(payload), tags, "the tags read back");
        // the chapter list in udta survived the rewrite
        let udta = child(payload, b"udta").unwrap();
        assert_eq!(super::super::chapters::nero(child(udta, b"chpl").unwrap()).len(), 1);
        // both chunk tables now point at the sample, which sits right after the grown moov and its mdat header
        let stbl = child(child(child(child(payload, b"trak").unwrap(), b"mdia").unwrap(), b"minf").unwrap(), b"stbl").unwrap();
        let be = |b: &[u8]| b.iter().fold(0u64, |a, &x| a << 8 | x as u64);
        let stco = be(&child(stbl, b"stco").unwrap()[8..12]) as usize;
        let co64 = be(&child(stbl, b"co64").unwrap()[8..16]) as usize;
        assert_eq!(stco, size + 8);
        assert_eq!(co64, size + 8);
        assert_eq!(&out[stco..stco + sample.len()], &sample[..]);
        assert_eq!(&out[co64..co64 + sample.len()], &sample[..]);
    }

    #[test]
    fn rewrite_without_a_meta_box_adds_one_and_keeps_the_file_otherwise() {
        let file = full(b"moov", &full(b"mvhd", &[0u8; 100]));
        let file = [file, full(b"mdat", b"xyz")].concat();
        let tags = Tags { title: Some("New".into()), ..Tags::default() };
        let out = rewrite(&file, &tags).unwrap();
        let (at, size, hl) = find_moov(&out).unwrap();
        assert_eq!(parse_moov(&out[at + hl..at + size]).title.as_deref(), Some("New"));
        assert!(child(&out[at + hl..at + size], b"mvhd").is_some());
        assert_eq!(&out[size..], &file[file.len() - 11..], "the mdat box is copied unchanged");
    }
}
