//! ALSA capture for Voice Memos: finds a capture PCM from /proc/asound/pcm,
//! routes the microphone through the mixer, records interleaved S16 on a
//! thread and writes WAV files. The playback side is `alsa.rs` / `speaker.rs`.
//!
//! Nothing here touches a device until `Recorder::start`, which the app only
//! calls on an explicit Record tap. Playback of the result goes through the
//! normal audio player.

use super::alsa;
use super::mixer::Mixer;
use super::volume::CONTROL_DEVICE;
use std::io;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

/// Microphone routing: `(control, value)` mixer writes made, in order, before capture starts
/// (controls the card lacks are skipped, a refused write does not stop the rest).
///
/// Empty means "capture whatever the card routes by default", which is what a desktop sound
/// server or HDA codec does; a card that needs an input mux switched lists its controls here.
pub const MIC_ROUTE_ON: &[(&str, &str)] = &[];
/// Written back after the recording, in order (the inverse of `MIC_ROUTE_ON`).
pub const MIC_ROUTE_OFF: &[(&str, &str)] = &[];

/// What to try, best first: (rate, channels). The first the device accepts is used.
pub const FORMATS: [(u32, u32); 5] = [(48_000, 2), (48_000, 1), (44_100, 2), (16_000, 1), (8_000, 1)];

/// One line of /proc/asound/pcm.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PcmEntry {
    pub card: u32,
    pub device: u32,
    pub name: String,
    pub playback: bool,
    pub capture: bool,
}

impl PcmEntry {
    /// The device node for capture, e.g. `/dev/snd/pcmC0D2c`.
    pub fn capture_node(&self) -> String {
        format!("/dev/snd/pcmC{}D{}c", self.card, self.device)
    }
}

/// Parses /proc/asound/pcm: `00-02: name : name : playback 1 : capture 1`.
pub fn parse_pcm_list(text: &str) -> Vec<PcmEntry> {
    text.lines()
        .filter_map(|line| {
            let mut parts = line.split(" : ");
            let (id, name) = parts.next()?.split_once(": ")?;
            let (card, device) = id.split_once('-')?;
            let name = name.trim().to_string();
            let rest: Vec<&str> = parts.collect();
            Some(PcmEntry {
                card: card.trim().parse().ok()?,
                device: device.trim().parse().ok()?,
                name,
                playback: rest.iter().any(|p| p.trim().starts_with("playback")),
                capture: rest.iter().any(|p| p.trim().starts_with("capture")),
            })
        })
        .collect()
}

/// The capture nodes to try, most likely microphone first: capture-only devices, then the rest;
/// names that look like HDMI/modem/BT loops go last.
pub fn capture_candidates(entries: &[PcmEntry]) -> Vec<PcmEntry> {
    let bad = |e: &PcmEntry| {
        let n = e.name.to_ascii_lowercase();
        ["hdmi", "bt", "modem", "voice", "loop", "fm"].iter().any(|w| n.contains(w))
    };
    let mut out: Vec<PcmEntry> = entries.iter().filter(|e| e.capture).cloned().collect();
    out.sort_by_key(|e| (bad(e), e.playback, e.card, e.device));
    out
}

/// Capture devices on this machine (reads /proc only; opens nothing).
pub fn discover() -> Vec<PcmEntry> {
    capture_candidates(&parse_pcm_list(&std::fs::read_to_string("/proc/asound/pcm").unwrap_or_default()))
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
    /// peak magnitude of the most recent read, 0..=32768
    level: u16,
    stop: bool,
    error: Option<String>,
}

/// A recording in progress.
pub struct Recorder {
    shared: Arc<Mutex<Shared>>,
    thread: Option<JoinHandle<()>>,
    pub rate: u32,
    pub channels: u32,
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

fn apply(route: &[(&str, &str)]) {
    if route.is_empty() {
        return;
    }
    let Ok(mixer) = Mixer::open(CONTROL_DEVICE) else { return };
    let have: Vec<String> = mixer.list().map(|l| l.into_iter().map(|c| c.name).collect()).unwrap_or_default();
    for &(name, value) in route {
        if have.iter().any(|h| h == name) {
            let _ = mixer.set_text(name, &[value]);
        }
    }
}

impl Recorder {
    /// Routes the microphone, opens the first capture PCM and format that work and starts the
    /// reader thread. Explicit user action only.
    pub fn start() -> io::Result<Recorder> {
        apply(MIC_ROUTE_ON);
        let mut last = io::Error::new(io::ErrorKind::NotFound, "no capture device");
        for entry in discover() {
            let node = entry.capture_node();
            for (rate, channels) in FORMATS {
                match alsa::open_capture(&node, rate, channels) {
                    Ok(cap) => return Ok(Self::spawn(cap, node, rate, channels)),
                    Err(e) => last = e,
                }
            }
        }
        apply(MIC_ROUTE_OFF);
        Err(last)
    }

    fn spawn(cap: alsa::CaptureFd, node: String, rate: u32, channels: u32) -> Recorder {
        let shared = Arc::new(Mutex::new(Shared::default()));
        let worker = shared.clone();
        let thread = crate::bare::sys::spawn_named("capture", move || {
            let mut buf = vec![0i16; cap.period.max(256) as usize * channels as usize];
            loop {
                if worker.lock().unwrap().stop {
                    break;
                }
                match alsa::read_capture(cap.fd, channels, &mut buf) {
                    Ok(0) => std::thread::sleep(Duration::from_millis(5)),
                    Ok(frames) => {
                        let got = &buf[..frames * channels as usize];
                        let mut s = worker.lock().unwrap();
                        s.level = got.iter().map(|v| v.unsigned_abs()).max().unwrap_or(0);
                        s.samples.extend_from_slice(got);
                    }
                    Err(e) => {
                        worker.lock().unwrap().error = Some(e.to_string());
                        break;
                    }
                }
            }
            alsa::close_capture(cap.fd);
        });
        Recorder { shared, thread: Some(thread), rate, channels, node }
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

    /// Stops the stream, releases the mic route and returns what was recorded.
    pub fn stop(mut self) -> Take {
        self.shared.lock().unwrap().stop = true;
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
        apply(MIC_ROUTE_OFF);
        let samples = std::mem::take(&mut self.shared.lock().unwrap().samples);
        Take { rate: self.rate, channels: self.channels, samples }
    }
}

impl Drop for Recorder {
    fn drop(&mut self) {
        // dropped without `stop` (app killed): still release the device
        if let Some(t) = self.thread.take() {
            self.shared.lock().unwrap().stop = true;
            let _ = t.join();
            apply(MIC_ROUTE_OFF);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PCM: &str = "00-00: ALC3246 Analog :  : playback 1 : capture 1\n00-03: HDMI 0 :  : playback 1\n00-02: ALC3246 Alt Analog :  : capture 1\n00-04: Loopback :  : capture 1\n";

    #[test]
    fn proc_pcm_lines_parse() {
        let e = parse_pcm_list(PCM);
        assert_eq!(e.len(), 4);
        assert_eq!((e[0].card, e[0].device, e[0].playback, e[0].capture), (0, 0, true, true));
        assert!(!e[1].capture);
        assert_eq!(e[2].capture_node(), "/dev/snd/pcmC0D2c");
    }

    #[test]
    fn candidates_prefer_capture_only_and_demote_loopbacks() {
        let c = capture_candidates(&parse_pcm_list(PCM));
        let nodes: Vec<String> = c.iter().map(PcmEntry::capture_node).collect();
        assert_eq!(nodes, ["/dev/snd/pcmC0D2c", "/dev/snd/pcmC0D0c", "/dev/snd/pcmC0D4c"]);
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
