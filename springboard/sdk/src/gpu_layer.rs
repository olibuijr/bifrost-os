//! The app-facing drawable layer: the model lives in `bifrost_ui::drawable_layer` (shared with the shell); this module
//! re-exports it and converts a presented drawable to an image.

pub use bifrost_ui::drawable_layer::*;

use crate::kit::Image;

/// A presented drawable as an image (moves the pixels).
pub fn into_image(d: Drawable) -> Image {
    Image { w: d.width, h: d.height, px: d.pixels }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_drawable_becomes_an_image() {
        let mut l = DrawableLayer::new(PixelFormat::Rgba8, 2, 1).unwrap();
        let mut d = l.next_drawable().unwrap();
        d.pixels[1] = 5;
        let img = into_image(d);
        assert_eq!((img.w, img.h, img.px[1]), (2, 1, 5));
    }
}
