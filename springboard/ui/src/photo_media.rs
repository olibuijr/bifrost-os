//! Media kinds of library assets and the pairing of a still with its movie. The kind of a file comes from its type
//! (`uttype`), so the same extension rules as the file manager decide it. A still and a short movie form one moving
//! photo when both carry the same content identifier, the key the capture writes into each file. The pairing only
//! reads identifiers the caller already has; no file is opened here.

use crate::uttype;
use std::collections::BTreeMap;

/// The general type of an asset.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Image,
    Video,
    Audio,
    Unknown,
}

impl Kind {
    pub fn is_video(self) -> bool {
        self == Kind::Video
    }
}

/// The kind of a file, from its name. A name whose type is not an image, a movie or audio is `Unknown`.
pub fn kind_for_name(name: &str) -> Kind {
    let Some(id) = uttype::type_for_extension(name) else {
        return Kind::Unknown;
    };
    if uttype::conforms(id, "public.image") {
        Kind::Image
    } else if uttype::conforms(id, "public.movie") {
        Kind::Video
    } else if uttype::conforms(id, "public.audio") {
        Kind::Audio
    } else {
        Kind::Unknown
    }
}

/// One file in a listing, with the content identifier read from it (if it has one).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    pub name: String,
    pub content_id: Option<String>,
}

/// A still image and its movie, as indexes into the listing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pair {
    pub still: usize,
    pub movie: usize,
}

/// Pairs stills with movies that share a content identifier. Within one identifier the first still pairs with the
/// first movie, the second with the second, and so on; extra files stay single. Items without an identifier never
/// pair. The result is ordered by the index of the still.
pub fn pair_moving(items: &[Item]) -> Vec<Pair> {
    let mut groups: BTreeMap<&str, (Vec<usize>, Vec<usize>)> = BTreeMap::new();
    for (i, item) in items.iter().enumerate() {
        let Some(id) = item.content_id.as_deref() else {
            continue;
        };
        if id.is_empty() {
            continue;
        }
        let entry = groups.entry(id).or_default();
        match kind_for_name(&item.name) {
            Kind::Image => entry.0.push(i),
            Kind::Video => entry.1.push(i),
            _ => {}
        }
    }
    let mut pairs: Vec<Pair> = groups
        .values()
        .flat_map(|(stills, movies)| stills.iter().zip(movies.iter()).map(|(&still, &movie)| Pair { still, movie }))
        .collect();
    pairs.sort_by_key(|p| p.still);
    pairs
}

/// The vendor content identifier in a vendor maker note (the EXIF maker note of a still): tag 0x0011 of the IFD that
/// follows the 14-byte header ("Apple iOS", a zero, then 0x00 0x01 and the byte order), with offsets from the start of
/// the note. The movie of a Live Photo carries the same string. None when the note is not the vendor's or has no identifier.
pub fn apple_content_id(note: &[u8]) -> Option<String> {
    if !note.starts_with(b"Apple iOS\0") {
        return None;
    }
    let u16_at = |o: usize| -> Option<usize> { Some(u16::from_be_bytes(note.get(o..o.checked_add(2)?)?.try_into().ok()?) as usize) };
    let u32_at = |o: usize| -> Option<usize> { Some(u32::from_be_bytes(note.get(o..o.checked_add(4)?)?.try_into().ok()?) as usize) };
    let count = u16_at(14)?;
    for i in 0..count {
        let e = 16 + i * 12;
        if u16_at(e)? != 0x0011 {
            continue;
        }
        if u16_at(e + 2)? != 2 {
            return None;
        }
        let n = u32_at(e + 4)?;
        let start = if n <= 4 { e + 8 } else { u32_at(e + 8)? };
        let text = std::str::from_utf8(note.get(start..start.checked_add(n)?)?).ok()?.trim_end_matches('\0').trim();
        return (!text.is_empty()).then(|| text.to_string());
    }
    None
}

/// The boxes of an ISO base media (QuickTime or MP4) buffer as (type, body). Stops at the first box that does not fit;
/// the 64-bit and to-end sizes are not followed.
fn boxes(mut data: &[u8]) -> Vec<([u8; 4], &[u8])> {
    let mut out = Vec::new();
    while data.len() >= 8 {
        let size = u32::from_be_bytes([data[0], data[1], data[2], data[3]]) as usize;
        if size < 8 || size > data.len() {
            break;
        }
        out.push(([data[4], data[5], data[6], data[7]], &data[8..size]));
        data = &data[size..];
    }
    out
}

fn find_box<'a>(data: &'a [u8], kind: &[u8; 4]) -> Option<&'a [u8]> {
    boxes(data).into_iter().find(|(k, _)| k == kind).map(|(_, body)| body)
}

/// The key that names the content identifier in a movie's metadata.
const CONTENT_ID_KEY: &str = "com.apple.quicktime.content.identifier";

/// The content identifier of a QuickTime movie: the value of `com.apple.quicktime.content.identifier` in the metadata
/// of `moov` (the keys box names each item, the item list holds the values). None when the movie has none.
pub fn quicktime_content_id(movie: &[u8]) -> Option<String> {
    let meta = find_box(find_box(movie, b"moov")?, b"meta")?;
    // `meta` is a full box: four bytes of version and flags come before its children.
    let meta = meta.get(4..)?;
    let keys = find_box(meta, b"keys")?.get(4..)?;
    let count = u32::from_be_bytes(keys.get(0..4)?.try_into().ok()?) as usize;
    let mut names = Vec::with_capacity(count);
    let mut at = 4;
    for _ in 0..count {
        let size = u32::from_be_bytes(keys.get(at..at + 4)?.try_into().ok()?) as usize;
        if size < 8 {
            return None;
        }
        names.push(keys.get(at + 8..at + size)?);
        at += size;
    }
    for (kind, item) in boxes(find_box(meta, b"ilst")?) {
        let index = u32::from_be_bytes(kind) as usize;
        if index == 0 || names.get(index - 1) != Some(&CONTENT_ID_KEY.as_bytes()) {
            continue;
        }
        // the `data` box: a type indicator (1 is UTF-8), a locale, then the value
        let value = find_box(item, b"data")?.get(8..)?;
        let text = std::str::from_utf8(value).ok()?.trim_end_matches('\0').trim();
        return (!text.is_empty()).then(|| text.to_string());
    }
    None
}

/// The pixel size of a QuickTime or MP4 movie: the picture size of its first video track (`moov/trak` whose `mdia/hdlr`
/// is `vide`, with a non-zero size in `tkhd`). None when the movie has no such track in the bytes given. A movie whose
/// `moov` box comes after the bytes given (not faststart) has none.
pub fn movie_pixel_size(movie: &[u8]) -> Option<(usize, usize)> {
    let moov = find_box(movie, b"moov")?;
    boxes(moov).into_iter().filter(|(kind, _)| kind == b"trak").find_map(|(_, trak)| {
        let handler = find_box(find_box(trak, b"mdia")?, b"hdlr")?.get(8..12)?;
        if handler != b"vide" {
            return None;
        }
        track_size(trak)
    })
}

/// The picture size of one track from its `tkhd`: the width and height are 16.16 fixed-point numbers at the end of
/// the box. A version 1 box has 64-bit times, so its fields are wider.
fn track_size(trak: &[u8]) -> Option<(usize, usize)> {
    let tkhd = find_box(trak, b"tkhd")?;
    // version and flags, the times, the track id, a reserved word and the duration
    let fields = if *tkhd.first()? == 1 { 4 + 8 + 8 + 4 + 4 + 8 } else { 4 + 4 + 4 + 4 + 4 + 4 };
    // reserved (8), layer, alternate group, volume and reserved (2 each), then the 36-byte matrix
    let at = fields + 8 + 8 + 36;
    let width = u32::from_be_bytes(tkhd.get(at..at + 4)?.try_into().ok()?) >> 16;
    let height = u32::from_be_bytes(tkhd.get(at + 4..at + 8)?.try_into().ok()?) >> 16;
    (width > 0 && height > 0).then_some((width as usize, height as usize))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A box with a four-character type and a body.
    fn boxed(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut v = ((body.len() + 8) as u32).to_be_bytes().to_vec();
        v.extend_from_slice(kind);
        v.extend_from_slice(body);
        v
    }

    /// A movie whose moov/meta carries the content identifier `id` under the QuickTime key.
    fn movie(id: &str) -> Vec<u8> {
        let mut key = b"mdta".to_vec();
        key.extend_from_slice(CONTENT_ID_KEY.as_bytes());
        // version and flags, then the entry count (one key), then the key entry itself
        let mut keys_body = vec![0, 0, 0, 0, 0, 0, 0, 1];
        // the entry size counts its own four bytes, then the namespace and the name (key is the namespace and the name)
        keys_body.extend_from_slice(&((key.len() + 4) as u32).to_be_bytes());
        keys_body.extend_from_slice(&key);
        let mut data_body = vec![0, 0, 0, 1, 0, 0, 0, 0];
        data_body.extend_from_slice(id.as_bytes());
        let item = boxed(b"data", &data_body);
        let mut ilst = Vec::new();
        ilst.extend_from_slice(&boxed(&1u32.to_be_bytes(), &item));
        let mut meta_body = vec![0, 0, 0, 0];
        meta_body.extend_from_slice(&boxed(b"keys", &keys_body));
        meta_body.extend_from_slice(&boxed(b"ilst", &ilst));
        let moov = boxed(b"moov", &boxed(b"meta", &meta_body));
        let mut file = boxed(b"ftyp", b"qt  \0\0\0\0");
        file.extend_from_slice(&moov);
        file
    }

    /// A vendor maker note carrying the identifier `id` as tag 0x0011 (the value is stored after the IFD).
    fn maker_note(id: &str) -> Vec<u8> {
        let mut note = b"Apple iOS\0\0\x01MM".to_vec();
        note.extend_from_slice(&1u16.to_be_bytes());
        let value_at = 14 + 2 + 12 + 4;
        note.extend_from_slice(&0x0011u16.to_be_bytes());
        note.extend_from_slice(&2u16.to_be_bytes());
        note.extend_from_slice(&((id.len() + 1) as u32).to_be_bytes());
        note.extend_from_slice(&(value_at as u32).to_be_bytes());
        note.extend_from_slice(&[0, 0, 0, 0]);
        note.extend_from_slice(id.as_bytes());
        note.push(0);
        note
    }

    #[test]
    fn the_apple_identifier_is_read_from_the_maker_note() {
        assert_eq!(apple_content_id(&maker_note("A1B2-C3")).as_deref(), Some("A1B2-C3"));
        assert_eq!(apple_content_id(b"Canon\0\0\0"), None, "another maker's note");
        assert_eq!(apple_content_id(&maker_note("")), None, "an empty identifier");
    }

    #[test]
    fn the_movie_identifier_is_read_from_the_metadata() {
        assert_eq!(quicktime_content_id(&movie("A1B2-C3")).as_deref(), Some("A1B2-C3"));
        assert_eq!(quicktime_content_id(&boxed(b"ftyp", b"qt  \0\0\0\0")), None, "no metadata");
        assert_eq!(quicktime_content_id(b"garbage"), None);
    }

    #[test]
    fn a_live_photo_pair_shares_the_identifier() {
        let still = apple_content_id(&maker_note("LIVE-7"));
        let clip = quicktime_content_id(&movie("LIVE-7"));
        assert!(still.is_some() && still == clip);
        let items = [
            Item { name: "IMG_7.jpg".into(), content_id: still },
            Item { name: "IMG_7.mov".into(), content_id: clip },
        ];
        assert_eq!(pair_moving(&items), vec![Pair { still: 0, movie: 1 }]);
    }

    /// A track with a `tkhd` of the given version and picture size, and a handler of the given type.
    fn track(version: u8, handler: &[u8; 4], width: u32, height: u32) -> Vec<u8> {
        let mut tkhd = vec![version, 0, 0, 7];
        let times = if version == 1 { 8 + 8 + 4 + 4 + 8 } else { 4 + 4 + 4 + 4 + 4 };
        tkhd.extend(std::iter::repeat(0).take(times));
        tkhd.extend(std::iter::repeat(0).take(8 + 8 + 36));
        tkhd.extend_from_slice(&(width << 16).to_be_bytes());
        tkhd.extend_from_slice(&(height << 16).to_be_bytes());
        let mut hdlr = vec![0; 8];
        hdlr.extend_from_slice(handler);
        let mut mdia = boxed(b"hdlr", &hdlr);
        mdia = boxed(b"mdia", &mdia);
        let mut body = boxed(b"tkhd", &tkhd);
        body.extend_from_slice(&mdia);
        boxed(b"trak", &body)
    }

    /// A movie made of the given tracks in `moov`.
    fn movie_with(tracks: &[Vec<u8>]) -> Vec<u8> {
        let mut moov_body = Vec::new();
        for t in tracks {
            moov_body.extend_from_slice(t);
        }
        boxed(b"moov", &moov_body)
    }

    #[test]
    fn the_movie_size_is_the_first_video_track() {
        let sound = track(0, b"soun", 0, 0);
        let picture = track(0, b"vide", 1920, 1080);
        assert_eq!(movie_pixel_size(&movie_with(&[sound, picture])), Some((1920, 1080)));
        let wide = track(1, b"vide", 3840, 2160);
        assert_eq!(movie_pixel_size(&movie_with(&[wide])), Some((3840, 2160)), "a version 1 header");
    }

    #[test]
    fn a_movie_without_a_picture_or_a_moov_has_no_size() {
        assert_eq!(movie_pixel_size(&movie_with(&[track(0, b"soun", 0, 0)])), None);
        assert_eq!(movie_pixel_size(&movie_with(&[track(0, b"vide", 0, 0)])), None, "a zero size");
        assert_eq!(movie_pixel_size(&boxed(b"ftyp", b"qt  \0\0\0\0")), None, "no moov in these bytes");
        assert_eq!(movie_pixel_size(b"garbage"), None);
    }

    fn item(name: &str, id: Option<&str>) -> Item {
        Item { name: name.to_string(), content_id: id.map(|s| s.to_string()) }
    }

    #[test]
    fn the_kind_follows_the_file_type() {
        assert_eq!(kind_for_name("holiday.jpg"), Kind::Image);
        assert_eq!(kind_for_name("CLIP.MOV"), Kind::Video);
        assert_eq!(kind_for_name("clip.mp4"), Kind::Video);
        assert_eq!(kind_for_name("song.m4a"), Kind::Audio);
        assert_eq!(kind_for_name("notes.txt"), Kind::Unknown);
        assert_eq!(kind_for_name("no-extension"), Kind::Unknown);
        assert!(kind_for_name("a.mov").is_video());
    }

    #[test]
    fn a_still_and_a_movie_with_one_identifier_pair_up() {
        let items = [item("IMG_1.jpg", Some("X")), item("IMG_1.mov", Some("X"))];
        assert_eq!(pair_moving(&items), vec![Pair { still: 0, movie: 1 }]);
    }

    #[test]
    fn different_identifiers_do_not_pair() {
        let items = [item("a.jpg", Some("1")), item("b.mov", Some("2"))];
        assert!(pair_moving(&items).is_empty());
    }

    #[test]
    fn files_without_an_identifier_never_pair() {
        let items = [item("a.jpg", None), item("b.mov", None), item("c.mov", Some(""))];
        assert!(pair_moving(&items).is_empty());
    }

    #[test]
    fn two_stills_with_no_movie_stay_single() {
        let items = [item("a.jpg", Some("X")), item("b.jpg", Some("X"))];
        assert!(pair_moving(&items).is_empty());
    }

    #[test]
    fn pairs_are_ordered_by_the_still_and_extra_stills_stay_single() {
        // Stills 0 and 1 share X with movies 2 and 3: the first still takes the first movie.
        let items = [
            item("s1.jpg", Some("X")),
            item("s2.jpg", Some("X")),
            item("m1.mov", Some("X")),
            item("m2.mov", Some("X")),
        ];
        assert_eq!(pair_moving(&items), vec![Pair { still: 0, movie: 2 }, Pair { still: 1, movie: 3 }]);
        // With one movie, the second still stays single.
        let one_movie = [item("s1.jpg", Some("X")), item("s2.jpg", Some("X")), item("m1.mov", Some("X"))];
        assert_eq!(pair_moving(&one_movie), vec![Pair { still: 0, movie: 2 }]);
    }

    #[test]
    fn a_movie_listed_before_its_still_still_pairs() {
        let items = [item("m.mov", Some("Y")), item("s.jpg", Some("Y"))];
        assert_eq!(pair_moving(&items), vec![Pair { still: 1, movie: 0 }]);
    }
}
