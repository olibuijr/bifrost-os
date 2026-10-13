//! `ext-background-effect-v1`: clients mark the region of a surface whose background the compositor blurs. The region
//! is double-buffered (applied on the surface's commit). The renderer reads `blur_region` for each surface it draws.

use std::sync::Mutex;

use smithay::{
    reexports::{
        wayland_protocols::ext::background_effect::v1::server::{
            ext_background_effect_manager_v1::{self, Capability, ExtBackgroundEffectManagerV1},
            ext_background_effect_surface_v1::{self, ExtBackgroundEffectSurfaceV1},
        },
        wayland_server::{
            backend::GlobalId, protocol::wl_surface::WlSurface, Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New,
            Resource, Weak,
        },
    },
    utils::{Logical, Rectangle},
    wayland::compositor::{get_region_attributes, with_states, RectangleKind},
};

use crate::state::WmState;

/// The blur region of a surface in surface coordinates: the committed one, and one waiting for the next commit.
#[derive(Default)]
struct Blur {
    pending: Option<Vec<Rectangle<i32, Logical>>>,
    current: Vec<Rectangle<i32, Logical>>,
    /// The surface already has an effect object (a second one is a protocol error).
    claimed: bool,
}

pub fn init(dh: &DisplayHandle) -> GlobalId {
    dh.create_global::<WmState, ExtBackgroundEffectManagerV1, ()>(1, ())
}

/// The committed blur region of `surface` (surface coordinates), empty when it asks for none.
pub fn blur_region(surface: &WlSurface) -> Vec<Rectangle<i32, Logical>> {
    with_states(surface, |s| s.data_map.get::<Mutex<Blur>>().map(|b| b.lock().unwrap_or_else(|p| p.into_inner()).current.clone()).unwrap_or_default())
}

/// Called on every surface commit: a pending region becomes the current one.
pub fn commit(surface: &WlSurface) {
    with_states(surface, |s| {
        if let Some(b) = s.data_map.get::<Mutex<Blur>>() {
            let mut b = b.lock().unwrap_or_else(|p| p.into_inner());
            if let Some(p) = b.pending.take() {
                b.current = p;
            }
        }
    });
}

/// A `wl_region` as plain rectangles: the added ones, with every subtracted rectangle cut out.
///
/// A client controls the input, so it is bounded (WM-6): at most `MAX_INPUT` rectangles are read, rectangles whose
/// far edge overflows are dropped (WM-17), and a result of more than `MAX_RECTS` pieces becomes its bounding box.
pub fn resolve(rects: &[(RectangleKind, Rectangle<i32, Logical>)]) -> Vec<Rectangle<i32, Logical>> {
    const MAX_INPUT: usize = 1024;
    const MAX_RECTS: usize = 16;
    // while resolving, the list may fragment up to here before it is collapsed (keeps the cuts cheap)
    const WORKING: usize = 256;
    let mut out: Vec<Rectangle<i32, Logical>> = Vec::new();
    for &(kind, r) in rects.iter().take(MAX_INPUT) {
        if r.size.w <= 0 || r.size.h <= 0 || r.loc.x.checked_add(r.size.w).is_none() || r.loc.y.checked_add(r.size.h).is_none() {
            continue;
        }
        if out.len() > WORKING {
            out = bounding(&out);
        }
        match kind {
            // only the part not already covered is added, so the list never overlaps
            RectangleKind::Add => {
                let mut parts = vec![r];
                for have in &out {
                    parts = parts.into_iter().flat_map(|p| cut(p, *have)).collect();
                }
                out.extend(parts);
            }
            RectangleKind::Subtract => out = out.into_iter().flat_map(|p| cut(p, r)).collect(),
        }
    }
    if out.len() > MAX_RECTS {
        out = bounding(&out);
    }
    out
}

/// The bounding box of `rects` as a one-element list (empty for an empty list).
fn bounding(rects: &[Rectangle<i32, Logical>]) -> Vec<Rectangle<i32, Logical>> {
    let Some(first) = rects.first() else { return Vec::new() };
    let all = rects.iter().fold(*first, |a, b| a.merge(*b));
    vec![all]
}

/// `a` without `b`: up to four rectangles (above, below, left and right of the overlap).
fn cut(a: Rectangle<i32, Logical>, b: Rectangle<i32, Logical>) -> Vec<Rectangle<i32, Logical>> {
    let Some(i) = a.intersection(b) else { return vec![a] };
    let (ax1, ay1) = (a.loc.x + a.size.w, a.loc.y + a.size.h);
    let (ix1, iy1) = (i.loc.x + i.size.w, i.loc.y + i.size.h);
    let rect = |x0: i32, y0: i32, x1: i32, y1: i32| Rectangle::new((x0, y0).into(), (x1 - x0, y1 - y0).into());
    [
        rect(a.loc.x, a.loc.y, ax1, i.loc.y),
        rect(a.loc.x, iy1, ax1, ay1),
        rect(a.loc.x, i.loc.y, i.loc.x, iy1),
        rect(ix1, i.loc.y, ax1, iy1),
    ]
    .into_iter()
    .filter(|r| r.size.w > 0 && r.size.h > 0)
    .collect()
}

impl GlobalDispatch<ExtBackgroundEffectManagerV1, ()> for WmState {
    fn bind(_: &mut Self, _: &DisplayHandle, _: &Client, resource: New<ExtBackgroundEffectManagerV1>, _: &(), init: &mut DataInit<'_, Self>) {
        let manager = init.init(resource, ());
        manager.capabilities(Capability::Blur);
    }
}

impl Dispatch<ExtBackgroundEffectManagerV1, ()> for WmState {
    fn request(_: &mut Self, _: &Client, manager: &ExtBackgroundEffectManagerV1, request: ext_background_effect_manager_v1::Request, _: &(), _: &DisplayHandle, init: &mut DataInit<'_, Self>) {
        match request {
            ext_background_effect_manager_v1::Request::GetBackgroundEffect { id, surface } => {
                let taken = with_states(&surface, |s| {
                    s.data_map.insert_if_missing_threadsafe(|| Mutex::new(Blur::default()));
                    let mut b = s.data_map.get::<Mutex<Blur>>().unwrap().lock().unwrap_or_else(|p| p.into_inner());
                    std::mem::replace(&mut b.claimed, true)
                });
                if taken {
                    manager.post_error(ext_background_effect_manager_v1::Error::BackgroundEffectExists, "the surface already has a background effect");
                    return;
                }
                init.init(id, surface.downgrade());
            }
            ext_background_effect_manager_v1::Request::Destroy => {}
            _ => {}
        }
    }
}

impl Dispatch<ExtBackgroundEffectSurfaceV1, Weak<WlSurface>> for WmState {
    fn request(_: &mut Self, _: &Client, effect: &ExtBackgroundEffectSurfaceV1, request: ext_background_effect_surface_v1::Request, surface: &Weak<WlSurface>, _: &DisplayHandle, _: &mut DataInit<'_, Self>) {
        let Ok(surface) = surface.upgrade() else {
            effect.post_error(ext_background_effect_surface_v1::Error::SurfaceDestroyed, "the surface is gone");
            return;
        };
        let region = match &request {
            ext_background_effect_surface_v1::Request::SetBlurRegion { region } => {
                Some(region.as_ref().map(|r| resolve(&get_region_attributes(r).rects)).unwrap_or_default())
            }
            // the effect goes away with its object: no blur from the next commit on
            ext_background_effect_surface_v1::Request::Destroy => Some(Vec::new()),
            _ => None,
        };
        with_states(&surface, |s| {
            if let Some(b) = s.data_map.get::<Mutex<Blur>>() {
                let mut b = b.lock().unwrap_or_else(|p| p.into_inner());
                if region.is_some() {
                    b.pending = region;
                }
                if matches!(request, ext_background_effect_surface_v1::Request::Destroy) {
                    b.claimed = false;
                }
            }
        });
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    fn r(x: i32, y: i32, w: i32, h: i32) -> Rectangle<i32, Logical> { Rectangle::new((x, y).into(), (w, h).into()) }
    fn area(v: &[Rectangle<i32, Logical>]) -> i32 { v.iter().map(|r| r.size.w * r.size.h).sum() }

    #[test]
    fn added_rectangles_pass_through() {
        assert_eq!(resolve(&[(RectangleKind::Add, r(0, 0, 100, 50))]), vec![r(0, 0, 100, 50)]);
        assert!(resolve(&[]).is_empty());
    }

    #[test]
    fn a_subtracted_hole_leaves_the_frame_around_it() {
        // a 100x100 sidebar with a 20x20 hole in the middle: four strips, no overlap, nothing inside the hole
        let out = resolve(&[(RectangleKind::Add, r(0, 0, 100, 100)), (RectangleKind::Subtract, r(40, 40, 20, 20))]);
        assert_eq!(area(&out), 100 * 100 - 20 * 20);
        assert!(out.iter().all(|x| !x.overlaps(r(40, 40, 20, 20))));
    }

    #[test]
    fn order_matters_an_add_after_a_subtract_comes_back() {
        let out = resolve(&[
            (RectangleKind::Add, r(0, 0, 100, 100)),
            (RectangleKind::Subtract, r(0, 0, 100, 50)),
            (RectangleKind::Add, r(0, 0, 10, 10)),
        ]);
        assert_eq!(area(&out), 100 * 50 + 10 * 10);
    }

    #[test]
    fn a_flood_of_rectangles_is_bounded() {
        let many: Vec<_> = (0..20_000).map(|i| (RectangleKind::Add, r((i % 200) * 20, (i / 200) * 20, 10, 10))).collect();
        let t = std::time::Instant::now();
        let out = resolve(&many);
        assert!(out.len() <= 16, "{} rects", out.len());
        assert!(t.elapsed() < std::time::Duration::from_millis(200));
    }

    #[test]
    fn a_rectangle_that_overflows_is_dropped() {
        assert!(resolve(&[(RectangleKind::Add, r(i32::MAX, 0, 1, 1))]).is_empty());
    }

    #[test]
    fn subtracting_everything_leaves_nothing() {
        assert!(resolve(&[(RectangleKind::Add, r(0, 0, 10, 10)), (RectangleKind::Subtract, r(-5, -5, 30, 30))]).is_empty());
    }
}
