//! `security`: reads and writes generic passwords in the BifrOSt keychain (`springboard::keychain`, the encrypted
//! store behind the SDK's Keychain API) from the command line.
//!
//! The items live in one access group of that store, [`GROUP`], in the shell data directory of the user who runs the
//! command (`/var/lib/springboard` for root, otherwise the user's data directory), so a script and `security` agree on
//! them and apps do not see them unless their manifest lists the group.
//!
//! Subcommands: `add-generic-password -s SERVICE -a ACCOUNT [-w PASSWORD] [-U]`, `find-generic-password -s SERVICE
//! -a ACCOUNT [-w]`, `delete-generic-password -s SERVICE -a ACCOUNT` and `dump-keychain` (service, account and
//! time of each item, never a secret). Without `-w` the password for `add-generic-password` is read from the first
//! line of standard input, so it need not appear in the process list. `-U` updates an item that exists.
//! Refused with a message: certificates, identities, internet passwords (`*-internet-password`), `unlock-keychain`,
//! `create-keychain` and the other keychain-file management subcommands: there is one store, always unlocked for its
//! owner.

use springboard::keychain::{Error as KeychainError, Store};
use std::path::PathBuf;

pub const USAGE: &str = "usage: security add-generic-password -s service -a account [-w password] [-U] | find-generic-password -s service -a account [-w] | delete-generic-password -s service -a account | dump-keychain";
/// The keychain access group the command's items are kept in.
pub const GROUP: &str = "is.bifrost.security";

/// What the command line asks for.
#[derive(Debug, PartialEq, Eq)]
pub enum Action {
    Help,
    Add { service: String, account: String, password: Option<String>, update: bool },
    Find { service: String, account: String, password_only: bool },
    Delete { service: String, account: String },
    Dump,
}

/// Reads the arguments.
pub fn parse(args: &[String]) -> Result<Action, String> {
    let Some((sub, rest)) = args.split_first() else { return Ok(Action::Help) };
    match sub.as_str() {
        "-h" | "-help" | "--help" | "help" => return Ok(Action::Help),
        "dump-keychain" => return Ok(Action::Dump),
        "add-generic-password" | "find-generic-password" | "delete-generic-password" => {}
        other => return Err(format!("{other} is not supported: only generic passwords in the one BifrOSt keychain")),
    }
    let (mut service, mut account, mut password, mut update, mut w_flag) = (None, None, None, false, false);
    let mut i = 0;
    while i < rest.len() {
        let value = |i: usize, flag: &str| rest.get(i + 1).filter(|v| !v.starts_with('-')).cloned().ok_or(format!("{flag} needs a value"));
        match rest[i].as_str() {
            "-s" => {
                service = Some(value(i, "-s")?);
                i += 1;
            }
            "-a" => {
                account = Some(value(i, "-a")?);
                i += 1;
            }
            "-w" => {
                w_flag = true;
                if sub == "add-generic-password" {
                    if let Some(v) = rest.get(i + 1).filter(|v| !v.starts_with('-')) {
                        password = Some(v.clone());
                        i += 1;
                    }
                }
            }
            "-U" => update = true,
            "-g" | "-l" | "-D" | "-j" | "-c" | "-C" | "-T" | "-A" | "-X" | "-t" | "-r" => {
                return Err(format!("{} is not supported", rest[i]));
            }
            other => return Err(format!("unknown option {other}")),
        }
        i += 1;
    }
    let (Some(service), Some(account)) = (service, account) else { return Err("-s service and -a account are required".into()) };
    Ok(match sub.as_str() {
        "add-generic-password" => Action::Add { service, account, password, update },
        "find-generic-password" => Action::Find { service, account, password_only: w_flag },
        _ => Action::Delete { service, account },
    })
}

/// The shell data directory the keychain lives under, by the same rule the shell uses for its other files.
pub fn data_root(root: bool, xdg_data_home: Option<&str>, home: Option<&str>) -> PathBuf {
    PathBuf::from(springboard::bare::platform::data_dir(root, xdg_data_home, home))
}

/// One line per item: `service<TAB>account<TAB>modified-unix-time`. No secret is read.
pub fn dump_text(store: &Store) -> Result<String, KeychainError> {
    Ok(store.list()?.iter().map(|l| format!("{}\t{}\t{}\n", l.service, l.account, l.modified)).collect())
}

/// What `find-generic-password` prints without `-w`: the attributes, never the password.
pub fn attributes_text(service: &str, account: &str) -> String {
    format!("keychain: \"{GROUP}\"\nattributes:\n    \"acct\"<blob>=\"{account}\"\n    \"svce\"<blob>=\"{service}\"\n")
}

/// Runs one action against `store` and returns what to print. `stdin_password` is the first line of standard input,
/// used when `add-generic-password` has no `-w` value.
pub fn run(store: &Store, action: &Action, stdin_password: Option<&str>) -> Result<String, String> {
    let fail = |e: KeychainError| match e {
        KeychainError::ItemNotFound => "The specified item could not be found in the keychain.".to_string(),
        KeychainError::DuplicateItem => "The specified item already exists in the keychain (use -U to update).".to_string(),
        other => other.to_string(),
    };
    match action {
        Action::Help => Ok(format!("{USAGE}\n")),
        Action::Dump => dump_text(store).map_err(fail),
        Action::Add { service, account, password, update } => {
            let secret = password.as_deref().or(stdin_password).ok_or("no password: pass -w password or give it on standard input")?;
            match store.add(service, account, secret.as_bytes()) {
                Err(KeychainError::DuplicateItem) if *update => store.update(service, account, secret.as_bytes()).map_err(fail)?,
                other => other.map_err(fail)?,
            }
            Ok(String::new())
        }
        Action::Find { service, account, password_only } => {
            let secret = store.get(service, account).map_err(fail)?;
            if *password_only {
                Ok(format!("{}\n", String::from_utf8_lossy(&secret)))
            } else {
                Ok(attributes_text(service, account))
            }
        }
        Action::Delete { service, account } => store.delete(service, account).map(|_| String::new()).map_err(fail),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    fn store(tag: &str) -> (Store, PathBuf) {
        let dir = std::env::temp_dir().join(format!("bifrost-security-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("cli.kc");
        (Store::plain(path), dir)
    }

    #[test]
    fn subcommands_parse_with_their_flags() {
        assert_eq!(
            parse(&args(&["add-generic-password", "-s", "mail", "-a", "me", "-w", "pw", "-U"])),
            Ok(Action::Add { service: "mail".into(), account: "me".into(), password: Some("pw".into()), update: true })
        );
        assert_eq!(
            parse(&args(&["find-generic-password", "-a", "me", "-s", "mail", "-w"])),
            Ok(Action::Find { service: "mail".into(), account: "me".into(), password_only: true })
        );
        assert_eq!(parse(&args(&["delete-generic-password", "-s", "mail", "-a", "me"])), Ok(Action::Delete { service: "mail".into(), account: "me".into() }));
        assert_eq!(parse(&args(&["dump-keychain"])), Ok(Action::Dump));
        assert_eq!(parse(&[]), Ok(Action::Help));
        assert!(parse(&args(&["find-generic-password", "-s", "mail"])).unwrap_err().contains("required"));
        assert!(parse(&args(&["unlock-keychain"])).unwrap_err().contains("not supported"));
        assert!(parse(&args(&["add-generic-password", "-s", "m", "-a", "a", "-T", "x"])).unwrap_err().contains("-T is not supported"));
    }

    #[test]
    fn a_password_is_added_found_updated_and_deleted() {
        let (store, dir) = store("round");
        let add = |pw: Option<&str>, update: bool| Action::Add { service: "mail".into(), account: "me".into(), password: pw.map(String::from), update };
        run(&store, &add(Some("one"), false), None).unwrap();
        let find = Action::Find { service: "mail".into(), account: "me".into(), password_only: true };
        assert_eq!(run(&store, &find, None).unwrap(), "one\n");
        assert!(run(&store, &add(Some("two"), false), None).unwrap_err().contains("already exists"));
        run(&store, &add(Some("two"), true), None).unwrap();
        assert_eq!(run(&store, &find, None).unwrap(), "two\n");
        let attrs = run(&store, &Action::Find { service: "mail".into(), account: "me".into(), password_only: false }, None).unwrap();
        assert!(attrs.contains("\"svce\"<blob>=\"mail\"") && !attrs.contains("two"), "{attrs}");
        let dump = run(&store, &Action::Dump, None).unwrap();
        assert!(dump.starts_with("mail\tme\t") && !dump.contains("two"), "{dump}");
        run(&store, &Action::Delete { service: "mail".into(), account: "me".into() }, None).unwrap();
        assert!(run(&store, &find, None).unwrap_err().contains("could not be found"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_missing_password_comes_from_standard_input_or_is_an_error() {
        let (store, dir) = store("stdin");
        let add = Action::Add { service: "s".into(), account: "a".into(), password: None, update: false };
        assert!(run(&store, &add, None).unwrap_err().contains("no password"));
        run(&store, &add, Some("from-stdin")).unwrap();
        assert_eq!(store.get("s", "a").unwrap(), b"from-stdin");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_store_is_the_group_file_in_the_shell_data_directory() {
        assert_eq!(data_root(true, None, None), PathBuf::from("/var/lib/springboard"));
        assert_eq!(data_root(false, None, Some("/home/me")), PathBuf::from("/home/me/.local/share/springboard"));
        let dir = std::env::temp_dir().join(format!("bifrost-security-group-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let first = Store::open_group(&dir, GROUP).unwrap();
        first.add("svc", "acct", b"sekret").unwrap();
        let again = Store::open_group(&dir, GROUP).unwrap();
        assert_eq!(again.get("svc", "acct").unwrap(), b"sekret");
        let raw = std::fs::read(dir.join("keychain/groups").join(format!("{GROUP}.kc"))).unwrap();
        assert!(!raw.windows(6).any(|w| w == b"sekret"), "the file holds the secret encrypted");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
