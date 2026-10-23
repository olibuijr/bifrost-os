//! The DRM/KMS backend: the window server on real hardware (and in the VM). libseat gives access to the GPU and
//! input devices, libinput reads input, and Smithay's DRM compositor presents on the first connected display at its
//! native size and fastest refresh.

use std::{
    cell::Cell,
    error::Error,
    os::fd::{AsRawFd, FromRawFd, OwnedFd},
    path::Path,
    rc::Rc,
    sync::atomic::{AtomicI32, Ordering},
    time::{Duration, Instant},
};

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
        calloop::{generic::Generic, timer::{TimeoutAction, Timer}, EventLoop, Interest, LoopHandle, Mode as IoMode, PostAction},
        drm::control::{connector, crtc, Device as ControlDevice, Mode as DrmMode, ModeTypeFlags},
        input::Libinput,
        rustix::fs::OFlags,
        wayland_server::Display,
    },
    utils::{DeviceFd, Transform},
};

use crate::{
    scene::SceneElement,
    session_protocols::{CaptureSpec, ModeInfo},
    state::WmState,
    white_point::{self, Mode as WhiteMode, MultiplyElement, WHITE},
    window_capture::WindowCaptureSpec,
};

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
    /// The CRTC's gamma ramp, when the display has one: Night Shift in `white_point::Mode::Hardware`.
    gamma: Option<Gamma>,
    /// Night Shift's final pass in `white_point::Mode::Composited` (kept so its id and commit last between frames).
    night_pass: MultiplyElement,
}

/// The CRTC's gamma ramp, programmed with the output's white point (`white_point.rs`): the display controller does the
/// multiply on scanout, so Night Shift costs no GPU work and tints whatever is shown, planes and cursor included. The
/// legacy gamma ioctl is used; on an atomic driver the kernel turns it into the GAMMA_LUT blob of the CRTC, and smithay's
/// commits never touch that property, so the ramp stays until it is set again.
struct Gamma {
    fd: DrmDeviceFd,
    crtc: crtc::Handle,
    /// The ramp the display had when the session started (its calibration), per channel; put back on exit.
    base: [Vec<u16>; 3],
    /// The white point the ramp shows; None when it must be programmed again (the session came back from a VT switch,
    /// where another DRM master may have changed it).
    shown: Option<[f32; 3]>,
    /// When the ramp was last set, for `white_point::ramp_due`.
    at: Option<Instant>,
}

impl Gamma {
    /// The ramp of `crtc`, or None when it has none (gamma size 0) or `BIFROST_WM_NIGHT=composited` asks for the pass.
    fn probe(fd: &DrmDeviceFd, crtc: crtc::Handle) -> Option<Gamma> {
        if std::env::var("BIFROST_WM_NIGHT").is_ok_and(|v| v == "composited") {
            return None;
        }
        let len = fd.get_crtc(crtc).ok()?.gamma_length() as usize;
        if len < 2 {
            return None;
        }
        let (mut r, mut g, mut b) = (vec![0u16; len], vec![0u16; len], vec![0u16; len]);
        let read = fd.get_gamma(crtc, &mut r, &mut g, &mut b).is_ok();
        let calibrated = read
            && [&r, &g, &b].iter().all(|c| white_point::usable_base(c, len))
            && !white_point::leftover_tint(&r, &g, &b);
        let base = if calibrated { [r, g, b] } else { [white_point::linear(len), white_point::linear(len), white_point::linear(len)] };
        eprintln!("bifrost-wm: Night Shift on the CRTC gamma ramp ({len} entries{})", if calibrated { "" } else { ", identity base" });
        // an identity base replaces whatever ramp was read, so it is programmed on the first step
        Some(Gamma { fd: fd.clone(), crtc, base, shown: if calibrated { Some(WHITE) } else { None }, at: None })
    }

    /// Programs the ramp for white point `f`.
    fn show(&mut self, f: [f32; 3]) -> std::io::Result<()> {
        let [r, g, b] = [0, 1, 2].map(|i| white_point::ramp(&self.base[i], f[i]));
        self.fd.set_gamma(self.crtc, &r, &g, &b)?;
        self.shown = Some(f);
        self.at = Some(Instant::now());
        Ok(())
    }

    /// Puts the display's own ramp back (exit). Needs the DRM master, so it cannot run once a VT switch took it.
    fn restore(&mut self) {
        if self.shown == Some(WHITE) {
            return;
        }
        match self.fd.set_gamma(self.crtc, &self.base[0], &self.base[1], &self.base[2]) {
            Ok(()) => self.shown = Some(WHITE),
            Err(e) => eprintln!("bifrost-wm: gamma restore: {e}"),
        }
    }
}

/// The signals that end the session; the window server puts the display's gamma ramp back before it goes.
const EXIT_SIGNALS: [libc::c_int; 3] = [libc::SIGTERM, libc::SIGINT, libc::SIGHUP];
/// The write end of the exit pipe (`exit_signals`), for the signal handler.
static EXIT_PIPE: AtomicI32 = AtomicI32::new(-1);

extern "C" fn on_exit_signal(sig: libc::c_int) {
    let fd = EXIT_PIPE.load(Ordering::Relaxed);
    if fd >= 0 {
        let b = sig as u8;
        // SAFETY: write(2) is async-signal-safe; the byte lives on this stack frame
        unsafe { libc::write(fd, (&b as *const u8).cast(), 1) };
    }
}

/// A pipe that receives the number of each exit signal, so the event loop can restore the gamma ramp and then let the
/// signal end the process as before (the handler is reset to the default and raised again). Handlers do not survive
/// exec, so the clients the window server starts are not affected.
fn exit_signals() -> Option<OwnedFd> {
    let mut fds = [-1; 2];
    // SAFETY: plain syscalls on a local array and zeroed sigaction structs
    unsafe {
        if libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC | libc::O_NONBLOCK) != 0 {
            return None;
        }
        EXIT_PIPE.store(fds[1], Ordering::Relaxed);
        for sig in EXIT_SIGNALS {
            let mut sa: libc::sigaction = std::mem::zeroed();
            sa.sa_sigaction = on_exit_signal as extern "C" fn(libc::c_int) as libc::sighandler_t;
            libc::sigemptyset(&mut sa.sa_mask);
            sa.sa_flags = libc::SA_RESTART;
            libc::sigaction(sig, &sa, std::ptr::null_mut());
        }
        Some(OwnedFd::from_raw_fd(fds[0]))
    }
}

/// How the white point is applied with this backend: the gamma ramp when the CRTC has one, else the GLES pass; the
/// Vulkan scanout has no pass, so without a ramp the shell keeps tinting its own surfaces (`Mode::None`).
pub(crate) fn white_point_mode(has_ramp: bool, vulkan: bool) -> WhiteMode {
    match (has_ramp, vulkan) {
        (true, _) => WhiteMode::Hardware,
        (false, false) => WhiteMode::Composited,
        (false, true) => WhiteMode::None,
    }
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
    let (mut session, notifier) = LibSeatSession::new().map_err(|e| format!("libseat: {e}"))?;
    let seat = session.seat();
    let gpu = primary_gpu(&seat)?.or_else(|| all_gpus(&seat).ok()?.into_iter().next()).ok_or("no GPU")?;
    eprintln!("bifrost-wm: GPU {}", gpu.display());
    // before the Wayland socket exists: a client that connected now would get no answer until a display is there
    wait_for_display(&mut session, &gpu);
    let display: Display<WmState> = Display::new()?;
    let mut state = WmState::new(&mut event_loop, display, socket);
    // the page flip's vblank time, written by the DRM event source and read by the vblank callback below
    let vblank_time: Rc<Cell<Option<Duration>>> = Rc::new(Cell::new(None));
    let mut drm = open_gpu(&mut session, &gpu, &mut state, event_loop.handle(), vblank_time.clone())?;
    // the Vulkan scanout on the same GPU; any failure keeps the GLES scanout that open_gpu just set up
    // the Vulkan scanout draws unrotated; a rotated output stays on GLES scanout, which follows the output transform
    let rotated = drm.output.current_transform() != Transform::Normal;
    if rotated && crate::render_server::selected() {
        eprintln!("bifrost-wm: vulkan scanout does not rotate, GLES");
    }
    if !rotated && crate::render_server::selected() {
        let size = drm.output.current_mode().map(|m| (m.size.w, m.size.h)).unwrap_or((0, 0));
        let refresh = drm.output.current_mode().map(|m| m.refresh).unwrap_or(0);
        match crate::drm_vulkan::start(&gpu, drm.compositor.surface(), size, refresh) {
            Ok(v) => drm.vulkan = Some(v),
            Err(e) => eprintln!("bifrost-wm: vulkan scanout unavailable, GLES: {e}"),
        }
    }
    // Night Shift (`white_point.rs`): decided before the first client binds, which hears the mode at once
    state.white_point.mode = white_point_mode(drm.gamma.is_some(), drm.vulkan.is_some());
    eprintln!("bifrost-wm: Night Shift {:?}", state.white_point.mode);
    // the shell's output mode requests (zwlr output management, `apply_outputs`) reach the scanout through this hook
    let b = Rc::new(std::cell::RefCell::new(drm));
    let hook = b.clone();
    state.session.hooks.set_mode = Some(Box::new(move |name: &str, mode: ModeInfo| hook.borrow_mut().set_mode(name, mode)));
    let hook = b.clone();
    state.session.hooks.set_transform = Some(Box::new(move |name: &str, t: Transform| hook.borrow_mut().set_transform(name, t)));
    let hook = b.clone();
    state.session.hooks.capture = Some(Box::new(move |state, spec| hook.borrow_mut().capture(state, spec)));
    let hook = b.clone();
    state.session.hooks.capture_window = Some(Box::new(move |state, spec| hook.borrow_mut().capture_window(state, spec)));
    let backend = b;

    // input
    let mut libinput = Libinput::new_with_udev::<LibinputSessionInterface<LibSeatSession>>(session.clone().into());
    libinput.udev_assign_seat(&seat).map_err(|_| "libinput: cannot assign seat")?;
    event_loop.handle().insert_source(LibinputInputBackend::new(libinput.clone()), |event, _, state| {
        match event {
            // a new device takes the shell's pointer and trackpad settings (input.rs)
            InputEvent::DeviceAdded { mut device } => {
                crate::input::configure_device(&mut device, &crate::input::InputPrefs::load(&crate::input::prefs_path()));
                // the seat advertises touch while a touchscreen is present (touch.rs)
                if device.has_capability(smithay::reexports::input::DeviceCapability::Touch) {
                    state.touch_device_added();
                }
            }
            InputEvent::DeviceRemoved { device } => {
                if device.has_capability(smithay::reexports::input::DeviceCapability::Touch) {
                    state.touch_device_removed();
                }
            }
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
            // another DRM master may have set its own ramp meanwhile: the white point is programmed again
            if let Some(g) = d.gamma.as_mut() {
                g.shown = None;
            }
            state.dirty = true;
            let _ = d.compositor.reset_state();
            if let Some(v) = d.vulkan.as_mut() {
                v.reset();
            }
        }
    })?;

    // the end of the session puts the display's own gamma ramp back (a VT switch cannot: the master is gone by then)
    if let Some(pipe) = exit_signals() {
        let b = backend.clone();
        event_loop.handle().insert_source(Generic::new(pipe, Interest::READ, IoMode::Level), move |_, pipe, _| {
            let mut sig = [0u8; 1];
            // SAFETY: a one-byte read into a local buffer from the pipe the source owns
            if unsafe { libc::read(pipe.as_raw_fd(), sig.as_mut_ptr().cast(), 1) } == 1 {
                if let Some(g) = b.borrow_mut().gamma.as_mut() {
                    g.restore();
                }
                // SAFETY: back to the default action, then the same signal ends the process as it did before
                unsafe {
                    libc::signal(sig[0] as libc::c_int, libc::SIG_DFL);
                    libc::raise(sig[0] as libc::c_int);
                }
            }
            Ok(PostAction::Continue)
        })?;
    }

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
    // linux-dmabuf v4 with default feedback naming the render node: Mesa's Wayland EGL platform learns the device
    // from `main_device` (a v3 global gives it none, so a client's EGL fails with "failed to get driver name for fd -1")
    let _dmabuf_global = match crate::sync::render_node_dev(&gpu)
        .and_then(|dev| crate::sync::build_feedback(&crate::sync::plan_from_formats(dev, formats.clone())).map(|f| (dev, f)))
    {
        Ok((dev, feedback)) => {
            eprintln!("bifrost-wm: dmabuf feedback v4, main device {}:{}", dev_major(dev), dev_minor(dev));
            state.dmabuf_state.create_global_with_default_feedback::<WmState>(&state.dh, &feedback)
        }
        Err(e) => {
            eprintln!("bifrost-wm: dmabuf feedback unavailable ({e}), linux-dmabuf v3");
            state.dmabuf_state.create_global::<WmState>(&state.dh, formats)
        }
    };
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
    let ran = event_loop.run(None, &mut state, |state| {
        state.space.refresh();
        state.popups.cleanup();
        let _ = state.dh.flush_clients();
    });
    if let Some(g) = backend.borrow_mut().gamma.as_mut() {
        g.restore();
    }
    ran?;
    Ok(())
}

/// Blocks until a display is connected to `gpu`, checking once a second with the same forced connector probe
/// `open_gpu` uses (sysfs `status` is a cached value: a monitor in power save reads `disconnected` there while a probe
/// finds it). A PC whose monitor is off, asleep or on another input when the session starts then gets its desktop when
/// the monitor comes back, instead of the session exiting with "no connected display". When the card cannot be opened
/// or probed it returns at once and `open_gpu` decides. A display plugged in while running is not handled yet. While
/// it waits, [`awaiting_display_marker`] exists, so the session keeps waiting for the socket.
fn wait_for_display(session: &mut LibSeatSession, gpu: &Path) {
    let marker = awaiting_display_marker();
    let mut logged = false;
    loop {
        let Ok(fd) = session.open(gpu, OFlags::RDWR | OFlags::CLOEXEC | OFlags::NOCTTY | OFlags::NONBLOCK) else { break };
        let card = Card(fd);
        let found = card.connector_states().map(|states| has_display(&states));
        let _ = session.close(card.0);
        match found {
            Some(false) => {}
            // connected, or the card could not be probed: open_gpu decides
            _ => break,
        }
        if !logged {
            eprintln!("bifrost-wm: no connected display on {}, waiting for one", gpu.display());
            let _ = std::fs::write(&marker, b"");
            logged = true;
        }
        std::thread::sleep(Duration::from_secs(1));
    }
    if logged {
        eprintln!("bifrost-wm: display connected on {}", gpu.display());
        let _ = std::fs::remove_file(&marker);
    }
}

/// Where the window server marks that it waits for a display: its runtime directory (it runs as the session account,
/// so `/run/bifrost` itself is not writable). `packaging/arch/wayland/session` reads `$runtime/wm-awaiting-display`.
fn awaiting_display_marker() -> std::path::PathBuf {
    std::path::PathBuf::from(std::env::var_os("XDG_RUNTIME_DIR").unwrap_or_else(|| "/run/bifrost/user".into())).join("wm-awaiting-display")
}

/// A DRM card opened only to probe its connectors.
struct Card(std::os::fd::OwnedFd);

impl std::os::fd::AsFd for Card {
    fn as_fd(&self) -> std::os::fd::BorrowedFd<'_> {
        self.0.as_fd()
    }
}
impl smithay::reexports::drm::Device for Card {}
impl ControlDevice for Card {}

impl Card {
    /// Each connector's state and mode count after a forced probe; `None` when the card's resources cannot be read.
    fn connector_states(&self) -> Option<Vec<(connector::State, usize)>> {
        let res = self.resource_handles().ok()?;
        Some(res.connectors().iter().filter_map(|c| self.get_connector(*c, true).ok()).map(|c| (c.state(), c.modes().len())).collect())
    }
}

/// Whether a probed card has a display: a connector that is connected (or reports an unknown state, as some drivers
/// do) and offers modes; the same rule `open_gpu` applies, widened to unknown.
fn has_display(states: &[(connector::State, usize)]) -> bool {
    states.iter().any(|(st, modes)| *modes > 0 && matches!(st, connector::State::Connected | connector::State::Unknown))
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

    let gamma = Gamma::probe(&fd, crtc);
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
    // the panel's rotation (`transform::load`): the output's logical size, the DRM compositor's render transform, the
    // layer map and the clients' wl_output all follow it
    let transform = crate::transform::load();
    if transform != Transform::Normal {
        let (lw, lh) = crate::transform::logical_size((w as i32, h as i32), transform);
        eprintln!("bifrost-wm: rotation {transform:?}: {w}x{h} panel shows a {lw}x{lh} desktop");
    }
    output.change_current_state(Some(wl_mode), Some(transform), None, Some((0, 0).into()));
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
    // the boot frame is drawn in desktop coordinates, so it takes the rotated size
    let (lw, lh) = crate::transform::logical_size((w as i32, h as i32), transform);
    let boot = std::env::var_os("BIFROST_BOOT_FADE").map(|_| Boot::new(lw, lh));
    let mut renderer = renderer;
    let glass = crate::glass::Glass::new(&mut renderer)?;
    Ok(Drm {
        renderer,
        compositor,
        output,
        modes,
        pointer,
        pending: false,
        boot,
        glass,
        vulkan: None,
        shutdown: Shutdown::default(),
        paused: false,
        gamma,
        night_pass: MultiplyElement::default(),
    })
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

    /// Accepts a live change of output `name` to transform `t` (`live_rotation::rotate_output`). The DRM compositor reads
    /// the output's transform for every frame, so nothing is reprogrammed here. The Vulkan scanout draws unrotated, so
    /// it cannot follow a turn away from `Normal`; the change is refused while it runs.
    fn set_transform(&mut self, name: &str, t: Transform) -> bool {
        if name != self.output.name().as_str() {
            return false;
        }
        if self.vulkan.is_some() && t != Transform::Normal {
            eprintln!("bifrost-wm: rotation refused: the Vulkan scanout draws unrotated");
            return false;
        }
        true
    }

    /// The boot frame over the desktop for the shutdown fade, with its opacity; None before a request.
    fn shutdown_overlay(&mut self) -> Option<(MemoryRenderBuffer, f32)> {
        self.poll_shutdown();
        let sd = &mut self.shutdown;
        sd.since?;
        let since = sd.since.map(|t| t.elapsed().as_secs_f64() * 1000.0);
        if sd.splash.is_none() {
            let (w, h) = crate::transform::output_size(&self.output)?;
            sd.splash = Some(boot_frame_buffer(w, h));
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
        self.step_white_point(state);
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

    /// Steps Night Shift's fade (`WmState::white_point`) and applies it: the gamma ramp (throttled to
    /// `white_point::RAMP_STEP`, the last step always lands), or a new frame for the compositor pass. A ramp the driver
    /// refuses falls back to the pass, and the shell is told the new mode. Nothing while the session is away.
    fn step_white_point(&mut self, state: &mut WmState) {
        if self.paused {
            return;
        }
        let now = Instant::now();
        let changed = state.white_point.advance(now);
        match state.white_point.mode {
            WhiteMode::Hardware => {
                let Some(g) = self.gamma.as_mut() else { return };
                let want = state.white_point.current();
                if g.shown == Some(want) || !white_point::ramp_due(g.at, now, want == state.white_point.target()) {
                    return;
                }
                if let Err(e) = g.show(want) {
                    self.gamma = None;
                    let mode = white_point_mode(false, self.vulkan.is_some());
                    eprintln!("bifrost-wm: gamma ramp refused ({e}), Night Shift {mode:?}");
                    state.white_point.mode = mode;
                    state.frame_reports.send_white_point_mode(mode);
                    state.dirty = true;
                }
            }
            WhiteMode::Composited => {
                if changed {
                    state.dirty = true;
                }
            }
            WhiteMode::None => {}
        }
    }

    /// A window capture (`SessionHooks::capture_window`): one window alone at `spec.size`, RGBA over a transparent
    /// ground. The Vulkan scanout has no GLES renderer to draw it with, so the request fails there (`unsupported`).
    fn capture_window(&mut self, state: &WmState, spec: &WindowCaptureSpec) -> Option<Vec<u8>> {
        if self.vulkan.is_some() {
            return None;
        }
        let window = state.window_any(spec.id)?;
        match crate::capture::capture_window(&mut self.renderer, &self.glass, state, &window, spec.size) {
            Ok(px) => Some(px),
            Err(e) => {
                eprintln!("bifrost-wm: window capture: {e}");
                None
            }
        }
    }

    /// A capture (`SessionHooks::capture`): the scene as RGBA rows, with the pointer when asked. Not served on the Vulkan
    /// scanout (its target is not readable here), which leaves the copy to fail.
    fn capture(&mut self, state: &WmState, spec: &CaptureSpec) -> Option<Vec<u8>> {
        if spec.output != self.output.name().as_str() || self.vulkan.is_some() {
            return None;
        }
        let mut top = Vec::new();
        if spec.cursor && state.pointer_cursor_shown() {
            let pos = state.pointer_location();
            if let Ok(e) = MemoryRenderBufferRenderElement::from_buffer(&mut self.renderer, (pos.x, pos.y), &self.pointer, None, None, None, Kind::Cursor) {
                top.push(SceneElement::Memory(e));
            }
        }
        match crate::capture::capture_scene(&mut self.renderer, &mut self.glass, state, &self.output, top, CLEAR, spec) {
            Ok(px) => Some(px),
            Err(e) => {
                eprintln!("bifrost-wm: capture: {e}");
                None
            }
        }
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
        } else if state.pointer_cursor_shown() {
            let pos = state.pointer_location();
            if let Ok(e) = MemoryRenderBufferRenderElement::from_buffer(&mut self.renderer, (pos.x, pos.y), &self.pointer, None, None, None, Kind::Cursor) {
                top.push(SceneElement::Memory(e));
            }
        }
        // Night Shift's pass, frontmost: everything under it (the pointer too) is multiplied by the white point. Captures
        // build their own scene without it, as they never see the gamma ramp either.
        if state.white_point.mode == WhiteMode::Composited {
            if let Some(size) = crate::transform::output_size(&output) {
                self.night_pass.update(size, state.white_point.current());
            }
            if !self.night_pass.is_identity() {
                top.insert(0, SceneElement::WhitePoint(self.night_pass.clone()));
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
    fn the_session_waits_until_a_probed_connector_has_a_display() {
        use connector::State::{Connected, Disconnected, Unknown};
        assert!(!has_display(&[(Disconnected, 0), (Disconnected, 0)]), "a monitor that is off: keep waiting");
        assert!(!has_display(&[(Connected, 0)]), "connected without modes is no display yet");
        assert!(has_display(&[(Disconnected, 0), (Connected, 3)]));
        assert!(has_display(&[(Unknown, 1)]), "a driver that cannot tell: let open_gpu decide");
    }

    #[test]
    fn night_shift_takes_the_ramp_first_then_the_gles_pass() {
        assert_eq!(white_point_mode(true, false), WhiteMode::Hardware, "the gamma ramp costs no GPU work");
        assert_eq!(white_point_mode(true, true), WhiteMode::Hardware, "the ramp serves the Vulkan scanout too");
        assert_eq!(white_point_mode(false, false), WhiteMode::Composited, "no ramp: the GLES final pass");
        assert_eq!(white_point_mode(false, true), WhiteMode::None, "Vulkan has no pass yet: the shell tints itself");
    }

    #[test]
    fn the_exit_signals_are_the_ones_that_end_a_session() {
        assert_eq!(EXIT_SIGNALS, [libc::SIGTERM, libc::SIGINT, libc::SIGHUP]);
        assert!(EXIT_SIGNALS.iter().all(|&s| (s as u8) as libc::c_int == s), "each fits the pipe's one byte");
    }

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

/// The major number of a `dev_t` (glibc `gnu_dev_major`).
fn dev_major(dev: u64) -> u32 {
    (((dev >> 32) & 0xffff_f000) | ((dev >> 8) & 0x0000_0fff)) as u32
}

/// The minor number of a `dev_t` (glibc `gnu_dev_minor`).
fn dev_minor(dev: u64) -> u32 {
    (((dev >> 12) & 0xffff_ff00) | (dev & 0x0000_00ff)) as u32
}
