//! Drawing a sprite batch into a view's drawable. A `bifrost_ui::sprite_batch::SpriteBatch` joins quads into texture runs;
//! [`draw_batch`] walks the runs and draws each quad into an `Image` (a `gpu_view::Frame` target), sampling the run's
//! `gpu_texture::Texture` with the nearest texel, tinting by the vertex colour and blending source-over. It is the CPU
//! reference of what a GPU pass would draw for the same batch.
//!
//! Quads are axis-aligned, which is all `SpriteBatch::push_quad` builds: the covered rectangle comes from the extremes of
//! the quad's positions, and the texture coordinates from its first and last corner. A pixel is covered when its centre is
//! inside the rectangle. Positions are in points and `scale` turns them into pixels.
use bifrost_render::gfx::Image;
use bifrost_ui::sprite_batch::{Run, SpriteBatch, Vertex, VERTICES_PER_QUAD};

use crate::gpu_texture::Texture;

/// Premultiplied source-over of `src` on `dst`.
fn over(src: u32, dst: u32) -> u32 {
    let sa = src >> 24;
    let mut out = 0;
    for shift in [0u32, 8, 16, 24] {
        let s = (src >> shift) & 255;
        let d = (dst >> shift) & 255;
        let v = (s + (d * (255 - sa) + 127) / 255).min(255);
        out |= v << shift;
    }
    out
}

/// A texel (premultiplied) tinted by a straight colour: colour channels scale by the tint colour and its alpha, alpha by
/// the tint alpha.
fn tint(texel: u32, color: [f32; 4]) -> u32 {
    let a = color[3].clamp(0.0, 1.0);
    let mut out = 0;
    for (i, shift) in [0u32, 8, 16].into_iter().enumerate() {
        let c = ((texel >> shift) & 255) as f32 * color[i].clamp(0.0, 1.0) * a;
        out |= (c.round().min(255.0) as u32) << shift;
    }
    let alpha = ((texel >> 24) & 255) as f32 * a;
    out | ((alpha.round().min(255.0) as u32) << 24)
}

fn solid(color: [f32; 4]) -> u32 {
    tint(0xffff_ffff, color)
}

fn quad_rect(quad: &[Vertex]) -> (f32, f32, f32, f32) {
    let xs = quad.iter().map(|v| v.pos[0]);
    let ys = quad.iter().map(|v| v.pos[1]);
    (
        xs.clone().fold(f32::INFINITY, f32::min),
        ys.clone().fold(f32::INFINITY, f32::min),
        xs.fold(f32::NEG_INFINITY, f32::max),
        ys.fold(f32::NEG_INFINITY, f32::max),
    )
}

/// Draws every quad of `batch` into `target`. `textures[i]` is the texture a run with `Some(i)` samples; a run whose index
/// is out of range is skipped. Returns the number of quads drawn. Quads wholly outside the target, or with a position that
/// is not finite, are skipped without touching memory outside it.
pub fn draw_batch(target: &mut Image, batch: &SpriteBatch, textures: &[&Texture], scale: f32) -> usize {
    if target.w.checked_mul(target.h) != Some(target.px.len()) || !scale.is_finite() || scale <= 0.0 {
        return 0;
    }
    let verts = batch.vertices();
    let mut drawn = 0;
    for run in batch.runs() {
        let Run { texture, start, count } = *run;
        let texture = match texture {
            Some(i) => match textures.get(i as usize) {
                Some(t) => Some(*t),
                None => continue,
            },
            None => None,
        };
        let Some(vs) = verts.get(start..start.saturating_add(count)) else { continue };
        for quad in vs.chunks_exact(VERTICES_PER_QUAD) {
            if quad.iter().any(|v| !v.pos[0].is_finite() || !v.pos[1].is_finite()) {
                continue;
            }
            let (x0, y0, x1, y1) = quad_rect(quad);
            let (px0, py0, px1, py1) = (x0 * scale, y0 * scale, x1 * scale, y1 * scale);
            if px1 <= px0 || py1 <= py0 {
                continue;
            }
            // Pixels whose centre (x + 0.5) lies in [px0, px1).
            let first_x = (px0 - 0.5).ceil().max(0.0) as usize;
            let first_y = (py0 - 0.5).ceil().max(0.0) as usize;
            let last_x = ((px1 - 0.5).ceil().max(0.0) as usize).min(target.w);
            let last_y = ((py1 - 0.5).ceil().max(0.0) as usize).min(target.h);
            if first_x >= last_x || first_y >= last_y {
                continue;
            }
            // The quad is [tl, tr, bl, tr, br, bl]: uv of the top left and the bottom right corner.
            let (uv_tl, uv_br) = (quad[0].uv, quad[4].uv);
            let color = quad[0].color;
            for y in first_y..last_y {
                let ty = (y as f32 + 0.5 - py0) / (py1 - py0);
                for x in first_x..last_x {
                    let tx = (x as f32 + 0.5 - px0) / (px1 - px0);
                    let src = match texture {
                        Some(t) => {
                            let u = uv_tl[0] + (uv_br[0] - uv_tl[0]) * tx;
                            let v = uv_tl[1] + (uv_br[1] - uv_tl[1]) * ty;
                            tint(t.sample_nearest(u, v, 0).unwrap_or(0), color)
                        }
                        None => solid(color),
                    };
                    let at = y * target.w + x;
                    target.px[at] = over(src, target.px[at]);
                }
            }
            drawn += 1;
        }
    }
    drawn
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gpu_texture::TextureLoader;
    use bifrost_render::gfx::pack;

    const RED: [f32; 4] = [1.0, 0.0, 0.0, 1.0];

    fn canvas(w: usize, h: usize, fill: u32) -> Image {
        Image { w, h, px: vec![fill; w * h] }
    }

    fn texture_2x2() -> Texture {
        // Row 0: red, green. Row 1: blue, white. All opaque.
        let px = vec![pack(255, 0, 0, 255), pack(0, 255, 0, 255), pack(0, 0, 255, 255), pack(255, 255, 255, 255)];
        TextureLoader::default().from_image(Image { w: 2, h: 2, px }).unwrap()
    }

    #[test]
    fn a_solid_quad_covers_the_pixels_whose_centres_it_contains() {
        let mut target = canvas(6, 4, pack(0, 0, 0, 255));
        let mut batch = SpriteBatch::new();
        batch.push_quad([1.0, 1.0, 3.0, 2.0], [0.0, 0.0, 1.0, 1.0], RED, None).unwrap();
        assert_eq!(draw_batch(&mut target, &batch, &[], 1.0), 1);
        let red = pack(255, 0, 0, 255);
        let covered: Vec<usize> = (0..24).filter(|&i| target.px[i] == red).collect();
        assert_eq!(covered, vec![7, 8, 9, 13, 14, 15], "x 1..4 on rows 1 and 2");
        assert_eq!(target.px[0], pack(0, 0, 0, 255));
    }

    #[test]
    fn the_scale_turns_points_into_pixels() {
        let mut target = canvas(8, 8, 0);
        let mut batch = SpriteBatch::new();
        batch.push_quad([1.0, 1.0, 2.0, 2.0], [0.0, 0.0, 1.0, 1.0], RED, None).unwrap();
        draw_batch(&mut target, &batch, &[], 2.0);
        let hit: Vec<(usize, usize)> = (0..64).filter(|&i| target.px[i] != 0).map(|i| (i % 8, i / 8)).collect();
        assert_eq!(hit, vec![(2, 2), (3, 2), (4, 2), (5, 2), (2, 3), (3, 3), (4, 3), (5, 3), (2, 4), (3, 4), (4, 4), (5, 4), (2, 5), (3, 5), (4, 5), (5, 5)]);
    }

    #[test]
    fn a_textured_quad_samples_the_nearest_texel() {
        let tex = texture_2x2();
        let mut target = canvas(4, 4, 0);
        let mut batch = SpriteBatch::new();
        batch.push_quad([0.0, 0.0, 4.0, 4.0], [0.0, 0.0, 1.0, 1.0], [1.0; 4], Some(0)).unwrap();
        assert_eq!(draw_batch(&mut target, &batch, &[&tex], 1.0), 1);
        assert_eq!(target.px[0], pack(255, 0, 0, 255));
        assert_eq!(target.px[3], pack(0, 255, 0, 255));
        assert_eq!(target.px[12], pack(0, 0, 255, 255));
        assert_eq!(target.px[15], pack(255, 255, 255, 255));
    }

    #[test]
    fn flipped_texture_coordinates_mirror_the_image() {
        let tex = texture_2x2();
        let mut target = canvas(2, 2, 0);
        let mut batch = SpriteBatch::new();
        batch.push_quad([0.0, 0.0, 2.0, 2.0], [1.0, 0.0, 0.0, 1.0], [1.0; 4], Some(0)).unwrap();
        draw_batch(&mut target, &batch, &[&tex], 1.0);
        assert_eq!(target.px[0], pack(0, 255, 0, 255));
        assert_eq!(target.px[1], pack(255, 0, 0, 255));
    }

    #[test]
    fn a_half_transparent_tint_blends_over_the_background() {
        let mut target = canvas(1, 1, pack(0, 0, 200, 255));
        let mut batch = SpriteBatch::new();
        batch.push_quad([0.0, 0.0, 1.0, 1.0], [0.0, 0.0, 1.0, 1.0], [1.0, 1.0, 1.0, 0.5], None).unwrap();
        draw_batch(&mut target, &batch, &[], 1.0);
        let px = target.px[0];
        // White at half alpha is premultiplied (128, 128, 128, 128); over blue 200 the blue becomes 128 + 200 * 127 / 255 = 228.
        assert_eq!((px & 255, (px >> 8) & 255, (px >> 16) & 255, px >> 24), (128, 128, 228, 255));
    }

    #[test]
    fn quads_outside_the_target_and_runs_without_a_texture_are_skipped() {
        let mut target = canvas(4, 4, 0);
        let mut batch = SpriteBatch::new();
        batch.push_quad([10.0, 10.0, 5.0, 5.0], [0.0, 0.0, 1.0, 1.0], RED, None).unwrap();
        batch.push_quad([-9.0, -9.0, 5.0, 5.0], [0.0, 0.0, 1.0, 1.0], RED, None).unwrap();
        assert_eq!(draw_batch(&mut target, &batch, &[], 1.0), 0);
        assert!(target.px.iter().all(|&p| p == 0));
        let mut textured = SpriteBatch::new();
        textured.push_quad([0.0, 0.0, 4.0, 4.0], [0.0, 0.0, 1.0, 1.0], [1.0; 4], Some(3)).unwrap();
        assert_eq!(draw_batch(&mut target, &textured, &[], 1.0), 0, "texture 3 does not exist");
        assert!(target.px.iter().all(|&p| p == 0));
    }

    #[test]
    fn a_quad_that_runs_past_the_edge_is_clipped() {
        let mut target = canvas(3, 3, 0);
        let mut batch = SpriteBatch::new();
        batch.push_quad([2.0, 2.0, 100.0, 100.0], [0.0, 0.0, 1.0, 1.0], RED, None).unwrap();
        assert_eq!(draw_batch(&mut target, &batch, &[], 1.0), 1);
        assert_eq!(target.px.iter().filter(|&&p| p != 0).count(), 1);
        assert_eq!(target.px[8], pack(255, 0, 0, 255));
    }

    #[test]
    fn a_bad_target_or_scale_draws_nothing() {
        let mut batch = SpriteBatch::new();
        batch.push_quad([0.0, 0.0, 1.0, 1.0], [0.0, 0.0, 1.0, 1.0], RED, None).unwrap();
        let mut broken = Image { w: 4, h: 4, px: vec![0; 3] };
        assert_eq!(draw_batch(&mut broken, &batch, &[], 1.0), 0);
        let mut ok = canvas(2, 2, 0);
        assert_eq!(draw_batch(&mut ok, &batch, &[], 0.0), 0);
        assert_eq!(draw_batch(&mut ok, &batch, &[], f32::NAN), 0);
        assert!(ok.px.iter().all(|&p| p == 0));
        assert_eq!(draw_batch(&mut ok, &SpriteBatch::new(), &[], 1.0), 0);
    }
}
