//! Rumble on a game controller's input node: uploads one two-motor force-feedback effect to the node and plays or
//! stops it. The effect is built from `bifrost_core::rumble::Rumble`; the kernel stops it after its length, and a
//! length of 0 runs until it is stopped.
//!
//! The kernel's `struct ff_effect` and `struct input_event` are written as bytes, little-endian, with the 64-bit layout
//! (the shell runs on 64-bit Linux only). Closing the node erases the effects it uploaded, so a [`Motor`] needs no
//! drop code of its own.

use super::sys;
use bifrost_core::rumble::Rumble;
use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::os::unix::io::AsRawFd;
use std::path::Path;

/// `EV_FF`: the event type that plays (value 1) or stops (value 0) an uploaded effect.
const EV_FF: u16 = 0x15;
/// `FF_RUMBLE`: the two-motor rumble effect code.
const FF_RUMBLE: u16 = 0x50;
/// `sizeof(struct ff_effect)` on a 64-bit kernel: a 16-byte header and the largest union member (a periodic effect).
const FF_EFFECT_SIZE: usize = 48;
/// `sizeof(struct input_event)` on a 64-bit kernel: a 16-byte timestamp, type, code and value.
const EVENT_SIZE: usize = 24;

// Byte offsets inside `struct ff_effect`.
const OFF_TYPE: usize = 0;
const OFF_ID: usize = 2;
const OFF_LENGTH: usize = 10;
const OFF_DELAY: usize = 12;
/// The rumble union member: `strong_magnitude` then `weak_magnitude`, both u16.
const OFF_STRONG: usize = 16;
const OFF_WEAK: usize = 18;

// Byte offsets inside `struct input_event`: type, code and value follow the timestamp.
const EV_TYPE: usize = 16;
const EV_CODE: usize = 18;
const EV_VALUE: usize = 20;

/// `EVIOCSFF` (`_IOW('E', 0x80, struct ff_effect)`): uploads an effect, or replaces the one whose id is in the request.
fn eviocsff() -> u32 {
    sys::iow::<[u8; FF_EFFECT_SIZE]>(b'E', 0x80)
}

/// `EVIOCRMFF` (`_IOW('E', 0x81, int)`): erases an uploaded effect. The id is passed by value.
fn eviocrmff() -> u32 {
    sys::iow::<i32>(b'E', 0x81)
}

/// The length the kernel plays an effect for, in milliseconds. 0 runs until stopped, and a timed rumble longer than
/// `u16::MAX` ms is cut to that.
fn length_ms(rumble: &Rumble) -> u16 {
    if rumble.is_infinite() {
        0
    } else {
        rumble.duration_ms.min(u32::from(u16::MAX)) as u16
    }
}

/// Whether a rumble ends at once: both motors off, or a timed rumble of no length.
fn ends_at_once(rumble: &Rumble) -> bool {
    rumble.is_silent() || (!rumble.is_infinite() && rumble.duration_ms == 0)
}

/// The `struct ff_effect` for a rumble. `id` is -1 for a new effect, or the id of the effect to replace.
pub fn effect_bytes(rumble: &Rumble, id: i16) -> [u8; FF_EFFECT_SIZE] {
    let (strong, weak) = rumble.magnitudes();
    let mut buf = [0u8; FF_EFFECT_SIZE];
    buf[OFF_TYPE..OFF_TYPE + 2].copy_from_slice(&FF_RUMBLE.to_le_bytes());
    buf[OFF_ID..OFF_ID + 2].copy_from_slice(&id.to_le_bytes());
    buf[OFF_LENGTH..OFF_LENGTH + 2].copy_from_slice(&length_ms(rumble).to_le_bytes());
    buf[OFF_DELAY..OFF_DELAY + 2].copy_from_slice(&0u16.to_le_bytes());
    buf[OFF_STRONG..OFF_STRONG + 2].copy_from_slice(&strong.to_le_bytes());
    buf[OFF_WEAK..OFF_WEAK + 2].copy_from_slice(&weak.to_le_bytes());
    buf
}

/// The `struct input_event` that plays (`value` 1) or stops (`value` 0) effect `id`. The timestamp is left zero.
pub fn play_event_bytes(id: i16, value: i32) -> [u8; EVENT_SIZE] {
    let mut ev = [0u8; EVENT_SIZE];
    ev[EV_TYPE..EV_TYPE + 2].copy_from_slice(&EV_FF.to_le_bytes());
    ev[EV_CODE..EV_CODE + 2].copy_from_slice(&(id as u16).to_le_bytes());
    ev[EV_VALUE..EV_VALUE + 4].copy_from_slice(&value.to_le_bytes());
    ev
}

/// The rumble motors of one controller: an open input node and the effect uploaded to it, if any.
#[derive(Debug)]
pub struct Motor {
    file: File,
    effect: Option<i16>,
}

impl Motor {
    /// Opens the input node for writing. Fails when the node is gone or the shell may not write to it.
    pub fn open(node: &Path) -> io::Result<Motor> {
        let file = OpenOptions::new().read(true).write(true).open(node)?;
        Ok(Motor { file, effect: None })
    }

    /// Plays `rumble`, replacing the one before. A silent rumble, or a timed one of no length, stops the motors.
    pub fn play(&mut self, rumble: Rumble) -> io::Result<()> {
        if ends_at_once(&rumble) {
            return self.stop();
        }
        let mut buf = effect_bytes(&rumble, self.effect.unwrap_or(-1));
        let fd = self.file.as_raw_fd();
        // SAFETY: `buf` is `FF_EFFECT_SIZE` bytes, the size `eviocsff` encodes; the kernel writes the effect id back.
        unsafe { sys::ioctl_ptr(fd, eviocsff(), buf.as_mut_ptr()) }?;
        let id = i16::from_le_bytes([buf[OFF_ID], buf[OFF_ID + 1]]);
        self.effect = Some(id);
        self.file.write_all(&play_event_bytes(id, 1))
    }

    /// Stops the effect and erases it. Nothing to stop: no-op.
    pub fn stop(&mut self) -> io::Result<()> {
        let Some(id) = self.effect.take() else {
            return Ok(());
        };
        self.file.write_all(&play_event_bytes(id, 0))?;
        let fd = self.file.as_raw_fd();
        // SAFETY: `EVIOCRMFF` takes the effect id as its integer argument, not a pointer.
        if unsafe { sys::ioctl(fd, eviocrmff() as _, i32::from(id)) } < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_timed_rumble_is_an_effect_with_its_magnitudes_and_length() {
        let buf = effect_bytes(&Rumble::new(1.0, 0.5, 250), -1);
        assert_eq!(u16::from_le_bytes([buf[0], buf[1]]), FF_RUMBLE);
        assert_eq!(i16::from_le_bytes([buf[2], buf[3]]), -1);
        assert_eq!(u16::from_le_bytes([buf[10], buf[11]]), 250);
        assert_eq!(u16::from_le_bytes([buf[12], buf[13]]), 0);
        assert_eq!(u16::from_le_bytes([buf[16], buf[17]]), 65_535);
        assert_eq!(u16::from_le_bytes([buf[18], buf[19]]), 32_768);
    }

    #[test]
    fn an_until_stopped_rumble_has_length_zero_and_a_long_one_is_cut() {
        let forever = effect_bytes(&Rumble::until_stopped(0.2, 0.2), 4);
        assert_eq!(u16::from_le_bytes([forever[10], forever[11]]), 0);
        assert_eq!(i16::from_le_bytes([forever[2], forever[3]]), 4);
        let long = effect_bytes(&Rumble::new(0.5, 0.5, 900_000), -1);
        assert_eq!(u16::from_le_bytes([long[10], long[11]]), u16::MAX);
    }

    #[test]
    fn a_play_event_names_the_effect_and_its_value() {
        let ev = play_event_bytes(7, 1);
        assert_eq!(ev.len(), 24);
        assert_eq!(u16::from_le_bytes([ev[16], ev[17]]), EV_FF);
        assert_eq!(u16::from_le_bytes([ev[18], ev[19]]), 7);
        assert_eq!(i32::from_le_bytes([ev[20], ev[21], ev[22], ev[23]]), 1);
        let stop = play_event_bytes(7, 0);
        assert_eq!(i32::from_le_bytes([stop[20], stop[21], stop[22], stop[23]]), 0);
    }

    #[test]
    fn a_silent_or_zero_length_rumble_ends_at_once() {
        assert!(ends_at_once(&Rumble::new(0.0, 0.0, 500)));
        assert!(ends_at_once(&Rumble::new(1.0, 1.0, 0)));
        assert!(!ends_at_once(&Rumble::until_stopped(1.0, 0.0)));
        assert!(!ends_at_once(&Rumble::new(0.0, 0.3, 100)));
    }

    #[test]
    fn a_missing_node_cannot_be_opened_as_a_motor() {
        assert!(Motor::open(Path::new("/nonexistent-dev-input/event99")).is_err());
    }
}
