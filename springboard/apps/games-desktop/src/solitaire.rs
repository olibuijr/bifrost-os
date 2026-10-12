//! Klondike Solitaire for the Games app: draw 1 or 3, tap to move, drag cards,
//! auto-complete and unlimited undo. State is memory only; the board is
//! dealt from the shared Games RNG.

use bifrost_app::kit::ui::{SPACE_L, SPACE_M, SPACE_S, SPACE_XS, SPACE_XXS};
use bifrost_app::kit::ui::{MAC_SUBHEAD_PT, MAC_TITLE1_PT};
use bifrost_app::kit::ui::{CARD_BG, FIELD_BG, LABEL, LABEL_2, SYSTEM_BLUE, SYSTEM_RED};
use bifrost_app::kit::{Event, Surface, TextCache, Ui, Weight};
use bifrost_core::random::{RandomSource, Xorshift64};


/// 0..52: suit = c / 13 (0 spades, 1 hearts, 2 diamonds, 3 clubs), rank = c % 13 (0 = ace).
fn suit(c: u8) -> usize {
    (c / 13) as usize
}
fn rank(c: u8) -> u8 {
    c % 13
}
fn red(c: u8) -> bool {
    matches!(suit(c), 1 | 2)
}

#[derive(Clone)]
struct State {
    stock: Vec<u8>,
    waste: Vec<u8>,
    found: [Vec<u8>; 4],
    tab: [Vec<(u8, bool)>; 7],
    moves: u32,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Src {
    Waste,
    Found(usize),
    Tab(usize, usize),
}

struct Drag {
    src: Src,
    cards: Vec<u8>,
    grab: (f32, f32),
    pos: (f32, f32),
    moved: bool,
}

pub struct Solitaire {
    st: State,
    undo: Vec<State>,
    pub draw3: bool,
    drag: Option<Drag>,
    down: (f32, f32),
    /// Top-left of the content area (pixels): right of the sidebar, under the toolbar.
    pub origin: (f32, f32),
    /// Window height in pixels; the card size fits it.
    pub height: f32,
}

impl Solitaire {
    pub fn new(seed: u32) -> Self {
        let mut s = Self {
            st: State { stock: vec![], waste: vec![], found: Default::default(), tab: Default::default(), moves: 0 },
            undo: vec![],
            draw3: false,
            drag: None,
            down: (0.0, 0.0),
            origin: (0.0, 0.0),
            height: f32::MAX,
        };
        s.deal(seed);
        s
    }

    pub fn deal(&mut self, seed: u32) {
        let mut deck: Vec<u8> = (0..52).collect();
        let mut rng = Xorshift64::new(seed as u64 | 1);
        for i in (1..52).rev() {
            let x = rng.next_u64();
            deck.swap(i, (x >> 16) as usize % (i + 1));
        }
        let mut st = State { stock: vec![], waste: vec![], found: Default::default(), tab: Default::default(), moves: 0 };
        for c in 0..7 {
            for _ in 0..=c {
                let card = deck.pop().unwrap();
                st.tab[c].push((card, false));
            }
            st.tab[c].last_mut().unwrap().1 = true;
        }
        st.stock = deck;
        self.st = st;
        self.undo.clear();
        self.drag = None;
    }

    pub fn won(&self) -> bool {
        self.st.found.iter().map(|f| f.len()).sum::<usize>() == 52
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn moves(&self) -> u32 {
        self.st.moves
    }

    pub fn describe(&self) -> String {
        format!(
            "games_sol_found={}\ngames_sol_moves={}\ngames_sol_draw={}\ngames_sol_won={}\n",
            self.st.found.iter().map(|f| f.len()).sum::<usize>(),
            self.st.moves,
            if self.draw3 { 3 } else { 1 },
            self.won()
        )
    }

    fn push_undo(&mut self) {
        self.undo.push(self.st.clone());
        if self.undo.len() > 300 {
            self.undo.remove(0);
        }
        self.st.moves += 1;
    }

    pub fn undo(&mut self) {
        if let Some(s) = self.undo.pop() {
            self.st = s;
        }
        self.drag = None;
    }

    #[cfg(test)]
    pub fn start_test_drag(&mut self) {
        self.drag = Some(Drag { src: Src::Waste, cards: vec![0], grab: (0.0, 0.0), pos: (0.0, 0.0), moved: true });
    }

    #[cfg(test)]
    pub fn is_dragging(&self) -> bool {
        self.drag.is_some()
    }

    /// Drop any in-flight card drag without moving the cards.
    pub fn cancel_drag(&mut self) {
        self.drag = None;
    }

    pub fn set_draw3(&mut self, d3: bool, seed: u32) {
        if self.draw3 != d3 {
            self.draw3 = d3;
            self.deal(seed);
        }
    }

    // ---- rules ----

    fn can_found(&self, c: u8, f: usize) -> bool {
        suit(c) == f && self.st.found[f].len() == rank(c) as usize
    }

    fn can_tab(&self, c: u8, col: usize) -> bool {
        match self.st.tab[col].last() {
            None => rank(c) == 12,
            Some(&(t, up)) => up && red(t) != red(c) && rank(t) == rank(c) + 1,
        }
    }

    fn cards(&self, src: Src) -> Vec<u8> {
        match src {
            Src::Waste => self.st.waste.last().copied().into_iter().collect(),
            Src::Found(f) => self.st.found[f].last().copied().into_iter().collect(),
            Src::Tab(c, i) => self.st.tab[c].get(i..).unwrap_or(&[]).iter().map(|x| x.0).collect(),
        }
    }

    fn take(&mut self, src: Src) {
        match src {
            Src::Waste => {
                self.st.waste.pop();
            }
            Src::Found(f) => {
                self.st.found[f].pop();
            }
            Src::Tab(c, i) => {
                self.st.tab[c].truncate(i);
                if let Some(l) = self.st.tab[c].last_mut() {
                    l.1 = true;
                }
            }
        }
    }

    fn to_found(&mut self, src: Src) -> bool {
        let cs = self.cards(src);
        if cs.len() != 1 || !self.can_found(cs[0], suit(cs[0])) {
            return false;
        }
        self.push_undo();
        self.take(src);
        self.st.found[suit(cs[0])].push(cs[0]);
        true
    }

    fn to_tab(&mut self, src: Src, col: usize) -> bool {
        let cs = self.cards(src);
        if cs.is_empty() || matches!(src, Src::Tab(c, _) if c == col) || !self.can_tab(cs[0], col) {
            return false;
        }
        self.push_undo();
        self.take(src);
        self.st.tab[col].extend(cs.into_iter().map(|c| (c, true)));
        true
    }

    fn auto_move(&mut self, src: Src) -> bool {
        if self.to_found(src) {
            return true;
        }
        (0..7).any(|c| self.to_tab(src, c))
    }

    fn draw_stock(&mut self) {
        if self.st.stock.is_empty() {
            if self.st.waste.is_empty() {
                return;
            }
            self.push_undo();
            let mut w = std::mem::take(&mut self.st.waste);
            w.reverse();
            self.st.stock = w;
        } else {
            self.push_undo();
            for _ in 0..if self.draw3 { 3 } else { 1 } {
                if let Some(c) = self.st.stock.pop() {
                    self.st.waste.push(c);
                }
            }
        }
    }

    /// Moves every card that can go to a foundation, repeatedly.
    pub fn auto_complete(&mut self) {
        loop {
            let mut srcs = vec![Src::Waste];
            srcs.extend((0..7).filter_map(|c| self.st.tab[c].len().checked_sub(1).map(|i| Src::Tab(c, i))));
            if !srcs.into_iter().any(|s| self.to_found(s)) {
                break;
            }
        }
    }

    /// True once only face-up cards remain on the tableau (the game is a certain win).
    pub fn can_auto(&self) -> bool {
        self.st.stock.is_empty() && self.st.waste.is_empty() && self.st.tab.iter().all(|c| c.iter().all(|x| x.1)) && !self.won()
    }

    // ---- geometry ----

    fn geo(&self, ui: &Ui, w: i32, h: i32) -> (f32, f32, f32, f32, f32, f32) {
        let m = ui.pt(SPACE_M);
        let g = ui.pt(SPACE_S);
        let _ = h;
        // `w` is the content width; cards also fit the height so a wide window does not push the tableau off
        let cw = ((w as f32 - 2.0 * m - 6.0 * g) / 7.0).min((self.height - self.origin.1) / 4.0 / 1.4).max(1.0);
        let ch = cw * 1.4;
        let m = self.origin.0 + (w as f32 - 7.0 * cw - 6.0 * g) / 2.0;
        let top = self.origin.1 + ui.pt(SPACE_L) * 2.0;
        let ty = top + ch + ui.pt(SPACE_L);
        (m, g, cw, ch, top, ty)
    }

    fn col_x(&self, ui: &Ui, w: i32, col: usize) -> f32 {
        let (m, g, cw, ..) = self.geo(ui, w, 0);
        m + col as f32 * (cw + g)
    }

    fn tab_ys(&self, ui: &Ui, w: i32, h: i32, col: usize) -> Vec<f32> {
        let (_, _, _, ch, _, ty) = self.geo(ui, w, h);
        let (dn, up) = (ch * 0.14, ch * 0.28);
        let cards = &self.st.tab[col];
        let n = cards.len();
        let avail = (h as f32 - ui.pt(SPACE_L) - ty - ch).max(1.0);
        let total: f32 = cards.iter().take(n.saturating_sub(1)).map(|c| if c.1 { up } else { dn }).sum();
        let k = if total > avail { avail / total } else { 1.0 };
        let mut y = ty;
        let mut v = Vec::with_capacity(n);
        for c in cards {
            v.push(y);
            y += (if c.1 { up } else { dn }) * k;
        }
        v
    }

    fn waste_x(&self, ui: &Ui, w: i32, k: usize, shown: usize) -> f32 {
        let (_, _, cw, ..) = self.geo(ui, w, 0);
        let _ = shown;
        self.col_x(ui, w, 1) + k as f32 * cw * 0.28
    }

    fn hit(&self, ui: &Ui, w: i32, h: i32, x: f32, y: f32) -> Option<(Src, f32, f32)> {
        let (_, _, cw, ch, top, _) = self.geo(ui, w, h);
        let inr = |rx: f32, ry: f32| x >= rx && x < rx + cw && y >= ry && y < ry + ch;
        if !self.st.waste.is_empty() {
            let shown = self.st.waste.len().min(if self.draw3 { 3 } else { 1 });
            let rx = self.waste_x(ui, w, shown - 1, shown);
            if inr(rx, top) {
                return Some((Src::Waste, rx, top));
            }
        }
        for f in 0..4 {
            let rx = self.col_x(ui, w, 3 + f);
            if !self.st.found[f].is_empty() && inr(rx, top) {
                return Some((Src::Found(f), rx, top));
            }
        }
        for c in 0..7 {
            let ys = self.tab_ys(ui, w, h, c);
            let rx = self.col_x(ui, w, c);
            for i in (0..ys.len()).rev() {
                if inr(rx, ys[i]) {
                    return if self.st.tab[c][i].1 { Some((Src::Tab(c, i), rx, ys[i])) } else { None };
                }
            }
        }
        None
    }

    // ---- input; returns true when the state changed ----

    pub fn touch(&mut self, ui: &Ui, ev: Event, w: i32, h: i32, seed: u32) -> bool {
        let _ = seed;
        match ev {
            Event::Down(x, y) => {
                self.down = (x, y);
                self.drag = None;
                if let Some((src, rx, ry)) = self.hit(ui, w, h, x, y) {
                    let cards = self.cards(src);
                    if !cards.is_empty() {
                        self.drag = Some(Drag { src, cards, grab: (x - rx, y - ry), pos: (rx, ry), moved: false });
                    }
                }
                false
            }
            Event::Move(x, y) => {
                if let Some(d) = &mut self.drag {
                    if d.moved || (x - self.down.0).abs().max((y - self.down.1).abs()) > ui.pt(SPACE_M) {
                        d.moved = true;
                        d.pos = (x - d.grab.0, y - d.grab.1);
                        return true;
                    }
                }
                false
            }
            Event::Up(x, y) => {
                let (_, _, cw, ch, top, _) = self.geo(ui, w, h);
                match self.drag.take() {
                    Some(d) if d.moved => {
                        let cx = d.pos.0 + cw / 2.0;
                        let cy = d.pos.1 + ch / 2.0;
                        let col = (0..7).min_by(|&a, &b| {
                            let da = (self.col_x(ui, w, a) + cw / 2.0 - cx).abs();
                            let db = (self.col_x(ui, w, b) + cw / 2.0 - cx).abs();
                            da.partial_cmp(&db).unwrap()
                        });
                        if let Some(col) = col {
                            if cy < top + ch + ui.pt(SPACE_S) {
                                if col >= 3 {
                                    self.to_found(d.src);
                                }
                            } else {
                                self.to_tab(d.src, col);
                            }
                        }
                    }
                    Some(d) => {
                        // A tap on a card: send it to a foundation or the first fitting pile.
                        if !matches!(d.src, Src::Found(_)) {
                            self.auto_move(d.src);
                        }
                    }
                    None => {
                        let rx = self.col_x(ui, w, 0);
                        if self.down.0 >= rx && self.down.0 < rx + cw && self.down.1 >= top && self.down.1 < top + ch && x >= rx && x < rx + cw && y >= top && y < top + ch {
                            self.draw_stock();
                        }
                    }
                }
                true
            }
            _ => false,
        }
    }

    // ---- drawing ----

    fn card(s: &mut Surface, ui: &Ui, cache: &mut TextCache, c: u8, x: f32, y: f32, cw: f32, ch: f32) {
        s.fill_rrect(x, y, cw, ch, ui.pt(SPACE_XS), CARD_BG.now());
        let col = if red(c) { SYSTEM_RED } else { LABEL.now() };
        let sym = ["\u{2660}", "\u{2665}", "\u{2666}", "\u{2663}"][suit(c)];
        let r = ["A", "2", "3", "4", "5", "6", "7", "8", "9", "10", "J", "Q", "K"][rank(c) as usize];
        let t = cache.get(ui, &format!("{r}{sym}"), MAC_SUBHEAD_PT, Weight::Bold, col);
        s.blit(t, (x + ui.pt(SPACE_XS)) as i32, (y + ui.pt(SPACE_XS)) as i32, 255, 255);
        let big = cache.get(ui, sym, MAC_TITLE1_PT, Weight::Regular, col);
        s.blit(big, (x + (cw - big.w as f32) / 2.0) as i32, (y + ch * 0.55 - big.h as f32 / 2.0) as i32, 255, 255);
    }

    fn back(s: &mut Surface, ui: &Ui, x: f32, y: f32, cw: f32, ch: f32) {
        s.fill_rrect(x, y, cw, ch, ui.pt(SPACE_XS), CARD_BG.now());
        s.fill_rrect(x + ui.pt(SPACE_XXS), y + ui.pt(SPACE_XXS), cw - ui.pt(SPACE_XS), ch - ui.pt(SPACE_XS), ui.pt(SPACE_XS), SYSTEM_BLUE);
    }

    fn slot(s: &mut Surface, ui: &Ui, x: f32, y: f32, cw: f32, ch: f32) {
        s.fill_rrect(x, y, cw, ch, ui.pt(SPACE_XS), FIELD_BG.now());
    }

    pub fn draw(&self, s: &mut Surface, ui: &Ui, cache: &mut TextCache, w: i32, h: i32) {
        let (_, _, cw, ch, top, ty) = self.geo(ui, w, h);
        let _ = ty;
        let dragging = self.drag.as_ref().filter(|d| d.moved);
        let hidden = |src: Src, i: usize| dragging.map_or(false, |d| match (d.src, src) {
            (Src::Tab(c, k), Src::Tab(c2, _)) => c == c2 && i >= k,
            (a, b) => a == b,
        });
        // stock
        let sx = self.col_x(ui, w, 0);
        if self.st.stock.is_empty() {
            Self::slot(s, ui, sx, top, cw, ch);
            let r = cache.get(ui, "\u{21BA}", MAC_TITLE1_PT, Weight::Regular, LABEL_2.now());
            s.blit(r, (sx + (cw - r.w as f32) / 2.0) as i32, (top + (ch - r.h as f32) / 2.0) as i32, 255, 255);
        } else {
            Self::back(s, ui, sx, top, cw, ch);
        }
        // waste
        let shown = self.st.waste.len().min(if self.draw3 { 3 } else { 1 });
        if shown == 0 {
            Self::slot(s, ui, self.col_x(ui, w, 1), top, cw, ch);
        }
        for k in 0..shown {
            let idx = self.st.waste.len() - shown + k;
            if k == shown - 1 && hidden(Src::Waste, 0) {
                if k == 0 {
                    Self::slot(s, ui, self.col_x(ui, w, 1), top, cw, ch);
                }
                continue;
            }
            Self::card(s, ui, cache, self.st.waste[idx], self.waste_x(ui, w, k, shown), top, cw, ch);
        }
        // foundations
        for f in 0..4 {
            let fx = self.col_x(ui, w, 3 + f);
            Self::slot(s, ui, fx, top, cw, ch);
            let n = self.st.found[f].len();
            let vis = if hidden(Src::Found(f), 0) { n.checked_sub(2) } else { n.checked_sub(1) };
            if let Some(i) = vis {
                Self::card(s, ui, cache, self.st.found[f][i], fx, top, cw, ch);
            } else {
                let sym = ["\u{2660}", "\u{2665}", "\u{2666}", "\u{2663}"][f];
                let t = cache.get(ui, sym, MAC_TITLE1_PT, Weight::Regular, LABEL_2.now());
                s.blit(t, (fx + (cw - t.w as f32) / 2.0) as i32, (top + (ch - t.h as f32) / 2.0) as i32, 255, 255);
            }
        }
        // tableau
        for c in 0..7 {
            let x = self.col_x(ui, w, c);
            Self::slot(s, ui, x, ty, cw, ch);
            let ys = self.tab_ys(ui, w, h, c);
            for (i, &(card, up)) in self.st.tab[c].iter().enumerate() {
                if hidden(Src::Tab(c, i), i) {
                    break;
                }
                if up {
                    Self::card(s, ui, cache, card, x, ys[i], cw, ch);
                } else {
                    Self::back(s, ui, x, ys[i], cw, ch);
                }
            }
        }
        if let Some(d) = dragging {
            let step = ch * 0.28;
            for (k, &c) in d.cards.iter().enumerate() {
                Self::card(s, ui, cache, c, d.pos.0, d.pos.1 + k as f32 * step, cw, ch);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every card 0..52 exactly once across stock, waste, foundations and tableau.
    fn all_cards(s: &Solitaire) -> Vec<u8> {
        let mut out: Vec<u8> = s.st.stock.iter().chain(&s.st.waste).copied().collect();
        out.extend(s.st.found.iter().flatten().copied());
        out.extend(s.st.tab.iter().flatten().map(|c| c.0));
        out.sort_unstable();
        out
    }

    #[test]
    fn a_deal_uses_the_whole_deck_with_seven_fanned_columns() {
        let s = Solitaire::new(7);
        assert_eq!(all_cards(&s), (0..52).collect::<Vec<u8>>());
        for (c, col) in s.st.tab.iter().enumerate() {
            assert_eq!(col.len(), c + 1);
            assert!(col.last().unwrap().1, "top card is face up");
            assert!(col[..c].iter().all(|x| !x.1), "cards below it are face down");
        }
        assert_eq!(s.st.stock.len(), 24);
    }

    #[test]
    fn the_same_seed_deals_the_same_board() {
        assert_eq!(Solitaire::new(42).st.stock, Solitaire::new(42).st.stock);
        assert_ne!(Solitaire::new(42).st.stock, Solitaire::new(44).st.stock);
    }

    #[test]
    fn drawing_one_moves_a_card_and_undo_restores_the_stock() {
        let mut s = Solitaire::new(42);
        let before = s.st.stock.clone();
        s.draw_stock();
        assert_eq!(s.st.waste.len(), 1);
        assert_eq!(s.moves(), 1);
        s.undo();
        assert!(s.st.waste.is_empty());
        assert_eq!(s.st.stock, before);
        assert_eq!(s.moves(), 0);
        assert!(!s.can_undo());
    }

    #[test]
    fn draw_three_turns_over_three_cards() {
        let mut s = Solitaire::new(42);
        s.set_draw3(true, 42);
        assert!(s.draw3);
        s.draw_stock();
        assert_eq!(s.st.waste.len(), 3);
        assert!(s.describe().contains("games_sol_draw=3\n"));
    }

    #[test]
    fn tableau_takes_a_king_into_an_empty_column_and_alternating_colours_down() {
        let mut s = Solitaire::new(1);
        s.st.tab[0].clear();
        assert!(s.can_tab(12, 0), "a king starts an empty column");
        assert!(!s.can_tab(11, 0), "only a king starts an empty column");
        // red queen of hearts face up; a black jack of spades can sit on it
        s.st.tab[1] = vec![(24, true)];
        assert!(s.can_tab(10, 1));
        assert!(!s.can_tab(23, 1), "same colour does not stack");
        s.st.tab[1] = vec![(24, false)];
        assert!(!s.can_tab(10, 1), "face-down cards cannot be built on");
    }

    #[test]
    fn foundations_build_up_from_the_ace_of_each_suit() {
        let mut s = Solitaire::new(3);
        s.st.waste = vec![0, 2];
        assert!(!s.to_found(Src::Waste), "the three of spades is not a legal start");
        assert_eq!(s.moves(), 0);
        s.st.waste = vec![0];
        assert!(s.to_found(Src::Waste));
        assert_eq!(s.st.found[0], vec![0]);
        assert_eq!(s.moves(), 1);
        assert!(s.st.waste.is_empty());
    }

    #[test]
    fn auto_complete_moves_every_playable_card_home() {
        let mut s = Solitaire::new(3);
        s.st.stock.clear();
        s.st.waste = vec![1, 0];
        for col in s.st.tab.iter_mut() {
            col.clear();
        }
        s.auto_complete();
        assert_eq!(s.st.found[0], vec![0, 1], "ace, then two, in order");
        assert!(s.st.waste.is_empty());
        assert!(!s.won(), "two cards of 52 is not a win");
    }

    #[test]
    fn a_full_set_of_foundations_is_a_win() {
        let mut s = Solitaire::new(3);
        s.st.stock.clear();
        s.st.waste.clear();
        s.st.tab = Default::default();
        s.st.found = [(0..13).collect(), (13..26).collect(), (26..39).collect(), (39..52).collect()];
        assert!(s.won());
        assert!(s.describe().contains("games_sol_won=true\n"));
    }
}
