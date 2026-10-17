//! The BifrOSt Account session on this device, kept in the `is.bifrost.account` keychain access group.
//!
//! The session is one item: service [`SERVICE`], account [`ACCOUNT`], value a small JSON object with the device bearer
//! token, the account id, the cloud server address and the expiry (Unix seconds, or null when the answer gave none). The
//! shell writes it: sign-in stores it, a refresh replaces it ([`store`]) and sign-out deletes it ([`sign_out`]). Apps
//! read it through the `keychain-groups` service, and only first-party apps are granted the group (see
//! `shell::services::keychain_groups::groups_of`).
//!
//! Before this item existed the session was a plain file, `bifrost/cloud_session` in the user's data directory, with a
//! `token=` line. [`migrate_file`] moves that file into the keychain once and deletes it.
//!
//! The token is a secret: [`Session`] prints without it, and nothing here logs it. The storage is a [`Vault`], so the
//! lifecycle is tested with a fake and runs on the real keychain group store ([`GroupVault`]) in production.

use crate::keychain::{Error, Store};
use bifrost_core::json::{self, Json};
use std::fmt;
use std::path::{Path, PathBuf};

/// The keychain access group the session lives in.
pub const GROUP: &str = "is.bifrost.account";
/// The item's service and account inside the group.
pub const SERVICE: &str = "bc_session";
pub const ACCOUNT: &str = "session";
/// The cloud API the session belongs to, when the stored item says nothing else (a migrated file).
pub const SERVER: &str = "https://bifrost.olibuijr.com/api/cloud";
/// The plain file a session used to live in, under the user's `bifrost` data directory.
pub const FILE: &str = "cloud_session";
/// The stored value's format version.
const VERSION: f64 = 1.0;

/// The BifrOSt Account session of this device. The token is printed as `..`, never in clear.
#[derive(Clone, PartialEq, Eq)]
pub struct Session {
    /// The device bearer token (letters and digits only).
    pub token: String,
    /// The account the token belongs to; empty when a migrated file did not say.
    pub account_id: String,
    /// The `https` address of the cloud API.
    pub server: String,
    /// Unix seconds when the session ends; `None` when the sign-in answer gave no lifetime.
    pub expires: Option<i64>,
}

impl fmt::Debug for Session {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Session").field("account_id", &self.account_id).field("server", &self.server).field("expires", &self.expires).field("token", &"<redacted>").finish()
    }
}

impl Session {
    /// A session, when the token is letters and digits, the server an `https` address with no spaces. The account
    /// id may be empty.
    pub fn new(token: &str, account_id: &str, server: &str, expires: Option<i64>) -> Option<Session> {
        let token_ok = !token.is_empty() && token.bytes().all(|b| b.is_ascii_alphanumeric());
        let server_ok = server.strip_prefix("https://").is_some_and(|host| !host.is_empty()) && !server.contains(char::is_whitespace);
        (token_ok && server_ok).then(|| Session { token: token.to_string(), account_id: account_id.to_string(), server: server.to_string(), expires })
    }

    /// The stored value: a JSON object `{"v":1,"token":..,"account":..,"server":..,"expires":..}`.
    pub fn encode(&self) -> Vec<u8> {
        let expires = self.expires.map_or(Json::Null, |t| Json::Num(t as f64));
        Json::Obj(vec![
            ("v".into(), Json::Num(VERSION)),
            ("token".into(), Json::Str(self.token.clone())),
            ("account".into(), Json::Str(self.account_id.clone())),
            ("server".into(), Json::Str(self.server.clone())),
            ("expires".into(), expires),
        ])
        .encode()
        .into_bytes()
    }

    /// Reads a value written by [`Session::encode`]. Anything else (another version, bad fields) is `None`.
    pub fn decode(bytes: &[u8]) -> Option<Session> {
        let parsed = json::parse(std::str::from_utf8(bytes).ok()?).ok()?;
        if parsed.get("v")?.number()? != VERSION {
            return None;
        }
        let expires = match parsed.get("expires")? {
            Json::Null => None,
            n => Some(n.number()? as i64),
        };
        Session::new(&parsed.str_field("token")?, &parsed.str_field("account")?, &parsed.str_field("server")?, expires)
    }
}

/// Where the session item is kept. [`GroupVault`] is the keychain group; tests use a fake.
pub trait Vault {
    /// The stored value, or `None` when there is no item.
    fn read(&self) -> Result<Option<Vec<u8>>, Error>;
    /// Stores the value, adding the item or replacing its value.
    fn write(&self, value: &[u8]) -> Result<(), Error>;
    /// Deletes the item. Deleting an item that is not there is not an error.
    fn remove(&self) -> Result<(), Error>;
}

/// The session item in the `is.bifrost.account` group of the shell data directory (`keychain::Store::open_group`).
pub struct GroupVault {
    root: PathBuf,
}

impl GroupVault {
    pub fn new(root: &Path) -> GroupVault {
        GroupVault { root: root.to_path_buf() }
    }
}

impl Vault for GroupVault {
    fn read(&self) -> Result<Option<Vec<u8>>, Error> {
        match Store::open_group(&self.root, GROUP).and_then(|s| s.get(SERVICE, ACCOUNT)) {
            Ok(value) => Ok(Some(value)),
            Err(Error::ItemNotFound) => Ok(None),
            Err(e) => Err(e),
        }
    }

    fn write(&self, value: &[u8]) -> Result<(), Error> {
        let store = Store::open_group(&self.root, GROUP)?;
        match store.update(SERVICE, ACCOUNT, value) {
            Err(Error::ItemNotFound) => store.add(SERVICE, ACCOUNT, value),
            other => other,
        }
    }

    fn remove(&self) -> Result<(), Error> {
        match Store::open_group(&self.root, GROUP).and_then(|s| s.delete(SERVICE, ACCOUNT)) {
            Ok(()) | Err(Error::ItemNotFound) => Ok(()),
            Err(e) => Err(e),
        }
    }
}

/// The session the vault holds, or `None` when signed out. A value that does not decode counts as signed out.
pub fn load(vault: &dyn Vault) -> Result<Option<Session>, Error> {
    Ok(vault.read()?.and_then(|bytes| Session::decode(&bytes)))
}

/// Sign-in and refresh: the session replaces whatever the item held.
pub fn store(vault: &dyn Vault, session: &Session) -> Result<(), Error> {
    vault.write(&session.encode())
}

/// Sign-out: the item is deleted. The server side ends with `cloud_account::sign_out_request`.
pub fn sign_out(vault: &dyn Vault) -> Result<(), Error> {
    vault.remove()
}

/// The plain session file's path for a user: `$XDG_DATA_HOME/bifrost/cloud_session`, or under `~/.local/share`.
pub fn file_path(xdg_data_home: Option<&str>, home: Option<&str>) -> Option<PathBuf> {
    let base = xdg_data_home
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| home.filter(|h| !h.is_empty()).map(|h| PathBuf::from(h).join(".local/share")))?;
    Some(base.join("bifrost").join(FILE))
}

/// The token of a plain session file: the first `token=` line, trimmed.
fn file_token(text: &str) -> Option<String> {
    text.lines().find_map(|l| l.trim().strip_prefix("token=")).map(|t| t.trim().to_string())
}

/// Moves a plain session file into the vault, once. When the vault already holds a session it keeps that one (the
/// keychain is written by the shell, so it is the newer copy). Then the file is deleted. Returns whether a file was
/// removed; a missing file is `Ok(false)`. If the vault refuses the write the file stays, so the next start retries.
pub fn migrate_file(vault: &dyn Vault, file: &Path) -> Result<bool, Error> {
    let text = match std::fs::read_to_string(file) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(Error::Io(e.to_string())),
    };
    if vault.read()?.is_none() {
        if let Some(session) = file_token(&text).and_then(|t| Session::new(&t, "", SERVER, None)) {
            store(vault, &session)?;
        }
    }
    std::fs::remove_file(file).map_err(|e| Error::Io(e.to_string()))?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    /// An in-memory vault. `refuse_writes` makes every write fail, as a full or unreadable store would.
    #[derive(Default)]
    struct Fake {
        item: RefCell<Option<Vec<u8>>>,
        refuse_writes: bool,
    }

    impl Vault for Fake {
        fn read(&self) -> Result<Option<Vec<u8>>, Error> {
            Ok(self.item.borrow().clone())
        }
        fn write(&self, value: &[u8]) -> Result<(), Error> {
            if self.refuse_writes {
                return Err(Error::Io("disk full".into()));
            }
            *self.item.borrow_mut() = Some(value.to_vec());
            Ok(())
        }
        fn remove(&self) -> Result<(), Error> {
            self.item.borrow_mut().take();
            Ok(())
        }
    }

    fn session(token: &str) -> Session {
        Session::new(token, "acc1", SERVER, Some(1_800_000_000)).unwrap()
    }

    fn dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("bifrost-cs-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn the_lifecycle_stores_refreshes_and_deletes_the_session() {
        let vault = Fake::default();
        assert_eq!(load(&vault), Ok(None), "signed out before any sign-in");
        store(&vault, &session("tok1")).unwrap();
        assert_eq!(load(&vault), Ok(Some(session("tok1"))));
        // a refresh replaces the token and keeps one item
        store(&vault, &session("tok2")).unwrap();
        assert_eq!(load(&vault).unwrap().unwrap().token, "tok2");
        sign_out(&vault).unwrap();
        assert_eq!(load(&vault), Ok(None), "signed out after sign-out");
        sign_out(&vault).unwrap();
    }

    #[test]
    fn the_stored_value_round_trips_with_and_without_an_expiry() {
        let no_expiry = Session::new("abc", "", SERVER, None).unwrap();
        assert_eq!(Session::decode(&no_expiry.encode()), Some(no_expiry));
        assert_eq!(Session::decode(&session("abc").encode()), Some(session("abc")));
    }

    #[test]
    fn a_bad_stored_value_reads_as_signed_out() {
        assert_eq!(Session::decode(b"token=abc"), None, "the old file format is not the item format");
        assert_eq!(Session::decode(br#"{"v":2,"token":"a","account":"","server":"https://x","expires":null}"#), None, "another version");
        assert_eq!(Session::decode(br#"{"v":1,"token":"a b","account":"","server":"https://x","expires":null}"#), None, "a token with a space");
        assert_eq!(Session::decode(br#"{"v":1,"token":"abc","account":"","server":"http://x","expires":null}"#), None, "a plain http server");
        let vault = Fake::default();
        *vault.item.borrow_mut() = Some(b"garbage".to_vec());
        assert_eq!(load(&vault), Ok(None));
    }

    #[test]
    fn the_session_is_never_printed_with_its_token() {
        let text = format!("{:?}", session("supersecret"));
        assert!(!text.contains("supersecret"), "Debug must not show the token: {text}");
        assert!(text.contains("acc1"));
    }

    #[test]
    fn a_plain_file_migrates_into_the_vault_once_and_is_deleted() {
        let d = dir("migrate");
        let file = d.join(FILE);
        std::fs::write(&file, "token=abc123\n").unwrap();
        let vault = Fake::default();
        assert_eq!(migrate_file(&vault, &file), Ok(true));
        assert!(!file.exists(), "the file is deleted after the move");
        let moved = load(&vault).unwrap().unwrap();
        assert_eq!(moved.token, "abc123");
        assert_eq!(moved.account_id, "", "a file does not name the account");
        assert_eq!(moved.server, SERVER);
        assert_eq!(migrate_file(&vault, &file), Ok(false), "a second start finds no file");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_migration_keeps_a_session_the_vault_already_holds() {
        let d = dir("keep");
        let file = d.join(FILE);
        std::fs::write(&file, "token=old999\n").unwrap();
        let vault = Fake::default();
        store(&vault, &session("new111")).unwrap();
        assert_eq!(migrate_file(&vault, &file), Ok(true));
        assert!(!file.exists());
        assert_eq!(load(&vault).unwrap().unwrap().token, "new111", "the keychain copy wins");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_failed_write_keeps_the_file_for_the_next_start() {
        let d = dir("failed");
        let file = d.join(FILE);
        std::fs::write(&file, "token=abc123\n").unwrap();
        let vault = Fake { refuse_writes: true, ..Fake::default() };
        assert!(matches!(migrate_file(&vault, &file), Err(Error::Io(_))));
        assert!(file.exists(), "nothing is lost when the keychain refuses");
        assert_eq!(load(&vault), Ok(None));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_file_without_a_token_is_deleted_and_stores_nothing() {
        let d = dir("empty");
        let file = d.join(FILE);
        std::fs::write(&file, "token=\nother=1\n").unwrap();
        let vault = Fake::default();
        assert_eq!(migrate_file(&vault, &file), Ok(true));
        assert!(!file.exists());
        assert_eq!(load(&vault), Ok(None));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn the_group_vault_keeps_the_item_in_the_group_store() {
        let d = dir("group");
        let vault = GroupVault::new(&d);
        assert_eq!(load(&vault), Ok(None));
        store(&vault, &session("grp1")).unwrap();
        store(&vault, &session("grp2")).unwrap();
        assert_eq!(load(&vault).unwrap().unwrap().token, "grp2", "a second store replaces the item");
        assert!(d.join("keychain/groups").join(format!("{GROUP}.kc")).exists());
        sign_out(&vault).unwrap();
        assert_eq!(load(&vault), Ok(None));
        sign_out(&vault).unwrap();
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn the_file_path_follows_xdg_then_home() {
        assert_eq!(file_path(Some("/x"), Some("/h")), Some(PathBuf::from("/x/bifrost/cloud_session")));
        assert_eq!(file_path(Some(""), Some("/h")), Some(PathBuf::from("/h/.local/share/bifrost/cloud_session")));
        assert_eq!(file_path(None, None), None);
    }
}
