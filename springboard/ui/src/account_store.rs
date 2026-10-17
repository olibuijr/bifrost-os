//! The shared accounts store file (`accounts.ini`): the account types, their access grants and the accounts saved in
//! them. The SDK's `AccountStore` (`bifrost_app::accounts`) keeps this file current; the shell reads it here too, for
//! Settings > Internet Accounts. The reader and writer live in this crate so that both sides share one format.
//!
//! The file holds no credential and no password. Sections are `[store]` (the next identifier number), `[type]` and
//! `[account]`, each followed by `key = value` lines. Values escape backslash, newline and carriage return, so every
//! value fits on one line. A reader skips unknown sections and keys, so an older build reads a newer file.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// The file name of the store the apps share, inside the `bifrost` directory of the user's data directory.
pub const FILE_NAME: &str = "accounts.ini";

const FILE_HEADER: &str = "# BifrOSt account store. Credentials are not written here; the app keeps them in its keychain.\n";

/// One account type as the file holds it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TypeEntry {
    pub identifier: String,
    pub description: String,
    /// `accessGranted`: whether the person let apps use accounts of this type.
    pub access_granted: bool,
}

/// One saved account as the file holds it. Its credential is kept in the keychain, not here.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AccountEntry {
    pub identifier: String,
    pub account_type: String,
    pub username: String,
    pub user_full_name: String,
    pub account_description: String,
}

/// An app's request for an account type that the person refused. The app asks again only after the person clears it
/// in Settings > Internet Accounts.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RefusalEntry {
    pub app: String,
    pub account_type: String,
}

/// Everything the file holds.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Contents {
    /// The number the next saved account takes (`account-N`). At least one above the highest number in use.
    pub next_id: u64,
    pub types: Vec<TypeEntry>,
    pub accounts: Vec<AccountEntry>,
    pub refusals: Vec<RefusalEntry>,
}

impl Contents {
    /// The type with this identifier, if the store holds it.
    pub fn account_type(&self, identifier: &str) -> Option<&TypeEntry> {
        self.types.iter().find(|t| t.identifier == identifier)
    }

    /// The description of an account's type. A type the store does not hold shows its identifier.
    pub fn type_description<'a>(&'a self, account_type: &'a str) -> &'a str {
        self.account_type(account_type).map(|t| t.description.as_str()).unwrap_or(account_type)
    }

    /// Whether the person refused this app's request for this type.
    pub fn refused(&self, app: &str, account_type: &str) -> bool {
        self.refusals.iter().any(|r| r.app == app && r.account_type == account_type)
    }

    /// The accounts saved with this type, in file order.
    pub fn accounts_with_type(&self, account_type: &str) -> Vec<&AccountEntry> {
        self.accounts.iter().filter(|a| a.account_type == account_type).collect()
    }
}

/// The path of the shared store for the given environment values: `$XDG_DATA_HOME/bifrost/accounts.ini`, or under
/// `$HOME/.local/share` when `XDG_DATA_HOME` is unset or empty. `None` when neither is set.
pub fn path_from(xdg_data_home: Option<std::ffi::OsString>, home: Option<std::ffi::OsString>) -> Option<PathBuf> {
    let base = xdg_data_home
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| home.filter(|h| !h.is_empty()).map(|h| PathBuf::from(h).join(".local/share")))?;
    Some(base.join("bifrost").join(FILE_NAME))
}

/// The path of the shared store in this session's environment.
pub fn store_path() -> Option<PathBuf> {
    path_from(std::env::var_os("XDG_DATA_HOME"), std::env::var_os("HOME"))
}

/// Reads the store at `path`. A missing file is an empty store; a file that cannot be read or parsed is an error.
pub fn read(path: &Path) -> io::Result<Contents> {
    match fs::read_to_string(path) {
        Ok(text) => decode(&text),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Contents::default()),
        Err(e) => Err(e),
    }
}

/// Writes the store to `path` through a temporary file and a rename, creating the folder when needed.
pub fn write(path: &Path, c: &Contents) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("ini.tmp");
    fs::write(&tmp, encode(c))?;
    fs::rename(&tmp, path)
}

/// Records the person's answer to an access request for an account type in the store at `path` (`accessGranted`).
/// Returns false, and writes nothing, when the store does not hold that type.
pub fn set_access(path: &Path, identifier: &str, granted: bool) -> io::Result<bool> {
    let mut c = read(path)?;
    let Some(t) = c.types.iter_mut().find(|t| t.identifier == identifier) else { return Ok(false) };
    t.access_granted = granted;
    write(path, &c)?;
    Ok(true)
}

/// Records that the person refused `app` access to accounts of `account_type`. Recording it twice keeps one entry.
pub fn add_refusal(path: &Path, app: &str, account_type: &str) -> io::Result<()> {
    let mut c = read(path)?;
    if !c.refused(app, account_type) {
        c.refusals.push(RefusalEntry { app: app.into(), account_type: account_type.into() });
        write(path, &c)?;
    }
    Ok(())
}

/// Clears a refusal so the app may ask again. Returns whether there was one.
pub fn remove_refusal(path: &Path, app: &str, account_type: &str) -> io::Result<bool> {
    let mut c = read(path)?;
    let before = c.refusals.len();
    c.refusals.retain(|r| !(r.app == app && r.account_type == account_type));
    let removed = c.refusals.len() != before;
    if removed {
        write(path, &c)?;
    }
    Ok(removed)
}

/// Serialises the store. Sections are `[store]`, `[type]`, `[account]` and `[refusal]`.
pub fn encode(c: &Contents) -> String {
    let mut out = String::from(FILE_HEADER);
    out.push_str(&format!("\n[store]\nnext-id = {}\n", c.next_id));
    for t in &c.types {
        out.push_str("\n[type]\n");
        write_field(&mut out, "identifier", &t.identifier);
        write_field(&mut out, "description", &t.description);
        write_field(&mut out, "access-granted", if t.access_granted { "true" } else { "false" });
    }
    for a in &c.accounts {
        out.push_str("\n[account]\n");
        write_field(&mut out, "id", &a.identifier);
        write_field(&mut out, "type", &a.account_type);
        write_field(&mut out, "username", &a.username);
        write_field(&mut out, "full-name", &a.user_full_name);
        write_field(&mut out, "description", &a.account_description);
    }
    for r in &c.refusals {
        out.push_str("\n[refusal]\n");
        write_field(&mut out, "app", &r.app);
        write_field(&mut out, "type", &r.account_type);
    }
    out
}

fn write_field(out: &mut String, key: &str, value: &str) {
    out.push_str(key);
    out.push_str(" = ");
    for c in value.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            c => out.push(c),
        }
    }
    out.push('\n');
}

fn unescape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some('\\') => out.push('\\'),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

fn invalid_data() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "account store file is malformed")
}

type Record = (String, Vec<(String, String)>);

/// Splits the text into sections. Comments and blank lines are skipped; a field outside any section is malformed.
fn records(text: &str) -> io::Result<Vec<Record>> {
    let mut out: Vec<Record> = Vec::new();
    for raw in text.lines() {
        let line = raw.trim_start();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            out.push((name.trim().to_string(), Vec::new()));
            continue;
        }
        let (key, value) = line.split_once('=').ok_or_else(invalid_data)?;
        let section = out.last_mut().ok_or_else(invalid_data)?;
        let value = value.strip_prefix(' ').unwrap_or(value);
        section.1.push((key.trim().to_string(), unescape(value)));
    }
    Ok(out)
}

fn field_of(fields: &[(String, String)], key: &str) -> String {
    fields.iter().rev().find(|(k, _)| k.as_str() == key).map(|(_, v)| v.clone()).unwrap_or_default()
}

/// Reads a store written by [`encode`]. The next identifier is at least one above the highest `account-N` in the file,
/// so a reopened store never reuses one.
pub fn decode(text: &str) -> io::Result<Contents> {
    let mut data = Contents::default();
    let mut stored_next = 0u64;
    let mut highest = 0u64;
    for (name, fields) in records(text)? {
        match name.as_str() {
            "store" => stored_next = field_of(&fields, "next-id").parse().map_err(|_| invalid_data())?,
            "type" => {
                let identifier = field_of(&fields, "identifier");
                if identifier.is_empty() {
                    return Err(invalid_data());
                }
                let access_granted = field_of(&fields, "access-granted") == "true";
                data.types.push(TypeEntry { identifier, description: field_of(&fields, "description"), access_granted });
            }
            "account" => {
                let identifier = field_of(&fields, "id");
                if identifier.is_empty() {
                    return Err(invalid_data());
                }
                if let Some(n) = identifier.strip_prefix("account-").and_then(|n| n.parse::<u64>().ok()) {
                    highest = highest.max(n);
                }
                data.accounts.push(AccountEntry {
                    identifier,
                    account_type: field_of(&fields, "type"),
                    username: field_of(&fields, "username"),
                    user_full_name: field_of(&fields, "full-name"),
                    account_description: field_of(&fields, "description"),
                });
            }
            "refusal" => {
                let app = field_of(&fields, "app");
                let account_type = field_of(&fields, "type");
                if app.is_empty() || account_type.is_empty() {
                    return Err(invalid_data());
                }
                data.refusals.push(RefusalEntry { app, account_type });
            }
            _ => {}
        }
    }
    data.next_id = stored_next.max(highest);
    Ok(data)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;

    fn sample() -> Contents {
        Contents {
            next_id: 3,
            types: vec![TypeEntry { identifier: "bifrost.mail".into(), description: "Mail".into(), access_granted: true }],
            accounts: vec![AccountEntry {
                identifier: "account-2".into(),
                account_type: "bifrost.mail".into(),
                username: "ann".into(),
                user_full_name: "Ann\\Lee".into(),
                account_description: "two\nlines".into(),
            }],
            refusals: vec![RefusalEntry { app: "app.notes".into(), account_type: "bifrost.mail".into() }],
        }
    }

    #[test]
    fn encode_then_decode_round_trips_every_field() {
        let c = sample();
        assert_eq!(decode(&encode(&c)).unwrap(), c);
    }

    #[test]
    fn the_next_identifier_is_above_the_highest_saved_one() {
        let text = "[store]\nnext-id = 1\n\n[account]\nid = account-7\ntype = bifrost.mail\n";
        assert_eq!(decode(text).unwrap().next_id, 8);
    }

    #[test]
    fn unknown_sections_and_keys_are_skipped_and_bad_lines_are_errors() {
        let text = "[future]\nx = 1\n\n[type]\nidentifier = bifrost.cloud\nnew-key = y\n";
        let c = decode(text).unwrap();
        assert_eq!(c.types.len(), 1);
        assert_eq!(decode("no section here = 1\n").unwrap_err().kind(), io::ErrorKind::InvalidData);
        assert_eq!(decode("[type]\ndescription = nameless\n").unwrap_err().kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn type_descriptions_fall_back_to_the_identifier() {
        let c = sample();
        assert_eq!(c.type_description("bifrost.mail"), "Mail");
        assert_eq!(c.type_description("unknown.type"), "unknown.type");
        assert_eq!(c.accounts_with_type("bifrost.mail").len(), 1);
        assert!(c.accounts_with_type("bifrost.cloud").is_empty());
    }

    #[test]
    fn the_shared_path_follows_the_data_directory_then_home() {
        assert_eq!(path_from(Some(OsString::from("/data")), Some(OsString::from("/home/u"))), Some(PathBuf::from("/data/bifrost/accounts.ini")));
        assert_eq!(path_from(Some(OsString::new()), Some(OsString::from("/home/u"))), Some(PathBuf::from("/home/u/.local/share/bifrost/accounts.ini")));
        assert_eq!(path_from(None, None), None);
    }

    #[test]
    fn an_access_answer_is_written_for_a_known_type_only() {
        let dir = std::env::temp_dir().join(format!("bifrost-account-access-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let path = dir.join(FILE_NAME);
        write(&path, &sample()).unwrap();
        assert_eq!(set_access(&path, "bifrost.mail", true).unwrap(), true);
        assert!(read(&path).unwrap().account_type("bifrost.mail").unwrap().access_granted);
        assert_eq!(set_access(&path, "bifrost.mail", false).unwrap(), true);
        assert!(!read(&path).unwrap().account_type("bifrost.mail").unwrap().access_granted);
        assert_eq!(set_access(&path, "unknown.type", true).unwrap(), false);
        assert_eq!(read(&path).unwrap().types.len(), 1, "an unknown type writes nothing");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_refusal_is_kept_until_it_is_removed() {
        let dir = std::env::temp_dir().join(format!("bifrost-account-refusal-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let path = dir.join(FILE_NAME);
        write(&path, &sample()).unwrap();
        add_refusal(&path, "app.other", "bifrost.caldav").unwrap();
        add_refusal(&path, "app.other", "bifrost.caldav").unwrap();
        let c = read(&path).unwrap();
        assert!(c.refused("app.other", "bifrost.caldav"));
        assert_eq!(c.refusals.len(), 2, "one entry per refusal");
        assert!(remove_refusal(&path, "app.other", "bifrost.caldav").unwrap());
        assert!(!read(&path).unwrap().refused("app.other", "bifrost.caldav"));
        assert!(!remove_refusal(&path, "app.other", "bifrost.caldav").unwrap(), "clearing twice finds nothing");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_missing_file_is_an_empty_store() {
        let path = std::env::temp_dir().join(format!("bifrost-account-store-missing-{}.ini", std::process::id()));
        let _ = fs::remove_file(&path);
        assert_eq!(read(&path).unwrap(), Contents::default());
    }
}
