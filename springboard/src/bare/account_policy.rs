//! Enforces the account restriction policy when an app is launched. The policy file is written by the account service
//! at `POLICY_FILE` (one rule per line, see `PolicyTable::parse` in `bifrost_ui::account_policy`). A missing file
//! means no account is restricted. A file that cannot be read or parsed refuses every launch: a damaged restriction
//! must not be treated as no restriction.

use super::account::Account;
use bifrost_ui::account_policy::{Feature, Policy, PolicyTable};
use std::{
    fs, io,
    path::{Path, PathBuf},
};

/// Longest app identifier or account name a rule may name.
const MAX_TOKEN: usize = 128;

/// Checks the rules a content-limits page sends before anything is written: each app id is a plain token (no
/// whitespace, not starting with `#`), and each feature has a policy name.
pub fn validate_rules(blocked: &[&str], disabled: &[&str]) -> Result<(), String> {
    for app in blocked {
        let plain = !app.is_empty() && app.len() <= MAX_TOKEN && !app.starts_with('#') && !app.chars().any(char::is_whitespace);
        if !plain {
            return Err(format!("invalid app id {app}"));
        }
    }
    for feature in disabled {
        if Feature::from_name(feature).is_none() {
            return Err(format!("unknown feature {feature}"));
        }
    }
    Ok(())
}

/// The policy table text after `account`'s rules are replaced by `blocked` and `disabled`. An account left with no
/// rules leaves the table. The text must parse first: a damaged table is refused, never overwritten.
pub fn replace_account_rules(text: &str, account: &str, blocked: &[&str], disabled: &[&str]) -> Result<String, String> {
    validate_rules(blocked, disabled)?;
    let mut table = PolicyTable::parse(text)?;
    let mut policy = Policy::unrestricted();
    for app in blocked {
        policy.block_app(app).map_err(|e| format!("{e:?}"))?;
    }
    for feature in disabled {
        if let Some(feature) = Feature::from_name(feature) {
            policy.disable(feature);
        }
    }
    if policy == Policy::unrestricted() {
        table.remove(account);
    } else {
        table.set(account, policy).map_err(|e| format!("{e:?}"))?;
    }
    Ok(table.to_text())
}

/// Writes `text` to `path` through a temporary file and a rename, so a reader sees the old or the new table.
pub fn write_atomic(path: &Path, text: &str) -> io::Result<()> {
    let temp = path.with_extension("tmp");
    fs::write(&temp, text)?;
    fs::rename(&temp, path)
}

/// Replaces one account's rules in the table at `path` (a missing table counts as empty). Runs as root in privd.
pub fn set_account_rules(path: &Path, account: &str, blocked: &[&str], disabled: &[&str]) -> Result<(), String> {
    let current = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e.to_string()),
    };
    let next = replace_account_rules(&current, account, blocked, disabled)?;
    write_atomic(path, &next).map_err(|e| e.to_string())
}

/// Where the account service keeps the restriction rules.
pub const POLICY_FILE: &str = "/var/lib/springboard/account-policy";

/// Reads the policy table at `path`. A missing file is an empty table.
pub fn load(path: &Path) -> io::Result<PolicyTable> {
    match std::fs::read_to_string(path) {
        Ok(text) => PolicyTable::parse(&text).map_err(io::Error::other),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(PolicyTable::new()),
        Err(e) => Err(e),
    }
}

/// The launch decision for `account` and `app_id` under `table`.
pub fn decide(table: &PolicyTable, account: &str, app_id: &str) -> io::Result<()> {
    table
        .check_app(account, app_id)
        .map_err(|denial| io::Error::new(io::ErrorKind::PermissionDenied, denial.to_string()))
}

/// Checks whether the installed account may launch `app_id`, reading the policy from `path`.
pub fn check_launch_at(path: &Path, account: &str, app_id: &str) -> io::Result<()> {
    decide(&load(path)?, account, app_id)
}

/// Whether `feature` is switched off for `account` in a table read as `table`. A table that could not be read counts as
/// switched off, so a damaged restriction never grants the feature.
pub fn feature_off_in(table: io::Result<PolicyTable>, account: &str, feature: Feature) -> bool {
    match table {
        Ok(table) => table.check_feature(account, feature).is_err(),
        Err(_) => true,
    }
}

/// Whether the installed account's policy switches `feature` off, for the privacy gates and the purchase sheet. With
/// no installed account there is no policy to apply.
pub fn feature_switched_off(feature: Feature) -> bool {
    match Account::installed() {
        Ok(account) => feature_off_in(load(Path::new(POLICY_FILE)), &account.name, feature),
        Err(_) => false,
    }
}

/// Called by the launcher before it starts an app. Checks the installed account against `POLICY_FILE`.
pub fn check_launch(app_id: &str) -> io::Result<()> {
    let account = Account::installed()?;
    check_launch_at(&PathBuf::from(POLICY_FILE), &account.name, app_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_policy(name: &str, text: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("account-policy-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("account-policy");
        std::fs::write(&path, text).unwrap();
        path
    }

    #[test]
    fn missing_policy_file_allows_every_launch() {
        let path = std::env::temp_dir().join("account-policy-missing-does-not-exist");
        assert!(check_launch_at(&path, "kid", "is.bifrost.store").is_ok());
    }

    #[test]
    fn blocked_app_is_refused_for_that_account_only() {
        let path = temp_policy("blocked", "kid block is.bifrost.store\n");
        let denied = check_launch_at(&path, "kid", "is.bifrost.store").unwrap_err();
        assert_eq!(denied.kind(), io::ErrorKind::PermissionDenied);
        assert!(check_launch_at(&path, "kid", "is.bifrost.notes").is_ok());
        assert!(check_launch_at(&path, "parent", "is.bifrost.store").is_ok());
    }

    #[test]
    fn damaged_policy_file_refuses_every_launch() {
        let path = temp_policy("damaged", "kid block\n");
        assert!(check_launch_at(&path, "parent", "is.bifrost.notes").is_err());
    }

    #[test]
    fn rules_are_validated_before_anything_is_written() {
        assert!(validate_rules(&["is.bifrost.store"], &["camera", "content-sharing"]).is_ok());
        assert!(validate_rules(&["two words"], &[]).is_err());
        assert!(validate_rules(&["#comment"], &[]).is_err());
        assert!(validate_rules(&[""], &[]).is_err());
        assert!(validate_rules(&[], &["teleport"]).is_err());
    }

    #[test]
    fn replacing_one_account_keeps_the_others() {
        let text = "parent block is.bifrost.store\nkid block is.bifrost.browser\nkid disable camera\n";
        let next = replace_account_rules(text, "kid", &["is.bifrost.store"], &["location"]).unwrap();
        let table = PolicyTable::parse(&next).unwrap();
        assert_eq!(table.check_app("kid", "is.bifrost.browser"), Ok(()));
        assert!(table.check_app("kid", "is.bifrost.store").is_err());
        assert!(table.check_feature("kid", Feature::Location).is_err());
        assert!(table.check_feature("kid", Feature::Camera).is_ok());
        assert!(table.check_app("parent", "is.bifrost.store").is_err());
    }

    #[test]
    fn an_account_with_no_rules_leaves_the_table() {
        let text = "kid block is.bifrost.store\nparent disable camera\n";
        let next = replace_account_rules(text, "kid", &[], &[]).unwrap();
        assert_eq!(next, "parent disable camera\n");
    }

    #[test]
    fn a_damaged_table_is_refused_not_overwritten() {
        assert!(replace_account_rules("kid block\n", "kid", &[], &["camera"]).is_err());
    }

    #[test]
    fn set_account_rules_writes_the_file_atomically() {
        let dir = std::env::temp_dir().join(format!("account-rules-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("account-policy");
        let _ = fs::remove_file(&path);
        set_account_rules(&path, "kid", &["is.bifrost.store"], &["camera"]).unwrap();
        let table = load(&path).unwrap();
        assert!(table.check_app("kid", "is.bifrost.store").is_err());
        assert!(table.check_feature("kid", Feature::Camera).is_err());
        assert!(!path.with_extension("tmp").exists());
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn a_switched_off_feature_is_read_from_the_table_and_fails_closed() {
        let table = PolicyTable::parse("kid disable camera\n").unwrap();
        assert!(feature_off_in(Ok(table.clone()), "kid", Feature::Camera));
        assert!(!feature_off_in(Ok(table), "kid", Feature::Microphone));
        assert!(feature_off_in(Err(io::Error::other("damaged")), "kid", Feature::Location));
    }

    #[test]
    fn decide_reports_the_denial_text() {
        let table = PolicyTable::parse("kid block is.bifrost.store").unwrap();
        let err = decide(&table, "kid", "is.bifrost.store").unwrap_err();
        assert_eq!(err.to_string(), "is.bifrost.store is blocked for this account");
    }
}
