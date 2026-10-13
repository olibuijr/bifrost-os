//! See `wl/mod.rs` for this module's contract and owner.
//!
//! One frame of the direct path. `FrameLoop::draw_frame` runs the app's drawing into a CPU surface (`App::draw` still
//! takes a `Surface`), uploads it as one image op of a `DisplayList`, renders that with `bifrost-2d` into a backing store
//! from the sink, binds the store to the root layer and commits one layer transaction. The sink (the connection and the
//! backing-store pool, owned by other agents) is a trait here so this module needs neither.
//!
//! Pacing: [`Pacer`] wraps the frame clock, [`DisplayLink`], and skips a refresh when `MAX_IN_FLIGHT` frames are not yet
//! shown. The compositor's `presented` feedback moves the clock's phase ([`Pacer::presented`]).
//!
//! Known limits (todo_left): the surface is copied into the display list each frame (one full-window image op, no
//! damage yet), so the whole window is redrawn; the shared-memory fallback is not part of this module.

use crate::display_link::{DisplayLink, TimeStamp};
use bifrost_2d::{FrameStats, Renderer};
use bifrost_gpu::{CommandQueue, Device, Error as GpuError, Submission, Texture};
use bifrost_layers::{Change, ContentsId, LayerContents, LayerId, Property, Rect as LayerRect, Transaction, Value};
use bifrost_render::canvas::{Canvas, ImageRef, RectF};
use bifrost_render::display_list::DisplayList;
use bifrost_render::gfx::{Damage, Image, Surface};

/// Frames committed and not yet shown before the loop skips a refresh.
pub const MAX_IN_FLIGHT: u32 = 2;

/// Why a frame failed: the GPU, or the sink (binding or commit).
#[derive(Debug)]
pub enum FrameError {
    Gpu(GpuError),
    Sink(String),
}

impl From<GpuError> for FrameError {
    fn from(e: GpuError) -> Self {
        FrameError::Gpu(e)
    }
}

impl std::fmt::Display for FrameError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FrameError::Gpu(e) => write!(f, "{e}"),
            FrameError::Sink(s) => write!(f, "frame sink: {s}"),
        }
    }
}

impl std::error::Error for FrameError {}

/// The connection side of a frame: the backing-store pool and the layer commit.
pub trait FrameSink {
    /// A free backing store of `width` x `height` device pixels, and the contents id it is bound to. The sink pools
    /// them (2 or 3 per layer) and gives back one that the compositor has released.
    fn acquire(&mut self, width: u32, height: u32) -> Result<(ContentsId, Texture), FrameError>;

    /// Binds `contents` to `texture` (once per pooled store), makes the compositor wait for `work` (its sync_file), and
    /// commits `tx` as one transaction.
    fn commit(&mut self, contents: ContentsId, texture: &Texture, work: &Submission, tx: &Transaction) -> Result<(), FrameError>;
}

/// The host time of a `presented` event: seconds as two 32-bit halves plus nanoseconds, in host nanoseconds.
pub fn presented_host_time(tv_sec_hi: u32, tv_sec_lo: u32, tv_nsec: u32) -> u64 {
    let secs = (u64::from(tv_sec_hi) << 32) | u64::from(tv_sec_lo);
    secs.saturating_mul(1_000_000_000).saturating_add(u64::from(tv_nsec))
}

/// The layer tree's setup and one frame's change, as one transaction: the first frame creates the root layer and makes
/// it the root, `size` (when set) sets its bounds, and every frame binds the store it drew into.
pub fn frame_transaction(seq: u64, root: LayerId, first: bool, size: Option<(u32, u32)>, contents: ContentsId) -> Transaction {
    let mut tx = Transaction::new(seq);
    if first {
        tx.push(Change::Create(root));
        tx.push(Change::SetRoot(root));
    }
    if let Some((w, h)) = size {
        tx.push(Change::Set {
            layer: root,
            key: Property::Bounds,
            value: Value::Rect(LayerRect { x: 0.0, y: 0.0, w: w as f32, h: h as f32 }),
        });
    }
    tx.push(Change::SetContents(root, LayerContents::Buffer(contents)));
    tx
}

/// The frame clock and the count of frames not yet shown.
#[derive(Debug)]
pub struct Pacer {
    link: DisplayLink,
    in_flight: u32,
}

impl Pacer {
    /// A stopped pacer for a display at `refresh_hz`. None for a rate of zero.
    pub fn new(refresh_hz: u32) -> Option<Pacer> {
        Some(Pacer { link: DisplayLink::new(refresh_hz)?, in_flight: 0 })
    }

    /// Starts the clock: refresh 0 is at `host_time`.
    pub fn start(&mut self, host_time: u64) {
        self.link.start(host_time);
    }

    /// Stops the clock. No refresh is reported until the next `start`.
    pub fn stop(&mut self) {
        self.link.stop();
    }

    /// The refresh to draw for at `host_time`, or None when no new refresh has begun or `MAX_IN_FLIGHT` frames are
    /// still waiting to be shown (the refresh is then skipped, not drawn late).
    pub fn next_frame(&mut self, host_time: u64) -> Option<TimeStamp> {
        let ts = self.link.tick(host_time)?;
        if self.in_flight >= MAX_IN_FLIGHT {
            return None;
        }
        Some(ts)
    }

    /// A frame was committed.
    pub fn committed(&mut self) {
        self.in_flight += 1;
    }

    /// A frame was shown at `host_time` on an output with refresh period `refresh_ns` (0 when unknown).
    pub fn presented(&mut self, host_time: u64, refresh_ns: u64) {
        self.in_flight = self.in_flight.saturating_sub(1);
        self.link.presented(host_time, refresh_ns);
    }

    /// Frames committed and not yet shown.
    pub fn in_flight(&self) -> u32 {
        self.in_flight
    }

    /// The clock, for its missed-refresh count and period.
    pub fn link(&self) -> &DisplayLink {
        &self.link
    }
}

/// What one `draw_frame` did.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FrameReport {
    /// The frame's sequence number (the transaction's `seq`, echoed by `presented`). 0 when nothing was committed.
    pub seq: u64,
    /// The app reported no change: nothing was rendered or committed.
    pub skipped: bool,
    pub stats: FrameStats,
}

/// The direct path's frame loop for one window.
pub struct FrameLoop {
    device: Device,
    queue: CommandQueue,
    renderer: Renderer,
    /// The CPU surface the app draws into (premultiplied RGBA8 as `u32`, stride = width).
    pixels: Vec<u32>,
    width: u32,
    height: u32,
    root: LayerId,
    seq: u64,
    pacer: Pacer,
    /// The root layer exists on the server (its setup went out in a committed transaction).
    created: bool,
    bounds_dirty: bool,
}

impl FrameLoop {
    /// A loop for a window of `width` x `height` device pixels on a display at `refresh_hz`.
    pub fn new(device: &Device, width: u32, height: u32, refresh_hz: u32) -> Result<FrameLoop, FrameError> {
        let pacer = Pacer::new(refresh_hz).ok_or_else(|| FrameError::Gpu(GpuError::Invalid("refresh rate is zero".into())))?;
        Ok(FrameLoop {
            device: device.clone(),
            queue: device.new_command_queue(),
            renderer: Renderer::new(device)?,
            pixels: vec![0; pixel_count(width, height)],
            width,
            height,
            root: LayerId(1),
            seq: 0,
            pacer,
            created: false,
            bounds_dirty: true,
        })
    }

    /// The device this loop draws with.
    pub fn device(&self) -> &Device {
        &self.device
    }

    /// The pacer, for the presentation feedback and the refresh rate.
    pub fn pacer(&self) -> &Pacer {
        &self.pacer
    }

    /// Current window size in device pixels.
    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// The window changed size. The next frame is drawn in full.
    pub fn resize(&mut self, width: u32, height: u32) {
        if (width, height) == (self.width, self.height) {
            return;
        }
        self.width = width;
        self.height = height;
        self.pixels = vec![0; pixel_count(width, height)];
        self.bounds_dirty = true;
    }

    /// The refresh to draw for at `host_time`, if any (see [`Pacer::next_frame`]).
    pub fn tick(&mut self, host_time: u64) -> Option<TimeStamp> {
        self.pacer.next_frame(host_time)
    }

    /// The layer protocol's `presented` event (or `wp_presentation`): feeds the pacer.
    pub fn presented(&mut self, host_time: u64, refresh_ns: u64) {
        self.pacer.presented(host_time, refresh_ns);
    }

    /// The display's refresh rate in hertz, for `Window::set_display_refresh`.
    pub fn refresh_hz(&self) -> f32 {
        1.0e9 / self.pacer.link.refresh_period().max(1) as f32
    }

    /// Presents the newest frame of an app drawable layer (the layer's pixels fill the surface, cropped or padded to
    /// the window size). Skipped when the app presented nothing new.
    pub fn present_drawable<S: FrameSink>(&mut self, sink: &mut S, layer: &mut bifrost_ui::drawable_layer::DrawableLayer) -> Result<FrameReport, FrameError> {
        let Some(d) = layer.take_presented() else {
            return Ok(FrameReport { seq: 0, skipped: true, stats: FrameStats::default() });
        };
        let (w, h) = (self.width as usize, self.height as usize);
        let report = self.draw_frame(sink, |s| {
            for y in 0..h.min(d.height) {
                let n = w.min(d.width);
                s.px[y * s.stride..y * s.stride + n].copy_from_slice(&d.pixels[y * d.width..y * d.width + n]);
            }
            let mut dmg = Damage::new(w, h);
            dmg.all();
            dmg
        });
        layer.recycle(d);
        report
    }

    /// Draws one frame into the CPU surface (the app's `draw`). When nothing changed, the frame is skipped. Otherwise the
    /// surface becomes one image op of a display list, which is presented (see [`FrameLoop::present`]).
    pub fn draw_frame<S: FrameSink>(&mut self, sink: &mut S, draw: impl FnOnce(&mut Surface) -> Damage) -> Result<FrameReport, FrameError> {
        let (w, h) = (self.width, self.height);
        let damage = {
            let mut surface = Surface { w: w as usize, h: h as usize, stride: w as usize, px: &mut self.pixels, y0: 0 };
            draw(&mut surface)
        };
        if frame_skipped(damage.is_empty(), self.created, self.bounds_dirty) {
            return Ok(FrameReport { seq: 0, skipped: true, stats: FrameStats::default() });
        }

        // The surface's pixels become one image of the list. The list keeps its own copy, so the surface is free again.
        let image = Image { w: w as usize, h: h as usize, px: std::mem::take(&mut self.pixels) };
        let mut list = DisplayList::new(w, h);
        list.image(ImageRef { id: self.seq + 1, image: &image }, RectF::new(0.0, 0.0, w as f32, h as f32), 1.0);
        let result = self.present(sink, &list);
        self.pixels = image.px;
        result
    }

    /// Draws one frame through the drawing vocabulary (`App::draw_canvas`): the app records into a display list, which
    /// is rendered on the GPU without a CPU copy. Skipped when nothing changed, as in [`FrameLoop::draw_frame`].
    pub fn draw_frame_canvas<S: FrameSink>(&mut self, sink: &mut S, draw: impl FnOnce(&mut DisplayList) -> Damage) -> Result<FrameReport, FrameError> {
        let (w, h) = (self.width, self.height);
        let mut list = DisplayList::new(w, h);
        let damage = draw(&mut list);
        if frame_skipped(damage.is_empty(), self.created, self.bounds_dirty) {
            return Ok(FrameReport { seq: 0, skipped: true, stats: FrameStats::default() });
        }
        self.present(sink, &list)
    }

    /// Renders `list` into the next backing store from `sink` and commits it as one transaction on the root layer.
    /// The whole list is rendered: the stores are pooled, so a damage rectangle would show two-frames-old pixels.
    fn present<S: FrameSink>(&mut self, sink: &mut S, list: &DisplayList) -> Result<FrameReport, FrameError> {
        let (w, h) = (self.width, self.height);
        let (contents, target) = sink.acquire(w, h)?;
        let mut cb = self.queue.command_buffer()?;
        let stats = self.renderer.draw(&mut cb, list, &target, None)?;
        let work = cb.commit()?;

        let seq = self.seq + 1;
        let size = self.bounds_dirty.then_some((w, h));
        let tx = frame_transaction(seq, self.root, !self.created, size, contents);
        sink.commit(contents, &target, &work, &tx)?;

        self.seq = seq;
        self.created = true;
        self.bounds_dirty = false;
        self.pacer.committed();
        Ok(FrameReport { seq, skipped: false, stats })
    }
}

/// Whether a frame can be skipped: the app reported no damage, the root layer exists and its size is current.
pub fn frame_skipped(damage_empty: bool, created: bool, bounds_dirty: bool) -> bool {
    damage_empty && created && !bounds_dirty
}

fn pixel_count(width: u32, height: u32) -> usize {
    width as usize * height as usize
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presented_host_time_joins_the_halves_and_nanoseconds() {
        assert_eq!(presented_host_time(0, 0, 0), 0);
        assert_eq!(presented_host_time(0, 2, 500_000_000), 2_500_000_000);
        // 2^32 seconds in the high half: 4_294_967_296 s, plus 1 s.
        assert_eq!(presented_host_time(1, 1, 0), 4_294_967_297_000_000_000);
        assert_eq!(presented_host_time(0, 7, 999_999_999), 7_999_999_999);
    }

    #[test]
    fn the_first_frame_creates_the_root_sizes_it_and_binds_the_store() {
        let tx = frame_transaction(1, LayerId(1), true, Some((800, 600)), ContentsId(4));
        assert_eq!(tx.seq, 1);
        assert_eq!(tx.changes.len(), 4);
        assert_eq!(tx.changes[0], Change::Create(LayerId(1)));
        assert_eq!(tx.changes[1], Change::SetRoot(LayerId(1)));
        assert_eq!(
            tx.changes[2],
            Change::Set {
                layer: LayerId(1),
                key: Property::Bounds,
                value: Value::Rect(LayerRect { x: 0.0, y: 0.0, w: 800.0, h: 600.0 }),
            }
        );
        assert_eq!(tx.changes[3], Change::SetContents(LayerId(1), LayerContents::Buffer(ContentsId(4))));
    }

    #[test]
    fn later_frames_only_rebind_the_contents() {
        let tx = frame_transaction(9, LayerId(1), false, None, ContentsId(2));
        assert_eq!(tx.seq, 9);
        assert_eq!(tx.changes, vec![Change::SetContents(LayerId(1), LayerContents::Buffer(ContentsId(2)))]);
        assert_eq!(tx.implicit, None);
    }

    #[test]
    fn a_resize_sets_the_bounds_without_recreating_the_root() {
        let tx = frame_transaction(3, LayerId(1), false, Some((1280, 720)), ContentsId(1));
        assert_eq!(tx.changes.len(), 2);
        assert!(matches!(tx.changes[0], Change::Set { key: Property::Bounds, .. }));
        assert!(matches!(tx.changes[1], Change::SetContents(..)));
    }

    #[test]
    fn a_pacer_reports_each_refresh_once() {
        let mut p = Pacer::new(100).unwrap();
        assert!(p.next_frame(0).is_none(), "a stopped pacer reports nothing");
        p.start(0);
        let ts = p.next_frame(0).unwrap();
        assert_eq!(ts.frame_number, 0);
        assert_eq!(p.next_frame(5_000_000), None);
        let ts = p.next_frame(10_000_000).unwrap();
        assert_eq!(ts.frame_number, 1);
        assert_eq!(p.in_flight(), 0);
    }

    #[test]
    fn two_frames_in_flight_skip_the_next_refresh() {
        let mut p = Pacer::new(100).unwrap();
        p.start(0);
        assert!(p.next_frame(0).is_some());
        p.committed();
        assert!(p.next_frame(10_000_000).is_some());
        p.committed();
        // Two frames are not shown yet: the refresh at 20 ms is consumed but not drawn.
        assert_eq!(p.next_frame(20_000_000), None);
        assert_eq!(p.in_flight(), 2);
        // The compositor shows the first frame at 20 ms: one slot is free again.
        p.presented(20_000_000, 10_000_000);
        assert_eq!(p.in_flight(), 1);
        assert!(p.next_frame(30_000_000).is_some());
    }

    #[test]
    fn presented_realigns_the_clock_to_the_shown_frame() {
        let mut p = Pacer::new(100).unwrap();
        p.start(0);
        p.next_frame(0).unwrap();
        p.committed();
        // Shown at 25 ms: the next refresh begins 10 ms later, at 35 ms, on the moved grid.
        p.presented(25_000_000, 0);
        assert_eq!(p.in_flight(), 0);
        assert!(p.next_frame(34_000_000).is_some());
        let next = p.next_frame(35_000_000).unwrap();
        assert_eq!(next.host_time, 35_000_000);
    }

    #[test]
    fn presented_never_takes_the_count_below_zero() {
        let mut p = Pacer::new(60).unwrap();
        p.presented(0, 0);
        assert_eq!(p.in_flight(), 0);
    }

    #[test]
    fn a_zero_refresh_has_no_pacer() {
        assert!(Pacer::new(0).is_none());
    }

    #[test]
    fn a_frame_without_damage_is_skipped_once_the_root_exists() {
        assert!(frame_skipped(true, true, false));
        // the first frame always goes out, even with no damage: it creates the root layer
        assert!(!frame_skipped(true, false, false));
        // a size change always goes out, even with no damage: the bounds must reach the server
        assert!(!frame_skipped(true, true, true));
        // any damage draws
        assert!(!frame_skipped(false, true, false));
        assert!(!frame_skipped(false, false, true));
    }

    #[test]
    fn pixel_count_is_width_times_height() {
        assert_eq!(pixel_count(2560, 1440), 3_686_400);
        assert_eq!(pixel_count(0, 10), 0);
    }
}
