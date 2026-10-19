//! The wire format between apps and `bifrost-midid` on its unix socket (`is.olibuijr.midi.sock`).
//!
//! A frame is a `u32` little-endian payload length followed by the payload; a payload is a `u16` tag and the fields,
//! all little-endian. Strings are a `u32` length and UTF-8; lists are a `u32` count and the items; a packet is its
//! word count (`u8`, 1 to 4) and the words. The decoder checks that a packet's word count matches its message type,
//! so a broken peer cannot make the server forward a half packet to a hardware port.
//!
//! `Request` goes from an app to the server, `Message` comes back: the replies to a request (`Welcome`, `Setup`,
//! `Created`, `Done`, `Failed`, `Pong`) and the events the server pushes (`Received`, `Delivered`, `SetupChanged`,
//! `SendRefused`).
//! `Send` and `Inject` have no reply; a refused one is answered with the event `SendRefused`, so it can never be
//! mistaken for the reply to a later request.
//!
//! Pure: no socket is touched here. `FrameReader` assembles frames from whatever a non-blocking read returned.

use crate::ump::{words_for_type, Ump};

/// The protocol version a `Hello` carries.
pub const VERSION: u32 = 1;
/// The longest payload either side accepts.
pub const MAX_FRAME: usize = 256 * 1024;
/// The most packets in one `Send` or `Inject`.
pub const MAX_PACKETS: usize = 8192;
/// The longest name (bytes) of a client or endpoint.
pub const MAX_NAME: usize = 256;
/// The socket name under the service run directory.
pub const SOCKET_NAME: &str = "is.olibuijr.midi.sock";

/// Why a payload could not be decoded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WireError {
    /// The payload ended inside a field.
    Short,
    /// A tag this version does not know.
    UnknownTag(u16),
    /// A string that is not UTF-8, or longer than `MAX_NAME`.
    BadString,
    /// A packet whose word count does not match its message type, or a list over its limit.
    BadPacket,
    /// A frame longer than `MAX_FRAME`.
    TooLong,
}

impl std::fmt::Display for WireError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WireError::Short => f.write_str("the message ends inside a field"),
            WireError::UnknownTag(t) => write!(f, "unknown message tag {t}"),
            WireError::BadString => f.write_str("a name is not valid text"),
            WireError::BadPacket => f.write_str("a packet or list is malformed"),
            WireError::TooLong => f.write_str("the frame is too long"),
        }
    }
}

impl std::error::Error for WireError {}

/// Why the server refused a request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// The request needs a `Hello` first, or came twice.
    NoClient,
    /// No such object, or not one this client may use.
    BadObject,
    /// The object is not a source or destination as the request needs.
    WrongDirection,
    /// The connection exists already, or the object does not.
    NotApplicable,
    /// The client version is not supported.
    BadVersion,
}

impl Refusal {
    fn code(self) -> u8 {
        match self {
            Refusal::NoClient => 1,
            Refusal::BadObject => 2,
            Refusal::WrongDirection => 3,
            Refusal::NotApplicable => 4,
            Refusal::BadVersion => 5,
        }
    }

    fn from_code(c: u8) -> Refusal {
        match c {
            1 => Refusal::NoClient,
            2 => Refusal::BadObject,
            3 => Refusal::WrongDirection,
            5 => Refusal::BadVersion,
            _ => Refusal::NotApplicable,
        }
    }
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Refusal::NoClient => "no client yet: say hello first",
            Refusal::BadObject => "no such object for this client",
            Refusal::WrongDirection => "the endpoint has the wrong direction for this request",
            Refusal::NotApplicable => "the request does not apply",
            Refusal::BadVersion => "unsupported protocol version",
        })
    }
}

/// Requests, app to server.
#[derive(Clone, Debug, PartialEq)]
pub enum Request {
    /// Creates the client (once per connection). Reply: `Welcome`.
    Hello { name: String, version: u32 },
    /// Asks for the device, entity and endpoint lists. Reply: `Setup`.
    List,
    /// Creates a virtual source owned by this client. Reply: `Created`.
    CreateSource { name: String },
    /// Creates a virtual destination owned by this client. Reply: `Created`.
    CreateDestination { name: String },
    /// Disposes a virtual endpoint this client created. Reply: `Done`.
    Dispose { id: u32 },
    /// Connects this client's input to a source. Reply: `Done`.
    Connect { source: u32 },
    /// Removes that connection. Reply: `Done`.
    Disconnect { source: u32 },
    /// Sends packets to a destination at `host_ns` (0 or past: now). No reply.
    Send { destination: u32, host_ns: u64, packets: Vec<Ump> },
    /// Injects packets from a virtual source this client owns, to everyone connected. No reply.
    Inject { source: u32, host_ns: u64, packets: Vec<Ump> },
    /// Stores a context value on an endpoint. Reply: `Done`.
    SetRefCon { id: u32, value: u64 },
    /// Reply: `Pong`.
    Ping,
}

/// One device of the setup.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeviceInfo {
    pub id: u32,
    pub name: String,
    pub external: bool,
    pub entities: Vec<u32>,
}

/// One entity of a device.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EntityInfo {
    pub id: u32,
    pub name: String,
    pub device: u32,
    pub sources: Vec<u32>,
    pub destinations: Vec<u32>,
}

/// One source or destination.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EndpointInfo {
    pub id: u32,
    pub name: String,
    /// True for a source, false for a destination.
    pub source: bool,
    /// The owning entity of a hardware endpoint, 0 for a virtual one.
    pub entity: u32,
    /// The owning client of a virtual endpoint, 0 for a hardware one.
    pub owner: u32,
    pub ref_con: u64,
    /// True when this client's input is connected to the source.
    pub connected: bool,
}

/// The devices, entities and endpoints at one moment.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Setup {
    /// Counts every change of the setup; `SetupChanged` carries the new value.
    pub generation: u32,
    pub devices: Vec<DeviceInfo>,
    pub entities: Vec<EntityInfo>,
    pub endpoints: Vec<EndpointInfo>,
}

/// Replies and events, server to app.
#[derive(Clone, Debug, PartialEq)]
pub enum Message {
    Welcome { client: u32, version: u32 },
    Setup(Setup),
    Created { id: u32 },
    Done,
    Failed(Refusal),
    Pong,
    /// A packet from a source this client's input is connected to.
    Received { source: u32, host_ns: u64, packet: Ump },
    /// A packet that arrived at one of this client's virtual destinations.
    Delivered { destination: u32, from: u32, host_ns: u64, packet: Ump },
    /// A device, entity or endpoint appeared or went away: ask for the list again.
    SetupChanged { generation: u32 },
    /// A `Send` or `Inject` was refused (they have no reply of their own).
    SendRefused(Refusal),
}

const T_HELLO: u16 = 1;
const T_LIST: u16 = 2;
const T_CREATE_SOURCE: u16 = 3;
const T_CREATE_DESTINATION: u16 = 4;
const T_DISPOSE: u16 = 5;
const T_CONNECT: u16 = 6;
const T_DISCONNECT: u16 = 7;
const T_SEND: u16 = 8;
const T_INJECT: u16 = 9;
const T_SET_REF_CON: u16 = 10;
const T_PING: u16 = 11;

const T_WELCOME: u16 = 0x101;
const T_SETUP: u16 = 0x102;
const T_CREATED: u16 = 0x103;
const T_DONE: u16 = 0x104;
const T_FAILED: u16 = 0x105;
const T_PONG: u16 = 0x106;
const T_RECEIVED: u16 = 0x110;
const T_DELIVERED: u16 = 0x111;
const T_SETUP_CHANGED: u16 = 0x112;
const T_SEND_REFUSED: u16 = 0x113;

#[derive(Default)]
struct Enc(Vec<u8>);

impl Enc {
    fn tag(t: u16) -> Enc {
        let mut e = Enc(Vec::new());
        e.u16(t);
        e
    }

    fn u8(&mut self, v: u8) {
        self.0.push(v);
    }

    fn u16(&mut self, v: u16) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }

    fn u32(&mut self, v: u32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }

    fn u64(&mut self, v: u64) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }

    fn bool(&mut self, v: bool) {
        self.u8(v as u8);
    }

    fn str(&mut self, s: &str) {
        // cut on a character boundary, so what is sent is still valid text
        let mut n = s.len().min(MAX_NAME);
        while !s.is_char_boundary(n) {
            n -= 1;
        }
        self.u32(n as u32);
        self.0.extend_from_slice(&s.as_bytes()[..n]);
    }

    fn ump(&mut self, p: &Ump) {
        self.u8(p.len);
        for w in p.as_words() {
            self.u32(*w);
        }
    }

    fn packets(&mut self, list: &[Ump]) {
        let n = list.len().min(MAX_PACKETS);
        self.u32(n as u32);
        for p in &list[..n] {
            self.ump(p);
        }
    }

    fn ids(&mut self, ids: &[u32]) {
        self.u32(ids.len() as u32);
        for i in ids {
            self.u32(*i);
        }
    }
}

struct Dec<'a> {
    b: &'a [u8],
    at: usize,
}

impl<'a> Dec<'a> {
    fn new(b: &'a [u8]) -> Dec<'a> {
        Dec { b, at: 0 }
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8], WireError> {
        let end = self.at.checked_add(n).ok_or(WireError::Short)?;
        if end > self.b.len() {
            return Err(WireError::Short);
        }
        let s = &self.b[self.at..end];
        self.at = end;
        Ok(s)
    }

    fn u8(&mut self) -> Result<u8, WireError> {
        Ok(self.take(1)?[0])
    }

    fn u16(&mut self) -> Result<u16, WireError> {
        let b = self.take(2)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }

    fn u32(&mut self) -> Result<u32, WireError> {
        let b = self.take(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn u64(&mut self) -> Result<u64, WireError> {
        let b = self.take(8)?;
        let mut a = [0u8; 8];
        a.copy_from_slice(b);
        Ok(u64::from_le_bytes(a))
    }

    fn bool(&mut self) -> Result<bool, WireError> {
        Ok(self.u8()? != 0)
    }

    fn str(&mut self) -> Result<String, WireError> {
        let n = self.u32()? as usize;
        if n > MAX_NAME {
            return Err(WireError::BadString);
        }
        let b = self.take(n)?;
        String::from_utf8(b.to_vec()).map_err(|_| WireError::BadString)
    }

    fn ump(&mut self) -> Result<Ump, WireError> {
        let len = self.u8()? as usize;
        if !(1..=4).contains(&len) {
            return Err(WireError::BadPacket);
        }
        let mut words = [0u32; 4];
        for w in words.iter_mut().take(len) {
            *w = self.u32()?;
        }
        if words_for_type((words[0] >> 28) as u8) != len {
            return Err(WireError::BadPacket);
        }
        Ok(Ump { words, len: len as u8 })
    }

    fn packets(&mut self) -> Result<Vec<Ump>, WireError> {
        let n = self.u32()? as usize;
        if n > MAX_PACKETS {
            return Err(WireError::BadPacket);
        }
        let mut v = Vec::with_capacity(n);
        for _ in 0..n {
            v.push(self.ump()?);
        }
        Ok(v)
    }

    fn ids(&mut self) -> Result<Vec<u32>, WireError> {
        let n = self.u32()? as usize;
        // every id takes four bytes, so a count the payload cannot hold is refused before allocating
        if n > (self.b.len() - self.at) / 4 {
            return Err(WireError::Short);
        }
        let mut v = Vec::with_capacity(n);
        for _ in 0..n {
            v.push(self.u32()?);
        }
        Ok(v)
    }

    /// A list count checked against the bytes left, each item taking at least `min_item` bytes.
    fn count(&mut self, min_item: usize) -> Result<usize, WireError> {
        let n = self.u32()? as usize;
        if n > (self.b.len() - self.at) / min_item.max(1) {
            return Err(WireError::Short);
        }
        Ok(n)
    }
}

impl Request {
    /// The payload (without the frame length).
    pub fn encode(&self) -> Vec<u8> {
        match self {
            Request::Hello { name, version } => {
                let mut e = Enc::tag(T_HELLO);
                e.str(name);
                e.u32(*version);
                e.0
            }
            Request::List => Enc::tag(T_LIST).0,
            Request::CreateSource { name } => {
                let mut e = Enc::tag(T_CREATE_SOURCE);
                e.str(name);
                e.0
            }
            Request::CreateDestination { name } => {
                let mut e = Enc::tag(T_CREATE_DESTINATION);
                e.str(name);
                e.0
            }
            Request::Dispose { id } => {
                let mut e = Enc::tag(T_DISPOSE);
                e.u32(*id);
                e.0
            }
            Request::Connect { source } => {
                let mut e = Enc::tag(T_CONNECT);
                e.u32(*source);
                e.0
            }
            Request::Disconnect { source } => {
                let mut e = Enc::tag(T_DISCONNECT);
                e.u32(*source);
                e.0
            }
            Request::Send { destination, host_ns, packets } => {
                let mut e = Enc::tag(T_SEND);
                e.u32(*destination);
                e.u64(*host_ns);
                e.packets(packets);
                e.0
            }
            Request::Inject { source, host_ns, packets } => {
                let mut e = Enc::tag(T_INJECT);
                e.u32(*source);
                e.u64(*host_ns);
                e.packets(packets);
                e.0
            }
            Request::SetRefCon { id, value } => {
                let mut e = Enc::tag(T_SET_REF_CON);
                e.u32(*id);
                e.u64(*value);
                e.0
            }
            Request::Ping => Enc::tag(T_PING).0,
        }
    }

    pub fn decode(payload: &[u8]) -> Result<Request, WireError> {
        let mut d = Dec::new(payload);
        Ok(match d.u16()? {
            T_HELLO => Request::Hello { name: d.str()?, version: d.u32()? },
            T_LIST => Request::List,
            T_CREATE_SOURCE => Request::CreateSource { name: d.str()? },
            T_CREATE_DESTINATION => Request::CreateDestination { name: d.str()? },
            T_DISPOSE => Request::Dispose { id: d.u32()? },
            T_CONNECT => Request::Connect { source: d.u32()? },
            T_DISCONNECT => Request::Disconnect { source: d.u32()? },
            T_SEND => Request::Send { destination: d.u32()?, host_ns: d.u64()?, packets: d.packets()? },
            T_INJECT => Request::Inject { source: d.u32()?, host_ns: d.u64()?, packets: d.packets()? },
            T_SET_REF_CON => Request::SetRefCon { id: d.u32()?, value: d.u64()? },
            T_PING => Request::Ping,
            t => return Err(WireError::UnknownTag(t)),
        })
    }
}

impl Message {
    /// True for the events the server pushes on its own (as opposed to the reply to a request).
    pub fn is_event(&self) -> bool {
        matches!(
            self,
            Message::Received { .. } | Message::Delivered { .. } | Message::SetupChanged { .. } | Message::SendRefused(_)
        )
    }

    /// The payload (without the frame length).
    pub fn encode(&self) -> Vec<u8> {
        match self {
            Message::Welcome { client, version } => {
                let mut e = Enc::tag(T_WELCOME);
                e.u32(*client);
                e.u32(*version);
                e.0
            }
            Message::Setup(s) => {
                let mut e = Enc::tag(T_SETUP);
                e.u32(s.generation);
                e.u32(s.devices.len() as u32);
                for d in &s.devices {
                    e.u32(d.id);
                    e.str(&d.name);
                    e.bool(d.external);
                    e.ids(&d.entities);
                }
                e.u32(s.entities.len() as u32);
                for n in &s.entities {
                    e.u32(n.id);
                    e.str(&n.name);
                    e.u32(n.device);
                    e.ids(&n.sources);
                    e.ids(&n.destinations);
                }
                e.u32(s.endpoints.len() as u32);
                for p in &s.endpoints {
                    e.u32(p.id);
                    e.str(&p.name);
                    e.bool(p.source);
                    e.u32(p.entity);
                    e.u32(p.owner);
                    e.u64(p.ref_con);
                    e.bool(p.connected);
                }
                e.0
            }
            Message::Created { id } => {
                let mut e = Enc::tag(T_CREATED);
                e.u32(*id);
                e.0
            }
            Message::Done => Enc::tag(T_DONE).0,
            Message::Failed(r) => {
                let mut e = Enc::tag(T_FAILED);
                e.u8(r.code());
                e.0
            }
            Message::Pong => Enc::tag(T_PONG).0,
            Message::Received { source, host_ns, packet } => {
                let mut e = Enc::tag(T_RECEIVED);
                e.u32(*source);
                e.u64(*host_ns);
                e.ump(packet);
                e.0
            }
            Message::Delivered { destination, from, host_ns, packet } => {
                let mut e = Enc::tag(T_DELIVERED);
                e.u32(*destination);
                e.u32(*from);
                e.u64(*host_ns);
                e.ump(packet);
                e.0
            }
            Message::SetupChanged { generation } => {
                let mut e = Enc::tag(T_SETUP_CHANGED);
                e.u32(*generation);
                e.0
            }
            Message::SendRefused(r) => {
                let mut e = Enc::tag(T_SEND_REFUSED);
                e.u8(r.code());
                e.0
            }
        }
    }

    pub fn decode(payload: &[u8]) -> Result<Message, WireError> {
        let mut d = Dec::new(payload);
        Ok(match d.u16()? {
            T_WELCOME => Message::Welcome { client: d.u32()?, version: d.u32()? },
            T_SETUP => {
                let generation = d.u32()?;
                let n = d.count(13)?;
                let mut devices = Vec::with_capacity(n);
                for _ in 0..n {
                    devices.push(DeviceInfo { id: d.u32()?, name: d.str()?, external: d.bool()?, entities: d.ids()? });
                }
                let n = d.count(20)?;
                let mut entities = Vec::with_capacity(n);
                for _ in 0..n {
                    entities.push(EntityInfo {
                        id: d.u32()?,
                        name: d.str()?,
                        device: d.u32()?,
                        sources: d.ids()?,
                        destinations: d.ids()?,
                    });
                }
                let n = d.count(26)?;
                let mut endpoints = Vec::with_capacity(n);
                for _ in 0..n {
                    endpoints.push(EndpointInfo {
                        id: d.u32()?,
                        name: d.str()?,
                        source: d.bool()?,
                        entity: d.u32()?,
                        owner: d.u32()?,
                        ref_con: d.u64()?,
                        connected: d.bool()?,
                    });
                }
                Message::Setup(Setup { generation, devices, entities, endpoints })
            }
            T_CREATED => Message::Created { id: d.u32()? },
            T_DONE => Message::Done,
            T_FAILED => Message::Failed(Refusal::from_code(d.u8()?)),
            T_PONG => Message::Pong,
            T_RECEIVED => Message::Received { source: d.u32()?, host_ns: d.u64()?, packet: d.ump()? },
            T_DELIVERED => {
                Message::Delivered { destination: d.u32()?, from: d.u32()?, host_ns: d.u64()?, packet: d.ump()? }
            }
            T_SETUP_CHANGED => Message::SetupChanged { generation: d.u32()? },
            T_SEND_REFUSED => Message::SendRefused(Refusal::from_code(d.u8()?)),
            t => return Err(WireError::UnknownTag(t)),
        })
    }
}

/// The bytes to write for one payload: its length, then the payload.
pub fn frame(payload: &[u8]) -> Vec<u8> {
    let mut v = Vec::with_capacity(payload.len() + 4);
    v.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    v.extend_from_slice(payload);
    v
}

/// Assembles frames from the bytes of non-blocking reads.
#[derive(Default)]
pub struct FrameReader {
    buf: Vec<u8>,
}

impl FrameReader {
    pub fn new() -> FrameReader {
        FrameReader::default()
    }

    /// Appends what a read returned.
    pub fn push(&mut self, bytes: &[u8]) {
        self.buf.extend_from_slice(bytes);
    }

    /// The next whole payload, `Ok(None)` while one is still incomplete, and `Err(TooLong)` for a length over
    /// `MAX_FRAME` (the stream cannot be resynchronised: close the connection).
    pub fn next_frame(&mut self) -> Result<Option<Vec<u8>>, WireError> {
        if self.buf.len() < 4 {
            return Ok(None);
        }
        let len = u32::from_le_bytes([self.buf[0], self.buf[1], self.buf[2], self.buf[3]]) as usize;
        if len > MAX_FRAME {
            return Err(WireError::TooLong);
        }
        if self.buf.len() < 4 + len {
            return Ok(None);
        }
        let payload = self.buf[4..4 + len].to_vec();
        self.buf.drain(..4 + len);
        Ok(Some(payload))
    }

    /// Bytes held that do not make a whole frame yet.
    pub fn pending(&self) -> usize {
        self.buf.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::MidiEvent;
    use crate::ump::{from_midi1, sysex_to_ump};

    fn note(key: u8) -> Ump {
        from_midi1(0, MidiEvent::NoteOn { channel: 1, key, velocity: 90 })
    }

    #[test]
    fn requests_round_trip() {
        let sysex = sysex_to_ump(0, &[0x7E, 0x7F, 0x06, 0x01, 0x10, 0x20, 0x30, 0x40]);
        assert!(sysex.len() >= 2);
        let all = vec![
            Request::Hello { name: "DAW \u{e9}".into(), version: VERSION },
            Request::List,
            Request::CreateSource { name: "out".into() },
            Request::CreateDestination { name: "in".into() },
            Request::Dispose { id: 7 },
            Request::Connect { source: 3 },
            Request::Disconnect { source: 3 },
            Request::Send { destination: 9, host_ns: 123_456_789_012, packets: vec![note(60), note(64)] },
            Request::Inject { source: 4, host_ns: 0, packets: sysex },
            Request::SetRefCon { id: 2, value: u64::MAX },
            Request::Ping,
        ];
        for r in all {
            assert_eq!(Request::decode(&r.encode()), Ok(r.clone()), "{r:?}");
        }
    }

    #[test]
    fn messages_round_trip_including_a_setup() {
        let setup = Setup {
            generation: 5,
            devices: vec![DeviceInfo { id: 2, name: "Keys".into(), external: true, entities: vec![3] }],
            entities: vec![EntityInfo { id: 3, name: "Port 1".into(), device: 2, sources: vec![4], destinations: vec![5] }],
            endpoints: vec![
                EndpointInfo { id: 4, name: "Keys out".into(), source: true, entity: 3, owner: 0, ref_con: 0, connected: true },
                EndpointInfo { id: 5, name: "Keys in".into(), source: false, entity: 3, owner: 0, ref_con: 9, connected: false },
                EndpointInfo { id: 6, name: "Virtual".into(), source: true, entity: 0, owner: 1, ref_con: 0, connected: false },
            ],
        };
        let all = vec![
            Message::Welcome { client: 1, version: VERSION },
            Message::Setup(setup),
            Message::Setup(Setup::default()),
            Message::Created { id: 12 },
            Message::Done,
            Message::Failed(Refusal::WrongDirection),
            Message::Pong,
            Message::Received { source: 4, host_ns: 99, packet: note(61) },
            Message::Delivered { destination: 8, from: 4, host_ns: 100, packet: note(62) },
            Message::SetupChanged { generation: 6 },
            Message::SendRefused(Refusal::BadObject),
        ];
        for m in all {
            assert_eq!(Message::decode(&m.encode()), Ok(m.clone()), "{m:?}");
        }
    }

    #[test]
    fn events_are_told_apart_from_replies() {
        assert!(Message::SetupChanged { generation: 1 }.is_event());
        assert!(Message::Received { source: 1, host_ns: 0, packet: note(1) }.is_event());
        assert!(!Message::Done.is_event());
        assert!(!Message::Failed(Refusal::BadObject).is_event());
        assert!(Message::SendRefused(Refusal::BadObject).is_event());
    }

    #[test]
    fn a_packet_with_the_wrong_word_count_is_refused() {
        // type 4 (MIDI 2.0 channel voice) takes two words; a single word is a broken packet
        let mut e = Enc::tag(T_SEND);
        e.u32(1);
        e.u64(0);
        e.u32(1);
        e.u8(1);
        e.u32(0x4090_3C00);
        assert_eq!(Request::decode(&e.0), Err(WireError::BadPacket));
        // zero words and five words are refused too
        let mut e = Enc::tag(T_RECEIVED);
        e.u32(1);
        e.u64(0);
        e.u8(0);
        assert_eq!(Message::decode(&e.0), Err(WireError::BadPacket));
        let mut e = Enc::tag(T_RECEIVED);
        e.u32(1);
        e.u64(0);
        e.u8(5);
        assert_eq!(Message::decode(&e.0), Err(WireError::BadPacket));
    }

    #[test]
    fn truncated_payloads_and_unknown_tags_fail_cleanly() {
        let full = Request::Send { destination: 1, host_ns: 2, packets: vec![note(60)] }.encode();
        for cut in 0..full.len() {
            assert!(Request::decode(&full[..cut]).is_err(), "cut at {cut}");
        }
        assert_eq!(Request::decode(&[0xEF, 0xBE]), Err(WireError::UnknownTag(0xBEEF)));
        assert_eq!(Message::decode(&[0x00, 0x00]), Err(WireError::UnknownTag(0)));
    }

    #[test]
    fn too_many_packets_and_huge_counts_are_refused_without_allocating() {
        let mut e = Enc::tag(T_SEND);
        e.u32(1);
        e.u64(0);
        e.u32(MAX_PACKETS as u32 + 1);
        assert_eq!(Request::decode(&e.0), Err(WireError::BadPacket));
        // a setup that claims four billion devices in a few bytes
        let mut e = Enc::tag(T_SETUP);
        e.u32(0);
        e.u32(u32::MAX);
        assert_eq!(Message::decode(&e.0), Err(WireError::Short));
    }

    #[test]
    fn long_names_are_cut_on_a_character_boundary() {
        let name = "\u{e9}".repeat(MAX_NAME); // two bytes each: 512 bytes
        let bytes = Request::CreateSource { name }.encode();
        match Request::decode(&bytes) {
            Ok(Request::CreateSource { name }) => {
                assert_eq!(name.len(), MAX_NAME);
                assert!(name.chars().all(|c| c == '\u{e9}'));
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn bad_utf8_in_a_name_is_refused() {
        let mut e = Enc::tag(T_CREATE_SOURCE);
        e.u32(2);
        e.0.extend_from_slice(&[0xFF, 0xFE]);
        assert_eq!(Request::decode(&e.0), Err(WireError::BadString));
    }

    #[test]
    fn frame_reader_assembles_split_and_joined_frames() {
        let a = frame(&Request::Ping.encode());
        let b = frame(&Request::List.encode());
        let mut stream = a.clone();
        stream.extend_from_slice(&b);
        let mut r = FrameReader::new();
        // one byte at a time: nothing until the first frame is whole
        let mut got = Vec::new();
        for byte in &stream {
            r.push(&[*byte]);
            while let Some(p) = r.next_frame().unwrap() {
                got.push(Request::decode(&p).unwrap());
            }
        }
        assert_eq!(got, vec![Request::Ping, Request::List]);
        assert_eq!(r.pending(), 0);
        // both at once
        r.push(&stream);
        assert!(r.next_frame().unwrap().is_some());
        assert!(r.next_frame().unwrap().is_some());
        assert_eq!(r.next_frame().unwrap(), None);
    }

    #[test]
    fn frame_reader_refuses_an_oversized_length() {
        let mut r = FrameReader::new();
        r.push(&((MAX_FRAME as u32 + 1).to_le_bytes()));
        assert_eq!(r.next_frame(), Err(WireError::TooLong));
    }
}
