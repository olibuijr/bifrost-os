//! Colour helpers for the Browser's own drawing. Every colour comes from a named token of `kit::ui`; the two derived
//! here are black at a given alpha (the page dimming behind sheets) and CSS text built from a token (the element
//! overlay injected into pages). No literal colour lives in the app code.
use bifrost_app::kit::ui::BLACK;

/// The switch knob: white in both appearances (the switch knob does not follow Light or Dark).
pub const KNOB: u32 = bifrost_app::kit::ui::WHITE;

/// Black at `alpha` (0-255), premultiplied as the surface expects: black has no colour channels to scale.
pub fn scrim(alpha: u8) -> u32 {
    (BLACK & 0x00ff_ffff) | (alpha as u32) << 24
}

/// `rgb(r,g,b)` CSS for the opaque part of a token.
pub fn css_rgb(c: u32) -> String {
    format!("rgb({},{},{})", c & 0xff, (c >> 8) & 0xff, (c >> 16) & 0xff)
}

/// `rgba(r,g,b,a)` CSS for a token at `alpha` (0.0-1.0).
pub fn css_rgba(c: u32, alpha: f32) -> String {
    format!("rgba({},{},{},{alpha})", c & 0xff, (c >> 8) & 0xff, (c >> 16) & 0xff)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_app::kit::ui::TINT;

    #[test]
    fn scrim_is_black_at_the_given_alpha() {
        assert_eq!(scrim(110) >> 24, 110);
        assert_eq!(scrim(110) & 0x00ff_ffff, 0, "premultiplied black has no colour");
        assert_eq!(scrim(0), 0);
    }

    #[test]
    fn css_reads_the_token_channels() {
        // TINT is pack(10, 132, 255, 255): r, g, b in the low three bytes
        assert_eq!(css_rgb(TINT), "rgb(10,132,255)");
        assert_eq!(css_rgba(TINT, 0.2), "rgba(10,132,255,0.2)");
    }
}
