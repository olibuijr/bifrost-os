//! Translate input protocol commands to Servo input events.
use super::*;

impl Engine {
    pub(super) fn input_command(&mut self, verb: &str, rest: &str) -> bool {
        let arg = |n: usize| rest.split(' ').nth(n).unwrap_or("");
        let tail = |n: usize| rest.splitn(n + 1, ' ').nth(n).unwrap_or("").to_string();
        match verb {
            "mouseleave" => {
                if let Some(view) = self.visible_view() {
                    view.notify_input_event(InputEvent::MouseLeftViewport(servo::MouseLeftViewportEvent::default()));
                }
            }
            "mouse" => {
                if let Some(view) = self.visible_view() {
                    let point = DevicePoint::new(coord(arg(1)), coord(arg(2))).into();
                    view.notify_input_event(InputEvent::MouseMove(servo::MouseMoveEvent::new(point)));
                    let action = match arg(0) {
                        "d" => Some(servo::MouseButtonAction::Down),
                        "u" | "c" => Some(servo::MouseButtonAction::Up),
                        _ => None,
                    };
                    if let Some(action) = action {
                        view.notify_input_event(InputEvent::MouseButton(servo::MouseButtonEvent::new(action, servo::MouseButton::Primary, point)));
                    }
                }
            }
            "touch" => {
                let kind = match arg(0) {
                    "d" => TouchEventType::Down,
                    "m" => TouchEventType::Move,
                    "u" => TouchEventType::Up,
                    _ => TouchEventType::Cancel,
                };
                let finger: i32 = arg(1).parse().unwrap_or(0);
                let (x, y) = (coord(arg(2)), coord(arg(3)));
                if let Some(v) = self.visible_view() {
                    let point = DevicePoint::new(x, y).into();
                    v.notify_input_event(InputEvent::Touch(TouchEvent::new(kind, TouchId(finger), point, TouchPointerType::Touch)));
                }
            }
            // secondary (right) click at a point of the visible view: opens the page's context menu
            "rclick" => {
                if let Some(v) = self.visible_view() {
                    let (px, py) = (coord(arg(0)), coord(arg(1)));
                    let point = || -> servo::WebViewPoint { DevicePoint::new(px, py).into() };
                    v.notify_input_event(InputEvent::MouseMove(servo::MouseMoveEvent::new(point())));
                    for action in [servo::MouseButtonAction::Down, servo::MouseButtonAction::Up] {
                        v.notify_input_event(InputEvent::MouseButton(servo::MouseButtonEvent::new(action, servo::MouseButton::Secondary, point())));
                    }
                }
            }
            "wheel" => {
                let n: Vec<f32> = rest.split(' ').map(coord).collect();
                if let (Some(v), [dx, dy, x, y]) = (self.visible_view(), n.as_slice()) {
                    let delta = servo::WheelDelta { x: *dx as f64, y: *dy as f64, z: 0.0, mode: servo::WheelMode::DeltaPixel };
                    v.notify_input_event(InputEvent::Wheel(servo::WheelEvent::new(delta, DevicePoint::new(*x, *y).into())));
                }
            }
            "text" => {
                if let Some(v) = self.visible_view() {
                    for c in rest.chars() {
                        let key = Key::Character(c.to_string());
                        v.notify_input_event(InputEvent::Keyboard(KeyboardEvent::from_state_and_key(KeyState::Down, key.clone())));
                        v.notify_input_event(InputEvent::Keyboard(KeyboardEvent::from_state_and_key(KeyState::Up, key)));
                    }
                }
            }
            "key" => {
                let (named, code) = match rest {
                    "Enter" => (NamedKey::Enter, Code::Enter),
                    "Backspace" => (NamedKey::Backspace, Code::Backspace),
                    "Tab" => (NamedKey::Tab, Code::Tab),
                    "Escape" => (NamedKey::Escape, Code::Escape),
                    "Delete" => (NamedKey::Delete, Code::Delete),
                    "ArrowLeft" => (NamedKey::ArrowLeft, Code::ArrowLeft),
                    "ArrowRight" => (NamedKey::ArrowRight, Code::ArrowRight),
                    "ArrowUp" => (NamedKey::ArrowUp, Code::ArrowUp),
                    "ArrowDown" => (NamedKey::ArrowDown, Code::ArrowDown),
                    "Home" => (NamedKey::Home, Code::Home),
                    "End" => (NamedKey::End, Code::End),
                    "PageUp" => (NamedKey::PageUp, Code::PageUp),
                    "PageDown" => (NamedKey::PageDown, Code::PageDown),
                    _ => return true,
                };
                if let Some(v) = self.visible_view() {
                    for state in [KeyState::Down, KeyState::Up] {
                        let e = KeyboardEvent::new_without_event(state, Key::Named(named.clone()), code, Location::Standard, Modifiers::empty(), false, false);
                        v.notify_input_event(InputEvent::Keyboard(e));
                    }
                }
            }
            "chord" => {
                let bits: u32 = arg(0).parse().unwrap_or(0);
                let ch = tail(1);
                let mut m = Modifiers::empty();
                for (bit, flag) in [(1, Modifiers::CONTROL), (2, Modifiers::SHIFT), (4, Modifiers::ALT), (8, Modifiers::META)] {
                    if bits & bit != 0 {
                        m |= flag;
                    }
                }
                if let Some(v) = self.visible_view() {
                    for state in [KeyState::Down, KeyState::Up] {
                        let e = KeyboardEvent::new_without_event(state, Key::Character(ch.clone()), Code::Unidentified, Location::Standard, m, false, false);
                        v.notify_input_event(InputEvent::Keyboard(e));
                    }
                }
            }
            _ => return false,
        }
        true
    }
}
