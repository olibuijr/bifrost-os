//! Typed client for the `gamepad` shell service: the game controllers the shell found, their connect and disconnect,
//! and the button and stick values an app gets while it is in front.
//!
//! ```ignore
//! // at launch, and again when the state is needed:
//! let list_request = gamepad_service::list(ctx);      // reply: App::service_reply, parse with parse_list
//! // App::service_event: a connect, a disconnect or one element's change, sent to the focused app only
//! if let Some(ev) = gamepad_service::event(service, event, body) {
//!     match ev {
//!         Event::Input { controller, change } => { /* Change::Button / Axis / Dpad */ }
//!         Event::Connected(c) | Event::Disconnected(c) => { /* c.id, c.name */ }
//!     }
//! }
//! ```
//! Controller ids are kept by the shell across unplugs of the same pad, but are never reused for another pad.
//! `rumble` drives the two motors of a pad that has them; the controller's Home button is the shell's, it opens the launcher.
//! Connect and disconnect go to every running app; `input` goes to the app in front only.
use crate::controls::gamepad::{Axis, Button, Change};
use crate::runtime::Ctx;
use crate::service;
use bifrost_core::json::Json;
use springboard::shell::services::gamepad_service::{axis_from_name, button_from_name};

/// The shell service name.
pub const SERVICE: &str = "gamepad";

/// A connected controller as the shell reports it.
#[derive(Clone, Debug, PartialEq)]
pub struct Controller {
    pub id: u32,
    pub name: String,
    /// the input node, for example `/dev/input/event5`
    pub path: String,
}

/// The state of one controller: pressed buttons, every axis value (-1..=1 for sticks, 0..=1 for triggers) and the
/// direction pad as (x, y), each -1, 0 or 1.
#[derive(Clone, Debug, PartialEq)]
pub struct State {
    pub id: u32,
    pub pressed: Vec<Button>,
    /// indexed by `Axis as usize`, in `Axis::ALL` order
    pub axes: [f32; 6],
    pub dpad: (i8, i8),
}

impl State {
    pub fn is_pressed(&self, button: Button) -> bool {
        self.pressed.contains(&button)
    }

    pub fn axis(&self, axis: Axis) -> f32 {
        self.axes[axis as usize]
    }
}

/// A service event, decoded.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    Connected(Controller),
    Disconnected(Controller),
    /// one element of controller `controller` changed
    Input { controller: u32, change: Change },
}

/// Asks for the connected controllers. The reply is parsed with [`parse_list`].
pub fn list(ctx: &Ctx) -> u32 {
    service::call(ctx, SERVICE, "list", &Json::Null)
}

/// Asks for the state of controller `id`. The reply is parsed with [`parse_state`].
pub fn state(ctx: &Ctx, id: u32) -> u32 {
    service::call(ctx, SERVICE, "state", &Json::Obj(vec![("id".into(), Json::Num(f64::from(id)))]))
}

/// Rumbles controller `id`: the heavy and light motors, each from 0 to 1, for `duration_ms` (a zero duration or two
/// zero levels stop it). Only the app in front may rumble; another app gets an error reply.
pub fn rumble(ctx: &Ctx, id: u32, strong: f32, weak: f32, duration_ms: u32) -> u32 {
    service::call(ctx, SERVICE, "rumble", &rumble_body(id, strong, weak, duration_ms))
}

/// The body of a `rumble` call.
pub fn rumble_body(id: u32, strong: f32, weak: f32, duration_ms: u32) -> Json {
    Json::Obj(vec![
        ("id".into(), Json::Num(f64::from(id))),
        ("strong".into(), Json::Num(f64::from(strong.clamp(0.0, 1.0)))),
        ("weak".into(), Json::Num(f64::from(weak.clamp(0.0, 1.0)))),
        ("duration".into(), Json::Num(f64::from(duration_ms))),
    ])
}

/// The controllers in a `list` reply. A malformed entry is skipped.
pub fn parse_list(reply: &Json) -> Vec<Controller> {
    reply.array().unwrap_or(&[]).iter().filter_map(parse_controller).collect()
}

/// The state in a `state` reply, or `None` when the reply is not one.
pub fn parse_state(reply: &Json) -> Option<State> {
    let id = number(reply, "id")? as u32;
    let pressed: Vec<Button> = reply
        .get("buttons")?
        .array()?
        .iter()
        .filter_map(|b| button_from_name(b.as_str()?))
        .collect();
    let mut axes = [0.0f32; 6];
    for axis in Axis::ALL {
        if let Some(v) = reply.get("axes").and_then(|a| number(a, axis_name(axis))) {
            axes[axis as usize] = v as f32;
        }
    }
    let dpad = reply.get("dpad").map_or((0, 0), |d| (number(d, "x").unwrap_or(0.0) as i8, number(d, "y").unwrap_or(0.0) as i8));
    Some(State { id, pressed, axes, dpad })
}

/// Decodes a service event for the `gamepad` service. Returns `None` for another service or an unknown event, so an
/// app can pass every `App::service_event` through it.
pub fn event(service: &str, event: &str, body: &Json) -> Option<Event> {
    if service != SERVICE {
        return None;
    }
    match event {
        "connected" => parse_controller(body).map(Event::Connected),
        "disconnected" => parse_controller(body).map(Event::Disconnected),
        "input" => {
            let controller = number(body, "id")? as u32;
            let change = match body.get("kind")?.as_str()? {
                "button" => Change::Button {
                    button: button_from_name(body.get("name")?.as_str()?)?,
                    pressed: matches!(body.get("pressed"), Some(Json::Bool(true))),
                },
                "axis" => Change::Axis {
                    axis: axis_from_name(body.get("name")?.as_str()?)?,
                    value: number(body, "value")? as f32,
                },
                "dpad" => Change::Dpad { x: number(body, "x")? as i8, y: number(body, "y")? as i8 },
                _ => return None,
            };
            Some(Event::Input { controller, change })
        }
        _ => None,
    }
}

fn parse_controller(v: &Json) -> Option<Controller> {
    Some(Controller {
        id: number(v, "id")? as u32,
        name: v.get("name")?.as_str()?.to_string(),
        path: v.get("path")?.as_str()?.to_string(),
    })
}

fn number(v: &Json, key: &str) -> Option<f64> {
    v.get(key)?.number()
}

fn axis_name(axis: Axis) -> &'static str {
    springboard::shell::services::gamepad_service::axis_name(axis)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn obj(fields: &[(&str, Json)]) -> Json {
        Json::Obj(fields.iter().map(|(k, v)| ((*k).to_string(), v.clone())).collect())
    }

    #[test]
    fn a_connect_event_decodes_the_controller() {
        let body = obj(&[("id", Json::Num(2.0)), ("name", Json::Str("Pad".into())), ("path", Json::Str("/dev/input/event5".into()))]);
        assert_eq!(
            event(SERVICE, "connected", &body),
            Some(Event::Connected(Controller { id: 2, name: "Pad".into(), path: "/dev/input/event5".into() }))
        );
        assert!(matches!(event(SERVICE, "disconnected", &body), Some(Event::Disconnected(c)) if c.id == 2));
    }

    #[test]
    fn input_events_decode_buttons_axes_and_the_direction_pad() {
        let press = obj(&[
            ("id", Json::Num(1.0)),
            ("kind", Json::Str("button".into())),
            ("name", Json::Str("south".into())),
            ("pressed", Json::Bool(true)),
        ]);
        assert_eq!(
            event(SERVICE, "input", &press),
            Some(Event::Input { controller: 1, change: Change::Button { button: Button::South, pressed: true } })
        );
        let stick = obj(&[
            ("id", Json::Num(1.0)),
            ("kind", Json::Str("axis".into())),
            ("name", Json::Str("leftX".into())),
            ("value", Json::Num(-0.5)),
        ]);
        assert_eq!(
            event(SERVICE, "input", &stick),
            Some(Event::Input { controller: 1, change: Change::Axis { axis: Axis::LeftX, value: -0.5 } })
        );
        let hat = obj(&[("id", Json::Num(1.0)), ("kind", Json::Str("dpad".into())), ("x", Json::Num(0.0)), ("y", Json::Num(-1.0))]);
        assert_eq!(event(SERVICE, "input", &hat), Some(Event::Input { controller: 1, change: Change::Dpad { x: 0, y: -1 } }));
    }

    #[test]
    fn other_services_and_unknown_events_decode_to_nothing() {
        let body = obj(&[("id", Json::Num(1.0))]);
        assert_eq!(event("dock", "input", &body), None);
        assert_eq!(event(SERVICE, "rumble", &body), None);
        let unknown_element = obj(&[("id", Json::Num(1.0)), ("kind", Json::Str("button".into())), ("name", Json::Str("triangle".into()))]);
        assert_eq!(event(SERVICE, "input", &unknown_element), None);
    }

    #[test]
    fn a_state_reply_parses_into_buttons_axes_and_the_pad() {
        let reply = obj(&[
            ("id", Json::Num(3.0)),
            ("buttons", Json::Arr(vec![Json::Str("start".into()), Json::Str("home".into())])),
            ("axes", obj(&[("leftX", Json::Num(0.25)), ("rightTrigger", Json::Num(1.0))])),
            ("dpad", obj(&[("x", Json::Num(1.0)), ("y", Json::Num(0.0))])),
        ]);
        let s = parse_state(&reply).unwrap();
        assert_eq!(s.id, 3);
        assert!(s.is_pressed(Button::Start) && s.is_pressed(Button::Home) && !s.is_pressed(Button::South));
        assert_eq!(s.axis(Axis::LeftX), 0.25);
        assert_eq!(s.axis(Axis::RightTrigger), 1.0);
        assert_eq!(s.axis(Axis::RightX), 0.0);
        assert_eq!(s.dpad, (1, 0));
        assert_eq!(parse_state(&Json::Null), None);
    }

    #[test]
    fn a_rumble_call_carries_the_controller_levels_and_duration() {
        let body = rumble_body(2, 1.5, 0.25, 300);
        assert_eq!(body.get("id").and_then(|v| v.number()), Some(2.0));
        assert_eq!(body.get("strong").and_then(|v| v.number()), Some(1.0));
        assert_eq!(body.get("weak").and_then(|v| v.number()), Some(0.25));
        assert_eq!(body.get("duration").and_then(|v| v.number()), Some(300.0));
    }

    #[test]
    fn a_list_reply_skips_malformed_entries() {
        let good = obj(&[("id", Json::Num(1.0)), ("name", Json::Str("A".into())), ("path", Json::Str("/dev/input/event4".into()))]);
        let reply = Json::Arr(vec![good, Json::Num(5.0)]);
        let list = parse_list(&reply);
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].name, "A");
        assert!(parse_list(&Json::Null).is_empty());
    }
}
