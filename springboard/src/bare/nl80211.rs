//! Wi-Fi power save through nl80211 over generic netlink (`iw` is not on the device).
//!
//! Message building and parsing are pure and tested; `Socket` is the thin libc layer under them.

use std::io;

const NETLINK_GENERIC: i32 = 16;
const GENL_ID_CTRL: u16 = 0x10;
const CTRL_CMD_GETFAMILY: u8 = 3;
const CTRL_ATTR_FAMILY_ID: u16 = 1;
const CTRL_ATTR_FAMILY_NAME: u16 = 2;
const NL80211_CMD_SET_POWER_SAVE: u8 = 61;
const NL80211_CMD_GET_POWER_SAVE: u8 = 62;
const NL80211_ATTR_IFINDEX: u16 = 3;
const NL80211_ATTR_PS_STATE: u16 = 93;
const NLM_F_REQUEST: u16 = 1;
const NLM_F_ACK: u16 = 4;
const NLMSG_ERROR: u16 = 2;
const NLMSG_DONE: u16 = 3;
const HDR: usize = 16;
const GENL_HDR: usize = 4;

fn pad4(n: usize) -> usize {
    (n + 3) & !3
}

fn attr(buf: &mut Vec<u8>, kind: u16, payload: &[u8]) {
    buf.extend_from_slice(&((4 + payload.len()) as u16).to_ne_bytes());
    buf.extend_from_slice(&kind.to_ne_bytes());
    buf.extend_from_slice(payload);
    buf.resize(pad4(buf.len()), 0);
}

/// One netlink message: header, generic netlink header (version 1), then the attributes.
fn message(family: u16, flags: u16, cmd: u8, seq: u32, attrs: &[(u16, Vec<u8>)]) -> Vec<u8> {
    let mut m = vec![0u8; HDR];
    m.extend_from_slice(&[cmd, 1, 0, 0]);
    for (kind, payload) in attrs {
        attr(&mut m, *kind, payload);
    }
    let len = m.len() as u32;
    m[0..4].copy_from_slice(&len.to_ne_bytes());
    m[4..6].copy_from_slice(&family.to_ne_bytes());
    m[6..8].copy_from_slice(&flags.to_ne_bytes());
    m[8..12].copy_from_slice(&seq.to_ne_bytes());
    m
}

/// The attributes of the first data message in `buf`, or the kernel's error (0 = ack).
fn parse(buf: &[u8]) -> Result<Vec<(u16, Vec<u8>)>, i32> {
    let mut off = 0;
    while off + HDR <= buf.len() {
        let len = u32::from_ne_bytes(buf[off..off + 4].try_into().unwrap()) as usize;
        let kind = u16::from_ne_bytes(buf[off + 4..off + 6].try_into().unwrap());
        if len < HDR || off + len > buf.len() {
            break;
        }
        let body = &buf[off + HDR..off + len];
        match kind {
            NLMSG_ERROR if body.len() >= 4 => {
                let e = i32::from_ne_bytes(body[..4].try_into().unwrap());
                if e != 0 {
                    return Err(e);
                }
            }
            NLMSG_DONE => {}
            _ if body.len() >= GENL_HDR => {
                let mut attrs = Vec::new();
                let mut a = GENL_HDR;
                while a + 4 <= body.len() {
                    let alen = u16::from_ne_bytes(body[a..a + 2].try_into().unwrap()) as usize;
                    let akind = u16::from_ne_bytes(body[a + 2..a + 4].try_into().unwrap());
                    if alen < 4 || a + alen > body.len() {
                        break;
                    }
                    attrs.push((akind & 0x3fff, body[a + 4..a + alen].to_vec()));
                    a += pad4(alen);
                }
                return Ok(attrs);
            }
            _ => {}
        }
        off += pad4(len);
    }
    Ok(Vec::new())
}

fn u32_attr(attrs: &[(u16, Vec<u8>)], kind: u16) -> Option<u32> {
    attrs.iter().find(|(k, _)| *k == kind).and_then(|(_, v)| Some(u32::from_ne_bytes(v.get(..4)?.try_into().ok()?)))
}

fn family_request(seq: u32) -> Vec<u8> {
    message(GENL_ID_CTRL, NLM_F_REQUEST, CTRL_CMD_GETFAMILY, seq, &[(CTRL_ATTR_FAMILY_NAME, b"nl80211\0".to_vec())])
}

fn family_id(reply: &[u8]) -> Option<u16> {
    let attrs = parse(reply).ok()?;
    attrs.iter().find(|(k, _)| *k == CTRL_ATTR_FAMILY_ID).and_then(|(_, v)| Some(u16::from_ne_bytes(v.get(..2)?.try_into().ok()?)))
}

fn set_request(family: u16, ifindex: u32, on: bool) -> Vec<u8> {
    message(family, NLM_F_REQUEST | NLM_F_ACK, NL80211_CMD_SET_POWER_SAVE, 2, &[(NL80211_ATTR_IFINDEX, ifindex.to_ne_bytes().to_vec()), (NL80211_ATTR_PS_STATE, (on as u32).to_ne_bytes().to_vec())])
}

fn get_request(family: u16, ifindex: u32) -> Vec<u8> {
    message(family, NLM_F_REQUEST, NL80211_CMD_GET_POWER_SAVE, 3, &[(NL80211_ATTR_IFINDEX, ifindex.to_ne_bytes().to_vec())])
}

#[cfg(unix)]
#[repr(C)]
struct SockaddrNl {
    family: u16,
    pad: u16,
    pid: u32,
    groups: u32,
}

#[cfg(unix)]
use super::sys;

#[cfg(unix)]
struct Socket(i32);

#[cfg(unix)]
impl Socket {
    fn open() -> io::Result<Self> {
        const AF_NETLINK: i32 = 16;
        const SOCK_RAW_CLOEXEC: i32 = 3 | 0o2000000;
        const SOL_SOCKET: i32 = 1;
        const SO_RCVTIMEO: i32 = 20;
        // SAFETY: plain syscalls with valid, correctly sized arguments; the fd is closed in Drop.
        unsafe {
            let s = Socket(sys::check(sys::socket(AF_NETLINK, SOCK_RAW_CLOEXEC, NETLINK_GENERIC))?);
            let addr = SockaddrNl { family: AF_NETLINK as u16, pad: 0, pid: 0, groups: 0 };
            sys::check(sys::bind(s.0, &addr as *const _ as *const _, std::mem::size_of::<SockaddrNl>() as u32))?;
            let tv = sys::timeval { tv_sec: 2, tv_usec: 0 };
            sys::setsockopt(s.0, SOL_SOCKET, SO_RCVTIMEO, &tv as *const _ as *const _, std::mem::size_of::<sys::timeval>() as u32);
            Ok(s)
        }
    }

    fn exchange(&self, msg: &[u8]) -> io::Result<Vec<u8>> {
        // SAFETY: buffers are valid for the lengths passed.
        unsafe {
            sys::check(sys::send(self.0, msg.as_ptr() as *const _, msg.len(), 0) as i32)?;
            let mut buf = vec![0u8; 8192];
            let n = sys::check(sys::recv(self.0, buf.as_mut_ptr() as *mut _, buf.len(), 0) as i32)?;
            buf.truncate(n as usize);
            Ok(buf)
        }
    }
}

#[cfg(unix)]
impl Drop for Socket {
    fn drop(&mut self) {
        // SAFETY: the fd is owned by this value.
        unsafe { sys::close(self.0) };
    }
}

#[cfg(unix)]
fn session(ifname: &str) -> io::Result<(Socket, u16, u32)> {
    let name = std::ffi::CString::new(ifname).map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?;
    // SAFETY: `name` is a valid NUL-terminated string.
    let ifindex = unsafe { sys::if_nametoindex(name.as_ptr()) };
    if ifindex == 0 {
        return Err(io::Error::new(io::ErrorKind::NotFound, format!("no interface {ifname}")));
    }
    let sock = Socket::open()?;
    let family = family_id(&sock.exchange(&family_request(1))?).ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "nl80211 not available"))?;
    Ok((sock, family, ifindex))
}

/// Turns the interface's Wi-Fi power save on or off.
#[cfg(unix)]
pub fn set_power_save(ifname: &str, on: bool) -> io::Result<()> {
    let (sock, family, ifindex) = session(ifname)?;
    let reply = sock.exchange(&set_request(family, ifindex, on))?;
    parse(&reply).map(|_| ()).map_err(|e| io::Error::from_raw_os_error(-e))
}

/// Whether the interface's power save is on.
#[cfg(unix)]
pub fn get_power_save(ifname: &str) -> io::Result<bool> {
    let (sock, family, ifindex) = session(ifname)?;
    let attrs = parse(&sock.exchange(&get_request(family, ifindex))?).map_err(|e| io::Error::from_raw_os_error(-e))?;
    u32_attr(&attrs, NL80211_ATTR_PS_STATE).map(|s| s != 0).ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "no power save state in reply"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_request_is_aligned_and_carries_both_attributes() {
        let m = set_request(0x1c, 7, true);
        assert_eq!(m.len() % 4, 0);
        assert_eq!(u32::from_ne_bytes(m[0..4].try_into().unwrap()) as usize, m.len());
        assert_eq!(u16::from_ne_bytes(m[4..6].try_into().unwrap()), 0x1c);
        assert_eq!(m[HDR], NL80211_CMD_SET_POWER_SAVE);
        let attrs = {
            let mut reply = m.clone();
            reply[4..6].copy_from_slice(&0x1cu16.to_ne_bytes());
            parse(&reply).unwrap()
        };
        assert_eq!(u32_attr(&attrs, NL80211_ATTR_IFINDEX), Some(7));
        assert_eq!(u32_attr(&attrs, NL80211_ATTR_PS_STATE), Some(1));
        assert_eq!(u32_attr(&parse(&set_request(0x1c, 7, false)).unwrap(), NL80211_ATTR_PS_STATE), Some(0));
    }

    #[test]
    fn family_name_is_nul_terminated_and_padded() {
        let m = family_request(1);
        assert_eq!(m.len(), HDR + GENL_HDR + 4 + 8);
        assert_eq!(&m[HDR + GENL_HDR + 4..HDR + GENL_HDR + 12], b"nl80211\0");
    }

    #[test]
    fn family_id_comes_from_the_reply() {
        let reply = message(GENL_ID_CTRL, 0, 1, 1, &[(CTRL_ATTR_FAMILY_ID, 0x25u16.to_ne_bytes().to_vec())]);
        assert_eq!(family_id(&reply), Some(0x25));
    }

    #[test]
    fn kernel_errors_and_acks_are_told_apart() {
        let mut err = vec![0u8; HDR];
        err.extend_from_slice(&(-19i32).to_ne_bytes());
        err.extend_from_slice(&[0u8; 16]);
        let len = err.len() as u32;
        err[0..4].copy_from_slice(&len.to_ne_bytes());
        err[4..6].copy_from_slice(&NLMSG_ERROR.to_ne_bytes());
        assert_eq!(parse(&err), Err(-19));
        err[HDR..HDR + 4].copy_from_slice(&0i32.to_ne_bytes());
        assert_eq!(parse(&err), Ok(vec![]));
    }

    #[test]
    fn truncated_input_does_not_panic() {
        assert_eq!(parse(&[1, 2, 3]), Ok(vec![]));
        let mut m = set_request(1, 1, true);
        m.truncate(m.len() - 3);
        assert_eq!(parse(&m), Ok(vec![]));
    }
}
