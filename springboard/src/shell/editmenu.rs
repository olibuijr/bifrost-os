//! The bifrost editing menu: a dark rounded bubble of actions ("Cut  Copy
//! Paste") with an arrow pointing at the selection or caret it belongs to.
//! It floats above its anchor and flips below when there is no room above.
//! Geometry follows the bifrost (bifrost 15) in points.

use super::edit::Action;
use super::ui::{Ui, Weight, WHITE};
use crate::gfx::{pack, rgba, Image, Surface};

pub const BG: u32 = pack(51, 51, 51, 255);
const HEIGHT_PT: f32 = 36.0;
const RADIUS_PT: f32 = 6.0;
const PAD_PT: f32 = 15.5;
const ARROW_W_PT: f32 = 16.0;
const ARROW_H_PT: f32 = 8.0;
const TEXT_PT: f32 = 14.0;
/// Gap kept from the screen's sides.
const MARGIN_PT: f32 = 8.0;

struct Item {
    action: Action,
    label: Image,
    /// left edge and width, relative to the bubble
    x: f32,
    w: f32,
}

pub struct EditMenu {
    items: Vec<Item>,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    /// where the arrow points, in screen px
    arrow_x: f32,
    /// the bubble sits below its anchor (arrow on top)
    below: bool,
    pressed: Option<usize>,
}

impl EditMenu {
    /// A menu of `actions` pointing at the px rect `anchor` (a selection or
    /// caret, including any handle it carries). It sits above the anchor,
    /// or below it when its top would cross `top`; horizontally it stays
    /// within `screen_w`, the arrow at the anchor's centre.
    pub fn new(ui: &Ui, actions: &[Action], anchor: (f32, f32, f32, f32), screen_w: f32, top: f32) -> Self {
        let (ax, ay, aw, ah) = anchor;
        let pad = ui.pt(PAD_PT);
        let mut x = 0.0;
        let items: Vec<Item> = actions
            .iter()
            .map(|&action| {
                let label = ui.text(action.label(), TEXT_PT, Weight::Regular, WHITE);
                let w = (label.w as f32 + 2.0 * pad).round();
                let item = Item { action, label, x, w };
                x += w;
                item
            })
            .collect();
        let (w, h) = (x, ui.pt(HEIGHT_PT).round());
        let arrow_h = ui.pt(ARROW_H_PT);
        let below = ay - arrow_h - h < top;
        let y = if below { ay + ah + arrow_h } else { ay - arrow_h - h }.round();
        let centre = ax + aw / 2.0;
        let margin = ui.pt(MARGIN_PT);
        let bx = (centre - w / 2.0).clamp(margin, (screen_w - margin - w).max(margin)).round();
        let r = ui.pt(RADIUS_PT);
        let arrow_x = centre.clamp(bx + r + ui.pt(ARROW_W_PT) / 2.0, bx + w - r - ui.pt(ARROW_W_PT) / 2.0);
        EditMenu { items, x: bx, y, w, h, arrow_x, below, pressed: None }
    }

    /// The bubble's px rect (without the arrow).
    pub fn rect(&self) -> (f32, f32, f32, f32) {
        (self.x, self.y, self.w, self.h)
    }

    pub fn below(&self) -> bool {
        self.below
    }

    fn index_at(&self, x: f32, y: f32) -> Option<usize> {
        if y < self.y || y >= self.y + self.h {
            return None;
        }
        self.items.iter().position(|i| x >= self.x + i.x && x < self.x + i.x + i.w)
    }

    /// The action under (x, y).
    pub fn hit(&self, x: f32, y: f32) -> Option<Action> {
        self.index_at(x, y).map(|i| self.items[i].action)
    }

    /// Whether (x, y) touches the bubble or its arrow.
    pub fn contains(&self, ui: &Ui, x: f32, y: f32) -> bool {
        let arrow = ui.pt(ARROW_H_PT);
        let (top, bottom) = if self.below { (self.y - arrow, self.y + self.h) } else { (self.y, self.y + self.h + arrow) };
        x >= self.x && x < self.x + self.w && y >= top && y < bottom
    }

    /// A finger landed on the menu: highlights the item under it.
    pub fn press(&mut self, x: f32, y: f32) {
        self.pressed = self.index_at(x, y);
    }

    /// The finger lifted: the pressed item's action if it is still under it.
    pub fn release(&mut self, x: f32, y: f32) -> Option<Action> {
        let pressed = self.pressed.take()?;
        (self.index_at(x, y) == Some(pressed)).then(|| self.items[pressed].action)
    }

    pub fn draw(&self, s: &mut Surface, ui: &Ui) {
        let r = ui.pt(RADIUS_PT);
        s.fill_rrect(self.x, self.y, self.w, self.h, r, BG);
        let (aw, ah) = (ui.pt(ARROW_W_PT), ui.pt(ARROW_H_PT));
        let rows = ah.ceil() as i32;
        for i in 0..rows {
            // a 45-degree arrow: each row away from the bubble narrows by two px
            let d = (i as f32 + 0.5) / ah;
            let half = (aw / 2.0 * (1.0 - d)).max(0.0);
            let y = if self.below { self.y - 1.0 - i as f32 } else { self.y + self.h + i as f32 };
            s.fill_rrect(self.arrow_x - half, y, 2.0 * half, 1.0, 0.0, BG);
        }
        let sep = rgba(255, 255, 255, 85);
        for (i, item) in self.items.iter().enumerate() {
            let x = self.x + item.x;
            if self.pressed == Some(i) {
                s.fill_rrect(x, self.y, item.w, self.h, if i == 0 || i + 1 == self.items.len() { r } else { 0.0 }, rgba(255, 255, 255, 30));
            }
            if i > 0 {
                s.fill_rrect(x.round(), self.y, 1.0, self.h, 0.0, sep);
            }
            let lx = x + (item.w - item.label.w as f32) / 2.0;
            let ly = self.y + (self.h - item.label.h as f32) / 2.0;
            s.blit(&item.label, lx.round() as i32, ly.round() as i32, 255, 255);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ui() -> Ui {
        crate::shell::tests::ui()
    }

    const ALL: [Action; 3] = [Action::Cut, Action::Copy, Action::Paste];

    #[test]
    fn sits_above_its_anchor_centred_with_the_arrow_on_it() {
        let ui = ui();
        let m = EditMenu::new(&ui, &ALL, (500.0, 900.0, 200.0, 40.0), 1200.0, 0.0);
        let (x, y, w, h) = m.rect();
        assert_eq!(h, ui.pt(HEIGHT_PT).round());
        assert_eq!(y + h, (900.0 - ui.pt(ARROW_H_PT)).round(), "bubble bottom leaves room for the arrow");
        assert!((x + w / 2.0 - 600.0).abs() <= 1.0, "centred on the anchor");
        assert_eq!(m.arrow_x, 600.0);
        assert!(!m.below());
        let widths: Vec<f32> = m.items.iter().map(|i| i.w).collect();
        assert!(widths[0] < widths[2] && widths[1] < widths[2], "Paste is the widest word: {widths:?}");
        assert_eq!(w, widths.iter().sum::<f32>());
    }

    #[test]
    fn flips_below_near_the_top_and_stays_on_screen() {
        let ui = ui();
        let top = ui.pt(24.0);
        let m = EditMenu::new(&ui, &ALL, (20.0, 60.0, 30.0, 40.0), 1200.0, top);
        assert!(m.below());
        let (x, y, w, _) = m.rect();
        assert_eq!(y, (100.0 + ui.pt(ARROW_H_PT)).round(), "bubble top below the anchor and arrow");
        assert_eq!(x, ui.pt(MARGIN_PT).round(), "clamped to the left margin");
        assert!(m.arrow_x > x && m.arrow_x < x + w, "arrow stays inside the bubble");
        let right = EditMenu::new(&ui, &ALL, (1180.0, 900.0, 10.0, 40.0), 1200.0, 0.0);
        let (rx, _, rw, _) = right.rect();
        assert_eq!(rx + rw, (1200.0 - ui.pt(MARGIN_PT)).round());
        assert!(right.arrow_x < rx + rw);
    }

    #[test]
    fn hit_press_and_release_pick_items() {
        let ui = ui();
        let mut m = EditMenu::new(&ui, &ALL, (500.0, 900.0, 200.0, 40.0), 1200.0, 0.0);
        let (x, y, w, h) = m.rect();
        let mid = y + h / 2.0;
        assert_eq!(m.hit(x + 5.0, mid), Some(Action::Cut));
        assert_eq!(m.hit(x + w - 5.0, mid), Some(Action::Paste));
        assert_eq!(m.hit(x + w / 2.0, mid), Some(Action::Copy));
        assert_eq!(m.hit(x - 1.0, mid), None);
        assert_eq!(m.hit(x + 5.0, y + h + 1.0), None);
        assert!(m.contains(&ui, x + 5.0, y + h + 2.0), "the arrow zone counts");
        assert!(!m.contains(&ui, x + 5.0, y - 1.0));
        m.press(x + 5.0, mid);
        assert_eq!(m.release(x + w - 5.0, mid), None, "sliding off the item cancels");
        m.press(x + 5.0, mid);
        assert_eq!(m.release(x + 6.0, mid), Some(Action::Cut));
        assert_eq!(m.release(x + 6.0, mid), None, "released once");
    }

    #[test]
    fn draws_the_bubble_and_arrow() {
        let ui = ui();
        let mut m = EditMenu::new(&ui, &[Action::Paste], (500.0, 300.0, 10.0, 40.0), 1200.0, 0.0);
        let mut buf = vec![0u32; 1200 * 400];
        m.draw(&mut Surface { w: 1200, h: 400, stride: 1200, px: &mut buf, y0: 0 }, &ui);
        let (x, y, w, h) = m.rect();
        let inside = (y as usize + 5) * 1200 + x as usize + 5;
        assert_eq!(buf[inside], BG);
        assert_eq!(buf[(y as usize + 5) * 1200 + x as usize + w as usize - 5], BG);
        assert_ne!(buf[(y as usize) * 1200 + x as usize], BG, "rounded corner is clear");
        let tip_row = (y + h + ui.pt(ARROW_H_PT) - 1.0) as usize;
        assert_eq!(buf[(y as usize + h as usize + 1) * 1200 + 505], BG, "arrow base under the bubble");
        assert_ne!(buf[tip_row * 1200 + 505 - 6], BG, "the arrow narrows towards its tip");
        assert!(buf.contains(&WHITE), "label drawn");
        m.press(505.0, y + 5.0);
        m.draw(&mut Surface { w: 1200, h: 400, stride: 1200, px: &mut buf, y0: 0 }, &ui);
        assert!(buf[inside] & 0xff > BG & 0xff, "a pressed item lightens");
        let below = EditMenu::new(&ui, &[Action::Paste], (500.0, 30.0, 10.0, 40.0), 1200.0, 60.0);
        below.draw(&mut Surface { w: 1200, h: 400, stride: 1200, px: &mut buf, y0: 0 }, &ui);
        let (_, by, ..) = below.rect();
        assert_eq!(buf[(by as usize - 2) * 1200 + 505], BG, "arrow above a flipped bubble");
    }
}
