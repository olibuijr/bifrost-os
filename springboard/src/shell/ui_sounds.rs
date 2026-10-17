//! The interface sound effects of the reference system (focus, drawer, link, error and similar cues), as BifrOSt's
//! own synthesised substitutes. Like the alert tones in `alert_sound` and the earcons in `earcon`, each cue is built
//! from notes, so BifrOSt ships no recordings. Names are the cue names the reference uses, so an app that asks for
//! one by name gets a cue of the same role. This module is pure: it names the cues and renders their samples.

use super::alert_sound::{render, Note};

const fn note(hz: f32, at_ms: f32, len_ms: f32, peak: f32) -> Note {
    Note { hz, at_ms, len_ms, peak }
}

/// A noise burst: a note with no pitch.
const fn noise(at_ms: f32, len_ms: f32, peak: f32) -> Note {
    Note { hz: 0.0, at_ms, len_ms, peak }
}

/// Every interface cue: its name and the notes that make it.
pub const UI_SOUNDS: &[(&str, &[Note])] = &[
    ("ActionsAvailable", &[note(880.0, 0.0, 90.0, 0.3), note(1175.0, 110.0, 120.0, 0.3)]),
    ("Alert", &[note(988.0, 0.0, 120.0, 0.4), note(988.0, 160.0, 160.0, 0.4)]),
    ("AlertRepeat", &[note(988.0, 0.0, 90.0, 0.35)]),
    ("AnimationFlyToDownloads", &[noise(0.0, 300.0, 0.12), note(523.0, 0.0, 300.0, 0.2)]),
    ("AnimationShake", &[note(180.0, 0.0, 60.0, 0.4), note(160.0, 70.0, 60.0, 0.4), note(180.0, 140.0, 60.0, 0.4)]),
    ("Announcement", &[note(784.0, 0.0, 160.0, 0.35), note(988.0, 190.0, 220.0, 0.35)]),
    ("AttachmentBegin", &[note(659.0, 0.0, 90.0, 0.3)]),
    ("AttachmentEnd", &[note(988.0, 0.0, 90.0, 0.3)]),
    ("Back", &[note(523.0, 0.0, 90.0, 0.3), note(392.0, 100.0, 90.0, 0.3)]),
    ("BackThrough", &[noise(0.0, 30.0, 0.2), note(392.0, 0.0, 110.0, 0.25)]),
    ("BrailleChordClockTick", &[note(3000.0, 0.0, 15.0, 0.35)]),
    ("BrailleChordFailed", &[note(300.0, 0.0, 120.0, 0.35), note(260.0, 130.0, 120.0, 0.35)]),
    ("BrailleChordMemorize", &[note(1200.0, 0.0, 60.0, 0.3), note(1500.0, 80.0, 60.0, 0.3)]),
    ("BrailleDisplayConnected", &[note(988.0, 0.0, 60.0, 0.3), note(1319.0, 80.0, 120.0, 0.3)]),
    ("BrailleDisplayDisconnected", &[note(1319.0, 0.0, 60.0, 0.3), note(988.0, 80.0, 120.0, 0.3)]),
    ("BubbleAppear", &[note(1200.0, 0.0, 70.0, 0.3), note(1600.0, 50.0, 90.0, 0.2)]),
    ("BubbleDisappear", &[note(1600.0, 0.0, 70.0, 0.25), note(1200.0, 50.0, 90.0, 0.2)]),
    ("CallOut", &[note(698.0, 0.0, 140.0, 0.35), note(880.0, 150.0, 200.0, 0.35)]),
    ("Capitals", &[note(1760.0, 0.0, 30.0, 0.3)]),
    ("ClickFast", &[note(2000.0, 0.0, 12.0, 0.4)]),
    ("DragShort", &[noise(0.0, 25.0, 0.2)]),
    ("DrawOn", &[noise(0.0, 120.0, 0.08), note(440.0, 0.0, 120.0, 0.12)]),
    ("DrawerClosing", &[noise(0.0, 140.0, 0.12), note(300.0, 0.0, 140.0, 0.12)]),
    ("DrawerOpening", &[noise(0.0, 140.0, 0.12), note(360.0, 0.0, 140.0, 0.12)]),
    ("ElementBorder", &[note(1400.0, 0.0, 40.0, 0.25)]),
    ("ElementBorderHit", &[note(1400.0, 0.0, 40.0, 0.25), note(2100.0, 50.0, 50.0, 0.25)]),
    ("Ellipsis", &[note(1000.0, 0.0, 25.0, 0.25), note(1000.0, 60.0, 25.0, 0.25), note(1000.0, 120.0, 25.0, 0.25)]),
    ("EmptySpace", &[note(600.0, 0.0, 40.0, 0.2)]),
    ("EnterInvisibleArea", &[note(900.0, 0.0, 50.0, 0.2)]),
    ("EnterVisibleArea", &[note(1100.0, 0.0, 50.0, 0.25)]),
    ("EnteringMultiSelect", &[note(784.0, 0.0, 60.0, 0.3), note(784.0, 80.0, 60.0, 0.3)]),
    ("Error", &[note(220.0, 0.0, 200.0, 0.45), note(165.0, 190.0, 260.0, 0.45)]),
    ("ExitingMultiSelect", &[note(784.0, 0.0, 60.0, 0.3), note(660.0, 80.0, 60.0, 0.3)]),
    ("Focus", &[note(1046.0, 0.0, 70.0, 0.25)]),
    ("Focus2", &[note(1318.0, 0.0, 50.0, 0.25)]),
    ("Forward", &[note(392.0, 0.0, 90.0, 0.3), note(523.0, 100.0, 90.0, 0.3)]),
    ("ForwardThrough", &[noise(0.0, 30.0, 0.2), note(523.0, 0.0, 110.0, 0.25)]),
    ("GazeEnteredScene", &[note(1500.0, 0.0, 80.0, 0.2), note(1800.0, 90.0, 80.0, 0.2)]),
    ("GazeLeftScene", &[note(1800.0, 0.0, 80.0, 0.2), note(1500.0, 90.0, 80.0, 0.2)]),
    ("GuideProgress", &[note(660.0, 0.0, 50.0, 0.25)]),
    ("GuideSuccess", &[note(523.0, 0.0, 100.0, 0.3), note(659.0, 110.0, 100.0, 0.3), note(784.0, 220.0, 200.0, 0.3)]),
    ("Hit", &[note(240.0, 0.0, 40.0, 0.4), noise(0.0, 8.0, 0.2)]),
    ("Indentation", &[note(1500.0, 0.0, 30.0, 0.25)]),
    ("Interactable", &[note(1200.0, 0.0, 40.0, 0.2), note(1200.0, 70.0, 40.0, 0.2)]),
    ("KBVONotSynched", &[note(480.0, 0.0, 50.0, 0.2)]),
    ("KBVOSynched", &[note(720.0, 0.0, 50.0, 0.2)]),
    ("LayoutChanged", &[note(600.0, 0.0, 60.0, 0.25), note(800.0, 70.0, 60.0, 0.25)]),
    ("LinkBegin", &[note(1000.0, 0.0, 40.0, 0.25)]),
    ("LinkEnd", &[note(800.0, 0.0, 40.0, 0.25)]),
];

/// The notes of the interface cue `name`, or `None` for a name that is not one of `UI_SOUNDS`.
pub fn notes(name: &str) -> Option<&'static [Note]> {
    UI_SOUNDS.iter().find(|(n, _)| *n == name).map(|(_, notes)| *notes)
}

/// The samples of the interface cue `name` at `volume` (0..=1), or `None` for an unknown name.
pub fn pcm(name: &str, volume: f32) -> Option<Vec<i16>> {
    notes(name).map(|n| render(n, volume))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_cue_has_notes_and_audible_samples() {
        for (name, _) in UI_SOUNDS {
            assert!(!notes(name).unwrap().is_empty(), "{name} has no notes");
            assert!(pcm(name, 1.0).unwrap().iter().any(|&s| s != 0), "{name} is silent");
        }
    }

    #[test]
    fn cue_names_are_unique() {
        for (i, (a, _)) in UI_SOUNDS.iter().enumerate() {
            for (b, _) in &UI_SOUNDS[..i] {
                assert_ne!(a, b, "cue {a} is listed twice");
            }
        }
    }

    #[test]
    fn unknown_names_have_no_cue() {
        assert!(notes("NoSuchCue").is_none());
        assert!(pcm("NoSuchCue", 1.0).is_none());
    }

    #[test]
    fn error_and_focus_sound_different() {
        assert_ne!(notes("Error"), notes("Focus"));
    }
}
