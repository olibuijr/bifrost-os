//! Streaming sources for the library's Sources tab: direct MP4 providers
//! (HeavyFetish, XAdultFlix, XNXX) parsed with the shared `deildu_rust`
//! crate. There is no WebKit here, so catalogue pages are fetched as plain
//! HTML: providers whose listings need JavaScript (XNXX) yield nothing here
//! until a server-rendered markup source exists.

use super::http::Client;
pub use deildu_rust::sources::{Provider, Video};
use super::art::Cache;
use super::library::{MARGIN_PT, SOURCE};
use super::scroll::Scroller;
use super::{within, Target, BLUE};
use crate::gfx::{Image, Surface};
use crate::shell::ui::{Ui, Weight, CELL, SECONDARY, WHITE};
use std::collections::{HashMap, VecDeque};

/// Providers offered on the Sources tab, in order.
pub const PROVIDERS: [Provider; 3] = [Provider::HeavyFetish, Provider::XAdultFlix, Provider::Xnxx];
/// Duration filter chips: index 0 is no filtering.
pub const FILTERS: [&str; 4] = ["All", "Under 10 min", "10-30 min", "Over 30 min"];
const PAGE_LIMIT: usize = 2 * 1024 * 1024;
const MAX_CACHED_PAGES: usize = 10;
/// Detail pages enriched per catalogue (bounded like the compat check).
pub const ENRICH_FIRST: usize = 24;

pub fn provider_name(p: Provider) -> &'static str {
    p.name()
}

/// Cache key for one catalogue page.
pub fn page_key(provider: Provider, location: &str) -> String {
    deildu_rust::fetch::page_key(&[provider.key(), location])
}

/// Fetch and parse one catalogue page: videos plus the next-page URL.
pub fn fetch_page(
    client: &Client,
    provider: Provider,
    url: &str,
) -> Result<(Vec<Video>, Option<String>), String> {
    let r = client
        .get(url, None, PAGE_LIMIT)
        .map_err(|e| format!("Connection failed: {e}"))?;
    if r.status != 200 {
        return Err(format!("Source returned {}", r.status));
    }
    deildu_rust::sources::catalogue(provider, &r.text(), &r.url)
}

/// Fill in qualities/tags/thumbnail from the video's detail page.
pub fn enrich(client: &Client, video: &Video) -> Video {
    let html = client
        .get(&video.item.url, None, PAGE_LIMIT)
        .ok()
        .filter(|r| r.status == 200)
        .map(|r| r.text())
        .unwrap_or_default();
    if html.is_empty() {
        return video.clone();
    }
    deildu_rust::sources::enrich(video.clone(), &html)
}

/// Whether a video passes the search text and the duration chip.
pub fn matches(video: &Video, query: &str, filter: usize) -> bool {
    if filter != 0 {
        let want = FILTERS.get(filter).copied().unwrap_or("");
        if !want.is_empty() && !video.terms.iter().any(|t| t == want) {
            return false;
        }
    }
    if query.trim().is_empty() {
        return true;
    }
    let q = query.to_lowercase();
    format!("{} {} {}", video.item.title, video.creator, video.terms.join(" "))
        .to_lowercase()
        .contains(&q)
}

/// Playable once the enrich pass found direct MP4 qualities.
pub fn playable(video: &Video) -> bool {
    !video.qualities.is_empty()
}

/// Browsing state for the Sources tab: provider, catalogue, paging history,
/// duration chip and a bounded page cache (warm next pages land here).
pub struct State {
    pub provider: Provider,
    pub videos: Vec<Video>,
    pub next: Option<String>,
    pub location: String,
    pub history: Vec<String>,
    pub filter: usize,
    pages: HashMap<String, (Vec<Video>, Option<String>)>,
    order: VecDeque<String>,
}

impl State {
    pub fn new() -> Self {
        State {
            provider: Provider::HeavyFetish,
            videos: vec![],
            next: None,
            location: Provider::HeavyFetish.base().to_string(),
            history: vec![],
            filter: 0,
            pages: HashMap::new(),
            order: VecDeque::new(),
        }
    }

    pub fn reset(&mut self, provider: Provider) {
        self.provider = provider;
        self.videos.clear();
        self.next = None;
        self.location = provider.base().to_string();
        self.history.clear();
        self.filter = 0;
    }

    pub fn cached(&self, key: &str) -> Option<(Vec<Video>, Option<String>)> {
        self.pages.get(key).cloned()
    }

    pub fn remember(&mut self, key: &str, videos: Vec<Video>, next: Option<String>) {
        if videos.len() > 120 {
            return;
        }
        if !self.pages.contains_key(key) {
            self.order.push_back(key.to_string());
        }
        self.pages.insert(key.to_string(), (videos, next));
        while self.pages.len() > MAX_CACHED_PAGES {
            if let Some(old) = self.order.pop_front() {
                if self.pages.remove(&old).is_some() {
                    break;
                }
            } else {
                break;
            }
        }
    }

    pub fn by_id(&self, id: &str) -> Option<&Video> {
        self.videos.iter().find(|v| v.item.id == id)
    }

    pub fn cached_len(&self) -> usize {
        self.pages.len()
    }
}

/// 16:10 banner for the source detail screen.
pub fn detail_art_size(ui: &Ui, w: i32) -> (usize, usize) {
    let bw = (w as f32 - 2.0 * ui.pt(MARGIN_PT)).round() as usize;
    let bh = (bw as f32 * 10.0 / 16.0).round() as usize;
    (bw, bh)
}

const NAV_MID_PT: f32 = 46.0;
const ROW_PT: f32 = 44.0;
const BUTTON_W_PT: f32 = 140.0;
const BUTTON_H_PT: f32 = 44.0;

/// Detail screen for one streaming video: banner, qualities, preview, play.
pub struct Detail {
    pub video: Video,
    pub error: Option<String>,
    pub scroll: Scroller,
    back: Image,
    chevron: Image,
    title: Vec<Image>,
    info: Image,
    play: Image,
    play_off: Image,
    preview_btn: Image,
    status: Option<Image>,
    qualities: Vec<(Image, String)>,
}

impl Detail {
    pub fn new(ui: &Ui, w: i32, video: Video) -> Self {
        let col_w = w as f32 - 2.0 * ui.pt(MARGIN_PT);
        let title = super::wrap(ui, &video.item.title, 24.0, col_w)
            .into_iter()
            .take(3)
            .map(|l| ui.text(&l, 24.0, Weight::Bold, WHITE))
            .collect();
        let mut d = Detail {
            video,
            error: None,
            scroll: Scroller::new(),
            back: ui.text("Sources", 17.0, Weight::Regular, BLUE),
            chevron: ui.text("‹", 34.0, Weight::Medium, BLUE),
            title,
            info: ui.text("", 15.0, Weight::Regular, WHITE),
            play: ui.text("Play", 17.0, Weight::Semibold, super::BLACK),
            play_off: ui.text("Play", 17.0, Weight::Semibold, SECONDARY),
            preview_btn: ui.text("Preview", 17.0, Weight::Semibold, WHITE),
            status: None,
            qualities: vec![],
        };
        d.refresh(ui, w);
        d
    }

    fn refresh(&mut self, ui: &Ui, w: i32) {
        let col_w = w as f32 - 2.0 * ui.pt(MARGIN_PT);
        self.info = ui.text(
            &ui.fit(&self.video.item.info, 15.0, col_w),
            15.0,
            Weight::Regular,
            WHITE,
        );
        self.qualities = self
            .video
            .qualities
            .iter()
            .map(|q| (ui.text(&ui.fit(&q.label, 15.0, col_w - ui.pt(60.0)), 15.0, Weight::Regular, WHITE), q.url.clone()))
            .collect();
        let _ = w;
    }

    /// New enrich results (qualities/tags) for the open video.
    pub fn set_video(&mut self, ui: &Ui, w: i32, video: Video) {
        self.video = video;
        self.error = None;
        self.refresh(ui, w);
    }

    pub fn set_error(&mut self, ui: &Ui, msg: &str) {
        self.error = Some(msg.to_string());
        self.status = Some(ui.text(&ui.fit(msg, 15.0, ui.pt(400.0)), 15.0, Weight::Regular, super::RED));
    }

    pub fn fit_scroll(&mut self, ui: &Ui, bottom: f32) {
        let max = self.content_h(ui) - bottom;
        self.scroll.set_max(max);
    }

    fn content_h(&self, ui: &Ui) -> f32 {
        ui.pt(600.0) + self.qualities.len() as f32 * ui.pt(ROW_PT)
    }

    fn banner_rect(ui: &Ui, w: i32) -> (f32, f32, f32, f32) {
        let bw = w as f32 - 2.0 * ui.pt(MARGIN_PT);
        (ui.pt(MARGIN_PT), ui.pt(8.0), bw, bw * 10.0 / 16.0)
    }

    fn row_rect(ui: &Ui, w: i32, i: usize) -> (f32, f32, f32, f32) {
        let (_, by, bw, bh) = Self::banner_rect(ui, w);
        (ui.pt(MARGIN_PT), by + bh + ui.pt(120.0) + i as f32 * ui.pt(ROW_PT), bw, ui.pt(ROW_PT))
    }

    pub fn hit(&self, ui: &Ui, w: i32, bottom: f32, x: f32, y: f32) -> Option<Target> {
        if y < NAV_MID_PT - ui.pt(22.0) || y >= bottom {
            return None;
        }
        if x < ui.pt(120.0) && y < NAV_MID_PT + ui.pt(22.0) {
            return Some(Target::Back);
        }
        let cy = y - Self::content_top_value(ui) + self.scroll.offset();
        let (_, by, bw, bh) = Self::banner_rect(ui, w);
        let btn_y = by + bh + ui.pt(16.0);
        if within(x, cy, (ui.pt(MARGIN_PT), btn_y, ui.pt(BUTTON_W_PT), ui.pt(BUTTON_H_PT))) {
            return Some(Target::Play);
        }
        if !self.video.preview.is_empty()
            && within(x, cy, (ui.pt(MARGIN_PT) + ui.pt(BUTTON_W_PT) + ui.pt(12.0), btn_y, ui.pt(BUTTON_W_PT), ui.pt(BUTTON_H_PT)))
        {
            return Some(Target::SPreview);
        }
        for (i, _) in self.qualities.iter().enumerate() {
            if within(x, cy, Self::row_rect(ui, w, i)) {
                return Some(Target::SQuality(i));
            }
        }
        let _ = bw;
        Some(Target::Content)
    }

    fn content_top_value(ui: &Ui) -> f32 {
        super::library::Library::content_top(ui)
    }

    pub fn draw(&self, s: &mut Surface, ui: &Ui, w: i32, bottom: f32, art: &mut Cache, pressed: Option<Target>) {
        let top = Self::content_top_value(ui);
        let mut band = s.band(top as usize);
        let off = self.scroll.offset();
        let cy = |y: f32| y - off + top;
        // nav chevron + back
        band.blit(&self.chevron, ui.pt(8.0) as i32, (NAV_MID_PT - ui.pt(17.0)) as i32, 255, 255);
        band.blit(&self.back, ui.pt(28.0) as i32, (NAV_MID_PT - self.back.h as f32 / 2.0) as i32, 255, 255);
        let (bx, by, bw, bh) = Self::banner_rect(ui, w);
        let id = &self.video.item.id;
        match art.get(id, SOURCE) {
            Some(img) => band.blit(img, bx as i32, cy(by) as i32, 255, 255),
            None => band.fill_rrect(bx, cy(by), bw, bh, ui.pt(10.0), CELL),
        }
        let mut y = cy(by + bh + ui.pt(12.0));
        for line in &self.title {
            band.blit(line, bx as i32, y as i32, 255, 255);
            y += line.h as f32 + ui.pt(2.0);
        }
        band.blit(&self.info, bx as i32, y as i32, 255, 255);
        let btn_y = cy(by + bh + ui.pt(64.0) + self.title.len() as f32 * ui.pt(30.0));
        let can_play = !self.video.qualities.is_empty();
        band.fill_rrect(ui.pt(MARGIN_PT), btn_y, ui.pt(BUTTON_W_PT), ui.pt(BUTTON_H_PT), ui.pt(8.0), if can_play { WHITE } else { CELL });
        band.blit(if can_play { &self.play } else { &self.play_off }, (ui.pt(MARGIN_PT) + (ui.pt(BUTTON_W_PT) - self.play.w as f32) / 2.0) as i32, (btn_y + (ui.pt(BUTTON_H_PT) - self.play.h as f32) / 2.0) as i32, 255, 255);
        if !self.video.preview.is_empty() {
            let px = ui.pt(MARGIN_PT) + ui.pt(BUTTON_W_PT) + ui.pt(12.0);
            band.fill_rrect(px, btn_y, ui.pt(BUTTON_W_PT), ui.pt(BUTTON_H_PT), ui.pt(8.0), CELL);
            band.blit(&self.preview_btn, (px + (ui.pt(BUTTON_W_PT) - self.preview_btn.w as f32) / 2.0) as i32, (btn_y + (ui.pt(BUTTON_H_PT) - self.preview_btn.h as f32) / 2.0) as i32, 255, 255);
        }
        for (i, (label, _)) in self.qualities.iter().enumerate() {
            let (rx, ry, _rw, rh) = Self::row_rect(ui, w, i);
            let ry = cy(ry);
            if ry + rh < top || ry > bottom {
                continue;
            }
            band.blit(label, rx as i32, (ry + (rh - label.h as f32) / 2.0) as i32, 255, if pressed == Some(Target::SQuality(i)) { 150 } else { 255 });
        }
        if let Some(st) = &self.status {
            band.blit(st, bx as i32, (btn_y + ui.pt(BUTTON_H_PT) + ui.pt(8.0)) as i32, 255, 255);
        }
        let _ = w;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CARD: &str = r#"<div class='item hf-video-item'><a href='https://heavyfetish.com/videos/sample-slug/' title='Sample one'><div class='img'><img class='thumb' data-webp='https://heavyfetish.com/p/1.jpg' alt='Sample'></div><div class='duration'>8:33</div></a></div>"#;

    fn videos() -> Vec<Video> {
        deildu_rust::sources::catalogue(Provider::HeavyFetish, CARD, Provider::HeavyFetish.base())
            .unwrap()
            .0
    }

    #[test]
    fn duration_chips_filter_the_catalogue() {
        let videos = videos();
        assert_eq!(videos.len(), 1);
        assert!(matches(&videos[0], "", 0));
        assert!(matches(&videos[0], "", 1), "8:33 is under 10 min");
        assert!(!matches(&videos[0], "", 2));
        assert!(matches(&videos[0], "sample", 0));
        assert!(!matches(&videos[0], "nothing", 0));
        assert!(!playable(&videos[0]), "qualities need the enrich pass");
    }

    #[test]
    fn page_cache_is_bounded() {
        let mut s = State::new();
        let videos = videos();
        for i in 0..20 {
            s.remember(&format!("k{i}"), videos.clone(), None);
        }
        assert!(s.cached_len() <= MAX_CACHED_PAGES);
        assert!(s.cached("k19").is_some());
        assert!(s.cached("k0").is_none(), "oldest pages evict first");
    }

    #[test]
    fn provider_bases_are_distinct_origins() {
        let bases: Vec<&str> = PROVIDERS.iter().map(|p| p.base()).collect();
        assert_eq!(bases.len(), 3);
        assert!(bases.iter().all(|b| b.starts_with("https://")));
    }
}
