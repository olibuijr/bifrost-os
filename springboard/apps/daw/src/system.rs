//! The DAW's links to the shell's system services (docs/parity/connect-daw.md):
//! - `audio-devices`: the output device and route. The app asks for the device list once at start, asks again when
//!   the shell says the list or the route changed, and shows the route in the status line.
//! - `now_playing`: the transport as a Control Center session. The session is published when the transport changes
//!   (play, pause, stop, the song ends) and withdrawn on stop. The tile's play, pause and stop come back as commands.
//! - `power`: Low Power Mode, read once at start and then from the change event; the app shows a warning while it is on.
//!
//! Nothing here runs per frame: a service call is made at start or on a real event, never because a frame was drawn.
//! Services an older shell does not have answer with an error, which leaves the state as it was.

use bifrost_app::audio_devices::{self, Change as DeviceChange, Listing};
use bifrost_app::now_playing::{
    self, Command, CommandState, MediaContent, MediaDuration, MediaSession, MediaType, PlaybackState, RemoteCommand,
};
use bifrost_app::power_service::{self, Change as PowerChange};
use bifrost_app::Ctx;
use bifrost_core::json::Json;

/// The Now Playing session id and the content id the DAW publishes under.
const SESSION_ID: &str = "daw";
const CONTENT_ID: &str = "song";
/// The title used when the project has no name.
const UNTITLED: &str = "Untitled Song";

/// The transport as the desktop sees it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Transport {
    Stopped,
    Playing,
    Paused,
}

impl Transport {
    fn playback(self) -> PlaybackState {
        match self {
            Transport::Stopped => PlaybackState::Stopped,
            Transport::Playing => PlaybackState::Playing,
            Transport::Paused => PlaybackState::Paused,
        }
    }
}

/// What a service event means for the app. The app applies it and redraws.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// Not one of the services this app uses, or nothing to show.
    Ignored,
    /// The output route changed; the text for the status line.
    Route(String),
    /// Low Power Mode changed; the warning is redrawn from `System::low_power`.
    Power,
    /// A transport command from the Now Playing tile.
    Command(Command),
}

/// The state the app keeps from the services.
#[derive(Default)]
pub struct System {
    started: bool,
    list_request: Option<u32>,
    power_request: Option<u32>,
    /// The last device listing the shell sent, if any.
    pub listing: Option<Listing>,
    /// True while the shell reports Low Power Mode.
    pub low_power: bool,
    /// The transport last shown to the desktop (published, or withdrawn when stopped).
    shown: Option<Transport>,
}

impl System {
    /// Asks for the device list and the Low Power Mode state. Runs once; later calls do nothing.
    pub fn start(&mut self, ctx: &Ctx) {
        if self.started {
            return;
        }
        self.started = true;
        self.list_request = Some(audio_devices::list(ctx));
        self.power_request = Some(power_service::low_power_mode(ctx));
    }

    /// The name of the default output device, from the last listing.
    pub fn output_name(&self) -> Option<String> {
        let listing = self.listing.as_ref()?;
        let id = listing.output.as_deref()?;
        listing.devices.iter().find(|d| d.id == id).map(|d| d.name.clone())
    }

    /// Takes the reply to one of this app's service calls. False for a request this module did not make.
    pub fn reply(&mut self, request: u32, result: &Result<Json, String>) -> bool {
        if self.list_request == Some(request) {
            self.list_request = None;
            if let Some(listing) = result.as_ref().ok().and_then(|v| audio_devices::decode_list(v).ok()) {
                self.listing = Some(listing);
            }
            return true;
        }
        if self.power_request == Some(request) {
            self.power_request = None;
            self.low_power = power_service::decode_low_power_mode(result).unwrap_or(false);
            return true;
        }
        false
    }

    /// Handles an event a service pushed to the app. Calls the shell only for a device or route change.
    pub fn event(&mut self, ctx: &Ctx, service: &str, event: &str, body: &Json) -> Event {
        if let Some(command) = now_playing::command_from_event(service, event, body) {
            return Event::Command(command);
        }
        if service == audio_devices::SERVICE {
            return match audio_devices::decode_event(event, body) {
                Some(DeviceChange::Route { route, .. }) => {
                    self.list_request = Some(audio_devices::list(ctx));
                    Event::Route(format!("Output: {route}"))
                }
                Some(DeviceChange::Devices(_)) => {
                    self.list_request = Some(audio_devices::list(ctx));
                    Event::Ignored
                }
                None => Event::Ignored,
            };
        }
        if service == power_service::SERVICE {
            if let Some(PowerChange::LowPowerMode(on)) = power_service::decode_change(service, event, body) {
                self.low_power = on;
                return Event::Power;
            }
        }
        Event::Ignored
    }

    /// The transport last shown to the desktop.
    pub fn shown(&self) -> Option<Transport> {
        self.shown
    }

    /// Shows the transport in Control Center when it changed since the last call: a session for play and pause, the
    /// session withdrawn for stop. `position_ms` is read only when a session is published. Does nothing when unchanged.
    pub fn sync_session(&mut self, ctx: &Ctx, transport: Transport, title: &str, position_ms: u64) {
        if self.shown == Some(transport) {
            return;
        }
        // a stopped app that never showed a session has nothing to withdraw
        if self.shown.is_none() && transport == Transport::Stopped {
            self.shown = Some(transport);
            return;
        }
        self.shown = Some(transport);
        if transport == Transport::Stopped {
            now_playing::clear(ctx);
            return;
        }
        if let Some(session) = session_for(transport, title, position_ms) {
            now_playing::publish(ctx, &session);
        }
    }
}

/// The Now Playing session for a transport: the title, the state and the commands the DAW handles (play, pause and
/// stop). A command that does not apply in the current state is disabled.
pub fn session_for(transport: Transport, title: &str, position_ms: u64) -> Option<MediaSession> {
    let title = if title.trim().is_empty() { UNTITLED } else { title };
    let mut session = MediaSession::new(SESSION_ID).ok()?;
    let content = MediaContent::new(CONTENT_ID, title, MediaType::Audio, MediaDuration::Continuous).ok()?;
    session.set_content(Some(content));
    session.set_playback(transport.playback(), position_ms);
    let (playing, stopped) = (transport == Transport::Playing, transport == Transport::Stopped);
    session.set_command(RemoteCommand::Play, if playing { CommandState::Disabled } else { CommandState::Enabled });
    session.set_command(RemoteCommand::Pause, if playing { CommandState::Enabled } else { CommandState::Disabled });
    session.set_command(RemoteCommand::Stop, if stopped { CommandState::Disabled } else { CommandState::Enabled });
    Some(session)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_app::kit::{Font, Ui};
    use bifrost_app::proto::{Configure, ToShell};

    fn test_ctx() -> Ctx {
        let data: &'static [u8] =
            Box::leak(std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/SFUI.ttf")).unwrap().into_boxed_slice());
        let cfg = Configure { w: 100, h: 100, scale: 1.0, safe_top: 0.0, safe_bottom: 0.0, dark: true, text_scale: 1.0, reduce_motion: false };
        Ctx::testing(Ui::new(Font::parse(data).unwrap(), 1.0), cfg)
    }

    fn parsed(text: &str) -> Json {
        bifrost_core::json::parse(text).unwrap()
    }

    /// The (service, method, body) of each service call the app sent, in order.
    fn calls(ctx: &Ctx) -> Vec<(String, String, Json)> {
        ctx.take_sent()
            .into_iter()
            .filter_map(|m| match m {
                ToShell::ServiceCall { service, method, body, .. } => Some((service, method, parsed(&body))),
                _ => None,
            })
            .collect()
    }

    /// The request id of the service call with `method` in `sent` (the queue is drained by `take_sent`, so pass its result).
    fn request_of(sent: &[ToShell], method: &str) -> u32 {
        sent.iter()
            .find_map(|m| match m {
                ToShell::ServiceCall { request, method: name, .. } if name == method => Some(*request),
                _ => None,
            })
            .expect("a call with that method")
    }

    #[test]
    fn start_asks_for_devices_and_low_power_once() {
        let ctx = test_ctx();
        let mut sys = System::default();
        sys.start(&ctx);
        sys.start(&ctx);
        let sent = calls(&ctx);
        assert_eq!(sent.len(), 2);
        assert_eq!(sent[0].0, "audio-devices");
        assert_eq!(sent[0].1, "list");
        assert_eq!(sent[1].0, "power");
        assert_eq!(sent[1].1, "lowPowerMode");
    }

    #[test]
    fn a_device_list_reply_sets_the_output_name() {
        let ctx = test_ctx();
        let mut sys = System::default();
        sys.start(&ctx);
        let list_id = request_of(&ctx.take_sent(), "list");
        let reply = Ok(parsed(
            r#"{"devices":[{"id":"speaker","name":"Speaker","input":false,"output":true,"sound_effects":true},
                           {"id":"headphones","name":"Headphones","input":false,"output":true,"sound_effects":true}],
                "defaults":{"output":"headphones"}}"#,
        ));
        assert!(sys.reply(list_id, &reply));
        assert_eq!(sys.output_name().as_deref(), Some("Headphones"));
    }

    #[test]
    fn an_error_reply_from_an_older_shell_leaves_the_state_alone() {
        let ctx = test_ctx();
        let mut sys = System::default();
        sys.start(&ctx);
        let sent = ctx.take_sent();
        let (list_id, power_id) = (request_of(&sent, "list"), request_of(&sent, "lowPowerMode"));
        assert!(sys.reply(list_id, &Err("no service audio-devices".into())));
        assert!(sys.reply(power_id, &Err("no service power".into())));
        assert_eq!(sys.output_name(), None);
        assert!(!sys.low_power);
    }

    #[test]
    fn a_reply_to_an_unknown_request_is_not_taken() {
        let mut sys = System::default();
        assert!(!sys.reply(4_242, &Ok(Json::Null)));
    }

    #[test]
    fn low_power_mode_reply_and_change_event_set_the_warning() {
        let ctx = test_ctx();
        let mut sys = System::default();
        sys.start(&ctx);
        let power_id = request_of(&ctx.take_sent(), "lowPowerMode");
        assert!(sys.reply(power_id, &Ok(parsed(r#"{"lowPowerMode":true}"#))));
        assert!(sys.low_power);
        let event = sys.event(&ctx, "power", "powerStateDidChange", &parsed(r#"{"lowPowerMode":false}"#));
        assert_eq!(event, Event::Power);
        assert!(!sys.low_power);
        assert!(ctx.take_sent().is_empty(), "a power change does not call the shell");
    }

    #[test]
    fn a_route_change_shows_the_route_and_asks_for_a_fresh_list() {
        let ctx = test_ctx();
        let mut sys = System::default();
        let event = sys.event(&ctx, "audio-devices", "route_changed", &parsed(r#"{"route":"Headphones","output":"headphones"}"#));
        assert_eq!(event, Event::Route("Output: Headphones".into()));
        let sent = calls(&ctx);
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].1, "list");
    }

    #[test]
    fn a_device_list_change_asks_for_a_fresh_list_without_a_status_line() {
        let ctx = test_ctx();
        let mut sys = System::default();
        let event = sys.event(&ctx, "audio-devices", "devices_changed", &parsed(r#"{"devices":["speaker"]}"#));
        assert_eq!(event, Event::Ignored);
        assert_eq!(calls(&ctx).len(), 1);
    }

    #[test]
    fn tile_commands_come_back_as_commands_and_other_events_are_ignored() {
        let ctx = test_ctx();
        let mut sys = System::default();
        assert_eq!(sys.event(&ctx, "now_playing", "command", &parsed(r#"{"command":"pause"}"#)), Event::Command(Command::Pause));
        assert_eq!(sys.event(&ctx, "now_playing", "command", &parsed(r#"{"command":"play"}"#)), Event::Command(Command::Play));
        assert_eq!(sys.event(&ctx, "dock", "command", &parsed(r#"{"command":"play"}"#)), Event::Ignored);
        assert_eq!(sys.event(&ctx, "battery", "batteryDidChange", &parsed("{}")), Event::Ignored);
        assert!(ctx.take_sent().is_empty());
    }

    #[test]
    fn playing_publishes_a_session_once_and_pausing_publishes_again() {
        let ctx = test_ctx();
        let mut sys = System::default();
        sys.sync_session(&ctx, Transport::Playing, "Demo", 0);
        sys.sync_session(&ctx, Transport::Playing, "Demo", 0);
        let sent = calls(&ctx);
        assert_eq!(sent.len(), 1, "an unchanged transport sends nothing");
        assert_eq!(sent[0].0, "now_playing");
        assert_eq!(sent[0].1, "publish");
        assert_eq!(sent[0].2.str_field("title").as_deref(), Some("Demo"));
        assert_eq!(sent[0].2.str_field("state").as_deref(), Some("playing"));
        assert_eq!(sent[0].2.get("commands").unwrap().str_field("pause").as_deref(), Some("enabled"));
        assert_eq!(sent[0].2.get("commands").unwrap().str_field("play").as_deref(), Some("disabled"));

        sys.sync_session(&ctx, Transport::Paused, "Demo", 1_500);
        let sent = calls(&ctx);
        assert_eq!(sent[0].2.str_field("state").as_deref(), Some("paused"));
        assert_eq!(sent[0].2.get("position_ms").and_then(Json::number), Some(1_500.0));
    }

    #[test]
    fn stopping_withdraws_the_session_and_a_stop_before_any_play_sends_nothing() {
        let ctx = test_ctx();
        let mut sys = System::default();
        sys.sync_session(&ctx, Transport::Stopped, "Demo", 0);
        assert!(ctx.take_sent().is_empty());
        sys.sync_session(&ctx, Transport::Playing, "Demo", 0);
        ctx.take_sent();
        sys.sync_session(&ctx, Transport::Stopped, "Demo", 0);
        let sent = calls(&ctx);
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].1, "clear");
    }

    #[test]
    fn an_empty_title_is_published_as_untitled() {
        let session = session_for(Transport::Playing, "  ", 0).expect("a session");
        assert_eq!(session.content().unwrap().title, "Untitled Song");
    }

    #[test]
    fn stop_is_disabled_when_already_stopped_and_pause_only_while_playing() {
        let stopped = session_for(Transport::Stopped, "x", 0).unwrap();
        assert_eq!(stopped.command_state(RemoteCommand::Stop), CommandState::Disabled);
        assert_eq!(stopped.command_state(RemoteCommand::Pause), CommandState::Disabled);
        let paused = session_for(Transport::Paused, "x", 0).unwrap();
        assert_eq!(paused.command_state(RemoteCommand::Play), CommandState::Enabled);
        assert_eq!(paused.command_state(RemoteCommand::Pause), CommandState::Disabled);
    }
}
