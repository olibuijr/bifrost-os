//! Glue between Voice Memos and the shell's system services: the microphone permission (`privacy`), the audio
//! devices (`audio-devices`) and Now Playing (`now_playing`).
//!
//! The model never calls a service itself. It queues a [`Want`] when a real event happens (a Record press, a route
//! change, playback starting or stopping), the app sends queued wants with [`Bus::send`], and each reply comes back
//! through [`Bus::take`], which says which question it answers. A service that is missing (an older shell answers
//! "no service ...") gives an error reply, and the app keeps working with the state it had.

use bifrost_app::audio_devices;
use bifrost_app::controls::capture_access::{Media, Status};
use bifrost_app::now_playing;
use bifrost_app::privacy_permissions;
use bifrost_app::service;
use bifrost_app::Ctx;
use bifrost_core::json::Json;

/// Calls the model wants the app to make.
pub enum Want {
    /// Ask the privacy service for the microphone status.
    MicStatus,
    /// Ask the audio-devices service for the device list and the defaults.
    ListDevices,
    /// Publish the playing recording to the media tile (the body from `now_playing::publish_body`).
    Publish(Json),
    /// Withdraw this app's session from the media tile.
    Withdraw,
}

/// The question a service reply answers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ask {
    Mic,
    Devices,
}

/// The replies still expected. Publish and withdraw need no reply, so they are not tracked.
#[derive(Default)]
pub struct Bus {
    asks: Vec<(u32, Ask)>,
}

/// How many questions may wait for a reply; the oldest is dropped beyond this.
const MAX_ASKS: usize = 16;

impl Bus {
    /// Sends one want to the shell.
    pub fn send(&mut self, ctx: &Ctx, want: Want) {
        let id = match want {
            Want::MicStatus => Some((privacy_permissions::status(ctx, Media::Audio), Ask::Mic)),
            Want::ListDevices => Some((audio_devices::list(ctx), Ask::Devices)),
            Want::Publish(body) => {
                service::call(ctx, now_playing::SERVICE, "publish", &body);
                None
            }
            Want::Withdraw => {
                now_playing::clear(ctx);
                None
            }
        };
        if let Some(entry) = id {
            if self.asks.len() >= MAX_ASKS {
                self.asks.remove(0);
            }
            self.asks.push(entry);
        }
    }

    /// The question `request` answers, if this app asked it. Each question is answered once.
    pub fn take(&mut self, request: u32) -> Option<Ask> {
        let at = self.asks.iter().position(|(id, _)| *id == request)?;
        Some(self.asks.remove(at).1)
    }
}

/// Refuses a recording the microphone permission does not allow. A status not answered yet (or unknown because the
/// privacy service is missing) allows it, the same as before the services existed.
pub fn record_gate(mic: Option<Status>) -> Result<(), &'static str> {
    match mic {
        Some(Status::Denied) => Err("Microphone access is off. Turn it on in Settings > Privacy > App Permissions."),
        Some(Status::Restricted) => Err("Microphone access is restricted on this PC."),
        _ => Ok(()),
    }
}

/// The line under the record button: a change notice first, otherwise the microphone state and the input device.
pub fn caption(mic: Option<Status>, input: Option<&str>, notice: &str) -> String {
    if !notice.is_empty() {
        return notice.to_string();
    }
    match mic {
        None => String::new(),
        Some(Status::Authorized) => match input {
            Some(name) => format!("Microphone on. Input: {name}"),
            None => "Microphone on".into(),
        },
        Some(Status::NotDetermined) => "Microphone access is not answered yet".into(),
        Some(Status::Denied) => "Microphone access is off".into(),
        Some(Status::Restricted) => "Microphone access is restricted on this PC".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_app::proto::{Configure, ToShell};

    fn ctx() -> Ctx {
        let ui = springboard::shell::tests::ui();
        Ctx::testing(ui, Configure { w: 100, h: 100, scale: 1.0, safe_top: 0.0, safe_bottom: 0.0, dark: true, text_scale: 1.0, reduce_motion: false })
    }

    #[test]
    fn a_mic_want_calls_the_privacy_service_and_its_reply_is_the_mic_question() {
        let ctx = ctx();
        let mut bus = Bus::default();
        bus.send(&ctx, Want::MicStatus);
        let sent = ctx.take_sent();
        let ToShell::ServiceCall { request, service, method, body } = &sent[0] else { panic!("a service call") };
        assert_eq!((service.as_str(), method.as_str()), ("privacy", "status"));
        assert!(body.contains("microphone"), "{body}");
        assert_eq!(bus.take(*request), Some(Ask::Mic));
        assert_eq!(bus.take(*request), None, "each reply is taken once");
    }

    #[test]
    fn a_device_want_calls_audio_devices_and_publish_and_withdraw_need_no_reply() {
        let ctx = ctx();
        let mut bus = Bus::default();
        bus.send(&ctx, Want::ListDevices);
        bus.send(&ctx, Want::Withdraw);
        bus.send(&ctx, Want::Publish(Json::Obj(vec![("title".into(), Json::Str("Standup".into()))])));
        let sent = ctx.take_sent();
        assert_eq!(sent.len(), 3);
        let ToShell::ServiceCall { request, service, method, .. } = &sent[0] else { panic!("a service call") };
        assert_eq!((service.as_str(), method.as_str()), ("audio-devices", "list"));
        assert_eq!(bus.take(*request), Some(Ask::Devices));
        let ToShell::ServiceCall { service, method, .. } = &sent[1] else { panic!("a service call") };
        assert_eq!((service.as_str(), method.as_str()), ("now_playing", "clear"));
        let ToShell::ServiceCall { service, method, body, .. } = &sent[2] else { panic!("a service call") };
        assert_eq!((service.as_str(), method.as_str()), ("now_playing", "publish"));
        assert!(body.contains("Standup"), "{body}");
    }

    #[test]
    fn an_unknown_request_is_not_an_answer() {
        let mut bus = Bus::default();
        assert_eq!(bus.take(424_242), None);
    }

    #[test]
    fn a_denied_or_restricted_microphone_refuses_the_recording_and_the_rest_allows_it() {
        assert!(record_gate(Some(Status::Denied)).unwrap_err().contains("Settings"));
        assert!(record_gate(Some(Status::Restricted)).is_err());
        assert!(record_gate(Some(Status::Authorized)).is_ok());
        assert!(record_gate(Some(Status::NotDetermined)).is_ok());
        assert!(record_gate(None).is_ok(), "a missing privacy service does not block recording");
    }

    #[test]
    fn the_caption_shows_the_microphone_state_and_the_input_name() {
        assert_eq!(caption(None, None, ""), "");
        assert_eq!(caption(Some(Status::Authorized), Some("Desk mic"), ""), "Microphone on. Input: Desk mic");
        assert_eq!(caption(Some(Status::Authorized), None, ""), "Microphone on");
        assert!(caption(Some(Status::Denied), None, "").contains("off"));
        assert_eq!(caption(Some(Status::Authorized), Some("Desk mic"), "Input changed to USB mic"), "Input changed to USB mic");
    }
}
