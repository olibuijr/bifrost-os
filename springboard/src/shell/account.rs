//! The local account created by the installer: `account` in the shell's data directory, written as
//! `full_name=...` / `username=...` lines. Settings shows it where bifrost shows the ID card.

use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Account {
    pub full_name: String,
    pub username: String,
}

fn candidates() -> Vec<PathBuf> {
    let mut v = vec![PathBuf::from("/var/lib/springboard/account")];
    if let Some(x) = std::env::var_os("XDG_DATA_HOME") {
        v.push(PathBuf::from(x).join("springboard/account"));
    }
    if let Some(h) = std::env::var_os("HOME") {
        v.push(PathBuf::from(h).join(".local/share/springboard/account"));
    }
    v
}

pub fn parse(text: &str) -> Option<Account> {
    let get = |key: &str| text.lines().find_map(|l| l.strip_prefix(key)?.strip_prefix('=')).map(|v| v.trim().to_string()).filter(|v| !v.is_empty());
    let username = get("username")?;
    Some(Account { full_name: get("full_name").unwrap_or_else(|| username.clone()), username })
}

/// The installed account, or a generic local owner when the file is missing.
pub fn load() -> Account {
    candidates()
        .iter()
        .find_map(|p| std::fs::read_to_string(p).ok().and_then(|t| parse(&t)))
        .unwrap_or_else(|| Account { full_name: "BifrOSt User".into(), username: std::env::var("USER").unwrap_or_else(|_| "owner".into()) })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_installer_file() {
        let a = parse("full_name=Ada Lovelace\nusername=ada\n").unwrap();
        assert_eq!((a.full_name.as_str(), a.username.as_str()), ("Ada Lovelace", "ada"));
        assert_eq!(parse("username=ada").unwrap().full_name, "ada");
        assert!(parse("full_name=x").is_none());
    }
}
