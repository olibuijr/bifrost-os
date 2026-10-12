//! USB pipe model: the transfer side of a host endpoint (std only).
//!
//! A pipe is one endpoint seen from the host. This module takes an endpoint descriptor from
//! `usb_descriptors` and works out what a host driver needs before it touches the bus: how a
//! request splits into packets, how many bytes an isochronous service interval may carry, which
//! frames a periodic schedule occupies, how queued I/O requests complete, when a halted pipe
//! must be cleared, and which bulk stream identifiers are open.
//!
//! Pure: no device is opened and no request is sent. The caller owns the transport and reports
//! back what the bus transferred. The names follow the official host framework's pipe concepts
//! (I/O requests, isochronous frames, stall clearing, idle timeout, streams), and the packet rules
//! follow the USB 2.0 and 3.2 specifications (`wMaxPacketSize` bits 0..=10 for the payload size,
//! bits 11..=12 for the extra high-bandwidth transactions of a high-speed periodic endpoint).

use crate::usb_descriptors::{EndpointDescriptor, TransferType};

/// Largest valid bulk stream identifier. Stream 0 means "no stream"; 65534 and 65535 are reserved.
pub const MAX_STREAM_ID: u16 = 65533;

/// Mask of the packet payload size in `wMaxPacketSize`.
const PACKET_SIZE_MASK: u16 = 0x07ff;

/// Whether a stream identifier is one a bulk pipe may open.
pub fn valid_stream_id(id: u16) -> bool {
    (1..=MAX_STREAM_ID).contains(&id)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PipeError {
    /// Zero packet size, or high-bandwidth bits the endpoint type does not allow.
    BadEndpoint,
    /// The call does not match the endpoint type, for example a bulk request on an isochronous pipe.
    WrongTransferType,
    /// The pipe is halted and must be cleared before it accepts requests.
    Halted,
    /// A request is larger than one service interval can carry.
    TooLarge { requested: usize, limit: usize },
    /// A completion reports more bytes than were requested.
    Overrun { requested: usize, transferred: usize },
    /// No pending request has this identifier.
    UnknownRequest(u32),
    /// Streams are already enabled on this pipe.
    StreamsEnabled,
    /// Streams are not enabled on this pipe.
    StreamsDisabled,
    /// The stream identifier is outside 1..=65533.
    BadStreamId(u16),
}

/// The result of a completed bulk or interrupt request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Completion {
    pub id: u32,
    pub requested: usize,
    pub transferred: usize,
}

impl Completion {
    /// True when the device sent less than was asked for.
    pub fn is_short(&self) -> bool {
        self.transferred < self.requested
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrameStatus {
    /// The bus has not reported this frame yet.
    Pending,
    /// The frame transferred every requested byte.
    Complete,
    /// The frame transferred fewer bytes than requested (a short packet, which is normal on isochronous pipes).
    Partial,
}

/// One frame of an isochronous request: the requested count and, once reported, the transferred count.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IsoFrame {
    pub request_count: u32,
    pub complete_count: Option<u32>,
}

impl IsoFrame {
    pub fn new(request_count: u32) -> IsoFrame {
        IsoFrame { request_count, complete_count: None }
    }

    /// Record what the bus transferred for this frame. More than requested is an error.
    pub fn record(&mut self, transferred: u32) -> Result<(), PipeError> {
        if transferred > self.request_count {
            return Err(PipeError::Overrun {
                requested: self.request_count as usize,
                transferred: transferred as usize,
            });
        }
        self.complete_count = Some(transferred);
        Ok(())
    }

    pub fn status(&self) -> FrameStatus {
        match self.complete_count {
            None => FrameStatus::Pending,
            Some(done) if done == self.request_count => FrameStatus::Complete,
            Some(_) => FrameStatus::Partial,
        }
    }
}

/// A periodic request: consecutive frames starting at `first_frame`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IsoSchedule {
    pub first_frame: u64,
    pub frames: Vec<IsoFrame>,
}

impl IsoSchedule {
    /// The frame number after the last frame of the schedule.
    pub fn end_frame(&self) -> u64 {
        self.first_frame + self.frames.len() as u64
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pipe {
    endpoint: EndpointDescriptor,
    halted: bool,
    /// `None` while streams are disabled; otherwise the stream identifiers that were opened.
    streams: Option<Vec<u16>>,
    idle_timeout_ms: u32,
    next_id: u32,
    /// Queued bulk or interrupt requests: (identifier, requested length).
    pending: Vec<(u32, usize)>,
}

impl Pipe {
    /// Open a pipe on an endpoint. The packet size must be non-zero and the high-bandwidth bits must
    /// be valid for the endpoint type.
    pub fn new(endpoint: EndpointDescriptor) -> Result<Pipe, PipeError> {
        let pipe = Pipe {
            endpoint,
            halted: false,
            streams: None,
            idle_timeout_ms: 0,
            next_id: 1,
            pending: Vec::new(),
        };
        pipe.bytes_per_service()?;
        if pipe.packet_size() == 0 {
            return Err(PipeError::BadEndpoint);
        }
        Ok(pipe)
    }

    pub fn endpoint(&self) -> &EndpointDescriptor {
        &self.endpoint
    }

    /// Payload bytes in one packet (`wMaxPacketSize` bits 0..=10).
    pub fn packet_size(&self) -> usize {
        (self.endpoint.max_packet_size & PACKET_SIZE_MASK) as usize
    }

    /// Transactions per service interval: 1 to 3, from bits 11..=12 of `wMaxPacketSize`.
    /// Only interrupt and isochronous endpoints may use the extra transactions; the value 3 is reserved.
    pub fn transactions_per_service(&self) -> Result<usize, PipeError> {
        let extra = (self.endpoint.max_packet_size >> 11) & 0x3;
        let periodic = matches!(self.endpoint.transfer_type(), TransferType::Interrupt | TransferType::Isochronous);
        if extra > 2 || (extra != 0 && !periodic) {
            return Err(PipeError::BadEndpoint);
        }
        Ok(extra as usize + 1)
    }

    /// The most bytes one service interval may carry: packet size times transactions.
    pub fn bytes_per_service(&self) -> Result<usize, PipeError> {
        let transactions = self.transactions_per_service()?;
        Ok(self.packet_size() * transactions)
    }

    /// Split a control, bulk or interrupt data stage into packet lengths: full packets, then one short
    /// packet for the remainder. A zero-length request has no packets.
    pub fn split_request(&self, length: usize) -> Result<Vec<usize>, PipeError> {
        if self.endpoint.transfer_type() == TransferType::Isochronous {
            return Err(PipeError::WrongTransferType);
        }
        let size = self.packet_size();
        let mut packets = Vec::with_capacity(length / size + 1);
        let mut left = length;
        while left > 0 {
            let n = if left < size { left } else { size };
            packets.push(n);
            left -= n;
        }
        Ok(packets)
    }

    /// Build the frame list of an isochronous request. Each entry is one service interval and must fit
    /// `bytes_per_service`.
    pub fn isochronous_schedule(&self, first_frame: u64, lengths: &[usize]) -> Result<IsoSchedule, PipeError> {
        if self.endpoint.transfer_type() != TransferType::Isochronous {
            return Err(PipeError::WrongTransferType);
        }
        let limit = self.bytes_per_service()?;
        let mut frames = Vec::with_capacity(lengths.len());
        for &len in lengths {
            if len > limit {
                return Err(PipeError::TooLarge { requested: len, limit });
            }
            frames.push(IsoFrame::new(len as u32));
        }
        Ok(IsoSchedule { first_frame, frames })
    }

    /// Queue a bulk or interrupt request and return its identifier. A halted pipe refuses it.
    pub fn enqueue(&mut self, length: usize) -> Result<u32, PipeError> {
        match self.endpoint.transfer_type() {
            TransferType::Bulk | TransferType::Interrupt => {}
            _ => return Err(PipeError::WrongTransferType),
        }
        if self.halted {
            return Err(PipeError::Halted);
        }
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1);
        self.pending.push((id, length));
        Ok(id)
    }

    /// Report the bytes the bus moved for a queued request. The request leaves the queue.
    pub fn complete(&mut self, id: u32, transferred: usize) -> Result<Completion, PipeError> {
        let pos = self
            .pending
            .iter()
            .position(|&(pid, _)| pid == id)
            .ok_or(PipeError::UnknownRequest(id))?;
        let (_, requested) = self.pending[pos];
        if transferred > requested {
            return Err(PipeError::Overrun { requested, transferred });
        }
        self.pending.remove(pos);
        Ok(Completion { id, requested, transferred })
    }

    /// Halt the pipe, as after a stall. Queued requests are aborted; their identifiers are returned.
    pub fn halt(&mut self) -> Vec<u32> {
        self.halted = true;
        self.pending.drain(..).map(|(id, _)| id).collect()
    }

    pub fn is_halted(&self) -> bool {
        self.halted
    }

    /// Clear the halt condition so the pipe accepts requests again.
    pub fn clear_stall(&mut self) {
        self.halted = false;
    }

    pub fn pending_count(&self) -> usize {
        self.pending.len()
    }

    pub fn set_idle_timeout(&mut self, ms: u32) {
        self.idle_timeout_ms = ms;
    }

    pub fn idle_timeout(&self) -> u32 {
        self.idle_timeout_ms
    }

    /// Enable bulk streams on this pipe. Only bulk pipes support them.
    pub fn enable_streams(&mut self) -> Result<(), PipeError> {
        if self.endpoint.transfer_type() != TransferType::Bulk {
            return Err(PipeError::WrongTransferType);
        }
        if self.streams.is_some() {
            return Err(PipeError::StreamsEnabled);
        }
        self.streams = Some(Vec::new());
        Ok(())
    }

    /// Open the stream with this identifier and return its handle, which is the identifier itself.
    /// Opening a stream twice is allowed and gives the same handle.
    pub fn copy_stream(&mut self, id: u16) -> Result<u16, PipeError> {
        if !valid_stream_id(id) {
            return Err(PipeError::BadStreamId(id));
        }
        let open = self.streams.as_mut().ok_or(PipeError::StreamsDisabled)?;
        if !open.contains(&id) {
            open.push(id);
        }
        Ok(id)
    }

    /// Identifiers of the streams open on this pipe, in the order they were opened.
    pub fn open_streams(&self) -> &[u16] {
        match &self.streams {
            Some(open) => open.as_slice(),
            None => &[],
        }
    }

    /// Disable streams and close every stream on the pipe.
    pub fn disable_streams(&mut self) -> Result<(), PipeError> {
        if self.streams.take().is_none() {
            return Err(PipeError::StreamsDisabled);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ep(address: u8, attributes: u8, max_packet_size: u16) -> EndpointDescriptor {
        EndpointDescriptor { address, attributes, max_packet_size, interval: 1 }
    }

    const BULK_IN: u8 = 0x81;
    const BULK: u8 = 2;
    const ISO: u8 = 1;
    const INTERRUPT: u8 = 3;

    #[test]
    fn bulk_request_splits_into_full_packets_and_a_short_tail() {
        let pipe = Pipe::new(ep(BULK_IN, BULK, 512)).unwrap();
        assert_eq!(pipe.split_request(1300).unwrap(), vec![512, 512, 276]);
        assert_eq!(pipe.split_request(1024).unwrap(), vec![512, 512]);
        assert_eq!(pipe.split_request(0).unwrap(), Vec::<usize>::new());
    }

    #[test]
    fn high_bandwidth_isochronous_allows_three_transactions_per_interval() {
        // 1024-byte packets, with bits 11..=12 = 2 (three transactions): 1024 | (2 << 11) = 5120.
        let pipe = Pipe::new(ep(0x82, ISO, 1024 | (2 << 11))).unwrap();
        assert_eq!(pipe.packet_size(), 1024);
        assert_eq!(pipe.transactions_per_service().unwrap(), 3);
        assert_eq!(pipe.bytes_per_service().unwrap(), 3072);
        let schedule = pipe.isochronous_schedule(100, &[3072, 1024, 0]).unwrap();
        assert_eq!(schedule.frames.len(), 3);
        assert_eq!(schedule.end_frame(), 103);
        assert_eq!(
            pipe.isochronous_schedule(100, &[3073]),
            Err(PipeError::TooLarge { requested: 3073, limit: 3072 })
        );
    }

    #[test]
    fn reserved_or_misplaced_high_bandwidth_bits_are_rejected() {
        // Value 3 in bits 11..=12 is reserved.
        assert_eq!(Pipe::new(ep(0x82, ISO, 512 | (3 << 11))), Err(PipeError::BadEndpoint));
        // Bulk endpoints may not use extra transactions.
        assert_eq!(Pipe::new(ep(BULK_IN, BULK, 512 | (1 << 11))), Err(PipeError::BadEndpoint));
        // A zero packet size cannot carry data.
        assert_eq!(Pipe::new(ep(BULK_IN, BULK, 0)), Err(PipeError::BadEndpoint));
    }

    #[test]
    fn requests_must_match_the_endpoint_type() {
        let mut iso = Pipe::new(ep(0x82, ISO, 256)).unwrap();
        assert_eq!(iso.enqueue(64), Err(PipeError::WrongTransferType));
        assert_eq!(iso.enable_streams(), Err(PipeError::WrongTransferType));
        let mut bulk = Pipe::new(ep(BULK_IN, BULK, 512)).unwrap();
        assert_eq!(bulk.isochronous_schedule(0, &[64]), Err(PipeError::WrongTransferType));
        let mut interrupt = Pipe::new(ep(0x83, INTERRUPT, 16)).unwrap();
        assert_eq!(interrupt.enqueue(8), Ok(1));
        assert_eq!(interrupt.enqueue(8), Ok(2));
        assert_eq!(interrupt.enable_streams(), Err(PipeError::WrongTransferType));
    }

    #[test]
    fn halt_aborts_the_queue_and_clear_stall_reopens_it() {
        let mut pipe = Pipe::new(ep(BULK_IN, BULK, 512)).unwrap();
        assert_eq!(pipe.enqueue(64), Ok(1));
        assert_eq!(pipe.enqueue(16), Ok(2));
        assert_eq!(pipe.halt(), vec![1, 2]);
        assert!(pipe.is_halted());
        assert_eq!(pipe.pending_count(), 0);
        assert_eq!(pipe.enqueue(8), Err(PipeError::Halted));
        pipe.clear_stall();
        // Identifiers keep counting after a halt.
        assert_eq!(pipe.enqueue(8), Ok(3));
        assert_eq!(pipe.complete(3, 8), Ok(Completion { id: 3, requested: 8, transferred: 8 }));
        assert_eq!(pipe.complete(3, 8), Err(PipeError::UnknownRequest(3)));
    }

    #[test]
    fn completion_reports_short_transfers_and_rejects_overrun() {
        let mut pipe = Pipe::new(ep(BULK_IN, BULK, 512)).unwrap();
        let id = pipe.enqueue(64).unwrap();
        let done = pipe.complete(id, 40).unwrap();
        assert!(done.is_short());
        assert_eq!(pipe.complete(id, 0), Err(PipeError::UnknownRequest(id)));
        let id = pipe.enqueue(4).unwrap();
        assert_eq!(pipe.complete(id, 5), Err(PipeError::Overrun { requested: 4, transferred: 5 }));
        // The overrunning completion leaves the request queued.
        assert_eq!(pipe.pending_count(), 1);
    }

    #[test]
    fn iso_frame_status_follows_the_reported_count() {
        let mut frame = IsoFrame::new(1024);
        assert_eq!(frame.status(), FrameStatus::Pending);
        frame.record(500).unwrap();
        assert_eq!(frame.status(), FrameStatus::Partial);
        frame.record(1024).unwrap();
        assert_eq!(frame.status(), FrameStatus::Complete);
        assert_eq!(frame.record(1025), Err(PipeError::Overrun { requested: 1024, transferred: 1025 }));
    }

    #[test]
    fn streams_open_once_per_id_and_close_together() {
        let mut pipe = Pipe::new(ep(BULK_IN, BULK, 512)).unwrap();
        assert_eq!(pipe.copy_stream(7), Err(PipeError::StreamsDisabled));
        pipe.enable_streams().unwrap();
        assert_eq!(pipe.enable_streams(), Err(PipeError::StreamsEnabled));
        assert_eq!(pipe.copy_stream(0), Err(PipeError::BadStreamId(0)));
        assert_eq!(pipe.copy_stream(65534), Err(PipeError::BadStreamId(65534)));
        assert_eq!(pipe.copy_stream(7), Ok(7));
        assert_eq!(pipe.copy_stream(7), Ok(7));
        assert_eq!(pipe.copy_stream(9), Ok(9));
        assert_eq!(pipe.open_streams(), &[7u16, 9u16]);
        pipe.disable_streams().unwrap();
        assert_eq!(pipe.open_streams(), &[] as &[u16]);
        assert_eq!(pipe.disable_streams(), Err(PipeError::StreamsDisabled));
    }

    #[test]
    fn idle_timeout_is_stored_per_pipe() {
        let mut pipe = Pipe::new(ep(0x83, INTERRUPT, 16)).unwrap();
        assert_eq!(pipe.idle_timeout(), 0);
        pipe.set_idle_timeout(2500);
        assert_eq!(pipe.idle_timeout(), 2500);
    }
}
