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

use crate::extras::{escape, unescape};
use crate::smart::{Condition, Rule, RuleField, UserList, MAX_LISTS, MAX_RULES};
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

const LISTS_HEADER: &str = "contact-smart-lists 1";
const LISTS_MAX_BYTES: u64 = 1024 * 1024;

/// The file text of the smart lists, as `smartlists.txt` holds them:
///
/// ```text
/// contact-smart-lists 1
/// list<TAB>name<TAB>all|any
/// rule<TAB>field<TAB>condition<TAB>value
/// ```
///
/// A `rule` line belongs to the `list` line before it. Names and values are escaped like the other contact files.
pub fn lists_text(lists: &[UserList]) -> String {
    let mut out = format!("{LISTS_HEADER}\n");
    for list in lists {
        out.push_str(&format!("list\t{}\t{}\n", escape(list.name.trim()), if list.any { "any" } else { "all" }));
        for rule in &list.rules {
            out.push_str(&format!("rule\t{}\t{}\t{}\n", rule.field.code(), rule.condition.code(), escape(rule.value.trim())));
        }
    }
    out
}

/// Reads the smart lists from their file text. `None` when the header is not this format's. Lines that do not read are
/// skipped, a list that is not valid is dropped, and at most `MAX_LISTS` lists of `MAX_RULES` rules are kept.
pub fn parse_lists(text: &str) -> Option<Vec<UserList>> {
    let mut lines = text.lines();
    if lines.next() != Some(LISTS_HEADER) {
        return None;
    }
    let mut lists: Vec<UserList> = Vec::new();
    for line in lines {
        let fields: Vec<&str> = line.split('\t').collect();
        match fields.as_slice() {
            ["list", name, mode] => lists.push(UserList { name: unescape(name), any: *mode == "any", rules: Vec::new() }),
            ["rule", field, condition, value] => {
                let (Some(field), Some(condition), Some(list)) = (RuleField::from_code(field), Condition::from_code(condition), lists.last_mut()) else {
                    continue;
                };
                if list.rules.len() < MAX_RULES {
                    list.rules.push(Rule { field, condition, value: unescape(value) });
                }
            }
            _ => {}
        }
    }
    lists.retain(UserList::valid);
    lists.truncate(MAX_LISTS);
    Some(lists)
}

/// Reads the smart lists file. A missing file is no lists. A file that is too large, cannot be read or has another header
/// is an error, so the app does not overwrite it.
pub fn load_lists(path: &Path) -> io::Result<Vec<UserList>> {
    match std::fs::metadata(path) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
        Ok(meta) if meta.len() > LISTS_MAX_BYTES => return Err(bad("The smart lists file is too large")),
        Ok(_) => {}
    }
    let text = std::fs::read_to_string(path)?;
    parse_lists(&text).ok_or_else(|| bad("Unknown smart lists format"))
}

/// Writes the smart lists file atomically.
pub fn save_lists(path: &Path, lists: &[UserList]) -> io::Result<()> {
    let temp = path.with_extension("txt.tmp");
    std::fs::write(&temp, lists_text(lists))?;
    std::fs::rename(&temp, path)
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

    fn sample_lists() -> Vec<UserList> {
        vec![
            UserList {
                name: "Acme, people".into(),
                any: false,
                rules: vec![
                    Rule { field: RuleField::Company, condition: Condition::Contains, value: "acme\\ltd".into() },
                    Rule { field: RuleField::Email, condition: Condition::IsSet, value: String::new() },
                ],
            },
            UserList { name: "Friends".into(), any: true, rules: vec![Rule { field: RuleField::Name, condition: Condition::BeginsWith, value: "Ó".into() }] },
        ]
    }

    #[test]
    fn smart_lists_round_trip_through_their_file_text_and_file() {
        let lists = sample_lists();
        let text = lists_text(&lists);
        assert!(text.starts_with("contact-smart-lists 1\nlist\t"), "{text}");
        assert!(text.contains("rule\tcompany\tcontains\tacme\\\\ltd\n"), "a backslash is escaped: {text}");
        let back = parse_lists(&text).unwrap();
        assert_eq!(back[0].name, "Acme, people");
        assert_eq!(back, lists);
        let path = dir("lists").join("smartlists.txt");
        assert_eq!(load_lists(&path).unwrap(), Vec::<UserList>::new(), "a missing file is no lists");
        save_lists(&path, &lists).unwrap();
        assert_eq!(load_lists(&path).unwrap(), lists);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn a_foreign_lists_file_is_refused_and_bad_lines_and_lists_are_dropped() {
        assert_eq!(parse_lists("contact-groups 2\n"), None);
        let path = dir("lists-bad").join("smartlists.txt");
        std::fs::write(&path, "something else\n").unwrap();
        assert!(load_lists(&path).is_err());
        let text = "contact-smart-lists 1\n\
rule\tname\tcontains\torphan\n\
list\tOk\tall\n\
rule\tshoe\tcontains\tx\n\
rule\tname\tcontains\tan\n\
list\tNo rules\tall\n\
list\t\tall\n\
rule\tname\tcontains\tx\n\
list\tEmpty value\tall\n\
rule\tname\tis\t\n\
junk\n";
        let lists = parse_lists(text).unwrap();
        assert_eq!(lists.iter().map(|l| l.name.as_str()).collect::<Vec<_>>(), ["Ok"], "only the valid list is kept");
        assert_eq!(lists[0].rules.len(), 1, "the rule with an unknown field is skipped");
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn at_most_thirty_lists_and_six_rules_are_kept() {
        let mut text = String::from("contact-smart-lists 1\n");
        for n in 0..MAX_LISTS + 5 {
            text.push_str(&format!("list\tList {n}\tall\n"));
            for r in 0..MAX_RULES + 3 {
                text.push_str(&format!("rule\tname\tcontains\tvalue {r}\n"));
            }
        }
        let lists = parse_lists(&text).unwrap();
        assert_eq!(lists.len(), MAX_LISTS);
        assert!(lists.iter().all(|l| l.rules.len() == MAX_RULES));
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
