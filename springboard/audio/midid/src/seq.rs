//! ALSA sequencer through the kernel uapi (`sound/asequencer.h`) on `/dev/snd/seq`. No libasound. The daemon opens a
//! client, creates one port other clients (hardware, software synths) can subscribe to, and reads its events; each
//! event is turned into a MIDI 1.0 event and stamped with the host clock on arrival.

use bifrost_midi::events::MidiEvent;
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
}

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

    /// Reads the events waiting; each is fixed size. A variable-length event (system exclusive) is followed by
    /// its payload, which is skipped here.
    pub fn read_events(&self, buf: &mut [u8]) -> io::Result<Vec<SeqEvent>> {
        // SAFETY: `buf` is a valid writable slice of the length passed.
        let n = unsafe { libc::read(self.fd, buf.as_mut_ptr() as *mut libc::c_void, buf.len()) };
        if n < 0 {
            let e = io::Error::last_os_error();
            return if e.kind() == io::ErrorKind::WouldBlock { Ok(Vec::new()) } else { Err(e) };
        }
        Ok(parse_events(&buf[..n as usize]))
    }
}

/// Splits a read buffer into events, skipping the payload that follows a variable-length event.
pub fn parse_events(buf: &[u8]) -> Vec<SeqEvent> {
    let mut out = Vec::new();
    let mut at = 0;
    while at + EVENT_SIZE <= buf.len() {
        let mut raw = [0u8; EVENT_SIZE];
        raw.copy_from_slice(&buf[at..at + EVENT_SIZE]);
        let ev = SeqEvent(raw);
        at += EVENT_SIZE;
        if ev.flags() & 0x0C == 0x04 {
            // Variable length: the length is the first u32 of the data union.
            let len = u32::from_ne_bytes([raw[16], raw[17], raw[18], raw[19]]) as usize;
            at += len;
        }
        out.push(ev);
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

    #[test]
    fn variable_length_payload_is_skipped() {
        let mut buf = Vec::new();
        let mut sysex = ev(130, 0x04, [5, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        sysex[16..20].copy_from_slice(&5u32.to_ne_bytes());
        buf.extend_from_slice(&sysex);
        buf.extend_from_slice(&[0xF0, 1, 2, 3, 0xF7]);
        buf.extend_from_slice(&ev(EVENT_NOTEOFF, 0, [0, 61, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]));
        let evs = parse_events(&buf);
        assert_eq!(evs.len(), 2);
        assert_eq!(evs[0].kind(), 130);
        assert_eq!(evs[1].to_midi1(), Some(MidiEvent::NoteOff { channel: 0, key: 61, velocity: 0 }));
    }

    #[test]
    fn names_are_truncated_with_a_nul() {
        let mut b = [0xFFu8; 8];
        put_name(&mut b, "abcdefghij");
        assert_eq!(&b[..7], b"abcdefg");
        assert_eq!(b[7], 0xFF); // dst len 8 keeps one byte for the NUL the zeroed struct already has
    }
}
