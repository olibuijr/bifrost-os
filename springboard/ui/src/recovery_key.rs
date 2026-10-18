//! The forgotten-password reset path (the recovery of the local authentication service): a recovery key that the
//! person keeps, or an administrator account that can reset another account's password.
//!
//! A recovery key is 16 characters from a 32-character alphabet without the look-alikes (no 0, O, 1 or I), shown in
//! four groups of four. It is made from 10 bytes of entropy the caller supplies ([`from_entropy`]). Only a salted
//! digest of the key is kept ([`Record`]); the key itself is shown once and never stored. A reset with the key, or
//! through an administrator, must pass [`authorize_reset`]: the wrong key is counted, and [`MAX_TRIES`] wrong keys in a
//! row lock the reset path until the person signs in with the password.
//!
//! Pure model: no disk, no prompt. The login screen asks for the key or the administrator's sign-in, and the password
//! change itself is the login backend's.

use bifrost_core::sha256;

/// The characters a key uses: 32 of them, so five bits per character.
pub const ALPHABET: &[u8; 32] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
/// Characters in a key.
pub const KEY_CHARS: usize = 16;
/// Bytes of entropy a key needs (16 characters of five bits is 80 bits).
pub const ENTROPY_BYTES: usize = 10;
/// Wrong keys in a row before the reset path locks.
pub const MAX_TRIES: u32 = 5;

/// Why a key could not be made.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KeyError {
    /// Fewer entropy bytes than a key needs.
    NotEnoughEntropy,
}

/// Makes a recovery key from `entropy` (at least [`ENTROPY_BYTES`] bytes, from a secure source) and formats it in four
/// groups of four, separated by dashes.
pub fn from_entropy(entropy: &[u8]) -> Result<String, KeyError> {
    if entropy.len() < ENTROPY_BYTES {
        return Err(KeyError::NotEnoughEntropy);
    }
    let mut bits: u32 = 0;
    let mut have = 0u32;
    let mut chars = String::with_capacity(KEY_CHARS + 3);
    for &byte in &entropy[..ENTROPY_BYTES] {
        bits = (bits << 8) | u32::from(byte);
        have += 8;
        while have >= 5 && chars.len() < KEY_CHARS {
            have -= 5;
            let index = ((bits >> have) & 0x1f) as usize;
            chars.push(char::from(ALPHABET[index]));
        }
    }
    let grouped: Vec<&str> = (0..KEY_CHARS).step_by(4).map(|i| &chars[i..i + 4]).collect();
    Ok(grouped.join("-"))
}

/// The key as typed, in the form it is checked in: upper case, with the dashes and spaces removed.
pub fn normalise(typed: &str) -> String {
    typed.chars().filter(|c| !c.is_whitespace() && *c != '-').map(|c| c.to_ascii_uppercase()).collect()
}

/// The stored form of a recovery key: a salt and the digest of salt and key. Written as `salt:digest` in hex.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Record {
    salt: String,
    digest: String,
}

fn digest_of(salt: &[u8], key: &str) -> String {
    let mut data = salt.to_vec();
    data.extend_from_slice(normalise(key).as_bytes());
    sha256::hex(&sha256::digest(&data))
}

impl Record {
    /// The record for `key`, salted with `salt` (16 random bytes from a secure source).
    pub fn enrol(key: &str, salt: &[u8]) -> Record {
        Record { salt: sha256::hex(salt), digest: digest_of(salt, key) }
    }

    /// Whether `typed` is the key this record was made from. The comparison looks at every character.
    pub fn matches(&self, typed: &str) -> bool {
        let mut salt = Vec::with_capacity(self.salt.len() / 2);
        let mut i = 0;
        while i + 1 < self.salt.len() {
            match u8::from_str_radix(&self.salt[i..i + 2], 16) {
                Ok(b) => salt.push(b),
                Err(_) => return false,
            }
            i += 2;
        }
        let expected = digest_of(&salt, typed);
        expected.len() == self.digest.len() && expected.bytes().zip(self.digest.bytes()).fold(0u8, |acc, (a, b)| acc | (a ^ b)) == 0
    }

    /// The record as one line: `salt:digest`.
    pub fn encode(&self) -> String {
        format!("{}:{}", self.salt, self.digest)
    }

    /// Reads a line written by [`Record::encode`]; anything else is `None`.
    pub fn decode(line: &str) -> Option<Record> {
        let (salt, digest) = line.trim().split_once(':')?;
        let hex_ok = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_hexdigit());
        (hex_ok(salt) && digest.len() == 64 && hex_ok(digest)).then(|| Record { salt: salt.to_string(), digest: digest.to_string() })
    }
}

/// The wrong keys in a row. A right key or a password sign-in clears them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Attempts {
    failed: u32,
}

impl Attempts {
    /// Whether the reset path is still open.
    pub fn open(&self) -> bool {
        self.failed < MAX_TRIES
    }

    /// Counts one answer: a wrong key adds to the count, a right one clears it.
    pub fn record(&mut self, right: bool) {
        self.failed = if right { 0 } else { self.failed.saturating_add(1) };
    }

    /// The wrong keys still allowed before the path locks.
    pub fn left(&self) -> u32 {
        MAX_TRIES.saturating_sub(self.failed)
    }

    /// The path locks until the person signs in with the password (the password sign-in clears the count).
    pub fn clear(&mut self) {
        self.failed = 0;
    }
}

/// How the person proves they may reset the password.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Proof {
    /// The recovery key the person kept, as typed.
    Key { typed: String },
    /// An administrator account: whether it is an admin and whether its password was just verified.
    Administrator { is_admin: bool, password_verified: bool },
}

/// What the reset path decided.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// The password may be reset now.
    Authorized,
    /// The key was wrong; the count went up and `left` more tries remain.
    WrongKey { left: u32 },
    /// Too many wrong keys: the reset path is locked until a password sign-in.
    LockedOut,
    /// The account signing in is not an administrator.
    NotAdministrator,
    /// The administrator's password was not verified.
    NotVerified,
}

/// Decides a reset. A key is checked against `record` and counted in `attempts`; an administrator needs to be an
/// admin whose password was verified. A locked path refuses everything.
pub fn authorize_reset(record: &Record, attempts: &mut Attempts, proof: &Proof) -> Outcome {
    match proof {
        _ if !attempts.open() => Outcome::LockedOut,
        Proof::Key { typed } => {
            let right = record.matches(typed);
            attempts.record(right);
            if right {
                Outcome::Authorized
            } else if attempts.open() {
                Outcome::WrongKey { left: attempts.left() }
            } else {
                Outcome::LockedOut
            }
        }
        Proof::Administrator { is_admin: false, .. } => Outcome::NotAdministrator,
        Proof::Administrator { password_verified: false, .. } => Outcome::NotVerified,
        Proof::Administrator { .. } => Outcome::Authorized,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SALT: [u8; 16] = [7; 16];

    fn entropy() -> Vec<u8> {
        (1..=10u8).collect()
    }

    fn key() -> String {
        from_entropy(&entropy()).unwrap()
    }

    #[test]
    fn a_key_has_sixteen_characters_from_the_alphabet_in_four_groups() {
        let k = key();
        let groups: Vec<&str> = k.split('-').collect();
        assert_eq!(groups.len(), 4);
        assert!(groups.iter().all(|g| g.len() == 4));
        assert!(normalise(&k).bytes().all(|b| ALPHABET.contains(&b)));
        assert!(!normalise(&k).contains(['0', 'O', '1', 'I']));
    }

    #[test]
    fn short_entropy_is_refused() {
        assert_eq!(from_entropy(&[1, 2, 3]), Err(KeyError::NotEnoughEntropy));
    }

    #[test]
    fn typing_is_forgiving_about_case_dashes_and_spaces() {
        assert_eq!(normalise(" abcd-efgh 2345 "), "ABCDEFGH2345");
    }

    #[test]
    fn the_record_checks_the_key_and_is_not_the_key() {
        let rec = Record::enrol(&key(), &SALT);
        assert!(rec.matches(&key().to_lowercase()));
        assert!(!rec.matches("AAAA-AAAA-AAAA-AAAA"));
        assert!(!rec.encode().contains(&normalise(&key())));
    }

    #[test]
    fn a_record_round_trips_through_its_line() {
        let rec = Record::enrol(&key(), &SALT);
        assert_eq!(Record::decode(&rec.encode()), Some(rec.clone()));
        assert_eq!(Record::decode("not a record"), None);
        assert_eq!(Record::decode(&format!("zz:{}", "0".repeat(64))), None);
    }

    #[test]
    fn five_wrong_keys_lock_the_path_until_cleared() {
        let rec = Record::enrol(&key(), &SALT);
        let mut tries = Attempts::default();
        let wrong = Proof::Key { typed: "WRONG".into() };
        assert_eq!(authorize_reset(&rec, &mut tries, &wrong), Outcome::WrongKey { left: 4 });
        for _ in 0..3 {
            authorize_reset(&rec, &mut tries, &wrong);
        }
        assert_eq!(authorize_reset(&rec, &mut tries, &wrong), Outcome::LockedOut);
        let right = Proof::Key { typed: key() };
        assert_eq!(authorize_reset(&rec, &mut tries, &right), Outcome::LockedOut, "a locked path refuses even the right key");
        tries.clear();
        assert_eq!(authorize_reset(&rec, &mut tries, &right), Outcome::Authorized);
    }

    #[test]
    fn a_right_key_clears_the_wrong_count() {
        let rec = Record::enrol(&key(), &SALT);
        let mut tries = Attempts::default();
        let wrong = Proof::Key { typed: "WRONG".into() };
        authorize_reset(&rec, &mut tries, &wrong);
        let right = Proof::Key { typed: key() };
        assert_eq!(authorize_reset(&rec, &mut tries, &right), Outcome::Authorized);
        assert_eq!(tries.left(), MAX_TRIES);
    }

    #[test]
    fn an_administrator_resets_only_as_an_admin_with_a_verified_password() {
        let rec = Record::enrol(&key(), &SALT);
        let mut tries = Attempts::default();
        assert_eq!(authorize_reset(&rec, &mut tries, &Proof::Administrator { is_admin: false, password_verified: true }), Outcome::NotAdministrator);
        assert_eq!(authorize_reset(&rec, &mut tries, &Proof::Administrator { is_admin: true, password_verified: false }), Outcome::NotVerified);
        assert_eq!(authorize_reset(&rec, &mut tries, &Proof::Administrator { is_admin: true, password_verified: true }), Outcome::Authorized);
    }
}
