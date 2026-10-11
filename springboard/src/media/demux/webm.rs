//! Matroska / WebM (EBML) element parsing.
//!
//! Slice based: `parse_info`, `parse_tracks`, `parse_cues`, `parse_block`
//! and `parse_block_group` take complete element payloads. Streaming (unknown
//! size Segment and Cluster, split input) is driven by `mse.rs` through
//! [`read_elem_header`].

use super::{CodecId, DemuxError, TrackInfo, TrackKind};

type R<T> = Result<T, DemuxError>;

pub const EBML: u32 = 0x1A45DFA3;
pub const SEGMENT: u32 = 0x18538067;
pub const SEEK_HEAD: u32 = 0x114D9B74;
pub const INFO: u32 = 0x1549A966;
pub const TIMECODE_SCALE: u32 = 0x2AD7B1;
pub const DURATION: u32 = 0x4489;
pub const TRACKS: u32 = 0x1654AE6B;
pub const TRACK_ENTRY: u32 = 0xAE;
pub const TRACK_NUMBER: u32 = 0xD7;
pub const TRACK_TYPE: u32 = 0x83;
pub const CODEC_ID: u32 = 0x86;
pub const CODEC_PRIVATE: u32 = 0x63A2;
pub const DEFAULT_DURATION: u32 = 0x23E383;
pub const VIDEO: u32 = 0xE0;
pub const PIXEL_WIDTH: u32 = 0xB0;
pub const PIXEL_HEIGHT: u32 = 0xBA;
pub const AUDIO: u32 = 0xE1;
pub const SAMPLING_FREQ: u32 = 0xB5;
pub const CHANNELS: u32 = 0x9F;
pub const CLUSTER: u32 = 0x1F43B675;
pub const TIMECODE: u32 = 0xE7;
pub const SIMPLE_BLOCK: u32 = 0xA3;
pub const BLOCK_GROUP: u32 = 0xA0;
pub const BLOCK: u32 = 0xA1;
pub const BLOCK_DURATION: u32 = 0x9B;
pub const REFERENCE_BLOCK: u32 = 0xFB;
pub const CUES: u32 = 0x1C53BB6B;
pub const CUE_POINT: u32 = 0xBB;
pub const CUE_TIME: u32 = 0xB3;
pub const CUE_TRACK_POSITIONS: u32 = 0xB7;
pub const CUE_TRACK: u32 = 0xF7;
pub const CUE_CLUSTER_POSITION: u32 = 0xF1;

/// Default TimecodeScale: 1 ms per tick.
pub const DEFAULT_TIMECODE_SCALE: u64 = 1_000_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ElemHeader {
    pub id: u32,
    /// `None` for the reserved "unknown size" value.
    pub size: Option<u64>,
    pub header_len: usize,
}

fn vint_len(b: u8) -> Option<usize> {
    if b == 0 {
        None
    } else {
        Some(b.leading_zeros() as usize + 1)
    }
}

/// Read an element header; `Ok(None)` if `d` is too short to hold it.
pub fn read_elem_header(d: &[u8]) -> R<Option<ElemHeader>> {
    if d.is_empty() {
        return Ok(None);
    }
    let il = match vint_len(d[0]) {
        Some(l) if l <= 4 => l,
        _ => return Err(DemuxError::Invalid("EBML element id")),
    };
    if d.len() < il {
        return Ok(None);
    }
    let id = d[..il].iter().fold(0u32, |a, b| (a << 8) | *b as u32);
    let rest = &d[il..];
    if rest.is_empty() {
        return Ok(None);
    }
    let sl = match vint_len(rest[0]) {
        Some(l) => l,
        None => return Err(DemuxError::Invalid("EBML element size")),
    };
    if rest.len() < sl {
        return Ok(None);
    }
    let mut v = (rest[0] & (0xFFu16 >> sl) as u8) as u64;
    for b in &rest[1..sl] {
        v = (v << 8) | *b as u64;
    }
    let unknown = v == (1u64 << (7 * sl)) - 1;
    Ok(Some(ElemHeader { id, size: if unknown { None } else { Some(v) }, header_len: il + sl }))
}

/// Read a variable-length integer value (marker bit removed) and its length.
fn read_vint(d: &[u8]) -> R<(u64, usize)> {
    let first = *d.first().ok_or(DemuxError::Invalid("truncated vint"))?;
    let l = vint_len(first).ok_or(DemuxError::Invalid("vint"))?;
    if d.len() < l {
        return Err(DemuxError::Invalid("truncated vint"));
    }
    let mut v = (first & (0xFFu16 >> l) as u8) as u64;
    for b in &d[1..l] {
        v = (v << 8) | *b as u64;
    }
    Ok((v, l))
}

/// Split a payload into its child elements (all must have known sizes).
pub fn elements(data: &[u8]) -> R<Vec<(u32, &[u8])>> {
    let mut pos = 0usize;
    let mut out = Vec::new();
    while pos < data.len() {
        let h = read_elem_header(&data[pos..])?.ok_or(DemuxError::Invalid("truncated element header"))?;
        let size = h.size.ok_or(DemuxError::Invalid("unknown size child element"))?;
        let start = pos + h.header_len;
        if size > (data.len() - start) as u64 {
            return Err(DemuxError::Invalid("element overruns parent"));
        }
        let end = start + size as usize;
        out.push((h.id, &data[start..end]));
        pos = end;
    }
    Ok(out)
}

pub fn read_uint(p: &[u8]) -> R<u64> {
    if p.len() > 8 {
        return Err(DemuxError::Invalid("integer element"));
    }
    Ok(p.iter().fold(0u64, |a, b| (a << 8) | *b as u64))
}

pub fn read_float(p: &[u8]) -> R<f64> {
    match p.len() {
        0 => Ok(0.0),
        4 => Ok(f32::from_be_bytes([p[0], p[1], p[2], p[3]]) as f64),
        8 => {
            let mut a = [0u8; 8];
            a.copy_from_slice(p);
            Ok(f64::from_be_bytes(a))
        }
        _ => Err(DemuxError::Invalid("float element")),
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SegmentInfo {
    /// Nanoseconds per timecode tick.
    pub timecode_scale: u64,
    /// Duration in ticks.
    pub duration: Option<f64>,
}

pub fn parse_info(payload: &[u8]) -> R<SegmentInfo> {
    let mut info = SegmentInfo { timecode_scale: DEFAULT_TIMECODE_SCALE, duration: None };
    for (id, p) in elements(payload)? {
        match id {
            TIMECODE_SCALE => {
                info.timecode_scale = read_uint(p)?;
                if info.timecode_scale == 0 {
                    return Err(DemuxError::Invalid("zero TimecodeScale"));
                }
            }
            DURATION => info.duration = Some(read_float(p)?),
            _ => {}
        }
    }
    Ok(info)
}

fn codec_from_id(id: &str) -> CodecId {
    match id {
        "V_VP8" => CodecId::Vp8,
        "V_VP9" => CodecId::Vp9,
        "V_AV1" => CodecId::Av1,
        "V_MPEG4/ISO/AVC" => CodecId::H264,
        "V_MPEGH/ISO/HEVC" => CodecId::Hevc,
        "A_OPUS" => CodecId::Opus,
        "A_VORBIS" => CodecId::Vorbis,
        s if s.starts_with("A_AAC") => CodecId::Aac,
        _ => CodecId::Unknown,
    }
}

/// Parse a `Tracks` payload. Only video and audio tracks are returned.
pub fn parse_tracks(payload: &[u8]) -> R<Vec<TrackInfo>> {
    let mut out = Vec::new();
    for (id, p) in elements(payload)? {
        if id != TRACK_ENTRY {
            continue;
        }
        let mut t = TrackInfo::default();
        let mut number = None;
        let mut ty = 0u64;
        for (cid, cp) in elements(p)? {
            match cid {
                TRACK_NUMBER => number = Some(read_uint(cp)?),
                TRACK_TYPE => ty = read_uint(cp)?,
                CODEC_ID => t.codec = codec_from_id(String::from_utf8_lossy(cp).trim_end_matches('\0')),
                CODEC_PRIVATE => t.codec_private = cp.to_vec(),
                DEFAULT_DURATION => t.default_duration_us = Some((read_uint(cp)? / 1000) as i64),
                VIDEO => {
                    for (vid, vp) in elements(cp)? {
                        match vid {
                            PIXEL_WIDTH => t.width = read_uint(vp)? as u32,
                            PIXEL_HEIGHT => t.height = read_uint(vp)? as u32,
                            _ => {}
                        }
                    }
                }
                AUDIO => {
                    for (aid, ap) in elements(cp)? {
                        match aid {
                            SAMPLING_FREQ => t.sample_rate = read_float(ap)? as u32,
                            CHANNELS => t.channels = read_uint(ap)? as u16,
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
        }
        t.kind = match ty {
            1 => TrackKind::Video,
            2 => TrackKind::Audio,
            _ => continue,
        };
        t.id = number.ok_or(DemuxError::Invalid("TrackEntry without TrackNumber"))? as u32;
        out.push(t);
    }
    Ok(out)
}

/// Split the Xiph-laced Vorbis CodecPrivate into identification, comment
/// and setup headers.
pub fn vorbis_headers(private: &[u8]) -> R<[Vec<u8>; 3]> {
    if private.first() != Some(&2) {
        return Err(DemuxError::Invalid("Vorbis CodecPrivate"));
    }
    let mut pos = 1usize;
    let mut sizes = [0usize; 2];
    for s in sizes.iter_mut() {
        loop {
            let b = *private.get(pos).ok_or(DemuxError::Invalid("Vorbis CodecPrivate"))?;
            pos += 1;
            *s += b as usize;
            if b != 255 {
                break;
            }
        }
    }
    let end1 = pos + sizes[0];
    let end2 = end1 + sizes[1];
    if end2 > private.len() {
        return Err(DemuxError::Invalid("Vorbis CodecPrivate"));
    }
    Ok([private[pos..end1].to_vec(), private[end1..end2].to_vec(), private[end2..].to_vec()])
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CuePoint {
    /// Ticks.
    pub time: u64,
    pub track: u64,
    /// Offset of the Cluster from the start of the Segment payload.
    pub cluster_position: u64,
}

pub fn parse_cues(payload: &[u8]) -> R<Vec<CuePoint>> {
    let mut out = Vec::new();
    for (id, p) in elements(payload)? {
        if id != CUE_POINT {
            continue;
        }
        let mut time = 0;
        let mut positions = Vec::new();
        for (cid, cp) in elements(p)? {
            match cid {
                CUE_TIME => time = read_uint(cp)?,
                CUE_TRACK_POSITIONS => {
                    let mut track = 0;
                    let mut pos = 0;
                    for (tid, tp) in elements(cp)? {
                        match tid {
                            CUE_TRACK => track = read_uint(tp)?,
                            CUE_CLUSTER_POSITION => pos = read_uint(tp)?,
                            _ => {}
                        }
                    }
                    positions.push((track, pos));
                }
                _ => {}
            }
        }
        for (track, cluster_position) in positions {
            out.push(CuePoint { time, track, cluster_position });
        }
    }
    Ok(out)
}

/// A parsed Block / SimpleBlock header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Block {
    pub track: u64,
    /// Relative to the Cluster Timecode, in ticks.
    pub timecode: i16,
    /// SimpleBlock keyframe flag (always false for a plain Block).
    pub keyframe: bool,
    pub invisible: bool,
    /// `(offset, len)` of each frame inside the block payload.
    pub frames: Vec<(usize, usize)>,
}

pub fn parse_block(p: &[u8], simple: bool) -> R<Block> {
    let (track, tl) = read_vint(p)?;
    if p.len() < tl + 3 {
        return Err(DemuxError::Invalid("truncated block"));
    }
    let timecode = i16::from_be_bytes([p[tl], p[tl + 1]]);
    let flags = p[tl + 2];
    let mut off = tl + 3;
    let total = p.len();
    let lacing = (flags >> 1) & 3;
    let frames = if lacing == 0 {
        vec![(off, total - off)]
    } else {
        if off >= total {
            return Err(DemuxError::Invalid("truncated lacing"));
        }
        let n = p[off] as usize + 1;
        off += 1;
        let mut sizes: Vec<usize> = Vec::with_capacity(n);
        match lacing {
            1 => {
                for _ in 0..n - 1 {
                    let mut s = 0usize;
                    loop {
                        let b = *p.get(off).ok_or(DemuxError::Invalid("truncated lacing"))?;
                        off += 1;
                        s += b as usize;
                        if b != 255 {
                            break;
                        }
                    }
                    sizes.push(s);
                }
            }
            3 => {
                if n > 1 {
                    let (first, l) = read_vint(&p[off..])?;
                    off += l;
                    sizes.push(first as usize);
                    let mut prev = first as i64;
                    for _ in 0..n - 2 {
                        let (raw, l) = read_vint(&p[off..])?;
                        off += l;
                        prev += raw as i64 - ((1i64 << (7 * l - 1)) - 1);
                        if prev < 0 {
                            return Err(DemuxError::Invalid("negative lace size"));
                        }
                        sizes.push(prev as usize);
                    }
                }
            }
            _ => {
                let rest = total - off;
                if rest % n != 0 {
                    return Err(DemuxError::Invalid("fixed lacing size"));
                }
                sizes.resize(n - 1, rest / n);
            }
        }
        let used: usize = sizes.iter().sum();
        if off + used > total {
            return Err(DemuxError::Invalid("lace sizes exceed block"));
        }
        sizes.push(total - off - used);
        let mut frames = Vec::with_capacity(n);
        let mut pos = off;
        for s in sizes {
            frames.push((pos, s));
            pos += s;
        }
        frames
    };
    Ok(Block { track, timecode, keyframe: simple && flags & 0x80 != 0, invisible: flags & 0x08 != 0, frames })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockGroup {
    pub block: Block,
    /// Offset of the Block payload within the BlockGroup payload.
    pub block_offset: usize,
    pub duration: Option<u64>,
    pub has_reference: bool,
}

pub fn parse_block_group(payload: &[u8]) -> R<BlockGroup> {
    let mut block = None;
    let mut block_offset = 0;
    let mut duration = None;
    let mut has_reference = false;
    for (id, p) in elements(payload)? {
        match id {
            BLOCK => {
                block = Some(parse_block(p, false)?);
                block_offset = p.as_ptr() as usize - payload.as_ptr() as usize;
            }
            BLOCK_DURATION => duration = Some(read_uint(p)?),
            REFERENCE_BLOCK => has_reference = true,
            _ => {}
        }
    }
    let block = block.ok_or(DemuxError::Invalid("BlockGroup without Block"))?;
    Ok(BlockGroup { block, block_offset, duration, has_reference })
}

#[cfg(test)]
mod tests {
    use super::super::fixtures::*;
    use super::*;

    #[test]
    fn element_header_sizes() {
        let h = read_elem_header(&[0xA3, 0x85, 1, 2, 3, 4, 5]).unwrap().unwrap();
        assert_eq!(h, ElemHeader { id: 0xA3, size: Some(5), header_len: 2 });
        let h = read_elem_header(&[0x1A, 0x45, 0xDF, 0xA3, 0x40, 0x10]).unwrap().unwrap();
        assert_eq!(h, ElemHeader { id: EBML, size: Some(16), header_len: 6 });
        let h = read_elem_header(&[0x1F, 0x43, 0xB6, 0x75, 0x01, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff])
            .unwrap()
            .unwrap();
        assert_eq!(h, ElemHeader { id: CLUSTER, size: None, header_len: 12 });
        let h = read_elem_header(&[0xEC, 0xFF]).unwrap().unwrap();
        assert_eq!(h.size, None, "one byte all-ones is unknown");
    }

    #[test]
    fn element_header_partial_and_invalid() {
        assert_eq!(read_elem_header(&[]).unwrap(), None);
        assert_eq!(read_elem_header(&[0x1A, 0x45]).unwrap(), None);
        assert_eq!(read_elem_header(&[0x1A, 0x45, 0xDF, 0xA3]).unwrap(), None);
        assert_eq!(read_elem_header(&[0x1A, 0x45, 0xDF, 0xA3, 0x40]).unwrap(), None);
        assert!(read_elem_header(&[0x00, 0x81]).is_err(), "zero first byte");
        assert!(read_elem_header(&[0x08, 0x81]).is_err(), "id longer than 4 bytes");
        assert!(read_elem_header(&[0xA3, 0x00]).is_err(), "zero size lead byte");
    }

    #[test]
    fn eight_byte_size() {
        let h = read_elem_header(&[0xA3, 0x01, 0, 0, 0, 0, 0, 1, 0]).unwrap().unwrap();
        assert_eq!(h.size, Some(256));
        assert_eq!(h.header_len, 9);
    }

    #[test]
    fn vint_size_roundtrip() {
        for n in [0u64, 1, 126, 127, 128, 16382, 16383, 1 << 20, 1 << 40] {
            let mut d = vint_size(n);
            let hl = d.len();
            d.insert(0, 0xEC);
            let h = read_elem_header(&d).unwrap().unwrap();
            assert_eq!(h.size, Some(n), "{n}");
            assert_eq!(h.header_len, 1 + hl);
        }
    }

    #[test]
    fn elements_rejects_overrun_and_unknown_size() {
        let mut d = el(0xEC, &[1, 2, 3]);
        d.pop();
        assert!(elements(&d).is_err());
        assert!(elements(&el_unknown(0xEC, &[])).is_err());
        let ok = [el(0xEC, &[1]), el(0xEC, &[2, 3])].concat();
        assert_eq!(elements(&ok).unwrap().len(), 2);
    }

    #[test]
    fn numbers() {
        assert_eq!(read_uint(&[]).unwrap(), 0);
        assert_eq!(read_uint(&[1, 0]).unwrap(), 256);
        assert!(read_uint(&[0; 9]).is_err());
        assert_eq!(read_float(&1.5f32.to_be_bytes()).unwrap(), 1.5);
        assert_eq!(read_float(&2.25f64.to_be_bytes()).unwrap(), 2.25);
        assert!(read_float(&[1, 2, 3]).is_err());
    }

    fn init_children() -> (Vec<u8>, Vec<u8>) {
        let init = webm_init();
        let ebml = read_elem_header(&init).unwrap().unwrap();
        let seg = &init[ebml.header_len + ebml.size.unwrap() as usize..];
        let sh = read_elem_header(seg).unwrap().unwrap();
        let kids = elements(&seg[sh.header_len..]).unwrap();
        let info = kids.iter().find(|k| k.0 == INFO).unwrap().1.to_vec();
        let tracks = kids.iter().find(|k| k.0 == TRACKS).unwrap().1.to_vec();
        (info, tracks)
    }

    #[test]
    fn info_scale_and_duration() {
        let (info, _) = init_children();
        let i = parse_info(&info).unwrap();
        assert_eq!(i.timecode_scale, 1_000_000);
        assert_eq!(i.duration, Some(360.0));
        assert_eq!(parse_info(&[]).unwrap().timecode_scale, DEFAULT_TIMECODE_SCALE);
        assert!(parse_info(&uint(TIMECODE_SCALE, 0)).is_err());
    }

    #[test]
    fn tracks_video_and_opus() {
        let (_, tracks) = init_children();
        let t = parse_tracks(&tracks).unwrap();
        assert_eq!(t.len(), 2);
        assert_eq!(t[0].id, 1);
        assert_eq!(t[0].kind, TrackKind::Video);
        assert_eq!(t[0].codec, CodecId::Vp9);
        assert_eq!((t[0].width, t[0].height), (640, 360));
        assert_eq!(t[0].default_duration_us, Some(40_000));
        assert_eq!(t[1].id, 2);
        assert_eq!(t[1].codec, CodecId::Opus);
        assert_eq!((t[1].channels, t[1].sample_rate), (2, 48000));
        assert_eq!(t[1].codec_private, opus_head());
    }

    #[test]
    fn track_codec_mapping_and_filtering() {
        let mk = |num: u64, ty: u64, codec: &[u8]| el(TRACK_ENTRY, &[uint(TRACK_NUMBER, num), uint(TRACK_TYPE, ty), el(CODEC_ID, codec)].concat());
        let tracks = [
            mk(1, 1, b"V_VP8"),
            mk(2, 1, b"V_AV1"),
            mk(3, 2, b"A_VORBIS"),
            mk(4, 1, b"V_MPEG4/ISO/AVC"),
            mk(5, 2, b"A_AAC"),
            mk(6, 17, b"S_TEXT/UTF8"),
            mk(7, 1, b"V_MPEGH/ISO/HEVC"),
            mk(8, 1, b"V_WEIRD"),
        ]
        .concat();
        let t = parse_tracks(&tracks).unwrap();
        let codecs: Vec<_> = t.iter().map(|x| x.codec).collect();
        assert_eq!(
            codecs,
            vec![
                CodecId::Vp8,
                CodecId::Av1,
                CodecId::Vorbis,
                CodecId::H264,
                CodecId::Aac,
                CodecId::Hevc,
                CodecId::Unknown
            ]
        );
        assert!(t.iter().all(|x| x.id != 6), "subtitle track dropped");
    }

    #[test]
    fn track_without_number_is_an_error() {
        let e = el(TRACK_ENTRY, &[uint(TRACK_TYPE, 1), el(CODEC_ID, b"V_VP8")].concat());
        assert!(parse_tracks(&e).is_err());
    }

    #[test]
    fn vorbis_header_split() {
        let mut p = vec![2, 3, 2];
        p.extend([1, 1, 1, 2, 2, 3, 3, 3, 3]);
        let h = vorbis_headers(&p).unwrap();
        assert_eq!(h[0], vec![1, 1, 1]);
        assert_eq!(h[1], vec![2, 2]);
        assert_eq!(h[2], vec![3, 3, 3, 3]);
        assert!(vorbis_headers(&[1, 0]).is_err());
        assert!(vorbis_headers(&[2, 200, 5, 1]).is_err());
    }

    #[test]
    fn simple_block_no_lacing() {
        let b = simple_block(1, -5, true, &[9, 8, 7]);
        let h = read_elem_header(&b).unwrap().unwrap();
        let p = &b[h.header_len..];
        let blk = parse_block(p, true).unwrap();
        assert_eq!((blk.track, blk.timecode, blk.keyframe), (1, -5, true));
        assert_eq!(blk.frames.len(), 1);
        let (o, l) = blk.frames[0];
        assert_eq!(&p[o..o + l], &[9, 8, 7]);
        assert!(!parse_block(p, false).unwrap().keyframe);
    }

    fn laced(flags: u8, body: &[u8]) -> Vec<u8> {
        let mut p = vec![0x81, 0, 10, flags];
        p.extend_from_slice(body);
        p
    }

    fn frames_of(p: &[u8]) -> Vec<Vec<u8>> {
        parse_block(p, true).unwrap().frames.iter().map(|&(o, l)| p[o..o + l].to_vec()).collect()
    }

    #[test]
    fn xiph_lacing() {
        let mut body = vec![2, 2, 255, 5];
        // sizes: 2, 260, remainder 1
        body.extend([1, 1]);
        body.extend(vec![2u8; 260]);
        body.push(3);
        let f = frames_of(&laced(0x02, &body));
        assert_eq!(f.len(), 3);
        assert_eq!((f[0].len(), f[1].len(), f[2].len()), (2, 260, 1));
        assert_eq!(f[2], vec![3]);
    }

    #[test]
    fn fixed_lacing() {
        let f = frames_of(&laced(0x04, &[2, 1, 1, 2, 2, 3, 3]));
        assert_eq!(f, vec![vec![1, 1], vec![2, 2], vec![3, 3]]);
        assert!(parse_block(&laced(0x04, &[2, 1, 1, 2, 2, 3]), true).is_err());
    }

    #[test]
    fn ebml_lacing_with_signed_deltas() {
        // 4 frames: sizes 3, 5 (delta +2), 4 (delta -1), remainder 2
        // first size vint 0x83; deltas as signed vint1: raw = delta + 63
        let mut body = vec![3, 0x83, 0x80 | (2 + 63), 0x80 | (63 - 1)];
        body.extend(vec![1u8; 3]);
        body.extend(vec![2u8; 5]);
        body.extend(vec![3u8; 4]);
        body.extend(vec![4u8; 2]);
        let f = frames_of(&laced(0x06, &body));
        assert_eq!(f.iter().map(|x| x.len()).collect::<Vec<_>>(), vec![3, 5, 4, 2]);
        assert_eq!(f[3], vec![4, 4]);
    }

    #[test]
    fn lacing_errors() {
        assert!(parse_block(&laced(0x02, &[]), true).is_err());
        assert!(parse_block(&laced(0x02, &[1, 200, 1]), true).is_err(), "sizes exceed block");
        assert!(parse_block(&[0x81, 0, 1], true).is_err());
        assert!(parse_block(&[], true).is_err());
    }

    #[test]
    fn block_group_with_duration_and_reference() {
        let mut blk = vec![0x82, 0, 20, 0];
        blk.extend([5, 5]);
        let bg = el(BLOCK_GROUP, &[el(BLOCK, &blk), uint(BLOCK_DURATION, 33), el(REFERENCE_BLOCK, &[0xff])].concat());
        let h = read_elem_header(&bg).unwrap().unwrap();
        let p = &bg[h.header_len..];
        let g = parse_block_group(p).unwrap();
        assert_eq!(g.block.track, 2);
        assert_eq!(g.block.timecode, 20);
        assert_eq!(g.duration, Some(33));
        assert!(g.has_reference);
        let (o, l) = g.block.frames[0];
        assert_eq!(&p[g.block_offset + o..g.block_offset + o + l], &[5, 5]);
        assert!(parse_block_group(&uint(BLOCK_DURATION, 1)).is_err());
    }

    #[test]
    fn cues() {
        let cue = |t: u64, track: u64, pos: u64| {
            el(
                CUE_POINT,
                &[
                    uint(CUE_TIME, t),
                    el(CUE_TRACK_POSITIONS, &[uint(CUE_TRACK, track), uint(CUE_CLUSTER_POSITION, pos)].concat()),
                ]
                .concat(),
            )
        };
        let c = parse_cues(&[cue(0, 1, 100), cue(5000, 1, 90_000)].concat()).unwrap();
        assert_eq!(
            c,
            vec![
                CuePoint { time: 0, track: 1, cluster_position: 100 },
                CuePoint { time: 5000, track: 1, cluster_position: 90_000 }
            ]
        );
    }

    #[test]
    fn whole_cluster_children() {
        let c = webm_cluster(1, false);
        let h = read_elem_header(&c).unwrap().unwrap();
        assert_eq!(h.id, CLUSTER);
        let kids = elements(&c[h.header_len..]).unwrap();
        assert_eq!(kids.len(), 7);
        assert_eq!(read_uint(kids[0].1).unwrap(), 120);
        assert_eq!(kids[1].0, SIMPLE_BLOCK);
    }
}
