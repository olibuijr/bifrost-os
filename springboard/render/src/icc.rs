//! Reads ICC colour profiles and keeps the profile each device uses.
//!
//! `Profile::parse` checks the 128-byte header, the `acsp` magic and the tag table, and rejects any tag whose data
//! would run past the declared size of the profile. `Profile::description` returns the text of the description tag in
//! its version 2 form (`desc`) or its version 4 form (`mluc`, UTF-16BE). `Profile::matrix_shaper` reads the colorants
//! (`rXYZ`, `gXYZ`, `bXYZ`) and tone curves (`rTRC`, `gTRC`, `bTRC`) of an RGB matrix-shaper profile and converts
//! colours from it into a named working space. `Profile::vcgt` reads the video card gamma table of a display profile.
//! `Devices` records the profile of each display, printer, scanner and camera, with an optional custom profile that
//! overrides the default. Profiles that use lookup tables are not converted; rendering intents and black point
//! compensation live in `rendering_intent`.

use std::collections::BTreeMap;

use super::colorspace::{self, Mat3, Space};
use super::icc_lut::{parse_mab, parse_mft, Lut};

/// The fixed part of the header, in bytes.
const HEADER_LEN: usize = 128;
/// Where the tag count sits: right after the header.
const TABLE_START: usize = 132;
/// One tag table entry: signature, offset and size, four bytes each.
const TAG_ENTRY_LEN: usize = 12;

/// Why a profile was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IccError {
    /// The data is shorter than the header and tag table it declares.
    Truncated,
    /// The `acsp` magic is missing, so the data is not an ICC profile.
    NotAProfile,
    /// A tag's data runs past the declared size of the profile.
    TagOutOfBounds(String),
}

/// The fields of the profile header that describe what the profile is for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Header {
    /// The declared size of the profile in bytes.
    pub size: u32,
    /// Major and minor version: version 2.1 is `(2, 1)`.
    pub version: (u8, u8),
    /// Four-character device class, for example `mntr` for a display or `prtr` for a printer.
    pub device_class: String,
    /// Four-character colour space of the data, for example `RGB ` or `GRAY`.
    pub color_space: String,
    /// Four-character profile connection space, `XYZ ` or `Lab `.
    pub connection_space: String,
}

/// One entry of the tag table. The offset is measured from the start of the profile.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tag {
    pub signature: String,
    pub offset: u32,
    pub size: u32,
}

/// A parsed profile header and tag table. The tag data stays in the caller's bytes; `description` takes them again.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Profile {
    pub header: Header,
    pub tags: Vec<Tag>,
}

impl Profile {
    /// Parses the header and the tag table of a profile held in `data`.
    pub fn parse(data: &[u8]) -> Result<Profile, IccError> {
        if data.len() < TABLE_START {
            return Err(IccError::Truncated);
        }
        let size_field = be_u32(data, 0).ok_or(IccError::Truncated)?;
        let size = size_field as usize;
        if size < TABLE_START || size > data.len() {
            return Err(IccError::Truncated);
        }
        if signature(data, 36) != "acsp" {
            return Err(IccError::NotAProfile);
        }
        let count = be_u32(data, HEADER_LEN).ok_or(IccError::Truncated)? as usize;
        if TABLE_START + TAG_ENTRY_LEN * count > size {
            return Err(IccError::Truncated);
        }
        let mut tags = Vec::with_capacity(count);
        for i in 0..count {
            let at = TABLE_START + TAG_ENTRY_LEN * i;
            let sig = signature(data, at);
            let offset = be_u32(data, at + 4).ok_or(IccError::Truncated)?;
            let len = be_u32(data, at + 8).ok_or(IccError::Truncated)?;
            if (offset as usize) + (len as usize) > size {
                return Err(IccError::TagOutOfBounds(sig));
            }
            tags.push(Tag { signature: sig, offset, size: len });
        }
        let header = Header {
            size: size_field,
            version: (data[8], data[9] >> 4),
            device_class: signature(data, 12),
            color_space: signature(data, 16),
            connection_space: signature(data, 20),
        };
        Ok(Profile { header, tags })
    }

    /// Whether the tag table has an entry with this signature.
    pub fn has_tag(&self, sig: &str) -> bool {
        self.tags.iter().any(|t| t.signature == sig)
    }

    /// The text of the description tag, or `None` when the profile has none or its data is malformed. `data` must be
    /// the bytes the profile was parsed from.
    pub fn description(&self, data: &[u8]) -> Option<String> {
        let tag = self.tags.iter().find(|t| t.signature == "desc")?;
        let body = tag_body(data, tag)?;
        let kind = body.get(0..4)?;
        if kind == b"desc" {
            // Version 2: count (bytes, including the terminating NUL) at +8, the ASCII text from +12.
            let count = be_u32(body, 8)? as usize;
            let text = body.get(12..12 + count)?;
            Some(String::from_utf8_lossy(text).trim_end_matches('\0').to_string())
        } else if kind == b"mluc" {
            // Version 4: record count at +8, then 12-byte records (language, country, length, offset) from +16.
            // The first record gives the text; its offset is measured from the start of the tag.
            if be_u32(body, 8)? == 0 {
                return None;
            }
            let len = be_u32(body, 20)? as usize;
            let off = be_u32(body, 24)? as usize;
            let bytes = body.get(off..off + len)?;
            let units: Vec<u16> = bytes.as_chunks::<2>().0.iter().map(|c| u16::from_be_bytes([c[0], c[1]])).collect();
            Some(char::decode_utf16(units).map(|r| r.unwrap_or('\u{FFFD}')).collect())
        } else {
            None
        }
    }
}

impl Profile {
    /// The matrix-shaper model of an RGB profile: its three colorants and three tone curves. None when the profile is
    /// not in the RGB colour space or lacks one of the six tags, or when one of them is malformed.
    pub fn matrix_shaper(&self, data: &[u8]) -> Option<MatrixShaper> {
        if self.header.color_space != "RGB " {
            return None;
        }
        let tags = &self.tags;
        let r = parse_xyz(data, find_tag(tags, "rXYZ")?)?;
        let g = parse_xyz(data, find_tag(tags, "gXYZ")?)?;
        let b = parse_xyz(data, find_tag(tags, "bXYZ")?)?;
        let curves = [
            parse_curve(data, find_tag(tags, "rTRC")?)?,
            parse_curve(data, find_tag(tags, "gTRC")?)?,
            parse_curve(data, find_tag(tags, "bTRC")?)?,
        ];
        let colorants = [[r[0], g[0], b[0]], [r[1], g[1], b[1]], [r[2], g[2], b[2]]];
        Some(MatrixShaper { colorants, curves })
    }

    /// The media white point (`wtpt`) as XYZ with luminance 1, when the profile has one.
    pub fn media_white(&self, data: &[u8]) -> Option<[f64; 3]> {
        parse_xyz(data, find_tag(&self.tags, "wtpt")?)
    }

    /// The grey tone curve (`kTRC`) of a grey profile.
    pub fn gray_curve(&self, data: &[u8]) -> Option<Curve> {
        parse_curve(data, find_tag(&self.tags, "kTRC")?)
    }

    /// A lookup-table pipeline from the tag `sig` (`A2B0` to `A2B2` or `B2A0` to `B2A2`), in any of the three tag
    /// types. `None` when the tag is missing or malformed.
    pub fn lut(&self, data: &[u8], sig: &str) -> Option<Lut> {
        let body = tag_body(data, find_tag(&self.tags, sig)?)?;
        let pcs_lab = self.header.connection_space == "Lab ";
        let b2a = sig.starts_with("B2A");
        match body.get(0..4)? {
            b"mft1" => parse_mft(body, false, b2a, pcs_lab),
            b"mft2" => parse_mft(body, true, b2a, pcs_lab),
            b"mAB " => parse_mab(body, false, pcs_lab),
            b"mBA " => parse_mab(body, true, pcs_lab),
            _ => None,
        }
    }

    /// The video card gamma table (`vcgt`) of a display profile, when it has a well-formed one.
    pub fn vcgt(&self, data: &[u8]) -> Option<Vcgt> {
        let tag = find_tag(&self.tags, "vcgt")?;
        parse_vcgt(tag_body(data, tag)?)
    }
}

fn find_tag<'a>(tags: &'a [Tag], sig: &str) -> Option<&'a Tag> {
    tags.iter().find(|t| t.signature == sig)
}

/// The bytes of a tag's data, when they lie inside `data`.
fn tag_body<'a>(data: &'a [u8], tag: &Tag) -> Option<&'a [u8]> {
    let start = tag.offset as usize;
    data.get(start..start + tag.size as usize)
}

/// Reads an XYZ tag (type `XYZ `) holding one colour.
fn parse_xyz(data: &[u8], tag: &Tag) -> Option<[f64; 3]> {
    let body = tag_body(data, tag)?;
    if body.get(0..4)? != b"XYZ " {
        return None;
    }
    Some([s15(body, 8)?, s15(body, 12)?, s15(body, 16)?])
}

/// Reads a tone curve tag: `curv` (identity, a gamma or a sampled table) or `para` (a parametric function).
fn parse_curve(data: &[u8], tag: &Tag) -> Option<Curve> {
    curve_in(tag_body(data, tag)?).map(|(c, _)| c)
}

/// Number of parameters of a parametric curve function type.
fn param_count(kind: u16) -> Option<usize> {
    match kind {
        0 => Some(1),
        1 => Some(3),
        2 => Some(4),
        3 => Some(5),
        4 => Some(7),
        _ => None,
    }
}

/// Reads one tone curve at the start of `body` and returns it with the bytes it occupies, padded to four.
fn curve_in(body: &[u8]) -> Option<(Curve, usize)> {
    let pad4 = |n: usize| (n + 3) & !3;
    match body.get(0..4)? {
        b"curv" => {
            let n = be_u32(body, 8)? as usize;
            match n {
                0 => Some((Curve::Identity, 12)),
                // A single entry is a gamma in unsigned 8.8 fixed point.
                1 => Some((Curve::Gamma(be_u16(body, 12)? as f64 / 256.0), 16)),
                n => {
                    let table = (0..n).map(|i| be_u16(body, 12 + 2 * i).map(|v| v as f64 / 65535.0)).collect::<Option<Vec<_>>>()?;
                    Some((Curve::Table(table), pad4(12 + 2 * n)))
                }
            }
        }
        b"para" => {
            let kind = be_u16(body, 8)?;
            let count = param_count(kind)?;
            let params = (0..count).map(|i| s15(body, 12 + 4 * i)).collect::<Option<Vec<_>>>()?;
            Some((Curve::Parametric { kind, params }, pad4(12 + 4 * count)))
        }
        _ => None,
    }
}

/// Reads the tone curve at byte offset `at` of `data` (a curve inside a lookup-table tag). Returns the curve and
/// the number of bytes it occupies, padded to four.
pub(crate) fn curve_at(data: &[u8], at: usize) -> Option<(Curve, usize)> {
    curve_in(data.get(at..)?)
}

/// Reads a `vcgt` tag. Type 0 holds a table of one or three channels with one-byte or two-byte entries; type 1 holds
/// a gamma, minimum and maximum for each of the three channels.
fn parse_vcgt(body: &[u8]) -> Option<Vcgt> {
    if body.get(0..4)? != b"vcgt" {
        return None;
    }
    match be_u32(body, 8)? {
        0 => {
            let channels = be_u16(body, 12)? as usize;
            let entries = be_u16(body, 14)? as usize;
            let size = be_u16(body, 16)? as usize;
            if (channels != 1 && channels != 3) || entries == 0 {
                return None;
            }
            let mut at = 18;
            let mut planes = Vec::with_capacity(channels);
            for _ in 0..channels {
                let mut plane = Vec::with_capacity(entries);
                for _ in 0..entries {
                    let v = match size {
                        1 => *body.get(at)? as f64 / 255.0,
                        2 => be_u16(body, at)? as f64 / 65535.0,
                        _ => return None,
                    };
                    plane.push(v);
                    at += size;
                }
                planes.push(plane);
            }
            let mut it = planes.into_iter();
            let r = it.next()?;
            let (g, b) = if channels == 3 { (it.next()?, it.next()?) } else { (r.clone(), r.clone()) };
            Some(Vcgt::Table([r, g, b]))
        }
        1 => {
            let mut channels = [(0.0, 0.0, 0.0); 3];
            for (i, slot) in channels.iter_mut().enumerate() {
                let base = 12 + 12 * i;
                *slot = (s15(body, base)?, s15(body, base + 4)?, s15(body, base + 8)?);
            }
            Some(Vcgt::Formula(channels))
        }
        _ => None,
    }
}

/// Evaluates a sampled curve at `x` (0..=1) by linear interpolation between its entries.
fn interpolate(table: &[f64], x: f64) -> f64 {
    match table.len() {
        0 => x,
        1 => table[0],
        n => {
            let pos = x.clamp(0.0, 1.0) * (n - 1) as f64;
            let i = (pos.floor() as usize).min(n - 2);
            let f = pos - i as f64;
            table[i] * (1.0 - f) + table[i + 1] * f
        }
    }
}

/// Evaluates an ICC parametric curve of function type `kind` (0 to 4) at `x`. Missing parameters count as zero.
fn parametric(kind: u16, p: &[f64], x: f64) -> f64 {
    let at = |i: usize| p.get(i).copied().unwrap_or(0.0);
    let (g, a, b, c, d, e, f) = (at(0), at(1), at(2), at(3), at(4), at(5), at(6));
    let power = (a * x + b).max(0.0).powf(g);
    let y = match kind {
        0 => x.powf(g),
        1 => power,
        2 => power + c,
        3 => {
            if x >= d {
                power
            } else {
                c * x
            }
        }
        4 => {
            if x >= d {
                power + e
            } else {
                c * x + f
            }
        }
        _ => x,
    };
    y.clamp(0.0, 1.0)
}

/// A tone curve of a matrix-shaper profile. Input and output are nominally 0..=1.
#[derive(Clone, Debug, PartialEq)]
pub enum Curve {
    /// The encoded value is the linear value.
    Identity,
    /// A power curve, `y = x^gamma`.
    Gamma(f64),
    /// Samples spread evenly over 0..=1, interpolated linearly between them.
    Table(Vec<f64>),
    /// An ICC parametric curve of function type `kind` with its parameters.
    Parametric { kind: u16, params: Vec<f64> },
}

impl Curve {
    /// The curve's output for an encoded input, clamped to 0..=1.
    pub fn eval(&self, x: f64) -> f64 {
        let x = x.clamp(0.0, 1.0);
        match self {
            Curve::Identity => x,
            Curve::Gamma(g) => x.powf(*g),
            Curve::Table(t) => interpolate(t, x),
            Curve::Parametric { kind, params } => parametric(*kind, params, x),
        }
    }

    /// The encoded input whose output is `y`, for a curve that rises from 0 to 1. `y` is clamped to 0..=1. Gamma and
    /// identity curves invert in closed form; the others by bisection on `eval`.
    pub fn invert(&self, y: f64) -> f64 {
        let y = y.clamp(0.0, 1.0);
        match self {
            Curve::Identity => y,
            Curve::Gamma(g) if *g > 0.0 => y.powf(1.0 / g),
            _ => {
                let (mut lo, mut hi) = (0.0f64, 1.0f64);
                for _ in 0..48 {
                    let mid = 0.5 * (lo + hi);
                    if self.eval(mid) < y {
                        lo = mid;
                    } else {
                        hi = mid;
                    }
                }
                0.5 * (lo + hi)
            }
        }
    }
}

/// The model of an RGB matrix-shaper profile: colorants relative to the D50 connection space and one tone curve per
/// channel. Converting a colour decodes it through the curves and maps the linear result through the colorants.
#[derive(Clone, Debug, PartialEq)]
pub struct MatrixShaper {
    /// Row-major; column 0 is the red colorant, column 1 green, column 2 blue, as XYZ.
    colorants: Mat3,
    curves: [Curve; 3],
}

impl MatrixShaper {
    /// Encoded profile RGB (0..=1) to XYZ relative to the D50 connection space.
    pub fn to_xyz_d50(&self, rgb: [f64; 3]) -> [f64; 3] {
        let lin = [0, 1, 2].map(|i| self.curves[i].eval(rgb[i]));
        colorspace::apply(self.colorants, lin)
    }

    /// Builds a matrix-shaper model from colorants (XYZ relative to D50, columns red, green, blue) and one curve per
    /// channel, as a profile would store them.
    pub fn new(colorants: Mat3, curves: [Curve; 3]) -> Self {
        MatrixShaper { colorants, curves }
    }

    /// XYZ relative to D50 back to encoded profile RGB: the inverse of `to_xyz_d50`. Values outside the profile's
    /// gamut come out clamped to 0..=1.
    pub fn from_xyz_d50(&self, xyz: [f64; 3]) -> [f64; 3] {
        let lin = colorspace::apply(colorspace::inverse(self.colorants), xyz);
        [0, 1, 2].map(|i| self.curves[i].invert(lin[i]))
    }

    /// Converts encoded profile RGB to the working space `to`, returned encoded. Colours outside the target gamut come
    /// out below 0 or above 1.
    pub fn convert(&self, rgb: [f64; 3], to: Space) -> [f64; 3] {
        let lin = colorspace::apply(colorspace::from_xyz_d50(to), self.to_xyz_d50(rgb));
        lin.map(|c| to.from_linear(c))
    }
}

/// A video card gamma table from a `vcgt` tag, one curve per channel (red, green, blue).
#[derive(Clone, Debug, PartialEq)]
pub enum Vcgt {
    /// Sampled ramps over 0..=1, normalised to 0..=1.
    Table([Vec<f64>; 3]),
    /// A formula per channel: `min + (max - min) * x^gamma`, as (gamma, min, max).
    Formula([(f64, f64, f64); 3]),
}

impl Vcgt {
    /// The output of one channel (0 red, 1 green, 2 blue; larger values count as blue) for an input in 0..=1.
    pub fn eval(&self, channel: usize, x: f64) -> f64 {
        let x = x.clamp(0.0, 1.0);
        let c = channel.min(2);
        match self {
            Vcgt::Table(planes) => interpolate(&planes[c], x),
            Vcgt::Formula(f) => {
                let (gamma, min, max) = f[c];
                (min + (max - min) * x.powf(gamma)).clamp(0.0, 1.0)
            }
        }
    }

    /// `n` evenly spaced 16-bit samples of one channel, the form a hardware gamma ramp takes.
    pub fn ramp(&self, channel: usize, n: usize) -> Vec<u16> {
        (0..n)
            .map(|i| {
                let x = if n > 1 { i as f64 / (n - 1) as f64 } else { 0.0 };
                (self.eval(channel, x) * 65535.0).round() as u16
            })
            .collect()
    }
}

fn be_u16(data: &[u8], at: usize) -> Option<u16> {
    let bytes: [u8; 2] = data.get(at..at + 2)?.try_into().ok()?;
    Some(u16::from_be_bytes(bytes))
}

/// A signed 15.16 fixed-point number.
fn s15(data: &[u8], at: usize) -> Option<f64> {
    Some(be_u32(data, at)? as i32 as f64 / 65536.0)
}

fn be_u32(data: &[u8], at: usize) -> Option<u32> {
    let bytes: [u8; 4] = data.get(at..at + 4)?.try_into().ok()?;
    Some(u32::from_be_bytes(bytes))
}

fn signature(data: &[u8], at: usize) -> String {
    data.get(at..at + 4).map(|b| String::from_utf8_lossy(b).into_owned()).unwrap_or_default()
}

/// The profile a display uses when none is registered for it.
pub const FALLBACK_DISPLAY_PROFILE: &str = "sRGB IEC61966-2.1";

/// The kinds of device that have profiles. The order is the order `Devices::iterate` reports them in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum DeviceClass {
    Display,
    Printer,
    Scanner,
    Camera,
}

/// Why a device change was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DeviceError {
    /// No device with that class and identifier is registered.
    NotRegistered,
    /// The profile name is empty.
    EmptyProfile,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Device {
    default: String,
    custom: Option<String>,
}

/// The registered devices and the profile each one uses.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Devices {
    by_id: BTreeMap<(DeviceClass, String), Device>,
}

impl Devices {
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a device with its default profile. Registering a device again replaces its default and keeps its
    /// custom profile.
    pub fn register(&mut self, class: DeviceClass, id: &str, default_profile: &str) -> Result<(), DeviceError> {
        if default_profile.is_empty() {
            return Err(DeviceError::EmptyProfile);
        }
        let entry = self
            .by_id
            .entry((class, id.to_string()))
            .or_insert_with(|| Device { default: String::new(), custom: None });
        entry.default = default_profile.to_string();
        Ok(())
    }

    /// Removes a device. Returns whether it was registered.
    pub fn unregister(&mut self, class: DeviceClass, id: &str) -> bool {
        self.by_id.remove(&(class, id.to_string())).is_some()
    }

    /// Sets the custom profile of a registered device, or clears it with `None` so the default applies again.
    pub fn set_custom(&mut self, class: DeviceClass, id: &str, profile: Option<&str>) -> Result<(), DeviceError> {
        if profile.is_some_and(str::is_empty) {
            return Err(DeviceError::EmptyProfile);
        }
        let device = self.by_id.get_mut(&(class, id.to_string())).ok_or(DeviceError::NotRegistered)?;
        device.custom = profile.map(str::to_string);
        Ok(())
    }

    /// The profile in use for a device: its custom profile when one is set, otherwise its default.
    pub fn profile_for(&self, class: DeviceClass, id: &str) -> Option<&str> {
        let device = self.by_id.get(&(class, id.to_string()))?;
        Some(device.custom.as_deref().unwrap_or(device.default.as_str()))
    }

    /// The profile a display uses, falling back to the sRGB profile when the display is not registered.
    pub fn display_profile(&self, id: &str) -> &str {
        self.profile_for(DeviceClass::Display, id).unwrap_or(FALLBACK_DISPLAY_PROFILE)
    }

    /// Every registered device with the profile in use, ordered by class and then by identifier.
    pub fn iterate(&self) -> Vec<(DeviceClass, &str, &str)> {
        self.by_id
            .iter()
            .map(|((class, id), d)| (*class, id.as_str(), d.custom.as_deref().unwrap_or(d.default.as_str())))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a profile with the given class, colour space and tags. The tag data follows the table, each tag padded
    /// to a four-byte boundary. The profile connection space is `XYZ `.
    fn build(class: &str, color_space: &str, tags: &[(&str, Vec<u8>)]) -> Vec<u8> {
        let table_end = TABLE_START + TAG_ENTRY_LEN * tags.len();
        let mut body = Vec::new();
        let mut offsets = Vec::new();
        for (_, bytes) in tags {
            offsets.push(table_end + body.len());
            body.extend_from_slice(bytes);
            while body.len() % 4 != 0 {
                body.push(0);
            }
        }
        let total = table_end + body.len();
        let mut data = vec![0u8; table_end];
        data[0..4].copy_from_slice(&(total as u32).to_be_bytes());
        data[8] = 2;
        data[9] = 0x10;
        data[12..16].copy_from_slice(class.as_bytes());
        data[16..20].copy_from_slice(color_space.as_bytes());
        data[20..24].copy_from_slice(b"XYZ ");
        data[36..40].copy_from_slice(b"acsp");
        data[HEADER_LEN..TABLE_START].copy_from_slice(&(tags.len() as u32).to_be_bytes());
        for (i, (sig, bytes)) in tags.iter().enumerate() {
            let at = TABLE_START + TAG_ENTRY_LEN * i;
            data[at..at + 4].copy_from_slice(sig.as_bytes());
            data[at + 4..at + 8].copy_from_slice(&(offsets[i] as u32).to_be_bytes());
            data[at + 8..at + 12].copy_from_slice(&(bytes.len() as u32).to_be_bytes());
        }
        data.extend_from_slice(&body);
        data
    }

    /// A version 2 description tag holding `text`.
    fn desc_tag(text: &str) -> Vec<u8> {
        let mut v = b"desc".to_vec();
        v.extend_from_slice(&[0; 4]);
        v.extend_from_slice(&((text.len() + 1) as u32).to_be_bytes());
        v.extend_from_slice(text.as_bytes());
        v.push(0);
        v
    }

    /// A version 4 multi-localized tag holding one en-US record with `text` in UTF-16BE.
    fn mluc_tag(text: &str) -> Vec<u8> {
        let units: Vec<u8> = text.encode_utf16().flat_map(|u| u.to_be_bytes()).collect();
        let mut v = b"mluc".to_vec();
        v.extend_from_slice(&[0; 4]);
        v.extend_from_slice(&1u32.to_be_bytes());
        v.extend_from_slice(&12u32.to_be_bytes());
        v.extend_from_slice(b"en");
        v.extend_from_slice(b"US");
        v.extend_from_slice(&(units.len() as u32).to_be_bytes());
        v.extend_from_slice(&28u32.to_be_bytes());
        v.extend_from_slice(&units);
        v
    }

    #[test]
    fn parse_reads_the_header_and_the_first_tag() {
        let data = build("mntr", "RGB ", &[("desc", desc_tag("Panel"))]);
        let profile = Profile::parse(&data).unwrap();
        assert_eq!(profile.header.device_class, "mntr");
        assert_eq!(profile.header.color_space, "RGB ");
        assert_eq!(profile.header.connection_space, "XYZ ");
        assert_eq!(profile.header.version, (2, 1));
        assert_eq!(profile.header.size as usize, data.len());
        // The table is 144 bytes long, so the one tag starts at 144 and its text is 18 bytes (4 + 4 + 4 + 5 + NUL).
        assert_eq!(profile.tags, vec![Tag { signature: "desc".into(), offset: 144, size: 18 }]);
    }

    #[test]
    fn description_reads_a_version_two_desc_tag() {
        let data = build("mntr", "RGB ", &[("desc", desc_tag("Panel"))]);
        let profile = Profile::parse(&data).unwrap();
        assert_eq!(profile.description(&data), Some("Panel".to_string()));
    }

    #[test]
    fn description_reads_a_version_four_mluc_tag_as_utf16() {
        let data = build("mntr", "RGB ", &[("desc", mluc_tag("Skjár"))]);
        let profile = Profile::parse(&data).unwrap();
        assert_eq!(profile.description(&data), Some("Skjár".to_string()));
    }

    #[test]
    fn description_is_none_without_a_desc_tag() {
        let data = build("prtr", "GRAY", &[("wtpt", vec![0; 20])]);
        let profile = Profile::parse(&data).unwrap();
        assert!(profile.has_tag("wtpt"));
        assert!(!profile.has_tag("desc"));
        assert_eq!(profile.description(&data), None);
    }

    #[test]
    fn data_without_the_acsp_magic_is_not_a_profile() {
        let mut data = build("mntr", "RGB ", &[("desc", desc_tag("Panel"))]);
        data[36..40].copy_from_slice(b"xxxx");
        assert_eq!(Profile::parse(&data), Err(IccError::NotAProfile));
    }

    #[test]
    fn a_tag_that_runs_past_the_declared_size_is_refused() {
        let mut data = build("mntr", "RGB ", &[("desc", desc_tag("Panel"))]);
        // The first table entry's size field sits at 132 + 8.
        data[140..144].copy_from_slice(&0xFFFF_FFFFu32.to_be_bytes());
        assert_eq!(Profile::parse(&data), Err(IccError::TagOutOfBounds("desc".into())));
    }

    #[test]
    fn short_data_and_an_overlong_declared_size_are_truncated() {
        let data = build("mntr", "RGB ", &[("desc", desc_tag("Panel"))]);
        assert_eq!(Profile::parse(&data[..100]), Err(IccError::Truncated));
        let mut long = data.clone();
        long[0..4].copy_from_slice(&10_000u32.to_be_bytes());
        assert_eq!(Profile::parse(&long), Err(IccError::Truncated));
    }

    #[test]
    fn a_registered_device_reports_its_default_until_a_custom_profile_is_set() {
        let mut devices = Devices::new();
        devices.register(DeviceClass::Display, "1", "Panel A").unwrap();
        assert_eq!(devices.profile_for(DeviceClass::Display, "1"), Some("Panel A"));
        devices.set_custom(DeviceClass::Display, "1", Some("Calibrated")).unwrap();
        assert_eq!(devices.profile_for(DeviceClass::Display, "1"), Some("Calibrated"));
        devices.set_custom(DeviceClass::Display, "1", None).unwrap();
        assert_eq!(devices.profile_for(DeviceClass::Display, "1"), Some("Panel A"));
    }

    #[test]
    fn re_registering_a_device_keeps_its_custom_profile() {
        let mut devices = Devices::new();
        devices.register(DeviceClass::Printer, "lp0", "Paper v1").unwrap();
        devices.set_custom(DeviceClass::Printer, "lp0", Some("Glossy")).unwrap();
        devices.register(DeviceClass::Printer, "lp0", "Paper v2").unwrap();
        assert_eq!(devices.profile_for(DeviceClass::Printer, "lp0"), Some("Glossy"));
        assert!(devices.unregister(DeviceClass::Printer, "lp0"));
        assert!(!devices.unregister(DeviceClass::Printer, "lp0"));
    }

    #[test]
    fn an_unregistered_display_uses_the_fallback_profile() {
        let devices = Devices::new();
        assert_eq!(devices.profile_for(DeviceClass::Display, "9"), None);
        assert_eq!(devices.display_profile("9"), FALLBACK_DISPLAY_PROFILE);
    }

    #[test]
    fn custom_profiles_on_unknown_devices_and_empty_names_are_refused() {
        let mut devices = Devices::new();
        assert_eq!(devices.set_custom(DeviceClass::Camera, "cam", Some("X")), Err(DeviceError::NotRegistered));
        devices.register(DeviceClass::Camera, "cam", "Sensor").unwrap();
        assert_eq!(devices.set_custom(DeviceClass::Camera, "cam", Some("")), Err(DeviceError::EmptyProfile));
        assert_eq!(devices.register(DeviceClass::Camera, "cam2", ""), Err(DeviceError::EmptyProfile));
    }

    fn close(a: [f64; 3], b: [f64; 3], tol: f64) -> bool {
        (0..3).all(|i| (a[i] - b[i]).abs() <= tol)
    }

    /// A signed 15.16 fixed-point value as four bytes.
    fn s15_bytes(v: f64) -> [u8; 4] {
        ((v * 65536.0).round() as i32).to_be_bytes()
    }

    /// An `XYZ ` tag holding one colour.
    fn xyz_tag(x: f64, y: f64, z: f64) -> Vec<u8> {
        let mut v = b"XYZ ".to_vec();
        v.extend_from_slice(&[0; 4]);
        for c in [x, y, z] {
            v.extend_from_slice(&s15_bytes(c));
        }
        v
    }

    /// A `para` tag of the given function type.
    fn para_tag(kind: u16, params: &[f64]) -> Vec<u8> {
        let mut v = b"para".to_vec();
        v.extend_from_slice(&[0; 4]);
        v.extend_from_slice(&kind.to_be_bytes());
        v.extend_from_slice(&[0; 2]);
        for p in params {
            v.extend_from_slice(&s15_bytes(*p));
        }
        v
    }

    /// The sRGB transfer curve as ICC parametric function type 3.
    fn srgb_para() -> Vec<u8> {
        para_tag(3, &[2.4, 1.0 / 1.055, 0.055 / 1.055, 1.0 / 12.92, 0.04045])
    }

    /// The sRGB profile's colorants (relative to D50) with its transfer curve on each channel.
    fn srgb_profile() -> Vec<u8> {
        build(
            "mntr",
            "RGB ",
            &[
                ("rXYZ", xyz_tag(0.4360747, 0.2225045, 0.0139322)),
                ("gXYZ", xyz_tag(0.3850649, 0.7168786, 0.0971045)),
                ("bXYZ", xyz_tag(0.1430804, 0.0606169, 0.7141733)),
                ("rTRC", srgb_para()),
                ("gTRC", srgb_para()),
                ("bTRC", srgb_para()),
            ],
        )
    }

    #[test]
    fn an_srgb_matrix_profile_converts_white_to_white() {
        let data = srgb_profile();
        let shaper = Profile::parse(&data).unwrap().matrix_shaper(&data).unwrap();
        let white = shaper.convert([1.0, 1.0, 1.0], Space::Srgb);
        assert!(close(white, [1.0, 1.0, 1.0], 2e-3), "{white:?}");
    }

    #[test]
    fn an_srgb_matrix_profile_reproduces_srgb_midgrey() {
        let data = srgb_profile();
        let shaper = Profile::parse(&data).unwrap().matrix_shaper(&data).unwrap();
        let grey = shaper.convert([0.5, 0.5, 0.5], Space::Srgb);
        assert!(close(grey, [0.5, 0.5, 0.5], 2e-3), "{grey:?}");
    }

    #[test]
    fn an_srgb_profile_to_display_p3_matches_the_colorspace_transform() {
        use crate::colorspace::Transform;
        let data = srgb_profile();
        let shaper = Profile::parse(&data).unwrap().matrix_shaper(&data).unwrap();
        let want = Transform::new(Space::Srgb, Space::DisplayP3).apply([1.0, 0.0, 0.0]);
        let got = shaper.convert([1.0, 0.0, 0.0], Space::DisplayP3);
        assert!(close(got, want, 5e-3), "got {got:?}, want {want:?}");
    }

    #[test]
    fn identity_curves_give_the_d50_white_as_xyz() {
        let data = build(
            "mntr",
            "RGB ",
            &[
                ("rXYZ", xyz_tag(0.4360747, 0.2225045, 0.0139322)),
                ("gXYZ", xyz_tag(0.3850649, 0.7168786, 0.0971045)),
                ("bXYZ", xyz_tag(0.1430804, 0.0606169, 0.7141733)),
                ("rTRC", vec![b'c', b'u', b'r', b'v', 0, 0, 0, 0, 0, 0, 0, 0]),
                ("gTRC", vec![b'c', b'u', b'r', b'v', 0, 0, 0, 0, 0, 0, 0, 0]),
                ("bTRC", vec![b'c', b'u', b'r', b'v', 0, 0, 0, 0, 0, 0, 0, 0]),
            ],
        );
        let shaper = Profile::parse(&data).unwrap().matrix_shaper(&data).unwrap();
        assert_eq!(shaper.curves, [Curve::Identity, Curve::Identity, Curve::Identity]);
        let xyz = shaper.to_xyz_d50([1.0, 1.0, 1.0]);
        assert!(close(xyz, [0.9642, 1.0, 0.8249], 2e-3), "{xyz:?}");
    }

    #[test]
    fn a_profile_without_colorants_or_in_another_space_is_not_a_matrix_shaper() {
        let only_red = build("mntr", "RGB ", &[("rXYZ", xyz_tag(0.4, 0.2, 0.01)), ("rTRC", srgb_para())]);
        assert!(Profile::parse(&only_red).unwrap().matrix_shaper(&only_red).is_none());
        let grey = build("mntr", "GRAY", &[("kTRC", srgb_para())]);
        assert!(Profile::parse(&grey).unwrap().matrix_shaper(&grey).is_none());
    }

    #[test]
    fn a_curv_tag_reads_identity_gamma_and_sampled_forms() {
        let mut gamma = b"curv".to_vec();
        gamma.extend_from_slice(&[0; 4]);
        gamma.extend_from_slice(&1u32.to_be_bytes());
        gamma.extend_from_slice(&563u16.to_be_bytes());
        let tag = Tag { signature: "rTRC".into(), offset: 0, size: gamma.len() as u32 };
        assert_eq!(parse_curve(&gamma, &tag), Some(Curve::Gamma(563.0 / 256.0)));

        let mut table = b"curv".to_vec();
        table.extend_from_slice(&[0; 4]);
        table.extend_from_slice(&3u32.to_be_bytes());
        for v in [0u16, 32768, 65535] {
            table.extend_from_slice(&v.to_be_bytes());
        }
        let tag = Tag { signature: "rTRC".into(), offset: 0, size: table.len() as u32 };
        let curve = parse_curve(&table, &tag).unwrap();
        assert!((curve.eval(0.25) - 0.25).abs() < 1e-3);
        assert!((curve.eval(0.75) - 0.75).abs() < 1e-3);
    }

    #[test]
    fn the_parametric_srgb_curve_matches_the_published_values() {
        let data = srgb_para();
        let tag = Tag { signature: "rTRC".into(), offset: 0, size: data.len() as u32 };
        let curve = parse_curve(&data, &tag).unwrap();
        assert!((curve.eval(0.5) - 0.2140).abs() < 1e-3, "{}", curve.eval(0.5));
        assert!((curve.eval(0.02) - 0.02 / 12.92).abs() < 1e-6);
    }

    #[test]
    fn a_parametric_tag_with_the_wrong_parameter_count_is_refused() {
        let data = para_tag(3, &[2.4, 1.0]);
        let tag = Tag { signature: "rTRC".into(), offset: 0, size: data.len() as u32 };
        assert_eq!(parse_curve(&data, &tag), None);
    }

    /// A `vcgt` table tag: three channels, two entries each, two-byte entries.
    fn vcgt_table(channels: u16, entries: &[u16]) -> Vec<u8> {
        let mut v = b"vcgt".to_vec();
        v.extend_from_slice(&[0; 4]);
        v.extend_from_slice(&0u32.to_be_bytes());
        v.extend_from_slice(&channels.to_be_bytes());
        v.extend_from_slice(&((entries.len() / channels as usize) as u16).to_be_bytes());
        v.extend_from_slice(&2u16.to_be_bytes());
        for e in entries {
            v.extend_from_slice(&e.to_be_bytes());
        }
        v
    }

    #[test]
    fn a_vcgt_table_gives_a_16_bit_ramp_per_channel() {
        // Red 0 to 65535, green 0 to 32768, blue 0 to 65535, each over two entries.
        let data = vcgt_table(3, &[0, 65535, 0, 32768, 0, 65535]);
        let vcgt = parse_vcgt(&data).unwrap();
        assert_eq!(vcgt.ramp(0, 3), vec![0, 32768, 65535]);
        assert_eq!(vcgt.ramp(1, 3), vec![0, 16384, 32768]);
        assert_eq!(vcgt.ramp(2, 2), vec![0, 65535]);
    }

    #[test]
    fn a_vcgt_formula_with_unit_gamma_is_a_straight_ramp() {
        let mut data = b"vcgt".to_vec();
        data.extend_from_slice(&[0; 4]);
        data.extend_from_slice(&1u32.to_be_bytes());
        for _ in 0..3 {
            data.extend_from_slice(&s15_bytes(1.0));
            data.extend_from_slice(&s15_bytes(0.0));
            data.extend_from_slice(&s15_bytes(1.0));
        }
        let vcgt = parse_vcgt(&data).unwrap();
        assert_eq!(vcgt.ramp(2, 3), vec![0, 32768, 65535]);
    }

    #[test]
    fn a_vcgt_with_a_bad_channel_count_is_refused() {
        assert!(parse_vcgt(&vcgt_table(2, &[0, 1, 2, 3])).is_none());
    }

    #[test]
    fn iterate_orders_by_class_then_identifier() {
        let mut devices = Devices::new();
        devices.register(DeviceClass::Camera, "b", "Cam").unwrap();
        devices.register(DeviceClass::Display, "z", "Zed").unwrap();
        devices.register(DeviceClass::Display, "a", "Aye").unwrap();
        let listed = devices.iterate();
        assert_eq!(
            listed,
            vec![
                (DeviceClass::Display, "a", "Aye"),
                (DeviceClass::Display, "z", "Zed"),
                (DeviceClass::Camera, "b", "Cam"),
            ]
        );
    }
}
