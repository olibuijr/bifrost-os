//! The Deildu "Continue watching" widget: the last played title with its
//! poster from the app's artwork cache.

use super::{put, surface, tile, Kind, PAD_PT};
use crate::deildu::art;
use crate::deildu::site::clean_title;
use crate::deildu::wrap;
use crate::gfx::{rrect_mask, Image};
use crate::shell::ui::{Ui, Weight, CELL, SECONDARY, WHITE};
use std::path::Path;

/// What the shell remembers about the last title Deildu played.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Played {
    pub id: String,
    pub title: String,
    pub url: String,
}

impl Played {
    /// One preferences line: `id|url|title` (the title goes last, it may hold anything).
    pub fn encode(&self) -> String {
        format!("{}|{}|{}", self.id, self.url, self.title)
    }

    pub fn decode(line: &str) -> Option<Played> {
        let mut it = line.splitn(3, '|');
        let (id, url, title) = (it.next()?, it.next()?, it.next()?);
        (!id.is_empty()).then(|| Played { id: id.into(), url: url.into(), title: title.into() })
    }
}

const POSTER_RATIO: f32 = 1.5;
const BLUE: u32 = crate::deildu::BLUE;

pub fn key(played: Option<&Played>, poster: bool) -> String {
    format!("deildu:{}:{poster}", played.map(|p| p.id.as_str()).unwrap_or(""))
}

/// Poster size in px: the tile height less the padding, 2:3.
pub fn poster_size(ui: &Ui) -> (usize, usize) {
    let h = ui.pt(super::SMALL_PT - 2.0 * 12.0);
    ((h / POSTER_RATIO).round() as usize, h.round() as usize)
}

/// Loads the cached artwork for `id` from Deildu's cache, cropped to the poster box.
pub fn poster(ui: &Ui, deildu_dir: &Path, id: &str) -> Option<Image> {
    let bytes = art::disk_read(deildu_dir, &format!("{id}.img"))?;
    let img = art::decode(&bytes)?;
    let (w, h) = poster_size(ui);
    let mut c = art::cover(&img, w, h);
    c.apply_mask(&rrect_mask(w, h, ui.pt(8.0)));
    Some(c)
}

pub fn render(ui: &Ui, played: Option<&Played>, poster: Option<&Image>) -> Image {
    let s = ui.s;
    let (w, h) = Kind::Deildu.size(s);
    let mut img = tile(s, w, h);
    let (pw, ph) = poster_size(ui);
    let (px, py) = (ui.pt(12.0), ui.pt(12.0));
    match poster {
        Some(p) => put(&mut img, p, px, py),
        None => surface(&mut img).fill_rrect(px, py, pw as f32, ph as f32, ui.pt(8.0), CELL),
    }
    let x = px + pw as f32 + ui.pt(PAD_PT);
    let max_w = w as f32 - x - ui.pt(PAD_PT);
    let head = ui.text("CONTINUE WATCHING", 11.0, Weight::Semibold, SECONDARY);
    put(&mut img, &head, x, ui.pt(PAD_PT));
    match played {
        Some(p) => {
            let mut y = ui.pt(PAD_PT + 22.0);
            for line in wrap(ui, &clean_title(&p.title), 20.0, max_w).into_iter().take(2) {
                let t = ui.text(&ui.fit(&line, 20.0, max_w), 20.0, Weight::Semibold, WHITE);
                put(&mut img, &t, x, y);
                y += t.h as f32 + ui.pt(2.0);
            }
            let resume = ui.text("Resume in Deildu", 13.0, Weight::Semibold, BLUE);
            put(&mut img, &resume, x, h as f32 - ui.pt(PAD_PT) - resume.h as f32);
        }
        None => {
            let t = ui.text("Nothing played yet", 17.0, Weight::Regular, SECONDARY);
            put(&mut img, &t, x, ui.pt(PAD_PT + 24.0));
        }
    }
    img
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gfx::pack;

    #[test]
    fn played_round_trips_through_one_line() {
        let p = Played { id: "42".into(), title: "A|B 2024".into(), url: "https://x/42".into() };
        assert_eq!(Played::decode(&p.encode()), Some(p.clone()));
        assert_eq!(Played::decode("|u|t"), None);
        assert_eq!(Played::decode("junk"), None);
    }

    #[test]
    fn poster_comes_from_the_cache_and_the_tile_shows_it() {
        let ui = crate::shell::tests::ui();
        let dir = std::path::PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/target/test-deildu-widget")).join(std::process::id().to_string());
        let _ = std::fs::remove_dir_all(&dir);
        assert!(poster(&ui, &dir, "7").is_none());
        art::disk_write(&dir, "7.img", include_bytes!("../../deildu/fixtures/dot.jpg"));
        let p = poster(&ui, &dir, "7").expect("decoded");
        assert_eq!((p.w, p.h), poster_size(&ui));
        let played = Played { id: "7".into(), title: "Some.Film.2024.1080p".into(), url: String::new() };
        let with = render(&ui, Some(&played), Some(&p));
        let without = render(&ui, Some(&played), None);
        let none = render(&ui, None, None);
        assert_eq!((with.w, with.h), Kind::Deildu.size(ui.s));
        let (pw, ph) = poster_size(&ui);
        let centre = (ui.pt(12.0) as usize + ph / 2) * with.w + ui.pt(12.0) as usize + pw / 2;
        assert_eq!(without.px[centre], pack(28, 28, 30, 255), "placeholder box without art");
        assert_ne!(with.px[centre], without.px[centre]);
        assert_ne!(none.px, without.px);
        assert_ne!(key(Some(&played), true), key(Some(&played), false));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
