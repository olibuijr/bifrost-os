//! The Clock widget: four analog world clocks (Reykjavik, London, New York,
//! Tokyo) with the city and its time underneath, light faces by day and
//! dark ones by night like the Clock app's World Clock cards.

use super::{put, surface, tile, Kind, PAD_PT};
use crate::gfx::{pack, Image};
use crate::shell::clock::tz::{City, CITIES};
use crate::shell::glyphs;
use crate::shell::ui::{Ui, Weight, BLACK, SECONDARY, WHITE};

/// Indices into `CITIES`.
const SHOWN: [usize; 4] = [0, 1, 5, 7];
const FACE_R_PT: f32 = 32.0;
const FACE_CY_PT: f32 = 58.0;
const NIGHT: u32 = pack(44, 44, 46, 255);

/// The cities shown, in order.
pub fn cities() -> [&'static City; 4] {
    SHOWN.map(|i| &CITIES[i])
}

/// Changes once a minute: the minute of each city's local time.
pub fn key(unix: i64) -> String {
    format!("clock:{}", unix.div_euclid(60))
}

/// Renders the tile for Unix time `unix`.
pub fn render(ui: &Ui, unix: i64) -> Image {
    let s = ui.s;
    let (w, h) = Kind::Clock.size(s);
    let mut img = tile(s, w, h);
    let pitch = (w as f32 - 2.0 * ui.pt(PAD_PT)) / 4.0;
    for (i, city) in cities().into_iter().enumerate() {
        let local = city.local(unix);
        let day = (6..18).contains(&local.hour);
        let (face, ink) = if day { (WHITE, BLACK) } else { (NIGHT, WHITE) };
        let c = (ui.pt(PAD_PT) + pitch * (i as f32 + 0.5), ui.pt(FACE_CY_PT));
        let r = ui.pt(FACE_R_PT);
        let bbox = (c.0 - r - 1.0, c.1 - r - 1.0, 2.0 * r + 2.0, 2.0 * r + 2.0);
        {
            let mut sf = surface(&mut img);
            glyphs::paint(&mut sf, bbox, face, glyphs::disc(c, r));
            let at = |frac: f32, len: f32| (c.0 + r * len * (frac * std::f32::consts::TAU).sin(), c.1 - r * len * (frac * std::f32::consts::TAU).cos());
            for n in 0..12 {
                let f = n as f32 / 12.0;
                let t = if n % 3 == 0 { ui.pt(2.0) } else { ui.pt(1.2) };
                glyphs::paint(&mut sf, bbox, ink, glyphs::segment(at(f, 0.82), at(f, 0.92), t));
            }
            let hour = (local.hour % 12) as f32 / 12.0 + local.min as f32 / 720.0;
            let min = local.min as f32 / 60.0;
            glyphs::paint(&mut sf, bbox, ink, glyphs::either(glyphs::segment(c, at(hour, 0.5), ui.pt(3.0)), glyphs::segment(c, at(min, 0.74), ui.pt(2.4))));
            glyphs::paint(&mut sf, bbox, ink, glyphs::disc(c, ui.pt(2.5)));
        }
        let name = ui.text(&ui.fit(city.name, 12.0, pitch - ui.pt(4.0)), 12.0, Weight::Semibold, WHITE);
        put(&mut img, &name, c.0 - name.w as f32 / 2.0, ui.pt(FACE_CY_PT + FACE_R_PT + 10.0));
        let time = ui.text(&format!("{}:{:02}", local.hour, local.min), 11.0, Weight::Regular, SECONDARY);
        put(&mut img, &time, c.0 - time.w as f32 / 2.0, ui.pt(FACE_CY_PT + FACE_R_PT + 28.0));
    }
    img
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shell::clock::tz::days_from_civil;

    #[test]
    fn key_changes_with_the_minute_and_faces_follow_the_zone() {
        assert_eq!(key(120), key(179));
        assert_ne!(key(120), key(180));
        let ui = crate::shell::tests::ui();
        // 2026-10-03 12:00 UTC: day in Reykjavik and London, night in Tokyo (21:00)
        let noon = days_from_civil(2026, 10, 3) * 86_400 + 12 * 3600;
        let img = render(&ui, noon);
        let (w, h) = Kind::Clock.size(ui.s);
        assert_eq!((img.w, img.h), (w, h));
        let pitch = (w as f32 - 2.0 * ui.pt(PAD_PT)) / 4.0;
        let probe = |i: usize| {
            // towards 4:30, clear of the hands at noon and 21:00
            let cx = (ui.pt(PAD_PT) + pitch * (i as f32 + 0.5) + ui.pt(14.0)) as usize;
            let cy = (ui.pt(FACE_CY_PT) + ui.pt(14.0)) as usize;
            img.px[cy * w + cx]
        };
        assert_eq!(probe(0), WHITE, "Reykjavik at noon has a light face");
        assert_eq!(probe(3), NIGHT, "Tokyo at 21:00 has a dark face");
        assert_eq!(cities()[2].name, "New York");
    }
}
