//! The desktop's notification card, shared by the Notification Center column and the banners at the top right
//! (27-Golden-Gate-Notification-Center.png): a translucent tile in the scheme's material (light in Light, dark in
//! Dark) with the app's icon, its name and the age, the title and the body; and the small controls drawn on it, the
//! circled close X, the Options chevron and the Clear All capsule.

use super::super::ui::{Ui, Weight, FIELD_BG, LABEL, LABEL_2, LABEL_3, MAC_BODY_PT, MAC_HEADLINE_PT, MAC_SUBHEAD_PT, MENU_SOLID, NC_TILE_TINT};
use super::super::AppId;
use bifrost_ui::material_request::ShellSurface;
use crate::gfx::{scale, Image, Surface};

pub type R4 = (f32, f32, f32, f32);

/// The app icon on a card, the gap after it and the padding inside the tile, in pt.
pub const ICON_PT: f32 = 40.0;
pub const ICON_GAP_PT: f32 = 10.0;
pub const PAD_PT: f32 = 14.0;
/// The round close and Options controls.
pub const CONTROL_PT: f32 = 20.0;
/// The Clear All capsule the header X grows into on hover.
pub const CLEAR_ALL_W_PT: f32 = 76.0;

/// The name a card shows for `app`: an installed package's manifest name, a built-in's name.
pub fn app_name(app: AppId) -> String {
    match app {
        AppId::External(slot) => super::super::apphost::installed().get(slot as usize).map_or_else(String::new, |a| a.manifest.name.clone()),
        other => super::super::apps().iter().find(|a| a.id == other).map_or_else(String::new, |a| a.name.to_string()),
    }
}

pub fn within((x, y, w, h): R4, px: f32, py: f32) -> bool {
    px >= x && py >= y && px < x + w && py < y + h
}

/// A tile of the column or a banner: the presenter's blur under the Notification Center tint, so the desktop shows
/// through (lighter than a menu's nearly opaque tint); solid with Reduce Transparency.
pub fn tile(s: &mut Surface, surface: ShellSurface, (x, y, w, h): R4, r: f32, alpha: u32) {
    let fill = if bifrost_render::appearance::reduce_transparency() {
        MENU_SOLID.now()
    } else if bifrost_render::glass::gpu() {
        bifrost_render::glass::push(super::super::control_center::shell_glass_rect(&surface.request((x, y, w, h), r)));
        bifrost_render::glass::tint(NC_TILE_TINT.now())
    } else {
        NC_TILE_TINT.now()
    };
    s.fill_rrect(x, y, w, h, r, scale(fill, alpha));
}

/// What a card says.
pub struct CardText<'a> {
    /// the app's name (with "· n more" for a stack)
    pub head: &'a str,
    /// the age ("now", "5m ago"); None hides it (a hovered banner shows its Options chevron there)
    pub age: Option<&'a str>,
    pub title: &'a str,
    pub body: &'a str,
}

/// Draws a card's text and icon into `rect` (the tile is drawn by the caller): the icon at the left, then the head
/// and age line, the title and the body. `trailing` is the room kept free at the right edge for a control, in px.
pub fn draw_text(s: &mut Surface, ui: &Ui, (x, y, w, h): R4, icon: Option<&Image>, text: &CardText, trailing: f32, alpha: u32) {
    let (label, secondary) = (LABEL.now(), LABEL_2.now());
    let pad = ui.pt(PAD_PT);
    let tx = x + pad + ui.pt(ICON_PT) + ui.pt(ICON_GAP_PT);
    if let Some(icon) = icon {
        s.blit(icon, (x + pad) as i32, (y + (h - icon.h as f32) / 2.0) as i32, 255, alpha);
    }
    let room = x + w - tx - pad - trailing;
    let head = ui.text(&ui.fit(text.head, MAC_SUBHEAD_PT, room - ui.pt(60.0)), MAC_SUBHEAD_PT, Weight::Regular, secondary);
    let t1 = ui.text(&ui.fit(text.title, MAC_HEADLINE_PT, room), MAC_HEADLINE_PT, Weight::Semibold, label);
    let t2 = ui.text(&ui.fit(text.body, MAC_BODY_PT, x + w - tx - pad), MAC_BODY_PT, Weight::Regular, label);
    let mut ly = y + (h - (head.h + t1.h + t2.h) as f32) / 2.0;
    s.blit(&head, tx as i32, ly as i32, 255, alpha);
    if let Some(age) = text.age {
        let age = ui.text(age, MAC_SUBHEAD_PT, Weight::Regular, secondary);
        s.blit(&age, (x + w - pad - trailing - ui.pt(6.0) - age.w as f32) as i32, ly as i32, 255, alpha);
    }
    ly += head.h as f32;
    s.blit(&t1, tx as i32, ly as i32, 255, alpha);
    ly += t1.h as f32;
    s.blit(&t2, tx as i32, ly as i32, 255, alpha);
}

/// A stroke from (ax, ay) to (bx, by), `thick` px wide, drawn as overlapping round dots.
fn stroke(s: &mut Surface, (ax, ay): (f32, f32), (bx, by): (f32, f32), thick: f32, ink: u32) {
    let len = ((bx - ax).powi(2) + (by - ay).powi(2)).sqrt();
    let n = (len / (thick * 0.5).max(0.5)).ceil().max(1.0) as i32;
    for i in 0..=n {
        let f = i as f32 / n as f32;
        let (px, py) = (ax + (bx - ax) * f, ay + (by - ay) * f);
        s.fill_rrect(px - thick / 2.0, py - thick / 2.0, thick, thick, thick / 2.0, ink);
    }
}

/// The circled close X (the reference's header and card close): a plate with a hairline ring and an X.
pub fn close_disc(s: &mut Surface, ui: &Ui, (x, y, w, h): R4, alpha: u32) {
    s.fill_rrect(x, y, w, h, w / 2.0, scale(FIELD_BG.now(), alpha));
    s.stroke_rrect(x, y, w, h, w / 2.0, ui.pt(0.5).max(1.0), scale(LABEL_3.now(), alpha));
    let (cx, cy, k) = (x + w / 2.0, y + h / 2.0, w * 0.2);
    let ink = scale(LABEL_2.now(), alpha);
    let thick = ui.pt(1.5);
    stroke(s, (cx - k, cy - k), (cx + k, cy + k), thick, ink);
    stroke(s, (cx - k, cy + k), (cx + k, cy - k), thick, ink);
}

/// The Options chevron of a hovered banner: a plate with a downward chevron (it opens the banner's actions menu).
pub fn chevron_disc(s: &mut Surface, ui: &Ui, (x, y, w, h): R4, alpha: u32) {
    s.fill_rrect(x, y, w, h, w / 2.0, scale(FIELD_BG.now(), alpha));
    s.stroke_rrect(x, y, w, h, w / 2.0, ui.pt(0.5).max(1.0), scale(LABEL_3.now(), alpha));
    let (cx, cy, k) = (x + w / 2.0, y + h / 2.0, w * 0.2);
    let ink = scale(LABEL_2.now(), alpha);
    let thick = ui.pt(1.5);
    stroke(s, (cx - k, cy - k * 0.5), (cx, cy + k * 0.5), thick, ink);
    stroke(s, (cx, cy + k * 0.5), (cx + k, cy - k * 0.5), thick, ink);
}

/// The rect of the Clear All capsule a close disc at `close` grows into: right-aligned with the disc, as tall.
pub fn clear_all_rect(ui: &Ui, (x, y, w, h): R4) -> R4 {
    let cw = ui.pt(CLEAR_ALL_W_PT).max(w);
    (x + w - cw, y, cw, h)
}

/// The Clear All capsule (`clear_all_rect` of the close disc).
pub fn clear_all_capsule(s: &mut Surface, ui: &Ui, (x, y, w, h): R4, alpha: u32) {
    s.fill_rrect(x, y, w, h, h / 2.0, scale(FIELD_BG.now(), alpha));
    s.stroke_rrect(x, y, w, h, h / 2.0, ui.pt(0.5).max(1.0), scale(LABEL_3.now(), alpha));
    let t = ui.text("Clear All", MAC_SUBHEAD_PT, Weight::Semibold, LABEL.now());
    s.blit(&t, (x + (w - t.w as f32) / 2.0) as i32, (y + (h - t.h as f32) / 2.0) as i32, 255, alpha);
}

/// The disc of the Focus status line: the crescent moon in the Focus indigo.
pub fn moon(s: &mut Surface, (cx, cy): (f32, f32), radius: f32, ink: u32) {
    let disc = super::super::glyphs::disc((cx, cy), radius);
    let cut = super::super::glyphs::disc((cx + radius * 0.55, cy - radius * 0.4), radius * 0.85);
    super::super::glyphs::paint(s, (cx - radius, cy - radius, 2.0 * radius, 2.0 * radius), ink, |x, y| disc(x, y).max(-cut(x, y)));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_clear_all_capsule_grows_left_from_the_disc() {
        let ui = crate::shell::tests::ui();
        let disc = (500.0, 40.0, ui.pt(CONTROL_PT), ui.pt(CONTROL_PT));
        let (x, y, w, h) = clear_all_rect(&ui, disc);
        assert!((x + w - (disc.0 + disc.2)).abs() < 0.01, "the right edges line up");
        assert!(w > disc.2 && y == disc.1 && h == disc.3);
        assert!(within((x, y, w, h), disc.0 - 1.0, disc.1 + 1.0), "the capsule reaches left of the disc");
    }
}
