//! Smart folders: saved searches that list their matches like a folder. A smart folder
//! names a scope folder and filters it by name words, item type, colour tag and age.
//! They live in the sidebar's Smart Folders section, stored one per line in `.smart-folders`
//! at home (`name`, `words`, `type`, `tag`, `days`, `scope`, tab-separated; `-` for none).
use crate::*;
use springboard::files::find::{name_matches, TypeFilter, MAX_DEPTH};

/// The stand-in folder of the Smart Folders sidebar rows: `/.smart/<index>`.
pub(crate) const SMART: &str = "/.smart";
/// The saved searches file, at home.
pub(crate) const STORE: &str = ".smart-folders";
/// Most matches one smart folder lists (keeps a huge tree from flooding the listing).
pub(crate) const LIMIT: usize = 1000;
const DAY: i64 = 86_400;

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct SmartFolder {
    pub(crate) name: String,
    /// Every word must appear in the name, ignoring case; empty matches any name.
    pub(crate) words: String,
    pub(crate) kind: TypeFilter,
    pub(crate) tag: Option<Tag>,
    /// Only items changed within this many days.
    pub(crate) within_days: Option<u32>,
    pub(crate) scope: PathBuf,
}

/// The stand-in path of smart folder `index`.
pub(crate) fn smart_path(index: usize) -> PathBuf {
    Path::new(SMART).join(index.to_string())
}

/// The smart folder index a stand-in path names, if any.
pub(crate) fn folder_index(path: &Path) -> Option<usize> {
    path.strip_prefix(SMART).ok()?.to_str()?.parse().ok()
}

#[cfg_attr(not(test), allow(dead_code))] // saving waits for the New Smart Folder sheet; the tests round-trip it
fn field(s: &str) -> String {
    s.replace(['\t', '\n', '\r'], " ")
}

fn dash_or(s: &str) -> Option<&str> {
    (s != "-").then_some(s)
}

impl SmartFolder {
    #[cfg_attr(not(test), allow(dead_code))] // saving waits for the New Smart Folder sheet; the tests round-trip it
    pub(crate) fn serialize(&self) -> String {
        let days = self.within_days.map_or("-".to_string(), |d| d.to_string());
        let tag = self.tag.map_or("-", |t| t.label());
        let words = if self.words.is_empty() { "-".to_string() } else { field(&self.words) };
        format!("{}\t{}\t{}\t{}\t{}\t{}", field(&self.name), words, self.kind.label(), tag, days, self.scope.display())
    }

    fn parse_line(line: &str) -> Option<SmartFolder> {
        let f: Vec<&str> = line.split('\t').collect();
        if f.len() != 6 || f[0].is_empty() {
            return None;
        }
        Some(SmartFolder {
            name: f[0].to_string(),
            words: dash_or(f[1]).unwrap_or("").to_string(),
            kind: TypeFilter::ALL.into_iter().find(|k| k.label() == f[2])?,
            tag: match dash_or(f[3]) {
                Some(t) => Some(Tag::ALL.into_iter().find(|x| x.label() == t)?),
                None => None,
            },
            within_days: dash_or(f[4]).and_then(|d| d.parse().ok()),
            scope: PathBuf::from(f[5]),
        })
    }

    /// Whether an item (with its colour tags) belongs in this folder, at time `now` (Unix seconds).
    pub(crate) fn matches(&self, entry: &Entry, tags: &[Tag], now: i64) -> bool {
        name_matches(&entry.name, &self.words)
            && self.kind.accepts(entry.kind)
            && self.tag.is_none_or(|t| tags.contains(&t))
            && self.within_days.is_none_or(|d| now - entry.mtime <= i64::from(d) * DAY)
    }

    /// The items under the scope that match, sorted by name and capped at `LIMIT`.
    /// Symlinked folders are not followed, the trash and the tag sidecar are skipped.
    pub(crate) fn results(&self, index: &TagIndex, now: i64) -> Vec<Entry> {
        let mut hits = vec![];
        let mut pending = vec![(self.scope.clone(), 0usize)];
        while let Some((dir, depth)) = pending.pop() {
            if hits.len() >= LIMIT {
                break;
            }
            let Ok(rd) = std::fs::read_dir(&dir) else { continue };
            for e in rd.flatten() {
                let name = e.file_name().to_string_lossy().into_owned();
                if name == springboard::files::trash::DIR || name == springboard::files::tags::SIDECAR {
                    continue;
                }
                let path = e.path();
                let Ok(link) = std::fs::symlink_metadata(&path) else { continue };
                let md = if link.file_type().is_symlink() { std::fs::metadata(&path).unwrap_or(link.clone()) } else { link.clone() };
                let is_dir = md.is_dir() && !fs::is_package_name(&name);
                if is_dir && !link.file_type().is_symlink() && depth < MAX_DEPTH {
                    pending.push((path.clone(), depth + 1));
                }
                let mtime = md.modified().ok().and_then(|m| m.duration_since(UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs() as i64);
                let entry = Entry { kind: fs::kind_of(&name, is_dir), name, is_dir, size: if is_dir { 0 } else { md.len() }, mtime, items: None, path };
                if self.matches(&entry, &index.tags(&entry.path), now) {
                    hits.push(entry);
                }
            }
        }
        hits.sort_by(|a, b| fs::natural_cmp(&a.name, &b.name));
        hits.truncate(LIMIT);
        hits
    }
}

/// The saved searches in the store file's text; lines that do not parse are skipped.
pub(crate) fn parse(text: &str) -> Vec<SmartFolder> {
    text.lines().filter_map(SmartFolder::parse_line).collect()
}

#[cfg_attr(not(test), allow(dead_code))] // saving waits for the New Smart Folder sheet; the tests round-trip it
pub(crate) fn serialize(folders: &[SmartFolder]) -> String {
    folders.iter().map(|f| f.serialize() + "\n").collect()
}

/// The saved searches at home (none when the file is missing).
pub(crate) fn load(home: &Path) -> Vec<SmartFolder> {
    std::fs::read_to_string(home.join(STORE)).map(|t| parse(&t)).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, kind: Kind, mtime: i64) -> Entry {
        Entry { name: name.into(), path: PathBuf::from("/x").join(name), is_dir: kind == Kind::Folder, size: 1, mtime, kind, items: None }
    }

    fn folder(words: &str) -> SmartFolder {
        SmartFolder { name: "Find".into(), words: words.into(), kind: TypeFilter::All, tag: None, within_days: None, scope: PathBuf::from("/x") }
    }

    #[test]
    fn round_trips_through_the_store_format() {
        let f = SmartFolder { tag: Some(Tag::Blue), within_days: Some(7), kind: TypeFilter::Images, words: "trip paris".into(), ..folder("") };
        let text = serialize(&[f.clone(), folder("")]);
        assert_eq!(parse(&text), vec![f, folder("")]);
    }

    #[test]
    fn skips_lines_that_do_not_parse() {
        assert!(parse("broken line\nName\t-\tbogus\t-\t-\t/x\n\tq\tAll\t-\t-\t/x\n").is_empty());
    }

    #[test]
    fn stand_in_paths_round_trip() {
        assert_eq!(folder_index(&smart_path(3)), Some(3));
        assert_eq!(folder_index(Path::new("/.tagged/Red")), None);
    }

    #[test]
    fn matches_words_kind_tag_and_age() {
        let now = 100 * DAY;
        let photo = entry("Trip Paris.jpg", Kind::Image, now - DAY);
        let old = entry("Trip Rome.jpg", Kind::Image, now - 30 * DAY);
        let notes = entry("trip notes.txt", Kind::Text, now);
        assert!(folder("trip").matches(&photo, &[], now));
        assert!(!folder("paris rome").matches(&photo, &[], now));
        let images = SmartFolder { kind: TypeFilter::Images, ..folder("trip") };
        assert!(images.matches(&photo, &[], now) && !images.matches(&notes, &[], now));
        let red = SmartFolder { tag: Some(Tag::Red), ..folder("") };
        assert!(red.matches(&notes, &[Tag::Red], now) && !red.matches(&notes, &[], now));
        let recent = SmartFolder { within_days: Some(7), ..folder("trip") };
        assert!(recent.matches(&photo, &[], now) && !recent.matches(&old, &[], now));
    }
}
