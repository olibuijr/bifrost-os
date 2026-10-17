//! `bifrost-audio-policy`: the session manager's decisions as pure, deterministic state machines (no sockets, no
//! clock). `main.rs` feeds them from the daemon's property notices and applies what they return.
//! Modules: `defaults` (default device per role, jack rules), `ducking`, `permissions` (microphone, capture, hog,
//! indicator), `volume` (per-app volume), `sessions` (session categories enforced on application streams),
//! `clients` (the application and process objects the server tracks), `dsp` (which processing stages run for
//! each device and stream).
pub mod clients;
pub mod defaults;
pub mod dsp;
pub mod ducking;
pub mod permissions;
pub mod sessions;
pub mod volume;

/// Unified Logging subsystem and the category this process writes to (contract: observability).
pub const SUBSYSTEM: &str = "is.olibuijr.audio";
pub const CATEGORY: &str = "policy";

/// Logs through stderr with the subsystem and category prefix (the logger crate is not reachable from here).
pub fn log(msg: &str) {
    eprintln!("[{SUBSYSTEM}:{CATEGORY}] {msg}");
}
