//! XWayland and the X11 window manager (`docs/graphics/g2-wm-xwayland.md`).
//!
//! The compositor starts an XWayland server, runs an X11 window manager on its X connection and maps X11 windows into
//! the same `Space` and `wm` stack as Wayland toplevels. Managed X11 windows take the frame `wm` gives them; override-
//! redirect windows (menus, tooltips) are mapped at the position they ask for, above the managed windows.
//! Clients started by the compositor get `DISPLAY` once XWayland is ready.

use std::{
    process::Stdio,
    sync::OnceLock,
};

use smithay::{
    delegate_xwayland_shell,
    desktop::Window,
    input::pointer::{Focus, GrabStartData},
    reexports::{
        calloop::LoopHandle,
        wayland_server::{Client, DisplayHandle},
    },
    utils::{Logical, Rectangle, SERIAL_COUNTER},
    wayland::xwayland_shell::{XWaylandShellHandler, XWaylandShellState},
    xwayland::{
        xwm::{ResizeEdge, XwmId},
        X11Surface, X11Wm, XWayland, XWaylandClientData, XWaylandEvent, XwmHandler,
    },
};

use crate::{
    grabs::{MoveGrab, ResizeGrab},
    state::WmState,
    wm::{Edges, Rect},
};

/// The `DISPLAY` clients get (`:N`), set once XWayland reports its display number.
static DISPLAY: OnceLock<String> = OnceLock::new();

/// The display clients spawned by the compositor should use, once XWayland is ready.
pub fn display() -> Option<&'static str> { DISPLAY.get().map(String::as_str) }

/// XWayland's state on the compositor: the xwayland-shell global, the XWayland client and the running X11 window manager.
pub struct XWaylandState {
    pub shell: XWaylandShellState,
    pub client: Option<Client>,
    pub wm: Option<X11Wm>,
}

impl XWaylandState {
    /// The xwayland-shell global exists from the start; XWayland binds it once it connects.
    pub fn new(dh: &DisplayHandle) -> Self {
        XWaylandState { shell: XWaylandShellState::new::<WmState>(dh), client: None, wm: None }
    }
}

/// Starts XWayland and, once it is ready, the X11 window manager. `on_ready` runs once the display is known, or at once
/// when XWayland is disabled (`BIFROST_XWAYLAND=0`) or cannot start, so clients still launch.
pub fn start(handle: &LoopHandle<'static, WmState>, state: &mut WmState, on_ready: Box<dyn FnOnce()>) {
    if std::env::var("BIFROST_XWAYLAND").is_ok_and(|v| v == "0") {
        on_ready();
        return;
    }
    let spawned = XWayland::spawn(
        &state.dh,
        None,
        std::iter::empty::<(&str, &str)>(),
        true,
        Stdio::null(),
        Stdio::null(),
        |_| {},
    );
    let (xwayland, client) = match spawned {
        Ok(pair) => pair,
        Err(e) => {
            eprintln!("bifrost-wm: XWayland unavailable: {e}");
            on_ready();
            return;
        }
    };
    state.xwayland.client = Some(client);
    let loop_handle = handle.clone();
    let mut on_ready = Some(on_ready);
    let inserted = handle.insert_source(xwayland, move |event, _, state: &mut WmState| match event {
        XWaylandEvent::Ready { x11_socket, display_number } => {
            let _ = DISPLAY.set(format!(":{display_number}"));
            let Some(client) = state.xwayland.client.clone() else { return };
            match X11Wm::start_wm(loop_handle.clone(), x11_socket, client) {
                Ok(wm) => state.xwayland.wm = Some(wm),
                Err(e) => eprintln!("bifrost-wm: X11 window manager: {e}"),
            }
            eprintln!("bifrost-wm: XWayland on DISPLAY={}", DISPLAY.get().map_or("?", String::as_str));
            if let Some(run) = on_ready.take() {
                run();
            }
        }
        XWaylandEvent::Error => {
            eprintln!("bifrost-wm: XWayland exited during startup");
            if let Some(run) = on_ready.take() {
                run();
            }
        }
    });
    if let Err(e) = inserted {
        eprintln!("bifrost-wm: XWayland source: {e}");
    }
}

/// `wm` rectangle as a Smithay rectangle.
fn geo(r: Rect) -> Rectangle<i32, Logical> { Rectangle::new((r.x, r.y).into(), (r.w, r.h).into()) }

/// The edges a resize from an X11 edge moves.
fn edges_of(e: ResizeEdge) -> Edges {
    let (left, right, top, bottom) = match e {
        ResizeEdge::Top => (false, false, true, false),
        ResizeEdge::Bottom => (false, false, false, true),
        ResizeEdge::Left => (true, false, false, false),
        ResizeEdge::Right => (false, true, false, false),
        ResizeEdge::TopLeft => (true, false, true, false),
        ResizeEdge::TopRight => (false, true, true, false),
        ResizeEdge::BottomLeft => (true, false, false, true),
        ResizeEdge::BottomRight => (false, true, false, true),
    };
    Edges { left, right, top, bottom }
}

impl WmState {
    /// The mapped window that wraps `surface`, if any.
    fn window_for_x11(&self, surface: &X11Surface) -> Option<Window> {
        self.space.elements().find(|w| w.x11_surface().is_some_and(|x| x.window_id() == surface.window_id())).cloned()
    }

    /// An X11 window went away: `wm` forgets it, and focus moves to the next window in the stack.
    fn forget_x11(&mut self, surface: &X11Surface) {
        let Some(window) = self.window_for_x11(surface) else { return };
        if let Some(id) = WmState::known_id(&window) {
            self.wm.unmap(id);
            self.forget_window(id, &window);
        }
        self.space.unmap_elem(&window);
        self.focus(None, SERIAL_COUNTER.next_serial());
    }
}

impl XwmHandler for WmState {
    fn xwm_state(&mut self, _xwm: XwmId) -> &mut X11Wm {
        self.xwayland.wm.as_mut().expect("the X11 window manager runs while XWayland does")
    }

    fn new_window(&mut self, _xwm: XwmId, _window: X11Surface) {}

    fn new_override_redirect_window(&mut self, _xwm: XwmId, _window: X11Surface) {}

    /// A managed X11 window asks to be shown: `wm` places it like a new toplevel, then it is raised and focused.
    fn map_window_request(&mut self, _xwm: XwmId, surface: X11Surface) {
        if surface.set_mapped(true).is_err() {
            return;
        }
        let size = surface.geometry().size;
        let window = Window::new_x11_window(surface.clone());
        let id = self.id_of(&window);
        let r = self.wm.map(id, size.w.max(1), size.h.max(1));
        let _ = surface.configure(geo(r));
        self.space.map_element(window, (r.x, r.y), false);
        self.focus(Some(id), SERIAL_COUNTER.next_serial());
    }

    /// Override-redirect windows (menus, tooltips, combo boxes) are mapped at the position they ask for, above the
    /// managed windows.
    fn mapped_override_redirect_window(&mut self, _xwm: XwmId, surface: X11Surface) {
        let loc = surface.geometry().loc;
        let window = match self.window_for_x11(&surface) {
            Some(w) => w,
            None => Window::new_x11_window(surface),
        };
        self.space.map_element(window, loc, false);
    }

    fn unmapped_window(&mut self, _xwm: XwmId, surface: X11Surface) { self.forget_x11(&surface); }

    fn destroyed_window(&mut self, _xwm: XwmId, surface: X11Surface) { self.forget_x11(&surface); }

    /// Managed windows get the frame `wm` holds for them. Override-redirect windows place themselves, so their
    /// requests are granted by XWayland and not repeated here.
    fn configure_request(
        &mut self,
        _xwm: XwmId,
        surface: X11Surface,
        x: Option<i32>,
        y: Option<i32>,
        w: Option<u32>,
        h: Option<u32>,
        _reorder: Option<smithay::xwayland::xwm::Reorder>,
    ) {
        if surface.is_override_redirect() {
            return;
        }
        let current = surface.geometry();
        let asked = Rectangle::new(
            (x.unwrap_or(current.loc.x), y.unwrap_or(current.loc.y)).into(),
            (w.map_or(current.size.w, |v| v as i32), h.map_or(current.size.h, |v| v as i32)).into(),
        );
        let frame = self
            .window_for_x11(&surface)
            .and_then(|win| WmState::known_id(&win))
            .and_then(|id| self.wm.rect(id));
        let _ = surface.configure(frame.map_or(asked, geo));
    }

    /// Override-redirect windows move themselves; their new place is applied to the space. A managed window that
    /// resized itself keeps its anchor and takes the new size as its frame.
    fn configure_notify(&mut self, _xwm: XwmId, surface: X11Surface, geometry: Rectangle<i32, Logical>, _above: Option<u32>) {
        let Some(window) = self.window_for_x11(&surface) else { return };
        if surface.is_override_redirect() {
            self.place(&window, geometry.loc);
            return;
        }
        let Some(id) = WmState::known_id(&window) else { return };
        let Some(frame) = self.wm.rect(id) else { return };
        if (geometry.size.w, geometry.size.h) != (frame.w, frame.h) && geometry.size.w > 0 && geometry.size.h > 0 {
            let r = Rect::new(frame.x, frame.y, geometry.size.w, geometry.size.h);
            self.wm.set_frame(id, r);
            self.place(&window, (r.x, r.y));
        }
    }

    /// A move from the X11 window's title area: a pointer grab that moves its frame, as for a Wayland toplevel.
    fn move_request(&mut self, _xwm: XwmId, surface: X11Surface, button: u32) {
        let Some(window) = self.window_for_x11(&surface) else { return };
        let Some(id) = WmState::known_id(&window) else { return };
        let Some(pointer) = self.seat.get_pointer() else { return };
        let location = pointer.current_location();
        let Some(grab) = self.wm.begin_move(id, location.x as i32, location.y as i32) else { return };
        let start = GrabStartData { focus: None, button, location };
        pointer.set_grab(self, MoveGrab { start, id, grab }, SERIAL_COUNTER.next_serial(), Focus::Clear);
    }

    /// A resize from an X11 window edge: a pointer grab that resizes its frame, as for a Wayland toplevel.
    fn resize_request(&mut self, _xwm: XwmId, surface: X11Surface, button: u32, resize_edge: ResizeEdge) {
        let Some(window) = self.window_for_x11(&surface) else { return };
        let Some(id) = WmState::known_id(&window) else { return };
        let Some(pointer) = self.seat.get_pointer() else { return };
        let location = pointer.current_location();
        let edges = edges_of(resize_edge);
        let Some(grab) = self.wm.begin_resize(id, edges, location.x as i32, location.y as i32) else { return };
        let start = GrabStartData { focus: None, button, location };
        pointer.set_grab(self, ResizeGrab { start, id, grab, edges, window }, SERIAL_COUNTER.next_serial(), Focus::Clear);
    }

    fn disconnected(&mut self, _xwm: XwmId) {
        eprintln!("bifrost-wm: X11 window manager disconnected");
        self.xwayland.wm = None;
    }
}

impl XWaylandShellHandler for WmState {
    fn xwayland_shell_state(&mut self) -> &mut XWaylandShellState { &mut self.xwayland.shell }
}

/// The XWayland client's compositor state, for the compositor handler (its client data is not `state::ClientState`).
pub fn client_compositor_state(client: &Client) -> Option<&smithay::wayland::compositor::CompositorClientState> {
    client.get_data::<XWaylandClientData>().map(|d| &d.compositor_state)
}

delegate_xwayland_shell!(WmState);

#[cfg(test)]
mod tests {
    use super::*;

    fn sides(e: ResizeEdge) -> (bool, bool, bool, bool) {
        let m = edges_of(e);
        (m.left, m.right, m.top, m.bottom)
    }

    #[test]
    fn geo_keeps_position_and_size() {
        let r = geo(Rect::new(3, -4, 100, 50));
        assert_eq!((r.loc.x, r.loc.y), (3, -4));
        assert_eq!((r.size.w, r.size.h), (100, 50));
    }

    #[test]
    fn side_edges_move_one_side() {
        assert_eq!(sides(ResizeEdge::Top), (false, false, true, false));
        assert_eq!(sides(ResizeEdge::Bottom), (false, false, false, true));
        assert_eq!(sides(ResizeEdge::Left), (true, false, false, false));
        assert_eq!(sides(ResizeEdge::Right), (false, true, false, false));
    }

    #[test]
    fn corner_edges_move_two_sides() {
        assert_eq!(sides(ResizeEdge::TopLeft), (true, false, true, false));
        assert_eq!(sides(ResizeEdge::TopRight), (false, true, true, false));
        assert_eq!(sides(ResizeEdge::BottomLeft), (true, false, false, true));
        assert_eq!(sides(ResizeEdge::BottomRight), (false, true, false, true));
    }

    #[test]
    fn every_edge_moves_one_or_two_sides_never_opposite_ones() {
        let all = [
            ResizeEdge::Top,
            ResizeEdge::Bottom,
            ResizeEdge::Left,
            ResizeEdge::Right,
            ResizeEdge::TopLeft,
            ResizeEdge::TopRight,
            ResizeEdge::BottomLeft,
            ResizeEdge::BottomRight,
        ];
        for e in all {
            let (l, r, t, b) = sides(e);
            let moved = [l, r, t, b].iter().filter(|s| **s).count();
            assert!(moved == 1 || moved == 2, "{e:?} moves {moved} sides");
            assert!(!(l && r) && !(t && b), "{e:?} moves opposite sides");
        }
    }

    #[test]
    fn display_is_unset_before_xwayland_is_ready() {
        // Nothing sets DISPLAY in the unit tests: clients spawned before XWayland is ready get none.
        assert_eq!(display(), None);
    }

    #[test]
    fn a_top_left_resize_keeps_the_bottom_right_corner() {
        let m = edges_of(ResizeEdge::TopLeft);
        assert!(m.left && m.top && !m.right && !m.bottom);
    }
}
