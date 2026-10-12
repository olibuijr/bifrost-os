//! Shaders: GLSL in `gpu/shaders/` compiled to SPIR-V by `build.rs` (glslc) and embedded in the binary.
//!
//! A shader is named by its file name with the stage suffix: `quad.vert`, `rrect.frag`, `kawase_down.comp`.
//! `build.rs` writes `$OUT_DIR/shaders.rs` with `pub static SHADERS: &[(&str, &[u8])]`.
//!
//! Owner: wave G1 agent `gpu-pipeline` (build.rs, this module). Shader files are added by their users
//! (bifrost-2d: gfx2d shapes and text; bifrost-wm: compose and blur) under `gpu/shaders/`.

use crate::{Error, Result};

include!(concat!(env!("OUT_DIR"), "/shaders.rs"));

/// The first word of every SPIR-V module (little endian in the byte stream: `03 02 23 07`).
pub const SPIRV_MAGIC: u32 = 0x0723_0203;

/// The pipeline stage a shader name is for, from its file suffix.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    Vertex,
    Fragment,
    Compute,
}

impl Stage {
    /// The stage a shader name ends with (`.vert`, `.frag`, `.comp`), or `None`.
    pub fn of_name(name: &str) -> Option<Stage> {
        match name.rsplit_once('.').map(|(_, ext)| ext) {
            Some("vert") => Some(Stage::Vertex),
            Some("frag") => Some(Stage::Fragment),
            Some("comp") => Some(Stage::Compute),
            _ => None,
        }
    }

    /// The file suffix of this stage.
    pub fn suffix(self) -> &'static str {
        match self {
            Stage::Vertex => "vert",
            Stage::Fragment => "frag",
            Stage::Compute => "comp",
        }
    }
}

/// Checks that `name` names a shader for `expected` (a pipeline given a fragment shader for its vertex stage is
/// a caller bug, reported before Vulkan sees it).
pub(crate) fn expect_stage(name: &str, expected: Stage) -> Result<()> {
    match Stage::of_name(name) {
        Some(s) if s == expected => Ok(()),
        _ => Err(Error::Invalid(format!("shader {name:?} is not a .{} shader", expected.suffix()))),
    }
}

/// Decodes little-endian bytes into SPIR-V words. `None` when the length is not a whole number of words or the
/// first word is not the SPIR-V magic (a corrupt or non-SPIR-V blob).
pub fn words_from_bytes(bytes: &[u8]) -> Option<Vec<u32>> {
    if bytes.is_empty() || bytes.len() % 4 != 0 {
        return None;
    }
    let words: Vec<u32> = bytes.chunks_exact(4).map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect();
    (words[0] == SPIRV_MAGIC).then_some(words)
}

/// The SPIR-V words of shader `name` (`"quad.vert"`), or `None` when no such shader was built.
pub fn shader(name: &str) -> Option<Vec<u32>> {
    let bytes = SHADERS.iter().find(|(n, _)| *n == name)?.1;
    words_from_bytes(bytes)
}

/// Every built shader name, for tests and diagnostics.
pub fn names() -> Vec<&'static str> {
    SHADERS.iter().map(|(n, _)| *n).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stage_comes_from_the_suffix() {
        assert_eq!(Stage::of_name("quad.vert"), Some(Stage::Vertex));
        assert_eq!(Stage::of_name("rrect.frag"), Some(Stage::Fragment));
        assert_eq!(Stage::of_name("kawase_down.comp"), Some(Stage::Compute));
        assert_eq!(Stage::of_name("common.glsl"), None);
        assert_eq!(Stage::of_name("noextension"), None);
    }

    #[test]
    fn suffix_round_trips_through_the_name() {
        for s in [Stage::Vertex, Stage::Fragment, Stage::Compute] {
            assert_eq!(Stage::of_name(&format!("x.{}", s.suffix())), Some(s));
        }
    }

    #[test]
    fn a_fragment_name_is_rejected_for_the_vertex_stage() {
        assert!(expect_stage("quad.vert", Stage::Vertex).is_ok());
        let err = expect_stage("quad.frag", Stage::Vertex).unwrap_err();
        assert!(matches!(err, Error::Invalid(ref m) if m.contains("quad.frag")));
    }

    #[test]
    fn words_are_little_endian_and_magic_checked() {
        // The magic 0x07230203 in little endian, then one more word 0x00010000 (SPIR-V 1.0).
        let bytes = [0x03, 0x02, 0x23, 0x07, 0x00, 0x00, 0x01, 0x00];
        assert_eq!(words_from_bytes(&bytes), Some(vec![SPIRV_MAGIC, 0x0001_0000]));
    }

    #[test]
    fn words_reject_bad_length_empty_and_wrong_magic() {
        assert_eq!(words_from_bytes(&[]), None);
        assert_eq!(words_from_bytes(&[0x03, 0x02, 0x23]), None);
        assert_eq!(words_from_bytes(&[0, 0, 0, 0]), None);
        // Big-endian magic bytes are not SPIR-V in this little-endian embedding.
        assert_eq!(words_from_bytes(&[0x07, 0x23, 0x02, 0x03]), None);
    }

    #[test]
    fn every_built_shader_is_spirv_and_named_by_its_stage() {
        for name in names() {
            let words = shader(name).expect("a listed shader decodes");
            assert_eq!(words[0], SPIRV_MAGIC, "{name}");
            assert!(Stage::of_name(name).is_some(), "{name} has no stage suffix");
        }
    }

    #[test]
    fn unknown_names_are_absent() {
        assert!(shader("no-such-shader.vert").is_none());
    }
}
