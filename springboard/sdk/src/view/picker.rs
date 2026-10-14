//! Spinning-wheel picker (`UIPickerView`) over `bifrost_ui::picker_wheel`: one wheel per component, drawn as rows around
//! a centre band. A drag scrolls a wheel, a flick carries it further and the release settles it on a row; a tap settles
//! it on the tapped row. The selection changes when a wheel settles: the view sends `action` and calls the selection
//! handler (`pickerView(_:didSelectRow:inComponent:)`). Programmatic changes do not (`select_row`), as in the reference
//! platform. Arrow keys step the focused component and Left and Right move the focus between components.
use super::controls::{draw_text, focus_ring, node};
use super::scroll::VelocityTracker;
use super::{View, ViewBase, ViewEvent};
use crate::kit::ui::{Weight, FIELD_BG, LABEL, LABEL_2, LABEL_3};
use crate::kit::{Event, Ui};
use crate::proto::KeyInput;
use bifrost_render::canvas::{Canvas, RectF};
use bifrost_render::ui::MAC_BODY_PT;
use bifrost_ui::picker_wheel::{PickerDataSource, PickerWheel};
use bifrost_ui::{Node, Rect, Role};

/// Height of one row, in points.
pub const WHEEL_ROW_PT: f32 = 36.0;
/// Rows drawn above and below the centre row.
const VISIBLE_RADIUS: usize = 2;
/// Corner radius of the selection band, in points.
const BAND_RADIUS_PT: f32 = 4.0;
/// A press that travels less than this many rows is a tap on a row.
const TAP_ROWS: f32 = 0.2;

/// Key codes of the arrow keys (evdev).
const KEY_LEFT: u32 = 105;
const KEY_RIGHT: u32 = 106;
const KEY_UP: u32 = 103;
const KEY_DOWN: u32 = 108;

/// A drag in progress: the wheel under the finger, the last finger height and how far the finger has travelled.
struct Drag {
    column: usize,
    last_y: f32,
    travelled: f32,
}

/// A picker view (`UIPickerView`). Build it with a data source, then read the selection with [`PickerView::selected_row`].
pub struct PickerView {
    base: ViewBase,
    wheel: PickerWheel,
    source: Box<dyn PickerDataSource>,
    action: String,
    handler: Option<Box<dyn FnMut(usize, usize)>>,
    focus: usize,
    drag: Option<Drag>,
    velocity: VelocityTracker,
    now: f64,
}

impl PickerView {
    /// A picker over `source` that sends `action` when the user changes a selection.
    pub fn new(source: Box<dyn PickerDataSource>, action: &str) -> Self {
        let wheel = PickerWheel::new(source.as_ref());
        PickerView {
            base: ViewBase::default(),
            wheel,
            source,
            action: action.into(),
            handler: None,
            focus: 0,
            drag: None,
            velocity: VelocityTracker::default(),
            now: 0.0,
        }
    }

    /// `pickerView(_:didSelectRow:inComponent:)`: called with `(component, row)` after each user selection.
    pub fn on_select(mut self, handler: impl FnMut(usize, usize) + 'static) -> Self {
        self.handler = Some(Box::new(handler));
        self
    }

    pub fn wheel(&self) -> &PickerWheel {
        &self.wheel
    }

    /// `selectedRow(inComponent:)`
    pub fn selected_row(&self, component: usize) -> Option<usize> {
        self.wheel.selected_row(component)
    }

    /// `selectRow(_:inComponent:animated: false)`: true when the selection changed. Neither the action nor the handler runs.
    pub fn select_row(&mut self, component: usize, row: usize) -> bool {
        self.base.dirty = true;
        self.wheel.select_row(component, row)
    }

    /// `reloadAllComponents()`: counts and titles are read again from the data source.
    pub fn reload_all(&mut self) {
        self.wheel.reload_all(self.source.as_ref());
        self.base.dirty = true;
    }

    /// `reloadComponent(_:)`
    pub fn reload_component(&mut self, component: usize) {
        self.wheel.reload_component(self.source.as_ref(), component);
        self.base.dirty = true;
    }

    /// The rectangle of each component, in window points.
    fn column_rects(&self) -> Vec<Rect> {
        let f = self.base.frame;
        let mut x = f.x;
        let mut rects = Vec::new();
        for w in self.wheel.column_widths(f.w) {
            rects.push(Rect { x, y: f.y, w, h: f.h });
            x += w;
        }
        rects
    }

    fn column_at(&self, x: f32) -> Option<usize> {
        self.column_rects().iter().position(|r| x >= r.x && x < r.x + r.w)
    }

    /// The window-point height of the centre row's middle.
    fn centre_y(&self) -> f32 {
        self.base.frame.y + self.base.frame.h / 2.0
    }

    fn position_pt(&self, component: usize) -> f32 {
        self.wheel.column(component).map_or(0.0, |c| c.position() * WHEEL_ROW_PT)
    }

    /// A user selection changed: send the action, then call the handler.
    fn selection_changed(&mut self, component: usize, row: usize) {
        self.base.dirty = true;
        let action = self.action.clone();
        self.base.send_action(&action);
        if let Some(handler) = self.handler.as_mut() {
            handler(component, row);
        }
    }

    /// Moves the focused component's selection by `delta` rows.
    fn step(&mut self, delta: i32) {
        let focus = self.focus;
        let changed = self.wheel.column_mut(focus).is_some_and(|col| col.step(delta));
        if changed {
            if let Some(row) = self.wheel.selected_row(focus) {
                self.selection_changed(focus, row);
            }
        }
    }

    /// The finger lifted (`Some(y)`) or the touch was cancelled (`None`): settle the wheel and report the new row.
    fn finish(&mut self, up_y: Option<f32>) -> bool {
        let Some(drag) = self.drag.take() else {
            return false;
        };
        let now = self.now;
        let velocity = match up_y {
            Some(_) => self.velocity.velocity(now * 1000.0) / WHEEL_ROW_PT,
            None => 0.0,
        };
        self.velocity.clear();
        let tap_row = match up_y {
            Some(y) if drag.travelled < TAP_ROWS => {
                let cy = self.centre_y();
                self.wheel.column(drag.column).map(|c| (c.position() + (y - cy) / WHEEL_ROW_PT).round().max(0.0) as usize)
            }
            _ => None,
        };
        let changed = match tap_row {
            Some(row) => self.wheel.column_mut(drag.column).and_then(|c| c.settle_to(row, now)),
            None => self.wheel.column_mut(drag.column).and_then(|c| c.release(velocity, now)),
        };
        if let Some(row) = changed {
            self.selection_changed(drag.column, row);
        }
        self.base.dirty = true;
        true
    }

    fn key(&mut self, key: &KeyInput) -> bool {
        let columns = self.wheel.number_of_components();
        match key {
            KeyInput::Code(KEY_UP) => {
                self.step(-1);
                columns > 0
            }
            KeyInput::Code(KEY_DOWN) => {
                self.step(1);
                columns > 0
            }
            KeyInput::Code(KEY_LEFT) => {
                self.focus = self.focus.saturating_sub(1);
                self.base.dirty = true;
                columns > 0
            }
            KeyInput::Code(KEY_RIGHT) => {
                if self.focus + 1 < columns {
                    self.focus += 1;
                }
                self.base.dirty = true;
                columns > 0
            }
            _ => false,
        }
    }
}

impl View for PickerView {
    crate::view_base!();

    fn intrinsic(&self, _ui: &Ui) -> (f32, f32) {
        (0.0, WHEEL_ROW_PT * (2 * VISIBLE_RADIUS + 1) as f32)
    }

    fn accepts_first_responder(&self) -> bool {
        true
    }

    fn tick(&mut self, t: f64) {
        self.now = t;
        if self.wheel.columns().iter().any(|c| c.is_settling()) {
            self.base.dirty = true;
        }
        self.wheel.tick(t);
    }

    fn wants_tick(&self) -> bool {
        self.wheel.columns().iter().any(|c| c.is_settling())
    }

    fn draw(&mut self, s: &mut dyn Canvas, ui: &Ui) {
        let f = self.base.frame;
        let cy = self.centre_y();
        s.fill_rrect(RectF::new(ui.pt(f.x), ui.pt(cy - WHEEL_ROW_PT / 2.0), ui.pt(f.w), ui.pt(WHEEL_ROW_PT)), ui.pt(BAND_RADIUS_PT), FIELD_BG.now());
        for (c, rect) in self.column_rects().into_iter().enumerate() {
            let Some(col) = self.wheel.column(c) else {
                continue;
            };
            s.push_clip(RectF::new(ui.pt(rect.x), ui.pt(rect.y), ui.pt(rect.w), ui.pt(rect.h)), 0.0);
            for row in col.visible_rows(VISIBLE_RADIUS) {
                let offset = row as f32 - col.position();
                let color = match offset.abs() {
                    d if d < 0.5 => LABEL,
                    d if d < 1.5 => LABEL_2,
                    _ => LABEL_3,
                };
                let cell = Rect { x: rect.x, y: cy + offset * WHEEL_ROW_PT - WHEEL_ROW_PT / 2.0, w: rect.w, h: WHEEL_ROW_PT };
                let title = self.source.title(c, row);
                draw_text(s, ui, cell, &title, MAC_BODY_PT, Weight::Regular, color.now(), true);
            }
            s.pop_clip();
        }
        if self.base.focused {
            focus_ring(s, ui, f, BAND_RADIUS_PT);
        }
    }

    fn handle(&mut self, ev: &ViewEvent) -> bool {
        if !self.base.enabled {
            return false;
        }
        match ev {
            ViewEvent::Pointer(Event::Down(x, y)) => {
                let Some(column) = self.column_at(*x) else {
                    return false;
                };
                self.focus = column;
                if let Some(col) = self.wheel.column_mut(column) {
                    col.grab();
                }
                self.velocity.clear();
                self.velocity.push(self.now * 1000.0, self.position_pt(column));
                self.drag = Some(Drag { column, last_y: *y, travelled: 0.0 });
                self.base.dirty = true;
                true
            }
            ViewEvent::Pointer(Event::Move(_, y)) => {
                let Some(drag) = self.drag.as_mut() else {
                    return false;
                };
                let delta = (drag.last_y - *y) / WHEEL_ROW_PT;
                drag.last_y = *y;
                drag.travelled += delta.abs();
                let column = drag.column;
                if let Some(col) = self.wheel.column_mut(column) {
                    col.drag(delta);
                }
                self.velocity.push(self.now * 1000.0, self.position_pt(column));
                self.base.dirty = true;
                true
            }
            ViewEvent::Pointer(Event::Up(_, y)) => self.finish(Some(*y)),
            ViewEvent::Pointer(_) => false,
            ViewEvent::Cancel => self.finish(None),
            ViewEvent::Key(key) => self.key(key),
        }
    }

    fn ax_node(&self) -> Option<Node> {
        let value = (0..self.wheel.number_of_components())
            .filter_map(|c| self.wheel.selected_row(c).map(|r| self.source.title(c, r)))
            .collect::<Vec<_>>()
            .join(", ");
        node(self, Role::List, "Picker", &value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    struct Two;

    impl PickerDataSource for Two {
        fn components(&self) -> usize {
            2
        }
        fn rows(&self, component: usize) -> usize {
            [3, 4][component]
        }
        fn title(&self, component: usize, row: usize) -> String {
            format!("{component}.{row}")
        }
    }

    fn picker() -> PickerView {
        let mut p = PickerView::new(Box::new(Two), "picked");
        p.base.frame = Rect { x: 0.0, y: 0.0, w: 200.0, h: 180.0 };
        p
    }

    #[test]
    fn arrow_keys_step_the_focused_component_and_send_the_action() {
        let mut p = picker();
        assert!(p.handle(&ViewEvent::Key(KeyInput::Code(KEY_DOWN))));
        assert_eq!(p.selected_row(0), Some(1));
        assert_eq!(p.base.pending, vec!["picked".to_string()]);
        assert!(p.handle(&ViewEvent::Key(KeyInput::Code(KEY_RIGHT))));
        p.handle(&ViewEvent::Key(KeyInput::Code(KEY_DOWN)));
        assert_eq!(p.selected_row(1), Some(1));
        assert_eq!(p.selected_row(0), Some(1));
        assert!(!p.handle(&ViewEvent::Key(KeyInput::Code(30))));
    }

    #[test]
    fn programmatic_selection_sends_nothing() {
        let mut p = picker();
        assert!(p.select_row(1, 2));
        assert_eq!(p.selected_row(1), Some(2));
        assert!(p.base.pending.is_empty());
    }

    #[test]
    fn the_handler_hears_each_user_change() {
        let seen = Rc::new(RefCell::new(Vec::new()));
        let log = seen.clone();
        let mut p = picker().on_select(move |c, r| log.borrow_mut().push((c, r)));
        p.handle(&ViewEvent::Key(KeyInput::Code(KEY_DOWN)));
        assert_eq!(*seen.borrow(), vec![(0, 1)]);
    }

    #[test]
    fn a_drag_then_a_release_settles_on_the_row_the_wheel_stops_on() {
        let mut p = picker();
        p.handle(&ViewEvent::Pointer(Event::Down(10.0, 90.0)));
        p.handle(&ViewEvent::Pointer(Event::Move(10.0, 54.0)));
        assert!(p.handle(&ViewEvent::Pointer(Event::Up(10.0, 54.0))));
        assert_eq!(p.selected_row(0), Some(1));
        assert_eq!(p.base.pending, vec!["picked".to_string()]);
    }

    #[test]
    fn a_tap_below_the_centre_row_selects_the_row_under_the_finger() {
        let mut p = picker();
        p.handle(&ViewEvent::Pointer(Event::Down(10.0, 90.0)));
        p.handle(&ViewEvent::Pointer(Event::Up(10.0, 126.0)));
        assert_eq!(p.selected_row(0), Some(1));
    }

    #[test]
    fn a_cancelled_touch_snaps_without_a_flick() {
        let mut p = picker();
        p.handle(&ViewEvent::Pointer(Event::Down(10.0, 90.0)));
        p.handle(&ViewEvent::Pointer(Event::Move(10.0, 46.0)));
        assert!(p.handle(&ViewEvent::Cancel));
        assert_eq!(p.selected_row(0), Some(1));
        assert!(!p.handle(&ViewEvent::Cancel));
    }
}
