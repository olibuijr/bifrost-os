//! AAC-LC decoding (symphonia, pure Rust) and conversion of whatever the
//! stream carries to the 48 kHz stereo S16 the output device runs at.

use super::Error;
use symphonia_core::codecs::audio::well_known::CODEC_ID_AAC;
use symphonia_core::codecs::audio::{AudioCodecParameters, AudioDecoder, AudioDecoderOptions};
use symphonia_core::packet::PacketRef;
use symphonia_core::units::{Duration, Timestamp};

/// Downmixes interleaved multichannel S16 to stereo. The channel order is the one the AAC
/// decoder produces, the bitstream element order: 3.0 = C L R; 4.0 = C L R Cs; 5.0 = C L R Ls Rs;
/// 5.1 = C L R Ls Rs LFE; 7.1 = C Lc Rc Ls Rs Lb Rb LFE. Centre and surrounds go in at -3 dB, the
/// LFE is dropped (as most players do), and the sum is scaled so a full-scale mix rarely clips.
pub fn downmix_to_stereo(src: &[i16], channels: usize, out: &mut Vec<i16>) {
    out.clear();
    if channels < 3 {
        out.extend_from_slice(src);
        return;
    }
    const K: f32 = 0.707;
    // (centre, left, right, left surround, right surround, rear centre) indices and weights
    let (c, l, r, ls, rs, cs): (usize, usize, usize, Option<usize>, Option<usize>, Option<usize>) = match channels {
        3 => (0, 1, 2, None, None, None),
        4 => (0, 1, 2, None, None, Some(3)),
        5 | 6 => (0, 1, 2, Some(3), Some(4), None),
        // 7.1: front-centre pair joins the fronts, side and back pairs are the surrounds
        _ => (0, 1, 2, Some(3), Some(4), None),
    };
    let norm = 1.0 / (1.0 + K + K);
    for frame in src.chunks_exact(channels) {
        let f = |i: usize| frame[i] as f32;
        let mut left = f(l) + K * f(c);
        let mut right = f(r) + K * f(c);
        if let (Some(a), Some(b)) = (ls, rs) {
            left += K * f(a);
            right += K * f(b);
        }
        if let Some(m) = cs {
            left += K * f(m);
            right += K * f(m);
        }
        if channels >= 8 {
            // Lc/Rc (index 1/2 are the front pair in 7.1 order C Lc Rc ...): fold them into L/R
            // and treat the back pair as extra surrounds
            left = f(1) + K * f(c) + K * f(3) + K * f(5);
            right = f(2) + K * f(c) + K * f(4) + K * f(6);
        }
        let scale = if channels >= 8 { 1.0 / (1.0 + 3.0 * K) } else { norm };
        // 1.25: the plain norm is quiet for dialogue, saturate the rare peak instead
        out.push((left * scale * 1.25).round().clamp(-32768.0, 32767.0) as i16);
        out.push((right * scale * 1.25).round().clamp(-32768.0, 32767.0) as i16);
    }
}

/// Output sample rate of the whole audio path.
pub const OUT_RATE: u32 = 48_000;

pub struct Decoder {
    inner: symphonia_codec_aac::AacDecoder,
    rate: u32,
    /// channels in the stream (up to 8); `decode` hands out at most two
    src_channels: usize,
    /// reusable buffer for multichannel packets before the downmix
    wide: Vec<i16>,
}

impl Decoder {
    /// `asc` is the AudioSpecificConfig from the track's `esds`.
    pub fn new(asc: &[u8]) -> Result<Self, Error> {
        let (rate, channels) = super::mp4::asc_params(asc).ok_or(Error::Invalid("AudioSpecificConfig"))?;
        let mut params = AudioCodecParameters::new();
        params.for_codec(CODEC_ID_AAC).with_extra_data(asc.into());
        let inner = symphonia_codec_aac::AacDecoder::try_new(&params, &AudioDecoderOptions::default()).map_err(|_| Error::Invalid("AAC stream"))?;
        Ok(Decoder { inner, rate, src_channels: channels as usize, wide: Vec::new() })
    }

    pub fn rate(&self) -> u32 {
        self.rate
    }

    /// Channels of the samples `decode` returns: mono stays mono, everything else is stereo.
    pub fn channels(&self) -> usize {
        self.src_channels.clamp(1, 2)
    }

    /// Decodes one raw AAC access unit into `out` (replaced) as interleaved
    /// S16 at the stream's own rate and channel count. A corrupt packet
    /// yields no samples.
    pub fn decode(&mut self, packet: &[u8], out: &mut Vec<i16>) -> Result<(), Error> {
        out.clear();
        let p = PacketRef::new(0, Timestamp::new(0), Duration::new(1024), packet);
        match self.inner.decode_ref(&p) {
            Ok(buf) => {
                let spec = buf.spec();
                self.rate = spec.rate();
                self.src_channels = spec.channels().count();
                if self.src_channels <= 2 {
                    buf.copy_to_vec_interleaved(out);
                } else {
                    buf.copy_to_vec_interleaved(&mut self.wide);
                    downmix_to_stereo(&self.wide, self.src_channels, out);
                }
                Ok(())
            }
            // corrupt or truncated packet: skip it, the stream continues
            Err(symphonia_core::errors::Error::DecodeError(_) | symphonia_core::errors::Error::IoError(_)) => Ok(()),
            Err(_) => Err(Error::Invalid("AAC packet")),
        }
    }

    /// Forgets inter-frame state after a seek.
    pub fn reset(&mut self) {
        self.inner.reset();
    }
}

/// Linear-interpolation resampler plus stereo downmix: any (rate,
/// channels) S16 stream in, 48 kHz stereo S16 out, continuous across calls.
pub struct Resampler {
    in_rate: u32,
    channels: usize,
    /// 16.16 position inside the virtual input stream `[last] + input`
    pos: u64,
    last: [i16; 2],
    primed: bool,
}

impl Resampler {
    pub fn new(in_rate: u32, channels: usize) -> Self {
        Resampler { in_rate: in_rate.max(1), channels: channels.max(1), pos: 0, last: [0; 2], primed: false }
    }

    /// Adapts to a stream whose parameters changed mid-way.
    pub fn retune(&mut self, in_rate: u32, channels: usize) {
        if (in_rate, channels) != (self.in_rate, self.channels) {
            *self = Resampler::new(in_rate, channels);
        }
    }

    /// Appends the converted frames to `out`.
    pub fn convert(&mut self, input: &[i16], out: &mut Vec<i16>) {
        let n = self.channels;
        let mut ext: Vec<[i16; 2]> = Vec::with_capacity(input.len() / n + 1);
        if self.primed {
            ext.push(self.last);
        }
        ext.extend(input.chunks_exact(n).map(downmix));
        if ext.len() < 2 {
            if let Some(&f) = ext.last() {
                self.last = f;
                self.primed = true;
            }
            return;
        }
        let step = ((self.in_rate as u64) << 16) / OUT_RATE as u64;
        let end = ((ext.len() - 1) as u64) << 16;
        while self.pos < end {
            let (i, w) = ((self.pos >> 16) as usize, (self.pos & 0xffff) as i64);
            for (&a, &b) in ext[i].iter().zip(&ext[i + 1]) {
                out.push(((a as i64 * (65536 - w) + b as i64 * w) >> 16) as i16);
            }
            self.pos += step;
        }
        self.pos -= end;
        self.last = ext[ext.len() - 1];
        self.primed = true;
    }
}

/// Folds a frame of any channel count to stereo (L R C LFE Ls Rs ... order).
fn downmix(f: &[i16]) -> [i16; 2] {
    match f.len() {
        1 => [f[0], f[0]],
        2 => [f[0], f[1]],
        n => {
            let c = f[2] as i32 * 181 / 256;
            let (ls, rs) = if n >= 6 { (f[4] as i32 * 181 / 256, f[5] as i32 * 181 / 256) } else { (0, 0) };
            let clip = |v: i32| v.clamp(i16::MIN as i32, i16::MAX as i32) as i16;
            [clip(f[0] as i32 + c + ls), clip(f[1] as i32 + c + rs)]
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::mp4::{Demuxer, Kind};
    use super::super::source::MemSource;
    use super::*;

    /// Power of `hz` in a stereo S16 signal, Goertzel on the left channel.
    fn power(stereo: &[i16], rate: u32, hz: f64) -> f64 {
        let n = stereo.len() / 2;
        let k = 2.0 * std::f64::consts::PI * hz / rate as f64;
        let (mut s1, mut s2) = (0.0f64, 0.0f64);
        for i in 0..n {
            let s = stereo[i * 2] as f64 / 32768.0 + 2.0 * k.cos() * s1 - s2;
            s2 = s1;
            s1 = s;
        }
        (s1 * s1 + s2 * s2 - 2.0 * k.cos() * s1 * s2) / n as f64
    }

    #[test]
    fn decodes_the_fixture_tone() {
        let mut src = MemSource::new(include_bytes!("testdata/tiny_baseline.mp4").to_vec());
        let mut d = Demuxer::open(&mut src).unwrap();
        let super::super::mp4::Codec::Aac { asc, .. } = d.audio().unwrap().codec.clone() else { panic!() };
        let mut dec = Decoder::new(&asc).unwrap();
        assert_eq!((dec.rate(), dec.channels()), (48000, 2));
        let mut pcm = Vec::new();
        let mut chunk = Vec::new();
        while let Some(s) = d.next(Kind::Audio, &mut src).unwrap() {
            dec.decode(&s.data, &mut chunk).unwrap();
            pcm.extend_from_slice(&chunk);
        }
        assert!(pcm.len() >= 2 * 1024 * 46, "{} samples", pcm.len());
        let tail = &pcm[pcm.len() / 2..];
        let p440 = power(tail, 48000, 440.0);
        assert!(p440 > 20.0 * power(tail, 48000, 1000.0).max(1e-9), "440 Hz dominates: {p440}");
        assert!(tail.iter().any(|&s| s.abs() > 2000), "has amplitude, peak {}", tail.iter().map(|s| s.abs()).max().unwrap());
        dec.reset();
        assert!(dec.decode(&[0xff; 7], &mut chunk).is_ok(), "garbage packet is not fatal");
    }

    #[test]
    fn stereo_and_mono_pass_through() {
        let mut out = vec![9];
        downmix_to_stereo(&[1, 2, 3, 4], 2, &mut out);
        assert_eq!(out, vec![1, 2, 3, 4]);
        downmix_to_stereo(&[5, 6], 1, &mut out);
        assert_eq!(out, vec![5, 6]);
    }

    #[test]
    fn five_one_puts_the_centre_in_both_ears_and_drops_the_lfe() {
        // one frame, order C L R Ls Rs LFE: only the centre speaks
        let mut out = Vec::new();
        downmix_to_stereo(&[10_000, 0, 0, 0, 0, 20_000], 6, &mut out);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0], out[1], "centre is centred");
        assert!(out[0] > 3_000 && out[0] < 6_000, "centre at about -3 dB scaled, got {}", out[0]);
        // LFE alone is silent
        downmix_to_stereo(&[0, 0, 0, 0, 0, 30_000], 6, &mut out);
        assert_eq!(out, vec![0, 0]);
    }

    #[test]
    fn five_one_keeps_left_and_right_apart_and_surrounds_on_their_side() {
        let mut out = Vec::new();
        downmix_to_stereo(&[0, 10_000, 0, 0, 0, 0], 6, &mut out);
        assert!(out[0] > 0 && out[1] == 0, "left only: {out:?}");
        downmix_to_stereo(&[0, 0, 10_000, 0, 0, 0], 6, &mut out);
        assert!(out[0] == 0 && out[1] > 0, "right only: {out:?}");
        downmix_to_stereo(&[0, 0, 0, 10_000, 0, 0], 6, &mut out);
        assert!(out[0] > 0 && out[1] == 0, "left surround to the left: {out:?}");
        downmix_to_stereo(&[0, 0, 0, 0, 10_000, 0], 6, &mut out);
        assert!(out[0] == 0 && out[1] > 0, "right surround to the right: {out:?}");
    }

    #[test]
    fn loud_mixes_saturate_instead_of_wrapping() {
        let mut out = Vec::new();
        downmix_to_stereo(&[32_767; 6], 6, &mut out);
        assert_eq!(out, vec![32_767, 32_767]);
        downmix_to_stereo(&[-32_768; 6], 6, &mut out);
        assert_eq!(out, vec![-32_768, -32_768]);
    }

    #[test]
    fn every_aac_channel_count_downmixes_whole_frames() {
        let mut out = Vec::new();
        for ch in [3usize, 4, 5, 6, 8] {
            let src = vec![1_000i16; ch * 5];
            downmix_to_stereo(&src, ch, &mut out);
            assert_eq!(out.len(), 10, "{ch} channels");
            assert!(out.iter().all(|&s| s > 0), "{ch} channels: {out:?}");
        }
    }

    #[test]
    fn rejects_a_bad_config() {
        assert!(Decoder::new(&[0x11]).is_err());
    }

    #[test]
    fn resampler_passes_48k_stereo_through_with_one_frame_delay() {
        let mut r = Resampler::new(48000, 2);
        let mut out = Vec::new();
        let input: Vec<i16> = (0..20).collect();
        r.convert(&input[..10], &mut out);
        r.convert(&input[10..], &mut out);
        r.convert(&[], &mut out);
        assert_eq!(out, input[..18]);
    }

    #[test]
    fn resampler_doubles_a_mono_24k_stream() {
        let mut r = Resampler::new(24000, 1);
        let mut out = Vec::new();
        r.convert(&[0, 100, 200, 300], &mut out);
        assert_eq!(out, [0, 0, 50, 50, 100, 100, 150, 150, 200, 200, 250, 250]);
        r.convert(&[400], &mut out);
        assert_eq!(&out[12..], [300, 300, 350, 350]);
    }

    #[test]
    fn resampler_halves_a_96k_stream_and_keeps_rate_over_time() {
        let mut r = Resampler::new(96000, 2);
        let mut out = Vec::new();
        let mut total_in = 0;
        for _ in 0..100 {
            let chunk: Vec<i16> = vec![1000; 2 * 1001];
            total_in += 1001;
            r.convert(&chunk, &mut out);
        }
        let frames = out.len() / 2;
        assert!((frames as i64 - total_in / 2).abs() <= 2, "{frames} out for {total_in} in");
        assert!(out.iter().all(|&s| s == 1000));
    }

    #[test]
    fn downmix_folds_centre_and_surrounds() {
        assert_eq!(downmix(&[5]), [5, 5]);
        assert_eq!(downmix(&[1, 2]), [1, 2]);
        assert_eq!(downmix(&[1000, 2000, 1000, 9999, 0, 1000]), [1707, 3414]);
        assert_eq!(downmix(&[32000, 0, 32000]), [32767, 22625]);
    }
}
