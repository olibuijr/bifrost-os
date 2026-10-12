//! Browser-style address suggestions: Top Hits from bookmarks and history ranked by frecency, then
//! "Search for ...". Ranking is pure; drawing uses named tokens only.
use crate::store::{Entry, Store};
use bifrost_app::controls::prefs_layout::m;
use bifrost_app::controls::Rect;
use bifrost_app::kit::ui::{Weight, CARD_BG, LABEL, LABEL_2, MAC_BODY_PT, MAC_SUBHEAD_PT, SELECTION, SEPARATOR_C, SPACE_M, SPACE_S, SPACE_XS, SPACE_XXL, SPACE_XXS, WHITE};
use bifrost_app::glyphs;
use bifrost_app::kit::{Surface, Ui};

pub const MAX_SUGGESTIONS: usize = 8;
/// Row height, points.
pub const ROW_PT: f32 = 36.0;

#[derive(Clone, Debug, PartialEq)]
pub enum Kind {
    Bookmark,
    History,
    Search,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Suggestion {
    pub kind: Kind,
    pub title: String,
    /// The URL to open, or the query text for `Kind::Search`.
    pub target: String,
}

/// Visits weighted by how recent the last one was (half-life of about a week).
pub fn frecency(e: &Entry, now: u64) -> f64 {
    let age_days = now.saturating_sub(e.last) as f64 / 86_400.0;
    (e.visits.max(1) as f64) * 0.5f64.powf(age_days / 7.0)
}

/// Case-insensitive substring test without allocating; `q` is already lowercase.
fn contains_ci(hay: &str, q: &str) -> bool {
    if q.is_ascii() {
        let (h, n) = (hay.as_bytes(), q.as_bytes());
        return n.is_empty() || h.windows(n.len()).any(|w| w.eq_ignore_ascii_case(n));
    }
    hay.to_lowercase().contains(q)
}

fn matches(e: &Entry, q: &str) -> bool {
    contains_ci(&e.url, q) || contains_ci(&e.title, q)
}

fn label(e: &Entry) -> String {
    if e.title.is_empty() { e.url.clone() } else { e.title.clone() }
}

pub fn suggestions(store: &Store, input: &str, now: u64) -> Vec<Suggestion> {
    let q = input.trim().to_lowercase();
    let mut out: Vec<Suggestion> = Vec::new();
    if q.is_empty() {
        let mut recent: Vec<&Entry> = store.bookmarks.iter().collect();
        recent.sort_by(|a, b| b.last.cmp(&a.last));
        for e in recent.into_iter().take(MAX_SUGGESTIONS) {
            out.push(Suggestion { kind: Kind::Bookmark, title: label(e), target: e.url.clone() });
        }
        return out;
    }
    for e in store.bookmarks.iter().filter(|e| matches(e, &q)) {
        out.push(Suggestion { kind: Kind::Bookmark, title: label(e), target: e.url.clone() });
    }
    let mut hist: Vec<&Entry> = store.history.iter().filter(|e| matches(e, &q) && !store.is_bookmarked(&e.url)).collect();
    hist.sort_by_cached_key(|e| std::cmp::Reverse((frecency(e, now) * 1e6) as u64));
    for e in hist {
        out.push(Suggestion { kind: Kind::History, title: label(e), target: e.url.clone() });
    }
    out.truncate(MAX_SUGGESTIONS - 1);
    out.push(Suggestion { kind: Kind::Search, title: format!("Search for \u{201c}{}\u{201d}", input.trim()), target: input.trim().to_string() });
    out
}

/// Height of the dropdown, points.
pub fn height(n: usize) -> f32 {
    n as f32 * ROW_PT
}

/// The suggestion row under (`px`, `py`) in points, for a dropdown under `field`.
pub fn hit(field: Rect, n: usize, px: f32, py: f32) -> Option<usize> {
    let top = field.y + field.h + 4.0;
    if n == 0 || px < field.x || px > field.x + field.w || py < top || py >= top + height(n) {
        return None;
    }
    Some((((py - top) / ROW_PT) as usize).min(n - 1))
}

/// Draw the dropdown under `field` (points); `selected` is highlighted.
pub fn draw(s: &mut Surface, ui: &Ui, field: Rect, list: &[Suggestion], selected: Option<usize>, fav: &mut crate::favicon::Favicons) {
    if list.is_empty() {
        return;
    }
    let (x, y) = (ui.pt(field.x), ui.pt(field.y + field.h + 4.0));
    let (w, h) = (ui.pt(field.w), ui.pt(height(list.len())));
    s.fill_rrect(x, y, w, h, ui.pt(m::CARD_RADIUS), CARD_BG.now());
    for (i, sg) in list.iter().enumerate() {
        let ry = y + ui.pt(i as f32 * ROW_PT);
        if selected == Some(i) {
            s.fill_rrect(x + ui.pt(SPACE_XS), ry + ui.pt(SPACE_XXS), w - ui.pt(SPACE_S), ui.pt(ROW_PT - 4.0), ui.pt(m::SELECTION_RADIUS), SELECTION.now());
        } else if i > 0 {
            s.fill_rect(x as usize + ui.pt(SPACE_M) as usize, ry as usize, (w - ui.pt(SPACE_XXL)) as usize, 1, SEPARATOR_C.now());
        }
        let room = w - ui.pt(28.0);
        // selected rows sit on the selection colour: white text in both appearances (named pair, no tint-on-label)
        let on = selected == Some(i);
        let cy = ry as i32 + ui.pt(ROW_PT / 2.0) as i32;
        // leading 16 pt slot: the site's icon, a globe while there is none, a magnifier for a search
        let icon_px = ui.pt(crate::favicon::ICON_PT).round() as usize;
        let slot = ui.pt(crate::favicon::ICON_PT + 8.0) as i32;
        let (ix, iy) = (x as i32 + ui.pt(14.0) as i32, cy - icon_px as i32 / 2);
        let site = if sg.kind == Kind::Search { None } else { crate::favicon::host_key(&sg.target) };
        match site.as_deref().and_then(|h| fav.get(h, icon_px)) {
            Some(ic) => s.blit(ic, ix, iy, 255, 255),
            None => {
                let p = ui.pt(1.0);
                let r = Rect { x: ix as f32 / p, y: iy as f32 / p, w: crate::favicon::ICON_PT, h: crate::favicon::ICON_PT };
                glyphs::glyph(s, ui, if sg.kind == Kind::Search { "magnifyingglass" } else { "globe" }, r, if on { WHITE } else { LABEL_2.now() });
            }
        }
        let t = ui.text(&ui.fit(&sg.title, MAC_BODY_PT, room * 0.6 - slot as f32), MAC_BODY_PT, Weight::Regular, if on { WHITE } else { LABEL.now() });
        s.blit(&t, x as i32 + ui.pt(14.0) as i32 + slot, cy - t.h as i32 / 2, 255, 255);
        if sg.kind != Kind::Search {
            let tag = if sg.kind == Kind::Bookmark { "Bookmark" } else { "History" };
            let g = ui.text(tag, MAC_SUBHEAD_PT, Weight::Regular, if on { WHITE } else { LABEL_2.now() });
            s.blit(&g, (x + w) as i32 - ui.pt(14.0) as i32 - g.w as i32, cy - g.h as i32 / 2, 255, 255);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bookmarks_first_then_frecent_history_then_search() {
        let mut st = Store::in_memory();
        let day = 86_400;
        st.visit("https://a.test/", "Alpha", 100 * day);
        st.visit("https://b.test/", "Alpha two", 100 * day);
        st.visit("https://b.test/", "Alpha two", 100 * day);
        st.visit("https://c.test/", "Alpha old", 10 * day);
        st.toggle_bookmark("https://c.test/", "Alpha old", 100 * day);
        let s = suggestions(&st, "alpha", 100 * day);
        let urls: Vec<&str> = s.iter().map(|x| x.target.as_str()).collect();
        assert_eq!(urls, ["https://c.test/", "https://b.test/", "https://a.test/", "alpha"]);
        assert_eq!(s.last().unwrap().kind, Kind::Search);
    }

    #[test]
    fn history_dedupes_caps_and_skips_non_web() {
        let mut st = Store::in_memory();
        st.visit("about:blank", "", 1);
        st.visit("https://a.test/", "A", 1);
        st.visit("https://a.test/", "A", 2);
        assert_eq!(st.history.len(), 1);
        assert_eq!(st.history[0].visits, 2);
        for i in 0..(crate::store::MAX_HISTORY + 10) {
            st.visit(&format!("https://x.test/{i}"), "", 10 + i as u64);
        }
        assert_eq!(st.history.len(), crate::store::MAX_HISTORY);
        st.private = true;
        st.visit("https://p.test/", "", 99_999);
        assert!(!st.history.iter().any(|e| e.url == "https://p.test/"));
    }

    #[test]
    fn serialization_round_trips() {
        let e = vec![Entry { url: "https://a.test/?x=1".into(), title: "T\tab\nline\\".into(), visits: 3, last: 7 }];
        assert_eq!(crate::store::parse(&crate::store::serialize(&e)), e);
    }
}
