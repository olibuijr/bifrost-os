//! Client side of `bifrost_window_level_v1` (wm/protocol/bifrost-window-level-v1.xml, docs/bifrost-ui.md "Window level"):
//! the shell tells the window server which toplevels float, when the compositor offers
//! `bifrost_window_level_manager_v1`. Without it every window stays on the normal level.

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
    use wayland_client;
    use wayland_client::protocol::*;

    pub mod __interfaces {
        use wayland_client::protocol::__interfaces::*;
        // The interface statics name `wayland_backend::protocol::*`; wayland-client re-exports that as `backend`.
        use wayland_client::backend as wayland_backend;
        wayland_scanner::generate_interfaces!("wm/protocol/bifrost-window-level-v1.xml");
    }
    use self::__interfaces::*;

    wayland_scanner::generate_client_code!("wm/protocol/bifrost-window-level-v1.xml");
}

/// The newest protocol version the shell binds: 2 adds `frame`, the window server's frame of each of the shell's
/// toplevels (the shell keeps its window rectangles from it).
pub const VERSION: u32 = 2;

/// The wire value of a window level: `true` is floating.
pub fn wire(floating: bool) -> u32 {
    floating as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shell_binds_the_version_that_has_the_frame_event() {
        let xml = include_str!("../../wm/protocol/bifrost-window-level-v1.xml");
        assert!(xml.contains(&format!("name=\"bifrost_window_level_manager_v1\" version=\"{VERSION}\"")));
        assert!(xml.contains("<event name=\"frame\" since=\"2\">"));
    }

    #[test]
    fn the_wire_values_are_the_protocol_levels() {
        assert_eq!((wire(false), wire(true)), (0, 1), "normal is 0 and floating is 1 in bifrost_window_level_v1");
    }
}
