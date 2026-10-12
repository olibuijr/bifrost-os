//! Saves and loads contact groups beside the contacts file, as `groups.txt`:
//!
//! ```text
//! contact-groups 1
//! group<TAB>Name
//! member<TAB>group number<TAB>contact identifier
//! ```
//!
//! Groups are numbered from 1 in the order they were made, which is how the model hands out identifiers, so the file
//! does not store them. The app never removes a group, so the numbers stay in step. Writes go to a temporary file and
//! then replace the old one.

use bifrost_app::controls::contact_groups::Groups;
use std::io;
use std::path::Path;

const HEADER: &str = "contact-groups 1";

/// Reads the groups file. A missing file is an empty set of groups.
pub fn load(path: &Path) -> io::Result<Groups> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Groups::new()),
        Err(e) => return Err(e),
    };
    let mut lines = text.lines();
    if lines.next() != Some(HEADER) {
        return Err(bad("Unknown groups format"));
    }
    let mut groups = Groups::new();
    for line in lines.filter(|l| !l.trim().is_empty()) {
        let fields: Vec<&str> = line.split('\t').collect();
        match fields.as_slice() {
            ["group", name] => {
                groups.add(name, None).map_err(|e| bad(&format!("Bad group name: {e:?}")))?;
            }
            ["member", group, contact] => {
                let group: u64 = group.parse().map_err(|_| bad("Bad group number"))?;
                let contact: u64 = contact.parse().map_err(|_| bad("Bad contact number"))?;
                groups.add_member(group, contact).map_err(|_| bad("Member of an unknown group"))?;
            }
            _ => return Err(bad("Unknown line in the groups file")),
        }
    }
    Ok(groups)
}

/// Writes the groups file. Names keep their text; tabs and line breaks become spaces.
pub fn save(path: &Path, groups: &Groups) -> io::Result<()> {
    let mut out = format!("{HEADER}\n");
    let count = groups.len() as u64;
    for id in 1..=count {
        let Some(group) = groups.get(id) else {
            return Err(bad("A group number is missing"));
        };
        let name: String = group.name.chars().map(|c| if c == '\t' || c == '\n' || c == '\r' { ' ' } else { c }).collect();
        out.push_str(&format!("group\t{}\n", name.trim()));
        let members = groups.members(id, false).map_err(|_| bad("A group cannot be read"))?;
        for contact in members {
            out.push_str(&format!("member\t{id}\t{contact}\n"));
        }
    }
    let temp = path.with_extension("txt.tmp");
    std::fs::write(&temp, out)?;
    std::fs::rename(&temp, path)
}

fn bad(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("contacts-groups-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn groups_and_their_members_round_trip() {
        let path = dir("round").join("groups.txt");
        let mut groups = Groups::new();
        let family = groups.add("Fjölskylda", None).unwrap();
        let work = groups.add("Work\tcrew", None).unwrap();
        groups.add_member(family, 1).unwrap();
        groups.add_member(family, 7).unwrap();
        groups.add_member(work, 7).unwrap();
        save(&path, &groups).unwrap();

        let back = load(&path).unwrap();
        assert_eq!(back.len(), 2);
        assert_eq!(back.get(1).unwrap().name, "Fjölskylda");
        assert_eq!(back.get(2).unwrap().name, "Work crew", "a tab in a name becomes a space");
        assert_eq!(back.members(1, false).unwrap(), vec![1, 7]);
        assert_eq!(back.groups_of(7), vec![1, 2]);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn a_missing_file_is_no_groups_and_a_foreign_file_is_refused() {
        let path = dir("missing").join("groups.txt");
        assert!(load(&path).unwrap().is_empty());
        std::fs::write(&path, "something else\n").unwrap();
        assert!(load(&path).is_err());
        std::fs::write(&path, format!("{HEADER}\nmember\t9\t1\n")).unwrap();
        assert!(load(&path).is_err(), "a member of a group that is not in the file is refused");
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }
}
