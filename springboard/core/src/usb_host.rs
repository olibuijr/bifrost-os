//! USB host transport: the part of the host layer that talks to a device. It opens the kernel's `usbfs` node
//! (`/dev/bus/usb/BBB/DDD`), reads the cached descriptors, claims interfaces and sends control, bulk and interrupt
//! transfers, so the planning types in `usb_claim`, `usb_descriptors` and `usb_pipe` have something to run on.
//!
//! The ioctl numbers and the three request structures follow the kernel's `usbdevice_fs.h` ABI. Everything that does
//! not need a device is pure and tested: the ioctl encoding, the structure sizes, the descriptor split, the error
//! classes and the sysfs listing. The transfer calls are blocking with a timeout in milliseconds; a caller that wants
//! to keep a window responsive runs them on a worker.
//!
//! A sandboxed app only sees the device nodes its sandbox binds; this module does not widen that. The user's approval
//! of a new device is `usb_approval`, which sets the kernel's `authorized` attribute before a node is usable at all.

use crate::usb_claim::ClaimPlan;
use crate::usb_descriptors::{
    parse_configuration, parse_device, request, ConfigurationDescriptor, DeviceDescriptor, Direction, RequestKind, SetupPacket,
};
use std::ffi::c_void;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read};
use std::os::fd::AsRawFd;
use std::os::raw::{c_int, c_ulong};
use std::path::{Path, PathBuf};

const IOC_NONE: c_ulong = 0;
const IOC_WRITE: c_ulong = 1;
const IOC_READ: c_ulong = 2;

/// The Linux `_IOC` encoding: direction in bits 30 and 31, size in bits 16 to 29, type in 8 to 15, number in 0 to 7.
const fn ioc(direction: c_ulong, number: c_ulong, size: usize) -> c_ulong {
    (direction << 30) | ((size as c_ulong) << 16) | ((b'U' as c_ulong) << 8) | number
}

/// `struct usbdevfs_ctrltransfer`.
#[repr(C)]
struct CtrlTransfer {
    request_type: u8,
    request: u8,
    value: u16,
    index: u16,
    length: u16,
    timeout: u32,
    data: *mut c_void,
}

/// `struct usbdevfs_bulktransfer`.
#[repr(C)]
struct BulkTransfer {
    endpoint: u32,
    length: u32,
    timeout: u32,
    data: *mut c_void,
}

/// `struct usbdevfs_setinterface`.
#[repr(C)]
struct SetInterface {
    interface: u32,
    alternate: u32,
}

const USBDEVFS_CONTROL: c_ulong = ioc(IOC_READ | IOC_WRITE, 0, std::mem::size_of::<CtrlTransfer>());
const USBDEVFS_BULK: c_ulong = ioc(IOC_READ | IOC_WRITE, 2, std::mem::size_of::<BulkTransfer>());
const USBDEVFS_SETINTERFACE: c_ulong = ioc(IOC_READ, 4, std::mem::size_of::<SetInterface>());
const USBDEVFS_SETCONFIGURATION: c_ulong = ioc(IOC_READ, 5, std::mem::size_of::<c_int>());
const USBDEVFS_CLAIMINTERFACE: c_ulong = ioc(IOC_READ, 15, std::mem::size_of::<c_int>());
const USBDEVFS_RELEASEINTERFACE: c_ulong = ioc(IOC_READ, 16, std::mem::size_of::<c_int>());
const USBDEVFS_RESET: c_ulong = ioc(IOC_NONE, 20, 0);
const USBDEVFS_CLEAR_HALT: c_ulong = ioc(IOC_READ, 21, std::mem::size_of::<c_int>());

extern "C" {
    fn ioctl(fd: c_int, request: c_ulong, ...) -> c_int;
}

/// The longest control or bulk buffer one call moves. usbfs refuses more than 16 KiB for a control transfer and the
/// kernel splits larger bulk buffers itself, so the cap only keeps the length field of the structures honest.
pub const MAX_TRANSFER: usize = 1 << 20;

/// What went wrong with a transfer, in the terms a driver reacts to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Failure {
    /// The device did not answer within the timeout.
    Timeout,
    /// The endpoint is halted (STALL). The caller clears it with [`UsbHostDevice::clear_halt`] and retries.
    Stall,
    /// The device was unplugged.
    Disconnected,
    /// Another driver owns the interface.
    Busy,
    /// The device sent more than the buffer holds.
    Overflow,
    /// Anything else.
    Other,
}

/// Sorts an I/O error from a transfer into a [`Failure`].
pub fn classify(error: &io::Error) -> Failure {
    match error.raw_os_error() {
        Some(110) => Failure::Timeout,
        Some(32) => Failure::Stall,
        Some(19) | Some(108) => Failure::Disconnected,
        Some(16) => Failure::Busy,
        Some(75) => Failure::Overflow,
        _ => Failure::Other,
    }
}

/// Splits what a read of a usbfs node returns: the 18-byte device descriptor, then every configuration block, each
/// as long as its own `wTotalLength` says.
pub fn split_descriptors(bytes: &[u8]) -> io::Result<(DeviceDescriptor, Vec<ConfigurationDescriptor>)> {
    let bad = |e: crate::usb_descriptors::ParseError| io::Error::new(io::ErrorKind::InvalidData, format!("USB descriptors: {e:?}"));
    let device = parse_device(bytes).map_err(bad)?;
    let mut at = 18;
    let mut configurations = Vec::new();
    while at + 4 <= bytes.len() {
        let total = usize::from(u16::from_le_bytes([bytes[at + 2], bytes[at + 3]]));
        if total < 9 || at + total > bytes.len() {
            return Err(bad(crate::usb_descriptors::ParseError::Truncated));
        }
        configurations.push(parse_configuration(&bytes[at..at + total]).map_err(bad)?);
        at += total;
    }
    Ok((device, configurations))
}

/// The `usbfs` node of a device: `/dev/bus/usb/<bus>/<address>`, both numbers in three digits.
pub fn node_path(dev_root: &Path, bus: u32, address: u32) -> PathBuf {
    dev_root.join("bus").join("usb").join(format!("{bus:03}")).join(format!("{address:03}"))
}

/// One USB device the system lists.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UsbNode {
    /// The sysfs name (`1-2`, `usb1`).
    pub name: String,
    pub bus: u32,
    pub address: u32,
    pub vendor_id: u16,
    pub product_id: u16,
    /// The `usbfs` node to open.
    pub node: PathBuf,
}

fn read_trimmed(path: &Path) -> Option<String> {
    fs::read_to_string(path).ok().map(|s| s.trim().to_string())
}

/// Lists the devices under `sysfs_devices` (`/sys/bus/usb/devices`), in bus and address order. Interface entries
/// (`1-2:1.0`) carry no bus number and are skipped. `dev_root` is `/dev`.
pub fn list_nodes(sysfs_devices: &Path, dev_root: &Path) -> Vec<UsbNode> {
    let mut nodes = Vec::new();
    let Ok(entries) = fs::read_dir(sysfs_devices) else { return nodes };
    for entry in entries.flatten() {
        let dir = entry.path();
        let (Some(bus), Some(address), Some(vendor), Some(product)) = (
            read_trimmed(&dir.join("busnum")).and_then(|s| s.parse::<u32>().ok()),
            read_trimmed(&dir.join("devnum")).and_then(|s| s.parse::<u32>().ok()),
            read_trimmed(&dir.join("idVendor")).and_then(|s| u16::from_str_radix(&s, 16).ok()),
            read_trimmed(&dir.join("idProduct")).and_then(|s| u16::from_str_radix(&s, 16).ok()),
        ) else {
            continue;
        };
        nodes.push(UsbNode {
            name: entry.file_name().to_string_lossy().into_owned(),
            bus,
            address,
            vendor_id: vendor,
            product_id: product,
            node: node_path(dev_root, bus, address),
        });
    }
    nodes.sort_by_key(|n| (n.bus, n.address));
    nodes
}

/// An open USB device.
pub struct UsbHostDevice {
    file: File,
    claimed: Vec<u8>,
}

impl UsbHostDevice {
    /// Opens a `usbfs` node for reading and writing.
    pub fn open(node: &Path) -> io::Result<UsbHostDevice> {
        Ok(UsbHostDevice::from_file(OpenOptions::new().read(true).write(true).open(node)?))
    }

    /// Wraps an already open node (or, in tests, any file).
    pub fn from_file(file: File) -> UsbHostDevice {
        UsbHostDevice { file, claimed: Vec::new() }
    }

    fn call(&self, request: c_ulong, argument: *mut c_void) -> io::Result<c_int> {
        // SAFETY: `file` is an open descriptor for the life of `self`, and every caller passes a pointer to a live
        // structure of the type the request expects (or null for the requests that take no argument).
        let rc = unsafe { ioctl(self.file.as_raw_fd(), request, argument) };
        if rc < 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(rc)
        }
    }

    fn call_int(&self, request: c_ulong, mut value: c_int) -> io::Result<()> {
        self.call(request, &mut value as *mut c_int as *mut c_void).map(|_| ())
    }

    /// Reads the cached device and configuration descriptors from the start of the node.
    pub fn descriptors(&mut self) -> io::Result<(DeviceDescriptor, Vec<ConfigurationDescriptor>)> {
        use std::io::Seek;
        self.file.rewind()?;
        let mut bytes = Vec::new();
        self.file.read_to_end(&mut bytes)?;
        split_descriptors(&bytes)
    }

    /// Claims an interface, so the kernel driver (if any) lets go of it and no other process can use it.
    pub fn claim_interface(&mut self, interface: u8) -> io::Result<()> {
        self.call_int(USBDEVFS_CLAIMINTERFACE, c_int::from(interface))?;
        if !self.claimed.contains(&interface) {
            self.claimed.push(interface);
        }
        Ok(())
    }

    /// Gives an interface back.
    pub fn release_interface(&mut self, interface: u8) -> io::Result<()> {
        self.call_int(USBDEVFS_RELEASEINTERFACE, c_int::from(interface))?;
        self.claimed.retain(|&n| n != interface);
        Ok(())
    }

    /// The interfaces this handle holds.
    pub fn claimed(&self) -> &[u8] {
        &self.claimed
    }

    /// Selects a configuration (`bConfigurationValue`; 0 unconfigures the device).
    pub fn set_configuration(&mut self, value: u8) -> io::Result<()> {
        self.call_int(USBDEVFS_SETCONFIGURATION, c_int::from(value))
    }

    /// Selects an alternate setting of a claimed interface.
    pub fn set_interface(&mut self, interface: u8, alternate: u8) -> io::Result<()> {
        let mut request = SetInterface { interface: u32::from(interface), alternate: u32::from(alternate) };
        self.call(USBDEVFS_SETINTERFACE, &mut request as *mut SetInterface as *mut c_void).map(|_| ())
    }

    /// Clears a halted endpoint (`endpoint` is the address with the direction bit).
    pub fn clear_halt(&mut self, endpoint: u8) -> io::Result<()> {
        self.call_int(USBDEVFS_CLEAR_HALT, c_int::from(endpoint))
    }

    /// Resets the device.
    pub fn reset(&mut self) -> io::Result<()> {
        self.call(USBDEVFS_RESET, std::ptr::null_mut()).map(|_| ())
    }

    /// Sends a control request on endpoint 0. For a device-to-host request `data` receives the answer and the count
    /// of bytes the device sent is returned; for a host-to-device request `data` is sent and its length is returned.
    /// `usbfs` takes `SET_CONFIGURATION` and `SET_INTERFACE` only as their own calls, so those two requests are routed
    /// to [`set_configuration`](Self::set_configuration) and [`set_interface`](Self::set_interface).
    pub fn control(&mut self, setup: &SetupPacket, data: &mut [u8], timeout_ms: u32) -> io::Result<usize> {
        if setup.direction == Direction::HostToDevice && setup.kind == RequestKind::Standard {
            if setup.request == request::SET_CONFIGURATION {
                self.set_configuration(setup.value as u8)?;
                return Ok(0);
            }
            if setup.request == request::SET_INTERFACE {
                self.set_interface(setup.index as u8, setup.value as u8)?;
                return Ok(0);
            }
        }
        if data.len() > usize::from(u16::MAX) {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "control data longer than 65535 bytes"));
        }
        let mut transfer = CtrlTransfer {
            request_type: setup.request_type(),
            request: setup.request,
            value: setup.value,
            index: setup.index,
            length: data.len() as u16,
            timeout: timeout_ms,
            data: data.as_mut_ptr() as *mut c_void,
        };
        let moved = self.call(USBDEVFS_CONTROL, &mut transfer as *mut CtrlTransfer as *mut c_void)?;
        Ok(moved as usize)
    }

    /// Moves a bulk or interrupt transfer on an endpoint (`endpoint` is the address with the direction bit). An IN
    /// endpoint fills `data` and returns the count; an OUT endpoint sends `data` and returns the count sent.
    pub fn transfer(&mut self, endpoint: u8, data: &mut [u8], timeout_ms: u32) -> io::Result<usize> {
        if data.len() > MAX_TRANSFER {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "transfer longer than 1 MiB"));
        }
        let mut transfer = BulkTransfer {
            endpoint: u32::from(endpoint),
            length: data.len() as u32,
            timeout: timeout_ms,
            data: data.as_mut_ptr() as *mut c_void,
        };
        let moved = self.call(USBDEVFS_BULK, &mut transfer as *mut BulkTransfer as *mut c_void)?;
        Ok(moved as usize)
    }

    /// Runs a claim plan for `interface`: claims it, then sends the plan's requests in order.
    pub fn claim(&mut self, plan: &ClaimPlan, interface: u8) -> io::Result<()> {
        self.claim_interface(interface)?;
        for packet in &plan.requests {
            let mut none = [0u8; 0];
            self.control(packet, &mut none, 1000)?;
        }
        Ok(())
    }
}

impl Drop for UsbHostDevice {
    fn drop(&mut self) {
        for interface in std::mem::take(&mut self.claimed) {
            let _ = self.call_int(USBDEVFS_RELEASEINTERFACE, c_int::from(interface));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::usb_descriptors::{get_descriptor, set_configuration, set_interface};
    use std::io::Write;

    /// A device descriptor with one configuration, which holds interface 0 with a bulk IN and a bulk OUT endpoint.
    fn blob() -> Vec<u8> {
        let mut b = vec![18, 1, 0x00, 0x02, 0xff, 0, 0, 64, 0x34, 0x12, 0x07, 0x00, 0x00, 0x01, 1, 2, 3, 1];
        let config = [
            9, 2, 32, 0, 1, 1, 0, 0x80, 50, // configuration, wTotalLength 32
            9, 4, 0, 0, 2, 0xff, 0, 0, 0, // interface 0, two endpoints
            7, 5, 0x81, 0x02, 0x00, 0x02, 0, // bulk IN 512
            7, 5, 0x01, 0x02, 0x00, 0x02, 0, // bulk OUT 512
        ];
        b.extend_from_slice(&config);
        b
    }

    #[test]
    fn ioctl_numbers_match_the_kernel_header() {
        assert_eq!(USBDEVFS_CLAIMINTERFACE, 0x8004_550f);
        assert_eq!(USBDEVFS_RELEASEINTERFACE, 0x8004_5510);
        assert_eq!(USBDEVFS_SETCONFIGURATION, 0x8004_5505);
        assert_eq!(USBDEVFS_CLEAR_HALT, 0x8004_5515);
        assert_eq!(USBDEVFS_RESET, 0x0000_5514);
        assert_eq!(USBDEVFS_SETINTERFACE, 0x8008_5504);
    }

    #[cfg(target_pointer_width = "64")]
    #[test]
    fn transfer_structures_have_the_kernel_sizes_on_64_bit() {
        assert_eq!(std::mem::size_of::<CtrlTransfer>(), 24);
        assert_eq!(std::mem::size_of::<BulkTransfer>(), 24);
        assert_eq!(USBDEVFS_CONTROL, 0xc018_5500);
        assert_eq!(USBDEVFS_BULK, 0xc018_5502);
    }

    #[test]
    fn the_descriptor_blob_splits_into_device_and_configurations() {
        let (device, configs) = split_descriptors(&blob()).unwrap();
        assert_eq!((device.vendor_id, device.product_id, device.configurations), (0x1234, 7, 1));
        assert_eq!(configs.len(), 1);
        let iface = configs[0].interface(0, 0).unwrap();
        assert_eq!(iface.endpoints.len(), 2);
        assert_eq!(iface.endpoints[0].address, 0x81);
    }

    #[test]
    fn a_truncated_blob_is_refused() {
        let mut b = blob();
        b.truncate(b.len() - 3);
        let err = split_descriptors(&b).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        assert!(split_descriptors(&[1, 2, 3]).is_err());
    }

    #[test]
    fn descriptors_are_read_from_the_start_of_the_node() {
        let mut file = tempfile::tempfile().unwrap();
        file.write_all(&blob()).unwrap();
        let mut device = UsbHostDevice::from_file(file);
        let (d, c) = device.descriptors().unwrap();
        assert_eq!(d.product_id, 7);
        assert_eq!(c.len(), 1);
        // a second read sees the same bytes
        assert_eq!(device.descriptors().unwrap().1.len(), 1);
    }

    #[test]
    fn errors_are_classified() {
        assert_eq!(classify(&io::Error::from_raw_os_error(110)), Failure::Timeout);
        assert_eq!(classify(&io::Error::from_raw_os_error(32)), Failure::Stall);
        assert_eq!(classify(&io::Error::from_raw_os_error(19)), Failure::Disconnected);
        assert_eq!(classify(&io::Error::from_raw_os_error(16)), Failure::Busy);
        assert_eq!(classify(&io::Error::from_raw_os_error(75)), Failure::Overflow);
        assert_eq!(classify(&io::Error::from_raw_os_error(5)), Failure::Other);
    }

    #[test]
    fn a_node_that_is_not_usbfs_refuses_the_calls() {
        // /dev/null is a character device that knows none of the usbfs requests, so each call fails with ENOTTY
        let mut device = UsbHostDevice::open(Path::new("/dev/null")).unwrap();
        assert!(device.claim_interface(0).is_err());
        assert!(device.claimed().is_empty());
        let mut buf = [0u8; 8];
        assert!(device.transfer(0x81, &mut buf, 10).is_err());
        let get = get_descriptor(1, 0, 0, 8);
        assert!(device.control(&get, &mut buf, 10).is_err());
        assert!(device.clear_halt(0x81).is_err());
    }

    #[test]
    fn standard_selection_requests_use_their_own_calls() {
        // routed to SETCONFIGURATION and SETINTERFACE, which /dev/null also refuses with ENOTTY rather than the
        // EINVAL a control transfer of those requests would give
        let mut device = UsbHostDevice::open(Path::new("/dev/null")).unwrap();
        let mut none = [0u8; 0];
        let a = device.control(&set_configuration(1), &mut none, 10).unwrap_err();
        let b = device.control(&set_interface(0, 1), &mut none, 10).unwrap_err();
        assert_eq!(a.raw_os_error(), Some(25));
        assert_eq!(b.raw_os_error(), Some(25));
    }

    #[test]
    fn an_oversized_transfer_is_refused_before_the_kernel() {
        let mut device = UsbHostDevice::open(Path::new("/dev/null")).unwrap();
        let mut big = vec![0u8; MAX_TRANSFER + 1];
        let err = device.transfer(0x01, &mut big, 10).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
    }

    #[test]
    fn node_paths_use_three_digits() {
        assert_eq!(node_path(Path::new("/dev"), 1, 4), PathBuf::from("/dev/bus/usb/001/004"));
        assert_eq!(node_path(Path::new("/dev"), 12, 130), PathBuf::from("/dev/bus/usb/012/130"));
    }

    #[test]
    fn the_sysfs_listing_skips_interfaces_and_sorts() {
        let root = tempfile::tempdir().unwrap();
        let make = |name: &str, files: &[(&str, &str)]| {
            let dir = root.path().join(name);
            fs::create_dir_all(&dir).unwrap();
            for (file, text) in files {
                fs::write(dir.join(file), format!("{text}\n")).unwrap();
            }
        };
        make("2-1", &[("busnum", "2"), ("devnum", "5"), ("idVendor", "046d"), ("idProduct", "c52b")]);
        make("1-2", &[("busnum", "1"), ("devnum", "3"), ("idVendor", "1234"), ("idProduct", "0007")]);
        make("1-2:1.0", &[("bInterfaceNumber", "00")]);
        let nodes = list_nodes(root.path(), Path::new("/dev"));
        assert_eq!(nodes.len(), 2);
        assert_eq!((nodes[0].name.as_str(), nodes[0].bus, nodes[0].address), ("1-2", 1, 3));
        assert_eq!(nodes[0].node, PathBuf::from("/dev/bus/usb/001/003"));
        assert_eq!((nodes[1].vendor_id, nodes[1].product_id), (0x046d, 0xc52b));
    }
}
