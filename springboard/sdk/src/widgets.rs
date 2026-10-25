//! System-style drawing adapters for renderer-independent BifrOSt controls.
use crate::kit::ui::{Weight, FOLDER_EMBLEM, FOLDER_EMBLEM_HILITE, CONTROL_PRESSED, CAPSULE_BG, CAPSULE_EDGE, CAPSULE_SHADOW, DETAIL_BG, FIELD_BG, HAIRLINE_C, LABEL, LABEL_2, LABEL_3, MATERIAL_HUD, MATERIAL_HUD_SOLID, MATERIAL_MENU, MATERIAL_SHEET, MATERIAL_TOOLTIP, MATERIAL_HEADER, MATERIAL_CONTENT, MATERIAL_UNDER_WINDOW, MATERIAL_UNDER_PAGE, MATERIAL_FULLSCREEN, MATERIAL_SELECTION_EMPHASIZED, MATERIAL_SELECTION_INACTIVE, MATERIAL_POPOVER, MATERIAL_SIDEBAR, MATERIAL_SIDEBAR_SOLID, MATERIAL_TITLEBAR, MATERIAL_TITLEBAR_SOLID, ROW_PRESSED, SEGMENT_ON, SELECTION, SEPARATOR_C, SIDEBAR_BG, SIDEBAR_PLATE, TINT, WHITE};
use crate::kit::{Surface, Ui};
use crate::kit::rgba;
use bifrost_render::canvas::{Canvas, ImageRef, MaterialStyle, RectF};
#[cfg(test)]
use crate::kit::pack;
use crate::layout::{Layout, TITLE_GAP_PT};
use crate::glyphs::glyph;
use bifrost_ui::pathbar::{self, Segment};
use bifrost_ui::menu::Menu;
use bifrost_ui::nav::{Toolbar, ToolbarKind, ToolbarLayout};
use bifrost_ui::sidebar::{SidebarItem, SourceList};
use bifrost_ui::{Rect, TextField};
use bifrost_render::ui::{MAC_SUBHEAD_PT, MAC_BODY_PT, MAC_CALLOUT_PT, MAC_FOOTNOTE_PT, MAC_LARGE_TITLE_PT, SPACE_S, SPACE_XS};
use bifrost_ui::prefs_layout::m;

/// A translucent system material, as `NativeVisualEffectView.Material` on Golden Gate (the current cases). Drawn
/// with `visual_effect`: behind-window materials show the blurred desktop and the windows under the window,
/// within-window ones (and `material`) blur the app's own content behind the rectangle. With Reduce Transparency
/// every material is an opaque colour instead.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Material {
    /// Source-list columns: light tint, wide blur.
    Sidebar,
    /// Title bars and toolbars over scrolling content.
    Titlebar,
    /// Popovers: nearly opaque.
    Popover,
    /// Heads-up panels over media: dark and strongly blurred (`.hudWindow`).
    Hud,
    /// Menus.
    Menu,
    /// Sheets.
    Sheet,
    /// Section and table headers (`.headerView`).
    HeaderView,
    /// The window's own background.
    WindowBackground,
    /// Content backgrounds (`.contentBackground`), such as a table behind its rows.
    ContentBackground,
    /// The area behind window content (`.underWindowBackground`).
    UnderWindowBackground,
    /// The background behind document pages (`.underPageBackground`).
    UnderPageBackground,
    /// Full-screen modal interfaces (`.fullScreenUI`).
    FullScreenUi,
    /// Tool tips.
    ToolTip,
    /// The selection highlight (`.selection`), in the accent colour.
    Selection,
}

impl Material {
    /// Every material, in `NativeVisualEffectView.Material` order of this enum.
    pub const ALL: [Material; 14] = [
        Material::Sidebar,
        Material::Titlebar,
        Material::Popover,
        Material::Hud,
        Material::Menu,
        Material::Sheet,
        Material::HeaderView,
        Material::WindowBackground,
        Material::ContentBackground,
        Material::UnderWindowBackground,
        Material::UnderPageBackground,
        Material::FullScreenUi,
        Material::ToolTip,
        Material::Selection,
    ];

    /// Premultiplied tint drawn over the blurred backdrop, in the current scheme.
    pub fn tint(self) -> u32 {
        self.tint_for(false, true)
    }
    /// The tint for `emphasized` (`isEmphasized`) and `active` (the resolved `State`): an emphasized selection is the
    /// full accent, an inactive selection neutral grey; other materials do not change with either.
    pub fn tint_for(self, emphasized: bool, active: bool) -> u32 {
        use crate::kit::ui::{MATERIAL_SELECTION, MATERIAL_WINDOW};
        let c = match self {
            Material::Sidebar => MATERIAL_SIDEBAR,
            Material::Titlebar => MATERIAL_TITLEBAR,
            Material::HeaderView => MATERIAL_HEADER,
            Material::Popover => MATERIAL_POPOVER,
            Material::Menu => MATERIAL_MENU,
            Material::Sheet => MATERIAL_SHEET,
            Material::ToolTip => MATERIAL_TOOLTIP,
            Material::Hud => MATERIAL_HUD,
            Material::FullScreenUi => MATERIAL_FULLSCREEN,
            Material::WindowBackground => MATERIAL_WINDOW,
            Material::ContentBackground => MATERIAL_CONTENT,
            Material::UnderWindowBackground => MATERIAL_UNDER_WINDOW,
            Material::UnderPageBackground => MATERIAL_UNDER_PAGE,
            Material::Selection if !active => MATERIAL_SELECTION_INACTIVE,
            Material::Selection if emphasized => MATERIAL_SELECTION_EMPHASIZED,
            Material::Selection => MATERIAL_SELECTION,
        }
        .now();
        rgba((c & 255) as u8, (c >> 8 & 255) as u8, (c >> 16 & 255) as u8, (c >> 24) as u8)
    }
    /// The shared window-server model kind of this material.
    pub fn model_kind(self) -> bifrost_ui::visual_effect::MaterialKind {
        use bifrost_ui::visual_effect::MaterialKind as K;
        match self {
            Material::Sidebar => K::Sidebar,
            Material::Titlebar => K::Titlebar,
            Material::Popover => K::Popover,
            Material::Hud => K::HudWindow,
            Material::Menu => K::Menu,
            Material::Sheet => K::Sheet,
            Material::HeaderView => K::HeaderView,
            Material::WindowBackground => K::WindowBackground,
            Material::ContentBackground => K::ContentBackground,
            Material::UnderWindowBackground => K::UnderWindowBackground,
            Material::UnderPageBackground => K::UnderPageBackground,
            Material::FullScreenUi => K::FullScreenUi,
            Material::ToolTip => K::ToolTip,
            Material::Selection => K::Selection,
        }
    }
    /// The SDK material for a model kind.
    pub fn from_model(k: bifrost_ui::visual_effect::MaterialKind) -> Material {
        Material::ALL.into_iter().find(|m| m.model_kind() == k).unwrap_or(Material::Titlebar)
    }
    /// The opaque colour used when Reduce Transparency is on, in the current scheme.
    pub fn solid(self) -> u32 {
        match self {
            Material::Sidebar => MATERIAL_SIDEBAR_SOLID.now(),
            Material::Hud | Material::FullScreenUi => MATERIAL_HUD_SOLID.now(),
            Material::Selection => SELECTION.now(),
            _ => MATERIAL_TITLEBAR_SOLID.now(),
        }
    }
    /// Blur radius in points (within-window blending).
    pub fn blur_pt(self) -> f32 {
        match self {
            Material::Popover | Material::Menu | Material::Sheet | Material::ToolTip | Material::Selection => 6.0,
            Material::Hud | Material::FullScreenUi => 16.0,
            _ => 10.0,
        }
    }
    /// The kind the shell's presenter draws behind the window (the shared model kind, which is also the wire kind).
    pub fn kind(self) -> crate::proto::MaterialKind {
        self.model_kind()
    }
}

/// `NativeVisualEffectView.BlendingMode`: what the material blurs. `BehindWindow` is the desktop and the windows
/// behind this window (the shell's GPU presenter blurs them; the opaque colour where the shell offers no vibrancy);
/// `WithinWindow` is the app's own content already drawn under the rectangle. The shared model type.
pub use bifrost_ui::visual_effect::BlendingMode as Blending;

/// `NativeVisualEffectView.State`: an inactive window's behind-window materials are opaque unless the state says
/// otherwise.
pub use crate::proto::MaterialState as State;

thread_local! {
    static VIBRANCY: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static ALL_MATERIALS: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static FRAME_MATERIALS: std::cell::RefCell<Vec<crate::proto::Material>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// Window vibrancy (`NativeVisualEffectView` blending behind the window): the shell presents on the GPU and blurs the
/// desktop behind the regions the app declares (`appearance::VIBRANCY`, cleared by Reduce Transparency). The runtime
/// sets it from `ToApp::Appearance`; off by default, so the CPU/SHM path keeps the opaque colours.
pub fn vibrancy() -> bool {
    VIBRANCY.with(|v| v.get())
}

/// Called by the runtime when the shell's appearance bits arrive. `all_materials` is `appearance::MATERIALS`:
/// the shell decodes every material kind and state (older shells get `MaterialKind::legacy` kinds).
pub fn set_vibrancy(on: bool, all_materials: bool) {
    VIBRANCY.with(|v| v.set(on));
    ALL_MATERIALS.with(|v| v.set(all_materials));
}

/// Paints a behind-window material over `bounds` (points) with corner `radius` (points) and records it for the
/// shell: the pixels inside the rounded rectangle are written (not blended) as the premultiplied translucent tint,
/// so the shell's blur shows through them; anti-aliased corner pixels mix the tint with what was drawn there.
/// Returns false (and draws nothing) when vibrancy is off; the caller then paints its opaque colour.
fn window_material(s: &mut dyn Canvas, ui: &Ui, bounds: Rect, radius: f32, material: Material, state: State) -> bool {
    let req = bifrost_ui::visual_effect::MaterialRequest::new(material.model_kind(), bounds, radius).state(state);
    window_material_request(s, ui, &req, radius, material.tint())
}

/// `window_material` for a full shared request: declares kind, state, blending, emphasis and per-corner radii to the
/// shell. `draw_radius` (points) is what the canvas writes the tint with (the canvas has one radius).
fn window_material_request(s: &mut dyn Canvas, ui: &Ui, req: &bifrost_ui::visual_effect::MaterialRequest, draw_radius: f32, tint: u32) -> bool {
    if !vibrancy() || crate::kit::appearance::reduce_transparency() {
        return false;
    }
    let bounds = req.rect;
    let (x, y, w, h, r) = (ui.pt(bounds.x), ui.pt(bounds.y), ui.pt(bounds.w), ui.pt(bounds.h), ui.pt(draw_radius));
    // The translucent tint is written, replacing what is drawn there with its alpha (the window's own pixels are
    // transparent under a behind-window material), so the window server's blur shows through it on every path.
    s.write_rrect(RectF::new(x, y, w, h), r, tint);
    let mut m = crate::proto::Material::from_request(req);
    if !ALL_MATERIALS.with(|v| v.get()) {
        // a shell without `appearance::MATERIALS` decodes only the legacy kinds and the default state, and uniform radii
        m = crate::proto::Material::new(m.kind.legacy(), bounds, draw_radius);
    }
    FRAME_MATERIALS.with(|f| {
        let mut f = f.borrow_mut();
        if f.len() < crate::proto::MAX_MATERIALS {
            f.push(m);
        }
    });
    true
}

/// Vibrant label levels: `labelColor` to `quaternaryLabelColor` drawn on a vibrant material (`NativeAppearance`
/// vibrant light and dark). The text is blended into the material, not painted over it: in Dark it adds light
/// (plus-lighter), in Light it takes light away (plus-darker), and the material's alpha is left alone, so the blurred
/// backdrop shows through the glyphs and tints them, as on the reference desktop. Strengths are the label colours'
/// alphas (0.85, 0.55 / 0.50, 0.25 / 0.26, 0.10 in Dark / Light).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Vibrant {
    Label,
    Secondary,
    Tertiary,
    Quaternary,
}

impl Vibrant {
    /// How much light the text adds (Dark) or removes (Light), 0..1, at the current scheme position.
    pub fn strength(self) -> f32 {
        let (dark, light) = match self {
            Vibrant::Label => (0.85, 0.85),
            Vibrant::Secondary => (0.55, 0.50),
            Vibrant::Tertiary => (0.25, 0.26),
            Vibrant::Quaternary => (0.10, 0.10),
        };
        dark + (light - dark) * crate::kit::appearance::scheme()
    }
}

/// True while text on a behind-window material should blend vibrantly (`vibrancy()` and no Reduce Transparency).
pub fn vibrant_text() -> bool {
    vibrancy() && !crate::kit::appearance::reduce_transparency()
}

/// Blends a text image (drawn in `WHITE`; its alpha is the glyph coverage) into `s` at (`x`, `y`) px as a vibrant
/// label: plus-lighter in Dark, plus-darker in Light (see `Vibrant`). Alpha is unchanged, so on a translucent
/// material the compositor's premultiplied blend gives backdrop + label (Dark) or backdrop + label - 1 (Light).
/// Pixel form of `blit_vibrant` for code that holds the `Surface` itself (the exact plus-lighter / plus-darker blend).
pub fn blit_vibrant_px(s: &mut Surface, img: &crate::kit::Image, x: i32, y: i32, level: Vibrant) {
    let k = level.strength();
    let light = crate::kit::appearance::scheme() >= 0.5;
    for row in 0..img.h as i32 {
        let py = y + row;
        if py < s.y0 as i32 || py >= s.h as i32 {
            continue;
        }
        let base = (py as usize - s.y0) * s.stride;
        for col in 0..img.w as i32 {
            let px = x + col;
            if px < 0 || px >= s.w as i32 {
                continue;
            }
            let cov = (img.px[row as usize * img.w + col as usize] >> 24) as f32 / 255.0;
            if cov <= 0.0 {
                continue;
            }
            let d = &mut s.px[base + px as usize];
            *d = vibrant_px(*d, cov * k, light);
        }
    }
}

/// `blit_vibrant` on a `Canvas`. The canvas vocabulary has no plus-lighter or plus-darker blend, so the coverage of the
/// text is blended toward white (Dark) or black (Light) at the level's strength, through `image_tinted_keep_alpha`: the
/// RGB takes the light or dark and the material's alpha under the glyph is kept. The exact additive blend stays in
/// `blit_vibrant_px` (see g2-kit-canvas.md).
pub fn blit_vibrant(c: &mut dyn Canvas, img: &crate::kit::Image, x: i32, y: i32, level: Vibrant) {
    let a = (level.strength().clamp(0.0, 1.0) * 255.0).round() as u8;
    let tint = if crate::kit::appearance::scheme() >= 0.5 { rgba(0, 0, 0, a) } else { rgba(255, 255, 255, a) };
    let dst = RectF::new(x as f32, y as f32, img.w as f32, img.h as f32);
    c.image_tinted_keep_alpha(ImageRef { id: image_id(img), image: img }, dst, tint);
}

/// The stable id of a rasterised image for the GPU cache: a hash of its size and pixels.
pub fn image_id(img: &crate::kit::Image) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (img.w, img.h).hash(&mut h);
    img.px.hash(&mut h);
    h.finish()
}

/// Draws `img` at (`x`, `y`) px unscaled and unblended: the canvas form of `Surface::blit` with full tint and alpha.
pub fn blit(c: &mut dyn Canvas, img: &crate::kit::Image, x: i32, y: i32) {
    let dst = RectF::new(x as f32, y as f32, img.w as f32, img.h as f32);
    c.image(ImageRef { id: image_id(img), image: img }, dst, 1.0);
}

/// Draws the coverage of a white text image at (`x`, `y`) px in `color` (`Canvas::image_tinted`).
pub fn image_tinted_at(c: &mut dyn Canvas, img: &crate::kit::Image, x: i32, y: i32, color: u32) {
    let dst = RectF::new(x as f32, y as f32, img.w as f32, img.h as f32);
    c.image_tinted(ImageRef { id: image_id(img), image: img }, dst, color);
}

/// One premultiplied pixel with `amount` (0..1) of light added (Dark: plus-lighter) or removed (Light:
/// plus-darker); alpha is kept.
fn vibrant_px(d: u32, amount: f32, light: bool) -> u32 {
    let a = (amount * 255.0).round() as i32;
    let ch = |sh: u32| -> u32 {
        let c = ((d >> sh) & 255) as i32;
        ((if light { c - a } else { c + a }).clamp(0, 255) as u32) << sh
    };
    ch(0) | ch(8) | ch(16) | (d & 0xff00_0000)
}

/// Draws `text` as a sidebar label at `x` and the top `y(height)` px: vibrantly (`blit_vibrant`) when `vibrant`,
/// else in `plain`. Returns the image size.
#[allow(clippy::too_many_arguments)]
fn label_text(s: &mut dyn Canvas, ui: &Ui, text: &str, pt: f32, weight: Weight, x: i32, y: impl Fn(i32) -> i32, vibrant: Option<Vibrant>, plain: u32) -> (usize, usize) {
    let img = ui.text(text, pt, weight, if vibrant.is_some() { WHITE } else { plain });
    let top = y(img.h as i32);
    match vibrant {
        Some(v) => blit_vibrant(s, &img, x, top, v),
        None => blit(s, &img, x, top),
    }
    (img.w, img.h)
}

/// The behind-window materials drawn since the last call (the runtime sends them to the shell when they change).
pub fn take_frame_materials() -> Vec<crate::proto::Material> {
    FRAME_MATERIALS.with(|m| std::mem::take(&mut *m.borrow_mut()))
}

/// Draws `material` over what is already on `s` inside `bounds` (logical points) with corner `radius` (points),
/// blending within the window (`visual_effect` with `Blending::WithinWindow`).
pub fn material(s: &mut dyn Canvas, ui: &Ui, bounds: Rect, radius: f32, material: Material) {
    let r = RectF::new(ui.pt(bounds.x), ui.pt(bounds.y), ui.pt(bounds.w), ui.pt(bounds.h));
    if crate::kit::appearance::reduce_transparency() {
        // Reduce Transparency: the opaque colour, as `Surface::glass_rrect` does
        s.fill_rrect(r, ui.pt(radius), material.solid());
        return;
    }
    s.material(r, ui.pt(radius), MaterialStyle { blur: ui.pt(material.blur_pt()), tint: material.tint(), saturation: 1.0 });
}

/// `NativeVisualEffectView`: draws `material` in `bounds` (points) with corner `radius` (points). Behind-window
/// blending writes the translucent tint and declares the region to the shell, which blurs the desktop and the
/// windows under this one there; `state` decides whether it turns opaque while the window is inactive. Where the
/// shell offers no vibrancy (or Reduce Transparency is on) it draws the material's opaque colour. Within-window
/// blending blurs the app's own pixels (`material`). Draw the content under a within-window material first.
pub fn visual_effect(s: &mut dyn Canvas, ui: &Ui, bounds: Rect, radius: f32, mat: Material, blending: Blending, state: State) {
    match blending {
        Blending::WithinWindow => material(s, ui, bounds, radius, mat),
        Blending::BehindWindow => {
            if !window_material(s, ui, bounds, radius, mat, state) {
                let Rect { x, y, w, h } = bounds;
                s.fill_rrect(RectF::new(ui.pt(x), ui.pt(y), ui.pt(w), ui.pt(h)), ui.pt(radius), mat.solid());
            }
        }
    }
}

/// `NativeVisualEffectView` from a shared `MaterialRequest` (kind, blending, state, emphasis, mask). `window_active` is
/// the window's key state, which `State::FollowsWindowActiveState` follows. Per-corner radii draw as the largest
/// corner and an alpha mask as its bounding rounded rectangle until the canvas and wire carry them (see
/// docs/bifrost-ui.md, "Window vibrancy"); emphasis changes the `.selection` tint.
pub fn visual_effect_request(s: &mut dyn Canvas, ui: &Ui, req: &bifrost_ui::visual_effect::MaterialRequest, window_active: bool) {
    let mat = Material::from_model(req.kind);
    let c = req.corners();
    let radius = c.tl.max(c.tr).max(c.br).max(c.bl);
    if req.for_window_server() {
        let tint = if mat == Material::Selection { mat.tint_for(req.emphasized, req.looks_active(window_active)) } else { mat.tint() };
        if window_material_request(s, ui, req, radius, tint) {
            return;
        }
        let b = req.rect;
        s.fill_rrect(RectF::new(ui.pt(b.x), ui.pt(b.y), ui.pt(b.w), ui.pt(b.h)), ui.pt(radius), mat.solid());
        return;
    }
    material(s, ui, req.rect, radius, mat);
}

/// A root screen's large title (the native toolkit/HIG: large title, bold) on the leading edge of the readable column.
/// `y` is the top in points; returns the y (points) where content below it starts.
pub fn large_title(s: &mut dyn Canvas, ui: &Ui, l: &Layout, text: &str, y: f32) -> f32 {
    let (x, _) = l.readable();
    let img = ui.text(text, MAC_LARGE_TITLE_PT, Weight::Bold, LABEL.now());
    blit(s, &img, ui.pt(x) as i32, ui.pt(y) as i32);
    y + MAC_LARGE_TITLE_PT * 1.25 + TITLE_GAP_PT
}

/// A group header or footer: footnote size, secondary grey, on the leading edge of the readable column.
/// Returns the y (points) below it.
pub fn section_label(s: &mut dyn Canvas, ui: &Ui, l: &Layout, text: &str, y: f32) -> f32 {
    let (x, _) = l.readable();
    let img = ui.text(text, MAC_FOOTNOTE_PT, Weight::Regular, LABEL_2.now());
    blit(s, &img, ui.pt(x) as i32, ui.pt(y) as i32);
    y + MAC_FOOTNOTE_PT * 1.6
}

/// Show Borders (Golden Gate Accessibility > Display, bifrost Button Shapes): when the setting is on, a control gets a visible
/// one-point outline so its edges can be seen without relying on a fill. `bounds` and `radius` are in points.
/// Controls call this after drawing their fill; it draws nothing when the setting is off.
pub fn outline_if_borders(s: &mut dyn Canvas, ui: &Ui, bounds: Rect, radius: f32) {
    if !crate::kit::appearance::button_shapes() {
        return;
    }
    let Rect { x, y, w, h } = bounds;
    s.stroke_rrect(RectF::new(ui.pt(x), ui.pt(y), ui.pt(w), ui.pt(h)), ui.pt(radius), ui.pt(1.0).max(1.0), HAIRLINE_C.now());
}

/// Height of a toolbar capsule and the side of a glyph button, in points (Golden Gate toolbar buttons).
pub const CAPSULE_PT: f32 = 36.0;
/// Side of a glyph inside a toolbar button.
const TOOLBAR_GLYPH_PT: f32 = 20.0;
/// Space between toolbar groups.
pub const TOOLBAR_GAP_PT: f32 = 10.0;

/// The unified toolbar band (`NativeToolbar` in a full-size window): the content colour with a hairline at the bottom,
/// or the translucent `.titlebar` material over the blurred desktop when the shell offers vibrancy.
/// Draw it first, then `toolbar` over it. `bounds` in points.
pub fn toolbar_band(s: &mut dyn Canvas, ui: &Ui, bounds: Rect) {
    let Rect { x, y, w, h } = bounds;
    if !window_material(s, ui, bounds, 0.0, Material::Titlebar, State::default()) {
        s.fill_rect(RectF::new((ui.pt(x) as usize) as f32, (ui.pt(y) as usize) as f32, (ui.pt(w).ceil() as usize) as f32, (ui.pt(h).ceil() as usize) as f32), DETAIL_BG.now());
    }
    s.fill_rect(RectF::new((ui.pt(x) as usize) as f32, ((ui.pt(y + h) as usize).saturating_sub(1)) as f32, (ui.pt(w).ceil() as usize) as f32, (1) as f32), SEPARATOR_C.now());
}

/// A Golden Gate glass capsule (`bounds` in points): a soft ring, the plate and a one-pixel edge.
pub fn capsule(s: &mut dyn Canvas, ui: &Ui, bounds: Rect) {
    let Rect { x, y, w, h } = bounds;
    let r = h / 2.0;
    let o = ui.pt(1.0).max(1.0);
    s.fill_rrect(RectF::new(ui.pt(x) - o, ui.pt(y) - o + o * 0.5, ui.pt(w) + 2.0 * o, ui.pt(h) + 2.0 * o), ui.pt(r) + o, CAPSULE_SHADOW.now());
    s.fill_rrect(RectF::new(ui.pt(x), ui.pt(y), ui.pt(w), ui.pt(h)), ui.pt(r), CAPSULE_BG.now());
    s.stroke_rrect(RectF::new(ui.pt(x), ui.pt(y), ui.pt(w), ui.pt(h)), ui.pt(r), o, CAPSULE_EDGE.now());
}

/// Width a pull-down toolbar item (`ToolbarItem::menu`) adds for its chevron, in points.
pub const TOOLBAR_MENU_CHEVRON_PT: f32 = 14.0;

/// The x, in points, where the first toolbar item starts in a full-size-content window: right of the traffic lights the
/// shell draws over the window's top left, plus the standard toolbar gap. `toolbar_layout` (and so `toolbar`) applies
/// it to any bar that starts further left, so an app need not compute it; call this when it positions other controls
/// (a title, a search field) in the same row.
pub fn toolbar_lead_pt() -> f32 {
    crate::layout::traffic_lights_trailing() + TOOLBAR_GAP_PT
}

/// `bounds` (points) moved clear of the traffic lights: a toolbar row that begins left of `toolbar_lead_pt` and meets the
/// lights' rows starts at `toolbar_lead_pt` instead and loses that much width. A bar below or above the lights' band,
/// or one that already starts clear of them (a content toolbar beside a sidebar), is returned unchanged, as is every bar
/// of a window that has no lights over its content (`layout::traffic_lights_over_content`).
pub fn clear_of_traffic_lights(bounds: Rect) -> Rect {
    // no lights over the content (a classic title-bar window, a tablet): nothing to clear
    if !crate::layout::traffic_lights_over_content() {
        return bounds;
    }
    let lead = toolbar_lead_pt();
    let (top, bottom) = crate::layout::traffic_lights_band();
    if bounds.x >= lead || bounds.y >= bottom || bounds.y + bounds.h <= top {
        return bounds;
    }
    Rect { x: lead, w: (bounds.w - (lead - bounds.x)).max(0.0), ..bounds }
}

/// `clear_of_traffic_lights` for bounds relative to a pane whose left edge is `origin_x` points from the window's.
pub fn clear_of_traffic_lights_in(bounds: Rect, origin_x: f32) -> Rect {
    let moved = clear_of_traffic_lights(Rect { x: bounds.x + origin_x, ..bounds });
    Rect { x: moved.x - origin_x, ..moved }
}

/// Where `toolbar` puts each item in `bounds` (points), without drawing: the NativeToolbar item frames. Uses exactly the
/// widths `toolbar` draws with (glyph buttons `CAPSULE_PT`, titled buttons their label plus padding, pull-down items
/// a chevron more), so apps hit-test with `ToolbarLayout::hit` and anchor menus with `menu_anchor`.
pub fn toolbar_layout(ui: &Ui, bounds: Rect, bar: &Toolbar) -> ToolbarLayout {
    toolbar_layout_in(ui, bounds, bar, 0.0)
}

/// `toolbar_layout` for a bar in a pane that draws in its own coordinates: `bounds` are relative to the pane, whose left
/// edge is `origin_x` points from the window's left edge. The traffic-light inset is judged in window coordinates, so a
/// pane to the right of a sidebar (`origin_x` at its edge) is left where it is.
pub fn toolbar_layout_in(ui: &Ui, bounds: Rect, bar: &Toolbar, origin_x: f32) -> ToolbarLayout {
    // a bar that starts at the window's left edge begins right of the traffic lights (`clear_of_traffic_lights`)
    let bounds = clear_of_traffic_lights_in(bounds, origin_x);
    let h = CAPSULE_PT.min(bounds.h);
    let inner = Rect { x: bounds.x, y: bounds.y + (bounds.h - h) / 2.0, w: bounds.w, h };
    let width = |it: &bifrost_ui::nav::ToolbarItem| {
        let base = if it.symbol.is_some() { CAPSULE_PT } else { (ui.measure(&it.label, MAC_BODY_PT, Weight::Regular) / ui.pt(1.0) + 2.0 * SPACE_S * 1.5).max(CAPSULE_PT) };
        if it.menu { base + TOOLBAR_MENU_CHEVRON_PT } else { base }
    };
    bar.layout_with(inner, width, CAPSULE_PT, TOOLBAR_GAP_PT)
}

/// The standard window toolbar (the native toolkit `NativeToolbar`; Golden Gate's unified bar). Lays the items out in `bounds`
/// (points) and draws the buttons: glyph buttons (`ToolbarItem::icon`) in light glass capsules, one capsule per
/// group (`in_group`) with hairline dividers, the selected segment of a group on a grey plate, titled buttons as
/// their own capsule. `Custom` slots are only laid out; the app draws them at the returned rectangles. Buttons that
/// do not fit move behind a `»` button. The band behind the bar is `toolbar_band`. `pressed` is the id being pressed.
pub fn toolbar(s: &mut dyn Canvas, ui: &Ui, bounds: Rect, bar: &Toolbar, pressed: Option<u32>) -> ToolbarLayout {
    toolbar_in(s, ui, bounds, bar, pressed, 0.0)
}

/// `toolbar` for a bar in a pane that draws in its own coordinates (`toolbar_layout_in`): `origin_x` is the pane's left
/// edge in points from the window's left edge.
pub fn toolbar_in(s: &mut dyn Canvas, ui: &Ui, bounds: Rect, bar: &Toolbar, pressed: Option<u32>, origin_x: f32) -> ToolbarLayout {
    // the buttons are the bar's children in the accessibility tree (`ax::begin_group`)
    bifrost_ui::ax::begin_group(bifrost_ui::Role::List, "Toolbar", bounds);
    let placed = toolbar_layout_in(ui, bounds, bar, origin_x);
    for (_, r) in &placed.groups {
        capsule(s, ui, *r);
        outline_if_borders(s, ui, *r, r.h / 2.0);
    }
    for (k, (id, r)) in placed.visible.iter().enumerate() {
        let Some(item) = bar.items.iter().find(|i| i.id == *id && i.kind == ToolbarKind::Button) else { continue };
        bifrost_ui::ax::record(bifrost_ui::Role::Button, &item.label, if item.selected { "selected" } else { "" }, *r, item.enabled);
        if item.group == 0 {
            capsule(s, ui, *r);
        } else if k > 0 && bar.items.iter().find(|i| i.id == placed.visible[k - 1].0).is_some_and(|p| p.group == item.group) {
            // a hairline between neighbours of one group
            let inset = r.h * 0.25;
            s.fill_rect(RectF::new((ui.pt(r.x) as usize) as f32, (ui.pt(r.y + inset) as usize) as f32, (ui.pt(1.0).max(1.0) as usize) as f32, (ui.pt(r.h - 2.0 * inset) as usize) as f32), SEPARATOR_C.now());
        }
        let plate = Rect { x: r.x + 3.0, y: r.y + 3.0, w: r.w - 6.0, h: r.h - 6.0 };
        let fill = if pressed == Some(*id) && item.enabled { Some(ROW_PRESSED.now()) } else if item.selected { Some(SEGMENT_ON.now()) } else { None };
        if let Some(fill) = fill {
            s.fill_rrect(RectF::new(ui.pt(plate.x), ui.pt(plate.y), ui.pt(plate.w), ui.pt(plate.h)), ui.pt(plate.h / 2.0), fill);
        }
        let color = if item.enabled { LABEL.now() } else { LABEL_3.now() };
        // a pull-down item keeps its chevron in the trailing TOOLBAR_MENU_CHEVRON_PT; the face is centred in the rest
        let r = &if item.menu {
            let c = 7.0;
            let cx = r.right() - TOOLBAR_MENU_CHEVRON_PT - 2.0 + (TOOLBAR_MENU_CHEVRON_PT - c) / 2.0;
            glyph(s, ui, "chevron.down", Rect { x: cx, y: r.y + (r.h - c) / 2.0, w: c, h: c }, color);
            Rect { w: r.w - TOOLBAR_MENU_CHEVRON_PT + 4.0, ..*r }
        } else {
            *r
        };
        let drew = item.symbol.as_deref().is_some_and(|name| {
            let g = TOOLBAR_GLYPH_PT;
            glyph(s, ui, name, Rect { x: r.x + (r.w - g) / 2.0, y: r.y + (r.h - g) / 2.0, w: g, h: g }, color)
        });
        if !drew {
            let img = ui.text(&ui.fit(&item.label, MAC_BODY_PT, ui.pt(r.w - 2.0 * SPACE_S)), MAC_BODY_PT, Weight::Regular, color);
            blit(s, &img, (ui.pt(r.x + r.w / 2.0) as i32) - img.w as i32 / 2, (ui.pt(r.y + r.h / 2.0) as i32) - img.h as i32 / 2);
        }
    }
    if let Some(r) = placed.overflow_button {
        capsule(s, ui, r);
        let img = ui.text("\u{bb}", MAC_BODY_PT, Weight::Regular, LABEL.now());
        blit(s, &img, (ui.pt(r.x + r.w / 2.0) as i32) - img.w as i32 / 2, (ui.pt(r.y + r.h / 2.0) as i32) - img.h as i32 / 2);
    }
    bifrost_ui::ax::end_group();
    placed
}

/// Side of a sidebar row glyph, in points.
const SIDEBAR_GLYPH_PT: f32 = 20.0;
/// The row glyph's side at Settings > Appearance > Sidebar Icon Size (the row height follows in `SourceList`).
fn sidebar_glyph_pt() -> f32 {
    bifrost_ui::sidebar::scaled_row(SIDEBAR_GLYPH_PT, crate::kit::ui::sidebar_icon_scale())
}
/// Diameter of a sidebar monogram avatar, in points (`SidebarItem::with_avatar`).
const SIDEBAR_AVATAR_PT: f32 = 24.0;
/// Diameter of a sidebar tag dot, in points.
const SIDEBAR_DOT_PT: f32 = 12.0;

/// A source-list sidebar (Golden Gate: the shaded column runs the full height of the window, edge to edge; with
/// vibrancy it is the translucent `.sidebar` material over the blurred desktop). Draws the
/// whole `bounds` in the sidebar colour, then the visible rows scrolled by `scroll` points: section headers in small
/// grey semibold text, rows with an accent glyph (`with_symbol`), the selection as a neutral plate under a bold
/// label (Files, Notes, Reminders), counts on the right. `pressed` is the row id being pressed.
pub fn source_list(s: &mut dyn Canvas, ui: &Ui, bounds: Rect, list: &SourceList, scroll: f32, pressed: Option<u32>) {
    source_list_in(s, ui, bounds, list, scroll, pressed, None);
}

/// `source_list` whose rows are drawn only inside `rows` (points), while the column's material still fills all of
/// `bounds`: a full-size window's sidebar keeps its material under the traffic lights' band but a scrolled row never
/// draws there (`NSScrollView`'s content insets under a unified toolbar). One pass, one Sidebar accessibility group.
pub fn source_list_in(s: &mut dyn Canvas, ui: &Ui, bounds: Rect, list: &SourceList, scroll: f32, pressed: Option<u32>, rows: Option<Rect>) {
    let (bx, by, bw, bh) = (ui.pt(bounds.x) as usize, ui.pt(bounds.y) as usize, ui.pt(bounds.w).ceil() as usize, ui.pt(bounds.h).ceil() as usize);
    if !window_material(s, ui, bounds, 0.0, Material::Sidebar, State::default()) {
        s.fill_rect(RectF::new((bx) as f32, (by) as f32, (bw) as f32, (bh) as f32), SIDEBAR_BG.now());
    }
    if let Some(r) = rows {
        s.push_clip(RectF::new(ui.pt(r.x), ui.pt(r.y), ui.pt(r.w), ui.pt(r.h)), 0.0);
    }
    source_list_rows(s, ui, bounds, list, scroll, pressed);
    if rows.is_some() {
        s.pop_clip();
    }
}

/// The rows of `source_list` (its Sidebar accessibility group) over the material already drawn.
fn source_list_rows(s: &mut dyn Canvas, ui: &Ui, bounds: Rect, list: &SourceList, scroll: f32, pressed: Option<u32>) {
    // labels on the material blend vibrantly; a selected or pressed row's plate is opaque, so its text is plain
    bifrost_ui::ax::begin_group(bifrost_ui::Role::List, "Sidebar", bounds);
    let vibrant = vibrant_text();
    let inset = SPACE_S + SPACE_XS / 2.0;
    for (i, item) in list.items.iter().enumerate() {
        let r = list.item_rect(i, bounds, scroll);
        if r.bottom() <= bounds.y || r.y >= bounds.bottom() {
            continue; // scrolled out of the column
        }
        match item {
            SidebarItem::Header(label) => {
                let base = ui.pt(r.y + r.h) as i32 - ui.pt(SPACE_XS) as i32;
                label_text(s, ui, label, MAC_SUBHEAD_PT, Weight::Semibold, ui.pt(r.x + inset + SPACE_S) as i32, |h| base - h, vibrant.then_some(Vibrant::Secondary), LABEL_2.now());
            }
            SidebarItem::Section { label, collapsed, .. } => {
                // the label as a header, the disclosure chevron at the trailing edge (shown always, as Golden Gate
                // Files shows it on hover; there is no hover on touch)
                bifrost_ui::ax::record(bifrost_ui::Role::Button, label, if *collapsed { "collapsed" } else { "expanded" }, r, true);
                let base = ui.pt(r.y + r.h) as i32 - ui.pt(SPACE_XS) as i32;
                label_text(s, ui, label, MAC_SUBHEAD_PT, Weight::Semibold, ui.pt(r.x + inset + SPACE_S) as i32, |h| base - h, vibrant.then_some(Vibrant::Secondary), LABEL_2.now());
                let c = 9.0;
                let name = if *collapsed { "chevron.right" } else { "chevron.down" };
                glyph(s, ui, name, Rect { x: r.right() - inset - SPACE_S - c, y: r.y + r.h - SPACE_XS - c - 2.0, w: c, h: c }, LABEL_2.now());
            }
            SidebarItem::Row { .. } if r.h <= 0.0 => {}
            SidebarItem::Row { id, label, count, enabled, symbol, dot, subtitle, trailing, avatar, neutral } => {
                let selected = list.selected() == Some(*id);
                bifrost_ui::ax::record(bifrost_ui::Role::Row, label, if selected { "selected" } else { "" }, r, *enabled);
                let plate = Rect { x: r.x + inset, y: r.y + 1.0, w: r.w - 2.0 * inset, h: r.h - 2.0 };
                if selected {
                    s.fill_rrect(RectF::new(ui.pt(plate.x), ui.pt(plate.y), ui.pt(plate.w), ui.pt(plate.h)), ui.pt(m::SELECTION_RADIUS), SIDEBAR_PLATE.now());
                } else if pressed == Some(*id) && *enabled {
                    s.fill_rrect(RectF::new(ui.pt(plate.x), ui.pt(plate.y), ui.pt(plate.w), ui.pt(plate.h)), ui.pt(m::SELECTION_RADIUS), ROW_PRESSED.now());
                }
                outline_if_borders(s, ui, plate, m::SELECTION_RADIUS);
                let color = if !*enabled { LABEL_3.now() } else { LABEL.now() };
                // off the plate the row's text blends into the material
                let on_plate = selected || (pressed == Some(*id) && *enabled);
                let vib = |v: Vibrant| (vibrant && !on_plate).then_some(v);
                let mut text_x = plate.x + SPACE_S;
                if let Some((initials, c)) = avatar {
                    let d = (plate.h - 6.0).min(SIDEBAR_AVATAR_PT);
                    let (cx, cy) = (text_x + d / 2.0, plate.y + plate.h / 2.0);
                    crate::glyphs::disc(s, ui.pt(cx), ui.pt(cy), ui.pt(d / 2.0), *c);
                    let img = ui.text(initials, MAC_FOOTNOTE_PT, Weight::Semibold, WHITE);
                    blit(s, &img, ui.pt(cx) as i32 - img.w as i32 / 2, ui.pt(cy) as i32 - img.h as i32 / 2);
                    text_x += d + SPACE_S;
                } else if let Some(c) = dot {
                    let g = sidebar_glyph_pt();
                    let r = ui.pt(SIDEBAR_DOT_PT / 2.0);
                    crate::glyphs::disc(s, ui.pt(text_x + g / 2.0), ui.pt(plate.y + plate.h / 2.0), r, *c);
                    text_x += g + SPACE_S;
                } else if let Some(name) = symbol {
                    let g = sidebar_glyph_pt();
                    // accent glyph, or the secondary grey for a neutral row (Files's Locations)
                    let tint = if *neutral { LABEL_2.now() } else { crate::kit::appearance::accent_now() };
                    glyph(s, ui, name, Rect { x: text_x, y: plate.y + (plate.h - g) / 2.0, w: g, h: g }, if *enabled { tint } else { LABEL_3.now() });
                    text_x += g + SPACE_S;
                }
                let mid = ui.pt(plate.y + plate.h / 2.0) as i32;
                // trailing accessories right to left: the count, then the trailing text before it
                let mut right = ui.pt(plate.x + plate.w - SPACE_S) as i32;
                for t in count.map(|n| n.to_string()).into_iter().chain(trailing.as_ref().map(|t| ui.fit(t, MAC_BODY_PT, ui.pt(plate.w / 3.0)))) {
                    // right-aligned: measure, then draw
                    let w = ui.text(&t, MAC_BODY_PT, Weight::Regular, WHITE).w as i32;
                    label_text(s, ui, &t, MAC_BODY_PT, Weight::Regular, right - w, |h| mid - h / 2, vib(Vibrant::Secondary), LABEL_2.now());
                    right -= w + ui.pt(SPACE_S) as i32;
                }
                let room = (right as f32 - ui.pt(text_x)).max(0.0);
                let weight = if selected { Weight::Bold } else { Weight::Regular };
                let text = ui.fit(label, MAC_BODY_PT, room);
                let level = if *enabled { Vibrant::Label } else { Vibrant::Tertiary };
                if let Some(sub) = subtitle {
                    let sub = ui.fit(sub, MAC_SUBHEAD_PT, room);
                    let sub_h = ui.text(&sub, MAC_SUBHEAD_PT, Weight::Regular, WHITE).h as i32;
                    let (_, h) = label_text(s, ui, &text, MAC_BODY_PT, weight, ui.pt(text_x) as i32, |h| mid - (h + sub_h) / 2, vib(level), color);
                    let top = mid - (h as i32 + sub_h) / 2 + h as i32;
                    label_text(s, ui, &sub, MAC_SUBHEAD_PT, Weight::Regular, ui.pt(text_x) as i32, |_| top, vib(Vibrant::Secondary), LABEL_2.now());
                } else {
                    label_text(s, ui, &text, MAC_BODY_PT, weight, ui.pt(text_x) as i32, |h| mid - h / 2, vib(level), color);
                }
            }
        }
    }
    bifrost_ui::ax::end_group();
}

/// A search field (`NativeSearchField`): a capsule with a leading magnifier, the placeholder or text, a clear button
/// while there is text, and the focus ring when `focused`. Geometry and clear-button hit-testing are
/// `bifrost_ui::search` (`search::hits_clear(bounds, !field.is_empty(), x, y)`). `bounds` in points.
pub fn search_field(s: &mut dyn Canvas, ui: &Ui, field: &TextField, bounds: Rect, placeholder: &str, focused: bool) {
    let Rect { x, y, w, h } = bounds;
    bifrost_ui::ax::record(bifrost_ui::Role::TextField, placeholder, field, bounds, true);
    let r = h / 2.0;
    if focused {
        let o = 3.0;
        s.fill_rrect(RectF::new(ui.pt(x - o), ui.pt(y - o), ui.pt(w + 2.0 * o), ui.pt(h + 2.0 * o)), ui.pt(r + o), crate::kit::appearance::accent_now());
        s.fill_rrect(RectF::new(ui.pt(x - 1.0), ui.pt(y - 1.0), ui.pt(w + 2.0), ui.pt(h + 2.0)), ui.pt(r + 1.0), DETAIL_BG.now());
    }
    s.fill_rrect(RectF::new(ui.pt(x), ui.pt(y), ui.pt(w), ui.pt(h)), ui.pt(r), FIELD_BG.now());
    outline_if_borders(s, ui, bounds, r);
    let value: &str = field;
    let parts = bifrost_ui::search::layout(bounds, !value.is_empty());
    glyph(s, ui, "magnifyingglass", parts.magnifier, LABEL_2.now());
    if let Some(c) = parts.clear {
        glyph(s, ui, "xmark.circle.fill", c, LABEL_3.now());
    }
    let room = ui.pt(parts.text.w);
    let left = ui.pt(parts.text.x);
    let im_h = ui.pt(17.0);
    let top = ui.pt(y) + (ui.pt(h) - im_h) / 2.0;
    if value.is_empty() {
        let im = ui.text(&ui.fit(placeholder, MAC_BODY_PT, room), MAC_BODY_PT, Weight::Regular, LABEL_2.now());
        blit(s, &im, left as i32, top as i32);
    } else {
        let mut start = 0;
        while start < field.caret() && ui.measure(&value[start..field.caret()], MAC_BODY_PT, Weight::Regular) > room {
            start += value[start..].chars().next().unwrap().len_utf8();
        }
        let visible = ui.fit(&value[start..], MAC_BODY_PT, room);
        if focused && !field.selection().is_empty() {
            let sel = field.selection();
            let rel = |i: usize| i.saturating_sub(start).min(visible.len());
            let (a, b) = (rel(sel.start), rel(sel.end));
            let sx = ui.measure(&visible[..a], MAC_BODY_PT, Weight::Regular);
            let ex = ui.measure(&visible[..b], MAC_BODY_PT, Weight::Regular);
            // Settings > Appearance > Highlight Colour (the accent unless chosen)
            s.fill_rrect(RectF::new(left + sx, top, ex - sx + ui.pt(1.0), im_h), 0.0, crate::kit::ui::highlight_now());
            let mut cx = left as i32;
            for (part, color) in [(&visible[..a], LABEL.now()), (&visible[a..b], WHITE), (&visible[b..], LABEL.now())] {
                if part.is_empty() {
                    continue;
                }
                let im = ui.text(part, MAC_BODY_PT, Weight::Regular, color);
                blit(s, &im, cx, top as i32);
                cx += ui.measure(part, MAC_BODY_PT, Weight::Regular) as i32;
            }
        } else {
            let im = ui.text(&visible, MAC_BODY_PT, Weight::Regular, LABEL.now());
            blit(s, &im, left as i32, top as i32);
            if focused {
                let cx = ui.measure(&value[start..field.caret()], MAC_BODY_PT, Weight::Regular).min(room);
                s.fill_rrect(RectF::new(left + cx, top, ui.pt(1.0), im_h), 0.0, TINT);
            }
        }
    }
}

/// Lerps two straight colours `(r, g, b)` by `t` and packs them opaque.
fn mix(a: (u8, u8, u8), b: (u8, u8, u8), t: f32) -> u32 {
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    rgba(l(a.0, b.0), l(a.1, b.1), l(a.2, b.2), 255)
}

/// A Golden Gate Files folder drawn at any size in `bounds` (points; drawn square, centred): the lighter back tab,
/// a faint white paper edge, the front body with a soft vertical gradient and darker bottom edge, and an optional
/// embossed SF symbol (`emblem`, Desktop, Documents, Downloads, ...) in a darker blue. Folders look the same in Dark
/// and Light, as in Files.
pub fn folder_icon(s: &mut dyn Canvas, ui: &Ui, bounds: Rect, emblem: Option<&str>) {
    let side = bounds.w.min(bounds.h);
    let (x, y) = (bounds.x + (bounds.w - side) / 2.0, bounds.y + (bounds.h - side) / 2.0);
    let (w, h) = (side * 0.92, side * 0.74);
    let (fx, fy) = (x + (side - w) / 2.0, y + side * 0.14);
    let r = side * 0.07;
    // the back: a tab on the left and the band above the front
    let back = rgba(89, 189, 233, 255);
    s.fill_rrect(RectF::new(ui.pt(fx), ui.pt(fy), ui.pt(w * 0.42), ui.pt(h * 0.3)), ui.pt(r), back);
    s.fill_rrect(RectF::new(ui.pt(fx), ui.pt(fy + h * 0.08), ui.pt(w), ui.pt(h * 0.92)), ui.pt(r), back);
    // the paper edge between the back and the front
    let body_y = fy + h * 0.2;
    s.fill_rrect(RectF::new(ui.pt(fx + w * 0.03), ui.pt(body_y - side * 0.02), ui.pt(w * 0.94), ui.pt(side * 0.04)), ui.pt(side * 0.02), rgba(255, 255, 255, 150));
    // the front body: a darker bottom edge, then the gradient in horizontal bands
    let body_h = fy + h - body_y;
    let one = 1.0 / ui.pt(1.0).max(1.0);
    s.fill_rrect(RectF::new(ui.pt(fx), ui.pt(body_y), ui.pt(w), ui.pt(body_h)), ui.pt(r), rgba(62, 150, 196, 255));
    let (top, bottom) = ((88, 170, 207), (74, 178, 227));
    s.fill_rrect(RectF::new(ui.pt(fx), ui.pt(body_y), ui.pt(w), ui.pt(body_h - one)), ui.pt(r), mix(top, bottom, 0.5));
    let inner_top = ui.pt(body_y + r);
    let inner_bottom = ui.pt(body_y + body_h - one - r);
    let rows = (inner_bottom - inner_top).max(0.0) as usize;
    for k in 0..rows {
        let t = (k as f32 + 0.5) / rows.max(1) as f32;
        s.fill_rect(RectF::new((ui.pt(fx) as usize) as f32, (inner_top as usize + k) as f32, (ui.pt(w).ceil() as usize) as f32, (1) as f32), mix(top, bottom, t));
    }
    if let Some(name) = emblem {
        // embossed: a light line a hair below the darker glyph, so it reads on the body in Light and Dark alike
        let g = body_h * 0.4;
        let at = Rect { x: fx + (w - g) / 2.0, y: body_y + (body_h - g) / 2.0, w: g, h: g };
        glyph(s, ui, name, Rect { y: at.y + side * 0.012, ..at }, FOLDER_EMBLEM_HILITE);
        glyph(s, ui, name, at, FOLDER_EMBLEM);
    }
}

/// A collapsed search control (Golden Gate toolbars): a round glass capsule `CAPSULE_PT` wide with a magnifier at
/// `origin` (points). The app swaps it for `search_field` when clicked and back when the field is emptied and loses
/// focus or on Esc. `pressed` shades the plate. Returns the button rectangle for hit-testing.
pub fn search_button(s: &mut dyn Canvas, ui: &Ui, origin: (f32, f32), pressed: bool) -> Rect {
    let r = Rect { x: origin.0, y: origin.1, w: CAPSULE_PT, h: CAPSULE_PT };
    bifrost_ui::ax::record(bifrost_ui::Role::Button, "Search", "", r, true);
    capsule(s, ui, r);
    outline_if_borders(s, ui, r, r.h / 2.0);
    if pressed {
        s.fill_rrect(RectF::new(ui.pt(r.x + 3.0), ui.pt(r.y + 3.0), ui.pt(r.w - 6.0), ui.pt(r.h - 6.0)), ui.pt((r.h - 6.0) / 2.0), ROW_PRESSED.now());
    }
    let g = TOOLBAR_GLYPH_PT;
    glyph(s, ui, "magnifyingglass", Rect { x: r.x + (r.w - g) / 2.0, y: r.y + (r.h - g) / 2.0, w: g, h: g }, LABEL.now());
    r
}

/// Side of a path-bar glyph and the space a chevron takes, in points.
const PATH_GLYPH_PT: f32 = 14.0;
const PATH_CHEVRON_PT: f32 = 14.0;

/// A path bar (Files's View > Show Path Bar, `NativePathControl`) in `bounds` (points): each `(label, symbol)` from the
/// root with a glyph, chevrons between, middle labels shrinking to their glyph when it is narrow. Returns the
/// segments for hit-testing (`bifrost_ui::pathbar::hit`).
pub fn path_bar(s: &mut dyn Canvas, ui: &Ui, bounds: Rect, items: &[(&str, &str)]) -> Vec<Segment> {
    let pt = MAC_CALLOUT_PT;
    let widths: Vec<f32> = items.iter().map(|(l, _)| ui.measure(l, pt, Weight::Regular) / ui.pt(1.0)).collect();
    let segs = pathbar::layout(&widths, bounds, PATH_GLYPH_PT + SPACE_XS, PATH_CHEVRON_PT);
    // the segments are the path bar's children in the accessibility tree
    bifrost_ui::ax::begin_group(bifrost_ui::Role::List, "Path", bounds);
    for (k, seg) in segs.iter().enumerate() {
        let (label, symbol) = items[seg.index];
        let r = seg.rect;
        bifrost_ui::ax::record(bifrost_ui::Role::Button, label, "", r, true);
        let g = PATH_GLYPH_PT;
        glyph(s, ui, symbol, Rect { x: r.x, y: r.y + (r.h - g) / 2.0, w: g, h: g }, if k + 1 == segs.len() { LABEL_2.now() } else { TINT });
        if seg.label_w > 0.5 {
            let img = ui.text(&ui.fit(label, pt, ui.pt(seg.label_w)), pt, Weight::Regular, LABEL_2.now());
            blit(s, &img, ui.pt(r.x + g + SPACE_XS) as i32, (ui.pt(r.y + r.h / 2.0) as i32) - img.h as i32 / 2);
        }
        if k + 1 < segs.len() {
            let c = 8.0;
            glyph(s, ui, "chevron.right", Rect { x: r.right() + (PATH_CHEVRON_PT - c) / 2.0, y: r.y + (r.h - c) / 2.0, w: c, h: c }, LABEL_3.now());
        }
    }
    bifrost_ui::ax::end_group();
    segs
}

/// A single-line field with selection, caret and visible keyboard focus.
pub fn text_field(
    s: &mut dyn Canvas,
    ui: &Ui,
    field: &TextField,
    bounds: Rect,
    placeholder: &str,
    focused: bool,
) {
    let Rect { x, y, w, h } = bounds;
    bifrost_ui::ax::record(bifrost_ui::Role::TextField, placeholder, field, bounds, true);
    s.fill_rrect(RectF::new(ui.pt(x), ui.pt(y), ui.pt(w), ui.pt(h)), ui.pt(6.), if focused { crate::kit::appearance::accent_now() } else { FIELD_BG.now() });
    if focused {
        s.fill_rrect(RectF::new(ui.pt(x + 1.), ui.pt(y + 1.), ui.pt(w - 2.), ui.pt(h - 2.)), ui.pt(5.), FIELD_BG.now());
    }
    outline_if_borders(s, ui, bounds, 6.);
    let room = ui.pt((w - 24.).max(0.));
    let value: &str = field;
    let mut start = 0;
    // Keep the caret visible without cutting a UTF-8 sequence.
    while start < field.caret()
        && ui.measure(&value[start..field.caret()], 13., Weight::Regular) > room
    {
        start += value[start..].chars().next().unwrap().len_utf8();
    }
    let visible = ui.fit(&value[start..], 13., room);
    let left = ui.pt(x + 12.);
    let top = ui.pt(y + (h - 16.) / 2.);
    if focused {
        let selection = field.selection();
        let a = selection.start.max(start);
        let b = selection.end.max(start);
        let sx = ui.measure(&value[start..a], 13., Weight::Regular).min(room);
        let ex = ui.measure(&value[start..b], 13., Weight::Regular).min(room);
        if ex > sx {
            s.fill_rrect(RectF::new(left + sx, top, ex - sx + ui.pt(1.), ui.pt(17.)), 0., crate::kit::ui::highlight_now());
        }
    }
    if value.is_empty() {
        let im = ui.text(placeholder, 13., Weight::Regular, LABEL_2.now());
        blit(s, &im, left as i32, top as i32);
    } else if focused && !field.selection().is_empty() {
        // Selected characters are white on the selection plate; the rest keeps the label colour.
        let sel = field.selection();
        let rel = |i: usize| i.saturating_sub(start).min(visible.len());
        let (a, b) = (rel(sel.start), rel(sel.end));
        let mut x = left as i32;
        for (part, color) in [(&visible[..a], LABEL.now()), (&visible[a..b], WHITE), (&visible[b..], LABEL.now())] {
            if part.is_empty() {
                continue;
            }
            let im = ui.text(part, 13., Weight::Regular, color);
            blit(s, &im, x, top as i32);
            x += ui.measure(part, 13., Weight::Regular) as i32;
        }
    } else {
        let im = ui.text(&visible, 13., Weight::Regular, LABEL.now());
        blit(s, &im, left as i32, top as i32);
    }
    if focused && field.selection().is_empty() {
        let cx = ui
            .measure(&value[start..field.caret()], 13., Weight::Regular)
            .min(room);
        s.fill_rrect(RectF::new(left + cx, top, ui.pt(1.), ui.pt(17.)), 0., TINT);
    }
}

/// Menu metrics in points: row height, separator height, vertical padding, side inset of the highlight.
pub const MENU_ROW_PT: f32 = 22.0;
pub const MENU_SEPARATOR_PT: f32 = 9.0;
pub const MENU_PAD_PT: f32 = 5.0;
const MENU_INSET_PT: f32 = 5.0;
/// The round colour swatch of a menu item (`MenuItem::swatch`) and the gap after it, in points.
pub const MENU_SWATCH_PT: f32 = 10.0;
pub const MENU_SWATCH_GAP_PT: f32 = 6.0;

/// A pop-up button (`NativePopUpButton`): the current choice with an up/down chevron pair. `open` draws it pressed.
/// `bounds` in points; the standard height is 22.
pub fn popup_button(s: &mut dyn Canvas, ui: &Ui, bounds: Rect, title: &str, open: bool, enabled: bool) {
    let Rect { x, y, w, h } = bounds;
    bifrost_ui::ax::record(bifrost_ui::Role::Button, title, if open { "open" } else { "" }, bounds, enabled);
    s.fill_rrect(RectF::new(ui.pt(x), ui.pt(y), ui.pt(w), ui.pt(h)), ui.pt(6.), if open { CONTROL_PRESSED.now() } else { FIELD_BG.now() });
    outline_if_borders(s, ui, bounds, 6.);
    let color = if enabled { LABEL.now() } else { LABEL_3.now() };
    let room = ui.pt((w - 34.).max(0.));
    let img = ui.text(&ui.fit(title, MAC_BODY_PT, room), MAC_BODY_PT, Weight::Regular, color);
    blit(s, &img, ui.pt(x + 10.) as i32, (ui.pt(y) + (ui.pt(h) - img.h as f32) / 2.0) as i32);
    // chevrons: two small triangles, tapering one pixel row at a time, centred in the right 24 pt
    let (cx, cy) = (ui.pt(x + w - 14.), ui.pt(y + h / 2.));
    let row = ui.pt(1.0).max(1.0);
    // up triangle on top (apex up), down triangle below (apex down): each is three rows, one pixel wider per row
    for k in 0..3 {
        let half = (k as f32 + 0.5) * row;
        s.fill_rrect(RectF::new(cx - half, cy - ui.pt(4.5) + k as f32 * row, half * 2.0, row), 0.0, color);
        s.fill_rrect(RectF::new(cx - half, cy + ui.pt(4.5) - (k as f32 + 1.0) * row, half * 2.0, row), 0.0, color);
    }
}

/// Row pitch, padding, width and the gap under the field of the accent candidate list, in points.
const CANDIDATE_ROW_PT: f32 = 24.0;
const CANDIDATE_PAD_PT: f32 = 6.0;
const CANDIDATE_WIDTH_PT: f32 = 180.0;
const CANDIDATE_GAP_PT: f32 = 4.0;

/// The accent candidate list of an open composition (`text::TextEditor::candidates`): a popover of numbered rows under
/// `anchor`, one row per candidate. Draws nothing when `items` is empty. Shared by every SDK text field and text view.
pub fn candidate_list(s: &mut dyn Canvas, ui: &Ui, anchor: Rect, items: &[String]) {
    if items.is_empty() {
        return;
    }
    let (row, pad) = (CANDIDATE_ROW_PT, CANDIDATE_PAD_PT);
    let bounds = Rect { x: anchor.x, y: anchor.bottom() + CANDIDATE_GAP_PT, w: CANDIDATE_WIDTH_PT, h: pad * 2.0 + row * items.len() as f32 };
    material(s, ui, bounds, 8.0, Material::Popover);
    bifrost_ui::ax::begin_group(bifrost_ui::Role::List, "Candidates", bounds);
    for (i, item) in items.iter().enumerate() {
        bifrost_ui::ax::record(bifrost_ui::Role::Row, item, "", Rect { x: bounds.x + pad, y: bounds.y + pad + row * i as f32, w: bounds.w - 2.0 * pad, h: row }, true);
        let img = ui.text(&format!("{}   {}", i + 1, item), MAC_BODY_PT, Weight::Regular, WHITE);
        let x = ui.pt(bounds.x + pad * 2.0);
        let y = ui.pt(bounds.y + pad + row * i as f32) + (ui.pt(row) - img.h as f32) / 2.0;
        image_tinted_at(s, &img, x as i32, y as i32, LABEL.now());
    }
    bifrost_ui::ax::end_group();
}

/// An open menu on the Popover material at `origin` (points). The highlighted row is the accent colour with white
/// text, disabled rows are tertiary, separators are hairlines, checked rows carry a check mark, and shortcuts sit
/// at the trailing edge. Returns the menu's rectangle in points.
pub fn menu(s: &mut dyn Canvas, ui: &Ui, origin: (f32, f32), width: f32, menu: &Menu) -> Rect {
    let (rows, total) = menu.rows(MENU_ROW_PT, MENU_SEPARATOR_PT, MENU_PAD_PT);
    let bounds = Rect { x: origin.0, y: origin.1, w: width, h: total };
    // a menu of colours (calendars, tags): a round swatch before each title, and every title moved over by it
    let swatched = menu.items.iter().any(|i| i.swatch.is_some());
    let lead = if swatched { MENU_SWATCH_PT + MENU_SWATCH_GAP_PT } else { 0.0 };
    material(s, ui, bounds, 8.0, Material::Popover);
    s.stroke_rrect(RectF::new(ui.pt(bounds.x), ui.pt(bounds.y), ui.pt(width), ui.pt(total)), ui.pt(8.), ui.pt(1.0).max(1.0), HAIRLINE_C.now());
    // the rows are the menu's children in the accessibility tree; a separator is not an element
    bifrost_ui::ax::begin_group(bifrost_ui::Role::List, "Menu", bounds);
    for (i, (item, &(top, h))) in menu.items.iter().zip(&rows).enumerate() {
        let y = origin.1 + top;
        if !item.separator {
            bifrost_ui::ax::record(bifrost_ui::Role::MenuItem, &item.title, if item.checked { "checked" } else { "" }, Rect { x: origin.0, y, w: width, h }, item.enabled);
        }
        if item.separator {
            s.fill_rect(RectF::new((ui.pt(origin.0 + MENU_INSET_PT * 2.) as usize) as f32, (ui.pt(y + h / 2.) as usize) as f32, (ui.pt(width - MENU_INSET_PT * 4.) as usize) as f32, (ui.pt(1.0).max(1.0) as usize) as f32), SEPARATOR_C.now());
            continue;
        }
        let hot = menu.highlighted == Some(i) && item.enabled;
        if hot {
            s.fill_rrect(RectF::new(ui.pt(origin.0 + MENU_INSET_PT), ui.pt(y), ui.pt(width - MENU_INSET_PT * 2.), ui.pt(h)), ui.pt(5.), TINT);
        }
        let color = if hot { WHITE } else if !item.enabled { LABEL_3.now() } else if item.destructive { bifrost_render::ui::DESTRUCTIVE } else { LABEL.now() };
        let line = |t: &str, c: u32| ui.text(t, MAC_BODY_PT, Weight::Regular, c);
        let center = |img: &crate::kit::Image| (ui.pt(y) + (ui.pt(h) - img.h as f32) / 2.0) as i32;
        if item.checked {
            let tick = line("\u{2713}", color);
            blit(s, &tick, ui.pt(origin.0 + MENU_INSET_PT + 6.) as i32, center(&tick));
        }
        if let Some(c) = item.swatch {
            let d = MENU_SWATCH_PT;
            s.fill_rrect(RectF::new(ui.pt(origin.0 + MENU_INSET_PT + 24.), ui.pt(y + (h - d) / 2.), ui.pt(d), ui.pt(d)), ui.pt(d / 2.), c);
        }
        let room = ui.pt(width - 2. * MENU_INSET_PT - 26. - 12. - lead - item.shortcut.as_ref().map_or(0.0, |_| 48.));
        let title = line(&ui.fit(&item.title, MAC_BODY_PT, room.max(0.)), color);
        blit(s, &title, ui.pt(origin.0 + MENU_INSET_PT + 24. + lead) as i32, center(&title));
        if let Some(sc) = &item.shortcut {
            let t = ui.text(sc, MAC_BODY_PT, Weight::Regular, if hot { WHITE } else { LABEL_2.now() });
            blit(s, &t, (ui.pt(origin.0 + width - MENU_INSET_PT - 10.) - t.w as f32) as i32, center(&t));
        }
    }
    bifrost_ui::ax::end_group();
    bounds
}

/// Height of the floating player capsule, in points.
pub const PLAYER_PT: f32 = 52.0;

/// What a floating player shows (Golden Gate Music and Podcasts mini player, `MPNowPlayingInfoCenter` fields).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Player<'a> {
    pub title: &'a str,
    pub subtitle: &'a str,
    pub playing: bool,
    /// Elapsed share of the item, 0..=1; `None` hides the progress line (a live radio stream).
    pub progress: Option<f32>,
    /// Skip buttons as the system symbol set names, `(back, forward)`: `("backward.fill", "forward.fill")` for Music,
    /// `("gobackward.15", "goforward.30")` for Podcasts. `None` draws only play/pause.
    pub skips: Option<(&'a str, &'a str)>,
}

/// Where the parts of a player capsule are, in points, for hit-testing.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PlayerLayout {
    pub capsule: Rect,
    /// The square the app draws the artwork into (after `player` returns), rounded by the app or left as drawn.
    pub artwork: Rect,
    pub text: Rect,
    pub back: Option<Rect>,
    pub play: Rect,
    pub forward: Option<Rect>,
    /// The progress track along the bottom of the text; a click here seeks to `(x - progress.x) / progress.w`.
    pub progress: Option<Rect>,
}

impl PlayerLayout {
    /// Lays the player out in `bounds` (its height is `PLAYER_PT` at most, centred vertically).
    pub fn new(bounds: Rect, player: &Player) -> Self {
        let h = PLAYER_PT.min(bounds.h);
        let capsule = Rect { x: bounds.x, y: bounds.y + (bounds.h - h) / 2.0, w: bounds.w, h };
        let pad = 6.0;
        let art = h - 2.0 * pad;
        let artwork = Rect { x: capsule.x + pad, y: capsule.y + pad, w: art, h: art };
        let b = (h - 2.0 * pad).min(CAPSULE_PT);
        let by = capsule.y + (h - b) / 2.0;
        let mut x = capsule.right() - pad * 2.0;
        let mut take = || {
            x -= b;
            Rect { x, y: by, w: b, h: b }
        };
        let (forward, play, back) = if player.skips.is_some() {
            let f = take();
            let p = take();
            (Some(f), p, Some(take()))
        } else {
            (None, take(), None)
        };
        let tx = artwork.right() + SPACE_S;
        let text = Rect { x: tx, y: capsule.y + pad, w: (back.unwrap_or(play).x - SPACE_S - tx).max(0.0), h: art };
        let progress = player.progress.map(|_| Rect { x: text.x, y: text.bottom() - 4.0, w: text.w, h: 4.0 });
        PlayerLayout { capsule, artwork, text, back, play, forward, progress }
    }
}

/// A floating player capsule (Golden Gate Music and Podcasts mini player; `MPNowPlayingInfoCenter` with
/// `MPRemoteCommandCenter` transport): a glass capsule with an artwork slot, the title and subtitle, a thin progress
/// line, and skip/play buttons. The app draws its artwork into `artwork` afterwards (a music note shows meanwhile).
/// `pressed` is the button rect being pressed (compare with the layout's rects).
pub fn player(s: &mut dyn Canvas, ui: &Ui, bounds: Rect, p: &Player, pressed: Option<Rect>) -> PlayerLayout {
    let l = PlayerLayout::new(bounds, p);
    let c = l.capsule;
    // the transport buttons are the player's children in the accessibility tree; its value is what is playing
    bifrost_ui::ax::begin_group(bifrost_ui::Role::List, "Player", c);
    capsule(s, ui, c);
    outline_if_borders(s, ui, c, c.h / 2.0);
    let a = l.artwork;
    s.fill_rrect(RectF::new(ui.pt(a.x), ui.pt(a.y), ui.pt(a.w), ui.pt(a.h)), ui.pt(a.h / 2.0), FIELD_BG.now());
    let g = a.w * 0.5;
    glyph(s, ui, "music.note", Rect { x: a.x + (a.w - g) / 2.0, y: a.y + (a.h - g) / 2.0, w: g, h: g }, LABEL_3.now());
    let room = ui.pt(l.text.w);
    let title = ui.text(&ui.fit(p.title, MAC_BODY_PT, room), MAC_BODY_PT, Weight::Semibold, LABEL.now());
    let sub = ui.text(&ui.fit(p.subtitle, MAC_SUBHEAD_PT, room), MAC_SUBHEAD_PT, Weight::Regular, LABEL_2.now());
    let top = ui.pt(l.text.y + l.text.h / 2.0) as i32 - (title.h + sub.h) as i32 / 2 - if l.progress.is_some() { ui.pt(2.0) as i32 } else { 0 };
    blit(s, &title, ui.pt(l.text.x) as i32, top);
    blit(s, &sub, ui.pt(l.text.x) as i32, top + title.h as i32);
    if let (Some(r), Some(f)) = (l.progress, p.progress) {
        let t = 2.0;
        let y = r.y + (r.h - t) / 2.0;
        s.fill_rrect(RectF::new(ui.pt(r.x), ui.pt(y), ui.pt(r.w), ui.pt(t)), ui.pt(t / 2.0), SEPARATOR_C.now());
        s.fill_rrect(RectF::new(ui.pt(r.x), ui.pt(y), ui.pt(r.w * f.clamp(0.0, 1.0)), ui.pt(t)), ui.pt(t / 2.0), LABEL_2.now());
    }
    let button = |s: &mut dyn Canvas, r: Rect, name: &str, label: &str| {
        bifrost_ui::ax::record(bifrost_ui::Role::Button, label, "", r, true);
        if pressed == Some(r) {
            s.fill_rrect(RectF::new(ui.pt(r.x), ui.pt(r.y), ui.pt(r.w), ui.pt(r.h)), ui.pt(r.h / 2.0), ROW_PRESSED.now());
        }
        let g = TOOLBAR_GLYPH_PT.min(r.h * 0.6);
        glyph(s, ui, name, Rect { x: r.x + (r.w - g) / 2.0, y: r.y + (r.h - g) / 2.0, w: g, h: g }, LABEL.now());
    };
    if let (Some((b, f)), Some(br), Some(fr)) = (p.skips, l.back, l.forward) {
        button(s, br, b, "Back");
        button(s, fr, f, "Forward");
    }
    button(s, l.play, if p.playing { "pause.fill" } else { "play.fill" }, if p.playing { "Pause" } else { "Play" });
    bifrost_ui::ax::end_group();
    l
}

/// A sub-surface for one pane of a split view (`NativeSplitViewController` child view): drawing at `(0, 0)` lands at the
/// top-left of `pane` (points) and is clipped to it, so a detail pane draws as if it were its own window. The pane is
/// clamped to `s`. Damage the app reports is still in the full surface's pixels: offset by `pane_origin_px`.
pub fn pane<'a>(s: &'a mut Surface, ui: &Ui, pane: Rect) -> Surface<'a> {
    let (x, y, w, h) = pane_px(s, ui, pane);
    let start = (y - s.y0) * s.stride + x;
    let end = if h == 0 { start } else { start + (h - 1) * s.stride + w };
    let (stride, len) = (s.stride, s.px.len());
    Surface { w, h, stride, px: &mut s.px[start..end.min(len)], y0: 0 }
}

/// The pixel origin of `pane` inside `s` (to offset damage reported from a `pane` surface).
pub fn pane_origin_px(s: &Surface, ui: &Ui, pane: Rect) -> (usize, usize) {
    let (x, y, _, _) = pane_px(s, ui, pane);
    (x, y)
}

fn pane_px(s: &Surface, ui: &Ui, pane: Rect) -> (usize, usize, usize, usize) {
    let x = (ui.pt(pane.x).round().max(0.0) as usize).min(s.w);
    let y = (ui.pt(pane.y).round().max(0.0) as usize).clamp(s.y0, s.h);
    let w = (ui.pt(pane.w).round().max(0.0) as usize).min(s.w - x);
    let h = (ui.pt(pane.h).round().max(0.0) as usize).min(s.h - y);
    (x, y, w, h)
}

/// Size of a popover's arrow (base width, height), in points.
pub const POPOVER_ARROW_PT: (f32, f32) = (18.0, 9.0);

/// A popover (`NativePopover`): a rounded panel on the Popover material next to `anchor` (points) inside `bounds`, with an
/// arrow pointing at the anchor. `size` is the content size; the panel opens on `preferred` and flips when there is
/// no room (`controls::split::place_popover`). Returns the content rectangle to draw the inspector into (Calendar
/// event details, Reminders info). Dismiss on a click outside it, as `NativePopover.Behavior.transient`.
pub fn popover(s: &mut dyn Canvas, ui: &Ui, anchor: Rect, size: (f32, f32), bounds: Rect, preferred: bifrost_ui::geometry::Edge) -> Rect {
    use bifrost_ui::geometry::Edge;
    let (aw, ah) = POPOVER_ARROW_PT;
    let (r, edge) = bifrost_ui::split::place_popover(anchor, size, bounds, preferred, ah);
    material(s, ui, r, 10.0, Material::Popover);
    s.stroke_rrect(RectF::new(ui.pt(r.x), ui.pt(r.y), ui.pt(r.w), ui.pt(r.h)), ui.pt(10.0), ui.pt(1.0).max(1.0), HAIRLINE_C.now());
    let fill = Material::Popover.solid();
    // the arrow sits on the side facing the anchor, centred on it as far as the panel's corners allow
    let cx = (anchor.x + anchor.w / 2.0).clamp(r.x + 14.0 + aw / 2.0, (r.right() - 14.0 - aw / 2.0).max(r.x + 14.0 + aw / 2.0));
    let cy = (anchor.y + anchor.h / 2.0).clamp(r.y + 14.0 + aw / 2.0, (r.bottom() - 14.0 - aw / 2.0).max(r.y + 14.0 + aw / 2.0));
    let pts: [(f32, f32); 3] = match edge {
        Edge::Bottom => [(cx - aw / 2.0, r.y + 1.0), (cx, r.y - ah), (cx + aw / 2.0, r.y + 1.0)],
        Edge::Top => [(cx - aw / 2.0, r.bottom() - 1.0), (cx, r.bottom() + ah), (cx + aw / 2.0, r.bottom() - 1.0)],
        Edge::Right => [(r.x + 1.0, cy - aw / 2.0), (r.x - ah, cy), (r.x + 1.0, cy + aw / 2.0)],
        Edge::Left => [(r.right() - 1.0, cy - aw / 2.0), (r.right() + ah, cy), (r.right() - 1.0, cy + aw / 2.0)],
    };
    let px: Vec<(f32, f32)> = pts.iter().map(|&(x, y)| (ui.pt(x), ui.pt(y))).collect();
    crate::glyphs::polygon(s, &px, fill);
    r
}

/// Inner padding of the translation popover's content, in points.
const TRANSLATION_PAD_PT: f32 = 12.0;

/// The system translation popover (`translationPresentation`): the panel of a [`crate::translation_popover::TranslationPresenter`]
/// next to `anchor` inside `bounds`. The panel shows the language pair in footnote size, then the translation, or
/// "Translating" while the engine runs, or the reason a translation could not run. A long translation wraps and ends in
/// an ellipsis when the panel is full. Returns the panel's content rectangle, or `None` when the popover is hidden.
/// The app writes the translation into its document itself (`TranslationPresenter::replacement`) and dismisses the
/// popover on a click outside the returned rectangle.
pub fn translation_popover(s: &mut dyn Canvas, ui: &Ui, presenter: &crate::translation_popover::TranslationPresenter, anchor: Rect, bounds: Rect) -> Option<Rect> {
    use crate::translation_popover::{Phase, TRANSLATION_POPOVER_SIZE_PT};
    let pop = presenter.popover();
    if !pop.is_presented() {
        return None;
    }
    let r = popover(s, ui, anchor, TRANSLATION_POPOVER_SIZE_PT, bounds, presenter.preferred_edge());
    let pad = TRANSLATION_PAD_PT;
    let x = ui.pt(r.x + pad) as i32;
    let mut y = r.y + pad;
    if let Some((source, target)) = pop.languages() {
        let head = format!("{} to {}", source, target);
        let img = ui.text(&head, MAC_FOOTNOTE_PT, Weight::Regular, LABEL_2.now());
        blit(s, &img, x, ui.pt(y) as i32);
        y += MAC_FOOTNOTE_PT * 1.6;
    }
    let (body, color) = match pop.phase() {
        Phase::Hidden => return Some(r),
        Phase::Translating => ("Translating".to_string(), LABEL_2.now()),
        Phase::Shown(text) => (text.clone(), LABEL.now()),
        Phase::Failed(e) => (e.error_description().to_string(), LABEL_2.now()),
    };
    let line_h = MAC_BODY_PT * 1.3;
    let room = (r.bottom() - pad - y).max(line_h);
    let max_lines = ((room / line_h).floor() as usize).max(1);
    let width_px = ui.pt(r.w - 2.0 * pad);
    let mut lines = ui.wrap(&body, MAC_BODY_PT, Weight::Regular, width_px);
    if lines.len() > max_lines {
        lines.truncate(max_lines);
        if let Some(last) = lines.last_mut() {
            let with_dots = format!("{}\u{2026}", last.trim_end());
            *last = ui.fit(&with_dots, MAC_BODY_PT, width_px);
        }
    }
    for line in &lines {
        let img = ui.text(line, MAC_BODY_PT, Weight::Regular, color);
        blit(s, &img, x, ui.pt(y) as i32);
        y += line_h;
    }
    Some(r)
}

/// Draws the keyboard focus ring around a control of `bounds` and corner `radius` (points): 3 pt of accent at
/// 45% strength outside the control (`bifrost_ui::sheet::focus_ring_rect`).
pub fn focus_ring(s: &mut dyn Canvas, ui: &Ui, bounds: Rect, radius: f32) {
    let (r, rad) = bifrost_ui::sheet::focus_ring_rect(bounds, radius);
    // the accent at ~45% (115 of 255), premultiplied: systemBlue by default
    let accent = crate::kit::appearance::accent_now();
    let part = |shift: u32| (((accent >> shift) & 255) * 115 / 255) as u8;
    let c = crate::kit::pack(part(0), part(8), part(16), 115);
    s.stroke_rrect(RectF::new(ui.pt(r.x), ui.pt(r.y), ui.pt(r.w), ui.pt(r.h)), ui.pt(rad), ui.pt(bifrost_ui::sheet::FOCUS_RING_PT), c);
}

/// A push button of a sheet. The default button is filled with the accent and white text; others are control plates;
/// destructive titles are red. `pressed` darkens. `radius` is the corner (points): an alert's stacked buttons are
/// capsules, a free-form sheet's buttons 6 pt.
fn sheet_button(s: &mut dyn Canvas, ui: &Ui, r: Rect, b: &bifrost_ui::sheet::SheetButton, pressed: bool, radius: f32) {
    use bifrost_ui::sheet::ButtonRole;
    bifrost_ui::ax::record(bifrost_ui::Role::Button, &b.title, if b.role == ButtonRole::Default { "default" } else { "" }, r, true);
    let default = b.role == ButtonRole::Default;
    let fill = if default { crate::kit::appearance::accent_now() } else if pressed { CONTROL_PRESSED.now() } else { FIELD_BG.now() };
    s.fill_rrect(RectF::new(ui.pt(r.x), ui.pt(r.y), ui.pt(r.w), ui.pt(r.h)), ui.pt(radius), fill);
    if !default {
        s.stroke_rrect(RectF::new(ui.pt(r.x), ui.pt(r.y), ui.pt(r.w), ui.pt(r.h)), ui.pt(radius), ui.pt(1.0).max(1.0), HAIRLINE_C.now());
    }
    let color = if default { WHITE } else if b.role == ButtonRole::Destructive { crate::kit::ui::DESTRUCTIVE } else { LABEL.now() };
    let img = ui.text(&ui.fit(&b.title, MAC_BODY_PT, ui.pt(r.w - 8.)), MAC_BODY_PT, Weight::Regular, color);
    blit(s, &img, (ui.pt(r.x + r.w / 2.0) as i32) - img.w as i32 / 2, (ui.pt(r.y) + (ui.pt(r.h) - img.h as f32) / 2.0) as i32);
}

/// The lines an alert sheet draws at panel width `panel_w_pt`: its title, and each paragraph of its message (an empty
/// paragraph has no lines and is drawn as a blank line; an empty message has no paragraphs). Wrapped with real glyph
/// widths (`Ui::wrap_capped`, long words broken); `sheet` draws exactly these and sizes the panel from their count.
pub fn alert_lines(ui: &Ui, sheet: &bifrost_ui::sheet::Sheet, panel_w_pt: f32) -> (Vec<String>, Vec<Vec<String>>) {
    let tw = ui.pt(panel_w_pt - bifrost_ui::sheet::SHEET_PAD_PT * 2.0);
    let title = ui.wrap_capped(&sheet.title, bifrost_render::ui::MAC_HEADLINE_PT, Weight::Semibold, tw, usize::MAX);
    let message = if sheet.message.is_empty() {
        Vec::new()
    } else {
        sheet.message.split('\n').map(|para| ui.wrap_capped(para, MAC_CALLOUT_PT, Weight::Regular, tw, usize::MAX)).collect()
    };
    (title, message)
}

/// A sheet over `parent` (the window rectangle, points): dims and blocks the parent, then draws the panel attached to
/// the title bar (`titlebar_h` tall) at slide `progress` (0..1, from `motion::Kind::SheetSlide`): its top edge is the
/// title bar's bottom edge, square, with no gap, and it slides out from under the title bar
/// (`bifrost_ui::sheet::attached_body`). An alert stacks its buttons full width (`Sheet::layout`). Alerts draw their
/// title and message; free-form sheets draw the title and leave the area above the buttons to the caller, who gets the
/// content rectangle back. Return/Escape and click blocking: `bifrost_ui::sheet::Sheet::key` and `::hit`.
pub fn sheet(s: &mut dyn Canvas, ui: &Ui, parent: Rect, titlebar_h: f32, sheet: &bifrost_ui::sheet::Sheet, progress: f32, pressed: Option<u32>) -> Rect {
    use bifrost_ui::sheet::SHEET_PAD_PT;
    // an alert is sized from the wrap it is drawn with: the measured height is recorded before the layout, so the
    // panel, the text and the button hit areas (`Sheet::hit` reads the same record) agree
    let alert_text = sheet.alert.then(|| {
        let (title, message) = alert_lines(ui, sheet, sheet.size.0.min(parent.w));
        let message_lines = message.iter().map(|lines| lines.len().max(1)).sum::<usize>();
        let h = bifrost_ui::sheet::alert_sheet_height(title.len().max(1), message_lines, sheet.buttons.len());
        bifrost_ui::sheet::record_alert_height(&sheet.title, &sheet.message, sheet.size.0, h);
        (title, message)
    });
    let l = sheet.layout(parent, titlebar_h, progress);
    // the sheet is a window whose buttons are its children in the accessibility tree (closed on every return below)
    bifrost_ui::ax::begin_group(bifrost_ui::Role::Window, &sheet.title, l.panel);
    let dim = (0.28 * progress.clamp(0.0, 1.0) * 255.0) as u8;
    s.fill_rrect(RectF::new(ui.pt(parent.x), ui.pt(parent.y), ui.pt(parent.w), ui.pt(parent.h)), ui.pt(m::WINDOW_RADIUS), crate::kit::pack(0, 0, 0, dim));
    // attached to the title bar: everything is clipped to below its bottom edge, and the rounded body reaches above
    // the clip, so its top corners are cut off square and only the bottom corners are rounded
    let r = bifrost_ui::sheet::SHEET_RADIUS_PT;
    let Some((clip, body)) = bifrost_ui::sheet::attached_body(l.panel, parent, titlebar_h) else {
        bifrost_ui::ax::end_group();
        return l.panel;
    };
    s.push_clip(RectF::new(ui.pt(clip.x), ui.pt(clip.y), ui.pt(clip.w), ui.pt(clip.h)), 0.0);
    material(s, ui, body, r, Material::Popover);
    s.stroke_rrect(RectF::new(ui.pt(body.x), ui.pt(body.y), ui.pt(body.w), ui.pt(body.h)), ui.pt(r), ui.pt(1.0).max(1.0), HAIRLINE_C.now());
    let mut y = l.panel.y + SHEET_PAD_PT;
    let tw = ui.pt(l.panel.w - SHEET_PAD_PT * 2.0);
    let head_pt = bifrost_render::ui::MAC_HEADLINE_PT;
    let cx = ui.pt(l.panel.x + l.panel.w / 2.0) as i32;
    if let Some((title, message)) = &alert_text {
        use bifrost_ui::sheet::{ALERT_MESSAGE_LINE_PT, ALERT_TITLE_LINE_PT};
        // an alert's title and message wrap, centred, never cut off: the panel was sized for these very lines
        for (i, line) in title.iter().enumerate() {
            let head = ui.text(line, head_pt, Weight::Semibold, LABEL.now());
            blit(s, &head, cx - head.w as i32 / 2, ui.pt(y + i as f32 * ALERT_TITLE_LINE_PT) as i32);
        }
        y += 26.0 + title.len().saturating_sub(1) as f32 * ALERT_TITLE_LINE_PT;
        for lines in message {
            for line in lines {
                let t = ui.text(line, MAC_CALLOUT_PT, Weight::Regular, LABEL_2.now());
                blit(s, &t, cx - t.w as i32 / 2, ui.pt(y) as i32);
                y += ALERT_MESSAGE_LINE_PT;
            }
            // an empty paragraph is a blank line
            if lines.is_empty() {
                y += ALERT_MESSAGE_LINE_PT;
            }
        }
    } else {
        let head = ui.text(&ui.fit(&sheet.title, head_pt, tw), head_pt, Weight::Semibold, LABEL.now());
        blit(s, &head, ui.pt(l.panel.x + SHEET_PAD_PT) as i32, ui.pt(y) as i32);
        y += 26.0;
    }
    let radius = if sheet.alert { bifrost_ui::sheet::ALERT_BUTTON_H_PT / 2.0 } else { 6.0 };
    for (id, rect) in &l.buttons {
        if let Some(b) = sheet.buttons.iter().find(|b| b.id == *id) {
            sheet_button(s, ui, *rect, b, pressed == Some(*id), radius);
        }
    }
    let btn_top = l.buttons.first().map(|(_, r)| r.y).unwrap_or(l.panel.bottom() - SHEET_PAD_PT);
    s.pop_clip();
    bifrost_ui::ax::end_group();
    Rect { x: l.panel.x + SHEET_PAD_PT, y, w: l.panel.w - SHEET_PAD_PT * 2.0, h: (btn_top - 12.0 - y).max(0.0) }
}

/// A segmented control (`NativeSegmentedControl`): a rounded plate of `labels`, the `selected` segment on the raised
/// segment plate, hairline dividers between unselected neighbours. Returns the segment rectangles for
/// `bifrost_ui::sheet::segment_hit`. Draw `focus_ring` around `bounds` when it has keyboard focus.
pub fn segmented(s: &mut dyn Canvas, ui: &Ui, bounds: Rect, labels: &[&str], selected: usize, enabled: bool) -> Vec<Rect> {
    let segs = bifrost_ui::sheet::segments(bounds, labels.len());
    let (Some(first), Some(last)) = (segs.first(), segs.last()) else { return segs };
    let all = Rect { x: first.x, y: first.y, w: last.right() - first.x, h: first.h };
    bifrost_ui::ax::begin_group(bifrost_ui::Role::SegmentedControl, labels.get(selected).copied().unwrap_or(""), all);
    for (i, (r, label)) in segs.iter().zip(labels).enumerate() {
        bifrost_ui::ax::record(bifrost_ui::Role::Button, label, if i == selected { "selected" } else { "" }, *r, enabled);
    }
    bifrost_ui::ax::end_group();
    draw_segments(s, ui, &segs, all, labels, selected, enabled);
    segs
}

/// [`segmented`] without recording accessibility elements, for a view that publishes the control itself through its
/// `ax_node` (`view::TabView`), so the control is not listed twice.
pub(crate) fn segmented_unrecorded(s: &mut dyn Canvas, ui: &Ui, bounds: Rect, labels: &[&str], selected: usize, enabled: bool) -> Vec<Rect> {
    let segs = bifrost_ui::sheet::segments(bounds, labels.len());
    let (Some(first), Some(last)) = (segs.first(), segs.last()) else { return segs };
    let all = Rect { x: first.x, y: first.y, w: last.right() - first.x, h: first.h };
    draw_segments(s, ui, &segs, all, labels, selected, enabled);
    segs
}

fn draw_segments(s: &mut dyn Canvas, ui: &Ui, segs: &[Rect], all: Rect, labels: &[&str], selected: usize, enabled: bool) {
    s.fill_rrect(RectF::new(ui.pt(all.x), ui.pt(all.y), ui.pt(all.w), ui.pt(all.h)), ui.pt(7.), CAPSULE_BG.now());
    s.stroke_rrect(RectF::new(ui.pt(all.x), ui.pt(all.y), ui.pt(all.w), ui.pt(all.h)), ui.pt(7.), ui.pt(1.0).max(1.0), CAPSULE_EDGE.now());
    for (i, (r, label)) in segs.iter().zip(labels).enumerate() {
        if i == selected {
            s.fill_rrect(RectF::new(ui.pt(r.x + 2.), ui.pt(r.y + 2.), ui.pt(r.w - 4.), ui.pt(r.h - 4.)), ui.pt(5.), SEGMENT_ON.now());
        } else if i > 0 && i != selected + 1 {
            s.fill_rect(RectF::new((ui.pt(r.x) as usize) as f32, (ui.pt(r.y + 5.) as usize) as f32, (ui.pt(1.0).max(1.0) as usize) as f32, (ui.pt(r.h - 10.) as usize) as f32), SEPARATOR_C.now());
        }
        let color = if enabled { LABEL.now() } else { LABEL_3.now() };
        let img = ui.text(&ui.fit(label, MAC_BODY_PT, ui.pt(r.w - 8.)), MAC_BODY_PT, Weight::Regular, color);
        blit(s, &img, (ui.pt(r.x + r.w / 2.0) as i32) - img.w as i32 / 2, (ui.pt(r.y) + (ui.pt(r.h) - img.h as f32) / 2.0) as i32);
    }
}

/// A stepper (`NativeStepper`): a 13 x 22 plate with an up and a down chevron. `pressed` darkens one half.
/// Returns its rectangle; hit-test with `bifrost_ui::sheet::stepper_hit`.
pub fn stepper(s: &mut dyn Canvas, ui: &Ui, origin: (f32, f32), pressed: Option<bifrost_ui::sheet::StepperPart>, enabled: bool) -> Rect {
    use bifrost_ui::sheet::StepperPart;
    let r = bifrost_ui::sheet::stepper_rect(origin);
    bifrost_ui::ax::record(bifrost_ui::Role::Button, "Stepper", "", r, enabled);
    s.fill_rrect(RectF::new(ui.pt(r.x), ui.pt(r.y), ui.pt(r.w), ui.pt(r.h)), ui.pt(5.), FIELD_BG.now());
    if let Some(p) = pressed {
        let hy = if p == StepperPart::Up { r.y } else { r.y + r.h / 2.0 };
        s.fill_rrect(RectF::new(ui.pt(r.x), ui.pt(hy), ui.pt(r.w), ui.pt(r.h / 2.0)), ui.pt(3.), CONTROL_PRESSED.now());
    }
    s.stroke_rrect(RectF::new(ui.pt(r.x), ui.pt(r.y), ui.pt(r.w), ui.pt(r.h)), ui.pt(5.), ui.pt(1.0).max(1.0), HAIRLINE_C.now());
    let color = if enabled { LABEL.now() } else { LABEL_3.now() };
    let row = ui.pt(1.0).max(1.0);
    let cx = ui.pt(r.x + r.w / 2.);
    for k in 0..3 {
        let half = (k as f32 + 0.5) * row;
        s.fill_rrect(RectF::new(cx - half, ui.pt(r.y + 5.5) + k as f32 * row, half * 2.0, row), 0.0, color);
        s.fill_rrect(RectF::new(cx - half, ui.pt(r.y + r.h - 5.5) - (k as f32 + 1.0) * row, half * 2.0, row), 0.0, color);
    }
    r
}

/// A mini month calendar (`NativeDatePicker`, graphical style; the Calendar sidebar's month): the month title with
/// previous/next chevrons, weekday initials, and the days. `today` gets the red marker (the calendar's one other red),
/// `selected` a neutral plate, and days in `marked` a small dot under them (days with events). Returns the layout for
/// `hit_day`, `prev` and `next`. `month_title` is the app's localised "October 2026".
#[allow(clippy::too_many_arguments)]
pub fn mini_month(s: &mut dyn Canvas, ui: &Ui, bounds: Rect, year: i32, month: u32, first_weekday: u32, month_title: &str, today: Option<u32>, selected: Option<u32>, marked: &[u32]) -> bifrost_ui::calendar::MiniMonth {
    let l = bifrost_ui::calendar::MiniMonth::new(bounds, year, month, first_weekday);
    bifrost_ui::ax::begin_group(bifrost_ui::Role::List, month_title, bounds);
    let mid = |r: &Rect, img: &crate::kit::Image| ((ui.pt(r.x + r.w / 2.0) as i32) - img.w as i32 / 2, (ui.pt(r.y + r.h / 2.0) as i32) - img.h as i32 / 2);
    let t = ui.text(&ui.fit(month_title, MAC_CALLOUT_PT, ui.pt(l.title.w)), MAC_CALLOUT_PT, Weight::Semibold, LABEL.now());
    blit(s, &t, ui.pt(l.title.x + SPACE_XS) as i32, (ui.pt(l.title.y + l.title.h / 2.0) as i32) - t.h as i32 / 2);
    for (r, name, label) in [(l.prev, "chevron.left", "Previous Month"), (l.next, "chevron.right", "Next Month")] {
        bifrost_ui::ax::record(bifrost_ui::Role::Button, label, "", r, true);
        let g = 10.0;
        glyph(s, ui, name, Rect { x: r.x + (r.w - g) / 2.0, y: r.y + (r.h - g) / 2.0, w: g, h: g }, LABEL_2.now());
    }
    const INITIALS: [&str; 7] = ["S", "M", "T", "W", "T", "F", "S"];
    for (c, r) in l.weekdays.iter().enumerate() {
        let img = ui.text(INITIALS[l.weekday_of_column[c] as usize], MAC_FOOTNOTE_PT, Weight::Regular, LABEL_2.now());
        let (x, y) = mid(r, &img);
        blit(s, &img, x, y);
    }
    for (d, r) in &l.days {
        let side = r.w.min(r.h) - 4.0;
        let plate = Rect { x: r.x + (r.w - side) / 2.0, y: r.y + (r.h - side) / 2.0, w: side, h: side };
        let is_today = today == Some(*d);
        let state = match (is_today, selected == Some(*d)) {
            (true, true) => "today, selected",
            (true, false) => "today",
            (false, true) => "selected",
            _ => "",
        };
        bifrost_ui::ax::record(bifrost_ui::Role::Button, &d.to_string(), state, *r, true);
        if is_today {
            s.fill_rrect(RectF::new(ui.pt(plate.x), ui.pt(plate.y), ui.pt(side), ui.pt(side)), ui.pt(side / 2.0), bifrost_render::ui::DESTRUCTIVE);
        } else if selected == Some(*d) {
            s.fill_rrect(RectF::new(ui.pt(plate.x), ui.pt(plate.y), ui.pt(side), ui.pt(side)), ui.pt(side / 2.0), SIDEBAR_PLATE.now());
        }
        let img = ui.text(&d.to_string(), MAC_FOOTNOTE_PT, if is_today { Weight::Semibold } else { Weight::Regular }, if is_today { WHITE } else { LABEL.now() });
        let (x, y) = mid(r, &img);
        blit(s, &img, x, y);
        if marked.contains(d) && !is_today {
            crate::glyphs::disc(s, ui.pt(r.x + r.w / 2.0), ui.pt(plate.bottom() - 2.0), ui.pt(1.5), LABEL_2.now());
        }
    }
    // the months around it, greyed (secondary label), through the sixth row; the app hit-tests them with `hit_date`
    for (d, r) in &l.outside {
        let img = ui.text(&d.day.to_string(), MAC_FOOTNOTE_PT, Weight::Regular, LABEL_2.now());
        let (x, y) = mid(r, &img);
        blit(s, &img, x, y);
    }
    bifrost_ui::ax::end_group();
    l
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stripes(w: usize, h: usize) -> Vec<u32> {
        (0..w * h).map(|i| if (i % w) / 4 % 2 == 0 { pack(255, 255, 255, 255) } else { pack(0, 0, 0, 255) }).collect()
    }

    #[test]
    fn a_material_blurs_and_tints_the_content_behind_it() {
        let (w, h) = (64, 64);
        let mut px = stripes(w, h);
        let before = px.clone();
        let data: &'static [u8] = Box::leak(std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../assets/SFUI.ttf")).unwrap().into_boxed_slice());
        let ui = Ui::new(crate::kit::Font::parse(data).unwrap(), 1.0);
        material(&mut Surface { w, h, stride: w, px: &mut px, y0: 0 }, &ui, Rect { x: 8.0, y: 8.0, w: 48.0, h: 48.0 }, 6.0, Material::Titlebar);
        assert_eq!(px[0], before[0], "outside the material nothing changes");
        let mid = px[32 * w + 32];
        assert!(mid != pack(255, 255, 255, 255) && mid != pack(0, 0, 0, 255), "stripes are blurred away: {mid:08x}");
        let (r, g, b) = ((mid >> 16) & 255, (mid >> 8) & 255, mid & 255);
        assert!(r < 128 && g < 128 && b < 128, "the dark tint pulls the blurred mid-grey down");
    }

    #[test]
    fn titles_and_labels_draw_inside_the_readable_column_and_advance() {
        let (w, h) = (1200usize, 300usize);
        let data: &'static [u8] = Box::leak(std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../assets/SFUI.ttf")).unwrap().into_boxed_slice());
        let ui = Ui::new(crate::kit::Font::parse(data).unwrap(), 1.0);
        let l = Layout::from_points(w as f32, h as f32, 0.0, 0.0);
        let mut px = vec![0u32; w * h];
        let mut surf = Surface { w, h, stride: w, px: &mut px, y0: 0 };
        let below = large_title(&mut surf, &ui, &l, "Reminders", 20.0);
        let end = section_label(&mut surf, &ui, &l, "My Lists", below);
        assert!(below > 20.0 + MAC_LARGE_TITLE_PT && end > below, "content advances below each element");
        let (x, rw) = l.readable();
        let inked: Vec<usize> = (0..w * h).filter(|&i| px[i] != 0).map(|i| i % w).collect();
        assert!(!inked.is_empty());
        assert!(inked.iter().all(|&c| c as f32 >= x - 1.0 && (c as f32) <= x + rw), "ink stays inside the readable column");
    }

    #[test]
    fn show_borders_outlines_a_control_only_when_the_setting_is_on() {
        let data: &'static [u8] = Box::leak(std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../assets/SFUI.ttf")).unwrap().into_boxed_slice());
        let ui = Ui::new(crate::kit::Font::parse(data).unwrap(), 1.0);
        let (w, h) = (60usize, 40usize);
        let draw = |on: bool| {
            let mut a = crate::kit::appearance::current();
            a.button_shapes = on;
            crate::kit::appearance::publish(a);
            let mut px = vec![0u32; w * h];
            outline_if_borders(&mut Surface { w, h, stride: w, px: &mut px, y0: 0 }, &ui, Rect { x: 10.0, y: 10.0, w: 40.0, h: 20.0 }, 4.0);
            px
        };
        let (off, on) = (draw(false), draw(true));
        assert!(off.iter().all(|&p| p == 0), "nothing is drawn with the setting off");
        assert!(on.iter().any(|&p| p != 0), "an outline appears with the setting on");
        assert_eq!(on[20 * w + 30], 0, "the inside of the control stays untouched");
        assert!(on[10 * w + 30] != 0, "the top edge is outlined");
        draw(false);
    }

    #[test]
    fn every_material_draws_behind_the_window_as_its_own_kind() {
        let kinds: std::collections::HashSet<_> = Material::ALL.iter().map(|m| m.kind() as u8).collect();
        assert_eq!(kinds.len(), Material::ALL.len());
        for m in Material::ALL {
            assert!(m.tint() >> 24 < 255, "{m:?} is translucent");
            assert_eq!(m.solid() >> 24, 255, "{m:?} has an opaque fallback");
        }
    }

    #[test]
    fn a_behind_window_effect_writes_its_tint_and_declares_kind_and_state() {
        let ui = test_ui();
        let (w, h) = (40, 40);
        let mut px = vec![pack(9, 9, 9, 255); w * h];
        set_vibrancy(true, true);
        let _ = take_frame_materials();
        let b = Rect { x: 4.0, y: 4.0, w: 30.0, h: 30.0 };
        visual_effect(&mut Surface { w, h, stride: w, px: &mut px, y0: 0 }, &ui, b, 8.0, Material::Popover, Blending::BehindWindow, State::Active);
        let got = take_frame_materials();
        // the shared request carries per-corner radii since the material reconcile (wire version 2)
        assert_eq!(got, vec![{
            let mut m = crate::proto::Material::new(crate::proto::MaterialKind::Popover, b, 8.0);
            m.state = State::Active;
            m.corners = Some(crate::proto::Corners { tl: 8.0, tr: 8.0, br: 8.0, bl: 8.0 });
            m
        }]);
        assert_eq!(px[20 * w + 20], Material::Popover.tint(), "the inside is replaced by the translucent tint");
        assert_eq!(px[4 * w + 4], pack(9, 9, 9, 255), "outside the rounded corner the content stays");
        assert_eq!(px[0], pack(9, 9, 9, 255));
        set_vibrancy(false, false);
    }

    #[test]
    fn an_older_shell_gets_legacy_kinds_and_the_default_state() {
        let ui = test_ui();
        let mut px = vec![0; 32 * 32];
        set_vibrancy(true, false);
        let _ = take_frame_materials();
        visual_effect(&mut Surface { w: 32, h: 32, stride: 32, px: &mut px, y0: 0 }, &ui, Rect { x: 0.0, y: 0.0, w: 32.0, h: 10.0 }, 0.0, Material::HeaderView, Blending::BehindWindow, State::Inactive);
        let got = take_frame_materials();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].kind, crate::proto::MaterialKind::Titlebar);
        assert_eq!(got[0].state, State::FollowsWindowActiveState);
        set_vibrancy(false, false);
    }

    #[test]
    fn without_vibrancy_a_behind_window_effect_is_opaque_and_undeclared() {
        let ui = test_ui();
        let mut px = vec![0; 32 * 32];
        set_vibrancy(false, false);
        let _ = take_frame_materials();
        visual_effect(&mut Surface { w: 32, h: 32, stride: 32, px: &mut px, y0: 0 }, &ui, Rect { x: 0.0, y: 0.0, w: 32.0, h: 32.0 }, 0.0, Material::Sidebar, Blending::BehindWindow, State::default());
        assert!(take_frame_materials().is_empty());
        assert_eq!(px[16 * 32 + 16], Material::Sidebar.solid());
    }

    #[test]
    fn a_within_window_effect_is_never_declared_to_the_shell() {
        let ui = test_ui();
        let mut px = stripes(32, 32);
        set_vibrancy(true, true);
        let _ = take_frame_materials();
        visual_effect(&mut Surface { w: 32, h: 32, stride: 32, px: &mut px, y0: 0 }, &ui, Rect { x: 0.0, y: 0.0, w: 32.0, h: 32.0 }, 4.0, Material::Hud, Blending::WithinWindow, State::default());
        assert!(take_frame_materials().is_empty());
        set_vibrancy(false, false);
    }

    #[test]
    fn vibrant_text_adds_light_in_dark_and_removes_it_in_light_keeping_alpha() {
        let tint = rgba(24, 24, 27, 190);
        let dark = vibrant_px(tint, 0.55, false);
        assert_eq!(dark >> 24, tint >> 24, "the material's alpha is kept, so the backdrop shows through the text");
        assert!((dark & 255) > (tint & 255) + 130);
        let tint_light = rgba(236, 236, 236, 200);
        let light = vibrant_px(tint_light, 0.5, true);
        assert_eq!(light >> 24, tint_light >> 24);
        assert!((light & 255) + 120 < (tint_light & 255));
        assert_eq!(vibrant_px(pack(5, 5, 5, 255), 1.0, true) & 0xff_ffff, 0, "clamped at black");
        assert_eq!(vibrant_px(pack(250, 250, 250, 255), 1.0, false) & 0xff_ffff, 0xff_ffff, "clamped at white");
    }

    #[test]
    fn vibrant_levels_weaken_from_label_to_quaternary() {
        let k: Vec<f32> = [Vibrant::Label, Vibrant::Secondary, Vibrant::Tertiary, Vibrant::Quaternary].iter().map(|v| v.strength()).collect();
        assert!(k.windows(2).all(|w| w[0] > w[1]), "{k:?}");
    }

    #[test]
    fn a_vibrant_source_list_keeps_its_material_alpha_under_the_text() {
        let ui = test_ui();
        let (w, h) = (200, 120);
        let mut px = vec![pack(9, 9, 9, 255); w * h];
        set_vibrancy(true, true);
        let list = SourceList::new(vec![SidebarItem::header("Favorites"), SidebarItem::row(1, "Desktop").with_count(4)], 28.0, 22.0);
        source_list(&mut Surface { w, h, stride: w, px: &mut px, y0: 0 }, &ui, Rect { x: 0.0, y: 0.0, w: 200.0, h: 120.0 }, &list, 0.0, None);
        let _ = take_frame_materials();
        set_vibrancy(false, false);
        let tint_a = Material::Sidebar.tint() >> 24;
        assert!(px.iter().all(|p| p >> 24 == tint_a), "text blends into the material instead of covering it");
        assert!(px.iter().any(|p| (p & 255) > (Material::Sidebar.tint() & 255) + 40), "the labels are drawn");
    }

    #[test]
    fn a_menu_publishes_its_rows_as_children_and_skips_separators() {
        use bifrost_ui::menu::{Menu, MenuItem};
        let ui = test_ui();
        let (w, h) = (200usize, 120usize);
        let mut px = vec![0u32; w * h];
        let m = Menu::new(vec![MenuItem::new(1, "Copy"), MenuItem::separator(), MenuItem::new(2, "Paste").enabled(false).checked(true)]);
        bifrost_ui::ax::begin_frame();
        menu(&mut Surface { w, h, stride: w, px: &mut px, y0: 0 }, &ui, (0.0, 0.0), 180.0, &m);
        let links = bifrost_ui::ax::take_links();
        let nodes = bifrost_ui::ax::take_frame();
        let list = nodes.iter().find(|n| n.role == bifrost_ui::Role::List && n.label == "Menu").expect("the menu is an element").id;
        let rows: Vec<&bifrost_ui::Node> = nodes.iter().filter(|n| bifrost_ui::ax::parent_of(&links, n.id) == Some(list)).collect();
        assert_eq!(rows.iter().map(|n| n.label.as_str()).collect::<Vec<_>>(), vec!["Copy", "Paste"]);
        assert!(rows.iter().all(|n| n.role == bifrost_ui::Role::MenuItem));
        assert!(!rows[1].enabled && rows[1].value == "checked");
    }

    #[test]
    fn a_segmented_control_holds_one_button_per_segment() {
        let ui = test_ui();
        let (w, h) = (300usize, 40usize);
        let mut px = vec![0u32; w * h];
        bifrost_ui::ax::begin_frame();
        segmented(&mut Surface { w, h, stride: w, px: &mut px, y0: 0 }, &ui, Rect { x: 0.0, y: 0.0, w: 240.0, h: 24.0 }, &["Day", "Week", "Month"], 1, true);
        let links = bifrost_ui::ax::take_links();
        let nodes = bifrost_ui::ax::take_frame();
        let group = nodes.iter().find(|n| n.role == bifrost_ui::Role::SegmentedControl).expect("the control").id;
        let kids: Vec<(&str, &str)> = nodes.iter().filter(|n| bifrost_ui::ax::parent_of(&links, n.id) == Some(group)).map(|n| (n.label.as_str(), n.value.as_str())).collect();
        assert_eq!(kids, vec![("Day", ""), ("Week", "selected"), ("Month", "")]);
    }

    /// E2E10: a sidebar whose rows are clipped below the traffic lights' band is drawn in one pass and is one Sidebar
    /// element holding its rows (Dictionary drew an empty list for the band first, a second Sidebar element).
    #[test]
    fn a_source_list_clipped_below_the_band_is_one_sidebar_element() {
        use bifrost_ui::sidebar::{SidebarItem, SourceList};
        let ui = test_ui();
        let (w, h) = (200usize, 300usize);
        let mut px = vec![0u32; w * h];
        let list = SourceList::new(vec![SidebarItem::row(0, "orange"), SidebarItem::row(1, "orbit")], 28.0, 22.0);
        let bounds = Rect { x: 0.0, y: 0.0, w: 200.0, h: 300.0 };
        let rows = Rect { y: 52.0, h: 248.0, ..bounds };
        bifrost_ui::ax::begin_frame();
        source_list_in(&mut Surface { w, h, stride: w, px: &mut px, y0: 0 }, &ui, bounds, &list, 0.0, None, Some(rows));
        let links = bifrost_ui::ax::take_links();
        let nodes = bifrost_ui::ax::take_frame();
        let sidebars: Vec<u64> = nodes.iter().filter(|n| n.role == bifrost_ui::Role::List && n.label == "Sidebar").map(|n| n.id).collect();
        assert_eq!(sidebars.len(), 1, "one Sidebar element");
        let kids: Vec<&str> = nodes.iter().filter(|n| bifrost_ui::ax::parent_of(&links, n.id) == Some(sidebars[0])).map(|n| n.label.as_str()).collect();
        assert_eq!(kids, vec!["orange", "orbit"]);
    }

    /// E2E11: under the rows clip an unselected row's vibrant label still has ink (Dictionary 0.6.5 showed only the
    /// selected row: the keep-alpha blend ran on an empty scratch and vanished).
    #[test]
    fn a_vibrant_source_list_draws_its_unselected_rows_inside_the_rows_clip() {
        let ui = test_ui();
        let (w, h) = (200usize, 200usize);
        let mut px = vec![pack(9, 9, 9, 255); w * h];
        set_vibrancy(true, true);
        let mut list = SourceList::new(vec![SidebarItem::row(0, "orange"), SidebarItem::row(1, "orbit"), SidebarItem::row(2, "ordeal")], 28.0, 22.0);
        list.set_selected(Some(0));
        let bounds = Rect { x: 0.0, y: 0.0, w: 200.0, h: 200.0 };
        let rows = Rect { y: 52.0, h: 148.0, ..bounds };
        source_list_in(&mut Surface { w, h, stride: w, px: &mut px, y0: 0 }, &ui, bounds, &list, -60.0, None, Some(rows));
        let _ = take_frame_materials();
        set_vibrancy(false, false);
        let tint = Material::Sidebar.tint();
        for i in [1usize, 2] {
            let r = list.item_rect(i, bounds, -60.0);
            assert!(r.y >= rows.y && r.bottom() <= rows.bottom(), "row {i} is inside the clip: {r:?}");
            let ink = (r.y as usize..r.bottom() as usize).flat_map(|y| (0..w).map(move |x| y * w + x)).filter(|&k| (px[k] & 255) > (tint & 255) + 40).count();
            assert!(ink > 10, "row {i} has ink under the clip ({ink} px)");
            assert!((r.y as usize..r.bottom() as usize).all(|y| (0..w).all(|x| px[y * w + x] >> 24 == tint >> 24)), "row {i} keeps the material alpha");
        }
    }

    fn test_ui() -> Ui {
        let data: &'static [u8] = Box::leak(std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../assets/SFUI.ttf")).unwrap().into_boxed_slice());
        Ui::new(crate::kit::Font::parse(data).unwrap(), 1.0)
    }

    fn bar() -> Toolbar {
        use bifrost_ui::nav::ToolbarItem;
        Toolbar { items: vec![ToolbarItem::button(1, "Back"), ToolbarItem::button(2, "Forward"), ToolbarItem::flexible_space(), ToolbarItem::button(3, "Share"), ToolbarItem::button(4, "Add")] }
    }

    #[test]
    fn the_band_fills_its_width_and_the_toolbar_reports_button_rects_inside_it() {
        let ui = test_ui();
        let (w, h) = (900usize, 120usize);
        let band = Rect { x: 0.0, y: 10.0, w: w as f32, h: 52.0 };
        let mut px = vec![0u32; w * h];
        let mut s = Surface { w, h, stride: w, px: &mut px, y0: 0 };
        toolbar_band(&mut s, &ui, band);
        let placed = toolbar(&mut s, &ui, band, &bar(), None);
        assert_eq!(px[5 * w + 5], 0, "above the band nothing is drawn");
        assert_eq!(px[(10 + 3) * w + 450], DETAIL_BG.now(), "the band is the content colour");
        assert_eq!(px[(10 + 51) * w + 450], SEPARATOR_C.now(), "a hairline under the band");
        assert!(placed.overflow.is_empty() && placed.visible.len() == 4);
        for (_, r) in &placed.visible {
            assert!(r.y >= 10.0 && r.bottom() <= 62.0 && r.right() <= w as f32, "{r:?}");
            assert_eq!(r.h, CAPSULE_PT);
            let (cx, cy) = ((r.x + r.w / 2.0) as usize, (r.y + 2.0) as usize);
            assert_eq!(px[cy * w + cx], CAPSULE_BG.now(), "a button is a capsule");
        }
    }

    #[test]
    fn glyph_groups_share_a_capsule_and_the_selected_segment_has_a_plate() {
        use bifrost_ui::nav::ToolbarItem;
        let ui = test_ui();
        let (w, h) = (400usize, 60usize);
        let t = Toolbar::new(vec![ToolbarItem::icon(1, "Icons", "square.grid.2x2").in_group(1).selected(true), ToolbarItem::icon(2, "List", "list.bullet").in_group(1)]);
        let mut px = vec![0u32; w * h];
        let placed = toolbar(&mut Surface { w, h, stride: w, px: &mut px, y0: 0 }, &ui, Rect { x: 10.0, y: 10.0, w: 380.0, h: 40.0 }, &t, None);
        assert_eq!(placed.groups.len(), 1);
        let (a, b) = (placed.rect_of(1).unwrap(), placed.rect_of(2).unwrap());
        assert_eq!(a.right(), b.x);
        let mid = (a.y + a.h / 2.0) as usize;
        assert_eq!(px[mid * w + (a.x + 5.0) as usize], SEGMENT_ON.now(), "selected segment plate");
        assert_eq!(px[mid * w + (b.x + 5.0) as usize], CAPSULE_BG.now(), "the other segment is the capsule");
    }

    #[test]
    fn a_narrow_window_puts_buttons_behind_the_overflow_button_and_pressed_buttons_darken() {
        let ui = test_ui();
        let (w, h) = (320usize, 80usize);
        let mut px = vec![0u32; w * h];
        let placed = toolbar(&mut Surface { w, h, stride: w, px: &mut px, y0: 0 }, &ui, Rect { x: 100.0, y: 0.0, w: 200.0, h: 40.0 }, &bar(), Some(1));
        assert!(!placed.overflow.is_empty() && placed.overflow_button.is_some());
        let first = placed.visible[0].1;
        assert_eq!(px[(first.y + 4.0) as usize * w + (first.x + first.w / 2.0) as usize], ROW_PRESSED.now(), "the pressed button has a plate");
    }

    #[test]
    fn the_source_list_shades_the_whole_column_and_puts_a_neutral_plate_under_the_selection() {
        let ui = test_ui();
        let (w, h) = (400usize, 300usize);
        let mut list = SourceList::new(vec![SidebarItem::header("Library"), SidebarItem::row(1, "Today").with_count(3).with_symbol("clock"), SidebarItem::row(2, "All").with_symbol("house")], 28.0, 22.0);
        list.select(2);
        let col = Rect { x: 20.0, y: 10.0, w: 180.0, h: 250.0 };
        let mut px = vec![0u32; w * h];
        source_list(&mut Surface { w, h, stride: w, px: &mut px, y0: 0 }, &ui, col, &list, 0.0, None);
        assert_eq!(px[5 * w + 50], 0, "above the column nothing is drawn");
        assert_eq!(px[(10 + 240) * w + 30], SIDEBAR_BG.now(), "the shaded column runs to its bottom edge, below the last row");
        assert_eq!(px[(10 + 22 + 28 + 14) * w + 150], SIDEBAR_PLATE.now(), "the selected row is a neutral plate");
        assert_eq!(px[(10 + 22 + 6) * w + 150], SIDEBAR_BG.now(), "the unselected row stays in the column colour");
        assert!((10 + 22..10 + 50).any(|y| (36..52).any(|x| px[y * w + x] != SIDEBAR_BG.now())), "the row glyph is drawn");
        assert_eq!(px[50 * w + 250], 0, "right of the column nothing is drawn");
    }

    #[test]
    fn a_neutral_sidebar_row_draws_its_glyph_in_the_secondary_grey_not_the_accent() {
        let ui = test_ui();
        let (w, h) = (400usize, 300usize);
        let col = Rect { x: 20.0, y: 10.0, w: 180.0, h: 250.0 };
        let draw = |row: SidebarItem| {
            let list = SourceList::new(vec![SidebarItem::header("Locations"), row], 28.0, 22.0);
            let mut px = vec![0u32; w * h];
            source_list(&mut Surface { w, h, stride: w, px: &mut px, y0: 0 }, &ui, col, &list, 0.0, None);
            px
        };
        // packed 0xAABBGGRR: a pixel is blue when its blue byte is well above its red byte (a grey has them near equal)
        let bluish = |px: &[u32]| (10 + 22..10 + 50).any(|y| (30..70).any(|x| (px[y * w + x] >> 16) & 255 > (px[y * w + x] & 255) + 60));
        assert!(bluish(&draw(SidebarItem::row(1, "Desktop").with_symbol("house"))), "an accent glyph is blue");
        assert!(!bluish(&draw(SidebarItem::row(1, "Home").with_symbol("house").neutral())), "a neutral glyph is grey");
    }

    #[test]
    fn the_folder_emblem_is_clearly_darker_than_the_folder_body() {
        // the body gradient runs from (88, 170, 207) to (74, 178, 227); luminance on the 0..255 scale
        let luma = |r: f32, g: f32, b: f32| 0.2126 * r + 0.7152 * g + 0.0722 * b;
        let emblem = luma((FOLDER_EMBLEM & 255) as f32, ((FOLDER_EMBLEM >> 8) & 255) as f32, ((FOLDER_EMBLEM >> 16) & 255) as f32);
        for body in [luma(88.0, 170.0, 207.0), luma(74.0, 178.0, 227.0)] {
            assert!(body - emblem >= 36.0, "the glyph reads on the body: body {body:.0}, glyph {emblem:.0}");
        }
        let hilite = luma((FOLDER_EMBLEM_HILITE & 255) as f32, ((FOLDER_EMBLEM_HILITE >> 8) & 255) as f32, ((FOLDER_EMBLEM_HILITE >> 16) & 255) as f32);
        assert!(hilite > emblem, "the emboss line under the glyph is the light one");
    }

    #[test]
    fn a_search_field_shows_the_magnifier_and_a_clear_button_only_with_text() {
        let ui = test_ui();
        let (w, h) = (240usize, 40usize);
        let b = Rect { x: 10.0, y: 6.0, w: 200.0, h: 28.0 };
        let draw = |text: &str| {
            let mut f = TextField::new();
            f.push_str(text);
            let mut px = vec![0u32; w * h];
            search_field(&mut Surface { w, h, stride: w, px: &mut px, y0: 0 }, &ui, &f, b, "Search", false);
            px
        };
        let (empty, full) = (draw(""), draw("aqua"));
        let clear = bifrost_ui::search::layout(b, true).clear.unwrap();
        let inked = |px: &[u32], r: Rect| (r.y as usize..r.bottom() as usize).any(|y| (r.x as usize..r.right() as usize).any(|x| px[y * w + x] != FIELD_BG.now()));
        assert!(inked(&empty, bifrost_ui::search::layout(b, false).magnifier), "magnifier");
        assert!(!inked(&empty, clear), "no clear button when empty");
        assert!(inked(&full, clear), "clear button with text");
        assert_eq!(empty[(b.y + 2.0) as usize * w + 100], FIELD_BG.now(), "the capsule is the field colour");
    }

    #[test]
    fn a_path_bar_lays_out_segments_with_glyphs() {
        let ui = test_ui();
        let (w, h) = (400usize, 30usize);
        let mut px = vec![0u32; w * h];
        let segs = path_bar(&mut Surface { w, h, stride: w, px: &mut px, y0: 0 }, &ui, Rect { x: 10.0, y: 5.0, w: 380.0, h: 20.0 }, &[("Home", "house"), ("Documents", "folder"), ("a.txt", "doc")]);
        assert_eq!(segs.len(), 3);
        assert!(segs.windows(2).all(|p| p[1].rect.x > p[0].rect.right()), "chevrons between");
        assert!(px.iter().any(|p| *p != 0));
    }

    #[test]
    fn a_popup_button_draws_its_plate_title_and_chevrons_and_darkens_when_open() {
        let ui = test_ui();
        let (w, h) = (200usize, 40usize);
        let draw = |open: bool| {
            let mut px = vec![0u32; w * h];
            popup_button(&mut Surface { w, h, stride: w, px: &mut px, y0: 0 }, &ui, Rect { x: 10.0, y: 8.0, w: 160.0, h: 22.0 }, "Automatic", open, true);
            px
        };
        let (closed, open) = (draw(false), draw(true));
        let plate = closed[19 * w + 140];
        assert_eq!(plate, FIELD_BG.now(), "the plate is the control fill");
        assert_ne!(closed[10 * w + 90], 0, "the plate is filled");
        assert_ne!(closed[10 * w + 90], open[10 * w + 90], "open is pressed");
        assert!((12..18).any(|y| (148..164).any(|x| closed[y * w + x] != plate)), "up chevron at the trailing edge");
        assert!((21..27).any(|y| (148..164).any(|x| closed[y * w + x] != plate)), "down chevron at the trailing edge");
        assert_eq!(closed[2 * w + 90], 0, "nothing outside the button");
    }

    #[test]
    fn an_open_menu_marks_the_highlight_the_check_and_separators() {
        use bifrost_ui::menu::{Menu, MenuItem};
        let ui = test_ui();
        let (w, h) = (260usize, 160usize);
        let mut m = Menu::popup(vec![MenuItem::new(1, "Automatic"), MenuItem::new(2, "Intel HD Graphics").shortcut("\u{2318}1"), MenuItem::separator(), MenuItem::new(3, "Disabled").enabled(false)], 0);
        m.highlighted = Some(1);
        let mut px = vec![0u32; w * h];
        let r = menu(&mut Surface { w, h, stride: w, px: &mut px, y0: 0 }, &ui, (10.0, 10.0), 220.0, &m);
        let (rows, total) = m.rows(MENU_ROW_PT, MENU_SEPARATOR_PT, MENU_PAD_PT);
        assert_eq!(r.h, total);
        let hot_y = (10.0 + rows[1].0 + 2.0) as usize;
        assert_eq!(px[hot_y * w + 40], TINT, "the highlighted row is the accent colour");
        let sep_y = (10.0 + rows[2].0 + rows[2].1 / 2.0) as usize;
        assert_eq!(px[sep_y * w + 60], SEPARATOR_C.now(), "separator hairline");
        assert_eq!(px[(r.y + r.h + 4.0) as usize * w + 100], 0, "nothing below the menu");
    }

    #[test]
    fn a_menu_of_colours_draws_a_round_swatch_before_each_title() {
        use bifrost_ui::menu::{Menu, MenuItem};
        let ui = test_ui();
        let (w, h) = (260usize, 120usize);
        let (green, blue) = (0xff34_c759u32, 0xff00_7affu32);
        let m = Menu::popup(vec![MenuItem::new(1, "Home").swatch(green), MenuItem::new(2, "Work").swatch(blue)], 0);
        let mut px = vec![0u32; w * h];
        menu(&mut Surface { w, h, stride: w, px: &mut px, y0: 0 }, &ui, (10.0, 10.0), 220.0, &m);
        let (rows, _) = m.rows(MENU_ROW_PT, MENU_SEPARATOR_PT, MENU_PAD_PT);
        // the swatch's centre, after the check mark column
        let at = |row: usize| {
            let x = ui.pt(10.0 + MENU_INSET_PT + 24.0 + MENU_SWATCH_PT / 2.0) as usize;
            let y = ui.pt(10.0 + rows[row].0 + rows[row].1 / 2.0) as usize;
            px[y * w + x]
        };
        assert_eq!(at(0), green);
        assert_eq!(at(1), blue);
    }

    #[test]
    fn materials_get_denser_towards_popover() {
        let a = |m: Material| m.tint() >> 24;
        assert!(a(Material::Popover) > a(Material::Titlebar) && a(Material::Titlebar) > a(Material::Hud));
    }

    #[test]
    fn toolbar_layout_matches_what_toolbar_draws_and_menu_items_are_wider() {
        use bifrost_ui::nav::ToolbarItem;
        let ui = test_ui();
        let t = Toolbar::new(vec![ToolbarItem::icon(1, "Add", "plus"), ToolbarItem::menu(2, "Sort", Some("arrow.up.arrow.down")), ToolbarItem::flexible_space(), ToolbarItem::button(3, "Share").centered()]);
        let band = Rect { x: 100.0, y: 0.0, w: 500.0, h: 52.0 };
        let (w, h) = (600usize, 60usize);
        let mut px = vec![0u32; w * h];
        let drawn = toolbar(&mut Surface { w, h, stride: w, px: &mut px, y0: 0 }, &ui, band, &t, None);
        assert_eq!(toolbar_layout(&ui, band, &t), drawn);
        assert_eq!(drawn.rect_of(2).unwrap().w, CAPSULE_PT + TOOLBAR_MENU_CHEVRON_PT);
        let share = drawn.rect_of(3).unwrap();
        assert!(((share.x + share.w / 2.0) - 350.0).abs() < 0.5, "centred: {share:?}");
    }

    #[test]
    fn a_toolbar_laid_out_from_the_window_edge_starts_right_of_the_traffic_lights_at_every_scale() {
        use bifrost_ui::nav::ToolbarItem;
        use bifrost_ui::prefs_layout::Frame;
        let t = Toolbar::new(vec![ToolbarItem::button(1, "Pause"), ToolbarItem::button(2, "Hold")]);
        let full = Rect { x: 0.0, y: 0.0, w: 600.0, h: crate::layout::UNIFIED_BAR_PT };
        let data: &'static [u8] = Box::leak(std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../assets/SFUI.ttf")).unwrap().into_boxed_slice());
        for scale in [1.0f32, 1.5625, 2.0] {
            let ui = Ui::new(crate::kit::Font::parse(data).unwrap(), scale);
            let placed = toolbar_layout(&ui, full, &t);
            let first = placed.rect_of(1).unwrap();
            // the right edge of the last light in window pixels, as the shell places it (a 14 pt disc centred on its column)
            let d = (14.0 * scale).round() as i32;
            let (cx, _) = Frame::traffic_light(2);
            let lights_right = (cx * scale).round() as i32 - d / 2 + d;
            let first_px = ui.pt(first.x) as i32;
            assert!(first_px > lights_right, "scale {scale}: first item at {first_px} px, lights end at {lights_right} px");
            assert_eq!(first.x, toolbar_lead_pt(), "the inset is the same number of points at every scale");
            assert!(first.x - crate::layout::traffic_lights_trailing() >= TOOLBAR_GAP_PT, "the standard gap follows the lights");
        }
    }

    #[test]
    fn the_inset_only_applies_to_a_bar_that_meets_the_lights() {
        let lead = toolbar_lead_pt();
        let bar = crate::layout::UNIFIED_BAR_PT;
        // from the edge: moved right, narrower by the same amount
        let moved = clear_of_traffic_lights(Rect { x: 0.0, y: 0.0, w: 600.0, h: bar });
        assert_eq!((moved.x, moved.w, moved.y, moved.h), (lead, 600.0 - lead, 0.0, bar));
        // a capsule-height row inside the bar still meets the lights
        assert_eq!(clear_of_traffic_lights(Rect { x: 16.0, y: 8.0, w: 400.0, h: CAPSULE_PT }).x, lead);
        // a content toolbar that starts at a sidebar's edge, or already clear of the lights, is unchanged
        let beside = Rect { x: 232.0, y: 0.0, w: 400.0, h: bar };
        assert_eq!(clear_of_traffic_lights(beside), beside);
        let own = Rect { x: lead, y: 0.0, w: 400.0, h: bar };
        assert_eq!(clear_of_traffic_lights(own), own);
        // a second bar lower in the window is not under the lights
        let low = Rect { x: 0.0, y: bar, w: 600.0, h: 44.0 };
        assert_eq!(clear_of_traffic_lights(low), low);
        // a pane's own coordinates are judged in the window's: right of a sidebar it is left alone, at the edge it moves
        let pane = Rect { x: 12.0, y: 0.0, w: 400.0, h: bar };
        assert_eq!(clear_of_traffic_lights_in(pane, 200.0), pane);
        assert_eq!(clear_of_traffic_lights_in(pane, 0.0).x, lead);
        assert_eq!(clear_of_traffic_lights_in(Rect { x: 0.0, ..pane }, 40.0).x, lead - 40.0);
        // a window narrower than the lights leaves no negative width
        assert_eq!(clear_of_traffic_lights(Rect { x: 0.0, y: 0.0, w: 20.0, h: bar }).w, 0.0);
    }

    #[test]
    fn a_window_without_lights_over_its_content_keeps_its_toolbar_where_it_is() {
        use bifrost_ui::form_factor::FormFactor;
        let bar = Rect { x: 0.0, y: 0.0, w: 600.0, h: crate::layout::UNIFIED_BAR_PT };
        // which windows and form factors have lights over the content
        assert!(crate::layout::lights_over_content(Some("full-size"), FormFactor::Desktop));
        assert!(crate::layout::lights_over_content(None, FormFactor::Desktop), "not told: the full-size layout");
        assert!(!crate::layout::lights_over_content(Some(""), FormFactor::Desktop), "a classic title-bar window");
        assert!(!crate::layout::lights_over_content(Some("native"), FormFactor::Desktop));
        assert!(!crate::layout::lights_over_content(Some("full-size"), FormFactor::Tablet), "the tablet has no lights");
        crate::layout::TEST_LIGHTS.with(|t| t.set(Some(false)));
        assert_eq!(clear_of_traffic_lights(bar), bar);
        assert_eq!(clear_of_traffic_lights_in(bar, 0.0), bar);
        crate::layout::TEST_LIGHTS.with(|t| t.set(Some(true)));
        assert_eq!(clear_of_traffic_lights(bar).x, toolbar_lead_pt());
    }

    #[test]
    fn the_player_lays_out_artwork_text_and_transport_without_overlap() {
        let p = Player { title: "Song", subtitle: "Artist", playing: true, progress: Some(0.5), skips: Some(("gobackward.15", "goforward.30")) };
        let l = PlayerLayout::new(Rect { x: 10.0, y: 0.0, w: 400.0, h: 80.0 }, &p);
        assert_eq!(l.capsule.h, PLAYER_PT);
        assert!(l.artwork.right() < l.text.x && l.text.right() < l.back.unwrap().x);
        assert!(l.back.unwrap().right() <= l.play.x && l.play.right() <= l.forward.unwrap().x);
        assert!(l.forward.unwrap().right() <= l.capsule.right());
        assert!(l.progress.is_some());
        let bare = PlayerLayout::new(Rect { x: 0.0, y: 0.0, w: 300.0, h: 52.0 }, &Player { title: "Live", ..Default::default() });
        assert!(bare.back.is_none() && bare.forward.is_none() && bare.progress.is_none());
        let ui = test_ui();
        let mut px = vec![0u32; 420 * 90];
        assert_eq!(player(&mut Surface { w: 420, h: 90, stride: 420, px: &mut px, y0: 0 }, &ui, Rect { x: 10.0, y: 0.0, w: 400.0, h: 80.0 }, &p, None), l);
    }

    #[test]
    fn a_popover_returns_its_content_rect_below_a_low_anchor_and_draws_an_arrow() {
        let ui = test_ui();
        let (w, h) = (400usize, 300usize);
        let mut px = vec![0u32; w * h];
        let anchor = Rect { x: 180.0, y: 10.0, w: 40.0, h: 20.0 };
        let r = popover(&mut Surface { w, h, stride: w, px: &mut px, y0: 0 }, &ui, anchor, (200.0, 120.0), Rect { x: 0.0, y: 0.0, w: 400.0, h: 300.0 }, bifrost_ui::geometry::Edge::Top);
        assert!(r.y > anchor.bottom(), "no room above, so it opens below: {r:?}");
        assert_eq!((r.w, r.h), (200.0, 120.0));
        assert!(px[(r.y as usize - 4) * w + 200] != 0, "the arrow points up at the anchor");
    }

    #[test]
    fn the_translation_popover_draws_nothing_when_hidden_and_a_panel_with_text_once_presented() {
        use crate::translation::{TranslationError, Translator};
        use crate::translation_popover::{TranslationPresenter, TRANSLATION_POPOVER_SIZE_PT};
        use bifrost_ui::translation::LanguageAvailability;
        struct Shout;
        impl Translator for Shout {
            fn translate(&self, _: &str, _: &str, text: &str) -> Result<String, TranslationError> {
                Ok(text.to_uppercase())
            }
        }
        let ui = test_ui();
        let (w, h) = (400usize, 300usize);
        let bounds = Rect { x: 0.0, y: 0.0, w: 400.0, h: 300.0 };
        let anchor = Rect { x: 180.0, y: 10.0, w: 40.0, h: 20.0 };
        let mut presenter = TranslationPresenter::new();
        let mut hidden = vec![0u32; w * h];
        assert_eq!(translation_popover(&mut Surface { w, h, stride: w, px: &mut hidden, y0: 0 }, &ui, &presenter, anchor, bounds), None);
        assert!(hidden.iter().all(|&p| p == 0));

        let mut availability = LanguageAvailability::new(&["en", "is"]);
        availability.install("en", "is");
        presenter.present("good morning", "en", "is", bifrost_ui::translation_popover::ArrowEdge::Bottom, &availability, &Shout).unwrap();
        let mut shown = vec![0u32; w * h];
        let r = translation_popover(&mut Surface { w, h, stride: w, px: &mut shown, y0: 0 }, &ui, &presenter, anchor, bounds).unwrap();
        assert_eq!((r.w, r.h), TRANSLATION_POPOVER_SIZE_PT);
        assert!(r.y > anchor.bottom(), "no room above, so it opens below: {r:?}");
        let inside = |p: &[u32]| (r.y as usize..(r.y + r.h) as usize).any(|row| (r.x as usize..(r.x + r.w) as usize).any(|col| p[row * w + col] != 0));
        assert!(inside(&shown), "the panel and its text are drawn");
    }

    #[test]
    fn a_sheet_dims_its_parent_and_draws_buttons_only_once_it_has_slid_in() {
        use bifrost_ui::sheet::{ButtonRole, Sheet, SheetButton};
        let ui = test_ui();
        let (w, h) = (400usize, 300usize);
        let parent = Rect { x: 0.0, y: 0.0, w: 400.0, h: 300.0 };
        let sh = Sheet::alert("Quit?", "Unsaved changes.", vec![SheetButton::new(1, "Quit", ButtonRole::Default), SheetButton::new(2, "Cancel", ButtonRole::Cancel)]);
        let mut a = vec![0u32; w * h];
        let content = sheet(&mut Surface { w, h, stride: w, px: &mut a, y0: 0 }, &ui, parent, 28.0, &sh, 1.0, None);
        assert!(content.w > 0.0 && content.y >= 28.0);
        assert!(a[290 * w + 10] != 0, "the parent is dimmed");
        let btn = sh.layout(parent, 28.0, 1.0).buttons[0].1;
        assert_eq!(a[(btn.y + 11.0) as usize * w + (btn.x + 3.0) as usize], crate::kit::appearance::accent_now(), "default button is filled with the accent");
        let mut b = vec![0u32; w * h];
        sheet(&mut Surface { w, h, stride: w, px: &mut b, y0: 0 }, &ui, parent, 28.0, &sh, 0.0, None);
        assert_eq!(b[(btn.y + 11.0) as usize * w + (btn.x + 3.0) as usize], 0, "at progress 0 nothing is drawn");
    }

    #[test]
    fn an_alert_of_wide_text_is_sized_from_the_wrap_it_draws_and_its_buttons_stay_clear() {
        use bifrost_ui::sheet::{alert_sheet_height, stacked_buttons_h, wrapped_lines, ButtonRole, Hit, Sheet, SheetButton, ALERT_BUTTON_INSET_PT};
        let ui = test_ui();
        let (w, h) = (400usize, 600usize);
        let parent = Rect { x: 0.0, y: 0.0, w: 400.0, h: 600.0 };
        let (title, message) = ("WWWW MMMM WWWW MMMM WWWW", "WWWW MMMM WWWW MMMM WWWW MMMM WWWW\nMMMM WWWW MMMM WWWW MMMM WWWW MMMM");
        let buttons = || vec![SheetButton::new(1, "OK", ButtonRole::Default), SheetButton::new(2, "Cancel", ButtonRole::Cancel)];
        let sh = Sheet::alert(title, message, buttons());
        let (t, m) = alert_lines(&ui, &sh, sh.size.0);
        let drawn: usize = m.iter().map(|l| l.len().max(1)).sum();
        assert!(drawn > wrapped_lines(message, 34), "wide glyphs wrap to more lines ({drawn}) than a character count says");
        let mut px = vec![0u32; w * h];
        let content = sheet(&mut Surface { w, h, stride: w, px: &mut px, y0: 0 }, &ui, parent, 28.0, &sh, 1.0, None);
        let l = sh.layout(parent, 28.0, 1.0);
        assert_eq!(l.panel.h, alert_sheet_height(t.len(), drawn, 2), "the panel is as tall as the lines drawn and the button stack");
        let btn_top = l.buttons[0].1.y;
        assert!(btn_top - content.y >= 12.0, "the last line ends above the buttons: text to {}, buttons at {btn_top}", content.y);
        assert_eq!(btn_top, l.panel.bottom() - ALERT_BUTTON_INSET_PT - stacked_buttons_h(2));
        // a sheet built again (as apps do each frame) hits its buttons where they were drawn
        let again = Sheet::alert(title, message, buttons());
        let b = l.buttons[0].1;
        assert_eq!(again.hit(parent, 28.0, 1.0, b.x + 2.0, b.y + 2.0), Hit::Button(1));
        assert_eq!(px[(b.y + 11.0) as usize * w + (b.x + 3.0) as usize], crate::kit::appearance::accent_now());
    }

    #[test]
    fn segmented_and_stepper_report_geometry_and_draw() {
        let ui = test_ui();
        let (w, h) = (300usize, 60usize);
        let mut px = vec![0u32; w * h];
        let mut s = Surface { w, h, stride: w, px: &mut px, y0: 0 };
        let segs = segmented(&mut s, &ui, Rect { x: 10.0, y: 10.0, w: 150.0, h: 24.0 }, &["A", "B", "C"], 0, true);
        assert_eq!(segs.len(), 3);
        let r = stepper(&mut s, &ui, (200.0, 10.0), Some(bifrost_ui::sheet::StepperPart::Up), true);
        assert_eq!((r.w, r.h), (13.0, 22.0));
        focus_ring(&mut s, &ui, Rect { x: 10.0, y: 10.0, w: 150.0, h: 24.0 }, 7.0);
        assert!(px[11 * w + 12] != 0 && px[20 * w + 205] != 0);
    }

    #[test]
    fn the_mini_month_draws_and_reports_its_days() {
        let ui = test_ui();
        let (w, h) = (240usize, 260usize);
        let mut px = vec![0u32; w * h];
        let l = mini_month(&mut Surface { w, h, stride: w, px: &mut px, y0: 0 }, &ui, Rect { x: 0.0, y: 0.0, w: 210.0, h: 240.0 }, 2026, 10, 1, "October 2026", Some(10), Some(12), &[3]);
        assert_eq!(l.days.len(), 31);
        let (_, today) = l.days[9];
        assert_eq!(px[(today.y + today.h / 2.0) as usize * w + (today.x + 3.0) as usize], bifrost_render::ui::DESTRUCTIVE, "today is red");
        // the greyed days of September and November fill the six rows, and are drawn
        assert_eq!(l.days.len() + l.outside.len(), 42);
        let (_, lead) = l.outside[0];
        let inked = |r: Rect| (r.y as usize..(r.y + r.h) as usize).any(|y| (r.x as usize..(r.x + r.w) as usize).any(|x| px[y * w + x] != 0));
        assert!(inked(lead), "the 28th of September is drawn before the 1st");
        assert!(inked(l.outside.last().unwrap().1), "the last trailing day is drawn");
    }

    #[test]
    fn a_pane_draws_at_its_own_origin_and_clips() {
        let ui = test_ui();
        let (w, h) = (100usize, 80usize);
        let mut px = vec![0u32; w * h];
        let mut s = Surface { w, h, stride: w, px: &mut px, y0: 0 };
        {
            let mut p = pane(&mut s, &ui, Rect { x: 40.0, y: 20.0, w: 30.0, h: 30.0 });
            assert_eq!((p.w, p.h), (30, 30));
            p.fill_rect(0, 0, 500, 500, 0xffff_ffff);
        }
        assert_eq!(pane_origin_px(&s, &ui, Rect { x: 40.0, y: 20.0, w: 30.0, h: 30.0 }), (40, 20));
        assert_eq!(px[20 * w + 40], 0xffff_ffff);
        assert_eq!(px[49 * w + 69], 0xffff_ffff);
        assert_eq!(px[20 * w + 70], 0, "clipped on the right");
        assert_eq!(px[50 * w + 40], 0, "clipped below");
        assert_eq!(px[19 * w + 40], 0);
    }
}


#[cfg(test)]
mod gallery {
    use super::*;
    use bifrost_ui::menu::{Menu, MenuItem};
    use bifrost_ui::nav::ToolbarItem;

    /// Developer preview of every SDK widget in a Golden Gate window, in both schemes:
    /// `PREVIEW_DIR=/tmp/gallery cargo test -p bifrost-app preview_sdk_gallery -- --ignored` writes
    /// gallery-light.png and gallery-dark.png. Compare with `goldengate_refimages/27-Golden-Gate-Files-*.png`.
    #[test]
    #[ignore]
    fn preview_sdk_gallery() {
        let Some(dir) = std::env::var_os("PREVIEW_DIR") else { return };
        let font = bifrost_render::font::Font::parse(include_bytes!("../../assets/SFUI.ttf")).unwrap();
        let ui = Ui::new(font, 1.5625);
        let (wp, hp) = (900.0f32, 560.0f32);
        let (w, h) = (ui.pt(wp) as usize, ui.pt(hp) as usize);
        for (name, light) in [("light", true), ("dark", false)] {
            crate::kit::appearance::set_light(light, 0.0);
            crate::kit::appearance::tick(1.0e6);
            let mut img = crate::kit::Image::new(w, h);
            img.px.fill(DETAIL_BG.now());
            let mut s = Surface { w, h, stride: w, px: &mut img.px, y0: 0 };
            let side = 210.0;
            let bar = crate::layout::UNIFIED_BAR_PT;
            // sidebar
            let mut list = SourceList::new(vec![
                SidebarItem::header("Favorites"),
                SidebarItem::row(1, "Recents").with_symbol("clock"),
                SidebarItem::row(2, "Desktop").with_symbol("menubar.dock.rectangle"),
                SidebarItem::row(3, "Documents").with_symbol("doc"),
                SidebarItem::row(4, "Downloads").with_symbol("arrow.down.circle").with_count(3),
                SidebarItem::header("Locations"),
                SidebarItem::row(5, "olibuijr").with_symbol("house"),
                SidebarItem::row(6, "Computer").with_symbol("desktopcomputer"),
                SidebarItem::row(7, "Network").with_symbol("globe").disabled(),
                SidebarItem::row(8, "Recently Deleted").with_symbol("trash"),
            ], 32.0, 28.0);
            list.select(5);
            source_list(&mut s, &ui, Rect { x: 0.0, y: 0.0, w: side, h: hp }, &list, -bar, None);
            // stand-in traffic lights where the shell draws them
            for (i, c) in [crate::kit::ui::SYSTEM_RED, crate::kit::ui::SYSTEM_YELLOW, crate::kit::ui::SYSTEM_GREEN].into_iter().enumerate() {
                let (cx, cy) = bifrost_ui::prefs_layout::Frame::traffic_light(i);
                s.fill_rrect(ui.pt(cx - 7.0), ui.pt(cy - 7.0), ui.pt(14.0), ui.pt(14.0), ui.pt(7.0), c);
            }
            // toolbar
            toolbar_band(&mut s, &ui, Rect { x: side, y: 0.0, w: wp - side, h: bar });
            let tb = Toolbar::new(vec![
                ToolbarItem::icon(1, "Back", "chevron.left").in_group(1),
                ToolbarItem::icon(2, "Forward", "chevron.right").in_group(1).enabled(false),
                ToolbarItem::custom(9, "Title", 100.0),
                ToolbarItem::flexible_space(),
                ToolbarItem::icon(3, "Icons", "square.grid.2x2").in_group(2).selected(true),
                ToolbarItem::icon(4, "List", "list.bullet").in_group(2),
                ToolbarItem::icon(5, "Columns", "rectangle.split.3x1").in_group(2),
                ToolbarItem::icon(6, "Share", "square.and.arrow.up").in_group(3),
                ToolbarItem::icon(7, "Tags", "tag").in_group(3),
                ToolbarItem::icon(8, "More", "ellipsis").in_group(3),
                ToolbarItem::custom(10, "Search", 170.0),
            ]);
            let placed = toolbar(&mut s, &ui, Rect { x: side + 12.0, y: 0.0, w: wp - side - 24.0, h: bar }, &tb, Some(4));
            if let Some(r) = placed.rect_of(9) {
                let t = ui.text("olibuijr", crate::kit::ui::MAC_TITLE3_PT, Weight::Bold, LABEL.now());
                blit(&mut s, &t, ui.pt(r.x) as i32, (ui.pt(r.y + r.h / 2.0) as i32) - t.h as i32 / 2);
            }
            let mut query = TextField::new();
            if let Some(r) = placed.rect_of(10) {
                search_field(&mut s, &ui, &query, Rect { x: r.x, y: r.y + 2.0, w: r.w, h: 28.0 }, "Search", false);
            }
            // content: controls in a column
            let x = side + 24.0;
            let mut y = bar + 16.0;
            let row = |s: &mut dyn Canvas, label: &str, y: f32| {
                let t = ui.text(label, MAC_FOOTNOTE_PT, Weight::Regular, LABEL_2.now());
                blit(s, &t, ui.pt(x) as i32, ui.pt(y) as i32);
            };
            row(&mut s, "search_field: empty, focused with text, focused with a selection", y);
            y += 18.0;
            search_field(&mut s, &ui, &query, Rect { x, y, w: 180.0, h: 28.0 }, "Search", false);
            query.push_str("aqua");
            search_field(&mut s, &ui, &query, Rect { x: x + 200.0, y, w: 180.0, h: 28.0 }, "Search", true);
            let mut sel = TextField::new();
            sel.push_str("Aqua and Bondi");
            sel.command(bifrost_ui::Command::SelectAll, "");
            search_field(&mut s, &ui, &sel, Rect { x: x + 400.0, y, w: 200.0, h: 28.0 }, "Search", true);
            y += 46.0;
            row(&mut s, "text_field and popup_button (closed, open, disabled)", y);
            y += 18.0;
            text_field(&mut s, &ui, &query, Rect { x, y, w: 180.0, h: 24.0 }, "Name", false);
            popup_button(&mut s, &ui, Rect { x: x + 200.0, y: y + 1.0, w: 120.0, h: 22.0 }, "Name", false, true);
            popup_button(&mut s, &ui, Rect { x: x + 330.0, y: y + 1.0, w: 120.0, h: 22.0 }, "Kind", true, true);
            popup_button(&mut s, &ui, Rect { x: x + 460.0, y: y + 1.0, w: 120.0, h: 22.0 }, "Size", false, false);
            y += 42.0;
            row(&mut s, "segmented, stepper, focus_ring, alert sheet", y);
            y += 18.0;
            segmented(&mut s, &ui, Rect { x, y, w: 240.0, h: 24.0 }, &["Day", "Week", "Month"], 1, true);
            stepper(&mut s, &ui, (x + 270.0, y + 1.0), None, true);
            focus_ring(&mut s, &ui, Rect { x: x + 310.0, y: y + 1.0, w: 90.0, h: 22.0 }, 6.0);
            popup_button(&mut s, &ui, Rect { x: x + 310.0, y: y + 1.0, w: 90.0, h: 22.0 }, "Focus", false, true);
            y += 36.0;
            let al = bifrost_ui::sheet::Sheet::alert("Delete this file?", "This cannot be undone.", vec![
                bifrost_ui::sheet::SheetButton::new(1, "Delete", bifrost_ui::sheet::ButtonRole::Default),
                bifrost_ui::sheet::SheetButton::new(2, "Cancel", bifrost_ui::sheet::ButtonRole::Cancel),
            ]);
            sheet(&mut s, &ui, Rect { x: x + 420.0, y: y - 40.0, w: 300.0, h: 150.0 }, 28.0, &al, 1.0, None);
            y += 120.0;
            row(&mut s, "glyphs (sdk::glyphs::NAMES)", y);
            y += 18.0;
            for (k, name) in crate::glyphs::NAMES.iter().enumerate() {
                let gx = x + (k % 14) as f32 * 30.0;
                let gy = y + (k / 14) as f32 * 28.0;
                crate::glyphs::glyph(&mut s, &ui, name, Rect { x: gx, y: gy, w: 18.0, h: 18.0 }, if k % 2 == 0 { LABEL.now() } else { TINT });
            }
            y += 64.0;
            row(&mut s, "menu", y);
            y += 18.0;
            let mut mnu = Menu::popup(vec![MenuItem::new(1, "as Icons").checked(true).shortcut("\u{2318}1"), MenuItem::new(2, "as List").shortcut("\u{2318}2"), MenuItem::separator(), MenuItem::new(3, "Show Path Bar"), MenuItem::new(4, "Unavailable").enabled(false)], 0);
            mnu.highlighted = Some(1);
            menu(&mut s, &ui, (x, y), 200.0, &mnu);
            // footer path bar
            let foot = hp - 30.0;
            s.fill_rect(ui.pt(side) as usize, ui.pt(foot) as usize, ui.pt(wp - side) as usize, 1, SEPARATOR_C.now());
            path_bar(&mut s, &ui, Rect { x: side + 12.0, y: foot + 5.0, w: wp - side - 24.0, h: 20.0 }, &[("Macintosh HD", "internaldrive"), ("Users", "folder"), ("olibuijr", "house"), ("Documents", "folder"), ("Aqua and Bondi.pdf", "doc")]);
            let png = springboard::shell::screenshot::encode(&img);
            std::fs::write(std::path::Path::new(&dir).join(format!("gallery-{name}.png")), png).unwrap();
        }
    }
}

/// A `Canvas` that records every call, for the drawing tests of the kit (`glyphs` and `widgets`).
#[cfg(test)]
pub(crate) mod recorder {
    use bifrost_render::canvas::{Canvas, ImageRef, MaterialStyle, PathData, PathPaint, PathVerb, RectF, Stop, TextRun, Transform2D};

    #[derive(Debug, Clone, PartialEq)]
    pub enum Op {
        Fill(RectF, f32, u32),
        FillRect(RectF, u32),
        Stroke(RectF, f32, f32, u32),
        Path(Vec<PathVerb>, PathPaint, u32),
        Image(u64, RectF, f32),
        Tinted(u64, RectF, u32),
        Material(RectF, f32, MaterialStyle),
        Other(&'static str),
    }

    #[derive(Default)]
    pub struct Rec {
        pub ops: Vec<Op>,
    }

    impl Canvas for Rec {
        fn size(&self) -> (u32, u32) {
            (0, 0)
        }
        fn fill_rect(&mut self, r: RectF, color: u32) {
            self.ops.push(Op::FillRect(r, color));
        }
        fn fill_rrect(&mut self, r: RectF, radius: f32, color: u32) {
            self.ops.push(Op::Fill(r, radius, color));
        }
        fn stroke_rrect(&mut self, r: RectF, radius: f32, width: f32, color: u32) {
            self.ops.push(Op::Stroke(r, radius, width, color));
        }
        fn linear_gradient(&mut self, _: RectF, _: f32, _: (f32, f32), _: (f32, f32), _: &[Stop]) {
            self.ops.push(Op::Other("gradient"));
        }
        fn shadow(&mut self, _: RectF, _: f32, _: f32, _: (f32, f32), _: u32) {
            self.ops.push(Op::Other("shadow"));
        }
        fn image(&mut self, img: ImageRef<'_>, dst: RectF, alpha: f32) {
            self.ops.push(Op::Image(img.id, dst, alpha));
        }
        fn image_tinted(&mut self, img: ImageRef<'_>, dst: RectF, color: u32) {
            self.ops.push(Op::Tinted(img.id, dst, color));
        }
        fn text(&mut self, _: &TextRun, _: u32) {
            self.ops.push(Op::Other("text"));
        }
        fn path(&mut self, path: &PathData, paint: PathPaint, color: u32) {
            self.ops.push(Op::Path(path.verbs.clone(), paint, color));
        }
        fn material(&mut self, r: RectF, radius: f32, style: MaterialStyle) {
            self.ops.push(Op::Material(r, radius, style));
        }
        fn push_clip(&mut self, _: RectF, _: f32) {}
        fn pop_clip(&mut self) {}
        fn push_transform(&mut self, _: Transform2D) {}
        fn pop_transform(&mut self) {}
        fn push_opacity(&mut self, _: f32) {}
        fn pop_opacity(&mut self) {}
    }
}

#[cfg(test)]
mod canvas_tests {
    use super::recorder::{Op, Rec};
    use super::*;

    fn ui() -> Ui {
        let data: &'static [u8] = Box::leak(std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../assets/SFUI.ttf")).unwrap().into_boxed_slice());
        Ui::new(crate::kit::Font::parse(data).unwrap(), 1.0)
    }

    fn img(w: usize, h: usize, v: u32) -> crate::kit::Image {
        crate::kit::Image { w, h, px: vec![v; w * h] }
    }

    #[test]
    fn the_image_id_is_stable_for_the_same_pixels_and_changes_with_them() {
        let a = img(4, 2, 0xff00_00ff);
        let b = img(4, 2, 0xff00_00ff);
        assert_eq!(image_id(&a), image_id(&b));
        assert_ne!(image_id(&a), image_id(&img(4, 2, 0xffff_0000)));
        // the same pixels in another shape are another image
        assert_ne!(image_id(&img(4, 2, 1)), image_id(&img(2, 4, 1)));
    }

    #[test]
    fn blit_draws_the_whole_image_at_its_pixel_origin_at_full_alpha() {
        let mut r = Rec::default();
        let a = img(3, 5, 7);
        blit(&mut r, &a, 10, -2);
        assert_eq!(r.ops.len(), 1);
        match &r.ops[0] {
            Op::Image(id, dst, alpha) => {
                assert_eq!(*id, image_id(&a));
                assert_eq!(*dst, RectF::new(10.0, -2.0, 3.0, 5.0));
                assert_eq!(*alpha, 1.0);
            }
            other => panic!("expected an image op, got {other:?}"),
        }
    }

    #[test]
    fn a_vibrant_label_is_a_tint_whose_alpha_is_the_level_strength() {
        for level in [Vibrant::Label, Vibrant::Secondary, Vibrant::Tertiary, Vibrant::Quaternary] {
            let mut r = Rec::default();
            blit_vibrant(&mut r, &img(2, 2, 0xffff_ffff), 0, 0, level);
            let want = (level.strength().clamp(0.0, 1.0) * 255.0).round() as u32;
            match &r.ops[0] {
                Op::Tinted(_, _, color) => assert_eq!(color >> 24, want, "{level:?}"),
                other => panic!("expected a tinted op, got {other:?}"),
            }
        }
    }

    #[test]
    fn a_behind_window_material_with_vibrancy_fills_its_tint_and_declares_the_region() {
        set_vibrancy(true, true);
        let mut r = Rec::default();
        visual_effect(&mut r, &ui(), Rect { x: 0.0, y: 0.0, w: 40.0, h: 20.0 }, 4.0, Material::Popover, Blending::BehindWindow, State::Active);
        set_vibrancy(false, false);
        let declared = take_frame_materials();
        assert_eq!(declared.len(), 1);
        assert_eq!(declared[0].rect, Rect { x: 0.0, y: 0.0, w: 40.0, h: 20.0 });
        match &r.ops[..] {
            [Op::Fill(rect, radius, color)] => {
                assert_eq!(*rect, RectF::new(0.0, 0.0, 40.0, 20.0));
                assert_eq!(*radius, 4.0);
                assert_eq!(*color, Material::Popover.tint());
            }
            other => panic!("expected one fill, got {other:?}"),
        }
    }

    #[test]
    fn without_vibrancy_a_behind_window_material_is_its_opaque_colour_and_declares_nothing() {
        set_vibrancy(false, false);
        let mut r = Rec::default();
        visual_effect(&mut r, &ui(), Rect { x: 2.0, y: 3.0, w: 30.0, h: 10.0 }, 0.0, Material::Sidebar, Blending::BehindWindow, State::Inactive);
        assert!(take_frame_materials().is_empty());
        match &r.ops[..] {
            [Op::Fill(rect, _, color)] => {
                assert_eq!(*rect, RectF::new(2.0, 3.0, 30.0, 10.0));
                assert_eq!(*color, Material::Sidebar.solid());
            }
            other => panic!("expected one opaque fill, got {other:?}"),
        }
    }

    #[test]
    fn a_within_window_material_is_a_canvas_material_with_the_blur_in_points_times_scale() {
        let mut r = Rec::default();
        let u = ui();
        material(&mut r, &u, Rect { x: 1.0, y: 2.0, w: 50.0, h: 40.0 }, 6.0, Material::Hud);
        match &r.ops[..] {
            [Op::Material(rect, radius, style)] => {
                assert_eq!(*rect, RectF::new(1.0, 2.0, 50.0, 40.0));
                assert_eq!(*radius, 6.0);
                assert_eq!(style.blur, Material::Hud.blur_pt());
                assert_eq!(style.tint, Material::Hud.tint());
                assert_eq!(style.saturation, 1.0);
            }
            other => panic!("expected one material op, got {other:?}"),
        }
    }

    #[test]
    fn a_toolbar_band_draws_its_separator_as_a_one_pixel_rectangle_at_the_bottom() {
        let mut r = Rec::default();
        toolbar_band(&mut r, &ui(), Rect { x: 0.0, y: 0.0, w: 100.0, h: 40.0 });
        let last = r.ops.last().expect("a separator");
        match last {
            Op::FillRect(rect, _) => {
                assert_eq!(rect.h, 1.0);
                assert_eq!(rect.y, 39.0);
                assert_eq!(rect.w, 100.0);
            }
            other => panic!("expected the separator, got {other:?}"),
        }
        assert!(matches!(r.ops[0], Op::FillRect(..) | Op::Material(..)));
    }
}

#[cfg(test)]
mod visual_effect_model_tests {
    use super::*;
    use bifrost_ui::visual_effect::MaterialKind;
    #[test]
    fn model_kinds_round_trip_and_cover_the_catalogue() {
        assert_eq!(Material::ALL.len(), MaterialKind::ALL.len());
        for m in Material::ALL {
            assert_eq!(Material::from_model(m.model_kind()), m);
        }
    }
    #[test]
    fn every_material_has_a_translucent_tint_and_selection_varies_with_state() {
        for m in Material::ALL {
            assert!(m.tint_for(false, true) >> 24 > 0);
        }
        assert_ne!(Material::Selection.tint_for(true, true), Material::Selection.tint_for(false, true));
        assert_ne!(Material::Selection.tint_for(false, false), Material::Selection.tint_for(false, true));
        assert_eq!(Material::Sidebar.tint_for(true, false), Material::Sidebar.tint());
    }
}
