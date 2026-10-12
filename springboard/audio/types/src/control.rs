//! The control-socket protocol between clients (the HAL library, the policy process, the compatibility layers,
//! tools) and `bifrost-audiod`: `/run/bifrost/audio/control`, `SOCK_SEQPACKET`, one message per packet, fds
//! (memfd, eventfd, pidfd) passed with `SCM_RIGHTS` next to the messages that say so.
//!
//! Owner: task 02 (the codec, hostile-input safe like `bifrost_layers::codec`). The message set is the contract;
//! add variants only at the end.

use crate::address::{ObjectId, PropertyAddress};
use crate::error::AudioError;
use crate::value::PropertyValue;
use alloc::string::String;
use alloc::vec::Vec;

/// The protocol version a client says in `Hello`.
pub const PROTOCOL_VERSION: u32 = 1;

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

pub fn encode_request(m: &Request) -> Vec<u8> {
    let _ = m;
    todo!("audio-02")
}

pub fn decode_request(b: &[u8]) -> Result<Request, DecodeError> {
    let _ = b;
    todo!("audio-02")
}

pub fn encode_reply(m: &Reply) -> Vec<u8> {
    let _ = m;
    todo!("audio-02")
}

pub fn decode_reply(b: &[u8]) -> Result<Reply, DecodeError> {
    let _ = b;
    todo!("audio-02")
}
