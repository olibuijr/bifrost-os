//! The Tips window: a sidebar with the search field and the scopes (All Tips, New, one per collection), the list of
//! cards, and the card view. Drawn with the SDK's Golden Gate widgets. Opening a card marks it read, and the read
//! state is saved at once.

use crate::catalogue::TIPS;
use crate::model::{self, Collection, Scope, Seen, Tip};
use crate::store;
use bifrost_app::controls::nav::{Toolbar, ToolbarItem, ToolbarLayout};
use bifrost_app::controls::sidebar::{SidebarItem, SourceList};
use bifrost_app::controls::{Node, Rect as KRect, Role, TextField};
use bifrost_app::kit::ui::{
    CARD_BG, CARD_RADIUS_PT, CONTENT_BG, LABEL, LABEL_2, LINK, MAC_BODY_PT, MAC_CALLOUT_PT, MAC_CAPTION1_PT,
    MAC_FOOTNOTE_PT, MAC_HEADLINE_PT, MAC_TITLE1_PT, MAC_TITLE3_PT, SEPARATOR_C, SPACE_M, SPACE_S, WHITE,
};
use bifrost_app::kit::{Scroller, Surface, TextCache, Ui, Weight};
use bifrost_app::layout::UNIFIED_BAR_PT;
use bifrost_app::widgets;
use bifrost_app::{Ctx, KeyInput, TouchKind};
use std::path::PathBuf;

/// Width of the sidebar, in points.
const SIDE_PT: f32 = 220.0;
/// Height of the search field's row in the sidebar, and the sidebar's row height.
const FIELD_H_PT: f32 = 36.0;
const SIDE_ROW_PT: f32 = 28.0;
/// Side margin of the list and the card view, the gap under the toolbar, and the card geometry.
const MARGIN_PT: f32 = 20.0;
const LIST_TOP_PT: f32 = 12.0;
const CARD_H_PT: f32 = 80.0;
const CARD_GAP_PT: f32 = 8.0;
const CARD_PAD_PT: f32 = 14.0;
const BADGE_W_PT: f32 = 34.0;
const BADGE_H_PT: f32 = 16.0;

/// Sidebar row ids: All Tips, New, then one per collection in `Collection::ALL` order.
const ROW_ALL: u32 = 1;
const ROW_NEW: u32 = 2;
const ROW_COLLECTION: u32 = 10;
/// Toolbar item ids.
const BAR_BACK: u32 = 1;
const BAR_MARK: u32 = 2;
/// Accessibility ids: the search field, the sidebar rows (`ROW_NODE + row`) and the cards (`CARD_NODE + tip`).
const SEARCH_NODE: u64 = 3;
const ROW_NODE: u64 = 100;
const CARD_NODE: u64 = 1000;

/// Where a press started, so a release on the same target acts on it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Target {
    Search,
    Row(u32),
    Bar(u32),
    Pane,
}

#[derive(Clone, Copy)]
enum Align {
    Left,
    Centre,
}

fn row_id(scope: Scope) -> u32 {
    match scope {
        Scope::All => ROW_ALL,
        Scope::New => ROW_NEW,
        Scope::Collection(c) => {
            ROW_COLLECTION + Collection::ALL.iter().position(|&other| other == c).unwrap_or(0) as u32
        }
    }
}

fn row_scope(id: u32) -> Option<Scope> {
    match id {
        ROW_ALL => Some(Scope::All),
        ROW_NEW => Some(Scope::New),
        id if id >= ROW_COLLECTION => {
            Collection::ALL.get((id - ROW_COLLECTION) as usize).map(|&c| Scope::Collection(c))
        }
        _ => None,
    }
}

pub struct Tips {
    data_dir: PathBuf,
    /// The window size in pixels.
    w: i32,
    h: i32,
    scope: Scope,
    query: TextField,
    seen: Seen,
    /// The index into `TIPS` of the card on screen, or `None` for the list.
    open: Option<usize>,
    scroll: Scroller,
    press: Option<(Target, f32, f32)>,
    keyboard_on: bool,
    labels: TextCache,
}

impl Tips {
    pub fn new(data_dir: PathBuf, w: i32, h: i32) -> Self {
        let seen = store::load(&data_dir);
        Tips {
            data_dir,
            w,
            h,
            scope: Scope::All,
            query: TextField::new(),
            seen,
            open: None,
            scroll: Scroller::new(),
            press: None,
            keyboard_on: false,
            labels: TextCache::default(),
        }
    }

    pub fn set_size(&mut self, w: i32, h: i32) {
        self.w = w;
        self.h = h;
    }

    /// The cards in the current scope that match the search, in catalogue order.
    fn order(&self) -> Vec<usize> {
        model::visible(TIPS, self.scope, &self.query, &self.seen)
    }

    fn side_px(&self, ui: &Ui) -> f32 {
        ui.pt(SIDE_PT).min(self.w as f32).max(0.0)
    }

    fn pane_w_px(&self, ui: &Ui) -> f32 {
        (self.w as f32 - self.side_px(ui)).max(0.0)
    }

    fn bar_px(ui: &Ui) -> f32 {
        ui.pt(UNIFIED_BAR_PT)
    }

    fn list_top_px(ui: &Ui) -> f32 {
        Self::bar_px(ui) + ui.pt(LIST_TOP_PT)
    }

    fn card_px(ui: &Ui) -> f32 {
        ui.pt(CARD_H_PT + CARD_GAP_PT)
    }

    /// Keeps the list's scroll range in step with the number of visible cards.
    fn update_scroll(&mut self, ui: &Ui) {
        let rows = self.order().len() as f32;
        let content = 2.0 * ui.pt(LIST_TOP_PT) + rows * Self::card_px(ui);
        let viewport = (self.h as f32 - Self::bar_px(ui)).max(0.0);
        self.scroll.set_max((content - viewport).max(0.0));
    }

    // Geometry in points, for the widgets.

    fn side_rect(&self, ui: &Ui) -> KRect {
        let p = ui.pt(1.0);
        KRect { x: 0.0, y: 0.0, w: self.side_px(ui) / p, h: self.h as f32 / p }
    }

    fn side_search(&self, ui: &Ui) -> KRect {
        let p = ui.pt(1.0);
        KRect { x: SPACE_M, y: UNIFIED_BAR_PT, w: (self.side_px(ui) / p - 2.0 * SPACE_M).max(0.0), h: FIELD_H_PT - 4.0 }
    }

    fn side_list(&self, ui: &Ui) -> KRect {
        let p = ui.pt(1.0);
        let top = UNIFIED_BAR_PT + FIELD_H_PT + SPACE_S;
        KRect { x: 0.0, y: top, w: self.side_px(ui) / p, h: (self.h as f32 / p - top).max(0.0) }
    }

    fn bar_bounds(&self, ui: &Ui) -> KRect {
        let p = ui.pt(1.0);
        KRect { x: SPACE_M, y: 0.0, w: (self.pane_w_px(ui) / p - 2.0 * SPACE_M).max(0.0), h: UNIFIED_BAR_PT }
    }

    /// The sidebar rows with their ids (`None` for the Collections header).
    fn sidebar_items(&self) -> Vec<(Option<u32>, SidebarItem)> {
        let mut items = vec![
            (
                Some(ROW_ALL),
                SidebarItem::row(ROW_ALL, Scope::All.title()).with_symbol("list.bullet").with_count(TIPS.len() as u32),
            ),
            (
                Some(ROW_NEW),
                SidebarItem::row(ROW_NEW, Scope::New.title())
                    .with_symbol("clock")
                    .with_count(model::new_count(TIPS, &self.seen) as u32),
            ),
            (None, SidebarItem::header("Collections")),
        ];
        for (index, collection) in Collection::ALL.iter().enumerate() {
            let id = ROW_COLLECTION + index as u32;
            let count = TIPS.iter().filter(|tip| tip.collection == *collection).count() as u32;
            items.push((Some(id), SidebarItem::row(id, collection.title()).with_count(count)));
        }
        items
    }

    fn sidebar(&self) -> SourceList {
        let items = self.sidebar_items().into_iter().map(|(_, item)| item).collect();
        let mut list = SourceList::new(items, SIDE_ROW_PT, SIDE_ROW_PT);
        list.set_selected(Some(row_id(self.scope)));
        list
    }

    fn bar_items(&self) -> Toolbar {
        match self.open {
            None => Toolbar::new(vec![
                ToolbarItem::flexible_space(),
                ToolbarItem::button(BAR_MARK, "Mark All Read"),
            ]),
            Some(_) => Toolbar::new(vec![
                ToolbarItem::icon(BAR_BACK, "Tips", "chevron.left"),
                ToolbarItem::flexible_space(),
            ]),
        }
    }

    fn bar_layout(&self, ui: &Ui) -> ToolbarLayout {
        let p = ui.pt(1.0);
        let width = |item: &ToolbarItem| {
            if item.symbol.is_some() {
                widgets::CAPSULE_PT
            } else {
                (ui.measure(&item.label, MAC_BODY_PT, Weight::Regular) / p + 3.0 * SPACE_S).max(widgets::CAPSULE_PT)
            }
        };
        let bounds = self.bar_bounds(ui);
        let h = widgets::CAPSULE_PT;
        self.bar_items().layout_with(
            KRect { y: bounds.y + (bounds.h - h) / 2.0, h, ..bounds },
            width,
            widgets::CAPSULE_PT,
            widgets::TOOLBAR_GAP_PT,
        )
    }

    /// What is under window pixel (x, y).
    fn target_at(&self, ui: &Ui, x: f32, y: f32) -> Option<Target> {
        let p = ui.pt(1.0);
        let side = self.side_px(ui);
        if x < side {
            let (xp, yp) = (x / p, y / p);
            if self.side_search(ui).contains(xp, yp) {
                return Some(Target::Search);
            }
            return self.sidebar().hit(self.side_list(ui), 0.0, xp, yp).map(Target::Row);
        }
        if y < Self::bar_px(ui) {
            return self.bar_layout(ui).hit((x - side) / p, y / p).map(Target::Bar);
        }
        Some(Target::Pane)
    }

    /// The card under a point in the list, given in pixels from the top-left of the content pane.
    fn card_at(&self, ui: &Ui, px: f32, py: f32) -> Option<usize> {
        let margin = ui.pt(MARGIN_PT);
        if px < margin || px > self.pane_w_px(ui) - margin {
            return None;
        }
        let rel = py - Self::list_top_px(ui) + self.scroll.offset();
        if rel < 0.0 {
            return None;
        }
        let row = (rel / Self::card_px(ui)) as usize;
        if rel - row as f32 * Self::card_px(ui) > ui.pt(CARD_H_PT) {
            return None;
        }
        self.order().get(row).copied()
    }

    pub fn draw(&mut self, ui: &Ui, s: &mut Surface) {
        self.update_scroll(ui);
        s.fill_rect(0, 0, s.w, s.h, CONTENT_BG.now());
        let side = (self.side_px(ui) as usize).min(s.w);
        let pressed = match self.press {
            Some((Target::Row(id), _, _)) => Some(id),
            _ => None,
        };
        // the sidebar background first, then its search field and rows
        widgets::source_list(s, ui, self.side_rect(ui), &SourceList::new(vec![], SIDE_ROW_PT, SIDE_ROW_PT), 0.0, None);
        widgets::search_field(s, ui, &self.query, self.side_search(ui), "Search", self.keyboard_on);
        widgets::source_list(s, ui, self.side_list(ui), &self.sidebar(), 0.0, pressed);
        if side > 0 {
            s.fill_rect(side - 1, 0, 1, s.h, SEPARATOR_C.now());
        }
        // the content pane draws in its own coordinates, right of the sidebar
        let mut pane = Surface { w: s.w - side, h: s.h, stride: s.stride, px: &mut s.px[side..], y0: s.y0 };
        if self.open.is_some() {
            self.draw_card_view(&mut pane, ui);
        } else {
            self.draw_list(&mut pane, ui);
        }
        self.draw_bar(&mut pane, ui);
    }

    fn draw_bar(&mut self, s: &mut Surface, ui: &Ui) {
        let width = s.w as f32 / ui.pt(1.0);
        widgets::toolbar_band(s, ui, KRect { x: 0.0, y: 0.0, w: width, h: UNIFIED_BAR_PT });
        let title = match self.open {
            Some(index) => TIPS[index].collection.title(),
            None => self.scope.title(),
        };
        let centre = (s.w as f32 / 2.0, ui.pt(UNIFIED_BAR_PT / 2.0));
        self.text(s, ui, title, MAC_TITLE3_PT, Weight::Bold, LABEL.now(), Align::Centre, centre);
        let pressed = match self.press {
            Some((Target::Bar(id), _, _)) => Some(id),
            _ => None,
        };
        widgets::toolbar(s, ui, self.bar_bounds(ui), &self.bar_items(), pressed);
    }

    fn draw_list(&mut self, s: &mut Surface, ui: &Ui) {
        let order = self.order();
        if order.is_empty() {
            let message = if !self.query.is_empty() {
                "No tips match the search."
            } else if self.scope == Scope::New {
                "You have read every tip."
            } else {
                "No tips here yet."
            };
            let centre = (s.w as f32 / 2.0, Self::list_top_px(ui) + ui.pt(40.0));
            self.text(s, ui, message, MAC_BODY_PT, Weight::Regular, LABEL_2.now(), Align::Centre, centre);
            return;
        }
        let x = ui.pt(MARGIN_PT);
        let w = (s.w as f32 - 2.0 * x).max(0.0);
        let h = ui.pt(CARD_H_PT);
        let top = Self::list_top_px(ui) - self.scroll.offset();
        for (row, &index) in order.iter().enumerate() {
            let y = top + row as f32 * Self::card_px(ui);
            if y + h < 0.0 || y > s.h as f32 {
                continue;
            }
            self.draw_card(s, ui, &TIPS[index], (x, y, w, h));
        }
    }

    /// One card: the title with a New badge while unread, the collection, and two lines of the text.
    fn draw_card(&mut self, s: &mut Surface, ui: &Ui, tip: &Tip, (x, y, w, h): (f32, f32, f32, f32)) {
        s.fill_rrect(x, y, w, h, ui.pt(CARD_RADIUS_PT), CARD_BG.now());
        let p = ui.pt(1.0);
        let pad = ui.pt(CARD_PAD_PT);
        let left = x + pad;
        let at = |pt: f32| y + ui.pt(pt);
        let mut right = x + w - pad;
        if !self.seen.contains(tip.id) {
            let badge_w = ui.pt(BADGE_W_PT);
            let badge_x = right - badge_w;
            let badge_y = at(16.0) - ui.pt(BADGE_H_PT) / 2.0;
            s.fill_rrect(badge_x, badge_y, badge_w, ui.pt(BADGE_H_PT), ui.pt(4.0), LINK);
            let centre = (badge_x + badge_w / 2.0, at(16.0));
            self.text(s, ui, "New", MAC_CAPTION1_PT, Weight::Bold, WHITE, Align::Centre, centre);
            right = badge_x - ui.pt(SPACE_S);
        }
        let title_w = ((right - left) / p).max(0.0);
        let title = model::lines(tip.title, title_w, 1, |t| ui.measure(t, MAC_HEADLINE_PT, Weight::Bold) / p);
        let shown = title.first().cloned().unwrap_or_default();
        self.text(s, ui, &shown, MAC_HEADLINE_PT, Weight::Bold, LABEL.now(), Align::Left, (left, at(16.0)));
        self.text(
            s,
            ui,
            tip.collection.title(),
            MAC_FOOTNOTE_PT,
            Weight::Regular,
            LABEL_2.now(),
            Align::Left,
            (left, at(34.0)),
        );
        let body_w = ((x + w - pad - left) / p).max(0.0);
        let excerpt = model::lines(tip.body, body_w, 2, |t| ui.measure(t, MAC_CALLOUT_PT, Weight::Regular) / p);
        for (line, text) in excerpt.iter().enumerate() {
            let mid = (left, at(54.0 + 16.0 * line as f32));
            self.text(s, ui, text, MAC_CALLOUT_PT, Weight::Regular, LABEL_2.now(), Align::Left, mid);
        }
    }

    /// The card view: title, collection and the whole text, with a hint for moving between cards.
    fn draw_card_view(&mut self, s: &mut Surface, ui: &Ui) {
        let Some(index) = self.open else { return };
        let tip = TIPS[index];
        let p = ui.pt(1.0);
        let x = ui.pt(MARGIN_PT);
        let width = ((s.w as f32 - 2.0 * x) / p).max(0.0);
        let at = |pt: f32| ui.pt(UNIFIED_BAR_PT + pt);
        let title = model::lines(tip.title, width, 1, |t| ui.measure(t, MAC_TITLE1_PT, Weight::Bold) / p);
        let shown = title.first().cloned().unwrap_or_default();
        self.text(s, ui, &shown, MAC_TITLE1_PT, Weight::Bold, LABEL.now(), Align::Left, (x, at(34.0)));
        self.text(
            s,
            ui,
            tip.collection.title(),
            MAC_FOOTNOTE_PT,
            Weight::Regular,
            LABEL_2.now(),
            Align::Left,
            (x, at(60.0)),
        );
        let body = model::wrap(tip.body, width, |t| ui.measure(t, MAC_BODY_PT, Weight::Regular) / p);
        for (line, text) in body.iter().enumerate() {
            let mid = (x, at(92.0 + 22.0 * line as f32));
            self.text(s, ui, text, MAC_BODY_PT, Weight::Regular, LABEL.now(), Align::Left, mid);
        }
        let hint = (x, s.h as f32 - ui.pt(24.0));
        let hint_text = "Left and Right move to the next card. Escape returns to the list.";
        self.text(s, ui, hint_text, MAC_FOOTNOTE_PT, Weight::Regular, LABEL_2.now(), Align::Left, hint);
    }

    /// Draws `text` at `mid` (x, centre y) in pixels.
    fn text(
        &mut self,
        s: &mut Surface,
        ui: &Ui,
        text: &str,
        pt: f32,
        weight: Weight,
        color: u32,
        align: Align,
        (x, mid): (f32, f32),
    ) {
        let img = self.labels.get(ui, text, pt, weight, color);
        let left = match align {
            Align::Left => x,
            Align::Centre => x - img.w as f32 / 2.0,
        };
        s.blit(img, left.round() as i32, (mid - img.h as f32 / 2.0).round() as i32, 255, 255);
    }

    /// A touch in window pixels. True when the window needs a new frame.
    pub fn touch(&mut self, ctx: &Ctx, kind: TouchKind, x: f32, y: f32, t: f64) -> bool {
        let ui = &ctx.ui;
        self.update_scroll(ui);
        match kind {
            TouchKind::Down => {
                let target = self.target_at(ui, x, y);
                if target == Some(Target::Pane) && self.open.is_none() {
                    self.scroll.down(y, t);
                }
                self.press = target.map(|target| (target, x, y));
                true
            }
            TouchKind::Move => {
                if self.open.is_none() && matches!(self.press, Some((Target::Pane, _, _))) {
                    self.scroll.moved(y, t);
                }
                true
            }
            TouchKind::Up | TouchKind::Cancel => {
                let Some((target, px, py)) = self.press.take() else { return false };
                if target == Target::Pane && self.open.is_none() {
                    // a release without a drag is a tap on the card under the press
                    let tapped = self.scroll.up(y, t) && matches!(kind, TouchKind::Up);
                    let side = self.side_px(ui);
                    if tapped {
                        if let Some(index) = self.card_at(ui, px - side, py) {
                            self.open_card(ctx, index);
                        }
                    }
                    return true;
                }
                if self.target_at(ui, x, y) == Some(target) {
                    self.activate(ctx, target);
                }
                true
            }
        }
    }

    fn activate(&mut self, ctx: &Ctx, target: Target) {
        match target {
            Target::Search => {
                self.open = None;
                self.keyboard(ctx, true);
            }
            Target::Row(id) => {
                if let Some(scope) = row_scope(id) {
                    self.scope = scope;
                    self.open = None;
                    self.scroll.jump_top();
                    self.keyboard(ctx, false);
                }
            }
            Target::Bar(BAR_BACK) => {
                self.open = None;
                self.keyboard(ctx, false);
            }
            Target::Bar(BAR_MARK) => {
                if self.seen.mark_all(TIPS) {
                    self.save();
                }
            }
            Target::Bar(_) | Target::Pane => {}
        }
    }

    /// Shows the on-screen keyboard for the search field, or hides it.
    fn keyboard(&mut self, ctx: &Ctx, on: bool) {
        if self.keyboard_on != on {
            self.keyboard_on = on;
            ctx.wants_keyboard(on);
        }
    }

    fn open_card(&mut self, ctx: &Ctx, index: usize) {
        self.open = Some(index);
        self.keyboard(ctx, false);
        if self.seen.mark(TIPS[index].id) {
            self.save();
        }
    }

    fn save(&self) {
        if let Err(error) = store::save(&self.data_dir, &self.seen) {
            eprintln!("tips: could not save the read state: {error}");
        }
    }

    /// Typed text and editing keys. True when the window needs a new frame.
    pub fn key(&mut self, ctx: &Ctx, key: KeyInput) -> bool {
        match key {
            KeyInput::Text(text) => {
                if self.open.is_some() {
                    return false;
                }
                self.query.push_str(&text);
                self.scroll.jump_top();
                true
            }
            KeyInput::Code(code) => self.code(ctx, code),
        }
    }

    fn code(&mut self, ctx: &Ctx, code: u32) -> bool {
        match code {
            8 => {
                if self.open.is_some() {
                    return false;
                }
                self.query.pop();
                self.scroll.jump_top();
                true
            }
            27 => {
                if self.open.is_some() {
                    self.open = None;
                    self.keyboard(ctx, false);
                    return true;
                }
                if self.query.is_empty() {
                    return false;
                }
                self.query.clear();
                self.scroll.jump_top();
                true
            }
            // Left and Right step through the cards of the list the open card came from
            37 | 39 => {
                let Some(current) = self.open else { return false };
                let order = self.order();
                if let Some(next) = model::step(&order, current, code == 39) {
                    if next != current {
                        self.open_card(ctx, next);
                    }
                }
                true
            }
            _ => false,
        }
    }

    pub fn tick(&mut self, t: f64) -> bool {
        self.scroll.tick(t)
    }

    /// The window's controls for accessibility, in window points.
    pub fn controls(&self, ui: &Ui) -> Vec<Node> {
        let p = ui.pt(1.0);
        let side = self.side_px(ui) / p;
        let mut nodes = Vec::new();
        let bar = self.bar_items();
        for (id, rect) in &self.bar_layout(ui).visible {
            if let Some(item) = bar.items.iter().find(|item| item.id == *id) {
                let bounds = KRect { x: rect.x + side, ..*rect };
                nodes.push(Node {
                    id: *id as u64,
                    role: Role::Button,
                    label: item.label.clone(),
                    value: String::new(),
                    bounds,
                    enabled: item.enabled,
                });
            }
        }
        nodes.push(Node {
            id: SEARCH_NODE,
            role: Role::TextField,
            label: "Search".into(),
            value: self.query.to_string(),
            bounds: self.side_search(ui),
            enabled: true,
        });
        let list_bounds = self.side_list(ui);
        let sidebar = self.sidebar();
        for (index, (id, _)) in self.sidebar_items().iter().enumerate() {
            let Some(id) = *id else { continue };
            let Some(scope) = row_scope(id) else { continue };
            nodes.push(Node {
                id: ROW_NODE + id as u64,
                role: Role::Row,
                label: scope.title().into(),
                value: if self.scope == scope { "selected".into() } else { String::new() },
                bounds: sidebar.item_rect(index, list_bounds, 0.0),
                enabled: true,
            });
        }
        if self.open.is_none() {
            let top = Self::list_top_px(ui) - self.scroll.offset();
            let x = ui.pt(MARGIN_PT);
            let w = self.pane_w_px(ui) - 2.0 * x;
            for (row, index) in self.order().into_iter().enumerate() {
                let y = top + row as f32 * Self::card_px(ui);
                if y + ui.pt(CARD_H_PT) < 0.0 || y > self.h as f32 {
                    continue;
                }
                let tip = TIPS[index];
                let value = if self.seen.contains(tip.id) { String::new() } else { "New".into() };
                nodes.push(Node {
                    id: CARD_NODE + index as u64,
                    role: Role::Button,
                    label: tip.title.into(),
                    value,
                    bounds: KRect { x: (side * p + x) / p, y: y / p, w: w / p, h: CARD_H_PT },
                    enabled: true,
                });
            }
        }
        nodes
    }
}
