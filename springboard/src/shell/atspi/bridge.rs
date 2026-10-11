//! The AT-SPI2 bridge: exports the app-control tree on the accessibility bus so Orca and other screen readers can read and
//! operate BifrOSt apps. Only built with the `atspi` feature, and only active when the session has an `org.a11y.Bus`.
//!
//! D-Bus comes from `zbus` (already a dependency of the desktop build, vendored for `--offline`). The bus address comes from
//! `org.a11y.Bus.GetAddress` on the session bus; the app registers its root with `org.a11y.atspi.Socket.Embed` on the
//! registry. Interface methods run on zbus's executor threads and only read a shared snapshot; the shell thread publishes
//! snapshots (`Bridge::publish`, a no-op when nothing changed) and drains requests (`Bridge::drain`) that it turns into
//! `ToApp::AxSet`. Nothing here runs per frame.
use super::model::*;
use bifrost_proto::AxAction;
use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex, MutexGuard};
use zbus::blocking::Connection;
use zbus::zvariant::{ObjectPath, OwnedObjectPath, Value};

/// An assistive technology asked to act on `id` in the app at `slot`.
#[derive(Debug, PartialEq)]
pub struct Request {
    pub slot: u8,
    pub id: u64,
    pub action: AxAction,
}

struct Shared {
    apps: Mutex<Vec<AppSnap>>,
    tx: Mutex<Sender<Request>>,
    bus_name: String,
}

type Ref = (String, OwnedObjectPath);

impl Shared {
    fn apps(&self) -> MutexGuard<'_, Vec<AppSnap>> {
        self.apps.lock().unwrap_or_else(|e| e.into_inner())
    }
    fn r(&self, k: Key) -> Ref {
        (self.bus_name.clone(), ObjectPath::try_from(key_path(k)).expect("valid path").into())
    }
    fn null(&self) -> Ref {
        (String::new(), ObjectPath::try_from(NULL_PATH).expect("valid path").into())
    }
    fn send(&self, slot: u8, id: u64, action: AxAction) -> bool {
        self.tx.lock().unwrap_or_else(|e| e.into_inner()).send(Request { slot, id, action }).is_ok()
    }
}

/// What every interface of one object needs: the snapshot and which object it is.
#[derive(Clone)]
struct Obj {
    shared: Arc<Shared>,
    key: Key,
}

struct Info {
    name: String,
    role: u32,
    role_name: &'static str,
    states: [u32; 2],
    parent: Key,
    children: Vec<Key>,
    index: i32,
    bounds: bifrost_proto::UiRect,
    value: String,
    editable: bool,
    actionable: bool,
    numeric: bool,
}

impl Obj {
    fn info(&self) -> Option<Info> {
        let apps = self.shared.apps();
        match self.key {
            Key::Root => Some(Info {
                name: "BifrOSt".into(),
                role: role::APPLICATION,
                role_name: "application",
                states: [0, 0],
                parent: Key::Root,
                children: apps.iter().map(|a| Key::App(a.slot)).collect(),
                index: -1,
                bounds: Default::default(),
                value: String::new(),
                editable: false,
                actionable: false,
                numeric: false,
            }),
            Key::App(slot) => {
                let (i, a) = apps.iter().enumerate().find(|(_, a)| a.slot == slot)?;
                Some(Info {
                    name: a.name.clone(),
                    role: role::FRAME,
                    role_name: "frame",
                    states: window_states(),
                    parent: Key::Root,
                    children: children(&a.nodes, None).into_iter().map(|id| Key::Node(slot, id)).collect(),
                    index: i as i32,
                    bounds: Default::default(),
                    value: String::new(),
                    editable: false,
                    actionable: false,
                    numeric: false,
                })
            }
            Key::Node(slot, id) => {
                let a = apps.iter().find(|a| a.slot == slot)?;
                let n = a.nodes.iter().find(|n| n.id == id)?;
                let parent = n.parent.map_or(Key::App(slot), |p| Key::Node(slot, p));
                let siblings = children(&a.nodes, n.parent);
                Some(Info {
                    name: n.name.clone(),
                    role: atspi_role(n.role),
                    role_name: role_name(n.role),
                    states: states(n),
                    parent,
                    children: children(&a.nodes, Some(id)).into_iter().map(|c| Key::Node(slot, c)).collect(),
                    index: siblings.iter().position(|s| *s == id).unwrap_or(0) as i32,
                    bounds: n.bounds,
                    value: n.value.clone(),
                    editable: n.role == bifrost_proto::Role::TextField,
                    actionable: n.enabled && !matches!(n.role, bifrost_proto::Role::StaticText | bifrost_proto::Role::Window),
                    numeric: n.role == bifrost_proto::Role::Slider,
                })
            }
        }
    }

    fn act(&self, action: AxAction) -> bool {
        match self.key {
            Key::Node(slot, id) => self.shared.send(slot, id, action),
            _ => false,
        }
    }
}

fn path_of(k: Key) -> ObjectPath<'static> {
    ObjectPath::try_from(key_path(k)).expect("valid path")
}

struct Accessible(Obj);

#[zbus::interface(name = "org.a11y.atspi.Accessible")]
impl Accessible {
    #[zbus(property)]
    fn name(&self) -> String {
        self.0.info().map(|i| i.name).unwrap_or_default()
    }
    #[zbus(property)]
    fn description(&self) -> String {
        self.0.info().map(|i| i.role_name.to_string()).unwrap_or_default()
    }
    #[zbus(property)]
    fn parent(&self) -> Ref {
        match self.0.info() {
            Some(i) if self.0.key != Key::Root => self.0.shared.r(i.parent),
            _ => self.0.shared.null(),
        }
    }
    #[zbus(property)]
    fn child_count(&self) -> i32 {
        self.0.info().map_or(0, |i| i.children.len() as i32)
    }
    #[zbus(property)]
    fn locale(&self) -> String {
        "is_IS".into()
    }
    #[zbus(property)]
    fn accessible_id(&self) -> String {
        key_path(self.0.key)
    }
    fn get_child_at_index(&self, index: i32) -> zbus::fdo::Result<Ref> {
        let i = self.0.info().ok_or_else(|| zbus::fdo::Error::UnknownObject("gone".into()))?;
        usize::try_from(index).ok().and_then(|n| i.children.get(n)).map(|k| self.0.shared.r(*k)).ok_or_else(|| zbus::fdo::Error::InvalidArgs("no such child".into()))
    }
    fn get_children(&self) -> Vec<Ref> {
        self.0.info().map(|i| i.children.iter().map(|k| self.0.shared.r(*k)).collect()).unwrap_or_default()
    }
    fn get_index_in_parent(&self) -> i32 {
        self.0.info().map_or(-1, |i| i.index)
    }
    fn get_relation_set(&self) -> Vec<(u32, Vec<Ref>)> {
        Vec::new()
    }
    fn get_role(&self) -> u32 {
        self.0.info().map_or(0, |i| i.role)
    }
    fn get_role_name(&self) -> String {
        self.0.info().map(|i| i.role_name.to_string()).unwrap_or_default()
    }
    fn get_localized_role_name(&self) -> String {
        self.0.info().map(|i| i.role_name.to_string()).unwrap_or_default()
    }
    fn get_state(&self) -> Vec<u32> {
        self.0.info().map_or(vec![0, 0], |i| i.states.to_vec())
    }
    fn get_attributes(&self) -> HashMap<String, String> {
        HashMap::new()
    }
    fn get_application(&self) -> Ref {
        self.0.shared.r(Key::Root)
    }
    fn get_interfaces(&self) -> Vec<String> {
        interfaces(&self.0)
    }
}

fn interfaces(o: &Obj) -> Vec<String> {
    let mut v = vec!["org.a11y.atspi.Accessible".to_string(), "org.a11y.atspi.Component".to_string()];
    if let Some(i) = o.info() {
        if i.actionable {
            v.push("org.a11y.atspi.Action".into());
        }
        if i.numeric {
            v.push("org.a11y.atspi.Value".into());
        }
        if i.editable {
            v.push("org.a11y.atspi.Text".into());
            v.push("org.a11y.atspi.EditableText".into());
        }
    }
    v
}

struct Component(Obj);

#[zbus::interface(name = "org.a11y.atspi.Component")]
impl Component {
    fn contains(&self, x: i32, y: i32, _coord: u32) -> bool {
        self.0.info().is_some_and(|i| {
            let b = i.bounds;
            (x as f32) >= b.x && (y as f32) >= b.y && (x as f32) < b.x + b.w && (y as f32) < b.y + b.h
        })
    }
    fn get_accessible_at_point(&self, x: i32, y: i32, _coord: u32) -> Ref {
        let apps = self.0.shared.apps();
        if let Key::App(slot) | Key::Node(slot, _) = self.0.key {
            if let Some(a) = apps.iter().find(|a| a.slot == slot) {
                let (px, py) = (x as f32, y as f32);
                // the last node drawn that contains the point is the topmost
                if let Some(n) = a.nodes.iter().rev().find(|n| px >= n.bounds.x && py >= n.bounds.y && px < n.bounds.x + n.bounds.w && py < n.bounds.y + n.bounds.h) {
                    return self.0.shared.r(Key::Node(slot, n.id));
                }
            }
        }
        self.0.shared.null()
    }
    /// Window-local points for both coordinate types: the shell does not report window positions to the bridge yet.
    fn get_extents(&self, _coord: u32) -> (i32, i32, i32, i32) {
        self.0.info().map_or((0, 0, 0, 0), |i| (i.bounds.x as i32, i.bounds.y as i32, i.bounds.w.ceil() as i32, i.bounds.h.ceil() as i32))
    }
    fn get_position(&self, _coord: u32) -> (i32, i32) {
        self.0.info().map_or((0, 0), |i| (i.bounds.x as i32, i.bounds.y as i32))
    }
    fn get_size(&self) -> (i32, i32) {
        self.0.info().map_or((0, 0), |i| (i.bounds.w.ceil() as i32, i.bounds.h.ceil() as i32))
    }
    fn get_layer(&self) -> u32 {
        3 // ATSPI_LAYER_WIDGET
    }
    fn get_mdi_z_order(&self) -> i16 {
        0
    }
    fn grab_focus(&self) -> bool {
        self.0.act(AxAction::Focus)
    }
    fn get_alpha(&self) -> f64 {
        1.0
    }
}

struct Action(Obj);

#[zbus::interface(name = "org.a11y.atspi.Action")]
impl Action {
    #[zbus(property)]
    fn n_actions(&self) -> i32 {
        1
    }
    fn do_action(&self, index: i32) -> bool {
        index == 0 && self.0.info().is_some_and(|i| i.actionable) && self.0.act(AxAction::Press)
    }
    fn get_name(&self, index: i32) -> String {
        if index == 0 { "press".into() } else { String::new() }
    }
    fn get_localized_name(&self, index: i32) -> String {
        if index == 0 { "press".into() } else { String::new() }
    }
    fn get_description(&self, _index: i32) -> String {
        String::new()
    }
    fn get_key_binding(&self, _index: i32) -> String {
        String::new()
    }
    fn get_actions(&self) -> Vec<(String, String, String)> {
        vec![("press".into(), String::new(), String::new())]
    }
}

struct ValueI(Obj);

#[zbus::interface(name = "org.a11y.atspi.Value")]
impl ValueI {
    #[zbus(property)]
    fn minimum_value(&self) -> f64 {
        0.0
    }
    #[zbus(property)]
    fn maximum_value(&self) -> f64 {
        self.0.info().and_then(|i| numeric(&i.value)).map_or(1.0, |(_, max)| max)
    }
    #[zbus(property)]
    fn minimum_increment(&self) -> f64 {
        self.0.info().and_then(|i| numeric(&i.value)).map_or(0.01, |(_, max)| max / 100.0)
    }
    #[zbus(property)]
    fn current_value(&self) -> f64 {
        self.0.info().and_then(|i| numeric(&i.value)).map_or(0.0, |(v, _)| v)
    }
    #[zbus(property)]
    fn set_current_value(&mut self, v: f64) {
        if v.is_finite() {
            self.0.act(AxAction::SetValue(format!("{v}")));
        }
    }
}

struct Text(Obj);

#[zbus::interface(name = "org.a11y.atspi.Text")]
impl Text {
    #[zbus(property)]
    fn character_count(&self) -> i32 {
        self.0.info().map_or(0, |i| i.value.chars().count() as i32)
    }
    #[zbus(property)]
    fn caret_offset(&self) -> i32 {
        self.0.info().map_or(0, |i| i.value.chars().count() as i32)
    }
    fn get_text(&self, start: i32, end: i32) -> String {
        let v = self.0.info().map(|i| i.value).unwrap_or_default();
        let n = v.chars().count() as i32;
        let end = if end < 0 || end > n { n } else { end };
        let start = start.clamp(0, end);
        v.chars().skip(start as usize).take((end - start) as usize).collect()
    }
    fn set_caret_offset(&self, _offset: i32) -> bool {
        false
    }
}

struct EditableText(Obj);

#[zbus::interface(name = "org.a11y.atspi.EditableText")]
impl EditableText {
    fn set_text_contents(&self, text: &str) -> bool {
        self.0.act(AxAction::SetValue(text.to_string()))
    }
}

/// The running bridge. Create it with [`Bridge::start`]; `None` means there is no accessibility bus (nothing to do).
pub struct Bridge {
    conn: Connection,
    shared: Arc<Shared>,
    rx: Receiver<Request>,
    registered: HashSet<Key>,
    last: Vec<AppSnap>,
}

impl Bridge {
    /// Connects to the AT-SPI bus and registers the application. Returns `None` when the session has no `org.a11y.Bus`
    /// or any step fails; an error is logged once, never fatal.
    pub fn start() -> Option<Bridge> {
        match Self::try_start() {
            Ok(b) => Some(b),
            Err(e) => {
                eprintln!("BifrOSt atspi: bridge off ({e})");
                None
            }
        }
    }

    fn try_start() -> zbus::Result<Bridge> {
        let session = Connection::session()?;
        let reply = session.call_method(Some("org.a11y.Bus"), "/org/a11y/bus", Some("org.a11y.Bus"), "GetAddress", &())?;
        let address: String = reply.body().deserialize()?;
        let conn = zbus::blocking::connection::Builder::address(address.as_str())?.build()?;
        let bus_name = conn.unique_name().map(|n| n.to_string()).unwrap_or_default();
        let (tx, rx) = channel();
        let shared = Arc::new(Shared { apps: Mutex::new(Vec::new()), tx: Mutex::new(tx), bus_name: bus_name.clone() });
        let mut bridge = Bridge { conn, shared, rx, registered: HashSet::new(), last: Vec::new() };
        bridge.register(Key::Root)?;
        let me: (&str, ObjectPath) = (bus_name.as_str(), path_of(Key::Root));
        bridge.conn.call_method(Some("org.a11y.atspi.Registry"), "/org/a11y/atspi/accessible/root", Some("org.a11y.atspi.Socket"), "Embed", &(me,))?;
        Ok(bridge)
    }

    fn obj(&self, key: Key) -> Obj {
        Obj { shared: self.shared.clone(), key }
    }

    fn register(&mut self, key: Key) -> zbus::Result<()> {
        let path = path_of(key);
        let server = self.conn.object_server();
        server.at(&path, Accessible(self.obj(key)))?;
        server.at(&path, Component(self.obj(key)))?;
        if let Some(i) = self.obj(key).info() {
            if i.actionable {
                server.at(&path, Action(self.obj(key)))?;
            }
            if i.numeric {
                server.at(&path, ValueI(self.obj(key)))?;
            }
            if i.editable {
                server.at(&path, Text(self.obj(key)))?;
                server.at(&path, EditableText(self.obj(key)))?;
            }
        }
        self.registered.insert(key);
        Ok(())
    }

    fn unregister(&mut self, key: Key) {
        let path = path_of(key);
        let server = self.conn.object_server();
        let _ = server.remove::<Accessible, _>(&path);
        let _ = server.remove::<Component, _>(&path);
        let _ = server.remove::<Action, _>(&path);
        let _ = server.remove::<ValueI, _>(&path);
        let _ = server.remove::<Text, _>(&path);
        let _ = server.remove::<EditableText, _>(&path);
        self.registered.remove(&key);
    }

    /// Replaces the exported tree with `apps`. Cheap when nothing changed. Registers new objects, drops gone ones and emits
    /// `StateChanged` for focus, checked and enabled flips plus `ChildrenChanged` on the root when apps come and go.
    pub fn publish(&mut self, apps: Vec<AppSnap>) {
        if apps == self.last {
            return;
        }
        *self.shared.apps() = apps.clone();
        let mut want: HashSet<Key> = HashSet::new();
        for a in &apps {
            want.insert(Key::App(a.slot));
            want.extend(a.nodes.iter().map(|n| Key::Node(a.slot, n.id)));
        }
        let gone: Vec<Key> = self.registered.iter().copied().filter(|k| *k != Key::Root && !want.contains(k)).collect();
        for k in gone {
            self.unregister(k);
        }
        for k in &want {
            // an object whose capabilities changed (a node that became actionable) is re-registered
            let old_caps = self.last_caps(*k);
            if !self.registered.contains(k) || old_caps != self.caps(*k) {
                if self.registered.contains(k) {
                    self.unregister(*k);
                }
                if let Err(e) = self.register(*k) {
                    eprintln!("BifrOSt atspi: register {}: {e}", key_path(*k));
                }
            }
        }
        self.emit_changes(&apps);
        self.last = apps;
    }

    fn caps(&self, k: Key) -> (bool, bool, bool) {
        self.obj(k).info().map_or((false, false, false), |i| (i.actionable, i.numeric, i.editable))
    }

    fn last_caps(&self, k: Key) -> (bool, bool, bool) {
        let Key::Node(slot, id) = k else { return self.caps(k) };
        use bifrost_proto::Role;
        self.last
            .iter()
            .find(|a| a.slot == slot)
            .and_then(|a| a.nodes.iter().find(|n| n.id == id))
            .map_or((false, false, false), |n| (n.enabled && !matches!(n.role, Role::StaticText | Role::Window), n.role == Role::Slider, n.role == Role::TextField))
    }

    fn emit_changes(&self, apps: &[AppSnap]) {
        let state = |path: &str, name: &str, on: bool| {
            let body = (name, on as i32, 0i32, Value::from(0i32), HashMap::<String, Value>::new());
            let _ = self.conn.emit_signal(None::<&str>, path, "org.a11y.atspi.Event.Object", "StateChanged", &body);
        };
        for a in apps {
            let old = self.last.iter().find(|o| o.slot == a.slot);
            for n in &a.nodes {
                let p = old.and_then(|o| o.nodes.iter().find(|m| m.id == n.id));
                let path = node_path(a.slot, n.id);
                if p.is_none_or(|p| p.focused != n.focused) && (n.focused || p.is_some()) {
                    state(&path, "focused", n.focused);
                    if n.focused {
                        let body = ("", 0i32, 0i32, Value::from(0i32), HashMap::<String, Value>::new());
                        let _ = self.conn.emit_signal(None::<&str>, path.as_str(), "org.a11y.atspi.Event.Focus", "Focus", &body);
                    }
                }
                if let Some(p) = p {
                    if p.enabled != n.enabled {
                        state(&path, "enabled", n.enabled);
                    }
                    if p.checked != n.checked {
                        state(&path, "checked", n.checked == Some(true));
                    }
                    if p.value != n.value && n.role == bifrost_proto::Role::Slider {
                        let body = ("accessible-value", 0i32, 0i32, Value::from(0i32), HashMap::<String, Value>::new());
                        let _ = self.conn.emit_signal(None::<&str>, path.as_str(), "org.a11y.atspi.Event.Object", "PropertyChange", &body);
                    }
                }
            }
        }
        if apps.len() != self.last.len() {
            let body = ("add", apps.len() as i32, 0i32, Value::from(0i32), HashMap::<String, Value>::new());
            let _ = self.conn.emit_signal(None::<&str>, ROOT_PATH, "org.a11y.atspi.Event.Object", "ChildrenChanged", &body);
        }
    }

    /// Requests screen readers made since the last call (press, value write, focus).
    pub fn drain(&self) -> Vec<Request> {
        self.rx.try_iter().collect()
    }
}
