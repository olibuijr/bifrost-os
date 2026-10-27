//! `bifrost_window_capture_v1` (protocol/bifrost-window-capture-v1.xml, docs/window-server.md "Window capture"): the
//! shell asks for one window's own pixels, scaled down, for Overview tiles, the Dock's thumbnails of minimized
//! windows and window screenshots.
//!
//! - **Protocol.** A private protocol rather than ext-image-copy-capture with the foreign-toplevel source: Smithay 0.7
//!   has no image-capture-source or image-copy-capture support for toplevels, the standard session takes the buffer
//!   size from the compositor (a full-size window per Overview tile) with no size hint from the client, and a
//!   standard handle (ext-foreign-toplevel-list) cannot be matched with the wlr handles the shell already holds. Here the
//!   request names the window by surface (the shell's own toplevels) or by wlr foreign-toplevel handle (any client's),
//!   and carries the buffer, whose size is the largest picture wanted.
//! - **Who may ask.** The shell only. The global is a capture global: it is visible only to privileged clients, the ones
//!   that connected through the privileged socket (`privileged.rs`), and a request from any other client is answered
//!   `failed(denied)` anyway. The shell runs as root and the socket is root's, so an app cannot reach it.
//! - **On demand, scaled in the renderer.** A request renders the window alone into a texture of the fitted size
//!   (`capture::capture_window` on GLES, `render_server::window_shot` on the Vulkan scanout, which answers
//!   `failed(unsupported)` for a window it cannot sample), never per frame.
//! - **No work when nothing changed.** Every commit of a window's surface tree advances its content serial; a request
//!   that names the serial the shell already holds is answered `unchanged`.
//! - **Minimized windows.** A minimized toplevel stays in `WmState::hidden` (it leaves the space but not the window
//!   server), so its last committed content can still be rendered, and a snapshot taken when it was hidden answers when
//!   the client dropped its buffer. Both are freed when the window closes.

#[allow(
    dead_code,
    non_camel_case_types,
    unused_unsafe,
    unused_variables,
    non_upper_case_globals,
    non_snake_case,
    unused_imports,
    missing_docs,
    clippy::all
)]
pub mod protocol {
    use smithay::reexports::wayland_protocols_wlr::foreign_toplevel::v1::server::*;
    use wayland_server;
    use wayland_server::protocol::*;

    pub mod __interfaces {
        use smithay::reexports::wayland_protocols_wlr::foreign_toplevel::v1::server::__interfaces::*;
        use wayland_server::protocol::__interfaces::*;
        wayland_scanner::generate_interfaces!("protocol/bifrost-window-capture-v1.xml");
    }
    use self::__interfaces::*;

    wayland_scanner::generate_server_code!("protocol/bifrost-window-capture-v1.xml");
}

use std::collections::HashMap;

use smithay::{
    desktop::Window,
    reexports::{
        wayland_protocols_wlr::foreign_toplevel::v1::server::zwlr_foreign_toplevel_handle_v1::ZwlrForeignToplevelHandleV1,
        wayland_server::{
            backend::GlobalId,
            protocol::{wl_buffer::WlBuffer, wl_shm, wl_surface::WlSurface},
            Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource,
        },
    },
    wayland::{
        seat::WaylandFocus,
        shm::{with_buffer_contents, BufferData},
    },
};

use self::protocol::{
    bifrost_window_capture_frame_v1::{self, BifrostWindowCaptureFrameV1},
    bifrost_window_capture_manager_v1::{self, BifrostWindowCaptureManagerV1},
};
use crate::{
    privileged::{is_privileged, PrivilegedOnly},
    session_protocols::write_shm_sized,
    state::WmState,
    wm::WindowId,
};

/// The largest side of the snapshot taken when a window is minimized.
pub const SNAPSHOT_BOUND: (i32, i32) = (640, 400);

// ---------------------------------------------------------------------------------------------------------------------
// Pure logic (host-tested)
// ---------------------------------------------------------------------------------------------------------------------

/// The picture size for a window of `src` (width, height) fitted into `bound`: the aspect kept, never scaled up,
/// each side at least 1. `None` when either size is empty.
pub fn fit_within(src: (i32, i32), bound: (i32, i32)) -> Option<(i32, i32)> {
    if src.0 <= 0 || src.1 <= 0 || bound.0 <= 0 || bound.1 <= 0 {
        return None;
    }
    let k = (bound.0 as f64 / src.0 as f64).min(bound.1 as f64 / src.1 as f64).min(1.0);
    let w = ((src.0 as f64 * k).round() as i32).clamp(1, bound.0);
    let h = ((src.1 as f64 * k).round() as i32).clamp(1, bound.1);
    Some((w, h))
}

/// The serial after `serial`: never 0, which means "no picture held" on the wire.
pub fn next_serial(serial: u32) -> u32 {
    match serial.wrapping_add(1) {
        0 => 1,
        n => n,
    }
}

/// What a capture request does after the serial check: the shell holds the window's current picture (`since` equals
/// the serial and is not 0), so nothing is drawn.
pub fn is_unchanged(since: u32, serial: u32) -> bool {
    since != 0 && since == serial
}

/// RGBA rows of `src` (`sw` by `sh`) reduced to `dw` by `dh` by averaging the source pixels each destination pixel
/// covers (a box filter, exact for the premultiplied pixels the window server renders). Used to serve a snapshot at a
/// smaller size than it was taken at. A destination larger than the source repeats pixels.
pub fn downscale_rgba(src: &[u8], sw: i32, sh: i32, dw: i32, dh: i32) -> Vec<u8> {
    let (sw, sh, dw, dh) = (sw.max(1) as usize, sh.max(1) as usize, dw.max(1) as usize, dh.max(1) as usize);
    let mut out = vec![0u8; dw * dh * 4];
    if src.len() < sw * sh * 4 {
        return out;
    }
    for dy in 0..dh {
        let (y0, y1) = (dy * sh / dh, ((dy + 1) * sh / dh).max(dy * sh / dh + 1).min(sh));
        for dx in 0..dw {
            let (x0, x1) = (dx * sw / dw, ((dx + 1) * sw / dw).max(dx * sw / dw + 1).min(sw));
            let mut sum = [0u32; 4];
            for y in y0..y1 {
                for x in x0..x1 {
                    let i = (y * sw + x) * 4;
                    for c in 0..4 {
                        sum[c] += src[i + c] as u32;
                    }
                }
            }
            let n = ((y1 - y0) * (x1 - x0)) as u32;
            let o = (dy * dw + dx) * 4;
            for c in 0..4 {
                out[o + c] = ((sum[c] + n / 2) / n) as u8;
            }
        }
    }
    out
}

/// Why a capture produced no picture; the value is the wire `reason`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Failure {
    Unsupported = 0,
    Gone = 1,
    BadBuffer = 2,
    Denied = 3,
    Empty = 4,
}

/// A window picture kept from when the window was minimized.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snapshot {
    pub w: i32,
    pub h: i32,
    /// RGBA rows, premultiplied.
    pub rgba: Vec<u8>,
}

/// The window server's capture state: each window's content serial, the snapshots of minimized windows, and who the
/// shell is.
#[derive(Default)]
pub struct WindowCapture {
    serials: HashMap<WindowId, u32>,
    snapshots: HashMap<WindowId, Snapshot>,
}

impl WindowCapture {
    /// The content serial of window `id`: 1 until it commits again.
    pub fn serial(&self, id: WindowId) -> u32 {
        self.serials.get(&id).copied().unwrap_or(1)
    }

    /// The window committed content: its serial advances.
    pub fn touch(&mut self, id: WindowId) {
        let next = next_serial(self.serial(id));
        self.serials.insert(id, next);
    }

    pub fn store_snapshot(&mut self, id: WindowId, snapshot: Snapshot) {
        self.snapshots.insert(id, snapshot);
    }

    pub fn snapshot(&self, id: WindowId) -> Option<&Snapshot> {
        self.snapshots.get(&id)
    }

    /// Drops the snapshot of a window that is shown again (its live content is the truth now).
    pub fn drop_snapshot(&mut self, id: WindowId) {
        self.snapshots.remove(&id);
    }

    /// The window closed: its serial and snapshot are freed.
    pub fn forget(&mut self, id: WindowId) {
        self.serials.remove(&id);
        self.snapshots.remove(&id);
    }

    /// How many windows hold state (tests).
    pub fn tracked(&self) -> usize {
        self.serials.len().max(self.snapshots.len())
    }
}

/// What a request named.
pub enum Target {
    Surface(WlSurface),
    Toplevel(ZwlrForeignToplevelHandleV1),
}

/// A served capture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Served {
    Ready { width: i32, height: i32, serial: u32 },
    Unchanged { serial: u32 },
}

/// The buffer a request filled: its size and stride, and whether its format is one a picture can be written in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BufferShape {
    pub width: i32,
    pub height: i32,
    pub stride: i32,
    pub format_ok: bool,
}

impl BufferShape {
    /// Whether a picture can be written into the buffer at all: a 32-bit format, positive size, rows long enough.
    pub fn usable(&self) -> bool {
        self.format_ok && self.width > 0 && self.height > 0 && self.stride >= self.width.saturating_mul(4)
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// The global
// ---------------------------------------------------------------------------------------------------------------------

pub fn init(dh: &DisplayHandle) -> GlobalId {
    dh.create_global::<WmState, BifrostWindowCaptureManagerV1, PrivilegedOnly>(1, PrivilegedOnly)
}

impl GlobalDispatch<BifrostWindowCaptureManagerV1, PrivilegedOnly> for WmState {
    fn bind(_: &mut Self, _: &DisplayHandle, _: &Client, resource: New<BifrostWindowCaptureManagerV1>, _: &PrivilegedOnly, init: &mut DataInit<'_, Self>) {
        init.init(resource, ());
    }

    /// Only privileged clients see the global; every other client's registry does not list it, and a bind is refused.
    fn can_view(client: Client, _: &PrivilegedOnly) -> bool {
        is_privileged(&client)
    }
}

impl Dispatch<BifrostWindowCaptureManagerV1, ()> for WmState {
    fn request(
        state: &mut Self,
        client: &Client,
        _: &BifrostWindowCaptureManagerV1,
        request: bifrost_window_capture_manager_v1::Request,
        _: &(),
        _: &DisplayHandle,
        init: &mut DataInit<'_, Self>,
    ) {
        use bifrost_window_capture_manager_v1::Request as R;
        let (frame, target, buffer, since) = match request {
            R::CaptureSurface { frame, surface, buffer, since } => (frame, Target::Surface(surface), buffer, since),
            R::CaptureToplevel { frame, toplevel, buffer, since } => (frame, Target::Toplevel(toplevel), buffer, since),
            _ => return,
        };
        let frame = init.init(frame, ());
        // the global is hidden from other clients; a client that got hold of it anyway is refused here
        if !is_privileged(client) {
            frame.failed(Failure::Denied as u32);
            return;
        }
        match state.serve_window_capture(target, &buffer, since) {
            Ok(Served::Ready { width, height, serial }) => frame.ready(width as u32, height as u32, serial),
            Ok(Served::Unchanged { serial }) => frame.unchanged(serial),
            Err(reason) => frame.failed(reason as u32),
        }
    }
}

impl Dispatch<BifrostWindowCaptureFrameV1, ()> for WmState {
    fn request(_: &mut Self, _: &Client, _: &BifrostWindowCaptureFrameV1, _: bifrost_window_capture_frame_v1::Request, _: &(), _: &DisplayHandle, _: &mut DataInit<'_, Self>) {}
}

/// What the buffer a request carries looks like; `None` when it is not a wl_shm buffer.
fn buffer_shape(buffer: &WlBuffer) -> Option<BufferShape> {
    with_buffer_contents(buffer, |_, _, d: BufferData| BufferShape {
        width: d.width,
        height: d.height,
        stride: d.stride,
        format_ok: matches!(d.format, wl_shm::Format::Argb8888 | wl_shm::Format::Xrgb8888),
    })
    .ok()
}

/// Whether the window's root surface has a committed buffer to draw.
fn has_content(window: &Window) -> bool {
    window.wl_surface().is_some_and(|surface| {
        smithay::backend::renderer::utils::with_renderer_surface_state(&*surface, |s| s.buffer().is_some()).unwrap_or(false)
    })
}

/// Parameters of one window render the backend performs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowCaptureSpec {
    pub id: WindowId,
    /// The picture size (width, height), already fitted to the window's aspect.
    pub size: (i32, i32),
}

impl WmState {
    /// The window `id` whether it is shown or minimized.
    pub fn window_any(&self, id: WindowId) -> Option<Window> {
        self.window(id).or_else(|| self.hidden.iter().find(|w| WmState::known_id(w) == Some(id)).cloned())
    }

    /// The minimized window whose root surface is `surface`.
    pub fn hidden_for_surface(&self, surface: &WlSurface) -> Option<Window> {
        self.hidden.iter().find(|w| w.wl_surface().is_some_and(|s| s.as_ref() == surface)).cloned()
    }

    /// Minimizes `window`: it leaves the space but the window server keeps it, with a snapshot of what it showed, so it
    /// can still be captured and shown again.
    pub fn hide_window(&mut self, id: WindowId, window: &Window) {
        self.snapshot_window(id, window);
        self.keep_hidden(window);
        self.space.unmap_elem(window);
    }

    /// Keeps `window` in `hidden` (once) and out of the space.
    pub fn keep_hidden(&mut self, window: &Window) {
        if !self.hidden.contains(window) {
            self.hidden.push(window.clone());
        }
        self.space.unmap_elem(window);
    }

    /// `window` is shown again: it is no longer held hidden and its snapshot is stale.
    pub fn unhide(&mut self, id: WindowId, window: &Window) {
        if let Some(i) = self.hidden.iter().position(|w| w == window) {
            self.hidden.remove(i);
            self.window_capture.drop_snapshot(id);
        }
    }

    /// The window closed: everything held for it is freed.
    pub fn forget_window(&mut self, id: WindowId, window: &Window) {
        self.hidden.retain(|w| w != window);
        self.window_capture.forget(id);
    }

    /// Renders `window` into the snapshot of a window being minimized (best effort: without a renderer hook the window
    /// is still kept, and is rendered live when asked for).
    fn snapshot_window(&mut self, id: WindowId, window: &Window) {
        let geo = window.geometry().size;
        let Some(size) = fit_within((geo.w, geo.h), SNAPSHOT_BOUND) else { return };
        if !has_content(window) {
            return;
        }
        if let Some(rgba) = self.render_window_pixels(id, size).filter(|px| px.len() == (size.0 * size.1 * 4) as usize) {
            self.window_capture.store_snapshot(id, Snapshot { w: size.0, h: size.1, rgba });
        }
    }

    /// Calls the backend to render window `id` at `size`; `None` without a hook or when the renderer cannot.
    fn render_window_pixels(&mut self, id: WindowId, size: (i32, i32)) -> Option<Vec<u8>> {
        // the hook renders from `self`, so it is taken out of the session for the call
        let mut hook = self.session.hooks.capture_window.take()?;
        let px = hook(self, &WindowCaptureSpec { id, size });
        self.session.hooks.capture_window = Some(hook);
        px
    }

    /// The picture of window `id`, at most `bound`: rendered live when the window has content, else served from the
    /// snapshot taken when it was minimized.
    fn window_picture(&mut self, id: WindowId, bound: (i32, i32)) -> Result<(i32, i32, Vec<u8>), Failure> {
        let window = self.window_any(id).ok_or(Failure::Gone)?;
        let live = has_content(&window);
        if live {
            let geo = window.geometry().size;
            let size = fit_within((geo.w, geo.h), bound).ok_or(Failure::Empty)?;
            if let Some(px) = self.render_window_pixels(id, size).filter(|px| px.len() == (size.0 * size.1 * 4) as usize) {
                return Ok((size.0, size.1, px));
            }
        }
        match self.window_capture.snapshot(id) {
            Some(snap) => {
                let (w, h) = fit_within((snap.w, snap.h), bound).ok_or(Failure::Empty)?;
                Ok((w, h, downscale_rgba(&snap.rgba, snap.w, snap.h, w, h)))
            }
            None if live => Err(Failure::Unsupported),
            None => Err(Failure::Gone),
        }
    }

    /// Answers one capture request: names the window, checks the buffer and the serial, renders and writes.
    fn serve_window_capture(&mut self, target: Target, buffer: &WlBuffer, since: u32) -> Result<Served, Failure> {
        let id = match target {
            Target::Surface(surface) => self
                .window_for_surface(&surface)
                .or_else(|| self.hidden_for_surface(&surface))
                .as_ref()
                .and_then(WmState::known_id),
            Target::Toplevel(handle) => handle.data::<WindowId>().copied(),
        }
        .ok_or(Failure::Gone)?;
        let shape = buffer_shape(buffer).filter(BufferShape::usable).ok_or(Failure::BadBuffer)?;
        let serial = self.window_capture.serial(id);
        if is_unchanged(since, serial) {
            return Ok(Served::Unchanged { serial });
        }
        let (width, height, px) = self.window_picture(id, (shape.width, shape.height))?;
        write_shm_sized(buffer, &px, width, height).map_err(|_| Failure::BadBuffer)?;
        Ok(Served::Ready { width, height, serial })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_window_is_fitted_into_the_buffer_with_its_aspect_and_never_enlarged() {
        assert_eq!(fit_within((1600, 900), (400, 400)), Some((400, 225)));
        assert_eq!(fit_within((1000, 300), (200, 200)), Some((200, 60)));
        assert_eq!(fit_within((300, 1000), (200, 200)), Some((60, 200)));
        assert_eq!(fit_within((100, 50), (400, 400)), Some((100, 50)), "a small window is not scaled up");
        assert_eq!(fit_within((800, 600), (800, 600)), Some((800, 600)), "an exact fit is the window itself");
    }

    #[test]
    fn an_extreme_aspect_keeps_at_least_one_pixel_on_each_side() {
        assert_eq!(fit_within((4000, 10), (100, 100)), Some((100, 1)));
        assert_eq!(fit_within((10, 4000), (100, 100)), Some((1, 100)));
    }

    #[test]
    fn an_empty_window_or_buffer_has_no_fit() {
        assert_eq!(fit_within((0, 100), (100, 100)), None);
        assert_eq!(fit_within((100, 100), (0, 100)), None);
        assert_eq!(fit_within((-1, 5), (5, 5)), None);
    }

    #[test]
    fn the_fit_never_exceeds_the_bound() {
        for (src, bound) in [((1921, 1081), (333, 211)), ((7, 3), (5, 5)), ((2560, 1440), (480, 300)), ((17, 1000), (64, 64))] {
            let (w, h) = fit_within(src, bound).unwrap();
            assert!(w <= bound.0 && h <= bound.1 && w >= 1 && h >= 1, "{src:?} in {bound:?} gave {w}x{h}");
        }
    }

    #[test]
    fn serials_advance_and_skip_zero() {
        assert_eq!(next_serial(1), 2);
        assert_eq!(next_serial(u32::MAX), 1, "0 means no picture held, so it is never a serial");
        assert_eq!(next_serial(0), 1);
    }

    #[test]
    fn a_picture_the_shell_already_holds_is_not_drawn_again() {
        assert!(is_unchanged(5, 5));
        assert!(!is_unchanged(4, 5), "the window committed after that picture");
        assert!(!is_unchanged(0, 0), "0 asks for a picture whatever the window did");
        assert!(!is_unchanged(0, 1));
    }

    #[test]
    fn a_commit_changes_the_serial_of_that_window_only() {
        let mut c = WindowCapture::default();
        assert_eq!(c.serial(7), 1, "a window never touched starts at 1");
        c.touch(7);
        c.touch(7);
        c.touch(8);
        assert_eq!((c.serial(7), c.serial(8), c.serial(9)), (3, 2, 1));
    }

    #[test]
    fn a_closed_window_frees_its_serial_and_snapshot() {
        let mut c = WindowCapture::default();
        c.touch(3);
        c.store_snapshot(3, Snapshot { w: 1, h: 1, rgba: vec![1, 2, 3, 4] });
        assert_eq!(c.tracked(), 1);
        assert!(c.snapshot(3).is_some());
        c.forget(3);
        assert_eq!(c.tracked(), 0);
        assert!(c.snapshot(3).is_none());
        assert_eq!(c.serial(3), 1, "a new window under the same id starts over");
    }

    #[test]
    fn a_shown_window_drops_its_stale_snapshot_only() {
        let mut c = WindowCapture::default();
        c.touch(3);
        c.store_snapshot(3, Snapshot { w: 1, h: 1, rgba: vec![0; 4] });
        c.drop_snapshot(3);
        assert!(c.snapshot(3).is_none());
        assert_eq!(c.serial(3), 2, "the serial is the window's, not the snapshot's");
    }

    #[test]
    fn a_box_filter_averages_what_each_pixel_covers() {
        // 2x2 -> 1x1: the mean of four pixels
        let src = [0, 0, 0, 255, 100, 40, 8, 255, 200, 80, 16, 255, 100, 40, 8, 255];
        assert_eq!(downscale_rgba(&src, 2, 2, 1, 1), vec![100, 40, 8, 255]);
        // the same size is a copy
        assert_eq!(downscale_rgba(&src, 2, 2, 2, 2), src.to_vec());
    }

    #[test]
    fn a_box_filter_keeps_the_halves_apart() {
        // 4x1: left two red, right two blue -> 2x1 keeps one of each
        let red = [255, 0, 0, 255];
        let blue = [0, 0, 255, 255];
        let src: Vec<u8> = [red, red, blue, blue].concat();
        assert_eq!(downscale_rgba(&src, 4, 1, 2, 1), [red, blue].concat());
    }

    #[test]
    fn a_short_source_gives_an_empty_picture_not_a_panic() {
        assert_eq!(downscale_rgba(&[1, 2, 3], 2, 2, 1, 1), vec![0, 0, 0, 0]);
    }

    #[test]
    fn a_buffer_must_be_a_32_bit_format_with_rows_long_enough() {
        let ok = BufferShape { width: 100, height: 50, stride: 400, format_ok: true };
        assert!(ok.usable());
        assert!(BufferShape { stride: 512, ..ok }.usable(), "padded rows are fine: the picture is written row by row");
        assert!(!BufferShape { stride: 396, ..ok }.usable(), "rows too short");
        assert!(!BufferShape { format_ok: false, ..ok }.usable());
        assert!(!BufferShape { width: 0, ..ok }.usable());
        assert!(!BufferShape { height: -1, ..ok }.usable());
    }

    #[test]
    fn the_failure_values_are_the_protocol_reasons() {
        let xml = include_str!("../protocol/bifrost-window-capture-v1.xml");
        for (name, f) in [("unsupported", Failure::Unsupported), ("gone", Failure::Gone), ("bad_buffer", Failure::BadBuffer), ("denied", Failure::Denied), ("empty", Failure::Empty)] {
            assert!(xml.contains(&format!("<entry name=\"{name}\" value=\"{}\"", f as u32)), "{name}");
        }
    }

    #[test]
    fn the_protocol_file_declares_what_the_server_implements() {
        let xml = include_str!("../protocol/bifrost-window-capture-v1.xml");
        assert!(xml.contains("<interface name=\"bifrost_window_capture_manager_v1\" version=\"1\">"));
        assert!(xml.contains("<request name=\"capture_surface\">"));
        assert!(xml.contains("<request name=\"capture_toplevel\">"));
        for event in ["ready", "unchanged", "failed"] {
            assert!(xml.contains(&format!("<event name=\"{event}\">")), "{event}");
        }
    }
}
