//! Material recipes of the window server (pure, host-testable): which blur, saturation and tint a material kind gets,
//! in Dark and Light, active or inactive, and the frame-time budget of a blur.
//!
//! The kinds mirror the reference platform's visual-effect materials (titlebar, menu, popover, sidebar, header view,
//! sheet, window background, HUD, full-screen UI, tooltip, content background, under-window and under-page
//! background) plus the two Liquid Glass variants. The tint colours are the shell's own `MATERIAL_*` pairs
//! (`render/src/ui.rs`) so a client region and a shell window of one kind look the same.
//!
//! Rules (reference behaviour):
//! - Behind-window blending samples everything composited below the window (other clients, layers, the wallpaper);
//!   within-window blending samples only the surface's own content below the region. The server cannot isolate a
//!   window's own earlier pixels yet, so a within-window region resolves to the flat opaque plate of its kind.
//! - A window that is not key shows the opaque inactive plate instead of the blur, except for materials that follow
//!   no window state (menu, popover, sheet, HUD, full-screen UI, tooltip), which always render active.
//! - The blur is bounded: the number of dual-Kawase passes drops until the pyramid fits `BLUR_TEXEL_BUDGET`, which
//!   keeps one blurred window well under 2 ms on a Skylake HD 530 at 2560x1440.

use crate::render_server::material::{kawase_passes, pyramid_sizes};

/// The material kinds a client can ask for (`bifrost_material_surface_v1.kind`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MaterialKind {
    Titlebar,
    Menu,
    Popover,
    Sidebar,
    HeaderView,
    Sheet,
    WindowBackground,
    HudWindow,
    FullScreenUi,
    ToolTip,
    ContentBackground,
    UnderWindowBackground,
    UnderPageBackground,
    GlassRegular,
    GlassClear,
    /// A plain `ext-background-effect-v1` blur region: no kind was sent.
    Plain,
}

impl MaterialKind {
    /// The kind for a wire value, `None` when it is not one.
    pub fn from_wire(v: u32) -> Option<MaterialKind> {
        use MaterialKind::*;
        Some(match v {
            0 => Titlebar,
            1 => Menu,
            2 => Popover,
            3 => Sidebar,
            4 => HeaderView,
            5 => Sheet,
            6 => WindowBackground,
            7 => HudWindow,
            8 => FullScreenUi,
            9 => ToolTip,
            10 => ContentBackground,
            11 => UnderWindowBackground,
            12 => UnderPageBackground,
            13 => GlassRegular,
            14 => GlassClear,
            _ => return None,
        })
    }
}

/// Whether a region renders active or inactive (`bifrost_material_surface_v1.state`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MaterialState {
    FollowsWindow,
    Active,
    Inactive,
}

impl MaterialState {
    pub fn from_wire(v: u32) -> Option<MaterialState> {
        match v {
            0 => Some(MaterialState::FollowsWindow),
            1 => Some(MaterialState::Active),
            2 => Some(MaterialState::Inactive),
            _ => None,
        }
    }
}

/// What a region samples (`bifrost_material_surface_v1.blending`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Blending {
    BehindWindow,
    WithinWindow,
}

impl Blending {
    pub fn from_wire(v: u32) -> Option<Blending> {
        match v {
            0 => Some(Blending::BehindWindow),
            1 => Some(Blending::WithinWindow),
            _ => None,
        }
    }
}

/// One committed material region of a surface, in surface coordinates.
#[derive(Debug, Clone, PartialEq)]
pub struct MaterialSpec {
    pub rects: Vec<smithay::utils::Rectangle<i32, smithay::utils::Logical>>,
    pub kind: MaterialKind,
    pub state: MaterialState,
    pub blending: Blending,
    pub emphasized: bool,
    /// Corner radii (top left, top right, bottom right, bottom left) in logical px.
    pub radii: [u32; 4],
}

impl MaterialSpec {
    /// A plain `ext-background-effect-v1` rectangle list: the default material, behind the window, follows the window.
    pub fn plain(rects: Vec<smithay::utils::Rectangle<i32, smithay::utils::Logical>>) -> MaterialSpec {
        MaterialSpec {
            rects,
            kind: MaterialKind::Plain,
            state: MaterialState::FollowsWindow,
            blending: Blending::BehindWindow,
            emphasized: false,
            radii: [0; 4],
        }
    }

    /// The outline radius the compositor masks with. The compose shader has one radius per rectangle, so the largest
    /// of the four is used (a client rounding only some corners gets all of them rounded until the shader is per-corner).
    pub fn mask_radius(&self) -> f32 {
        self.radii.iter().copied().max().unwrap_or(0) as f32
    }
}

/// A straight (not premultiplied) RGBA colour, 0..=1.
pub type Rgba = [f32; 4];

/// The recipe of one kind: blur, saturation, tints for Dark and Light, and the opaque inactive plate.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Recipe {
    /// Blur radius in logical px.
    pub blur: f32,
    /// 1.0 keeps the backdrop's colours.
    pub saturation: f32,
    pub tint_dark: Rgba,
    pub tint_light: Rgba,
    /// The opaque plate shown instead of the blur when inactive (and for within-window blending).
    pub plate_dark: Rgba,
    pub plate_light: Rgba,
    /// Luminosity grain over the blur, 0..1 of full scale (the reference materials carry a faint noise).
    pub noise: f32,
    /// The material renders active whatever the window state (menus, HUD, ...).
    pub always_active: bool,
}

const fn c(r: u8, g: u8, b: u8, a: u8) -> Rgba {
    [r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, a as f32 / 255.0]
}

const SIDEBAR_PLATE: (Rgba, Rgba) = (c(20, 20, 22, 255), c(235, 235, 235, 255));
const TITLEBAR_PLATE: (Rgba, Rgba) = (c(28, 28, 30, 255), c(246, 246, 246, 255));
const WINDOW_PLATE: (Rgba, Rgba) = (c(30, 30, 32, 255), c(240, 240, 240, 255));
const HUD_PLATE: (Rgba, Rgba) = (c(12, 12, 14, 255), c(12, 12, 14, 255));

/// The recipe of `kind` (pure). Tints equal the shell's `MATERIAL_*` pairs where the shell has one.
pub fn recipe(kind: MaterialKind) -> Recipe {
    use MaterialKind::*;
    let r = |blur, saturation, tint_dark, tint_light, plate: (Rgba, Rgba), noise, always_active| Recipe {
        blur,
        saturation,
        tint_dark,
        tint_light,
        plate_dark: plate.0,
        plate_light: plate.1,
        noise,
        always_active,
    };
    match kind {
        Sidebar => r(24.0, 1.8, c(24, 24, 27, 190), c(236, 236, 236, 200), SIDEBAR_PLATE, 0.02, false),
        Titlebar | HeaderView => r(20.0, 1.6, c(28, 28, 30, 200), c(246, 246, 246, 205), TITLEBAR_PLATE, 0.02, false),
        Menu | Popover | Sheet => r(30.0, 1.8, c(28, 28, 30, 236), c(246, 246, 246, 238), TITLEBAR_PLATE, 0.02, true),
        WindowBackground | ContentBackground => r(20.0, 1.4, c(30, 30, 32, 228), c(240, 240, 240, 228), WINDOW_PLATE, 0.015, false),
        HudWindow | ToolTip | FullScreenUi => r(30.0, 1.6, c(12, 12, 14, 170), c(12, 12, 14, 170), HUD_PLATE, 0.02, true),
        UnderWindowBackground | UnderPageBackground => r(28.0, 1.8, c(24, 24, 27, 170), c(236, 236, 236, 175), SIDEBAR_PLATE, 0.02, false),
        GlassRegular => r(18.0, 1.8, c(255, 255, 255, 38), c(255, 255, 255, 70), TITLEBAR_PLATE, 0.0, false),
        GlassClear => r(10.0, 1.4, c(255, 255, 255, 18), c(255, 255, 255, 36), TITLEBAR_PLATE, 0.0, false),
        // the plain ext-background-effect blur keeps its old look (a white veil at 25 %) and never falls back to a plate
        Plain => r(20.0, 1.8, c(255, 255, 255, 64), c(255, 255, 255, 64), WINDOW_PLATE, 0.0, true),
    }
}

/// What the compositor draws for a region.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Resolved {
    /// Blur what is below, saturate, tint (straight RGBA tint).
    Glass { blur: f32, saturation: f32, tint: Rgba, noise: f32 },
    /// A flat opaque plate (inactive window, or within-window blending with nothing of its own below).
    Plate(Rgba),
}

/// Whether the region renders active: an explicit state wins, otherwise the window's, except for always-active kinds.
pub fn is_active(state: MaterialState, window_active: bool, always_active: bool) -> bool {
    match state {
        MaterialState::Active => true,
        MaterialState::Inactive => false,
        MaterialState::FollowsWindow => always_active || window_active,
    }
}

/// The emphasised (selected) variant: a stronger tint and a little more vibrancy.
fn emphasize(tint: Rgba, saturation: f32) -> (Rgba, f32) {
    ([tint[0], tint[1], tint[2], (tint[3] + 0.10).min(1.0)], saturation + 0.2)
}

/// What to draw for `spec` in a window that is `window_active`, in Dark or Light.
pub fn resolve(spec: &MaterialSpec, window_active: bool, dark: bool) -> Resolved {
    let rc = recipe(spec.kind);
    let plate = if dark { rc.plate_dark } else { rc.plate_light };
    if !is_active(spec.state, window_active, rc.always_active) || spec.blending == Blending::WithinWindow {
        return Resolved::Plate(plate);
    }
    let tint = if dark { rc.tint_dark } else { rc.tint_light };
    let (tint, saturation) = if spec.emphasized { emphasize(tint, rc.saturation) } else { (tint, rc.saturation) };
    Resolved::Glass { blur: rc.blur, saturation, tint, noise: rc.noise }
}

/// Premultiplies a straight colour.
pub fn premultiply(c: Rgba) -> Rgba {
    [c[0] * c[3], c[1] * c[3], c[2] * c[3], c[3]]
}

/// The most texels one blur may process (all pyramid levels, down and up): about 2 ms on a Skylake HD 530.
pub const BLUR_TEXEL_BUDGET: u64 = 3_500_000;

/// The texels one blur of a `w` x `h` device-pixel region with `passes` passes processes (down and up).
pub fn blur_cost(w: u32, h: u32, passes: u32) -> u64 {
    // the first level T_1 is half the region; the pyramid halves again per pass; every level is written twice
    // (down, then up). The copy of T_0 is a sampler read, not counted.
    let total: u64 = pyramid_sizes(w, h, passes).iter().map(|&(sw, sh)| sw as u64 * sh as u64).sum();
    total * 2
}

/// The blur radius (logical px) to use for a region of `w` x `h` device px at `scale`, lowered when the wanted
/// radius would cost more than `BLUR_TEXEL_BUDGET`. Never below one pass.
pub fn budgeted_blur(radius: f32, w: u32, h: u32, scale: f32) -> f32 {
    let scale = scale.max(0.1);
    let mut passes = kawase_passes(radius * scale);
    while passes > 1 && blur_cost(w, h, passes) > BLUR_TEXEL_BUDGET {
        passes -= 1;
    }
    // `kawase_passes` maps a radius to passes with reach 2^passes
    let reach = (1u32 << passes) as f32 / scale;
    radius.min(reach)
}

/// The material regions of a surface: the committed ones, or the plain blur regions when the client sent none.
pub fn effective(materials: Vec<MaterialSpec>, plain: Vec<smithay::utils::Rectangle<i32, smithay::utils::Logical>>) -> Vec<MaterialSpec> {
    if !materials.is_empty() {
        materials
    } else if plain.is_empty() {
        Vec::new()
    } else {
        vec![MaterialSpec::plain(plain)]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use smithay::utils::{Logical, Rectangle};

    fn rect(x: i32, y: i32, w: i32, h: i32) -> Rectangle<i32, Logical> {
        Rectangle::new((x, y).into(), (w, h).into())
    }

    fn spec(kind: MaterialKind) -> MaterialSpec {
        MaterialSpec { kind, ..MaterialSpec::plain(vec![rect(0, 0, 10, 10)]) }
    }

    #[test]
    fn every_wire_kind_maps_and_the_next_does_not() {
        for v in 0..=14 {
            assert!(MaterialKind::from_wire(v).is_some(), "kind {v}");
        }
        assert!(MaterialKind::from_wire(15).is_none());
        assert!(MaterialState::from_wire(3).is_none());
        assert!(Blending::from_wire(2).is_none());
    }

    #[test]
    fn the_sidebar_tint_is_the_shell_pair() {
        let rc = recipe(MaterialKind::Sidebar);
        assert_eq!(rc.tint_dark, c(24, 24, 27, 190));
        assert_eq!(rc.tint_light, c(236, 236, 236, 200));
        assert_eq!(rc.plate_dark[3], 1.0, "the inactive plate is opaque");
    }

    #[test]
    fn every_recipe_is_sane() {
        for v in 0..=14 {
            let rc = recipe(MaterialKind::from_wire(v).unwrap());
            assert!(rc.blur > 0.0 && rc.blur <= 64.0);
            assert!(rc.saturation >= 1.0 && rc.saturation <= 2.5);
            assert_eq!(rc.plate_dark[3], 1.0);
            assert_eq!(rc.plate_light[3], 1.0);
            assert!(rc.tint_dark[3] <= 1.0 && rc.tint_light[3] <= 1.0);
        }
    }

    #[test]
    fn a_sidebar_in_an_inactive_window_is_the_opaque_plate() {
        let s = spec(MaterialKind::Sidebar);
        assert_eq!(resolve(&s, false, true), Resolved::Plate(c(20, 20, 22, 255)));
        assert_eq!(resolve(&s, false, false), Resolved::Plate(c(235, 235, 235, 255)));
        assert!(matches!(resolve(&s, true, true), Resolved::Glass { .. }));
    }

    #[test]
    fn menus_and_huds_stay_blurred_in_an_inactive_window() {
        for k in [MaterialKind::Menu, MaterialKind::HudWindow, MaterialKind::Popover, MaterialKind::ToolTip] {
            assert!(matches!(resolve(&spec(k), false, true), Resolved::Glass { .. }), "{k:?}");
        }
    }

    #[test]
    fn an_explicit_state_wins_over_the_window() {
        let mut s = spec(MaterialKind::Sidebar);
        s.state = MaterialState::Active;
        assert!(matches!(resolve(&s, false, true), Resolved::Glass { .. }));
        let mut m = spec(MaterialKind::Menu);
        m.state = MaterialState::Inactive;
        assert!(matches!(resolve(&m, true, true), Resolved::Plate(_)));
    }

    #[test]
    fn within_window_has_no_blur_of_the_desktop() {
        let mut s = spec(MaterialKind::Sidebar);
        s.blending = Blending::WithinWindow;
        assert!(matches!(resolve(&s, true, true), Resolved::Plate(_)));
    }

    #[test]
    fn the_plain_blur_region_is_blurred_whatever_the_window_state() {
        let s = MaterialSpec::plain(vec![rect(0, 0, 4, 4)]);
        assert!(matches!(resolve(&s, false, true), Resolved::Glass { .. }));
    }

    #[test]
    fn emphasized_is_stronger() {
        let mut s = spec(MaterialKind::Sidebar);
        let Resolved::Glass { tint: a, saturation: sa, .. } = resolve(&s, true, true) else { panic!() };
        s.emphasized = true;
        let Resolved::Glass { tint: b, saturation: sb, .. } = resolve(&s, true, true) else { panic!() };
        assert!(b[3] > a[3] && sb > sa);
    }

    #[test]
    fn premultiply_scales_the_colour_by_alpha() {
        assert_eq!(premultiply([1.0, 0.5, 0.0, 0.5]), [0.5, 0.25, 0.0, 0.5]);
    }

    #[test]
    fn the_mask_radius_is_the_largest_corner() {
        let mut s = spec(MaterialKind::Sheet);
        s.radii = [4, 10, 2, 0];
        assert_eq!(s.mask_radius(), 10.0);
    }

    #[test]
    fn material_regions_replace_the_plain_blur_when_present() {
        let m = vec![spec(MaterialKind::Sidebar)];
        let out = effective(m.clone(), vec![rect(0, 0, 5, 5)]);
        assert_eq!(out, m);
        assert_eq!(effective(Vec::new(), vec![rect(0, 0, 5, 5)])[0].kind, MaterialKind::Plain);
        assert!(effective(Vec::new(), Vec::new()).is_empty());
    }

    #[test]
    fn a_full_screen_blur_stays_inside_the_budget() {
        let r = budgeted_blur(30.0, 2560, 1440, 1.0);
        let p = kawase_passes(r);
        assert!(p >= 1);
        assert!(blur_cost(2560, 1440, p) <= BLUR_TEXEL_BUDGET || p == 1);
    }

    #[test]
    fn a_small_region_keeps_its_radius() {
        assert_eq!(budgeted_blur(20.0, 300, 600, 1.0), 20.0);
    }

    #[test]
    fn the_budget_never_raises_the_radius() {
        assert!(budgeted_blur(4.0, 2560, 1440, 1.0) <= 4.0);
    }
}
