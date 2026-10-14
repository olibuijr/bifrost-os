//! Passing a shared buffer to another process over a connected channel.
//!
//! A [`SharedBuffer`] is a memfd region (ashmem where memfd is refused). [`send_buffer`] writes one packet that holds a
//! caller header and passes the region's descriptor with SCM_RIGHTS; [`receive_buffer`] takes the packet back and maps
//! the same memory on the receiving side. Both processes then read and write the same bytes, with no copy. The
//! descriptor replaces the port-based handoff of the reference system: there is no lookup by ID.
//!
//! Not mirrored: global buffer IDs, lookup by name, and transfer between two apps that the shell does not relay.

use crate::proto::sys::{self, Channel, Map, Owned, Recv};
use std::fmt;
use std::io;

/// The largest buffer accepted, in bytes. Larger requests are refused before any memory is made.
pub const MAX_BUFFER_BYTES: usize = 1 << 30;

/// Why a buffer could not be made, sent or received.
#[derive(Debug)]
pub enum BufferError {
    /// A buffer must hold at least one byte.
    Empty,
    /// The requested size is above [`MAX_BUFFER_BYTES`]. Carries the requested size.
    TooLarge(usize),
    /// The packet carried no descriptor, so it does not carry a buffer.
    MissingDescriptor,
    /// The packet carried this many descriptors. A buffer packet carries exactly one.
    ExtraDescriptors(usize),
    /// The other end of the channel is closed.
    Closed,
    /// The operating system refused the call.
    Io(io::Error),
}

impl fmt::Display for BufferError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BufferError::Empty => write!(f, "a buffer must hold at least one byte"),
            BufferError::TooLarge(n) => write!(f, "a buffer of {n} bytes is above the {MAX_BUFFER_BYTES} byte limit"),
            BufferError::MissingDescriptor => write!(f, "the packet carries no buffer descriptor"),
            BufferError::ExtraDescriptors(n) => write!(f, "the packet carries {n} descriptors, not one"),
            BufferError::Closed => write!(f, "the other end of the channel is closed"),
            BufferError::Io(err) => write!(f, "the system refused the buffer call: {err}"),
        }
    }
}

impl std::error::Error for BufferError {}

impl From<io::Error> for BufferError {
    fn from(err: io::Error) -> Self {
        match err.kind() {
            io::ErrorKind::BrokenPipe | io::ErrorKind::ConnectionReset => BufferError::Closed,
            _ => BufferError::Io(err),
        }
    }
}

/// A shared memory region that can be mapped by this process and sent to another one.
#[derive(Debug)]
pub struct SharedBuffer {
    fd: Owned,
    len: usize,
}

impl SharedBuffer {
    /// Makes a region of `len` bytes. `name` labels the memory in the kernel and carries no meaning otherwise.
    pub fn create(name: &str, len: usize) -> Result<Self, BufferError> {
        if len == 0 {
            return Err(BufferError::Empty);
        }
        if len > MAX_BUFFER_BYTES {
            return Err(BufferError::TooLarge(len));
        }
        let fd = sys::shm_create(name, len)?;
        Ok(SharedBuffer { fd, len })
    }

    /// The size of the region in bytes.
    pub fn byte_len(&self) -> usize {
        self.len
    }

    /// Maps the whole region read and write. Writes through the mapping are seen by every process that holds the buffer.
    pub fn map(&self) -> Result<Map, BufferError> {
        Ok(Map::new(&self.fd, self.len)?)
    }

    /// Adopts a descriptor that came in a packet. The size is read from the descriptor itself.
    fn from_descriptor(fd: Owned) -> Result<Self, BufferError> {
        let len = sys::shm_len(&fd)?;
        if len == 0 {
            return Err(BufferError::Empty);
        }
        if len > MAX_BUFFER_BYTES {
            return Err(BufferError::TooLarge(len));
        }
        Ok(SharedBuffer { fd, len })
    }
}

/// A buffer that arrived in a packet, with the header the sender put beside it.
#[derive(Debug)]
pub struct ReceivedBuffer {
    pub header: Vec<u8>,
    pub buffer: SharedBuffer,
}

/// Sends `buffer` over `channel` with `header` as the packet bytes. The sender keeps its own descriptor and mapping.
pub fn send_buffer(channel: &Channel, header: &[u8], buffer: &SharedBuffer) -> Result<(), BufferError> {
    channel.send(header, &[buffer.fd.0])?;
    Ok(())
}

/// Receives one buffer packet. With `block` false, returns `Ok(None)` when no packet is queued.
pub fn receive_buffer(channel: &Channel, block: bool) -> Result<Option<ReceivedBuffer>, BufferError> {
    match channel.recv(block)? {
        Recv::Empty => Ok(None),
        Recv::Closed => Err(BufferError::Closed),
        Recv::Packet(header, mut fds) => match fds.len() {
            0 => Err(BufferError::MissingDescriptor),
            1 => {
                let buffer = SharedBuffer::from_descriptor(fds.remove(0))?;
                Ok(Some(ReceivedBuffer { header, buffer }))
            }
            n => Err(BufferError::ExtraDescriptors(n)),
        },
    }
}
