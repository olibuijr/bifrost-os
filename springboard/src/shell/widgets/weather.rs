//! The Weather widget: place, big temperature, the condition glyph and the
//! day's high and low, from the summary the Weather app saved.

use super::{put, surface, tile, Kind, PAD_PT};
use crate::gfx::Image;
use crate::shell::ui::{Ui, Weight, SECONDARY, WHITE};
use crate::shell::weather::data::Summary;
use crate::shell::weather::glyph;

pub fn key(s: Option<&Summary>) -> String {
    format!("weather:{}", s.map_or(String::new(), Summary::to_line))
}

pub fn render(ui: &Ui, s: &Summary) -> Image {
    let sc = ui.s;
    let (w, h) = Kind::Weather.size(sc);
    let mut img = tile(sc, w, h);
    let pad = ui.pt(PAD_PT);
    let place = ui.text(&ui.fit(&s.place, 15.0, w as f32 - 2.0 * pad - ui.pt(40.0)), 15.0, Weight::Semibold, WHITE);
    put(&mut img, &place, pad, pad - ui.pt(2.0));
    let temp = ui.text(&s.units.degrees(s.temp), 44.0, Weight::Regular, WHITE);
    put(&mut img, &temp, pad - ui.pt(2.0), pad + ui.pt(18.0));
    glyph::draw(&mut surface(&mut img), s.sky(), s.day, (w as f32 - pad - ui.pt(14.0), pad + ui.pt(14.0)), ui.pt(15.0));
    let sky = ui.text(s.sky().label(), 13.0, Weight::Medium, WHITE);
    put(&mut img, &sky, pad, h as f32 - pad - ui.pt(32.0));
    let hl = ui.text(&format!("H:{}  L:{}", s.units.degrees(s.max), s.units.degrees(s.min)), 13.0, Weight::Regular, SECONDARY);
    put(&mut img, &hl, pad, h as f32 - pad - hl.h as f32 + ui.pt(2.0));
    img
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shell::weather::data::Units;

    fn summary(temp: f64) -> Summary {
        Summary { place: "Reykjav\u{ed}k".into(), temp, code: 61, day: true, max: 9.0, min: 2.0, units: Units::Celsius }
    }

    #[test]
    fn renders_a_tile_and_keys_on_the_data() {
        let ui = crate::shell::tests::ui();
        let s = summary(7.0);
        assert_ne!(key(Some(&s)), key(Some(&summary(8.0))));
        assert_ne!(key(Some(&s)), key(None));
        let img = render(&ui, &s);
        assert_eq!((img.w, img.h), Kind::Weather.size(ui.s));
        assert!(img.px[img.w * img.h / 2..].iter().any(|&p| p >> 24 > 200 && p & 0xff > 150), "text and glyph paint");
    }
}
