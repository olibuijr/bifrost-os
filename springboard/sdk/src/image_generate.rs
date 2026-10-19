//! Generators and colour lookup for app images: the checkerboard, stripes, radial gradient and noise
//! generators (`CICheckerboardGenerator`, `CIStripesGenerator`, `CIRadialGradient`,
//! `CIRandomGenerator`), the QR code generators (`CIQRCodeGenerator`, `CIRoundedQRCodeGenerator`), colour
//! cube lookup from a `.cube` table (`CIColorCube`), colour maps (`CIColorMap`) and palettes
//! (`CIPalettize`), and the CIE L*a*b* conversion (`CIConvertRGBtoLab`, `CIConvertLabtoRGB`). The pixel
//! work lives in the render crate (`generators`, `qr_generate`, `colorcube`, `colormap`, `colorlab`),
//! which the shell can use too. Apps put the returned `Image` on screen with the kit's surface.
//! Limits: QR codes stop at version 10 (271 bytes at level L); there is no barcode generator; the
//! colour conversions assume sRGB and ignore ICC profiles and rendering intents.
use bifrost_render::colorfx::{self, Rgb};
use bifrost_render::gfx::Image;

pub use bifrost_render::colorcube::ColorCube;
pub use bifrost_render::colorlab::Lab;
pub use bifrost_render::colormap::{ColorMap, Palette};
pub use bifrost_render::gradient::Ramp;
pub use bifrost_render::qr_generate::{Correction, QrCode};

/// A checkerboard of `square` pixel cells with colour `a` in the top-left cell. Colours are packed like
/// `Image` pixels. `None` when `square` is 0.
pub fn checkerboard(w: usize, h: usize, square: usize, a: u32, b: u32) -> Option<Image> {
    bifrost_render::generators::checkerboard(w, h, square, a, b)
}

/// Stripes `width` pixels across, turned by `angle` radians from the vertical. `None` when `width` is not
/// a positive finite number or `angle` is not finite.
pub fn stripes(w: usize, h: usize, width: f32, angle: f32, a: u32, b: u32) -> Option<Image> {
    bifrost_render::generators::stripes(w, h, width, angle, a, b)
}

/// A radial gradient: offset 0 of `ramp` at `center`, offset 1 at `radius` pixels from it.
pub fn radial_gradient(w: usize, h: usize, center: (f32, f32), radius: f32, ramp: &Ramp) -> Image {
    bifrost_render::generators::radial_gradient(w, h, center, radius, ramp)
}

/// Opaque noise. The same `seed` always gives the same image.
pub fn random(w: usize, h: usize, seed: u32) -> Image {
    bifrost_render::generators::random(w, h, seed)
}

/// A QR code of `text` as a black-on-white picture, `scale` pixels per module with the four-module quiet
/// zone the specification asks for. `None` when `text` is empty, does not fit version 10 at `level`, or
/// `scale` is 0.
pub fn qr_code(text: &str, level: Correction, scale: usize) -> Option<Image> {
    qr_code_styled(text, level, scale, 4, 0xff00_0000, 0xffff_ffff, false)
}

/// A QR code with its own colours and quiet zone. `dark` and `light` are packed like `Image` pixels. With
/// `rounded` the data modules are discs and the finder squares stay square. For a scanner to read the code
/// `dark` must be clearly darker than `light`.
pub fn qr_code_styled(text: &str, level: Correction, scale: usize, quiet: usize, dark: u32, light: u32, rounded: bool) -> Option<Image> {
    QrCode::encode(text, level)?.to_image(scale, quiet, dark, light, rounded)
}

/// The module matrix of a QR code for text: for callers that draw the modules themselves.
pub fn qr_modules(text: &str, level: Correction) -> Option<QrCode> {
    QrCode::encode(text, level)
}

/// Looks every pixel of `img` up in `cube`, keeping alpha. With `extrapolate` colours outside 0 to 1 are
/// not clamped first (`CIColorCubeWithColorSpace` with extrapolation).
pub fn color_cube(img: &Image, cube: &ColorCube, extrapolate: bool) -> Image {
    map_colours(img, |c| if extrapolate { cube.lookup_extrapolated(c) } else { cube.lookup(c) })
}

/// Reads a `.cube` text table into a colour cube; see `bifrost_render::colorcube::parse_cube_lut` for the
/// accepted format. `None` when the text is not a usable 3D table.
pub fn color_cube_from_text(text: &str) -> Option<ColorCube> {
    bifrost_render::colorcube::parse_cube_lut(text)
}

/// Maps the luma of every pixel along `map`'s gradient, keeping alpha (`CIColorMap`).
pub fn color_map(img: &Image, map: &ColorMap) -> Image {
    Image { w: img.w, h: img.h, px: img.px.iter().map(|&p| map.map_pixel(p)).collect() }
}

/// Snaps every pixel to its nearest palette colour, keeping alpha (`CIPalettize`).
pub fn palettize(img: &Image, palette: &Palette) -> Image {
    Image { w: img.w, h: img.h, px: img.px.iter().map(|&p| palette.palettize_pixel(p)).collect() }
}

/// An sRGB colour (each channel 0 to 1) in CIE L*a*b* under D65 (`CIConvertRGBtoLab`, normalised off).
pub fn rgb_to_lab(c: Rgb) -> Lab {
    Lab::from_rgb(c)
}

/// A CIE L*a*b* colour as sRGB, clamped to 0 to 1 (`CIConvertLabtoRGB`).
pub fn lab_to_rgb(lab: Lab) -> Rgb {
    lab.to_rgb().clamped()
}

fn map_colours(img: &Image, f: impl Fn(Rgb) -> Rgb) -> Image {
    Image { w: img.w, h: img.h, px: img.px.iter().map(|&p| colorfx::map_pixel(p, &f)).collect() }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BLACK: u32 = 0xff00_0000;
    const WHITE: u32 = 0xffff_ffff;

    #[test]
    fn checkerboard_and_stripes_alternate_their_colours() {
        let c = checkerboard(4, 4, 2, BLACK, WHITE).unwrap();
        assert_eq!((c.px[0], c.px[2], c.px[2 * 4 + 2]), (BLACK, WHITE, BLACK));
        assert!(checkerboard(4, 4, 0, BLACK, WHITE).is_none());
        let s = stripes(8, 1, 2.0, 0.0, BLACK, WHITE).unwrap();
        assert_ne!(s.px[0], s.px[2]);
        assert!(stripes(8, 1, 0.0, 0.0, BLACK, WHITE).is_none());
    }

    #[test]
    fn radial_gradient_runs_from_the_first_to_the_last_colour() {
        let ramp = Ramp::new(&[(0.0, Rgb::new(0.0, 0.0, 0.0)), (1.0, Rgb::new(1.0, 1.0, 1.0))]).unwrap();
        let g = radial_gradient(9, 9, (4.5, 4.5), 4.0, &ramp);
        assert!((g.px[4 * 9 + 4] & 0xff) < 0x40, "centre is near the first colour");
        assert_eq!(g.px[0] & 0xff, 0xff, "corner is past the radius");
    }

    #[test]
    fn random_is_repeatable() {
        assert_eq!(random(4, 4, 9), random(4, 4, 9));
        assert_ne!(random(4, 4, 9), random(4, 4, 10));
    }

    #[test]
    fn qr_code_picture_has_the_quiet_zone_and_the_requested_scale() {
        let img = qr_code("https://bifrost.example", Correction::M, 3).unwrap();
        let modules = qr_modules("https://bifrost.example", Correction::M).unwrap();
        assert_eq!(img.w, (modules.size() + 8) * 3);
        assert_eq!(img.px[0], WHITE);
        assert!(qr_code("", Correction::M, 3).is_none());
        assert!(qr_code("x", Correction::M, 0).is_none());
        let rounded = qr_code_styled("x", Correction::L, 6, 0, BLACK, WHITE, true).unwrap();
        assert_eq!(rounded.w, 21 * 6);
    }

    #[test]
    fn color_cube_identity_leaves_pixels_and_the_inverted_cube_flips_them() {
        let img = Image { w: 2, h: 1, px: vec![0x8040_80c0, WHITE] };
        let id = ColorCube::identity(9).unwrap();
        let same = color_cube(&img, &id, false);
        for (a, b) in same.px.iter().zip(&img.px) {
            assert_eq!(a >> 24, b >> 24);
            for shift in [0, 8, 16] {
                assert!(((a >> shift & 0xff) as i32 - (b >> shift & 0xff) as i32).abs() <= 1);
            }
        }
        let text = "LUT_3D_SIZE 2\n1 1 1\n0 1 1\n1 0 1\n0 0 1\n1 1 0\n0 1 0\n1 0 0\n0 0 0\n";
        let inv = color_cube_from_text(text).unwrap();
        let out = color_cube(&Image { w: 1, h: 1, px: vec![WHITE] }, &inv, false);
        assert_eq!(out.px[0], BLACK);
    }

    #[test]
    fn colour_map_and_palette_keep_alpha() {
        let map = ColorMap::new(vec![Rgb::new(0.0, 0.0, 1.0), Rgb::new(1.0, 0.0, 0.0)]).unwrap();
        let img = Image { w: 1, h: 1, px: vec![0x8000_0000] };
        let out = color_map(&img, &map);
        assert_eq!(out.px[0] >> 24, 0x80);
        assert_eq!(out.px[0] >> 16 & 0xff, 0xff, "black maps to the first entry, blue");
        let pal = Palette::new(vec![Rgb::new(0.0, 0.0, 0.0), Rgb::new(1.0, 1.0, 1.0)], true).unwrap();
        let near_white = Image { w: 1, h: 1, px: vec![0xffe0_e0e0] };
        assert_eq!(palettize(&near_white, &pal).px[0], WHITE);
    }

    #[test]
    fn lab_round_trips_and_white_has_full_lightness() {
        let white = rgb_to_lab(Rgb::new(1.0, 1.0, 1.0));
        assert!((white.l - 100.0).abs() < 0.05 && white.a.abs() < 0.1 && white.b.abs() < 0.1);
        let c = Rgb::new(0.2, 0.5, 0.8);
        let back = lab_to_rgb(rgb_to_lab(c));
        assert!((back.r - c.r).abs() < 1e-3 && (back.g - c.g).abs() < 1e-3 && (back.b - c.b).abs() < 1e-3);
    }
}
