//! `bifrost-audio-units` (owner: task 14): the component trait (render block on the RT thread, parameter tree
//! with ramps, presets, MIDI input, latency and tail time) and in-process and out-of-process hosting.
//! `bifrost-audio-engine` (owner: task 14): the node graph (player, mixer, effect, IO and tap nodes) with manual
//! and offline rendering, on `bifrost-audio-hal` and `bifrost-audio-units`; re-exported by the SDK.
//!
//! Layout of this crate: `param` (parameter specs, the shared control handle and the RT-side ramping tree),
//! `preset`, `midi`, `unit` (the `AudioUnit` trait, `Block`, errors), `registry` (component descriptions),
//! `host` (in-process host), `sequencer` (sample-accurate MIDI sequencing inside the render callback), `synth` (a polyphonic General MIDI instrument without a sound bank), `plugin` (loads third-party units from shared libraries over a C ABI), `remote`
//! (out-of-process hosting on the activation-record and ring ABI of `bifrost-audio-shm`) and `builtin` (gain, biquad
//! filter, test synth). Safe code only, except `plugin`, the one module allowed `unsafe` (`dlopen` and the calls
//! through a plug-in's table). The old models in
//! `ui/src/audio_render.rs` and `plugin_host.rs` are absorbed here (component registry, parameter tree, presets).
//!
//! Real-time rules: `AudioUnit::render`, `ParameterTree::{sync, advance, fill}` and `UnitHost::render` never
//! allocate and never lock.

#![deny(unsafe_code)]

pub mod builtin;
pub mod host;
pub mod midi;
pub mod param;
pub mod plugin;
pub mod preset;
pub mod registry;
pub mod remote;
pub mod sequencer;
pub mod synth;
pub mod unit;

pub use host::UnitHost;
pub use midi::MidiEvent;
pub use param::{ParamHandle, ParamSpec, ParamUnit, ParameterTree};
pub use plugin::{PluginError, PluginLibrary, PluginUnit};
pub use sequencer::{SeqEvent, Sequence, SequencerHandle, SequencerUnit};
pub use synth::GmSynth;
pub use preset::{Preset, PresetStore};
pub use registry::{ComponentDescription, ComponentRegistry};
pub use unit::{AudioUnit, Block, RenderConfig, UnitError, UnitInfo, UnitKind};
