//! Text markup for legible media: the font, style, colour and placement a caption cue can carry, set over the
//! viewer's caption look in `captions`. A cue holds one `TextMarkup`. Merging a later cue's markup over an earlier one
//! replaces each attribute the later one sets and keeps the rest. This is the pure model: no drawing, and no reading
//! of container formats.
use crate::captions::EdgeStyle;

/// A colour as 0xAARRGGBB, the layout the attribute values use.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Argb(pub u32);

impl Argb {
    pub fn from_parts(alpha: u8, red: u8, green: u8, blue: u8) -> Self {
        Argb(((alpha as u32) << 24) | ((red as u32) << 16) | ((green as u32) << 8) | blue as u32)
    }

    pub fn alpha(self) -> u8 {
        (self.0 >> 24) as u8
    }

    pub fn red(self) -> u8 {
        (self.0 >> 16) as u8
    }

    pub fn green(self) -> u8 {
        (self.0 >> 8) as u8
    }

    pub fn blue(self) -> u8 {
        self.0 as u8
    }
}

/// Whether a block of text runs horizontally or vertically.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layout {
    Horizontal,
    Vertical,
}

/// Alignment of the first line in the writing direction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Alignment {
    Start,
    Center,
    End,
}

/// The markup attributes of one cue. An `Option` is unset until the cue states it. The three style flags are on or off,
/// and a cue can only turn them on.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TextMarkup {
    pub font_family: Option<String>,
    pub generic_font_family: Option<String>,
    /// Base font size as a percentage of the video height.
    pub base_size_percent: Option<f32>,
    /// Font size as a percentage of the default font size. Unset means 100.
    pub relative_size_percent: Option<f32>,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub edge: Option<EdgeStyle>,
    pub foreground: Option<Argb>,
    pub background: Option<Argb>,
    pub character_background: Option<Argb>,
    pub layout: Option<Layout>,
    pub alignment: Option<Alignment>,
    /// Block placement along the writing direction, as a percentage.
    pub position_percent: Option<f32>,
    /// First-line placement across the writing direction, as a percentage.
    pub line_position_percent: Option<f32>,
    /// Width (horizontal) or height (vertical) of the block, as a percentage of its bounding box.
    pub size_percent: Option<f32>,
}

impl TextMarkup {
    /// Applies `later` over `self`. Each attribute `later` sets replaces the one here; attributes it leaves unset stay.
    /// A style flag that `later` turns on is turned on here, and nothing turns a flag off.
    pub fn merge(&mut self, later: &TextMarkup) {
        if later.font_family.is_some() {
            self.font_family = later.font_family.clone();
        }
        if later.generic_font_family.is_some() {
            self.generic_font_family = later.generic_font_family.clone();
        }
        if later.base_size_percent.is_some() {
            self.base_size_percent = later.base_size_percent;
        }
        if later.relative_size_percent.is_some() {
            self.relative_size_percent = later.relative_size_percent;
        }
        self.bold |= later.bold;
        self.italic |= later.italic;
        self.underline |= later.underline;
        if later.edge.is_some() {
            self.edge = later.edge;
        }
        if later.foreground.is_some() {
            self.foreground = later.foreground;
        }
        if later.background.is_some() {
            self.background = later.background;
        }
        if later.character_background.is_some() {
            self.character_background = later.character_background;
        }
        if later.layout.is_some() {
            self.layout = later.layout;
        }
        if later.alignment.is_some() {
            self.alignment = later.alignment;
        }
        if later.position_percent.is_some() {
            self.position_percent = later.position_percent;
        }
        if later.line_position_percent.is_some() {
            self.line_position_percent = later.line_position_percent;
        }
        if later.size_percent.is_some() {
            self.size_percent = later.size_percent;
        }
    }

    /// The font height in pixels for a video `video_height_px` tall. The base size is `base_size_percent` when set,
    /// otherwise `default_percent`; the relative size scales that. Negative or non-finite inputs count as zero.
    pub fn font_height_px(&self, video_height_px: f32, default_percent: f32) -> f32 {
        let base = non_negative(self.base_size_percent.unwrap_or(default_percent));
        let relative = non_negative(self.relative_size_percent.unwrap_or(100.0));
        non_negative(video_height_px) * base / 100.0 * relative / 100.0
    }

    /// The top-left corner of the text block inside a `width` by `height` box. Along the writing direction the
    /// position is `position_percent`, and across it `line_position_percent`. A horizontal block runs along x, and a
    /// vertical one along y. Percentages are clamped to 0..=100 and unset ones count as 0.
    pub fn origin(&self, width: f32, height: f32) -> (f32, f32) {
        let along = clamp_pct(self.position_percent.unwrap_or(0.0));
        let across = clamp_pct(self.line_position_percent.unwrap_or(0.0));
        match self.layout.unwrap_or(Layout::Horizontal) {
            Layout::Horizontal => (width * along / 100.0, height * across / 100.0),
            Layout::Vertical => (width * across / 100.0, height * along / 100.0),
        }
    }
}

/// Keeps a percentage in 0..=100. NaN counts as 0.
fn clamp_pct(value: f32) -> f32 {
    if value.is_nan() {
        0.0
    } else {
        value.clamp(0.0, 100.0)
    }
}

/// Zero for negative, NaN or infinite values; the value itself otherwise.
fn non_negative(value: f32) -> f32 {
    if value.is_finite() && value > 0.0 {
        value
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-3
    }

    #[test]
    fn argb_packs_and_unpacks_channels() {
        let c = Argb::from_parts(0xFF, 0x12, 0x34, 0x56);
        assert_eq!(c.0, 0xFF12_3456);
        assert_eq!(c.alpha(), 0xFF);
        assert_eq!(c.red(), 0x12);
        assert_eq!(c.green(), 0x34);
        assert_eq!(c.blue(), 0x56);
    }

    #[test]
    fn merge_replaces_only_the_attributes_the_later_cue_sets() {
        let mut first = TextMarkup {
            font_family: Some("Sans".to_string()),
            foreground: Some(Argb(0xFF00_0000)),
            base_size_percent: Some(5.0),
            ..Default::default()
        };
        let later = TextMarkup {
            foreground: Some(Argb(0xFFFF_FFFF)),
            ..Default::default()
        };
        first.merge(&later);
        assert_eq!(first.font_family.as_deref(), Some("Sans"));
        assert_eq!(first.foreground, Some(Argb(0xFFFF_FFFF)));
        assert_eq!(first.base_size_percent, Some(5.0));
    }

    #[test]
    fn style_flags_turn_on_and_never_off_through_merge() {
        let mut first = TextMarkup { bold: true, ..Default::default() };
        first.merge(&TextMarkup { italic: true, ..Default::default() });
        assert!(first.bold);
        assert!(first.italic);
        first.merge(&TextMarkup::default());
        assert!(first.bold);
        assert!(first.italic);
        assert!(!first.underline);
    }

    #[test]
    fn font_height_scales_base_and_relative_size() {
        let mut m = TextMarkup { base_size_percent: Some(5.0), ..Default::default() };
        assert!(approx(m.font_height_px(1080.0, 4.0), 54.0));
        m.relative_size_percent = Some(50.0);
        assert!(approx(m.font_height_px(1080.0, 4.0), 27.0));
    }

    #[test]
    fn font_height_uses_default_percent_when_base_is_unset() {
        let m = TextMarkup::default();
        assert!(approx(m.font_height_px(1000.0, 4.0), 40.0));
    }

    #[test]
    fn font_height_treats_negative_relative_size_as_zero() {
        let m = TextMarkup {
            base_size_percent: Some(5.0),
            relative_size_percent: Some(-10.0),
            ..Default::default()
        };
        assert_eq!(m.font_height_px(1080.0, 4.0), 0.0);
    }

    #[test]
    fn horizontal_origin_runs_along_x_and_vertical_along_y() {
        let horizontal = TextMarkup {
            position_percent: Some(50.0),
            line_position_percent: Some(90.0),
            ..Default::default()
        };
        assert_eq!(horizontal.origin(1920.0, 1080.0), (960.0, 972.0));

        let vertical = TextMarkup {
            layout: Some(Layout::Vertical),
            position_percent: Some(10.0),
            line_position_percent: Some(50.0),
            ..Default::default()
        };
        assert_eq!(vertical.origin(1920.0, 1080.0), (960.0, 108.0));
    }

    #[test]
    fn origin_clamps_percentages_to_the_box() {
        let m = TextMarkup {
            position_percent: Some(150.0),
            line_position_percent: Some(-20.0),
            ..Default::default()
        };
        assert_eq!(m.origin(200.0, 100.0), (200.0, 0.0));
    }
}
