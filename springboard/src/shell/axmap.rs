//! The OS half of the element map (`bifrost_ui::ax`): producers that walk what the shell already knows and name it.
//! Settings is generated from its page specs, so a new row or page is addressable the moment it exists.
use super::list::{Accessory, Item, Spec};
use super::{AppId, Effect, Shell};
use super::pages::{Action, PageId};
use super::accessibility::actions::{window_index, WINDOW_ACTIONS};
use super::accessibility::tree::{self, add_app_tree, AxState};
use bifrost_ui::ax::{Element, Map};
use std::collections::HashSet;

/// The one element a query names, cloned; a miss or an ambiguous name is an error the agent can read.
fn resolve_one(map: &Map, query: &str) -> Result<Element, String> {
    use bifrost_ui::ax::Resolve;
    match map.resolve(query) {
        Resolve::One(e) => Ok(e.clone()),
        Resolve::None => Err(format!("no element {query:?}")),
        Resolve::Ambiguous(c) => Err(format!("{query:?} is ambiguous: {}", c.join(" "))),
    }
}

/// `os/settings`: every page reachable from the root by `Push`, each row named by its label. A page's address is the row that
/// opens it (`os/settings/general`), its rows sit below (`os/settings/general/software-update`). The handle is the label
/// path (`General>Software Update`), which is how the shell navigates there. Pages that need a selection (a Wi-Fi network's
/// detail, one app's notifications) are not listed: their rows depend on live data.
pub fn settings(map: &mut Map, spec: &dyn Fn(PageId) -> Spec) {
    let mut seen = HashSet::new();
    seen.insert(PageId::Root);
    walk(map, spec, PageId::Root, "os/settings", &[], &mut seen);
}

fn walk(map: &mut Map, spec: &dyn Fn(PageId) -> Spec, page: PageId, parent: &str, trail: &[String], seen: &mut HashSet<PageId>) {
    let s = spec(page);
    for group in &s.groups {
        for item in &group.items {
            let (role, value) = item_role(item);
            let mut path = trail.to_vec();
            path.push(item.label.clone());
            let address = map.add(parent, role, &item.label, &value, item.tappable() || role == "text", None, &path.join(">"));
            if let Action::Push(id) = item.action {
                let detail = matches!(id, PageId::WifiDetail | PageId::BluetoothDetail | PageId::NotificationApp);
                if !detail && seen.insert(id) {
                    walk(map, spec, id, &address, &path, seen);
                }
            }
        }
    }
}

/// The role and value a Settings row shows to assistive tech. Shared with the screen reader's cursor (`accessibility::reader`).
pub(super) fn item_role(item: &Item) -> (&'static str, String) {
    match (&item.action, &item.accessory) {
        (Action::Push(_), _) => ("page", item.value.clone()),
        (Action::Toggle(_), Accessory::Switch(on)) | (_, Accessory::Switch(on)) => ("toggle", if *on { "on" } else { "off" }.to_string()),
        (Action::Slider(_), _) => ("slider", item.value.clone()),
        (Action::Pick(_), Accessory::Check(on)) => ("choice", if *on { "selected" } else { "" }.to_string()),
        (Action::Pick(_), _) => ("choice", item.value.clone()),
        (Action::None, _) => ("text", item.value.clone()),
        _ => ("row", item.value.clone()),
    }
}

impl Shell {
    /// The map of the OS as it is now: Settings (always, from its page specs) and one `os/app/<name>` entry per open window.
    pub fn ax_map(&mut self, t: f64) -> Map {
        self.ax_state(t).0
    }

    /// The map and the state beside it (keyboard focus, checked state) that `ax list` prints.
    pub fn ax_state(&mut self, t: f64) -> (Map, AxState) {
        let mut map = Map::new();
        let mut st = AxState::default();
        if self.settings.is_none() {
            self.init_app(AppId::Settings, t);
        }
        if let Some(st) = &self.settings {
            settings(&mut map, &|id| st.ax_spec(id));
            // a modal alert is the first thing to answer: its buttons are addressable while it is open
            if let Some(a) = st.ax_alert() {
                let alert = map.add("os/settings", "alert", "Alert", &a.title, true, None, "");
                map.add(&alert, "text", "Message", &a.message, true, None, "");
                map.add(&alert, "button", "Cancel", "", true, None, "alert:cancel");
                map.add(&alert, "button", &a.confirm, "", true, None, "alert:confirm");
            }
        }
        for (m, name, rows) in self.ax_menus() {
            let menu = map.add("os/menubar", "menu", name, "", true, None, "");
            for (i, text, enabled) in rows {
                map.add(&menu, "item", &text, "", enabled, None, &format!("menu:{m}:{i}"));
            }
        }
        // every app that can be launched, then the windows that are open (an app with a window shows `open` as its value)
        let open: Vec<AppId> = self.desk.as_ref().map(|d| d.wins.iter().map(|w| w.app).collect()).unwrap_or_default();
        for a in super::apps::apps() {
            let state = if open.contains(&a.id) { "open" } else { "" };
            map.add("os/apps", "app", a.name, state, true, None, &format!("open:{}", a.icon));
        }
        // every running app process: the elements it declared, else the ones the kit's draw helpers recorded
        for host in self.ext.iter().flatten() {
            add_app_tree(&mut map, &format!("app/{}", host.id), &host.controls, &host.controls_tree, &mut st);
        }
        self.add_window_elements(&mut map);
        (map, st)
    }

    /// The shell chrome for the AT-SPI bridge: the menu bar and its menus, the dock's apps (open ones marked) and the search field.
    #[cfg(feature = "atspi")]
    pub(super) fn ax_chrome_snap(&self) -> super::atspi::model::AppSnap {
        let menus: Vec<(String, Vec<(String, bool)>)> =
            self.ax_menus().into_iter().map(|(_, name, rows)| (name.to_string(), rows.into_iter().map(|(_, text, enabled)| (text, enabled)).collect())).collect();
        let open: Vec<AppId> = self.desk.as_ref().map(|d| d.wins.iter().map(|w| w.app).collect()).unwrap_or_default();
        let dock: Vec<(String, bool)> = super::apps::apps().iter().map(|a| (a.name.to_string(), open.contains(&a.id))).collect();
        super::atspi::model::chrome_snap(&menus, &dock)
    }

    /// The open windows as elements under `os/windows`, each carrying the title-bar actions in `WINDOW_ACTIONS`.
    fn add_window_elements(&self, map: &mut Map) {
        let Some(desk) = self.desk.as_ref() else { return };
        for (i, w) in desk.wins.iter().enumerate() {
            let name = super::apps::apps().iter().find(|a| a.id == w.app).map(|a| a.name.to_string()).unwrap_or_else(|| "Window".to_string());
            let state = if w.minimized { "minimized" } else { "open" };
            let addr = map.add("os/windows", "window", &name, state, true, None, &format!("win:{i}"));
            for action in WINDOW_ACTIONS {
                map.add_custom_action(&addr, action);
            }
        }
    }

    /// `ax list [prefix]`: the listing with parent, checked and focused columns.
    pub fn ax_listing(&mut self, prefix: &str, t: f64) -> String {
        let (map, st) = self.ax_state(t);
        tree::listing(&map, &st, prefix)
    }

    /// `ax actions <element>`: the custom action names of one element, one per line.
    pub fn ax_actions(&mut self, query: &str, t: f64) -> Result<String, String> {
        let map = self.ax_map(t);
        let el = resolve_one(&map, query)?;
        Ok(map.custom_actions(&el.address).map(|n| format!("{n}\n")).collect())
    }

    /// `ax action <element> :: <name>`: runs one custom action of an element. Answers the element's address.
    pub fn ax_action(&mut self, query: &str, name: &str, t: f64) -> Result<(String, Vec<Effect>), String> {
        let map = self.ax_map(t);
        let el = resolve_one(&map, query)?;
        if !el.enabled {
            return Err(format!("{} is disabled", el.address));
        }
        if !map.has_custom_action(&el.address, name) {
            let names: Vec<&str> = map.custom_actions(&el.address).collect();
            return Err(format!("{} has no action {name:?}; its actions: {}", el.address, names.join(", ")));
        }
        let i = window_index(&el.handle).ok_or("this element has no action the shell can run")?;
        let fx = self.window_action(i, name, t)?;
        self.dirty = true;
        Ok((el.address, fx))
    }

    /// Resolves `query` in the map and acts on it. Settings rows open Settings first; the answer names what was pressed.
    pub fn ax_press(&mut self, query: &str, t: f64) -> Result<(String, Vec<Effect>), String> {
        use bifrost_ui::ax::Resolve;
        let map = self.ax_map(t);
        let el = match map.resolve(query) {
            Resolve::One(e) => e.clone(),
            Resolve::None => return Err(format!("no element {query:?}")),
            Resolve::Ambiguous(c) => return Err(format!("{query:?} is ambiguous: {}", c.join(" "))),
        };
        if !el.enabled {
            return Err(format!("{} is disabled", el.address));
        }
        if el.handle.starts_with("win:") {
            return Err(format!("{} is a window: list its actions with `ax actions` and run one with `ax action`", el.address));
        }
        if el.address.starts_with("os/settings") {
            let ui = &self.ui;
            if let Some(answer) = el.handle.strip_prefix("alert:") {
                let st = self.settings.as_mut().ok_or("Settings is not open")?;
                let fx = st.ax_answer(ui, &mut self.prefs, answer == "confirm", t)?;
                self.dirty = true;
                return Ok((el.address, fx));
            }
            if self.settings.as_ref().and_then(|st| st.ax_alert()).is_some() {
                return Err("an alert is open: answer it first (see `ax os/settings/alert`)".into());
            }
            if self.desk_active() {
                self.desk_open(AppId::Settings, t);
            } else {
                self.open(AppId::Settings, t);
            }
            let labels: Vec<String> = el.handle.split('>').map(str::to_string).collect();
            let ui = &self.ui;
            let fx = match self.settings.as_mut() {
                Some(st) => st.ax_press(ui, &mut self.prefs, &labels, t)?,
                None => return Err("Settings did not open".into()),
            };
            self.dirty = true;
            return Ok((el.address, fx));
        }
        if let Some(spec) = el.handle.strip_prefix("menu:") {
            let (m, r) = spec.split_once(':').and_then(|(m, r)| Some((m.parse().ok()?, r.parse().ok()?))).ok_or("bad menu handle")?;
            let fx = self.ax_menu_press(m, r, t)?;
            self.dirty = true;
            return Ok((el.address, fx));
        }
        if let Some(icon) = el.handle.strip_prefix("open:") {
            if !self.desktop_open_app(icon, t) {
                return Err(format!("{} could not be opened", el.address));
            }
            self.dirty = true;
            return Ok((el.address, Vec::new()));
        }
        if let Some(id) = el.address.strip_prefix("app/").and_then(|a| a.split('/').next()) {
            let scale = self.ui.pt(1.0);
            let node = el.handle.parse::<u64>().ok();
            let host = self.ext.iter_mut().flatten().find(|h| h.id == id).ok_or("that app is not running")?;
            let n = host.controls.iter().find(|n| Some(n.id) == node).ok_or("that element is gone")?;
            let (x, y) = ((n.bounds.x + n.bounds.w / 2.0) * scale, (n.bounds.y + n.bounds.h / 2.0) * scale);
            // a tap in the app's own coordinates: down, then up 40 ms later
            use bifrost_proto::{ToApp, TouchKind};
            let t_ms = t as u32;
            host.send(&ToApp::Touch { kind: TouchKind::Down, id: 0, x, y, t_ms });
            host.send(&ToApp::Touch { kind: TouchKind::Up, id: 0, x, y, t_ms: t_ms.wrapping_add(40) });
            return Ok((el.address, Vec::new()));
        }
        Err(format!("{} cannot be pressed yet", el.address))
    }
}
