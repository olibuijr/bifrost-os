//! The `pip` service: a player's Picture in Picture window (`shell/pip.rs`). A player first says that it can show a
//! window (`possible {on}`), then asks for the window (`start`) and takes it down (`stop`). Each change sends the
//! controller's delegate events back to that player on the next tick, as `pip.will_start`, `pip.did_start`,
//! `pip.will_stop` and `pip.did_stop`.
//!
//! Only the window's life cycle is here: a player that has not said it is possible gets no window.

use super::{Event, Shell, SystemService};
use bifrost_core::json::Json;
use bifrost_ui::pip_window::Event as PipEvent;

/// The service name the shell registers.
pub const SERVICE: &str = "pip";

/// The event name a delegate event goes out under.
pub fn event_name(e: PipEvent) -> &'static str {
    match e {
        PipEvent::WillStart => "pip.will_start",
        PipEvent::DidStart => "pip.did_start",
        PipEvent::WillStop => "pip.will_stop",
        PipEvent::DidStop => "pip.did_stop",
    }
}

/// The delegate events waiting for their player, delivered by the next tick.
#[derive(Default)]
pub struct PictureInPicture {
    pending: Vec<Event>,
}

impl PictureInPicture {
    fn queue(&mut self, app: &str, events: Vec<PipEvent>) {
        self.pending.extend(events.into_iter().map(|e| Event { app: Some(app.to_string()), event: event_name(e).to_string(), body: Json::Obj(vec![]) }));
    }
}

impl SystemService for PictureInPicture {
    fn name(&self) -> &'static str {
        SERVICE
    }

    fn call(&mut self, shell: &mut Shell, app_id: &str, method: &str, params: &Json, _t: f64) -> Result<Json, String> {
        let events = match method {
            "possible" => {
                let on = match params.get("on") {
                    Some(Json::Bool(on)) => *on,
                    _ => return Err("possible needs on: true or false".into()),
                };
                shell.pip_possible(on)
            }
            "start" => shell.pip_start(),
            "stop" => shell.pip_stop(),
            _ => return Err(format!("unknown pip method {method}")),
        };
        self.queue(app_id, events);
        Ok(Json::Obj(vec![]))
    }

    fn tick(&mut self, _shell: &mut Shell, _t: f64) -> Vec<Event> {
        std::mem::take(&mut self.pending)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_delegate_event_has_its_name() {
        assert_eq!(event_name(PipEvent::WillStart), "pip.will_start");
        assert_eq!(event_name(PipEvent::DidStart), "pip.did_start");
        assert_eq!(event_name(PipEvent::WillStop), "pip.will_stop");
        assert_eq!(event_name(PipEvent::DidStop), "pip.did_stop");
    }

    #[test]
    fn queued_events_go_to_the_player_that_asked() {
        let mut s = PictureInPicture::default();
        s.queue("is.example.player", vec![PipEvent::WillStart, PipEvent::DidStart]);
        assert_eq!(s.pending.len(), 2);
        assert_eq!(s.pending[0].app.as_deref(), Some("is.example.player"));
        assert_eq!(s.pending[1].event, "pip.did_start");
        assert!(std::mem::take(&mut s.pending).len() == 2 && s.pending.is_empty(), "taken once");
    }
}
