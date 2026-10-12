//! `bifrost-midi` (owner: task 15): endpoints, UMP (MIDI 2.0) inside with MIDI 1.0 at the edges, timestamps on
//! the audio host clock, and the routing hub of `bifrost-midid`. Pure models: no device is opened here.
//!
//! Absorbs `core/src/midi_endpoints.rs` (`endpoints`) and `ui/src/midi_{events,thru,ble}.rs`
//! (`events`, `thru`, `ble`).

pub mod ble;
pub mod clock;
pub mod endpoints;
pub mod events;
pub mod hub;
pub mod thru;
pub mod ump;
