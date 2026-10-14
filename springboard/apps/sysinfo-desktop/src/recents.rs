//! The recently opened reports (File > Open Recent). Each entry is a document grant the shell gave when the file was
//! opened, with the file's name; the grant reopens the file without a panel. The list is kept in the app's preferences.

use bifrost_app::proto::GrantId;

/// The preference key that holds the list.
pub const KEY: &str = "report.recents";
/// How many reports Open Recent remembers.
pub const MAX: usize = 10;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Recent {
    pub grant: GrantId,
    pub name: String,
}

fn hex(grant: &GrantId) -> String {
    grant.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(text: &str) -> Option<GrantId> {
    let mut out = [0u8; 16];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(text.get(2 * i..2 * i + 2)?, 16).ok()?;
    }
    Some(out)
}

/// The stored text: one `grant<TAB>name` line per report, newest first.
pub fn format(list: &[Recent]) -> String {
    list.iter().map(|r| format!("{}\t{}\n", hex(&r.grant), r.name.replace(['\n', '\t'], " "))).collect()
}

pub fn parse(text: &str) -> Vec<Recent> {
    text.lines().filter_map(|l| l.split_once('\t').and_then(|(g, n)| Some(Recent { grant: unhex(g)?, name: n.to_string() }))).take(MAX).collect()
}

/// Puts a report first (an entry for the same grant moves up). Returns the grants that fell off the end, which the
/// caller releases.
pub fn note(list: &mut Vec<Recent>, grant: GrantId, name: &str) -> Vec<GrantId> {
    list.retain(|r| r.grant != grant);
    list.insert(0, Recent { grant, name: name.to_string() });
    let mut dropped = Vec::new();
    while list.len() > MAX {
        if let Some(old) = list.pop() {
            dropped.push(old.grant);
        }
    }
    dropped
}

/// Takes a report out of the list (its file is gone or was replaced). True when it was there.
pub fn forget(list: &mut Vec<Recent>, grant: &GrantId) -> bool {
    let before = list.len();
    list.retain(|r| r.grant != *grant);
    list.len() != before
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grant(n: u8) -> GrantId {
        [n; 16]
    }

    #[test]
    fn the_list_round_trips_through_its_text() {
        let list = vec![Recent { grant: grant(1), name: "A.json".into() }, Recent { grant: grant(2), name: "B report.json".into() }];
        assert_eq!(parse(&format(&list)), list);
        assert!(parse("").is_empty());
        assert!(parse("short\tname\nnot a line\n").is_empty(), "a bad grant is skipped");
    }

    #[test]
    fn a_name_cannot_break_the_line_format() {
        let list = vec![Recent { grant: grant(3), name: "a\tb\nc".into() }];
        assert_eq!(parse(&format(&list))[0].name, "a b c");
    }

    #[test]
    fn noting_a_report_moves_it_first_and_drops_the_oldest() {
        let mut list = Vec::new();
        for n in 0..MAX as u8 {
            assert!(note(&mut list, grant(n), "r").is_empty());
        }
        assert_eq!(list.len(), MAX);
        assert!(note(&mut list, grant(5), "again").is_empty(), "an entry already there only moves up");
        assert_eq!(list[0].grant, grant(5));
        assert_eq!(list.len(), MAX);
        let dropped = note(&mut list, grant(200), "new");
        assert_eq!(dropped, vec![grant(0)], "the oldest falls off");
        assert_eq!(list.len(), MAX);
    }

    #[test]
    fn forgetting_removes_one_entry() {
        let mut list = vec![Recent { grant: grant(1), name: "A".into() }];
        assert!(!forget(&mut list, &grant(9)));
        assert!(forget(&mut list, &grant(1)));
        assert!(list.is_empty());
    }
}
