//! MIDI 1.0 channel messages as app-facing events: a byte parser with running status and
//! real-time bytes, an encoder back to wire bytes, and a note tracker that follows held notes and the
//! sustain pedal per channel. This is the pure model behind note input. No device is opened here: the
//! shell or a driver feeds bytes to `MidiParser::push` and sends the events to `NoteTracker::apply`.

/// One MIDI 1.0 channel voice message. Channels are 0 to 15, keys and data values are 0 to 127.
/// Pitch bend is the 14-bit value 0 to 16383, where 8192 is the centre.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MidiEvent {
    /// A key was released. A note-on with velocity 0 also parses to this.
    NoteOff { channel: u8, key: u8, velocity: u8 },
    /// A key was pressed with a velocity from 1 to 127.
    NoteOn { channel: u8, key: u8, velocity: u8 },
    /// Pressure applied to one key after it was pressed.
    PolyPressure { channel: u8, key: u8, pressure: u8 },
    /// A controller changed, for example the sustain pedal (controller 64).
    ControlChange { channel: u8, controller: u8, value: u8 },
    /// The instrument or patch was changed.
    ProgramChange { channel: u8, program: u8 },
    /// Pressure applied to the whole channel.
    ChannelPressure { channel: u8, pressure: u8 },
    /// The pitch wheel moved. 8192 is centred, 0 is fully down, 16383 is fully up.
    PitchBend { channel: u8, value: u16 },
}

/// The controller number of the sustain pedal.
pub const CONTROLLER_SUSTAIN: u8 = 64;
/// The controller number that silences every note on the channel.
pub const CONTROLLER_ALL_NOTES_OFF: u8 = 123;
/// The pitch bend centre value.
pub const PITCH_BEND_CENTRE: u16 = 8192;

/// Parses a MIDI 1.0 byte stream into events. Running status is kept across messages, real-time bytes
/// (0xF8 to 0xFF) are skipped without ending a message, and system messages and system exclusive data
/// are skipped and clear running status.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MidiParser {
    status: u8,
    data: [u8; 2],
    filled: usize,
    sysex: bool,
}

impl Default for MidiParser {
    fn default() -> Self {
        Self::new()
    }
}

impl MidiParser {
    /// A parser with no running status.
    pub fn new() -> Self {
        MidiParser { status: 0, data: [0; 2], filled: 0, sysex: false }
    }

    /// Feeds one byte. Returns an event when this byte completes a channel message.
    pub fn push(&mut self, byte: u8) -> Option<MidiEvent> {
        // Real-time bytes may appear anywhere and never end a message or running status.
        if byte >= 0xF8 {
            return None;
        }
        if self.sysex {
            if byte == 0xF7 {
                self.sysex = false;
                return None;
            }
            if byte < 0x80 {
                return None;
            }
            // Any other status byte ends the exclusive message and is handled below.
            self.sysex = false;
        }
        if byte >= 0x80 {
            if byte == 0xF0 {
                self.status = 0;
                self.sysex = true;
                return None;
            }
            if byte >= 0xF0 {
                // System common messages clear running status and their data bytes are dropped.
                self.status = 0;
                return None;
            }
            self.status = byte;
            self.filled = 0;
            return None;
        }
        // A data byte with no running status belongs to nothing.
        if self.status == 0 {
            return None;
        }
        self.data[self.filled] = byte;
        self.filled += 1;
        if self.filled < data_length(self.status) {
            return None;
        }
        self.filled = 0;
        Some(build(self.status, self.data))
    }

    /// Parses a whole slice with a fresh parser and returns every event in order.
    pub fn parse_all(bytes: &[u8]) -> Vec<MidiEvent> {
        let mut parser = MidiParser::new();
        bytes.iter().filter_map(|&b| parser.push(b)).collect()
    }
}

/// The number of data bytes a channel status byte takes.
fn data_length(status: u8) -> usize {
    match status & 0xF0 {
        0xC0 | 0xD0 => 1,
        _ => 2,
    }
}

/// Builds the event for a complete channel status and its data bytes.
fn build(status: u8, data: [u8; 2]) -> MidiEvent {
    let channel = status & 0x0F;
    match status & 0xF0 {
        0x80 => MidiEvent::NoteOff { channel, key: data[0], velocity: data[1] },
        0x90 if data[1] == 0 => MidiEvent::NoteOff { channel, key: data[0], velocity: 0 },
        0x90 => MidiEvent::NoteOn { channel, key: data[0], velocity: data[1] },
        0xA0 => MidiEvent::PolyPressure { channel, key: data[0], pressure: data[1] },
        0xB0 => MidiEvent::ControlChange { channel, controller: data[0], value: data[1] },
        0xC0 => MidiEvent::ProgramChange { channel, program: data[0] },
        0xD0 => MidiEvent::ChannelPressure { channel, pressure: data[0] },
        _ => MidiEvent::PitchBend {
            channel,
            value: u16::from(data[0]) | (u16::from(data[1]) << 7),
        },
    }
}

/// Encodes an event as wire bytes. Returns the bytes and how many of them are used (2 or 3). Channels
/// are masked to 4 bits and data values to 7 bits, so out-of-range input cannot corrupt the stream.
pub fn encode(event: MidiEvent) -> ([u8; 3], usize) {
    let seven = |v: u8| v & 0x7F;
    match event {
        MidiEvent::NoteOff { channel, key, velocity } => {
            ([0x80 | (channel & 0x0F), seven(key), seven(velocity)], 3)
        }
        MidiEvent::NoteOn { channel, key, velocity } => {
            ([0x90 | (channel & 0x0F), seven(key), seven(velocity)], 3)
        }
        MidiEvent::PolyPressure { channel, key, pressure } => {
            ([0xA0 | (channel & 0x0F), seven(key), seven(pressure)], 3)
        }
        MidiEvent::ControlChange { channel, controller, value } => {
            ([0xB0 | (channel & 0x0F), seven(controller), seven(value)], 3)
        }
        MidiEvent::ProgramChange { channel, program } => {
            ([0xC0 | (channel & 0x0F), seven(program), 0], 2)
        }
        MidiEvent::ChannelPressure { channel, pressure } => {
            ([0xD0 | (channel & 0x0F), seven(pressure), 0], 2)
        }
        MidiEvent::PitchBend { channel, value } => {
            let v = value & 0x3FFF;
            ([0xE0 | (channel & 0x0F), (v & 0x7F) as u8, (v >> 7) as u8], 3)
        }
    }
}

/// The frequency of a key in hertz on equal temperament, with key 69 at 440 Hz.
pub fn key_frequency(key: u8) -> f64 {
    440.0 * 2f64.powf((f64::from(key) - 69.0) / 12.0)
}

/// Maps a velocity from 0 to 127 onto a level from 0.0 to 1.0.
pub fn velocity_level(velocity: u8) -> f32 {
    f32::from(velocity.min(127)) / 127.0
}

/// Tracks which keys are held or sustained on each of the 16 channels.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NoteTracker {
    /// Velocity of each held key, 0 when the key is not held.
    held: [[u8; 128]; 16],
    /// Keys released while the pedal was down and still ringing.
    sustained: [[bool; 128]; 16],
    /// Whether the sustain pedal is down on each channel.
    pedal: [bool; 16],
}

impl Default for NoteTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl NoteTracker {
    /// A tracker with no keys down and both pedals up.
    pub fn new() -> Self {
        NoteTracker { held: [[0; 128]; 16], sustained: [[false; 128]; 16], pedal: [false; 16] }
    }

    /// Updates the tracker from one event. Other messages do not change note state.
    pub fn apply(&mut self, event: MidiEvent) {
        match event {
            MidiEvent::NoteOn { channel, key, velocity } => {
                let (c, k) = slot(channel, key);
                self.held[c][k] = velocity.clamp(1, 127);
                self.sustained[c][k] = false;
            }
            MidiEvent::NoteOff { channel, key, .. } => {
                let (c, k) = slot(channel, key);
                if self.pedal[c] {
                    if self.held[c][k] != 0 {
                        self.sustained[c][k] = true;
                        self.held[c][k] = 0;
                    }
                } else {
                    self.held[c][k] = 0;
                    self.sustained[c][k] = false;
                }
            }
            MidiEvent::ControlChange { channel, controller, value } => {
                let c = usize::from(channel & 0x0F);
                if controller == CONTROLLER_SUSTAIN {
                    self.pedal[c] = value >= 64;
                    if !self.pedal[c] {
                        self.sustained[c] = [false; 128];
                    }
                } else if controller == CONTROLLER_ALL_NOTES_OFF {
                    self.held[c] = [0; 128];
                    self.sustained[c] = [false; 128];
                }
            }
            _ => {}
        }
    }

    /// The velocity a key is held at, or None when it is not held (sustained keys are not held).
    pub fn held_velocity(&self, channel: u8, key: u8) -> Option<u8> {
        let (c, k) = slot(channel, key);
        match self.held[c][k] {
            0 => None,
            v => Some(v),
        }
    }

    /// Whether a key is held or still ringing under the pedal.
    pub fn is_sounding(&self, channel: u8, key: u8) -> bool {
        let (c, k) = slot(channel, key);
        self.held[c][k] != 0 || self.sustained[c][k]
    }

    /// Whether the sustain pedal is down on a channel.
    pub fn pedal_down(&self, channel: u8) -> bool {
        self.pedal[usize::from(channel & 0x0F)]
    }

    /// The number of keys that are held or sustained, across all channels.
    pub fn sounding_count(&self) -> usize {
        let mut count = 0;
        for c in 0..16 {
            for k in 0..128 {
                if self.held[c][k] != 0 || self.sustained[c][k] {
                    count += 1;
                }
            }
        }
        count
    }
}

/// The array indices of a channel and key, with both masked into range.
fn slot(channel: u8, key: u8) -> (usize, usize) {
    (usize::from(channel & 0x0F), usize::from(key & 0x7F))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_note_on_and_zero_velocity_as_note_off() {
        // 0x90 is note-on on channel 0. Velocity 0 means note-off.
        let events = MidiParser::parse_all(&[0x90, 60, 100, 0x90, 60, 0]);
        assert_eq!(
            events,
            vec![
                MidiEvent::NoteOn { channel: 0, key: 60, velocity: 100 },
                MidiEvent::NoteOff { channel: 0, key: 60, velocity: 0 },
            ]
        );
    }

    #[test]
    fn running_status_repeats_the_last_status_byte() {
        // The second note-on has no status byte of its own.
        let events = MidiParser::parse_all(&[0x91, 60, 100, 62, 90]);
        assert_eq!(
            events,
            vec![
                MidiEvent::NoteOn { channel: 1, key: 60, velocity: 100 },
                MidiEvent::NoteOn { channel: 1, key: 62, velocity: 90 },
            ]
        );
    }

    #[test]
    fn real_time_byte_inside_a_message_is_skipped() {
        // 0xF8 is a clock tick. It arrives between the status and its data without ending the message.
        let events = MidiParser::parse_all(&[0x90, 0xF8, 60, 100]);
        assert_eq!(events, vec![MidiEvent::NoteOn { channel: 0, key: 60, velocity: 100 }]);
    }

    #[test]
    fn system_common_clears_running_status_and_its_data_is_dropped() {
        // 0xF1 (time code) has one data byte. Without running status the following bytes do nothing.
        let events = MidiParser::parse_all(&[0x90, 60, 100, 0xF1, 0x20, 62, 90]);
        assert_eq!(events, vec![MidiEvent::NoteOn { channel: 0, key: 60, velocity: 100 }]);
    }

    #[test]
    fn system_exclusive_data_is_skipped_until_the_end_byte() {
        // Three data bytes inside 0xF0 ... 0xF7 produce nothing; the note after the end byte does.
        let events = MidiParser::parse_all(&[0xF0, 0x01, 0x02, 0x03, 0xF7, 0x90, 60, 100]);
        assert_eq!(events, vec![MidiEvent::NoteOn { channel: 0, key: 60, velocity: 100 }]);
    }

    #[test]
    fn hostile_bytes_never_panic_or_produce_events_from_nothing() {
        // F7 with no F0 clears running status.
        assert!(MidiParser::parse_all(&[0x90, 60, 100, 0xF7, 62, 90]).len() == 1);
        // Truncated sysex followed by a channel status: the status ends the sysex.
        let ev = MidiParser::parse_all(&[0xF0, 1, 2, 3, 0x91, 60, 100]);
        assert_eq!(ev, vec![MidiEvent::NoteOn { channel: 1, key: 60, velocity: 100 }]);
        // Real-time bytes inside sysex, including 0xFF, do not end it; the data after them is still skipped.
        let ev = MidiParser::parse_all(&[0xF0, 1, 0xF8, 0xFF, 2, 0xFE, 0xF7, 0x90, 60, 100]);
        assert_eq!(ev.len(), 1);
        // Data bytes with no status give nothing.
        assert!(MidiParser::parse_all(&[1, 2, 3, 4]).is_empty());
        // A status interrupts an incomplete message: the first note-on is dropped.
        let ev = MidiParser::parse_all(&[0x90, 60, 0x80, 61, 0]);
        assert_eq!(ev, vec![MidiEvent::NoteOff { channel: 0, key: 61, velocity: 0 }]);
        // Running status survives real-time bytes between messages.
        let ev = MidiParser::parse_all(&[0x90, 60, 100, 0xF8, 61, 100]);
        assert_eq!(ev.len(), 2);
        // Every possible byte pair after every status is handled.
        for status in 0x80u8..=0xFF {
            for a in 0..=255u8 {
                let _ = MidiParser::parse_all(&[status, a, a, a, a]);
            }
        }
    }

    #[test]
    fn pitch_bend_joins_its_two_seven_bit_bytes() {
        // Least significant byte 0x00, most significant 0x40: 0x40 << 7 = 8192, the centre.
        let events = MidiParser::parse_all(&[0xE1, 0x00, 0x40]);
        assert_eq!(events, vec![MidiEvent::PitchBend { channel: 1, value: PITCH_BEND_CENTRE }]);
    }

    #[test]
    fn encode_writes_the_status_nibble_and_the_channel() {
        let (bytes, len) = encode(MidiEvent::NoteOn { channel: 3, key: 64, velocity: 80 });
        assert_eq!((bytes, len), ([0x93, 64, 80], 3));
        // Pitch bend 16383 splits into LSB 0x7F and MSB 0x7F.
        let (bytes, len) = encode(MidiEvent::PitchBend { channel: 0, value: 16383 });
        assert_eq!((bytes, len), ([0xE0, 0x7F, 0x7F], 3));
        // Program change is two bytes.
        let (bytes, len) = encode(MidiEvent::ProgramChange { channel: 2, program: 5 });
        assert_eq!((bytes[0], bytes[1], len), (0xC2, 5, 2));
    }

    #[test]
    fn encode_then_parse_round_trips() {
        let original = MidiEvent::ControlChange { channel: 9, controller: 64, value: 127 };
        let (bytes, len) = encode(original);
        assert_eq!(MidiParser::parse_all(&bytes[..len]), vec![original]);
    }

    #[test]
    fn key_frequency_and_velocity_level_follow_equal_temperament() {
        assert!((key_frequency(69) - 440.0).abs() < 1e-9);
        assert!((key_frequency(57) - 220.0).abs() < 1e-9);
        assert!((key_frequency(81) - 880.0).abs() < 1e-9);
        assert!((velocity_level(127) - 1.0).abs() < 1e-6);
        assert!((velocity_level(0) - 0.0).abs() < 1e-6);
    }

    #[test]
    fn sustain_pedal_keeps_released_keys_ringing_until_it_lifts() {
        let mut tracker = NoteTracker::new();
        tracker.apply(MidiEvent::NoteOn { channel: 0, key: 60, velocity: 90 });
        tracker.apply(MidiEvent::ControlChange { channel: 0, controller: 64, value: 127 });
        tracker.apply(MidiEvent::NoteOff { channel: 0, key: 60, velocity: 0 });
        // Released but sustained: not held, still sounding.
        assert_eq!(tracker.held_velocity(0, 60), None);
        assert!(tracker.is_sounding(0, 60));
        assert_eq!(tracker.sounding_count(), 1);
        // Pedal up lets it go.
        tracker.apply(MidiEvent::ControlChange { channel: 0, controller: 64, value: 0 });
        assert!(!tracker.is_sounding(0, 60));
        assert_eq!(tracker.sounding_count(), 0);
    }

    #[test]
    fn note_off_without_pedal_stops_at_once_and_note_on_retriggers() {
        let mut tracker = NoteTracker::new();
        tracker.apply(MidiEvent::NoteOn { channel: 2, key: 48, velocity: 70 });
        assert_eq!(tracker.held_velocity(2, 48), Some(70));
        tracker.apply(MidiEvent::NoteOff { channel: 2, key: 48, velocity: 0 });
        assert!(!tracker.is_sounding(2, 48));
        // A key held under the pedal is replaced by a fresh press, not kept as a sustained copy.
        tracker.apply(MidiEvent::ControlChange { channel: 2, controller: 64, value: 127 });
        tracker.apply(MidiEvent::NoteOn { channel: 2, key: 50, velocity: 60 });
        tracker.apply(MidiEvent::NoteOff { channel: 2, key: 50, velocity: 0 });
        tracker.apply(MidiEvent::NoteOn { channel: 2, key: 50, velocity: 100 });
        assert_eq!(tracker.held_velocity(2, 50), Some(100));
        assert_eq!(tracker.sounding_count(), 1);
    }

    #[test]
    fn all_notes_off_clears_one_channel_only() {
        let mut tracker = NoteTracker::new();
        tracker.apply(MidiEvent::NoteOn { channel: 0, key: 60, velocity: 90 });
        tracker.apply(MidiEvent::NoteOn { channel: 1, key: 60, velocity: 90 });
        tracker.apply(MidiEvent::ControlChange { channel: 0, controller: 123, value: 0 });
        assert!(!tracker.is_sounding(0, 60));
        assert!(tracker.is_sounding(1, 60));
        assert_eq!(tracker.sounding_count(), 1);
    }
}
