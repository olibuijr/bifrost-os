//! Virtual on-screen controller: a software pad laid over a touch surface. The layout is a list of controls, each
//! a button, a stick or the direction pad, placed in unit-square coordinates (0..=1 on each axis) so the same layout
//! fits any screen size. Each finger is tracked by its id. A button is pressed while a finger is on it; a stick or
//! the direction pad reports the finger's offset from the control's centre, scaled by its radius and clamped to the
//! unit circle. A value set by the app (the framework's "set value" and "set position" calls) applies while no finger
//! is on that control.
//!
//! Pure: no touch device and no drawing. The shell feeds touch down, move and up records and reads the state back.
//! Drawing the pad is left to the caller; the layout gives the regions it needs.

use crate::gamepad::Button;

/// The shape of a control, in unit-square coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Region {
    Circle { cx: f32, cy: f32, r: f32 },
    Rect { x: f32, y: f32, w: f32, h: f32 },
}

impl Region {
    /// Whether a point lies inside the region. Edges count as inside.
    pub fn contains(&self, x: f32, y: f32) -> bool {
        match *self {
            Region::Circle { cx, cy, r } => {
                let dx = x - cx;
                let dy = y - cy;
                dx * dx + dy * dy <= r * r
            }
            Region::Rect { x: rx, y: ry, w, h } => x >= rx && x <= rx + w && y >= ry && y <= ry + h,
        }
    }

    /// The centre of the region: the rest position of a stick.
    pub fn center(&self) -> (f32, f32) {
        match *self {
            Region::Circle { cx, cy, .. } => (cx, cy),
            Region::Rect { x, y, w, h } => (x + w * 0.5, y + h * 0.5),
        }
    }

    /// The distance from the centre that counts as full deflection: the radius of a circle, or half the shorter
    /// side of a rectangle.
    pub fn radius(&self) -> f32 {
        match *self {
            Region::Circle { r, .. } => r,
            Region::Rect { w, h, .. } => w.min(h) * 0.5,
        }
    }
}

/// An element of the virtual pad.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Element {
    Button(Button),
    LeftStick,
    RightStick,
    DirectionPad,
}

/// One placed element.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Control {
    pub element: Element,
    pub region: Region,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Touch {
    id: u32,
    control: usize,
    x: f32,
    y: f32,
}

/// The analogue elements, in the order of `VirtualController::positions`.
fn analog_slot(element: Element) -> Option<usize> {
    match element {
        Element::LeftStick => Some(0),
        Element::RightStick => Some(1),
        Element::DirectionPad => Some(2),
        Element::Button(_) => None,
    }
}

fn button_bit(button: Button) -> u32 {
    1u32 << (button as u32)
}

/// A software controller: a fixed layout and the touches that currently press it.
#[derive(Clone, Debug, PartialEq)]
pub struct VirtualController {
    controls: Vec<Control>,
    touches: Vec<Touch>,
    buttons: u32,
    positions: [Option<(f32, f32)>; 3],
}

impl VirtualController {
    /// A pad with the given layout. Where two controls overlap, the one that comes first in the list wins.
    pub fn new(controls: Vec<Control>) -> Self {
        VirtualController { controls, touches: Vec::new(), buttons: 0, positions: [None; 3] }
    }

    /// The index of the control under a point, or `None` when the point misses every control.
    pub fn hit(&self, x: f32, y: f32) -> Option<usize> {
        self.controls.iter().position(|c| c.region.contains(x, y))
    }

    /// Starts a touch with the given id. Returns the element it landed on, or `None` when it missed.
    /// A touch id that is already down is moved to the new point.
    pub fn touch_down(&mut self, id: u32, x: f32, y: f32) -> Option<Element> {
        let control = self.hit(x, y)?;
        self.touches.retain(|t| t.id != id);
        self.touches.push(Touch { id, control, x, y });
        Some(self.controls[control].element)
    }

    /// Moves a touch. Returns `false` when no touch has that id. A stick keeps following the finger outside its
    /// region, and its value clamps at full deflection.
    pub fn touch_move(&mut self, id: u32, x: f32, y: f32) -> bool {
        match self.touches.iter_mut().find(|t| t.id == id) {
            Some(t) => {
                t.x = x;
                t.y = y;
                true
            }
            None => false,
        }
    }

    /// Ends a touch. Returns `false` when no touch has that id.
    pub fn touch_up(&mut self, id: u32) -> bool {
        let before = self.touches.len();
        self.touches.retain(|t| t.id != id);
        self.touches.len() != before
    }

    /// Sets a button from the app, as the framework's button value call does. Applies while no finger is on it.
    pub fn set_value(&mut self, button: Button, pressed: bool) {
        if pressed {
            self.buttons |= button_bit(button);
        } else {
            self.buttons &= !button_bit(button);
        }
    }

    /// Sets a stick or the direction pad from the app. Each component is clamped to -1..=1. Has no effect on a
    /// button. Applies while no finger is on that element.
    pub fn set_position(&mut self, element: Element, x: f32, y: f32) {
        if let Some(slot) = analog_slot(element) {
            self.positions[slot] = Some((x.clamp(-1.0, 1.0), y.clamp(-1.0, 1.0)));
        }
    }

    /// Whether a button is pressed by a finger or by the app.
    pub fn is_pressed(&self, button: Button) -> bool {
        if self.buttons & button_bit(button) != 0 {
            return true;
        }
        self.touches.iter().any(|t| self.controls[t.control].element == Element::Button(button))
    }

    /// The position of a stick or the direction pad as (x, y), each -1..=1. Zero at rest. Elements that are not
    /// analogue report (0, 0).
    pub fn position(&self, element: Element) -> (f32, f32) {
        let slot = match analog_slot(element) {
            Some(slot) => slot,
            None => return (0.0, 0.0),
        };
        if let Some(t) = self.touches.iter().find(|t| self.controls[t.control].element == element) {
            let region = self.controls[t.control].region;
            let (cx, cy) = region.center();
            let r = region.radius();
            if r <= 0.0 {
                return (0.0, 0.0);
            }
            let dx = (t.x - cx) / r;
            let dy = (t.y - cy) / r;
            let mag = (dx * dx + dy * dy).sqrt();
            let scale = if mag > 1.0 { 1.0 / mag } else { 1.0 };
            return (dx * scale, dy * scale);
        }
        self.positions[slot].unwrap_or((0.0, 0.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn near(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-5
    }

    /// South button at (0.8, 0.5) radius 0.1; left stick centred at (0.2, 0.5) radius 0.2; direction pad a
    /// rectangle 0.2 square at (0.05, 0.05), so its centre is (0.15, 0.15) and its radius 0.1.
    fn pad() -> VirtualController {
        VirtualController::new(vec![
            Control { element: Element::Button(Button::South), region: Region::Circle { cx: 0.8, cy: 0.5, r: 0.1 } },
            Control { element: Element::LeftStick, region: Region::Circle { cx: 0.2, cy: 0.5, r: 0.2 } },
            Control {
                element: Element::DirectionPad,
                region: Region::Rect { x: 0.05, y: 0.05, w: 0.2, h: 0.2 },
            },
        ])
    }

    #[test]
    fn button_is_hit_inside_its_circle_and_missed_outside() {
        let mut p = pad();
        // (0.85, 0.5): dx 0.05, dx^2 0.0025 <= r^2 0.01, inside.
        assert_eq!(p.touch_down(1, 0.85, 0.5), Some(Element::Button(Button::South)));
        assert!(p.is_pressed(Button::South));
        assert!(p.touch_up(1));
        assert!(!p.is_pressed(Button::South));
        // (0.95, 0.5): dx 0.15, dx^2 0.0225 > 0.01, outside.
        assert_eq!(p.touch_down(2, 0.95, 0.5), None);
        assert!(!p.is_pressed(Button::South));
    }

    #[test]
    fn stick_reports_offset_from_centre_over_radius() {
        let mut p = pad();
        // Touch at (0.3, 0.5): offset (0.1, 0) over radius 0.2 gives (0.5, 0).
        assert_eq!(p.touch_down(1, 0.3, 0.5), Some(Element::LeftStick));
        let (x, y) = p.position(Element::LeftStick);
        assert!(near(x, 0.5) && near(y, 0.0), "got {x}, {y}");
    }

    #[test]
    fn stick_clamps_to_the_unit_circle_and_returns_to_rest() {
        let mut p = pad();
        p.touch_down(1, 0.3, 0.5);
        // Moved to (0.5, 0.5): offset (0.3, 0) over 0.2 gives 1.5, clamped to (1, 0).
        assert!(p.touch_move(1, 0.5, 0.5));
        let (x, y) = p.position(Element::LeftStick);
        assert!(near(x, 1.0) && near(y, 0.0), "got {x}, {y}");
        assert!(p.touch_up(1));
        assert_eq!(p.position(Element::LeftStick), (0.0, 0.0));
    }

    #[test]
    fn button_stays_pressed_until_its_last_finger_lifts() {
        let mut p = pad();
        p.touch_down(1, 0.8, 0.5);
        p.touch_down(2, 0.8, 0.55);
        assert!(p.touch_up(1));
        assert!(p.is_pressed(Button::South));
        assert!(p.touch_up(2));
        assert!(!p.is_pressed(Button::South));
    }

    #[test]
    fn app_values_apply_at_rest_and_clamp() {
        let mut p = pad();
        p.set_value(Button::South, true);
        assert!(p.is_pressed(Button::South));
        p.set_value(Button::South, false);
        assert!(!p.is_pressed(Button::South));

        // Direction pad: 2.0 clamps to 1.0, -0.5 stays.
        p.set_position(Element::DirectionPad, 2.0, -0.5);
        let (x, y) = p.position(Element::DirectionPad);
        assert!(near(x, 1.0) && near(y, -0.5));
        // A finger on the direction pad (centre (0.15, 0.15), radius 0.1) overrides the app value:
        // touch at (0.15, 0.05) gives offset (0, -1).
        p.touch_down(7, 0.15, 0.05);
        let (x, y) = p.position(Element::DirectionPad);
        assert!(near(x, 0.0) && near(y, -1.0));
    }

    #[test]
    fn unknown_touch_ids_are_reported() {
        let mut p = pad();
        assert!(!p.touch_move(9, 0.5, 0.5));
        assert!(!p.touch_up(9));
        assert_eq!(p.position(Element::Button(Button::South)), (0.0, 0.0));
    }
}
