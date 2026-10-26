//! A small "new item" sheet: labelled single-line fields (one of them can be secret) under a title, with an Add and a
//! Cancel button, hung from the title bar of a full-size window (`NativeWindow.beginSheet`). Passwords and Keychain
//! Access use it for New Password. The model, layout and hit tests are in points and pure, so they are tested on the
//! host; `draw` paints the same rectangles. Typing goes to the focused field, Tab moves to the next one, Return adds
//! (when every field has text) and Escape cancels.

use crate::kit::ui::{Weight, DESTRUCTIVE, LABEL, MAC_BODY_PT, MAC_FOOTNOTE_PT};
use crate::kit::{Surface, Ui};
use crate::widgets;
use bifrost_ui::sheet::{self, ButtonRole, Hit, Sheet, SheetButton, SHEET_PAD_PT};
use bifrost_ui::{Rect, TextField};

/// The Cancel and Add buttons' ids.
pub const CANCEL: u32 = 1;
pub const ADD: u32 = 2;

/// Sheet width, label column width, row and field heights, and the room the title takes (points).
const WIDTH_PT: f32 = 380.0;
const LABEL_PT: f32 = 92.0;
const ROW_PT: f32 = 32.0;
const FIELD_H_PT: f32 = 24.0;
const TITLE_PT: f32 = 26.0;
const NOTE_PT: f32 = 15.0;
/// The most bytes a field takes; typing past it is cut.
const LIMIT: usize = 256;
/// Panel height that is not content: the padding, the title, the gap above the buttons and the buttons.
const CHROME_PT: f32 = SHEET_PAD_PT * 2.0 + TITLE_PT + 12.0 + sheet::BUTTON_H_PT;

/// What a press or a key did to the sheet.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Nothing changed (the sheet still swallowed the press).
    None,
    /// The sheet changed: draw a new frame.
    Redraw,
    /// Add was chosen and every field has text: the trimmed values, in field order.
    Add(Vec<String>),
    /// Cancel was chosen, or Escape pressed: close the sheet.
    Cancel,
}

struct Field {
    label: String,
    placeholder: String,
    secret: bool,
    text: TextField,
}

/// The open sheet.
pub struct ItemSheet {
    title: String,
    fields: Vec<Field>,
    focus: usize,
    error: Option<String>,
}

impl ItemSheet {
    /// A sheet titled `title` with one `(label, placeholder, secret)` row per field. The first field has the focus.
    pub fn new(title: &str, fields: &[(&str, &str, bool)]) -> Self {
        let fields = fields
            .iter()
            .map(|(label, placeholder, secret)| Field {
                label: label.to_string(),
                placeholder: placeholder.to_string(),
                secret: *secret,
                text: TextField::new(),
            })
            .collect();
        ItemSheet { title: title.to_string(), fields, focus: 0, error: None }
    }

    fn content_h(&self) -> f32 {
        self.fields.len() as f32 * ROW_PT + if self.error.is_some() { 4.0 + NOTE_PT } else { 0.0 }
    }

    /// The sheet that frames the fields: Add is the first (rightmost) button.
    pub fn sheet(&self) -> Sheet {
        let buttons = vec![SheetButton::new(ADD, "Add", ButtonRole::Default), SheetButton::new(CANCEL, "Cancel", ButtonRole::Cancel)];
        Sheet::new(&self.title, (WIDTH_PT, self.content_h() + CHROME_PT), buttons)
    }

    /// The panel and the rectangle of each field, for a sheet hung from the title bar (`titlebar_h` tall) of `parent`.
    pub fn geometry(&self, parent: Rect, titlebar_h: f32) -> (Rect, Vec<Rect>) {
        let panel = self.sheet().layout(parent, titlebar_h, 1.0).panel;
        let x = panel.x + SHEET_PAD_PT + LABEL_PT;
        let w = (panel.w - SHEET_PAD_PT * 2.0 - LABEL_PT).max(0.0);
        let top = panel.y + SHEET_PAD_PT + TITLE_PT;
        let fields = (0..self.fields.len()).map(|i| Rect { x, y: top + i as f32 * ROW_PT + (ROW_PT - FIELD_H_PT) / 2.0, w, h: FIELD_H_PT }).collect();
        (panel, fields)
    }

    /// A press at `(x, y)` (points, window coordinates). The sheet swallows every press: a field takes the focus, a
    /// button answers.
    pub fn press(&mut self, parent: Rect, titlebar_h: f32, x: f32, y: f32) -> Outcome {
        match self.sheet().hit(parent, titlebar_h, 1.0, x, y) {
            Hit::Button(ADD) => self.add(),
            Hit::Button(_) => Outcome::Cancel,
            Hit::Panel => {
                let (_, fields) = self.geometry(parent, titlebar_h);
                match fields.iter().position(|r| r.contains(x, y)) {
                    Some(i) if i != self.focus => {
                        self.focus = i;
                        Outcome::Redraw
                    }
                    _ => Outcome::None,
                }
            }
            _ => Outcome::None,
        }
    }

    /// Typed text goes to the focused field.
    pub fn type_text(&mut self, text: &str) -> Outcome {
        let Some(field) = self.fields.get_mut(self.focus) else { return Outcome::None };
        field.text.push_str(text);
        while field.text.len() > LIMIT {
            field.text.pop();
        }
        self.error = None;
        Outcome::Redraw
    }

    /// A key code: Backspace (8) deletes, Tab (9) moves to the next field, Return (13) adds, Escape (1, 27) cancels.
    pub fn code(&mut self, code: u32) -> Outcome {
        match code {
            8 => {
                if let Some(field) = self.fields.get_mut(self.focus) {
                    field.text.pop();
                }
                self.error = None;
                Outcome::Redraw
            }
            9 => {
                self.focus = (self.focus + 1) % self.fields.len().max(1);
                Outcome::Redraw
            }
            13 => self.add(),
            1 | 27 => Outcome::Cancel,
            _ => Outcome::None,
        }
    }

    /// The trimmed values when every field has text; otherwise the sheet says so and stays open.
    fn add(&mut self) -> Outcome {
        let values: Vec<String> = self.fields.iter().map(|f| f.text.trim().to_string()).collect();
        if values.iter().any(String::is_empty) {
            self.error = Some("Fill in every field.".to_string());
            return Outcome::Redraw;
        }
        Outcome::Add(values)
    }

    /// The text a field shows: the text itself, or one bullet for each character of a secret.
    fn shown(field: &Field) -> TextField {
        let mut shown = TextField::new();
        if field.secret {
            shown.set_text(&"\u{2022}".repeat(field.text.chars().count()));
        } else {
            shown.set_text(&field.text);
        }
        shown.end();
        shown
    }

    /// Paints the sheet over `parent` (the window rectangle, points).
    pub fn draw(&self, s: &mut Surface, ui: &Ui, parent: Rect, titlebar_h: f32) {
        widgets::sheet(s, ui, parent, titlebar_h, &self.sheet(), 1.0, None);
        let (panel, rects) = self.geometry(parent, titlebar_h);
        for (i, (field, rect)) in self.fields.iter().zip(&rects).enumerate() {
            let label = ui.text(&ui.fit(&field.label, MAC_BODY_PT, ui.pt(LABEL_PT - 8.0)), MAC_BODY_PT, Weight::Regular, LABEL.now());
            let top = ui.pt(rect.y + rect.h / 2.0) - label.h as f32 / 2.0;
            s.blit_colored(&label, ui.pt(panel.x + SHEET_PAD_PT) as i32, top as i32, LABEL.now(), 255);
            widgets::text_field(s, ui, &Self::shown(field), *rect, &field.placeholder, i == self.focus);
        }
        if let Some(error) = &self.error {
            let line = ui.text(&ui.fit(error, MAC_FOOTNOTE_PT, ui.pt(panel.w - SHEET_PAD_PT * 2.0)), MAC_FOOTNOTE_PT, Weight::Regular, DESTRUCTIVE);
            let y = rects.last().map_or(panel.y, |r| r.bottom() + (ROW_PT - FIELD_H_PT) / 2.0 + 4.0);
            s.blit_colored(&line, ui.pt(panel.x + SHEET_PAD_PT) as i32, ui.pt(y) as i32, DESTRUCTIVE, 255);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window() -> Rect {
        Rect { x: 0.0, y: 0.0, w: 1000.0, h: 700.0 }
    }

    fn bar() -> f32 {
        crate::layout::UNIFIED_BAR_PT
    }

    fn sample() -> ItemSheet {
        ItemSheet::new("New Password", &[("Website", "example.com", false), ("Account", "name", false), ("Password", "", true)])
    }

    fn type_all(sheet: &mut ItemSheet, values: &[&str]) {
        for (i, v) in values.iter().enumerate() {
            if i > 0 {
                sheet.code(9);
            }
            sheet.type_text(v);
        }
    }

    #[test]
    fn the_fields_sit_inside_the_panel_below_the_title() {
        let s = sample();
        let (panel, fields) = s.geometry(window(), bar());
        assert_eq!(fields.len(), 3);
        for r in &fields {
            assert!(r.x >= panel.x && r.right() <= panel.right() && r.bottom() < panel.bottom());
            assert!(r.y > panel.y + TITLE_PT);
        }
        assert!(fields[1].y > fields[0].y);
    }

    #[test]
    fn tab_moves_the_focus_and_a_press_on_a_field_takes_it() {
        let mut s = sample();
        assert_eq!(s.code(9), Outcome::Redraw);
        assert_eq!(s.focus, 1);
        let (_, fields) = s.geometry(window(), bar());
        let r = fields[2];
        assert_eq!(s.press(window(), bar(), r.x + 4.0, r.y + 4.0), Outcome::Redraw);
        assert_eq!(s.focus, 2);
    }

    #[test]
    fn add_returns_the_trimmed_values_only_when_every_field_has_text() {
        let mut s = sample();
        type_all(&mut s, &["example.com", "  me ", ""]);
        assert_eq!(s.code(13), Outcome::Redraw, "the password is empty");
        assert!(s.error.is_some());
        s.type_text("secret");
        assert!(s.error.is_none(), "typing clears the refusal");
        assert_eq!(s.code(13), Outcome::Add(vec!["example.com".into(), "me".into(), "secret".into()]));
    }

    #[test]
    fn escape_and_the_cancel_button_cancel() {
        let mut s = sample();
        assert_eq!(s.code(27), Outcome::Cancel);
        let l = s.sheet().layout(window(), bar(), 1.0);
        let (_, r) = l.buttons.iter().find(|(id, _)| *id == CANCEL).copied().unwrap();
        assert_eq!(s.press(window(), bar(), r.x + 2.0, r.y + 2.0), Outcome::Cancel);
    }

    #[test]
    fn a_press_outside_the_panel_is_swallowed_and_backspace_deletes() {
        let mut s = sample();
        assert_eq!(s.press(window(), bar(), 4.0, 600.0), Outcome::None);
        s.type_text("ab");
        s.code(8);
        assert_eq!(&*s.fields[0].text, "a");
    }

    #[test]
    fn a_secret_field_shows_one_bullet_per_character() {
        let mut s = sample();
        type_all(&mut s, &["a", "b", "pw\u{e9}"]);
        assert_eq!(&*ItemSheet::shown(&s.fields[2]), "\u{2022}\u{2022}\u{2022}");
        assert_eq!(&*ItemSheet::shown(&s.fields[0]), "a");
    }
}
