//! The Dock client: a bottom (or side) layer-shell surface with an exclusive zone, drawn with `bifrost-2d` through the
//! chrome surface API. Its pure parts (geometry, magnification, launch bounce, badges, progress, drag gestures, the
//! context menu) are host-tested; `run` connects them to the window server and the shell link.
//!
//! Layout is computed in surface-local logical pixels for a bottom Dock (the Dock sits on the bottom edge, its icons
//! grow away from that edge); side Docks map the same numbers onto the left or right edge. Magnification follows the
//! pointer with the cosine falloff of the shell Dock (`desktop_dock::magnify_factor`), quantised to 2 px.
//!
//! Owner: wave G2 agent `chrome-dock`. Known gaps are listed in `docs/graphics/g2-chrome-dock.md`.

use crate::link::Link;
use crate::model::{DockItem, DockModel, DockPosition, FromChrome, ToChrome};
use crate::surface::{Anchor, Display, Event, Layer, SurfaceId, SurfaceSpec};
use bifrost_render::canvas::{register_face, with_face, Canvas, ImageRef, RectF, TextRun, Transform2D};
use bifrost_render::display_list::DisplayList;
use bifrost_render::gfx::Image;
use bifrost_render::motion::{self, Kind};
use bifrost_render::ui;
use std::collections::HashMap;
use std::time::Instant;

/// Gap between the screen edge and the capsule (the shell's `DOCK_MARGIN_PT`).
pub const MARGIN: f32 = 10.0;
/// Padding inside the capsule around the icon row (the shell's `DOCK_PAD_PT`).
pub const PAD: f32 = 10.0;
/// Gap between icons (the shell's `DOCK_GAP_PT`).
pub const GAP: f32 = 10.0;
/// Smallest icon the Dock shrinks to when the row does not fit the output.
pub const MIN_ICON: f32 = 24.0;
/// A launch bounce gives up after this long even when no window maps (the shell's `BOUNCE_MAX_S`).
pub const BOUNCE_MAX_S: f64 = 10.0;
/// Pointer travel before a press becomes a drag, px.
pub const DRAG_THRESHOLD: f32 = 6.0;
/// How far an icon must be dragged off the capsule to remove it from the Dock, px.
pub const REMOVE_DISTANCE: f32 = 80.0;
/// Height of one context menu row and the menu's inner padding, px.
pub const MENU_ROW_H: f32 = 26.0;
pub const MENU_PAD: f32 = 6.0;
/// Width of the context menu surface, px, and the gap between it and the capsule.
pub const MENU_W: f32 = 220.0;
pub const MENU_GAP: f32 = 8.0;
/// Fill of an icon badge: a badge is a count or an alert, the reference platform's red.
pub const BADGE_FILL: u32 = ui::SYSTEM_RED;
/// The seconds one sweep of an indeterminate progress bar takes.
pub const PROGRESS_PERIOD_S: f64 = 1.2;

/// A rectangle in surface-local logical pixels (y down).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub fn new(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect { x, y, w, h }
    }
    pub fn right(&self) -> f32 {
        self.x + self.w
    }
    pub fn bottom(&self) -> f32 {
        self.y + self.h
    }
    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && x < self.right() && y >= self.y && y < self.bottom()
    }
    pub fn to_array(self) -> [f32; 4] {
        [self.x, self.y, self.w, self.h]
    }
    fn offset(self, dx: f32, dy: f32) -> Rect {
        Rect { x: self.x + dx, y: self.y + dy, ..self }
    }
}

/// Pointer magnification: 1 at the pointer, 0 at `radius` and beyond, a cosine in between (the shell's formula).
pub fn magnify_factor(dist: f32, radius: f32) -> f32 {
    if radius <= 0.0 || dist >= radius {
        0.0
    } else {
        0.5 + 0.5 * (std::f32::consts::PI * dist.max(0.0) / radius).cos()
    }
}

/// The side of one icon `dist` px from the pointer, at magnification `amount` (0..1): between `base` and `max`,
/// quantised to 2 px so scaled icons cache well.
pub fn icon_size(base: f32, max: f32, dist: f32, radius: f32, amount: f32) -> f32 {
    let f = magnify_factor(dist, radius) * amount.clamp(0.0, 1.0);
    base + ((max - base).max(0.0) * f / 2.0).round() * 2.0
}

/// Inputs of one layout pass.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LayoutInput {
    /// Items in the Dock.
    pub count: usize,
    /// Resting icon size, px (`DockModel::size`).
    pub size: f32,
    /// Size of the icon under the pointer, px (`DockModel::magnified`); at most `size` disables magnification.
    pub magnified: f32,
    pub position: DockPosition,
    /// The output's length along the edge the Dock sits on (its width for a bottom Dock).
    pub along_len: f32,
    /// The pointer along the edge, while it is over the Dock.
    pub pointer: Option<f32>,
    /// Magnification amount 0..1 (animated by the client).
    pub amount: f32,
}

/// The Dock's geometry for one pass, surface-local.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Layout {
    /// The capsule behind the icons.
    pub bar: Rect,
    /// One rect per item, in list order.
    pub icons: Vec<Rect>,
    /// Icon size at rest, after shrinking to fit.
    pub base: f32,
    /// Surface size requested from the compositor, (width, height).
    pub surface: (f32, f32),
    /// The exclusive zone the capsule reserves from the edge, px.
    pub exclusive: f32,
}

/// The surface size for an edge, the length along the edge and the thickness across it.
fn surface_size(edge: DockPosition, along: f32, across: f32) -> (f32, f32) {
    match edge {
        DockPosition::Bottom => (along, across),
        DockPosition::Left | DockPosition::Right => (across, along),
    }
}

/// Computes the Dock's geometry. The icon row is centred along the edge; magnified icons keep the edge they stand on
/// and spread out from the centre of the row.
pub fn layout(input: &LayoutInput) -> Layout {
    let n = input.count;
    let edge = input.position;
    if n == 0 {
        let across = MARGIN + 2.0 * PAD + input.size;
        return Layout {
            bar: Rect::default(),
            icons: vec![],
            base: input.size,
            surface: surface_size(edge, input.along_len, across),
            exclusive: MARGIN + input.size + 2.0 * PAD,
        };
    }
    // shrink the resting size when the row does not fit the output (margins and padding at both ends, gaps between)
    let fit = (input.along_len - 2.0 * (PAD + MARGIN) - (n as f32 - 1.0) * GAP) / n as f32;
    let base = input.size.min(fit).max(MIN_ICON.min(input.size));
    let k = if input.size > 0.0 { base / input.size } else { 1.0 };
    let max = input.magnified.max(input.size) * k;
    let across = MARGIN + 2.0 * PAD + max;
    let thick = base + 2.0 * PAD;
    let rest_span = n as f32 * base + (n as f32 - 1.0) * GAP;
    let bar_len = rest_span + 2.0 * PAD;
    let bar_start = (input.along_len - bar_len) / 2.0;
    let row_centre = bar_start + bar_len / 2.0;
    let rest_centre = |i: usize| bar_start + PAD + i as f32 * (base + GAP) + base / 2.0;
    let sizes: Vec<f32> = (0..n)
        .map(|i| match input.pointer {
            Some(p) => icon_size(base, max, (rest_centre(i) - p).abs(), max * 1.6, input.amount),
            None => base,
        })
        .collect();
    let span: f32 = sizes.iter().sum::<f32>() + (n as f32 - 1.0) * GAP;
    let mut at = row_centre - span / 2.0;
    let edge_dist = MARGIN + PAD;
    let (sw, sh) = surface_size(edge, input.along_len, across);
    let mut icons = Vec::with_capacity(n);
    for s in sizes {
        icons.push(match edge {
            DockPosition::Bottom => Rect::new(at, sh - edge_dist - s, s, s),
            DockPosition::Left => Rect::new(edge_dist, at, s, s),
            DockPosition::Right => Rect::new(sw - edge_dist - s, at, s, s),
        });
        at += s + GAP;
    }
    let bar = match edge {
        DockPosition::Bottom => Rect::new(bar_start, sh - MARGIN - thick, bar_len, thick),
        DockPosition::Left => Rect::new(MARGIN, bar_start, thick, bar_len),
        DockPosition::Right => Rect::new(sw - MARGIN - thick, bar_start, thick, bar_len),
    };
    Layout { bar, icons, base, surface: (sw, sh), exclusive: MARGIN + thick }
}

/// The bounce offset (dx, dy) of a lift: up for a bottom Dock, inwards for a side one.
pub fn lift_offset(edge: DockPosition, lift: f32) -> (f32, f32) {
    match edge {
        DockPosition::Bottom => (0.0, -lift),
        DockPosition::Left => (lift, 0.0),
        DockPosition::Right => (-lift, 0.0),
    }
}

/// The launch bounce: hops of `hop_s` seconds, lifting an icon of `icon` px by half its size at each peak. Zero
/// before the bounce starts, after `BOUNCE_MAX_S`, and when the hop is zero (Reduce Motion).
pub fn bounce_lift(elapsed: f64, hop_s: f64, icon: f32) -> f32 {
    if hop_s <= 0.0 || elapsed < 0.0 || elapsed > BOUNCE_MAX_S {
        return 0.0;
    }
    let ph = (elapsed / hop_s).fract() as f32;
    icon * 0.5 * 4.0 * ph * (1.0 - ph)
}

/// The hop length from the motion table, seconds (zero under Reduce Motion).
pub fn bounce_hop_s() -> f64 {
    motion::duration(Kind::DockBounce) as f64
}

/// The badge pill for `text` on an icon: top right, overlapping the corner. None for an empty badge. The pill is as
/// high as 40% of the icon (at least 14 px) and as wide as its text plus room for a round end.
pub fn badge_rect(icon: Rect, text: &str, base: f32) -> Option<Rect> {
    if text.is_empty() {
        return None;
    }
    let h = (base * 0.4).max(14.0);
    let chars = text.chars().count() as f32;
    let w = (chars * 0.6 * h + h * 0.8).max(h);
    Some(Rect::new(icon.right() - w * 0.6, icon.y - h * 0.3, w, h))
}

/// The fill of a progress bar in `track`: a fraction (filled from the left), or, when indeterminate (None), a
/// segment of 30% of the track sweeping across once per `period_s` seconds. None when nothing is visible.
pub fn progress_fill(track: Rect, fraction: Option<f32>, t_s: f64, period_s: f64) -> Option<Rect> {
    match fraction {
        Some(f) => {
            let f = f.clamp(0.0, 1.0);
            (f > 0.0).then(|| Rect::new(track.x, track.y, track.w * f, track.h))
        }
        None => {
            if period_s <= 0.0 {
                return None;
            }
            let seg = track.w * 0.3;
            let phase = (t_s / period_s).rem_euclid(1.0) as f32;
            let x = track.x - seg + (track.w + seg) * phase;
            let left = x.max(track.x);
            let right = (x + seg).min(track.right());
            (right > left).then(|| Rect::new(left, track.y, right - left, track.h))
        }
    }
}

/// What a press on an icon turned into.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Outcome {
    /// A click: activate the item.
    Click,
    /// Dragged to a new slot: move the item after or before the slot's item.
    Move { target: usize, after: bool },
    /// Dragged off the Dock: remove it from the Dock.
    Remove,
    /// Dragged and dropped where it was.
    Nothing,
}

/// A press on an icon, followed by pointer motion and a release.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Gesture {
    pub index: usize,
    start: (f32, f32),
    pos: (f32, f32),
    pub dragging: bool,
}

impl Gesture {
    /// A press on item `index` at (`along`, `lift`): `along` is the pointer along the edge, `lift` how far it is past
    /// the capsule's inner edge, away from the screen edge.
    pub fn press(index: usize, along: f32, lift: f32) -> Gesture {
        Gesture { index, start: (along, lift), pos: (along, lift), dragging: false }
    }

    /// Pointer motion; the press becomes a drag after `DRAG_THRESHOLD` px.
    pub fn move_to(&mut self, along: f32, lift: f32) {
        self.pos = (along, lift);
        let (dx, dy) = (along - self.start.0, lift - self.start.1);
        if (dx * dx + dy * dy).sqrt() >= DRAG_THRESHOLD {
            self.dragging = true;
        }
    }

    /// The release: `centres` are the icons' centres along the edge at rest, in list order.
    pub fn release(self, centres: &[f32]) -> Outcome {
        if !self.dragging {
            return Outcome::Click;
        }
        if self.pos.1 > REMOVE_DISTANCE {
            return Outcome::Remove;
        }
        let target = centres
            .iter()
            .enumerate()
            .min_by(|a, b| (a.1 - self.pos.0).abs().total_cmp(&(b.1 - self.pos.0).abs()))
            .map(|(i, _)| i);
        match target {
            Some(t) if t == self.index => Outcome::Nothing,
            Some(t) => Outcome::Move { target: t, after: t > self.index },
            None => Outcome::Nothing,
        }
    }
}

/// One entry of an item's context menu. `command` is the `FromChrome::DockCommand` it sends.
#[derive(Debug, Clone, PartialEq)]
pub struct MenuEntry {
    pub command: &'static str,
    pub label: &'static str,
    pub enabled: bool,
    pub checked: bool,
}

/// The context menu of an item: Open / Show, Keep in Dock or Remove from Dock (the pin), and Quit while it runs.
pub fn context_menu(running: bool, pinned: bool) -> Vec<MenuEntry> {
    let mut v = vec![MenuEntry { command: "show", label: "Open / Show", enabled: true, checked: false }];
    if pinned {
        v.push(MenuEntry { command: "remove", label: "Remove from Dock", enabled: true, checked: true });
    } else {
        v.push(MenuEntry { command: "keep", label: "Keep in Dock", enabled: true, checked: false });
    }
    if running {
        v.push(MenuEntry { command: "quit", label: "Quit", enabled: true, checked: false });
    }
    v
}

/// The menu popup's rect in the menu surface's coordinates for `entries` rows: width `MENU_W`, height the rows plus
/// padding. Its left edge is centred on the icon at `anchor_along` and kept on the output.
pub fn menu_rect(anchor_along: f32, along_len: f32, entries: usize) -> Rect {
    let h = 2.0 * MENU_PAD + entries as f32 * MENU_ROW_H;
    let x = (anchor_along - MENU_W / 2.0).clamp(0.0, (along_len - MENU_W).max(0.0));
    Rect::new(x, 0.0, MENU_W, h)
}

/// Which row of a menu surface a surface-local y is over.
pub fn menu_row(y: f32, rows: usize) -> Option<usize> {
    if y < MENU_PAD {
        return None;
    }
    let r = ((y - MENU_PAD) / MENU_ROW_H).floor() as usize;
    (r < rows).then_some(r)
}

/// The item index under a point (surface-local), if any.
pub fn hit(icons: &[Rect], x: f32, y: f32) -> Option<usize> {
    icons.iter().position(|r| r.contains(x, y))
}

/// Eases `cur` toward `target` at a rate that does not depend on the frame rate (about 14 per second).
pub fn approach(cur: f32, target: f32, dt_s: f64) -> f32 {
    let k = 1.0 - (-(dt_s.max(0.0)) * 14.0).exp() as f32;
    cur + (target - cur) * k
}

/// An icon's rect as a minimize target, in output logical pixels: the rect shifted by where the surface sits.
pub fn minimize_rect(icon: Rect, origin: (f32, f32)) -> [f32; 4] {
    [icon.x + origin.0, icon.y + origin.1, icon.w, icon.h]
}

/// Whether a minimize target moved enough to publish again (a quarter of a pixel is not worth a message).
pub fn target_changed(prev: Option<[f32; 4]>, now: [f32; 4]) -> bool {
    match prev {
        None => true,
        Some(p) => p.iter().zip(now.iter()).any(|(a, b)| (a - b).abs() > 0.25),
    }
}

/// The surface spec of the Dock for a layout: anchored to its edge, with the capsule's exclusive zone unless it hides.
pub fn dock_spec(lay: &Layout, position: DockPosition, autohide: bool) -> SurfaceSpec {
    let (anchor, size) = match position {
        DockPosition::Bottom => (Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT, (0, lay.surface.1.ceil() as u32)),
        DockPosition::Left => (Anchor::LEFT | Anchor::TOP | Anchor::BOTTOM, (lay.surface.0.ceil() as u32, 0)),
        DockPosition::Right => (Anchor::RIGHT | Anchor::TOP | Anchor::BOTTOM, (lay.surface.0.ceil() as u32, 0)),
    };
    SurfaceSpec {
        namespace: "bifrost-dock".into(),
        layer: Layer::Top,
        anchor,
        size,
        exclusive_zone: if autohide { 0 } else { lay.exclusive.round() as i32 },
        margin: (0, 0, 0, 0),
        keyboard: false,
    }
}

/// The Dock's state as the client keeps it between events.
#[derive(Default)]
pub struct DockState {
    pub model: DockModel,
    /// Decoded icons, one per item (None draws a placeholder tile).
    pub images: Vec<Option<Image>>,
    /// Pointer along the edge while over the Dock, and its surface-local position.
    pub pointer: Option<f32>,
    pub pointer_xy: (f32, f32),
    /// Magnification, animated toward 1 under the pointer and 0 otherwise.
    pub amount: f32,
    /// Launch bounces: (item index, start seconds).
    pub bounces: Vec<(usize, f64)>,
    pub gesture: Option<Gesture>,
}

impl DockState {
    /// Replaces the model. Bounces of items that are gone are dropped; icons are decoded again.
    pub fn set_model(&mut self, model: DockModel) {
        self.bounces.retain(|(i, _)| *i < model.items.len());
        self.images = model.items.iter().map(load_icon).collect();
        self.model = model;
    }

    /// Whether anything animates (magnification still moving, a launch bounce running), so the loop keeps drawing.
    pub fn animating(&self, now_s: f64) -> bool {
        let target = if self.pointer.is_some() { 1.0 } else { 0.0 };
        (self.amount - target).abs() > 0.001 || self.bounces.iter().any(|(_, s)| now_s - s < BOUNCE_MAX_S)
    }
}

/// Loads an item's icon: an image file at its path (PNG or the shell's raw format), or None.
fn load_icon(item: &DockItem) -> Option<Image> {
    if item.icon_path.is_empty() {
        return None;
    }
    let bytes = std::fs::read(&item.icon_path).ok()?;
    bifrost_render::imgdec::decode(&bytes)
}

/// The face the Dock draws its text with (badges and menus), registered once per process.
fn face() -> u64 {
    static DATA: &[u8] = include_bytes!("../../assets/SFUI.ttf");
    register_face(DATA)
}

/// Draws the Dock into a fresh display list of `size` device pixels, at `scale` device pixels per logical pixel.
pub fn paint(state: &DockState, lay: &Layout, now_s: f64, size: (u32, u32), scale: f32) -> DisplayList {
    let mut list = DisplayList::new(size.0, size.1);
    let c: &mut dyn Canvas = &mut list;
    c.push_transform(Transform2D::scale(scale, scale));
    paint_into(c, state, lay, now_s);
    c.pop_transform();
    list
}

fn paint_into(c: &mut dyn Canvas, state: &DockState, lay: &Layout, now_s: f64) {
    let edge = state.model.position;
    let bar = lay.bar;
    c.fill_rrect(to_rectf(bar), bar.w.min(bar.h) / 2.0, ui::DOCK_TINT.now());
    let face = face();
    for (i, rect) in lay.icons.iter().enumerate() {
        let Some(item) = state.model.items.get(i) else { continue };
        // the launch bounce lifts the icon while its app starts
        let lift = state
            .bounces
            .iter()
            .find(|(b, _)| *b == i)
            .map(|(_, s)| bounce_lift(now_s - s, bounce_hop_s(), lay.base))
            .unwrap_or(0.0);
        let (dx, dy) = lift_offset(edge, lift);
        let rr = rect.offset(dx, dy);
        match state.images.get(i).and_then(Option::as_ref) {
            Some(img) => c.image(ImageRef { id: i as u64 + 1, image: img }, to_rectf(rr), 1.0),
            None => c.fill_rrect(to_rectf(rr), rr.w * 0.225, ui::FILL_GRAY2),
        }
        if item.running {
            let d = 4.0;
            let dot = match edge {
                DockPosition::Bottom => RectF::new(rr.x + rr.w / 2.0 - d / 2.0, rr.bottom() + 3.0, d, d),
                DockPosition::Left => RectF::new(rr.right() + 3.0, rr.y + rr.h / 2.0 - d / 2.0, d, d),
                DockPosition::Right => RectF::new(rr.x - 3.0 - d, rr.y + rr.h / 2.0 - d / 2.0, d, d),
            };
            c.fill_rrect(dot, d / 2.0, if item.active { ui::WHITE } else { ui::SECONDARY });
        }
        if let Some(b) = badge_rect(rr, &item.badge, lay.base) {
            c.fill_rrect(to_rectf(b), b.h / 2.0, BADGE_FILL);
            let px = ui::MAC_CAPTION1_PT;
            let width = with_face(face, |f| f.measure(&item.badge, px)).unwrap_or(0.0);
            let run = TextRun {
                face,
                text: item.badge.clone(),
                px,
                vars: vec![],
                x: b.x + (b.w - width) / 2.0,
                y: b.y + (b.h - px) / 2.0,
            };
            c.text(&run, ui::WHITE);
        }
        if item.progress >= 0.0 {
            let track = Rect::new(rr.x + rr.w * 0.15, rr.bottom() - 12.0, rr.w * 0.7, 4.0);
            c.fill_rrect(to_rectf(track), 2.0, ui::FILL_GRAY);
            if let Some(fill) = progress_fill(track, Some(item.progress), now_s, PROGRESS_PERIOD_S) {
                c.fill_rrect(to_rectf(fill), 2.0, ui::TINT);
            }
        }
    }
}

/// Paints the context menu surface: its rows, the hovered one highlighted when it is enabled.
pub fn paint_menu(entries: &[MenuEntry], hover: Option<usize>, size: (u32, u32), scale: f32) -> DisplayList {
    let mut list = DisplayList::new(size.0, size.1);
    let c: &mut dyn Canvas = &mut list;
    c.push_transform(Transform2D::scale(scale, scale));
    let (w, h) = (size.0 as f32 / scale, size.1 as f32 / scale);
    c.fill_rrect(RectF::new(0.0, 0.0, w, h), 9.0, ui::MENU_TINT.now());
    let face = face();
    for (i, e) in entries.iter().enumerate() {
        let row = Rect::new(MENU_PAD, MENU_PAD + i as f32 * MENU_ROW_H, w - 2.0 * MENU_PAD, MENU_ROW_H);
        let selected = hover == Some(i) && e.enabled;
        if selected {
            c.fill_rrect(to_rectf(row), 5.0, ui::MENU_HIGHLIGHT);
        }
        let color = if selected {
            ui::MENU_HIGHLIGHT_TEXT
        } else if e.enabled {
            ui::LABEL.now()
        } else {
            ui::LABEL_2.now()
        };
        let run = TextRun {
            face,
            text: e.label.into(),
            px: ui::MAC_BODY_PT,
            vars: vec![],
            x: row.x + 10.0,
            y: row.y + (MENU_ROW_H - ui::MAC_BODY_PT) / 2.0,
        };
        c.text(&run, color);
    }
    c.pop_transform();
    list
}

fn to_rectf(r: Rect) -> RectF {
    RectF::new(r.x, r.y, r.w, r.h)
}

/// The context menu surface while it is open.
struct MenuSurface {
    id: SurfaceId,
    /// The item it was opened on, and its entries.
    item: usize,
    entries: Vec<MenuEntry>,
    /// The row under the pointer.
    hover: Option<usize>,
    /// Set once the compositor has configured it; nothing is drawn before.
    ready: bool,
}

/// `FromChrome::DockCommand` for a command on an item.
fn command_message(app_id: &str, command: &str) -> FromChrome {
    FromChrome::DockCommand { app_id: app_id.into(), command: command.into() }
}

/// Runs the client until the shell or the compositor goes away.
pub fn run() -> Result<(), String> {
    let mut display = Display::connect()?;
    let mut link = Link::connect("dock").map_err(|e| format!("shell link: {e}"))?;
    let mut state = DockState::default();
    let started = Instant::now();
    let mut last = 0.0f64;
    let mut surface: Option<SurfaceId> = None;
    let mut along_len = 0.0f32;
    let mut scale = 1.0f32;
    let mut lay = Layout::default();
    let mut published: HashMap<String, [f32; 4]> = HashMap::new();
    let mut menu: Option<MenuSurface> = None;
    loop {
        let now = started.elapsed().as_secs_f64();
        let wait = if state.animating(now) { 16 } else { -1 };
        let events = display.dispatch(wait, Some(link.fd()))?;
        for m in link.recv().map_err(|e| format!("shell link: {e}"))? {
            if let ToChrome::Dock(model) = m {
                state.set_model(model);
            }
        }
        let mut dirty = false;
        let mut menu_dirty = false;
        for ev in events {
            match ev {
                Event::Configured { surface: s, width, height, scale: sc } => {
                    if Some(s) == surface {
                        surface = Some(s);
                        scale = sc.max(1.0);
                        along_len = match state.model.position {
                            DockPosition::Bottom => width as f32,
                            _ => height as f32,
                        };
                        dirty = true;
                    } else if let Some(m) = menu.as_mut().filter(|m| m.id == s) {
                        m.ready = true;
                        menu_dirty = true;
                    }
                }
                Event::Closed(s) if Some(s) == surface => return Ok(()),
                Event::Closed(s) if menu.as_ref().is_some_and(|m| m.id == s) => menu = None,
                Event::PointerEnter { surface: s, x, y } | Event::PointerMotion { surface: s, x, y } => {
                    if Some(s) == surface {
                        let (along, lift) = to_axis(state.model.position, x, y, &lay);
                        state.pointer = Some(along);
                        state.pointer_xy = (x, y);
                        if let Some(g) = state.gesture.as_mut() {
                            g.move_to(along, lift);
                        }
                        dirty = true;
                    } else if let Some(m) = menu.as_mut().filter(|m| m.id == s) {
                        m.hover = menu_row(y, m.entries.len());
                        menu_dirty = true;
                    }
                }
                Event::PointerLeave(s) => {
                    if Some(s) == surface {
                        state.pointer = None;
                        dirty = true;
                    } else if let Some(m) = menu.as_mut().filter(|m| m.id == s) {
                        m.hover = None;
                        menu_dirty = true;
                    }
                }
                Event::PointerButton { surface: s, button, pressed } => {
                    if let Some(m) = menu.as_ref().filter(|m| m.id == s) {
                        // a press on a menu row runs its command; any press on the menu closes it
                        if pressed && button == 0x110 {
                            if let Some(row) = m.hover {
                                let app = state.model.items.get(m.item).map(|it| it.app_id.clone()).unwrap_or_default();
                                let _ = link.send(&command_message(&app, m.entries[row].command));
                            }
                        }
                        if pressed {
                            if let Some(old) = menu.take() {
                                display.destroy(old.id);
                            }
                        }
                        continue;
                    }
                    if Some(s) != surface {
                        continue;
                    }
                    let (x, y) = state.pointer_xy;
                    if pressed && button == 0x110 {
                        if let Some(old) = menu.take() {
                            display.destroy(old.id);
                        }
                        if let Some(i) = hit(&lay.icons, x, y) {
                            let (along, lift) = to_axis(state.model.position, x, y, &lay);
                            state.gesture = Some(Gesture::press(i, along, lift));
                        }
                    } else if !pressed && button == 0x110 {
                        if let Some(g) = state.gesture.take() {
                            let centres: Vec<f32> = lay.icons.iter().map(|r| axis_centre(state.model.position, r)).collect();
                            if let Some(item) = state.model.items.get(g.index) {
                                let app = item.app_id.clone();
                                let running = item.running;
                                match g.release(&centres) {
                                    Outcome::Click => {
                                        if !running {
                                            state.bounces.push((g.index, now));
                                        }
                                        let _ = link.send(&FromChrome::DockActivate { app_id: app });
                                    }
                                    Outcome::Remove => {
                                        let _ = link.send(&command_message(&app, "remove"));
                                    }
                                    Outcome::Move { target, after } => {
                                        if let Some(t) = state.model.items.get(target) {
                                            let cmd = format!("move:{}:{}", t.app_id, after as u8);
                                            let _ = link.send(&command_message(&app, &cmd));
                                        }
                                    }
                                    Outcome::Nothing => {}
                                }
                            }
                        }
                    } else if pressed && button == 0x111 && state.model.position == DockPosition::Bottom {
                        if let Some(i) = hit(&lay.icons, x, y) {
                            let item = &state.model.items[i];
                            let entries = context_menu(item.running, item.pinned);
                            let rect = menu_rect(axis_centre(state.model.position, &lay.icons[i]), along_len, entries.len());
                            let spec = SurfaceSpec {
                                namespace: "bifrost-dock-menu".into(),
                                layer: Layer::Top,
                                anchor: Anchor::BOTTOM | Anchor::LEFT,
                                size: (MENU_W as u32, rect.h.ceil() as u32),
                                exclusive_zone: 0,
                                // (top, right, bottom, left): the menu stands above the capsule, at the icon's column
                                margin: (0, 0, (MARGIN + lay.bar.h + MENU_GAP) as i32, rect.x as i32),
                                keyboard: false,
                            };
                            if let Some(old) = menu.take() {
                                display.destroy(old.id);
                            }
                            let id = display.create(spec)?;
                            menu = Some(MenuSurface { id, item: i, entries, hover: None, ready: false });
                            menu_dirty = true;
                        }
                    }
                    dirty = true;
                }
                _ => {}
            }
        }
        // magnification eases toward the pointer; the launch bounce runs on the same clock
        let target = if state.pointer.is_some() { 1.0 } else { 0.0 };
        let dt = now - last;
        last = now;
        let next = approach(state.amount, target, dt);
        if (next - state.amount).abs() > 1e-4 {
            state.amount = next;
            dirty = true;
        }
        if state.animating(now) {
            dirty = true;
        }
        if let Some(m) = menu.as_ref().filter(|m| m.ready && menu_dirty) {
            let h = (2.0 * MENU_PAD + m.entries.len() as f32 * MENU_ROW_H).ceil();
            let size = ((MENU_W * scale).round() as u32, (h * scale).round() as u32);
            let list = paint_menu(&m.entries, m.hover, size, scale);
            display.draw(m.id, &list, None)?;
        }
        let Some(id) = surface else { continue };
        if along_len <= 0.0 {
            continue;
        }
        let input = LayoutInput {
            count: state.model.items.len(),
            size: state.model.size,
            magnified: state.model.magnified,
            position: state.model.position,
            along_len,
            pointer: state.pointer.filter(|_| state.model.magnified > state.model.size),
            amount: state.amount,
        };
        let new_lay = layout(&input);
        if new_lay.surface != lay.surface {
            display.update(id, dock_spec(&new_lay, state.model.position, state.model.autohide));
        }
        lay = new_lay;
        if !dirty {
            continue;
        }
        // publish where each icon is, so the window server can aim minimize animations at it
        for (i, item) in state.model.items.iter().enumerate() {
            if let Some(icon) = lay.icons.get(i) {
                let rect = minimize_rect(*icon, (0.0, 0.0));
                if target_changed(published.get(&item.app_id).copied(), rect) {
                    published.insert(item.app_id.clone(), rect);
                    let _ = link.send(&FromChrome::MinimizeTarget { app_id: item.app_id.clone(), rect });
                }
            }
        }
        let device = ((lay.surface.0 * scale).round() as u32, (lay.surface.1 * scale).round() as u32);
        let list = paint(&state, &lay, now, device, scale);
        display.draw(id, &list, None)?;
    }
}

/// The pointer's position along the edge and how far it is past the capsule's inner edge (away from the screen edge).
fn to_axis(edge: DockPosition, x: f32, y: f32, lay: &Layout) -> (f32, f32) {
    match edge {
        DockPosition::Bottom => (x, lay.bar.y - y),
        DockPosition::Left => (y, x - lay.bar.right()),
        DockPosition::Right => (y, lay.bar.x - x),
    }
}

/// An icon's centre along the edge.
fn axis_centre(edge: DockPosition, r: &Rect) -> f32 {
    match edge {
        DockPosition::Bottom => r.x + r.w / 2.0,
        _ => r.y + r.h / 2.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn items(n: usize) -> DockModel {
        DockModel {
            items: (0..n).map(|i| DockItem { app_id: format!("app{i}"), ..DockItem::default() }).collect(),
            size: 48.0,
            magnified: 96.0,
            position: DockPosition::Bottom,
            autohide: false,
        }
    }

    fn input(count: usize, pointer: Option<f32>, amount: f32) -> LayoutInput {
        LayoutInput {
            count,
            size: 48.0,
            magnified: 96.0,
            position: DockPosition::Bottom,
            along_len: 1000.0,
            pointer,
            amount,
        }
    }

    #[test]
    fn magnify_falls_off_with_a_cosine() {
        assert_eq!(magnify_factor(0.0, 100.0), 1.0);
        assert!((magnify_factor(50.0, 100.0) - 0.5).abs() < 1e-6);
        assert_eq!(magnify_factor(100.0, 100.0), 0.0);
        assert_eq!(magnify_factor(500.0, 100.0), 0.0);
        assert_eq!(magnify_factor(10.0, 0.0), 0.0);
    }

    #[test]
    fn icon_size_is_quantised_between_base_and_max() {
        assert_eq!(icon_size(48.0, 96.0, 0.0, 150.0, 1.0), 96.0);
        assert_eq!(icon_size(48.0, 96.0, 150.0, 150.0, 1.0), 48.0);
        assert_eq!(icon_size(48.0, 96.0, 75.0, 150.0, 1.0), 72.0);
        assert_eq!(icon_size(48.0, 96.0, 0.0, 150.0, 0.5), 72.0);
        assert_eq!(icon_size(48.0, 96.0, 0.0, 150.0, 0.0), 48.0);
    }

    #[test]
    fn resting_row_is_centred_with_padding_and_a_margin() {
        // 3 icons: row 3*48 + 2*10 = 164, capsule 164 + 2*10 = 184 long, starting at (1000 - 184) / 2 = 408
        let lay = layout(&input(3, None, 0.0));
        assert_eq!(lay.bar.x, 408.0);
        assert_eq!(lay.bar.w, 184.0);
        assert_eq!(lay.icons[0].x, 418.0);
        assert_eq!(lay.icons[1].x, 476.0);
        assert_eq!(lay.icons[2].x, 534.0);
        // the surface is 10 + 2*10 + 96 = 126 tall; the capsule is 48 + 20 = 68 tall, 10 px above the bottom edge
        assert_eq!(lay.surface, (1000.0, 126.0));
        assert_eq!(lay.bar.h, 68.0);
        assert_eq!(lay.bar.y, 48.0);
        assert_eq!(lay.bar.bottom(), 116.0);
        // icons stand 10 px above the capsule's bottom edge (its padding)
        assert_eq!(lay.icons[0].bottom(), 106.0);
        assert_eq!(lay.exclusive, 78.0);
    }

    #[test]
    fn magnified_row_keeps_the_bottom_edge_and_stays_centred() {
        let rest = layout(&input(5, None, 0.0));
        // icon 2 rests at x 476, centre 500: the pointer is over it
        let mag = layout(&input(5, Some(500.0), 1.0));
        assert_eq!(mag.icons[2].w, 96.0);
        assert!(mag.icons[2].w > mag.icons[1].w && mag.icons[1].w > mag.icons[0].w);
        assert_eq!(mag.icons[2].bottom(), rest.icons[2].bottom(), "icons keep the bottom edge");
        let span = |l: &Layout| l.icons[4].right() - l.icons[0].x;
        assert!(span(&mag) > span(&rest), "the row spreads");
        let centre = |l: &Layout| (l.icons[0].x + l.icons[4].right()) / 2.0;
        assert!((centre(&mag) - centre(&rest)).abs() <= 1.0, "centred on the same point");
    }

    #[test]
    fn side_docks_mirror_the_bottom_layout() {
        let mut inp = input(2, None, 0.0);
        inp.position = DockPosition::Left;
        inp.along_len = 800.0;
        let lay = layout(&inp);
        // the capsule is against the left edge, icons inset by margin + padding; the surface is across-wide
        assert_eq!(lay.bar.x, MARGIN);
        assert_eq!(lay.icons[0].x, MARGIN + PAD);
        assert_eq!(lay.surface, (126.0, 800.0));
        // 2 icons: capsule 2*48 + 10 + 20 = 126 long, centred on 800: starts at 337
        assert_eq!(lay.bar.y, 337.0);
        let mut right = inp;
        right.position = DockPosition::Right;
        let r = layout(&right);
        assert_eq!(r.bar.x, 126.0 - MARGIN - 68.0);
        assert_eq!(r.icons[0].x, 126.0 - MARGIN - PAD - 48.0);
    }

    #[test]
    fn shrinks_the_icons_when_the_row_does_not_fit() {
        let mut inp = input(20, None, 0.0);
        inp.along_len = 300.0;
        let lay = layout(&inp);
        // (300 - 40 - 19*10) / 20 = 3.5 px, clamped up to the minimum of 24 px
        assert_eq!(lay.base, MIN_ICON);
        assert!(lay.icons.iter().all(|r| r.w == MIN_ICON));
    }

    #[test]
    fn launch_bounce_peaks_at_half_an_icon_then_stops() {
        assert_eq!(bounce_lift(0.0, 0.7, 48.0), 0.0);
        assert!((bounce_lift(0.35, 0.7, 48.0) - 24.0).abs() < 1e-4, "the middle of a hop lifts half the icon");
        assert_eq!(bounce_lift(-1.0, 0.7, 48.0), 0.0);
        assert_eq!(bounce_lift(11.0, 0.7, 48.0), 0.0, "gives up after the timeout");
        assert_eq!(bounce_lift(0.35, 0.0, 48.0), 0.0, "no bounce under Reduce Motion");
        assert_eq!(lift_offset(DockPosition::Bottom, 24.0), (0.0, -24.0));
        assert_eq!(lift_offset(DockPosition::Left, 24.0), (24.0, 0.0));
    }

    #[test]
    fn badge_pill_hangs_off_the_top_right_corner() {
        let icon = Rect::new(100.0, 100.0, 48.0, 48.0);
        assert_eq!(badge_rect(icon, "", 48.0), None);
        let b = badge_rect(icon, "3", 48.0).unwrap();
        // h = 0.4 * 48 = 19.2; one character: 0.6*19.2 + 0.8*19.2 = 26.88 wide
        assert!((b.h - 19.2).abs() < 1e-4);
        assert!((b.w - 26.88).abs() < 1e-4);
        assert!((b.x - (148.0 - 0.6 * 26.88)).abs() < 1e-3);
        assert!((b.y - (100.0 - 0.3 * 19.2)).abs() < 1e-3);
        let small = badge_rect(icon, "3", 24.0).unwrap();
        assert_eq!(small.h, 14.0, "never smaller than 14 px");
    }

    #[test]
    fn progress_fills_a_fraction_or_sweeps_when_indeterminate() {
        let track = Rect::new(0.0, 0.0, 100.0, 4.0);
        assert_eq!(progress_fill(track, Some(0.5), 0.0, 1.2), Some(Rect::new(0.0, 0.0, 50.0, 4.0)));
        assert_eq!(progress_fill(track, Some(0.0), 0.0, 1.2), None);
        assert_eq!(progress_fill(track, Some(2.0), 0.0, 1.2), Some(track), "clamped to the track");
        // indeterminate: a 30 px segment enters from the left; half way through the period it is at x = 35
        assert_eq!(progress_fill(track, None, 0.0, 1.2), None, "at the start it is off the track");
        let mid = progress_fill(track, None, 0.6, 1.2).unwrap();
        assert!((mid.x - 35.0).abs() < 1e-3 && (mid.w - 30.0).abs() < 1e-3);
        assert_eq!(progress_fill(track, None, 0.6, 0.0), None);
    }

    #[test]
    fn a_click_and_the_drag_outcomes() {
        let centres = [24.0, 82.0, 140.0, 198.0];
        assert_eq!(Gesture::press(1, 82.0, 0.0).release(&centres), Outcome::Click);
        // a small wobble under the threshold is still a click
        let mut g = Gesture::press(1, 82.0, 0.0);
        g.move_to(84.0, 0.0);
        assert!(!g.dragging);
        assert_eq!(g.release(&centres), Outcome::Click);
        // dragged to the last slot: move after it
        let mut g = Gesture::press(1, 82.0, 0.0);
        g.move_to(190.0, 2.0);
        assert!(g.dragging);
        assert_eq!(g.release(&centres), Outcome::Move { target: 3, after: true });
        // dragged to the first slot: move before it
        let mut g = Gesture::press(2, 140.0, 0.0);
        g.move_to(20.0, 0.0);
        assert_eq!(g.release(&centres), Outcome::Move { target: 0, after: false });
        // dropped in its own slot: nothing happens
        let mut g = Gesture::press(2, 140.0, 0.0);
        g.move_to(150.0, 1.0);
        assert_eq!(g.release(&centres), Outcome::Nothing);
        // dragged more than 80 px off the capsule: remove
        let mut g = Gesture::press(0, 24.0, 0.0);
        g.move_to(24.0, 90.0);
        assert_eq!(g.release(&centres), Outcome::Remove);
    }

    #[test]
    fn context_menu_follows_running_and_pinned() {
        let cmds = |v: Vec<MenuEntry>| v.into_iter().map(|e| e.command).collect::<Vec<_>>();
        assert_eq!(cmds(context_menu(true, true)), vec!["show", "remove", "quit"]);
        assert_eq!(cmds(context_menu(false, false)), vec!["show", "keep"]);
        assert_eq!(cmds(context_menu(true, false)), vec!["show", "keep", "quit"]);
        let pinned = context_menu(true, true);
        assert!(pinned[1].checked, "a pinned item shows its pin checked");
        assert!(pinned.iter().all(|e| e.enabled));
    }

    #[test]
    fn menu_is_centred_on_its_icon_and_kept_on_the_output() {
        let r = menu_rect(500.0, 1000.0, 3);
        assert_eq!(r.w, MENU_W);
        assert_eq!(r.h, 2.0 * MENU_PAD + 3.0 * MENU_ROW_H);
        assert_eq!(r.x, 500.0 - MENU_W / 2.0);
        assert_eq!(menu_rect(10.0, 1000.0, 2).x, 0.0, "clamped to the left");
        assert_eq!(menu_rect(990.0, 1000.0, 2).x, 1000.0 - MENU_W, "clamped to the right");
        // rows start below the padding
        assert_eq!(menu_row(3.0, 3), None);
        assert_eq!(menu_row(6.0, 3), Some(0));
        assert_eq!(menu_row(6.0 + 2.0 * MENU_ROW_H + 1.0, 3), Some(2));
        assert_eq!(menu_row(6.0 + 3.0 * MENU_ROW_H + 1.0, 3), None);
    }

    #[test]
    fn hit_approach_and_publishing() {
        let lay = layout(&input(3, None, 0.0));
        assert_eq!(hit(&lay.icons, 430.0, 80.0), Some(0));
        assert_eq!(hit(&lay.icons, 0.0, 0.0), None);
        // approach moves a fixed fraction per 0.1 s, never overshoots, and converges
        let a = approach(0.0, 1.0, 0.1);
        assert!(a > 0.0 && a < 1.0);
        assert!(approach(0.0, 1.0, 0.0).abs() < 1e-6, "no time, no move");
        assert!(approach(0.0, 1.0, 10.0) > 0.9999);
        // minimize targets are the icon's rect shifted by the surface origin
        assert_eq!(minimize_rect(Rect::new(10.0, 20.0, 48.0, 48.0), (0.0, 900.0)), [10.0, 920.0, 48.0, 48.0]);
        assert!(target_changed(None, [0.0; 4]));
        assert!(!target_changed(Some([1.0, 2.0, 3.0, 4.0]), [1.1, 2.0, 3.0, 4.0]), "a tenth of a pixel is not published");
        assert!(target_changed(Some([1.0, 2.0, 3.0, 4.0]), [2.0, 2.0, 3.0, 4.0]));
    }

    #[test]
    fn the_model_drops_bounces_of_removed_items_and_animates_while_bouncing() {
        let mut s = DockState::default();
        s.set_model(items(3));
        s.bounces = vec![(0, 1.0), (2, 1.0)];
        s.set_model(items(2));
        assert_eq!(s.bounces, vec![(0, 1.0)]);
        assert!(s.animating(5.0), "a bounce running keeps the loop drawing");
        assert!(!s.animating(12.0), "after the timeout nothing animates");
        s.bounces.clear();
        assert!(!s.animating(5.0), "at rest nothing animates");
    }

    #[test]
    fn the_dock_spec_reserves_space_unless_autohidden() {
        let lay = layout(&input(3, None, 0.0));
        let spec = dock_spec(&lay, DockPosition::Bottom, false);
        assert_eq!(spec.exclusive_zone, 78);
        assert_eq!(spec.size, (0, 126));
        assert_eq!(spec.anchor, Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT);
        assert_eq!(dock_spec(&lay, DockPosition::Bottom, true).exclusive_zone, 0);
        assert!(!spec.keyboard);
    }
}
