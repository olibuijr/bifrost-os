//! Saves and loads contact groups beside the contacts file, as `groups.txt`:
//!
//! ```text
//! contact-groups 2
//! group<TAB>group number<TAB>Name
//! member<TAB>group number<TAB>contact identifier
//! ```
//!
//! Group numbers are the model's identifiers. A deleted group leaves a gap, so the numbers are written out. Version 1
//! files (`group<TAB>Name`, numbered from 1 in order) still load. Writes go to a temporary file and then replace the
//! old one.

use bifrost_app::controls::contact_groups::Groups;
use std::collections::BTreeMap;
use std::io;
use std::path::Path;

const HEADER: &str = "contact-groups 2";
const OLD_HEADER: &str = "contact-groups 1";

/// Reads the groups file. A missing file is an empty set of groups. Returns the groups and their identifiers in the
/// order the file lists them, which is the order the sidebar shows them.
pub fn load(path: &Path) -> io::Result<(Groups, Vec<u64>)> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok((Groups::new(), Vec::new())),
        Err(e) => return Err(e),
    };
    let mut lines = text.lines();
    let numbered = match lines.next() {
        Some(HEADER) => true,
        Some(OLD_HEADER) => false,
        _ => return Err(bad("Unknown groups format")),
    };
    let mut groups = Groups::new();
    let mut ids: Vec<u64> = Vec::new();
    // File group number -> identifier in the model.
    let mut by_number: BTreeMap<u64, u64> = BTreeMap::new();
    let mut members: Vec<(u64, u64)> = Vec::new();
    for line in lines.filter(|l| !l.trim().is_empty()) {
        let fields: Vec<&str> = line.split('\t').collect();
        match (fields.as_slice(), numbered) {
            (["group", name], false) => {
                let number = ids.len() as u64 + 1;
                let id = groups.add(name, None).map_err(|e| bad(&format!("Bad group name: {e:?}")))?;
                by_number.insert(number, id);
                ids.push(id);
            }
            (["group", number, name], true) => {
                let number: u64 = number.parse().map_err(|_| bad("Bad group number"))?;
                if by_number.contains_key(&number) {
                    return Err(bad("A group number is listed twice"));
                }
                let id = groups.add(name, None).map_err(|e| bad(&format!("Bad group name: {e:?}")))?;
                by_number.insert(number, id);
                ids.push(id);
            }
            (["member", group, contact], _) => {
                let group: u64 = group.parse().map_err(|_| bad("Bad group number"))?;
                let contact: u64 = contact.parse().map_err(|_| bad("Bad contact number"))?;
                members.push((group, contact));
            }
            _ => return Err(bad("Unknown line in the groups file")),
        }
    }
    for (group, contact) in members {
        let id = *by_number.get(&group).ok_or_else(|| bad("Member of an unknown group"))?;
        groups.add_member(id, contact).map_err(|_| bad("Member of an unknown group"))?;
    }
    Ok((groups, ids))
}

/// Writes the groups file, the groups in the order of `ids`. Names keep their text; tabs and line breaks become
/// spaces.
pub fn save(path: &Path, groups: &Groups, ids: &[u64]) -> io::Result<()> {
    let mut out = format!("{HEADER}\n");
    for &id in ids {
        let Some(group) = groups.get(id) else {
            return Err(bad("A group number is missing"));
        };
        let name: String = group.name.chars().map(|c| if c == '\t' || c == '\n' || c == '\r' { ' ' } else { c }).collect();
        out.push_str(&format!("group\t{id}\t{}\n", name.trim()));
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
        save(&path, &groups, &[family, work]).unwrap();

        let (back, ids) = load(&path).unwrap();
        assert_eq!(ids.len(), 2);
        let (f, w) = (ids[0], ids[1]);
        assert_eq!(back.get(f).unwrap().name, "Fjölskylda");
        assert_eq!(back.get(w).unwrap().name, "Work crew", "a tab in a name becomes a space");
        assert_eq!(back.members(f, false).unwrap(), vec![1, 7]);
        assert_eq!(back.groups_of(7), vec![f, w]);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn a_deleted_group_leaves_a_gap_that_round_trips() {
        let path = dir("gap").join("groups.txt");
        let mut groups = Groups::new();
        let a = groups.add("A", None).unwrap();
        let b = groups.add("B", None).unwrap();
        let c = groups.add("C", None).unwrap();
        groups.add_member(c, 4).unwrap();
        assert_eq!(groups.remove(b).unwrap(), vec![b]);
        save(&path, &groups, &[a, c]).unwrap();

        let (back, ids) = load(&path).unwrap();
        assert_eq!(ids.len(), 2);
        assert_eq!(back.get(ids[1]).unwrap().name, "C");
        assert_eq!(back.members(ids[1], false).unwrap(), vec![4]);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn version_one_files_still_load_in_order() {
        let path = dir("v1").join("groups.txt");
        std::fs::write(&path, "contact-groups 1\ngroup\tFirst\ngroup\tSecond\nmember\t2\t9\n").unwrap();
        let (groups, ids) = load(&path).unwrap();
        assert_eq!(ids.len(), 2);
        assert_eq!(groups.get(ids[0]).unwrap().name, "First");
        assert_eq!(groups.members(ids[1], false).unwrap(), vec![9]);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn a_missing_file_is_no_groups_and_a_foreign_file_is_refused() {
        let path = dir("missing").join("groups.txt");
        let (groups, ids) = load(&path).unwrap();
        assert!(groups.is_empty() && ids.is_empty());
        std::fs::write(&path, "something else\n").unwrap();
        assert!(load(&path).is_err());
        std::fs::write(&path, format!("{HEADER}\nmember\t9\t1\n")).unwrap();
        assert!(load(&path).is_err(), "a member of a group that is not in the file is refused");
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }
}
