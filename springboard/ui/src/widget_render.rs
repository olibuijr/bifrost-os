//! How a widget's content is coloured: full colour, accented, or vibrant. Pure model of the rendering modes a widget
//! can be drawn in (`WidgetRenderingMode`) and the rule that tints an element. An element marked accentable takes the
//! user's accent colour in the accented mode; every other element keeps its own colour. The vibrant mode reduces an
//! element to its luminance, so the widget reads through the material behind it. The model says which colour each
//! element gets; nothing here draws.

/// The rendering mode a widget asks for, or the mode the shell resolves to.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RenderingMode {
    /// The widget's own colours.
    #[default]
    FullColor,
    /// Accentable elements take the user's accent colour.
    Accented,
    /// Every element becomes a grey of its luminance.
    Vibrant,
}

/// An RGBA colour, each channel 0 to 255.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rgba(pub u8, pub u8, pub u8, pub u8);

/// Resolves the mode a widget is drawn in. The accented and vibrant modes only exist under a tinted look (the user
/// has chosen tinted widgets); without it the shell draws the widget in full colour.
pub fn resolve_mode(requested: RenderingMode, tinted_look: bool) -> RenderingMode {
    match requested {
        RenderingMode::FullColor => RenderingMode::FullColor,
        _ if tinted_look => requested,
        _ => RenderingMode::FullColor,
    }
}

/// Luminance of a colour as a grey level, using the Rec. 601 weights in integer arithmetic.
pub fn luminance(c: Rgba) -> u8 {
    let y = (299 * c.0 as u32 + 587 * c.1 as u32 + 114 * c.2 as u32) / 1000;
    y as u8
}

/// The colour an element is drawn with. `color` is the element's own colour and `accent` the user's accent colour.
/// The alpha of `color` is kept in every mode.
pub fn tint(color: Rgba, accentable: bool, accent: Rgba, mode: RenderingMode) -> Rgba {
    match mode {
        RenderingMode::FullColor => color,
        RenderingMode::Accented if accentable => Rgba(accent.0, accent.1, accent.2, color.3),
        RenderingMode::Accented => color,
        RenderingMode::Vibrant => {
            let y = luminance(color);
            Rgba(y, y, y, color.3)
        }
    }
}

/// The colour a widget fills its container with (the reference `containerBackground(_:for: .widget)` given a colour).
/// A fixed set of system colours, so the shell resolves each to its named constant for the appearance in effect.
/// Red is left out: in BifrOSt red marks destructive actions (and the calendar's today), never a widget's fill.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ContainerTint {
    Orange,
    Yellow,
    Green,
    Mint,
    Teal,
    Cyan,
    Blue,
    Indigo,
    Purple,
    Pink,
    Brown,
    Gray,
}

impl ContainerTint {
    /// Every tint, in the order of the system colour list.
    pub const ALL: [ContainerTint; 12] = [
        ContainerTint::Orange,
        ContainerTint::Yellow,
        ContainerTint::Green,
        ContainerTint::Mint,
        ContainerTint::Teal,
        ContainerTint::Cyan,
        ContainerTint::Blue,
        ContainerTint::Indigo,
        ContainerTint::Purple,
        ContainerTint::Pink,
        ContainerTint::Brown,
        ContainerTint::Gray,
    ];

    /// The name an app sends for the tint.
    pub fn key(self) -> &'static str {
        match self {
            ContainerTint::Orange => "orange",
            ContainerTint::Yellow => "yellow",
            ContainerTint::Green => "green",
            ContainerTint::Mint => "mint",
            ContainerTint::Teal => "teal",
            ContainerTint::Cyan => "cyan",
            ContainerTint::Blue => "blue",
            ContainerTint::Indigo => "indigo",
            ContainerTint::Purple => "purple",
            ContainerTint::Pink => "pink",
            ContainerTint::Brown => "brown",
            ContainerTint::Gray => "gray",
        }
    }

    /// The tint named `key` (case and surrounding space ignored); `grey` is `gray`.
    pub fn from_key(key: &str) -> Option<ContainerTint> {
        let k = key.trim().to_ascii_lowercase();
        let k = if k == "grey" { "gray".to_string() } else { k };
        ContainerTint::ALL.iter().copied().find(|t| t.key() == k)
    }

    /// The text on the tint is light: every system colour is dark enough for white text except yellow and mint,
    /// whose light faces take dark text (luminance of the reference light values above 160).
    pub fn wants_light_text(self) -> bool {
        !matches!(self, ContainerTint::Yellow | ContainerTint::Mint)
    }
}

/// Whether the widget's container background is drawn in `mode`. The reference removes it in the accented and
/// vibrant modes, where the widget sits on the system's own material and only its content is tinted.
pub fn shows_container_background(mode: RenderingMode) -> bool {
    mode == RenderingMode::FullColor
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_container_tint_is_named_by_its_key() {
        for t in ContainerTint::ALL {
            assert_eq!(ContainerTint::from_key(t.key()), Some(t));
        }
        assert_eq!(ContainerTint::from_key(" Blue "), Some(ContainerTint::Blue));
        assert_eq!(ContainerTint::from_key("grey"), Some(ContainerTint::Gray));
        assert_eq!(ContainerTint::from_key("chartreuse"), None);
        assert_eq!(ContainerTint::from_key("red"), None, "red is kept for destructive actions");
        assert!(!ContainerTint::Yellow.wants_light_text() && ContainerTint::Blue.wants_light_text());
    }

    #[test]
    fn the_container_background_is_only_drawn_in_full_colour() {
        assert!(shows_container_background(RenderingMode::FullColor));
        assert!(!shows_container_background(RenderingMode::Accented));
        assert!(!shows_container_background(RenderingMode::Vibrant));
    }

    const BLUE: Rgba = Rgba(10, 132, 255, 255);
    const WHITE: Rgba = Rgba(255, 255, 255, 255);

    #[test]
    fn the_default_mode_is_full_colour() {
        assert_eq!(RenderingMode::default(), RenderingMode::FullColor);
    }

    #[test]
    fn accented_and_vibrant_need_a_tinted_look() {
        assert_eq!(resolve_mode(RenderingMode::Accented, false), RenderingMode::FullColor);
        assert_eq!(resolve_mode(RenderingMode::Vibrant, false), RenderingMode::FullColor);
        assert_eq!(resolve_mode(RenderingMode::Accented, true), RenderingMode::Accented);
        assert_eq!(resolve_mode(RenderingMode::FullColor, true), RenderingMode::FullColor);
    }

    #[test]
    fn full_colour_leaves_every_element_alone() {
        assert_eq!(tint(BLUE, true, WHITE, RenderingMode::FullColor), BLUE);
    }

    #[test]
    fn the_accented_mode_recolours_only_accentable_elements_and_keeps_alpha() {
        let accent = Rgba(255, 149, 0, 255);
        let half = Rgba(10, 132, 255, 128);
        assert_eq!(tint(half, true, accent, RenderingMode::Accented), Rgba(255, 149, 0, 128));
        assert_eq!(tint(BLUE, false, accent, RenderingMode::Accented), BLUE);
    }

    #[test]
    fn luminance_uses_integer_weights() {
        assert_eq!(luminance(WHITE), 255);
        assert_eq!(luminance(Rgba(0, 0, 0, 255)), 0);
        assert_eq!(luminance(Rgba(255, 0, 0, 255)), 76, "299 * 255 / 1000 = 76");
        assert_eq!(luminance(Rgba(0, 255, 0, 255)), 149, "587 * 255 / 1000 = 149");
    }

    #[test]
    fn the_vibrant_mode_is_a_grey_that_keeps_alpha() {
        assert_eq!(tint(BLUE, true, WHITE, RenderingMode::Vibrant), Rgba(luminance(BLUE), luminance(BLUE), luminance(BLUE), 255));
        let faded = Rgba(255, 255, 255, 40);
        assert_eq!(tint(faded, false, WHITE, RenderingMode::Vibrant), Rgba(255, 255, 255, 40));
    }
}
