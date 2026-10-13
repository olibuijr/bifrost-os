//! The viewer's copy of the remote screen: packed 0x00RRGGBB pixels, updated by decoded rectangles.

use crate::message::{Payload, Rect};
use crate::{Error, PixelFormat};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Framebuffer {
    pub w: usize,
    pub h: usize,
    /// Row-major pixels, 0x00RRGGBB, `w * h` long.
    pub px: Vec<u32>,
}

impl Framebuffer {
    /// A black framebuffer of the given size.
    pub fn new(w: usize, h: usize) -> Self {
        Framebuffer { w, h, px: vec![0; w * h] }
    }

    /// Changes the size. The contents are cleared: the server sends a full update after a DesktopSize change.
    pub fn resize(&mut self, w: usize, h: usize) {
        *self = Framebuffer::new(w, h);
    }

    /// The pixel at (x, y), or None outside the framebuffer.
    pub fn get(&self, x: usize, y: usize) -> Option<u32> {
        if x < self.w && y < self.h {
            Some(self.px[y * self.w + x])
        } else {
            None
        }
    }

    /// Applies one rectangle from an update, read in `fmt`. A rectangle that leaves the framebuffer, or a Raw payload
    /// of the wrong length, is a protocol error and nothing is drawn.
    pub fn apply(&mut self, rect: &Rect, fmt: &PixelFormat) -> Result<(), Error> {
        match &rect.payload {
            Payload::DesktopSize => {
                self.resize(rect.w, rect.h);
                Ok(())
            }
            Payload::Raw(bytes) => {
                self.check(rect.x, rect.y, rect.w, rect.h)?;
                let bpp = fmt.bytes_per_pixel();
                if bytes.len() != rect.w * rect.h * bpp {
                    return Err(Error::Protocol("raw rectangle has the wrong length".into()));
                }
                for row in 0..rect.h {
                    let dst = (rect.y + row) * self.w + rect.x;
                    for col in 0..rect.w {
                        let i = (row * rect.w + col) * bpp;
                        self.px[dst + col] = fmt.read_rgb(&bytes[i..i + bpp]);
                    }
                }
                Ok(())
            }
            Payload::CopyRect { src_x, src_y } => {
                self.check(rect.x, rect.y, rect.w, rect.h)?;
                self.check(*src_x, *src_y, rect.w, rect.h)?;
                // The source and destination may overlap, so the source rows are gathered before any is written.
                let mut tmp = Vec::with_capacity(rect.w * rect.h);
                for row in 0..rect.h {
                    let s = (src_y + row) * self.w + src_x;
                    tmp.extend_from_slice(&self.px[s..s + rect.w]);
                }
                for row in 0..rect.h {
                    let d = (rect.y + row) * self.w + rect.x;
                    self.px[d..d + rect.w].copy_from_slice(&tmp[row * rect.w..(row + 1) * rect.w]);
                }
                Ok(())
            }
        }
    }

    fn check(&self, x: usize, y: usize, w: usize, h: usize) -> Result<(), Error> {
        if x + w <= self.w && y + h <= self.h {
            Ok(())
        } else {
            Err(Error::Protocol(format!("rectangle {x},{y} {w}x{h} outside the {}x{} desktop", self.w, self.h)))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(x: usize, y: usize, w: usize, h: usize, px: &[u32]) -> Rect {
        let mut bytes = Vec::new();
        for p in px {
            // 32-bit little-endian, B G R pad.
            bytes.extend_from_slice(&[(p & 0xff) as u8, ((p >> 8) & 0xff) as u8, ((p >> 16) & 0xff) as u8, 0]);
        }
        Rect { x, y, w, h, payload: Payload::Raw(bytes) }
    }

    #[test]
    fn raw_rectangle_paints_its_pixels() {
        let mut fb = Framebuffer::new(4, 3);
        fb.apply(&raw(1, 1, 2, 2, &[0xff0000, 0x00ff00, 0x0000ff, 0xffffff]), &PixelFormat::VIEWER).unwrap();
        assert_eq!(fb.get(1, 1), Some(0xff0000));
        assert_eq!(fb.get(2, 1), Some(0x00ff00));
        assert_eq!(fb.get(1, 2), Some(0x0000ff));
        assert_eq!(fb.get(2, 2), Some(0xffffff));
        assert_eq!(fb.get(0, 0), Some(0));
        assert_eq!(fb.get(4, 0), None);
    }

    #[test]
    fn copy_rect_handles_overlap() {
        let mut fb = Framebuffer::new(3, 1);
        fb.apply(&raw(0, 0, 3, 1, &[1, 2, 3]), &PixelFormat::VIEWER).unwrap();
        // Shift the row right by one: destination (1,0) 2x1 from source (0,0).
        let r = Rect { x: 1, y: 0, w: 2, h: 1, payload: Payload::CopyRect { src_x: 0, src_y: 0 } };
        fb.apply(&r, &PixelFormat::VIEWER).unwrap();
        assert_eq!(fb.px, vec![1, 1, 2]);
    }

    #[test]
    fn desktop_size_resizes_and_clears() {
        let mut fb = Framebuffer::new(2, 2);
        fb.apply(&raw(0, 0, 1, 1, &[7]), &PixelFormat::VIEWER).unwrap();
        fb.apply(&Rect { x: 0, y: 0, w: 5, h: 4, payload: Payload::DesktopSize }, &PixelFormat::VIEWER).unwrap();
        assert_eq!((fb.w, fb.h, fb.px.len()), (5, 4, 20));
        assert!(fb.px.iter().all(|p| *p == 0));
    }

    #[test]
    fn out_of_bounds_and_short_payloads_are_refused() {
        let mut fb = Framebuffer::new(2, 2);
        assert!(fb.apply(&raw(1, 1, 2, 1, &[1, 2]), &PixelFormat::VIEWER).is_err());
        let short = Rect { x: 0, y: 0, w: 2, h: 1, payload: Payload::Raw(vec![0; 4]) };
        assert!(fb.apply(&short, &PixelFormat::VIEWER).is_err());
        let copy = Rect { x: 0, y: 0, w: 2, h: 2, payload: Payload::CopyRect { src_x: 1, src_y: 0 } };
        assert!(fb.apply(&copy, &PixelFormat::VIEWER).is_err());
        assert_eq!(fb.px, vec![0; 4], "a refused rectangle paints nothing");
    }
}
