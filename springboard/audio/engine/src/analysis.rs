//! Audio analysis: level and spectrum measurements of a block of audio, and a hub that publishes the latest
//! measurement of each tap to the clients that subscribed (a level meter in a mixer, a spectrum in a recorder).
//!
//! A measurement is the peak, the RMS level and eight octave bands from 63 Hz to 8 kHz. Each band is the amplitude
//! of a sinusoid at the band's centre, found with a Hann-windowed Goertzel filter, so a full-scale sine at a band
//! centre reads 1.0 there and close to 0 in the other bands. Bands at or above the Nyquist frequency read 0. The
//! measurements are plain data with a one-line text form, so a service can send them over any socket.
//!
//! Pure and allocation-light: nothing here touches a device, and `analyze` is meant for the control side (a tap
//! ring read by a non-real-time thread), not for the render thread.

use std::collections::BTreeMap;
use std::f64::consts::PI;

/// The centre frequency of each band, in hertz.
pub const BANDS_HZ: [f32; 8] = [63.0, 125.0, 250.0, 500.0, 1000.0, 2000.0, 4000.0, 8000.0];

/// One analysis of a block.
#[derive(Clone, Debug, PartialEq)]
pub struct Measurement {
    /// Which publication of its tap this is, from 1. Zero until the hub numbers it.
    pub seq: u64,
    pub rate: u32,
    pub frames: u32,
    /// Largest absolute sample over all channels.
    pub peak: f32,
    /// Root mean square of the channel average.
    pub rms: f32,
    /// Amplitude at each of [`BANDS_HZ`].
    pub bands: [f32; 8],
}

impl Measurement {
    fn empty(rate: u32) -> Measurement {
        Measurement { seq: 0, rate, frames: 0, peak: 0.0, rms: 0.0, bands: [0.0; 8] }
    }

    /// The text form: `m1 <seq> <rate> <frames> <peak> <rms> <band0> ... <band7>`.
    pub fn encode(&self) -> String {
        let mut s = format!("m1 {} {} {} {} {}", self.seq, self.rate, self.frames, self.peak, self.rms);
        for b in &self.bands {
            s.push_str(&format!(" {b}"));
        }
        s
    }

    /// Reads what `encode` wrote; anything else is `None`.
    pub fn decode(text: &str) -> Option<Measurement> {
        let mut f = text.split_whitespace();
        if f.next()? != "m1" {
            return None;
        }
        let seq = f.next()?.parse().ok()?;
        let rate = f.next()?.parse().ok()?;
        let frames = f.next()?.parse().ok()?;
        let peak: f32 = f.next()?.parse().ok()?;
        let rms: f32 = f.next()?.parse().ok()?;
        let mut bands = [0.0f32; 8];
        for b in &mut bands {
            *b = f.next()?.parse().ok()?;
        }
        if f.next().is_some() || !peak.is_finite() || !rms.is_finite() || bands.iter().any(|b| !b.is_finite()) {
            return None;
        }
        Some(Measurement { seq, rate, frames, peak, rms, bands })
    }
}

/// A linear level in decibels full scale; silence is -180.
pub fn to_db(level: f32) -> f32 {
    20.0 * level.max(1e-9).log10()
}

fn clean(x: f32) -> f32 {
    if x.is_finite() {
        x
    } else {
        0.0
    }
}

fn band_level(x: &[f32], rate: f64, freq: f64) -> f32 {
    let n = x.len();
    if n < 2 || freq >= rate / 2.0 {
        return 0.0;
    }
    let coeff = 2.0 * (2.0 * PI * freq / rate).cos();
    let (mut s1, mut s2) = (0.0f64, 0.0f64);
    for (i, &v) in x.iter().enumerate() {
        let window = 0.5 - 0.5 * (2.0 * PI * i as f64 / n as f64).cos();
        let s = v as f64 * window + coeff * s1 - s2;
        s2 = s1;
        s1 = s;
    }
    let power = (s1 * s1 + s2 * s2 - coeff * s1 * s2).max(0.0);
    // a sinusoid of amplitude A at the filter frequency gives |X| = A * n / 4 under a Hann window
    (4.0 * power.sqrt() / n as f64) as f32
}

/// Analyses one channel of samples at `rate` hertz. Samples that are not finite count as silence.
pub fn analyze(samples: &[f32], rate: u32) -> Measurement {
    if samples.is_empty() || rate == 0 {
        return Measurement::empty(rate);
    }
    let x: Vec<f32> = samples.iter().map(|v| clean(*v)).collect();
    let peak = x.iter().fold(0.0f32, |m, v| m.max(v.abs()));
    let rms = (x.iter().map(|v| (*v as f64) * (*v as f64)).sum::<f64>() / x.len() as f64).sqrt() as f32;
    let mut bands = [0.0f32; 8];
    for (b, f) in bands.iter_mut().zip(BANDS_HZ) {
        *b = band_level(&x, rate as f64, f as f64);
    }
    Measurement { seq: 0, rate, frames: x.len() as u32, peak, rms, bands }
}

/// Analyses planar channels: the peak is over every channel, the RMS and the bands are of the channel average. The
/// block is as long as the shortest channel.
pub fn analyze_planar(channels: &[&[f32]], rate: u32) -> Measurement {
    let frames = channels.iter().map(|c| c.len()).min().unwrap_or(0);
    if frames == 0 || rate == 0 {
        return Measurement::empty(rate);
    }
    let mut mono = vec![0.0f32; frames];
    let mut peak = 0.0f32;
    for c in channels {
        for (i, &v) in c[..frames].iter().enumerate() {
            let v = clean(v);
            peak = peak.max(v.abs());
            mono[i] += v;
        }
    }
    let n = channels.len() as f32;
    for m in &mut mono {
        *m /= n;
    }
    let mut m = analyze(&mono, rate);
    m.peak = peak;
    m
}

#[derive(Debug, Default)]
struct TapState {
    seq: u64,
    latest: Option<Measurement>,
}

/// Publishes the newest measurement of each named tap to its subscribers. A subscriber that polls slower than
/// the taps publish gets the newest measurement and skips the ones in between.
#[derive(Debug, Default)]
pub struct AnalysisHub {
    taps: BTreeMap<String, TapState>,
    /// subscriber -> (tap, last seq it was given)
    subscribers: BTreeMap<u32, (String, u64)>,
    next_subscriber: u32,
}

impl AnalysisHub {
    pub fn new() -> AnalysisHub {
        AnalysisHub { next_subscriber: 1, ..Default::default() }
    }

    /// Analyses a block of `tap` and publishes it. Returns its sequence number.
    pub fn publish(&mut self, tap: &str, channels: &[&[f32]], rate: u32) -> u64 {
        let state = self.taps.entry(tap.to_string()).or_default();
        state.seq += 1;
        let mut m = analyze_planar(channels, rate);
        m.seq = state.seq;
        state.latest = Some(m);
        state.seq
    }

    pub fn latest(&self, tap: &str) -> Option<&Measurement> {
        self.taps.get(tap).and_then(|t| t.latest.as_ref())
    }

    /// The tap names that have published, sorted.
    pub fn taps(&self) -> Vec<&str> {
        self.taps.keys().map(String::as_str).collect()
    }

    /// Subscribes to a tap (it need not have published yet). Returns the subscription id.
    pub fn subscribe(&mut self, tap: &str) -> u32 {
        if self.next_subscriber == 0 {
            self.next_subscriber = 1;
        }
        let id = self.next_subscriber;
        self.next_subscriber += 1;
        self.subscribers.insert(id, (tap.to_string(), 0));
        id
    }

    pub fn unsubscribe(&mut self, id: u32) -> bool {
        self.subscribers.remove(&id).is_some()
    }

    /// The newest measurement of the subscribed tap that this subscriber has not been given yet.
    pub fn poll(&mut self, id: u32) -> Option<Measurement> {
        let (tap, seen) = self.subscribers.get_mut(&id)?;
        let m = self.taps.get(tap.as_str())?.latest.as_ref()?;
        if m.seq <= *seen {
            return None;
        }
        *seen = m.seq;
        Some(m.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(freq: f64, amp: f32, rate: u32, n: usize) -> Vec<f32> {
        (0..n).map(|i| amp * (2.0 * PI * freq * i as f64 / rate as f64).sin() as f32).collect()
    }

    #[test]
    fn a_sine_reads_its_amplitude_in_its_own_band_only() {
        let x = sine(1000.0, 0.5, 48_000, 4800);
        let m = analyze(&x, 48_000);
        assert!((m.peak - 0.5).abs() < 0.01);
        assert!((m.rms - 0.5 / 2f32.sqrt()).abs() < 0.005);
        assert!((m.bands[4] - 0.5).abs() < 0.01, "1 kHz band: {}", m.bands[4]);
        for (i, b) in m.bands.iter().enumerate() {
            if i != 4 {
                assert!(*b < 0.005, "band {i} leaks {b}");
            }
        }
        assert_eq!((m.rate, m.frames), (48_000, 4800));
    }

    #[test]
    fn silence_and_empty_blocks_measure_zero() {
        let m = analyze(&[0.0; 1024], 48_000);
        assert_eq!((m.peak, m.rms, m.bands), (0.0, 0.0, [0.0; 8]));
        assert_eq!(analyze(&[], 48_000).frames, 0);
        assert_eq!(analyze(&[1.0, 1.0], 0).frames, 0);
        assert_eq!(analyze_planar(&[], 48_000).frames, 0);
        assert_eq!(to_db(0.0), -180.0);
        assert!((to_db(1.0)).abs() < 1e-6);
        assert!((to_db(0.5) + 6.0206).abs() < 0.001);
    }

    #[test]
    fn channels_are_averaged_for_the_bands_and_the_peak_is_over_all_of_them() {
        let l = sine(1000.0, 0.5, 48_000, 4800);
        let r = vec![0.0f32; 4800];
        let m = analyze_planar(&[&l, &r], 48_000);
        assert!((m.peak - 0.5).abs() < 0.01);
        assert!((m.bands[4] - 0.25).abs() < 0.01, "half the amplitude after averaging: {}", m.bands[4]);
        // the shortest channel sets the length
        let short = analyze_planar(&[&l, &r[..100]], 48_000);
        assert_eq!(short.frames, 100);
    }

    #[test]
    fn bands_at_or_above_the_nyquist_frequency_read_zero_and_bad_samples_are_silence() {
        let x = sine(500.0, 0.8, 8_000, 4000);
        let m = analyze(&x, 8_000);
        assert!((m.bands[3] - 0.8).abs() < 0.02);
        assert_eq!((m.bands[6], m.bands[7]), (0.0, 0.0));
        let bad = analyze(&[f32::NAN, f32::INFINITY, 0.5, -0.5], 48_000);
        assert!(bad.peak.is_finite() && bad.rms.is_finite() && bad.bands.iter().all(|b| b.is_finite()));
        assert_eq!(bad.peak, 0.5);
    }

    #[test]
    fn the_text_form_round_trips_and_refuses_garbage() {
        let mut m = analyze(&sine(250.0, 0.3, 48_000, 4800), 48_000);
        m.seq = 7;
        assert_eq!(Measurement::decode(&m.encode()), Some(m.clone()));
        assert_eq!(Measurement::decode(""), None);
        assert_eq!(Measurement::decode("m2 1 2 3 4 5 1 2 3 4 5 6 7 8"), None);
        assert_eq!(Measurement::decode("m1 1 48000 10 0.5 0.1 0 0 0 0 0 0 0"), None, "seven bands");
        assert_eq!(Measurement::decode(&format!("{} extra", m.encode())), None);
        assert_eq!(Measurement::decode("m1 1 48000 10 NaN 0.1 0 0 0 0 0 0 0 0"), None);
    }

    #[test]
    fn subscribers_get_the_newest_measurement_once() {
        let mut hub = AnalysisHub::new();
        let block = sine(1000.0, 0.5, 48_000, 480);
        let a = hub.subscribe("master");
        assert!(hub.poll(a).is_none(), "nothing published yet");
        assert_eq!(hub.publish("master", &[&block], 48_000), 1);
        assert_eq!(hub.publish("master", &[&block], 48_000), 2);
        let m = hub.poll(a).unwrap();
        assert_eq!(m.seq, 2, "the first measurement was skipped");
        assert!(hub.poll(a).is_none());
        hub.publish("master", &[&block], 48_000);
        assert_eq!(hub.poll(a).unwrap().seq, 3);
        // a late subscriber starts from the latest
        let b = hub.subscribe("master");
        assert_eq!(hub.poll(b).unwrap().seq, 3);
        // taps are separate
        let c = hub.subscribe("mic");
        assert!(hub.poll(c).is_none());
        hub.publish("mic", &[&block], 48_000);
        assert_eq!(hub.poll(c).unwrap().seq, 1);
        assert_eq!(hub.taps(), vec!["master", "mic"]);
        assert!(hub.unsubscribe(a));
        assert!(!hub.unsubscribe(a));
        assert!(hub.poll(a).is_none());
        assert_eq!(hub.latest("master").unwrap().seq, 3);
    }
}
