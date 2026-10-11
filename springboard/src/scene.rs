//! Composes one home-screen frame: wallpaper, status bar, icon pages, page
//! dots and the floating dock.

use crate::gfx::{rgba, rrect_mask, shade, Area, Damage, Image, Surface, View};
use crate::layout::{Layout, DOCK_RADIUS_PT, PER_PAGE};
use crate::pager::Pager;
use crate::shell::applib::AppLibrary;
use crate::shell::today::Today;
use crate::status;

/// Everything a home frame's icons and dots depend on; comparing two states
/// says which tiles moved, which changed highlight and whether the page dot
/// moved, so a swipe redraws only the tiles and not the wallpaper under them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HomeState {
    /// page offset in whole pixels, as drawn
    pub offset: i32,
    pub page: usize,
    pub pressed: Option<usize>,
    pub dock: usize,
    pub grid: usize,
    pub pages: usize,
    /// the Today View's generation (0 without one)
    pub today: u32,
    /// the widgets and App Library generation (0 without them)
    pub extra: u32,
}

#[derive(Clone)]
pub struct Tile {
    pub icon: Image,
    pub label: Image,
}

#[derive(Default)]
pub struct Chrome {
    pub time: Option<Image>,
    pub date: Option<Image>,
    pub percent: Option<Image>,
    pub battery: u8,
    pub charging: bool,
    /// Wi-Fi connected
    pub online: bool,
    /// wired link with an address (LAN glyph when Wi-Fi is not connected)
    pub wired: bool,
    /// no battery in this machine: the battery and percentage are not drawn
    pub no_battery: bool,
    pub wifi_bars: u8,
    /// Settings > Battery > Battery Percentage off: the percent text is not drawn
    pub hide_percent: bool,
    pub low_power: bool,
    pub dnd: bool,
    pub rot_lock: bool,
}

/// Height of the status bar in points (bifrost mini, no notch).
pub const STATUS_PT: f32 = 20.0;
/// Where the clock starts, in points from the left edge.
pub const STATUS_LEFT_PT: f32 = 5.0;

/// Right side of the status bar: wifi (only when connected), percent,
/// battery. `mid` is the bar's vertical centre, `pt` px per point.
/// Ink spacing as measured on the bifrost: battery nub 5.5 pt from the edge,
/// 3.5 pt to the percentage, 5.5 pt to the wifi fan (the text images carry
/// a 2 px pad and the fan's box is wider than its ink, hence the offsets).
pub fn draw_status_right(s: &mut Surface, w: i32, mid: i32, pt: f32, c: &Chrome) {
    let mut right = w as f32 - 5.5 * pt;
    if !c.no_battery {
        let bat_x = w as f32 - (5.5 + status::BATTERY_NUB_PT + status::BATTERY_W_PT) * pt;
        status::draw_battery_mode(s, bat_x, mid as f32 - status::BATTERY_H_PT / 2.0 * pt, pt, c.battery, c.charging, c.low_power);
        right = bat_x - 1.5 * pt;
        if let Some(p) = c.percent.as_ref().filter(|_| !c.hide_percent) {
            right -= p.w as f32;
            s.blit(p, right as i32, mid - p.h as i32 / 2, 255, 255);
            right -= 1.0 * pt;
        }
    }
    // Wi-Fi when joined, else the LAN glyph when a cable carries an address; nothing when neither
    if c.online || c.wired {
        let r = status::WIFI_PT * pt;
        let (x, y) = (right - 2.0 * r, mid as f32 - r / 2.0);
        if c.online {
            status::draw_wifi_bars(s, x, y, r, c.wifi_bars, rgba(255, 255, 255, 255));
        } else {
            status::draw_lan(s, x, y, r, rgba(255, 255, 255, 255));
        }
        right -= 2.0 * r + 5.5 * pt;
    }
    let white = rgba(255, 255, 255, 255);
    if c.dnd {
        let m = 9.0 * pt;
        right -= m;
        status::draw_moon(s, right, mid as f32 - m / 2.0, m, white);
        right -= 5.0 * pt;
    }
    if c.rot_lock {
        let m = 10.0 * pt;
        right -= m;
        status::draw_rotation_lock(s, right, mid as f32 - m / 2.0, m, white);
    }
}

/// A home-screen widget, pre-rendered at its footprint: `page` is the
/// visible page, (x, y) page-relative.
pub struct HomeWidget {
    pub page: usize,
    pub x: i32,
    pub y: i32,
    pub img: Image,
}

/// What the home pages carry beyond app icons: placed widgets and the App
/// Library on the last page. `gen` changes whenever either looks different.
#[derive(Clone, Copy)]
pub struct Extras<'a> {
    /// the App Library folder icon at the end of the dock
    pub dock_library: Option<&'a Image>,
    pub widgets: &'a [HomeWidget],
    pub library: Option<&'a AppLibrary>,
    pub gen: u32,
}

pub struct Scene<'a> {
    pub layout: &'a Layout,
    pub wallpaper: View<'a>,
    pub dock_bg: Option<&'a Image>,
    pub dock: &'a [Tile],
    pub grid: &'a [Tile],
    pub pager: &'a Pager,
    pub chrome: &'a Chrome,
    /// the Today View on page -1, when the pager allows a leading page
    pub today: Option<&'a Today>,
    pub extras: Option<Extras<'a>>,
}

const PRESSED_TINT: u32 = 150;

/// The slot under (x, y): dock tiles first, then grid tiles.
pub fn hit(l: &Layout, offset: f32, dock: usize, grid: usize, x: f32, y: f32) -> Option<usize> {
    if let Some(i) = l.dock_icons.iter().take(dock).position(|r| r.contains(x, y)) {
        return Some(i);
    }
    let page_x = x + offset;
    let page = (page_x / l.w as f32).floor();
    if page < 0.0 {
        return None;
    }
    let local = page_x - page * l.w as f32;
    let i = l.cells.iter().position(|r| r.contains(local, y))?;
    let slot = page as usize * PER_PAGE + i;
    (slot < grid).then_some(dock + slot)
}

/// The dock's frosted backdrop: the wallpaper under it, blurred and darkened,
/// clipped to the dock's rounded rectangle.
pub fn dock_background(wallpaper: View, l: &Layout) -> Option<Image> {
    if l.dock.w == 0 {
        return None;
    }
    let pad = l.pt(40.0) as usize;
    let (x, y) = (l.dock.x as usize, l.dock.y as usize);
    let src = wallpaper.crop(x.saturating_sub(pad), y.saturating_sub(pad), l.dock.w as usize + 2 * pad, l.dock.h as usize + 2 * pad);
    let mut blurred = src;
    blurred.blur_material(l.pt(20.0) as usize);
    // measured on the bifrost: dock = 0.29 * wallpaper + 30, i.e. a heavy grey tint
    let tint = rgba(44, 44, 46, 140);
    for p in blurred.px.iter_mut() {
        *p = crate::gfx::over(shade(*p, 158), tint);
    }
    let mut dock = blurred.crop(x.min(pad), y.min(pad), l.dock.w as usize, l.dock.h as usize);
    dock.apply_mask(&rrect_mask(dock.w, dock.h, l.pt(DOCK_RADIUS_PT)));
    Some(dock)
}

impl Scene<'_> {
    pub fn draw(&self, s: &mut Surface) {
        self.draw_at(s, 0);
    }

    /// Whether `draw_clipped` reproduces `draw` exactly: the pages are plain icon grids (no Today
    /// frost, widgets or App Library over them).
    pub fn can_clip(&self) -> bool {
        let off = self.pager.offset().round() as i32;
        off >= 0 && self.extras.is_none_or(|e| e.widgets.is_empty()) && !self.library_visible(off)
    }

    /// Draws just `areas` (frame coordinates) of the frame, each through a window that cuts
    /// everything at the rectangle, so a swipe repaints the moving columns and not whole rows of
    /// wallpaper. Only for `can_clip` scenes. `s` is a band of rows (see `Surface::draw_rows`).
    pub fn draw_clipped(&self, s: &mut Surface, areas: &[Area]) {
        for a in areas {
            let (top, bottom) = (a.y0.max(s.y0), a.y1.min(s.h));
            if top >= bottom || a.x0 >= a.x1 || a.x1 > s.w {
                continue;
            }
            let from = (top - s.y0) * s.stride + a.x0;
            let mut win = Surface { w: a.x1 - a.x0, h: bottom, stride: s.stride, px: &mut s.px[from..], y0: top };
            self.draw_at(&mut win, a.x0 as i32);
        }
    }

    /// `draw` into a surface whose left edge is frame column `ox`.
    fn draw_at(&self, s: &mut Surface, ox: i32) {
        let l = self.layout;
        s.copy(self.wallpaper, (l.w - self.wallpaper.w as i32) / 2 - ox, (l.h - self.wallpaper.h as i32) / 2);
        if let (Some(t), 0) = (self.today, ox) {
            t.draw(s, l, self.pager.offset());
        }
        self.draw_status(s, ox);
        self.draw_pages(s, ox);
        self.draw_extras(s, ox);
        self.draw_dots(s, ox);
        self.draw_dock(s, ox);
    }

    pub fn state(&self) -> HomeState {
        HomeState {
            offset: self.pager.offset().round() as i32,
            page: self.pager.page(),
            pressed: self.pager.pressed(),
            dock: self.dock.len(),
            grid: self.grid.len(),
            pages: self.pager.pages,
            today: self.today.map_or(0, Today::gen),
            extra: self.extras.map_or(0, |e| e.gen),
        }
    }

    /// True while the App Library page (the last) is at least partly on screen at `offset`.
    fn library_visible(&self, offset: i32) -> bool {
        self.extras.is_some_and(|e| e.library.is_some()) && offset as f32 > (self.pager.pages as f32 - 2.0) * self.layout.w as f32
    }

    /// The widgets on the pages in view, then the library page.
    fn draw_extras(&self, s: &mut Surface, ox: i32) {
        let Some(e) = self.extras else { return };
        let l = self.layout;
        let off = self.pager.offset().round() as i32;
        for w in e.widgets {
            let dx = w.page as i32 * l.w - off;
            if dx > -l.w && dx < l.w {
                s.blit(&w.img, w.x + dx - ox, w.y, 255, 255);
            }
        }
        if let Some(lib) = e.library {
            let x0 = (self.pager.pages as i32 - 1) * l.w - off;
            if x0 > -l.w && x0 < l.w {
                lib.draw(s, l, x0 - ox);
            }
        }
    }

    /// Adds to `out` what redrawing from `prev` to the current state touches:
    /// every tile that moved or changed highlight and the dots when the page
    /// changed, or the whole frame when the icon set or page count changed.
    /// The Today page frosts the whole wallpaper, so any change while it is
    /// in view repaints the frame.
    pub fn damage(&self, prev: Option<HomeState>, out: &mut Damage) {
        let now = self.state();
        let Some(prev) = prev.filter(|p| (p.dock, p.grid, p.pages) == (now.dock, now.grid, now.pages)) else {
            return out.all();
        };
        if (prev.offset < 0 || now.offset < 0) && (prev.offset != now.offset || prev.today != now.today) {
            return out.all();
        }
        let moving_extras = self.extras.is_some_and(|e| !e.widgets.is_empty()) || self.library_visible(prev.offset) || self.library_visible(now.offset);
        if prev.extra != now.extra || (prev.offset != now.offset && moving_extras) {
            return out.all();
        }
        if prev.offset != now.offset {
            for i in 0..self.grid.len() {
                for off in [prev.offset, now.offset] {
                    if let Some((x, y, w, h)) = self.grid_tile(i, off) {
                        out.add(x, y, w, h);
                    }
                }
            }
        }
        if prev.page != now.page {
            if let Some((x, y, w, h)) = self.dots_rect() {
                out.add(x, y, w, h);
            }
        }
        if prev.pressed != now.pressed {
            for slot in [prev.pressed, now.pressed].into_iter().flatten() {
                if slot < self.dock.len() {
                    let r = self.layout.dock_icons[slot];
                    out.add(r.x, r.y, r.w, r.h);
                } else {
                    for off in [prev.offset, now.offset] {
                        if let Some((x, y, w, h)) = self.grid_tile(slot - self.dock.len(), off) {
                            out.add(x, y, w, h);
                        }
                    }
                }
            }
        }
    }

    /// Where grid tile `i` (icon and label) lands at page offset `offset`, if on screen.
    fn grid_tile(&self, i: usize, offset: i32) -> Option<(i32, i32, i32, i32)> {
        let l = self.layout;
        let dx = (i / PER_PAGE) as i32 * l.w - offset;
        if dx <= -l.w || dx >= l.w {
            return None;
        }
        let r = l.cells[i % PER_PAGE];
        let label = &self.grid[i].label;
        let lx = r.x + l.icon / 2 - label.w as i32 / 2;
        let x0 = r.x.min(lx);
        let x1 = (r.x + l.icon).max(lx + label.w as i32);
        Some((x0 + dx, r.y, x1 - x0, l.icon + l.label_gap + label.h as i32))
    }

    /// The dots: one per page plus one for the Today page when there is one;
    /// returns the current dot's index and the count.
    fn dots(&self) -> (usize, usize) {
        let lead = self.pager.has_leading() as usize;
        let current = if self.pager.leading() { 0 } else { self.pager.page() + lead };
        (current, self.pager.pages + lead)
    }

    /// The strip the page dots occupy, if there is more than one page.
    fn dots_rect(&self) -> Option<(i32, i32, i32, i32)> {
        let l = self.layout;
        let (_, pages) = self.dots();
        if pages < 2 {
            return None;
        }
        let total = (pages as i32 - 1) * l.dot_pitch + l.dot;
        Some(((l.w - total) / 2, l.dots_y - (l.dot + 1) / 2, total, l.dot + 1))
    }

    fn draw_status(&self, s: &mut Surface, ox: i32) {
        let l = self.layout;
        let c = self.chrome;
        let mid = l.status_h / 2;
        let mut x = l.pt(STATUS_LEFT_PT) as i32 - ox;
        for img in [&c.time, &c.date].into_iter().flatten() {
            s.blit(img, x, mid - img.h as i32 / 2, 255, 255);
            x += img.w as i32 + l.pt(7.5) as i32;
        }
        draw_status_right(s, l.w - ox, mid, l.s, c);
    }

    fn draw_tile(&self, s: &mut Surface, t: &Tile, x: i32, y: i32, pressed: bool, label: bool) {
        let tint = if pressed { PRESSED_TINT } else { 255 };
        s.blit(&t.icon, x, y, tint, 255);
        if label {
            let l = self.layout;
            s.blit(&t.label, x + l.icon / 2 - t.label.w as i32 / 2, y + l.icon + l.label_gap, 255, 255);
        }
    }

    fn draw_pages(&self, s: &mut Surface, ox: i32) {
        let l = self.layout;
        let off = self.pager.offset().round() as i32;
        let pressed = self.pager.pressed();
        for (i, t) in self.grid.iter().enumerate() {
            let dx = (i / PER_PAGE) as i32 * l.w - off;
            if dx <= -l.w || dx >= l.w {
                continue;
            }
            let r = l.cells[i % PER_PAGE];
            self.draw_tile(s, t, r.x + dx - ox, r.y, pressed == Some(self.dock.len() + i), true);
        }
    }

    fn draw_dots(&self, s: &mut Surface, ox: i32) {
        let l = self.layout;
        let (current, pages) = self.dots();
        if pages < 2 {
            return;
        }
        let total = (pages as i32 - 1) * l.dot_pitch + l.dot;
        let x0 = (l.w - total) / 2 - ox;
        for p in 0..pages {
            let color = if p == current { rgba(255, 255, 255, 255) } else { rgba(255, 255, 255, 90) };
            let x = (x0 + p as i32 * l.dot_pitch) as f32;
            let d = l.dot as f32;
            s.fill_rrect(x, l.dots_y as f32 - d / 2.0, d, d, d / 2.0, color);
        }
    }

    fn draw_dock(&self, s: &mut Surface, ox: i32) {
        let l = self.layout;
        if let Some(bg) = self.dock_bg {
            s.blit(bg, l.dock.x - ox, l.dock.y, 255, 255);
        }
        let pressed = self.pager.pressed();
        for (i, (t, r)) in self.dock.iter().zip(&l.dock_icons).enumerate() {
            self.draw_tile(s, t, r.x - ox, r.y, pressed == Some(i), false);
        }
        if let (Some(img), Some(r)) = (self.extras.and_then(|e| e.dock_library), l.dock_icons.get(self.dock.len())) {
            s.blit(img, r.x - ox, r.y, 255, 255);
        }
        if l.dock_sep > 0 && l.dock_sep < l.dock_icons.len().min(self.dock.len()) {
            let (a, b) = (l.dock_icons[l.dock_sep - 1], l.dock_icons[l.dock_sep]);
            let w = l.pt(1.0).round().max(1.0);
            let x = ((a.x + a.w + b.x) as f32 - w) / 2.0 - ox as f32;
            let h = a.h as f32 * 0.75;
            s.fill_rrect(x, a.y as f32 + (a.h as f32 - h) / 2.0, w, h, w / 2.0, rgba(255, 255, 255, 90));
        }
    }
}

/// A grey squircle for apps whose icon could not be loaded.
pub fn placeholder_icon(size: usize) -> Image {
    let mut img = Image { w: size, h: size, px: vec![rgba(120, 120, 128, 255); size * size] };
    img.apply_mask(&crate::gfx::squircle_mask(size));
    img
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gfx::{pack, Damage};

    const W: i32 = 1200;
    const H: i32 = 1920;

    fn tile(color: u32, l: &Layout) -> Tile {
        Tile { icon: Image { w: l.icon as usize, h: l.icon as usize, px: vec![color; (l.icon * l.icon) as usize] }, label: Image::new(10, 4) }
    }

    #[test]
    fn hit_finds_dock_then_grid_and_respects_pages() {
        let l = Layout::new(W, H, 2);
        let d = l.dock_icons[1];
        let grid = PER_PAGE + 10;
        assert_eq!(hit(&l, 0.0, 2, grid, d.x as f32 + 5.0, d.y as f32 + 5.0), Some(1));
        let c = l.cells[5];
        assert_eq!(hit(&l, 0.0, 2, grid, c.x as f32 + 5.0, c.y as f32 + 5.0), Some(2 + 5));
        // page 2, same cell
        assert_eq!(hit(&l, W as f32, 2, grid, c.x as f32 + 5.0, c.y as f32 + 5.0), Some(2 + PER_PAGE + 5));
        // empty slot past the end of the grid
        assert_eq!(hit(&l, W as f32, 2, PER_PAGE + 3, c.x as f32 + 5.0, c.y as f32 + 5.0), None);
        // between icons
        assert_eq!(hit(&l, 0.0, 2, grid, 1.0, 1.0), None);
    }

    #[test]
    fn draw_places_icons_and_pressed_icon_is_darker() {
        let l = Layout::new(W, H, 1);
        let wp = Image { w: W as usize, h: H as usize, px: vec![pack(20, 20, 20, 255); (W * H) as usize] };
        let white = pack(255, 255, 255, 255);
        let dock = [tile(white, &l)];
        let grid = [tile(white, &l), tile(white, &l)];
        let mut pager = Pager::new(1, W as f32);
        let chrome = Chrome::default();
        let bg = dock_background(wp.view(), &l);
        let scene = Scene { layout: &l, wallpaper: wp.view(), dock_bg: bg.as_ref(), dock: &dock, grid: &grid, pager: &pager, chrome: &chrome, today: None, extras: None };
        let mut buf = vec![0u32; (W * H) as usize];
        scene.draw(&mut Surface { w: W as usize, h: H as usize, stride: W as usize, px: &mut buf, y0: 0 });
        let at = |buf: &[u32], r: crate::layout::Rect| buf[(r.y + 10) as usize * W as usize + (r.x + 10) as usize];
        assert_eq!(at(&buf, l.cells[0]), white);
        assert_eq!(at(&buf, l.cells[1]), white);
        assert_eq!(at(&buf, l.cells[2]), pack(20, 20, 20, 255), "empty cell shows wallpaper");
        assert_eq!(at(&buf, l.dock_icons[0]), white);

        let c = l.cells[1];
        pager.down(c.x as f32 + 5.0, c.y as f32 + 5.0, 0.0, Some(2));
        let scene = Scene { layout: &l, wallpaper: wp.view(), dock_bg: bg.as_ref(), dock: &dock, grid: &grid, pager: &pager, chrome: &chrome, today: None, extras: None };
        scene.draw(&mut Surface { w: W as usize, h: H as usize, stride: W as usize, px: &mut buf, y0: 0 });
        assert!(at(&buf, l.cells[1]) & 0xff < 200);
        assert_eq!(at(&buf, l.cells[0]), white);
    }

    /// A two-page home screen: one dock tile and PER_PAGE + 2 grid tiles.
    fn two_pages(l: &Layout) -> (Image, Vec<Tile>, Vec<Tile>, Option<Image>) {
        let wp = Image { w: W as usize, h: H as usize, px: (0..(W * H) as usize).map(|i| pack((i % 251) as u8, (i % 241) as u8, (i % 239) as u8, 255)).collect() };
        let dock = vec![tile(pack(255, 255, 255, 255), l)];
        let grid = (0..PER_PAGE + 2).map(|i| tile(pack(200, (i * 9) as u8, 50, 255), l)).collect();
        let bg = dock_background(wp.view(), l);
        (wp, dock, grid, bg)
    }

    fn drag(pager: &mut Pager, frame: usize) {
        let t = frame as f64 * 16.0;
        match frame {
            0 => pager.down(1000.0, 800.0, t, None),
            1..=29 => pager.moved(1000.0 - 25.0 * frame as f32, 800.0, t),
            30 => {
                pager.up(250.0, 800.0, t, None);
            }
            _ => {}
        }
        pager.tick(t);
    }

    #[test]
    fn damage_follows_the_tiles_the_dots_and_the_pressed_icon() {
        let l = Layout::new(W, H, 1);
        let (wp, dock, grid, bg) = two_pages(&l);
        let mut pager = Pager::new(2, W as f32);
        let chrome = Chrome::default();
        macro_rules! scene {
            ($pager:expr) => {
                Scene { layout: &l, wallpaper: wp.view(), dock_bg: bg.as_ref(), dock: &dock, grid: &grid, pager: $pager, chrome: &chrome, today: None, extras: None }
            };
        }
        let mut d = Damage::new(W as usize, H as usize);
        scene!(&pager).damage(None, &mut d);
        assert!(d.is_all(), "first frame");
        let rest = scene!(&pager).state();
        d.clear();
        scene!(&pager).damage(Some(rest), &mut d);
        assert!(d.is_empty(), "nothing changed");
        for changed in [HomeState { grid: rest.grid - 1, ..rest }, HomeState { pages: 3, ..rest }, HomeState { dock: 0, ..rest }] {
            d.clear();
            scene!(&pager).damage(Some(changed), &mut d);
            assert!(d.is_all(), "icon set or page count changed: {changed:?}");
        }

        pager.down(1000.0, 800.0, 0.0, None);
        pager.moved(900.0, 800.0, 50.0);
        d.clear();
        scene!(&pager).damage(Some(rest), &mut d);
        assert!(!d.is_all() && !d.is_empty());
        let (top, bottom) = (l.status_h as usize, (l.dots_y - l.dot) as usize);
        assert!(d.areas().iter().all(|a| a.y0 >= top && a.y1 <= bottom), "a swipe touches only the grid rows: {:?}", d.areas());
        assert!(d.pixels() < (W * H) as usize / 2);

        pager.moved(1000.0 - W as f32, 800.0, 100.0);
        let page1 = scene!(&pager).state();
        assert_eq!(page1.page, 1);
        d.clear();
        scene!(&pager).damage(Some(rest), &mut d);
        let dots_row = l.dots_y as usize;
        assert!(d.areas().iter().any(|a| a.y0 <= dots_row && a.y1 > dots_row), "page change redraws the dots: {:?}", d.areas());
        pager.up(1000.0 - W as f32, 800.0, 150.0, None);
        pager.tick(5000.0);

        let r = l.dock_icons[0];
        pager.down(r.x as f32 + 5.0, r.y as f32 + 5.0, 6000.0, Some(0));
        d.clear();
        let before = HomeState { pressed: None, ..scene!(&pager).state() };
        scene!(&pager).damage(Some(before), &mut d);
        assert_eq!(d.areas().len(), 1);
        let a = d.areas()[0];
        assert_eq!((a.x0, a.y0, a.x1, a.y1), (r.x as usize, r.y as usize, (r.x + r.w) as usize, (r.y + r.h) as usize), "pressing a dock icon dirties just that icon");
    }

    #[test]
    fn incremental_redraw_matches_a_full_redraw_through_a_swipe() {
        let l = Layout::new(W, H, 1);
        let (wp, dock, grid, bg) = two_pages(&l);
        let mut pager = Pager::new(2, W as f32);
        let chrome = Chrome::default();
        let (w, h) = (W as usize, H as usize);
        let mut incremental = vec![0u32; w * h];
        let mut full = vec![0u32; w * h];
        let mut damage = Damage::new(w, h);
        let mut rows = Vec::new();
        let mut prev = None;
        let mut partial = 0;
        for frame in 0..60 {
            drag(&mut pager, frame);
            let scene = Scene { layout: &l, wallpaper: wp.view(), dock_bg: bg.as_ref(), dock: &dock, grid: &grid, pager: &pager, chrome: &chrome, today: None, extras: None };
            damage.clear();
            scene.damage(prev, &mut damage);
            prev = Some(scene.state());
            damage.rows(&mut rows);
            if scene.can_clip() && !damage.is_all() {
                Surface { w, h, stride: w, px: &mut incremental, y0: 0 }.draw_rows(&rows, 3, &|s| scene.draw_clipped(s, damage.areas()));
            } else {
                Surface { w, h, stride: w, px: &mut incremental, y0: 0 }.draw_rows(&rows, 3, &|s| scene.draw(s));
            }
            scene.draw(&mut Surface { w, h, stride: w, px: &mut full, y0: 0 });
            assert!(incremental == full, "frame {frame} differs");
            partial += (!damage.is_all() && !damage.is_empty()) as usize;
        }
        assert!(partial > 30, "most swipe frames are partial redraws ({partial})");
        assert!(pager.page() == 1 && !pager.animating());
    }

    #[test]
    fn today_page_frosts_the_frame_adds_a_dot_and_damages_everything_while_in_view() {
        let l = Layout::new(W, H, 1);
        let (wp, dock, grid, bg) = two_pages(&l);
        let (ui, mut today) = crate::shell::today::tests::today();
        let info = crate::shell::Info { battery: 50, ..Default::default() };
        today.refresh(&ui, &crate::shell::today::Sources { info: &info, term: None, unix: 0 }, 0.0);
        today.set_wallpaper(wp.view());
        let mut pager = Pager::new(1, W as f32);
        pager.set_leading(true);
        let chrome = Chrome::default();
        macro_rules! scene {
            ($pager:expr, $today:expr) => {
                Scene { layout: &l, wallpaper: wp.view(), dock_bg: bg.as_ref(), dock: &dock, grid: &grid, pager: $pager, chrome: &chrome, today: $today, extras: None }
            };
        }
        assert_eq!(scene!(&pager, Some(&today)).dots(), (1, 2), "one page plus Today: the home dot is the second");
        assert_eq!(scene!(&pager, None).dots(), (1, 2));
        let (w, h) = (W as usize, H as usize);
        let mut buf = vec![0u32; w * h];
        scene!(&pager, Some(&today)).draw(&mut Surface { w, h, stride: w, px: &mut buf, y0: 0 });
        let home = buf.clone();
        let rest = scene!(&pager, Some(&today)).state();
        assert_eq!(rest.today, today.gen());
        let mut d = Damage::new(w, h);
        // a refresh while the page is hidden costs nothing
        today.refresh(&ui, &crate::shell::today::Sources { info: &crate::shell::Info { battery: 20, ..Default::default() }, term: None, unix: 0 }, 1.0);
        scene!(&pager, Some(&today)).damage(Some(rest), &mut d);
        assert!(d.is_empty(), "hidden page changes are free: {:?}", d.areas());
        let rest = scene!(&pager, Some(&today)).state();
        // swipe right: the page slides in and the frost touches every row
        pager.down(100.0, 800.0, 0.0, None);
        pager.moved(700.0, 800.0, 50.0);
        let mid = scene!(&pager, Some(&today)).state();
        assert!(mid.offset < 0);
        d.clear();
        scene!(&pager, Some(&today)).damage(Some(rest), &mut d);
        assert!(d.is_all());
        pager.moved(100.0 + W as f32, 800.0, 100.0);
        pager.up(100.0 + W as f32, 800.0, 150.0, None);
        pager.tick(5000.0);
        assert!(pager.leading());
        assert_eq!(scene!(&pager, Some(&today)).dots(), (0, 2));
        scene!(&pager, Some(&today)).draw(&mut Surface { w, h, stride: w, px: &mut buf, y0: 0 });
        assert_ne!(buf, home);
        let r = today.placed()[0].rect;
        let tile = buf[(r.y + 10) as usize * w + (r.x + r.w / 2) as usize];
        assert!(tile & 0xff < 60, "a widget tile shows: {tile:08x}");
        let r = l.dock_icons[0];
        let icon_px = buf[(r.y + 5) as usize * w + (r.x + 5) as usize];
        assert_eq!(icon_px, home[(r.y + 5) as usize * w + (r.x + 5) as usize], "the dock icons stay");
        let shown = scene!(&pager, Some(&today)).state();
        d.clear();
        scene!(&pager, Some(&today)).damage(Some(shown), &mut d);
        assert!(d.is_empty());
        today.refresh(&ui, &crate::shell::today::Sources { info: &crate::shell::Info { battery: 90, ..Default::default() }, term: None, unix: 0 }, 2.0);
        d.clear();
        scene!(&pager, Some(&today)).damage(Some(shown), &mut d);
        assert!(d.is_all(), "a tile changed while in view");
    }

    #[test]
    fn dock_background_is_rounded() {
        let l = Layout::new(W, H, 3);
        let wp = Image { w: W as usize, h: H as usize, px: vec![pack(200, 100, 50, 255); (W * H) as usize] };
        let bg = dock_background(wp.view(), &l).unwrap();
        assert_eq!((bg.w as i32, bg.h as i32), (l.dock.w, l.dock.h));
        assert_eq!(bg.px[0] >> 24, 0, "corner is clear");
        assert_eq!(bg.px[bg.w * bg.h / 2 + bg.w / 2] >> 24, 255);
    }
}
