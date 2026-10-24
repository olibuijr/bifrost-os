//! The lens-shaped distortion filters that `distortion_filters` does not cover, with the reference
//! parameter names and defaults:
//!
//! - `CIHoleDistortion` (center, radius 150): a transparent hole of `radius` at `center`, with the
//!   picture pushed outwards around it. A point at distance `d >= r` reads the source at
//!   `sqrt(d^2 - r^2)`, so the rim shows the centre pixel and the push fades with distance.
//! - `CILightTunnel` (center, rotation 0, radius 100): the disc inside `radius` is unchanged; outside
//!   it every point reads the rim along its ray, turned by `rotation * (d - r) / r`, so the rim colours
//!   stream outwards in a spiral.
//! - `CITorusLensDistortion` (center, radius 160, width 80, refraction 1.7): a glass ring of `width`
//!   centred on the circle of `radius`. Inside the ring the picture is magnified about the ring's middle
//!   line by up to `1 + refraction` (the ring's height profile is a half circle), and it is unchanged
//!   at the ring's edges and outside. This replaces the whole-disc model in `distortion_filters`, which
//!   had no width.
//! - `CICircleSplashDistortion` (center, radius 150) as the reference documents it: the disc is
//!   unchanged and every point outside reads the circumference along its ray, so the rim splashes
//!   outwards. `distortion_filters::circle_splash` keeps its earlier magnifier model.
//!
//! The profiles are this model's own (the reference kernels are not measured); the parameter names,
//! defaults and the overall shape follow the reference documentation. Backwards warps with bilinear
//! sampling over premultiplied pixels; outside the image reads transparent.
use crate::distortion_filters::{sample, warp};
use crate::gfx::Image;

/// Reference default radius of the hole distortion.
pub const HOLE_RADIUS: f32 = 150.0;
/// Reference default radius of the light tunnel.
pub const TUNNEL_RADIUS: f32 = 100.0;
/// Reference defaults of the torus lens: radius, width and refraction.
pub const TORUS_RADIUS: f32 = 160.0;
pub const TORUS_WIDTH: f32 = 80.0;
pub const TORUS_REFRACTION: f32 = 1.7;
/// Reference default radius of the circle splash.
pub const SPLASH_RADIUS: f32 = 150.0;

fn finite_or(v: f32, d: f32) -> f32 {
    if v.is_finite() { v } else { d }
}

/// `CIHoleDistortion`. A radius at or below zero leaves the image unchanged.
pub fn hole(img: &Image, center: (f32, f32), radius: f32) -> Image {
    let r = finite_or(radius, 0.0);
    if r <= 0.0 {
        return img.clone();
    }
    let mut out = Image::new(img.w, img.h);
    for y in 0..img.h {
        for x in 0..img.w {
            let (dx, dy) = (x as f32 + 0.5 - center.0, y as f32 + 0.5 - center.1);
            let d = (dx * dx + dy * dy).sqrt();
            if d < r {
                continue;
            }
            let k = (d * d - r * r).sqrt() / d;
            out.px[y * img.w + x] = sample(img, center.0 + dx * k, center.1 + dy * k);
        }
    }
    out
}

/// `CILightTunnel`. A radius at or below zero leaves the image unchanged.
pub fn light_tunnel(img: &Image, center: (f32, f32), rotation: f32, radius: f32) -> Image {
    let r = finite_or(radius, 0.0);
    let rot = finite_or(rotation, 0.0);
    if r <= 0.0 {
        return img.clone();
    }
    warp(img, |x, y| {
        let (dx, dy) = (x - center.0, y - center.1);
        let d = (dx * dx + dy * dy).sqrt();
        if d <= r {
            return (x, y);
        }
        let a = dy.atan2(dx) + rot * (d - r) / r;
        // Just inside the rim, so the sample reads the disc's edge pixels.
        let rim = (r - 0.5).max(0.0);
        (center.0 + rim * a.cos(), center.1 + rim * a.sin())
    })
}

/// The source distance that `torus_lens` reads for a point at distance `d`.
pub fn torus_source_distance(d: f32, radius: f32, width: f32, refraction: f32) -> f32 {
    let half = width / 2.0;
    if half <= 0.0 || (d - radius).abs() >= half {
        return d;
    }
    let t = (d - radius) / half;
    let height = (1.0 - t * t).sqrt();
    radius + (d - radius) / (1.0 + refraction * height)
}

/// `CITorusLensDistortion` with its ring `width`. Width and refraction at or below zero leave the image
/// unchanged; refraction is clamped to the reference range 0..=5.
pub fn torus_lens(img: &Image, center: (f32, f32), radius: f32, width: f32, refraction: f32) -> Image {
    let r = finite_or(radius, 0.0).max(0.0);
    let w = finite_or(width, 0.0).max(0.0);
    let k = finite_or(refraction, 0.0).clamp(0.0, 5.0);
    if w == 0.0 || k == 0.0 {
        return img.clone();
    }
    warp(img, |x, y| {
        let (dx, dy) = (x - center.0, y - center.1);
        let d = (dx * dx + dy * dy).sqrt();
        if d == 0.0 {
            return (x, y);
        }
        let s = torus_source_distance(d, r, w, k) / d;
        (center.0 + dx * s, center.1 + dy * s)
    })
}

/// `CICircleSplashDistortion` as documented: the disc stays, the outside reads the circumference.
pub fn circle_splash_rim(img: &Image, center: (f32, f32), radius: f32) -> Image {
    light_tunnel(img, center, 0.0, radius)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ramp(n: usize) -> Image {
        let mut i = Image::new(n, n);
        for (k, p) in i.px.iter_mut().enumerate() {
            *p = (k as u32 % 251) | 0xff00_0000;
        }
        i
    }

    #[test]
    fn hole_is_clear_inside_and_opaque_far_out() {
        let img = ramp(32);
        let out = hole(&img, (16.0, 16.0), 6.0);
        assert_eq!(out.px[16 * 32 + 16] >> 24, 0, "centre is a hole");
        assert_eq!(out.px[16 * 32 + 30] >> 24, 255, "outside the radius is picture");
        assert_eq!(hole(&img, (16.0, 16.0), 0.0), img);
    }

    #[test]
    fn hole_push_fades_with_distance() {
        // Far from the hole the source distance approaches the distance itself.
        let (d, r) = (1000.0f32, 10.0f32);
        assert!(((d * d - r * r).sqrt() - d).abs() < 0.1);
    }

    #[test]
    fn light_tunnel_keeps_the_disc_and_streams_the_rim() {
        let img = ramp(32);
        // The centre sits on a pixel centre, so row 16 lies exactly on the ray at angle 0.
        let c = (16.5, 16.5);
        let out = light_tunnel(&img, c, 0.0, 8.0);
        assert_eq!(out.px[16 * 32 + 16], img.px[16 * 32 + 16]);
        // Without rotation every point to the right of the disc reads the same rim pixel.
        assert_eq!(out.px[16 * 32 + 26], out.px[16 * 32 + 30]);
        let turned = light_tunnel(&img, c, 2.0, 8.0);
        assert_ne!(turned.px[16 * 32 + 30], out.px[16 * 32 + 30]);
        assert_eq!(light_tunnel(&img, c, 1.0, -1.0), img);
    }

    #[test]
    fn torus_profile_is_continuous_monotonic_and_magnifies_the_middle() {
        let (r, w, k) = (TORUS_RADIUS, TORUS_WIDTH, TORUS_REFRACTION);
        assert_eq!(torus_source_distance(r - w / 2.0, r, w, k), r - w / 2.0);
        assert_eq!(torus_source_distance(r + w / 2.0, r, w, k), r + w / 2.0);
        assert_eq!(torus_source_distance(10.0, r, w, k), 10.0);
        let mut prev = f32::MIN;
        for i in 0..=80 {
            let d = r - w / 2.0 + i as f32;
            let s = torus_source_distance(d, r, w, k);
            assert!(s >= prev, "monotonic at {d}");
            prev = s;
        }
        // Near the middle line a step of 1 reads about 1 / (1 + k) of a step.
        let step = torus_source_distance(r + 1.0, r, w, k) - torus_source_distance(r, r, w, k);
        assert!((step - 1.0 / (1.0 + k)).abs() < 0.01, "step {step}");
    }

    #[test]
    fn torus_without_width_or_refraction_is_identity() {
        let img = ramp(16);
        assert_eq!(torus_lens(&img, (8.0, 8.0), 5.0, 0.0, 1.7), img);
        assert_eq!(torus_lens(&img, (8.0, 8.0), 5.0, 4.0, 0.0), img);
        assert_ne!(torus_lens(&img, (8.0, 8.0), 5.0, 4.0, 1.7), img);
    }

    #[test]
    fn circle_splash_rim_keeps_the_disc() {
        let img = ramp(24);
        let out = circle_splash_rim(&img, (12.5, 12.5), 6.0);
        assert_eq!(out.px[12 * 24 + 12], img.px[12 * 24 + 12]);
        assert_eq!(out.px[12 * 24 + 20], out.px[12 * 24 + 23]);
    }

    #[test]
    fn non_finite_parameters_do_not_panic() {
        let img = ramp(8);
        let c = (f32::NAN, 4.0);
        let _ = hole(&img, c, f32::NAN);
        let _ = light_tunnel(&img, c, f32::INFINITY, 3.0);
        let _ = torus_lens(&img, c, f32::NAN, f32::INFINITY, f32::NAN);
    }
}
