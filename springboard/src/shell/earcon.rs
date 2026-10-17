//! Earcons: the short non-speech cues that accompany accessibility events (focus, menus, selection, wrapping, text
//! tracking). Each cue is synthesised from notes, like the alert tones in `alert_sound`, so BifrOSt ships no recordings.
//! This module is pure: it names the cues and renders their samples. The player in `media::system_sound` registers and
//! plays them with `SoundPlayer::earcon`.

use super::alert_sound::{render, Note};

const fn note(hz: f32, at_ms: f32, len_ms: f32, peak: f32) -> Note {
    Note { hz, at_ms, len_ms, peak }
}

/// A noise burst: a note with no pitch.
const fn noise(at_ms: f32, len_ms: f32, peak: f32) -> Note {
    Note { hz: 0.0, at_ms, len_ms, peak }
}

/// Every earcon: its stable name and the notes that make it.
pub const EARCONS: &[(&str, &[Note])] = &[
    ("menu-down", &[note(392.0, 0.0, 60.0, 0.35), note(588.0, 90.0, 60.0, 0.3)]),
    ("misspelled", &[note(415.3, 0.0, 80.0, 0.4), noise(0.0, 12.0, 0.15)]),
    ("more-content", &[note(660.0, 0.0, 140.0, 0.3)]),
    ("mouse-down", &[note(466.2, 0.0, 120.0, 0.35), note(582.7, 150.0, 120.0, 0.3)]),
    ("mouse-drag", &[note(493.9, 0.0, 140.0, 0.4), noise(0.0, 12.0, 0.15)]),
    ("mouse-up", &[note(654.1, 0.0, 100.0, 0.3)]),
    ("not-found", &[note(554.4, 0.0, 80.0, 0.35), note(831.6, 110.0, 80.0, 0.3)]),
    ("nothing-tracking", &[note(587.3, 0.0, 100.0, 0.4), noise(0.0, 12.0, 0.15)]),
    ("popup-appeared", &[note(933.4, 0.0, 160.0, 0.3)]),
    ("reorder", &[note(659.3, 0.0, 140.0, 0.35), note(824.1, 170.0, 140.0, 0.3)]),
    ("ring", &[note(698.5, 0.0, 60.0, 0.4), noise(0.0, 12.0, 0.15)]),
    ("rotor-left", &[note(925.0, 0.0, 120.0, 0.3)]),
    ("rotor-right", &[note(392.0, 0.0, 100.0, 0.35), note(588.0, 130.0, 100.0, 0.3)]),
    ("scroll-page", &[note(415.3, 0.0, 120.0, 0.4), noise(0.0, 12.0, 0.15)]),
    ("search-range", &[note(660.0, 0.0, 180.0, 0.3)]),
    ("secure-character", &[note(466.2, 0.0, 60.0, 0.35), note(582.7, 90.0, 60.0, 0.3)]),
    ("select", &[note(493.9, 0.0, 80.0, 0.4), noise(0.0, 12.0, 0.15)]),
    ("single-click", &[note(654.1, 0.0, 140.0, 0.3)]),
    ("spring-back", &[note(554.4, 0.0, 120.0, 0.35), note(831.6, 150.0, 120.0, 0.3)]),
    ("spring-forward", &[note(587.3, 0.0, 140.0, 0.4), noise(0.0, 12.0, 0.15)]),
    ("style-change-double", &[note(933.4, 0.0, 100.0, 0.3)]),
    ("style-change-multiple", &[note(659.3, 0.0, 80.0, 0.35), note(824.1, 110.0, 80.0, 0.3)]),
    ("style-change-single", &[note(698.5, 0.0, 100.0, 0.4), noise(0.0, 12.0, 0.15)]),
    ("tracking-off", &[note(925.0, 0.0, 160.0, 0.3)]),
    ("tracking-on", &[note(392.0, 0.0, 140.0, 0.35), note(588.0, 170.0, 140.0, 0.3)]),
    ("tracking-system-element", &[note(415.3, 0.0, 60.0, 0.4), noise(0.0, 12.0, 0.15)]),
    ("web-area-loaded", &[note(660.0, 0.0, 120.0, 0.3)]),
    ("web-page-loaded", &[note(466.2, 0.0, 100.0, 0.35), note(582.7, 130.0, 100.0, 0.3)]),
    ("window-activated", &[note(493.9, 0.0, 120.0, 0.4), noise(0.0, 12.0, 0.15)]),
    ("wrap-back", &[note(654.1, 0.0, 180.0, 0.3)]),
    ("wrap-back-hard", &[note(554.4, 0.0, 60.0, 0.35), note(831.6, 90.0, 60.0, 0.3)]),
    ("wrap-boundary", &[note(587.3, 0.0, 80.0, 0.4), noise(0.0, 12.0, 0.15)]),
    ("wrap-forward", &[note(933.4, 0.0, 140.0, 0.3)]),
    ("wrap-forward-hard", &[note(659.3, 0.0, 120.0, 0.35), note(824.1, 150.0, 120.0, 0.3)]),
    ("line-breakpoint", &[note(698.5, 0.0, 140.0, 0.4), noise(0.0, 12.0, 0.15)]),
    ("line-error", &[note(925.0, 0.0, 100.0, 0.3)]),
    ("line-warning", &[note(392.0, 0.0, 80.0, 0.35), note(588.0, 110.0, 80.0, 0.3)]),
];

/// The earcon a popup (a desktop menu or the power menu) plays when it appears.
pub const POPUP_APPEARED: &str = "popup-appeared";

/// The earcon a popup plays when it closes (the falling tone).
pub const POPUP_CLOSED: &str = "tracking-off";

/// Watches, frame by frame, whether a popup is open and queues the earcon of each change. Opening one menu while another
/// is open is not a change, so moving between menus stays quiet. Pure: the shell feeds it `observe` and plays `take`.
#[derive(Debug, Default)]
pub struct PopupCues {
    open: bool,
    queued: Vec<usize>,
}

impl PopupCues {
    /// Records whether a popup is open now; a change queues its earcon.
    pub fn observe(&mut self, open: bool) {
        if open == self.open {
            return;
        }
        self.open = open;
        let name = if open { POPUP_APPEARED } else { POPUP_CLOSED };
        if let Some(i) = index_of(name) {
            self.queued.push(i);
        }
    }

    /// The earcon indices queued since the last call, in order.
    pub fn take(&mut self) -> Vec<usize> {
        std::mem::take(&mut self.queued)
    }
}

/// The number of earcons.
pub fn count() -> usize {
    EARCONS.len()
}

/// The stable name of earcon `index`, if it exists.
pub fn name(index: usize) -> Option<&'static str> {
    EARCONS.get(index).map(|e| e.0)
}

/// The index of the earcon called `name`.
pub fn index_of(name: &str) -> Option<usize> {
    EARCONS.iter().position(|e| e.0 == name)
}

/// The notes of earcon `index`. An index past the list has no notes.
pub fn notes(index: usize) -> &'static [Note] {
    EARCONS.get(index).map(|e| e.1).unwrap_or(&[])
}

/// Mono i16 PCM of earcon `index` at `RATE`, scaled by `volume` (0..=1). Empty when the index is unknown.
pub fn pcm(index: usize, volume: f32) -> Vec<i16> {
    if index >= EARCONS.len() {
        return Vec::new();
    }
    render(notes(index), volume)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_earcon_has_a_name_notes_and_audible_samples() {
        assert!(count() >= 38, "the accessibility earcon set is complete");
        for i in 0..count() {
            assert!(name(i).is_some_and(|n| !n.is_empty()), "earcon {i} has no name");
            assert!(!notes(i).is_empty(), "earcon {i} has no notes");
            assert!(pcm(i, 1.0).iter().any(|&s| s != 0), "earcon {i} is silent");
        }
    }

    #[test]
    fn names_are_unique_and_found_by_index_of() {
        for i in 0..count() {
            let n = name(i).unwrap();
            assert_eq!(index_of(n), Some(i), "{n} is not unique");
            assert!(n.chars().all(|c| c.is_ascii_lowercase() || c == '-'), "{n} is not a stable slug");
        }
        assert_eq!(index_of("no-such-earcon"), None);
    }

    #[test]
    fn earcons_sound_different_from_each_other() {
        for i in 0..count() {
            for j in 0..i {
                assert_ne!(notes(i), notes(j), "{} and {} share notes", name(i).unwrap(), name(j).unwrap());
            }
        }
    }

    #[test]
    fn every_earcon_is_short_and_does_not_clip() {
        for i in 0..count() {
            let p = pcm(i, 1.0);
            assert!(p.len() < (RATE as usize), "{} is longer than a second", name(i).unwrap());
            assert!(p.iter().all(|&s| s.abs() < i16::MAX), "{} clips", name(i).unwrap());
        }
    }

    #[test]
    fn opening_and_closing_a_popup_queues_one_earcon_each() {
        let mut c = PopupCues::default();
        c.observe(false);
        assert!(c.take().is_empty(), "no popup, no cue");
        c.observe(true);
        assert_eq!(c.take(), vec![index_of(POPUP_APPEARED).unwrap()]);
        c.observe(true);
        assert!(c.take().is_empty(), "a popup that stays open is quiet");
        c.observe(false);
        assert_eq!(c.take(), vec![index_of(POPUP_CLOSED).unwrap()]);
        assert!(c.take().is_empty(), "taking drains the queue");
    }

    #[test]
    fn the_popup_earcons_exist_in_the_set() {
        assert!(index_of(POPUP_APPEARED).is_some());
        assert!(index_of(POPUP_CLOSED).is_some());
    }

    #[test]
    fn an_unknown_earcon_renders_nothing() {
        assert!(pcm(count(), 1.0).is_empty());
        assert!(notes(count()).is_empty());
        assert!(name(count()).is_none());
    }
}
