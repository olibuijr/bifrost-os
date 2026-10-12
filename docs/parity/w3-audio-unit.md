# Wave 3: Audio Unit (MIDI input and note data)

Branch `parity/w3-audio-unit`, based on `parity/integration` (waves 1 and 2).

## What was built

Only one gap row of the Audio Unit framework can be built in pure Rust without new crates, unsafe code or
hardware access: **MIDI input and note data**. The other gap rows need a plug-in host (dynamic loading of
third-party binaries, which needs unsafe code and a loader) or an extension point (an app-extension process
model). They are left as gaps.

New module `springboard/ui/src/midi_events.rs` (one `pub mod midi_events;` line in `springboard/ui/src/lib.rs`):

- `MidiEvent`: the seven MIDI 1.0 channel voice messages (note off, note on, poly pressure, control change,
  program change, channel pressure, pitch bend).
- `MidiParser`: byte-stream parser with running status, real-time bytes (0xF8 to 0xFF) skipped inside a
  message, system common messages clearing running status, and system exclusive data skipped. A note-on with
  velocity 0 parses as a note-off.
- `encode`: the reverse of the parser, masking channels to 4 bits and data to 7 bits.
- `NoteTracker`: per-channel held keys with velocity, sustain pedal (controller 64) holding released keys,
  and all-notes-off (controller 123) per channel.
- `key_frequency` (equal temperament, key 69 = 440 Hz) and `velocity_level` (0 to 127 mapped to 0.0 to 1.0).

The device side (opening an input and feeding bytes) is not part of this module. The shell or a driver calls
`MidiParser::push` and `NoteTracker::apply`.

## Apple API mirrored

Official page: https://developer.apple.com/documentation/audiounit (the feature row "MIDI events delivered to
instrument units"). The module mirrors the event concepts (note on, note off, control change, pitch bend,
program change) rather than specific symbols. The message symbols of the Core MIDI rows belong to wave 2 and
are not repeated here.

REA evidence: no new `rea-page.sh` run was made in this wave. The matrix row's existing evidence is
`ev_3bf9c005e0da939facd73989466bcbdab9f09176783e42e4aeed3b6e830276e4`.

## Tests

`#[cfg(test)] mod tests` in `midi_events.rs`, 11 tests worked out by hand from the MIDI 1.0 rules:

- note-on and velocity-0 note-off parsing
- running status repeating the note-on status byte
- real-time byte skipped inside a message
- system common clearing running status
- system exclusive data skipped until 0xF7
- pitch bend 0x00 0x40 giving 8192
- encode bytes (0x93 64 80; pitch bend 16383 as 0x7F 0x7F)
- encode then parse round trip
- key frequency (key 69 = 440 Hz, key 57 = 220 Hz, key 81 = 880 Hz) and velocity level
- sustain pedal holding a released key until the pedal lifts
- note-off without pedal, and note-on retriggering a sustained key
- all-notes-off clearing only one channel

No cargo or test run was made, as the wave rules require. `python3 tools/check-names.py` exits 0.

## Matrix rows closed

| Framework slug | Feature (exact text from `data/audio-unit.json`) | New status | BifrOSt path |
|---|---|---|---|
| audio-unit | MIDI input and note data | partial | springboard/ui/src/midi_events.rs |

The device input side (opening a MIDI port and feeding bytes into `MidiParser`) is still missing, so the status
is partial, not parity.

## Gap rows left open

- Hosting third-party AU v2 and v3 plug-ins: needs dynamic loading (unsafe code), not pure Rust.
- Implementing AU v3 plug-in extensions: needs an app-extension process model.
