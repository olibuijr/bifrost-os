//! Colours and colour spaces.
//!
//! A [`ColorSpace`] says what the components of a [`Color`] mean: generic and device RGB, grey and CMYK, sRGB (plain,
//! extended-range and linear), a wide-gamut RGB space with the Display-P3 primaries, Rec. 709 and Rec. 2020, Lab,
//! indexed and pattern spaces, and spaces read from an ICC profile that stores a matrix and tone curves. Every space
//! that has a colorimetric definition maps to CIE XYZ relative to the D50 white, the connection space of ICC profiles;
//! a [`ColorConversionInfo`] chains two such mappings under a rendering intent.
//!
//! Device and generic spaces are fixed definitions here: device RGB is sRGB, generic RGB is the sRGB primaries with a
//! 2.2 power curve, grey spaces use a 2.2 power curve, and CMYK uses the naive subtractive formula through sRGB.
//! Perceptual and saturation intents behave as relative colorimetric (no gamut mapping tables are read).

use crate::colorspace::{inverse, Mat3};
use crate::icc::{IccError, Profile};
use std::sync::{Arc, OnceLock};

/// The D50 white as XYZ, the white of the connection space.
const D50: [f64; 3] = [0.9642, 1.0, 0.8249];

fn xyz_of(x: f64, y: f64) -> [f64; 3] {
    [x / y, 1.0, (1.0 - x - y) / y]
}

fn mul(a: Mat3, b: Mat3) -> Mat3 {
    let mut r = [[0.0; 3]; 3];
    for (i, row) in r.iter_mut().enumerate() {
        for (j, cell) in row.iter_mut().enumerate() {
            *cell = (0..3).map(|k| a[i][k] * b[k][j]).sum();
        }
    }
    r
}

fn apply(m: &Mat3, v: [f64; 3]) -> [f64; 3] {
    [0, 1, 2].map(|i| m[i][0] * v[0] + m[i][1] * v[1] + m[i][2] * v[2])
}

fn det(m: &Mat3) -> f64 {
    let [[a, b, c], [d, e, f], [g, h, i]] = *m;
    a * (e * i - f * h) - b * (d * i - f * g) + c * (d * h - e * g)
}

/// The matrix that adapts XYZ values from the white `src` to the white `dst` (Bradford cone response).
fn adapt(src: [f64; 3], dst: [f64; 3]) -> Mat3 {
    const B: Mat3 = [[0.8951, 0.2664, -0.1614], [-0.7502, 1.7135, 0.0367], [0.0389, -0.0685, 1.0296]];
    let s = apply(&B, src);
    let d = apply(&B, dst);
    let scale: Mat3 = [[d[0] / s[0], 0.0, 0.0], [0.0, d[1] / s[1], 0.0], [0.0, 0.0, d[2] / s[2]]];
    mul(inverse(B), mul(scale, B))
}

// ---------------------------------------------------------------------------------------------------------------
// tone curves

/// A tone reproduction curve: how an encoded channel in 0..=1 maps to linear light.
#[derive(Clone, Debug, PartialEq)]
pub enum Trc {
    Linear,
    /// `linear = encoded ^ gamma`.
    Gamma(f64),
    /// The piecewise sRGB curve.
    Srgb,
    /// The piecewise Rec. 709 / Rec. 2020 curve.
    Rec709,
    /// Evenly spaced samples of the decoding curve, linearly interpolated.
    Table(Vec<f32>),
    /// An ICC parametric curve: `kind` 0..=4, parameters `g a b c d e f` in that order.
    Parametric { kind: u8, p: [f64; 7] },
}

fn parametric(kind: u8, p: &[f64; 7], x: f64) -> f64 {
    let [g, a, b, c, d, e, f] = *p;
    let pw = |x: f64| (a * x + b).max(0.0).powf(g);
    match kind {
        0 => x.powf(g),
        1 => {
            if a != 0.0 && x >= -b / a {
                pw(x)
            } else {
                0.0
            }
        }
        2 => {
            if a != 0.0 && x >= -b / a {
                pw(x) + c
            } else {
                c
            }
        }
        3 => {
            if x >= d {
                pw(x)
            } else {
                c * x
            }
        }
        _ => {
            if x >= d {
                pw(x) + e
            } else {
                c * x + f
            }
        }
    }
}

impl Trc {
    /// Encoded value to linear light. Negative input is mirrored around zero.
    pub fn decode(&self, v: f64) -> f64 {
        if v < 0.0 {
            return -self.decode(-v);
        }
        match self {
            Trc::Linear => v,
            Trc::Gamma(g) => v.powf(*g),
            Trc::Srgb => {
                if v <= 0.04045 {
                    v / 12.92
                } else {
                    ((v + 0.055) / 1.055).powf(2.4)
                }
            }
            Trc::Rec709 => {
                if v < 0.081 {
                    v / 4.5
                } else {
                    ((v + 0.099) / 1.099).powf(1.0 / 0.45)
                }
            }
            Trc::Table(t) => {
                let n = t.len();
                if n == 0 {
                    return v;
                }
                if n == 1 {
                    return t[0] as f64;
                }
                let pos = v.min(1.0) * (n - 1) as f64;
                let i = (pos.floor() as usize).min(n - 2);
                let f = pos - i as f64;
                t[i] as f64 * (1.0 - f) + t[i + 1] as f64 * f
            }
            Trc::Parametric { kind, p } => parametric(*kind, p, v.min(1.0)),
        }
    }

    /// Linear light to an encoded value. Negative input is mirrored around zero.
    pub fn encode(&self, l: f64) -> f64 {
        if l < 0.0 {
            return -self.encode(-l);
        }
        match self {
            Trc::Linear => l,
            Trc::Gamma(g) => l.powf(1.0 / *g),
            Trc::Srgb => {
                if l <= 0.0031308 {
                    12.92 * l
                } else {
                    1.055 * l.powf(1.0 / 2.4) - 0.055
                }
            }
            Trc::Rec709 => {
                if l < 0.018 {
                    4.5 * l
                } else {
                    1.099 * l.powf(0.45) - 0.099
                }
            }
            Trc::Table(_) | Trc::Parametric { .. } => {
                // The curve is monotonic: bisect for the encoded value.
                if l >= self.decode(1.0) {
                    return 1.0;
                }
                let (mut lo, mut hi) = (0.0f64, 1.0f64);
                for _ in 0..40 {
                    let mid = (lo + hi) / 2.0;
                    if self.decode(mid) < l {
                        lo = mid;
                    } else {
                        hi = mid;
                    }
                }
                (lo + hi) / 2.0
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------
// colorimetric spaces

/// An RGB space defined by a matrix to XYZ (D50) and one tone curve per channel.
#[derive(Clone, Debug, PartialEq)]
pub struct RgbSpace {
    to_xyz: Mat3,
    from_xyz: Mat3,
    trc: [Trc; 3],
    /// The white of the medium in its own terms (XYZ, Y = 1), for absolute colorimetric conversion.
    media_white: [f64; 3],
    /// Whether values outside 0..=1 are kept.
    extended: bool,
    wide: bool,
}

/// The xy area of the sRGB gamut triangle.
const SRGB_AREA: f64 = 0.11205;

impl RgbSpace {
    /// A space from a matrix taking linear RGB to XYZ relative to D50. `None` when the matrix is singular.
    fn from_matrix(to_xyz: Mat3, trc: [Trc; 3], media_white: [f64; 3], extended: bool) -> Option<RgbSpace> {
        if !to_xyz.iter().flatten().all(|v| v.is_finite()) || det(&to_xyz).abs() < 1e-9 {
            return None;
        }
        let from_xyz = inverse(to_xyz);
        let mut pts = [(0.0f64, 0.0f64); 3];
        for (j, pt) in pts.iter_mut().enumerate() {
            let (x, y, z) = (to_xyz[0][j], to_xyz[1][j], to_xyz[2][j]);
            let s = x + y + z;
            if s.abs() < 1e-12 {
                return None;
            }
            *pt = (x / s, y / s);
        }
        let area = 0.5
            * (pts[0].0 * (pts[1].1 - pts[2].1) + pts[1].0 * (pts[2].1 - pts[0].1) + pts[2].0 * (pts[0].1 - pts[1].1)).abs();
        Some(RgbSpace { to_xyz, from_xyz, trc, media_white, extended, wide: area > SRGB_AREA * 1.05 })
    }

    /// A space from CIE xy primaries and white.
    fn from_primaries(prim: [(f64, f64); 3], white: (f64, f64), trc: Trc, extended: bool) -> RgbSpace {
        let p = prim.map(|(x, y)| xyz_of(x, y));
        let cols: Mat3 = [[p[0][0], p[1][0], p[2][0]], [p[0][1], p[1][1], p[2][1]], [p[0][2], p[1][2], p[2][2]]];
        let w = xyz_of(white.0, white.1);
        let s = apply(&inverse(cols), w);
        let mut native = cols;
        for row in native.iter_mut() {
            for (j, cell) in row.iter_mut().enumerate() {
                *cell *= s[j];
            }
        }
        let to_xyz = mul(adapt(w, D50), native);
        RgbSpace::from_matrix(to_xyz, [trc.clone(), trc.clone(), trc], w, extended).expect("fixed primaries are valid")
    }

    fn to_pcs(&self, c: &[f32]) -> [f64; 3] {
        let lin = [0, 1, 2].map(|i| {
            let v = c[i] as f64;
            self.trc[i].decode(if self.extended { v } else { v.clamp(0.0, 1.0) })
        });
        apply(&self.to_xyz, lin)
    }

    #[allow(clippy::wrong_self_convention)] // converts from the PCS into this space; the name mirrors `to_pcs`
    fn from_pcs(&self, xyz: [f64; 3]) -> [f32; 3] {
        let lin = apply(&self.from_xyz, xyz);
        [0, 1, 2].map(|i| {
            let e = self.trc[i].encode(lin[i]);
            (if self.extended { e } else { e.clamp(0.0, 1.0) }) as f32
        })
    }

    /// Like `from_pcs`, but a colour outside the gamut is compressed instead of clipped channel by channel: in linear
    /// light it moves along the straight line towards the grey of equal luminance until every channel fits. That
    /// keeps hue and lightness and only gives up saturation, which is what the perceptual intent asks for. Spaces
    /// that keep out-of-range values (the extended ones) are not compressed.
    fn from_pcs_mapped(&self, xyz: [f64; 3]) -> [f32; 3] {
        if self.extended {
            return self.from_pcs(xyz);
        }
        let lin = apply(&self.from_xyz, xyz);
        let eps = 1e-9;
        if lin.iter().all(|v| *v >= -eps && *v <= 1.0 + eps) {
            return self.from_pcs(xyz);
        }
        // the luminance of the colour, as the grey every channel shares; white is 1, so Y in 0..=1 is a valid grey
        let l = xyz[1].clamp(0.0, 1.0);
        let mut t = 1.0f64;
        for v in lin {
            if v > 1.0 && v - l > 1e-12 {
                t = t.min((1.0 - l) / (v - l));
            } else if v < 0.0 && l - v > 1e-12 {
                t = t.min(l / (l - v));
            }
        }
        let t = t.clamp(0.0, 1.0);
        let mapped = lin.map(|v| (l + t * (v - l)).clamp(0.0, 1.0));
        [0, 1, 2].map(|i| self.trc[i].encode(mapped[i]).clamp(0.0, 1.0) as f32)
    }

    /// Whether the gamut is larger than sRGB's.
    pub fn is_wide_gamut(&self) -> bool {
        self.wide
    }
}

/// A grey space: one channel and a tone curve, white at D50.
#[derive(Clone, Debug, PartialEq)]
pub struct GraySpace {
    trc: Trc,
}

/// Lab: lightness 0..100 and two opponent axes, relative to a white point.
#[derive(Clone, Debug, PartialEq)]
pub struct LabSpace {
    white: [f64; 3],
    /// Range of the a axis then the b axis: `[a_min, a_max, b_min, b_max]`.
    range: [f32; 4],
}

/// An indexed space: each index selects `base.num_components()` bytes from `table`.
#[derive(Clone, Debug, PartialEq)]
pub struct IndexedSpace {
    base: ColorSpace,
    hival: usize,
    table: Vec<u8>,
}

/// The colorimetric part of a space read from a profile.
#[derive(Clone, Debug, PartialEq)]
#[allow(clippy::large_enum_variant)] // public variants are constructed and matched directly; boxing would change the API
pub enum ProfileBody {
    Rgb(RgbSpace),
    Gray(GraySpace),
}

/// A space defined by an ICC profile.
#[derive(Clone, Debug, PartialEq)]
pub struct ProfileSpace {
    name: String,
    body: ProfileBody,
}

/// Why a profile could not become a space.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProfileError {
    Icc(IccError),
    /// The profile is valid but has no matrix and tone curves this module reads (a lookup-table profile).
    Unsupported(&'static str),
    /// A required tag is missing or its data is malformed.
    Malformed(&'static str),
}

/// How the components of a colour are laid out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColorModel {
    Monochrome,
    Rgb,
    Cmyk,
    Lab,
    Indexed,
    Pattern,
}

/// The rendering intent of a conversion.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum RenderingIntent {
    #[default]
    Default,
    AbsoluteColorimetric,
    RelativeColorimetric,
    Perceptual,
    Saturation,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ColorSpace {
    DeviceRgb,
    DeviceGray,
    DeviceCmyk,
    GenericRgb,
    GenericGray,
    GenericCmyk,
    Srgb,
    ExtendedSrgb,
    LinearSrgb,
    ExtendedLinearSrgb,
    /// Wide gamut: the Display-P3 primaries and the sRGB curve.
    DisplayP3,
    Rec709,
    Rec2020,
    GenericGrayGamma22,
    Lab(Arc<LabSpace>),
    Indexed(Arc<IndexedSpace>),
    /// A pattern space, with the space the pattern's own colour is given in when the pattern is uncoloured.
    Pattern(Option<Box<ColorSpace>>),
    Icc(Arc<ProfileSpace>),
}

enum Core<'a> {
    Rgb(&'a RgbSpace),
    Gray(&'a GraySpace),
}

fn std_rgb(i: usize) -> &'static RgbSpace {
    static STD: OnceLock<Vec<RgbSpace>> = OnceLock::new();
    let all = STD.get_or_init(|| {
        const D65: (f64, f64) = (0.3127, 0.3290);
        let srgb_prim = [(0.64, 0.33), (0.30, 0.60), (0.15, 0.06)];
        vec![
            RgbSpace::from_primaries(srgb_prim, D65, Trc::Srgb, false),
            RgbSpace::from_primaries(srgb_prim, D65, Trc::Srgb, true),
            RgbSpace::from_primaries(srgb_prim, D65, Trc::Linear, false),
            RgbSpace::from_primaries(srgb_prim, D65, Trc::Linear, true),
            RgbSpace::from_primaries([(0.680, 0.320), (0.265, 0.690), (0.150, 0.060)], D65, Trc::Srgb, false),
            RgbSpace::from_primaries(srgb_prim, D65, Trc::Rec709, false),
            RgbSpace::from_primaries([(0.708, 0.292), (0.170, 0.797), (0.131, 0.046)], D65, Trc::Rec709, false),
            RgbSpace::from_primaries(srgb_prim, D65, Trc::Gamma(2.2), false),
        ]
    });
    &all[i]
}

fn std_gray() -> &'static GraySpace {
    static G: OnceLock<GraySpace> = OnceLock::new();
    G.get_or_init(|| GraySpace { trc: Trc::Gamma(2.2) })
}

fn clamp01(v: f64) -> f32 {
    v.clamp(0.0, 1.0) as f32
}

impl ColorSpace {
    /// A Lab space with the given white point (XYZ) and a/b ranges.
    pub fn lab(white: [f64; 3], range: [f32; 4]) -> ColorSpace {
        ColorSpace::Lab(Arc::new(LabSpace { white, range }))
    }

    /// Lab relative to D50 with a and b in -128..=127.
    pub fn lab_d50() -> ColorSpace {
        ColorSpace::lab(D50, [-128.0, 127.0, -128.0, 127.0])
    }

    /// An indexed space over `base`. `table` holds `(hival + 1) * base.num_components()` bytes. `None` when the base
    /// is indexed or a pattern space or the table is too short.
    pub fn indexed(base: ColorSpace, hival: usize, table: Vec<u8>) -> Option<ColorSpace> {
        if matches!(base, ColorSpace::Indexed(_) | ColorSpace::Pattern(_)) {
            return None;
        }
        if table.len() < (hival + 1) * base.num_components() {
            return None;
        }
        Some(ColorSpace::Indexed(Arc::new(IndexedSpace { base, hival, table })))
    }

    /// A pattern space; give the underlying space for uncoloured patterns.
    pub fn pattern(base: Option<ColorSpace>) -> ColorSpace {
        ColorSpace::Pattern(base.map(Box::new))
    }

    /// A space from an ICC profile that stores a matrix and tone curves (RGB) or one tone curve (grey).
    pub fn from_icc(data: &[u8]) -> Result<ColorSpace, ProfileError> {
        let profile = Profile::parse(data).map_err(ProfileError::Icc)?;
        let name = profile.description(data).unwrap_or_else(|| "ICC profile".to_string());
        let tag = |sig: &str| -> Option<&[u8]> {
            let t = profile.tags.iter().find(|t| t.signature == sig)?;
            data.get(t.offset as usize..t.offset as usize + t.size as usize)
        };
        let body = match profile.header.color_space.as_str() {
            "RGB " => {
                let mut cols = [[0.0f64; 3]; 3];
                for (j, sig) in ["rXYZ", "gXYZ", "bXYZ"].iter().enumerate() {
                    let v = parse_xyz(tag(sig).ok_or(unsupported_or(&profile, "missing colorant tag"))?)
                        .ok_or(ProfileError::Malformed("colorant tag"))?;
                    for i in 0..3 {
                        cols[i][j] = v[i];
                    }
                }
                let mut trc: Vec<Trc> = Vec::new();
                for sig in ["rTRC", "gTRC", "bTRC"] {
                    let body = tag(sig).ok_or(unsupported_or(&profile, "missing tone curve tag"))?;
                    trc.push(parse_curve(body).ok_or(ProfileError::Malformed("tone curve tag"))?);
                }
                let white = tag("wtpt").and_then(parse_xyz).unwrap_or(D50);
                let trc: [Trc; 3] = [trc[0].clone(), trc[1].clone(), trc[2].clone()];
                ProfileBody::Rgb(
                    RgbSpace::from_matrix(cols, trc, white, false).ok_or(ProfileError::Malformed("colorant matrix"))?,
                )
            }
            "GRAY" => {
                let body = tag("kTRC").ok_or(unsupported_or(&profile, "missing grey tone curve"))?;
                ProfileBody::Gray(GraySpace { trc: parse_curve(body).ok_or(ProfileError::Malformed("tone curve tag"))? })
            }
            _ => return Err(ProfileError::Unsupported("colour model")),
        };
        Ok(ColorSpace::Icc(Arc::new(ProfileSpace { name, body })))
    }

    fn core(&self) -> Option<Core<'_>> {
        Some(match self {
            ColorSpace::DeviceRgb | ColorSpace::Srgb => Core::Rgb(std_rgb(0)),
            ColorSpace::ExtendedSrgb => Core::Rgb(std_rgb(1)),
            ColorSpace::LinearSrgb => Core::Rgb(std_rgb(2)),
            ColorSpace::ExtendedLinearSrgb => Core::Rgb(std_rgb(3)),
            ColorSpace::DisplayP3 => Core::Rgb(std_rgb(4)),
            ColorSpace::Rec709 => Core::Rgb(std_rgb(5)),
            ColorSpace::Rec2020 => Core::Rgb(std_rgb(6)),
            ColorSpace::GenericRgb => Core::Rgb(std_rgb(7)),
            ColorSpace::DeviceGray | ColorSpace::GenericGray | ColorSpace::GenericGrayGamma22 => Core::Gray(std_gray()),
            ColorSpace::Icc(p) => match &p.body {
                ProfileBody::Rgb(r) => Core::Rgb(r),
                ProfileBody::Gray(g) => Core::Gray(g),
            },
            _ => return None,
        })
    }

    /// A short human-readable name.
    pub fn name(&self) -> &str {
        match self {
            ColorSpace::DeviceRgb => "Device RGB",
            ColorSpace::DeviceGray => "Device Gray",
            ColorSpace::DeviceCmyk => "Device CMYK",
            ColorSpace::GenericRgb => "Generic RGB",
            ColorSpace::GenericGray => "Generic Gray",
            ColorSpace::GenericCmyk => "Generic CMYK",
            ColorSpace::Srgb => "sRGB",
            ColorSpace::ExtendedSrgb => "Extended sRGB",
            ColorSpace::LinearSrgb => "Linear sRGB",
            ColorSpace::ExtendedLinearSrgb => "Extended Linear sRGB",
            ColorSpace::DisplayP3 => "Display P3",
            ColorSpace::Rec709 => "Rec. 709",
            ColorSpace::Rec2020 => "Rec. 2020",
            ColorSpace::GenericGrayGamma22 => "Generic Gray Gamma 2.2",
            ColorSpace::Lab(_) => "Lab",
            ColorSpace::Indexed(_) => "Indexed",
            ColorSpace::Pattern(_) => "Pattern",
            ColorSpace::Icc(p) => &p.name,
        }
    }

    pub fn model(&self) -> ColorModel {
        match self {
            ColorSpace::DeviceGray | ColorSpace::GenericGray | ColorSpace::GenericGrayGamma22 => ColorModel::Monochrome,
            ColorSpace::DeviceCmyk | ColorSpace::GenericCmyk => ColorModel::Cmyk,
            ColorSpace::Lab(_) => ColorModel::Lab,
            ColorSpace::Indexed(_) => ColorModel::Indexed,
            ColorSpace::Pattern(_) => ColorModel::Pattern,
            ColorSpace::Icc(p) => match p.body {
                ProfileBody::Rgb(_) => ColorModel::Rgb,
                ProfileBody::Gray(_) => ColorModel::Monochrome,
            },
            _ => ColorModel::Rgb,
        }
    }

    /// The number of colour components, without alpha. A pattern space counts the components of its base space.
    pub fn num_components(&self) -> usize {
        match self.model() {
            ColorModel::Monochrome | ColorModel::Indexed => 1,
            ColorModel::Rgb | ColorModel::Lab => 3,
            ColorModel::Cmyk => 4,
            ColorModel::Pattern => match self {
                ColorSpace::Pattern(Some(b)) => b.num_components(),
                _ => 0,
            },
        }
    }

    /// Whether this is an RGB space whose gamut is larger than sRGB's.
    pub fn is_wide_gamut(&self) -> bool {
        matches!(self.core(), Some(Core::Rgb(r)) if r.wide)
    }

    /// Whether component values outside 0..=1 are meaningful.
    pub fn supports_extended_range(&self) -> bool {
        matches!(self.core(), Some(Core::Rgb(r)) if r.extended)
    }

    /// The model of the base space of an indexed or uncoloured pattern space.
    pub fn base(&self) -> Option<&ColorSpace> {
        match self {
            ColorSpace::Indexed(ix) => Some(&ix.base),
            ColorSpace::Pattern(Some(b)) => Some(b.as_ref()),
            _ => None,
        }
    }

    /// The colour as XYZ relative to D50, or `None` for a pattern space or when `c` is too short.
    pub fn to_xyz(&self, c: &[f32]) -> Option<[f64; 3]> {
        if matches!(self, ColorSpace::Pattern(_)) || c.len() < self.num_components() {
            return None;
        }
        match self {
            ColorSpace::DeviceCmyk | ColorSpace::GenericCmyk => {
                let k = 1.0 - c[3];
                let rgb = [(1.0 - c[0]) * k, (1.0 - c[1]) * k, (1.0 - c[2]) * k];
                Some(std_rgb(0).to_pcs(&rgb))
            }
            ColorSpace::Lab(l) => {
                let fy = (c[0] as f64 + 16.0) / 116.0;
                let fx = fy + c[1] as f64 / 500.0;
                let fz = fy - c[2] as f64 / 200.0;
                let inv = |t: f64| if t > 6.0 / 29.0 { t * t * t } else { 3.0 * (6.0f64 / 29.0).powi(2) * (t - 4.0 / 29.0) };
                let xyz = [l.white[0] * inv(fx), l.white[1] * inv(fy), l.white[2] * inv(fz)];
                Some([xyz[0] * D50[0] / l.white[0], xyz[1] * D50[1] / l.white[1], xyz[2] * D50[2] / l.white[2]])
            }
            ColorSpace::Indexed(ix) => {
                let n = ix.base.num_components();
                let i = (c[0].round().max(0.0) as usize).min(ix.hival);
                let comps: Vec<f32> = (0..n).map(|k| ix.base_component(k, ix.table[i * n + k])).collect();
                ix.base.to_xyz(&comps)
            }
            _ => match self.core()? {
                Core::Rgb(r) => Some(r.to_pcs(c)),
                Core::Gray(g) => {
                    let y = g.trc.decode((c[0] as f64).clamp(0.0, 1.0));
                    Some([D50[0] * y, y, D50[2] * y])
                }
            },
        }
    }

    /// The components that show `xyz` (relative to D50) in this space. `None` for pattern and indexed spaces.
    pub fn from_xyz(&self, xyz: [f64; 3]) -> Option<Vec<f32>> {
        match self {
            ColorSpace::Pattern(_) | ColorSpace::Indexed(_) => None,
            ColorSpace::DeviceCmyk | ColorSpace::GenericCmyk => {
                let rgb = std_rgb(0).from_pcs(xyz);
                let k = 1.0 - rgb[0].max(rgb[1]).max(rgb[2]);
                if k >= 1.0 {
                    return Some(vec![0.0, 0.0, 0.0, 1.0]);
                }
                let d = 1.0 - k;
                Some(vec![(1.0 - rgb[0] - k) / d, (1.0 - rgb[1] - k) / d, (1.0 - rgb[2] - k) / d, k])
            }
            ColorSpace::Lab(l) => {
                let t = [xyz[0] * l.white[0] / D50[0], xyz[1] * l.white[1] / D50[1], xyz[2] * l.white[2] / D50[2]];
                let f = |v: f64| if v > (6.0f64 / 29.0).powi(3) { v.cbrt() } else { v / (3.0 * (6.0f64 / 29.0).powi(2)) + 4.0 / 29.0 };
                let (fx, fy, fz) = (f(t[0] / l.white[0]), f(t[1] / l.white[1]), f(t[2] / l.white[2]));
                let (a, b) = (500.0 * (fx - fy), 200.0 * (fy - fz));
                Some(vec![
                    (116.0 * fy - 16.0).clamp(0.0, 100.0) as f32,
                    (a as f32).clamp(l.range[0], l.range[1]),
                    (b as f32).clamp(l.range[2], l.range[3]),
                ])
            }
            _ => match self.core()? {
                Core::Rgb(r) => Some(r.from_pcs(xyz).to_vec()),
                Core::Gray(g) => Some(vec![clamp01(g.trc.encode(xyz[1].max(0.0)))]),
            },
        }
    }
}

impl ColorSpace {
    /// Like [`ColorSpace::from_xyz`], but a colour outside an RGB space's gamut is desaturated towards the grey of
    /// the same luminance rather than clipped per channel (the perceptual rendering intent). Every other kind of
    /// space, and the extended-range RGB spaces, behave as `from_xyz`.
    pub fn from_xyz_gamut_mapped(&self, xyz: [f64; 3]) -> Option<Vec<f32>> {
        match self {
            ColorSpace::Pattern(_)
            | ColorSpace::Indexed(_)
            | ColorSpace::DeviceCmyk
            | ColorSpace::GenericCmyk
            | ColorSpace::Lab(_) => self.from_xyz(xyz),
            _ => match self.core()? {
                Core::Rgb(r) => Some(r.from_pcs_mapped(xyz).to_vec()),
                Core::Gray(_) => self.from_xyz(xyz),
            },
        }
    }
}

impl IndexedSpace {
    /// Scales a table byte to the natural range of component `k` of the base space.
    fn base_component(&self, k: usize, byte: u8) -> f32 {
        let v = byte as f32 / 255.0;
        match &self.base {
            ColorSpace::Lab(l) => {
                if k == 0 {
                    v * 100.0
                } else {
                    let (lo, hi) = (l.range[(k - 1) * 2], l.range[(k - 1) * 2 + 1]);
                    lo + v * (hi - lo)
                }
            }
            _ => v,
        }
    }
}

fn unsupported_or(profile: &Profile, what: &'static str) -> ProfileError {
    if profile.has_tag("A2B0") || profile.has_tag("A2B1") {
        ProfileError::Unsupported("lookup-table profile")
    } else {
        ProfileError::Malformed(what)
    }
}

fn be_u32(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

fn be_u16(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_be_bytes(b.get(at..at + 2)?.try_into().ok()?))
}

fn s15f16(b: &[u8], at: usize) -> Option<f64> {
    Some(be_u32(b, at)? as i32 as f64 / 65536.0)
}

fn parse_xyz(b: &[u8]) -> Option<[f64; 3]> {
    if b.get(0..4)? != b"XYZ " {
        return None;
    }
    Some([s15f16(b, 8)?, s15f16(b, 12)?, s15f16(b, 16)?])
}

fn parse_curve(b: &[u8]) -> Option<Trc> {
    match b.get(0..4)? {
        b"curv" => {
            let n = be_u32(b, 8)? as usize;
            match n {
                0 => Some(Trc::Linear),
                1 => Some(Trc::Gamma(be_u16(b, 12)? as f64 / 256.0)),
                _ => {
                    let mut t = Vec::with_capacity(n);
                    for i in 0..n {
                        t.push(be_u16(b, 12 + i * 2)? as f32 / 65535.0);
                    }
                    Some(Trc::Table(t))
                }
            }
        }
        b"para" => {
            let kind = be_u16(b, 8)? as u8;
            let count = match kind {
                0 => 1,
                1 => 3,
                2 => 4,
                3 => 5,
                4 => 7,
                _ => return None,
            };
            let mut p = [0.0f64; 7];
            for (i, slot) in p.iter_mut().enumerate().take(count) {
                *slot = s15f16(b, 12 + i * 4)?;
            }
            Some(Trc::Parametric { kind, p })
        }
        _ => None,
    }
}

// ---------------------------------------------------------------------------------------------------------------
// conversion

/// A prepared conversion between two spaces.
#[derive(Clone, Debug, PartialEq)]
pub struct ColorConversionInfo {
    src: ColorSpace,
    dst: ColorSpace,
    intent: RenderingIntent,
    /// Per-channel XYZ scale for absolute colorimetric intent, `[1; 3]` otherwise.
    abs_scale: [f64; 3],
}

fn media_white(s: &ColorSpace) -> [f64; 3] {
    match s.core() {
        Some(Core::Rgb(r)) => r.media_white,
        _ => D50,
    }
}

impl ColorConversionInfo {
    /// A conversion from `src` to `dst`. `None` when either is a pattern space or `dst` is indexed.
    pub fn new(src: &ColorSpace, dst: &ColorSpace, intent: RenderingIntent) -> Option<ColorConversionInfo> {
        if matches!(src, ColorSpace::Pattern(_)) || matches!(dst, ColorSpace::Pattern(_) | ColorSpace::Indexed(_)) {
            return None;
        }
        let abs_scale = if intent == RenderingIntent::AbsoluteColorimetric {
            let (s, d) = (media_white(src), media_white(dst));
            [s[0] / d[0], s[1] / d[1], s[2] / d[2]]
        } else {
            [1.0; 3]
        };
        Some(ColorConversionInfo { src: src.clone(), dst: dst.clone(), intent, abs_scale })
    }

    pub fn source(&self) -> &ColorSpace {
        &self.src
    }

    pub fn destination(&self) -> &ColorSpace {
        &self.dst
    }

    pub fn intent(&self) -> RenderingIntent {
        self.intent
    }

    /// Converts the components `c` of the source space. `None` when `c` is too short.
    pub fn apply(&self, c: &[f32]) -> Option<Vec<f32>> {
        let mut xyz = self.src.to_xyz(c)?;
        for (x, s) in xyz.iter_mut().zip(self.abs_scale) {
            *x *= s;
        }
        // Perceptual compresses out-of-gamut colours towards grey; every other intent clips (the matrix profiles
        // here carry no saturation or perceptual tables, so saturation is the same as relative colorimetric).
        if self.intent == RenderingIntent::Perceptual {
            self.dst.from_xyz_gamut_mapped(xyz)
        } else {
            self.dst.from_xyz(xyz)
        }
    }

    /// Converts a colour given as three components to three components (both spaces three-component).
    pub fn apply_rgb(&self, c: [f32; 3]) -> Option<[f32; 3]> {
        let out = self.apply(&c)?;
        Some([*out.first()?, *out.get(1)?, *out.get(2)?])
    }
}

// ---------------------------------------------------------------------------------------------------------------
// colours

/// A colour: components in a space, and an alpha.
#[derive(Clone, Debug, PartialEq)]
pub struct Color {
    space: ColorSpace,
    comps: Vec<f32>,
    alpha: f32,
}

impl Color {
    /// A colour from `comps` (exactly `space.num_components()` of them) and an alpha clamped to 0..=1. `None` for a
    /// wrong component count.
    pub fn new(space: ColorSpace, comps: &[f32], alpha: f32) -> Option<Color> {
        if comps.len() != space.num_components() {
            return None;
        }
        Some(Color { space, comps: comps.to_vec(), alpha: if alpha.is_nan() { 1.0 } else { alpha.clamp(0.0, 1.0) } })
    }

    pub fn generic_rgb(r: f32, g: f32, b: f32, a: f32) -> Color {
        Color::new(ColorSpace::GenericRgb, &[r, g, b], a).expect("three components")
    }

    pub fn generic_gray(g: f32, a: f32) -> Color {
        Color::new(ColorSpace::GenericGray, &[g], a).expect("one component")
    }

    pub fn generic_cmyk(c: f32, m: f32, y: f32, k: f32, a: f32) -> Color {
        Color::new(ColorSpace::GenericCmyk, &[c, m, y, k], a).expect("four components")
    }

    pub fn srgb(r: f32, g: f32, b: f32, a: f32) -> Color {
        Color::new(ColorSpace::Srgb, &[r, g, b], a).expect("three components")
    }

    /// An sRGB colour from straight 8-bit channels.
    pub fn from_rgba8(r: u8, g: u8, b: u8, a: u8) -> Color {
        Color::srgb(r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, a as f32 / 255.0)
    }

    pub fn white() -> Color {
        Color::generic_gray(1.0, 1.0)
    }

    pub fn black() -> Color {
        Color::generic_gray(0.0, 1.0)
    }

    /// Fully transparent.
    pub fn clear() -> Color {
        Color::generic_gray(0.0, 0.0)
    }

    pub fn space(&self) -> &ColorSpace {
        &self.space
    }

    /// The colour components, without alpha.
    pub fn components(&self) -> &[f32] {
        &self.comps
    }

    pub fn alpha(&self) -> f32 {
        self.alpha
    }

    pub fn num_components(&self) -> usize {
        self.comps.len()
    }

    /// The same colour with another alpha (clamped to 0..=1).
    pub fn with_alpha(&self, alpha: f32) -> Color {
        Color { space: self.space.clone(), comps: self.comps.clone(), alpha: if alpha.is_nan() { 1.0 } else { alpha.clamp(0.0, 1.0) } }
    }

    /// The colour in another space under `intent`. `None` for pattern spaces and indexed targets.
    pub fn converted(&self, to: &ColorSpace, intent: RenderingIntent) -> Option<Color> {
        if &self.space == to {
            return Some(self.clone());
        }
        let info = ColorConversionInfo::new(&self.space, to, intent)?;
        Some(Color { space: to.clone(), comps: info.apply(&self.comps)?, alpha: self.alpha })
    }

    /// Straight sRGB 0..=1 channels and alpha. `None` for pattern colours.
    pub fn to_srgb(&self) -> Option<[f32; 4]> {
        let c = self.converted(&ColorSpace::Srgb, RenderingIntent::Default)?;
        Some([c.comps[0], c.comps[1], c.comps[2], self.alpha])
    }

    /// Straight 8-bit sRGB. A pattern colour gives transparent black.
    pub fn to_rgba8(&self) -> [u8; 4] {
        match self.to_srgb() {
            Some(c) => c.map(|v| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8),
            None => [0, 0, 0, 0],
        }
    }

    /// The premultiplied pixel the software rasteriser stores (red in the low byte).
    pub fn to_premul(&self) -> u32 {
        let [r, g, b, a] = self.to_rgba8();
        crate::gfx::rgba(r, g, b, a)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn near(a: f32, b: f32, tol: f32) -> bool {
        (a - b).abs() <= tol
    }

    #[test]
    fn component_counts_and_models() {
        assert_eq!(ColorSpace::Srgb.num_components(), 3);
        assert_eq!(ColorSpace::DeviceGray.num_components(), 1);
        assert_eq!(ColorSpace::GenericCmyk.num_components(), 4);
        assert_eq!(ColorSpace::lab_d50().num_components(), 3);
        assert_eq!(ColorSpace::pattern(None).num_components(), 0);
        assert_eq!(ColorSpace::pattern(Some(ColorSpace::Srgb)).num_components(), 3);
        assert_eq!(ColorSpace::GenericGrayGamma22.model(), ColorModel::Monochrome);
        assert_eq!(ColorSpace::Rec2020.model(), ColorModel::Rgb);
        assert_eq!(ColorSpace::DisplayP3.name(), "Display P3");
    }

    #[test]
    fn wide_gamut_flag() {
        assert!(ColorSpace::DisplayP3.is_wide_gamut());
        assert!(ColorSpace::Rec2020.is_wide_gamut());
        assert!(!ColorSpace::Srgb.is_wide_gamut());
        assert!(!ColorSpace::Rec709.is_wide_gamut());
        assert!(!ColorSpace::GenericGray.is_wide_gamut());
        assert!(ColorSpace::ExtendedSrgb.supports_extended_range());
        assert!(!ColorSpace::Srgb.supports_extended_range());
    }

    #[test]
    fn white_maps_to_white_in_every_rgb_space() {
        for to in [ColorSpace::DisplayP3, ColorSpace::Rec2020, ColorSpace::LinearSrgb, ColorSpace::GenericRgb] {
            let c = Color::srgb(1.0, 1.0, 1.0, 1.0).converted(&to, RenderingIntent::Default).unwrap();
            for v in c.components() {
                assert!(near(*v, 1.0, 0.002), "{}: {:?}", to.name(), c.components());
            }
        }
    }

    #[test]
    fn srgb_red_in_display_p3() {
        let c = Color::srgb(1.0, 0.0, 0.0, 1.0).converted(&ColorSpace::DisplayP3, RenderingIntent::Default).unwrap();
        let v = c.components();
        assert!(near(v[0], 0.9175, 0.005) && near(v[1], 0.2003, 0.005) && near(v[2], 0.1386, 0.005), "{v:?}");
    }

    #[test]
    fn p3_green_is_out_of_gamut_for_srgb_and_clamps() {
        let p3 = Color::new(ColorSpace::DisplayP3, &[0.0, 1.0, 0.0], 1.0).unwrap();
        let clamped = p3.converted(&ColorSpace::Srgb, RenderingIntent::Default).unwrap();
        assert!(clamped.components().iter().all(|v| (0.0..=1.0).contains(v)));
        let ext = p3.converted(&ColorSpace::ExtendedSrgb, RenderingIntent::Default).unwrap();
        assert!(ext.components()[0] < -0.1, "{:?}", ext.components());
    }

    #[test]
    fn perceptual_intent_compresses_out_of_gamut_colours_instead_of_clipping() {
        let p3 = Color::new(ColorSpace::DisplayP3, &[0.0, 1.0, 0.0], 1.0).unwrap();
        let clipped = p3.converted(&ColorSpace::Srgb, RenderingIntent::RelativeColorimetric).unwrap();
        let mapped = p3.converted(&ColorSpace::Srgb, RenderingIntent::Perceptual).unwrap();
        assert!(mapped.components().iter().all(|v| (0.0..=1.0).contains(v)), "{:?}", mapped.components());
        // clipping leaves blue at zero; compression gives up some saturation, so blue rises and green drops
        assert!(mapped.components()[2] > clipped.components()[2] + 0.05, "{:?} {:?}", mapped.components(), clipped.components());
        assert!(mapped.components()[1] < clipped.components()[1], "{:?}", mapped.components());
        assert!(mapped.components()[1] > mapped.components()[0] && mapped.components()[1] > mapped.components()[2], "hue is kept");
    }

    #[test]
    fn perceptual_intent_leaves_in_gamut_colours_and_greys_alone() {
        let c = Color::srgb(0.2, 0.6, 0.4, 1.0);
        let rel = c.converted(&ColorSpace::DisplayP3, RenderingIntent::RelativeColorimetric).unwrap();
        let per = c.converted(&ColorSpace::DisplayP3, RenderingIntent::Perceptual).unwrap();
        for (a, b) in rel.components().iter().zip(per.components()) {
            assert!(near(*a, *b, 1e-4), "{a} {b}");
        }
        let grey = Color::new(ColorSpace::DisplayP3, &[0.5, 0.5, 0.5], 1.0).unwrap();
        let g = grey.converted(&ColorSpace::Srgb, RenderingIntent::Perceptual).unwrap();
        assert!(g.components().iter().all(|v| near(*v, 0.5, 2e-3)), "{:?}", g.components());
        // extended-range targets keep out-of-range values whatever the intent
        let p3 = Color::new(ColorSpace::DisplayP3, &[0.0, 1.0, 0.0], 1.0).unwrap();
        let ext = p3.converted(&ColorSpace::ExtendedSrgb, RenderingIntent::Perceptual).unwrap();
        assert!(ext.components()[0] < -0.1);
    }

    #[test]
    fn linear_srgb_round_trip() {
        let c = Color::srgb(0.5, 0.25, 0.75, 1.0);
        let lin = c.converted(&ColorSpace::LinearSrgb, RenderingIntent::Default).unwrap();
        assert!(near(lin.components()[0], 0.2140, 0.002));
        let back = lin.converted(&ColorSpace::Srgb, RenderingIntent::Default).unwrap();
        for (a, b) in back.components().iter().zip(c.components()) {
            assert!(near(*a, *b, 1e-4));
        }
    }

    #[test]
    fn gray_gamma_to_srgb() {
        let g = Color::generic_gray(0.5, 1.0).converted(&ColorSpace::Srgb, RenderingIntent::Default).unwrap();
        for v in g.components() {
            assert!(near(*v, 0.5037, 0.01), "{v}");
        }
    }

    #[test]
    fn cmyk_naive() {
        let c = Color::generic_cmyk(0.0, 0.0, 0.0, 1.0, 1.0).to_srgb().unwrap();
        assert!(c[0] < 0.01 && c[1] < 0.01 && c[2] < 0.01);
        let w = Color::generic_cmyk(0.0, 0.0, 0.0, 0.0, 1.0).to_srgb().unwrap();
        assert!(w[0] > 0.99);
        // and back
        let back = Color::srgb(1.0, 0.0, 0.0, 1.0).converted(&ColorSpace::DeviceCmyk, RenderingIntent::Default).unwrap();
        let v = back.components();
        assert!(near(v[0], 0.0, 1e-4) && near(v[1], 1.0, 1e-4) && near(v[2], 1.0, 1e-4) && near(v[3], 0.0, 1e-4), "{v:?}");
    }

    #[test]
    fn lab_white_and_black() {
        let white = Color::new(ColorSpace::lab_d50(), &[100.0, 0.0, 0.0], 1.0).unwrap().to_srgb().unwrap();
        assert!(white[..3].iter().all(|v| near(*v, 1.0, 0.01)), "{white:?}");
        let black = Color::new(ColorSpace::lab_d50(), &[0.0, 0.0, 0.0], 1.0).unwrap().to_srgb().unwrap();
        assert!(black[..3].iter().all(|v| near(*v, 0.0, 0.01)));
        let back = Color::srgb(0.2, 0.6, 0.4, 1.0).converted(&ColorSpace::lab_d50(), RenderingIntent::Default).unwrap();
        let again = back.converted(&ColorSpace::Srgb, RenderingIntent::Default).unwrap();
        assert!(near(again.components()[1], 0.6, 0.01));
    }

    #[test]
    fn indexed_looks_up_the_base() {
        let ix = ColorSpace::indexed(ColorSpace::Srgb, 1, vec![255, 0, 0, 0, 0, 255]).unwrap();
        let c = Color::new(ix.clone(), &[1.0], 1.0).unwrap();
        assert_eq!(c.to_rgba8(), [0, 0, 255, 255]);
        assert!(ColorSpace::indexed(ColorSpace::Srgb, 1, vec![0; 3]).is_none());
        assert!(Color::new(ix, &[0.0, 0.0], 1.0).is_none());
    }

    #[test]
    fn pattern_colours_do_not_convert() {
        assert!(ColorConversionInfo::new(&ColorSpace::pattern(None), &ColorSpace::Srgb, RenderingIntent::Default).is_none());
        let c = Color::new(ColorSpace::pattern(None), &[], 1.0).unwrap();
        assert_eq!(c.to_rgba8(), [0, 0, 0, 0]);
    }

    #[test]
    fn named_colours_alpha_and_equality() {
        assert_eq!(Color::white().to_rgba8(), [255, 255, 255, 255]);
        assert_eq!(Color::black().to_rgba8(), [0, 0, 0, 255]);
        assert_eq!(Color::clear().alpha(), 0.0);
        assert_eq!(Color::white().with_alpha(0.5).alpha(), 0.5);
        assert_eq!(Color::white().with_alpha(2.0).alpha(), 1.0);
        assert_eq!(Color::srgb(0.1, 0.2, 0.3, 1.0), Color::srgb(0.1, 0.2, 0.3, 1.0));
        assert_ne!(Color::srgb(0.1, 0.2, 0.3, 1.0), Color::generic_rgb(0.1, 0.2, 0.3, 1.0));
        assert!(Color::new(ColorSpace::Srgb, &[0.0], 1.0).is_none());
    }

    #[test]
    fn premultiplied_pixel() {
        let c = Color::from_rgba8(255, 0, 0, 128);
        assert_eq!(c.to_premul(), crate::gfx::rgba(255, 0, 0, 128));
    }

    #[test]
    fn absolute_intent_differs_only_when_whites_differ() {
        let a = ColorConversionInfo::new(&ColorSpace::Srgb, &ColorSpace::DisplayP3, RenderingIntent::AbsoluteColorimetric).unwrap();
        let r = ColorConversionInfo::new(&ColorSpace::Srgb, &ColorSpace::DisplayP3, RenderingIntent::RelativeColorimetric).unwrap();
        let (x, y) = (a.apply_rgb([0.3, 0.5, 0.7]).unwrap(), r.apply_rgb([0.3, 0.5, 0.7]).unwrap());
        for i in 0..3 {
            assert!(near(x[i], y[i], 1e-4), "same white, same result");
        }
    }

    /// A minimal matrix/TRC profile with the sRGB colorants (D50-adapted) and a gamma-2.2 curve.
    fn profile_bytes(gamma: bool) -> Vec<u8> {
        fn xyz(v: [f64; 3]) -> Vec<u8> {
            let mut b = b"XYZ \0\0\0\0".to_vec();
            for x in v {
                b.extend(((x * 65536.0).round() as i32).to_be_bytes());
            }
            b
        }
        let curve = if gamma {
            let mut b = b"curv\0\0\0\0".to_vec();
            b.extend(1u32.to_be_bytes());
            b.extend(((2.2f64 * 256.0).round() as u16).to_be_bytes());
            b.extend([0, 0]);
            b
        } else {
            let mut b = b"curv\0\0\0\0".to_vec();
            b.extend(0u32.to_be_bytes());
            b
        };
        let tags: Vec<(&str, Vec<u8>)> = vec![
            ("rXYZ", xyz([0.4360747, 0.2225045, 0.0139322])),
            ("gXYZ", xyz([0.3850649, 0.7168786, 0.0971045])),
            ("bXYZ", xyz([0.1430804, 0.0606169, 0.7141733])),
            ("rTRC", curve.clone()),
            ("gTRC", curve.clone()),
            ("bTRC", curve),
        ];
        let table_len = 4 + 12 * tags.len();
        let mut offset = 128 + table_len;
        let mut table = (tags.len() as u32).to_be_bytes().to_vec();
        let mut body: Vec<u8> = Vec::new();
        for (sig, data) in &tags {
            table.extend(sig.as_bytes());
            table.extend((offset as u32).to_be_bytes());
            table.extend((data.len() as u32).to_be_bytes());
            body.extend(data);
            offset += data.len();
        }
        let mut out = vec![0u8; 128];
        out.extend(table);
        out.extend(body);
        let size = out.len() as u32;
        out[0..4].copy_from_slice(&size.to_be_bytes());
        out[8] = 2;
        out[12..16].copy_from_slice(b"mntr");
        out[16..20].copy_from_slice(b"RGB ");
        out[20..24].copy_from_slice(b"XYZ ");
        out[36..40].copy_from_slice(b"acsp");
        out
    }

    #[test]
    fn icc_matrix_profile_becomes_a_space() {
        let space = ColorSpace::from_icc(&profile_bytes(true)).unwrap();
        assert_eq!(space.model(), ColorModel::Rgb);
        assert_eq!(space.num_components(), 3);
        assert!(!space.is_wide_gamut());
        let white = Color::new(space.clone(), &[1.0, 1.0, 1.0], 1.0).unwrap().to_srgb().unwrap();
        assert!(white[..3].iter().all(|v| near(*v, 1.0, 0.01)), "{white:?}");
        // The same primaries as sRGB: red stays pure red.
        let red = Color::new(space, &[1.0, 0.0, 0.0], 1.0).unwrap().to_srgb().unwrap();
        assert!(near(red[0], 1.0, 0.01) && near(red[1], 0.0, 0.01), "{red:?}");
    }

    #[test]
    fn icc_linear_profile_matches_linear_srgb() {
        let space = ColorSpace::from_icc(&profile_bytes(false)).unwrap();
        let c = Color::new(space, &[0.2140, 0.2140, 0.2140], 1.0).unwrap().to_srgb().unwrap();
        assert!(near(c[0], 0.5, 0.01), "{c:?}");
    }

    #[test]
    fn broken_profiles_are_refused() {
        assert!(matches!(ColorSpace::from_icc(&[0u8; 10]), Err(ProfileError::Icc(_))));
        let mut p = profile_bytes(true);
        p[36..40].copy_from_slice(b"nope");
        assert!(matches!(ColorSpace::from_icc(&p), Err(ProfileError::Icc(IccError::NotAProfile))));
        let mut p = profile_bytes(true);
        p[16..20].copy_from_slice(b"CMYK");
        assert!(matches!(ColorSpace::from_icc(&p), Err(ProfileError::Unsupported(_))));
    }

    #[test]
    fn parametric_curve_inverts() {
        let t = Trc::Parametric { kind: 3, p: [2.4, 1.0 / 1.055, 0.055 / 1.055, 1.0 / 12.92, 0.04045, 0.0, 0.0] };
        for v in [0.0, 0.02, 0.3, 0.8, 1.0] {
            let back = t.encode(t.decode(v));
            assert!((back - v).abs() < 1e-6, "{v} -> {back}");
        }
        let tab = Trc::Table(vec![0.0, 0.1, 1.0]);
        assert!((tab.encode(tab.decode(0.7)) - 0.7).abs() < 1e-6);
    }
}
