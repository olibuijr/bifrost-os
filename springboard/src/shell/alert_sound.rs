//! The alert tones of Settings > Sound > Alert Sound (`options::ALERT_SOUNDS`): short note sequences that the shell
//! plays through the sound-effects role of the audio server when a timer or alarm fires. This module is pure: it
//! only synthesises samples. The player in `media::system_sound` registers and plays them.

use super::kb_clicks::RATE;
use super::options::ALERT_SOUNDS;

/// One note: frequency in hertz (zero or less makes a noise burst), start and length in milliseconds, and peak amplitude (0..=1).
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

/// A noise burst: a note with no pitch.
const fn noise(at_ms: f32, len_ms: f32, peak: f32) -> Note {
    Note { hz: 0.0, at_ms, len_ms, peak }
}

/// Bell: one long ringing note.
const BELL: [Note; 1] = [note(880.0, 0.0, 900.0, 0.45)];
/// Chime: three rising notes that overlap into a short arpeggio.
const CHIME: [Note; 3] = [note(880.0, 0.0, 260.0, 0.4), note(1175.0, 170.0, 260.0, 0.4), note(1568.0, 340.0, 560.0, 0.4)];
/// Pulse: three equal pulses with silence between them.
const PULSE: [Note; 3] = [note(1000.0, 0.0, 120.0, 0.5), note(1000.0, 250.0, 120.0, 0.5), note(1000.0, 500.0, 120.0, 0.5)];
/// Tick: two short clicks.
const TICK: [Note; 2] = [note(2400.0, 0.0, 25.0, 0.5), note(2400.0, 120.0, 25.0, 0.5)];

/// Basso: a low two-step drop.
const BASSO: [Note; 2] = [note(196.0, 0.0, 320.0, 0.45), note(147.0, 220.0, 620.0, 0.45)];
/// Blow: a breathy noise burst over a low hum.
const BLOW: [Note; 2] = [noise(0.0, 500.0, 0.25), note(220.0, 0.0, 500.0, 0.3)];
/// Bottle: a hollow mid tone with a soft noise onset.
const BOTTLE: [Note; 2] = [noise(0.0, 40.0, 0.2), note(520.0, 0.0, 450.0, 0.5)];
/// Frog: two quick low croaks.
const FROG: [Note; 4] = [note(180.0, 0.0, 110.0, 0.4), note(360.0, 0.0, 110.0, 0.2), note(200.0, 160.0, 130.0, 0.4), note(400.0, 160.0, 130.0, 0.2)];
/// Funk: a low note followed by a lower one with a bright overtone.
const FUNK: [Note; 3] = [note(330.0, 0.0, 200.0, 0.4), note(247.0, 180.0, 450.0, 0.4), note(494.0, 180.0, 450.0, 0.15)];
/// Glass: a high bright ring with a quiet overtone.
const GLASS: [Note; 2] = [note(1319.0, 0.0, 800.0, 0.4), note(2637.0, 0.0, 500.0, 0.12)];
/// Hero: a rising three-note fanfare.
const HERO: [Note; 3] = [note(523.0, 0.0, 220.0, 0.4), note(659.0, 180.0, 220.0, 0.4), note(784.0, 360.0, 700.0, 0.4)];
/// Morse: two short beeps and a long one.
const MORSE: [Note; 3] = [note(700.0, 0.0, 80.0, 0.45), note(700.0, 140.0, 80.0, 0.45), note(700.0, 280.0, 240.0, 0.45)];
/// Ping: one clear mid-high ring.
const PING: [Note; 1] = [note(1568.0, 0.0, 650.0, 0.45)];
/// Pop: a very short low blip.
const POP: [Note; 2] = [note(300.0, 0.0, 70.0, 0.5), noise(0.0, 8.0, 0.3)];
/// Purr: a low rumble of two close tones.
const PURR: [Note; 3] = [note(110.0, 0.0, 700.0, 0.35), note(116.0, 0.0, 700.0, 0.35), noise(0.0, 700.0, 0.08)];
/// Sosumi: a quick rising two-note figure.
const SOSUMI: [Note; 2] = [note(784.0, 0.0, 160.0, 0.45), note(1047.0, 140.0, 420.0, 0.45)];
/// Submarine: a deep long sonar-like ping.
const SUBMARINE: [Note; 2] = [note(262.0, 0.0, 1100.0, 0.45), note(524.0, 0.0, 600.0, 0.15)];
/// Tink: a tiny high tap.
const TINK: [Note; 1] = [note(3136.0, 0.0, 60.0, 0.5)];

/// The notes of alert `index` (an index into `ALERT_SOUNDS`). An index past the list plays the first tone.
pub fn notes(index: usize) -> &'static [Note] {
    match ALERT_SOUNDS.get(index).copied().unwrap_or(ALERT_SOUNDS[0]) {
        "Basso" => &BASSO,
        "Blow" => &BLOW,
        "Bottle" => &BOTTLE,
        "Frog" => &FROG,
        "Funk" => &FUNK,
        "Glass" => &GLASS,
        "Hero" => &HERO,
        "Morse" => &MORSE,
        "Ping" => &PING,
        "Pop" => &POP,
        "Purr" => &PURR,
        "Sosumi" => &SOSUMI,
        "Submarine" => &SUBMARINE,
        "Tink" => &TINK,
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
    let mut seed = 0x2545_F491u32;
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
            let wave = if note.hz > 0.0 {
                (t * note.hz * std::f32::consts::TAU).sin()
            } else {
                // a fixed-seed xorshift, so a noise burst sounds the same every time
                seed ^= seed << 13;
                seed ^= seed >> 17;
                seed ^= seed << 5;
                seed as f32 / u32::MAX as f32 * 2.0 - 1.0
            };
            mix[start + i] += wave * note.peak * attack * decay;
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
    fn every_listed_alert_has_its_own_notes() {
        for i in 1..ALERT_SOUNDS.len() {
            assert_ne!(notes(i), notes(0), "{} falls through to the first tone", ALERT_SOUNDS[i]);
            for j in 0..i {
                assert_ne!(notes(i), notes(j), "{} and {} share notes", ALERT_SOUNDS[i], ALERT_SOUNDS[j]);
            }
        }
    }

    #[test]
    fn the_selectable_names_are_listed() {
        for name in ["Basso", "Blow", "Bottle", "Frog", "Funk", "Glass", "Hero", "Morse", "Ping", "Pop", "Purr", "Sosumi", "Submarine", "Tink"] {
            assert!(ALERT_SOUNDS.contains(&name), "{name} missing");
        }
    }

    #[test]
    fn every_alert_is_short_and_does_not_clip() {
        for i in 0..ALERT_SOUNDS.len() {
            let p = pcm(i, 1.0);
            assert!(p.len() < (RATE as usize * 3) / 2, "{} is too long", ALERT_SOUNDS[i]);
            assert!(p.iter().all(|&s| s.abs() < i16::MAX), "{} clips", ALERT_SOUNDS[i]);
            assert!(p.iter().any(|&s| s != 0), "{} is silent", ALERT_SOUNDS[i]);
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
        // A 1 kHz tone has a zero crossing at every whole millisecond, so 60 ms alone can read silent; look at a window.
        let sounds = (RATE as f32 * 40.0 / 1000.0) as usize..(RATE as f32 * 80.0 / 1000.0) as usize;
        assert!(p[sounds].iter().any(|s| s.abs() > 0), "first pulse sounds");
        assert_eq!(at(180.0), 0, "gap after the first pulse");
    }
}
