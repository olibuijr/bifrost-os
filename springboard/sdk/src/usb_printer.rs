//! USB printers for apps: the printer-class rules (interface protocol, bulk endpoints, `GET_DEVICE_ID`, `GET_PORT_STATUS`,
//! `SOFT_RESET`, port status and the IEEE 1284 device ID) from `bifrost_core::usb_printer`, and a way to send a job to
//! one over the shell's `usb` service ([`crate::usb_access`]).
//!
//! The person grants the printer to the app first (the shell's device sheet; a printer interface is not one of the
//! classes the system drives, so it can be offered). Then [`find_printer`] picks the interface and bulk endpoints from
//! the descriptors of the opened device, [`crate::usb_access::claim`] claims it, [`device_id_request`] and
//! [`port_status_request`] ask the printer about itself, and a [`PrintJob`] sends the bytes in chunks. Print jobs of the
//! system's own print dialogs still go through `print` and CUPS; this path is for apps that speak a printer's language
//! themselves (label printers, receipt printers).
pub use bifrost_core::usb_printer::*;

use crate::runtime::Ctx;
use crate::usb_access::{self, ControlRequest, Done, Interface};
use bifrost_core::usb_descriptors::{Direction, SetupPacket};

/// Where to send print data on an opened device.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PrinterPath {
    pub interface: u8,
    pub alternate: u8,
    pub protocol: Protocol,
    pub out_endpoint: u8,
    /// The bulk IN endpoint of a bidirectional printer, for status and replies.
    pub in_endpoint: Option<u8>,
}

/// The first printer interface of an opened device that can carry print data and that the shell will let the app
/// claim, with the lowest alternate setting.
pub fn find_printer(interfaces: &[Interface]) -> Option<PrinterPath> {
    let mut best: Option<PrinterPath> = None;
    for i in interfaces.iter().filter(|i| i.claimable && i.class == CLASS_PRINTER && i.subclass == SUBCLASS_PRINTER) {
        let protocol = match i.protocol {
            1 => Protocol::Unidirectional,
            2 => Protocol::Bidirectional,
            3 => Protocol::Ieee1284Dot4,
            _ => continue,
        };
        let bulk = |want_in: bool| i.endpoints.iter().find(|e| e.transfer == "bulk" && e.is_in == want_in).map(|e| e.address);
        let Some(out_endpoint) = bulk(false) else { continue };
        let path = PrinterPath { interface: i.number, alternate: i.alternate, protocol, out_endpoint, in_endpoint: bulk(true) };
        if best.is_none_or(|b| (path.interface, path.alternate) < (b.interface, b.alternate)) {
            best = Some(path);
        }
    }
    best
}

fn control(ctx: &Ctx, handle: u32, packet: SetupPacket) -> u32 {
    let request = ControlRequest {
        direction: packet.direction,
        kind: packet.kind,
        recipient: packet.recipient,
        request: packet.request,
        value: packet.value,
        index: packet.index,
        length: packet.length,
    };
    let data = vec![0u8; if packet.direction == Direction::HostToDevice { usize::from(packet.length) } else { 0 }];
    usb_access::control(ctx, handle, &request, &data, 1000)
}

/// Asks the printer for its IEEE 1284 device ID (up to `length` bytes). The answer arrives as `Done.data`; parse it with
/// [`parse_device_id`].
pub fn device_id_request(ctx: &Ctx, handle: u32, path: &PrinterPath, length: u16) -> u32 {
    control(ctx, handle, get_device_id(path.interface, length))
}

/// Asks the printer for its port status byte; read the answer with [`PortStatus`].
pub fn port_status_request(ctx: &Ctx, handle: u32, path: &PrinterPath) -> u32 {
    control(ctx, handle, get_port_status(path.interface))
}

/// Clears the printer's buffers and the bulk pipes.
pub fn soft_reset_request(ctx: &Ctx, handle: u32, path: &PrinterPath) -> u32 {
    control(ctx, handle, soft_reset(path.interface))
}

/// One print job: bytes sent to the bulk OUT endpoint in chunks, one write in flight at a time.
///
/// ```ignore
/// let mut job = PrintJob::new(path.out_endpoint, bytes);
/// while let Some((endpoint, chunk)) = job.next_chunk() { usb_access::write(ctx, handle, endpoint, &chunk, 5000); }
/// // on every usb_access::Event::Done of the handle: job.complete(&done), then send job.next_chunk() again
/// ```
#[derive(Debug)]
pub struct PrintJob {
    endpoint: u8,
    data: Vec<u8>,
    chunk: usize,
    /// Bytes handed out by `next_chunk`.
    offered: usize,
    /// Bytes the printer took.
    sent: usize,
    writing: bool,
    failed: Option<String>,
}

impl PrintJob {
    /// The most bytes one write carries.
    pub const CHUNK: usize = usb_access::MAX_BYTES;

    pub fn new(out_endpoint: u8, data: Vec<u8>) -> Self {
        PrintJob { endpoint: out_endpoint, data, chunk: Self::CHUNK, offered: 0, sent: 0, writing: false, failed: None }
    }

    /// Changes the chunk size, from 1 to [`PrintJob::CHUNK`].
    pub fn with_chunk(mut self, chunk: usize) -> Self {
        self.chunk = chunk.clamp(1, Self::CHUNK);
        self
    }

    /// The next write to send as `(endpoint, bytes)`, or `None` while a write is in flight, when everything was handed
    /// out, or after a failure.
    pub fn next_chunk(&mut self) -> Option<(u8, Vec<u8>)> {
        if self.writing || self.failed.is_some() || self.offered >= self.data.len() {
            return None;
        }
        let end = (self.offered + self.chunk).min(self.data.len());
        let bytes = self.data[self.offered..end].to_vec();
        self.offered = end;
        self.writing = true;
        Some((self.endpoint, bytes))
    }

    /// A finished request of the handle. A write that moved fewer bytes than offered hands the rest out again; a failed
    /// write stops the job. Other requests are ignored. Returns whether it was this job's write.
    pub fn complete(&mut self, done: &Done) -> bool {
        if done.op != "write" || !self.writing {
            return false;
        }
        self.writing = false;
        match (&done.error, done.count) {
            (Some(e), _) => self.failed = Some(e.clone()),
            (None, Some(n)) => {
                // `n` of the bytes offered went out; anything short is offered again
                let in_flight = self.offered - self.sent;
                self.sent += n.min(in_flight);
                self.offered = self.sent;
            }
            (None, None) => self.failed = Some("the write did not report a byte count".into()),
        }
        true
    }

    /// `(bytes the printer took, total bytes)`.
    pub fn progress(&self) -> (usize, usize) {
        (self.sent, self.data.len())
    }

    /// Whether the printer took every byte.
    pub fn is_done(&self) -> bool {
        self.failed.is_none() && self.sent == self.data.len() && !self.writing
    }

    /// Why the job stopped, if it did.
    pub fn error(&self) -> Option<&str> {
        self.failed.as_deref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::usb_access::Endpoint;

    fn ep(address: u8, transfer: &str) -> Endpoint {
        Endpoint { address, transfer: transfer.into(), is_in: address & 0x80 != 0, max_packet_size: 512, interval: 0 }
    }

    fn iface(number: u8, alternate: u8, class: u8, protocol: u8, claimable: bool, endpoints: Vec<Endpoint>) -> Interface {
        Interface { number, alternate, class, subclass: 1, protocol, claimable, endpoints }
    }

    fn done(count: Option<usize>, error: Option<&str>) -> Done {
        Done { handle: 1, token: 1, op: "write".into(), data: None, count, error: error.map(String::from), failure: error.map(|_| "other".to_string()) }
    }

    #[test]
    fn the_printer_interface_is_found_with_its_bulk_endpoints() {
        let list = vec![
            iface(0, 0, 0xff, 0, true, vec![ep(0x81, "bulk")]),
            iface(1, 0, 7, 2, true, vec![ep(0x02, "bulk"), ep(0x81, "bulk"), ep(0x83, "interrupt")]),
        ];
        let p = find_printer(&list).unwrap();
        assert_eq!((p.interface, p.alternate, p.protocol, p.out_endpoint, p.in_endpoint), (1, 0, Protocol::Bidirectional, 0x02, Some(0x81)));
    }

    #[test]
    fn a_printer_without_a_bulk_out_or_that_cannot_be_claimed_is_not_offered() {
        assert_eq!(find_printer(&[iface(0, 0, 7, 1, true, vec![ep(0x81, "bulk")])]), None, "status only");
        assert_eq!(find_printer(&[iface(0, 0, 7, 1, false, vec![ep(0x02, "bulk")])]), None, "not claimable");
        assert_eq!(find_printer(&[iface(0, 0, 7, 9, true, vec![ep(0x02, "bulk")])]), None, "unknown protocol");
        assert_eq!(find_printer(&[iface(0, 0, 7, 1, true, vec![ep(0x02, "interrupt")])]), None, "no bulk out");
        assert_eq!(find_printer(&[]), None);
    }

    #[test]
    fn the_lowest_alternate_setting_wins() {
        let list = vec![iface(0, 1, 7, 2, true, vec![ep(0x04, "bulk")]), iface(0, 0, 7, 1, true, vec![ep(0x02, "bulk")])];
        let p = find_printer(&list).unwrap();
        assert_eq!((p.alternate, p.out_endpoint, p.protocol), (0, 0x02, Protocol::Unidirectional));
    }

    #[test]
    fn a_job_goes_out_in_chunks_one_write_at_a_time() {
        let mut job = PrintJob::new(0x02, (0..10u8).collect()).with_chunk(4);
        assert_eq!(job.next_chunk(), Some((0x02, vec![0, 1, 2, 3])));
        assert_eq!(job.next_chunk(), None, "one write in flight");
        assert!(job.complete(&done(Some(4), None)));
        assert_eq!(job.next_chunk(), Some((0x02, vec![4, 5, 6, 7])));
        assert!(job.complete(&done(Some(4), None)));
        assert_eq!(job.next_chunk(), Some((0x02, vec![8, 9])));
        assert!(!job.is_done());
        assert!(job.complete(&done(Some(2), None)));
        assert!(job.is_done());
        assert_eq!(job.progress(), (10, 10));
        assert_eq!(job.next_chunk(), None);
    }

    #[test]
    fn a_short_write_offers_the_rest_again() {
        let mut job = PrintJob::new(0x02, vec![1, 2, 3, 4, 5, 6]).with_chunk(4);
        job.next_chunk().unwrap();
        job.complete(&done(Some(1), None));
        assert_eq!(job.progress(), (1, 6));
        assert_eq!(job.next_chunk(), Some((0x02, vec![2, 3, 4, 5])));
        job.complete(&done(Some(4), None));
        assert_eq!(job.next_chunk(), Some((0x02, vec![6])));
        job.complete(&done(Some(1), None));
        assert!(job.is_done());
    }

    #[test]
    fn a_failed_write_stops_the_job_and_a_count_past_the_offer_is_clamped() {
        let mut job = PrintJob::new(0x02, vec![0; 8]).with_chunk(4);
        job.next_chunk().unwrap();
        job.complete(&done(None, Some("the printer stalled")));
        assert_eq!(job.error(), Some("the printer stalled"));
        assert_eq!(job.next_chunk(), None);
        assert!(!job.is_done());
        let mut greedy = PrintJob::new(0x02, vec![0; 8]).with_chunk(4);
        greedy.next_chunk().unwrap();
        greedy.complete(&done(Some(100), None));
        assert_eq!(greedy.progress(), (4, 8), "a count past the bytes offered cannot skip data");
        let mut odd = PrintJob::new(0x02, vec![0; 2]);
        odd.next_chunk().unwrap();
        odd.complete(&done(None, None));
        assert!(odd.error().is_some());
    }

    #[test]
    fn other_results_and_an_empty_job_are_ignored() {
        let mut job = PrintJob::new(0x02, vec![1]);
        let mut read = done(Some(1), None);
        read.op = "read".into();
        assert!(!job.complete(&done(Some(1), None)), "no write is in flight yet");
        job.next_chunk().unwrap();
        assert!(!job.complete(&read));
        let mut empty = PrintJob::new(0x02, Vec::new());
        assert_eq!(empty.next_chunk(), None);
        assert!(empty.is_done());
        assert_eq!(PrintJob::new(0x02, vec![0]).with_chunk(0).chunk, 1);
        assert_eq!(PrintJob::new(0x02, vec![0]).with_chunk(usize::MAX).chunk, PrintJob::CHUNK);
    }
}
