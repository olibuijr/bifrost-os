//! MP4 / ISO BMFF demuxer: one video (avc1/avc3/hvc1/hev1) and one audio
//! (mp4a AAC) track, progressive or fragmented, from a `Source` that may
//! still be growing.
//!
//! Progressive files expand `stbl` into a full sample table at open.
//! Fragmented files (`moof`) are parsed one fragment at a time as samples
//! are consumed, so playback can start before the file is complete.
//! Every read that touches bytes not yet available comes back as
//! `Error::NeedData` with the cursor untouched; the same call succeeds
//! once the range exists.

use super::source::Source;
use super::Error;

/// The elementary stream carried by a track.
#[derive(Clone, Debug, PartialEq)]
pub enum Codec {
    /// `avcC` box payload (SPS/PPS, NAL length size) and coded size.
    H264 { avcc: Vec<u8>, width: u32, height: u32 },
    /// `hvcC` box payload and coded size.
    Hevc { hvcc: Vec<u8>, width: u32, height: u32 },
    /// AudioSpecificConfig from `esds`.
    Aac { asc: Vec<u8>, sample_rate: u32, channels: u16 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Video,
    Audio,
}

/// One entry of the sample table.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SampleInfo {
    pub offset: u64,
    pub size: u32,
    /// decode time in track timescale units
    pub dts: u64,
    /// composition offset (pts - dts) in timescale units
    pub cts: i32,
    pub sync: bool,
}

/// A sample with its payload, times in milliseconds.
#[derive(Clone, Debug, PartialEq)]
pub struct Sample {
    pub data: Vec<u8>,
    pub pts_ms: f64,
    pub dts_ms: f64,
    pub duration_ms: f64,
    pub sync: bool,
}

#[derive(Clone, Debug)]
pub struct Track {
    pub id: u32,
    pub kind: Kind,
    pub timescale: u32,
    pub codec: Codec,
    /// media duration in timescale units (0 if unknown)
    pub duration: u64,
    pub samples: Vec<SampleInfo>,
    /// edit-list shift subtracted from composition times
    pts_offset: i64,
    /// trex defaults for fragments: [duration, size, flags]
    defaults: [u32; 3],
    /// read cursor into `samples`
    next: usize,
}

impl Track {
    fn ms(&self, t: i64) -> f64 {
        t as f64 * 1000.0 / self.timescale as f64
    }

    /// Presentation time of a sample in ms (never negative).
    pub fn pts_ms(&self, s: &SampleInfo) -> f64 {
        self.ms((s.dts as i64 + s.cts as i64 - self.pts_offset).max(0))
    }

    fn duration_of(&self, i: usize) -> f64 {
        match self.samples.get(i + 1) {
            Some(n) => self.ms(n.dts as i64 - self.samples[i].dts as i64),
            None => self.samples.len().checked_sub(2).map_or(0.0, |p| self.ms(self.samples[i].dts as i64 - self.samples[p].dts as i64)),
        }
    }
}

pub struct Demuxer {
    tracks: Vec<Track>,
    movie_timescale: u32,
    movie_duration: u64,
    /// fragment duration from `mehd`, if any
    fragment_duration: u64,
    fragmented: bool,
    /// offset of the next top-level box to scan for fragments
    frag_pos: u64,
    frag_done: bool,
}

/// Largest sample we are willing to allocate.
const MAX_SAMPLE: u32 = 64 << 20;
/// Most samples one track may declare; a crafted count must not allocate gigabytes.
const MAX_SAMPLES: usize = 1 << 24;

/// Bounds-checked big-endian readers: out-of-range reads yield 0 (never
/// panic); callers validate box and table lengths with `need` / `entries`.
fn be16(b: &[u8], o: usize) -> u16 {
    match o.checked_add(2).and_then(|e| b.get(o..e)) {
        Some(x) => u16::from_be_bytes([x[0], x[1]]),
        None => 0,
    }
}
fn be32(b: &[u8], o: usize) -> u32 {
    match o.checked_add(4).and_then(|e| b.get(o..e)) {
        Some(x) => u32::from_be_bytes([x[0], x[1], x[2], x[3]]),
        None => 0,
    }
}
fn be64(b: &[u8], o: usize) -> u64 {
    (be32(b, o) as u64) << 32 | be32(b, o.saturating_add(4)) as u64
}

/// The box must hold at least `n` bytes.
fn need(b: &[u8], n: usize) -> Result<(), Error> {
    if b.len() < n {
        invalid("box too short")
    } else {
        Ok(())
    }
}

/// Entry count of a table box (count at offset 4) after checking that
/// `head + count * entry` bytes fit in the box.
fn entries(b: &[u8], head: usize, entry: usize) -> Result<usize, Error> {
    need(b, 8)?;
    let n = be32(b, 4) as usize;
    match n.checked_mul(entry).and_then(|x| x.checked_add(head)) {
        Some(total) if total <= b.len() => Ok(n),
        _ => invalid("table larger than its box"),
    }
}

/// Box header: (type, header length, total size); size 0 means "to the end".
pub(crate) fn header(b: &[u8]) -> Option<([u8; 4], usize, u64)> {
    if b.len() < 8 {
        return None;
    }
    let kind = [b[4], b[5], b[6], b[7]];
    match be32(b, 0) {
        1 if b.len() >= 16 => Some((kind, 16, be64(b, 8))),
        1 => None,
        n => Some((kind, 8, n as u64)),
    }
}

/// Iterates the child boxes of a container payload as (type, payload).
pub(crate) fn children(data: &[u8]) -> impl Iterator<Item = ([u8; 4], &[u8])> {
    let mut pos = 0;
    std::iter::from_fn(move || {
        let (kind, hl, size) = header(&data[pos..])?;
        let size = if size == 0 { (data.len() - pos) as u64 } else { size };
        if size < hl as u64 || pos as u64 + size > data.len() as u64 {
            return None;
        }
        let body = &data[pos + hl..pos + size as usize];
        pos += size as usize;
        Some((kind, body))
    })
}

pub(crate) fn child<'a>(data: &'a [u8], kind: &[u8; 4]) -> Option<&'a [u8]> {
    children(data).find(|(k, _)| k == kind).map(|(_, b)| b)
}

fn invalid<T>(what: &'static str) -> Result<T, Error> {
    Err(Error::Invalid(what))
}

/// Reads `len` bytes at `offset`.
fn read(src: &mut dyn Source, offset: u64, len: usize) -> Result<Vec<u8>, Error> {
    let mut buf = vec![0u8; len];
    src.read_at(offset, &mut buf)?;
    Ok(buf)
}

/// Reads a whole top-level box at `offset`: (type, header length, bytes).
fn read_box(src: &mut dyn Source, offset: u64) -> Result<([u8; 4], usize, Vec<u8>), Error> {
    let head = read(src, offset, 16)?;
    let (kind, hl, size) = header(&head).ok_or(Error::Invalid("box header"))?;
    if size < hl as u64 || size > (256 << 20) {
        return invalid("box size");
    }
    Ok((kind, hl, read(src, offset, size as usize)?))
}

impl Demuxer {
    /// Scans the top level for `moov` (skipping `mdat` without reading it)
    /// and builds the sample tables.
    pub fn open(src: &mut dyn Source) -> Result<Demuxer, Error> {
        let mut pos = 0;
        loop {
            let head = read(src, pos, 16)?;
            let (kind, hl, size) = header(&head).ok_or(Error::Invalid("box header"))?;
            if &kind == b"moov" {
                let (_, _, body) = read_box(src, pos)?;
                let mut d = Self::from_moov(&body[hl..])?;
                d.frag_pos = pos + size;
                return Ok(d);
            }
            if size == 0 || size < hl as u64 {
                return invalid("no moov box");
            }
            pos += size;
        }
    }

    fn from_moov(moov: &[u8]) -> Result<Demuxer, Error> {
        let mvhd = child(moov, b"mvhd").ok_or(Error::Invalid("mvhd"))?;
        let v1 = mvhd.first() == Some(&1);
        need(mvhd, if v1 { 32 } else { 20 })?;
        let (movie_timescale, movie_duration) = if v1 { (be32(mvhd, 20), be64(mvhd, 24)) } else { (be32(mvhd, 12), be32(mvhd, 16) as u64) };
        let mut tracks = Vec::new();
        for (_, trak) in children(moov).filter(|(k, _)| k == b"trak") {
            if let Some(t) = parse_trak(trak, movie_timescale)? {
                if tracks.iter().all(|x: &Track| x.kind != t.kind) {
                    tracks.push(t);
                }
            }
        }
        if tracks.is_empty() {
            return invalid("no playable track");
        }
        let mut fragment_duration = 0;
        let fragmented = match child(moov, b"mvex") {
            Some(mvex) => {
                if let Some(mehd) = child(mvex, b"mehd") {
                    fragment_duration = if mehd.first() == Some(&1) { be64(mehd, 4) } else { be32(mehd, 4) as u64 };
                }
                for (_, trex) in children(mvex).filter(|(k, b)| k == b"trex" && b.len() >= 24) {
                    if let Some(t) = tracks.iter_mut().find(|t| t.id == be32(trex, 4)) {
                        t.defaults = [be32(trex, 12), be32(trex, 16), be32(trex, 20)];
                    }
                }
                true
            }
            None => false,
        };
        Ok(Demuxer { tracks, movie_timescale, movie_duration, fragment_duration, fragmented, frag_pos: 0, frag_done: !fragmented })
    }

    pub fn track(&self, kind: Kind) -> Option<&Track> {
        self.tracks.iter().find(|t| t.kind == kind)
    }

    fn track_mut(&mut self, kind: Kind) -> Option<&mut Track> {
        self.tracks.iter_mut().find(|t| t.kind == kind)
    }

    pub fn video(&self) -> Option<&Track> {
        self.track(Kind::Video)
    }

    pub fn audio(&self) -> Option<&Track> {
        self.track(Kind::Audio)
    }

    /// Movie duration in ms, if the file declares one.
    pub fn duration_ms(&self) -> Option<f64> {
        let movie = |t: u64| t as f64 * 1000.0 / self.movie_timescale.max(1) as f64;
        if self.fragment_duration > 0 {
            return Some(movie(self.fragment_duration));
        }
        if self.movie_duration > 0 && !self.fragmented {
            return Some(movie(self.movie_duration));
        }
        let longest = self.tracks.iter().filter(|t| t.duration > 0).map(|t| t.ms(t.duration as i64)).fold(0.0, f64::max);
        (longest > 0.0 && !self.fragmented).then_some(longest)
    }

    /// The next sample of a track, None at the end of the stream.
    pub fn next(&mut self, kind: Kind, src: &mut dyn Source) -> Result<Option<Sample>, Error> {
        loop {
            let Some(t) = self.track_mut(kind) else { return Ok(None) };
            if let Some(&info) = t.samples.get(t.next) {
                if info.size > MAX_SAMPLE {
                    return invalid("sample too large");
                }
                let data = read(src, info.offset, info.size as usize)?;
                let i = t.next;
                t.next += 1;
                return Ok(Some(Sample { data, pts_ms: t.pts_ms(&info), dts_ms: t.ms(info.dts as i64), duration_ms: t.duration_of(i), sync: info.sync }));
            }
            if self.frag_done || !self.read_fragment(src)? {
                return Ok(None);
            }
        }
    }

    /// Parses the next `moof` (skipping anything else); false at the end.
    fn read_fragment(&mut self, src: &mut dyn Source) -> Result<bool, Error> {
        loop {
            if self.frag_pos >= src.size() {
                let last = self.tracks.iter().filter_map(|t| t.samples.last().map(|s| t.ms(s.dts as i64))).fold(0.0, f64::max);
                let declared = self.duration_ms().unwrap_or(0.0);
                if declared <= 0.0 || last + 1000.0 >= declared {
                    self.frag_done = true;
                    return Ok(false);
                }
                return Err(Error::NeedData { offset: self.frag_pos, len: 16 });
            }
            let head = read(src, self.frag_pos, 16)?;
            let (kind, hl, size) = header(&head).ok_or(Error::Invalid("box header"))?;
            if size == 0 {
                self.frag_done = true;
                return Ok(false);
            }
            if size < hl as u64 {
                return invalid("fragment box size");
            }
            if &kind == b"moof" {
                let (_, _, body) = read_box(src, self.frag_pos)?;
                let moof_start = self.frag_pos;
                self.frag_pos += size;
                self.parse_moof(&body[hl..], moof_start)?;
                return Ok(true);
            }
            self.frag_pos += size;
        }
    }

    fn parse_moof(&mut self, moof: &[u8], moof_start: u64) -> Result<(), Error> {
        for (_, traf) in children(moof).filter(|(k, _)| k == b"traf") {
            let tfhd = child(traf, b"tfhd").ok_or(Error::Invalid("tfhd"))?;
            need(tfhd, 8)?;
            let flags = be32(tfhd, 0) & 0xff_ffff;
            let track_id = be32(tfhd, 4);
            let Some(t) = self.tracks.iter_mut().find(|t| t.id == track_id) else { continue };
            let mut p = 8;
            let mut base = moof_start;
            if flags & 1 != 0 {
                base = be64(tfhd, p);
                p += 8;
            }
            if flags & 2 != 0 {
                p += 4;
            }
            let mut defaults = t.defaults;
            for (bit, slot) in [(8, 0), (0x10, 1), (0x20, 2)] {
                if flags & bit != 0 {
                    defaults[slot] = be32(tfhd, p);
                    p += 4;
                }
            }
            let mut dts = match child(traf, b"tfdt") {
                Some(tfdt) if tfdt.first() == Some(&1) => be64(tfdt, 4),
                Some(tfdt) => be32(tfdt, 4) as u64,
                None => t.samples.last().map_or(0, |s| s.dts.wrapping_add(defaults[0] as u64)),
            };
            let mut data_pos = base;
            for (_, trun) in children(traf).filter(|(k, _)| k == b"trun") {
                need(trun, 8)?;
                let (version, tflags) = (trun[0], be32(trun, 0) & 0xff_ffff);
                let count = be32(trun, 4) as usize;
                let per = 4 * (tflags & 0xf00).count_ones() as usize;
                let fixed = 8 + 4 * (tflags & 1 != 0) as usize + 4 * (tflags & 4 != 0) as usize;
                let room = trun.len().saturating_sub(fixed);
                if per > 0 && count.checked_mul(per).map_or(true, |x| x > room) {
                    return invalid("trun larger than its box");
                }
                if per == 0 && count > 1 << 20 {
                    return invalid("trun sample count");
                }
                let mut p = 8;
                if tflags & 1 != 0 {
                    data_pos = (base as i64).wrapping_add(be32(trun, p) as i32 as i64) as u64;
                    p += 4;
                }
                let mut first_flags = None;
                if tflags & 4 != 0 {
                    first_flags = Some(be32(trun, p));
                    p += 4;
                }
                for i in 0..count {
                    let mut dur = defaults[0];
                    let mut size = defaults[1];
                    let mut sflags = if i == 0 { first_flags.unwrap_or(defaults[2]) } else { defaults[2] };
                    let mut cts = 0i32;
                    if tflags & 0x100 != 0 {
                        dur = be32(trun, p);
                        p += 4;
                    }
                    if tflags & 0x200 != 0 {
                        size = be32(trun, p);
                        p += 4;
                    }
                    if tflags & 0x400 != 0 {
                        sflags = be32(trun, p);
                        p += 4;
                    }
                    if tflags & 0x800 != 0 {
                        cts = if version == 0 { be32(trun, p).min(i32::MAX as u32) as i32 } else { be32(trun, p) as i32 };
                        p += 4;
                    }
                    t.samples.push(SampleInfo { offset: data_pos, size, dts, cts, sync: sflags & 0x1_0000 == 0 });
                    data_pos = data_pos.wrapping_add(size as u64);
                    dts = dts.wrapping_add(dur as u64);
                }
            }
        }
        Ok(())
    }

    /// Positions both tracks at the last video keyframe at or before `ms`
    /// (or the exact audio sample without video); returns its time.
    pub fn seek_ms(&mut self, ms: f64, src: &mut dyn Source) -> Result<f64, Error> {
        let lead = self.video().map_or(Kind::Audio, |_| Kind::Video);
        let target = ms.max(0.0);
        // fragmented files only know samples that have been parsed so far
        while self.fragmented && !self.frag_done {
            let covered = self.track(lead).and_then(|t| t.samples.last().map(|s| t.ms(s.dts as i64))).unwrap_or(-1.0) >= target;
            if covered || !self.read_fragment(src)? {
                break;
            }
        }
        let t = self.track_mut(lead).ok_or(Error::Invalid("no track"))?;
        let key = (0..t.samples.len()).rev().find(|&i| t.samples[i].sync && t.pts_ms(&t.samples[i]) <= target).unwrap_or(0);
        t.next = key;
        let at = t.samples.get(key).map_or(0.0, |s| t.pts_ms(s));
        // the other track starts at the target: video frames before it are dropped, so audio
        // must not play from the earlier keyframe
        if let Some(a) = self.tracks.iter_mut().find(|t| t.kind != lead) {
            a.next = a.samples.partition_point(|s| a.pts_ms(s) < target);
        }
        Ok(at)
    }
}

fn parse_trak(trak: &[u8], movie_timescale: u32) -> Result<Option<Track>, Error> {
    let tkhd = child(trak, b"tkhd").ok_or(Error::Invalid("tkhd"))?;
    need(tkhd, if tkhd.first() == Some(&1) { 24 } else { 16 })?;
    let id = if tkhd[0] == 1 { be32(tkhd, 20) } else { be32(tkhd, 12) };
    let mdia = child(trak, b"mdia").ok_or(Error::Invalid("mdia"))?;
    let mdhd = child(mdia, b"mdhd").ok_or(Error::Invalid("mdhd"))?;
    need(mdhd, if mdhd.first() == Some(&1) { 32 } else { 20 })?;
    let (timescale, duration) = if mdhd[0] == 1 { (be32(mdhd, 20), be64(mdhd, 24)) } else { (be32(mdhd, 12), be32(mdhd, 16) as u64) };
    if timescale == 0 {
        return invalid("timescale");
    }
    let hdlr = child(mdia, b"hdlr").ok_or(Error::Invalid("hdlr"))?;
    need(hdlr, 12)?;
    let kind = match &hdlr[8..12] {
        b"vide" => Kind::Video,
        b"soun" => Kind::Audio,
        _ => return Ok(None),
    };
    let stbl = child(mdia, b"minf").and_then(|m| child(m, b"stbl")).ok_or(Error::Invalid("stbl"))?;
    let stsd = child(stbl, b"stsd").ok_or(Error::Invalid("stsd"))?;
    let Some(codec) = parse_stsd(stsd, kind)? else { return Ok(None) };
    let samples = sample_table(stbl)?;
    let mut pts_offset = 0i64;
    if let Some(elst) = child(trak, b"edts").and_then(|e| child(e, b"elst")) {
        need(elst, 8)?;
        let v = elst[0];
        let n = entries(elst, 8, if v == 1 { 20 } else { 12 })?;
        let mut p = 8;
        for _ in 0..n {
            let (seg, media) = if v == 1 { (be64(elst, p), be64(elst, p + 8) as i64) } else { (be32(elst, p) as u64, be32(elst, p + 4) as i32 as i64) };
            p += if v == 1 { 20 } else { 12 };
            if media < 0 {
                // empty edit: presentation starts late by `seg` (movie timescale)
                pts_offset -= (seg as f64 * timescale as f64 / movie_timescale.max(1) as f64) as i64;
            } else {
                pts_offset = pts_offset.wrapping_add(media);
                break;
            }
        }
    }
    Ok(Some(Track { id, kind, timescale, codec, duration, samples, pts_offset, defaults: [0; 3], next: 0 }))
}

fn parse_stsd(stsd: &[u8], kind: Kind) -> Result<Option<Codec>, Error> {
    need(stsd, 8)?;
    for (fourcc, entry) in children(&stsd[8..]) {
        match (&fourcc, kind) {
            (b"avc1" | b"avc3", Kind::Video) | (b"hvc1" | b"hev1", Kind::Video) => {
                if entry.len() < 78 {
                    return invalid("visual sample entry");
                }
                let (width, height) = (be16(entry, 24) as u32, be16(entry, 26) as u32);
                let ext = &entry[78..];
                return Ok(Some(if fourcc[0] == b'a' {
                    Codec::H264 { avcc: child(ext, b"avcC").ok_or(Error::Invalid("avcC"))?.to_vec(), width, height }
                } else {
                    Codec::Hevc { hvcc: child(ext, b"hvcC").ok_or(Error::Invalid("hvcC"))?.to_vec(), width, height }
                }));
            }
            (b"mp4a", Kind::Audio) => {
                if entry.len() < 28 {
                    return invalid("audio sample entry");
                }
                let version = be16(entry, 8);
                let channels = be16(entry, 16);
                let sample_rate = be32(entry, 24) >> 16;
                let ext = entry.get(28 + match version {
                    1 => 16,
                    2 => 36,
                    _ => 0,
                }..).ok_or(Error::Invalid("audio sample entry"))?;
                let esds = child(ext, b"esds").ok_or(Error::Invalid("esds"))?;
                let asc = audio_specific_config(esds.get(4..).unwrap_or(&[])).ok_or(Error::Invalid("esds descriptors"))?;
                let (sample_rate, channels) = asc_params(&asc).unwrap_or((sample_rate, channels));
                return Ok(Some(Codec::Aac { asc, sample_rate, channels }));
            }
            _ => {}
        }
    }
    Ok(None)
}

/// Reads an MPEG-4 descriptor header: (tag, payload) of the first descriptor.
fn descriptor(b: &[u8]) -> Option<(u8, &[u8])> {
    let tag = *b.first()?;
    let (mut len, mut p) = (0usize, 1);
    for _ in 0..4 {
        let c = *b.get(p)?;
        p += 1;
        len = len << 7 | (c & 0x7f) as usize;
        if c & 0x80 == 0 {
            break;
        }
    }
    Some((tag, b.get(p..p + len)?))
}

/// The DecoderSpecificInfo (AudioSpecificConfig) inside an ES_Descriptor.
fn audio_specific_config(es: &[u8]) -> Option<Vec<u8>> {
    let (tag, es) = descriptor(es)?;
    if tag != 0x03 {
        return None;
    }
    let flags = *es.get(2)?;
    let mut p = 3;
    if flags & 0x80 != 0 {
        p += 2;
    }
    if flags & 0x40 != 0 {
        p += 1 + *es.get(p)? as usize;
    }
    if flags & 0x20 != 0 {
        p += 2;
    }
    let (tag, dcd) = descriptor(es.get(p..)?)?;
    if tag != 0x04 {
        return None;
    }
    let (tag, dsi) = descriptor(dcd.get(13..)?)?;
    (tag == 0x05).then(|| dsi.to_vec())
}

pub(super) const AAC_RATES: [u32; 13] = [96000, 88200, 64000, 48000, 44100, 32000, 24000, 22050, 16000, 12000, 11025, 8000, 7350];

/// (sample rate, channels) from an AudioSpecificConfig.
pub fn asc_params(asc: &[u8]) -> Option<(u32, u16)> {
    let mut pos = 0;
    let mut bits = |n: usize| -> Option<u32> {
        let mut v = 0;
        for _ in 0..n {
            v = v << 1 | (asc.get(pos / 8)? >> (7 - pos % 8) & 1) as u32;
            pos += 1;
        }
        Some(v)
    };
    if bits(5)? == 31 {
        bits(6)?;
    }
    let rate = match bits(4)? {
        15 => bits(24)?,
        i => *AAC_RATES.get(i as usize)?,
    };
    let channels = bits(4)? as u16;
    Some((rate, if channels == 7 { 8 } else { channels }))
}

fn sample_table(stbl: &[u8]) -> Result<Vec<SampleInfo>, Error> {
    let table = |name: &[u8; 4]| child(stbl, name);
    let Some(stts) = table(b"stts") else { return Ok(Vec::new()) };
    // the sample count the size table declares caps everything else
    let (stsz, stz2) = (table(b"stsz"), table(b"stz2"));
    let max_count = match (stsz, stz2) {
        (Some(z), _) => {
            need(z, 12)?;
            // a per-sample size list must fit its box
            if be32(z, 4) == 0 && (be32(z, 8) as usize).checked_mul(4).map_or(true, |x| x + 12 > z.len()) {
                return invalid("stsz larger than its box");
            }
            be32(z, 8) as usize
        }
        (None, Some(z)) => {
            need(z, 12)?;
            // even 4-bit sizes take half a byte each
            if (be32(z, 8) as usize) / 2 + 12 > z.len() {
                return invalid("stz2 larger than its box");
            }
            be32(z, 8) as usize
        }
        _ => return invalid("stsz"),
    };
    if max_count > MAX_SAMPLES {
        return invalid("too many samples");
    }
    let n = entries(stts, 8, 8)?;
    let mut dts = Vec::new();
    let mut t = 0u64;
    'stts: for i in 0..n {
        let (count, delta) = (be32(stts, 8 + i * 8), be32(stts, 12 + i * 8));
        for _ in 0..count {
            if dts.len() >= max_count {
                break 'stts;
            }
            dts.push(t);
            t = t.wrapping_add(delta as u64);
        }
    }
    let count = dts.len();
    let mut cts = vec![0i32; count];
    if let Some(ctts) = table(b"ctts") {
        let mut s = 0usize;
        for i in 0..entries(ctts, 8, 8)? {
            if s >= count {
                break;
            }
            let (c, off) = (be32(ctts, 8 + i * 8) as usize, be32(ctts, 12 + i * 8) as i32);
            cts[s..s.saturating_add(c).min(count)].fill(off);
            s = s.saturating_add(c);
        }
    }
    let sizes: Vec<u32> = match (stsz, stz2) {
        (Some(z), _) if be32(z, 4) != 0 => vec![be32(z, 4); count],
        (Some(z), _) => {
            if count.checked_mul(4).map_or(true, |x| x + 12 > z.len()) {
                return invalid("stsz larger than its box");
            }
            (0..count).map(|i| be32(z, 12 + i * 4)).collect()
        }
        (None, Some(z)) => {
            let field = z[7] as usize;
            let bytes = match field {
                4 => count / 2 + count % 2,
                8 => count,
                16 => count * 2,
                _ => return invalid("stz2 field size"),
            };
            if bytes + 12 > z.len() {
                return invalid("stz2 larger than its box");
            }
            (0..count)
                .map(|i| match field {
                    4 => (z[12 + i / 2] >> (4 * (1 - i % 2) as u8) & 0xf) as u32,
                    8 => z[12 + i] as u32,
                    _ => be16(z, 12 + i * 2) as u32,
                })
                .collect()
        }
        _ => return invalid("stsz"),
    };
    let chunks: Vec<u64> = match (table(b"stco"), table(b"co64")) {
        (Some(stco), _) => (0..entries(stco, 8, 4)?).map(|i| be32(stco, 8 + i * 4) as u64).collect(),
        (None, Some(co64)) => (0..entries(co64, 8, 8)?).map(|i| be64(co64, 8 + i * 8)).collect(),
        _ => return invalid("stco"),
    };
    let stsc = table(b"stsc").ok_or(Error::Invalid("stsc"))?;
    let runs: Vec<(u32, u32)> = (0..entries(stsc, 8, 12)?).map(|i| (be32(stsc, 8 + i * 12), be32(stsc, 12 + i * 12))).collect();
    let mut offsets = Vec::with_capacity(count);
    let mut s = 0;
    'chunks: for (ci, &base) in chunks.iter().enumerate() {
        let chunk_no = ci as u32 + 1;
        let per = runs.iter().rev().find(|r| r.0 <= chunk_no).map_or(0, |r| r.1);
        let mut off = base;
        for _ in 0..per {
            if s >= count {
                break 'chunks;
            }
            offsets.push(off);
            off = off.wrapping_add(sizes[s] as u64);
            s += 1;
        }
    }
    if offsets.len() != count {
        return invalid("chunk table shorter than sample table");
    }
    let mut sync = vec![true; count];
    if let Some(stss) = table(b"stss") {
        sync.fill(false);
        for i in 0..entries(stss, 8, 4)? {
            if let Some(s) = (be32(stss, 8 + i * 4) as usize).checked_sub(1).and_then(|k| sync.get_mut(k)) {
                *s = true;
            }
        }
    }
    Ok((0..count).map(|i| SampleInfo { offset: offsets[i], size: sizes[i], dts: dts[i], cts: cts[i], sync: sync[i] }).collect())
}

/// NAL length-prefix size (1, 2 or 4) declared by an `avcC`/`hvcC` box.
pub fn nal_length_size(config: &[u8], hevc: bool) -> usize {
    let b = if hevc { config.get(21) } else { config.get(4) };
    (b.copied().unwrap_or(3) & 3) as usize + 1
}

/// Converts a length-prefixed (AVCC) sample to Annex B start codes.
pub fn to_annex_b(sample: &[u8], len_size: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(sample.len() + 16);
    let mut p = 0;
    while p + len_size <= sample.len() {
        let n = sample[p..p + len_size].iter().fold(0usize, |a, &b| a << 8 | b as usize);
        p += len_size;
        let Some(nal) = sample.get(p..p + n) else { break };
        out.extend_from_slice(&[0, 0, 0, 1]);
        out.extend_from_slice(nal);
        p += n;
    }
    out
}

/// The parameter sets of an `avcC` box as Annex B NAL units (SPS then PPS).
pub fn avcc_parameter_sets(avcc: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let Some(&n_sps) = avcc.get(5) else { return out };
    let mut p = 6;
    let take = |p: &mut usize, out: &mut Vec<u8>| {
        if let (Some(hi), Some(lo)) = (avcc.get(*p), avcc.get(*p + 1)) {
            let n = u16::from_be_bytes([*hi, *lo]) as usize;
            if let Some(nal) = avcc.get(*p + 2..*p + 2 + n) {
                out.extend_from_slice(&[0, 0, 0, 1]);
                out.extend_from_slice(nal);
            }
            *p += 2 + n;
        }
    };
    for _ in 0..n_sps & 0x1f {
        take(&mut p, &mut out);
    }
    let n_pps = avcc.get(p).copied().unwrap_or(0);
    p += 1;
    for _ in 0..n_pps {
        take(&mut p, &mut out);
    }
    out
}

/// The parameter sets of an `hvcC` box (VPS/SPS/PPS arrays) as Annex B.
pub fn hvcc_parameter_sets(hvcc: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let Some(&arrays) = hvcc.get(22) else { return out };
    let mut p = 23;
    for _ in 0..arrays {
        let Some(&n) = hvcc.get(p + 1) else { break };
        let count = u16::from_be_bytes([n, *hvcc.get(p + 2).unwrap_or(&0)]) as usize;
        p += 3;
        for _ in 0..count {
            let (Some(&hi), Some(&lo)) = (hvcc.get(p), hvcc.get(p + 1)) else { return out };
            let len = u16::from_be_bytes([hi, lo]) as usize;
            if let Some(nal) = hvcc.get(p + 2..p + 2 + len) {
                out.extend_from_slice(&[0, 0, 0, 1]);
                out.extend_from_slice(nal);
            }
            p += 2 + len;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::super::source::MemSource;
    use super::*;

    pub const BASELINE: &[u8] = include_bytes!("testdata/tiny_baseline.mp4");
    pub const HIGH: &[u8] = include_bytes!("testdata/tiny_high.mp4");
    pub const FRAG: &[u8] = include_bytes!("testdata/tiny_frag.mp4");

    fn open(bytes: &[u8]) -> (Demuxer, MemSource) {
        let mut src = MemSource::new(bytes.to_vec());
        let d = Demuxer::open(&mut src).unwrap();
        (d, src)
    }

    fn drain(d: &mut Demuxer, kind: Kind, src: &mut MemSource) -> Vec<Sample> {
        std::iter::from_fn(|| d.next(kind, src).unwrap()).collect()
    }

    #[test]
    fn progressive_tracks_and_codecs() {
        let (d, _) = open(BASELINE);
        let v = d.video().unwrap();
        let a = d.audio().unwrap();
        assert!(matches!(&v.codec, Codec::H264 { width: 96, height: 64, avcc } if avcc[0] == 1 && avcc[1] == 66), "{:?}", v.codec);
        assert!(matches!(&a.codec, Codec::Aac { sample_rate: 48000, channels: 2, asc } if asc.len() >= 2), "{:?}", a.codec);
        assert_eq!(v.samples.len(), 30);
        assert_eq!(a.samples.len(), 48);
        assert_eq!(v.timescale, 15360);
        assert!((d.duration_ms().unwrap() - 1000.0).abs() < 50.0, "{:?}", d.duration_ms());
        assert!(v.samples[0].sync && !v.samples[1].sync && v.samples[15].sync, "stss: keyframes every 15");
        assert!(a.samples.iter().all(|s| s.sync));
    }

    #[test]
    fn progressive_samples_are_timed_and_in_the_mdat() {
        let (mut d, mut src) = open(BASELINE);
        let video = drain(&mut d, Kind::Video, &mut src);
        let audio = drain(&mut d, Kind::Audio, &mut src);
        assert_eq!(video.len(), 30);
        assert_eq!(audio.len(), 48);
        let dts: Vec<f64> = video.iter().map(|s| s.dts_ms).collect();
        for w in dts.windows(2) {
            assert!((w[1] - w[0] - 1000.0 / 30.0).abs() < 0.01);
        }
        assert!(video.iter().all(|s| (s.duration_ms - 33.333).abs() < 0.01));
        assert!(audio.iter().all(|s| (s.duration_ms - 1024.0 / 48.0).abs() < 0.01));
        // baseline: pts == dts
        assert!(video.iter().all(|s| (s.pts_ms - s.dts_ms).abs() < 0.01));
        // every video sample is length-prefixed NAL units
        for s in &video {
            let n = u32::from_be_bytes(s.data[..4].try_into().unwrap()) as usize;
            assert!(n + 4 <= s.data.len(), "nal length {n} fits in {}", s.data.len());
        }
        let total: usize = video.iter().chain(&audio).map(|s| s.data.len()).sum();
        assert!(total > 10_000 && total < BASELINE.len());
    }

    #[test]
    fn b_frames_reorder_pts_with_ctts_and_the_edit_list() {
        let (mut d, mut src) = open(HIGH);
        let video = drain(&mut d, Kind::Video, &mut src);
        let mut pts: Vec<f64> = video.iter().map(|s| s.pts_ms).collect();
        assert!(pts.windows(2).any(|w| w[1] < w[0]), "decode order differs from display order");
        pts.sort_by(|a, b| a.partial_cmp(b).unwrap());
        assert!(pts[0].abs() < 0.01, "edit list makes the first frame start at 0, got {}", pts[0]);
        for (i, p) in pts.iter().enumerate() {
            assert!((p - i as f64 * 1000.0 / 30.0).abs() < 0.01, "frame {i} at {p}");
        }
    }

    #[test]
    fn fragmented_file_yields_samples_from_moofs() {
        let (mut d, mut src) = open(FRAG);
        assert!(d.fragmented);
        assert_eq!(d.video().unwrap().samples.len(), 0, "no samples before the first moof");
        let video = drain(&mut d, Kind::Video, &mut src);
        let audio = drain(&mut d, Kind::Audio, &mut src);
        assert_eq!(video.len(), 30);
        assert_eq!(audio.len(), 48);
        assert!(video[0].sync && !video[1].sync);
        let mut pts: Vec<f64> = video.iter().map(|s| s.pts_ms).collect();
        pts.sort_by(|a, b| a.partial_cmp(b).unwrap());
        // no edit list in a fragmented mux: presentation starts two frames late
        assert!((pts[0] - 2000.0 / 30.0).abs() < 0.01, "first frame at {}", pts[0]);
        for (i, p) in pts.iter().enumerate() {
            assert!((p - pts[0] - i as f64 * 1000.0 / 30.0).abs() < 0.01, "frame {i} at {p}");
        }
        let (mut d2, mut src2) = open(HIGH);
        let video2 = drain(&mut d2, Kind::Video, &mut src2);
        assert_eq!(video.iter().map(|s| s.data.len()).sum::<usize>(), video2.iter().map(|s| s.data.len()).sum::<usize>(), "same payload bytes as the progressive mux");
    }

    #[test]
    fn growing_file_reports_need_data_and_resumes() {
        let mut src = MemSource { data: BASELINE.to_vec(), available: 100, complete: false };
        match Demuxer::open(&mut src) {
            Err(Error::NeedData { offset, len }) => assert!(offset >= 100 && len > 0),
            Err(e) => panic!("{e:?}"),
            Ok(_) => panic!("opened without the moov"),
        }
        src.available = 4100;
        let mut d = Demuxer::open(&mut src).unwrap();
        let first = d.next(Kind::Video, &mut src).unwrap().unwrap();
        let second = d.next(Kind::Video, &mut src);
        let need = match second {
            Err(Error::NeedData { offset, len }) => (offset, len),
            other => panic!("{other:?}"),
        };
        assert!(need.0 >= 4100 && need.0 < BASELINE.len() as u64);
        src.available = need.0 + need.1;
        let second = d.next(Kind::Video, &mut src).unwrap().unwrap();
        assert!(second.dts_ms > first.dts_ms, "cursor did not skip the sample while waiting");
        src.available = BASELINE.len() as u64;
        assert_eq!(drain(&mut d, Kind::Video, &mut src).len(), 28);
    }

    #[test]
    fn growing_fragmented_file_waits_for_the_next_moof() {
        let mut src = MemSource { data: FRAG.to_vec(), available: 1300, complete: false };
        let mut d = Demuxer::open(&mut src).unwrap();
        assert!(matches!(d.next(Kind::Video, &mut src), Err(Error::NeedData { .. })));
        src.available = FRAG.len() as u64;
        assert_eq!(drain(&mut d, Kind::Video, &mut src).len(), 30);
    }

    #[test]
    fn seek_lands_on_a_keyframe_and_aligns_audio() {
        let (mut d, mut src) = open(BASELINE);
        let at = d.seek_ms(700.0, &mut src).unwrap();
        assert!((at - 500.0).abs() < 0.01, "keyframe 15 at 500 ms, got {at}");
        let v = d.next(Kind::Video, &mut src).unwrap().unwrap();
        assert!(v.sync && (v.pts_ms - 500.0).abs() < 0.01);
        let a = d.next(Kind::Audio, &mut src).unwrap().unwrap();
        assert!(a.pts_ms >= 700.0 && a.pts_ms < 700.0 + 1024.0 / 48.0, "audio starts at the target, not the keyframe: {}", a.pts_ms);
        assert_eq!(d.seek_ms(0.0, &mut src).unwrap(), 0.0);
        assert_eq!(d.seek_ms(1e9, &mut src).unwrap(), 500.0);
        let (mut f, mut fsrc) = open(FRAG);
        let at = f.seek_ms(600.0, &mut fsrc).unwrap();
        assert!((at - 500.0 - 2000.0 / 30.0).abs() < 0.01, "fragment seek parses ahead to keyframe 15, got {at}");
    }

    fn full_box(kind: &[u8; 4], body: Vec<u8>) -> Vec<u8> {
        let mut b = ((body.len() + 8) as u32).to_be_bytes().to_vec();
        b.extend_from_slice(kind);
        b.extend(body);
        b
    }

    fn be_words(v: &[u32]) -> Vec<u8> {
        v.iter().flat_map(|x| x.to_be_bytes()).collect()
    }

    #[test]
    fn crafted_huge_sample_counts_are_errors_not_allocations() {
        for count in [u32::MAX, (MAX_SAMPLES as u32) + 1] {
            // variable sizes declared but absent
            let mut stbl = full_box(b"stts", be_words(&[0, 1, count, 1]));
            stbl.extend(full_box(b"stsz", be_words(&[0, 0, count])));
            assert!(sample_table(&stbl).is_err(), "stsz count {count}");
            // constant size: only the cap stops it
            let mut stbl = full_box(b"stts", be_words(&[0, 1, count, 1]));
            stbl.extend(full_box(b"stsz", be_words(&[0, 4, count])));
            assert!(sample_table(&stbl).is_err(), "constant stsz count {count}");
            // stz2 with 4-bit sizes
            let mut stbl = full_box(b"stts", be_words(&[0, 1, count, 1]));
            stbl.extend(full_box(b"stz2", be_words(&[0, 4, count])));
            assert!(sample_table(&stbl).is_err(), "stz2 count {count}");
        }
    }

    #[test]
    fn synthetic_sample_table_expands_stsc_runs_and_constant_sizes() {
        fn full(kind: &[u8; 4], body: Vec<u8>) -> Vec<u8> {
            let mut b = ((body.len() + 8) as u32).to_be_bytes().to_vec();
            b.extend_from_slice(kind);
            b.extend(body);
            b
        }
        let be = |v: &[u32]| v.iter().flat_map(|x| x.to_be_bytes()).collect::<Vec<u8>>();
        let mut stbl = full(b"stts", be(&[0, 2, 3, 10, 2, 20]));
        stbl.extend(full(b"stsc", be(&[0, 2, 1, 2, 1, 3, 1, 1])));
        stbl.extend(full(b"stsz", be(&[0, 100, 5])));
        stbl.extend(full(b"stco", be(&[0, 3, 1000, 2000, 3000])));
        stbl.extend(full(b"stss", be(&[0, 1, 3])));
        let t = sample_table(&stbl).unwrap();
        assert_eq!(t.len(), 5);
        assert_eq!(t.iter().map(|s| s.offset).collect::<Vec<_>>(), [1000, 1100, 2000, 2100, 3000]);
        assert_eq!(t.iter().map(|s| s.dts).collect::<Vec<_>>(), [0, 10, 20, 30, 50]);
        assert_eq!(t.iter().map(|s| s.sync).collect::<Vec<_>>(), [false, false, true, false, false]);
        assert!(t.iter().all(|s| s.size == 100));
    }

    #[test]
    fn annex_b_conversion_and_parameter_sets() {
        assert_eq!(to_annex_b(&[0, 0, 0, 2, 0x65, 0xaa, 0, 0, 0, 1, 0x41], 4), [0, 0, 0, 1, 0x65, 0xaa, 0, 0, 0, 1, 0x41]);
        assert_eq!(to_annex_b(&[0, 2, 0x65, 0xaa], 2), [0, 0, 0, 1, 0x65, 0xaa]);
        let (d, _) = open(BASELINE);
        let Codec::H264 { avcc, .. } = &d.video().unwrap().codec else { panic!() };
        assert_eq!(nal_length_size(avcc, false), 4);
        let ps = avcc_parameter_sets(avcc);
        assert_eq!(&ps[..4], &[0, 0, 0, 1]);
        assert_eq!(ps[4] & 0x1f, 7, "SPS first");
        let pps = ps.windows(4).rposition(|w| w == [0, 0, 0, 1]).unwrap();
        assert_eq!(ps[pps + 4] & 0x1f, 8, "then PPS");
        // hvcC: 23-byte header, one array (type 33 SPS) with one 3-byte NAL
        let mut hvcc = vec![0u8; 22];
        hvcc.push(1);
        hvcc.extend([0x21, 0, 1, 0, 3, 0x42, 1, 2]);
        assert_eq!(hvcc_parameter_sets(&hvcc), [0, 0, 0, 1, 0x42, 1, 2]);
        assert_eq!(nal_length_size(&hvcc, true), 1);
    }

    #[test]
    fn audio_specific_config_is_decoded() {
        // AAC-LC, 48 kHz (index 3), stereo: 0001 0001 1001 0000 = 0x11 0x90
        assert_eq!(asc_params(&[0x11, 0x90]), Some((48000, 2)));
        // 44.1 kHz mono
        assert_eq!(asc_params(&[0x12, 0x08]), Some((44100, 1)));
        assert_eq!(asc_params(&[0x11]), None);
    }

    #[test]
    fn garbage_is_rejected_without_panicking() {
        let mut src = MemSource::new(vec![0u8; 64]);
        assert!(matches!(Demuxer::open(&mut src).map(|_| ()), Err(Error::Invalid(_))));
        let mut src = MemSource::new(b"\0\0\0\x08ftyp".to_vec());
        assert!(matches!(Demuxer::open(&mut src).map(|_| ()), Err(Error::Eof)));
    }
}
