//! Text: a glyph atlas (`R8Unorm`, 2048x2048, shelf packing, LRU page reset when full) filled by the existing font
//! rasteriser (`bifrost_render::font::Font::render_glyph` at the run's size and a quarter-pixel subpixel offset),
//! and the layout of a `TextRun` into glyph quads (`shapes::Kind::Mask` instances) with advances and kerning from
//! `bifrost_render::font`, matching what `Font::render` draws on the CPU.
//!
//! Owner: wave G1 agent `gfx2d-text`.

use bifrost_gpu::{Device, Result, Texture};

/// A glyph's place in the atlas.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GlyphKey {
    pub face: u64,
    pub glyph: u16,
    /// Size in 1/64 px.
    pub size_64: u32,
    /// Subpixel x offset in quarter pixels (0..3).
    pub subpixel: u8,
    /// Hash of the variation values.
    pub vars: u64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AtlasEntry {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
    /// Offset of the bitmap from the pen position.
    pub left: f32,
    pub top: f32,
}

pub struct GlyphAtlas {
    pub(crate) texture: Texture,
}

impl GlyphAtlas {
    pub fn new(device: &Device) -> Result<GlyphAtlas> {
        let _ = device;
        todo!("gfx2d-text")
    }
}
