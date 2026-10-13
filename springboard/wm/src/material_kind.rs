//! Material recipes of the window server (pure, host-testable): which blur, saturation and tint a material kind gets,
//! in Dark and Light, active or inactive, and the frame-time budget of a blur.
//!
//! The kinds mirror the reference platform's visual-effect materials (titlebar, menu, popover, sidebar, header view,
//! sheet, window background, HUD, full-screen UI, tooltip, content background, under-window and under-page
//! background) plus the two Liquid Glass variants. The numbers (blur, saturation, brightness, luminance mapping) are
//! measured from the platform's material recipes (`docs/reference/materials-recipes.md`); the opaque plates equal the
//! flat colours of the shell's `MATERIAL_*` pairs (`render/src/ui.rs`) so a client region and a shell window of one
//! kind look the same.
//!
//! Rules (reference behaviour):
//! - Behind-window blending samples everything composited below the window (other clients, layers, the wallpaper);
//!   within-window blending samples only the surface's own content below the region. The server cannot isolate a
//!   window's own earlier pixels yet, so a within-window region resolves to the flat opaque plate of its kind.
//! - A window that is not key shows the opaque inactive plate instead of the blur, except for materials that follow
//!   no window state (menu, popover, sheet, HUD, full-screen UI, tooltip), which always render active.
//! - The blur is bounded: the number of dual-Kawase passes drops until the pyramid fits `BLUR_TEXEL_BUDGET`, which
//!   keeps one blurred window well under 2 ms on a Skylake HD 530 at 2560x1440.

use crate::material_filter::Tone;
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
    /// `.selection` (the accent plate of a selected row).
    Selection,
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
            15 => Selection,
            _ => return None,
        })
    }

    /// The window-server kind of a shared model kind (`bifrost_ui::visual_effect`). Exhaustive on purpose: a kind added
    /// to the model does not compile here until it has a window-server kind.
    pub fn from_model(k: bifrost_ui::visual_effect::MaterialKind) -> MaterialKind {
        use bifrost_ui::visual_effect::MaterialKind as K;
        match k {
            K::Sidebar => MaterialKind::Sidebar,
            K::Titlebar => MaterialKind::Titlebar,
            K::HeaderView => MaterialKind::HeaderView,
            K::WindowBackground => MaterialKind::WindowBackground,
            K::ContentBackground => MaterialKind::ContentBackground,
            K::UnderWindowBackground => MaterialKind::UnderWindowBackground,
            K::UnderPageBackground => MaterialKind::UnderPageBackground,
            K::Menu => MaterialKind::Menu,
            K::Popover => MaterialKind::Popover,
            K::Sheet => MaterialKind::Sheet,
            K::HudWindow => MaterialKind::HudWindow,
            K::FullScreenUi => MaterialKind::FullScreenUi,
            K::ToolTip => MaterialKind::ToolTip,
            K::Selection => MaterialKind::Selection,
        }
    }

    /// The window-server kind of a whole shared request: the glass recipe when it asks for Liquid Glass, else its kind.
    pub fn from_request(r: &bifrost_ui::visual_effect::MaterialRequest) -> MaterialKind {
        match r.glass {
            Some(bifrost_ui::visual_effect::GlassKind::Regular) => MaterialKind::GlassRegular,
            Some(bifrost_ui::visual_effect::GlassKind::Clear) => MaterialKind::GlassClear,
            None => MaterialKind::from_model(r.kind),
        }
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

/// The recipe of one kind: blur, saturation and tone for Dark and Light, a tint, and the opaque inactive plate.
///
/// The numbers of the measured kinds come from the reference OS 27 material recipes (`docs/reference/materials-recipes.md`);
/// kinds without a recipe (clear glass, selection, the plain blur) keep their designed values and are marked below.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Recipe {
    /// Blur radius in logical px (the recipe's `blurRadius`).
    pub blur: f32,
    /// The scale the backdrop is blurred at, relative to the output (the recipe's `backdropScale`).
    pub backdrop_scale: f32,
    pub saturation_dark: f32,
    pub saturation_light: f32,
    /// Luminance mapping and brightness (`material_filter`).
    pub tone_dark: Tone,
    pub tone_light: Tone,
    /// A veil over the filtered blur. The measured recipes carry none (alpha 0); glass and selection do.
    pub tint_dark: Rgba,
    pub tint_light: Rgba,
    /// The opaque plate shown instead of the blur when inactive (and for within-window blending, and Reduce
    /// Transparency): the recipe's tone applied to a neutral backdrop (`NEUTRAL_DARK`, `NEUTRAL_LIGHT`).
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

/// The luma of the neutral backdrop the opaque plates are computed over: a typical wallpaper under each appearance
/// (our choice; the recipes themselves have no fixed backdrop).
pub const NEUTRAL_DARK: f32 = 0.15;
pub const NEUTRAL_LIGHT: f32 = 0.85;

// Measured tones (platform recipe files; brightness, luminance amount, the four luminance values).
const CHROME_DARK: Tone = Tone::new(-0.1, 0.75, [0.23, 0.52, 0.27, 0.255]);
const CHROME_LIGHT: Tone = Tone::new(0.1, 0.75, [0.8, 0.9, 1.1, 0.825]);
/// Reduce Transparency variants of the chrome (the backdrop is replaced by its average colour).
pub const CHROME_DARK_REDUCED: Tone = Tone::new(-0.1, 0.85, [0.225, 0.31, 0.25, 0.2]);
pub const CHROME_LIGHT_REDUCED: Tone = Tone::new(0.1, 0.75, [0.95, 0.9, 1.1, 0.825]);
const REGULAR_DARK: Tone = Tone::new(0.0, 0.75, [0.16, 0.26, 0.1, 0.1]);
const REGULAR_LIGHT: Tone = Tone::new(0.1, 0.75, [0.9, 0.83, 0.925, 0.815]);
const THICK_DARK: Tone = Tone::new(0.0, 0.88, [0.14, 0.16, 0.1, 0.03]);
const THICK_LIGHT: Tone = Tone::new(0.045, 0.88, [0.99, 0.95, 0.98, 0.905]);
const THIN_DARK: Tone = Tone::new(0.0, 0.6, [0.2, 0.21, 0.1, 0.15]);
const THIN_LIGHT: Tone = Tone::new(0.12, 0.6, [0.725, 0.825, 0.76, 0.73]);
const ULTRA_THIN_DARK: Tone = Tone::new(0.0, 0.5, [0.24, 0.24, 0.3, 0.39]);
const ULTRA_THIN_LIGHT: Tone = Tone::new(0.12, 0.5, [0.45, 0.55, 0.65, 0.68]);
const DOCK_DARK: Tone = Tone::new(0.0, 0.5, [0.29, -0.2, 0.375, 0.65]);
const DOCK_LIGHT: Tone = Tone::new(0.08, 0.5, [0.3, 0.5, 1.0, 0.77]);

/// The opaque grey a tone gives the neutral backdrop of an appearance.
pub fn plate_of(tone: Tone, dark: bool) -> Rgba {
    let n = if dark { NEUTRAL_DARK } else { NEUTRAL_LIGHT };
    let g = tone.apply([n; 3]);
    [g[0], g[1], g[2], 1.0]
}

/// A measured recipe: no veil, the plates from the plate tones (the Reduce Transparency tones where the platform has
/// them).
fn measured(blur: f32, saturation: (f32, f32), tone: (Tone, Tone), plate_tone: (Tone, Tone), always_active: bool) -> Recipe {
    let (pd, pl) = (plate_of(plate_tone.0, true), plate_of(plate_tone.1, false));
    Recipe {
        blur,
        backdrop_scale: 0.25,
        saturation_dark: saturation.0,
        saturation_light: saturation.1,
        tone_dark: tone.0,
        tone_light: tone.1,
        tint_dark: [pd[0], pd[1], pd[2], 0.0],
        tint_light: [pl[0], pl[1], pl[2], 0.0],
        plate_dark: pd,
        plate_light: pl,
        noise: 0.02,
        always_active,
    }
}

/// A recipe that stays dark in both appearances (HUD, tool tip, full-screen UI).
fn measured_dark(blur: f32, saturation: f32, tone: Tone) -> Recipe {
    measured(blur, (saturation, saturation), (tone, tone), (tone, tone), true)
}

/// A designed recipe (no platform recipe): constant tone, explicit tints and plates.
fn designed(blur: f32, saturation: f32, tint_dark: Rgba, tint_light: Rgba, plate: (Rgba, Rgba), noise: f32, always_active: bool) -> Recipe {
    Recipe {
        blur,
        backdrop_scale: 0.25,
        saturation_dark: saturation,
        saturation_light: saturation,
        tone_dark: Tone::NONE,
        tone_light: Tone::NONE,
        tint_dark,
        tint_light,
        plate_dark: plate.0,
        plate_light: plate.1,
        noise,
        always_active,
    }
}

const TITLEBAR_PLATE: (Rgba, Rgba) = (c(28, 28, 30, 255), c(246, 246, 246, 255));
const WINDOW_PLATE: (Rgba, Rgba) = (c(30, 30, 32, 255), c(240, 240, 240, 255));

/// The recipe of `kind` (pure). Which platform recipe each kind takes is in `docs/reference/materials-recipes.md`.
pub fn recipe(kind: MaterialKind) -> Recipe {
    use MaterialKind::*;
    match kind {
        Sidebar => measured(30.0, (1.35, 1.35), (THIN_DARK, THIN_LIGHT), (THIN_DARK, THIN_LIGHT), false),
        Titlebar | HeaderView => measured(22.5, (2.0, 1.1), (CHROME_DARK, CHROME_LIGHT), (CHROME_DARK_REDUCED, CHROME_LIGHT_REDUCED), false),
        Menu | Popover | Sheet => measured(30.0, (1.5, 1.5), (REGULAR_DARK, REGULAR_LIGHT), (REGULAR_DARK, REGULAR_LIGHT), true),
        WindowBackground | ContentBackground => measured(45.0, (1.5, 1.5), (THICK_DARK, THICK_LIGHT), (THICK_DARK, THICK_LIGHT), false),
        HudWindow | ToolTip | FullScreenUi => measured_dark(45.0, 1.5, THICK_DARK),
        UnderWindowBackground => measured(22.5, (1.1, 1.1), (ULTRA_THIN_DARK, ULTRA_THIN_LIGHT), (ULTRA_THIN_DARK, ULTRA_THIN_LIGHT), false),
        UnderPageBackground => measured(30.0, (1.35, 1.35), (THIN_DARK, THIN_LIGHT), (THIN_DARK, THIN_LIGHT), false),
        // Regular glass: the platform's glass recipe is a blur of 45 and a colour matrix, which decomposes exactly into
        // saturation 1.5885, then a grey veil (0.6343) at alpha 0.3705 (gain 0.6295, offset 0.235).
        GlassRegular => {
            let veil = [0.6343, 0.6343, 0.6343, 0.3705];
            let mut r = designed(45.0, 1.5885, veil, veil, TITLEBAR_PLATE, 0.0, false);
            r.plate_dark = plate_of(CHROME_DARK_REDUCED, true);
            r.plate_light = plate_of(CHROME_LIGHT_REDUCED, false);
            r
        }
        // Not in any recipe file (designed): the accent plate, clear glass and the plain ext-background-effect blur.
        Selection => designed(12.0, 1.6, c(10, 95, 214, 170), c(10, 132, 255, 150), TITLEBAR_PLATE, 0.0, false),
        GlassClear => designed(10.0, 1.4, c(255, 255, 255, 18), c(255, 255, 255, 36), TITLEBAR_PLATE, 0.0, false),
        // the plain ext-background-effect blur keeps its old look (a white veil at 25 %) and never falls back to a plate
        Plain => designed(20.0, 1.8, c(255, 255, 255, 64), c(255, 255, 255, 64), WINDOW_PLATE, 0.0, true),
    }
}

/// The recipe of the shell's dock (`ShellSurface::Dock`, which is `UnderWindowBackground` in the shared model): the
/// platform has a dedicated dock recipe pair.
pub fn dock_recipe() -> Recipe {
    measured(30.0, (1.6, 1.8), (DOCK_DARK, DOCK_LIGHT), (DOCK_DARK, DOCK_LIGHT), true)
}

/// What the compositor draws for a region.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Resolved {
    /// Blur what is below, saturate, map the luminance, add the brightness, tint (straight RGBA tint).
    Glass { blur: f32, saturation: f32, tone: Tone, tint: Rgba, noise: f32 },
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
    let (base_saturation, tone) = if dark { (rc.saturation_dark, rc.tone_dark) } else { (rc.saturation_light, rc.tone_light) };
    let (tint, saturation) = if spec.emphasized { emphasize(tint, base_saturation) } else { (tint, base_saturation) };
    Resolved::Glass { blur: rc.blur, saturation, tone, tint, noise: rc.noise }
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
        for v in 0..=15 {
            assert!(MaterialKind::from_wire(v).is_some(), "kind {v}");
        }
        assert!(MaterialKind::from_wire(16).is_none());
        assert!(MaterialState::from_wire(3).is_none());
        assert!(Blending::from_wire(2).is_none());
    }

    /// The table that keeps the shared model and the window server in step: every model kind maps to a distinct
    /// window-server kind whose protocol value is the model's `window_server_wire`, and the protocol values the model
    /// does not use are exactly the two glass recipes.
    #[test]
    fn model_kinds_map_one_to_one_onto_the_protocol_kinds() {
        use bifrost_ui::visual_effect::{GlassKind, MaterialKind as K, MaterialRequest, WINDOW_SERVER_GLASS_CLEAR, WINDOW_SERVER_GLASS_REGULAR};
        let mut seen = std::collections::HashSet::new();
        for k in K::ALL {
            let wm = MaterialKind::from_model(k);
            assert_eq!(MaterialKind::from_wire(k.window_server_wire()), Some(wm), "{k:?}");
            assert!(seen.insert(wm), "{k:?} maps to a kind another model kind already has");
        }
        assert_eq!(seen.len(), K::ALL.len());
        // the wire values 0..=15 are the model kinds plus the two glass recipes
        let used: std::collections::HashSet<u32> = K::ALL.iter().map(|k| k.window_server_wire()).collect();
        let free: Vec<u32> = (0..=15).filter(|v| !used.contains(v)).collect();
        assert_eq!(free, vec![WINDOW_SERVER_GLASS_REGULAR, WINDOW_SERVER_GLASS_CLEAR]);
        assert_eq!(MaterialKind::from_wire(WINDOW_SERVER_GLASS_REGULAR), Some(MaterialKind::GlassRegular));
        assert_eq!(MaterialKind::from_wire(WINDOW_SERVER_GLASS_CLEAR), Some(MaterialKind::GlassClear));
        let req = MaterialRequest::new(K::Popover, bifrost_ui::Rect { x: 0.0, y: 0.0, w: 1.0, h: 1.0 }, 0.0);
        assert_eq!(MaterialKind::from_request(&req.clone().glass(GlassKind::Clear)), MaterialKind::GlassClear);
        assert_eq!(MaterialKind::from_request(&req), MaterialKind::Popover);
    }

    #[test]
    fn state_and_blending_wire_values_match_the_model() {
        use bifrost_ui::visual_effect::{BlendingMode as B, MaterialState as S};
        for (m, w) in [(S::FollowsWindowActiveState, MaterialState::FollowsWindow), (S::Active, MaterialState::Active), (S::Inactive, MaterialState::Inactive)] {
            assert_eq!(MaterialState::from_wire(m.window_server_wire()), Some(w));
        }
        assert_eq!(Blending::from_wire(B::BehindWindow.window_server_wire()), Some(Blending::BehindWindow));
        assert_eq!(Blending::from_wire(B::WithinWindow.window_server_wire()), Some(Blending::WithinWindow));
    }

    #[test]
    fn the_sidebar_is_the_thin_recipe() {
        let rc = recipe(MaterialKind::Sidebar);
        assert_eq!((rc.blur, rc.backdrop_scale), (30.0, 0.25));
        assert_eq!((rc.saturation_dark, rc.saturation_light), (1.35, 1.35));
        assert_eq!(rc.tone_dark, Tone::new(0.0, 0.6, [0.2, 0.21, 0.1, 0.15]));
        assert_eq!(rc.tone_light, Tone::new(0.12, 0.6, [0.725, 0.825, 0.76, 0.73]));
        assert_eq!(rc.tint_dark[3], 0.0, "the measured recipes carry no veil");
        assert_eq!(rc.plate_dark[3], 1.0, "the inactive plate is opaque");
    }

    /// The recipe table against the documented values (`docs/reference/materials-recipes.md`, section "Recipes per
    /// material kind"): blur, saturation Dark/Light, tone Dark/Light.
    #[test]
    fn the_recipe_table_matches_the_document() {
        use MaterialKind::*;
        let chrome = (Tone::new(-0.1, 0.75, [0.23, 0.52, 0.27, 0.255]), Tone::new(0.1, 0.75, [0.8, 0.9, 1.1, 0.825]));
        let regular = (Tone::new(0.0, 0.75, [0.16, 0.26, 0.1, 0.1]), Tone::new(0.1, 0.75, [0.9, 0.83, 0.925, 0.815]));
        let thick = (Tone::new(0.0, 0.88, [0.14, 0.16, 0.1, 0.03]), Tone::new(0.045, 0.88, [0.99, 0.95, 0.98, 0.905]));
        let thin = (Tone::new(0.0, 0.6, [0.2, 0.21, 0.1, 0.15]), Tone::new(0.12, 0.6, [0.725, 0.825, 0.76, 0.73]));
        let ultra = (Tone::new(0.0, 0.5, [0.24, 0.24, 0.3, 0.39]), Tone::new(0.12, 0.5, [0.45, 0.55, 0.65, 0.68]));
        let rows: [(MaterialKind, f32, (f32, f32), (Tone, Tone)); 11] = [
            (Titlebar, 22.5, (2.0, 1.1), chrome),
            (HeaderView, 22.5, (2.0, 1.1), chrome),
            (Menu, 30.0, (1.5, 1.5), regular),
            (Popover, 30.0, (1.5, 1.5), regular),
            (Sheet, 30.0, (1.5, 1.5), regular),
            (WindowBackground, 45.0, (1.5, 1.5), thick),
            (ContentBackground, 45.0, (1.5, 1.5), thick),
            (Sidebar, 30.0, (1.35, 1.35), thin),
            (UnderPageBackground, 30.0, (1.35, 1.35), thin),
            (UnderWindowBackground, 22.5, (1.1, 1.1), ultra),
            (HudWindow, 45.0, (1.5, 1.5), (thick.0, thick.0)),
        ];
        for (k, blur, sat, tone) in rows {
            let rc = recipe(k);
            assert_eq!(rc.blur, blur, "{k:?}");
            assert_eq!((rc.saturation_dark, rc.saturation_light), sat, "{k:?}");
            assert_eq!((rc.tone_dark, rc.tone_light), tone, "{k:?}");
            assert_eq!(rc.backdrop_scale, 0.25, "{k:?}");
        }
        // the always-dark kinds use the dark recipe in Light too
        for k in [HudWindow, ToolTip, FullScreenUi] {
            assert_eq!(recipe(k).tone_light, recipe(k).tone_dark, "{k:?}");
        }
        let dock = dock_recipe();
        assert_eq!((dock.saturation_dark, dock.saturation_light), (1.6, 1.8));
        assert_eq!(dock.tone_light, Tone::new(0.08, 0.5, [0.3, 0.5, 1.0, 0.77]));
    }

    /// The opaque plates are the recipe tone on the neutral backdrop, hand-computed (0..255).
    #[test]
    fn the_plates_match_the_hand_computed_tones() {
        let at = |rc: Rgba| (rc[0] * 255.0, rc[1] * 255.0, rc[2] * 255.0);
        let near = |got: f32, want: f32| (got - want).abs() < 0.2;
        // (kind, dark, light)
        for (k, d, l) in [
            (MaterialKind::Titlebar, 35.3, 255.0),
            (MaterialKind::Menu, 45.7, 242.6),
            (MaterialKind::WindowBackground, 36.9, 246.7),
            (MaterialKind::Sidebar, 45.5, 231.3),
            (MaterialKind::UnderWindowBackground, 50.2, 223.4),
        ] {
            let rc = recipe(k);
            let (dr, dg, db) = at(rc.plate_dark);
            let (lr, lg, lb) = at(rc.plate_light);
            assert!(near(dr, d) && near(dg, d) && near(db, d), "{k:?} dark {dr}");
            assert!(near(lr, l) && near(lg, l) && near(lb, l), "{k:?} light {lr}");
        }
        assert!(near(at(dock_recipe().plate_dark).0, 36.6) && near(at(dock_recipe().plate_light).0, 234.3));
    }

    #[test]
    fn regular_glass_is_the_decomposed_colour_matrix() {
        let rc = recipe(MaterialKind::GlassRegular);
        assert_eq!((rc.blur, rc.saturation_dark), (45.0, 1.5885));
        assert_eq!(rc.tint_dark, [0.6343, 0.6343, 0.6343, 0.3705]);
        // gain 1 - alpha, offset alpha * veil: 0.6295 and 0.235
        assert!((1.0 - rc.tint_dark[3] - 0.6295).abs() < 1e-4 && (rc.tint_dark[3] * rc.tint_dark[0] - 0.235).abs() < 1e-3);
    }

    #[test]
    fn a_resolved_glass_carries_the_appearance_tone() {
        let s = spec(MaterialKind::Titlebar);
        let Resolved::Glass { tone: d, saturation: sd, .. } = resolve(&s, true, true) else { panic!() };
        let Resolved::Glass { tone: l, saturation: sl, .. } = resolve(&s, true, false) else { panic!() };
        assert_eq!((sd, sl), (2.0, 1.1));
        assert_eq!((d.brightness, l.brightness), (-0.1, 0.1));
    }

    #[test]
    fn every_recipe_is_sane() {
        for v in 0..=15 {
            let rc = recipe(MaterialKind::from_wire(v).unwrap());
            assert!(rc.blur > 0.0 && rc.blur <= 64.0);
            assert!(rc.saturation_dark >= 1.0 && rc.saturation_dark <= 2.5);
            assert!(rc.saturation_light >= 1.0 && rc.saturation_light <= 2.5);
            assert_eq!(rc.plate_dark[3], 1.0);
            assert_eq!(rc.plate_light[3], 1.0);
            assert!(rc.tint_dark[3] <= 1.0 && rc.tint_light[3] <= 1.0);
        }
    }

    #[test]
    fn a_sidebar_in_an_inactive_window_is_the_opaque_plate() {
        let s = spec(MaterialKind::Sidebar);
        assert_eq!(resolve(&s, false, true), Resolved::Plate(recipe(MaterialKind::Sidebar).plate_dark));
        assert_eq!(resolve(&s, false, false), Resolved::Plate(recipe(MaterialKind::Sidebar).plate_light));
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
