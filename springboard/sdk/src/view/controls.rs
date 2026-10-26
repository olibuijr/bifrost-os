//! Control views: thin `View`s over the bifrost-ui value models (`Slider`, `Segmented`) and the shared drawers.
//! Sizes are named point constants, colours are the named pairs from render ui.rs, text uses `MAC_*_PT`.
use super::button_style::{ButtonConfiguration, ButtonRole, ButtonStyle};
use super::{blit, View, ViewBase, ViewEvent};
use crate::kit::ui::{Weight, CONTROL_PRESSED, FIELD_BG, LABEL, LABEL_2, LABEL_3, SEGMENT_ON, SYSTEM_GREEN, accent_now, TRACK_C, WHITE};
use crate::kit::{Event, Ui};
use bifrost_render::canvas::{Canvas, RectF};
use crate::layout::ROW_PT;
use crate::proto::KeyInput;
use bifrost_render::ui::{MAC_BODY_PT, SPACE_L, SPACE_S};
use bifrost_ui::geometry::EdgeInsets;
use bifrost_ui::stack::{distribute, stack, Axis, Distribution, StackItem};
use bifrost_ui::{values, Node, Rect, Role};

/// Focus ring thickness, in points.
const FOCUS_RING_PT: f32 = 2.0;
/// `NativeSwitch` track size and the gap between track and knob, in points.
const SWITCH_W_PT: f32 = 38.0;
const SWITCH_H_PT: f32 = 22.0;
const SWITCH_INSET_PT: f32 = 2.0;
/// `NativeSlider` track thickness and knob diameter, in points.
const SLIDER_TRACK_PT: f32 = 4.0;
const SLIDER_KNOB_PT: f32 = 18.0;

pub(super) fn is_activate(ev: &ViewEvent) -> bool {
    match ev {
        ViewEvent::Key(KeyInput::Code(13 | 32)) => true,
        ViewEvent::Key(KeyInput::Text(t)) => t == " ",
        _ => false,
    }
}

/// Draws `text` inside `frame` (points), vertically centred; horizontally centred or leading.
pub(super) fn draw_text(s: &mut dyn Canvas, ui: &Ui, frame: Rect, text: &str, pt: f32, weight: Weight, color: u32, centred: bool) {
    let img = ui.text(&ui.fit(text, pt, ui.pt(frame.w)), pt, weight, color);
    let x = if centred { ui.pt(frame.x) + (ui.pt(frame.w) - img.w as f32) / 2.0 } else { ui.pt(frame.x) };
    let y = ui.pt(frame.y) + (ui.pt(frame.h) - img.h as f32) / 2.0;
    blit(s, &img, x as i32, y as i32);
}

/// The baselines of a view that draws one line of `pt` text centred in its frame (`draw_text`): first and last are
/// the same line.
pub(super) fn centred_baselines(ui: &Ui, pt: f32, frame: Rect) -> Option<(f32, f32)> {
    let b = ui.centred_baseline(pt, frame.h);
    Some((b, b))
}

pub(super) fn focus_ring(s: &mut dyn Canvas, ui: &Ui, f: Rect, radius: f32) {
    let t = ui.pt(FOCUS_RING_PT);
    s.stroke_rrect(RectF::new(ui.pt(f.x) - t, ui.pt(f.y) - t, ui.pt(f.w) + 2.0 * t, ui.pt(f.h) + 2.0 * t), ui.pt(radius) + t, t, accent_now());
}

pub(super) fn node(v: &dyn View, role: Role, label: &str, value: &str) -> Option<Node> {
    Some(Node { id: v.id(), role, label: label.into(), value: value.into(), bounds: v.frame(), enabled: v.base().enabled })
}

/// Static text (`NativeTextField` label).
pub struct Label {
    base: ViewBase,
    text: String,
    pub pt: f32,
    pub weight: Weight,
    /// secondary (grey) text
    pub secondary: bool,
}

impl Label {
    pub fn new(text: &str) -> Self {
        Label { base: ViewBase::default(), text: text.into(), pt: MAC_BODY_PT, weight: Weight::Regular, secondary: false }
    }
    pub fn text(&self) -> &str {
        &self.text
    }
    pub fn set_text(&mut self, text: &str) {
        if self.text != text {
            self.text = text.into();
            self.base.dirty = true;
        }
    }
}

impl View for Label {
    crate::view_base!();
    fn intrinsic(&self, ui: &Ui) -> (f32, f32) {
        (ui.measure(&self.text, self.pt, self.weight) / ui.pt(1.0), self.pt + SPACE_S)
    }
    fn baselines(&self, ui: &Ui) -> Option<(f32, f32)> {
        centred_baselines(ui, self.pt, self.base.frame)
    }
    fn draw(&mut self, s: &mut dyn Canvas, ui: &Ui) {
        let c = if !self.base.enabled { LABEL_3 } else if self.secondary { LABEL_2 } else { LABEL };
        draw_text(s, ui, self.base.frame, &self.text, self.pt, self.weight, c.now(), false);
    }
    fn ax_node(&self) -> Option<Node> {
        node(self, Role::StaticText, &self.text, "")
    }
}

/// A push button that sends `action` (`NativeButton` with a nil target). Its look comes from a [`ButtonStyle`]
/// (default [`Bordered`](super::button_style::Bordered)), `buttonStyle(_:)`; its `role` tells the style its purpose.
pub struct Button {
    base: ViewBase,
    title: String,
    action: String,
    pressed: bool,
    style: Box<dyn ButtonStyle>,
    role: Option<ButtonRole>,
}

impl Button {
    pub fn new(title: &str, action: &str) -> Self {
        Button { base: ViewBase::default(), title: title.into(), action: action.into(), pressed: false, style: Box::new(super::button_style::Bordered), role: None }
    }
    /// `buttonStyle(_:)`.
    pub fn styled(mut self, style: impl ButtonStyle) -> Self {
        self.set_style(Box::new(style));
        self
    }
    pub fn set_style(&mut self, style: Box<dyn ButtonStyle>) {
        self.style = style;
        self.base.dirty = true;
    }
    /// `Button(_:role:action:)`.
    pub fn with_role(mut self, role: ButtonRole) -> Self {
        self.role = Some(role);
        self
    }
    pub fn role(&self) -> Option<ButtonRole> {
        self.role
    }
    /// What the style sees (`ButtonStyleConfiguration`).
    pub fn configuration(&self) -> ButtonConfiguration<'_> {
        ButtonConfiguration { label: &self.title, is_pressed: self.pressed, role: self.role, enabled: self.base.enabled, focused: self.base.focused, frame: self.base.frame }
    }
    pub fn title(&self) -> &str {
        &self.title
    }
    pub fn set_title(&mut self, title: &str) {
        self.title = title.into();
        self.base.dirty = true;
    }
    fn fire(&mut self) {
        let a = self.action.clone();
        self.base.send_action(&a);
    }
}

impl View for Button {
    crate::view_base!();
    fn intrinsic(&self, ui: &Ui) -> (f32, f32) {
        self.style.size(ui, &self.configuration())
    }
    fn baselines(&self, ui: &Ui) -> Option<(f32, f32)> {
        centred_baselines(ui, MAC_BODY_PT, self.base.frame)
    }
    fn accepts_first_responder(&self) -> bool {
        true
    }
    fn draw(&mut self, s: &mut dyn Canvas, ui: &Ui) {
        let f = self.base.frame;
        self.style.draw(s, ui, &self.configuration());
        if self.base.focused {
            focus_ring(s, ui, f, f.h / 2.0);
        }
    }
    fn handle(&mut self, ev: &ViewEvent) -> bool {
        if !self.base.enabled {
            return false;
        }
        let was = self.pressed;
        let handled = match ev {
            ViewEvent::Pointer(Event::Down(..)) => {
                self.pressed = true;
                true
            }
            ViewEvent::Pointer(Event::Move(x, y)) => {
                self.pressed = self.base.frame.contains(*x, *y);
                true
            }
            ViewEvent::Pointer(Event::Up(x, y)) => {
                if self.pressed && self.base.frame.contains(*x, *y) {
                    self.fire();
                }
                self.pressed = false;
                true
            }
            ViewEvent::Cancel => {
                self.pressed = false;
                true
            }
            k if is_activate(k) => {
                self.fire();
                true
            }
            _ => false,
        };
        if was != self.pressed {
            self.base.dirty = true;
        }
        handled
    }
    fn ax_node(&self) -> Option<Node> {
        node(self, Role::Button, &self.title, "")
    }
}

/// An on/off switch (`NativeSwitch`) that sends `action` when toggled; read the state with `on()`.
pub struct Switch {
    base: ViewBase,
    label: String,
    action: String,
    on: bool,
    pressed: bool,
}

impl Switch {
    pub fn new(label: &str, on: bool, action: &str) -> Self {
        Switch { base: ViewBase::default(), label: label.into(), action: action.into(), on, pressed: false }
    }
    pub fn on(&self) -> bool {
        self.on
    }
    pub fn set_on(&mut self, on: bool) {
        if self.on != on {
            self.on = on;
            self.base.dirty = true;
        }
    }
    fn toggle(&mut self) {
        self.set_on(!self.on);
        let a = self.action.clone();
        self.base.send_action(&a);
    }
    fn track(&self) -> Rect {
        let f = self.base.frame;
        Rect { x: f.right() - SWITCH_W_PT, y: f.y + (f.h - SWITCH_H_PT) / 2.0, w: SWITCH_W_PT, h: SWITCH_H_PT }
    }
}

impl View for Switch {
    crate::view_base!();
    fn intrinsic(&self, ui: &Ui) -> (f32, f32) {
        (ui.measure(&self.label, MAC_BODY_PT, Weight::Regular) / ui.pt(1.0) + SPACE_L + SWITCH_W_PT, ROW_PT)
    }
    fn baselines(&self, ui: &Ui) -> Option<(f32, f32)> {
        centred_baselines(ui, MAC_BODY_PT, self.base.frame)
    }
    fn accepts_first_responder(&self) -> bool {
        true
    }
    fn draw(&mut self, s: &mut dyn Canvas, ui: &Ui) {
        let f = self.base.frame;
        let t = self.track();
        let c = if self.base.enabled { LABEL } else { LABEL_3 };
        draw_text(s, ui, Rect { w: (f.w - SWITCH_W_PT - SPACE_S).max(0.0), ..f }, &self.label, MAC_BODY_PT, Weight::Regular, c.now(), false);
        let fill = if self.on { SYSTEM_GREEN } else { TRACK_C.now() };
        s.fill_rrect(RectF::new(ui.pt(t.x), ui.pt(t.y), ui.pt(t.w), ui.pt(t.h)), ui.pt(t.h / 2.0), fill);
        let d = t.h - 2.0 * SWITCH_INSET_PT;
        let kx = if self.on { t.right() - SWITCH_INSET_PT - d } else { t.x + SWITCH_INSET_PT };
        s.fill_rrect(RectF::new(ui.pt(kx), ui.pt(t.y + SWITCH_INSET_PT), ui.pt(d), ui.pt(d)), ui.pt(d / 2.0), WHITE);
        if self.base.focused {
            focus_ring(s, ui, t, t.h / 2.0);
        }
    }
    fn handle(&mut self, ev: &ViewEvent) -> bool {
        if !self.base.enabled {
            return false;
        }
        match ev {
            ViewEvent::Pointer(Event::Down(..)) => self.pressed = true,
            ViewEvent::Pointer(Event::Move(..)) => {}
            ViewEvent::Pointer(Event::Up(x, y)) => {
                if self.pressed && self.base.frame.contains(*x, *y) {
                    self.toggle();
                }
                self.pressed = false;
            }
            ViewEvent::Cancel => self.pressed = false,
            k if is_activate(k) => self.toggle(),
            _ => return false,
        }
        true
    }
    fn ax_node(&self) -> Option<Node> {
        node(self, Role::Toggle, &self.label, if self.on { "on" } else { "off" })
    }
    fn ax_checked(&self) -> Option<bool> {
        Some(self.on)
    }
    fn ax_set_value(&mut self, value: &str) -> bool {
        match super::forms::parse_bool(value) {
            Some(b) => {
                if b != self.on {
                    self.toggle();
                }
                true
            }
            None => false,
        }
    }
}

/// A horizontal slider (`NativeSlider`) over `values::Slider`; sends `action` whenever the value changes.
pub struct Slider {
    base: ViewBase,
    label: String,
    action: String,
    pub model: values::Slider,
    dragging: bool,
}

impl Slider {
    pub fn new(label: &str, model: values::Slider, action: &str) -> Self {
        Slider { base: ViewBase::default(), label: label.into(), action: action.into(), model, dragging: false }
    }
    pub fn value(&self) -> f32 {
        self.model.value()
    }
    /// The track the knob travels on: the frame less half a knob each side.
    fn track(&self) -> Rect {
        let f = self.base.frame;
        Rect { x: f.x + SLIDER_KNOB_PT / 2.0, y: f.y + f.h / 2.0, w: (f.w - SLIDER_KNOB_PT).max(0.0), h: 0.0 }
    }
    fn changed(&mut self, changed: bool) {
        if changed {
            self.base.dirty = true;
            let a = self.action.clone();
            self.base.send_action(&a);
        }
    }
}

impl View for Slider {
    crate::view_base!();
    fn intrinsic(&self, _ui: &Ui) -> (f32, f32) {
        (0.0, ROW_PT)
    }
    fn accepts_first_responder(&self) -> bool {
        true
    }
    fn draw(&mut self, s: &mut dyn Canvas, ui: &Ui) {
        let t = self.track();
        let kx = self.model.knob_x(t);
        s.fill_rrect(RectF::new(ui.pt(t.x), ui.pt(t.y - SLIDER_TRACK_PT / 2.0), ui.pt(t.w), ui.pt(SLIDER_TRACK_PT)), ui.pt(SLIDER_TRACK_PT / 2.0), TRACK_C.now());
        s.fill_rrect(RectF::new(ui.pt(t.x), ui.pt(t.y - SLIDER_TRACK_PT / 2.0), ui.pt(kx - t.x), ui.pt(SLIDER_TRACK_PT)), ui.pt(SLIDER_TRACK_PT / 2.0), accent_now());
        let k = Rect { x: kx - SLIDER_KNOB_PT / 2.0, y: t.y - SLIDER_KNOB_PT / 2.0, w: SLIDER_KNOB_PT, h: SLIDER_KNOB_PT };
        s.fill_rrect(RectF::new(ui.pt(k.x), ui.pt(k.y), ui.pt(k.w), ui.pt(k.h)), ui.pt(k.h / 2.0), if self.base.enabled { WHITE } else { CONTROL_PRESSED.now() });
        if self.base.focused {
            focus_ring(s, ui, k, k.h / 2.0);
        }
    }
    fn handle(&mut self, ev: &ViewEvent) -> bool {
        if !self.base.enabled {
            return false;
        }
        let t = self.track();
        match ev {
            ViewEvent::Pointer(Event::Down(x, _)) => {
                self.dragging = true;
                let c = self.model.drag(t, *x);
                self.changed(c);
            }
            ViewEvent::Pointer(Event::Move(x, _)) if self.dragging => {
                let c = self.model.drag(t, *x);
                self.changed(c);
            }
            ViewEvent::Pointer(_) | ViewEvent::Cancel => self.dragging = false,
            ViewEvent::Key(KeyInput::Code(c @ (37 | 39))) => {
                let c = self.model.nudge(if *c == 37 { -1 } else { 1 });
                self.changed(c);
            }
            _ => return false,
        }
        true
    }
    fn ax_node(&self) -> Option<Node> {
        node(self, Role::Slider, &self.label, &format!("{}", self.model.value()))
    }
    fn ax_set_value(&mut self, value: &str) -> bool {
        match value.trim().parse::<f32>() {
            Ok(v) => {
                let c = self.model.set(v);
                self.changed(c);
                true
            }
            Err(_) => false,
        }
    }
}

/// A segmented control (`NativeSegmentedControl`, select-one) over `values::Segmented`; sends `action` on a new selection.
pub struct Segmented {
    base: ViewBase,
    labels: Vec<String>,
    action: String,
    pub model: values::Segmented,
}

impl Segmented {
    pub fn new(labels: &[&str], selected: Option<usize>, action: &str) -> Self {
        Segmented { base: ViewBase::default(), labels: labels.iter().map(|l| l.to_string()).collect(), action: action.into(), model: values::Segmented::new(labels.len(), selected) }
    }
    pub fn selected(&self) -> Option<usize> {
        self.model.selected()
    }
    fn changed(&mut self, changed: bool) {
        if changed {
            let a = self.action.clone();
            self.base.send_action(&a);
        }
        self.base.dirty = true;
    }
}

impl View for Segmented {
    crate::view_base!();
    fn intrinsic(&self, ui: &Ui) -> (f32, f32) {
        let w = self.labels.iter().map(|l| ui.measure(l, MAC_BODY_PT, Weight::Regular) / ui.pt(1.0) + 2.0 * SPACE_L).sum();
        (w, ROW_PT)
    }
    fn baselines(&self, ui: &Ui) -> Option<(f32, f32)> {
        centred_baselines(ui, MAC_BODY_PT, self.base.frame)
    }
    fn accepts_first_responder(&self) -> bool {
        true
    }
    fn draw(&mut self, s: &mut dyn Canvas, ui: &Ui) {
        let f = self.base.frame;
        s.fill_rrect(RectF::new(ui.pt(f.x), ui.pt(f.y), ui.pt(f.w), ui.pt(f.h)), ui.pt(f.h / 2.0), FIELD_BG.now());
        let n = self.labels.len().max(1) as f32;
        for (i, l) in self.labels.iter().enumerate() {
            let cell = Rect { x: f.x + f.w * i as f32 / n, y: f.y, w: f.w / n, h: f.h };
            if self.model.selected() == Some(i) {
                s.fill_rrect(RectF::new(ui.pt(cell.x), ui.pt(cell.y), ui.pt(cell.w), ui.pt(cell.h)), ui.pt(cell.h / 2.0), SEGMENT_ON.now());
            }
            let c = if self.model.is_enabled(i) && self.base.enabled { LABEL } else { LABEL_3 };
            draw_text(s, ui, cell, l, MAC_BODY_PT, Weight::Regular, c.now(), true);
        }
        if self.base.focused {
            focus_ring(s, ui, f, f.h / 2.0);
        }
    }
    fn handle(&mut self, ev: &ViewEvent) -> bool {
        if !self.base.enabled {
            return false;
        }
        let f = self.base.frame;
        match ev {
            ViewEvent::Pointer(Event::Down(x, y)) => self.model.press(f, *x, *y),
            ViewEvent::Pointer(Event::Up(x, y)) => {
                let c = self.model.release(f, *x, *y);
                self.changed(c);
            }
            ViewEvent::Pointer(_) => {}
            ViewEvent::Cancel => self.model.press(Rect::default(), 0.0, 0.0),
            ViewEvent::Key(KeyInput::Code(c @ (37 | 39))) => {
                let c = self.model.step(if *c == 37 { -1 } else { 1 });
                self.changed(c);
            }
            _ => return false,
        }
        true
    }
    fn ax_node(&self) -> Option<Node> {
        let value = self.model.selected().and_then(|i| self.labels.get(i)).map(String::as_str).unwrap_or("");
        node(self, Role::SegmentedControl, &self.labels.join(", "), value)
    }
}

/// How a stack places a child across its axis (`NativeStackView.alignment`). For a horizontal stack `Leading` is
/// `.top` and `Trailing` is `.bottom` (`Align::TOP`, `Align::BOTTOM`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    /// the full cross extent
    Fill,
    Leading,
    Center,
    Trailing,
    /// `.firstTextBaseline` (horizontal stacks): the children's top-most text baselines line up. A child without text
    /// (`View::baselines` is `None`) aligns its bottom edge with them, as the reference guide does. A vertical stack
    /// treats it as `Leading`.
    FirstBaseline,
    /// `.lastTextBaseline` (horizontal stacks): the bottom-most text baselines line up; otherwise as `FirstBaseline`.
    LastBaseline,
}

impl Align {
    pub const TOP: Align = Align::Leading;
    pub const BOTTOM: Align = Align::Trailing;
}

/// `NativeStackView`: arranges children along an axis. `distribution` picks how they share the main axis (default
/// `Fill`: a child with an intrinsic length keeps it, one without shares the spare space; when every child has an
/// intrinsic length the ones with a strictly lower content hugging priority than the rest stretch instead). `align`
/// places them across it. Children with an intrinsic cross size keep it unless `align` is `Fill`.
pub struct Stack {
    base: ViewBase,
    axis: Axis,
    pub spacing: f32,
    pub insets: EdgeInsets,
    pub align: Align,
    pub distribution: Distribution,
    children: Vec<Box<dyn View>>,
}

impl Stack {
    pub fn new(axis: Axis, spacing: f32, children: Vec<Box<dyn View>>) -> Self {
        Stack { base: ViewBase::default(), axis, spacing, insets: EdgeInsets::ZERO, align: Align::Leading, distribution: Distribution::Fill, children }
    }
    /// `NativeStackView` with a vertical orientation.
    pub fn vstack(spacing: f32, children: Vec<Box<dyn View>>) -> Self {
        Stack::new(Axis::Vertical, spacing, children)
    }
    pub fn hstack(spacing: f32, children: Vec<Box<dyn View>>) -> Self {
        Stack::new(Axis::Horizontal, spacing, children)
    }
    /// `NativeStackView.distribution`.
    pub fn distribution(mut self, d: Distribution) -> Self {
        self.distribution = d;
        self
    }
    /// `NativeStackView.alignment`.
    pub fn aligned(mut self, a: Align) -> Self {
        self.align = a;
        self
    }
}

impl View for Stack {
    crate::view_base!();
    fn children(&self) -> &[Box<dyn View>] {
        &self.children
    }
    fn children_mut(&mut self) -> &mut [Box<dyn View>] {
        &mut self.children
    }
    fn layout(&mut self, ui: &Ui, bounds: Rect) {
        self.set_frame(bounds);
        let horizontal = self.axis == Axis::Horizontal;
        let sizes: Vec<(f32, f32)> = self.children.iter().map(|c| c.intrinsic(ui)).collect();
        let main_of = |&(w, h): &(f32, f32)| if horizontal { w } else { h };
        let rects: Vec<Rect> = if self.distribution == Distribution::Fill {
            let hug: Vec<u16> = self.children.iter().map(|c| c.hugging(self.axis)).collect();
            let all_intrinsic = sizes.iter().all(|s| main_of(s) > 0.0);
            let (lo, hi) = (hug.iter().copied().min().unwrap_or(0), hug.iter().copied().max().unwrap_or(0));
            let items: Vec<StackItem> = sizes
                .iter()
                .zip(&hug)
                .map(|(s, &h)| match main_of(s) {
                    m if m > 0.0 && !(all_intrinsic && lo < hi && h == lo) => StackItem::fixed(m),
                    m => StackItem::flexible(m.max(0.0), 1.0),
                })
                .collect();
            stack(bounds, self.axis, self.spacing, self.insets, &items)
        } else {
            let area = bounds.inset(self.insets);
            let main = if horizontal { area.w } else { area.h };
            let lens: Vec<f32> = sizes.iter().map(main_of).collect();
            distribute(main, self.spacing, &lens, self.distribution)
                .into_iter()
                .map(|(off, len)| if horizontal { Rect { x: area.x + off, y: area.y, w: len, h: area.h } } else { Rect { x: area.x, y: area.y + off, w: area.w, h: len } })
                .collect()
        };
        let by_baseline = horizontal && matches!(self.align, Align::FirstBaseline | Align::LastBaseline);
        for ((c, r), (w, h)) in self.children.iter_mut().zip(rects).zip(sizes) {
            let cross = if horizontal { h } else { w };
            let room = if horizontal { r.h } else { r.w };
            let r = if self.align == Align::Fill || cross <= 0.0 || cross >= room {
                r
            } else {
                let off = match self.align {
                    Align::Center => (room - cross) / 2.0,
                    Align::Trailing => room - cross,
                    _ => 0.0,
                };
                if horizontal { Rect { y: r.y + off, h: cross, ..r } } else { Rect { x: r.x + off, w: cross, ..r } }
            };
            c.layout(ui, r);
        }
        if by_baseline {
            self.align_baselines(ui);
        }
    }
    /// The first baseline of the top-most text and the last baseline of the bottom-most text among the children
    /// (laid out), relative to the stack's top; `None` when no child has text.
    fn baselines(&self, ui: &Ui) -> Option<(f32, f32)> {
        let top = self.base.frame.y;
        self.children
            .iter()
            .filter(|c| !c.base().hidden)
            .filter_map(|c| c.baselines(ui).map(|(f, l)| (c.frame().y - top + f, c.frame().y - top + l)))
            .fold(None, |acc, (f, l)| Some(acc.map_or((f, l), |(af, al): (f32, f32)| (af.min(f), al.max(l)))))
    }
}

impl Stack {
    /// The baseline guide of child `c` in window points: its first or last text baseline, or its bottom edge.
    fn guide(&self, ui: &Ui, c: &dyn View) -> f32 {
        let f = c.frame();
        let last = self.align == Align::LastBaseline;
        f.y + c.baselines(ui).map_or(f.h, |(first, l)| if last { l } else { first })
    }
    /// Moves the children (laid out top-aligned) down so their baseline guides meet the lowest one. The row then
    /// starts at the stack's top: the child whose guide is lowest relative to its own top stays where it is.
    fn align_baselines(&mut self, ui: &Ui) {
        let guides: Vec<f32> = self.children.iter().map(|c| self.guide(ui, c.as_ref())).collect();
        let Some(line) = guides.iter().copied().reduce(f32::max) else { return };
        for (c, g) in self.children.iter_mut().zip(guides) {
            let dy = line - g;
            if dy != 0.0 {
                super::shift_frames(c.as_mut(), 0.0, dy);
                c.set_needs_display();
            }
        }
    }
}
