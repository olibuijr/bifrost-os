//! The composited scene of one output, front to back, shared by every backend: layer surfaces, windows in stacking
//! order, and the live glass backdrop right under every surface that asks for blur.

use smithay::{
    backend::renderer::{
        element::{
            memory::MemoryRenderBufferRenderElement,
            surface::{render_elements_from_surface_tree, WaylandSurfaceRenderElement},
            AsRenderElements, Kind,
        },
        gles::{element::PixelShaderElement, GlesRenderer},
    },
    desktop::{layer_map_for_output, Window},
    output::Output,
    reexports::wayland_server::{protocol::wl_surface::WlSurface, Resource},
    render_elements,
    utils::{Logical, Physical, Point, Rectangle, Scale},
    wayland::shell::wlr_layer::Layer,
};

use crate::{
    bifrost_material::effective_regions,
    glass::{Glass, GlassElement, Plate, Region, SATURATION},
    material_filter::Tone,
    material_kind::{premultiply, resolve, MaterialSpec, Resolved},
    state::WmState,
};

render_elements! {
    pub SceneElement<=GlesRenderer>;
    Surface = WaylandSurfaceRenderElement<GlesRenderer>,
    Glass = GlassElement,
    Plate = PixelShaderElement,
    Memory = MemoryRenderBufferRenderElement<GlesRenderer>,
}

/// One surface of the scene: its elements and what sits behind its material regions on the output.
struct Item {
    key: u64,
    elements: Vec<SceneElement>,
    blur: Vec<Region>,
    /// Saturation of the backdrop from the material recipe of the first blurred region.
    saturation: f32,
    /// Brightness and luminance mapping from the same recipe.
    tone: Tone,
    /// The regions that show a flat opaque plate instead of a blur (an inactive window's sidebar and toolbar).
    plates: Vec<Plate>,
}

/// What the window server draws behind one surface's material regions (output logical px).
struct Backing {
    /// The regions that blur what lies below them, each with its corner radius.
    blur: Vec<Region>,
    /// The recipe's saturation of the first blurred region.
    saturation: f32,
    /// Brightness and luminance mapping from the same recipe.
    tone: Tone,
    /// The regions whose material resolves to a flat plate: an inactive window's materials and within-window ones.
    /// The client leaves those regions transparent (or under a faint tint) for the window server to back, so a region
    /// with neither a blur nor a plate would show the windows behind it through the window.
    plates: Vec<Plate>,
}

/// The backing of `specs` (a surface's material regions) with the surface's origin at `origin` (pure). A blur region
/// and a plate take the material's corner radius when the material is one rectangle (the window's radius, which the
/// shell sends for every region, so nothing shows past the window's rounded corners), like the Vulkan path's
/// `material_nodes`; a plate takes the recipe's opaque colour.
fn backing_of(specs: &[MaterialSpec], origin: Point<i32, Logical>, active: bool, dark: bool) -> Backing {
    let mut blur = Vec::new();
    let mut plates = Vec::new();
    let mut look = None;
    for spec in specs {
        let rects = spec.rects.iter().map(|r| Rectangle::new(r.loc + origin, r.size));
        let radius = if spec.rects.len() == 1 { spec.mask_radius() } else { 0.0 };
        match resolve(spec, active, dark) {
            Resolved::Glass { saturation, tone, .. } => {
                look.get_or_insert((saturation, tone));
                blur.extend(rects.map(|rect| Region { rect, radius }));
            }
            Resolved::Plate(c) => {
                let color = premultiply(c);
                plates.extend(rects.map(|rect| Plate { rect, color, radius }));
            }
        }
    }
    let (saturation, tone) = look.unwrap_or((SATURATION, Tone::NONE));
    Backing { blur, saturation, tone, plates }
}

/// The backing of `surface`'s material regions: `backing_of` its committed materials, else its plain blur regions.
fn backing_at(surface: &WlSurface, origin: Point<i32, Logical>, active: bool, dark: bool) -> Backing {
    backing_of(&effective_regions(surface), origin, active, dark)
}

/// A stable key for a surface's glass between frames.
fn surface_key(surface: &WlSurface) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    surface.id().hash(&mut h);
    h.finish()
}

/// The surfaces of one layer, bottom first (the map lists them top first).
fn layer_items(renderer: &mut GlesRenderer, map: &smithay::desktop::LayerMap, layer: Layer, scale: Scale<f64>, dark: bool, out: &mut Vec<Item>) {
    let mut ls: Vec<_> = map.layers_on(layer).collect();
    ls.reverse();
    for l in ls {
        let Some(geo) = map.layer_geometry(l) else { continue };
        let loc: Point<i32, Physical> = geo.loc.to_physical_precise_round(scale);
        let elements = render_elements_from_surface_tree(renderer, l.wl_surface(), loc, scale, 1.0, Kind::Unspecified)
            .into_iter()
            .map(SceneElement::Surface)
            .collect();
        let Backing { blur, saturation, tone, plates } = backing_at(l.wl_surface(), geo.loc, true, dark);
        out.push(Item { key: surface_key(l.wl_surface()), elements, blur, saturation, tone, plates });
    }
}

/// Bottom-first list of the output's surfaces.
fn items(renderer: &mut GlesRenderer, state: &WmState, output: &Output, scale: Scale<f64>) -> Vec<Item> {
    let mut out = Vec::new();
    let map = layer_map_for_output(output);
    layer_items(renderer, &map, Layer::Background, scale, state.material_dark, &mut out);
    layer_items(renderer, &map, Layer::Bottom, scale, state.material_dark, &mut out);
    // windows bottom first; a window's surface origin is its frame minus its geometry offset (client shadows)
    let windows: Vec<Window> = state.space.elements().cloned().collect();
    for w in windows {
        let Some(loc) = state.space.element_location(&w) else { continue };
        let origin = loc - w.geometry().loc;
        let phys: Point<i32, Physical> = origin.to_physical_precise_round(scale);
        let elements: Vec<WaylandSurfaceRenderElement<GlesRenderer>> = w.render_elements(renderer, phys, scale, 1.0);
        let (key, backing) = match w.toplevel() {
            Some(t) => {
                let active = t.current_state().states.contains(smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::State::Activated);
                (surface_key(t.wl_surface()), backing_at(t.wl_surface(), origin, active, state.material_dark))
            }
            None => (0, Backing { blur: Vec::new(), saturation: SATURATION, tone: Tone::NONE, plates: Vec::new() }),
        };
        let Backing { blur, saturation, tone, plates } = backing;
        out.push(Item { key, elements: elements.into_iter().map(SceneElement::Surface).collect(), blur, saturation, tone, plates });
    }
    // an input method's candidate window, above the windows
    for (surface, origin) in state.input_popups() {
        let phys: Point<i32, Physical> = origin.to_physical_precise_round(scale);
        let elements = render_elements_from_surface_tree(renderer, &surface, phys, scale, 1.0, Kind::Unspecified)
            .into_iter()
            .map(SceneElement::Surface)
            .collect();
        out.push(Item { key: surface_key(&surface), elements, blur: Vec::new(), saturation: SATURATION, tone: Tone::NONE, plates: Vec::new() });
    }
    layer_items(renderer, &map, Layer::Top, scale, state.material_dark, &mut out);
    layer_items(renderer, &map, Layer::Overlay, scale, state.material_dark, &mut out);
    out
}

/// The scene front to back, with each blurred surface's backdrop and each plated region's plate under it. `top` goes
/// first (the pointer, the boot fade).
pub fn build(renderer: &mut GlesRenderer, state: &WmState, output: &Output, glass: &mut Glass, top: Vec<SceneElement>) -> Vec<SceneElement> {
    let scale = Scale::from(output.current_scale().fractional_scale());
    // the scene is composed in desktop coordinates: the mode turned by the output transform
    let size: smithay::utils::Size<i32, smithay::utils::Physical> = crate::transform::output_size(output).unwrap_or_default().into();
    // locked: the lock surface and nothing else (black until it arrives)
    if state.locked {
        let mut all = top;
        if let Some(l) = &state.lock_surface {
            all.extend(
                render_elements_from_surface_tree(renderer, l.wl_surface(), (0, 0), scale, 1.0, Kind::Unspecified)
                    .into_iter()
                    .map(SceneElement::Surface),
            );
        }
        return all;
    }
    let mut below: Vec<SceneElement> = Vec::new();
    let mut blurred = Vec::new();
    for item in items(renderer, state, output, scale) {
        let mut next = item.elements;
        if !item.blur.is_empty() {
            match glass.backdrop(renderer, item.key, size, &below, &item.blur, item.saturation, item.tone) {
                Ok(g) => next.extend(g.into_iter().map(SceneElement::Glass)),
                Err(e) => eprintln!("bifrost-wm: glass: {e}"),
            }
            blurred.push(item.key);
        }
        // an inactive window's materials are flat plates: nothing else backs those regions, the client leaves them clear
        if !item.plates.is_empty() {
            next.extend(glass.plates(item.key, &item.plates).into_iter().map(SceneElement::Plate));
        }
        next.extend(below);
        below = next;
    }
    glass.retain(&blurred);
    glass.retain_plates();
    let mut all = top;
    all.extend(below);
    all
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::material_kind::{recipe, Blending, MaterialKind, MaterialState};

    fn rect(x: i32, y: i32, w: i32, h: i32) -> Rectangle<i32, Logical> {
        Rectangle::new((x, y).into(), (w, h).into())
    }

    /// A behind-window region of `kind` that follows its window's state, 10 px radius on every corner.
    fn spec(kind: MaterialKind, rects: Vec<Rectangle<i32, Logical>>) -> MaterialSpec {
        MaterialSpec { rects, kind, state: MaterialState::FollowsWindow, blending: Blending::BehindWindow, emphasized: false, radii: [10; 4] }
    }

    /// What an app with a sidebar and a toolbar band declares (surface coordinates, a 20 px shadow margin).
    fn app_materials() -> Vec<MaterialSpec> {
        vec![spec(MaterialKind::Sidebar, vec![rect(20, 20, 200, 600)]), spec(MaterialKind::Titlebar, vec![rect(220, 20, 780, 52)])]
    }

    /// An inactive window shows its materials as opaque plates: nothing blurs, every region has a plate of the recipe's
    /// colour with alpha 1, so the windows and the wallpaper behind do not show through the window (the sidebar of one
    /// window printed over the text of the window behind it).
    #[test]
    fn an_inactive_windows_materials_are_opaque_plates_not_clear_holes() {
        for dark in [false, true] {
            let b = backing_of(&app_materials(), (100, 50).into(), false, dark);
            assert!(b.blur.is_empty(), "an inactive window blurs nothing (dark {dark})");
            assert_eq!(b.plates.len(), 2, "one plate per region (dark {dark})");
            let sidebar = recipe(MaterialKind::Sidebar);
            let plate = if dark { sidebar.plate_dark } else { sidebar.plate_light };
            assert_eq!(b.plates[0].rect, rect(120, 70, 200, 600), "placed at the surface origin");
            assert_eq!(b.plates[0].color, premultiply(plate));
            assert_eq!(b.plates[1].rect, rect(320, 70, 780, 52));
            for p in &b.plates {
                assert_eq!(p.color[3], 1.0, "the plate is fully opaque (dark {dark})");
                assert_eq!(p.radius, 10.0, "a single-rectangle material keeps its corner radius");
            }
        }
    }

    /// The key window blurs its materials instead and draws no plate.
    #[test]
    fn an_active_windows_materials_blur_and_have_no_plate() {
        let b = backing_of(&app_materials(), (100, 50).into(), true, false);
        assert!(b.plates.is_empty());
        // each region keeps its corner radius, so the blur stays inside the window's rounded corners
        assert_eq!(b.blur, vec![Region { rect: rect(120, 70, 200, 600), radius: 10.0 }, Region { rect: rect(320, 70, 780, 52), radius: 10.0 }]);
    }

    /// Kinds that follow no window state (menus) and the plain blur region stay blurred in an inactive window; a region
    /// whose state is explicitly inactive is a plate even in the key window.
    #[test]
    fn only_regions_that_resolve_to_a_plate_get_one() {
        let menu = spec(MaterialKind::Menu, vec![rect(0, 0, 100, 100)]);
        let plain = MaterialSpec::plain(vec![rect(0, 100, 100, 100)]);
        let mut pinned = spec(MaterialKind::Sidebar, vec![rect(0, 200, 100, 100)]);
        pinned.state = MaterialState::Inactive;
        let b = backing_of(&[menu, plain, pinned.clone()], (0, 0).into(), false, true);
        assert_eq!(b.blur, vec![Region { rect: rect(0, 0, 100, 100), radius: 10.0 }, Region { rect: rect(0, 100, 100, 100), radius: 0.0 }]);
        assert_eq!(b.plates.len(), 1);
        assert_eq!(b.plates[0].rect, rect(0, 200, 100, 100));
        let follows = spec(MaterialKind::Sidebar, vec![rect(0, 0, 10, 10)]);
        let b = backing_of(&[follows, pinned], (0, 0).into(), true, true);
        assert_eq!((b.blur.len(), b.plates.len()), (1, 1), "the key window's explicit inactive region is a plate");
    }

    /// A material made of several rectangles has square plates (the compose shader's one radius per rectangle).
    #[test]
    fn a_material_of_several_rectangles_has_square_plates() {
        let b = backing_of(&[spec(MaterialKind::Sidebar, vec![rect(0, 0, 50, 50), rect(50, 0, 50, 50)])], (0, 0).into(), false, false);
        assert_eq!(b.plates.len(), 2);
        assert!(b.plates.iter().all(|p| p.radius == 0.0));
    }

    /// No materials: nothing behind the window but the window itself.
    #[test]
    fn a_surface_without_materials_has_no_backing() {
        let b = backing_of(&[], (5, 5).into(), false, false);
        assert!(b.blur.is_empty() && b.plates.is_empty());
        assert_eq!(b.saturation, SATURATION);
    }

    /// Every plate of every kind and appearance is opaque: an inactive window can never be translucent through a plate.
    #[test]
    fn every_plate_of_every_kind_is_opaque() {
        for v in 0..=15 {
            let kind = MaterialKind::from_wire(v).unwrap();
            for dark in [false, true] {
                let mut s = spec(kind, vec![rect(0, 0, 8, 8)]);
                s.state = MaterialState::Inactive;
                let b = backing_of(&[s], (0, 0).into(), false, dark);
                assert_eq!(b.plates.len(), 1, "{kind:?}");
                assert_eq!(b.plates[0].color[3], 1.0, "{kind:?} dark {dark}");
            }
        }
    }
}
