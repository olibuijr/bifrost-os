//! The Voice Memos folder: WAV files named after their title.

use springboard::media::capture::wav_duration_ms;
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
}

fn read_memo(path: PathBuf) -> Option<Memo> {
    let md = std::fs::metadata(&path).ok()?;
    let mut head = [0u8; 44];
    std::fs::File::open(&path).ok()?.read_exact(&mut head).ok()?;
    let duration_ms = wav_duration_ms(&head, md.len())?;
    let mtime = md.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs() as i64);
    let title = path.file_stem()?.to_string_lossy().into_owned();
    Some(Memo { path, title, duration_ms, mtime })
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
    }
    Ok(to)
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
    fn clock_formats_minutes_and_seconds() {
        assert_eq!(clock(0), "0:00");
        assert_eq!(clock(65_400), "1:05");
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
