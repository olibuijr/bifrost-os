//! `bifrost_window_level_v1` (protocol/bifrost-window-level-v1.xml, docs/bifrost-ui.md "Window level"): the shell puts a
//! toplevel on the floating level, which the window model (`wm::Level`) keeps above the normal windows. The level is
//! kept on the surface, so a window mapped later takes it too.
//!
//! Version 2 adds the way back. The window server places, moves and resizes toplevels and xdg-shell never tells a
//! client where, so it reports each frame (`frame`) to the client that owns the toplevel. The shell keeps its window
//! rectangles from these reports instead of its own guess, so what it places on the screen from them (context menus,
//! the share menu, taps) lands where the window really is.

#[allow(
    dead_code,
    non_camel_case_types,
    unused_unsafe,
    unused_variables,
    non_upper_case_globals,
    non_snake_case,
    unused_imports,
    missing_docs,
    clippy::all
)]
pub mod protocol {
    use wayland_server;
    use wayland_server::protocol::*;

    pub mod __interfaces {
        use wayland_server::protocol::__interfaces::*;
        wayland_scanner::generate_interfaces!("protocol/bifrost-window-level-v1.xml");
    }
    use self::__interfaces::*;

    wayland_scanner::generate_server_code!("protocol/bifrost-window-level-v1.xml");
}

use std::{collections::HashMap, sync::Mutex};

use smithay::{
    reexports::wayland_server::{
        backend::GlobalId, protocol::wl_surface::WlSurface, Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New,
        Resource,
    },
    wayland::compositor::with_states,
};

use self::protocol::bifrost_window_level_manager_v1::{self, BifrostWindowLevelManagerV1};
use crate::{
    privileged::is_privileged,
    state::WmState,
    wm::{Level, Rect, WindowId},
};

/// The protocol version the global offers: 2 adds the `frame` event, 3 the `tile` request, 4 `set_origin`.
pub const VERSION: u32 = 4;
/// The version that introduced `frame`; a manager bound at an older version is never sent one.
pub const FRAME_SINCE: u32 = 2;
/// The version that introduced `tile`.
pub const TILE_SINCE: u32 = 3;
/// The version that introduced `set_origin`.
pub const ORIGIN_SINCE: u32 = 4;

/// Most managers one client may keep bound: each one is a reporter that `report_frames` walks, so a client that binds in
/// a loop would otherwise make every frame report cost more. The shell binds once.
pub const MAX_MANAGERS_PER_CLIENT: usize = 4;

/// What a request of the manager asks for, for the policy below.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LevelRequest {
    /// `set_level` to this level.
    SetLevel(Level),
    /// `set_origin`: where the toplevel maps (the window model keeps at least part of it on the usable area).
    SetOrigin,
    /// `tile`: a placement of a mapped toplevel.
    Tile,
}

/// Which requests a client may make. Every request names a surface of the requesting client only (a client holds no
/// other client's `wl_surface`), so the policy is about what a window may do to the desktop: an ordinary client may drop
/// its windows to the normal level and say where a window maps (`wm::map_at` keeps it on the usable area), but only a
/// privileged client (the shell) may raise a window to the floating level, which stays above every normal window and
/// could cover the menu bar, the Dock or another app's permission dialog, or have the window server place it.
pub fn request_allowed(privileged: bool, request: LevelRequest) -> bool {
    match request {
        LevelRequest::SetLevel(Level::Normal) | LevelRequest::SetOrigin => true,
        LevelRequest::SetLevel(_) | LevelRequest::Tile => privileged,
    }
}

pub fn init(dh: &DisplayHandle) -> GlobalId {
    dh.create_global::<WmState, BifrostWindowLevelManagerV1, ()>(VERSION, ())
}

/// The toplevels in `now` the client was never told about, or was told about with another frame than `sent` holds.
pub fn changed_frames(sent: &HashMap<WindowId, Rect>, now: &[(WindowId, Rect)]) -> Vec<(WindowId, Rect)> {
    now.iter().filter(|(id, rect)| sent.get(id) != Some(rect)).copied().collect()
}

/// One bound manager and the frame it was last told for each toplevel.
struct Reporter {
    manager: BifrostWindowLevelManagerV1,
    sent: HashMap<WindowId, Rect>,
}

/// Every bound `bifrost_window_level_manager_v1`, so a change of a toplevel's frame reaches the client that owns it.
#[derive(Default)]
pub struct FrameReports {
    reporters: Vec<Reporter>,
}

impl FrameReports {
    pub fn add(&mut self, manager: BifrostWindowLevelManagerV1) {
        self.reporters.push(Reporter { manager, sent: HashMap::new() });
    }

    pub fn is_empty(&self) -> bool {
        self.reporters.is_empty()
    }

    /// How many live managers the client of `manager` already has registered.
    pub fn managers_of_client(&self, manager: &BifrostWindowLevelManagerV1) -> usize {
        self.reporters.iter().filter(|r| r.manager.is_alive() && r.manager.id().same_client_as(&manager.id())).count()
    }

    /// Sends each manager the frames that changed since it was last told, for its own client's toplevels only (a
    /// client never learns where another client's windows are). `now` is every mapped toplevel with its frame and
    /// surface. A toplevel that is gone or minimized is forgotten, so it is reported again when it is shown.
    pub fn send(&mut self, now: &[(WindowId, Rect, WlSurface)]) {
        self.reporters.retain(|r| r.manager.is_alive());
        for reporter in &mut self.reporters {
            if reporter.manager.version() < FRAME_SINCE {
                continue;
            }
            let mine: Vec<(WindowId, Rect)> = now
                .iter()
                .filter(|(_, _, surface)| reporter.manager.id().same_client_as(&surface.id()))
                .map(|(id, rect, _)| (*id, *rect))
                .collect();
            for (id, rect) in changed_frames(&reporter.sent, &mine) {
                if let Some((_, _, surface)) = now.iter().find(|(other, _, _)| *other == id) {
                    reporter.manager.frame(surface, rect.x, rect.y, rect.w, rect.h);
                }
                reporter.sent.insert(id, rect);
            }
            reporter.sent.retain(|id, _| mine.iter().any(|(other, _)| other == id));
        }
    }
}

/// The level the shell asked for `surface`; normal when it asked for none.
pub fn level(surface: &WlSurface) -> Level {
    with_states(surface, |s| s.data_map.get::<Mutex<Level>>().map(|l| *l.lock().unwrap_or_else(|p| p.into_inner())).unwrap_or_default())
}

/// The position the shell asked for the toplevel of `surface` to map at (`set_origin`), if it asked.
pub fn origin(surface: &WlSurface) -> Option<(i32, i32)> {
    with_states(surface, |s| s.data_map.get::<Mutex<Option<(i32, i32)>>>().and_then(|o| *o.lock().unwrap_or_else(|p| p.into_inner())))
}

/// The level of a wire value (`bifrost_window_level_manager_v1.set_level`), or `None` for an unknown one.
pub fn from_wire(v: u32) -> Option<Level> {
    match v {
        0 => Some(Level::Normal),
        1 => Some(Level::Floating),
        _ => None,
    }
}

impl GlobalDispatch<BifrostWindowLevelManagerV1, ()> for WmState {
    fn bind(state: &mut Self, _: &DisplayHandle, _: &Client, resource: New<BifrostWindowLevelManagerV1>, _: &(), init: &mut DataInit<'_, Self>) {
        let manager = init.init(resource, ());
        // past the cap the object still works for requests but is never sent frames
        if state.frame_reports.managers_of_client(&manager) >= MAX_MANAGERS_PER_CLIENT {
            return;
        }
        state.frame_reports.add(manager);
        // a client that binds after its windows mapped hears their frames now
        state.report_frames();
    }
}

impl Dispatch<BifrostWindowLevelManagerV1, ()> for WmState {
    fn request(
        state: &mut Self,
        client: &Client,
        manager: &BifrostWindowLevelManagerV1,
        request: bifrost_window_level_manager_v1::Request,
        _: &(),
        _: &DisplayHandle,
        _: &mut DataInit<'_, Self>,
    ) {
        let privileged = is_privileged(client);
        match request {
            bifrost_window_level_manager_v1::Request::SetLevel { surface, level } => {
                let Some(level) = from_wire(level) else {
                    manager.post_error(bifrost_window_level_manager_v1::Error::InvalidLevel, "unknown window level");
                    return;
                };
                if !request_allowed(privileged, LevelRequest::SetLevel(level)) {
                    return;
                }
                with_states(&surface, |s| {
                    s.data_map.insert_if_missing_threadsafe(|| Mutex::new(Level::Normal));
                    if let Some(l) = s.data_map.get::<Mutex<Level>>() {
                        *l.lock().unwrap_or_else(|p| p.into_inner()) = level;
                    }
                });
                state.set_window_level(&surface, level);
            }
            bifrost_window_level_manager_v1::Request::SetOrigin { surface, x, y } => {
                if !request_allowed(privileged, LevelRequest::SetOrigin) {
                    return;
                }
                with_states(&surface, |s| {
                    s.data_map.insert_if_missing_threadsafe(|| Mutex::new(None::<(i32, i32)>));
                    if let Some(o) = s.data_map.get::<Mutex<Option<(i32, i32)>>>() {
                        *o.lock().unwrap_or_else(|p| p.into_inner()) = Some((x, y));
                    }
                });
            }
            bifrost_window_level_manager_v1::Request::Tile { surface, placement } => {
                let Some(placement) = crate::tiling::placement_from_wire(placement) else {
                    manager.post_error(bifrost_window_level_manager_v1::Error::InvalidPlacement, "unknown window placement");
                    return;
                };
                if !request_allowed(privileged, LevelRequest::Tile) {
                    return;
                }
                state.place_surface(&surface, placement);
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: i32, y: i32) -> Rect {
        Rect::new(x, y, 640, 480)
    }

    #[test]
    fn a_toplevel_the_client_was_never_told_about_is_reported() {
        let sent = HashMap::new();
        assert_eq!(changed_frames(&sent, &[(1, rect(10, 20))]), vec![(1, rect(10, 20))]);
    }

    #[test]
    fn an_unchanged_frame_is_not_reported_again() {
        let sent = HashMap::from([(1, rect(10, 20)), (2, rect(30, 40))]);
        assert!(changed_frames(&sent, &[(1, rect(10, 20)), (2, rect(30, 40))]).is_empty());
    }

    #[test]
    fn a_moved_or_resized_toplevel_is_reported_alone() {
        let sent = HashMap::from([(1, rect(10, 20)), (2, rect(30, 40))]);
        let moved = [(1, rect(10, 20)), (2, rect(52, 19))];
        assert_eq!(changed_frames(&sent, &moved), vec![(2, rect(52, 19))], "only the moved window");
        let grown = [(1, Rect::new(10, 20, 700, 480)), (2, rect(30, 40))];
        assert_eq!(changed_frames(&sent, &grown), vec![(1, Rect::new(10, 20, 700, 480))], "a size change counts");
    }

    #[test]
    fn the_protocol_file_declares_the_version_and_the_event_the_server_sends() {
        let xml = include_str!("../protocol/bifrost-window-level-v1.xml");
        assert!(xml.contains(&format!("name=\"bifrost_window_level_manager_v1\" version=\"{VERSION}\"")));
        assert!(xml.contains(&format!("<event name=\"frame\" since=\"{FRAME_SINCE}\">")));
        assert!(xml.contains(&format!("<request name=\"tile\" since=\"{TILE_SINCE}\">")));
        assert!(xml.contains(&format!("<request name=\"set_origin\" since=\"{ORIGIN_SINCE}\">")));
        assert!(FRAME_SINCE <= TILE_SINCE && TILE_SINCE <= ORIGIN_SINCE && ORIGIN_SINCE <= VERSION);
    }

    #[test]
    fn the_protocol_placements_match_the_wire_values_the_window_server_decodes() {
        let xml = include_str!("../protocol/bifrost-window-level-v1.xml");
        for (name, value) in [
            ("fill", 0), ("left", 1), ("right", 2), ("top", 3), ("bottom", 4), ("top_left", 5), ("top_right", 6),
            ("bottom_left", 7), ("bottom_right", 8), ("centre", 9), ("previous", 10),
        ] {
            assert!(xml.contains(&format!("<entry name=\"{name}\" value=\"{value}\"")), "{name} = {value}");
            assert!(crate::tiling::placement_from_wire(value).is_some());
        }
    }

    #[test]
    fn an_ordinary_client_may_not_float_or_tile_but_may_drop_to_normal() {
        assert!(!request_allowed(false, LevelRequest::SetLevel(Level::Floating)));
        assert!(!request_allowed(false, LevelRequest::Tile));
        assert!(request_allowed(false, LevelRequest::SetLevel(Level::Normal)));
        assert!(request_allowed(false, LevelRequest::SetOrigin));
    }

    #[test]
    fn the_shell_may_make_every_request() {
        for request in [LevelRequest::SetLevel(Level::Normal), LevelRequest::SetLevel(Level::Floating), LevelRequest::SetOrigin, LevelRequest::Tile] {
            assert!(request_allowed(true, request), "{request:?}");
        }
    }

    #[test]
    fn the_manager_cap_leaves_room_for_the_shell_and_stops_a_loop() {
        assert!(MAX_MANAGERS_PER_CLIENT >= 1 && MAX_MANAGERS_PER_CLIENT <= 8);
    }

    #[test]
    fn the_wire_values_are_the_levels() {
        assert_eq!(from_wire(0), Some(Level::Normal));
        assert_eq!(from_wire(1), Some(Level::Floating));
        assert_eq!(from_wire(2), None);
    }
}
