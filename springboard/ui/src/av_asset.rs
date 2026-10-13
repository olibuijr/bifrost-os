//! Asset loading, composition, audio mix and trimmed export for the playback model.
//! Mirrors the asset, composition track, audio mix input parameter and export session types of the media framework.
//! Pure Rust: audio is decoded PCM (16-bit WAV in and out), times are microseconds, nothing here touches a device.

use crate::export_session::{ExportError, ExportSession};

const US: i64 = 1_000_000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AssetError {
    /// The bytes are not a 16-bit PCM WAV file.
    BadFormat,
    /// Sample rate or channel count differs from the composition track.
    FormatMismatch,
    /// A time range lies outside the media or has no length.
    OutOfRange,
}

/// Interleaved float audio.
#[derive(Clone, Debug, PartialEq)]
pub struct Pcm {
    pub sample_rate: u32,
    pub channels: u16,
    pub samples: Vec<f32>,
}

impl Pcm {
    pub fn frames(&self) -> usize {
        self.samples.len() / self.channels.max(1) as usize
    }
    pub fn duration_us(&self) -> i64 {
        self.frames() as i64 * US / self.sample_rate.max(1) as i64
    }
    /// The frame index nearest a time.
    pub fn frame_at(&self, us: i64) -> usize {
        ((us as i128 * self.sample_rate as i128 + (US / 2) as i128) / US as i128).max(0) as usize
    }
    /// A sine tone, the fixture generator for tests.
    pub fn sine(sample_rate: u32, channels: u16, hz: f32, seconds: f32) -> Pcm {
        let frames = (sample_rate as f32 * seconds) as usize;
        let mut samples = Vec::with_capacity(frames * channels as usize);
        for i in 0..frames {
            let v = (2.0 * std::f32::consts::PI * hz * i as f32 / sample_rate as f32).sin() * 0.5;
            for _ in 0..channels {
                samples.push(v);
            }
        }
        Pcm { sample_rate, channels, samples }
    }
    /// The frames from `start_us` for `duration_us`.
    pub fn slice(&self, start_us: i64, duration_us: i64) -> Result<Pcm, AssetError> {
        if start_us < 0 || duration_us <= 0 || start_us + duration_us > self.duration_us() {
            return Err(AssetError::OutOfRange);
        }
        let ch = self.channels as usize;
        let a = self.frame_at(start_us) * ch;
        let b = (self.frame_at(start_us + duration_us) * ch).min(self.samples.len());
        Ok(Pcm { sample_rate: self.sample_rate, channels: self.channels, samples: self.samples[a..b].to_vec() })
    }
    pub fn from_wav(bytes: &[u8]) -> Result<Pcm, AssetError> {
        if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
            return Err(AssetError::BadFormat);
        }
        let (mut pos, mut fmt, mut data) = (12usize, None, None);
        while pos + 8 <= bytes.len() {
            let size = u32::from_le_bytes([bytes[pos + 4], bytes[pos + 5], bytes[pos + 6], bytes[pos + 7]]) as usize;
            let body = pos + 8;
            let end = (body + size).min(bytes.len());
            match &bytes[pos..pos + 4] {
                b"fmt " if end - body >= 16 => {
                    let b = &bytes[body..end];
                    let tag = u16::from_le_bytes([b[0], b[1]]);
                    let ch = u16::from_le_bytes([b[2], b[3]]);
                    let rate = u32::from_le_bytes([b[4], b[5], b[6], b[7]]);
                    let bits = u16::from_le_bytes([b[14], b[15]]);
                    fmt = Some((tag, ch, rate, bits));
                }
                b"data" => data = Some(&bytes[body..end]),
                _ => {}
            }
            pos = body + size + (size & 1);
        }
        match (fmt, data) {
            (Some((1, ch, rate, 16)), Some(d)) if ch > 0 && rate > 0 => {
                let samples = d.chunks_exact(2).map(|c| i16::from_le_bytes([c[0], c[1]]) as f32 / 32768.0).collect();
                Ok(Pcm { sample_rate: rate, channels: ch, samples })
            }
            _ => Err(AssetError::BadFormat),
        }
    }
    pub fn to_wav(&self) -> Vec<u8> {
        let data_len = (self.samples.len() * 2) as u32;
        let block = self.channels as u32 * 2;
        let mut o = Vec::with_capacity(44 + data_len as usize);
        o.extend_from_slice(b"RIFF");
        o.extend_from_slice(&(36 + data_len).to_le_bytes());
        o.extend_from_slice(b"WAVEfmt ");
        o.extend_from_slice(&16u32.to_le_bytes());
        o.extend_from_slice(&1u16.to_le_bytes());
        o.extend_from_slice(&self.channels.to_le_bytes());
        o.extend_from_slice(&self.sample_rate.to_le_bytes());
        o.extend_from_slice(&(self.sample_rate * block).to_le_bytes());
        o.extend_from_slice(&(block as u16).to_le_bytes());
        o.extend_from_slice(&16u16.to_le_bytes());
        o.extend_from_slice(b"data");
        o.extend_from_slice(&data_len.to_le_bytes());
        for s in &self.samples {
            o.extend_from_slice(&((s.clamp(-1.0, 1.0) * 32767.0).round() as i16).to_le_bytes());
        }
        o
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MediaKind {
    Audio,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AssetTrack {
    pub track_id: u32,
    pub media_kind: MediaKind,
    pub duration_us: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AssetKey {
    Duration,
    Tracks,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoadStatus {
    Loaded,
    Failed,
}

/// A loaded (or failed) media asset.
#[derive(Clone, Debug, PartialEq)]
pub struct Asset {
    pub url: String,
    media: Result<Pcm, AssetError>,
}

impl Asset {
    pub fn from_wav_bytes(url: &str, bytes: &[u8]) -> Asset {
        Asset { url: url.to_string(), media: Pcm::from_wav(bytes) }
    }
    pub fn from_pcm(url: &str, pcm: Pcm) -> Asset {
        Asset { url: url.to_string(), media: Ok(pcm) }
    }
    pub fn pcm(&self) -> Option<&Pcm> {
        self.media.as_ref().ok()
    }
    pub fn load_status(&self, _key: AssetKey) -> LoadStatus {
        if self.media.is_ok() { LoadStatus::Loaded } else { LoadStatus::Failed }
    }
    pub fn duration_us(&self) -> Option<i64> {
        self.pcm().map(Pcm::duration_us)
    }
    pub fn tracks(&self) -> Vec<AssetTrack> {
        self.pcm()
            .map(|p| vec![AssetTrack { track_id: 1, media_kind: MediaKind::Audio, duration_us: p.duration_us() }])
            .unwrap_or_default()
    }
}

/// A mutable composition track: source ranges spliced into one audio timeline.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MutableCompositionTrack {
    pcm: Option<Pcm>,
}

impl MutableCompositionTrack {
    pub fn duration_us(&self) -> i64 {
        self.pcm.as_ref().map_or(0, Pcm::duration_us)
    }
    /// Inserts `range` (start, duration) of the asset's audio at `at_us` on this track, shifting later media.
    pub fn insert_time_range(&mut self, asset: &Asset, range: (i64, i64), at_us: i64) -> Result<(), AssetError> {
        let src = asset.pcm().ok_or(AssetError::BadFormat)?.slice(range.0, range.1)?;
        let Some(cur) = self.pcm.as_mut() else {
            if at_us != 0 {
                return Err(AssetError::OutOfRange);
            }
            self.pcm = Some(src);
            return Ok(());
        };
        if cur.sample_rate != src.sample_rate || cur.channels != src.channels {
            return Err(AssetError::FormatMismatch);
        }
        if at_us < 0 || at_us > cur.duration_us() {
            return Err(AssetError::OutOfRange);
        }
        let at = (cur.frame_at(at_us) * cur.channels as usize).min(cur.samples.len());
        cur.samples.splice(at..at, src.samples);
        Ok(())
    }
    pub fn pcm(&self) -> Option<&Pcm> {
        self.pcm.as_ref()
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct MutableComposition {
    tracks: Vec<MutableCompositionTrack>,
}

impl MutableComposition {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn add_track(&mut self) -> usize {
        self.tracks.push(MutableCompositionTrack::default());
        self.tracks.len() - 1
    }
    pub fn track_mut(&mut self, i: usize) -> Option<&mut MutableCompositionTrack> {
        self.tracks.get_mut(i)
    }
    pub fn duration_us(&self) -> i64 {
        self.tracks.iter().map(MutableCompositionTrack::duration_us).max().unwrap_or(0)
    }
    /// The tracks summed into one asset, ready for a player item.
    pub fn as_asset(&self, url: &str) -> Asset {
        let Some(first) = self.tracks.iter().find_map(|t| t.pcm()) else {
            return Asset { url: url.to_string(), media: Err(AssetError::BadFormat) };
        };
        let len = self.tracks.iter().filter_map(|t| t.pcm()).map(|p| p.samples.len()).max().unwrap_or(0);
        let mut out = Pcm { sample_rate: first.sample_rate, channels: first.channels, samples: vec![0.0; len] };
        for p in self.tracks.iter().filter_map(|t| t.pcm()) {
            for (o, s) in out.samples.iter_mut().zip(&p.samples) {
                *o += s;
            }
        }
        Asset::from_pcm(url, out)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Ramp {
    start_us: i64,
    end_us: i64,
    from: f32,
    to: f32,
}

/// Volume ramps for one audio input, like a mutable audio mix input parameter set.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MutableAudioMixInputParameters {
    ramps: Vec<Ramp>,
}

impl MutableAudioMixInputParameters {
    pub fn new() -> Self {
        Self::default()
    }
    /// Volume goes linearly from `from` to `to` over `range` (start, duration).
    pub fn set_volume_ramp(&mut self, from: f32, to: f32, range: (i64, i64)) {
        self.ramps.push(Ramp { start_us: range.0, end_us: range.0 + range.1, from, to });
        self.ramps.sort_by_key(|r| r.start_us);
    }
    /// A step to `volume` at `at_us`.
    pub fn set_volume(&mut self, volume: f32, at_us: i64) {
        self.set_volume_ramp(volume, volume, (at_us, 0));
    }
    /// Volume at a time: 1 before any ramp, the ramp value inside one, the last ramp's end value after it.
    pub fn volume_at(&self, t: i64) -> f32 {
        let mut v = 1.0;
        for r in &self.ramps {
            if t < r.start_us {
                break;
            }
            v = if t >= r.end_us {
                r.to
            } else {
                r.from + (r.to - r.from) * (t - r.start_us) as f32 / (r.end_us - r.start_us) as f32
            };
        }
        v
    }
    pub fn apply(&self, pcm: &Pcm) -> Pcm {
        let ch = pcm.channels.max(1) as usize;
        let mut out = pcm.clone();
        for (i, s) in out.samples.iter_mut().enumerate() {
            let t = (i / ch) as i64 * US / pcm.sample_rate as i64;
            *s *= self.volume_at(t);
        }
        out
    }
}

/// Exports a trimmed (and optionally mixed) WAV through an export session. `preset` is an opaque name.
pub fn export_wav(
    asset: &Asset,
    preset: &str,
    range: Option<(i64, i64)>,
    mix: Option<&MutableAudioMixInputParameters>,
) -> Result<Vec<u8>, ExportError> {
    let pcm = asset.pcm().ok_or(ExportError::NoSource)?;
    let mut session = ExportSession::new(preset, pcm.duration_us())?;
    session.set_supported_file_types(&["wav"]);
    if let Some((s, d)) = range {
        session.set_time_range(s, d)?;
    }
    session.start("wav")?;
    let (s, d) = session.time_range();
    let mut out = pcm.slice(s, d).map_err(|_| ExportError::RangeOutOfSource)?;
    if let Some(m) = mix {
        out = m.apply(&out);
    }
    let bytes = out.to_wav();
    if !session.fits_limit(bytes.len() as u64) {
        session.fail("file length limit").ok();
        return Err(ExportError::Finished);
    }
    session.complete()?;
    Ok(bytes)
}
