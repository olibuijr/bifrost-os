//! Screen capture readback for the GLES backends (DRM and headless): the `SessionHooks::capture` hook.
//!
//! A capture renders the output scene again into its own offscreen texture, with the element list the output itself
//! uses (`scene::build`: wallpaper layer, windows, layer surfaces, glass), then reads the requested region back. It runs
//! only when a client asked for a copy, from the request handler, so a quiet desktop pays nothing per frame. The scene
//! is built from the same `WmState` the next scanout frame is, so the copy shows what the screen shows. The texture is
//! Abgr8888: a little-endian word A, B, G, R, so memory is R, G, B, A, which is the order `rgba_to_shm` expects.

use std::error::Error;

use smithay::{
    backend::{
        allocator::Fourcc,
        renderer::{
            damage::OutputDamageTracker,
            element::{surface::WaylandSurfaceRenderElement, AsRenderElements},
            gles::{GlesRenderer, GlesTexture},
            Bind, ExportMem, Offscreen,
        },
    },
    desktop::Window,
    output::Output,
    utils::{Logical, Physical, Point, Rectangle, Scale, Transform},
    wayland::seat::WaylandFocus,
};

use crate::{
    bifrost_material::effective_regions,
    glass::{Glass, Plate},
    material_kind::{premultiply, recipe, MaterialSpec},
    scene::SceneElement,
    session_protocols::{clamp_region, CaptureSpec},
    state::WmState,
    wm::Rect,
};

/// The region to read from a texture of `size`, as `(x, y, w, h)`: the request clipped to the texture. `None` when
/// nothing is left. The texture is in the displayed orientation (`capture_size`), so no transform applies to it.
pub fn read_rect(size: (i32, i32), region: Rect) -> Option<(i32, i32, i32, i32)> {
    let r = clamp_region(size, region)?;
    Some((r.x, r.y, r.w, r.h))
}

/// Renders the scene of `output` (with `top` over it, the pointer when the request asks for it) into an offscreen
/// texture over `clear` and returns `spec.region` as RGBA rows, top first.
pub fn capture_scene(
    renderer: &mut GlesRenderer,
    glass: &mut Glass,
    state: &WmState,
    output: &Output,
    top: Vec<SceneElement>,
    clear: [f32; 4],
    spec: &CaptureSpec,
) -> Result<Vec<u8>, Box<dyn Error>> {
    let size = crate::transform::output_size(output).ok_or("output has no mode")?;
    let (x, y, w, h) = read_rect(size, spec.region.clone()).ok_or("empty capture region")?;
    if (w, h) != (spec.region.w, spec.region.h) {
        return Err("capture region leaves the output".into());
    }
    let elements = crate::scene::build(renderer, state, output, glass, top);
    let mut target: GlesTexture = renderer.create_buffer(Fourcc::Abgr8888, size.into())?;
    // the displayed orientation is rendered directly, so the output transform is Normal here and no rows are turned
    let mut damage = OutputDamageTracker::new(size, output.current_scale().fractional_scale(), Transform::Normal);
    let mut fb = renderer.bind(&mut target)?;
    damage.render_output(renderer, &mut fb, 0, &elements, clear)?;
    let mapping = renderer.copy_framebuffer(&fb, Rectangle::new((x, y).into(), (w, h).into()), Fourcc::Abgr8888)?;
    Ok(renderer.map_texture(&mapping)?.to_vec())
}

/// The flat plates a captured window shows where it declares material regions: a window picture is the window alone,
/// so a region that blurs the desktop on screen has nothing to blur here and is drawn as the recipe's opaque plate
/// instead (what an inactive window shows). `origin` is the surface origin in picture-logical px (the negated window
/// geometry offset), and `px_scale` the picture's pixels per logical pixel, for the corner radius. Pure.
pub fn window_plates(specs: &[MaterialSpec], origin: Point<i32, Logical>, dark: bool, px_scale: f32) -> Vec<Plate> {
    let mut plates = Vec::new();
    for spec in specs {
        let rc = recipe(spec.kind);
        let color = premultiply(if dark { rc.plate_dark } else { rc.plate_light });
        let radius = if spec.rects.len() == 1 { spec.mask_radius() * px_scale } else { 0.0 };
        plates.extend(spec.rects.iter().map(|r| Plate { rect: Rectangle::new(r.loc + origin, r.size), color, radius }));
    }
    plates
}

/// Renders `window` alone into a `size` picture (width, height) and returns it as RGBA rows, top first, premultiplied
/// over a transparent ground. The window's geometry is scaled to fill the picture: the elements are placed and the
/// damage tracker is set at the scale `size / geometry`, so the GPU does the reduction in the one render. Nothing that
/// overlaps the window on the screen is drawn; its material regions are flat plates (`window_plates`). Runs only for a
/// pending request or a window being minimized, never per frame.
pub fn capture_window(renderer: &mut GlesRenderer, glass: &Glass, state: &WmState, window: &Window, size: (i32, i32)) -> Result<Vec<u8>, Box<dyn Error>> {
    let geo = window.geometry();
    if geo.size.w <= 0 || geo.size.h <= 0 || size.0 <= 0 || size.1 <= 0 {
        return Err("empty window".into());
    }
    let scale: Scale<f64> = Scale::from((size.0 as f64 / geo.size.w as f64, size.1 as f64 / geo.size.h as f64));
    // the surface origin sits at minus the geometry offset, so the geometry's corner lands on the picture's
    let origin: Point<i32, Logical> = (-geo.loc.x, -geo.loc.y).into();
    let phys: Point<i32, Physical> = origin.to_physical_precise_round(scale);
    let surfaces: Vec<WaylandSurfaceRenderElement<GlesRenderer>> = window.render_elements(renderer, phys, scale, 1.0);
    let mut elements: Vec<SceneElement> = surfaces.into_iter().map(SceneElement::Surface).collect();
    if let Some(surface) = window.wl_surface() {
        let plates = window_plates(&effective_regions(&surface), origin, state.material_dark, scale.x.min(scale.y) as f32);
        elements.extend(glass.plates_once(&plates).into_iter().map(SceneElement::Plate));
    }
    let mut target: GlesTexture = renderer.create_buffer(Fourcc::Abgr8888, size.into())?;
    let mut damage = OutputDamageTracker::new(size, scale, Transform::Normal);
    let mut fb = renderer.bind(&mut target)?;
    damage.render_output(renderer, &mut fb, 0, &elements, [0.0f32; 4])?;
    let mapping = renderer.copy_framebuffer(&fb, Rectangle::new((0, 0).into(), (size.0, size.1).into()), Fourcc::Abgr8888)?;
    Ok(renderer.map_texture(&mapping)?.to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::material_kind::{Blending, MaterialKind, MaterialState};

    fn rect(x: i32, y: i32, w: i32, h: i32) -> Rectangle<i32, Logical> {
        Rectangle::new((x, y).into(), (w, h).into())
    }

    fn spec(kind: MaterialKind, rects: Vec<Rectangle<i32, Logical>>, radius: u32) -> MaterialSpec {
        MaterialSpec { rects, kind, state: MaterialState::FollowsWindow, blending: Blending::BehindWindow, emphasized: false, radii: [radius; 4] }
    }

    #[test]
    fn every_material_region_of_a_captured_window_is_an_opaque_plate_at_the_surface_origin() {
        let specs = [spec(MaterialKind::Sidebar, vec![rect(20, 20, 200, 600)], 10), spec(MaterialKind::Titlebar, vec![rect(220, 20, 780, 52)], 0)];
        for dark in [false, true] {
            let plates = window_plates(&specs, (-20, -20).into(), dark, 0.5);
            assert_eq!(plates.len(), 2, "one plate per rectangle (dark {dark})");
            assert_eq!(plates[0].rect, rect(0, 0, 200, 600), "the shadow margin offset is removed");
            assert_eq!(plates[1].rect, rect(200, 0, 780, 52));
            assert!(plates.iter().all(|p| p.color[3] == 1.0), "a plate is fully opaque (dark {dark})");
            assert_eq!(plates[0].radius, 5.0, "the corner radius follows the picture scale");
        }
    }

    #[test]
    fn a_material_of_several_rectangles_has_square_plates_and_none_means_no_plates() {
        let two = spec(MaterialKind::Sidebar, vec![rect(0, 0, 50, 50), rect(50, 0, 50, 50)], 8);
        let plates = window_plates(&[two], (0, 0).into(), true, 1.0);
        assert_eq!(plates.len(), 2);
        assert!(plates.iter().all(|p| p.radius == 0.0));
        assert!(window_plates(&[], (0, 0).into(), true, 1.0).is_empty(), "a window without materials draws only itself");
    }

    #[test]
    fn the_read_rect_is_the_request_clipped_to_the_texture() {
        assert_eq!(read_rect((100, 50), Rect::new(10, 5, 20, 10)), Some((10, 5, 20, 10)));
        assert_eq!(read_rect((100, 50), Rect::new(90, 40, 50, 50)), Some((90, 40, 10, 10)));
        assert_eq!(read_rect((100, 50), Rect::new(100, 0, 5, 5)), None);
    }
}
