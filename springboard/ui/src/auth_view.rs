//! Authentication view model (task theme-03): the state of a standard biometric icon in a custom authentication view.
//! Pure model: no drawing and no sensor. A caller feeds it events from an authentication context; the view answers
//! its state, the icon key for the device's biometric kind, and its size in points.

/// The state the icon shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ViewState {
    Idle,
    Evaluating,
    Succeeded,
    Failed,
    LockedOut,
}

/// An event from the authentication context that moves the icon.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    Start,
    Succeeded,
    Failed,
    LockedOut,
    Reset,
}

/// The biometric method the device has; it picks the idle icon.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BiometryKind {
    None,
    Touch,
    Face,
    Optic,
}

/// The size of the icon as a control size.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ControlSize {
    Mini,
    Small,
    Regular,
    Large,
    ExtraLarge,
}

impl ControlSize {
    /// Edge length in points. These steps are BifrOSt's own: the official page gives no numbers.
    pub fn points(self) -> f32 {
        match self {
            ControlSize::Mini => 16.0,
            ControlSize::Small => 20.0,
            ControlSize::Regular => 24.0,
            ControlSize::Large => 32.0,
            ControlSize::ExtraLarge => 40.0,
        }
    }

    /// Edge length in pixels at `scale` pixels per point.
    pub fn pixels(self, scale: f32) -> f32 {
        self.points() * scale
    }
}

/// The next state after `event`. Start moves to Evaluating, except that a lockout stays locked until a reset.
/// Outcome events only apply while an evaluation runs; Reset returns to Idle from any state.
pub fn next(state: ViewState, event: Event) -> ViewState {
    use ViewState::*;
    match (state, event) {
        (_, Event::Reset) => Idle,
        (LockedOut, Event::Start) => LockedOut,
        (_, Event::Start) => Evaluating,
        (Evaluating, Event::Succeeded) => Succeeded,
        (Evaluating, Event::Failed) => Failed,
        (Evaluating, Event::LockedOut) => LockedOut,
        (other, _) => other,
    }
}

/// The icon key for `state`. Outcome states use the same key for every biometric kind.
pub fn icon(state: ViewState, kind: BiometryKind) -> &'static str {
    match state {
        ViewState::Succeeded => "checkmark.circle",
        ViewState::Failed => "xmark.circle",
        ViewState::LockedOut => "lock.fill",
        ViewState::Idle | ViewState::Evaluating => match kind {
            BiometryKind::Touch => "fingerprint",
            BiometryKind::Face => "face",
            BiometryKind::Optic => "iris",
            BiometryKind::None => "lock",
        },
    }
}

/// An authentication icon with its state, kind and size.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AuthView {
    kind: BiometryKind,
    size: ControlSize,
    state: ViewState,
}

impl AuthView {
    /// An idle icon for `kind` at `size`.
    pub fn new(kind: BiometryKind, size: ControlSize) -> AuthView {
        AuthView { kind, size, state: ViewState::Idle }
    }

    /// Applies one event from the authentication context.
    pub fn send(&mut self, event: Event) {
        self.state = next(self.state, event);
    }

    /// The state the icon shows now.
    pub fn state(&self) -> ViewState {
        self.state
    }

    /// The icon key for the current state.
    pub fn icon(&self) -> &'static str {
        icon(self.state, self.kind)
    }

    /// The edge length in points.
    pub fn points(&self) -> f32 {
        self.size.points()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn start_then_success_follows_the_evaluation() {
        let s = next(ViewState::Idle, Event::Start);
        assert_eq!(s, ViewState::Evaluating);
        assert_eq!(next(s, Event::Succeeded), ViewState::Succeeded);
    }

    #[test]
    fn outcome_events_outside_an_evaluation_are_ignored() {
        assert_eq!(next(ViewState::Idle, Event::Failed), ViewState::Idle);
        assert_eq!(next(ViewState::Succeeded, Event::LockedOut), ViewState::Succeeded);
    }

    #[test]
    fn lockout_stays_until_reset() {
        assert_eq!(next(ViewState::LockedOut, Event::Start), ViewState::LockedOut);
        assert_eq!(next(ViewState::LockedOut, Event::Reset), ViewState::Idle);
        assert_eq!(next(ViewState::Failed, Event::Start), ViewState::Evaluating);
    }

    #[test]
    fn icon_depends_on_kind_only_while_idle_or_evaluating() {
        assert_eq!(icon(ViewState::Idle, BiometryKind::Face), "face");
        assert_eq!(icon(ViewState::Evaluating, BiometryKind::Touch), "fingerprint");
        assert_eq!(icon(ViewState::Evaluating, BiometryKind::None), "lock");
        assert_eq!(icon(ViewState::Succeeded, BiometryKind::Face), "checkmark.circle");
        assert_eq!(icon(ViewState::Failed, BiometryKind::Optic), "xmark.circle");
        assert_eq!(icon(ViewState::LockedOut, BiometryKind::Touch), "lock.fill");
    }

    #[test]
    fn control_size_maps_to_points_and_pixels() {
        assert_eq!(ControlSize::Regular.points(), 24.0);
        assert_eq!(ControlSize::Regular.pixels(1.5), 36.0);
        assert_eq!(ControlSize::Mini.pixels(2.0), 32.0);
        assert!(ControlSize::Large.points() > ControlSize::Small.points());
    }

    #[test]
    fn view_sequence_reports_state_icon_and_size() {
        let mut v = AuthView::new(BiometryKind::Touch, ControlSize::Large);
        assert_eq!(v.icon(), "fingerprint");
        v.send(Event::Start);
        v.send(Event::Failed);
        assert_eq!(v.state(), ViewState::Failed);
        assert_eq!(v.icon(), "xmark.circle");
        assert_eq!(v.points(), 32.0);
    }
}
