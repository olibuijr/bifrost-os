//! Automix: plan a beat-matched transition from one song into the next (std only, pure).
//!
//! The planner reads each song's beat grid (`crate::music::RhythmResult`: beats, bar starts and tempo, from
//! `analyze_rhythm`) and its length, and answers where the outgoing song starts to fade, where the incoming one must
//! start so its first bar line lands on that bar line of the outgoing song, how long the overlap lasts (a whole number
//! of bars), and how fast the incoming song plays so the two tempos agree and then relax back to its own.
//!
//! `Transition::frame` gives the mixer settings at any point of the overlap: the two gains (equal power, so the level
//! holds), a low cut on each song (the bass swap: the outgoing bass leaves before the incoming bass arrives, so the two
//! kick drums never pile up) and the incoming playback rate. These are the parameters a two-channel gain, high-pass
//! filter and varispeed stage take; the audio service applies them. When the songs cannot be matched (no beat grid,
//! tempos too far apart) the plan is a plain equal-power crossfade, which is also what `music::playback` offers.
//! The plan is data and reads no audio, so the tests cover it without a device.

use crate::music::RhythmResult;
use std::f64::consts::FRAC_PI_2;

/// The largest tempo change, as a fraction, the incoming song is stretched by to match the outgoing one (8 %).
pub const MAX_STRETCH: f64 = 0.08;
/// The overlaps tried, in bars, longest first.
const BAR_CHOICES: [u32; 5] = [16, 8, 4, 2, 1];
/// Beats in a bar (`RhythmResult::bars` counts every fourth beat).
const BEATS_PER_BAR: f64 = 4.0;
/// The low cut of a song that is not being cut, in hertz.
pub const CUT_OPEN_HZ: f64 = 20.0;
/// The low cut of a song whose bass has left, in hertz.
pub const CUT_CLOSED_HZ: f64 = 400.0;

/// What the planner knows about one song.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Song<'a> {
    /// Length in seconds.
    pub duration: f64,
    pub rhythm: &'a RhythmResult,
}

/// How the overlap is mixed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Style {
    /// Gains only: the songs are not beat matched.
    Crossfade,
    /// Beat matched, with the bass swap.
    BassSwap,
}

/// Why no plan was made.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlanError {
    /// A length is zero, negative or not a number, or the overlap limit is not positive.
    BadInput,
}

/// A planned transition. Times are seconds; `*_start` of the outgoing song are positions in that song.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Transition {
    pub style: Style,
    /// Where in the outgoing song the fade begins (a bar line when the songs are matched).
    pub out_start: f64,
    /// How long both songs sound.
    pub overlap: f64,
    /// Where in the incoming song playback begins.
    pub in_seek: f64,
    /// When, on the outgoing song's timeline, the incoming song starts playing. Its first bar line then meets `out_start`.
    pub in_start_at: f64,
    /// The incoming song's playback rate at the start of the overlap; it eases to 1 by the end.
    pub in_rate: f64,
}

/// The mixer settings at one point of the overlap.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Frame {
    pub out_gain: f64,
    pub in_gain: f64,
    /// Low-cut corner of the outgoing song, in hertz.
    pub out_low_cut_hz: f64,
    /// Low-cut corner of the incoming song, in hertz.
    pub in_low_cut_hz: f64,
    pub in_rate: f64,
}

/// The tempo ratio that makes `incoming` follow `outgoing`, folding half and double time (a 70 and a 140 tempo are
/// the same pulse), or None when it needs a stretch beyond `MAX_STRETCH`.
fn tempo_ratio(outgoing: f64, incoming: f64) -> Option<f64> {
    if !(outgoing > 0.0 && incoming > 0.0) {
        return None;
    }
    let mut ratio = outgoing / incoming;
    while ratio >= 1.5 {
        ratio /= 2.0;
    }
    while ratio < 0.75 {
        ratio *= 2.0;
    }
    ((ratio - 1.0).abs() <= MAX_STRETCH).then_some(ratio)
}

/// The latest bar line `b` with `b + overlap <= duration`.
fn last_bar_fitting(bars: &[f64], overlap: f64, duration: f64) -> Option<f64> {
    bars.iter().copied().filter(|b| b.is_finite() && *b >= 0.0 && b + overlap <= duration).fold(None, |best, b| match best {
        Some(x) if x >= b => Some(x),
        _ => Some(b),
    })
}

fn valid(song: &Song<'_>) -> bool {
    song.duration.is_finite() && song.duration > 0.0
}

/// Plans the transition from `outgoing` into `incoming`, overlapping at most `max_overlap` seconds.
pub fn plan(outgoing: &Song<'_>, incoming: &Song<'_>, max_overlap: f64) -> Result<Transition, PlanError> {
    if !valid(outgoing) || !valid(incoming) || !(max_overlap.is_finite() && max_overlap > 0.0) {
        return Err(PlanError::BadInput);
    }
    // The longest overlap that neither song runs out of: not more than half the incoming song.
    let room = max_overlap.min(outgoing.duration).min(incoming.duration / 2.0);

    let matched = tempo_ratio(outgoing.rhythm.beats_per_minute, incoming.rhythm.beats_per_minute)
        .filter(|_| !outgoing.rhythm.bars.is_empty() && !incoming.rhythm.bars.is_empty());
    let Some(ratio) = matched else {
        return Ok(Transition { style: Style::Crossfade, out_start: outgoing.duration - room, overlap: room, in_seek: 0.0, in_start_at: outgoing.duration - room, in_rate: 1.0 });
    };

    // Bars are counted in the outgoing song's own tempo; the incoming song is stretched to it.
    let bar_secs = 60.0 / outgoing.rhythm.beats_per_minute * BEATS_PER_BAR;
    for bars in BAR_CHOICES {
        let overlap = f64::from(bars) * bar_secs;
        if overlap > room {
            continue;
        }
        let Some(out_start) = last_bar_fitting(&outgoing.rhythm.bars, overlap, outgoing.duration) else { continue };
        // The incoming song's first bar line meets `out_start`; its intro before that plays at the stretched rate.
        let first_bar = incoming.rhythm.bars[0].max(0.0);
        let lead = first_bar / ratio;
        let (in_start_at, in_seek) = if lead <= out_start { (out_start - lead, 0.0) } else { (0.0, (lead - out_start) * ratio) };
        return Ok(Transition { style: Style::BassSwap, out_start, overlap, in_seek, in_start_at, in_rate: ratio });
    }
    // Matched songs but no bar line leaves room: fall back to a plain crossfade at the end.
    Ok(Transition { style: Style::Crossfade, out_start: outgoing.duration - room, overlap: room, in_seek: 0.0, in_start_at: outgoing.duration - room, in_rate: 1.0 })
}

/// 0 to 1 over `x` from `a` to `b`, clamped.
fn ramp(x: f64, a: f64, b: f64) -> f64 {
    if b <= a {
        return if x >= b { 1.0 } else { 0.0 };
    }
    ((x - a) / (b - a)).clamp(0.0, 1.0)
}

impl Transition {
    /// The mixer settings `progress` of the way through the overlap (0 at its start, 1 at its end; clamped).
    pub fn frame(&self, progress: f64) -> Frame {
        let p = if progress.is_finite() { progress.clamp(0.0, 1.0) } else { 0.0 };
        let angle = p * FRAC_PI_2;
        let (out_cut, in_cut) = match self.style {
            Style::Crossfade => (CUT_OPEN_HZ, CUT_OPEN_HZ),
            // the outgoing bass closes over the first half, the incoming bass opens over the second half
            Style::BassSwap => (CUT_OPEN_HZ + (CUT_CLOSED_HZ - CUT_OPEN_HZ) * ramp(p, 0.0, 0.5), CUT_CLOSED_HZ - (CUT_CLOSED_HZ - CUT_OPEN_HZ) * ramp(p, 0.5, 1.0)),
        };
        let eased = p * p * (3.0 - 2.0 * p);
        Frame { out_gain: angle.cos(), in_gain: angle.sin(), out_low_cut_hz: out_cut, in_low_cut_hz: in_cut, in_rate: self.in_rate + (1.0 - self.in_rate) * eased }
    }

    /// When the overlap ends, in the outgoing song.
    pub fn out_end(&self) -> f64 {
        self.out_start + self.overlap
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A steady grid: a beat every `60 / bpm` seconds from `first`, a bar every fourth beat, for `len` seconds.
    fn grid(bpm: f64, first: f64, len: f64) -> RhythmResult {
        let step = 60.0 / bpm;
        let mut beats = Vec::new();
        let mut t = first;
        while t < len {
            beats.push(t);
            t += step;
        }
        let bars = beats.iter().copied().step_by(4).collect();
        RhythmResult { beats, bars, beats_per_minute: bpm }
    }

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    #[test]
    fn matched_songs_overlap_on_bar_lines_for_whole_bars() {
        // 120 bpm: a bar is 2 s
        let (a, b) = (grid(120.0, 0.0, 200.0), grid(120.0, 0.0, 180.0));
        let out = Song { duration: 200.0, rhythm: &a };
        let inn = Song { duration: 180.0, rhythm: &b };
        let t = plan(&out, &inn, 20.0).unwrap();
        assert_eq!(t.style, Style::BassSwap);
        assert!(close(t.overlap, 16.0), "eight bars fit in 20 s: {}", t.overlap);
        assert!(close(t.out_start % 2.0, 0.0), "the fade starts on a bar line");
        assert!(t.out_end() <= 200.0 + 1e-9);
        assert!(close(t.out_start, 184.0), "the latest bar line that leaves 16 s: {}", t.out_start);
        assert!(close(t.in_rate, 1.0));
        assert!(close(t.in_start_at, t.out_start) && close(t.in_seek, 0.0));
    }

    #[test]
    fn the_incoming_song_is_stretched_to_the_outgoing_tempo_and_relaxes() {
        let (a, b) = (grid(124.0, 0.0, 200.0), grid(120.0, 0.0, 200.0));
        let t = plan(&Song { duration: 200.0, rhythm: &a }, &Song { duration: 200.0, rhythm: &b }, 30.0).unwrap();
        assert_eq!(t.style, Style::BassSwap);
        assert!(close(t.in_rate, 124.0 / 120.0));
        assert!(close(t.frame(0.0).in_rate, 124.0 / 120.0));
        assert!(close(t.frame(1.0).in_rate, 1.0));
        let mid = t.frame(0.5).in_rate;
        assert!(mid < 124.0 / 120.0 && mid > 1.0);
    }

    #[test]
    fn half_and_double_time_count_as_the_same_pulse() {
        let (a, b) = (grid(70.0, 0.0, 200.0), grid(140.0, 0.0, 200.0));
        let t = plan(&Song { duration: 200.0, rhythm: &a }, &Song { duration: 200.0, rhythm: &b }, 40.0).unwrap();
        assert_eq!(t.style, Style::BassSwap);
        assert!(close(t.in_rate, 1.0), "70 against 140 needs no stretch: {}", t.in_rate);
        let (c, d) = (grid(100.0, 0.0, 200.0), grid(130.0, 0.0, 200.0));
        let far = plan(&Song { duration: 200.0, rhythm: &c }, &Song { duration: 200.0, rhythm: &d }, 40.0).unwrap();
        assert_eq!(far.style, Style::Crossfade, "a 30 % stretch is too much");
        assert!(close(far.in_rate, 1.0));
    }

    #[test]
    fn a_song_without_a_beat_grid_gets_a_plain_crossfade_at_its_end() {
        let none = RhythmResult::default();
        let b = grid(120.0, 0.0, 100.0);
        let t = plan(&Song { duration: 90.0, rhythm: &none }, &Song { duration: 100.0, rhythm: &b }, 8.0).unwrap();
        assert_eq!(t.style, Style::Crossfade);
        assert!(close(t.overlap, 8.0) && close(t.out_start, 82.0) && close(t.in_start_at, 82.0));
        // tempo known but no bar lines
        let no_bars = RhythmResult { beats: vec![0.0, 0.5], bars: vec![], beats_per_minute: 120.0 };
        let t = plan(&Song { duration: 90.0, rhythm: &no_bars }, &Song { duration: 100.0, rhythm: &b }, 8.0).unwrap();
        assert_eq!(t.style, Style::Crossfade);
    }

    #[test]
    fn short_songs_limit_the_overlap() {
        let (a, b) = (grid(120.0, 0.0, 30.0), grid(120.0, 0.0, 10.0));
        let t = plan(&Song { duration: 30.0, rhythm: &a }, &Song { duration: 10.0, rhythm: &b }, 60.0).unwrap();
        assert!(t.overlap <= 5.0 + 1e-9, "at most half of the 10 s incoming song: {}", t.overlap);
        assert!(close(t.overlap, 4.0), "two bars of 2 s: {}", t.overlap);
        assert!(t.out_end() <= 30.0 + 1e-9);
    }

    #[test]
    fn an_intro_before_the_first_bar_line_starts_the_incoming_song_early() {
        let a = grid(120.0, 0.0, 100.0);
        let b = grid(120.0, 3.0, 100.0); // the first bar line is 3 s into the song
        let t = plan(&Song { duration: 100.0, rhythm: &a }, &Song { duration: 100.0, rhythm: &b }, 8.0).unwrap();
        assert_eq!(t.style, Style::BassSwap);
        assert!(close(t.in_start_at, t.out_start - 3.0), "the intro plays under the outgoing song");
        assert!(close(t.in_seek, 0.0));
        // when the outgoing fade would start sooner than the intro is long, the incoming song starts from inside it
        let early = RhythmResult { beats: vec![0.0, 0.5, 1.0, 1.5, 2.0], bars: vec![0.0, 2.0], beats_per_minute: 120.0 };
        let late_intro = grid(120.0, 6.0, 100.0);
        let t = plan(&Song { duration: 4.0, rhythm: &early }, &Song { duration: 100.0, rhythm: &late_intro }, 2.0).unwrap();
        assert_eq!(t.style, Style::BassSwap);
        assert!(close(t.in_start_at, 0.0));
        assert!(close(t.in_seek, 6.0 - t.out_start), "{} {}", t.in_seek, t.out_start);
    }

    #[test]
    fn the_gains_hold_the_level_and_the_bass_swaps() {
        let (a, b) = (grid(120.0, 0.0, 200.0), grid(120.0, 0.0, 200.0));
        let t = plan(&Song { duration: 200.0, rhythm: &a }, &Song { duration: 200.0, rhythm: &b }, 16.0).unwrap();
        for i in 0..=20 {
            let f = t.frame(f64::from(i) / 20.0);
            assert!(close(f.out_gain * f.out_gain + f.in_gain * f.in_gain, 1.0));
        }
        let start = t.frame(0.0);
        assert!(close(start.out_gain, 1.0) && close(start.in_gain, 0.0));
        assert!(close(start.out_low_cut_hz, CUT_OPEN_HZ) && close(start.in_low_cut_hz, CUT_CLOSED_HZ));
        let mid = t.frame(0.5);
        assert!(close(mid.out_low_cut_hz, CUT_CLOSED_HZ) && close(mid.in_low_cut_hz, CUT_CLOSED_HZ), "both bass lines are out at the swap");
        let end = t.frame(1.0);
        assert!(close(end.out_gain, 0.0) && close(end.in_gain, 1.0));
        assert!(close(end.out_low_cut_hz, CUT_CLOSED_HZ) && close(end.in_low_cut_hz, CUT_OPEN_HZ));
        // out of range and not-a-number progress are clamped
        assert_eq!(t.frame(-3.0), t.frame(0.0));
        assert_eq!(t.frame(9.0), t.frame(1.0));
        assert_eq!(t.frame(f64::NAN), t.frame(0.0));
    }

    #[test]
    fn a_plain_crossfade_leaves_the_low_cuts_open() {
        let none = RhythmResult::default();
        let t = plan(&Song { duration: 60.0, rhythm: &none }, &Song { duration: 60.0, rhythm: &none }, 6.0).unwrap();
        let f = t.frame(0.5);
        assert!(close(f.out_low_cut_hz, CUT_OPEN_HZ) && close(f.in_low_cut_hz, CUT_OPEN_HZ) && close(f.in_rate, 1.0));
    }

    #[test]
    fn bad_input_is_refused() {
        let a = grid(120.0, 0.0, 100.0);
        let ok = Song { duration: 100.0, rhythm: &a };
        assert_eq!(plan(&Song { duration: 0.0, rhythm: &a }, &ok, 8.0), Err(PlanError::BadInput));
        assert_eq!(plan(&ok, &Song { duration: f64::NAN, rhythm: &a }, 8.0), Err(PlanError::BadInput));
        assert_eq!(plan(&ok, &ok, 0.0), Err(PlanError::BadInput));
        assert_eq!(plan(&ok, &ok, f64::INFINITY), Err(PlanError::BadInput));
    }
}
