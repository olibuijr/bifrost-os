//! Markup: the editor a tap on the screenshot thumbnail opens. Pen, marker,
//! pencil and eraser draw on the capture; a colour row, Undo, Cancel and Done
//! (which saves the marked-up PNG over the capture) frame it as in bifrost.

use super::switcher::draw_scaled;
use super::ui::{Ui, Weight, BLACK, CARD_RADIUS_PT, CELL, SECONDARY, SEPARATOR, SYSTEM_GREEN, SYSTEM_ORANGE, SYSTEM_RED, SYSTEM_YELLOW, TINT, WHITE};
use crate::gfx::{over, pack, scale, Image, Surface};
use crate::layout::Rect;
use std::path::PathBuf;

const BAR_PT: f32 = 56.0;
const TOP_PT: f32 = 24.0;
const TRAY_PT: f32 = 112.0;
const SWATCH_PT: f32 = 30.0;

/// The colours the picker offers, in order.
pub const COLORS: [u32; 7] = [BLACK, WHITE, TINT, SYSTEM_GREEN, SYSTEM_YELLOW, SYSTEM_ORANGE, SYSTEM_RED];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tool {
    Pen,
    Marker,
    Pencil,
    Eraser,
}

const TOOLS: [(Tool, &str); 4] = [(Tool::Pen, "Pen"), (Tool::Marker, "Marker"), (Tool::Pencil, "Pencil"), (Tool::Eraser, "Eraser")];

impl Tool {
    /// Stroke width in points and opacity (0-255).
    fn brush(self) -> (f32, u32) {
        match self {
            Tool::Pen => (3.0, 255),
            Tool::Marker => (14.0, 110),
            Tool::Pencil => (2.0, 190),
            Tool::Eraser => (16.0, 255),
        }
    }
}

struct Stroke {
    tool: Tool,
    color: u32,
    /// points in image pixels
    pts: Vec<(f32, f32)>,
}

/// What a touch asks the shell to do.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Outcome {
    Cancel,
    Done,
}

pub struct Markup {
    pub path: PathBuf,
    base: Image,
    canvas: Image,
    strokes: Vec<Stroke>,
    /// strokes taken back by undo, newest last, so redo can put them back
    undone: Vec<Stroke>,
    tool: Tool,
    color: usize,
    drawing: bool,
    /// the stroke's coverage so far, so translucent tools do not build up
    mask: Vec<bool>,
    pressed: Option<Hit>,
    w: i32,
    h: i32,
    s: f32,
}

/// Left edges of the Undo, Redo and Cancel buttons, one after another from the margin. Undo starts a pad to the
/// left of the margin so its text lines up with the margin; each button's text sits `pad` inside its edges.
fn bar_starts(m: f32, pad: f32, undo_w: f32, redo_w: f32) -> [f32; 3] {
    let undo = m - pad;
    let redo = undo + undo_w + 2.0 * pad;
    let cancel = redo + redo_w + 2.0 * pad;
    [undo, redo, cancel]
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Hit {
    Undo,
    Redo,
    Cancel,
    Done,
    Tool(Tool),
    Color(usize),
}

impl Markup {
    pub fn new(frame: Image, path: PathBuf, s: f32) -> Self {
        let (w, h) = (frame.w as i32, frame.h as i32);
        Markup { path, canvas: frame.clone(), base: frame, strokes: vec![], undone: vec![], tool: Tool::Pen, color: 2, drawing: false, mask: vec![], pressed: None, w, h, s }
    }

    pub fn tool(&self) -> Tool {
        self.tool
    }

    pub fn can_undo(&self) -> bool {
        !self.strokes.is_empty()
    }

    /// The marked-up image.
    pub fn image(&self) -> &Image {
        &self.canvas
    }

    fn bar(&self) -> Rect {
        let top = (TOP_PT * self.s) as i32;
        Rect { x: 0, y: top, w: self.w, h: (BAR_PT * self.s) as i32 }
    }

    fn tray(&self) -> Rect {
        let th = (TRAY_PT * self.s) as i32;
        Rect { x: 0, y: self.h - th, w: self.w, h: th }
    }

    /// Where the picture sits: fitted between the bar and the tray.
    fn pic(&self) -> Rect {
        let pad = (12.0 * self.s) as i32;
        let (top, bottom) = (self.bar().y + self.bar().h + pad, self.tray().y - pad);
        let (aw, ah) = ((self.w - 2 * pad).max(1), (bottom - top).max(1));
        let k = (aw as f32 / self.base.w as f32).min(ah as f32 / self.base.h as f32);
        let (pw, ph) = ((self.base.w as f32 * k) as i32, (self.base.h as f32 * k) as i32);
        Rect { x: (self.w - pw) / 2, y: top + (ah - ph) / 2, w: pw, h: ph }
    }

    fn to_image(&self, x: f32, y: f32) -> (f32, f32) {
        let r = self.pic();
        ((x - r.x as f32) * self.base.w as f32 / r.w as f32, (y - r.y as f32) * self.base.h as f32 / r.h as f32)
    }

    fn button(&self, at: f32, text_w: f32) -> Rect {
        let h = (44.0 * self.s) as i32;
        Rect { x: at as i32, y: self.bar().y + (self.bar().h - h) / 2, w: text_w as i32, h }
    }

    fn rects(&self, ui: &Ui) -> Vec<(Hit, Rect)> {
        let m = 16.0 * self.s;
        let mut v = vec![];
        let (uw, rw, cw, dw) = (ui.measure("Undo", 17.0, Weight::Regular), ui.measure("Redo", 17.0, Weight::Regular), ui.measure("Cancel", 17.0, Weight::Regular), ui.measure("Done", 17.0, Weight::Semibold));
        let pad = 12.0 * self.s;
        let xs = bar_starts(m, pad, uw, rw);
        v.push((Hit::Undo, self.button(xs[0], uw + 2.0 * pad)));
        v.push((Hit::Redo, self.button(xs[1], rw + 2.0 * pad)));
        v.push((Hit::Cancel, self.button(xs[2], cw + 2.0 * pad)));
        v.push((Hit::Done, self.button(self.w as f32 - m - dw - pad, dw + 2.0 * pad)));
        let tray = self.tray();
        let n = TOOLS.len() as i32;
        let gap = (8.0 * self.s) as i32;
        let tw = ((self.w - 2 * m as i32 - gap * (n - 1)) / n).min((120.0 * self.s) as i32);
        let total = tw * n + gap * (n - 1);
        let th = (44.0 * self.s) as i32;
        for (i, (t, _)) in TOOLS.iter().enumerate() {
            v.push((Hit::Tool(*t), Rect { x: (self.w - total) / 2 + i as i32 * (tw + gap), y: tray.y + (8.0 * self.s) as i32, w: tw, h: th }));
        }
        let sw = (SWATCH_PT * self.s) as i32;
        let n = COLORS.len() as i32;
        let cell = ((self.w - 2 * m as i32) / n).min((52.0 * self.s) as i32).max(sw);
        let x0 = (self.w - cell * n) / 2;
        let cy = tray.y + (8.0 * self.s) as i32 + th + (8.0 * self.s) as i32;
        for i in 0..COLORS.len() {
            v.push((Hit::Color(i), Rect { x: x0 + i as i32 * cell, y: cy, w: cell, h: (44.0 * self.s) as i32 }));
        }
        v
    }

    fn hit(&self, ui: &Ui, x: f32, y: f32) -> Option<Hit> {
        self.rects(ui).into_iter().find(|(_, r)| r.contains(x, y)).map(|(h, _)| h)
    }

    /// A touch; `Some` when the editor is finished.
    pub fn touch(&mut self, ui: &Ui, ev: super::Event, _t: f64) -> Option<Outcome> {
        use super::Event;
        match ev {
            Event::Down(x, y) => {
                if let Some(h) = self.hit(ui, x, y) {
                    self.pressed = Some(h);
                } else if self.pic().contains(x, y) {
                    self.drawing = true;
                    self.undone.clear();
                    self.mask = vec![false; self.base.w * self.base.h];
                    let p = self.to_image(x, y);
                    let color = self.stroke_color();
                    self.strokes.push(Stroke { tool: self.tool, color, pts: vec![p] });
                    let n = self.strokes.len() - 1;
                    self.stamp_last(n, 0);
                }
            }
            Event::Move(x, y) if self.drawing => {
                let p = self.to_image(x, y);
                let (w, h) = (self.base.w as f32, self.base.h as f32);
                let p = (p.0.clamp(0.0, w - 1.0), p.1.clamp(0.0, h - 1.0));
                if let Some(st) = self.strokes.last_mut() {
                    st.pts.push(p);
                    let n = st.pts.len() - 2;
                    let i = self.strokes.len() - 1;
                    self.stamp_last(i, n);
                }
            }
            Event::Up(x, y) => {
                self.drawing = false;
                if let Some(h) = self.pressed.take().filter(|h| self.hit(ui, x, y) == Some(*h)) {
                    match h {
                        Hit::Undo => self.undo(),
                        Hit::Redo => self.redo(),
                        Hit::Cancel => return Some(Outcome::Cancel),
                        Hit::Done => return Some(Outcome::Done),
                        Hit::Tool(t) => self.tool = t,
                        Hit::Color(i) => {
                            self.color = i;
                            if self.tool == Tool::Eraser {
                                self.tool = Tool::Pen;
                            }
                        }
                    }
                }
            }
            _ => {}
        }
        None
    }

    fn stroke_color(&self) -> u32 {
        COLORS[self.color]
    }

    pub fn can_redo(&self) -> bool {
        !self.undone.is_empty()
    }

    pub fn undo(&mut self) {
        if let Some(st) = self.strokes.pop() {
            self.undone.push(st);
            self.repaint();
        }
    }

    /// Put back the stroke the last undo took away. A new stroke clears the redo history.
    pub fn redo(&mut self) {
        if let Some(st) = self.undone.pop() {
            self.strokes.push(st);
            self.repaint();
        }
    }

    /// Redraw the base picture with every kept stroke, in order.
    fn repaint(&mut self) {
        self.canvas = self.base.clone();
        let strokes = std::mem::take(&mut self.strokes);
        for (i, st) in strokes.iter().enumerate() {
            self.mask = vec![false; self.base.w * self.base.h];
            self.paint(st, 0, st.pts.len().saturating_sub(1).max(0), i);
        }
        self.strokes = strokes;
    }

    fn stamp_last(&mut self, i: usize, from: usize) {
        let st = std::mem::replace(&mut self.strokes[i], Stroke { tool: Tool::Pen, color: 0, pts: vec![] });
        self.paint(&st, from, st.pts.len().saturating_sub(1), i);
        self.strokes[i] = st;
    }

    /// Draws the stroke's segment from point `from` to the last point (or
    /// just the first dot for a one-point stroke) onto the canvas.
    fn paint(&mut self, st: &Stroke, from: usize, to: usize, _i: usize) {
        let (wpt, alpha) = st.tool.brush();
        let r = (wpt * self.s * self.base.w as f32 / self.w as f32 / 2.0).max(1.0);
        let color = scale(st.color, alpha); // premultiplied, as `over` expects
        let pts = &st.pts;
        if pts.is_empty() {
            return;
        }
        let (a, b) = (pts[from.min(pts.len() - 1)], pts[to.min(pts.len() - 1)]);
        let len = ((b.0 - a.0).powi(2) + (b.1 - a.1).powi(2)).sqrt();
        let steps = (len / (r * 0.4).max(0.5)).ceil().max(1.0) as i32;
        for k in 0..=steps {
            let f = k as f32 / steps as f32;
            self.disc(a.0 + (b.0 - a.0) * f, a.1 + (b.1 - a.1) * f, r, st.tool, color);
        }
    }

    fn disc(&mut self, cx: f32, cy: f32, r: f32, tool: Tool, color: u32) {
        let (w, h) = (self.base.w as i32, self.base.h as i32);
        let (x0, x1) = (((cx - r).floor() as i32).max(0), ((cx + r).ceil() as i32).min(w - 1));
        let (y0, y1) = (((cy - r).floor() as i32).max(0), ((cy + r).ceil() as i32).min(h - 1));
        for y in y0..=y1 {
            for x in x0..=x1 {
                let (dx, dy) = (x as f32 + 0.5 - cx, y as f32 + 0.5 - cy);
                if dx * dx + dy * dy > r * r {
                    continue;
                }
                let i = y as usize * self.base.w + x as usize;
                if tool == Tool::Eraser {
                    self.canvas.px[i] = self.base.px[i];
                } else if tool == Tool::Pen {
                    self.canvas.px[i] = over(self.canvas.px[i], color);
                } else if !self.mask[i] {
                    self.mask[i] = true;
                    self.canvas.px[i] = over(self.canvas.px[i], color);
                }
            }
        }
    }

    pub fn draw(&self, s: &mut Surface, ui: &Ui) {
        s.fill_rect(0, 0, self.w as usize, self.h as usize, BLACK);
        let pic = self.pic();
        draw_scaled(s, &self.canvas, pic, 0.0, 255);
        let tray = self.tray();
        s.fill_rrect(0.0, tray.y as f32, self.w as f32, tray.h as f32, 0.0, CELL);
        s.fill_rect(0, tray.y as usize, self.w as usize, 1.max((0.5 * self.s) as usize), SEPARATOR);
        for (h, r) in self.rects(ui) {
            let down = self.pressed == Some(h);
            match h {
                Hit::Undo | Hit::Redo | Hit::Cancel | Hit::Done => {
                    let (label, weight) = match h {
                        Hit::Undo => ("Undo", Weight::Regular),
                        Hit::Redo => ("Redo", Weight::Regular),
                        Hit::Cancel => ("Cancel", Weight::Regular),
                        _ => ("Done", Weight::Semibold),
                    };
                    let dim = (h == Hit::Undo && !self.can_undo()) || (h == Hit::Redo && !self.can_redo());
                    let color = if dim { SECONDARY } else { TINT };
                    let img = ui.text(label, 17.0, weight, color);
                    let a = if down { 120 } else { 255 };
                    s.blit(&img, r.x + (r.w - img.w as i32) / 2, r.y + (r.h - img.h as i32) / 2, 255, a);
                }
                Hit::Tool(t) => {
                    let on = self.tool == t;
                    let fill = if on { TINT } else { pack(58, 58, 60, 255) };
                    s.fill_rrect(r.x as f32, r.y as f32, r.w as f32, r.h as f32, CARD_RADIUS_PT * self.s, fill);
                    let label = TOOLS.iter().find(|(x, _)| *x == t).map_or("", |(_, l)| l);
                    let img = ui.text(label, 15.0, Weight::Medium, WHITE);
                    s.blit(&img, r.x + (r.w - img.w as i32) / 2, r.y + (r.h - img.h as i32) / 2, 255, 255);
                }
                Hit::Color(i) => {
                    let d = SWATCH_PT * self.s;
                    let (cx, cy) = (r.x as f32 + r.w as f32 / 2.0, r.y as f32 + r.h as f32 / 2.0);
                    if self.color == i {
                        let ring = d + 8.0 * self.s;
                        s.fill_rrect(cx - ring / 2.0, cy - ring / 2.0, ring, ring, ring / 2.0, WHITE);
                        s.fill_rrect(cx - ring / 2.0 + 2.0 * self.s, cy - ring / 2.0 + 2.0 * self.s, ring - 4.0 * self.s, ring - 4.0 * self.s, ring / 2.0, CELL);
                    }
                    s.fill_rrect(cx - d / 2.0, cy - d / 2.0, d, d, d / 2.0, COLORS[i]);
                    if COLORS[i] == BLACK {
                        s.stroke_rrect(cx - d / 2.0, cy - d / 2.0, d, d, d / 2.0, self.s, SECONDARY);
                    }
                }
            }
        }
        let title = ui.text("Markup", 17.0, Weight::Semibold, WHITE);
        let bar = self.bar();
        s.blit(&title, (self.w - title.w as i32) / 2, bar.y + (bar.h - title.h as i32) / 2, 255, 255);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strokes_draw_undo_and_the_eraser_restores() {
        let base = Image { w: 100, h: 100, px: vec![pack(0, 0, 0, 255); 100 * 100] };
        let mut m = Markup::new(base, PathBuf::from("/x.png"), 1.0);
        m.strokes.push(Stroke { tool: Tool::Pen, color: WHITE, pts: vec![(10.0, 10.0), (60.0, 10.0)] });
        m.stamp_last(0, 0);
        assert_ne!(m.canvas.px[10 * 100 + 30], pack(0, 0, 0, 255));
        m.undo();
        assert_eq!(m.canvas.px[10 * 100 + 30], pack(0, 0, 0, 255));
        assert!(!m.can_undo());
    }

    #[test]
    fn redo_button_sits_between_undo_and_cancel() {
        // margin 16, pad 12, Undo and Redo 40 wide: Redo starts where Cancel used to start after Undo (16 + 40 + 12)
        let xs = bar_starts(16.0, 12.0, 40.0, 40.0);
        assert_eq!(xs[0], 4.0);
        assert_eq!(xs[1], 68.0);
        assert_eq!(xs[2], 132.0);
        assert!(xs[0] < xs[1] && xs[1] < xs[2]);
    }

    #[test]
    fn redo_restores_the_stroke_undo_took_and_a_new_stroke_clears_redo() {
        let base = Image { w: 100, h: 100, px: vec![pack(0, 0, 0, 255); 100 * 100] };
        let mut m = Markup::new(base, PathBuf::from("/x.png"), 1.0);
        m.strokes.push(Stroke { tool: Tool::Pen, color: WHITE, pts: vec![(10.0, 10.0), (60.0, 10.0)] });
        m.stamp_last(0, 0);
        m.undo();
        assert!(m.can_redo());
        assert_eq!(m.canvas.px[10 * 100 + 30], pack(0, 0, 0, 255));
        m.redo();
        assert!(!m.can_redo());
        assert!(m.can_undo());
        assert_ne!(m.canvas.px[10 * 100 + 30], pack(0, 0, 0, 255));
        m.undo();
        assert!(m.can_redo());
        m.undone.clear(); // what a new stroke does on touch-down
        assert!(!m.can_redo());
        m.redo();
        assert!(!m.can_undo());
    }
}
