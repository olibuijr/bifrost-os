//! The photo picker sheet: a card over the dimmed screen that lists the person's photo library as a grid of thumbnails.
//! The person taps the pictures to choose (up to the app's selection limit), switches between All, Photos and Videos
//! when the app allows more than one kind, and ends with Add or Cancel. The app receives only the identifiers of what was
//! chosen (see `photo_pick`); it never reads the library through this sheet.
//!
//! The model is `photo_pick::Pick`; this module is the geometry, the drawing, the touch handling and the thumbnails.
//! Thumbnails are the same cover squares the Photos app keeps (`photos::thumbs::DiskCache`, shared by key), made on one
//! worker thread, only for the rows on screen and two rows beyond, so the shell's frame never decodes a picture.
//! Touch: a tap on a picture chooses or unchooses it; dragging scrolls the grid.
use super::photo_pick::{self, Filter, Pick, Toggle};
use super::photos::thumbs::{self, DiskCache};
use super::photos::{edit, exif, video};
use super::ui::{Ui, Weight, CARD_BG, FIELD_BG, GRAY_DIM, LABEL, LABEL_2, LINK, MAC_BODY_PT, MAC_FOOTNOTE_PT, MAC_TITLE3_PT, SEPARATOR, TINT, WHITE};
use super::Event;
use crate::gfx::{rgba, Image, Surface};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, Sender};

/// The card's size and its margin to the screen edge, in pt.
const PANEL_W_PT: f32 = 560.0;
const PANEL_H_PT: f32 = 500.0;
const MARGIN_PT: f32 = 16.0;
const RADIUS_PT: f32 = 17.0;
/// Inner padding, the header (title and the line under it), the tab row, the footer, in pt.
const PAD_PT: f32 = 16.0;
const HEADER_PT: f32 = 62.0;
const TABS_PT: f32 = 40.0;
const FOOTER_PT: f32 = 52.0;
/// The tab capsules: width, height and the gap between them.
const TAB_W_PT: f32 = 84.0;
const TAB_H_PT: f32 = 26.0;
const TAB_GAP_PT: f32 = 6.0;
/// Footer buttons.
const CANCEL_W_PT: f32 = 64.0;
const ADD_W_PT: f32 = 76.0;
const BTN_H_PT: f32 = 28.0;
/// The thumbnail cell the grid aims for and the hairline between cells.
const CELL_PT: f32 = 104.0;
const GAP_PT: f32 = 2.0;
const CELL_RADIUS_PT: f32 = 4.0;
/// The selection ring and the numbered dot.
const RING_PT: f32 = 3.0;
const DOT_PT: f32 = 20.0;
/// A touch that moves less than this is a tap, not a scroll.
const TAP_SLOP_PT: f32 = 8.0;
/// Rows of thumbnails made beyond the screenful in each direction.
const AHEAD_ROWS: f32 = 2.0;
/// Thumbnails kept in memory (the pictures on screen are far fewer).
const MAX_THUMBS: usize = 480;

/// x, y, w, h in px.
type Rect = (f32, f32, f32, f32);

fn inside(r: Rect, x: f32, y: f32) -> bool {
    x >= r.0 && x < r.0 + r.2 && y >= r.1 && y < r.1 + r.3
}

/// What a touch hit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
    /// The picture at this index of the pick's entries.
    Cell(usize),
    Tab(Option<Filter>),
    Cancel,
    Add,
    /// The card itself, or outside it.
    Nothing,
}

/// What the sheet asks of the shell after a touch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Idle,
    Redraw,
    /// Add: the pick ends with the choice.
    Add,
    /// Cancel: the pick ends with no choice.
    Cancel,
}

struct Job {
    key: String,
    path: PathBuf,
}

/// The positions of one frame, in px.
struct Geo {
    card: Rect,
    pad: f32,
    tabs: Rect,
    grid: Rect,
    footer: Rect,
    cols: usize,
    cell: f32,
    gap: f32,
}

pub struct PhotoPickSheet {
    pick: Pick,
    /// The entries on show (indexes into `pick.entries`), refreshed when the tab changes.
    visible: Vec<usize>,
    scroll: f32,
    /// (y at touch down, last y, moved beyond the slop) while a touch that began on the grid is down.
    drag: Option<(f32, f32, bool)>,
    jobs: Sender<Job>,
    results: Receiver<(String, Option<Image>)>,
    /// The cover squares by thumbnail key, as the cache gives them.
    thumbs: HashMap<String, Image>,
    requested: HashSet<String>,
    failed: HashSet<String>,
    /// The squares scaled to the cell size on screen, by key.
    scaled: RefCell<HashMap<String, (usize, Image)>>,
}

impl PhotoPickSheet {
    /// The sheet for `pick`; thumbnails are kept in `thumb_dir` (the Photos app's cache directory).
    pub fn new(pick: Pick, thumb_dir: PathBuf) -> Self {
        let (jobs, job_rx) = channel::<Job>();
        let (res_tx, results) = channel::<(String, Option<Image>)>();
        let disk = DiskCache::new(thumb_dir, thumbs::MAX_FILES);
        crate::bare::sys::spawn_named("photo-pick-thumbs", move || {
            for job in job_rx {
                let image = disk.read(&job.key).or_else(|| {
                    let img = if video::is_video(&job.path) {
                        video::poster(&job.path)?
                    } else {
                        let bytes = std::fs::read(&job.path).ok()?;
                        edit::orient(crate::cg::imageio::decode(&bytes)?, exif::orientation(&bytes))
                    };
                    let thumb = crate::imgdec::cover(&img, thumbs::SIZE, thumbs::SIZE);
                    disk.write(&job.key, &thumb);
                    Some(thumb)
                });
                if res_tx.send((job.key, image)).is_err() {
                    break;
                }
            }
        });
        let visible = pick.visible();
        PhotoPickSheet {
            pick,
            visible,
            scroll: 0.0,
            drag: None,
            jobs,
            results,
            thumbs: HashMap::new(),
            requested: HashSet::new(),
            failed: HashSet::new(),
            scaled: RefCell::new(HashMap::new()),
        }
    }

    pub fn pick(&self) -> &Pick {
        &self.pick
    }

    /// The app this sheet picks for.
    pub fn app_id(&self) -> &str {
        &self.pick.app_id
    }

    fn geo(&self, ui: &Ui, w: i32, h: i32) -> Geo {
        let margin = ui.pt(MARGIN_PT);
        let cw = ui.pt(PANEL_W_PT).min(w as f32 - 2.0 * margin).max(0.0);
        let ch = ui.pt(PANEL_H_PT).min(h as f32 - 2.0 * margin).max(0.0);
        let card = (((w as f32 - cw) / 2.0).round(), ((h as f32 - ch) / 2.0).round(), cw, ch);
        let pad = ui.pt(PAD_PT);
        let tabs_h = if self.pick.tabs().is_empty() { 0.0 } else { ui.pt(TABS_PT) };
        let top = card.1 + ui.pt(HEADER_PT);
        let footer_h = ui.pt(FOOTER_PT);
        let footer = (card.0, card.1 + ch - footer_h, cw, footer_h);
        let grid_y = top + tabs_h;
        let grid = (card.0 + pad, grid_y, (cw - 2.0 * pad).max(0.0), (footer.1 - grid_y).max(0.0));
        let gap = ui.pt(GAP_PT).round().max(1.0);
        let cols = (((grid.2 + gap) / (ui.pt(CELL_PT) + gap)).floor() as usize).max(2);
        let cell = ((grid.2 - gap * (cols - 1) as f32) / cols as f32).floor().max(1.0);
        Geo { card, pad, tabs: (card.0 + pad, top, cw - 2.0 * pad, tabs_h), grid, footer, cols, cell, gap }
    }

    /// The most the grid can scroll.
    fn max_scroll(&self, g: &Geo) -> f32 {
        let rows = self.visible.len().div_ceil(g.cols);
        let content = if rows == 0 { 0.0 } else { rows as f32 * (g.cell + g.gap) - g.gap };
        (content - g.grid.3).max(0.0)
    }

    fn tab_rect(&self, ui: &Ui, g: &Geo, i: usize) -> Rect {
        // the capsules shrink on a narrow card so the row never leaves it
        let n = self.pick.tabs().len().max(1) as f32;
        let (th, tg) = (ui.pt(TAB_H_PT), ui.pt(TAB_GAP_PT));
        let tw = ui.pt(TAB_W_PT).min((g.tabs.2 - (n - 1.0) * tg) / n).max(1.0);
        (g.tabs.0 + i as f32 * (tw + tg), g.tabs.1 + (g.tabs.3 - th) / 2.0, tw, th)
    }

    fn cancel_rect(&self, ui: &Ui, g: &Geo) -> Rect {
        (g.card.0 + g.pad, g.footer.1, ui.pt(CANCEL_W_PT), g.footer.3)
    }

    fn add_rect(&self, ui: &Ui, g: &Geo) -> Rect {
        let (bw, bh) = (ui.pt(ADD_W_PT), ui.pt(BTN_H_PT));
        (g.card.0 + g.card.2 - g.pad - bw, g.footer.1 + (g.footer.3 - bh) / 2.0, bw, bh)
    }

    /// The top-left of the cell in slot `slot` of the visible list.
    fn cell_origin(&self, g: &Geo, slot: usize) -> (f32, f32) {
        let (row, col) = (slot / g.cols, slot % g.cols);
        (g.grid.0 + col as f32 * (g.cell + g.gap), g.grid.1 + row as f32 * (g.cell + g.gap) - self.scroll)
    }

    /// What is under (x, y).
    pub fn hit(&self, ui: &Ui, w: i32, h: i32, x: f32, y: f32) -> Hit {
        let g = self.geo(ui, w, h);
        if !inside(g.card, x, y) {
            return Hit::Nothing;
        }
        if inside(self.cancel_rect(ui, &g), x, y) {
            return Hit::Cancel;
        }
        if inside(self.add_rect(ui, &g), x, y) {
            return Hit::Add;
        }
        for (i, tab) in self.pick.tabs().into_iter().enumerate() {
            if inside(self.tab_rect(ui, &g, i), x, y) {
                return Hit::Tab(tab);
            }
        }
        if inside(g.grid, x, y) {
            let step = g.cell + g.gap;
            let (dx, dy) = (x - g.grid.0, y - g.grid.1 + self.scroll);
            let (col, row) = ((dx / step).floor() as usize, (dy / step).floor() as usize);
            // the hairline between two cells is not a cell
            if col < g.cols && dx - col as f32 * step < g.cell && dy - row as f32 * step < g.cell {
                if let Some(&index) = self.visible.get(row * g.cols + col) {
                    return Hit::Cell(index);
                }
            }
        }
        Hit::Nothing
    }

    /// Applies a tap.
    fn tapped(&mut self, hit: Hit) -> Outcome {
        match hit {
            Hit::Cell(index) => {
                let id = self.pick.entries[index].id.clone();
                match self.pick.toggle(&id) {
                    Toggle::Selected | Toggle::Deselected => Outcome::Redraw,
                    Toggle::Full | Toggle::Unknown => Outcome::Idle,
                }
            }
            Hit::Tab(tab) => {
                self.pick.set_tab(tab);
                self.visible = self.pick.visible();
                self.scroll = 0.0;
                Outcome::Redraw
            }
            Hit::Cancel => Outcome::Cancel,
            Hit::Add if !self.pick.selected().is_empty() => Outcome::Add,
            Hit::Add | Hit::Nothing => Outcome::Idle,
        }
    }

    /// A touch event: a tap chooses, a drag on the grid scrolls.
    pub fn event(&mut self, ui: &Ui, w: i32, h: i32, ev: Event) -> Outcome {
        let slop = ui.pt(TAP_SLOP_PT);
        match ev {
            Event::Down(x, y) => {
                let g = self.geo(ui, w, h);
                self.drag = inside(g.grid, x, y).then_some((y, y, false));
                Outcome::Idle
            }
            Event::Move(_, y) => {
                let Some((start, last, moved)) = self.drag else { return Outcome::Idle };
                let moved = moved || (y - start).abs() > slop;
                if moved {
                    let max = self.max_scroll(&self.geo(ui, w, h));
                    self.scroll = (self.scroll - (y - last)).clamp(0.0, max);
                }
                self.drag = Some((start, y, moved));
                if moved {
                    Outcome::Redraw
                } else {
                    Outcome::Idle
                }
            }
            Event::Up(x, y) => {
                let dragged = self.drag.take().is_some_and(|d| d.2);
                if dragged {
                    return Outcome::Idle;
                }
                let hit = self.hit(ui, w, h, x, y);
                self.tapped(hit)
            }
            _ => Outcome::Idle,
        }
    }

    /// Ends the pick with the choice, in pick order.
    pub fn finish(self) {
        let chosen = self.pick.selected().to_vec();
        if self.pick.app_id == photo_pick::WIDGET_APP {
            // the shell's own widget keeps the thumbnail of the picture, not an app's identifier
            let keys = chosen.iter().filter_map(|id| self.pick.entries.iter().find(|e| &e.id == id)).map(|e| e.key.clone()).collect();
            photo_pick::finish_widget(keys);
        } else {
            photo_pick::finish(&self.pick.app_id, chosen);
        }
    }

    /// Ends the pick with no choice.
    pub fn cancel(self) {
        photo_pick::cancel(&self.pick.app_id);
    }

    /// Takes in the thumbnails the worker finished and asks for those of the rows on screen and two beyond. Called from
    /// the shell's tick; true when a thumbnail arrived.
    pub fn prepare(&mut self, ui: &Ui, w: i32, h: i32) -> bool {
        let mut arrived = false;
        while let Ok((key, image)) = self.results.try_recv() {
            arrived = true;
            match image {
                Some(img) => {
                    if self.thumbs.len() >= MAX_THUMBS {
                        // the ones that scrolled far away go; a later look finds them in the disk cache
                        self.thumbs.clear();
                        self.requested.clear();
                    }
                    self.thumbs.insert(key, img);
                }
                None => {
                    self.failed.insert(key);
                }
            }
        }
        let g = self.geo(ui, w, h);
        let step = g.cell + g.gap;
        let first_row = ((self.scroll / step).floor() - AHEAD_ROWS).max(0.0) as usize;
        let last_row = ((self.scroll + g.grid.3) / step).ceil() as usize + AHEAD_ROWS as usize;
        let from = (first_row * g.cols).min(self.visible.len());
        let to = ((last_row + 1) * g.cols).min(self.visible.len());
        for slot in from..to {
            let e = &self.pick.entries[self.visible[slot]];
            if self.thumbs.contains_key(&e.key) || self.failed.contains(&e.key) || !self.requested.insert(e.key.clone()) {
                continue;
            }
            let _ = self.jobs.send(Job { key: e.key.clone(), path: e.path.clone() });
        }
        arrived
    }

    pub fn draw(&self, s: &mut Surface, ui: &Ui, w: i32, h: i32) {
        let g = self.geo(ui, w, h);
        let (x, y, cw, ch) = g.card;
        s.fill_rrect(0.0, 0.0, w as f32, h as f32, 0.0, rgba(0, 0, 0, 110));
        s.fill_rrect(x, y, cw, ch, ui.pt(RADIUS_PT), CARD_BG.now());
        s.stroke_rrect(x, y, cw, ch, ui.pt(RADIUS_PT), 1.0, SEPARATOR);

        // the header: the title, and who receives what
        let text_w = cw - 2.0 * g.pad;
        put(s, ui, "Choose Photos", MAC_TITLE3_PT, Weight::Semibold, LABEL.now(), (x + g.pad, y + ui.pt(12.0), text_w, ui.pt(22.0)), false);
        let who = ui.fit(&format!("\u{201c}{}\u{201d} receives only the photos you choose.", self.pick.app_name), MAC_FOOTNOTE_PT, text_w);
        put(s, ui, &who, MAC_FOOTNOTE_PT, Weight::Regular, LABEL_2.now(), (x + g.pad, y + ui.pt(34.0), text_w, ui.pt(16.0)), false);

        // the tabs
        for (i, tab) in self.pick.tabs().into_iter().enumerate() {
            let r = self.tab_rect(ui, &g, i);
            let on = self.pick.tab() == tab;
            s.fill_rrect(r.0, r.1, r.2, r.3, r.3 / 2.0, if on { TINT } else { FIELD_BG.now() });
            let label = tab.map_or("All", Filter::label);
            put(s, ui, label, MAC_BODY_PT, Weight::Semibold, if on { WHITE } else { LABEL.now() }, r, true);
        }

        // the grid, clipped to its area
        if self.visible.is_empty() {
            let r = (g.grid.0, g.grid.1, g.grid.2, g.grid.3);
            put(s, ui, "No Photos", MAC_TITLE3_PT, Weight::Semibold, LABEL_2.now(), (r.0, r.1 + r.3 / 2.0 - ui.pt(20.0), r.2, ui.pt(20.0)), true);
            put(s, ui, "Pictures in the Pictures folder appear here.", MAC_BODY_PT, Weight::Regular, LABEL_2.now(), (r.0, r.1 + r.3 / 2.0, r.2, ui.pt(20.0)), true);
        } else {
            let mut clip = s.rows(g.grid.1.max(0.0) as usize, (g.grid.1 + g.grid.3).max(0.0) as usize);
            self.draw_cells(&mut clip, ui, &g);
        }

        // the footer: Cancel, the count and Add
        s.fill_rrect(x, g.footer.1, cw, 1.0, 0.0, SEPARATOR);
        put(s, ui, "Cancel", MAC_BODY_PT, Weight::Regular, LINK, self.cancel_rect(ui, &g), false);
        let add = self.add_rect(ui, &g);
        let ready = !self.pick.selected().is_empty();
        s.fill_rrect(add.0, add.1, add.2, add.3, add.3 / 2.0, if ready { TINT } else { FIELD_BG.now() });
        put(s, ui, "Add", MAC_BODY_PT, Weight::Semibold, if ready { WHITE } else { GRAY_DIM }, add, true);
        let mid_x = g.card.0 + g.pad + ui.pt(CANCEL_W_PT);
        let mid_w = add.0 - mid_x - ui.pt(TAB_GAP_PT);
        let summary = ui.fit(&self.pick.summary(), MAC_BODY_PT, mid_w.max(0.0));
        put(s, ui, &summary, MAC_BODY_PT, Weight::Regular, LABEL_2.now(), (mid_x, g.footer.1, mid_w.max(0.0), g.footer.3), true);
    }

    fn draw_cells(&self, s: &mut Surface, ui: &Ui, g: &Geo) {
        let cell_px = g.cell as usize;
        let step = g.cell + g.gap;
        let first = (((self.scroll / step).floor().max(0.0)) as usize * g.cols).min(self.visible.len());
        let last = ((((self.scroll + g.grid.3) / step).ceil() as usize + 1) * g.cols).min(self.visible.len());
        let mut scaled = self.scaled.borrow_mut();
        if scaled.len() > MAX_THUMBS {
            scaled.clear();
        }
        let radius = ui.pt(CELL_RADIUS_PT);
        for slot in first..last {
            let e = &self.pick.entries[self.visible[slot]];
            let (cx, cy) = self.cell_origin(g, slot);
            s.fill_rrect(cx, cy, g.cell, g.cell, radius, FIELD_BG.now());
            if let Some(raw) = self.thumbs.get(&e.key) {
                if scaled.get(&e.key).is_none_or(|(size, _)| *size != cell_px) {
                    scaled.insert(e.key.clone(), (cell_px, crate::imgdec::cover(raw, cell_px, cell_px)));
                }
                if let Some((_, img)) = scaled.get(&e.key) {
                    s.blit(img, cx as i32, cy as i32, 255, 255);
                }
            }
            // movies and Live Photos carry a small label
            let tag = if e.kind.is_video() { Some("Video") } else if e.live { Some("Live") } else { None };
            if let Some(tag) = tag {
                let label = ui.text(tag, MAC_FOOTNOTE_PT, Weight::Semibold, WHITE);
                let (px, py) = (ui.pt(5.0), ui.pt(2.0));
                let (bw, bh) = (label.w as f32 + 2.0 * px, label.h as f32 + 2.0 * py);
                let (bx, by) = (cx + ui.pt(4.0), cy + g.cell - ui.pt(4.0) - bh);
                s.fill_rrect(bx, by, bw, bh, bh / 2.0, rgba(0, 0, 0, 140));
                s.blit_colored(&label, (bx + px) as i32, (by + py) as i32, WHITE, 255);
            }
            if let Some(order) = self.pick.order_of(&e.id) {
                let ring = ui.pt(RING_PT);
                s.stroke_rrect(cx, cy, g.cell, g.cell, radius, ring, TINT);
                let dot = ui.pt(DOT_PT);
                let (dx, dy) = (cx + g.cell - dot - ui.pt(5.0), cy + ui.pt(5.0));
                s.fill_rrect(dx, dy, dot, dot, dot / 2.0, TINT);
                if self.pick.limit != 1 {
                    put(s, ui, &order.to_string(), MAC_FOOTNOTE_PT, Weight::Semibold, WHITE, (dx, dy, dot, dot), true);
                }
            }
        }
    }
}

/// Draws `text` vertically centred in `r`, centred or at the left edge.
fn put(s: &mut Surface, ui: &Ui, text: &str, pt: f32, weight: Weight, color: u32, r: Rect, centred: bool) {
    let img = ui.text(text, pt, weight, WHITE);
    let x = if centred { r.0 + (r.2 - img.w as f32) / 2.0 } else { r.0 };
    let y = r.1 + (r.3 - img.h as f32) / 2.0;
    s.blit_colored(&img, x as i32, y as i32, color, 255);
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::photo_pick::Entry;
    use bifrost_ui::photo_media::Kind;
    use std::time::{Duration, Instant};

    const W: i32 = 1200;
    const H: i32 = 768;

    fn entry(n: usize, kind: Kind) -> Entry {
        Entry { id: format!("PH-{n:03}"), path: PathBuf::from(format!("/nonexistent/{n}.png")), key: format!("key-{n:03}"), kind, live: false, mtime: n as i64 }
    }

    fn sheet(app: &str, filters: Vec<Filter>, limit: usize, count: usize) -> PhotoPickSheet {
        let entries = (0..count).map(|n| entry(n, if n % 4 == 3 { Kind::Video } else { Kind::Image })).collect();
        let dir = std::env::temp_dir().join(format!("bifrost-photo-pick-sheet-{}", std::process::id()));
        PhotoPickSheet::new(Pick::new(app, "Notes", filters, limit, entries), dir)
    }

    fn centre(r: Rect) -> (f32, f32) {
        (r.0 + r.2 / 2.0, r.1 + r.3 / 2.0)
    }

    fn tap(sh: &mut PhotoPickSheet, ui: &Ui, (x, y): (f32, f32)) -> Outcome {
        sh.event(ui, W, H, Event::Down(x, y));
        sh.event(ui, W, H, Event::Up(x, y))
    }

    fn cell_centre(sh: &PhotoPickSheet, ui: &Ui, slot: usize) -> (f32, f32) {
        cell_centre_on(sh, ui, W, H, slot)
    }

    /// The same on a screen of another size (the harness shell is 1200x1920).
    fn cell_centre_on(sh: &PhotoPickSheet, ui: &Ui, w: i32, h: i32, slot: usize) -> (f32, f32) {
        let g = sh.geo(ui, w, h);
        let (x, y) = sh.cell_origin(&g, slot);
        (x + g.cell / 2.0, y + g.cell / 2.0)
    }

    #[test]
    fn a_tap_chooses_a_picture_and_the_limit_holds() {
        let ui = crate::shell::tests::ui();
        let mut sh = sheet("is.example.sheet-limit", vec![], 2, 40);
        // slot 0 is the newest picture (the library is listed newest first)
        let newest = sh.pick().entries[sh.visible[0]].id.clone();
        assert_eq!(newest, "PH-039");
        assert_eq!({ let p = cell_centre(&sh, &ui, 0); tap(&mut sh, &ui, p) }, Outcome::Redraw);
        assert_eq!({ let p = cell_centre(&sh, &ui, 1); tap(&mut sh, &ui, p) }, Outcome::Redraw);
        assert_eq!({ let p = cell_centre(&sh, &ui, 2); tap(&mut sh, &ui, p) }, Outcome::Idle, "the limit of two is reached");
        assert_eq!(sh.pick().selected().len(), 2);
        assert_eq!({ let p = cell_centre(&sh, &ui, 0); tap(&mut sh, &ui, p) }, Outcome::Redraw, "a chosen picture unchooses");
        assert_eq!(sh.pick().selected().len(), 1);
    }

    #[test]
    fn add_needs_a_choice_and_ends_the_pick_with_the_chosen_ids_only() {
        let ui = crate::shell::tests::ui();
        let app = "is.example.sheet-add";
        let mut sh = sheet(app, vec![], 3, 12);
        let add = centre(sh.add_rect(&ui, &sh.geo(&ui, W, H)));
        assert_eq!(tap(&mut sh, &ui, add), Outcome::Idle, "nothing chosen yet");
        { let p = cell_centre(&sh, &ui, 3); tap(&mut sh, &ui, p) };
        { let p = cell_centre(&sh, &ui, 0); tap(&mut sh, &ui, p) };
        assert_eq!(tap(&mut sh, &ui, add), Outcome::Add);
        let chosen = sh.pick().selected().to_vec();
        assert_eq!(chosen.len(), 2);
        sh.finish();
        assert_eq!(photo_pick::take_finished(), vec![(app.to_string(), chosen)]);
    }

    #[test]
    fn cancel_ends_the_pick_with_no_choice() {
        let ui = crate::shell::tests::ui();
        let app = "is.example.sheet-cancel";
        let mut sh = sheet(app, vec![], 1, 5);
        { let p = cell_centre(&sh, &ui, 0); tap(&mut sh, &ui, p) };
        let cancel = centre(sh.cancel_rect(&ui, &sh.geo(&ui, W, H)));
        assert_eq!(tap(&mut sh, &ui, cancel), Outcome::Cancel);
        sh.cancel();
        assert_eq!(photo_pick::take_finished(), vec![(app.to_string(), Vec::new())]);
    }

    #[test]
    fn the_filter_tabs_narrow_the_grid() {
        let ui = crate::shell::tests::ui();
        let mut sh = sheet("is.example.sheet-tabs", vec![], 0, 40);
        assert_eq!(sh.visible.len(), 40);
        let g = sh.geo(&ui, W, H);
        let videos = centre(sh.tab_rect(&ui, &g, 2));
        assert_eq!(sh.hit(&ui, W, H, videos.0, videos.1), Hit::Tab(Some(Filter::Videos)));
        assert_eq!(tap(&mut sh, &ui, videos), Outcome::Redraw);
        assert_eq!(sh.visible.len(), 10, "every fourth entry is a movie");
        assert!(sh.visible.iter().all(|&i| sh.pick().entries[i].kind.is_video()));
        // an app that asked for pictures only gets no tab row at all
        let only = sheet("is.example.sheet-images", vec![Filter::Images], 0, 40);
        assert!(only.pick().tabs().is_empty());
        assert_eq!(only.visible.len(), 30);
    }

    #[test]
    fn dragging_scrolls_the_grid_and_does_not_choose() {
        let ui = crate::shell::tests::ui();
        let mut sh = sheet("is.example.sheet-scroll", vec![], 0, 200);
        let g = sh.geo(&ui, W, H);
        assert!(sh.max_scroll(&g) > 0.0);
        let (x, y) = cell_centre(&sh, &ui, 0);
        sh.event(&ui, W, H, Event::Down(x, y + 200.0));
        assert_eq!(sh.event(&ui, W, H, Event::Move(x, y + 100.0)), Outcome::Redraw);
        assert!((sh.scroll - 100.0).abs() < 0.5, "the grid followed the finger");
        assert_eq!(sh.event(&ui, W, H, Event::Up(x, y + 100.0)), Outcome::Idle);
        assert!(sh.pick().selected().is_empty());
        sh.event(&ui, W, H, Event::Move(x, y - 100_000.0));
        sh.event(&ui, W, H, Event::Down(x, y + 200.0));
        sh.event(&ui, W, H, Event::Move(x, y - 100_000.0));
        assert!(sh.scroll <= sh.max_scroll(&g), "the scroll stops at the end");
    }

    #[test]
    fn the_layout_follows_the_screen_size() {
        let ui = crate::shell::tests::ui();
        let sh = sheet("is.example.sheet-size", vec![], 0, 10);
        let wide = sh.geo(&ui, 1920, 1080);
        let narrow = sh.geo(&ui, 480, 800);
        assert!(narrow.card.2 < wide.card.2, "a narrow screen gets a narrower card");
        assert!(narrow.card.0 >= 0.0 && narrow.card.0 + narrow.card.2 <= 480.0);
        assert!(wide.cols >= narrow.cols && narrow.cols >= 2);
        assert!(narrow.grid.0 + narrow.grid.2 <= narrow.card.0 + narrow.card.2);
    }

    #[test]
    fn thumbnails_of_unreadable_files_are_given_up_once_and_the_sheet_draws() {
        let ui = crate::shell::tests::ui();
        let mut sh = sheet("is.example.sheet-thumbs", vec![], 1, 6);
        sh.prepare(&ui, W, H);
        let start = Instant::now();
        while sh.failed.len() < 6 && start.elapsed() < Duration::from_secs(10) {
            sh.prepare(&ui, W, H);
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(sh.failed.len(), 6, "each file was tried and none is readable");
        let before = sh.requested.len();
        sh.prepare(&ui, W, H);
        assert_eq!(sh.requested.len(), before, "a failed thumbnail is not asked for again");
        let mut px = vec![0u32; (W * H) as usize];
        let mut surface = Surface { w: W as usize, h: H as usize, stride: W as usize, px: &mut px, y0: 0 };
        { let p = cell_centre(&sh, &ui, 0); tap(&mut sh, &ui, p) };
        sh.draw(&mut surface, &ui, W, H);
        assert!(px.iter().any(|p| *p != 0), "the dimmed screen and the card were drawn");
    }

    #[test]
    fn an_apps_pick_runs_through_the_shell_from_the_call_to_the_event() {
        use super::super::services::SystemService;
        let app = "is.example.sheet-shell-flow";
        let dir = std::env::temp_dir().join(format!("bifrost-photo-pick-flow-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for name in ["one.png", "two.png"] {
            std::fs::write(dir.join(name), b"not a picture").unwrap();
        }
        let library = super::super::photos::library::scan(std::slice::from_ref(&dir));
        assert_eq!(library.len(), 2);
        // the photos service call opens the pick; the shell tick puts the sheet on screen
        let reply = super::super::services::photos::answer(app, false, "picker", &bifrost_core::json::Json::Null, &library).unwrap();
        assert_eq!(reply.str_field("status").as_deref(), Some("pending"));
        let mut sh = crate::shell::tests::shell();
        assert!(sh.photo_pick_sheet.is_none());
        sh.photo_pick_tick();
        assert_eq!(sh.photo_pick_sheet.as_ref().map(|s| s.app_id().to_string()).as_deref(), Some(app));
        // the touch goes to the sheet, not to what is under it
        let (cell, add) = {
            let sheet = sh.photo_pick_sheet.as_ref().unwrap();
            let g = sheet.geo(&sh.ui, sh.w, sh.h);
            (cell_centre_on(sheet, &sh.ui, sh.w, sh.h, 0), centre(sheet.add_rect(&sh.ui, &g)))
        };
        for (x, y) in [cell, add] {
            assert!(sh.modal_event(Event::Down(x, y)).is_some());
            assert!(sh.modal_event(Event::Up(x, y)).is_some());
        }
        assert!(sh.photo_pick_sheet.is_none(), "Add closed the sheet");
        // the service tick tells the app what was chosen
        let events = super::super::services::photos::Photos::default().tick(&mut sh, 10.0);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event, super::super::services::photos::PICKER_DID_FINISH);
        assert_eq!(events[0].body.get("results").and_then(bifrost_core::json::Json::as_array).map(|r| r.len()), Some(1));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_photos_widget_picker_pins_the_chosen_picture_on_the_today_page() {
        let dir = std::env::temp_dir().join(format!("bifrost-photo-pick-widget-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("only.png"), b"not a picture").unwrap();
        let key = super::super::photos::library::Photo::from_path(dir.join("only.png")).unwrap().key();
        let mut sh = crate::shell::tests::shell();
        sh.photo_dirs = vec![dir.clone()];
        if sh.is_locked() {
            sh.act(super::super::today::Action::ChoosePhoto, 0.0);
            assert!(photo_pick::take_waiting().is_none(), "the lock screen does not open the picker");
        }
        sh.choose_widget_photo();
        sh.photo_pick_tick();
        let sheet = sh.photo_pick_sheet.as_ref().expect("the sheet is up");
        assert_eq!((sheet.app_id(), sheet.pick().limit, sheet.pick().allowed.clone()), (photo_pick::WIDGET_APP, 1, vec![Filter::Images]));
        let (cell, add) = {
            let g = sheet.geo(&sh.ui, sh.w, sh.h);
            (cell_centre_on(sheet, &sh.ui, sh.w, sh.h, 0), centre(sheet.add_rect(&sh.ui, &g)))
        };
        for (x, y) in [cell, add] {
            sh.modal_event(Event::Down(x, y));
            sh.modal_event(Event::Up(x, y));
        }
        assert!(sh.photo_pick_sheet.is_none());
        assert!(sh.today.pinned_photo().is_none(), "the shell takes the choice on its next tick");
        sh.photo_pick_tick();
        assert_eq!(sh.today.pinned_photo(), Some(key.as_str()));
        assert!(photo_pick::take_finished().is_empty(), "no app hears of the widget pick");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_empty_library_shows_a_notice_and_add_stays_off() {
        let ui = crate::shell::tests::ui();
        let mut sh = sheet("is.example.sheet-empty", vec![], 1, 0);
        assert!(sh.visible.is_empty());
        let add = centre(sh.add_rect(&ui, &sh.geo(&ui, W, H)));
        assert_eq!(tap(&mut sh, &ui, add), Outcome::Idle);
        let mut px = vec![0u32; (W * H) as usize];
        let mut surface = Surface { w: W as usize, h: H as usize, stride: W as usize, px: &mut px, y0: 0 };
        sh.draw(&mut surface, &ui, W, H);
    }
}
