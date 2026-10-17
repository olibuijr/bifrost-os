//! The keychain store (`SecItemAdd` / `SecItemCopyMatching` / `SecItemUpdate` / `SecItemDelete`,
//! `kSecClassGenericPassword`): secrets an app must not keep in its own, app-readable data directory.
//!
//! The shell owns the store and speaks for the app (`ToShell::Keychain`, docs/bifrost-ui.md "Keychain"). One file
//! per app id under `<root>/keychain/`, mode 0600 in a 0700 directory owned by the shell (root), so the app's own
//! user cannot read or replace it. The app id is the one the shell launched, never one the app sends, so an app
//! reaches only its own items. Items are keyed by `(service, account)`.
//!
//! At rest the file is `BKC1 | nonce(12) | ChaCha20(items) | HMAC-SHA256`, with per-app subkeys derived from a
//! random master key in `<root>/keychain/.master` (0600). Limitation, stated plainly: the master key lives on the same
//! disk as the items, so this protects against apps, other users and stray backups of the app data, not against
//! root or an attacker with the whole disk (there is no TPM or login-password key source yet). Without a master key
//! (`Store::plain`, tests only) the file is plain text behind the same permissions.

use bifrost_core::sha256::Sha256;
use bifrost_proto::{KeychainOp, KeychainStatus, MAX_KEYCHAIN_NAME, MAX_KEYCHAIN_VALUE};
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

/// Items one app may hold.
pub const MAX_ITEMS: usize = 256;
const MAGIC: &[u8; 4] = b"BKC1";
const MAGIC_PLAIN: &[u8; 4] = b"BKP0";

/// What a keychain call returns on failure; `status()` is the wire form, `code()` the `OSStatus`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// errSecDuplicateItem (-25299)
    DuplicateItem,
    /// errSecItemNotFound (-25300)
    ItemNotFound,
    /// errSecNotAvailable (-25291): the app has no `keychain` permission
    NotPermitted,
    /// errSecParam (-50)
    Param,
    /// errSecIO (-36): the store could not be read, decrypted or written
    Io(String),
}

impl Error {
    pub fn code(&self) -> i32 {
        match self {
            Error::DuplicateItem => -25299,
            Error::ItemNotFound => -25300,
            Error::NotPermitted => -25291,
            Error::Param => -50,
            Error::Io(_) => -36,
        }
    }

    pub fn status(&self) -> KeychainStatus {
        match self {
            Error::DuplicateItem => KeychainStatus::Duplicate,
            Error::ItemNotFound => KeychainStatus::NotFound,
            Error::NotPermitted => KeychainStatus::NotPermitted,
            Error::Param => KeychainStatus::Param,
            Error::Io(_) => KeychainStatus::Io,
        }
    }

    /// The error for a non-`Ok` wire status.
    pub fn from_status(s: KeychainStatus) -> Option<Error> {
        Some(match s {
            KeychainStatus::Ok => return None,
            KeychainStatus::Duplicate => Error::DuplicateItem,
            KeychainStatus::NotFound => Error::ItemNotFound,
            KeychainStatus::NotPermitted => Error::NotPermitted,
            KeychainStatus::Param => Error::Param,
            KeychainStatus::Io => Error::Io(String::new()),
        })
    }
}

impl Error {
    /// The error for an `OSStatus` code (as [`Error::code`] gives it); `None` for errSecSuccess (0). Codes this store
    /// does not produce are I/O errors.
    pub fn from_code(code: i32) -> Option<Error> {
        Some(match code {
            0 => return None,
            -25299 => Error::DuplicateItem,
            -25300 => Error::ItemNotFound,
            -25291 => Error::NotPermitted,
            -50 => Error::Param,
            other => Error::Io(format!("OSStatus {other}")),
        })
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Error::DuplicateItem => write!(f, "the item already exists (errSecDuplicateItem)"),
            Error::ItemNotFound => write!(f, "the item could not be found (errSecItemNotFound)"),
            Error::NotPermitted => write!(f, "the app has no keychain permission"),
            Error::Param => write!(f, "invalid service, account or value (errSecParam)"),
            Error::Io(e) => write!(f, "keychain I/O error: {e}"),
        }
    }
}

impl std::error::Error for Error {}

/// One item's secret and the Unix time it was last written (0 when the file predates the time column).
type Items = BTreeMap<(String, String), (Vec<u8>, u64)>;

/// One item in a listing: its `(service, account)` and the time it was last written, with no secret.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Listing {
    pub service: String,
    pub account: String,
    pub modified: u64,
}

/// One app's items, loaded from and written to its file. Reads reload, so the shell keeps no secrets in memory
/// between calls.
pub struct Store {
    path: PathBuf,
    keys: Option<Keys>,
}

struct Keys {
    enc: [u8; 32],
    mac: [u8; 32],
}

impl Store {
    /// The store of `app_id` under the shell data dir `root`: `<root>/keychain/<app_id>.kc`, with the master key
    /// created on first use. Refuses ids that are not a plain file name.
    pub fn open(root: &Path, app_id: &str) -> Result<Store, Error> {
        let path = file_for(root, app_id)?;
        let dir = path.parent().unwrap();
        make_private_dir(dir)?;
        let master = master_key(&dir.join(".master"))?;
        Ok(Store { path, keys: Some(derive(&master, app_id)) })
    }

    /// The store of an access group under the shell data dir `root`: `<root>/keychain/groups/<group>.kc`. Every app
    /// that lists `group` in its manifest opens this one file, keyed by the group rather than by any app id. The
    /// master key is the same one the app stores use. Refuses names that are not a plain file name.
    pub fn open_group(root: &Path, group: &str) -> Result<Store, Error> {
        if !group_ok(group) {
            return Err(Error::Param);
        }
        let dir = root.join("keychain");
        let groups = dir.join("groups");
        make_private_dir(&groups)?;
        let master = master_key(&dir.join(".master"))?;
        let path = groups.join(format!("{group}.kc"));
        Ok(Store { path, keys: Some(derive(&master, &format!("group:{group}"))) })
    }

    /// An encrypted store with a caller-supplied master key (tests, and a future key source such as a TPM).
    pub fn with_master(path: PathBuf, master: &[u8; 32], app_id: &str) -> Store {
        Store { path, keys: Some(derive(master, app_id)) }
    }

    /// An unencrypted store (tests only).
    pub fn plain(path: PathBuf) -> Store {
        Store { path, keys: None }
    }

    /// Runs one wire operation. Returns the secret for `Get`, empty otherwise.
    pub fn apply(&self, op: &KeychainOp) -> Result<Vec<u8>, Error> {
        match op {
            KeychainOp::Add { service, account, value } => self.add(service, account, value).map(|_| Vec::new()),
            KeychainOp::Get { service, account } => self.get(service, account),
            KeychainOp::Update { service, account, value } => self.update(service, account, value).map(|_| Vec::new()),
            KeychainOp::Delete { service, account } => self.delete(service, account).map(|_| Vec::new()),
        }
    }

    /// `SecItemAdd`: `DuplicateItem` when `(service, account)` exists.
    pub fn add(&self, service: &str, account: &str, value: &[u8]) -> Result<(), Error> {
        check(service, account, Some(value))?;
        let mut items = self.load()?;
        let key = (service.to_string(), account.to_string());
        if items.contains_key(&key) {
            return Err(Error::DuplicateItem);
        }
        if items.len() >= MAX_ITEMS {
            return Err(Error::Param);
        }
        items.insert(key, (value.to_vec(), now()));
        self.save(&items)
    }

    /// `SecItemCopyMatching` (`kSecReturnData`): `ItemNotFound` when there is no such item.
    pub fn get(&self, service: &str, account: &str) -> Result<Vec<u8>, Error> {
        check(service, account, None)?;
        self.load()?.remove(&(service.to_string(), account.to_string())).map(|(v, _)| v).ok_or(Error::ItemNotFound)
    }

    /// Every item's `(service, account)` and modified time, sorted by service then account. No secrets are read out.
    pub fn list(&self) -> Result<Vec<Listing>, Error> {
        Ok(self.load()?.into_iter().map(|((service, account), (_, modified))| Listing { service, account, modified }).collect())
    }

    /// `SecItemUpdate`: replaces the value; `ItemNotFound` when there is no such item.
    pub fn update(&self, service: &str, account: &str, value: &[u8]) -> Result<(), Error> {
        check(service, account, Some(value))?;
        let mut items = self.load()?;
        let slot = items.get_mut(&(service.to_string(), account.to_string())).ok_or(Error::ItemNotFound)?;
        *slot = (value.to_vec(), now());
        self.save(&items)
    }

    /// `SecItemDelete`: `ItemNotFound` when there is no such item.
    pub fn delete(&self, service: &str, account: &str) -> Result<(), Error> {
        check(service, account, None)?;
        let mut items = self.load()?;
        items.remove(&(service.to_string(), account.to_string())).ok_or(Error::ItemNotFound)?;
        self.save(&items)
    }

    fn load(&self) -> Result<Items, Error> {
        let raw = match std::fs::read(&self.path) {
            Ok(r) => r,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Items::new()),
            Err(e) => return Err(Error::Io(e.to_string())),
        };
        let text = match (&self.keys, raw.get(..4)) {
            (Some(k), Some(m)) if m == MAGIC => open_box(k, &raw)?,
            (None, Some(m)) if m == MAGIC_PLAIN => raw[4..].to_vec(),
            _ => return Err(Error::Io("keychain file has the wrong format".into())),
        };
        parse(&text)
    }

    fn save(&self, items: &Items) -> Result<(), Error> {
        let text = render(items);
        let blob = match &self.keys {
            Some(k) => seal(k, &text)?,
            None => [MAGIC_PLAIN.as_slice(), &text].concat(),
        };
        write_private(&self.path, &blob)
    }
}

fn check(service: &str, account: &str, value: Option<&[u8]>) -> Result<(), Error> {
    let ok = |s: &str| !s.is_empty() && s.len() <= MAX_KEYCHAIN_NAME && !s.contains('\0');
    if !ok(service) || !ok(account) || value.is_some_and(|v| v.len() > MAX_KEYCHAIN_VALUE) {
        return Err(Error::Param);
    }
    Ok(())
}

/// A plain file-name id: the alphabet and length of an app id, so it cannot escape the keychain directory.
fn group_ok(group: &str) -> bool {
    !group.is_empty()
        && group.len() <= 128
        && !group.starts_with('.')
        && group.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-' || b == b'_')
}

fn file_for(root: &Path, app_id: &str) -> Result<PathBuf, Error> {
    if !group_ok(app_id) {
        return Err(Error::Param);
    }
    Ok(root.join("keychain").join(format!("{app_id}.kc")))
}

// ---- text form of the items: hex(service) TAB hex(account) TAB hex(value) per line ----

fn render(items: &Items) -> Vec<u8> {
    let mut out = String::new();
    for ((s, a), (v, t)) in items {
        out.push_str(&format!("{}\t{}\t{}\t{t}\n", hex(s.as_bytes()), hex(a.as_bytes()), hex(v)));
    }
    out.into_bytes()
}

fn parse(text: &[u8]) -> Result<Items, Error> {
    let bad = || Error::Io("keychain file is damaged".into());
    let text = std::str::from_utf8(text).map_err(|_| bad())?;
    let mut items = Items::new();
    for line in text.lines().filter(|l| !l.is_empty()) {
        let mut f = line.split('\t');
        let (Some(s), Some(a), Some(v)) = (f.next(), f.next(), f.next()) else { return Err(bad()) };
        // the modified time is optional, so a file written before it was added still reads
        let modified = match f.next() {
            None => 0,
            Some(t) => t.parse::<u64>().map_err(|_| bad())?,
        };
        if f.next().is_some() {
            return Err(bad());
        }
        let s = String::from_utf8(unhex(s).ok_or_else(bad)?).map_err(|_| bad())?;
        let a = String::from_utf8(unhex(a).ok_or_else(bad)?).map_err(|_| bad())?;
        items.insert((s, a), (unhex(v).ok_or_else(bad)?, modified));
    }
    Ok(items)
}

/// Unix time now, in seconds (0 if the clock is before 1970).
fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Lowercase hex of `b`: the form a secret takes in the service JSON.
pub fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// The bytes of a hex string; `None` when it is odd-length or not hex.
pub fn unhex(s: &str) -> Option<Vec<u8>> {
    if s.len() % 2 != 0 || !s.is_ascii() {
        return None;
    }
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok()).collect()
}

// ---- files ----

fn make_private_dir(dir: &Path) -> Result<(), Error> {
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
    std::fs::DirBuilder::new().recursive(true).mode(0o700).create(dir).map_err(io)?;
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700)).map_err(io)
}

fn write_private(path: &Path, data: &[u8]) -> Result<(), Error> {
    use std::os::unix::fs::OpenOptionsExt;
    let tmp = path.with_extension("tmp");
    let _ = std::fs::remove_file(&tmp);
    let mut f = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&tmp).map_err(io)?;
    f.write_all(data).map_err(io)?;
    f.sync_all().map_err(io)?;
    std::fs::rename(&tmp, path).map_err(io)
}

fn io(e: std::io::Error) -> Error {
    Error::Io(e.to_string())
}

fn random(buf: &mut [u8]) -> Result<(), Error> {
    std::fs::File::open("/dev/urandom").and_then(|mut f| f.read_exact(buf)).map_err(io)
}

fn master_key(path: &Path) -> Result<[u8; 32], Error> {
    match std::fs::read(path) {
        Ok(k) if k.len() == 32 => Ok(k.try_into().unwrap()),
        Ok(_) => Err(Error::Io("keychain master key is damaged".into())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let mut k = [0u8; 32];
            random(&mut k)?;
            write_private(path, &k)?;
            Ok(k)
        }
        Err(e) => Err(io(e)),
    }
}

// ---- crypto: HMAC-SHA256 and ChaCha20 (RFC 8439), encrypt-then-MAC ----

fn derive(master: &[u8; 32], app_id: &str) -> Keys {
    Keys {
        enc: hmac(master, format!("bifrost.keychain.v1:enc:{app_id}").as_bytes()),
        mac: hmac(master, format!("bifrost.keychain.v1:mac:{app_id}").as_bytes()),
    }
}

fn seal(k: &Keys, plain: &[u8]) -> Result<Vec<u8>, Error> {
    let mut nonce = [0u8; 12];
    random(&mut nonce)?;
    Ok(seal_with(k, &nonce, plain))
}

fn seal_with(k: &Keys, nonce: &[u8; 12], plain: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(4 + 12 + plain.len() + 32);
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(nonce);
    out.extend_from_slice(plain);
    chacha20_xor(&k.enc, nonce, &mut out[16..]);
    let tag = hmac(&k.mac, &out);
    out.extend_from_slice(&tag);
    out
}

fn open_box(k: &Keys, blob: &[u8]) -> Result<Vec<u8>, Error> {
    if blob.len() < 4 + 12 + 32 {
        return Err(Error::Io("keychain file is truncated".into()));
    }
    let (body, tag) = blob.split_at(blob.len() - 32);
    let want = hmac(&k.mac, body);
    let diff = want.iter().zip(tag).fold(0u8, |a, (x, y)| a | (x ^ y));
    if diff != 0 {
        return Err(Error::Io("keychain file failed its integrity check".into()));
    }
    let nonce: [u8; 12] = body[4..16].try_into().unwrap();
    let mut plain = body[16..].to_vec();
    chacha20_xor(&k.enc, &nonce, &mut plain);
    Ok(plain)
}

fn hmac(key: &[u8], msg: &[u8]) -> [u8; 32] {
    let mut k = [0u8; 64];
    if key.len() > 64 {
        k[..32].copy_from_slice(&bifrost_core::sha256::digest(key));
    } else {
        k[..key.len()].copy_from_slice(key);
    }
    let mut inner = Sha256::new();
    inner.update(&k.map(|b| b ^ 0x36));
    inner.update(msg);
    let inner = inner.finish();
    let mut outer = Sha256::new();
    outer.update(&k.map(|b| b ^ 0x5c));
    outer.update(&inner);
    outer.finish()
}

fn chacha20_block(key: &[u8; 32], counter: u32, nonce: &[u8; 12]) -> [u8; 64] {
    let mut s = [0u32; 16];
    s[..4].copy_from_slice(&[0x6170_7865, 0x3320_646e, 0x7962_2d32, 0x6b20_6574]);
    for i in 0..8 {
        s[4 + i] = u32::from_le_bytes(key[i * 4..i * 4 + 4].try_into().unwrap());
    }
    s[12] = counter;
    for i in 0..3 {
        s[13 + i] = u32::from_le_bytes(nonce[i * 4..i * 4 + 4].try_into().unwrap());
    }
    let mut w = s;
    fn qr(w: &mut [u32; 16], a: usize, b: usize, c: usize, d: usize) {
        w[a] = w[a].wrapping_add(w[b]);
        w[d] = (w[d] ^ w[a]).rotate_left(16);
        w[c] = w[c].wrapping_add(w[d]);
        w[b] = (w[b] ^ w[c]).rotate_left(12);
        w[a] = w[a].wrapping_add(w[b]);
        w[d] = (w[d] ^ w[a]).rotate_left(8);
        w[c] = w[c].wrapping_add(w[d]);
        w[b] = (w[b] ^ w[c]).rotate_left(7);
    }
    for _ in 0..10 {
        qr(&mut w, 0, 4, 8, 12);
        qr(&mut w, 1, 5, 9, 13);
        qr(&mut w, 2, 6, 10, 14);
        qr(&mut w, 3, 7, 11, 15);
        qr(&mut w, 0, 5, 10, 15);
        qr(&mut w, 1, 6, 11, 12);
        qr(&mut w, 2, 7, 8, 13);
        qr(&mut w, 3, 4, 9, 14);
    }
    let mut out = [0u8; 64];
    for i in 0..16 {
        out[i * 4..i * 4 + 4].copy_from_slice(&w[i].wrapping_add(s[i]).to_le_bytes());
    }
    out
}

fn chacha20_xor(key: &[u8; 32], nonce: &[u8; 12], data: &mut [u8]) {
    for (n, chunk) in data.chunks_mut(64).enumerate() {
        let ks = chacha20_block(key, 1 + n as u32, nonce);
        for (b, k) in chunk.iter_mut().zip(ks) {
            *b ^= k;
        }
    }
}

/// The keychain service that holds SMB share logins (the network-authentication agent's items).
pub const SHARE_LOGIN_SERVICE: &str = "bifrost.network-auth.smb";

/// The account of a share's login: `host/share`, in lower case, so one share is found whatever the case it is named in.
pub fn share_login_account(host: &str, share: &str) -> String {
    format!("{}/{}", host.trim().to_ascii_lowercase(), share.trim().to_ascii_lowercase())
}

/// Stores a share's login as `user NUL password`. Replaces the login already stored for that share (`SecItemUpdate`
/// after `DuplicateItem`). A user name with a NUL byte is `Param`, because the NUL separates the two fields.
pub fn save_share_login(store: &Store, host: &str, share: &str, user: &str, password: &str) -> Result<(), Error> {
    if user.contains('\0') {
        return Err(Error::Param);
    }
    let account = share_login_account(host, share);
    let value = format!("{user}\0{password}");
    match store.add(SHARE_LOGIN_SERVICE, &account, value.as_bytes()) {
        Err(Error::DuplicateItem) => store.update(SHARE_LOGIN_SERVICE, &account, value.as_bytes()),
        other => other,
    }
}

/// Reads a share's login back as `(user, password)`. `ItemNotFound` when none is stored, or the stored item is not a
/// login.
pub fn share_login(store: &Store, host: &str, share: &str) -> Result<(String, String), Error> {
    let bytes = store.get(SHARE_LOGIN_SERVICE, &share_login_account(host, share))?;
    let at = bytes.iter().position(|&b| b == 0).ok_or(Error::ItemNotFound)?;
    let user = String::from_utf8(bytes[..at].to_vec()).map_err(|_| Error::ItemNotFound)?;
    let password = String::from_utf8(bytes[at + 1..].to_vec()).map_err(|_| Error::ItemNotFound)?;
    Ok((user, password))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("bifrost-keychain-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn enc(d: &Path, app: &str) -> Store {
        Store::with_master(d.join(format!("{app}.kc")), &[9u8; 32], app)
    }

    #[test]
    fn a_share_login_is_saved_found_and_replaced_without_case() {
        let d = dir("share-login");
        let s = enc(&d, "files");
        assert_eq!(share_login(&s, "NAS.local", "Music"), Err(Error::ItemNotFound));
        save_share_login(&s, "NAS.local", "Music", "ola", "pw1").unwrap();
        assert_eq!(share_login(&s, "nas.local", "music"), Ok(("ola".to_string(), "pw1".to_string())));
        save_share_login(&s, "nas.local", "music", "ola", "pw2").unwrap();
        assert_eq!(share_login(&s, "nas.local", "MUSIC"), Ok(("ola".to_string(), "pw2".to_string())));
        assert_eq!(share_login(&s, "nas.local", "photos"), Err(Error::ItemNotFound), "another share has no login");
        assert_eq!(save_share_login(&s, "nas.local", "music", "o\0la", "x"), Err(Error::Param));
    }

    #[test]
    fn chacha20_matches_rfc8439_section_2_4_2() {
        let key: [u8; 32] = core::array::from_fn(|i| i as u8);
        let nonce = [0, 0, 0, 0, 0, 0, 0, 0x4a, 0, 0, 0, 0];
        let mut data = b"Ladies and Gentlemen of the class of '99: If I could offer you only one tip for the future, sunscreen would be it.".to_vec();
        chacha20_xor(&key, &nonce, &mut data);
        assert_eq!(hex(&data[..32]), "6e2e359a2568f98041ba0728dd0d6981e97e7aec1d4360c20a27afccfd9fae0b");
    }

    #[test]
    fn hmac_matches_rfc4231_case_2() {
        assert_eq!(hex(&hmac(b"Jefe", b"what do ya want for nothing?")), "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843");
    }

    #[test]
    fn chacha20_round_trips_and_differs_per_nonce() {
        let key = [3u8; 32];
        let plain: Vec<u8> = (0..200u8).collect();
        let mut a = plain.clone();
        chacha20_xor(&key, &[1; 12], &mut a);
        assert_ne!(a, plain);
        let mut b = plain.clone();
        chacha20_xor(&key, &[2; 12], &mut b);
        assert_ne!(a, b);
        chacha20_xor(&key, &[1; 12], &mut a);
        assert_eq!(a, plain);
    }

    #[test]
    fn add_get_update_delete_with_errors() {
        let d = dir("crud");
        let s = enc(&d, "app.a");
        assert_eq!(s.get("svc", "me"), Err(Error::ItemNotFound));
        s.add("svc", "me", b"hunter2").unwrap();
        assert_eq!(s.add("svc", "me", b"other"), Err(Error::DuplicateItem));
        assert_eq!(s.get("svc", "me").unwrap(), b"hunter2");
        assert_eq!(s.update("svc", "nobody", b"x"), Err(Error::ItemNotFound));
        s.update("svc", "me", b"correct horse").unwrap();
        assert_eq!(s.get("svc", "me").unwrap(), b"correct horse");
        s.add("svc", "you", b"").unwrap();
        s.delete("svc", "me").unwrap();
        assert_eq!(s.delete("svc", "me"), Err(Error::ItemNotFound));
        assert_eq!(s.get("svc", "you").unwrap(), b"");
        assert_eq!(Error::DuplicateItem.code(), -25299);
        assert_eq!(Error::ItemNotFound.code(), -25300);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn items_are_keyed_by_service_and_account_and_values_may_be_binary() {
        let d = dir("keys");
        let s = enc(&d, "app.a");
        let bin: Vec<u8> = (0..=255u8).collect();
        s.add("s1", "a", &bin).unwrap();
        s.add("s2", "a", b"two").unwrap();
        s.add("s1", "tab\tand\nnewline", b"three").unwrap();
        assert_eq!(s.get("s1", "a").unwrap(), bin);
        assert_eq!(s.get("s2", "a").unwrap(), b"two");
        assert_eq!(s.get("s1", "tab\tand\nnewline").unwrap(), b"three");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn each_app_has_its_own_items_and_key() {
        let d = dir("apps");
        let a = enc(&d, "app.a");
        let b = enc(&d, "app.b");
        a.add("svc", "me", b"secret-a").unwrap();
        assert_eq!(b.get("svc", "me"), Err(Error::ItemNotFound));
        // even pointed at app a's file, app b's derived key fails the integrity check
        let stolen = Store::with_master(d.join("app.a.kc"), &[9u8; 32], "app.b");
        assert!(matches!(stolen.get("svc", "me"), Err(Error::Io(_))));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn the_file_is_ciphertext_private_and_tamper_evident() {
        use std::os::unix::fs::PermissionsExt;
        let d = dir("file");
        let s = enc(&d, "app.a");
        s.add("svc", "me", b"hunter2-plaintext").unwrap();
        let path = d.join("app.a.kc");
        let raw = std::fs::read(&path).unwrap();
        assert_eq!(&raw[..4], MAGIC);
        assert!(!raw.windows(7).any(|w| w == b"hunter2") && !raw.windows(3).any(|w| w == b"svc"));
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        let mut bad = raw.clone();
        bad[20] ^= 1;
        std::fs::write(&path, &bad).unwrap();
        assert!(matches!(s.get("svc", "me"), Err(Error::Io(_))), "a flipped bit is detected, not decrypted");
        // a replaced plain file is not accepted by an encrypted store
        std::fs::write(&path, b"BKP0").unwrap();
        assert!(matches!(s.get("svc", "me"), Err(Error::Io(_))));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn open_creates_a_private_directory_and_a_stable_master_key() {
        use std::os::unix::fs::PermissionsExt;
        let d = dir("open");
        let s = Store::open(&d, "is.example.app").unwrap();
        s.add("svc", "me", b"v").unwrap();
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&d.join("keychain")), 0o700);
        assert_eq!(mode(&d.join("keychain/.master")), 0o600);
        let again = Store::open(&d, "is.example.app").unwrap();
        assert_eq!(again.get("svc", "me").unwrap(), b"v", "the master key is kept across opens");
        for bad in ["", "../x", "a/b", ".hidden", "sp ace"] {
            assert!(matches!(Store::open(&d, bad), Err(Error::Param)), "{bad:?}");
        }
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn limits_and_parameters_are_refused() {
        let d = dir("limits");
        let s = Store::plain(d.join("p.kc"));
        assert_eq!(s.add("", "a", b"x"), Err(Error::Param));
        assert_eq!(s.add("s", "", b"x"), Err(Error::Param));
        assert_eq!(s.add(&"s".repeat(MAX_KEYCHAIN_NAME + 1), "a", b"x"), Err(Error::Param));
        assert_eq!(s.add("s", "a", &vec![0; MAX_KEYCHAIN_VALUE + 1]), Err(Error::Param));
        s.add("s", "a", &vec![0; MAX_KEYCHAIN_VALUE]).unwrap();
        for i in 1..MAX_ITEMS {
            s.add("s", &format!("a{i}"), b"x").unwrap();
        }
        assert_eq!(s.add("s", "one-too-many", b"x"), Err(Error::Param));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn wire_ops_map_to_statuses() {
        let d = dir("wire");
        let s = enc(&d, "app.a");
        let add = KeychainOp::Add { service: "s".into(), account: "a".into(), value: b"v".to_vec() };
        assert_eq!(s.apply(&add), Ok(Vec::new()));
        assert_eq!(s.apply(&add).unwrap_err().status(), KeychainStatus::Duplicate);
        assert_eq!(s.apply(&KeychainOp::Get { service: "s".into(), account: "a".into() }), Ok(b"v".to_vec()));
        assert_eq!(s.apply(&KeychainOp::Delete { service: "s".into(), account: "x".into() }).unwrap_err().status(), KeychainStatus::NotFound);
        assert_eq!(Error::from_status(KeychainStatus::Ok), None);
        assert_eq!(Error::from_status(KeychainStatus::Duplicate), Some(Error::DuplicateItem));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn listing_names_each_item_with_its_time_and_no_secret() {
        let d = dir("listing");
        let s = enc(&d, "is.example.list");
        assert_eq!(s.list(), Ok(Vec::new()));
        s.add("b.example", "me", b"two").unwrap();
        s.add("a.example", "me", b"one").unwrap();
        let rows = s.list().unwrap();
        let names: Vec<(&str, &str)> = rows.iter().map(|r| (r.service.as_str(), r.account.as_str())).collect();
        assert_eq!(names, vec![("a.example", "me"), ("b.example", "me")], "sorted by service");
        assert!(rows.iter().all(|r| r.modified > 0), "a write stamps the time");
        // an update moves the time forward or keeps it, and the secret still reads back
        s.update("a.example", "me", b"uno").unwrap();
        assert_eq!(s.get("a.example", "me"), Ok(b"uno".to_vec()));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_file_without_the_time_column_still_reads() {
        let d = dir("legacy");
        let path = d.join("old.kc");
        let line = format!("{}\t{}\t{}\n", hex(b"svc"), hex(b"me"), hex(b"v"));
        std::fs::write(&path, [MAGIC_PLAIN.as_slice(), line.as_bytes()].concat()).unwrap();
        let s = Store::plain(path);
        assert_eq!(s.get("svc", "me"), Ok(b"v".to_vec()));
        assert_eq!(s.list().unwrap()[0].modified, 0);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_group_is_one_file_shared_by_name_and_not_an_app_file() {
        let d = dir("group");
        let a = Store::open_group(&d, "is.olibuijr.keys").unwrap();
        a.add("svc", "me", b"shared").unwrap();
        // a second open of the same group sees the item; the group file is not an app file
        let b = Store::open_group(&d, "is.olibuijr.keys").unwrap();
        assert_eq!(b.get("svc", "me"), Ok(b"shared".to_vec()));
        assert!(d.join("keychain").join("groups").join("is.olibuijr.keys.kc").exists());
        assert!(!d.join("keychain").join("is.olibuijr.keys.kc").exists());
        // another group is another file: no item leaks between groups
        let other = Store::open_group(&d, "is.olibuijr.other").unwrap();
        assert_eq!(other.get("svc", "me"), Err(Error::ItemNotFound));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn group_names_are_plain_file_names() {
        let d = dir("group-names");
        for bad in ["", ".hidden", "../escape", "a/b", "a b", "a:b"] {
            assert!(matches!(Store::open_group(&d, bad), Err(Error::Param)), "{bad:?}");
        }
        assert!(Store::open_group(&d, "is.olibuijr.ok-1_2").is_ok());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn os_status_codes_map_back_to_errors() {
        assert_eq!(Error::from_code(0), None);
        for e in [Error::DuplicateItem, Error::ItemNotFound, Error::NotPermitted, Error::Param] {
            assert_eq!(Error::from_code(e.code()), Some(e));
        }
        assert_eq!(Error::from_code(-36), Some(Error::Io("OSStatus -36".into())));
    }

    #[test]
    fn hex_round_trips_and_refuses_bad_text() {
        assert_eq!(hex(&[0, 0xab, 0xff]), "00abff");
        assert_eq!(unhex("00abff"), Some(vec![0, 0xab, 0xff]));
        assert_eq!(unhex("zz"), None);
        assert_eq!(unhex("abc"), None);
    }
}
