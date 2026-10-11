//! The compositor state: Smithay's protocol states, the window space, and the `wm` rules that place and move
//! windows in it.

use std::{ffi::OsString, sync::Arc, time::Instant};

use smithay::{
    desktop::{layer_map_for_output, PopupManager, Space, Window, WindowSurfaceType},
    input::{Seat, SeatState},
    output::Output,
    reexports::{
        calloop::{generic::Generic, EventLoop, Interest, LoopSignal, Mode, PostAction},
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
    pub socket_name: OsString,
    pub dh: DisplayHandle,
    pub space: Space<Window>,
    pub loop_signal: LoopSignal,
    pub wm: Wm,
    next_id: WindowId,
    /// Toplevels waiting for their first buffer before `wm` places them.
    pub pending: Vec<Window>,

    pub compositor_state: CompositorState,
    pub xdg_shell_state: XdgShellState,
    pub layer_shell_state: WlrLayerShellState,
    pub shm_state: ShmState,
    pub output_manager_state: OutputManagerState,
    pub seat_state: SeatState<WmState>,
    pub data_device_state: DataDeviceState,
    pub popups: PopupManager,
    pub seat: Seat<WmState>,
}

impl WmState {
    /// Listens on `socket` (or the next free `wayland-N`) and dispatches clients from `event_loop`.
    pub fn new(event_loop: &mut EventLoop<'static, WmState>, display: Display<WmState>, socket: Option<&str>) -> Self {
        let dh = display.handle();
        let mut seat_state = SeatState::new();
        let mut seat: Seat<WmState> = seat_state.new_wl_seat(&dh, "seat0");
        seat.add_keyboard(Default::default(), 300, 30).expect("keyboard");
        seat.add_pointer();

        let listening = match socket {
            Some(name) => ListeningSocketSource::with_name(name).expect("wayland socket"),
            None => ListeningSocketSource::new_auto().expect("wayland socket"),
        };
        let socket_name = listening.socket_name().to_os_string();
        let handle = event_loop.handle();
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
                unsafe { display.get_mut().dispatch_clients(state).expect("dispatch clients") };
                Ok(PostAction::Continue)
            })
            .expect("display source");

        WmState {
            start: Instant::now(),
            socket_name,
            space: Space::default(),
            loop_signal: event_loop.get_signal(),
            wm: Wm::new(Rect::default()),
            next_id: 1,
            pending: Vec::new(),
            compositor_state: CompositorState::new::<WmState>(&dh),
            xdg_shell_state: XdgShellState::new::<WmState>(&dh),
            layer_shell_state: WlrLayerShellState::new::<WmState>(&dh),
            shm_state: ShmState::new::<WmState>(&dh, vec![]),
            output_manager_state: OutputManagerState::new_with_xdg_output::<WmState>(&dh),
            data_device_state: DataDeviceState::new::<WmState>(&dh),
            popups: PopupManager::default(),
            seat_state,
            seat,
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
        self.space.elements().find(|w| w.toplevel().is_some_and(|t| t.wl_surface() == surface)).cloned()
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
        let surface = focused.and_then(|id| self.window(id)).and_then(|w| w.toplevel().map(|t| t.wl_surface().clone()));
        if let Some(keyboard) = self.seat.get_keyboard() {
            keyboard.set_focus(self, surface, serial);
        }
    }

    /// The client surface under a point, and that surface's origin.
    pub fn surface_under(&self, pos: Point<f64, Logical>) -> Option<(WlSurface, Point<f64, Logical>)> {
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
