//! View groupings: `GroupBox` (a titled card around content), `ControlGroup` (related controls joined on one plate)
//! and `TabView` (tabs over a card; one tab's content shows at a time, over the `bifrost_ui::tab_selection` model).
//! Sizes are named tokens, colours the named pairs from render ui.rs, text `MAC_*_PT`.
use super::button_style::Borderless;
use super::controls::{draw_text, focus_ring, node};
use super::{Button, View, ViewBase, ViewEvent};
use crate::kit::ui::{Weight, CAPSULE_BG, CAPSULE_EDGE, CARD_BG, LABEL_2, SEPARATOR_C};
use crate::kit::{Event, Ui};
use crate::layout::ROW_PT;
use crate::proto::KeyInput;
use bifrost_render::canvas::{Canvas, RectF};
use bifrost_render::ui::{MAC_BODY_PT, MAC_GROUP_HEADER_PT, SPACE_L, SPACE_M, SPACE_S, SPACE_XS};
use bifrost_ui::prefs_layout::m;
use bifrost_ui::sheet::{segment_hit, SEGMENT_H_PT};
use bifrost_ui::tab_selection::TabSelection;
use bifrost_ui::{Node, Rect, Role};

fn fill_card(s: &mut dyn Canvas, ui: &Ui, r: Rect) {
    s.fill_rrect(RectF::new(ui.pt(r.x), ui.pt(r.y), ui.pt(r.w), ui.pt(r.h)), ui.pt(m::CARD_RADIUS), CARD_BG.now());
}

// ---- GroupBox ----

/// `GroupBox`: content inside a rounded card (`m::CARD_RADIUS`, `CARD_BG`) inset by `SPACE_M`, with an optional title
/// above the card in the group-header style (footnote size, secondary grey). Opaque: a dirty child repaints the card.
pub struct GroupBox {
    base: ViewBase,
    title: Option<String>,
    content: Vec<Box<dyn View>>,
}

impl GroupBox {
    /// `GroupBox(content:)`: an unlabeled box.
    pub fn new(content: Box<dyn View>) -> Self {
        GroupBox { base: ViewBase::default(), title: None, content: vec![content] }
    }
    /// `GroupBox(_:content:)`: a box with a title.
    pub fn titled(mut self, title: &str) -> Self {
        self.title = Some(title.into());
        self
    }
    pub fn title(&self) -> Option<&str> {
        self.title.as_deref()
    }
    fn title_h(&self) -> f32 {
        if self.title.is_some() {
            MAC_GROUP_HEADER_PT + SPACE_S
        } else {
            0.0
        }
    }
    /// The card's rectangle inside the frame (below the title).
    pub fn card(&self) -> Rect {
        let f = self.base.frame;
        let t = self.title_h();
        Rect { x: f.x, y: f.y + t, w: f.w, h: (f.h - t).max(0.0) }
    }
    /// `content`, downcast.
    pub fn content<T: View>(&self) -> Option<&T> {
        self.content[0].as_any().downcast_ref::<T>()
    }
}

impl View for GroupBox {
    crate::view_base!();
    fn children(&self) -> &[Box<dyn View>] {
        &self.content
    }
    fn children_mut(&mut self) -> &mut [Box<dyn View>] {
        &mut self.content
    }
    fn opaque_backdrop(&self) -> bool {
        true
    }
    fn intrinsic(&self, ui: &Ui) -> (f32, f32) {
        let (w, h) = self.content[0].intrinsic(ui);
        let w = if w > 0.0 { w + 2.0 * SPACE_M } else { 0.0 };
        let h = if h > 0.0 { h + 2.0 * SPACE_M + self.title_h() } else { 0.0 };
        (w, h)
    }
    fn layout(&mut self, ui: &Ui, bounds: Rect) {
        self.set_frame(bounds);
        let c = self.card();
        let inner = Rect { x: c.x + SPACE_M, y: c.y + SPACE_M, w: (c.w - 2.0 * SPACE_M).max(0.0), h: (c.h - 2.0 * SPACE_M).max(0.0) };
        self.content[0].layout(ui, inner);
    }
    fn draw(&mut self, s: &mut dyn Canvas, ui: &Ui) {
        let f = self.base.frame;
        if let Some(t) = &self.title {
            let r = Rect { x: f.x + SPACE_M, y: f.y, w: (f.w - 2.0 * SPACE_M).max(0.0), h: self.title_h() };
            draw_text(s, ui, r, t, MAC_GROUP_HEADER_PT, Weight::Regular, LABEL_2.now(), false);
        }
        fill_card(s, ui, self.card());
    }
    fn ax_node(&self) -> Option<Node> {
        self.title.as_deref().and_then(|t| node(self, Role::StaticText, t, ""))
    }
}

// ---- ControlGroup ----

/// `ControlGroupStyle` (the styles a desktop control group draws inline).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ControlGroupStyle {
    /// `.automatic`: the controls joined on one capsule plate with hairline dividers
    Automatic,
    /// `.navigation`: as automatic (back and forward pairs)
    Navigation,
    /// `.palette`: the controls in a row, `SPACE_XS` apart, without a shared plate
    Palette,
}

/// `ControlGroup`: semantically related controls shown together. Buttons inside take the borderless style, so the
/// group's plate is their bezel; each gets an equal share of the group's width (at least its own width). The label
/// names the group (`ControlGroup(_:content:)`), for a toolbar overflow menu or assistive technology.
pub struct ControlGroup {
    base: ViewBase,
    label: String,
    style: ControlGroupStyle,
    children: Vec<Box<dyn View>>,
}

impl ControlGroup {
    pub fn new(label: &str, mut children: Vec<Box<dyn View>>) -> Self {
        for c in children.iter_mut() {
            if let Some(b) = c.as_any_mut().downcast_mut::<Button>() {
                b.set_style(Box::new(Borderless));
            }
        }
        ControlGroup { base: ViewBase::default(), label: label.into(), style: ControlGroupStyle::Automatic, children }
    }
    /// `controlGroupStyle(_:)`.
    pub fn styled(mut self, style: ControlGroupStyle) -> Self {
        self.style = style;
        self
    }
    pub fn style(&self) -> ControlGroupStyle {
        self.style
    }
    pub fn label(&self) -> &str {
        &self.label
    }
    fn joined(&self) -> bool {
        self.style != ControlGroupStyle::Palette
    }
    fn gap(&self) -> f32 {
        if self.joined() {
            0.0
        } else {
            SPACE_XS
        }
    }
    /// The children's widths for a group `w` points wide: equal shares when they fit (joined), their own widths
    /// otherwise, scaled down together when the group is narrower than they are.
    fn widths(&self, ui: &Ui, w: f32) -> Vec<f32> {
        let own: Vec<f32> = self.children.iter().map(|c| c.intrinsic(ui).0.max(0.0)).collect();
        let n = own.len();
        if n == 0 {
            return own;
        }
        let room = (w - self.gap() * (n - 1) as f32).max(0.0);
        let total: f32 = own.iter().sum();
        if total > room && total > 0.0 {
            return own.iter().map(|v| v * room / total).collect();
        }
        if self.joined() {
            let share = room / n as f32;
            if own.iter().all(|v| *v <= share) {
                return vec![share; n];
            }
        }
        own
    }
}

impl View for ControlGroup {
    crate::view_base!();
    fn children(&self) -> &[Box<dyn View>] {
        &self.children
    }
    fn children_mut(&mut self) -> &mut [Box<dyn View>] {
        &mut self.children
    }
    fn opaque_backdrop(&self) -> bool {
        self.joined()
    }
    fn intrinsic(&self, ui: &Ui) -> (f32, f32) {
        let n = self.children.len();
        let w: f32 = self.children.iter().map(|c| c.intrinsic(ui).0.max(0.0)).sum::<f32>() + self.gap() * n.saturating_sub(1) as f32;
        let h = self.children.iter().map(|c| c.intrinsic(ui).1).fold(ROW_PT, f32::max);
        (w, h)
    }
    fn layout(&mut self, ui: &Ui, bounds: Rect) {
        self.set_frame(bounds);
        let widths = self.widths(ui, bounds.w);
        let gap = self.gap();
        let mut x = bounds.x;
        for (c, w) in self.children.iter_mut().zip(widths) {
            c.layout(ui, Rect { x, y: bounds.y, w, h: bounds.h });
            x += w + gap;
        }
    }
    fn draw(&mut self, s: &mut dyn Canvas, ui: &Ui) {
        if !self.joined() {
            return;
        }
        let f = self.base.frame;
        let r = RectF::new(ui.pt(f.x), ui.pt(f.y), ui.pt(f.w), ui.pt(f.h));
        s.fill_rrect(r, ui.pt(f.h / 2.0), CAPSULE_BG.now());
        s.stroke_rrect(r, ui.pt(f.h / 2.0), ui.pt(1.0).max(1.0), CAPSULE_EDGE.now());
        let inset = (f.h / 4.0).min(SPACE_S);
        for c in self.children.iter().skip(1) {
            let x = c.frame().x;
            s.fill_rect(RectF::new(ui.pt(x).trunc(), ui.pt(f.y + inset).trunc(), ui.pt(1.0).max(1.0).trunc(), ui.pt((f.h - 2.0 * inset).max(0.0)).trunc()), SEPARATOR_C.now());
        }
    }
}

// ---- TabView ----

/// `TabView` on the desktop: a row of tabs centred on the top edge of a card, the selected tab's content inside the
/// card and the other tabs' content hidden. Clicking a tab, or Left and Right while the tab row has focus, selects it
/// and sends `action` up the responder chain; read the choice with `selected_id`. Tab identifiers are unique (repeats
/// are dropped with their content), as `Tab(value:)` requires.
pub struct TabView {
    base: ViewBase,
    tabs: TabSelection,
    labels: Vec<String>,
    action: String,
    pages: Vec<Box<dyn View>>,
    /// the tab row measured by the last layout (window points)
    last_bar: Option<Rect>,
}

impl TabView {
    /// Tabs as `(id, label, content)`, the first selected.
    pub fn new(action: &str, tabs: Vec<(&str, &str, Box<dyn View>)>) -> Self {
        let mut ids: Vec<String> = Vec::new();
        let mut labels = Vec::new();
        let mut pages = Vec::new();
        for (id, label, page) in tabs {
            if ids.iter().any(|i| i == id) {
                continue;
            }
            ids.push(id.into());
            labels.push(label.into());
            pages.push(page);
        }
        let mut t = TabView { base: ViewBase::default(), tabs: TabSelection::new(ids.as_slice()), labels, action: action.into(), pages, last_bar: None };
        t.show_selected();
        t
    }
    /// `init(selection:content:)` read side: the selected tab's identifier.
    pub fn selected_id(&self) -> Option<&str> {
        self.tabs.selected_id()
    }
    /// Selects tab `id` from app state (no action is sent). False when there is no such tab.
    pub fn select(&mut self, id: &str) -> bool {
        let ok = self.tabs.select(id);
        if ok {
            self.show_selected();
        }
        ok
    }
    fn selected_index(&self) -> Option<usize> {
        let id = self.tabs.selected_id()?;
        self.tabs.ids().iter().position(|i| i == id)
    }
    fn show_selected(&mut self) {
        let sel = self.selected_index();
        for (i, p) in self.pages.iter_mut().enumerate() {
            p.base_mut().hidden = Some(i) != sel;
        }
        self.base.dirty = true;
    }
    /// The user picked tab `i`: select it and tell the chain.
    fn pick(&mut self, i: usize) {
        if self.selected_index() == Some(i) {
            return;
        }
        if let Some(id) = self.tabs.ids().get(i).cloned() {
            self.tabs.select(&id);
            self.show_selected();
            let a = self.action.clone();
            self.base.send_action(&a);
        }
    }
    /// The tab row's rectangle: wide enough for the widest label on every tab, centred, at most the frame's width.
    pub fn bar(&self, ui: &Ui) -> Rect {
        let f = self.base.frame;
        let widest = self.labels.iter().map(|l| ui.measure(l, MAC_BODY_PT, Weight::Regular) / ui.pt(1.0)).fold(0.0, f32::max);
        let w = ((widest + 2.0 * SPACE_L) * self.labels.len() as f32).min(f.w);
        Rect { x: f.x + (f.w - w) / 2.0, y: f.y, w, h: SEGMENT_H_PT }
    }
    /// The card under the tabs: its top edge runs through the middle of the tab row.
    pub fn card(&self) -> Rect {
        let f = self.base.frame;
        let top = SEGMENT_H_PT / 2.0;
        Rect { x: f.x, y: f.y + top, w: f.w, h: (f.h - top).max(0.0) }
    }
    /// `TabView`'s content, downcast: the page of tab `id`.
    pub fn page<T: View>(&self, id: &str) -> Option<&T> {
        let i = self.tabs.ids().iter().position(|t| t == id)?;
        self.pages[i].as_any().downcast_ref::<T>()
    }
}

impl View for TabView {
    crate::view_base!();
    fn children(&self) -> &[Box<dyn View>] {
        &self.pages
    }
    fn children_mut(&mut self) -> &mut [Box<dyn View>] {
        &mut self.pages
    }
    fn opaque_backdrop(&self) -> bool {
        true
    }
    fn accepts_first_responder(&self) -> bool {
        true
    }
    fn intrinsic(&self, ui: &Ui) -> (f32, f32) {
        let (w, h) = self.pages.iter().map(|p| p.intrinsic(ui)).fold((0.0f32, 0.0f32), |a, b| (a.0.max(b.0), a.1.max(b.1)));
        let w = if w > 0.0 { w + 2.0 * SPACE_M } else { 0.0 };
        let h = if h > 0.0 { h + SEGMENT_H_PT + 2.0 * SPACE_M } else { 0.0 };
        (w, h)
    }
    fn layout(&mut self, ui: &Ui, bounds: Rect) {
        self.set_frame(bounds);
        self.last_bar = Some(self.bar(ui));
        let c = self.card();
        let top = SEGMENT_H_PT / 2.0 + SPACE_M;
        let inner = Rect { x: c.x + SPACE_M, y: c.y + top, w: (c.w - 2.0 * SPACE_M).max(0.0), h: (c.h - top - SPACE_M).max(0.0) };
        for p in self.pages.iter_mut() {
            p.layout(ui, inner);
        }
    }
    fn draw(&mut self, s: &mut dyn Canvas, ui: &Ui) {
        fill_card(s, ui, self.card());
        let labels: Vec<&str> = self.labels.iter().map(String::as_str).collect();
        let bar = self.bar(ui);
        // the tab row is published once, by `ax_node`: the drawn control records nothing
        crate::widgets::segmented_unrecorded(s, ui, bar, &labels, self.selected_index().unwrap_or(0), self.base.enabled);
        if self.base.focused {
            focus_ring(s, ui, bar, bar.h / 2.0);
        }
    }
    fn handle(&mut self, ev: &ViewEvent) -> bool {
        if !self.base.enabled {
            return false;
        }
        match ev {
            ViewEvent::Pointer(Event::Up(x, y) | Event::Down(x, y)) => {
                // the bar depends on text widths, so the hit uses the one the last layout measured
                let Some(bar) = self.last_bar else { return false };
                if !bar.contains(*x, *y) {
                    return false;
                }
                if matches!(ev, ViewEvent::Pointer(Event::Up(..))) {
                    if let Some(i) = segment_hit(bar, self.labels.len(), *x, *y) {
                        self.pick(i);
                    }
                }
                true
            }
            ViewEvent::Key(KeyInput::Code(c @ (37 | 39))) => {
                let n = self.labels.len();
                let cur = self.selected_index().unwrap_or(0);
                let next = if *c == 37 { cur.saturating_sub(1) } else { (cur + 1).min(n.saturating_sub(1)) };
                self.pick(next);
                true
            }
            _ => false,
        }
    }
    fn ax_node(&self) -> Option<Node> {
        let value = self.selected_index().and_then(|i| self.labels.get(i)).map(String::as_str).unwrap_or("");
        node(self, Role::SegmentedControl, &self.labels.join(", "), value)
    }
}
