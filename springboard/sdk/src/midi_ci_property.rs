//! MIDI-CI Property Exchange: the capabilities handshake and the Get and Set Property Data messages that move a
//! property (a JSON header naming it, and a body such as a program list) to or from a device.
//!
//! Every message starts with the MIDI-CI common header of `midi_ci` (`F0 7E <device id> 0D <sub-id 2> <version>
//! <source MUID> <destination MUID>`) and ends with `F7`. A capabilities message carries one byte, the number of
//! requests the sender handles at once. A Get or Set message carries a request id, a header, and one chunk of the
//! body with its chunk number and the chunk count. Sizes are 14-bit values sent least significant byte first,
//! every payload byte is 7-bit, and chunks are numbered from 1.
//!
//! `split` cuts a body into chunk messages and `Reassembler` joins chunks that arrive in order. Nothing here opens
//! a port; the caller moves the bytes.

use crate::midi_ci::{self as ci, get14, put14, Muid, SUB_ID_1, SYSEX_END, SYSEX_START, UNIVERSAL_NON_REALTIME, VERSION};

pub const CAPABILITIES_INQUIRY: u8 = 0x30;
pub const CAPABILITIES_REPLY: u8 = 0x31;
pub const GET_INQUIRY: u8 = 0x34;
pub const GET_REPLY: u8 = 0x35;
pub const SET_INQUIRY: u8 = 0x36;
pub const SET_REPLY: u8 = 0x37;

/// The largest value a 14-bit field can carry: the size limit for one chunk, a header, and the chunk count.
pub const MAX_CHUNK_SIZE: usize = 0x3FFF;

/// Bytes before the Property Exchange fields: the common header up to and including the destination MUID.
const COMMON_LEN: usize = 14;

/// Which Property Exchange message this is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    CapabilitiesInquiry,
    CapabilitiesReply,
    GetInquiry,
    GetReply,
    SetInquiry,
    SetReply,
}

impl Kind {
    pub fn sub_id(self) -> u8 {
        match self {
            Kind::CapabilitiesInquiry => CAPABILITIES_INQUIRY,
            Kind::CapabilitiesReply => CAPABILITIES_REPLY,
            Kind::GetInquiry => GET_INQUIRY,
            Kind::GetReply => GET_REPLY,
            Kind::SetInquiry => SET_INQUIRY,
            Kind::SetReply => SET_REPLY,
        }
    }

    pub fn from_sub_id(sub_id: u8) -> Option<Kind> {
        Some(match sub_id {
            CAPABILITIES_INQUIRY => Kind::CapabilitiesInquiry,
            CAPABILITIES_REPLY => Kind::CapabilitiesReply,
            GET_INQUIRY => Kind::GetInquiry,
            GET_REPLY => Kind::GetReply,
            SET_INQUIRY => Kind::SetInquiry,
            SET_REPLY => Kind::SetReply,
            _ => return None,
        })
    }

    /// Whether the message carries a request id, header and chunk (Get and Set) rather than a capability count.
    fn carries_data(self) -> bool {
        !matches!(self, Kind::CapabilitiesInquiry | Kind::CapabilitiesReply)
    }
}

/// One chunk of a Get or Set message. `index` runs from 1 to `count`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Chunk {
    pub request_id: u8,
    pub header: Vec<u8>,
    pub index: u16,
    pub count: u16,
    pub data: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Body {
    /// Capabilities inquiry and reply: the number of requests the sender handles at once.
    Capabilities { max_requests: u8 },
    /// Get and Set messages.
    Data(Chunk),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Message {
    pub device_id: u8,
    pub source: Muid,
    pub destination: Muid,
    pub kind: Kind,
    pub body: Body,
}

/// Why a Property Exchange message could not be built or read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// The common MIDI-CI framing is wrong; the inner error says how.
    Framing(ci::Error),
    /// The kind and the body do not match (a capability body on a Get message, for example).
    BodyMismatch,
    /// A field or payload byte has its high bit set, so it cannot be sent in SysEx.
    NotSevenBit,
    /// A header, body or chunk count is larger than a 14-bit field holds.
    FieldTooLarge,
    /// Chunk numbers are zero, or the index is past the count.
    BadChunk,
    /// A chunk arrived out of order.
    OutOfOrder { expected: u16, got: u16 },
    /// A chunk belongs to another request, or its count differs from the chunks before it.
    RequestMismatch,
}

impl From<ci::Error> for Error {
    fn from(e: ci::Error) -> Error {
        Error::Framing(e)
    }
}

fn seven(b: u8) -> Result<u8, Error> {
    if b & 0x80 == 0 {
        Ok(b)
    } else {
        Err(Error::NotSevenBit)
    }
}

fn put_seven_bytes(out: &mut Vec<u8>, bytes: &[u8]) -> Result<(), Error> {
    for b in bytes {
        out.push(seven(*b)?);
    }
    Ok(())
}

impl Chunk {
    fn encode_into(&self, out: &mut Vec<u8>) -> Result<(), Error> {
        let count = self.count as usize;
        if self.count == 0 || self.index == 0 || self.index > self.count || count > MAX_CHUNK_SIZE {
            return Err(Error::BadChunk);
        }
        if self.header.len() > MAX_CHUNK_SIZE || self.data.len() > MAX_CHUNK_SIZE {
            return Err(Error::FieldTooLarge);
        }
        out.push(seven(self.request_id)?);
        put14(out, self.header.len() as u16);
        put_seven_bytes(out, &self.header)?;
        put14(out, self.count);
        put14(out, self.index);
        put14(out, self.data.len() as u16);
        put_seven_bytes(out, &self.data)
    }

    fn decode(p: &[u8]) -> Result<Chunk, Error> {
        let truncated = Error::Framing(ci::Error::Truncated);
        let request_id = *p.first().ok_or(truncated)?;
        let rest = &p[1..];
        if rest.len() < 2 {
            return Err(truncated);
        }
        let header_len = get14(rest) as usize;
        let rest = &rest[2..];
        if rest.len() < header_len + 6 {
            return Err(truncated);
        }
        let header = rest[..header_len].to_vec();
        let rest = &rest[header_len..];
        let count = get14(rest);
        let index = get14(&rest[2..]);
        let data_len = get14(&rest[4..]) as usize;
        let rest = &rest[6..];
        if rest.len() != data_len {
            return Err(Error::BadChunk);
        }
        if count == 0 || index == 0 || index > count {
            return Err(Error::BadChunk);
        }
        Ok(Chunk { request_id, header, index, count, data: rest.to_vec() })
    }
}

impl Message {
    /// The SysEx bytes, `F0` through `F7`.
    pub fn encode(&self) -> Result<Vec<u8>, Error> {
        if self.kind.carries_data() != matches!(self.body, Body::Data(_)) {
            return Err(Error::BodyMismatch);
        }
        let mut out = vec![SYSEX_START, UNIVERSAL_NON_REALTIME, self.device_id & 0x7F, SUB_ID_1, self.kind.sub_id(), VERSION];
        out.extend_from_slice(&self.source.to_bytes());
        out.extend_from_slice(&self.destination.to_bytes());
        match &self.body {
            Body::Capabilities { max_requests } => out.push(seven(*max_requests)?),
            Body::Data(chunk) => chunk.encode_into(&mut out)?,
        }
        out.push(SYSEX_END);
        Ok(out)
    }

    /// Parses one complete Property Exchange message. MIDI-CI messages of other kinds are `UnknownMessage`.
    pub fn decode(bytes: &[u8]) -> Result<Message, Error> {
        if bytes.len() < COMMON_LEN + 2 {
            return Err(Error::Framing(ci::Error::Truncated));
        }
        if bytes[0] != SYSEX_START || bytes[1] != UNIVERSAL_NON_REALTIME || bytes[3] != SUB_ID_1 {
            return Err(Error::Framing(ci::Error::NotMidiCi));
        }
        if bytes[bytes.len() - 1] != SYSEX_END {
            return Err(Error::Framing(ci::Error::Truncated));
        }
        let kind = Kind::from_sub_id(bytes[4]).ok_or(Error::Framing(ci::Error::UnknownMessage(bytes[4])))?;
        if bytes[1..bytes.len() - 1].iter().any(|b| b & 0x80 != 0) {
            return Err(Error::Framing(ci::Error::BadByte));
        }
        if bytes[5] != VERSION {
            return Err(Error::Framing(ci::Error::UnsupportedVersion(bytes[5])));
        }
        let source = Muid::from_bytes(&bytes[6..10]).ok_or(Error::Framing(ci::Error::BadByte))?;
        let destination = Muid::from_bytes(&bytes[10..14]).ok_or(Error::Framing(ci::Error::BadByte))?;
        let p = &bytes[COMMON_LEN..bytes.len() - 1];
        let body = if kind.carries_data() {
            Body::Data(Chunk::decode(p)?)
        } else {
            Body::Capabilities { max_requests: *p.first().ok_or(Error::Framing(ci::Error::Truncated))? }
        };
        Ok(Message { device_id: bytes[2], source, destination, kind, body })
    }
}

/// Cuts a body into chunks of at most `chunk_size` bytes, numbered from 1. An empty body is one empty chunk.
/// Every chunk carries the same request id and header.
pub fn split(request_id: u8, header: &[u8], body: &[u8], chunk_size: usize) -> Result<Vec<Chunk>, Error> {
    if chunk_size == 0 || chunk_size > MAX_CHUNK_SIZE {
        return Err(Error::FieldTooLarge);
    }
    let pieces: Vec<&[u8]> = if body.is_empty() { vec![body] } else { body.chunks(chunk_size).collect() };
    if pieces.len() > MAX_CHUNK_SIZE {
        return Err(Error::FieldTooLarge);
    }
    let count = pieces.len() as u16;
    Ok(pieces
        .into_iter()
        .enumerate()
        .map(|(i, data)| Chunk {
            request_id,
            header: header.to_vec(),
            index: i as u16 + 1,
            count,
            data: data.to_vec(),
        })
        .collect())
}

/// The whole of a Get or Set message once every chunk has arrived.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Assembled {
    pub header: Vec<u8>,
    pub data: Vec<u8>,
}

/// Joins the chunks of one request. Chunks must arrive in order with the same request id and count. An error leaves
/// the partial message in place; call `reset` to drop it.
#[derive(Clone, Debug, Default)]
pub struct Reassembler {
    request_id: u8,
    count: u16,
    /// The number of the next chunk expected; 0 when no message is in progress.
    next: u16,
    header: Vec<u8>,
    data: Vec<u8>,
}

impl Reassembler {
    pub fn new() -> Reassembler {
        Reassembler::default()
    }

    pub fn reset(&mut self) {
        *self = Reassembler::default();
    }

    /// Adds the next chunk. Returns the whole message when this is its last chunk.
    pub fn push(&mut self, chunk: &Chunk) -> Result<Option<Assembled>, Error> {
        if self.next == 0 {
            if chunk.index != 1 {
                return Err(Error::OutOfOrder { expected: 1, got: chunk.index });
            }
            self.request_id = chunk.request_id;
            self.count = chunk.count;
            self.header = chunk.header.clone();
            self.data.clear();
            self.next = 1;
        } else if chunk.request_id != self.request_id || chunk.count != self.count {
            return Err(Error::RequestMismatch);
        } else if chunk.index != self.next {
            return Err(Error::OutOfOrder { expected: self.next, got: chunk.index });
        }
        self.data.extend_from_slice(&chunk.data);
        if self.next == self.count {
            self.next = 0;
            return Ok(Some(Assembled {
                header: std::mem::take(&mut self.header),
                data: std::mem::take(&mut self.data),
            }));
        }
        self.next += 1;
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ME: Muid = Muid(0x0123_456);
    const PEER: Muid = Muid(0x0ABC_DEF);

    fn get_chunk(index: u16, count: u16, data: &[u8]) -> Chunk {
        Chunk { request_id: 3, header: b"{\"resource\":\"X\"}".to_vec(), index, count, data: data.to_vec() }
    }

    #[test]
    fn kind_sub_ids_round_trip() {
        for kind in [
            Kind::CapabilitiesInquiry,
            Kind::CapabilitiesReply,
            Kind::GetInquiry,
            Kind::GetReply,
            Kind::SetInquiry,
            Kind::SetReply,
        ] {
            assert_eq!(Kind::from_sub_id(kind.sub_id()), Some(kind));
        }
        assert_eq!(Kind::from_sub_id(0x20), None);
    }

    #[test]
    fn capabilities_message_has_expected_bytes() {
        let m = Message {
            device_id: 0x7F,
            source: ME,
            destination: PEER,
            kind: Kind::CapabilitiesReply,
            body: Body::Capabilities { max_requests: 4 },
        };
        let bytes = m.encode().unwrap();
        assert_eq!(bytes.len(), 16);
        assert_eq!(&bytes[..6], &[0xF0, 0x7E, 0x7F, 0x0D, 0x31, 0x01]);
        assert_eq!(bytes[14], 4);
        assert_eq!(bytes[15], 0xF7);
        assert_eq!(Message::decode(&bytes), Ok(m));
    }

    #[test]
    fn get_message_round_trips_with_header_and_chunk() {
        let m = Message {
            device_id: 1,
            source: ME,
            destination: PEER,
            kind: Kind::GetReply,
            body: Body::Data(get_chunk(2, 3, b"[1,2,3]")),
        };
        let bytes = m.encode().unwrap();
        assert!(bytes[1..bytes.len() - 1].iter().all(|b| b & 0x80 == 0));
        assert_eq!(Message::decode(&bytes), Ok(m));
    }

    #[test]
    fn encode_rejects_mismatched_body_and_bad_chunk_numbers() {
        let mismatch = Message {
            device_id: 0,
            source: ME,
            destination: PEER,
            kind: Kind::SetInquiry,
            body: Body::Capabilities { max_requests: 1 },
        };
        assert_eq!(mismatch.encode(), Err(Error::BodyMismatch));
        let zero = Message {
            device_id: 0,
            source: ME,
            destination: PEER,
            kind: Kind::SetInquiry,
            body: Body::Data(get_chunk(0, 1, b"")),
        };
        assert_eq!(zero.encode(), Err(Error::BadChunk));
        let past = Message { body: Body::Data(get_chunk(3, 2, b"")), ..zero };
        assert_eq!(past.encode(), Err(Error::BadChunk));
    }

    #[test]
    fn encode_rejects_high_bit_bytes() {
        let m = Message {
            device_id: 0,
            source: ME,
            destination: PEER,
            kind: Kind::GetInquiry,
            body: Body::Data(get_chunk(1, 1, &[0x41, 0x80])),
        };
        assert_eq!(m.encode(), Err(Error::NotSevenBit));
        let bad_request = Chunk { request_id: 0x90, ..get_chunk(1, 1, b"") };
        let m2 = Message { body: Body::Data(bad_request), ..m };
        assert_eq!(m2.encode(), Err(Error::NotSevenBit));
    }

    #[test]
    fn decode_reports_framing_errors() {
        assert_eq!(Message::decode(&[0xF0, 0x7E]), Err(Error::Framing(ci::Error::Truncated)));
        let profile_inquiry = [0xF0, 0x7E, 0x7F, 0x0D, 0x20, 0x01, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xF7];
        assert_eq!(
            Message::decode(&profile_inquiry),
            Err(Error::Framing(ci::Error::UnknownMessage(0x20)))
        );
        let not_ci = [0xF0, 0x7E, 0x7F, 0x09, 0x34, 0x01, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xF7];
        assert_eq!(Message::decode(&not_ci), Err(Error::Framing(ci::Error::NotMidiCi)));
        let mut cut = Message {
            device_id: 0,
            source: ME,
            destination: PEER,
            kind: Kind::GetInquiry,
            body: Body::Data(get_chunk(1, 1, b"abc")),
        }
        .encode()
        .unwrap();
        cut.remove(cut.len() - 2);
        assert!(Message::decode(&cut).is_err());
    }

    #[test]
    fn split_numbers_chunks_from_one() {
        let chunks = split(9, b"{}", b"0123456789", 4).unwrap();
        assert_eq!(chunks.len(), 3);
        let lens: Vec<usize> = chunks.iter().map(|c| c.data.len()).collect();
        assert_eq!(lens, vec![4, 4, 2]);
        assert_eq!(chunks.iter().map(|c| c.index).collect::<Vec<_>>(), vec![1, 2, 3]);
        assert!(chunks.iter().all(|c| c.count == 3 && c.request_id == 9 && c.header == b"{}"));
    }

    #[test]
    fn split_empty_body_is_one_chunk_and_zero_size_is_refused() {
        let chunks = split(1, b"", b"", 8).unwrap();
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].count, 1);
        assert!(chunks[0].data.is_empty());
        assert_eq!(split(1, b"", b"x", 0), Err(Error::FieldTooLarge));
    }

    #[test]
    fn reassembler_joins_chunks_in_order_and_resets_for_the_next_request() {
        let body = b"the program list, in two parts";
        let chunks = split(5, b"{\"resource\":\"ProgramList\"}", body, 10).unwrap();
        let mut r = Reassembler::new();
        let mut out = None;
        for c in &chunks {
            out = r.push(c).unwrap();
        }
        let done = out.expect("the last chunk completes the message");
        assert_eq!(done.data, body.to_vec());
        assert_eq!(done.header, b"{\"resource\":\"ProgramList\"}".to_vec());
        // A second request starts from chunk 1 again.
        assert_eq!(r.push(&chunks[0]).unwrap(), None);
    }

    #[test]
    fn reassembler_refuses_out_of_order_and_foreign_chunks() {
        let chunks = split(5, b"", b"abcdef", 2).unwrap();
        let mut r = Reassembler::new();
        assert_eq!(r.push(&chunks[1]), Err(Error::OutOfOrder { expected: 1, got: 2 }));
        r.push(&chunks[0]).unwrap();
        assert_eq!(r.push(&chunks[2]), Err(Error::OutOfOrder { expected: 2, got: 3 }));
        let other = Chunk { request_id: 6, ..chunks[1].clone() };
        assert_eq!(r.push(&other), Err(Error::RequestMismatch));
        r.reset();
        assert_eq!(r.push(&chunks[1]), Err(Error::OutOfOrder { expected: 1, got: 2 }));
    }
}
