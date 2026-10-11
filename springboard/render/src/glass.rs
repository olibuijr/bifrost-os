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
    /// strength of the specular rim (0 none)
    pub rim: f32,
    /// the material follows its window's active state: opaque while the window is inactive
    pub follows_active: bool,
}

impl Kind {
    pub fn style(self) -> Style {
        let saturation = saturation();
        match self {
            Kind::Chrome | Kind::Menu => Style { saturation, rim: 1.0, follows_active: self == Kind::Menu },
            Kind::Sidebar | Kind::Titlebar | Kind::Content => Style { saturation, rim: 0.0, follows_active: true },
        }
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

/// The backdrop's saturation under a material, by scheme. The system materials lift saturation by about
/// 1.8 (the platform's published image-effects sample uses `saturationDeltaFactor` 1.8 for its light and
/// dark blurs; a cross-platform toolkit fits 2.0 in light and 1.7 in dark to the simulator): light 1.8, dark 1.7,
/// following the scheme's cross-fade.
pub fn saturation() -> f32 {
    1.7 + 0.1 * crate::appearance::scheme()
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
        assert!(Kind::Sidebar.style().saturation >= 1.7);
        assert_eq!(Kind::Sidebar.inactive() >> 24, 255, "the inactive material is opaque");
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
