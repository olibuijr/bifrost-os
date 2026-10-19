//! Pointer events for the external apps on the desktop: the scroll wheel and the pointer moving over a window.
//!
//! An app says which kinds it takes (`ToShell::PointerInterest`, stored as `AppHost::pointer_interest`). The wheel then
//! reaches it as `ToApp::Pointer(Wheel)` at the position in its window, and the drag the Wayland layer also emulates for
//! the wheel is dropped for that app (`AppHost::swallow_wheel_drag`), so it does not scroll twice. The hover reaches it
//! as `Hover` while the pointer is over its window content and `HoverExit` when the pointer goes elsewhere. An app that
//! asked for nothing gets what it always got: the wheel as a drag, no hover. Windows are found as the desk finds a
//! click (`Desk::pick`, content hits only); positions are relative to the window content, as for touches.
use bifrost_ui::pointer_event::{from_gesture, PointerEvent};
use bifrost_ui::wheel::Phase;

use super::desk::Hit;
use super::{AppId, Shell};

impl Shell {
    /// The external app whose window content is at screen point (`x`, `y`): its slot and the point relative to the
    /// content. `None` over the desktop, a title bar, an edge, another screen or a window that is not an external app.
    fn ext_content_at(&self, x: f32, y: f32) -> Option<(u8, f32, f32)> {
        if !self.desk_active() || self.is_locked() {
            return None;
        }
        let desk = self.desk.as_ref()?;
        let m = self.desk_metrics();
        let (i, hit) = desk.pick(&m, x, y)?;
        if hit != Hit::Content {
            return None;
        }
        let win = desk.wins.get(i)?;
        let AppId::External(slot) = win.app else { return None };
        let body = win.body(&m);
        Some((slot, x - body.x as f32, y - body.y as f32))
    }

    /// The wheel moved `dx`, `dy` pixels of content at screen point (`x`, `y`) at `t` ms. True when an app that asked for
    /// the wheel took it; the drag emulated for it is then dropped for that app.
    pub fn pointer_wheel(&mut self, x: f32, y: f32, dx: f32, dy: f32, t: f64) -> bool {
        let Some((slot, lx, ly)) = self.ext_content_at(x, y) else { return false };
        let Some(Some(host)) = self.ext.get_mut(slot as usize) else { return false };
        let event = PointerEvent::Wheel { x: lx, y: ly, dx, dy, precise: false, phase: Phase::None, momentum: Phase::None };
        if !host.pointer(event) {
            return false;
        }
        host.swallow_wheel_drag(t);
        true
    }

    /// The pointer is at screen point (`x`, `y`) with no button down. The app under it that asked for hover gets
    /// `Hover`; the app it was over before gets `HoverExit` when that is another one or none.
    pub fn pointer_hover(&mut self, x: f32, y: f32) {
        let Some((slot, lx, ly)) = self.ext_content_at(x, y) else {
            self.pointer_leave();
            return;
        };
        if self.pointer_over != Some(slot) {
            self.pointer_leave();
        }
        if let Some(Some(host)) = self.ext.get_mut(slot as usize) {
            if host.pointer(PointerEvent::Hover { x: lx, y: ly }) {
                self.pointer_over = Some(slot);
            }
        }
    }

    /// A touchscreen contact of the app in front (`slot`) at (`x`, `y`), in the app's surface pixels. Two fingers run the
    /// pinch and the rotation recognizers together (`ui::trackpad_gesture`): the app that asked for gestures gets
    /// `Magnify` and `Rotate` events at the midpoint of the fingers, and the lift of either finger ends them. A third
    /// finger is ignored.
    pub(super) fn pinch_contact(&mut self, slot: u8, id: i32, phase: crate::bare::input::TouchPhase, x: f32, y: f32) {
        use crate::bare::input::TouchPhase as P;
        let mid = |a: (f32, f32), b: (f32, f32)| ((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0);
        let mut events = Vec::new();
        match phase {
            P::Down => {
                if self.pinch_fingers.len() < 2 && !self.pinch_fingers.iter().any(|(i, _)| *i == id) {
                    self.pinch_fingers.push((id, (x, y)));
                }
            }
            P::Move => {
                if let Some(finger) = self.pinch_fingers.iter_mut().find(|(i, _)| *i == id) {
                    finger.1 = (x, y);
                }
                if self.pinch_fingers.len() == 2 {
                    let (a, b) = (self.pinch_fingers[0].1, self.pinch_fingers[1].1);
                    let at = mid(a, b);
                    events = self.pinch.frame(a, b).iter().map(|e| from_gesture(e, at)).collect();
                }
            }
            P::Up | P::Cancel => {
                let pair = self.pinch_fingers.len() == 2 && self.pinch_fingers.iter().any(|(i, _)| *i == id);
                let at = if pair { mid(self.pinch_fingers[0].1, self.pinch_fingers[1].1) } else { (x, y) };
                self.pinch_fingers.retain(|(i, _)| *i != id);
                if pair {
                    events = self.pinch.lift().iter().map(|e| from_gesture(e, at)).collect();
                }
            }
        }
        if events.is_empty() {
            return;
        }
        if let Some(Some(host)) = self.ext.get_mut(slot as usize) {
            for event in events {
                host.pointer(event);
            }
        }
    }

    /// The pointer left the content of the app it was over: that app gets `HoverExit`.
    fn pointer_leave(&mut self) {
        let Some(slot) = self.pointer_over.take() else { return };
        if let Some(Some(host)) = self.ext.get_mut(slot as usize) {
            host.pointer(PointerEvent::HoverExit);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn two_fingers_are_tracked_a_third_is_ignored_and_lifting_frees_a_slot() {
        use crate::bare::input::TouchPhase as P;
        let mut sh = crate::shell::tests::shell();
        sh.pinch_contact(0, 1, P::Down, 10.0, 10.0);
        sh.pinch_contact(0, 2, P::Down, 50.0, 10.0);
        sh.pinch_contact(0, 3, P::Down, 90.0, 10.0);
        assert_eq!(sh.pinch_fingers.len(), 2);
        // a finger that moves is remembered where it went
        sh.pinch_contact(0, 2, P::Move, 80.0, 10.0);
        assert_eq!(sh.pinch_fingers[1], (2, (80.0, 10.0)));
        // the ignored finger lifting changes nothing; a tracked one frees its place
        sh.pinch_contact(0, 3, P::Up, 90.0, 10.0);
        assert_eq!(sh.pinch_fingers.len(), 2);
        sh.pinch_contact(0, 1, P::Up, 10.0, 10.0);
        assert_eq!(sh.pinch_fingers, vec![(2, (80.0, 10.0))]);
        sh.pinch_contact(0, 2, P::Cancel, 80.0, 10.0);
        assert!(sh.pinch_fingers.is_empty());
    }

    #[test]
    fn with_no_desktop_and_no_app_nothing_is_taken() {
        let mut sh = crate::shell::tests::shell();
        assert!(!sh.pointer_wheel(10.0, 10.0, 0.0, -30.0, 0.0));
        sh.pointer_hover(10.0, 10.0);
        assert_eq!(sh.pointer_over, None);
        // leaving with nothing entered is quiet
        sh.pointer_hover(-100.0, -100.0);
        assert_eq!(sh.pointer_over, None);
    }
}
