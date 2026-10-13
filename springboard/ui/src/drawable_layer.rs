//! A drawable surface an app renders into and presents to the window server.
//!
//! A [`DrawableLayer`] owns a small pool of pixel buffers (up to three). The app takes one with `next_drawable`,
//! draws into its pixels, and hands it back with `present`. The host (shell or SDK frame loop) takes the latest presented frame with
//! `take_presented` and composes it: on the GPU path the compositor uploads it as a layer, on the CPU fallback the
//! shell blits it. The pixels are always plain `u32`s, so both paths share one code path in the app. Drive drawing
//! from a [`crate::frame_clock::FrameLink`] made with [`DrawableLayer::display_link`].

use crate::frame_clock::{FrameLink, FrameTick};

use crate::frame_rate::FrameRateRange;

/// Pixel layouts a drawable can have (8 bits a channel).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PixelFormat {
    Bgra8,
    Rgba8,
}

/// Buffers in flight at once.
pub const MAX_DRAWABLES: usize = 3;

/// One buffer to draw into.
#[derive(Debug)]
pub struct Drawable {
    pub width: usize,
    pub height: usize,
    pub format: PixelFormat,
    /// Row-major pixels, `width * height` of them.
    pub pixels: Vec<u32>,
}

#[derive(Debug)]
pub struct DrawableLayer {
    pub pixel_format: PixelFormat,
    size: (usize, usize),
    live: usize,
    free: Vec<Vec<u32>>,
    presented: Option<Drawable>,
    pub presented_count: u64,
}

impl DrawableLayer {
    /// A layer of `width` x `height`; `None` for a zero size.
    pub fn new(pixel_format: PixelFormat, width: usize, height: usize) -> Option<DrawableLayer> {
        (width > 0 && height > 0).then(|| DrawableLayer { pixel_format, size: (width, height), live: 0, free: Vec::new(), presented: None, presented_count: 0 })
    }
    pub fn drawable_size(&self) -> (usize, usize) {
        self.size
    }
    /// Changes the size; buffers of the old size are dropped. A zero size is ignored.
    pub fn set_drawable_size(&mut self, width: usize, height: usize) {
        if width > 0 && height > 0 && (width, height) != self.size {
            self.size = (width, height);
            self.free.clear();
        }
    }
    /// A cleared buffer to draw into, or `None` while all [`MAX_DRAWABLES`] are in use (draw next frame).
    pub fn next_drawable(&mut self) -> Option<Drawable> {
        if self.live >= MAX_DRAWABLES {
            return None;
        }
        let n = self.size.0 * self.size.1;
        let mut pixels = self.free.pop().unwrap_or_default();
        pixels.clear();
        pixels.resize(n, 0);
        self.live += 1;
        Some(Drawable { width: self.size.0, height: self.size.1, format: self.pixel_format, pixels })
    }
    /// Queues `d` for display. A frame still waiting is replaced (the newest wins) and its buffer reused.
    pub fn present(&mut self, d: Drawable) {
        self.presented_count += 1;
        if let Some(old) = self.presented.replace(d) {
            self.free.push(old.pixels);
            self.live = self.live.saturating_sub(1);
        }
    }
    /// Gives a buffer back without presenting it.
    pub fn discard(&mut self, d: Drawable) {
        self.free.push(d.pixels);
        self.live = self.live.saturating_sub(1);
    }
    /// The host side: the newest presented frame as an image. `None` when nothing new was presented.
    pub fn take_presented(&mut self) -> Option<Drawable> {
        let d = self.presented.take()?;
        self.live = self.live.saturating_sub(1);
        Some(d)
    }
    /// Returns an image from `take_presented` for reuse once the host is done with it.
    pub fn recycle(&mut self, d: Drawable) {
        if (d.width, d.height) == self.size {
            self.free.push(d.pixels);
        }
    }
    /// A link that calls `draw` at `range` (the app draws and presents inside it).
    pub fn display_link(range: FrameRateRange, draw: impl FnMut(&FrameTick) + 'static) -> FrameLink {
        let mut l = FrameLink::new(draw);
        l.preferred_frame_rate_range = range;
        l
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_zero_size_is_refused_and_resizing_keeps_the_old_size_on_zero() {
        assert!(DrawableLayer::new(PixelFormat::Bgra8, 0, 4).is_none());
        let mut l = DrawableLayer::new(PixelFormat::Bgra8, 4, 2).unwrap();
        l.set_drawable_size(0, 9);
        assert_eq!(l.drawable_size(), (4, 2));
        l.set_drawable_size(8, 2);
        assert_eq!(l.next_drawable().unwrap().pixels.len(), 16);
    }

    #[test]
    fn present_then_take_returns_the_frame() {
        let mut l = DrawableLayer::new(PixelFormat::Rgba8, 2, 2).unwrap();
        let mut d = l.next_drawable().unwrap();
        d.pixels[3] = 7;
        l.present(d);
        let img = l.take_presented().unwrap();
        assert_eq!((img.width, img.height, img.pixels[3]), (2, 2, 7));
        assert!(l.take_presented().is_none());
        l.recycle(img);
        assert!(l.next_drawable().unwrap().pixels.iter().all(|p| *p == 0), "a recycled buffer is cleared");
    }

    #[test]
    fn the_pool_runs_dry_at_three_and_refills_on_present() {
        let mut l = DrawableLayer::new(PixelFormat::Bgra8, 1, 1).unwrap();
        let a = l.next_drawable().unwrap();
        let b = l.next_drawable().unwrap();
        let c = l.next_drawable().unwrap();
        assert!(l.next_drawable().is_none());
        l.discard(c);
        assert!(l.next_drawable().is_some());
        l.present(a);
        l.present(b);
        assert_eq!(l.presented_count, 2);
    }

    #[test]
    fn the_link_carries_the_range() {
        let l = DrawableLayer::display_link(FrameRateRange::fixed(30.0), |_| {});
        assert_eq!(l.preferred_frame_rate_range, FrameRateRange::fixed(30.0));
    }
}
