//! Universal MIDI Packet (MIDI 2.0) words inside, MIDI 1.0 bytes at the edges.
//!
//! A packet is 1, 2, 3 or 4 32-bit words; the top nibble of word 0 is the message type (0 utility, 1 system,
//! 2 MIDI 1.0 channel voice, 3 64-bit data (7-bit system exclusive), 4 MIDI 2.0 channel voice, 5 128-bit data).
//! Velocities and controller values scale between 7, 16 and 32 bits with the min-centre-max rule of the MIDI 2.0
//! specification, so the centre stays the centre and the maximum stays the maximum.

use crate::events::MidiEvent;

/// One UMP packet: up to four words, `len` of them used.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Ump {
    pub words: [u32; 4],
    pub len: u8,
}

impl Ump {
    pub fn one(w0: u32) -> Ump {
        Ump { words: [w0, 0, 0, 0], len: 1 }
    }
    pub fn two(w0: u32, w1: u32) -> Ump {
        Ump { words: [w0, w1, 0, 0], len: 2 }
    }
    /// The message type nibble.
    pub fn message_type(&self) -> u8 {
        (self.words[0] >> 28) as u8
    }
    pub fn group(&self) -> u8 {
        ((self.words[0] >> 24) & 0xF) as u8
    }
    pub fn as_words(&self) -> &[u32] {
        &self.words[..self.len as usize]
    }
}

/// The number of words of a packet of message type `mt`.
pub fn words_for_type(mt: u8) -> usize {
    match mt {
        0..=2 | 6 | 7 => 1,
        3 | 4 | 8..=0xA => 2,
        0xB | 0xC => 3,
        _ => 4,
    }
}

/// Scales `v` of `src` bits up to `dst` bits (min-centre-max).
pub fn scale_up(v: u32, src: u32, dst: u32) -> u32 {
    let mut shift = (dst - src) as i32;
    let mut scaled = (v as u64) << shift;
    let centre = 1u32 << (src - 1);
    if v <= centre {
        return scaled as u32;
    }
    let repeat_bits = (src - 1) as i32;
    let mut repeat = (v & ((1 << repeat_bits) - 1)) as u64;
    if repeat_bits > shift {
        repeat >>= repeat_bits - shift;
    }
    while shift > 0 {
        if shift >= repeat_bits {
            scaled |= repeat << (shift - repeat_bits);
        } else {
            scaled |= repeat >> (repeat_bits - shift);
        }
        shift -= repeat_bits;
    }
    scaled as u32
}

/// Scales `v` of `src` bits down to `dst` bits by dropping the low bits.
pub fn scale_down(v: u32, src: u32, dst: u32) -> u32 {
    v >> (src - dst)
}

/// A MIDI 1.0 channel voice event as one type 2 packet.
pub fn from_midi1(group: u8, event: MidiEvent) -> Ump {
    let (bytes, n) = crate::events::encode(event);
    let d2 = if n == 3 { bytes[2] as u32 } else { 0 };
    let w = (2u32 << 28)
        | (((group & 0xF) as u32) << 24)
        | ((bytes[0] as u32) << 16)
        | ((bytes[1] as u32) << 8)
        | d2;
    Ump::one(w)
}

/// A type 2 packet back to a channel voice event.
pub fn to_midi1(p: &Ump) -> Option<MidiEvent> {
    if p.message_type() != 2 {
        return None;
    }
    let w = p.words[0];
    let st = ((w >> 16) & 0xFF) as u8;
    let d1 = ((w >> 8) & 0x7F) as u8;
    let d2 = (w & 0x7F) as u8;
    let bytes = [st, d1, d2];
    let n = if matches!(st & 0xF0, 0xC0 | 0xD0) { 2 } else { 3 };
    let mut parser = crate::events::MidiParser::new();
    bytes[..n].iter().filter_map(|&b| parser.push(b)).next()
}

/// Upgrades a MIDI 1.0 event to a MIDI 2.0 channel voice packet (type 4).
pub fn midi1_to_midi2(group: u8, event: MidiEvent) -> Ump {
    let g = ((group & 0xF) as u32) << 24;
    let head = |op: u32, ch: u8, b1: u8, b2: u8| {
        (4u32 << 28) | g | (op << 20) | (((ch & 0xF) as u32) << 16) | ((b1 as u32) << 8) | b2 as u32
    };
    match event {
        MidiEvent::NoteOff { channel, key, velocity } => {
            Ump::two(head(0x8, channel, key, 0), scale_up(velocity as u32, 7, 16) << 16)
        }
        MidiEvent::NoteOn { channel, key, velocity } => {
            Ump::two(head(0x9, channel, key, 0), scale_up(velocity as u32, 7, 16) << 16)
        }
        MidiEvent::PolyPressure { channel, key, pressure } => {
            Ump::two(head(0xA, channel, key, 0), scale_up(pressure as u32, 7, 32))
        }
        MidiEvent::ControlChange { channel, controller, value } => {
            Ump::two(head(0xB, channel, controller, 0), scale_up(value as u32, 7, 32))
        }
        MidiEvent::ProgramChange { channel, program } => {
            Ump::two(head(0xC, channel, 0, 0), (program as u32) << 24)
        }
        MidiEvent::ChannelPressure { channel, pressure } => {
            Ump::two(head(0xD, channel, 0, 0), scale_up(pressure as u32, 7, 32))
        }
        MidiEvent::PitchBend { channel, value } => {
            Ump::two(head(0xE, channel, 0, 0), scale_up((value & 0x3FFF) as u32, 14, 32))
        }
    }
}

/// Downgrades a type 4 packet to a MIDI 1.0 event. A note-on that scales to velocity 0 becomes velocity 1 so it
/// stays a note-on. Per-note controllers and registered controllers have no MIDI 1.0 form here and give `None`.
pub fn midi2_to_midi1(p: &Ump) -> Option<MidiEvent> {
    if p.message_type() != 4 {
        return None;
    }
    let w0 = p.words[0];
    let w1 = p.words[1];
    let op = ((w0 >> 20) & 0xF) as u8;
    let channel = ((w0 >> 16) & 0xF) as u8;
    let b1 = ((w0 >> 8) & 0x7F) as u8;
    let s7 = |v: u32| scale_down(v, 32, 7) as u8;
    Some(match op {
        0x8 => MidiEvent::NoteOff { channel, key: b1, velocity: scale_down(w1 >> 16, 16, 7) as u8 },
        0x9 => MidiEvent::NoteOn { channel, key: b1, velocity: (scale_down(w1 >> 16, 16, 7) as u8).max(1) },
        0xA => MidiEvent::PolyPressure { channel, key: b1, pressure: s7(w1) },
        0xB => MidiEvent::ControlChange { channel, controller: b1, value: s7(w1) },
        0xC => MidiEvent::ProgramChange { channel, program: (w1 >> 24) as u8 & 0x7F },
        0xD => MidiEvent::ChannelPressure { channel, pressure: s7(w1) },
        0xE => MidiEvent::PitchBend { channel, value: scale_down(w1, 32, 14) as u16 },
        _ => return None,
    })
}

/// 7-bit system exclusive payload (without F0 and F7) as type 3 packets, 6 bytes per packet.
pub fn sysex_to_ump(group: u8, data: &[u8]) -> Vec<Ump> {
    let chunks: Vec<&[u8]> = if data.is_empty() { vec![&[][..]] } else { data.chunks(6).collect() };
    let last = chunks.len() - 1;
    chunks
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let status = match (i == 0, i == last) {
                (true, true) => 0u32,
                (true, false) => 1,
                (false, true) => 3,
                (false, false) => 2,
            };
            let mut b = [0u8; 6];
            b[..c.len()].copy_from_slice(c);
            let w0 = (3u32 << 28)
                | (((group & 0xF) as u32) << 24)
                | (status << 20)
                | ((c.len() as u32) << 16)
                | ((b[0] as u32) << 8)
                | b[1] as u32;
            let w1 = u32::from_be_bytes([b[2], b[3], b[4], b[5]]);
            Ump::two(w0, w1)
        })
        .collect()
}

/// Collects type 3 packets into whole system exclusive payloads.
#[derive(Default, Debug)]
pub struct SysexAssembler {
    buf: Vec<u8>,
}

impl SysexAssembler {
    pub fn new() -> Self {
        Self::default()
    }
    /// Feeds one packet; returns the payload when this packet completes a message.
    pub fn push(&mut self, p: &Ump) -> Option<Vec<u8>> {
        if p.message_type() != 3 {
            return None;
        }
        let status = (p.words[0] >> 20) & 0xF;
        let n = (((p.words[0] >> 16) & 0xF) as usize).min(6);
        let b = [
            (p.words[0] >> 8) as u8,
            p.words[0] as u8,
            (p.words[1] >> 24) as u8,
            (p.words[1] >> 16) as u8,
            (p.words[1] >> 8) as u8,
            p.words[1] as u8,
        ];
        if status == 0 || status == 1 {
            self.buf.clear();
        }
        self.buf.extend_from_slice(&b[..n]);
        if status == 0 || status == 3 {
            Some(std::mem::take(&mut self.buf))
        } else {
            None
        }
    }
}

/// Converts a MIDI 1.0 byte stream (running status, real-time bytes anywhere, system exclusive, system common)
/// into type 1, 2 and 3 packets in one group.
pub struct Midi1ToUmp {
    group: u8,
    status: u8,
    data: [u8; 2],
    filled: usize,
    sysex: Option<Vec<u8>>,
}

impl Midi1ToUmp {
    pub fn new(group: u8) -> Self {
        Midi1ToUmp { group: group & 0xF, status: 0, data: [0; 2], filled: 0, sysex: None }
    }

    fn sys(&self, st: u8, d1: u8, d2: u8) -> Ump {
        Ump::one((1u32 << 28) | ((self.group as u32) << 24) | ((st as u32) << 16) | ((d1 as u32) << 8) | d2 as u32)
    }

    /// Feeds one byte; completed packets are appended to `out`.
    pub fn push(&mut self, byte: u8, out: &mut Vec<Ump>) {
        if byte >= 0xF8 {
            out.push(self.sys(byte, 0, 0));
            return;
        }
        if byte >= 0x80 {
            if let Some(s) = self.sysex.take() {
                // F7 ends the message; any other status truncates it. Either way what arrived is delivered.
                out.extend(sysex_to_ump(self.group, &s));
                if byte == 0xF7 {
                    return;
                }
            }
            self.filled = 0;
            match byte {
                0xF0 => {
                    self.status = 0;
                    self.sysex = Some(Vec::new());
                }
                0xF6 => {
                    self.status = 0;
                    out.push(self.sys(0xF6, 0, 0));
                }
                0xF7 => self.status = 0,
                _ => self.status = byte,
            }
            return;
        }
        if let Some(s) = self.sysex.as_mut() {
            s.push(byte);
            return;
        }
        if self.status == 0 {
            return;
        }
        let need = match self.status {
            0xF1 | 0xF3 => 1,
            0xF2 => 2,
            0xF4 | 0xF5 => return,
            s if matches!(s & 0xF0, 0xC0 | 0xD0) => 1,
            _ => 2,
        };
        self.data[self.filled] = byte;
        self.filled += 1;
        if self.filled < need {
            return;
        }
        self.filled = 0;
        let st = self.status;
        if st >= 0xF0 {
            out.push(self.sys(st, self.data[0], if need == 2 { self.data[1] } else { 0 }));
            self.status = 0;
        } else {
            let d2 = if need == 2 { self.data[1] as u32 } else { 0 };
            out.push(Ump::one(
                (2u32 << 28) | ((self.group as u32) << 24) | ((st as u32) << 16) | ((self.data[0] as u32) << 8) | d2,
            ));
        }
    }
}

/// Writes a type 1 or type 2 packet as MIDI 1.0 wire bytes. Other types give `None`.
pub fn ump_to_bytes(p: &Ump) -> Option<Vec<u8>> {
    let w = p.words[0];
    let st = ((w >> 16) & 0xFF) as u8;
    let d1 = ((w >> 8) & 0x7F) as u8;
    let d2 = (w & 0x7F) as u8;
    match p.message_type() {
        1 => Some(match st {
            0xF1 | 0xF3 => vec![st, d1],
            0xF2 => vec![st, d1, d2],
            _ => vec![st],
        }),
        2 => Some(if matches!(st & 0xF0, 0xC0 | 0xD0) { vec![st, d1] } else { vec![st, d1, d2] }),
        _ => None,
    }
}

/// A system exclusive payload as wire bytes with F0 and F7.
pub fn sysex_bytes(payload: &[u8]) -> Vec<u8> {
    let mut v = Vec::with_capacity(payload.len() + 2);
    v.push(0xF0);
    v.extend_from_slice(payload);
    v.push(0xF7);
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scaling_keeps_min_centre_max() {
        assert_eq!(scale_up(0, 7, 16), 0);
        assert_eq!(scale_up(64, 7, 16), 0x8000);
        assert_eq!(scale_up(127, 7, 16), 0xFFFF);
        assert_eq!(scale_up(1, 7, 16), 0x0200);
        assert_eq!(scale_up(127, 7, 32), 0xFFFF_FFFF);
        assert_eq!(scale_up(64, 7, 32), 0x8000_0000);
        assert_eq!(scale_up(0x3FFF, 14, 32), 0xFFFF_FFFF);
        assert_eq!(scale_up(8192, 14, 32), 0x8000_0000);
        assert_eq!(scale_down(0xFFFF, 16, 7), 127);
    }

    #[test]
    fn midi1_packet_words() {
        // type 2, group 1, status 0x92, key 60, velocity 100.
        let p = from_midi1(1, MidiEvent::NoteOn { channel: 2, key: 60, velocity: 100 });
        assert_eq!(p.words[0], 0x2192_3C64);
        assert_eq!(to_midi1(&p), Some(MidiEvent::NoteOn { channel: 2, key: 60, velocity: 100 }));
        let pc = from_midi1(0, MidiEvent::ProgramChange { channel: 0, program: 5 });
        assert_eq!(pc.words[0], 0x20C0_0500);
    }

    #[test]
    fn midi2_note_on_words_and_round_trip() {
        let p = midi1_to_midi2(0, MidiEvent::NoteOn { channel: 3, key: 60, velocity: 127 });
        assert_eq!(p.words[0], 0x4093_3C00);
        assert_eq!(p.words[1], 0xFFFF_0000);
        assert_eq!(midi2_to_midi1(&p), Some(MidiEvent::NoteOn { channel: 3, key: 60, velocity: 127 }));
        for ev in [
            MidiEvent::NoteOff { channel: 1, key: 10, velocity: 33 },
            MidiEvent::ControlChange { channel: 4, controller: 7, value: 99 },
            MidiEvent::ProgramChange { channel: 9, program: 17 },
            MidiEvent::ChannelPressure { channel: 0, pressure: 1 },
            MidiEvent::PolyPressure { channel: 0, key: 5, pressure: 127 },
            MidiEvent::PitchBend { channel: 15, value: 12345 },
        ] {
            let up = midi1_to_midi2(5, ev);
            assert_eq!(up.group(), 5);
            assert_eq!(midi2_to_midi1(&up).unwrap(), ev, "{ev:?}");
        }
    }

    #[test]
    fn quiet_midi2_note_on_stays_a_note_on() {
        let p = Ump::two(0x4090_3C00, 0x0010_0000);
        assert_eq!(midi2_to_midi1(&p), Some(MidiEvent::NoteOn { channel: 0, key: 60, velocity: 1 }));
    }

    #[test]
    fn sysex_packets_split_by_six() {
        let data = [1u8, 2, 3, 4, 5, 6, 7, 8];
        let pk = sysex_to_ump(0, &data);
        assert_eq!(pk.len(), 2);
        assert_eq!(pk[0].words[0], 0x3016_0102); // start, 6 bytes
        assert_eq!(pk[0].words[1], 0x0304_0506);
        assert_eq!(pk[1].words[0], 0x3032_0708); // end, 2 bytes
        let mut a = SysexAssembler::new();
        assert_eq!(a.push(&pk[0]), None);
        assert_eq!(a.push(&pk[1]), Some(data.to_vec()));
        let small = sysex_to_ump(2, &[0x7E]);
        assert_eq!(small[0].words[0], 0x3201_7E00);
    }

    #[test]
    fn byte_stream_conversion() {
        let mut c = Midi1ToUmp::new(0);
        let mut out = Vec::new();
        for b in [0x90u8, 60, 100, 0xF8, 62, 90, 0xC1, 7, 0xF0, 0x7D, 1, 2, 0xF7, 0xF2, 1, 0] {
            c.push(b, &mut out);
        }
        assert_eq!(out[0].words[0], 0x2090_3C64);
        assert_eq!(out[1].words[0], 0x10F8_0000);
        assert_eq!(out[2].words[0], 0x2090_3E5A);
        assert_eq!(out[3].words[0], 0x20C1_0700);
        assert_eq!(out[4].message_type(), 3);
        assert_eq!(out[5].words[0], 0x10F2_0100);
        assert_eq!(ump_to_bytes(&out[2]), Some(vec![0x90, 62, 90]));
        assert_eq!(ump_to_bytes(&out[3]), Some(vec![0xC1, 7]));
        assert_eq!(ump_to_bytes(&out[5]), Some(vec![0xF2, 1, 0]));
        let mut a = SysexAssembler::new();
        assert_eq!(a.push(&out[4]), Some(vec![0x7D, 1, 2]));
        assert_eq!(sysex_bytes(&[0x7D, 1, 2]), vec![0xF0, 0x7D, 1, 2, 0xF7]);
    }

    #[test]
    fn packet_word_counts() {
        assert_eq!(words_for_type(2), 1);
        assert_eq!(words_for_type(4), 2);
        assert_eq!(words_for_type(5), 4);
    }
}
