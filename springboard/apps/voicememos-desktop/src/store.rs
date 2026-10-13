//! The Voice Memos folder: WAV files named after their title.

use bifrost_app::controls::recording::Trim;
use bifrost_app::controls::voice_activity::{DetectionOptions, Event, SpeechDetector};
use springboard::media::capture::{pcm16, wav_bytes, wav_duration_ms};
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
    /// the folder the recording sits in; empty at the top level
    pub folder: String,
    /// marked as a favourite (a hidden sidecar next to it)
    pub favorite: bool,
    /// in Recently Deleted
    pub deleted: bool,
}

/// The hidden folder holding Recently Deleted recordings.
pub const DELETED_DIR: &str = ".deleted";
/// How long Recently Deleted keeps a recording before it is removed for good (30 days).
pub const KEEP_DELETED_SECS: i64 = 30 * 24 * 3600;

/// Sample rate and channel count of a 16-bit PCM WAV header; `None` for any other layout.
fn pcm16_format(head: &[u8]) -> Option<(u32, u16)> {
    let h = head.get(..44)?;
    let tag = u16::from_le_bytes([h[20], h[21]]);
    let channels = u16::from_le_bytes([h[22], h[23]]);
    let rate = u32::from_le_bytes([h[24], h[25], h[26], h[27]]);
    let bits = u16::from_le_bytes([h[34], h[35]]);
    (tag == 1 && bits == 16 && channels > 0 && rate > 0).then_some((rate, channels))
}

fn read_memo(path: PathBuf, folder: &str, deleted: bool) -> Option<Memo> {
    let md = std::fs::metadata(&path).ok()?;
    let mut head = [0u8; 44];
    std::fs::File::open(&path).ok()?.read_exact(&mut head).ok()?;
    let duration_ms = wav_duration_ms(&head, md.len())?;
    let mtime = md.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs() as i64);
    let title = path.file_stem()?.to_string_lossy().into_owned();
    let (rate, channels) = pcm16_format(&head).unwrap_or((0, 0));
    let favorite = is_favorite(&path);
    Some(Memo { path, title, duration_ms, mtime, rate, channels, folder: folder.to_string(), favorite, deleted })
}

/// The memos directly in `dir`, newest first (hidden files and non-WAVs skipped).
fn scan_dir(dir: &Path, folder: &str, deleted: bool) -> Vec<Memo> {
    let mut out: Vec<Memo> = std::fs::read_dir(dir)
        .map(|d| {
            d.flatten()
                .filter(|e| {
                    let n = e.file_name().to_string_lossy().to_ascii_lowercase();
                    !n.starts_with('.') && n.ends_with(".wav")
                })
                .filter_map(|e| read_memo(e.path(), folder, deleted))
                .collect()
        })
        .unwrap_or_default();
    sort_newest(&mut out);
    out
}

fn sort_newest(memos: &mut [Memo]) {
    memos.sort_by(|a, b| b.mtime.cmp(&a.mtime).then_with(|| b.title.cmp(&a.title)));
}

/// The recordings in `dir` and in its folders, newest first. Recently Deleted is not included (see `scan_deleted`).
pub fn scan(dir: &Path) -> Vec<Memo> {
    let mut out = scan_dir(dir, "", false);
    for folder in folders(dir) {
        out.extend(scan_dir(&dir.join(&folder), &folder, false));
    }
    sort_newest(&mut out);
    out
}

/// The recordings in Recently Deleted, newest first.
pub fn scan_deleted(dir: &Path) -> Vec<Memo> {
    scan_dir(&dir.join(DELETED_DIR), "", true)
}

/// The folders in `dir`, sorted without regard to case. Hidden directories (Recently Deleted among them) are not folders.
pub fn folders(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .map(|d| {
            d.flatten()
                .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .filter(|n| !n.starts_with('.'))
                .collect()
        })
        .unwrap_or_default();
    names.sort_by_key(|n| n.to_lowercase());
    names
}

/// Creates a folder named `name` (sanitised) in `dir` and returns its name. An empty name, or one taken (without
/// regard to case), is refused.
pub fn create_folder(dir: &Path, name: &str) -> std::io::Result<String> {
    let name = sanitize(name);
    if name.is_empty() {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "empty folder name"));
    }
    if folders(dir).iter().any(|f| f.to_lowercase() == name.to_lowercase()) {
        return Err(std::io::Error::new(std::io::ErrorKind::AlreadyExists, "a folder with that name exists"));
    }
    std::fs::create_dir_all(dir.join(&name))?;
    Ok(name)
}

/// Deletes the folder `name`. Its recordings move up to `dir` first, so nothing is lost.
pub fn delete_folder(dir: &Path, name: &str) -> std::io::Result<()> {
    if !folders(dir).iter().any(|f| f == name) {
        return Err(std::io::Error::new(std::io::ErrorKind::NotFound, "no such folder"));
    }
    let inside = dir.join(name);
    for memo in scan_dir(&inside, name, false) {
        move_to_folder(dir, &memo, "")?;
    }
    // what is left is hidden leftovers (sidecars of files that are gone)
    std::fs::remove_dir_all(&inside)
}

/// Moves a recording into `folder` of `dir` (the top level when empty), with its sidecars. Returns the new path; a
/// recording already there stays where it is.
pub fn move_to_folder(dir: &Path, memo: &Memo, folder: &str) -> std::io::Result<PathBuf> {
    let target = if folder.is_empty() { dir.to_path_buf() } else { dir.join(folder) };
    if !target.is_dir() {
        return Err(std::io::Error::new(std::io::ErrorKind::NotFound, "no such folder"));
    }
    if memo.path.parent() == Some(target.as_path()) {
        return Ok(memo.path.clone());
    }
    let to = free_path(&target, &memo.title, None);
    std::fs::rename(&memo.path, &to)?;
    move_sidecars(&memo.path, &to)?;
    Ok(to)
}

/// Whether `query` matches a recording: every word of it must appear, without regard to case, in one of its `fields`
/// (the title, the notes). An empty query matches everything.
pub fn matches(query: &str, fields: &[&str]) -> bool {
    let haystack: Vec<String> = fields.iter().map(|f| f.to_lowercase()).collect();
    query.to_lowercase().split_whitespace().all(|word| haystack.iter().any(|f| f.contains(word)))
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
        // the notes and the favourite mark follow the recording (a missing sidecar is not an error)
        move_sidecars(&memo.path, &to)?;
    }
    Ok(to)
}

/// The hidden files kept next to a recording, by the suffix after `.<stem>.`: its notes, the favourite mark and (in
/// Recently Deleted) the deletion record.
const SIDECARS: [&str; 3] = ["notes.txt", "favorite", "trashed"];

/// A hidden file next to a recording: `.<stem>.<kind>` (the folder scan skips hidden files).
pub fn sidecar_path(recording: &Path, kind: &str) -> PathBuf {
    let stem = recording.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    recording.with_file_name(format!(".{stem}.{kind}"))
}

/// Moves the sidecars of a recording that moved from `from` to `to`.
fn move_sidecars(from: &Path, to: &Path) -> std::io::Result<()> {
    for kind in SIDECARS {
        let source = sidecar_path(from, kind);
        if source.exists() {
            std::fs::rename(&source, sidecar_path(to, kind))?;
        }
    }
    Ok(())
}

/// The notes sidecar of a recording: a hidden `.<stem>.notes.txt` next to it.
pub fn notes_path(recording: &Path) -> PathBuf {
    sidecar_path(recording, "notes.txt")
}

/// Whether a recording is marked as a favourite.
pub fn is_favorite(recording: &Path) -> bool {
    sidecar_path(recording, "favorite").exists()
}

/// Marks or unmarks a recording as a favourite.
pub fn set_favorite(recording: &Path, on: bool) -> std::io::Result<()> {
    let path = sidecar_path(recording, "favorite");
    if on {
        return springboard::shell::fsutil::write_atomic(&path, b"1");
    }
    match std::fs::remove_file(&path) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

/// Moves a recording to Recently Deleted, with its sidecars, and notes when and from which folder so a recover can put
/// it back. Returns the new path.
pub fn trash(dir: &Path, memo: &Memo, now_unix: i64) -> std::io::Result<PathBuf> {
    let bin = dir.join(DELETED_DIR);
    std::fs::create_dir_all(&bin)?;
    let to = free_path(&bin, &memo.title, None);
    std::fs::rename(&memo.path, &to)?;
    move_sidecars(&memo.path, &to)?;
    springboard::shell::fsutil::write_atomic(&sidecar_path(&to, "trashed"), format!("{now_unix}\n{}", memo.folder).as_bytes())?;
    Ok(to)
}

/// The deletion record of a recording in Recently Deleted: when (Unix seconds) and from which folder.
fn trashed_record(recording: &Path) -> Option<(i64, String)> {
    let text = std::fs::read_to_string(sidecar_path(recording, "trashed")).ok()?;
    let mut lines = text.lines();
    let at = lines.next()?.trim().parse().ok()?;
    Some((at, lines.next().unwrap_or("").to_string()))
}

/// Takes a recording out of Recently Deleted, back into the folder it came from (the top level when that folder is
/// gone). Returns the new path.
pub fn recover(dir: &Path, memo: &Memo) -> std::io::Result<PathBuf> {
    let from = trashed_record(&memo.path).map(|(_, folder)| folder).unwrap_or_default();
    let target = if !from.is_empty() && folders(dir).contains(&from) { dir.join(&from) } else { dir.to_path_buf() };
    let _ = std::fs::remove_file(sidecar_path(&memo.path, "trashed"));
    let to = free_path(&target, &memo.title, None);
    std::fs::rename(&memo.path, &to)?;
    move_sidecars(&memo.path, &to)?;
    Ok(to)
}

/// Removes a recording and its sidecars for good.
pub fn delete_forever(memo: &Memo) -> std::io::Result<()> {
    std::fs::remove_file(&memo.path)?;
    for kind in SIDECARS {
        let _ = std::fs::remove_file(sidecar_path(&memo.path, kind));
    }
    Ok(())
}

/// Removes the recordings that have been in Recently Deleted longer than `KEEP_DELETED_SECS`; returns how many went.
/// Does nothing when `now_unix` is not a real time (0 or less), and keeps a recording that has no deletion record.
pub fn purge_expired(dir: &Path, now_unix: i64) -> usize {
    if now_unix <= 0 {
        return 0;
    }
    scan_deleted(dir)
        .iter()
        .filter(|m| trashed_record(&m.path).is_some_and(|(at, _)| now_unix - at > KEEP_DELETED_SECS))
        .filter(|m| delete_forever(m).is_ok())
        .count()
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

fn invalid(message: &str) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidData, message.to_string())
}

/// A 16-bit PCM WAV as its sample rate, channel count and interleaved samples; `None` for any other file.
fn read_pcm16(path: &Path) -> Option<(u32, u16, Vec<i16>)> {
    let bytes = std::fs::read(path).ok()?;
    let (rate, channels) = pcm16_format(&bytes)?;
    let samples = bytes.get(44..)?.chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]])).collect();
    Some((rate, channels, samples))
}

/// Where part `n` of the take that ends up as `target` is written while it is recorded: a hidden file next to it, so
/// the folder scan does not list it. A take that is paused and resumed is recorded in parts.
pub fn part_path(target: &Path, n: usize) -> PathBuf {
    sidecar_path(target, &format!("part{n}.wav"))
}

/// Joins the parts of a take, in order, into the WAV `out`, then removes the parts. One part is moved as it is. The
/// parts must be 16-bit PCM of one format; nothing is removed when they are not.
pub fn join_parts(parts: &[PathBuf], out: &Path) -> std::io::Result<()> {
    if let [only] = parts {
        return std::fs::rename(only, out);
    }
    let mut format: Option<(u32, u16)> = None;
    let mut samples: Vec<i16> = Vec::new();
    for part in parts {
        let (rate, channels, data) = read_pcm16(part).ok_or_else(|| invalid("a part is not a 16-bit WAV"))?;
        match format {
            None => format = Some((rate, channels)),
            Some(f) if f != (rate, channels) => return Err(invalid("the parts differ in format")),
            Some(_) => {}
        }
        samples.extend(data);
    }
    let (rate, channels) = format.ok_or_else(|| invalid("nothing was recorded"))?;
    springboard::shell::fsutil::write_atomic(out, &wav_bytes(&pcm16(rate, channels as u32), &samples))?;
    for part in parts {
        let _ = std::fs::remove_file(part);
    }
    Ok(())
}

/// `samples` (interleaved, `channels` wide) with the span from `start` to `end` seconds replaced by `insert`.
/// A span past the end is cut to the end, and an end before the start replaces nothing (it inserts at the start).
pub fn splice_samples(samples: &[i16], rate: u32, channels: u16, start: f64, end: f64, insert: &[i16]) -> Vec<i16> {
    let width = channels.max(1) as usize;
    let frames = samples.len() / width;
    let at = |seconds: f64| ((seconds * rate as f64).round().max(0.0) as usize).min(frames) * width;
    let first = at(start);
    let last = at(end).max(first);
    let mut out = Vec::with_capacity(first + insert.len() + frames * width - last);
    out.extend_from_slice(&samples[..first]);
    out.extend_from_slice(insert);
    out.extend_from_slice(&samples[last..frames * width]);
    out
}

/// Writes `memo` with the span from `start` to `end` seconds replaced by the recording `take`, as a new memo named
/// `<title> replaced` (numbered when taken). The original stays. The two must be one format.
pub fn save_replaced(memo: &Memo, start: f64, end: f64, take: &Path) -> std::io::Result<PathBuf> {
    let (rate, channels, original) = read_pcm16(&memo.path).ok_or_else(|| invalid("not a 16-bit WAV"))?;
    let (take_rate, take_channels, new) = read_pcm16(take).ok_or_else(|| invalid("the new take is not a 16-bit WAV"))?;
    if (rate, channels) != (take_rate, take_channels) {
        return Err(invalid("the new take differs in format from the recording"));
    }
    let joined = splice_samples(&original, rate, channels, start, end, &new);
    let dir = memo.path.parent().unwrap_or(Path::new("."));
    let path = free_path(dir, &format!("{} replaced", memo.title), None);
    springboard::shell::fsutil::write_atomic(&path, &wav_bytes(&pcm16(rate, channels as u32), &joined))?;
    Ok(path)
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
    Some(wav_bytes(&pcm16(rate, channels as u32), &samples))
}

/// Splits `memo` at `at` seconds into two new memos, `<title> part 1` and `<title> part 2`. The original is kept.
/// Both parts must be at least the shortest trim long; otherwise nothing is written.
pub fn save_split(memo: &Memo, at: f64) -> std::io::Result<(PathBuf, PathBuf)> {
    let length = memo.duration_ms as f64 / 1000.0;
    let (Some(head), Some(tail)) = (Trim::new(0.0, at, length), Trim::new(at, length, length)) else {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "the split is too close to an end"));
    };
    let first = trimmed_wav(&memo.path, head).ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidData, "not a 16-bit WAV"))?;
    let second = trimmed_wav(&memo.path, tail).ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidData, "not a 16-bit WAV"))?;
    let dir = memo.path.parent().unwrap_or(Path::new("."));
    let a = free_path(dir, &format!("{} part 1", memo.title), None);
    springboard::shell::fsutil::write_atomic(&a, &first)?;
    let b = free_path(dir, &format!("{} part 2", memo.title), None);
    springboard::shell::fsutil::write_atomic(&b, &second)?;
    Ok((a, b))
}

/// The speech in a 16-bit PCM WAV, as `(start, end)` in milliseconds, from the speech detector over 10 ms frames
/// of the mixed-down signal. Empty when the file is not 16-bit PCM or holds no speech.
pub fn speech_spans(path: &Path) -> Vec<(f64, f64)> {
    let Ok(bytes) = std::fs::read(path) else { return Vec::new() };
    let Some((rate, channels)) = pcm16_format(&bytes) else { return Vec::new() };
    let Some(data) = bytes.get(44..) else { return Vec::new() };
    let width = channels.max(1) as usize;
    let samples: Vec<f32> = data.chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]]) as f32 / 32768.0).collect();
    let mono: Vec<f32> = samples.chunks_exact(width).map(|f| f.iter().sum::<f32>() / width as f32).collect();
    let frame = (rate / 100).max(1) as usize;
    let mut detector = SpeechDetector::new(DetectionOptions::default(), rate);
    let mut spans = Vec::new();
    let mut open: Option<f64> = None;
    for event in detector.process(&mono, frame) {
        match event {
            Event::Started(t) => open = Some(t * 1000.0),
            Event::Ended(t) => {
                if let Some(start) = open.take() {
                    spans.push((start, t * 1000.0));
                }
            }
        }
    }
    if let Some(start) = open {
        spans.push((start, mono.len() as f64 * 1000.0 / rate as f64));
    }
    spans
}

/// Where playback jumps to skip silence: the start of the next speech after `pos_ms`, when `pos_ms` is in silence.
/// `None` while it is in speech, or when no speech comes later. `spans` are sorted, as `speech_spans` returns them.
pub fn next_speech_ms(spans: &[(f64, f64)], pos_ms: f64) -> Option<f64> {
    if spans.iter().any(|&(start, end)| pos_ms >= start && pos_ms < end) {
        return None;
    }
    spans.iter().map(|&(start, _)| start).find(|&start| start > pos_ms)
}

/// The whole of a 16-bit PCM WAV with the noise gate applied (see `enhance`), as a new WAV of the same format and
/// length. `None` when the file is not 16-bit PCM.
pub fn enhanced_wav(path: &Path) -> Option<Vec<u8>> {
    let bytes = std::fs::read(path).ok()?;
    let (rate, channels) = pcm16_format(&bytes)?;
    let data = bytes.get(44..)?;
    let mut samples: Vec<i16> = data.chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]])).collect();
    crate::enhance::gate_samples(&mut samples, rate, channels as usize);
    Some(wav_bytes(&pcm16(rate, channels as u32), &samples))
}

/// Writes an enhanced copy of `memo` as a new memo named `<title> enhanced` (numbered when taken). The original is
/// kept as it was.
pub fn save_enhanced(memo: &Memo) -> std::io::Result<PathBuf> {
    let wav = enhanced_wav(&memo.path).ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidData, "not a 16-bit WAV"))?;
    let dir = memo.path.parent().unwrap_or(Path::new("."));
    let path = free_path(dir, &format!("{} enhanced", memo.title), None);
    springboard::shell::fsutil::write_atomic(&path, &wav)?;
    Ok(path)
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
    use springboard::media::capture::{pcm16, wav_bytes};

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
        let wav = wav_bytes(&pcm16(8000, 1), &vec![0; 8000]);
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
        let wav = wav_bytes(&pcm16(8000, 1), &vec![0; 800]);
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

    fn tone(dir: &Path, stamp: &str, samples: usize, unix: i64) -> PathBuf {
        save(dir, stamp, &wav_bytes(&pcm16(8000, 1), &vec![100; samples]), unix).unwrap()
    }

    #[test]
    fn a_favourite_mark_is_hidden_follows_the_rename_and_clears() {
        let dir = temp("fav");
        let path = tone(&dir, "20261013-080000", 800, 1_000);
        assert!(!scan(&dir)[0].favorite);
        set_favorite(&path, true).unwrap();
        assert!(is_favorite(&path));
        assert_eq!(scan(&dir).len(), 1, "the mark is not listed as a recording");
        assert!(scan(&dir)[0].favorite);
        let moved = rename(&scan(&dir)[0], "Idea").unwrap();
        assert!(is_favorite(&moved) && !is_favorite(&path), "the mark follows the recording");
        set_favorite(&moved, false).unwrap();
        assert!(!scan(&dir)[0].favorite);
        assert!(set_favorite(&moved, false).is_ok(), "clearing a missing mark is not an error");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn folders_hold_recordings_and_a_move_takes_the_sidecars_along() {
        let dir = temp("folders");
        let path = tone(&dir, "20261013-080000", 800, 1_000);
        write_notes(&path, "agenda").unwrap();
        set_favorite(&path, true).unwrap();
        assert_eq!(create_folder(&dir, "  Work ").unwrap(), "Work");
        assert!(create_folder(&dir, "work").is_err(), "a name taken, in any case, is refused");
        assert!(create_folder(&dir, " / ").is_err(), "an empty name is refused");
        assert_eq!(create_folder(&dir, "archive").unwrap(), "archive");
        assert_eq!(folders(&dir), vec!["archive".to_string(), "Work".to_string()], "sorted without regard to case");

        let memo = scan(&dir).remove(0);
        assert_eq!(memo.folder, "");
        let moved = move_to_folder(&dir, &memo, "Work").unwrap();
        assert_eq!(moved.parent().unwrap(), dir.join("Work"));
        assert!(!path.exists() && moved.exists());
        assert_eq!(read_notes(&moved), "agenda");
        assert!(is_favorite(&moved));
        let listed = scan(&dir);
        assert_eq!(listed.len(), 1, "the scan finds recordings inside folders");
        assert_eq!(listed[0].folder, "Work");
        assert_eq!(move_to_folder(&dir, &listed[0], "Work").unwrap(), moved, "already there: nothing moves");
        assert!(move_to_folder(&dir, &listed[0], "Nope").is_err());

        let back = move_to_folder(&dir, &listed[0], "").unwrap();
        assert_eq!(back, path, "an empty folder name is the top level");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn deleting_a_folder_moves_its_recordings_up() {
        let dir = temp("folder-delete");
        create_folder(&dir, "Trips").unwrap();
        let path = tone(&dir, "20261013-080000", 800, 1_000);
        let moved = move_to_folder(&dir, &scan(&dir)[0], "Trips").unwrap();
        write_notes(&moved, "packing list").unwrap();
        delete_folder(&dir, "Trips").unwrap();
        assert!(folders(&dir).is_empty() && !dir.join("Trips").exists());
        let listed = scan(&dir);
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].path, path);
        assert_eq!(read_notes(&path), "packing list");
        assert!(delete_folder(&dir, "Trips").is_err(), "a folder that is gone cannot be deleted again");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_deleted_recording_waits_in_recently_deleted_and_can_be_recovered() {
        let dir = temp("trash");
        create_folder(&dir, "Work").unwrap();
        tone(&dir, "20261013-080000", 800, 1_000);
        let memo = move_to_folder(&dir, &scan(&dir)[0], "Work").unwrap();
        write_notes(&memo, "keep me").unwrap();
        let listed = scan(&dir).remove(0);

        let bin = trash(&dir, &listed, 5_000).unwrap();
        assert!(scan(&dir).is_empty(), "a deleted recording leaves the list");
        let deleted = scan_deleted(&dir);
        assert_eq!(deleted.len(), 1);
        assert!(deleted[0].deleted && deleted[0].path == bin);
        assert!(!folders(&dir).contains(&".deleted".to_string()), "Recently Deleted is not a folder");
        assert_eq!(read_notes(&bin), "keep me", "the notes go with it");

        let back = recover(&dir, &deleted[0]).unwrap();
        assert_eq!(back.parent().unwrap(), dir.join("Work"), "it returns to its folder");
        assert!(scan_deleted(&dir).is_empty());
        assert_eq!(read_notes(&back), "keep me");
        assert!(!sidecar_path(&back, "trashed").exists(), "the deletion record is gone");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_recovered_recording_goes_to_the_top_level_when_its_folder_is_gone() {
        let dir = temp("trash-gone");
        create_folder(&dir, "Work").unwrap();
        tone(&dir, "20261013-080000", 800, 1_000);
        move_to_folder(&dir, &scan(&dir)[0], "Work").unwrap();
        trash(&dir, &scan(&dir)[0], 5_000).unwrap();
        delete_folder(&dir, "Work").unwrap();
        let back = recover(&dir, &scan_deleted(&dir)[0]).unwrap();
        assert_eq!(back.parent().unwrap(), dir);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn delete_forever_removes_the_recording_and_its_sidecars() {
        let dir = temp("forever");
        let path = tone(&dir, "20261013-080000", 800, 1_000);
        write_notes(&path, "x").unwrap();
        let bin = trash(&dir, &scan(&dir)[0], 5_000).unwrap();
        delete_forever(&scan_deleted(&dir)[0]).unwrap();
        assert!(!bin.exists() && !notes_path(&bin).exists() && !sidecar_path(&bin, "trashed").exists());
        assert!(scan_deleted(&dir).is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn recently_deleted_is_purged_after_thirty_days() {
        let dir = temp("purge");
        let now = 100 * 24 * 3600;
        tone(&dir, "20261013-080000", 800, 1_000);
        tone(&dir, "20261013-090000", 800, 2_000);
        let (old, recent) = {
            let list = scan(&dir);
            (list[1].clone(), list[0].clone())
        };
        trash(&dir, &old, now - KEEP_DELETED_SECS - 1).unwrap();
        trash(&dir, &recent, now - KEEP_DELETED_SECS + 60).unwrap();
        assert_eq!(purge_expired(&dir, 0), 0, "with no real clock nothing is removed");
        assert_eq!(purge_expired(&dir, now), 1);
        let left = scan_deleted(&dir);
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].title, recent.title, "the newer deletion stays");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn search_needs_every_word_in_the_title_or_the_notes() {
        assert!(matches("", &["anything"]));
        assert!(matches("standup", &["Team Standup", ""]));
        assert!(matches("STANDUP  team", &["Team Standup", ""]), "any case, any spacing");
        assert!(matches("milk", &["Voice Memo", "Buy milk and eggs"]), "the notes are searched");
        assert!(matches("voice milk", &["Voice Memo", "Buy milk and eggs"]), "the words may come from different fields");
        assert!(!matches("bread", &["Voice Memo", "Buy milk and eggs"]));
        assert!(!matches("milk bread", &["Voice Memo", "Buy milk and eggs"]), "every word must be found");
    }

    #[test]
    fn parts_of_a_paused_take_join_in_order_and_one_part_is_just_renamed() {
        let dir = temp("join");
        let target = dir.join("Voice Memo X.wav");
        let (p0, p1) = (part_path(&target, 0), part_path(&target, 1));
        assert_eq!(p1.file_name().unwrap(), ".Voice Memo X.part1.wav");
        std::fs::write(&p0, wav_bytes(&pcm16(8000, 1), &vec![1i16; 4000])).unwrap();
        std::fs::write(&p1, wav_bytes(&pcm16(8000, 1), &vec![2i16; 2000])).unwrap();
        assert!(scan(&dir).is_empty(), "the parts are hidden");
        join_parts(&[p0.clone(), p1.clone()], &target).unwrap();
        assert!(!p0.exists() && !p1.exists(), "the parts are removed once joined");
        let memo = scan(&dir).remove(0);
        assert_eq!(memo.duration_ms, 750);
        let (_, _, samples) = read_pcm16(&target).unwrap();
        assert_eq!((samples[0], samples[3999], samples[4000], samples[5999]), (1, 1, 2, 2));

        let single = part_path(&dir.join("Other.wav"), 0);
        let bytes = wav_bytes(&pcm16(8000, 1), &vec![3i16; 800]);
        std::fs::write(&single, &bytes).unwrap();
        join_parts(&[single.clone()], &dir.join("Other.wav")).unwrap();
        assert_eq!(std::fs::read(dir.join("Other.wav")).unwrap(), bytes, "one part moves unchanged");
        assert!(!single.exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn parts_in_different_formats_or_none_are_refused_and_kept() {
        let dir = temp("join-bad");
        let target = dir.join("T.wav");
        let (p0, p1) = (part_path(&target, 0), part_path(&target, 1));
        std::fs::write(&p0, wav_bytes(&pcm16(8000, 1), &vec![1i16; 800])).unwrap();
        std::fs::write(&p1, wav_bytes(&pcm16(16000, 1), &vec![1i16; 800])).unwrap();
        assert!(join_parts(&[p0.clone(), p1.clone()], &target).is_err());
        assert!(p0.exists() && p1.exists() && !target.exists(), "nothing is lost when the join fails");
        assert!(join_parts(&[], &target).is_err(), "no parts, no recording");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn splicing_replaces_the_span_and_the_length_follows_the_new_take() {
        let original: Vec<i16> = (0..8000).collect();
        let insert = vec![-1i16; 2000];
        // replace 0.25 s to 0.5 s (samples 2000..4000) with 0.25 s of -1
        let out = splice_samples(&original, 8000, 1, 0.25, 0.5, &insert);
        assert_eq!(out.len(), 8000);
        assert_eq!((out[1999], out[2000], out[3999], out[4000]), (1999, -1, -1, 4000));
        // a longer take makes the recording longer
        let longer = splice_samples(&original, 8000, 1, 0.25, 0.5, &vec![0i16; 4000]);
        assert_eq!(longer.len(), 10_000);
        // a span past the end is cut to the end, an end before the start inserts
        assert_eq!(splice_samples(&original, 8000, 1, 0.9, 5.0, &insert).len(), 7200 + 2000);
        assert_eq!(splice_samples(&original, 8000, 1, 0.5, 0.25, &insert).len(), 10_000);
        // stereo keeps frames whole
        let stereo: Vec<i16> = (0..16_000).collect();
        let out = splice_samples(&stereo, 8000, 2, 0.5, 1.0, &[7, 7]);
        assert_eq!(out.len(), 8000 + 2 + 0);
        assert_eq!(&out[8000..], &[7, 7]);
    }

    #[test]
    fn a_replaced_copy_keeps_the_original_and_refuses_another_format() {
        let dir = temp("replace");
        let samples: Vec<i16> = (0..8000).map(|i| i as i16).collect();
        let src = save(&dir, "20261013-100000", &wav_bytes(&pcm16(8000, 1), &samples), 100).unwrap();
        let take = dir.join(".take.wav");
        std::fs::write(&take, wav_bytes(&pcm16(8000, 1), &vec![0i16; 2000])).unwrap();
        let memo = scan(&dir).remove(0);
        let copy = save_replaced(&memo, 0.25, 0.5, &take).unwrap();
        assert_eq!(copy.file_name().unwrap(), "Voice Memo 20261013-100000 replaced.wav");
        assert!(src.exists(), "the original stays");
        let (_, _, back) = read_pcm16(&copy).unwrap();
        assert_eq!((back.len(), back[1999], back[2000], back[4000]), (8000, 1999, 0, 4000));

        let other = dir.join(".other.wav");
        std::fs::write(&other, wav_bytes(&pcm16(16000, 1), &vec![0i16; 100])).unwrap();
        assert!(save_replaced(&memo, 0.25, 0.5, &other).is_err());
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
    fn enhanced_copy_keeps_the_length_and_the_original() {
        let dir = temp("enhance");
        let mut samples = vec![40i16; 8000];
        samples.extend((0..8000).map(|i| (((i as f32) * 0.2).sin() * 12000.0) as i16));
        let path = dir.join("Lecture.wav");
        std::fs::write(&path, wav_bytes(&pcm16(8000, 1), &samples)).unwrap();
        let memo = scan(&dir).into_iter().next().unwrap();
        let out = enhanced_wav(&memo.path).unwrap();
        let back: Vec<i16> = out[44..].chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]])).collect();
        assert_eq!(back.len(), samples.len());
        // The gate starts open so a take that opens on speech is not clipped; the first few ms of hiss pass
        // unchanged and the gate has closed well before sample 200 (25 ms at 8 kHz).
        assert!(back[200..4000].iter().all(|s| s.abs() < 40), "the hiss is turned down");
        let copy = save_enhanced(&memo).unwrap();
        assert_eq!(copy.file_name().unwrap(), "Lecture enhanced.wav");
        assert_eq!(scan(&dir).len(), 2);
        assert_eq!(std::fs::read(&memo.path).unwrap(), wav_bytes(&pcm16(8000, 1), &samples), "the original is untouched");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn trimmed_copy_keeps_the_kept_samples_and_the_original() {
        let dir = temp("trim");
        let samples: Vec<i16> = (0..8000).map(|i| i as i16).collect();
        let src = save(&dir, "20261004-120000", &wav_bytes(&pcm16(8000, 1), &samples), 100).unwrap();
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

    #[test]
    fn split_writes_two_parts_and_keeps_the_original() {
        let dir = temp("split");
        let samples: Vec<i16> = (0..8000).map(|i| i as i16).collect();
        save(&dir, "20261004-120000", &wav_bytes(&pcm16(8000, 1), &samples), 100).unwrap();
        let memo = scan(&dir).remove(0);
        let (a, b) = save_split(&memo, 0.25).unwrap();
        assert!(a.file_name().unwrap().to_string_lossy().ends_with(" part 1.wav"));
        assert!(b.file_name().unwrap().to_string_lossy().ends_with(" part 2.wav"));
        let mut durations: Vec<u64> = scan(&dir).iter().map(|m| m.duration_ms).collect();
        durations.sort();
        assert_eq!(durations, vec![250, 750, 1000], "two parts that add up, and the original");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_split_too_close_to_an_end_writes_nothing() {
        let dir = temp("split-short");
        let samples: Vec<i16> = (0..8000).map(|i| i as i16).collect();
        save(&dir, "20261004-120000", &wav_bytes(&pcm16(8000, 1), &samples), 100).unwrap();
        let memo = scan(&dir).remove(0);
        assert!(save_split(&memo, 0.05).is_err(), "a 0.05 s head is below the 0.1 s minimum");
        assert_eq!(scan(&dir).len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn speech_spans_find_the_loud_half_second() {
        let dir = temp("spans");
        // 0.5 s silence, 0.5 s loud (-10 dB, above the medium threshold), 0.5 s silence at 8 kHz
        let mut samples = vec![0i16; 4000];
        samples.extend(std::iter::repeat(10_000i16).take(4000));
        samples.extend(std::iter::repeat(0i16).take(4000));
        let path = save(&dir, "20261004-120000", &wav_bytes(&pcm16(8000, 1), &samples), 100).unwrap();
        let spans = speech_spans(&path);
        assert_eq!(spans.len(), 1, "{spans:?}");
        assert!((spans[0].0 - 500.0).abs() < 0.5, "starts at 500 ms: {spans:?}");
        assert!((spans[0].1 - 1000.0).abs() < 0.5, "ends at 1000 ms: {spans:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn next_speech_jumps_over_silence_and_waits_inside_speech() {
        let spans = [(500.0, 1000.0), (2000.0, 2500.0)];
        assert_eq!(next_speech_ms(&spans, 0.0), Some(500.0), "leading silence is skipped");
        assert_eq!(next_speech_ms(&spans, 700.0), None, "inside speech plays on");
        assert_eq!(next_speech_ms(&spans, 1000.0), Some(2000.0), "the end is exclusive: a pause is silence");
        assert_eq!(next_speech_ms(&spans, 2600.0), None, "no speech left");
        assert_eq!(next_speech_ms(&[], 0.0), None, "no speech found, nothing to skip to");
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
    use springboard::media::capture::{pcm16, wav_bytes};

    #[test]
    fn peaks_show_the_loud_half_and_stay_in_range() {
        let dir = std::env::temp_dir().join(format!("voicememos-peaks-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("tone.wav");
        let mut samples = vec![0i16; 800];
        samples.extend(vec![32767i16; 800]);
        std::fs::write(&path, wav_bytes(&pcm16(8000, 1), &samples)).unwrap();

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
