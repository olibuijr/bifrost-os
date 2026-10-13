//! Reusable offscreen drawing: a layer is a pixel buffer that a caller draws into once and then reads many times.
//!
//! `ReusableLayer::draw` runs a closure on a `Surface` over the layer's own pixels. Drawing keeps what is already
//! there, so a caller can build the content in several passes and then hand `image` to `Surface::blit` as often as
//! it needs. `draws` counts the passes, so a caller can check that the content was drawn once and reused. The layer
//! has no window or device behind it; it is plain packed pixels in the same format as `gfx::Image`.

use crate::gfx::{Image, Surface};

/// An offscreen pixel buffer with a draw count. Build it with `new`, which refuses an empty size.
#[derive(Clone, Debug, PartialEq)]
pub struct ReusableLayer {
    image: Image,
    draws: usize,
}

impl ReusableLayer {
    /// A transparent layer of `w` by `h` pixels. Returns `None` when either side is zero or the pixel count overflows.
    pub fn new(w: usize, h: usize) -> Option<Self> {
        if w == 0 || h == 0 {
            return None;
        }
        w.checked_mul(h)?;
        Some(ReusableLayer { image: Image::new(w, h), draws: 0 })
    }

    /// The width and height of the layer, in pixels.
    pub fn size(&self) -> (usize, usize) {
        (self.image.w, self.image.h)
    }

    /// The number of completed draw passes.
    pub fn draws(&self) -> usize {
        self.draws
    }

    /// The layer's pixels, for blitting onto another surface.
    pub fn image(&self) -> &Image {
        &self.image
    }

    /// Runs `f` on a surface over the layer's pixels. The surface starts at row 0 with a stride equal to the
    /// width, and whatever `f` does not overwrite stays as it was.
    pub fn draw<F: FnOnce(&mut Surface<'_>)>(&mut self, f: F) {
        let (w, h) = self.size();
        let mut surface = Surface { w, h, stride: w, px: &mut self.image.px, y0: 0 };
        f(&mut surface);
        self.draws += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_refuses_an_empty_or_overflowing_size() {
        assert!(ReusableLayer::new(0, 4).is_none());
        assert!(ReusableLayer::new(4, 0).is_none());
        assert!(ReusableLayer::new(usize::MAX, 2).is_none());
        let layer = ReusableLayer::new(3, 2).unwrap();
        assert_eq!(layer.size(), (3, 2));
        assert_eq!(layer.draws(), 0);
        assert_eq!(layer.image().px, vec![0; 6]);
    }

    #[test]
    fn draw_passes_a_surface_of_the_layer_size_and_counts_the_pass() {
        let mut layer = ReusableLayer::new(3, 2).unwrap();
        let mut seen = (0, 0, 0);
        layer.draw(|s| seen = (s.w, s.h, s.stride));
        assert_eq!(seen, (3, 2, 3));
        assert_eq!(layer.draws(), 1);
    }

    #[test]
    fn a_pixel_lands_at_row_times_width_plus_column() {
        // Width 3: column 2 of row 1 is index 1 * 3 + 2 = 5.
        let mut layer = ReusableLayer::new(3, 2).unwrap();
        layer.draw(|s| {
            let i = 1 * s.stride + 2;
            s.px[i] = 0xff11_2233;
        });
        assert_eq!(layer.image().px, vec![0, 0, 0, 0, 0, 0xff11_2233]);
    }

    #[test]
    fn later_passes_keep_earlier_content() {
        let mut layer = ReusableLayer::new(3, 2).unwrap();
        layer.draw(|s| s.px[0] = 7);
        layer.draw(|s| s.px[5] = 9);
        assert_eq!(layer.image().px[0], 7);
        assert_eq!(layer.image().px[5], 9);
        assert_eq!(layer.draws(), 2);
    }

    #[test]
    fn pixels_read_back_as_drawn() {
        let mut layer = ReusableLayer::new(2, 1).unwrap();
        layer.draw(|s| {
            s.px[0] = 0xff00_00ff;
            s.px[1] = 0xff00_ff00;
        });
        assert_eq!(layer.image(), &Image { w: 2, h: 1, px: vec![0xff00_00ff, 0xff00_ff00] });
    }
}
