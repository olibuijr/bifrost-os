//! Shaders: GLSL in `gpu/shaders/` compiled to SPIR-V by `build.rs` (glslc) and embedded in the binary.
//!
//! A shader is named by its file name with the stage suffix: `quad.vert`, `rrect.frag`, `kawase_down.comp`.
//! `build.rs` writes `$OUT_DIR/shaders.rs` with `pub static SHADERS: &[(&str, &[u8])]`.
//!
//! Owner: wave G1 agent `gpu-pipeline` (build.rs, this module). Shader files are added by their users
//! (bifrost-2d: gfx2d shapes and text; bifrost-wm: compose and blur) under `gpu/shaders/`.

include!(concat!(env!("OUT_DIR"), "/shaders.rs"));

/// The SPIR-V words of shader `name` (`"quad.vert"`), or `None` when no such shader was built.
pub fn shader(name: &str) -> Option<Vec<u32>> {
    let bytes = SHADERS.iter().find(|(n, _)| *n == name)?.1;
    if bytes.len() % 4 != 0 {
        return None;
    }
    Some(bytes.chunks_exact(4).map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect())
}

/// Every built shader name, for tests and diagnostics.
pub fn names() -> Vec<&'static str> {
    SHADERS.iter().map(|(n, _)| *n).collect()
}
