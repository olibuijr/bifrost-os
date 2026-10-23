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
        seat::WaylandFocus,
        compositor::{get_parent, is_sync_subsurface, with_states, CompositorClientState, CompositorHandler, CompositorState},
        output::OutputHandler,
        selection::{
            data_device::{set_data_device_focus, ClientDndGrabHandler, DataDeviceHandler, DataDeviceState, ServerDndGrabHandler},
            SelectionHandler,
        },
        shell::{
            wlr_layer::{KeyboardInteractivity, Layer, LayerSurface as WlrLayerSurface, LayerSurfaceCachedState, LayerSurfaceData, WlrLayerShellHandler, WlrLayerShellState},
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

/// Whether a layer surface may take the keyboard exclusively: not while a session lock is held, and only a privileged
/// client (the shell). Everyone else keeps the ordinary on-demand focus.
pub fn exclusive_keyboard_allowed(locked: bool, privileged: bool) -> bool { !locked && privileged }

/// A resize in progress: the frame `wm` asked for and the edges that move, applied when the client commits.
#[derive(Default)]
pub struct ResizeTarget(pub RefCell<Option<(Edges, Rect)>>);

/// The shell asked to activate a toplevel that had no buffer yet (`activate_surface`): it takes the front when it maps.
#[derive(Default)]
pub struct ActivateOnMap(pub std::cell::Cell<bool>);

/// Whether a toplevel that just got its first buffer is raised and focused as it maps. A window the shell stacks itself
/// (`WmState::shell_stacks`) takes the front only when the shell asked for it (an activation that came before the
/// buffer) or when nothing else is shown; otherwise the shell's choice of front window stands, and a window it brings
/// back behind that one (Overview, a minimize) maps under it without the keyboard (W4-1, W4-5). Without the shell (an
/// app's own window, a session without the shell) every new window takes the front, as before.
pub fn map_takes_front(shell_stacks: bool, activation_asked: bool, others_shown: bool) -> bool {
    !shell_stacks || activation_asked || !others_shown
}

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

    fn new_surface(&mut self, surface: &WlSurface) {
        crate::sync::watch_surface(surface);
    }

    fn commit(&mut self, surface: &WlSurface) {
        on_commit_buffer_handler::<Self>(surface);
        crate::background_effect::commit(surface);
        crate::bifrost_material::commit(surface);
        if !is_sync_subsurface(surface) {
            let mut root = surface.clone();
            while let Some(parent) = get_parent(&root) {
                root = parent;
            }
            if let Some(window) = self.window_for_surface(&root) {
                window.on_commit();
                if let Some(id) = WmState::known_id(&window) {
                    self.window_capture.touch(id);
                }
                self.settle(&window);
                self.sync_toplevels();
            } else if let Some(window) = self.hidden_for_surface(&root) {
                // a minimized window keeps its content current (a capture of it shows the last commit); it is not
                // placed or settled, which would map it
                window.on_commit();
                if let Some(id) = WmState::known_id(&window) {
                    self.window_capture.touch(id);
                }
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
    /// A new toplevel gets its initial configure; once it has a buffer, `wm` places it. It is raised and focused unless
    /// the shell stacks it (`map_takes_front`).
    fn place_pending(&mut self, root: &WlSurface) {
        let Some(i) = self.pending.iter().position(|w| w.toplevel().is_some_and(|t| t.wl_surface() == root)) else { return };
        let window = self.pending[i].clone();
        let toplevel = window.toplevel().expect("toplevel").clone();
        let configured = with_states(root, |states| {
            states.data_map.get::<XdgToplevelSurfaceData>().is_some_and(|d| d.lock().unwrap_or_else(|p| p.into_inner()).initial_configure_sent)
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
        // a window the shell brought back maps where it was (`bifrost_window_level_v1` set_origin), not at the cascade
        let origin = window.wl_surface().and_then(|s| crate::window_level::origin(&s));
        let activation_asked = window.user_data().get::<ActivateOnMap>().is_some_and(|a| a.0.get());
        let others_shown = self.wm.stack().into_iter().any(|o| o != id && self.wm.visible(o));
        let takes_front = map_takes_front(self.shell_stacks(&window), activation_asked, others_shown);
        // the level the shell asked for this surface before it mapped (`bifrost_window_level_v1`)
        let level = window.wl_surface().map(|s| crate::window_level::level(&s)).unwrap_or_default();
        let r = if takes_front {
            let r = self.wm.map_at(id, size.w, size.h, origin);
            self.wm.set_level(id, level);
            r
        } else {
            self.wm.map_behind(id, size.w, size.h, origin, level)
        };
        if (r.w, r.h) != (size.w, size.h) {
            toplevel.with_pending_state(|s| s.size = Some((r.w, r.h).into()));
        }
        window.user_data().insert_if_missing(ResizeTarget::default);
        self.space.map_element(window.clone(), (r.x, r.y), false);
        if takes_front {
            // the owner hears the frame the window server chose for it (a lock keeps `focus` from running)
            self.report_frames();
            self.focus(Some(id), smithay::utils::SERIAL_COUNTER.next_serial());
            // `focus` configures only a change of activation: a size `wm` fitted the window to still has to be sent
            toplevel.send_pending_configure();
        } else {
            // the shell's window goes under the one in front, inactive, and the keyboard stays where it was; the space
            // follows the `wm` order, and the shell activates the window when it brings it forward
            window.set_activated(false);
            toplevel.send_pending_configure();
            self.apply_frames();
            self.sync_toplevels();
        }
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
            // a tiled or full-screen window is sized by the window server (the client was configured to it): a commit made
            // before the client applied that configure must not turn the old size back into its frame
            None if self.wm.snapped(id).is_some() || self.wm.is_full_screen(id) => return,
            // nor does a commit drawn before the client saw a resize it was sent (a maximize, a centre)
            None if awaiting_resize(window, (size.w, size.h)) => return,
            None if (size.w, size.h) != (frame.w, frame.h) && size.w > 0 && size.h > 0 => Rect { w: size.w, h: size.h, ..frame },
            None => return,
        };
        self.set_frame(window, id, rect);
        self.place(window, (rect.x, rect.y));
    }

    /// Layer surfaces: initial configure, arrangement, and the usable area their exclusive zones leave.
    fn layer_commit(&mut self, surface: &WlSurface) {
        let Some(output) = self.output() else { return };
        let mut map = layer_map_for_output(&output);
        let Some(layer) = map.layer_for_surface(surface, smithay::desktop::WindowSurfaceType::TOPLEVEL).cloned() else { return };
        let initial = with_states(surface, |states| {
            states.data_map.get::<LayerSurfaceData>().is_some_and(|d| d.lock().unwrap_or_else(|p| p.into_inner()).initial_configure_sent)
        });
        map.arrange();
        if !initial {
            layer.layer_surface().send_configure();
        }
        let ki = with_states(surface, |states| {
            states.cached_state.get::<LayerSurfaceCachedState>().current().keyboard_interactivity
        });
        // an exclusive grab takes the keyboard from every window and from the lock screen: only the shell may hold one,
        // and never while the session is locked (a sandboxed app must not receive the password being typed)
        let privileged = surface.client().is_some_and(|client| crate::privileged::is_privileged(&client));
        if ki == KeyboardInteractivity::Exclusive && exclusive_keyboard_allowed(self.locked, privileged) {
            let serial = smithay::utils::SERIAL_COUNTER.next_serial();
            if let Some(keyboard) = self.seat.get_keyboard() {
                if keyboard.current_focus().as_ref() != Some(surface) {
                    keyboard.set_focus(self, Some(surface.clone()), serial);
                }
            }
        } else if ki == KeyboardInteractivity::None {
            if let Some(keyboard) = self.seat.get_keyboard() {
                if keyboard.current_focus().as_ref() == Some(surface) {
                    let serial = smithay::utils::SERIAL_COUNTER.next_serial();
                    let top_surface = self.wm.focused().and_then(|id| self.window(id)).and_then(|w| w.wl_surface().map(|s| s.into_owned()));
                    keyboard.set_focus(self, top_surface, serial);
                }
            }
        }
        drop(map);
        self.refresh_area();
    }
}

impl BufferHandler for WmState {
    fn buffer_destroyed(&mut self, buffer: &wl_buffer::WlBuffer) {
        if self.destroyed_buffers.len() < 65_536 {
            self.destroyed_buffers.push(crate::render_server::scene_build::buffer_key(buffer));
        }
    }
}

impl ShmHandler for WmState {
    fn shm_state(&self) -> &ShmState { &self.shm_state }
}

impl SeatHandler for WmState {
    type KeyboardFocus = WlSurface;
    type PointerFocus = WlSurface;
    type TouchFocus = WlSurface;

    fn seat_state(&mut self) -> &mut SeatState<WmState> { &mut self.seat_state }

    fn cursor_image(&mut self, _seat: &Seat<Self>, image: smithay::input::pointer::CursorImageStatus) {
        self.session.cursor = Some(image);
    }

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
        // a maximized window without a frame has nothing to restore to: it just moves from where it is
        if let Some(r) = was_max.then(|| self.wm.rect(id)).flatten() {
            surface.with_pending_state(|s| {
                s.states.unset(xdg_toplevel::State::Maximized);
                s.size = Some((r.w, r.h).into());
            });
            surface.send_pending_configure();
            self.place(&window, (r.x, r.y));
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

    fn fullscreen_request(&mut self, surface: ToplevelSurface, _output: Option<WlOutput>) {
        let Some(window) = self.window_for_surface(surface.wl_surface()) else { return };
        let Some(id) = WmState::known_id(&window) else { return };
        self.set_full_screen(id, true);
    }

    fn unfullscreen_request(&mut self, surface: ToplevelSurface) {
        let Some(window) = self.window_for_surface(surface.wl_surface()) else { return };
        let Some(id) = WmState::known_id(&window) else { return };
        self.set_full_screen(id, false);
    }

    fn minimize_request(&mut self, surface: ToplevelSurface) {
        let Some(window) = self.window_for_surface(surface.wl_surface()) else { return };
        let Some(id) = WmState::known_id(&window) else { return };
        self.wm.minimize(id);
        self.hide_window(id, &window);
        self.focus(None, smithay::utils::SERIAL_COUNTER.next_serial());
    }

    fn toplevel_destroyed(&mut self, surface: ToplevelSurface) {
        self.pending.retain(|w| w.toplevel().is_some_and(|t| t != &surface));
        // a window closed while minimized is held hidden: free it with what capture kept for it
        if let Some(window) = self.hidden_for_surface(surface.wl_surface()) {
            if let Some(id) = WmState::known_id(&window) {
                self.wm.unmap(id);
                self.forget_window(id, &window);
            }
            self.focus(None, smithay::utils::SERIAL_COUNTER.next_serial());
        }
        if let Some(window) = self.window_for_surface(surface.wl_surface()) {
            if let Some(id) = WmState::known_id(&window) {
                self.wm.unmap(id);
                self.forget_window(id, &window);
            }
            self.space.unmap_elem(&window);
            self.focus(None, smithay::utils::SERIAL_COUNTER.next_serial());
        }
    }
}

impl WmState {
    /// Toggles maximize through `wm` and tells the client its new size and state.
    pub(crate) fn set_maximized(&mut self, window: &Window, id: crate::wm::WindowId, surface: &ToplevelSurface) {
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
        self.place(window, (r.x, r.y));
        self.sync_toplevels();
    }
}

impl WmState {
    /// Tells a toplevel the state and size `wm` now holds for it (maximized, full screen, frame size) and moves it
    /// there; its owner hears the frame (`bifrost_window_level_v1`) and the toplevel list is refreshed.
    pub(crate) fn configure_frame(&mut self, window: &Window, id: crate::wm::WindowId) {
        let Some(r) = self.wm.rect(id) else { return };
        let (maximized, full) = (self.wm.is_maximized(id), self.wm.is_full_screen(id));
        if let Some(t) = window.toplevel() {
            t.with_pending_state(|s| {
                if maximized {
                    s.states.set(xdg_toplevel::State::Maximized);
                } else {
                    s.states.unset(xdg_toplevel::State::Maximized);
                }
                if full {
                    s.states.set(xdg_toplevel::State::Fullscreen);
                } else {
                    s.states.unset(xdg_toplevel::State::Fullscreen);
                }
                s.size = Some((r.w, r.h).into());
            });
            t.send_pending_configure();
        }
        self.place(window, (r.x, r.y));
        self.sync_toplevels();
    }

    /// Puts the window `id` on the whole output (`on`) or back to the frame it had. A request that changes nothing
    /// (already there, or a window `wm` does not know) still answers with a configure, so the client's pending
    /// request is settled.
    pub fn set_full_screen(&mut self, id: crate::wm::WindowId, on: bool) {
        let Some(window) = self.window(id) else { return };
        let changed = if on {
            if self.wm.is_full_screen(id) { None } else { self.wm.enter_full_screen(id) }
        } else {
            self.wm.leave_full_screen(id)
        };
        match changed {
            Some(_) => self.configure_frame(&window, id),
            None => {
                if let Some(t) = window.toplevel() {
                    t.send_configure();
                }
            }
        }
    }

    /// Applies a window-management choice to `id` (a tile, the centre, the way back) and resizes the client to match.
    /// The Ctrl+Super arrow keys and the shell's green-button menu both come through here.
    pub fn place_window(&mut self, id: crate::wm::WindowId, placement: crate::tiling::Placement) {
        let Some(window) = self.window(id) else { return };
        if self.wm.place(id, placement).is_some() {
            self.configure_frame(&window, id);
        }
    }

    /// `place_window` for the toplevel of `surface` (the shell names its windows by surface).
    pub fn place_surface(&mut self, surface: &WlSurface, placement: crate::tiling::Placement) {
        let Some(id) = self.window_for_surface(surface).as_ref().and_then(WmState::known_id) else { return };
        self.place_window(id, placement);
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
        if let Some(keyboard) = self.seat.get_keyboard() {
            if keyboard.current_focus().as_ref() == Some(surface.wl_surface()) {
                let serial = smithay::utils::SERIAL_COUNTER.next_serial();
                let top_surface = self.wm.focused().and_then(|id| self.window(id)).and_then(|w| w.wl_surface().map(|s| s.into_owned()));
                keyboard.set_focus(self, top_surface, serial);
            }
        }
        self.refresh_area();
    }
}

/// Whether `window` committed `committed` while a configure that resizes it is still unacknowledged: one whose size is
/// neither the size it was last configured to nor the size it just committed (`resize_in_flight`).
fn awaiting_resize(window: &Window, committed: (i32, i32)) -> bool {
    let Some(t) = window.toplevel() else { return false };
    with_states(t.wl_surface(), |states| {
        states.data_map.get::<XdgToplevelSurfaceData>().is_some_and(|d| {
            let d = d.lock().unwrap_or_else(|p| p.into_inner());
            let unacked: Vec<Option<(i32, i32)>> = d.pending_configures().iter().map(|c| c.state.size.map(|s| (s.w, s.h))).collect();
            resize_in_flight(d.current.size.map(|s| (s.w, s.h)), &unacked, committed)
        })
    })
}

/// Whether one of the `unacked` configure sizes asks the client for a new size (not the `current` configured one) that
/// its `committed` buffer does not have yet: that commit was drawn before the client saw the request (a maximize, a
/// centre), and taking it as the window's own frame would put the old size back into the pending state, to go out with
/// the next configure. A configure that only repeats the size (an activation change) never holds a commit back.
pub fn resize_in_flight(current: Option<(i32, i32)>, unacked: &[Option<(i32, i32)>], committed: (i32, i32)) -> bool {
    unacked.iter().any(|&size| size.is_some() && size != current && size != Some(committed))
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
smithay::delegate_presentation!(WmState);

#[cfg(feature = "drm")]
impl smithay::wayland::drm_syncobj::DrmSyncobjHandler for WmState {
    fn drm_syncobj_state(&mut self) -> Option<&mut smithay::wayland::drm_syncobj::DrmSyncobjState> {
        self.drm_syncobj_state.as_mut()
    }
}

#[cfg(feature = "drm")]
smithay::delegate_drm_syncobj!(WmState);
delegate_compositor!(WmState);
delegate_shm!(WmState);
delegate_seat!(WmState);
delegate_data_device!(WmState);
delegate_output!(WmState);
delegate_xdg_shell!(WmState);
delegate_layer_shell!(WmState);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_window_the_shell_stacks_takes_the_front_only_when_the_shell_asks() {
        assert!(map_takes_front(false, false, true), "an app's window, or no shell: every new window comes forward");
        assert!(map_takes_front(false, false, false));
        assert!(!map_takes_front(true, false, true), "the shell's window mapped behind the front one stays behind (W4-1)");
        assert!(map_takes_front(true, true, true), "the shell activated it before its first buffer");
        assert!(map_takes_front(true, false, false), "the first window on an empty desktop takes the keyboard");
    }

    /// W4-3: a commit drawn before the client saw a resize is not its own frame; one that only crosses an activation
    /// configure is.
    #[test]
    fn a_commit_older_than_a_resize_request_is_not_the_windows_own_frame() {
        let old = Some((923, 861));
        assert!(!resize_in_flight(old, &[], (800, 600)), "nothing in flight: the client chose its size");
        assert!(resize_in_flight(old, &[Some((2560, 1233))], (923, 861)), "a maximize not applied yet");
        assert!(!resize_in_flight(old, &[Some((2560, 1233))], (2560, 1233)), "the maximize applied before its ack");
        assert!(!resize_in_flight(old, &[old], (700, 500)), "an activation change repeats the size: the resize is the client's");
        assert!(!resize_in_flight(old, &[None], (700, 500)), "a configure without a size asks for none");
        assert!(resize_in_flight(old, &[old, Some((1280, 1233))], (923, 861)), "any resize in flight counts");
    }

    #[test]
    fn only_the_shell_takes_the_keyboard_exclusively_and_never_under_a_lock() {
        assert!(exclusive_keyboard_allowed(false, true));
        assert!(!exclusive_keyboard_allowed(false, false), "an app's overlay must not capture every keystroke");
        assert!(!exclusive_keyboard_allowed(true, true), "the lock surface keeps the keyboard");
        assert!(!exclusive_keyboard_allowed(true, false));
    }
}
