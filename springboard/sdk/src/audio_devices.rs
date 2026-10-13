//! Typed client for the `audio-devices` shell service: the output and input devices, the default device for each role,
//! the output volume and mute, and route-change events. The shell side is `springboard/src/shell/services/audio_devices.rs`.
//!
//! ```ignore
//! let id = audio_devices::list(ctx);
//! // in App::service_reply, when the request is `id`:
//! //   let listing = audio_devices::decode_list(&result?)?;  // listing.output, listing.devices
//! // in App::service_event, for service "audio-devices":
//! //   if let Some(change) = audio_devices::decode_event(event, body) { ... }
//! ```
//! The shell reports a level or mute change as [`Change::Volume`], a route or default change as [`Change::Route`] and
//! a device list change as [`Change::Devices`].
//! The calls return the request id; the reply comes back through `App::service_reply` (or `service::take_reply`).
use crate::runtime::Ctx;
use crate::service;
use bifrost_core::json::Json;
pub use bifrost_ui::default_device::{Device, Role};

/// The name the service is registered under in the shell.
pub const SERVICE: &str = "audio-devices";

/// The device list and the default device for each role. Ids name devices in `devices`.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Listing {
    pub devices: Vec<Device>,
    pub input: Option<String>,
    pub output: Option<String>,
    pub sound_effects: Option<String>,
}

impl Listing {
    /// The default device id for `role`, None when no device can take it.
    pub fn default_for(&self, role: Role) -> Option<&str> {
        match role {
            Role::Input => self.input.as_deref(),
            Role::Output => self.output.as_deref(),
            Role::SoundEffects => self.sound_effects.as_deref(),
        }
    }
}

/// The output volume: `level` is what the user set (0 to 1), `gain` is what plays (0 while muted).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Volume {
    pub level: f32,
    pub muted: bool,
    pub gain: f32,
}

/// A change the shell reports to every app.
#[derive(Clone, Debug, PartialEq)]
pub enum Change {
    /// The output route or the default output changed. `output` is None when no device can take the output role.
    Route { route: String, output: Option<String> },
    /// The device list changed; the ids of the devices now present.
    Devices(Vec<String>),
    /// The output level or mute changed; the new level, mute state and the gain that plays.
    Volume(Volume),
}

/// Lists the devices and the defaults.
pub fn list(ctx: &Ctx) -> u32 {
    service::call(ctx, SERVICE, "list", &Json::Null)
}

/// Chooses `id` as the default for `role`.
pub fn set_default(ctx: &Ctx, role: Role, id: &str) -> u32 {
    service::call(ctx, SERVICE, "set_default", &obj(vec![("role", Json::Str(role_name(role).into())), ("id", Json::Str(id.into()))]))
}

/// Returns `role` to the automatic choice.
pub fn clear_default(ctx: &Ctx, role: Role) -> u32 {
    service::call(ctx, SERVICE, "clear_default", &obj(vec![("role", Json::Str(role_name(role).into()))]))
}

/// Reads the output volume and mute.
pub fn volume(ctx: &Ctx) -> u32 {
    service::call(ctx, SERVICE, "volume", &Json::Null)
}

/// Sets the output level (0 to 1); the shell keeps it in range.
pub fn set_volume(ctx: &Ctx, level: f32) -> u32 {
    service::call(ctx, SERVICE, "set_volume", &obj(vec![("level", Json::Num(level as f64))]))
}

/// Mutes or unmutes the output. The level is kept while muted.
pub fn set_muted(ctx: &Ctx, muted: bool) -> u32 {
    service::call(ctx, SERVICE, "set_muted", &obj(vec![("muted", Json::Bool(muted))]))
}

/// The wire name of a role.
pub fn role_name(role: Role) -> &'static str {
    match role {
        Role::Input => "input",
        Role::Output => "output",
        Role::SoundEffects => "sound_effects",
    }
}

/// Reads a `list` reply.
pub fn decode_list(v: &Json) -> Result<Listing, String> {
    let items = v.get("devices").and_then(|d| d.as_array()).ok_or_else(|| "the reply has no devices".to_string())?;
    let devices = items
        .iter()
        .map(|d| -> Result<Device, String> {
            Ok(Device {
                id: d.str_field("id").ok_or_else(|| "a device has no id".to_string())?,
                name: d.str_field("name").unwrap_or_default(),
                can_input: flag(d, "input"),
                can_output: flag(d, "output"),
                can_sound_effects: flag(d, "sound_effects"),
                hidden: false,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    let defaults = v.get("defaults");
    let pick = |role: &str| defaults.and_then(|d| d.str_field(role));
    Ok(Listing { devices, input: pick("input"), output: pick("output"), sound_effects: pick("sound_effects") })
}

/// Reads a `volume` reply (or the reply to `set_volume` and `set_muted`).
pub fn decode_volume(v: &Json) -> Result<Volume, String> {
    let level = v.get("level").and_then(|x| x.number()).ok_or_else(|| "the reply has no level".to_string())?;
    let gain = v.get("gain").and_then(|x| x.number()).ok_or_else(|| "the reply has no gain".to_string())?;
    Ok(Volume { level: level as f32, muted: flag(v, "muted"), gain: gain as f32 })
}

/// Reads a service event from the shell. None for an event this client does not know.
pub fn decode_event(event: &str, body: &Json) -> Option<Change> {
    match event {
        "route_changed" => Some(Change::Route { route: body.str_field("route")?, output: body.str_field("output") }),
        "devices_changed" => {
            let ids = body.get("devices")?.as_array()?.iter().filter_map(|d| d.as_str().map(str::to_string)).collect();
            Some(Change::Devices(ids))
        }
        "volume_changed" => decode_volume(body).ok().map(Change::Volume),
        _ => None,
    }
}

fn flag(v: &Json, key: &str) -> bool {
    matches!(v.get(key), Some(Json::Bool(true)))
}

fn obj(pairs: Vec<(&str, Json)>) -> Json {
    Json::Obj(pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(text: &str) -> Json {
        bifrost_core::json::parse(text).unwrap()
    }

    #[test]
    fn a_list_reply_decodes_devices_and_defaults() {
        let v = parsed(
            r#"{"devices":[{"id":"headphones","name":"Headphones","input":false,"output":true,"sound_effects":true},
                {"id":"microphone","name":"Microphone","input":true,"output":false,"sound_effects":false}],
                "defaults":{"input":"microphone","output":"headphones","sound_effects":null}}"#,
        );
        let listing = decode_list(&v).unwrap();
        assert_eq!(listing.devices.len(), 2);
        assert_eq!(listing.devices[1].name, "Microphone");
        assert!(listing.devices[1].can_input && !listing.devices[1].can_output);
        assert_eq!(listing.default_for(Role::Output), Some("headphones"));
        assert_eq!(listing.default_for(Role::Input), Some("microphone"));
        assert_eq!(listing.default_for(Role::SoundEffects), None);
    }

    #[test]
    fn a_list_reply_without_devices_is_an_error() {
        assert!(decode_list(&parsed(r#"{"defaults":{}}"#)).is_err());
        assert!(decode_list(&parsed(r#"{"devices":[{"name":"No id"}],"defaults":{}}"#)).is_err());
    }

    #[test]
    fn a_volume_reply_decodes_level_mute_and_gain() {
        let v = decode_volume(&parsed(r#"{"level":0.25,"muted":true,"gain":0}"#)).unwrap();
        assert_eq!(v, Volume { level: 0.25, muted: true, gain: 0.0 });
        assert!(decode_volume(&parsed(r#"{"muted":false}"#)).is_err());
    }

    #[test]
    fn route_and_device_events_decode() {
        let route = parsed(r#"{"route":"Headphones","output":"headphones"}"#);
        assert_eq!(
            decode_event("route_changed", &route),
            Some(Change::Route { route: "Headphones".into(), output: Some("headphones".into()) })
        );
        let none = parsed(r#"{"route":"Speaker","output":null}"#);
        assert_eq!(decode_event("route_changed", &none), Some(Change::Route { route: "Speaker".into(), output: None }));
        let devices = parsed(r#"{"devices":["speaker","microphone"]}"#);
        assert_eq!(
            decode_event("devices_changed", &devices),
            Some(Change::Devices(vec!["speaker".into(), "microphone".into()]))
        );
        assert_eq!(decode_event("something_else", &devices), None);
    }

    #[test]
    fn a_volume_event_decodes_to_the_new_level_and_mute() {
        let body = parsed(r#"{"level":0.4,"muted":false,"gain":0.4}"#);
        assert_eq!(decode_event("volume_changed", &body), Some(Change::Volume(Volume { level: 0.4, muted: false, gain: 0.4 })));
        assert_eq!(decode_event("volume_changed", &parsed(r#"{"muted":true}"#)), None);
    }

    #[test]
    fn role_names_match_the_shell_service() {
        assert_eq!(role_name(Role::Input), "input");
        assert_eq!(role_name(Role::Output), "output");
        assert_eq!(role_name(Role::SoundEffects), "sound_effects");
    }
}
