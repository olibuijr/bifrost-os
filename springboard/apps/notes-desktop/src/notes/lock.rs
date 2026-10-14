//! Locked notes: a note's text sealed with a password. The key comes from PBKDF2-HMAC-SHA256 over the password
//! and a random salt; the text is sealed with AES-256-GCM under a random nonce. The locked form is plain text
//! (`BIFROST-LOCKED-1`, iteration count, salt, nonce and sealed text in hex), so it can live in the note file.
//! The caller supplies the random salt and nonce, so this module stays deterministic and testable.
#![allow(dead_code)] // the Lock Note action, the lock icon and the unlock sheet wire this in next

use bifrost_core::aes_gcm;
use bifrost_core::hkdf::hmac_sha256;

pub const MAGIC: &str = "BIFROST-LOCKED-1";
/// Key-stretching rounds for new locks.
pub const DEFAULT_ITERATIONS: u32 = 120_000;
pub const SALT_LEN: usize = 16;
pub const NONCE_LEN: usize = 12;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LockError {
    /// The text is not in the locked form.
    NotLocked,
    /// The locked form is damaged.
    Malformed,
    /// The password is wrong (or the text was changed).
    WrongPassword,
}

/// PBKDF2 with HMAC-SHA256, one 32-byte block.
pub fn pbkdf2(password: &[u8], salt: &[u8], iterations: u32) -> [u8; 32] {
    let mut first = salt.to_vec();
    first.extend_from_slice(&1u32.to_be_bytes());
    let mut u = hmac_sha256(password, &first);
    let mut t = u;
    for _ in 1..iterations.max(1) {
        u = hmac_sha256(password, &u);
        for (a, b) in t.iter_mut().zip(u.iter()) {
            *a ^= b;
        }
    }
    t
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn unhex(text: &str) -> Option<Vec<u8>> {
    let t = text.trim();
    if t.len() % 2 != 0 || !t.is_ascii() {
        return None;
    }
    (0..t.len()).step_by(2).map(|i| u8::from_str_radix(&t[i..i + 2], 16).ok()).collect()
}

pub fn is_locked(text: &str) -> bool {
    text.starts_with(MAGIC)
}

/// Locks `text`. `iterations` is stored in the result so the cost can grow over time.
pub fn lock(text: &str, password: &str, salt: &[u8; SALT_LEN], nonce: &[u8; NONCE_LEN], iterations: u32) -> String {
    let key = pbkdf2(password.as_bytes(), salt, iterations);
    // sealing with a 32-byte key and a 12-byte nonce cannot fail unless the text is far beyond a note's size
    let sealed = aes_gcm::seal(&key, nonce, MAGIC.as_bytes(), text.as_bytes()).unwrap_or_default();
    format!("{MAGIC}\n{iterations}\n{}\n{}\n{}\n", hex(salt), hex(nonce), hex(&sealed))
}

/// Unlocks text made by [`lock`].
pub fn unlock(locked: &str, password: &str) -> Result<String, LockError> {
    if !is_locked(locked) {
        return Err(LockError::NotLocked);
    }
    let mut lines = locked.lines();
    lines.next();
    let iterations: u32 = lines.next().and_then(|l| l.trim().parse().ok()).ok_or(LockError::Malformed)?;
    let salt = lines.next().and_then(unhex).ok_or(LockError::Malformed)?;
    let nonce = lines.next().and_then(unhex).ok_or(LockError::Malformed)?;
    let sealed = lines.next().and_then(unhex).ok_or(LockError::Malformed)?;
    if iterations == 0 || iterations > 10_000_000 || nonce.len() != NONCE_LEN {
        return Err(LockError::Malformed);
    }
    let key = pbkdf2(password.as_bytes(), &salt, iterations);
    let plain = aes_gcm::open(&key, &nonce, MAGIC.as_bytes(), &sealed).map_err(|_| LockError::WrongPassword)?;
    String::from_utf8(plain).map_err(|_| LockError::Malformed)
}

/// What the note list shows for a locked note instead of its text.
pub fn placeholder_title() -> &'static str {
    "Locked Note"
}

#[cfg(test)]
mod tests {
    use super::*;

    const SALT: [u8; SALT_LEN] = [7; SALT_LEN];
    const NONCE: [u8; NONCE_LEN] = [9; NONCE_LEN];

    #[test]
    fn hex_round_trip() {
        assert_eq!(hex(&[0, 15, 255]), "000fff");
        assert_eq!(unhex("000fff"), Some(vec![0, 15, 255]));
        assert_eq!(unhex("0"), None);
        assert_eq!(unhex("zz"), None);
    }

    #[test]
    fn pbkdf2_depends_on_every_input() {
        let a = pbkdf2(b"pw", b"salt", 4);
        assert_eq!(a, pbkdf2(b"pw", b"salt", 4));
        assert_ne!(a, pbkdf2(b"pw2", b"salt", 4));
        assert_ne!(a, pbkdf2(b"pw", b"salt2", 4));
        assert_ne!(a, pbkdf2(b"pw", b"salt", 5));
        // one iteration is a single HMAC over salt and the block counter
        let mut first = b"salt".to_vec();
        first.extend_from_slice(&[0, 0, 0, 1]);
        assert_eq!(pbkdf2(b"pw", b"salt", 1), hmac_sha256(b"pw", &first));
    }

    #[test]
    fn lock_and_unlock() {
        let text = "Secret plans\nline two ✓";
        let locked = lock(text, "hunter2", &SALT, &NONCE, 8);
        assert!(is_locked(&locked));
        assert!(!locked.contains("Secret"));
        assert_eq!(unlock(&locked, "hunter2").as_deref(), Ok(text));
        assert_eq!(unlock(&locked, "wrong"), Err(LockError::WrongPassword));
    }

    #[test]
    fn damaged_or_plain_text_is_refused() {
        assert_eq!(unlock("hello", "x"), Err(LockError::NotLocked));
        assert_eq!(unlock(&format!("{MAGIC}\nnope\n"), "x"), Err(LockError::Malformed));
        let locked = lock("abc", "pw", &SALT, &NONCE, 2);
        let mut tampered = locked.clone();
        let last = tampered.trim_end().len() - 1;
        let c = if tampered.as_bytes()[last] == b'0' { "1" } else { "0" };
        tampered.replace_range(last..last + 1, c);
        assert_eq!(unlock(&tampered, "pw"), Err(LockError::WrongPassword));
    }
}
