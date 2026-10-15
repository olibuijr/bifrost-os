//! The Contacts archive: one file that holds every data file of the Contacts folder (the contacts, their extra values
//! and card fields, the groups and the display settings), so the whole address book can be backed up and restored.
//!
//! ```text
//! bifrost-contacts-archive 1
//! file<TAB><name><TAB><byte length>
//! <the file's bytes, exactly that many><LF>
//! ```
//!
//! The archive is text, so it can be read, mailed and compared. File names are checked on the way in: only names the
//! caller lists are accepted, and a name with a path in it is refused. Pure std.

use std::io;
use std::path::Path;

const HEADER: &str = "bifrost-contacts-archive 1";
/// Largest archive read, in bytes.
pub const MAX_BYTES: usize = 32 * 1024 * 1024;

/// Packs files, given as (name, text), into one archive text.
pub fn pack(files: &[(String, String)]) -> String {
    let mut out = format!("{HEADER}\n");
    for (name, text) in files {
        out.push_str(&format!("file\t{name}\t{}\n{text}\n", text.len()));
    }
    out
}

/// The next line of `bytes` from `at`, without its line feed; `at` moves past it. `None` at the end or for text that is
/// not UTF-8.
fn take_line<'a>(bytes: &'a [u8], at: &mut usize) -> Option<&'a str> {
    let rest = bytes.get(*at..)?;
    let end = rest.iter().position(|b| *b == b'\n')?;
    let line = std::str::from_utf8(&rest[..end]).ok()?;
    *at += end + 1;
    Some(line)
}

/// Unpacks an archive into (name, text) pairs. Any damage (a wrong header, a short file, a repeated name) is an error,
/// so a half-read archive never replaces the address book.
pub fn unpack(bytes: &[u8]) -> Result<Vec<(String, String)>, String> {
    if bytes.len() > MAX_BYTES {
        return Err("The archive is too large".into());
    }
    let mut at = 0;
    if take_line(bytes, &mut at) != Some(HEADER) {
        return Err("This file is not a Contacts archive".into());
    }
    let mut out: Vec<(String, String)> = Vec::new();
    while at < bytes.len() {
        let damaged = || "The archive is damaged".to_string();
        let line = take_line(bytes, &mut at).ok_or_else(damaged)?;
        let fields: Vec<&str> = line.split('\t').collect();
        let &["file", name, length] = fields.as_slice() else { return Err(damaged()) };
        let length: usize = length.parse().map_err(|_| damaged())?;
        let end = at.checked_add(length).filter(|end| *end < bytes.len()).ok_or_else(damaged)?;
        let text = String::from_utf8(bytes[at..end].to_vec()).map_err(|_| damaged())?;
        if bytes[end] != b'\n' {
            return Err(damaged());
        }
        at = end + 1;
        if out.iter().any(|(existing, _)| existing == name) {
            return Err(format!("The archive lists {name} twice"));
        }
        out.push((name.to_string(), text));
    }
    Ok(out)
}

/// Whether `name` is a plain file name: letters, digits, `.`, `-` and `_`, no leading dot, at most 64 characters.
pub fn plain_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && !name.starts_with('.')
        && name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
}

/// Refuses an archive that holds a name outside `allowed`.
pub fn check_names(files: &[(String, String)], allowed: &[&str]) -> Result<(), String> {
    for (name, _) in files {
        if !plain_name(name) || !allowed.contains(&name.as_str()) {
            return Err(format!("The archive holds a file that Contacts does not use: {name}"));
        }
    }
    Ok(())
}

/// Reads the files of `names` that exist in `folder`, as (name, text) pairs in the order of `names`.
pub fn read_files(folder: &Path, names: &[&str]) -> io::Result<Vec<(String, String)>> {
    let mut out = Vec::new();
    for name in names {
        match std::fs::read_to_string(folder.join(name)) {
            Ok(text) => out.push((name.to_string(), text)),
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
    }
    Ok(out)
}

/// Makes `folder` hold exactly `files` among the files of `managed`: each file is written beside its place and renamed
/// over it, and a managed file the archive does not hold is removed, so the folder matches the archive.
pub fn restore(folder: &Path, files: &[(String, String)], managed: &[&str]) -> io::Result<()> {
    std::fs::create_dir_all(folder)?;
    for (name, text) in files {
        let target = folder.join(name);
        let temp = folder.join(format!("{name}.restore"));
        std::fs::write(&temp, text)?;
        std::fs::rename(&temp, &target)?;
    }
    for name in managed {
        if !files.iter().any(|(n, _)| n == name) {
            match std::fs::remove_file(folder.join(name)) {
                Ok(()) => {}
                Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                Err(e) => return Err(e),
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn files() -> Vec<(String, String)> {
        vec![
            ("contacts.tsv".into(), "springboard-contacts-v1\n1\tAnna\n".into()),
            ("values.txt".into(), "line one\n\nfile\tfake\t3\nline four".into()),
            ("empty.txt".into(), String::new()),
            ("names.txt".into(), "Ólafur Búi\n".into()),
        ]
    }

    #[test]
    fn files_round_trip_with_text_that_looks_like_the_format() {
        let packed = pack(&files());
        assert!(packed.starts_with("bifrost-contacts-archive 1\nfile\tcontacts.tsv\t"), "{packed}");
        assert_eq!(unpack(packed.as_bytes()).unwrap(), files());
    }

    #[test]
    fn lengths_count_bytes_not_characters() {
        let packed = pack(&[("n.txt".into(), "Óli".into())]);
        assert!(packed.contains("file\tn.txt\t4\n"), "{packed}");
        assert_eq!(unpack(packed.as_bytes()).unwrap()[0].1, "Óli");
    }

    #[test]
    fn an_empty_archive_has_no_files() {
        assert_eq!(unpack(pack(&[]).as_bytes()).unwrap(), Vec::<(String, String)>::new());
    }

    #[test]
    fn damaged_archives_are_refused() {
        let good = pack(&files());
        assert!(unpack(b"something else\n").is_err());
        assert!(unpack(b"").is_err());
        assert!(unpack(&good.as_bytes()[..good.len() - 5]).is_err(), "a short last file");
        assert!(unpack(format!("{HEADER}\nfile\ta.txt\t9999\nabc\n").as_bytes()).is_err(), "a length past the end");
        assert!(unpack(format!("{HEADER}\nfile\ta.txt\tx\nabc\n").as_bytes()).is_err(), "a length that is not a number");
        assert!(unpack(format!("{HEADER}\nfile\ta.txt\t3\nabcd").as_bytes()).is_err(), "no line feed after the file");
        assert!(unpack(format!("{HEADER}\nfile\ta.txt\t1\na\nfile\ta.txt\t1\nb\n").as_bytes()).is_err(), "a repeated name");
        assert!(unpack(format!("{HEADER}\ndir\ta.txt\t1\na\n").as_bytes()).is_err(), "an unknown record");
    }

    #[test]
    fn only_listed_plain_names_are_accepted() {
        assert!(check_names(&files(), &["contacts.tsv", "values.txt", "empty.txt", "names.txt"]).is_ok());
        assert!(check_names(&files(), &["contacts.tsv"]).is_err(), "values.txt is not listed");
        let sneaky = vec![("../contacts.tsv".to_string(), String::new())];
        assert!(check_names(&sneaky, &["../contacts.tsv"]).is_err(), "a path is never a plain name");
        assert!(!plain_name(".hidden"));
        assert!(!plain_name(""));
        assert!(!plain_name("a/b"));
        assert!(plain_name("groups.txt"));
    }

    #[test]
    fn restore_writes_the_archive_files_and_removes_the_managed_ones_it_lacks() {
        let dir = std::env::temp_dir().join(format!("contacts-archive-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("groups.txt"), "old groups").unwrap();
        std::fs::write(dir.join("notes-of-mine.txt"), "not managed").unwrap();
        let managed = ["contacts.tsv", "values.txt", "groups.txt"];
        let archive = vec![("contacts.tsv".to_string(), "new".to_string())];
        restore(&dir, &archive, &managed).unwrap();
        assert_eq!(std::fs::read_to_string(dir.join("contacts.tsv")).unwrap(), "new");
        assert!(!dir.join("groups.txt").exists(), "a managed file the archive lacks is removed");
        assert!(dir.join("notes-of-mine.txt").exists(), "other files are left alone");
        assert_eq!(read_files(&dir, &managed).unwrap(), archive);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
