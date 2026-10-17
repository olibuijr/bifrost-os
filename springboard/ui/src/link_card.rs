//! The rich link view drawn (`LPLinkView` parity, see `docs/parity/link-presentation.md`): a card with the site's icon,
//! the page title and the site name, drawn from the [`Card`] that [`crate::linkmeta::LinkView::card`] makes.
//!
//! The card is one row high (`LINK_VIEW_HEIGHT_PT`). With no icon loaded yet, the first letter of the site name sits on
//! a tile, so the placeholder and the loaded card have the same shape. The caller fetches the icon and passes it in as
//! an image; nothing here touches the network. Text follows the appearance: it is drawn white and tinted with
//! `Surface::blit_colored`, as the rest of the shell's text is.
//!
//! The text decisions (the subtitle, the initial) are pure functions and tested; the drawing is checked on the host
//! preview and on Midget.

use bifrost_render::gfx::{Image, Surface};
use bifrost_render::ui::{Ui, Weight, CARD_BG, CARD_RADIUS_PT, CONTROL_PRESSED, LABEL, LABEL_2, MAC_CALLOUT_PT, MAC_HEADLINE_PT, SEPARATOR_C, WHITE};

use crate::linkmeta::Card;

/// Height of one link row, in points.
pub const LINK_VIEW_HEIGHT_PT: f32 = 64.0;
/// Side of the icon tile, in points.
pub const ICON_PT: f32 = 40.0;
/// Inner padding of the card, in points.
pub const PAD_PT: f32 = 12.0;
/// Radius of the icon tile, in points.
pub const ICON_RADIUS_PT: f32 = 8.0;

/// The second line: the site name, then "Video" when the page has one. A placeholder shows the host.
pub fn subtitle_for(card: &Card) -> String {
    if card.has_video {
        format!("{} · Video", card.subtitle)
    } else {
        card.subtitle.clone()
    }
}

/// The letter shown on the icon tile when no icon is loaded: the first character of the site name, upper case.
/// A bullet when the name is empty.
pub fn initial_for(card: &Card) -> String {
    match card.subtitle.trim().chars().next() {
        Some(c) => c.to_uppercase().collect(),
        None => "•".to_string(),
    }
}

/// Draws the link row at (`x`, `y`) with width `w`, and returns its height in pixels. `icon` is the loaded site icon,
/// if any.
pub fn draw(s: &mut Surface, ui: &Ui, card: &Card, icon: Option<&Image>, x: f32, y: f32, w: f32) -> f32 {
    let h = ui.pt(LINK_VIEW_HEIGHT_PT);
    let radius = ui.pt(CARD_RADIUS_PT);
    s.fill_rrect(x, y, w, h, radius, CARD_BG.now());
    s.stroke_rrect(x, y, w, h, radius, ui.pt(1.0).max(1.0), SEPARATOR_C.now());

    let icon_px = ui.pt(ICON_PT);
    let pad = ui.pt(PAD_PT);
    let (ix, iy) = (x + pad, y + (h - icon_px) / 2.0);
    match icon {
        Some(img) => s.blit(img, ix as i32, iy as i32, 255, 255),
        None => {
            s.fill_rrect(ix, iy, icon_px, icon_px, ui.pt(ICON_RADIUS_PT), CONTROL_PRESSED.now());
            let letter = ui.text(&initial_for(card), MAC_HEADLINE_PT, Weight::Semibold, WHITE);
            let (cx, cy) = (ix + icon_px / 2.0 - letter.w as f32 / 2.0, iy + icon_px / 2.0 - letter.h as f32 / 2.0);
            s.blit_colored(&letter, cx as i32, cy as i32, LABEL.now(), 255);
        }
    }

    let tx = ix + icon_px + pad;
    let text_w = (x + w - pad - tx).max(0.0);
    let title = ui.text(&ui.fit(&card.title, MAC_HEADLINE_PT, text_w), MAC_HEADLINE_PT, Weight::Semibold, WHITE);
    s.blit_colored(&title, tx as i32, (y + ui.pt(12.0)) as i32, LABEL.now(), 255);
    let sub = ui.text(&ui.fit(&subtitle_for(card), MAC_CALLOUT_PT, text_w), MAC_CALLOUT_PT, Weight::Regular, WHITE);
    s.blit_colored(&sub, tx as i32, (y + ui.pt(32.0)) as i32, LABEL_2.now(), 255);
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    fn card(title: &str, subtitle: &str, video: bool) -> Card {
        Card { title: title.into(), subtitle: subtitle.into(), icon: None, image: None, has_video: video, placeholder: false }
    }

    #[test]
    fn the_second_line_names_the_site_and_marks_a_video() {
        assert_eq!(subtitle_for(&card("T", "RÚV", false)), "RÚV");
        assert_eq!(subtitle_for(&card("T", "RÚV", true)), "RÚV · Video");
    }

    #[test]
    fn the_icon_tile_shows_the_upper_case_first_letter_of_the_site() {
        assert_eq!(initial_for(&card("T", "visir.is", false)), "V");
        assert_eq!(initial_for(&card("T", "  mbl.is", false)), "M");
        assert_eq!(initial_for(&card("T", "", false)), "•");
    }

    #[test]
    fn a_row_is_one_link_height_tall() {
        assert!(LINK_VIEW_HEIGHT_PT > ICON_PT);
        assert_eq!(LINK_VIEW_HEIGHT_PT, 64.0);
    }
}
