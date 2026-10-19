//! A guarded USB session: what an app may do once the person granted it one device.
//!
//! The usbfs node lets its holder send anything to any interface of the device, so the shell never hands the node to
//! an app. It keeps a [`Session`] per open device and runs the app's requests through the checks here first:
//!
//! - only interfaces the system does not drive may be claimed ([`protected_class`]);
//! - an endpoint may be used only when it belongs to the active alternate setting of a claimed interface, in its own
//!   direction, and as bulk or interrupt (isochronous streams are a separate call, [`Session::isochronous_plan`]);
//! - a control request must be a vendor or class request aimed at the device, a claimed interface or an endpoint of
//!   one; standard requests are refused, because configuration and alternate-setting changes have their own calls
//!   and the rest (address, features) would let an app disturb the bus;
//! - one call moves at most [`MAX_BROKER_BYTES`], so a reply always fits a service body.
//!
//! The session is generic over a [`Transport`], so every rule is tested without hardware; the real transport is
//! `usb_host::UsbHostDevice`.

use crate::usb_descriptors::{
    ConfigurationDescriptor, DeviceDescriptor, Direction, EndpointDescriptor, InterfaceDescriptor, Recipient, RequestKind,
    SetupPacket, TransferType,
};
use crate::usb_grants::protected_class;
use crate::usb_host::UsbHostDevice;
use crate::usb_transfer::{plan_isochronous, IsochFrame, Pipe, PipeError, Speed};
use std::collections::BTreeMap;
use std::fmt;
use std::io;

/// The most bytes one transfer or control call moves. Hex doubles it on the wire, which stays well inside a service
/// body.
pub const MAX_BROKER_BYTES: usize = 4096;
/// The longest timeout an app may ask for, in milliseconds. A longer one is shortened, never refused.
pub const MAX_TIMEOUT_MS: u32 = 5000;
/// The timeout used when the app gives none.
pub const DEFAULT_TIMEOUT_MS: u32 = 1000;

/// The operations a session needs from a device node.
pub trait Transport {
    fn claim_interface(&mut self, interface: u8) -> io::Result<()>;
    fn release_interface(&mut self, interface: u8) -> io::Result<()>;
    fn set_interface(&mut self, interface: u8, alternate: u8) -> io::Result<()>;
    fn clear_halt(&mut self, endpoint: u8) -> io::Result<()>;
    fn control(&mut self, setup: &SetupPacket, data: &mut [u8], timeout_ms: u32) -> io::Result<usize>;
    fn transfer(&mut self, endpoint: u8, data: &mut [u8], timeout_ms: u32) -> io::Result<usize>;
    fn reset(&mut self) -> io::Result<()>;
}

impl Transport for UsbHostDevice {
    fn claim_interface(&mut self, interface: u8) -> io::Result<()> {
        UsbHostDevice::claim_interface(self, interface)
    }
    fn release_interface(&mut self, interface: u8) -> io::Result<()> {
        UsbHostDevice::release_interface(self, interface)
    }
    fn set_interface(&mut self, interface: u8, alternate: u8) -> io::Result<()> {
        UsbHostDevice::set_interface(self, interface, alternate)
    }
    fn clear_halt(&mut self, endpoint: u8) -> io::Result<()> {
        UsbHostDevice::clear_halt(self, endpoint)
    }
    fn control(&mut self, setup: &SetupPacket, data: &mut [u8], timeout_ms: u32) -> io::Result<usize> {
        UsbHostDevice::control(self, setup, data, timeout_ms)
    }
    fn transfer(&mut self, endpoint: u8, data: &mut [u8], timeout_ms: u32) -> io::Result<usize> {
        UsbHostDevice::transfer(self, endpoint, data, timeout_ms)
    }
    fn reset(&mut self) -> io::Result<()> {
        UsbHostDevice::reset(self)
    }
}

/// Why a request was refused before it reached the device.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Denied {
    /// The configuration has no such interface.
    NoSuchInterface(u8),
    /// The interface class is one the system drives itself.
    ProtectedInterface(u8),
    /// The interface is not claimed by this session.
    NotClaimed(u8),
    /// The interface is already claimed.
    AlreadyClaimed(u8),
    /// No such endpoint in a claimed interface's active alternate setting.
    NoSuchEndpoint(u8),
    /// An IN call on an OUT endpoint or the other way round.
    WrongDirection(u8),
    /// The endpoint is not bulk or interrupt (or, for the isochronous plan, not isochronous).
    WrongType(u8),
    /// The call asks to move more than [`MAX_BROKER_BYTES`].
    TooLong(usize),
    /// A control request the session never sends, with the reason.
    BadRequest(&'static str),
}

impl fmt::Display for Denied {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Denied::NoSuchInterface(n) => write!(f, "the device has no interface {n}"),
            Denied::ProtectedInterface(n) => write!(f, "interface {n} is driven by the system and cannot be claimed"),
            Denied::NotClaimed(n) => write!(f, "interface {n} is not claimed"),
            Denied::AlreadyClaimed(n) => write!(f, "interface {n} is already claimed"),
            Denied::NoSuchEndpoint(e) => write!(f, "no endpoint {e:#04x} in a claimed interface"),
            Denied::WrongDirection(e) => write!(f, "endpoint {e:#04x} has the other direction"),
            Denied::WrongType(e) => write!(f, "endpoint {e:#04x} is not the transfer type this call moves"),
            Denied::TooLong(n) => write!(f, "{n} bytes is more than one call moves ({MAX_BROKER_BYTES})"),
            Denied::BadRequest(why) => write!(f, "control request refused: {why}"),
        }
    }
}

/// A failed call: refused by the rules, or failed in the transport.
#[derive(Debug)]
pub enum Error {
    Denied(Denied),
    Io(io::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Denied(d) => d.fmt(f),
            Error::Io(e) => write!(f, "{e}"),
        }
    }
}

impl From<Denied> for Error {
    fn from(d: Denied) -> Self {
        Error::Denied(d)
    }
}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        Error::Io(e)
    }
}

/// A device the person granted to one app, with the rules applied to its requests.
pub struct Session<T: Transport> {
    transport: T,
    device: DeviceDescriptor,
    config: ConfigurationDescriptor,
    /// The claimed interfaces and the alternate setting each is on.
    claimed: BTreeMap<u8, u8>,
}

fn clamp_timeout(ms: u32) -> u32 {
    if ms == 0 {
        DEFAULT_TIMEOUT_MS
    } else {
        ms.min(MAX_TIMEOUT_MS)
    }
}

impl<T: Transport> Session<T> {
    pub fn new(transport: T, device: DeviceDescriptor, config: ConfigurationDescriptor) -> Self {
        Session { transport, device, config, claimed: BTreeMap::new() }
    }

    pub fn device(&self) -> &DeviceDescriptor {
        &self.device
    }

    pub fn configuration(&self) -> &ConfigurationDescriptor {
        &self.config
    }

    /// The claimed interface numbers, ascending.
    pub fn claimed(&self) -> Vec<u8> {
        self.claimed.keys().copied().collect()
    }

    /// Every alternate setting of interface `number`.
    fn alternates(&self, number: u8) -> Vec<&InterfaceDescriptor> {
        self.config.interfaces.iter().filter(|i| i.number == number).collect()
    }

    /// The interfaces an app may claim: those with no alternate setting of a system-driven class, ascending.
    pub fn claimable(&self) -> Vec<u8> {
        let mut numbers: Vec<u8> = self.config.interfaces.iter().map(|i| i.number).collect();
        numbers.sort_unstable();
        numbers.dedup();
        numbers.into_iter().filter(|n| !self.alternates(*n).iter().any(|i| protected_class(i.class))).collect()
    }

    /// Claims an interface (alternate setting 0). The kernel driver, if one is bound, lets go of it.
    pub fn claim(&mut self, interface: u8) -> Result<(), Error> {
        let alts = self.alternates(interface);
        if alts.is_empty() {
            return Err(Denied::NoSuchInterface(interface).into());
        }
        if alts.iter().any(|i| protected_class(i.class)) {
            return Err(Denied::ProtectedInterface(interface).into());
        }
        if self.claimed.contains_key(&interface) {
            return Err(Denied::AlreadyClaimed(interface).into());
        }
        self.transport.claim_interface(interface)?;
        self.claimed.insert(interface, 0);
        Ok(())
    }

    /// Gives an interface back.
    pub fn release(&mut self, interface: u8) -> Result<(), Error> {
        if !self.claimed.contains_key(&interface) {
            return Err(Denied::NotClaimed(interface).into());
        }
        self.transport.release_interface(interface)?;
        self.claimed.remove(&interface);
        Ok(())
    }

    /// Selects an alternate setting of a claimed interface.
    pub fn select_alternate(&mut self, interface: u8, alternate: u8) -> Result<(), Error> {
        if !self.claimed.contains_key(&interface) {
            return Err(Denied::NotClaimed(interface).into());
        }
        if self.config.interface(interface, alternate).is_none() {
            return Err(Denied::NoSuchInterface(interface).into());
        }
        self.transport.set_interface(interface, alternate)?;
        self.claimed.insert(interface, alternate);
        Ok(())
    }

    /// The endpoint `address` in the active alternate setting of a claimed interface.
    fn endpoint(&self, address: u8) -> Result<(u8, EndpointDescriptor), Denied> {
        for (&number, &alternate) in &self.claimed {
            if let Some(i) = self.config.interface(number, alternate) {
                if let Some(e) = i.endpoints.iter().find(|e| e.address == address) {
                    return Ok((number, *e));
                }
            }
        }
        Err(Denied::NoSuchEndpoint(address))
    }

    fn data_endpoint(&self, address: u8, want_in: bool) -> Result<EndpointDescriptor, Denied> {
        let (_, e) = self.endpoint(address)?;
        if e.is_in() != want_in {
            return Err(Denied::WrongDirection(address));
        }
        if !matches!(e.transfer_type(), TransferType::Bulk | TransferType::Interrupt) {
            return Err(Denied::WrongType(address));
        }
        Ok(e)
    }

    /// Reads up to `length` bytes from a bulk or interrupt IN endpoint.
    pub fn read(&mut self, address: u8, length: usize, timeout_ms: u32) -> Result<Vec<u8>, Error> {
        if length > MAX_BROKER_BYTES {
            return Err(Denied::TooLong(length).into());
        }
        self.data_endpoint(address, true)?;
        let mut buf = vec![0u8; length];
        let got = self.transport.transfer(address, &mut buf, clamp_timeout(timeout_ms))?;
        buf.truncate(got.min(length));
        Ok(buf)
    }

    /// Writes `data` to a bulk or interrupt OUT endpoint and returns the count the device took.
    pub fn write(&mut self, address: u8, data: &[u8], timeout_ms: u32) -> Result<usize, Error> {
        if data.len() > MAX_BROKER_BYTES {
            return Err(Denied::TooLong(data.len()).into());
        }
        self.data_endpoint(address, false)?;
        let mut buf = data.to_vec();
        Ok(self.transport.transfer(address, &mut buf, clamp_timeout(timeout_ms))?)
    }

    /// Clears a halted endpoint of a claimed interface.
    pub fn clear_halt(&mut self, address: u8) -> Result<(), Error> {
        self.endpoint(address)?;
        Ok(self.transport.clear_halt(address)?)
    }

    /// Checks a control request against the rules.
    pub fn check_control(&self, setup: &SetupPacket) -> Result<(), Denied> {
        if setup.kind == RequestKind::Standard {
            return Err(Denied::BadRequest("standard requests have their own calls"));
        }
        if usize::from(setup.length) > MAX_BROKER_BYTES {
            return Err(Denied::TooLong(usize::from(setup.length)));
        }
        match setup.recipient {
            Recipient::Device => Ok(()),
            Recipient::Interface => {
                let n = (setup.index & 0xff) as u8;
                if self.claimed.contains_key(&n) {
                    Ok(())
                } else {
                    Err(Denied::NotClaimed(n))
                }
            }
            Recipient::Endpoint => self.endpoint((setup.index & 0xff) as u8).map(|_| ()),
        }
    }

    /// Sends a class or vendor control request. For a device-to-host request `data` is ignored and the reply is
    /// returned; for a host-to-device request `data` (exactly `setup.length` bytes) is sent and an empty vector is
    /// returned.
    pub fn control(&mut self, setup: &SetupPacket, data: &[u8], timeout_ms: u32) -> Result<Vec<u8>, Error> {
        self.check_control(setup)?;
        let timeout = clamp_timeout(timeout_ms);
        match setup.direction {
            Direction::DeviceToHost => {
                let mut buf = vec![0u8; usize::from(setup.length)];
                let got = self.transport.control(setup, &mut buf, timeout)?;
                buf.truncate(got.min(usize::from(setup.length)));
                Ok(buf)
            }
            Direction::HostToDevice => {
                if data.len() != usize::from(setup.length) {
                    return Err(Denied::BadRequest("the data length differs from the request's length").into());
                }
                let mut buf = data.to_vec();
                self.transport.control(setup, &mut buf, timeout)?;
                Ok(Vec::new())
            }
        }
    }

    /// Resets the device. The claims stay recorded; the caller re-reads the descriptors afterwards.
    pub fn reset(&mut self) -> Result<(), Error> {
        Ok(self.transport.reset()?)
    }

    /// The frame list an isochronous transfer of `bytes` on `address` takes at `speed` over `frames` frames
    /// (`usb_transfer::plan_isochronous`). The endpoint must be isochronous and in a claimed interface. Isochronous
    /// data does not go through the service (its rate needs a descriptor, not messages); this is the schedule a
    /// descriptor-based transport follows.
    pub fn isochronous_plan(&self, address: u8, speed: Speed, bytes: usize, frames: usize) -> Result<Vec<IsochFrame>, Error> {
        let (_, e) = self.endpoint(address)?;
        if e.transfer_type() != TransferType::Isochronous {
            return Err(Denied::WrongType(address).into());
        }
        let pipe = Pipe::from_endpoint(&e);
        plan_isochronous(&pipe, speed, e.is_in(), bytes, frames)
            .map_err(|e: PipeError| Error::Io(io::Error::new(io::ErrorKind::InvalidInput, format!("{e:?}"))))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::usb_descriptors::{ConfigurationDescriptor, EndpointDescriptor, InterfaceDescriptor};

    #[derive(Default)]
    struct Mock {
        log: Vec<String>,
        reply: Vec<u8>,
        fail: bool,
    }

    impl Transport for Mock {
        fn claim_interface(&mut self, n: u8) -> io::Result<()> {
            self.log.push(format!("claim {n}"));
            Ok(())
        }
        fn release_interface(&mut self, n: u8) -> io::Result<()> {
            self.log.push(format!("release {n}"));
            Ok(())
        }
        fn set_interface(&mut self, n: u8, a: u8) -> io::Result<()> {
            self.log.push(format!("alt {n} {a}"));
            Ok(())
        }
        fn clear_halt(&mut self, e: u8) -> io::Result<()> {
            self.log.push(format!("halt {e:#x}"));
            Ok(())
        }
        fn control(&mut self, s: &SetupPacket, data: &mut [u8], _t: u32) -> io::Result<usize> {
            self.log.push(format!("control {:#x} {}", s.request_type(), s.request));
            if self.fail {
                return Err(io::Error::from_raw_os_error(32));
            }
            let n = self.reply.len().min(data.len());
            if s.direction == Direction::DeviceToHost {
                data[..n].copy_from_slice(&self.reply[..n]);
            }
            Ok(if s.direction == Direction::DeviceToHost { n } else { data.len() })
        }
        fn transfer(&mut self, e: u8, data: &mut [u8], t: u32) -> io::Result<usize> {
            self.log.push(format!("transfer {e:#x} {} {t}", data.len()));
            let n = self.reply.len().min(data.len());
            if e & 0x80 != 0 {
                data[..n].copy_from_slice(&self.reply[..n]);
                Ok(n)
            } else {
                Ok(data.len())
            }
        }
        fn reset(&mut self) -> io::Result<()> {
            self.log.push("reset".into());
            Ok(())
        }
    }

    fn ep(address: u8, attributes: u8) -> EndpointDescriptor {
        EndpointDescriptor { address, attributes, max_packet_size: 64, interval: 0 }
    }

    fn iface(number: u8, alternate: u8, class: u8, endpoints: Vec<EndpointDescriptor>) -> InterfaceDescriptor {
        InterfaceDescriptor { number, alternate, class, subclass: 0, protocol: 0, endpoints }
    }

    /// Interface 0 is vendor-specific with bulk IN 0x81, bulk OUT 0x02 and an interrupt IN 0x83 (alternate 1 adds an
    /// isochronous IN 0x84); interface 1 is HID.
    fn session() -> Session<Mock> {
        let device = DeviceDescriptor {
            usb_version: 0x0200,
            class: 0,
            subclass: 0,
            protocol: 0,
            max_packet_size0: 64,
            vendor_id: 0x1234,
            product_id: 1,
            device_version: 0,
            configurations: 1,
        };
        let config = ConfigurationDescriptor {
            value: 1,
            attributes: 0x80,
            max_power_ma: 100,
            interfaces: vec![
                iface(0, 0, 0xff, vec![ep(0x81, 2), ep(0x02, 2), ep(0x83, 3)]),
                iface(0, 1, 0xff, vec![ep(0x81, 2), ep(0x02, 2), ep(0x84, 1)]),
                iface(1, 0, 0x03, vec![ep(0x85, 3)]),
            ],
        };
        Session::new(Mock::default(), device, config)
    }

    fn vendor_in(recipient: Recipient, index: u16, length: u16) -> SetupPacket {
        SetupPacket { direction: Direction::DeviceToHost, kind: RequestKind::Vendor, recipient, request: 1, value: 0, index, length }
    }

    #[test]
    fn only_interfaces_the_system_does_not_drive_are_claimable() {
        let mut s = session();
        assert_eq!(s.claimable(), vec![0]);
        assert!(matches!(s.claim(1), Err(Error::Denied(Denied::ProtectedInterface(1)))));
        assert!(matches!(s.claim(7), Err(Error::Denied(Denied::NoSuchInterface(7)))));
        s.claim(0).unwrap();
        assert!(matches!(s.claim(0), Err(Error::Denied(Denied::AlreadyClaimed(0)))));
        assert_eq!(s.claimed(), vec![0]);
        assert_eq!(s.transport.log, vec!["claim 0"], "refused claims never reach the device");
    }

    #[test]
    fn bulk_and_interrupt_move_data_in_their_own_direction_only() {
        let mut s = session();
        s.transport.reply = vec![1, 2, 3];
        assert!(matches!(s.read(0x81, 8, 0), Err(Error::Denied(Denied::NoSuchEndpoint(0x81)))), "not claimed yet");
        s.claim(0).unwrap();
        assert_eq!(s.read(0x81, 8, 0).unwrap(), vec![1, 2, 3]);
        assert_eq!(s.read(0x83, 8, 50).unwrap(), vec![1, 2, 3]);
        assert_eq!(s.write(0x02, &[9, 9], 0).unwrap(), 2);
        assert!(matches!(s.read(0x02, 8, 0), Err(Error::Denied(Denied::WrongDirection(0x02)))));
        assert!(matches!(s.write(0x81, &[1], 0), Err(Error::Denied(Denied::WrongDirection(0x81)))));
        assert!(matches!(s.read(0x85, 8, 0), Err(Error::Denied(Denied::NoSuchEndpoint(0x85)))), "an endpoint of the HID interface");
        assert!(s.transport.log.contains(&format!("transfer 0x83 8 50")));
    }

    #[test]
    fn the_timeout_is_defaulted_and_capped() {
        let mut s = session();
        s.claim(0).unwrap();
        s.read(0x81, 1, 0).unwrap();
        s.read(0x81, 1, 999_999).unwrap();
        assert!(s.transport.log.contains(&format!("transfer 0x81 1 {DEFAULT_TIMEOUT_MS}")));
        assert!(s.transport.log.contains(&format!("transfer 0x81 1 {MAX_TIMEOUT_MS}")));
    }

    #[test]
    fn one_call_moves_at_most_the_broker_limit() {
        let mut s = session();
        s.claim(0).unwrap();
        assert!(matches!(s.read(0x81, MAX_BROKER_BYTES + 1, 0), Err(Error::Denied(Denied::TooLong(_)))));
        assert!(matches!(s.write(0x02, &vec![0; MAX_BROKER_BYTES + 1], 0), Err(Error::Denied(Denied::TooLong(_)))));
        assert!(s.write(0x02, &vec![0; MAX_BROKER_BYTES], 0).is_ok());
    }

    #[test]
    fn an_alternate_setting_changes_which_endpoints_exist() {
        let mut s = session();
        s.claim(0).unwrap();
        assert!(matches!(s.read(0x84, 4, 0), Err(Error::Denied(Denied::NoSuchEndpoint(0x84)))));
        s.select_alternate(0, 1).unwrap();
        assert!(matches!(s.read(0x84, 4, 0), Err(Error::Denied(Denied::WrongType(0x84)))), "isochronous is not a bulk call");
        assert!(matches!(s.read(0x83, 4, 0), Err(Error::Denied(Denied::NoSuchEndpoint(0x83)))), "alternate 1 has no interrupt endpoint");
        assert!(matches!(s.select_alternate(0, 9), Err(Error::Denied(Denied::NoSuchInterface(0)))));
        assert!(matches!(s.select_alternate(1, 0), Err(Error::Denied(Denied::NotClaimed(1)))));
    }

    #[test]
    fn standard_control_requests_are_refused() {
        let mut s = session();
        let get = crate::usb_descriptors::get_descriptor(1, 0, 0, 18);
        assert!(matches!(s.control(&get, &[], 0), Err(Error::Denied(Denied::BadRequest(_)))));
        assert!(matches!(s.control(&crate::usb_descriptors::set_configuration(2), &[], 0), Err(Error::Denied(Denied::BadRequest(_)))));
        assert!(matches!(s.control(&crate::usb_descriptors::set_interface(0, 1), &[], 0), Err(Error::Denied(Denied::BadRequest(_)))));
        assert!(s.transport.log.is_empty());
    }

    #[test]
    fn vendor_requests_to_the_device_pass_and_to_an_interface_need_the_claim() {
        let mut s = session();
        s.transport.reply = vec![0xAA, 0xBB, 0xCC];
        assert_eq!(s.control(&vendor_in(Recipient::Device, 0, 8), &[], 0).unwrap(), vec![0xAA, 0xBB, 0xCC]);
        assert!(matches!(s.control(&vendor_in(Recipient::Interface, 0, 4), &[], 0), Err(Error::Denied(Denied::NotClaimed(0)))));
        s.claim(0).unwrap();
        assert!(s.control(&vendor_in(Recipient::Interface, 0, 4), &[], 0).is_ok());
        assert!(matches!(s.control(&vendor_in(Recipient::Interface, 1, 4), &[], 0), Err(Error::Denied(Denied::NotClaimed(1)))), "the HID interface stays out");
        assert!(s.control(&vendor_in(Recipient::Endpoint, 0x81, 4), &[], 0).is_ok());
        assert!(matches!(s.control(&vendor_in(Recipient::Endpoint, 0x85, 4), &[], 0), Err(Error::Denied(Denied::NoSuchEndpoint(0x85)))));
    }

    #[test]
    fn an_out_control_request_must_carry_exactly_its_length() {
        let mut s = session();
        let out = SetupPacket { direction: Direction::HostToDevice, kind: RequestKind::Class, recipient: Recipient::Device, request: 9, value: 0, index: 0, length: 3 };
        assert_eq!(s.control(&out, &[1, 2, 3], 0).unwrap(), Vec::<u8>::new());
        assert!(matches!(s.control(&out, &[1, 2], 0), Err(Error::Denied(Denied::BadRequest(_)))));
        let big = SetupPacket { length: (MAX_BROKER_BYTES + 1) as u16, ..out };
        assert!(matches!(s.control(&big, &vec![0; MAX_BROKER_BYTES + 1], 0), Err(Error::Denied(Denied::TooLong(_)))));
    }

    #[test]
    fn a_transport_failure_comes_back_as_an_io_error() {
        let mut s = session();
        s.transport.fail = true;
        match s.control(&vendor_in(Recipient::Device, 0, 4), &[], 0) {
            Err(Error::Io(e)) => assert_eq!(e.raw_os_error(), Some(32)),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn release_and_clear_halt_need_a_claimed_interface() {
        let mut s = session();
        assert!(matches!(s.release(0), Err(Error::Denied(Denied::NotClaimed(0)))));
        assert!(matches!(s.clear_halt(0x81), Err(Error::Denied(Denied::NoSuchEndpoint(0x81)))));
        s.claim(0).unwrap();
        s.clear_halt(0x81).unwrap();
        s.release(0).unwrap();
        assert!(s.claimed().is_empty());
        s.reset().unwrap();
        assert_eq!(s.transport.log, vec!["claim 0", "halt 0x81", "release 0", "reset"]);
    }

    #[test]
    fn the_isochronous_plan_needs_an_isochronous_endpoint_of_a_claimed_interface() {
        let mut s = session();
        s.claim(0).unwrap();
        assert!(s.isochronous_plan(0x81, Speed::High, 1024, 4).is_err(), "a bulk endpoint");
        s.select_alternate(0, 1).unwrap();
        assert!(matches!(s.isochronous_plan(0x99, Speed::High, 1024, 4), Err(Error::Denied(Denied::NoSuchEndpoint(0x99)))));
        assert!(!matches!(s.isochronous_plan(0x84, Speed::High, 1024, 4), Err(Error::Denied(_))));
    }
}
