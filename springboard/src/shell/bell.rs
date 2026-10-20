//! The terminal bell (BEL, `tput bel`): a program rang it, so the shell plays the Alert Sound of Settings > Sounds at
//! the alert volume. While sound is muted or the audio stack cannot play, the bell flashes the terminal instead (a
//! visual bell). A program that rings in a loop (cat of a binary file) is held to one ring per [`MIN_GAP_MS`].
//!
//! The pty side only counts rings (`term::Terminal::take_rang`); [`Bell::take`] decides what one ring does at time
//! `t`, and the shell turns the answer into `Effect::AlertSound` and a short white veil over the terminal.

use std::sync::atomic::{AtomicBool, Ordering};

/// Rings closer together than this are one ring.
pub const MIN_GAP_MS: f64 = 250.0;
/// How long the visual bell stays on screen.
pub const FLASH_MS: f64 = 160.0;
/// The veil's alpha at the first frame of a flash; it fades to nothing over [`FLASH_MS`].
pub const FLASH_ALPHA: u8 = 90;

/// Set by the audio worker when the last sound it tried did not play (no audio server, no device), cleared by the
/// next one that did. The bell flashes as well while it is set.
static SOUND_FAILED: AtomicBool = AtomicBool::new(false);

/// Records whether the last system sound reached the audio server.
pub fn set_sound_failed(failed: bool) {
    SOUND_FAILED.store(failed, Ordering::Relaxed);
}

/// Whether the last system sound failed to play.
pub fn sound_failed() -> bool {
    SOUND_FAILED.load(Ordering::Relaxed)
}

/// What one ring does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ring {
    /// the alert tone to play (an index into `options::ALERT_SOUNDS`); `None` while sound is muted
    pub sound: Option<usize>,
    /// flash the terminal: sound is muted, or the last sound did not play
    pub flash: bool,
}

#[derive(Default)]
pub struct Bell {
    pending: u32,
    last: Option<f64>,
    flash_from: Option<f64>,
}

impl Bell {
    /// A program rang the bell.
    pub fn ring(&mut self) {
        self.pending = self.pending.saturating_add(1);
    }

    /// Whether a ring waits for [`Bell::take`].
    pub fn pending(&self) -> bool {
        self.pending > 0
    }

    /// Settles the rings seen since the last call at time `t` (ms). `tone` is the alert tone to play, `None` while
    /// sound is muted; `failed` is [`sound_failed`]. None when nothing rang or the ring is inside the gap of the last.
    pub fn take(&mut self, t: f64, tone: Option<usize>, failed: bool) -> Option<Ring> {
        if std::mem::take(&mut self.pending) == 0 {
            return None;
        }
        if self.last.is_some_and(|last| t - last < MIN_GAP_MS) {
            return None;
        }
        self.last = Some(t);
        let flash = tone.is_none() || failed;
        if flash {
            self.flash_from = Some(t);
        }
        Some(Ring { sound: tone, flash })
    }

    /// The veil alpha (0 when none) at time `t`: [`FLASH_ALPHA`] at the start of a flash, linear to 0 at its end.
    pub fn flash_alpha(&self, t: f64) -> u8 {
        match self.flash_from {
            Some(from) if t >= from && t < from + FLASH_MS => (f64::from(FLASH_ALPHA) * (1.0 - (t - from) / FLASH_MS)).round() as u8,
            _ => 0,
        }
    }

    /// When the flash on screen ends, so the frame after it is drawn without the veil. None while no flash shows at `t`.
    pub fn flash_end(&self, t: f64) -> Option<f64> {
        self.flash_from.map(|from| from + FLASH_MS).filter(|end| t < *end)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_ring_plays_the_chosen_tone_and_does_not_flash_while_sound_works() {
        let mut b = Bell::default();
        assert_eq!(b.take(1000.0, Some(3), false), None, "nothing rang");
        b.ring();
        assert!(b.pending());
        assert_eq!(b.take(1000.0, Some(3), false), Some(Ring { sound: Some(3), flash: false }));
        assert!(!b.pending());
        assert_eq!(b.flash_alpha(1010.0), 0);
    }

    #[test]
    fn a_muted_bell_flashes_instead() {
        let mut b = Bell::default();
        b.ring();
        assert_eq!(b.take(2000.0, None, false), Some(Ring { sound: None, flash: true }));
        assert_eq!(b.flash_alpha(2000.0), FLASH_ALPHA);
        assert!(b.flash_alpha(2000.0 + FLASH_MS / 2.0) < FLASH_ALPHA);
        assert!(b.flash_alpha(2000.0 + FLASH_MS / 2.0) > 0);
        assert_eq!(b.flash_alpha(2000.0 + FLASH_MS), 0, "gone at the end");
        assert_eq!(b.flash_end(2000.0), Some(2000.0 + FLASH_MS));
        assert_eq!(b.flash_end(2000.0 + FLASH_MS), None);
    }

    #[test]
    fn a_sound_that_did_not_play_flashes_too() {
        let mut b = Bell::default();
        b.ring();
        let ring = b.take(3000.0, Some(0), true).unwrap();
        assert_eq!(ring.sound, Some(0), "the sound is still tried, so a recovered server is noticed");
        assert!(ring.flash);
    }

    #[test]
    fn a_ring_loop_is_held_to_one_per_gap() {
        let mut b = Bell::default();
        b.ring();
        assert!(b.take(0.0, Some(0), false).is_some());
        b.ring();
        b.ring();
        assert_eq!(b.take(MIN_GAP_MS - 1.0, Some(0), false), None, "inside the gap");
        assert!(!b.pending(), "the dropped rings are gone, not queued");
        b.ring();
        assert!(b.take(MIN_GAP_MS, Some(0), false).is_some());
    }
}
