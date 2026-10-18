//! Profile-to-profile colour conversion through the ICC profile connection space (XYZ relative to D50).
//!
//! A `Model` is what a profile gives the engine: the forward and backward lookup tables of each rendering intent when
//! the profile has them (A2B0 to A2B2, B2A0 to B2A2), otherwise the matrix-shaper colorants and tone curves, or a
//! grey tone curve. A `Conversion` runs source device RGB to PCS XYZ, applies black point compensation when asked,
//! applies the absolute-colorimetric white scaling when that intent is chosen, and runs PCS XYZ to destination RGB.
//!
//! Intents follow the ICC definitions. Perceptual and saturation use their own tables when the profile has them and
//! fall back to relative colorimetric; a matrix-shaper profile has only the one transform, so every intent uses it.
//! Black point compensation is computed from the PCS black point (the table evaluated at device black), so it is a
//! no-op for matrix profiles, whose black is zero. Out-of-gamut colours are clamped at the end; no gamut mapping is
//! done beyond what a table already contains.
//!
//! Decoded pictures with an embedded profile are converted to the display profile by `to_display`. The conversion is
//! baked into a 33 x 33 x 33 table per (source profile, display, intent), cached, and applied with trilinear
//! interpolation, which is cheap enough for every decode. The display profile is built from the connected display's
//! EDID chromaticities (`edid::display_edid`), or from built-in sRGB when no display is readable.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex, OnceLock};

use super::cg::color::RenderingIntent;
use super::cg::imageio::{Decoded, Frame};
use super::colorspace::{self, Space};
use super::edid;
use super::icc::{Curve, MatrixShaper, Profile};
use super::icc_builtin::{self, Builtin};
use super::icc_lut::Lut;

const A2B: [&str; 3] = ["A2B0", "A2B1", "A2B2"];
const B2A: [&str; 3] = ["B2A0", "B2A1", "B2A2"];

/// The table slot of an intent: 0 perceptual, 1 relative colorimetric (also used by absolute), 2 saturation.
fn slot(intent: RenderingIntent) -> usize {
    match intent {
        RenderingIntent::Perceptual => 0,
        RenderingIntent::Saturation => 2,
        _ => 1,
    }
}

/// Three values from a slice of exactly three.
fn triple(v: &[f64]) -> Option<[f64; 3]> {
    (v.len() == 3).then(|| [v[0], v[1], v[2]])
}

/// What a profile contributes to a conversion.
#[derive(Clone, Debug)]
pub struct Model {
    matrix: Option<MatrixShaper>,
    grey: Option<Curve>,
    /// Device to PCS tables for perceptual, relative colorimetric and saturation.
    a2b: [Option<Lut>; 3],
    /// PCS to device tables for perceptual, relative colorimetric and saturation.
    b2a: [Option<Lut>; 3],
    /// Media white as XYZ with luminance 1 (`wtpt`, or the D50 white when the profile has none).
    media_white: [f64; 3],
}

impl Model {
    /// Reads the model of a profile from its bytes. Errors name what was refused.
    pub fn from_icc(data: &[u8]) -> Result<Model, String> {
        let p = Profile::parse(data).map_err(|e| format!("{e:?}"))?;
        let mut model = Model {
            matrix: None,
            grey: None,
            a2b: [None, None, None],
            b2a: [None, None, None],
            media_white: p.media_white(data).unwrap_or(colorspace::d50_xyz()),
        };
        match p.header.color_space.as_str() {
            "GRAY" => model.grey = Some(p.gray_curve(data).ok_or("grey profile without a kTRC curve")?),
            "RGB " => {
                model.matrix = p.matrix_shaper(data);
                for s in 0..3 {
                    model.a2b[s] = p.lut(data, A2B[s]);
                    model.b2a[s] = p.lut(data, B2A[s]);
                }
            }
            other => return Err(format!("colour space {other:?} is not supported")),
        }
        if model.grey.is_none() && model.matrix.is_none() && model.a2b.iter().all(Option::is_none) {
            return Err("profile has neither colorants nor a lookup table".into());
        }
        Ok(model)
    }

    /// Encoded device RGB to XYZ relative to D50 for an intent. `None` when the profile cannot take that input.
    pub fn to_pcs(&self, rgb: [f64; 3], intent: RenderingIntent) -> Option<[f64; 3]> {
        if let Some(curve) = &self.grey {
            let y = curve.eval(rgb[0]);
            return Some(colorspace::d50_xyz().map(|w| w * y));
        }
        if let Some(lut) = self.a2b[slot(intent)].as_ref().or(self.a2b[1].as_ref()) {
            let out = lut.eval(&rgb)?;
            return triple(&out).map(|e| lut.pcs_to_xyz(e));
        }
        self.matrix.as_ref().map(|m| m.to_xyz_d50(rgb))
    }

    /// XYZ relative to D50 to encoded device RGB for an intent. Values come back clamped to 0..=1 by the caller.
    pub fn from_pcs(&self, xyz: [f64; 3], intent: RenderingIntent) -> Option<[f64; 3]> {
        if let Some(curve) = &self.grey {
            let e = curve.invert(xyz[1]);
            return Some([e, e, e]);
        }
        if let Some(lut) = self.b2a[slot(intent)].as_ref().or(self.b2a[1].as_ref()) {
            let out = lut.eval(&lut.xyz_to_pcs(xyz))?;
            return triple(&out);
        }
        self.matrix.as_ref().map(|m| m.from_xyz_d50(xyz))
    }

    /// The PCS black point: the profile evaluated at device black.
    fn black_point(&self, intent: RenderingIntent) -> Option<[f64; 3]> {
        self.to_pcs([0.0; 3], intent)
    }
}

/// A conversion from one profile to another under one rendering intent.
#[derive(Clone, Debug)]
pub struct Conversion {
    src: Model,
    dst: Model,
    intent: RenderingIntent,
    black_point_compensation: bool,
}

impl Conversion {
    /// Builds the conversion from `src` to `dst`.
    pub fn new(src: Model, dst: Model, intent: RenderingIntent, black_point_compensation: bool) -> Self {
        Conversion { src, dst, intent, black_point_compensation }
    }

    /// Converts one colour: encoded source channels (nominally 0..=1) to encoded destination channels clamped to
    /// 0..=1. `None` when either profile cannot take the colour.
    pub fn apply(&self, rgb: [f64; 3]) -> Option<[f64; 3]> {
        let w = colorspace::d50_xyz();
        let mut xyz = self.src.to_pcs(rgb, self.intent)?;
        if self.black_point_compensation {
            if let (Some(bs), Some(bd)) = (self.src.black_point(self.intent), self.dst.black_point(self.intent)) {
                xyz = compensate_black(xyz, bs, bd, w);
            }
        }
        if matches!(self.intent, RenderingIntent::AbsoluteColorimetric) {
            // Absolute colorimetric keeps the source media white: scale to absolute on the way out, back to the
            // destination's relative scale on the way in.
            let (sw, dw) = (self.src.media_white, self.dst.media_white);
            xyz = [0, 1, 2].map(|i| xyz[i] * sw[i] / w[i]);
            xyz = [0, 1, 2].map(|i| xyz[i] * w[i] / dw[i]);
        }
        let out = self.dst.from_pcs(xyz, self.intent)?;
        Some(out.map(|c| c.clamp(0.0, 1.0)))
    }
}

/// ICC black point compensation: maps the source black point onto the destination black point and keeps the D50
/// white fixed.
fn compensate_black(xyz: [f64; 3], bs: [f64; 3], bd: [f64; 3], w: [f64; 3]) -> [f64; 3] {
    [0, 1, 2].map(|i| {
        let span = w[i] - bs[i];
        if span.abs() < 1e-9 {
            xyz[i]
        } else {
            (xyz[i] - bs[i]) * (w[i] - bd[i]) / span + bd[i]
        }
    })
}

/// A conversion baked into a regular grid over encoded RGB, sampled with trilinear interpolation.
#[derive(Debug)]
pub struct Lut3d {
    n: usize,
    data: Vec<[f32; 3]>,
}

impl Lut3d {
    /// Grid points per channel. 33 keeps the error of interpolation well under one 8-bit step for these profiles.
    pub const GRID: usize = 33;

    /// Evaluates `conv` at every grid point. `None` when any point fails.
    pub fn build(conv: &Conversion, n: usize) -> Option<Lut3d> {
        if n < 2 {
            return None;
        }
        let step = |i: usize| i as f64 / (n - 1) as f64;
        let mut data = Vec::with_capacity(n * n * n);
        for r in 0..n {
            for g in 0..n {
                for b in 0..n {
                    let out = conv.apply([step(r), step(g), step(b)])?;
                    data.push(out.map(|v| v as f32));
                }
            }
        }
        Some(Lut3d { n, data })
    }

    /// The converted colour for encoded input (0..=1), trilinearly interpolated.
    pub fn sample(&self, rgb: [f64; 3]) -> [f64; 3] {
        let n = self.n;
        let pos = rgb.map(|v| v.clamp(0.0, 1.0) * (n - 1) as f64);
        let base = pos.map(|p| (p.floor() as usize).min(n - 2));
        let frac = [0, 1, 2].map(|k| pos[k] - base[k] as f64);
        let mut out = [0.0f64; 3];
        for corner in 0..8usize {
            let d = [(corner >> 2) & 1, (corner >> 1) & 1, corner & 1];
            let weight: f64 = (0..3).map(|k| if d[k] == 1 { frac[k] } else { 1.0 - frac[k] }).product();
            let idx = ((base[0] + d[0]) * n + base[1] + d[1]) * n + base[2] + d[2];
            let v = self.data[idx];
            for k in 0..3 {
                out[k] += weight * v[k] as f64;
            }
        }
        out
    }
}

/// Small cache of baked tables, most recent last.
static CACHE: Mutex<Vec<(u64, Arc<Lut3d>)>> = Mutex::new(Vec::new());
const CACHE_LIMIT: usize = 8;

/// Returns the table for `key`, building it once. `None` when the build fails (nothing is cached then).
fn cached(key: u64, build: impl FnOnce() -> Option<Lut3d>) -> Option<Arc<Lut3d>> {
    if let Ok(cache) = CACHE.lock() {
        if let Some((_, hit)) = cache.iter().find(|(k, _)| *k == key) {
            return Some(hit.clone());
        }
    }
    let lut = Arc::new(build()?);
    if let Ok(mut cache) = CACHE.lock() {
        if cache.len() >= CACHE_LIMIT {
            cache.remove(0);
        }
        cache.push((key, lut.clone()));
    }
    Some(lut)
}

/// A hash of the parts and a tag, for the cache.
fn cache_key(parts: &[&[u8]], tag: &str) -> u64 {
    let mut h = DefaultHasher::new();
    for p in parts {
        p.hash(&mut h);
    }
    tag.hash(&mut h);
    h.finish()
}

/// The display that decoded pictures are converted to.
pub struct Target {
    model: Model,
    /// Identifies the display in cache keys.
    id: u64,
    /// True when the display is built-in sRGB, so sRGB-tagged pictures pass through unchanged.
    srgb: bool,
}

static DISPLAY: OnceLock<Option<Target>> = OnceLock::new();

/// The display profile: the connected display's chromaticities when its EDID is readable, else built-in sRGB. `None`
/// only when no profile can be read at all, in which case pictures are left as decoded.
pub fn display_target() -> Option<&'static Target> {
    DISPLAY.get_or_init(build_target).as_ref()
}

fn build_target() -> Option<Target> {
    if let Some(e) = edid::display_edid() {
        let gamma = e.gamma;
        let bytes = icc_builtin::rgb_profile("Display", [e.red, e.green, e.blue], e.white, move |v| match gamma {
            Some(g) => v.powf(g),
            None => Space::Srgb.to_linear(v),
        });
        if let Ok(model) = Model::from_icc(&bytes) {
            return Some(Target { model, id: cache_key(&[bytes.as_slice()], "display"), srgb: false });
        }
    }
    let bytes = Builtin::Srgb.bytes();
    let model = Model::from_icc(&bytes).ok()?;
    Some(Target { model, id: cache_key(&[bytes.as_slice()], "display"), srgb: true })
}

/// The baked table that converts pictures tagged with `profile` to the display, or `None` when no conversion is
/// needed (sRGB-tagged pictures on an sRGB display) or the profile cannot be read.
fn picture_table(profile: &[u8], target: &Target) -> Option<Arc<Lut3d>> {
    if target.srgb {
        let parsed = Profile::parse(profile).ok()?;
        if parsed.description(profile).as_deref() == Some(Builtin::Srgb.description()) {
            return None;
        }
    }
    let id = target.id.to_le_bytes();
    let key = cache_key(&[profile, &id[..]], "perceptual+bpc");
    cached(key, || {
        let src = Model::from_icc(profile).ok()?;
        let conv = Conversion::new(src, target.model.clone(), RenderingIntent::Perceptual, true);
        Lut3d::build(&conv, Lut3d::GRID)
    })
}

/// Whether a picture tagged with `profile` is converted to the display profile when decoded.
pub fn converts(profile: &[u8]) -> bool {
    display_target().is_some_and(|t| picture_table(profile, t).is_some())
}

/// Converts the frames of a decoded file, in place, from the profile each frame carries to the display profile. The
/// frame's 16-bit copy, when present, is converted too.
pub fn to_display(d: &mut Decoded) {
    let Some(target) = display_target() else { return };
    for (i, frame) in d.frames.iter_mut().enumerate() {
        let Some(profile) = d.props.get(i).and_then(|p| p.icc_profile.as_deref()) else { continue };
        if let Some(table) = picture_table(profile, target) {
            convert_frame(frame, &table);
        }
    }
}

fn convert_frame(frame: &mut Frame, table: &Lut3d) {
    for p in frame.rgba8.as_chunks_mut::<4>().0.iter_mut() {
        let out = table.sample([p[0], p[1], p[2]].map(|c| c as f64 / 255.0));
        for k in 0..3 {
            p[k] = (out[k] * 255.0).round() as u8;
        }
    }
    if let Some(wide) = frame.rgba16.as_mut() {
        for p in wide.as_chunks_mut::<4>().0.iter_mut() {
            let out = table.sample([p[0], p[1], p[2]].map(|c| c as f64 / 65535.0));
            for k in 0..3 {
                p[k] = (out[k] * 65535.0).round() as u16;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model(b: Builtin) -> Model {
        Model::from_icc(&b.bytes()).expect("built-in profile reads")
    }

    #[test]
    fn srgb_to_p3_and_back_returns_the_same_colours() {
        let there = Conversion::new(model(Builtin::Srgb), model(Builtin::DisplayP3), RenderingIntent::RelativeColorimetric, false);
        let back = Conversion::new(model(Builtin::DisplayP3), model(Builtin::Srgb), RenderingIntent::RelativeColorimetric, false);
        for c in [[0.0, 0.0, 0.0], [1.0, 1.0, 1.0], [0.8, 0.3, 0.1], [0.2, 0.5, 0.9], [0.5, 0.5, 0.5]] {
            let mid = there.apply(c).unwrap();
            let out = back.apply(mid).unwrap();
            for k in 0..3 {
                assert!((out[k] - c[k]).abs() < 3e-3, "{c:?} -> {mid:?} -> {out:?}");
            }
        }
    }

    #[test]
    fn srgb_red_is_inside_display_p3() {
        // sRGB pure red in Display P3, as the two profiles' matrices give it (about 234, 51, 35 of 255).
        let conv = Conversion::new(model(Builtin::Srgb), model(Builtin::DisplayP3), RenderingIntent::RelativeColorimetric, false);
        let out = conv.apply([1.0, 0.0, 0.0]).unwrap();
        assert!((out[0] - 0.918).abs() < 0.01 && (out[1] - 0.200).abs() < 0.01 && (out[2] - 0.137).abs() < 0.01, "{out:?}");
    }

    #[test]
    fn white_stays_white_under_relative_colorimetric() {
        let conv = Conversion::new(model(Builtin::Srgb), model(Builtin::AdobeRgb1998), RenderingIntent::RelativeColorimetric, true);
        let out = conv.apply([1.0, 1.0, 1.0]).unwrap();
        assert!(out.iter().all(|c| (c - 1.0).abs() < 2e-3), "{out:?}");
    }

    #[test]
    fn srgb_mid_grey_maps_to_gamma_2_2_grey() {
        // sRGB 0.5 is linear luminance 0.2140, which is 0.4963 on a gamma 2.2 grey profile.
        let conv = Conversion::new(model(Builtin::Srgb), model(Builtin::Gray22), RenderingIntent::RelativeColorimetric, false);
        let out = conv.apply([0.5, 0.5, 0.5]).unwrap();
        assert!((out[0] - 0.4963).abs() < 0.005, "{out:?}");
        assert!((out[0] - out[1]).abs() < 1e-9 && (out[1] - out[2]).abs() < 1e-9);
    }

    #[test]
    fn absolute_intent_with_equal_whites_still_keeps_white() {
        let conv = Conversion::new(model(Builtin::Srgb), model(Builtin::DisplayP3), RenderingIntent::AbsoluteColorimetric, false);
        let out = conv.apply([1.0, 1.0, 1.0]).unwrap();
        assert!(out.iter().all(|c| (c - 1.0).abs() < 2e-3), "{out:?}");
    }

    #[test]
    fn baked_table_agrees_with_the_direct_conversion() {
        let conv = Conversion::new(model(Builtin::DisplayP3), model(Builtin::Srgb), RenderingIntent::RelativeColorimetric, false);
        let table = Lut3d::build(&conv, 17).expect("table builds");
        for c in [[0.25, 0.5, 0.75], [0.9, 0.1, 0.4]] {
            let direct = conv.apply(c).unwrap();
            let baked = table.sample(c);
            for k in 0..3 {
                assert!((direct[k] - baked[k]).abs() < 0.01, "{direct:?} vs {baked:?}");
            }
        }
    }

    #[test]
    fn the_cache_returns_the_same_table_for_the_same_key() {
        let key = cache_key(&[b"cache-test-profile"], "unit");
        let build = || {
            let conv = Conversion::new(model(Builtin::Srgb), model(Builtin::Rec2020), RenderingIntent::RelativeColorimetric, false);
            Lut3d::build(&conv, 5)
        };
        let a = cached(key, build).unwrap();
        let b = cached(key, || None).unwrap();
        assert!(Arc::ptr_eq(&a, &b));
    }
}
