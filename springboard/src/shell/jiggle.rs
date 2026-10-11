//! Home-screen edit mode ("jiggle"): a long press on an icon makes the
//! icons wiggle, puts the bifrost minus badge on each app, a Done button top
//! right and, on a badge tap, the "Remove App?" sheet (Remove from Home
//! Screen / Cancel). A removed app leaves the home pages but stays in the App
//! Library and Search; the removed list persists in prefs (`home.removed`).

use super::prefs::Prefs;
use super::ui::{Ui, Weight, BLACK, HAIRLINE, MINUS_FILL, SECONDARY, SYSTEM_BLUE, WHITE};
use super::{accessibility, Effect, Event, Screen, Shell, apps, IN_DOCK};
use crate::gfx::{rgba, Image, Surface};
use crate::layout::{Layout, PER_PAGE};
use crate::scene::{self, Scene, Tile};

const KEY: &str = "home.removed";
/// Peak tilt in degrees and the wiggle period in ms.
const TILT_DEG: f32 = 1.6;
const PERIOD_MS: f64 = 260.0;
const TAP_SLOP: f32 = 24.0;
const BADGE_PT: f32 = 22.0;
const SHEET_W_PT: f32 = 270.0;
const ROW_PT: f32 = 44.0;
const HEAD_PT: f32 = 84.0;
const PAD: usize = 4;

/// The edit mode's state: the animation clock, the pending touch and the
/// grid slot whose remove sheet is up.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct Jiggle {
    t: f64,
    sheet: Option<usize>,
    down: Option<(f32, f32)>,
}

/// `1,4,9`: the grid app indices taken off the home pages.
pub(super) fn load_removed(prefs: &Prefs) -> Vec<usize> {
    prefs.get(KEY).unwrap_or("").split(',').filter_map(|p| p.trim().parse().ok()).collect()
}

fn save_removed(prefs: &mut Prefs, v: &[usize]) {
    let s = v.iter().map(|p| p.to_string()).collect::<Vec<_>>().join(",");
    if let Err(e) = prefs.set(KEY, &s) {
        eprintln!("springboard: {KEY}: {e}");
    }
}

/// Where the minus badge of the tile at `(x, y)` is centred, and its radius.
fn badge(l: &Layout, x: i32, y: i32) -> (f32, f32, f32) {
    let r = l.pt(BADGE_PT) / 2.0;
    (x as f32 + l.pt(2.0), y as f32 + l.pt(2.0), r)
}

/// The Done button's tap area: (x, y, w, h).
fn done_rect(ui: &Ui, l: &Layout) -> (f32, f32, f32, f32) {
    let w = ui.measure("Done", 17.0, Weight::Semibold) + ui.pt(32.0);
    (l.w as f32 - w, l.status_h as f32, w, ui.pt(ROW_PT))
}

/// The sheet card and its two row rects: (card, remove row, cancel row).
fn sheet_rects(ui: &Ui, l: &Layout) -> ([f32; 4], [f32; 4], [f32; 4]) {
    let (w, row, head) = (ui.pt(SHEET_W_PT), ui.pt(ROW_PT), ui.pt(HEAD_PT));
    let h = head + 2.0 * row;
    let (x, y) = ((l.w as f32 - w) / 2.0, (l.h as f32 - h) / 2.0);
    ([x, y, w, h], [x, y + head, w, row], [x, y + head + row, w, row])
}

fn inside(r: [f32; 4], x: f32, y: f32) -> bool {
    x >= r[0] && y >= r[1] && x < r[0] + r[2] && y < r[1] + r[3]
}

/// `src` turned by `ang` radians about its centre (bilinear, premultiplied).
fn rotated(src: &Image, ang: f32) -> Image {
    let (sn, cs) = ang.sin_cos();
    let (w, h) = (src.w as f32, src.h as f32);
    let (ow, oh) = ((w * cs.abs() + h * sn.abs()).ceil() as usize + 2, (w * sn.abs() + h * cs.abs()).ceil() as usize + 2);
    let mut out = Image::new(ow, oh);
    let ch = |p: u32, k: u32| ((p >> k) & 255) as f32;
    let at = |x: i32, y: i32| if x < 0 || y < 0 || x >= src.w as i32 || y >= src.h as i32 { 0 } else { src.px[y as usize * src.w + x as usize] };
    for oy in 0..oh {
        for ox in 0..ow {
            let (dx, dy) = (ox as f32 + 0.5 - ow as f32 / 2.0, oy as f32 + 0.5 - oh as f32 / 2.0);
            let (sx, sy) = (dx * cs + dy * sn + w / 2.0 - 0.5, -dx * sn + dy * cs + h / 2.0 - 0.5);
            let (x0, y0) = (sx.floor(), sy.floor());
            let (fx, fy) = (sx - x0, sy - y0);
            let ps = [at(x0 as i32, y0 as i32), at(x0 as i32 + 1, y0 as i32), at(x0 as i32, y0 as i32 + 1), at(x0 as i32 + 1, y0 as i32 + 1)];
            let wt = [(1.0 - fx) * (1.0 - fy), fx * (1.0 - fy), (1.0 - fx) * fy, fx * fy];
            let mut px = 0u32;
            for k in [0u32, 8, 16, 24] {
                let v: f32 = ps.iter().zip(wt).map(|(&p, w)| ch(p, k) * w).sum();
                px |= (v.round().clamp(0.0, 255.0) as u32) << k;
            }
            out.px[oy * ow + ox] = px;
        }
    }
    out
}

/// A tile (icon over label) on a padded transparent canvas.
fn compose(l: &Layout, t: &Tile) -> Image {
    let inner_w = (l.icon as usize).max(t.label.w);
    let inner_h = l.icon as usize + l.label_gap as usize + t.label.h;
    let mut img = Image::new(inner_w + 2 * PAD, inner_h + 2 * PAD);
    let mut s = crate::shell::widgets::surface(&mut img);
    s.blit(&t.icon, (PAD + (inner_w - l.icon as usize) / 2) as i32, PAD as i32, 255, 255);
    s.blit(&t.label, (PAD + (inner_w - t.label.w) / 2) as i32, PAD as i32 + l.icon + l.label_gap, 255, 255);
    img
}

fn draw_minus(s: &mut Surface, l: &Layout, x: i32, y: i32) {
    let (cx, cy, r) = badge(l, x, y);
    s.fill_rrect(cx - r, cy - r, 2.0 * r, 2.0 * r, r, MINUS_FILL);
    let (bw, bh) = (r * 1.1, l.pt(2.0).max(2.0));
    s.fill_rrect(cx - bw / 2.0, cy - bh / 2.0, bw, bh, bh / 2.0, BLACK);
}

impl Shell {
    /// True once a long press on `(x, y)` lands on an app or folder icon.
    pub(super) fn hold_on_icon(&self, x: f32, y: f32) -> bool {
        scene::hit(&self.layout, self.pager.offset(), self.dock.len(), self.home_grid.len(), x, y).is_some_and(|slot| slot >= self.dock.len() && self.home.slots.get(slot - self.dock.len()).is_some_and(Option::is_some))
    }

    pub(super) fn enter_jiggle(&mut self, t: f64) {
        self.jiggle = Some(Jiggle { t, ..Jiggle::default() });
        self.drawn = super::Drawn::Nothing;
        self.dirty = true;
    }

    /// Advances the wiggle clock.
    pub(super) fn jiggle_tick(&mut self, t: f64) {
        if let Some(j) = self.jiggle.as_mut() {
            j.t = t;
            self.dirty = true;
        }
    }

    /// The grid slot whose minus badge `(x, y)` hits.
    fn badge_at(&self, x: f32, y: f32) -> Option<usize> {
        let l = &self.layout;
        let off = self.pager.offset().round() as i32;
        let reach = l.pt(BADGE_PT);
        let total = self.grid.len();
        (0..self.home_grid.len()).find(|&i| {
            let dx = (i / PER_PAGE) as i32 * l.w - off;
            let c = l.cells[i % PER_PAGE];
            let (cx, cy, _) = badge(l, c.x + dx, c.y);
            self.home.slots.get(i).copied().flatten().is_some_and(|e| e < total) && (x - cx).abs() <= reach && (y - cy).abs() <= reach
        })
    }

    /// Routes a touch while the edit mode is on.
    pub(super) fn jiggle_event(&mut self, ev: Event, t: f64) -> Option<Vec<Effect>> {
        let mut j = self.jiggle.filter(|_| self.screen == Screen::Home)?;
        if !matches!(ev, Event::Down(..) | Event::Move(..) | Event::Up(..)) {
            return None;
        }
        self.dirty = true;
        match ev {
            Event::Down(x, y) => {
                j.down = Some((x, y));
                if j.sheet.is_none() {
                    self.pager.down(x, y, t, None);
                }
            }
            Event::Move(x, y) => {
                if j.down.is_some() && j.sheet.is_none() {
                    self.pager.moved(x, y, t);
                }
            }
            Event::Up(x, y) => {
                if let Some((dx, dy)) = j.down.take() {
                    let tap = (x - dx).abs() <= TAP_SLOP && (y - dy).abs() <= TAP_SLOP;
                    if let Some(slot) = j.sheet {
                        let (_, remove, _) = sheet_rects(&self.ui, &self.layout);
                        j.sheet = None;
                        if tap && inside(remove, x, y) {
                            self.remove_from_home(slot);
                        }
                    } else {
                        self.pager.up(x, y, t, None);
                        if tap {
                            let (rx, ry, rw, rh) = done_rect(&self.ui, &self.layout);
                            if inside([rx, ry, rw, rh], x, y) {
                                self.jiggle = None;
                                self.drawn = super::Drawn::Nothing;
                                return Some(vec![]);
                            }
                            j.sheet = self.badge_at(x, y);
                        }
                    }
                }
            }
            _ => {}
        }
        self.jiggle = Some(j);
        Some(vec![])
    }

    /// Takes the app in grid `slot` off the home pages (it stays in the App Library).
    fn remove_from_home(&mut self, slot: usize) {
        let Some(e) = self.home.slots.get(slot).copied().flatten().filter(|&e| e < self.grid.len()) else { return };
        if !self.removed.contains(&e) {
            self.removed.push(e);
            save_removed(&mut self.prefs, &self.removed);
        }
        self.rebuild_home();
    }

    /// The home screen in edit mode: wiggling icons with minus badges, the
    /// Done button and the remove sheet.
    pub(super) fn draw_jiggle(&self, s: &mut Surface) {
        let Some(j) = &self.jiggle else { return };
        let l = &self.layout;
        Scene { grid: &[], ..self.scene() }.draw(s);
        let off = self.pager.offset().round() as i32;
        let total = self.grid.len();
        let still = accessibility::reduce_motion();
        for (i, tile) in self.home_grid.iter().enumerate() {
            let Some(e) = self.home.slots.get(i).copied().flatten() else { continue };
            let dx = (i / PER_PAGE) as i32 * l.w - off;
            if dx <= -l.w || dx >= l.w {
                continue;
            }
            let c = l.cells[i % PER_PAGE];
            let img = compose(l, tile);
            let (bx, by) = (c.x + dx - (img.w as i32 - l.icon) / 2, c.y - PAD as i32);
            let phase = (j.t / PERIOD_MS) * std::f64::consts::TAU + i as f64 * 1.9;
            let ang = if still { 0.0 } else { phase.sin() as f32 * TILT_DEG.to_radians() };
            if ang == 0.0 {
                s.blit(&img, bx, by, 255, 255);
            } else {
                let r = rotated(&img, ang);
                s.blit(&r, bx - (r.w as i32 - img.w as i32) / 2, by - (r.h as i32 - img.h as i32) / 2, 255, 255);
            }
            if e < total {
                draw_minus(s, l, c.x + dx, c.y);
            }
        }
        let done = self.ui.text("Done", 17.0, Weight::Semibold, WHITE);
        let (rx, ry, rw, rh) = done_rect(&self.ui, l);
        s.blit(&done, (rx + (rw - done.w as f32) / 2.0) as i32, (ry + (rh - done.h as f32) / 2.0) as i32, 255, 255);
        if let Some(slot) = j.sheet {
            self.draw_sheet(s, slot);
        }
    }

    fn draw_sheet(&self, s: &mut Surface, slot: usize) {
        let (ui, l) = (&self.ui, &self.layout);
        s.fill_rrect(0.0, 0.0, l.w as f32, l.h as f32, 0.0, rgba(0, 0, 0, 110));
        let ([x, y, w, h], remove, cancel) = sheet_rects(ui, l);
        s.fill_rrect(x, y, w, h, ui.pt(14.0), rgba(44, 44, 46, 240));
        let name = self.home.slots.get(slot).copied().flatten().and_then(|e| apps().get(e + IN_DOCK)).map_or("", |a| a.name);
        let title = ui.text("Remove App?", 17.0, Weight::Semibold, WHITE);
        let msg = ui.text(&ui.fit(&format!("\"{name}\" stays in your App Library."), 13.0, w - ui.pt(32.0)), 13.0, Weight::Regular, SECONDARY);
        s.blit(&title, (x + (w - title.w as f32) / 2.0) as i32, (y + ui.pt(18.0)) as i32, 255, 255);
        s.blit(&msg, (x + (w - msg.w as f32) / 2.0) as i32, (y + ui.pt(46.0)) as i32, 255, 255);
        for (r, label) in [(remove, ui.text("Remove from Home Screen", 17.0, Weight::Regular, SYSTEM_BLUE)), (cancel, ui.text("Cancel", 17.0, Weight::Semibold, SYSTEM_BLUE))] {
            s.fill_rect(r[0] as usize, r[1] as usize, r[2] as usize, ui.pt(1.0).max(1.0) as usize, HAIRLINE);
            s.blit(&label, (r[0] + (r[2] - label.w as f32) / 2.0) as i32, (r[1] + (r[3] - label.h as f32) / 2.0) as i32, 255, 255);
        }
    }
}
