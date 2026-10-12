//! The render server: the Vulkan compositor of the final architecture (docs/graphics-architecture.md, "Window
//! server"). It replaces Smithay's `GlesRenderer` path in `drm.rs` and `nested.rs`; until the switch it is selected
//! with `BIFROST_WM_RENDERER=vulkan` and the GLES path stays the fallback.
//!
//! Module owners (wave G1): `import` (client buffers to textures), `output` (scanout targets, KMS present, direct
//! scanout), `compose` (scene drawing and damage), `material` (live blur, rounded clips, window shadows), `clock`
//! (frame scheduling, presentation feedback), `layers` (the `bifrost_layer_v1` server and per-context trees),
//! `stats` (frame timings for the bench).
//!
//! The data flow per output frame: `clock` decides to start a frame -> `layers` applies pending transactions and
//! evaluates presentation trees at the predicted presentation time -> `compose` builds the draw list from the
//! window stack (surface trees via `import`, layer trees via their bound contents) and the materials (`material`)
//! -> one command buffer renders into the `output` target -> `output` presents it (in-fence from the submission) ->
//! `clock` records the vblank time and sends presentation feedback and frame callbacks.

pub mod clock;
pub mod compose;
pub mod import;
pub mod layers;
pub mod material;
pub mod output;
pub mod stats;

/// Whether the Vulkan render server is selected (`BIFROST_WM_RENDERER=vulkan`, or `/etc/bifrost/renderer` says
/// `vulkan`). The default flips to Vulkan at the switch-over.
pub fn selected() -> bool {
    std::env::var("BIFROST_WM_RENDERER").map(|v| v == "vulkan").unwrap_or(false)
        || std::fs::read_to_string("/etc/bifrost/renderer").map(|v| v.trim() == "vulkan").unwrap_or(false)
}
