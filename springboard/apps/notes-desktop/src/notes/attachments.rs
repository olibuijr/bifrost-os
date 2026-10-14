//! The attachment browser model: every attachment file of every note, newest note first. A file can be given a
//! caption; captions live in a sidecar file beside the notes, so the file names on disk never change and the note
//! that owns a file stays findable by its id.

use super::folders;
use super::store::Note;
use std::collections::HashMap;
use std::io;
use std::path::Path;

/// Longest caption accepted, in characters.
pub const MAX_CAPTION: usize = 80;
const CAPTIONS_FILE: &str = ".springboard-notes-captions";

/// One attachment as the browser lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    /// The id of the note the file belongs to.
    pub note_id: String,
    /// The folder of that note (None for the root).
    pub folder: Option<String>,
    /// The file name on disk.
    pub file: String,
    /// The caption the user gave the file, if any.
    pub caption: Option<String>,
}

impl Entry {
    /// The name the browser shows: the caption when there is one, else the file name.
    pub fn label(&self) -> &str {
        self.caption.as_deref().unwrap_or(&self.file)
    }
}

/// Every attachment of every note in `notes` (as `folders::load_all` returns them), newest note first, files of
/// one note in name order. `dir` is the notes directory that holds the caption file.
pub fn browse(dir: &Path, notes: &[(Option<String>, Note)]) -> Vec<Entry> {
    let captions = load_captions(dir);
    let mut order: Vec<&(Option<String>, Note)> = notes.iter().collect();
    order.sort_by(|a, b| b.1.modified.cmp(&a.1.modified).then_with(|| a.1.id.cmp(&b.1.id)));
    let mut out = Vec::new();
    for (folder, note) in order {
        let base = folders::folder_dir(dir, folder.as_deref());
        for file in folders::attachments(&base, &note.id) {
            let caption = captions.get(&file).cloned();
            out.push(Entry { note_id: note.id.clone(), folder: folder.clone(), file, caption });
        }
    }
    out
}

/// Checks a caption: trimmed, not empty, at most `MAX_CAPTION` characters, and no control characters (the caption
/// file is one line per file, tab-separated).
pub fn valid_caption(text: &str) -> Option<String> {
    let t = text.trim();
    (!t.is_empty() && t.chars().count() <= MAX_CAPTION && !t.chars().any(char::is_control)).then(|| t.to_string())
}

/// The captions saved in `dir`, keyed by file name. Lines that do not parse are skipped.
pub fn load_captions(dir: &Path) -> HashMap<String, String> {
    std::fs::read_to_string(dir.join(CAPTIONS_FILE))
        .unwrap_or_default()
        .lines()
        .filter_map(|line| {
            let (file, caption) = line.split_once('\t')?;
            Some((file.to_string(), valid_caption(caption)?))
        })
        .collect()
}

/// Gives attachment `file` a caption, or removes its caption when `caption` is blank.
pub fn set_caption(dir: &Path, file: &str, caption: &str) -> io::Result<()> {
    if file.is_empty() || file.contains(['/', '\\', '\t', '\n']) {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "Invalid attachment name"));
    }
    let mut captions = load_captions(dir);
    if caption.trim().is_empty() {
        captions.remove(file);
    } else {
        let valid = valid_caption(caption).ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "Caption is too long or has control characters"))?;
        captions.insert(file.to_string(), valid);
    }
    let mut files: Vec<&String> = captions.keys().collect();
    files.sort();
    let text: String = files.iter().map(|f| format!("{f}\t{}\n", captions[*f])).collect();
    springboard::shell::fsutil::write_atomic(&dir.join(CAPTIONS_FILE), text)
}
