//! The alert tones of Settings > Sound > Alert Sound (`options::ALERT_SOUNDS`): short note sequences that the shell
//! plays through the sound-effects role of the audio server when a timer or alarm fires. This module is pure: it
//! only synthesises samples. The player in `media::system_sound` registers and plays them.

use super::kb_clicks::RATE;
use super::options::ALERT_SOUNDS;

/// One note: frequency in hertz, start and length in milliseconds, and peak amplitude (0..=1).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Note {
    pub hz: f32,
    pub at_ms: f32,
    pub len_ms: f32,
    pub peak: f32,
}

const fn note(hz: f32, at_ms: f32, len_ms: f32, peak: f32) -> Note {
    Note { hz, at_ms, len_ms, peak }
}

/// Bell: one long ringing note.
const BELL: [Note; 1] = [note(880.0, 0.0, 900.0, 0.45)];
/// Chime: three rising notes that overlap into a short arpeggio.
const CHIME: [Note; 3] = [note(880.0, 0.0, 260.0, 0.4), note(1175.0, 170.0, 260.0, 0.4), note(1568.0, 340.0, 560.0, 0.4)];
/// Pulse: three equal pulses with silence between them.
const PULSE: [Note; 3] = [note(1000.0, 0.0, 120.0, 0.5), note(1000.0, 250.0, 120.0, 0.5), note(1000.0, 500.0, 120.0, 0.5)];
/// Tick: two short clicks.
const TICK: [Note; 2] = [note(2400.0, 0.0, 25.0, 0.5), note(2400.0, 120.0, 25.0, 0.5)];

/// The notes of alert `index` (an index into `ALERT_SOUNDS`). An index past the list plays the first tone.
pub fn notes(index: usize) -> &'static [Note] {
    match ALERT_SOUNDS.get(index).copied().unwrap_or(ALERT_SOUNDS[0]) {
        "Chime" => &CHIME,
        "Pulse" => &PULSE,
        "Tick" => &TICK,
        _ => &BELL,
    }
}

/// Mono i16 PCM of alert `index` at `RATE`, scaled by `volume` (0..=1).
pub fn pcm(index: usize, volume: f32) -> Vec<i16> {
    let notes = notes(index);
    let end_ms = notes.iter().map(|n| n.at_ms + n.len_ms).fold(0.0f32, f32::max);
    let n = (RATE as f32 * end_ms / 1000.0).ceil() as usize;
    let gain = volume.clamp(0.0, 1.0);
    let mut mix = vec![0f32; n];
    for note in notes {
        let start = (RATE as f32 * note.at_ms / 1000.0) as usize;
        let len = (RATE as f32 * note.len_ms / 1000.0) as usize;
        if len == 0 {
            continue;
        }
        for i in 0..len.min(n.saturating_sub(start)) {
            // a 5 ms attack, then a quadratic decay to silence at the note's end
            let attack = (i as f32 / (RATE as f32 * 0.005)).min(1.0);
            let decay = (1.0 - i as f32 / len as f32).powi(2);
            let t = i as f32 / RATE as f32;
            mix[start + i] += (t * note.hz * std::f32::consts::TAU).sin() * note.peak * attack * decay;
        }
    }
    mix.iter().map(|s| (s.clamp(-1.0, 1.0) * gain * i16::MAX as f32) as i16).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_listed_alert_has_notes_and_audible_samples() {
        for i in 0..ALERT_SOUNDS.len() {
            assert!(!notes(i).is_empty(), "alert {i} has no notes");
            assert!(pcm(i, 1.0).iter().any(|&s| s != 0), "alert {i} is silent");
        }
    }

    #[test]
    fn an_index_past_the_list_plays_the_first_tone() {
        assert_eq!(pcm(99, 1.0), pcm(0, 1.0));
        assert_eq!(notes(99), notes(0));
    }

    #[test]
    fn zero_volume_is_silent_and_the_length_follows_the_last_note() {
        assert!(pcm(1, 0.0).iter().all(|&s| s == 0));
        // Chime ends at 340 + 560 ms
        assert_eq!(pcm(1, 1.0).len(), 43_200);
    }

    #[test]
    fn pulse_is_silent_between_its_pulses() {
        let p = pcm(2, 1.0);
        let at = |ms: f32| p[(RATE as f32 * ms / 1000.0) as usize];
        assert!(at(60.0).abs() > 0, "first pulse sounds");
        assert_eq!(at(180.0), 0, "gap after the first pulse");
    }
}
