//! Browser-owned color selection built from the SDK color model, sliders, text field and push buttons.
use bifrost_app::controls::{panels::ColorPanel, values::Slider as SliderModel, Node, Rect, Role, TextField};
use bifrost_app::kit::ui::{Weight, CARD_BG, LABEL, LABEL_2, MAC_BODY_PT, MAC_FOOTNOTE_PT, MAC_TITLE2_PT};
use bifrost_app::kit::{Surface, Ui};
use bifrost_app::view::{Button, Slider, View, ViewEvent};
use bifrost_app::{widgets, KeyInput};

pub(super) const CONTROL_BASE: u64 = 0x200000;
const PAD: f32 = 16.0;
const WIDTH: f32 = 340.0;
const ROW: f32 = 34.0;
const BUTTON: f32 = 28.0;

pub(super) struct ColorState {
    panel: ColorPanel,
    hex: TextField,
    sliders: [SliderModel; 3],
    focus: usize,
    drag: Option<usize>,
    invalid: bool,
}

pub(super) struct Layout {
    pub card: Rect,
    pub hex: Rect,
    pub sliders: [Rect; 3],
    pub cancel: Rect,
    pub apply: Rect,
    swatch: Rect,
}

impl ColorState {
    pub fn new(color: (u8, u8, u8)) -> Self {
        let panel = ColorPanel::new(color);
        let mut hex = TextField::new();
        hex.push_str(&panel.hex());
        let _ = hex.command(bifrost_app::controls::Command::SelectAll, "");
        Self { panel, hex, sliders: [color.0, color.1, color.2].map(|value| SliderModel::new(0.0, 255.0, value as f32, Some(1.0))), focus: 0, drag: None, invalid: false }
    }

    pub fn color(&self) -> (u8, u8, u8) { self.panel.color() }

    pub fn layout(ui: &Ui, w: u32, h: u32) -> Layout {
        let (w, h) = (w as f32 / ui.pt(1.0), h as f32 / ui.pt(1.0));
        let width = WIDTH.min((w - PAD).max(0.0));
        let height = PAD * 2.0 + 20.0 + 20.0 + ROW * 5.0 + BUTTON + 20.0;
        let card = Rect { x: ((w - width) / 2.0).max(0.0), y: ((h - height) / 2.0).max(8.0), w: width, h: height };
        let x = card.x + PAD;
        let inner = (width - PAD * 2.0).max(0.0);
        let swatch = Rect { x, y: card.y + PAD + 48.0, w: 48.0_f32.min(inner), h: ROW };
        let hex = Rect { x: x + 64.0_f32.min(inner), y: swatch.y + 3.0, w: (inner - 64.0).max(0.0), h: BUTTON };
        let sliders = [0, 1, 2].map(|i| Rect { x: x + 48.0, y: swatch.y + ROW + ROW * i as f32, w: (inner - 84.0).max(0.0), h: ROW });
        let by = card.y + card.h - PAD - BUTTON;
        let bw = ((inner - 8.0) / 2.0).max(0.0);
        Layout { card, hex, sliders, swatch, cancel: Rect { x, y: by, w: bw, h: BUTTON }, apply: Rect { x: x + bw + 8.0, y: by, w: bw, h: BUTTON } }
    }

    fn update_rgb(&mut self) {
        self.panel.set_rgb((self.sliders[0].value() as u8, self.sliders[1].value() as u8, self.sliders[2].value() as u8));
        self.hex.clear();
        self.hex.push_str(&self.panel.hex());
        self.invalid = false;
    }

    fn update_hex(&mut self) {
        self.invalid = !self.panel.set_hex(&self.hex.to_string());
        if !self.invalid {
            let color = self.panel.color();
            for (slider, value) in self.sliders.iter_mut().zip([color.0, color.1, color.2]) { slider.set(value as f32); }
        }
    }

    fn drag_slider(&mut self, index: usize, rect: Rect, x: f32) {
        let mut slider = Slider::new("Color channel", self.sliders[index], "");
        slider.set_frame(rect);
        slider.handle(&ViewEvent::Pointer(bifrost_app::kit::Event::Down(x, rect.y + rect.h / 2.0)));
        self.sliders[index] = slider.model;
        self.update_rgb();
    }

    pub fn down(&mut self, layout: &Layout, x: f32, y: f32) {
        if layout.hex.contains(x, y) { self.focus = 0; }
        self.drag = layout.sliders.iter().position(|rect| rect.contains(x, y));
        if let Some(index) = self.drag {
            self.focus = index + 1;
            self.drag_slider(index, layout.sliders[index], x);
        }
    }

    pub fn move_to(&mut self, layout: &Layout, x: f32) {
        if let Some(index) = self.drag {
            self.drag_slider(index, layout.sliders[index], x);
        }
    }

    pub fn up(&mut self, layout: &Layout, x: f32, y: f32) -> Option<bool> {
        if self.drag.take().is_some() { return None; }
        if layout.cancel.contains(x, y) { return Some(false); }
        if layout.apply.contains(x, y) && !self.invalid { return Some(true); }
        if layout.hex.contains(x, y) { self.focus = 0; }
        None
    }

    pub fn cancel_drag(&mut self) { self.drag = None; }

    pub fn activate(&mut self, id: u64) -> Option<bool> {
        let focus = id.checked_sub(CONTROL_BASE)? as usize;
        match focus {
            0..=3 => self.focus = focus,
            4 => return Some(false),
            5 if !self.invalid => return Some(true),
            _ => {}
        }
        None
    }

    pub fn key(&mut self, key: &KeyInput) -> Option<bool> {
        if key.as_chord().is_some_and(|(key, mods)| key == 15 && mods & bifrost_app::proto::CHORD_SHIFT != 0) {
            self.focus = (self.focus + 5) % 6;
            return None;
        }
        if self.focus == 0 {
            if let Some((key, _)) = key.as_chord().filter(|(_, mods)| mods & (bifrost_app::proto::CHORD_CTRL | bifrost_app::proto::CHORD_META) != 0) {
                use bifrost_app::controls::Command;
                let command = match key { 30 => Some(Command::SelectAll), 46 => Some(Command::Copy), 45 => Some(Command::Cut), 47 => Some(Command::Paste), _ => None };
                if let Some(command) = command {
                    let clipboard = bifrost_app::kit::clipboard::text().unwrap_or_default();
                    if command != Command::Paste || clipboard.len() <= 16 {
                        if let Some(text) = self.hex.command(command, &clipboard) { bifrost_app::kit::clipboard::set_text(&text); }
                        self.update_hex();
                    }
                    return None;
                }
            }
        }
        match key {
            KeyInput::Code(1 | 27) => return Some(false),
            KeyInput::Code(9) => self.focus = (self.focus + 1) % 6,
            KeyInput::Code(13) => return if self.focus == 4 { Some(false) } else if !self.invalid { Some(true) } else { None },
            KeyInput::Code(37 | 39) if (1..=3).contains(&self.focus) => {
                self.sliders[self.focus - 1].nudge(if matches!(key, KeyInput::Code(37)) { -1 } else { 1 });
                self.update_rgb();
            }
            _ if self.focus == 0 => {
                match key {
                    KeyInput::Text(text) => {
                        if self.hex.to_string().len() + text.len() <= 16 { self.hex.push_str(text); }
                    }
                    KeyInput::Code(8) => self.hex.pop(),
                    KeyInput::Code(46) => self.hex.delete(),
                    KeyInput::Code(37) => self.hex.move_caret(false),
                    KeyInput::Code(39) => self.hex.move_caret(true),
                    _ => {}
                }
                self.update_hex();
            }
            _ => {}
        }
        None
    }

    pub fn draw(&self, s: &mut Surface, ui: &Ui, w: u32, h: u32, origin: &str) {
        let layout = Self::layout(ui, w, h);
        let card = layout.card;
        s.fill_rect(0, 0, s.w, s.h, crate::tokens::scrim(110));
        s.fill_rrect(ui.pt(card.x), ui.pt(card.y), ui.pt(card.w), ui.pt(card.h), ui.pt(bifrost_app::controls::prefs_layout::m::CARD_RADIUS), CARD_BG.now());
        let title = ui.text("Browser — Choose Color", MAC_TITLE2_PT, Weight::Semibold, LABEL.now());
        s.blit(&title, ui.pt(card.x + PAD) as i32, ui.pt(card.y + PAD) as i32, 255, 255);
        let from = ui.text(&ui.fit(&format!("For {origin}"), MAC_FOOTNOTE_PT, ui.pt((card.w - PAD * 2.0).max(0.0))), MAC_FOOTNOTE_PT, Weight::Regular, LABEL_2.now());
        s.blit(&from, ui.pt(card.x + PAD) as i32, ui.pt(card.y + PAD + 24.0) as i32, 255, 255);
        let (r, g, b) = self.panel.color();
        let color = bifrost_app::kit::pack(r, g, b, 255);
        s.fill_rrect(ui.pt(layout.swatch.x), ui.pt(layout.swatch.y), ui.pt(layout.swatch.w), ui.pt(layout.swatch.h), ui.pt(bifrost_app::controls::prefs_layout::m::ICON_RADIUS), color);
        widgets::text_field(s, ui, &self.hex, layout.hex, "#RRGGBB", self.focus == 0);
        for (index, rect) in layout.sliders.iter().enumerate() {
            let text = ui.text(["Red", "Green", "Blue"][index], MAC_BODY_PT, Weight::Regular, LABEL.now());
            s.blit(&text, ui.pt(card.x + PAD) as i32, ui.pt(rect.y + (rect.h - MAC_BODY_PT) / 2.0) as i32, 255, 255);
            let mut slider = Slider::new(["Red", "Green", "Blue"][index], self.sliders[index], "");
            slider.set_frame(*rect);
            slider.base_mut().focused = self.focus == index + 1;
            slider.draw(s, ui);
            let value = ui.text(&format!("{}", self.sliders[index].value() as u8), MAC_BODY_PT, Weight::Regular, LABEL_2.now());
            s.blit(&value, ui.pt(rect.x + rect.w + 4.0) as i32, ui.pt(rect.y + (rect.h - MAC_BODY_PT) / 2.0) as i32, 255, 255);
        }
        if self.invalid {
            let error = ui.text("Enter a color such as #336699.", MAC_FOOTNOTE_PT, Weight::Regular, LABEL_2.now());
            s.blit(&error, ui.pt(card.x + PAD) as i32, ui.pt(layout.cancel.y - 20.0) as i32, 255, 255);
        }
        draw_button(s, ui, layout.cancel, "Cancel", self.focus == 4, true);
        draw_button(s, ui, layout.apply, "Apply", self.focus == 5, !self.invalid);
    }

    pub fn controls(&self, ui: &Ui, w: u32, h: u32) -> Vec<Node> {
        let layout = Self::layout(ui, w, h);
        let mut nodes = vec![Node { id: CONTROL_BASE, role: Role::TextField, label: "Hex color".into(), value: self.hex.to_string(), bounds: layout.hex, enabled: true }];
        for (index, rect) in layout.sliders.iter().enumerate() {
            nodes.push(Node { id: CONTROL_BASE + index as u64 + 1, role: Role::Slider, label: ["Red", "Green", "Blue"][index].into(), value: format!("{}", self.sliders[index].value() as u8), bounds: *rect, enabled: true });
        }
        for (index, (label, bounds)) in [("Cancel", layout.cancel), ("Apply", layout.apply)].into_iter().enumerate() {
            nodes.push(Node { id: CONTROL_BASE + index as u64 + 4, role: Role::Button, label: label.into(), value: String::new(), bounds, enabled: index == 0 || !self.invalid });
        }
        nodes
    }
}

pub(super) fn draw_button(s: &mut Surface, ui: &Ui, frame: Rect, label: &str, focused: bool, enabled: bool) {
    let mut button = Button::new(label, "");
    button.set_frame(frame);
    button.base_mut().focused = focused;
    button.base_mut().enabled = enabled;
    button.draw(s, ui);
}
