//! RFCOMM: a byte stream to a Bluetooth device's channel, the way a serial cable works (`BTPROTO_RFCOMM` on an
//! `AF_BLUETOOTH` socket, the kernel's own RFCOMM layer). The socket is opened with the same libc calls as the mgmt
//! and L2CAP sockets in [`super::bluetooth`]; no daemon is involved.
//!
//! Channels run from 1 to 30. A device names its channels in its service records (SDP), which BifrOSt does not query
//! yet, so the caller has to know the channel. [`sockaddr`] is the pure part: the `sockaddr_rc` the kernel expects, with
//! the address in wire order (least significant byte first, as [`super::bluetooth::parse_addr`] gives it). [`connect`]
//! opens and connects the socket, with the same send timeout as the L2CAP link.
//!
//! Not mirrored yet: no app can open a stream (the shell does not route RFCOMM to apps), and no RFCOMM server side.

use std::io;

use super::bluetooth::parse_addr;

pub const AF_BLUETOOTH: u16 = 31;
pub const BTPROTO_RFCOMM: i32 = 3;
pub const CHANNEL_MIN: u8 = 1;
pub const CHANNEL_MAX: u8 = 30;

/// How long a connect may block before it gives up, in seconds.
pub const CONNECT_SECS: i64 = 10;

/// `struct sockaddr_rc` from the kernel's `net/bluetooth/rfcomm.h`: the family, the device address in wire order, and
/// the channel. 10 bytes.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SockaddrRc {
    pub family: u16,
    pub bdaddr: [u8; 6],
    pub channel: u8,
}

/// The socket address for `channel` on the device `addr` ("AA:BB:CC:DD:EE:FF"). None for a bad address or a channel
/// outside 1 to 30.
pub fn sockaddr(addr: &str, channel: u8) -> Option<SockaddrRc> {
    if !(CHANNEL_MIN..=CHANNEL_MAX).contains(&channel) {
        return None;
    }
    Some(SockaddrRc { family: AF_BLUETOOTH, bdaddr: parse_addr(addr)?, channel })
}

/// An open RFCOMM stream. The socket closes when the stream is dropped.
#[cfg(target_os = "linux")]
pub struct Stream {
    fd: std::os::fd::OwnedFd,
}

#[cfg(target_os = "linux")]
impl Stream {
    /// Reads what the device sent, at most `buf.len()` bytes. Zero means the device closed the channel.
    pub fn read(&self, buf: &mut [u8]) -> io::Result<usize> {
        use std::os::fd::AsRawFd;
        // SAFETY: `buf` is a valid writable slice of `buf.len()` bytes; the fd is open for as long as `self` lives.
        let n = unsafe { super::sys::read(self.fd.as_raw_fd(), buf.as_mut_ptr() as *mut std::ffi::c_void, buf.len()) };
        if n < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(n as usize)
    }

    /// Reads one whole OBEX packet: the three head bytes, then the rest that the length field gives. A closed channel
    /// before the packet ends is an error.
    pub fn read_obex(&self) -> io::Result<Vec<u8>> {
        use bifrost_core::obex_packet::packet_len;
        let mut buf: Vec<u8> = Vec::new();
        let mut want = 3usize;
        while buf.len() < want {
            let mut chunk = [0u8; 512];
            let n = self.read(&mut chunk)?;
            if n == 0 {
                return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "the device closed the channel mid-packet"));
            }
            buf.extend_from_slice(&chunk[..n]);
            want = packet_len(&buf).unwrap_or(3);
        }
        buf.truncate(want);
        Ok(buf)
    }

    /// Sends all of `data`, retrying after partial writes.
    pub fn write_all(&self, mut data: &[u8]) -> io::Result<()> {
        use std::os::fd::AsRawFd;
        while !data.is_empty() {
            // SAFETY: `data` is a valid readable slice; the fd is open for as long as `self` lives.
            let n = unsafe { super::sys::write(self.fd.as_raw_fd(), data.as_ptr() as *const std::ffi::c_void, data.len()) };
            if n < 0 {
                let e = io::Error::last_os_error();
                if e.kind() == io::ErrorKind::Interrupted {
                    continue;
                }
                return Err(e);
            }
            data = &data[n as usize..];
        }
        Ok(())
    }
}

/// Opens a channel to `addr`, sends one request packet and reads the one packet that answers it. Blocks for at most
/// [`CONNECT_SECS`] to connect and as long to get each read or write through.
#[cfg(target_os = "linux")]
pub fn request(addr: &str, channel: u8, packet: &[u8]) -> io::Result<Vec<u8>> {
    let stream = connect(addr, channel)?;
    stream.write_all(packet)?;
    stream.read_obex()
}

/// Opens an RFCOMM socket and connects it to `channel` on `addr`. Blocks for at most [`CONNECT_SECS`].
#[cfg(target_os = "linux")]
pub fn connect(addr: &str, channel: u8) -> io::Result<Stream> {
    use std::os::fd::{FromRawFd, OwnedFd};

    const SOCK_STREAM: i32 = 1;
    const SOL_SOCKET: i32 = 1;
    const SO_RCVTIMEO: i32 = 20;
    const SO_SNDTIMEO: i32 = 21;

    let sa = sockaddr(addr, channel).ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "bad address or channel"))?;
    // SAFETY: plain syscalls; the sockaddr is a valid `sockaddr_rc` of the size passed.
    let fd = super::sys::check(unsafe { super::sys::socket(AF_BLUETOOTH as i32, SOCK_STREAM | super::sys::O_CLOEXEC, BTPROTO_RFCOMM) })?;
    // SAFETY: `fd` is a fresh socket that nothing else owns; it closes on every path from here.
    let fd = unsafe { OwnedFd::from_raw_fd(fd) };
    use std::os::fd::AsRawFd;
    let tv = super::sys::timeval { tv_sec: CONNECT_SECS as std::ffi::c_long, tv_usec: 0 };
    for option in [SO_SNDTIMEO, SO_RCVTIMEO] {
        super::sys::check(unsafe {
            super::sys::setsockopt(fd.as_raw_fd(), SOL_SOCKET, option, &tv as *const _ as *const _, std::mem::size_of::<super::sys::timeval>() as u32)
        })?;
    }
    super::sys::check(unsafe { super::sys::connect(fd.as_raw_fd(), &sa as *const SockaddrRc as *const u8, std::mem::size_of::<SockaddrRc>() as u32) })?;
    Ok(Stream { fd })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_socket_address_is_ten_bytes_with_the_address_in_wire_order() {
        assert_eq!(std::mem::size_of::<SockaddrRc>(), 10);
        let sa = sockaddr("AA:BB:CC:DD:EE:FF", 3).unwrap();
        assert_eq!(sa, SockaddrRc { family: AF_BLUETOOTH, bdaddr: [0xFF, 0xEE, 0xDD, 0xCC, 0xBB, 0xAA], channel: 3 });
    }

    #[test]
    fn only_channels_one_to_thirty_are_addressed() {
        assert!(sockaddr("AA:BB:CC:DD:EE:FF", CHANNEL_MIN).is_some());
        assert!(sockaddr("AA:BB:CC:DD:EE:FF", CHANNEL_MAX).is_some());
        assert_eq!(sockaddr("AA:BB:CC:DD:EE:FF", 0), None);
        assert_eq!(sockaddr("AA:BB:CC:DD:EE:FF", 31), None);
    }

    #[test]
    fn a_request_to_a_bad_address_fails_before_any_socket_opens() {
        assert_eq!(request("not-an-address", 1, &[0x80, 0x00, 0x03]).unwrap_err().kind(), io::ErrorKind::InvalidInput);
    }

    #[test]
    fn a_bad_address_has_no_socket_address() {
        assert_eq!(sockaddr("not-an-address", 1), None);
        assert_eq!(sockaddr("AA:BB:CC:DD:EE", 1), None);
    }
}
