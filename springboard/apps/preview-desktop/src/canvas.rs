//! Layout, hit tests and drawing for the Viewer window. Every size comes from the window size and the
//! `Ui` scale (`ui.pt`), and every colour from the shared tokens in `kit::ui`. The picture and text
//! draw first; the sidebar and the toolbar draw last, so anything that runs under them is covered.

use crate::document::{Body, Document, THUMB_W};
use crate::model::{self, Adjust, Mark, MarkKind, Norm, View, Zoom, HIGHLIGHT, MARK_BLUE};
use bifrost_app::kit::ui::{self, Ui, Weight};
use bifrost_app::kit::{rgba, Image, Surface};

/// The tool a drag or tap on the canvas uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    Pan,
    Highlight,
    Note,
    Rect,
    Rounded,
}

impl Tool {
    /// The mark a drag with this tool makes, or `None` for panning.
    pub fn mark_kind(self) -> Option<MarkKind> {
        match self {
            Tool::Pan => None,
            Tool::Highlight => Some(MarkKind::Highlight),
            Tool::Note => Some(MarkKind::Note),
            Tool::Rect => Some(MarkKind::Rect),
            Tool::Rounded => Some(MarkKind::Rounded),
        }
    }
}

/// Everything a toolbar button or a menu item does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Previous,
    Next,
    ZoomOut,
    ZoomIn,
    ActualSize,
    FitWindow,
    RotateLeft,
    RotateRight,
    UseTool(Tool),
    Brighter,
    Darker,
    Saturate,
    Desaturate,
    ResetColour,
    ExportPng,
}

/// Every action, in toolbar order. A menu item's id is its index here.
pub const ACTIONS: [Action; 19] = [
    Action::Previous,
    Action::Next,
    Action::ZoomOut,
    Action::ZoomIn,
    Action::FitWindow,
    Action::ActualSize,
    Action::RotateLeft,
    Action::RotateRight,
    Action::UseTool(Tool::Pan),
    Action::UseTool(Tool::Highlight),
    Action::UseTool(Tool::Note),
    Action::UseTool(Tool::Rect),
    Action::UseTool(Tool::Rounded),
    Action::Brighter,
    Action::Darker,
    Action::Saturate,
    Action::Desaturate,
    Action::ResetColour,
    Action::ExportPng,
];

impl Action {
    pub fn id(self) -> u32 {
        ACTIONS.iter().position(|a| *a == self).unwrap_or(0) as u32
    }

    pub fn from_id(id: u32) -> Option<Action> {
        ACTIONS.get(id as usize).copied()
    }

    pub fn label(self) -> &'static str {
        match self {
            Action::Previous => "Previous",
            Action::Next => "Next",
            Action::ZoomOut => "Zoom Out",
            Action::ZoomIn => "Zoom In",
            Action::ActualSize => "Actual Size",
            Action::FitWindow => "Fit",
            Action::RotateLeft => "Rotate Left",
            Action::RotateRight => "Rotate Right",
            Action::UseTool(Tool::Pan) => "Pan",
            Action::UseTool(Tool::Highlight) => "Highlight",
            Action::UseTool(Tool::Note) => "Note",
            Action::UseTool(Tool::Rect) => "Rectangle",
            Action::UseTool(Tool::Rounded) => "Rounded",
            Action::Brighter => "Brighter",
            Action::Darker => "Darker",
            Action::Saturate => "Saturate",
            Action::Desaturate => "Desaturate",
            Action::ResetColour => "Reset Colour",
            Action::ExportPng => "Export PNG",
        }
    }

    /// The toolbar buttons, in the order of `ACTIONS`.
    pub fn toolbar() -> [Action; 19] {
        ACTIONS
    }
}

/// Window geometry: the toolbar along the top, the thumbnails down the left, the canvas in between.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Layout {
    pub w: i32,
    pub h: i32,
    pub bar: i32,
    pub side: i32,
}

impl Layout {
    pub fn new(ui: &Ui, w: i32, h: i32) -> Layout {
        let bar = ui.pt(ui::SPACE_XXXL + ui::SPACE_L) as i32;
        let side = (ui.pt(ui::SPACE_XXXL * 5.0) as i32).min(w / 3).max(0);
        Layout { w: w.max(1), h: h.max(1), bar, side }
    }

    /// The canvas rectangle (x, y, w, h).
    pub fn canvas(&self) -> (i32, i32, i32, i32) {
        (self.side, self.bar, (self.w - self.side).max(1), (self.h - self.bar).max(1))
    }
}

/// The toolbar buttons that fit the window: each with its x and width. Buttons past the right edge
/// are left out; every action is still in the menus.
pub fn buttons(ui: &Ui, lay: &Layout) -> Vec<(Action, i32, i32)> {
    let pad = ui.pt(ui::SPACE_S);
    let gap = ui.pt(ui::SPACE_XS);
    let mut x = lay.side as f32 + pad;
    let mut out = Vec::new();
    for action in Action::toolbar() {
        let w = ui.measure(action.label(), ui::MAC_BODY_PT, Weight::Regular) + 2.0 * pad;
        if x + w > lay.w as f32 - pad {
            break;
        }
        out.push((action, x as i32, w as i32));
        x += w + gap;
    }
    out
}

/// The thumbnail rows in content coordinates: (top, height) for each page, with the gap before.
pub fn rows(ui: &Ui, doc: &Document) -> Vec<(i32, i32)> {
    let gap = ui.pt(ui::SPACE_S) as i32;
    let label = ui.pt(ui::MAC_FOOTNOTE_PT * 1.6) as i32;
    let mut y = gap;
    let mut out = Vec::with_capacity(doc.pages.len());
    for page in &doc.pages {
        let box_h = match &page.thumb {
            Some(t) => t.h as i32,
            None => (THUMB_W as f32 * 1.3) as i32,
        };
        let h = box_h + label + gap;
        out.push((y, h));
        y += h;
    }
    out
}

/// The sidebar's total content height.
pub fn rows_height(rows: &[(i32, i32)]) -> i32 {
    rows.last().map_or(0, |(top, h)| top + h)
}

/// The page under a sidebar point, if any.
pub fn sidebar_hit(lay: &Layout, rows: &[(i32, i32)], scroll: f32, x: f32, y: f32) -> Option<usize> {
    if x < 0.0 || x >= lay.side as f32 || y < lay.bar as f32 {
        return None;
    }
    let content = y - lay.bar as f32 + scroll;
    rows.iter().position(|(top, h)| content >= *top as f32 && content < (top + h) as f32)
}

/// The toolbar action under a point, if any.
pub fn button_hit(ui: &Ui, lay: &Layout, x: f32, y: f32) -> Option<Action> {
    if y < 0.0 || y >= lay.bar as f32 {
        return None;
    }
    buttons(ui, lay).into_iter().find(|(_, bx, bw)| x >= *bx as f32 && x < (bx + bw) as f32).map(|(a, _, _)| a)
}

/// Where the picture sits on the canvas: its top-left and its drawn size, after the pan is clamped.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placed {
    pub x: i32,
    pub y: i32,
    pub w: usize,
    pub h: usize,
    pub pan: (f32, f32),
}

/// Places a `rw` by `rh` picture drawn at `scale` on the canvas, centred, panned by `pan` and kept
/// in view horizontally and vertically (a picture smaller than the canvas is not panned).
pub fn place(canvas: (i32, i32, i32, i32), rw: usize, rh: usize, scale: f32, pan: (f32, f32)) -> Placed {
    let (cx, cy, cw, ch) = canvas;
    let w = ((rw as f32 * scale).round() as usize).max(1);
    let h = ((rh as f32 * scale).round() as usize).max(1);
    let lim_x = ((w as f32 - cw as f32) / 2.0).max(0.0);
    let lim_y = ((h as f32 - ch as f32) / 2.0).max(0.0);
    let px = pan.0.clamp(-lim_x, lim_x);
    let py = pan.1.clamp(-lim_y, lim_y);
    let x = cx + (cw - w as i32) / 2 + px.round() as i32;
    let y = cy + (ch - h as i32) / 2 + py.round() as i32;
    Placed { x, y, w, h, pan: (px, py) }
}

/// The source-page coordinates (normalised, unrotated) of a canvas point over a placed picture.
/// Points past the picture are clamped to its edge.
pub fn to_source(placed: &Placed, quarters: u8, x: f32, y: f32) -> (f32, f32) {
    let u = ((x - placed.x as f32) / placed.w as f32).clamp(0.0, 1.0);
    let v = ((y - placed.y as f32) / placed.h as f32).clamp(0.0, 1.0);
    model::unrot_point(u, v, quarters)
}

/// The canvas rectangle of a source-page rectangle over a placed picture.
fn to_screen(placed: &Placed, quarters: u8, r: Norm) -> (f32, f32, f32, f32) {
    let t = model::rot_rect(r, quarters);
    (
        placed.x as f32 + t.x * placed.w as f32,
        placed.y as f32 + t.y * placed.h as f32,
        t.w * placed.w as f32,
        t.h * placed.h as f32,
    )
}

/// The rendered picture of one page, kept while nothing that changes it has changed.
pub struct PictureCache {
    key: (usize, Adjust, u8, usize, usize),
    pub img: Image,
}

/// Looks up or makes the scaled, adjusted and turned picture for the current view.
pub fn picture_for<'a>(cache: &'a mut Option<PictureCache>, page: usize, src: &Image, adjust: &Adjust, quarters: u8, w: usize, h: usize) -> &'a Image {
    let key = (page, *adjust, quarters, w, h);
    if cache.as_ref().map(|c| c.key) != Some(key) {
        let base = crate::document::adjusted(src, adjust);
        let turned = model::rotate(&base, quarters);
        let img = if (turned.w, turned.h) == (w, h) { turned } else { turned.resized(w, h) };
        *cache = Some(PictureCache { key, img });
    }
    &cache.as_ref().expect("the picture was just cached").img
}

/// What the canvas draws this frame.
pub struct Scene<'a> {
    pub doc: Option<&'a Document>,
    pub view: View,
    pub tool: Tool,
    pub adjust: Adjust,
    /// The mark being dragged out, drawn with the saved ones.
    pub draft: Option<Mark>,
    /// The note whose text the keyboard is editing.
    pub editing: Option<usize>,
    /// A message to show in the middle of the canvas (an open error, an export result).
    pub status: &'a str,
}

/// Draws the window. `cache` keeps the scaled picture between frames.
pub fn draw(s: &mut Surface, ui: &Ui, lay: &Layout, scene: &Scene, cache: &mut Option<PictureCache>) -> Option<Placed> {
    let (cx, cy, cw, ch) = lay.canvas();
    fill(s, cx, cy, cw, ch, ui::CONTENT_BG.now());
    let mut placed = None;
    match scene.doc {
        None => {
            let msg = if scene.status.is_empty() { "Open an image or a PDF to view it." } else { scene.status };
            centred_text(s, ui, msg, lay, ui::LABEL_2.now());
        }
        Some(doc) => {
            let page_no = scene.view.page.min(doc.pages.len().saturating_sub(1));
            match &doc.pages[page_no].body {
                Body::Picture(img) => {
                    let q = scene.view.quarters;
                    let (rw, rh) = model::turn_size(img.w, img.h, q);
                    let scale = scene.view.zoom.scale(rw, rh, cw as usize, ch as usize);
                    let target_w = ((rw as f32 * scale).round() as usize).max(1);
                    let target_h = ((rh as f32 * scale).round() as usize).max(1);
                    let p = place((cx, cy, cw, ch), rw, rh, scale, scene.view.pan);
                    let shown = picture_for(cache, page_no, img, &scene.adjust, q, target_w, target_h);
                    s.blit(shown, p.x, p.y, 255, 255);
                    for m in doc.marks.iter().filter(|m| m.page == page_no).chain(scene.draft.iter()) {
                        draw_mark(s, &p, q, m);
                    }
                    placed = Some(p);
                }
                Body::Text(text) => {
                    let factor = match scene.view.zoom {
                        Zoom::Factor(f) => f.clamp(0.5, 3.0),
                        Zoom::Fit => 1.0,
                    };
                    let pt = ui::MAC_BODY_PT * factor;
                    let margin = ui.pt(ui::SPACE_XXL) as i32;
                    let max_px = (cw - 2 * margin).max(1) as f32;
                    let lines: Vec<String> = text.split('\n').flat_map(|l| ui.wrap(l, pt, Weight::Regular, max_px)).collect();
                    let line_h = pt * 1.5;
                    let text_h = line_h * lines.len() as f32;
                    let lim = ((text_h - ch as f32) / 2.0).max(0.0);
                    let oy = scene.view.pan.1.clamp(-lim, lim);
                    let top = cy as f32 + margin as f32 + oy;
                    for (i, line) in lines.iter().enumerate() {
                        let y = top + i as f32 * line_h;
                        if y + line_h < cy as f32 || y > (cy + ch) as f32 {
                            continue;
                        }
                        let img = ui.text(line, pt, Weight::Regular, ui::WHITE);
                        s.blit_colored(&img, cx + margin, y as i32, ui::LABEL.now(), 255);
                    }
                    for m in doc.marks.iter().filter(|m| m.page == page_no) {
                        if m.kind == MarkKind::Note {
                            let y = (top + m.rect.y * text_h).max(cy as f32) as i32;
                            fill(s, cx + 2, y, 3, (line_h as i32).max(4), MARK_BLUE);
                        }
                    }
                }
            }
        }
    }
    // a note's text sits beside its marker on picture pages
    if let (Some(doc), Some(p)) = (scene.doc, placed) {
        let page_no = scene.view.page;
        for (i, m) in doc.marks.iter().enumerate().filter(|(_, m)| m.page == page_no && m.kind == MarkKind::Note) {
            if m.text.is_empty() && scene.editing != Some(i) {
                continue;
            }
            let (x, y, _, _) = to_screen(&p, scene.view.quarters, m.rect);
            let shown = if scene.editing == Some(i) { format!("{}|", m.text) } else { m.text.clone() };
            let img = ui.text(&shown, ui::MAC_CALLOUT_PT, Weight::Regular, ui::WHITE);
            s.blit_colored(&img, x as i32 + ui.pt(ui::SPACE_L) as i32, y as i32, ui::LABEL.now(), 255);
        }
    }
    if scene.doc.is_some() && !scene.status.is_empty() {
        let img = ui.text(scene.status, ui::MAC_FOOTNOTE_PT, Weight::Regular, ui::WHITE);
        let pad = ui.pt(ui::SPACE_S) as i32;
        s.blit_colored(&img, cx + pad, cy + ch - img.h as i32 - pad, ui::LABEL_2.now(), 255);
    }
    draw_sidebar(s, ui, lay, scene);
    draw_toolbar(s, ui, lay, scene);
    placed
}

fn draw_mark(s: &mut Surface, p: &Placed, quarters: u8, m: &Mark) {
    let (x, y, w, h) = to_screen(p, quarters, m.rect);
    let (x, y, w, h) = (x as i32, y as i32, w.max(1.0) as i32, h.max(1.0) as i32);
    match m.kind {
        MarkKind::Highlight => {
            // the highlight colour as a premultiplied tile, blended source-over
            let c = HIGHLIGHT;
            let hl = rgba((c & 0xff) as u8, ((c >> 8) & 0xff) as u8, ((c >> 16) & 0xff) as u8, (c >> 24) as u8);
            let px = vec![hl; (w.max(1) * h.max(1)) as usize];
            let tile = Image { w: w.max(1) as usize, h: h.max(1) as usize, px };
            s.blit(&tile, x, y, 255, 255);
        }
        MarkKind::Rect => s.stroke_rrect(x as f32, y as f32, w as f32, h as f32, 0.0, 2.0, m.color),
        MarkKind::Rounded => {
            let r = (w.min(h) as f32) * 0.2;
            s.stroke_rrect(x as f32, y as f32, w as f32, h as f32, r, 2.0, m.color);
        }
        MarkKind::Note => {
            fill(s, x, y, 14, 14, MARK_BLUE);
        }
    }
}

fn draw_sidebar(s: &mut Surface, ui: &Ui, lay: &Layout, scene: &Scene) {
    if lay.side <= 0 {
        return;
    }
    fill(s, 0, lay.bar, lay.side, lay.h - lay.bar, ui::SIDEBAR_FILL);
    let Some(doc) = scene.doc else { return };
    let rows = rows(ui, doc);
    let pad = ui.pt(ui::SPACE_S) as i32;
    for (i, (page, (top, h))) in doc.pages.iter().zip(rows.iter()).enumerate() {
        let y = lay.bar + top - scene.view.sidebar_scroll as i32;
        if y + h < lay.bar || y > lay.h {
            continue;
        }
        if i == scene.view.page {
            s.fill_rrect(pad as f32 / 2.0, (y + pad / 2) as f32, (lay.side - pad) as f32, (h - pad) as f32, ui.pt(ui::SPACE_S), ui::SIDEBAR_SELECTED);
        }
        match &page.thumb {
            Some(t) => {
                let x = (lay.side - t.w as i32) / 2;
                s.blit(t, x, y + pad, 255, 255);
            }
            None => {
                let bh = (THUMB_W as f32 * 1.3) as i32;
                let x = (lay.side - THUMB_W as i32) / 2;
                fill(s, x, y + pad, THUMB_W as i32, bh, ui::FILL);
            }
        }
        let box_h = match &page.thumb {
            Some(t) => t.h as i32,
            None => (THUMB_W as f32 * 1.3) as i32,
        };
        let text = format!("{}", i + 1);
        let img = ui.text(&text, ui::MAC_FOOTNOTE_PT, Weight::Regular, ui::WHITE);
        let tw = ui.measure(&text, ui::MAC_FOOTNOTE_PT, Weight::Regular) as i32;
        s.blit_colored(&img, (lay.side - tw) / 2, y + pad + box_h + pad / 4, ui::LABEL_2.now(), 255);
    }
}

fn draw_toolbar(s: &mut Surface, ui: &Ui, lay: &Layout, scene: &Scene) {
    fill(s, 0, 0, lay.w, lay.bar, ui::CONTENT_BG.now());
    fill(s, 0, lay.bar - 1, lay.w, 1, ui::SEPARATOR_C.now());
    let y = (lay.bar - ui.pt(ui::MAC_BODY_PT * 1.5) as i32) / 2;
    for (action, x, w) in buttons(ui, lay) {
        let active = matches!(action, Action::UseTool(t) if t == scene.tool);
        let color = if active { ui::TINT } else { ui::LABEL.now() };
        if active {
            s.fill_rrect(x as f32, (y - 4) as f32, w as f32, (lay.bar - 2 * y + 8) as f32, ui.pt(ui::SPACE_S), ui::FILL);
        }
        let img = ui.text(action.label(), ui::MAC_BODY_PT, Weight::Regular, ui::WHITE);
        let tx = x + (w - ui.measure(action.label(), ui::MAC_BODY_PT, Weight::Regular) as i32) / 2;
        s.blit_colored(&img, tx, y, color, 255);
    }
}

fn centred_text(s: &mut Surface, ui: &Ui, text: &str, lay: &Layout, color: u32) {
    let (cx, cy, cw, ch) = lay.canvas();
    let img = ui.text(text, ui::MAC_BODY_PT, Weight::Regular, ui::WHITE);
    let x = cx + (cw - img.w as i32) / 2;
    let y = cy + ch / 2;
    s.blit_colored(&img, x.max(cx), y, color, 255);
}

/// A plain fill clipped to the surface.
fn fill(s: &mut Surface, x: i32, y: i32, w: i32, h: i32, color: u32) {
    let x0 = x.max(0);
    let y0 = y.max(0);
    let x1 = (x + w).min(s.w as i32);
    let y1 = (y + h).min(s.h as i32);
    if x1 > x0 && y1 > y0 {
        s.fill_rect(x0 as usize, y0 as usize, (x1 - x0) as usize, (y1 - y0) as usize, color);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn action_ids_round_trip_and_are_unique() {
        for (i, a) in ACTIONS.iter().enumerate() {
            assert_eq!(a.id() as usize, i);
            assert_eq!(Action::from_id(a.id()), Some(*a));
        }
        assert_eq!(Action::from_id(999), None);
    }

    #[test]
    fn placed_picture_is_centred_and_pan_is_clamped() {
        let p = place((160, 48, 800, 600), 400, 300, 1.0, (0.0, 0.0));
        assert_eq!((p.x, p.y), (160 + 200, 48 + 150));
        // a 2000 wide picture can move at most half its overhang each way
        let p = place((0, 0, 800, 600), 2000, 300, 1.0, (5000.0, 0.0));
        assert_eq!(p.pan.0, 600.0);
        assert_eq!(p.x, 0 + (800 - 2000) / 2 + 600);
        let p = place((0, 0, 800, 600), 100, 100, 1.0, (50.0, 50.0));
        assert_eq!(p.pan, (0.0, 0.0));
    }

    #[test]
    fn canvas_point_maps_back_through_a_turn() {
        let p = place((0, 0, 100, 100), 100, 50, 1.0, (0.0, 0.0));
        // a point at the top-left of the turned canvas is the bottom-left of the source page for one turn
        let (u, v) = to_source(&p, 1, p.x as f32, p.y as f32);
        assert!((u - 0.0).abs() < 1e-5 && (v - 1.0).abs() < 1e-5, "got {u} {v}");
    }

    #[test]
    fn sidebar_hit_uses_scroll_and_ignores_the_toolbar() {
        let lay = Layout { w: 600, h: 400, bar: 40, side: 150 };
        let rows = vec![(8, 100), (116, 100)];
        assert_eq!(sidebar_hit(&lay, &rows, 0.0, 10.0, 60.0), Some(0));
        assert_eq!(sidebar_hit(&lay, &rows, 100.0, 10.0, 60.0), Some(1));
        assert_eq!(sidebar_hit(&lay, &rows, 0.0, 10.0, 20.0), None);
        assert_eq!(sidebar_hit(&lay, &rows, 0.0, 200.0, 60.0), None);
        assert_eq!(rows_height(&rows), 216);
    }

    #[test]
    fn tool_maps_to_mark_kind() {
        assert_eq!(Tool::Pan.mark_kind(), None);
        assert_eq!(Tool::Note.mark_kind(), Some(MarkKind::Note));
        assert_eq!(Tool::Rounded.mark_kind(), Some(MarkKind::Rounded));
    }
}
