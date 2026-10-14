//! Account password records: a salted, iterated keyed hash, checked in constant time, and a failed-attempt lockout.
//!
//! A record stores the salt, the iteration count and the derived key, never the password. The key is PBKDF2 (RFC 8018)
//! with HMAC-SHA256 from [`crate::hkdf`], one output block. Verification compares the derived key in constant time, and
//! [`Attempts`] counts failures per account and locks the account for a period after too many of them.
//!
//! Pure: no directory, network or clock is read. The caller supplies the salt and the current time in seconds.

use crate::hkdf::{hmac_sha256, HASH_LEN};

/// Smallest salt accepted when a record is made. Salts are random bytes chosen by the caller.
pub const MIN_SALT_LEN: usize = 16;
/// Fewest PBKDF2 iterations a record may use.
pub const MIN_ITERATIONS: u32 = 1_000;
/// Failed attempts in a row that lock an account.
pub const MAX_FAILURES: u32 = 5;
/// Seconds an account stays locked after it reaches `MAX_FAILURES`.
pub const LOCK_SECONDS: u64 = 60;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PasswordError {
    /// An empty password cannot be recorded.
    EmptyPassword,
    /// The salt is shorter than `MIN_SALT_LEN`.
    ShortSalt { len: usize },
    /// The iteration count is below `MIN_ITERATIONS`.
    TooFewIterations { requested: u32 },
}

/// The stored form of a password: enough to check a guess, not enough to recover it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PasswordRecord {
    iterations: u32,
    salt: Vec<u8>,
    key: [u8; HASH_LEN],
}

/// The result of one sign-in attempt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// The password matched; the failure count is cleared.
    Accepted,
    /// The password did not match. `failures_left` attempts remain before the lock.
    Rejected { failures_left: u32 },
    /// The account is locked until `until` (seconds). The password was not checked.
    Locked { until: u64 },
}

/// PBKDF2-HMAC-SHA256 with one output block: the derived key for `password`, `salt` and `iterations`.
pub fn pbkdf2_sha256(password: &[u8], salt: &[u8], iterations: u32) -> [u8; HASH_LEN] {
    let mut first_input = salt.to_vec();
    first_input.extend_from_slice(&1u32.to_be_bytes());
    let mut block = hmac_sha256(password, &first_input);
    let mut key = block;
    for _ in 1..iterations {
        block = hmac_sha256(password, &block);
        for (out, byte) in key.iter_mut().zip(block.iter()) {
            *out ^= byte;
        }
    }
    key
}

/// Compares two byte strings without an early exit on the first difference.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b.iter()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

impl PasswordRecord {
    /// Records `password` under `salt` with `iterations` rounds.
    pub fn new(password: &[u8], salt: &[u8], iterations: u32) -> Result<PasswordRecord, PasswordError> {
        if password.is_empty() {
            return Err(PasswordError::EmptyPassword);
        }
        if salt.len() < MIN_SALT_LEN {
            return Err(PasswordError::ShortSalt { len: salt.len() });
        }
        if iterations < MIN_ITERATIONS {
            return Err(PasswordError::TooFewIterations { requested: iterations });
        }
        Ok(PasswordRecord { iterations, salt: salt.to_vec(), key: pbkdf2_sha256(password, salt, iterations) })
    }

    pub fn iterations(&self) -> u32 {
        self.iterations
    }

    pub fn salt(&self) -> &[u8] {
        &self.salt
    }

    /// True when `guess` derives the stored key.
    pub fn matches(&self, guess: &[u8]) -> bool {
        if guess.is_empty() {
            return false;
        }
        let derived = pbkdf2_sha256(guess, &self.salt, self.iterations);
        constant_time_eq(&derived, &self.key)
    }
}

/// Failed-attempt state for one account.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Attempts {
    failures: u32,
    locked_until: Option<u64>,
}

impl Attempts {
    /// Checks one attempt at time `now` (seconds). A locked account refuses without checking the password;
    /// once the lock has passed, the next attempt starts a fresh count.
    pub fn check(&mut self, record: &PasswordRecord, guess: &[u8], now: u64) -> Outcome {
        if let Some(until) = self.locked_until {
            if now < until {
                return Outcome::Locked { until };
            }
            self.locked_until = None;
            self.failures = 0;
        }
        if record.matches(guess) {
            self.failures = 0;
            return Outcome::Accepted;
        }
        self.failures += 1;
        if self.failures >= MAX_FAILURES {
            let until = now + LOCK_SECONDS;
            self.locked_until = Some(until);
            self.failures = 0;
            return Outcome::Locked { until };
        }
        Outcome::Rejected { failures_left: MAX_FAILURES - self.failures }
    }

    /// Failures counted since the last success or lock.
    pub fn failures(&self) -> u32 {
        self.failures
    }
}
