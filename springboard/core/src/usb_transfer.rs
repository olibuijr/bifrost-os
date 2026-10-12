//! USB transfer planning: alternate settings, endpoint pipes, packet splits and isochronous frame lists.
//!
//! Pure, like the rest of the USB model: no device is opened and no request is sent. The caller reads the
//! descriptors with `usb_descriptors`, picks an interface and an endpoint with this module, and then checks that the
//! transfer it wants fits the endpoint's type and direction. The same module splits a buffer into the packets of a
//! bulk, interrupt or control transfer, and into the per-frame request counts of an isochronous transfer, so the host
//! controller side only has to schedule what is planned here.
//!
//! Model:
//! - A pipe is one endpoint of the selected interface alternate setting. Its address, direction, transfer type,
//!   payload size per packet and high-speed transactions per microframe come from the endpoint descriptor.
//! - An isochronous frame is one bus frame (1 ms at full speed) or one microframe (125 us at high speed, eight to a
//!   frame). A frame carries at most `packet size x transactions` bytes per microframe.
//! - A frame list spreads the bytes over the requested frames: every frame gets the same count, and the first
//!   `bytes % frames` frames get one byte more. A frame list that cannot hold the bytes is refused, not truncated.
//!
//! The shape follows the official host-mode USB framework (interface, pipe and isochronous frame types), without its
//! driver objects, completion handlers or the bandwidth adjustment calls.

use std::ops::Range;

use crate::usb_descriptors::{ConfigurationDescriptor, EndpointDescriptor, InterfaceDescriptor, TransferType};

/// The bus speed that sets the length of an isochronous frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Speed {
    /// Full speed: one frame is 1000 microseconds and carries one packet.
    Full,
    /// High speed: one microframe is 125 microseconds, eight microframes make a frame.
    High,
}

impl Speed {
    /// Length of one frame (full speed) or microframe (high speed) in microseconds.
    pub fn frame_micros(self) -> u32 {
        match self {
            Speed::Full => 1000,
            Speed::High => 125,
        }
    }

    /// Microframes that make up one bus frame.
    fn microframes_per_frame(self) -> usize {
        match self {
            Speed::Full => 1,
            Speed::High => 8,
        }
    }
}

/// Why a planning call refuses a request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PipeError {
    /// The configuration has no interface with this number and alternate setting.
    NoInterface,
    /// The interface has no endpoint at this address.
    NoEndpoint,
    /// The endpoint has a different transfer type than the request needs.
    WrongTransferType,
    /// The endpoint runs in the other direction from the one the request needs.
    WrongDirection,
    /// The request needs at least one frame.
    NoFrames,
    /// The bytes do not fit the frames at the endpoint's per-frame capacity.
    FrameOverflow,
}

/// One endpoint of an interface alternate setting, with the packet geometry the transfer planning needs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pipe {
    /// `bEndpointAddress`.
    pub address: u8,
    pub transfer: TransferType,
    /// Device-to-host when true.
    pub is_in: bool,
    /// Payload bytes per packet: bits 0 to 10 of `wMaxPacketSize`.
    pub packet_size: u16,
    /// Transactions per microframe for high-speed periodic endpoints: bits 11 and 12 of `wMaxPacketSize`, plus one.
    pub transactions: u8,
    /// `bInterval`.
    pub interval: u8,
}

impl Pipe {
    /// Reads the pipe geometry from an endpoint descriptor.
    pub fn from_endpoint(e: &EndpointDescriptor) -> Pipe {
        let raw = e.max_packet_size;
        Pipe {
            address: e.address,
            transfer: e.transfer_type(),
            is_in: e.is_in(),
            packet_size: raw & 0x07ff,
            transactions: ((raw >> 11) & 0x03) as u8 + 1,
            interval: e.interval,
        }
    }

    /// Refuses unless the pipe has the transfer type and the direction the request needs.
    pub fn check(&self, transfer: TransferType, is_in: bool) -> Result<(), PipeError> {
        if self.transfer != transfer {
            return Err(PipeError::WrongTransferType);
        }
        if self.is_in != is_in {
            return Err(PipeError::WrongDirection);
        }
        Ok(())
    }

    /// Packets a transfer of `bytes` takes. A zero-byte transfer still takes one zero-length packet, and a pipe with
    /// no packet size takes none.
    pub fn packet_count(&self, bytes: usize) -> usize {
        let size = self.packet_size as usize;
        if size == 0 {
            0
        } else if bytes == 0 {
            1
        } else {
            (bytes - 1) / size + 1
        }
    }

    /// The byte range of each packet of a transfer of `bytes`, in order. The last range may be short.
    pub fn packet_ranges(&self, bytes: usize) -> Vec<Range<usize>> {
        let size = self.packet_size as usize;
        (0..self.packet_count(bytes))
            .map(|i| {
                let start = i * size;
                let end = (start + size).min(bytes);
                start..end
            })
            .collect()
    }

    /// Bytes this endpoint can move in one bus frame at the given speed.
    pub fn bytes_per_frame(&self, speed: Speed) -> usize {
        let per_microframe = match speed {
            Speed::Full => 1,
            Speed::High => self.transactions as usize,
        };
        self.packet_size as usize * per_microframe * speed.microframes_per_frame()
    }

    /// Splits `bytes` over `frames` isochronous frames. Each frame gets `bytes / frames` bytes, and the first
    /// `bytes % frames` frames get one more. Refused when there are no frames, or when a frame would need more than
    /// the endpoint's capacity for that speed.
    pub fn frame_list(&self, speed: Speed, bytes: usize, frames: usize) -> Result<Vec<IsochFrame>, PipeError> {
        if frames == 0 {
            return Err(PipeError::NoFrames);
        }
        let capacity = self.bytes_per_frame(speed).min(u32::MAX as usize);
        if bytes > frames.saturating_mul(capacity) {
            return Err(PipeError::FrameOverflow);
        }
        let base = bytes / frames;
        let extra = bytes % frames;
        Ok((0..frames)
            .map(|i| IsochFrame {
                request_count: (base + usize::from(i < extra)) as u32,
                complete_count: 0,
            })
            .collect())
    }
}

/// Plans an isochronous request on a pipe: the transfer type and direction must match, then the bytes go into the
/// frame list. Returns the frames with `request_count` set and `complete_count` at zero.
pub fn plan_isochronous(
    pipe: &Pipe,
    speed: Speed,
    is_in: bool,
    bytes: usize,
    frames: usize,
) -> Result<Vec<IsochFrame>, PipeError> {
    pipe.check(TransferType::Isochronous, is_in)?;
    pipe.frame_list(speed, bytes, frames)
}

/// One frame of an isochronous transfer: the bytes asked for and the bytes the controller moved.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct IsochFrame {
    pub request_count: u32,
    pub complete_count: u32,
}

/// Totals over a finished frame list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IsochSummary {
    pub requested: u64,
    pub completed: u64,
    /// Frames that moved fewer bytes than they asked for.
    pub short_frames: usize,
}

/// Adds up a finished frame list.
pub fn summarize(frames: &[IsochFrame]) -> IsochSummary {
    let mut s = IsochSummary {
        requested: 0,
        completed: 0,
        short_frames: 0,
    };
    for f in frames {
        s.requested += f.request_count as u64;
        s.completed += f.complete_count as u64;
        if f.complete_count < f.request_count {
            s.short_frames += 1;
        }
    }
    s
}

/// Time the frame list takes on the bus, in microseconds.
pub fn duration_micros(frames: usize, speed: Speed) -> u64 {
    frames as u64 * speed.frame_micros() as u64
}

/// The frame number after `frames` frames from `first`. Frame numbers wrap at 64 bits.
pub fn frame_number_after(first: u64, frames: usize) -> u64 {
    first.wrapping_add(frames as u64)
}

/// The alternate settings of an interface number, in the order the configuration lists them.
pub fn alternate_settings(config: &ConfigurationDescriptor, number: u8) -> Vec<u8> {
    config
        .interfaces
        .iter()
        .filter(|i| i.number == number)
        .map(|i| i.alternate)
        .collect()
}

/// The interface alternate setting to select, or `NoInterface` when the configuration does not have it.
pub fn select_alternate(
    config: &ConfigurationDescriptor,
    number: u8,
    alternate: u8,
) -> Result<&InterfaceDescriptor, PipeError> {
    config.interface(number, alternate).ok_or(PipeError::NoInterface)
}

/// The pipe at an endpoint address of an interface, or `NoEndpoint` when the interface does not have it.
pub fn copy_pipe(iface: &InterfaceDescriptor, address: u8) -> Result<Pipe, PipeError> {
    iface
        .endpoints
        .iter()
        .find(|e| e.address == address)
        .map(Pipe::from_endpoint)
        .ok_or(PipeError::NoEndpoint)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn endpoint(address: u8, attributes: u8, max_packet_size: u16) -> EndpointDescriptor {
        EndpointDescriptor {
            address,
            attributes,
            max_packet_size,
            interval: 4,
        }
    }

    fn bulk_out(mps: u16) -> Pipe {
        Pipe::from_endpoint(&endpoint(0x02, 2, mps))
    }

    fn iso_in(raw: u16) -> Pipe {
        Pipe::from_endpoint(&endpoint(0x83, 1, raw))
    }

    #[test]
    fn reads_size_and_transactions_from_the_packet_field() {
        // 0x1400 = 2 << 11 (three transactions) + 1024 bytes per packet; attributes 3 is interrupt.
        let p = Pipe::from_endpoint(&endpoint(0x81, 3, 0x1400));
        assert_eq!(p.packet_size, 1024);
        assert_eq!(p.transactions, 3);
        assert!(p.is_in);
        assert_eq!(p.transfer, TransferType::Interrupt);
        assert_eq!(p.address & 0x0f, 1);
        assert_eq!(p.interval, 4);
    }

    #[test]
    fn counts_packets_with_a_zero_length_packet_for_empty_transfers() {
        let p = bulk_out(64);
        assert_eq!(p.packet_count(0), 1);
        assert_eq!(p.packet_count(1), 1);
        assert_eq!(p.packet_count(64), 1);
        assert_eq!(p.packet_count(65), 2);
        assert_eq!(p.packet_count(200), 4);
        assert_eq!(bulk_out(0).packet_count(10), 0);
    }

    #[test]
    fn splits_a_buffer_into_packet_ranges_with_a_short_tail() {
        let p = bulk_out(64);
        assert_eq!(p.packet_ranges(150), vec![0..64, 64..128, 128..150]);
        assert_eq!(p.packet_ranges(128), vec![0..64, 64..128]);
        assert_eq!(p.packet_ranges(0), vec![0..0]);
    }

    #[test]
    fn refuses_a_transfer_of_the_wrong_type_or_direction() {
        let p = bulk_out(64);
        assert_eq!(p.check(TransferType::Bulk, false), Ok(()));
        assert_eq!(p.check(TransferType::Isochronous, false), Err(PipeError::WrongTransferType));
        assert_eq!(p.check(TransferType::Bulk, true), Err(PipeError::WrongDirection));
        assert_eq!(
            plan_isochronous(&p, Speed::Full, false, 64, 1),
            Err(PipeError::WrongTransferType)
        );
    }

    #[test]
    fn finds_endpoints_and_alternate_settings_by_address_and_number() {
        let iface = InterfaceDescriptor {
            number: 1,
            alternate: 2,
            class: 1,
            subclass: 2,
            protocol: 0,
            endpoints: vec![endpoint(0x83, 1, 0x0200)],
        };
        let config = ConfigurationDescriptor {
            value: 1,
            attributes: 0xa0,
            max_power_ma: 100,
            interfaces: vec![
                iface.clone(),
                InterfaceDescriptor {
                    alternate: 0,
                    endpoints: Vec::new(),
                    ..iface.clone()
                },
            ],
        };
        assert_eq!(alternate_settings(&config, 1), vec![2, 0]);
        assert!(select_alternate(&config, 1, 2).is_ok());
        assert_eq!(select_alternate(&config, 1, 7).err(), Some(PipeError::NoInterface));
        assert_eq!(copy_pipe(&iface, 0x83).map(|p| p.packet_size), Ok(512));
        assert_eq!(copy_pipe(&iface, 0x84), Err(PipeError::NoEndpoint));
    }

    #[test]
    fn spreads_bytes_over_full_speed_frames_with_the_remainder_in_front() {
        // Packet size 100 gives one 100-byte packet per 1 ms frame. 750 bytes over 8 frames: 93 each, and the
        // first six frames take 94.
        let p = iso_in(100);
        let frames = plan_isochronous(&p, Speed::Full, true, 750, 8).unwrap();
        let counts: Vec<u32> = frames.iter().map(|f| f.request_count).collect();
        assert_eq!(counts, vec![94, 94, 94, 94, 94, 94, 93, 93]);
        assert_eq!(counts.iter().map(|&c| c as u64).sum::<u64>(), 750);
        assert!(frames.iter().all(|f| f.complete_count == 0));
        // 1000 bytes cannot fit eight 100-byte frames.
        assert_eq!(
            plan_isochronous(&p, Speed::Full, true, 1000, 8),
            Err(PipeError::FrameOverflow)
        );
        assert_eq!(p.frame_list(Speed::Full, 0, 0), Err(PipeError::NoFrames));
    }

    #[test]
    fn uses_eight_microframes_and_transactions_at_high_speed() {
        // 512 bytes times two transactions times eight microframes is 8192 bytes per frame.
        let p = iso_in(512 | (1 << 11));
        assert_eq!(p.transactions, 2);
        assert_eq!(p.bytes_per_frame(Speed::High), 8192);
        assert_eq!(p.bytes_per_frame(Speed::Full), 512);
        let full = plan_isochronous(&p, Speed::High, true, 8192 * 4, 4).unwrap();
        assert!(full.iter().all(|f| f.request_count == 8192));
        assert_eq!(
            plan_isochronous(&p, Speed::High, true, 8192 * 4 + 1, 4),
            Err(PipeError::FrameOverflow)
        );
    }

    #[test]
    fn summarizes_completed_frames_and_counts_the_short_ones() {
        let frames = [
            IsochFrame {
                request_count: 10,
                complete_count: 10,
            },
            IsochFrame {
                request_count: 10,
                complete_count: 4,
            },
            IsochFrame::default(),
        ];
        let s = summarize(&frames);
        assert_eq!(s.requested, 20);
        assert_eq!(s.completed, 14);
        assert_eq!(s.short_frames, 1);
    }

    #[test]
    fn times_frames_by_the_bus_speed_and_wraps_frame_numbers() {
        assert_eq!(duration_micros(8, Speed::Full), 8000);
        assert_eq!(duration_micros(8, Speed::High), 1000);
        assert_eq!(Speed::High.frame_micros(), 125);
        assert_eq!(frame_number_after(10, 5), 15);
        assert_eq!(frame_number_after(u64::MAX, 2), 1);
    }
}
