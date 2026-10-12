//! System preferences for apps: the staged, locked session over the system-wide store.
//!
//! The model lives in `bifrost_core::prefs_session`. An app opens a [`PrefsSession`] over the values it loaded,
//! edits them, takes the lock, commits, and pushes the keys returned by [`PrefsSession::apply`] to the running
//! system. Reading and writing the store file is the caller's job; this module does no file or service I/O.
//! Per-app values stay in `defaults` (UserDefaults).

pub use bifrost_core::prefs_session::{validate_key, PrefValue, PrefsError, PrefsSession, MAX_KEY_BYTES, MAX_TEXT_BYTES};
