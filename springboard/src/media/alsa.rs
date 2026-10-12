//! The `AudioOut` trait the players write to, and the silent `NullOut` fallback. Playback goes
//! through the audio server (`super::speaker`) or libasound (`super::alsalib`); the mixer side
//! (controls) is `super::mixer`. Capture is not here: the audio server owns the input hardware,
//! and `super::capture` reads it through `bifrost-audio-hal`.

use std::io;

/// Something that consumes 48 kHz stereo S16 and reports how far it got;
/// the player's clock is derived from `played`.
pub trait AudioOut: Send {
    /// Blocks until the interleaved frames are queued.
    fn write(&mut self, frames: &[i16]) -> io::Result<()>;
    /// Frames that have reached the speaker since the last `discard`.
    fn played(&mut self) -> u64;
    fn pause(&mut self, on: bool);
    /// Lets what is queued play out over silence, then drops it: no cut mid-wave (a click) and
    /// nothing stale left. Defaults to a plain `discard`.
    fn flush(&mut self) {
        self.discard();
    }
    /// Drops everything queued and restarts the count.
    fn discard(&mut self);
}

/// Waits between retries of a failed write; one more attempt than there are entries.
pub const WRITE_RETRY_BACKOFF: [std::time::Duration; 3] = [std::time::Duration::from_millis(20), std::time::Duration::from_millis(60), std::time::Duration::from_millis(180)];

/// Writes the chunk, retrying the same chunk after each `backoff` wait; the last error when all
/// attempts fail, so a hiccup does not skip audio and a dead device is reported.
pub fn write_with_retry(out: &mut dyn AudioOut, frames: &[i16], backoff: &[std::time::Duration]) -> io::Result<()> {
    let mut waits = backoff.iter();
    loop {
        match out.write(frames) {
            Ok(()) => return Ok(()),
            Err(e) => match waits.next() {
                Some(wait) => std::thread::sleep(*wait),
                None => return Err(e),
            },
        }
    }
}

/// A silent output that advances at wall-clock speed; used when no PCM
/// device opens (and by host tests) so video still plays.
pub struct NullOut {
    start: std::time::Instant,
    /// frames played before the current run (while paused this is all)
    base: u64,
    paused: bool,
    written: u64,
    /// Behaves like a PCM that drops its queue on pause: pausing drops the queue and `played` restarts at 0.
    drops_on_pause: bool,
}

impl NullOut {
    pub fn new() -> Self {
        NullOut { start: std::time::Instant::now(), base: 0, paused: false, written: 0, drops_on_pause: false }
    }

    /// A fake that, like some PCM drivers, loses its position when paused.
    pub fn dropping_on_pause() -> Self {
        NullOut { drops_on_pause: true, ..Self::new() }
    }
}

impl Default for NullOut {
    fn default() -> Self {
        Self::new()
    }
}

impl AudioOut for NullOut {
    fn write(&mut self, frames: &[i16]) -> io::Result<()> {
        self.written += (frames.len() / 2) as u64;
        // keep about 100 ms queued, like a real device would
        while !self.paused && self.written > self.played() + 4800 {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        Ok(())
    }

    fn played(&mut self) -> u64 {
        let run = if self.paused { 0 } else { (self.start.elapsed().as_secs_f64() * 48_000.0) as u64 };
        (self.base + run).min(self.written)
    }

    fn pause(&mut self, on: bool) {
        if on != self.paused {
            self.base = self.played();
            self.start = std::time::Instant::now();
            self.paused = on;
            if on && self.drops_on_pause {
                self.discard();
            }
        }
    }

    fn discard(&mut self) {
        self.base = 0;
        self.written = 0;
        self.start = std::time::Instant::now();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn null_output_paces_and_pauses() {
        let mut o = NullOut::new();
        o.write(&[0; 2 * 480]).unwrap();
        assert!(o.played() <= 480);
        o.pause(true);
        let p = o.played();
        std::thread::sleep(std::time::Duration::from_millis(20));
        assert_eq!(o.played(), p, "frozen while paused");
        o.pause(false);
        o.write(&[0; 2 * 9600]).unwrap();
        assert!(o.played() > 4000, "write blocks until the queue drains to ~100 ms");
        o.discard();
        assert_eq!(o.played(), 0);
    }

    #[test]
    fn dropping_null_output_restarts_at_zero_after_pause() {
        let mut o = NullOut::dropping_on_pause();
        o.write(&[0; 2 * 480]).unwrap();
        o.pause(true);
        o.pause(false);
        assert_eq!(o.played(), 0);
    }

    /// Fails its first `fails` writes.
    struct Flaky {
        fails: usize,
        attempts: usize,
        chunks: Vec<usize>,
    }

    impl AudioOut for Flaky {
        fn write(&mut self, frames: &[i16]) -> io::Result<()> {
            self.attempts += 1;
            if self.fails > 0 {
                self.fails -= 1;
                return Err(io::Error::other("busy"));
            }
            self.chunks.push(frames.len());
            Ok(())
        }
        fn played(&mut self) -> u64 {
            0
        }
        fn pause(&mut self, _: bool) {}
        fn discard(&mut self) {}
    }

    #[test]
    fn a_failed_write_is_retried_with_the_same_chunk_then_reported() {
        let none = [std::time::Duration::ZERO; 3];
        let mut o = Flaky { fails: 2, attempts: 0, chunks: vec![] };
        write_with_retry(&mut o, &[0; 8], &none).unwrap();
        assert_eq!((o.attempts, o.chunks.clone()), (3, vec![8]));
        let mut dead = Flaky { fails: 99, attempts: 0, chunks: vec![] };
        assert!(write_with_retry(&mut dead, &[0; 8], &none).is_err());
        assert_eq!(dead.attempts, 4, "one try plus one per backoff");
    }

}
