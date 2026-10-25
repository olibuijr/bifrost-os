//! The World Clock's day and night map above the city cards (Golden Gate Clock): the land outline (`land`, Natural
//! Earth, public domain) on an equirectangular projection, the night side shaded behind the terminator computed from
//! the sun's position (`sun`), a hairline every 15 degrees of longitude, and each chosen city as a dot with its name
//! and local time. The map scrolls with the cards and takes its size from the window.

use super::land::{POINTS, RINGS};
use super::sun::{self, Daylight, Sun};
use super::*;
use bifrost_app::kit::cg::path::{Affine, Path, Point};
use bifrost_app::kit::cg::raster::{FillRule, StrokeStyle};
use bifrost_app::kit::ui::{MAP_LAND, MAP_NIGHT, MAP_OCEAN};

/// The latitudes at the map's top and bottom edge: the inhabited world, without the polar caps.
pub const LAT_TOP: f32 = 84.0;
pub const LAT_BOTTOM: f32 = -60.0;
/// Width over height of the projection between those latitudes.
const MAP_ASPECT: f32 = 360.0 / (LAT_TOP - LAT_BOTTOM);
/// The most of the content height the map takes, so the cards stay in view on a short window.
const MAP_MAX_SHARE: f32 = 0.5;
/// Samples along the terminator, west edge to east edge.
const TERMINATOR_STEPS: usize = 180;
/// Longitude between the hairlines (one hour of the sun's travel).
const MERIDIAN_STEP_DEG: i32 = 15;
/// City dot radius and the gap to its label (points).
const DOT_R_PT: f32 = 4.0;
const DOT_LABEL_GAP_PT: f32 = 4.0;
/// World Clock card: the sunrise and sunset line, measured from the card's top.
pub(super) const WORLD_SUN_Y_PT: f32 = 266.0;

/// The map band's height for a window `w` wide with `content_h` of content: the projection's aspect, at most half the
/// content height.
pub fn map_height(w: f32, content_h: f32) -> f32 {
    (w / MAP_ASPECT).min(content_h * MAP_MAX_SHARE).max(0.0).floor()
}

/// Where latitude `lat` and longitude `lon` (degrees) land in `rect` (x, y, w, h).
pub fn project(rect: (f32, f32, f32, f32), lat: f32, lon: f32) -> Point {
    let (x, y, w, h) = rect;
    (x + (lon + 180.0) / 360.0 * w, y + (LAT_TOP - lat) / (LAT_TOP - LAT_BOTTOM) * h)
}

/// The terminator across `rect`, west edge to east edge.
pub fn terminator(sun: &Sun, rect: (f32, f32, f32, f32)) -> Vec<Point> {
    (0..=TERMINATOR_STEPS)
        .map(|i| {
            let lon = -180.0 + 360.0 * i as f64 / TERMINATOR_STEPS as f64;
            project(rect, sun::terminator_lat(sun, lon) as f32, lon as f32)
        })
        .collect()
}

/// The night side's outline: the terminator closed along the bottom edge (northern summer, the south in the dark) or
/// the top edge, a pixel outside `rect` so the shade reaches the edge.
pub fn night_outline(sun: &Sun, rect: (f32, f32, f32, f32)) -> Vec<Point> {
    let mut pts = terminator(sun, rect);
    let edge = if sun::night_is_south(sun) { rect.1 + rect.3 + 1.0 } else { rect.1 - 1.0 };
    pts.push((rect.0 + rect.2, edge));
    pts.push((rect.0, edge));
    pts
}

/// The land outline as one path in `rect` (filled even-odd: the rings include the holes).
pub fn land_path(rect: (f32, f32, f32, f32)) -> Path {
    let mut path = Path::new();
    let mut at = 0;
    for &n in RINGS {
        let ring = &POINTS[at..(at + n as usize).min(POINTS.len())];
        at += n as usize;
        let pts: Vec<Point> = ring.iter().map(|&[lon, lat]| project(rect, lat as f32 / 10.0, lon as f32 / 10.0)).collect();
        path.add_lines(&pts);
        path.close();
    }
    path
}

/// The sunrise and sunset line of a World Clock card.
pub fn daylight_text(city: &City, unix: i64) -> Option<String> {
    let (lat, lon) = sun::coords(city.name)?;
    Some(match sun::daylight(unix, city.offset_at(unix), lat, lon) {
        Daylight::Times(rise, set) => {
            let (r, s) = (city.local(rise), city.local(set));
            format!("Sunrise {:02}:{:02}  Sunset {:02}:{:02}", r.hour, r.min, s.hour, s.min)
        }
        Daylight::MidnightSun => "Sun up all day".to_string(),
        Daylight::PolarNight => "Sun down all day".to_string(),
    })
}

impl Clock {
    /// The map band's height in pixels.
    pub(super) fn world_map_h(&self, ui: &Ui) -> f32 {
        map_height(self.w as f32, self.tab_top(ui) - Self::content_top(ui))
    }

    /// The map band (x, y, w, h), scrolled with the cards.
    pub(super) fn world_map_rect(&self, ui: &Ui) -> (f32, f32, f32, f32) {
        (0.0, Self::content_top(ui) - self.world.scroll.offset(), self.w as f32, self.world_map_h(ui))
    }

    /// Draws the map into `s` (the World Clock's content rows) at Unix time `unix`.
    pub(super) fn draw_world_map(&mut self, s: &mut Surface, ui: &Ui, unix: i64) {
        let rect = self.world_map_rect(ui);
        let (x, y, w, h) = rect;
        if h < 1.0 || y + h <= Self::content_top(ui) {
            return;
        }
        // everything is clipped to the band, so the land south of the bottom edge never reaches the cards
        let mut band = s.rows(y.max(0.0) as usize, (y + h).max(0.0) as usize);
        let s = &mut band;
        s.fill_rect(x as usize, y.max(0.0) as usize, w as usize, h as usize + 1, MAP_OCEAN.now());
        s.fill_path(&land_path(rect), FillRule::EvenOdd, &Affine::IDENTITY, MAP_LAND.now());
        for lon in (-180 + MERIDIAN_STEP_DEG..180).step_by(MERIDIAN_STEP_DEG as usize) {
            let (mx, _) = project(rect, 0.0, lon as f32);
            s.fill_rect(mx as usize, y.max(0.0) as usize, 1, h as usize + 1, SEPARATOR_C.now());
        }
        let sun = sun::sun_at(unix as f64);
        let mut night = Path::new();
        night.add_lines(&night_outline(&sun, rect));
        night.close();
        s.fill_path(&night, FillRule::NonZero, &Affine::IDENTITY, MAP_NIGHT.now());
        let mut line = Path::new();
        line.add_lines(&terminator(&sun, rect));
        s.stroke_path(&line, &StrokeStyle { width: ui.pt(1.0), ..Default::default() }, &Affine::IDENTITY, LABEL_3.now());
        let r = ui.pt(DOT_R_PT);
        let name_h = ui.pt(MAC_BODY_PT);
        let time_dy = ui.pt(MAC_BODY_PT + SPACE_XXS);
        let label_h = time_dy + (name_h + ui.pt(MAC_FOOTNOTE_PT)) / 2.0;
        let mut dots: Vec<Point> = Vec::new();
        let mut sizes: Vec<(f32, f32)> = Vec::new();
        let mut texts: Vec<(&'static str, String)> = Vec::new();
        for index in self.world.selected.clone() {
            let city = &CITIES[index];
            let Some((lat, lon)) = sun::coords(city.name) else { continue };
            dots.push(project(rect, lat as f32, lon as f32));
            let local = city.local(unix);
            let time = format!("{:02}:{:02}", local.hour, local.min);
            let w = ui.measure(city.name, MAC_BODY_PT, Weight::Semibold).max(ui.measure(&time, MAC_FOOTNOTE_PT, Weight::Regular));
            sizes.push((w, label_h));
            texts.push((city.name, time));
        }
        for &(cx, cy) in &dots {
            glyphs::paint(s, (cx - r - 1.0, cy - r - 1.0, 2.0 * r + 2.0, 2.0 * r + 2.0), ORANGE, glyphs::disc((cx, cy), r));
        }
        let places = place_labels(rect, &dots, &sizes, r, ui.pt(DOT_LABEL_GAP_PT), name_h);
        for (place, (name, time)) in places.into_iter().zip(texts) {
            let Some((lx, top)) = place else { continue };
            let mid = top + name_h / 2.0;
            self.label(s, ui, name, MAC_BODY_PT, Weight::Semibold, LABEL.now(), lx, mid, 0.0);
            self.dynamic(s, ui, &time, MAC_FOOTNOTE_PT, Weight::Regular, LABEL.now(), lx, mid + time_dy, 0.0);
        }
    }
}

/// Where each city's label goes on the map: the top left corner of a `sizes[i]` box next to the dot at `dots[i]`
/// (radius `dot_r`, `gap` from the box), or `None` when it has no room. Each label tries the right of its dot (name
/// centred on the dot, `name_h` tall), then the left, above and below, and takes the first place inside `rect` that
/// covers no dot and no label placed before it; a label with no free place is left out (its card still shows it).
pub fn place_labels(rect: (f32, f32, f32, f32), dots: &[Point], sizes: &[(f32, f32)], dot_r: f32, gap: f32, name_h: f32) -> Vec<Option<Point>> {
    type Box4 = (f32, f32, f32, f32);
    let overlaps = |a: Box4, b: Box4| a.0 < b.0 + b.2 && b.0 < a.0 + a.2 && a.1 < b.1 + b.3 && b.1 < a.1 + a.3;
    let inside = |a: Box4| a.0 >= rect.0 && a.1 >= rect.1 && a.0 + a.2 <= rect.0 + rect.2 && a.1 + a.3 <= rect.1 + rect.3;
    let mut taken: Vec<Box4> = dots.iter().map(|&(x, y)| (x - dot_r, y - dot_r, 2.0 * dot_r, 2.0 * dot_r)).collect();
    let clear = dot_r + gap;
    dots.iter()
        .zip(sizes)
        .map(|(&(cx, cy), &(w, h))| {
            let candidates = [
                (cx + clear, cy - name_h / 2.0),
                (cx - clear - w, cy - name_h / 2.0),
                (cx - w / 2.0, cy - clear - h),
                (cx - w / 2.0, cy + clear),
            ];
            let free = candidates.into_iter().find(|&(x, y)| inside((x, y, w, h)) && !taken.iter().any(|&t| overlaps((x, y, w, h), t)))?;
            taken.push((free.0, free.1, w, h));
            Some(free)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const RECT: (f32, f32, f32, f32) = (0.0, 100.0, 1440.0, 576.0);

    #[test]
    fn the_projection_spans_the_band() {
        assert_eq!(project(RECT, LAT_TOP, -180.0), (0.0, 100.0));
        assert_eq!(project(RECT, LAT_BOTTOM, 180.0), (1440.0, 676.0));
        let (x, y) = project(RECT, 0.0, 0.0);
        assert_eq!(x, 720.0);
        assert!((y - (100.0 + 84.0 / 144.0 * 576.0)).abs() < 1e-3);
    }

    #[test]
    fn the_map_keeps_its_aspect_until_it_would_take_more_than_half_the_content() {
        assert_eq!(map_height(1440.0, 2000.0), 576.0);
        assert_eq!(map_height(1440.0, 800.0), 400.0);
        assert_eq!(map_height(0.0, 800.0), 0.0);
    }

    #[test]
    fn the_land_outline_covers_every_ring_and_stays_on_the_globe() {
        let total: usize = RINGS.iter().map(|&n| n as usize).sum();
        assert_eq!(total, POINTS.len());
        assert!(POINTS.iter().all(|&[lon, lat]| (-1800..=1800).contains(&lon) && (-900..=900).contains(&lat)));
    }

    #[test]
    fn the_night_outline_closes_on_the_dark_pole() {
        // 21 June: northern summer, so the outline closes along the bottom edge; 21 December along the top
        let june = sun::sun_at((tz::days_from_civil(2026, 6, 21) * 86_400 + 12 * 3600) as f64);
        let pts = night_outline(&june, RECT);
        assert_eq!(pts.len(), TERMINATOR_STEPS + 3);
        assert!(pts[pts.len() - 1].1 > RECT.1 + RECT.3);
        let december = sun::sun_at((tz::days_from_civil(2026, 12, 21) * 86_400 + 12 * 3600) as f64);
        assert!(night_outline(&december, RECT).last().unwrap().1 < RECT.1);
        // at noon UTC the sun is over Greenwich: under it the terminator runs far south (past the Antarctic circle),
        // at the date line far north, so the line's middle is lower on the map than its ends
        let t = terminator(&june, RECT);
        let mid = t[TERMINATOR_STEPS / 2].1;
        let edge = t[0].1;
        assert!(mid > edge, "in June the terminator is lowest under the sun ({mid} vs {edge})");
    }

    /// E2E8 finding 8: London and Paris sit a few pixels apart; their labels must not print over each other.
    #[test]
    fn map_labels_never_overlap_each_other_or_a_dot() {
        // the nine cities of a fresh World Clock
        let dots: Vec<Point> = CITIES
            .iter()
            .take(9)
            .filter_map(|c| sun::coords(c.name))
            .map(|(lat, lon)| project(RECT, lat as f32, lon as f32))
            .collect();
        let sizes = vec![(110.0, 34.0); dots.len()];
        let places = place_labels(RECT, &dots, &sizes, 4.0, 4.0, 16.0);
        assert_eq!(places.len(), dots.len());
        let boxes: Vec<(f32, f32, f32, f32)> = places.iter().zip(&sizes).filter_map(|(p, &(w, h))| p.map(|(x, y)| (x, y, w, h))).collect();
        assert!(boxes.len() >= dots.len() - 1, "at most one label is crowded out: {places:?}");
        let overlaps = |a: (f32, f32, f32, f32), b: (f32, f32, f32, f32)| a.0 < b.0 + b.2 && b.0 < a.0 + a.2 && a.1 < b.1 + b.3 && b.1 < a.1 + a.3;
        for (i, &a) in boxes.iter().enumerate() {
            assert!(a.0 >= RECT.0 && a.1 >= RECT.1 && a.0 + a.2 <= RECT.0 + RECT.2 && a.1 + a.3 <= RECT.1 + RECT.3, "{a:?} is on the map");
            for &b in &boxes[i + 1..] {
                assert!(!overlaps(a, b), "{a:?} and {b:?} overlap");
            }
            for &(x, y) in &dots {
                assert!(!overlaps(a, (x - 4.0, y - 4.0, 8.0, 8.0)), "{a:?} covers the dot at {x},{y}");
            }
        }
        // of two close cities the first keeps the right of its dot and the second moves to its left
        let close = [(700.0, 300.0), (692.0, 306.0)];
        let p = place_labels(RECT, &close, &[(110.0, 34.0); 2], 4.0, 4.0, 16.0);
        assert_eq!(p, vec![Some((708.0, 292.0)), Some((692.0 - 8.0 - 110.0, 298.0))]);
    }

    #[test]
    fn a_label_at_the_east_edge_goes_left_of_its_dot_and_one_without_room_is_left_out() {
        let p = place_labels(RECT, &[(1430.0, 300.0)], &[(110.0, 34.0)], 4.0, 4.0, 16.0);
        assert_eq!(p[0], Some((1430.0 - 8.0 - 110.0, 292.0)));
        // a box wider than the map has no place
        assert_eq!(place_labels(RECT, &[(720.0, 300.0)], &[(2000.0, 34.0)], 4.0, 4.0, 16.0), vec![None]);
    }

    #[test]
    fn cards_show_sunrise_and_sunset_in_local_time() {
        let reykjavik = CITIES.iter().find(|c| c.name == "Reykjavik").unwrap();
        // 3 October 2026, Reykjavik: sunrise about 07:40, sunset about 18:55 (UTC, no daylight saving)
        let noon = tz::days_from_civil(2026, 10, 3) * 86_400 + 12 * 3600;
        let text = daylight_text(reykjavik, noon).unwrap();
        assert!(text.starts_with("Sunrise 07:"), "{text}");
        assert!(text.contains("Sunset 18:"), "{text}");
        for city in CITIES {
            assert!(daylight_text(city, noon).is_some(), "{}", city.name);
        }
    }
}
