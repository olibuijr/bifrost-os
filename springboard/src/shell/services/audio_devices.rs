//! The `audio-devices` service: the output and input devices the shell can see, the default device for each role,
//! the output volume and mute, and route-change events.
//!
//! The devices come from the shell's own audio state (the output route from the headset jack, the connected
//! Bluetooth audio device, and the capture devices the shell found in `/proc/asound/pcm`). The default device per role
//! is kept by `bifrost_ui::default_device::Defaults`, so a chosen default survives the device leaving and coming back.
//! The volume is the Control Center level and mute, read through `bifrost_ui::volume_control::VolumeControl`.
//! Sound is still routed by the sound server; this service reports and sets the state, it does not route samples.
//!
//! Methods (JSON params):
//! - `list` -> `{devices: [{id, name, input, output, sound_effects}], defaults: {input, output, sound_effects}}`
//! - `set_default` `{role, id}` and `clear_default` `{role}`; role is `input`, `output` or `sound_effects`
//! - `volume` -> `{level, muted, gain}`; `set_volume` `{level}` (0 to 1); `set_muted` `{muted}`
//!
//! Events (to every app): `devices_changed` `{devices: [id]}` when the device list changes, and `route_changed`
//! `{route, output}` when the output route or the default output changes, and `volume_changed` `{level, muted, gain}`
//! when the output level or mute changes (from any control, the Control Center included).
use super::{Event, Shell, SystemService};
use crate::media::route::Route;
use bifrost_core::json::Json;
use bifrost_ui::default_device::{Defaults, Device, DeviceError, Role};
use bifrost_ui::route_picker::Destination;
use bifrost_ui::route_prediction::{suggest, RouteHistory};
use bifrost_ui::volume_control::VolumeControl;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

pub const HEADPHONES: &str = "headphones";
pub const BLUETOOTH: &str = "bluetooth";
pub const SPEAKER: &str = "speaker";
pub const MICROPHONE: &str = "microphone";

/// The shell clock `t` of a service tick is in milliseconds.
/// How often the capture devices are looked up again: a microphone appearing shows within 10 s; a faster poll was the
/// audio thread's idle wake-up (round 75).
const PROBE_EVERY_MS: f64 = 10_000.0;
/// While the audio server is not there, a new connection is tried after this long, then twice as long after each
/// failure, up to `RECONNECT_MAX_MS`.
const RECONNECT_FIRST_MS: f64 = 1_000.0;
const RECONNECT_MAX_MS: f64 = 30_000.0;

pub struct AudioDevices {
    defaults: Defaults,
    /// Published by the probe worker; the UI thread only reads it and never waits on the audio server.
    microphone: Arc<AtomicBool>,
    seen: Option<Seen>,
    /// Started on the first tick; owns the connection to the audio server.
    worker: Option<std::thread::JoinHandle<()>>,
}

/// The wait before the next connection attempt: 1 s, doubling after each failure up to 30 s, 1 s again after a good one.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Backoff(f64);

impl Backoff {
    fn new() -> Self {
        Backoff(RECONNECT_FIRST_MS)
    }

    /// The wait to use now, then doubles it.
    fn fail(&mut self) -> f64 {
        let wait = self.0;
        self.0 = (self.0 * 2.0).min(RECONNECT_MAX_MS);
        wait
    }

    fn reset(&mut self) {
        self.0 = RECONNECT_FIRST_MS;
    }
}

/// How long the worker waits for the audio server to accept and answer Hello (never the UI thread).
const CONNECT_LIMIT: std::time::Duration = std::time::Duration::from_millis(100);

/// The probe worker: keeps one connection, asks for the default input every `PROBE_EVERY_MS`, reconnects after a
/// backoff, and publishes the answer. It ends when the service is dropped (`flag` has no other owner).
fn probe_loop(flag: std::sync::Weak<AtomicBool>) {
    let mut system: Option<bifrost_audio_hal::System> = None;
    let mut backoff = Backoff::new();
    let mut retry_at = std::time::Instant::now();
    loop {
        let Some(flag) = flag.upgrade() else { return };
        if system.is_none() && std::time::Instant::now() >= retry_at {
            system = bifrost_audio_hal::System::connect_within(crate::media::capture::CLIENT, CONNECT_LIMIT).ok();
            if system.is_some() {
                backoff.reset();
            } else {
                retry_at = std::time::Instant::now() + std::time::Duration::from_millis(backoff.fail() as u64);
            }
        }
        let has_input = match system.as_ref().map(|s| s.default_device(bifrost_audio_hal::Role::Input)) {
            None => false,
            Some(Ok(_)) => true,
            // no input device: the server is there, keep the connection
            Some(Err(bifrost_audio_types::AudioError::UnknownObject)) => false,
            // the server went away or answers nonsense: reconnect after a backoff
            Some(Err(_)) => {
                system = None;
                retry_at = std::time::Instant::now() + std::time::Duration::from_millis(backoff.fail() as u64);
                false
            }
        };
        flag.store(has_input, Ordering::Release);
        drop(flag);
        std::thread::sleep(std::time::Duration::from_millis(PROBE_EVERY_MS as u64));
    }
}

/// What the last tick saw, to tell the next tick what changed.
#[derive(Clone, Debug, PartialEq)]
struct Seen {
    ids: Vec<String>,
    output: Option<String>,
    route: Route,
    level: f32,
    muted: bool,
}

impl AudioDevices {
    pub fn new() -> Self {
        AudioDevices { defaults: Defaults::new(), microphone: Arc::new(AtomicBool::new(false)), seen: None, worker: None }
    }

    /// The devices present now, with the chosen defaults kept. The microphone comes from the probe worker's last
    /// answer; nothing here talks to the audio server.
    fn state(&mut self, shell: &Shell, _t: f64) -> Vec<Device> {
        if self.worker.is_none() {
            let weak = Arc::downgrade(&self.microphone);
            self.worker = std::thread::Builder::new().name("audio-probe".into()).spawn(move || probe_loop(weak)).ok();
        }
        let bluetooth = shell.info.bt.audio_sink().map(str::to_string);
        let want = present(shell.cc.route, bluetooth.as_deref(), self.microphone.load(Ordering::Acquire));
        sync(&mut self.defaults, &want);
        want
    }

    fn seen(&mut self, shell: &Shell, t: f64) -> Seen {
        let devices = self.state(shell, t);
        Seen {
            ids: devices.iter().map(|d| d.id.clone()).collect(),
            output: self.defaults.default_for(Role::Output).map(|d| d.id.clone()),
            route: shell.cc.route,
            level: shell.volume(),
            muted: shell.music_muted,
        }
    }
}

impl SystemService for AudioDevices {
    fn name(&self) -> &'static str {
        "audio-devices"
    }

    fn call(&mut self, shell: &mut Shell, _app_id: &str, method: &str, params: &Json, t: f64) -> Result<Json, String> {
        match method {
            "list" => {
                let devices = self.state(shell, t);
                let mut list = list_json(&devices, &self.defaults);
                if let Json::Obj(fields) = &mut list {
                    let suggested = suggested_outputs(&devices, &load_history(&history_path()), SUGGESTIONS).into_iter().map(Json::Str).collect();
                    fields.push(("suggested".to_string(), Json::Arr(suggested)));
                }
                Ok(list)
            }
            "set_default" => {
                let role = role_arg(params)?;
                let id = params.str_field("id").ok_or_else(|| "set_default needs an id".to_string())?;
                self.state(shell, t);
                self.defaults.set_default(role, &id).map_err(device_error)?;
                if matches!(role, Role::Output) {
                    // the choice feeds the suggestions; a history that cannot be saved does not undo the choice
                    let path = history_path();
                    let mut history = load_history(&path);
                    if history.record(&id) {
                        let _ = save_history(&path, &history);
                    }
                }
                Ok(Json::Null)
            }
            "clear_default" => {
                let role = role_arg(params)?;
                self.defaults.clear_default(role);
                Ok(Json::Null)
            }
            "volume" => Ok(volume_json(shell)),
            "set_volume" => {
                let level = params.get("level").and_then(|v| v.number()).ok_or_else(|| "set_volume needs a level from 0 to 1".to_string())?;
                let mut control = VolumeControl::new();
                control.set_level(level as f32).map_err(|_| "level must be a finite number".to_string())?;
                shell.set_volume(control.level());
                Ok(volume_json(shell))
            }
            "set_muted" => {
                let muted = match params.get("muted") {
                    Some(Json::Bool(b)) => *b,
                    _ => return Err("set_muted needs a boolean muted".to_string()),
                };
                shell.set_music_muted(muted);
                Ok(volume_json(shell))
            }
            _ => Err(format!("audio-devices has no method {method}")),
        }
    }

    fn tick(&mut self, shell: &mut Shell, t: f64) -> Vec<Event> {
        let now = self.seen(shell, t);
        let out = diff(self.seen.as_ref(), &now);
        self.seen = Some(now);
        out
    }
}

/// The devices present for this state, in list order. The first device that may take a role is the automatic
/// default, so the headset comes first, then a Bluetooth audio device, then the speaker.
pub fn present(route: Route, bluetooth: Option<&str>, microphone: bool) -> Vec<Device> {
    let mut out = Vec::new();
    if route == Route::Headphones {
        out.push(device(HEADPHONES, "Headphones", false, true, true));
    }
    if let Some(name) = bluetooth {
        out.push(device(BLUETOOTH, name, false, true, true));
    }
    out.push(device(SPEAKER, "Speaker", false, true, true));
    if microphone {
        out.push(device(MICROPHONE, "Microphone", true, false, false));
    }
    out
}

/// How many suggested outputs the list reports.
const SUGGESTIONS: usize = 3;

/// The output choice history: the outputs the picker chose, one identifier per line, oldest first.
fn history_path() -> std::path::PathBuf {
    crate::privd_client::user_data_dir().join("audio").join("route-history.txt")
}

/// The stored history; a missing or unreadable file is an empty history.
fn load_history(path: &std::path::Path) -> RouteHistory {
    std::fs::read_to_string(path).map(|text| RouteHistory::parse(&text)).unwrap_or_default()
}

fn save_history(path: &std::path::Path, history: &RouteHistory) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let temp = path.with_extension("tmp");
    std::fs::write(&temp, history.to_text())?;
    std::fs::rename(&temp, path)
}

/// The output devices listed now that the person chose most often (then most recently), at most `limit` of them.
pub fn suggested_outputs(devices: &[Device], history: &RouteHistory, limit: usize) -> Vec<String> {
    let outputs: Vec<Destination> = devices
        .iter()
        .filter(|d| d.can_output)
        .map(|d| Destination { id: d.id.clone(), name: d.name.clone(), local: true, active: false })
        .collect();
    suggest(&outputs, &history.ids(), limit).into_iter().map(|d| d.id.clone()).collect()
}

fn device(id: &str, name: &str, input: bool, output: bool, sound_effects: bool) -> Device {
    Device { id: id.into(), name: name.into(), can_input: input, can_output: output, can_sound_effects: sound_effects, hidden: false }
}

/// Makes the list of `defaults` equal to `want`, in that order. Removing a device keeps its chosen default, so a
/// device that goes away and comes back is the default again.
fn sync(defaults: &mut Defaults, want: &[Device]) {
    let listed: Vec<String> = defaults.devices().iter().map(|d| d.id.clone()).collect();
    for id in listed {
        let _ = defaults.remove(&id);
    }
    for d in want {
        let _ = defaults.add(d.clone());
    }
}

fn diff(prev: Option<&Seen>, now: &Seen) -> Vec<Event> {
    let prev = match prev {
        Some(p) => p,
        // the first look only records the state; nothing changed yet for an app to hear about
        None => return Vec::new(),
    };
    let mut out = Vec::new();
    if prev.ids != now.ids {
        let ids = now.ids.iter().map(|s| Json::Str(s.clone())).collect();
        out.push(event("devices_changed", obj(vec![("devices", Json::Arr(ids))])));
    }
    if prev.route != now.route || prev.output != now.output {
        let body = obj(vec![("route", Json::Str(now.route.label().into())), ("output", opt_str(now.output.as_deref()))]);
        out.push(event("route_changed", body));
    }
    if prev.level != now.level || prev.muted != now.muted {
        out.push(event("volume_changed", volume_view(now.level, now.muted)));
    }
    out
}

fn event(name: &str, body: Json) -> Event {
    Event { app: None, event: name.to_string(), body }
}

fn list_json(devices: &[Device], defaults: &Defaults) -> Json {
    let items = devices
        .iter()
        .map(|d| {
            obj(vec![
                ("id", Json::Str(d.id.clone())),
                ("name", Json::Str(d.name.clone())),
                ("input", Json::Bool(d.can_input)),
                ("output", Json::Bool(d.can_output)),
                ("sound_effects", Json::Bool(d.can_sound_effects)),
            ])
        })
        .collect();
    let chosen = |role: Role| opt_str(defaults.default_for(role).map(|d| d.id.as_str()));
    obj(vec![
        ("devices", Json::Arr(items)),
        (
            "defaults",
            obj(vec![("input", chosen(Role::Input)), ("output", chosen(Role::Output)), ("sound_effects", chosen(Role::SoundEffects))]),
        ),
    ])
}

fn volume_json(shell: &Shell) -> Json {
    volume_view(shell.volume(), shell.music_muted)
}

/// The volume view for a level and a mute flag: `gain` is 0 while muted.
pub fn volume_view(level: f32, muted: bool) -> Json {
    let mut control = VolumeControl::new();
    let _ = control.set_level(level);
    control.set_muted(muted);
    obj(vec![
        ("level", Json::Num(control.level() as f64)),
        ("muted", Json::Bool(control.is_muted())),
        ("gain", Json::Num(control.effective() as f64)),
    ])
}

pub fn role_of(name: &str) -> Result<Role, String> {
    match name {
        "input" => Ok(Role::Input),
        "output" => Ok(Role::Output),
        "sound_effects" => Ok(Role::SoundEffects),
        _ => Err(format!("unknown role {name}")),
    }
}

fn role_arg(params: &Json) -> Result<Role, String> {
    let name = params.str_field("role").ok_or_else(|| "a role is needed: input, output or sound_effects".to_string())?;
    role_of(&name)
}

fn device_error(e: DeviceError) -> String {
    match e {
        DeviceError::Empty => "the device id is empty".into(),
        DeviceError::Duplicate => "the device is listed twice".into(),
        DeviceError::NotFound => "no such audio device".into(),
        DeviceError::NotEligible => "that device cannot take this role".into(),
    }
}

fn obj(pairs: Vec<(&str, Json)>) -> Json {
    Json::Obj(pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
}

fn opt_str(s: Option<&str>) -> Json {
    match s {
        Some(s) => Json::Str(s.to_string()),
        None => Json::Null,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_reconnect_wait_doubles_to_30_s_and_resets() {
        let mut b = Backoff::new();
        let waits: Vec<f64> = (0..8).map(|_| b.fail()).collect();
        assert_eq!(waits, vec![1000.0, 2000.0, 4000.0, 8000.0, 16000.0, 30000.0, 30000.0, 30000.0]);
        b.reset();
        assert_eq!(b.fail(), 1000.0);
    }

    #[test]
    fn a_new_service_has_no_connection_on_the_ui_thread() {
        // the UI side only reads an atomic; the worker starts on the first tick
        let d = AudioDevices::new();
        assert!(!d.microphone.load(Ordering::Acquire));
        assert!(d.worker.is_none());
    }


    #[test]
    fn the_most_chosen_listed_outputs_are_suggested() {
        let devices = vec![
            device(SPEAKER, "Speaker", false, true, false),
            device(HEADPHONES, "Headphones", false, true, false),
            device(MICROPHONE, "Microphone", true, false, false),
        ];
        // bluetooth is not listed now, and the microphone is not an output
        let history = RouteHistory::parse("headphones\nbluetooth\nheadphones\nmicrophone\n");
        assert_eq!(suggested_outputs(&devices, &history, 3), vec!["headphones".to_string()]);
        // equal use: the most recent choice first
        let tied = RouteHistory::parse("speaker\nheadphones\n");
        assert_eq!(suggested_outputs(&devices, &tied, 1), vec!["headphones".to_string()]);
    }

    #[test]
    fn the_choice_history_round_trips_through_its_file() {
        let dir = std::env::temp_dir().join(format!("audio-history-{}", std::process::id()));
        let path = dir.join("route-history.txt");
        assert!(load_history(&path).ids().is_empty(), "a missing file is an empty history");
        let mut history = RouteHistory::new();
        history.record(SPEAKER);
        history.record(HEADPHONES);
        save_history(&path, &history).unwrap();
        assert_eq!(load_history(&path).ids(), vec![SPEAKER, HEADPHONES]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn ids(list: &[Device]) -> Vec<&str> {
        list.iter().map(|d| d.id.as_str()).collect()
    }

    #[test]
    fn the_speaker_is_always_listed_and_the_headset_comes_first_when_plugged() {
        assert_eq!(ids(&present(Route::Speaker, None, false)), vec![SPEAKER]);
        assert_eq!(ids(&present(Route::Headphones, None, false)), vec![HEADPHONES, SPEAKER]);
        assert_eq!(ids(&present(Route::Headphones, Some("Buds"), true)), vec![HEADPHONES, BLUETOOTH, SPEAKER, MICROPHONE]);
    }

    #[test]
    fn the_microphone_is_an_input_only_device() {
        let list = present(Route::Speaker, None, true);
        let mic = list.iter().find(|d| d.id == MICROPHONE).unwrap();
        assert!(mic.can_input && !mic.can_output && !mic.can_sound_effects);
        let speaker = list.iter().find(|d| d.id == SPEAKER).unwrap();
        assert!(!speaker.can_input && speaker.can_output);
    }

    #[test]
    fn sync_keeps_a_chosen_default_while_its_device_is_away() {
        let mut d = Defaults::new();
        sync(&mut d, &present(Route::Speaker, None, false));
        d.set_default(Role::Output, SPEAKER).unwrap();
        sync(&mut d, &present(Route::Headphones, None, false));
        assert_eq!(d.default_for(Role::Output).map(|x| x.id.as_str()), Some(SPEAKER));
        sync(&mut d, &present(Route::Speaker, None, false));
        assert_eq!(ids(d.devices()), vec![SPEAKER]);
    }

    #[test]
    fn sync_puts_the_headset_first_so_it_is_the_automatic_output() {
        let mut d = Defaults::new();
        sync(&mut d, &present(Route::Speaker, None, false));
        assert_eq!(d.default_for(Role::Output).map(|x| x.id.as_str()), Some(SPEAKER));
        sync(&mut d, &present(Route::Headphones, None, false));
        assert_eq!(d.default_for(Role::Output).map(|x| x.id.as_str()), Some(HEADPHONES));
    }

    #[test]
    fn a_connected_bluetooth_audio_device_takes_the_output_default() {
        let mut d = Defaults::new();
        sync(&mut d, &present(Route::Speaker, None, false));
        assert_eq!(d.set_default(Role::Output, BLUETOOTH), Err(DeviceError::NotFound), "not connected, so not listed");
        sync(&mut d, &present(Route::Speaker, Some("Buds"), false));
        d.set_default(Role::Output, BLUETOOTH).unwrap();
        assert_eq!(d.default_for(Role::Output).map(|x| x.id.as_str()), Some(BLUETOOTH));
    }

    #[test]
    fn a_microphone_is_never_the_output_default() {
        let mut d = Defaults::new();
        sync(&mut d, &present(Route::Speaker, None, true));
        assert_eq!(d.default_for(Role::Input).map(|x| x.id.as_str()), Some(MICROPHONE));
        assert_eq!(d.default_for(Role::Output).map(|x| x.id.as_str()), Some(SPEAKER));
        assert_eq!(d.set_default(Role::Output, MICROPHONE), Err(DeviceError::NotEligible));
    }

    #[test]
    fn roles_are_named_as_the_wire_names_them() {
        assert_eq!(role_of("input"), Ok(Role::Input));
        assert_eq!(role_of("output"), Ok(Role::Output));
        assert_eq!(role_of("sound_effects"), Ok(Role::SoundEffects));
        assert!(role_of("speakers").is_err());
    }

    #[test]
    fn the_volume_view_reports_zero_gain_while_muted_and_keeps_the_level() {
        let on = volume_view(0.4, false);
        assert_eq!(on.get("gain"), Some(&Json::Num(0.4f32 as f64)));
        let muted = volume_view(0.4, true);
        assert_eq!(muted.get("muted"), Some(&Json::Bool(true)));
        assert_eq!(muted.get("gain"), Some(&Json::Num(0.0)));
        assert_eq!(muted.get("level"), Some(&Json::Num(0.4f32 as f64)));
    }

    #[test]
    fn a_route_change_and_a_device_change_are_each_one_event() {
        let before = Seen { ids: vec![SPEAKER.into()], output: Some(SPEAKER.into()), route: Route::Speaker, level: 0.5, muted: false };
        let after = Seen {
            ids: vec![HEADPHONES.into(), SPEAKER.into()],
            output: Some(HEADPHONES.into()),
            route: Route::Headphones,
            level: 0.5,
            muted: false,
        };
        assert!(diff(None, &after).is_empty());
        assert!(diff(Some(&before), &before).is_empty());
        let events = diff(Some(&before), &after);
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].event, "devices_changed");
        assert_eq!(events[1].event, "route_changed");
        assert_eq!(events[1].body.get("route"), Some(&Json::Str("Headphones".into())));
        assert_eq!(events[1].body.get("output"), Some(&Json::Str(HEADPHONES.into())));
        assert!(events.iter().all(|e| e.app.is_none()));
    }

    #[test]
    fn a_volume_or_mute_change_is_one_volume_event_with_the_new_view() {
        let before = Seen { ids: vec![SPEAKER.into()], output: Some(SPEAKER.into()), route: Route::Speaker, level: 0.5, muted: false };
        let louder = Seen { level: 0.75, ..before.clone() };
        let muted = Seen { muted: true, ..louder.clone() };
        let events = diff(Some(&before), &louder);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event, "volume_changed");
        assert_eq!(events[0].body.get("level"), Some(&Json::Num(0.75)));
        assert_eq!(events[0].body.get("gain"), Some(&Json::Num(0.75)));
        let events = diff(Some(&louder), &muted);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].body.get("muted"), Some(&Json::Bool(true)));
        assert_eq!(events[0].body.get("gain"), Some(&Json::Num(0.0)));
        assert!(diff(Some(&muted), &muted).is_empty());
    }

    #[test]
    fn the_list_names_each_device_and_the_chosen_default_per_role() {
        let mut d = Defaults::new();
        let want = present(Route::Headphones, None, false);
        sync(&mut d, &want);
        let json = list_json(&want, &d);
        let items = json.get("devices").and_then(|v| v.as_array()).unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].get("id"), Some(&Json::Str(HEADPHONES.into())));
        assert_eq!(items[1].get("sound_effects"), Some(&Json::Bool(true)));
        let defaults = json.get("defaults").unwrap();
        assert_eq!(defaults.get("output"), Some(&Json::Str(HEADPHONES.into())));
        assert_eq!(defaults.get("input"), Some(&Json::Null));
    }

    #[test]
    fn device_errors_have_readable_messages() {
        assert_eq!(device_error(DeviceError::NotFound), "no such audio device");
        assert_eq!(device_error(DeviceError::NotEligible), "that device cannot take this role");
    }
}
