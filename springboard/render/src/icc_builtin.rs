//! Built-in colour profiles. None of them is a file: each is generated as ICC bytes at run time, from the chromaticities
//! and transfer functions in `colorspace`, and then read back by `icc::Profile::parse` like any profile on disk. The
//! writer here produces matrix-shaper RGB profiles (rXYZ, gXYZ, bXYZ, one shared tone-curve table) and grey profiles
//! (kTRC), the same layout the reader expects. The tone curves are sampled into `CURVE_SAMPLES` entries.

use super::colorspace::{self, Space};

/// The built-in profiles.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Builtin {
    /// sRGB (IEC 61966-2.1).
    Srgb,
    /// Display P3 with the sRGB transfer curve.
    DisplayP3,
    /// Adobe RGB (1998), gamma 563/256.
    AdobeRgb1998,
    /// ITU-R BT.2020 primaries with the BT.2020 10-bit transfer curve.
    Rec2020,
    /// ROMM RGB (ProPhoto): D50 white, gamma 1.8 with a linear toe.
    RommRgb,
    /// A grey profile with gamma 2.2.
    Gray22,
}

/// Samples in each tone-curve table.
pub const CURVE_SAMPLES: usize = 1024;

/// BT.2020 primaries and D65 white (CIE xy).
const REC2020_PRIMARIES: [(f64, f64); 3] = [(0.708, 0.292), (0.170, 0.797), (0.131, 0.046)];
const D65_XY: (f64, f64) = (0.3127, 0.3290);

impl Builtin {
    /// Every built-in profile, in the order a colour settings page lists them.
    pub const ALL: [Builtin; 6] =
        [Builtin::Srgb, Builtin::DisplayP3, Builtin::AdobeRgb1998, Builtin::Rec2020, Builtin::RommRgb, Builtin::Gray22];

    /// The description tag text, as the profile names itself.
    pub fn description(self) -> &'static str {
        match self {
            Builtin::Srgb => "sRGB IEC61966-2.1",
            Builtin::DisplayP3 => "Display P3",
            Builtin::AdobeRgb1998 => "Adobe RGB (1998)",
            Builtin::Rec2020 => "ITU-R BT.2020",
            Builtin::RommRgb => "ROMM RGB",
            Builtin::Gray22 => "Generic Gray Gamma 2.2",
        }
    }

    /// The profile as ICC bytes.
    pub fn bytes(self) -> Vec<u8> {
        let desc = self.description();
        match self {
            Builtin::Srgb => space_profile(desc, Space::Srgb),
            Builtin::DisplayP3 => space_profile(desc, Space::DisplayP3),
            Builtin::AdobeRgb1998 => space_profile(desc, Space::AdobeRgb),
            Builtin::Rec2020 => rgb_profile(desc, REC2020_PRIMARIES, D65_XY, rec2020_to_linear),
            Builtin::RommRgb => space_profile(desc, Space::RommRgb),
            Builtin::Gray22 => gray_profile(desc, |v| v.powf(2.2)),
        }
    }
}

/// BT.2020 transfer curve (10-bit form): encoded to linear.
fn rec2020_to_linear(v: f64) -> f64 {
    const ALPHA: f64 = 1.09929682680944;
    const BETA: f64 = 0.018053968510807;
    if v < 4.5 * BETA {
        v / 4.5
    } else {
        ((v + ALPHA - 1.0) / ALPHA).powf(1.0 / 0.45)
    }
}

/// The profile of one of the `colorspace` working spaces.
fn space_profile(desc: &str, space: Space) -> Vec<u8> {
    let (prim, white) = space.chromaticities();
    rgb_profile(desc, prim, white, |v| space.to_linear(v))
}

/// An RGB matrix-shaper profile with the given primaries and white (CIE xy) and a tone curve `decode`, which maps an
/// encoded channel (0..=1) to linear light.
pub fn rgb_profile(desc: &str, prim: [(f64, f64); 3], white: (f64, f64), decode: impl Fn(f64) -> f64) -> Vec<u8> {
    let m = colorspace::colorants_d50(prim, white);
    let column = |j: usize| [m[0][j], m[1][j], m[2][j]];
    let bodies = vec![
        desc_body(desc),
        xyz_body(xy_to_xyz(white)),
        xyz_body(column(0)),
        xyz_body(column(1)),
        xyz_body(column(2)),
        curve_body(decode),
    ];
    let entries = vec![("desc", 0), ("wtpt", 1), ("rXYZ", 2), ("gXYZ", 3), ("bXYZ", 4), ("rTRC", 5), ("gTRC", 5), ("bTRC", 5)];
    assemble(*b"RGB ", bodies, entries)
}

/// A grey profile whose tone curve is `decode` (encoded grey to linear luminance).
pub fn gray_profile(desc: &str, decode: impl Fn(f64) -> f64) -> Vec<u8> {
    let bodies = vec![desc_body(desc), xyz_body(xy_to_xyz(D65_XY)), curve_body(decode)];
    let entries = vec![("desc", 0), ("wtpt", 1), ("kTRC", 2)];
    assemble(*b"GRAY", bodies, entries)
}

/// XYZ of a chromaticity with luminance 1.
pub(crate) fn xy_to_xyz((x, y): (f64, f64)) -> [f64; 3] {
    [x / y, 1.0, (1.0 - x - y) / y]
}

/// A signed 15.16 fixed-point value, big-endian.
pub(crate) fn s15(v: f64) -> [u8; 4] {
    ((v * 65536.0).round() as i32).to_be_bytes()
}

pub(crate) fn xyz_body(v: [f64; 3]) -> Vec<u8> {
    let mut b = b"XYZ ".to_vec();
    b.extend([0u8; 4]);
    for x in v {
        b.extend(s15(x));
    }
    b
}

/// A version 2 `desc` tag: the ASCII text, then empty Unicode and ScriptCode fields.
pub(crate) fn desc_body(text: &str) -> Vec<u8> {
    let mut b = b"desc".to_vec();
    b.extend([0u8; 4]);
    b.extend(((text.len() + 1) as u32).to_be_bytes());
    b.extend(text.as_bytes());
    b.push(0);
    b.extend([0u8; 8 + 2 + 1 + 67]);
    b
}

/// A `curv` tag with `CURVE_SAMPLES` entries of the decoded curve.
fn curve_body(decode: impl Fn(f64) -> f64) -> Vec<u8> {
    let mut b = b"curv".to_vec();
    b.extend([0u8; 4]);
    b.extend((CURVE_SAMPLES as u32).to_be_bytes());
    for i in 0..CURVE_SAMPLES {
        let x = i as f64 / (CURVE_SAMPLES - 1) as f64;
        let y = decode(x).clamp(0.0, 1.0);
        b.extend(((y * 65535.0).round() as u16).to_be_bytes());
    }
    b
}

/// Lays out a 128-byte header, the tag table and the tag bodies (each padded to four bytes). `entries` maps a tag
/// signature to an index into `bodies`, so tags may share one body.
fn assemble(space: [u8; 4], bodies: Vec<Vec<u8>>, entries: Vec<(&str, usize)>) -> Vec<u8> {
    const HEADER: usize = 128;
    let start = HEADER + 4 + 12 * entries.len();
    let mut offsets = Vec::with_capacity(bodies.len());
    let mut data = Vec::new();
    for body in &bodies {
        offsets.push(start + data.len());
        data.extend_from_slice(body);
        while data.len() % 4 != 0 {
            data.push(0);
        }
    }
    let total = start + data.len();
    let mut out = vec![0u8; HEADER];
    out[0..4].copy_from_slice(&(total as u32).to_be_bytes());
    out[8] = 2;
    out[9] = 0x10;
    out[12..16].copy_from_slice(b"mntr");
    out[16..20].copy_from_slice(&space);
    out[20..24].copy_from_slice(b"XYZ ");
    out[36..40].copy_from_slice(b"acsp");
    let d50 = colorspace::d50_xyz();
    for (i, v) in d50.iter().enumerate() {
        out[68 + 4 * i..72 + 4 * i].copy_from_slice(&s15(*v));
    }
    out.extend((entries.len() as u32).to_be_bytes());
    for (sig, idx) in entries {
        out.extend(sig.as_bytes());
        out.extend((offsets[idx] as u32).to_be_bytes());
        out.extend((bodies[idx].len() as u32).to_be_bytes());
    }
    out.extend(data);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::icc::Profile;

    fn close(a: f64, b: f64, tol: f64) -> bool {
        (a - b).abs() <= tol
    }

    #[test]
    fn a_hand_built_profile_parses_its_header_and_tag_table() {
        // 128-byte header, a count of one, one `wtpt` entry at offset 144 holding an XYZ of the D50 white.
        let mut b = vec![0u8; 144];
        b[0..4].copy_from_slice(&(144u32 + 20).to_be_bytes());
        b[8] = 4;
        b[9] = 0x30;
        b[12..16].copy_from_slice(b"mntr");
        b[16..20].copy_from_slice(b"RGB ");
        b[20..24].copy_from_slice(b"XYZ ");
        b[36..40].copy_from_slice(b"acsp");
        b[128..132].copy_from_slice(&1u32.to_be_bytes());
        b[132..136].copy_from_slice(b"wtpt");
        b[136..140].copy_from_slice(&144u32.to_be_bytes());
        b[140..144].copy_from_slice(&20u32.to_be_bytes());
        b.extend(b"XYZ ");
        b.extend([0u8; 4]);
        b.extend(s15(0.9642));
        b.extend(s15(1.0));
        b.extend(s15(0.8249));
        let p = Profile::parse(&b).expect("hand-built profile parses");
        assert_eq!(p.header.version, (4, 3));
        assert_eq!(p.header.device_class, "mntr");
        assert_eq!(p.header.color_space, "RGB ");
        assert_eq!(p.header.connection_space, "XYZ ");
        assert!(p.has_tag("wtpt"));
        let w = p.media_white(&b).expect("wtpt reads");
        assert!(close(w[0], 0.9642, 1e-4) && close(w[1], 1.0, 1e-4) && close(w[2], 0.8249, 1e-4));
    }

    #[test]
    fn a_tag_running_past_the_end_is_refused() {
        // Declared size equals the data, but the one tag runs past its end.
        let mut b = vec![0u8; 144];
        b[0..4].copy_from_slice(&144u32.to_be_bytes());
        b[36..40].copy_from_slice(b"acsp");
        b[128..132].copy_from_slice(&1u32.to_be_bytes());
        b[132..136].copy_from_slice(b"wtpt");
        b[136..140].copy_from_slice(&144u32.to_be_bytes());
        b[140..144].copy_from_slice(&20u32.to_be_bytes());
        assert!(matches!(Profile::parse(&b), Err(crate::icc::IccError::TagOutOfBounds(_))));
    }

    #[test]
    fn srgb_profile_describes_itself_and_maps_white_to_white() {
        let data = Builtin::Srgb.bytes();
        let p = Profile::parse(&data).expect("sRGB profile parses");
        assert_eq!(p.header.color_space, "RGB ");
        assert_eq!(p.description(&data).as_deref(), Some("sRGB IEC61966-2.1"));
        let m = p.matrix_shaper(&data).expect("matrix shaper");
        let white = m.convert([1.0, 1.0, 1.0], Space::Srgb);
        for c in white {
            assert!(close(c, 1.0, 2e-3), "{white:?}");
        }
        // The red colorant column matches the D50 sRGB value that every sRGB profile stores.
        let xyz = m.to_xyz_d50([1.0, 0.0, 0.0]);
        assert!(close(xyz[0], 0.4361, 2e-3) && close(xyz[1], 0.2225, 2e-3) && close(xyz[2], 0.0139, 2e-3), "{xyz:?}");
    }

    #[test]
    fn every_builtin_profile_parses_with_the_expected_kind() {
        for b in Builtin::ALL {
            let data = b.bytes();
            let p = Profile::parse(&data).unwrap_or_else(|e| panic!("{b:?}: {e:?}"));
            assert_eq!(p.description(&data).as_deref(), Some(b.description()));
            match b {
                Builtin::Gray22 => {
                    assert_eq!(p.header.color_space, "GRAY");
                    assert!(p.gray_curve(&data).is_some());
                }
                _ => {
                    assert_eq!(p.header.color_space, "RGB ");
                    assert!(p.matrix_shaper(&data).is_some(), "{b:?}");
                }
            }
        }
    }

    #[test]
    fn romm_profile_has_d50_white_and_the_romm_red_colorant() {
        let data = Builtin::RommRgb.bytes();
        let p = Profile::parse(&data).unwrap();
        assert_eq!(p.header.color_space, "RGB ");
        assert!(p.matrix_shaper(&data).is_some());
        // The D50 white is stored as the media white point, so the ROMM white maps to unit Y.
        let m = p.matrix_shaper(&data).unwrap();
        let white = m.to_xyz_d50([1.0, 1.0, 1.0]);
        assert!(close(white[1], 1.0, 2e-3), "{white:?}");
        assert_eq!(Builtin::RommRgb.description(), "ROMM RGB");
    }

    #[test]
    fn gray_profile_gives_a_gamma_2_2_curve() {
        let data = Builtin::Gray22.bytes();
        let p = Profile::parse(&data).unwrap();
        let curve = p.gray_curve(&data).unwrap();
        assert!(close(curve.eval(0.5), 0.5f64.powf(2.2), 1e-3));
    }
}
