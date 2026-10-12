//! The control-socket protocol between clients (the HAL library, the policy process, the compatibility layers,
//! tools) and `bifrost-audiod`: `/run/bifrost/audio/control`, `SOCK_SEQPACKET`, one message per packet, fds
//! (memfd, eventfd, pidfd) passed with `SCM_RIGHTS` next to the messages that say so.
//!
//! Owner: task 02 (the codec, hostile-input safe like `bifrost_layers::codec`). The message set is the contract;
//! add variants only at the end.

use crate::address::{FourCC, ObjectId, PropertyAddress, Scope};
use crate::format::{FormatFlags, FormatId, StreamFormat};
use crate::error::AudioError;
use crate::value::PropertyValue;
use alloc::string::String;
use alloc::vec::Vec;

/// The protocol version a client says in `Hello`.
pub const PROTOCOL_VERSION: u32 = 2;

/// The most channels one client IO procedure stream has: the one wire limit the server's `AddIoProc` validation, the
/// client library's scratch arrays and the ring layout all share.
pub const MAX_CLIENT_CHANNELS: usize = 32;
/// The largest graph quantum, in frames (bounds the ring layout together with `MAX_CLIENT_CHANNELS`).
pub const MAX_RING_QUANTUM: usize = 4096;

/// The control socket path.
pub const CONTROL_SOCKET: &str = "/run/bifrost/audio/control";

/// What a client's IO procedure does on a device.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IoProcRequest {
    pub device: ObjectId,
    pub input_channels: u32,
    pub output_channels: u32,
    /// The buffer size this client wants, in frames (the server runs the device at the smallest any client asked
    /// for and calls larger clients every N cycles).
    pub buffer_frames: u32,
}

/// Client to server.
#[derive(Debug, Clone, PartialEq)]
pub enum Request {
    /// First message: protocol version, the client's name (app id) and whether it asks for policy rights (only the
    /// policy process; the server checks the peer's uid).
    Hello { version: u32, client: String, policy: bool },
    Get { object: ObjectId, address: PropertyAddress },
    Set { object: ObjectId, address: PropertyAddress, value: PropertyValue },
    Listen { object: ObjectId, address: PropertyAddress },
    Unlisten { object: ObjectId, address: PropertyAddress },
    /// Lists the objects owned by `object` (devices of the system, streams and controls of a device).
    Owned { object: ObjectId },
    /// Adds an IO procedure. The reply `IoProcAdded` comes with fds: the memfd (rings and activation record) and
    /// the eventfd (wake).
    AddIoProc(IoProcRequest),
    RemoveIoProc { id: u32 },
    StartIoProc { id: u32 },
    StopIoProc { id: u32 },
    /// Asks for real-time priority for the client's IO thread `tid` (the server checks it with the pidfd sent
    /// alongside, then sets SCHED_FIFO 80 and the workgroup's uclamp).
    PromoteThread { tid: i32 },
}

/// Server to client.
#[derive(Debug, Clone, PartialEq)]
pub enum Reply {
    Welcome { version: u32, client_id: u32 },
    Value(PropertyValue),
    Done,
    Objects(Vec<ObjectId>),
    /// The IO procedure's id and the layout of its shared memory (offsets in the memfd sent alongside).
    IoProcAdded { id: u32, layout: ShmLayout },
    Error(AudioError),
    /// Coalesced change notices for listened addresses (re-read the values).
    Changed(Vec<(ObjectId, PropertyAddress)>),
    /// The client's IO procedure missed its deadline this many times since the last notice.
    Overload { id: u32, count: u32 },
}

/// Where things are in an IO procedure's memfd (byte offsets and sizes), as `bifrost-audio-shm` lays them out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ShmLayout {
    pub activation: u32,
    pub zero_stamp: u32,
    pub input_ring: u32,
    pub output_ring: u32,
    pub ring_frames: u32,
    pub total: u32,
    /// Offset of the server's per-client "done" activation record: the client's `finish` targets it so the server's
    /// wait for the client ends at once. Appended last (protocol 2).
    pub done: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecodeError {
    Truncated,
    UnknownTag(u8),
    BadValue,
    TooLong,
}

/// The largest message.
pub const MAX_LEN: usize = 64 * 1024;

struct W(Vec<u8>);

impl W {
    fn u8(&mut self, v: u8) {
        self.0.push(v);
    }
    fn u32(&mut self, v: u32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn i32(&mut self, v: i32) {
        self.u32(v as u32);
    }
    fn f64(&mut self, v: f64) {
        self.0.extend_from_slice(&v.to_bits().to_le_bytes());
    }
    fn bytes(&mut self, b: &[u8]) {
        self.u32(b.len() as u32);
        self.0.extend_from_slice(b);
    }
    fn addr(&mut self, a: &PropertyAddress) {
        self.u32(a.selector.as_u32());
        self.u8(match a.scope {
            Scope::Global => 0,
            Scope::Input => 1,
            Scope::Output => 2,
            Scope::PlayThrough => 3,
        });
        self.u32(a.element);
    }
    fn format(&mut self, f: &StreamFormat) {
        self.f64(f.sample_rate);
        self.u8(0); // FormatId::LinearPcm
        self.u32(f.flags.0);
        self.u32(f.bytes_per_packet);
        self.u32(f.frames_per_packet);
        self.u32(f.bytes_per_frame);
        self.u32(f.channels);
        self.u32(f.bits_per_channel);
    }
    fn ids(&mut self, os: &[ObjectId]) {
        self.u32(os.len() as u32);
        for o in os {
            self.u32(o.0);
        }
    }
    fn value(&mut self, v: &PropertyValue) {
        match v {
            PropertyValue::Bool(b) => {
                self.u8(0);
                self.u8(*b as u8)
            }
            PropertyValue::U32(x) => {
                self.u8(1);
                self.u32(*x)
            }
            PropertyValue::I32(x) => {
                self.u8(2);
                self.i32(*x)
            }
            PropertyValue::F32(x) => {
                self.u8(3);
                self.u32(x.to_bits())
            }
            PropertyValue::F64(x) => {
                self.u8(4);
                self.f64(*x)
            }
            PropertyValue::String(s) => {
                self.u8(5);
                self.bytes(s.as_bytes())
            }
            PropertyValue::Object(o) => {
                self.u8(6);
                self.u32(o.0)
            }
            PropertyValue::Objects(os) => {
                self.u8(7);
                self.ids(os)
            }
            PropertyValue::RangeU32(a, b) => {
                self.u8(8);
                self.u32(*a);
                self.u32(*b)
            }
            PropertyValue::Rates(rs) => {
                self.u8(9);
                self.u32(rs.len() as u32);
                for (a, b) in rs {
                    self.f64(*a);
                    self.f64(*b);
                }
            }
            PropertyValue::Format(f) => {
                self.u8(10);
                self.format(f)
            }
            PropertyValue::Formats(fs) => {
                self.u8(11);
                self.u32(fs.len() as u32);
                for f in fs {
                    self.format(f);
                }
            }
            PropertyValue::ChannelPair(a, b) => {
                self.u8(12);
                self.u32(*a);
                self.u32(*b)
            }
            PropertyValue::Pid(p) => {
                self.u8(13);
                self.i32(*p)
            }
            PropertyValue::Bytes(b) => {
                self.u8(14);
                self.bytes(b)
            }
        }
    }
}

const ERRORS: [AudioError; 11] = [
    AudioError::UnknownObject,
    AudioError::UnknownProperty,
    AudioError::NotSettable,
    AudioError::BadValueType,
    AudioError::BadValue,
    AudioError::Hogged,
    AudioError::PermissionDenied,
    AudioError::DeviceGone,
    AudioError::NoServer,
    AudioError::Protocol,
    AudioError::Io,
];

struct R<'a> {
    b: &'a [u8],
    pos: usize,
}

impl<'a> R<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], DecodeError> {
        let end = self.pos.checked_add(n).ok_or(DecodeError::Truncated)?;
        if end > self.b.len() {
            return Err(DecodeError::Truncated);
        }
        let s = &self.b[self.pos..end];
        self.pos = end;
        Ok(s)
    }
    fn u8(&mut self) -> Result<u8, DecodeError> {
        Ok(self.take(1)?[0])
    }
    fn u32(&mut self) -> Result<u32, DecodeError> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn i32(&mut self) -> Result<i32, DecodeError> {
        Ok(self.u32()? as i32)
    }
    fn f64(&mut self) -> Result<f64, DecodeError> {
        Ok(f64::from_bits(u64::from_le_bytes(self.take(8)?.try_into().unwrap())))
    }
    /// An element count, checked against the bytes left so a hostile count cannot allocate.
    fn count(&mut self, elem: usize) -> Result<usize, DecodeError> {
        let n = self.u32()? as usize;
        if n.saturating_mul(elem) > self.b.len() - self.pos {
            return Err(DecodeError::Truncated);
        }
        Ok(n)
    }
    fn bytes(&mut self) -> Result<&'a [u8], DecodeError> {
        let n = self.count(1)?;
        self.take(n)
    }
    fn string(&mut self) -> Result<String, DecodeError> {
        let b = self.bytes()?;
        core::str::from_utf8(b).map(String::from).map_err(|_| DecodeError::BadValue)
    }
    fn boolean(&mut self) -> Result<bool, DecodeError> {
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(DecodeError::BadValue),
        }
    }
    fn addr(&mut self) -> Result<PropertyAddress, DecodeError> {
        let selector = FourCC::from_u32(self.u32()?);
        let scope = match self.u8()? {
            0 => Scope::Global,
            1 => Scope::Input,
            2 => Scope::Output,
            3 => Scope::PlayThrough,
            _ => return Err(DecodeError::BadValue),
        };
        Ok(PropertyAddress { selector, scope, element: self.u32()? })
    }
    fn format(&mut self) -> Result<StreamFormat, DecodeError> {
        let sample_rate = self.f64()?;
        if self.u8()? != 0 {
            return Err(DecodeError::BadValue);
        }
        Ok(StreamFormat {
            sample_rate,
            format: FormatId::LinearPcm,
            flags: FormatFlags(self.u32()?),
            bytes_per_packet: self.u32()?,
            frames_per_packet: self.u32()?,
            bytes_per_frame: self.u32()?,
            channels: self.u32()?,
            bits_per_channel: self.u32()?,
        })
    }
    fn ids(&mut self) -> Result<Vec<ObjectId>, DecodeError> {
        let n = self.count(4)?;
        (0..n).map(|_| self.u32().map(ObjectId)).collect()
    }
    fn value(&mut self) -> Result<PropertyValue, DecodeError> {
        Ok(match self.u8()? {
            0 => PropertyValue::Bool(self.boolean()?),
            1 => PropertyValue::U32(self.u32()?),
            2 => PropertyValue::I32(self.i32()?),
            3 => PropertyValue::F32(f32::from_bits(self.u32()?)),
            4 => PropertyValue::F64(self.f64()?),
            5 => PropertyValue::String(self.string()?),
            6 => PropertyValue::Object(ObjectId(self.u32()?)),
            7 => PropertyValue::Objects(self.ids()?),
            8 => PropertyValue::RangeU32(self.u32()?, self.u32()?),
            9 => {
                let n = self.count(16)?;
                let mut v = Vec::with_capacity(n);
                for _ in 0..n {
                    v.push((self.f64()?, self.f64()?));
                }
                PropertyValue::Rates(v)
            }
            10 => PropertyValue::Format(self.format()?),
            11 => {
                let n = self.count(37)?;
                let mut v = Vec::with_capacity(n);
                for _ in 0..n {
                    v.push(self.format()?);
                }
                PropertyValue::Formats(v)
            }
            12 => PropertyValue::ChannelPair(self.u32()?, self.u32()?),
            13 => PropertyValue::Pid(self.i32()?),
            14 => PropertyValue::Bytes(self.bytes()?.to_vec()),
            t => return Err(DecodeError::UnknownTag(t)),
        })
    }
    fn done(&self) -> Result<(), DecodeError> {
        if self.pos == self.b.len() {
            Ok(())
        } else {
            Err(DecodeError::BadValue)
        }
    }
}

fn reader(b: &[u8]) -> Result<R<'_>, DecodeError> {
    if b.len() > MAX_LEN {
        return Err(DecodeError::TooLong);
    }
    Ok(R { b, pos: 0 })
}

pub fn encode_request(m: &Request) -> Vec<u8> {
    let mut w = W(Vec::new());
    match m {
        Request::Hello { version, client, policy } => {
            w.u8(1);
            w.u32(*version);
            w.bytes(client.as_bytes());
            w.u8(*policy as u8);
        }
        Request::Get { object, address } => {
            w.u8(2);
            w.u32(object.0);
            w.addr(address);
        }
        Request::Set { object, address, value } => {
            w.u8(3);
            w.u32(object.0);
            w.addr(address);
            w.value(value);
        }
        Request::Listen { object, address } => {
            w.u8(4);
            w.u32(object.0);
            w.addr(address);
        }
        Request::Unlisten { object, address } => {
            w.u8(5);
            w.u32(object.0);
            w.addr(address);
        }
        Request::Owned { object } => {
            w.u8(6);
            w.u32(object.0);
        }
        Request::AddIoProc(r) => {
            w.u8(7);
            w.u32(r.device.0);
            w.u32(r.input_channels);
            w.u32(r.output_channels);
            w.u32(r.buffer_frames);
        }
        Request::RemoveIoProc { id } => {
            w.u8(8);
            w.u32(*id);
        }
        Request::StartIoProc { id } => {
            w.u8(9);
            w.u32(*id);
        }
        Request::StopIoProc { id } => {
            w.u8(10);
            w.u32(*id);
        }
        Request::PromoteThread { tid } => {
            w.u8(11);
            w.i32(*tid);
        }
    }
    w.0
}

pub fn decode_request(b: &[u8]) -> Result<Request, DecodeError> {
    let mut r = reader(b)?;
    let m = match r.u8()? {
        1 => Request::Hello { version: r.u32()?, client: r.string()?, policy: r.boolean()? },
        2 => Request::Get { object: ObjectId(r.u32()?), address: r.addr()? },
        3 => Request::Set { object: ObjectId(r.u32()?), address: r.addr()?, value: r.value()? },
        4 => Request::Listen { object: ObjectId(r.u32()?), address: r.addr()? },
        5 => Request::Unlisten { object: ObjectId(r.u32()?), address: r.addr()? },
        6 => Request::Owned { object: ObjectId(r.u32()?) },
        7 => Request::AddIoProc(IoProcRequest {
            device: ObjectId(r.u32()?),
            input_channels: r.u32()?,
            output_channels: r.u32()?,
            buffer_frames: r.u32()?,
        }),
        8 => Request::RemoveIoProc { id: r.u32()? },
        9 => Request::StartIoProc { id: r.u32()? },
        10 => Request::StopIoProc { id: r.u32()? },
        11 => Request::PromoteThread { tid: r.i32()? },
        t => return Err(DecodeError::UnknownTag(t)),
    };
    r.done()?;
    Ok(m)
}

pub fn encode_reply(m: &Reply) -> Vec<u8> {
    let mut w = W(Vec::new());
    match m {
        Reply::Welcome { version, client_id } => {
            w.u8(1);
            w.u32(*version);
            w.u32(*client_id);
        }
        Reply::Value(v) => {
            w.u8(2);
            w.value(v);
        }
        Reply::Done => w.u8(3),
        Reply::Objects(os) => {
            w.u8(4);
            w.ids(os);
        }
        Reply::IoProcAdded { id, layout } => {
            w.u8(5);
            w.u32(*id);
            for v in [layout.activation, layout.zero_stamp, layout.input_ring, layout.output_ring, layout.ring_frames, layout.total, layout.done] {
                w.u32(v);
            }
        }
        Reply::Error(e) => {
            w.u8(6);
            w.u8(ERRORS.iter().position(|x| x == e).unwrap_or(ERRORS.len() - 1) as u8);
        }
        Reply::Changed(list) => {
            w.u8(7);
            w.u32(list.len() as u32);
            for (o, a) in list {
                w.u32(o.0);
                w.addr(a);
            }
        }
        Reply::Overload { id, count } => {
            w.u8(8);
            w.u32(*id);
            w.u32(*count);
        }
    }
    w.0
}

pub fn decode_reply(b: &[u8]) -> Result<Reply, DecodeError> {
    let mut r = reader(b)?;
    let m = match r.u8()? {
        1 => Reply::Welcome { version: r.u32()?, client_id: r.u32()? },
        2 => Reply::Value(r.value()?),
        3 => Reply::Done,
        4 => Reply::Objects(r.ids()?),
        5 => Reply::IoProcAdded {
            id: r.u32()?,
            layout: ShmLayout {
                activation: r.u32()?,
                zero_stamp: r.u32()?,
                input_ring: r.u32()?,
                output_ring: r.u32()?,
                ring_frames: r.u32()?,
                total: r.u32()?,
                done: r.u32()?,
            },
        },
        6 => Reply::Error(*ERRORS.get(r.u8()? as usize).ok_or(DecodeError::BadValue)?),
        7 => {
            let n = r.count(13)?;
            let mut v = Vec::with_capacity(n);
            for _ in 0..n {
                v.push((ObjectId(r.u32()?), r.addr()?));
            }
            Reply::Changed(v)
        }
        8 => Reply::Overload { id: r.u32()?, count: r.u32()? },
        t => return Err(DecodeError::UnknownTag(t)),
    };
    r.done()?;
    Ok(m)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::address::SYSTEM_OBJECT;
    use crate::props;
    use alloc::vec;

    fn fmt() -> StreamFormat {
        StreamFormat {
            sample_rate: 48000.0,
            format: FormatId::LinearPcm,
            flags: FormatFlags::FLOAT | FormatFlags::PACKED,
            bytes_per_packet: 8,
            frames_per_packet: 1,
            bytes_per_frame: 8,
            channels: 2,
            bits_per_channel: 32,
        }
    }

    #[test]
    fn request_layout_by_hand() {
        let m = Request::Get { object: ObjectId(2), address: PropertyAddress::global(props::NOMINAL_RATE) };
        // tag 2, object 2 LE, selector "nsrt" as LE u32 (0x6e737274), scope 0, element 0
        let want = [2u8, 2, 0, 0, 0, 0x74, 0x72, 0x73, 0x6e, 0, 0, 0, 0, 0];
        assert_eq!(encode_request(&m), want);
        assert_eq!(decode_request(&want), Ok(m));
    }

    #[test]
    fn requests_round_trip() {
        let a = PropertyAddress::scoped(props::VOLUME_SCALAR, Scope::Output);
        let all = vec![
            Request::Hello { version: PROTOCOL_VERSION, client: "is.olibuijr.test".into(), policy: true },
            Request::Get { object: SYSTEM_OBJECT, address: a },
            Request::Set { object: ObjectId(9), address: a, value: PropertyValue::F32(0.5) },
            Request::Listen { object: ObjectId(3), address: a },
            Request::Unlisten { object: ObjectId(3), address: a },
            Request::Owned { object: SYSTEM_OBJECT },
            Request::AddIoProc(IoProcRequest { device: ObjectId(4), input_channels: 2, output_channels: 2, buffer_frames: 64 }),
            Request::RemoveIoProc { id: 1 },
            Request::StartIoProc { id: 2 },
            Request::StopIoProc { id: 3 },
            Request::PromoteThread { tid: -5 },
        ];
        for m in all {
            assert_eq!(decode_request(&encode_request(&m)), Ok(m));
        }
    }

    #[test]
    fn every_value_round_trips() {
        let vals = vec![
            PropertyValue::Bool(true),
            PropertyValue::U32(7),
            PropertyValue::I32(-7),
            PropertyValue::F32(0.25),
            PropertyValue::F64(44100.0),
            PropertyValue::String("Built-in".into()),
            PropertyValue::Object(ObjectId(5)),
            PropertyValue::Objects(vec![ObjectId(1), ObjectId(2)]),
            PropertyValue::RangeU32(32, 4096),
            PropertyValue::Rates(vec![(44100.0, 44100.0), (48000.0, 96000.0)]),
            PropertyValue::Format(fmt()),
            PropertyValue::Formats(vec![fmt(), fmt()]),
            PropertyValue::ChannelPair(1, 2),
            PropertyValue::Pid(1234),
            PropertyValue::Bytes(vec![1, 2, 3]),
        ];
        for v in vals {
            let r = Reply::Value(v);
            assert_eq!(decode_reply(&encode_reply(&r)), Ok(r));
        }
    }

    #[test]
    fn replies_round_trip() {
        let a = PropertyAddress::global(props::DEVICES);
        let all = vec![
            Reply::Welcome { version: 1, client_id: 8 },
            Reply::Done,
            Reply::Objects(vec![ObjectId(2)]),
            Reply::IoProcAdded {
                id: 1,
                layout: ShmLayout { activation: 0, zero_stamp: 64, input_ring: 128, output_ring: 4224, ring_frames: 1024, total: 8320, done: 8256 },
            },
            Reply::Error(AudioError::NotSettable),
            Reply::Error(AudioError::Io),
            Reply::Changed(vec![(SYSTEM_OBJECT, a), (ObjectId(2), a)]),
            Reply::Overload { id: 1, count: 3 },
        ];
        for m in all {
            assert_eq!(decode_reply(&encode_reply(&m)), Ok(m));
        }
    }

    #[test]
    fn hostile_input_is_refused() {
        assert_eq!(decode_request(&[]), Err(DecodeError::Truncated));
        assert_eq!(decode_request(&[99]), Err(DecodeError::UnknownTag(99)));
        // every truncation of a valid message fails cleanly
        let full = encode_request(&Request::Set {
            object: ObjectId(1),
            address: PropertyAddress::global(props::NAME),
            value: PropertyValue::String("abc".into()),
        });
        for n in 0..full.len() {
            assert!(decode_request(&full[..n]).is_err());
        }
        // a huge element count with no data does not allocate
        let mut b = vec![4u8];
        b.extend_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(decode_reply(&b), Err(DecodeError::Truncated));
        // trailing bytes, bad scope, bad bool, too long, bad error index
        let mut t = encode_reply(&Reply::Done);
        t.push(0);
        assert_eq!(decode_reply(&t), Err(DecodeError::BadValue));
        assert_eq!(decode_request(&[2, 1, 0, 0, 0, 0, 0, 0, 0, 9, 0, 0, 0, 0]), Err(DecodeError::BadValue));
        assert_eq!(decode_request(&[1, 1, 0, 0, 0, 0, 0, 0, 0, 7]), Err(DecodeError::BadValue));
        assert_eq!(decode_request(&vec![0u8; MAX_LEN + 1]), Err(DecodeError::TooLong));
        assert_eq!(decode_reply(&[6, 200]), Err(DecodeError::BadValue));
    }

    #[test]
    fn every_error_round_trips_and_the_table_is_complete() {
        for (i, e) in ERRORS.iter().enumerate() {
            let b = encode_reply(&Reply::Error(*e));
            assert_eq!(b, [6, i as u8]);
            assert_eq!(decode_reply(&b), Ok(Reply::Error(*e)));
        }
        for (i, a) in ERRORS.iter().enumerate() {
            assert!(!ERRORS[i + 1..].contains(a), "{a:?} is listed twice");
        }
    }

    #[test]
    fn every_truncation_of_every_message_fails_cleanly() {
        let a = PropertyAddress::scoped(props::STREAMS, Scope::Input);
        let reqs = [
            Request::Hello { version: 1, client: "x".into(), policy: false },
            Request::Set { object: ObjectId(1), address: a, value: PropertyValue::Formats(vec![fmt()]) },
            Request::AddIoProc(IoProcRequest { device: ObjectId(4), input_channels: 0, output_channels: 2, buffer_frames: 128 }),
            Request::PromoteThread { tid: 77 },
        ];
        for m in &reqs {
            let b = encode_request(m);
            for n in 0..b.len() {
                assert!(decode_request(&b[..n]).is_err(), "{m:?} cut at {n}");
            }
        }
        let reps = [
            Reply::Welcome { version: 1, client_id: 2 },
            Reply::Value(PropertyValue::Rates(vec![(8000.0, 192000.0)])),
            Reply::Objects(vec![ObjectId(1), ObjectId(2)]),
            Reply::IoProcAdded { id: 1, layout: ShmLayout::default() },
            Reply::Changed(vec![(ObjectId(2), a)]),
            Reply::Overload { id: 1, count: 1 },
        ];
        for m in &reps {
            let b = encode_reply(m);
            for n in 0..b.len() {
                assert!(decode_reply(&b[..n]).is_err(), "{m:?} cut at {n}");
            }
        }
    }

    #[test]
    fn strings_must_be_utf8() {
        let mut b = encode_request(&Request::Hello { version: 1, client: "ab".into(), policy: false });
        // tag, version (4), length (4), then the two string bytes
        b[9] = 0xff;
        assert_eq!(decode_request(&b), Err(DecodeError::BadValue));
    }

    #[test]
    fn max_channels_fit_the_ring_layout() {
        // one ring of the widest stream at the largest quantum, doubled for the ring's slack, still fits a u32 offset
        let bytes = MAX_CLIENT_CHANNELS as u64 * MAX_RING_QUANTUM as u64 * 2 * 4;
        assert_eq!(bytes, 1_048_576);
        assert!(2 * bytes + 64 * 4 < u32::MAX as u64);
    }

    #[test]
    fn random_bytes_never_panic() {
        // A fixed LCG so the sweep is reproducible; decoding must return, not panic or over-allocate.
        let mut x: u64 = 0x2545_f491_4f6c_dd1d;
        let mut buf = Vec::new();
        for round in 0..20_000 {
            buf.clear();
            let len = round % 64;
            for _ in 0..len {
                x = x.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                buf.push((x >> 56) as u8);
            }
            if let Some(t) = buf.first_mut() {
                *t %= 12;
            }
            let _ = decode_request(&buf);
            let _ = decode_reply(&buf);
        }
    }
}
