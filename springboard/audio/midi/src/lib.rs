//! `bifrost-midi` (owner: task 15): endpoints, UMP (MIDI 2.0) inside with MIDI 1.0 at the edges, timestamps on
//! the audio host clock, and the routing hub of `bifrost-midid`. Pure models: no device is opened here.
//!
//! `wire` and `broker` are the client protocol of the daemon (frames, requests and events) and its socket-free
//! server side; the SDK client and `bifrost-midid` both build on them.
//!
//! Absorbs `core/src/midi_endpoints.rs` (`endpoints`) and `ui/src/midi_{events,thru,ble}.rs`
//! (`events`, `thru`, `ble`).

pub mod ble;
pub mod broker;
pub mod clock;
pub mod endpoints;
pub mod events;
pub mod hub;
pub mod thru;
pub mod ump;
pub mod wire;
