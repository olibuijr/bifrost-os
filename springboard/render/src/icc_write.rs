//! Creating and editing ICC profiles: a mutable profile is a 128-byte header plus a list of tags (signature and raw
//! body). It starts empty (`MutableProfile::new`), as an RGB matrix-shaper display profile
//! (`MutableProfile::rgb_display`), or from the bytes of an existing profile (`MutableProfile::from_bytes`); tags are
//! added, replaced and removed; `to_bytes` lays the profile out again with the tag table, offsets and size filled in.
//! Tags with identical bodies share one copy. The output is read back by `icc::Profile::parse`, and the bytes can be
//! embedded in a picture through the image writer's profile option.

use super::colorspace;
use super::icc::{IccError, Profile};
use super::icc_builtin::{desc_body, s15, xyz_body};

const HEADER_LEN: usize = 128;

/// Why a tag or profile could not be written.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WriteError {
    /// A tag signature must be four printable ASCII characters.
    BadSignature(String),
    /// A curve or table was empty, had too many entries or an unknown function type.
    BadCurve,
    /// The bytes given to `from_bytes` are not a profile.
    Profile(IccError),
}

/// A profile being built or edited.
#[derive(Clone, Debug, PartialEq)]
pub struct MutableProfile {
    header: [u8; HEADER_LEN],
    tags: Vec<(String, Vec<u8>)>,
}

fn valid_signature(sig: &str) -> bool {
    sig.len() == 4 && sig.bytes().all(|b| (0x20..0x7f).contains(&b))
}

fn put(out: &mut [u8; HEADER_LEN], at: usize, bytes: &[u8]) {
    out[at..at + bytes.len()].copy_from_slice(bytes);
}

/// The number of parameters of an ICC parametric curve of function type `kind`.
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

impl MutableProfile {
    /// An empty version 2.4 profile of `device_class` (`mntr` display, `prtr` printer, `scnr` input, `spac` colour
    /// space) over the data colour space `color_space` (`RGB `, `GRAY`, `CMYK`), with the XYZ connection space.
    pub fn new(device_class: [u8; 4], color_space: [u8; 4]) -> Self {
        let mut header = [0u8; HEADER_LEN];
        header[8] = 2;
        header[9] = 0x40;
        put(&mut header, 12, &device_class);
        put(&mut header, 16, &color_space);
        put(&mut header, 20, b"XYZ ");
        put(&mut header, 36, b"acsp");
        let d50 = colorspace::d50_xyz();
        for (i, v) in d50.iter().enumerate() {
            put(&mut header, 68 + 4 * i, &s15(*v));
        }
        MutableProfile { header, tags: Vec::new() }
    }

    /// An RGB matrix-shaper display profile: description, media white, the three colorants worked out from the
    /// primaries and white (CIE xy) and one gamma curve for all channels.
    pub fn rgb_display(description: &str, primaries: [(f64, f64); 3], white: (f64, f64), gamma: f64) -> Result<Self, WriteError> {
        let mut p = MutableProfile::new(*b"mntr", *b"RGB ");
        let m = colorspace::colorants_d50(primaries, white);
        p.set_description(description);
        p.set_media_white(super::icc_builtin::xy_to_xyz(white));
        p.set_colorants([[m[0][0], m[1][0], m[2][0]], [m[0][1], m[1][1], m[2][1]], [m[0][2], m[1][2], m[2][2]]]);
        p.set_gamma_curves(gamma)?;
        Ok(p)
    }

    /// Adopts the header and tags of an existing profile so they can be edited.
    pub fn from_bytes(data: &[u8]) -> Result<Self, WriteError> {
        let parsed = Profile::parse(data).map_err(WriteError::Profile)?;
        let mut header = [0u8; HEADER_LEN];
        header.copy_from_slice(&data[..HEADER_LEN]);
        let mut tags = Vec::with_capacity(parsed.tags.len());
        for t in &parsed.tags {
            let start = t.offset as usize;
            let body = data.get(start..start + t.size as usize).ok_or_else(|| WriteError::Profile(IccError::TagOutOfBounds(t.signature.clone())))?;
            tags.push((t.signature.clone(), body.to_vec()));
        }
        Ok(MutableProfile { header, tags })
    }

    // ---- header ---------------------------------------------------------------------------------------

    pub fn set_device_class(&mut self, class: [u8; 4]) {
        put(&mut self.header, 12, &class);
    }

    pub fn set_color_space(&mut self, space: [u8; 4]) {
        put(&mut self.header, 16, &space);
    }

    /// The profile connection space: `XYZ ` or `Lab `.
    pub fn set_connection_space(&mut self, pcs: [u8; 4]) {
        put(&mut self.header, 20, &pcs);
    }

    /// The version as major and minor (`(4, 3)` for 4.3).
    pub fn set_version(&mut self, major: u8, minor: u8) {
        self.header[8] = major;
        self.header[9] = (minor & 0x0f) << 4;
    }

    /// The preferred rendering intent: 0 perceptual, 1 relative colorimetric, 2 saturation, 3 absolute colorimetric.
    pub fn set_rendering_intent(&mut self, intent: u32) {
        put(&mut self.header, 64, &intent.to_be_bytes());
    }

    // ---- tags -----------------------------------------------------------------------------------------

    /// The signatures of the tags, in table order.
    pub fn tag_signatures(&self) -> Vec<&str> {
        self.tags.iter().map(|(s, _)| s.as_str()).collect()
    }

    pub fn has_tag(&self, sig: &str) -> bool {
        self.tags.iter().any(|(s, _)| s == sig)
    }

    /// The raw body of a tag.
    pub fn tag(&self, sig: &str) -> Option<&[u8]> {
        self.tags.iter().find(|(s, _)| s == sig).map(|(_, b)| b.as_slice())
    }

    /// Adds a tag or replaces the one with the same signature.
    pub fn set_tag(&mut self, sig: &str, body: Vec<u8>) -> Result<(), WriteError> {
        if !valid_signature(sig) {
            return Err(WriteError::BadSignature(sig.to_string()));
        }
        match self.tags.iter_mut().find(|(s, _)| s == sig) {
            Some(slot) => slot.1 = body,
            None => self.tags.push((sig.to_string(), body)),
        }
        Ok(())
    }

    /// Removes a tag; false when there was none.
    pub fn remove_tag(&mut self, sig: &str) -> bool {
        let before = self.tags.len();
        self.tags.retain(|(s, _)| s != sig);
        self.tags.len() != before
    }

    pub fn set_description(&mut self, text: &str) {
        self.tags.retain(|(s, _)| s != "desc");
        self.tags.insert(0, ("desc".to_string(), desc_body(text)));
    }

    /// The copyright text (`cprt`, a `text` tag).
    pub fn set_copyright(&mut self, text: &str) {
        let mut b = b"text".to_vec();
        b.extend([0u8; 4]);
        b.extend(text.as_bytes());
        b.push(0);
        let _ = self.set_tag("cprt", b);
    }

    /// The media white point (`wtpt`), XYZ relative to D50 with luminance 1.
    pub fn set_media_white(&mut self, xyz: [f64; 3]) {
        let _ = self.set_tag("wtpt", xyz_body(xyz));
    }

    /// The colorants (`rXYZ`, `gXYZ`, `bXYZ`): `columns` holds the red, green and blue colorant as XYZ.
    pub fn set_colorants(&mut self, columns: [[f64; 3]; 3]) {
        for (sig, v) in ["rXYZ", "gXYZ", "bXYZ"].into_iter().zip(columns) {
            let _ = self.set_tag(sig, xyz_body(v));
        }
    }

    /// One gamma curve (`y = x^gamma`) for the three channels of an RGB profile. The gamma is stored as unsigned
    /// 8.8 fixed point, so it must lie in 0 to 255.
    pub fn set_gamma_curves(&mut self, gamma: f64) -> Result<(), WriteError> {
        let body = gamma_curve(gamma)?;
        for sig in ["rTRC", "gTRC", "bTRC"] {
            self.set_tag(sig, body.clone())?;
        }
        Ok(())
    }

    /// A sampled tone curve for `sig` (`rTRC`, `gTRC`, `bTRC` or `kTRC`), entries spread evenly over 0..=1.
    pub fn set_curve_table(&mut self, sig: &str, entries: &[u16]) -> Result<(), WriteError> {
        if entries.is_empty() || entries.len() > u32::MAX as usize / 4 {
            return Err(WriteError::BadCurve);
        }
        let mut b = b"curv".to_vec();
        b.extend([0u8; 4]);
        b.extend((entries.len() as u32).to_be_bytes());
        for e in entries {
            b.extend(e.to_be_bytes());
        }
        self.set_tag(sig, b)
    }

    /// A parametric tone curve (function type 0 to 4 of the ICC specification) for `sig`.
    pub fn set_parametric_curve(&mut self, sig: &str, kind: u16, params: &[f64]) -> Result<(), WriteError> {
        if param_count(kind) != Some(params.len()) {
            return Err(WriteError::BadCurve);
        }
        let mut b = b"para".to_vec();
        b.extend([0u8; 4]);
        b.extend(kind.to_be_bytes());
        b.extend([0u8; 2]);
        for p in params {
            b.extend(s15(*p));
        }
        self.set_tag(sig, b)
    }

    /// A video card gamma table (`vcgt`) of three 16-bit ramps of the same length.
    pub fn set_gamma_table(&mut self, ramps: [&[u16]; 3]) -> Result<(), WriteError> {
        let n = ramps[0].len();
        if n == 0 || n > u16::MAX as usize || ramps.iter().any(|r| r.len() != n) {
            return Err(WriteError::BadCurve);
        }
        let mut b = b"vcgt".to_vec();
        b.extend([0u8; 4]);
        b.extend(0u32.to_be_bytes());
        b.extend(3u16.to_be_bytes());
        b.extend((n as u16).to_be_bytes());
        b.extend(2u16.to_be_bytes());
        for r in ramps {
            for v in r {
                b.extend(v.to_be_bytes());
            }
        }
        self.set_tag("vcgt", b)
    }

    // ---- output ---------------------------------------------------------------------------------------

    /// The profile as ICC bytes: header, tag table and tag data (each body padded to four bytes, identical bodies
    /// stored once), with the size field filled in and the profile ID cleared.
    pub fn to_bytes(&self) -> Vec<u8> {
        let table_len = 4 + 12 * self.tags.len();
        let start = HEADER_LEN + table_len;
        let mut unique: Vec<&Vec<u8>> = Vec::new();
        let mut which: Vec<usize> = Vec::new();
        for (_, body) in &self.tags {
            let i = match unique.iter().position(|b| *b == body) {
                Some(i) => i,
                None => {
                    unique.push(body);
                    unique.len() - 1
                }
            };
            which.push(i);
        }
        let mut offsets = Vec::with_capacity(unique.len());
        let mut data = Vec::new();
        for body in &unique {
            offsets.push(start + data.len());
            data.extend_from_slice(body);
            while data.len() % 4 != 0 {
                data.push(0);
            }
        }
        let total = start + data.len();
        let mut out = self.header.to_vec();
        out[0..4].copy_from_slice(&(total as u32).to_be_bytes());
        out[84..100].fill(0);
        out.extend((self.tags.len() as u32).to_be_bytes());
        for ((sig, body), idx) in self.tags.iter().zip(&which) {
            out.extend(sig.as_bytes());
            out.extend((offsets[*idx] as u32).to_be_bytes());
            out.extend((body.len() as u32).to_be_bytes());
        }
        out.extend(data);
        out
    }
}

/// A `curv` tag holding one gamma in unsigned 8.8 fixed point.
fn gamma_curve(gamma: f64) -> Result<Vec<u8>, WriteError> {
    if !gamma.is_finite() || !(0.0..256.0).contains(&gamma) {
        return Err(WriteError::BadCurve);
    }
    let mut b = b"curv".to_vec();
    b.extend([0u8; 4]);
    b.extend(1u32.to_be_bytes());
    b.extend(((gamma * 256.0).round() as u16).to_be_bytes());
    b.extend([0u8; 2]);
    Ok(b)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::colorspace::Space;
    use crate::icc::Curve;
    use crate::icc_builtin::Builtin;

    fn close(a: f64, b: f64, tol: f64) -> bool {
        (a - b).abs() <= tol
    }

    fn srgb_primaries() -> ([(f64, f64); 3], (f64, f64)) {
        Space::Srgb.chromaticities()
    }

    #[test]
    fn a_built_rgb_display_profile_parses_back_with_its_tags() {
        let (prim, white) = srgb_primaries();
        let mut p = MutableProfile::rgb_display("Test Display", prim, white, 2.2).unwrap();
        p.set_copyright("Public domain");
        let data = p.to_bytes();
        let parsed = Profile::parse(&data).expect("the writer's output parses");
        assert_eq!(parsed.header.device_class, "mntr");
        assert_eq!(parsed.header.color_space, "RGB ");
        assert_eq!(parsed.header.connection_space, "XYZ ");
        assert_eq!(parsed.header.version, (2, 4));
        assert_eq!(parsed.header.size as usize, data.len());
        assert_eq!(parsed.description(&data).as_deref(), Some("Test Display"));
        for sig in ["desc", "cprt", "wtpt", "rXYZ", "gXYZ", "bXYZ", "rTRC", "gTRC", "bTRC"] {
            assert!(parsed.has_tag(sig), "{sig}");
        }
        let shaper = parsed.matrix_shaper(&data).expect("colorants and curves are readable");
        let white_out = shaper.to_xyz_d50([1.0, 1.0, 1.0]);
        assert!(close(white_out[1], 1.0, 3e-3), "{white_out:?}");
        let red = shaper.to_xyz_d50([1.0, 0.0, 0.0]);
        assert!(close(red[0], 0.4361, 3e-3) && close(red[1], 0.2225, 3e-3), "{red:?}");
    }

    #[test]
    fn the_gamma_curve_is_stored_in_8_8_fixed_point() {
        let (prim, white) = srgb_primaries();
        let data = MutableProfile::rgb_display("g", prim, white, 1.8).unwrap().to_bytes();
        let parsed = Profile::parse(&data).unwrap();
        let shaper = parsed.matrix_shaper(&data).unwrap();
        // 1.8 * 256 = 460.8, stored as 461
        let half = shaper.to_xyz_d50([0.5, 0.5, 0.5])[1];
        let expect = 0.5f64.powf(461.0 / 256.0);
        assert!(close(half, expect, 5e-3), "{half} {expect}");
        assert_eq!(MutableProfile::rgb_display("g", prim, white, -1.0), Err(WriteError::BadCurve));
        assert_eq!(MutableProfile::rgb_display("g", prim, white, 300.0), Err(WriteError::BadCurve));
    }

    #[test]
    fn editing_an_existing_profile_keeps_everything_else() {
        let original = Builtin::DisplayP3.bytes();
        let mut p = MutableProfile::from_bytes(&original).unwrap();
        assert!(p.has_tag("rXYZ") && p.has_tag("rTRC"));
        p.set_description("Studio P3");
        p.set_copyright("Mine");
        assert!(p.remove_tag("wtpt"));
        assert!(!p.remove_tag("wtpt"));
        let data = p.to_bytes();
        let parsed = Profile::parse(&data).unwrap();
        assert_eq!(parsed.description(&data).as_deref(), Some("Studio P3"));
        assert!(parsed.has_tag("cprt") && !parsed.has_tag("wtpt"));
        let before = Profile::parse(&original).unwrap().matrix_shaper(&original).unwrap();
        let after = parsed.matrix_shaper(&data).unwrap();
        let a = before.to_xyz_d50([0.3, 0.6, 0.9]);
        let b = after.to_xyz_d50([0.3, 0.6, 0.9]);
        assert!((0..3).all(|i| close(a[i], b[i], 1e-9)), "{a:?} {b:?}");
    }

    #[test]
    fn identical_tag_bodies_are_stored_once() {
        let (prim, white) = srgb_primaries();
        let data = MutableProfile::rgb_display("shared", prim, white, 2.2).unwrap().to_bytes();
        let parsed = Profile::parse(&data).unwrap();
        let offset = |sig: &str| parsed.tags.iter().find(|t| t.signature == sig).unwrap().offset;
        assert_eq!(offset("rTRC"), offset("gTRC"));
        assert_eq!(offset("gTRC"), offset("bTRC"));
    }

    #[test]
    fn a_grey_profile_with_a_sampled_curve_round_trips() {
        let mut p = MutableProfile::new(*b"mntr", *b"GRAY");
        p.set_description("Grey table");
        p.set_media_white([0.9642, 1.0, 0.8249]);
        let table: Vec<u16> = (0..256).map(|i| ((i as f64 / 255.0).powf(2.0) * 65535.0).round() as u16).collect();
        p.set_curve_table("kTRC", &table).unwrap();
        let data = p.to_bytes();
        let parsed = Profile::parse(&data).unwrap();
        assert_eq!(parsed.header.color_space, "GRAY");
        let curve = parsed.gray_curve(&data).unwrap();
        assert!(close(curve.eval(0.5), 0.25, 2e-3), "{}", curve.eval(0.5));
        assert!(matches!(curve, Curve::Table(_)));
        assert_eq!(p.set_curve_table("kTRC", &[]), Err(WriteError::BadCurve));
    }

    #[test]
    fn a_parametric_curve_round_trips_and_wrong_parameter_counts_are_refused() {
        let mut p = MutableProfile::new(*b"mntr", *b"GRAY");
        // type 3 (the sRGB shape): gamma, a, b, c, d
        p.set_parametric_curve("kTRC", 3, &[2.4, 1.0 / 1.055, 0.055 / 1.055, 1.0 / 12.92, 0.04045]).unwrap();
        let data = p.to_bytes();
        let curve = Profile::parse(&data).unwrap().gray_curve(&data).unwrap();
        let srgb = |v: f64| if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) };
        assert!(close(curve.eval(0.5), srgb(0.5), 2e-3), "{}", curve.eval(0.5));
        assert!(close(curve.eval(0.02), srgb(0.02), 2e-3));
        assert_eq!(p.set_parametric_curve("kTRC", 3, &[2.4]), Err(WriteError::BadCurve));
        assert_eq!(p.set_parametric_curve("kTRC", 9, &[1.0]), Err(WriteError::BadCurve));
    }

    #[test]
    fn a_gamma_table_is_read_back_as_a_vcgt() {
        let mut p = MutableProfile::new(*b"mntr", *b"RGB ");
        let ramp: Vec<u16> = (0..16).map(|i| (i * 4369) as u16).collect();
        let dim: Vec<u16> = ramp.iter().map(|v| v / 2).collect();
        p.set_gamma_table([&ramp, &ramp, &dim]).unwrap();
        let data = p.to_bytes();
        let parsed = Profile::parse(&data).unwrap();
        let vcgt = parsed.vcgt(&data).expect("table parses");
        assert!(close(vcgt.eval(0, 1.0), 1.0, 1e-4));
        assert!(close(vcgt.eval(2, 1.0), 0.5, 1e-3), "blue is halved: {}", vcgt.eval(2, 1.0));
        assert_eq!(vcgt.ramp(0, 16), ramp);
        let short = [1u16, 2];
        assert_eq!(p.set_gamma_table([&ramp, &ramp, &short]), Err(WriteError::BadCurve));
    }

    #[test]
    fn header_setters_and_signature_checks() {
        let mut p = MutableProfile::new(*b"mntr", *b"RGB ");
        p.set_device_class(*b"prtr");
        p.set_color_space(*b"CMYK");
        p.set_connection_space(*b"Lab ");
        p.set_version(4, 3);
        p.set_rendering_intent(1);
        let data = p.to_bytes();
        let h = Profile::parse(&data).unwrap().header;
        assert_eq!((h.device_class.as_str(), h.color_space.as_str(), h.connection_space.as_str()), ("prtr", "CMYK", "Lab "));
        assert_eq!(h.version, (4, 3));
        assert_eq!(u32::from_be_bytes([data[64], data[65], data[66], data[67]]), 1);
        assert_eq!(p.set_tag("ab", vec![1]), Err(WriteError::BadSignature("ab".to_string())));
        assert!(p.set_tag("tech", b"sig \0\0\0\0dsub".to_vec()).is_ok());
        assert_eq!(p.tag_signatures(), vec!["tech"]);
        assert_eq!(p.tag("tech").map(|b| b.len()), Some(12));
    }

    #[test]
    fn bytes_that_are_not_a_profile_are_refused() {
        assert!(matches!(MutableProfile::from_bytes(&[0u8; 200]), Err(WriteError::Profile(_))));
        assert!(matches!(MutableProfile::from_bytes(&[0u8; 10]), Err(WriteError::Profile(IccError::Truncated))));
    }
}
