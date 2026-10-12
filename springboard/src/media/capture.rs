//! Voice Memos capture: reads the default input device of `bifrost-audiod` through an IO
//! procedure, converts to S16 on the real-time thread into a lock-free ring, and a reader thread
//! collects the samples for the WAV file. No `/dev/snd`, no libasound, no mixer routing: the
//! server owns the hardware and the microphone permission.
//!
//! Nothing here touches a device until `Recorder::start`, which the app only calls on an
//! explicit Record tap. Playback of the result goes through the normal audio player.

use bifrost_audio_hal::{Device, IoContext, IoProcId, IoProcConfig, Role, System};
use bifrost_audio_types::{props, AudioError, BufferList, PropertyAddress, PropertyValue, TimeStamp};
use std::io;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

/// The client name the recorder says hello with (the recording lives in the player service).
pub const CLIENT: &str = "is.olibuijr.mediaplayer";

/// A voice memo is mono; the server converts from the device's own channel count.
pub const CHANNELS: u32 = 1;

/// Ring size in samples: about 2.7 s at 48 kHz mono, drained every 20 ms.
const RING_SAMPLES: usize = 131_072;

/// Single-producer single-consumer ring of S16 samples (atomics, no `unsafe`). The producer is the
/// IO procedure, the consumer the reader thread.
pub struct CaptureRing {
    cells: Box<[AtomicU32]>,
    head: AtomicU64,
    tail: AtomicU64,
    /// samples dropped because the reader fell behind
    dropped: AtomicU64,
    /// peak magnitude of the latest push, 0..=32767
    level: AtomicU32,
}

impl CaptureRing {
    pub fn new(samples: usize) -> Arc<CaptureRing> {
        Arc::new(CaptureRing { cells: (0..samples).map(|_| AtomicU32::new(0)).collect(), head: AtomicU64::new(0), tail: AtomicU64::new(0), dropped: AtomicU64::new(0), level: AtomicU32::new(0) })
    }

    /// Converts f32 samples (-1..=1) to S16 and queues them; what does not fit is counted as dropped.
    /// Real-time safe.
    pub fn push_f32(&self, samples: &[f32]) {
        let cap = self.cells.len() as u64;
        let head = self.head.load(Ordering::Relaxed);
        let room = cap - (head - self.tail.load(Ordering::Acquire)).min(cap);
        let take = (samples.len() as u64).min(room) as usize;
        let mut peak = 0u32;
        for (i, &v) in samples[..take].iter().enumerate() {
            let q = (v.clamp(-1.0, 1.0) * 32767.0).round() as i16;
            peak = peak.max(q.unsigned_abs() as u32);
            self.cells[((head + i as u64) % cap) as usize].store(q as u16 as u32, Ordering::Relaxed);
        }
        self.head.store(head + take as u64, Ordering::Release);
        self.level.store(peak, Ordering::Relaxed);
        if take < samples.len() {
            self.dropped.fetch_add((samples.len() - take) as u64, Ordering::Relaxed);
        }
    }

    /// Moves everything queued into `out`; returns how many samples.
    pub fn drain(&self, out: &mut Vec<i16>) -> usize {
        let cap = self.cells.len() as u64;
        let tail = self.tail.load(Ordering::Relaxed);
        let head = self.head.load(Ordering::Acquire);
        for i in tail..head {
            out.push(self.cells[(i % cap) as usize].load(Ordering::Relaxed) as u16 as i16);
        }
        self.tail.store(head, Ordering::Release);
        (head - tail) as usize
    }

    pub fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }

    pub fn level(&self) -> u32 {
        self.level.load(Ordering::Relaxed)
    }
}

/// The names of the input devices the audio server offers (the default input only today); empty
/// when the server is not running or has no input. Opens no stream.
pub fn discover() -> Vec<String> {
    let Ok(system) = System::connect(CLIENT) else { return Vec::new() };
    discover_with(&system).unwrap_or_default()
}

/// Like `discover` on a connection the caller keeps; None when the default input cannot be read (no input, or the
/// server went away: the caller reconnects).
pub fn discover_with(system: &System) -> Option<Vec<String>> {
    let device = system.default_device(Role::Input).ok()?;
    Some(match device.get(PropertyAddress::global(props::NAME)) {
        Ok(PropertyValue::String(n)) => vec![n],
        _ => vec![String::from("default input")],
    })
}

/// The IO procedure's body: the first channel of the first input buffer becomes mono S16.
/// Real-time safe: no allocation, no locks.
pub fn capture_cycle(ring: &CaptureRing, input: &BufferList) {
    if let Some(b) = input.buffers.first() {
        let n = b.channels.max(1) as usize;
        if n == 1 {
            ring.push_f32(&b.data);
        } else {
            for frame in b.data.chunks(n) {
                ring.push_f32(&frame[..1]);
            }
        }
    }
}

/// Interleaved S16 samples as a canonical 44-byte-header PCM WAV file.
pub fn wav_bytes(rate: u32, channels: u32, samples: &[i16]) -> Vec<u8> {
    let data_len = (samples.len() * 2) as u32;
    let mut out = Vec::with_capacity(44 + data_len as usize);
    out.extend(b"RIFF");
    out.extend((36 + data_len).to_le_bytes());
    out.extend(b"WAVEfmt ");
    out.extend(16u32.to_le_bytes());
    out.extend(1u16.to_le_bytes()); // PCM
    out.extend((channels as u16).to_le_bytes());
    out.extend(rate.to_le_bytes());
    out.extend((rate * channels * 2).to_le_bytes());
    out.extend(((channels * 2) as u16).to_le_bytes());
    out.extend(16u16.to_le_bytes());
    out.extend(b"data");
    out.extend(data_len.to_le_bytes());
    for s in samples {
        out.extend(s.to_le_bytes());
    }
    out
}

/// Duration in ms of a PCM WAV written by `wav_bytes` (or any canonical WAV); None when it is not one.
pub fn wav_duration_ms(header: &[u8], file_len: u64) -> Option<u64> {
    if header.len() < 44 || &header[..4] != b"RIFF" || &header[8..12] != b"WAVE" {
        return None;
    }
    let byte_rate = u32::from_le_bytes(header[28..32].try_into().ok()?) as u64;
    if byte_rate == 0 {
        return None;
    }
    Some(file_len.saturating_sub(44) * 1000 / byte_rate)
}

#[derive(Default)]
struct Shared {
    samples: Vec<i16>,
    /// peak magnitude of the most recent cycle, 0..=32768
    level: u16,
    error: Option<String>,
}

/// A recording in progress.
pub struct Recorder {
    shared: Arc<Mutex<Shared>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    device: Device,
    proc_id: IoProcId,
    pub rate: u32,
    pub channels: u32,
    /// the device the recording reads, for logs
    pub node: String,
}

/// A finished take.
pub struct Take {
    pub rate: u32,
    pub channels: u32,
    pub samples: Vec<i16>,
}

impl Take {
    pub fn wav(&self) -> Vec<u8> {
        wav_bytes(self.rate, self.channels, &self.samples)
    }
}

fn io_err(what: &str, e: AudioError) -> io::Error {
    let kind = match e {
        AudioError::PermissionDenied => io::ErrorKind::PermissionDenied,
        AudioError::NoServer => io::ErrorKind::NotConnected,
        AudioError::UnknownObject | AudioError::DeviceGone => io::ErrorKind::NotFound,
        _ => io::ErrorKind::Other,
    };
    io::Error::new(kind, format!("{what}: {e:?}"))
}

impl Recorder {
    /// Opens the default input device and starts the reader thread. Explicit user action only.
    pub fn start() -> io::Result<Recorder> {
        let system = System::connect(CLIENT).map_err(|e| io_err("connect to the audio server", e))?;
        let device = system.default_device(Role::Input).map_err(|e| io_err("default input device", e))?;
        let rate = match device.get(PropertyAddress::global(props::NOMINAL_RATE)) {
            Ok(PropertyValue::F64(r)) if r >= 1.0 => r.round() as u32,
            _ => 48_000,
        };
        let node = match device.get(PropertyAddress::global(props::NAME)) {
            Ok(PropertyValue::String(n)) => n,
            _ => String::from("default input"),
        };
        let ring = CaptureRing::new(RING_SAMPLES);
        let producer = ring.clone();
        let proc_id = device
            .add_io_proc(
                IoProcConfig { input_channels: CHANNELS, output_channels: 0, buffer_frames: 0 },
                Box::new(move |_ctx: &IoContext, _now: &TimeStamp, input: &BufferList, _input_ts: &TimeStamp, _out: &mut BufferList, _out_ts: &TimeStamp| capture_cycle(&producer, input)),
            )
            .map_err(|e| io_err("add the IO procedure", e))?;
        if let Err(e) = device.start(proc_id) {
            let _ = device.remove_io_proc(proc_id);
            return Err(io_err("start the input device", e));
        }
        let shared = Arc::new(Mutex::new(Shared::default()));
        let stop = Arc::new(AtomicBool::new(false));
        let (worker, halt) = (shared.clone(), stop.clone());
        let thread = crate::bare::sys::spawn_named("capture", move || {
            let mut chunk: Vec<i16> = Vec::with_capacity(4096);
            while !halt.load(Ordering::Acquire) {
                std::thread::sleep(Duration::from_millis(20));
                chunk.clear();
                ring.drain(&mut chunk);
                let mut s = worker.lock().unwrap();
                s.level = ring.level().min(32768) as u16;
                s.samples.extend_from_slice(&chunk);
                if ring.dropped() > 0 && s.error.is_none() {
                    s.error = Some(format!("{} samples were dropped: the recorder fell behind", ring.dropped()));
                }
            }
            chunk.clear();
            ring.drain(&mut chunk);
            worker.lock().unwrap().samples.extend_from_slice(&chunk);
        });
        eprintln!("audio.capture: recording {node} at {rate} Hz");
        Ok(Recorder { shared, stop, thread: Some(thread), device, proc_id, rate, channels: CHANNELS, node })
    }

    pub fn elapsed_ms(&self) -> u64 {
        let frames = self.shared.lock().unwrap().samples.len() as u64 / self.channels as u64;
        frames * 1000 / self.rate as u64
    }

    /// 0.0..=1.0 level of the latest audio, for a meter.
    pub fn level(&self) -> f32 {
        self.shared.lock().unwrap().level as f32 / 32768.0
    }

    /// The reader's error, if it died.
    pub fn error(&self) -> Option<String> {
        self.shared.lock().unwrap().error.clone()
    }

    fn halt(&mut self) {
        let _ = self.device.stop(self.proc_id);
        let _ = self.device.remove_io_proc(self.proc_id);
        self.stop.store(true, Ordering::Release);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }

    /// Stops the stream and returns what was recorded.
    pub fn stop(mut self) -> Take {
        self.halt();
        let samples = std::mem::take(&mut self.shared.lock().unwrap().samples);
        Take { rate: self.rate, channels: self.channels, samples }
    }
}

impl Drop for Recorder {
    fn drop(&mut self) {
        // dropped without `stop` (app killed): still release the device
        if self.thread.is_some() {
            self.halt();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_audio_types::Buffer;

    #[test]
    fn ring_converts_f32_to_s16_with_clamping_and_tracks_the_peak() {
        let r = CaptureRing::new(8);
        r.push_f32(&[0.5, -0.5, 2.0, -2.0]);
        let mut out = Vec::new();
        assert_eq!(r.drain(&mut out), 4);
        // 0.5 * 32767 = 16383.5 rounds away from zero to 16384
        assert_eq!(out, vec![16384, -16384, 32767, -32767]);
        assert_eq!(r.level(), 32767);
        assert_eq!(r.drain(&mut out), 0);
    }

    #[test]
    fn ring_drops_and_counts_what_does_not_fit() {
        let r = CaptureRing::new(4);
        r.push_f32(&[0.0; 6]);
        assert_eq!(r.dropped(), 2);
        let mut out = Vec::new();
        assert_eq!(r.drain(&mut out), 4);
        r.push_f32(&[0.25; 3]);
        assert_eq!(r.drain(&mut out), 3);
    }

    #[test]
    fn cycle_takes_the_first_channel_of_an_interleaved_buffer() {
        let r = CaptureRing::new(8);
        let input = BufferList { buffers: vec![Buffer { channels: 2, data: vec![0.5, -1.0, 0.25, 1.0] }] };
        capture_cycle(&r, &input);
        let mut out = Vec::new();
        r.drain(&mut out);
        // 0.25 * 32767 = 8191.75 rounds to 8192
        assert_eq!(out, vec![16384, 8192]);
    }

    #[test]
    fn io_errors_keep_the_permission_kind() {
        assert_eq!(io_err("x", AudioError::PermissionDenied).kind(), io::ErrorKind::PermissionDenied);
        assert_eq!(io_err("x", AudioError::NoServer).kind(), io::ErrorKind::NotConnected);
    }

    #[test]
    fn wav_header_is_canonical() {
        let w = wav_bytes(16_000, 1, &[1, -1, 2]);
        assert_eq!(w.len(), 44 + 6);
        assert_eq!(&w[..4], b"RIFF");
        assert_eq!(u32::from_le_bytes(w[4..8].try_into().unwrap()), 36 + 6);
        assert_eq!(u32::from_le_bytes(w[24..28].try_into().unwrap()), 16_000);
        assert_eq!(u32::from_le_bytes(w[28..32].try_into().unwrap()), 32_000);
        assert_eq!(i16::from_le_bytes(w[46..48].try_into().unwrap()), -1);
    }

    #[test]
    fn wav_duration_comes_from_the_byte_rate() {
        let w = wav_bytes(8_000, 1, &vec![0; 8_000]);
        assert_eq!(wav_duration_ms(&w[..44], w.len() as u64), Some(1000));
        assert_eq!(wav_duration_ms(b"nope", 4), None);
    }
}
