//! Anti-aliased vector shapes from signed distance functions. The code lives in `bifrost_render::sdf_glyphs`, the
//! crate both the shell and the SDK depend on; this module keeps the shell's paths (`shell::glyphs::paint` and the rest)
//! for the shell and the apps that import them.

pub use bifrost_render::sdf_glyphs::*;
