//! The Voice Memos folder: WAV files named after their title.

use bifrost_app::controls::recording::Trim;
use springboard::media::capture::{wav_bytes, wav_duration_ms};
use std::io::Read;
use std::path::{Path, PathBuf};

pub const MAX_TITLE: usize = 60;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Memo {
    pub path: PathBuf,
    pub title: String,
    pub duration_ms: u64,
    /// modification time, Unix seconds
    pub mtime: i64,
    /// sample rate in Hz and channel count from the WAV header (0 when the header is not 16-bit PCM)
    pub rate: u32,
    pub channels: u16,
}

/// Sample rate and channel count of a 16-bit PCM WAV header; `None` for any other layout.
fn pcm16_format(head: &[u8]) -> Option<(u32, u16)> {
    let h = head.get(..44)?;
    let tag = u16::from_le_bytes([h[20], h[21]]);
    let channels = u16::from_le_bytes([h[22], h[23]]);
    let rate = u32::from_le_bytes([h[24], h[25], h[26], h[27]]);
    let bits = u16::from_le_bytes([h[34], h[35]]);
    (tag == 1 && bits == 16 && channels > 0 && rate > 0).then_some((rate, channels))
}

fn read_memo(path: PathBuf) -> Option<Memo> {
    let md = std::fs::metadata(&path).ok()?;
    let mut head = [0u8; 44];
    std::fs::File::open(&path).ok()?.read_exact(&mut head).ok()?;
    let duration_ms = wav_duration_ms(&head, md.len())?;
    let mtime = md.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs() as i64);
    let title = path.file_stem()?.to_string_lossy().into_owned();
    let (rate, channels) = pcm16_format(&head).unwrap_or((0, 0));
    Some(Memo { path, title, duration_ms, mtime, rate, channels })
}

/// The memos in `dir`, newest first (hidden files and non-WAVs skipped).
pub fn scan(dir: &Path) -> Vec<Memo> {
    let mut out: Vec<Memo> = std::fs::read_dir(dir)
        .map(|d| {
            d.flatten()
                .filter(|e| {
                    let n = e.file_name().to_string_lossy().to_ascii_lowercase();
                    !n.starts_with('.') && n.ends_with(".wav")
                })
                .filter_map(|e| read_memo(e.path()))
                .collect()
        })
        .unwrap_or_default();
    out.sort_by(|a, b| b.mtime.cmp(&a.mtime).then_with(|| b.title.cmp(&a.title)));
    out
}

/// A title safe as a file stem: no path separators or control characters, trimmed, capped.
pub fn sanitize(title: &str) -> String {
    let cleaned: String = title.chars().filter(|c| !c.is_control() && !matches!(c, '/' | '\\' | ':')).collect();
    cleaned.trim().trim_start_matches('.').trim().chars().take(MAX_TITLE).collect()
}

/// `dir/<stem>.wav`, numbered (`stem 2`) when taken, ignoring `own` (the file being renamed).
pub fn free_path(dir: &Path, stem: &str, own: Option<&Path>) -> PathBuf {
    let mut path = dir.join(format!("{stem}.wav"));
    let mut n = 2;
    while path.exists() && Some(path.as_path()) != own {
        path = dir.join(format!("{stem} {n}.wav"));
        n += 1;
    }
    path
}

/// Renames a memo; the title is sanitised and an empty one is refused.
pub fn rename(memo: &Memo, title: &str) -> std::io::Result<PathBuf> {
    let stem = sanitize(title);
    if stem.is_empty() {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "empty title"));
    }
    let dir = memo.path.parent().unwrap_or(Path::new("."));
    let to = free_path(dir, &stem, Some(&memo.path));
    if to != memo.path {
        std::fs::rename(&memo.path, &to)?;
        // the notes follow the recording (a missing notes file is not an error)
        let (from_notes, to_notes) = (notes_path(&memo.path), notes_path(&to));
        if from_notes.exists() {
            std::fs::rename(&from_notes, &to_notes)?;
        }
    }
    Ok(to)
}

/// The notes sidecar of a recording: a hidden `.<stem>.notes.txt` next to it (the folder scan skips hidden files).
pub fn notes_path(recording: &Path) -> PathBuf {
    let stem = recording.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    recording.with_file_name(format!(".{stem}.notes.txt"))
}

/// The notes of a recording; empty when it has none or the file cannot be read.
pub fn read_notes(recording: &Path) -> String {
    std::fs::read_to_string(notes_path(recording)).unwrap_or_default()
}

/// Saves the notes of a recording. Empty notes remove the sidecar.
pub fn write_notes(recording: &Path, text: &str) -> std::io::Result<()> {
    let path = notes_path(recording);
    if text.is_empty() {
        return match std::fs::remove_file(&path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        };
    }
    springboard::shell::fsutil::write_atomic(&path, text.as_bytes())
}

/// Copies a recording into `dir` as `<title>.wav` (numbered when taken). The original stays where it is.
pub fn export_copy(memo: &Memo, dir: &Path) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let path = free_path(dir, &memo.title, None);
    std::fs::copy(&memo.path, &path)?;
    Ok(path)
}

#[cfg(test)]
/// Writes a new take as `Voice Memo <stamp>.wav` dated `unix`.
pub fn save(dir: &Path, stamp: &str, wav: &[u8], unix: i64) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let path = free_path(dir, &format!("Voice Memo {stamp}"), None);
    springboard::shell::fsutil::write_atomic(&path, wav)?;
    if let Ok(f) = std::fs::File::options().write(true).open(&path) {
        let _ = f.set_modified(std::time::UNIX_EPOCH + std::time::Duration::from_secs(unix.max(0) as u64));
    }
    Ok(path)
}

/// `m:ss` for a duration.
pub fn clock(ms: u64) -> String {
    let s = ms / 1000;
    format!("{}:{:02}", s / 60, s % 60)
}

/// Recording metadata for the detail subtitle: `8 kHz mono`, `44.1 kHz stereo`, `48 kHz 6 channels`.
/// Empty when the format is unknown (rate or channels 0).
pub fn format_title(rate: u32, channels: u16) -> String {
    if rate == 0 || channels == 0 {
        return String::new();
    }
    let khz = if rate % 1000 == 0 { format!("{} kHz", rate / 1000) } else { format!("{:.1} kHz", rate as f64 / 1000.0) };
    let layout = match channels {
        1 => "mono".to_string(),
        2 => "stereo".to_string(),
        n => format!("{n} channels"),
    };
    format!("{khz} {layout}")
}

/// The kept part of a 16-bit PCM WAV as a new WAV (`trim` in seconds). `None` when the file is not 16-bit PCM.
pub fn trimmed_wav(path: &Path, trim: Trim) -> Option<Vec<u8>> {
    let bytes = std::fs::read(path).ok()?;
    let (rate, channels) = pcm16_format(&bytes)?;
    let frame = channels as usize * 2;
    let data = bytes.get(44..)?;
    let frames = data.len() / frame;
    let first = ((trim.start() * rate as f64).round() as usize).min(frames);
    let last = ((trim.end() * rate as f64).round() as usize).min(frames);
    let samples: Vec<i16> = data[first * frame..last * frame].chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]])).collect();
    Some(wav_bytes(rate, channels as u32, &samples))
}

/// Writes the trimmed part of `memo` as a new memo named `<title> trimmed` (numbered when taken).
/// The original recording is kept as it was.
pub fn save_trim(memo: &Memo, trim: Trim) -> std::io::Result<PathBuf> {
    let wav = trimmed_wav(&memo.path, trim).ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidData, "not a 16-bit WAV"))?;
    let dir = memo.path.parent().unwrap_or(Path::new("."));
    let path = free_path(dir, &format!("{} trimmed", memo.title), None);
    springboard::shell::fsutil::write_atomic(&path, &wav)?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use springboard::media::capture::wav_bytes;

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("voicememos-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn titles_are_sanitised() {
        assert_eq!(sanitize("  ../etc/passwd "), "etcpasswd");
        assert_eq!(sanitize("a:b\\c\n"), "abc");
        assert_eq!(sanitize(&"x".repeat(100)).len(), MAX_TITLE);
        assert_eq!(sanitize("///"), "");
    }

    #[test]
    fn save_list_rename_and_delete_round_trip() {
        let dir = temp("rt");
        let wav = wav_bytes(8000, 1, &vec![0; 8000]);
        let a = save(&dir, "20261004-120000", &wav, 1_000).unwrap();
        let b = save(&dir, "20261004-120000", &wav, 2_000).unwrap();
        assert_eq!(b.file_name().unwrap(), "Voice Memo 20261004-120000 2.wav");
        std::fs::write(dir.join("notes.txt"), b"x").unwrap();
        let list = scan(&dir);
        assert_eq!(list.len(), 2, "only WAVs");
        assert_eq!(list[0].path, b, "newest first");
        assert_eq!(list[0].duration_ms, 1000);
        let moved = rename(&list[1], "Groceries").unwrap();
        assert_eq!(moved.file_name().unwrap(), "Groceries.wav");
        assert!(!a.exists() && moved.exists());
        assert!(rename(&scan(&dir)[0], "  ").is_err());
        // renaming to the same title keeps the file
        let again = rename(&scan(&dir).into_iter().find(|m| m.title == "Groceries").unwrap(), "Groceries").unwrap();
        assert_eq!(again, moved);
        std::fs::remove_file(&moved).unwrap();
        assert_eq!(scan(&dir).len(), 1);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn notes_sidecar_is_hidden_follows_the_rename_and_empty_notes_remove_it() {
        let dir = temp("notes");
        let wav = wav_bytes(8000, 1, &vec![0; 800]);
        let a = save(&dir, "20261012-090000", &wav, 1_000).unwrap();
        assert_eq!(notes_path(&a).file_name().unwrap(), ".Voice Memo 20261012-090000.notes.txt");
        assert_eq!(read_notes(&a), "", "no notes yet");
        write_notes(&a, "Milk, eggs\nBread").unwrap();
        assert_eq!(read_notes(&a), "Milk, eggs\nBread");
        // hidden: the folder scan still lists one recording
        assert_eq!(scan(&dir).len(), 1);
        let moved = rename(&scan(&dir)[0], "Shopping").unwrap();
        assert_eq!(read_notes(&moved), "Milk, eggs\nBread", "the notes follow the recording");
        assert!(!notes_path(&a).exists());
        write_notes(&moved, "").unwrap();
        assert!(!notes_path(&moved).exists(), "empty notes remove the sidecar");
        assert!(write_notes(&moved, "").is_ok(), "removing missing notes is not an error");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn export_copies_the_recording_and_keeps_the_original() {
        let dir = temp("export");
        let out = dir.join("Downloads");
        let wav = wav_bytes(8000, 1, &vec![0; 800]);
        let a = save(&dir, "20261012-090500", &wav, 1_000).unwrap();
        let memo = scan(&dir).remove(0);
        let first = export_copy(&memo, &out).unwrap();
        assert_eq!(first.file_name().unwrap(), "Voice Memo 20261012-090500.wav");
        assert_eq!(std::fs::read(&first).unwrap(), wav, "the copy holds the same bytes");
        let second = export_copy(&memo, &out).unwrap();
        assert_eq!(second.file_name().unwrap(), "Voice Memo 20261012-090500 2.wav", "a second export is numbered");
        assert!(a.exists(), "the original stays");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn clock_formats_minutes_and_seconds() {
        assert_eq!(clock(0), "0:00");
        assert_eq!(clock(65_400), "1:05");
    }

    #[test]
    fn metadata_line_names_rate_and_channels() {
        assert_eq!(format_title(8000, 1), "8 kHz mono");
        assert_eq!(format_title(44100, 2), "44.1 kHz stereo");
        assert_eq!(format_title(48000, 6), "48 kHz 6 channels");
        assert_eq!(format_title(0, 1), "", "unknown format is left out");
    }

    #[test]
    fn trimmed_copy_keeps_the_kept_samples_and_the_original() {
        let dir = temp("trim");
        let samples: Vec<i16> = (0..8000).map(|i| i as i16).collect();
        let src = save(&dir, "20261004-120000", &wav_bytes(8000, 1, &samples), 100).unwrap();
        let memo = scan(&dir).remove(0);
        assert_eq!((memo.rate, memo.channels), (8000, 1));

        let trim = Trim::new(0.25, 0.5, 1.0).unwrap();
        let out = trimmed_wav(&memo.path, trim).unwrap();
        assert_eq!(wav_duration_ms(&out[..44], out.len() as u64), Some(250));
        assert_eq!(i16::from_le_bytes([out[44], out[45]]), 2000, "starts at the start handle");

        let copy = save_trim(&memo, trim).unwrap();
        assert_eq!(copy.file_name().unwrap(), "Voice Memo 20261004-120000 trimmed.wav");
        assert!(src.exists() && copy.exists(), "the original stays");
        let list = scan(&dir);
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].duration_ms, 250, "the trimmed copy is newest");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// `n` waveform bars for a memo: the peak of each equal slice of 16-bit samples, 0..=1
/// (square-rooted so quiet speech is still visible). Empty when the file cannot be read.
pub fn peaks(path: &Path, n: usize) -> Vec<f32> {
    let Ok(bytes) = std::fs::read(path) else { return Vec::new() };
    let data = bytes.get(44..).unwrap_or(&[]);
    let count = data.len() / 2;
    if n == 0 || count == 0 {
        return Vec::new();
    }
    (0..n)
        .map(|i| {
            let (a, b) = (i * count / n, ((i + 1) * count / n).max(i * count / n + 1).min(count));
            let peak = (a..b).map(|k| i16::from_le_bytes([data[2 * k], data[2 * k + 1]]).unsigned_abs()).max().unwrap_or(0);
            (peak as f32 / 32768.0).sqrt().min(1.0)
        })
        .collect()
}

#[cfg(test)]
mod peak_tests {
    use super::*;
    use springboard::media::capture::wav_bytes;

    #[test]
    fn peaks_show_the_loud_half_and_stay_in_range() {
        let dir = std::env::temp_dir().join(format!("voicememos-peaks-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("tone.wav");
        let mut samples = vec![0i16; 800];
        samples.extend(vec![32767i16; 800]);
        std::fs::write(&path, wav_bytes(8000, 1, &samples)).unwrap();

        let p = peaks(&path, 4);
        assert_eq!(p.len(), 4);
        assert_eq!(p[0], 0.0, "silent first quarter");
        assert!(p[3] > 0.99 && p[3] <= 1.0, "{p:?}");
        assert!(p.iter().all(|v| (0.0..=1.0).contains(v)));
        assert!(peaks(&dir.join("missing.wav"), 4).is_empty());
        assert!(peaks(&path, 0).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
