//! Standard MIDI File import. `import_notes` reads a .mid file with `bifrost_core::smf` and feeds its channel
//! messages to `midi::Recorder`, which pairs note-ons and note-offs and holds notes for the sustain pedal. Times
//! are placed in 16th steps at the song tempo, so a file plays in the song for as long as it plays on its own.
//! `fill_clip` puts the notes into a clip and sizes the clip to whole bars.
use crate::midi::Recorder;
use crate::model::{Clip, Note};
use bifrost_app::controls::midi_events::MidiEvent;
use bifrost_core::smf::{Message, Smf};

/// Sixteenth steps in one bar of 4/4.
pub const BAR_STEPS: u32 = 16;
/// The longest clip the song format allows, in 16th steps.
pub const MAX_STEPS: u32 = 4096;

/// Notes from the channel messages of a Standard MIDI File, in song order. Fails when the bytes are not a file the
/// reader accepts, or when the song tempo is not a positive number.
pub fn import_notes(bytes: &[u8], bpm: f64) -> Result<Vec<Note>, String> {
    if !(bpm.is_finite() && bpm > 0.0) {
        return Err("the song tempo is not set".into());
    }
    let smf = Smf::parse(bytes).map_err(|e| e.to_string())?;
    let rate = bpm / 60.0 * 4.0;
    let mut recorder = Recorder::new();
    for ev in smf.events() {
        if let Some(event) = channel_event(ev.message) {
            recorder.event(ev.seconds * rate, event);
        }
    }
    Ok(recorder.finish(smf.duration() * rate))
}

/// The messages the recorder uses. Program, pressure and pitch-bend messages are not notes and are skipped.
fn channel_event(message: Message) -> Option<MidiEvent> {
    match message {
        Message::NoteOn { channel, key, velocity } => Some(MidiEvent::NoteOn { channel, key, velocity }),
        Message::NoteOff { channel, key, velocity } => Some(MidiEvent::NoteOff { channel, key, velocity }),
        Message::ControlChange { channel, controller, value } => Some(MidiEvent::ControlChange { channel, controller, value }),
        _ => None,
    }
}

/// The clip length that holds the notes: whole bars covering the last note's end, at least one bar, at most
/// `MAX_STEPS`.
pub fn clip_length(notes: &[Note]) -> u32 {
    let end = notes.iter().map(|n| n.t + n.d).fold(0.0_f64, f64::max);
    let bars = ((end / f64::from(BAR_STEPS)).ceil() as u32).max(1);
    (bars * BAR_STEPS).min(MAX_STEPS)
}

/// Puts the notes into the clip. The clip grows to fit them and keeps a longer length it already had.
pub fn fill_clip(clip: &mut Clip, notes: Vec<Note>) {
    clip.length = clip.length.max(clip_length(&notes)).min(MAX_STEPS);
    clip.notes = notes;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Extra;

    /// One format 0 track, 96 ticks per quarter note: a C4 (key 60, velocity 100) from tick 0 to tick 96.
    /// The default tempo is 120 BPM, so the note lasts half a second.
    fn one_note_file() -> Vec<u8> {
        let mut b = Vec::new();
        b.extend_from_slice(b"MThd");
        b.extend_from_slice(&6u32.to_be_bytes());
        b.extend_from_slice(&[0, 0, 0, 1, 0, 96]);
        b.extend_from_slice(b"MTrk");
        let body: [u8; 12] = [0x00, 0x90, 0x3C, 0x64, 0x60, 0x80, 0x3C, 0x00, 0x00, 0xFF, 0x2F, 0x00];
        b.extend_from_slice(&(body.len() as u32).to_be_bytes());
        b.extend_from_slice(&body);
        b
    }

    fn note(t: f64, d: f64) -> Note {
        Note { t, d, p: 60.0, v: 1.0, prob: None, extra: Extra::new() }
    }

    #[test]
    fn imports_a_note_at_the_song_tempo_in_steps() {
        let notes = import_notes(&one_note_file(), 120.0).expect("import");
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].t, 0.0);
        assert!((notes[0].d - 4.0).abs() < 1e-9, "half a second at 120 BPM is 4 steps");
        assert_eq!(notes[0].p, 60.0);
        assert!((notes[0].v - 100.0 / 127.0).abs() < 1e-9);
    }

    #[test]
    fn a_faster_song_tempo_gives_more_steps_for_the_same_time() {
        let notes = import_notes(&one_note_file(), 240.0).expect("import");
        assert!((notes[0].d - 8.0).abs() < 1e-9);
    }

    #[test]
    fn rejects_bytes_that_are_not_a_midi_file_and_a_bad_tempo() {
        assert!(import_notes(b"RIFF0000WAVE", 120.0).is_err());
        assert!(import_notes(&one_note_file(), 0.0).is_err());
        assert!(import_notes(&one_note_file(), f64::NAN).is_err());
    }

    #[test]
    fn clip_length_is_whole_bars_covering_the_notes() {
        assert_eq!(clip_length(&[]), 16);
        assert_eq!(clip_length(&[note(0.0, 4.0)]), 16);
        assert_eq!(clip_length(&[note(0.0, 16.0)]), 16);
        assert_eq!(clip_length(&[note(10.0, 7.0)]), 32);
        assert_eq!(clip_length(&[note(0.0, 1.0e6)]), MAX_STEPS);
    }

    #[test]
    fn fill_clip_takes_the_notes_and_keeps_a_longer_length() {
        let mut clip = Clip { id: "c".into(), name: String::new(), length: 64, notes: vec![], automation: None, cc: None, extra: Extra::new() };
        fill_clip(&mut clip, vec![note(0.0, 4.0)]);
        assert_eq!(clip.length, 64);
        assert_eq!(clip.notes.len(), 1);
        let mut short = Clip { length: 16, ..clip.clone() };
        fill_clip(&mut short, vec![note(20.0, 4.0)]);
        assert_eq!(short.length, 32);
    }
}
