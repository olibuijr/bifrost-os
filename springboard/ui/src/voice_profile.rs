//! The first stage of a personal voice (Voice Banking): measuring a speaker from the samples they recorded. A recorded
//! phrase goes in as mono PCM; what comes out is a [`VoiceProfile`] holding the speaker's pitch (mean and range), how
//! fast they speak (syllable-like energy peaks per second) and how loud. Profiles from the phrases of one bank are merged
//! ([`VoiceProfile::merge`]) and then shape the rule-based synthesiser: [`VoiceProfile::apply`] sets an
//! [`crate::speech::Utterance`]'s pitch multiplier and rate so the system voice speaks at the speaker's pitch and pace.
//!
//! This is speaker adaptation of the synthesiser's delivery settings, not a trained voice: it does not learn the
//! speaker's timbre or phoneme shapes, which needs a trainable speech model. Pitch comes from normalised
//! autocorrelation over 50 ms frames every 10 ms (voiced when the correlation is at least [`VOICED_CORRELATION`], the
//! smallest lag within 90% of the best one wins so a pitch is never reported an octave low); the rate counts prominent
//! peaks of the 20 ms energy envelope at least 100 ms apart. Input above 16 kHz is decimated by box averaging first.

use crate::speech::{Utterance, PITCH_MAX, PITCH_MIN};

/// Sample rates the analysis accepts.
pub const RATE_RANGE: (u32, u32) = (8_000, 48_000);
/// The pitch range searched, in Hz.
pub const F0_RANGE: (f32, f32) = (60.0, 400.0);
/// The normalised correlation a frame needs to count as voiced.
pub const VOICED_CORRELATION: f32 = 0.6;
/// A frame is speech when its energy is at least this fraction of the loudest frame's.
pub const ACTIVE_RATIO: f32 = 0.1;
/// The loudest frame must have at least this RMS (full scale is 1.0), or the sample is taken as silence.
pub const MIN_PEAK_RMS: f32 = 0.005;
/// Voiced frames needed (10 ms each) before a pitch is reported.
pub const MIN_VOICED_FRAMES: usize = 10;
/// The pitch the synthesiser's natural voice has; [`VoiceProfile::pitch_multiplier`] is relative to it.
pub const REFERENCE_F0_HZ: f32 = 140.0;
/// Speaking rates that map to the slowest and fastest synthesiser rate, in syllable peaks per second.
pub const RATE_SLOW_SPS: f32 = 2.0;
pub const RATE_FAST_SPS: f32 = 7.0;

const HOP_SECONDS: f32 = 0.01;
const FRAME_SECONDS: f32 = 0.05;
const ENVELOPE_SECONDS: f32 = 0.02;
const MIN_PEAK_GAP_FRAMES: usize = 10;
const MAX_ANALYSIS_RATE: f32 = 16_000.0;

/// Why a sample could not be measured.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProfileError {
    /// The sample rate is outside [`RATE_RANGE`].
    BadRate,
    /// Less than 100 ms of audio.
    TooShort,
    /// The loudest part is below [`MIN_PEAK_RMS`].
    NoSpeech,
    /// There is sound but fewer than [`MIN_VOICED_FRAMES`] frames have a pitch (noise, a whisper, a click).
    NoVoicedSpeech,
    /// A stored profile text is malformed.
    Parse,
}

/// What was measured of a speaker.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VoiceProfile {
    /// Mean pitch of the voiced frames, in Hz.
    pub mean_f0_hz: f32,
    /// The 5th and 95th percentile of the pitch, in Hz.
    pub low_f0_hz: f32,
    pub high_f0_hz: f32,
    /// Syllable-like energy peaks per second, over the span from the first to the last frame of speech.
    pub syllables_per_second: f32,
    /// Mean RMS of the speech frames, full scale 1.0.
    pub loudness: f32,
    /// How many 10 ms frames had a pitch. This is the weight when profiles are merged.
    pub voiced_frames: u32,
}

fn decimate(samples: &[f32], rate: u32) -> (Vec<f32>, f32) {
    let factor = ((rate as f32 / MAX_ANALYSIS_RATE).floor() as usize).max(1);
    let clean = |v: f32| if v.is_finite() { v } else { 0.0 };
    if factor == 1 {
        return (samples.iter().map(|&v| clean(v)).collect(), rate as f32);
    }
    let out = samples.chunks_exact(factor).map(|c| c.iter().map(|&v| clean(v)).sum::<f32>() / factor as f32).collect();
    (out, rate as f32 / factor as f32)
}

fn rms(x: &[f32]) -> f32 {
    if x.is_empty() {
        return 0.0;
    }
    (x.iter().map(|v| v * v).sum::<f32>() / x.len() as f32).sqrt()
}

/// The pitch of one frame in Hz, or `None` when the frame is not voiced.
fn frame_f0(frame: &[f32], rate: f32) -> Option<f32> {
    let min_lag = (rate / F0_RANGE.1).ceil() as usize;
    let max_lag = (rate / F0_RANGE.0).floor() as usize;
    if min_lag < 2 || max_lag + 2 >= frame.len() / 2 {
        return None;
    }
    let corr = |lag: usize| -> f32 {
        let (a, b) = (&frame[..frame.len() - lag], &frame[lag..]);
        let (mut ab, mut aa, mut bb) = (0.0f32, 0.0f32, 0.0f32);
        for (x, y) in a.iter().zip(b) {
            ab += x * y;
            aa += x * x;
            bb += y * y;
        }
        let norm = (aa * bb).sqrt();
        if norm <= f32::EPSILON {
            0.0
        } else {
            ab / norm
        }
    };
    let c: Vec<f32> = (min_lag - 1..=max_lag + 1).map(corr).collect();
    let at = |lag: usize| c[lag - (min_lag - 1)];
    let (mut best_lag, mut best) = (min_lag, f32::MIN);
    for lag in min_lag..=max_lag {
        if at(lag) > best {
            best = at(lag);
            best_lag = lag;
        }
    }
    if best < VOICED_CORRELATION {
        return None;
    }
    let mut lag = best_lag;
    for candidate in min_lag..=max_lag {
        if at(candidate) >= 0.9 * best && at(candidate) >= at(candidate - 1) && at(candidate) >= at(candidate + 1) {
            lag = candidate;
            break;
        }
    }
    let (l, m, r) = (at(lag - 1), at(lag), at(lag + 1));
    let denom = l - 2.0 * m + r;
    let shift = if denom.abs() > f32::EPSILON { (0.5 * (l - r) / denom).clamp(-0.5, 0.5) } else { 0.0 };
    Some(rate / (lag as f32 + shift))
}

/// Counts the syllable-like peaks of an energy envelope: local maxima at least `ACTIVE`-ish loud, 100 ms apart, with a
/// dip below 80% of the lower neighbour peak between them. Of two peaks too close or too shallow, the higher one stays.
fn count_peaks(env: &[f32], max: f32) -> usize {
    let smooth: Vec<f32> = (0..env.len())
        .map(|i| {
            let lo = i.saturating_sub(1);
            let hi = (i + 1).min(env.len() - 1);
            env[lo..=hi].iter().sum::<f32>() / (hi - lo + 1) as f32
        })
        .collect();
    let mut peaks = 0usize;
    let mut last: Option<(usize, f32)> = None;
    let mut valley = f32::MAX;
    for i in 1..smooth.len().saturating_sub(1) {
        valley = valley.min(smooth[i]);
        let is_peak = smooth[i] > smooth[i - 1] && smooth[i] >= smooth[i + 1] && smooth[i] >= 0.3 * max;
        if !is_peak {
            continue;
        }
        match last {
            None => {
                peaks = 1;
                last = Some((i, smooth[i]));
                valley = smooth[i];
            }
            Some((at, value)) => {
                let close = i - at < MIN_PEAK_GAP_FRAMES;
                let shallow = valley > 0.8 * value.min(smooth[i]);
                if close || shallow {
                    if smooth[i] > value {
                        last = Some((i, smooth[i]));
                        valley = smooth[i];
                    }
                } else {
                    peaks += 1;
                    last = Some((i, smooth[i]));
                    valley = smooth[i];
                }
            }
        }
    }
    peaks
}

/// Measures one recorded sample (mono, `-1.0..=1.0`) at `sample_rate`.
pub fn analyze(samples: &[f32], sample_rate: u32) -> Result<VoiceProfile, ProfileError> {
    if !(RATE_RANGE.0..=RATE_RANGE.1).contains(&sample_rate) {
        return Err(ProfileError::BadRate);
    }
    let (x, rate) = decimate(samples, sample_rate);
    let hop = ((rate * HOP_SECONDS) as usize).max(1);
    let win = (rate * FRAME_SECONDS) as usize;
    let env_win = (rate * ENVELOPE_SECONDS) as usize;
    if x.len() < win + hop * 2 {
        return Err(ProfileError::TooShort);
    }
    let frames = (x.len() - win) / hop + 1;
    let env: Vec<f32> = (0..frames).map(|i| rms(&x[i * hop..i * hop + env_win])).collect();
    let max = env.iter().copied().fold(0.0f32, f32::max);
    if max < MIN_PEAK_RMS {
        return Err(ProfileError::NoSpeech);
    }
    let active: Vec<usize> = (0..frames).filter(|&i| env[i] >= ACTIVE_RATIO * max).collect();
    let mut f0s: Vec<f32> = active.iter().filter_map(|&i| frame_f0(&x[i * hop..i * hop + win], rate)).collect();
    if f0s.len() < MIN_VOICED_FRAMES {
        return Err(ProfileError::NoVoicedSpeech);
    }
    f0s.sort_by(|a, b| a.total_cmp(b));
    let percentile = |p: f32| f0s[((f0s.len() - 1) as f32 * p).round() as usize];
    let mean_f0_hz = f0s.iter().sum::<f32>() / f0s.len() as f32;
    let loudness = active.iter().map(|&i| env[i]).sum::<f32>() / active.len() as f32;
    let (first, last) = (active[0], active[active.len() - 1]);
    let span_seconds = ((last - first + 1) as f32 * HOP_SECONDS).max(0.1);
    let peaks = count_peaks(&env[first..=last], max);
    Ok(VoiceProfile {
        mean_f0_hz,
        low_f0_hz: percentile(0.05),
        high_f0_hz: percentile(0.95),
        syllables_per_second: peaks as f32 / span_seconds,
        loudness,
        voiced_frames: f0s.len() as u32,
    })
}

impl VoiceProfile {
    /// One profile for a whole bank: the means weighted by each sample's voiced frames, the widest pitch range.
    /// `None` for an empty list or when no profile has any voiced frame.
    pub fn merge(profiles: &[VoiceProfile]) -> Option<VoiceProfile> {
        let total: f32 = profiles.iter().map(|p| p.voiced_frames as f32).sum();
        if profiles.is_empty() || total <= 0.0 {
            return None;
        }
        let weighted = |f: fn(&VoiceProfile) -> f32| profiles.iter().map(|p| f(p) * p.voiced_frames as f32).sum::<f32>() / total;
        Some(VoiceProfile {
            mean_f0_hz: weighted(|p| p.mean_f0_hz),
            low_f0_hz: profiles.iter().map(|p| p.low_f0_hz).fold(f32::MAX, f32::min),
            high_f0_hz: profiles.iter().map(|p| p.high_f0_hz).fold(0.0, f32::max),
            syllables_per_second: weighted(|p| p.syllables_per_second),
            loudness: weighted(|p| p.loudness),
            voiced_frames: profiles.iter().map(|p| p.voiced_frames).sum(),
        })
    }

    /// The synthesiser pitch multiplier that moves the voice from `reference_f0_hz` to this speaker's mean pitch, kept
    /// inside the synthesiser's range.
    pub fn pitch_multiplier(&self, reference_f0_hz: f32) -> f32 {
        if reference_f0_hz <= 0.0 || !reference_f0_hz.is_finite() {
            return 1.0;
        }
        (self.mean_f0_hz / reference_f0_hz).clamp(PITCH_MIN, PITCH_MAX)
    }

    /// The synthesiser rate (0 slowest, 1 fastest) for this speaker's pace.
    pub fn rate_setting(&self) -> f32 {
        ((self.syllables_per_second - RATE_SLOW_SPS) / (RATE_FAST_SPS - RATE_SLOW_SPS)).clamp(0.0, 1.0)
    }

    /// `utterance` delivered at this speaker's pitch and pace. Text, language, volume and delays are unchanged.
    pub fn apply(&self, utterance: Utterance) -> Utterance {
        utterance.with_pitch(self.pitch_multiplier(REFERENCE_F0_HZ)).with_rate(self.rate_setting())
    }

    /// The text form kept with the bank: `mean:low:high:syllables:loudness:voiced`.
    pub fn encode(&self) -> String {
        format!(
            "{:.2}:{:.2}:{:.2}:{:.3}:{:.5}:{}",
            self.mean_f0_hz, self.low_f0_hz, self.high_f0_hz, self.syllables_per_second, self.loudness, self.voiced_frames
        )
    }

    /// Reads [`encode`](Self::encode). Anything malformed, non-finite or out of order (low above mean or mean above
    /// high) is refused.
    pub fn parse(text: &str) -> Result<VoiceProfile, ProfileError> {
        let parts: Vec<&str> = text.trim().split(':').collect();
        let [mean, low, high, sps, loud, voiced] = parts.as_slice() else { return Err(ProfileError::Parse) };
        let num = |s: &str| s.parse::<f32>().ok().filter(|v| v.is_finite() && *v >= 0.0).ok_or(ProfileError::Parse);
        let p = VoiceProfile {
            mean_f0_hz: num(mean)?,
            low_f0_hz: num(low)?,
            high_f0_hz: num(high)?,
            syllables_per_second: num(sps)?,
            loudness: num(loud)?,
            voiced_frames: voiced.parse().map_err(|_| ProfileError::Parse)?,
        };
        if p.low_f0_hz > p.mean_f0_hz || p.mean_f0_hz > p.high_f0_hz || p.mean_f0_hz <= 0.0 {
            return Err(ProfileError::Parse);
        }
        Ok(p)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A harmonic tone at `f0` whose loudness rises and falls `beat` times a second, like syllables.
    fn speechlike(f0: f32, rate: u32, seconds: f32, beat: f32) -> Vec<f32> {
        let tau = std::f32::consts::TAU;
        (0..(rate as f32 * seconds) as usize)
            .map(|n| {
                let t = n as f32 / rate as f32;
                let env = 0.5 - 0.5 * (tau * beat * t).cos();
                0.4 * env * ((tau * f0 * t).sin() + 0.5 * (2.0 * tau * f0 * t).sin() + 0.25 * (3.0 * tau * f0 * t).sin())
            })
            .collect()
    }

    fn near(value: f32, want: f32, fraction: f32) -> bool {
        (value - want).abs() <= want * fraction
    }

    #[test]
    fn the_pitch_of_a_low_and_a_high_voice_is_found() {
        let low = analyze(&speechlike(120.0, 16_000, 2.0, 4.0), 16_000).unwrap();
        assert!(near(low.mean_f0_hz, 120.0, 0.03), "got {}", low.mean_f0_hz);
        let high = analyze(&speechlike(220.0, 16_000, 2.0, 4.0), 16_000).unwrap();
        assert!(near(high.mean_f0_hz, 220.0, 0.03), "got {}", high.mean_f0_hz);
        assert!(high.mean_f0_hz > low.mean_f0_hz);
    }

    #[test]
    fn a_high_sample_rate_is_decimated_and_still_measured() {
        let p = analyze(&speechlike(150.0, 44_100, 2.0, 4.0), 44_100).unwrap();
        assert!(near(p.mean_f0_hz, 150.0, 0.03), "got {}", p.mean_f0_hz);
    }

    #[test]
    fn the_syllable_rate_follows_the_beat() {
        let slow = analyze(&speechlike(130.0, 16_000, 3.0, 3.0), 16_000).unwrap();
        let fast = analyze(&speechlike(130.0, 16_000, 3.0, 6.0), 16_000).unwrap();
        assert!(near(slow.syllables_per_second, 3.0, 0.25), "slow {}", slow.syllables_per_second);
        assert!(near(fast.syllables_per_second, 6.0, 0.25), "fast {}", fast.syllables_per_second);
        assert!(fast.rate_setting() > slow.rate_setting());
    }

    #[test]
    fn a_louder_recording_reports_more_loudness() {
        let quiet: Vec<f32> = speechlike(130.0, 16_000, 2.0, 4.0).iter().map(|v| v * 0.25).collect();
        let loud = speechlike(130.0, 16_000, 2.0, 4.0);
        assert!(analyze(&loud, 16_000).unwrap().loudness > 3.0 * analyze(&quiet, 16_000).unwrap().loudness);
    }

    #[test]
    fn silence_noise_and_clips_are_refused_with_the_reason() {
        assert_eq!(analyze(&vec![0.0; 16_000], 16_000), Err(ProfileError::NoSpeech));
        let mut state = 12345u32;
        let noise: Vec<f32> = (0..32_000)
            .map(|_| {
                state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                ((state >> 8) as f32 / (1u32 << 24) as f32 - 0.5)
            })
            .collect();
        assert_eq!(analyze(&noise, 16_000), Err(ProfileError::NoVoicedSpeech));
        assert_eq!(analyze(&[0.1; 100], 16_000), Err(ProfileError::TooShort));
        assert_eq!(analyze(&speechlike(120.0, 16_000, 1.0, 4.0), 4_000), Err(ProfileError::BadRate));
    }

    #[test]
    fn non_finite_samples_count_as_silence() {
        let mut s = speechlike(120.0, 16_000, 2.0, 4.0);
        s[100] = f32::NAN;
        s[200] = f32::INFINITY;
        assert!(analyze(&s, 16_000).is_ok());
    }

    #[test]
    fn the_pitch_range_brackets_the_mean() {
        let p = analyze(&speechlike(140.0, 16_000, 2.0, 4.0), 16_000).unwrap();
        assert!(p.low_f0_hz <= p.mean_f0_hz && p.mean_f0_hz <= p.high_f0_hz);
        assert!(p.voiced_frames as usize >= MIN_VOICED_FRAMES);
    }

    fn profile(mean: f32, sps: f32, voiced: u32) -> VoiceProfile {
        VoiceProfile { mean_f0_hz: mean, low_f0_hz: mean - 10.0, high_f0_hz: mean + 10.0, syllables_per_second: sps, loudness: 0.1, voiced_frames: voiced }
    }

    #[test]
    fn merging_weights_by_voiced_frames_and_keeps_the_widest_range() {
        let merged = VoiceProfile::merge(&[profile(100.0, 3.0, 100), profile(200.0, 5.0, 300)]).unwrap();
        assert!(near(merged.mean_f0_hz, 175.0, 0.001));
        assert!(near(merged.syllables_per_second, 4.5, 0.001));
        assert_eq!((merged.low_f0_hz, merged.high_f0_hz), (90.0, 210.0));
        assert_eq!(merged.voiced_frames, 400);
        assert_eq!(VoiceProfile::merge(&[]), None);
        assert_eq!(VoiceProfile::merge(&[profile(100.0, 3.0, 0)]), None);
    }

    #[test]
    fn the_profile_sets_the_synthesiser_pitch_and_rate_inside_its_ranges() {
        assert!(near(profile(REFERENCE_F0_HZ, 4.5, 10).pitch_multiplier(REFERENCE_F0_HZ), 1.0, 0.001));
        assert_eq!(profile(REFERENCE_F0_HZ * 5.0, 4.5, 10).pitch_multiplier(REFERENCE_F0_HZ), PITCH_MAX);
        assert_eq!(profile(REFERENCE_F0_HZ / 5.0, 4.5, 10).pitch_multiplier(REFERENCE_F0_HZ), PITCH_MIN);
        assert_eq!(profile(100.0, 4.0, 10).pitch_multiplier(0.0), 1.0);
        assert_eq!(profile(100.0, 1.0, 10).rate_setting(), 0.0);
        assert_eq!(profile(100.0, 9.0, 10).rate_setting(), 1.0);
        assert!(near(profile(100.0, 4.5, 10).rate_setting(), 0.5, 0.001));
    }

    #[test]
    fn apply_changes_only_pitch_and_rate() {
        let base = Utterance::new("hello").with_language("en-GB").with_volume(0.4).with_pre_delay(0.2);
        let spoken = profile(210.0, 6.0, 10).apply(base.clone());
        assert!(near(spoken.pitch(), 1.5, 0.001));
        assert!(near(spoken.rate(), 0.8, 0.001));
        assert_eq!((spoken.text.as_str(), spoken.language.clone(), spoken.volume(), spoken.pre_delay()), ("hello", Some("en-GB".to_string()), 0.4, 0.2));
    }

    #[test]
    fn the_text_form_round_trips_and_refuses_damage() {
        let p = profile(133.5, 4.25, 321);
        let back = VoiceProfile::parse(&p.encode()).unwrap();
        assert!(near(back.mean_f0_hz, 133.5, 0.001) && near(back.syllables_per_second, 4.25, 0.001));
        assert_eq!(back.voiced_frames, 321);
        for bad in ["", "1:2:3", "a:b:c:d:e:f", "100:200:150:4:0.1:5", "100:90:110:NaN:0.1:5", "0:0:0:4:0.1:5", "100:90:110:-1:0.1:5"] {
            assert_eq!(VoiceProfile::parse(bad), Err(ProfileError::Parse), "{bad:?}");
        }
    }
}
