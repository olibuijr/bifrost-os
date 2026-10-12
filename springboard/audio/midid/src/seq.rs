//! ALSA sequencer through the kernel uapi (`sound/asequencer.h`) on `/dev/snd/seq`. No libasound. The daemon opens a
//! client, creates one port other clients (hardware, software synths) can subscribe to, and reads its events; each
//! event is turned into a UMP packet (channel voice, system common, real-time) or, for system exclusive, its raw
//! bytes, and stamped with the host clock right after the read.

use bifrost_midi::events::MidiEvent;
use bifrost_midi::ump::{from_midi1, system_packet, Ump};
use std::ffi::CString;
use std::io;

pub const IOCTL_CLIENT_ID: libc::c_ulong = 0x8004_5301;
pub const IOCTL_SET_CLIENT_INFO: libc::c_ulong = 0x40bc_5311;
pub const IOCTL_CREATE_PORT: libc::c_ulong = 0xc0a8_5320;
pub const IOCTL_SUBSCRIBE_PORT: libc::c_ulong = 0x4050_5330;

pub const EVENT_NOTEON: u8 = 6;
pub const EVENT_NOTEOFF: u8 = 7;
pub const EVENT_KEYPRESS: u8 = 8;
pub const EVENT_CONTROLLER: u8 = 10;
pub const EVENT_PGMCHANGE: u8 = 11;
pub const EVENT_CHANPRESS: u8 = 12;
pub const EVENT_PITCHBEND: u8 = 13;
pub const EVENT_SONGPOS: u8 = 20;
pub const EVENT_SONGSEL: u8 = 21;
pub const EVENT_QFRAME: u8 = 22;
pub const EVENT_START: u8 = 30;
pub const EVENT_CONTINUE: u8 = 31;
pub const EVENT_STOP: u8 = 32;
pub const EVENT_CLOCK: u8 = 36;
pub const EVENT_TUNE_REQUEST: u8 = 40;
pub const EVENT_RESET: u8 = 41;
pub const EVENT_SENSING: u8 = 42;
pub const EVENT_SYSEX: u8 = 130;

/// `SNDRV_SEQ_EVENT_LENGTH_MASK` and `SNDRV_SEQ_EVENT_LENGTH_VARIABLE` in the event flags.
pub const LENGTH_MASK: u8 = 0x0C;
pub const LENGTH_VARIABLE: u8 = 0x04;
/// The top two bits of `ext.len` are kernel flags (`SNDRV_SEQ_EXT_MASK`), not length.
pub const EXT_LEN_MASK: u32 = 0x3FFF_FFFF;

pub const PORT_CAP_READ: u32 = 0x1;
pub const PORT_CAP_WRITE: u32 = 0x2;
pub const PORT_CAP_SUBS_READ: u32 = 0x20;
pub const PORT_CAP_SUBS_WRITE: u32 = 0x40;
pub const PORT_TYPE_MIDI_GENERIC: u32 = 0x2;
pub const PORT_TYPE_APPLICATION: u32 = 0x10_0000;

/// `struct snd_seq_client_info` (the tail past the name is carried opaque).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct ClientInfo {
    pub client: i32,
    pub kind: i32,
    pub name: [u8; 64],
    pub rest: [u8; 116],
}

/// `struct snd_seq_port_info`.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct PortInfo {
    pub client: u8,
    pub port: u8,
    pub name: [u8; 64],
    pub capability: u32,
    pub kind: u32,
    pub midi_channels: i32,
    pub midi_voices: i32,
    pub synth_voices: i32,
    pub read_use: i32,
    pub write_use: i32,
    pub kernel: usize,
    pub flags: u32,
    pub time_queue: u8,
    pub direction: u8,
    pub ump_group: u8,
    pub reserved: [u8; 57],
}

/// `struct snd_seq_port_subscribe`.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct PortSubscribe {
    pub sender: [u8; 2],
    pub dest: [u8; 2],
    pub voices: u32,
    pub flags: u32,
    pub queue: u8,
    pub rest: [u8; 67],
}

/// One `struct snd_seq_event` (28 bytes) as read from the device.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SeqEvent(pub [u8; EVENT_SIZE]);

pub const EVENT_SIZE: usize = 28;

impl SeqEvent {
    pub fn kind(&self) -> u8 {
        self.0[0]
    }
    pub fn flags(&self) -> u8 {
        self.0[1]
    }
    /// The source client and port.
    pub fn source(&self) -> (u8, u8) {
        (self.0[12], self.0[13])
    }
    fn i32_at(&self, at: usize) -> i32 {
        i32::from_ne_bytes([self.0[at], self.0[at + 1], self.0[at + 2], self.0[at + 3]])
    }
    /// The MIDI 1.0 event this carries, if it is a channel voice event. Channels are masked to 4 bits. Pitch bend
    /// is signed in the kernel (-8192 to 8191) and offset to 0 to 16383.
    pub fn to_midi1(&self) -> Option<MidiEvent> {
        let channel = self.0[16] & 0x0F;
        let b17 = self.0[17] & 0x7F;
        let b18 = self.0[18] & 0x7F;
        Some(match self.kind() {
            EVENT_NOTEON if b18 == 0 => MidiEvent::NoteOff { channel, key: b17, velocity: 0 },
            EVENT_NOTEON => MidiEvent::NoteOn { channel, key: b17, velocity: b18 },
            EVENT_NOTEOFF => MidiEvent::NoteOff { channel, key: b17, velocity: b18 },
            EVENT_KEYPRESS => MidiEvent::PolyPressure { channel, key: b17, pressure: b18 },
            EVENT_CONTROLLER => MidiEvent::ControlChange {
                channel,
                controller: self.i32_at(20).clamp(0, 127) as u8,
                value: self.i32_at(24).clamp(0, 127) as u8,
            },
            EVENT_PGMCHANGE => MidiEvent::ProgramChange { channel, program: self.i32_at(24).clamp(0, 127) as u8 },
            EVENT_CHANPRESS => MidiEvent::ChannelPressure { channel, pressure: self.i32_at(24).clamp(0, 127) as u8 },
            EVENT_PITCHBEND => MidiEvent::PitchBend {
                channel,
                value: (self.i32_at(24).clamp(-8192, 8191) + 8192) as u16,
            },
            _ => return None,
        })
    }

    /// The UMP packet this carries in `group`: channel voice as type 2, system common and real-time as type 1.
    /// System exclusive (its bytes travel in the payload) and every other kind give `None`.
    pub fn to_ump(&self, group: u8) -> Option<Ump> {
        if let Some(m) = self.to_midi1() {
            return Some(from_midi1(group, m));
        }
        let value = self.i32_at(24);
        let seven = value.clamp(0, 127) as u8;
        let sys = |st: u8, d1: u8, d2: u8| Some(system_packet(group, st, d1, d2));
        match self.kind() {
            EVENT_QFRAME => sys(0xF1, seven, 0),
            EVENT_SONGPOS => {
                // The kernel keeps the 14-bit position as msb * 128 + lsb.
                let v = value.clamp(0, 0x3FFF) as u16;
                sys(0xF2, (v & 0x7F) as u8, (v >> 7) as u8)
            }
            EVENT_SONGSEL => sys(0xF3, seven, 0),
            EVENT_TUNE_REQUEST => sys(0xF6, 0, 0),
            EVENT_CLOCK => sys(0xF8, 0, 0),
            EVENT_START => sys(0xFA, 0, 0),
            EVENT_CONTINUE => sys(0xFB, 0, 0),
            EVENT_STOP => sys(0xFC, 0, 0),
            EVENT_SENSING => sys(0xFE, 0, 0),
            EVENT_RESET => sys(0xFF, 0, 0),
            _ => None,
        }
    }
}

/// One event read from the sequencer with its variable-length payload (the system exclusive bytes), empty for a
/// fixed-size event.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SeqRead {
    pub event: SeqEvent,
    pub payload: Vec<u8>,
}

/// The largest read buffer: one event header and a payload as large as the biggest system exclusive message kept
/// anywhere else (the kernel's event pools hold far less, so no real event is larger).
pub const READ_MAX: usize = EVENT_SIZE + bifrost_midi::ump::SYSEX_MAX_BYTES;

fn zeroed<T: Copy>() -> T {
    // SAFETY: only used for the plain-old-data uapi structs above, for which all-zero bytes are a valid value.
    unsafe { std::mem::zeroed() }
}

fn put_name(dst: &mut [u8], name: &str) {
    let n = name.len().min(dst.len() - 1);
    dst[..n].copy_from_slice(&name.as_bytes()[..n]);
}

/// An open sequencer client.
pub struct Seq {
    fd: libc::c_int,
    pub client: i32,
}

impl Seq {
    pub fn open(path: &str) -> io::Result<Seq> {
        let c = CString::new(path).map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?;
        // SAFETY: `c` is a valid NUL-terminated path that outlives the call.
        let fd = unsafe { libc::open(c.as_ptr(), libc::O_RDWR | libc::O_NONBLOCK | libc::O_CLOEXEC) };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        let mut client: libc::c_int = 0;
        // SAFETY: `client` is a live c_int, which is what CLIENT_ID writes.
        let r = unsafe { libc::ioctl(fd, IOCTL_CLIENT_ID as _, &mut client as *mut libc::c_int) };
        if r < 0 {
            let e = io::Error::last_os_error();
            // SAFETY: fd is open and closed once on this error path.
            unsafe { libc::close(fd) };
            return Err(e);
        }
        Ok(Seq { fd, client })
    }

    pub fn fd(&self) -> libc::c_int {
        self.fd
    }

    fn ioctl<T>(&self, req: libc::c_ulong, arg: &mut T) -> io::Result<()> {
        // SAFETY: `arg` is a live, exclusively borrowed uapi struct of the size encoded in `req` (pinned by tests).
        let r = unsafe { libc::ioctl(self.fd, req as _, arg as *mut T) };
        if r < 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }

    pub fn set_name(&self, name: &str) -> io::Result<()> {
        let mut info: ClientInfo = zeroed();
        info.client = self.client;
        put_name(&mut info.name, name);
        self.ioctl(IOCTL_SET_CLIENT_INFO, &mut info)
    }

    /// Creates a port other clients can read from and write to. Returns the port number.
    pub fn create_port(&self, name: &str) -> io::Result<u8> {
        let mut p: PortInfo = zeroed();
        p.client = self.client as u8;
        put_name(&mut p.name, name);
        p.capability = PORT_CAP_READ | PORT_CAP_WRITE | PORT_CAP_SUBS_READ | PORT_CAP_SUBS_WRITE;
        p.kind = PORT_TYPE_MIDI_GENERIC | PORT_TYPE_APPLICATION;
        self.ioctl(IOCTL_CREATE_PORT, &mut p)?;
        Ok(p.port)
    }

    /// Subscribes `sender` (client, port) to `dest` (client, port).
    #[allow(dead_code)]
    pub fn subscribe(&self, sender: (u8, u8), dest: (u8, u8)) -> io::Result<()> {
        let mut s: PortSubscribe = zeroed();
        s.sender = [sender.0, sender.1];
        s.dest = [dest.0, dest.1];
        self.ioctl(IOCTL_SUBSCRIBE_PORT, &mut s)
    }

    /// True when `poll` says an event is waiting now (zero timeout).
    fn readable_now(&self) -> bool {
        let mut p = libc::pollfd { fd: self.fd, events: libc::POLLIN, revents: 0 };
        // SAFETY: `p` is one live pollfd and the count passed is 1.
        let r = unsafe { libc::poll(&mut p, 1, 0) };
        r > 0 && p.revents & libc::POLLIN != 0
    }

    /// Reads the events waiting, each with its payload. The kernel copies whole events only: when the first one
    /// waiting carries a payload larger than `buf` it answers EAGAIN, exactly as for an empty queue, and keeps the
    /// event at the head, so the port would stall with `poll` always ready. A zero-timeout poll tells the two apart
    /// and `buf` grows (doubling, up to `READ_MAX`) until the event fits. An overflowed input queue is reported once
    /// by the kernel as ENOSPC (the events in it are lost) and comes back as that error.
    pub fn read_events(&self, buf: &mut Vec<u8>) -> io::Result<Vec<SeqRead>> {
        if buf.len() < EVENT_SIZE {
            buf.resize(EVENT_SIZE * 64, 0);
        }
        loop {
            // SAFETY: `buf` is a valid writable buffer of the length passed.
            let n = unsafe { libc::read(self.fd, buf.as_mut_ptr() as *mut libc::c_void, buf.len()) };
            if n >= 0 {
                return Ok(parse_events(&buf[..n as usize]));
            }
            let e = io::Error::last_os_error();
            match e.kind() {
                io::ErrorKind::Interrupted => continue,
                io::ErrorKind::WouldBlock => {
                    let next = grown_len(buf.len());
                    if next > buf.len() && self.readable_now() {
                        buf.resize(next, 0);
                        continue;
                    }
                    return Ok(Vec::new());
                }
                _ => return Err(e),
            }
        }
    }
}

/// The next read buffer size after an event did not fit: double, capped at `READ_MAX`.
pub fn grown_len(len: usize) -> usize {
    len.saturating_mul(2).min(READ_MAX).max(len)
}

/// Splits a read buffer into events with their payloads. Only a variable-length event has bytes after its header
/// (`ext.len` of them, flag bits masked off); the kernel writes whole events, so a buffer that ends inside a header
/// or a payload is a short read: the partial event and anything after it are dropped rather than misframed.
pub fn parse_events(buf: &[u8]) -> Vec<SeqRead> {
    let mut out = Vec::new();
    let mut at = 0;
    while buf.len() - at >= EVENT_SIZE {
        let mut raw = [0u8; EVENT_SIZE];
        raw.copy_from_slice(&buf[at..at + EVENT_SIZE]);
        at += EVENT_SIZE;
        let event = SeqEvent(raw);
        let mut payload = Vec::new();
        if event.flags() & LENGTH_MASK == LENGTH_VARIABLE {
            // The length is the first u32 of the data union (`struct snd_seq_ev_ext`).
            let len = (u32::from_ne_bytes([raw[16], raw[17], raw[18], raw[19]]) & EXT_LEN_MASK) as usize;
            if len > buf.len() - at {
                break;
            }
            payload.extend_from_slice(&buf[at..at + len]);
            at += len;
        }
        out.push(SeqRead { event, payload });
    }
    out
}

impl Drop for Seq {
    fn drop(&mut self) {
        // SAFETY: `fd` was opened by `open` and is closed exactly once here.
        unsafe { libc::close(self.fd) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::{offset_of, size_of};

    #[test]
    fn struct_layouts_match_the_kernel_header() {
        // Measured against sound/asequencer.h with a C compiler.
        assert_eq!(size_of::<ClientInfo>(), 188);
        assert_eq!(offset_of!(ClientInfo, name), 8);
        assert_eq!(size_of::<PortInfo>(), 168);
        assert_eq!(offset_of!(PortInfo, name), 2);
        assert_eq!(offset_of!(PortInfo, capability), 68);
        assert_eq!(offset_of!(PortInfo, kind), 72);
        assert_eq!(size_of::<PortSubscribe>(), 80);
        assert_eq!(offset_of!(PortSubscribe, dest), 2);
        assert_eq!(offset_of!(PortSubscribe, flags), 8);
        assert_eq!(offset_of!(PortSubscribe, queue), 12);
    }

    #[test]
    fn ioctl_numbers_encode_direction_size_type_nr() {
        let enc = |dir: u64, size: u64, nr: u64| ((dir << 30) | (size << 16) | ((b'S' as u64) << 8) | nr) as libc::c_ulong;
        assert_eq!(IOCTL_CLIENT_ID, enc(2, 4, 0x01));
        assert_eq!(IOCTL_SET_CLIENT_INFO, enc(1, 188, 0x11));
        assert_eq!(IOCTL_CREATE_PORT, enc(3, 168, 0x20));
        assert_eq!(IOCTL_SUBSCRIBE_PORT, enc(1, 80, 0x30));
    }

    fn ev(kind: u8, flags: u8, data: [u8; 12]) -> [u8; EVENT_SIZE] {
        let mut b = [0u8; EVENT_SIZE];
        b[0] = kind;
        b[1] = flags;
        b[12] = 20; // source client
        b[13] = 1; // source port
        b[16..28].copy_from_slice(&data);
        b
    }

    #[test]
    fn note_and_controller_events() {
        let on = SeqEvent(ev(EVENT_NOTEON, 0, [2, 60, 100, 0, 0, 0, 0, 0, 0, 0, 0, 0]));
        assert_eq!(on.to_midi1(), Some(MidiEvent::NoteOn { channel: 2, key: 60, velocity: 100 }));
        assert_eq!(on.source(), (20, 1));
        let silent = SeqEvent(ev(EVENT_NOTEON, 0, [2, 60, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]));
        assert_eq!(silent.to_midi1(), Some(MidiEvent::NoteOff { channel: 2, key: 60, velocity: 0 }));
        let mut d = [0u8; 12];
        d[0] = 1;
        d[4..8].copy_from_slice(&64i32.to_ne_bytes());
        d[8..12].copy_from_slice(&127i32.to_ne_bytes());
        assert_eq!(
            SeqEvent(ev(EVENT_CONTROLLER, 0, d)).to_midi1(),
            Some(MidiEvent::ControlChange { channel: 1, controller: 64, value: 127 })
        );
    }

    #[test]
    fn pitch_bend_is_offset_to_unsigned() {
        let mk = |v: i32| {
            let mut d = [0u8; 12];
            d[8..12].copy_from_slice(&v.to_ne_bytes());
            SeqEvent(ev(EVENT_PITCHBEND, 0, d)).to_midi1()
        };
        assert_eq!(mk(0), Some(MidiEvent::PitchBend { channel: 0, value: 8192 }));
        assert_eq!(mk(-8192), Some(MidiEvent::PitchBend { channel: 0, value: 0 }));
        assert_eq!(mk(8191), Some(MidiEvent::PitchBend { channel: 0, value: 16383 }));
    }

    fn sysex_header(len: u32) -> [u8; EVENT_SIZE] {
        let mut h = ev(EVENT_SYSEX, LENGTH_VARIABLE, [0; 12]);
        h[16..20].copy_from_slice(&len.to_ne_bytes());
        h
    }

    #[test]
    fn variable_length_payload_is_kept() {
        let mut buf = Vec::new();
        buf.extend_from_slice(&sysex_header(5));
        buf.extend_from_slice(&[0xF0, 1, 2, 3, 0xF7]);
        buf.extend_from_slice(&ev(EVENT_NOTEOFF, 0, [0, 61, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]));
        let evs = parse_events(&buf);
        assert_eq!(evs.len(), 2);
        assert_eq!(evs[0].event.kind(), EVENT_SYSEX);
        assert_eq!(evs[0].payload, vec![0xF0, 1, 2, 3, 0xF7]);
        assert_eq!(evs[0].event.to_ump(0), None);
        assert!(evs[1].payload.is_empty());
        assert_eq!(evs[1].event.to_midi1(), Some(MidiEvent::NoteOff { channel: 0, key: 61, velocity: 0 }));
    }

    #[test]
    fn short_reads_drop_the_partial_event() {
        let note = ev(EVENT_NOTEON, 0, [0, 60, 100, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        // A whole event and 10 bytes of the next: one event.
        let mut buf = note.to_vec();
        buf.extend_from_slice(&note[..10]);
        assert_eq!(parse_events(&buf).len(), 1);
        // Less than a header: nothing.
        assert!(parse_events(&note[..27]).is_empty());
        assert!(parse_events(&[]).is_empty());
        // A payload that claims 100 bytes with 5 in the buffer: the event is dropped, not misframed.
        let mut buf = note.to_vec();
        buf.extend_from_slice(&sysex_header(100));
        buf.extend_from_slice(&[0xF0, 1, 2, 3, 4]);
        buf.extend_from_slice(&note);
        let evs = parse_events(&buf);
        assert_eq!(evs.len(), 1);
        assert_eq!(evs[0].event.kind(), EVENT_NOTEON);
        // A length of u32::MAX cannot overflow the cursor.
        assert!(parse_events(&sysex_header(u32::MAX)).is_empty());
    }

    #[test]
    fn ext_len_flag_bits_are_not_length() {
        // 0x8000_0005: the pointer-kind flag bit plus a length of 5.
        let mut buf = sysex_header(0x8000_0005).to_vec();
        buf.extend_from_slice(&[0xF0, 0x7D, 1, 2, 0xF7]);
        buf.extend_from_slice(&ev(EVENT_CLOCK, 0, [0; 12]));
        let evs = parse_events(&buf);
        assert_eq!(evs.len(), 2);
        assert_eq!(evs[0].payload, vec![0xF0, 0x7D, 1, 2, 0xF7]);
        assert_eq!(evs[1].event.to_ump(0), Some(Ump::one(0x10F8_0000)));
    }

    #[test]
    fn system_events_become_type_1_packets() {
        let with_value = |kind: u8, v: i32| {
            let mut d = [0u8; 12];
            d[8..12].copy_from_slice(&v.to_ne_bytes());
            SeqEvent(ev(kind, 0, d)).to_ump(3)
        };
        // Song position 0x1234 = 36 * 128 + 52: lsb 0x34, msb 0x24.
        assert_eq!(with_value(EVENT_SONGPOS, 0x1234), Some(Ump::one(0x13F2_3424)));
        assert_eq!(with_value(EVENT_SONGPOS, -5), Some(Ump::one(0x13F2_0000)));
        assert_eq!(with_value(EVENT_SONGSEL, 9), Some(Ump::one(0x13F3_0900)));
        assert_eq!(with_value(EVENT_QFRAME, 0x7FFF), Some(Ump::one(0x13F1_7F00)));
        for (kind, st) in [
            (EVENT_TUNE_REQUEST, 0xF6u32),
            (EVENT_CLOCK, 0xF8),
            (EVENT_START, 0xFA),
            (EVENT_CONTINUE, 0xFB),
            (EVENT_STOP, 0xFC),
            (EVENT_SENSING, 0xFE),
            (EVENT_RESET, 0xFF),
        ] {
            assert_eq!(with_value(kind, 0), Some(Ump::one(0x1300_0000 | st << 16)), "{kind}");
        }
        // Channel voice goes through the MIDI 1.0 path; unknown kinds give nothing.
        let on = SeqEvent(ev(EVENT_NOTEON, 0, [2, 60, 100, 0, 0, 0, 0, 0, 0, 0, 0, 0]));
        assert_eq!(on.to_ump(0), Some(Ump::one(0x2092_3C64)));
        for kind in [0u8, 5, 14, 35, 63, 99, 255] {
            assert_eq!(with_value(kind, 1), None, "{kind}");
        }
    }

    #[test]
    fn read_buffer_growth_is_capped() {
        assert_eq!(grown_len(EVENT_SIZE * 64), EVENT_SIZE * 128);
        assert_eq!(grown_len(READ_MAX - 1), READ_MAX);
        assert_eq!(grown_len(READ_MAX), READ_MAX);
    }

    #[test]
    fn names_are_truncated_with_a_nul() {
        let mut b = [0xFFu8; 8];
        put_name(&mut b, "abcdefghij");
        assert_eq!(&b[..7], b"abcdefg");
        assert_eq!(b[7], 0xFF); // dst len 8 keeps one byte for the NUL the zeroed struct already has
    }
}
