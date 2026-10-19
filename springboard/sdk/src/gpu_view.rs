//! A GPU view for apps: the app-facing counterpart of a view that owns a drawable loop and calls a delegate to draw. A
//! [`GpuView`] joins the render view model (`bifrost_ui::render_view`: drawable size from bounds and content scale, timed
//! or on-demand loop, pausing), the drawable pool (`drawable_pool`) and a [`GpuViewDelegate`]. The caller reports bounds,
//! scale and time; `frame` asks the loop whether a pass is due, takes a free drawable, clears it to the clear colour, calls
//! `draw` with a [`Frame`], and queues the drawn drawable for the presenter, which `take_presented`, reads with `image`
//! and hands back with `release`.
//!
//! The drawables are CPU images (premultiplied RGBA, see `bifrost_render::gfx`), so a view runs and is tested without a
//! GPU. The shell's presenter uploads a presented image today; a GPU device handle for apps is not part of this
//! module. The delegate hears about the drawable size once before the first pass and again after each change, even while
//! the loop is paused. A size change waits for the presenter to release every drawable before the pool is rebuilt.
//! See docs/bifrost-ui.md "GPU kits" for the decisions and the gaps.
use std::collections::VecDeque;

use bifrost_render::gfx::{rgba, Image};
use bifrost_ui::drawable_setup::{Drawable as DrawableRequest, DevicePreference, DrawableSetup, SetupError};
use bifrost_ui::render_view::{PixelSize, RenderView};

use crate::drawable_pool::{DrawableId, DrawablePool, PoolError, MAX_DRAWABLES, MIN_DRAWABLES};

pub use bifrost_ui::drawable_setup::{FORMAT_ALPHA, FORMAT_OPAQUE};

/// The longest side a drawable may have.
pub const MAX_DRAWABLE_SIDE: u32 = 16384;
/// The most pixels one drawable may have; the view keeps one image of this size per drawable.
pub const MAX_DRAWABLE_PIXELS: u64 = 1 << 26;

/// Why a view call was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ViewError {
    /// The drawable request was refused (size, device node).
    Setup(SetupError),
    /// The drawable count is not 2 or 3, or the pool refused a call.
    Pool(PoolError),
    /// The drawable has more pixels than `MAX_DRAWABLE_PIXELS`.
    TooManyPixels { pixels: u64 },
}

impl core::fmt::Display for ViewError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            ViewError::Setup(e) => write!(f, "view: {e}"),
            ViewError::Pool(e) => write!(f, "view: {e}"),
            ViewError::TooManyPixels { pixels } => write!(f, "view: a drawable of {pixels} pixels is above {MAX_DRAWABLE_PIXELS}"),
        }
    }
}

impl std::error::Error for ViewError {}

/// What `frame` did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrameOutcome {
    /// The delegate drew into drawable `id` as frame number `index` (counting from 0); it is queued for the presenter.
    Drawn { id: DrawableId, index: u64 },
    /// No pass was due: the loop is paused, on demand without a request, or between frames.
    NotDue,
    /// The drawable has no area, so nothing was drawn.
    EmptyDrawable,
    /// A pass was due, but no drawable was free (the presenter still holds them all) or a size change is waiting for
    /// the presenter to release them. The pass is dropped.
    NoDrawable,
}

/// One pass the delegate draws.
pub struct Frame<'a> {
    /// The drawable size in device pixels.
    pub size: PixelSize,
    /// Device pixels per point.
    pub content_scale: f64,
    /// The time the caller reported for this pass, in seconds.
    pub time: f64,
    /// The number of passes drawn before this one.
    pub index: u64,
    /// The drawable, already cleared to the view's clear colour.
    pub target: &'a mut Image,
}

/// The object that draws a view's passes.
pub trait GpuViewDelegate {
    /// The drawable size is about to change to `size` (and is reported once before the first pass). Called even while
    /// the loop is paused.
    fn drawable_size_will_change(&mut self, _size: PixelSize) {}

    /// Draws one pass into `frame.target`.
    fn draw(&mut self, frame: &mut Frame);
}

/// A view with a drawable loop, a pool of drawables and a delegate that draws.
pub struct GpuView {
    view: RenderView,
    content_scale: f64,
    opaque: bool,
    device: DevicePreference,
    clear: u32,
    buffers: usize,
    pool: Option<DrawablePool>,
    images: Vec<Image>,
    notified: Option<PixelSize>,
    presented: VecDeque<DrawableId>,
    drawn: u64,
}

impl GpuView {
    /// A view of `width` by `height` points at `content_scale` device pixels per point, with `buffers` drawables (2 or 3).
    /// The clear colour is opaque black, the drawable is opaque, and the loop is timed at 60 passes a second.
    pub fn new(width: f64, height: f64, content_scale: f64, buffers: usize) -> Result<GpuView, ViewError> {
        if !(MIN_DRAWABLES..=MAX_DRAWABLES).contains(&buffers) {
            return Err(ViewError::Pool(PoolError::CountOutOfRange(buffers)));
        }
        let scale = if content_scale.is_finite() && content_scale > 0.0 { content_scale } else { 1.0 };
        Ok(GpuView {
            view: RenderView::new(width, height, scale),
            content_scale: scale,
            opaque: true,
            device: DevicePreference::Default,
            clear: rgba(0, 0, 0, 255),
            buffers,
            pool: None,
            images: Vec::new(),
            notified: None,
            presented: VecDeque::new(),
            drawn: 0,
        })
    }

    /// The drawable size the next pass draws into.
    pub fn drawable_size(&self) -> PixelSize {
        self.view.drawable_size()
    }

    /// The content scale in device pixels per point.
    pub fn content_scale(&self) -> f64 {
        self.content_scale
    }

    /// Reports the view's bounds in points. The delegate hears about a changed drawable size at the next `frame` or `draw`.
    pub fn set_bounds(&mut self, width: f64, height: f64) {
        self.view.set_bounds(width, height);
    }

    /// Reports the content scale. A value that is not finite and positive counts as 1.
    pub fn set_content_scale(&mut self, scale: f64) {
        self.content_scale = if scale.is_finite() && scale > 0.0 { scale } else { 1.0 };
        self.view.set_content_scale(self.content_scale);
    }

    /// Whether the drawable follows the bounds and scale (the default).
    pub fn set_auto_resize(&mut self, on: bool) {
        self.view.set_auto_resize(on);
    }

    /// Sets the drawable size by hand; applies only while automatic resizing is off.
    pub fn set_drawable_size(&mut self, width: u32, height: u32) {
        self.view.set_drawable_size(width, height);
    }

    /// The colour the drawable is cleared to before each pass, straight (not premultiplied) components in 0..1.
    pub fn set_clear_color(&mut self, rgba_straight: [f32; 4]) {
        let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
        self.clear = rgba(q(rgba_straight[0]), q(rgba_straight[1]), q(rgba_straight[2]), q(rgba_straight[3]));
    }

    /// Whether the drawable has no alpha channel (the default). A transparent view lets what is behind it show.
    pub fn set_opaque(&mut self, opaque: bool) {
        self.opaque = opaque;
    }

    /// The device the drawable is made on.
    pub fn set_preferred_device(&mut self, device: DevicePreference) {
        self.device = device;
    }

    /// The request the shell allocates for the current drawable: size, format code and device. Refused when the size is
    /// zero or above `MAX_DRAWABLE_SIDE`, or the device is not a render node path.
    pub fn drawable_request(&self) -> Result<DrawableRequest, SetupError> {
        let size = self.view.drawable_size();
        DrawableSetup { width: size.width, height: size.height, alpha: !self.opaque, device: self.device.clone() }.validate(MAX_DRAWABLE_SIDE)
    }

    /// The loop's frame rate in passes a second (1 to 120).
    pub fn set_preferred_fps(&mut self, fps: u32) {
        self.view.set_preferred_fps(fps);
    }

    pub fn preferred_fps(&self) -> u32 {
        self.view.preferred_fps()
    }

    /// The program pauses or resumes the loop.
    pub fn set_paused(&mut self, paused: bool) {
        self.view.set_paused(paused);
    }

    pub fn is_paused(&self) -> bool {
        self.view.is_paused()
    }

    /// On-demand drawing: the view draws only after `request_redraw`.
    pub fn set_on_demand(&mut self, on_demand: bool) {
        self.view.set_on_demand(on_demand);
    }

    /// Asks an on-demand view for one pass at its next `frame`.
    pub fn request_redraw(&mut self) {
        self.view.request_redraw();
    }

    /// Whether the loop pauses itself when the app resigns active.
    pub fn set_pause_on_resign(&mut self, on: bool) {
        self.view.set_pause_on_resign(on);
    }

    /// Whether the loop resumes itself when the app becomes active.
    pub fn set_resume_on_active(&mut self, on: bool) {
        self.view.set_resume_on_active(on);
    }

    pub fn app_resigned_active(&mut self) {
        self.view.app_resigned_active();
    }

    pub fn app_became_active(&mut self) {
        self.view.app_became_active();
    }

    /// Reports the time `now` in seconds and draws a pass when the loop asks for one.
    pub fn frame(&mut self, now: f64, delegate: &mut dyn GpuViewDelegate) -> Result<FrameOutcome, ViewError> {
        self.sync_size(delegate);
        if self.view.drawable_size().is_empty() {
            return Ok(FrameOutcome::EmptyDrawable);
        }
        match self.view.frame(now) {
            Some(request) => self.draw_into(delegate, now, request.size),
            None => Ok(FrameOutcome::NotDue),
        }
    }

    /// Draws a pass now, whatever the loop says (`draw()`). Use it for a still capture or after a hand-made change.
    pub fn draw(&mut self, now: f64, delegate: &mut dyn GpuViewDelegate) -> Result<FrameOutcome, ViewError> {
        self.sync_size(delegate);
        let size = self.view.drawable_size();
        if size.is_empty() {
            return Ok(FrameOutcome::EmptyDrawable);
        }
        self.draw_into(delegate, now, size)
    }

    /// The next drawn drawable for the presenter, oldest first.
    pub fn take_presented(&mut self) -> Option<DrawableId> {
        self.presented.pop_front()
    }

    /// How many drawn drawables wait for the presenter.
    pub fn presented_count(&self) -> usize {
        self.presented.len()
    }

    /// The pixels of a drawable, for the presenter to read.
    pub fn image(&self, id: DrawableId) -> Option<&Image> {
        self.images.get(id.0)
    }

    /// The presenter is done with a drawable.
    pub fn release(&mut self, id: DrawableId) -> Result<(), ViewError> {
        match self.pool.as_mut() {
            Some(pool) => pool.release(id).map_err(ViewError::Pool),
            None => Err(ViewError::Pool(PoolError::WrongState(id))),
        }
    }

    /// Passes drawn so far.
    pub fn frames_drawn(&self) -> u64 {
        self.drawn
    }

    fn sync_size(&mut self, delegate: &mut dyn GpuViewDelegate) {
        let size = self.view.drawable_size();
        if !size.is_empty() && self.notified != Some(size) {
            self.notified = Some(size);
            delegate.drawable_size_will_change(size);
        }
    }

    /// Makes the pool match `size`. False when a size change must wait for the presenter to release its drawables.
    fn ensure_pool(&mut self, size: PixelSize) -> Result<bool, ViewError> {
        if let Some(pool) = &self.pool {
            if pool.size() == (size.width, size.height) {
                return Ok(true);
            }
            if pool.free_count() != pool.count() {
                return Ok(false);
            }
        }
        DrawableSetup { width: size.width, height: size.height, alpha: !self.opaque, device: self.device.clone() }
            .validate(MAX_DRAWABLE_SIDE)
            .map_err(ViewError::Setup)?;
        let pixels = u64::from(size.width) * u64::from(size.height);
        if pixels > MAX_DRAWABLE_PIXELS {
            return Err(ViewError::TooManyPixels { pixels });
        }
        let pool = DrawablePool::new(self.buffers, size.width, size.height).map_err(ViewError::Pool)?;
        self.images = (0..self.buffers).map(|_| Image::new(size.width as usize, size.height as usize)).collect();
        self.pool = Some(pool);
        self.presented.clear();
        Ok(true)
    }

    fn draw_into(&mut self, delegate: &mut dyn GpuViewDelegate, now: f64, size: PixelSize) -> Result<FrameOutcome, ViewError> {
        if !self.ensure_pool(size)? {
            return Ok(FrameOutcome::NoDrawable);
        }
        let Some(pool) = self.pool.as_mut() else { return Ok(FrameOutcome::NoDrawable) };
        let Some(id) = pool.next_drawable() else { return Ok(FrameOutcome::NoDrawable) };
        let index = self.drawn;
        let target = &mut self.images[id.0];
        target.px.fill(self.clear);
        let mut frame = Frame { size, content_scale: self.content_scale, time: now, index, target };
        delegate.draw(&mut frame);
        pool.present(id).map_err(ViewError::Pool)?;
        self.presented.push_back(id);
        self.drawn += 1;
        Ok(FrameOutcome::Drawn { id, index })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct Recorder {
        sizes: Vec<PixelSize>,
        draws: Vec<(PixelSize, f64, u64)>,
        paint: Option<u32>,
    }

    impl GpuViewDelegate for Recorder {
        fn drawable_size_will_change(&mut self, size: PixelSize) {
            self.sizes.push(size);
        }

        fn draw(&mut self, frame: &mut Frame) {
            self.draws.push((frame.size, frame.time, frame.index));
            if let Some(p) = self.paint {
                frame.target.px[0] = p;
            }
        }
    }

    fn size(width: u32, height: u32) -> PixelSize {
        PixelSize { width, height }
    }

    fn drawn(outcome: FrameOutcome) -> DrawableId {
        match outcome {
            FrameOutcome::Drawn { id, .. } => id,
            other => panic!("expected a drawn pass, got {other:?}"),
        }
    }

    #[test]
    fn the_first_pass_reports_the_size_clears_and_queues_the_drawable() {
        let mut view = GpuView::new(100.0, 50.0, 2.0, 2).unwrap();
        view.set_clear_color([1.0, 0.0, 0.0, 1.0]);
        let mut d = Recorder { paint: Some(0xff12_3456), ..Recorder::default() };
        let id = drawn(view.frame(0.0, &mut d).unwrap());
        assert_eq!(d.sizes, vec![size(200, 100)]);
        assert_eq!(d.draws, vec![(size(200, 100), 0.0, 0)]);
        assert_eq!(view.take_presented(), Some(id));
        assert_eq!(view.take_presented(), None);
        let img = view.image(id).unwrap();
        assert_eq!((img.w, img.h), (200, 100));
        assert_eq!(img.px[0], 0xff12_3456, "the delegate's pixel");
        assert_eq!(img.px[1], rgba(255, 0, 0, 255), "the rest is the clear colour");
        view.release(id).unwrap();
        assert_eq!(view.frames_drawn(), 1);
    }

    #[test]
    fn a_timed_loop_draws_at_its_rate_and_skips_early_calls() {
        let mut view = GpuView::new(10.0, 10.0, 1.0, 3).unwrap();
        let mut d = Recorder::default();
        let a = drawn(view.frame(0.0, &mut d).unwrap());
        assert_eq!(view.frame(0.001, &mut d).unwrap(), FrameOutcome::NotDue);
        view.take_presented();
        view.release(a).unwrap();
        let b = view.frame(1.0 / 60.0, &mut d).unwrap();
        assert!(matches!(b, FrameOutcome::Drawn { index: 1, .. }));
        assert_eq!(d.draws.len(), 2);
    }

    #[test]
    fn the_pool_runs_dry_when_the_presenter_keeps_every_drawable() {
        let mut view = GpuView::new(4.0, 4.0, 1.0, 2).unwrap();
        view.set_preferred_fps(120);
        let mut d = Recorder::default();
        let first = drawn(view.frame(0.0, &mut d).unwrap());
        drawn(view.frame(1.0, &mut d).unwrap());
        assert_eq!(view.frame(2.0, &mut d).unwrap(), FrameOutcome::NoDrawable);
        assert_eq!(d.draws.len(), 2, "the dropped pass never reaches the delegate");
        assert_eq!(view.presented_count(), 2);
        assert_eq!(view.take_presented(), Some(first));
        view.release(first).unwrap();
        assert!(matches!(view.frame(3.0, &mut d).unwrap(), FrameOutcome::Drawn { .. }));
    }

    #[test]
    fn a_size_change_is_reported_even_while_paused_and_waits_for_the_presenter() {
        let mut view = GpuView::new(10.0, 10.0, 1.0, 2).unwrap();
        let mut d = Recorder::default();
        let id = drawn(view.frame(0.0, &mut d).unwrap());
        view.set_paused(true);
        view.set_bounds(20.0, 10.0);
        assert_eq!(view.frame(1.0, &mut d).unwrap(), FrameOutcome::NotDue);
        assert_eq!(d.sizes, vec![size(10, 10), size(20, 10)]);
        view.set_paused(false);
        // The first drawable is still with the presenter, so the pool cannot be rebuilt yet.
        assert_eq!(view.frame(2.0, &mut d).unwrap(), FrameOutcome::NoDrawable);
        view.take_presented();
        view.release(id).unwrap();
        let again = drawn(view.draw(3.0, &mut d).unwrap());
        let img = view.image(again).unwrap();
        assert_eq!((img.w, img.h), (20, 10));
        assert_eq!(d.sizes.len(), 2, "the delegate is told once per change");
    }

    #[test]
    fn an_empty_drawable_draws_nothing() {
        let mut view = GpuView::new(0.0, 50.0, 2.0, 2).unwrap();
        let mut d = Recorder::default();
        assert_eq!(view.frame(0.0, &mut d).unwrap(), FrameOutcome::EmptyDrawable);
        assert_eq!(view.draw(0.0, &mut d).unwrap(), FrameOutcome::EmptyDrawable);
        assert!(d.sizes.is_empty() && d.draws.is_empty());
        view.set_bounds(5.0, 5.0);
        assert!(matches!(view.frame(0.0, &mut d).unwrap(), FrameOutcome::Drawn { .. }));
    }

    #[test]
    fn an_on_demand_view_draws_only_when_asked_and_draw_ignores_the_loop() {
        let mut view = GpuView::new(8.0, 8.0, 1.0, 3).unwrap();
        view.set_on_demand(true);
        let mut d = Recorder::default();
        assert_eq!(view.frame(0.0, &mut d).unwrap(), FrameOutcome::NotDue);
        view.request_redraw();
        view.request_redraw();
        assert!(matches!(view.frame(0.1, &mut d).unwrap(), FrameOutcome::Drawn { .. }));
        assert_eq!(view.frame(0.2, &mut d).unwrap(), FrameOutcome::NotDue, "two requests made one pass");
        view.set_paused(true);
        assert!(matches!(view.draw(0.3, &mut d).unwrap(), FrameOutcome::Drawn { .. }), "draw works on a paused view");
        assert_eq!(d.draws.len(), 2);
    }

    #[test]
    fn the_loop_pauses_with_the_app_when_asked() {
        let mut view = GpuView::new(8.0, 8.0, 1.0, 2).unwrap();
        view.set_pause_on_resign(true);
        view.set_resume_on_active(true);
        view.app_resigned_active();
        assert!(view.is_paused());
        let mut d = Recorder::default();
        assert_eq!(view.frame(0.0, &mut d).unwrap(), FrameOutcome::NotDue);
        view.app_became_active();
        assert!(!view.is_paused());
        assert!(matches!(view.frame(0.1, &mut d).unwrap(), FrameOutcome::Drawn { .. }));
    }

    #[test]
    fn drawable_requests_follow_the_view_and_refuse_bad_ones() {
        let mut view = GpuView::new(100.0, 50.0, 1.5, 2).unwrap();
        let req = view.drawable_request().unwrap();
        assert_eq!((req.width, req.height, req.format), (150, 75, FORMAT_OPAQUE));
        view.set_opaque(false);
        assert_eq!(view.drawable_request().unwrap().format, FORMAT_ALPHA);
        view.set_preferred_device(DevicePreference::RenderNode("/dev/dri/renderD128".into()));
        assert!(view.drawable_request().is_ok());
        view.set_preferred_device(DevicePreference::RenderNode("/etc/passwd".into()));
        assert_eq!(view.drawable_request(), Err(SetupError::BadRenderNode("/etc/passwd".into())));
        view.set_preferred_device(DevicePreference::Default);
        view.set_bounds(20000.0, 10.0);
        assert_eq!(view.drawable_request(), Err(SetupError::TooLarge { side: 30000, max: MAX_DRAWABLE_SIDE }));
        let mut d = Recorder::default();
        assert_eq!(view.frame(0.0, &mut d), Err(ViewError::Setup(SetupError::TooLarge { side: 30000, max: MAX_DRAWABLE_SIDE })));
    }

    #[test]
    fn a_drawable_with_too_many_pixels_is_refused_before_any_image_is_made() {
        let mut view = GpuView::new(16000.0, 16000.0, 1.0, 2).unwrap();
        let mut d = Recorder::default();
        assert_eq!(view.frame(0.0, &mut d), Err(ViewError::TooManyPixels { pixels: 256_000_000 }));
        assert!(d.draws.is_empty());
    }

    #[test]
    fn the_buffer_count_and_the_scale_are_checked() {
        assert!(matches!(GpuView::new(1.0, 1.0, 1.0, 1), Err(ViewError::Pool(PoolError::CountOutOfRange(1)))));
        assert!(matches!(GpuView::new(1.0, 1.0, 1.0, 4), Err(ViewError::Pool(PoolError::CountOutOfRange(4)))));
        let view = GpuView::new(10.0, 10.0, f64::NAN, 2).unwrap();
        assert_eq!(view.content_scale(), 1.0);
        assert_eq!(view.drawable_size(), size(10, 10));
    }

    #[test]
    fn releasing_without_a_pool_or_a_queued_drawable_is_refused() {
        let mut view = GpuView::new(4.0, 4.0, 1.0, 2).unwrap();
        assert!(view.release(DrawableId(0)).is_err());
        let mut d = Recorder::default();
        view.frame(0.0, &mut d).unwrap();
        assert_eq!(view.release(DrawableId(1)), Err(ViewError::Pool(PoolError::WrongState(DrawableId(1)))));
        assert_eq!(view.release(DrawableId(9)), Err(ViewError::Pool(PoolError::WrongState(DrawableId(9)))));
    }
}
