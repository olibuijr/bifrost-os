//! Settings › Wallpaper: a grid of the bifrost 15 stills (packed by
//! tools/pack.py) and photos; tapping one opens a preview with Set Lock
//! Screen / Set Home Screen / Set Both and the "Dark Appearance Dims
//! Wallpaper" switch. The navigation bar above is drawn by Settings.

use super::list::{nav_bottom, GROUP_GAP_PT, MARGIN_PT};
use super::controls::{draw_switch, SWITCH_H_PT, SWITCH_W_PT};
use super::ui::{centred, home_zone, Ui, Weight, CARD_RADIUS_PT, CARD_BG, CELL_RADIUS_PT, FIELD_BG, LABEL, ROW_PRESSED, SECONDARY};
use super::wifi_page::BLUE;
use crate::gfx::{rrect_mask, Image, Surface, View};

/// One packed wallpaper: `id` names `wallpapers/<id>.raw`.
#[derive(Clone, Debug, PartialEq)]
pub struct Wallpaper {
    pub id: String,
    pub name: String,
    pub thumb: Image,
}

/// The bundled stills photographed in Iceland (CREDITS.md); a fresh install starts on one of them.
pub const ICELAND: [&str; 6] = ["waterfall_idyllic", "svartifoss_basalt", "kirkjufell_dusk", "gullfoss_canyon", "arnarstapi_coast", "iceland_falls"];

/// The wallpaper a fresh install starts on: one of the `ICELAND` stills packed in `dir/wallpapers`,
/// `seed` picking which. None when none is packed (the shell then keeps `wallpaper.raw`).
pub fn first_wallpaper(dir: &str, seed: u64) -> Option<&'static str> {
    let packed: Vec<&'static str> = ICELAND.into_iter().filter(|id| std::path::Path::new(&format!("{dir}/wallpapers/{id}.raw")).exists()).collect();
    (!packed.is_empty()).then(|| packed[(seed % packed.len() as u64) as usize])
}

/// The shell's lock screen wallpaper and the dimmed copies (Dark Appearance
/// Dims Wallpaper) of the home and lock stills.
#[derive(Default)]
pub struct Extra {
    pub dim: bool,
    pub lock: Option<View<'static>>,
    pub home_dim: Option<Image>,
    pub lock_dim: Option<Image>,
}

/// `v` at 70 % brightness (opaque pixels).
pub fn dimmed(v: View) -> Image {
    let mut img = Image::new(v.w, v.h);
    for (o, &p) in img.px.iter_mut().zip(v.px) {
        *o = (((p & 0x00ff_00ff) * 179 >> 8) & 0x00ff_00ff) | (((p & 0x0000_ff00) * 179 >> 8) & 0x0000_ff00) | (p & 0xff00_0000);
    }
    img
}

pub const COLS: usize = 6;
/// Corner radius in px of the preview buttons (rule: at most 6, no pills).
const BUTTON_RADIUS: f32 = 6.0;
const GAP_PT: f32 = 12.0;
const LABEL_PT: f32 = 11.0;
/// Space under a thumbnail for its label.
const LABEL_ROW_PT: f32 = 22.0;
const THUMB_RADIUS_PT: f32 = 6.0;
const RING_PT: f32 = 3.0;
/// The stored thumbnails: 16:9 centre crops of the stills (`pack.py` THUMB). The grid and the preview take the
/// screen's aspect (`aspect` = screen height / width) and cover-fit these, so a landscape desktop shows landscape art.
pub const THUMB: (usize, usize) = (320, 180);

/// `img` centre-cropped to `w` x `h` (cover, as the desktop backdrop), then resized.
pub fn cover_fit(img: &Image, w: usize, h: usize) -> Image {
    if img.w == 0 || img.h == 0 || w == 0 || h == 0 {
        return Image::new(w, h);
    }
    let scale = (w as f32 / img.w as f32).max(h as f32 / img.h as f32);
    let (cw, ch) = (((w as f32 / scale).round() as usize).clamp(1, img.w), ((h as f32 / scale).round() as usize).clamp(1, img.h));
    img.crop((img.w - cw) / 2, (img.h - ch) / 2, cw, ch).resized(w, h)
}

/// Thumbnail cell size in px: 6 columns between the page margins, `aspect` = screen height / width.
pub fn cell_size(ui: &Ui, w: i32, aspect: f32) -> (f32, f32) {
    let cw = ((w as f32 - 2.0 * ui.pt(MARGIN_PT) - (COLS - 1) as f32 * ui.pt(GAP_PT)) / COLS as f32).floor();
    (cw, (cw * aspect).floor())
}

/// Top of the Dynamic Wallpaper switch row, under the nav bar.
fn dynamic_row_top(ui: &Ui) -> f32 {
    nav_bottom(ui) + ui.pt(GROUP_GAP_PT)
}

/// The grid starts under the Dynamic Wallpaper switch row.
pub fn grid_top(ui: &Ui) -> f32 {
    dynamic_row_top(ui) + ui.pt(ROW_PT) + ui.pt(GROUP_GAP_PT)
}

/// The Dynamic Wallpaper switch row (x, y, w, h): a day of stills follows the time of day.
pub fn dynamic_row(ui: &Ui, w: i32) -> (f32, f32, f32, f32) {
    (ui.pt(MARGIN_PT), dynamic_row_top(ui), w as f32 - 2.0 * ui.pt(MARGIN_PT), ui.pt(ROW_PT))
}

/// Thumbnail rect (x, y, w, h) in px of item `i`.
pub fn cell(ui: &Ui, w: i32, aspect: f32, i: usize) -> (f32, f32, f32, f32) {
    let (cw, ch) = cell_size(ui, w, aspect);
    let (c, r) = ((i % COLS) as f32, (i / COLS) as f32);
    let x = ui.pt(MARGIN_PT) + c * (cw + ui.pt(GAP_PT));
    let y = grid_top(ui) + r * (ch + ui.pt(LABEL_ROW_PT) + ui.pt(GAP_PT));
    (x, y, cw, ch)
}

/// Where a previewed wallpaper goes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Target {
    Lock,
    Home,
    Both,
}

/// What a tap on the page meant.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Tap {
    /// a grid thumbnail
    Thumb(usize),
    /// a Set button in the preview
    Set(usize, Target),
    /// the Cancel button
    Cancel,
    /// the dim switch
    Dim,
    /// the Dynamic Wallpaper switch above the grid
    Dynamic,
}

const ROW_PT: f32 = 44.0;
const BUTTON_PT: f32 = 50.0;
const BUTTON_GAP_PT: f32 = 12.0;
const BUTTONS: [(&str, Option<Target>); 4] = [("Set Lock Screen", Some(Target::Lock)), ("Set Home Screen", Some(Target::Home)), ("Set Both", Some(Target::Both)), ("Cancel", None)];

/// The open preview: the picked item's enlarged thumbnail and button labels.
struct Preview {
    item: usize,
    img: Image,
    dim_label: Image,
    labels: Vec<Image>,
}

pub struct WallpaperPage {
    items: Vec<(String, Image, Image)>,
    current: Option<usize>,
    empty: Image,
    dynamic_label: Image,
    preview: Option<Preview>,
    dim: bool,
    /// Dynamic Wallpaper: the shell changes the still with the time of day
    dynamic: bool,
    /// screen height / width: the cells and the preview are this shape
    aspect: f32,
    /// unmasked thumbnails, enlarged for the preview
    sources: Vec<Image>,
}

impl WallpaperPage {
    pub fn new(ui: &Ui, w: i32, aspect: f32, wallpapers: &[Wallpaper], current: &str, dim: bool) -> Self {
        let (cw, ch) = cell_size(ui, w, aspect);
        let mask = rrect_mask(cw as usize, ch as usize, ui.pt(THUMB_RADIUS_PT));
        let items = wallpapers
            .iter()
            .map(|wp| {
                let mut thumb = cover_fit(&wp.thumb, cw as usize, ch as usize);
                thumb.apply_mask(&mask);
                let label = ui.text(&ui.fit(&wp.name, LABEL_PT, cw), LABEL_PT, Weight::Regular, LABEL.now());
                (wp.id.clone(), thumb, label)
            })
            .collect();
        WallpaperPage { current: wallpapers.iter().position(|wp| wp.id == current), items, empty: ui.text("No wallpapers packed", 17.0, Weight::Regular, SECONDARY), dynamic_label: ui.text("Dynamic Wallpaper", 17.0, Weight::Regular, LABEL.now()), preview: None, dim, dynamic: false, aspect, sources: wallpapers.iter().map(|wp| wp.thumb.clone()).collect() }
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// The id of the highlighted wallpaper.
    pub fn current(&self) -> Option<&str> {
        self.current.map(|i| self.items[i].0.as_str())
    }

    /// Highlights item `i` and returns its id.
    pub fn select(&mut self, i: usize) -> Option<String> {
        let id = self.items.get(i)?.0.clone();
        self.current = Some(i);
        Some(id)
    }

    /// The id of item `i`.
    pub fn id_of(&self, i: usize) -> Option<&str> {
        self.items.get(i).map(|it| it.0.as_str())
    }

    pub fn dim(&self) -> bool {
        self.dim
    }

    pub fn set_dim(&mut self, on: bool) {
        self.dim = on;
    }

    pub fn dynamic(&self) -> bool {
        self.dynamic
    }

    pub fn set_dynamic(&mut self, on: bool) {
        self.dynamic = on;
    }

    pub fn previewing(&self) -> bool {
        self.preview.is_some()
    }

    pub fn close_preview(&mut self) {
        self.preview = None;
    }

    /// Opens the preview of item `i`.
    pub fn open_preview(&mut self, ui: &Ui, w: i32, h: i32, i: usize) {
        let Some(src) = self.sources.get(i) else { return };
        let (pw, ph, ..) = self.preview_rect(ui, w, h);
        let (pw, ph) = (pw as usize, ph as usize);
        let mut img = cover_fit(src, pw, ph);
        img.apply_mask(&rrect_mask(pw, ph, ui.pt(CARD_RADIUS_PT)));
        let labels = BUTTONS.iter().map(|(t, target)| ui.text(t, 17.0, if target.is_some() { Weight::Semibold } else { Weight::Regular }, BLUE)).collect();
        self.preview = Some(Preview { item: i, img, dim_label: ui.text("Dark Appearance Dims Wallpaper", 17.0, Weight::Regular, LABEL.now()), labels });
    }

    /// (w, h, x, y) of the preview image: half the free height, in the screen's aspect.
    fn preview_rect(&self, ui: &Ui, w: i32, h: i32) -> (f32, f32, f32, f32) {
        let free = h as f32 - home_zone(ui) - grid_top(ui);
        let ph = (free * 0.45).floor();
        let pw = (ph / self.aspect).floor();
        (pw, ph, ((w as f32 - pw) / 2.0).floor(), grid_top(ui))
    }

    /// The dim switch row (x, y, w, h).
    pub fn dim_row(&self, ui: &Ui, w: i32, h: i32) -> (f32, f32, f32, f32) {
        let (_, ph, _, py) = self.preview_rect(ui, w, h);
        (ui.pt(MARGIN_PT), py + ph + ui.pt(GROUP_GAP_PT), w as f32 - 2.0 * ui.pt(MARGIN_PT), ui.pt(ROW_PT))
    }

    /// Button `i` (x, y, w, h).
    pub fn button(&self, ui: &Ui, w: i32, h: i32, i: usize) -> (f32, f32, f32, f32) {
        let (rx, ry, rw, rh) = self.dim_row(ui, w, h);
        (rx, ry + rh + ui.pt(GROUP_GAP_PT) + i as f32 * (ui.pt(BUTTON_PT) + ui.pt(BUTTON_GAP_PT)), rw, ui.pt(BUTTON_PT))
    }

    /// What a touch at (x, y) taps: grid thumbnails, or the preview's controls.
    pub fn tap(&self, ui: &Ui, w: i32, h: i32, x: f32, y: f32) -> Option<Tap> {
        let inside = |(rx, ry, rw, rh): (f32, f32, f32, f32)| x >= rx && x < rx + rw && y >= ry && y < ry + rh;
        let Some(p) = &self.preview else {
            if inside(dynamic_row(ui, w)) {
                return Some(Tap::Dynamic);
            }
            return self.hit(ui, w, h, x, y).map(Tap::Thumb);
        };
        if inside(self.dim_row(ui, w, h)) {
            return Some(Tap::Dim);
        }
        (0..BUTTONS.len()).find(|&i| inside(self.button(ui, w, h, i))).map(|i| match BUTTONS[i].1 {
            Some(t) => Tap::Set(p.item, t),
            None => Tap::Cancel,
        })
    }

    /// How many items fit above the home indicator.
    fn visible(&self, ui: &Ui, w: i32, h: i32) -> usize {
        let limit = h as f32 - home_zone(ui);
        (0..self.items.len()).take_while(|&i| cell(ui, w, self.aspect, i).1 + cell_size(ui, w, self.aspect).1 + ui.pt(LABEL_ROW_PT) <= limit).count()
    }

    /// The thumbnail under (x, y).
    pub fn hit(&self, ui: &Ui, w: i32, h: i32, x: f32, y: f32) -> Option<usize> {
        (0..self.visible(ui, w, h)).find(|&i| {
            let (cx, cy, cw, ch) = cell(ui, w, self.aspect, i);
            x >= cx && x < cx + cw && y >= cy && y < cy + ch
        })
    }

    pub fn draw(&self, s: &mut Surface, ui: &Ui, w: i32, h: i32, pressed: Option<usize>) {
        let m = ui.pt(MARGIN_PT);
        if let Some(p) = &self.preview {
            let (_, _, px, py) = self.preview_rect(ui, w, h);
            s.blit(&p.img, px as i32, py as i32, 255, 255);
            let (x, y, rw, rh) = self.dim_row(ui, w, h);
            s.fill_rrect(x, y, rw, rh, ui.pt(CELL_RADIUS_PT), CARD_BG.now());
            s.blit(&p.dim_label, (x + ui.pt(16.0)) as i32, (y + (rh - p.dim_label.h as f32) / 2.0) as i32, 255, 255);
            draw_switch(s, ui, x + rw - ui.pt(16.0) - ui.pt(SWITCH_W_PT), y + (rh - ui.pt(SWITCH_H_PT)) / 2.0, if self.dim { 1.0 } else { 0.0 });
            for (i, label) in p.labels.iter().enumerate() {
                let (bx, by, bw, bh) = self.button(ui, w, h, i);
                s.fill_rrect(bx, by, bw, bh, BUTTON_RADIUS.min(bh / 2.0), if pressed == Some(i) { ROW_PRESSED.now() } else { FIELD_BG.now() });
                centred(s, label, (bx + bw / 2.0) as i32, (by + (bh - label.h as f32) / 2.0) as i32);
            }
            return;
        }
        let (x, y, rw, rh) = dynamic_row(ui, w);
        s.fill_rrect(x, y, rw, rh, ui.pt(CELL_RADIUS_PT), CARD_BG.now());
        s.blit(&self.dynamic_label, (x + ui.pt(16.0)) as i32, (y + (rh - self.dynamic_label.h as f32) / 2.0) as i32, 255, 255);
        draw_switch(s, ui, x + rw - ui.pt(16.0) - ui.pt(SWITCH_W_PT), y + (rh - ui.pt(SWITCH_H_PT)) / 2.0, if self.dynamic { 1.0 } else { 0.0 });
        if self.items.is_empty() {
            s.blit(&self.empty, (m + ui.pt(16.0)) as i32, grid_top(ui) as i32, 255, 255);
            return;
        }
        for (i, (_, thumb, label)) in self.items.iter().take(self.visible(ui, w, h)).enumerate() {
            let (x, y, cw, ch) = cell(ui, w, self.aspect, i);
            let tint = if pressed == Some(i) { 150 } else { 255 };
            s.blit(thumb, x as i32, y as i32, tint, 255);
            if self.current == Some(i) {
                let r = ui.pt(RING_PT);
                s.stroke_rrect(x - r, y - r, cw + 2.0 * r, ch + 2.0 * r, ui.pt(THUMB_RADIUS_PT) + r, r, BLUE);
            }
            centred(s, label, (x + cw / 2.0) as i32, (y + ch + (ui.pt(LABEL_ROW_PT) - label.h as f32) / 2.0) as i32);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gfx::pack;

    fn ui() -> Ui {
        super::super::tests::ui()
    }

    #[test]
    fn a_fresh_install_starts_on_a_packed_icelandic_still() {
        let dir = std::env::temp_dir().join(format!("wp-first-{}", std::process::id()));
        let d = dir.to_str().unwrap();
        assert_eq!(first_wallpaper(d, 0), None, "nothing packed: keep wallpaper.raw");
        std::fs::create_dir_all(dir.join("wallpapers")).unwrap();
        for id in ["kirkjufell_dusk", "gullfoss_canyon", "desert_day"] {
            std::fs::write(dir.join(format!("wallpapers/{id}.raw")), b"").unwrap();
        }
        let picks: std::collections::BTreeSet<_> = (0..8).filter_map(|seed| first_wallpaper(d, seed)).collect();
        assert_eq!(picks.into_iter().collect::<Vec<_>>(), ["gullfoss_canyon", "kirkjufell_dusk"], "only Icelandic stills, and the seed varies the pick");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    fn wallpapers(n: usize) -> Vec<Wallpaper> {
        (0..n)
            .map(|i| Wallpaper { id: format!("wp{i}"), name: format!("Wallpaper {i}"), thumb: Image { w: 320, h: 180, px: vec![pack(10 * i as u8, 0, 0, 255); 320 * 180] } })
            .collect()
    }

    /// Developer preview: `PREVIEW_DIR=/tmp cargo test preview_wallpaper_grid -- --ignored` writes wallpaper-grid.png,
    /// the packed 16:9 thumbnails on a 16:9 screen (1280 x 720 px).
    #[test]
    #[ignore]
    fn preview_wallpaper_grid() {
        let Some(dir) = std::env::var_os("PREVIEW_DIR") else { return };
        let assets = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/wallpapers");
        let index = crate::shell::prefs::Prefs::load(format!("{assets}/index.ini"));
        let wps: Vec<Wallpaper> = index
            .entries()
            .filter_map(|(id, name)| {
                let thumb = Image::from_raw(&std::fs::read(format!("{assets}/{id}.thumb.raw")).ok()?)?;
                Some(Wallpaper { id: id.to_string(), name: name.to_string(), thumb })
            })
            .collect();
        let ui = ui();
        let (w, h) = (1280i32, 720i32);
        let page = WallpaperPage::new(&ui, w, h as f32 / w as f32, &wps, "svartifoss_basalt", true);
        let mut img = Image::new(w as usize, h as usize);
        img.px.fill(u32::MAX);
        page.draw(&mut Surface { w: w as usize, h: h as usize, stride: w as usize, px: &mut img.px, y0: 0 }, &ui, w, h, None);
        std::fs::write(std::path::Path::new(&dir).join("wallpaper-grid.png"), crate::shell::screenshot::encode(&img)).unwrap();
    }

    #[test]
    fn cells_tile_six_columns_inside_the_margins() {
        let ui = ui();
        let (cw, ch) = cell_size(&ui, 1200, 9.0 / 16.0);
        assert!((ch / cw - 9.0 / 16.0).abs() < 0.02, "the grid follows the screen's 16:9 aspect");
        let (x0, y0, ..) = cell(&ui, 1200, 9.0 / 16.0, 0);
        let (x5, y5, ..) = cell(&ui, 1200, 9.0 / 16.0, 5);
        let (x6, y6, ..) = cell(&ui, 1200, 9.0 / 16.0, 6);
        assert_eq!(x0, ui.pt(MARGIN_PT));
        assert!(x5 + cw <= 1200.0 - ui.pt(MARGIN_PT) + 1.0);
        assert_eq!((x6, y0), (x0, y5));
        assert!(y6 > y0 + ch);
        assert_eq!(y0, grid_top(&ui));
        assert!(y0 > dynamic_row(&ui, 1200).1 + dynamic_row(&ui, 1200).3, "the grid sits under the Dynamic Wallpaper row");
    }

    #[test]
    fn the_dynamic_switch_row_taps_toggle_only_outside_the_preview() {
        let ui = ui();
        let mut page = WallpaperPage::new(&ui, 1200, 9.0 / 16.0, &wallpapers(3), "wp0", true);
        assert!(!page.dynamic());
        let (x, y, w, h) = dynamic_row(&ui, 1200);
        assert_eq!(page.tap(&ui, 1200, 1920, x + w / 2.0, y + h / 2.0), Some(Tap::Dynamic));
        page.set_dynamic(true);
        assert!(page.dynamic());
        let (cx, cy, cw, ch) = cell(&ui, 1200, 9.0 / 16.0, 1);
        assert_eq!(page.tap(&ui, 1200, 1920, cx + cw / 2.0, cy + ch / 2.0), Some(Tap::Thumb(1)), "the grid is below the row");
        page.open_preview(&ui, 1200, 1920, 0);
        assert_eq!(page.tap(&ui, 1200, 1920, x + w / 2.0, y + h / 2.0), None, "the preview hides the row");
    }

    #[test]
    fn taps_resolve_to_items() {
        let ui = ui();
        let mut page = WallpaperPage::new(&ui, 1200, 9.0 / 16.0, &wallpapers(25), "wp3", true);
        assert_eq!(page.current(), Some("wp3"));
        let (x, y, cw, ch) = cell(&ui, 1200, 9.0 / 16.0, 7);
        assert_eq!(page.hit(&ui, 1200, 1920, x + cw / 2.0, y + ch / 2.0), Some(7));
        assert_eq!(page.hit(&ui, 1200, 1920, x - 2.0, y + ch / 2.0), None, "between columns");
        assert_eq!(page.hit(&ui, 1200, 1920, ui.pt(60.0), ui.pt(42.0)), None, "the bar is not an item");
        assert_eq!(page.select(7).as_deref(), Some("wp7"));
        assert_eq!(page.current(), Some("wp7"));
        assert_eq!(page.select(99), None);
        assert_eq!(page.len(), 25);
        assert!(page.visible(&ui, 1200, 1920) == 25, "all 25 stills fit above the home indicator");
        assert!(page.visible(&ui, 1200, 400) < 25, "a short screen hides the lower rows");
    }

    #[test]
    fn draws_thumbnails_and_highlights_the_current_one() {
        let ui = ui();
        let page = WallpaperPage::new(&ui, 1200, 9.0 / 16.0, &wallpapers(2), "wp1", true);
        let mut buf = vec![0u32; 1200 * 1920];
        page.draw(&mut Surface { w: 1200, h: 1920, stride: 1200, px: &mut buf, y0: 0 }, &ui, 1200, 1920, None);
        let (x, y, cw, ch) = cell(&ui, 1200, 9.0 / 16.0, 1);
        let centre = buf[(y + ch / 2.0) as usize * 1200 + (x + cw / 2.0) as usize];
        assert_eq!(centre, pack(10, 0, 0, 255));
        let ring = buf[(y + ch / 2.0) as usize * 1200 + (x - ui.pt(RING_PT) / 2.0) as usize];
        assert_eq!(ring, BLUE);
        let (x0, y0, ..) = cell(&ui, 1200, 9.0 / 16.0, 0);
        assert_ne!(buf[(y0 + 3.0) as usize * 1200 + (x0 - ui.pt(RING_PT) / 2.0) as usize], BLUE, "only the current one has a ring");
        let empty = WallpaperPage::new(&ui, 1200, 9.0 / 16.0, &[], "", true);
        assert!(empty.is_empty());
        empty.draw(&mut Surface { w: 1200, h: 1920, stride: 1200, px: &mut buf, y0: 0 }, &ui, 1200, 1920, None);
    }

    #[test]
    fn preview_buttons_and_dim_switch_resolve_to_taps() {
        let ui = ui();
        let mut page = WallpaperPage::new(&ui, 1200, 9.0 / 16.0, &wallpapers(3), "wp0", true);
        page.open_preview(&ui, 1200, 1920, 2);
        assert!(page.previewing());
        let (x, y, w, h) = page.dim_row(&ui, 1200, 1920);
        assert_eq!(page.tap(&ui, 1200, 1920, x + w / 2.0, y + h / 2.0), Some(Tap::Dim));
        for (i, want) in [Tap::Set(2, Target::Lock), Tap::Set(2, Target::Home), Tap::Set(2, Target::Both), Tap::Cancel].into_iter().enumerate() {
            let (x, y, w, h) = page.button(&ui, 1200, 1920, i);
            assert_eq!(page.tap(&ui, 1200, 1920, x + w / 2.0, y + h / 2.0), Some(want));
        }
        let mut buf = vec![0u32; 1200 * 1920];
        page.draw(&mut Surface { w: 1200, h: 1920, stride: 1200, px: &mut buf, y0: 0 }, &ui, 1200, 1920, None);
        page.close_preview();
        assert!(!page.previewing());
    }
}
