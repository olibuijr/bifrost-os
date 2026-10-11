//! ISO BMFF (MP4) demuxer: plain and fragmented files.
//!
//! Parsing is slice based: callers hand over complete box payloads
//! (`parse_moov`, `parse_moof`, `parse_sidx`) or a whole file (`parse_file`).
//! The incremental driver lives in `mse.rs`. Edit lists (`elst`) are ignored.

use super::bitreader::{BitReader, ByteReader};
use super::{CodecId, DemuxError, TrackInfo, TrackKind};
use std::collections::HashMap;

type R<T> = Result<T, DemuxError>;

const MAX_SAMPLES: usize = 1 << 24;

/// Header of one box.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BoxHeader {
    pub typ: [u8; 4],
    pub header_len: usize,
    /// Total size including the header; 0 means "to the end of the data".
    pub size: u64,
}

/// Read a box header; `Ok(None)` when `data` is too short to hold it.
pub fn read_box_header(data: &[u8]) -> R<Option<BoxHeader>> {
    if data.len() < 8 {
        return Ok(None);
    }
    let size32 = u32::from_be_bytes([data[0], data[1], data[2], data[3]]);
    let typ = [data[4], data[5], data[6], data[7]];
    let (header_len, size) = if size32 == 1 {
        if data.len() < 16 {
            return Ok(None);
        }
        let mut b = [0u8; 8];
        b.copy_from_slice(&data[8..16]);
        (16usize, u64::from_be_bytes(b))
    } else {
        (8usize, size32 as u64)
    };
    if size != 0 && size < header_len as u64 {
        return Err(DemuxError::Invalid("box size"));
    }
    Ok(Some(BoxHeader { typ, header_len, size }))
}

type Kids<'a> = Vec<([u8; 4], &'a [u8])>;

/// Split a container payload into its child boxes.
pub fn children(data: &[u8]) -> R<Kids<'_>> {
    let mut pos = 0usize;
    let mut out = Vec::new();
    while pos < data.len() {
        let h = read_box_header(&data[pos..])?.ok_or(DemuxError::Invalid("truncated box header"))?;
        let remaining = (data.len() - pos) as u64;
        let size = if h.size == 0 { remaining } else { h.size };
        if size > remaining {
            return Err(DemuxError::Invalid("box overruns parent"));
        }
        out.push((h.typ, &data[pos + h.header_len..pos + size as usize]));
        pos += size as usize;
    }
    Ok(out)
}

fn find<'a>(kids: &[([u8; 4], &'a [u8])], t: &[u8; 4]) -> Option<&'a [u8]> {
    kids.iter().find(|(k, _)| k == t).map(|(_, p)| *p)
}

/// One sample with its place in the file. Times are in track timescale ticks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mp4Sample {
    pub track_id: u32,
    pub dts: i64,
    pub pts: i64,
    pub duration: u32,
    pub keyframe: bool,
    /// Absolute byte offset in the stream the box was read from.
    pub offset: u64,
    pub size: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Mp4Track {
    pub info: TrackInfo,
    pub timescale: u32,
    /// Media duration in ticks (0 for fragmented init segments).
    pub duration: u64,
    /// Samples from `stbl` (plus fragments when read by [`parse_file`]).
    pub samples: Vec<Mp4Sample>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Trex {
    pub track_id: u32,
    pub default_duration: u32,
    pub default_size: u32,
    pub default_flags: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Moov {
    pub tracks: Vec<Mp4Track>,
    pub trex: Vec<Trex>,
    /// An `mvex` box is present: media arrives in `moof` fragments.
    pub fragmented: bool,
}

impl Moov {
    pub fn track(&self, id: u32) -> Option<&Mp4Track> {
        self.tracks.iter().find(|t| t.info.id == id)
    }

    fn trex_for(&self, id: u32) -> Trex {
        self.trex.iter().copied().find(|t| t.track_id == id).unwrap_or_default()
    }
}

// ---------------------------------------------------------------- moov

pub fn parse_moov(payload: &[u8]) -> R<Moov> {
    let kids = children(payload)?;
    let mut tracks = Vec::new();
    let mut trex = Vec::new();
    let mut fragmented = false;
    for (t, p) in &kids {
        if t == b"trak" {
            if let Some(tr) = parse_trak(p)? {
                tracks.push(tr);
            }
        } else if t == b"mvex" {
            fragmented = true;
            for (t2, p2) in children(p)? {
                if &t2 == b"trex" {
                    trex.push(parse_trex(p2)?);
                }
            }
        }
    }
    if tracks.is_empty() {
        return Err(DemuxError::Invalid("moov without supported tracks"));
    }
    Ok(Moov { tracks, trex, fragmented })
}

fn parse_trex(p: &[u8]) -> R<Trex> {
    let mut r = ByteReader::new(p);
    r.skip(4)?;
    let track_id = r.u32()?;
    r.skip(4)?;
    Ok(Trex { track_id, default_duration: r.u32()?, default_size: r.u32()?, default_flags: r.u32()? })
}

fn parse_trak(payload: &[u8]) -> R<Option<Mp4Track>> {
    let kids = children(payload)?;
    let tkhd = find(&kids, b"tkhd").ok_or(DemuxError::Invalid("trak without tkhd"))?;
    let id = {
        let mut r = ByteReader::new(tkhd);
        let ver = r.u8()?;
        r.skip(3)?;
        r.skip(if ver == 1 { 16 } else { 8 })?;
        r.u32()?
    };
    let mdia = children(find(&kids, b"mdia").ok_or(DemuxError::Invalid("trak without mdia"))?)?;
    let (timescale, duration) = {
        let mut r = ByteReader::new(find(&mdia, b"mdhd").ok_or(DemuxError::Invalid("mdia without mdhd"))?);
        let ver = r.u8()?;
        r.skip(3)?;
        r.skip(if ver == 1 { 16 } else { 8 })?;
        let ts = r.u32()?;
        let d = if ver == 1 { r.u64()? } else { r.u32()? as u64 };
        (ts, d)
    };
    let hdlr = find(&mdia, b"hdlr").ok_or(DemuxError::Invalid("mdia without hdlr"))?;
    if hdlr.len() < 12 {
        return Err(DemuxError::Invalid("hdlr"));
    }
    let kind = match &hdlr[8..12] {
        b"vide" => TrackKind::Video,
        b"soun" => TrackKind::Audio,
        _ => return Ok(None),
    };
    let minf = children(find(&mdia, b"minf").ok_or(DemuxError::Invalid("mdia without minf"))?)?;
    let stbl = children(find(&minf, b"stbl").ok_or(DemuxError::Invalid("minf without stbl"))?)?;
    let stsd = find(&stbl, b"stsd").ok_or(DemuxError::Invalid("stbl without stsd"))?;
    let info = parse_stsd(stsd, id, kind)?;
    let samples = parse_stbl_samples(&stbl, id)?;
    Ok(Some(Mp4Track { info, timescale, duration, samples }))
}

fn parse_stsd(data: &[u8], id: u32, kind: TrackKind) -> R<TrackInfo> {
    let mut r = ByteReader::new(data);
    r.skip(4)?;
    if r.u32()? == 0 {
        return Err(DemuxError::Invalid("empty stsd"));
    }
    let rest = r.rest();
    let h = read_box_header(rest)?.ok_or(DemuxError::Invalid("stsd entry"))?;
    let size = if h.size == 0 { rest.len() as u64 } else { h.size };
    if size > rest.len() as u64 {
        return Err(DemuxError::Invalid("stsd entry overruns"));
    }
    let entry = &rest[h.header_len..size as usize];
    let mut info = TrackInfo { id, kind, ..Default::default() };
    match kind {
        TrackKind::Video => {
            if entry.len() < 78 {
                return Err(DemuxError::Invalid("video sample entry"));
            }
            info.width = u16::from_be_bytes([entry[24], entry[25]]) as u32;
            info.height = u16::from_be_bytes([entry[26], entry[27]]) as u32;
            let kids = children(&entry[78..])?;
            let (codec, cfg): (CodecId, &[u8; 4]) = match &h.typ {
                b"avc1" | b"avc3" => (CodecId::H264, b"avcC"),
                b"hvc1" | b"hev1" => (CodecId::Hevc, b"hvcC"),
                b"vp09" => (CodecId::Vp9, b"vpcC"),
                b"vp08" => (CodecId::Vp8, b"vpcC"),
                b"av01" => (CodecId::Av1, b"av1C"),
                _ => (CodecId::Unknown, b"----"),
            };
            info.codec = codec;
            if codec != CodecId::Unknown {
                info.codec_private = find(&kids, cfg).ok_or(DemuxError::Invalid("missing codec config box"))?.to_vec();
            }
        }
        TrackKind::Audio => {
            if entry.len() < 28 {
                return Err(DemuxError::Invalid("audio sample entry"));
            }
            info.channels = u16::from_be_bytes([entry[16], entry[17]]);
            info.sample_rate = u32::from_be_bytes([entry[24], entry[25], entry[26], entry[27]]) >> 16;
            let kids = children(&entry[28..])?;
            match &h.typ {
                b"mp4a" => {
                    info.codec = CodecId::Aac;
                    let esds = find(&kids, b"esds").ok_or(DemuxError::Invalid("mp4a without esds"))?;
                    info.codec_private = parse_esds(esds)?;
                    if let Ok((rate, ch)) = parse_asc(&info.codec_private) {
                        info.sample_rate = rate;
                        info.channels = ch;
                    }
                }
                b"Opus" => {
                    info.codec = CodecId::Opus;
                    let dops = find(&kids, b"dOps").ok_or(DemuxError::Invalid("Opus without dOps"))?;
                    info.codec_private = dops_to_opus_head(dops)?;
                    info.channels = info.codec_private[9] as u16;
                }
                _ => {}
            }
        }
    }
    Ok(info)
}

fn read_desc(r: &mut ByteReader) -> R<(u8, usize)> {
    let tag = r.u8()?;
    let mut len = 0usize;
    for _ in 0..4 {
        let b = r.u8()?;
        len = (len << 7) | (b & 0x7f) as usize;
        if b & 0x80 == 0 {
            break;
        }
    }
    Ok((tag, len))
}

/// Extract the AudioSpecificConfig from an `esds` payload.
pub fn parse_esds(p: &[u8]) -> R<Vec<u8>> {
    let mut r = ByteReader::new(p);
    r.skip(4)?;
    let (tag, _) = read_desc(&mut r)?;
    if tag != 3 {
        return Err(DemuxError::Invalid("esds ES descriptor"));
    }
    r.skip(2)?;
    let flags = r.u8()?;
    if flags & 0x80 != 0 {
        r.skip(2)?;
    }
    if flags & 0x40 != 0 {
        let n = r.u8()? as usize;
        r.skip(n)?;
    }
    if flags & 0x20 != 0 {
        r.skip(2)?;
    }
    let (tag, _) = read_desc(&mut r)?;
    if tag != 4 {
        return Err(DemuxError::Invalid("esds decoder config"));
    }
    r.skip(13)?;
    let (tag, len) = read_desc(&mut r)?;
    if tag != 5 {
        return Err(DemuxError::Invalid("esds decoder specific info"));
    }
    Ok(r.take(len)?.to_vec())
}

const AAC_RATES: [u32; 13] =
    [96000, 88200, 64000, 48000, 44100, 32000, 24000, 22050, 16000, 12000, 11025, 8000, 7350];

/// Sample rate and channel count from an AudioSpecificConfig.
pub fn parse_asc(asc: &[u8]) -> R<(u32, u16)> {
    let mut b = BitReader::new(asc);
    if b.bits(5)? == 31 {
        b.skip(6)?;
    }
    let idx = b.bits(4)? as usize;
    let rate = if idx == 15 {
        b.bits(24)?
    } else {
        *AAC_RATES.get(idx).ok_or(DemuxError::Invalid("AAC sample rate index"))?
    };
    let ch = match b.bits(4)? {
        7 => 8,
        c => c as u16,
    };
    Ok((rate, ch))
}

/// Convert a `dOps` payload (big endian) to an `OpusHead` packet.
pub fn dops_to_opus_head(p: &[u8]) -> R<Vec<u8>> {
    let mut r = ByteReader::new(p);
    r.u8()?;
    let ch = r.u8()?;
    let pre_skip = r.u16()?;
    let rate = r.u32()?;
    let gain = r.u16()?;
    let family = r.u8()?;
    let mut out = b"OpusHead".to_vec();
    out.push(1);
    out.push(ch);
    out.extend(pre_skip.to_le_bytes());
    out.extend(rate.to_le_bytes());
    out.extend(gain.to_le_bytes());
    out.push(family);
    if family != 0 {
        out.extend_from_slice(r.rest());
    }
    Ok(out)
}

// ---------------------------------------------------------------- stbl

fn table<T>(p: &[u8], entry_size: usize, f: impl Fn(&mut ByteReader) -> R<T>) -> R<Vec<T>> {
    let mut r = ByteReader::new(p);
    r.skip(4)?;
    let n = r.u32()? as usize;
    if r.remaining() / entry_size < n {
        return Err(DemuxError::Invalid("sample table truncated"));
    }
    (0..n).map(|_| f(&mut r)).collect()
}

fn expand<T: Copy>(e: &[(u32, T)], n: usize, pad: T) -> Vec<T> {
    let mut v = Vec::with_capacity(n);
    for &(c, x) in e {
        for _ in 0..c {
            if v.len() >= n {
                return v;
            }
            v.push(x);
        }
    }
    while v.len() < n {
        v.push(pad);
    }
    v
}

fn parse_stbl_samples(stbl: &Kids, track_id: u32) -> R<Vec<Mp4Sample>> {
    let Some(stsz) = find(stbl, b"stsz") else {
        return Ok(Vec::new());
    };
    let mut r = ByteReader::new(stsz);
    r.skip(4)?;
    let fixed = r.u32()?;
    let count = r.u32()? as usize;
    if count > MAX_SAMPLES {
        return Err(DemuxError::Invalid("too many samples"));
    }
    let mut sizes = Vec::new();
    if fixed != 0 {
        sizes.resize(count, fixed);
    } else {
        if r.remaining() / 4 < count {
            return Err(DemuxError::Invalid("stsz truncated"));
        }
        for _ in 0..count {
            sizes.push(r.u32()?);
        }
    }
    let stts = table(find(stbl, b"stts").ok_or(DemuxError::Invalid("stbl without stts"))?, 8, |r| {
        Ok((r.u32()?, r.u32()?))
    })?;
    let durations = expand(&stts, count, 0u32);
    let ctts = match find(stbl, b"ctts") {
        Some(p) => table(p, 8, |r| Ok((r.u32()?, r.i32()?)))?,
        None => Vec::new(),
    };
    let ctos = expand(&ctts, count, 0i32);
    let stss = match find(stbl, b"stss") {
        Some(p) => Some(table(p, 4, |r| r.u32())?),
        None => None,
    };
    let stsc = table(find(stbl, b"stsc").ok_or(DemuxError::Invalid("stbl without stsc"))?, 12, |r| {
        let a = r.u32()?;
        let b = r.u32()?;
        r.skip(4)?;
        Ok((a, b))
    })?;
    let chunks: Vec<u64> = if let Some(p) = find(stbl, b"stco") {
        table(p, 4, |r| Ok(r.u32()? as u64))?
    } else if let Some(p) = find(stbl, b"co64") {
        table(p, 8, |r| r.u64())?
    } else {
        return Err(DemuxError::Invalid("stbl without chunk offsets"));
    };

    let mut out: Vec<Mp4Sample> = Vec::with_capacity(count);
    let mut dts = 0i64;
    let mut j = 0usize;
    for (ci, &chunk_off) in chunks.iter().enumerate() {
        let chunk_no = ci as u32 + 1;
        if stsc.is_empty() || stsc[0].0 > chunk_no {
            return Err(DemuxError::Invalid("stsc"));
        }
        while j + 1 < stsc.len() && stsc[j + 1].0 <= chunk_no {
            j += 1;
        }
        let mut off = chunk_off;
        for _ in 0..stsc[j].1 {
            let i = out.len();
            if i >= count {
                break;
            }
            let keyframe = match &stss {
                None => true,
                Some(s) => s.binary_search(&(i as u32 + 1)).is_ok(),
            };
            out.push(Mp4Sample {
                track_id,
                dts,
                pts: dts + ctos[i] as i64,
                duration: durations[i],
                keyframe,
                offset: off,
                size: sizes[i],
            });
            dts += durations[i] as i64;
            off += sizes[i] as u64;
        }
    }
    if out.len() < count {
        return Err(DemuxError::Invalid("sample table shorter than stsz"));
    }
    Ok(out)
}

// ---------------------------------------------------------------- moof

/// Parse a `moof` payload into samples. `moof_start` is the absolute offset
/// of the `moof` box; sample offsets are absolute in the same space.
/// `next_dts` carries each track's running decode time for fragments without
/// `tfdt`, and is updated.
pub fn parse_moof(
    payload: &[u8],
    moof_start: u64,
    moov: &Moov,
    next_dts: &mut HashMap<u32, u64>,
) -> R<Vec<Mp4Sample>> {
    let mut out = Vec::new();
    for (t, p) in children(payload)? {
        if &t != b"traf" {
            continue;
        }
        let kids = children(p)?;
        let tfhd = find(&kids, b"tfhd").ok_or(DemuxError::Invalid("traf without tfhd"))?;
        let mut r = ByteReader::new(tfhd);
        r.skip(1)?;
        let tf = r.u24()?;
        let id = r.u32()?;
        let trex = moov.trex_for(id);
        let mut base = moof_start;
        if tf & 0x1 != 0 {
            base = r.u64()?;
        }
        if tf & 0x2 != 0 {
            r.skip(4)?;
        }
        let def_dur = if tf & 0x8 != 0 { r.u32()? } else { trex.default_duration };
        let def_size = if tf & 0x10 != 0 { r.u32()? } else { trex.default_size };
        let def_flags = if tf & 0x20 != 0 { r.u32()? } else { trex.default_flags };

        let mut dts: u64 = match find(&kids, b"tfdt") {
            Some(d) => {
                let mut r = ByteReader::new(d);
                let ver = r.u8()?;
                r.skip(3)?;
                if ver == 1 {
                    r.u64()?
                } else {
                    r.u32()? as u64
                }
            }
            None => next_dts.get(&id).copied().unwrap_or(0),
        };
        let mut cur = base;
        for (t2, tp) in &kids {
            if t2 != b"trun" {
                continue;
            }
            let mut r = ByteReader::new(tp);
            r.skip(1)?;
            let flags = r.u24()?;
            let count = r.u32()? as usize;
            if count > 1 << 20 {
                return Err(DemuxError::Invalid("trun sample count"));
            }
            if flags & 0x1 != 0 {
                let d = r.i32()?;
                cur = (base as i64 + d as i64) as u64;
            }
            let first_flags = if flags & 0x4 != 0 { Some(r.u32()?) } else { None };
            for i in 0..count {
                let dur = if flags & 0x100 != 0 { r.u32()? } else { def_dur };
                let size = if flags & 0x200 != 0 { r.u32()? } else { def_size };
                let sflags = if flags & 0x400 != 0 {
                    r.u32()?
                } else if i == 0 && first_flags.is_some() {
                    first_flags.unwrap_or(0)
                } else {
                    def_flags
                };
                let cto = if flags & 0x800 != 0 { r.i32()? } else { 0 };
                out.push(Mp4Sample {
                    track_id: id,
                    dts: dts as i64,
                    pts: dts as i64 + cto as i64,
                    duration: dur,
                    keyframe: sflags & 0x1_0000 == 0,
                    offset: cur,
                    size,
                });
                dts += dur as u64;
                cur += size as u64;
            }
        }
        next_dts.insert(id, dts);
    }
    Ok(out)
}

// ---------------------------------------------------------------- sidx

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SidxRef {
    /// The reference points at another `sidx` rather than media.
    pub is_sidx: bool,
    pub size: u32,
    pub duration: u32,
    pub starts_with_sap: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sidx {
    pub reference_id: u32,
    pub timescale: u32,
    pub earliest_presentation_time: u64,
    /// Offset of the first referenced byte from the end of the `sidx` box.
    pub first_offset: u64,
    pub references: Vec<SidxRef>,
}

pub fn parse_sidx(p: &[u8]) -> R<Sidx> {
    let mut r = ByteReader::new(p);
    let ver = r.u8()?;
    r.skip(3)?;
    let reference_id = r.u32()?;
    let timescale = r.u32()?;
    let (ept, first_offset) = if ver == 0 { (r.u32()? as u64, r.u32()? as u64) } else { (r.u64()?, r.u64()?) };
    r.skip(2)?;
    let n = r.u16()? as usize;
    let mut references = Vec::with_capacity(n);
    for _ in 0..n {
        let a = r.u32()?;
        let duration = r.u32()?;
        let c = r.u32()?;
        references.push(SidxRef {
            is_sidx: a & 0x8000_0000 != 0,
            size: a & 0x7fff_ffff,
            duration,
            starts_with_sap: c & 0x8000_0000 != 0,
        });
    }
    Ok(Sidx { reference_id, timescale, earliest_presentation_time: ept, first_offset, references })
}

// ---------------------------------------------------------------- file

/// Parse a complete file: `moov` with its sample tables plus every `moof`
/// fragment appended to the matching track's `samples`.
pub fn parse_file(data: &[u8]) -> R<Moov> {
    let mut moov: Option<Moov> = None;
    let mut next_dts = HashMap::new();
    let mut frag = Vec::new();
    let mut pos = 0usize;
    while pos < data.len() {
        let h = read_box_header(&data[pos..])?.ok_or(DemuxError::Invalid("truncated box header"))?;
        let remaining = (data.len() - pos) as u64;
        let size = if h.size == 0 { remaining } else { h.size };
        if size > remaining {
            return Err(DemuxError::Invalid("box overruns file"));
        }
        let payload = &data[pos + h.header_len..pos + size as usize];
        match &h.typ {
            b"moov" => moov = Some(parse_moov(payload)?),
            b"moof" => {
                let m = moov.as_ref().ok_or(DemuxError::MissingInit)?;
                frag.extend(parse_moof(payload, pos as u64, m, &mut next_dts)?);
            }
            _ => {}
        }
        pos += size as usize;
    }
    let mut moov = moov.ok_or(DemuxError::Invalid("file without moov"))?;
    for s in frag {
        if let Some(t) = moov.tracks.iter_mut().find(|t| t.info.id == s.track_id) {
            t.samples.push(s);
        }
    }
    Ok(moov)
}

#[cfg(test)]
mod tests {
    use super::super::fixtures::*;
    use super::*;

    #[test]
    fn box_header_small_and_large() {
        let b = bx(b"free", &[1, 2, 3]);
        let h = read_box_header(&b).unwrap().unwrap();
        assert_eq!(h, BoxHeader { typ: *b"free", header_len: 8, size: 11 });

        let mut v = 1u32.to_be_bytes().to_vec();
        v.extend(b"mdat");
        v.extend(24u64.to_be_bytes());
        let h = read_box_header(&v).unwrap().unwrap();
        assert_eq!((h.header_len, h.size), (16, 24));
    }

    #[test]
    fn box_header_needs_more_data_and_rejects_bad_size() {
        assert_eq!(read_box_header(&[0, 0, 0]).unwrap(), None);
        let mut v = 1u32.to_be_bytes().to_vec();
        v.extend(b"mdat");
        assert_eq!(read_box_header(&v).unwrap(), None);
        let mut bad = 4u32.to_be_bytes().to_vec();
        bad.extend(b"free");
        assert!(read_box_header(&bad).is_err());
    }

    #[test]
    fn children_walks_and_rejects_overrun() {
        let d = [bx(b"aaaa", &[1]), bx(b"bbbb", &[2, 3])].concat();
        let k = children(&d).unwrap();
        assert_eq!(k.len(), 2);
        assert_eq!(&k[1].0, b"bbbb");
        assert_eq!(k[1].1, &[2, 3]);
        let mut cut = d.clone();
        cut.pop();
        assert!(children(&cut).is_err());
    }

    #[test]
    fn init_segment_tracks() {
        let m = parse_file(&mp4_init()).unwrap();
        assert!(m.fragmented);
        assert_eq!(m.tracks.len(), 2);
        let v = &m.tracks[0];
        assert_eq!(v.timescale, 1000);
        assert_eq!(v.info.id, 1);
        assert_eq!(v.info.kind, TrackKind::Video);
        assert_eq!(v.info.codec, CodecId::H264);
        assert_eq!((v.info.width, v.info.height), (320, 240));
        assert_eq!(v.info.codec_private, avcc());
        let a = &m.tracks[1];
        assert_eq!(a.timescale, 48000);
        assert_eq!(a.info.codec, CodecId::Aac);
        assert_eq!((a.info.channels, a.info.sample_rate), (2, 48000));
        assert_eq!(a.info.codec_private, ASC_LC_48K_STEREO.to_vec());
        assert_eq!(m.trex.len(), 2);
    }

    #[test]
    fn video_codec_config_boxes() {
        for (fourcc, cfg, codec) in [
            (b"hvc1", b"hvcC", CodecId::Hevc),
            (b"hev1", b"hvcC", CodecId::Hevc),
            (b"vp09", b"vpcC", CodecId::Vp9),
            (b"av01", b"av1C", CodecId::Av1),
            (b"avc3", b"avcC", CodecId::H264),
        ] {
            let t = trak(7, b"vide", 90000, video_entry(fourcc, cfg, &[9, 8, 7], 1280, 720));
            let m = parse_moov(&t).unwrap();
            let i = &m.tracks[0].info;
            assert_eq!(i.codec, codec, "{fourcc:?}");
            assert_eq!(i.codec_private, vec![9, 8, 7]);
            assert_eq!((i.width, i.height, i.id), (1280, 720, 7));
        }
    }

    #[test]
    fn opus_dops_becomes_opus_head() {
        let mut dops = vec![0, 2];
        dops.extend(312u16.to_be_bytes());
        dops.extend(48000u32.to_be_bytes());
        dops.extend(0u16.to_be_bytes());
        dops.push(0);
        let t = trak(3, b"soun", 48000, audio_entry(b"Opus", 2, 48000, bx(b"dOps", &dops)));
        let m = parse_moov(&t).unwrap();
        let i = &m.tracks[0].info;
        assert_eq!(i.codec, CodecId::Opus);
        assert_eq!(i.channels, 2);
        assert_eq!(i.codec_private, opus_head());
    }

    #[test]
    fn opus_dops_with_mapping_table_keeps_table() {
        let mut dops = vec![0, 6];
        dops.extend(0u16.to_be_bytes());
        dops.extend(48000u32.to_be_bytes());
        dops.extend(0u16.to_be_bytes());
        dops.push(1);
        dops.extend([4, 2, 0, 4, 1, 2, 3, 5]);
        let head = dops_to_opus_head(&dops).unwrap();
        assert_eq!(&head[..8], b"OpusHead");
        assert_eq!(head[9], 6);
        assert_eq!(&head[19..], &[4, 2, 0, 4, 1, 2, 3, 5]);
    }

    #[test]
    fn unsupported_handlers_are_skipped() {
        let v = trak(1, b"vide", 1000, video_entry(b"avc1", b"avcC", &avcc(), 16, 16));
        let text = trak(2, b"text", 1000, bx(b"tx3g", &[0; 8]));
        let m = parse_moov(&[v, text].concat()).unwrap();
        assert_eq!(m.tracks.len(), 1);
    }

    #[test]
    fn missing_codec_config_is_an_error() {
        let t = trak(1, b"vide", 1000, video_entry(b"avc1", b"free", &[], 16, 16));
        assert!(parse_moov(&t).is_err());
    }

    #[test]
    fn asc_parsing() {
        assert_eq!(parse_asc(&[0x11, 0x90]).unwrap(), (48000, 2));
        assert_eq!(parse_asc(&[0x12, 0x10]).unwrap(), (44100, 2));
        // explicit 24-bit rate: aot 2, idx 15, rate 0x00bb80, ch 1
        // 00010 1111 000000001011101110000000 0001
        assert_eq!(parse_asc(&[0x17, 0x80, 0x5d, 0xc0, 0x08]).unwrap(), (48000, 1));
        assert!(parse_asc(&[0x11]).is_err());
    }

    #[test]
    fn esds_with_optional_descriptor_fields() {
        // ES_Descr with streamDependence flag set (2 extra bytes)
        let mut dss = vec![0x05, 2, 0x11, 0x90];
        let mut dc = vec![0x04, (13 + dss.len()) as u8, 0x40, 0x15];
        dc.extend([0u8; 11]);
        dc.append(&mut dss);
        let mut es = vec![0x03, 0, 0, 1, 0x80, 0xaa, 0xbb];
        es.extend(dc);
        es[1] = (es.len() - 2) as u8;
        let p = full(b"esds", 0, 0, &es);
        assert_eq!(parse_esds(&p[8..]).unwrap(), vec![0x11, 0x90]);
    }

    #[test]
    fn moof_samples() {
        let m = parse_file(&mp4_init()).unwrap();
        let seg = mp4_segment(1);
        let hdr = read_box_header(&seg).unwrap().unwrap();
        let mut nd = HashMap::new();
        let s = parse_moof(&seg[8..hdr.size as usize], 1000, &m, &mut nd).unwrap();
        assert_eq!(s.len(), 6);
        let v: Vec<_> = s.iter().filter(|x| x.track_id == 1).collect();
        assert_eq!(v.iter().map(|x| x.dts).collect::<Vec<_>>(), vec![120, 160, 200]);
        assert_eq!(v.iter().map(|x| x.keyframe).collect::<Vec<_>>(), vec![true, false, false]);
        assert_eq!(v.iter().map(|x| x.size).collect::<Vec<_>>(), V_SIZES.to_vec());
        assert!(v.iter().all(|x| x.duration == 40));
        let a: Vec<_> = s.iter().filter(|x| x.track_id == 2).collect();
        assert_eq!(a[0].dts, 5760);
        assert_eq!(a[1].dts, 5760 + 1920);
        assert!(a.iter().all(|x| x.keyframe));
        // data offsets are absolute: moof_start + moof size + 8 + running
        assert_eq!(v[0].offset, 1000 + hdr.size + 8);
        assert_eq!(v[1].offset, v[0].offset + 5);
        assert_eq!(a[0].offset, v[2].offset + 3);
        assert_eq!(nd[&1], 240);
    }

    #[test]
    fn moof_composition_offsets_and_defaults() {
        let m = parse_file(&mp4_init()).unwrap();
        let s = vec![
            FSample { dur: 40, size: 1, flags: KEY, cto: 80 },
            FSample { dur: 40, size: 1, flags: NON_KEY, cto: -40 },
        ];
        let seg = moof_mdat(1, &[(1, 0, s)]);
        let hdr = read_box_header(&seg).unwrap().unwrap();
        let out = parse_moof(&seg[8..hdr.size as usize], 0, &m, &mut HashMap::new()).unwrap();
        assert_eq!((out[0].dts, out[0].pts), (0, 80));
        assert_eq!((out[1].dts, out[1].pts), (40, 0));
    }

    #[test]
    fn moof_without_tfdt_continues_previous_run() {
        let m = parse_file(&mp4_init()).unwrap();
        let moof = bx(
            b"moof",
            &bx(
                b"traf",
                &[
                    full(b"tfhd", 0, 0x2_0000 | 0x8 | 0x10 | 0x20, &[0, 0, 0, 1, 0, 0, 0, 40, 0, 0, 0, 2, 0x01, 0x01, 0, 0]),
                    // no data offset, no per-sample fields: 2 samples of defaults
                    full(b"trun", 0, 0, &[0, 0, 0, 2]),
                ]
                .concat(),
            ),
        );
        let mut nd = HashMap::new();
        nd.insert(1u32, 500u64);
        let s = parse_moof(&moof[8..], 64, &m, &mut nd).unwrap();
        assert_eq!(s.len(), 2);
        assert_eq!((s[0].dts, s[1].dts), (500, 540));
        assert_eq!((s[0].offset, s[1].offset), (64, 66));
        assert!(!s[0].keyframe, "default flags carry the non-sync bit");
        assert_eq!(nd[&1], 580);
    }

    #[test]
    fn moof_first_sample_flags_override_default() {
        let m = parse_file(&mp4_init()).unwrap();
        let mut trun = vec![0, 0, 0, 2];
        trun.extend(KEY.to_be_bytes());
        let moof = bx(
            b"moof",
            &bx(
                b"traf",
                &[
                    full(b"tfhd", 0, 0x2_0000 | 0x8 | 0x10 | 0x20, &[0, 0, 0, 1, 0, 0, 0, 40, 0, 0, 0, 2, 0x00, 0x01, 0, 0]),
                    full(b"trun", 0, 0x4, &trun),
                ]
                .concat(),
            ),
        );
        let s = parse_moof(&moof[8..], 0, &m, &mut HashMap::new()).unwrap();
        assert!(s[0].keyframe);
        assert!(!s[1].keyframe);
    }

    #[test]
    fn moof_base_data_offset_wins() {
        let m = parse_file(&mp4_init()).unwrap();
        let mut tfhd = vec![0, 0, 0, 1];
        tfhd.extend(5000u64.to_be_bytes());
        let mut trun = vec![0, 0, 0, 1];
        trun.extend(16i32.to_be_bytes());
        trun.extend(40u32.to_be_bytes());
        trun.extend(9u32.to_be_bytes());
        let moof = bx(
            b"moof",
            &bx(b"traf", &[full(b"tfhd", 0, 0x1, &tfhd), full(b"trun", 0, 0x1 | 0x100 | 0x200, &trun)].concat()),
        );
        let s = parse_moof(&moof[8..], 77, &m, &mut HashMap::new()).unwrap();
        assert_eq!(s[0].offset, 5016);
        assert_eq!(s[0].size, 9);
    }

    #[test]
    fn truncated_trun_is_an_error() {
        let m = parse_file(&mp4_init()).unwrap();
        let moof = bx(
            b"moof",
            &bx(
                b"traf",
                &[full(b"tfhd", 0, 0x2_0000, &[0, 0, 0, 1]), full(b"trun", 0, 0x100 | 0x200, &[0, 0, 0, 3, 0, 0, 0, 1])]
                    .concat(),
            ),
        );
        assert!(parse_moof(&moof[8..], 0, &m, &mut HashMap::new()).is_err());
    }

    #[test]
    fn whole_fragmented_file_collects_samples_with_correct_bytes() {
        let file = [mp4_init(), mp4_segment(0), mp4_segment(1)].concat();
        let m = parse_file(&file).unwrap();
        let v = &m.tracks[0];
        assert_eq!(v.samples.len(), 6);
        for (i, s) in v.samples.iter().enumerate() {
            let idx = i % 3;
            let bytes = &file[s.offset as usize..s.offset as usize + s.size as usize];
            assert!(bytes.iter().all(|b| *b == fill(1, idx)), "sample {i}");
        }
        let a = &m.tracks[1];
        assert_eq!(a.samples.len(), 6);
        assert_eq!(a.samples[3].dts, 5760);
    }

    #[test]
    fn fragment_before_moov_is_missing_init() {
        assert_eq!(parse_file(&mp4_segment(0)).unwrap_err(), DemuxError::MissingInit);
    }

    fn sidx_payload(ver: u8) -> Vec<u8> {
        let mut p = vec![ver, 0, 0, 0];
        p.extend(1u32.to_be_bytes());
        p.extend(1000u32.to_be_bytes());
        if ver == 0 {
            p.extend(500u32.to_be_bytes());
            p.extend(32u32.to_be_bytes());
        } else {
            p.extend(500u64.to_be_bytes());
            p.extend(32u64.to_be_bytes());
        }
        p.extend([0, 0, 0, 2]);
        p.extend(1234u32.to_be_bytes());
        p.extend(2000u32.to_be_bytes());
        p.extend(0x9000_0000u32.to_be_bytes());
        p.extend((0x8000_0000u32 | 77).to_be_bytes());
        p.extend(10u32.to_be_bytes());
        p.extend(0u32.to_be_bytes());
        p
    }

    #[test]
    fn sidx_parses_both_versions() {
        for ver in [0u8, 1] {
            let s = parse_sidx(&sidx_payload(ver)).unwrap();
            assert_eq!(s.reference_id, 1);
            assert_eq!(s.timescale, 1000);
            assert_eq!(s.earliest_presentation_time, 500);
            assert_eq!(s.first_offset, 32);
            assert_eq!(s.references.len(), 2);
            assert_eq!(
                s.references[0],
                SidxRef { is_sidx: false, size: 1234, duration: 2000, starts_with_sap: true }
            );
            assert!(s.references[1].is_sidx);
            assert_eq!(s.references[1].size, 77);
            assert!(!s.references[1].starts_with_sap);
        }
        let mut cut = sidx_payload(0);
        cut.pop();
        assert!(parse_sidx(&cut).is_err());
    }

    #[test]
    fn progressive_sample_table() {
        let u = |v: &[u32]| v.iter().flat_map(|x| x.to_be_bytes()).collect::<Vec<u8>>();
        let mut stts = u(&[1]);
        stts.extend(u(&[4, 100]));
        let mut ctts = u(&[2]);
        ctts.extend(u(&[1, 0, 3, 50]));
        let mut stsz = u(&[0, 4]);
        stsz.extend(u(&[3, 2, 4, 1]));
        let mut stsc = u(&[1]);
        stsc.extend(u(&[1, 2, 1]));
        let mut stco = u(&[2]);
        stco.extend(u(&[1000, 2000]));
        let stss = [u(&[2]), u(&[1, 3])].concat();
        let extra = [
            full(b"stts", 0, 0, &stts),
            full(b"ctts", 0, 0, &ctts),
            full(b"stsz", 0, 0, &stsz),
            full(b"stsc", 0, 0, &stsc),
            full(b"stco", 0, 0, &stco),
            full(b"stss", 0, 0, &stss),
        ]
        .concat();
        let t = trak_with(1, b"vide", 1000, extra, video_entry(b"avc1", b"avcC", &avcc(), 64, 48));
        let file = [bx(b"ftyp", b"isom\0\0\0\0"), moov(vec![t], &[])].concat();
        let m = parse_file(&file).unwrap();
        assert!(!m.fragmented);
        let s = &m.tracks[0].samples;
        assert_eq!(s.len(), 4);
        assert_eq!(s.iter().map(|x| x.offset).collect::<Vec<_>>(), vec![1000, 1003, 2000, 2004]);
        assert_eq!(s.iter().map(|x| x.dts).collect::<Vec<_>>(), vec![0, 100, 200, 300]);
        assert_eq!(s.iter().map(|x| x.pts).collect::<Vec<_>>(), vec![0, 150, 250, 350]);
        assert_eq!(s.iter().map(|x| x.keyframe).collect::<Vec<_>>(), vec![true, false, true, false]);
        assert_eq!(s.iter().map(|x| x.size).collect::<Vec<_>>(), vec![3, 2, 4, 1]);
    }

    #[test]
    fn co64_and_fixed_sample_size() {
        let u = |v: &[u32]| v.iter().flat_map(|x| x.to_be_bytes()).collect::<Vec<u8>>();
        let mut stts = u(&[1]);
        stts.extend(u(&[3, 10]));
        let stsz = u(&[7, 3]);
        let mut stsc = u(&[1]);
        stsc.extend(u(&[1, 3, 1]));
        let mut co64 = u(&[1]);
        co64.extend((1u64 << 33).to_be_bytes());
        let extra = [
            full(b"stts", 0, 0, &stts),
            full(b"stsz", 0, 0, &stsz),
            full(b"stsc", 0, 0, &stsc),
            full(b"co64", 0, 0, &co64),
        ]
        .concat();
        let t = trak_with(1, b"vide", 1000, extra, video_entry(b"avc1", b"avcC", &avcc(), 8, 8));
        let m = parse_moov(&t).unwrap();
        let s = &m.tracks[0].samples;
        assert_eq!(s[2].offset, (1u64 << 33) + 14);
        assert!(s.iter().all(|x| x.keyframe && x.size == 7));
    }

    #[test]
    fn stsz_count_larger_than_data_is_rejected() {
        let u = |v: &[u32]| v.iter().flat_map(|x| x.to_be_bytes()).collect::<Vec<u8>>();
        let extra = [full(b"stsz", 0, 0, &u(&[0, 1000]))].concat();
        let t = trak_with(1, b"vide", 1000, extra, video_entry(b"avc1", b"avcC", &avcc(), 8, 8));
        assert!(parse_moov(&t).is_err());
    }
}
