//! Protocol handlers: compositor, shm, seat, data device, output, xdg-shell and wlr-layer-shell.

use std::cell::RefCell;

use smithay::{
    backend::renderer::utils::on_commit_buffer_handler,
    delegate_compositor, delegate_data_device, delegate_layer_shell, delegate_output, delegate_seat, delegate_shm,
    delegate_xdg_shell,
    desktop::{layer_map_for_output, LayerSurface, PopupKind, Window},
    input::{
        pointer::{Focus, GrabStartData},
        Seat, SeatHandler, SeatState,
    },
    output::Output,
    reexports::{
        wayland_protocols::xdg::shell::server::xdg_toplevel,
        wayland_server::{
            protocol::{wl_buffer, wl_output::WlOutput, wl_seat, wl_surface::WlSurface},
            Client, Resource,
        },
    },
    utils::Serial,
    wayland::{
        buffer::BufferHandler,
        compositor::{get_parent, is_sync_subsurface, with_states, CompositorClientState, CompositorHandler, CompositorState},
        output::OutputHandler,
        selection::{
            data_device::{set_data_device_focus, ClientDndGrabHandler, DataDeviceHandler, DataDeviceState, ServerDndGrabHandler},
            SelectionHandler,
        },
        shell::{
            wlr_layer::{Layer, LayerSurface as WlrLayerSurface, LayerSurfaceData, WlrLayerShellHandler, WlrLayerShellState},
            xdg::{PopupSurface, PositionerState, ToplevelSurface, XdgShellHandler, XdgShellState, XdgToplevelSurfaceData},
        },
        shm::{ShmHandler, ShmState},
    },
};

use crate::{
    grabs::{MoveGrab, ResizeGrab},
    state::{ClientState, WmState},
    wm::{Edges, Rect},
};

/// A resize in progress: the frame `wm` asked for and the edges that move, applied when the client commits.
#[derive(Default)]
pub struct ResizeTarget(pub RefCell<Option<(Edges, Rect)>>);

impl CompositorHandler for WmState {
    fn compositor_state(&mut self) -> &mut CompositorState { &mut self.compositor_state }

    fn client_compositor_state<'a>(&self, client: &'a Client) -> &'a CompositorClientState {
        if let Some(state) = client.get_data::<ClientState>() {
            return &state.compositor_state;
        }
        #[cfg(feature = "xwayland")]
        if let Some(state) = crate::xwayland::client_compositor_state(client) {
            return state;
        }
        panic!("client state")
    }

    fn commit(&mut self, surface: &WlSurface) {
        on_commit_buffer_handler::<Self>(surface);
        crate::background_effect::commit(surface);
        if !is_sync_subsurface(surface) {
            let mut root = surface.clone();
            while let Some(parent) = get_parent(&root) {
                root = parent;
            }
            if let Some(window) = self.window_for_surface(&root) {
                window.on_commit();
                self.settle(&window);
            }
            self.place_pending(&root);
        }
        self.popups.commit(surface);
        if let Some(PopupKind::Xdg(xdg)) = self.popups.find_popup(surface) {
            if !xdg.is_initial_configure_sent() {
                let _ = xdg.send_configure();
            }
        }
        self.layer_commit(surface);
    }
}

impl WmState {
    /// A new toplevel gets its initial configure; once it has a buffer, `wm` places, raises and focuses it.
    fn place_pending(&mut self, root: &WlSurface) {
        let Some(i) = self.pending.iter().position(|w| w.toplevel().is_some_and(|t| t.wl_surface() == root)) else { return };
        let window = self.pending[i].clone();
        let toplevel = window.toplevel().expect("toplevel").clone();
        let configured = with_states(root, |states| {
            states.data_map.get::<XdgToplevelSurfaceData>().is_some_and(|d| d.lock().unwrap().initial_configure_sent)
        });
        if !configured {
            toplevel.send_configure();
            return;
        }
        window.on_commit();
        let size = window.geometry().size;
        if size.w <= 0 || size.h <= 0 {
            return;
        }
        self.pending.remove(i);
        let id = self.id_of(&window);
        let r = self.wm.map(id, size.w, size.h);
        if (r.w, r.h) != (size.w, size.h) {
            toplevel.with_pending_state(|s| s.size = Some((r.w, r.h).into()));
        }
        window.user_data().insert_if_missing(ResizeTarget::default);
        self.space.map_element(window, (r.x, r.y), true);
        self.focus(Some(id), smithay::utils::SERIAL_COUNTER.next_serial());
    }

    /// After a commit: a resizing window is anchored at the edges that do not move; any other size change the
    /// client made becomes its frame.
    fn settle(&mut self, window: &Window) {
        let Some(id) = WmState::known_id(window) else { return };
        let Some(frame) = self.wm.rect(id) else { return };
        let size = window.geometry().size;
        let target = window.user_data().get::<ResizeTarget>().and_then(|t| *t.0.borrow());
        let rect = match target {
            Some((edges, want)) => {
                let x = if edges.left { want.x + want.w - size.w } else { want.x };
                let y = if edges.top { want.y + want.h - size.h } else { want.y };
                Rect::new(x, y, size.w, size.h)
            }
            None if (size.w, size.h) != (frame.w, frame.h) && size.w > 0 && size.h > 0 => Rect { w: size.w, h: size.h, ..frame },
            None => return,
        };
        self.wm.set_frame(id, rect);
        self.space.map_element(window.clone(), (rect.x, rect.y), false);
    }

    /// Layer surfaces: initial configure, arrangement, and the usable area their exclusive zones leave.
    fn layer_commit(&mut self, surface: &WlSurface) {
        let Some(output) = self.output() else { return };
        let mut map = layer_map_for_output(&output);
        let Some(layer) = map.layer_for_surface(surface, smithay::desktop::WindowSurfaceType::TOPLEVEL).cloned() else { return };
        let initial = with_states(surface, |states| {
            states.data_map.get::<LayerSurfaceData>().is_some_and(|d| d.lock().unwrap().initial_configure_sent)
        });
        map.arrange();
        if !initial {
            layer.layer_surface().send_configure();
        }
        drop(map);
        self.refresh_area();
    }
}

impl BufferHandler for WmState {
    fn buffer_destroyed(&mut self, _buffer: &wl_buffer::WlBuffer) {}
}

impl ShmHandler for WmState {
    fn shm_state(&self) -> &ShmState { &self.shm_state }
}

impl SeatHandler for WmState {
    type KeyboardFocus = WlSurface;
    type PointerFocus = WlSurface;
    type TouchFocus = WlSurface;

    fn seat_state(&mut self) -> &mut SeatState<WmState> { &mut self.seat_state }

    fn cursor_image(&mut self, _seat: &Seat<Self>, _image: smithay::input::pointer::CursorImageStatus) {}

    fn focus_changed(&mut self, seat: &Seat<Self>, focused: Option<&WlSurface>) {
        let client = focused.and_then(|s| self.dh.get_client(s.id()).ok());
        set_data_device_focus(&self.dh, seat, client);
    }
}

impl SelectionHandler for WmState {
    type SelectionUserData = ();
}

impl DataDeviceHandler for WmState {
    fn data_device_state(&self) -> &DataDeviceState { &self.data_device_state }
}

impl ClientDndGrabHandler for WmState {}
impl ServerDndGrabHandler for WmState {}

impl OutputHandler for WmState {}

impl XdgShellHandler for WmState {
    fn xdg_shell_state(&mut self) -> &mut XdgShellState { &mut self.xdg_shell_state }

    fn new_toplevel(&mut self, surface: ToplevelSurface) {
        self.pending.push(Window::new_wayland_window(surface));
    }

    fn new_popup(&mut self, surface: PopupSurface, _positioner: PositionerState) {
        let _ = self.popups.track_popup(PopupKind::Xdg(surface));
    }

    fn reposition_request(&mut self, surface: PopupSurface, positioner: PositionerState, token: u32) {
        surface.with_pending_state(|state| {
            state.geometry = positioner.get_geometry();
            state.positioner = positioner;
        });
        surface.send_repositioned(token);
    }

    fn grab(&mut self, _surface: PopupSurface, _seat: wl_seat::WlSeat, _serial: Serial) {}

    fn move_request(&mut self, surface: ToplevelSurface, seat: wl_seat::WlSeat, serial: Serial) {
        let Some(seat) = Seat::<WmState>::from_resource(&seat) else { return };
        let Some(start) = check_grab(&seat, surface.wl_surface(), serial) else { return };
        let Some(window) = self.window_for_surface(surface.wl_surface()) else { return };
        let Some(id) = WmState::known_id(&window) else { return };
        let was_max = self.wm.is_maximized(id);
        let Some(grab) = self.wm.begin_move(id, start.location.x as i32, start.location.y as i32) else { return };
        if was_max {
            let r = self.wm.rect(id).expect("frame");
            surface.with_pending_state(|s| {
                s.states.unset(xdg_toplevel::State::Maximized);
                s.size = Some((r.w, r.h).into());
            });
            surface.send_pending_configure();
            self.space.map_element(window.clone(), (r.x, r.y), true);
        }
        let pointer = seat.get_pointer().expect("pointer");
        pointer.set_grab(self, MoveGrab { start, id, grab }, serial, Focus::Clear);
    }

    fn resize_request(&mut self, surface: ToplevelSurface, seat: wl_seat::WlSeat, serial: Serial, edges: xdg_toplevel::ResizeEdge) {
        let Some(seat) = Seat::<WmState>::from_resource(&seat) else { return };
        let Some(start) = check_grab(&seat, surface.wl_surface(), serial) else { return };
        let Some(window) = self.window_for_surface(surface.wl_surface()) else { return };
        let Some(id) = WmState::known_id(&window) else { return };
        let edges = edges_of(edges);
        let Some(grab) = self.wm.begin_resize(id, edges, start.location.x as i32, start.location.y as i32) else { return };
        surface.with_pending_state(|s| {
            s.states.unset(xdg_toplevel::State::Maximized);
            s.states.set(xdg_toplevel::State::Resizing);
        });
        surface.send_pending_configure();
        let pointer = seat.get_pointer().expect("pointer");
        pointer.set_grab(self, ResizeGrab { start, id, grab, edges, window }, serial, Focus::Clear);
    }

    fn maximize_request(&mut self, surface: ToplevelSurface) {
        let Some(window) = self.window_for_surface(surface.wl_surface()) else { return };
        let Some(id) = WmState::known_id(&window) else { return };
        if self.wm.is_maximized(id) {
            surface.send_configure();
            return;
        }
        self.set_maximized(&window, id, &surface);
    }

    fn unmaximize_request(&mut self, surface: ToplevelSurface) {
        let Some(window) = self.window_for_surface(surface.wl_surface()) else { return };
        let Some(id) = WmState::known_id(&window) else { return };
        if self.wm.is_maximized(id) {
            self.set_maximized(&window, id, &surface);
        } else {
            surface.send_configure();
        }
    }

    fn minimize_request(&mut self, surface: ToplevelSurface) {
        let Some(window) = self.window_for_surface(surface.wl_surface()) else { return };
        let Some(id) = WmState::known_id(&window) else { return };
        self.wm.minimize(id);
        self.space.unmap_elem(&window);
        self.focus(None, smithay::utils::SERIAL_COUNTER.next_serial());
    }

    fn toplevel_destroyed(&mut self, surface: ToplevelSurface) {
        self.pending.retain(|w| w.toplevel().is_some_and(|t| t != &surface));
        if let Some(window) = self.window_for_surface(surface.wl_surface()) {
            if let Some(id) = WmState::known_id(&window) {
                self.wm.unmap(id);
            }
            self.space.unmap_elem(&window);
            self.focus(None, smithay::utils::SERIAL_COUNTER.next_serial());
        }
    }
}

impl WmState {
    /// Toggles maximize through `wm` and tells the client its new size and state.
    fn set_maximized(&mut self, window: &Window, id: crate::wm::WindowId, surface: &ToplevelSurface) {
        let Some(r) = self.wm.toggle_maximize(id) else { return };
        let maximized = self.wm.is_maximized(id);
        surface.with_pending_state(|s| {
            if maximized {
                s.states.set(xdg_toplevel::State::Maximized);
            } else {
                s.states.unset(xdg_toplevel::State::Maximized);
            }
            s.size = Some((r.w, r.h).into());
        });
        surface.send_pending_configure();
        self.space.map_element(window.clone(), (r.x, r.y), true);
    }
}

impl WlrLayerShellHandler for WmState {
    fn shell_state(&mut self) -> &mut WlrLayerShellState { &mut self.layer_shell_state }

    fn new_layer_surface(&mut self, surface: WlrLayerSurface, output: Option<WlOutput>, _layer: Layer, namespace: String) {
        let output = output.as_ref().and_then(Output::from_resource).or_else(|| self.output());
        let Some(output) = output else { return };
        let mapped = layer_map_for_output(&output).map_layer(&LayerSurface::new(surface, namespace));
        if let Err(e) = mapped {
            eprintln!("bifrost-wm: layer surface refused: {e}");
        }
    }

    fn layer_destroyed(&mut self, surface: WlrLayerSurface) {
        let Some(output) = self.output() else { return };
        {
            let mut map = layer_map_for_output(&output);
            let layer = map.layers().find(|l| l.layer_surface() == &surface).cloned();
            if let Some(layer) = layer {
                map.unmap_layer(&layer);
            }
        }
        self.refresh_area();
    }
}

/// The pointer grab a move or resize request may start from: the client must hold a button press on its surface.
fn check_grab(seat: &Seat<WmState>, surface: &WlSurface, serial: Serial) -> Option<GrabStartData<WmState>> {
    let pointer = seat.get_pointer()?;
    if !pointer.has_grab(serial) {
        return None;
    }
    let start = pointer.grab_start_data()?;
    let (focus, _) = start.focus.as_ref()?;
    focus.id().same_client_as(&surface.id()).then_some(start)
}

fn edges_of(e: xdg_toplevel::ResizeEdge) -> Edges {
    use xdg_toplevel::ResizeEdge as R;
    Edges {
        left: matches!(e, R::Left | R::TopLeft | R::BottomLeft),
        right: matches!(e, R::Right | R::TopRight | R::BottomRight),
        top: matches!(e, R::Top | R::TopLeft | R::TopRight),
        bottom: matches!(e, R::Bottom | R::BottomLeft | R::BottomRight),
    }
}

impl smithay::wayland::idle_notify::IdleNotifierHandler for WmState {
    fn idle_notifier_state(&mut self) -> &mut smithay::wayland::idle_notify::IdleNotifierState<Self> { &mut self.idle_notifier_state }
}

impl smithay::wayland::dmabuf::DmabufHandler for WmState {
    fn dmabuf_state(&mut self) -> &mut smithay::wayland::dmabuf::DmabufState { &mut self.dmabuf_state }

    /// A client's dmabuf is accepted only if the renderer can import it (so a frame never fails on it later).
    fn dmabuf_imported(&mut self, _global: &smithay::wayland::dmabuf::DmabufGlobal, dmabuf: smithay::backend::allocator::dmabuf::Dmabuf, notifier: smithay::wayland::dmabuf::ImportNotifier) {
        let ok = self.dmabuf_import.as_mut().is_some_and(|f| f(&dmabuf));
        if ok {
            let _ = notifier.successful::<WmState>();
        } else {
            notifier.failed();
        }
    }
}

smithay::delegate_idle_notify!(WmState);
smithay::delegate_dmabuf!(WmState);
delegate_compositor!(WmState);
delegate_shm!(WmState);
delegate_seat!(WmState);
delegate_data_device!(WmState);
delegate_output!(WmState);
delegate_xdg_shell!(WmState);
delegate_layer_shell!(WmState);
