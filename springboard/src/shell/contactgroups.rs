//! Contact groups kept in a sidecar file beside the contacts file, so the contacts file keeps its format and an older
//! shell ignores the sidecar. A missing sidecar is an empty set of groups. The file is a header line, a `last` line
//! with the last identifier handed out, then one `group` line per group: id, parent (`-` at the top), escaped name, and
//! the member contact ids, comma separated.
use super::prefs::{escape_field as escape, unescape_field as unescape};
use bifrost_ui::contact_groups::Groups;
use std::io;
use std::path::{Path, PathBuf};

const HEADER: &str = "springboard-contact-groups-v1";
const MAX_NAME_FIELD: usize = 160;

/// The sidecar path for a contacts file.
pub fn path_for(contacts_file: &Path) -> PathBuf {
    contacts_file.with_file_name("contact-groups.tsv")
}

/// Writes the groups as the sidecar text.
pub fn render(groups: &Groups) -> String {
    let mut out = format!("{HEADER}\nlast\t{}\n", groups.last_id());
    for group in groups.all() {
        let parent = group.parent.map(|p| p.to_string()).unwrap_or_else(|| "-".to_string());
        let members: Vec<String> = groups.members(group.id, false).unwrap_or_default().iter().map(|m| m.to_string()).collect();
        out.push_str(&format!("group\t{}\t{}\t{}\t{}\n", group.id, parent, escape(&group.name), members.join(",")));
    }
    out
}

/// Reads the sidecar text. Text that is not a groups sidecar, or whose groups do not hold together, is an error.
pub fn parse(text: &str) -> Result<Groups, String> {
    let mut lines = text.lines();
    if lines.next() != Some(HEADER) {
        return Err("Unknown contact groups format".into());
    }
    let mut last_id = None;
    let mut rows = Vec::new();
    for line in lines.filter(|l| !l.is_empty()) {
        let fields: Vec<&str> = line.split('\t').collect();
        match fields.as_slice() {
            ["last", n] => last_id = Some(n.parse::<u64>().map_err(|_| "bad group counter".to_string())?),
            ["group", id, parent, name, members] => {
                let id = id.parse::<u64>().map_err(|_| "bad group id".to_string())?;
                let parent = if *parent == "-" { None } else { Some(parent.parse::<u64>().map_err(|_| "bad group parent".to_string())?) };
                let name = unescape(name, MAX_NAME_FIELD).ok_or_else(|| "bad group name".to_string())?;
                let members = if members.is_empty() {
                    Vec::new()
                } else {
                    members
                        .split(',')
                        .map(|m| m.parse::<u64>())
                        .collect::<Result<Vec<u64>, _>>()
                        .map_err(|_| "bad group member".to_string())?
                };
                rows.push((id, name, parent, members));
            }
            _ => return Err("bad contact groups line".into()),
        }
    }
    let last_id = last_id.ok_or_else(|| "missing group counter".to_string())?;
    Groups::restore(last_id, rows).map_err(|e| format!("{e:?}"))
}

/// The sidecar at `path`; empty when it is missing or unreadable.
pub fn load(path: &Path) -> Groups {
    std::fs::read_to_string(path).ok().and_then(|t| parse(&t).ok()).unwrap_or_default()
}

/// Writes the sidecar atomically.
pub fn save(path: &Path, groups: &Groups) -> io::Result<()> {
    super::fsutil::write_atomic(path, render(groups))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Groups {
        let mut groups = Groups::new();
        let family = groups.add("Family", None).unwrap();
        let kin = groups.add("Kin, far", Some(family)).unwrap();
        groups.add_member(kin, 7).unwrap();
        groups.add_member(family, 3).unwrap();
        groups
    }

    #[test]
    fn groups_round_trip_with_names_members_and_the_counter() {
        let text = render(&sample());
        let back = parse(&text).unwrap();
        assert_eq!(render(&back), text);
        assert_eq!(back.len(), 2);
        assert_eq!(back.members(1, true).unwrap(), vec![3, 7]);
        assert_eq!(back.last_id(), 2);
    }

    #[test]
    fn a_restored_set_never_reuses_an_identifier() {
        let mut back = parse(&render(&sample())).unwrap();
        assert_eq!(back.add("Work", None).unwrap(), 3);
    }

    #[test]
    fn a_missing_or_foreign_sidecar_is_refused_or_empty() {
        assert!(load(Path::new("/nonexistent/contact-groups.tsv")).is_empty());
        assert!(parse("not a groups file").is_err());
    }

    #[test]
    fn a_group_with_an_unknown_parent_is_refused() {
        let text = format!("{HEADER}\nlast\t2\ngroup\t2\t9\tOrphan\t\n");
        assert!(parse(&text).is_err());
    }

    #[test]
    fn the_sidecar_sits_beside_the_contacts_file() {
        assert_eq!(path_for(Path::new("/data/Contacts/contacts.tsv")), PathBuf::from("/data/Contacts/contact-groups.tsv"));
    }
}
