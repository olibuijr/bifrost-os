//! The shell's `bluetooth` service as an app uses it: the radio state and the nearby and paired devices, the requests
//! Settings makes (power, scan, pair, connect, disconnect, forget), and the connect and disconnect events.
//!
//! The calls queue a request with the shell and return at once; the reply is `App::service_reply` (or
//! `service::take_reply`). Feed each `status` reply to [`Bluetooth::apply_status`] and each `App::service_event` for
//! [`SERVICE`] to [`Bluetooth::apply_event`]. The device list is `bluetooth_devices::DeviceList` (paired first) and the
//! events are queued per registration in `bluetooth_events::DeviceEvents`. The shell keeps the radio; this module only
//! keeps the app's copy and never changes the radio itself.
//!
//! ```ignore
//! let mut bt = bluetooth_service::Bluetooth::new();
//! let id = bluetooth_service::status(ctx);      // later: bt.apply_status(&body) from the reply
//! // App::service_event: if bt.apply_event(event, body) { for e in bt.take_events() { ... } }
//! ```

use std::str::FromStr;

use bifrost_core::bt_address::BdAddr;
use bifrost_core::json::Json;

use crate::bluetooth_devices::DeviceList;
use crate::bluetooth_events::{DeviceEvent, DeviceEventKind, DeviceEvents, Filter, Registration};
use crate::runtime::Ctx;
use crate::service;

/// The service name the shell registers.
pub const SERVICE: &str = "bluetooth";

/// Asks for the radio state and the device table. Reply: `Bluetooth::apply_status`.
pub fn status(ctx: &Ctx) -> u32 {
    service::call(ctx, SERVICE, "status", &Json::Null)
}

/// Turns the radio on or off.
pub fn enable(ctx: &Ctx, on: bool) -> u32 {
    service::call(ctx, SERVICE, "enable", &Json::Bool(on))
}

/// Starts a scan for nearby devices.
pub fn scan(ctx: &Ctx) -> u32 {
    service::call(ctx, SERVICE, "scan", &Json::Null)
}

/// Pairs with a device the shell has found.
pub fn pair(ctx: &Ctx, address: BdAddr) -> u32 {
    service::call(ctx, SERVICE, "pair", &Json::Str(address.to_string()))
}

/// Connects to a paired device.
pub fn connect(ctx: &Ctx, address: BdAddr) -> u32 {
    service::call(ctx, SERVICE, "connect", &Json::Str(address.to_string()))
}

/// Disconnects a device, keeping its pairing.
pub fn disconnect(ctx: &Ctx, address: BdAddr) -> u32 {
    service::call(ctx, SERVICE, "disconnect", &Json::Str(address.to_string()))
}

/// Unpairs a device and drops its stored key.
pub fn forget(ctx: &Ctx, address: BdAddr) -> u32 {
    service::call(ctx, SERVICE, "forget", &Json::Str(address.to_string()))
}

/// The state of the shell's radio, as the last `status` reply said.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RadioState {
    /// The computer has no Bluetooth controller.
    Unavailable,
    Off,
    On,
}

/// An app's copy of the Bluetooth radio: its state, its devices and the connect and disconnect events since it last
/// asked. Created with `new`, which registers for every connection event.
pub struct Bluetooth {
    pub state: RadioState,
    pub scanning: bool,
    /// The device the shell is pairing or connecting right now.
    pub busy: Option<BdAddr>,
    /// The last error the radio reported.
    pub error: Option<String>,
    pub devices: DeviceList,
    events: DeviceEvents,
    reg: Registration,
}

impl Bluetooth {
    pub fn new() -> Self {
        let mut events = DeviceEvents::new();
        let reg = events.register(Filter::all());
        Bluetooth { state: RadioState::Unavailable, scanning: false, busy: None, error: None, devices: DeviceList::new(), events, reg }
    }

    /// Replaces the state and the device list with a `status` reply. A device whose address does not read is skipped.
    pub fn apply_status(&mut self, body: &Json) -> Result<(), String> {
        let state = match body.get("state").and_then(Json::as_str) {
            Some("on") => RadioState::On,
            Some("off") => RadioState::Off,
            Some("unavailable") => RadioState::Unavailable,
            _ => return Err("status has no radio state".to_string()),
        };
        let mut devices = DeviceList::new();
        for d in body.get("devices").and_then(Json::as_array).unwrap_or(&[]) {
            let Some(address) = d.get("addr").and_then(Json::as_str).and_then(|a| BdAddr::from_str(a).ok()) else {
                continue;
            };
            let name = d.get("name").and_then(Json::as_str).filter(|n| !n.is_empty());
            let rssi = match d.get("rssi") {
                Some(Json::Num(n)) => Some(*n as i8),
                _ => None,
            };
            devices.report(address, None, name, rssi);
            devices.set_paired(address, flag(d, "paired"));
            devices.set_connected(address, flag(d, "connected"));
        }
        self.state = state;
        self.scanning = flag(body, "scanning");
        self.busy = body.get("busy").and_then(Json::as_str).and_then(|a| BdAddr::from_str(a).ok());
        self.error = body.get("error").and_then(Json::as_str).map(str::to_string);
        self.devices = devices;
        Ok(())
    }

    /// Applies a service event. Returns true when it was a Bluetooth connection event (the app may then call
    /// [`Bluetooth::take_events`]); other events are not this module's and return false.
    pub fn apply_event(&mut self, event: &str, body: &Json) -> bool {
        let kind = match event {
            "connected" => DeviceEventKind::Connected,
            "disconnected" => DeviceEventKind::Disconnected,
            _ => return false,
        };
        let Some(address) = body.get("addr").and_then(Json::as_str).and_then(|a| BdAddr::from_str(a).ok()) else {
            return false;
        };
        let name = body.get("name").and_then(Json::as_str).filter(|n| !n.is_empty());
        if name.is_some() {
            self.devices.report(address, None, name, None);
        }
        self.devices.set_connected(address, kind == DeviceEventKind::Connected);
        self.events.post(DeviceEvent { kind, address });
        true
    }

    /// The connect and disconnect events queued since the last call, oldest first.
    pub fn take_events(&mut self) -> Vec<DeviceEvent> {
        self.events.take(self.reg)
    }
}

impl Default for Bluetooth {
    fn default() -> Self {
        Self::new()
    }
}

fn flag(v: &Json, key: &str) -> bool {
    matches!(v.get(key), Some(Json::Bool(true)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn obj(pairs: Vec<(&str, Json)>) -> Json {
        Json::Obj(pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
    }

    fn device(addr: &str, name: &str, paired: bool, connected: bool) -> Json {
        obj(vec![
            ("addr", Json::Str(addr.into())),
            ("name", Json::Str(name.into())),
            ("paired", Json::Bool(paired)),
            ("connected", Json::Bool(connected)),
            ("rssi", Json::Num(-61.0)),
        ])
    }

    fn status_body(state: &str, devices: Vec<Json>) -> Json {
        obj(vec![
            ("state", Json::Str(state.into())),
            ("scanning", Json::Bool(true)),
            ("busy", Json::Null),
            ("error", Json::Null),
            ("devices", Json::Arr(devices)),
        ])
    }

    #[test]
    fn a_status_reply_sets_the_radio_and_the_device_list_paired_first() {
        let mut bt = Bluetooth::new();
        let body = status_body(
            "on",
            vec![device("AA:BB:CC:DD:EE:02", "Speaker", false, false), device("AA:BB:CC:DD:EE:01", "Buds", true, true)],
        );
        bt.apply_status(&body).unwrap();
        assert_eq!(bt.state, RadioState::On);
        assert!(bt.scanning);
        let labels: Vec<String> = bt.devices.all().iter().map(|d| d.label()).collect();
        assert_eq!(labels, vec!["Buds", "Speaker"]);
        let buds = bt.devices.get(BdAddr::from_str("AA:BB:CC:DD:EE:01").unwrap()).unwrap();
        assert!(buds.paired && buds.connected);
        assert_eq!(buds.rssi, Some(-61));
    }

    #[test]
    fn a_status_without_a_radio_state_is_refused_and_a_bad_address_is_skipped() {
        let mut bt = Bluetooth::new();
        assert!(bt.apply_status(&obj(vec![("devices", Json::Arr(vec![]))])).is_err());
        let body = status_body("off", vec![device("not-an-address", "X", true, false), device("AA:BB:CC:DD:EE:03", "", false, false)]);
        bt.apply_status(&body).unwrap();
        assert_eq!(bt.state, RadioState::Off);
        assert_eq!(bt.devices.len(), 1);
    }

    #[test]
    fn a_connection_event_updates_the_list_and_queues_for_the_app() {
        let mut bt = Bluetooth::new();
        bt.apply_status(&status_body("on", vec![device("AA:BB:CC:DD:EE:01", "Buds", true, false)])).unwrap();
        let body = obj(vec![("addr", Json::Str("AA:BB:CC:DD:EE:01".into())), ("name", Json::Str("Buds".into()))]);
        assert!(bt.apply_event("connected", &body));
        assert!(bt.devices.connected().len() == 1);
        assert!(bt.apply_event("disconnected", &body));
        assert!(bt.devices.connected().is_empty());
        let events = bt.take_events();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].kind, DeviceEventKind::Connected);
        assert_eq!(events[1].kind, DeviceEventKind::Disconnected);
        assert!(bt.take_events().is_empty());
    }

    #[test]
    fn other_services_events_and_bad_bodies_are_not_taken() {
        let mut bt = Bluetooth::new();
        assert!(!bt.apply_event("connected", &obj(vec![("addr", Json::Str("nope".into()))])));
        assert!(!bt.apply_event("dock_changed", &Json::Null));
        assert!(bt.take_events().is_empty());
    }
}
