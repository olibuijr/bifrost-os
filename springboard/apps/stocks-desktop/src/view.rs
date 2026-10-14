//! Stocks' layout and drawing. The screen is one column under the unified toolbar band: the add field, the watchlist
//! with a quote on each row and the Up, Down and remove buttons, and a price chart of the selected symbol. The
//! geometry is computed once (`layout`) for both drawing and hit testing. All positions are pixels; the layout
//! constants are points, scaled with `Ui::pt`.

use crate::data;
use crate::feed::Feed;
use crate::model::{self, Bar};
use crate::state::{now_unix, Stocks};
use bifrost_app::kit::appearance::Pair;
use bifrost_app::kit::ui::{
    CARD_BG, CONTENT_BG, DESTRUCTIVE, LABEL, LABEL_2, LINK, MAC_BODY_PT, MAC_CAPTION1_PT, MAC_FOOTNOTE_PT, MAC_TITLE1_PT,
    MAC_TITLE3_PT, SEPARATOR_C, SPACE_L, SPACE_M, SPACE_XXL, SYSTEM_GREEN, SYSTEM_RED,
};
use bifrost_app::kit::{pack, Surface, TextCache, Ui, Weight};
use bifrost_app::layout::{traffic_lights_trailing, UNIFIED_BAR_PT};

/// Points: the side margin of the column.
const MARGIN_PT: f32 = 16.0;
/// Points: the add field's height and its top, under the band.
const FIELD_H_PT: f32 = 32.0;
const FIELD_TOP_PT: f32 = UNIFIED_BAR_PT + 10.0;
/// Points: the line under the field for notes, and where the list card starts.
const NOTE_H_PT: f32 = 16.0;
const LIST_TOP_PT: f32 = FIELD_TOP_PT + FIELD_H_PT + NOTE_H_PT + 8.0;
/// Points: one watchlist row, and the width of each row button.
const ROW_PT: f32 = 44.0;
const BUTTON_PT: f32 = 36.0;
/// Points: the chart card's height and the gap above it.
const CHART_PT: f32 = 300.0;
const GAP_PT: f32 = 12.0;
/// Points: the corner radius of the cards.
const RADIUS_PT: f32 = 10.0;

/// A pixel rectangle: x, y, width, height.
pub type Rect = (f32, f32, f32, f32);

/// What a press landed on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
    Field,
    Row(usize),
    Up(usize),
    Down(usize),
    Remove(usize),
}

/// The screen's rectangles in pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Layout {
    pub field: Rect,
    /// The top of the note line under the field (pixels).
    pub note_y: f32,
    pub list: Rect,
    pub chart: Rect,
    /// One row's height in pixels.
    pub row_px: f32,
}

/// The screen's rectangles for a window of `size` pixels.
pub fn layout(ui: &Ui, size: (usize, usize)) -> Layout {
    let (w, h) = (size.0 as f32, size.1 as f32);
    let margin = ui.pt(MARGIN_PT);
    let card_w = (w - 2.0 * margin).max(0.0);
    let chart_h = ui.pt(CHART_PT);
    let chart_y = (h - margin - chart_h).max(0.0);
    let list_top = ui.pt(LIST_TOP_PT);
    let list_bottom = (chart_y - ui.pt(GAP_PT)).max(list_top);
    Layout {
        field: (margin, ui.pt(FIELD_TOP_PT), card_w, ui.pt(FIELD_H_PT)),
        note_y: ui.pt(FIELD_TOP_PT + FIELD_H_PT + 4.0),
        list: (margin, list_top, card_w, list_bottom - list_top),
        chart: (margin, chart_y, card_w, chart_h),
        row_px: ui.pt(ROW_PT),
    }
}

/// Whether a pixel row lies inside the watchlist card.
pub fn in_list(ui: &Ui, size: (usize, usize), y: f32) -> bool {
    let l = layout(ui, size);
    y >= l.list.1 && y < l.list.1 + l.list.3
}

fn inside(r: Rect, x: f32, y: f32) -> bool {
    x >= r.0 && x < r.0 + r.2 && y >= r.1 && y < r.1 + r.3
}

/// The control under a pixel position: the field, a row, or one of a row's three buttons (right to left: remove,
/// down, up).
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
    let i = index as usize;
    let button = ui.pt(BUTTON_PT);
    let right = l.list.0 + l.list.2 - ui.pt(SPACE_M);
    if x >= right - button {
        Some(Hit::Remove(i))
    } else if x >= right - 2.0 * button {
        Some(Hit::Down(i))
    } else if x >= right - 3.0 * button {
        Some(Hit::Up(i))
    } else {
        Some(Hit::Row(i))
    }
}

/// A colour that stands for a selected row: a fixed tone for both appearances.
const SELECTED: Pair = Pair::new(pack(58, 58, 62, 255), pack(226, 226, 232, 255));

#[derive(Clone, Copy, PartialEq)]
enum Align {
    Left,
    Centre,
    Right,
}

/// Draws `text` with its anchor at pixel `x` (left, centre or right edge) and its middle at pixel `mid`.
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

/// The colour of a change: green for a rise or no change, red for a fall.
fn change_colour(pct: f64) -> u32 {
    if pct < 0.0 { SYSTEM_RED } else { SYSTEM_GREEN }
}

impl Stocks {
    /// Draws the whole screen into `s`.
    pub fn draw(&mut self, s: &mut Surface, ui: &Ui, t: f64) {
        self.dirty = false;
        let l = layout(ui, (s.w, s.h));
        s.fill_rect(0, 0, s.w, s.h, CONTENT_BG.now());
        self.draw_band(s, ui);
        self.draw_field(s, ui, &l, t);
        if let Some(note) = self.note.clone() {
            put(&mut self.labels, s, ui, &note, MAC_FOOTNOTE_PT, Weight::Regular, LABEL_2.now(), Align::Left, l.field.0 + ui.pt(SPACE_L), l.note_y);
        }
        self.draw_list(s, ui, &l);
        self.draw_chart(s, ui, &l);
    }

    /// The title at the left of the band (after the traffic lights) and the quote status at the right.
    fn draw_band(&mut self, s: &mut Surface, ui: &Ui) {
        let mid = ui.pt(UNIFIED_BAR_PT / 2.0);
        put(&mut self.labels, s, ui, "Stocks", MAC_TITLE3_PT, Weight::Semibold, LABEL.now(), Align::Left, ui.pt(traffic_lights_trailing() + SPACE_M), mid);
        let right = s.w as f32 - ui.pt(MARGIN_PT);
        let text = status(&self.feed, now_unix());
        if !text.is_empty() {
            put(&mut self.labels, s, ui, &text, MAC_CAPTION1_PT, Weight::Regular, LABEL_2.now(), Align::Right, right, mid);
        }
    }

    fn draw_field(&self, s: &mut Surface, ui: &Ui, l: &Layout, t: f64) {
        let (x, y, w, h) = l.field;
        s.fill_rrect(x, y, w, h, ui.pt(RADIUS_PT), CARD_BG.now());
        self.field.draw(s, ui, (x + ui.pt(SPACE_M), y, w - ui.pt(SPACE_XXL), h), t);
    }

    fn draw_list(&mut self, s: &mut Surface, ui: &Ui, l: &Layout) {
        let (x, y, w, h) = l.list;
        s.fill_rrect(x, y, w, h, ui.pt(RADIUS_PT), CARD_BG.now());
        let row = l.row_px;
        let content = self.list.len() as f32 * row;
        self.scroll.set_max((content - h).max(0.0));
        let offset = self.scroll.offset();
        let (top, bottom) = (y as usize, (y + h) as usize);
        let symbols = self.list.symbols().to_vec();
        if symbols.is_empty() {
            put(&mut self.labels, s, ui, "Your watchlist is empty. Type a symbol above to add one.", MAC_BODY_PT, Weight::Regular, LABEL_2.now(), Align::Centre, x + w / 2.0, y + ui.pt(24.0));
            return;
        }
        let selected = self.list.selected();
        let count = symbols.len();
        let right = x + w - ui.pt(SPACE_M);
        let button = ui.pt(BUTTON_PT);
        let price_x = right - 3.0 * button - ui.pt(SPACE_M);
        let pct_x = price_x - ui.pt(92.0);
        let mut c = s.rows(top, bottom);
        for (i, sym) in symbols.iter().enumerate() {
            let ry = y - offset + i as f32 * row;
            if ry + row < top as f32 || ry > bottom as f32 {
                continue;
            }
            let mid = ry + row / 2.0;
            if i == selected {
                fill(&mut c, x, ry, w, row, SELECTED.now());
            }
            if i + 1 < count {
                fill(&mut c, x + ui.pt(SPACE_L), ry + row - 1.0, w - 2.0 * ui.pt(SPACE_L), 1.0, SEPARATOR_C.now());
            }
            put(&mut self.labels, &mut c, ui, &data::display_symbol(sym), MAC_BODY_PT, Weight::Semibold, LABEL.now(), Align::Left, x + ui.pt(SPACE_L), mid);
            match self.feed.quotes.get(sym) {
                Some(q) => {
                    put(&mut self.labels, &mut c, ui, &format!("{:.2}", q.close), MAC_BODY_PT, Weight::Semibold, LABEL.now(), Align::Right, price_x, mid);
                    let pct = q.change_pct();
                    put(&mut self.labels, &mut c, ui, &format!("{:+.2}%", pct), MAC_BODY_PT, Weight::Medium, change_colour(pct), Align::Right, pct_x, mid);
                }
                None => put(&mut self.labels, &mut c, ui, "\u{2013}", MAC_BODY_PT, Weight::Regular, LABEL_2.now(), Align::Right, price_x, mid),
            }
            let slot = |n: f32| right - button * n + button / 2.0;
            put(&mut self.labels, &mut c, ui, "Up", MAC_CAPTION1_PT, Weight::Medium, if i == 0 { LABEL_2.now() } else { LINK }, Align::Centre, slot(2.5), mid);
            put(&mut self.labels, &mut c, ui, "Down", MAC_CAPTION1_PT, Weight::Medium, if i + 1 == count { LABEL_2.now() } else { LINK }, Align::Centre, slot(1.5), mid);
            put(&mut self.labels, &mut c, ui, "\u{d7}", MAC_TITLE1_PT, Weight::Regular, DESTRUCTIVE, Align::Centre, slot(0.5), mid);
        }
    }

    fn draw_chart(&mut self, s: &mut Surface, ui: &Ui, l: &Layout) {
        let (x, y, w, h) = l.chart;
        s.fill_rrect(x, y, w, h, ui.pt(RADIUS_PT), CARD_BG.now());
        let Some(symbol) = self.list.selected_symbol().map(str::to_string) else {
            put(&mut self.labels, s, ui, "Add a symbol to see its chart.", MAC_BODY_PT, Weight::Regular, LABEL_2.now(), Align::Centre, x + w / 2.0, y + h / 2.0);
            return;
        };
        let quote = self.feed.quotes.get(&symbol).cloned();
        let bars: Option<Vec<Bar>> = self.feed.history.get(&symbol).cloned();
        let left = x + ui.pt(SPACE_L);
        let right = x + w - ui.pt(SPACE_L);
        put(&mut self.labels, s, ui, &data::display_symbol(&symbol), MAC_TITLE3_PT, Weight::Semibold, LABEL.now(), Align::Left, left, y + ui.pt(26.0));
        let last = quote.as_ref().map(|q| q.close).or_else(|| bars.as_ref().and_then(|b| b.last()).map(|b| b.close));
        if let Some(price) = last {
            put(&mut self.labels, s, ui, &format!("{:.2}", price), MAC_TITLE1_PT, Weight::Semibold, LABEL.now(), Align::Right, right, y + ui.pt(26.0));
        }
        if let Some(q) = &quote {
            put(&mut self.labels, s, ui, &format!("Priced {}", q.stamp), MAC_CAPTION1_PT, Weight::Regular, LABEL_2.now(), Align::Left, left, y + ui.pt(46.0));
            let pct = q.change_pct();
            put(&mut self.labels, s, ui, &format!("{:+.2} ({:+.2}%) since the open", q.change(), pct), MAC_CAPTION1_PT, Weight::Medium, change_colour(pct), Align::Right, right, y + ui.pt(46.0));
        }
        let plot_x0 = x + ui.pt(SPACE_L + 52.0);
        let plot_x1 = right;
        let plot_y0 = y + ui.pt(86.0);
        let plot_y1 = y + h - ui.pt(30.0);
        let values: Vec<f64> = bars.as_ref().map(|b| b.iter().map(|x| x.close).collect()).unwrap_or_default();
        let Some((vlo, vhi)) = model::value_range(&values) else {
            let msg = if self.feed.is_history_pending(&symbol) {
                "Loading price history...".to_string()
            } else {
                self.feed.history_failed.get(&symbol).cloned().unwrap_or_else(|| "No price history yet.".to_string())
            };
            put(&mut self.labels, s, ui, &msg, MAC_BODY_PT, Weight::Regular, LABEL_2.now(), Align::Centre, (plot_x0 + plot_x1) / 2.0, (plot_y0 + plot_y1) / 2.0);
            return;
        };
        let (lo, hi) = model::padded_range(vlo, vhi);
        let plot_h = plot_y1 - plot_y0;
        let plot_w = plot_x1 - plot_x0;
        for gy in [plot_y0, plot_y0 + plot_h / 2.0, plot_y1] {
            fill(s, plot_x0, gy, plot_w, 1.0, SEPARATOR_C.now());
        }
        put(&mut self.labels, s, ui, &format!("{:.2}", vhi), MAC_CAPTION1_PT, Weight::Regular, LABEL_2.now(), Align::Right, plot_x0 - ui.pt(6.0), plot_y0);
        put(&mut self.labels, s, ui, &format!("{:.2}", vlo), MAC_CAPTION1_PT, Weight::Regular, LABEL_2.now(), Align::Right, plot_x0 - ui.pt(6.0), plot_y1);
        if let (Some(first), Some(last_bar)) = (bars.as_ref().and_then(|b| b.first()), bars.as_ref().and_then(|b| b.last())) {
            put(&mut self.labels, s, ui, &first.date, MAC_CAPTION1_PT, Weight::Regular, LABEL_2.now(), Align::Left, plot_x0, plot_y1 + ui.pt(16.0));
            put(&mut self.labels, s, ui, &last_bar.date, MAC_CAPTION1_PT, Weight::Regular, LABEL_2.now(), Align::Right, plot_x1, plot_y1 + ui.pt(16.0));
        }
        let n = values.len();
        let points: Vec<(f32, f32)> = values
            .iter()
            .enumerate()
            .map(|(i, &v)| {
                let px = if n == 1 { (plot_x0 + plot_x1) / 2.0 } else { plot_x0 + plot_w * i as f32 / (n - 1) as f32 };
                (px, model::plot_y(v, lo, hi, plot_y0, plot_h))
            })
            .collect();
        let rising = values.last() >= values.first();
        let line = if rising { SYSTEM_GREEN } else { SYSTEM_RED };
        if n == 1 {
            fill(s, points[0].0 - 1.0, points[0].1 - 1.0, 3.0, 3.0, line);
        }
        for pair in points.windows(2) {
            let ((xa, ya), (xb, yb)) = (pair[0], pair[1]);
            let steps = (xb - xa).ceil().max(1.0) as usize;
            for k in 0..=steps {
                let f = k as f32 / steps as f32;
                let (px, py) = (xa + (xb - xa) * f, ya + (yb - ya) * f);
                fill(s, px - 1.0, py - 1.0, 2.0, 2.0, line);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ages_read_in_plain_words() {
        assert_eq!(age_text(5.0), "just now");
        assert_eq!(age_text(90.0), "1 min ago");
        assert_eq!(age_text(3 * 3600.0 + 5.0), "3 h ago");
    }

    #[test]
    fn a_falling_quote_is_red_and_a_flat_or_rising_one_is_green() {
        assert_eq!(change_colour(-0.1), SYSTEM_RED);
        assert_eq!(change_colour(0.0), SYSTEM_GREEN);
        assert_eq!(change_colour(2.0), SYSTEM_GREEN);
    }

    #[test]
    fn the_status_line_says_what_the_quotes_are() {
        let mut feed = Feed::default();
        assert_eq!(status(&feed, 100.0), "", "nothing yet");
        feed.quotes_error = Some("down".into());
        assert_eq!(status(&feed, 100.0), "Quotes unavailable");
        feed.load_quotes("Symbol,Date,Time,Open,High,Low,Close,Volume\nAAPL.US,2026-10-02,22:00:09,100,1,1,105,5\n", 0.0);
        assert_eq!(status(&feed, 100.0), "Offline. Showing saved quotes.");
        feed.quotes_error = None;
        assert_eq!(status(&feed, 60.0 * 7.0), "Updated 7 min ago");
    }
}
