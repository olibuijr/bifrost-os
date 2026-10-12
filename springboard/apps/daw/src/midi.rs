//! MIDI 1.0 wiring for the DAW. Note input is a `Recorder` that turns channel messages (from a byte
//! stream or from events the shell delivers) into song notes, with the sustain pedal holding notes open.
//! Note output is `schedule`, the wire bytes of a clip's notes at their song positions. `clock_pulses` gives
//! the MIDI clock pulses between two song positions for external sync. Parsing and encoding come from the
//! `bifrost_ui::midi_events` model; nothing here opens a device.
use crate::model::{Extra, Note};
use bifrost_app::controls::midi_events::{encode, MidiEvent, MidiParser, CONTROLLER_SUSTAIN};
use std::collections::BTreeMap;

/// Real-time byte: one MIDI clock pulse.
pub const CLOCK: u8 = 0xF8;
/// Real-time byte: start from the beginning of the song.
pub const START: u8 = 0xFA;
/// Real-time byte: continue from the current position.
pub const CONTINUE: u8 = 0xFB;
/// Real-time byte: stop.
pub const STOP: u8 = 0xFC;
/// MIDI clock pulses per sixteenth step: 24 per quarter note and four steps per quarter note.
pub const CLOCKS_PER_STEP: f64 = 6.0;

/// The clock pulses between two song positions given in sixteenth steps. Positions are rounded to the nearest
/// pulse, and a position that goes backwards gives 0.
pub fn clock_pulses(from_step: f64, to_step: f64) -> u32 {
    let a = (from_step * CLOCKS_PER_STEP).round();
    let b = (to_step * CLOCKS_PER_STEP).round();
    (b - a).max(0.0) as u32
}

/// Records channel messages as notes. Each message carries the song position in sixteenth steps when it
/// arrived. Channels are kept apart for the sustain pedal, and the note model has no channel, so notes from
/// different channels land in the same track.
#[derive(Default)]
pub struct Recorder {
    parser: MidiParser,
    held: BTreeMap<(u8, u8), (f64, f64)>,
    pedal: [bool; 16],
    sustained: Vec<(u8, u8, f64, f64)>,
    done: Vec<Note>,
}

impl Recorder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feeds raw MIDI 1.0 bytes that all arrived at `step`. Running status carries across calls.
    pub fn bytes(&mut self, step: f64, bytes: &[u8]) {
        for &b in bytes {
            if let Some(event) = self.parser.push(b) {
                self.event(step, event);
            }
        }
    }

    /// Applies one channel message that arrived at `step`.
    pub fn event(&mut self, step: f64, event: MidiEvent) {
        match event {
            MidiEvent::NoteOn { channel, key, velocity } => {
                if let Some((start, velocity)) = self.held.remove(&(channel, key)) {
                    self.push(key, start, step, velocity);
                }
                self.held.insert((channel, key), (step, f64::from(velocity) / 127.0));
            }
            MidiEvent::NoteOff { channel, key, .. } => {
                if let Some((start, velocity)) = self.held.remove(&(channel, key)) {
                    if self.pedal[usize::from(channel & 0x0F)] {
                        self.sustained.push((channel, key, start, velocity));
                    } else {
                        self.push(key, start, step, velocity);
                    }
                }
            }
            MidiEvent::ControlChange { channel, controller: CONTROLLER_SUSTAIN, value } => {
                let index = usize::from(channel & 0x0F);
                let down = value >= 64;
                if !down && self.pedal[index] {
                    self.release(channel, step);
                }
                self.pedal[index] = down;
            }
            _ => {}
        }
    }

    /// Closes every note still held or sustained at `end_step` and returns the notes in song order.
    pub fn finish(mut self, end_step: f64) -> Vec<Note> {
        for ((_, key), (start, velocity)) in std::mem::take(&mut self.held) {
            self.push(key, start, end_step, velocity);
        }
        for (_, key, start, velocity) in std::mem::take(&mut self.sustained) {
            self.push(key, start, end_step, velocity);
        }
        self.done.sort_by(|a, b| a.t.total_cmp(&b.t).then(a.p.total_cmp(&b.p)));
        self.done
    }

    /// Ends the sustained notes of one channel at `step`.
    fn release(&mut self, channel: u8, step: f64) {
        let (mine, rest): (Vec<_>, Vec<_>) = std::mem::take(&mut self.sustained)
            .into_iter()
            .partition(|s| s.0 == channel);
        self.sustained = rest;
        for (_, key, start, velocity) in mine {
            self.push(key, start, step, velocity);
        }
    }

    fn push(&mut self, key: u8, start: f64, end: f64, velocity: f64) {
        self.done.push(Note {
            t: start,
            d: (end - start).max(0.0),
            p: f64::from(key),
            v: velocity,
            prob: None,
            extra: Extra::new(),
        });
    }
}

/// One wire message of a clip, due at a song position.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Scheduled {
    /// Song position in sixteenth steps.
    pub step: f64,
    /// True for a note-on, false for a note-off.
    pub on: bool,
    /// The wire bytes; only the first `len` (2 or 3) are used.
    pub bytes: [u8; 3],
    pub len: usize,
}

/// The note-on and note-off messages for `notes` on `channel` (0 to 15), in song order. At one position a
/// note-off comes before a note-on so a repeated key sounds again. Notes with a non-finite field are skipped.
pub fn schedule(notes: &[Note], channel: u8) -> Vec<Scheduled> {
    let mut out = Vec::with_capacity(notes.len() * 2);
    for n in notes {
        if !(n.t.is_finite() && n.d.is_finite() && n.p.is_finite() && n.v.is_finite()) {
            continue;
        }
        let key = n.p.round().clamp(0.0, 127.0) as u8;
        let velocity = (n.v.clamp(0.0, 1.0) * 127.0).round().max(1.0) as u8;
        let (bytes, len) = encode(MidiEvent::NoteOn { channel, key, velocity });
        out.push(Scheduled { step: n.t, on: true, bytes, len });
        let (bytes, len) = encode(MidiEvent::NoteOff { channel, key, velocity: 0 });
        out.push(Scheduled { step: n.t + n.d.max(0.0), on: false, bytes, len });
    }
    out.sort_by(|a, b| a.step.total_cmp(&b.step).then(a.on.cmp(&b.on)));
    out
}
