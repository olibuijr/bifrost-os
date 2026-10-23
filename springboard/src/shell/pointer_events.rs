//! Pointer events for the external apps on the desktop: the scroll wheel and the pointer moving over a window.
//!
//! An app says which kinds it takes (`ToShell::PointerInterest`, stored as `AppHost::pointer_interest`). The wheel then
//! reaches it as `ToApp::Pointer(Wheel)` at the position in its window, and the drag the Wayland layer also emulates for
//! the wheel is dropped for that app (`AppHost::swallow_wheel_drag`), so it does not scroll twice. The hover reaches it
//! as `Hover` while the pointer is over its window content and `HoverExit` when the pointer goes elsewhere. An app that
//! asked for nothing gets what it always got: the wheel as a drag, no hover. Windows are found as the desk finds a
//! click (`Desk::pick`, content hits only); positions are relative to the window content, as for touches.
//!
//! An app's extra windows (`AppId::ExternalWindow`) get the same events tagged with their window id
//! (`WindowInput::Pointer`), in window points as their touches are (`ToApp::WindowTouch`); the main window keeps
//! `ToApp::Pointer` in surface pixels. The emulated drag is then dropped for that window as well.
use bifrost_ui::pointer_event::{from_gesture, PointerEvent};
use bifrost_ui::wheel::Phase;

use super::desk::Hit;
use super::{AppId, Shell};

/// The event as window `window` takes it: the main window (0) in surface pixels as given, an extra window divided by
/// `pt` (pixels per point), in window points as its touches are.
fn for_window(ev: PointerEvent, window: u32, pt: f32) -> PointerEvent {
    if window == 0 { ev } else { ev.to_points(pt) }
}

impl Shell {
    /// The external app whose window content is at screen point (`x`, `y`): its slot, the window (0 for the main
    /// window, else the extra window's id) and the point relative to the content, in pixels. `None` over the desktop, a
    /// title bar, an edge, another screen or a window that is not an external app's.
    fn ext_content_at(&self, x: f32, y: f32) -> Option<(u8, u32, f32, f32)> {
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
        let (slot, window) = match win.app {
            AppId::External(slot) => (slot, 0),
            AppId::ExternalWindow(slot, id) => (slot, u32::from(id)),
            _ => return None,
        };
        let body = win.body(&m);
        Some((slot, window, x - body.x as f32, y - body.y as f32))
    }

    /// Sends `ev` (content pixels) to `window` of the app in `slot`, converted for that window (`for_window`). True
    /// when the app asked for that kind and the window is open.
    fn send_pointer(&mut self, slot: u8, window: u32, ev: PointerEvent) -> bool {
        let pt = self.ui.pt(1.0) as f32;
        let Some(Some(host)) = self.ext.get_mut(slot as usize) else { return false };
        host.pointer_window(window, for_window(ev, window, pt))
    }

    /// The wheel moved `dx`, `dy` pixels of content at screen point (`x`, `y`) at `t` ms. True when an app that asked for
    /// the wheel took it; the drag emulated for it is then dropped for that app. The window under the pointer gets it:
    /// the main window as `ToApp::Pointer`, an extra window as `WindowInput::Pointer` with its id.
    pub fn pointer_wheel(&mut self, x: f32, y: f32, dx: f32, dy: f32, t: f64) -> bool {
        // a system alert on the desktop is screen-modal: the wheel is used up over it and never reaches the app under the veil
        if self.modal_layer.is_some() {
            return true;
        }
        // an open or save panel is modal over the desktop: its list and icon grid take the wheel (`doc_wheel`)
        #[cfg(feature = "desktop-wayland")]
        if self.doc_wheel(x, y, dy, t) {
            return true;
        }
        // an open menu taller than the screen scrolls under the wheel (`desktop_menu_wheel`)
        #[cfg(feature = "desktop-wayland")]
        if self.desktop_menu_wheel(x, y, dy) {
            return true;
        }
        let Some((slot, window, lx, ly)) = self.ext_content_at(x, y) else { return false };
        let event = PointerEvent::Wheel { x: lx, y: ly, dx, dy, precise: false, phase: Phase::None, momentum: Phase::None };
        if !self.send_pointer(slot, window, event) {
            return false;
        }
        if let Some(Some(host)) = self.ext.get_mut(slot as usize) {
            host.swallow_wheel_drag(t);
        }
        true
    }

    /// The pointer is at screen point (`x`, `y`) with no button down. The window under it, of an app that asked for
    /// hover, gets `Hover`; the window it was over before gets `HoverExit` when that is another one or none.
    pub fn pointer_hover(&mut self, x: f32, y: f32) {
        let Some((slot, window, lx, ly)) = self.ext_content_at(x, y) else {
            self.pointer_leave();
            return;
        };
        if self.pointer_over != Some((slot, window)) {
            self.pointer_leave();
        }
        if self.send_pointer(slot, window, PointerEvent::Hover { x: lx, y: ly }) {
            self.pointer_over = Some((slot, window));
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

    /// The pointer left the content of the window it was over: that window gets `HoverExit`.
    fn pointer_leave(&mut self) {
        let Some((slot, window)) = self.pointer_over.take() else { return };
        if let Some(Some(host)) = self.ext.get_mut(slot as usize) {
            host.pointer_window(window, PointerEvent::HoverExit);
        }
    }
}

#[cfg(test)]
mod tests {

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
    fn an_extra_window_gets_its_pointer_events_in_points_and_the_main_window_in_pixels() {
        use super::for_window;
        use bifrost_ui::pointer_event::PointerEvent;
        use bifrost_ui::wheel::Phase;
        let wheel = PointerEvent::Wheel { x: 50.0, y: 100.0, dx: 0.0, dy: -30.0, precise: false, phase: Phase::None, momentum: Phase::None };
        assert_eq!(for_window(wheel, 0, 2.0), wheel, "the main window keeps surface pixels");
        assert_eq!(
            for_window(wheel, 3, 2.0),
            PointerEvent::Wheel { x: 25.0, y: 50.0, dx: 0.0, dy: -15.0, precise: false, phase: Phase::None, momentum: Phase::None },
            "an extra window gets window points, as its touches"
        );
        assert_eq!(for_window(PointerEvent::HoverExit, 3, 2.0), PointerEvent::HoverExit);
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
