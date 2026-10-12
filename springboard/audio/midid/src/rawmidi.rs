//! ALSA rawmidi through the kernel uapi (`sound/asound.h`): the character devices `/dev/snd/midiC<card>D<device>`.
//! No libasound. A port is a plain file descriptor: `read` returns the received MIDI 1.0 bytes, `write` sends them,
//! `poll` waits.

use std::ffi::CString;
use std::io;

/// `struct snd_rawmidi_info`.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct RawmidiInfo {
    pub device: u32,
    pub subdevice: u32,
    pub stream: i32,
    pub card: i32,
    pub flags: u32,
    pub id: [u8; 64],
    pub name: [u8; 80],
    pub subname: [u8; 32],
    pub subdevices_count: u32,
    pub subdevices_avail: u32,
    pub tied_device: i32,
    pub reserved: [u8; 60],
}

/// `SNDRV_RAWMIDI_IOCTL_INFO` = `_IOR('W', 0x01, struct snd_rawmidi_info)`.
pub const IOCTL_INFO: libc::c_ulong = 0x810c_5701;

/// The `stream` value for input (`SNDRV_RAWMIDI_STREAM_INPUT`).
pub const STREAM_INPUT: i32 = 1;

/// A NUL-terminated byte array as text.
pub fn c_text(b: &[u8]) -> String {
    let end = b.iter().position(|&c| c == 0).unwrap_or(b.len());
    String::from_utf8_lossy(&b[..end]).into_owned()
}

/// A rawmidi device found under `/dev/snd`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawDevice {
    pub card: u32,
    pub device: u32,
    pub path: String,
}

/// Parses `midiC<card>D<device>`.
pub fn parse_node_name(name: &str) -> Option<(u32, u32)> {
    let rest = name.strip_prefix("midiC")?;
    let (card, dev) = rest.split_once('D')?;
    Some((card.parse().ok()?, dev.parse().ok()?))
}

/// The rawmidi nodes in `dir`, ordered by card then device.
pub fn scan(dir: &str) -> Vec<RawDevice> {
    let mut out = Vec::new();
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if let Some((card, device)) = parse_node_name(&name) {
                out.push(RawDevice { card, device, path: format!("{dir}/{name}") });
            }
        }
    }
    out.sort_by_key(|d| (d.card, d.device));
    out
}

/// An open rawmidi port.
pub struct RawPort {
    fd: libc::c_int,
}

impl RawPort {
    /// Opens a node non-blocking. Read-write when the device has both directions, else whichever it has.
    pub fn open(path: &str) -> io::Result<RawPort> {
        let c = CString::new(path).map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?;
        for mode in [libc::O_RDWR, libc::O_RDONLY, libc::O_WRONLY] {
            // SAFETY: `c` is a valid NUL-terminated path that outlives the call; open has no other requirements.
            let fd = unsafe { libc::open(c.as_ptr(), mode | libc::O_NONBLOCK | libc::O_CLOEXEC) };
            if fd >= 0 {
                return Ok(RawPort { fd });
            }
        }
        Err(io::Error::last_os_error())
    }

    pub fn fd(&self) -> libc::c_int {
        self.fd
    }

    /// The device's description (`SNDRV_RAWMIDI_IOCTL_INFO`).
    pub fn info(&self) -> io::Result<RawmidiInfo> {
        // SAFETY: RawmidiInfo is plain old data, all-zero is a valid value.
        let mut info: RawmidiInfo = unsafe { std::mem::zeroed() };
        info.stream = STREAM_INPUT;
        // SAFETY: `info` is a live, correctly sized `snd_rawmidi_info` (size pinned by a test); the kernel writes
        // at most that many bytes.
        let r = unsafe { libc::ioctl(self.fd, IOCTL_INFO as _, &mut info as *mut RawmidiInfo) };
        if r < 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(info)
        }
    }

    /// Reads whatever bytes are waiting. `Ok(0)` when nothing is pending.
    pub fn read(&self, buf: &mut [u8]) -> io::Result<usize> {
        // SAFETY: `buf` is a valid writable slice of the length passed.
        let n = unsafe { libc::read(self.fd, buf.as_mut_ptr() as *mut libc::c_void, buf.len()) };
        if n >= 0 {
            return Ok(n as usize);
        }
        let e = io::Error::last_os_error();
        if e.kind() == io::ErrorKind::WouldBlock {
            Ok(0)
        } else {
            Err(e)
        }
    }

    /// Writes MIDI bytes. Returns how many the device took.
    pub fn write(&self, data: &[u8]) -> io::Result<usize> {
        // SAFETY: `data` is a valid readable slice of the length passed.
        let n = unsafe { libc::write(self.fd, data.as_ptr() as *const libc::c_void, data.len()) };
        if n >= 0 {
            return Ok(n as usize);
        }
        let e = io::Error::last_os_error();
        if e.kind() == io::ErrorKind::WouldBlock {
            Ok(0)
        } else {
            Err(e)
        }
    }
}

impl Drop for RawPort {
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
    fn info_layout_matches_the_kernel_header() {
        // Measured against sound/asound.h with a C compiler.
        assert_eq!(size_of::<RawmidiInfo>(), 268);
        assert_eq!(offset_of!(RawmidiInfo, id), 20);
        assert_eq!(offset_of!(RawmidiInfo, name), 84);
        assert_eq!(offset_of!(RawmidiInfo, subdevices_count), 196);
    }

    #[test]
    fn ioctl_number_is_read_direction_w_type_size_268() {
        // dir 2 (read) << 30, size 268 << 16, type 'W' << 8, nr 1.
        let n = (2u64 << 30) | (268 << 16) | ((b'W' as u64) << 8) | 1;
        assert_eq!(n as libc::c_ulong, IOCTL_INFO);
    }

    #[test]
    fn node_names() {
        assert_eq!(parse_node_name("midiC1D0"), Some((1, 0)));
        assert_eq!(parse_node_name("midiC12D3"), Some((12, 3)));
        assert_eq!(parse_node_name("controlC0"), None);
        assert_eq!(parse_node_name("midiCxD0"), None);
    }

    #[test]
    fn c_text_stops_at_nul() {
        assert_eq!(c_text(b"USB MIDI\0garbage"), "USB MIDI");
        assert_eq!(c_text(b"abc"), "abc");
    }

    #[test]
    fn scan_orders_by_card_and_device() {
        let dir = std::env::temp_dir().join(format!("bifrost-midid-scan-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for n in ["midiC1D0", "midiC0D1", "midiC0D0", "seq", "timer"] {
            std::fs::write(dir.join(n), b"").unwrap();
        }
        let got: Vec<(u32, u32)> = scan(dir.to_str().unwrap()).iter().map(|d| (d.card, d.device)).collect();
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(got, vec![(0, 0), (0, 1), (1, 0)]);
    }

    #[test]
    fn read_and_write_on_a_pipe_like_node() {
        // A regular file stands in for the node: write then read back through the same code path.
        let path = std::env::temp_dir().join(format!("bifrost-midid-port-{}", std::process::id()));
        std::fs::write(&path, [0x90, 60, 100]).unwrap();
        let p = RawPort::open(path.to_str().unwrap()).unwrap();
        let mut b = [0u8; 8];
        assert_eq!(p.read(&mut b).unwrap(), 3);
        assert_eq!(&b[..3], &[0x90, 60, 100]);
        assert_eq!(p.read(&mut b).unwrap(), 0);
        assert_eq!(p.write(&[0x80, 60, 0]).unwrap(), 3);
        drop(p);
        std::fs::remove_file(&path).unwrap();
    }
}
