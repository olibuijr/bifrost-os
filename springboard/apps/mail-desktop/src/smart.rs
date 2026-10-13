//! Smart mailboxes (Mailbox > New Smart Mailbox). A smart mailbox is a saved search: choosing it in the sidebar applies
//! its query (the syntax of [`crate::search`]) to the shown mailbox. They are kept in the app's data directory, one a
//! line: the name, a tab, the query.

use std::path::Path;

/// File name of the smart mailboxes in the app's data directory.
pub const FILE: &str = "smart.txt";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Smart {
    pub name: String,
    pub query: String,
}

fn clean(text: &str) -> String {
    text.chars().map(|c| if c == '\t' || c == '\r' || c == '\n' { ' ' } else { c }).collect::<String>().trim().to_string()
}

impl Smart {
    /// A smart mailbox from the sheet's two fields. The name must be new among `existing`.
    pub fn new(name: &str, query: &str, existing: &[Smart]) -> Result<Smart, String> {
        let (name, query) = (clean(name), clean(query));
        if name.is_empty() {
            return Err("Enter a name for the smart mailbox".into());
        }
        if query.is_empty() {
            return Err("Enter a search for the smart mailbox".into());
        }
        if existing.iter().any(|s| s.name.eq_ignore_ascii_case(&name)) {
            return Err(format!("There is already a smart mailbox named {name}"));
        }
        Ok(Smart { name, query })
    }
}

pub fn parse(text: &str) -> Vec<Smart> {
    text.lines()
        .filter_map(|line| {
            let (name, query) = line.split_once('\t')?;
            (!name.trim().is_empty() && !query.trim().is_empty()).then(|| Smart { name: name.trim().to_string(), query: query.trim().to_string() })
        })
        .collect()
}

pub fn serialize(list: &[Smart]) -> String {
    list.iter().map(|s| format!("{}\t{}\n", clean(&s.name), clean(&s.query))).collect()
}

pub fn load(path: &Path) -> Vec<Smart> {
    std::fs::read_to_string(path).map(|t| parse(&t)).unwrap_or_default()
}

pub fn save(path: &Path, list: &[Smart]) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    std::fs::write(path, serialize(list)).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_smart_mailbox_needs_a_new_name_and_a_query() {
        let one = Smart::new("Ana", "from:ana", &[]).unwrap();
        assert!(Smart::new(" ", "x", &[]).is_err());
        assert!(Smart::new("Bo", "  ", &[]).is_err());
        assert!(Smart::new("ana", "from:bo", &[one]).is_err());
    }

    #[test]
    fn smart_mailboxes_round_trip() {
        let list = vec![Smart::new("Ana", "from:ana", &[]).unwrap(), Smart::new("Invoices", "subject:invoice \"due soon\"", &[]).unwrap()];
        assert_eq!(parse(&serialize(&list)), list);
        assert!(parse("no tab here\n\tquery only\nname only\t\n").is_empty());
    }
}
