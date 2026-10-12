//! `bifrost-audio-units` (owner: task 14): the component trait (render block on the RT thread, parameter tree
//! with ramps, presets, MIDI input, latency and tail time) and in-process and out-of-process hosting.
//! `bifrost-audio-engine` (owner: task 14): the node graph (player, mixer, effect, IO and tap nodes) with manual
//! and offline rendering, on `bifrost-audio-hal` and `bifrost-audio-units`; re-exported by the SDK.
//!
//! Layout of this crate: `param` (parameter specs, the shared control handle and the RT-side ramping tree),
//! `preset`, `midi`, `unit` (the `AudioUnit` trait, `Block`, errors), `registry` (component descriptions),
//! `host` (in-process host), `remote` (out-of-process hosting on the activation-record and ring ABI of
//! `bifrost-audio-shm`) and `builtin` (gain, biquad filter, test synth). Safe code only. The old models in
//! `ui/src/audio_render.rs` and `plugin_host.rs` are absorbed here (component registry, parameter tree, presets).
//!
//! Real-time rules: `AudioUnit::render`, `ParameterTree::{sync, advance, fill}` and `UnitHost::render` never
//! allocate and never lock.

#![forbid(unsafe_code)]

pub mod builtin;
pub mod host;
pub mod midi;
pub mod param;
pub mod preset;
pub mod registry;
pub mod remote;
pub mod unit;

pub use host::UnitHost;
pub use midi::MidiEvent;
pub use param::{ParamHandle, ParamSpec, ParamUnit, ParameterTree};
pub use preset::{Preset, PresetStore};
pub use registry::{ComponentDescription, ComponentRegistry};
pub use unit::{AudioUnit, Block, RenderConfig, UnitError, UnitInfo, UnitKind};
