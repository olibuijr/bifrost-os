//! Input routing. Backends turn device events into these calls; tests call them directly.

use smithay::{
    backend::input::{
        AbsolutePositionEvent, Axis, AxisSource, ButtonState, Event, InputBackend, InputEvent, KeyState, KeyboardKeyEvent,
        PointerAxisEvent, PointerButtonEvent, PointerMotionEvent,
    },
    input::{
        keyboard::{FilterResult, Keycode},
        pointer::{AxisFrame, ButtonEvent, MotionEvent},
    },
    utils::{Logical, Point, SERIAL_COUNTER},
};

use crate::state::WmState;

impl WmState {
    pub fn process_input_event<I: InputBackend>(&mut self, event: InputEvent<I>) {
        // any input is activity for ext-idle-notify (the shell dims and locks on idle)
        let seat = self.seat.clone();
        self.idle_notifier_state.notify_activity(&seat);
        match event {
            InputEvent::Keyboard { event } => self.key(event.key_code(), event.state(), event.time_msec()),
            InputEvent::PointerMotion { event } => {
                let pos = self.pointer_location() + event.delta();
                self.pointer_motion(pos, event.time_msec());
            }
            InputEvent::PointerMotionAbsolute { event } => {
                let Some(geo) = self.output().and_then(|o| self.space.output_geometry(&o)) else { return };
                let pos = event.position_transformed(geo.size) + geo.loc.to_f64();
                self.pointer_motion(pos, event.time_msec());
            }
            InputEvent::PointerButton { event } => {
                self.pointer_button(event.button_code(), event.state() == ButtonState::Pressed, event.time_msec())
            }
            InputEvent::PointerAxis { event } => {
                let source = event.source();
                let amount = |axis| event.amount(axis).unwrap_or_else(|| event.amount_v120(axis).unwrap_or(0.0) * 15.0 / 120.0);
                let mut frame = AxisFrame::new(event.time_msec()).source(source);
                for axis in [Axis::Horizontal, Axis::Vertical] {
                    let v = amount(axis);
                    if v != 0.0 {
                        frame = frame.value(axis, v);
                        if let Some(d) = event.amount_v120(axis) {
                            frame = frame.v120(axis, d as i32);
                        }
                    } else if source == AxisSource::Finger && event.amount(axis) == Some(0.0) {
                        frame = frame.stop(axis);
                    }
                }
                let pointer = self.seat.get_pointer().expect("pointer");
                pointer.axis(self, frame);
                pointer.frame(self);
            }
            _ => {}
        }
    }

    pub fn pointer_location(&self) -> Point<f64, Logical> {
        self.seat.get_pointer().map(|p| p.current_location()).unwrap_or_default()
    }

    /// Moves the pointer to `pos`, kept on the output.
    pub fn pointer_motion(&mut self, pos: Point<f64, Logical>, time: u32) {
        let pos = match self.output().and_then(|o| self.space.output_geometry(&o)) {
            Some(g) => (
                pos.x.clamp(g.loc.x as f64, (g.loc.x + g.size.w - 1) as f64),
                pos.y.clamp(g.loc.y as f64, (g.loc.y + g.size.h - 1) as f64),
            )
                .into(),
            None => pos,
        };
        let under = self.surface_under(pos);
        let pointer = self.seat.get_pointer().expect("pointer");
        pointer.motion(self, under, &MotionEvent { location: pos, serial: SERIAL_COUNTER.next_serial(), time });
        pointer.frame(self);
    }

    /// A press on a window raises and focuses it (on the desktop it clears focus) before the client sees it.
    pub fn pointer_button(&mut self, button: u32, pressed: bool, time: u32) {
        let pointer = self.seat.get_pointer().expect("pointer");
        let serial = SERIAL_COUNTER.next_serial();
        if pressed && !pointer.is_grabbed() {
            let pos = pointer.current_location();
            let hit = self.wm.at(pos.x as i32, pos.y as i32);
            if hit.is_some() || self.surface_under(pos).is_none() {
                self.focus(hit, serial);
            }
        }
        let state = if pressed { ButtonState::Pressed } else { ButtonState::Released };
        pointer.button(self, &ButtonEvent { button, state, serial, time });
        pointer.frame(self);
    }

    pub fn key(&mut self, code: Keycode, state: KeyState, time: u32) {
        let keyboard = self.seat.get_keyboard().expect("keyboard");
        keyboard.input::<(), _>(self, code, state, SERIAL_COUNTER.next_serial(), time, |_, _, _| FilterResult::Forward);
    }
}
