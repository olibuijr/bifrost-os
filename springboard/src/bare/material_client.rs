//! Client side of `bifrost_material_v1` (wm/protocol/bifrost-material-v1.xml, docs/window-server.md "Materials"): the
//! shell's window surfaces declare each material region with its kind, state, blending, emphasis and per-corner radii
//! when the compositor offers `bifrost_material_manager_v1`; otherwise the Wayland backend falls back to
//! `ext-background-effect` blur regions (`wayland.rs`, `blur_rects`).
//!
//! The values come from the shared model (`bifrost_ui::visual_effect`, via `glass::Detail`), so a built-in window and
//! an external app's window reach the window server through one path.

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
        wayland_scanner::generate_interfaces!("wm/protocol/bifrost-material-v1.xml");
    }
    use self::__interfaces::*;

    wayland_scanner::generate_client_code!("wm/protocol/bifrost-material-v1.xml");
}

use bifrost_render::glass::{Kind, Rect as Glass};
use bifrost_ui::visual_effect::MaterialKind;

/// The arguments of one `add_region` for a glass rect (surface-local px; `x`/`y` offsets are the caller's).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Args {
    pub kind: u32,
    pub state: u32,
    pub blending: u32,
    pub emphasized: u32,
    /// top-left, top-right, bottom-right, bottom-left
    pub radii: [u32; 4],
}

/// The wire arguments for `g`: its full request when it carries one, else a request derived from its presenter kind.
pub fn args(g: &Glass) -> Args {
    let state = match g.active {
        None => 0,
        Some(true) => 1,
        Some(false) => 2,
    };
    let px = |r: f32| r.max(0.0).round() as u32;
    match g.detail {
        Some(d) => Args { kind: d.kind, state, blending: d.within_window as u32, emphasized: d.emphasized as u32, radii: d.radii.map(px) },
        None => {
            let kind = match g.kind {
                Kind::Sidebar => MaterialKind::Sidebar,
                Kind::Titlebar => MaterialKind::Titlebar,
                Kind::Content => MaterialKind::WindowBackground,
                Kind::Chrome | Kind::Menu => MaterialKind::Menu,
            };
            Args { kind: kind.window_server_wire(), state, blending: 0, emphasized: 0, radii: [px(g.r); 4] }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_render::glass::Detail;

    #[test]
    fn a_detailed_rect_sends_its_full_request() {
        let d = Detail { kind: MaterialKind::HudWindow.window_server_wire(), within_window: true, emphasized: true, radii: [1.2, 2.0, 3.6, 0.0] };
        let g = Glass { x: 0.0, y: 0.0, w: 10.0, h: 10.0, r: 3.6, kind: Kind::Menu, active: Some(false), detail: Some(d), tint: [0.0; 4] };
        assert_eq!(args(&g), Args { kind: 7, state: 2, blending: 1, emphasized: 1, radii: [1, 2, 4, 0] });
    }

    #[test]
    fn a_plain_rect_derives_its_kind_from_the_presenter_kind() {
        for (k, want) in [(Kind::Sidebar, MaterialKind::Sidebar), (Kind::Titlebar, MaterialKind::Titlebar), (Kind::Content, MaterialKind::WindowBackground), (Kind::Chrome, MaterialKind::Menu), (Kind::Menu, MaterialKind::Menu)] {
            let g = Glass { x: 0.0, y: 0.0, w: 1.0, h: 1.0, r: 5.0, kind: k, active: None, detail: None, tint: [0.0; 4] };
            let a = args(&g);
            assert_eq!((a.kind, a.state, a.radii), (want.window_server_wire(), 0, [5; 4]), "{k:?}");
        }
    }
}
