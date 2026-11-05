//! Button styles (`ButtonStyle`, `ButtonStyleConfiguration`, `ButtonRole`): a style is a value that paints a button
//! from its configuration (label, pressed state, role, enabled, focus, frame), so an app can restyle every button or
//! write its own style without a new view. `Button` keeps the interaction (press tracking, the action, keys) and asks
//! its style for the size and the drawing, as `makeBody(configuration:)` does.
//!
//! Built-in styles: [`Bordered`] (the default capsule plate), [`BorderedProminent`] (accent plate, white label),
//! [`Borderless`] (no plate; a pressed plate only while pressed), [`Plain`] (label only, dimmed while pressed) and
//! [`LinkStyle`] (link-coloured label). A destructive role draws the label (or the prominent plate) in `DESTRUCTIVE`.
use super::controls::draw_text;
use crate::kit::ui::{Weight, CAPSULE_BG, CONTROL_PRESSED, DESTRUCTIVE, LABEL, LABEL_2, LABEL_3, LINK, WHITE};
use crate::kit::Ui;
use crate::layout::ROW_PT;
use bifrost_render::canvas::{Canvas, RectF};
use bifrost_render::ui::{MAC_BODY_PT, SPACE_L, SPACE_XS};
use bifrost_ui::Rect;

/// `ButtonRole`: the semantic purpose of a button.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ButtonRole {
    /// deletes data or cannot be undone: drawn in the destructive colour
    Destructive,
    /// cancels the operation (the Escape button of a dialog)
    Cancel,
}

/// `ButtonStyleConfiguration`: what a style sees of the button it paints.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ButtonConfiguration<'a> {
    /// `label`: the button's title
    pub label: &'a str,
    /// `isPressed`
    pub is_pressed: bool,
    /// `role`
    pub role: Option<ButtonRole>,
    pub enabled: bool,
    pub focused: bool,
    /// the button's frame, window points
    pub frame: Rect,
}

impl ButtonConfiguration<'_> {
    fn destructive(&self) -> bool {
        self.role == Some(ButtonRole::Destructive)
    }
    /// The label colour for a style with no plate of its own: disabled, destructive, else `normal`.
    fn label_color(&self, normal: u32) -> u32 {
        if !self.enabled {
            LABEL_3.now()
        } else if self.destructive() {
            DESTRUCTIVE
        } else {
            normal
        }
    }
}

/// The label width plus `pad` on each side, one row tall.
fn label_size(ui: &Ui, cfg: &ButtonConfiguration, pad: f32) -> (f32, f32) {
    (ui.measure(cfg.label, MAC_BODY_PT, Weight::Regular) / ui.pt(1.0) + 2.0 * pad, ROW_PT)
}

/// An opaque colour at 77% brightness: the pressed prominent plate.
fn deepen(c: u32) -> u32 {
    let ch = |shift: u32| ((((c >> shift) & 0xff) as f32) * 0.77) as u32;
    ch(0) | ch(8) << 8 | ch(16) << 16 | 0xff00_0000
}

fn plate(s: &mut dyn Canvas, ui: &Ui, f: Rect, color: u32) {
    s.fill_rrect(RectF::new(ui.pt(f.x), ui.pt(f.y), ui.pt(f.w), ui.pt(f.h)), ui.pt(f.h / 2.0), color);
}

/// `ButtonStyle`: paints a button from its configuration (`makeBody(configuration:)`). The focus ring is drawn by the
/// button after the style.
pub trait ButtonStyle: 'static {
    fn draw(&self, s: &mut dyn Canvas, ui: &Ui, cfg: &ButtonConfiguration);
    /// The size the style asks for, in points (`intrinsicContentSize`). Default: the label with the standard padding.
    fn size(&self, ui: &Ui, cfg: &ButtonConfiguration) -> (f32, f32) {
        label_size(ui, cfg, SPACE_L)
    }
}

/// `.bordered` / `.automatic`: the capsule plate, darker while pressed.
#[derive(Clone, Copy, Debug, Default)]
pub struct Bordered;
impl ButtonStyle for Bordered {
    fn draw(&self, s: &mut dyn Canvas, ui: &Ui, cfg: &ButtonConfiguration) {
        plate(s, ui, cfg.frame, if cfg.is_pressed { CONTROL_PRESSED.now() } else { CAPSULE_BG.now() });
        draw_text(s, ui, cfg.frame, cfg.label, MAC_BODY_PT, Weight::Regular, cfg.label_color(LABEL.now()), true);
    }
}

/// `.borderedProminent`: an accent plate (destructive red for a destructive role) with a white label.
#[derive(Clone, Copy, Debug, Default)]
pub struct BorderedProminent;
impl ButtonStyle for BorderedProminent {
    fn draw(&self, s: &mut dyn Canvas, ui: &Ui, cfg: &ButtonConfiguration) {
        let fill = if !cfg.enabled {
            CAPSULE_BG.now()
        } else if cfg.destructive() {
            DESTRUCTIVE
        } else {
            crate::kit::appearance::accent_now()
        };
        plate(s, ui, cfg.frame, if cfg.is_pressed && cfg.enabled { deepen(fill) } else { fill });
        let text = if cfg.enabled { WHITE } else { LABEL_3.now() };
        draw_text(s, ui, cfg.frame, cfg.label, MAC_BODY_PT, Weight::Semibold, text, true);
    }
}

/// `.borderless`: no plate at rest; the pressed plate shows while the button is held (toolbar and control-group
/// buttons).
#[derive(Clone, Copy, Debug, Default)]
pub struct Borderless;
impl ButtonStyle for Borderless {
    fn draw(&self, s: &mut dyn Canvas, ui: &Ui, cfg: &ButtonConfiguration) {
        if cfg.is_pressed {
            plate(s, ui, cfg.frame, CONTROL_PRESSED.now());
        }
        draw_text(s, ui, cfg.frame, cfg.label, MAC_BODY_PT, Weight::Regular, cfg.label_color(LABEL.now()), true);
    }
    fn size(&self, ui: &Ui, cfg: &ButtonConfiguration) -> (f32, f32) {
        label_size(ui, cfg, SPACE_L / 2.0)
    }
}

/// `.plain`: the label only, dimmed while pressed.
#[derive(Clone, Copy, Debug, Default)]
pub struct Plain;
impl ButtonStyle for Plain {
    fn draw(&self, s: &mut dyn Canvas, ui: &Ui, cfg: &ButtonConfiguration) {
        let c = if cfg.is_pressed && cfg.enabled { LABEL_2.now() } else { cfg.label_color(LABEL.now()) };
        draw_text(s, ui, cfg.frame, cfg.label, MAC_BODY_PT, Weight::Regular, c, true);
    }
    fn size(&self, ui: &Ui, cfg: &ButtonConfiguration) -> (f32, f32) {
        label_size(ui, cfg, SPACE_XS)
    }
}

/// `.link`: the label in the link colour, dimmed while pressed.
#[derive(Clone, Copy, Debug, Default)]
pub struct LinkStyle;
impl ButtonStyle for LinkStyle {
    fn draw(&self, s: &mut dyn Canvas, ui: &Ui, cfg: &ButtonConfiguration) {
        let c = if cfg.is_pressed && cfg.enabled { LABEL_2.now() } else { cfg.label_color(LINK) };
        draw_text(s, ui, cfg.frame, cfg.label, MAC_BODY_PT, Weight::Regular, c, true);
    }
    fn size(&self, ui: &Ui, cfg: &ButtonConfiguration) -> (f32, f32) {
        label_size(ui, cfg, 0.0)
    }
}
