//! Raw HID access through the kernel's `hidraw` nodes (`/dev/hidrawN`): listing the devices with their identity and
//! report descriptor, and reading input reports, writing output reports and exchanging feature reports.
//!
//! Unlike evdev, a hidraw node is not grabbed exclusively: opening one does not take the device away from the system,
//! so the shell can read a device's reports for an app while the desktop keeps using it. That also makes a hidraw node
//! the most direct way to log what a person types, so [`protected`] marks the devices an app may never open:
//! keyboards, keypads and security keys (FIDO). Everything else needs the person's consent for that one device
//! (`usb_grants`).
//!
//! The sysfs layout read here is `/sys/class/hidraw/hidrawN/device/{uevent,report_descriptor}`; `uevent` carries
//! `HID_ID=bus:vendor:product` (8 hex digits each), `HID_NAME` and `HID_UNIQ`. Pure parsing is separated from the
//! device calls so it is tested on text.

use std::ffi::c_void;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::fd::AsRawFd;
use std::os::raw::{c_int, c_ulong};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

/// Where the kernel lists hidraw devices.
pub const SYSFS_HIDRAW: &str = "/sys/class/hidraw";
/// The longest report one call reads or writes (the kernel's hidraw buffer is 16 KiB; apps never need that much).
pub const MAX_REPORT: usize = 4096;

const O_NONBLOCK: i32 = 0o4000;

/// `_IOC(_IOC_READ | _IOC_WRITE, 'H', number, size)`: the hidraw feature-report ioctls.
const fn hid_ioc(number: c_ulong, size: usize) -> c_ulong {
    (3 << 30) | ((size as c_ulong) << 16) | ((b'H' as c_ulong) << 8) | number
}

/// `HIDIOCSFEATURE(len)`.
pub const fn set_feature_request(len: usize) -> c_ulong {
    hid_ioc(0x06, len)
}

/// `HIDIOCGFEATURE(len)`.
pub const fn get_feature_request(len: usize) -> c_ulong {
    hid_ioc(0x07, len)
}

extern "C" {
    fn ioctl(fd: c_int, request: c_ulong, ...) -> c_int;
}

/// The bus a HID device sits on (`HID_ID`'s first field, the kernel's `BUS_*` numbers).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Bus {
    Usb,
    Bluetooth,
    I2c,
    Other(u16),
}

impl Bus {
    pub fn from_number(n: u16) -> Bus {
        match n {
            0x03 => Bus::Usb,
            0x05 => Bus::Bluetooth,
            0x18 => Bus::I2c,
            other => Bus::Other(other),
        }
    }

    /// The name apps see.
    pub fn name(self) -> &'static str {
        match self {
            Bus::Usb => "usb",
            Bus::Bluetooth => "bluetooth",
            Bus::I2c => "i2c",
            Bus::Other(_) => "other",
        }
    }
}

/// The fields of a hidraw device's `uevent` file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Uevent {
    pub bus: Bus,
    pub vendor_id: u16,
    pub product_id: u16,
    pub name: String,
    pub unique: String,
}

/// Parses `uevent` text. `None` when there is no well-formed `HID_ID` line.
pub fn parse_uevent(text: &str) -> Option<Uevent> {
    let mut id = None;
    let mut name = String::new();
    let mut unique = String::new();
    for line in text.lines() {
        let Some((key, value)) = line.split_once('=') else { continue };
        match key {
            "HID_ID" => {
                let mut parts = value.split(':');
                let (Some(bus), Some(vendor), Some(product), None) = (parts.next(), parts.next(), parts.next(), parts.next()) else {
                    continue;
                };
                let parse = |s: &str| u32::from_str_radix(s.trim(), 16).ok();
                if let (Some(bus), Some(vendor), Some(product)) = (parse(bus), parse(vendor), parse(product)) {
                    // the kernel prints each as 8 hex digits; a value above 16 bits is not a USB-style id
                    if bus <= 0xffff && vendor <= 0xffff && product <= 0xffff {
                        id = Some((bus as u16, vendor as u16, product as u16));
                    }
                }
            }
            "HID_NAME" => name = value.trim().to_string(),
            "HID_UNIQ" => unique = value.trim().to_string(),
            _ => {}
        }
    }
    let (bus, vendor_id, product_id) = id?;
    Some(Uevent { bus: Bus::from_number(bus), vendor_id, product_id, name, unique })
}

/// The `(usage page, usage)` of every top-level application collection of a report descriptor, in order. Reads the
/// short items only; a descriptor that ends inside an item gives what was found before it.
pub fn top_level_usages(descriptor: &[u8]) -> Vec<(u16, u16)> {
    let mut found = Vec::new();
    let mut page: u16 = 0;
    let mut usage: Option<(u16, u16)> = None;
    let mut depth = 0usize;
    let mut i = 0;
    while i < descriptor.len() {
        let prefix = descriptor[i];
        if prefix == 0xfe {
            // long item: the next byte is the data length
            let Some(&len) = descriptor.get(i + 1) else { break };
            i += 3 + usize::from(len);
            continue;
        }
        let size = match prefix & 3 {
            0 => 0usize,
            1 => 1,
            2 => 2,
            _ => 4,
        };
        if i + 1 + size > descriptor.len() {
            break;
        }
        let data = &descriptor[i + 1..i + 1 + size];
        let value = data.iter().rev().fold(0u32, |acc, &b| (acc << 8) | u32::from(b));
        match prefix & 0xfc {
            0x04 => page = value as u16,
            // a four-byte usage carries its own page in the high half
            0x08 => usage = Some(if size == 4 && value > 0xffff { ((value >> 16) as u16, value as u16) } else { (page, value as u16) }),
            0xa0 => {
                if depth == 0 && value == 1 {
                    found.push(usage.unwrap_or((page, 0)));
                }
                depth += 1;
                usage = None;
            }
            0xc0 => {
                depth = depth.saturating_sub(1);
                usage = None;
            }
            _ => {}
        }
        i += 1 + size;
    }
    found
}

/// Whether an app may never open a device whose top-level collections are `usages`: keyboards (generic desktop usage 6),
/// keypads (7) and security keys (page 0xF1D0). A device with any such collection is refused as a whole, because a
/// hidraw node carries every collection of the interface.
pub fn protected(usages: &[(u16, u16)]) -> bool {
    usages.iter().any(|&(page, usage)| (page == 0x01 && matches!(usage, 0x06 | 0x07)) || page == 0xf1d0)
}

/// One hidraw device.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HidrawInfo {
    /// `hidrawN`.
    pub name: String,
    /// The node to open.
    pub node: PathBuf,
    pub bus: Bus,
    pub vendor_id: u16,
    pub product_id: u16,
    pub product: String,
    pub unique: String,
    pub descriptor: Vec<u8>,
    pub usages: Vec<(u16, u16)>,
}

impl HidrawInfo {
    /// Whether apps are refused this device outright (see [`protected`]).
    pub fn protected(&self) -> bool {
        protected(&self.usages)
    }
}

fn is_hidraw_name(name: &str) -> bool {
    name.strip_prefix("hidraw").is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
}

/// Lists the hidraw devices under `sysfs` (`/sys/class/hidraw`), in name order of their number. `dev_root` is `/dev`.
/// A directory without a readable `uevent` is skipped.
pub fn list(sysfs: &Path, dev_root: &Path) -> Vec<HidrawInfo> {
    let Ok(entries) = fs::read_dir(sysfs) else { return Vec::new() };
    let mut out: Vec<(u32, HidrawInfo)> = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if !is_hidraw_name(&name) {
            continue;
        }
        let device = entry.path().join("device");
        let Some(uevent) = fs::read_to_string(device.join("uevent")).ok().and_then(|t| parse_uevent(&t)) else { continue };
        let descriptor = fs::read(device.join("report_descriptor")).unwrap_or_default();
        let usages = top_level_usages(&descriptor);
        let number = name["hidraw".len()..].parse().unwrap_or(0);
        out.push((
            number,
            HidrawInfo {
                node: dev_root.join(&name),
                name,
                bus: uevent.bus,
                vendor_id: uevent.vendor_id,
                product_id: uevent.product_id,
                product: uevent.name,
                unique: uevent.unique,
                descriptor,
                usages,
            },
        ));
    }
    out.sort_by_key(|(n, _)| *n);
    out.into_iter().map(|(_, info)| info).collect()
}

/// An open hidraw node. Reads never block: [`HidrawDevice::read_report`] returns `None` when no report is waiting.
pub struct HidrawDevice {
    file: File,
}

impl HidrawDevice {
    /// Opens a node for reading and writing, non-blocking.
    pub fn open(node: &Path) -> io::Result<HidrawDevice> {
        let file = OpenOptions::new().read(true).write(true).custom_flags(O_NONBLOCK).open(node)?;
        Ok(HidrawDevice { file })
    }

    /// Wraps an already open file (tests).
    pub fn from_file(file: File) -> HidrawDevice {
        HidrawDevice { file }
    }

    /// Reads one input report into `buf`. `Ok(None)` when none is waiting. A device without report IDs gives the
    /// payload only; one with report IDs gives the ID as the first byte.
    pub fn read_report(&mut self, buf: &mut [u8]) -> io::Result<Option<usize>> {
        match self.file.read(buf) {
            Ok(n) => Ok(Some(n)),
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Writes one output report (the report ID first, or zero when the device has none).
    pub fn write_report(&mut self, report: &[u8]) -> io::Result<usize> {
        if report.is_empty() || report.len() > MAX_REPORT {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "a report is 1 to 4096 bytes"));
        }
        self.file.write(report)
    }

    fn feature(&self, request: c_ulong, buf: &mut [u8]) -> io::Result<usize> {
        // SAFETY: `file` is an open descriptor for the life of `self`, and `buf` is a live buffer of the length the
        // request number was built from.
        let rc = unsafe { ioctl(self.file.as_raw_fd(), request, buf.as_mut_ptr() as *mut c_void) };
        if rc < 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(rc as usize)
        }
    }

    /// Reads a feature report: `report_id` (0 when the device has none) is sent as the first byte and the answer
    /// comes back in the same buffer, report ID included. `len` counts that first byte.
    pub fn get_feature(&mut self, report_id: u8, len: usize) -> io::Result<Vec<u8>> {
        if len == 0 || len > MAX_REPORT {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "a feature report is 1 to 4096 bytes"));
        }
        let mut buf = vec![0u8; len];
        buf[0] = report_id;
        let got = self.feature(get_feature_request(len), &mut buf)?;
        buf.truncate(got.min(len));
        Ok(buf)
    }

    /// Sends a feature report (the report ID first, or zero when the device has none).
    pub fn set_feature(&mut self, report: &[u8]) -> io::Result<usize> {
        if report.is_empty() || report.len() > MAX_REPORT {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "a feature report is 1 to 4096 bytes"));
        }
        let mut buf = report.to_vec();
        self.feature(set_feature_request(buf.len()), &mut buf)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A boot keyboard: generic desktop page, keyboard usage, one application collection.
    const KEYBOARD: &[u8] = &[0x05, 0x01, 0x09, 0x06, 0xa1, 0x01, 0x05, 0x07, 0x19, 0xe0, 0x29, 0xe7, 0x15, 0x00, 0x25, 0x01, 0x75, 0x01, 0x95, 0x08, 0x81, 0x02, 0xc0];
    /// A mouse with a nested physical collection.
    const MOUSE: &[u8] = &[0x05, 0x01, 0x09, 0x02, 0xa1, 0x01, 0x09, 0x01, 0xa1, 0x00, 0x05, 0x09, 0x19, 0x01, 0x29, 0x03, 0x81, 0x02, 0xc0, 0xc0];
    /// A vendor-defined device (page 0xFF00, usage 1) followed by a second application collection.
    const VENDOR: &[u8] = &[0x06, 0x00, 0xff, 0x09, 0x01, 0xa1, 0x01, 0x09, 0x02, 0x81, 0x02, 0xc0, 0x05, 0x0c, 0x09, 0x01, 0xa1, 0x01, 0xc0];
    /// A FIDO security key (page 0xF1D0, usage 1).
    const FIDO: &[u8] = &[0x06, 0xd0, 0xf1, 0x09, 0x01, 0xa1, 0x01, 0xc0];

    #[test]
    fn ioctl_numbers_match_the_kernel_header() {
        assert_eq!(get_feature_request(64), 0xC040_4807);
        assert_eq!(set_feature_request(64), 0xC040_4806);
        assert_eq!(get_feature_request(0), 0xC000_4807);
    }

    #[test]
    fn a_uevent_gives_the_bus_ids_name_and_unique() {
        let text = "DRIVER=hid-generic\nHID_ID=0003:0000046D:0000C52B\nHID_NAME=Logitech USB Receiver\nHID_PHYS=usb-0000:00:14.0-2/input0\nHID_UNIQ=ABC123\nMODALIAS=hid:b0003g0001v0000046Dp0000C52B\n";
        let u = parse_uevent(text).unwrap();
        assert_eq!(u.bus, Bus::Usb);
        assert_eq!((u.vendor_id, u.product_id), (0x046d, 0xc52b));
        assert_eq!(u.name, "Logitech USB Receiver");
        assert_eq!(u.unique, "ABC123");
        assert_eq!(parse_uevent("HID_ID=0005:00000001:00000002\n").unwrap().bus, Bus::Bluetooth);
        assert_eq!(parse_uevent("HID_ID=0018:00000001:00000002\n").unwrap().bus.name(), "i2c");
    }

    #[test]
    fn a_malformed_uevent_is_refused() {
        for bad in ["", "HID_NAME=x\n", "HID_ID=0003:0000046D\n", "HID_ID=0003:zzzz:0001\n", "HID_ID=0003:00010000:0001\n", "HID_ID=0003:1:2:3\n"] {
            assert_eq!(parse_uevent(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn top_level_usages_find_each_application_collection_once() {
        assert_eq!(top_level_usages(KEYBOARD), vec![(0x01, 0x06)]);
        assert_eq!(top_level_usages(MOUSE), vec![(0x01, 0x02)], "the nested physical collection is not a top-level usage");
        assert_eq!(top_level_usages(VENDOR), vec![(0xff00, 0x01), (0x0c, 0x01)]);
        assert_eq!(top_level_usages(FIDO), vec![(0xf1d0, 0x01)]);
    }

    #[test]
    fn a_hostile_descriptor_never_panics_and_gives_what_came_before() {
        assert!(top_level_usages(&[]).is_empty());
        assert_eq!(top_level_usages(&[0x05, 0x01, 0x09, 0x06, 0xa1, 0x01, 0x05]), vec![(0x01, 0x06)], "cut inside an item");
        assert!(top_level_usages(&[0xfe, 0xff]).is_empty(), "a long item that runs past the end");
        assert!(top_level_usages(&[0xc0, 0xc0, 0xc0]).is_empty(), "end collections with nothing open");
        let noise: Vec<u8> = (0..=255u8).cycle().take(4000).collect();
        let _ = top_level_usages(&noise);
    }

    #[test]
    fn keyboards_keypads_and_security_keys_are_protected() {
        assert!(protected(&top_level_usages(KEYBOARD)));
        assert!(protected(&top_level_usages(FIDO)));
        assert!(protected(&[(0x01, 0x07)]));
        assert!(protected(&[(0xff00, 0x01), (0x01, 0x06)]), "one keyboard collection refuses the whole node");
        assert!(!protected(&top_level_usages(MOUSE)));
        assert!(!protected(&top_level_usages(VENDOR)));
        assert!(!protected(&[]), "a descriptor that did not parse is not a keyboard; the consent sheet still applies");
    }

    fn plug(root: &Path, name: &str, uevent: &str, descriptor: &[u8]) {
        let device = root.join(name).join("device");
        fs::create_dir_all(&device).unwrap();
        fs::write(device.join("uevent"), uevent).unwrap();
        fs::write(device.join("report_descriptor"), descriptor).unwrap();
    }

    #[test]
    fn the_listing_reads_sysfs_in_numeric_order_and_skips_what_is_not_hidraw() {
        let tmp = tempfile::tempdir().unwrap();
        plug(tmp.path(), "hidraw10", "HID_ID=0003:00001234:00000002\nHID_NAME=Pad\n", VENDOR);
        plug(tmp.path(), "hidraw2", "HID_ID=0003:0000046D:0000C52B\nHID_NAME=Keys\nHID_UNIQ=U1\n", KEYBOARD);
        plug(tmp.path(), "hidraw3", "no id here\n", KEYBOARD);
        fs::create_dir_all(tmp.path().join("hidrawX").join("device")).unwrap();
        fs::create_dir_all(tmp.path().join("other")).unwrap();
        let list = list(tmp.path(), Path::new("/dev"));
        let names: Vec<&str> = list.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(names, ["hidraw2", "hidraw10"]);
        assert_eq!(list[0].node, PathBuf::from("/dev/hidraw2"));
        assert_eq!((list[0].vendor_id, list[0].product_id, list[0].unique.as_str()), (0x046d, 0xc52b, "U1"));
        assert!(list[0].protected() && !list[1].protected());
        assert!(super::list(&tmp.path().join("missing"), Path::new("/dev")).is_empty());
    }

    #[test]
    fn a_node_that_is_not_hidraw_refuses_the_feature_calls_and_oversized_reports() {
        let mut d = HidrawDevice::open(Path::new("/dev/null")).unwrap();
        assert!(d.get_feature(1, 8).is_err());
        assert!(d.set_feature(&[1, 2, 3]).is_err());
        assert_eq!(d.get_feature(0, 0).unwrap_err().kind(), io::ErrorKind::InvalidInput);
        assert_eq!(d.get_feature(0, MAX_REPORT + 1).unwrap_err().kind(), io::ErrorKind::InvalidInput);
        assert_eq!(d.set_feature(&[]).unwrap_err().kind(), io::ErrorKind::InvalidInput);
        assert_eq!(d.write_report(&vec![0; MAX_REPORT + 1]).unwrap_err().kind(), io::ErrorKind::InvalidInput);
        assert_eq!(d.write_report(&[0, 1]).unwrap(), 2, "a write to the null device is accepted");
    }

    #[test]
    fn reading_when_no_report_waits_gives_none_and_a_waiting_report_is_returned() {
        use std::os::fd::FromRawFd;
        extern "C" {
            fn pipe2(fds: *mut c_int, flags: c_int) -> c_int;
        }
        let mut fds = [0 as c_int; 2];
        // SAFETY: `fds` has room for the two descriptors pipe2 writes.
        assert_eq!(unsafe { pipe2(fds.as_mut_ptr(), O_NONBLOCK) }, 0);
        // SAFETY: both descriptors were just created and are owned here; each becomes one File.
        let (read_end, mut write_end) = unsafe { (File::from_raw_fd(fds[0]), File::from_raw_fd(fds[1])) };
        let mut d = HidrawDevice::from_file(read_end);
        let mut buf = [0u8; 8];
        assert!(matches!(d.read_report(&mut buf), Ok(None)), "nothing waits: no block, no error");
        write_end.write_all(&[7, 1, 2]).unwrap();
        assert!(matches!(d.read_report(&mut buf), Ok(Some(3))));
        assert_eq!(&buf[..3], &[7, 1, 2]);
    }
}
