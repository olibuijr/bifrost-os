//! Authentication icon view (the reference platform's authentication view): a round control that shows the device's
//! biometric state and sends `action` when the user taps it. The state moves only when the app feeds it events with
//! [`LocalAuthenticationView::send`], which wraps the pure model in `bifrost_ui::auth_view`. The icon is a disc whose
//! colour follows the state; the accessibility label names the biometric kind.
use super::controls::{focus_ring, is_activate, node};
use super::{View, ViewBase, ViewEvent};
use crate::kit::ui::{CAPSULE_BG, CONTROL_PRESSED, LABEL_2, SYSTEM_GREEN, TINT};
use crate::kit::{Event, Ui};
use bifrost_render::canvas::{Canvas, RectF};
use bifrost_ui::auth_view::{AuthView, BiometryKind, ControlSize, Event as AuthEvent, ViewState};
use bifrost_ui::{Node, Role};

/// The accessibility label for a biometric kind (names only; no platform product names).
pub fn kind_label(kind: BiometryKind) -> &'static str {
    match kind {
        BiometryKind::Touch => "Fingerprint",
        BiometryKind::Face => "Face",
        BiometryKind::Optic => "Iris",
        BiometryKind::None => "Passcode",
    }
}

/// A round authentication control that sends `action` on tap while it is idle or after a failure.
pub struct LocalAuthenticationView {
    base: ViewBase,
    model: AuthView,
    action: String,
    pressed: bool,
}

impl LocalAuthenticationView {
    /// An idle control for `kind` at `size`; a tap sends `action`.
    pub fn new(kind: BiometryKind, size: ControlSize, action: &str) -> Self {
        LocalAuthenticationView { base: ViewBase::default(), model: AuthView::new(kind, size), action: action.into(), pressed: false }
    }

    /// Applies one event from the authentication context and redraws.
    pub fn send(&mut self, event: AuthEvent) {
        self.model.send(event);
        self.base.dirty = true;
    }

    /// The state the control shows.
    pub fn state(&self) -> ViewState {
        self.model.state()
    }

    /// The icon key the control would draw (see `bifrost_ui::auth_view::icon`).
    pub fn icon(&self) -> &'static str {
        self.model.icon()
    }

    fn tappable(&self) -> bool {
        matches!(self.model.state(), ViewState::Idle | ViewState::Failed)
    }

    fn fire(&mut self) {
        if self.tappable() {
            let a = self.action.clone();
            self.base.send_action(&a);
        }
    }
}

impl View for LocalAuthenticationView {
    crate::view_base!();

    fn intrinsic(&self, _ui: &Ui) -> (f32, f32) {
        (self.model.points(), self.model.points())
    }

    fn accepts_first_responder(&self) -> bool {
        true
    }

    fn draw(&mut self, s: &mut dyn Canvas, ui: &Ui) {
        let f = self.base.frame;
        let fill = match self.model.state() {
            ViewState::Succeeded => SYSTEM_GREEN,
            ViewState::Evaluating => TINT,
            ViewState::Failed | ViewState::LockedOut => LABEL_2.now(),
            ViewState::Idle => {
                if self.pressed {
                    CONTROL_PRESSED.now()
                } else {
                    CAPSULE_BG.now()
                }
            }
        };
        let d = ui.pt(f.w.min(f.h));
        let x = ui.pt(f.x) + (ui.pt(f.w) - d) / 2.0;
        let y = ui.pt(f.y) + (ui.pt(f.h) - d) / 2.0;
        s.fill_rrect(RectF::new(x, y, d, d), d / 2.0, fill);
        if self.base.focused {
            focus_ring(s, ui, f, f.w.min(f.h) / 2.0);
        }
    }

    fn handle(&mut self, ev: &ViewEvent) -> bool {
        if !self.base.enabled {
            return false;
        }
        let was = self.pressed;
        let handled = match ev {
            ViewEvent::Pointer(Event::Down(x, y)) => {
                self.pressed = self.tappable() && self.base.frame.contains(*x, *y);
                true
            }
            ViewEvent::Pointer(Event::Move(x, y)) => {
                self.pressed = self.pressed && self.base.frame.contains(*x, *y);
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
        node(self, Role::Button, kind_label(self.model.kind()), "")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tap_while_idle_sends_the_action_and_a_busy_control_does_not() {
        let mut v = LocalAuthenticationView::new(BiometryKind::Touch, ControlSize::Regular, "auth.evaluate");
        v.fire();
        assert_eq!(v.base.pending, vec!["auth.evaluate".to_string()]);
        v.base.pending.clear();
        v.send(AuthEvent::Start);
        assert_eq!(v.state(), ViewState::Evaluating);
        v.fire();
        assert!(v.base.pending.is_empty());
    }

    #[test]
    fn a_failed_attempt_can_be_tried_again() {
        let mut v = LocalAuthenticationView::new(BiometryKind::Face, ControlSize::Regular, "auth.evaluate");
        v.send(AuthEvent::Start);
        v.send(AuthEvent::Failed);
        v.fire();
        assert_eq!(v.base.pending, vec!["auth.evaluate".to_string()]);
    }

    #[test]
    fn a_lockout_stays_put_until_reset() {
        let mut v = LocalAuthenticationView::new(BiometryKind::Optic, ControlSize::Large, "auth.evaluate");
        // A lockout is an outcome of an evaluation, so the evaluation starts first (the model ignores it while idle).
        v.send(AuthEvent::Start);
        v.send(AuthEvent::LockedOut);
        assert_eq!(v.state(), ViewState::LockedOut);
        assert!(!v.tappable());
        v.fire();
        assert!(v.base.pending.is_empty());
        v.send(AuthEvent::Reset);
        assert_eq!(v.state(), ViewState::Idle);
        assert_eq!(v.icon(), "iris");
    }

    #[test]
    fn the_size_and_accessibility_label_follow_the_kind() {
        let v = LocalAuthenticationView::new(BiometryKind::Face, ControlSize::Large, "x");
        assert_eq!(v.model.points(), 32.0);
        assert_eq!(kind_label(BiometryKind::None), "Passcode");
    }
}
