//! Paints the Setup Assistant: the title and body of the step on show, then its controls from `Setup::items`, so what
//! is drawn is what a tap hits. Colours are the kit's semantic tokens, resolved with `now()`, so Light and Dark follow
//! the appearance. Text is cached per string in `TextCache`.

use crate::window::{Item, Kind, Setup, BTN_PT, CONTENT_Y, GAP_PT, PAD_PT};
use bifrost_app::kit::ui::{CARD_BG, CONTENT_BG, DESTRUCTIVE, FIELD_BG, LABEL, LABEL_2, LINK, MAC_BODY_PT, MAC_CALLOUT_PT, MAC_LARGE_TITLE_PT, SEPARATOR_C, WHITE};
use bifrost_app::kit::{Surface, TextCache, Ui, Weight};
use springboard::setup::Step;

/// Corner radii in points: chips and buttons, grouped cards.
const CHIP_R: f32 = 8.0;
const CARD_R: f32 = 13.0;
/// The switch's track size in points.
const TRACK_W: f32 = 51.0;
const TRACK_H: f32 = 31.0;
const KNOB_PT: f32 = 27.0;
/// Body text starts this far below the title, one line per step of `BODY_LINE_PT`.
const BODY_Y: f32 = 104.0;
const BODY_LINE_PT: f32 = 22.0;

/// A filled rounded rectangle in points.
fn fill(s: &mut Surface, ui: &Ui, x: f32, y: f32, w: f32, h: f32, radius: f32, color: u32) {
    s.fill_rrect(ui.pt(x), ui.pt(y), ui.pt(w.max(0.0)), ui.pt(h.max(0.0)), ui.pt(radius), color);
}

/// A text run whose vertical centre is `mid`, cut to `max` points, starting at `x`.
fn text_mid(labels: &mut TextCache, s: &mut Surface, ui: &Ui, label: &str, pt: f32, weight: Weight, color: u32, x: f32, mid: f32, max: f32) {
    let fitted = ui.fit(label, pt, ui.pt(max.max(0.0)));
    let img = labels.get(ui, &fitted, pt, weight, color);
    let top = ui.pt(mid) - img.h as f32 / 2.0;
    s.blit(img, ui.pt(x) as i32, top as i32, 255, 255);
}

/// A text run centred in a rectangle of points.
fn text_center(labels: &mut TextCache, s: &mut Surface, ui: &Ui, label: &str, pt: f32, weight: Weight, color: u32, x: f32, y: f32, w: f32, h: f32) {
    let fitted = ui.fit(label, pt, ui.pt(w.max(0.0)));
    let text_w = ui.measure(&fitted, pt, weight) / ui.pt(1.0);
    text_mid(labels, s, ui, &fitted, pt, weight, color, x + (w - text_w) / 2.0, y + h / 2.0, w);
}

/// The text of the step on show: a few short lines, from the state the window holds.
pub fn body(w: &Setup) -> Vec<String> {
    let lines: Vec<String> = match w.step {
        Step::Welcome => vec!["Set up BifrOSt in a few steps.".into(), "You can change these choices later in Settings.".into()],
        Step::Language => vec![
            format!("BifrOSt uses {}.", if w.locale.is_empty() { "the system language" } else { w.locale.as_str() }),
            "The language was chosen when BifrOSt was installed.".into(),
        ],
        Step::Region => vec!["Pick the time zone for the clock and the dates.".into()],
        Step::Keyboard => vec!["Choose the keyboard layout you type on.".into()],
        Step::Network => {
            if w.online {
                vec!["This computer is connected to a network.".into()]
            } else {
                vec!["There is no network connection.".into(), "You can connect later in Settings, Wi-Fi.".into()]
            }
        }
        Step::Accessibility => vec!["Allow a quick shortcut that starts the accessibility features.".into()],
        Step::Account => vec![format!("Signed in as {} ({}).", w.account.0, w.account.1), "You can change the picture and password in Settings.".into()],
        Step::Appearance => vec!["Choose the look of the desktop and the colour that tints buttons, switches and selections.".into()],
        Step::Privacy => vec![
            "Location Services lets apps use your location.".into(),
            "Analytics & Diagnostics sends crash reports to the developer.".into(),
            "Both stay off until you turn them on.".into(),
        ],
        Step::Cloud => vec![
            "A BifrOSt Account keeps your settings and backups in step across your computers.".into(),
            "Sign In opens the account page in your browser. You can do this later.".into(),
        ],
        Step::Done => vec!["Your desktop is ready.".into()],
    };
    lines
}

/// Paints the whole window.
pub fn paint(w: &mut Setup, s: &mut Surface, ui: &Ui) {
    s.fill_rrect(0.0, 0.0, s.w as f32, s.h as f32, 0.0, CONTENT_BG.now());
    let cw = (w.size.0 - 2.0 * PAD_PT).max(0.0);
    text_mid(&mut w.labels, s, ui, w.step.title(), MAC_LARGE_TITLE_PT, Weight::Bold, LABEL.now(), PAD_PT, 58.0, cw);
    for (i, line) in body(w).iter().enumerate() {
        let mid = BODY_Y + i as f32 * BODY_LINE_PT + BODY_LINE_PT / 2.0;
        text_mid(&mut w.labels, s, ui, line, MAC_BODY_PT, Weight::Regular, LABEL_2.now(), PAD_PT, mid, cw);
    }

    if w.step == Step::Appearance {
        let y = CONTENT_Y + BTN_PT + GAP_PT + 40.0 - 26.0;
        text_mid(&mut w.labels, s, ui, "Accent colour", MAC_CALLOUT_PT, Weight::Semibold, LABEL.now(), PAD_PT, y, cw);
    }

    for item in w.items() {
        paint_item(&mut w.labels, s, ui, &item);
    }

    if let Some(err) = w.error.clone() {
        let y = w.size.1 - PAD_PT - BTN_PT - 22.0;
        text_mid(&mut w.labels, s, ui, &err, MAC_CALLOUT_PT, Weight::Regular, DESTRUCTIVE, PAD_PT, y, cw);
    }
}

fn paint_item(labels: &mut TextCache, s: &mut Surface, ui: &Ui, item: &Item) {
    let r = item.r;
    match item.kind {
        Kind::Plain => {
            fill(s, ui, r.x, r.y, r.w, r.h, CHIP_R, FIELD_BG.now());
            text_center(labels, s, ui, &item.label, MAC_BODY_PT, Weight::Regular, LABEL.now(), r.x, r.y, r.w, r.h);
        }
        Kind::Primary => {
            fill(s, ui, r.x, r.y, r.w, r.h, CHIP_R, LINK);
            text_center(labels, s, ui, &item.label, MAC_BODY_PT, Weight::Semibold, WHITE, r.x, r.y, r.w, r.h);
        }
        Kind::Chip => {
            let (plate, ink) = if item.on { (LINK, WHITE) } else { (FIELD_BG.now(), LABEL.now()) };
            fill(s, ui, r.x, r.y, r.w, r.h, CHIP_R, plate);
            match item.swatch {
                Some(colour) => {
                    let d = 14.0;
                    fill(s, ui, r.x + 12.0, r.y + (r.h - d) / 2.0, d, d, d / 2.0, colour);
                    text_mid(labels, s, ui, &item.label, MAC_BODY_PT, Weight::Regular, ink, r.x + 34.0, r.y + r.h / 2.0, r.w - 40.0);
                }
                None => text_center(labels, s, ui, &item.label, MAC_BODY_PT, Weight::Regular, ink, r.x, r.y, r.w, r.h),
            }
        }
        Kind::Switch => {
            fill(s, ui, r.x, r.y, r.w, r.h, CARD_R, CARD_BG.now());
            let label_x = r.x + 16.0;
            let track_x = r.x + r.w - 16.0 - TRACK_W;
            let track_y = r.y + (r.h - TRACK_H) / 2.0;
            text_mid(labels, s, ui, &item.label, MAC_BODY_PT, Weight::Regular, LABEL.now(), label_x, r.y + r.h / 2.0, track_x - label_x - 12.0);
            let plate = if item.on { LINK } else { SEPARATOR_C.now() };
            fill(s, ui, track_x, track_y, TRACK_W, TRACK_H, TRACK_H / 2.0, plate);
            let knob_x = if item.on { track_x + TRACK_W - KNOB_PT - 2.0 } else { track_x + 2.0 };
            fill(s, ui, knob_x, track_y + 2.0, KNOB_PT, KNOB_PT, KNOB_PT / 2.0, WHITE);
        }
    }
}
