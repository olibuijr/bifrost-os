//! A GPU-backed render view as a pure model: the drawable size that follows the view's bounds and content scale, the timed
//! or on-demand render loop behind it (`render_loop::RenderLoop`), and the drawable-size changes that a delegate must hear
//! about. The model creates no GPU object and calls no delegate. The caller reports bounds and times, and runs its draw
//! callback when `frame` returns a request.
//!
//! Rules: the drawable is the bounds in points times the content scale, rounded to whole pixels. A zero-sized drawable
//! draws nothing and does not advance the loop. With automatic resizing on (the default), the drawable follows the bounds;
//! with it off, the size is what `set_drawable_size` last set. `set_bounds` and `set_content_scale` report whether the
//! drawable size changed, so the caller can tell its delegate.

use crate::render_loop::RenderLoop;

/// The frame rate a new render view asks for (`preferredFramesPerSecond` defaults to 60).
pub const DEFAULT_FPS: u32 = 60;

/// A drawable size in device pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PixelSize {
    pub width: u32,
    pub height: u32,
}

impl PixelSize {
    /// True when the drawable has no area and so cannot be drawn into.
    pub fn is_empty(self) -> bool {
        self.width == 0 || self.height == 0
    }
}

/// Returned by [`RenderView::frame`] when the loop asks for a draw. The pass draws into `size`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DrawRequest {
    pub size: PixelSize,
}

/// A render view: bounds, content scale, drawable size and the loop that draws it.
#[derive(Clone, Debug, PartialEq)]
pub struct RenderView {
    width: f64,
    height: f64,
    content_scale: f64,
    auto_resize: bool,
    drawable: PixelSize,
    render_loop: RenderLoop,
}

impl RenderView {
    /// A view of `width` by `height` points at `content_scale` device pixels per point. Negative or non-finite sizes count
    /// as zero, and a scale that is not finite and positive counts as 1. The loop is timed at `DEFAULT_FPS`.
    pub fn new(width: f64, height: f64, content_scale: f64) -> Self {
        let mut view = RenderView {
            width: sanitize_points(width),
            height: sanitize_points(height),
            content_scale: sanitize_scale(content_scale),
            auto_resize: true,
            drawable: PixelSize { width: 0, height: 0 },
            render_loop: RenderLoop::new(DEFAULT_FPS),
        };
        view.drawable = view.bounds_pixels();
        view
    }

    /// The drawable size the next pass draws into.
    pub fn drawable_size(&self) -> PixelSize {
        self.drawable
    }

    /// `autoResizeDrawable`: whether the drawable follows the bounds and content scale.
    pub fn auto_resize(&self) -> bool {
        self.auto_resize
    }

    /// Turns automatic resizing on or off. Turning it on takes the size of the bounds at once. Returns true when the
    /// drawable size changed.
    pub fn set_auto_resize(&mut self, on: bool) -> bool {
        self.auto_resize = on;
        if on {
            self.follow_bounds()
        } else {
            false
        }
    }

    /// Sets the drawable size by hand. It applies only while automatic resizing is off, and returns true when the size
    /// changed. A zero size is kept, and the view draws nothing until a size with area is set.
    pub fn set_drawable_size(&mut self, width: u32, height: u32) -> bool {
        if self.auto_resize {
            return false;
        }
        let next = PixelSize { width, height };
        let changed = next != self.drawable;
        self.drawable = next;
        changed
    }

    /// Reports the view's bounds in points. Returns true when the drawable size changed.
    pub fn set_bounds(&mut self, width: f64, height: f64) -> bool {
        self.width = sanitize_points(width);
        self.height = sanitize_points(height);
        self.follow_bounds()
    }

    /// The content scale in device pixels per point (`contentScaleFactor`). Returns true when the drawable size changed.
    pub fn set_content_scale(&mut self, scale: f64) -> bool {
        self.content_scale = sanitize_scale(scale);
        self.follow_bounds()
    }

    /// The frame rate the loop asks for.
    pub fn preferred_fps(&self) -> u32 {
        self.render_loop.preferred_fps()
    }

    /// Changes the frame rate, clamped to the loop's range.
    pub fn set_preferred_fps(&mut self, fps: u32) {
        self.render_loop.set_preferred_fps(fps);
    }

    /// `isPaused`: whether the loop is paused by the program or by the app.
    pub fn is_paused(&self) -> bool {
        self.render_loop.is_paused()
    }

    /// `isPaused` setter: the program pauses or resumes the loop.
    pub fn set_paused(&mut self, paused: bool) {
        self.render_loop.set_paused(paused);
    }

    /// `enableSetNeedsDisplay`: on-demand drawing. A view in this mode draws only after `request_redraw`.
    pub fn set_on_demand(&mut self, on_demand: bool) {
        self.render_loop.set_on_demand(on_demand);
    }

    /// `setNeedsDisplay`: asks an on-demand view for one pass. Several requests before the next `frame` make one pass.
    pub fn request_redraw(&mut self) {
        self.render_loop.request_redraw();
    }

    /// Reports the time `now` in seconds. Returns the pass to draw, or None when no pass is due or the drawable is empty.
    /// An empty drawable never advances the loop, so a view that is hidden and then shown draws again at once.
    pub fn frame(&mut self, now: f64) -> Option<DrawRequest> {
        if self.drawable.is_empty() {
            return None;
        }
        if self.render_loop.tick(now) {
            Some(DrawRequest { size: self.drawable })
        } else {
            None
        }
    }

    fn bounds_pixels(&self) -> PixelSize {
        PixelSize {
            width: to_pixels(self.width, self.content_scale),
            height: to_pixels(self.height, self.content_scale),
        }
    }

    /// Moves the drawable to the bounds when automatic resizing is on. Returns true when the size changed.
    fn follow_bounds(&mut self) -> bool {
        if !self.auto_resize {
            return false;
        }
        let next = self.bounds_pixels();
        let changed = next != self.drawable;
        self.drawable = next;
        changed
    }
}

fn sanitize_points(value: f64) -> f64 {
    if value.is_finite() && value > 0.0 {
        value
    } else {
        0.0
    }
}

fn sanitize_scale(value: f64) -> f64 {
    if value.is_finite() && value > 0.0 {
        value
    } else {
        1.0
    }
}

fn to_pixels(points: f64, scale: f64) -> u32 {
    (points * scale).round() as u32
}
