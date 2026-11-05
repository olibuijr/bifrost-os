//! The compositor state: Smithay's protocol states, the window space, and the `wm` rules that place and move
//! windows in it.

use std::{
    ffi::OsString,
    os::{
        fd::{AsRawFd, FromRawFd, OwnedFd},
        unix::net::UnixStream,
    },
    sync::Arc,
    time::Instant,
};

use smithay::{
    desktop::{layer_map_for_output, PopupManager, Space, Window, WindowSurfaceType},
    wayland::seat::WaylandFocus,
    input::{Seat, SeatState},
    output::Output,
    reexports::{
        calloop::{generic::Generic, EventLoop, Interest, LoopHandle, LoopSignal, Mode, PostAction},
        wayland_server::{
            backend::{ClientData, ClientId, DisconnectReason},
            protocol::wl_surface::WlSurface,
            Display, DisplayHandle,
        },
    },
    utils::{Logical, Point},
    wayland::{
        compositor::{CompositorClientState, CompositorState},
        output::OutputManagerState,
        presentation::PresentationState,
        selection::data_device::DataDeviceState,
        shell::{wlr_layer::WlrLayerShellState, xdg::XdgShellState},
        shm::ShmState,
        socket::ListeningSocketSource,
    },
};

use crate::wm::{Level, Rect, WindowId, Wm};

/// Capture the Linux Unix-socket peer uid before registering the client with Wayland.
/// Global visibility filters run inside Wayland backend dispatch and must not call back into its credential lock.
pub(crate) fn peer_uid(stream: &UnixStream) -> Option<u32> {
    smithay::reexports::rustix::net::sockopt::socket_peercred(stream).ok().map(|credentials| credentials.uid.as_raw())
}

/// Capture the Linux Unix-socket peer pid on accept (`SO_PEERCRED`), in this process's pid namespace. `None` when the
/// credentials cannot be read.
pub(crate) fn peer_pid(stream: &UnixStream) -> Option<u32> {
    smithay::reexports::rustix::net::sockopt::socket_peercred(stream)
        .ok()
        .and_then(|credentials| u32::try_from(credentials.pid.as_raw_nonzero().get()).ok())
}

/// Whether two namespace links (`readlink /proc/<pid>/ns/mnt`) name the same namespace. Anything unreadable is not the
/// same: the check fails closed.
pub(crate) fn same_namespace(own: Option<&std::path::Path>, peer: Option<&std::path::Path>) -> bool {
    matches!((own, peer), (Some(a), Some(b)) if a == b)
}

/// Whether process `pid` shares this process's mount namespace. A sandboxed native app (`bwrap`, `src/bare/app_policy.rs`)
/// always has a mount namespace of its own, a program the user started in their session does not. An unknown pid, a pid
/// of 0 (the peer lives in another pid namespace) or an unreadable link is "not shared".
pub(crate) fn shares_mount_ns(pid: Option<u32>) -> bool {
    let Some(pid) = pid.filter(|pid| *pid != 0) else { return false };
    let own = std::fs::read_link("/proc/self/ns/mnt").ok();
    let peer = std::fs::read_link(format!("/proc/{pid}/ns/mnt")).ok();
    same_namespace(own.as_deref(), peer.as_deref())
}

/// `SO_PEERPIDFD` (Linux 6.5): a pidfd for the process that connected, taken by the kernel at connect time. It names that
/// very process even after its pid was recycled, which `SO_PEERCRED`'s number cannot.
const SO_PEERPIDFD: libc::c_int = 77;

/// The pidfd of the process on the other end of `stream`, or `None` (kernel older than 6.5, or no credentials).
fn peer_pidfd(stream: &UnixStream) -> Option<OwnedFd> {
    let mut fd: libc::c_int = -1;
    let mut len = std::mem::size_of::<libc::c_int>() as libc::socklen_t;
    // SAFETY: `fd` and `len` are valid for the call; on success the kernel stored a new descriptor (close-on-exec) that
    // nothing else owns, which `OwnedFd` takes over.
    let rc = unsafe {
        libc::getsockopt(stream.as_raw_fd(), libc::SOL_SOCKET, SO_PEERPIDFD, (&mut fd as *mut libc::c_int).cast(), &mut len)
    };
    // SAFETY: see above; `fd` is only used when the call succeeded.
    (rc == 0 && fd >= 0).then(|| unsafe { OwnedFd::from_raw_fd(fd) })
}

/// The pid in a `/proc/self/fdinfo/<pidfd>` text (`Pid:` line, in this pid namespace). `-1` (the process is gone) and
/// `0` (it lives in another pid namespace) give `None`.
pub(crate) fn parse_fdinfo_pid(text: &str) -> Option<u32> {
    let value = text.lines().find_map(|line| line.strip_prefix("Pid:"))?;
    value.trim().parse::<i64>().ok().filter(|pid| *pid > 0).and_then(|pid| u32::try_from(pid).ok())
}

/// Whether the process behind `pidfd` has terminated (a pidfd becomes readable then).
fn pidfd_exited(pidfd: &OwnedFd) -> bool {
    let mut poll = libc::pollfd { fd: pidfd.as_raw_fd(), events: libc::POLLIN, revents: 0 };
    // SAFETY: one valid pollfd; a zero timeout returns at once.
    let ready = unsafe { libc::poll(&mut poll, 1, 0) };
    ready != 0
}

/// The mount-namespace check made through a pidfd: the pid is read from the pidfd itself (so it is the connecting
/// process's, never a recycled number), and the process must still be running after its namespace link was read (so the
/// link was its own). A connector that already exited fails closed.
fn pidfd_shares_mount_ns(pidfd: &OwnedFd) -> bool {
    if pidfd_exited(pidfd) {
        return false;
    }
    let pid = std::fs::read_to_string(format!("/proc/self/fdinfo/{}", pidfd.as_raw_fd())).ok().and_then(|text| parse_fdinfo_pid(&text));
    shares_mount_ns(pid) && !pidfd_exited(pidfd)
}

/// Whether the peer of `stream` shares this process's mount namespace, as of accept. `pid` is the `SO_PEERCRED` pid and
/// `uid` its uid. Through `SO_PEERPIDFD` where the kernel has it (the answer is about the process that connected);
/// otherwise through the pid, which must belong to the peer's uid (a recycled pid of another account's process is
/// refused; a recycled pid within the account on a kernel before 6.5 remains a narrow race).
pub(crate) fn peer_shares_mount_ns(stream: &UnixStream, pid: Option<u32>, uid: Option<u32>) -> bool {
    if let Some(pidfd) = peer_pidfd(stream) {
        return pidfd_shares_mount_ns(&pidfd);
    }
    use std::os::unix::fs::MetadataExt;
    let owner = pid.filter(|pid| *pid != 0).and_then(|pid| std::fs::metadata(format!("/proc/{pid}")).ok()).map(|m| m.uid());
    owner.is_some() && owner == uid && shares_mount_ns(pid)
}

/// The `wm` id of a Smithay window, kept in the window's user data.
struct Id(WindowId);

pub struct WmState {
    pub start: Instant,
    /// The scene may have changed (a client request, input): the backend composes once and clears it (WM-4).
    pub dirty: bool,
    /// Importer keys of destroyed `wl_buffer`s, drained by the backend that owns the importer (WM-7).
    pub destroyed_buffers: Vec<u64>,
    pub socket_name: OsString,
    pub dh: DisplayHandle,
    pub space: Space<Window>,
    pub loop_signal: LoopSignal,
    pub wm: Wm,
    next_id: WindowId,
    /// The DRM backend's frame hook, run on every vblank.
    pub on_vblank: Option<Box<dyn FnMut(&mut WmState)>>,
    /// Toplevels waiting for their first buffer before `wm` places them.
    pub pending: Vec<Window>,
    /// The event loop's handle, for sources that wake the loop (acquire-point blockers in `sync`).
    pub loop_handle: LoopHandle<'static, WmState>,
    /// `wp_presentation` (clock: CLOCK_MONOTONIC, id 1).
    pub presentation: PresentationState,
    /// Presentation feedback of the frames in flight, presented at their vblank (`sync`).
    pub frame_feedback: crate::sync::FrameFeedback,
    /// `wp_linux_drm_syncobj_v1`, installed by the DRM backend when the kernel supports syncobj eventfds.
    #[cfg(feature = "drm")]
    pub drm_syncobj_state: Option<smithay::wayland::drm_syncobj::DrmSyncobjState>,

    /// `ext-background-effect-v1` (live glass); the renderer reads each surface's blur region.
    pub background_effect: smithay::reexports::wayland_server::backend::GlobalId,
    /// `bifrost_material_v1` (materials with a kind per region).
    pub bifrost_material: smithay::reexports::wayland_server::backend::GlobalId,
    /// `bifrost_window_level_v1` (the shell puts a toplevel on the floating level).
    pub bifrost_window_level: smithay::reexports::wayland_server::backend::GlobalId,
    /// The clients that bound it and the frame each was last told for its toplevels (`report_frames`).
    pub frame_reports: crate::window_level::FrameReports,
    /// Night Shift for every output (`set_white_point`): the shell's target, the fade towards it, and how the backend
    /// applies it (`white_point::Mode`, reported to the shell when it binds).
    pub white_point: crate::white_point::WhitePoint,
    /// Screen Zoom of the whole output (`set_zoom`, `zoom.rs`): the shell's factor as sent, 256 being 1.0.
    pub zoom: u32,
    /// Reduce Transparency and Increase Contrast as the shell last wrote them (`display_options.rs`).
    pub display: crate::display_options::DisplayPoll,
    /// `bifrost_window_capture_v1` (the shell captures one window's pixels; shell only, `window_capture.rs`).
    pub bifrost_window_capture: smithay::reexports::wayland_server::backend::GlobalId,
    /// Content serials and snapshots for window capture.
    pub window_capture: crate::window_capture::WindowCapture,
    /// Minimized toplevels: out of the space, but still held so they can be captured and shown again.
    pub hidden: Vec<Window>,
    /// Frames of toplevels destroyed while minimized, for the toplevel their client creates again (`wm::ParkedFrames`).
    pub parked: crate::wm::ParkedFrames<smithay::reexports::wayland_server::backend::ClientId>,
    /// The appearance materials follow: the shell's `dark=` line in its display file (`display_options::apply_scheme`),
    /// Dark until it is read unless `BIFROST_WM_APPEARANCE=light`.
    pub material_dark: bool,
    pub lock_state: smithay::wayland::session_lock::SessionLockManagerState,
    /// A session lock is held: only `lock_surface` is drawn and takes input.
    pub locked: bool,
    pub lock_surface: Option<smithay::wayland::session_lock::LockSurface>,
    /// The lock object of the client holding the lock; a second `lock` request is refused while it is alive. When the
    /// holder died the session stays locked and a new lock client may take over (and unlock), as the protocol requires.
    pub lock_holder: Option<smithay::reexports::wayland_protocols::ext::session_lock::v1::server::ext_session_lock_v1::ExtSessionLockV1>,
    pub idle_notifier_state: smithay::wayland::idle_notify::IdleNotifierState<WmState>,
    pub dmabuf_state: smithay::wayland::dmabuf::DmabufState,
    /// The backend's dmabuf import check (the DRM backend's renderer); without one no dmabuf global exists.
    pub dmabuf_import: Option<Box<dyn FnMut(&smithay::backend::allocator::dmabuf::Dmabuf) -> bool>>,
    pub compositor_state: CompositorState,
    pub xdg_shell_state: XdgShellState,
    pub layer_shell_state: WlrLayerShellState,
    pub shm_state: ShmState,
    pub output_manager_state: OutputManagerState,
    pub seat_state: SeatState<WmState>,
    pub data_device_state: DataDeviceState,
    pub popups: PopupManager,
    pub seat: Seat<WmState>,
    /// Pointer gestures that are running (begin and end paired per kind; see `input.rs`).
    pub gestures: crate::input::GestureGate,
    /// The pressed keys per keyboard device (libinput id), merged into the seat's one keyboard: a key that a
    /// keyboard reports on two interfaces is pressed once and released when the last of them lets go (`input.rs`).
    pub seat_keys: bifrost_ui::seat_keys::SeatKeys<String>,
    /// The fingers on the touchscreen and the one that drives an emulated pointer (`touch.rs`).
    pub contacts: crate::touch::Contacts,
    /// Touchscreens announced by the backend; the seat has the touch capability while there is one.
    pub touch_devices: crate::touch::TouchDevices,
    /// The last input was a touch contact, so the pointer image is hidden until real pointer motion (`touch.rs`).
    pub last_input_touch: bool,
    /// XWayland, its X11 window manager and the xwayland-shell global (`xwayland.rs`).
    #[cfg(feature = "xwayland")]
    pub xwayland: crate::xwayland::XWaylandState,
    /// Session protocols: foreign toplevels, screencopy, output management and power, cursor shapes, activation.
    pub session: crate::session_protocols::SessionState,
    /// Text input, input method and virtual keyboard (`text_input.rs`).
    pub text_input: crate::text_input::TextInputProtocols,
    /// Clipboard control for the shell and the user's tools (`data_control.rs`).
    pub data_control: crate::data_control::DataControl,
}

impl WmState {
    /// Listens on `socket` (or the next free `wayland-N`) and dispatches clients from `event_loop`.
    pub fn new(event_loop: &mut EventLoop<'static, WmState>, display: Display<WmState>, socket: Option<&str>) -> Self {
        let dh = display.handle();
        let session = crate::session_protocols::SessionState::new(&dh);
        let mut seat_state = SeatState::new();
        let mut seat: Seat<WmState> = seat_state.new_wl_seat(&dh, "seat0");
        let prefs = crate::input::InputPrefs::load(&crate::input::prefs_path());
        crate::input::add_keyboard(&mut seat, &prefs).expect("keyboard");
        seat.add_pointer();
        crate::input::register_protocols(&dh);

        let listening = match socket {
            Some(name) => ListeningSocketSource::with_name(name).expect("wayland socket"),
            None => ListeningSocketSource::new_auto().expect("wayland socket"),
        };
        let socket_name = listening.socket_name().to_os_string();
        let handle = event_loop.handle();
        let presentation = PresentationState::new::<WmState>(&dh, 1);
        handle
            .insert_source(listening, |stream, _, state: &mut WmState| {
                let pid = peer_pid(&stream);
                let uid = peer_uid(&stream);
                let data = Arc::new(ClientState {
                    peer_uid: uid,
                    peer_pid: pid,
                    shares_mount_ns: peer_shares_mount_ns(&stream, pid, uid),
                    ..ClientState::default()
                });
                if let Err(e) = state.dh.insert_client(stream, data) {
                    eprintln!("bifrost-wm: client refused: {e}");
                }
            })
            .expect("socket source");
        // the root-only socket the shell and its capture helpers connect through (the capture globals are theirs alone)
        crate::privileged::start(&handle);
        handle
            .insert_source(Generic::new(display, Interest::READ, Mode::Level), |_, display, state| {
                // SAFETY: the display is never dropped while the loop runs.
                // a failed dispatch is logged, not fatal: one broken client connection must not take the session down
                if let Err(e) = unsafe { display.get_mut().dispatch_clients(state) } {
                    eprintln!("bifrost-wm: dispatch clients: {e}");
                }
                // Reply to registry/sync requests as part of dispatch. The headless backend may
                // not render immediately (and rendering can block on first-use GPU setup), so
                // protocol progress must not depend on its frame timer.
                if let Err(e) = state.dh.flush_clients() {
                    eprintln!("bifrost-wm: flush clients: {e}");
                }
                state.dirty = true;
                Ok(PostAction::Continue)
            })
            .expect("display source");

        WmState {
            start: Instant::now(),
            dirty: true,
            destroyed_buffers: Vec::new(),
            socket_name,
            space: Space::default(),
            loop_signal: event_loop.get_signal(),
            wm: Wm::new(Rect::default()),
            next_id: 1,
            pending: Vec::new(),
            loop_handle: handle.clone(),
            presentation,
            frame_feedback: Default::default(),
            #[cfg(feature = "drm")]
            drm_syncobj_state: None,
            on_vblank: None,
            background_effect: crate::background_effect::init(&dh),
            bifrost_material: crate::bifrost_material::init(&dh),
            bifrost_window_level: crate::window_level::init(&dh),
            frame_reports: Default::default(),
            white_point: Default::default(),
            zoom: crate::zoom::ONE,
            display: Default::default(),
            bifrost_window_capture: crate::window_capture::init(&dh),
            window_capture: Default::default(),
            hidden: Vec::new(),
            parked: Default::default(),
            material_dark: std::env::var("BIFROST_WM_APPEARANCE").map(|v| v != "light").unwrap_or(true),
            lock_state: smithay::wayland::session_lock::SessionLockManagerState::new::<WmState, _>(&dh, crate::privileged::is_privileged),
            locked: false,
            lock_surface: None,
            lock_holder: None,
            idle_notifier_state: smithay::wayland::idle_notify::IdleNotifierState::new(&dh, event_loop.handle()),
            dmabuf_state: smithay::wayland::dmabuf::DmabufState::new(),
            dmabuf_import: None,
            compositor_state: CompositorState::new::<WmState>(&dh),
            xdg_shell_state: XdgShellState::new::<WmState>(&dh),
            layer_shell_state: WlrLayerShellState::new::<WmState>(&dh),
            shm_state: ShmState::new::<WmState>(&dh, vec![]),
            output_manager_state: OutputManagerState::new_with_xdg_output::<WmState>(&dh),
            data_device_state: DataDeviceState::new::<WmState>(&dh),
            popups: PopupManager::default(),
            seat_state,
            seat,
            gestures: Default::default(),
            seat_keys: Default::default(),
            contacts: Default::default(),
            touch_devices: Default::default(),
            last_input_touch: false,
            #[cfg(feature = "xwayland")]
            xwayland: crate::xwayland::XWaylandState::new(&dh),
            session,
            text_input: crate::text_input::TextInputProtocols::new(&dh),
            data_control: crate::data_control::DataControl::new(&dh),
            dh,
        }
    }

    /// Adds `output` at the origin and makes its area the usable area.
    pub fn add_output(&mut self, output: &Output) {
        self.space.map_output(output, (0, 0));
        self.refresh_area();
    }

    pub fn output(&self) -> Option<Output> { self.space.outputs().next().cloned() }

    /// The usable area is the output minus the layer surfaces' exclusive zones (the menu bar). Windows follow it.
    pub fn refresh_area(&mut self) {
        let Some(output) = self.output() else { return };
        let zone = layer_map_for_output(&output).non_exclusive_zone();
        let mut area = Rect::new(zone.loc.x, zone.loc.y, zone.size.w, zone.size.h);
        // a full-screen window fills the whole output, the menu bar's strip included
        let mut output_changed = false;
        if let Some(geo) = self.space.output_geometry(&output) {
            let whole = Rect::new(geo.loc.x, geo.loc.y, geo.size.w, geo.size.h);
            output_changed = self.wm.output() != whole;
            self.wm.set_output(whole);
            // a client's layer surface may claim an exclusive zone of any size: the area stays a sane part of the output
            area = crate::wm::clamp_area(area, whole);
        }
        let resized = self.wm.set_area(area);
        self.apply_frames();
        // a maximized or tiled window that grew or shrank with the area is configured to its new size, or its client
        // keeps drawing the old one (filled windows kept the Dock's strip after a full-screen exit, W3-6)
        for id in resized {
            if let Some(window) = self.window(id) {
                self.configure_frame(&window, id);
            }
        }
        // a full-screen window follows an output change: its client is told the new size (not on every menu bar commit)
        if output_changed {
            let full: Vec<crate::wm::WindowId> = self.wm.stack().into_iter().filter(|id| self.wm.is_full_screen(*id)).collect();
            for id in full {
                if let Some(window) = self.window(id) {
                    self.configure_frame(&window, id);
                }
            }
        }
    }

    /// Gives a window its `wm` id on first sight.
    pub fn id_of(&mut self, window: &Window) -> WindowId {
        if let Some(Id(id)) = window.user_data().get::<Id>() {
            return *id;
        }
        let id = self.next_id;
        self.next_id += 1;
        window.user_data().insert_if_missing(|| Id(id));
        id
    }

    pub fn known_id(window: &Window) -> Option<WindowId> { window.user_data().get::<Id>().map(|Id(id)| *id) }

    pub fn window(&self, id: WindowId) -> Option<Window> {
        self.space.elements().find(|w| Self::known_id(w) == Some(id)).cloned()
    }

    /// The shell set the level of `surface`: the window model moves its toplevel to that band, if it is mapped.
    pub fn set_window_level(&mut self, surface: &WlSurface, level: Level) {
        let Some(window) = self.window_for_surface(surface) else { return };
        let Some(id) = Self::known_id(&window) else { return };
        self.wm.set_level(id, level);
        self.apply_frames();
        self.dirty = true;
    }

    /// `set_stack_rank` for a mapped toplevel: it goes over its band, under the shown windows with a higher rank
    /// (`Wm::raise_ranked`), and the keyboard stays where it is. False when `surface` is no mapped toplevel: its rank
    /// then waits for the map.
    pub fn rank_window(&mut self, surface: &WlSurface, rank: u32) -> bool {
        let Some(window) = self.window_for_surface(surface) else { return false };
        let Some(id) = Self::known_id(&window) else { return false };
        self.wm.raise_ranked(id, rank);
        self.apply_frames();
        // the foreign-toplevel list follows the new stack, as after a placement (`place_pending`)
        self.sync_toplevels();
        self.dirty = true;
        true
    }

    pub fn window_for_surface(&self, surface: &WlSurface) -> Option<Window> {
        self.space.elements().find(|w| w.wl_surface().is_some_and(|s| s.as_ref() == surface)).cloned()
    }

    /// Moves `window` to `loc` without changing its place in the stacking order or the activation of any window
    /// (`Space::map_element` raises the element, so the order is restored afterwards). Only `focus` raises (WM-2).
    pub fn place(&mut self, window: &Window, loc: impl Into<Point<i32, Logical>>) {
        let order: Vec<Window> = self.space.elements().cloned().collect();
        self.space.map_element(window.clone(), loc, false);
        for w in &order {
            self.space.raise_element(w, false);
        }
        self.report_frames();
    }

    /// Moves every mapped window to the frame `wm` holds for it, in `wm` stacking order.
    pub fn apply_frames(&mut self) {
        for id in self.wm.stack() {
            // a minimized window is not in the space but is still held (`hidden`), so it can come back
            let Some(window) = self.window_any(id) else { continue };
            match self.wm.rect(id).filter(|_| self.wm.visible(id)) {
                Some(r) => {
                    self.unhide(id, &window);
                    self.space.map_element(window, (r.x, r.y), false)
                }
                None => self.keep_hidden(&window),
            }
        }
        self.report_frames();
    }

    /// Tells each client that bound `bifrost_window_level_v1` where the window server put its mapped toplevels, when
    /// that changed (map, move, resize, tile, maximize). The window server owns the positions and xdg-shell never
    /// tells a client, so the shell would otherwise place menus and taps from a frame of its own that drifts from the
    /// real one. Called wherever a window's frame reaches the space (`place`, `apply_frames`).
    pub fn report_frames(&mut self) {
        if self.frame_reports.is_empty() {
            return;
        }
        let now: Vec<(WindowId, Rect, WlSurface)> = self
            .wm
            .stack()
            .into_iter()
            .filter_map(|id| {
                let rect = self.wm.rect(id)?;
                let surface = self.window(id)?.wl_surface()?.into_owned();
                Some((id, rect, surface))
            })
            .collect();
        self.frame_reports.send(&now);
    }

    /// The client settled on a frame of its own (a size it chose, a resize it finished): `wm` keeps it, and the
    /// toplevel's pending size follows. Smithay sends the whole pending state with every configure, so a size left
    /// behind there (an old tile) came back with the next activation change and resized the window to it (W4-3).
    /// Nothing is sent here: the size rides along with the next configure that carries a real change.
    pub fn set_frame(&mut self, window: &Window, id: WindowId, rect: Rect) {
        if !self.wm.set_frame(id, rect) {
            return;
        }
        if let (Some(t), Some(r)) = (window.toplevel(), self.wm.rect(id)) {
            t.with_pending_state(|s| s.size = Some((r.w, r.h).into()));
        }
    }

    /// Whether the shell stacks and activates `window` itself: its client connected through the privileged socket and
    /// follows the frames the window server reports (`bifrost_window_level_v1`). The window server then does not
    /// raise or focus it when it maps; the shell asks for that with xdg-activation (`activate_surface`).
    pub fn shell_stacks(&self, window: &Window) -> bool {
        let Some(surface) = window.wl_surface() else { return false };
        let privileged = smithay::reexports::wayland_server::Resource::client(surface.as_ref()).is_some_and(|c| crate::privileged::is_privileged(&c));
        privileged && self.frame_reports.bound_by_owner_of(&surface)
    }

    /// Raises `id`, gives it keyboard focus and marks it active; every other window is marked inactive. Only a window
    /// whose activation changed is sent a configure: the others have nothing new to hear.
    pub fn focus(&mut self, id: Option<WindowId>, serial: smithay::utils::Serial) {
        if self.locked {
            return;
        }
        if let Some(id) = id {
            // the window model raises within its band (a floating window stays above); the space follows its order
            self.wm.raise(id);
            self.apply_frames();
        }
        let focused = self.wm.focused();
        let windows: Vec<Window> = self.space.elements().cloned().collect();
        for w in &windows {
            let active = Self::known_id(w) == focused && focused.is_some();
            if w.set_activated(active) {
                if let Some(t) = w.toplevel() {
                    t.send_pending_configure();
                }
            }
        }
        let surface = focused.and_then(|id| self.window(id)).and_then(|w| w.wl_surface().map(|s| s.into_owned()));
        if let Some(keyboard) = self.seat.get_keyboard() {
            keyboard.set_focus(self, surface, serial);
        }
        self.sync_toplevels();
    }

    /// Frame callbacks after a composited frame: windows, layer surfaces and the lock surface draw their next frame.
    pub fn send_frames(&self, output: &Output) {
        let elapsed = self.start.elapsed();
        let zero = Some(std::time::Duration::ZERO);
        for window in self.space.elements() {
            window.send_frame(output, elapsed, zero, |_, _| Some(output.clone()));
        }
        for layer in layer_map_for_output(output).layers() {
            layer.send_frame(output, elapsed, zero, |_, _| Some(output.clone()));
        }
        if let Some(lock) = &self.lock_surface {
            smithay::desktop::utils::send_frames_surface_tree(lock.wl_surface(), output, elapsed, zero, |_, _| Some(output.clone()));
        }
        // an input method's candidate window draws its next frame too
        for popup in self.text_input.popup_surfaces() {
            smithay::desktop::utils::send_frames_surface_tree(popup, output, elapsed, zero, |_, _| Some(output.clone()));
        }
    }

    /// The client surface under a point, and that surface's origin.
    pub fn surface_under(&self, pos: Point<f64, Logical>) -> Option<(WlSurface, Point<f64, Logical>)> {
        if self.locked {
            return self.lock_surface.as_ref().map(|l| (l.wl_surface().clone(), (0.0, 0.0).into()));
        }
        use smithay::wayland::shell::wlr_layer::Layer;
        self.layer_surface_under(Layer::Overlay, pos)
            .or_else(|| self.layer_surface_under(Layer::Top, pos))
            .or_else(|| self.input_popup_under(pos))
            .or_else(|| {
                self.space.element_under(pos).and_then(|(window, loc)| {
                    window.surface_under(pos - loc.to_f64(), WindowSurfaceType::ALL).map(|(s, p)| (s, (p + loc).to_f64()))
                })
            })
            .or_else(|| self.layer_surface_under(Layer::Bottom, pos))
            .or_else(|| self.layer_surface_under(Layer::Background, pos))
    }

    /// The surface of a layer surface on `layer` that takes the pointer at `pos`, the topmost first, and its origin.
    /// Each surface's input region decides, not its size: a surface exported with room for its shadow (a menu), or one
    /// that takes no input at all (a sweep, a caption), lets the point through to the surfaces below it on the same
    /// layer. Picking the topmost surface by its bounds alone (smithay's `layer_under`) ended the search at such a
    /// surface, and the press fell through to the window under it (E2E R1).
    pub fn layer_surface_under(&self, layer: smithay::wayland::shell::wlr_layer::Layer, pos: Point<f64, Logical>) -> Option<(WlSurface, Point<f64, Logical>)> {
        let output = self.output()?;
        let layers = layer_map_for_output(&output);
        // the order they are drawn in, from the top (`scene::layer_surfaces_bottom_first`: z rank, then map order)
        let found = crate::scene::layer_surfaces_bottom_first(&layers, layer).into_iter().rev().find_map(|l| {
            let geo = layers.layer_geometry(l)?;
            l.surface_under(pos - geo.loc.to_f64(), WindowSurfaceType::ALL).map(|(s, p)| (s, (p + geo.loc).to_f64()))
        });
        found
    }

    /// Whether a layer surface above the windows (the Overlay or Top layer: the shell's menus, menu bar, Dock and
    /// panels) takes the pointer at `pos`. A press there belongs to that surface, so it must not raise or focus the
    /// window beneath it: an app's pop-up menu that reaches past its window raised the window under that part (E2E R1).
    pub fn layer_above_windows_at(&self, pos: Point<f64, Logical>) -> bool {
        use smithay::wayland::shell::wlr_layer::Layer;
        !self.locked && (self.layer_surface_under(Layer::Overlay, pos).is_some() || self.layer_surface_under(Layer::Top, pos).is_some())
    }
}

#[derive(Default)]
pub struct ClientState {
    pub compositor_state: CompositorClientState,
    /// Cached from SO_PEERCRED on accept; filters must not query the Wayland backend while dispatching globals.
    pub peer_uid: Option<u32>,
    /// Cached from SO_PEERCRED on accept (`None` when unreadable; 0 for a peer in another pid namespace).
    pub peer_pid: Option<u32>,
    /// The peer was in this process's mount namespace when it connected. A sandboxed app has its own, so this is how
    /// the same-user data-control programs are told from a sandboxed app that runs as the same account. False by default
    /// (unknown peers fail closed).
    pub shares_mount_ns: bool,
    /// The client connected through the privileged socket (`privileged.rs`): the capture globals are visible to it.
    pub privileged: bool,
}

impl ClientData for ClientState {
    fn initialized(&self, _client_id: ClientId) {}
    fn disconnected(&self, _client_id: ClientId, _reason: DisconnectReason) {}
}

#[cfg(test)]
mod peer_tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn namespace_links_compare_equal_only_when_both_are_known_and_identical() {
        let a = Path::new("mnt:[4026531841]");
        let b = Path::new("mnt:[4026532500]");
        assert!(same_namespace(Some(a), Some(a)));
        assert!(!same_namespace(Some(a), Some(b)), "a sandbox has its own mount namespace");
        assert!(!same_namespace(None, Some(a)));
        assert!(!same_namespace(Some(a), None));
        assert!(!same_namespace(None, None), "unreadable links fail closed");
    }

    #[test]
    fn this_process_shares_its_own_mount_namespace_and_unknown_pids_do_not() {
        assert!(shares_mount_ns(Some(std::process::id())));
        assert!(!shares_mount_ns(None));
        assert!(!shares_mount_ns(Some(0)), "pid 0 is a peer in another pid namespace");
        assert!(!shares_mount_ns(Some(u32::MAX)), "no such process");
    }

    #[test]
    fn the_pid_comes_out_of_the_pidfd_info_text() {
        assert_eq!(parse_fdinfo_pid("pos:\t0\nflags:\t02000002\nmnt_id:\t17\nPid:\t4242\nNSpid:\t4242\n"), Some(4242));
        assert_eq!(parse_fdinfo_pid("Pid:\t-1\n"), None, "the process is gone");
        assert_eq!(parse_fdinfo_pid("Pid:\t0\n"), None, "another pid namespace");
        assert_eq!(parse_fdinfo_pid("Pid:\tabc\n"), None);
        assert_eq!(parse_fdinfo_pid("pos:\t0\n"), None);
    }

    #[test]
    fn a_peer_in_this_namespace_is_admitted_by_pidfd_or_by_pid() {
        let (a, b) = UnixStream::pair().expect("socketpair");
        let (pid, uid) = (peer_pid(&a), peer_uid(&a));
        assert!(peer_shares_mount_ns(&a, pid, uid), "this process is its own peer");
        assert!(peer_shares_mount_ns(&b, peer_pid(&b), peer_uid(&b)));
        // a wrong uid for the pid is refused on the pid path; the pidfd path ignores the number altogether
        if peer_pidfd(&a).is_none() {
            assert!(!peer_shares_mount_ns(&a, pid, uid.map(|u| u.wrapping_add(1))), "a recycled pid of another account");
        }
    }

    #[test]
    fn a_pidfd_notices_that_its_process_exited() {
        use smithay::reexports::rustix::process::{pidfd_open, Pid, PidfdFlags};
        let mut child = std::process::Command::new("true").spawn().expect("spawn true");
        let pid = Pid::from_raw(child.id() as i32).expect("pid");
        let Ok(pidfd) = pidfd_open(pid, PidfdFlags::empty()) else {
            let _ = child.wait();
            return; // no pidfd_open here (an old kernel or a seccomp filter): nothing to check
        };
        child.wait().expect("wait");
        assert!(pidfd_exited(&pidfd));
        assert!(!pidfd_shares_mount_ns(&pidfd), "an exited connector fails closed");
        let own = pidfd_open(Pid::from_raw(std::process::id() as i32).expect("own pid"), PidfdFlags::empty()).expect("own pidfd");
        assert!(!pidfd_exited(&own));
        assert!(pidfd_shares_mount_ns(&own));
    }
}
