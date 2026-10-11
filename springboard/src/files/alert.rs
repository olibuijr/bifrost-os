//! The bifrost alert used for Delete confirmation, Rename and New Folder (with
//! a text field) and error messages: a 270 pt card centred on the screen,
//! with its buttons in a row along the bottom.

use crate::gfx::{pack, rgba, Image, Surface};
use crate::shell::keyboard::Out;
use crate::shell::textfield::TextField;
use crate::shell::ui::{Ui, Weight, SECONDARY, WHITE};
use std::path::PathBuf;

pub const WIDTH_PT: f32 = 270.0;
pub const RADIUS_PT: f32 = 14.0;
pub const BUTTON_PT: f32 = 44.0;
const PAD_PT: f32 = 20.0;
const FIELD_PT: f32 = 30.0;
/// The card's fill.
pub const CARD: u32 = pack(44, 44, 46, 255);
const HAIRLINE: u32 = pack(84, 84, 88, 255);
const FIELD_BG: u32 = pack(28, 28, 30, 255);
pub const RED: u32 = pack(255, 69, 58, 255);
pub const BLUE: u32 = pack(10, 132, 255, 255);

/// Why the alert is up: what its confirming button does.
#[derive(Clone, Debug, PartialEq)]
pub enum Purpose {
    Delete(PathBuf),
    /// every item of a multi-selection
    DeleteMany(Vec<PathBuf>),
    Rename(PathBuf),
    /// create a folder inside this directory
    NewFolder(PathBuf),
    /// a message with only an OK button
    Error,
}

pub struct Alert {
    pub purpose: Purpose,
    title: Image,
    message: Option<Image>,
    pub field: Option<TextField>,
    /// (label, destructive) left to right; the last one confirms
    buttons: Vec<(Image, bool)>,
}

impl Alert {
    /// "Delete “name”?" with Cancel and a red Delete.
    pub fn delete(ui: &Ui, path: PathBuf, name: &str, is_dir: bool) -> Self {
        let title = format!("Delete “{}”?", ui.fit(name, 17.0, ui.pt(WIDTH_PT - 2.0 * PAD_PT - 70.0)));
        let msg = if is_dir { "This folder and everything in it will be deleted." } else { "This file will be deleted." };
        Alert { purpose: Purpose::Delete(path), title: ui.text(&title, 17.0, Weight::Semibold, WHITE), message: Some(ui.text(msg, 13.0, Weight::Regular, WHITE)), field: None, buttons: vec![(ui.text("Cancel", 17.0, Weight::Regular, BLUE), false), (ui.text("Delete", 17.0, Weight::Semibold, RED), true)] }
    }

    /// "Delete n items?" for a multi-selection.
    pub fn delete_many(ui: &Ui, paths: Vec<PathBuf>) -> Self {
        let n = paths.len();
        let title = format!("Delete {n} item{}?", if n == 1 { "" } else { "s" });
        Alert { purpose: Purpose::DeleteMany(paths), title: ui.text(&title, 17.0, Weight::Semibold, WHITE), message: Some(ui.text("They will be kept in Recently Deleted for 30 days.", 13.0, Weight::Regular, WHITE)), field: None, buttons: vec![(ui.text("Cancel", 17.0, Weight::Regular, BLUE), false), (ui.text("Delete", 17.0, Weight::Semibold, RED), true)] }
    }

    /// "Rename" with the current name in the field, Cancel and Save.
    pub fn rename(ui: &Ui, path: PathBuf, name: &str) -> Self {
        let mut field = TextField::new(ui, "Name", 15.0, false, 128);
        field.set_text(ui, name);
        Alert { purpose: Purpose::Rename(path), title: ui.text("Rename", 17.0, Weight::Semibold, WHITE), message: None, field: Some(field), buttons: vec![(ui.text("Cancel", 17.0, Weight::Regular, BLUE), false), (ui.text("Save", 17.0, Weight::Semibold, BLUE), false)] }
    }

    /// "New Folder" with an empty name field, Cancel and Create.
    pub fn new_folder(ui: &Ui, dir: PathBuf) -> Self {
        Alert { purpose: Purpose::NewFolder(dir), title: ui.text("New Folder", 17.0, Weight::Semibold, WHITE), message: Some(ui.text("Enter a name for this folder.", 13.0, Weight::Regular, WHITE)), field: Some(TextField::new(ui, "Name", 15.0, false, 128)), buttons: vec![(ui.text("Cancel", 17.0, Weight::Regular, BLUE), false), (ui.text("Create", 17.0, Weight::Semibold, BLUE), false)] }
    }

    /// A message with an OK button.
    pub fn error(ui: &Ui, title: &str, message: &str) -> Self {
        let max = ui.pt(WIDTH_PT - 2.0 * PAD_PT);
        Alert { purpose: Purpose::Error, title: ui.text(&ui.fit(title, 17.0, max), 17.0, Weight::Semibold, WHITE), message: Some(ui.text(&ui.fit(message, 13.0, max), 13.0, Weight::Regular, WHITE)), field: None, buttons: vec![(ui.text("OK", 17.0, Weight::Semibold, BLUE), false)] }
    }

    /// Index of the confirming button (the last one).
    pub fn confirm_index(&self) -> usize {
        self.buttons.len() - 1
    }

    /// Whether the confirming button is enabled (a field must hold a valid name).
    pub fn can_confirm(&self) -> bool {
        self.field.as_ref().is_none_or(|f| super::fs::valid_name(f.text()))
    }

    /// The typed name, trimmed.
    pub fn name(&self) -> String {
        self.field.as_ref().map(|f| f.text().trim().to_string()).unwrap_or_default()
    }

    /// Feeds a key to the field; true when Return confirmed.
    pub fn edit(&mut self, ui: &Ui, out: &Out, t: f64) -> bool {
        match &mut self.field {
            Some(f) => f.edit(ui, out, t) && self.can_confirm(),
            None => false,
        }
    }

    /// Height of the card in px.
    pub fn height(&self, ui: &Ui) -> f32 {
        let mut h = ui.pt(PAD_PT) + self.title.h as f32 + ui.pt(4.0);
        if let Some(m) = &self.message {
            h += m.h as f32 + ui.pt(4.0);
        }
        if self.field.is_some() {
            h += ui.pt(12.0 + FIELD_PT);
        }
        h + ui.pt(PAD_PT - 4.0 + BUTTON_PT)
    }

    /// The card (x, y, w, h) in px, centred in the area above `bottom`.
    pub fn rect(&self, ui: &Ui, w: i32, bottom: f32) -> (f32, f32, f32, f32) {
        let (cw, ch) = (ui.pt(WIDTH_PT), self.height(ui));
        let top = ui.pt(crate::scene::STATUS_PT);
        ((w as f32 - cw) / 2.0, (top + (bottom - top - ch) / 2.0).round(), cw, ch)
    }

    /// The field's text area in px, when there is one.
    pub fn field_rect(&self, ui: &Ui, w: i32, bottom: f32) -> Option<(f32, f32, f32, f32)> {
        self.field.as_ref()?;
        let (x, y, cw, ch) = self.rect(ui, w, bottom);
        let fy = y + ch - ui.pt(BUTTON_PT + PAD_PT - 4.0 + FIELD_PT);
        Some((x + ui.pt(16.0), fy, cw - ui.pt(32.0), ui.pt(FIELD_PT)))
    }

    /// The rect of button `i` in px.
    pub fn button_rect(&self, ui: &Ui, w: i32, bottom: f32, i: usize) -> (f32, f32, f32, f32) {
        let (x, y, cw, ch) = self.rect(ui, w, bottom);
        let bw = cw / self.buttons.len() as f32;
        (x + i as f32 * bw, y + ch - ui.pt(BUTTON_PT), bw, ui.pt(BUTTON_PT))
    }

    /// What a touch hits: a button index, the field, or nothing.
    pub fn hit(&self, ui: &Ui, w: i32, bottom: f32, x: f32, y: f32) -> Option<Hit> {
        for i in 0..self.buttons.len() {
            let (bx, by, bw, bh) = self.button_rect(ui, w, bottom, i);
            if x >= bx && x < bx + bw && y >= by && y < by + bh {
                return Some(Hit::Button(i));
            }
        }
        if let Some((fx, fy, fw, fh)) = self.field_rect(ui, w, bottom) {
            if x >= fx - ui.pt(8.0) && x < fx + fw + ui.pt(8.0) && y >= fy - ui.pt(8.0) && y < fy + fh + ui.pt(8.0) {
                return Some(Hit::Field);
            }
        }
        None
    }

    pub fn draw(&self, s: &mut Surface, ui: &Ui, w: i32, bottom: f32, pressed: Option<usize>, t: f64) {
        let (x, y, cw, ch) = self.rect(ui, w, bottom);
        let r = ui.pt(RADIUS_PT);
        s.fill_rrect(x, y, cw, ch, r, CARD);
        let cx = x + cw / 2.0;
        let mut ty = y + ui.pt(PAD_PT);
        s.blit(&self.title, (cx - self.title.w as f32 / 2.0) as i32, ty as i32, 255, 255);
        ty += self.title.h as f32 + ui.pt(4.0);
        if let Some(m) = &self.message {
            s.blit(m, (cx - m.w as f32 / 2.0) as i32, ty as i32, 255, 255);
        }
        if let (Some(f), Some((fx, fy, fw, fh))) = (&self.field, self.field_rect(ui, w, bottom)) {
            s.fill_rrect(fx, fy, fw, fh, ui.pt(5.0), FIELD_BG);
            s.stroke_rrect(fx, fy, fw, fh, ui.pt(5.0), 1.0, HAIRLINE);
            f.draw(s, ui, (fx + ui.pt(8.0), fy, fw - ui.pt(16.0), fh), t);
        }
        let by = y + ch - ui.pt(BUTTON_PT);
        s.fill_rect(x as usize, by as usize, cw as usize, 1, HAIRLINE);
        let enabled = self.can_confirm();
        for (i, (label, _)) in self.buttons.iter().enumerate() {
            let (bx, by, bw, bh) = self.button_rect(ui, w, bottom, i);
            if i > 0 {
                s.fill_rect(bx as usize, by as usize, 1, bh as usize, HAIRLINE);
            }
            if pressed == Some(i) {
                s.fill_rect(bx as usize + 1, by as usize + 1, bw as usize - 1, bh as usize - 1, rgba(255, 255, 255, 24));
            }
            crate::shell::ui::button_plate(s, ui, (bx, by, bw, bh), label.w as f32);
            let alpha = if i == self.confirm_index() && !enabled { 90 } else { 255 };
            s.blit(label, (bx + (bw - label.w as f32) / 2.0) as i32, (by + (bh - label.h as f32) / 2.0) as i32, 255, alpha);
        }
        let _ = SECONDARY;
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
    Button(usize),
    Field,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shell::tests::ui;

    #[test]
    fn buttons_and_field_hit_and_confirm_rules() {
        let ui = ui();
        let bottom = 1800.0;
        let a = Alert::delete(&ui, "/x/a".into(), "a", false);
        assert_eq!(a.confirm_index(), 1);
        assert!(a.can_confirm());
        let (bx, by, bw, bh) = a.button_rect(&ui, 1200, bottom, 1);
        assert_eq!(a.hit(&ui, 1200, bottom, bx + bw / 2.0, by + bh / 2.0), Some(Hit::Button(1)));
        assert_eq!(a.hit(&ui, 1200, bottom, bx - 5.0, by + bh / 2.0), Some(Hit::Button(0)));
        assert_eq!(a.hit(&ui, 1200, bottom, 5.0, 5.0), None);
        assert!(a.field_rect(&ui, 1200, bottom).is_none());
        let (x, y, w, h) = a.rect(&ui, 1200, bottom);
        assert_eq!(w, ui.pt(WIDTH_PT));
        assert!((x + w / 2.0 - 600.0).abs() < 1.0 && y > ui.pt(20.0) && y + h < bottom);

        let mut r = Alert::rename(&ui, "/x/a".into(), "a");
        assert_eq!(r.name(), "a");
        let (fx, fy, fw, fh) = r.field_rect(&ui, 1200, bottom).unwrap();
        assert_eq!(r.hit(&ui, 1200, bottom, fx + fw / 2.0, fy + fh / 2.0), Some(Hit::Field));
        r.edit(&ui, &Out::Backspace, 0.0);
        assert!(!r.can_confirm(), "an empty name cannot be saved");
        assert!(!r.edit(&ui, &Out::Enter, 1.0), "return with an empty name does nothing");
        r.edit(&ui, &Out::Text("new/name".into()), 2.0);
        assert!(!r.can_confirm(), "slashes are not allowed");
        let mut n = Alert::new_folder(&ui, "/x".into());
        assert!(matches!(n.purpose, Purpose::NewFolder(_)));
        n.edit(&ui, &Out::Text(" Docs ".into()), 0.0);
        assert!(n.edit(&ui, &Out::Enter, 1.0), "return creates");
        assert_eq!(n.name(), "Docs");
        let e = Alert::error(&ui, "Couldn’t rename", "Permission denied");
        assert_eq!(e.confirm_index(), 0);
        let mut buf = vec![0u32; 1200 * 1920];
        let mut s = Surface { w: 1200, h: 1920, stride: 1200, px: &mut buf, y0: 0 };
        for al in [&e, &n, &r, &a] {
            al.draw(&mut s, &ui, 1200, bottom, Some(0), 0.0);
        }
        assert!(buf.contains(&CARD) && buf.contains(&RED), "the Delete alert drawn last shows its red button");
    }
}
