//! Typed client for the shell's `power` service (`springboard/src/shell/services/power_service.rs`). It gives apps the
//! `ProcessInfo` power state (thermal state, Low Power Mode) and the battery, and it pushes change events.
//!
//! ```ignore
//! power_service::snapshot(ctx);               // later, in App::service_reply(ctx, request, &result):
//! if let Some(s) = power_service::decode_snapshot(result) { /* s.thermal_state, s.low_power_mode, s.battery */ }
//! // App::service_event(ctx, service, event, body):
//! if let Some(change) = power_service::decode_change(service, event, body) { /* react */ }
//! ```
//! The calls answer through `App::service_reply`; the events arrive through `App::service_event` for every running
//! app. Decoding is pure, so the app can keep the replies in its own state. The models are `bifrost_core::thermal` (the
//! same type as `ProcessInfo::thermal_state`) and `bifrost_core::power_supply` (through `power_sources::PowerState`).
use crate::power_sources::PowerState;
use crate::process_info::ThermalState;
use crate::runtime::Ctx;
use crate::service;
use bifrost_core::json::Json;

/// The service name the shell registers.
pub const SERVICE: &str = "power";
/// `ProcessInfo.thermalStateDidChangeNotification`: body `{"thermalState": ...}`.
pub const THERMAL_STATE_DID_CHANGE: &str = "thermalStateDidChange";
/// `ProcessInfo.powerStateDidChangeNotification`: body `{"lowPowerMode": bool}`.
pub const POWER_STATE_DID_CHANGE: &str = "powerStateDidChange";
/// The battery changed level, charging state or plug state: body `{"battery": {...}}`.
pub const BATTERY_DID_CHANGE: &str = "batteryDidChange";

/// The battery as the shell reports it. `percent` is None when the battery does not say.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Battery {
    pub percent: Option<u8>,
    pub state: PowerState,
    pub plugged_in: Option<bool>,
    pub minutes_to_empty: Option<u32>,
    pub minutes_to_full: Option<u32>,
}

impl Battery {
    /// The time left in the direction the battery is moving: minutes to empty while discharging, minutes to full
    /// while charging, else None.
    pub fn time_remaining(&self) -> Option<u32> {
        match self.state {
            PowerState::Discharging => self.minutes_to_empty,
            PowerState::Charging => self.minutes_to_full,
            _ => None,
        }
    }
}

/// Everything the service reports, read at one time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    pub thermal_state: ThermalState,
    pub low_power_mode: bool,
    /// None when the machine has no battery.
    pub battery: Option<Battery>,
}

/// One pushed change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    ThermalState(ThermalState),
    LowPowerMode(bool),
    /// None when the battery went away.
    Battery(Option<Battery>),
}

/// Asks for the thermal state. The reply is `{"thermalState": ...}`; see [`decode_thermal_state`].
pub fn thermal_state(ctx: &Ctx) -> u32 {
    service::call(ctx, SERVICE, "thermalState", &Json::Null)
}

/// Asks for Low Power Mode. The reply is `{"lowPowerMode": bool}`; see [`decode_low_power_mode`].
pub fn low_power_mode(ctx: &Ctx) -> u32 {
    service::call(ctx, SERVICE, "lowPowerMode", &Json::Null)
}

/// Asks for the battery. The reply is `{"battery": {...}}`; see [`decode_battery`].
pub fn battery(ctx: &Ctx) -> u32 {
    service::call(ctx, SERVICE, "battery", &Json::Null)
}

/// Asks for all three at once; see [`decode_snapshot`].
pub fn snapshot(ctx: &Ctx) -> u32 {
    service::call(ctx, SERVICE, "snapshot", &Json::Null)
}

fn boolean(j: &Json) -> Option<bool> {
    match j {
        Json::Bool(b) => Some(*b),
        _ => None,
    }
}

fn minutes(j: Option<&Json>) -> Option<u32> {
    j.and_then(|j| j.number()).filter(|m| *m >= 0.0).map(|m| m as u32)
}

/// The battery object. Outer None: malformed. Inner None: no battery on this machine.
fn battery_value(b: &Json) -> Option<Option<Battery>> {
    if !boolean(b.get("present")?)? {
        return Some(None);
    }
    let percent = b.get("percent").and_then(|j| j.number()).map(|p| p.clamp(0.0, 100.0) as u8);
    let state = match b.get("state")?.as_str()? {
        "charging" => PowerState::Charging,
        "discharging" => PowerState::Discharging,
        "full" => PowerState::Full,
        "notCharging" => PowerState::NotCharging,
        _ => PowerState::Unknown,
    };
    Some(Some(Battery {
        percent,
        state,
        plugged_in: b.get("pluggedIn").and_then(boolean),
        minutes_to_empty: minutes(b.get("minutesToEmpty")),
        minutes_to_full: minutes(b.get("minutesToFull")),
    }))
}

/// The thermal state from a reply to [`thermal_state`]. None on an error reply or a malformed one.
pub fn decode_thermal_state(reply: &Result<Json, String>) -> Option<ThermalState> {
    ThermalState::parse(reply.as_ref().ok()?.get("thermalState")?.as_str()?)
}

/// Low Power Mode from a reply to [`low_power_mode`].
pub fn decode_low_power_mode(reply: &Result<Json, String>) -> Option<bool> {
    boolean(reply.as_ref().ok()?.get("lowPowerMode")?)
}

/// The battery from a reply to [`battery`]. Outer None: the reply is an error or malformed. Inner None: no battery.
pub fn decode_battery(reply: &Result<Json, String>) -> Option<Option<Battery>> {
    battery_value(reply.as_ref().ok()?.get("battery")?)
}

/// The whole snapshot from a reply to [`snapshot`].
pub fn decode_snapshot(reply: &Result<Json, String>) -> Option<Snapshot> {
    let j = reply.as_ref().ok()?;
    Some(Snapshot {
        thermal_state: ThermalState::parse(j.get("thermalState")?.as_str()?)?,
        low_power_mode: boolean(j.get("lowPowerMode")?)?,
        battery: battery_value(j.get("battery")?)?,
    })
}

/// The change a pushed `ServiceEvent` carries, or None when it is not one of this service's events.
pub fn decode_change(service: &str, event: &str, body: &Json) -> Option<Change> {
    if service != SERVICE {
        return None;
    }
    match event {
        THERMAL_STATE_DID_CHANGE => ThermalState::parse(body.get("thermalState")?.as_str()?).map(Change::ThermalState),
        POWER_STATE_DID_CHANGE => boolean(body.get("lowPowerMode")?).map(Change::LowPowerMode),
        BATTERY_DID_CHANGE => battery_value(body.get("battery")?).map(Change::Battery),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reply(text: &str) -> Result<Json, String> {
        Ok(bifrost_core::json::parse(text).unwrap())
    }

    #[test]
    fn thermal_and_low_power_replies_decode() {
        assert_eq!(decode_thermal_state(&reply(r#"{"thermalState":"serious"}"#)), Some(ThermalState::Serious));
        assert_eq!(decode_thermal_state(&reply(r#"{"thermalState":"hot"}"#)), None);
        assert_eq!(decode_thermal_state(&Err("no service power".into())), None);
        assert_eq!(decode_low_power_mode(&reply(r#"{"lowPowerMode":true}"#)), Some(true));
        assert_eq!(decode_low_power_mode(&reply(r#"{"lowPowerMode":1}"#)), None);
    }

    #[test]
    fn a_battery_decodes_with_its_estimates() {
        let b = decode_battery(&reply(
            r#"{"battery":{"present":true,"percent":42,"state":"discharging","pluggedIn":false,"minutesToEmpty":75,"minutesToFull":null}}"#,
        ));
        let b = b.unwrap().unwrap();
        assert_eq!(b.percent, Some(42));
        assert_eq!(b.state, PowerState::Discharging);
        assert_eq!(b.plugged_in, Some(false));
        assert_eq!(b.time_remaining(), Some(75));
        assert_eq!(decode_battery(&reply(r#"{"battery":{"present":false}}"#)), Some(None));
        assert_eq!(decode_battery(&reply(r#"{"battery":{"present":true}}"#)), None, "state is required");
    }

    #[test]
    fn time_remaining_follows_the_state() {
        let mut b = Battery { percent: Some(50), state: PowerState::Charging, plugged_in: Some(true), minutes_to_empty: None, minutes_to_full: Some(30) };
        assert_eq!(b.time_remaining(), Some(30));
        b.state = PowerState::Full;
        assert_eq!(b.time_remaining(), None);
    }

    #[test]
    fn the_snapshot_needs_all_three_keys() {
        let s = decode_snapshot(&reply(
            r#"{"thermalState":"nominal","lowPowerMode":false,"battery":{"present":false}}"#,
        ))
        .unwrap();
        assert_eq!(s, Snapshot { thermal_state: ThermalState::Nominal, low_power_mode: false, battery: None });
        assert!(decode_snapshot(&reply(r#"{"thermalState":"nominal","lowPowerMode":false}"#)).is_none());
    }

    #[test]
    fn pushed_events_decode_by_name() {
        let body = bifrost_core::json::parse(r#"{"thermalState":"critical"}"#).unwrap();
        assert_eq!(decode_change(SERVICE, THERMAL_STATE_DID_CHANGE, &body), Some(Change::ThermalState(ThermalState::Critical)));
        let body = bifrost_core::json::parse(r#"{"lowPowerMode":true}"#).unwrap();
        assert_eq!(decode_change(SERVICE, POWER_STATE_DID_CHANGE, &body), Some(Change::LowPowerMode(true)));
        let body = bifrost_core::json::parse(r#"{"battery":{"present":false}}"#).unwrap();
        assert_eq!(decode_change(SERVICE, BATTERY_DID_CHANGE, &body), Some(Change::Battery(None)));
    }

    #[test]
    fn other_services_and_events_are_ignored() {
        let body = Json::Null;
        assert_eq!(decode_change("system", THERMAL_STATE_DID_CHANGE, &body), None);
        assert_eq!(decode_change(SERVICE, "somethingElse", &body), None);
        assert_eq!(decode_change(SERVICE, POWER_STATE_DID_CHANGE, &body), None, "a body without the key");
    }
}
