//! Parity sdk-complete-10: headless tests of the account password record (`password_verify`): the key derivation
//! against published PBKDF2-HMAC-SHA256 vectors, record refusals, constant-time matching, and the failed-attempt lock.
//! No directory, network or clock is read; the tests pass the time in.

use bifrost_core::password_verify::{
    pbkdf2_sha256, Attempts, Outcome, PasswordError, PasswordRecord, LOCK_SECONDS, MAX_FAILURES, MIN_ITERATIONS,
    MIN_SALT_LEN,
};
use bifrost_core::sha256::hex;

const SALT: &[u8] = b"0123456789abcdef";

fn record(password: &str) -> PasswordRecord {
    PasswordRecord::new(password.as_bytes(), SALT, MIN_ITERATIONS).expect("a valid record")
}

#[test]
fn pbkdf2_matches_the_published_one_iteration_vector() {
    let key = pbkdf2_sha256(b"password", b"salt", 1);
    assert_eq!(hex(&key), "120fb6cffcf8b32c43e7225256c4f837a86548c92ccc35480805987cb70be17b");
}

#[test]
fn pbkdf2_matches_the_published_two_iteration_vector() {
    let key = pbkdf2_sha256(b"password", b"salt", 2);
    assert_eq!(hex(&key), "ae4d0c95af6b46d32d0adff928f06dd02a303f8ef3c251dfd6e2d85a95474c43");
}

#[test]
fn a_record_refuses_an_empty_password_a_short_salt_or_few_iterations() {
    assert_eq!(PasswordRecord::new(b"", SALT, MIN_ITERATIONS), Err(PasswordError::EmptyPassword));
    assert_eq!(
        PasswordRecord::new(b"secret", &SALT[..MIN_SALT_LEN - 1], MIN_ITERATIONS),
        Err(PasswordError::ShortSalt { len: MIN_SALT_LEN - 1 })
    );
    assert_eq!(
        PasswordRecord::new(b"secret", SALT, MIN_ITERATIONS - 1),
        Err(PasswordError::TooFewIterations { requested: MIN_ITERATIONS - 1 })
    );
}

#[test]
fn a_record_keeps_its_salt_and_iteration_count_and_never_the_password() {
    let stored = record("correct horse");
    assert_eq!(stored.iterations(), MIN_ITERATIONS);
    assert_eq!(stored.salt(), SALT);
    assert_ne!(stored, record("another password"));
}

#[test]
fn the_right_password_matches_and_a_near_miss_does_not() {
    let stored = record("correct horse");
    assert!(stored.matches(b"correct horse"));
    assert!(!stored.matches(b"correct horse "));
    assert!(!stored.matches(b"Correct horse"));
    assert!(!stored.matches(b""), "an empty guess never matches");
}

#[test]
fn the_same_password_under_another_salt_gives_another_record() {
    let other_salt = b"fedcba9876543210";
    let a = PasswordRecord::new(b"secret", SALT, MIN_ITERATIONS).unwrap();
    let b = PasswordRecord::new(b"secret", other_salt, MIN_ITERATIONS).unwrap();
    assert_ne!(a, b);
    assert!(a.matches(b"secret"));
    assert!(b.matches(b"secret"));
}

#[test]
fn a_correct_password_clears_the_failure_count() {
    let stored = record("open sesame");
    let mut attempts = Attempts::default();
    assert_eq!(attempts.check(&stored, b"nope", 100), Outcome::Rejected { failures_left: MAX_FAILURES - 1 });
    assert_eq!(attempts.failures(), 1);
    assert_eq!(attempts.check(&stored, b"open sesame", 101), Outcome::Accepted);
    assert_eq!(attempts.failures(), 0);
}

#[test]
fn the_fifth_failure_locks_the_account_for_sixty_seconds() {
    let stored = record("open sesame");
    let mut attempts = Attempts::default();
    for i in 0..MAX_FAILURES - 1 {
        assert_eq!(
            attempts.check(&stored, b"wrong", 1000 + u64::from(i)),
            Outcome::Rejected { failures_left: MAX_FAILURES - 1 - i }
        );
    }
    assert_eq!(attempts.check(&stored, b"wrong", 1010), Outcome::Locked { until: 1010 + LOCK_SECONDS });
}

#[test]
fn a_locked_account_refuses_even_the_right_password_until_the_lock_passes() {
    let stored = record("open sesame");
    let mut attempts = Attempts::default();
    for _ in 0..MAX_FAILURES {
        attempts.check(&stored, b"wrong", 500);
    }
    assert_eq!(attempts.check(&stored, b"open sesame", 559), Outcome::Locked { until: 560 });
    assert_eq!(attempts.check(&stored, b"open sesame", 560), Outcome::Accepted);
}

#[test]
fn after_the_lock_passes_the_count_starts_again() {
    let stored = record("open sesame");
    let mut attempts = Attempts::default();
    for _ in 0..MAX_FAILURES {
        attempts.check(&stored, b"wrong", 0);
    }
    assert_eq!(attempts.check(&stored, b"wrong", LOCK_SECONDS), Outcome::Rejected { failures_left: MAX_FAILURES - 1 });
}
