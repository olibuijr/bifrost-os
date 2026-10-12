//! The boot timing record `EFI/BIFROST/boot-timing`: measured stage durations of the last boots, smoothed, plus the
//! appearance. Little endian, versioned (`b"BBT1"`), fixed size, checksummed, hostile-input safe. Bootweave writes it
//! after `desktop-faded` (task 06); the loader reads it (task 03). The loader also hands the record and its own
//! progress to the kernel side in the `BifrostBootProgress` EFI variable (`Handoff`). Owner: task 03.
//!
//! Layout of the 24-byte record (offsets in bytes):
//!
//! | Offset | Size | Field |
//! |---|---|---|
//! | 0 | 4 | magic `BBT1` |
//! | 4 | 4 | `loader_ms` (u32 LE) |
//! | 8 | 4 | `kernel_ms` (u32 LE) |
//! | 12 | 4 | `userspace_ms` (u32 LE) |
//! | 16 | 1 | `samples` (0..=5) |
//! | 17 | 1 | appearance (0 Dark, 1 Light) |
//! | 18 | 2 | reserved, zero |
//! | 20 | 4 | checksum: FNV-1a 32 over bytes 0..20 (u32 LE) |

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
/// The most boots the average holds.
pub const MAX_SAMPLES: u32 = 5;
/// Largest believable stage duration; anything above is a corrupt record.
pub const MAX_STAGE_MS: u32 = 600_000;

/// Size of the `BifrostBootProgress` variable: the record plus the loader's own numbers.
pub const HANDOFF_LEN: usize = LEN + 8;

/// FNV-1a, 32 bit: the checksum of the record (offset basis 0x811c9dc5, prime 0x01000193).
fn fnv1a(bytes: &[u8]) -> u32 {
    let mut h: u32 = 0x811c_9dc5;
    for &b in bytes {
        h ^= b as u32;
        h = h.wrapping_mul(0x0100_0193);
    }
    h
}

fn u32_at(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
}

impl BootTiming {
    /// The defaults used when there is no record (first boot, after an update).
    pub const DEFAULT: BootTiming = BootTiming { loader_ms: 300, kernel_ms: 1500, userspace_ms: 2000, samples: 0, appearance: Appearance::Dark };

    pub fn total_ms(&self) -> u32 {
        self.loader_ms + self.kernel_ms + self.userspace_ms
    }

    /// The predicted time since loader start at which `stage` ends, in milliseconds.
    pub fn stage_end_ms(&self, stage: Stage) -> u32 {
        match stage {
            Stage::Loader => self.loader_ms,
            Stage::Kernel => self.loader_ms.saturating_add(self.kernel_ms),
            Stage::Userspace => self.total_ms(),
        }
    }

    /// Folds one measured boot into the average. With `n` earlier samples (capped at 4) the new value is
    /// `(old * n + sample) / (n + 1)`, so the average settles at a weight of 1/5 per boot (the last 5 boots).
    /// The first measured boot replaces the defaults outright.
    pub fn add_sample(&mut self, loader_ms: u32, kernel_ms: u32, userspace_ms: u32) {
        let n = self.samples.min(MAX_SAMPLES - 1) as u64;
        let fold = |old: u32, new: u32| -> u32 { ((old as u64 * n + new as u64) / (n + 1)) as u32 };
        self.loader_ms = fold(self.loader_ms, loader_ms);
        self.kernel_ms = fold(self.kernel_ms, kernel_ms);
        self.userspace_ms = fold(self.userspace_ms, userspace_ms);
        self.samples = (self.samples + 1).min(MAX_SAMPLES);
    }

    pub fn encode(&self) -> [u8; LEN] {
        let mut out = [0u8; LEN];
        out[0..4].copy_from_slice(MAGIC);
        out[4..8].copy_from_slice(&self.loader_ms.to_le_bytes());
        out[8..12].copy_from_slice(&self.kernel_ms.to_le_bytes());
        out[12..16].copy_from_slice(&self.userspace_ms.to_le_bytes());
        out[16] = self.samples.min(MAX_SAMPLES) as u8;
        out[17] = match self.appearance {
            Appearance::Dark => 0,
            Appearance::Light => 1,
        };
        let sum = fnv1a(&out[..20]);
        out[20..24].copy_from_slice(&sum.to_le_bytes());
        out
    }

    /// `None` for a wrong magic, length, checksum, reserved byte, or a value out of range.
    pub fn decode(bytes: &[u8]) -> Option<BootTiming> {
        if bytes.len() != LEN || &bytes[0..4] != MAGIC {
            return None;
        }
        if fnv1a(&bytes[..20]) != u32_at(bytes, 20) {
            return None;
        }
        if bytes[18] != 0 || bytes[19] != 0 {
            return None;
        }
        let (loader_ms, kernel_ms, userspace_ms) = (u32_at(bytes, 4), u32_at(bytes, 8), u32_at(bytes, 12));
        for ms in [loader_ms, kernel_ms, userspace_ms] {
            if ms == 0 || ms > MAX_STAGE_MS {
                return None;
            }
        }
        let samples = bytes[16] as u32;
        if samples > MAX_SAMPLES {
            return None;
        }
        let appearance = match bytes[17] {
            0 => Appearance::Dark,
            1 => Appearance::Light,
            _ => return None,
        };
        Some(BootTiming { loader_ms, kernel_ms, userspace_ms, samples, appearance })
    }
}

/// The value of the `BifrostBootProgress` variable: the record, then the loader's elapsed time at hand-off and the
/// bar position it last showed (0..=1000 per mille). Read by Bootweave (task 06) to continue the bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Handoff {
    pub timing: BootTiming,
    /// Milliseconds from loader start to the kernel start.
    pub loader_ms: u32,
    /// The bar position at hand-off, in thousandths.
    pub shown_permille: u32,
}

impl Handoff {
    pub fn encode(&self) -> [u8; HANDOFF_LEN] {
        let mut out = [0u8; HANDOFF_LEN];
        out[..LEN].copy_from_slice(&self.timing.encode());
        out[24..28].copy_from_slice(&self.loader_ms.to_le_bytes());
        out[28..32].copy_from_slice(&self.shown_permille.min(1000).to_le_bytes());
        out
    }

    /// `None` when the length is wrong, the record is invalid, or a value is out of range.
    pub fn decode(bytes: &[u8]) -> Option<Handoff> {
        if bytes.len() != HANDOFF_LEN {
            return None;
        }
        let timing = BootTiming::decode(&bytes[..LEN])?;
        let loader_ms = u32_at(bytes, 24);
        let shown_permille = u32_at(bytes, 28);
        if loader_ms > MAX_STAGE_MS || shown_permille > 1000 {
            return None;
        }
        Some(Handoff { timing, loader_ms, shown_permille })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Re-seals a hand-edited record so only the field under test is wrong.
    fn reseal(bytes: &mut [u8; LEN]) {
        let sum = fnv1a(&bytes[..20]);
        bytes[20..24].copy_from_slice(&sum.to_le_bytes());
    }

    #[test]
    fn fnv1a_matches_the_published_vectors() {
        assert_eq!(fnv1a(b""), 0x811c_9dc5);
        assert_eq!(fnv1a(b"a"), 0xe40c_292c);
    }

    #[test]
    fn default_encodes_to_known_bytes() {
        let bytes = BootTiming::DEFAULT.encode();
        assert_eq!(&bytes[0..4], b"BBT1");
        // 300 ms = 0x12c, little endian.
        assert_eq!(&bytes[4..8], &[0x2c, 0x01, 0x00, 0x00]);
        assert_eq!(&bytes[8..12], &1500u32.to_le_bytes());
        assert_eq!(bytes[16], 0);
        assert_eq!(bytes[17], 0);
    }

    #[test]
    fn record_round_trips() {
        let t = BootTiming { loader_ms: 280, kernel_ms: 1410, userspace_ms: 1930, samples: 3, appearance: Appearance::Light };
        assert_eq!(BootTiming::decode(&t.encode()), Some(t));
    }

    #[test]
    fn decode_rejects_wrong_magic_length_and_checksum() {
        let mut bytes = BootTiming::DEFAULT.encode();
        assert!(BootTiming::decode(&bytes[..23]).is_none());
        let mut wrong = bytes;
        wrong[0] = b'X';
        reseal(&mut wrong);
        assert!(BootTiming::decode(&wrong).is_none());
        bytes[5] ^= 0x01; // a torn write: the checksum no longer matches
        assert!(BootTiming::decode(&bytes).is_none());
    }

    #[test]
    fn decode_rejects_out_of_range_fields() {
        let mut raw = BootTiming::DEFAULT.encode();
        raw[16] = 6; // samples above the cap
        reseal(&mut raw);
        assert!(BootTiming::decode(&raw).is_none());

        let mut raw = BootTiming::DEFAULT.encode();
        raw[17] = 2; // unknown appearance
        reseal(&mut raw);
        assert!(BootTiming::decode(&raw).is_none());

        let mut raw = BootTiming::DEFAULT.encode();
        raw[12..16].copy_from_slice(&(MAX_STAGE_MS + 1).to_le_bytes()); // userspace too long
        reseal(&mut raw);
        assert!(BootTiming::decode(&raw).is_none());

        let mut raw = BootTiming::DEFAULT.encode();
        raw[4..8].copy_from_slice(&0u32.to_le_bytes()); // a zero stage
        reseal(&mut raw);
        assert!(BootTiming::decode(&raw).is_none());
    }

    #[test]
    fn first_sample_replaces_the_defaults() {
        let mut t = BootTiming::DEFAULT;
        t.add_sample(280, 1400, 1900);
        assert_eq!((t.loader_ms, t.kernel_ms, t.userspace_ms, t.samples), (280, 1400, 1900, 1));
    }

    #[test]
    fn second_sample_averages_evenly() {
        let mut t = BootTiming { loader_ms: 280, kernel_ms: 1400, userspace_ms: 1900, samples: 1, appearance: Appearance::Dark };
        t.add_sample(320, 1600, 2100);
        // (280 + 320) / 2, (1400 + 1600) / 2, (1900 + 2100) / 2
        assert_eq!((t.loader_ms, t.kernel_ms, t.userspace_ms, t.samples), (300, 1500, 2000, 2));
    }

    #[test]
    fn average_settles_over_the_last_five_boots() {
        let mut t = BootTiming { loader_ms: 100, kernel_ms: 1000, userspace_ms: 1000, samples: 4, appearance: Appearance::Dark };
        t.add_sample(200, 1000, 1000);
        // n = 4: (100 * 4 + 200) / 5 = 120; the sample count stops at 5.
        assert_eq!((t.loader_ms, t.samples), (120, 5));
        t.add_sample(200, 1000, 1000);
        // Still n = 4: (120 * 4 + 200) / 5 = 136.
        assert_eq!((t.loader_ms, t.samples), (136, 5));
    }

    #[test]
    fn stage_ends_and_total() {
        let t = BootTiming::DEFAULT;
        assert_eq!(t.total_ms(), 3800);
        assert_eq!(t.stage_end_ms(Stage::Loader), 300);
        assert_eq!(t.stage_end_ms(Stage::Kernel), 1800);
        assert_eq!(t.stage_end_ms(Stage::Userspace), 3800);
    }

    #[test]
    fn handoff_round_trips_and_rejects_bad_values() {
        let h = Handoff { timing: BootTiming::DEFAULT, loader_ms: 240, shown_permille: 63 };
        let bytes = h.encode();
        assert_eq!(bytes.len(), 32);
        assert_eq!(&bytes[24..28], &240u32.to_le_bytes());
        assert_eq!(Handoff::decode(&bytes), Some(h));
        assert!(Handoff::decode(&bytes[..31]).is_none());
        let mut bad = bytes;
        bad[28..32].copy_from_slice(&1001u32.to_le_bytes());
        assert!(Handoff::decode(&bad).is_none());
    }
}
