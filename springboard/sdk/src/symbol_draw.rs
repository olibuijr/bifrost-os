//! Symbol styling for apps: the weight, scale and rendering mode of a symbol applied to the shape painters of
//! [`crate::glyph_draw`]. The steps and factors are the model in `bifrost_ui::symbol_layers`; this module turns them into
//! the stroke width, the size and the colour of each layer, and paints the layer.
//!
//! ```ignore
//! let style = SymbolStyle { weight: Weight::Semibold, mode: RenderingMode::Hierarchical, ..SymbolStyle::default() };
//! let ring = bifrost_app::glyph_draw::ring(c, style.size(12.0), style.stroke(2.0), 0.0);
//! paint_layer(surface, rect, &style, 0, colour, ring);
//! ```
//!
//! A colour is premultiplied RGBA, as `bifrost_render::gfx::rgba` makes it.

use bifrost_render::gfx::Surface;
use bifrost_ui::symbol_layers::{palette_slot, rendering_opacity, RenderingMode, Scale, Weight};

/// How one symbol is drawn: its weight and scale steps, its rendering mode and the palette the mode draws from.
#[derive(Clone, Debug, PartialEq)]
pub struct SymbolStyle {
    pub weight: Weight,
    pub scale: Scale,
    pub mode: RenderingMode,
    /// Colours for [`RenderingMode::Palette`] and [`RenderingMode::Multicolor`]: layer `n` takes slot `n`, the last
    /// slot repeats. Empty means the caller's colour is used for every layer.
    pub palette: Vec<u32>,
}

impl Default for SymbolStyle {
    /// The baseline: regular weight, medium scale, monochrome, no palette.
    fn default() -> Self {
        SymbolStyle { weight: Weight::Regular, scale: Scale::Medium, mode: RenderingMode::Monochrome, palette: Vec::new() }
    }
}

impl SymbolStyle {
    /// The stroke width for a base width drawn at the regular weight.
    pub fn stroke(&self, base: f32) -> f32 {
        base * self.weight.stroke_factor()
    }

    /// The size for a base size drawn at the medium scale.
    pub fn size(&self, base: f32) -> f32 {
        base * self.scale.factor()
    }

    /// The colour of the layer of the given rank (0 is the primary layer). `colour` is the caller's colour for the
    /// modes that use one colour: monochrome, hierarchical, and palette or multicolour with an empty palette.
    pub fn layer_colour(&self, rank: usize, colour: u32) -> u32 {
        match self.mode {
            RenderingMode::Monochrome => colour,
            RenderingMode::Hierarchical => fade(colour, rendering_opacity(RenderingMode::Hierarchical, rank)),
            RenderingMode::Palette | RenderingMode::Multicolor => match palette_slot(rank, self.palette.len()) {
                Some(slot) => self.palette[slot],
                None => colour,
            },
        }
    }
}

/// Scales every channel of a premultiplied colour by `opacity`, which keeps it premultiplied.
fn fade(colour: u32, opacity: f32) -> u32 {
    let k = (opacity.clamp(0.0, 1.0) * 255.0 + 0.5) as u32;
    let channel = |shift: u32| ((colour >> shift) & 0xff) * k / 255;
    channel(0) | (channel(8) << 8) | (channel(16) << 16) | (channel(24) << 24)
}

/// Paints one layer of a symbol: `dist` is the signed distance of the layer's shape (from [`crate::glyph_draw`]
/// painters built with the style's [`SymbolStyle::stroke`] and [`SymbolStyle::size`]), and the layer is coloured
/// by [`SymbolStyle::layer_colour`] inside the rectangle `rect` (x, y, width, height).
pub fn paint_layer(
    surface: &mut Surface,
    rect: (f32, f32, f32, f32),
    style: &SymbolStyle,
    rank: usize,
    colour: u32,
    dist: impl Fn(f32, f32) -> f32,
) {
    crate::glyph_draw::paint(surface, rect, style.layer_colour(rank, colour), dist);
}
