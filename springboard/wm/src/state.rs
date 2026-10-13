//! The compositor state: Smithay's protocol states, the window space, and the `wm` rules that place and move
//! windows in it.

use std::{ffi::OsString, sync::Arc, time::Instant};

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

use crate::wm::{Rect, WindowId, Wm};

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
    /// The appearance materials follow: Dark unless `BIFROST_WM_APPEARANCE=light`.
    pub material_dark: bool,
    pub lock_state: smithay::wayland::session_lock::SessionLockManagerState,
    /// A session lock is held: only `lock_surface` is drawn and takes input.
    pub locked: bool,
    pub lock_surface: Option<smithay::wayland::session_lock::LockSurface>,
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
    /// XWayland, its X11 window manager and the xwayland-shell global (`xwayland.rs`).
    #[cfg(feature = "xwayland")]
    pub xwayland: crate::xwayland::XWaylandState,
    /// Session protocols: foreign toplevels, screencopy, output management and power, cursor shapes, activation.
    pub session: crate::session_protocols::SessionState,
    /// Text input, input method and virtual keyboard (`text_input.rs`).
    pub text_input: crate::text_input::TextInputProtocols,
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
                if let Err(e) = state.dh.insert_client(stream, Arc::new(ClientState::default())) {
                    eprintln!("bifrost-wm: client refused: {e}");
                }
            })
            .expect("socket source");
        handle
            .insert_source(Generic::new(display, Interest::READ, Mode::Level), |_, display, state| {
                // SAFETY: the display is never dropped while the loop runs.
                // a failed dispatch is logged, not fatal: one broken client connection must not take the session down
                if let Err(e) = unsafe { display.get_mut().dispatch_clients(state) } {
                    eprintln!("bifrost-wm: dispatch clients: {e}");
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
            material_dark: std::env::var("BIFROST_WM_APPEARANCE").map(|v| v != "light").unwrap_or(true),
            lock_state: smithay::wayland::session_lock::SessionLockManagerState::new::<WmState, _>(&dh, |_| true),
            locked: false,
            lock_surface: None,
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
            #[cfg(feature = "xwayland")]
            xwayland: crate::xwayland::XWaylandState::new(&dh),
            session,
            text_input: crate::text_input::TextInputProtocols::new(&dh),
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
        let area = Rect::new(zone.loc.x, zone.loc.y, zone.size.w, zone.size.h);
        self.wm.set_area(area);
        self.apply_frames();
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
    }

    /// Moves every mapped window to the frame `wm` holds for it, in `wm` stacking order.
    pub fn apply_frames(&mut self) {
        for id in self.wm.stack() {
            let Some(window) = self.window(id) else { continue };
            match self.wm.rect(id).filter(|_| self.wm.visible(id)) {
                Some(r) => self.space.map_element(window, (r.x, r.y), false),
                None => self.space.unmap_elem(&window),
            }
        }
    }

    /// Raises `id`, gives it keyboard focus and marks it active; every other window is marked inactive.
    pub fn focus(&mut self, id: Option<WindowId>, serial: smithay::utils::Serial) {
        if self.locked {
            return;
        }
        if let Some(id) = id {
            self.wm.raise(id);
            if let Some(w) = self.window(id) {
                self.space.raise_element(&w, true);
            }
        }
        let focused = self.wm.focused();
        let windows: Vec<Window> = self.space.elements().cloned().collect();
        for w in &windows {
            let active = Self::known_id(w) == focused && focused.is_some();
            w.set_activated(active);
            if let Some(t) = w.toplevel() {
                t.send_pending_configure();
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
        let output = self.output()?;
        let layers = layer_map_for_output(&output);
        let layer_hit = |l: &smithay::wayland::shell::wlr_layer::Layer| {
            layers.layer_under(*l, pos).and_then(|layer| {
                let geo = layers.layer_geometry(layer)?;
                layer
                    .surface_under(pos - geo.loc.to_f64(), WindowSurfaceType::ALL)
                    .map(|(s, p)| (s, (p + geo.loc).to_f64()))
            })
        };
        use smithay::wayland::shell::wlr_layer::Layer;
        layer_hit(&Layer::Overlay)
            .or_else(|| layer_hit(&Layer::Top))
            .or_else(|| self.input_popup_under(pos))
            .or_else(|| {
                self.space.element_under(pos).and_then(|(window, loc)| {
                    window.surface_under(pos - loc.to_f64(), WindowSurfaceType::ALL).map(|(s, p)| (s, (p + loc).to_f64()))
                })
            })
            .or_else(|| layer_hit(&Layer::Bottom))
            .or_else(|| layer_hit(&Layer::Background))
    }
}

#[derive(Default)]
pub struct ClientState {
    pub compositor_state: CompositorClientState,
}

impl ClientData for ClientState {
    fn initialized(&self, _client_id: ClientId) {}
    fn disconnected(&self, _client_id: ClientId, _reason: DisconnectReason) {}
}
