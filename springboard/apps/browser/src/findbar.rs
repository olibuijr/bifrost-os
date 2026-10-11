//! Find in page (Cmd+F; Browser's Find banner, `WebView.find(_:configuration:)`). The bar sits under the tab strip
//! and above the content, so `content_top` grows by `FIND_BAR_PT` and `place_web` shrinks the frame.
use bifrost_app::controls::{Rect, TextField};
use bifrost_app::kit::ui::{Weight, CARD_BG, LABEL_2, MAC_SUBHEAD_PT, SEPARATOR_C};
use bifrost_app::kit::{Surface, Ui};
use bifrost_app::webview::FindDirection;
use bifrost_app::{widgets, KeyInput};

use crate::Browser;

/// Find bar height, points.
pub const FIND_BAR_PT: f32 = 36.0;

#[derive(Default)]
pub struct FindBar {
    pub open: bool,
    pub field: Option<TextField>,
}

impl Browser {
    pub fn find_height(&self) -> f32 {
        if self.find.open { FIND_BAR_PT } else { 0.0 }
    }

    pub fn show_find(&mut self) {
        self.find.open = true;
        self.find.field.get_or_insert_with(TextField::new);
    }

    pub fn hide_find(&mut self) {
        self.find.open = false;
        if let (Some(p), Some(v)) = (&mut self.pool, self.tabs.get(self.current).map(|t| t.view)) {
            p.find_clear(v);
        }
    }

    pub fn find_step(&mut self, dir: FindDirection) {
        let q = self.find.field.as_ref().map(|f| f.to_string()).unwrap_or_default();
        if let (Some(p), Some(v)) = (&mut self.pool, self.tabs.get(self.current).map(|t| t.view)) {
            if q.is_empty() {
                p.find_clear(v);
            } else {
                p.find(v, &q, dir);
            }
        }
    }

    /// Typing in the find field; returns true when the key was used. Enter is next, Esc closes.
    pub fn find_key(&mut self, k: &KeyInput) -> bool {
        let Some(f) = self.find.field.as_mut() else { return false };
        match k {
            KeyInput::Text(s) => {
                f.push_str(s);
                self.find_step(FindDirection::New);
            }
            KeyInput::Code(8) => {
                f.pop();
                self.find_step(FindDirection::New);
            }
            KeyInput::Code(13) => self.find_step(FindDirection::Forward),
            KeyInput::Code(1) | KeyInput::Code(27) => self.hide_find(),
            KeyInput::Code(37) => f.move_caret(false),
            KeyInput::Code(39) => f.move_caret(true),
            _ => return false,
        }
        true
    }

    pub fn draw_find(&self, s: &mut Surface, ui: &Ui, wpt: f32) {
        if !self.find.open {
            return;
        }
        let top = self.content_top() - FIND_BAR_PT;
        let (y, h) = (ui.pt(top) as usize, ui.pt(FIND_BAR_PT).ceil() as usize);
        s.fill_rect(0, y, s.w, h, CARD_BG.now());
        s.fill_rect(0, y + h - 1, s.w, 1, SEPARATOR_C.now());
        let width = (wpt - 24.0).clamp(120.0, 320.0);
        let r = Rect { x: 12.0, y: top + 6.0, w: width, h: FIND_BAR_PT - 12.0 };
        if let Some(f) = &self.find.field {
            widgets::search_field(s, ui, f, r, "Find in page", true);
        }
        let count = self.view().and_then(|v| self.pool.as_ref()?.state(v)).and_then(|st| st.find_matches);
        let q_empty = self.find.field.as_ref().is_none_or(|f| f.to_string().is_empty());
        let label = match count {
            Some(0) => "Not found".to_string(),
            Some(n) => format!("{n} match{}", if n == 1 { "" } else { "es" }),
            None if q_empty => String::new(),
            None => "Searching".to_string(),
        };
        if !label.is_empty() {
            let img = ui.text(&label, MAC_SUBHEAD_PT, Weight::Regular, LABEL_2.now());
            s.blit(&img, ui.pt(r.x + r.w + 12.0) as i32, ui.pt(r.y + r.h / 2.0) as i32 - img.h as i32 / 2, 255, 255);
        }
    }
}
