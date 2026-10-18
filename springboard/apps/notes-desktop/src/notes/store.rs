//! Notes on disk: one plain-text `<id>.md` per note (its first line is the
//! title) under the notes directory, written atomically; plus the pure
//! helpers the list is built from: sorting, search, titles, snippets, date
//! labels and the debounced autosave timer.

use springboard::shell::clock::tz::civil;
use std::collections::HashSet;
use std::io;
use std::path::{Path, PathBuf};

/// A note as the list and editor see it.
#[derive(Clone, Debug, PartialEq)]
pub struct Note {
    pub id: String,
    pub text: String,
    /// last change, Unix seconds
    pub modified: i64,
}

impl Note {
    /// The first non-blank line, or the placeholder for an empty note.
    pub fn title(&self) -> &str {
        let line = self.text.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("New Note");
        // a Markdown heading marker is not part of the name
        let bare = line.trim_start_matches('#').trim_start();
        if bare.is_empty() { line } else { bare }
    }

    /// The first non-blank line after the title.
    pub fn snippet(&self) -> &str {
        self.text.lines().map(str::trim).filter(|l| !l.is_empty()).nth(1).unwrap_or("No additional text")
    }

    /// `(done, total)` checklist items in the note.
    pub fn checklist(&self) -> (usize, usize) {
        let (mut done, mut total) = (0, 0);
        for l in self.text.lines() {
            let l = l.trim_start();
            if l.starts_with("- [ ] ") { total += 1; } else if l.starts_with("- [x] ") || l.starts_with("- [X] ") { total += 1; done += 1; }
        }
        (done, total)
    }

    /// Row snippet: checklist progress when the note has a checklist.
    pub fn row_snippet(&self) -> String {
        match self.checklist() {
            (_, 0) => self.snippet().to_string(),
            (d, t) => format!("{d} of {t} done"),
        }
    }

    /// Case-insensitive substring match on the whole text.
    pub fn matches(&self, query: &str) -> bool {
        let q = query.trim().to_lowercase();
        q.is_empty() || self.text.to_lowercase().contains(&q)
    }
}

/// Newest first; ties keep the id order so the list never jitters.
pub fn sort(notes: &mut [Note]) {
    notes.sort_by(|a, b| b.modified.cmp(&a.modified).then_with(|| b.id.cmp(&a.id)));
}

/// Pinned notes first, retaining newest-first ordering within each section.
/// Pin state lives separately so a Markdown edit never changes it.
pub fn sort_with_pins(notes: &mut [Note], pinned: &HashSet<String>) {
    notes.sort_by(|a, b| pinned.contains(&b.id).cmp(&pinned.contains(&a.id))
        .then_with(|| b.modified.cmp(&a.modified)).then_with(|| b.id.cmp(&a.id)));
}

/// How the list is ordered inside each section.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum SortKey {
    #[default]
    DateEdited,
    DateCreated,
    Title,
}

impl SortKey {
    pub const ALL: [SortKey; 3] = [SortKey::DateEdited, SortKey::DateCreated, SortKey::Title];

    pub fn label(self) -> &'static str {
        match self {
            SortKey::DateEdited => "Date Edited",
            SortKey::DateCreated => "Date Created",
            SortKey::Title => "Title",
        }
    }

    fn name(self) -> &'static str {
        match self {
            SortKey::DateEdited => "edited",
            SortKey::DateCreated => "created",
            SortKey::Title => "title",
        }
    }

    fn parse(s: &str) -> Option<SortKey> {
        Self::ALL.into_iter().find(|k| k.name() == s)
    }
}

/// Creation time in Unix seconds: ids are the creation time in ms; imported
/// notes with other names fall back to their last change.
pub fn created(note: &Note) -> i64 {
    match note.id.parse::<i64>() {
        Ok(ms) if ms >= 1_000_000_000_000 => ms / 1000,
        _ => note.modified,
    }
}

/// Orders notes by `key` (newest or A-Z), pinned ones first.
pub fn sort_by_key(notes: &mut [Note], key: SortKey, pinned: &HashSet<String>) {
    sort_ordered(notes, key, false, pinned);
}

/// Orders notes by `key` inside the pinned and the other section. The dates run newest first, or oldest first
/// with `oldest_first`; the title always runs A to Z.
pub fn sort_ordered(notes: &mut [Note], key: SortKey, oldest_first: bool, pinned: &HashSet<String>) {
    notes.sort_by(|a, b| {
        let order = match key {
            SortKey::DateEdited => b.modified.cmp(&a.modified),
            SortKey::DateCreated => created(b).cmp(&created(a)),
            SortKey::Title => a.title().to_lowercase().cmp(&b.title().to_lowercase()),
        };
        let order = if oldest_first && key != SortKey::Title { order.reverse() } else { order };
        pinned.contains(&b.id).cmp(&pinned.contains(&a.id)).then(order).then_with(|| b.id.cmp(&a.id))
    });
}

/// The Group by Date section a note's date falls in, counted in local calendar days before `now`: Today, Previous
/// 7 Days, Previous 30 Days, then the month ("October", with the year when it is not the current one).
pub fn date_group(unix: i64, now: i64, offset: i32) -> String {
    let day = |t: i64| (t + offset as i64).div_euclid(86_400);
    match day(now) - day(unix) {
        i64::MIN..=0 => "Today".to_string(),
        1..=7 => "Previous 7 Days".to_string(),
        8..=30 => "Previous 30 Days".to_string(),
        _ => {
            let (then, today) = (civil(unix, offset), civil(now, offset));
            let month = MONTHS[then.month as usize - 1];
            if then.year == today.year { month.to_string() } else { format!("{month} {}", then.year) }
        }
    }
}

/// The tags written in `text`, lowercase, each once, in order of appearance. A tag is a `#` that starts a word,
/// followed by letters, digits, `_` or `-` (not only digits): `#groceries`, not `# Heading`, `C#` or a link anchor.
pub fn tags(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut out: Vec<String> = vec![];
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '#' && (i == 0 || chars[i - 1].is_whitespace()) {
            let mut j = i + 1;
            while j < chars.len() && (chars[j].is_alphanumeric() || chars[j] == '_' || chars[j] == '-') {
                j += 1;
            }
            let word: String = chars[i + 1..j].iter().collect::<String>().trim_end_matches('-').to_lowercase();
            if !word.is_empty() && !word.chars().all(|c| c.is_ascii_digit()) && !out.contains(&word) {
                out.push(word);
            }
            i = j.max(i + 1);
        } else {
            i += 1;
        }
    }
    out
}

/// Whether `text` carries `tag` (any case, without the `#`).
pub fn has_tag(text: &str, tag: &str) -> bool {
    let tag = tag.trim_start_matches('#').to_lowercase();
    tags(text).iter().any(|t| *t == tag)
}

/// Char ranges of every case-insensitive occurrence of `query` in `text`.
pub fn find_all(text: &str, query: &str) -> Vec<std::ops::Range<usize>> {
    let fold = |c: char| c.to_lowercase().next().unwrap_or(c);
    let hay: Vec<char> = text.chars().map(fold).collect();
    let needle: Vec<char> = query.trim().chars().map(fold).collect();
    if needle.is_empty() || needle.len() > hay.len() {
        return vec![];
    }
    let mut out = vec![];
    let mut i = 0;
    while i + needle.len() <= hay.len() {
        if hay[i..i + needle.len()] == needle[..] {
            out.push(i..i + needle.len());
            i += needle.len();
        } else {
            i += 1;
        }
    }
    out
}

/// Relevance of a note for `query`: a title hit counts most, then body hits.
/// Zero means no match.
pub fn score(note: &Note, query: &str) -> u32 {
    let hits = find_all(&note.text, query).len() as u32;
    if hits == 0 {
        return 0;
    }
    let title = note.title();
    let in_title = find_all(title, query);
    let mut s = 1 + hits.min(10);
    if !in_title.is_empty() {
        s += 50;
        if in_title[0].start == 0 {
            s += 30;
        }
    }
    s
}

/// Indices of matching notes: pinned first, then best score, then list order.
pub fn search_ranked(notes: &[Note], query: &str, pinned: &HashSet<String>) -> Vec<usize> {
    if query.trim().is_empty() {
        let mut all: Vec<usize> = (0..notes.len()).collect();
        all.sort_by_key(|&i| !pinned.contains(&notes[i].id));
        return all;
    }
    let mut hits: Vec<(usize, u32)> = notes.iter().enumerate().map(|(i, n)| (i, score(n, query))).filter(|&(_, s)| s > 0).collect();
    hits.sort_by(|a, b| pinned.contains(&notes[b.0].id).cmp(&pinned.contains(&notes[a.0].id)).then(b.1.cmp(&a.1)).then(a.0.cmp(&b.0)));
    hits.into_iter().map(|(i, _)| i).collect()
}

/// Which match is current after stepping `delta` through `count` matches (wraps).
pub fn step_match(current: usize, count: usize, delta: i32) -> usize {
    if count == 0 {
        return 0;
    }
    (current as i64 + delta as i64).rem_euclid(count as i64) as usize
}

/// List settings persisted beside the notes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Prefs {
    pub sort: SortKey,
    /// move a checklist item to the bottom of its list when ticked
    pub checked_to_bottom: bool,
    /// the date sorts run oldest first (the title always runs A to Z)
    pub oldest_first: bool,
    /// Math Results: a line ending with `=` after a calculation gets its answer as it is typed
    pub math_results: bool,
    /// New Quick Note reopens the last quick note instead of starting another
    pub resume_quick: bool,
    /// the list is cut into Today, Previous 7 Days, Previous 30 Days and month sections (date sorts only)
    pub group_by_date: bool,
    /// the note list shows rows or a gallery of cards (View > as List / as Gallery)
    pub layout: super::gallery::Layout,
}

impl Default for Prefs {
    fn default() -> Self {
        Prefs { sort: SortKey::DateEdited, checked_to_bottom: false, oldest_first: false, math_results: false, resume_quick: false, group_by_date: false, layout: super::gallery::Layout::List }
    }
}

const PREFS_FILE: &str = ".springboard-notes-prefs";

pub fn load_prefs(dir: &Path) -> Prefs {
    let mut p = Prefs::default();
    for line in std::fs::read_to_string(dir.join(PREFS_FILE)).unwrap_or_default().lines() {
        match line.split_once('=') {
            Some(("sort", v)) => p.sort = SortKey::parse(v.trim()).unwrap_or_default(),
            Some(("checked_to_bottom", v)) => p.checked_to_bottom = v.trim() == "1",
            Some(("oldest_first", v)) => p.oldest_first = v.trim() == "1",
            Some(("math_results", v)) => p.math_results = v.trim() == "1",
            Some(("resume_quick", v)) => p.resume_quick = v.trim() == "1",
            Some(("group_by_date", v)) => p.group_by_date = v.trim() == "1",
            Some(("layout", v)) => p.layout = super::gallery::Layout::parse(v.trim()).unwrap_or(p.layout),
            _ => {}
        }
    }
    p
}

pub fn save_prefs(dir: &Path, p: Prefs) -> io::Result<()> {
    let text = format!(
        "sort={}\nchecked_to_bottom={}\noldest_first={}\nmath_results={}\nresume_quick={}\ngroup_by_date={}\nlayout={}\n",
        p.sort.name(),
        p.checked_to_bottom as u8,
        p.oldest_first as u8,
        p.math_results as u8,
        p.resume_quick as u8,
        p.group_by_date as u8,
        p.layout.name()
    );
    springboard::shell::fsutil::write_atomic(&dir.join(PREFS_FILE), text)
}

const QUICK_FILE: &str = ".springboard-notes-quick";

/// The id of the note New Quick Note made last, if one was recorded.
pub fn load_quick(dir: &Path) -> Option<String> {
    let id = std::fs::read_to_string(dir.join(QUICK_FILE)).ok()?;
    Some(id.trim().to_string()).filter(|id| !id.is_empty())
}

/// Records the note New Quick Note made, so "resume" can find it again.
pub fn save_quick(dir: &Path, id: &str) -> io::Result<()> {
    springboard::shell::fsutil::write_atomic(&dir.join(QUICK_FILE), id)
}

/// Writes the note as `<title>.md` under `<dir>/.export` for sharing and
/// returns the file; the name is the sanitised title, the content the
/// note's Markdown (attachments are not bundled).
pub fn export_md(dir: &Path, note: &Note) -> io::Result<PathBuf> {
    export_file(dir, note, "md", note.text.as_bytes())
}

/// Writes `bytes` as `<title>.<ext>` under `<dir>/.export` and returns the
/// file. The name is the note's sanitised title.
pub fn export_file(dir: &Path, note: &Note, ext: &str, bytes: &[u8]) -> io::Result<PathBuf> {
    let name: String = note.title().chars().filter(|c| !c.is_control() && !"/\\:*?\"<>|".contains(*c)).take(40).collect();
    let name = name.trim().trim_start_matches(['.', '#', ' ']).trim().to_string();
    let name = if name.is_empty() { "Note".to_string() } else { name };
    let out = dir.join(".export");
    std::fs::create_dir_all(&out)?;
    let path = out.join(format!("{name}.{ext}"));
    std::fs::write(&path, bytes)?;
    Ok(path)
}

const PIN_DIR: &str = ".springboard-pins-v1";
const PIN_HEADER: &str = "springboard-note-pinned-v1\n";

fn pin_path(dir: &Path, id: &str) -> PathBuf {
    // Hex-encoded UTF-8 handles every existing filename without path traversal,
    // including notes imported with whitespace or non-ASCII names.
    let encoded: String = id.as_bytes().iter().map(|byte| format!("{byte:02x}")).collect();
    // Split long names so the encoded path cannot exceed a filesystem's
    // per-component filename limit even for a 255-byte imported note name.
    let mut path = dir.join(PIN_DIR);
    let chunks: Vec<_> = encoded.as_bytes().chunks(100).collect();
    for (i, chunk) in chunks.iter().enumerate() {
        let part = std::str::from_utf8(chunk).unwrap();
        path.push(if i + 1 == chunks.len() { format!("{part}.pin") } else { part.to_string() });
    }
    path
}

/// Reads only recognized pin markers for the notes currently on disk.
/// Unknown files are left untouched; legacy directories need no migration.
pub fn load_pins(dir: &Path, notes: &[Note]) -> HashSet<String> {
    notes.iter().filter(|note| std::fs::read(pin_path(dir, &note.id)).ok()
        .is_some_and(|bytes| bytes == PIN_HEADER.as_bytes()))
        .map(|note| note.id.clone()).collect()
}

/// Atomically persists just this note's pin state without writing its text.
/// An unrecognized existing marker is never overwritten or removed.
pub fn set_pinned(dir: &Path, id: &str, pinned: bool) -> io::Result<()> {
    let marker = pin_path(dir, id);
    match std::fs::read(&marker) {
        Ok(bytes) if bytes != PIN_HEADER.as_bytes() => return Err(io::Error::new(io::ErrorKind::InvalidData, "Unknown note pin marker")),
        Ok(_) => {}
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            if !pinned { return Ok(()); }
        }
        Err(e) => return Err(e),
    }
    if pinned {
        springboard::shell::fsutil::write_atomic(&marker, PIN_HEADER)
    } else {
        match std::fs::remove_file(marker) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            result => result,
        }
    }
}

/// Indices of the notes matching `query`, in list order.
pub fn search(notes: &[Note], query: &str) -> Vec<usize> {
    notes.iter().enumerate().filter(|(_, n)| n.matches(query)).map(|(i, _)| i).collect()
}

const DAYS: [&str; 7] = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];
const MONTHS: [&str; 12] = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];

/// The Recently Deleted caption: when the note was deleted and how many days
/// it has before it is purged for good.
pub fn trash_caption(deleted: i64, now: i64) -> String {
    let elapsed = (now - deleted).max(0);
    let when = match elapsed / 86_400 {
        0 => "today".to_string(),
        1 => "yesterday".to_string(),
        n => format!("{n} days ago"),
    };
    let left = super::folders::PURGE_SECS - elapsed;
    let days = (left + 86_399) / 86_400;
    match days {
        ..=0 => format!("Deleted {when} · purging soon"),
        1 => format!("Deleted {when} · 1 day left"),
        n => format!("Deleted {when} · {n} days left"),
    }
}

/// The list's date column: the time today, the weekday within the last
/// week, otherwise the date.
pub fn list_date(modified: i64, now: i64, offset: i32) -> String {
    let (m, n) = (civil(modified, offset), civil(now, offset));
    let same_day = (m.year, m.month, m.day) == (n.year, n.month, n.day);
    if same_day {
        format!("{:02}:{:02}", m.hour, m.min)
    } else if now - modified < 6 * 86_400 && modified <= now {
        DAYS[m.wday as usize].to_string()
    } else {
        format!("{:02}/{:02}/{}", m.day, m.month, m.year)
    }
}

/// The editor's heading: "4 October 2026 at 00:23".
pub fn long_date(unix: i64, offset: i32) -> String {
    let c = civil(unix, offset);
    format!("{} {} {} at {:02}:{:02}", c.day, MONTHS[c.month as usize - 1], c.year, c.hour, c.min)
}

/// Loads every `*.md` in `dir`, newest first; a missing directory is empty.
pub fn load(dir: &Path) -> Vec<Note> {
    let Ok(entries) = std::fs::read_dir(dir) else { return vec![] };
    let mut notes: Vec<Note> = entries
        .flatten()
        .filter_map(|e| {
            let path = e.path();
            let id = path.file_stem()?.to_str()?.to_string();
            if path.extension()?.to_str()? != "md" {
                return None;
            }
            let text = std::fs::read_to_string(&path).ok()?;
            let modified = e.metadata().and_then(|m| m.modified()).ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs() as i64);
            Some(Note { id, text, modified })
        })
        .collect();
    sort(&mut notes);
    notes
}

fn path(dir: &Path, id: &str) -> PathBuf {
    dir.join(format!("{id}.md"))
}

/// Writes the note's text through a synced temporary file and a rename.
pub fn save(dir: &Path, note: &Note) -> std::io::Result<()> {
    springboard::shell::fsutil::write_atomic(&path(dir, &note.id), &note.text)
}

pub fn delete(dir: &Path, id: &str) {
    let _ = std::fs::remove_file(path(dir, id));
    if let Err(error) = set_pinned(dir, id, false) {
        eprintln!("springboard: notes: removing pin marker: {error}");
    }
}

/// A fresh id from the creation time; `taken` avoids a clash within the
/// same millisecond.
pub fn new_id(unix_ms: i64, taken: &[Note]) -> String {
    let mut id = unix_ms.max(0);
    while taken.iter().any(|n| n.id == id.to_string()) {
        id += 1;
    }
    id.to_string()
}

/// Debounced write: every change restarts a 500 ms timer and the save
/// happens when it runs out, so fast typing costs one write.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Autosave {
    due: Option<f64>,
}

pub const DEBOUNCE_MS: f64 = 500.0;

impl Autosave {
    /// Something changed at `t`.
    pub fn touch(&mut self, t: f64) {
        self.due = Some(t + DEBOUNCE_MS);
    }

    pub fn pending(&self) -> bool {
        self.due.is_some()
    }

    /// When the save is due, for frame scheduling.
    pub fn due(&self) -> Option<f64> {
        self.due
    }

    /// True once at `t` when the timer has run out (or `now` forces it).
    pub fn fire(&mut self, t: f64, now: bool) -> bool {
        match self.due {
            Some(due) if now || t >= due => {
                self.due = None;
                true
            }
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(id: &str, text: &str, modified: i64) -> Note {
        Note { id: id.into(), text: text.into(), modified }
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/target/test-notes")).join(format!("{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn title_and_snippet_skip_blank_lines() {
        let n = note("1", "\n  Shopping \n\nmilk\neggs", 0);
        assert_eq!(n.title(), "Shopping");
        assert_eq!(n.snippet(), "milk");
        let empty = note("2", "", 0);
        assert_eq!((empty.title(), empty.snippet()), ("New Note", "No additional text"));
        assert_eq!(note("3", "only", 0).snippet(), "No additional text");
        assert_eq!(note("4", "## Plan\nx", 0).title(), "Plan", "heading markers are dropped");
        assert_eq!(note("5", "#hashtag", 0).title(), "hashtag");
        assert_eq!(note("6", "###", 0).title(), "###", "a bare marker stays");
    }

    #[test]
    fn sorts_newest_first_and_searches_case_insensitively() {
        let mut v = vec![note("a", "Pears", 10), note("b", "bread and Butter", 30), note("c", "cheese", 30)];
        sort(&mut v);
        assert_eq!(v.iter().map(|n| n.id.as_str()).collect::<Vec<_>>(), ["c", "b", "a"]);
        assert_eq!(search(&v, "BUTTER"), vec![1]);
        assert_eq!(search(&v, "e"), vec![0, 1, 2]);
        assert_eq!(search(&v, "  "), vec![0, 1, 2], "blank query matches all");
        assert!(search(&v, "zzz").is_empty());
    }

    #[test]
    fn date_labels_depend_on_distance_from_now() {
        // 2026-10-04 00:23 UTC
        let now = 1_791_073_380;
        assert_eq!(list_date(now - 60, now, 0), "00:22");
        assert_eq!(list_date(now - 3 * 3600, now, 0), "Saturday", "yesterday evening is a weekday");
        assert_eq!(list_date(now - 3 * 3600, now, 4 * 3600), "01:23", "but the same local day further east");
        assert_eq!(list_date(now - 2 * 86_400, now, 0), "Friday");
        assert_eq!(list_date(now - 10 * 86_400, now, 0), "24/09/2026");
        assert_eq!(list_date(now + 86_400, now, 0), "05/10/2026", "a future stamp is never a weekday");
        assert_eq!(long_date(now, 0), "4 October 2026 at 00:23");
    }

    #[test]
    fn trash_caption_counts_days_until_the_purge() {
        let now = 1_791_073_380;
        assert_eq!(trash_caption(now, now), "Deleted today · 30 days left");
        assert_eq!(trash_caption(now - 86_400, now), "Deleted yesterday · 29 days left");
        assert_eq!(trash_caption(now - 3 * 86_400, now), "Deleted 3 days ago · 27 days left");
        assert_eq!(trash_caption(now - 31 * 86_400, now), "Deleted 31 days ago · purging soon");
    }

    #[test]
    fn saves_loads_and_deletes_files() {
        let dir = scratch("io");
        assert!(load(&dir).is_empty(), "no directory yet");
        let a = note("100", "First\nbody", 0);
        let b = note("200", "Second", 0);
        save(&dir, &a).unwrap();
        save(&dir, &b).unwrap();
        std::fs::write(dir.join("junk.txt"), "x").unwrap();
        let loaded = load(&dir);
        assert_eq!(loaded.len(), 2);
        assert!(loaded.iter().all(|n| n.modified > 1_700_000_000), "mtime read back");
        assert_eq!(loaded.iter().find(|n| n.id == "100").unwrap().text, "First\nbody");
        assert!(!dir.join("100.md.tmp").exists());
        delete(&dir, "100");
        assert_eq!(load(&dir).len(), 1);
        assert_eq!(new_id(5, &loaded), "5");
        assert_eq!(new_id(200, &loaded), "201", "ids never clash");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn autosave_waits_half_a_second_after_the_last_change() {
        let mut a = Autosave::default();
        assert!(!a.pending() && !a.fire(0.0, false));
        a.touch(1000.0);
        a.touch(1200.0);
        assert_eq!(a.due(), Some(1700.0));
        assert!(!a.fire(1699.0, false));
        assert!(a.fire(1700.0, false));
        assert!(!a.pending() && !a.fire(5000.0, false), "fires once");
        a.touch(2000.0);
        assert!(a.fire(2001.0, true), "forced by leaving the note");
    }

    #[test]
    fn pins_persist_without_changing_markdown_or_modification_time() {
        let dir = scratch("pins");
        let mut original = note("Íslensk note", "# Keep this title\n\nMarkdown **body**", 0);
        save(&dir, &original).unwrap();
        let modified = std::fs::metadata(path(&dir, &original.id)).unwrap().modified().unwrap();
        set_pinned(&dir, &original.id, true).unwrap();
        assert_eq!(std::fs::read_to_string(path(&dir, &original.id)).unwrap(), original.text);
        assert_eq!(std::fs::metadata(path(&dir, &original.id)).unwrap().modified().unwrap(), modified);
        assert!(load_pins(&dir, &load(&dir)).contains(&original.id));
        original.text.push_str("\nMore text");
        save(&dir, &original).unwrap();
        assert!(load_pins(&dir, &load(&dir)).contains(&original.id), "text saves preserve pin state");
        set_pinned(&dir, &original.id, false).unwrap();
        assert!(load_pins(&dir, &load(&dir)).is_empty());
        set_pinned(&dir, &original.id, true).unwrap();
        delete(&dir, &original.id);
        assert!(!pin_path(&dir, &original.id).exists());
        let long_name = note(&"é".repeat(120), "Imported note with a long filename", 0);
        save(&dir, &long_name).unwrap();
        set_pinned(&dir, &long_name.id, true).unwrap();
        assert!(load_pins(&dir, &load(&dir)).contains(&long_name.id));
        set_pinned(&dir, &long_name.id, false).unwrap();
        assert!(load_pins(&dir, &load(&dir)).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn pinned_sort_and_search_keep_section_order() {
        let mut notes = vec![note("a", "match oldest pinned", 10), note("b", "match newest regular", 100), note("c", "match newer pinned", 30), note("d", "other", 200)];
        let pins: HashSet<_> = ["a".to_string(), "c".to_string()].into_iter().collect();
        sort_with_pins(&mut notes, &pins);
        assert_eq!(notes.iter().map(|n| n.id.as_str()).collect::<Vec<_>>(), ["c", "a", "d", "b"]);
        let shown = search(&notes, "MATCH");
        assert_eq!(shown.iter().map(|&i| notes[i].id.as_str()).collect::<Vec<_>>(), ["c", "a", "b"]);
    }

    #[test]
    fn sort_keys_order_pinned_first() {
        let mut v = vec![note("1700000000000", "Banana", 500), note("1800000000000", "apricot", 100), note("1600000000000", "Cherry", 900), note("odd", "Date", 50)];
        let none = HashSet::new();
        sort_by_key(&mut v, SortKey::DateEdited, &none);
        assert_eq!(v.iter().map(|n| n.title()).collect::<Vec<_>>(), ["Cherry", "Banana", "apricot", "Date"]);
        sort_by_key(&mut v, SortKey::DateCreated, &none);
        assert_eq!(v.iter().map(|n| n.title()).collect::<Vec<_>>(), ["apricot", "Banana", "Cherry", "Date"], "odd id falls back to modified (50 s)");
        sort_by_key(&mut v, SortKey::Title, &none);
        assert_eq!(v.iter().map(|n| n.title()).collect::<Vec<_>>(), ["apricot", "Banana", "Cherry", "Date"]);
        let pins: HashSet<String> = ["odd".to_string()].into_iter().collect();
        sort_by_key(&mut v, SortKey::Title, &pins);
        assert_eq!(v[0].title(), "Date");
        assert_eq!(created(&v[1]), 1_800_000_000);
    }

    #[test]
    fn find_all_is_case_insensitive_and_non_overlapping() {
        assert_eq!(find_all("Rust rust RUST", "rust"), vec![0..4, 5..9, 10..14]);
        assert_eq!(find_all("aaaa", "aa"), vec![0..2, 2..4]);
        assert_eq!(find_all("Éa éa", "éA"), vec![0..2, 3..5]);
        assert!(find_all("abc", "  ").is_empty());
        assert!(find_all("ab", "abc").is_empty());
    }

    #[test]
    fn ranking_prefers_titles_then_body_hits_and_keeps_pins_first() {
        let notes = vec![note("a", "Groceries\nbuy rust-proof paint", 3), note("b", "Rust shell\nnotes", 2), note("c", "Ideas\nrust rust rust", 1), note("d", "My rust ideas", 0), note("e", "unrelated", 9)];
        let none = HashSet::new();
        assert_eq!(search_ranked(&notes, "RUST", &none), vec![1, 3, 2, 0]);
        assert_eq!(search_ranked(&notes, "", &none), vec![0, 1, 2, 3, 4]);
        let pins: HashSet<String> = ["c".to_string()].into_iter().collect();
        assert_eq!(search_ranked(&notes, "rust", &pins)[0], 2);
        assert!(search_ranked(&notes, "zzz", &none).is_empty());
        assert_eq!(score(&notes[4], "rust"), 0);
    }

    #[test]
    fn match_stepping_wraps_both_ways() {
        assert_eq!(step_match(0, 3, 1), 1);
        assert_eq!(step_match(2, 3, 1), 0);
        assert_eq!(step_match(0, 3, -1), 2);
        assert_eq!(step_match(0, 0, 1), 0);
    }

    #[test]
    fn prefs_round_trip_and_tolerate_garbage() {
        let dir = scratch("prefs");
        assert_eq!(load_prefs(&dir), Prefs::default());
        let set = Prefs { sort: SortKey::Title, checked_to_bottom: true, oldest_first: true, math_results: true, resume_quick: true, group_by_date: true, layout: super::super::gallery::Layout::Gallery };
        save_prefs(&dir, set).unwrap();
        assert_eq!(load_prefs(&dir), set);
        std::fs::write(dir.join(".springboard-notes-prefs"), "sort=nonsense\nzzz\nchecked_to_bottom=0\nlayout=grid\n").unwrap();
        assert_eq!(load_prefs(&dir), Prefs::default());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn export_writes_a_sanitised_markdown_file() {
        let dir = scratch("export");
        let n = note("1", "# My: plan/2\n- [ ] x", 0);
        let path = export_md(&dir, &n).unwrap();
        assert_eq!(path.file_name().unwrap(), "My plan2.md");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), n.text);
        assert_eq!(export_md(&dir, &note("2", "", 0)).unwrap().file_name().unwrap(), "New Note.md");
        assert!(load(&dir).is_empty(), "the export directory is not a note source");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn unknown_pin_metadata_is_preserved() {
        let dir = scratch("pin-unknown");
        let n = note("1", "Precious text", 0);
        save(&dir, &n).unwrap();
        let marker = pin_path(&dir, &n.id);
        std::fs::create_dir_all(marker.parent().unwrap()).unwrap();
        std::fs::write(&marker, "future format").unwrap();
        assert!(load_pins(&dir, &[n.clone()]).is_empty());
        assert!(set_pinned(&dir, &n.id, true).is_err());
        assert!(set_pinned(&dir, &n.id, false).is_err());
        assert_eq!(std::fs::read_to_string(marker).unwrap(), "future format");
        assert_eq!(load(&dir)[0].text, n.text);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn date_sorts_can_run_oldest_first_but_the_title_stays_a_to_z() {
        let mut v = vec![note("1700000000000", "Banana", 500), note("1800000000000", "apricot", 100), note("1600000000000", "Cherry", 900)];
        let pins: HashSet<String> = ["1700000000000".to_string()].into_iter().collect();
        sort_ordered(&mut v, SortKey::DateEdited, true, &HashSet::new());
        assert_eq!(v.iter().map(|n| n.title()).collect::<Vec<_>>(), ["apricot", "Banana", "Cherry"]);
        sort_ordered(&mut v, SortKey::DateCreated, true, &HashSet::new());
        assert_eq!(v.iter().map(|n| n.title()).collect::<Vec<_>>(), ["Cherry", "Banana", "apricot"]);
        sort_ordered(&mut v, SortKey::Title, true, &HashSet::new());
        assert_eq!(v.iter().map(|n| n.title()).collect::<Vec<_>>(), ["apricot", "Banana", "Cherry"], "direction does not reverse the title");
        sort_ordered(&mut v, SortKey::DateEdited, true, &pins);
        assert_eq!(v[0].title(), "Banana", "pinned notes still come first");
        assert_eq!(v[1].title(), "apricot");
    }

    #[test]
    fn dates_fall_into_today_week_month_and_older_sections() {
        // 2026-10-13 15:00 UTC
        let now = 1_791_903_600;
        let day = 86_400;
        assert_eq!(date_group(now, now, 0), "Today");
        assert_eq!(date_group(now - 3_600 * 14, now, 0), "Today", "earlier the same day");
        assert_eq!(date_group(now - 3_600 * 16, now, 0), "Previous 7 Days", "yesterday morning is the day before");
        assert_eq!(date_group(now - 7 * day, now, 0), "Previous 7 Days");
        assert_eq!(date_group(now - 8 * day, now, 0), "Previous 30 Days");
        assert_eq!(date_group(now - 30 * day, now, 0), "Previous 30 Days");
        assert_eq!(date_group(now - 31 * day, now, 0), "September");
        assert_eq!(date_group(now - 120 * day, now, 0), "June");
        assert_eq!(date_group(now - 400 * day, now, 0), "September 2025");
        assert_eq!(date_group(now + day, now, 0), "Today", "a date in the future is today");
        // the local day, not the UTC day, decides: 23:30 UTC is already tomorrow at +2h
        assert_eq!(date_group(now - 3_600 * 15 - 1_800, now, 7_200), "Today");
        assert_eq!(date_group(now - 3_600 * 15 - 1_800, now, 0), "Previous 7 Days");
    }

    #[test]
    fn tags_are_hash_words_that_start_a_word() {
        assert_eq!(tags("Trip\nbuy #Milk and #eggs, #milk again\n#to-do"), vec!["milk", "eggs", "to-do"]);
        assert!(tags("# Heading\n## Second\nC# code\nsee page#anchor\n#123 and ##x").is_empty());
        assert_eq!(tags("#a_b-c- end"), vec!["a_b-c"]);
        assert_eq!(tags("(#paren) #caf\u{e9}"), vec!["caf\u{e9}"], "a bracket in front means no tag; letters with accents count");
        assert!(tags("").is_empty());
        assert!(has_tag("x #Home", "home") && has_tag("x #home", "#HOME"));
        assert!(!has_tag("x #homework", "home"));
    }

    #[test]
    fn the_last_quick_note_is_remembered() {
        let dir = scratch("quick");
        assert_eq!(load_quick(&dir), None);
        save_quick(&dir, "1700000000123").unwrap();
        assert_eq!(load_quick(&dir), Some("1700000000123".to_string()));
        std::fs::write(dir.join(".springboard-notes-quick"), "\n").unwrap();
        assert_eq!(load_quick(&dir), None, "an empty record is no record");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
