//! The rating and review sheet of an installed app, opened by Rate on its product page. A card with five numbered
//! rating buttons (tap the chosen one again to clear it), a nickname field, a written review field, and Send and
//! Cancel. The rules are `bifrost_core::app_review::ReviewDraft`; this module is the card's geometry, its touches and
//! its typing, all pure and tested on the host. Send keeps the review in the store's `ReviewBook`
//! (`<root>/store-reviews.json`): the catalogue has no review endpoint yet, so nothing leaves the PC, and Rate opens the
//! sheet again with the saved review to edit.

use super::sheet::Rect;
use super::{cell, fill, label, AppStore};
use crate::gfx::{rgba, Surface};
use crate::shell::ui::{Ui, Weight, CAPSULE_RADIUS, SECONDARY, TINT, WHITE};
use crate::shell::Event;
use bifrost_core::app_review::{Error, Review, ReviewDraft, MAX_STARS};

/// The field the keyboard types into.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Field {
    Nickname,
    Body,
}

/// What a touch on the sheet decided.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReviewOut {
    /// Send was tapped on a valid review: keep it.
    Send(Review),
    Cancelled,
}

/// One open rating sheet.
pub struct ReviewSheet {
    pub id: String,
    pub name: String,
    draft: ReviewDraft,
    focus: Option<Field>,
    /// A line under the card's title: why Send did nothing.
    pub message: String,
}

impl ReviewSheet {
    /// Opens the sheet for app `id`, with `earlier` (the review kept before) to edit, or empty.
    pub fn open(id: &str, name: &str, earlier: Option<&Review>) -> Self {
        let draft = earlier.map(ReviewDraft::from_review).unwrap_or_default();
        ReviewSheet { id: id.to_string(), name: name.to_string(), draft, focus: None, message: String::new() }
    }

    pub fn stars(&self) -> u8 {
        self.draft.stars()
    }

    pub fn nickname(&self) -> &str {
        self.draft.nickname()
    }

    pub fn body(&self) -> &str {
        self.draft.body()
    }

    /// The field the keyboard types into, if one was tapped.
    pub fn focus(&self) -> Option<Field> {
        self.focus
    }

    /// Whether Send is enabled (a rating is chosen and the review is valid).
    pub fn can_send(&self) -> bool {
        self.draft.can_submit()
    }

    /// Types `text` into the field in focus; returns how many characters it took (0 with no field in focus).
    pub fn type_text(&mut self, text: &str) -> usize {
        match self.focus {
            Some(Field::Nickname) => self.draft.type_nickname(text),
            Some(Field::Body) => self.draft.type_body(text),
            None => 0,
        }
    }

    /// Removes the last character of the field in focus.
    pub fn backspace(&mut self) -> bool {
        match self.focus {
            Some(Field::Nickname) => self.draft.backspace_nickname(),
            Some(Field::Body) => self.draft.backspace_body(),
            None => false,
        }
    }

    /// A touch in the sheet's coordinates (pane pixels). Only a lifted touch counts. A tap on a rating button sets it, on a
    /// field puts the keyboard there, on Send sends a valid review, on Cancel closes the sheet; a tap elsewhere takes the
    /// keyboard away.
    pub fn touch(&mut self, ui: &Ui, w: i32, h: i32, ev: Event) -> Option<ReviewOut> {
        let Event::Up(x, y) = ev else { return None };
        if let Some(i) = rating_rects(ui, w, h).iter().position(|r| inside(*r, x, y)) {
            self.draft.tap_star(i as u8 + 1);
            self.message.clear();
            return None;
        }
        if inside(nickname_rect(ui, w, h), x, y) {
            self.focus = Some(Field::Nickname);
            return None;
        }
        if inside(body_rect(ui, w, h), x, y) {
            self.focus = Some(Field::Body);
            return None;
        }
        let (send, cancel) = buttons(ui, w, h);
        if inside(cancel, x, y) {
            return Some(ReviewOut::Cancelled);
        }
        if inside(send, x, y) {
            return self.send();
        }
        self.focus = None;
        None
    }

    fn send(&mut self) -> Option<ReviewOut> {
        match self.draft.submit() {
            Ok(_) => Some(ReviewOut::Send(self.draft.review())),
            Err(Error::Stars(_)) => {
                self.message = "Choose a rating to send.".to_string();
                None
            }
            Err(Error::NicknameTooLong(_) | Error::BodyTooLong(_)) => {
                self.message = "The review is too long.".to_string();
                None
            }
            Err(_) => None,
        }
    }
}

fn inside((x0, y0, w, h): Rect, x: f32, y: f32) -> bool {
    x >= x0 && x < x0 + w && y >= y0 && y < y0 + h
}

/// The card, centred in a `w` x `h` pane (a small pane shortens it).
pub fn card(ui: &Ui, w: i32, h: i32) -> Rect {
    let cw = (w as f32 - ui.pt(32.0)).min(ui.pt(360.0));
    let ch = ui.pt(340.0).min(h as f32 - ui.pt(16.0));
    ((w as f32 - cw) / 2.0, (h as f32 - ch) / 2.0, cw, ch)
}

/// The five rating buttons in a row under the title, centred; button `i` stands for `i + 1`.
pub fn rating_rects(ui: &Ui, w: i32, h: i32) -> [Rect; MAX_STARS as usize] {
    let (cx, cy, cw, _) = card(ui, w, h);
    let (size, gap) = (ui.pt(36.0), ui.pt(8.0));
    let row = MAX_STARS as f32 * size + (MAX_STARS as f32 - 1.0) * gap;
    let x0 = cx + (cw - row) / 2.0;
    let y = cy + ui.pt(64.0);
    std::array::from_fn(|i| (x0 + i as f32 * (size + gap), y, size, size))
}

/// The nickname field.
pub fn nickname_rect(ui: &Ui, w: i32, h: i32) -> Rect {
    let (cx, cy, cw, _) = card(ui, w, h);
    (cx + ui.pt(20.0), cy + ui.pt(120.0), cw - ui.pt(40.0), ui.pt(36.0))
}

/// The written review field.
pub fn body_rect(ui: &Ui, w: i32, h: i32) -> Rect {
    let (cx, cy, cw, _) = card(ui, w, h);
    (cx + ui.pt(20.0), cy + ui.pt(168.0), cw - ui.pt(40.0), ui.pt(96.0))
}

/// The Send and Cancel buttons along the card's bottom edge: (send, cancel).
pub fn buttons(ui: &Ui, w: i32, h: i32) -> (Rect, Rect) {
    let (cx, cy, cw, ch) = card(ui, w, h);
    let pad = ui.pt(20.0);
    let gap = ui.pt(12.0);
    let bw = (cw - 2.0 * pad - gap) / 2.0;
    let by = cy + ch - ui.pt(64.0);
    let bh = ui.pt(40.0);
    ((cx + pad, by, bw, bh), (cx + pad + bw + gap, by, bw, bh))
}

/// The lines of the written review that fit in its field: the last `max` lines of `lines`, so the end being typed stays in
/// view.
fn visible_tail(lines: Vec<String>, max: usize) -> Vec<String> {
    let skip = lines.len().saturating_sub(max);
    lines.into_iter().skip(skip).collect()
}

impl AppStore {
    /// The rating sheet: the card with its five rating buttons, the two fields, Send and Cancel.
    pub(super) fn draw_review(&mut self, s: &mut Surface, ui: &Ui) {
        let (w, h) = (self.w, self.h);
        let Some(open) = self.review.as_ref() else { return };
        let (title, stars, nickname, body, focus, message, can_send) =
            (format!("Rate {}", open.name), open.stars(), open.nickname().to_string(), open.body().to_string(), open.focus(), open.message.clone(), open.can_send());
        let (cx, cy, cw, ch) = card(ui, w, h);
        s.fill_rect(0, 0, w as usize, h as usize, rgba(0, 0, 0, 110));
        s.fill_rrect(cx, cy, cw, ch, ui.pt(13.0), cell());
        let mid = cx + cw / 2.0;
        let title = ui.fit(&title, 17.0, cw - ui.pt(40.0));
        let tw = ui.measure(&title, 17.0, Weight::Semibold);
        self.put(s, ui, &title, 17.0, Weight::Semibold, label(), false, mid - tw / 2.0, cy + ui.pt(28.0));
        if !message.is_empty() {
            let mw = ui.measure(&message, 13.0, Weight::Regular);
            self.put(s, ui, &message, 13.0, Weight::Regular, SECONDARY, false, mid - mw / 2.0, cy + ui.pt(48.0));
        }
        for (i, r) in rating_rects(ui, w, h).iter().enumerate() {
            let on = (i as u8) < stars;
            s.fill_rrect(r.0, r.1, r.2, r.3, r.2 / 2.0, if on { TINT } else { fill() });
            let n = (i + 1).to_string();
            let nw = ui.measure(&n, 15.0, Weight::Bold);
            self.put(s, ui, &n, 15.0, Weight::Bold, if on { WHITE } else { label() }, false, r.0 + (r.2 - nw) / 2.0, r.1 + r.3 / 2.0);
        }
        let (nick, body_box) = (nickname_rect(ui, w, h), body_rect(ui, w, h));
        for (r, field) in [(nick, Field::Nickname), (body_box, Field::Body)] {
            s.fill_rrect(r.0, r.1, r.2, r.3, ui.pt(8.0), fill());
            if focus == Some(field) {
                s.stroke_rrect(r.0, r.1, r.2, r.3, ui.pt(8.0), 2.0, TINT);
            }
        }
        let inner = nick.2 - ui.pt(24.0);
        if nickname.is_empty() {
            self.put(s, ui, "Nickname (optional)", 15.0, Weight::Regular, SECONDARY, false, nick.0 + ui.pt(12.0), nick.1 + nick.3 / 2.0);
        } else {
            let text = ui.fit(&nickname, 15.0, inner);
            self.put(s, ui, &text, 15.0, Weight::Regular, label(), false, nick.0 + ui.pt(12.0), nick.1 + nick.3 / 2.0);
        }
        if body.is_empty() {
            self.put(s, ui, "Review (optional)", 15.0, Weight::Regular, SECONDARY, false, body_box.0 + ui.pt(12.0), body_box.1 + ui.pt(18.0));
        } else {
            let max = (((body_box.3 - ui.pt(16.0)) / ui.pt(20.0)) as usize).max(1);
            for (i, line) in visible_tail(ui.wrap(&body, 15.0, Weight::Regular, inner), max).iter().enumerate() {
                self.put(s, ui, line, 15.0, Weight::Regular, label(), false, body_box.0 + ui.pt(12.0), body_box.1 + ui.pt(18.0) + i as f32 * ui.pt(20.0));
            }
        }
        let (send, cancel) = buttons(ui, w, h);
        s.fill_rrect(send.0, send.1, send.2, send.3, CAPSULE_RADIUS, if can_send { TINT } else { fill() });
        s.fill_rrect(cancel.0, cancel.1, cancel.2, cancel.3, CAPSULE_RADIUS, fill());
        for (text, r, ink) in [("Send", send, if can_send { WHITE } else { SECONDARY }), ("Cancel", cancel, TINT)] {
            let tw = ui.measure(text, 17.0, Weight::Semibold);
            self.put(s, ui, text, 17.0, Weight::Semibold, ink, false, r.0 + (r.2 - tw) / 2.0, r.1 + r.3 / 2.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_end_of_a_long_review_stays_in_view() {
        let lines: Vec<String> = (1..=6).map(|n| n.to_string()).collect();
        assert_eq!(visible_tail(lines.clone(), 4), ["3", "4", "5", "6"]);
        assert_eq!(visible_tail(lines.clone(), 10).len(), 6);
        assert_eq!(visible_tail(Vec::new(), 3), Vec::<String>::new());
    }

    fn ui() -> Ui {
        crate::shell::tests::ui()
    }

    fn tap(sheet: &mut ReviewSheet, ui: &Ui, r: Rect) -> Option<ReviewOut> {
        sheet.touch(ui, 800, 600, Event::Up(r.0 + r.2 / 2.0, r.1 + r.3 / 2.0))
    }

    #[test]
    fn the_parts_of_the_card_sit_inside_it_and_do_not_overlap() {
        let ui = ui();
        let (cx, cy, cw, ch) = card(&ui, 800, 600);
        let (send, cancel) = buttons(&ui, 800, 600);
        let mut parts: Vec<Rect> = rating_rects(&ui, 800, 600).to_vec();
        parts.extend([nickname_rect(&ui, 800, 600), body_rect(&ui, 800, 600), send, cancel]);
        for &(x, y, w, h) in &parts {
            assert!(x >= cx && x + w <= cx + cw && y >= cy && y + h <= cy + ch, "{:?} leaves the card", (x, y, w, h));
        }
        for (i, a) in parts.iter().enumerate() {
            for b in &parts[i + 1..] {
                let apart = a.0 + a.2 <= b.0 || b.0 + b.2 <= a.0 || a.1 + a.3 <= b.1 || b.1 + b.3 <= a.1;
                assert!(apart, "{a:?} overlaps {b:?}");
            }
        }
    }

    #[test]
    fn a_small_pane_shortens_the_card_and_keeps_it_inside() {
        let ui = ui();
        let (x, y, w, h) = card(&ui, 300, 260);
        assert!(x >= 0.0 && y >= 0.0 && x + w <= 300.0 && y + h <= 260.0);
    }

    #[test]
    fn tapping_a_rating_sets_it_and_tapping_it_again_clears_it() {
        let ui = ui();
        let mut s = ReviewSheet::open("is.o.chess", "Chess", None);
        assert_eq!(s.stars(), 0);
        assert_eq!(tap(&mut s, &ui, rating_rects(&ui, 800, 600)[3]), None);
        assert_eq!(s.stars(), 4);
        tap(&mut s, &ui, rating_rects(&ui, 800, 600)[1]);
        assert_eq!(s.stars(), 2);
        tap(&mut s, &ui, rating_rects(&ui, 800, 600)[1]);
        assert_eq!(s.stars(), 0, "the chosen rating tapped again clears it");
    }

    #[test]
    fn a_press_alone_does_nothing_only_a_lifted_touch_counts() {
        let ui = ui();
        let mut s = ReviewSheet::open("a", "A", None);
        let r = rating_rects(&ui, 800, 600)[0];
        assert_eq!(s.touch(&ui, 800, 600, Event::Down(r.0 + 1.0, r.1 + 1.0)), None);
        assert_eq!(s.touch(&ui, 800, 600, Event::Move(r.0 + 1.0, r.1 + 1.0)), None);
        assert_eq!(s.stars(), 0);
    }

    #[test]
    fn typing_goes_to_the_field_that_was_tapped() {
        let ui = ui();
        let mut s = ReviewSheet::open("a", "A", None);
        assert_eq!(s.type_text("lost"), 0, "no field is in focus yet");
        tap(&mut s, &ui, nickname_rect(&ui, 800, 600));
        assert_eq!(s.focus(), Some(Field::Nickname));
        s.type_text("Ana");
        tap(&mut s, &ui, body_rect(&ui, 800, 600));
        assert_eq!(s.focus(), Some(Field::Body));
        s.type_text("Great\napp");
        assert!(s.backspace());
        assert_eq!((s.nickname(), s.body()), ("Ana", "Great\nap"));
        // a tap beside every control takes the keyboard away
        s.touch(&ui, 800, 600, Event::Up(1.0, 1.0));
        assert_eq!(s.focus(), None);
        assert!(!s.backspace());
    }

    #[test]
    fn send_needs_a_rating_and_then_hands_back_the_review() {
        let ui = ui();
        let mut s = ReviewSheet::open("is.o.chess", "Chess", None);
        let (send, _) = buttons(&ui, 800, 600);
        assert!(!s.can_send());
        assert_eq!(tap(&mut s, &ui, send), None);
        assert_eq!(s.message, "Choose a rating to send.");
        tap(&mut s, &ui, rating_rects(&ui, 800, 600)[4]);
        assert!(s.message.is_empty(), "choosing a rating takes the message away");
        tap(&mut s, &ui, nickname_rect(&ui, 800, 600));
        s.type_text(" Ana ");
        assert!(s.can_send());
        let out = tap(&mut s, &ui, send);
        assert_eq!(out, Some(ReviewOut::Send(Review { stars: 5, nickname: " Ana ".into(), body: String::new() })));
        assert_eq!(tap(&mut s, &ui, send), None, "a second tap on Send sends nothing");
    }

    #[test]
    fn cancel_closes_the_sheet_without_a_review() {
        let ui = ui();
        let mut s = ReviewSheet::open("a", "A", None);
        tap(&mut s, &ui, rating_rects(&ui, 800, 600)[2]);
        let (_, cancel) = buttons(&ui, 800, 600);
        assert_eq!(tap(&mut s, &ui, cancel), Some(ReviewOut::Cancelled));
    }

    #[test]
    fn an_earlier_review_opens_for_editing() {
        let ui = ui();
        let before = Review { stars: 3, nickname: "Ana".into(), body: "fine".into() };
        let mut s = ReviewSheet::open("a", "A", Some(&before));
        assert_eq!((s.stars(), s.nickname(), s.body()), (3, "Ana", "fine"));
        assert!(s.can_send());
        tap(&mut s, &ui, body_rect(&ui, 800, 600));
        s.type_text(" and fast");
        let (send, _) = buttons(&ui, 800, 600);
        match tap(&mut s, &ui, send) {
            Some(ReviewOut::Send(r)) => assert_eq!((r.stars, r.body.as_str()), (3, "fine and fast")),
            other => panic!("{other:?}"),
        }
    }
}
