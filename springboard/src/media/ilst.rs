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
            return Ok(parse_moov(&moov[hl..]));
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
}
