//! `bifrost-audio-bench`: millisecond benchmarks of the audio and MIDI path (docs/audio/19-bench-instrument.md).
//!
//! Pure modules (`stats`, `onset`, `schedule`, `json`, `args`) are tested on known inputs; `instrument` is the
//! virtual MIDI instrument; `rig` is the model of the server's IO cycle around a driver; the four benches
//! (`midi_latency`, `roundtrip`, `sweep`, `throughput`) each return one `json::Report`.

pub mod args;
pub mod instrument;
pub mod json;
pub mod midi_latency;
pub mod onset;
pub mod rig;
pub mod roundtrip;
pub mod schedule;
pub mod stats;
pub mod sweep;
pub mod throughput;
