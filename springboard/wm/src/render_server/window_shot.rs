//! A window capture on the Vulkan render server: `bifrost_window_capture_v1` (Mission Control tiles, the Dock thumbnail
//! of a window being minimized, window screenshots) for the Vulkan scanout, which has no GLES renderer to draw
//! `capture::capture_window` with. Same contract: the window alone, its geometry scaled to fill the `size` picture,
//! its material regions as flat plates behind it, over a transparent ground, RGBA rows premultiplied, top first.
//!
//! The window's surface tree becomes `SceneNode`s with the same builder the scanout uses (`scene_build::surface_nodes`,
//! so the nodes sample the window's current dma-buf, imported once per `wl_buffer`). They are placed at the surface
//! origin (minus the geometry offset) and scaled per axis by `size / geometry` through `to_root`; the server composites
//! them into an offscreen target of `size` and the pixels are read back. It runs only for a request, never per frame.
//!
//! A window with no dma-buf in its tree (shm clients are not drawn by this path yet, see `scene_build`) has nothing to
//! sample: `window_nodes` answers `None`, the request fails, and the shell keeps the plate with the window's title.

use bifrost_gpu::{LoadAction, PixelFormat, Region, TextureDescriptor, TextureUsage};
use smithay::{
    desktop::Window,
    utils::{Logical, Point},
    wayland::seat::WaylandFocus,
};

use crate::{
    bifrost_material::effective_regions,
    capture::window_plates,
    glass::Plate,
    headless::bgra_to_rgba,
    render_server::{compose::SceneNode, scene_build::surface_nodes, server::Server},
    state::WmState,
};

/// The ground of a window picture: fully transparent.
const GROUND: [f32; 4] = [0.0; 4];

/// The per-axis scale from a window of `geo` (width, height) to a `size` picture, or `None` for an empty window or
/// picture. The picture is already fitted to the window's aspect, so the two factors differ only by rounding.
pub fn picture_scale(geo: (i32, i32), size: (i32, i32)) -> Option<(f32, f32)> {
    if geo.0 <= 0 || geo.1 <= 0 || size.0 <= 0 || size.1 <= 0 {
        return None;
    }
    Some((size.0 as f32 / geo.0 as f32, size.1 as f32 / geo.1 as f32))
}

/// Places `nodes` (logical px, scale 1) in the picture: `to_root` scales each axis by its factor, so the node scale
/// stays 1 and the corner radius stays in picture px.
pub fn place_in_picture(nodes: &mut [SceneNode], scale: (f32, f32)) {
    for n in nodes {
        n.to_root = [scale.0, 0.0, 0.0, scale.1, 0.0, 0.0];
        n.scale = 1.0;
    }
}

/// One opaque node per plate (`capture::window_plates`: the material regions of the window as flat plates), in logical
/// px. The plate's corner radius is already in picture px, and `place_in_picture` leaves the node scale at 1.
pub fn plate_nodes(plates: &[Plate]) -> Vec<SceneNode> {
    plates
        .iter()
        .map(|p| {
            let mut n = SceneNode::solid([p.rect.loc.x as f32, p.rect.loc.y as f32, p.rect.size.w as f32, p.rect.size.h as f32], p.color);
            n.corner_radius = p.radius;
            n
        })
        .collect()
}

/// Whether the nodes sample a client buffer at all: a picture of only plates would show an empty window.
pub fn has_contents(nodes: &[SceneNode]) -> bool {
    nodes.iter().any(|n| n.texture.is_some())
}

/// The nodes of `window` alone for a `size` picture, back to front (plates, then the surface tree), or `None` for an
/// empty window or one with no dma-buf buffer to sample.
pub fn window_nodes(server: &mut Server, state: &WmState, window: &Window, size: (i32, i32)) -> Option<Vec<SceneNode>> {
    let geo = window.geometry();
    let scale = picture_scale((geo.size.w, geo.size.h), size)?;
    let surface = window.wl_surface()?.into_owned();
    // the surface origin sits at minus the geometry offset, so the geometry's corner lands on the picture's
    let origin: Point<i32, Logical> = (-geo.loc.x, -geo.loc.y).into();
    let mut tree = Vec::new();
    surface_nodes(&mut server.importer, &surface, origin, 1.0, 0.0, &mut tree);
    if !has_contents(&tree) {
        return None;
    }
    let plates = window_plates(&effective_regions(&surface), origin, state.material_dark, scale.0.min(scale.1));
    let mut nodes = plate_nodes(&plates);
    nodes.extend(tree);
    place_in_picture(&mut nodes, scale);
    Some(nodes)
}

/// Composites `nodes` into a fresh `size` target over a transparent ground, waits for the GPU and returns the picture
/// as RGBA rows, top first.
pub fn render(server: &mut Server, nodes: &[SceneNode], size: (i32, i32)) -> bifrost_gpu::Result<Vec<u8>> {
    let (w, h) = (size.0.max(1) as u32, size.1.max(1) as u32);
    let target = server.device.new_texture(&TextureDescriptor {
        width: w,
        height: h,
        format: PixelFormat::Bgra8Unorm,
        usage: TextureUsage::RENDER_TARGET | TextureUsage::SAMPLED | TextureUsage::COPY_SRC,
        exportable: false,
        modifiers: vec![],
    })?;
    let damage = vec![[0, 0, w as i32, h as i32]];
    server.frame(nodes, &target, &damage, LoadAction::Clear(GROUND))?.wait()?;
    let bgra = target.read_region(Region { x: 0, y: 0, width: w, height: h })?;
    Ok(bgra_to_rgba(&bgra))
}

/// The window capture itself: `None` when the window has nothing to sample or the render fails (logged).
pub fn capture_window(server: &mut Server, state: &WmState, window: &Window, size: (i32, i32)) -> Option<Vec<u8>> {
    let nodes = window_nodes(server, state, window, size)?;
    match render(server, &nodes, size) {
        Ok(px) => Some(px),
        Err(e) => {
            eprintln!("bifrost-wm: vulkan window capture: {e}");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use smithay::utils::Rectangle;

    fn plate(x: i32, y: i32, w: i32, h: i32, radius: f32) -> Plate {
        Plate { rect: Rectangle::new((x, y).into(), (w, h).into()), color: [0.2, 0.2, 0.2, 1.0], radius }
    }

    fn textured(key: u64) -> SceneNode {
        let mut n = SceneNode::solid([0.0, 0.0, 800.0, 600.0], [0.0; 4]);
        n.texture = Some(key);
        n
    }

    #[test]
    fn the_picture_scale_is_the_size_over_the_geometry_per_axis() {
        assert_eq!(picture_scale((800, 600), (400, 300)), Some((0.5, 0.5)));
        assert_eq!(picture_scale((1000, 500), (200, 100)), Some((0.2, 0.2)));
    }

    #[test]
    fn an_empty_window_or_picture_has_no_scale() {
        assert_eq!(picture_scale((0, 600), (400, 300)), None);
        assert_eq!(picture_scale((800, -1), (400, 300)), None);
        assert_eq!(picture_scale((800, 600), (0, 300)), None);
        assert_eq!(picture_scale((800, 600), (400, 0)), None);
    }

    #[test]
    fn placing_in_the_picture_scales_each_axis_and_keeps_the_node_scale_at_one() {
        let mut nodes = vec![textured(1), SceneNode::solid([10.0, 20.0, 30.0, 40.0], [1.0; 4])];
        nodes[1].scale = 2.0;
        place_in_picture(&mut nodes, (0.5, 0.25));
        for n in &nodes {
            assert_eq!(n.to_root, [0.5, 0.0, 0.0, 0.25, 0.0, 0.0]);
            assert_eq!(n.scale, 1.0);
        }
    }

    #[test]
    fn a_placed_window_fills_the_picture_exactly() {
        // the window's geometry corner maps to the picture's origin and its far corner to the picture size
        let mut nodes = vec![textured(7)];
        place_in_picture(&mut nodes, picture_scale((800, 600), (200, 150)).unwrap());
        let n = &nodes[0];
        let [x, y, w, h] = n.rect;
        let map = |px: f32, py: f32| [(n.to_root[0] * px + n.to_root[2] * py + n.to_root[4]) * n.scale, (n.to_root[1] * px + n.to_root[3] * py + n.to_root[5]) * n.scale];
        assert_eq!(map(x, y), [0.0, 0.0]);
        assert_eq!(map(x + w, y + h), [200.0, 150.0]);
    }

    #[test]
    fn every_plate_becomes_one_opaque_node_at_its_rectangle() {
        let nodes = plate_nodes(&[plate(0, 0, 200, 600, 5.0), plate(200, 0, 780, 52, 0.0)]);
        assert_eq!(nodes.len(), 2);
        assert_eq!(nodes[0].rect, [0.0, 0.0, 200.0, 600.0]);
        assert_eq!(nodes[0].corner_radius, 5.0);
        assert_eq!(nodes[1].rect, [200.0, 0.0, 780.0, 52.0]);
        assert!(nodes.iter().all(|n| n.color[3] == 1.0 && n.texture.is_none() && n.material.is_none() && n.shadow.is_none()));
        assert!(plate_nodes(&[]).is_empty());
    }

    #[test]
    fn plates_alone_are_not_window_contents() {
        assert!(!has_contents(&[]));
        assert!(!has_contents(&plate_nodes(&[plate(0, 0, 10, 10, 0.0)])));
        assert!(has_contents(&[SceneNode::solid([0.0; 4], [0.0; 4]), textured(3)]));
    }
}
