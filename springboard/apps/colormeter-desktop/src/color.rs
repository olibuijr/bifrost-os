//! Pure colour conversions for the Color Meter. A pixel read from the display is 8-bit sRGB; this module converts it to
//! Display P3, Adobe RGB and Generic RGB, averages the pixels of an aperture, and formats the result as hex, decimal,
//! 16-bit and percent. Nothing here reads the screen or the clipboard. The sRGB transfer function and the sRGB-to-Display-P3
//! and sRGB-to-Adobe-RGB matrices are public standards. Generic RGB is modelled as a gamma of 1.8 on the sRGB primaries,
//! which is an approximation until its own primaries are wired in. `DisplayProfile` is the display's own ICC profile:
//! Native shows the codes the display is sent as they come out in that profile.

use bifrost_app::colorsync::{Builtin, Profile, ProfileToProfile, RenderingIntent};
use std::path::{Path, PathBuf};

/// One pixel as 8-bit codes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rgb8 {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Rgb8 {
    pub const fn new(r: u8, g: u8, b: u8) -> Rgb8 {
        Rgb8 { r, g, b }
    }
}

/// The colour space a reading is shown in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Space {
    /// The display's own values, as the compositor delivers them (the display is driven as sRGB), shown unconverted.
    Native,
    Srgb,
    DisplayP3,
    AdobeRgb,
    GenericRgb,
}

impl Space {
    /// Every space, in the order the toolbar cycles through and the View menu lists them.
    pub const ALL: [Space; 5] = [Space::Native, Space::Srgb, Space::DisplayP3, Space::AdobeRgb, Space::GenericRgb];

    pub fn next(self) -> Space {
        let i = Space::ALL.iter().position(|s| *s == self).unwrap_or(0);
        Space::ALL[(i + 1) % Space::ALL.len()]
    }

    pub fn name(self) -> &'static str {
        match self {
            Space::Native => "Native",
            Space::Srgb => "sRGB",
            Space::DisplayP3 => "Display P3",
            Space::AdobeRgb => "Adobe RGB",
            Space::GenericRgb => "Generic RGB",
        }
    }
}

/// The gamma of Generic RGB.
pub const GENERIC_GAMMA: f32 = 1.8;

/// Linear sRGB to linear Display P3 (D65), row by row. Public standard matrix.
const SRGB_TO_P3: [[f32; 3]; 3] = [
    [0.8224621, 0.1775380, 0.0000000],
    [0.0331941, 0.9668058, 0.0000000],
    [0.0170827, 0.0723974, 0.9105199],
];

/// Linear sRGB to linear Adobe RGB (1998) (D65), row by row. Public standard matrix.
const SRGB_TO_ADOBE: [[f32; 3]; 3] = [
    [0.7151627, 0.2848373, 0.0000000],
    [0.0000000, 1.0000000, 0.0000000],
    [0.0000000, 0.0411619, 0.9588381],
];

/// The gamma of Adobe RGB (1998), 563/256.
pub const ADOBE_GAMMA: f32 = 2.199_218_8;

/// A pixel in one space: the 8-bit codes and the normalised 0 to 1 values they stand for.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Reading {
    pub codes: Rgb8,
    pub values: [f32; 3],
}

/// The sRGB electro-optical transfer function: encoded 0 to 1 to linear light.
pub fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// The inverse of `srgb_to_linear`.
pub fn linear_to_srgb(l: f32) -> f32 {
    if l <= 0.0031308 {
        l * 12.92
    } else {
        1.055 * l.powf(1.0 / 2.4) - 0.055
    }
}

fn quantize(v: f32) -> u8 {
    (v * 255.0).round().clamp(0.0, 255.0) as u8
}

/// Converts a pixel read as sRGB into `space`.
pub fn convert(px: Rgb8, space: Space) -> Reading {
    let srgb = [px.r, px.g, px.b].map(|c| c as f32 / 255.0);
    let values = match space {
        Space::Native | Space::Srgb => srgb,
        Space::AdobeRgb => {
            let lin = srgb.map(srgb_to_linear);
            let mut out = [0.0f32; 3];
            for (i, row) in SRGB_TO_ADOBE.iter().enumerate() {
                let l = row[0] * lin[0] + row[1] * lin[1] + row[2] * lin[2];
                out[i] = l.clamp(0.0, 1.0).powf(1.0 / ADOBE_GAMMA);
            }
            out
        }
        Space::DisplayP3 => {
            let lin = srgb.map(srgb_to_linear);
            let mut out = [0.0f32; 3];
            for (i, row) in SRGB_TO_P3.iter().enumerate() {
                let l = row[0] * lin[0] + row[1] * lin[1] + row[2] * lin[2];
                out[i] = linear_to_srgb(l.clamp(0.0, 1.0));
            }
            out
        }
        Space::GenericRgb => srgb.map(|c| srgb_to_linear(c).max(0.0).powf(1.0 / GENERIC_GAMMA)),
    };
    let codes = Rgb8::new(quantize(values[0]), quantize(values[1]), quantize(values[2]));
    Reading { codes, values }
}

/// `#RRGGBB`, upper case.
pub fn hex(c: Rgb8) -> String {
    format!("#{:02X}{:02X}{:02X}", c.r, c.g, c.b)
}

/// The three 8-bit codes, separated by commas.
pub fn decimal(c: Rgb8) -> String {
    format!("{}, {}, {}", c.r, c.g, c.b)
}

/// The three normalised values as percentages with one decimal.
pub fn percent(values: [f32; 3]) -> String {
    format!("{:.1}%, {:.1}%, {:.1}%", values[0] * 100.0, values[1] * 100.0, values[2] * 100.0)
}

/// The mean of `pixels`, each channel rounded to the nearest code: what an aperture wider than one pixel reads. The
/// codes are averaged as encoded, the way the display shows them. `None` for no pixels.
pub fn average(pixels: &[Rgb8]) -> Option<Rgb8> {
    let n = pixels.len() as u32;
    if n == 0 {
        return None;
    }
    let mut sum = [0u32; 3];
    for p in pixels {
        sum[0] += p.r as u32;
        sum[1] += p.g as u32;
        sum[2] += p.b as u32;
    }
    let mean = |s: u32| ((s + n / 2) / n) as u8;
    Some(Rgb8::new(mean(sum[0]), mean(sum[1]), mean(sum[2])))
}

/// The three normalised values as 16-bit codes (0 to 65535).
pub fn codes16(values: [f32; 3]) -> [u16; 3] {
    values.map(|v| (v.clamp(0.0, 1.0) * 65535.0).round() as u16)
}

/// The three 16-bit codes, separated by commas.
pub fn bit16(values: [f32; 3]) -> String {
    let c = codes16(values);
    format!("{}, {}, {}", c[0], c[1], c[2])
}

/// The text a copy puts on the clipboard, and the value format the readout marks as chosen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CopyFormat {
    Hex,
    Decimal,
    Bit16,
    Percent,
}

impl CopyFormat {
    /// Every format, in the order the toolbar cycles through and the View menu lists them.
    pub const ALL: [CopyFormat; 4] = [CopyFormat::Hex, CopyFormat::Decimal, CopyFormat::Bit16, CopyFormat::Percent];

    pub fn next(self) -> CopyFormat {
        let i = CopyFormat::ALL.iter().position(|f| *f == self).unwrap_or(0);
        CopyFormat::ALL[(i + 1) % CopyFormat::ALL.len()]
    }

    pub fn name(self) -> &'static str {
        match self {
            CopyFormat::Hex => "Hex",
            CopyFormat::Decimal => "Decimal",
            CopyFormat::Bit16 => "16-bit",
            CopyFormat::Percent => "Percent",
        }
    }
}

/// The text for `reading` in `format`.
pub fn copy_text(reading: &Reading, format: CopyFormat) -> String {
    match format {
        CopyFormat::Hex => hex(reading.codes),
        CopyFormat::Decimal => decimal(reading.codes),
        CopyFormat::Bit16 => bit16(reading.values),
        CopyFormat::Percent => percent(reading.values),
    }
}

/// The most bytes of a display profile file that are read.
pub const MAX_PROFILE_BYTES: u64 = 16 << 20;

/// The ICC profile of the display, as an sRGB-to-profile transform. The display is driven with sRGB codes, so this is what
/// a reading looks like in the profile the display was characterised with.
#[derive(Clone, Debug)]
pub struct DisplayProfile {
    /// The profile's own description, or its file name when it has none.
    pub name: String,
    transform: ProfileToProfile,
}

impl DisplayProfile {
    /// Prepares the transform for the profile in `bytes`. `fallback_name` names it when it has no description. An error
    /// is a sentence: the bytes are not a profile, or it is one the transform cannot take (not an RGB profile).
    pub fn from_bytes(fallback_name: &str, bytes: &[u8]) -> Result<DisplayProfile, String> {
        let profile = Profile::parse(bytes).map_err(|e| format!("{fallback_name} is not a colour profile ({e:?})."))?;
        let name = profile.description(bytes).map(|d| d.trim().to_string()).filter(|d| !d.is_empty()).unwrap_or_else(|| fallback_name.to_string());
        let transform = ProfileToProfile::open(&Builtin::Srgb.bytes(), bytes, RenderingIntent::RelativeColorimetric, false)
            .map_err(|e| format!("{fallback_name} cannot be used as a display profile ({e:?})."))?;
        Ok(DisplayProfile { name, transform })
    }

    /// The reading of the sRGB pixel `px` in this profile, or None when the profile cannot take the colour. Values the
    /// display cannot show (outside its gamut) are cut to 0 to 1.
    pub fn convert(&self, px: Rgb8) -> Option<Reading> {
        let out = self.transform.apply([px.r, px.g, px.b].map(|c| c as f64 / 255.0))?;
        if out.iter().any(|v| !v.is_finite()) {
            return None;
        }
        let values = out.map(|c| c.clamp(0.0, 1.0) as f32);
        Some(Reading { codes: Rgb8::new(quantize(values[0]), quantize(values[1]), quantize(values[2])), values })
    }
}

/// The files that can hold the display's profile, best first: the file `BIFROST_DISPLAY_PROFILE` names, the user's
/// `display.icc` in the colour profile folder, and the system's.
pub fn profile_paths(from_env: Option<&str>, home: &Path) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Some(p) = from_env.filter(|p| !p.is_empty()) {
        paths.push(PathBuf::from(p));
    }
    paths.push(home.join(".local/share/color/icc/display.icc"));
    paths.push(PathBuf::from("/usr/share/color/icc/display.icc"));
    paths
}

/// The first of `paths` that is a usable display profile. A missing, oversized or unusable file is skipped.
pub fn load_display_profile(paths: &[PathBuf]) -> Option<DisplayProfile> {
    for path in paths {
        let Ok(meta) = std::fs::metadata(path) else { continue };
        if !meta.is_file() || meta.len() > MAX_PROFILE_BYTES {
            continue;
        }
        let Ok(bytes) = std::fs::read(path) else { continue };
        let fallback = path.file_stem().and_then(|s| s.to_str()).unwrap_or("Display").to_string();
        if let Ok(profile) = DisplayProfile::from_bytes(&fallback, &bytes) {
            return Some(profile);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-3
    }

    #[test]
    fn black_and_white_stay_put_in_every_space() {
        for space in Space::ALL {
            assert_eq!(convert(Rgb8::new(0, 0, 0), space).codes, Rgb8::new(0, 0, 0));
            assert_eq!(convert(Rgb8::new(255, 255, 255), space).codes, Rgb8::new(255, 255, 255));
        }
    }

    #[test]
    fn srgb_reading_is_the_pixel_itself() {
        let r = convert(Rgb8::new(12, 200, 99), Space::Srgb);
        assert_eq!(r.codes, Rgb8::new(12, 200, 99));
        assert!(close(r.values[1], 200.0 / 255.0));
    }

    #[test]
    fn pure_srgb_red_is_the_known_display_p3_code() {
        // sRGB red sits at (234, 51, 35) in Display P3: the matrix and transfer function are both exercised here.
        let r = convert(Rgb8::new(255, 0, 0), Space::DisplayP3);
        assert!((r.codes.r as i32 - 234).abs() <= 1, "{:?}", r.codes);
        assert!((r.codes.g as i32 - 51).abs() <= 1, "{:?}", r.codes);
        assert!((r.codes.b as i32 - 35).abs() <= 1, "{:?}", r.codes);
    }

    #[test]
    fn generic_rgb_lifts_mid_grey_by_the_1_8_gamma() {
        // sRGB 128 is linear 0.2159; 0.2159^(1/1.8) is about 0.426, which is code 109.
        let r = convert(Rgb8::new(128, 128, 128), Space::GenericRgb);
        assert!((r.codes.g as i32 - 109).abs() <= 1, "{:?}", r.codes);
    }

    #[test]
    fn transfer_functions_invert_each_other() {
        for i in 0..=255u32 {
            let c = i as f32 / 255.0;
            assert!(close(linear_to_srgb(srgb_to_linear(c)), c), "{}", c);
        }
    }

    #[test]
    fn hex_is_upper_case_and_padded() {
        assert_eq!(hex(Rgb8::new(0, 10, 255)), "#000AFF");
        assert_eq!(hex(Rgb8::new(171, 205, 239)), "#ABCDEF");
    }

    #[test]
    fn decimal_and_percent_formats() {
        assert_eq!(decimal(Rgb8::new(1, 22, 255)), "1, 22, 255");
        assert_eq!(percent([0.0, 0.5, 1.0]), "0.0%, 50.0%, 100.0%");
    }

    #[test]
    fn copy_text_follows_the_format() {
        let r = convert(Rgb8::new(255, 128, 0), Space::Srgb);
        assert_eq!(copy_text(&r, CopyFormat::Hex), "#FF8000");
        assert_eq!(copy_text(&r, CopyFormat::Decimal), "255, 128, 0");
        assert!(copy_text(&r, CopyFormat::Percent).starts_with("100.0%, 50.2%"));
    }

    #[test]
    fn cycles_return_to_the_start() {
        let mut s = Space::Srgb;
        for _ in 0..Space::ALL.len() {
            s = s.next();
        }
        assert_eq!(s, Space::Srgb);
        let mut f = CopyFormat::Hex;
        for _ in 0..CopyFormat::ALL.len() {
            f = f.next();
        }
        assert_eq!(f, CopyFormat::Hex);
    }

    #[test]
    fn native_shows_the_codes_unconverted() {
        let r = convert(Rgb8::new(12, 200, 99), Space::Native);
        assert_eq!(r.codes, Rgb8::new(12, 200, 99));
    }

    #[test]
    fn pure_srgb_green_is_the_known_adobe_rgb_code() {
        // sRGB green sits at (144, 255, 60) in Adobe RGB (1998).
        let r = convert(Rgb8::new(0, 255, 0), Space::AdobeRgb);
        assert!((r.codes.r as i32 - 144).abs() <= 1, "{:?}", r.codes);
        assert_eq!(r.codes.g, 255);
        assert!((r.codes.b as i32 - 60).abs() <= 1, "{:?}", r.codes);
    }

    #[test]
    fn adobe_rgb_greys_stay_grey_with_the_2_2_gamma() {
        // sRGB 128 is linear 0.2159; 0.2159^(1/2.2) is about 0.498, which is code 127.
        let r = convert(Rgb8::new(128, 128, 128), Space::AdobeRgb);
        assert!((r.codes.r as i32 - 127).abs() <= 1, "{:?}", r.codes);
        assert_eq!(r.codes.r, r.codes.g);
        assert_eq!(r.codes.g, r.codes.b);
    }

    #[test]
    fn sixteen_bit_codes_scale_the_values_to_65535() {
        assert_eq!(codes16([0.0, 0.5, 1.0]), [0, 32768, 65535]);
        assert_eq!(codes16([-1.0, 2.0, 0.0]), [0, 65535, 0]);
        // an 8-bit sRGB code c is exactly c * 257 in 16 bits
        let r = convert(Rgb8::new(255, 128, 0), Space::Srgb);
        assert_eq!(copy_text(&r, CopyFormat::Bit16), "65535, 32896, 0");
    }

    #[test]
    fn the_average_of_an_aperture_rounds_each_channel() {
        let px = [Rgb8::new(0, 10, 255), Rgb8::new(1, 11, 254), Rgb8::new(1, 11, 255), Rgb8::new(1, 10, 255)];
        assert_eq!(average(&px), Some(Rgb8::new(1, 11, 255)));
        assert_eq!(average(&[Rgb8::new(7, 8, 9)]), Some(Rgb8::new(7, 8, 9)));
        assert_eq!(average(&[]), None);
    }
}

#[cfg(test)]
mod profile_tests {
    use super::*;

    fn p3() -> DisplayProfile {
        DisplayProfile::from_bytes("fallback", &Builtin::DisplayP3.bytes()).expect("a built-in profile")
    }

    #[test]
    fn a_display_profile_reads_the_pixel_in_that_profile() {
        let profile = p3();
        assert_eq!(profile.name, "Display P3");
        // sRGB green is inside the P3 gamut at about (0.46, 0.98, 0.30)
        let via_profile = profile.convert(Rgb8::new(0, 255, 0)).expect("a reading");
        let direct = convert(Rgb8::new(0, 255, 0), Space::DisplayP3);
        for (a, b) in [(via_profile.codes.r, direct.codes.r), (via_profile.codes.g, direct.codes.g), (via_profile.codes.b, direct.codes.b)] {
            assert!((a as i32 - b as i32).abs() <= 3, "{:?} against {:?}", via_profile.codes, direct.codes);
        }
    }

    #[test]
    fn white_and_black_stay_put_and_values_stay_in_range() {
        let profile = p3();
        let white = profile.convert(Rgb8::new(255, 255, 255)).unwrap();
        assert!(white.codes.r >= 253 && white.codes.g >= 253 && white.codes.b >= 253, "{:?}", white.codes);
        assert_eq!(profile.convert(Rgb8::new(0, 0, 0)).unwrap().codes, Rgb8::new(0, 0, 0));
        for px in [Rgb8::new(255, 0, 0), Rgb8::new(0, 0, 255), Rgb8::new(7, 250, 3)] {
            let r = profile.convert(px).unwrap();
            assert!(r.values.iter().all(|v| (0.0..=1.0).contains(v)));
        }
    }

    #[test]
    fn bytes_that_are_not_a_profile_are_refused_with_a_sentence() {
        let err = DisplayProfile::from_bytes("junk", b"definitely not an icc profile").unwrap_err();
        assert!(err.contains("junk"), "{err}");
        assert!(DisplayProfile::from_bytes("empty", &[]).is_err());
        let mut broken = Builtin::Srgb.bytes();
        broken.truncate(100);
        assert!(DisplayProfile::from_bytes("cut", &broken).is_err());
    }

    #[test]
    fn the_candidate_files_are_the_named_one_then_the_user_then_the_system() {
        let home = Path::new("/home/me");
        assert_eq!(
            profile_paths(Some("/x/panel.icc"), home),
            vec![PathBuf::from("/x/panel.icc"), PathBuf::from("/home/me/.local/share/color/icc/display.icc"), PathBuf::from("/usr/share/color/icc/display.icc")]
        );
        assert_eq!(profile_paths(Some(""), home).len(), 2);
        assert_eq!(profile_paths(None, home).len(), 2);
    }

    #[test]
    fn the_first_usable_file_wins_and_missing_or_junk_files_are_skipped() {
        let dir = std::env::temp_dir().join(format!("bifrost-colormeter-profile-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let junk = dir.join("junk.icc");
        let good = dir.join("panel.icc");
        std::fs::write(&junk, b"junk").unwrap();
        std::fs::write(&good, Builtin::AdobeRgb1998.bytes()).unwrap();
        let found = load_display_profile(&[dir.join("missing.icc"), dir.clone(), junk, good]).expect("the good file");
        assert_eq!(found.name, "Adobe RGB (1998)");
        assert!(load_display_profile(&[dir.join("missing.icc")]).is_none());
        assert!(load_display_profile(&[]).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
