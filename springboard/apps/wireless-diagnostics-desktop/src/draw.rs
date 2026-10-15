//! Paints the Wireless Diagnostics window: the toolbar, the Connection card, the live signal chart, the nearby networks
//! and the footer. Colours are the kit's semantic tokens, resolved with `now()`, so Light and Dark follow the appearance.

use crate::model::{band_label, chart_fraction, security_label, Rating, HISTORY_LEN};
use crate::source::Reading;
use crate::state::{Diagnostics, Layout, ScanState, MARGIN_PT, ROW_PT};
use bifrost_app::controls::Rect;
use bifrost_app::kit::colors::{SYSTEM_GREEN, SYSTEM_ORANGE, SYSTEM_RED};
use bifrost_app::kit::ui::{CARD_BG, CONTENT_BG, LABEL, LABEL_2, LABEL_3, MAC_BODY_PT, MAC_CAPTION1_PT, MAC_FOOTNOTE_PT, MAC_HEADLINE_PT, SEPARATOR_C, TINT};
use bifrost_app::kit::{Surface, TextCache, Ui, Weight};
use bifrost_app::widgets;
use bifrost_app::wlan::Network;

/// Card corner radius (points), as the grouped cards in the other system apps.
const CARD_R: f32 = 13.0;
/// The width of the Connection card's key column (points).
const KEY_PT: f32 = 120.0;
/// The padding inside a card (points).
const PAD_PT: f32 = 16.0;
/// Where the rows start below a card's title (points).
const TITLE_PT: f32 = 30.0;
/// The dBm levels the chart draws a guide line at.
const GUIDES: [i32; 3] = [-40, -60, -80];

/// The text style of one run.
#[derive(Clone, Copy)]
struct Style {
    pt: f32,
    weight: Weight,
    color: u32,
}

fn style(pt: f32, weight: Weight, color: u32) -> Style {
    Style { pt, weight, color }
}

/// Everything the painter needs, copied out of the state so the text cache can be borrowed mutably meanwhile.
struct View {
    layout: Layout,
    link: crate::model::Link,
    noise: Option<i32>,
    rating: Rating,
    history: Vec<Option<i32>>,
    networks: Vec<Network>,
    scan: ScanState,
    footer: String,
    iface: String,
}

fn view(d: &Diagnostics) -> View {
    View {
        layout: d.layout(),
        link: d.link(),
        noise: d.noise(),
        rating: d.rating(),
        history: d.history.iter().collect(),
        networks: d.networks.clone(),
        scan: d.scan.clone(),
        footer: d.footer_text(),
        iface: d.reading.as_ref().map_or_else(String::new, |r: &Reading| r.iface.clone()),
    }
}

/// A filled rounded rectangle in points.
fn fill(s: &mut Surface, ui: &Ui, r: Rect, radius: f32, color: u32) {
    s.fill_rrect(ui.pt(r.x), ui.pt(r.y), ui.pt(r.w.max(0.0)), ui.pt(r.h.max(0.0)), ui.pt(radius), color);
}

/// A one-point rule: horizontal when `w` is the length, vertical when `h` is.
fn rule(s: &mut Surface, ui: &Ui, x: f32, y: f32, w: f32, h: f32) {
    let t = ui.pt(1.0).max(1.0);
    s.fill_rrect(ui.pt(x), ui.pt(y), ui.pt(w).max(t), ui.pt(h).max(t), 0.0, SEPARATOR_C.now());
}

/// A text run whose vertical centre is `mid`, starting at `x` (or ending at `x` when `right`), cut to `max_w` points.
#[allow(clippy::too_many_arguments)]
fn text(labels: &mut TextCache, s: &mut Surface, ui: &Ui, label: &str, st: Style, x: f32, mid: f32, max_w: f32, right: bool) {
    let fitted = ui.fit(label, st.pt, ui.pt(max_w.max(0.0)));
    let img = labels.get(ui, &fitted, st.pt, st.weight, st.color);
    let (w, h) = (img.w as f32, img.h as f32);
    let left = if right { ui.pt(x) - w } else { ui.pt(x) };
    let top = ui.pt(mid) - h / 2.0;
    s.blit(img, left as i32, top as i32, 255, 255);
}

/// The colour of a rating: green when good, orange when fair, red when poor, grey when unknown.
fn rating_color(rating: Rating) -> u32 {
    match rating {
        Rating::Excellent | Rating::Good => SYSTEM_GREEN.now(),
        Rating::Fair => SYSTEM_ORANGE.now(),
        Rating::Poor => SYSTEM_RED.now(),
        Rating::Unknown => LABEL_2.now(),
    }
}

fn dbm_text(v: Option<i32>) -> String {
    v.map_or_else(|| "Not available".to_string(), |v| format!("{v} dBm"))
}

/// The Connection card's rows: key, value and the value's colour.
fn connection_rows(v: &View) -> Vec<(&'static str, String, u32)> {
    let link = &v.link;
    let (ssid, bssid) = if link.connected {
        (if link.ssid.is_empty() { "Hidden network".to_string() } else { link.ssid.clone() }, link.bssid.clone())
    } else {
        ("Not connected".to_string(), "Not available".to_string())
    };
    let channel = link.channel().map_or_else(|| "Not available".to_string(), |c| c.to_string());
    let snr = crate::model::snr_db(link.signal_dbm, v.noise).map_or_else(|| "Not available".to_string(), |s| format!("{s} dB"));
    let rate = |r: Option<f32>| r.map_or_else(|| "Not available".to_string(), |r| format!("{r:.1} Mbit/s"));
    vec![
        ("Network", ssid, LABEL.now()),
        ("BSSID", bssid, LABEL_2.now()),
        ("Channel", channel, LABEL_2.now()),
        ("Band", band_label(link.band()).to_string(), LABEL_2.now()),
        ("Signal", dbm_text(link.signal_dbm), LABEL.now()),
        ("Noise", dbm_text(v.noise), LABEL_2.now()),
        ("Signal-to-noise", snr, LABEL_2.now()),
        ("Quality", v.rating.label().to_string(), rating_color(v.rating)),
        ("Receive rate", rate(link.rx_mbps), LABEL_2.now()),
        ("Transmit rate", rate(link.tx_mbps), LABEL_2.now()),
        ("Interface", if v.iface.is_empty() { "None".to_string() } else { v.iface.clone() }, LABEL_2.now()),
    ]
}

/// The Connection card: a title and one key and value per row.
fn paint_connection(labels: &mut TextCache, s: &mut Surface, ui: &Ui, v: &View, r: Rect) {
    fill(s, ui, r, CARD_R, CARD_BG.now());
    let title_mid = r.y + TITLE_PT / 2.0 + 2.0;
    text(labels, s, ui, "Connection", style(MAC_HEADLINE_PT, Weight::Semibold, LABEL.now()), r.x + PAD_PT, title_mid, r.w - 2.0 * PAD_PT, false);
    let rows = connection_rows(v);
    let top = r.y + TITLE_PT;
    for (i, (key, value, color)) in rows.iter().enumerate() {
        let row_top = top + i as f32 * ROW_PT;
        if row_top + ROW_PT > r.y + r.h {
            break;
        }
        let mid = row_top + ROW_PT / 2.0;
        text(labels, s, ui, key, style(MAC_BODY_PT, Weight::Regular, LABEL.now()), r.x + PAD_PT, mid, KEY_PT - PAD_PT, false);
        text(labels, s, ui, value, style(MAC_BODY_PT, Weight::Regular, *color), r.x + KEY_PT, mid, r.w - KEY_PT - 2.0 * PAD_PT, false);
        if i + 1 < rows.len() {
            rule(s, ui, r.x + PAD_PT, row_top + ROW_PT, r.w - 2.0 * PAD_PT, 0.0);
        }
    }
}

/// The signal chart: one column per reading, newest at the right, with guide lines at a few signal levels.
fn paint_chart(labels: &mut TextCache, s: &mut Surface, ui: &Ui, v: &View, r: Rect) {
    fill(s, ui, r, CARD_R, CARD_BG.now());
    let title_mid = r.y + TITLE_PT / 2.0 + 2.0;
    text(labels, s, ui, "Signal, last two minutes", style(MAC_HEADLINE_PT, Weight::Semibold, LABEL.now()), r.x + PAD_PT, title_mid, r.w * 0.6, false);
    let latest = v.history.iter().rev().find_map(|x| *x);
    let latest_text = dbm_text(latest);
    text(labels, s, ui, &latest_text, style(MAC_BODY_PT, Weight::Regular, LABEL_2.now()), r.x + r.w - PAD_PT, title_mid, r.w * 0.4, true);

    let plot = Rect {
        x: r.x + PAD_PT,
        y: r.y + TITLE_PT + 8.0,
        w: (r.w - 2.0 * PAD_PT).max(0.0),
        h: (r.h - TITLE_PT - 8.0 - 26.0).max(0.0),
    };
    let bottom = plot.y + plot.h;
    for dbm in GUIDES {
        let y = bottom - chart_fraction(dbm) * plot.h;
        rule(s, ui, plot.x, y, plot.w, 0.0);
        text(labels, s, ui, &format!("{dbm}"), style(MAC_CAPTION1_PT, Weight::Regular, LABEL_3.now()), plot.x, y - 7.0, 40.0, false);
    }
    if v.history.iter().all(|x| x.is_none()) {
        let mid = plot.y + plot.h / 2.0;
        text(labels, s, ui, "Waiting for the first reading", style(MAC_BODY_PT, Weight::Regular, LABEL_2.now()), plot.x, mid, plot.w, false);
    }
    let col = plot.w / HISTORY_LEN as f32;
    let offset = HISTORY_LEN - v.history.len().min(HISTORY_LEN);
    for (k, value) in v.history.iter().enumerate() {
        let Some(dbm) = value else { continue };
        let height = chart_fraction(*dbm) * plot.h;
        let x = plot.x + (offset + k) as f32 * col;
        fill(s, ui, Rect { x, y: bottom - height, w: (col - 1.0).max(1.0), h: height }, 0.0, TINT);
    }
    text(
        labels,
        s,
        ui,
        "One column per second. A gap means no signal was read.",
        style(MAC_CAPTION1_PT, Weight::Regular, LABEL_3.now()),
        r.x + PAD_PT,
        r.y + r.h - 12.0,
        r.w - 2.0 * PAD_PT,
        false,
    );
}

/// The scan line of the networks card, in words.
fn scan_text(scan: &ScanState, count: usize) -> String {
    match scan {
        ScanState::Idle => "Press Scan to list nearby networks".into(),
        ScanState::Scanning => "Scanning".into(),
        ScanState::Done { cached } => {
            let plural = if count == 1 { "" } else { "s" };
            if *cached {
                format!("{count} network{plural} from the last scan")
            } else {
                format!("{count} network{plural}")
            }
        }
        ScanState::Failed(message) => message.clone(),
    }
}

/// The nearby networks card: a title, the scan line, column titles and one row per network, strongest first.
fn paint_networks(labels: &mut TextCache, s: &mut Surface, ui: &Ui, v: &View, r: Rect) {
    fill(s, ui, r, CARD_R, CARD_BG.now());
    let title_mid = r.y + TITLE_PT / 2.0 + 2.0;
    text(labels, s, ui, "Nearby networks", style(MAC_HEADLINE_PT, Weight::Semibold, LABEL.now()), r.x + PAD_PT, title_mid, r.w * 0.4, false);
    let status = scan_text(&v.scan, v.networks.len());
    let status_color = if matches!(v.scan, ScanState::Failed(_)) { SYSTEM_RED.now() } else { LABEL_2.now() };
    text(labels, s, ui, &status, style(MAC_BODY_PT, Weight::Regular, status_color), r.x + r.w - PAD_PT, title_mid, r.w * 0.55, true);

    let inner_x = r.x + PAD_PT;
    let inner_w = (r.w - 2.0 * PAD_PT).max(0.0);
    let cols = Columns::new(inner_x, inner_w);
    let head_mid = r.y + TITLE_PT + 12.0;
    let head = style(MAC_FOOTNOTE_PT, Weight::Semibold, LABEL_2.now());
    text(labels, s, ui, "Network", head, cols.name, head_mid, cols.name_w, false);
    text(labels, s, ui, "Channel", head, cols.channel, head_mid, cols.channel_w, false);
    text(labels, s, ui, "Band", head, cols.band, head_mid, cols.band_w, false);
    text(labels, s, ui, "Security", head, cols.security, head_mid, cols.security_w, false);
    text(labels, s, ui, "Signal", head, cols.signal, head_mid, cols.signal_w, true);

    let first_row = r.y + TITLE_PT + 26.0;
    let bottom = r.y + r.h;
    rule(s, ui, inner_x, first_row - 2.0, inner_w, 0.0);
    let max_rows = ((bottom - first_row) / ROW_PT).floor().max(0.0) as usize;
    for (i, n) in v.networks.iter().take(max_rows).enumerate() {
        let row_top = first_row + i as f32 * ROW_PT;
        let mid = row_top + ROW_PT / 2.0;
        let body = style(MAC_BODY_PT, Weight::Regular, LABEL.now());
        let dim = style(MAC_BODY_PT, Weight::Regular, LABEL_2.now());
        text(labels, s, ui, &n.ssid, body, cols.name, mid, cols.name_w, false);
        text(labels, s, ui, &network_channel(n), dim, cols.channel, mid, cols.channel_w, false);
        text(labels, s, ui, band_label(n.band), dim, cols.band, mid, cols.band_w, false);
        text(labels, s, ui, security_label(n.security), dim, cols.security, mid, cols.security_w, false);
        text(labels, s, ui, &format!("{} dBm", n.rssi), body, cols.signal, mid, cols.signal_w, true);
        if i + 1 < max_rows.min(v.networks.len()) {
            rule(s, ui, inner_x, row_top + ROW_PT, inner_w, 0.0);
        }
    }
    if v.networks.is_empty() && matches!(v.scan, ScanState::Done { .. }) {
        let mid = first_row + ROW_PT;
        text(labels, s, ui, "No networks answered", style(MAC_BODY_PT, Weight::Regular, LABEL_2.now()), inner_x, mid, inner_w, false);
    }
}

fn network_channel(n: &Network) -> String {
    n.channel.map_or_else(|| "Not available".to_string(), |c| c.to_string())
}

/// The x positions of the networks table columns, as fractions of the card's inner width.
struct Columns {
    name: f32,
    name_w: f32,
    channel: f32,
    channel_w: f32,
    band: f32,
    band_w: f32,
    security: f32,
    security_w: f32,
    signal: f32,
    signal_w: f32,
}

impl Columns {
    fn new(x: f32, w: f32) -> Columns {
        let at = |f: f32| x + w * f;
        Columns {
            name: at(0.0),
            name_w: w * 0.36,
            channel: at(0.38),
            channel_w: w * 0.12,
            band: at(0.52),
            band_w: w * 0.13,
            security: at(0.67),
            security_w: w * 0.21,
            // the signal column ends at the right edge of the card
            signal: x + w,
            signal_w: w * 0.12,
        }
    }
}

/// The footer line under the cards.
fn paint_footer(labels: &mut TextCache, s: &mut Surface, ui: &Ui, v: &View, r: Rect) {
    let mid = r.y + r.h / 2.0;
    let width = (r.w - 2.0 * MARGIN_PT).max(0.0);
    text(labels, s, ui, &v.footer, style(MAC_FOOTNOTE_PT, Weight::Regular, LABEL_2.now()), MARGIN_PT, mid, width, false);
}

/// Paints the whole window.
pub fn paint(d: &mut Diagnostics, s: &mut Surface, ui: &Ui) {
    s.fill_rrect(0.0, 0.0, s.w as f32, s.h as f32, 0.0, CONTENT_BG.now());
    let v = view(d);
    let bar = d.toolbar();
    widgets::toolbar_band(s, ui, v.layout.bar);
    widgets::toolbar(s, ui, v.layout.bar, &bar, None);
    let labels = &mut d.labels;
    paint_connection(labels, s, ui, &v, v.layout.connection);
    paint_chart(labels, s, ui, &v, v.layout.chart);
    paint_networks(labels, s, ui, &v, v.layout.networks);
    paint_footer(labels, s, ui, &v, v.layout.footer);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn columns_stay_inside_the_card() {
        let c = Columns::new(16.0, 400.0);
        assert!(c.name + c.name_w <= c.channel + 0.01);
        assert!(c.channel + c.channel_w <= c.band + 0.01);
        assert!(c.band + c.band_w <= c.security + 0.01);
        assert!(c.security + c.security_w <= c.signal + 0.01);
        assert_eq!(c.signal, 416.0);
    }

    #[test]
    fn scan_line_reads_in_words() {
        assert_eq!(scan_text(&ScanState::Done { cached: false }, 1), "1 network");
        assert_eq!(scan_text(&ScanState::Done { cached: true }, 3), "3 networks from the last scan");
        assert_eq!(scan_text(&ScanState::Scanning, 0), "Scanning");
    }

    #[test]
    fn rating_colours_follow_the_scale() {
        assert_eq!(rating_color(Rating::Excellent), SYSTEM_GREEN.now());
        assert_eq!(rating_color(Rating::Poor), SYSTEM_RED.now());
    }
}
