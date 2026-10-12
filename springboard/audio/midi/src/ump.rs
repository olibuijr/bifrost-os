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

/// A one-word system packet (type 1): system common or real-time. Data bytes are masked to 7 bits.
pub fn system_packet(group: u8, status: u8, d1: u8, d2: u8) -> Ump {
    Ump::one(
        (1u32 << 28)
            | (((group & 0xF) as u32) << 24)
            | ((status as u32) << 16)
            | (((d1 & 0x7F) as u32) << 8)
            | (d2 & 0x7F) as u32,
    )
}

/// One type 3 packet: `status` 0 complete, 1 start, 2 continue, 3 end; at most 6 `bytes`, each masked to 7 bits.
fn sysex_packet(group: u8, status: u32, bytes: &[u8]) -> Ump {
    let n = bytes.len().min(6);
    let mut b = [0u8; 6];
    for (d, s) in b.iter_mut().zip(&bytes[..n]) {
        *d = s & 0x7F;
    }
    let w0 = (3u32 << 28)
        | (((group & 0xF) as u32) << 24)
        | (status << 20)
        | ((n as u32) << 16)
        | ((b[0] as u32) << 8)
        | b[1] as u32;
    Ump::two(w0, u32::from_be_bytes([b[2], b[3], b[4], b[5]]))
}

/// The six data bytes of a type 3 packet, before its length nibble is applied.
fn sysex_packet_bytes(p: &Ump) -> [u8; 6] {
    [
        (p.words[0] >> 8) as u8,
        p.words[0] as u8,
        (p.words[1] >> 24) as u8,
        (p.words[1] >> 16) as u8,
        (p.words[1] >> 8) as u8,
        p.words[1] as u8,
    ]
}

/// Packs a system exclusive payload into type 3 packets as its bytes arrive, so a message never waits for its end
/// and memory stays at one packet however long a sender runs. The last six bytes are held until the next byte (they
/// were a start or continue packet) or the end (an end or complete packet) says which kind their packet is.
#[derive(Clone, Copy, Debug, Default)]
struct SysexPacker {
    held: [u8; 6],
    n: usize,
    started: bool,
}

impl SysexPacker {
    fn push(&mut self, group: u8, byte: u8, out: &mut Vec<Ump>) {
        if self.n == 6 {
            out.push(sysex_packet(group, if self.started { 2 } else { 1 }, &self.held));
            self.started = true;
            self.n = 0;
        }
        self.held[self.n] = byte & 0x7F;
        self.n += 1;
    }

    fn finish(&self, group: u8, out: &mut Vec<Ump>) {
        out.push(sysex_packet(group, if self.started { 3 } else { 0 }, &self.held[..self.n]));
    }
}

/// A system exclusive payload (the bytes between F0 and F7) as type 3 packets, 6 bytes per packet. A byte with the
/// high bit set follows the MIDI 1.0 rule: a real-time byte (F8 to FF) becomes its own type 1 packet where it stands
/// and the message goes on; F7 ends the message; any other status ends it as truncated. What came before the end is
/// delivered; the bytes after it belong to no message here and are dropped.
pub fn sysex_to_ump(group: u8, data: &[u8]) -> Vec<Ump> {
    let mut out = Vec::new();
    let mut packer = SysexPacker::default();
    for &b in data {
        match b {
            0x00..=0x7F => packer.push(group, b, &mut out),
            0xF8..=0xFF => out.push(system_packet(group, b, 0, 0)),
            _ => break,
        }
    }
    packer.finish(group, &mut out);
    out
}

/// Collects type 3 packets into whole system exclusive payloads.
#[derive(Default, Debug)]
pub struct SysexAssembler {
    buf: Vec<u8>,
    /// True between a start packet and its end packet.
    active: bool,
    /// Real-time bytes found inside packets, oldest first, for `take_realtime`.
    realtime: Vec<u8>,
}

/// The most payload bytes one assembled message may hold. A longer message is dropped, so a peer that never
/// sends the end packet cannot grow the buffer without bound.
pub const SYSEX_MAX_BYTES: usize = 1 << 20;

/// The most real-time bytes the assembler keeps for `take_realtime`; later ones are dropped until those are taken.
pub const SYSEX_REALTIME_HELD: usize = 64;

impl SysexAssembler {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feeds one packet; returns the payload when this packet completes a message. Data bytes are 7-bit; a byte
    /// with the high bit set follows the MIDI 1.0 rule: a real-time byte is kept apart for `take_realtime` and the
    /// message goes on, any other status ends the message as truncated (what arrived before it is returned, and the
    /// continue and end packets after it belong to nothing).
    pub fn push(&mut self, p: &Ump) -> Option<Vec<u8>> {
        if p.message_type() != 3 {
            return None;
        }
        let status = (p.words[0] >> 20) & 0xF;
        let n = (((p.words[0] >> 16) & 0xF) as usize).min(6);
        let b = sysex_packet_bytes(p);
        match status {
            0 | 1 => {
                self.buf.clear();
                self.active = status == 1;
            }
            // A continue or end packet with no start before it (or after an overflow) belongs to nothing.
            2 | 3 if !self.active => return None,
            2 | 3 => {}
            // Status 4 to 15 are reserved.
            _ => return None,
        }
        if self.buf.len() + n > SYSEX_MAX_BYTES {
            self.buf.clear();
            self.active = false;
            return None;
        }
        let mut done = status == 0 || status == 3;
        for &x in &b[..n] {
            match x {
                0x00..=0x7F => self.buf.push(x),
                0xF8..=0xFF => {
                    if self.realtime.len() < SYSEX_REALTIME_HELD {
                        self.realtime.push(x);
                    }
                }
                _ => {
                    done = true;
                    break;
                }
            }
        }
        if done {
            self.active = false;
            Some(std::mem::take(&mut self.buf))
        } else {
            None
        }
    }

    /// The real-time bytes found inside packets since the last call, in order. Each is a message of its own.
    pub fn take_realtime(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.realtime)
    }
}

/// Converts a MIDI 1.0 byte stream (running status, real-time bytes anywhere, system exclusive, system common)
/// into type 1, 2 and 3 packets in one group. System exclusive is packed as it arrives (one packet per six bytes),
/// so an endless message from a device costs no memory.
pub struct Midi1ToUmp {
    group: u8,
    status: u8,
    data: [u8; 2],
    filled: usize,
    sysex: Option<SysexPacker>,
}

impl Midi1ToUmp {
    pub fn new(group: u8) -> Self {
        Midi1ToUmp { group: group & 0xF, status: 0, data: [0; 2], filled: 0, sysex: None }
    }

    /// Feeds one byte; completed packets are appended to `out`.
    pub fn push(&mut self, byte: u8, out: &mut Vec<Ump>) {
        let g = self.group;
        if byte >= 0xF8 {
            out.push(system_packet(g, byte, 0, 0));
            return;
        }
        if byte >= 0x80 {
            if let Some(s) = self.sysex.take() {
                // F7 ends the message; any other status truncates it. Either way what arrived is delivered.
                s.finish(g, out);
                if byte == 0xF7 {
                    return;
                }
            }
            self.filled = 0;
            match byte {
                0xF0 => {
                    self.status = 0;
                    self.sysex = Some(SysexPacker::default());
                }
                0xF6 => {
                    self.status = 0;
                    out.push(system_packet(g, 0xF6, 0, 0));
                }
                0xF7 => self.status = 0,
                _ => self.status = byte,
            }
            return;
        }
        if let Some(s) = self.sysex.as_mut() {
            s.push(g, byte, out);
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
            out.push(system_packet(g, st, self.data[0], if need == 2 { self.data[1] } else { 0 }));
            self.status = 0;
        } else {
            let d2 = if need == 2 { self.data[1] as u32 } else { 0 };
            out.push(Ump::one(
                (2u32 << 28) | ((g as u32) << 24) | ((st as u32) << 16) | ((self.data[0] as u32) << 8) | d2,
            ));
        }
    }
}

/// Writes a type 1 or type 2 packet as MIDI 1.0 wire bytes. Other types give `None`, and so does a packet whose
/// status byte would corrupt the wire: a type 2 status that is not a channel voice status, or a type 1 status that is
/// not system common (F1, F2, F3, F6) or real-time (F8 to FF). F0, F7, F4 and F5 never travel as type 1.
pub fn ump_to_bytes(p: &Ump) -> Option<Vec<u8>> {
    let w = p.words[0];
    let st = ((w >> 16) & 0xFF) as u8;
    let d1 = ((w >> 8) & 0x7F) as u8;
    let d2 = (w & 0x7F) as u8;
    match p.message_type() {
        1 => match st {
            0xF1 | 0xF3 => Some(vec![st, d1]),
            0xF2 => Some(vec![st, d1, d2]),
            0xF6 | 0xF8..=0xFF => Some(vec![st]),
            _ => None,
        },
        2 => match st & 0xF0 {
            0xC0 | 0xD0 => Some(vec![st, d1]),
            0x80..=0xE0 => Some(vec![st, d1, d2]),
            _ => None,
        },
        _ => None,
    }
}

/// Turns the packets of one group back into a MIDI 1.0 byte stream for a wire port (the playback edge): types 1 and
/// 2 as their bytes, type 3 as F0 ... F7 written as the packets come. A byte with the high bit set inside a type 3
/// packet follows the MIDI 1.0 rule: a real-time byte is written where it stands (legal inside system exclusive on
/// the wire), any other status ends the message as truncated with F7 and the rest of that message is dropped. A
/// channel or system common message arriving inside a message ends it the same way first; a real-time one does not.
#[derive(Debug, Default)]
pub struct UmpToMidi1 {
    in_sysex: bool,
}

impl UmpToMidi1 {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feeds one packet; its wire bytes are appended to `out`.
    pub fn push(&mut self, p: &Ump, out: &mut Vec<u8>) {
        match p.message_type() {
            1 | 2 => {
                let Some(bytes) = ump_to_bytes(p) else { return };
                if self.in_sysex && bytes[0] < 0xF8 {
                    out.push(0xF7);
                    self.in_sysex = false;
                }
                out.extend_from_slice(&bytes);
            }
            3 => {
                let status = (p.words[0] >> 20) & 0xF;
                match status {
                    0 | 1 => {
                        // A new start while a message is open truncates the open one.
                        if self.in_sysex {
                            out.push(0xF7);
                        }
                        out.push(0xF0);
                        self.in_sysex = true;
                    }
                    2 | 3 if self.in_sysex => {}
                    // An orphaned continue or end, or a reserved status.
                    _ => return,
                }
                let n = (((p.words[0] >> 16) & 0xF) as usize).min(6);
                for &x in &sysex_packet_bytes(p)[..n] {
                    if (0x80..0xF8).contains(&x) {
                        out.push(0xF7);
                        self.in_sysex = false;
                        return;
                    }
                    out.push(x);
                }
                if status == 0 || status == 3 {
                    out.push(0xF7);
                    self.in_sysex = false;
                }
            }
            _ => {}
        }
    }
}

/// A system exclusive payload as wire bytes with F0 and F7. Real-time bytes inside stay where they are; any other
/// status byte ends the payload there (truncated), so a payload can never put a status on the wire.
pub fn sysex_bytes(payload: &[u8]) -> Vec<u8> {
    let mut v = Vec::with_capacity(payload.len() + 2);
    v.push(0xF0);
    for &b in payload {
        if (0x80..0xF8).contains(&b) {
            break;
        }
        v.push(b);
    }
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
    fn sysex_assembler_survives_hostile_packets() {
        let pk = sysex_to_ump(0, &[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13]);
        assert_eq!(pk.len(), 3);
        let mut a = SysexAssembler::new();
        // An end or continue packet with no start gives nothing.
        assert_eq!(a.push(&pk[2]), None);
        assert_eq!(a.push(&pk[1]), None);
        // A new start restarts the message and discards the truncated one.
        assert_eq!(a.push(&pk[0]), None);
        assert_eq!(a.push(&pk[0]), None);
        assert_eq!(a.push(&pk[1]), None);
        assert_eq!(a.push(&pk[2]), Some((1..=13).collect::<Vec<u8>>()));
        // After the end the next end is orphaned again.
        assert_eq!(a.push(&pk[2]), None);
        // A reserved status is ignored, and a non-sysex packet too.
        assert_eq!(a.push(&Ump::two(0x3040_0000, 0)), None);
        assert_eq!(a.push(&Ump::one(0x2090_3C64)), None);
        // A length nibble above 6 is clamped to 6.
        assert_eq!(a.push(&Ump::two(0x300F_0102, 0x0304_0506)), Some(vec![1, 2, 3, 4, 5, 6]));
    }

    #[test]
    fn empty_sysex_round_trips() {
        let pk = sysex_to_ump(0, &[]);
        assert_eq!(pk.len(), 1);
        assert_eq!(pk[0].words[0], 0x3000_0000);
        assert_eq!(SysexAssembler::new().push(&pk[0]), Some(Vec::new()));
        // Exactly 6 bytes is one complete packet, 12 bytes is two.
        assert_eq!(sysex_to_ump(0, &[1; 6]).len(), 1);
        assert_eq!(sysex_to_ump(0, &[1; 12]).len(), 2);
    }

    #[test]
    fn byte_stream_hostile_sysex() {
        let run = |bytes: &[u8]| {
            let mut c = Midi1ToUmp::new(0);
            let mut out = Vec::new();
            for &b in bytes {
                c.push(b, &mut out);
            }
            out
        };
        // F7 without F0 produces nothing and clears running status.
        assert!(run(&[0xF7]).is_empty());
        assert!(run(&[0x90, 60, 100, 0xF7, 62, 90]).len() == 1);
        // A real-time byte inside sysex is delivered at once and the sysex continues.
        let out = run(&[0xF0, 1, 0xF8, 2, 0xF7]);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].words[0], 0x10F8_0000);
        assert_eq!(SysexAssembler::new().push(&out[1]), Some(vec![1, 2]));
        // A truncated sysex ended by a channel status is delivered, then the channel message parses.
        let out = run(&[0xF0, 1, 2, 0x90, 60, 100]);
        assert_eq!(out.len(), 2);
        assert_eq!(SysexAssembler::new().push(&out[0]), Some(vec![1, 2]));
        assert_eq!(out[1].words[0], 0x2090_3C64);
        // F0 F7 is an empty message.
        let out = run(&[0xF0, 0xF7]);
        assert_eq!(out.len(), 1);
        assert_eq!(SysexAssembler::new().push(&out[0]), Some(Vec::new()));
        // Data bytes with no status are dropped; undefined F4 swallows its data.
        assert!(run(&[1, 2, 3, 0xF4, 5]).is_empty());
        // Tune request is one byte.
        assert_eq!(run(&[0xF6]).len(), 1);
    }

    #[test]
    fn every_channel_message_round_trips_through_both_formats() {
        for ch in [0u8, 15] {
            for ev in [
                MidiEvent::NoteOff { channel: ch, key: 127, velocity: 0 },
                MidiEvent::NoteOn { channel: ch, key: 0, velocity: 1 },
                MidiEvent::PolyPressure { channel: ch, key: 64, pressure: 64 },
                MidiEvent::ControlChange { channel: ch, controller: 120, value: 127 },
                MidiEvent::ProgramChange { channel: ch, program: 127 },
                MidiEvent::ChannelPressure { channel: ch, pressure: 0 },
                MidiEvent::PitchBend { channel: ch, value: 0 },
                MidiEvent::PitchBend { channel: ch, value: 16383 },
                MidiEvent::PitchBend { channel: ch, value: 8192 },
            ] {
                let p1 = from_midi1(7, ev);
                assert_eq!(to_midi1(&p1), Some(ev), "{ev:?}");
                let bytes = ump_to_bytes(&p1).unwrap();
                assert_eq!(crate::events::MidiParser::parse_all(&bytes), vec![ev], "{ev:?}");
                let p2 = midi1_to_midi2(7, ev);
                assert_eq!(midi2_to_midi1(&p2), Some(ev), "{ev:?}");
            }
        }
        // Other message types give None.
        assert_eq!(to_midi1(&Ump::one(0x1000_0000)), None);
        assert_eq!(midi2_to_midi1(&Ump::one(0x2000_0000)), None);
        // Reserved MIDI 2.0 opcode gives None.
        assert_eq!(midi2_to_midi1(&Ump::two(0x40F0_0000, 0)), None);
    }

    #[test]
    fn packet_word_counts() {
        assert_eq!(words_for_type(2), 1);
        assert_eq!(words_for_type(4), 2);
        assert_eq!(words_for_type(5), 4);
    }

    #[test]
    fn sysex_payload_high_bit_bytes_follow_the_midi1_rule() {
        // A real-time byte inside the payload is its own packet and the message goes on.
        assert_eq!(sysex_to_ump(0, &[1, 0xF8, 2]), vec![Ump::one(0x10F8_0000), Ump::two(0x3002_0102, 0)]);
        // Any other status truncates: what came before is delivered, the rest is dropped.
        assert_eq!(sysex_to_ump(1, &[1, 2, 0x90, 60, 100]), vec![Ump::two(0x3102_0102, 0)]);
        // F7 inside ends the message.
        assert_eq!(sysex_to_ump(0, &[5, 0xF7, 6]), vec![Ump::two(0x3001_0500, 0)]);
        // A status first leaves an empty message.
        assert_eq!(sysex_to_ump(0, &[0xF0, 1]), vec![Ump::two(0x3000_0000, 0)]);
        // A real-time byte after the seventh byte lands between the start and the end packet.
        assert_eq!(
            sysex_to_ump(0, &[1, 2, 3, 4, 5, 6, 7, 0xFE, 8]),
            vec![Ump::two(0x3016_0102, 0x0304_0506), Ump::one(0x10FE_0000), Ump::two(0x3032_0708, 0)]
        );
    }

    #[test]
    fn sysex_assembler_high_bit_bytes() {
        let mut a = SysexAssembler::new();
        // A complete packet carrying [01 F8 02]: the real-time byte is kept apart.
        assert_eq!(a.push(&Ump::two(0x3003_01F8, 0x0200_0000)), Some(vec![1, 2]));
        assert_eq!(a.take_realtime(), vec![0xF8]);
        assert!(a.take_realtime().is_empty());
        // A start packet carrying [01 02 90 04 05 06] is truncated at the status and returned at once ...
        assert_eq!(a.push(&Ump::two(0x3016_0102, 0x9004_0506)), Some(vec![1, 2]));
        // ... and its end packet then belongs to nothing.
        assert_eq!(a.push(&Ump::two(0x3032_0708, 0)), None);
        // Real-time bytes nobody takes stop at the cap.
        for _ in 0..70 {
            assert_eq!(a.push(&Ump::two(0x3001_F800, 0)), Some(Vec::new()));
        }
        assert_eq!(a.take_realtime().len(), SYSEX_REALTIME_HELD);
    }

    #[test]
    fn byte_stream_sysex_is_packed_as_it_arrives() {
        let mut c = Midi1ToUmp::new(0);
        let mut out = Vec::new();
        for b in [0xF0u8, 1, 2, 3, 4, 5, 6, 7, 0xF8, 8, 0xF7] {
            c.push(b, &mut out);
        }
        // The start packet leaves with the seventh byte, before the real-time byte that follows it.
        assert_eq!(
            out,
            vec![Ump::two(0x3016_0102, 0x0304_0506), Ump::one(0x10F8_0000), Ump::two(0x3032_0708, 0)]
        );
        // A message that never ends costs one packet of memory: 600 bytes give 99 packets so far (one each at
        // bytes 7, 13, ..., 595), the last six are held, and a channel status ends the message as truncated.
        let mut c = Midi1ToUmp::new(0);
        let mut out = Vec::new();
        c.push(0xF0, &mut out);
        for _ in 0..600 {
            c.push(0x11, &mut out);
        }
        assert_eq!(out.len(), 99);
        assert_eq!(out[0], Ump::two(0x3016_1111, 0x1111_1111));
        assert_eq!(out[98], Ump::two(0x3026_1111, 0x1111_1111));
        for b in [0x90u8, 60, 100] {
            c.push(b, &mut out);
        }
        assert_eq!(out.len(), 101);
        assert_eq!(out[99], Ump::two(0x3036_1111, 0x1111_1111));
        assert_eq!(out[100], Ump::one(0x2090_3C64));
    }

    #[test]
    fn playback_edge_writes_safe_wire_bytes() {
        let wire = |pk: &[Ump]| {
            let mut w = UmpToMidi1::new();
            let mut out = Vec::new();
            for p in pk {
                w.push(p, &mut out);
            }
            out
        };
        assert_eq!(wire(&sysex_to_ump(0, &[1, 2, 3, 4, 5, 6, 7, 8])), vec![0xF0, 1, 2, 3, 4, 5, 6, 7, 8, 0xF7]);
        // A real-time byte inside a packet is written where it stands.
        assert_eq!(
            wire(&[Ump::two(0x3013_01F8, 0x0200_0000), Ump::two(0x3031_0300, 0)]),
            vec![0xF0, 1, 0xF8, 2, 3, 0xF7]
        );
        // Any other status truncates the message; its end packet is dropped and the next message plays.
        assert_eq!(
            wire(&[Ump::two(0x3013_0190, 0x0200_0000), Ump::two(0x3031_0300, 0), Ump::one(0x2090_3C64)]),
            vec![0xF0, 1, 0xF7, 0x90, 0x3C, 0x64]
        );
        // A channel message inside an open message ends it first; a real-time one does not.
        assert_eq!(
            wire(&[Ump::two(0x3012_0102, 0), Ump::one(0x10F8_0000), Ump::one(0x2090_3C64)]),
            vec![0xF0, 1, 2, 0xF8, 0xF7, 0x90, 0x3C, 0x64]
        );
        // A new start truncates an open message.
        assert_eq!(wire(&[Ump::two(0x3011_0100, 0), Ump::two(0x3001_0200, 0)]), vec![0xF0, 1, 0xF7, 0xF0, 2, 0xF7]);
        // Status bytes that would corrupt the wire are not written.
        assert_eq!(ump_to_bytes(&Ump::one(0x1090_0000)), None);
        assert_eq!(ump_to_bytes(&Ump::one(0x10F0_0000)), None);
        assert_eq!(ump_to_bytes(&Ump::one(0x10F7_0000)), None);
        assert_eq!(ump_to_bytes(&Ump::one(0x2010_0000)), None);
        assert_eq!(ump_to_bytes(&Ump::one(0x20F8_0000)), None);
        assert_eq!(ump_to_bytes(&Ump::one(0x10FE_0000)), Some(vec![0xFE]));
        assert!(wire(&[Ump::one(0x1090_0000)]).is_empty());
        // A payload keeps its real-time bytes and stops at any other status.
        assert_eq!(sysex_bytes(&[1, 0xF8, 2, 0x90, 3]), vec![0xF0, 1, 0xF8, 2, 0xF7]);
    }
}
