//! The audio converter (task 13): linear PCM sample formats (integer and float, either endianness), channel
//! mixing matrices and sample-rate conversion in one object. Interleaved buffers only; planar f32 goes through
//! [`interleave`] and [`deinterleave`]. Samples are floats in -1.0 to 1.0: integers decode as `v / 2^(bits-1)` and
//! encode as `round(x * 2^(bits-1))` clamped to the integer range, so every S16 value survives a round trip
//! exactly. Not real-time safe (it allocates); the server's own mixing path never calls it.

use crate::resample::{Quality, SincResampler};
use bifrost_audio_types::{AudioError, FormatFlags, FormatId, StreamFormat};

/// One sample encoding.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SampleFormat {
    /// Unsigned 8 bit, 128 is silence.
    U8,
    /// Signed 8 bit.
    S8,
    S16,
    /// 24 bits packed in 3 bytes.
    S24,
    /// 24 bits in the low bits of 4 bytes (sign extended on decode).
    S24In32,
    S32,
    F32,
    F64,
}

impl SampleFormat {
    /// Bytes per sample.
    pub const fn bytes(self) -> usize {
        match self {
            SampleFormat::U8 | SampleFormat::S8 => 1,
            SampleFormat::S16 => 2,
            SampleFormat::S24 => 3,
            SampleFormat::S24In32 | SampleFormat::S32 | SampleFormat::F32 => 4,
            SampleFormat::F64 => 8,
        }
    }

    /// Significant bits per sample.
    pub const fn bits(self) -> u32 {
        match self {
            SampleFormat::U8 | SampleFormat::S8 => 8,
            SampleFormat::S16 => 16,
            SampleFormat::S24 | SampleFormat::S24In32 => 24,
            SampleFormat::S32 | SampleFormat::F32 => 32,
            SampleFormat::F64 => 64,
        }
    }

    pub const fn is_float(self) -> bool {
        matches!(self, SampleFormat::F32 | SampleFormat::F64)
    }

    /// The sample format and endianness (true = big endian) of an interleaved linear PCM stream format.
    pub fn from_stream(f: &StreamFormat) -> Result<(SampleFormat, bool), AudioError> {
        if f.format != FormatId::LinearPcm || f.channels == 0 || !f.is_interleaved() {
            return Err(AudioError::BadValue);
        }
        let container = f.bytes_per_frame / f.channels;
        let be = f.flags.contains(FormatFlags::BIG_ENDIAN);
        let signed = f.flags.contains(FormatFlags::SIGNED_INTEGER);
        let sf = match (f.is_float(), f.bits_per_channel, container) {
            (true, 32, 4) => SampleFormat::F32,
            (true, 64, 8) => SampleFormat::F64,
            (false, 8, 1) if signed => SampleFormat::S8,
            (false, 8, 1) => SampleFormat::U8,
            (false, 16, 2) if signed => SampleFormat::S16,
            (false, 24, 3) if signed => SampleFormat::S24,
            (false, 24, 4) if signed && f.flags.contains(FormatFlags::ALIGNED_HIGH) => SampleFormat::S32,
            (false, 24, 4) if signed => SampleFormat::S24In32,
            (false, 32, 4) if signed => SampleFormat::S32,
            _ => return Err(AudioError::BadValue),
        };
        Ok((sf, be))
    }

    /// The interleaved stream format for this encoding.
    pub fn stream_format(self, big_endian: bool, sample_rate: f64, channels: u32) -> StreamFormat {
        let mut flags = FormatFlags::PACKED;
        if self.is_float() {
            flags = flags | FormatFlags::FLOAT;
        } else if self != SampleFormat::U8 {
            flags = flags | FormatFlags::SIGNED_INTEGER;
        }
        if big_endian {
            flags = flags | FormatFlags::BIG_ENDIAN;
        }
        let bytes = self.bytes() as u32;
        StreamFormat {
            sample_rate,
            format: FormatId::LinearPcm,
            flags,
            bytes_per_packet: bytes * channels,
            frames_per_packet: 1,
            bytes_per_frame: bytes * channels,
            channels,
            bits_per_channel: self.bits(),
        }
    }
}

/// Decodes `bytes` (whole samples; a ragged tail is ignored) and appends floats to `out`.
pub fn decode_samples(bytes: &[u8], fmt: SampleFormat, big_endian: bool, out: &mut Vec<f32>) {
    let n = fmt.bytes();
    out.reserve(bytes.len() / n);
    for c in bytes.chunks_exact(n) {
        let v = match fmt {
            SampleFormat::U8 => (c[0] as f32 - 128.0) / 128.0,
            SampleFormat::S8 => (c[0] as i8) as f32 / 128.0,
            SampleFormat::S16 => {
                let a = [c[0], c[1]];
                (if big_endian { i16::from_be_bytes(a) } else { i16::from_le_bytes(a) }) as f32 / 32768.0
            }
            SampleFormat::S24 => {
                let raw = if big_endian {
                    (c[0] as i32) << 16 | (c[1] as i32) << 8 | c[2] as i32
                } else {
                    (c[2] as i32) << 16 | (c[1] as i32) << 8 | c[0] as i32
                };
                ((raw << 8) >> 8) as f32 / 8_388_608.0
            }
            SampleFormat::S24In32 => {
                let a = [c[0], c[1], c[2], c[3]];
                let raw = if big_endian { i32::from_be_bytes(a) } else { i32::from_le_bytes(a) };
                ((raw << 8) >> 8) as f32 / 8_388_608.0
            }
            SampleFormat::S32 => {
                let a = [c[0], c[1], c[2], c[3]];
                let raw = if big_endian { i32::from_be_bytes(a) } else { i32::from_le_bytes(a) };
                (raw as f64 / 2_147_483_648.0) as f32
            }
            SampleFormat::F32 => {
                let a = [c[0], c[1], c[2], c[3]];
                f32::from_bits(if big_endian { u32::from_be_bytes(a) } else { u32::from_le_bytes(a) })
            }
            SampleFormat::F64 => {
                let mut a = [0u8; 8];
                a.copy_from_slice(c);
                f64::from_bits(if big_endian { u64::from_be_bytes(a) } else { u64::from_le_bytes(a) }) as f32
            }
        };
        out.push(v);
    }
}

/// Quantises a float to a signed integer of `bits` bits (NaN becomes 0).
fn quantise(x: f32, bits: u32) -> i64 {
    if x.is_nan() {
        return 0;
    }
    let scale = (1u64 << (bits - 1)) as f64;
    let v = (x as f64 * scale).round();
    v.clamp(-scale, scale - 1.0) as i64
}

/// Encodes floats and appends the bytes to `out`.
pub fn encode_samples(samples: &[f32], fmt: SampleFormat, big_endian: bool, out: &mut Vec<u8>) {
    out.reserve(samples.len() * fmt.bytes());
    for &x in samples {
        match fmt {
            SampleFormat::U8 => out.push((quantise(x, 8) + 128) as u8),
            SampleFormat::S8 => out.push(quantise(x, 8) as i8 as u8),
            SampleFormat::S16 => {
                let v = quantise(x, 16) as i16;
                out.extend_from_slice(&if big_endian { v.to_be_bytes() } else { v.to_le_bytes() });
            }
            SampleFormat::S24 => {
                let v = quantise(x, 24) as i32;
                if big_endian {
                    out.extend_from_slice(&[(v >> 16) as u8, (v >> 8) as u8, v as u8]);
                } else {
                    out.extend_from_slice(&[v as u8, (v >> 8) as u8, (v >> 16) as u8]);
                }
            }
            SampleFormat::S24In32 => {
                let v = quantise(x, 24) as i32;
                out.extend_from_slice(&if big_endian { v.to_be_bytes() } else { v.to_le_bytes() });
            }
            SampleFormat::S32 => {
                let v = quantise(x, 32) as i32;
                out.extend_from_slice(&if big_endian { v.to_be_bytes() } else { v.to_le_bytes() });
            }
            SampleFormat::F32 => {
                let b = x.to_bits();
                out.extend_from_slice(&if big_endian { b.to_be_bytes() } else { b.to_le_bytes() });
            }
            SampleFormat::F64 => {
                let b = (x as f64).to_bits();
                out.extend_from_slice(&if big_endian { b.to_be_bytes() } else { b.to_le_bytes() });
            }
        }
    }
}

/// The default mixing matrix from `from` to `to` channels, row-major with `to` rows of `from` columns (so
/// `out[r] = sum(m[r * from + c] * in[c])`). Equal counts are the identity; mono feeds every output; any input to
/// mono is the average; 5.1 (front left and right, centre, LFE, back left and right) to stereo folds the centre
/// and backs in at -3 dB, scaled so full-scale input cannot clip; other pairs copy the shared channels and leave the
/// extra outputs silent.
pub fn default_matrix(from: usize, to: usize) -> Vec<f32> {
    let mut m = vec![0.0f32; from * to];
    if from == 0 || to == 0 {
        return m;
    }
    if from == to {
        for i in 0..from {
            m[i * from + i] = 1.0;
        }
    } else if from == 1 {
        m.iter_mut().for_each(|v| *v = 1.0);
    } else if to == 1 {
        m.iter_mut().for_each(|v| *v = 1.0 / from as f32);
    } else if from == 6 && to == 2 {
        let g = 1.0 / (1.0 + 2.0 * std::f32::consts::FRAC_1_SQRT_2);
        let s = g * std::f32::consts::FRAC_1_SQRT_2;
        // left: FL, C, BL; right: FR, C, BR
        m[0] = g;
        m[2] = s;
        m[4] = s;
        m[6 + 1] = g;
        m[6 + 2] = s;
        m[6 + 5] = s;
    } else {
        for r in 0..to.min(from) {
            m[r * from + r] = 1.0;
        }
    }
    m
}

/// Applies a mixing matrix to interleaved `input`; `out` is replaced.
pub fn apply_matrix(m: &[f32], from: usize, to: usize, input: &[f32], out: &mut Vec<f32>) {
    out.clear();
    if from == 0 || to == 0 {
        return;
    }
    out.reserve(input.len() / from * to);
    for frame in input.chunks_exact(from) {
        for r in 0..to {
            let row = &m[r * from..(r + 1) * from];
            out.push(row.iter().zip(frame).map(|(a, b)| a * b).sum());
        }
    }
}

/// Interleaves per-channel slices (all the same length) into `out`, replacing it.
pub fn interleave(planar: &[&[f32]], out: &mut Vec<f32>) {
    out.clear();
    let frames = planar.first().map_or(0, |p| p.len());
    out.reserve(frames * planar.len());
    for i in 0..frames {
        for ch in planar {
            out.push(ch[i]);
        }
    }
}

/// Splits interleaved `input` into the per-channel slices of `planar` (each as long as the frame count).
pub fn deinterleave(input: &[f32], planar: &mut [&mut [f32]]) {
    let ch = planar.len();
    if ch == 0 {
        return;
    }
    for (i, frame) in input.chunks_exact(ch).enumerate() {
        for (c, p) in planar.iter_mut().enumerate() {
            if i < p.len() {
                p[i] = frame[c];
            }
        }
    }
}

/// A converter from one interleaved linear PCM format to another: format, channel count and sample rate.
pub struct Converter {
    src: SampleFormat,
    src_be: bool,
    dst: SampleFormat,
    dst_be: bool,
    src_channels: usize,
    dst_channels: usize,
    matrix: Vec<f32>,
    resampler: Option<SincResampler>,
    a: Vec<f32>,
    b: Vec<f32>,
}

impl Converter {
    /// Builds a converter. Both formats must be interleaved linear PCM this module understands.
    pub fn new(src: &StreamFormat, dst: &StreamFormat) -> Result<Converter, AudioError> {
        Converter::with_quality(src, dst, Quality::Standard)
    }

    pub fn with_quality(src: &StreamFormat, dst: &StreamFormat, quality: Quality) -> Result<Converter, AudioError> {
        let (sf, sbe) = SampleFormat::from_stream(src)?;
        let (df, dbe) = SampleFormat::from_stream(dst)?;
        let (sr, dr) = (src.sample_rate.round(), dst.sample_rate.round());
        if !(sr >= 1.0 && dr >= 1.0 && sr <= u32::MAX as f64 && dr <= u32::MAX as f64) {
            return Err(AudioError::BadValue);
        }
        let (sc, dc) = (src.channels as usize, dst.channels as usize);
        let resampler = if sr != dr { Some(SincResampler::new(sr as u32, dr as u32, dc, quality)?) } else { None };
        Ok(Converter {
            src: sf,
            src_be: sbe,
            dst: df,
            dst_be: dbe,
            src_channels: sc,
            dst_channels: dc,
            matrix: default_matrix(sc, dc),
            resampler,
            a: Vec::new(),
            b: Vec::new(),
        })
    }

    /// Replaces the channel mixing matrix (`dst_channels` rows of `src_channels` columns).
    pub fn set_matrix(&mut self, m: &[f32]) -> Result<(), AudioError> {
        if m.len() != self.src_channels * self.dst_channels {
            return Err(AudioError::BadValue);
        }
        self.matrix = m.to_vec();
        Ok(())
    }

    /// Converts `input` (whole source frames) and appends the destination bytes to `out`. With a rate change the
    /// output lags by the resampler's latency until [`Converter::finish`].
    pub fn convert(&mut self, input: &[u8], out: &mut Vec<u8>) -> Result<(), AudioError> {
        let frame_bytes = self.src.bytes() * self.src_channels;
        if input.len() % frame_bytes != 0 {
            return Err(AudioError::BadValue);
        }
        self.a.clear();
        decode_samples(input, self.src, self.src_be, &mut self.a);
        apply_matrix(&self.matrix, self.src_channels, self.dst_channels, &self.a, &mut self.b);
        match &mut self.resampler {
            Some(r) => {
                self.a.clear();
                r.process(&self.b, &mut self.a);
                encode_samples(&self.a, self.dst, self.dst_be, out);
            }
            None => encode_samples(&self.b, self.dst, self.dst_be, out),
        }
        Ok(())
    }

    /// Flushes the resampler's tail and starts a new stream.
    pub fn finish(&mut self, out: &mut Vec<u8>) {
        if let Some(r) = &mut self.resampler {
            self.a.clear();
            r.finish(&mut self.a);
            encode_samples(&self.a, self.dst, self.dst_be, out);
        }
    }

    /// Forgets resampler state.
    pub fn reset(&mut self) {
        if let Some(r) = &mut self.resampler {
            r.reset();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fmt(sf: SampleFormat, be: bool, rate: f64, ch: u32) -> StreamFormat {
        sf.stream_format(be, rate, ch)
    }

    #[test]
    fn s16_decodes_to_the_float_range() {
        let mut out = Vec::new();
        decode_samples(&[0x00, 0x80, 0xFF, 0x7F, 0x00, 0x00, 0x00, 0x40], SampleFormat::S16, false, &mut out);
        // -32768, 32767, 0, 16384
        assert_eq!(out, vec![-1.0, 32767.0 / 32768.0, 0.0, 0.5]);
    }

    #[test]
    fn s16_round_trips_every_value() {
        let all: Vec<f32> = (-32768..=32767).map(|v: i32| v as f32 / 32768.0).collect();
        for be in [false, true] {
            let mut bytes = Vec::new();
            encode_samples(&all, SampleFormat::S16, be, &mut bytes);
            let mut back = Vec::new();
            decode_samples(&bytes, SampleFormat::S16, be, &mut back);
            assert_eq!(back, all);
        }
    }

    #[test]
    fn full_scale_float_clamps_to_the_integer_range() {
        let mut b = Vec::new();
        encode_samples(&[1.0, -1.0, 2.0, f32::NAN], SampleFormat::S16, false, &mut b);
        assert_eq!(b, vec![0xFF, 0x7F, 0x00, 0x80, 0xFF, 0x7F, 0x00, 0x00]);
    }

    #[test]
    fn u8_has_silence_at_128() {
        let mut out = Vec::new();
        decode_samples(&[0, 128, 255], SampleFormat::U8, false, &mut out);
        assert_eq!(out, vec![-1.0, 0.0, 127.0 / 128.0]);
        let mut b = Vec::new();
        encode_samples(&[-1.0, 0.0, 0.5], SampleFormat::U8, false, &mut b);
        assert_eq!(b, vec![0, 128, 192]);
    }

    #[test]
    fn s24_packed_both_endians() {
        // 0x400000 = 4194304 = 0.5 of 2^23
        let mut out = Vec::new();
        decode_samples(&[0x00, 0x00, 0x40], SampleFormat::S24, false, &mut out);
        decode_samples(&[0x40, 0x00, 0x00], SampleFormat::S24, true, &mut out);
        // 0x800000 is the most negative value
        decode_samples(&[0x00, 0x00, 0x80], SampleFormat::S24, false, &mut out);
        assert_eq!(out, vec![0.5, 0.5, -1.0]);
        let mut b = Vec::new();
        encode_samples(&[0.5, -1.0], SampleFormat::S24, false, &mut b);
        assert_eq!(b, vec![0x00, 0x00, 0x40, 0x00, 0x00, 0x80]);
    }

    #[test]
    fn s24_in_32_sign_extends() {
        let mut out = Vec::new();
        decode_samples(&[0x00, 0x00, 0x80, 0xFF], SampleFormat::S24In32, false, &mut out);
        assert_eq!(out, vec![-1.0]);
        let mut b = Vec::new();
        encode_samples(&[-1.0], SampleFormat::S24In32, false, &mut b);
        assert_eq!(b, vec![0x00, 0x00, 0x80, 0xFF]);
    }

    #[test]
    fn s32_and_floats() {
        let mut out = Vec::new();
        decode_samples(&0x4000_0000i32.to_le_bytes(), SampleFormat::S32, false, &mut out);
        decode_samples(&0.25f32.to_be_bytes(), SampleFormat::F32, true, &mut out);
        decode_samples(&(-0.5f64).to_le_bytes(), SampleFormat::F64, false, &mut out);
        assert_eq!(out, vec![0.5, 0.25, -0.5]);
    }

    #[test]
    fn stream_format_round_trip() {
        for sf in [
            SampleFormat::U8,
            SampleFormat::S8,
            SampleFormat::S16,
            SampleFormat::S24,
            SampleFormat::S24In32,
            SampleFormat::S32,
            SampleFormat::F32,
            SampleFormat::F64,
        ] {
            let f = fmt(sf, false, 48000.0, 2);
            let (back, be) = SampleFormat::from_stream(&f).unwrap();
            assert_eq!((back, be), (sf, false));
        }
        let mut bad = fmt(SampleFormat::S16, false, 48000.0, 2);
        bad.channels = 0;
        assert_eq!(SampleFormat::from_stream(&bad), Err(AudioError::BadValue));
    }

    #[test]
    fn default_matrices() {
        assert_eq!(default_matrix(2, 2), vec![1.0, 0.0, 0.0, 1.0]);
        assert_eq!(default_matrix(1, 2), vec![1.0, 1.0]);
        assert_eq!(default_matrix(2, 1), vec![0.5, 0.5]);
        assert_eq!(default_matrix(2, 3), vec![1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);
        // 5.1 to stereo: full-scale input on every channel but the LFE cannot exceed 1
        let m = default_matrix(6, 2);
        let l: f32 = m[..6].iter().sum();
        assert!((l - 1.0).abs() < 1e-6);
        assert_eq!(m[3], 0.0);
    }

    #[test]
    fn apply_matrix_mixes_frames() {
        let mut out = Vec::new();
        apply_matrix(&default_matrix(2, 1), 2, 1, &[1.0, 0.0, 0.5, 0.5], &mut out);
        assert_eq!(out, vec![0.5, 0.5]);
        apply_matrix(&default_matrix(1, 2), 1, 2, &[0.25, -0.5], &mut out);
        assert_eq!(out, vec![0.25, 0.25, -0.5, -0.5]);
    }

    #[test]
    fn interleave_and_back() {
        let l = [1.0, 2.0, 3.0];
        let r = [4.0, 5.0, 6.0];
        let mut out = Vec::new();
        interleave(&[&l, &r], &mut out);
        assert_eq!(out, vec![1.0, 4.0, 2.0, 5.0, 3.0, 6.0]);
        let (mut a, mut b) = ([0.0; 3], [0.0; 3]);
        deinterleave(&out, &mut [&mut a, &mut b]);
        assert_eq!((a, b), (l, r));
    }

    #[test]
    fn converter_s16_stereo_to_f32_mono() {
        let src = fmt(SampleFormat::S16, false, 48000.0, 2);
        let dst = fmt(SampleFormat::F32, false, 48000.0, 1);
        let mut c = Converter::new(&src, &dst).unwrap();
        // frame 1: (16384, -16384) -> 0; frame 2: (16384, 16384) -> 0.5
        let mut input = Vec::new();
        for v in [16384i16, -16384, 16384, 16384] {
            input.extend_from_slice(&v.to_le_bytes());
        }
        let mut out = Vec::new();
        c.convert(&input, &mut out).unwrap();
        let mut f = Vec::new();
        decode_samples(&out, SampleFormat::F32, false, &mut f);
        assert_eq!(f, vec![0.0, 0.5]);
        // a ragged input is refused
        assert_eq!(c.convert(&input[..3], &mut out), Err(AudioError::BadValue));
    }

    #[test]
    fn converter_changes_the_rate() {
        let src = fmt(SampleFormat::F32, false, 44100.0, 1);
        let dst = fmt(SampleFormat::F32, false, 48000.0, 1);
        let mut c = Converter::new(&src, &dst).unwrap();
        let mut input = Vec::new();
        encode_samples(&vec![0.5f32; 4410], SampleFormat::F32, false, &mut input);
        let mut out = Vec::new();
        c.convert(&input, &mut out).unwrap();
        c.finish(&mut out);
        // 4410 frames at 44.1 kHz are exactly 4800 frames at 48 kHz
        assert_eq!(out.len() / 4, 4800);
        let mut f = Vec::new();
        decode_samples(&out, SampleFormat::F32, false, &mut f);
        assert!((f[2400] - 0.5).abs() < 1e-3);
    }
}
