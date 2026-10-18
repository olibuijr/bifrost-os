//! Per-pixel Lambert lighting for 2D sprites, as plain std code (the CPU reference for a sprite lighting step). A sprite
//! pixel has a base colour and a normal; the light has a direction towards it, a colour and an ambient level. The result
//! is `colour * (ambient + light * max(0, n . l))` per channel, clamped to 0..=1. A zero normal counts as flat (facing
//! the viewer), and a zero light direction gives ambient light only.

fn unit(v: [f32; 3]) -> Option<[f32; 3]> {
    let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if len > 0.0 && len.is_finite() {
        Some([v[0] / len, v[1] / len, v[2] / len])
    } else {
        None
    }
}

/// The lit colour of one sprite pixel. `to_light` points from the pixel towards the light; it need not be unit length.
pub fn lit_pixel(colour: [f32; 3], normal: [f32; 3], to_light: [f32; 3], light: [f32; 3], ambient: f32) -> [f32; 3] {
    let n = unit(normal).unwrap_or([0.0, 0.0, 1.0]);
    let diffuse = unit(to_light).map_or(0.0, |l| (n[0] * l[0] + n[1] * l[1] + n[2] * l[2]).max(0.0));
    let ambient = ambient.max(0.0);
    let mut out = [0.0f32; 3];
    for c in 0..3 {
        out[c] = (colour[c] * (ambient + light[c].max(0.0) * diffuse)).clamp(0.0, 1.0);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn near(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-5
    }

    #[test]
    fn light_facing_the_pixel_keeps_its_colour() {
        let c = lit_pixel([0.2, 0.4, 0.6], [0.0, 0.0, 1.0], [0.0, 0.0, 5.0], [1.0, 1.0, 1.0], 0.0);
        assert!(near(c[0], 0.2) && near(c[1], 0.4) && near(c[2], 0.6));
    }

    #[test]
    fn light_edge_on_gives_ambient_only() {
        let c = lit_pixel([0.5, 0.5, 0.5], [0.0, 0.0, 1.0], [1.0, 0.0, 0.0], [1.0, 1.0, 1.0], 0.2);
        assert!(near(c[0], 0.1));
    }

    #[test]
    fn zero_normal_is_flat_and_zero_light_is_ambient() {
        let flat = lit_pixel([1.0, 1.0, 1.0], [0.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.5, 0.5, 0.5], 0.0);
        assert!(near(flat[0], 0.5));
        let dark = lit_pixel([1.0, 1.0, 1.0], [0.0, 0.0, 1.0], [0.0, 0.0, 0.0], [1.0, 1.0, 1.0], 0.25);
        assert!(near(dark[0], 0.25));
    }

    #[test]
    fn output_is_clamped_to_the_displayable_range() {
        let c = lit_pixel([1.0, 1.0, 1.0], [0.0, 0.0, 1.0], [0.0, 0.0, 1.0], [1.0, 1.0, 1.0], 1.0);
        assert!(near(c[0], 1.0));
    }
}
