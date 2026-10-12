//! The DRM/KMS backend: the window server on real hardware (and in the VM). libseat gives access to the GPU and
//! input devices, libinput reads input, and Smithay's DRM compositor presents on the first connected display at its
//! native size and fastest refresh.

use std::{error::Error, path::Path, time::Duration};

use smithay::{
    backend::{
        allocator::{
            gbm::{GbmAllocator, GbmBufferFlags, GbmDevice},
            Fourcc,
        },
        drm::{
            compositor::{DrmCompositor, FrameFlags},
            exporter::gbm::GbmFramebufferExporter,
            DrmDevice, DrmDeviceFd, DrmEvent,
        },
        egl::{EGLContext, EGLDisplay},
        input::InputEvent,
        libinput::{LibinputInputBackend, LibinputSessionInterface},
        renderer::{
            element::{
                memory::{MemoryRenderBuffer, MemoryRenderBufferRenderElement},
                Kind,
            },
            gles::GlesRenderer,
        },
        session::{libseat::LibSeatSession, Event as SessionEvent, Session},
        udev::{all_gpus, primary_gpu},
    },
    output::{Mode as WlMode, Output, PhysicalProperties, Subpixel},
    reexports::{
        calloop::{timer::{TimeoutAction, Timer}, EventLoop, LoopHandle},
        drm::control::{connector, crtc, Device as ControlDevice, Mode as DrmMode, ModeTypeFlags},
        input::Libinput,
        rustix::fs::OFlags,
        wayland_server::Display,
    },
    utils::{DeviceFd, Transform},
};

use crate::{scene::SceneElement, state::WmState};

const CLEAR: [f32; 4] = [0.06, 0.06, 0.08, 1.0];
const SUPPORTED: &[Fourcc] = &[Fourcc::Argb8888, Fourcc::Xrgb8888, Fourcc::Abgr8888, Fourcc::Xbgr8888];


type Compositor = DrmCompositor<GbmAllocator<DrmDeviceFd>, GbmFramebufferExporter<DrmDeviceFd>, (), DrmDeviceFd>;

struct Drm {
    renderer: GlesRenderer,
    compositor: Compositor,
    output: Output,
    pointer: MemoryRenderBuffer,
    /// A frame is queued and its vblank has not arrived yet.
    pending: bool,
    /// The loader's frame while the first session of a boot fades in (`BIFROST_BOOT_FADE`).
    boot: Option<Boot>,
    glass: crate::glass::Glass,
}

struct Boot {
    splash: MemoryRenderBuffer,
    /// When the shell's first desktop frame arrived (`$XDG_RUNTIME_DIR/shell-ready`).
    ready_at: Option<std::time::Instant>,
}

impl Boot {
    fn new(w: i32, h: i32) -> Self {
        let mut px = vec![0u32; (w * h) as usize];
        bootweave_frame::render(&bootweave_frame::Frame::boot(w as u32, h as u32), &mut px, w as usize, bootweave_frame::PixelOrder::Bgrx);
        let bytes: Vec<u8> = px.iter().flat_map(|p| p.to_le_bytes()).collect();
        Boot { splash: MemoryRenderBuffer::from_slice(&bytes, Fourcc::Argb8888, (w, h), 1, Transform::Normal, None), ready_at: None }
    }

    /// The splash opacity for this frame, None once the fade is done.
    fn alpha(&mut self) -> Option<f32> {
        if self.ready_at.is_none() {
            let dir = std::env::var_os("XDG_RUNTIME_DIR").map(std::path::PathBuf::from).unwrap_or_default();
            if dir.join("shell-ready").exists() {
                self.ready_at = Some(std::time::Instant::now());
                crate::boot::mark("desktop-first-ui");
            }
        }
        let a = crate::boot::splash_alpha(self.ready_at.map(|t| t.elapsed().as_secs_f64() * 1000.0));
        if a.is_none() {
            crate::boot::mark("desktop-faded");
        }
        a
    }
}

/// Runs the window server on the seat's primary GPU until the session ends.
pub fn run(socket: Option<&str>, command: &[String]) -> Result<(), Box<dyn Error>> {
    let mut event_loop: EventLoop<'static, WmState> = EventLoop::try_new()?;
    let display: Display<WmState> = Display::new()?;
    let mut state = WmState::new(&mut event_loop, display, socket);

    let (mut session, notifier) = LibSeatSession::new().map_err(|e| format!("libseat: {e}"))?;
    let seat = session.seat();
    let gpu = primary_gpu(&seat)?.or_else(|| all_gpus(&seat).ok()?.into_iter().next()).ok_or("no GPU")?;
    eprintln!("bifrost-wm: GPU {}", gpu.display());
    let drm = open_gpu(&mut session, &gpu, &mut state, event_loop.handle())?;
    let backend = std::rc::Rc::new(std::cell::RefCell::new(drm));

    // input
    let mut libinput = Libinput::new_with_udev::<LibinputSessionInterface<LibSeatSession>>(session.clone().into());
    libinput.udev_assign_seat(&seat).map_err(|_| "libinput: cannot assign seat")?;
    event_loop.handle().insert_source(LibinputInputBackend::new(libinput.clone()), |event, _, state| {
        if !matches!(event, InputEvent::DeviceAdded { .. } | InputEvent::DeviceRemoved { .. }) {
            state.process_input_event(event);
        }
    })?;

    // VT switches pause and resume the GPU and input
    let b = backend.clone();
    event_loop.handle().insert_source(notifier, move |event, _, _state| match event {
        SessionEvent::PauseSession => libinput.suspend(),
        SessionEvent::ActivateSession => {
            if let Err(e) = libinput.resume() {
                eprintln!("bifrost-wm: libinput resume: {e:?}");
            }
            let mut d = b.borrow_mut();
            d.pending = false;
            let _ = d.compositor.reset_state();
        }
    })?;

    // first frame, then one per vblank; a quiet desktop is re-checked at the refresh rate
    let b = backend.clone();
    event_loop.handle().insert_source(Timer::immediate(), move |_, _, state| {
        let mut d = b.borrow_mut();
        if !d.pending {
            d.render(state);
        }
        TimeoutAction::ToDuration(Duration::from_millis(16))
    })?;
    // clients' dmabufs (the shell's GPU presenter) are imported by this renderer
    let formats = {
        use smithay::backend::renderer::ImportDma;
        backend.borrow().renderer.dmabuf_formats()
    };
    let _dmabuf_global = state.dmabuf_state.create_global::<WmState>(&state.dh, formats);
    let b = backend.clone();
    state.dmabuf_import = Some(Box::new(move |d| {
        use smithay::backend::renderer::ImportDma;
        b.borrow_mut().renderer.import_dmabuf(d, None).is_ok()
    }));
    let b = backend.clone();
    state.on_vblank = Some(Box::new(move |state| {
        let mut d = b.borrow_mut();
        if let Err(e) = d.compositor.frame_submitted() {
            eprintln!("bifrost-wm: frame submitted: {e}");
        }
        d.pending = false;
        d.render(state);
    }));

    let name = state.socket_name.to_string_lossy().into_owned();
    eprintln!("bifrost-wm: DRM on {name}");
    std::env::set_var("WAYLAND_DISPLAY", &name);
    crate::spawn(command, &name);
    event_loop.run(None, &mut state, |state| {
        state.space.refresh();
        state.popups.cleanup();
        let _ = state.dh.flush_clients();
    })?;
    Ok(())
}

fn open_gpu(session: &mut LibSeatSession, path: &Path, state: &mut WmState, handle: LoopHandle<'static, WmState>) -> Result<Drm, Box<dyn Error>> {
    let fd = session.open(path, OFlags::RDWR | OFlags::CLOEXEC | OFlags::NOCTTY | OFlags::NONBLOCK)?;
    let fd = DrmDeviceFd::new(DeviceFd::from(fd));
    // `false`: keep the connectors as the loader left them; resetting them would blank the display (one modeset per boot)
    let (mut device, notifier) = DrmDevice::new(fd.clone(), false)?;
    let gbm = GbmDevice::new(fd.clone())?;
    // SAFETY: the EGL display and context live as long as the renderer.
    let display = unsafe { EGLDisplay::new(gbm.clone())? };
    let context = EGLContext::new(&display)?;
    let renderer = unsafe { GlesRenderer::new(context)? };

    let res = device.resource_handles()?;
    let conn = res
        .connectors()
        .iter()
        .filter_map(|c| device.get_connector(*c, true).ok())
        .find(|c| c.state() == connector::State::Connected && !c.modes().is_empty())
        .ok_or("no connected display")?;
    let crtc = pick_crtc(&device, &res, &conn).ok_or("no CRTC for the display")?;
    let active = device.get_crtc(crtc).ok().and_then(|c| c.mode());
    let mode = choose_mode(conn.modes(), active, std::env::var("BIFROST_WM_MODE").ok().as_deref()).ok_or("no display mode")?;
    eprintln!("bifrost-wm: mode {}", if active == Some(mode) { "inherited (no modeset)" } else { "selected" });
    let (w, h) = mode.size();
    eprintln!("bifrost-wm: {}-{} {w}x{h}@{}", conn.interface().as_str(), conn.interface_id(), mode.vrefresh());

    let surface = device.create_surface(crtc, mode, &[conn.handle()])?;
    let allocator = GbmAllocator::new(gbm.clone(), GbmBufferFlags::RENDERING | GbmBufferFlags::SCANOUT);
    let exporter = GbmFramebufferExporter::new(gbm.clone(), None);
    let render_formats = renderer.egl_context().dmabuf_render_formats().clone();

    let output = Output::new(
        format!("{}-{}", conn.interface().as_str(), conn.interface_id()),
        PhysicalProperties {
            size: conn.size().map(|(w, h)| (w as i32, h as i32)).unwrap_or((0, 0)).into(),
            subpixel: Subpixel::Unknown,
            make: "BifrOSt".into(),
            model: "Display".into(),
        },
    );
    let wl_mode = WlMode::from(mode);
    let _global = output.create_global::<WmState>(&state.dh);
    output.change_current_state(Some(wl_mode), Some(Transform::Normal), None, Some((0, 0).into()));
    output.set_preferred(wl_mode);
    state.add_output(&output);

    let compositor = DrmCompositor::new(
        &output,
        surface,
        None,
        allocator,
        exporter,
        SUPPORTED.iter().copied(),
        render_formats,
        device.cursor_size(),
        Some(gbm),
    )
    .map_err(|e| format!("DRM compositor: {e}"))?;

    handle.insert_source(notifier, |event, _, state: &mut WmState| match event {
        DrmEvent::VBlank(_) => {
            if let Some(mut f) = state.on_vblank.take() {
                f(state);
                state.on_vblank = Some(f);
            }
        }
        DrmEvent::Error(e) => eprintln!("bifrost-wm: DRM: {e}"),
    })?;
    // the device must outlive its surfaces; it is kept for the life of the process
    std::mem::forget(device);

    let pointer = MemoryRenderBuffer::from_slice(&arrow(), Fourcc::Abgr8888, (ARROW, ARROW), 1, Transform::Normal, None);
    let boot = std::env::var_os("BIFROST_BOOT_FADE").map(|_| Boot::new(w as i32, h as i32));
    let mut renderer = renderer;
    let glass = crate::glass::Glass::new(&mut renderer)?;
    Ok(Drm { renderer, compositor, output, pointer, pending: false, boot, glass })
}

impl Drm {
    fn render(&mut self, state: &mut WmState) {
        let output = self.output.clone();
        let mut top: Vec<SceneElement> = Vec::new();
        // front to back: the boot frame over everything while it fades, else the pointer on top
        let splash = self.boot.as_mut().and_then(|b| b.alpha().map(|a| (b.splash.clone(), a)));
        if splash.is_none() {
            self.boot = None;
        }
        if let Some((buffer, alpha)) = splash {
            if let Ok(e) = MemoryRenderBufferRenderElement::from_buffer(&mut self.renderer, (0.0, 0.0), &buffer, Some(alpha), None, None, Kind::Unspecified) {
                top.push(SceneElement::Memory(e));
            }
        } else {
            let pos = state.pointer_location();
            if let Ok(e) = MemoryRenderBufferRenderElement::from_buffer(&mut self.renderer, (pos.x, pos.y), &self.pointer, None, None, None, Kind::Cursor) {
                top.push(SceneElement::Memory(e));
            }
        }
        let elements = crate::scene::build(&mut self.renderer, state, &output, &mut self.glass, top);
        match self.compositor.render_frame(&mut self.renderer, &elements, CLEAR, FrameFlags::DEFAULT) {
            Ok(frame) => {
                if !frame.is_empty {
                    match self.compositor.queue_frame(()) {
                        Ok(()) => self.pending = true,
                        Err(e) => eprintln!("bifrost-wm: queue frame: {e}"),
                    }
                }
            }
            Err(e) => eprintln!("bifrost-wm: render: {e}"),
        }
        state.send_frames(&output);
    }
}

/// The boot's one modeset stays: the mode the CRTC already shows (the loader's, or the learned `video=` mode) is
/// kept. Without one, the native size at its fastest refresh. `BIFROST_WM_MODE=WxH@R` overrides both.
fn choose_mode(modes: &[DrmMode], active: Option<DrmMode>, want: Option<&str>) -> Option<DrmMode> {
    let summary: Vec<ModeSummary> = modes
        .iter()
        .map(|m| ModeSummary { w: m.size().0, h: m.size().1, hz: m.vrefresh(), preferred: m.mode_type().contains(ModeTypeFlags::PREFERRED) })
        .collect();
    let active = active.and_then(|a| modes.iter().position(|m| *m == a));
    pick_mode(&summary, active, want).map(|i| modes[i])
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct ModeSummary { pub w: u16, pub h: u16, pub hz: u32, pub preferred: bool }

/// `choose_mode` on plain data: the index of the mode to show.
pub(crate) fn pick_mode(modes: &[ModeSummary], active: Option<usize>, want: Option<&str>) -> Option<usize> {
    if let Some(want) = want {
        if let Some(i) = modes.iter().position(|m| format!("{}x{}@{}", m.w, m.h, m.hz) == want) {
            return Some(i);
        }
    }
    if let Some(a) = active.filter(|&a| a < modes.len()) {
        return Some(a);
    }
    let native = modes.iter().find(|m| m.preferred).or(modes.first())?;
    (0..modes.len()).filter(|&i| (modes[i].w, modes[i].h) == (native.w, native.h)).max_by_key(|&i| modes[i].hz)
}

fn pick_crtc(device: &DrmDevice, res: &smithay::reexports::drm::control::ResourceHandles, conn: &connector::Info) -> Option<crtc::Handle> {
    conn.encoders().iter().filter_map(|e| device.get_encoder(*e).ok()).find_map(|enc| {
        enc.crtc().or_else(|| res.filter_crtcs(enc.possible_crtcs()).first().copied())
    })
}

const ARROW: i32 = 24;

/// The arrow outline in pointer pixels, tip at the origin.
const ARROW_SHAPE: [(f32, f32); 7] = [(0.5, 0.5), (0.5, 17.5), (4.5, 13.5), (7.5, 20.5), (10.5, 19.0), (7.5, 12.5), (12.5, 12.5)];

fn inside(x: f32, y: f32) -> bool {
    let mut hit = false;
    let n = ARROW_SHAPE.len();
    for i in 0..n {
        let (x0, y0) = ARROW_SHAPE[i];
        let (x1, y1) = ARROW_SHAPE[(i + 1) % n];
        if (y0 > y) != (y1 > y) && x < x0 + (y - y0) * (x1 - x0) / (y1 - y0) {
            hit = !hit;
        }
    }
    hit
}

/// A plain arrow pointer: white with a dark outline, until client cursor images arrive (phase 3).
fn arrow() -> Vec<u8> {
    let n = ARROW as usize;
    let mut px = vec![0u8; n * n * 4];
    for y in 0..n {
        for x in 0..n {
            let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
            if !inside(fx, fy) {
                continue;
            }
            let edge = [(-1.0, 0.0), (1.0, 0.0), (0.0, -1.0), (0.0, 1.0)].iter().any(|(dx, dy)| !inside(fx + dx, fy + dy));
            let c = if edge { 24 } else { 255 };
            let i = (y * n + x) * 4;
            px[i..i + 4].copy_from_slice(&[c, c, c, 255]);
        }
    }
    px
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(w: u16, h: u16, hz: u32, preferred: bool) -> ModeSummary { ModeSummary { w, h, hz, preferred } }

    #[test]
    fn the_active_mode_is_kept_so_the_boot_has_one_modeset() {
        let modes = [m(2560, 1440, 60, true), m(2560, 1440, 165, false), m(1920, 1080, 60, false)];
        assert_eq!(pick_mode(&modes, Some(0), None), Some(0), "the loader's 60 Hz stays: no retrain blink");
        assert_eq!(pick_mode(&modes, Some(1), None), Some(1), "a learned video= mode stays");
    }

    #[test]
    fn without_an_active_mode_the_native_size_at_its_fastest_refresh_is_chosen() {
        let modes = [m(1920, 1080, 75, false), m(2560, 1440, 60, true), m(2560, 1440, 165, false)];
        assert_eq!(pick_mode(&modes, None, None), Some(2));
        assert_eq!(pick_mode(&[], None, None), None);
    }

    #[test]
    fn an_explicit_mode_wins_when_the_display_offers_it() {
        let modes = [m(2560, 1440, 165, true), m(1920, 1080, 60, false)];
        assert_eq!(pick_mode(&modes, Some(0), Some("1920x1080@60")), Some(1));
        assert_eq!(pick_mode(&modes, Some(0), Some("800x600@60")), Some(0), "an unknown request keeps the active mode");
    }

    #[test]
    fn the_arrow_has_a_white_body_a_dark_outline_and_a_transparent_outside() {
        let px = arrow();
        let at = |x: usize, y: usize| px[(y * ARROW as usize + x) * 4..][..4].to_vec();
        assert_eq!(at(3, 10), vec![255, 255, 255, 255], "body");
        assert_eq!(at(0, 5), vec![24, 24, 24, 255], "left outline");
        assert_eq!(at(20, 2)[3], 0, "outside");
    }
}
