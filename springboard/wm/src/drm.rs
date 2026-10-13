//! The DRM/KMS backend: the window server on real hardware (and in the VM). libseat gives access to the GPU and
//! input devices, libinput reads input, and Smithay's DRM compositor presents on the first connected display at its
//! native size and fastest refresh.

use std::{cell::Cell, error::Error, path::Path, rc::Rc, time::Duration};

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

use crate::{scene::SceneElement, session_protocols::ModeInfo, state::WmState};

pub(crate) const CLEAR: [f32; 4] = [0.06, 0.06, 0.08, 1.0];
const SUPPORTED: &[Fourcc] = &[Fourcc::Argb8888, Fourcc::Xrgb8888, Fourcc::Abgr8888, Fourcc::Xbgr8888];


type Compositor = DrmCompositor<GbmAllocator<DrmDeviceFd>, GbmFramebufferExporter<DrmDeviceFd>, (), DrmDeviceFd>;

struct Drm {
    renderer: GlesRenderer,
    compositor: Compositor,
    output: Output,
    /// The connector's modes, for a mode request from the shell (`set_mode`).
    modes: Vec<DrmMode>,
    pointer: MemoryRenderBuffer,
    /// A frame is queued and its vblank has not arrived yet.
    pending: bool,
    /// The loader's frame while the first session of a boot fades in (`BIFROST_BOOT_FADE`).
    boot: Option<Boot>,
    glass: crate::glass::Glass,
    /// The Vulkan scanout, when `BIFROST_WM_RENDERER=vulkan` started it; `None` keeps GLES scanout.
    vulkan: Option<crate::drm_vulkan::Vulkan>,
    /// The boot frame back over the desktop at shutdown (`SHUTDOWN_REQUEST`).
    shutdown: Shutdown,
    /// The session is switched away (VT switch): nothing is composed or flipped until it comes back (WM-16).
    paused: bool,
}

/// The loader's frame as a render buffer of the panel's size (the boot's first fade and the shutdown fade share it).
/// The bar continues from the supervisor's published position and appearance (`boot::held_frame`, BOOT-4).
fn boot_frame_buffer(w: i32, h: i32) -> MemoryRenderBuffer {
    let record = std::fs::read(crate::boot::PROGRESS_FILE).ok();
    let frame = crate::boot::held_frame(w as u32, h as u32, record.as_deref(), 0.0);
    let mut px = vec![0u32; (w * h) as usize];
    bootweave_frame::render(&frame, &mut px, w as usize, bootweave_frame::PixelOrder::Bgrx);
    let mut bytes = Vec::with_capacity(px.len() * 4);
    for p in &px {
        bytes.extend_from_slice(&p.to_le_bytes());
    }
    MemoryRenderBuffer::from_slice(&bytes, Fourcc::Argb8888, (w, h), 1, Transform::Normal, None)
}

struct Boot {
    splash: MemoryRenderBuffer,
    /// Holds the boot frame until the desktop is ready (`bootreveal::Reveal`: the three desktop-part marks, 4 s ceiling).
    reveal: crate::boot::Reveal,
    /// The gate's clock: milliseconds are counted from here.
    started: std::time::Instant,
}

impl Boot {
    fn new(w: i32, h: i32) -> Self {
        Boot { splash: boot_frame_buffer(w, h), reveal: crate::boot::Reveal::new(0.0), started: std::time::Instant::now() }
    }

    /// The splash opacity for this frame: 1 while held, then the one fade; None once the fade is done.
    fn alpha(&mut self) -> Option<f32> {
        let now = self.started.elapsed().as_secs_f64() * 1000.0;
        let was_ready = self.reveal.ready_at().is_some();
        let ready_at = self.reveal.poll(now, crate::boot::read_marks_file);
        if ready_at.is_some() && !was_ready {
            crate::boot::mark("desktop-ready");
        }
        let a = crate::boot::splash_alpha(ready_at.map(|t| now - t));
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
    // the page flip's vblank time, written by the DRM event source and read by the vblank callback below
    let vblank_time: Rc<Cell<Option<Duration>>> = Rc::new(Cell::new(None));
    let mut drm = open_gpu(&mut session, &gpu, &mut state, event_loop.handle(), vblank_time.clone())?;
    // the Vulkan scanout on the same GPU; any failure keeps the GLES scanout that open_gpu just set up
    if crate::render_server::selected() {
        let size = drm.output.current_mode().map(|m| (m.size.w, m.size.h)).unwrap_or((0, 0));
        let refresh = drm.output.current_mode().map(|m| m.refresh).unwrap_or(0);
        match crate::drm_vulkan::start(&gpu, drm.compositor.surface(), size, refresh) {
            Ok(v) => drm.vulkan = Some(v),
            Err(e) => eprintln!("bifrost-wm: vulkan scanout unavailable, GLES: {e}"),
        }
    }
    // the shell's output mode requests (zwlr output management, `apply_outputs`) reach the scanout through this hook
    let b = Rc::new(std::cell::RefCell::new(drm));
    let hook = b.clone();
    state.session.hooks.set_mode = Some(Box::new(move |name: &str, mode: ModeInfo| hook.borrow_mut().set_mode(name, mode)));
    let backend = b;

    // input
    let mut libinput = Libinput::new_with_udev::<LibinputSessionInterface<LibSeatSession>>(session.clone().into());
    libinput.udev_assign_seat(&seat).map_err(|_| "libinput: cannot assign seat")?;
    event_loop.handle().insert_source(LibinputInputBackend::new(libinput.clone()), |event, _, state| {
        match event {
            // a new device takes the shell's pointer and trackpad settings (input.rs)
            InputEvent::DeviceAdded { mut device } => {
                crate::input::configure_device(&mut device, &crate::input::InputPrefs::load(&crate::input::prefs_path()))
            }
            InputEvent::DeviceRemoved { .. } => {}
            event => state.process_input_event(event),
        }
    })?;

    // VT switches pause and resume the GPU and input
    let b = backend.clone();
    event_loop.handle().insert_source(notifier, move |event, _, state| match event {
        SessionEvent::PauseSession => {
            libinput.suspend();
            b.borrow_mut().paused = true;
        }
        SessionEvent::ActivateSession => {
            if let Err(e) = libinput.resume() {
                eprintln!("bifrost-wm: libinput resume: {e:?}");
            }
            let mut d = b.borrow_mut();
            d.paused = false;
            d.pending = false;
            state.dirty = true;
            let _ = d.compositor.reset_state();
            if let Some(v) = d.vulkan.as_mut() {
                v.reset();
            }
        }
    })?;

    // first frame, then a frame only when something changed (`WmState::dirty`) or the boot/shutdown fade runs; the timer
    // is the retry for a dirty flag set while a flip was pending, and costs one flag check per tick on a quiet desktop
    let b = backend.clone();
    event_loop.handle().insert_source(Timer::immediate(), move |_, _, state| {
        b.borrow_mut().tick(state);
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
        if d.vulkan.is_some() {
            let time = vblank_time.get();
            if let Some(v) = d.vulkan.as_mut() {
                v.on_vblank(time);
            }
        } else if let Err(e) = d.compositor.frame_submitted() {
            eprintln!("bifrost-wm: frame submitted: {e}");
        }
        d.pending = false;
        d.tick(state);
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

fn open_gpu(
    session: &mut LibSeatSession,
    path: &Path,
    state: &mut WmState,
    handle: LoopHandle<'static, WmState>,
    vblank_time: Rc<Cell<Option<Duration>>>,
) -> Result<Drm, Box<dyn Error>> {
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
    // every mode the connector lists is announced, so a mode request can name any of them
    let modes = conn.modes().to_vec();
    for m in &modes {
        output.add_mode(WlMode::from(*m));
    }
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

    handle.insert_source(notifier, move |event, meta, state: &mut WmState| match event {
        DrmEvent::VBlank(_) => {
            vblank_time.set(meta.as_ref().map(|m| match m.time {
                smithay::backend::drm::DrmEventTime::Monotonic(d) => d,
                smithay::backend::drm::DrmEventTime::Realtime(t) => t.duration_since(std::time::UNIX_EPOCH).unwrap_or_default(),
            }));
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
    Ok(Drm { renderer, compositor, output, modes, pointer, pending: false, boot, glass, vulkan: None, shutdown: Shutdown::default(), paused: false })
}

/// The shutdown fade (`crate::boot::shutdown_alpha`): once `SHUTDOWN_REQUEST` exists the boot frame comes back over the
/// desktop and then stays until power-off. Built on the first frame of a shutdown only.
#[derive(Default)]
struct Shutdown {
    splash: Option<MemoryRenderBuffer>,
    /// When the request was first seen.
    since: Option<std::time::Instant>,
    /// `SHUTDOWN_FADED` is written once.
    faded_marked: bool,
    /// When the request file was last looked for.
    checked: Option<std::time::Instant>,
}

impl Drm {
    /// Switches the scanout of output `name` to the connector mode `want` (size and refresh in millihertz). The mode is
    /// one of the connector's own, matched exactly; smithay's DRM compositor takes the atomic or the legacy commit the
    /// driver supports and resizes the swapchain. The vblank that follows the commit shows the new mode.
    fn set_mode(&mut self, name: &str, want: ModeInfo) -> bool {
        if name != self.output.name().as_str() {
            return false;
        }
        if self.vulkan.is_some() {
            eprintln!("bifrost-wm: mode change refused: the Vulkan scanout keeps its swapchain");
            return false;
        }
        let wanted = WlMode { size: (want.w, want.h).into(), refresh: want.refresh };
        let Some(mode) = self.modes.iter().copied().find(|m| WlMode::from(*m) == wanted) else {
            eprintln!("bifrost-wm: mode change refused: {name} has no {}x{}@{}", want.w, want.h, want.refresh);
            return false;
        };
        match self.compositor.use_mode(mode) {
            Ok(()) => {
                eprintln!("bifrost-wm: modeset {name} {}x{}@{}", want.w, want.h, want.refresh);
                true
            }
            Err(e) => {
                eprintln!("bifrost-wm: modeset {name} failed: {e}");
                false
            }
        }
    }

    /// The boot frame over the desktop for the shutdown fade, with its opacity; None before a request.
    fn shutdown_overlay(&mut self) -> Option<(MemoryRenderBuffer, f32)> {
        self.poll_shutdown();
        let sd = &mut self.shutdown;
        sd.since?;
        let since = sd.since.map(|t| t.elapsed().as_secs_f64() * 1000.0);
        if sd.splash.is_none() {
            let mode = self.output.current_mode()?;
            sd.splash = Some(boot_frame_buffer(mode.size.w, mode.size.h));
        }
        if crate::boot::shutdown_faded(since) && !sd.faded_marked {
            sd.faded_marked = true;
            // the account's runtime directory exists for the whole session; the marker is written once
            let _ = std::fs::write(crate::boot::SHUTDOWN_FADED, b"1\n");
            crate::boot::mark("shutdown-faded");
        }
        Some((sd.splash.clone()?, crate::boot::shutdown_alpha(since)?))
    }

    /// Looks for the shutdown request at most every 250 ms (it was a `stat` per frame).
    fn poll_shutdown(&mut self) {
        let sd = &mut self.shutdown;
        if sd.since.is_some() || sd.checked.is_some_and(|t| t.elapsed() < Duration::from_millis(250)) {
            return;
        }
        sd.checked = Some(std::time::Instant::now());
        if std::path::Path::new(crate::boot::SHUTDOWN_REQUEST).exists() {
            sd.since = Some(std::time::Instant::now());
            crate::boot::mark("shutdown-start");
        }
    }

    /// Composes when the scene changed or a fade runs, and not while a flip is pending or the session is away.
    fn tick(&mut self, state: &mut WmState) {
        if !should_render(self.paused, self.pending) {
            return;
        }
        self.poll_shutdown();
        let fading = self.boot.is_some() || (self.shutdown.since.is_some() && !self.shutdown.faded_marked);
        if !state.dirty && !fading {
            return;
        }
        state.dirty = false;
        let gone = std::mem::take(&mut state.destroyed_buffers);
        if let Some(v) = self.vulkan.as_mut() {
            v.forget_buffers(&gone);
        }
        self.render(state);
    }

    fn render(&mut self, state: &mut WmState) {
        let output = self.output.clone();
        if let Some(v) = self.vulkan.as_mut() {
            match v.frame(self.compositor.surface(), state, &output) {
                Ok(queued) => self.pending = queued,
                Err(e) => {
                    eprintln!("bifrost-wm: vulkan frame: {e}");
                    self.pending = false;
                }
            }
            state.send_frames(&output);
            return;
        }
        let mut top: Vec<SceneElement> = Vec::new();
        // front to back: the boot frame over everything while it fades, else the pointer on top
        let splash = self.boot.as_mut().and_then(|b| b.alpha().map(|a| (b.splash.clone(), a)));
        if splash.is_none() {
            self.boot = None;
        }
        // a shutdown replaces the boot's fade-in: the boot frame returns over the desktop and stays
        let overlay = self.shutdown_overlay().or(splash);
        if let Some((buffer, alpha)) = overlay {
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

/// Whether a frame may be composed now: not while the session is switched away and not with a flip in flight.
pub(crate) fn should_render(paused: bool, pending: bool) -> bool { !paused && !pending }

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
    fn no_frame_while_paused_or_pending() {
        assert!(should_render(false, false));
        assert!(!should_render(true, false));
        assert!(!should_render(false, true));
    }

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
