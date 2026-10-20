//! Session protocols of the window server (`docs/graphics-architecture.md`, "Session parity with labwc"): the
//! foreign-toplevel list (Dock, window switcher, MCP), screencopy (`zwlr_screencopy_manager_v1` for grim and
//! `bifrost-apps --screenshot`, `ext_image_copy_capture_manager_v1`), cursor shapes, xdg-activation, output management
//! (`video-arg`) and output power.
//!
//! The pure parts (the toplevel diff, the mode choice, the capture region and buffer checks, the activation policy and
//! the timestamp split) take no Wayland types and are tested on the host. The protocol parts are Smithay and
//! wayland-server code. Work that needs the renderer or the display backend goes through hooks in [`SessionHooks`]
//! that the backend sets (`capture`, `set_mode`, `set_power`); without a hook the request is refused, never faked.
//!
//! A captured frame is written into the client's wl_shm buffer by `write_shm` (one audited write; the pixel format
//! conversion is [`rgba_to_shm`]). The DRM and headless backends set the `capture` hook (`capture.rs`); a copy is answered with
//! `failed` until the render server's readback is wired to it. See `docs/graphics/g2-wm-session.md`.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Duration,
};

use smithay::{
    delegate_cursor_shape,
    input::pointer::CursorImageStatus,
    output::{Mode, Output},
    reexports::{
        wayland_protocols::ext::{
            image_capture_source::v1::server::{
                ext_image_capture_source_v1::{self, ExtImageCaptureSourceV1},
                ext_output_image_capture_source_manager_v1::{self, ExtOutputImageCaptureSourceManagerV1},
            },
            image_copy_capture::v1::server::{
                ext_image_copy_capture_frame_v1::{self, ExtImageCopyCaptureFrameV1},
                ext_image_copy_capture_cursor_session_v1::{self, ExtImageCopyCaptureCursorSessionV1},
                ext_image_copy_capture_manager_v1::{self, ExtImageCopyCaptureManagerV1},
                ext_image_copy_capture_session_v1::{self, ExtImageCopyCaptureSessionV1},
            },
        },
        wayland_protocols_wlr::{
            foreign_toplevel::v1::server::{
                zwlr_foreign_toplevel_handle_v1::{self, ZwlrForeignToplevelHandleV1},
                zwlr_foreign_toplevel_manager_v1::{self, ZwlrForeignToplevelManagerV1},
            },
            output_management::v1::server::{
                zwlr_output_configuration_head_v1::{self, ZwlrOutputConfigurationHeadV1},
                zwlr_output_configuration_v1::{self, ZwlrOutputConfigurationV1},
                zwlr_output_head_v1::{self, ZwlrOutputHeadV1},
                zwlr_output_manager_v1::{self, ZwlrOutputManagerV1},
                zwlr_output_mode_v1::{self, ZwlrOutputModeV1},
            },
            output_power_management::v1::server::{
                zwlr_output_power_manager_v1::{self, ZwlrOutputPowerManagerV1},
                zwlr_output_power_v1::{self, ZwlrOutputPowerV1},
            },
            screencopy::v1::server::{
                zwlr_screencopy_frame_v1::{self, ZwlrScreencopyFrameV1},
                zwlr_screencopy_manager_v1::{self, ZwlrScreencopyManagerV1},
            },
        },
        wayland_server::{
            backend::{ClientId, ObjectId},
            protocol::{
                wl_buffer::WlBuffer,
                wl_output,
                wl_shm,
                wl_surface::WlSurface,
            },
            Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource, WEnum,
        },
    },
    utils::{Transform, SERIAL_COUNTER},
    wayland::{
        compositor::with_states,
        cursor_shape::CursorShapeManagerState,
        shell::xdg::XdgToplevelSurfaceData,
        shm::{with_buffer_contents, with_buffer_contents_mut, BufferData},
        xdg_activation::{XdgActivationHandler, XdgActivationState, XdgActivationToken, XdgActivationTokenData},
    },
};

use crate::{
    state::WmState,
    wm::{Rect, WindowId},
};

/// How long after its serial an activation token may still raise a window.
pub const ACTIVATION_WINDOW: Duration = Duration::from_secs(10);

/// Refresh rates closer than this (1 Hz, in millihertz) count as the same rate.
pub const REFRESH_TOLERANCE: i32 = 1000;

// ---------------------------------------------------------------------------------------------------------------------
// Pure logic (host-tested)
// ---------------------------------------------------------------------------------------------------------------------

/// What the foreign-toplevel list reports about one window.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ToplevelInfo {
    pub id: WindowId,
    pub title: String,
    pub app_id: String,
    pub activated: bool,
    pub maximized: bool,
    pub minimized: bool,
    pub fullscreen: bool,
}

impl ToplevelInfo {
    /// The `zwlr_foreign_toplevel_handle_v1.state` values, in the protocol's enum order (maximized 0, minimized 1,
    /// activated 2, fullscreen 3), each as a native-endian u32 in the array.
    pub fn state_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        for (on, value) in [(self.maximized, 0u32), (self.minimized, 1), (self.activated, 2), (self.fullscreen, 3)] {
            if on {
                out.extend_from_slice(&value.to_ne_bytes());
            }
        }
        out
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToplevelChange {
    Added(ToplevelInfo),
    /// The new state of a window that already existed and changed in some field.
    Changed(ToplevelInfo),
    Removed(WindowId),
}

/// Diffs two toplevel snapshots: additions and changes in the new order, then removals in the old order.
pub fn diff_toplevels(old: &[ToplevelInfo], new: &[ToplevelInfo]) -> Vec<ToplevelChange> {
    let mut out = Vec::new();
    for n in new {
        match old.iter().find(|o| o.id == n.id) {
            None => out.push(ToplevelChange::Added(n.clone())),
            Some(o) if o != n => out.push(ToplevelChange::Changed(n.clone())),
            Some(_) => {}
        }
    }
    for o in old {
        if !new.iter().any(|n| n.id == o.id) {
            out.push(ToplevelChange::Removed(o.id));
        }
    }
    out
}

/// One output mode, as the output-management protocol describes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModeInfo {
    pub w: i32,
    pub h: i32,
    /// Millihertz, as in `wl_output` and the output-management protocol.
    pub refresh: i32,
    pub preferred: bool,
}

/// A requested mode: width, height and an optional refresh (millihertz).
pub type Want = (i32, i32, Option<i32>);

/// Picks the mode a configuration asks for.
///
/// - `None`: the preferred mode, else the first listed one.
/// - `(w, h, None)`: among the modes of that size, the highest refresh rate.
/// - `(w, h, Some(r))`: among the modes of that size, the refresh rate nearest `r`, if it is within
///   [`REFRESH_TOLERANCE`].
pub fn choose_mode(modes: &[ModeInfo], want: Option<Want>) -> Option<ModeInfo> {
    let Some((w, h, refresh)) = want else {
        return modes.iter().find(|m| m.preferred).or_else(|| modes.first()).copied();
    };
    let same_size = modes.iter().filter(|m| m.w == w && m.h == h);
    match refresh {
        None => same_size.max_by_key(|m| m.refresh).copied(),
        Some(r) => same_size
            .min_by_key(|m| (m.refresh - r).abs())
            .copied()
            .filter(|m| (m.refresh - r).abs() <= REFRESH_TOLERANCE),
    }
}

/// The part of an output region a capture may read: the request clipped to the output, or `None` when nothing is left.
pub fn clamp_region(output: (i32, i32), req: Rect) -> Option<Rect> {
    let x0 = req.x.max(0);
    let y0 = req.y.max(0);
    let x1 = (req.x + req.w).min(output.0);
    let y1 = (req.y + req.h).min(output.1);
    (x1 > x0 && y1 > y0).then(|| Rect::new(x0, y0, x1 - x0, y1 - y0))
}

/// The size of an output as clients see it, in pixels: the mode turned by the output transform (a panel rotated by 90
/// degrees shows a portrait image). Captures, their regions and their buffers are in this orientation.
pub fn capture_size(mode: (i32, i32), transform: Transform) -> (i32, i32) {
    crate::transform::logical_size(mode, transform)
}

/// `capture_size` of an output; `None` before it has a mode.
fn output_capture_size(out: &Output) -> Option<(i32, i32)> {
    crate::transform::output_size(out)
}

/// Bytes per row of a 32-bit shm buffer (XRGB8888 or ARGB8888).
pub fn shm_stride(width: i32) -> i32 { width * 4 }

/// Whether a client's shm buffer has the size and stride a capture of `want` (width, height) writes into.
pub fn buffer_fits(size: (i32, i32), stride: i32, want: (i32, i32)) -> bool { size == want && stride == shm_stride(want.0) }

/// An activation token raises its window when it carries a serial from a recent input event.
pub fn activation_allowed(has_serial: bool, age: Duration) -> bool { has_serial && age <= ACTIVATION_WINDOW }

/// Splits a timestamp into the `tv_sec_hi`, `tv_sec_lo`, `tv_nsec` triple the capture protocols send.
pub fn stamp(d: Duration) -> (u32, u32, u32) {
    let secs = d.as_secs();
    ((secs >> 32) as u32, secs as u32, d.subsec_nanos())
}

// ---------------------------------------------------------------------------------------------------------------------
// Protocol state
// ---------------------------------------------------------------------------------------------------------------------

/// The window server's hooks into the backend. Each is optional; a request that needs a missing hook is refused.
#[derive(Default)]
pub struct SessionHooks {
    /// Reads the pixels of a capture region as RGBA rows (R, G, B, A in memory) of `region.w * 4` bytes, wallpaper and
    /// every layer composed; `write_shm` converts them to the client's format. The Vulkan server reads them
    /// back from the output target; the headless backend from its texture.
    pub capture: Option<Box<dyn FnMut(&mut WmState, &CaptureSpec) -> Option<Vec<u8>>>>,
    /// Switches an output to a mode (a DRM modeset). Returns whether it took.
    pub set_mode: Option<Box<dyn FnMut(&str, ModeInfo) -> bool>>,
    /// Checks and prepares a change of an output's transform (`live_rotation::rotate_output`). The transform itself is
    /// set by the window server; the backend answers whether its scanout can follow it. Returns whether it can.
    pub set_transform: Option<Box<dyn FnMut(&str, Transform) -> bool>>,
    /// Turns an output on or off (DPMS). Returns whether it took.
    pub set_power: Option<Box<dyn FnMut(&str, bool) -> bool>>,
    /// Renders one window alone, scaled to `spec.size`, as RGBA rows (premultiplied, transparent ground): the picture
    /// behind `bifrost_window_capture_v1` (`window_capture.rs`). `None` when the renderer cannot (the Vulkan scanout).
    pub capture_window: Option<Box<dyn FnMut(&mut WmState, &crate::window_capture::WindowCaptureSpec) -> Option<Vec<u8>>>>,
}

/// One capture the backend is asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaptureSpec {
    pub output: String,
    /// Output-local pixels in the output's displayed orientation (`capture_size`); the buffer is `region.w` by `region.h`.
    pub region: Rect,
    /// Whether the pointer is painted into the capture (`overlay_cursor`).
    pub cursor: bool,
}

/// What a capture frame reads: an output and a region of it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CaptureTarget {
    pub output: String,
    pub region: Rect,
    pub cursor: bool,
}

/// A capture frame's target, and for ext-image-copy-capture the buffer attached before `capture`.
pub struct FrameData {
    target: CaptureTarget,
    buffer: Mutex<Option<WlBuffer>>,
}

impl FrameData {
    fn new(target: CaptureTarget) -> Self { FrameData { target, buffer: Mutex::new(None) } }
}

/// The handles one window has in each foreign-toplevel manager, with what was last sent.
struct ToplevelHandles {
    id: WindowId,
    handles: Vec<ZwlrForeignToplevelHandleV1>,
}

/// One output head announced to one output manager.
struct HeadRec {
    manager: ObjectId,
    output: String,
    head: ZwlrOutputHeadV1,
    modes: Vec<(ModeInfo, ZwlrOutputModeV1)>,
}

/// What a configuration asks for one head.
#[derive(Debug, Default)]
pub struct HeadWant {
    pub mode: Option<Want>,
    /// The transform the configuration asks for, if it set one.
    pub transform: Option<Transform>,
}

type HeadWantRef = Arc<Mutex<HeadWant>>;

/// The heads of one configuration request.
#[derive(Default)]
pub struct ConfigEntries(Mutex<Vec<ConfigEntry>>);

struct ConfigEntry {
    output: String,
    enabled: bool,
    want: HeadWantRef,
}

/// The output a power object controls; `None` when the output was gone when the object was made.
pub struct PowerData(Option<String>);

/// Session protocol state, owned by [`WmState`].
pub struct SessionState {
    pub hooks: SessionHooks,
    /// The cursor image the seat last asked for (a named shape, a surface, or hidden).
    pub cursor: Option<CursorImageStatus>,
    pub cursor_shape: CursorShapeManagerState,
    pub activation: XdgActivationState,

    foreign_managers: Vec<ZwlrForeignToplevelManagerV1>,
    /// The last snapshot the foreign-toplevel list announced.
    pub toplevels: Vec<ToplevelInfo>,
    /// Last known title and app id per window; a minimized window is not in the space but keeps its names.
    names: HashMap<WindowId, (String, String)>,
    handles: Vec<ToplevelHandles>,

    out_managers: Vec<ZwlrOutputManagerV1>,
    out_heads: Vec<HeadRec>,
    out_serial: u32,
    power_on: HashMap<String, bool>,
}

impl SessionState {
    /// Registers the globals: foreign toplevels, screencopy, output management and power, and the image copy
    /// capture sources. Cursor shapes and xdg-activation come from Smithay.
    pub fn new(dh: &DisplayHandle) -> Self {
        dh.create_global::<WmState, ZwlrForeignToplevelManagerV1, ()>(3, ());
        dh.create_global::<WmState, ZwlrScreencopyManagerV1, ()>(3, ());
        dh.create_global::<WmState, ZwlrOutputManagerV1, ()>(4, ());
        dh.create_global::<WmState, ZwlrOutputPowerManagerV1, ()>(1, ());
        dh.create_global::<WmState, ExtImageCopyCaptureManagerV1, ()>(1, ());
        dh.create_global::<WmState, ExtOutputImageCaptureSourceManagerV1, ()>(1, ());
        SessionState {
            hooks: SessionHooks::default(),
            cursor: None,
            cursor_shape: CursorShapeManagerState::new::<WmState>(dh),
            activation: XdgActivationState::new::<WmState>(dh),
            foreign_managers: Vec::new(),
            toplevels: Vec::new(),
            names: HashMap::new(),
            handles: Vec::new(),
            out_managers: Vec::new(),
            out_heads: Vec::new(),
            out_serial: 0,
            power_on: HashMap::new(),
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// Window server side: snapshots, announcements, capture
// ---------------------------------------------------------------------------------------------------------------------

impl WmState {
    /// Rebuilds the toplevel snapshot from `wm` and the space, and announces what changed to every foreign-toplevel
    /// manager. Called after focus changes, maximize and commits.
    pub fn sync_toplevels(&mut self) {
        let mut now = Vec::new();
        for id in self.wm.stack() {
            if let Some(names) = self.window(id).and_then(|w| w.toplevel().and_then(|t| toplevel_names(t.wl_surface()))) {
                self.session.names.insert(id, names);
            }
            let (title, app_id) = self.session.names.get(&id).cloned().unwrap_or_default();
            now.push(ToplevelInfo {
                id,
                title,
                app_id,
                activated: self.wm.focused() == Some(id),
                maximized: self.wm.is_maximized(id),
                minimized: !self.wm.visible(id),
                fullscreen: self.wm.is_full_screen(id),
            });
        }
        let changes = diff_toplevels(&self.session.toplevels, &now);
        self.session.toplevels = now;
        for change in &changes {
            announce(&mut self.session, &self.dh, change);
        }
    }

    /// Sends the current `wlr_output` state of every output to every output manager, with a new serial.
    pub fn publish_outputs(&mut self) {
        let outputs: Vec<Output> = self.space.outputs().cloned().collect();
        let dh = self.dh.clone();
        let managers = self.session.out_managers.clone();
        for manager in managers {
            let Some(client) = manager.client() else { continue };
            self.session.out_serial += 1;
            let serial = self.session.out_serial;
            for out in &outputs {
                let name = out.name();
                let idx = match self.session.out_heads.iter().position(|h| h.manager == manager.id() && h.output == name) {
                    Some(i) => i,
                    None => {
                        let Ok(head) = client.create_resource::<ZwlrOutputHeadV1, String, WmState>(&dh, manager.version(), name.clone()) else {
                            continue;
                        };
                        manager.head(&head);
                        head.name(name.clone());
                        head.description(out.description());
                        let phys = out.physical_properties();
                        head.physical_size(phys.size.w, phys.size.h);
                        head.make(phys.make.clone());
                        head.model(phys.model.clone());
                        self.session.out_heads.push(HeadRec { manager: manager.id(), output: name.clone(), head, modes: Vec::new() });
                        self.session.out_heads.len() - 1
                    }
                };
                let rec = &mut self.session.out_heads[idx];
                for mode in out.modes() {
                    let info = mode_info(out, mode);
                    if rec.modes.iter().any(|(m, _)| *m == info) {
                        continue;
                    }
                    let Ok(res) = client.create_resource::<ZwlrOutputModeV1, ModeInfo, WmState>(&dh, rec.head.version(), info) else {
                        continue;
                    };
                    rec.head.mode(&res);
                    res.size(info.w, info.h);
                    res.refresh(info.refresh);
                    if info.preferred {
                        res.preferred();
                    }
                    rec.modes.push((info, res));
                }
                rec.head.enabled(1);
                if let Some(cur) = out.current_mode().map(|m| mode_info(out, m)) {
                    if let Some((_, res)) = rec.modes.iter().find(|(m, _)| *m == cur) {
                        rec.head.current_mode(res);
                    }
                }
                let loc = out.current_location();
                rec.head.position(loc.x, loc.y);
                rec.head.transform(wl_transform(out.current_transform()));
                rec.head.scale(out.current_scale().fractional_scale());
            }
            manager.done(serial);
        }
    }

    /// Applies (or, with `test_only`, only checks) a configuration. Disabling heads is accepted; enabling one needs a
    /// mode the output has, and a mode change needs the `set_mode` hook (the DRM backend sets it in `drm::run`).
    pub fn apply_outputs(&mut self, entries: &[(String, bool, Option<Want>, Option<Transform>)], test_only: bool) -> bool {
        let mut plan: Vec<(Output, String, ModeInfo, bool, Option<Transform>)> = Vec::new();
        for (name, enabled, want, transform) in entries {
            if !enabled {
                continue;
            }
            let Some(out) = self.output_named(name) else { return false };
            let modes: Vec<ModeInfo> = out.modes().into_iter().map(|m| mode_info(&out, m)).collect();
            let current = out.current_mode().map(|m| mode_info(&out, m));
            let chosen = match want {
                Some(w) => choose_mode(&modes, Some(*w)),
                None => current,
            };
            let Some(chosen) = chosen else { return false };
            let changed = Some(chosen) != current;
            plan.push((out, name.clone(), chosen, changed, *transform));
        }
        if test_only {
            return true;
        }
        for (out, name, chosen, changed, transform) in plan {
            if changed {
                let Some(hook) = self.session.hooks.set_mode.as_mut() else { return false };
                if !hook(&name, chosen) {
                    return false;
                }
                let mode = Mode { size: (chosen.w, chosen.h).into(), refresh: chosen.refresh };
                out.change_current_state(Some(mode), None, None, None);
                self.refresh_area();
                // the next frame is composed at the new size (the backend's swapchain was resized by the hook)
                self.dirty = true;
            }
            // after the mode, so the rotation is measured on the mode the output now has
            if let Some(t) = transform {
                if !self.rotate_output(&out, t) {
                    return false;
                }
            }
        }
        true
    }

    /// Looks up a live output by name.
    pub fn output_named(&self, name: &str) -> Option<Output> { self.space.outputs().find(|o| o.name() == name).cloned() }

    /// Runs an xdg-activation request: a token with a recent serial raises and focuses the window.
    pub fn activate_surface(&mut self, surface: &WlSurface, has_serial: bool, age: Duration) {
        if !activation_allowed(has_serial, age) {
            return;
        }
        // a minimized window is not in the space: activation shows it again
        let Some(window) = self.window_for_surface(surface).or_else(|| self.hidden_for_surface(surface)) else { return };
        let Some(id) = WmState::known_id(&window) else { return };
        self.wm.restore(id);
        self.apply_frames();
        self.focus(Some(id), SERIAL_COUNTER.next_serial());
    }

    /// Captures `spec` through the backend hook and writes it into `buffer`; `Err` when a step is missing.
    fn capture_into(&mut self, spec: &CaptureSpec, buffer: &WlBuffer) -> Result<(), &'static str> {
        let (size, stride) = with_buffer_contents(buffer, |_, _, d: BufferData| ((d.width, d.height), d.stride))
            .map_err(|_| "not a shm buffer")?;
        if !buffer_fits(size, stride, (spec.region.w, spec.region.h)) {
            return Err("buffer does not match the capture");
        }
        // the hook renders the scene from `self`, so it is taken out of the session for the call
        let mut hook = self.session.hooks.capture.take().ok_or("no capture backend")?;
        let pixels = hook(self, spec);
        self.session.hooks.capture = Some(hook);
        write_shm(buffer, &pixels.ok_or("capture failed")?)
    }
}

/// Converts RGBA rows (R, G, B, A in memory, what a GPU readback gives) into `format` rows of `dst_stride` bytes.
/// XRGB8888 and ARGB8888 are little-endian words, so in memory they are B, G, R, X/A: the channels are swapped
/// explicitly, never copied through. XRGB8888 stores 0xff in the unused byte. `Err` for any other format or short data.
pub fn rgba_to_shm(src: &[u8], width: usize, height: usize, format: wl_shm::Format, dst: &mut [u8], dst_stride: usize) -> Result<(), &'static str> {
    let opaque = match format {
        wl_shm::Format::Xrgb8888 => true,
        wl_shm::Format::Argb8888 => false,
        _ => return Err("unsupported shm format"),
    };
    if src.len() < width * height * 4 {
        return Err("short capture");
    }
    if dst_stride < width * 4 || (height > 0 && dst.len() < dst_stride * (height - 1) + width * 4) {
        return Err("buffer too small");
    }
    for y in 0..height {
        let from = &src[y * width * 4..(y + 1) * width * 4];
        let to = &mut dst[y * dst_stride..y * dst_stride + width * 4];
        for (p, q) in from.chunks_exact(4).zip(to.chunks_exact_mut(4)) {
            q.copy_from_slice(&[p[2], p[1], p[0], if opaque { 0xff } else { p[3] }]);
        }
    }
    Ok(())
}

/// Cuts `region` out of a full RGBA frame of `frame` (width, height) pixels. The region is clipped to the frame; the
/// result is the clipped rectangle and its rows, `None` when nothing is left.
pub fn crop_rgba(frame_px: &[u8], frame: (i32, i32), region: Rect) -> Option<(Rect, Vec<u8>)> {
    let r = clamp_region(frame, region)?;
    if frame_px.len() < (frame.0 as usize) * (frame.1 as usize) * 4 {
        return None;
    }
    let mut out = Vec::with_capacity((r.w * r.h * 4) as usize);
    for y in r.y..r.y + r.h {
        let start = ((y * frame.0 + r.x) * 4) as usize;
        out.extend_from_slice(&frame_px[start..start + (r.w * 4) as usize]);
    }
    Some((r, out))
}

/// Writes captured RGBA pixels (the `capture` hook's rows) into a client's shm buffer in the format it asked for.
/// This is the one audited raw write into a client pool: the slice covers only the buffer's own rows, and its end is
/// checked against the pool length first.
fn write_shm(buffer: &WlBuffer, pixels: &[u8]) -> Result<(), &'static str> {
    write_shm_region(buffer, pixels, None)
}

/// `write_shm` for a picture of `width` by `height` pixels written from the top-left corner of a buffer at least that
/// large (a window picture is fitted into the buffer the shell sent, so it can be smaller). The rest is not touched.
pub(crate) fn write_shm_sized(buffer: &WlBuffer, pixels: &[u8], width: i32, height: i32) -> Result<(), &'static str> {
    if width <= 0 || height <= 0 {
        return Err("empty picture");
    }
    write_shm_region(buffer, pixels, Some((width as usize, height as usize)))
}

/// The one audited write into a client pool: `region` is the picture's size (`None`: the whole buffer).
fn write_shm_region(buffer: &WlBuffer, pixels: &[u8], region: Option<(usize, usize)>) -> Result<(), &'static str> {
    with_buffer_contents_mut(buffer, |ptr, len, d: BufferData| {
        let (bw, bh) = (d.width.max(0) as usize, d.height.max(0) as usize);
        let (w, h) = region.unwrap_or((bw, bh));
        if w > bw || h > bh {
            return Err("picture is larger than the buffer");
        }
        let (stride, offset) = (d.stride.max(0) as usize, d.offset.max(0) as usize);
        let need = if h == 0 { 0 } else { stride * (h - 1) + w * 4 };
        if offset.checked_add(need).map_or(true, |end| end > len) {
            return Err("buffer is outside its pool");
        }
        // SAFETY: `ptr` starts the mapped pool of `len` bytes; `offset + need <= len` was checked above, and the slice
        // lives only for this call, while the pool stays mapped.
        let dst = unsafe { std::slice::from_raw_parts_mut(ptr.add(offset), need) };
        rgba_to_shm(pixels, w, h, d.format, dst, stride)
    })
    .map_err(|_| "shm buffer is not readable")?
}

/// Title and app id of a toplevel's surface, if it has been configured.
fn toplevel_names(surface: &WlSurface) -> Option<(String, String)> {
    with_states(surface, |states| {
        states.data_map.get::<XdgToplevelSurfaceData>().map(|d| {
            let d = d.lock().unwrap_or_else(|p| p.into_inner());
            (d.title.clone().unwrap_or_default(), d.app_id.clone().unwrap_or_default())
        })
    })
}

fn mode_info(out: &Output, mode: Mode) -> ModeInfo {
    ModeInfo { w: mode.size.w, h: mode.size.h, refresh: mode.refresh, preferred: out.preferred_mode() == Some(mode) }
}

fn wl_transform(t: Transform) -> wl_output::Transform {
    use wl_output::Transform as W;
    match t {
        Transform::Normal => W::Normal,
        Transform::_90 => W::_90,
        Transform::_180 => W::_180,
        Transform::_270 => W::_270,
        Transform::Flipped => W::Flipped,
        Transform::Flipped90 => W::Flipped90,
        Transform::Flipped180 => W::Flipped180,
        Transform::Flipped270 => W::Flipped270,
    }
}

/// The transform a `wl_output.transform` value names: the inverse of `wl_transform`.
fn from_wl_transform(t: wl_output::Transform) -> Transform {
    use wl_output::Transform as W;
    match t {
        W::Normal => Transform::Normal,
        W::_90 => Transform::_90,
        W::_180 => Transform::_180,
        W::_270 => Transform::_270,
        W::Flipped => Transform::Flipped,
        W::Flipped90 => Transform::Flipped90,
        W::Flipped180 => Transform::Flipped180,
        W::Flipped270 => Transform::Flipped270,
        _ => Transform::Normal,
    }
}

/// Sends one toplevel change to every foreign-toplevel manager.
fn announce(session: &mut SessionState, dh: &DisplayHandle, change: &ToplevelChange) {
    session.foreign_managers.retain(|m| m.is_alive());
    match change {
        ToplevelChange::Added(info) => {
            for manager in session.foreign_managers.clone() {
                publish_added(session, dh, &manager, info);
            }
        }
        ToplevelChange::Changed(info) => {
            for entry in session.handles.iter().filter(|h| h.id == info.id) {
                for handle in &entry.handles {
                    send_state(handle, info);
                }
            }
        }
        ToplevelChange::Removed(id) => {
            if let Some(pos) = session.handles.iter().position(|h| h.id == *id) {
                let gone = session.handles.remove(pos);
                for handle in &gone.handles {
                    handle.closed();
                }
            }
        }
    }
}

/// Creates the handle for a window in one manager and sends its whole state.
fn publish_added(session: &mut SessionState, dh: &DisplayHandle, manager: &ZwlrForeignToplevelManagerV1, info: &ToplevelInfo) {
    let Some(client) = manager.client() else { return };
    let Ok(handle) = client.create_resource::<ZwlrForeignToplevelHandleV1, WindowId, WmState>(dh, manager.version(), info.id) else {
        return;
    };
    manager.toplevel(&handle);
    send_state(&handle, info);
    match session.handles.iter_mut().find(|h| h.id == info.id) {
        Some(entry) => entry.handles.push(handle),
        None => session.handles.push(ToplevelHandles { id: info.id, handles: vec![handle] }),
    }
}

/// Sends title, app id and state, then `done`.
fn send_state(handle: &ZwlrForeignToplevelHandleV1, info: &ToplevelInfo) {
    handle.title(info.title.clone());
    handle.app_id(info.app_id.clone());
    handle.state(info.state_bytes());
    handle.done();
}

/// Replays the current toplevels to a manager that has just bound.
fn publish_all_to(session: &mut SessionState, dh: &DisplayHandle, manager: &ZwlrForeignToplevelManagerV1) {
    let all = session.toplevels.clone();
    for info in &all {
        publish_added(session, dh, manager, info);
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// zwlr_foreign_toplevel_manager_v1 and handles
// ---------------------------------------------------------------------------------------------------------------------

impl GlobalDispatch<ZwlrForeignToplevelManagerV1, ()> for WmState {
    fn bind(state: &mut Self, dh: &DisplayHandle, _client: &Client, resource: New<ZwlrForeignToplevelManagerV1>, _data: &(), data_init: &mut DataInit<'_, Self>) {
        let manager = data_init.init(resource, ());
        state.session.foreign_managers.push(manager.clone());
        publish_all_to(&mut state.session, dh, &manager);
    }
}

impl Dispatch<ZwlrForeignToplevelManagerV1, ()> for WmState {
    fn request(state: &mut Self, _client: &Client, resource: &ZwlrForeignToplevelManagerV1, request: zwlr_foreign_toplevel_manager_v1::Request, _data: &(), _dh: &DisplayHandle, _data_init: &mut DataInit<'_, Self>) {
        if let zwlr_foreign_toplevel_manager_v1::Request::Stop = request {
            resource.finished();
            state.session.foreign_managers.retain(|m| m != resource);
        }
    }

    fn destroyed(state: &mut Self, _client: ClientId, resource: &ZwlrForeignToplevelManagerV1, _data: &()) {
        state.session.foreign_managers.retain(|m| m != resource);
    }
}

impl Dispatch<ZwlrForeignToplevelHandleV1, WindowId> for WmState {
    fn request(state: &mut Self, _client: &Client, _resource: &ZwlrForeignToplevelHandleV1, request: zwlr_foreign_toplevel_handle_v1::Request, id: &WindowId, _dh: &DisplayHandle, _data_init: &mut DataInit<'_, Self>) {
        use zwlr_foreign_toplevel_handle_v1::Request as R;
        let id = *id;
        let serial = SERIAL_COUNTER.next_serial();
        // showing a minimized window again needs no window from the space: `apply_frames` maps the one held hidden
        if matches!(request, R::UnsetMinimized | R::Activate { .. }) {
            if state.window_any(id).is_some() {
                state.wm.restore(id);
                state.apply_frames();
                state.focus(Some(id), serial);
            }
            return;
        }
        let Some(window) = state.window(id) else { return };
        match request {
            R::SetMaximized | R::UnsetMaximized => {
                let want = matches!(request, R::SetMaximized);
                if state.wm.is_maximized(id) != want {
                    if let Some(t) = window.toplevel().cloned() {
                        state.set_maximized(&window, id, &t);
                    }
                }
            }
            R::SetMinimized => {
                state.wm.minimize(id);
                state.hide_window(id, &window);
                state.focus(None, serial);
            }
            R::Close => {
                if let Some(t) = window.toplevel() {
                    t.send_close();
                }
            }
            R::SetFullscreen { .. } => state.set_full_screen(id, true),
            R::UnsetFullscreen => state.set_full_screen(id, false),
            // The window manager has no taskbar-rectangle model yet: that request is accepted and ignored.
            _ => {}
        }
    }

    fn destroyed(state: &mut Self, _client: ClientId, resource: &ZwlrForeignToplevelHandleV1, id: &WindowId) {
        for entry in state.session.handles.iter_mut().filter(|h| h.id == *id) {
            entry.handles.retain(|h| h.id() != resource.id());
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// zwlr_output_manager_v1, heads, modes and configurations
// ---------------------------------------------------------------------------------------------------------------------

impl GlobalDispatch<ZwlrOutputManagerV1, ()> for WmState {
    fn bind(state: &mut Self, _dh: &DisplayHandle, _client: &Client, resource: New<ZwlrOutputManagerV1>, _data: &(), data_init: &mut DataInit<'_, Self>) {
        let manager = data_init.init(resource, ());
        state.session.out_managers.push(manager);
        state.publish_outputs();
    }
}

impl Dispatch<ZwlrOutputManagerV1, ()> for WmState {
    fn request(state: &mut Self, _client: &Client, resource: &ZwlrOutputManagerV1, request: zwlr_output_manager_v1::Request, _data: &(), _dh: &DisplayHandle, data_init: &mut DataInit<'_, Self>) {
        match request {
            zwlr_output_manager_v1::Request::CreateConfiguration { id, serial: _ } => {
                data_init.init(id, ConfigEntries::default());
            }
            zwlr_output_manager_v1::Request::Stop => {
                resource.finished();
                state.session.out_managers.retain(|m| m != resource);
            }
            _ => {}
        }
    }

    fn destroyed(state: &mut Self, _client: ClientId, resource: &ZwlrOutputManagerV1, _data: &()) {
        state.session.out_managers.retain(|m| m != resource);
        let id = resource.id();
        state.session.out_heads.retain(|h| h.manager != id);
    }
}

impl Dispatch<ZwlrOutputHeadV1, String> for WmState {
    fn request(_state: &mut Self, _client: &Client, _resource: &ZwlrOutputHeadV1, _request: zwlr_output_head_v1::Request, _data: &String, _dh: &DisplayHandle, _data_init: &mut DataInit<'_, Self>) {}
}

impl Dispatch<ZwlrOutputModeV1, ModeInfo> for WmState {
    fn request(_state: &mut Self, _client: &Client, _resource: &ZwlrOutputModeV1, _request: zwlr_output_mode_v1::Request, _data: &ModeInfo, _dh: &DisplayHandle, _data_init: &mut DataInit<'_, Self>) {}
}

impl Dispatch<ZwlrOutputConfigurationV1, ConfigEntries> for WmState {
    fn request(state: &mut Self, _client: &Client, resource: &ZwlrOutputConfigurationV1, request: zwlr_output_configuration_v1::Request, data: &ConfigEntries, _dh: &DisplayHandle, data_init: &mut DataInit<'_, Self>) {
        use zwlr_output_configuration_v1::Request as R;
        match request {
            R::EnableHead { id, head } => {
                let want: HeadWantRef = Arc::new(Mutex::new(HeadWant::default()));
                data_init.init(id, want.clone());
                if let Some(name) = head.data::<String>() {
                    data.0.lock().unwrap_or_else(|p| p.into_inner()).push(ConfigEntry { output: name.clone(), enabled: true, want });
                }
            }
            R::DisableHead { head } => {
                if let Some(name) = head.data::<String>() {
                    data.0.lock().unwrap_or_else(|p| p.into_inner()).push(ConfigEntry { output: name.clone(), enabled: false, want: HeadWantRef::default() });
                }
            }
            R::Apply => run_configuration(state, resource, data, false),
            R::Test => run_configuration(state, resource, data, true),
            _ => {}
        }
    }
}

/// Applies or tests a configuration and answers `succeeded` or `failed`.
fn run_configuration(state: &mut WmState, resource: &ZwlrOutputConfigurationV1, data: &ConfigEntries, test_only: bool) {
    let entries: Vec<(String, bool, Option<Want>, Option<Transform>)> = data
        .0
        .lock()
        .unwrap()
        .iter()
        .map(|e| {
            let want = e.want.lock().unwrap_or_else(|p| p.into_inner());
            (e.output.clone(), e.enabled, want.mode, want.transform)
        })
        .collect();
    if state.apply_outputs(&entries, test_only) {
        resource.succeeded();
        if !test_only {
            state.publish_outputs();
        }
    } else {
        resource.failed();
    }
}

impl Dispatch<ZwlrOutputConfigurationHeadV1, HeadWantRef> for WmState {
    fn request(_state: &mut Self, _client: &Client, _resource: &ZwlrOutputConfigurationHeadV1, request: zwlr_output_configuration_head_v1::Request, data: &HeadWantRef, _dh: &DisplayHandle, _data_init: &mut DataInit<'_, Self>) {
        use zwlr_output_configuration_head_v1::Request as R;
        match request {
            R::SetMode { mode } => {
                if let Some(info) = mode.data::<ModeInfo>() {
                    data.lock().unwrap_or_else(|p| p.into_inner()).mode = Some((info.w, info.h, Some(info.refresh)));
                }
            }
            R::SetCustomMode { width, height, refresh } => {
                data.lock().unwrap_or_else(|p| p.into_inner()).mode = Some((width, height, (refresh > 0).then_some(refresh)));
            }
            R::SetTransform { transform } => {
                if let WEnum::Value(t) = transform {
                    data.lock().unwrap_or_else(|p| p.into_inner()).transform = Some(from_wl_transform(t));
                }
            }
            // Position, scale and adaptive sync are accepted and ignored until the backend can apply them.
            _ => {}
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// zwlr_output_power_manager_v1 and zwlr_output_power_v1
// ---------------------------------------------------------------------------------------------------------------------

impl GlobalDispatch<ZwlrOutputPowerManagerV1, ()> for WmState {
    fn bind(_state: &mut Self, _dh: &DisplayHandle, _client: &Client, resource: New<ZwlrOutputPowerManagerV1>, _data: &(), data_init: &mut DataInit<'_, Self>) {
        data_init.init(resource, ());
    }
}

impl Dispatch<ZwlrOutputPowerManagerV1, ()> for WmState {
    fn request(state: &mut Self, _client: &Client, _resource: &ZwlrOutputPowerManagerV1, request: zwlr_output_power_manager_v1::Request, _data: &(), _dh: &DisplayHandle, data_init: &mut DataInit<'_, Self>) {
        if let zwlr_output_power_manager_v1::Request::GetOutputPower { id, output } = request {
            let Some(name) = Output::from_resource(&output).map(|o| o.name()) else {
                data_init.init(id, PowerData(None)).failed();
                return;
            };
            let on = state.session.power_on.get(&name).copied().unwrap_or(true);
            let power = data_init.init(id, PowerData(Some(name)));
            power.mode(if on { zwlr_output_power_v1::Mode::On } else { zwlr_output_power_v1::Mode::Off });
        }
    }
}

impl Dispatch<ZwlrOutputPowerV1, PowerData> for WmState {
    fn request(state: &mut Self, _client: &Client, resource: &ZwlrOutputPowerV1, request: zwlr_output_power_v1::Request, data: &PowerData, _dh: &DisplayHandle, _data_init: &mut DataInit<'_, Self>) {
        let zwlr_output_power_v1::Request::SetMode { mode } = request else { return };
        let Some(name) = data.0.clone() else {
            resource.failed();
            return;
        };
        let on = matches!(mode, WEnum::Value(zwlr_output_power_v1::Mode::On));
        let ok = state.session.hooks.set_power.as_mut().is_some_and(|hook| hook(&name, on));
        if ok {
            state.session.power_on.insert(name, on);
            resource.mode(if on { zwlr_output_power_v1::Mode::On } else { zwlr_output_power_v1::Mode::Off });
        } else {
            resource.failed();
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// zwlr_screencopy_manager_v1 and frames
// ---------------------------------------------------------------------------------------------------------------------

impl GlobalDispatch<ZwlrScreencopyManagerV1, ()> for WmState {
    fn bind(_state: &mut Self, _dh: &DisplayHandle, _client: &Client, resource: New<ZwlrScreencopyManagerV1>, _data: &(), data_init: &mut DataInit<'_, Self>) {
        data_init.init(resource, ());
    }
}

impl Dispatch<ZwlrScreencopyManagerV1, ()> for WmState {
    fn request(state: &mut Self, _client: &Client, _resource: &ZwlrScreencopyManagerV1, request: zwlr_screencopy_manager_v1::Request, _data: &(), _dh: &DisplayHandle, data_init: &mut DataInit<'_, Self>) {
        use zwlr_screencopy_manager_v1::Request as R;
        let (frame, output, region, cursor) = match request {
            R::CaptureOutput { frame, output, overlay_cursor, .. } => (frame, output, None, overlay_cursor != 0),
            R::CaptureOutputRegion { frame, output, overlay_cursor, x, y, width, height, .. } => (frame, output, Some(Rect::new(x, y, width, height)), overlay_cursor != 0),
            _ => return,
        };
        let target = Output::from_resource(&output).and_then(|out| {
            let size = output_capture_size(&out)?;
            let req = region.unwrap_or(Rect::new(0, 0, size.0, size.1));
            Some(CaptureTarget { output: out.name(), region: clamp_region(size, req)?, cursor })
        });
        match target {
            Some(target) => {
                let frame = data_init.init(frame, FrameData::new(target.clone()));
                send_buffer_info(&frame, &target, state.session.hooks.capture.is_some());
            }
            None => data_init.init(frame, FrameData::new(CaptureTarget::default())).failed(),
        }
    }
}

/// Announces the buffer a frame needs: XRGB8888 at the capture size. Without a capture backend the frame fails at once.
fn send_buffer_info(frame: &ZwlrScreencopyFrameV1, target: &CaptureTarget, backend: bool) {
    let (w, h) = (target.region.w, target.region.h);
    frame.buffer(wl_shm::Format::Xrgb8888, w as u32, h as u32, shm_stride(w) as u32);
    if frame.version() >= 3 {
        frame.buffer_done();
    }
    if !backend {
        frame.failed();
    }
}

impl Dispatch<ZwlrScreencopyFrameV1, FrameData> for WmState {
    fn request(state: &mut Self, _client: &Client, resource: &ZwlrScreencopyFrameV1, request: zwlr_screencopy_frame_v1::Request, data: &FrameData, _dh: &DisplayHandle, _data_init: &mut DataInit<'_, Self>) {
        use zwlr_screencopy_frame_v1::Request as R;
        let (R::Copy { buffer } | R::CopyWithDamage { buffer }) = request else { return };
        let spec = CaptureSpec { output: data.target.output.clone(), region: data.target.region.clone(), cursor: data.target.cursor };
        match state.capture_into(&spec, &buffer) {
            Ok(()) => {
                resource.flags(zwlr_screencopy_frame_v1::Flags::empty());
                let (hi, lo, ns) = stamp(state.start.elapsed());
                resource.ready(hi, lo, ns);
            }
            Err(_) => resource.failed(),
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// ext-image-copy-capture-v1: sources, sessions and frames
// ---------------------------------------------------------------------------------------------------------------------

impl GlobalDispatch<ExtOutputImageCaptureSourceManagerV1, ()> for WmState {
    fn bind(_state: &mut Self, _dh: &DisplayHandle, _client: &Client, resource: New<ExtOutputImageCaptureSourceManagerV1>, _data: &(), data_init: &mut DataInit<'_, Self>) {
        data_init.init(resource, ());
    }
}

impl Dispatch<ExtOutputImageCaptureSourceManagerV1, ()> for WmState {
    fn request(_state: &mut Self, _client: &Client, _resource: &ExtOutputImageCaptureSourceManagerV1, request: ext_output_image_capture_source_manager_v1::Request, _data: &(), _dh: &DisplayHandle, data_init: &mut DataInit<'_, Self>) {
        if let ext_output_image_capture_source_manager_v1::Request::CreateSource { source, output } = request {
            let name = Output::from_resource(&output).map(|o| o.name()).unwrap_or_default();
            data_init.init(source, name);
        }
    }
}

impl Dispatch<ExtImageCaptureSourceV1, String> for WmState {
    fn request(_state: &mut Self, _client: &Client, _resource: &ExtImageCaptureSourceV1, _request: ext_image_capture_source_v1::Request, _data: &String, _dh: &DisplayHandle, _data_init: &mut DataInit<'_, Self>) {}
}

impl GlobalDispatch<ExtImageCopyCaptureManagerV1, ()> for WmState {
    fn bind(_state: &mut Self, _dh: &DisplayHandle, _client: &Client, resource: New<ExtImageCopyCaptureManagerV1>, _data: &(), data_init: &mut DataInit<'_, Self>) {
        data_init.init(resource, ());
    }
}

impl Dispatch<ExtImageCopyCaptureManagerV1, ()> for WmState {
    fn request(state: &mut Self, _client: &Client, _resource: &ExtImageCopyCaptureManagerV1, request: ext_image_copy_capture_manager_v1::Request, _data: &(), _dh: &DisplayHandle, data_init: &mut DataInit<'_, Self>) {
        use ext_image_copy_capture_manager_v1::Request as R;
        match request {
            R::CreateSession { session, source, .. } => {
                let name = source.data::<String>().cloned().unwrap_or_default();
                let size = state.output_named(&name).and_then(|o| output_capture_size(&o));
                match size {
                    Some(size) => {
                        let target = CaptureTarget { output: name, region: Rect::new(0, 0, size.0, size.1), cursor: false };
                        let s = data_init.init(session, target.clone());
                        s.buffer_size(target.region.w as u32, target.region.h as u32);
                        s.shm_format(wl_shm::Format::Xrgb8888);
                        s.done();
                    }
                    None => data_init.init(session, CaptureTarget::default()).stopped(),
                }
            }
            R::CreatePointerCursorSession { session, .. } => {
                // Cursor sessions are not served: the object is created, and its capture session is stopped at once.
                data_init.init(session, ());
            }
            _ => {}
        }
    }
}

impl Dispatch<ExtImageCopyCaptureCursorSessionV1, ()> for WmState {
    fn request(_state: &mut Self, _client: &Client, _resource: &ExtImageCopyCaptureCursorSessionV1, request: ext_image_copy_capture_cursor_session_v1::Request, _data: &(), _dh: &DisplayHandle, data_init: &mut DataInit<'_, Self>) {
        if let ext_image_copy_capture_cursor_session_v1::Request::GetCaptureSession { session } = request {
            data_init.init(session, CaptureTarget::default()).stopped();
        }
    }
}

impl Dispatch<ExtImageCopyCaptureSessionV1, CaptureTarget> for WmState {
    fn request(_state: &mut Self, _client: &Client, _resource: &ExtImageCopyCaptureSessionV1, request: ext_image_copy_capture_session_v1::Request, data: &CaptureTarget, _dh: &DisplayHandle, data_init: &mut DataInit<'_, Self>) {
        if let ext_image_copy_capture_session_v1::Request::CreateFrame { frame } = request {
            data_init.init(frame, FrameData::new(data.clone()));
        }
    }
}

impl Dispatch<ExtImageCopyCaptureFrameV1, FrameData> for WmState {
    fn request(state: &mut Self, _client: &Client, resource: &ExtImageCopyCaptureFrameV1, request: ext_image_copy_capture_frame_v1::Request, data: &FrameData, _dh: &DisplayHandle, _data_init: &mut DataInit<'_, Self>) {
        use ext_image_copy_capture_frame_v1::{FailureReason, Request as R};
        match request {
            R::AttachBuffer { buffer } => *data.buffer.lock().unwrap_or_else(|p| p.into_inner()) = Some(buffer),
            R::DamageBuffer { .. } => {}
            R::Capture => {
                let buffer = data.buffer.lock().unwrap_or_else(|p| p.into_inner()).take();
                let Some(buffer) = buffer else {
                    resource.failed(FailureReason::BufferConstraints);
                    return;
                };
                let spec = CaptureSpec { output: data.target.output.clone(), region: data.target.region.clone(), cursor: data.target.cursor };
                match state.capture_into(&spec, &buffer) {
                    Ok(()) => {
                        resource.transform(wl_output::Transform::Normal);
                        resource.damage(0, 0, data.target.region.w, data.target.region.h);
                        let (hi, lo, ns) = stamp(state.start.elapsed());
                        resource.presentation_time(hi, lo, ns);
                        resource.ready();
                    }
                    Err(_) => resource.failed(FailureReason::Unknown),
                }
            }
            _ => {}
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// xdg-activation and cursor shapes (Smithay)
// ---------------------------------------------------------------------------------------------------------------------

impl XdgActivationHandler for WmState {
    fn activation_state(&mut self) -> &mut XdgActivationState { &mut self.session.activation }

    fn request_activation(&mut self, token: XdgActivationToken, data: XdgActivationTokenData, surface: WlSurface) {
        self.activate_surface(&surface, data.serial.is_some(), data.timestamp.elapsed());
        self.session.activation.remove_token(&token);
    }
}

smithay::delegate_xdg_activation!(WmState);
delegate_cursor_shape!(WmState);

impl smithay::wayland::tablet_manager::TabletSeatHandler for WmState {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_transform_request_reads_back_the_transform_it_names() {
        for t in [
            Transform::Normal,
            Transform::_90,
            Transform::_180,
            Transform::_270,
            Transform::Flipped,
            Transform::Flipped90,
            Transform::Flipped180,
            Transform::Flipped270,
        ] {
            assert_eq!(from_wl_transform(wl_transform(t)), t, "{t:?}");
        }
    }

    #[test]
    fn argb8888_is_bgra_in_memory() {
        let src = [1, 2, 3, 4, 5, 6, 7, 8];
        let mut dst = [0u8; 8];
        rgba_to_shm(&src, 2, 1, wl_shm::Format::Argb8888, &mut dst, 8).unwrap();
        assert_eq!(dst, [3, 2, 1, 4, 7, 6, 5, 8]);
    }

    #[test]
    fn xrgb8888_swaps_red_and_blue_and_forces_opaque() {
        let mut dst = [0u8; 4];
        rgba_to_shm(&[10, 20, 30, 0], 1, 1, wl_shm::Format::Xrgb8888, &mut dst, 4).unwrap();
        assert_eq!(dst, [30, 20, 10, 0xff]);
    }

    #[test]
    fn rows_honour_the_destination_stride_and_leave_padding() {
        let src = [1, 2, 3, 4, 5, 6, 7, 8];
        let mut dst = [9u8; 12];
        rgba_to_shm(&src, 1, 2, wl_shm::Format::Argb8888, &mut dst, 8).unwrap();
        assert_eq!(dst, [3, 2, 1, 4, 9, 9, 9, 9, 7, 6, 5, 8]);
    }

    #[test]
    fn conversion_refuses_other_formats_and_short_buffers() {
        let mut dst = [0u8; 4];
        assert!(rgba_to_shm(&[0; 4], 1, 1, wl_shm::Format::Rgb565, &mut dst, 4).is_err());
        assert!(rgba_to_shm(&[0; 3], 1, 1, wl_shm::Format::Argb8888, &mut dst, 4).is_err());
        assert!(rgba_to_shm(&[0; 8], 2, 1, wl_shm::Format::Argb8888, &mut dst, 4).is_err());
    }

    #[test]
    fn a_region_is_cropped_and_clipped_to_the_frame() {
        // 3x2 frame; each pixel's first byte is its index
        let frame: Vec<u8> = (0..6u8).flat_map(|i| [i, 0, 0, 0]).collect();
        let (r, px) = crop_rgba(&frame, (3, 2), Rect::new(1, 0, 5, 5)).unwrap();
        assert_eq!(r, Rect::new(1, 0, 2, 2));
        assert_eq!(px.iter().step_by(4).copied().collect::<Vec<_>>(), vec![1, 2, 4, 5]);
        assert!(crop_rgba(&frame, (3, 2), Rect::new(3, 0, 2, 2)).is_none());
        assert!(crop_rgba(&frame[..8], (3, 2), Rect::new(0, 0, 3, 2)).is_none());
    }

    fn info(id: WindowId, title: &str) -> ToplevelInfo { ToplevelInfo { id, title: title.into(), ..Default::default() } }

    fn mode(w: i32, h: i32, refresh: i32) -> ModeInfo { ModeInfo { w, h, refresh, preferred: false } }

    #[test]
    fn diffing_reports_added_changed_and_removed_in_order() {
        let old = vec![info(1, "a"), info(2, "b")];
        let new = vec![info(1, "a2"), info(3, "c")];
        assert_eq!(
            diff_toplevels(&old, &new),
            vec![ToplevelChange::Changed(info(1, "a2")), ToplevelChange::Added(info(3, "c")), ToplevelChange::Removed(2)]
        );
    }

    #[test]
    fn an_unchanged_list_produces_no_changes() {
        let list = vec![info(1, "a"), info(2, "b")];
        assert!(diff_toplevels(&list, &list).is_empty());
        assert!(diff_toplevels(&[], &[]).is_empty());
    }

    #[test]
    fn a_focus_or_state_change_is_a_change() {
        let old = vec![info(1, "a")];
        let focused = ToplevelInfo { activated: true, ..info(1, "a") };
        assert_eq!(diff_toplevels(&old, std::slice::from_ref(&focused)), vec![ToplevelChange::Changed(focused)]);
    }

    #[test]
    fn state_bytes_follow_the_protocol_enum_order() {
        let t = ToplevelInfo { activated: true, minimized: true, ..Default::default() };
        let mut want = Vec::new();
        want.extend_from_slice(&1u32.to_ne_bytes());
        want.extend_from_slice(&2u32.to_ne_bytes());
        assert_eq!(t.state_bytes(), want);
        assert!(ToplevelInfo::default().state_bytes().is_empty());
    }

    #[test]
    fn the_exact_mode_wins_and_a_missing_refresh_is_the_nearest_within_one_hertz() {
        let modes = [mode(1920, 1080, 60000), mode(1920, 1080, 59940), mode(1280, 720, 60000), mode(1920, 1080, 144000)];
        assert_eq!(choose_mode(&modes, Some((1920, 1080, Some(59940)))), Some(mode(1920, 1080, 59940)));
        assert_eq!(choose_mode(&modes, Some((1920, 1080, Some(60100)))), Some(mode(1920, 1080, 60000)));
        // 75 Hz is 15 Hz from 60 and 69 from 144: outside the tolerance, so no mode.
        assert_eq!(choose_mode(&modes, Some((1920, 1080, Some(75000)))), None);
        // Without a refresh the fastest mode of that size is chosen.
        assert_eq!(choose_mode(&modes, Some((1920, 1080, None))), Some(mode(1920, 1080, 144000)));
        assert_eq!(choose_mode(&modes, Some((800, 600, None))), None);
    }

    #[test]
    fn no_request_picks_the_preferred_mode_then_the_first() {
        let preferred = ModeInfo { preferred: true, ..mode(2560, 1440, 60000) };
        let modes = [mode(1920, 1080, 60000), preferred];
        assert_eq!(choose_mode(&modes, None), Some(preferred));
        assert_eq!(choose_mode(&modes[..1], None), Some(mode(1920, 1080, 60000)));
        assert_eq!(choose_mode(&[], None), None);
    }

    #[test]
    fn a_capture_region_is_clipped_to_the_output() {
        let out = (1920, 1080);
        assert_eq!(clamp_region(out, Rect::new(100, 50, 400, 300)), Some(Rect::new(100, 50, 400, 300)));
        assert_eq!(clamp_region(out, Rect::new(-100, 1000, 300, 300)), Some(Rect::new(0, 1000, 200, 80)));
        assert_eq!(clamp_region(out, Rect::new(2000, 0, 10, 10)), None);
        assert_eq!(clamp_region(out, Rect::new(0, 0, 0, 10)), None);
    }

    #[test]
    fn a_rotated_output_is_captured_in_its_displayed_orientation() {
        assert_eq!(capture_size((1920, 1080), Transform::Normal), (1920, 1080));
        assert_eq!(capture_size((1920, 1080), Transform::_180), (1920, 1080));
        assert_eq!(capture_size((1920, 1080), Transform::_90), (1080, 1920));
        assert_eq!(capture_size((1920, 1080), Transform::Flipped270), (1080, 1920));
        // a region past the displayed width is clipped to it, not to the panel's native width
        let shown = capture_size((1920, 1080), Transform::_90);
        assert_eq!(clamp_region(shown, Rect::new(1000, 0, 500, 500)), Some(Rect::new(1000, 0, 80, 500)));
    }

    #[test]
    fn a_shm_buffer_must_match_the_capture_size_and_row_stride() {
        assert_eq!(shm_stride(1920), 7680);
        assert!(buffer_fits((1920, 1080), 7680, (1920, 1080)));
        assert!(!buffer_fits((1920, 1080), 7680 + 64, (1920, 1080)), "padded rows are refused");
        assert!(!buffer_fits((1280, 720), 7680, (1920, 1080)));
    }

    #[test]
    fn an_activation_needs_a_serial_and_a_recent_token() {
        assert!(activation_allowed(true, Duration::from_secs(1)));
        assert!(activation_allowed(true, ACTIVATION_WINDOW));
        assert!(!activation_allowed(true, ACTIVATION_WINDOW + Duration::from_millis(1)));
        assert!(!activation_allowed(false, Duration::ZERO));
    }

    #[test]
    fn timestamps_split_into_hi_lo_and_nanoseconds() {
        assert_eq!(stamp(Duration::new(5, 250)), (0, 5, 250));
        assert_eq!(stamp(Duration::new((1u64 << 32) + 7, 999_999_999)), (1, 7, 999_999_999));
    }
}
