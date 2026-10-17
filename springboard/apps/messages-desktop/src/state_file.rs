//! The encrypted sessions file of one Matrix login: its Megolm and Olm sessions (see [`crate::crypto`]), kept in the app's
//! data directory. The keychain caps an item at 8 KiB, which a long-lived account overflows.
//!
//! File layout: the magic `BMSS`, one version byte (1), a 12-byte nonce, then the ChaCha20-Poly1305 box of the sessions
//! text (ciphertext and 16-byte tag, from `bifrost_core::chacha_poly`). The key is the pickle key from the keychain. The
//! additional data is the magic, the version and the account's user id, so a file moved to another account fails its tag.
//! A save writes a temporary file, flushes it, and renames it over the old one, so a crash leaves the old file or the
//! new one, never half of either.

use bifrost_core::{chacha_poly, sha256};
use std::io::Write;
use std::path::{Path, PathBuf};

/// The first four bytes of every sessions file.
pub const MAGIC: &[u8; 4] = b"BMSS";
/// The layout version this module reads and writes.
pub const VERSION: u8 = 1;
const HEADER: usize = MAGIC.len() + 1;

/// The file of one login: `<dir>/sessions/<digest of homeserver and user>.bin`. The name is a digest, so an account
/// address never reaches the file system.
pub fn path(dir: &Path, homeserver: &str, user: &str) -> PathBuf {
    let name = sha256::hex(&sha256::digest(format!("{homeserver}\n{user}").as_bytes()));
    dir.join("sessions").join(format!("{name}.bin"))
}

fn additional_data(user: &str) -> Vec<u8> {
    let mut aad = Vec::with_capacity(HEADER + user.len());
    aad.extend_from_slice(MAGIC);
    aad.push(VERSION);
    aad.extend_from_slice(user.as_bytes());
    aad
}

/// Seals the sessions text under `key` for the account `user`. Every call draws a fresh nonce.
pub fn seal(key: &[u8; 32], user: &str, text: &str) -> Result<Vec<u8>, String> {
    let nonce: [u8; chacha_poly::NONCE_LEN] = crate::crypto::random_bytes()?;
    let boxed = chacha_poly::seal(key, &nonce, &additional_data(user), text.as_bytes()).map_err(|e| format!("{e:?}"))?;
    let mut out = Vec::with_capacity(HEADER + nonce.len() + boxed.len());
    out.extend_from_slice(MAGIC);
    out.push(VERSION);
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&boxed);
    Ok(out)
}

/// Opens the bytes of a file made by [`seal`]. A change to any byte, a wrong key or another account is an error, and no
/// text is returned.
pub fn open(key: &[u8; 32], user: &str, data: &[u8]) -> Result<String, String> {
    if data.len() < HEADER + chacha_poly::NONCE_LEN + chacha_poly::TAG_LEN || !data.starts_with(MAGIC) || data[MAGIC.len()] != VERSION {
        return Err("the sessions file is damaged".into());
    }
    let (nonce, boxed) = data[HEADER..].split_at(chacha_poly::NONCE_LEN);
    let plain = chacha_poly::open(key, nonce, &additional_data(user), boxed).map_err(|_| "the sessions file failed its check".to_string())?;
    String::from_utf8(plain).map_err(|_| "the sessions file is damaged".to_string())
}

/// Writes the sessions of one login, sealed under `key`. The bytes go to a temporary file that is flushed and then
/// renamed over the sessions file.
pub fn save(dir: &Path, homeserver: &str, user: &str, key: &[u8; 32], text: &str) -> Result<(), String> {
    let target = path(dir, homeserver, user);
    let parent = target.parent().ok_or("the sessions directory is missing")?;
    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let bytes = seal(key, user, text)?;
    let temp = target.with_extension("tmp");
    let written = std::fs::File::create(&temp).and_then(|mut file| file.write_all(&bytes).and_then(|_| file.sync_all()));
    if let Err(e) = written {
        let _ = std::fs::remove_file(&temp);
        return Err(e.to_string());
    }
    std::fs::rename(&temp, &target).map_err(|e| {
        let _ = std::fs::remove_file(&temp);
        e.to_string()
    })
}

/// Reads the sessions of one login. `None` when no file has been written yet.
pub fn load(dir: &Path, homeserver: &str, user: &str, key: &[u8; 32]) -> Result<Option<String>, String> {
    match std::fs::read(path(dir, homeserver, user)) {
        Ok(data) => open(key, user, &data).map(Some),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: [u8; 32] = [3u8; 32];
    const TEXT: &str = "out\t!room:x\t5\t2\t@bob:x\t\tpickle\nolm\tcurve\tpickle";

    fn temp(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!("messages-state-{tag}-{}", std::process::id()))
    }

    #[test]
    fn sessions_survive_a_seal_and_an_open() {
        let sealed = seal(&KEY, "@alice:x", TEXT).unwrap();
        assert!(sealed.starts_with(MAGIC));
        assert_eq!(open(&KEY, "@alice:x", &sealed).unwrap(), TEXT);
    }

    #[test]
    fn any_changed_byte_is_refused() {
        let sealed = seal(&KEY, "@alice:x", TEXT).unwrap();
        for at in [HEADER, HEADER + 5, sealed.len() - 1] {
            let mut bad = sealed.clone();
            bad[at] ^= 0x01;
            assert!(open(&KEY, "@alice:x", &bad).is_err(), "a change at byte {at} must fail");
        }
        assert!(open(&KEY, "@alice:x", &sealed[..sealed.len() - 1]).is_err(), "a truncated file must fail");
        let mut wrong_magic = sealed.clone();
        wrong_magic[0] = b'X';
        assert!(open(&KEY, "@alice:x", &wrong_magic).is_err());
    }

    #[test]
    fn a_wrong_key_or_another_account_is_refused() {
        let sealed = seal(&KEY, "@alice:x", TEXT).unwrap();
        assert!(open(&[4u8; 32], "@alice:x", &sealed).is_err());
        assert!(open(&KEY, "@bob:x", &sealed).is_err(), "the account is authenticated");
    }

    #[test]
    fn each_seal_draws_a_new_nonce() {
        assert_ne!(seal(&KEY, "@alice:x", TEXT).unwrap(), seal(&KEY, "@alice:x", TEXT).unwrap());
    }

    #[test]
    fn save_and_load_round_trip_without_a_temporary_file() {
        let dir = temp("round");
        assert_eq!(load(&dir, "https://h.x", "@alice:x", &KEY).unwrap(), None, "nothing saved loads as none");
        save(&dir, "https://h.x", "@alice:x", &KEY, TEXT).unwrap();
        assert_eq!(load(&dir, "https://h.x", "@alice:x", &KEY).unwrap().as_deref(), Some(TEXT));
        assert!(!path(&dir, "https://h.x", "@alice:x").with_extension("tmp").exists());
        save(&dir, "https://h.x", "@alice:x", &KEY, "newer").unwrap();
        assert_eq!(load(&dir, "https://h.x", "@alice:x", &KEY).unwrap().as_deref(), Some("newer"), "a save replaces the file");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_damaged_file_on_disk_is_an_error() {
        let dir = temp("damaged");
        save(&dir, "https://h.x", "@alice:x", &KEY, TEXT).unwrap();
        let file = path(&dir, "https://h.x", "@alice:x");
        let mut bytes = std::fs::read(&file).unwrap();
        let last = bytes.len() - 1;
        bytes[last] ^= 0xff;
        std::fs::write(&file, &bytes).unwrap();
        assert!(load(&dir, "https://h.x", "@alice:x", &KEY).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn each_login_has_its_own_file_and_no_account_address_in_the_name() {
        let a = path(Path::new("/d"), "https://h.x", "@alice:x");
        let b = path(Path::new("/d"), "https://h.x", "@bob:x");
        assert_ne!(a, b);
        assert!(!a.to_string_lossy().contains("alice"));
        assert_eq!(a.parent(), Some(Path::new("/d/sessions")));
    }
}
