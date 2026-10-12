//! PCM uapi of `include/uapi/sound/asound.h` on a 64-bit target: hw and sw params, the mmap status and control
//! pages, `SYNC_PTR`, the ioctl numbers and the constants. Copied from `springboard/src/media/alsa.rs` and
//! extended; sizes, offsets and request numbers are pinned by the tests (checked against the installed header).

use crate::sys::{io, ior, iow, iowr};

#[repr(C)]
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct Interval {
    pub min: u32,
    pub max: u32,
    /// bit 0 openmin, 1 openmax, 2 integer, 3 empty
    pub flags: u32,
}

pub const INTERVAL_OPENMIN: u32 = 1;
pub const INTERVAL_OPENMAX: u32 = 2;
pub const INTERVAL_INTEGER: u32 = 4;
pub const INTERVAL_EMPTY: u32 = 8;

#[repr(C)]
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct Mask {
    pub bits: [u32; 8],
}

/// struct snd_pcm_hw_params
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct HwParams {
    pub flags: u32,
    pub masks: [Mask; 3],
    pub mres: [Mask; 5],
    pub intervals: [Interval; 12],
    pub ires: [Interval; 9],
    pub rmask: u32,
    pub cmask: u32,
    pub info: u32,
    pub msbits: u32,
    pub rate_num: u32,
    pub rate_den: u32,
    pub fifo_size: u64,
    pub reserved: [u8; 64],
}

/// struct snd_pcm_sw_params
#[repr(C)]
#[derive(Clone, Copy)]
pub struct SwParams {
    pub tstamp_mode: i32,
    pub period_step: u32,
    pub sleep_min: u32,
    pub avail_min: u64,
    pub xfer_align: u64,
    pub start_threshold: u64,
    pub stop_threshold: u64,
    pub silence_threshold: u64,
    pub silence_size: u64,
    pub boundary: u64,
    pub proto: u32,
    pub tstamp_type: u32,
    pub reserved: [u8; 56],
}

/// struct timespec as the 64-bit mmap pages carry it.
#[repr(C)]
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct Timespec {
    pub sec: i64,
    pub nsec: i64,
}

impl Timespec {
    pub fn ns(&self) -> u64 {
        (self.sec as u64).wrapping_mul(1_000_000_000).wrapping_add(self.nsec as u64)
    }
}

/// struct snd_pcm_mmap_status (the 64-bit layout, `snd_pcm_mmap_status64`).
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct MmapStatus {
    pub state: i32,
    pub pad1: i32,
    pub hw_ptr: u64,
    pub tstamp: Timespec,
    pub suspended_state: i32,
    pub pad2: i32,
    pub audio_tstamp: Timespec,
}

/// struct snd_pcm_mmap_control
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct MmapControl {
    pub appl_ptr: u64,
    pub avail_min: u64,
}

/// struct snd_pcm_sync_ptr: the status and control unions are padded to 64 bytes each.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct SyncPtr {
    pub flags: u32,
    pub pad: u32,
    pub status: MmapStatus,
    pub status_pad: [u8; 8],
    pub control: MmapControl,
    pub control_pad: [u8; 48],
}

impl SyncPtr {
    pub fn zeroed() -> SyncPtr {
        SyncPtr { flags: 0, pad: 0, status: MmapStatus::default(), status_pad: [0; 8], control: MmapControl::default(), control_pad: [0; 48] }
    }
}

/// struct snd_pcm_info (the part of the control device's PCM_INFO answer we read).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct PcmInfo {
    pub device: u32,
    pub subdevice: u32,
    pub stream: i32,
    pub card: i32,
    pub id: [u8; 64],
    pub name: [u8; 80],
    pub subname: [u8; 32],
    pub dev_class: i32,
    pub dev_subclass: i32,
    pub subdevices_count: u32,
    pub subdevices_avail: u32,
    pub pad1: [u8; 16],
    pub reserved: [u8; 64],
}

// hw_params parameter numbers (masks 0..2, intervals from 8).
pub const PARAM_ACCESS: usize = 0;
pub const PARAM_FORMAT: usize = 1;
pub const PARAM_SUBFORMAT: usize = 2;
pub const PARAM_SAMPLE_BITS: usize = 8;
pub const PARAM_FRAME_BITS: usize = 9;
pub const PARAM_CHANNELS: usize = 10;
pub const PARAM_RATE: usize = 11;
pub const PARAM_PERIOD_TIME: usize = 12;
pub const PARAM_PERIOD_SIZE: usize = 13;
pub const PARAM_PERIOD_BYTES: usize = 14;
pub const PARAM_PERIODS: usize = 15;
pub const PARAM_BUFFER_TIME: usize = 16;
pub const PARAM_BUFFER_SIZE: usize = 17;
pub const PARAM_BUFFER_BYTES: usize = 18;

pub const ACCESS_MMAP_INTERLEAVED: u32 = 0;
pub const ACCESS_MMAP_NONINTERLEAVED: u32 = 1;
pub const FORMAT_S16_LE: u32 = 2;
pub const FORMAT_S24_LE: u32 = 6;
pub const FORMAT_S32_LE: u32 = 10;

pub const INFO_MMAP: u32 = 0x1;
pub const INFO_MMAP_VALID: u32 = 0x2;
pub const INFO_SYNC_APPLPTR: u32 = 0x20;
pub const INFO_NO_PERIOD_WAKEUP: u32 = 0x0080_0000;
/// `hw_params.flags`: disable period wakeups.
pub const HW_PARAMS_NO_PERIOD_WAKEUP: u32 = 1 << 2;

pub const STATE_OPEN: i32 = 0;
pub const STATE_SETUP: i32 = 1;
pub const STATE_PREPARED: i32 = 2;
pub const STATE_RUNNING: i32 = 3;
pub const STATE_XRUN: i32 = 4;
pub const STATE_DRAINING: i32 = 5;
pub const STATE_PAUSED: i32 = 6;
pub const STATE_SUSPENDED: i32 = 7;
pub const STATE_DISCONNECTED: i32 = 8;

pub const TSTAMP_MODE_ENABLE: i32 = 1;
pub const TSTAMP_TYPE_MONOTONIC_RAW: u32 = 2;

pub const SYNC_PTR_HWSYNC: u32 = 1 << 0;
pub const SYNC_PTR_APPL: u32 = 1 << 1;
pub const SYNC_PTR_AVAIL_MIN: u32 = 1 << 2;

pub const MMAP_OFFSET_DATA: i64 = 0;
pub const MMAP_OFFSET_STATUS_OLD: i64 = 0x8000_0000;
pub const MMAP_OFFSET_CONTROL_OLD: i64 = 0x8100_0000;
pub const MMAP_OFFSET_STATUS_NEW: i64 = 0x8200_0000;
pub const MMAP_OFFSET_CONTROL_NEW: i64 = 0x8300_0000;

pub const STREAM_PLAYBACK: i32 = 0;
pub const STREAM_CAPTURE: i32 = 1;

pub const PCM_PVERSION: u32 = ior::<i32>(b'A', 0x00);
pub const PCM_HW_REFINE: u32 = iowr::<HwParams>(b'A', 0x10);
pub const PCM_HW_PARAMS: u32 = iowr::<HwParams>(b'A', 0x11);
pub const PCM_HW_FREE: u32 = io(b'A', 0x12);
pub const PCM_SW_PARAMS: u32 = iowr::<SwParams>(b'A', 0x13);
pub const PCM_HWSYNC: u32 = io(b'A', 0x22);
pub const PCM_SYNC_PTR: u32 = iowr::<SyncPtr>(b'A', 0x23);
pub const PCM_PREPARE: u32 = io(b'A', 0x40);
pub const PCM_RESET: u32 = io(b'A', 0x41);
pub const PCM_START: u32 = io(b'A', 0x42);
pub const PCM_DROP: u32 = io(b'A', 0x43);
pub const PCM_RESUME: u32 = io(b'A', 0x47);
pub const PCM_LINK: u32 = iow::<i32>(b'A', 0x60);
pub const PCM_UNLINK: u32 = io(b'A', 0x61);

impl HwParams {
    /// Everything allowed, as alsa-lib's `snd_pcm_hw_params_any`.
    pub fn any() -> HwParams {
        let full = Interval { min: 0, max: u32::MAX, flags: 0 };
        let mut p = HwParams {
            flags: 0,
            masks: [Mask { bits: [u32::MAX; 8] }; 3],
            mres: [Mask::default(); 5],
            intervals: [full; 12],
            ires: [Interval::default(); 9],
            rmask: u32::MAX,
            cmask: 0,
            info: 0,
            msbits: 0,
            rate_num: 0,
            rate_den: 0,
            fifo_size: 0,
            reserved: [0; 64],
        };
        for param in [PARAM_SAMPLE_BITS, PARAM_FRAME_BITS, PARAM_CHANNELS, PARAM_RATE, PARAM_PERIOD_SIZE, PARAM_PERIODS, PARAM_BUFFER_SIZE] {
            p.intervals[param - 8].flags = INTERVAL_INTEGER;
        }
        p
    }

    /// Allows exactly the listed bits of a mask parameter.
    pub fn set_mask(&mut self, param: usize, bits: &[u32]) {
        let mut m = Mask::default();
        for &b in bits {
            m.bits[(b / 32) as usize] |= 1 << (b % 32);
        }
        self.masks[param] = m;
    }

    pub fn mask_has(&self, param: usize, bit: u32) -> bool {
        self.masks[param].bits[(bit / 32) as usize] & (1 << (bit % 32)) != 0
    }

    pub fn set_range(&mut self, param: usize, min: u32, max: u32) {
        let i = &mut self.intervals[param - 8];
        i.min = min;
        i.max = max;
        i.flags &= !(INTERVAL_OPENMIN | INTERVAL_OPENMAX | INTERVAL_EMPTY);
    }

    pub fn range(&self, param: usize) -> (u32, u32) {
        let i = &self.intervals[param - 8];
        (i.min, i.max)
    }

    /// True when the kernel marked the interval empty (no solution).
    pub fn is_empty(&self, param: usize) -> bool {
        self.intervals[param - 8].flags & INTERVAL_EMPTY != 0
    }
}

impl SwParams {
    pub fn zeroed() -> SwParams {
        SwParams {
            tstamp_mode: 0,
            period_step: 0,
            sleep_min: 0,
            avail_min: 0,
            xfer_align: 0,
            start_threshold: 0,
            stop_threshold: 0,
            silence_threshold: 0,
            silence_size: 0,
            boundary: 0,
            proto: 0,
            tstamp_type: 0,
            reserved: [0; 56],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::{offset_of, size_of};

    #[test]
    fn struct_sizes_match_the_kernel_header() {
        assert_eq!(size_of::<Interval>(), 12);
        assert_eq!(size_of::<Mask>(), 32);
        assert_eq!(size_of::<HwParams>(), 608);
        assert_eq!(size_of::<SwParams>(), 136);
        assert_eq!(size_of::<Timespec>(), 16);
        assert_eq!(size_of::<MmapStatus>(), 56);
        assert_eq!(size_of::<MmapControl>(), 16);
        assert_eq!(size_of::<SyncPtr>(), 136);
        assert_eq!(size_of::<PcmInfo>(), 288);
    }

    #[test]
    fn struct_offsets_match_the_kernel_header() {
        assert_eq!(offset_of!(HwParams, rmask), 512);
        assert_eq!(offset_of!(HwParams, info), 520);
        assert_eq!(offset_of!(HwParams, fifo_size), 536);
        assert_eq!(offset_of!(SwParams, avail_min), 16);
        assert_eq!(offset_of!(SwParams, stop_threshold), 40);
        assert_eq!(offset_of!(SwParams, boundary), 64);
        assert_eq!(offset_of!(SwParams, tstamp_type), 76);
        assert_eq!(offset_of!(MmapStatus, hw_ptr), 8);
        assert_eq!(offset_of!(MmapStatus, tstamp), 16);
        assert_eq!(offset_of!(MmapStatus, suspended_state), 32);
        assert_eq!(offset_of!(MmapStatus, audio_tstamp), 40);
        assert_eq!(offset_of!(MmapControl, avail_min), 8);
        assert_eq!(offset_of!(SyncPtr, status), 8);
        assert_eq!(offset_of!(SyncPtr, control), 72);
        assert_eq!(offset_of!(PcmInfo, id), 16);
        assert_eq!(offset_of!(PcmInfo, name), 80);
        assert_eq!(offset_of!(PcmInfo, subname), 160);
        assert_eq!(offset_of!(PcmInfo, dev_class), 192);
    }

    #[test]
    fn ioctl_numbers_match_the_kernel_header() {
        assert_eq!(PCM_PVERSION, 0x8004_4100);
        assert_eq!(PCM_HW_REFINE, 0xc260_4110);
        assert_eq!(PCM_HW_PARAMS, 0xc260_4111);
        assert_eq!(PCM_HW_FREE, 0x4112);
        assert_eq!(PCM_SW_PARAMS, 0xc088_4113);
        assert_eq!(PCM_HWSYNC, 0x4122);
        assert_eq!(PCM_SYNC_PTR, 0xc088_4123);
        assert_eq!(PCM_PREPARE, 0x4140);
        assert_eq!(PCM_RESET, 0x4141);
        assert_eq!(PCM_START, 0x4142);
        assert_eq!(PCM_DROP, 0x4143);
        assert_eq!(PCM_RESUME, 0x4147);
        assert_eq!(PCM_LINK, 0x4004_4160);
        assert_eq!(PCM_UNLINK, 0x4161);
    }

    #[test]
    fn any_allows_everything_with_integer_intervals() {
        let p = HwParams::any();
        assert!(p.mask_has(PARAM_FORMAT, FORMAT_S32_LE));
        assert_eq!(p.range(PARAM_RATE), (0, u32::MAX));
        assert_eq!(p.intervals[PARAM_RATE - 8].flags, INTERVAL_INTEGER);
        // time parameters are not integer intervals
        assert_eq!(p.intervals[PARAM_PERIOD_TIME - 8].flags, 0);
    }

    #[test]
    fn masks_and_ranges_narrow() {
        let mut p = HwParams::any();
        p.set_mask(PARAM_FORMAT, &[FORMAT_S32_LE, FORMAT_S16_LE]);
        assert!(p.mask_has(PARAM_FORMAT, 10) && p.mask_has(PARAM_FORMAT, 2) && !p.mask_has(PARAM_FORMAT, 6));
        // bit 10 sits in word 0, bit 33 in word 1
        p.set_mask(PARAM_ACCESS, &[1, 33]);
        assert_eq!(p.masks[PARAM_ACCESS].bits[..2], [2, 2]);
        p.set_range(PARAM_RATE, 48_000, 48_000);
        assert_eq!(p.range(PARAM_RATE), (48_000, 48_000));
        p.intervals[0].flags = INTERVAL_EMPTY;
        assert!(p.is_empty(PARAM_SAMPLE_BITS));
        p.set_range(PARAM_SAMPLE_BITS, 1, 2);
        assert!(!p.is_empty(PARAM_SAMPLE_BITS), "set_range clears the empty flag");
    }

    #[test]
    fn timespec_to_ns() {
        assert_eq!(Timespec { sec: 3, nsec: 250 }.ns(), 3_000_000_250);
    }
}
