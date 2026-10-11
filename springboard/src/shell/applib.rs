//! The App Library: the last home-screen page. Category cards (from the
//! static map in `catalog`) each show a 2 x 2 grid of the category's icons;
//! a tap on an icon launches it, a tap on the card opens the category's list.
//! The sixth card, All Apps, lists every app A to Z, and the search field at
//! the top hands over to Search. Apps hidden from the home pages live here.

use super::ui::{Ui, Weight, SECONDARY, WHITE};
use super::apps;
use crate::catalog::{self, Category};
use crate::deildu::scroll::Scroller;
use crate::gfx::{rgba, Image, Surface};
use crate::layout::{Layout, Rect};

const SLOP: f32 = 12.0;
const RADIUS: f32 = 6.0;
const MARGIN_PT: f32 = 32.0;
const GAP_PT: f32 = 24.0;
const FIELD_TOP_PT: f32 = 14.0;
const FIELD_H_PT: f32 = 44.0;
const CARDS_TOP_PT: f32 = 28.0;
const TITLE_H_PT: f32 = 36.0;
const MINI_GAP_PT: f32 = 18.0;
const ROW_PT: f32 = 60.0;
const LETTER_PT: f32 = 30.0;
const ICON_PT: f32 = 44.0;
/// the card index of "All Apps"
const ALL: usize = 5;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum View {
    Cards,
    /// the list behind card `i`
    List(usize),
}

/// What a tap asks the shell to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Out {
    /// open this app (an index into `apps()`)
    Launch(usize),
    /// the search field was tapped
    Search,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Target {
    Search,
    Card(usize),
    Mini(usize),
    Back,
    Row(usize),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Claim {
    Undecided,
    Vertical,
    Horizontal,
}

#[derive(Clone, Copy, Debug)]
struct Touch {
    x0: f32,
    y0: f32,
    claim: Claim,
    target: Option<Target>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Row {
    /// index into `letters`
    Letter(usize),
    /// an `apps()` index
    App(usize),
}

pub struct AppLibrary {
    /// the apps on each card (`apps()` indices): the five categories, then All Apps
    cards: Vec<Vec<usize>>,
    titles: Vec<Image>,
    minis: Vec<Image>,
    icons: Vec<Image>,
    names: Vec<Image>,
    letters: Vec<(char, Image)>,
    search: Image,
    back: Image,
    mini: i32,
    view: View,
    scroll: Scroller,
    touch: Option<Touch>,
    gen: u32,
}

fn px(l: &Layout, pt: f32) -> i32 {
    l.pt(pt).round() as i32
}

fn initial(name: &str) -> char {
    name.chars().next().filter(|c| c.is_alphabetic()).map_or('#', |c| c.to_ascii_uppercase())
}

/// The apps per card: each category by name, then every app by name.
fn group() -> Vec<Vec<usize>> {
    let by_name = |v: &mut Vec<usize>| v.sort_by_key(|&i| apps()[i].name.to_lowercase());
    let mut cards: Vec<Vec<usize>> = Category::ALL
        .iter()
        .map(|&c| {
            let mut v: Vec<usize> = (0..apps().len()).filter(|&i| catalog::category(apps()[i].icon) == c).collect();
            by_name(&mut v);
            v
        })
        .collect();
    let mut all: Vec<usize> = (0..apps().len()).collect();
    by_name(&mut all);
    cards.push(all);
    cards
}

impl AppLibrary {
    /// `icons` are the apps' masked icons in `apps()` order at the home icon size.
    pub fn new(ui: &Ui, l: &Layout, icons: &[Image]) -> Self {
        let cards = group();
        let mut titles: Vec<Image> = Category::ALL.iter().map(|c| ui.text(c.name(), 15.0, Weight::Medium, WHITE)).collect();
        titles.push(ui.text("All Apps", 15.0, Weight::Medium, WHITE));
        let (cw, g) = (Self::card_w(l), px(l, MINI_GAP_PT));
        let mini = ((cw - 3 * g) / 2).min(l.icon).max(8);
        let list = ui.pt(ICON_PT) as usize;
        let mut letters: Vec<(char, Image)> = Vec::new();
        for &i in &cards[ALL] {
            let c = initial(apps()[i].name);
            if !letters.iter().any(|(k, _)| *k == c) {
                letters.push((c, ui.text(&c.to_string(), 15.0, Weight::Semibold, SECONDARY)));
            }
        }
        AppLibrary {
            titles,
            minis: icons.iter().map(|i| i.resized(mini as usize, mini as usize)).collect(),
            icons: icons.iter().map(|i| i.resized(list, list)).collect(),
            names: apps().iter().map(|a| ui.text(a.name, 17.0, Weight::Regular, WHITE)).collect(),
            letters,
            search: ui.text("Search", 17.0, Weight::Regular, rgba(235, 235, 245, 150)),
            back: ui.text("Back", 17.0, Weight::Semibold, WHITE),
            mini,
            cards,
            view: View::Cards,
            scroll: Scroller::new(),
            touch: None,
            gen: 1,
        }
    }

    /// Bumped whenever the page looks different.
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

    /// Back to the cards with nothing held (leaving home, screen off).
    pub fn reset(&mut self) {
        self.view = View::Cards;
        self.touch = None;
        self.scroll.jump_top();
        self.gen = self.gen.wrapping_add(1);
    }

    pub fn in_list(&self) -> bool {
        self.view != View::Cards
    }

    /// The apps on card `i` (an `apps()` index list).
    pub fn card_apps(&self, i: usize) -> &[usize] {
        self.cards.get(i).map_or(&[][..], |c| c.as_slice())
    }

    // -- geometry (px, page-relative) --

    fn card_w(l: &Layout) -> i32 {
        (l.w - 2 * px(l, MARGIN_PT) - 2 * px(l, GAP_PT)) / 3
    }

    fn field(&self, l: &Layout) -> Rect {
        Rect { x: px(l, MARGIN_PT), y: l.status_h + px(l, FIELD_TOP_PT), w: l.w - 2 * px(l, MARGIN_PT), h: px(l, FIELD_H_PT) }
    }

    fn back_rect(&self, l: &Layout) -> Rect {
        Rect { x: px(l, MARGIN_PT), y: l.status_h + px(l, FIELD_TOP_PT), w: px(l, 90.0), h: px(l, FIELD_H_PT) }
    }

    /// The six card squares (titles hang below them).
    fn cells(&self, l: &Layout) -> Vec<Rect> {
        let f = self.field(l);
        let (cw, gap) = (Self::card_w(l), px(l, GAP_PT));
        let y0 = f.y + f.h + px(l, CARDS_TOP_PT);
        let row = cw + px(l, TITLE_H_PT) + px(l, 12.0);
        (0..6).map(|i| Rect { x: px(l, MARGIN_PT) + (i as i32 % 3) * (cw + gap), y: y0 + (i as i32 / 3) * row, w: cw, h: cw }).collect()
    }

    fn mini_rect(&self, l: &Layout, card: Rect, k: usize) -> Rect {
        let g = px(l, MINI_GAP_PT);
        let total = 2 * self.mini + g;
        let (ox, oy) = (card.x + (card.w - total) / 2, card.y + (card.h - total) / 2);
        Rect { x: ox + (k as i32 % 2) * (self.mini + g), y: oy + (k as i32 / 2) * (self.mini + g), w: self.mini, h: self.mini }
    }

    /// The list's viewport: (top, bottom).
    fn list_span(&self, l: &Layout) -> (i32, i32) {
        let b = self.back_rect(l);
        (b.y + b.h + px(l, 12.0), l.dots_y - l.dot)
    }

    /// The rows of the open list as (y from the top of the list, height, row), and the content height.
    fn rows(&self, l: &Layout) -> (Vec<(i32, i32, Row)>, i32) {
        let View::List(i) = self.view else { return (vec![], 0) };
        let (row_h, letter_h) = (px(l, ROW_PT), px(l, LETTER_PT));
        let mut y = 0;
        let mut out = Vec::new();
        let mut last = None;
        for &a in self.card_apps(i) {
            if i == ALL {
                let c = initial(apps()[a].name);
                if last != Some(c) {
                    last = Some(c);
                    if let Some(k) = self.letters.iter().position(|(ch, _)| *ch == c) {
                        out.push((y, letter_h, Row::Letter(k)));
                        y += letter_h;
                    }
                }
            }
            out.push((y, row_h, Row::App(a)));
            y += row_h;
        }
        (out, y)
    }

    fn fit_scroll(&mut self, l: &Layout) {
        let (top, bottom) = self.list_span(l);
        let (_, content) = self.rows(l);
        self.scroll.set_max((content - (bottom - top)).max(0) as f32);
    }

    fn hit(&self, l: &Layout, x: f32, y: f32) -> Option<Target> {
        match self.view {
            View::Cards => {
                if self.field(l).contains(x, y) {
                    return Some(Target::Search);
                }
                for (i, c) in self.cells(l).into_iter().enumerate() {
                    if !c.contains(x, y) {
                        continue;
                    }
                    for (k, &a) in self.cards[i].iter().take(4).enumerate() {
                        if self.mini_rect(l, c, k).contains(x, y) {
                            return Some(Target::Mini(a));
                        }
                    }
                    return Some(Target::Card(i));
                }
                None
            }
            View::List(_) => {
                if self.back_rect(l).contains(x, y) {
                    return Some(Target::Back);
                }
                let (top, bottom) = self.list_span(l);
                if y < top as f32 || y >= bottom as f32 {
                    return None;
                }
                let off = self.scroll.offset();
                self.rows(l).0.into_iter().find_map(|(ry, rh, row)| match row {
                    Row::App(a) if y >= (top + ry) as f32 - off && y < (top + ry + rh) as f32 - off => Some(Target::Row(a)),
                    _ => None,
                })
            }
        }
    }

    // -- touch --

    pub fn down(&mut self, l: &Layout, x: f32, y: f32, t: f64) {
        self.scroll.down(y, t);
        self.touch = Some(Touch { x0: x, y0: y, claim: Claim::Undecided, target: self.hit(l, x, y) });
    }

    /// True while this page owns the touch (it has not turned out horizontal).
    pub fn owns(&self) -> bool {
        self.touch.is_some_and(|t| t.claim != Claim::Horizontal)
    }

    /// Follows the finger; true the moment the page claims it as vertical.
    pub fn moved(&mut self, x: f32, y: f32, t: f64) -> bool {
        let Some(mut tc) = self.touch else { return false };
        let mut claimed = false;
        if tc.claim == Claim::Undecided {
            let (dx, dy) = (x - tc.x0, y - tc.y0);
            if dx.abs() > SLOP || dy.abs() > SLOP {
                tc.claim = if dy.abs() >= dx.abs() { Claim::Vertical } else { Claim::Horizontal };
                claimed = tc.claim == Claim::Vertical;
            }
        }
        if tc.claim == Claim::Vertical {
            self.scroll.moved(y, t);
            self.gen = self.gen.wrapping_add(1);
        }
        self.touch = Some(tc);
        claimed
    }

    pub fn up(&mut self, l: &Layout, x: f32, y: f32, t: f64) -> Option<Out> {
        let tc = self.touch.take()?;
        self.gen = self.gen.wrapping_add(1);
        match tc.claim {
            Claim::Horizontal => {
                self.scroll.up(tc.y0, t);
                None
            }
            Claim::Vertical => {
                self.scroll.up(y, t);
                None
            }
            Claim::Undecided => {
                self.scroll.up(tc.y0, t);
                let target = tc.target.filter(|&tg| self.hit(l, x, y) == Some(tg))?;
                self.tap(l, target)
            }
        }
    }

    fn tap(&mut self, l: &Layout, target: Target) -> Option<Out> {
        match target {
            Target::Search => Some(Out::Search),
            Target::Mini(a) | Target::Row(a) => Some(Out::Launch(a)),
            Target::Card(i) => {
                self.view = View::List(i);
                self.scroll.jump_top();
                self.fit_scroll(l);
                None
            }
            Target::Back => {
                self.view = View::Cards;
                self.scroll.jump_top();
                None
            }
        }
    }

    // -- drawing --

    /// Draws the page with its left edge at screen x `x0`.
    pub fn draw(&self, s: &mut Surface, l: &Layout, x0: i32) {
        match self.view {
            View::Cards => self.draw_cards(s, l, x0),
            View::List(i) => self.draw_list(s, l, x0, i),
        }
    }

    fn draw_cards(&self, s: &mut Surface, l: &Layout, x0: i32) {
        let f = self.field(l);
        s.fill_rrect((f.x + x0) as f32, f.y as f32, f.w as f32, f.h as f32, RADIUS, rgba(255, 255, 255, 40));
        super::today::draw_magnifier(s, (f.x + x0) as f32 + l.pt(12.0), f.y as f32 + f.h as f32 / 2.0, l.s, rgba(235, 235, 245, 150));
        s.blit(&self.search, f.x + x0 + px(l, 36.0), f.y + (f.h - self.search.h as i32) / 2, 255, 255);
        for (i, c) in self.cells(l).into_iter().enumerate() {
            s.fill_rrect((c.x + x0) as f32, c.y as f32, c.w as f32, c.h as f32, RADIUS, rgba(40, 40, 44, 175));
            for (k, &a) in self.cards[i].iter().take(4).enumerate() {
                let r = self.mini_rect(l, c, k);
                s.blit(&self.minis[a], r.x + x0, r.y, 255, 255);
            }
            s.blit(&self.titles[i], c.x + x0, c.y + c.h + px(l, 8.0), 255, 255);
        }
    }

    fn draw_list(&self, s: &mut Surface, l: &Layout, x0: i32, card: usize) {
        let b = self.back_rect(l);
        s.blit(&self.back, b.x + x0, b.y + (b.h - self.back.h as i32) / 2, 255, 255);
        let title = &self.titles[card];
        s.blit(title, x0 + (l.w - title.w as i32) / 2, b.y + (b.h - title.h as i32) / 2, 255, 255);
        let (top, bottom) = self.list_span(l);
        if bottom <= top {
            return;
        }
        let mut band = s.rows(top as usize, bottom as usize);
        let scroll = self.scroll.offset().round() as i32;
        let (left, gap) = (px(l, MARGIN_PT) + x0, px(l, 14.0));
        for (ry, rh, row) in self.rows(l).0 {
            let y = top + ry - scroll;
            if y + rh < top || y > bottom {
                continue;
            }
            match row {
                Row::Letter(k) => {
                    let img = &self.letters[k].1;
                    band.blit(img, left, y + (rh - img.h as i32) / 2, 255, 255);
                }
                Row::App(a) => {
                    let (icon, name) = (&self.icons[a], &self.names[a]);
                    band.blit(icon, left, y + (rh - icon.h as i32) / 2, 255, 255);
                    band.blit(name, left + icon.w as i32 + gap, y + (rh - name.h as i32) / 2, 255, 255);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shell::tests::ui;

    fn library() -> (AppLibrary, Layout) {
        let l = Layout::new(1200, 1920, 3);
        let icons: Vec<Image> = apps().iter().map(|_| Image::new(l.icon as usize, l.icon as usize)).collect();
        (AppLibrary::new(&ui(), &l, &icons), l)
    }

    fn tap(lib: &mut AppLibrary, l: &Layout, x: f32, y: f32) -> Option<Out> {
        lib.down(l, x, y, 0.0);
        lib.up(l, x, y, 30.0)
    }

    #[test]
    fn every_app_is_in_one_category_and_all_apps_is_sorted() {
        let cards = group();
        assert_eq!(cards.len(), 6);
        let mut seen: Vec<usize> = cards[..5].iter().flatten().copied().collect();
        seen.sort();
        assert_eq!(seen, (0..apps().len()).collect::<Vec<_>>());
        let names: Vec<String> = cards[ALL].iter().map(|&i| apps()[i].name.to_lowercase()).collect();
        let mut sorted = names.clone();
        sorted.sort();
        assert_eq!(names, sorted);
        let terminal = apps().iter().position(|a| a.name == "Term").unwrap();
        assert!(cards[4].contains(&terminal), "the terminal is a Developer app");
    }

    #[test]
    fn tapping_a_mini_icon_launches_and_the_card_opens_its_list() {
        let (mut lib, l) = library();
        let cells = lib.cells(&l);
        let first = lib.card_apps(0)[0];
        let r = lib.mini_rect(&l, cells[0], 0);
        assert_eq!(tap(&mut lib, &l, (r.x + 5) as f32, (r.y + 5) as f32), Some(Out::Launch(first)));
        let c = cells[0];
        assert_eq!(tap(&mut lib, &l, (c.x + 2) as f32, (c.y + 2) as f32), None);
        assert!(lib.in_list());
        let b = lib.back_rect(&l);
        assert_eq!(tap(&mut lib, &l, (b.x + 5) as f32, (b.y + 5) as f32), None);
        assert!(!lib.in_list());
    }

    #[test]
    fn the_list_launches_the_row_under_the_finger() {
        let (mut lib, l) = library();
        let all = lib.cells(&l)[ALL];
        assert_eq!(tap(&mut lib, &l, (all.x + 2) as f32, (all.y + 2) as f32), None);
        let (top, _) = lib.list_span(&l);
        let rows = lib.rows(&l).0;
        let (ry, rh, row) = rows.iter().copied().find(|r| matches!(r.2, Row::App(_))).unwrap();
        let Row::App(a) = row else { unreachable!() };
        assert_eq!(tap(&mut lib, &l, 100.0, (top + ry + rh / 2) as f32), Some(Out::Launch(a)));
    }

    #[test]
    fn the_search_field_asks_for_quicksearch() {
        let (mut lib, l) = library();
        let f = lib.field(&l);
        assert_eq!(tap(&mut lib, &l, (f.x + 10) as f32, (f.y + 10) as f32), Some(Out::Search));
    }

    #[test]
    fn a_vertical_drag_claims_the_touch_and_a_sideways_one_leaves_it_to_the_pager() {
        let (mut lib, l) = library();
        lib.down(&l, 600.0, 800.0, 0.0);
        assert!(lib.moved(600.0, 850.0, 10.0));
        assert!(lib.owns());
        assert_eq!(lib.up(&l, 600.0, 850.0, 20.0), None);
        lib.down(&l, 600.0, 800.0, 100.0);
        assert!(!lib.moved(700.0, 805.0, 110.0));
        assert!(!lib.owns());
    }
}
