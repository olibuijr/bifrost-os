//! The `bluetooth` service: the shell's Bluetooth radio and device table for apps, the same data Settings › Bluetooth
//! shows.
//!
//! - `status` returns `{state, scanning, summary, busy, error, devices}`. `devices` lists paired devices first, then the
//!   ones found, as the shell keeps them. Each device is `{addr, name, label, kind, paired, connected, audio, rssi}`.
//! - `enable(bool)`, `scan`, `pair(addr)`, `connect(addr)`, `disconnect(addr)` and `forget(addr)` make the same requests
//!   Settings makes. A command is queued here and the shell's tick hands it to the radio thread, so the call never
//!   blocks. An address must name a device the shell already knows.
//! - Events: `connected` and `disconnected`, body `{addr, name}`, go to every running app when a device's connection
//!   changes. The first tick only records the connected devices, so no event is sent for the state at start-up.
//!
//! Commands are queued in a process-wide list because a service cannot return effects. Only the shell's tick drains it.
use std::str::FromStr;
use std::sync::Mutex;

use super::{Event, Shell, SystemService};
use crate::bare::bluetooth::{Cmd, Device, State, Status};
use bifrost_core::bt_address::BdAddr;
use bifrost_core::json::Json;

/// Most queued commands kept. The radio thread drains them on every shell tick, so this only bounds a flood.
const QUEUE_CAP: usize = 256;

/// Commands apps asked for, waiting for the shell's tick.
static QUEUE: Mutex<Vec<Cmd>> = Mutex::new(Vec::new());

/// The commands queued since the last call. The shell's tick turns each into `Effect::Bluetooth`.
pub fn take_commands() -> Vec<Cmd> {
    match QUEUE.lock() {
        Ok(mut q) => std::mem::take(&mut *q),
        Err(_) => Vec::new(),
    }
}

fn queue(cmd: Cmd) {
    if let Ok(mut q) = QUEUE.lock() {
        if q.len() < QUEUE_CAP {
            q.push(cmd);
        }
    }
}

#[derive(Default)]
pub struct BluetoothService {
    /// The addresses connected at the last tick. `None` before the first tick, which only records them.
    connected: Option<Vec<String>>,
}

impl SystemService for BluetoothService {
    fn name(&self) -> &'static str {
        "bluetooth"
    }

    fn call(&mut self, shell: &mut Shell, _app_id: &str, method: &str, params: &Json, _t: f64) -> Result<Json, String> {
        if method == "status" {
            return Ok(status_json(&shell.info.bt));
        }
        if let Some(cmd) = command(&shell.info.bt, method, params)? {
            queue(cmd);
        }
        Ok(Json::Null)
    }

    fn tick(&mut self, shell: &mut Shell, _t: f64) -> Vec<Event> {
        connection_events(&mut self.connected, &shell.info.bt)
    }
}

/// The radio state and device table as an app reads them.
pub(crate) fn status_json(bt: &Status) -> Json {
    let state = match bt.state {
        State::Unavailable => "unavailable",
        State::Off => "off",
        State::On => "on",
    };
    Json::Obj(vec![
        ("state".to_string(), Json::Str(state.to_string())),
        ("scanning".to_string(), Json::Bool(bt.scanning)),
        ("summary".to_string(), Json::Str(bt.summary().to_string())),
        ("busy".to_string(), opt_str(bt.busy.as_deref())),
        ("error".to_string(), opt_str(bt.error.as_deref())),
        ("devices".to_string(), Json::Arr(bt.devices.iter().map(device_json).collect())),
    ])
}

fn device_json(d: &Device) -> Json {
    Json::Obj(vec![
        ("addr".to_string(), Json::Str(d.addr.clone())),
        ("name".to_string(), Json::Str(d.name.clone())),
        ("label".to_string(), Json::Str(d.label().to_string())),
        ("kind".to_string(), Json::Num(f64::from(d.kind))),
        ("paired".to_string(), Json::Bool(d.paired)),
        ("connected".to_string(), Json::Bool(d.connected)),
        ("audio".to_string(), Json::Bool(d.audio)),
        ("rssi".to_string(), d.rssi.map_or(Json::Null, |r| Json::Num(f64::from(r)))),
    ])
}

fn opt_str(s: Option<&str>) -> Json {
    s.map_or(Json::Null, |s| Json::Str(s.to_string()))
}

/// The radio command a method asks for, checked against the current state. `Ok(None)` is not a command. Pure: it
/// queues nothing, so tests can call it.
pub(crate) fn command(bt: &Status, method: &str, params: &Json) -> Result<Option<Cmd>, String> {
    match method {
        "enable" => {
            let on = match params {
                Json::Bool(b) => *b,
                _ => return Err("enable takes true or false".to_string()),
            };
            if bt.state == State::Unavailable {
                return Err(UNAVAILABLE.to_string());
            }
            Ok(Some(Cmd::Enable(on)))
        }
        "scan" => {
            radio_on(bt)?;
            Ok(Some(Cmd::Scan))
        }
        "pair" | "connect" | "disconnect" | "forget" => {
            radio_on(bt)?;
            let addr = address(params)?;
            let device = bt
                .devices
                .iter()
                .find(|d| d.addr.parse::<BdAddr>().ok() == Some(addr))
                .ok_or_else(|| format!("no device {addr}"))?;
            let a = device.addr.clone();
            Ok(Some(match method {
                "pair" => Cmd::Pair(a),
                "connect" => Cmd::Connect(a),
                "disconnect" => Cmd::Disconnect(a),
                _ => Cmd::Forget(a),
            }))
        }
        _ => Err(format!("bluetooth has no method {method}")),
    }
}

const UNAVAILABLE: &str = "bluetooth is not available on this computer";

fn radio_on(bt: &Status) -> Result<(), String> {
    match bt.state {
        State::Unavailable => Err(UNAVAILABLE.to_string()),
        State::Off => Err("bluetooth is off".to_string()),
        State::On => Ok(()),
    }
}

fn address(params: &Json) -> Result<BdAddr, String> {
    let text = params.as_str().ok_or_else(|| "expected an address such as AA:BB:CC:DD:EE:FF".to_string())?;
    BdAddr::from_str(text).map_err(|_| format!("bad address {text}"))
}

/// The `connected` and `disconnected` events for the change since the last tick. Records the new set in `prev`.
pub(crate) fn connection_events(prev: &mut Option<Vec<String>>, bt: &Status) -> Vec<Event> {
    let now: Vec<String> = bt.devices.iter().filter(|d| d.connected).map(|d| d.addr.clone()).collect();
    let before = prev.replace(now.clone());
    let Some(before) = before else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for d in bt.devices.iter().filter(|d| d.connected && !before.contains(&d.addr)) {
        out.push(event("connected", &d.addr, &d.name));
    }
    for addr in before.iter().filter(|a| !now.contains(a)) {
        let name = bt.device(addr).map_or("", |d| d.name.as_str());
        out.push(event("disconnected", addr, name));
    }
    out
}

fn event(kind: &str, addr: &str, name: &str) -> Event {
    Event {
        app: None,
        event: kind.to_string(),
        body: Json::Obj(vec![
            ("addr".to_string(), Json::Str(addr.to_string())),
            ("name".to_string(), Json::Str(name.to_string())),
        ]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dev(addr: &str, name: &str, paired: bool, connected: bool) -> Device {
        Device { addr: addr.into(), kind: 0, name: name.into(), paired, connected, audio: true, rssi: Some(-50) }
    }

    fn radio(state: State, devices: Vec<Device>) -> Status {
        Status { state, devices, ..Status::default() }
    }

    #[test]
    fn status_lists_state_and_devices_with_their_flags() {
        let mut bt = radio(State::On, vec![dev("AA:BB:CC:DD:EE:01", "Buds", true, true), dev("AA:BB:CC:DD:EE:02", "", false, false)]);
        bt.scanning = true;
        let j = status_json(&bt);
        assert_eq!(j.get("state").and_then(Json::as_str), Some("on"));
        assert_eq!(j.get("summary").and_then(Json::as_str), Some("Connected"));
        assert_eq!(j.get("scanning"), Some(&Json::Bool(true)));
        assert_eq!(j.get("busy"), Some(&Json::Null));
        let devices = j.get("devices").and_then(Json::as_array).unwrap();
        assert_eq!(devices.len(), 2);
        assert_eq!(devices[0].get("name").and_then(Json::as_str), Some("Buds"));
        assert_eq!(devices[0].get("connected"), Some(&Json::Bool(true)));
        assert_eq!(devices[1].get("label").and_then(Json::as_str), Some("AA:BB:CC:DD:EE:02"));
    }

    #[test]
    fn an_unavailable_radio_reports_that_and_no_devices() {
        let j = status_json(&Status::default());
        assert_eq!(j.get("state").and_then(Json::as_str), Some("unavailable"));
        assert_eq!(j.get("summary").and_then(Json::as_str), Some("Off"));
    }

    #[test]
    fn enable_takes_a_boolean_and_refuses_without_a_controller() {
        let on = radio(State::Off, vec![]);
        assert_eq!(command(&on, "enable", &Json::Bool(true)), Ok(Some(Cmd::Enable(true))));
        assert!(command(&on, "enable", &Json::Str("yes".into())).is_err());
        assert_eq!(command(&Status::default(), "enable", &Json::Bool(true)), Err(UNAVAILABLE.to_string()));
    }

    #[test]
    fn device_commands_need_a_radio_and_a_known_address() {
        let bt = radio(State::On, vec![dev("AA:BB:CC:DD:EE:01", "Buds", true, false)]);
        let addr = Json::Str("aa:bb:cc:dd:ee:01".into());
        assert_eq!(command(&bt, "connect", &addr), Ok(Some(Cmd::Connect("AA:BB:CC:DD:EE:01".into()))));
        assert_eq!(command(&bt, "forget", &addr), Ok(Some(Cmd::Forget("AA:BB:CC:DD:EE:01".into()))));
        assert_eq!(command(&bt, "disconnect", &addr), Ok(Some(Cmd::Disconnect("AA:BB:CC:DD:EE:01".into()))));
        assert!(command(&bt, "connect", &Json::Str("AA:BB:CC:DD:EE:99".into())).is_err());
        assert!(command(&bt, "connect", &Json::Str("nonsense".into())).is_err());
        assert!(command(&radio(State::Off, bt.devices.clone()), "connect", &addr).is_err());
        assert_eq!(command(&bt, "scan", &Json::Null), Ok(Some(Cmd::Scan)));
        assert!(command(&bt, "reboot", &Json::Null).is_err());
    }

    #[test]
    fn the_first_tick_only_records_and_later_ticks_send_connection_changes() {
        let mut prev = None;
        let a = radio(State::On, vec![dev("AA:BB:CC:DD:EE:01", "Buds", true, true)]);
        assert!(connection_events(&mut prev, &a).is_empty());
        let same = a.clone();
        assert!(connection_events(&mut prev, &same).is_empty());

        let b = radio(State::On, vec![dev("AA:BB:CC:DD:EE:01", "Buds", true, false), dev("AA:BB:CC:DD:EE:02", "Mouse", true, true)]);
        let ev = connection_events(&mut prev, &b);
        assert_eq!(ev.len(), 2);
        assert_eq!(ev[0].event, "connected");
        assert_eq!(ev[0].body.get("addr").and_then(Json::as_str), Some("AA:BB:CC:DD:EE:02"));
        assert_eq!(ev[1].event, "disconnected");
        assert_eq!(ev[1].body.get("name").and_then(Json::as_str), Some("Buds"));
        assert!(ev.iter().all(|e| e.app.is_none()));
    }

    #[test]
    fn the_queue_hands_each_command_over_once() {
        queue(Cmd::Scan);
        queue(Cmd::Enable(false));
        assert_eq!(take_commands(), vec![Cmd::Scan, Cmd::Enable(false)]);
        assert!(take_commands().is_empty());
    }
}
