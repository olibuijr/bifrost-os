//! Speaker output on the desktop: the "default" PCM through libasound (`alsalib`), so the sound
//! server (PipeWire or PulseAudio) routes and mixes. The stream opens on the first write and is
//! released again after `IDLE_CLOSE` without playback so a paused or finished player does not
//! hold the device (opening the Music app alone never does).

use super::alsa::AudioOut;
use super::alsalib::AlsaLib;
use std::io;
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

/// Playback idle for this long closes the PCM.
const IDLE_CLOSE: Duration = Duration::from_secs(5);

const REPORT_EVERY: Duration = Duration::from_secs(1);

/// What was handed to the PCM since the last report: the trace that tells silence from stalls.
#[derive(Default, Debug, PartialEq)]
struct Meter {
    frames: u64,
    peak: u16,
    writes: u32,
    /// the longest single write, i.e. how long the writer waited for room
    slowest: Duration,
}

impl Meter {
    fn add(&mut self, samples: &[i16], took: Duration) {
        self.frames += samples.len() as u64 / 2;
        self.peak = samples.iter().map(|s| s.unsigned_abs()).max().unwrap_or(0).max(self.peak);
        self.writes += 1;
        self.slowest = self.slowest.max(took);
    }

    fn take(&mut self) -> Meter {
        std::mem::take(self)
    }
}

struct Session {
    rate: u32,
    channels: u32,
    /// the stream on the sound server (libasound)
    desk: Option<AlsaLib>,
    /// the last write, pause or discard
    active: Instant,
    meter: Meter,
    reported: Instant,
}

impl Session {
    /// Opens the PCM if that has not happened yet.
    fn ensure(&mut self) -> io::Result<()> {
        if self.desk.is_some() {
            return Ok(());
        }
        self.desk = Some(AlsaLib::open_default(self.rate, self.channels)?);
        eprintln!("speaker: opened the default PCM through libasound");
        Ok(())
    }

    fn is_open(&self) -> bool {
        self.desk.is_some()
    }

    /// The open output.
    fn out(&mut self) -> Option<&mut dyn AudioOut> {
        self.desk.as_mut().map(|d| d as &mut dyn AudioOut)
    }

    /// One trace line a second while playing: input level, throughput and the played count.
    fn report(&mut self) {
        let m = self.meter.take();
        self.reported = Instant::now();
        let played = self.desk.as_mut().map_or(0, |p| p.played());
        eprintln!("speaker: trace {} frames in {} writes, peak {}, slowest write {:?}, played {played}", m.frames, m.writes, m.peak, m.slowest);
    }

    /// Closes the PCM.
    fn release(&mut self) {
        self.desk = None;
    }
}

/// The speaker as an `AudioOut`.
pub struct SpeakerOut(Arc<Mutex<Session>>);

impl SpeakerOut {
    pub fn new(rate: u32, channels: u32) -> Self {
        let session = Arc::new(Mutex::new(Session { rate, channels, desk: None, active: Instant::now(), meter: Meter::default(), reported: Instant::now() }));
        let weak: Weak<Mutex<Session>> = Arc::downgrade(&session);
        std::thread::Builder::new()
            .name("speaker-idle".into())
            .spawn(move || {
                while let Some(session) = weak.upgrade() {
                    {
                        let mut s = session.lock().unwrap();
                        if s.is_open() && s.active.elapsed() >= IDLE_CLOSE {
                            s.release();
                            eprintln!("speaker: idle, closed");
                        }
                    }
                    drop(session);
                    std::thread::sleep(Duration::from_secs(1));
                }
            })
            .ok();
        SpeakerOut(session)
    }
}

impl AudioOut for SpeakerOut {
    fn write(&mut self, frames: &[i16]) -> io::Result<()> {
        let mut s = self.0.lock().unwrap();
        s.ensure()?;
        let began = Instant::now();
        let r = s.out().expect("opened above").write(frames);
        s.meter.add(frames, began.elapsed());
        if let Err(e) = &r {
            eprintln!("speaker: write failed after {:?}: {e}", began.elapsed());
            // start over on the next write
            s.release();
        }
        s.active = Instant::now();
        if s.reported.elapsed() >= REPORT_EVERY {
            s.report();
        }
        r
    }

    fn played(&mut self) -> u64 {
        self.0.lock().unwrap().out().map_or(0, |p| p.played())
    }

    fn pause(&mut self, on: bool) {
        let mut s = self.0.lock().unwrap();
        if let Some(p) = s.out() {
            p.pause(on);
        }
        s.active = Instant::now();
    }

    fn flush(&mut self) {
        let mut s = self.0.lock().unwrap();
        if let Some(p) = s.out() {
            p.flush();
        }
        s.active = Instant::now();
    }

    fn discard(&mut self) {
        let mut s = self.0.lock().unwrap();
        if let Some(p) = s.out() {
            p.discard();
        }
        s.active = Instant::now();
    }
}

impl Drop for SpeakerOut {
    fn drop(&mut self) {
        if let Ok(mut s) = self.0.lock() {
            if s.is_open() {
                s.release();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn meter_tracks_frames_peak_and_slowest_write() {
        let mut m = Meter::default();
        m.add(&[0, 0, -300, 120], Duration::from_millis(3));
        m.add(&[10, 10], Duration::from_millis(1));
        assert_eq!(m.frames, 3);
        assert_eq!(m.peak, 300);
        assert_eq!(m.writes, 2);
        assert_eq!(m.slowest, Duration::from_millis(3));
    }

    #[test]
    fn meter_take_resets() {
        let mut m = Meter::default();
        m.add(&[i16::MIN, 0], Duration::ZERO);
        assert_eq!(m.take().peak, 32768);
        assert_eq!(m, Meter::default());
    }

    #[test]
    fn silence_has_zero_peak() {
        let mut m = Meter::default();
        m.add(&[0; 8], Duration::ZERO);
        assert_eq!(m.peak, 0);
    }
}
