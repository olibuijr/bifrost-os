//! Custom note colours: parsing and printing `#RRGGBB`, a legible text colour for any background, and tinting a
//! custom colour into the pale body and the stronger title bar a sticky note draws. Pure std.
#![allow(dead_code)] // Format > Custom Colour... and the JSON field for it wire this in next

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Rgb {
    pub const fn new(r: u8, g: u8, b: u8) -> Rgb {
        Rgb { r, g, b }
    }

    /// Reads `#RRGGBB`, `RRGGBB` or `#RGB`.
    pub fn parse(text: &str) -> Option<Rgb> {
        let t = text.trim().trim_start_matches('#');
        if !t.is_ascii() {
            return None;
        }
        let byte = |s: &str| u8::from_str_radix(s, 16).ok();
        match t.len() {
            6 => Some(Rgb::new(byte(&t[0..2])?, byte(&t[2..4])?, byte(&t[4..6])?)),
            3 => {
                let d = |i: usize| byte(&t[i..i + 1]).map(|v| v * 17);
                Some(Rgb::new(d(0)?, d(1)?, d(2)?))
            }
            _ => None,
        }
    }

    pub fn hex(self) -> String {
        format!("#{:02X}{:02X}{:02X}", self.r, self.g, self.b)
    }

    /// Relative luminance, 0.0 (black) to 1.0 (white).
    pub fn luminance(self) -> f32 {
        let lin = |c: u8| {
            let v = f32::from(c) / 255.0;
            if v <= 0.03928 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
        };
        0.2126 * lin(self.r) + 0.7152 * lin(self.g) + 0.0722 * lin(self.b)
    }

    /// Black or white text, whichever reads better on this background.
    pub fn text(self) -> Rgb {
        if self.luminance() > 0.35 { Rgb::new(0, 0, 0) } else { Rgb::new(255, 255, 255) }
    }

    /// Mixes towards white by `amount` (0.0 to 1.0).
    pub fn lighten(self, amount: f32) -> Rgb {
        let m = |c: u8| (f32::from(c) + (255.0 - f32::from(c)) * amount.clamp(0.0, 1.0)).round() as u8;
        Rgb::new(m(self.r), m(self.g), m(self.b))
    }

    /// Mixes towards black by `amount` (0.0 to 1.0).
    pub fn darken(self, amount: f32) -> Rgb {
        let m = |c: u8| (f32::from(c) * (1.0 - amount.clamp(0.0, 1.0))).round() as u8;
        Rgb::new(m(self.r), m(self.g), m(self.b))
    }

    /// The pale body colour and the title bar colour of a note whose chosen colour is `self`.
    pub fn note_colours(self) -> (Rgb, Rgb) {
        (self.lighten(0.55), self.darken(0.08))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_and_print() {
        assert_eq!(Rgb::parse("#FFcc00"), Some(Rgb::new(255, 204, 0)));
        assert_eq!(Rgb::parse("0a0B0c"), Some(Rgb::new(10, 11, 12)));
        assert_eq!(Rgb::parse("#f80"), Some(Rgb::new(255, 136, 0)));
        assert_eq!(Rgb::parse("#12"), None);
        assert_eq!(Rgb::parse("#gggggg"), None);
        assert_eq!(Rgb::parse("#ééé"), None);
        assert_eq!(Rgb::new(255, 204, 0).hex(), "#FFCC00");
        assert_eq!(Rgb::parse(&Rgb::new(1, 2, 3).hex()), Some(Rgb::new(1, 2, 3)));
    }

    #[test]
    fn text_colour_follows_luminance() {
        assert_eq!(Rgb::new(255, 255, 0).text(), Rgb::new(0, 0, 0));
        assert_eq!(Rgb::new(10, 20, 80).text(), Rgb::new(255, 255, 255));
        assert!(Rgb::new(255, 255, 255).luminance() > 0.99);
        assert!(Rgb::new(0, 0, 0).luminance() < 0.01);
    }

    #[test]
    fn mixing() {
        assert_eq!(Rgb::new(100, 100, 100).lighten(1.0), Rgb::new(255, 255, 255));
        assert_eq!(Rgb::new(100, 100, 100).darken(0.5), Rgb::new(50, 50, 50));
        assert_eq!(Rgb::new(100, 100, 100).lighten(0.0), Rgb::new(100, 100, 100));
        let (body, bar) = Rgb::new(200, 100, 0).note_colours();
        assert!(body.r > 200 && bar.r < 200);
    }
}
