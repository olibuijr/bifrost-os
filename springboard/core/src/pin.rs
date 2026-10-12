//! The pinned clock and seed for snapshot tests, shared by the shell crate and the SDK (`bifrost_app::testing`
//! re-exports it). The compliance harness sets [`ENV_PIN`] to `1` before it spawns an app binary, so anything that
//! reads the wall clock through [`pinned`] draws the same frame on every run.

/// Environment variable that pins the clock and the random seed.
pub const ENV_PIN: &str = "BIFROST_TEST_PIN";
/// The pinned wall clock: 2025-10-10 12:00:00 UTC, in Unix seconds.
pub const PINNED_UNIX_SECS: i64 = 1_760_097_600;
/// The pinned random seed (non-zero, so xorshift generators work).
pub const PINNED_SEED: u64 = 0x9E37_79B9_7F4A_7C15;

/// True when the clock and seed are pinned (`BIFROST_TEST_PIN=1`).
pub fn pinned() -> bool {
    std::env::var(ENV_PIN).is_ok_and(|v| v == "1")
}
