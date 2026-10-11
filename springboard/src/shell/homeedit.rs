//! Home-screen editing glue: builds the visible pages, widgets and App
//! Library from `HomeConf`, routes touches on the library page, detects the
//! long presses (page dots: page editor; page: Add Widget gallery) and runs
//! the two overlays. The pure parts live in `homeconf`, `applib`,
//! `pageedit` and `widgetgallery`.

use super::applib::{self, AppLibrary};
use super::folders;
use super::homeconf::{self, Size, GALLERY};
use super::pageedit::PageEditor;
use super::widgetgallery::{self, Gallery};
use super::widgets::{self, Kind};
use super::{Drawn, Effect, Event, Screen, Shell, apps, IN_DOCK};
use crate::gfx::{rrect_mask, Image, Surface};
use crate::scene::{self, Extras, Tile};
use crate::layout::{COLS, PER_PAGE};

/// How long a finger rests before it counts as a long press.
const HOLD_MS: f64 = 550.0;
/// How far it may wander meanwhile, px.
const HOLD_SLOP: f32 = 24.0;
const WIDGET_RADIUS: f32 = 6.0;

/// A touch on a home page that may turn into a long press.
#[derive(Clone, Copy, Debug)]
pub(super) struct Hold {
    t0: f64,
    x: f32,
    y: f32,
    /// on the page dots (page editor) rather than the page (widget gallery)
    dots: bool,
}

impl Shell {
    /// Rebuilds the visible pages, their tiles, the widgets and the pager
    /// from `conf`. With no widgets and the default order this is the plain
    /// grid plus the App Library page.
    pub(super) fn rebuild_home(&mut self) {
        let total = self.grid.len();
        let entries: Vec<usize> = self.folders.entries(total).into_iter().filter(|&e| e >= total || !self.removed.contains(&e)).collect();
        self.home = homeconf::build(entries.len(), &self.conf);
        // slots hold grid indices; `total + f` stands for folder `f`
        let remap = |slots: &mut Vec<Option<usize>>| slots.iter_mut().for_each(|s| *s = s.and_then(|e| entries.get(e).copied()));
        remap(&mut self.home.slots);
        self.home.flow.iter_mut().for_each(|p| remap(&mut p.slots));
        let tiles: Vec<Tile> = self.home.slots.iter().map(|s| s.map_or_else(|| Tile { icon: Image::new(1, 1), label: Image::new(1, 1) }, |i| self.entry_tile(i))).collect();
        self.home_grid = tiles;
        self.pager.set_pages(self.home.pages + 1);
        self.rebuild_widgets();
        self.home_gen = self.home_gen.wrapping_add(1);
        self.drawn = Drawn::Nothing;
        self.dirty = true;
    }

    /// The tile of a home entry: an app's, or folder `i - grid.len()`'s mini-icon grid.
    fn entry_tile(&self, i: usize) -> Tile {
        let total = self.grid.len();
        if i < total {
            return self.grid[i].clone();
        }
        let f = i - total;
        let icons: Vec<&Image> = self.folders.members(f, total).into_iter().filter_map(|a| self.base_icons.get(IN_DOCK + a)).collect();
        folders::tile(&self.ui, &self.layout, self.folders.list.get(f).map_or("", |fo| fo.name.as_str()), &icons)
    }

    /// The folder a pager slot (dock first, then the visible cells) stands for.
    pub(super) fn slot_folder(&self, slot: usize) -> Option<usize> {
        let e = self.home.slots.get(slot.checked_sub(self.dock.len())?).copied().flatten()?;
        e.checked_sub(self.grid.len())
    }

    /// Opens folder `f` over a blurred copy of the home screen.
    pub(super) fn open_folder(&mut self, f: usize) {
        let total = self.grid.len();
        let items: Vec<(usize, Tile)> = self.folders.members(f, total).into_iter().map(|a| (a, self.grid[a].clone())).collect();
        let Some(fo) = self.folders.list.get(f) else { return };
        let mut sharp = Image::new(self.w as usize, self.h as usize);
        self.draw_home(&mut widgets::surface(&mut sharp));
        self.folder_view = Some(folders::View::open(&self.ui, &self.layout, sharp, &fo.name, items));
        self.drawn = Drawn::Nothing;
        self.dirty = true;
    }

    /// Re-renders the widgets placed on the visible pages from the Today tiles:
    /// each fills its cells' footprint, the tile scaled to fit and centred.
    pub(super) fn rebuild_widgets(&mut self) {
        let l = &self.layout;
        let label = l.label_gap + l.pt(16.0) as i32;
        let mut out = Vec::new();
        for &(page, w) in &self.home.widgets {
            let (cw, ch) = w.size.cells();
            let at = |col: usize, row: usize| l.cells[row * COLS + col];
            let (first, last_col, last_row) = (at(w.col, w.row), at(w.col + cw - 1, w.row), at(w.col, w.row + ch - 1));
            let (fw, fh) = ((last_col.x + l.icon - first.x).max(1) as usize, (last_row.y + l.icon + label - first.y).max(1) as usize);
            let mut img = Image { w: fw, h: fh, px: vec![widgets::background(); fw * fh] };
            img.apply_mask(&rrect_mask(fw, fh, WIDGET_RADIUS));
            if let Some(tile) = self.today.tile(w.kind) {
                let k = (fw as f32 / tile.w as f32).min(fh as f32 / tile.h as f32);
                let (nw, nh) = (((tile.w as f32 * k) as usize).max(1), ((tile.h as f32 * k) as usize).max(1));
                let scaled = tile.resized(nw, nh);
                widgets::surface(&mut img).blit(&scaled, ((fw - nw) / 2) as i32, ((fh - nh) / 2) as i32, 255, 255);
            }
            out.push(scene::HomeWidget { page, x: first.x, y: first.y, img });
        }
        self.home_widgets = out;
    }

    /// Re-renders the library's images (new icon size or text scale).
    pub(super) fn rebuild_library(&mut self) {
        self.library = AppLibrary::new(&self.ui, &self.layout, &self.base_icons);
        self.forget_other_library();
        self.home_gen = self.home_gen.wrapping_add(1);
        self.dirty = true;
    }

    pub(super) fn extras(&self) -> Extras<'_> {
        Extras { dock_library: self.lib_dock.as_ref(), widgets: &self.home_widgets, library: Some(&self.library), gen: self.home_gen.wrapping_mul(31).wrapping_add(self.library.gen()) }
    }

    /// The `apps()` index the pager slot (dock first, then the visible grid) stands for.
    pub(super) fn slot_app(&self, slot: usize) -> Option<usize> {
        if slot < self.dock.len() {
            return self.dock_apps.get(slot).copied();
        }
        self.home.slots.get(slot - self.dock.len()).copied().flatten().filter(|&a| a < self.grid.len()).map(|a| a + IN_DOCK)
    }

    /// Where app `i` (an `apps()` index) sits on a page, for the launch zoom.
    pub(super) fn grid_cell(&self, i: usize) -> usize {
        let g = i - IN_DOCK;
        self.home.slots.iter().position(|s| *s == Some(g)).unwrap_or(g) % PER_PAGE
    }

    /// True while the App Library (the last page) is the nearest one.
    pub(super) fn on_library(&self) -> bool {
        self.pager.pages >= 2 && self.pager.page() + 1 == self.pager.pages
    }

    /// A touch that began on the library page: vertical drags and taps are
    /// the page's, horizontal swipes the pager's.
    pub(super) fn library_touch(&mut self, ev: Event, t: f64) {
        match ev {
            Event::Down(x, y) => {
                self.pager.down(x, y, t, None);
                self.library.down(&self.layout, x, y, t);
            }
            Event::Move(x, y) => {
                if self.library.moved(x, y, t) {
                    self.pager.cancel(t);
                } else if !self.library.owns() {
                    self.pager.moved(x, y, t);
                }
            }
            Event::Up(x, y) => {
                let out = if self.library.owns() {
                    self.pager.cancel(t);
                    self.library.up(&self.layout, x, y, t)
                } else {
                    let out = self.library.up(&self.layout, x, y, t);
                    self.pager.up(x, y, t, None);
                    out
                };
                match out {
                    Some(applib::Out::Launch(app)) => {
                        self.open(apps()[app].id, t);
                    }
                    Some(applib::Out::Search) => self.quicksearch_show(t),
                    None => {}
                }
            }
            Event::Power | Event::Wake | Event::VolumeUp | Event::VolumeDown | Event::Screenshot => {}
        }
    }

    /// Starts or cancels the long-press timer for a touch on a home page.
    pub(super) fn hold_touch(&mut self, ev: Event, t: f64) {
        match ev {
            Event::Down(x, y) => {
                self.hold = self.hold_zone(x, y).map(|dots| Hold { t0: t, x, y, dots });
            }
            Event::Move(x, y) => {
                if self.hold.is_some_and(|h| (x - h.x).abs() > HOLD_SLOP || (y - h.y).abs() > HOLD_SLOP) {
                    self.hold = None;
                }
            }
            _ => self.hold = None,
        }
    }

    /// Some(true) on the page dots, Some(false) on the page, None elsewhere (dock, status bar).
    fn hold_zone(&self, x: f32, y: f32) -> Option<bool> {
        let l = &self.layout;
        let pages = self.home.pages + 1 + 1;
        let half = ((pages as i32 - 1) * l.dot_pitch + l.dot) as f32 / 2.0 + l.pt(24.0);
        if (y - l.dots_y as f32).abs() <= l.pt(18.0) && (x - l.w as f32 / 2.0).abs() <= half {
            return Some(true);
        }
        (y >= l.status_h as f32 && y < l.dock.y as f32).then_some(false)
    }

    /// When the pending long press fires, for `next_wake`.
    pub(super) fn home_wake(&self) -> Option<f64> {
        self.hold.map(|h| h.t0 + HOLD_MS)
    }

    /// Fires a due long press and advances the library and gallery scrolls.
    pub(super) fn home_tick(&mut self, t: f64) {
        if let Some(h) = self.hold {
            if self.screen != Screen::Home || self.anim.is_some() || self.quicksearch.active() {
                self.hold = None;
            } else if t - h.t0 >= HOLD_MS {
                self.hold = None;
                // the finger is still down: the pager must not turn its release into a launch
                self.pager.cancel(t);
                if h.dots {
                    self.open_page_editor();
                } else if self.hold_on_icon(h.x, h.y) {
                    self.enter_jiggle(t);
                } else {
                    self.open_gallery();
                }
                self.dirty = true;
            }
        }
        self.jiggle_tick(t);
        if self.library.tick(t) {
            self.dirty = true;
        }
        if self.gallery.as_mut().is_some_and(|g| g.tick(t)) {
            self.dirty = true;
        }
    }

    pub(super) fn home_animating(&self) -> bool {
        self.jiggle.is_some() || self.library.animating() || self.gallery.as_ref().is_some_and(Gallery::animating)
    }

    fn open_page_editor(&mut self) {
        self.page_editor = Some(PageEditor::open(&self.ui, &self.layout, &self.base_icons, &self.home, &self.conf, IN_DOCK));
        self.drawn = Drawn::Nothing;
    }

    fn open_gallery(&mut self) {
        self.gallery_page = self.pager.page().min(self.home.pages.saturating_sub(1));
        let tiles: Vec<(Kind, &Image)> = GALLERY.iter().filter_map(|&k| self.today.tile(k).map(|img| (k, img))).collect();
        let here = self.here_widgets();
        self.gallery = Some(Gallery::open(&self.ui, &tiles, &here));
        self.drawn = Drawn::Nothing;
    }

    /// The widgets on the gallery's page: (index in `conf.widgets`, kind, size).
    fn here_widgets(&self) -> Vec<(usize, Kind, Size)> {
        let id = self.home.shown.get(self.gallery_page).copied();
        self.conf.widgets.iter().enumerate().filter(|(_, w)| Some(w.page) == id).map(|(i, w)| (i, w.kind, w.size)).collect()
    }

    /// Closes the page editor and the gallery without applying anything further.
    pub(super) fn close_home_overlays(&mut self) {
        self.hold = None;
        self.jiggle = None;
        if self.page_editor.take().is_some() | self.gallery.take().is_some() | self.folder_view.take().is_some() {
            self.drawn = Drawn::Nothing;
            self.dirty = true;
        }
    }

    /// Stores the edits and rebuilds the home pages.
    fn commit_home(&mut self) {
        self.conf.save(&mut self.prefs);
        self.rebuild_home();
        let here = self.here_widgets();
        if let Some(g) = self.gallery.as_mut() {
            g.set_here(&self.ui, &here);
        }
    }

    /// Routes a touch to the page editor or gallery while one is open.
    pub(super) fn overlay_event(&mut self, ev: Event, t: f64) -> Option<Vec<Effect>> {
        if self.page_editor.is_none() && self.gallery.is_none() && self.folder_view.is_none() {
            return None;
        }
        self.dirty = true;
        if let Some(v) = self.folder_view.as_mut() {
            let out = match ev {
                Event::Down(x, y) => {
                    v.down(x, y);
                    None
                }
                Event::Up(x, y) => v.up(x, y),
                _ => None,
            };
            match out {
                Some(folders::Out::Close) => self.close_home_overlays(),
                Some(folders::Out::Launch(a)) => {
                    self.close_home_overlays();
                    self.open(apps()[a + IN_DOCK].id, t);
                }
                None => {}
            }
            return Some(vec![]);
        }
        if let Some(e) = self.page_editor.as_mut() {
            let done = match ev {
                Event::Down(x, y) => {
                    e.down(&self.layout, x, y);
                    false
                }
                Event::Move(x, y) => {
                    e.moved(x, y);
                    false
                }
                Event::Up(x, y) => e.up(&self.layout, x, y),
                _ => false,
            };
            if done {
                if let Some(e) = self.page_editor.take() {
                    let (order, hidden) = e.result();
                    self.conf.order = order;
                    self.conf.hidden = hidden;
                    self.commit_home();
                    self.pager.jump_to(0);
                }
            }
            return Some(vec![]);
        }
        let out = self.gallery.as_mut().and_then(|g| match ev {
            Event::Down(x, y) => {
                g.down(&self.layout, x, y, t);
                None
            }
            Event::Move(x, y) => {
                g.moved(x, y, t);
                None
            }
            Event::Up(x, y) => g.up(&self.layout, x, y, t),
            _ => None,
        });
        if let Some(out) = out {
            self.gallery_action(out);
        }
        Some(vec![])
    }

    fn gallery_action(&mut self, out: widgetgallery::Out) {
        use widgetgallery::Out;
        match out {
            Out::Close => self.close_home_overlays(),
            Out::Add(kind, size) => {
                let Some(&page) = self.home.shown.get(self.gallery_page) else { return };
                if let Some(p) = homeconf::find_spot(&self.conf.widgets, page, kind, size) {
                    self.conf.widgets.push(p);
                    self.commit_home();
                }
            }
            Out::Resize(i) => {
                if homeconf::resize(&mut self.conf.widgets, i) {
                    self.commit_home();
                }
            }
            Out::Remove(i) => {
                if i < self.conf.widgets.len() {
                    self.conf.widgets.remove(i);
                    self.commit_home();
                }
            }
        }
    }

    /// The home screen with the open overlay over it.
    pub(super) fn draw_home_overlay(&self, s: &mut Surface) {
        if self.jiggle.is_some() {
            return self.draw_jiggle(s);
        }
        self.draw_home(s);
        if let Some(e) = &self.page_editor {
            e.draw(s, &self.ui, &self.layout);
        }
        if let Some(g) = &self.gallery {
            g.draw(s, &self.layout);
        }
        if let Some(v) = &self.folder_view {
            v.draw(s);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::{shell, unlock};
    use super::*;

    fn home() -> (Shell, f64) {
        let mut sh = shell();
        unlock(&mut sh);
        (sh, 3000.0)
    }

    #[test]
    fn without_edits_the_home_grid_is_the_plain_one_plus_the_library_page() {
        let (sh, _) = home();
        assert_eq!(sh.home_grid.len(), sh.grid.len());
        assert!((0..sh.grid.len()).all(|i| sh.slot_app(sh.dock.len() + i) == Some(IN_DOCK + i)));
        assert_eq!(sh.pager.pages, crate::layout::Layout::pages(sh.grid.len()) + 1);
        assert!(sh.home_widgets.is_empty());
    }

    #[test]
    fn a_widget_takes_cells_and_pushes_apps_on() {
        let (mut sh, _) = home();
        sh.conf.widgets.push(homeconf::Placed { kind: Kind::Battery, size: Size::Small, page: 0, col: 0, row: 0 });
        sh.rebuild_home();
        assert_eq!(sh.home_widgets.len(), 1);
        assert_eq!(sh.home.slots[0], None, "the first cell is under the widget");
        assert_eq!(sh.slot_app(sh.dock.len()), None);
        assert_eq!(sh.slot_app(sh.dock.len() + 2), Some(IN_DOCK));
    }

    #[test]
    fn hidden_pages_leave_the_grid_but_stay_in_the_library() {
        let (mut sh, _) = home();
        sh.conf.widgets.push(homeconf::Placed { kind: Kind::Clock, size: Size::Medium, page: 1, col: 0, row: 0 });
        sh.conf.hidden = vec![1];
        sh.rebuild_home();
        assert_eq!(sh.home.pages, 1);
        assert!((0..apps().len()).all(|i| sh.library.card_apps(5).contains(&i)));
    }

    #[test]
    fn a_long_press_on_the_dots_opens_the_page_editor_and_on_the_page_the_gallery() {
        let (mut sh, t) = home();
        let (x, y) = (sh.w as f32 / 2.0, sh.layout.dots_y as f32);
        sh.event(Event::Down(x, y), t);
        sh.tick(t + HOLD_MS + 10.0);
        assert!(sh.page_editor.is_some());
        sh.event(Event::Up(x, y), t + 800.0);
        sh.close_home_overlays();
        // a long press on an icon starts edit (jiggle) mode instead
        let c = sh.layout.cells[7];
        sh.event(Event::Down((c.x + 5) as f32, (c.y + 5) as f32), t + 900.0);
        sh.tick(t + 900.0 + HOLD_MS + 10.0);
        assert!(sh.jiggle.is_some() && sh.gallery.is_none());
        sh.event(Event::Up((c.x + 5) as f32, (c.y + 5) as f32), t + 1500.0);
        sh.close_home_overlays();
        // an empty cell (under a widget) opens the gallery
        sh.conf.widgets.push(homeconf::Placed { kind: Kind::Battery, size: Size::Small, page: 0, col: 0, row: 0 });
        sh.rebuild_home();
        let c = sh.layout.cells[0];
        sh.event(Event::Down((c.x + 5) as f32, (c.y + 5) as f32), t + 2000.0);
        sh.tick(t + 2000.0 + HOLD_MS + 10.0);
        assert!(sh.gallery.is_some());
        assert_eq!(sh.screen, Screen::Home, "the release of the long press launched nothing");
    }

    #[test]
    fn tapping_a_folder_slot_opens_the_folder_without_launching() {
        let (mut sh, t) = home();
        sh.folders.list.push(folders::Folder { name: "F".into(), apps: vec![0, 1] });
        sh.rebuild_home();
        assert_eq!(sh.slot_app(sh.dock.len()), None, "a folder slot is no app");
        let c = sh.layout.cells[0];
        let (x, y) = ((c.x + 5) as f32, (c.y + 5) as f32);
        sh.event(Event::Down(x, y), t);
        sh.event(Event::Up(x, y), t + 50.0);
        assert!(sh.folder_view.is_some());
        assert_eq!(sh.screen, Screen::Home);
    }
}
