//! A file-backed store for a [`ManagedContext`] (the reference persistent store over one file). The file holds the saved state
//! only: working changes are never written, so a context is saved first and then written. The file is one header
//! line, a `next <id>` line with the identity counter, and one `<id><TAB><record>` line per saved object. A record
//! comes from its [`Codec`] and must not contain a line break. Tabs are allowed: the line is split at its first tab, so a
//! record may use tabs as its own field separators.
//!
//! [`write`] writes a temporary file and renames it over the store, so a crash leaves either the old file or the new
//! one. A damaged line is an error on read: a store never loads with objects silently missing.

use crate::fetch::Record;
use crate::managed_context::{ManagedContext, ObjectId};
use std::{fs, io, path::Path};

const HEADER: &str = "# bifrost managed store 1";

/// How a record is written into the store file, as one line of text.
pub trait Codec: Sized {
    /// The line form of the record. It must not contain `\n` or `\r`; tabs are fine, the id is split off at the first one.
    fn encode(&self) -> String;
    /// Reads a record written by [`Codec::encode`]. `None` marks the line as damaged.
    fn decode(text: &str) -> Option<Self>;
}

/// Reads the store at `path`. A missing file is an empty context.
pub fn load<R>(path: &Path) -> io::Result<ManagedContext<R>>
where
    R: Clone + PartialEq + Record + Codec,
{
    match fs::read_to_string(path) {
        Ok(text) => parse(&text),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(ManagedContext::new()),
        Err(e) => Err(e),
    }
}

/// Reads store text, as [`load`] does for a file.
pub fn parse<R>(text: &str) -> io::Result<ManagedContext<R>>
where
    R: Clone + PartialEq + Record + Codec,
{
    let damaged = |line: usize| io::Error::new(io::ErrorKind::InvalidData, format!("store line {line} is damaged"));
    let mut next_id: ObjectId = 1;
    let mut saved: Vec<(ObjectId, R)> = Vec::new();
    for (index, line) in text.lines().enumerate() {
        let number = index + 1;
        if line.trim().is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(rest) = line.strip_prefix("next ") {
            next_id = rest.trim().parse().map_err(|_| damaged(number))?;
            continue;
        }
        let (id, body) = line.split_once('\t').ok_or_else(|| damaged(number))?;
        let id: ObjectId = id.parse().map_err(|_| damaged(number))?;
        let record = R::decode(body).ok_or_else(|| damaged(number))?;
        saved.push((id, record));
    }
    Ok(ManagedContext::restored(next_id, saved))
}

/// The store text for the saved state of `context`.
pub fn render<R>(context: &ManagedContext<R>) -> io::Result<String>
where
    R: Clone + PartialEq + Record + Codec,
{
    let mut out = format!("{HEADER}\nnext {}\n", context.next_id());
    for (id, record) in context.saved_state() {
        let body = record.encode();
        if body.contains(['\n', '\r']) {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "record encoding holds a line break"));
        }
        out.push_str(&format!("{id}\t{body}\n"));
    }
    Ok(out)
}

/// Writes the saved state of `context` to `path`. Call [`ManagedContext::save`] first, or the working changes are
/// not in the file.
pub fn write<R>(path: &Path, context: &ManagedContext<R>) -> io::Result<()>
where
    R: Clone + PartialEq + Record + Codec,
{
    let text = render(context)?;
    let temp = path.with_extension("tmp");
    fs::write(&temp, text)?;
    fs::rename(&temp, path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fetch::Field;

    #[derive(Clone, Debug, PartialEq)]
    struct Note {
        title: String,
        pages: i64,
    }

    impl Record for Note {
        fn field(&self, key: &str) -> Option<Field> {
            match key {
                "title" => Some(Field::Text(self.title.clone())),
                "pages" => Some(Field::Int(self.pages)),
                _ => None,
            }
        }
    }

    impl Codec for Note {
        fn encode(&self) -> String {
            format!("{}|{}", self.title, self.pages)
        }

        fn decode(text: &str) -> Option<Note> {
            let (title, pages) = text.rsplit_once('|')?;
            Some(Note { title: title.to_string(), pages: pages.parse().ok()? })
        }
    }

    fn note(title: &str, pages: i64) -> Note {
        Note { title: title.to_string(), pages }
    }

    #[test]
    fn saved_objects_survive_a_round_trip_with_their_identities() {
        let mut context = ManagedContext::new();
        let a = context.insert(note("Atlas", 120));
        let b = context.insert(note("Birds", 40));
        context.save();
        context.update(b, |n| n.pages = 41).unwrap();
        context.save();
        let text = render(&context).unwrap();
        let loaded: ManagedContext<Note> = parse(&text).unwrap();
        assert_eq!(loaded.get(a), Some(&note("Atlas", 120)));
        assert_eq!(loaded.get(b), Some(&note("Birds", 41)));
        assert!(!loaded.has_changes());
    }

    #[test]
    fn unsaved_changes_are_not_written() {
        let mut context = ManagedContext::new();
        context.insert(note("Saved", 1));
        context.save();
        context.insert(note("Pending", 2));
        let loaded: ManagedContext<Note> = parse(&render(&context).unwrap()).unwrap();
        let titles: Vec<String> = loaded.fetch(&Default::default()).into_iter().map(|(_, n)| n.title).collect();
        assert_eq!(titles, vec!["Saved".to_string()]);
    }

    #[test]
    fn identities_are_not_reused_after_a_delete_and_reload() {
        let mut context = ManagedContext::new();
        context.insert(note("Kept", 1));
        let last = context.insert(note("Old", 2));
        context.save();
        context.delete(last).unwrap();
        context.save();
        let mut loaded: ManagedContext<Note> = parse(&render(&context).unwrap()).unwrap();
        let next = loaded.insert(note("New", 3));
        assert!(next > last);
    }

    #[test]
    fn a_missing_file_is_an_empty_context() {
        let path = std::env::temp_dir().join(format!("persistent-store-missing-{}", std::process::id()));
        let loaded: ManagedContext<Note> = load(&path).unwrap();
        assert!(loaded.saved_state().is_empty());
    }

    #[test]
    fn write_then_load_reads_the_same_store() {
        let dir = std::env::temp_dir().join(format!("persistent-store-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("notes.store");
        let mut context = ManagedContext::new();
        context.insert(note("Disk", 7));
        context.save();
        write(&path, &context).unwrap();
        let loaded: ManagedContext<Note> = load(&path).unwrap();
        assert_eq!(loaded.saved_state(), context.saved_state());
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn a_damaged_line_refuses_the_whole_store() {
        assert!(parse::<Note>("next 3\n1\tno-separator\n").is_err());
        assert!(parse::<Note>("next x\n").is_err());
        assert!(parse::<Note>("1\tAtlas|many\n").is_err());
    }

    #[test]
    fn a_record_with_a_line_break_cannot_be_written() {
        let mut context = ManagedContext::new();
        context.insert(note("Two\nLines", 1));
        context.save();
        assert!(render(&context).is_err());
    }
}
