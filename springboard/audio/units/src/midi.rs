//! MIDI input for units: a short message at a frame offset inside the block. Wire packing puts one message in a
//! `u32` (used by the out-of-process transport).

/// A channel-voice message at `offset` frames into the block.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct MidiEvent {
    pub offset: u32,
    pub status: u8,
    pub data1: u8,
    pub data2: u8,
}

impl MidiEvent {
    pub fn note_on(offset: u32, channel: u8, note: u8, velocity: u8) -> MidiEvent {
        MidiEvent { offset, status: 0x90 | (channel & 15), data1: note & 127, data2: velocity & 127 }
    }

    pub fn note_off(offset: u32, channel: u8, note: u8) -> MidiEvent {
        MidiEvent { offset, status: 0x80 | (channel & 15), data1: note & 127, data2: 0 }
    }

    pub fn control(offset: u32, channel: u8, controller: u8, value: u8) -> MidiEvent {
        MidiEvent { offset, status: 0xB0 | (channel & 15), data1: controller & 127, data2: value & 127 }
    }

    pub fn channel(&self) -> u8 {
        self.status & 15
    }

    /// A note-on with velocity 0 is a note-off.
    pub fn is_note_on(&self) -> bool {
        self.status & 0xF0 == 0x90 && self.data2 != 0
    }

    pub fn is_note_off(&self) -> bool {
        self.status & 0xF0 == 0x80 || (self.status & 0xF0 == 0x90 && self.data2 == 0)
    }

    /// Status in bits 0..8, data1 in 8..16, data2 in 16..24, bit 24 set (so an empty slot is 0).
    pub fn pack(&self) -> u32 {
        self.status as u32 | (self.data1 as u32) << 8 | (self.data2 as u32) << 16 | 1 << 24
    }

    /// `None` for an empty slot; the offset comes from the slot's position.
    pub fn unpack(word: u32, offset: u32) -> Option<MidiEvent> {
        if word & (1 << 24) == 0 {
            return None;
        }
        Some(MidiEvent { offset, status: word as u8, data1: (word >> 8) as u8, data2: (word >> 16) as u8 })
    }
}

/// Equal-tempered frequency of a MIDI note (note 69 = 440 Hz).
pub fn note_to_hz(note: u8) -> f64 {
    440.0 * 2f64.powf((note as f64 - 69.0) / 12.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constructors_set_status_bytes() {
        let e = MidiEvent::note_on(5, 3, 60, 100);
        assert_eq!((e.status, e.data1, e.data2, e.offset), (0x93, 60, 100, 5));
        assert!(e.is_note_on() && !e.is_note_off());
        assert_eq!(e.channel(), 3);
        assert_eq!(MidiEvent::note_off(0, 0, 60).status, 0x80);
        assert_eq!(MidiEvent::control(0, 1, 7, 127).status, 0xB1);
    }

    #[test]
    fn zero_velocity_note_on_is_off() {
        let e = MidiEvent::note_on(0, 0, 60, 0);
        assert!(e.is_note_off() && !e.is_note_on());
    }

    #[test]
    fn pack_round_trips_and_empty_is_zero() {
        let e = MidiEvent::note_on(9, 2, 64, 90);
        assert_eq!(e.pack(), 0x01_5A_40_92);
        assert_eq!(MidiEvent::unpack(e.pack(), 9), Some(e));
        assert_eq!(MidiEvent::unpack(0, 4), None);
    }

    #[test]
    fn note_frequencies() {
        assert_eq!(note_to_hz(69), 440.0);
        assert_eq!(note_to_hz(81), 880.0);
        assert_eq!(note_to_hz(57), 220.0);
    }
}
