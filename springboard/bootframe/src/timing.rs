//! The boot timing record `EFI/BIFROST/boot-timing`: measured stage durations of the last boots, smoothed, plus the
//! appearance. Little endian, versioned (`b"BBT1"`), fixed size, hostile-input safe. Bootweave writes it after
//! `desktop-faded` (task 06); the loader reads it (task 03). Owner: task 03.

use crate::Appearance;

/// The stages the bar has checkpoints for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    /// Loader start to kernel start.
    Loader,
    /// Kernel start to Bootweave start.
    Kernel,
    /// Bootweave start to desktop-ready.
    Userspace,
}

/// Smoothed durations in milliseconds and how many boots they average (at most 5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BootTiming {
    pub loader_ms: u32,
    pub kernel_ms: u32,
    pub userspace_ms: u32,
    pub samples: u32,
    pub appearance: Appearance,
}

pub const MAGIC: &[u8; 4] = b"BBT1";
/// The encoded size in bytes.
pub const LEN: usize = 24;

impl BootTiming {
    /// The defaults used when there is no record (first boot, after an update).
    pub const DEFAULT: BootTiming = BootTiming { loader_ms: 300, kernel_ms: 1500, userspace_ms: 2000, samples: 0, appearance: Appearance::Dark };

    pub fn total_ms(&self) -> u32 {
        self.loader_ms + self.kernel_ms + self.userspace_ms
    }

    /// Folds one measured boot into the average (a running mean over at most 5 boots).
    pub fn add_sample(&mut self, loader_ms: u32, kernel_ms: u32, userspace_ms: u32) {
        let _ = (loader_ms, kernel_ms, userspace_ms);
        todo!("boot-03")
    }

    pub fn encode(&self) -> [u8; LEN] {
        todo!("boot-03")
    }

    /// `None` for a wrong magic, length or a value out of range.
    pub fn decode(bytes: &[u8]) -> Option<BootTiming> {
        let _ = bytes;
        todo!("boot-03")
    }
}
