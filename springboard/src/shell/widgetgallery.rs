//! The Add Widget gallery, a sheet opened by a long press on a home page.
//! Each widget offers a Small (2 x 2) and a Medium (4 x 2) button that place
//! it on the current page; the widgets already there can be resized or
//! removed. Previews are the Today View's rendered tiles.

use super::homeconf::{kind_name, Size, GALLERY};
use super::ui::{Ui, Weight, SECONDARY, WHITE};
use super::widgets::{battery, Kind};
use crate::deildu::scroll::Scroller;
use crate::gfx::{rgba, Image, Surface};
use crate::layout::{Layout, Rect};

const SLOP: f32 = 12.0;
const RADIUS: f32 = 6.0;
const MARGIN_PT: f32 = 24.0;
const HEADER_PT: f32 = 56.0;
const HEAD_ROW_PT: f32 = 36.0;
const ADD_ROW_PT: f32 = 96.0;
const HERE_ROW_PT: f32 = 64.0;
const PREVIEW_PT: f32 = 72.0;
const BTN_W_PT: f32 = 104.0;
const BTN_H_PT: f32 = 40.0;
const BTN_GAP_PT: f32 = 12.0;

/// What a tap on the sheet asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Out {
    Add(Kind, Size),
    /// swap the size of widget `i` (an index into the home widgets)
    Resize(usize),
    Remove(usize),
    Close,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Item {
    /// 0: the add list, 1: the widgets already on the page
    Head(usize),
    Add(usize),
    Here(usize),
}

#[derive(Clone, Copy, Debug)]
struct Touch {
    x0: f32,
    y0: f32,
    target: Option<Out>,
    dragging: bool,
}

pub struct Gallery {
    title: Image,
    done: Image,
    heads: [Image; 2],
    small: Image,
    medium: Image,
    resize: Image,
    remove: Image,
    /// per gallery kind: preview, name
    previews: Vec<(Kind, Image, Image)>,
    /// per widget on this page: home widget index, kind, label
    here: Vec<(usize, Kind, Image)>,
    scroll: Scroller,
    touch: Option<Touch>,
    gen: u32,
}

fn button(s: &mut Surface, r: Rect, label: &Image) {
    s.fill_rrect(r.x as f32, r.y as f32, r.w as f32, r.h as f32, RADIUS, rgba(255, 255, 255, 30));
    s.blit(label, r.x + (r.w - label.w as i32) / 2, r.y + (r.h - label.h as i32) / 2, 255, 255);
}

fn px(l: &Layout, pt: f32) -> i32 {
    l.pt(pt).round() as i32
}

impl Gallery {
    /// `tiles` are the rendered Today tiles per kind; kinds without one show no preview.
    pub fn open(ui: &Ui, tiles: &[(Kind, &Image)], here: &[(usize, Kind, Size)]) -> Self {
        let ph = ui.pt(PREVIEW_PT) as usize;
        let previews = GALLERY
            .iter()
            .map(|&k| {
                let preview = tiles.iter().find(|(t, _)| *t == k).map_or_else(|| Image::new(ph, ph), |(_, img)| img.resized((img.w * ph / img.h.max(1)).max(1), ph));
                (k, preview, ui.text(kind_name(k), 17.0, Weight::Semibold, WHITE))
            })
            .collect();
        let mut g = Gallery {
            title: ui.text("Add Widget", 20.0, Weight::Semibold, WHITE),
            done: ui.text("Done", 17.0, Weight::Semibold, WHITE),
            heads: [ui.text("Widgets", 13.0, Weight::Semibold, SECONDARY), ui.text("On This Page", 13.0, Weight::Semibold, SECONDARY)],
            small: ui.text(Size::Small.label(), 15.0, Weight::Medium, WHITE),
            medium: ui.text(Size::Medium.label(), 15.0, Weight::Medium, WHITE),
            resize: ui.text("Resize", 15.0, Weight::Medium, WHITE),
            remove: ui.text("Remove", 15.0, Weight::Medium, battery::RED),
            previews,
            here: Vec::new(),
            scroll: Scroller::new(),
            touch: None,
            gen: 1,
        };
        g.set_here(ui, here);
        g
    }

    /// Lists the widgets on the page: (home widget index, kind, size).
    pub fn set_here(&mut self, ui: &Ui, here: &[(usize, Kind, Size)]) {
        self.here = here.iter().map(|&(i, k, s)| (i, k, ui.text(&format!("{} ({})", kind_name(k), s.label()), 17.0, Weight::Regular, WHITE))).collect();
        self.gen = self.gen.wrapping_add(1);
    }

    pub fn gen(&self) -> u32 {
        self.gen
    }

    pub fn animating(&self) -> bool {
        self.scroll.animating()
    }

    pub fn tick(&mut self, t: f64) -> bool {
        let moving = self.scroll.tick(t);
        if moving {
            self.gen = self.gen.wrapping_add(1);
        }
        moving
    }

    // -- geometry --

    fn sheet_top(l: &Layout) -> i32 {
        l.h * 22 / 100
    }

    fn viewport(l: &Layout) -> (i32, i32) {
        (Self::sheet_top(l) + px(l, HEADER_PT), l.h - px(l, 20.0))
    }

    fn done_rect(l: &Layout) -> Rect {
        let (w, h) = (px(l, 90.0), px(l, HEADER_PT));
        Rect { x: l.w - px(l, MARGIN_PT) - w, y: Self::sheet_top(l), w, h }
    }

    /// The rows as (y from the viewport top, height, item), and the content height.
    fn items(&self, l: &Layout) -> (Vec<(i32, i32, Item)>, i32) {
        let mut out = vec![(0, px(l, HEAD_ROW_PT), Item::Head(0))];
        let mut y = px(l, HEAD_ROW_PT);
        for k in 0..self.previews.len() {
            out.push((y, px(l, ADD_ROW_PT), Item::Add(k)));
            y += px(l, ADD_ROW_PT);
        }
        if !self.here.is_empty() {
            out.push((y, px(l, HEAD_ROW_PT), Item::Head(1)));
            y += px(l, HEAD_ROW_PT);
            for j in 0..self.here.len() {
                out.push((y, px(l, HERE_ROW_PT), Item::Here(j)));
                y += px(l, HERE_ROW_PT);
            }
        }
        (out, y)
    }

    /// The two buttons of a row spanning (y, h) in screen px: left, right.
    fn buttons(l: &Layout, y: i32, h: i32) -> (Rect, Rect) {
        let (bw, bh, gap) = (px(l, BTN_W_PT), px(l, BTN_H_PT), px(l, BTN_GAP_PT));
        let right = Rect { x: l.w - px(l, MARGIN_PT) - bw, y: y + (h - bh) / 2, w: bw, h: bh };
        (Rect { x: right.x - gap - bw, ..right }, right)
    }

    fn fit_scroll(&mut self, l: &Layout) {
        let (top, bottom) = Self::viewport(l);
        let (_, content) = self.items(l);
        self.scroll.set_max((content - (bottom - top)).max(0) as f32);
    }

    fn hit(&self, l: &Layout, x: f32, y: f32) -> Option<Out> {
        if y < Self::sheet_top(l) as f32 || Self::done_rect(l).contains(x, y) {
            return Some(Out::Close);
        }
        let (top, bottom) = Self::viewport(l);
        if y < top as f32 || y >= bottom as f32 {
            return None;
        }
        let off = self.scroll.offset();
        for (ry, rh, item) in self.items(l).0 {
            let sy = top + ry - off.round() as i32;
            let (left, right) = Self::buttons(l, sy, rh);
            match item {
                Item::Add(k) if left.contains(x, y) => return Some(Out::Add(self.previews[k].0, Size::Small)),
                Item::Add(k) if right.contains(x, y) => return Some(Out::Add(self.previews[k].0, Size::Medium)),
                Item::Here(j) if left.contains(x, y) => return Some(Out::Resize(self.here[j].0)),
                Item::Here(j) if right.contains(x, y) => return Some(Out::Remove(self.here[j].0)),
                _ => {}
            }
        }
        None
    }

    // -- touch --

    pub fn down(&mut self, l: &Layout, x: f32, y: f32, t: f64) {
        self.fit_scroll(l);
        self.scroll.down(y, t);
        self.touch = Some(Touch { x0: x, y0: y, target: self.hit(l, x, y), dragging: false });
    }

    pub fn moved(&mut self, x: f32, y: f32, t: f64) {
        let Some(tc) = self.touch.as_mut() else { return };
        if !tc.dragging && ((x - tc.x0).abs() > SLOP || (y - tc.y0).abs() > SLOP) {
            tc.dragging = true;
        }
        if tc.dragging {
            self.scroll.moved(y, t);
            self.gen = self.gen.wrapping_add(1);
        }
    }

    pub fn up(&mut self, l: &Layout, x: f32, y: f32, t: f64) -> Option<Out> {
        let tc = self.touch.take()?;
        self.gen = self.gen.wrapping_add(1);
        if tc.dragging {
            self.scroll.up(y, t);
            return None;
        }
        self.scroll.up(tc.y0, t);
        tc.target.filter(|&tg| self.hit(l, x, y) == Some(tg))
    }

    // -- drawing --

    pub fn draw(&self, s: &mut Surface, l: &Layout) {
        s.fill_rrect(0.0, 0.0, l.w as f32, l.h as f32, 0.0, rgba(0, 0, 0, 120));
        let top = Self::sheet_top(l);
        s.fill_rrect(0.0, top as f32, l.w as f32, (l.h - top + 20) as f32, RADIUS, rgba(28, 28, 30, 250));
        let head = px(l, HEADER_PT);
        s.blit(&self.title, px(l, MARGIN_PT), top + (head - self.title.h as i32) / 2, 255, 255);
        let d = Self::done_rect(l);
        s.blit(&self.done, d.x + d.w - self.done.w as i32, d.y + (d.h - self.done.h as i32) / 2, 255, 255);
        let (vt, vb) = Self::viewport(l);
        if vb <= vt {
            return;
        }
        let mut band = s.rows(vt as usize, vb as usize);
        let scroll = self.scroll.offset().round() as i32;
        let margin = px(l, MARGIN_PT);
        for (ry, rh, item) in self.items(l).0 {
            let y = vt + ry - scroll;
            if y + rh < vt || y > vb {
                continue;
            }
            let center = |img: &Image| y + (rh - img.h as i32) / 2;
            let (left, right) = Self::buttons(l, y, rh);
            match item {
                Item::Head(k) => {
                    let img = &self.heads[k];
                    band.blit(img, margin, y + rh - img.h as i32 - px(l, 6.0), 255, 255);
                }
                Item::Add(k) => {
                    let (_, preview, name) = &self.previews[k];
                    band.blit(preview, margin, center(preview), 255, 255);
                    band.blit(name, margin + preview.w as i32 + px(l, 14.0), center(name), 255, 255);
                    button(&mut band, left, &self.small);
                    button(&mut band, right, &self.medium);
                }
                Item::Here(j) => {
                    let label = &self.here[j].2;
                    band.blit(label, margin, center(label), 255, 255);
                    button(&mut band, left, &self.resize);
                    button(&mut band, right, &self.remove);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shell::tests::ui;

    fn gallery(here: &[(usize, Kind, Size)]) -> (Gallery, Layout) {
        (Gallery::open(&ui(), &[], here), Layout::new(1200, 1920, 3))
    }

    fn tap(g: &mut Gallery, l: &Layout, r: Rect) -> Option<Out> {
        let (x, y) = ((r.x + r.w / 2) as f32, (r.y + r.h / 2) as f32);
        g.down(l, x, y, 0.0);
        g.up(l, x, y, 30.0)
    }

    #[test]
    fn the_size_buttons_add_the_widget_in_that_size() {
        let (mut g, l) = gallery(&[]);
        let (top, _) = Gallery::viewport(&l);
        let (y, h, _) = g.items(&l).0[1];
        let (small, medium) = Gallery::buttons(&l, top + y, h);
        assert_eq!(tap(&mut g, &l, small), Some(Out::Add(GALLERY[0], Size::Small)));
        assert_eq!(tap(&mut g, &l, medium), Some(Out::Add(GALLERY[0], Size::Medium)));
    }

    #[test]
    fn placed_widgets_can_be_resized_or_removed() {
        let (mut g, l) = gallery(&[(4, Kind::Battery, Size::Small)]);
        let (top, _) = Gallery::viewport(&l);
        let (y, h, item) = *g.items(&l).0.last().unwrap();
        assert_eq!(item, Item::Here(0));
        let (resize, remove) = Gallery::buttons(&l, top + y, h);
        assert_eq!(tap(&mut g, &l, resize), Some(Out::Resize(4)));
        assert_eq!(tap(&mut g, &l, remove), Some(Out::Remove(4)));
    }

    #[test]
    fn tapping_the_dimmed_area_or_done_closes() {
        let (mut g, l) = gallery(&[]);
        assert_eq!(tap(&mut g, &l, Rect { x: 10, y: 10, w: 10, h: 10 }), Some(Out::Close));
        assert_eq!(tap(&mut g, &l, Gallery::done_rect(&l)), Some(Out::Close));
    }

    #[test]
    fn a_drag_scrolls_instead_of_tapping() {
        let (mut g, l) = gallery(&[(0, Kind::Clock, Size::Medium), (1, Kind::Battery, Size::Small), (2, Kind::Calendar, Size::Small)]);
        let (top, _) = Gallery::viewport(&l);
        let (y, h, _) = g.items(&l).0[1];
        let (small, _) = Gallery::buttons(&l, top + y, h);
        let (x, y0) = ((small.x + 5) as f32, (small.y + 5) as f32);
        g.down(&l, x, y0, 0.0);
        g.moved(x, y0 - 60.0, 20.0);
        assert_eq!(g.up(&l, x, y0 - 60.0, 40.0), None);
    }
}
