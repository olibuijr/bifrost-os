//! Where the Color Meter's pixels come from. The app reads the display through `ScreenSource` and never talks to the
//! compositor. The shell does not hand screen pixels or the pointer position to apps yet (the same gap as the visual
//! search path, docs/parity/visual-intelligence.md), so the production source is `NoCapture`: it reports that nothing
//! can be read rather than inventing pixels.

use crate::color::Rgb8;

/// A rectangle of display pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PixelRect {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

pub trait ScreenSource {
    /// The display size in pixels, or `None` when no display can be read.
    fn display_size(&self) -> Option<(u32, u32)>;

    /// The pointer position in display pixels, or `None` when the source does not know it.
    fn pointer(&self) -> Option<(u32, u32)>;

    /// The codes of every pixel of `rect`, row by row, or `None` when the source cannot read it.
    fn read(&self, rect: PixelRect) -> Option<Vec<Rgb8>>;
}

/// The production source until the shell delivers screen pixels to apps: nothing can be read.
pub struct NoCapture;

impl ScreenSource for NoCapture {
    fn display_size(&self) -> Option<(u32, u32)> {
        None
    }

    fn pointer(&self) -> Option<(u32, u32)> {
        None
    }

    fn read(&self, _rect: PixelRect) -> Option<Vec<Rgb8>> {
        None
    }
}

/// An in-memory display for tests.
#[cfg(test)]
pub struct Fixture {
    pub w: u32,
    pub h: u32,
    pub pixels: Vec<Rgb8>,
    pub at: Option<(u32, u32)>,
}

#[cfg(test)]
impl Fixture {
    /// `None` when the pixel count does not match the size.
    pub fn new(w: u32, h: u32, pixels: Vec<Rgb8>) -> Option<Fixture> {
        (pixels.len() == (w as usize) * (h as usize)).then_some(Fixture { w, h, pixels, at: None })
    }
}

#[cfg(test)]
impl ScreenSource for Fixture {
    fn display_size(&self) -> Option<(u32, u32)> {
        Some((self.w, self.h))
    }

    fn pointer(&self) -> Option<(u32, u32)> {
        self.at
    }

    fn read(&self, rect: PixelRect) -> Option<Vec<Rgb8>> {
        if rect.x.checked_add(rect.w)? > self.w || rect.y.checked_add(rect.h)? > self.h {
            return None;
        }
        let mut out = Vec::with_capacity((rect.w * rect.h) as usize);
        for row in rect.y..rect.y + rect.h {
            let start = (row * self.w + rect.x) as usize;
            out.extend_from_slice(&self.pixels[start..start + rect.w as usize]);
        }
        Some(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_production_source_reads_nothing() {
        assert_eq!(NoCapture.display_size(), None);
        assert_eq!(NoCapture.pointer(), None);
        assert_eq!(NoCapture.read(PixelRect { x: 0, y: 0, w: 1, h: 1 }), None);
    }

    #[test]
    fn a_fixture_reads_rows_in_order_and_refuses_outside_rects() {
        let px: Vec<Rgb8> = (0..6u8).map(|i| Rgb8::new(i, i, i)).collect();
        let f = Fixture::new(3, 2, px).unwrap();
        let got = f.read(PixelRect { x: 1, y: 0, w: 2, h: 2 }).unwrap();
        assert_eq!(got, vec![Rgb8::new(1, 1, 1), Rgb8::new(2, 2, 2), Rgb8::new(4, 4, 4), Rgb8::new(5, 5, 5)]);
        assert!(f.read(PixelRect { x: 2, y: 0, w: 2, h: 1 }).is_none());
        assert!(f.read(PixelRect { x: 0, y: 1, w: 1, h: 2 }).is_none());
    }

    #[test]
    fn a_fixture_refuses_a_wrong_pixel_count() {
        assert!(Fixture::new(2, 2, vec![Rgb8::new(0, 0, 0); 3]).is_none());
    }
}
