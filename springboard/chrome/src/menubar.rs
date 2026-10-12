//! The menu bar client: a top layer-shell surface with an exclusive zone, the menus as overlay popups, drawn with
//! `Canvas` into a `DisplayList` (executed by `bifrost-2d` on the GPU through `surface::Display::draw`). The models
//! come from the shell over `link` (`ToChrome::MenuBar`); a chosen item goes back as `FromChrome::MenuAction`.
//!
//! Model ids (the shell side in `springboard/src/shell/chrome_host/menubar.rs` relies on these): `menus[0]` is the
//! system menu (drawn as the mark), `menus[1..]` are the app, the app's own menus, File, Edit, View, Window and Help.
//! A menu's `id` is its index in the shell's bar order and a row's `id` is its index in that menu's rows.
//!
//! Layout is pure (points, `layout_bar`, `layout_popup`), navigation is pure (`Nav`), and both are host-tested. Only
//! `run` and the painters touch the compositor or the font.
//!
//! Owner: wave G2 agent `chrome-menubar`.

use crate::link::Link;
use crate::model::{FromChrome, MenuBarModel, MenuItem, ToChrome};
use crate::surface::{Anchor, Display, Event, Layer, SurfaceId, SurfaceSpec};
use bifrost_render::canvas::{register_face, with_face, Canvas, RectF, TextRun, Transform2D};
use bifrost_render::display_list::DisplayList;
use bifrost_render::ui;

/// The UI face (the same file the shell draws with).
static FONT: &[u8] = include_bytes!("../../assets/SFUI.ttf");

/// Bar height in points (the shell's `desktop_bar_height`).
pub const BAR_HEIGHT_PT: f32 = 28.0;
/// The system mark: its width and its left inset.
pub const SYSTEM_PT: f32 = 34.0;
pub const SYSTEM_LEFT_PT: f32 = 6.0;
/// Padding on each side of a menu title (the title's width plus twice this).
pub const ITEM_PAD_PT: f32 = 12.0;
/// Width of a status item without text (an icon) and the padding on each side of its text.
pub const STATUS_ICON_PT: f32 = 30.0;
pub const STATUS_PAD_PT: f32 = 10.0;
/// The gap kept between the menus and the status items, and the inset of the status items from the right edge.
pub const BAR_GAP_PT: f32 = 8.0;
pub const BAR_EDGE_PT: f32 = 16.0;
/// Popup geometry: rows, separators, padding, the gap under the bar, the label and shortcut insets, and the minimum
/// width.
pub const ROW_PT: f32 = 24.0;
pub const SEP_PT: f32 = 11.0;
pub const PAD_PT: f32 = 5.0;
pub const MENU_GAP_PT: f32 = 6.0;
pub const ROW_LEFT_PT: f32 = 18.0;
pub const ROW_RIGHT_PT: f32 = 16.0;
pub const SHORTCUT_GAP_PT: f32 = 24.0;
pub const MIN_WIDTH_PT: f32 = 180.0;
/// Corner radius of a popup and of the highlighted row.
pub const MENU_RADIUS_PT: f32 = 6.0;
const ROW_RADIUS_PT: f32 = 4.0;
/// Title and status text sizes (the reference platform table: headline for titles, callout for status).
const TITLE_PT: f32 = ui::MAC_HEADLINE_PT;
const TEXT_PT: f32 = ui::MAC_CALLOUT_PT;

/// Linux button code of the primary button.
pub const BTN_LEFT: u32 = 0x110;
const XK_ESCAPE: u32 = 0xff1b;
const XK_RETURN: u32 = 0xff0d;
const XK_KP_ENTER: u32 = 0xff8d;
const XK_LEFT: u32 = 0xff51;
const XK_UP: u32 = 0xff52;
const XK_RIGHT: u32 = 0xff53;
const XK_DOWN: u32 = 0xff54;

/// Where an item sits in the bar: an index into `model.menus` or `model.status`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Slot {
    Menu(usize),
    Status(usize),
}

/// One laid-out bar item, in points from the left edge of the bar.
#[derive(Debug, Clone, PartialEq)]
pub struct BarItem {
    pub slot: Slot,
    pub x: f32,
    pub w: f32,
    /// Where the label starts.
    pub text_x: f32,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct BarLayout {
    pub items: Vec<BarItem>,
    pub width: f32,
}

impl BarLayout {
    pub fn item(&self, slot: Slot) -> Option<&BarItem> {
        self.items.iter().find(|i| i.slot == slot)
    }

    /// The item under `x` (points from the left edge).
    pub fn hit(&self, x: f32) -> Option<Slot> {
        self.items.iter().find(|i| x >= i.x && x < i.x + i.w).map(|i| i.slot)
    }

    pub fn slots(&self) -> Vec<Slot> {
        self.items.iter().map(|i| i.slot).collect()
    }
}

/// Places the menus from the left (the system mark, then each title) and the status items from the right edge.
/// Status items that would run into the menus are left out, the leftmost first, as the shell's bar does.
pub fn layout_bar(model: &MenuBarModel, width: f32, measure: &dyn Fn(&str, f32) -> f32) -> BarLayout {
    let mut items = Vec::new();
    let mut x = SYSTEM_LEFT_PT;
    for (i, m) in model.menus.iter().enumerate() {
        if i == 0 {
            items.push(BarItem { slot: Slot::Menu(0), x, w: SYSTEM_PT, text_x: x, label: String::new() });
            x += SYSTEM_PT;
        } else {
            let w = measure(&m.title, TITLE_PT) + 2.0 * ITEM_PAD_PT;
            items.push(BarItem { slot: Slot::Menu(i), x, w, text_x: x + ITEM_PAD_PT, label: m.title.clone() });
            x += w;
        }
    }
    let limit = x + BAR_GAP_PT;
    let mut edge = width - BAR_EDGE_PT;
    let mut status = Vec::new();
    for (i, s) in model.status.iter().enumerate().rev() {
        let w = if s.text.is_empty() {
            STATUS_ICON_PT
        } else {
            measure(&s.text, TEXT_PT) + 2.0 * STATUS_PAD_PT
        };
        if edge - w < limit {
            break;
        }
        edge -= w;
        status.push(BarItem { slot: Slot::Status(i), x: edge, w, text_x: edge + STATUS_PAD_PT, label: s.text.clone() });
    }
    status.reverse();
    items.extend(status);
    BarLayout { items, width }
}

/// One row of a popup: its top and height from the top of the popup, and where its shortcut starts.
#[derive(Debug, Clone, PartialEq)]
pub struct RowSpan {
    pub top: f32,
    pub h: f32,
    pub separator: bool,
    pub shortcut_x: f32,
}

/// A popup's place on the screen (points) and its rows.
#[derive(Debug, Clone, PartialEq)]
pub struct PopupLayout {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub rows: Vec<RowSpan>,
}

impl PopupLayout {
    /// The row under `y` (popup-local points), if any.
    pub fn row_at(&self, y: f32) -> Option<usize> {
        self.rows.iter().position(|r| y >= r.top && y < r.top + r.h)
    }
}

/// Lays out a popup under the bar slot at `anchor_x`: the width is the widest row (clamped to `MIN_WIDTH_PT`), and the
/// popup moves left so it stays on the screen of width `screen_w`.
pub fn layout_popup(items: &[MenuItem], anchor_x: f32, bar_h: f32, screen_w: f32, measure: &dyn Fn(&str, f32) -> f32) -> PopupLayout {
    let mut rows = Vec::with_capacity(items.len());
    let mut y = PAD_PT;
    let mut content_w: f32 = 0.0;
    let mut shortcut_w = Vec::with_capacity(items.len());
    for item in items {
        let separator = item.label.is_empty();
        let h = if separator { SEP_PT } else { ROW_PT };
        rows.push(RowSpan { top: y, h, separator, shortcut_x: 0.0 });
        y += h;
        if separator {
            shortcut_w.push(0.0);
            continue;
        }
        let sw = if item.shortcut.is_empty() { 0.0 } else { measure(&item.shortcut, TEXT_PT) };
        shortcut_w.push(sw);
        let gap = if sw > 0.0 { SHORTCUT_GAP_PT + sw } else { 0.0 };
        content_w = content_w.max(ROW_LEFT_PT + measure(&item.label, TEXT_PT) + gap + ROW_RIGHT_PT);
    }
    let w = content_w.max(MIN_WIDTH_PT);
    for (row, sw) in rows.iter_mut().zip(shortcut_w) {
        row.shortcut_x = w - ROW_RIGHT_PT - sw;
    }
    let h = y + PAD_PT;
    let x = anchor_x.min(screen_w - w).max(0.0);
    PopupLayout { x, y: bar_h + MENU_GAP_PT, w, h, rows }
}

/// A row a keyboard or pointer can choose: a label with text, not a separator, and enabled.
pub fn selectable(item: &MenuItem) -> bool {
    !item.label.is_empty() && item.enabled
}

/// The first selectable row at or after the start, going `dir` (1 or -1) and wrapping.
pub fn next_row(items: &[MenuItem], from: Option<usize>, dir: i32) -> Option<usize> {
    let n = items.len() as i64;
    if n == 0 {
        return None;
    }
    let dir = if dir < 0 { -1i64 } else { 1 };
    let mut i = match from {
        None => {
            if dir > 0 {
                0
            } else {
                n - 1
            }
        }
        Some(f) => (f as i64 + dir).rem_euclid(n),
    };
    for _ in 0..n {
        if selectable(&items[i as usize]) {
            return Some(i as usize);
        }
        i = (i + dir).rem_euclid(n);
    }
    None
}

/// The first selectable row of a menu.
pub fn first_row(items: &[MenuItem]) -> Option<usize> {
    next_row(items, None, 1)
}

/// The next selectable row, after `from` (wrapping), whose label starts with `c` (case-insensitive).
pub fn type_select(items: &[MenuItem], from: Option<usize>, c: char) -> Option<usize> {
    let n = items.len() as i64;
    if n == 0 {
        return None;
    }
    let want = c.to_lowercase().next()?;
    let mut i = match from {
        None => 0,
        Some(f) => (f as i64 + 1).rem_euclid(n),
    };
    for _ in 0..n {
        let item = &items[i as usize];
        if selectable(item) && item.label.chars().next().and_then(|f| f.to_lowercase().next()) == Some(want) {
            return Some(i as usize);
        }
        i = (i + 1).rem_euclid(n);
    }
    None
}

/// The items of a slot's menu.
pub fn items_of(model: &MenuBarModel, slot: Slot) -> &[MenuItem] {
    match slot {
        Slot::Menu(i) => model.menus.get(i).map_or(&[][..], |m| m.items.as_slice()),
        Slot::Status(i) => model.status.get(i).map_or(&[][..], |s| s.menu.as_slice()),
    }
}

/// The id the shell knows a slot's menu by.
pub fn slot_id(model: &MenuBarModel, slot: Slot) -> Option<u32> {
    match slot {
        Slot::Menu(i) => model.menus.get(i).map(|m| m.id),
        Slot::Status(i) => model.status.get(i).map(|s| s.id),
    }
}

pub fn slot_exists(model: &MenuBarModel, slot: Slot) -> bool {
    match slot {
        Slot::Menu(i) => i < model.menus.len(),
        Slot::Status(i) => i < model.status.len(),
    }
}

/// What is open and which row is highlighted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Nav {
    pub open: Option<Slot>,
    pub row: Option<usize>,
}

impl Nav {
    pub fn open(&mut self, model: &MenuBarModel, slot: Slot) {
        self.open = Some(slot);
        self.row = first_row(items_of(model, slot));
    }

    pub fn close(&mut self) {
        *self = Nav::default();
    }

    /// Moves the highlight one selectable row up (-1) or down (1).
    pub fn step(&mut self, model: &MenuBarModel, dir: i32) {
        if let Some(slot) = self.open {
            self.row = next_row(items_of(model, slot), self.row, dir);
        }
    }

    /// Opens the neighbouring slot in bar order (`slots`, the order the bar draws them), wrapping.
    pub fn switch(&mut self, model: &MenuBarModel, slots: &[Slot], dir: i32) {
        let Some(cur) = self.open else { return };
        if slots.is_empty() {
            return;
        }
        let Some(pos) = slots.iter().position(|s| *s == cur) else { return };
        let n = slots.len() as i64;
        let next = slots[((pos as i64 + dir as i64).rem_euclid(n)) as usize];
        self.open(model, next);
    }

    /// Highlights a row under the pointer, if it is selectable.
    pub fn hover(&mut self, model: &MenuBarModel, row: usize) {
        if let Some(slot) = self.open {
            if items_of(model, slot).get(row).is_some_and(selectable) {
                self.row = Some(row);
            }
        }
    }

    pub fn type_select(&mut self, model: &MenuBarModel, c: char) {
        if let Some(slot) = self.open {
            self.row = type_select(items_of(model, slot), self.row, c).or(self.row);
        }
    }

    /// The (menu id, item id) to send when the highlighted row is chosen; `None` for a disabled or missing row.
    pub fn activate(&self, model: &MenuBarModel) -> Option<(u32, u32)> {
        let slot = self.open?;
        let row = self.row?;
        let item = items_of(model, slot).get(row)?;
        if !selectable(item) {
            return None;
        }
        Some((slot_id(model, slot)?, item.id))
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// Painting
// ---------------------------------------------------------------------------------------------------------------------

/// A text measurer over the UI face.
fn measurer(face: u64) -> impl Fn(&str, f32) -> f32 {
    move |t: &str, px: f32| with_face(face, |f| f.measure(t, px)).unwrap_or(0.0)
}

/// The top of a line of text of `px` size, centred in a band of `h` points.
fn text_top(h: f32, px: f32) -> f32 {
    (h - px * 1.25) / 2.0
}

fn text(c: &mut dyn Canvas, face: u64, s: &str, x: f32, y: f32, px: f32, color: u32) {
    if s.is_empty() {
        return;
    }
    c.text(&TextRun { face, text: s.to_string(), px, vars: Vec::new(), x, y }, color);
}

fn paint_bar(c: &mut dyn Canvas, face: u64, layout: &BarLayout, nav: &Nav) {
    c.fill_rect(RectF::new(0.0, 0.0, layout.width, BAR_HEIGHT_PT), ui::BAR_TINT.now());
    for item in &layout.items {
        if nav.open == Some(item.slot) {
            c.fill_rrect(RectF::new(item.x + 2.0, 3.0, item.w - 4.0, BAR_HEIGHT_PT - 6.0), 5.0, ui::BAR_OPEN_CAPSULE);
        }
        match item.slot {
            // the system mark: a rounded square until the BifrOSt mark symbol is drawn here
            Slot::Menu(0) => c.fill_rrect(RectF::new(item.x + 10.0, 7.0, 14.0, 14.0), 4.0, ui::BAR_TEXT),
            Slot::Menu(_) => text(c, face, &item.label, item.text_x, text_top(BAR_HEIGHT_PT, TITLE_PT), TITLE_PT, ui::BAR_TEXT),
            Slot::Status(_) => text(c, face, &item.label, item.text_x, text_top(BAR_HEIGHT_PT, TEXT_PT), TEXT_PT, ui::BAR_TEXT),
        }
    }
}

fn paint_popup(c: &mut dyn Canvas, face: u64, layout: &PopupLayout, items: &[MenuItem], row: Option<usize>) {
    c.fill_rrect(RectF::new(0.0, 0.0, layout.w, layout.h), MENU_RADIUS_PT, ui::MENU_SOLID.now());
    for (i, (item, span)) in items.iter().zip(&layout.rows).enumerate() {
        if span.separator {
            c.fill_rect(
                RectF::new(ROW_LEFT_PT, span.top + SEP_PT / 2.0, layout.w - ROW_LEFT_PT - ROW_RIGHT_PT, 1.0),
                ui::SEPARATOR,
            );
            continue;
        }
        let selected = row == Some(i);
        if selected {
            c.fill_rrect(
                RectF::new(PAD_PT, span.top, layout.w - 2.0 * PAD_PT, span.h),
                ROW_RADIUS_PT,
                ui::MENU_HIGHLIGHT,
            );
        }
        let color = if selected {
            ui::MENU_HIGHLIGHT_TEXT
        } else if selectable(item) {
            ui::LABEL.now()
        } else {
            ui::LABEL_3.now()
        };
        let ty = span.top + text_top(span.h, TEXT_PT);
        if item.checked {
            text(c, face, "\u{2713}", ROW_LEFT_PT - 13.0, ty, TEXT_PT, color);
        }
        text(c, face, &item.label, ROW_LEFT_PT, ty, TEXT_PT, color);
        let shortcut_color = if selected { color } else { ui::MENU_SHORTCUT };
        text(c, face, &item.shortcut, span.shortcut_x, ty, TEXT_PT, shortcut_color);
    }
}

/// Device pixels of a size in points.
fn device(w: f32, h: f32, scale: f32) -> (u32, u32) {
    (((w * scale).round()).max(1.0) as u32, ((h * scale).round()).max(1.0) as u32)
}

fn bar_spec() -> SurfaceSpec {
    SurfaceSpec {
        namespace: "bifrost-menubar".into(),
        layer: Layer::Top,
        anchor: Anchor::TOP | Anchor::LEFT | Anchor::RIGHT,
        size: (0, BAR_HEIGHT_PT as u32),
        exclusive_zone: BAR_HEIGHT_PT as i32,
        margin: (0, 0, 0, 0),
        keyboard: false,
    }
}

fn popup_spec(l: &PopupLayout) -> SurfaceSpec {
    SurfaceSpec {
        namespace: "bifrost-menu".into(),
        layer: Layer::Overlay,
        anchor: Anchor::TOP | Anchor::LEFT,
        size: (l.w.ceil() as u32, l.h.ceil() as u32),
        exclusive_zone: 0,
        // (top, right, bottom, left) in logical pixels, from the top-left of the output
        margin: (l.y as i32, 0, 0, l.x as i32),
        keyboard: true,
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// The client
// ---------------------------------------------------------------------------------------------------------------------

struct Menubar {
    face: u64,
    model: MenuBarModel,
    nav: Nav,
    width: f32,
    scale: f32,
    bar: Option<SurfaceId>,
    bar_ready: bool,
    bar_dirty: bool,
    popup: Option<SurfaceId>,
    popup_ready: bool,
    popup_dirty: bool,
    popup_spec: Option<SurfaceSpec>,
    /// The last pointer position and the surface it was over.
    pointer: Option<(SurfaceId, f32, f32)>,
}

impl Menubar {
    fn new(face: u64) -> Menubar {
        Menubar {
            face,
            model: MenuBarModel::default(),
            nav: Nav::default(),
            width: 0.0,
            scale: 1.0,
            bar: None,
            bar_ready: false,
            bar_dirty: true,
            popup: None,
            popup_ready: false,
            popup_dirty: true,
            popup_spec: None,
            pointer: None,
        }
    }

    fn layout(&self) -> BarLayout {
        layout_bar(&self.model, self.width, &measurer(self.face))
    }

    fn popup_layout(&self, slot: Slot) -> PopupLayout {
        let anchor = self.layout().item(slot).map_or(0.0, |i| i.x);
        layout_popup(items_of(&self.model, slot), anchor, BAR_HEIGHT_PT, self.width, &measurer(self.face))
    }

    fn changed(&mut self) {
        self.bar_dirty = true;
        self.popup_dirty = true;
    }

    fn set_model(&mut self, model: MenuBarModel) {
        self.model = model;
        self.changed();
    }

    fn event(&mut self, link: &mut Link, e: Event) -> Result<(), String> {
        match e {
            Event::Configured { surface, width, scale, .. } => {
                if Some(surface) == self.bar {
                    self.width = width as f32;
                    self.scale = scale;
                    self.bar_ready = true;
                    self.bar_dirty = true;
                } else if Some(surface) == self.popup {
                    self.popup_ready = true;
                    self.popup_dirty = true;
                }
            }
            Event::Closed(surface) => {
                if Some(surface) == self.bar {
                    return Err("the menu bar surface was closed".into());
                }
                if Some(surface) == self.popup {
                    self.popup = None;
                    self.popup_spec = None;
                    self.popup_ready = false;
                    self.nav.close();
                    self.changed();
                }
            }
            Event::PointerEnter { surface, x, y } | Event::PointerMotion { surface, x, y } => {
                self.pointer = Some((surface, x, y));
                self.hover(surface, x, y);
            }
            Event::PointerLeave(surface) => {
                if self.pointer.map(|p| p.0) == Some(surface) {
                    self.pointer = None;
                }
            }
            Event::PointerButton { surface, button, pressed: true } if button == BTN_LEFT => {
                if let Some((_, x, y)) = self.pointer.filter(|p| p.0 == surface) {
                    self.click(surface, x, y, link)?;
                }
            }
            Event::Key { keysym, pressed: true, .. } => self.key(link, keysym)?,
            _ => {}
        }
        Ok(())
    }

    /// The pointer moved: over the bar, an open menu follows it to the slot under it; over the popup, the row under
    /// it is highlighted.
    fn hover(&mut self, surface: SurfaceId, x: f32, y: f32) {
        if Some(surface) == self.bar {
            if let Some(slot) = self.layout().hit(x) {
                if self.nav.open.is_some() && self.nav.open != Some(slot) {
                    self.nav.open(&self.model, slot);
                    self.changed();
                }
            }
        } else if Some(surface) == self.popup {
            if let Some(slot) = self.nav.open {
                if let Some(row) = self.popup_layout(slot).row_at(y) {
                    self.nav.hover(&self.model, row);
                    self.popup_dirty = true;
                }
            }
        }
    }

    fn click(&mut self, surface: SurfaceId, x: f32, y: f32, link: &mut Link) -> Result<(), String> {
        if Some(surface) == self.bar {
            match self.layout().hit(x) {
                Some(slot) if self.nav.open == Some(slot) => self.nav.close(),
                Some(slot) => self.nav.open(&self.model, slot),
                None => self.nav.close(),
            }
            self.changed();
        } else if Some(surface) == self.popup {
            if let Some(slot) = self.nav.open {
                let items = items_of(&self.model, slot);
                if let Some(row) = self.popup_layout(slot).row_at(y).filter(|&r| items.get(r).is_some_and(selectable)) {
                    self.nav.row = Some(row);
                    if let Some((menu, item)) = self.nav.activate(&self.model) {
                        link.send(&FromChrome::MenuAction { menu, item, generation: self.model.generation }).map_err(|e| format!("shell link: {e}"))?;
                        self.nav.close();
                    }
                }
            }
            self.changed();
        }
        Ok(())
    }

    fn key(&mut self, link: &mut Link, keysym: u32) -> Result<(), String> {
        if self.nav.open.is_none() {
            return Ok(());
        }
        let slots = self.layout().slots();
        match keysym {
            XK_ESCAPE => self.nav.close(),
            XK_DOWN => self.nav.step(&self.model, 1),
            XK_UP => self.nav.step(&self.model, -1),
            XK_LEFT => self.nav.switch(&self.model, &slots, -1),
            XK_RIGHT => self.nav.switch(&self.model, &slots, 1),
            XK_RETURN | XK_KP_ENTER => {
                if let Some((menu, item)) = self.nav.activate(&self.model) {
                    link.send(&FromChrome::MenuAction { menu, item, generation: self.model.generation }).map_err(|e| format!("shell link: {e}"))?;
                    self.nav.close();
                }
            }
            k => {
                if let Some(c) = char::from_u32(k).filter(|c| c.is_ascii_graphic()) {
                    self.nav.type_select(&self.model, c);
                }
            }
        }
        self.changed();
        Ok(())
    }

    /// Draws what changed. Closes an open menu whose slot the model no longer has.
    fn paint(&mut self, d: &mut Display) -> Result<(), String> {
        if let Some(slot) = self.nav.open {
            if !slot_exists(&self.model, slot) {
                self.nav.close();
                self.changed();
            }
        }
        if let (Some(bar), true, true) = (self.bar, self.bar_ready, self.bar_dirty) {
            let layout = self.layout();
            let (w, h) = device(self.width, BAR_HEIGHT_PT, self.scale);
            let mut list = DisplayList::new(w, h);
            list.push_transform(Transform2D::scale(self.scale, self.scale));
            paint_bar(&mut list, self.face, &layout, &self.nav);
            list.pop_transform();
            d.draw(bar, &list, None)?;
            self.bar_dirty = false;
        }
        self.sync_popup(d)
    }

    /// Keeps the popup surface for the open menu: created on open, moved or resized when the menu changes, destroyed
    /// on close, and drawn once it is configured.
    fn sync_popup(&mut self, d: &mut Display) -> Result<(), String> {
        let Some(slot) = self.nav.open else {
            if let Some(id) = self.popup.take() {
                d.destroy(id);
            }
            self.popup_spec = None;
            self.popup_ready = false;
            return Ok(());
        };
        let layout = self.popup_layout(slot);
        let spec = popup_spec(&layout);
        match self.popup {
            None => {
                self.popup = Some(d.create(spec.clone())?);
                self.popup_spec = Some(spec);
                self.popup_ready = false;
            }
            Some(id) => {
                if self.popup_spec.as_ref() != Some(&spec) {
                    d.update(id, spec.clone());
                    self.popup_spec = Some(spec);
                    self.popup_ready = false;
                }
            }
        }
        if let (Some(id), true, true) = (self.popup, self.popup_ready, self.popup_dirty) {
            let (w, h) = device(layout.w, layout.h, self.scale);
            let mut list = DisplayList::new(w, h);
            list.push_transform(Transform2D::scale(self.scale, self.scale));
            paint_popup(&mut list, self.face, &layout, items_of(&self.model, slot), self.nav.row);
            list.pop_transform();
            d.draw(id, &list, None)?;
            self.popup_dirty = false;
        }
        Ok(())
    }
}

/// Runs the client until the shell or the compositor goes away: connects to the shell link and the compositor,
/// makes the bar surface, then waits for models, input and configure events and paints what changed.
pub fn run() -> Result<(), String> {
    let mut link = Link::connect("menubar").map_err(|e| format!("shell link: {e}"))?;
    let mut display = Display::connect()?;
    let mut bar = Menubar::new(register_face(FONT));
    bar.bar = Some(display.create(bar_spec())?);
    loop {
        let events = display.dispatch(-1, Some(link.fd()))?;
        for m in link.recv().map_err(|e| format!("shell link closed: {e}"))? {
            if let ToChrome::MenuBar(model) = m {
                bar.set_model(model);
            }
        }
        for e in events {
            bar.event(&mut link, e)?;
        }
        bar.paint(&mut display)?;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Menu, StatusItem};

    /// Seven pixels per character: easy to check by hand.
    fn mono(t: &str, _px: f32) -> f32 {
        t.chars().count() as f32 * 7.0
    }

    fn item(id: u32, label: &str, enabled: bool) -> MenuItem {
        MenuItem { id, label: label.into(), enabled, ..MenuItem::default() }
    }

    fn sep(id: u32) -> MenuItem {
        MenuItem { id, ..MenuItem::default() }
    }

    fn bar_model() -> MenuBarModel {
        MenuBarModel {
            app_name: "Files".into(),
            menus: vec![
                Menu { id: 0, title: String::new(), items: vec![item(0, "About", true), sep(1), item(2, "Quit", true)] },
                Menu { id: 1, title: "App".into(), items: vec![item(0, "Open", true), item(1, "Save", false)] },
                Menu { id: 2, title: "File".into(), items: vec![item(0, "New", true), sep(1), item(2, "Close", true)] },
            ],
            status: Vec::new(),
            generation: 0,
        }
    }

    #[test]
    fn menus_follow_the_system_mark_with_title_padding() {
        // system mark: x 6, w 34; "App" (3 chars = 21) + 24 = 45 at x 40; "File" (4 chars = 28) + 24 = 52 at x 85
        let l = layout_bar(&bar_model(), 400.0, &mono);
        assert_eq!(l.item(Slot::Menu(0)).map(|i| (i.x, i.w)), Some((6.0, 34.0)));
        let app = l.item(Slot::Menu(1)).unwrap();
        assert_eq!((app.x, app.w, app.text_x), (40.0, 45.0, 52.0));
        let file = l.item(Slot::Menu(2)).unwrap();
        assert_eq!((file.x, file.w, file.text_x), (85.0, 52.0, 97.0));
    }

    #[test]
    fn status_items_are_placed_from_the_right_edge() {
        // clock "Mon 12:00" (9 chars) = 63 + 20 = 83; wifi "wifi" (4) = 28 + 20 = 48; right edge 400 - 16 = 384
        let mut m = bar_model();
        m.status = vec![
            StatusItem { id: 10, text: "Mon 12:00".into(), ..StatusItem::default() },
            StatusItem { id: 11, text: "wifi".into(), ..StatusItem::default() },
        ];
        let l = layout_bar(&m, 400.0, &mono);
        let wifi = l.item(Slot::Status(1)).unwrap();
        assert_eq!((wifi.x, wifi.w, wifi.text_x), (336.0, 48.0, 346.0));
        let clock = l.item(Slot::Status(0)).unwrap();
        assert_eq!((clock.x, clock.w), (253.0, 83.0));
    }

    #[test]
    fn status_items_that_would_touch_the_menus_are_left_out() {
        // menus end at 137; the limit is 145. Width 150 leaves an edge of 134, so the wifi item (48) does not fit.
        let mut m = bar_model();
        m.status = vec![StatusItem { id: 11, text: "wifi".into(), ..StatusItem::default() }];
        let l = layout_bar(&m, 150.0, &mono);
        assert!(l.item(Slot::Status(0)).is_none());
        // at width 200 the edge is 184 and 184 - 48 = 136 < 145, so it is dropped too; at 210 the edge is 194 and
        // 194 - 48 = 146 >= 145, so it fits
        let l = layout_bar(&m, 210.0, &mono);
        assert_eq!(l.item(Slot::Status(0)).map(|i| i.x), Some(146.0));
    }

    #[test]
    fn a_status_icon_without_text_takes_the_icon_width() {
        let mut m = bar_model();
        m.status = vec![StatusItem { id: 12, text: String::new(), ..StatusItem::default() }];
        let l = layout_bar(&m, 400.0, &mono);
        assert_eq!(l.item(Slot::Status(0)).map(|i| (i.x, i.w)), Some((354.0, 30.0)));
    }

    #[test]
    fn hit_finds_the_item_under_the_pointer() {
        let mut m = bar_model();
        m.status = vec![StatusItem { id: 10, text: "Mon 12:00".into(), ..StatusItem::default() }];
        let l = layout_bar(&m, 400.0, &mono);
        assert_eq!(l.hit(10.0), Some(Slot::Menu(0)));
        assert_eq!(l.hit(50.0), Some(Slot::Menu(1)));
        // the clock (83 wide) sits at 384 - 83 = 301..384
        assert_eq!(l.hit(320.0), Some(Slot::Status(0)));
        assert_eq!(l.hit(300.0), None);
        assert_eq!(l.hit(200.0), None);
    }

    #[test]
    fn popup_rows_stack_under_the_bar_with_separators() {
        // About (24), separator (11), Quit (24), padding 5 top and bottom: 5 + 24 + 11 + 24 + 5 = 69 high
        let items = vec![item(0, "About", true), sep(1), item(2, "Print Preview", true)];
        let p = layout_popup(&items, 40.0, 28.0, 400.0, &mono);
        assert_eq!((p.x, p.y), (40.0, 34.0));
        assert_eq!(p.h, 69.0);
        assert_eq!(p.rows[0], RowSpan { top: 5.0, h: 24.0, separator: false, shortcut_x: 0.0 });
        assert_eq!((p.rows[1].top, p.rows[1].h, p.rows[1].separator), (29.0, 11.0, true));
        assert_eq!(p.rows[2].top, 40.0);
        // the separator is a row too; a click on it selects nothing (the caller checks `selectable`)
        assert_eq!(p.row_at(35.0), Some(1));
        assert_eq!(p.row_at(10.0), Some(0));
        assert_eq!(p.row_at(60.0), Some(2));
    }

    #[test]
    fn popup_width_is_the_widest_row_and_has_a_minimum() {
        // "Print Preview" (13 chars) = 91: 18 + 91 + 16 = 125, under the minimum of 180
        let narrow = layout_popup(&[item(0, "Print Preview", true)], 0.0, 28.0, 400.0, &mono);
        assert_eq!(narrow.w, MIN_WIDTH_PT);
        // "Export as PDF" (13) with shortcut "cmd e" (5 = 35): 18 + 91 + 24 + 35 + 16 = 184, over the minimum
        let mut wide = item(0, "Export as PDF", true);
        wide.shortcut = "cmd e".into();
        let p = layout_popup(&[wide], 0.0, 28.0, 400.0, &mono);
        assert_eq!(p.w, 184.0);
        assert_eq!(p.rows[0].shortcut_x, 184.0 - 16.0 - 35.0);
    }

    #[test]
    fn popup_stays_on_the_screen() {
        let items = vec![item(0, "About", true)];
        assert_eq!(layout_popup(&items, 350.0, 28.0, 400.0, &mono).x, 220.0);
        assert_eq!(layout_popup(&items, 0.0, 28.0, 400.0, &mono).x, 0.0);
    }

    #[test]
    fn next_row_skips_separators_and_disabled_rows_and_wraps() {
        let items = vec![item(0, "A", true), sep(1), item(2, "B", false), item(3, "C", true)];
        assert_eq!(first_row(&items), Some(0));
        assert_eq!(next_row(&items, Some(0), 1), Some(3));
        assert_eq!(next_row(&items, Some(3), 1), Some(0));
        assert_eq!(next_row(&items, Some(0), -1), Some(3));
        assert_eq!(next_row(&[sep(0), item(1, "X", false)], None, 1), None);
    }

    #[test]
    fn type_select_jumps_to_the_next_row_starting_with_the_letter() {
        let items = vec![item(0, "Open", true), item(1, "Save", true), item(2, "Save As", true), item(3, "Quit", true)];
        assert_eq!(type_select(&items, None, 's'), Some(1));
        assert_eq!(type_select(&items, Some(1), 's'), Some(2));
        assert_eq!(type_select(&items, Some(2), 'S'), Some(1));
        assert_eq!(type_select(&items, Some(3), 'z'), None);
    }

    #[test]
    fn nav_opens_steps_and_activates_the_highlighted_row() {
        let m = bar_model();
        let mut nav = Nav::default();
        nav.open(&m, Slot::Menu(2));
        assert_eq!(nav.row, Some(0));
        nav.step(&m, 1);
        assert_eq!(nav.row, Some(2));
        // "Close" is item id 2 of menu id 2
        assert_eq!(nav.activate(&m), Some((2, 2)));
        nav.close();
        assert_eq!(nav, Nav::default());
        assert_eq!(nav.activate(&m), None);
    }

    #[test]
    fn nav_switches_between_slots_and_wraps() {
        let m = bar_model();
        let slots = vec![Slot::Menu(0), Slot::Menu(1), Slot::Menu(2)];
        let mut nav = Nav::default();
        nav.open(&m, Slot::Menu(2));
        nav.switch(&m, &slots, 1);
        assert_eq!(nav.open, Some(Slot::Menu(0)));
        nav.switch(&m, &slots, -1);
        assert_eq!(nav.open, Some(Slot::Menu(2)));
    }

    #[test]
    fn nav_never_activates_a_disabled_row() {
        let m = bar_model();
        let mut nav = Nav::default();
        nav.open(&m, Slot::Menu(1));
        // row 0 "Open" is enabled, row 1 "Save" is disabled: a hover on it is refused
        nav.hover(&m, 1);
        assert_eq!(nav.row, Some(0));
        nav.hover(&m, 0);
        assert_eq!(nav.activate(&m), Some((1, 0)));
    }

    #[test]
    fn items_of_and_slot_id_follow_the_model_ids() {
        let mut m = bar_model();
        m.status = vec![StatusItem { id: 9, text: "x".into(), menu: vec![item(0, "Y", true)], ..StatusItem::default() }];
        assert_eq!(items_of(&m, Slot::Status(0)).len(), 1);
        assert!(items_of(&m, Slot::Status(3)).is_empty());
        assert_eq!(slot_id(&m, Slot::Status(0)), Some(9));
        assert_eq!(slot_id(&m, Slot::Menu(2)), Some(2));
        assert!(!slot_exists(&m, Slot::Menu(3)));
    }
}
