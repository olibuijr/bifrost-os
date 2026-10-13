//! STUB (cg-context branch): minimal colour so the context compiles. The colour agent owns the real module
//! (colour + colour space); drop this file on merge.

/// A straight (non-premultiplied) RGBA colour, components 0..=1.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Color {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Color {
    pub const BLACK: Color = Color { r: 0.0, g: 0.0, b: 0.0, a: 1.0 };
    pub const CLEAR: Color = Color { r: 0.0, g: 0.0, b: 0.0, a: 0.0 };

    pub fn rgba(r: f32, g: f32, b: f32, a: f32) -> Self {
        Color { r, g, b, a }
    }

    pub fn gray(w: f32, a: f32) -> Self {
        Color { r: w, g: w, b: w, a }
    }

    /// From a packed premultiplied pixel (`gfx::pack` layout).
    pub fn from_packed(px: u32) -> Self {
        let a = (px >> 24) as f32 / 255.0;
        let un = |c: u32| if a > 0.0 { ((c & 255) as f32 / 255.0 / a).min(1.0) } else { 0.0 };
        Color { r: un(px), g: un(px >> 8), b: un(px >> 16), a }
    }
}
