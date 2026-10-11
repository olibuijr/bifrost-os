//! Audio assets for the devices (impulse responses, crackle loops, sample zones). Decoding goes through the
//! shell's pure-Rust symphonia wrapper (`springboard::media`), so OGG Vorbis and WAV load the same way the
//! Music app loads them. Samples come out as 16-bit interleaved PCM from that wrapper and are resampled
//! linearly to the render rate.

use springboard::media::audiofile::AudioDecoder;
use springboard::media::codec::Kind;
use springboard::media::symfile::SymFile;
use std::path::Path;

/// Decoded audio, planar, at the requested sample rate.
#[derive(Clone, Debug)]
pub struct Audio {
    pub channels: Vec<Vec<f32>>,
}

/// Decodes an OGG or WAV file; None when the file is missing or cannot be decoded.
pub fn load(path: &Path, sr: f32) -> Option<Audio> {
    let kind = match path.extension()?.to_str()? {
        "ogg" => Kind::Ogg,
        "wav" => Kind::Wav,
        "flac" => Kind::Flac,
        _ => return None,
    };
    let mut file = SymFile::open(kind, path).ok()?;
    let rate = file.rate() as f32;
    let nch = file.channels().max(1);
    let mut interleaved: Vec<f32> = Vec::new();
    let mut buf: Vec<i16> = Vec::new();
    loop {
        match file.next(&mut buf) {
            Ok(true) => interleaved.extend(buf.iter().map(|&s| s as f32 / 32768.0)),
            Ok(false) => break,
            Err(_) => return None,
        }
    }
    if interleaved.is_empty() {
        return None;
    }
    let frames = interleaved.len() / nch;
    let mut planar: Vec<Vec<f32>> = (0..nch).map(|c| (0..frames).map(|i| interleaved[i * nch + c]).collect()).collect();
    if (rate - sr).abs() > 0.5 && rate > 0.0 {
        planar = planar.iter().map(|ch| resample(ch, rate, sr)).collect();
    }
    Some(Audio { channels: planar })
}

/// Linear-interpolation resampler (adequate for impulse responses and loops).
fn resample(x: &[f32], from: f32, to: f32) -> Vec<f32> {
    let n = ((x.len() as f64) * (to as f64) / (from as f64)).floor() as usize;
    (0..n)
        .map(|i| {
            let pos = i as f64 * from as f64 / to as f64;
            let i0 = pos.floor() as usize;
            let frac = (pos - i0 as f64) as f32;
            let a = x.get(i0).copied().unwrap_or(0.0);
            let b = x.get(i0 + 1).copied().unwrap_or(a);
            a + (b - a) * frac
        })
        .collect()
}

/// Stereo copy of a file (a mono file is duplicated to both channels).
pub fn load_stereo(path: &Path, sr: f32) -> Option<Vec<Vec<f32>>> {
    let a = load(path, sr)?;
    let l = a.channels[0].clone();
    let r = a.channels.get(1).cloned().unwrap_or_else(|| l.clone());
    Some(vec![l, r])
}

/// Mono (channel average) of a file.
pub fn load_mono(path: &Path, sr: f32) -> Option<Vec<f32>> {
    let a = load(path, sr)?;
    let n = a.channels[0].len();
    Some((0..n).map(|i| a.channels.iter().map(|c| c[i]).sum::<f32>() / a.channels.len() as f32).collect())
}

type Cache = std::sync::Mutex<std::collections::HashMap<(std::path::PathBuf, u32), std::sync::Arc<Audio>>>;
static CACHE: std::sync::OnceLock<Cache> = std::sync::OnceLock::new();

/// Decoded audio shared by every device that plays this file at this rate (decoded once per process).
pub fn cached(path: &Path, sr: f32) -> Option<std::sync::Arc<Audio>> {
    let cache = CACHE.get_or_init(|| std::sync::Mutex::new(Default::default()));
    let key = (path.to_path_buf(), sr.round() as u32);
    if let Some(hit) = cache.lock().ok()?.get(&key) {
        return Some(hit.clone());
    }
    let audio = std::sync::Arc::new(load(path, sr)?);
    cache.lock().ok()?.insert(key, audio.clone());
    Some(audio)
}

/// The packaged asset root: `DAW_ASSETS`, else `assets/` beside the executable, else the crate's `assets/`
/// (development builds). Holds `plugins/` (instrument and effect data) and `samples/` (drum and sampler WAVs).
pub fn asset_root() -> std::path::PathBuf {
    if let Ok(dir) = std::env::var("DAW_ASSETS") {
        return std::path::PathBuf::from(dir);
    }
    if let Some(beside) = std::env::current_exe().ok().and_then(|e| e.parent().map(|p| p.join("assets"))) {
        if beside.is_dir() {
            return beside;
        }
    }
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets")
}
