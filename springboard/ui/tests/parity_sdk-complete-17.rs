//! Parity sdk-complete-17: headless tests for the MIDI 1.0 channel event model (`midi_events`). The parser, the
//! encoder, the note tracker with the sustain pedal, and the pitch and velocity mappings are driven through the public
//! API with byte streams. No device is opened.

use bifrost_ui::midi_events::{encode, key_frequency, velocity_level, MidiEvent, MidiParser, NoteTracker, CONTROLLER_SUSTAIN};

fn note_on(channel: u8, key: u8, velocity: u8) -> MidiEvent {
    MidiEvent::NoteOn { channel, key, velocity }
}

#[test]
fn a_note_on_and_its_note_off_parse_in_stream_order() {
    let events = MidiParser::parse_all(&[0x90, 60, 100, 0x80, 60, 0]);
    assert_eq!(events, vec![note_on(0, 60, 100), MidiEvent::NoteOff { channel: 0, key: 60, velocity: 0 }]);
}

#[test]
fn a_note_on_with_zero_velocity_parses_as_a_note_off() {
    assert_eq!(
        MidiParser::parse_all(&[0x91, 64, 0]),
        vec![MidiEvent::NoteOff { channel: 1, key: 64, velocity: 0 }]
    );
}

#[test]
fn running_status_carries_the_status_byte_to_the_next_message() {
    assert_eq!(MidiParser::parse_all(&[0x90, 60, 100, 62, 100]), vec![note_on(0, 60, 100), note_on(0, 62, 100)]);
}

#[test]
fn a_real_time_byte_inside_a_message_does_not_end_it() {
    assert_eq!(MidiParser::parse_all(&[0x90, 60, 0xF8, 100]), vec![note_on(0, 60, 100)]);
}

#[test]
fn a_program_change_takes_one_data_byte() {
    assert_eq!(
        MidiParser::parse_all(&[0xC2, 5, 0x92, 60, 90]),
        vec![MidiEvent::ProgramChange { channel: 2, program: 5 }, note_on(2, 60, 90)]
    );
}

#[test]
fn pitch_bend_joins_its_low_and_high_seven_bits_with_centre_at_8192() {
    assert_eq!(MidiParser::parse_all(&[0xE0, 0x00, 0x40]), vec![MidiEvent::PitchBend { channel: 0, value: 8192 }]);
    assert_eq!(MidiParser::parse_all(&[0xE0, 0x7F, 0x7F]), vec![MidiEvent::PitchBend { channel: 0, value: 16383 }]);
}

#[test]
fn system_exclusive_data_is_skipped_and_the_next_message_parses() {
    assert_eq!(MidiParser::parse_all(&[0xF0, 1, 2, 3, 0xF7, 0x90, 60, 100]), vec![note_on(0, 60, 100)]);
}

#[test]
fn a_data_byte_without_running_status_is_dropped() {
    assert!(MidiParser::parse_all(&[60, 100]).is_empty());
}

#[test]
fn an_encoded_note_on_is_three_wire_bytes_on_its_channel() {
    assert_eq!(encode(note_on(3, 60, 100)), ([0x93, 60, 100], 3));
}

#[test]
fn an_encoded_program_change_is_two_wire_bytes() {
    assert_eq!(encode(MidiEvent::ProgramChange { channel: 1, program: 7 }), ([0xC1, 7, 0], 2));
}

#[test]
fn out_of_range_fields_are_masked_when_encoding() {
    assert_eq!(encode(note_on(0x13, 0xFF, 0x80)), ([0x93, 0x7F, 0x00], 3));
}

#[test]
fn a_pitch_bend_encodes_its_two_seven_bit_halves() {
    assert_eq!(encode(MidiEvent::PitchBend { channel: 0, value: 8192 }), ([0xE0, 0x00, 0x40], 3));
}

#[test]
fn every_event_survives_an_encode_and_parse_round_trip() {
    let events = [
        note_on(5, 48, 90),
        MidiEvent::NoteOff { channel: 5, key: 48, velocity: 64 },
        MidiEvent::PolyPressure { channel: 5, key: 48, pressure: 20 },
        MidiEvent::ControlChange { channel: 5, controller: CONTROLLER_SUSTAIN, value: 127 },
        MidiEvent::ChannelPressure { channel: 5, pressure: 33 },
        MidiEvent::PitchBend { channel: 5, value: 1000 },
    ];
    for event in events {
        let (bytes, n) = encode(event);
        assert_eq!(MidiParser::parse_all(&bytes[..n]), vec![event]);
    }
}

#[test]
fn key_69_sounds_at_440_hertz_and_each_octave_doubles_the_frequency() {
    assert!((key_frequency(69) - 440.0).abs() < 1e-9);
    assert!((key_frequency(81) - 880.0).abs() < 1e-9);
    assert!((key_frequency(57) - 220.0).abs() < 1e-9);
}

#[test]
fn velocity_maps_linearly_onto_zero_to_one_and_clamps() {
    assert_eq!(velocity_level(0), 0.0);
    assert_eq!(velocity_level(127), 1.0);
    assert_eq!(velocity_level(200), 1.0, "values above 127 clamp");
}

#[test]
fn a_held_key_is_sounding_until_it_is_released() {
    let mut t = NoteTracker::new();
    t.apply(note_on(0, 60, 100));
    assert_eq!(t.held_velocity(0, 60), Some(100));
    assert!(t.is_sounding(0, 60));
    t.apply(MidiEvent::NoteOff { channel: 0, key: 60, velocity: 0 });
    assert_eq!(t.held_velocity(0, 60), None);
    assert!(!t.is_sounding(0, 60));
}

#[test]
fn the_sustain_pedal_keeps_a_released_key_ringing() {
    let mut t = NoteTracker::new();
    t.apply(MidiEvent::ControlChange { channel: 0, controller: CONTROLLER_SUSTAIN, value: 127 });
    assert!(t.pedal_down(0));
    t.apply(note_on(0, 60, 100));
    t.apply(MidiEvent::NoteOff { channel: 0, key: 60, velocity: 0 });
    assert_eq!(t.held_velocity(0, 60), None, "a sustained key is not held");
    assert!(t.is_sounding(0, 60), "but it still rings");
    assert_eq!(t.sounding_count(), 1);
}

#[test]
fn lifting_the_pedal_silences_the_sustained_keys() {
    let mut t = NoteTracker::new();
    t.apply(MidiEvent::ControlChange { channel: 0, controller: CONTROLLER_SUSTAIN, value: 127 });
    t.apply(note_on(0, 60, 100));
    t.apply(MidiEvent::NoteOff { channel: 0, key: 60, velocity: 0 });
    t.apply(MidiEvent::ControlChange { channel: 0, controller: CONTROLLER_SUSTAIN, value: 0 });
    assert!(!t.pedal_down(0));
    assert!(!t.is_sounding(0, 60));
    assert_eq!(t.sounding_count(), 0);
}

#[test]
fn a_pedal_value_below_64_is_up() {
    let mut t = NoteTracker::new();
    t.apply(MidiEvent::ControlChange { channel: 2, controller: CONTROLLER_SUSTAIN, value: 63 });
    assert!(!t.pedal_down(2));
    t.apply(MidiEvent::ControlChange { channel: 2, controller: CONTROLLER_SUSTAIN, value: 64 });
    assert!(t.pedal_down(2));
}

#[test]
fn all_notes_off_silences_every_key_on_its_channel_only() {
    let mut t = NoteTracker::new();
    t.apply(note_on(0, 60, 100));
    t.apply(note_on(0, 62, 100));
    t.apply(note_on(1, 64, 100));
    t.apply(MidiEvent::ControlChange { channel: 0, controller: 123, value: 0 });
    assert_eq!(t.sounding_count(), 1, "channel 1 keeps its key");
    assert!(t.is_sounding(1, 64));
}

#[test]
fn channels_track_their_keys_independently() {
    let mut t = NoteTracker::new();
    t.apply(note_on(0, 60, 100));
    assert!(!t.is_sounding(1, 60));
    assert_eq!(t.held_velocity(1, 60), None);
}
