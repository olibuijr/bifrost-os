//! The accessible-appearance decisions an app makes from the user's display settings: reduced motion (a
//! transition becomes a short cross-fade), increased contrast (a higher minimum text contrast), reduced
//! transparency (translucent fills become opaque) and differentiation without colour (a state needs a shape or
//! text mark as well as its colour). The colour contrast figure is the WCAG 2 relative-luminance ratio.
//! Pure functions; the shell supplies the four flags.

/// Longest cross-fade used in place of a movement under reduced motion, in milliseconds.
pub const CROSSFADE_MS: u32 = 120;

/// WCAG 2 minimum contrast for normal text, and the stricter level used under increased contrast.
pub const MIN_CONTRAST: f64 = 4.5;
pub const MIN_CONTRAST_INCREASED: f64 = 7.0;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Preferences {
    pub reduce_motion: bool,
    pub increase_contrast: bool,
    pub reduce_transparency: bool,
    pub differentiate_without_color: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Transition {
    /// The element moves over `ms` milliseconds.
    Move { ms: u32 },
    /// The element stays where it is and fades over `ms` milliseconds.
    Crossfade { ms: u32 },
}

impl Preferences {
    /// How a movement of `base_ms` is shown. Under reduced motion it becomes a cross-fade of at most `CROSSFADE_MS`.
    pub fn transition(&self, base_ms: u32) -> Transition {
        if self.reduce_motion {
            Transition::Crossfade { ms: base_ms.min(CROSSFADE_MS) }
        } else {
            Transition::Move { ms: base_ms }
        }
    }

    /// The minimum contrast ratio for normal text.
    pub fn min_text_contrast(&self) -> f64 {
        if self.increase_contrast { MIN_CONTRAST_INCREASED } else { MIN_CONTRAST }
    }

    /// True when `fg` on `bg` reaches the minimum for normal text under these settings.
    pub fn text_meets_contrast(&self, fg: [u8; 3], bg: [u8; 3]) -> bool {
        contrast_ratio(fg, bg) >= self.min_text_contrast()
    }

    /// The opacity a translucent fill uses: 1.0 under reduced transparency, otherwise `alpha` clamped to 0..=1.
    pub fn fill_alpha(&self, alpha: f64) -> f64 {
        if self.reduce_transparency { 1.0 } else { alpha.clamp(0.0, 1.0) }
    }

    /// A state shown only by colour needs a second mark (a shape, an icon or text) when this is true.
    pub fn needs_non_colour_mark(&self) -> bool {
        self.differentiate_without_color
    }
}

/// WCAG 2 relative luminance of an sRGB colour, 0.0 for black and 1.0 for white.
pub fn relative_luminance(rgb: [u8; 3]) -> f64 {
    let lin = |c: u8| {
        let v = f64::from(c) / 255.0;
        if v <= 0.03928 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
    };
    0.2126 * lin(rgb[0]) + 0.7152 * lin(rgb[1]) + 0.0722 * lin(rgb[2])
}

/// WCAG 2 contrast ratio, from 1.0 (identical) to 21.0 (black on white). The order of the two colours does not matter.
pub fn contrast_ratio(a: [u8; 3], b: [u8; 3]) -> f64 {
    let la = relative_luminance(a);
    let lb = relative_luminance(b);
    let (hi, lo) = if la >= lb { (la, lb) } else { (lb, la) };
    (hi + 0.05) / (lo + 0.05)
}

#[cfg(test)]
mod tests {
    use super::*;

    const WHITE: [u8; 3] = [255, 255, 255];
    const BLACK: [u8; 3] = [0, 0, 0];

    #[test]
    fn reduced_motion_turns_movement_into_a_short_crossfade() {
        let p = Preferences { reduce_motion: true, ..Default::default() };
        assert_eq!(p.transition(300), Transition::Crossfade { ms: 120 });
        assert_eq!(p.transition(80), Transition::Crossfade { ms: 80 });
        assert_eq!(p.transition(0), Transition::Crossfade { ms: 0 });
        let normal = Preferences::default();
        assert_eq!(normal.transition(300), Transition::Move { ms: 300 });
    }

    #[test]
    fn black_on_white_is_twenty_one_to_one_and_order_does_not_matter() {
        assert!((contrast_ratio(BLACK, WHITE) - 21.0).abs() < 1e-9);
        assert!((contrast_ratio(WHITE, BLACK) - 21.0).abs() < 1e-9);
        assert!((contrast_ratio(WHITE, WHITE) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn grey_767676_on_white_passes_normal_text_only() {
        // #767676 has relative luminance about 0.1812, so the ratio is 1.05 / 0.2312, about 4.542.
        let r = contrast_ratio([0x76, 0x76, 0x76], WHITE);
        assert!(r > 4.54 && r < 4.55, "ratio was {r}");
        let normal = Preferences::default();
        assert!(normal.text_meets_contrast([0x76, 0x76, 0x76], WHITE));
        let increased = Preferences { increase_contrast: true, ..Default::default() };
        assert!(!increased.text_meets_contrast([0x76, 0x76, 0x76], WHITE));
    }

    #[test]
    fn mid_grey_777777_on_white_fails_the_normal_minimum() {
        // #777777 has relative luminance about 0.1845, so the ratio is 1.05 / 0.2345, about 4.478, under 4.5.
        let r = contrast_ratio([0x77, 0x77, 0x77], WHITE);
        assert!(r > 4.47 && r < 4.49, "ratio was {r}");
        assert!(!Preferences::default().text_meets_contrast([0x77, 0x77, 0x77], WHITE));
    }

    #[test]
    fn increased_contrast_raises_the_minimum_to_seven() {
        let p = Preferences { increase_contrast: true, ..Default::default() };
        assert_eq!(p.min_text_contrast(), 7.0);
        assert!(p.text_meets_contrast(BLACK, WHITE));
        assert_eq!(Preferences::default().min_text_contrast(), 4.5);
    }

    #[test]
    fn reduced_transparency_makes_fills_opaque_and_clamps_otherwise() {
        let p = Preferences { reduce_transparency: true, ..Default::default() };
        assert_eq!(p.fill_alpha(0.6), 1.0);
        let normal = Preferences::default();
        assert_eq!(normal.fill_alpha(0.6), 0.6);
        assert_eq!(normal.fill_alpha(1.5), 1.0);
        assert_eq!(normal.fill_alpha(-0.2), 0.0);
        assert!(!normal.needs_non_colour_mark());
        let dwc = Preferences { differentiate_without_color: true, ..Default::default() };
        assert!(dwc.needs_non_colour_mark());
    }
}
