//! The page editor, opened by a long press on the page dots: every home
//! page as a thumbnail with a check under it. Tap the check to hide or show
//! the page (hidden pages' apps stay in the App Library), drag a thumbnail
//! onto another to reorder. Done hands the new order and hidden set back.

use super::homeconf::{self, Home, HomeConf, Page};
use super::ui::{Ui, Weight, WHITE};
use crate::gfx::{rgba, Image, Surface};
use crate::layout::{Layout, Rect, COLS, ROWS};

const SLOP: f32 = 12.0;
const RADIUS: f32 = 6.0;
const PER_ROW: usize = 4;
const MARGIN_PT: f32 = 32.0;
const GAP_PT: f32 = 20.0;
const CHECK_PT: f32 = 28.0;
const PAD_PT: f32 = 10.0;

#[derive(Clone, Copy, Debug)]
struct Touch {
    /// the position in `order` of the thumbnail the finger landed on
    pos: Option<usize>,
    on_done: bool,
    x0: f32,
    y0: f32,
    x: f32,
    y: f32,
    dragging: bool,
}

pub struct PageEditor {
    order: Vec<usize>,
    hidden: Vec<usize>,
    flow: Vec<Page>,
    /// widget footprints per flow page: (col, row, cols, rows)
    widgets: Vec<Vec<(usize, usize, usize, usize)>>,
    thumbs: Vec<Image>,
    title: Image,
    done: Image,
    touch: Option<Touch>,
}

impl PageEditor {
    /// `icons` are the apps' icons in grid order (`Home.slots` values index them).
    pub fn open(ui: &Ui, l: &Layout, icons: &[Image], home: &Home, conf: &HomeConf, grid_offset: usize) -> Self {
        let (tw, _) = Self::thumb(l);
        let sz = ((tw - 2 * l.pt(PAD_PT) as i32) / COLS as i32 * 7 / 10).max(4) as usize;
        let mut widgets = vec![Vec::new(); home.flow.len()];
        for w in &conf.widgets {
            if let Some(v) = widgets.get_mut(w.page) {
                let (cw, ch) = w.size.cells();
                v.push((w.col, w.row, cw, ch));
            }
        }
        PageEditor {
            order: home.full.clone(),
            hidden: conf.hidden.clone(),
            flow: home.flow.clone(),
            widgets,
            thumbs: icons.iter().skip(grid_offset).map(|i| i.resized(sz, sz)).collect(),
            title: ui.text("Edit Pages", 17.0, Weight::Semibold, WHITE),
            done: ui.text("Done", 17.0, Weight::Semibold, WHITE),
            touch: None,
        }
    }

    /// The new (order, hidden) to store.
    pub fn result(&self) -> (Vec<usize>, Vec<usize>) {
        (self.order.clone(), self.hidden.clone())
    }

    fn thumb(l: &Layout) -> (i32, i32) {
        let (m, g) = (l.pt(MARGIN_PT) as i32, l.pt(GAP_PT) as i32);
        let tw = (l.w - 2 * m - (PER_ROW as i32 - 1) * g) / PER_ROW as i32;
        (tw, tw * 7 / 5)
    }

    fn cell(l: &Layout, i: usize) -> Rect {
        let (tw, th) = Self::thumb(l);
        let (m, g) = (l.pt(MARGIN_PT) as i32, l.pt(GAP_PT) as i32);
        let pitch = th + l.pt(CHECK_PT) as i32 + 3 * g;
        let top = l.status_h + l.pt(90.0) as i32;
        Rect { x: m + (i % PER_ROW) as i32 * (tw + g), y: top + (i / PER_ROW) as i32 * pitch, w: tw, h: th }
    }

    /// The tap target under a thumbnail's check.
    fn check(l: &Layout, i: usize) -> Rect {
        let c = Self::cell(l, i);
        let t = l.pt(CHECK_PT * 1.6) as i32;
        Rect { x: c.x + (c.w - t) / 2, y: c.y + c.h + l.pt(4.0) as i32, w: t, h: t }
    }

    fn done_rect(&self, l: &Layout) -> Rect {
        let (w, h) = (l.pt(90.0) as i32, l.pt(40.0) as i32);
        Rect { x: l.w - l.pt(MARGIN_PT) as i32 - w, y: l.status_h + l.pt(20.0) as i32, w, h }
    }

    fn card_at(&self, l: &Layout, x: f32, y: f32) -> Option<usize> {
        // the thumbnail and the check under it count as one target
        (0..self.order.len()).find(|&i| {
            let (c, k) = (Self::cell(l, i), Self::check(l, i));
            Rect { h: k.y + k.h - c.y, ..c }.contains(x, y)
        })
    }

    pub fn is_hidden(&self, flow_id: usize) -> bool {
        self.hidden.contains(&flow_id)
    }

    fn toggle(&mut self, pos: usize) {
        let id = self.order[pos];
        if let Some(k) = self.hidden.iter().position(|&h| h == id) {
            self.hidden.remove(k);
        } else if self.order.len() - self.hidden.len() > 1 {
            self.hidden.push(id);
        }
    }

    pub fn down(&mut self, l: &Layout, x: f32, y: f32) {
        self.touch = Some(Touch { pos: self.card_at(l, x, y), on_done: self.done_rect(l).contains(x, y), x0: x, y0: y, x, y, dragging: false });
    }

    pub fn moved(&mut self, x: f32, y: f32) {
        if let Some(tc) = self.touch.as_mut() {
            tc.x = x;
            tc.y = y;
            if tc.pos.is_some() && !tc.dragging && ((x - tc.x0).abs() > SLOP || (y - tc.y0).abs() > SLOP) {
                tc.dragging = true;
            }
        }
    }

    /// Ends the touch; true when Done was tapped.
    pub fn up(&mut self, l: &Layout, x: f32, y: f32) -> bool {
        self.moved(x, y);
        let Some(tc) = self.touch.take() else { return false };
        if tc.on_done {
            return self.done_rect(l).contains(x, y);
        }
        let Some(from) = tc.pos else { return false };
        if tc.dragging {
            if let Some(to) = self.card_at(l, x, y) {
                homeconf::move_to(&mut self.order, from, to);
            }
        } else if (0..self.order.len()).any(|i| i == from && Self::check(l, i).contains(x, y)) {
            self.toggle(from);
        }
        false
    }

    pub fn draw(&self, s: &mut Surface, ui: &Ui, l: &Layout) {
        s.fill_rrect(0.0, 0.0, l.w as f32, l.h as f32, 0.0, rgba(0, 0, 0, 170));
        s.blit(&self.title, (l.w - self.title.w as i32) / 2, l.status_h + l.pt(20.0) as i32 + (l.pt(40.0) as i32 - self.title.h as i32) / 2, 255, 255);
        let d = self.done_rect(l);
        s.blit(&self.done, d.x + (d.w - self.done.w as i32) / 2, d.y + (d.h - self.done.h as i32) / 2, 255, 255);
        let lifted = self.touch.filter(|t| t.dragging).and_then(|t| t.pos.map(|p| (p, t)));
        for i in 0..self.order.len() {
            if lifted.is_some_and(|(p, _)| p == i) {
                continue;
            }
            self.draw_card(s, ui, l, i, Self::cell(l, i), 255);
        }
        if let Some((p, t)) = lifted {
            let c = Self::cell(l, p);
            let moved = Rect { x: c.x + (t.x - t.x0) as i32, y: c.y + (t.y - t.y0) as i32, ..c };
            self.draw_card(s, ui, l, p, moved, 220);
        }
    }

    fn draw_card(&self, s: &mut Surface, ui: &Ui, l: &Layout, pos: usize, c: Rect, alpha: u32) {
        let id = self.order[pos];
        let hidden = self.is_hidden(id);
        let a = if hidden { alpha * 90 / 255 } else { alpha };
        s.fill_rrect(c.x as f32, c.y as f32, c.w as f32, c.h as f32, RADIUS, crate::gfx::scale(rgba(255, 255, 255, 46), a));
        let pad = l.pt(PAD_PT) as i32;
        let (cx, cy) = ((c.w - 2 * pad) / COLS as i32, (c.h - 2 * pad) / ROWS as i32);
        if let Some(widgets) = self.widgets.get(id) {
            for &(col, row, w, h) in widgets {
                let (x, y) = (c.x + pad + col as i32 * cx, c.y + pad + row as i32 * cy);
                s.fill_rrect(x as f32 + 1.0, y as f32 + 1.0, (w as i32 * cx - 2) as f32, (h as i32 * cy - 2) as f32, RADIUS, crate::gfx::scale(rgba(255, 255, 255, 80), a));
            }
        }
        if let Some(page) = self.flow.get(id) {
            for (i, slot) in page.slots.iter().enumerate() {
                let Some(img) = slot.and_then(|app| self.thumbs.get(app)) else { continue };
                let (col, row) = ((i % COLS) as i32, (i / COLS) as i32);
                let (x, y) = (c.x + pad + col * cx + (cx - img.w as i32) / 2, c.y + pad + row * cy + (cy - img.h as i32) / 2);
                s.blit(img, x, y, 255, a);
            }
        }
        let k = l.pt(CHECK_PT) as i32;
        let (bx, by) = (c.x + (c.w - k) / 2, c.y + c.h + l.pt(4.0) as i32 + (l.pt(CHECK_PT * 1.6) as i32 - k) / 2);
        if hidden {
            s.stroke_rrect(bx as f32, by as f32, k as f32, k as f32, RADIUS, 2.0, rgba(255, 255, 255, 200));
        } else {
            s.fill_rrect(bx as f32, by as f32, k as f32, k as f32, RADIUS, WHITE);
            let (kf, t) = (k as f32, 2.6 * ui.s);
            let p = |u: f32, v: f32| (bx as f32 + kf * u, by as f32 + kf * v);
            let shape = super::glyphs::either(super::glyphs::segment(p(0.24, 0.54), p(0.43, 0.72), t), super::glyphs::segment(p(0.43, 0.72), p(0.77, 0.30), t));
            super::glyphs::paint(s, (bx as f32, by as f32, kf, kf), rgba(20, 20, 22, 255), shape);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::PER_PAGE;
    use crate::shell::tests::ui;

    fn editor(conf: &HomeConf) -> (PageEditor, Layout) {
        let l = Layout::new(1200, 1920, 3);
        let home = homeconf::build(PER_PAGE * 2 + 1, conf);
        let icons = vec![Image::new(100, 100); PER_PAGE * 2 + 1];
        (PageEditor::open(&ui(), &l, &icons, &home, conf, 0), l)
    }

    fn tap(e: &mut PageEditor, r: Rect, l: &Layout) -> bool {
        let (x, y) = ((r.x + r.w / 2) as f32, (r.y + r.h / 2) as f32);
        e.down(l, x, y);
        e.up(l, x, y)
    }

    #[test]
    fn the_check_hides_a_page_but_never_the_last() {
        let (mut e, l) = editor(&HomeConf::default());
        assert_eq!(e.order, [0, 1, 2]);
        tap(&mut e, PageEditor::check(&l, 1), &l);
        assert_eq!(e.result().1, [1]);
        tap(&mut e, PageEditor::check(&l, 1), &l);
        assert!(e.result().1.is_empty(), "tapping again shows it");
        for i in [0, 1, 2] {
            tap(&mut e, PageEditor::check(&l, i), &l);
        }
        assert_eq!(e.result().1.len(), 2, "one page always stays");
    }

    #[test]
    fn dragging_a_thumbnail_onto_another_reorders() {
        let (mut e, l) = editor(&HomeConf::default());
        let (a, b) = (PageEditor::cell(&l, 0), PageEditor::cell(&l, 2));
        e.down(&l, (a.x + 20) as f32, (a.y + 20) as f32);
        e.moved((b.x + 30) as f32, (b.y + 30) as f32);
        assert!(!e.up(&l, (b.x + 30) as f32, (b.y + 30) as f32));
        assert_eq!(e.result().0, [1, 2, 0]);
    }

    #[test]
    fn done_is_reported_once_and_only_over_the_button() {
        let (mut e, l) = editor(&HomeConf::default());
        let d = e.done_rect(&l);
        assert!(tap(&mut e, d, &l));
        let c = PageEditor::cell(&l, 0);
        assert!(!tap(&mut e, Rect { x: c.x, y: c.y, w: 10, h: 10 }, &l));
    }
}
