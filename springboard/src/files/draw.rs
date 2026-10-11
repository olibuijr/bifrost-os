//! Drawing the Files app: pages at their transition offsets, then the
//! keyboard, the context menu, an alert and the video overlay on top.

use super::alert::{CARD, RED};
use super::fs::Sort;
use super::icons::{self, BLUE};
use super::menu::{Menu, GAP_PT, ROW_PT as MENU_ROW_PT, RADIUS_PT as MENU_RADIUS_PT};
use super::pages::{self, Body, DirPage, LocationsPage, Page, PreviewPage, Target, CELL_H_PT, CONTENT_TOP_PT, CRUMB_H_PT, CRUMB_TOP_PT, FOOTER_PT, GRID_COLS, LIST_ICON_PT, LOC_HEADER_PT, LOC_ROW_PT, MARGIN_PT, NAV_MID_PT, NAV_PT, ROW_PT, SORT_SLOT_PT, TAGS_GAP_PT};
use super::text::TextView;
use super::Files;
use crate::gfx::{rgba, Image, Surface};
use crate::scene::STATUS_PT;
use crate::shell::keyboard;
use crate::shell::ui::{Ui, Weight, BLACK, CELL, CHEVRON, HAIRLINE, PRESSED, SECONDARY, SEPARATOR, WHITE};

/// The search field's fill (same grey as grouped cells).
const FIELD: u32 = CELL;
const HIGHLIGHT: u32 = PRESSED;
/// Highlight radius for pressed sidebar/grid items (bifrost 10 pt).
const ITEM_RADIUS_PT: f32 = 10.0;

impl Files {
    pub fn draw(&mut self, s: &mut Surface, ui: &Ui, t: f64) {
        self.dirty = false;
        s.fill_rect(0, 0, self.w as usize, self.h as usize, BLACK);
        let bottom = self.bottom(ui);
        let nav = self.picker.as_ref().map_or(&self.nav, |p| &p.nav);
        let frame = nav.frame(t, self.w as f32);
        // Back labels: a page's reads the name of the page beneath it
        let pages = nav.pages();
        let label_below = |i: usize| i.checked_sub(1).map(|j| self.back_image(ui, &pages[j].back_label()));
        let (over_back, under_back) = match nav.direction() {
            Some(super::nav::Dir::Pop) => (label_below(pages.len()), label_below(pages.len() - 1)),
            _ => (label_below(pages.len() - 1), label_below(pages.len().saturating_sub(2))),
        };
        if let Some((under, x, dim)) = frame.under {
            self.draw_page(s, ui, under, x, bottom, under_back.as_ref(), t);
            s.fill_rrect(0.0, 0.0, self.w as f32, self.h as f32, 0.0, rgba(0, 0, 0, dim));
        }
        self.draw_page(s, ui, frame.over, frame.over_x, bottom, over_back.as_ref(), t);
        if self.keyboard.shown() || self.keyboard.animating() {
            let top = self.kb_top(ui);
            let kb = &mut self.keyboard;
            kb.draw(s, ui, top, t);
            s.fill_rect(0, (top + kb.h() + kb.offset(t)) as usize, self.w as usize, self.h as usize, keyboard::BG);
        }
        if let Some(m) = &self.menu {
            self.draw_menu(s, ui, m);
        }
        if let Some(a) = &self.alert {
            s.fill_rrect(0.0, 0.0, self.w as f32, self.h as f32, 0.0, rgba(0, 0, 0, 110));
            let bottom = if self.keyboard.shown() { self.kb_top(ui) } else { self.h as f32 - crate::shell::ui::home_zone(ui) };
            let pressed = match self.alert_pressed {
                Some(super::alert::Hit::Button(i)) => Some(i),
                _ => None,
            };
            a.draw(s, ui, self.w, bottom, pressed, t);
            if self.keyboard.shown() || self.keyboard.animating() {
                let top = self.kb_top(ui);
                let kb = &mut self.keyboard;
                kb.draw(s, ui, top, t);
            }
        }
        if let Some(v) = self.playing.as_ref().and_then(|p| p.view.as_ref()) {
            v.draw(s, ui, t);
        } else if let Some(p) = &self.playing {
            s.fill_rect(0, 0, self.w as usize, self.h as usize, BLACK);
            if let Some(f) = &p.frame {
                s.blit(f, (self.w - f.w as i32) / 2, (self.h - f.h as i32) / 2, 255, 255);
            }
            if p.overlay {
                let cx = self.w / 2;
                crate::shell::ui::centred(s, &p.title, cx, (self.h as f32 * 0.5 - ui.pt(60.0)) as i32);
                crate::shell::ui::centred(s, &p.status_img, cx, (self.h as f32 * 0.5 - ui.pt(28.0)) as i32);
                let (x, y, w, h) = self.stop_rect(ui);
                s.fill_rrect(x, y, w, h, ui.pt(10.0), WHITE);
                s.blit(&p.stop, (x + (w - p.stop.w as f32) / 2.0) as i32, (y + (h - p.stop.h as f32) / 2.0) as i32, 255, 255);
            }
        }
    }

    /// One page with its left edge at `ox` px.
    #[allow(clippy::too_many_arguments)]
    fn draw_page(&self, s: &mut Surface, ui: &Ui, page: &Page, ox: f32, bottom: f32, back: Option<&Image>, t: f64) {
        let (w, h) = (self.w as f32, self.h as f32);
        if ox >= w {
            return;
        }
        s.fill_rrect(ox, 0.0, w, h, 0.0, BLACK);
        let pressed = self.pressed;
        match page {
            Page::Locations(l) => self.draw_locations(s, ui, l, ox, pressed),
            Page::Dir(d) => self.draw_dir(s, ui, d, ox, bottom, pressed, t),
            Page::Preview(p) => self.draw_preview(s, ui, p, ox, bottom),
        }
        self.draw_nav(s, ui, page, ox, back);
    }

    /// The nav bar: back/cancel at the left, the title centred, the
    /// right-hand buttons of the page.
    fn draw_nav(&self, s: &mut Surface, ui: &Ui, page: &Page, ox: f32, back: Option<&Image>) {
        let mid = ui.pt(NAV_MID_PT);
        let m = ui.pt(MARGIN_PT);
        let picking = self.picker.is_some();
        let (left, right) = self.nav_buttons();
        match (left, &self.picker) {
            (Some(Target::Cancel), Some(p)) => s.blit(&p.cancel, (ox + m) as i32, (mid - p.cancel.h as f32 / 2.0) as i32, 255, 255),
            (Some(Target::Back), _) => {
                let dim = if self.pressed == Some(Target::Back) { 140 } else { 255 };
                icons::chevron(s, (ox + m + ui.pt(6.0), mid), ui.pt(20.0), ui.pt(3.0), icons::Point::Left, rgba(10, 132, 255, dim));
                if let Some(b) = back {
                    s.blit(b, (ox + m + ui.pt(18.0)) as i32, (mid - b.h as f32 / 2.0) as i32, 255, dim as u32);
                }
            }
            _ => {}
        }
        if let Some(title) = page.title() {
            s.blit(title, (ox + (self.w as f32 - title.w as f32) / 2.0) as i32, (mid - title.h as f32 / 2.0) as i32, 255, 255);
        }
        for (i, tgt) in right.iter().enumerate() {
            let x = ox + pages::nav_right_x(ui, self.w, i);
            let alpha = if self.pressed == Some(*tgt) { 140 } else { 255 };
            match tgt {
                Target::ViewToggle => icons::view_button(s, (x + ui.pt(8.0), mid - ui.pt(14.0)), ui.pt(28.0), !self.grid),
                Target::NewFolder => icons::new_folder_button(s, (x + ui.pt(8.0), mid - ui.pt(14.0)), ui.pt(28.0)),
                Target::Select => {
                    let label = self.back_image(ui, if self.selecting { "Done" } else { "Select" });
                    let cx = ox + pages::nav_text_x(ui, self.w, i);
                    s.blit(&label, (cx - label.w as f32 / 2.0) as i32, (mid - label.h as f32 / 2.0) as i32, 255, alpha);
                }
                Target::Share => icons::menu(s, (x + ui.pt(10.0), mid - ui.pt(12.0)), ui.pt(24.0), super::menu::Glyph::Share, rgba(10, 132, 255, alpha as u8)),
                Target::Markup => icons::menu(s, (x + ui.pt(10.0), mid - ui.pt(12.0)), ui.pt(24.0), super::menu::Glyph::Pencil, rgba(10, 132, 255, alpha as u8)),
                Target::Confirm => {
                    if let Some(p) = &self.picker {
                        let enabled = picking && matches!(page, Page::Dir(_));
                        s.blit(&p.confirm, (ox + self.w as f32 - m - p.confirm.w as f32) as i32, (mid - p.confirm.h as f32 / 2.0) as i32, 255, if enabled { alpha } else { 90 });
                    }
                }
                _ => {}
            }
        }
        let _ = CELL;
    }

    fn draw_locations(&self, s: &mut Surface, ui: &Ui, l: &LocationsPage, ox: f32, pressed: Option<Target>) {
        let m = ui.pt(MARGIN_PT);
        s.blit(&l.title, (ox + m - ui.pt(2.0)) as i32, ui.pt(STATUS_PT + 44.0) as i32, 255, 255);
        s.blit(&l.header, (ox + m) as i32, (ui.pt(LOC_HEADER_PT) + ui.pt(4.0)) as i32, 255, 255);
        let rh = ui.pt(LOC_ROW_PT);
        let text_x = ox + m + ui.pt(52.0);
        for (i, loc) in l.locations.iter().enumerate() {
            let y = l.top(ui, i);
            if i == l.tags_from && i < l.locations.len() {
                s.blit(&l.tags_header, (ox + m) as i32, (y - ui.pt(TAGS_GAP_PT) + ui.pt(10.0)) as i32, 255, 255);
            }
            if pressed == Some(Target::Location(i)) {
                s.fill_rrect(ox + m, y, self.w as f32 - 2.0 * m, rh, ui.pt(ITEM_RADIUS_PT), HIGHLIGHT);
            }
            let isz = ui.pt(28.0);
            icons::location(s, (ox + m + ui.pt(8.0), y + (rh - isz) / 2.0), isz, loc.glyph);
            let label = &l.labels[i];
            s.blit(label, text_x as i32, (y + (rh - label.h as f32) / 2.0) as i32, 255, 255);
            if i + 1 < l.locations.len() && i + 1 != l.tags_from {
                s.fill_rrect(text_x, y + rh - 1.0, self.w as f32 - m - (text_x - ox), 1.0, 0.0, SEPARATOR);
            }
        }
        // storage usage under the list: one bar of used over total, as in Settings
        if let Some((used, total, caption)) = &l.storage {
            let y = l.top(ui, l.locations.len()) + ui.pt(16.0);
            let bw = (self.w as f32 - 2.0 * m) as u32;
            let (bh, r) = (ui.pt(8.0), ui.pt(3.0));
            s.fill_rrect(ox + m, y, bw as f32, bh, r, FIELD);
            let fill = crate::shell::storage::bar_widths(&[*used], *total, bw)[0];
            if fill > 0 {
                s.fill_rrect(ox + m, y, fill as f32, bh, r, BLUE);
            }
            s.blit(caption, (ox + m) as i32, (y + bh + ui.pt(8.0)) as i32, 255, 255);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_dir(&self, s: &mut Surface, ui: &Ui, d: &DirPage, ox: f32, bottom: f32, pressed: Option<Target>, t: f64) {
        let w = self.w;
        let m = ui.pt(MARGIN_PT);
        // search field
        let (sx, sy, sw, sh) = pages::search_rect(ui, w, d.searching);
        s.fill_rrect(ox + sx, sy, sw, sh, 1.0e4, FIELD);
        icons::magnifier(s, (ox + sx + ui.pt(10.0), sy + (sh - ui.pt(20.0)) / 2.0), ui.pt(20.0), SECONDARY);
        let text_x = ox + sx + ui.pt(36.0);
        d.query.draw(s, ui, (text_x, sy, sw - ui.pt(36.0 + 36.0), sh), t);
        if !d.query.is_empty() {
            let (cx, cy) = pages::clear_centre(ui, w, d.searching);
            icons::clear_button(s, (ox + cx, cy), ui.pt(9.0));
        }
        if d.searching {
            let cancel = ui.text("Cancel", 17.0, Weight::Regular, BLUE);
            s.blit(&cancel, (ox + sx + sw + ui.pt(14.0)) as i32, (sy + (sh - cancel.h as f32) / 2.0) as i32, 255, 255);
        }
        // breadcrumb and sort control
        let cy = ui.pt(CRUMB_TOP_PT + CRUMB_H_PT / 2.0);
        let mut cx = ox + m;
        let n = d.crumbs.len();
        for (i, (img, _)) in d.crumbs.iter().enumerate() {
            let last = i + 1 == n;
            let alpha = if !last && pressed == Some(Target::Crumb(i)) { 120 } else { 255 };
            s.blit(img, cx as i32, (cy - img.h as f32 / 2.0) as i32, 255, alpha);
            cx += img.w as f32 + ui.pt(8.0);
            if !last {
                icons::chevron(s, (cx + ui.pt(2.0), cy), ui.pt(9.0), ui.pt(1.5), icons::Point::Right, SECONDARY);
                cx += ui.pt(14.0);
            }
        }
        for (i, sort) in Sort::ALL.iter().enumerate() {
            let x = ox + pages::sort_x(ui, w, i);
            let current = *sort == self.sort;
            let label = ui.text(sort.label(), 13.0, if current { Weight::Semibold } else { Weight::Regular }, if current { WHITE } else { SECONDARY });
            let lw = label.w as f32 + if current { ui.pt(12.0) } else { 0.0 };
            let lx = x + (ui.pt(SORT_SLOT_PT) - lw) / 2.0;
            s.blit(&label, lx as i32, (cy - label.h as f32 / 2.0) as i32, 255, if pressed == Some(Target::Sort(*sort)) { 120 } else { 255 });
            if current {
                icons::chevron(s, (lx + label.w as f32 + ui.pt(7.0), cy + ui.pt(1.0)), ui.pt(8.0), ui.pt(1.5), icons::Point::Down, WHITE);
            }
        }
        // listing, clipped to its band
        let top = ui.pt(CONTENT_TOP_PT);
        {
            let mut band = s.rows(top as usize, bottom as usize);
            self.draw_entries(&mut band, ui, d, ox, bottom, pressed);
        }
        if self.selecting && self.picker.is_none() {
            let n = self.selected.len();
            let third = w as f32 / 3.0;
            let labels = [("Copy", Target::BulkCopy, BLUE), ("Move", Target::BulkMove, BLUE), ("Delete", Target::BulkDelete, RED)];
            for (k, (text, tgt, color)) in labels.into_iter().enumerate() {
                let img = ui.text(text, 17.0, Weight::Regular, color);
                let alpha = if n == 0 { 90 } else if pressed == Some(tgt) { 140 } else { 255 };
                let cx = ox + third * (k as f32 + 0.5);
                s.blit(&img, (cx - img.w as f32 / 2.0) as i32, (bottom + (ui.pt(FOOTER_PT) - img.h as f32) / 2.0) as i32, 255, alpha);
            }
        } else if let Some(f) = &d.footer {
            s.blit(f, (ox + (w as f32 - f.w as f32) / 2.0) as i32, (bottom + (ui.pt(FOOTER_PT) - f.h as f32) / 2.0) as i32, 255, 255);
        }
    }

    fn draw_entries(&self, s: &mut Surface, ui: &Ui, d: &DirPage, ox: f32, bottom: f32, pressed: Option<Target>) {
        let w = self.w;
        let top = ui.pt(CONTENT_TOP_PT);
        let mid_y = (top + (bottom - top) / 2.0) as i32;
        let cx = (ox + w as f32 / 2.0) as i32;
        if let Some(e) = &d.error {
            crate::shell::ui::centred(s, e, cx, mid_y);
            return;
        }
        if d.loading && d.entries.is_empty() {
            crate::shell::ui::centred(s, &ui.text("Loading…", 15.0, Weight::Regular, SECONDARY), cx, mid_y);
            return;
        }
        if d.shown.is_empty() {
            let msg = if d.query.is_empty() { "No Items" } else { "No Results" };
            crate::shell::ui::centred(s, &ui.text(msg, 20.0, Weight::Semibold, SECONDARY), cx, mid_y);
            return;
        }
        let menu_on = self.menu.as_ref().and_then(|m| match m.subject {
            super::menu::Subject::Entry(i) => Some(i),
            _ => None,
        });
        for k in d.visible(ui, w, self.grid, bottom) {
            let i = d.shown[k];
            let e = &d.entries[i];
            let label = d.label(ui, w, i, self.now, self.offset);
            let (rx, ry, rw, rh) = d.item_rect(ui, w, self.grid, k);
            let (rx, ry) = (rx + ox, ry);
            let marked = self.selecting && self.picker.is_none() && self.selected.contains(&e.path);
            let lit = pressed == Some(Target::Entry(i)) || menu_on == Some(i) || marked;
            if self.grid {
                if lit {
                    s.fill_rrect(rx + ui.pt(4.0), ry + ui.pt(4.0), rw - ui.pt(8.0), rh - ui.pt(8.0), ui.pt(ITEM_RADIUS_PT), HIGHLIGHT);
                }
                let (iw, ih) = (ui.pt(80.0), ui.pt(64.0));
                icons::entry(s, (rx + (rw - iw) / 2.0, ry + ui.pt(10.0), iw, ih), e.kind);
                let ccx = (rx + rw / 2.0) as i32;
                crate::shell::ui::centred(s, &label.grid_name, ccx, (ry + ui.pt(84.0)) as i32);
                crate::shell::ui::centred(s, &label.grid_sub, ccx, (ry + ui.pt(106.0)) as i32);
                if self.selecting && self.picker.is_none() {
                    icons::check_circle(s, (rx + rw - ui.pt(30.0), ry + ui.pt(8.0)), ui.pt(22.0), marked);
                }
            } else {
                if lit {
                    s.fill_rrect(rx, ry, rw, rh, 0.0, HIGHLIGHT);
                }
                let isz = ui.pt(LIST_ICON_PT);
                let ix = rx + ui.pt(MARGIN_PT);
                icons::entry(s, (ix, ry + (rh - isz) / 2.0, isz, isz), e.kind);
                let tx = rx + ui.pt(MARGIN_PT + 52.0);
                s.blit(&label.name, tx as i32, (ry + ui.pt(10.0)) as i32, 255, 255);
                s.blit(&label.sub, tx as i32, (ry + ui.pt(34.0)) as i32, 255, 255);
                if self.selecting && self.picker.is_none() {
                    icons::check_circle(s, (rx + rw - ui.pt(MARGIN_PT + 52.0), ry + (rh - ui.pt(22.0)) / 2.0), ui.pt(22.0), marked);
                }
                for (n, tag) in self.tags.tags(&e.path).into_iter().enumerate() {
                    icons::tag_dot(s, (rx + rw - ui.pt(MARGIN_PT + 34.0 + 14.0 * n as f32), ry + rh / 2.0), ui.pt(5.0), tag);
                }
                if e.is_dir {
                    icons::chevron(s, (rx + rw - ui.pt(MARGIN_PT + 6.0), ry + rh / 2.0), ui.pt(12.0), ui.pt(2.0), icons::Point::Right, CHEVRON);
                }
                if k + 1 < d.shown.len() {
                    s.fill_rrect(tx, ry + rh - 1.0, rx + rw - tx, 1.0, 0.0, SEPARATOR);
                }
            }
        }
        let _ = (GRID_COLS, CELL_H_PT, ROW_PT);
    }

    fn draw_preview(&self, s: &mut Surface, ui: &Ui, p: &PreviewPage, ox: f32, bottom: f32) {
        let top = ui.pt(STATUS_PT + NAV_PT);
        let (w, cx) = (self.w, (ox + self.w as f32 / 2.0) as i32);
        let mid_y = (top + (bottom - top) / 2.0) as i32;
        match &p.body {
            Body::Loading => crate::shell::ui::centred(s, &ui.text("Loading…", 15.0, Weight::Regular, SECONDARY), cx, mid_y),
            Body::Error(e) => crate::shell::ui::centred(s, e, cx, mid_y),
            Body::Image(img) => {
                let x = ox + (w as f32 - img.w as f32) / 2.0;
                let y = top + (bottom - top - img.h as f32) / 2.0;
                s.blit(img, x as i32, y as i32, 255, 255);
            }
            Body::Text(tv) => {
                let mut band = s.rows(top as usize, bottom as usize);
                tv.draw(&mut band, &self.mono, ui, ox, top, bottom, p.scroll.offset());
                let _ = TextView::line_h(ui);
            }
            Body::Info(rows) => {
                let m = ui.pt(MARGIN_PT);
                let rh = ui.pt(44.0);
                let cw = w as f32 - 2.0 * m;
                let y0 = top + ui.pt(20.0) - p.scroll.offset();
                let mut band = s.rows(top as usize, bottom as usize);
                band.fill_rrect(ox + m, y0, cw, rh * rows.len() as f32, ui.pt(10.0), CELL);
                for (i, (k, v)) in rows.iter().enumerate() {
                    let ry = y0 + i as f32 * rh;
                    band.blit(k, (ox + m + ui.pt(16.0)) as i32, (ry + (rh - k.h as f32) / 2.0) as i32, 255, 255);
                    band.blit(v, (ox + m + cw - ui.pt(16.0) - v.w as f32) as i32, (ry + (rh - v.h as f32) / 2.0) as i32, 255, 255);
                    if i + 1 < rows.len() {
                        band.fill_rrect(ox + m + ui.pt(16.0), ry + rh - 1.0, cw - ui.pt(16.0), 1.0, 0.0, SEPARATOR);
                    }
                }
            }
        }
    }

    fn draw_menu(&self, s: &mut Surface, ui: &Ui, m: &Menu) {
        let bottom = self.h as f32 - crate::shell::ui::home_zone(ui);
        s.fill_rrect(0.0, 0.0, self.w as f32, self.h as f32, 0.0, rgba(0, 0, 0, 90));
        let r = ui.pt(MENU_RADIUS_PT);
        for (x, y, w, h) in m.sections(ui, self.w, bottom) {
            s.fill_rrect(x, y, w, h, r, CARD);
        }
        let rows = m.rows(ui, self.w, bottom);
        for (i, (a, (x, y, w, h))) in rows.iter().enumerate() {
            if self.menu_pressed == Some(*a) {
                let first = i == 0 || rows[i - 1].1 .1 + rows[i - 1].1 .3 < *y;
                let last = i + 1 == rows.len() || rows[i + 1].1 .1 > y + h;
                crate::shell::settings::fill_row(s, ui, (*x, *y, *w, *h), first, last, HIGHLIGHT);
            }
            let color = if a.destructive() { RED } else { WHITE };
            let label = ui.text(a.label(), 17.0, Weight::Regular, color);
            s.blit(&label, (x + ui.pt(16.0)) as i32, (y + (h - label.h as f32) / 2.0) as i32, 255, 255);
            let g = ui.pt(22.0);
            icons::menu(s, (x + w - ui.pt(16.0) - g, y + (h - g) / 2.0), g, a.glyph(), color);
            let next_in_section = rows.get(i + 1).is_some_and(|(_, (_, ny, ..))| (*ny - (y + h)).abs() < 0.5);
            if next_in_section {
                s.fill_rect((x + ui.pt(16.0)) as usize, (y + h - 1.0) as usize, (w - ui.pt(16.0)) as usize, 1, HAIRLINE);
            }
        }
        let _ = (GAP_PT, MENU_ROW_PT);
    }
}
