//! Pure-Rust container demuxers for web video (YouTube-style MSE).
//!
//! * [`mp4`]: ISO BMFF, plain and fragmented (fMP4 / CMAF), `sidx`.
//! * [`webm`]: Matroska / WebM EBML.
//! * [`mse`]: an incremental `SourceBuffer`-like front end over both.
//! * [`bitreader`]: byte and bit readers.
//!
//! Times are exposed as microseconds (`*_us`) on [`Sample`] and as seconds
//! (`f64`) on [`TimeRange`] and the MSE API.

pub mod bitreader;
pub mod mp4;
pub mod mse;
pub mod webm;

use std::fmt;

/// Why a demux step failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DemuxError {
    /// Malformed input.
    Invalid(&'static str),
    /// Valid input this demuxer does not handle.
    Unsupported(&'static str),
    /// A media segment arrived before the init segment.
    MissingInit,
    /// An earlier append failed; the buffer must be recreated.
    Failed,
}

impl fmt::Display for DemuxError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DemuxError::Invalid(w) => write!(f, "invalid {w}"),
            DemuxError::Unsupported(w) => write!(f, "unsupported {w}"),
            DemuxError::MissingInit => write!(f, "media segment before init segment"),
            DemuxError::Failed => write!(f, "source buffer is in an error state"),
        }
    }
}

impl std::error::Error for DemuxError {}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum TrackKind {
    #[default]
    Video,
    Audio,
}

/// Codec of a demuxed track. Distinct from `media::Codec`, which only names
/// what the hardware decoder accepts.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum CodecId {
    H264,
    Hevc,
    Vp8,
    Vp9,
    Av1,
    Aac,
    Opus,
    Vorbis,
    #[default]
    Unknown,
}

/// Static description of one track.
///
/// `codec_private` per codec: `avcC`/`hvcC`/`av1C`/`vpcC` box payloads for
/// MP4 video (WebM: the raw CodecPrivate), the AudioSpecificConfig for AAC,
/// an `OpusHead` for Opus (MP4 `dOps` is converted), the Xiph-laced header
/// triple for Vorbis.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct TrackInfo {
    pub id: u32,
    pub kind: TrackKind,
    pub codec: CodecId,
    pub width: u32,
    pub height: u32,
    pub channels: u16,
    pub sample_rate: u32,
    pub codec_private: Vec<u8>,
    /// Container-declared frame duration (WebM DefaultDuration), if any.
    pub default_duration_us: Option<i64>,
}

/// One demuxed access unit.
#[derive(Clone, Debug, PartialEq)]
pub struct Sample {
    pub track: u32,
    pub pts_us: i64,
    pub dts_us: i64,
    pub duration_us: i64,
    pub keyframe: bool,
    pub data: Vec<u8>,
}

/// A buffered time span in seconds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TimeRange {
    pub start: f64,
    pub end: f64,
}

/// Convert container ticks to microseconds.
pub fn ticks_to_us(ticks: i64, timescale: u32) -> i64 {
    ((ticks as i128 * 1_000_000) / timescale.max(1) as i128) as i64
}

#[cfg(test)]
pub(crate) mod fixtures {
    //! Byte builders for tiny MP4 and WebM streams. No external files.

    // ---- ISO BMFF -------------------------------------------------------

    pub fn bx(t: &[u8; 4], p: &[u8]) -> Vec<u8> {
        let mut v = ((p.len() + 8) as u32).to_be_bytes().to_vec();
        v.extend_from_slice(t);
        v.extend_from_slice(p);
        v
    }

    pub fn full(t: &[u8; 4], ver: u8, flags: u32, p: &[u8]) -> Vec<u8> {
        let mut body = vec![ver, (flags >> 16) as u8, (flags >> 8) as u8, flags as u8];
        body.extend_from_slice(p);
        bx(t, &body)
    }

    pub fn avcc() -> Vec<u8> {
        vec![1, 0x64, 0, 0x1f, 0xff, 0xe1, 0, 4, 0x67, 1, 2, 3, 1, 0, 2, 0x68, 4]
    }

    pub fn video_entry(fourcc: &[u8; 4], cfg_fourcc: &[u8; 4], cfg: &[u8], w: u16, h: u16) -> Vec<u8> {
        let mut p = vec![0u8; 78];
        p[7] = 1;
        p[24..26].copy_from_slice(&w.to_be_bytes());
        p[26..28].copy_from_slice(&h.to_be_bytes());
        p.extend(bx(cfg_fourcc, cfg));
        bx(fourcc, &p)
    }

    pub fn audio_entry(fourcc: &[u8; 4], channels: u16, rate: u32, child: Vec<u8>) -> Vec<u8> {
        let mut p = vec![0u8; 28];
        p[7] = 1;
        p[16..18].copy_from_slice(&channels.to_be_bytes());
        p[18..20].copy_from_slice(&16u16.to_be_bytes());
        p[24..28].copy_from_slice(&(rate << 16).to_be_bytes());
        p.extend(child);
        bx(fourcc, &p)
    }

    pub fn esds(asc: &[u8]) -> Vec<u8> {
        let mut dss = vec![0x05, asc.len() as u8];
        dss.extend_from_slice(asc);
        let mut dc = vec![0x04, (13 + dss.len()) as u8, 0x40, 0x15, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        dc.extend(dss);
        let mut es = vec![0x03, 0, 0, 1, 0];
        es.extend(dc);
        es.extend([0x06, 0x01, 0x02]);
        es[1] = (es.len() - 2) as u8;
        full(b"esds", 0, 0, &es)
    }

    pub fn trak_with(id: u32, handler: &[u8; 4], timescale: u32, stbl_extra: Vec<u8>, entry: Vec<u8>) -> Vec<u8> {
        let mut tkhd = vec![0u8; 84];
        tkhd[12..16].copy_from_slice(&id.to_be_bytes());
        let mut mdhd = vec![0u8; 24];
        mdhd[12..16].copy_from_slice(&timescale.to_be_bytes());
        let mut hdlr = vec![0u8; 25];
        hdlr[8..12].copy_from_slice(handler);
        let mut stsd = vec![0, 0, 0, 0, 0, 0, 0, 1];
        stsd.extend(entry);
        let mut stbl = bx(b"stsd", &stsd);
        stbl.extend(stbl_extra);
        let minf = bx(b"minf", &bx(b"stbl", &stbl));
        let mdia = bx(b"mdia", &[bx(b"mdhd", &mdhd), bx(b"hdlr", &hdlr), minf].concat());
        bx(b"trak", &[bx(b"tkhd", &tkhd), mdia].concat())
    }

    pub fn trak(id: u32, handler: &[u8; 4], timescale: u32, entry: Vec<u8>) -> Vec<u8> {
        trak_with(id, handler, timescale, Vec::new(), entry)
    }

    pub fn moov(traks: Vec<Vec<u8>>, trex_ids: &[u32]) -> Vec<u8> {
        let mut body = traks.concat();
        if !trex_ids.is_empty() {
            let mut mvex = Vec::new();
            for id in trex_ids {
                let mut p = id.to_be_bytes().to_vec();
                p.extend([0u8; 16]);
                mvex.extend(full(b"trex", 0, 0, &p));
            }
            body.extend(bx(b"mvex", &mvex));
        }
        bx(b"moov", &body)
    }

    pub const ASC_LC_48K_STEREO: [u8; 2] = [0x11, 0x90];

    /// ftyp + moov: track 1 H.264 320x240 (timescale 1000), track 2 AAC
    /// 48 kHz stereo (timescale 48000).
    pub fn mp4_init() -> Vec<u8> {
        let ftyp = bx(b"ftyp", b"iso5\0\0\0\x01iso5cmfc");
        let v = trak(1, b"vide", 1000, video_entry(b"avc1", b"avcC", &avcc(), 320, 240));
        let a = trak(2, b"soun", 48000, audio_entry(b"mp4a", 2, 48000, esds(&ASC_LC_48K_STEREO)));
        [ftyp, moov(vec![v, a], &[1, 2])].concat()
    }

    #[derive(Clone, Copy)]
    pub struct FSample {
        pub dur: u32,
        pub size: u32,
        pub flags: u32,
        pub cto: i32,
    }

    pub const KEY: u32 = 0x0200_0000;
    pub const NON_KEY: u32 = 0x0101_0000;

    /// Byte every payload byte of sample `idx` of `track` is filled with.
    pub fn fill(track: u32, idx: usize) -> u8 {
        ((track as u8) << 4) | idx as u8
    }

    fn traf(track: u32, base: u64, samples: &[FSample], data_offset: i32) -> Vec<u8> {
        let tfhd = full(b"tfhd", 0, 0x2_0000, &track.to_be_bytes());
        let tfdt = full(b"tfdt", 1, 0, &base.to_be_bytes());
        let mut p = (samples.len() as u32).to_be_bytes().to_vec();
        p.extend(data_offset.to_be_bytes());
        for s in samples {
            p.extend(s.dur.to_be_bytes());
            p.extend(s.size.to_be_bytes());
            p.extend(s.flags.to_be_bytes());
            p.extend(s.cto.to_be_bytes());
        }
        let trun = full(b"trun", 1, 0xF01, &p);
        bx(b"traf", &[tfhd, tfdt, trun].concat())
    }

    /// moof + mdat for `(track, base_dts, samples)` runs. Sample data is
    /// `fill(track, index)` repeated `size` times.
    pub fn moof_mdat(seq: u32, trafs: &[(u32, u64, Vec<FSample>)]) -> Vec<u8> {
        let build = |offs: &[i32]| -> Vec<u8> {
            let mut body = full(b"mfhd", 0, 0, &seq.to_be_bytes());
            for (i, (t, b, s)) in trafs.iter().enumerate() {
                body.extend(traf(*t, *b, s, offs[i]));
            }
            bx(b"moof", &body)
        };
        let probe = build(&vec![0; trafs.len()]);
        let mut offs = Vec::new();
        let mut run = probe.len() as i32 + 8;
        for (_, _, s) in trafs {
            offs.push(run);
            run += s.iter().map(|x| x.size as i32).sum::<i32>();
        }
        let moof = build(&offs);
        let mut mdat = Vec::new();
        for (t, _, s) in trafs {
            for (i, x) in s.iter().enumerate() {
                mdat.extend(vec![fill(*t, i); x.size as usize]);
            }
        }
        [moof, bx(b"mdat", &mdat)].concat()
    }

    pub const V_SIZES: [u32; 3] = [5, 4, 3];
    pub const A_SIZES: [u32; 3] = [2, 3, 2];

    /// Media segment `n`: 3 video samples of 40 ms and 3 audio samples of
    /// 1920 ticks (40 ms) starting at `n * 120` ms.
    pub fn mp4_segment(n: u32) -> Vec<u8> {
        let v: Vec<FSample> = (0..3)
            .map(|i| FSample { dur: 40, size: V_SIZES[i], flags: if i == 0 { KEY } else { NON_KEY }, cto: 0 })
            .collect();
        let a: Vec<FSample> = (0..3).map(|i| FSample { dur: 1920, size: A_SIZES[i], flags: KEY, cto: 0 }).collect();
        moof_mdat(n + 1, &[(1, n as u64 * 120, v), (2, n as u64 * 5760, a)])
    }

    // ---- EBML -----------------------------------------------------------

    pub fn vint_size(n: u64) -> Vec<u8> {
        let mut l = 1usize;
        while n >= (1u64 << (7 * l)) - 1 {
            l += 1;
        }
        let v = n | (1u64 << (7 * l));
        v.to_be_bytes()[8 - l..].to_vec()
    }

    pub fn id_bytes(id: u32) -> Vec<u8> {
        let b = id.to_be_bytes();
        let skip = b.iter().take_while(|x| **x == 0).count();
        b[skip..].to_vec()
    }

    pub fn el(id: u32, p: &[u8]) -> Vec<u8> {
        let mut v = id_bytes(id);
        v.extend(vint_size(p.len() as u64));
        v.extend_from_slice(p);
        v
    }

    pub fn el_unknown(id: u32, p: &[u8]) -> Vec<u8> {
        let mut v = id_bytes(id);
        v.extend([0x01, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff]);
        v.extend_from_slice(p);
        v
    }

    pub fn uint(id: u32, v: u64) -> Vec<u8> {
        let b = v.to_be_bytes();
        let skip = b.iter().take_while(|x| **x == 0).count().min(7);
        el(id, &b[skip..])
    }

    pub fn simple_block(track: u8, tc: i16, key: bool, data: &[u8]) -> Vec<u8> {
        let mut p = vec![0x80 | track];
        p.extend(tc.to_be_bytes());
        p.push(if key { 0x80 } else { 0 });
        p.extend_from_slice(data);
        el(0xA3, &p)
    }

    pub fn opus_head() -> Vec<u8> {
        let mut h = b"OpusHead".to_vec();
        h.extend([1, 2, 0x38, 0x01, 0x80, 0xbb, 0, 0, 0, 0, 0]);
        h
    }

    /// EBML header + unknown-size Segment holding Info and Tracks:
    /// track 1 VP9 640x360 40 ms, track 2 Opus stereo 48 kHz 40 ms.
    pub fn webm_init() -> Vec<u8> {
        let ebml = el(0x1A45DFA3, &el(0x4282, b"webm"));
        let info = el(
            0x1549A966,
            &[uint(0x2AD7B1, 1_000_000), el(0x4489, &360.0f64.to_be_bytes())].concat(),
        );
        let video = el(
            0xAE,
            &[
                uint(0xD7, 1),
                uint(0x83, 1),
                el(0x86, b"V_VP9"),
                uint(0x23E383, 40_000_000),
                el(0xE0, &[uint(0xB0, 640), uint(0xBA, 360)].concat()),
            ]
            .concat(),
        );
        let audio = el(
            0xAE,
            &[
                uint(0xD7, 2),
                uint(0x83, 2),
                el(0x86, b"A_OPUS"),
                el(0x63A2, &opus_head()),
                uint(0x23E383, 40_000_000),
                el(0xE1, &[el(0xB5, &48000.0f64.to_be_bytes()), uint(0x9F, 2)].concat()),
            ]
            .concat(),
        );
        let tracks = el(0x1654AE6B, &[video, audio].concat());
        [ebml, el_unknown(0x18538067, &[info, tracks].concat())].concat()
    }

    /// Cluster `n` at `n * 120` ms: 3 video and 3 audio SimpleBlocks 40 ms
    /// apart, payloads `fill(track, i)` repeated `V_SIZES`/`A_SIZES` times.
    pub fn webm_cluster(n: u32, unknown: bool) -> Vec<u8> {
        let mut body = uint(0xE7, n as u64 * 120);
        for i in 0..3usize {
            body.extend(simple_block(1, i as i16 * 40, i == 0, &vec![fill(1, i); V_SIZES[i] as usize]));
            body.extend(simple_block(2, i as i16 * 40, true, &vec![fill(2, i); A_SIZES[i] as usize]));
        }
        if unknown {
            el_unknown(0x1F43B675, &body)
        } else {
            el(0x1F43B675, &body)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ticks_convert_to_microseconds_for_common_timescales() {
        assert_eq!(ticks_to_us(90_000, 90_000), 1_000_000);
        assert_eq!(ticks_to_us(1, 1_000), 1_000);
        assert_eq!(ticks_to_us(48_000 * 3, 48_000), 3_000_000);
        assert_eq!(ticks_to_us(-45_000, 90_000), -500_000);
    }

    #[test]
    fn ticks_do_not_overflow_for_long_streams_and_a_zero_timescale_does_not_divide_by_zero() {
        assert_eq!(ticks_to_us(i64::MAX / 2_000_000, 1), (i64::MAX / 2_000_000) * 1_000_000);
        assert_eq!(ticks_to_us(5, 0), 5_000_000);
    }

    #[test]
    fn errors_describe_what_went_wrong() {
        assert_eq!(DemuxError::Invalid("box").to_string(), "invalid box");
        assert_eq!(DemuxError::Unsupported("codec").to_string(), "unsupported codec");
        assert_eq!(DemuxError::MissingInit.to_string(), "media segment before init segment");
        assert_eq!(DemuxError::Failed.to_string(), "source buffer is in an error state");
    }

    #[test]
    fn track_defaults_are_an_unknown_video_track() {
        let t = TrackInfo::default();
        assert_eq!((t.kind, t.codec, t.default_duration_us), (TrackKind::Video, CodecId::Unknown, None));
        assert!(t.codec_private.is_empty());
    }

    #[test]
    fn a_sample_keeps_separate_presentation_and_decode_times() {
        let s = Sample { track: 1, pts_us: 66_000, dts_us: 33_000, duration_us: 33_000, keyframe: false, data: vec![1] };
        assert!(s.pts_us > s.dts_us);
        assert_eq!(s.clone(), s);
    }
}
