//! `bifrost-audio-engine` (owner: task 14): the node graph (player, mixer, effect, IO and tap nodes) with manual
//! and offline rendering, on `bifrost-audio-hal` and `bifrost-audio-units`; re-exported by the SDK.
//!
//! * `rt`: the lock-free pieces shared between the control side and the render thread (node and player controls,
//!   tap rings, MIDI queues).
//! * `graph`: the render-side graph; one `render_block` walks the nodes in dependency order into pre-allocated
//!   buffers, with no allocation, locks or syscalls.
//! * `engine`: the control-side `Engine` that builds the graph, renders it manually or offline, and runs it on a
//!   device through `bifrost-audio-hal` (the only place that touches the hardware path).
//!
//! The models of `ui/src/audio_engine.rs` (attach, connect, detach, prepare, start, render) are absorbed here.
//! Safe code only.

#![forbid(unsafe_code)]

pub mod analysis;
pub mod engine;
pub mod graph;
pub mod rt;

pub use engine::{Engine, EngineConfig, EngineError, NodeId, OUTPUT};
pub use rt::{EdgeHandle, MidiHandle, NodeHandle, PlayerHandle, Tap};
