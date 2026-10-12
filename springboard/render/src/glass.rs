//! GPU materials. When a GPU presenter owns the backdrop (the Wayland desktop on Mesa), glass
//! surfaces record their shape here instead of blurring on the CPU; the presenter draws the
//! blurred backdrop under them and the CPU only draws the tint and the content.
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering::Relaxed};
use std::sync::Mutex;

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub r: f32,
    pub kind: Kind,
    /// `NativeVisualEffectView.State`: `None` follows the window's active state, `Some(active)` fixes it.
    pub active: Option<bool>,
    /// The full request when the declarer has one (the shared `visual_effect::MaterialRequest`), for the window
    /// server; `None` for shell-side regions, which derive theirs from `kind`.
    pub detail: Option<Detail>,
}

/// What `Rect` loses of a shared material request: the window server's `bifrost_material_v1` values (plain data;
/// this crate does not depend on the UI model) and the four corner radii in the same units as `Rect::r`.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Detail {
    /// `bifrost_material_v1.kind` (`MaterialKind::window_server_wire`)
    pub kind: u32,
    /// within-window blending (`bifrost_material_v1.blending` 1)
    pub within_window: bool,
    pub emphasized: bool,
    /// top-left, top-right, bottom-right, bottom-left
    pub radii: [f32; 4],
}

/// What a glass shape is, for the presenter (`NativeVisualEffectView.Material` families it tells apart).
/// Chrome and menus are Liquid Glass with a specular rim; window backgrounds (sidebars, title bars,
/// content) are plain vibrancy without one, and turn opaque while their window is inactive.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Kind {
    /// The shell's own glass over the desktop: menu bar menus, the dock, panels, Search.
    #[default]
    Chrome,
    /// A window's source-list column (`.sidebar`).
    Sidebar,
    /// A window's title bar and toolbar band (`.titlebar`, `.headerView`).
    Titlebar,
    /// Window background materials (`.windowBackground`, `.contentBackground`, `.underWindowBackground`,
    /// `.underPageBackground`).
    Content,
    /// Floating materials inside a window (`.menu`, `.popover`, `.sheet`, `.hudWindow`, `.toolTip`,
    /// `.fullScreenUI`, `.selection`).
    Menu,
}

/// How the presenter shades one kind of glass.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Style {
    /// saturation of the backdrop (1.0 unchanged)
    pub saturation: f32,
    /// added to every channel after the luminance mapping
    pub brightness: f32,
    /// 0 skips the luminance mapping, 1 replaces the luma with the curve
    pub lum_amount: f32,
    /// the four control values of the luminance curve (cubic Bezier over the input luma)
    pub lum_values: [f32; 4],
    /// strength of the specular rim (0 none)
    pub rim: f32,
    /// the material follows its window's active state: opaque while the window is inactive
    pub follows_active: bool,
}

/// The measured filter of one material in one appearance (macOS 27 material recipes,
/// `docs/reference/materials-macos27.md`); the window server holds the same numbers in `wm/src/material_kind.rs`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Look {
    pub saturation: f32,
    pub brightness: f32,
    pub lum_amount: f32,
    pub lum_values: [f32; 4],
}

impl Look {
    /// The dark and light looks mixed by the scheme position (0 dark .. 1 light), so the cross-fade follows it.
    pub fn mix(dark: Look, light: Look, t: f32) -> Look {
        let m = |a: f32, b: f32| a + (b - a) * t;
        Look {
            saturation: m(dark.saturation, light.saturation),
            brightness: m(dark.brightness, light.brightness),
            lum_amount: m(dark.lum_amount, light.lum_amount),
            lum_values: [
                m(dark.lum_values[0], light.lum_values[0]),
                m(dark.lum_values[1], light.lum_values[1]),
                m(dark.lum_values[2], light.lum_values[2]),
                m(dark.lum_values[3], light.lum_values[3]),
            ],
        }
    }

    /// The filter on one backdrop pixel (straight RGB, 0..=1): saturation, luminance mapping, brightness; every stage
    /// clamped. The shaders (`gpu/wl.rs`, the window server's `compose.frag`) do the same arithmetic.
    pub fn apply(&self, rgb: [f32; 3]) -> [f32; 3] {
        let luma = |c: [f32; 3]| c[0] * 0.2126 + c[1] * 0.7152 + c[2] * 0.0722;
        let clamp = |c: [f32; 3]| [c[0].clamp(0.0, 1.0), c[1].clamp(0.0, 1.0), c[2].clamp(0.0, 1.0)];
        let l = luma(rgb);
        let mut c = clamp([l + (rgb[0] - l) * self.saturation, l + (rgb[1] - l) * self.saturation, l + (rgb[2] - l) * self.saturation]);
        if self.lum_amount != 0.0 {
            let m = luma(c).clamp(0.0, 1.0);
            let u = 1.0 - m;
            let v = self.lum_values;
            let curve = u * u * u * v[0] + 3.0 * u * u * m * v[1] + 3.0 * u * m * m * v[2] + m * m * m * v[3];
            let d = self.lum_amount * (curve - luma(c));
            c = clamp([c[0] + d, c[1] + d, c[2] + d]);
        }
        clamp([c[0] + self.brightness, c[1] + self.brightness, c[2] + self.brightness])
    }
}

const fn look(saturation: f32, brightness: f32, lum_amount: f32, lum_values: [f32; 4]) -> Look {
    Look { saturation, brightness, lum_amount, lum_values }
}

impl Kind {
    /// The measured (dark, light) looks of the kind: chrome and the shell's own glass take the title-bar recipe, menus
    /// the regular content recipe, sidebars the thin one, window backgrounds the thick one.
    pub fn looks(self) -> (Look, Look) {
        match self {
            Kind::Chrome | Kind::Titlebar => (look(2.0, -0.1, 0.75, [0.23, 0.52, 0.27, 0.255]), look(1.1, 0.1, 0.75, [0.8, 0.9, 1.1, 0.825])),
            Kind::Menu => (look(1.5, 0.0, 0.75, [0.16, 0.26, 0.1, 0.1]), look(1.5, 0.1, 0.75, [0.9, 0.83, 0.925, 0.815])),
            Kind::Sidebar => (look(1.35, 0.0, 0.6, [0.2, 0.21, 0.1, 0.15]), look(1.35, 0.12, 0.6, [0.725, 0.825, 0.76, 0.73])),
            Kind::Content => (look(1.5, 0.0, 0.88, [0.14, 0.16, 0.1, 0.03]), look(1.5, 0.045, 0.88, [0.99, 0.95, 0.98, 0.905])),
        }
    }

    pub fn style(self) -> Style {
        let (dark, light) = self.looks();
        let l = Look::mix(dark, light, crate::appearance::scheme());
        let (saturation, brightness, lum_amount, lum_values) = (l.saturation, l.brightness, l.lum_amount, l.lum_values);
        match self {
            Kind::Chrome | Kind::Menu => Style { saturation, brightness, lum_amount, lum_values, rim: 1.0, follows_active: self == Kind::Menu },
            Kind::Sidebar | Kind::Titlebar | Kind::Content => Style { saturation, brightness, lum_amount, lum_values, rim: 0.0, follows_active: true },
        }
    }

    /// `allowsVibrancy`: content on window materials and menus blends vibrantly (`vibrancy`); the shell's own
    /// chrome glass is Liquid Glass, whose content stays plain. False when Reduce Transparency is on.
    pub fn allows_vibrancy(self) -> bool {
        !crate::appearance::reduce_transparency() && !matches!(self, Kind::Chrome)
    }

    /// The opaque colour of the material while its window is inactive (premultiplied, current scheme).
    pub fn inactive(self) -> u32 {
        use crate::ui::{MATERIAL_SIDEBAR_SOLID, MATERIAL_TITLEBAR_SOLID, MENU_SOLID};
        match self {
            Kind::Sidebar => MATERIAL_SIDEBAR_SOLID.now(),
            Kind::Titlebar | Kind::Content => MATERIAL_TITLEBAR_SOLID.now(),
            Kind::Chrome | Kind::Menu => MENU_SOLID.now(),
        }
    }
}

/// The backdrop's saturation under the shell's own chrome glass, by scheme: the measured chrome recipe (dark 2.0, light
/// 1.1), following the scheme's cross-fade. Each material kind has its own value in `Kind::looks`.
pub fn saturation() -> f32 {
    let (d, l) = Kind::Chrome.looks();
    Look::mix(d, l, crate::appearance::scheme()).saturation
}

static ON: AtomicBool = AtomicBool::new(false);
static RECTS: Mutex<Vec<Rect>> = Mutex::new(Vec::new());
/// Liquid Glass clarity, 0 (clear) to 1 (tinted), as f32 bits; 0.5 by default.
static CLARITY: AtomicU32 = AtomicU32::new(0x3f00_0000);

/// The presenter turns GPU materials on once it is running.
pub fn enable(on: bool) {
    ON.store(on, Relaxed);
}

/// True when glass is drawn by the GPU presenter.
pub fn gpu() -> bool {
    ON.load(Relaxed)
}

/// Records a glass shape in frame pixels (deduplicated).
pub fn push(r: Rect) {
    if let Ok(mut v) = RECTS.lock() {
        if !v.contains(&r) {
            v.push(r);
        }
    }
}

/// The shapes recorded since the last call.
pub fn take() -> Vec<Rect> {
    RECTS.lock().map(|mut v| std::mem::take(&mut *v)).unwrap_or_default()
}

/// Updates the shapes the presenter keeps between frames. `damage` is the frame's redrawn areas as
/// (x0, y0, x1, y1). A stored shape that lies wholly inside redrawn pixels was drawn again this frame, so
/// it is dropped unless `fresh` records it (a closed menu's shape goes away); shapes outside the damage,
/// whose pixels were not redrawn, stay. `fresh` shapes are added once.
pub fn merge(stored: &mut Vec<Rect>, fresh: Vec<Rect>, damage: &[(f32, f32, f32, f32)]) {
    stored.retain(|r| !damage.iter().any(|&(x0, y0, x1, y1)| r.x >= x0 && r.y >= y0 && r.x + r.w <= x1 && r.y + r.h <= y1));
    for r in fresh {
        if !stored.contains(&r) {
            stored.push(r);
        }
    }
}


/// A per-surface transform applied by the presenter inside the drawable: the content rect is scaled
/// about its centre and shifted by (`dx`, `dy`) px, drawn at `alpha`, with the shadow scaled by `shadow`.
/// The drawable is fixed, so the content stays within its shadow margin (scale <= 1, shifts clamped).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fx {
    pub scale: f32,
    pub alpha: f32,
    pub dx: f32,
    pub dy: f32,
    pub shadow: f32,
}

impl Fx {
    pub const IDENTITY: Fx = Fx { scale: 1.0, alpha: 1.0, dx: 0.0, dy: 0.0, shadow: 1.0 };
    pub fn is_identity(&self) -> bool {
        *self == Fx::IDENTITY
    }
}

/// Settings > Appearance > Liquid Glass: 0 is clear, 1 is tinted.
pub fn set_clarity(v: f32) {
    CLARITY.store(v.clamp(0.0, 1.0).to_bits(), Relaxed);
}

pub fn clarity() -> f32 {
    f32::from_bits(CLARITY.load(Relaxed))
}

/// Scales a premultiplied tint's alpha by the clarity setting: clear halves it, tinted raises it by half.
pub fn tint(c: u32) -> u32 {
    let k = 0.5 + clarity();
    let a = ((c >> 24) as f32 * k).min(255.0) / ((c >> 24).max(1) as f32);
    let ch = |s: u32| (((c >> s) & 0xff) as f32 * a).min(255.0) as u32;
    ch(0) | ch(8) << 8 | ch(16) << 16 | ch(24) << 24
}

#[cfg(test)]
mod merge_tests {
    use super::*;

    fn r(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect { x, y, w, h, r: 4.0, ..Default::default() }
    }

    #[test]
    fn a_shape_inside_the_damage_that_is_not_recorded_again_is_dropped() {
        let menu = r(100.0, 40.0, 200.0, 100.0);
        let mut stored = vec![r(0.0, 0.0, 800.0, 30.0), menu];
        merge(&mut stored, vec![r(0.0, 0.0, 800.0, 30.0)], &[(0.0, 0.0, 800.0, 160.0)]);
        assert_eq!(stored, vec![r(0.0, 0.0, 800.0, 30.0)]);
    }

    #[test]
    fn a_shape_outside_the_damage_stays() {
        let dock = r(300.0, 700.0, 400.0, 60.0);
        let mut stored = vec![dock];
        merge(&mut stored, vec![], &[(0.0, 0.0, 800.0, 160.0)]);
        assert_eq!(stored, vec![dock]);
    }

    #[test]
    fn a_shape_only_partly_in_the_damage_stays() {
        let bar = r(0.0, 0.0, 800.0, 30.0);
        let mut stored = vec![bar];
        merge(&mut stored, vec![], &[(0.0, 10.0, 800.0, 20.0)]);
        assert_eq!(stored, vec![bar]);
    }

    #[test]
    fn window_materials_have_no_rim_and_follow_the_active_state() {
        for k in [Kind::Sidebar, Kind::Titlebar, Kind::Content] {
            let st = k.style();
            assert_eq!(st.rim, 0.0);
            assert!(st.follows_active);
        }
        assert!(Kind::Chrome.style().rim > 0.0 && !Kind::Chrome.style().follows_active);
        assert!(Kind::Menu.style().rim > 0.0 && Kind::Menu.style().follows_active);
        assert!(Kind::Sidebar.style().saturation >= 1.35);
        assert_eq!(Kind::Sidebar.inactive() >> 24, 255, "the inactive material is opaque");
    }

    #[test]
    fn the_looks_are_the_measured_recipes() {
        let (d, l) = Kind::Titlebar.looks();
        assert_eq!((d.saturation, l.saturation), (2.0, 1.1));
        assert_eq!((d.brightness, l.brightness), (-0.1, 0.1));
        assert_eq!(Kind::Chrome.looks(), Kind::Titlebar.looks());
        assert_eq!(Kind::Sidebar.looks().0.lum_values, [0.2, 0.21, 0.1, 0.15]);
        assert_eq!(Kind::Content.looks().1.lum_amount, 0.88);
    }

    #[test]
    fn the_look_filter_matches_hand_computed_pixels() {
        let (dark, light) = Kind::Titlebar.looks();
        // black under dark chrome: l = 0, curve(0) = 0.23, shift 0.75 * 0.23 = 0.1725, brightness -0.1 -> 0.0725
        let p = dark.apply([0.0; 3]);
        assert!(p.iter().all(|v| (v - 0.0725).abs() < 1e-4), "{p:?}");
        // mid grey under dark chrome: curve(0.5) = 0.356875, shift -0.10734375, brightness -0.1 -> 0.29265625
        let p = dark.apply([0.5; 3]);
        assert!(p.iter().all(|v| (v - 0.292_656).abs() < 1e-4), "{p:?}");
        // white under light chrome: curve(1) = 0.825, shift 0.75 * -0.175, brightness +0.1 -> 0.96875
        let p = light.apply([1.0; 3]);
        assert!(p.iter().all(|v| (v - 0.968_75).abs() < 1e-4), "{p:?}");
    }

    #[test]
    fn the_scheme_mix_is_linear_between_dark_and_light() {
        let (d, l) = Kind::Menu.looks();
        assert_eq!(Look::mix(d, l, 0.0), d);
        let end = Look::mix(d, l, 1.0);
        assert!((end.brightness - l.brightness).abs() < 1e-6 && (end.lum_values[1] - l.lum_values[1]).abs() < 1e-6);
        assert!((Look::mix(d, l, 0.5).brightness - 0.05).abs() < 1e-6);
    }

    #[test]
    fn a_recorded_shape_is_stored_once() {
        let a = r(1.0, 2.0, 30.0, 40.0);
        let mut stored = vec![];
        merge(&mut stored, vec![a, a], &[(0.0, 0.0, 100.0, 100.0)]);
        merge(&mut stored, vec![a], &[(0.0, 0.0, 100.0, 100.0)]);
        assert_eq!(stored, vec![a]);
    }
}
