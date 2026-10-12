//! Interactive controls in a widget or live activity, as a pure model. A widget can hold buttons and toggles that run
//! an app intent without launching the app, can open a link when it is clicked (the widget's own URL), and gives assistive technology a description of itself and of each control.
//!
//! Rules: a press on a control that is disabled does nothing. On a locked device a button or toggle is inert and
//! changes nothing, because the system runs no action until the person unlocks. A toggle flips on each press and
//! sends its new state with the intent. A click on a control is handled by that control and never reaches the
//! widget's link. A link needs a scheme, no whitespace, something after the scheme, and must not be a script or
//! data scheme.

use crate::Rect;

/// What a control does when pressed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    /// A button: runs the app intent `intent`.
    Button { intent: String },
    /// A toggle: flips `on`, then runs the app intent `intent` with the new state.
    Toggle { intent: String, on: bool },
}

/// The intent run by a press, with the value a toggle sends (None for a button).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Invocation {
    pub intent: String,
    pub value: Option<bool>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Control {
    pub label: String,
    pub bounds: Rect,
    pub action: Action,
    pub enabled: bool,
}

/// The result of a click on a widget.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Tap {
    /// A control took the click and ran an intent.
    Ran(Invocation),
    /// A control took the click but nothing ran (disabled, or the device is locked).
    Consumed,
    /// The click missed every control and the widget opens its link.
    Open(String),
    /// The click missed every control and the widget has no link.
    Nothing,
}

/// A widget with its controls, its link and the text that describes it to assistive technology.
#[derive(Clone, Debug, PartialEq)]
pub struct Widget {
    pub title: String,
    /// An explicit description. Empty means the title describes the widget.
    pub description: String,
    pub controls: Vec<Control>,
    pub link: Option<String>,
}

/// Presses `control` at `locked` state. A button yields its invocation. A toggle flips its state first, so the
/// invocation carries the new state. Returns None when the control is disabled or the device is locked; then
/// nothing changes.
pub fn press(control: &mut Control, locked: bool) -> Option<Invocation> {
    if !control.enabled || locked {
        return None;
    }
    match &mut control.action {
        Action::Button { intent } => Some(Invocation { intent: intent.clone(), value: None }),
        Action::Toggle { intent, on } => {
            *on = !*on;
            Some(Invocation { intent: intent.clone(), value: Some(*on) })
        }
    }
}

/// The index of the control under (x, y). Controls later in the list are drawn on top, so the last match wins.
pub fn hit(controls: &[Control], x: f32, y: f32) -> Option<usize> {
    controls.iter().rposition(|c| c.bounds.contains(x, y))
}

/// A click at (x, y) on `widget`. A control under the point takes the click; otherwise the widget's link opens.
pub fn tap(widget: &mut Widget, x: f32, y: f32, locked: bool) -> Tap {
    match hit(&widget.controls, x, y) {
        Some(i) => match press(&mut widget.controls[i], locked) {
            Some(invocation) => Tap::Ran(invocation),
            None => Tap::Consumed,
        },
        None => match &widget.link {
            Some(link) => Tap::Open(link.clone()),
            None => Tap::Nothing,
        },
    }
}

/// Whether `url` can be a widget link: a scheme of letters, digits, `+`, `-` or `.` that starts with a letter, a
/// colon, something after it, and no whitespace. Script and data schemes are refused.
pub fn valid_link(url: &str) -> bool {
    let Some((scheme, rest)) = url.split_once(':') else {
        return false;
    };
    let mut chars = scheme.chars();
    let starts_with_letter = matches!(chars.next(), Some(c) if c.is_ascii_alphabetic());
    let scheme_ok = starts_with_letter && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'));
    let refused = matches!(scheme.to_ascii_lowercase().as_str(), "javascript" | "data" | "vbscript");
    scheme_ok && !refused && !rest.is_empty() && !url.chars().any(char::is_whitespace)
}

/// What assistive technology reads for one control.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ControlDescription {
    pub label: String,
    /// The state of a toggle: "On" or "Off". Empty for a button.
    pub value: String,
}

pub fn describe_control(control: &Control) -> ControlDescription {
    let value = match &control.action {
        Action::Button { .. } => String::new(),
        Action::Toggle { on: true, .. } => "On".to_string(),
        Action::Toggle { on: false, .. } => "Off".to_string(),
    };
    ControlDescription { label: control.label.trim().to_string(), value }
}

/// The description of the widget as a whole: its explicit description, else its title. None when both are blank,
/// so the shell can skip the description rather than read nothing.
pub fn describe_widget(widget: &Widget) -> Option<String> {
    let explicit = widget.description.trim();
    if !explicit.is_empty() {
        return Some(explicit.to_string());
    }
    let title = widget.title.trim();
    (!title.is_empty()).then(|| title.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect { x, y, w, h }
    }

    fn button(label: &str, bounds: Rect) -> Control {
        Control { label: label.into(), bounds, action: Action::Button { intent: "note.new".into() }, enabled: true }
    }

    fn toggle(label: &str, bounds: Rect, on: bool) -> Control {
        Control {
            label: label.into(),
            bounds,
            action: Action::Toggle { intent: "task.complete".into(), on },
            enabled: true,
        }
    }

    fn widget(controls: Vec<Control>, link: Option<&str>) -> Widget {
        Widget { title: "Tasks".into(), description: String::new(), controls, link: link.map(str::to_string) }
    }

    #[test]
    fn a_button_runs_its_intent_without_a_value() {
        let mut b = button("New", rect(0.0, 0.0, 40.0, 20.0));
        assert_eq!(press(&mut b, false), Some(Invocation { intent: "note.new".into(), value: None }));
    }

    #[test]
    fn a_toggle_flips_on_each_press_and_sends_the_new_state() {
        let mut t = toggle("Done", rect(0.0, 0.0, 20.0, 20.0), false);
        assert_eq!(press(&mut t, false).unwrap().value, Some(true));
        assert_eq!(press(&mut t, false).unwrap().value, Some(false));
        assert_eq!(t.action, Action::Toggle { intent: "task.complete".into(), on: false });
    }

    #[test]
    fn a_locked_device_runs_nothing_and_the_toggle_keeps_its_state() {
        let mut t = toggle("Done", rect(0.0, 0.0, 20.0, 20.0), false);
        assert_eq!(press(&mut t, true), None);
        assert_eq!(t.action, Action::Toggle { intent: "task.complete".into(), on: false }, "unchanged while locked");
    }

    #[test]
    fn a_disabled_control_does_nothing() {
        let mut b = button("New", rect(0.0, 0.0, 40.0, 20.0));
        b.enabled = false;
        assert_eq!(press(&mut b, false), None);
    }

    #[test]
    fn a_click_on_a_control_is_consumed_and_a_miss_opens_the_link() {
        let mut w = widget(vec![button("New", rect(10.0, 10.0, 40.0, 20.0))], Some("bifrost://tasks/today"));
        assert_eq!(tap(&mut w, 12.0, 12.0, false), Tap::Ran(Invocation { intent: "note.new".into(), value: None }));
        assert_eq!(tap(&mut w, 0.0, 0.0, false), Tap::Open("bifrost://tasks/today".into()));
        w.link = None;
        assert_eq!(tap(&mut w, 0.0, 0.0, false), Tap::Nothing);
    }

    #[test]
    fn a_click_on_a_locked_control_is_consumed_and_does_not_open_the_link() {
        let mut w = widget(vec![button("New", rect(10.0, 10.0, 40.0, 20.0))], Some("bifrost://tasks/today"));
        assert_eq!(tap(&mut w, 12.0, 12.0, true), Tap::Consumed);
    }

    #[test]
    fn the_control_drawn_on_top_takes_an_overlapping_click() {
        let controls = [button("Under", rect(0.0, 0.0, 50.0, 50.0)), button("Over", rect(20.0, 20.0, 10.0, 10.0))];
        assert_eq!(hit(&controls, 25.0, 25.0), Some(1));
        assert_eq!(hit(&controls, 5.0, 5.0), Some(0));
        assert_eq!(hit(&controls, 60.0, 60.0), None);
    }

    #[test]
    fn links_need_a_plain_scheme_and_refuse_scripts() {
        assert!(valid_link("bifrost://notes/new"));
        assert!(valid_link("mailto:a@b.c"));
        assert!(!valid_link("notes/new"), "no scheme");
        assert!(!valid_link("1app://x"), "scheme starts with a digit");
        assert!(!valid_link("bifrost:"), "nothing after the scheme");
        assert!(!valid_link("bifrost:// x"), "whitespace");
        assert!(!valid_link("javascript:alert(1)"));
        assert!(!valid_link("DATA:text/html,x"));
    }

    #[test]
    fn a_toggle_is_described_by_its_state_and_a_widget_falls_back_to_its_title() {
        assert_eq!(describe_control(&toggle(" Done ", rect(0.0, 0.0, 1.0, 1.0), true)).value, "On");
        let off = describe_control(&toggle("Done", rect(0.0, 0.0, 1.0, 1.0), false));
        assert_eq!(off, ControlDescription { label: "Done".into(), value: "Off".into() });
        assert_eq!(describe_control(&button("New", rect(0.0, 0.0, 1.0, 1.0))).value, "");
        let mut w = widget(vec![], None);
        assert_eq!(describe_widget(&w), Some("Tasks".into()));
        w.description = "  Open tasks due today  ".into();
        assert_eq!(describe_widget(&w), Some("Open tasks due today".into()));
        w.title = "  ".into();
        w.description = String::new();
        assert_eq!(describe_widget(&w), None);
    }
}
