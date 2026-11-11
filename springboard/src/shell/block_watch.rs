//! The wake-up for block devices: a watch on `/dev` in the shell's poll set, so the `disks` service reads the device
//! list when a disk or a partition arrived or left and never otherwise. An idle shell gets no wake-up from it.
//!
//! The kernel makes and removes a device's node in `/dev` itself, before anything in user space hears of the device,
//! so a create or a delete there is the earliest sign of a plug or a pull. It is the same mechanism the input devices
//! use for `/dev/input` (`bare::input`). Unlike a timer scan it also tells a pull and a plug apart when both happen
//! between two reads of the device list: each is one event with the kernel name ([`Change`]).
//!
//! The watch says that something happened and to which name, nothing more: what the device is comes from the kernel's
//! list under `/sys/block`, as before. A name that is no disk (a terminal, an input node) is dropped by the service
//! when the device list does not know it.
//!
//! The loop polls [`poll_fd`]; `Shell::pump` calls [`pump`] when it is readable; the `disks` service takes what was
//! read on its tick ([`take`]). When the watch cannot be opened the service keeps reading the list on a slow
//! interval, as it did before there was a watch.

use std::cell::RefCell;
use std::ffi::c_int;

/// What happened to a device node, by kernel name (`sdb`, `sdb1`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Change {
    Added(String),
    Removed(String),
}

const IN_CREATE: u32 = 0x100;
const IN_DELETE: u32 = 0x200;
const IN_Q_OVERFLOW: u32 = 0x4000;
const IN_ISDIR: u32 = 0x4000_0000;
const IN_NONBLOCK: c_int = 0o4000;
const IN_CLOEXEC: c_int = 0o2000000;
/// The fixed part of one event: watch, mask, cookie and the length of the name that follows.
const HEADER: usize = 16;
/// The most changes kept between two ticks; past it the names are dropped and the service reads the whole list.
const MAX_PENDING: usize = 256;

extern "C" {
    fn inotify_init1(flags: c_int) -> c_int;
    fn inotify_add_watch(fd: c_int, path: *const std::ffi::c_char, mask: u32) -> c_int;
    fn read(fd: c_int, buf: *mut u8, len: usize) -> isize;
    fn close(fd: c_int) -> c_int;
}

/// True for a plain kernel device name: letters and digits only, as `volume_eject::valid_device` asks of what
/// follows `/dev/`.
fn plain(name: &str) -> bool {
    !name.is_empty() && name.len() <= 32 && name.bytes().all(|b| b.is_ascii_alphanumeric())
}

/// Reads the watch's events in `buf` (one read's bytes): the nodes made and removed, in order, and whether the
/// kernel dropped events because its queue was full. Folders, names that are not plain and a cut-off last event are
/// passed over.
pub fn parse(buf: &[u8]) -> (Vec<Change>, bool) {
    let word = |at: usize| buf.get(at..at + 4).map(|b| u32::from_ne_bytes([b[0], b[1], b[2], b[3]]));
    let (mut out, mut lost, mut at) = (Vec::new(), false, 0);
    while let (Some(mask), Some(len)) = (word(at + 4), word(at + 12)) {
        let Some(raw) = buf.get(at + HEADER..at + HEADER + len as usize) else { break };
        at += HEADER + len as usize;
        lost |= mask & IN_Q_OVERFLOW != 0;
        if mask & IN_ISDIR != 0 {
            continue;
        }
        let end = raw.iter().position(|&b| b == 0).unwrap_or(raw.len());
        let Some(name) = std::str::from_utf8(&raw[..end]).ok().filter(|n| plain(n)) else { continue };
        if mask & IN_CREATE != 0 {
            out.push(Change::Added(name.to_string()));
        } else if mask & IN_DELETE != 0 {
            out.push(Change::Removed(name.to_string()));
        }
    }
    (out, lost)
}

struct Watch {
    fd: c_int,
    tried: bool,
    pending: Vec<Change>,
    lost: bool,
}

thread_local! {
    static WATCH: RefCell<Watch> = const { RefCell::new(Watch { fd: -1, tried: false, pending: Vec::new(), lost: false }) };
}

/// Opens the watch on `/dev` once and says whether there is one. A failure is logged and not tried again.
pub fn open() -> bool {
    WATCH.with(|w| {
        let mut w = w.borrow_mut();
        if !std::mem::replace(&mut w.tried, true) {
            // SAFETY: plain system calls; the path is a constant C string
            let fd = unsafe { inotify_init1(IN_NONBLOCK | IN_CLOEXEC) };
            if fd >= 0 && unsafe { inotify_add_watch(fd, c"/dev".as_ptr(), IN_CREATE | IN_DELETE) } >= 0 {
                w.fd = fd;
            } else {
                if fd >= 0 {
                    // SAFETY: `fd` was opened above and is not kept
                    unsafe { close(fd) };
                }
                eprintln!("disks: no watch on /dev ({}): the device list is read on an interval", std::io::Error::last_os_error());
            }
        }
        w.fd >= 0
    })
}

/// Whether the watch is open.
pub fn active() -> bool {
    WATCH.with(|w| w.borrow().fd >= 0)
}

/// The watch's descriptor for the shell's poll set, `None` before [`open`] and when it failed.
pub fn poll_fd() -> Option<c_int> {
    WATCH.with(|w| Some(w.borrow().fd).filter(|fd| *fd >= 0))
}

/// Reads everything the watch has, so the descriptor is quiet again, and keeps the changes for [`take`].
pub fn pump() {
    WATCH.with(|w| {
        let mut w = w.borrow_mut();
        if w.fd < 0 {
            return;
        }
        let mut buf = [0u8; 4096];
        loop {
            // SAFETY: `buf` is valid for its length; the descriptor does not block
            let n = unsafe { read(w.fd, buf.as_mut_ptr(), buf.len()) };
            if n <= 0 {
                break;
            }
            let (changes, lost) = parse(&buf[..n as usize]);
            w.lost |= lost;
            w.pending.extend(changes);
            if w.pending.len() > MAX_PENDING {
                w.pending.clear();
                w.lost = true;
            }
        }
    });
}

/// The changes since the last call, in order, and whether some were lost (the whole device list is read then).
pub fn take() -> (Vec<Change>, bool) {
    WATCH.with(|w| {
        let mut w = w.borrow_mut();
        (std::mem::take(&mut w.pending), std::mem::take(&mut w.lost))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One event as the kernel writes it: the name padded with zeros to a multiple of 16 bytes.
    fn event(mask: u32, name: &str) -> Vec<u8> {
        let len = if name.is_empty() { 0 } else { (name.len() / 16 + 1) * 16 };
        let mut out = Vec::new();
        out.extend(1i32.to_ne_bytes());
        out.extend(mask.to_ne_bytes());
        out.extend(0u32.to_ne_bytes());
        out.extend((len as u32).to_ne_bytes());
        out.extend(name.bytes());
        out.resize(HEADER + len, 0);
        out
    }

    fn added(name: &str) -> Change {
        Change::Added(name.to_string())
    }

    fn removed(name: &str) -> Change {
        Change::Removed(name.to_string())
    }

    #[test]
    fn a_plug_and_a_pull_are_read_in_order() {
        let buf = [event(IN_CREATE, "sda"), event(IN_CREATE, "sda1"), event(IN_DELETE, "sda1"), event(IN_DELETE, "sda")].concat();
        assert_eq!(parse(&buf), (vec![added("sda"), added("sda1"), removed("sda1"), removed("sda")], false));
        assert_eq!(parse(&[]), (Vec::new(), false));
    }

    #[test]
    fn a_pull_and_a_plug_of_the_same_name_in_one_read_are_both_seen() {
        let buf = [event(IN_DELETE, "sda1"), event(IN_DELETE, "sda"), event(IN_CREATE, "sda"), event(IN_CREATE, "sda1")].concat();
        assert_eq!(parse(&buf).0, vec![removed("sda1"), removed("sda"), added("sda"), added("sda1")]);
    }

    #[test]
    fn folders_and_names_that_are_not_plain_are_passed_over() {
        let buf = [
            event(IN_CREATE | IN_ISDIR, "disk"),
            event(IN_CREATE, "dm-0"),
            event(IN_CREATE, "..sdb"),
            event(IN_CREATE, &"x".repeat(33)),
            event(IN_CREATE, "mmcblk1p1"),
            event(IN_CREATE, "nvme0n1"),
            event(0x4, "sdc"),
        ]
        .concat();
        assert_eq!(parse(&buf), (vec![added("mmcblk1p1"), added("nvme0n1")], false));
    }

    #[test]
    fn a_full_queue_and_a_cut_off_read_lose_nothing_silently() {
        let buf = [event(IN_CREATE, "sdb"), event(IN_Q_OVERFLOW, "")].concat();
        assert_eq!(parse(&buf), (vec![added("sdb")], true), "the service reads the whole list");
        let mut cut = [event(IN_CREATE, "sdb"), event(IN_DELETE, "sdb")].concat();
        cut.truncate(cut.len() - 4);
        assert_eq!(parse(&cut), (vec![added("sdb")], false));
        assert_eq!(parse(&cut[..10]), (Vec::new(), false));
        // a length that runs past the buffer ends the read
        let mut long = event(IN_CREATE, "sdb");
        long[12..16].copy_from_slice(&u32::MAX.to_ne_bytes());
        assert_eq!(parse(&long), (Vec::new(), false));
    }

    #[test]
    fn nothing_is_watched_until_the_service_opens_it() {
        assert_eq!(poll_fd(), None);
        assert!(!active());
        pump();
        assert_eq!(take(), (Vec::new(), false));
    }
}
