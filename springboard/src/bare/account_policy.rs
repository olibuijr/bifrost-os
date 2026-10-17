//! Enforces the account restriction policy when an app is launched. The policy file is written by the account service
//! at `POLICY_FILE` (one rule per line, see `PolicyTable::parse` in `bifrost_ui::account_policy`). A missing file
//! means no account is restricted. A file that cannot be read or parsed refuses every launch: a damaged restriction
//! must not be treated as no restriction.

use super::account::Account;
use bifrost_ui::account_policy::PolicyTable;
use std::{
    io,
    path::{Path, PathBuf},
};

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
    fn decide_reports_the_denial_text() {
        let table = PolicyTable::parse("kid block is.bifrost.store").unwrap();
        let err = decide(&table, "kid", "is.bifrost.store").unwrap_err();
        assert_eq!(err.to_string(), "is.bifrost.store is blocked for this account");
    }
}
