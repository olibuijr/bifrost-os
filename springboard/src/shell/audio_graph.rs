//! Audio graphs on the sound-effects role: the tones of a chart's audio graph (`ui::chart_ax::ChartDescriptor::audio_graph`)
//! rendered as a short sequence of notes. Pitch and length come from the tone; a tone with no pitch (`hz` 0, a missing
//! value) is a rest that keeps the timing. The notes are rendered with the alert tone synthesiser, so nothing is recorded.

use super::alert_sound::{render, Note};
use bifrost_ui::chart_ax::Tone;

/// Peak level of every note: quiet enough to play under speech.
const PEAK: f32 = 0.3;

/// The notes of a graph, in order. Each tone starts where the previous one ended; a rest adds its length and no note.
pub fn notes(tones: &[Tone]) -> Vec<Note> {
    let mut at_ms = 0.0f32;
    let mut out = Vec::with_capacity(tones.len());
    for t in tones {
        let len_ms = t.millis as f32;
        if t.hz > 0.0 {
            out.push(Note { hz: t.hz as f32, at_ms, len_ms, peak: PEAK });
        }
        at_ms += len_ms;
    }
    out
}

/// Mono i16 PCM of a graph at `RATE`, scaled by `volume` (0..=1). Empty for a graph with no tones.
pub fn pcm(tones: &[Tone], volume: f32) -> Vec<i16> {
    if tones.is_empty() {
        return Vec::new();
    }
    render(&notes(tones), volume)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(hz: f64, millis: u32) -> Tone {
        Tone { hz, millis, timbre: 0 }
    }

    #[test]
    fn each_tone_starts_where_the_last_one_ended() {
        let n = notes(&[tone(440.0, 100), tone(880.0, 250)]);
        assert_eq!(n.len(), 2);
        assert_eq!((n[0].at_ms, n[0].len_ms), (0.0, 100.0));
        assert_eq!((n[1].at_ms, n[1].len_ms), (100.0, 250.0));
        assert_eq!(n[1].hz, 880.0);
    }

    #[test]
    fn a_rest_keeps_its_timing_and_makes_no_note() {
        let n = notes(&[tone(440.0, 100), tone(0.0, 200), tone(660.0, 50)]);
        assert_eq!(n.len(), 2);
        assert_eq!(n[1].at_ms, 300.0, "the rest counts toward the start of the next tone");
    }

    #[test]
    fn an_empty_graph_renders_nothing_and_a_tone_renders_samples() {
        assert!(pcm(&[], 1.0).is_empty());
        assert!(pcm(&[tone(440.0, 120)], 1.0).iter().any(|&s| s != 0));
        assert!(pcm(&[tone(0.0, 120)], 1.0).iter().all(|&s| s == 0), "a rest alone is silent");
    }
}
