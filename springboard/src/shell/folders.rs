//! Home-screen folders: which grid apps a folder holds (persisted in prefs),
//! the folder icon (3x3 mini icons on a translucent squircle) and the open
//! folder (blurred backdrop, rounded panel with the title, 3x3 icons per page).

use super::control_center::Frost;
use super::prefs::Prefs;
use super::ui::{DOCK_LIBRARY_FILL, Ui, Weight, FOLDER_FILL, FOLDER_ICON_FILL, WHITE};
use super::widgets;
use crate::gfx::{rgba, squircle_mask, Image, Surface};
use crate::layout::Layout;
use crate::scene::Tile;

const KEY: &str = "home.folders";
const PER_FOLDER_PAGE: usize = 9;
const PANEL_W_PT: f32 = 480.0;
const PANEL_RADIUS_PT: f32 = 38.0;
const TITLE_PT: f32 = 64.0;
const ROW_GAP_PT: f32 = 22.0;
const PAD_PT: f32 = 20.0;
const DOTS_PT: f32 = 28.0;
const SWIPE_PX: f32 = 40.0;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Folder {
    pub name: String,
    /// grid app indices (into `Shell::grid`)
    pub apps: Vec<usize>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Folders {
    pub list: Vec<Folder>,
}

impl Folders {
    /// `Name:1,2,3;Other:4,5` (names never hold `:` or `;`).
    pub fn load(prefs: &Prefs) -> Folders {
        let mut list: Vec<Folder> = Vec::new();
        for rec in prefs.get(KEY).unwrap_or("").split(';') {
            let Some((name, apps)) = rec.split_once(':') else { continue };
            let apps: Vec<usize> = apps.split(',').filter_map(|a| a.trim().parse().ok()).collect();
            if !name.is_empty() && !apps.is_empty() {
                list.push(Folder { name: name.to_string(), apps });
            }
        }
        Folders { list }
    }

    /// The home entries for `total` grid apps: ids below `total` are apps,
    /// `total + f` is folder `f`. A folder stands where its first app was;
    /// unknown or repeated members are ignored and an empty folder vanishes.
    pub fn entries(&self, total: usize) -> Vec<usize> {
        let mut taken = vec![false; total];
        let members: Vec<Vec<usize>> = self
            .list
            .iter()
            .map(|f| {
                let mut v = Vec::new();
                for &a in &f.apps {
                    if a < total && !taken[a] {
                        taken[a] = true;
                        v.push(a);
                    }
                }
                v
            })
            .collect();
        let mut out = Vec::new();
        for a in 0..total {
            if let Some(f) = members.iter().position(|m| m.first() == Some(&a)) {
                out.push(total + f);
            } else if !taken[a] {
                out.push(a);
            }
        }
        out
    }

    /// The valid members of folder `f` in order (apps an earlier folder holds are not its).
    pub fn members(&self, f: usize, total: usize) -> Vec<usize> {
        let mut seen: Vec<usize> = Vec::new();
        for (i, fo) in self.list.iter().enumerate() {
            let mut mine = Vec::new();
            for &a in &fo.apps {
                if a < total && !seen.contains(&a) {
                    seen.push(a);
                    mine.push(a);
                }
            }
            if i == f {
                return mine;
            }
        }
        Vec::new()
    }
}

/// The folder's home tile: a translucent squircle with up to 9 mini icons in 3 x 3.
pub fn tile(ui: &Ui, layout: &Layout, name: &str, icons: &[&Image]) -> Tile {
    let size = layout.icon as usize;
    let mut img = Image { w: size, h: size, px: vec![FOLDER_ICON_FILL; size * size] };
    let pad = size as f32 * 0.14;
    let gap = size as f32 * 0.05;
    let cell = (size as f32 - 2.0 * pad - 2.0 * gap) / 3.0;
    let mini = (cell as usize).max(1);
    for (i, icon) in icons.iter().take(PER_FOLDER_PAGE).enumerate() {
        let (c, r) = ((i % 3) as f32, (i / 3) as f32);
        let (x, y) = (pad + c * (cell + gap), pad + r * (cell + gap));
        widgets::surface(&mut img).blit(&icon.resized(mini, mini), x.round() as i32, y.round() as i32, 255, 255);
    }
    img.apply_mask(&squircle_mask(size));
    Tile { icon: img, label: ui.styled(&ui.fit(name, 12.0, layout.label_w as f32), 12.0, Weight::Regular, WHITE, true) }
}

/// The App Library icon at the end of the dock: a dark squircle with a 2 x 2 grid of mini icons.
pub fn library_icon(size: usize, icons: &[&Image]) -> Image {
    let mut img = Image { w: size, h: size, px: vec![DOCK_LIBRARY_FILL; size * size] };
    let (pad, gap) = (size as f32 * 0.14, size as f32 * 0.08);
    let cell = (size as f32 - 2.0 * pad - gap) / 2.0;
    let mini = (cell as usize).max(1);
    for (i, icon) in icons.iter().take(4).enumerate() {
        let (c, r) = ((i % 2) as f32, (i / 2) as f32);
        widgets::surface(&mut img).blit(&icon.resized(mini, mini), (pad + c * (cell + gap)).round() as i32, (pad + r * (cell + gap)).round() as i32, 255, 255);
    }
    img.apply_mask(&squircle_mask(size));
    img
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Out {
    /// launch this grid app
    Launch(usize),
    Close,
}

/// An open folder.
pub struct View {
    frost: Frost,
    title: Image,
    /// (grid app index, icon + label) of every member
    items: Vec<(usize, Tile)>,
    page: usize,
    panel: (f32, f32, f32, f32),
    radius: f32,
    /// icon rects (x, y, w, h) of the 9 cells of a page
    cells: Vec<(i32, i32, i32, i32)>,
    dots_y: f32,
    title_y: f32,
    scale: f32,
    down: Option<(f32, f32)>,
}

impl View {
    pub fn open(ui: &Ui, l: &Layout, sharp: Image, name: &str, items: Vec<(usize, Tile)>) -> View {
        let s = l.s;
        let pw = (PANEL_W_PT * s).min(l.w as f32 - 2.0 * 16.0 * s);
        let icon = l.icon;
        let row_h = icon + l.label_gap + l.pt(16.0) as i32 + (ROW_GAP_PT * s) as i32;
        let title_h = TITLE_PT * s;
        let ph = title_h + 3.0 * row_h as f32 + PAD_PT * s + DOTS_PT * s;
        let (px, py) = ((l.w as f32 - pw) / 2.0, ((l.h as f32 - ph) * 0.42).max(l.status_h as f32));
        let col_w = (pw - 2.0 * PAD_PT * s) / 3.0;
        let cells = (0..PER_FOLDER_PAGE)
            .map(|i| {
                let (c, r) = ((i % 3) as f32, (i / 3) as i32);
                let x = px + PAD_PT * s + col_w * (c + 0.5) - icon as f32 / 2.0;
                (x as i32, (py + title_h) as i32 + r * row_h, icon, icon)
            })
            .collect();
        View {
            frost: Frost::new(sharp, 6, 150),
            title: ui.text(&ui.fit(name, 22.0, pw - 2.0 * PAD_PT * s), 22.0, Weight::Semibold, WHITE),
            items,
            page: 0,
            panel: (px, py, pw, ph),
            radius: PANEL_RADIUS_PT * s,
            cells,
            dots_y: py + ph - DOTS_PT * s * 0.6,
            title_y: py + title_h / 2.0,
            scale: s,
            down: None,
        }
    }

    fn pages(&self) -> usize {
        self.items.len().div_ceil(PER_FOLDER_PAGE).max(1)
    }

    fn at(&self, x: f32, y: f32) -> Option<usize> {
        let i = self.cells.iter().position(|&(cx, cy, w, h)| x >= cx as f32 && y >= cy as f32 && x < (cx + w) as f32 && y < (cy + h + 30) as f32)?;
        self.items.get(self.page * PER_FOLDER_PAGE + i).map(|it| it.0)
    }

    pub fn down(&mut self, x: f32, y: f32) {
        self.down = Some((x, y));
    }

    pub fn up(&mut self, x: f32, y: f32) -> Option<Out> {
        let (x0, y0) = self.down.take()?;
        let (dx, dy) = (x - x0, y - y0);
        if dx.abs() > SWIPE_PX && dx.abs() > dy.abs() {
            if dx < 0.0 && self.page + 1 < self.pages() {
                self.page += 1;
            } else if dx > 0.0 && self.page > 0 {
                self.page -= 1;
            }
            return None;
        }
        if dx.abs() > 16.0 || dy.abs() > 16.0 {
            return None;
        }
        if let Some(app) = self.at(x, y) {
            return Some(Out::Launch(app));
        }
        let (px, py, pw, ph) = self.panel;
        (!(x >= px && y >= py && x < px + pw && y < py + ph)).then_some(Out::Close)
    }

    pub fn draw(&self, s: &mut Surface) {
        s.copy(self.frost.frosted.view(), 0, 0);
        let (px, py, pw, ph) = self.panel;
        s.fill_rrect(px, py, pw, ph, self.radius, FOLDER_FILL);
        s.blit(&self.title, (px + (pw - self.title.w as f32) / 2.0) as i32, (self.title_y - self.title.h as f32 / 2.0) as i32, 255, 255);
        for (i, &(cx, cy, w, _)) in self.cells.iter().enumerate() {
            let Some((_, t)) = self.items.get(self.page * PER_FOLDER_PAGE + i) else { break };
            s.blit(&t.icon, cx, cy, 255, 255);
            s.blit(&t.label, cx + (w - t.label.w as i32) / 2, cy + t.icon.h as i32 + 4, 255, 255);
        }
        let n = self.pages();
        if n > 1 {
            let r = 3.5 * self.scale;
            let x0 = px + pw / 2.0 - (n as f32 - 1.0) * r * 2.4;
            for p in 0..n {
                let a = if p == self.page { 230 } else { 90 };
                s.fill_rrect(x0 + p as f32 * r * 4.8 - r, self.dots_y - r, 2.0 * r, 2.0 * r, r, rgba(255, 255, 255, a));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f(name: &str, apps: &[usize]) -> Folder {
        Folder { name: name.into(), apps: apps.to_vec() }
    }

    #[test]
    fn a_folder_replaces_its_apps_at_the_first_one() {
        let fo = Folders { list: vec![f("A", &[2, 4, 9, 4]), f("Empty", &[])] };
        assert_eq!(fo.entries(6), [0, 1, 6, 3, 5]);
        assert_eq!(Folders::default().entries(3), [0, 1, 2]);
    }

    #[test]
    fn a_folder_with_only_unknown_apps_vanishes() {
        let fo = Folders { list: vec![f("Gone", &[40])] };
        assert_eq!(fo.entries(2), [0, 1]);
    }

    #[test]
    fn members_drop_unknown_and_repeated_apps() {
        let fo = Folders { list: vec![f("A", &[1, 7, 1, 2])] };
        assert_eq!(fo.members(0, 4), [1, 2]);
    }
}
