//! Audio and video thumbnails: the cover picture a file carries. MP3 and AAC files with an ID3v2 tag keep it in an
//! `APIC` (or `PIC`) frame, FLAC in a PICTURE metadata block, MP4 and M4A in the `covr` atom of the iTunes-style
//! metadata list. A file with no cover gets its type icon.
//!
//! A video with no embedded cover gets its poster frame: the first key frame of its H.264 or HEVC track, decoded by the
//! system decoder (`media::hwdec`, libavcodec through `dlopen`: VA-API when the GPU has it, software otherwise). Files
//! the decoder cannot open fall back to the waveform card.

use super::images::decode_fitted;
use crate::media::mp4::{self, Codec as Mp4Codec, Demuxer, Kind as TrackKind};
use crate::media::source::FileSource;
use crate::media::Codec;
use bifrost_ui::thumb_service::{Pixels, SourceFile, ThumbArt, ThumbProvider};
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

/// Access units read while looking for a key frame and a decoded picture before giving up.
const POSTER_SAMPLES: usize = 240;

/// The poster frame of a video file: its first key frame, decoded and fitted into a square of `px` pixels. `None` for
/// a file with no H.264 or HEVC track, or when the decoder cannot produce a picture.
pub fn poster_frame(path: &Path, px: u32) -> Option<Pixels> {
    let mut src = FileSource::open(path).ok()?;
    let mut demux = Demuxer::open(&mut src).ok()?;
    let (kind, config, hevc) = match &demux.video()?.codec {
        Mp4Codec::H264 { avcc, .. } => (Codec::H264, avcc.clone(), false),
        Mp4Codec::Hevc { hvcc, .. } => (Codec::Hevc, hvcc.clone(), true),
        Mp4Codec::Aac { .. } => return None,
    };
    let (width, height) = match demux.video()?.codec {
        Mp4Codec::H264 { width, height, .. } | Mp4Codec::Hevc { width, height, .. } => (width, height),
        Mp4Codec::Aac { .. } => return None,
    };
    let mut decoder = crate::media::hwdec::open(kind).ok()?;
    decoder.configure(kind, &config, width, height).ok()?;
    let len_size = mp4::nal_length_size(&config, hevc);
    let parameter_sets = if hevc { mp4::hvcc_parameter_sets(&config) } else { mp4::avcc_parameter_sets(&config) };
    let side = px.max(1) as usize;
    let mut started = false;
    let mut sent_parameter_sets = false;
    for _ in 0..POSTER_SAMPLES {
        let sample = match demux.next(TrackKind::Video, &mut src) {
            Ok(Some(sample)) => sample,
            _ => break,
        };
        // the picture starts at the first key frame; the samples before it cannot be decoded
        if !started && !sample.sync {
            continue;
        }
        started = true;
        let mut access_unit = Vec::new();
        if !sent_parameter_sets {
            access_unit.extend_from_slice(&parameter_sets);
            sent_parameter_sets = true;
        }
        access_unit.extend(mp4::to_annex_b(&sample.data, len_size));
        decoder.push(&access_unit, (sample.pts_ms * 1000.0) as i64).ok()?;
        if let Ok(Some(frame)) = decoder.pull() {
            let fitted = crate::imgdec::contain(&frame.to_rgba(), side, side);
            return pixels_of_frame(&fitted);
        }
    }
    None
}

fn pixels_of_frame(img: &bifrost_render::gfx::Image) -> Option<Pixels> {
    Pixels::new(img.w, img.h, img.px.clone())
}

#[cfg(test)]
mod poster_tests {
    use super::*;

    #[test]
    fn a_missing_or_unreadable_file_has_no_poster_frame() {
        assert!(poster_frame(Path::new("/nonexistent/bifrost-video.mp4"), 64).is_none());
        let dir = std::env::temp_dir().join(format!("bifrost-poster-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("not-a-video.mp4");
        std::fs::write(&file, b"this is not an mp4 container").unwrap();
        assert!(poster_frame(&file, 64).is_none());
        std::fs::remove_dir_all(&dir).ok();
    }
}

/// Largest tag or metadata block read.
const TAG_MAX: usize = 16 << 20;

fn be32(b: &[u8], p: usize) -> Option<usize> {
    Some(u32::from_be_bytes(b.get(p..p + 4)?.try_into().ok()?) as usize)
}

/// The first JPEG or PNG inside `payload`, from the first byte that starts one.
fn picture_in(payload: &[u8]) -> Option<&[u8]> {
    let start = (0..payload.len().saturating_sub(3)).find(|&i| payload[i..].starts_with(&[0xFF, 0xD8, 0xFF]) || payload[i..].starts_with(&[0x89, b'P', b'N', b'G']))?;
    Some(&payload[start..])
}

fn syncsafe(b: &[u8]) -> Option<usize> {
    let b = b.get(..4)?;
    if b.iter().any(|x| x & 0x80 != 0) {
        return None;
    }
    Some(b.iter().fold(0usize, |a, &x| a << 7 | x as usize))
}

/// The cover picture of an ID3v2 tag (versions 2.2 to 2.4), from the bytes that start with the tag.
pub fn id3_cover(tag: &[u8]) -> Option<&[u8]> {
    if tag.len() < 10 || &tag[..3] != b"ID3" {
        return None;
    }
    let version = tag[3];
    let flags = tag[5];
    let size = syncsafe(&tag[6..10])?;
    let end = (10 + size).min(tag.len());
    let mut p = 10;
    // an extended header precedes the frames in 2.3 and 2.4
    if flags & 0x40 != 0 && version >= 3 {
        let ext = if version == 4 { syncsafe(tag.get(p..)?)? } else { be32(tag, p)? + 4 };
        p += ext;
    }
    let (id_len, size_len, flag_len) = if version == 2 { (3, 3, 0) } else { (4, 4, 2) };
    while p + id_len + size_len + flag_len <= end {
        let id = &tag[p..p + id_len];
        if id[0] == 0 {
            break;
        }
        let sz_bytes = &tag[p + id_len..p + id_len + size_len];
        let fsize = match (version, size_len) {
            (2, _) => (sz_bytes[0] as usize) << 16 | (sz_bytes[1] as usize) << 8 | sz_bytes[2] as usize,
            (4, _) => syncsafe(sz_bytes)?,
            _ => be32(tag, p + id_len)?,
        };
        let body = p + id_len + size_len + flag_len;
        let body_end = body.checked_add(fsize)?;
        if body_end > end {
            break;
        }
        if id == b"APIC" || id == b"PIC" {
            return picture_in(&tag[body..body_end]);
        }
        p = body_end;
    }
    None
}

/// The first picture block of a FLAC stream, from the bytes that start with `fLaC`.
pub fn flac_cover(data: &[u8]) -> Option<&[u8]> {
    if data.len() < 4 || &data[..4] != b"fLaC" {
        return None;
    }
    let mut p = 4;
    loop {
        let head = *data.get(p)?;
        let len = (*data.get(p + 1)? as usize) << 16 | (*data.get(p + 2)? as usize) << 8 | *data.get(p + 3)? as usize;
        let body = p + 4;
        let end = body.checked_add(len)?;
        if head & 0x7F == 6 {
            // picture type, mime, description, then width, height, depth, colours, and the data
            let mut q = body + 4;
            q += 4 + be32(data, q)?;
            q += 4 + be32(data, q)?;
            q += 16;
            let n = be32(data, q)?;
            return data.get(q + 4..(q + 4).checked_add(n)?.min(end));
        }
        if head & 0x80 != 0 || end > data.len() {
            return None;
        }
        p = end;
    }
}

/// An atom walk over `moov` bytes: the cover bytes of the first `covr` atom.
fn covr_in(buf: &[u8]) -> Option<Vec<u8>> {
    const CONTAINERS: [&[u8; 4]; 4] = [b"moov", b"udta", b"ilst", b"covr"];
    let mut p = 0;
    while p + 8 <= buf.len() {
        let size = be32(buf, p)?;
        let kind: &[u8] = &buf[p + 4..p + 8];
        let (head, size) = if size == 1 { (16, u64::from_be_bytes(buf.get(p + 8..p + 16)?.try_into().ok()?) as usize) } else { (8, size) };
        let end = if size == 0 { buf.len() } else { p.checked_add(size)? };
        if end > buf.len() || end < p + head {
            return None;
        }
        let inner = &buf[p + head..end];
        if kind == b"data" {
            // type and locale precede the data
            return inner.get(8..).and_then(picture_in).map(<[u8]>::to_vec);
        }
        if kind == b"meta" {
            // a full box: version and flags first
            if let Some(found) = inner.get(4..).and_then(covr_in) {
                return Some(found);
            }
        } else if CONTAINERS.iter().any(|c| &c[..] == kind) {
            if let Some(found) = covr_in(inner) {
                return Some(found);
            }
        }
        p = end;
    }
    None
}

/// The cover of an MP4 or M4A stream: top-level atoms are skipped by seeking, and only `moov` is read.
pub fn mp4_cover<R: Read + Seek>(r: &mut R) -> Option<Vec<u8>> {
    let len = r.seek(SeekFrom::End(0)).ok()?;
    let mut pos = 0u64;
    while pos + 8 <= len {
        r.seek(SeekFrom::Start(pos)).ok()?;
        let mut h = [0u8; 8];
        r.read_exact(&mut h).ok()?;
        let mut size = u32::from_be_bytes(h[..4].try_into().ok()?) as u64;
        let mut head = 8;
        if size == 1 {
            let mut big = [0u8; 8];
            r.read_exact(&mut big).ok()?;
            size = u64::from_be_bytes(big);
            head = 16;
        } else if size == 0 {
            size = len - pos;
        }
        if size < head || pos + size > len {
            return None;
        }
        if &h[4..8] == b"moov" {
            let n = (size - head) as usize;
            if n > TAG_MAX {
                return None;
            }
            let mut moov = vec![0u8; n];
            r.read_exact(&mut moov).ok()?;
            // wrap it back so the walk sees the moov box as a container
            let mut wrapped = Vec::with_capacity(n + 8);
            wrapped.extend_from_slice(&((n + 8) as u32).to_be_bytes());
            wrapped.extend_from_slice(b"moov");
            wrapped.extend_from_slice(&moov);
            return covr_in(&wrapped);
        }
        pos += size;
    }
    None
}

/// The cover picture bytes of the file at `path`, found by what the file starts with.
pub fn cover_of(path: &std::path::Path) -> Option<Vec<u8>> {
    let mut f = std::fs::File::open(path).ok()?;
    let mut head = [0u8; 12];
    f.read_exact(&mut head).ok()?;
    f.seek(SeekFrom::Start(0)).ok()?;
    if &head[..3] == b"ID3" {
        let size = syncsafe(&head[6..10])? + 10;
        let mut tag = vec![0u8; size.min(TAG_MAX)];
        f.read_exact(&mut tag).ok()?;
        return id3_cover(&tag).map(<[u8]>::to_vec);
    }
    if &head[..4] == b"fLaC" {
        let mut data = Vec::new();
        f.by_ref().take(TAG_MAX as u64).read_to_end(&mut data).ok()?;
        return flac_cover(&data).map(<[u8]>::to_vec);
    }
    if &head[4..8] == b"ftyp" {
        return mp4_cover(&mut f);
    }
    None
}

/// Audio and video: the embedded cover; the type icon otherwise.
pub struct AudiovisualProvider;

impl ThumbProvider for AudiovisualProvider {
    fn name(&self) -> &'static str {
        "audiovisual"
    }
    fn types(&self) -> &'static [&'static str] {
        &["public.audiovisual-content"]
    }
    fn render(&self, file: &SourceFile, px: u32) -> Option<ThumbArt> {
        if let Some(p) = cover_of(&file.path).and_then(|c| decode_fitted(&c, px)) {
            return Some(ThumbArt::Pixels(p));
        }
        if let Some(p) = poster_frame(&file.path, px) {
            return Some(ThumbArt::Pixels(p));
        }
        crate::shell::services::thumb_providers::waveform_card(&file.name(), px)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    const JPEG: [u8; 6] = [0xFF, 0xD8, 0xFF, 0xE0, 1, 2];

    fn syncsafe_bytes(n: usize) -> [u8; 4] {
        [(n >> 21) as u8 & 0x7F, (n >> 14) as u8 & 0x7F, (n >> 7) as u8 & 0x7F, n as u8 & 0x7F]
    }

    fn id3(version: u8, frames: &[u8]) -> Vec<u8> {
        let mut t = vec![b'I', b'D', b'3', version, 0, 0];
        t.extend(syncsafe_bytes(frames.len()));
        t.extend_from_slice(frames);
        t
    }

    fn frame24(id: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut f = id.to_vec();
        f.extend(syncsafe_bytes(body.len()));
        f.extend([0, 0]);
        f.extend_from_slice(body);
        f
    }

    fn apic_body() -> Vec<u8> {
        let mut b = vec![0];
        b.extend(b"image/jpeg\0");
        b.push(3);
        b.extend(b"front\0");
        b.extend(JPEG);
        b
    }

    #[test]
    fn an_id3v24_tag_gives_its_cover() {
        let mut frames = frame24(b"TIT2", b"\0Song");
        frames.extend(frame24(b"APIC", &apic_body()));
        let tag = id3(4, &frames);
        assert_eq!(id3_cover(&tag), Some(&JPEG[..]));
    }

    #[test]
    fn an_id3v23_tag_uses_plain_frame_sizes() {
        let body = apic_body();
        let mut f = b"APIC".to_vec();
        f.extend((body.len() as u32).to_be_bytes());
        f.extend([0, 0]);
        f.extend(&body);
        assert_eq!(id3_cover(&id3(3, &f)), Some(&JPEG[..]));
    }

    #[test]
    fn an_id3v22_tag_uses_three_byte_frames() {
        let mut body = vec![0];
        body.extend(b"JPG");
        body.push(3);
        body.push(0);
        body.extend(JPEG);
        let mut f = b"PIC".to_vec();
        f.extend([(body.len() >> 16) as u8, (body.len() >> 8) as u8, body.len() as u8]);
        f.extend(&body);
        assert_eq!(id3_cover(&id3(2, &f)), Some(&JPEG[..]));
    }

    #[test]
    fn a_tag_without_a_picture_or_with_broken_sizes_has_none() {
        assert_eq!(id3_cover(&id3(4, &frame24(b"TIT2", b"\0Song"))), None);
        assert_eq!(id3_cover(b"not a tag"), None);
        let mut broken = id3(4, &frame24(b"APIC", &apic_body()));
        broken[10 + 4..10 + 8].copy_from_slice(&syncsafe_bytes(9999));
        assert_eq!(id3_cover(&broken), None);
    }

    fn be(n: usize) -> [u8; 4] {
        (n as u32).to_be_bytes()
    }

    #[test]
    fn a_flac_picture_block_gives_its_cover() {
        let mut pic = Vec::new();
        pic.extend(be(3));
        pic.extend(be(10));
        pic.extend(b"image/jpeg");
        pic.extend(be(0));
        pic.extend([0u8; 16]);
        pic.extend(be(JPEG.len()));
        pic.extend(JPEG);
        let mut flac = b"fLaC".to_vec();
        // a STREAMINFO-sized dummy block, then the picture as the last block
        flac.extend([0, 0, 0, 4, 1, 2, 3, 4]);
        flac.push(0x80 | 6);
        flac.extend([(pic.len() >> 16) as u8, (pic.len() >> 8) as u8, pic.len() as u8]);
        flac.extend(&pic);
        assert_eq!(flac_cover(&flac), Some(&JPEG[..]));
        assert_eq!(flac_cover(b"fLaC\x80\0\0\0"), None);
    }

    fn atom(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut a = be(body.len() + 8).to_vec();
        a.extend(kind);
        a.extend(body);
        a
    }

    #[test]
    fn an_mp4_cover_is_found_in_the_metadata_list() {
        let mut data_body = vec![0, 0, 0, 13, 0, 0, 0, 0];
        data_body.extend(JPEG);
        let covr = atom(b"covr", &atom(b"data", &data_body));
        let ilst = atom(b"ilst", &covr);
        let mut meta_body = vec![0, 0, 0, 0];
        meta_body.extend(ilst);
        let udta = atom(b"udta", &atom(b"meta", &meta_body));
        let moov = atom(b"moov", &udta);
        let mut file = atom(b"ftyp", b"M4A \0\0\0\0");
        file.extend(atom(b"mdat", &[7u8; 100]));
        file.extend(moov);
        assert_eq!(mp4_cover(&mut Cursor::new(file)), Some(JPEG.to_vec()));
    }

    #[test]
    fn an_mp4_without_a_cover_has_none() {
        let mut file = atom(b"ftyp", b"isom\0\0\0\0");
        file.extend(atom(b"moov", &atom(b"udta", &[])));
        assert_eq!(mp4_cover(&mut Cursor::new(file)), None);
        assert_eq!(mp4_cover(&mut Cursor::new(vec![0u8; 4])), None);
    }

    #[test]
    fn a_picture_is_found_by_its_magic() {
        assert_eq!(picture_in(&[1, 2, 0x89, b'P', b'N', b'G', 5]), Some(&[0x89, b'P', b'N', b'G', 5][..]));
        assert_eq!(picture_in(&[1, 2, 3]), None);
    }
}
