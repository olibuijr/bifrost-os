//! PDF colour spaces reduced to RGB: device spaces, calibrated and ICC-based
//! spaces (by component count), Lab, indexed, separation and DeviceN with
//! their tint transforms.

use super::doc::Doc;
use super::function::Function;
use super::obj::{Dict, Obj};
use super::sink::Rgb;
use std::rc::Rc;

#[derive(Clone, Debug)]
pub enum ColorSpace {
    Gray,
    Rgb,
    Cmyk,
    Lab { white: [f32; 3], range: [f32; 4] },
    /// ICC profiles are not applied: the component count picks the device space
    Icc { n: usize },
    Indexed { base: Box<ColorSpace>, hival: usize, table: Rc<Vec<u8>> },
    /// `none` marks a separation named /None, which paints nothing
    Separation { alt: Box<ColorSpace>, tint: Rc<Function>, none: bool },
    DeviceN { n: usize, alt: Box<ColorSpace>, tint: Rc<Function> },
    Pattern(Option<Box<ColorSpace>>),
}

fn clamp01(v: f32) -> f32 {
    v.clamp(0.0, 1.0)
}

fn srgb_encode(v: f32) -> f32 {
    let v = clamp01(v);
    if v <= 0.003_130_8 {
        12.92 * v
    } else {
        1.055 * v.powf(1.0 / 2.4) - 0.055
    }
}

impl ColorSpace {
    /// Resolves a colour space object. `res` is the resource dictionary's /ColorSpace entry for names.
    pub fn parse(doc: &Doc, o: &Obj, res: Option<&Dict>) -> Option<ColorSpace> {
        Self::parse_depth(doc, o, res, 0)
    }

    fn parse_depth(doc: &Doc, o: &Obj, res: Option<&Dict>, depth: u8) -> Option<ColorSpace> {
        if depth > 6 {
            return None;
        }
        match doc.get(o) {
            Obj::Name(n) => match n.as_str() {
                "DeviceGray" | "G" | "CalGray" => Some(ColorSpace::Gray),
                "DeviceRGB" | "RGB" | "CalRGB" => Some(ColorSpace::Rgb),
                "DeviceCMYK" | "CMYK" => Some(ColorSpace::Cmyk),
                "Pattern" => Some(ColorSpace::Pattern(None)),
                other => {
                    let entry = res?.get(other)?.clone();
                    // a name that resolves to itself would loop
                    if matches!(&entry, Obj::Name(m) if m == other) {
                        return None;
                    }
                    Self::parse_depth(doc, &entry, res, depth + 1)
                }
            },
            Obj::Array(a) => {
                let head = doc.get(a.first()?);
                match head.name()? {
                    "ICCBased" => {
                        let s = doc.get(a.get(1)?);
                        let n = s.dict().map(|d| doc.field(d, "N").int().unwrap_or(3)).unwrap_or(3);
                        Some(ColorSpace::Icc { n: n.clamp(1, 4) as usize })
                    }
                    "CalGray" => Some(ColorSpace::Gray),
                    "CalRGB" => Some(ColorSpace::Rgb),
                    "Lab" => {
                        let d = doc.get(a.get(1)?).dict().cloned().unwrap_or_default();
                        let nums = |k: &str, def: &[f32]| -> Vec<f32> {
                            match doc.field(&d, k) {
                                Obj::Array(v) => v.iter().filter_map(|x| doc.get(x).num()).map(|x| x as f32).collect(),
                                _ => def.to_vec(),
                            }
                        };
                        let w = nums("WhitePoint", &[0.9642, 1.0, 0.8249]);
                        let r = nums("Range", &[-100.0, 100.0, -100.0, 100.0]);
                        Some(ColorSpace::Lab {
                            white: [*w.first().unwrap_or(&0.9642), *w.get(1).unwrap_or(&1.0), *w.get(2).unwrap_or(&0.8249)],
                            range: [*r.first().unwrap_or(&-100.0), *r.get(1).unwrap_or(&100.0), *r.get(2).unwrap_or(&-100.0), *r.get(3).unwrap_or(&100.0)],
                        })
                    }
                    "Indexed" | "I" => {
                        let base = Self::parse_depth(doc, a.get(1)?, res, depth + 1)?;
                        let hival = doc.get(a.get(2)?).int()?.clamp(0, 255) as usize;
                        let table = match doc.get(a.get(3)?) {
                            Obj::Str(s) => s,
                            Obj::Stream(s) => doc.decode(&s).0,
                            _ => return None,
                        };
                        Some(ColorSpace::Indexed { base: Box::new(base), hival, table: Rc::new(table) })
                    }
                    "Separation" => {
                        let none = doc.get(a.get(1)?).name() == Some("None");
                        let alt = Self::parse_depth(doc, a.get(2)?, res, depth + 1)?;
                        let tint = Function::parse(doc, a.get(3)?).unwrap_or(Function::Identity);
                        Some(ColorSpace::Separation { alt: Box::new(alt), tint: Rc::new(tint), none })
                    }
                    "DeviceN" => {
                        let n = doc.get(a.get(1)?).array().map(|x| x.len())?;
                        let alt = Self::parse_depth(doc, a.get(2)?, res, depth + 1)?;
                        let tint = Function::parse(doc, a.get(3)?).unwrap_or(Function::Identity);
                        Some(ColorSpace::DeviceN { n, alt: Box::new(alt), tint: Rc::new(tint) })
                    }
                    "Pattern" => {
                        let under = a.get(1).and_then(|u| Self::parse_depth(doc, u, res, depth + 1)).map(Box::new);
                        Some(ColorSpace::Pattern(under))
                    }
                    "DeviceGray" | "G" => Some(ColorSpace::Gray),
                    "DeviceRGB" | "RGB" => Some(ColorSpace::Rgb),
                    "DeviceCMYK" | "CMYK" => Some(ColorSpace::Cmyk),
                    _ => None,
                }
            }
            _ => None,
        }
    }

    /// Number of components of a colour in this space.
    pub fn n(&self) -> usize {
        match self {
            ColorSpace::Gray | ColorSpace::Indexed { .. } | ColorSpace::Separation { .. } => 1,
            ColorSpace::Rgb | ColorSpace::Lab { .. } => 3,
            ColorSpace::Cmyk => 4,
            ColorSpace::Icc { n } => *n,
            ColorSpace::DeviceN { n, .. } => *n,
            ColorSpace::Pattern(under) => under.as_ref().map_or(0, |u| u.n()),
        }
    }

    /// The colour a space starts with.
    pub fn initial(&self) -> Vec<f32> {
        match self {
            ColorSpace::Cmyk => vec![0.0, 0.0, 0.0, 1.0],
            ColorSpace::Icc { n: 4 } => vec![0.0, 0.0, 0.0, 1.0],
            ColorSpace::Separation { .. } => vec![1.0],
            ColorSpace::DeviceN { n, .. } => vec![1.0; *n],
            ColorSpace::Pattern(_) => vec![],
            other => vec![0.0; other.n()],
        }
    }

    /// The default `Decode` range of component `i` for images with `bpc` bits.
    pub fn default_decode(&self, i: usize, bpc: usize) -> (f32, f32) {
        match self {
            ColorSpace::Indexed { .. } => (0.0, ((1u32 << bpc.min(16)) - 1) as f32),
            ColorSpace::Lab { range, .. } => match i {
                0 => (0.0, 100.0),
                1 => (range[0], range[1]),
                _ => (range[2], range[3]),
            },
            _ => (0.0, 1.0),
        }
    }

    pub fn is_pattern(&self) -> bool {
        matches!(self, ColorSpace::Pattern(_))
    }

    /// True for a separation named /None (it paints nothing).
    pub fn paints_nothing(&self) -> bool {
        matches!(self, ColorSpace::Separation { none: true, .. })
    }

    pub fn to_rgb(&self, c: &[f32]) -> Rgb {
        let g = |i: usize| c.get(i).copied().unwrap_or(0.0);
        match self {
            ColorSpace::Gray => {
                let v = clamp01(g(0));
                [v, v, v]
            }
            ColorSpace::Rgb => [clamp01(g(0)), clamp01(g(1)), clamp01(g(2))],
            ColorSpace::Cmyk => {
                let (cc, m, y, k) = (clamp01(g(0)), clamp01(g(1)), clamp01(g(2)), clamp01(g(3)));
                [(1.0 - cc) * (1.0 - k), (1.0 - m) * (1.0 - k), (1.0 - y) * (1.0 - k)]
            }
            ColorSpace::Icc { n } => match n {
                1 => ColorSpace::Gray.to_rgb(c),
                4 => ColorSpace::Cmyk.to_rgb(c),
                _ => ColorSpace::Rgb.to_rgb(c),
            },
            ColorSpace::Lab { white, range } => {
                let l = g(0).clamp(0.0, 100.0);
                let a = g(1).clamp(range[0], range[1]);
                let b = g(2).clamp(range[2], range[3]);
                let m = (l + 16.0) / 116.0;
                let (lx, my, nz) = (m + a / 500.0, m, m - b / 200.0);
                let f = |x: f32| if x >= 6.0 / 29.0 { x * x * x } else { 108.0 / 841.0 * (x - 4.0 / 29.0) };
                let (x, y, z) = (white[0] * f(lx), white[1] * f(my), white[2] * f(nz));
                let r = 3.133_856 * x - 1.616_866_7 * y - 0.490_614_6 * z;
                let gg = -0.978_768_4 * x + 1.916_141_5 * y + 0.033_454 * z;
                let bb = 0.071_945_3 * x - 0.228_991_4 * y + 1.405_242_7 * z;
                [srgb_encode(r), srgb_encode(gg), srgb_encode(bb)]
            }
            ColorSpace::Indexed { base, hival, table } => {
                let i = (g(0).round().max(0.0) as usize).min(*hival);
                let n = base.n();
                match table.get(i * n..i * n + n) {
                    Some(raw) => {
                        let comps: Vec<f32> = raw.iter().enumerate().map(|(k, &b)| {
                            let (lo, hi) = base.default_decode(k, 8);
                            lo + b as f32 / 255.0 * (hi - lo)
                        }).collect();
                        base.to_rgb(&comps)
                    }
                    None => [0.0, 0.0, 0.0],
                }
            }
            ColorSpace::Separation { alt, tint, .. } => alt.to_rgb(&tint.eval(&[clamp01(g(0))])),
            ColorSpace::DeviceN { alt, tint, n } => {
                let input: Vec<f32> = (0..*n).map(|i| clamp01(g(i))).collect();
                alt.to_rgb(&tint.eval(&input))
            }
            ColorSpace::Pattern(under) => under.as_ref().map_or([0.0, 0.0, 0.0], |u| u.to_rgb(c)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cg::pdf::doc::tests::tiny_pdf;

    fn doc() -> Doc {
        Doc::open(tiny_pdf("")).unwrap()
    }

    #[test]
    fn device_spaces() {
        assert_eq!(ColorSpace::Gray.to_rgb(&[0.5]), [0.5, 0.5, 0.5]);
        assert_eq!(ColorSpace::Cmyk.to_rgb(&[0.0, 0.0, 0.0, 1.0]), [0.0, 0.0, 0.0]);
        assert_eq!(ColorSpace::Cmyk.to_rgb(&[1.0, 0.0, 0.0, 0.0]), [0.0, 1.0, 1.0]);
        assert_eq!(ColorSpace::Rgb.to_rgb(&[2.0, -1.0, 0.25]), [1.0, 0.0, 0.25]);
        assert_eq!(ColorSpace::Icc { n: 1 }.to_rgb(&[1.0]), [1.0, 1.0, 1.0]);
        assert_eq!(ColorSpace::Cmyk.initial(), vec![0.0, 0.0, 0.0, 1.0]);
    }

    #[test]
    fn names_and_indexed() {
        let d = doc();
        assert!(matches!(ColorSpace::parse(&d, &Obj::Name("DeviceRGB".into()), None), Some(ColorSpace::Rgb)));
        let idx = Obj::Array(vec![Obj::Name("Indexed".into()), Obj::Name("DeviceRGB".into()), Obj::Int(1), Obj::Str(vec![255, 0, 0, 0, 0, 255])]);
        let cs = ColorSpace::parse(&d, &idx, None).unwrap();
        assert_eq!(cs.to_rgb(&[1.0]), [0.0, 0.0, 1.0]);
        assert_eq!(cs.to_rgb(&[9.0]), [0.0, 0.0, 1.0], "the index clamps to hival");
        let mut res = Dict::new();
        res.insert("Cs1".into(), Obj::Name("DeviceGray".into()));
        assert!(matches!(ColorSpace::parse(&d, &Obj::Name("Cs1".into()), Some(&res)), Some(ColorSpace::Gray)));
        assert!(ColorSpace::parse(&d, &Obj::Name("Nope".into()), Some(&res)).is_none());
    }

    #[test]
    fn lab_white_and_black() {
        let cs = ColorSpace::Lab { white: [0.9642, 1.0, 0.8249], range: [-100.0, 100.0, -100.0, 100.0] };
        let w = cs.to_rgb(&[100.0, 0.0, 0.0]);
        assert!(w.iter().all(|v| *v > 0.98), "{w:?}");
        let k = cs.to_rgb(&[0.0, 0.0, 0.0]);
        assert!(k.iter().all(|v| *v < 0.02), "{k:?}");
    }
}
