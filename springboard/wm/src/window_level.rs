//! `bifrost_window_level_v1` (protocol/bifrost-window-level-v1.xml, docs/bifrost-ui.md "Window level"): the shell puts a
//! toplevel on the floating level, which the window model (`wm::Level`) keeps above the normal windows. The level is
//! kept on the surface, so a window mapped later takes it too.

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

use std::sync::Mutex;

use smithay::{
    reexports::wayland_server::{
        backend::GlobalId, protocol::wl_surface::WlSurface, Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New,
        Resource,
    },
    wayland::compositor::with_states,
};

use self::protocol::bifrost_window_level_manager_v1::{self, BifrostWindowLevelManagerV1};
use crate::{state::WmState, wm::Level};

pub fn init(dh: &DisplayHandle) -> GlobalId {
    dh.create_global::<WmState, BifrostWindowLevelManagerV1, ()>(1, ())
}

/// The level the shell asked for `surface`; normal when it asked for none.
pub fn level(surface: &WlSurface) -> Level {
    with_states(surface, |s| s.data_map.get::<Mutex<Level>>().map(|l| *l.lock().unwrap_or_else(|p| p.into_inner())).unwrap_or_default())
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
    fn bind(_: &mut Self, _: &DisplayHandle, _: &Client, resource: New<BifrostWindowLevelManagerV1>, _: &(), init: &mut DataInit<'_, Self>) {
        init.init(resource, ());
    }
}

impl Dispatch<BifrostWindowLevelManagerV1, ()> for WmState {
    fn request(
        state: &mut Self,
        _: &Client,
        manager: &BifrostWindowLevelManagerV1,
        request: bifrost_window_level_manager_v1::Request,
        _: &(),
        _: &DisplayHandle,
        _: &mut DataInit<'_, Self>,
    ) {
        if let bifrost_window_level_manager_v1::Request::SetLevel { surface, level } = request {
            let Some(level) = from_wire(level) else {
                manager.post_error(bifrost_window_level_manager_v1::Error::InvalidLevel, "unknown window level");
                return;
            };
            with_states(&surface, |s| {
                s.data_map.insert_if_missing_threadsafe(|| Mutex::new(Level::Normal));
                if let Some(l) = s.data_map.get::<Mutex<Level>>() {
                    *l.lock().unwrap_or_else(|p| p.into_inner()) = level;
                }
            });
            state.set_window_level(&surface, level);
        }
    }
}
