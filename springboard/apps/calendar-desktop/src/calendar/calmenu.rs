//! The Calendar pop-up menu (Golden Gate Calendar): the calendar row of the quick-event popover and of the event form
//! open a pop-up menu of the calendars (`NativePopUpButton`), each with its colour dot and a check mark on the current
//! one. The menu is the kit's (`controls::menu::Menu::popup`, drawn by `widgets::menu` with `MenuItem::swatch`) and is
//! placed by `menu::popup_origin`, so the current calendar's row sits over the button. A click on a row picks that
//! calendar and a click outside closes the menu; Up, Down, type-to-select and Return work as in any pop-up menu, and
//! Escape closes it.
use super::*;
use bifrost_app::controls::menu::{self, Menu, MenuItem};
use bifrost_app::controls::{Node, Rect, Role};
use bifrost_app::widgets::{self, MENU_PAD_PT, MENU_ROW_PT, MENU_SEPARATOR_PT, MENU_SWATCH_GAP_PT, MENU_SWATCH_PT};

/// Accessibility ids of the menu's rows (clear of the search results and the popover at 400..).
const NODE: u64 = 600;
/// The narrowest menu, and the room a row takes besides its title: the check mark column, the swatch, the insets.
const MIN_W: f32 = 160.0;
const ROW_CHROME: f32 = 24.0 + MENU_SWATCH_PT + MENU_SWATCH_GAP_PT + 2.0 * 5.0 + 20.0;

/// Which pop-up button opened the menu: what a pick changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum MenuFor {
    /// the quick-event popover's calendar
    Popover,
    /// the event form's Calendar row
    Form,
}

/// The open menu of calendars.
pub(super) struct CalMenu {
    pub menu: Menu,
    /// The calendar (`extras.rs` id) of each row.
    pub ids: Vec<u32>,
    /// Where the menu is, in window points.
    pub rect: Rect,
    pub from: MenuFor,
}

impl Calendar {
    /// Opens the menu of calendars over `button` (window points), with `current` checked and highlighted.
    pub(super) fn open_cal_menu(&mut self, ui: &Ui, button: Rect, current: u32, from: MenuFor) {
        let choices = self.extras.choices();
        if choices.is_empty() {
            return;
        }
        let selected = choices.iter().position(|(id, _)| *id == current).unwrap_or(0);
        let k = ui.pt(1.0);
        let widest = choices.iter().map(|(_, name)| ui.measure(name, MAC_BODY_PT, Weight::Regular) / k).fold(0.0, f32::max);
        let items: Vec<MenuItem> = choices.iter().enumerate().map(|(i, (id, name))| MenuItem::new(i as u32 + 1, name).swatch(self.calendar_colour(*id))).collect();
        let menu = Menu::popup(items, selected);
        let (rows, h) = menu.rows(MENU_ROW_PT, MENU_SEPARATOR_PT, MENU_PAD_PT);
        let bounds = Rect { x: 0.0, y: 0.0, w: self.full_w as f32 / k, h: self.h as f32 / k };
        let w = (widest + ROW_CHROME).max(button.w).max(MIN_W).min(bounds.w);
        let top = rows.get(selected).map_or(0.0, |r| r.0);
        // the menu's selected row lines up with the button's text
        let (x, y) = menu::popup_origin(Rect { y: button.y - (MENU_ROW_PT - button.h) / 2.0, ..button }, w, h, top, bounds);
        self.cal_menu = Some(CalMenu { menu, ids: choices.into_iter().map(|(id, _)| id).collect(), rect: Rect { x, y, w, h }, from });
        self.dirty = true;
    }

    /// Opens the menu from the event form's Calendar row (its value, at the row's trailing edge).
    pub(super) fn open_form_cal_menu(&mut self, ui: &Ui) {
        self.focus(None);
        let Some(d) = &self.editor else { return };
        let current = if self.extras.exists(d.extra.calendar) { d.extra.calendar } else { self.extras.default_id() };
        let Some((_, top, h)) = self.rows().into_iter().find(|r| r.0 == Target::Calendar) else { return };
        let k = ui.pt(1.0);
        let (x0, w) = (self.x0 / k, self.w as f32 / k);
        let bw = MIN_W.min(w);
        let button = Rect { x: x0 + (w - 24.0 - bw).max(0.0), y: top, w: bw, h };
        self.open_cal_menu(ui, button, current, MenuFor::Form);
    }

    /// Whether the menu of calendars is open.
    pub fn cal_menu_open(&self) -> bool {
        self.cal_menu.is_some()
    }

    /// A press at window pixel `(x, y)` while the menu is open: a row picks its calendar, the padding does nothing, and
    /// anywhere outside the menu closes it (the press goes no further).
    pub(super) fn cal_menu_press(&mut self, ui: &Ui, x: f32, y: f32) {
        let k = ui.pt(1.0);
        let (px, py) = (x / k, y / k);
        let Some(m) = &self.cal_menu else { return };
        self.dirty = true;
        if !m.rect.contains(px, py) {
            self.cal_menu = None;
            return;
        }
        if let Some(i) = m.menu.hit(py - m.rect.y, MENU_ROW_PT, MENU_SEPARATOR_PT, MENU_PAD_PT) {
            self.cal_menu_choose(i);
        }
    }

    /// The pointer moved over the open menu with the button down: the row under it is highlighted.
    pub(super) fn cal_menu_hover(&mut self, ui: &Ui, x: f32, y: f32) {
        let k = ui.pt(1.0);
        let (px, py) = (x / k, y / k);
        let Some(m) = &mut self.cal_menu else { return };
        if m.rect.contains(px, py) {
            if let Some(i) = m.menu.hit(py - m.rect.y, MENU_ROW_PT, MENU_SEPARATOR_PT, MENU_PAD_PT) {
                m.menu.highlighted = Some(i);
                self.dirty = true;
            }
        }
    }

    /// Picks the calendar of row `i` for the popover's event or the form's draft, and closes the menu.
    pub(super) fn cal_menu_choose(&mut self, i: usize) {
        let Some(m) = self.cal_menu.take() else { return };
        self.dirty = true;
        let Some(&id) = m.ids.get(i) else { return };
        match m.from {
            MenuFor::Popover => {
                if let Some(pop) = &mut self.event_pop {
                    pop.calendar = id;
                }
            }
            MenuFor::Form => {
                if let Some(d) = &mut self.editor {
                    d.extra.calendar = id;
                }
            }
        }
    }

    /// A key while the menu is open: Up and Down move the highlight, letters select by name, Return picks the
    /// highlighted calendar.
    pub(super) fn cal_menu_key(&mut self, out: Out, t: f64) {
        self.dirty = true;
        let Some(m) = &mut self.cal_menu else { return };
        match out {
            Out::Named("Up") => {
                m.menu.step(-1);
            }
            Out::Named("Down") => {
                m.menu.step(1);
            }
            Out::Text(text) => {
                for ch in text.chars() {
                    m.menu.type_select(ch, t / 1000.0);
                }
            }
            Out::Enter => {
                if let Some(i) = m.menu.highlighted {
                    self.cal_menu_choose(i);
                }
            }
            _ => {}
        }
    }

    /// Escape: the menu closes and nothing changes. False when it is not open.
    pub(super) fn cal_menu_cancel(&mut self) -> bool {
        self.dirty = true;
        self.cal_menu.take().is_some()
    }

    /// Draws the open menu over everything else (`draw` and `draw_pane` call it last). A menu whose popover or form has
    /// closed meanwhile is dropped.
    pub(super) fn draw_cal_menu(&mut self, s: &mut Surface, ui: &Ui) {
        let gone = self.cal_menu.as_ref().is_some_and(|m| match m.from {
            MenuFor::Popover => self.event_pop.is_none(),
            MenuFor::Form => self.editor.is_none(),
        });
        if gone {
            self.cal_menu = None;
        }
        if let Some(m) = &self.cal_menu {
            widgets::menu(s, ui, (m.rect.x, m.rect.y), m.rect.w, &m.menu);
        }
    }

    /// The menu's rows for assistive technology (points).
    pub(super) fn cal_menu_nodes(&self) -> Vec<Node> {
        let Some(m) = &self.cal_menu else { return vec![] };
        let (rows, _) = m.menu.rows(MENU_ROW_PT, MENU_SEPARATOR_PT, MENU_PAD_PT);
        m.menu
            .items
            .iter()
            .zip(rows)
            .enumerate()
            .map(|(i, (item, (top, h)))| Node {
                id: NODE + i as u64,
                role: Role::MenuItem,
                label: item.title.clone(),
                value: if item.checked { "checked".into() } else { String::new() },
                bounds: Rect { x: m.rect.x, y: m.rect.y + top, w: m.rect.w, h },
                enabled: true,
            })
            .collect()
    }
}
