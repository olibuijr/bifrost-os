//! `MaterialView` model: the material catalogue, blending, state, emphasis, mask and interior background
//! style, and the per-view `MaterialRequest` the window server consumes. Renderer-independent: the shell and the SDK
//! (`bifrost_app::widgets`) share it. Tints live in `bifrost_render::ui` (`MATERIAL_*`), not here.
use crate::Rect;

/// `MaterialView.Material` (the current, non-deprecated cases).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MaterialKind {
    Sidebar,
    Titlebar,
    HeaderView,
    WindowBackground,
    ContentBackground,
    UnderWindowBackground,
    UnderPageBackground,
    Menu,
    Popover,
    Sheet,
    HudWindow,
    FullScreenUi,
    ToolTip,
    Selection,
}

impl MaterialKind {
    /// The whole catalogue.
    pub const ALL: [MaterialKind; 14] = [
        MaterialKind::Sidebar,
        MaterialKind::Titlebar,
        MaterialKind::HeaderView,
        MaterialKind::WindowBackground,
        MaterialKind::ContentBackground,
        MaterialKind::UnderWindowBackground,
        MaterialKind::UnderPageBackground,
        MaterialKind::Menu,
        MaterialKind::Popover,
        MaterialKind::Sheet,
        MaterialKind::HudWindow,
        MaterialKind::FullScreenUi,
        MaterialKind::ToolTip,
        MaterialKind::Selection,
    ];
    /// The kind a shell without the full catalogue understands: the sidebar, or the title bar for the rest.
    pub fn legacy(self) -> MaterialKind {
        match self {
            MaterialKind::Sidebar => MaterialKind::Sidebar,
            _ => MaterialKind::Titlebar,
        }
    }
    /// The `bifrost_material_v1.kind` value the window server (`bifrost-wm`) knows this kind by. The values are the
    /// protocol's (`wm/protocol/bifrost-material-v1.xml`), not the declaration order. The exhaustive match means a
    /// kind added here cannot compile until it has a wire value; `bifrost-wm` has a table test over `ALL`.
    pub fn window_server_wire(self) -> u32 {
        match self {
            MaterialKind::Titlebar => 0,
            MaterialKind::Menu => 1,
            MaterialKind::Popover => 2,
            MaterialKind::Sidebar => 3,
            MaterialKind::HeaderView => 4,
            MaterialKind::Sheet => 5,
            MaterialKind::WindowBackground => 6,
            MaterialKind::HudWindow => 7,
            MaterialKind::FullScreenUi => 8,
            MaterialKind::ToolTip => 9,
            MaterialKind::ContentBackground => 10,
            MaterialKind::UnderWindowBackground => 11,
            MaterialKind::UnderPageBackground => 12,
            MaterialKind::Selection => 15,
        }
    }
    /// Dark-by-nature materials (`.hudWindow`, `.fullScreenUI`): vibrant content on them is always light.
    pub fn is_dark_vibrant(self) -> bool {
        matches!(self, MaterialKind::HudWindow | MaterialKind::FullScreenUi)
    }
}

/// `bifrost_material_v1.kind` values of the two Liquid Glass recipes, which have no `MaterialKind` (see `GlassKind`).
pub const WINDOW_SERVER_GLASS_REGULAR: u32 = 13;
pub const WINDOW_SERVER_GLASS_CLEAR: u32 = 14;

/// `MaterialView.BlendingMode`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Hash)]
pub enum BlendingMode {
    /// The desktop and the windows behind the window.
    #[default]
    BehindWindow,
    /// The view's own window content under the rectangle.
    WithinWindow,
}

/// `MaterialView.State`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Hash)]
pub enum MaterialState {
    /// Looks active while the window is key or main, inactive (opaque plate) otherwise.
    #[default]
    FollowsWindowActiveState,
    Active,
    Inactive,
}

impl BlendingMode {
    /// The `bifrost_material_v1.blending` value.
    pub fn window_server_wire(self) -> u32 {
        match self { BlendingMode::BehindWindow => 0, BlendingMode::WithinWindow => 1 }
    }
}

impl MaterialState {
    /// The `bifrost_material_v1.state` value.
    pub fn window_server_wire(self) -> u32 {
        match self {
            MaterialState::FollowsWindowActiveState => 0,
            MaterialState::Active => 1,
            MaterialState::Inactive => 2,
        }
    }
    /// Whether the material is drawn active, given the window's active state.
    pub fn looks_active(self, window_active: bool) -> bool {
        match self {
            MaterialState::FollowsWindowActiveState => window_active,
            MaterialState::Active => true,
            MaterialState::Inactive => false,
        }
    }
}

/// `BackgroundStyle`, as reported by `interiorBackgroundStyle`: how content on the material should draw.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Hash)]
pub enum BackgroundStyle {
    #[default]
    Normal,
    /// Content sits on an accent or dark surface and draws light.
    Emphasized,
    Raised,
    Lowered,
}

/// Per-corner radii in points (top-left, top-right, bottom-right, bottom-left).
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Corners {
    pub tl: f32,
    pub tr: f32,
    pub br: f32,
    pub bl: f32,
}

impl Corners {
    pub fn uniform(r: f32) -> Corners {
        let r = r.max(0.0);
        Corners { tl: r, tr: r, br: r, bl: r }
    }
    /// Radii clamped so adjacent corners never overlap on a `w` by `h` rectangle.
    pub fn clamped(self, w: f32, h: f32) -> Corners {
        let c = |r: f32| r.max(0.0);
        let (tl, tr, br, bl) = (c(self.tl), c(self.tr), c(self.br), c(self.bl));
        let mut k = 1.0f32;
        for (sum, len) in [(tl + tr, w), (bl + br, w), (tl + bl, h), (tr + br, h)] {
            if sum > len && sum > 0.0 {
                k = k.min(len.max(0.0) / sum);
            }
        }
        Corners { tl: tl * k, tr: tr * k, br: br * k, bl: bl * k }
    }
    pub fn is_square(self) -> bool {
        self.tl <= 0.0 && self.tr <= 0.0 && self.br <= 0.0 && self.bl <= 0.0
    }
}

/// `maskImage`: which part of the view's rectangle the material covers.
#[derive(Clone, Debug, PartialEq, Default)]
pub enum Mask {
    /// The whole rectangle.
    #[default]
    None,
    /// A rounded rectangle (the usual mask), per-corner radii in points.
    RoundedRect(Corners),
    /// An arbitrary alpha mask, `w` by `h` coverage bytes stretched over the rectangle (`maskImage`'s alpha).
    Alpha { w: u32, h: u32, data: Vec<u8> },
}

impl Mask {
    /// An alpha mask, or `None` if `data` is not exactly `w * h` bytes.
    pub fn alpha(w: u32, h: u32, data: Vec<u8>) -> Option<Mask> {
        (w > 0 && h > 0 && data.len() == (w as usize) * (h as usize)).then_some(Mask::Alpha { w, h, data })
    }
    /// Coverage 0..=1 at the normalised position (`u`, `v`) in 0..1 of a `rect` (points). Rounded rectangles
    /// are evaluated exactly (hard edge); the window server anti-aliases.
    pub fn coverage(&self, rect: Rect, u: f32, v: f32) -> f32 {
        match self {
            Mask::None => 1.0,
            Mask::Alpha { w, h, data } => {
                let x = ((u.clamp(0.0, 1.0) * *w as f32) as u32).min(w - 1);
                let y = ((v.clamp(0.0, 1.0) * *h as f32) as u32).min(h - 1);
                data[(y * w + x) as usize] as f32 / 255.0
            }
            Mask::RoundedRect(c) => {
                let c = c.clamped(rect.w, rect.h);
                let (px, py) = (u * rect.w, v * rect.h);
                let corner = |cx: f32, cy: f32, r: f32| -> bool {
                    let (dx, dy) = (px - cx, py - cy);
                    dx * dx + dy * dy <= r * r
                };
                let inside = if px < c.tl && py < c.tl {
                    corner(c.tl, c.tl, c.tl)
                } else if px > rect.w - c.tr && py < c.tr {
                    corner(rect.w - c.tr, c.tr, c.tr)
                } else if px > rect.w - c.br && py > rect.h - c.br {
                    corner(rect.w - c.br, rect.h - c.br, c.br)
                } else if px < c.bl && py > rect.h - c.bl {
                    corner(c.bl, rect.h - c.bl, c.bl)
                } else {
                    true
                };
                if inside && (0.0..=1.0).contains(&u) && (0.0..=1.0).contains(&v) { 1.0 } else { 0.0 }
            }
        }
    }
}

/// declarative UI `Glass.regular` / `Glass.clear` (Liquid Glass), which the window server draws with its own recipes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GlassKind {
    Regular,
    Clear,
}

/// One `MaterialView` as the window server consumes it. `rect` is in window points.
#[derive(Clone, Debug, PartialEq)]
pub struct MaterialRequest {
    pub kind: MaterialKind,
    pub blending: BlendingMode,
    pub state: MaterialState,
    /// `isEmphasized`: the stronger look (an emphasized `.selection` is the full accent).
    pub emphasized: bool,
    pub rect: Rect,
    pub mask: Mask,
    /// Blur strength rank 0 (ultra thin) to 4 (ultra thick) from a declarative UI `Material` (`material_style`); `None` is
    /// the kind's own strength. Not on the shell/app wire yet.
    pub strength: Option<u8>,
    /// Set for Liquid Glass: the window server draws its glass recipe (`GlassRegular`/`GlassClear`) instead of the
    /// kind's. `kind` is then only the fallback look. Not on the shell/app wire yet.
    pub glass: Option<GlassKind>,
}

impl MaterialRequest {
    /// A behind-window request with a uniform corner radius, follows-active state, not emphasized.
    pub fn new(kind: MaterialKind, rect: Rect, radius: f32) -> MaterialRequest {
        MaterialRequest {
            kind,
            blending: BlendingMode::BehindWindow,
            state: MaterialState::FollowsWindowActiveState,
            emphasized: false,
            rect,
            strength: None,
            glass: None,
            mask: if radius > 0.0 { Mask::RoundedRect(Corners::uniform(radius)) } else { Mask::None },
        }
    }
    pub fn blending(mut self, b: BlendingMode) -> Self { self.blending = b; self }
    pub fn state(mut self, s: MaterialState) -> Self { self.state = s; self }
    pub fn emphasized(mut self, e: bool) -> Self { self.emphasized = e; self }
    pub fn mask(mut self, m: Mask) -> Self { self.mask = m; self }
    pub fn strength(mut self, level: u8) -> Self { self.strength = Some(level.min(4)); self }
    pub fn glass(mut self, g: GlassKind) -> Self { self.glass = Some(g); self }
    /// Whether the material is drawn active for a window in `window_active` state.
    pub fn looks_active(&self, window_active: bool) -> bool {
        self.state.looks_active(window_active)
    }
    /// The corner radii the window server rounds the rectangle with (square for alpha masks, which carry their own).
    pub fn corners(&self) -> Corners {
        match &self.mask {
            Mask::RoundedRect(c) => c.clamped(self.rect.w, self.rect.h),
            _ => Corners::default(),
        }
    }
    /// Only behind-window requests reach the window server; within-window ones are blurred by the app.
    pub fn for_window_server(&self) -> bool {
        self.blending == BlendingMode::BehindWindow
    }
    /// `interiorBackgroundStyle`: dark materials and an emphasized selection are emphasized; others are normal.
    pub fn interior_background_style(&self) -> BackgroundStyle {
        if self.kind.is_dark_vibrant() || (self.kind == MaterialKind::Selection && self.emphasized) {
            BackgroundStyle::Emphasized
        } else {
            BackgroundStyle::Normal
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const R: Rect = Rect { x: 0.0, y: 0.0, w: 100.0, h: 40.0 };

    #[test]
    fn catalogue_is_complete_and_unique() {
        assert_eq!(MaterialKind::ALL.len(), 14);
        for (i, a) in MaterialKind::ALL.iter().enumerate() {
            for b in &MaterialKind::ALL[i + 1..] {
                assert_ne!(a, b);
            }
        }
    }

    #[test]
    fn window_server_wire_values_are_unique_and_in_the_protocol_range() {
        let mut seen = std::collections::HashSet::new();
        for k in MaterialKind::ALL {
            let v = k.window_server_wire();
            assert!(v <= 15 && v != WINDOW_SERVER_GLASS_REGULAR && v != WINDOW_SERVER_GLASS_CLEAR, "{k:?}");
            assert!(seen.insert(v), "{k:?}");
        }
    }

    #[test]
    fn legacy_maps_to_sidebar_or_titlebar() {
        for k in MaterialKind::ALL {
            let l = k.legacy();
            assert!(l == MaterialKind::Sidebar || l == MaterialKind::Titlebar);
        }
        assert_eq!(MaterialKind::Sidebar.legacy(), MaterialKind::Sidebar);
    }

    #[test]
    fn state_resolution() {
        let s = MaterialState::FollowsWindowActiveState;
        assert!(s.looks_active(true) && !s.looks_active(false));
        assert!(MaterialState::Active.looks_active(false));
        assert!(!MaterialState::Inactive.looks_active(true));
        assert_eq!(MaterialState::default(), s);
    }

    #[test]
    fn blending_selects_window_server_requests() {
        let r = MaterialRequest::new(MaterialKind::Sidebar, R, 8.0);
        assert!(r.for_window_server());
        assert!(!r.clone().blending(BlendingMode::WithinWindow).for_window_server());
    }

    #[test]
    fn corners_clamp_and_mask() {
        let r = MaterialRequest::new(MaterialKind::Menu, R, 100.0);
        assert_eq!(r.corners().tl, 20.0);
        let m = Mask::RoundedRect(Corners::uniform(10.0));
        assert_eq!(m.coverage(R, 0.0, 0.0), 0.0);
        assert_eq!(m.coverage(R, 0.5, 0.5), 1.0);
        assert_eq!(MaterialRequest::new(MaterialKind::Menu, R, 0.0).mask, Mask::None);
    }

    #[test]
    fn alpha_mask_validates_and_samples() {
        assert!(Mask::alpha(2, 2, vec![0; 3]).is_none());
        let m = Mask::alpha(2, 1, vec![0, 255]).unwrap();
        assert_eq!(m.coverage(R, 0.1, 0.5), 0.0);
        assert_eq!(m.coverage(R, 0.9, 0.5), 1.0);
        assert_eq!(m.coverage(R, 1.0, 1.0), 1.0);
    }

    #[test]
    fn interior_style_and_emphasis() {
        let sel = MaterialRequest::new(MaterialKind::Selection, R, 4.0);
        assert_eq!(sel.interior_background_style(), BackgroundStyle::Normal);
        assert_eq!(sel.emphasized(true).interior_background_style(), BackgroundStyle::Emphasized);
        assert_eq!(MaterialRequest::new(MaterialKind::HudWindow, R, 0.0).interior_background_style(), BackgroundStyle::Emphasized);
    }
}
