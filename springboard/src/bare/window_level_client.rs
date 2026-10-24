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
    use wayland_protocols_wlr::foreign_toplevel::v1::client::*;

    pub mod __interfaces {
        use wayland_client::protocol::__interfaces::*;
        // The interface statics name `wayland_backend::protocol::*`; wayland-client re-exports that as `backend`.
        use wayland_client::backend as wayland_backend;
        use wayland_protocols_wlr::foreign_toplevel::v1::client::__interfaces::*;
        wayland_scanner::generate_interfaces!("wm/protocol/bifrost-window-level-v1.xml");
    }
    use self::__interfaces::*;

    wayland_scanner::generate_client_code!("wm/protocol/bifrost-window-level-v1.xml");
}

/// The newest protocol version the shell binds: 2 adds `frame`, the window server's frame of each of the shell's
/// toplevels (the shell keeps its window rectangles from it); 3 adds `tile`, the shell's request to place one; 4 adds
/// `set_origin`, where a toplevel maps again after the shell destroyed it while the window was hidden; 5 adds
/// `set_white_point` and `white_point_mode`, Night Shift applied by the window server to the whole output; 6 adds
/// `set_zoom`, Screen Zoom of the whole output; 7 adds `tile_toplevel`, the shell's request to place another client's
/// window (a native app's), named by its foreign toplevel handle.
pub const VERSION: u32 = 7;
/// The version that introduced `tile_toplevel`; a manager bound at an older version is never sent one.
pub const TILE_TOPLEVEL_SINCE: u32 = 7;
/// The version that introduced `tile`; a manager bound at an older version is never sent one.
pub const TILE_SINCE: u32 = 3;
/// The version that introduced `set_origin`; a manager bound at an older version is never sent one.
pub const ORIGIN_SINCE: u32 = 4;
/// The version that introduced `set_white_point` and the `white_point_mode` event.
pub const WHITE_POINT_SINCE: u32 = 5;
/// The version that introduced `set_zoom`.
pub const ZOOM_SINCE: u32 = 6;

/// The `set_zoom` argument for the shell's Screen Zoom: the Full Screen factor (256 = 1.0), 256 when zoom is off or in
/// Picture in Picture (the lens stays with the shell's presenter).
pub fn zoom_wire(z: crate::shell::accessibility::zoom::ScreenZoom) -> u32 {
    use crate::shell::accessibility::zoom::{Mode, MAX_FACTOR, MIN_FACTOR};
    if !z.on || z.mode != Mode::Full || z.factor <= MIN_FACTOR {
        return 256;
    }
    (z.factor.min(MAX_FACTOR) * 256.0).round() as u32
}

/// The `set_white_point` arguments for the Night Shift multipliers `m` (`night_shift::multipliers`, 256 = 1.0, in R, G,
/// B order): off is 256 256 256, and nothing above 1.0 is sent.
pub fn white_point_wire(m: Option<[u32; 3]>) -> [u32; 3] {
    m.map_or([256; 3], |m| m.map(|v| v.min(256)))
}

/// Whether the window server applies the white point itself for a `white_point_mode` value (1 the display's gamma
/// ramp, 2 its compositor pass); 0, or a window server without the event, leaves the tint to the shell.
pub fn wm_tints(mode: u32) -> bool {
    mode != 0
}

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
    fn the_shell_asks_for_the_zoom_the_protocol_declares() {
        let xml = include_str!("../../wm/protocol/bifrost-window-level-v1.xml");
        assert!(xml.contains(&format!("<request name=\"set_zoom\" since=\"{ZOOM_SINCE}\">")));
        assert!(ZOOM_SINCE <= VERSION);
    }

    #[test]
    fn the_shell_asks_for_the_foreign_tile_request_the_protocol_declares() {
        let xml = include_str!("../../wm/protocol/bifrost-window-level-v1.xml");
        assert!(xml.contains(&format!("<request name=\"tile_toplevel\" since=\"{TILE_TOPLEVEL_SINCE}\">")));
        assert!(xml.contains("<arg name=\"toplevel\" type=\"object\" interface=\"zwlr_foreign_toplevel_handle_v1\"/>"));
        assert!(TILE_SINCE <= TILE_TOPLEVEL_SINCE && TILE_TOPLEVEL_SINCE <= VERSION);
    }

    #[test]
    fn the_zoom_wire_value_is_the_full_screen_factor_only() {
        use crate::shell::accessibility::zoom::{Mode, ScreenZoom};
        assert_eq!(zoom_wire(ScreenZoom::default()), 256, "off");
        assert_eq!(zoom_wire(ScreenZoom { on: true, factor: 2.0, mode: Mode::Full }), 512);
        assert_eq!(zoom_wire(ScreenZoom { on: true, factor: 1.25, mode: Mode::Full }), 320);
        assert_eq!(zoom_wire(ScreenZoom { on: true, factor: 2.0, mode: Mode::Lens }), 256, "the lens stays with the shell");
        assert_eq!(zoom_wire(ScreenZoom { on: true, factor: 99.0, mode: Mode::Full }), 2048);
    }

    #[test]
    fn the_shell_asks_for_the_white_point_the_protocol_declares() {
        let xml = include_str!("../../wm/protocol/bifrost-window-level-v1.xml");
        assert!(xml.contains(&format!("<request name=\"set_white_point\" since=\"{WHITE_POINT_SINCE}\">")));
        assert!(xml.contains(&format!("<event name=\"white_point_mode\" since=\"{WHITE_POINT_SINCE}\">")));
        assert!(WHITE_POINT_SINCE <= VERSION);
        for (name, value, tints) in [("none", 0, false), ("hardware", 1, true), ("composited", 2, true)] {
            assert!(xml.contains(&format!("<entry name=\"{name}\" value=\"{value}\"")), "{name}");
            assert_eq!(wm_tints(value), tints, "{name}");
        }
    }

    #[test]
    fn the_white_point_goes_out_on_the_wire_scale() {
        assert_eq!(white_point_wire(None), [256; 3], "off is white");
        assert_eq!(white_point_wire(Some([256, 200, 100])), [256, 200, 100]);
        assert_eq!(white_point_wire(Some([300, 256, 0])), [256, 256, 0], "never brighter than white");
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
