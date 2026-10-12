//! Framing, command numbers, error codes and sample formats of the native protocol (version 35).
//! Error, sample-format and subscription numbers are from /usr/include/pulse/{def,sample}.h; the command numbers
//! are the protocol's enumeration (the header set does not install it).

pub const PROTOCOL_VERSION: u32 = 35;
pub const VERSION_MASK: u32 = 0xFFFF;
pub const INVALID: u32 = u32::MAX;
pub const VOLUME_NORM: u32 = 0x10000;
pub const DESCRIPTOR_LEN: usize = 20;
/// Largest frame accepted from a client.
pub const MAX_FRAME: usize = 16 << 20;
/// Channel value of a command packet.
pub const COMMAND_CHANNEL: u32 = u32::MAX;

pub mod cmd {
    pub const ERROR: u32 = 0;
    pub const REPLY: u32 = 2;
    pub const CREATE_PLAYBACK_STREAM: u32 = 3;
    pub const DELETE_PLAYBACK_STREAM: u32 = 4;
    pub const CREATE_RECORD_STREAM: u32 = 5;
    pub const DELETE_RECORD_STREAM: u32 = 6;
    pub const EXIT: u32 = 7;
    pub const AUTH: u32 = 8;
    pub const SET_CLIENT_NAME: u32 = 9;
    pub const LOOKUP_SINK: u32 = 10;
    pub const LOOKUP_SOURCE: u32 = 11;
    pub const DRAIN_PLAYBACK_STREAM: u32 = 12;
    pub const STAT: u32 = 13;
    pub const GET_PLAYBACK_LATENCY: u32 = 14;
    pub const GET_SERVER_INFO: u32 = 20;
    pub const GET_SINK_INFO: u32 = 21;
    pub const GET_SINK_INFO_LIST: u32 = 22;
    pub const GET_SOURCE_INFO: u32 = 23;
    pub const GET_SOURCE_INFO_LIST: u32 = 24;
    pub const GET_MODULE_INFO_LIST: u32 = 26;
    pub const GET_CLIENT_INFO: u32 = 27;
    pub const GET_CLIENT_INFO_LIST: u32 = 28;
    pub const GET_SINK_INPUT_INFO: u32 = 29;
    pub const GET_SINK_INPUT_INFO_LIST: u32 = 30;
    pub const GET_SOURCE_OUTPUT_INFO: u32 = 31;
    pub const GET_SOURCE_OUTPUT_INFO_LIST: u32 = 32;
    pub const GET_SAMPLE_INFO_LIST: u32 = 34;
    pub const SUBSCRIBE: u32 = 35;
    pub const SET_SINK_VOLUME: u32 = 36;
    pub const SET_SINK_INPUT_VOLUME: u32 = 37;
    pub const SET_SOURCE_VOLUME: u32 = 38;
    pub const SET_SINK_MUTE: u32 = 39;
    pub const SET_SOURCE_MUTE: u32 = 40;
    pub const CORK_PLAYBACK_STREAM: u32 = 41;
    pub const FLUSH_PLAYBACK_STREAM: u32 = 42;
    pub const TRIGGER_PLAYBACK_STREAM: u32 = 43;
    pub const SET_DEFAULT_SINK: u32 = 44;
    pub const SET_DEFAULT_SOURCE: u32 = 45;
    pub const SET_PLAYBACK_STREAM_NAME: u32 = 46;
    pub const SET_RECORD_STREAM_NAME: u32 = 47;
    pub const KILL_CLIENT: u32 = 48;
    pub const KILL_SINK_INPUT: u32 = 49;
    pub const KILL_SOURCE_OUTPUT: u32 = 50;
    pub const GET_RECORD_LATENCY: u32 = 57;
    pub const CORK_RECORD_STREAM: u32 = 58;
    pub const FLUSH_RECORD_STREAM: u32 = 59;
    pub const PREBUF_PLAYBACK_STREAM: u32 = 60;
    pub const REQUEST: u32 = 61;
    pub const OVERFLOW: u32 = 62;
    pub const UNDERFLOW: u32 = 63;
    pub const PLAYBACK_STREAM_KILLED: u32 = 64;
    pub const RECORD_STREAM_KILLED: u32 = 65;
    pub const SUBSCRIBE_EVENT: u32 = 66;
    pub const SET_SINK_INPUT_MUTE: u32 = 69;
    pub const SUSPEND_SINK: u32 = 70;
    pub const SUSPEND_SOURCE: u32 = 71;
    pub const SET_PLAYBACK_STREAM_BUFFER_ATTR: u32 = 72;
    pub const SET_RECORD_STREAM_BUFFER_ATTR: u32 = 73;
    pub const UPDATE_PLAYBACK_STREAM_SAMPLE_RATE: u32 = 74;
    pub const UPDATE_RECORD_STREAM_SAMPLE_RATE: u32 = 75;
    pub const UPDATE_RECORD_STREAM_PROPLIST: u32 = 80;
    pub const UPDATE_PLAYBACK_STREAM_PROPLIST: u32 = 81;
    pub const UPDATE_CLIENT_PROPLIST: u32 = 82;
    pub const REMOVE_RECORD_STREAM_PROPLIST: u32 = 83;
    pub const REMOVE_PLAYBACK_STREAM_PROPLIST: u32 = 84;
    pub const REMOVE_CLIENT_PROPLIST: u32 = 85;
    pub const STARTED: u32 = 86;
    pub const SET_SOURCE_OUTPUT_VOLUME: u32 = 98;
    pub const SET_SOURCE_OUTPUT_MUTE: u32 = 99;
}

/// Protocol error codes (`pa_error_code_t`).
pub mod err {
    pub const ACCESS: u32 = 1;
    /// Unknown command; the session answers unknown commands with NOTIMPLEMENTED instead.
    #[allow(dead_code)]
    pub const COMMAND: u32 = 2;
    pub const INVALID: u32 = 3;
    pub const NOENTITY: u32 = 5;
    pub const PROTOCOL: u32 = 7;
    pub const INTERNAL: u32 = 10;
    /// Operation in a stream state that does not allow it; no command reaches that check yet.
    #[allow(dead_code)]
    pub const BADSTATE: u32 = 15;
    pub const VERSION: u32 = 17;
    pub const NOTSUPPORTED: u32 = 19;
    pub const NOTIMPLEMENTED: u32 = 23;
}

/// Subscription facilities and event types (`pa_subscription_event_type_t`).
pub mod sub {
    pub const SINK: u32 = 0;
    pub const SOURCE: u32 = 1;
    pub const SINK_INPUT: u32 = 2;
    pub const SOURCE_OUTPUT: u32 = 3;
    pub const CLIENT: u32 = 5;
    pub const SERVER: u32 = 7;
    pub const NEW: u32 = 0x0000;
    pub const CHANGE: u32 = 0x0010;
    pub const REMOVE: u32 = 0x0020;
    /// A subscription mask bit for a facility.
    pub const fn mask_bit(facility: u32) -> u32 {
        1 << facility
    }
}

/// A packet header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Descriptor {
    pub length: u32,
    pub channel: u32,
    /// Seek offset of a data packet (signed, high word first on the wire).
    pub offset: i64,
    pub flags: u32,
}

impl Descriptor {
    pub fn parse(b: &[u8; DESCRIPTOR_LEN]) -> Descriptor {
        let w = |i: usize| u32::from_be_bytes(b[i..i + 4].try_into().unwrap());
        Descriptor { length: w(0), channel: w(4), offset: (((w(8) as u64) << 32) | w(12) as u64) as i64, flags: w(16) }
    }
    pub fn encode(&self) -> [u8; DESCRIPTOR_LEN] {
        let mut b = [0u8; DESCRIPTOR_LEN];
        b[0..4].copy_from_slice(&self.length.to_be_bytes());
        b[4..8].copy_from_slice(&self.channel.to_be_bytes());
        b[8..12].copy_from_slice(&((self.offset as u64 >> 32) as u32).to_be_bytes());
        b[12..16].copy_from_slice(&(self.offset as u64 as u32).to_be_bytes());
        b[16..20].copy_from_slice(&self.flags.to_be_bytes());
        b
    }
}

/// A complete frame (descriptor then payload) for a command or reply payload.
pub fn command_frame(payload: &[u8]) -> Vec<u8> {
    let d = Descriptor { length: payload.len() as u32, channel: COMMAND_CHANNEL, offset: 0, flags: 0 };
    let mut v = d.encode().to_vec();
    v.extend_from_slice(payload);
    v
}

/// A complete frame carrying stream data for `channel`.
pub fn data_frame(channel: u32, data: &[u8]) -> Vec<u8> {
    let d = Descriptor { length: data.len() as u32, channel, offset: 0, flags: 0 };
    let mut v = d.encode().to_vec();
    v.extend_from_slice(data);
    v
}

/// Incremental frame reassembly over a byte stream.
#[derive(Default)]
pub struct FrameBuf {
    buf: Vec<u8>,
}

impl FrameBuf {
    pub fn push(&mut self, bytes: &[u8]) {
        self.buf.extend_from_slice(bytes);
    }
    /// The next complete frame, `Err` when the descriptor announces an oversized frame.
    pub fn next(&mut self) -> Result<Option<(Descriptor, Vec<u8>)>, ()> {
        if self.buf.len() < DESCRIPTOR_LEN {
            return Ok(None);
        }
        let d = Descriptor::parse(self.buf[..DESCRIPTOR_LEN].try_into().unwrap());
        if d.length as usize > MAX_FRAME {
            return Err(());
        }
        let total = DESCRIPTOR_LEN + d.length as usize;
        if self.buf.len() < total {
            return Ok(None);
        }
        let payload = self.buf[DESCRIPTOR_LEN..total].to_vec();
        self.buf.drain(..total);
        Ok(Some((d, payload)))
    }
}

/// The sample formats the server converts (`pa_sample_format_t` numbers).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SampleFormat {
    U8,
    S16Le,
    S16Be,
    F32Le,
    F32Be,
    S32Le,
    S32Be,
    S24Le,
    S24In32Le,
}

impl SampleFormat {
    pub fn from_wire(n: u8) -> Option<SampleFormat> {
        Some(match n {
            0 => SampleFormat::U8,
            3 => SampleFormat::S16Le,
            4 => SampleFormat::S16Be,
            5 => SampleFormat::F32Le,
            6 => SampleFormat::F32Be,
            7 => SampleFormat::S32Le,
            8 => SampleFormat::S32Be,
            9 => SampleFormat::S24Le,
            11 => SampleFormat::S24In32Le,
            _ => return None,
        })
    }
    pub fn to_wire(self) -> u8 {
        match self {
            SampleFormat::U8 => 0,
            SampleFormat::S16Le => 3,
            SampleFormat::S16Be => 4,
            SampleFormat::F32Le => 5,
            SampleFormat::F32Be => 6,
            SampleFormat::S32Le => 7,
            SampleFormat::S32Be => 8,
            SampleFormat::S24Le => 9,
            SampleFormat::S24In32Le => 11,
        }
    }
    pub fn bytes(self) -> usize {
        match self {
            SampleFormat::U8 => 1,
            SampleFormat::S16Le | SampleFormat::S16Be => 2,
            SampleFormat::S24Le => 3,
            _ => 4,
        }
    }

    /// Decodes whole samples of `src` into `out` (appending); a trailing partial sample is ignored.
    pub fn decode(self, src: &[u8], out: &mut Vec<f32>) {
        let n = self.bytes();
        for c in src.chunks_exact(n) {
            out.push(match self {
                SampleFormat::U8 => (c[0] as f32 - 128.0) / 128.0,
                SampleFormat::S16Le => i16::from_le_bytes([c[0], c[1]]) as f32 / 32768.0,
                SampleFormat::S16Be => i16::from_be_bytes([c[0], c[1]]) as f32 / 32768.0,
                SampleFormat::F32Le => f32::from_le_bytes([c[0], c[1], c[2], c[3]]),
                SampleFormat::F32Be => f32::from_be_bytes([c[0], c[1], c[2], c[3]]),
                SampleFormat::S32Le => i32::from_le_bytes([c[0], c[1], c[2], c[3]]) as f32 / 2147483648.0,
                SampleFormat::S32Be => i32::from_be_bytes([c[0], c[1], c[2], c[3]]) as f32 / 2147483648.0,
                SampleFormat::S24Le => {
                    let v = ((c[2] as i8 as i32) << 16) | ((c[1] as i32) << 8) | c[0] as i32;
                    v as f32 / 8388608.0
                }
                SampleFormat::S24In32Le => {
                    let v = i32::from_le_bytes([c[0], c[1], c[2], c[3]]) << 8 >> 8;
                    v as f32 / 8388608.0
                }
            });
        }
    }

    /// Encodes `src` (clamped to [-1, 1]) and appends the bytes to `out`.
    pub fn encode(self, src: &[f32], out: &mut Vec<u8>) {
        for &s in src {
            let s = if s.is_nan() { 0.0 } else { s.clamp(-1.0, 1.0) };
            match self {
                SampleFormat::U8 => out.push((s * 127.0 + 128.0).round() as u8),
                SampleFormat::S16Le => out.extend_from_slice(&((s * 32767.0).round() as i16).to_le_bytes()),
                SampleFormat::S16Be => out.extend_from_slice(&((s * 32767.0).round() as i16).to_be_bytes()),
                SampleFormat::F32Le => out.extend_from_slice(&s.to_le_bytes()),
                SampleFormat::F32Be => out.extend_from_slice(&s.to_be_bytes()),
                SampleFormat::S32Le => out.extend_from_slice(&((s as f64 * 2147483647.0).round() as i32).to_le_bytes()),
                SampleFormat::S32Be => out.extend_from_slice(&((s as f64 * 2147483647.0).round() as i32).to_be_bytes()),
                SampleFormat::S24Le => {
                    let v = (s * 8388607.0).round() as i32;
                    out.extend_from_slice(&v.to_le_bytes()[..3]);
                }
                SampleFormat::S24In32Le => out.extend_from_slice(&((s * 8388607.0).round() as i32).to_le_bytes()),
            }
        }
    }
}

/// The default channel map (`pa_channel_position_t`: mono 0, left 1, right 2) for `n` channels.
pub fn default_channel_map(n: u8) -> Vec<u8> {
    match n {
        1 => vec![0],
        2 => vec![1, 2],
        _ => (0..n).map(|i| 3 + i).collect(),
    }
}

/// The linear gain of a protocol volume (the cubic curve of the reference server).
pub fn volume_to_gain(v: u32) -> f32 {
    let x = v as f32 / VOLUME_NORM as f32;
    x * x * x
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn descriptor_round_trip_and_layout() {
        let d = Descriptor { length: 5, channel: 0xFFFF_FFFF, offset: -1, flags: 0x100 };
        let b = d.encode();
        assert_eq!(&b[0..4], &[0, 0, 0, 5]);
        assert_eq!(&b[4..8], &[0xFF; 4]);
        assert_eq!(&b[8..16], &[0xFF; 8]);
        assert_eq!(&b[16..20], &[0, 0, 1, 0]);
        assert_eq!(Descriptor::parse(&b), d);
        let d2 = Descriptor { length: 0, channel: 1, offset: 0x1_0000_0002, flags: 0 };
        assert_eq!(&d2.encode()[8..16], &[0, 0, 0, 1, 0, 0, 0, 2]);
    }

    #[test]
    fn frame_buf_reassembles_split_and_joined_frames() {
        let a = command_frame(&[1, 2, 3]);
        let b = data_frame(7, &[9]);
        let mut all = a.clone();
        all.extend_from_slice(&b);
        let mut fb = FrameBuf::default();
        fb.push(&all[..10]);
        assert_eq!(fb.next(), Ok(None));
        fb.push(&all[10..a.len() + 3]);
        let (d, p) = fb.next().unwrap().unwrap();
        assert_eq!((d.channel, p), (COMMAND_CHANNEL, vec![1, 2, 3]));
        assert_eq!(fb.next(), Ok(None));
        fb.push(&all[a.len() + 3..]);
        let (d, p) = fb.next().unwrap().unwrap();
        assert_eq!((d.channel, p), (7, vec![9]));
    }

    #[test]
    fn oversized_frame_is_an_error() {
        let d = Descriptor { length: MAX_FRAME as u32 + 1, channel: 0, offset: 0, flags: 0 };
        let mut fb = FrameBuf::default();
        fb.push(&d.encode());
        assert_eq!(fb.next(), Err(()));
    }

    #[test]
    fn s16_decode_values() {
        let mut v = Vec::new();
        SampleFormat::S16Le.decode(&[0x00, 0x40, 0x00, 0xC0, 0xFF], &mut v);
        // 0x4000 = 16384 -> 0.5; 0xC000 = -16384 -> -0.5; the last byte is a partial sample
        assert_eq!(v, vec![0.5, -0.5]);
    }

    #[test]
    fn encode_decode_round_trips() {
        let src = [0.0f32, 0.5, -0.5, 1.0, -1.0];
        for f in [SampleFormat::S16Le, SampleFormat::S16Be, SampleFormat::F32Le, SampleFormat::F32Be, SampleFormat::S32Le, SampleFormat::S32Be, SampleFormat::S24Le, SampleFormat::S24In32Le] {
            let mut bytes = Vec::new();
            f.encode(&src, &mut bytes);
            assert_eq!(bytes.len(), src.len() * f.bytes());
            let mut back = Vec::new();
            f.decode(&bytes, &mut back);
            for (a, b) in src.iter().zip(&back) {
                assert!((a - b).abs() < 1e-3, "{:?}: {} vs {}", f, a, b);
            }
        }
    }

    #[test]
    fn u8_is_offset_binary() {
        let mut v = Vec::new();
        SampleFormat::U8.decode(&[128, 0, 255], &mut v);
        assert_eq!(v[0], 0.0);
        assert_eq!(v[1], -1.0);
        assert!((v[2] - 127.0 / 128.0).abs() < 1e-6);
    }

    #[test]
    fn encode_clamps() {
        let mut b = Vec::new();
        SampleFormat::S16Le.encode(&[2.0, -2.0], &mut b);
        assert_eq!(b, vec![0xFF, 0x7F, 0x01, 0x80]);
    }

    #[test]
    fn wire_numbers() {
        assert_eq!(SampleFormat::from_wire(3), Some(SampleFormat::S16Le));
        assert_eq!(SampleFormat::from_wire(5), Some(SampleFormat::F32Le));
        assert_eq!(SampleFormat::from_wire(1), None);
        assert_eq!(SampleFormat::S24In32Le.to_wire(), 11);
    }

    #[test]
    fn channel_maps_and_gain() {
        assert_eq!(default_channel_map(2), vec![1, 2]);
        assert_eq!(default_channel_map(1), vec![0]);
        assert_eq!(default_channel_map(4), vec![3, 4, 5, 6]);
        assert_eq!(volume_to_gain(VOLUME_NORM), 1.0);
        assert_eq!(volume_to_gain(0), 0.0);
        assert!((volume_to_gain(VOLUME_NORM / 2) - 0.125).abs() < 1e-6);
    }
}
