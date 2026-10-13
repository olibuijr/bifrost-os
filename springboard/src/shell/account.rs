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

/// The account picture, if the user has one: `~/.face` (the freedesktop convention) or `avatar.png` in the data dir.
pub fn avatar_path() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from)?;
    [home.join(".face"), home.join(".local/share/springboard/avatar.png")].into_iter().find(|p| p.is_file())
}

/// First-run state: an account whose password is unset or locked (`passwd -S` shows NP or L) asks for one.
pub fn password_status_needs_set(passwd_s_line: &str) -> bool {
    matches!(passwd_s_line.split_whitespace().nth(1), Some("NP" | "L" | "LK"))
}

/// Why a password change is refused before privd sees it (the same length rule privd enforces), or None
/// when it may be sent: the new password is long enough, typed twice the same, and not the old one.
pub fn password_change_problem(old: &str, new: &str, again: &str) -> Option<&'static str> {
    if new.len() < crate::privd::auth::MIN_PASSWORD {
        Some("The new password needs at least 8 characters.")
    } else if new.contains(['\n', ':', '\0']) {
        Some("The password cannot contain a colon or a line break.")
    } else if new != again {
        Some("The two passwords do not match.")
    } else if new == old {
        Some("Choose a password that is different from the current one.")
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passwd_status_detects_missing_passwords() {
        assert!(password_status_needs_set("ada NP 2026-10-01 0 99999 7 -1"));
        assert!(password_status_needs_set("ada L 2026-10-01 0 99999 7 -1"));
        assert!(!password_status_needs_set("ada P 2026-10-01 0 99999 7 -1"));
    }

    #[test]
    fn parses_the_installer_file() {
        let a = parse("full_name=Ada Lovelace\nusername=ada\n").unwrap();
        assert_eq!((a.full_name.as_str(), a.username.as_str()), ("Ada Lovelace", "ada"));
        assert_eq!(parse("username=ada").unwrap().full_name, "ada");
        assert!(parse("full_name=x").is_none());
    }

    #[test]
    fn password_change_checks_before_privd() {
        assert_eq!(password_change_problem("old-pass", "short", "short"), Some("The new password needs at least 8 characters."));
        assert_eq!(password_change_problem("old-pass", "a:bcdefgh", "a:bcdefgh"), Some("The password cannot contain a colon or a line break."));
        assert_eq!(password_change_problem("old-pass", "longenough", "longenugh"), Some("The two passwords do not match."));
        assert_eq!(password_change_problem("longenough", "longenough", "longenough"), Some("Choose a password that is different from the current one."));
        assert_eq!(password_change_problem("old-pass", "longenough", "longenough"), None);
    }
}
