//! The desktop's notification banners (27-Golden-Gate-Notification-Center.png, "Backup Complete"): a tile of the
//! Notification Center column's width at the top right, under the menu bar, sliding in from the right edge, in the
//! scheme's material (light in Light, dark in Dark). Hovering it keeps it up and shows a circled close X at its top
//! left corner and an Options chevron at its right edge; the chevron opens a small menu of the notification's actions,
//! Mute and Notification Settings. A click on the card opens the app and clears that one notification; the X only puts
//! the banner away (the notification stays in Notification Center). A ringing Clock alert's banner carries its answers
//! as buttons in a column at its right edge, always shown (Stop and Repeat for a timer, Stop and Snooze for an alarm),
//! where other banners have the chevron. A banner that arrives while another is up takes
//! the front and the earlier one waits behind it, its edge showing under the card, and comes back when the front one
//! leaves. The tablet keeps its centred banner (`super::banner_rect`).

use super::super::clock::Answer;
use super::super::ui::{Weight, LABEL, MAC_BODY_PT, menu_highlight, MENU_HIGHLIGHT_TEXT, SEPARATOR_C};
use super::super::{AppId, Effect, Event, Shell};
use super::center::{NC_CARD_PT, NC_CARD_R_PT, NC_RIGHT_PT, NC_STACK_PT, NC_TOP_GAP_PT};
use super::desk_card::{self, within, CardText, R4, CONTROL_PT, PAD_PT};
use super::list::Mode;
use bifrost_ui::material_request::ShellSurface;
use crate::gfx::Surface;

/// How many earlier banners wait behind the front one (older ones stay in Notification Center only).
pub const MAX_BEHIND: usize = 2;
/// The stack's edges under the front banner: inset from the sides per level.
const PEEK_INSET_PT: f32 = 10.0;
/// The Options menu under the banner: width, row height, inner padding and the gap above it.
const MENU_W_PT: f32 = 220.0;
const MENU_ROW_PT: f32 = 24.0;
const MENU_PAD_PT: f32 = 5.0;
const MENU_GAP_PT: f32 = 6.0;
const MENU_R_PT: f32 = 10.0;
/// The width of the answer buttons' column at the right edge of a ringing Clock alert's banner.
const ANSWER_W_PT: f32 = 76.0;
/// The chrome overlay surface of the banner (above every window); its id is kept clear of the others in
/// `shell::chrome_ids`.
#[cfg_attr(not(feature = "desktop-wayland"), allow(dead_code))]
pub const BANNER_CHROME_ID: u32 = crate::shell::chrome_ids::BANNER;

/// A part of the desktop banner the pointer can be on.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Part {
    /// the card itself: a click opens the app
    Card,
    /// the circled X at the top left corner (shown while hovered)
    Close,
    /// the chevron at the right edge (shown while hovered): opens the Options menu
    Options,
    /// answer button `n` of a ringing Clock alert's banner (always shown, top to bottom)
    Answer(usize),
    /// row `n` of the open Options menu
    Menu(usize),
    /// the Options menu outside its rows
    MenuPad,
}

/// A row of the banner's Options menu.
#[derive(Clone, Debug, PartialEq)]
pub enum MenuItem {
    /// a ringing Clock alert's answer: Stop, Snooze or Repeat
    Alarm(Answer),
    /// one of the notification's category actions: its identifier and title
    Action(String, String),
    /// mutes the app (its notifications are still listed and badged)
    Mute,
    /// opens Settings > Notifications
    Settings,
}

impl Shell {
    /// The front banner's card at `t`: at rest under the menu bar, right-aligned with the Notification Center column,
    /// sliding in from (and out to) the right edge of the screen.
    /// It is not shown while Control Center, Notification Center or a menu-bar menu is open: those sit where it does.
    pub(super) fn desk_banner_rect(&self, t: f64) -> Option<R4> {
        let b = self.notify.banner.as_ref()?;
        if self.cc_active() || self.center_active() || self.desktop_menu.is_some() {
            return None;
        }
        let (x, y, w, h) = self.desk_banner_rest();
        let off = (1.0 - b.shown(t)) * (w + self.ui.pt(NC_RIGHT_PT) + self.ui.pt(24.0));
        Some((x + off, y, w, h))
    }

    /// Where the banner rests: the column's width and right margin, just under the menu bar.
    fn desk_banner_rest(&self) -> R4 {
        let ui = &self.ui;
        let w = self.center_w();
        (self.w as f32 - ui.pt(NC_RIGHT_PT) - w, self.desktop_bar_height() as f32 + ui.pt(NC_TOP_GAP_PT), w, ui.pt(NC_CARD_PT))
    }

    /// The circled close X over the card's top left corner.
    fn desk_banner_close(&self, (x, y, _, _): R4) -> R4 {
        let c = self.ui.pt(CONTROL_PT);
        let inset = self.ui.pt(4.0);
        (x - c / 2.0 + inset, y - c / 2.0 + inset, c, c)
    }

    /// The Options chevron at the card's right edge, centred on its height.
    fn desk_banner_chevron(&self, (x, y, w, h): R4) -> R4 {
        let c = self.ui.pt(CONTROL_PT);
        (x + w - self.ui.pt(PAD_PT) - c, y + (h - c) / 2.0, c, c)
    }

    /// The front banner's answer buttons: the ringing Clock alert's answers when the banner stands for it (Stop and
    /// Repeat for a timer, Stop and Snooze for an alarm, as the reference's Clock banner offers them; E2E N5, R6),
    /// else none.
    fn desk_banner_answers(&self) -> Vec<Answer> {
        match self.notify.banner.as_ref() {
            Some(b) if self.clock_banner(b.id) => self.clock.answers(),
            _ => Vec::new(),
        }
    }

    /// The answer buttons' column at the card's right edge, one row per answer, top to bottom.
    fn desk_banner_answer_rects(&self, (x, y, w, h): R4) -> Vec<R4> {
        let n = self.desk_banner_answers().len();
        if n == 0 {
            return Vec::new();
        }
        let cw = self.ui.pt(ANSWER_W_PT).min(w / 3.0);
        let row = h / n as f32;
        (0..n).map(|i| (x + w - cw, y + i as f32 * row, cw, row)).collect()
    }

    /// The Options menu's rows: the notification's actions, then Mute and Notification Settings.
    pub(super) fn desk_banner_menu(&self) -> Vec<MenuItem> {
        let Some(id) = self.notify.banner.as_ref().map(|b| b.id) else { return Vec::new() };
        let mut rows: Vec<MenuItem> = if self.clock_banner(id) {
            self.desk_banner_answers().into_iter().map(MenuItem::Alarm).collect()
        } else {
            self.usernotif.actions(id).into_iter().map(|a| MenuItem::Action(a.id, a.title)).collect()
        };
        rows.push(MenuItem::Mute);
        rows.push(MenuItem::Settings);
        rows
    }

    /// The text of a menu row.
    fn desk_banner_menu_title(&self, item: &MenuItem) -> String {
        match item {
            MenuItem::Action(_, title) => title.clone(),
            MenuItem::Alarm(answer) => answer.title().to_string(),
            MenuItem::Mute => {
                let app = self.notify.banner_card.as_ref().map(|c| c.app);
                match app {
                    Some(app) if self.app_muted(app) => format!("Unmute {}", desk_card::app_name(app)),
                    Some(app) => format!("Mute {}", desk_card::app_name(app)),
                    None => "Mute".to_string(),
                }
            }
            MenuItem::Settings => "Notification Settings\u{2026}".to_string(),
        }
    }

    /// The Options menu under the card (right-aligned with it) and its row rects.
    fn desk_banner_menu_rects(&self, (x, y, w, h): R4) -> (R4, Vec<R4>) {
        let ui = &self.ui;
        let n = self.desk_banner_menu().len();
        let (mw, row, pad) = (ui.pt(MENU_W_PT).min(w), ui.pt(MENU_ROW_PT), ui.pt(MENU_PAD_PT));
        let menu = (x + w - mw, y + h + ui.pt(MENU_GAP_PT), mw, n as f32 * row + 2.0 * pad);
        let rows = (0..n).map(|i| (menu.0 + pad, menu.1 + pad + i as f32 * row, mw - 2.0 * pad, row)).collect();
        (menu, rows)
    }

    /// The stack's edges under the card, one per waiting banner (none while the menu is open).
    fn desk_banner_peeks(&self, (x, y, w, h): R4) -> Vec<R4> {
        if self.notify.banner_menu {
            return Vec::new();
        }
        let (inset, peek) = (self.ui.pt(PEEK_INSET_PT), self.ui.pt(NC_STACK_PT));
        (1..=self.notify.banner_behind.len().min(MAX_BEHIND)).map(|k| (x + inset * k as f32, y + h + (k - 1) as f32 * peek, w - 2.0 * inset * k as f32, peek)).collect()
    }

    /// Everything of the banner drawn at `t`, which is also what takes the pointer: the card, the stack's edges, the X
    /// while hovered and the open menu.
    fn desk_banner_bounds(&self, t: f64) -> Option<R4> {
        let card = self.desk_banner_rect(t)?;
        let mut parts = vec![card];
        parts.extend(self.desk_banner_peeks(card));
        if self.notify.banner_hover.is_some() {
            parts.push(self.desk_banner_close(card));
        }
        if self.notify.banner_menu {
            parts.push(self.desk_banner_menu_rects(card).0);
        }
        let x0 = parts.iter().map(|r| r.0).fold(f32::MAX, f32::min);
        let y0 = parts.iter().map(|r| r.1).fold(f32::MAX, f32::min);
        let x1 = parts.iter().map(|r| r.0 + r.2).fold(f32::MIN, f32::max);
        let y1 = parts.iter().map(|r| r.1 + r.3).fold(f32::MIN, f32::max);
        Some((x0, y0, x1 - x0, y1 - y0))
    }

    /// The part of the banner under (x, y) at `t`. The X and the chevron exist only while the banner is hovered.
    pub(super) fn desk_banner_part(&self, x: f32, y: f32, t: f64) -> Option<Part> {
        if self.overview_active() {
            return None;
        }
        let card = self.desk_banner_rect(t)?;
        if self.notify.banner_menu {
            let (menu, rows) = self.desk_banner_menu_rects(card);
            if let Some(i) = rows.iter().position(|r| within(*r, x, y)) {
                return Some(Part::Menu(i));
            }
            if within(menu, x, y) {
                return Some(Part::MenuPad);
            }
        }
        let answers = self.desk_banner_answer_rects(card);
        if self.notify.banner_hover.is_some() {
            if within(self.desk_banner_close(card), x, y) {
                return Some(Part::Close);
            }
            // a ringing alert's buttons stand where the chevron would
            if answers.is_empty() && within(self.desk_banner_chevron(card), x, y) {
                return Some(Part::Options);
            }
        }
        // the answer buttons show without a hover: a click that comes straight there still answers the alert
        if let Some(i) = answers.iter().position(|r| within(*r, x, y)) {
            return Some(Part::Answer(i));
        }
        within(card, x, y).then_some(Part::Card)
    }

    /// The pointer moved to (x, y) without a button: the banner shows its controls and stays while hovered, and
    /// Notification Center's group headers offer Clear All.
    pub(in crate::shell) fn notify_hover(&mut self, x: f32, y: f32, t: f64) {
        if !self.desk_active() {
            return;
        }
        let part = self.desk_banner_part(x, y, t);
        if part != self.notify.banner_hover {
            self.notify.banner_hover = part;
            self.dirty = true;
        }
        if let Some(b) = &mut self.notify.banner {
            b.set_hovered(part.is_some(), t);
        }
        self.center_desk_hover(x, y);
    }

    /// Pointer presses on the desktop banner; None leaves the event to what lies under it (and closes an open menu).
    pub(super) fn desk_banner_event(&mut self, ev: Event, t: f64) -> Option<Vec<Effect>> {
        match ev {
            Event::Down(x, y) => {
                let part = self.desk_banner_part(x, y, t);
                if part.is_none() {
                    if std::mem::take(&mut self.notify.banner_menu) {
                        self.dirty = true;
                    }
                    return None;
                }
                self.notify.banner_press = part;
                Some(vec![])
            }
            Event::Move(..) => self.notify.banner_press.map(|_| vec![]),
            Event::Up(x, y) => {
                let press = self.notify.banner_press.take()?;
                let fx = if self.desk_banner_part(x, y, t) == Some(press) { self.desk_banner_act(press, t) } else { vec![] };
                self.dirty = true;
                Some(fx)
            }
            _ => None,
        }
    }

    /// Whether notification `id`'s banner stands for the Clock alert that rings now (E2E N5).
    fn clock_banner(&self, id: u64) -> bool {
        self.clock.alert().is_some() && self.clock_banners.contains(&id)
    }

    /// A click landed on `part` of the front banner. Returns the effects of answering a ringing alarm.
    fn desk_banner_act(&mut self, part: Part, t: f64) -> Vec<Effect> {
        let Some(id) = self.notify.banner.as_ref().map(|b| b.id) else { return vec![] };
        let app = self.notify.banner_card.as_ref().map(|c| c.app);
        let alarm = self.clock_banner(id);
        let mut fx = vec![];
        match part {
            Part::Card => {
                self.desk_banner_dismiss(t);
                // a click on a ringing alarm's banner stops it and opens the Clock
                if alarm {
                    fx = self.clock_banner_answer(id, Answer::Stop, t);
                }
                if let Some(app) = app {
                    self.open_from_notification(app, t);
                }
                // the reference clears the notification the banner showed, and only that one
                self.clear_notification(id);
            }
            Part::Close => {
                self.desk_banner_dismiss(t);
                // the X of a ringing alarm's banner stops it: an alert left ringing has nothing on screen to stop it by
                if alarm {
                    fx = self.clock_banner_answer(id, Answer::Stop, t);
                }
            }
            Part::Options => self.notify.banner_menu = !self.notify.banner_menu,
            Part::Answer(i) => {
                let answer = self.desk_banner_answers().get(i).copied();
                self.desk_banner_dismiss(t);
                if let Some(answer) = answer {
                    fx = self.clock_banner_answer(id, answer, t);
                    self.clear_notification(id);
                }
            }
            Part::MenuPad => {}
            Part::Menu(i) => {
                let item = self.desk_banner_menu().get(i).cloned();
                self.desk_banner_dismiss(t);
                match item {
                    Some(MenuItem::Alarm(answer)) => {
                        fx = self.clock_banner_answer(id, answer, t);
                        self.clear_notification(id);
                    }
                    Some(MenuItem::Action(action, _)) => {
                        self.usernotif_respond(id, &action, "", t);
                        self.clear_notification(id);
                    }
                    Some(MenuItem::Mute) => {
                        if let Some(app) = app {
                            self.toggle_mute(app);
                        }
                    }
                    Some(MenuItem::Settings) => {
                        self.desk_open_or_start(AppId::Settings, t);
                        let ui = &self.ui;
                        if let Some(st) = self.settings.as_mut() {
                            let _ = st.open_page(ui, "Notifications", t);
                        }
                    }
                    None => {}
                }
            }
        }
        fx
    }

    /// Puts the front banner away (its notification stays listed); the next one waiting behind comes when it is gone.
    fn desk_banner_dismiss(&mut self, t: f64) {
        if let Some(b) = &mut self.notify.banner {
            b.dismiss(t);
        }
        self.notify.banner_menu = false;
        self.notify.banner_hover = None;
    }

    /// The front banner has gone: the newest banner waiting behind it comes to the front with a fresh rest, unless its
    /// notification was cleared meanwhile or something now covers the desktop (Overview, a panel).
    pub(super) fn desk_banner_next(&mut self) {
        if !self.desk_active() {
            return;
        }
        if self.overview_active() || self.cc_active() || self.center_active() || self.is_locked() {
            self.notify.banner_behind.clear();
            return;
        }
        while let Some((id, app)) = self.notify.banner_behind.pop() {
            if self.notify_exists(id) {
                self.start_banner(id, app);
                self.dirty = true;
                return;
            }
        }
    }

    /// The banner's chrome overlay surface (`BANNER_CHROME_ID`): the strip from where the banner rests to the right
    /// edge of the screen (the slide plays inside it), tall enough for the stack's edges and the menu, with room for
    /// the shadow; and the part of it that takes the pointer (surface px). None while no banner shows.
    #[cfg_attr(not(feature = "desktop-wayland"), allow(dead_code))] // read by the desktop chrome export (desktop_bar.rs)
    pub(in crate::shell) fn desk_banner_overlay(&self) -> Option<((usize, usize, usize, usize), [i32; 4])> {
        if !self.desk_active() || self.overview_active() {
            return None;
        }
        let t = self.notify.t;
        let input = self.desk_banner_bounds(t)?;
        let rest = self.desk_banner_rest();
        let g = self.ui.pt(super::super::desk::SHADOW_REACH_PT);
        let menu_h = if self.notify.banner_menu { self.desk_banner_menu_rects(rest).0 .3 + self.ui.pt(MENU_GAP_PT) } else { 0.0 };
        let below = menu_h.max(MAX_BEHIND as f32 * self.ui.pt(NC_STACK_PT));
        let bar = self.desktop_bar_height() as f32;
        let x = (rest.0 - g).max(0.0);
        let y = (rest.1 - g).max(bar);
        let w = self.w as f32 - x;
        let h = rest.1 + rest.3 + below + g - y;
        let (x, y, w, h) = (x as usize, y as usize, w.max(0.0) as usize, h.max(0.0) as usize);
        let rel = |v: f32, o: usize| (v - o as f32).floor() as i32;
        let region = [rel(input.0, x), rel(input.1, y), input.2.ceil() as i32 + 1, input.3.ceil() as i32 + 1];
        Some(((x, y, w, h), region))
    }

    /// Leaves only the part of an exported banner strip (`desk_banner_overlay`, `w` x `h` px) that the banner covers
    /// (`region`, surface px): the rest of the strip is the desktop under it, which must not show above the windows.
    #[cfg_attr(not(feature = "desktop-wayland"), allow(dead_code))]
    pub(in crate::shell) fn desk_banner_mask(data: &mut [u32], w: usize, h: usize, [rx, ry, rw, rh]: [i32; 4]) {
        for row in 0..h {
            let inside_y = (row as i32) >= ry && (row as i32) < ry + rh;
            for col in 0..w {
                if !(inside_y && (col as i32) >= rx && (col as i32) < rx + rw) {
                    if let Some(p) = data.get_mut(row * w + col) {
                        *p = 0;
                    }
                }
            }
        }
    }

    /// Draws the front banner (with the stack's edges, its hover controls and the Options menu) and remembers that it
    /// did, so the frame repaints under it once it is gone.
    pub(super) fn draw_desk_banner(&mut self, s: &mut Surface, t: f64) {
        let rect = self.desk_banner_rect(t).filter(|_| !self.overview_active());
        self.notify.banner_drawn = rect.is_some();
        let Some(card) = rect else { return };
        let Some(n) = self.notify.banner.as_ref().and_then(|b| self.notify.store.get(b.id)).cloned() else { return };
        self.desk_notification_icon(n.app);
        if bifrost_render::glass::gpu() {
            // the banner floats over the live desktop on its own surface: clear what it covers for the glass, as the
            // column does (only its own bounds, so the desktop's icons around it stay)
            if let Some((x, y, w, h)) = self.desk_banner_bounds(t) {
                let (x0, y0) = (x.floor().max(0.0), y.floor().max(self.desktop_bar_height() as f32));
                let (x1, y1) = ((x + w).ceil().min(self.w as f32), (y + h).ceil().min(self.h as f32));
                if x1 > x0 && y1 > y0 {
                    s.fill_rect(x0 as usize, y0 as usize, (x1 - x0) as usize, (y1 - y0) as usize, 0);
                }
            }
        }
        let ui = &self.ui;
        let r = ui.pt(NC_CARD_R_PT);
        for (k, peek) in self.desk_banner_peeks(card).into_iter().enumerate() {
            desk_card::tile(s, ShellSurface::NotificationBanner, peek, peek.3 / 2.0, 255 - 70 * (k as u32 + 1));
        }
        desk_card::tile(s, ShellSurface::NotificationBanner, card, r, 255);
        let hovered = self.notify.banner_hover.is_some();
        let name = desk_card::app_name(n.app);
        let (title, body) = if self.previews_hidden(n.app, Mode::Center) { (name.clone(), "Notification".to_string()) } else { (n.title.clone(), n.body.clone()) };
        let age = super::store::age_text(n.time, self.notify.now());
        let icon = self.notify.center.icons.iter().find(|(a, _)| *a == n.app).and_then(|(_, i)| i.as_ref());
        let text = CardText { head: &name, age: (!hovered).then_some(age.as_str()), title: &title, body: &body };
        let answers = self.desk_banner_answers();
        let answer_rects = self.desk_banner_answer_rects(card);
        let trailing = answer_rects.first().map_or(ui.pt(CONTROL_PT), |r| r.2 - ui.pt(PAD_PT) + ui.pt(8.0));
        desk_card::draw_text(s, ui, card, icon, &text, trailing, 255);
        if let Some(first) = answer_rects.first() {
            // the buttons' column: a hairline on its left and between its rows, a centred title in each
            let hair = ui.pt(0.5).max(1.0);
            s.fill_rrect(first.0, card.1, hair, card.3, 0.0, SEPARATOR_C.now());
            for (i, (answer, r)) in answers.iter().zip(&answer_rects).enumerate() {
                if i > 0 {
                    s.fill_rrect(r.0, r.1, r.2, hair, 0.0, SEPARATOR_C.now());
                }
                let label = ui.text(&ui.fit(answer.title(), MAC_BODY_PT, r.2 - ui.pt(8.0)), MAC_BODY_PT, Weight::Regular, LABEL.now());
                s.blit(&label, (r.0 + (r.2 - label.w as f32) / 2.0) as i32, (r.1 + (r.3 - label.h as f32) / 2.0) as i32, 255, 255);
            }
        }
        if hovered {
            desk_card::close_disc(s, ui, self.desk_banner_close(card), 255);
            if answer_rects.is_empty() {
                desk_card::chevron_disc(s, ui, self.desk_banner_chevron(card), 255);
            }
        }
        if self.notify.banner_menu {
            let (menu, rows) = self.desk_banner_menu_rects(card);
            desk_card::tile(s, ShellSurface::Popover, menu, ui.pt(MENU_R_PT), 255);
            for (i, (item, row)) in self.desk_banner_menu().iter().zip(rows).enumerate() {
                let lit = self.notify.banner_hover == Some(Part::Menu(i));
                if lit {
                    s.fill_rrect(row.0, row.1, row.2, row.3, ui.pt(5.0), menu_highlight());
                }
                let ink = if lit { MENU_HIGHLIGHT_TEXT } else { LABEL.now() };
                let label = ui.text(&ui.fit(&self.desk_banner_menu_title(item), MAC_BODY_PT, row.2 - ui.pt(16.0)), MAC_BODY_PT, Weight::Regular, ink);
                s.blit(&label, (row.0 + ui.pt(8.0)) as i32, (row.1 + (row.3 - label.h as f32) / 2.0) as i32, 255, 255);
            }
        }
    }
}

#[cfg(all(test, feature = "desktop-wayland"))]
mod tests {
    use super::*;
    use crate::shell::notify::Notification;

    fn desk() -> Shell {
        let mut sh = crate::shell::tests::desktop_shell();
        sh.screen = crate::shell::Screen::Home;
        sh.notify_tick(500.0);
        sh
    }

    fn note(app: AppId, title: &str) -> Notification {
        Notification::new(app, title, "Something happened", 0)
    }

    fn tap(sh: &mut Shell, (x, y): (f32, f32), t: f64) {
        sh.event(Event::Down(x, y), t);
        sh.event(Event::Up(x, y), t + 30.0);
    }

    fn mid((x, y, w, h): R4) -> (f32, f32) {
        (x + w / 2.0, y + h / 2.0)
    }

    /// E2E W5-5: the desktop banner rests at the top right under the menu bar, right-aligned with the Notification
    /// Center column, not centred like the tablet's.
    #[test]
    fn the_desktop_banner_rests_at_the_top_right_under_the_menu_bar() {
        let mut sh = desk();
        sh.notify(note(AppId::Files, "Backup Complete"));
        // mid-slide (the banner is still coming in): it is off to the right of where it rests
        let (sx, _, _, _) = sh.banner_rect(500.0 + super::super::banner::SLIDE_MS / 4.0).expect("a banner sliding in");
        sh.notify_tick(500.0 + super::super::banner::SLIDE_MS);
        let (x, y, w, h) = sh.banner_rect(2000.0).expect("a banner");
        let right = sh.w as f32 - sh.ui.pt(NC_RIGHT_PT);
        assert!((x + w - right).abs() < 0.5, "right-aligned with the column");
        assert!(x > sh.w as f32 / 2.0, "on the right half, not centred");
        assert!(y >= sh.desktop_bar_height() as f32, "under the menu bar");
        assert!(h > 0.0 && (w - sh.center_w()).abs() < 0.5);
        assert!(sx > x, "it slides in from the right edge");
    }

    /// E2E W5-5: hovering shows the close X and the Options chevron and keeps the banner up; the X puts it away and
    /// leaves the notification in Notification Center.
    #[test]
    fn hover_shows_the_close_x_and_options_and_the_x_only_puts_the_banner_away() {
        let mut sh = desk();
        sh.notify(note(AppId::Files, "Backup Complete"));
        let t = 500.0 + super::super::banner::SLIDE_MS;
        sh.notify_tick(t);
        let card = sh.banner_rect(t).unwrap();
        assert_eq!(sh.desk_banner_part(mid(sh.desk_banner_chevron(card)).0, mid(card).1, t), Some(Part::Card), "no chevron before the pointer comes");
        sh.notify_hover(mid(card).0, mid(card).1, t);
        assert_eq!(sh.notify.banner_hover, Some(Part::Card));
        assert_eq!(sh.desk_banner_part(mid(sh.desk_banner_chevron(card)).0, mid(sh.desk_banner_chevron(card)).1, t), Some(Part::Options));
        assert_eq!(sh.desk_banner_part(mid(sh.desk_banner_close(card)).0, mid(sh.desk_banner_close(card)).1, t), Some(Part::Close));
        sh.notify_tick(t + 3.0 * super::super::banner::HOLD_MS);
        assert!(sh.notify.banner().is_some_and(|b| b.phase() == super::super::banner::Phase::Hold), "a hovered banner stays");
        let close = mid(sh.desk_banner_close(card));
        tap(&mut sh, close, t + 20_000.0);
        assert_eq!(sh.notify.banner().map(|b| b.phase()), Some(super::super::banner::Phase::Out));
        assert_eq!(sh.notify.store().len(), 1, "the notification stays in Notification Center");
    }

    /// E2E W5-5: the chevron opens the Options menu (Mute and Notification Settings at least); Mute mutes the app.
    #[test]
    fn the_options_chevron_opens_a_menu_whose_mute_row_mutes_the_app() {
        let mut sh = desk();
        sh.notify(note(AppId::Files, "Backup Complete"));
        let t = 500.0 + super::super::banner::SLIDE_MS;
        sh.notify_tick(t);
        let card = sh.banner_rect(t).unwrap();
        sh.notify_hover(mid(card).0, mid(card).1, t);
        let chevron = mid(sh.desk_banner_chevron(card));
        tap(&mut sh, chevron, t + 10.0);
        assert!(sh.notify.banner_menu, "the menu is open");
        let items = sh.desk_banner_menu();
        assert_eq!(&items[items.len() - 2..], &[MenuItem::Mute, MenuItem::Settings]);
        let (_, rows) = sh.desk_banner_menu_rects(card);
        let mute = items.iter().position(|i| *i == MenuItem::Mute).unwrap();
        assert_eq!(sh.desk_banner_part(mid(rows[mute]).0, mid(rows[mute]).1, t + 100.0), Some(Part::Menu(mute)));
        tap(&mut sh, mid(rows[mute]), t + 100.0);
        assert!(sh.app_muted(AppId::Files));
        assert!(!sh.notify.banner_menu);
    }

    /// E2E N5: on the desktop a ringing Clock alert is its banner; the Options menu offers Stop (and Snooze for an
    /// alarm that allows it), and Stop answers the alert and clears the notification.
    #[test]
    fn a_ringing_alert_s_banner_offers_stop_which_stops_it() {
        let mut sh = desk();
        let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/test-clock-banner").join(std::process::id().to_string());
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        sh.set_wall(1_000_000, 0, 1000.0);
        sh.clock.load(&dir.join("clock.ini").to_string_lossy(), 1000.0);
        assert!(sh.clock.start_timer("", 1000, 1000.0).is_some());
        sh.tick(2100.0);
        assert!(sh.clock.alert().is_some(), "the timer rings");
        // the notification `notify_alarm` files under the installed Clock app and its banner (`clock_banner_sync`); no
        // Clock app is installed in tests, so both are made by hand
        let id = sh.notify_id(note(AppId::Files, "Timer"));
        sh.clock_banners.push(id);
        sh.start_banner(id, AppId::Files);
        assert_eq!(sh.notify.banner().map(|b| b.id), Some(id));
        let items = sh.desk_banner_menu();
        assert_eq!(items, vec![MenuItem::Alarm(Answer::Stop), MenuItem::Alarm(Answer::Repeat), MenuItem::Mute, MenuItem::Settings], "a timer has Stop and Repeat, no Snooze");
        assert_eq!(sh.desk_banner_menu_title(&items[0]), "Stop");
        assert_eq!(sh.desk_banner_menu_title(&items[1]), "Repeat");
        sh.desk_banner_act(Part::Menu(0), 2200.0);
        assert!(sh.clock.alert().is_none(), "Stop answered the alert");
        assert!(sh.clock_banners.is_empty());
        assert!(!sh.notify_exists(id), "and cleared its notification");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// E2E R6: a ringing timer's banner shows Stop and Repeat as buttons without a hover (no chevron that would only
    /// put it away); Repeat answers the alert, clears its notification and runs the timer again.
    #[test]
    fn a_ringing_timer_s_banner_buttons_stop_and_repeat_it() {
        let mut sh = desk();
        let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/test-clock-banner-repeat").join(std::process::id().to_string());
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        sh.set_wall(1_000_000, 0, 1000.0);
        sh.clock.load(&dir.join("clock.ini").to_string_lossy(), 1000.0);
        assert!(sh.clock.start_timer("Tea", 1000, 1000.0).is_some());
        sh.tick(2100.0);
        assert!(sh.clock.alert().is_some(), "the timer rings");
        let id = sh.notify_id(note(AppId::Files, "Tea"));
        sh.clock_banners.push(id);
        sh.start_banner(id, AppId::Files);
        let t = 2100.0 + super::super::banner::SLIDE_MS;
        let card = sh.desk_banner_rect(t).expect("the banner shows");
        let buttons = sh.desk_banner_answer_rects(card);
        assert_eq!(sh.desk_banner_answers(), vec![Answer::Stop, Answer::Repeat]);
        assert_eq!(buttons.len(), 2);
        assert!(buttons.iter().all(|b| b.0 + b.2 <= card.0 + card.2 + 0.5 && b.0 > card.0 + card.2 / 2.0), "a column at the right edge");
        // no hover first: the click lands on Repeat, not on a hidden chevron or the card
        assert_eq!(sh.notify.banner_hover, None);
        let repeat = mid(buttons[1]);
        assert_eq!(sh.desk_banner_part(repeat.0, repeat.1, t), Some(Part::Answer(1)));
        sh.notify_hover(repeat.0, repeat.1, t);
        let chevron = mid(sh.desk_banner_chevron(card));
        assert!(matches!(sh.desk_banner_part(chevron.0, chevron.1, t), Some(Part::Answer(_))), "no chevron over the buttons");
        sh.desk_banner_act(Part::Answer(1), t + 10.0);
        assert!(sh.clock.alert().is_none(), "Repeat answered the alert");
        assert!(!sh.notify_exists(id), "and cleared its notification");
        let timers = &sh.clock.timers().list;
        assert_eq!(timers.len(), 1, "the timer runs again");
        assert_eq!((timers[0].label.as_str(), timers[0].duration_ms, timers[0].running()), ("Tea", 1000, true));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// E2E W5-5: a click on the banner opens the app and clears only the notification it showed.
    #[test]
    fn a_click_on_the_banner_clears_only_its_notification() {
        let mut sh = desk();
        sh.notify(note(AppId::Files, "First"));
        sh.notify(note(AppId::Files, "Second"));
        let t = 500.0 + super::super::banner::SLIDE_MS;
        sh.notify_tick(t);
        let card = sh.banner_rect(t).unwrap();
        tap(&mut sh, mid(card), t + 10.0);
        assert_eq!(sh.notify.store().len(), 1, "the other notification of the app stays");
        assert_eq!(sh.notify.store().groups()[0].latest().title, "First");
    }

    /// E2E W5-5: a second notification while one banner is up takes the front, the first waits behind it (its edge
    /// shows), and comes back once the front one has gone.
    #[test]
    fn banners_stack_and_the_one_behind_comes_back() {
        use super::super::banner::{HOLD_MS, SLIDE_MS};
        let mut sh = desk();
        sh.notify(note(AppId::Files, "First"));
        sh.notify_tick(1000.0);
        sh.notify(note(AppId::Terminal, "Second"));
        let front = sh.notify.banner().unwrap().id;
        assert_eq!(sh.notify.banner_behind.len(), 1, "the first waits behind");
        let card = sh.desk_banner_rest();
        assert_eq!(sh.desk_banner_peeks(card).len(), 1, "its edge shows under the front banner");
        let end = 1000.0 + 2.0 * SLIDE_MS + HOLD_MS + 10.0;
        sh.notify_tick(end - SLIDE_MS - 5.0);
        sh.notify_tick(end);
        let next = sh.notify.banner().expect("the first comes back");
        assert_ne!(next.id, front);
        assert!(sh.notify.banner_behind.is_empty());
        // the queue never grows past MAX_BEHIND
        for i in 0..5 {
            sh.notify(note(AppId::Files, &format!("More {i}")));
        }
        assert_eq!(sh.notify.banner_behind.len(), MAX_BEHIND);
    }

    /// E2E W5-5: the banner is its own surface above the windows while it shows, and the surface goes with it.
    #[test]
    fn the_banner_has_an_overlay_surface_only_while_it_shows() {
        let mut sh = desk();
        assert!(sh.desk_banner_overlay().is_none());
        sh.notify(note(AppId::Files, "Backup Complete"));
        sh.notify_tick(500.0 + super::super::banner::SLIDE_MS);
        let ((x, y, w, h), input) = sh.desk_banner_overlay().expect("an overlay");
        assert!(x + w <= sh.w as usize && y >= sh.desktop_bar_height() as usize && h > 0);
        assert!(input[2] > 0 && input[3] > 0 && input[0] >= 0 && input[1] >= 0, "the card takes the pointer");
        sh.notify.banner = None;
        assert!(sh.desk_banner_overlay().is_none());
    }

    /// E2E W5-8: the menu bar shows the Focus moon, right beside Control Center, only while a Focus is on; a press
    /// on it opens Control Center (where the Focus module is).
    #[test]
    fn the_focus_moon_shows_in_the_menu_bar_while_focus_is_on() {
        use crate::shell::desktop_bar::Menu;
        let mut sh = desk();
        sh.w = 1920;
        assert!(sh.menu_buttons().iter().all(|b| b.0 != Menu::Focus), "no moon while Focus is off");
        sh.set_dnd(true);
        let buttons = sh.menu_buttons();
        let focus = buttons.iter().position(|b| b.0 == Menu::Focus).expect("the moon");
        assert_eq!(buttons.get(focus + 1).map(|b| b.0), Some(Menu::ControlCenter), "right beside Control Center");
        let (_, x, w, _) = buttons[focus].clone();
        sh.event(Event::Down(x + w / 2.0, sh.desktop_bar_height() as f32 / 2.0), 1000.0);
        assert!(sh.cc_active(), "Control Center opens");
        assert_eq!(sh.desktop_menu, None, "the moon has no menu of its own");
        sh.set_dnd(false);
        assert!(sh.menu_buttons().iter().all(|b| b.0 != Menu::Focus));
    }

    #[test]
    fn the_exported_strip_keeps_only_the_banner() {
        let mut px = vec![7u32; 4 * 3];
        Shell::desk_banner_mask(&mut px, 4, 3, [1, 1, 2, 1]);
        assert_eq!(px, vec![0, 0, 0, 0, 0, 7, 7, 0, 0, 0, 0, 0]);
    }

    /// E2E W5-5: the tablet keeps its centred banner.
    #[test]
    fn the_tablet_banner_stays_centred() {
        let mut sh = crate::shell::tests::shell();
        crate::shell::tests::unlock(&mut sh);
        sh.tick(1000.0);
        sh.notify(note(AppId::Files, "Timer"));
        sh.tick(1000.0 + super::super::banner::SLIDE_MS);
        let (x, _, w, _) = sh.banner_rect(2000.0).unwrap();
        assert!((x + w / 2.0 - sh.w as f32 / 2.0).abs() <= 1.0);
    }
}
