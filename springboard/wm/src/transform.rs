//! Output rotation: the pure geometry between the desktop clients see (logical) and the panel's own pixels (physical).
//!
//! The transform follows the Wayland output convention, the one Smithay's `Output` carries: `_90` shows the desktop
//! turned 90 degrees counter-clockwise, so a 1200x1920 portrait panel with `_90` gives a 1920x1200 landscape desktop
//! whose top edge runs along the panel's left edge. `_270` is the other landscape mounting. The flipped variants
//! mirror first and then rotate. Which of `_90` and `_270` suits a device depends on how the panel is built in, so
//! the choice is configuration: `BIFROST_ROTATION`, else the first line of `/etc/bifrost/rotation`.
//!
//! Everything here is plain arithmetic on sizes, points and rectangles. The window server gives the transform to the
//! `Output`, so the DRM compositor, the space, the layer map and the clients' `wl_output` and xdg-output all follow
//! it; touch and other absolute input use `physical_to_logical` to land in the same desktop.
//!
//! Points are mapped the way rectangles are (a point is an empty rectangle), because the DRM compositor places
//! elements with the rectangle mapping; the two then agree for the mirrored 90 and 270 degree variants too.

use smithay::{
    output::Output,
    utils::{Logical, Physical, Rectangle, Size, Transform},
};

use crate::wm::Rect;

/// The configuration file read when `BIFROST_ROTATION` is not set.
pub const FILE: &str = "/etc/bifrost/rotation";
/// The environment variable that overrides the file.
pub const ENV: &str = "BIFROST_ROTATION";

/// Reads a transform name: `normal` or `0`, `90`, `180`, `270`, `flipped`, `flipped-90`, `flipped-180`,
/// `flipped-270`. Case, `-`, `_` and spaces are ignored, and a `rotate` prefix or `deg` suffix is accepted
/// (`rotate-90`, `270deg`).
pub fn parse(text: &str) -> Option<Transform> {
    let mut s: String = text.trim().to_ascii_lowercase().chars().filter(|c| !matches!(c, '-' | '_' | ' ')).collect();
    if let Some(rest) = s.strip_prefix("rotate") {
        s = rest.to_string();
    }
    if let Some(rest) = s.strip_suffix("deg") {
        s = rest.to_string();
    }
    Some(match s.as_str() {
        "normal" | "none" | "0" => Transform::Normal,
        "90" => Transform::_90,
        "180" => Transform::_180,
        "270" => Transform::_270,
        "flipped" | "flipped0" => Transform::Flipped,
        "flipped90" => Transform::Flipped90,
        "flipped180" => Transform::Flipped180,
        "flipped270" => Transform::Flipped270,
        _ => return None,
    })
}

/// The transform to use: the environment value when it names one, else the file's first line, else `Normal`.
pub fn resolve(env: Option<&str>, file: Option<&str>) -> Transform {
    env.and_then(parse).or_else(|| file.and_then(|f| parse(f.lines().next().unwrap_or("")))).unwrap_or(Transform::Normal)
}

/// `resolve` on the real environment and file. A value that names no transform is reported and skipped.
pub fn load() -> Transform {
    let env = std::env::var(ENV).ok();
    let file = std::fs::read_to_string(FILE).ok();
    for (what, value) in [(ENV, env.as_deref()), (FILE, file.as_deref().map(|f| f.lines().next().unwrap_or("")))] {
        if let Some(v) = value.filter(|v| !v.trim().is_empty()) {
            if parse(v).is_none() {
                eprintln!("bifrost-wm: {what}: unknown rotation {:?}, ignored", v.trim());
            }
        }
    }
    resolve(env.as_deref(), file.as_deref())
}

/// The desktop size clients see for a panel of `physical` pixels: swapped for 90 and 270 degrees.
pub fn logical_size(physical: (i32, i32), t: Transform) -> (i32, i32) {
    let s = t.transform_size(Size::<i32, Physical>::from(physical));
    (s.w, s.h)
}

/// The panel size for a desktop of `logical` pixels (the same swap, so it is its own inverse).
pub fn physical_size(logical: (i32, i32), t: Transform) -> (i32, i32) {
    logical_size(logical, t)
}

/// The size of `output`'s desktop as clients see it: its current mode turned by its current transform. `None` before
/// the output has a mode.
pub fn output_size(output: &Output) -> Option<(i32, i32)> {
    output.current_mode().map(|m| logical_size((m.size.w, m.size.h), output.current_transform()))
}

/// The transform that takes a desktop plane onto the panel. The output transform is specified as the rotation of the
/// desktop, so the render transform is its inverse (Smithay's DRM compositor does the same).
fn to_panel(t: Transform) -> Transform {
    t.invert()
}

/// The transform that takes the panel plane back to the desktop: the inverse of `to_panel`. Rotations invert by
/// swapping 90 and 270; every mirrored transform is its own inverse.
fn to_desktop(t: Transform) -> Transform {
    if t.flipped() { to_panel(t) } else { t }
}

/// An empty rectangle at `p` through `t` inside `area`: the point's image.
fn map_point(p: (f64, f64), t: Transform, area: (f64, f64)) -> (f64, f64) {
    let r = Rectangle::<f64, Logical>::new(p.into(), (0.0, 0.0).into());
    let area = Size::<f64, Logical>::from(area);
    let out = t.transform_rect_in(r, &area);
    (out.loc.x, out.loc.y)
}

/// A point of the desktop on the panel. `physical` is the panel's size in pixels.
pub fn logical_to_physical(p: (f64, f64), t: Transform, physical: (i32, i32)) -> (f64, f64) {
    let (lw, lh) = logical_size(physical, t);
    map_point(p, to_panel(t), (lw as f64, lh as f64))
}

/// A point of the panel in the desktop: where a touch at panel position `p` lands. `physical` is the panel's size.
pub fn physical_to_logical(p: (f64, f64), t: Transform, physical: (i32, i32)) -> (f64, f64) {
    map_point(p, to_desktop(t), (physical.0 as f64, physical.1 as f64))
}

/// A position given as a fraction of the panel (0..1 on each panel axis, as an absolute input device reports it) in
/// desktop pixels.
pub fn normalized_to_logical(n: (f64, f64), t: Transform, physical: (i32, i32)) -> (f64, f64) {
    physical_to_logical((n.0 * physical.0 as f64, n.1 * physical.1 as f64), t, physical)
}

/// A rectangle of the desktop on the panel (damage, a capture region, a window frame).
pub fn logical_rect_to_physical(r: Rect, t: Transform, physical: (i32, i32)) -> Rect {
    let (lw, lh) = logical_size(physical, t);
    from_rect(to_panel(t).transform_rect_in(to_smithay(r), &Size::<i32, Logical>::from((lw, lh))))
}

/// A rectangle of the panel in the desktop.
pub fn physical_rect_to_logical(r: Rect, t: Transform, physical: (i32, i32)) -> Rect {
    from_rect(to_desktop(t).transform_rect_in(to_smithay(r), &Size::<i32, Logical>::from(physical)))
}

fn to_smithay(r: Rect) -> Rectangle<i32, Logical> {
    Rectangle::new((r.x, r.y).into(), (r.w, r.h).into())
}

fn from_rect(r: Rectangle<i32, Logical>) -> Rect {
    Rect::new(r.loc.x, r.loc.y, r.size.w, r.size.h)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: [Transform; 8] = [
        Transform::Normal,
        Transform::_90,
        Transform::_180,
        Transform::_270,
        Transform::Flipped,
        Transform::Flipped90,
        Transform::Flipped180,
        Transform::Flipped270,
    ];
    /// The tablet panel, portrait.
    const PANEL: (i32, i32) = (1200, 1920);

    #[test]
    fn names_parse_in_the_forms_a_config_file_uses() {
        assert_eq!(parse("normal"), Some(Transform::Normal));
        assert_eq!(parse("0"), Some(Transform::Normal));
        assert_eq!(parse(" 90\n"), Some(Transform::_90));
        assert_eq!(parse("Rotate-180"), Some(Transform::_180));
        assert_eq!(parse("270deg"), Some(Transform::_270));
        assert_eq!(parse("flipped"), Some(Transform::Flipped));
        assert_eq!(parse("flipped-90"), Some(Transform::Flipped90));
        assert_eq!(parse("FLIPPED_270"), Some(Transform::Flipped270));
        assert_eq!(parse("45"), None);
        assert_eq!(parse(""), None);
    }

    #[test]
    fn the_environment_wins_over_the_file_and_bad_values_fall_through() {
        assert_eq!(resolve(Some("90"), Some("270\n")), Transform::_90);
        assert_eq!(resolve(None, Some("270\n# comment")), Transform::_270);
        assert_eq!(resolve(Some("sideways"), Some("180")), Transform::_180);
        assert_eq!(resolve(None, Some("sideways")), Transform::Normal);
        assert_eq!(resolve(None, None), Transform::Normal);
    }

    #[test]
    fn a_portrait_panel_turned_90_degrees_is_a_landscape_desktop() {
        assert_eq!(logical_size(PANEL, Transform::_90), (1920, 1200));
        assert_eq!(logical_size(PANEL, Transform::_270), (1920, 1200));
        assert_eq!(logical_size(PANEL, Transform::Flipped90), (1920, 1200));
        assert_eq!(logical_size(PANEL, Transform::Normal), PANEL);
        assert_eq!(logical_size(PANEL, Transform::_180), PANEL);
        assert_eq!(logical_size(PANEL, Transform::Flipped), PANEL);
        assert_eq!(physical_size((1920, 1200), Transform::_90), PANEL);
    }

    #[test]
    fn the_desktop_corners_land_on_the_panel_corners_for_90_degrees() {
        let t = Transform::_90;
        // the desktop's top edge runs along the panel's left edge
        assert_eq!(logical_to_physical((0.0, 0.0), t, PANEL), (0.0, 1920.0));
        assert_eq!(logical_to_physical((1920.0, 0.0), t, PANEL), (0.0, 0.0));
        assert_eq!(logical_to_physical((1920.0, 1200.0), t, PANEL), (1200.0, 0.0));
        assert_eq!(logical_to_physical((0.0, 1200.0), t, PANEL), (1200.0, 1920.0));
        // and a touch at the panel's top-left is the desktop's top-right
        assert_eq!(physical_to_logical((0.0, 0.0), t, PANEL), (1920.0, 0.0));
        assert_eq!(physical_to_logical((0.0, 1920.0), t, PANEL), (0.0, 0.0));
    }

    #[test]
    fn the_desktop_corners_land_on_the_panel_corners_for_270_and_180_degrees() {
        assert_eq!(logical_to_physical((0.0, 0.0), Transform::_270, PANEL), (1200.0, 0.0));
        assert_eq!(logical_to_physical((1920.0, 0.0), Transform::_270, PANEL), (1200.0, 1920.0));
        assert_eq!(logical_to_physical((0.0, 0.0), Transform::_180, PANEL), (1200.0, 1920.0));
        assert_eq!(logical_to_physical((1200.0, 1920.0), Transform::_180, PANEL), (0.0, 0.0));
        assert_eq!(logical_to_physical((0.0, 0.0), Transform::Normal, PANEL), (0.0, 0.0));
    }

    #[test]
    fn a_flipped_transform_mirrors_the_desktop() {
        assert_eq!(logical_to_physical((0.0, 0.0), Transform::Flipped, PANEL), (1200.0, 0.0));
        assert_eq!(logical_to_physical((0.0, 0.0), Transform::Flipped180, PANEL), (0.0, 1920.0));
    }

    #[test]
    fn points_round_trip_for_every_transform() {
        for t in ALL {
            let (lw, lh) = logical_size(PANEL, t);
            for p in [(0.0, 0.0), (lw as f64, lh as f64), (123.5, 77.25), (lw as f64 - 1.0, 3.0)] {
                let q = logical_to_physical(p, t, PANEL);
                assert!(q.0 >= 0.0 && q.0 <= PANEL.0 as f64 && q.1 >= 0.0 && q.1 <= PANEL.1 as f64, "{t:?} {p:?} -> {q:?} leaves the panel");
                assert_eq!(physical_to_logical(q, t, PANEL), p, "{t:?} {p:?}");
            }
        }
    }

    #[test]
    fn rectangles_round_trip_and_keep_their_area_for_every_transform() {
        let r = Rect::new(10, 20, 300, 40);
        for t in ALL {
            let phys = logical_rect_to_physical(r, t, PANEL);
            assert_eq!(phys.w * phys.h, r.w * r.h, "{t:?}");
            assert!(phys.x >= 0 && phys.y >= 0 && phys.x + phys.w <= PANEL.0 && phys.y + phys.h <= PANEL.1, "{t:?} {phys:?}");
            assert_eq!(physical_rect_to_logical(phys, t, PANEL), r, "{t:?}");
        }
    }

    #[test]
    fn a_rectangle_at_the_desktop_top_left_sits_at_the_panel_bottom_left_for_90_degrees() {
        let phys = logical_rect_to_physical(Rect::new(0, 0, 100, 50), Transform::_90, PANEL);
        assert_eq!(phys, Rect::new(0, 1820, 50, 100));
    }

    #[test]
    fn a_rectangle_agrees_with_its_corner_points() {
        for t in ALL {
            let r = Rect::new(40, 30, 200, 100);
            let phys = logical_rect_to_physical(r, t, PANEL);
            let a = logical_to_physical((r.x as f64, r.y as f64), t, PANEL);
            let b = logical_to_physical(((r.x + r.w) as f64, (r.y + r.h) as f64), t, PANEL);
            let (x0, x1) = (a.0.min(b.0), a.0.max(b.0));
            let (y0, y1) = (a.1.min(b.1), a.1.max(b.1));
            assert_eq!((phys.x as f64, phys.y as f64, (phys.x + phys.w) as f64, (phys.y + phys.h) as f64), (x0, y0, x1, y1), "{t:?}");
        }
    }

    #[test]
    fn a_normalized_touch_position_scales_to_the_panel_and_then_the_desktop() {
        assert_eq!(normalized_to_logical((0.5, 0.25), Transform::_90, PANEL), (1440.0, 600.0));
        assert_eq!(normalized_to_logical((0.5, 0.5), Transform::Normal, PANEL), (600.0, 960.0));
    }

    #[test]
    fn an_output_reports_the_rotated_size() {
        use smithay::output::{Mode, PhysicalProperties, Subpixel};
        let output = Output::new(
            "DSI-1".into(),
            PhysicalProperties { size: (0, 0).into(), subpixel: Subpixel::Unknown, make: "t".into(), model: "t".into() },
        );
        assert_eq!(output_size(&output), None);
        output.change_current_state(Some(Mode { size: PANEL.into(), refresh: 60_000 }), Some(Transform::_90), None, None);
        assert_eq!(output_size(&output), Some((1920, 1200)));
    }
}
