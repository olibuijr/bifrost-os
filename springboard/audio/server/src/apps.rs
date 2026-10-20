//! The applications of the audio server: what the daemon keeps and publishes about the processes that use audio, the
//! audio sessions they declare, the DSP plan of each device and the levels it measures. The decisions are the policy
//! crate's (`bifrost_audio_policy::{clients, sessions, dsp}`) and the measurements the engine's
//! (`bifrost_audio_engine::analysis`); this module is the control-thread state that joins them to the object tree.
//!
//! Published objects:
//! - the System object lists `PROCESSES` (and `DEVICES`);
//! - each connected process is a `Process` object (id `PROCESS_ID_BASE` + its process number, never reused) with
//!   `APP_ID`, `PROCESS_PID`, `RUNNING_OUTPUT`, `RUNNING_INPUT` and the session (`SESSION_CATEGORY`, `SESSION_ACTIVE`,
//!   `SESSION_MIXES`, `SESSION_DUCKS`). The process itself (or a client with policy rights) sets the session
//!   properties; the arbiter then decides, and the server applies what it returns to the application's streams;
//! - each device is a `Device` object (the id the control side gave it) with its name, uid, transport, whether it runs,
//!   and per scope the `LEVEL` of what it plays or captures, the `DSP_PLAN` that runs for it and the `DSP_LATENCY`.
//!
//! Single-threaded: everything here runs on the control thread. The audio thread only sees the results (the session gain
//! of a stream, the DSP plan in the device's mailbox, the level tap it writes).

use crate::io::log;
use crate::objects::ObjectTree;
use bifrost_audio_driver::transport::Transport;
use bifrost_audio_driver::DeviceDesc;
use bifrost_audio_engine::analysis::{AnalysisHub, Measurement};
use bifrost_audio_policy::clients::Applications;
use bifrost_audio_policy::dsp::{Assignment, DspManager, Origin};
use bifrost_audio_policy::sessions::{Action, Arbiter, Category, Denied, Direction, Options};
use bifrost_audio_types::{props, AudioError, Class, ObjectId, PropertyAddress, PropertyValue, Scope, SYSTEM_OBJECT};
use std::collections::HashMap;

/// Process `n` is object `PROCESS_ID_BASE + n`.
pub const PROCESS_ID_BASE: u32 = 0x1_0000;

fn g(selector: bifrost_audio_types::FourCC) -> PropertyAddress {
    PropertyAddress::global(selector)
}

/// The object id of a process number.
pub fn process_object(number: u32) -> ObjectId {
    ObjectId(PROCESS_ID_BASE + number)
}

/// The `TRANSPORT` property value of a transport: a four-character code.
pub fn transport_code(t: &Transport) -> u32 {
    let code: &[u8; 4] = match t {
        Transport::BuiltIn => b"bltn",
        Transport::Pci => b"pci ",
        Transport::Usb => b"usb ",
        Transport::Hdmi => b"hdmi",
        Transport::Bluetooth => b"blue",
        Transport::Network => b"netw",
        Transport::Virtual => b"virt",
        Transport::Aggregate => b"grup",
        Transport::Other(_) => b"unkn",
    };
    u32::from_be_bytes(*code)
}

/// The text of a DSP plan: one `kind id` line per stage in the order they run, with `(app)` after a stage an application
/// added.
pub fn plan_text(a: &Assignment) -> String {
    a.stages
        .iter()
        .map(|p| format!("{} {}{}", p.stage.kind.name(), p.stage.id, if p.origin == Origin::App { " (app)" } else { "" }))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The default-device selectors, in the order of `AppState::pinned`.
const DEFAULT_ROLES: [bifrost_audio_types::FourCC; 3] = [props::DEFAULT_OUTPUT, props::DEFAULT_INPUT, props::DEFAULT_SOUND_EFFECTS];

/// What the choice of a default device knows about a device.
#[derive(Clone, Debug)]
struct Candidate {
    uid: String,
    name: String,
    transport: String,
    output: bool,
    input: bool,
}

impl Candidate {
    fn of(desc: &DeviceDesc) -> Candidate {
        Candidate { uid: desc.uid.clone(), name: desc.name.clone(), transport: desc.transport.clone(), output: desc.output_channels > 0, input: desc.input_channels > 0 }
    }

    /// Lower is better; `None` when the device is never a default (virtual loopback and null devices).
    fn rank(&self) -> Option<u32> {
        match self.transport.as_str() {
            "virtual" => None,
            "hdmi" => {
                // a connected sink gives the device its monitor's name; a bare port keeps the generic PCM name
                let generic = self.name.to_ascii_lowercase();
                Some(if generic.starts_with("hdmi") || generic.starts_with("displayport") { 2 } else { 1 })
            }
            _ => Some(0),
        }
    }
}

/// The index of the default playback (`output`) or capture device among `candidates`: the best rank, then the first.
fn pick_default(candidates: &[Candidate], output: bool) -> Option<usize> {
    candidates
        .iter()
        .enumerate()
        .filter(|(_, c)| if output { c.output } else { c.input })
        // a capture default is never an HDMI port
        .filter(|(_, c)| output || c.transport != "hdmi")
        .filter_map(|(i, c)| c.rank().map(|r| (r, i)))
        .min()
        .map(|(_, i)| i)
}

/// The session an application declared, as the properties show it.
#[derive(Clone, Copy, Debug, Default)]
struct Declared {
    category: Option<Category>,
    options: Options,
}

pub struct AppState {
    tree: ObjectTree,
    apps: Applications,
    arbiter: Arbiter,
    dsp: DspManager,
    hub: AnalysisHub,
    /// connection -> pid
    clients: HashMap<u32, i32>,
    /// pid -> (application id, connections of that pid)
    pids: HashMap<i32, (String, u32)>,
    declared: HashMap<String, Declared>,
    devices: Vec<ObjectId>,
    /// What the default-device choice looks at, per published device (same order as `devices`).
    candidates: Vec<Candidate>,
    /// Roles whose default a policy client set: the server's own choice no longer replaces them.
    pinned: [bool; 3],
    /// (device, output) -> whether the last published level was silence
    quiet: HashMap<(ObjectId, bool), bool>,
}

impl Default for AppState {
    fn default() -> Self {
        AppState::new()
    }
}

impl AppState {
    pub fn new() -> AppState {
        let mut tree = ObjectTree::new();
        tree.define(SYSTEM_OBJECT, g(props::PROCESSES), PropertyValue::Objects(Vec::new())).expect("the system object carries the process list");
        tree.define(SYSTEM_OBJECT, g(props::DEVICES), PropertyValue::Objects(Vec::new())).expect("the system object carries the device list");
        AppState {
            tree,
            apps: Applications::new(),
            arbiter: Arbiter::new(),
            dsp: DspManager::new(),
            hub: AnalysisHub::new(),
            clients: HashMap::new(),
            pids: HashMap::new(),
            declared: HashMap::new(),
            devices: Vec::new(),
            candidates: Vec::new(),
            pinned: [false; 3],
            quiet: HashMap::new(),
        }
        .with_default_properties()
    }

    /// The system object carries the default device of each role. Until a device is chosen they read 0, which the HAL
    /// client reports as "no default device"; the server chooses (`choose_defaults`) when no policy process does.
    fn with_default_properties(mut self) -> AppState {
        for role in DEFAULT_ROLES {
            self.tree.define(SYSTEM_OBJECT, g(role), PropertyValue::Object(ObjectId(0))).expect("the system object carries the default devices");
        }
        self
    }

    /// Chooses the default device of every role the policy has not set: the first real playback device (HDMI and
    /// DisplayPort outputs only after the others, those with a monitor name before those without) and the first real
    /// capture device. The loopback and null devices are never a default. The sound-effects role follows the output.
    fn choose_defaults(&mut self) {
        let output = pick_default(&self.candidates, true);
        let input = pick_default(&self.candidates, false);
        for (slot, (selector, choice)) in [(props::DEFAULT_OUTPUT, output), (props::DEFAULT_INPUT, input), (props::DEFAULT_SOUND_EFFECTS, output)].into_iter().enumerate() {
            if self.pinned[slot] {
                continue;
            }
            let Some(index) = choice else { continue };
            let id = self.devices[index];
            if self.tree.get(SYSTEM_OBJECT, g(selector)) != Ok(PropertyValue::Object(id)) {
                let c = &self.candidates[index];
                log("default", &format!("{} is now {} ({}, {})", props::lookup(selector).map_or("default", |d| d.name), c.uid, c.name, c.transport));
                let _ = self.tree.update(SYSTEM_OBJECT, g(selector), PropertyValue::Object(id));
            }
        }
    }

    // -- the object tree ------------------------------------------------------------------------------------------

    /// Whether this state publishes `id` (the system object, a device it was told about, a process).
    pub fn owns(&self, id: ObjectId) -> bool {
        self.tree.class(id).is_ok()
    }

    pub fn get(&self, object: ObjectId, address: PropertyAddress) -> Result<PropertyValue, AudioError> {
        self.tree.get(object, address)
    }

    pub fn listen(&mut self, client: u32, object: ObjectId, address: PropertyAddress) -> Result<(), AudioError> {
        self.tree.listen(client, object, address)
    }

    pub fn unlisten(&mut self, client: u32, object: ObjectId, address: PropertyAddress) -> Result<(), AudioError> {
        self.tree.unlisten(client, object, address)
    }

    pub fn owned(&self, object: ObjectId) -> Result<Vec<ObjectId>, AudioError> {
        self.tree.owned(object)
    }

    /// The change notices of this turn, one per listening client.
    pub fn drain_changes(&mut self) -> Vec<(u32, Vec<(ObjectId, PropertyAddress)>)> {
        self.tree.flush()
    }

    /// Publishes a device the control side knows under `id`.
    pub fn add_device(&mut self, id: ObjectId, desc: &DeviceDesc) {
        if self.tree.add_with_id(Class::Device, SYSTEM_OBJECT, id).is_err() {
            return;
        }
        let t = &mut self.tree;
        let _ = t.define(id, g(props::NAME), PropertyValue::String(desc.name.clone()));
        let _ = t.define(id, g(props::DEVICE_UID), PropertyValue::String(desc.uid.clone()));
        let _ = t.define(id, g(props::TRANSPORT), PropertyValue::U32(transport_code(&Transport::from_name(&desc.transport))));
        let _ = t.define(id, g(props::IS_RUNNING), PropertyValue::Bool(false));
        for (scope, channels) in [(Scope::Output, desc.output_channels), (Scope::Input, desc.input_channels)] {
            if channels == 0 {
                continue;
            }
            let _ = t.define(id, PropertyAddress::scoped(props::LEVEL, scope), PropertyValue::String(String::new()));
            let _ = t.define(id, PropertyAddress::scoped(props::DSP_PLAN, scope), PropertyValue::String(String::new()));
            let _ = t.define(id, PropertyAddress::scoped(props::DSP_LATENCY, scope), PropertyValue::U32(0));
        }
        self.devices.push(id);
        self.candidates.push(Candidate::of(desc));
        let _ = self.tree.update(SYSTEM_OBJECT, g(props::DEVICES), PropertyValue::Objects(self.devices.clone()));
        self.choose_defaults();
    }

    /// A device started or stopped.
    pub fn device_running(&mut self, id: ObjectId, running: bool) {
        let _ = self.tree.update(id, g(props::IS_RUNNING), PropertyValue::Bool(running));
    }

    /// Publishes the plan that runs for a device in one direction and the latency it adds.
    pub fn publish_dsp(&mut self, id: ObjectId, output: bool, text: String, latency: u32) {
        let scope = if output { Scope::Output } else { Scope::Input };
        let _ = self.tree.update(id, PropertyAddress::scoped(props::DSP_PLAN, scope), PropertyValue::String(text));
        let _ = self.tree.update(id, PropertyAddress::scoped(props::DSP_LATENCY, scope), PropertyValue::U32(latency));
    }

    /// Analyses a window of what a device plays (`output`) or captures and publishes the measurement as the device's
    /// `LEVEL`. Silence after silence publishes nothing, so an idle device does not wake its listeners.
    pub fn publish_level(&mut self, id: ObjectId, output: bool, channels: &[&[f32]], rate: u32) {
        let tap = format!("{}:{}", id.0, if output { "out" } else { "in" });
        self.hub.publish(&tap, channels, rate);
        let Some(m) = self.hub.latest(&tap) else { return };
        let silent = m.peak == 0.0 && m.rms == 0.0;
        let text = m.encode();
        if self.quiet.insert((id, output), silent) == Some(true) && silent {
            return;
        }
        let scope = if output { Scope::Output } else { Scope::Input };
        let _ = self.tree.update(id, PropertyAddress::scoped(props::LEVEL, scope), PropertyValue::String(text));
    }

    /// The newest measurement of a device's output or input.
    pub fn level(&self, id: ObjectId, output: bool) -> Option<&Measurement> {
        self.hub.latest(&format!("{}:{}", id.0, if output { "out" } else { "in" }))
    }

    // -- the DSP plan ---------------------------------------------------------------------------------------------

    pub fn dsp(&self) -> &DspManager {
        &self.dsp
    }

    pub fn set_dsp(&mut self, manager: DspManager) {
        self.dsp = manager;
    }

    // -- processes and sessions -----------------------------------------------------------------------------------

    /// A client said Hello: its process becomes an object (a second connection of the same pid shares it).
    pub fn client_hello(&mut self, client: u32, app: &str, pid: i32) {
        if self.clients.contains_key(&client) {
            return;
        }
        self.clients.insert(client, pid);
        let entry = self.pids.entry(pid).or_insert_with(|| (app.to_string(), 0));
        entry.1 += 1;
        let first = entry.1 == 1;
        let app = entry.0.clone();
        let number = self.apps.connect(&app, pid);
        if !first {
            return;
        }
        let id = process_object(number);
        if self.tree.add_with_id(Class::Process, SYSTEM_OBJECT, id).is_err() {
            return;
        }
        let d = self.declared.get(&app).copied().unwrap_or_default();
        let t = &mut self.tree;
        let _ = t.define(id, g(props::APP_ID), PropertyValue::String(app.clone()));
        let _ = t.define(id, g(props::PROCESS_PID), PropertyValue::Pid(pid));
        let _ = t.define(id, g(props::RUNNING_OUTPUT), PropertyValue::Bool(false));
        let _ = t.define(id, g(props::RUNNING_INPUT), PropertyValue::Bool(false));
        let _ = t.define(id, g(props::SESSION_CATEGORY), PropertyValue::String(d.category.map_or("", |c| c.name()).to_string()));
        let _ = t.define(id, g(props::SESSION_ACTIVE), PropertyValue::Bool(self.arbiter.is_active(&app)));
        let _ = t.define(id, g(props::SESSION_MIXES), PropertyValue::Bool(d.options.mix_with_others));
        let _ = t.define(id, g(props::SESSION_DUCKS), PropertyValue::Bool(d.options.duck_others));
        self.publish_processes();
    }

    /// A connection closed. When it was the last of its process, the process object goes, and so does the application's
    /// session if no other process of the application remains; the actions that releases are returned.
    pub fn client_gone(&mut self, client: u32) -> Vec<Action> {
        self.tree.drop_client(client);
        let Some(pid) = self.clients.remove(&client) else { return Vec::new() };
        let Some(entry) = self.pids.get_mut(&pid) else { return Vec::new() };
        entry.1 = entry.1.saturating_sub(1);
        if entry.1 > 0 {
            return Vec::new();
        }
        let app = self.pids.remove(&pid).map(|(a, _)| a).unwrap_or_default();
        if let Some(p) = self.apps.disconnect(pid) {
            let _ = self.tree.remove(process_object(p.object_id));
        }
        self.publish_processes();
        if self.pids.values().any(|(a, _)| *a == app) {
            return Vec::new();
        }
        self.declared.remove(&app);
        self.arbiter.remove(&app)
    }

    fn publish_processes(&mut self) {
        let ids: Vec<ObjectId> = self.apps.processes().iter().map(|p| process_object(p.object_id)).collect();
        let _ = self.tree.update(SYSTEM_OBJECT, g(props::PROCESSES), PropertyValue::Objects(ids));
    }

    /// The application id a connection said in its Hello.
    pub fn app_of(&self, client: u32) -> Option<&str> {
        self.clients.get(&client).and_then(|pid| self.pids.get(pid)).map(|(a, _)| a.as_str())
    }

    /// The connections of an application.
    pub fn clients_of(&self, app: &str) -> Vec<u32> {
        let mut v: Vec<u32> = self.clients.iter().filter(|(_, pid)| self.pids.get(*pid).map_or(false, |(a, _)| a == app)).map(|(c, _)| *c).collect();
        v.sort_unstable();
        v
    }

    /// Whether the application's session allows a stream of `direction` now (an application with no session is
    /// unmanaged).
    pub fn check_stream(&self, client: u32, direction: Direction) -> Result<(), Denied> {
        match self.app_of(client) {
            Some(app) => self.arbiter.check_stream(app, direction),
            None => Ok(()),
        }
    }

    /// The gain the mixer applies to the application's output.
    pub fn gain_of(&self, app: &str) -> f32 {
        self.arbiter.gain(app)
    }

    pub fn category_of(&self, app: &str) -> Option<Category> {
        self.arbiter.category(app)
    }

    /// A connection's stream started running in `direction` (`stream` is unique per stream and direction).
    pub fn stream_started(&mut self, client: u32, stream: u32, direction: Direction) {
        let Some(&pid) = self.clients.get(&client) else { return };
        if self.apps.stream_started(pid, stream, direction).is_ok() {
            self.sync_running();
        }
    }

    pub fn stream_stopped(&mut self, stream: u32) {
        if self.apps.stream_stopped(stream) {
            self.sync_running();
        }
    }

    /// The applications with a stream running in `direction` (the menu-bar indicator reads the input list).
    pub fn apps_running(&self, direction: Direction) -> Vec<String> {
        self.apps.apps_running(direction).into_iter().map(str::to_string).collect()
    }

    fn sync_running(&mut self) {
        let list: Vec<(ObjectId, bool, bool)> = self
            .apps
            .processes()
            .iter()
            .map(|p| (process_object(p.object_id), p.is_running(Direction::Output), p.is_running(Direction::Input)))
            .collect();
        for (id, out, inp) in list {
            let _ = self.tree.update(id, g(props::RUNNING_OUTPUT), PropertyValue::Bool(out));
            let _ = self.tree.update(id, g(props::RUNNING_INPUT), PropertyValue::Bool(inp));
        }
    }

    /// Writes an application's session into the properties of each of its processes.
    fn sync_session(&mut self, app: &str) {
        let d = self.declared.get(app).copied().unwrap_or_default();
        let active = self.arbiter.is_active(app);
        let ids: Vec<ObjectId> = self.apps.processes().iter().filter(|p| p.app_id == app).map(|p| process_object(p.object_id)).collect();
        for id in ids {
            let _ = self.tree.update(id, g(props::SESSION_CATEGORY), PropertyValue::String(d.category.map_or("", |c| c.name()).to_string()));
            let _ = self.tree.update(id, g(props::SESSION_ACTIVE), PropertyValue::Bool(active));
            let _ = self.tree.update(id, g(props::SESSION_MIXES), PropertyValue::Bool(d.options.mix_with_others));
            let _ = self.tree.update(id, g(props::SESSION_DUCKS), PropertyValue::Bool(d.options.duck_others));
        }
    }

    /// A client's set. Only the process itself or a client with policy rights may change a process's session; every
    /// other property is read-only. Returns the actions the arbiter decided, for the caller to apply to the streams.
    pub fn set(&mut self, client: u32, policy: bool, object: ObjectId, address: PropertyAddress, value: PropertyValue) -> Result<Vec<Action>, AudioError> {
        if self.tree.class(object)? != Class::Process {
            self.tree.set(object, address, value)?;
            if object == SYSTEM_OBJECT {
                if let Some(slot) = DEFAULT_ROLES.iter().position(|r| *r == address.selector) {
                    self.pinned[slot] = true;
                }
            }
            return Ok(Vec::new());
        }
        // the property must exist at this address before anything else is judged
        self.tree.get(object, address)?;
        let number = object.0.checked_sub(PROCESS_ID_BASE).ok_or(AudioError::UnknownObject)?;
        let (owner_pid, app) = match self.apps.by_object(number) {
            Some(p) => (p.pid, p.app_id.clone()),
            None => return Err(AudioError::UnknownObject),
        };
        let selector = address.selector;
        let session = selector == props::SESSION_CATEGORY || selector == props::SESSION_ACTIVE || selector == props::SESSION_MIXES || selector == props::SESSION_DUCKS;
        if session && !policy && self.clients.get(&client) != Some(&owner_pid) {
            return Err(AudioError::PermissionDenied);
        }
        let mut d = self.declared.get(&app).copied().unwrap_or_default();
        let actions = match (selector, &value) {
            (s, PropertyValue::String(name)) if s == props::SESSION_CATEGORY => {
                if name.is_empty() {
                    d = Declared { options: d.options, ..Declared::default() };
                    self.arbiter.remove(&app)
                } else {
                    let category = Category::from_name(name).ok_or(AudioError::BadValue)?;
                    d.category = Some(category);
                    self.arbiter.set_category(&app, category, d.options)
                }
            }
            (s, PropertyValue::Bool(on)) if s == props::SESSION_ACTIVE => {
                if d.category.is_none() {
                    return Err(AudioError::BadValue);
                }
                self.arbiter.set_active(&app, *on)
            }
            (s, PropertyValue::Bool(on)) if s == props::SESSION_MIXES || s == props::SESSION_DUCKS => {
                if s == props::SESSION_MIXES {
                    d.options.mix_with_others = *on;
                } else {
                    d.options.duck_others = *on;
                }
                match d.category {
                    Some(category) => self.arbiter.set_category(&app, category, d.options),
                    None => Vec::new(),
                }
            }
            _ => {
                // read-only, or the wrong type: the tree gives the matching error
                self.tree.set(object, address, value.clone())?;
                return Ok(Vec::new());
            }
        };
        self.declared.insert(app.clone(), d);
        self.sync_session(&app);
        Ok(actions)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_audio_policy::dsp::{Chain, Kind, Stage};
    use bifrost_audio_policy::sessions::DUCK_GAIN;

    fn dev(uid: &str, transport: &str, out: u32, inp: u32) -> DeviceDesc {
        DeviceDesc {
            uid: uid.into(),
            name: format!("{uid} name"),
            manufacturer: "t".into(),
            transport: transport.into(),
            input_channels: inp,
            output_channels: out,
            rates: vec![(48_000.0, 48_000.0)],
            buffer_frames_range: (16, 4096),
            clock_domain: 1,
        }
    }

    fn text(v: Result<PropertyValue, AudioError>) -> String {
        match v {
            Ok(PropertyValue::String(s)) => s,
            other => panic!("{other:?}"),
        }
    }

    fn processes(a: &AppState) -> Vec<ObjectId> {
        match a.get(SYSTEM_OBJECT, g(props::PROCESSES)) {
            Ok(PropertyValue::Objects(v)) => v,
            other => panic!("{other:?}"),
        }
    }

    fn set_text(a: &mut AppState, client: u32, policy: bool, object: ObjectId, selector: bifrost_audio_types::FourCC, v: &str) -> Result<Vec<Action>, AudioError> {
        a.set(client, policy, object, g(selector), PropertyValue::String(v.into()))
    }

    fn set_bool(a: &mut AppState, client: u32, policy: bool, object: ObjectId, selector: bifrost_audio_types::FourCC, v: bool) -> Result<Vec<Action>, AudioError> {
        a.set(client, policy, object, g(selector), PropertyValue::Bool(v))
    }

    #[test]
    fn a_process_is_published_with_its_application_and_pid_and_goes_when_its_last_connection_does() {
        let mut a = AppState::new();
        a.client_hello(1, "is.olibuijr.music", 100);
        a.client_hello(2, "is.olibuijr.music", 100);
        a.client_hello(3, "is.olibuijr.notes", 200);
        let ids = processes(&a);
        assert_eq!(ids, vec![process_object(1), process_object(2)], "a second connection of one pid shares its process");
        assert_eq!(text(a.get(ids[0], g(props::APP_ID))), "is.olibuijr.music");
        assert_eq!(a.get(ids[1], g(props::PROCESS_PID)), Ok(PropertyValue::Pid(200)));
        assert_eq!(a.get(ids[0], g(props::RUNNING_OUTPUT)), Ok(PropertyValue::Bool(false)));
        assert_eq!(a.get(ids[0], g(props::SESSION_CATEGORY)), Ok(PropertyValue::String(String::new())));
        assert_eq!((a.app_of(2), a.clients_of("is.olibuijr.music")), (Some("is.olibuijr.music"), vec![1, 2]));
        a.client_gone(1);
        assert_eq!(processes(&a).len(), 2);
        a.client_gone(2);
        assert_eq!(processes(&a), vec![process_object(2)]);
        assert_eq!(a.get(ids[0], g(props::APP_ID)), Err(AudioError::UnknownObject));
        a.client_hello(4, "is.olibuijr.music", 300);
        assert_eq!(processes(&a), vec![process_object(2), process_object(3)], "ids are not reused");
    }

    #[test]
    fn running_streams_show_on_the_process() {
        let mut a = AppState::new();
        a.client_hello(1, "app", 10);
        let p = process_object(1);
        a.stream_started(1, 2, Direction::Output);
        a.stream_started(1, 3, Direction::Input);
        assert_eq!(a.get(p, g(props::RUNNING_OUTPUT)), Ok(PropertyValue::Bool(true)));
        assert_eq!(a.get(p, g(props::RUNNING_INPUT)), Ok(PropertyValue::Bool(true)));
        assert_eq!((a.apps_running(Direction::Output), a.apps_running(Direction::Input)), (vec!["app".to_string()], vec!["app".to_string()]));
        a.stream_stopped(3);
        assert_eq!(a.get(p, g(props::RUNNING_INPUT)), Ok(PropertyValue::Bool(false)));
        assert_eq!(a.get(p, g(props::RUNNING_OUTPUT)), Ok(PropertyValue::Bool(true)));
        a.stream_stopped(2);
        a.stream_stopped(2);
        assert!(a.apps_running(Direction::Output).is_empty());
    }

    #[test]
    fn only_the_process_or_the_policy_client_sets_its_session() {
        let mut a = AppState::new();
        a.client_hello(1, "player", 10);
        a.client_hello(2, "other", 20);
        let p = process_object(1);
        assert_eq!(set_text(&mut a, 2, false, p, props::SESSION_CATEGORY, "playback"), Err(AudioError::PermissionDenied));
        assert_eq!(set_text(&mut a, 1, false, p, props::SESSION_CATEGORY, "loud"), Err(AudioError::BadValue));
        assert_eq!(set_text(&mut a, 1, false, p, props::SESSION_CATEGORY, "playback"), Ok(vec![]));
        assert_eq!(text(a.get(p, g(props::SESSION_CATEGORY))), "playback");
        assert_eq!(set_text(&mut a, 3, true, p, props::SESSION_CATEGORY, "ambient"), Ok(vec![]), "a policy client may");
        assert_eq!(a.category_of("player"), Some(Category::Ambient));
        assert_eq!(a.set(1, false, p, g(props::APP_ID), PropertyValue::String("x".into())), Err(AudioError::NotSettable));
        assert_eq!(a.set(1, false, p, g(props::SESSION_ACTIVE), PropertyValue::String("x".into())), Err(AudioError::BadValueType));
        assert_eq!(a.set(1, false, p, PropertyAddress::scoped(props::SESSION_ACTIVE, Scope::Output), PropertyValue::Bool(true)), Err(AudioError::UnknownProperty));
        assert_eq!(set_bool(&mut a, 2, false, p, props::SESSION_ACTIVE, true), Err(AudioError::PermissionDenied));
    }

    #[test]
    fn an_active_session_needs_a_category_and_the_options_apply_to_it() {
        let mut a = AppState::new();
        a.client_hello(1, "nav", 10);
        let p = process_object(1);
        assert_eq!(set_bool(&mut a, 1, false, p, props::SESSION_ACTIVE, true), Err(AudioError::BadValue));
        set_bool(&mut a, 1, false, p, props::SESSION_DUCKS, true).unwrap();
        assert_eq!(a.get(p, g(props::SESSION_DUCKS)), Ok(PropertyValue::Bool(true)));
        set_text(&mut a, 1, false, p, props::SESSION_CATEGORY, "playback").unwrap();
        set_bool(&mut a, 1, false, p, props::SESSION_ACTIVE, true).unwrap();
        assert_eq!(a.get(p, g(props::SESSION_ACTIVE)), Ok(PropertyValue::Bool(true)));
        // clearing the category ends the session
        set_text(&mut a, 1, false, p, props::SESSION_CATEGORY, "").unwrap();
        assert_eq!(a.get(p, g(props::SESSION_ACTIVE)), Ok(PropertyValue::Bool(false)));
        assert_eq!(a.category_of("nav"), None);
    }

    #[test]
    fn a_session_interrupts_pauses_ducks_and_denies_streams_by_category() {
        let mut a = AppState::new();
        a.client_hello(1, "music", 10);
        a.client_hello(2, "video", 20);
        a.client_hello(3, "nav", 30);
        let (music, video, nav) = (process_object(1), process_object(2), process_object(3));
        set_text(&mut a, 1, false, music, props::SESSION_CATEGORY, "playback").unwrap();
        set_text(&mut a, 2, false, video, props::SESSION_CATEGORY, "playback").unwrap();
        set_text(&mut a, 3, false, nav, props::SESSION_CATEGORY, "record").unwrap();
        assert_eq!(a.check_stream(3, Direction::Output), Err(Denied::Category(Category::Record)));
        assert_eq!(a.check_stream(3, Direction::Input), Ok(()));
        set_bool(&mut a, 1, false, music, props::SESSION_ACTIVE, true).unwrap();
        let actions = set_bool(&mut a, 2, false, video, props::SESSION_ACTIVE, true).unwrap();
        assert_eq!(actions, vec![Action::Pause("music".into())]);
        assert_eq!(a.gain_of("music"), 0.0);
        assert_eq!(a.check_stream(1, Direction::Output), Err(Denied::Interrupted));
        let actions = set_bool(&mut a, 2, false, video, props::SESSION_ACTIVE, false).unwrap();
        assert_eq!(actions, vec![Action::Resume("music".into())]);
        assert_eq!(a.gain_of("music"), 1.0);
        // a ducking session lowers the others
        set_text(&mut a, 3, false, nav, props::SESSION_CATEGORY, "playback").unwrap();
        set_bool(&mut a, 3, false, nav, props::SESSION_DUCKS, true).unwrap();
        let actions = set_bool(&mut a, 3, false, nav, props::SESSION_ACTIVE, true).unwrap();
        assert!(actions.contains(&Action::Duck("music".into(), DUCK_GAIN)));
        assert_eq!(a.gain_of("music"), DUCK_GAIN);
        // the session goes with the process
        let released = a.client_gone(3);
        assert!(released.contains(&Action::Unduck("music".into())));
        assert_eq!(a.gain_of("music"), 1.0);
    }

    #[test]
    fn two_processes_of_one_application_share_its_session() {
        let mut a = AppState::new();
        a.client_hello(1, "web", 10);
        a.client_hello(2, "web", 11);
        set_text(&mut a, 1, false, process_object(1), props::SESSION_CATEGORY, "playback").unwrap();
        assert_eq!(text(a.get(process_object(2), g(props::SESSION_CATEGORY))), "playback", "both processes show it");
        a.client_gone(1);
        assert_eq!(a.category_of("web"), Some(Category::Playback), "the other process still holds the application");
        a.client_gone(2);
        assert_eq!(a.category_of("web"), None);
    }

    #[test]
    fn devices_are_published_with_their_transport_and_levels_reach_listeners_when_they_change() {
        let mut a = AppState::new();
        let d = ObjectId(1000);
        a.add_device(d, &dev("pci:0", "pci", 2, 0));
        a.add_device(ObjectId(1001), &dev("bt:1", "bluetooth", 2, 1));
        assert_eq!(a.get(SYSTEM_OBJECT, g(props::DEVICES)), Ok(PropertyValue::Objects(vec![d, ObjectId(1001)])));
        assert_eq!(text(a.get(d, g(props::DEVICE_UID))), "pci:0");
        assert_eq!(a.get(d, g(props::TRANSPORT)), Ok(PropertyValue::U32(u32::from_be_bytes(*b"pci "))));
        assert_eq!(a.get(ObjectId(1001), g(props::TRANSPORT)), Ok(PropertyValue::U32(u32::from_be_bytes(*b"blue"))));
        assert_eq!(a.get(d, PropertyAddress::scoped(props::LEVEL, Scope::Input)), Err(AudioError::UnknownProperty), "no input side");
        assert!(a.get(ObjectId(1001), PropertyAddress::scoped(props::LEVEL, Scope::Input)).is_ok());
        assert_eq!(a.owned(SYSTEM_OBJECT).unwrap().len(), 2);

        let level = PropertyAddress::scoped(props::LEVEL, Scope::Output);
        a.listen(9, d, level).unwrap();
        let sine: Vec<f32> = (0..4096).map(|i| 0.5 * (2.0 * std::f32::consts::PI * 1000.0 * i as f32 / 48_000.0).sin()).collect();
        a.publish_level(d, true, &[&sine, &sine], 48_000);
        let m = Measurement::decode(&text(a.get(d, level))).expect("the level is the measurement's text form");
        assert!((m.peak - 0.5).abs() < 0.01 && (m.bands[4] - 0.5).abs() < 0.02, "{m:?}");
        assert_eq!(a.level(d, true).map(|m| m.seq), Some(1));
        assert_eq!(a.drain_changes(), vec![(9, vec![(d, level)])]);
        // silence is published once, then not again
        let silence = vec![0.0f32; 4096];
        a.publish_level(d, true, &[&silence], 48_000);
        assert_eq!(a.drain_changes().len(), 1);
        a.publish_level(d, true, &[&silence], 48_000);
        assert!(a.drain_changes().is_empty(), "silence after silence wakes nobody");
        assert_eq!(a.level(d, true).map(|m| m.seq), Some(3), "it was measured all the same");
    }

    fn default_of(a: &AppState, selector: bifrost_audio_types::FourCC) -> PropertyValue {
        a.get(SYSTEM_OBJECT, g(selector)).unwrap()
    }

    #[test]
    fn the_default_output_is_the_first_real_playback_device_and_never_the_loopback() {
        let mut a = AppState::new();
        assert_eq!(default_of(&a, props::DEFAULT_OUTPUT), PropertyValue::Object(ObjectId(0)), "nothing chosen before a device exists");
        let mut lo = dev("loopback:out", "virtual", 2, 0);
        lo.name = "Loopback".into();
        a.add_device(ObjectId(1000), &lo);
        assert_eq!(default_of(&a, props::DEFAULT_OUTPUT), PropertyValue::Object(ObjectId(0)), "the loopback is never a default");
        let mut hdmi = dev("alsa:PCH:3", "hdmi", 2, 0);
        hdmi.name = "HDMI 0".into();
        a.add_device(ObjectId(1001), &hdmi);
        assert_eq!(default_of(&a, props::DEFAULT_OUTPUT), PropertyValue::Object(ObjectId(1001)), "an HDMI port when nothing else plays");
        let mut mon = dev("alsa:PCH:7", "hdmi", 2, 0);
        mon.name = "DELL U2720Q".into();
        a.add_device(ObjectId(1002), &mon);
        assert_eq!(default_of(&a, props::DEFAULT_OUTPUT), PropertyValue::Object(ObjectId(1002)), "a port with a monitor before a bare one");
        a.add_device(ObjectId(1003), &dev("alsa:PCH:0", "pci", 2, 2));
        assert_eq!(default_of(&a, props::DEFAULT_OUTPUT), PropertyValue::Object(ObjectId(1003)), "the analog output before any HDMI");
        assert_eq!(default_of(&a, props::DEFAULT_SOUND_EFFECTS), PropertyValue::Object(ObjectId(1003)));
        assert_eq!(default_of(&a, props::DEFAULT_INPUT), PropertyValue::Object(ObjectId(1003)));
        a.add_device(ObjectId(1004), &dev("alsa:PCH:2", "pci", 2, 0));
        assert_eq!(default_of(&a, props::DEFAULT_OUTPUT), PropertyValue::Object(ObjectId(1003)), "the first of equals stays");
    }

    #[test]
    fn a_default_the_policy_set_is_kept_when_more_devices_appear() {
        let mut a = AppState::new();
        a.add_device(ObjectId(1000), &dev("alsa:PCH:0", "pci", 2, 0));
        a.add_device(ObjectId(1001), &dev("usb:1", "usb", 2, 0));
        a.set(1, true, SYSTEM_OBJECT, g(props::DEFAULT_OUTPUT), PropertyValue::Object(ObjectId(1001))).unwrap();
        a.add_device(ObjectId(1002), &dev("alsa:PCH:2", "pci", 2, 0));
        assert_eq!(default_of(&a, props::DEFAULT_OUTPUT), PropertyValue::Object(ObjectId(1001)));
        assert_eq!(default_of(&a, props::DEFAULT_SOUND_EFFECTS), PropertyValue::Object(ObjectId(1000)), "the other roles keep following the server");
    }

    #[test]
    fn the_plan_and_the_running_state_are_published_per_device() {
        let mut a = AppState::new();
        let d = ObjectId(1000);
        a.add_device(d, &dev("pci:0", "pci", 2, 2));
        let mut m = DspManager::new();
        m.set_device_chain("pci:0", Direction::Output, Chain::new(vec![Stage::new("trim", Kind::Gain).with("db", -3.0), Stage::new("lim", Kind::Limiter)])).unwrap();
        a.set_dsp(m);
        let plan = a.dsp().assignment("pci:0", Direction::Output, None, None);
        a.publish_dsp(d, true, plan_text(&plan), plan.latency_frames);
        assert_eq!(text(a.get(d, PropertyAddress::scoped(props::DSP_PLAN, Scope::Output))), "gain trim\nlimiter lim");
        assert_eq!(a.get(d, PropertyAddress::scoped(props::DSP_LATENCY, Scope::Output)), Ok(PropertyValue::U32(0)));
        assert_eq!(text(a.get(d, PropertyAddress::scoped(props::DSP_PLAN, Scope::Input))), "", "the input side has no chain");
        a.device_running(d, true);
        assert_eq!(a.get(d, g(props::IS_RUNNING)), Ok(PropertyValue::Bool(true)));
    }

    #[test]
    fn an_app_stage_is_marked_in_the_plan_text() {
        let mut m = DspManager::new();
        m.set_app_chain("music", Chain::new(vec![Stage::new("boost", Kind::Eq).with("gain_db", 3.0)])).unwrap();
        let plan = m.assignment("any", Direction::Output, Some("music"), None);
        assert_eq!(plan_text(&plan), "eq boost (app)");
    }

    #[test]
    fn transports_have_distinct_codes() {
        let all = [Transport::BuiltIn, Transport::Pci, Transport::Usb, Transport::Hdmi, Transport::Bluetooth, Transport::Network, Transport::Virtual, Transport::Aggregate, Transport::Other("x".into())];
        let mut codes: Vec<u32> = all.iter().map(transport_code).collect();
        codes.sort_unstable();
        codes.dedup();
        assert_eq!(codes.len(), all.len());
    }
}
