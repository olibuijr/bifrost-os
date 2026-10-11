//! The composited scene of one output, front to back, shared by every backend: layer surfaces, windows in stacking
//! order, and the live glass backdrop right under every surface that asks for blur.

use smithay::{
    backend::renderer::{
        element::{
            memory::MemoryRenderBufferRenderElement,
            surface::{render_elements_from_surface_tree, WaylandSurfaceRenderElement},
            texture::TextureRenderElement,
            AsRenderElements, Kind,
        },
        gles::{GlesRenderer, GlesTexture},
    },
    desktop::{layer_map_for_output, Window},
    output::Output,
    reexports::wayland_server::{protocol::wl_surface::WlSurface, Resource},
    render_elements,
    utils::{Logical, Physical, Point, Rectangle, Scale},
    wayland::shell::wlr_layer::Layer,
};

use crate::{background_effect::blur_region, glass::Glass, state::WmState};

render_elements! {
    pub SceneElement<=GlesRenderer>;
    Surface = WaylandSurfaceRenderElement<GlesRenderer>,
    Glass = TextureRenderElement<GlesTexture>,
    Memory = MemoryRenderBufferRenderElement<GlesRenderer>,
}

/// One surface of the scene: its elements and where its blur region sits on the output.
struct Item {
    key: u64,
    elements: Vec<SceneElement>,
    blur: Vec<Rectangle<i32, Logical>>,
}

/// The blur region of `surface` placed with the surface's origin at `origin` (output logical px).
fn blur_at(surface: &WlSurface, origin: Point<i32, Logical>) -> Vec<Rectangle<i32, Logical>> {
    blur_region(surface).into_iter().map(|r| Rectangle::new(r.loc + origin, r.size)).collect()
}

/// A stable key for a surface's glass between frames.
fn surface_key(surface: &WlSurface) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    surface.id().hash(&mut h);
    h.finish()
}

/// The surfaces of one layer, bottom first (the map lists them top first).
fn layer_items(renderer: &mut GlesRenderer, map: &smithay::desktop::LayerMap, layer: Layer, scale: Scale<f64>, out: &mut Vec<Item>) {
    let mut ls: Vec<_> = map.layers_on(layer).collect();
    ls.reverse();
    for l in ls {
        let Some(geo) = map.layer_geometry(l) else { continue };
        let loc: Point<i32, Physical> = geo.loc.to_physical_precise_round(scale);
        let elements = render_elements_from_surface_tree(renderer, l.wl_surface(), loc, scale, 1.0, Kind::Unspecified)
            .into_iter()
            .map(SceneElement::Surface)
            .collect();
        out.push(Item { key: surface_key(l.wl_surface()), elements, blur: blur_at(l.wl_surface(), geo.loc) });
    }
}

/// Bottom-first list of the output's surfaces.
fn items(renderer: &mut GlesRenderer, state: &WmState, output: &Output, scale: Scale<f64>) -> Vec<Item> {
    let mut out = Vec::new();
    let map = layer_map_for_output(output);
    layer_items(renderer, &map, Layer::Background, scale, &mut out);
    layer_items(renderer, &map, Layer::Bottom, scale, &mut out);
    // windows bottom first; a window's surface origin is its frame minus its geometry offset (client shadows)
    let windows: Vec<Window> = state.space.elements().cloned().collect();
    for w in windows {
        let Some(loc) = state.space.element_location(&w) else { continue };
        let origin = loc - w.geometry().loc;
        let phys: Point<i32, Physical> = origin.to_physical_precise_round(scale);
        let elements: Vec<WaylandSurfaceRenderElement<GlesRenderer>> = w.render_elements(renderer, phys, scale, 1.0);
        let (key, blur) = match w.toplevel() {
            Some(t) => (surface_key(t.wl_surface()), blur_at(t.wl_surface(), origin)),
            None => (0, Vec::new()),
        };
        out.push(Item { key, elements: elements.into_iter().map(SceneElement::Surface).collect(), blur });
    }
    layer_items(renderer, &map, Layer::Top, scale, &mut out);
    layer_items(renderer, &map, Layer::Overlay, scale, &mut out);
    out
}

/// The scene front to back, with each blurred surface's backdrop under it. `top` goes first (the pointer, the boot fade).
pub fn build(renderer: &mut GlesRenderer, state: &WmState, output: &Output, glass: &mut Glass, top: Vec<SceneElement>) -> Vec<SceneElement> {
    let scale = Scale::from(output.current_scale().fractional_scale());
    let size = output.current_mode().map(|m| m.size).unwrap_or_default();
    let mut below: Vec<SceneElement> = Vec::new();
    let mut blurred = Vec::new();
    for item in items(renderer, state, output, scale) {
        let mut next = item.elements;
        if !item.blur.is_empty() {
            match glass.backdrop(renderer, item.key, size, &below, &item.blur) {
                Ok(g) => next.extend(g.into_iter().map(SceneElement::Glass)),
                Err(e) => eprintln!("bifrost-wm: glass: {e}"),
            }
            blurred.push(item.key);
        }
        next.extend(below);
        below = next;
    }
    glass.retain(&blurred);
    let mut all = top;
    all.extend(below);
    all
}
