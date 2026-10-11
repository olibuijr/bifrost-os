//! Incremental `SourceBuffer`-like front end over the MP4 and WebM demuxers.
//!
//! Bytes arrive through [`SourceBuffer::append`] split at arbitrary
//! boundaries. The first bytes select the container (EBML magic means WebM,
//! anything else MP4). Init segments define the track list; media segments
//! (fMP4 `moof`+`mdat`, WebM Clusters) are turned into [`Sample`]s queued per
//! track, from which [`SourceBuffer::next_sample`] reads in decode order.
//!
//! Limits: MP4 input must be fragmented (progressive files have no media
//! segments); edit lists and timestamp offsets are not applied.

use super::mp4::{self, Moov};
use super::webm;
use super::{ticks_to_us, DemuxError, Sample, TimeRange, TrackInfo};
use std::collections::{BTreeMap, HashMap};

type R<T> = Result<T, DemuxError>;

/// Gaps up to this many microseconds do not split a buffered range.
const GAP_US: i64 = 1_000;

/// What one `append` call produced.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AppendResult {
    /// An init segment (`moov` / `Tracks`) completed during this call.
    pub init_segment: bool,
    /// Samples added to the track queues during this call.
    pub samples_added: usize,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Container {
    Mp4,
    Webm,
}

#[derive(Default)]
struct Mp4State {
    moov: Option<Moov>,
    next_dts: HashMap<u32, u64>,
    /// Samples of a `moof` whose `mdat` has not fully arrived.
    pending: Option<Vec<mp4::Mp4Sample>>,
}

struct WebmState {
    timecode_scale: u64,
    have_tracks: bool,
    cluster_tc: Option<i64>,
    cues: Vec<webm::CuePoint>,
}

impl Default for WebmState {
    fn default() -> Self {
        WebmState { timecode_scale: webm::DEFAULT_TIMECODE_SCALE, have_tracks: false, cluster_tc: None, cues: Vec::new() }
    }
}

#[derive(Default)]
struct Queue {
    /// Sorted by `dts_us`.
    samples: Vec<Sample>,
    /// Next sample to read has `dts_us >= cursor`.
    cursor: i64,
    default_duration_us: Option<i64>,
}

pub struct SourceBuffer {
    buf: Vec<u8>,
    /// Absolute stream offset of `buf[0]`.
    base: u64,
    /// Bytes of `buf` already consumed but kept (pending `moof`).
    cursor: usize,
    container: Option<Container>,
    mp4: Mp4State,
    webm: WebmState,
    tracks: Vec<TrackInfo>,
    queues: BTreeMap<u32, Queue>,
    failed: bool,
}

impl Default for SourceBuffer {
    fn default() -> Self {
        Self::new()
    }
}

impl SourceBuffer {
    pub fn new() -> Self {
        SourceBuffer {
            buf: Vec::new(),
            base: 0,
            cursor: 0,
            container: None,
            mp4: Mp4State::default(),
            webm: WebmState::default(),
            tracks: Vec::new(),
            queues: BTreeMap::new(),
            failed: false,
        }
    }

    /// Tracks announced by the last init segment.
    pub fn tracks(&self) -> &[TrackInfo] {
        &self.tracks
    }

    /// WebM cue points seen so far (empty for MP4).
    pub fn cues(&self) -> &[webm::CuePoint] {
        &self.webm.cues
    }

    /// Feed bytes. Incomplete boxes/elements stay buffered until more data
    /// arrives. After an error every further call returns `Failed`.
    pub fn append(&mut self, data: &[u8]) -> R<AppendResult> {
        if self.failed {
            return Err(DemuxError::Failed);
        }
        self.buf.extend_from_slice(data);
        let r = self.pump();
        if r.is_err() {
            self.failed = true;
        }
        r
    }

    /// Drop unparsed bytes and parser position (MSE `abort()`); buffered
    /// samples and tracks stay.
    pub fn reset_parser(&mut self) {
        self.buf.clear();
        self.base = 0;
        self.cursor = 0;
        self.mp4.pending = None;
        self.webm.cluster_tc = None;
        self.failed = false;
    }

    pub fn sample_count(&self, track: u32) -> usize {
        self.queues.get(&track).map_or(0, |q| q.samples.len())
    }

    /// Next sample of `track` in decode order, advancing the read position.
    pub fn next_sample(&mut self, track: u32) -> Option<Sample> {
        let q = self.queues.get_mut(&track)?;
        let i = q.samples.partition_point(|s| s.dts_us < q.cursor);
        let s = q.samples.get(i)?.clone();
        q.cursor = s.dts_us + 1;
        Some(s)
    }

    /// Buffered time ranges: per track, the union of sample spans, then the
    /// intersection over all tracks that hold data.
    pub fn buffered_ranges(&self) -> Vec<TimeRange> {
        let mut acc: Option<Vec<(i64, i64)>> = None;
        for q in self.queues.values().filter(|q| !q.samples.is_empty()) {
            let r = ranges_of(q);
            acc = Some(match acc {
                None => r,
                Some(a) => intersect(&a, &r),
            });
        }
        acc.unwrap_or_default().into_iter().map(to_range).collect()
    }

    /// Buffered ranges of a single track.
    pub fn buffered_ranges_for(&self, track: u32) -> Vec<TimeRange> {
        self.queues.get(&track).map(|q| ranges_of(q).into_iter().map(to_range).collect()).unwrap_or_default()
    }

    /// Remove samples whose presentation time lies in `[start, end)`.
    pub fn remove(&mut self, range: TimeRange) {
        let (a, b) = ((range.start * 1e6).round() as i64, (range.end * 1e6).round() as i64);
        for q in self.queues.values_mut() {
            q.samples.retain(|s| !(s.pts_us >= a && s.pts_us < b));
        }
    }

    /// Move every track's read position to the last keyframe at or before
    /// `time` (the first keyframe when none precedes it). Returns the
    /// presentation time reading will restart from (the earliest over all
    /// tracks), or `time` when nothing is buffered.
    pub fn seek(&mut self, time: f64) -> f64 {
        let target = (time * 1e6).round() as i64;
        let mut landed: Option<i64> = None;
        for q in self.queues.values_mut() {
            if q.samples.is_empty() {
                continue;
            }
            let mut best = None;
            for (i, s) in q.samples.iter().enumerate() {
                if s.keyframe && s.pts_us <= target {
                    best = Some(i);
                }
            }
            let i = best.unwrap_or_else(|| q.samples.iter().position(|s| s.keyframe).unwrap_or(0));
            q.cursor = q.samples[i].dts_us;
            let p = q.samples[i].pts_us;
            landed = Some(landed.map_or(p, |l| l.min(p)));
        }
        landed.map_or(time, |u| u as f64 / 1e6)
    }

    // ------------------------------------------------------------ internals

    fn set_tracks(&mut self, tracks: Vec<TrackInfo>) {
        for t in &tracks {
            let q = self.queues.entry(t.id).or_default();
            q.default_duration_us = t.default_duration_us;
            if q.samples.is_empty() {
                q.cursor = i64::MIN;
            }
        }
        self.tracks = tracks;
    }

    fn push(&mut self, mut s: Sample) {
        let Some(q) = self.queues.get_mut(&s.track) else {
            return;
        };
        if s.duration_us == 0 {
            s.duration_us = q.default_duration_us.unwrap_or(0);
        }
        match q.samples.binary_search_by_key(&s.dts_us, |x| x.dts_us) {
            Ok(i) => q.samples[i] = s,
            Err(i) => {
                if i > 0 && q.samples[i - 1].duration_us == 0 {
                    let d = s.dts_us - q.samples[i - 1].dts_us;
                    q.samples[i - 1].duration_us = d;
                }
                q.samples.insert(i, s);
            }
        }
    }

    fn pump(&mut self) -> R<AppendResult> {
        let mut res = AppendResult::default();
        if self.container.is_none() {
            let webm_magic = [0x1Au8, 0x45, 0xDF, 0xA3];
            if self.buf.len() >= 4 && self.buf[..4] == webm_magic {
                self.container = Some(Container::Webm);
            } else if self.buf.len() >= 8 {
                self.container = Some(Container::Mp4);
            } else {
                return Ok(res);
            }
        }
        match self.container {
            Some(Container::Mp4) => self.pump_mp4(&mut res)?,
            Some(Container::Webm) => self.pump_webm(&mut res)?,
            None => {}
        }
        if self.mp4.pending.is_none() {
            self.buf.drain(..self.cursor);
            self.base += self.cursor as u64;
            self.cursor = 0;
        }
        Ok(res)
    }

    fn pump_mp4(&mut self, res: &mut AppendResult) -> R<()> {
        loop {
            let (typ, hl, total) = {
                let d = &self.buf[self.cursor..];
                let Some(h) = mp4::read_box_header(d)? else {
                    break;
                };
                if h.size == 0 || (d.len() as u64) < h.size {
                    break;
                }
                (h.typ, h.header_len, h.size as usize)
            };
            let start = self.cursor;
            let end = start + total;
            match &typ {
                b"moov" => {
                    let moov = mp4::parse_moov(&self.buf[start + hl..end])?;
                    let tracks = moov.tracks.iter().map(|t| t.info.clone()).collect();
                    self.mp4.moov = Some(moov);
                    self.set_tracks(tracks);
                    res.init_segment = true;
                }
                b"moof" => {
                    let moov = self.mp4.moov.as_ref().ok_or(DemuxError::MissingInit)?;
                    let samples = mp4::parse_moof(
                        &self.buf[start + hl..end],
                        self.base + start as u64,
                        moov,
                        &mut self.mp4.next_dts,
                    )?;
                    self.mp4.pending = Some(samples);
                }
                b"mdat" => {
                    if let Some(samples) = self.mp4.pending.take() {
                        let out = self.extract_mp4(&samples, end)?;
                        res.samples_added += out.len();
                        for s in out {
                            self.push(s);
                        }
                    }
                }
                _ => {}
            }
            self.cursor = end;
        }
        Ok(())
    }

    fn extract_mp4(&self, samples: &[mp4::Mp4Sample], mdat_end: usize) -> R<Vec<Sample>> {
        let moov = self.mp4.moov.as_ref().ok_or(DemuxError::MissingInit)?;
        let mut out = Vec::with_capacity(samples.len());
        for s in samples {
            let Some(track) = moov.track(s.track_id) else {
                continue;
            };
            if s.offset < self.base {
                return Err(DemuxError::Invalid("sample precedes buffered data"));
            }
            let rel = (s.offset - self.base) as usize;
            let e = rel.checked_add(s.size as usize).ok_or(DemuxError::Invalid("sample size"))?;
            if e > mdat_end {
                return Err(DemuxError::Invalid("sample outside mdat"));
            }
            let ts = track.timescale;
            out.push(Sample {
                track: s.track_id,
                pts_us: ticks_to_us(s.pts, ts),
                dts_us: ticks_to_us(s.dts, ts),
                duration_us: ticks_to_us(s.duration as i64, ts),
                keyframe: s.keyframe,
                data: self.buf[rel..e].to_vec(),
            });
        }
        Ok(out)
    }

    fn pump_webm(&mut self, res: &mut AppendResult) -> R<()> {
        loop {
            let (id, hl, size) = {
                let d = &self.buf[self.cursor..];
                let Some(h) = webm::read_elem_header(d)? else {
                    break;
                };
                match h.id {
                    webm::SEGMENT | webm::CLUSTER => (h.id, h.header_len, None),
                    _ => {
                        let size = h.size.ok_or(DemuxError::Invalid("unknown size element"))?;
                        if (d.len() as u64) < h.header_len as u64 + size {
                            break;
                        }
                        (h.id, h.header_len, Some(size as usize))
                    }
                }
            };
            let start = self.cursor + hl;
            let end = start + size.unwrap_or(0);
            match id {
                webm::SEGMENT => {}
                webm::CLUSTER => self.webm.cluster_tc = None,
                webm::EBML => {
                    self.webm.have_tracks = false;
                    self.webm.cluster_tc = None;
                    self.webm.timecode_scale = webm::DEFAULT_TIMECODE_SCALE;
                }
                webm::INFO => {
                    self.webm.timecode_scale = webm::parse_info(&self.buf[start..end])?.timecode_scale;
                }
                webm::TRACKS => {
                    let tracks = webm::parse_tracks(&self.buf[start..end])?;
                    self.set_tracks(tracks);
                    self.webm.have_tracks = true;
                    res.init_segment = true;
                }
                webm::CUES => self.webm.cues = webm::parse_cues(&self.buf[start..end])?,
                webm::TIMECODE => {
                    self.webm.cluster_tc = Some(webm::read_uint(&self.buf[start..end])? as i64);
                }
                webm::SIMPLE_BLOCK | webm::BLOCK_GROUP => {
                    let out = self.webm_samples(id == webm::SIMPLE_BLOCK, start, end)?;
                    res.samples_added += out.len();
                    for s in out {
                        self.push(s);
                    }
                }
                _ => {}
            }
            self.cursor = end;
        }
        Ok(())
    }

    fn webm_samples(&self, simple: bool, start: usize, end: usize) -> R<Vec<Sample>> {
        if !self.webm.have_tracks {
            return Err(DemuxError::MissingInit);
        }
        let tc = self.webm.cluster_tc.ok_or(DemuxError::Invalid("block outside cluster"))?;
        let payload = &self.buf[start..end];
        let (block, off, dur_ticks, keyframe) = if simple {
            let b = webm::parse_block(payload, true)?;
            let k = b.keyframe;
            (b, 0usize, None, k)
        } else {
            let g = webm::parse_block_group(payload)?;
            (g.block, g.block_offset, g.duration, !g.has_reference)
        };
        let track = block.track as u32;
        let Some(q) = self.queues.get(&track) else {
            return Ok(Vec::new());
        };
        let scale = self.webm.timecode_scale as i128;
        let to_us = |ticks: i64| ((ticks as i128 * scale) / 1000) as i64;
        let base_pts = to_us(tc + block.timecode as i64);
        let n = block.frames.len() as i64;
        let total_us = dur_ticks.map(|d| to_us(d as i64)).or(q.default_duration_us.map(|d| d * n)).unwrap_or(0);
        let per = total_us / n.max(1);
        let mut out = Vec::with_capacity(block.frames.len());
        for (i, (o, l)) in block.frames.iter().enumerate() {
            let pts = base_pts + i as i64 * per;
            out.push(Sample {
                track,
                pts_us: pts,
                dts_us: pts,
                duration_us: per,
                keyframe,
                data: payload[off + o..off + o + l].to_vec(),
            });
        }
        Ok(out)
    }
}

fn ranges_of(q: &Queue) -> Vec<(i64, i64)> {
    let mut v: Vec<(i64, i64)> = q.samples.iter().map(|s| (s.pts_us, s.pts_us + s.duration_us)).collect();
    v.sort();
    let mut out: Vec<(i64, i64)> = Vec::new();
    for (a, b) in v {
        match out.last_mut() {
            Some(l) if a <= l.1 + GAP_US => {
                if b > l.1 {
                    l.1 = b;
                }
            }
            _ => out.push((a, b)),
        }
    }
    out
}

fn intersect(a: &[(i64, i64)], b: &[(i64, i64)]) -> Vec<(i64, i64)> {
    let (mut i, mut j) = (0, 0);
    let mut out = Vec::new();
    while i < a.len() && j < b.len() {
        let s = a[i].0.max(b[j].0);
        let e = a[i].1.min(b[j].1);
        if s < e {
            out.push((s, e));
        }
        if a[i].1 < b[j].1 {
            i += 1;
        } else {
            j += 1;
        }
    }
    out
}

fn to_range((a, b): (i64, i64)) -> TimeRange {
    TimeRange { start: a as f64 / 1e6, end: b as f64 / 1e6 }
}

#[cfg(test)]
mod tests {
    use super::super::fixtures::*;
    use super::super::{CodecId, TrackKind};
    use super::*;

    fn drain(sb: &mut SourceBuffer) -> Vec<Sample> {
        let ids: Vec<u32> = sb.tracks().iter().map(|t| t.id).collect();
        let mut out = Vec::new();
        for id in ids {
            while let Some(s) = sb.next_sample(id) {
                out.push(s);
            }
        }
        out
    }

    fn mp4_stream(segments: u32) -> Vec<u8> {
        let mut v = mp4_init();
        for n in 0..segments {
            v.extend(mp4_segment(n));
        }
        v
    }

    fn webm_stream(segments: u32, unknown: bool) -> Vec<u8> {
        let mut v = webm_init();
        for n in 0..segments {
            v.extend(webm_cluster(n, unknown));
        }
        v
    }

    fn feed_whole(stream: &[u8]) -> (SourceBuffer, AppendResult) {
        let mut sb = SourceBuffer::new();
        let r = sb.append(stream).unwrap();
        (sb, r)
    }

    // ---- MP4 ----

    #[test]
    fn mp4_init_segment_announces_tracks() {
        let mut sb = SourceBuffer::new();
        let r = sb.append(&mp4_init()).unwrap();
        assert!(r.init_segment);
        assert_eq!(r.samples_added, 0);
        let t = sb.tracks();
        assert_eq!(t.len(), 2);
        assert_eq!((t[0].kind, t[0].codec, t[0].width, t[0].height), (TrackKind::Video, CodecId::H264, 320, 240));
        assert_eq!((t[1].kind, t[1].codec, t[1].channels, t[1].sample_rate), (TrackKind::Audio, CodecId::Aac, 2, 48000));
        assert_eq!(t[1].codec_private, ASC_LC_48K_STEREO.to_vec());
        assert!(sb.buffered_ranges().is_empty());
    }

    #[test]
    fn mp4_media_segment_samples() {
        let (mut sb, r) = feed_whole(&mp4_stream(2));
        assert_eq!(r.samples_added, 12);
        assert_eq!(sb.sample_count(1), 6);
        let v: Vec<Sample> = std::iter::from_fn(|| sb.next_sample(1)).collect();
        assert_eq!(v.len(), 6);
        assert_eq!(v[0].dts_us, 0);
        assert_eq!(v[1].dts_us, 40_000);
        assert_eq!(v[3].dts_us, 120_000);
        assert_eq!(v[0].duration_us, 40_000);
        assert!(v[0].keyframe && !v[1].keyframe && v[3].keyframe);
        assert_eq!(v[0].data, vec![fill(1, 0); 5]);
        assert_eq!(v[5].data, vec![fill(1, 2); 3]);
        let a: Vec<Sample> = std::iter::from_fn(|| sb.next_sample(2)).collect();
        assert_eq!(a[1].dts_us, 40_000);
        assert_eq!(a[3].dts_us, 120_000);
        assert_eq!(a[1].data, vec![fill(2, 1); 3]);
        assert!(a.iter().all(|s| s.keyframe));
        assert!(sb.next_sample(1).is_none());
    }

    #[test]
    fn mp4_split_at_every_byte_boundary_matches_whole() {
        let stream = mp4_stream(2);
        let (mut whole, _) = feed_whole(&stream);
        let expect = drain(&mut whole);
        assert_eq!(expect.len(), 12);
        for split in 0..=stream.len() {
            let mut sb = SourceBuffer::new();
            let mut added = 0;
            let mut inits = 0;
            for chunk in [&stream[..split], &stream[split..]] {
                let r = sb.append(chunk).unwrap();
                added += r.samples_added;
                inits += r.init_segment as usize;
            }
            assert_eq!((added, inits), (12, 1), "split {split}");
            assert_eq!(drain(&mut sb), expect, "split {split}");
        }
    }

    #[test]
    fn mp4_fed_one_byte_at_a_time() {
        let stream = mp4_stream(3);
        let (mut whole, _) = feed_whole(&stream);
        let expect = drain(&mut whole);
        let mut sb = SourceBuffer::new();
        for b in &stream {
            sb.append(std::slice::from_ref(b)).unwrap();
        }
        assert_eq!(drain(&mut sb), expect);
        assert_eq!(expect.len(), 18);
    }

    #[test]
    fn mp4_segments_may_arrive_later_after_idle() {
        let mut sb = SourceBuffer::new();
        sb.append(&mp4_init()).unwrap();
        assert_eq!(sb.append(&mp4_segment(0)).unwrap().samples_added, 6);
        let seg = mp4_segment(1);
        let (a, b) = seg.split_at(seg.len() - 1);
        assert_eq!(sb.append(a).unwrap().samples_added, 0, "mdat incomplete");
        assert_eq!(sb.append(b).unwrap().samples_added, 6);
    }

    #[test]
    fn mp4_segment_before_init_fails() {
        let mut sb = SourceBuffer::new();
        assert_eq!(sb.append(&mp4_segment(0)).unwrap_err(), DemuxError::MissingInit);
        assert_eq!(sb.append(&mp4_init()).unwrap_err(), DemuxError::Failed);
    }

    #[test]
    fn mp4_skips_unknown_top_level_boxes() {
        let mut stream = mp4_init();
        stream.extend(bx(b"free", &[0; 10]));
        stream.extend(bx(b"styp", b"msdh\0\0\0\0"));
        let sidx = full(b"sidx", 0, 0, &[0, 0, 0, 1, 0, 0, 3, 232, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        stream.extend(sidx);
        stream.extend(mp4_segment(0));
        let (sb, r) = feed_whole(&stream);
        assert_eq!(r.samples_added, 6);
        assert_eq!(sb.sample_count(2), 3);
    }

    #[test]
    fn mp4_buffered_ranges_merge_contiguous_segments() {
        let (sb, _) = feed_whole(&mp4_stream(2));
        assert_eq!(sb.buffered_ranges(), vec![TimeRange { start: 0.0, end: 0.24 }]);
        assert_eq!(sb.buffered_ranges_for(1), vec![TimeRange { start: 0.0, end: 0.24 }]);
    }

    #[test]
    fn buffered_ranges_show_gaps() {
        let mut sb = SourceBuffer::new();
        sb.append(&mp4_init()).unwrap();
        sb.append(&mp4_segment(0)).unwrap();
        sb.append(&mp4_segment(2)).unwrap();
        assert_eq!(
            sb.buffered_ranges(),
            vec![TimeRange { start: 0.0, end: 0.12 }, TimeRange { start: 0.24, end: 0.36 }]
        );
    }

    #[test]
    fn buffered_ranges_intersect_tracks() {
        let mut sb = SourceBuffer::new();
        sb.append(&mp4_init()).unwrap();
        // video 0..120 ms, audio only 0..80 ms
        let v: Vec<FSample> = (0..3)
            .map(|i| FSample { dur: 40, size: 1, flags: if i == 0 { KEY } else { NON_KEY }, cto: 0 })
            .collect();
        let a: Vec<FSample> = (0..2).map(|_| FSample { dur: 1920, size: 1, flags: KEY, cto: 0 }).collect();
        sb.append(&moof_mdat(1, &[(1, 0, v), (2, 0, a)])).unwrap();
        assert_eq!(sb.buffered_ranges(), vec![TimeRange { start: 0.0, end: 0.08 }]);
        assert_eq!(sb.buffered_ranges_for(1), vec![TimeRange { start: 0.0, end: 0.12 }]);
    }

    #[test]
    fn mp4_b_frame_pts_reordering() {
        let mut sb = SourceBuffer::new();
        sb.append(&mp4_init()).unwrap();
        let v = vec![
            FSample { dur: 40, size: 1, flags: KEY, cto: 80 },
            FSample { dur: 40, size: 1, flags: NON_KEY, cto: 80 },
            FSample { dur: 40, size: 1, flags: NON_KEY, cto: 0 },
        ];
        sb.append(&moof_mdat(1, &[(1, 0, v)])).unwrap();
        let s: Vec<Sample> = std::iter::from_fn(|| sb.next_sample(1)).collect();
        assert_eq!(s.iter().map(|x| x.dts_us).collect::<Vec<_>>(), vec![0, 40_000, 80_000]);
        assert_eq!(s.iter().map(|x| x.pts_us).collect::<Vec<_>>(), vec![80_000, 120_000, 80_000]);
        assert_eq!(sb.buffered_ranges_for(1), vec![TimeRange { start: 0.08, end: 0.16 }]);
    }

    #[test]
    fn seek_goes_to_previous_keyframe() {
        let (mut sb, _) = feed_whole(&mp4_stream(3));
        // consume everything, then seek into the middle of segment 1 (170 ms)
        drain(&mut sb);
        let t = sb.seek(0.170);
        assert!((t - 0.120).abs() < 1e-9, "landed at {t}");
        let v = sb.next_sample(1).unwrap();
        assert_eq!(v.dts_us, 120_000);
        assert!(v.keyframe);
        assert_eq!(sb.next_sample(1).unwrap().dts_us, 160_000);
        let a = sb.next_sample(2).unwrap();
        assert_eq!(a.dts_us, 160_000, "audio keyframes are every sample");
    }

    #[test]
    fn seek_before_start_and_past_end() {
        let (mut sb, _) = feed_whole(&mp4_stream(2));
        drain(&mut sb);
        assert_eq!(sb.seek(-1.0), 0.0);
        assert_eq!(sb.next_sample(1).unwrap().dts_us, 0);
        let t = sb.seek(99.0);
        assert!((t - 0.120).abs() < 1e-9);
        assert_eq!(sb.next_sample(1).unwrap().dts_us, 120_000);
    }

    #[test]
    fn seek_with_nothing_buffered_returns_input() {
        let mut sb = SourceBuffer::new();
        assert_eq!(sb.seek(3.5), 3.5);
    }

    #[test]
    fn remove_drops_samples_and_splits_ranges() {
        let (mut sb, _) = feed_whole(&mp4_stream(3));
        sb.remove(TimeRange { start: 0.12, end: 0.24 });
        assert_eq!(sb.sample_count(1), 6);
        assert_eq!(
            sb.buffered_ranges(),
            vec![TimeRange { start: 0.0, end: 0.12 }, TimeRange { start: 0.24, end: 0.36 }]
        );
        let first: Vec<i64> = std::iter::from_fn(|| sb.next_sample(1)).map(|s| s.dts_us).collect();
        assert_eq!(first, vec![0, 40_000, 80_000, 240_000, 280_000, 320_000]);
    }

    #[test]
    fn reappending_a_segment_replaces_samples() {
        let mut sb = SourceBuffer::new();
        sb.append(&mp4_init()).unwrap();
        sb.append(&mp4_segment(0)).unwrap();
        sb.append(&mp4_segment(0)).unwrap();
        assert_eq!(sb.sample_count(1), 3);
        assert_eq!(sb.sample_count(2), 3);
    }

    #[test]
    fn out_of_order_segments_are_sorted() {
        let mut sb = SourceBuffer::new();
        sb.append(&mp4_init()).unwrap();
        sb.append(&mp4_segment(1)).unwrap();
        sb.append(&mp4_segment(0)).unwrap();
        let d: Vec<i64> = std::iter::from_fn(|| sb.next_sample(1)).map(|s| s.dts_us).collect();
        assert_eq!(d, vec![0, 40_000, 80_000, 120_000, 160_000, 200_000]);
    }

    #[test]
    fn mp4_segment_with_sample_outside_mdat_fails() {
        let mut seg = mp4_segment(0);
        let moof_len = u32::from_be_bytes([seg[0], seg[1], seg[2], seg[3]]) as usize;
        seg.truncate(seg.len() - 4);
        let new_size = (seg.len() - moof_len) as u32;
        seg[moof_len..moof_len + 4].copy_from_slice(&new_size.to_be_bytes());
        let mut sb = SourceBuffer::new();
        sb.append(&mp4_init()).unwrap();
        assert!(sb.append(&seg).is_err());
    }

    #[test]
    fn reset_parser_discards_partial_data() {
        let mut sb = SourceBuffer::new();
        sb.append(&mp4_init()).unwrap();
        let seg = mp4_segment(0);
        sb.append(&seg[..seg.len() / 2]).unwrap();
        sb.reset_parser();
        assert_eq!(sb.append(&mp4_segment(1)).unwrap().samples_added, 6);
        assert_eq!(sb.sample_count(1), 3);
    }

    #[test]
    fn short_garbage_waits_then_errors_cleanly() {
        let mut sb = SourceBuffer::new();
        assert_eq!(sb.append(&[0, 0, 0]).unwrap(), AppendResult::default());
        assert_eq!(sb.append(&[16]).unwrap(), AppendResult::default());
        assert!(sb.append(b"free\0\0\0\0\0\0\0\0").is_ok());
        let mut bad = SourceBuffer::new();
        let mut junk = 4u32.to_be_bytes().to_vec();
        junk.extend(b"moovxxxx");
        assert!(bad.append(&junk).is_err());
    }

    // ---- WebM ----

    #[test]
    fn webm_init_segment_announces_tracks() {
        let mut sb = SourceBuffer::new();
        let r = sb.append(&webm_init()).unwrap();
        assert!(r.init_segment);
        let t = sb.tracks();
        assert_eq!(t.len(), 2);
        assert_eq!((t[0].codec, t[0].width, t[0].height), (CodecId::Vp9, 640, 360));
        assert_eq!((t[1].codec, t[1].channels, t[1].sample_rate), (CodecId::Opus, 2, 48000));
        assert_eq!(t[1].codec_private, opus_head());
    }

    #[test]
    fn webm_clusters_known_and_unknown_size() {
        for unknown in [false, true] {
            let (mut sb, r) = feed_whole(&webm_stream(2, unknown));
            assert_eq!(r.samples_added, 12, "unknown={unknown}");
            let v: Vec<Sample> = std::iter::from_fn(|| sb.next_sample(1)).collect();
            assert_eq!(v.iter().map(|s| s.pts_us).collect::<Vec<_>>(), vec![0, 40_000, 80_000, 120_000, 160_000, 200_000]);
            assert!(v[0].keyframe && !v[1].keyframe && v[3].keyframe);
            assert_eq!(v[0].data, vec![fill(1, 0); 5]);
            assert_eq!(v[0].duration_us, 40_000);
            let a: Vec<Sample> = std::iter::from_fn(|| sb.next_sample(2)).collect();
            assert_eq!(a[4].data, vec![fill(2, 1); 3]);
            assert!(a.iter().all(|s| s.keyframe));
        }
    }

    #[test]
    fn webm_split_at_every_byte_boundary_matches_whole() {
        for unknown in [false, true] {
            let stream = webm_stream(2, unknown);
            let (mut whole, _) = feed_whole(&stream);
            let expect = drain(&mut whole);
            assert_eq!(expect.len(), 12);
            for split in 0..=stream.len() {
                let mut sb = SourceBuffer::new();
                let mut added = 0;
                for chunk in [&stream[..split], &stream[split..]] {
                    added += sb.append(chunk).unwrap().samples_added;
                }
                assert_eq!(added, 12, "unknown={unknown} split {split}");
                assert_eq!(drain(&mut sb), expect, "unknown={unknown} split {split}");
            }
        }
    }

    #[test]
    fn webm_fed_one_byte_at_a_time() {
        let stream = webm_stream(2, true);
        let (mut whole, _) = feed_whole(&stream);
        let expect = drain(&mut whole);
        let mut sb = SourceBuffer::new();
        for b in &stream {
            sb.append(std::slice::from_ref(b)).unwrap();
        }
        assert_eq!(drain(&mut sb), expect);
    }

    #[test]
    fn webm_buffered_seek_and_remove() {
        let (mut sb, _) = feed_whole(&webm_stream(3, true));
        assert_eq!(sb.buffered_ranges(), vec![TimeRange { start: 0.0, end: 0.36 }]);
        drain(&mut sb);
        let t = sb.seek(0.25);
        assert!((t - 0.24).abs() < 1e-9, "landed at {t}");
        assert_eq!(sb.next_sample(1).unwrap().pts_us, 240_000);
        sb.remove(TimeRange { start: 0.0, end: 0.12 });
        assert_eq!(sb.buffered_ranges(), vec![TimeRange { start: 0.12, end: 0.36 }]);
    }

    #[test]
    fn webm_cluster_before_tracks_fails() {
        let mut sb = SourceBuffer::new();
        let mut stream = el(0x1A45DFA3, &el(0x4282, b"webm"));
        stream.extend(webm_cluster(0, false));
        assert_eq!(sb.append(&stream).unwrap_err(), DemuxError::MissingInit);
    }

    #[test]
    fn webm_block_group_with_duration() {
        let mut sb = SourceBuffer::new();
        sb.append(&webm_init()).unwrap();
        let mut blk = vec![0x81, 0, 10, 0];
        blk.extend([7, 7, 7]);
        let bg = el(0xA0, &[el(0xA1, &blk), uint(0x9B, 33)].concat());
        let cluster = el_unknown(0x1F43B675, &[uint(0xE7, 1000), bg].concat());
        let r = sb.append(&cluster).unwrap();
        assert_eq!(r.samples_added, 1);
        let s = sb.next_sample(1).unwrap();
        assert_eq!(s.pts_us, 1_010_000);
        assert_eq!(s.duration_us, 33_000);
        assert!(s.keyframe, "no ReferenceBlock means keyframe");
        assert_eq!(s.data, vec![7, 7, 7]);
    }

    #[test]
    fn webm_laced_block_yields_one_sample_per_frame() {
        let mut sb = SourceBuffer::new();
        sb.append(&webm_init()).unwrap();
        // two Opus frames of 2 and 2 bytes, fixed lacing
        let p = [0x82u8, 0, 0, 0x84, 1, 0xa, 0xa, 0xb, 0xb];
        let cluster = el(0x1F43B675, &[uint(0xE7, 0), el(0xA3, &p)].concat());
        sb.append(&cluster).unwrap();
        let a: Vec<Sample> = std::iter::from_fn(|| sb.next_sample(2)).collect();
        assert_eq!(a.len(), 2);
        assert_eq!((a[0].pts_us, a[1].pts_us), (0, 40_000));
        assert_eq!(a[1].data, vec![0xb, 0xb]);
    }

    #[test]
    fn webm_cues_are_collected() {
        let mut sb = SourceBuffer::new();
        let cue = el(
            0xBB,
            &[uint(0xB3, 40), el(0xB7, &[uint(0xF7, 1), uint(0xF1, 77)].concat())].concat(),
        );
        let mut stream = webm_init();
        stream.extend(el(0x1C53BB6B, &cue));
        sb.append(&stream).unwrap();
        assert_eq!(sb.cues().len(), 1);
        assert_eq!(sb.cues()[0].cluster_position, 77);
    }

    #[test]
    fn webm_timecode_scale_is_applied() {
        let ebml = el(0x1A45DFA3, &el(0x4282, b"webm"));
        let info = el(0x1549A966, &uint(0x2AD7B1, 2_000_000));
        let tracks = el(
            0x1654AE6B,
            &el(0xAE, &[uint(0xD7, 1), uint(0x83, 1), el(0x86, b"V_VP8")].concat()),
        );
        let cluster = el(0x1F43B675, &[uint(0xE7, 10), simple_block(1, 5, true, &[1])].concat());
        let mut sb = SourceBuffer::new();
        sb.append(&[ebml, el_unknown(0x18538067, &[info, tracks].concat()), cluster].concat()).unwrap();
        // 15 ticks * 2 ms
        assert_eq!(sb.next_sample(1).unwrap().pts_us, 30_000);
    }

    #[test]
    fn webm_duration_falls_back_to_next_sample_gap() {
        let ebml = el(0x1A45DFA3, &el(0x4282, b"webm"));
        let tracks = el(
            0x1654AE6B,
            &el(0xAE, &[uint(0xD7, 1), uint(0x83, 1), el(0x86, b"V_VP8")].concat()),
        );
        let cluster = el(
            0x1F43B675,
            &[uint(0xE7, 0), simple_block(1, 0, true, &[1]), simple_block(1, 33, false, &[2])].concat(),
        );
        let mut sb = SourceBuffer::new();
        sb.append(&[ebml, el_unknown(0x18538067, &tracks), cluster].concat()).unwrap();
        assert_eq!(sb.next_sample(1).unwrap().duration_us, 33_000);
    }

    #[test]
    fn webm_unknown_size_non_master_is_an_error() {
        let mut sb = SourceBuffer::new();
        let mut stream = webm_init();
        stream.extend(el_unknown(0xEC, &[]));
        assert!(sb.append(&stream).is_err());
    }

    #[test]
    fn container_detection_needs_enough_bytes() {
        let mut sb = SourceBuffer::new();
        let init = webm_init();
        assert_eq!(sb.append(&init[..2]).unwrap(), AppendResult::default());
        assert!(sb.append(&init[2..]).unwrap().init_segment);
    }
}
