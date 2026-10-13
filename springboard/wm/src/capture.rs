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
            gles::{GlesRenderer, GlesTexture},
            Bind, ExportMem, Offscreen,
        },
    },
    output::Output,
    utils::{Rectangle, Transform},
};

use crate::{
    glass::Glass,
    scene::SceneElement,
    session_protocols::{capture_size, clamp_region, CaptureSpec},
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
    let mode = output.current_mode().ok_or("output has no mode")?;
    let size = capture_size((mode.size.w, mode.size.h), output.current_transform());
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_read_rect_is_the_request_clipped_to_the_texture() {
        assert_eq!(read_rect((100, 50), Rect::new(10, 5, 20, 10)), Some((10, 5, 20, 10)));
        assert_eq!(read_rect((100, 50), Rect::new(90, 40, 50, 50)), Some((90, 40, 10, 10)));
        assert_eq!(read_rect((100, 50), Rect::new(100, 0, 5, 5)), None);
    }
}
