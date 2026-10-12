//! The `power` service: the power state apps read through `ProcessInfo` (`thermalState`, `isLowPowerModeEnabled`) and the
//! battery (`IOPowerSources`), plus change events. Methods, each answering one JSON object:
//!
//! - `thermalState` -> `{"thermalState": "nominal"|"fair"|"serious"|"critical"}`
//! - `lowPowerMode` -> `{"lowPowerMode": bool}`
//! - `battery` -> `{"battery": {"present", "percent", "state", "pluggedIn", "minutesToEmpty", "minutesToFull"}}`
//! - `snapshot` -> all three keys at once.
//!
//! Events go to every running app and carry the same keys as the method that answers them: `thermalStateDidChange`,
//! `powerStateDidChange` and `batteryDidChange` (the battery event fires when the percent, the charging state or the
//! plug state changes, not on every energy reading). The sensors are read at most every [`POLL_SECS`].
//!
//! The models are `bifrost_core::thermal` and `bifrost_core::power_supply`. The shell's Low Power Mode setting is
//! also written to [`POWER_STATE_PATH`], which the SDK's `ProcessInfo::is_low_power_mode_enabled` reads. Per-process
//! energy impact has no model yet and is not answered here.
use super::{Event, Shell, SystemService};
use bifrost_core::json::Json;
use bifrost_core::power_supply::{self, PowerSource, PowerState, SupplyKind};
use bifrost_core::thermal::{self, ThermalState};
use std::path::Path;

/// The kernel's thermal zones.
pub const THERMAL_CLASS: &str = "/sys/class/thermal";
/// The kernel's power supplies.
pub const SUPPLY_CLASS: &str = "/sys/class/power_supply";
/// The file the shell writes its Low Power Mode setting to (`low_power=0|1`). Must match the SDK's
/// `bifrost_app::process_info::POWER_STATE_PATH`.
pub const POWER_STATE_PATH: &str = "/run/bifrost/power-state";
/// Seconds between sensor reads (the events are checked on each read).
pub const POLL_SECS: f64 = 10.0;

pub const EVENT_THERMAL: &str = "thermalStateDidChange";
pub const EVENT_LOW_POWER: &str = "powerStateDidChange";
pub const EVENT_BATTERY: &str = "batteryDidChange";

/// One reading of everything the service reports.
#[derive(Clone, Debug, PartialEq)]
pub struct Reading {
    pub thermal: ThermalState,
    pub low_power: bool,
    pub battery: Option<PowerSource>,
}

impl Reading {
    /// Reads the sensors. `low_power` is the shell's own setting, passed in because the shell owns it.
    pub fn now(low_power: bool) -> Reading {
        let supplies = power_supply::sources_in(Path::new(SUPPLY_CLASS));
        Reading {
            thermal: thermal::worst_in(Path::new(THERMAL_CLASS)),
            low_power,
            battery: supplies.into_iter().find(|s| s.kind == SupplyKind::Battery),
        }
    }
}

fn obj(pairs: Vec<(&str, Json)>) -> Json {
    Json::Obj(pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
}

fn state_name(s: PowerState) -> &'static str {
    match s {
        PowerState::Charging => "charging",
        PowerState::Discharging => "discharging",
        PowerState::Full => "full",
        PowerState::NotCharging => "notCharging",
        PowerState::Unknown => "unknown",
    }
}

fn number(v: Option<u32>) -> Json {
    v.map_or(Json::Null, |m| Json::Num(m as f64))
}

/// The battery as the service reports it. `present` is false when the machine has no battery.
pub fn battery_json(b: Option<&PowerSource>) -> Json {
    match b {
        None => obj(vec![("present", Json::Bool(false))]),
        Some(b) => obj(vec![
            ("present", Json::Bool(true)),
            ("percent", b.percent.map_or(Json::Null, |p| Json::Num(p as f64))),
            ("state", Json::Str(state_name(b.state).into())),
            ("pluggedIn", b.online.map_or(Json::Null, Json::Bool)),
            ("minutesToEmpty", number(b.minutes_to_empty)),
            ("minutesToFull", number(b.minutes_to_full)),
        ]),
    }
}

/// The three keys of the `snapshot` method, which are also the keys of the three events.
pub fn snapshot_json(r: &Reading) -> Json {
    obj(vec![
        ("thermalState", Json::Str(r.thermal.name().into())),
        ("lowPowerMode", Json::Bool(r.low_power)),
        ("battery", battery_json(r.battery.as_ref())),
    ])
}

/// The events for going from `prev` to `now`. The first reading (no `prev`) reports nothing.
pub fn changes(prev: Option<&Reading>, now: &Reading) -> Vec<(&'static str, Json)> {
    let Some(prev) = prev else { return Vec::new() };
    let mut out = Vec::new();
    if prev.thermal != now.thermal {
        out.push((EVENT_THERMAL, obj(vec![("thermalState", Json::Str(now.thermal.name().into()))])));
    }
    if prev.low_power != now.low_power {
        out.push((EVENT_LOW_POWER, obj(vec![("lowPowerMode", Json::Bool(now.low_power))])));
    }
    if battery_key(prev.battery.as_ref()) != battery_key(now.battery.as_ref()) {
        out.push((EVENT_BATTERY, obj(vec![("battery", battery_json(now.battery.as_ref()))])));
    }
    out
}

/// What a battery event compares: the level, the charging state and the plug state. Time estimates are left out,
/// because they move with every energy reading.
fn battery_key(b: Option<&PowerSource>) -> Option<(Option<u8>, PowerState, Option<bool>)> {
    b.map(|b| (b.percent, b.state, b.online))
}

/// Writes the Low Power Mode setting for the SDK's `ProcessInfo`. Best effort: a read-only `/run` only means apps
/// keep seeing "off".
fn write_low_power(on: bool) {
    let path = Path::new(POWER_STATE_PATH);
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(path, format!("low_power={}\n", on as u8));
}

#[derive(Default)]
pub struct PowerService {
    next_poll: f64,
    last: Option<Reading>,
}

impl SystemService for PowerService {
    fn name(&self) -> &'static str {
        "power"
    }

    fn call(&mut self, shell: &mut Shell, _app_id: &str, method: &str, _params: &Json, _t: f64) -> Result<Json, String> {
        let r = Reading::now(shell.low_power());
        match method {
            "thermalState" => Ok(obj(vec![("thermalState", Json::Str(r.thermal.name().into()))])),
            "lowPowerMode" => Ok(obj(vec![("lowPowerMode", Json::Bool(r.low_power))])),
            "battery" => Ok(obj(vec![("battery", battery_json(r.battery.as_ref()))])),
            "snapshot" => Ok(snapshot_json(&r)),
            _ => Err(format!("power has no method {method}")),
        }
    }

    fn tick(&mut self, shell: &mut Shell, t: f64) -> Vec<Event> {
        if t < self.next_poll {
            return Vec::new();
        }
        self.next_poll = t + POLL_SECS;
        let now = Reading::now(shell.low_power());
        let events = changes(self.last.as_ref(), &now)
            .into_iter()
            .map(|(event, body)| Event { app: None, event: event.to_string(), body })
            .collect();
        if self.last.as_ref().map_or(true, |p| p.low_power != now.low_power) {
            write_low_power(now.low_power);
        }
        self.last = Some(now);
        events
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn battery(percent: u8, state: PowerState, minutes: u32) -> PowerSource {
        PowerSource {
            name: "BAT0".into(),
            kind: SupplyKind::Battery,
            percent: Some(percent),
            state,
            online: Some(state == PowerState::Charging),
            minutes_to_empty: (state == PowerState::Discharging).then_some(minutes),
            minutes_to_full: (state == PowerState::Charging).then_some(minutes),
        }
    }

    fn reading(thermal: ThermalState, low_power: bool, battery: Option<PowerSource>) -> Reading {
        Reading { thermal, low_power, battery }
    }

    fn field<'a>(j: &'a Json, key: &str) -> &'a Json {
        j.get(key).unwrap_or(&Json::Null)
    }

    #[test]
    fn the_first_reading_reports_nothing() {
        let now = reading(ThermalState::Fair, true, Some(battery(50, PowerState::Discharging, 90)));
        assert!(changes(None, &now).is_empty());
    }

    #[test]
    fn thermal_and_low_power_changes_emit_their_own_events() {
        let before = reading(ThermalState::Nominal, false, None);
        let after = reading(ThermalState::Serious, true, None);
        let ev = changes(Some(&before), &after);
        assert_eq!(ev.len(), 2);
        assert_eq!(ev[0].0, EVENT_THERMAL);
        assert_eq!(field(&ev[0].1, "thermalState"), &Json::Str("serious".into()));
        assert_eq!(ev[1].0, EVENT_LOW_POWER);
        assert_eq!(field(&ev[1].1, "lowPowerMode"), &Json::Bool(true));
        assert!(changes(Some(&after), &after).is_empty(), "no change, no event");
    }

    #[test]
    fn energy_only_battery_readings_do_not_emit() {
        let before = reading(ThermalState::Nominal, false, Some(battery(50, PowerState::Discharging, 90)));
        let mut after = before.clone();
        after.battery.as_mut().unwrap().minutes_to_empty = Some(80);
        assert!(changes(Some(&before), &after).is_empty());
        let level = reading(ThermalState::Nominal, false, Some(battery(49, PowerState::Discharging, 80)));
        let ev = changes(Some(&before), &level);
        assert_eq!(ev.len(), 1);
        assert_eq!(ev[0].0, EVENT_BATTERY);
    }

    #[test]
    fn a_battery_appearing_or_going_is_an_event() {
        let none = reading(ThermalState::Nominal, false, None);
        let some = reading(ThermalState::Nominal, false, Some(battery(80, PowerState::Charging, 30)));
        assert_eq!(changes(Some(&none), &some).len(), 1);
        assert_eq!(changes(Some(&some), &none).len(), 1);
    }

    #[test]
    fn battery_json_reports_presence_and_estimates() {
        let j = battery_json(Some(&battery(42, PowerState::Discharging, 75)));
        assert_eq!(field(&j, "present"), &Json::Bool(true));
        assert_eq!(field(&j, "percent"), &Json::Num(42.0));
        assert_eq!(field(&j, "state"), &Json::Str("discharging".into()));
        assert_eq!(field(&j, "pluggedIn"), &Json::Bool(false));
        assert_eq!(field(&j, "minutesToEmpty"), &Json::Num(75.0));
        assert_eq!(field(&j, "minutesToFull"), &Json::Null);
        let none = battery_json(None);
        assert_eq!(field(&none, "present"), &Json::Bool(false));
        assert_eq!(none.encode(), r#"{"present":false}"#);
    }

    #[test]
    fn snapshot_has_the_three_keys_the_events_use() {
        let j = snapshot_json(&reading(ThermalState::Critical, true, None));
        assert_eq!(field(&j, "thermalState"), &Json::Str("critical".into()));
        assert_eq!(field(&j, "lowPowerMode"), &Json::Bool(true));
        assert_eq!(field(&j, "battery").get("present"), Some(&Json::Bool(false)));
    }

    #[test]
    fn the_power_state_path_is_the_sdk_path() {
        assert_eq!(POWER_STATE_PATH, "/run/bifrost/power-state");
    }

    #[test]
    fn the_sensor_read_never_fails() {
        // On a host with no battery and no thermal zones this is Nominal and no battery; on a device it is real.
        let r = Reading::now(false);
        assert!(r.thermal >= ThermalState::Nominal);
        if let Some(b) = &r.battery {
            assert_eq!(b.kind, SupplyKind::Battery);
        }
    }
}
