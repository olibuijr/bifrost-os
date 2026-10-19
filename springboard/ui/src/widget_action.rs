//! What a tap on a widget does: open a link, run an app intent, or flip a toggle. Pure model: the action under a
//! point (the topmost hit region wins), and a toggle that shows its new state at once and returns to the old one
//! when the app reports failure. The shell sends the action to the app; nothing here runs an intent.

use crate::control_widget::{toggle_param_value, TOGGLE_PARAM};
use crate::widget_link::Link;

/// A rectangle in widget-local points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    /// True inside the rectangle. The left and top edges are included, the right and bottom edges are not.
    pub fn contains(&self, px: f32, py: f32) -> bool {
        px >= self.x && py >= self.y && px < self.x + self.w && py < self.y + self.h
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    /// Open a widget link in its app.
    OpenLink(Link),
    /// Run the app intent with this identifier.
    RunIntent { id: String },
    /// Set the named toggle to the given state.
    SetToggle { id: String, on: bool },
}

/// One tappable region of a widget and the action it starts.
#[derive(Clone, Debug, PartialEq)]
pub struct Hit {
    pub frame: Rect,
    pub action: Action,
}

/// The action under (x, y). Later regions lie on top, so the last one that contains the point wins.
pub fn hit_test(hits: &[Hit], x: f32, y: f32) -> Option<&Action> {
    hits.iter().rev().find(|h| h.frame.contains(x, y)).map(|h| &h.action)
}

/// A toggle that shows its new state at once. The app's answer then commits it, or puts back the last committed
/// state when the change failed. Taps made while a change is pending are ignored.
#[derive(Clone, Debug, PartialEq)]
pub struct Toggle {
    committed: bool,
    shown: bool,
    pending: bool,
}

impl Toggle {
    pub fn new(on: bool) -> Self {
        Toggle { committed: on, shown: on, pending: false }
    }

    /// The state the widget shows.
    pub fn on(&self) -> bool {
        self.shown
    }

    pub fn is_pending(&self) -> bool {
        self.pending
    }

    /// A tap. Returns the new state to send to the app, or None while an earlier change is still pending.
    pub fn tap(&mut self) -> Option<bool> {
        if self.pending {
            return None;
        }
        let next = !self.shown;
        self.shown = next;
        self.pending = true;
        Some(next)
    }

    /// The app's answer to the last tap. Success commits the shown state. Failure puts back the committed one.
    /// Does nothing when no change is pending.
    pub fn settle(&mut self, ok: bool) {
        if !self.pending {
            return;
        }
        self.pending = false;
        if ok {
            self.committed = self.shown;
        } else {
            self.shown = self.committed;
        }
    }
}

/// The most buttons one widget entry may carry.
pub const MAX_BUTTONS: usize = 3;
/// The longest button id or intent id, in bytes.
pub const MAX_BUTTON_ID: usize = 64;

/// Why a button was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ButtonError {
    /// The button id or the intent id is empty, too long, or has a character other than a letter, a digit, `-`, `_` or `.`.
    BadId(String),
    /// The label is blank.
    NoLabel,
    /// Two buttons of one entry share an id.
    Duplicate(String),
    /// An entry carries more than `MAX_BUTTONS` buttons.
    TooMany,
}

/// One button or toggle on a widget entry: a label, the app intent a tap runs and the parameters it carries. A
/// toggle (`toggle` is `Some`, holding the state the app shows) also sends its new state as the intent parameter
/// `on`, as a Control Center toggle does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ButtonSpec {
    pub id: String,
    pub label: String,
    pub intent: String,
    pub params: Vec<(String, String)>,
    pub toggle: Option<bool>,
}

fn safe_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= MAX_BUTTON_ID && id.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

impl ButtonSpec {
    /// A button that runs `intent`. The ids must be safe (they become part of a request line), the label not blank.
    pub fn button(id: &str, label: &str, intent: &str) -> Result<ButtonSpec, ButtonError> {
        for s in [id, intent] {
            if !safe_id(s) {
                return Err(ButtonError::BadId(s.to_string()));
            }
        }
        if label.trim().is_empty() {
            return Err(ButtonError::NoLabel);
        }
        Ok(ButtonSpec { id: id.to_string(), label: label.trim().to_string(), intent: intent.to_string(), params: Vec::new(), toggle: None })
    }

    /// A toggle that shows `on` and runs `intent` with the new state.
    pub fn toggle(id: &str, label: &str, intent: &str, on: bool) -> Result<ButtonSpec, ButtonError> {
        let mut b = ButtonSpec::button(id, label, intent)?;
        b.toggle = Some(on);
        Ok(b)
    }

    /// Adds a string parameter the intent receives.
    pub fn with_param(mut self, name: &str, value: &str) -> Self {
        self.params.push((name.to_string(), value.to_string()));
        self
    }

    /// What a tap sends: the intent and its parameters, plus `on` with the new state for a toggle (`new_on`).
    pub fn invocation(&self, new_on: Option<bool>) -> Invocation {
        let mut params = self.params.clone();
        if let (Some(_), Some(on)) = (self.toggle, new_on) {
            params.retain(|(k, _)| k != TOGGLE_PARAM);
            params.push((TOGGLE_PARAM.to_string(), toggle_param_value(on).to_string()));
        }
        Invocation { intent: self.intent.clone(), params }
    }
}

/// The intent a tap on a widget button runs, ready for the app's control socket.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Invocation {
    pub intent: String,
    pub params: Vec<(String, String)>,
}

/// Checks the buttons of one entry: at most `MAX_BUTTONS`, no repeated id.
pub fn check_buttons(buttons: &[ButtonSpec]) -> Result<(), ButtonError> {
    if buttons.len() > MAX_BUTTONS {
        return Err(ButtonError::TooMany);
    }
    for (i, b) in buttons.iter().enumerate() {
        if buttons[..i].iter().any(|o| o.id == b.id) {
            return Err(ButtonError::Duplicate(b.id.clone()));
        }
    }
    Ok(())
}

/// The state of a widget's toggles between taps: each shows its new state at once and settles on the app's answer
/// (see [`Toggle`]). A reload clears it, so the app's own state wins.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ToggleSet {
    toggles: Vec<(String, Toggle)>,
}

impl ToggleSet {
    /// The state a toggle shows: its pending or settled state once tapped, else `spec`'s own. None for a button.
    pub fn shown(&self, spec: &ButtonSpec) -> Option<bool> {
        let own = spec.toggle?;
        Some(self.toggles.iter().find(|(id, _)| *id == spec.id).map_or(own, |(_, t)| t.on()))
    }

    /// True while the app has not answered the last tap on this button.
    pub fn is_pending(&self, id: &str) -> bool {
        self.toggles.iter().any(|(i, t)| i == id && t.is_pending())
    }

    /// True while any toggle waits for the app's answer.
    pub fn any_pending(&self) -> bool {
        self.toggles.iter().any(|(_, t)| t.is_pending())
    }

    /// A tap on `spec`: the invocation to send. A toggle flips and waits for [`ToggleSet::settle`]; a second tap while
    /// it waits is refused with None. A button always yields its invocation.
    pub fn tap(&mut self, spec: &ButtonSpec) -> Option<Invocation> {
        let Some(own) = spec.toggle else { return Some(spec.invocation(None)) };
        let idx = match self.toggles.iter().position(|(id, _)| *id == spec.id) {
            Some(i) => i,
            None => {
                self.toggles.push((spec.id.clone(), Toggle::new(own)));
                self.toggles.len() - 1
            }
        };
        let next = self.toggles[idx].1.tap()?;
        Some(spec.invocation(Some(next)))
    }

    /// The app's answer to the last tap on `id`. Does nothing for a button or when none is pending.
    pub fn settle(&mut self, id: &str, ok: bool) {
        if let Some((_, t)) = self.toggles.iter_mut().find(|(i, _)| i == id) {
            t.settle(ok);
        }
    }

    /// Forgets every toggle (a reload brings the app's own state).
    pub fn clear(&mut self) {
        self.toggles.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_button_carries_its_intent_and_parameters_and_no_state() {
        let b = ButtonSpec::button("done", " Mark done ", "mark-done").unwrap().with_param("item", "7");
        assert_eq!(b.label, "Mark done", "the label is trimmed");
        let inv = b.invocation(None);
        assert_eq!(inv, Invocation { intent: "mark-done".into(), params: vec![("item".into(), "7".into())] });
        assert_eq!(b.invocation(Some(true)).params.len(), 1, "a button sends no toggle state");
    }

    #[test]
    fn a_toggle_sends_its_new_state_in_the_on_parameter() {
        let t = ButtonSpec::toggle("mute", "Mute", "set-mute", false).unwrap().with_param("room", "a");
        let inv = t.invocation(Some(true));
        assert_eq!(inv.params, vec![("room".to_string(), "a".to_string()), ("on".to_string(), "true".to_string())]);
        let again = t.with_param("on", "stale").invocation(Some(false));
        assert_eq!(again.params.iter().filter(|(k, _)| k == "on").count(), 1, "one on parameter, the new state");
        assert_eq!(again.params.last().unwrap().1, "false");
    }

    #[test]
    fn unsafe_ids_and_blank_labels_are_refused() {
        assert_eq!(ButtonSpec::button("a b", "x", "i").unwrap_err(), ButtonError::BadId("a b".into()));
        assert_eq!(ButtonSpec::button("a", "x", "i/../j").unwrap_err(), ButtonError::BadId("i/../j".into()));
        assert_eq!(ButtonSpec::button("", "x", "i").unwrap_err(), ButtonError::BadId(String::new()));
        assert_eq!(ButtonSpec::button("a", "  ", "i").unwrap_err(), ButtonError::NoLabel);
        let long = "a".repeat(MAX_BUTTON_ID + 1);
        assert!(ButtonSpec::button(&long, "x", "i").is_err());
        assert!(ButtonSpec::button("a.b-c_d", "x", "i").is_ok());
    }

    #[test]
    fn an_entry_takes_a_few_buttons_with_distinct_ids() {
        let b = |id: &str| ButtonSpec::button(id, "x", "i").unwrap();
        assert_eq!(check_buttons(&[b("a"), b("b"), b("c")]), Ok(()));
        assert_eq!(check_buttons(&[b("a"), b("b"), b("c"), b("d")]), Err(ButtonError::TooMany));
        assert_eq!(check_buttons(&[b("a"), b("a")]), Err(ButtonError::Duplicate("a".into())));
        assert_eq!(check_buttons(&[]), Ok(()));
    }

    #[test]
    fn a_tapped_toggle_shows_the_new_state_until_the_app_answers() {
        let spec = ButtonSpec::toggle("mute", "Mute", "set-mute", false).unwrap();
        let mut set = ToggleSet::default();
        assert_eq!(set.shown(&spec), Some(false));
        let inv = set.tap(&spec).expect("first tap goes out");
        assert_eq!(inv.params, vec![("on".to_string(), "true".to_string())]);
        assert_eq!(set.shown(&spec), Some(true));
        assert!(set.is_pending("mute"));
        assert_eq!(set.tap(&spec), None, "a tap while the app has not answered is ignored");
        set.settle("mute", false);
        assert_eq!(set.shown(&spec), Some(false), "a failed change goes back");
        assert!(!set.is_pending("mute"));
        let next = set.tap(&spec).unwrap();
        set.settle("mute", true);
        assert_eq!(next.params[0].1, "true");
        assert_eq!(set.shown(&spec), Some(true), "a successful change stays");
        assert_eq!(set.tap(&spec).unwrap().params[0].1, "false", "the next tap turns it off");
    }

    #[test]
    fn a_button_has_no_shown_state_and_clear_brings_back_the_apps_state() {
        let button = ButtonSpec::button("go", "Go", "go").unwrap();
        let spec = ButtonSpec::toggle("t", "T", "t", true).unwrap();
        let mut set = ToggleSet::default();
        assert_eq!(set.shown(&button), None);
        assert!(set.tap(&button).is_some() && set.tap(&button).is_some(), "a button can be tapped again at once");
        set.tap(&spec);
        set.settle("t", true);
        assert_eq!(set.shown(&spec), Some(false));
        set.clear();
        assert_eq!(set.shown(&spec), Some(true));
    }

    fn rect(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect { x, y, w, h }
    }

    #[test]
    fn edges_are_left_edge_inclusive_and_right_edge_exclusive() {
        let r = rect(0.0, 0.0, 10.0, 10.0);
        assert!(r.contains(0.0, 0.0));
        assert!(r.contains(9.9_f32, 5.0));
        assert!(!r.contains(10.0, 5.0));
        assert!(!r.contains(5.0, 10.0));
    }

    #[test]
    fn topmost_region_wins_and_outside_is_none() {
        let hits = vec![
            Hit { frame: rect(0.0, 0.0, 100.0, 100.0), action: Action::RunIntent { id: "under".into() } },
            Hit { frame: rect(10.0, 10.0, 20.0, 20.0), action: Action::RunIntent { id: "over".into() } },
        ];
        assert_eq!(hit_test(&hits, 15.0, 15.0), Some(&Action::RunIntent { id: "over".into() }));
        assert_eq!(hit_test(&hits, 50.0, 50.0), Some(&Action::RunIntent { id: "under".into() }));
        assert_eq!(hit_test(&hits, 150.0, 50.0), None);
    }

    #[test]
    fn a_tapped_link_region_yields_its_url() {
        let link = Link::parse("bifrost-app://notes/a").unwrap();
        let hits = vec![Hit { frame: rect(0.0, 0.0, 50.0, 50.0), action: Action::OpenLink(link) }];
        match hit_test(&hits, 1.0, 1.0) {
            Some(Action::OpenLink(l)) => assert_eq!(l.to_url(), "bifrost-app://notes/a"),
            other => panic!("unexpected {:?}", other),
        }
    }

    #[test]
    fn tap_shows_new_state_and_waits_for_the_app() {
        let mut t = Toggle::new(false);
        assert_eq!(t.tap(), Some(true));
        assert!(t.on());
        assert!(t.is_pending());
        assert_eq!(t.tap(), None);
        assert!(t.on());
    }

    #[test]
    fn failed_change_puts_back_the_committed_state() {
        let mut t = Toggle::new(false);
        t.tap();
        t.settle(false);
        assert!(!t.on());
        assert!(!t.is_pending());
        assert_eq!(t.tap(), Some(true));
    }

    #[test]
    fn successful_change_is_committed() {
        let mut t = Toggle::new(true);
        assert_eq!(t.tap(), Some(false));
        t.settle(true);
        assert!(!t.on());
        assert_eq!(t.tap(), Some(true));
    }

    #[test]
    fn settle_without_a_pending_change_does_nothing() {
        let mut t = Toggle::new(true);
        t.settle(false);
        assert!(t.on());
        assert!(!t.is_pending());
    }
}
