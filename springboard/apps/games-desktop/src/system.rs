//! The Games app's links to the shell's system services: game controllers (`gamepad`), power (`power`) and displays
//! (`screens`).
//!
//! The app asks each service once, when it opens. After that the shell pushes changes (a controller plugged in, Low
//! Power Mode switched on, the display resized) and this module turns each one into an [`Act`] for the game. A
//! service the shell does not have answers with an error; that is ignored and the game runs as it did before.
//! The state is plain data, so the tests below run without a window.

use bifrost_app::controls::gamepad::{Axis, Button, Change};
use bifrost_app::gamepad_service::{self, Controller, Event as PadEvent};
use bifrost_app::power_service::{self, Change as PowerChange};
use bifrost_app::process_info::ThermalState;
use bifrost_app::screens::{self, Screens};
use bifrost_app::Ctx;
use bifrost_core::json::Json;

/// What a system event asks the game to do.
#[derive(Clone, Debug, PartialEq)]
pub enum Act {
    /// slide the 2048 board: 0 left, 1 right, 2 up, 3 down
    Slide(u8),
    /// the controller's South button (the game starts a new one only when the current game is over)
    NewGame,
    /// a line for the game's message row
    Notice(String),
    /// the power or display state changed; read it through [`Link::reduce_effects`] and [`Link::display`]
    State,
}

/// Which call a request id belongs to, so its reply is decoded the right way.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Want {
    Pads,
    Power,
    Displays,
}

/// A stick past this value counts as pushed, and one back inside `STICK_OFF` as released; the gap stops a stick
/// resting near the line from sending a slide each time it wobbles.
const STICK_ON: f32 = 0.6;
const STICK_OFF: f32 = 0.4;

/// The state of the links: the controllers, the power state, the main display and the last stick and pad values.
pub struct Link {
    pads: Vec<Controller>,
    low_power: bool,
    thermal: ThermalState,
    /// the main display's visible size in points
    display: Option<(f32, f32)>,
    /// last direction per left-stick axis, -1, 0 or 1
    stick: (i8, i8),
    /// last direction-pad value
    hat: (i8, i8),
    pending: Vec<(u32, Want)>,
}

impl Default for Link {
    fn default() -> Self {
        Self::new()
    }
}

impl Link {
    pub fn new() -> Self {
        Self {
            pads: Vec::new(),
            low_power: false,
            thermal: ThermalState::Nominal,
            display: None,
            stick: (0, 0),
            hat: (0, 0),
            pending: Vec::new(),
        }
    }

    /// Asks the shell for the controllers, the power state and the displays. Called once, when the app opens.
    pub fn start(&mut self, ctx: &Ctx) {
        self.pending.push((gamepad_service::list(ctx), Want::Pads));
        self.pending.push((power_service::snapshot(ctx), Want::Power));
        self.pending.push((screens::list(ctx), Want::Displays));
    }

    /// The reply to one of the calls [`Link::start`] made. Returns false for a request this module did not make, so
    /// the app leaves that reply alone. An error reply (the shell lacks the service) changes nothing.
    pub fn on_reply(&mut self, request: u32, result: &Result<Json, String>) -> bool {
        let Some(i) = self.pending.iter().position(|(id, _)| *id == request) else {
            return false;
        };
        let (_, want) = self.pending.remove(i);
        match want {
            Want::Pads => {
                if let Ok(list) = result {
                    self.pads = gamepad_service::parse_list(list);
                }
            }
            Want::Power => {
                if let Some(s) = power_service::decode_snapshot(result) {
                    self.thermal = s.thermal_state;
                    self.low_power = s.low_power_mode;
                }
            }
            Want::Displays => {
                if let Ok(list) = screens::from_reply(result) {
                    self.display = main_size(&list);
                }
            }
        }
        true
    }

    /// A pushed service event. None when the event is not one the app uses (another service, an unknown event, or a
    /// battery change, which the Games app does not show).
    pub fn on_event(&mut self, service: &str, event: &str, body: &Json) -> Option<Act> {
        if let Some(pad) = gamepad_service::event(service, event, body) {
            return match pad {
                PadEvent::Connected(c) => {
                    let line = format!("Controller connected: {}", c.name);
                    if !self.pads.iter().any(|p| p.id == c.id) {
                        self.pads.push(c);
                    }
                    Some(Act::Notice(line))
                }
                PadEvent::Disconnected(c) => {
                    self.pads.retain(|p| p.id != c.id);
                    // a pad that left may have been holding a direction; start from neutral
                    self.stick = (0, 0);
                    self.hat = (0, 0);
                    Some(Act::Notice(format!("Controller disconnected: {}", c.name)))
                }
                PadEvent::Input { change, .. } => self.input(change),
            };
        }
        if let Some(change) = power_service::decode_change(service, event, body) {
            return match change {
                PowerChange::ThermalState(t) => {
                    self.thermal = t;
                    Some(Act::State)
                }
                PowerChange::LowPowerMode(on) => {
                    self.low_power = on;
                    Some(Act::State)
                }
                PowerChange::Battery(_) => None,
            };
        }
        match screens::changed(service, event, body)? {
            Ok(list) => {
                self.display = main_size(&list);
                Some(Act::State)
            }
            Err(_) => None,
        }
    }

    /// Motion is lowered in Low Power Mode and at a serious or critical thermal state.
    pub fn reduce_effects(&self) -> bool {
        self.low_power || self.thermal >= ThermalState::Serious
    }

    /// The main display's visible size in points, once the shell has reported one.
    pub fn display(&self) -> Option<(f32, f32)> {
        self.display
    }

    /// A controller's element changed. Only the directions and the South button mean something to the games.
    fn input(&mut self, change: Change) -> Option<Act> {
        match change {
            Change::Button { button: Button::South, pressed: true } => Some(Act::NewGame),
            Change::Button { .. } => None,
            // a diagonal arrives as two events; the component that just became pressed decides the slide
            Change::Dpad { x, y } => {
                let (px, py) = std::mem::replace(&mut self.hat, (x, y));
                if x != 0 && x != px {
                    Some(Act::Slide(if x < 0 { 0 } else { 1 }))
                } else if y != 0 && y != py {
                    // the hat reports y down as positive
                    Some(Act::Slide(if y < 0 { 2 } else { 3 }))
                } else {
                    None
                }
            }
            Change::Axis { axis: Axis::LeftX, value } => {
                let d = stick_dir(value, self.stick.0);
                let last = std::mem::replace(&mut self.stick.0, d);
                (d != 0 && d != last).then(|| Act::Slide(if d < 0 { 0 } else { 1 }))
            }
            Change::Axis { axis: Axis::LeftY, value } => {
                let d = stick_dir(value, self.stick.1);
                let last = std::mem::replace(&mut self.stick.1, d);
                (d != 0 && d != last).then(|| Act::Slide(if d < 0 { 2 } else { 3 }))
            }
            Change::Axis { .. } => None,
        }
    }
}

/// The direction a stick value means: 1 or -1 past the on line, 0 inside the off line, and the last direction in
/// between (the dead band).
fn stick_dir(value: f32, last: i8) -> i8 {
    if value >= STICK_ON {
        1
    } else if value <= -STICK_ON {
        -1
    } else if value.abs() < STICK_OFF {
        0
    } else {
        last
    }
}

fn main_size(list: &Screens) -> Option<(f32, f32)> {
    list.main().map(|m| (m.visible_frame.w, m.visible_frame.h))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::games::test_ui;
    use bifrost_app::proto::{Configure, ToShell};

    fn ctx() -> Ctx {
        let cfg = Configure {
            w: 1200,
            h: 780,
            scale: 1.0,
            safe_top: 0.0,
            safe_bottom: 0.0,
            dark: false,
            text_scale: 1.0,
            reduce_motion: false,
        };
        Ctx::testing(test_ui(), cfg)
    }

    fn j(text: &str) -> Json {
        bifrost_core::json::parse(text).unwrap()
    }

    /// The request ids of the calls the app made, in order, with their service names.
    fn calls(ctx: &Ctx) -> Vec<(u32, String, String)> {
        ctx.take_sent()
            .into_iter()
            .filter_map(|m| match m {
                ToShell::ServiceCall { request, service, method, .. } => Some((request, service, method)),
                _ => None,
            })
            .collect()
    }

    /// A started link, with the ids of its three calls.
    fn started() -> (Link, [u32; 3]) {
        let c = ctx();
        let mut link = Link::new();
        link.start(&c);
        let sent = calls(&c);
        assert_eq!(sent.len(), 3);
        assert_eq!(sent[0].1, "gamepad");
        assert_eq!(sent[1].1, "power");
        assert_eq!(sent[2].1, "screens");
        (link, [sent[0].0, sent[1].0, sent[2].0])
    }

    fn pad_input(kind: &str, name: &str, extra: &[(&str, &str)]) -> Json {
        let mut s = format!(r#"{{"id":1,"kind":"{kind}","name":"{name}""#);
        for (k, v) in extra {
            s.push_str(&format!(r#","{k}":{v}"#));
        }
        s.push('}');
        j(&s)
    }

    fn dpad(x: i32, y: i32) -> Option<Act> {
        let mut link = Link::new();
        link.on_event("gamepad", "input", &j(&format!(r#"{{"id":1,"kind":"dpad","x":{x},"y":{y}}}"#)))
    }

    #[test]
    fn start_asks_the_gamepad_power_and_screens_services_once_each() {
        let c = ctx();
        let mut link = Link::new();
        link.start(&c);
        let sent = calls(&c);
        let names: Vec<(&str, &str)> = sent.iter().map(|(_, s, m)| (s.as_str(), m.as_str())).collect();
        assert_eq!(names, vec![("gamepad", "list"), ("power", "snapshot"), ("screens", "list")]);
    }

    #[test]
    fn a_power_snapshot_in_serious_heat_or_low_power_mode_reduces_effects() {
        let (mut link, [_, power, _]) = started();
        assert!(!link.reduce_effects());
        let reply = Ok(j(r#"{"thermalState":"serious","lowPowerMode":false,"battery":{"present":false}}"#));
        assert!(link.on_reply(power, &reply));
        assert!(link.reduce_effects());

        let (mut link, [_, power, _]) = started();
        let reply = Ok(j(r#"{"thermalState":"nominal","lowPowerMode":true,"battery":{"present":false}}"#));
        assert!(link.on_reply(power, &reply));
        assert!(link.reduce_effects());
    }

    #[test]
    fn a_low_power_change_event_turns_the_reduction_on_and_off() {
        let mut link = Link::new();
        let on = j(r#"{"lowPowerMode":true}"#);
        assert_eq!(link.on_event("power", "powerStateDidChange", &on), Some(Act::State));
        assert!(link.reduce_effects());
        let off = j(r#"{"lowPowerMode":false}"#);
        assert_eq!(link.on_event("power", "powerStateDidChange", &off), Some(Act::State));
        assert!(!link.reduce_effects());
    }

    #[test]
    fn a_thermal_change_event_reduces_effects_only_when_serious() {
        let mut link = Link::new();
        assert_eq!(link.on_event("power", "thermalStateDidChange", &j(r#"{"thermalState":"fair"}"#)), Some(Act::State));
        assert!(!link.reduce_effects());
        link.on_event("power", "thermalStateDidChange", &j(r#"{"thermalState":"critical"}"#));
        assert!(link.reduce_effects());
    }

    #[test]
    fn battery_events_and_other_services_are_not_the_games_business() {
        let mut link = Link::new();
        assert_eq!(link.on_event("power", "batteryDidChange", &j(r#"{"battery":{"present":false}}"#)), None);
        assert_eq!(link.on_event("dock", "input", &j("{}")), None);
    }

    #[test]
    fn a_missing_service_answers_with_an_error_and_nothing_changes() {
        let (mut link, [pads, power, displays]) = started();
        assert!(link.on_reply(pads, &Err("no service gamepad".into())));
        assert!(link.on_reply(power, &Err("no service power".into())));
        assert!(link.on_reply(displays, &Err("no service screens".into())));
        assert!(!link.reduce_effects());
        assert_eq!(link.display(), None);
    }

    #[test]
    fn a_reply_to_an_unknown_request_is_left_for_the_shell_mailbox() {
        let (mut link, _) = started();
        assert!(!link.on_reply(9_999_999, &Ok(Json::Null)));
    }

    #[test]
    fn the_display_comes_from_the_list_reply_and_from_a_resize() {
        let (mut link, [_, _, displays]) = started();
        let list = j(
            r#"[{"name":"Display","frame":{"x":0,"y":0,"w":1440,"h":900},"visible_frame":{"x":0,"y":24,"w":1440,"h":816},"scale":2,"main":true}]"#,
        );
        assert!(link.on_reply(displays, &Ok(list)));
        assert_eq!(link.display(), Some((1440.0, 816.0)));

        let resized = j(
            r#"[{"name":"Display","frame":{"x":0,"y":0,"w":1280,"h":800},"visible_frame":{"x":0,"y":24,"w":1280,"h":776},"scale":1,"main":true}]"#,
        );
        assert_eq!(link.on_event("screens", "changed", &resized), Some(Act::State));
        assert_eq!(link.display(), Some((1280.0, 776.0)));
    }

    #[test]
    fn a_controller_connect_and_disconnect_post_a_notice_with_its_name() {
        let mut link = Link::new();
        let pad = j(r#"{"id":4,"name":"Pad Two","path":"/dev/input/event7"}"#);
        assert_eq!(
            link.on_event("gamepad", "connected", &pad),
            Some(Act::Notice("Controller connected: Pad Two".into()))
        );
        assert_eq!(
            link.on_event("gamepad", "disconnected", &pad),
            Some(Act::Notice("Controller disconnected: Pad Two".into()))
        );
    }

    #[test]
    fn the_list_reply_fills_the_controllers_without_duplicates_on_connect() {
        let (mut link, [pads, _, _]) = started();
        let list = j(r#"[{"id":1,"name":"Pad","path":"/dev/input/event5"}]"#);
        assert!(link.on_reply(pads, &Ok(list)));
        let again = j(r#"{"id":1,"name":"Pad","path":"/dev/input/event5"}"#);
        link.on_event("gamepad", "connected", &again);
        assert_eq!(link.pads.len(), 1);
    }

    #[test]
    fn the_direction_pad_slides_once_per_press_and_diagonals_do_not_double() {
        let mut link = Link::new();
        let at = |link: &mut Link, x: i32, y: i32| {
            link.on_event("gamepad", "input", &j(&format!(r#"{{"id":1,"kind":"dpad","x":{x},"y":{y}}}"#)))
        };
        assert_eq!(at(&mut link, -1, 0), Some(Act::Slide(0)));
        // a diagonal down-left: the x component is already held, so only the new y slides
        assert_eq!(at(&mut link, -1, 1), Some(Act::Slide(3)));
        assert_eq!(at(&mut link, 0, 0), None);
        assert_eq!(at(&mut link, 1, 0), Some(Act::Slide(1)));
        assert_eq!(at(&mut link, 0, -1), Some(Act::Slide(2)));
        assert_eq!(dpad(0, 0), None);
    }

    #[test]
    fn the_left_stick_slides_when_pushed_past_the_line_and_not_while_held() {
        let mut link = Link::new();
        let stick = |link: &mut Link, axis: &str, value: f32| {
            let text = value.to_string();
            let body = pad_input("axis", axis, &[("value", text.as_str())]);
            link.on_event("gamepad", "input", &body)
        };
        assert_eq!(stick(&mut link, "leftX", 0.7), Some(Act::Slide(1)));
        // still held right, and the wobble to 0.5 is inside the dead band: no second slide
        assert_eq!(stick(&mut link, "leftX", 0.8), None);
        assert_eq!(stick(&mut link, "leftX", 0.5), None);
        // back to the middle, then left
        assert_eq!(stick(&mut link, "leftX", 0.1), None);
        assert_eq!(stick(&mut link, "leftX", -0.9), Some(Act::Slide(0)));
        assert_eq!(stick(&mut link, "leftY", -0.8), Some(Act::Slide(2)));
        assert_eq!(stick(&mut link, "leftY", 0.9), Some(Act::Slide(3)));
        // the right stick does nothing to the games
        assert_eq!(stick(&mut link, "rightX", 1.0), None);
    }

    #[test]
    fn only_a_press_of_south_asks_for_a_new_game() {
        let mut link = Link::new();
        let south = |link: &mut Link, pressed: bool| {
            let body = pad_input("button", "south", &[("pressed", if pressed { "true" } else { "false" })]);
            link.on_event("gamepad", "input", &body)
        };
        assert_eq!(south(&mut link, true), Some(Act::NewGame));
        assert_eq!(south(&mut link, false), None);
        let east = pad_input("button", "east", &[("pressed", "true")]);
        assert_eq!(link.on_event("gamepad", "input", &east), None);
    }

    #[test]
    fn a_disconnect_clears_a_held_direction() {
        let mut link = Link::new();
        link.on_event("gamepad", "input", &j(r#"{"id":1,"kind":"dpad","x":1,"y":0}"#));
        link.on_event("gamepad", "disconnected", &j(r#"{"id":1,"name":"Pad","path":"/dev/input/event5"}"#));
        // a fresh press of the same direction slides again
        assert_eq!(
            link.on_event("gamepad", "input", &j(r#"{"id":1,"kind":"dpad","x":1,"y":0}"#)),
            Some(Act::Slide(1))
        );
    }

    #[test]
    fn the_snapshot_reply_sets_the_thermal_state() {
        let (mut link, [_, power, _]) = started();
        let reply = Ok(j(r#"{"thermalState":"fair","lowPowerMode":false,"battery":{"present":false}}"#));
        link.on_reply(power, &reply);
        assert!(!link.reduce_effects());
        assert_eq!(link.thermal, ThermalState::Fair);
    }
}
