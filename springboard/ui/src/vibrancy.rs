//! Vibrancy model (`MaterialView.allowsVibrancy`, `Appearance.vibrantLight/vibrantDark`, declarative UI
//! `foregroundStyle(.primary/.secondary/.tertiary/.quaternary)` on a material).
//!
//! Content placed on a material is not painted over the blurred backdrop: it is blended into it. In a dark
//! appearance light is added (plus-lighter), in a light appearance light is removed (plus-darker); the
//! level (label or fill, primary to quaternary) sets how much. Pure data and math, no drawing; the renderer
//! (`bifrost_render::vibrancy`) applies it to pixels on the CPU and the glass path.

/// Which blend a vibrant element uses over the backdrop.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Blend {
    /// Adds light: out = backdrop + amount (Dark).
    PlusLighter,
    /// Removes light: out = backdrop - amount (Light).
    PlusDarker,
    /// Ordinary source-over in the opaque label colour (no vibrancy: Reduce Transparency, or no material).
    Normal,
}

/// The hierarchy: `labelColor` .. `quaternaryLabelColor`, and the matching `fillColor` .. `quaternarySystemFill`
/// (declarative UI `.primary` .. `.quaternary` hierarchical styles).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Level {
    Primary,
    Secondary,
    Tertiary,
    Quaternary,
}

/// What is drawn: text and symbols use the label ramp, shapes use the fill ramp.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Role {
    Label,
    Fill,
}

impl Level {
    pub const ALL: [Level; 4] = [Level::Primary, Level::Secondary, Level::Tertiary, Level::Quaternary];

    /// declarative UI hierarchical style index (`.primary` = 0).
    pub fn hierarchy(self) -> usize {
        self as usize
    }
}

/// How much light the element adds (Dark) or removes (Light), 0..1. Labels follow the label colours' alphas
/// (0.85 / 0.55 / 0.25 / 0.10 in Dark; 0.85 / 0.50 / 0.26 / 0.10 in Light); fills follow the system fill alphas
/// (0.36 / 0.32 / 0.24 / 0.18 Dark, 0.20 / 0.16 / 0.12 / 0.08 Light). The platform's exact vibrant values are private.
pub fn strength(role: Role, level: Level, light: bool) -> f32 {
    let i = level.hierarchy();
    match (role, light) {
        (Role::Label, false) => [0.85, 0.55, 0.25, 0.10][i],
        (Role::Label, true) => [0.85, 0.50, 0.26, 0.10][i],
        (Role::Fill, false) => [0.36, 0.32, 0.24, 0.18][i],
        (Role::Fill, true) => [0.20, 0.16, 0.12, 0.08][i],
    }
}

/// The strength at a position between Dark (`scheme` 0) and Light (1), following the appearance cross-fade.
pub fn strength_at(role: Role, level: Level, scheme: f32) -> f32 {
    let d = strength(role, level, false);
    let l = strength(role, level, true);
    d + (l - d) * scheme.clamp(0.0, 1.0)
}

/// The blend for an appearance. Reduce Transparency, or content that is not on a material, falls back to
/// `Normal` (opaque label colours at the same strength as alpha).
pub fn blend(light: bool, on_material: bool, reduce_transparency: bool) -> Blend {
    if !on_material || reduce_transparency {
        Blend::Normal
    } else if light {
        Blend::PlusDarker
    } else {
        Blend::PlusLighter
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ramps_fall_with_the_level() {
        for role in [Role::Label, Role::Fill] {
            for light in [false, true] {
                let v: Vec<f32> = Level::ALL.iter().map(|&l| strength(role, l, light)).collect();
                assert!(v.windows(2).all(|w| w[0] > w[1]), "{role:?} {light}: {v:?}");
            }
        }
    }

    #[test]
    fn label_levels_match_the_label_alphas() {
        assert_eq!(strength(Role::Label, Level::Secondary, false), 0.55);
        assert_eq!(strength(Role::Label, Level::Secondary, true), 0.50);
        assert_eq!(strength(Role::Label, Level::Tertiary, true), 0.26);
        assert_eq!(Level::Quaternary.hierarchy(), 3);
    }

    #[test]
    fn blend_follows_appearance_and_fallback() {
        assert_eq!(blend(false, true, false), Blend::PlusLighter);
        assert_eq!(blend(true, true, false), Blend::PlusDarker);
        assert_eq!(blend(false, true, true), Blend::Normal);
        assert_eq!(blend(true, false, false), Blend::Normal);
    }

    #[test]
    fn cross_fade_interpolates() {
        let m = strength_at(Role::Label, Level::Secondary, 0.5);
        assert!((m - 0.525).abs() < 1e-6);
        assert_eq!(strength_at(Role::Fill, Level::Primary, 9.0), 0.20);
    }
}
