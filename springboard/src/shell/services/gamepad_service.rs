//! The `gamepad` service: the game controllers on the input nodes, their connect and disconnect, and the button and
//! stick values of each one to the focused app.
//!
//! Discovery reads the kernel's sysfs view of each `eventN` node (its capability bitmaps and name), so no ioctl and no
//! `unsafe` are needed. A node that `bifrost_core::gamepad` accepts gets a reader thread that reads its 24-byte
//! `input_event` records and hands them to the tick through a channel; the tick feeds them to a per-controller
//! `bifrost_ui::gamepad::Gamepad`, applies the controller's saved button remap and sends each change to the focused app.
//!
//! Methods: `list` (the connected controllers) and `state` (`{"id": n}`: pressed buttons, axis values, direction pad).
//! Events: `connected` and `disconnected` (a controller, sent to every running app) and `input` (`{"id", "kind": ...}`,
//! sent to the focused app only; `kind` is `button`, `axis` or `dpad`).
//!
//! The Settings page (`game_controllers_page`) reads a snapshot the tick publishes: each controller with its battery,
//! its live state (the physical buttons) and its remap. See `snapshot` and `generation`.
use super::super::apphost;
use super::super::game_controllers_page::{self, Remap};
use super::super::prefs::Prefs;
use super::super::AppId;
use super::{Event, Shell, SystemService};
use bifrost_core::gamepad::{Capabilities, Controller, Event as PadEvent, Registry};
use bifrost_core::json::Json;
use bifrost_ui::controller_power::{Battery, BatteryState};
use bifrost_ui::gamepad::{Axis, Button, Change, Gamepad};
use std::collections::{BTreeSet, HashMap};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Mutex;

const SYSFS_INPUT: &str = "/sys/class/input";
const DEV_INPUT: &str = "/dev/input";
/// Seconds between rescans of the input nodes (the kernel's hot-plug is also seen by the chord watch).
const SCAN_EVERY: f64 = 0.5;
/// `struct input_event` on a 64-bit kernel: timeval (16), type (2), code (2), value (4).
const RECORD: usize = 24;

/// The wire names of the buttons.
pub const BUTTON_NAMES: [(Button, &str); 13] = [
    (Button::South, "south"),
    (Button::East, "east"),
    (Button::North, "north"),
    (Button::West, "west"),
    (Button::LeftShoulder, "leftShoulder"),
    (Button::RightShoulder, "rightShoulder"),
    (Button::LeftTrigger, "leftTrigger"),
    (Button::RightTrigger, "rightTrigger"),
    (Button::LeftStick, "leftStick"),
    (Button::RightStick, "rightStick"),
    (Button::Select, "select"),
    (Button::Start, "start"),
    (Button::Home, "home"),
];

/// The wire names of the analogue elements.
pub const AXIS_NAMES: [(Axis, &str); 6] = [
    (Axis::LeftX, "leftX"),
    (Axis::LeftY, "leftY"),
    (Axis::RightX, "rightX"),
    (Axis::RightY, "rightY"),
    (Axis::LeftTrigger, "leftTrigger"),
    (Axis::RightTrigger, "rightTrigger"),
];

pub fn button_name(button: Button) -> &'static str {
    BUTTON_NAMES.iter().find(|(b, _)| *b == button).map_or("", |(_, n)| n)
}

pub fn button_from_name(name: &str) -> Option<Button> {
    BUTTON_NAMES.iter().find(|(_, n)| *n == name).map(|(b, _)| *b)
}

pub fn axis_name(axis: Axis) -> &'static str {
    AXIS_NAMES.iter().find(|(a, _)| *a == axis).map_or("", |(_, n)| n)
}

pub fn axis_from_name(name: &str) -> Option<Axis> {
    AXIS_NAMES.iter().find(|(_, n)| *n == name).map(|(a, _)| *a)
}

/// One raw record from a controller's node.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Raw {
    id: u32,
    ty: u16,
    code: u16,
    value: i32,
}

/// One connected controller as the Settings page shows it: its name, battery, live state and remap.
#[derive(Clone, Debug, PartialEq)]
pub struct PadView {
    pub id: u32,
    pub name: String,
    pub battery: Battery,
    /// the physical buttons and axes as the controller sends them now
    pub pad: Gamepad,
    pub remap: Remap,
}

/// The published snapshot of the controllers, and a generation that moves on every change of it.
static VIEWS: Mutex<(u64, Vec<PadView>)> = Mutex::new((0, Vec::new()));

/// The controllers as last published.
pub fn snapshot() -> Vec<PadView> {
    VIEWS.lock().map(|g| g.1.clone()).unwrap_or_default()
}

/// Moves on every change of the snapshot.
pub fn generation() -> u64 {
    VIEWS.lock().map(|g| g.0).unwrap_or(0)
}

fn store(pads: Vec<PadView>) {
    if let Ok(mut g) = VIEWS.lock() {
        if g.1 != pads {
            g.0 += 1;
            g.1 = pads;
        }
    }
}

/// Replaces the remap a controller shows, after the Settings page saved one.
pub fn set_remap(name: &str, remap: Remap) {
    if let Ok(mut g) = VIEWS.lock() {
        for v in g.1.iter_mut().filter(|v| v.name == name) {
            v.remap = remap.clone();
        }
        g.0 += 1;
    }
}

pub struct GamepadService {
    sysfs: PathBuf,
    dev: PathBuf,
    registry: Registry,
    pads: HashMap<u32, Gamepad>,
    /// the sysfs device directory of each controller, read for its battery
    devices: HashMap<u32, PathBuf>,
    /// the `eventN` names seen by the last scans, controller or not
    seen: BTreeSet<String>,
    last_scan: Option<f64>,
    raw_tx: Sender<Raw>,
    raw_rx: Receiver<Raw>,
}

impl Default for GamepadService {
    fn default() -> Self {
        Self::with_roots(Path::new(SYSFS_INPUT), Path::new(DEV_INPUT))
    }
}

impl GamepadService {
    /// A service that reads the node list from `sysfs` (`/sys/class/input` in the shell) and opens nodes under `dev`.
    pub fn with_roots(sysfs: &Path, dev: &Path) -> Self {
        let (raw_tx, raw_rx) = mpsc::channel();
        GamepadService {
            sysfs: sysfs.to_path_buf(),
            dev: dev.to_path_buf(),
            registry: Registry::default(),
            pads: HashMap::new(),
            devices: HashMap::new(),
            seen: BTreeSet::new(),
            last_scan: None,
            raw_tx,
            raw_rx,
        }
    }

    fn node(&self, name: &str) -> String {
        self.dev.join(name).to_string_lossy().into_owned()
    }

    /// Rescans the input nodes: a new controller is registered and starts its reader, a vanished one is dropped.
    /// Returns the connect and disconnect events, sent to every running app.
    fn scan(&mut self) -> Vec<Event> {
        let mut now = BTreeSet::new();
        if let Ok(rd) = std::fs::read_dir(&self.sysfs) {
            for entry in rd.flatten() {
                let name = entry.file_name().to_string_lossy().into_owned();
                if name.len() > 5 && name.starts_with("event") && name[5..].bytes().all(|b| b.is_ascii_digit()) {
                    now.insert(name);
                }
            }
        }
        let mut out = Vec::new();
        let gone: Vec<String> = self.seen.difference(&now).cloned().collect();
        for name in gone {
            self.seen.remove(&name);
            let node = self.node(&name);
            if let Some(PadEvent::Disconnected(c)) = self.registry.disconnected(&node) {
                self.pads.remove(&c.id);
                self.devices.remove(&c.id);
                out.push(lifecycle("disconnected", &c));
            }
        }
        let new: Vec<String> = now.difference(&self.seen).cloned().collect();
        for name in new {
            self.seen.insert(name.clone());
            let device = self.sysfs.join(&name).join("device");
            let caps = Capabilities {
                keys: parse_bitmap(&read_text(&device.join("capabilities/key"))),
                abs: parse_bitmap(&read_text(&device.join("capabilities/abs"))),
            };
            let label = read_text(&device.join("name"));
            let node = self.node(&name);
            if let Some(PadEvent::Connected(c)) = self.registry.connected(&node, &label, &caps) {
                self.pads.insert(c.id, Gamepad::new());
                self.devices.insert(c.id, device.clone());
                spawn_reader(c.id, PathBuf::from(&node), self.raw_tx.clone());
                out.push(lifecycle("connected", &c));
            }
        }
        out
    }

    /// Feeds every record the readers queued and returns the changes, each with its controller id.
    fn drain(&mut self) -> Vec<(u32, Change)> {
        let mut out = Vec::new();
        while let Ok(raw) = self.raw_rx.try_recv() {
            if let Some(pad) = self.pads.get_mut(&raw.id) {
                if let Some(change) = pad.feed(raw.ty, raw.code, raw.value) {
                    out.push((raw.id, change));
                }
            }
        }
        out
    }

    /// The controllers in arrival order, with their battery, live state and the remap saved for each name.
    pub fn views(&self, prefs: &Prefs) -> Vec<PadView> {
        self.registry
            .controllers()
            .iter()
            .filter_map(|c| {
                let pad = self.pads.get(&c.id)?;
                let battery = self.devices.get(&c.id).map_or_else(Battery::unknown, |d| read_battery(d));
                Some(PadView { id: c.id, name: c.name.clone(), battery, pad: pad.clone(), remap: game_controllers_page::load(prefs, &c.name) })
            })
            .collect()
    }

    /// The answer to `list` and `state`. Needs no shell, so tests call it directly.
    fn answer(&self, method: &str, params: &Json) -> Result<Json, String> {
        match method {
            "list" => Ok(Json::Arr(self.registry.controllers().iter().map(controller_json).collect())),
            "state" => {
                let id = params
                    .get("id")
                    .and_then(|v| v.as_u64())
                    .ok_or_else(|| "state needs an id".to_string())? as u32;
                let pad = self.pads.get(&id).ok_or_else(|| format!("no controller {id}"))?;
                Ok(state_json(id, pad))
            }
            _ => Err(format!("gamepad has no method {method}")),
        }
    }
}

impl SystemService for GamepadService {
    fn name(&self) -> &'static str {
        "gamepad"
    }

    fn call(&mut self, _shell: &mut Shell, _app_id: &str, method: &str, params: &Json, _t: f64) -> Result<Json, String> {
        self.answer(method, params)
    }

    fn tick(&mut self, shell: &mut Shell, t: f64) -> Vec<Event> {
        let mut out = Vec::new();
        let scanned = self.last_scan.map_or(true, |last| t - last >= SCAN_EVERY);
        if scanned {
            self.last_scan = Some(t);
            out.extend(self.scan());
        }
        let changes = self.drain();
        let changed = !changes.is_empty();
        // input goes to the focused app only; with no SDK app in front the changes are dropped
        let ids: Vec<String> = apphost::installed().iter().map(|a| a.manifest.id.clone()).collect();
        let front = shell.desk.as_ref().and_then(|d| d.front()).map(|w| w.app);
        if let Some(app) = front.and_then(|app| focused_app(app, &ids)) {
            for (id, change) in changes {
                // the app receives the output its button is remapped to; the snapshot keeps the physical buttons
                let remap = self.registry.get(id).map(|c| game_controllers_page::load(&shell.prefs, &c.name)).unwrap_or_default();
                let change = remap_change(&remap, change);
                out.push(Event { app: Some(app.clone()), event: "input".into(), body: change_json(id, &change) });
            }
        }
        if scanned || changed {
            store(self.views(&shell.prefs));
        }
        out
    }
}

/// A button change through a remap: the button becomes the output it is mapped to. Axes and the direction pad pass.
pub fn remap_change(remap: &Remap, change: Change) -> Change {
    match change {
        Change::Button { button, pressed } => Change::Button { button: remap.target(button), pressed },
        other => other,
    }
}

/// The battery of a wireless controller. Its input node hangs off the HID device that owns the `power_supply` entry,
/// so the device directory and its parents are searched. No entry (a wired pad): unknown.
pub fn read_battery(device: &Path) -> Battery {
    for base in [device.to_path_buf(), device.join(".."), device.join("../.."), device.join("../../..")] {
        let Ok(rd) = std::fs::read_dir(base.join("power_supply")) else { continue };
        for entry in rd.flatten() {
            let dir = entry.path();
            let Ok(percent) = read_text(&dir.join("capacity")).parse::<u8>() else { continue };
            let state = match read_text(&dir.join("status")).as_str() {
                "Charging" => BatteryState::Charging,
                "Discharging" => BatteryState::Discharging,
                "Full" => BatteryState::Full,
                _ => BatteryState::Unknown,
            };
            return Battery::from_percent(percent, state);
        }
    }
    Battery::unknown()
}

/// The app id of a window: only an installed (SDK) app has one; the shell's own windows get no input.
pub fn focused_app(app: AppId, installed_ids: &[String]) -> Option<String> {
    match app {
        AppId::External(slot) => installed_ids.get(usize::from(slot)).cloned(),
        _ => None,
    }
}

/// Starts the reader of one controller's node. The thread ends when the read fails (the node went away).
fn spawn_reader(id: u32, node: PathBuf, tx: Sender<Raw>) {
    std::thread::spawn(move || {
        let Ok(mut file) = std::fs::File::open(&node) else { return };
        let mut rec = [0u8; RECORD];
        while file.read_exact(&mut rec).is_ok() {
            let raw = Raw {
                id,
                ty: u16::from_le_bytes([rec[16], rec[17]]),
                code: u16::from_le_bytes([rec[18], rec[19]]),
                value: i32::from_le_bytes([rec[20], rec[21], rec[22], rec[23]]),
            };
            if tx.send(raw).is_err() {
                return;
            }
        }
    });
}

/// Reads one sysfs attribute, trimmed; a missing attribute is empty.
fn read_text(path: &Path) -> String {
    std::fs::read_to_string(path).map(|s| s.trim().to_string()).unwrap_or_default()
}

/// Parses an `EVIOCGBIT`-style sysfs bitmap: hexadecimal machine words, most significant first. The result is the
/// byte bitmap `bifrost_core::gamepad::has_bit` reads (bit `code` is byte `code / 8`, bit `code % 8`).
pub fn parse_bitmap(text: &str) -> Vec<u8> {
    let words: Vec<u64> = text.split_whitespace().filter_map(|w| u64::from_str_radix(w, 16).ok()).collect();
    let mut out = Vec::with_capacity(words.len() * 8);
    for word in words.iter().rev() {
        out.extend_from_slice(&word.to_le_bytes());
    }
    out
}

fn controller_json(c: &Controller) -> Json {
    Json::Obj(vec![
        ("id".into(), Json::Num(f64::from(c.id))),
        ("name".into(), Json::Str(c.name.clone())),
        ("path".into(), Json::Str(c.path.clone())),
    ])
}

fn lifecycle(event: &str, c: &Controller) -> Event {
    Event { app: None, event: event.into(), body: controller_json(c) }
}

/// The body of an `input` event: the controller and the element that changed.
pub fn change_json(id: u32, change: &Change) -> Json {
    let mut fields: Vec<(String, Json)> = vec![("id".into(), Json::Num(f64::from(id)))];
    match *change {
        Change::Button { button, pressed } => {
            fields.push(("kind".into(), Json::Str("button".into())));
            fields.push(("name".into(), Json::Str(button_name(button).into())));
            fields.push(("pressed".into(), Json::Bool(pressed)));
        }
        Change::Axis { axis, value } => {
            fields.push(("kind".into(), Json::Str("axis".into())));
            fields.push(("name".into(), Json::Str(axis_name(axis).into())));
            fields.push(("value".into(), Json::Num(f64::from(value))));
        }
        Change::Dpad { x, y } => {
            fields.push(("kind".into(), Json::Str("dpad".into())));
            fields.push(("x".into(), Json::Num(f64::from(x))));
            fields.push(("y".into(), Json::Num(f64::from(y))));
        }
    }
    Json::Obj(fields)
}

/// The body of a `state` answer: the pressed buttons, every axis value and the direction pad.
fn state_json(id: u32, pad: &Gamepad) -> Json {
    let pressed: Vec<Json> = BUTTON_NAMES
        .iter()
        .filter(|(b, _)| pad.is_pressed(*b))
        .map(|(_, n)| Json::Str((*n).into()))
        .collect();
    let axes: Vec<(String, Json)> = AXIS_NAMES
        .iter()
        .map(|(a, n)| ((*n).to_string(), Json::Num(f64::from(pad.axis(*a)))))
        .collect();
    let (x, y) = pad.dpad();
    Json::Obj(vec![
        ("id".into(), Json::Num(f64::from(id))),
        ("buttons".into(), Json::Arr(pressed)),
        ("axes".into(), Json::Obj(axes)),
        ("dpad".into(), Json::Obj(vec![("x".into(), Json::Num(f64::from(x))), ("y".into(), Json::Num(f64::from(y)))])),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(tag: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
        let dir = std::env::temp_dir().join(format!("bifrost-gamepad-{tag}-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A sysfs node of a pad with the south button and both left-stick axes.
    fn make_pad(sysfs: &Path, name: &str, label: &str) {
        let device = sysfs.join(name).join("device");
        std::fs::create_dir_all(device.join("capabilities")).unwrap();
        std::fs::write(device.join("name"), format!("{label}\n")).unwrap();
        // BTN_SOUTH (0x130) is bit 48 of word 4: five 64-bit words, most significant first
        std::fs::write(device.join("capabilities/key"), "0001000000000000 0 0 0 0\n").unwrap();
        // ABS_X (0) and ABS_Y (1) in word 0
        std::fs::write(device.join("capabilities/abs"), "3\n").unwrap();
    }

    #[test]
    fn a_bitmap_is_read_most_significant_word_first() {
        let bits = parse_bitmap("0001000000000000 0 0 0 0");
        assert_eq!(bits.len(), 40);
        assert!(bifrost_core::gamepad::has_bit(&bits, 0x130));
        assert!(!bifrost_core::gamepad::has_bit(&bits, 0x131));
        assert!(bifrost_core::gamepad::has_bit(&parse_bitmap("3"), 0));
        assert!(parse_bitmap("").is_empty());
    }

    #[test]
    fn a_pad_node_connects_and_a_removed_node_disconnects() {
        let sysfs = temp_root("sys");
        make_pad(&sysfs, "event5", "  Test Pad  ");
        // a keyboard node has keys and no axes: it is not a controller
        let kb = sysfs.join("event6/device/capabilities");
        std::fs::create_dir_all(&kb).unwrap();
        std::fs::write(kb.join("key"), "0 0 0 0 0\n").unwrap();
        let mut svc = GamepadService::with_roots(&sysfs, Path::new("/nonexistent-dev-input"));

        let events = svc.scan();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event, "connected");
        assert_eq!(events[0].app, None);
        assert_eq!(events[0].body.get("name").and_then(|v| v.as_str()), Some("Test Pad"));
        assert_eq!(events[0].body.get("id").and_then(|v| v.as_u64()), Some(1));
        assert_eq!(svc.pads.len(), 1);

        // a second scan with no change reports nothing
        assert!(svc.scan().is_empty());

        std::fs::remove_dir_all(sysfs.join("event5")).unwrap();
        let gone = svc.scan();
        assert_eq!(gone.len(), 1);
        assert_eq!(gone[0].event, "disconnected");
        assert!(svc.pads.is_empty());
        assert!(svc.devices.is_empty());
        let _ = std::fs::remove_dir_all(&sysfs);
    }

    #[test]
    fn queued_records_become_button_and_stick_changes() {
        let mut svc = GamepadService::with_roots(Path::new("/nonexistent-sys"), Path::new("/nonexistent-dev"));
        svc.pads.insert(3, Gamepad::new());
        svc.raw_tx.send(Raw { id: 3, ty: 0x01, code: 0x130, value: 1 }).unwrap();
        // a record for an id that is not connected is dropped
        svc.raw_tx.send(Raw { id: 9, ty: 0x01, code: 0x130, value: 1 }).unwrap();
        // a key repeat changes nothing
        svc.raw_tx.send(Raw { id: 3, ty: 0x01, code: 0x130, value: 2 }).unwrap();
        svc.raw_tx.send(Raw { id: 3, ty: 0x03, code: 0x00, value: 32767 }).unwrap();
        let changes = svc.drain();
        assert_eq!(changes.len(), 2);
        assert_eq!(changes[0], (3, Change::Button { button: Button::South, pressed: true }));
        assert_eq!(changes[1].0, 3);
        assert!(matches!(changes[1].1, Change::Axis { axis: Axis::LeftX, value } if value > 0.99));
        assert!(svc.pads[&3].is_pressed(Button::South));
    }

    #[test]
    fn input_json_names_the_element_and_the_controller() {
        let b = change_json(2, &Change::Button { button: Button::Start, pressed: true });
        assert_eq!(b.get("kind").and_then(|v| v.as_str()), Some("button"));
        assert_eq!(b.get("name").and_then(|v| v.as_str()), Some("start"));
        assert_eq!(b.get("pressed"), Some(&Json::Bool(true)));
        let a = change_json(2, &Change::Axis { axis: Axis::RightTrigger, value: 0.5 });
        assert_eq!(a.get("name").and_then(|v| v.as_str()), Some("rightTrigger"));
        let d = change_json(2, &Change::Dpad { x: -1, y: 1 });
        assert_eq!(d.get("kind").and_then(|v| v.as_str()), Some("dpad"));
        assert_eq!(d.get("x"), Some(&Json::Num(-1.0)));
        assert_eq!(d.get("id"), Some(&Json::Num(2.0)));
    }

    #[test]
    fn list_and_state_answer_from_the_registry() {
        let mut svc = GamepadService::with_roots(Path::new("/nonexistent-sys"), Path::new("/nonexistent-dev"));
        assert_eq!(svc.answer("list", &Json::Null), Ok(Json::Arr(vec![])));
        assert!(svc.answer("state", &Json::Null).is_err());
        assert!(svc.answer("rumble", &Json::Null).is_err());
        let mut pad = Gamepad::new();
        pad.feed(0x01, 0x13b, 1);
        svc.pads.insert(1, pad);
        let state = svc.answer("state", &Json::Obj(vec![("id".into(), Json::Num(1.0))])).unwrap();
        assert_eq!(state.get("buttons"), Some(&Json::Arr(vec![Json::Str("start".into())])));
        assert!(svc.answer("state", &Json::Obj(vec![("id".into(), Json::Num(7.0))])).is_err());
    }

    #[test]
    fn names_round_trip_and_input_goes_to_sdk_apps_only() {
        for (b, n) in BUTTON_NAMES {
            assert_eq!(button_from_name(n), Some(b));
            assert_eq!(button_name(b), n);
        }
        for (a, n) in AXIS_NAMES {
            assert_eq!(axis_from_name(n), Some(a));
        }
        assert_eq!(button_from_name("triangle"), None);
        let ids = vec!["is.example.a".to_string(), "is.example.b".to_string()];
        assert_eq!(focused_app(AppId::External(1), &ids), Some("is.example.b".to_string()));
        assert_eq!(focused_app(AppId::External(4), &ids), None);
        assert_eq!(focused_app(AppId::Settings, &ids), None);
    }

    #[test]
    fn a_remap_changes_only_the_button_it_names() {
        let mut r = Remap::identity();
        r.assign(Button::South, Button::East);
        assert_eq!(remap_change(&r, Change::Button { button: Button::South, pressed: true }), Change::Button { button: Button::East, pressed: true });
        assert_eq!(remap_change(&r, Change::Button { button: Button::North, pressed: true }), Change::Button { button: Button::North, pressed: true });
        assert_eq!(remap_change(&r, Change::Dpad { x: 1, y: 0 }), Change::Dpad { x: 1, y: 0 });
    }

    #[test]
    fn the_battery_is_read_from_the_power_supply_of_the_hid_device() {
        let root = temp_root("battery");
        // the HID device owns power_supply/; the input device hangs under it at hid/input/input1
        let bat = root.join("hid/power_supply/ps-controller-battery-1");
        std::fs::create_dir_all(&bat).unwrap();
        std::fs::write(bat.join("capacity"), "72\n").unwrap();
        std::fs::write(bat.join("status"), "Discharging\n").unwrap();
        let device = root.join("hid/input/input1");
        std::fs::create_dir_all(&device).unwrap();
        assert_eq!(read_battery(&device), Battery::from_percent(72, BatteryState::Discharging));
        // a wired pad has no entry
        assert_eq!(read_battery(&root.join("wired/input/input2")), Battery::unknown());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn views_carry_the_name_the_live_state_and_the_saved_remap() {
        let sysfs = temp_root("views");
        make_pad(&sysfs, "event5", "Test Pad");
        let mut svc = GamepadService::with_roots(&sysfs, Path::new("/nonexistent-dev-input"));
        let _ = svc.scan();
        svc.pads.get_mut(&1).unwrap().feed(0x01, 0x130, 1);
        let mut prefs = Prefs::memory();
        prefs.set("gamepad_remap.test_pad", "south>east,east>south").unwrap();
        let views = svc.views(&prefs);
        assert_eq!(views.len(), 1);
        assert_eq!(views[0].id, 1);
        assert_eq!(views[0].name, "Test Pad");
        assert_eq!(views[0].battery, Battery::unknown());
        assert!(views[0].pad.is_pressed(Button::South));
        assert_eq!(views[0].remap.target(Button::South), Button::East);
        let _ = std::fs::remove_dir_all(&sysfs);
    }
}
