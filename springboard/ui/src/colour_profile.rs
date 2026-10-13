//! Colour profiles for apps: reads the matrix and tone curve tags of an RGB ICC profile and converts colour values
//! between two profiles through CIE XYZ.
//!
//! Pure model. A profile is parsed from its bytes; nothing is looked up on disk and no display is calibrated. The
//! conversion does not adapt white points: both profiles must describe the same white (for example D50 in ICC version
//! 4 files). Profiles that use lookup tables (A2B and B2A tags) are not supported and are refused as missing tags.

const HEADER_LEN: usize = 128;
/// Data colour space field of an ICC header.
const DATA_SPACE_AT: usize = 16;
/// Profile file signature field of an ICC header.
const SIGNATURE_AT: usize = 36;
/// Tag count field, directly after the header.
const TAG_COUNT_AT: usize = 128;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProfileError {
    /// The data is shorter than the header or a tag table it declares.
    TooShort,
    /// The profile signature field is not the ICC signature.
    BadSignature,
    /// The data colour space is not RGB.
    NotRgb,
    /// A required tag is absent. The payload is the tag signature.
    MissingTag([u8; 4]),
    /// A tag is present but malformed or uses an unsupported type. The payload is the tag signature.
    BadTag([u8; 4]),
}

/// A tone reproduction curve, mapping an encoded value in 0..=1 to a linear value and back.
#[derive(Debug, Clone, PartialEq)]
pub enum Curve {
    Identity,
    /// `y = x^g`.
    Gamma(f64),
    /// Values at evenly spaced inputs over 0..=1, ascending.
    Table(Vec<f64>),
    /// Parametric curve type 3: `y = c * x` below `d`, `y = (a * x + b)^g` from `d` up.
    Parametric { g: f64, a: f64, b: f64, c: f64, d: f64 },
}

impl Curve {
    /// Encoded value to linear value. Input outside 0..=1 is clamped.
    pub fn apply(&self, x: f64) -> f64 {
        let x = x.clamp(0.0, 1.0);
        match self {
            Curve::Identity => x,
            Curve::Gamma(g) => x.powf(*g),
            Curve::Table(t) => interpolate(t, x),
            Curve::Parametric { g, a, b, c, d } => {
                if x < *d {
                    c * x
                } else {
                    (a * x + b).max(0.0).powf(*g)
                }
            }
        }
    }

    /// Linear value to encoded value, the inverse of [`Curve::apply`]. Input outside 0..=1 is clamped.
    pub fn invert(&self, y: f64) -> f64 {
        let y = y.clamp(0.0, 1.0);
        match self {
            Curve::Identity => y,
            Curve::Gamma(g) => y.powf(1.0 / g),
            Curve::Table(t) => invert_table(t, y),
            Curve::Parametric { g, a, b, c, d } => {
                if y < c * d {
                    y / c
                } else {
                    ((y.powf(1.0 / g) - b) / a).clamp(0.0, 1.0)
                }
            }
        }
    }
}

/// An RGB profile: the XYZ of each primary and one tone curve per channel.
#[derive(Debug, Clone, PartialEq)]
pub struct Profile {
    /// The XYZ value of the red, green and blue primaries at full intensity, in that order.
    pub primaries: [[f64; 3]; 3],
    /// Tone curves for red, green and blue.
    pub curves: [Curve; 3],
}

impl Profile {
    /// Parses an ICC matrix/TRC RGB profile.
    pub fn parse(data: &[u8]) -> Result<Profile, ProfileError> {
        if data.len() < HEADER_LEN {
            return Err(ProfileError::TooShort);
        }
        if data[SIGNATURE_AT..SIGNATURE_AT + 4] != b"acsp"[..] {
            return Err(ProfileError::BadSignature);
        }
        if data[DATA_SPACE_AT..DATA_SPACE_AT + 4] != b"RGB "[..] {
            return Err(ProfileError::NotRgb);
        }
        let primaries = [xyz_tag(data, *b"rXYZ")?, xyz_tag(data, *b"gXYZ")?, xyz_tag(data, *b"bXYZ")?];
        let curves = [curve_tag(data, *b"rTRC")?, curve_tag(data, *b"gTRC")?, curve_tag(data, *b"bTRC")?];
        Ok(Profile { primaries, curves })
    }

    /// Encoded RGB (each channel 0..=1) to CIE XYZ.
    pub fn rgb_to_xyz(&self, rgb: [f64; 3]) -> [f64; 3] {
        let linear = [self.curves[0].apply(rgb[0]), self.curves[1].apply(rgb[1]), self.curves[2].apply(rgb[2])];
        mul3(self.matrix(), linear)
    }

    /// CIE XYZ to encoded RGB. Returns None when the primaries do not span a volume. Linear values outside the
    /// profile's gamut are clamped to 0..=1 per channel.
    pub fn xyz_to_rgb(&self, xyz: [f64; 3]) -> Option<[f64; 3]> {
        let linear = mul3(invert3(self.matrix())?, xyz);
        Some([self.curves[0].invert(linear[0]), self.curves[1].invert(linear[1]), self.curves[2].invert(linear[2])])
    }

    /// Row-major matrix from linear RGB to XYZ. Column `c` is primary `c`.
    fn matrix(&self) -> [[f64; 3]; 3] {
        let mut m = [[0.0; 3]; 3];
        for (c, primary) in self.primaries.iter().enumerate() {
            for (r, v) in primary.iter().enumerate() {
                m[r][c] = *v;
            }
        }
        m
    }
}

/// Converts an encoded RGB colour from one profile to another through XYZ.
pub fn convert(from: &Profile, to: &Profile, rgb: [f64; 3]) -> Option<[f64; 3]> {
    to.xyz_to_rgb(from.rgb_to_xyz(rgb))
}

fn interpolate(t: &[f64], x: f64) -> f64 {
    match t.len() {
        0 => x,
        1 => t[0],
        n => {
            let p = x * (n - 1) as f64;
            let i = (p.floor() as usize).min(n - 2);
            let f = p - i as f64;
            t[i] + (t[i + 1] - t[i]) * f
        }
    }
}

fn invert_table(t: &[f64], y: f64) -> f64 {
    let n = t.len();
    if n < 2 {
        return y;
    }
    for i in 0..n - 1 {
        if y <= t[i + 1] {
            let span = t[i + 1] - t[i];
            let f = if span > 0.0 { ((y - t[i]) / span).clamp(0.0, 1.0) } else { 0.0 };
            return (i as f64 + f) / (n - 1) as f64;
        }
    }
    1.0
}

fn mul3(m: [[f64; 3]; 3], v: [f64; 3]) -> [f64; 3] {
    let mut out = [0.0; 3];
    for (r, row) in m.iter().enumerate() {
        out[r] = row[0] * v[0] + row[1] * v[1] + row[2] * v[2];
    }
    out
}

fn invert3(m: [[f64; 3]; 3]) -> Option<[[f64; 3]; 3]> {
    let det = m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1]) - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
        + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0]);
    if det.abs() < 1e-12 {
        return None;
    }
    let d = 1.0 / det;
    Some([
        [
            (m[1][1] * m[2][2] - m[1][2] * m[2][1]) * d,
            (m[0][2] * m[2][1] - m[0][1] * m[2][2]) * d,
            (m[0][1] * m[1][2] - m[0][2] * m[1][1]) * d,
        ],
        [
            (m[1][2] * m[2][0] - m[1][0] * m[2][2]) * d,
            (m[0][0] * m[2][2] - m[0][2] * m[2][0]) * d,
            (m[0][2] * m[1][0] - m[0][0] * m[1][2]) * d,
        ],
        [
            (m[1][0] * m[2][1] - m[1][1] * m[2][0]) * d,
            (m[0][1] * m[2][0] - m[0][0] * m[2][1]) * d,
            (m[0][0] * m[1][1] - m[0][1] * m[1][0]) * d,
        ],
    ])
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    let s = b.get(at..at.checked_add(4)?)?;
    Some(u32::from_be_bytes([s[0], s[1], s[2], s[3]]))
}

fn u16_at(b: &[u8], at: usize) -> Option<u16> {
    let s = b.get(at..at.checked_add(2)?)?;
    Some(u16::from_be_bytes([s[0], s[1]]))
}

/// A signed 15.16 fixed-point value.
fn s15_at(b: &[u8], at: usize) -> Option<f64> {
    Some(u32_at(b, at)? as i32 as f64 / 65536.0)
}

/// The body of the tag with signature `sig`.
fn tag(data: &[u8], sig: [u8; 4]) -> Result<&[u8], ProfileError> {
    let count = u32_at(data, TAG_COUNT_AT).ok_or(ProfileError::TooShort)? as usize;
    for i in 0..count {
        let at = TAG_COUNT_AT + 4 + i * 12;
        let entry = data.get(at..at + 12).ok_or(ProfileError::TooShort)?;
        if entry[..4] == sig[..] {
            let off = u32_at(entry, 4).ok_or(ProfileError::TooShort)? as usize;
            let size = u32_at(entry, 8).ok_or(ProfileError::TooShort)? as usize;
            let end = off.checked_add(size).ok_or(ProfileError::BadTag(sig))?;
            return data.get(off..end).ok_or(ProfileError::BadTag(sig));
        }
    }
    Err(ProfileError::MissingTag(sig))
}

fn xyz_tag(data: &[u8], sig: [u8; 4]) -> Result<[f64; 3], ProfileError> {
    let t = tag(data, sig)?;
    let bad = ProfileError::BadTag(sig);
    if t.len() < 20 || t[..4] != *b"XYZ " {
        return Err(bad);
    }
    Ok([s15_at(t, 8).ok_or(bad)?, s15_at(t, 12).ok_or(bad)?, s15_at(t, 16).ok_or(bad)?])
}

fn curve_tag(data: &[u8], sig: [u8; 4]) -> Result<Curve, ProfileError> {
    let t = tag(data, sig)?;
    let bad = ProfileError::BadTag(sig);
    if t.len() < 12 {
        return Err(bad);
    }
    match &t[..4] {
        b"curv" => {
            let count = u32_at(t, 8).ok_or(bad)? as usize;
            match count {
                0 => Ok(Curve::Identity),
                1 => {
                    let g = u16_at(t, 12).ok_or(bad)? as f64 / 256.0;
                    if g > 0.0 {
                        Ok(Curve::Gamma(g))
                    } else {
                        Err(bad)
                    }
                }
                n => {
                    if t.len() < 12 + 2 * n {
                        return Err(bad);
                    }
                    let mut table = Vec::with_capacity(n);
                    for i in 0..n {
                        table.push(u16_at(t, 12 + 2 * i).ok_or(bad)? as f64 / 65535.0);
                    }
                    Ok(Curve::Table(table))
                }
            }
        }
        b"para" => {
            let kind = u16_at(t, 8).ok_or(bad)?;
            let g = s15_at(t, 12).ok_or(bad)?;
            if g <= 0.0 {
                return Err(bad);
            }
            match kind {
                0 => Ok(Curve::Gamma(g)),
                3 => {
                    if t.len() < 32 {
                        return Err(bad);
                    }
                    let a = s15_at(t, 16).ok_or(bad)?;
                    let b = s15_at(t, 20).ok_or(bad)?;
                    let c = s15_at(t, 24).ok_or(bad)?;
                    let d = s15_at(t, 28).ok_or(bad)?;
                    if a <= 0.0 || c <= 0.0 || !(0.0..=1.0).contains(&d) {
                        return Err(bad);
                    }
                    Ok(Curve::Parametric { g, a, b, c, d })
                }
                _ => Err(bad),
            }
        }
        _ => Err(bad),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The sRGB primaries as XYZ under D65.
    const SRGB_PRIMARIES: [[f64; 3]; 3] = [[0.4124, 0.2126, 0.0193], [0.3576, 0.7152, 0.1192], [0.1805, 0.0722, 0.9505]];

    fn s15(v: f64) -> [u8; 4] {
        ((v * 65536.0).round() as i32).to_be_bytes()
    }

    fn curv_gamma(v: u16) -> Vec<u8> {
        let mut b = b"curv\0\0\0\0".to_vec();
        b.extend_from_slice(&1u32.to_be_bytes());
        b.extend_from_slice(&v.to_be_bytes());
        b
    }

    fn para_srgb() -> Vec<u8> {
        let mut b = b"para\0\0\0\0".to_vec();
        b.extend_from_slice(&3u16.to_be_bytes());
        b.extend_from_slice(&[0, 0]);
        for v in [2.4, 1.0 / 1.055, 0.055 / 1.055, 1.0 / 12.92, 0.04045] {
            b.extend_from_slice(&s15(v));
        }
        b
    }

    /// A minimal RGB matrix/TRC profile with the given primaries and curves.
    fn build(primaries: [[f64; 3]; 3], curves: [Vec<u8>; 3]) -> Vec<u8> {
        let mut tags: Vec<([u8; 4], Vec<u8>)> = Vec::new();
        for (i, sig) in [b"rXYZ", b"gXYZ", b"bXYZ"].iter().enumerate() {
            let mut body = b"XYZ \0\0\0\0".to_vec();
            for v in primaries[i] {
                body.extend_from_slice(&s15(v));
            }
            tags.push((**sig, body));
        }
        for (i, sig) in [b"rTRC", b"gTRC", b"bTRC"].iter().enumerate() {
            tags.push((**sig, curves[i].clone()));
        }
        let mut out = vec![0u8; HEADER_LEN];
        out[DATA_SPACE_AT..DATA_SPACE_AT + 4].copy_from_slice(b"RGB ");
        out[SIGNATURE_AT..SIGNATURE_AT + 4].copy_from_slice(b"acsp");
        out.extend_from_slice(&(tags.len() as u32).to_be_bytes());
        let mut data_at = HEADER_LEN + 4 + tags.len() * 12;
        let mut data = Vec::new();
        for (sig, body) in &tags {
            out.extend_from_slice(sig);
            out.extend_from_slice(&(data_at as u32).to_be_bytes());
            out.extend_from_slice(&(body.len() as u32).to_be_bytes());
            data.extend_from_slice(body);
            data_at += body.len();
        }
        out.extend_from_slice(&data);
        out
    }

    fn srgb_bytes() -> Vec<u8> {
        build(SRGB_PRIMARIES, [para_srgb(), para_srgb(), para_srgb()])
    }

    fn close(a: [f64; 3], b: [f64; 3], tol: f64) -> bool {
        a.iter().zip(b.iter()).all(|(x, y)| (x - y).abs() <= tol)
    }

    #[test]
    fn short_data_and_a_bad_signature_are_refused() {
        assert_eq!(Profile::parse(&[0; 100]), Err(ProfileError::TooShort));
        let mut bytes = srgb_bytes();
        bytes[SIGNATURE_AT] = 0;
        assert_eq!(Profile::parse(&bytes), Err(ProfileError::BadSignature));
    }

    #[test]
    fn a_grey_profile_is_not_an_rgb_profile() {
        let mut bytes = srgb_bytes();
        bytes[DATA_SPACE_AT..DATA_SPACE_AT + 4].copy_from_slice(b"GRAY");
        assert_eq!(Profile::parse(&bytes), Err(ProfileError::NotRgb));
    }

    #[test]
    fn a_missing_colourant_tag_is_reported_by_signature() {
        let bytes = build(SRGB_PRIMARIES, [para_srgb(), para_srgb(), para_srgb()]);
        let mut renamed = bytes.clone();
        // Rename the green colourant entry (the second tag) so that lookup fails.
        renamed[TAG_COUNT_AT + 4 + 12..TAG_COUNT_AT + 4 + 16].copy_from_slice(b"xxxx");
        assert_eq!(Profile::parse(&renamed), Err(ProfileError::MissingTag(*b"gXYZ")));
    }

    #[test]
    fn a_zero_gamma_curve_is_refused() {
        let bytes = build(SRGB_PRIMARIES, [curv_gamma(0), curv_gamma(256), curv_gamma(256)]);
        assert_eq!(Profile::parse(&bytes), Err(ProfileError::BadTag(*b"rTRC")));
    }

    #[test]
    fn white_maps_to_the_sum_of_the_primaries() {
        let p = Profile::parse(&srgb_bytes()).unwrap();
        let xyz = p.rgb_to_xyz([1.0, 1.0, 1.0]);
        assert!(close(xyz, [0.9505, 1.0, 1.089], 0.002), "{xyz:?}");
    }

    #[test]
    fn a_colour_survives_a_round_trip_through_the_same_profile() {
        let p = Profile::parse(&srgb_bytes()).unwrap();
        let back = convert(&p, &p, [0.2, 0.5, 0.8]).unwrap();
        assert!(close(back, [0.2, 0.5, 0.8], 1e-9), "{back:?}");
    }

    #[test]
    fn a_colour_converts_between_two_profiles_with_different_curves() {
        let srgb = Profile::parse(&srgb_bytes()).unwrap();
        let gamma = Profile::parse(&build(SRGB_PRIMARIES, [curv_gamma(563), curv_gamma(563), curv_gamma(563)])).unwrap();
        // Pure red has the same XYZ in both; the encoded value differs only through the tone curve.
        let encoded = convert(&srgb, &gamma, [1.0, 0.0, 0.0]).unwrap();
        assert!(close(encoded, [1.0, 0.0, 0.0], 1e-6), "{encoded:?}");
        let mid = convert(&srgb, &gamma, [0.5, 0.5, 0.5]).unwrap();
        // 0.5 under the sRGB curve is about 0.215 linear; a gamma 2.2 profile encodes that as about 0.497.
        assert!((mid[0] - 0.497).abs() < 0.01, "{mid:?}");
    }

    #[test]
    fn a_table_curve_inverts_by_linear_search() {
        let t = Curve::Table(vec![0.0, 0.25, 1.0]);
        assert!((t.apply(0.5) - 0.25).abs() < 1e-12);
        assert!((t.invert(0.25) - 0.5).abs() < 1e-12);
        assert!((t.invert(1.0) - 1.0).abs() < 1e-12);
    }

    #[test]
    fn primaries_that_collapse_to_a_line_cannot_convert_back() {
        let flat = build([[0.0; 3]; 3], [para_srgb(), para_srgb(), para_srgb()]);
        let p = Profile::parse(&flat).unwrap();
        assert_eq!(p.xyz_to_rgb([0.5, 0.5, 0.5]), None);
    }

    #[test]
    fn out_of_gamut_values_clamp_into_range() {
        let p = Profile::parse(&srgb_bytes()).unwrap();
        let rgb = p.xyz_to_rgb([2.0, 0.0, 0.0]).unwrap();
        assert!(rgb.iter().all(|v| (0.0..=1.0).contains(v)), "{rgb:?}");
    }
}
