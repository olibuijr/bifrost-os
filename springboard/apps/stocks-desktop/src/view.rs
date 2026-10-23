//! Stocks' layout and drawing (Golden Gate Stocks): a sidebar with the add field and the watchlist, each row with
//! the symbol and its name, a sparkline of the last month and the price over a green or red change pill; and the
//! detail pane with the selected symbol's name, price, change and price chart. The geometry is computed once
//! (`layout`) for both drawing and hit testing. All positions are pixels; the layout constants are points, scaled
//! with `Ui::pt`. Rows are reordered and removed from their context menu or with the Delete key.

use crate::data;
use crate::feed::Feed;
use crate::model::{self, Bar, Quote};
use crate::state::{now_unix, Stocks};
use bifrost_app::kit::ui::{
    CARD_BG, CONTENT_BG, LABEL, LABEL_2, MAC_BODY_PT, MAC_CAPTION1_PT, MAC_FOOTNOTE_PT, MAC_LARGE_TITLE_PT, MAC_SUBHEAD_PT,
    MAC_TITLE1_PT, MAC_TITLE3_PT, SEPARATOR_C, SIDEBAR_BG, SIDEBAR_PLATE, SPACE_L, SPACE_M, SPACE_S, SPACE_XS, SPACE_XXL,
    SYSTEM_GREEN, SYSTEM_RED, WHITE,
};
use bifrost_app::kit::{Surface, TextCache, Ui, Weight};
use bifrost_app::controls::prefs_layout::m::{CARD_RADIUS, SELECTION_RADIUS};
use bifrost_app::layout::UNIFIED_BAR_PT;

/// Points: the side margin inside the sidebar and the detail pane.
const MARGIN_PT: f32 = 16.0;
/// Points: the sidebar's width range and its share of the window.
const SIDEBAR_MIN_PT: f32 = 260.0;
const SIDEBAR_MAX_PT: f32 = 340.0;
const SIDEBAR_SHARE: f32 = 0.34;
/// Points: the add field's height and its top, under the band.
const FIELD_H_PT: f32 = 28.0;
const FIELD_TOP_PT: f32 = UNIFIED_BAR_PT + 8.0;
/// Points: the line under the field for notes, and where the list starts.
const NOTE_H_PT: f32 = 16.0;
const LIST_TOP_PT: f32 = FIELD_TOP_PT + FIELD_H_PT + NOTE_H_PT + 6.0;
/// Points: the source credit's line at the foot of the sidebar (`data::ATTRIBUTION`).
const FOOTER_PT: f32 = 28.0;
/// Points: one watchlist row (symbol over name, sparkline, price over the change pill).
const ROW_PT: f32 = 56.0;
/// Points: the sparkline's width and height, and the change pill's size.
const SPARK_W_PT: f32 = 58.0;
const SPARK_H_PT: f32 = 22.0;
const PILL_W_PT: f32 = 62.0;
const PILL_H_PT: f32 = 18.0;
/// Points: the detail header's height above the chart card.
const HEADER_PT: f32 = 66.0;
/// Points: the corner radius of the cards.
const RADIUS_PT: f32 = CARD_RADIUS;
/// What a row without a price says while quotes are out, when its symbol is unknown, and after a failed request.
pub const ROW_LOADING: &str = "Loading\u{2026}";
pub const ROW_NOT_FOUND: &str = "Not Found";
pub const ROW_UNAVAILABLE: &str = "Unavailable";

/// A pixel rectangle: x, y, width, height.
pub type Rect = (f32, f32, f32, f32);

/// What a press landed on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
    Field,
    Row(usize),
}

/// The screen's rectangles in pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Layout {
    /// The sidebar's width.
    pub side: f32,
    pub field: Rect,
    /// The middle of the note line under the field (pixels).
    pub note_y: f32,
    pub list: Rect,
    /// The middle of the source credit's line at the foot of the sidebar (pixels).
    pub footer_y: f32,
    /// The detail pane: the header and the chart card under it.
    pub chart: Rect,
    /// One row's height in pixels.
    pub row_px: f32,
}

/// The screen's rectangles for a window of `size` pixels.
pub fn layout(ui: &Ui, size: (usize, usize)) -> Layout {
    let (w, h) = (size.0 as f32, size.1 as f32);
    let margin = ui.pt(MARGIN_PT);
    let side = (w * SIDEBAR_SHARE).clamp(ui.pt(SIDEBAR_MIN_PT), ui.pt(SIDEBAR_MAX_PT)).min(w * 0.5).round();
    let list_top = ui.pt(LIST_TOP_PT);
    let chart_top = ui.pt(UNIFIED_BAR_PT + SPACE_S);
    Layout {
        side,
        field: (margin, ui.pt(FIELD_TOP_PT), (side - 2.0 * margin).max(0.0), ui.pt(FIELD_H_PT)),
        note_y: ui.pt(FIELD_TOP_PT + FIELD_H_PT + 4.0 + NOTE_H_PT / 2.0),
        list: (0.0, list_top, side, (h - list_top - ui.pt(FOOTER_PT)).max(0.0)),
        footer_y: h - ui.pt(FOOTER_PT / 2.0),
        chart: (side + margin, chart_top, (w - side - 2.0 * margin).max(0.0), (h - chart_top - margin).max(0.0)),
        row_px: ui.pt(ROW_PT),
    }
}

/// Whether a pixel position lies inside the watchlist.
pub fn in_list(ui: &Ui, size: (usize, usize), x: f32, y: f32) -> bool {
    inside(layout(ui, size).list, x, y)
}

fn inside(r: Rect, x: f32, y: f32) -> bool {
    x >= r.0 && x < r.0 + r.2 && y >= r.1 && y < r.1 + r.3
}

/// The control under a pixel position: the field or a row.
pub fn hit(s: &Stocks, ui: &Ui, x: f32, y: f32) -> Option<Hit> {
    let l = layout(ui, s.size);
    if inside(l.field, x, y) {
        return Some(Hit::Field);
    }
    if !inside(l.list, x, y) {
        return None;
    }
    let index = ((y - l.list.1 + s.scroll.offset()) / l.row_px).floor();
    if index < 0.0 || index as usize >= s.list.len() {
        return None;
    }
    Some(Hit::Row(index as usize))
}

#[derive(Clone, Copy, PartialEq)]
enum Align {
    Left,
    Centre,
    Right,
}

/// Draws `text` with its anchor at pixel `x` (left, centre or right edge) and its middle at pixel `mid`.
#[allow(clippy::too_many_arguments)]
fn put(labels: &mut TextCache, s: &mut Surface, ui: &Ui, text: &str, pt: f32, weight: Weight, color: u32, align: Align, x: f32, mid: f32) {
    let img = labels.get(ui, text, pt, weight, color);
    let left = match align {
        Align::Left => x,
        Align::Centre => x - img.w as f32 / 2.0,
        Align::Right => x - img.w as f32,
    };
    let top = mid - img.h as f32 / 2.0;
    s.blit(img, left.round() as i32, top.round() as i32, 255, 255);
}

/// A filled pixel rectangle. The surface clips it to its own rows.
fn fill(s: &mut Surface, x: f32, y: f32, w: f32, h: f32, color: u32) {
    s.fill_rect(x.max(0.0) as usize, y.max(0.0) as usize, w.max(0.0) as usize, h.max(0.0) as usize, color);
}

/// "just now", "5 min ago" or "3 h ago" for an age in seconds.
pub fn age_text(secs: f64) -> String {
    if secs < 60.0 {
        "just now".to_string()
    } else if secs < 3600.0 {
        format!("{} min ago", (secs / 60.0) as u64)
    } else {
        format!("{} h ago", (secs / 3600.0) as u64)
    }
}

/// The line in the band: the state of the quotes.
fn status(feed: &Feed, now: f64) -> String {
    if feed.is_quotes_pending() {
        "Updating quotes...".to_string()
    } else if feed.quotes_error.is_some() {
        if feed.quotes.is_empty() { "Quotes unavailable".to_string() } else { "Offline. Showing saved quotes.".to_string() }
    } else {
        feed.quotes_at.map_or(String::new(), |at| format!("Updated {}", age_text(now - at)))
    }
}

/// What a row without a price says: "Not Found" for a symbol the service does not know, "Loading" while quotes are
/// out or before the first answer, "Unavailable" after a failed request (E2E W4-14: a dash stayed for ever before).
pub fn row_status(feed: &Feed, symbol: &str) -> &'static str {
    if feed.quote_failed.contains_key(symbol) {
        ROW_NOT_FOUND
    } else if feed.is_quotes_pending() || (feed.quotes_error.is_none() && feed.quotes_at.is_none()) {
        ROW_LOADING
    } else {
        ROW_UNAVAILABLE
    }
}

/// The colour of a change: green for a rise or no change, red for a fall.
fn change_colour(pct: f64) -> u32 {
    if pct < 0.0 { SYSTEM_RED } else { SYSTEM_GREEN }
}

/// "+4.23" or "-0.57": a change with its sign.
fn signed(value: f64) -> String {
    format!("{value:+.2}")
}

/// A line through `points` (pixels), `width` thick.
fn polyline(s: &mut Surface, points: &[(f32, f32)], width: f32, color: u32) {
    if let [only] = points {
        fill(s, only.0 - width, only.1 - width, 2.0 * width, 2.0 * width, color);
    }
    for pair in points.windows(2) {
        let ((xa, ya), (xb, yb)) = (pair[0], pair[1]);
        let steps = (xb - xa).abs().max((yb - ya).abs()).ceil().max(1.0) as usize;
        for k in 0..=steps {
            let f = k as f32 / steps as f32;
            let (px, py) = (xa + (xb - xa) * f, ya + (yb - ya) * f);
            fill(s, px - width / 2.0, py - width / 2.0, width, width, color);
        }
    }
}

/// The points of `values` spread across `r` (the high at the top), in pixels.
fn plot(values: &[f64], r: Rect) -> Vec<(f32, f32)> {
    let Some((lo, hi)) = model::value_range(values) else { return Vec::new() };
    let (lo, hi) = model::padded_range(lo, hi);
    let n = values.len();
    values
        .iter()
        .enumerate()
        .map(|(i, &v)| {
            let px = if n == 1 { r.0 + r.2 / 2.0 } else { r.0 + r.2 * i as f32 / (n - 1) as f32 };
            (px, model::plot_y(v, lo, hi, r.1, r.3))
        })
        .collect()
}

impl Stocks {
    /// Draws the whole screen into `s`.
    pub fn draw(&mut self, s: &mut Surface, ui: &Ui, t: f64) {
        self.dirty = false;
        let l = layout(ui, (s.w, s.h));
        s.fill_rect(0, 0, s.w, s.h, CONTENT_BG.now());
        // the sidebar runs the window's full height, under the traffic lights, with a hairline at its edge
        fill(s, 0.0, 0.0, l.side, s.h as f32, SIDEBAR_BG.now());
        fill(s, l.side - 1.0, 0.0, 1.0, s.h as f32, SEPARATOR_C.now());
        self.draw_band(s, ui);
        self.draw_field(s, ui, &l, t);
        if let Some(note) = self.note.clone() {
            let note = ui.fit(&note, MAC_FOOTNOTE_PT, l.field.2);
            put(&mut self.labels, s, ui, &note, MAC_FOOTNOTE_PT, Weight::Regular, LABEL_2.now(), Align::Left, l.field.0 + ui.pt(SPACE_XS), l.note_y);
        }
        self.draw_list(s, ui, &l);
        // the source of the prices, under the list as the reference credits it (privacy-first: who answers is shown)
        let credit = ui.fit(data::ATTRIBUTION, MAC_FOOTNOTE_PT, (l.side - 2.0 * ui.pt(MARGIN_PT)).max(0.0));
        put(&mut self.labels, s, ui, &credit, MAC_FOOTNOTE_PT, Weight::Regular, LABEL_2.now(), Align::Centre, l.side / 2.0, l.footer_y);
        self.draw_chart(s, ui, &l);
    }

    /// The quote status at the trailing end of the band.
    fn draw_band(&mut self, s: &mut Surface, ui: &Ui) {
        let mid = ui.pt(UNIFIED_BAR_PT / 2.0);
        let right = s.w as f32 - ui.pt(MARGIN_PT);
        let text = status(&self.feed, now_unix());
        if !text.is_empty() {
            put(&mut self.labels, s, ui, &text, MAC_CAPTION1_PT, Weight::Regular, LABEL_2.now(), Align::Right, right, mid);
        }
    }

    /// The add field: a capsule at the top of the sidebar.
    fn draw_field(&self, s: &mut Surface, ui: &Ui, l: &Layout, t: f64) {
        let (x, y, w, h) = l.field;
        s.fill_rrect(x, y, w, h, h / 2.0, CARD_BG.now());
        self.field.draw(s, ui, (x + ui.pt(SPACE_M), y, w - ui.pt(SPACE_XXL), h), t);
    }

    fn draw_list(&mut self, s: &mut Surface, ui: &Ui, l: &Layout) {
        let (x, y, w, h) = l.list;
        let row = l.row_px;
        let content = self.list.len() as f32 * row;
        self.scroll.set_max((content - h).max(0.0));
        let offset = self.scroll.offset();
        let (top, bottom) = (y as usize, (y + h) as usize);
        let symbols = self.list.symbols().to_vec();
        if symbols.is_empty() {
            let msg = ui.fit("Type a symbol above to add one.", MAC_BODY_PT, w - 2.0 * ui.pt(MARGIN_PT));
            put(&mut self.labels, s, ui, &msg, MAC_BODY_PT, Weight::Regular, LABEL_2.now(), Align::Centre, x + w / 2.0, y + ui.pt(SPACE_XXL));
            return;
        }
        let selected = self.list.selected();
        let count = symbols.len();
        let inset = ui.pt(SPACE_S);
        let left = x + ui.pt(MARGIN_PT);
        let right = x + w - ui.pt(MARGIN_PT);
        let spark_r = right - ui.pt(PILL_W_PT) - ui.pt(SPACE_M);
        let spark_l = spark_r - ui.pt(SPARK_W_PT);
        let mut c = s.rows(top, bottom);
        for (i, sym) in symbols.iter().enumerate() {
            let ry = y - offset + i as f32 * row;
            if ry + row < top as f32 || ry > bottom as f32 {
                continue;
            }
            let mid = ry + row / 2.0;
            if i == selected {
                c.fill_rrect(x + inset, ry + 2.0, w - 2.0 * inset, row - 4.0, ui.pt(SELECTION_RADIUS), SIDEBAR_PLATE.now());
            } else if i + 1 < count && i + 1 != selected {
                fill(&mut c, left, ry + row - 1.0, right - left, 1.0, SEPARATOR_C.now());
            }
            let quote = self.feed.quotes.get(sym).cloned();
            let text_w = spark_l - left - ui.pt(SPACE_S);
            let shown = ui.fit(&data::display_symbol(sym), MAC_BODY_PT, text_w);
            put(&mut self.labels, &mut c, ui, &shown, MAC_BODY_PT, Weight::Bold, LABEL.now(), Align::Left, left, mid - ui.pt(8.0));
            if let Some(name) = quote.as_ref().map(|q| q.name.clone()).filter(|n| !n.is_empty()) {
                let name = ui.fit(&name, MAC_SUBHEAD_PT, text_w);
                put(&mut self.labels, &mut c, ui, &name, MAC_SUBHEAD_PT, Weight::Regular, LABEL_2.now(), Align::Left, left, mid + ui.pt(9.0));
            }
            match quote {
                Some(q) => self.draw_row_quote(&mut c, ui, &q, (spark_l, spark_r), right, mid),
                None => {
                    let word = row_status(&self.feed, sym);
                    let ink = if word == ROW_NOT_FOUND { SYSTEM_RED } else { LABEL_2.now() };
                    put(&mut self.labels, &mut c, ui, word, MAC_SUBHEAD_PT, Weight::Regular, ink, Align::Right, right, mid);
                }
            }
        }
    }

    /// A row's sparkline (the previous close dotted across it), the price and the change pill.
    fn draw_row_quote(&mut self, c: &mut Surface, ui: &Ui, q: &Quote, (spark_l, spark_r): (f32, f32), right: f32, mid: f32) {
        let colour = change_colour(q.change());
        let spark_h = ui.pt(SPARK_H_PT);
        if q.spark.len() >= 2 {
            // the previous close goes into the scale too, so its dotted line stays inside the sparkline's box
            let mut values = q.spark.clone();
            values.push(q.open);
            let points = plot(&values, (spark_l, mid - spark_h / 2.0, spark_r - spark_l, spark_h));
            let base_y = points.last().map_or(mid, |p| p.1);
            let mut dx = spark_l;
            while dx < spark_r {
                fill(c, dx, base_y, ui.pt(2.0), 1.0, LABEL_2.now());
                dx += ui.pt(4.0);
            }
            polyline(c, &points[..points.len() - 1], ui.pt(1.2), colour);
        }
        put(&mut self.labels, c, ui, &format!("{:.2}", q.close), MAC_BODY_PT, Weight::Semibold, LABEL.now(), Align::Right, right, mid - ui.pt(9.0));
        let (pw, ph) = (ui.pt(PILL_W_PT), ui.pt(PILL_H_PT));
        let (px, py) = (right - pw, mid + ui.pt(2.0));
        c.fill_rrect(px, py, pw, ph, ui.pt(SPACE_XS), colour);
        put(&mut self.labels, c, ui, &signed(q.change()), MAC_SUBHEAD_PT, Weight::Semibold, WHITE, Align::Right, right - ui.pt(SPACE_XS + 2.0), py + ph / 2.0);
    }

    fn draw_chart(&mut self, s: &mut Surface, ui: &Ui, l: &Layout) {
        let (x, y, w, h) = l.chart;
        let Some(symbol) = self.list.selected_symbol().map(str::to_string) else {
            put(&mut self.labels, s, ui, "Add a symbol to see its chart.", MAC_BODY_PT, Weight::Regular, LABEL_2.now(), Align::Centre, x + w / 2.0, y + h / 2.0);
            return;
        };
        let quote = self.feed.quotes.get(&symbol).cloned();
        let bars: Option<Vec<Bar>> = self.feed.history.get(&symbol).cloned();
        let (left, right) = (x, x + w);
        // the header: the symbol large with its name after it, and the price at the trailing end
        let title_mid = y + ui.pt(22.0);
        let title = data::display_symbol(&symbol);
        put(&mut self.labels, s, ui, &title, MAC_LARGE_TITLE_PT, Weight::Bold, LABEL.now(), Align::Left, left, title_mid);
        if let Some(name) = quote.as_ref().map(|q| q.name.clone()).filter(|n| !n.is_empty()) {
            let after = left + ui.measure(&title, MAC_LARGE_TITLE_PT, Weight::Bold) + ui.pt(SPACE_M);
            let room = (right - after - ui.pt(160.0)).max(0.0);
            let name = ui.fit(&name, MAC_TITLE3_PT, room);
            put(&mut self.labels, s, ui, &name, MAC_TITLE3_PT, Weight::Regular, LABEL_2.now(), Align::Left, after, title_mid + ui.pt(2.0));
        }
        let last = quote.as_ref().map(|q| q.close).or_else(|| bars.as_ref().and_then(|b| b.last()).map(|b| b.close));
        if let Some(price) = last {
            put(&mut self.labels, s, ui, &format!("{:.2}", price), MAC_TITLE1_PT, Weight::Semibold, LABEL.now(), Align::Right, right, title_mid);
        }
        let sub_mid = y + ui.pt(48.0);
        if let Some(q) = &quote {
            put(&mut self.labels, s, ui, &format!("Priced {}", q.stamp), MAC_CAPTION1_PT, Weight::Regular, LABEL_2.now(), Align::Left, left, sub_mid);
            let pct = q.change_pct();
            let change = format!("{} ({:+.2}%) since the previous close", signed(q.change()), pct);
            put(&mut self.labels, s, ui, &change, MAC_CAPTION1_PT, Weight::Medium, change_colour(pct), Align::Right, right, sub_mid);
        } else if let Some(message) = self.feed.quote_failed.get(&symbol).cloned() {
            put(&mut self.labels, s, ui, &message, MAC_CAPTION1_PT, Weight::Regular, SYSTEM_RED, Align::Left, left, sub_mid);
        }
        // the chart card under the header
        let card = (x, y + ui.pt(HEADER_PT), w, (h - ui.pt(HEADER_PT)).max(0.0));
        s.fill_rrect(card.0, card.1, card.2, card.3, ui.pt(RADIUS_PT), CARD_BG.now());
        let plot_x0 = card.0 + ui.pt(SPACE_L);
        let plot_x1 = card.0 + card.2 - ui.pt(SPACE_L + 52.0);
        let plot_y0 = card.1 + ui.pt(SPACE_L);
        let plot_y1 = card.1 + card.3 - ui.pt(30.0);
        let values: Vec<f64> = bars.as_ref().map(|b| b.iter().map(|x| x.close).collect()).unwrap_or_default();
        let Some((vlo, vhi)) = model::value_range(&values) else {
            let msg = if self.feed.is_history_pending(&symbol) {
                "Loading price history\u{2026}".to_string()
            } else {
                self.feed.history_failed.get(&symbol).cloned().unwrap_or_else(|| "No price history yet.".to_string())
            };
            put(&mut self.labels, s, ui, &msg, MAC_BODY_PT, Weight::Regular, LABEL_2.now(), Align::Centre, card.0 + card.2 / 2.0, (plot_y0 + plot_y1) / 2.0);
            return;
        };
        let plot_h = plot_y1 - plot_y0;
        let plot_w = plot_x1 - plot_x0;
        for gy in [plot_y0, plot_y0 + plot_h / 2.0, plot_y1] {
            fill(s, plot_x0, gy, plot_w, 1.0, SEPARATOR_C.now());
        }
        // the value scale on the trailing side, as the reference draws it
        put(&mut self.labels, s, ui, &format!("{:.2}", vhi), MAC_CAPTION1_PT, Weight::Regular, LABEL_2.now(), Align::Left, plot_x1 + ui.pt(6.0), plot_y0);
        put(&mut self.labels, s, ui, &format!("{:.2}", vlo), MAC_CAPTION1_PT, Weight::Regular, LABEL_2.now(), Align::Left, plot_x1 + ui.pt(6.0), plot_y1);
        if let (Some(first), Some(last_bar)) = (bars.as_ref().and_then(|b| b.first()), bars.as_ref().and_then(|b| b.last())) {
            put(&mut self.labels, s, ui, &first.date, MAC_CAPTION1_PT, Weight::Regular, LABEL_2.now(), Align::Left, plot_x0, plot_y1 + ui.pt(SPACE_L));
            put(&mut self.labels, s, ui, &last_bar.date, MAC_CAPTION1_PT, Weight::Regular, LABEL_2.now(), Align::Right, plot_x1, plot_y1 + ui.pt(SPACE_L));
        }
        let points = plot(&values, (plot_x0, plot_y0, plot_w, plot_h));
        let line = if values.last() >= values.first() { SYSTEM_GREEN } else { SYSTEM_RED };
        polyline(s, &points, ui.pt(1.5), line);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ages_read_in_plain_words() {
        assert_eq!(age_text(5.0), "just now");
        assert_eq!(age_text(90.0), "1 min ago");
        assert_eq!(age_text(3.0 * 3600.0 + 5.0), "3 h ago");
    }

    #[test]
    fn a_falling_quote_is_red_and_a_flat_or_rising_one_is_green() {
        assert_eq!(change_colour(-0.1), SYSTEM_RED);
        assert_eq!(change_colour(0.0), SYSTEM_GREEN);
        assert_eq!(change_colour(2.0), SYSTEM_GREEN);
        assert_eq!(signed(4.234), "+4.23");
        assert_eq!(signed(-0.57), "-0.57");
    }

    #[test]
    fn the_status_line_says_what_the_quotes_are() {
        let mut feed = Feed::default();
        assert_eq!(status(&feed, 100.0), "", "nothing yet");
        feed.quotes_error = Some("down".into());
        assert_eq!(status(&feed, 100.0), "Quotes unavailable");
        feed.load_quotes("Symbol,Date,Time,Previous,Close,Name,Spark\nAAPL,2026-10-02,22:00,100,105,Apple Inc.,\n", 0.0);
        assert_eq!(status(&feed, 100.0), "Offline. Showing saved quotes.");
        feed.quotes_error = None;
        assert_eq!(status(&feed, 60.0 * 7.0), "Updated 7 min ago");
    }

    /// E2E W4-14: a row without a price says why instead of a dash.
    #[test]
    fn a_row_without_a_price_says_why() {
        let mut feed = Feed::default();
        assert_eq!(row_status(&feed, "AAPL"), ROW_LOADING, "before the first answer");
        feed.begin_quotes();
        assert_eq!(row_status(&feed, "AAPL"), ROW_LOADING);
        feed.apply(crate::feed::Done::Quotes(Err("down".into())), 10.0);
        assert_eq!(row_status(&feed, "AAPL"), ROW_UNAVAILABLE);
        feed.apply(crate::feed::Done::Quotes(Ok((vec![], vec![("NOPE".into(), "NOPE was not found.".into())]))), 100.0);
        assert_eq!(row_status(&feed, "NOPE"), ROW_NOT_FOUND);
    }

    #[test]
    fn the_layout_puts_the_list_in_a_sidebar_and_the_chart_beside_it() {
        let ui = crate::test_ui();
        let l = layout(&ui, (1875, 1219));
        assert!(l.side >= ui.pt(SIDEBAR_MIN_PT) && l.side <= ui.pt(SIDEBAR_MAX_PT));
        assert_eq!(l.list.0, 0.0);
        assert!(l.list.0 + l.list.2 <= l.side && l.chart.0 >= l.side, "the chart is right of the sidebar");
        assert!(l.field.0 + l.field.2 <= l.side);
        assert!(l.list.1 + l.list.3 <= l.footer_y - ui.pt(FOOTER_PT / 2.0) + 0.5, "the source credit sits under the list");
        assert!(l.footer_y < 1219.0);
        assert!(ui.measure(data::ATTRIBUTION, MAC_FOOTNOTE_PT, Weight::Regular) <= l.side - 2.0 * ui.pt(MARGIN_PT), "the credit fits the sidebar");
        let points = plot(&[1.0, 3.0, 2.0], (0.0, 10.0, 100.0, 50.0));
        assert_eq!(points.len(), 3);
        assert!(points[1].1 < points[0].1, "the high is above the low");
        assert_eq!((points[0].0, points[2].0), (0.0, 100.0));
    }
}
