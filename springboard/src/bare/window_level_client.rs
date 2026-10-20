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
/// toplevels (the shell keeps its window rectangles from it); 3 adds `tile`, the shell's request to place one; 4 adds
/// `set_origin`, where a toplevel maps again after the shell destroyed it while the window was hidden.
pub const VERSION: u32 = 4;
/// The version that introduced `tile`; a manager bound at an older version is never sent one.
pub const TILE_SINCE: u32 = 3;
/// The version that introduced `set_origin`; a manager bound at an older version is never sent one.
pub const ORIGIN_SINCE: u32 = 4;

/// The wire value of a placement in `bifrost_window_level_v1` `tile` (the `placement` enum).
pub fn placement_wire(p: Placement) -> u32 {
    match p {
        Placement::Fill => 0,
        Placement::Left => 1,
        Placement::Right => 2,
        Placement::Top => 3,
        Placement::Bottom => 4,
        Placement::TopLeft => 5,
        Placement::TopRight => 6,
        Placement::BottomLeft => 7,
        Placement::BottomRight => 8,
        Placement::Centre => 9,
        Placement::Previous => 10,
    }
}

/// What the shell can ask the window server to do with one of its toplevels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Placement { Fill, Left, Right, Top, Bottom, TopLeft, TopRight, BottomLeft, BottomRight, Centre, Previous }

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
    fn the_shell_asks_for_the_tile_request_the_protocol_declares() {
        let xml = include_str!("../../wm/protocol/bifrost-window-level-v1.xml");
        assert!(xml.contains(&format!("<request name=\"tile\" since=\"{TILE_SINCE}\">")));
        assert!(TILE_SINCE <= VERSION);
    }

    #[test]
    fn the_shell_asks_for_the_origin_request_the_protocol_declares() {
        let xml = include_str!("../../wm/protocol/bifrost-window-level-v1.xml");
        assert!(xml.contains(&format!("<request name=\"set_origin\" since=\"{ORIGIN_SINCE}\">")));
        assert!(ORIGIN_SINCE <= VERSION);
    }

    #[test]
    fn the_placement_wire_values_are_the_protocol_values() {
        let xml = include_str!("../../wm/protocol/bifrost-window-level-v1.xml");
        use Placement::*;
        for (p, name) in [
            (Fill, "fill"), (Left, "left"), (Right, "right"), (Top, "top"), (Bottom, "bottom"), (TopLeft, "top_left"),
            (TopRight, "top_right"), (BottomLeft, "bottom_left"), (BottomRight, "bottom_right"), (Centre, "centre"),
            (Previous, "previous"),
        ] {
            assert!(xml.contains(&format!("<entry name=\"{name}\" value=\"{}\"", placement_wire(p))), "{name}");
        }
    }

    #[test]
    fn the_wire_values_are_the_protocol_levels() {
        assert_eq!((wire(false), wire(true)), (0, 1), "normal is 0 and floating is 1 in bifrost_window_level_v1");
    }
}
