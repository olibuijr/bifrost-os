//! Pure colour conversions for the Color Meter. A pixel read from the display is 8-bit sRGB; this module converts it to
//! Display P3 and to Generic RGB and formats the result as hex, decimal and percent. Nothing here reads the screen or
//! the clipboard. The sRGB transfer function and the sRGB-to-Display-P3 matrix are public standards. Generic RGB is
//! modelled as a gamma of 1.8 on the sRGB primaries, which is an approximation until its own primaries are wired in.

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
    Srgb,
    DisplayP3,
    GenericRgb,
}

impl Space {
    /// The order the toolbar cycles through.
    pub fn next(self) -> Space {
        match self {
            Space::Srgb => Space::DisplayP3,
            Space::DisplayP3 => Space::GenericRgb,
            Space::GenericRgb => Space::Srgb,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Space::Srgb => "sRGB",
            Space::DisplayP3 => "Display P3",
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
        Space::Srgb => srgb,
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

/// The text a copy puts on the clipboard.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CopyFormat {
    Hex,
    Decimal,
    Percent,
}

impl CopyFormat {
    pub fn next(self) -> CopyFormat {
        match self {
            CopyFormat::Hex => CopyFormat::Decimal,
            CopyFormat::Decimal => CopyFormat::Percent,
            CopyFormat::Percent => CopyFormat::Hex,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            CopyFormat::Hex => "Hex",
            CopyFormat::Decimal => "Decimal",
            CopyFormat::Percent => "Percent",
        }
    }
}

/// The text for `reading` in `format`.
pub fn copy_text(reading: &Reading, format: CopyFormat) -> String {
    match format {
        CopyFormat::Hex => hex(reading.codes),
        CopyFormat::Decimal => decimal(reading.codes),
        CopyFormat::Percent => percent(reading.values),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-3
    }

    #[test]
    fn black_and_white_stay_put_in_every_space() {
        for space in [Space::Srgb, Space::DisplayP3, Space::GenericRgb] {
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
        for _ in 0..3 {
            s = s.next();
        }
        assert_eq!(s, Space::Srgb);
        let mut f = CopyFormat::Hex;
        for _ in 0..3 {
            f = f.next();
        }
        assert_eq!(f, CopyFormat::Hex);
    }
}
