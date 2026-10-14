//! Spot light: adds the light of a cone-shaped lamp to an image, with the parameters of the reference
//! spot light filter. The lamp is a point at `light` that points at `target`; the image lies in the
//! plane `z = 0`, and the coordinates are in pixels with the origin at the top left, as elsewhere in
//! the render crate. A pixel gets the colour `colour * brightness * cone`, where the cone is the cosine of
//! the angle between the lamp's axis and the ray to the pixel, raised to `concentration`. Pixels outside
//! the cone get nothing. The light is added to the premultiplied colour and clamped to the alpha, so
//! transparent pixels stay transparent.
//! Differences from the reference: the light is additive and does not fall off with distance, so the
//! brightness is the same at any range.
use crate::colorfx::Rgb;
use crate::gfx::Image;

/// A point in space, in pixels, with the image plane at `z = 0`.
pub type Point3 = (f32, f32, f32);

/// The light of a spot at `light` aimed at `target`. `brightness` 0 returns the image unchanged. A
/// higher `concentration` narrows the cone. When the lamp and its target coincide, or any parameter is
/// not finite, the image is returned unchanged.
pub fn spot_light(img: &Image, light: Point3, target: Point3, brightness: f32, concentration: f32, colour: Rgb) -> Image {
    let axis = (target.0 - light.0, target.1 - light.1, target.2 - light.2);
    let len = (axis.0 * axis.0 + axis.1 * axis.1 + axis.2 * axis.2).sqrt();
    let all = [light.0, light.1, light.2, target.0, target.1, target.2, brightness, concentration, colour.r, colour.g, colour.b];
    if len <= 0.0 || !len.is_finite() || !all.iter().all(|v| v.is_finite()) || brightness <= 0.0 {
        return img.clone();
    }
    let axis = (axis.0 / len, axis.1 / len, axis.2 / len);
    let exponent = concentration.max(0.0);
    let mut out = Image::new(img.w, img.h);
    for y in 0..img.h {
        for x in 0..img.w {
            let (dx, dy, dz) = (x as f32 + 0.5 - light.0, y as f32 + 0.5 - light.1, -light.2);
            let dist = (dx * dx + dy * dy + dz * dz).sqrt();
            let cos = if dist > 0.0 { (dx * axis.0 + dy * axis.1 + dz * axis.2) / dist } else { 1.0 };
            let lit = if cos > 0.0 { cos.powf(exponent) * brightness } else { 0.0 };
            out.px[y * img.w + x] = add_light(img.px[y * img.w + x], lit, colour);
        }
    }
    out
}

fn add_light(p: u32, lit: f32, colour: Rgb) -> u32 {
    let alpha = p >> 24;
    let mut out = [0u32; 4];
    for (i, channel) in [colour.r, colour.g, colour.b].into_iter().enumerate() {
        let base = (p >> (8 * i)) & 255;
        let v = base as f32 + lit * channel.clamp(0.0, 1.0) * 255.0;
        out[i] = (v + 0.5).clamp(0.0, alpha as f32) as u32;
    }
    out[3] = alpha;
    out[0] | out[1] << 8 | out[2] << 16 | out[3] << 24
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grey(n: usize) -> Image {
        let mut i = Image::new(n, n);
        i.px.iter_mut().for_each(|p| *p = 0xff40_4040);
        i
    }

    fn white_light() -> Rgb {
        Rgb { r: 1.0, g: 1.0, b: 1.0 }
    }

    #[test]
    fn zero_brightness_is_the_identity() {
        let img = grey(4);
        let warm = Rgb { r: 1.0, g: 0.5, b: 0.0 };
        assert_eq!(spot_light(&img, (2.0, 2.0, 5.0), (2.0, 2.0, 0.0), 0.0, 1.0, warm).px, img.px);
    }

    #[test]
    fn a_lamp_aimed_straight_down_lights_the_pixel_under_it_most() {
        let img = grey(5);
        let out = spot_light(&img, (2.5, 2.5, 10.0), (2.5, 2.5, 0.0), 0.5, 1.0, white_light());
        let centre = out.px[2 * 5 + 2] & 255;
        let corner = out.px[0] & 255;
        assert_eq!(centre, 192, "the pixel under the lamp gets the full half brightness");
        assert!(centre > corner, "the lit centre {centre} is brighter than the corner {corner}");
    }

    #[test]
    fn transparent_pixels_stay_transparent() {
        let img = Image::new(2, 2);
        let out = spot_light(&img, (1.0, 1.0, 4.0), (1.0, 1.0, 0.0), 1.0, 1.0, white_light());
        assert_eq!(out.px, vec![0; 4]);
    }

    #[test]
    fn the_light_is_clamped_to_the_alpha_so_channels_never_exceed_it() {
        let mut img = Image::new(1, 1);
        img.px[0] = 0x8080_8080;
        let out = spot_light(&img, (0.5, 0.5, 3.0), (0.5, 0.5, 0.0), 4.0, 0.0, white_light());
        assert_eq!(out.px[0], 0x8080_8080);
    }

    #[test]
    fn a_lamp_on_its_target_leaves_the_image_unchanged() {
        let img = grey(3);
        let out = spot_light(&img, (1.0, 1.0, 1.0), (1.0, 1.0, 1.0), 1.0, 1.0, white_light());
        assert_eq!(out.px, img.px);
    }
}
