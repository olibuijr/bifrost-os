//! Login items (`SMAppService`) and menu-bar extras (`NativeStatusItem`) on the shell side.
//!
//! `Registry` is plain data: the login-items list (app ids, kept in the `login.items` pref and launched once per
//! session), the requests waiting for the user's approval in Settings > General > Login Items (in memory), the status
//! changes to report to apps, and the status items the running apps published. `Shell` feeds it from the app sockets (`apphost.rs`); the
//! menu bar draws `status_entries()` in its extras area and sends a chosen entry back as `ToApp::StatusItemAction`.
//!
//! Identity is always the app id or slot of the connection, never anything in the message.

use bifrost_proto::extras::MAX_STATUS_ITEMS_PER_APP;
use bifrost_proto::{LoginOp, LoginStatus, StatusMenuItem, ToApp, ToShell};

/// The pref that holds the login items, comma separated app ids.
pub const PREF_KEY: &str = "login.items";

/// A menu-bar extra an app published.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StatusEntry {
    pub slot: u8,
    pub item: u32,
    pub title: String,
    pub symbol: String,
    pub menu: Vec<StatusMenuItem>,
}

#[derive(Default)]
pub struct Registry {
    login: Vec<String>,
    /// registered by an app and not yet allowed by the user: `RequiresApproval` until `approve` (in memory; the app asks again at its next start)
    pending: Vec<String>,
    /// status changes the `login-items` service reports to apps, drained by `take_changes`
    changes: Vec<(String, LoginStatus)>,
    status: Vec<StatusEntry>,
    /// the login items were launched for this session
    pub launched: bool,
}

impl Registry {
    pub fn from_pref(value: Option<&str>) -> Self {
        let mut r = Registry::default();
        for id in value.unwrap_or("").split(',').map(str::trim).filter(|s| !s.is_empty()) {
            r.register(id);
        }
        r
    }

    pub fn to_pref(&self) -> String {
        self.login.join(",")
    }

    pub fn login_ids(&self) -> &[String] {
        &self.login
    }

    /// Adds an approved login item (a list entry the user allowed). True when the list changed.
    pub fn register(&mut self, app: &str) -> bool {
        if self.login.iter().any(|a| a == app) {
            return false;
        }
        self.login.push(app.to_string());
        true
    }

    /// Moves an approved login item one place earlier in the start order. True when the order changed.
    pub fn move_earlier(&mut self, app: &str) -> bool {
        super::services::login_items_approval::move_earlier(&mut self.login, app)
    }

    /// An app asks to open at login: it waits for the user's approval. True when the request is new.
    pub fn request(&mut self, app: &str) -> bool {
        if self.login.iter().any(|a| a == app) || self.pending.iter().any(|a| a == app) {
            return false;
        }
        self.pending.push(app.to_string());
        self.changes.push((app.to_string(), LoginStatus::RequiresApproval));
        true
    }

    /// The user allows a pending request: it becomes a login item and the app is told `Enabled`. True when it was pending.
    pub fn approve(&mut self, app: &str) -> bool {
        let Some(i) = self.pending.iter().position(|a| a == app) else { return false };
        self.pending.remove(i);
        self.register(app);
        self.changes.push((app.to_string(), LoginStatus::Enabled));
        true
    }

    /// The requests waiting for approval, in the order they asked.
    pub fn pending_ids(&self) -> &[String] {
        &self.pending
    }

    /// Removes an app from the login items and from the pending requests. True when either changed.
    pub fn unregister(&mut self, app: &str) -> bool {
        let before = self.login.len() + self.pending.len();
        self.login.retain(|a| a != app);
        self.pending.retain(|a| a != app);
        let changed = self.login.len() + self.pending.len() != before;
        if changed {
            self.changes.push((app.to_string(), LoginStatus::NotRegistered));
        }
        changed
    }

    pub fn login_status(&self, app: &str) -> LoginStatus {
        if self.login.iter().any(|a| a == app) {
            LoginStatus::Enabled
        } else if self.pending.iter().any(|a| a == app) {
            LoginStatus::RequiresApproval
        } else {
            LoginStatus::NotRegistered
        }
    }

    /// Applies a `LoginItem` or `login-items` call of `app`; returns the status to answer with and whether the lists changed.
    pub fn login_op(&mut self, app: &str, op: LoginOp) -> (LoginStatus, bool) {
        let changed = match op {
            LoginOp::Register => self.request(app),
            LoginOp::Unregister => self.unregister(app),
            LoginOp::Query => false,
        };
        (self.login_status(app), changed)
    }

    /// The status changes since the last call, in order, for the `login-items` service to report.
    pub fn take_changes(&mut self) -> Vec<(String, LoginStatus)> {
        std::mem::take(&mut self.changes)
    }

    /// `StatusItemSet`: creates or updates item `item` of `slot`. An app may hold `MAX_STATUS_ITEMS_PER_APP`; more are ignored.
    pub fn set_status(&mut self, slot: u8, item: u32, visible: bool, title: String, symbol: String, menu: Vec<StatusMenuItem>) {
        if !visible {
            self.remove_status(slot, item);
            return;
        }
        let entry = StatusEntry { slot, item, title, symbol, menu };
        if let Some(e) = self.status.iter_mut().find(|e| e.slot == slot && e.item == item) {
            *e = entry;
        } else if self.status.iter().filter(|e| e.slot == slot).count() < MAX_STATUS_ITEMS_PER_APP {
            self.status.push(entry);
        }
    }

    pub fn remove_status(&mut self, slot: u8, item: u32) {
        self.status.retain(|e| !(e.slot == slot && e.item == item));
    }

    /// The app in `slot` exited: its extras go.
    pub fn app_gone(&mut self, slot: u8) {
        self.status.retain(|e| e.slot != slot);
    }

    /// The extras to draw, in the order of the bar, for the slots `running` accepts.
    pub fn status_entries(&self, running: impl Fn(u8) -> bool) -> Vec<&StatusEntry> {
        self.status.iter().filter(|e| running(e.slot)).collect()
    }
}

impl super::Shell {
    fn extras_app_id(slot: u8) -> Option<String> {
        super::apphost::installed().get(slot as usize).map(|a| a.manifest.id.clone())
    }

    pub(super) fn login_save(&mut self) {
        let _ = self.prefs.set(PREF_KEY, &self.extras.to_pref());
    }

    /// A `LoginItem`, `StatusItemSet` or `StatusItemRemove` message from the app in `slot`.
    pub(super) fn extras_msg(&mut self, slot: u8, msg: ToShell) {
        match msg {
            ToShell::LoginItem { request, op } => {
                let Some(app) = Self::extras_app_id(slot) else { return };
                let (status, changed) = self.extras.login_op(&app, op);
                if changed {
                    self.login_save();
                    self.dirty = true;
                }
                self.ext_send(slot as usize, &ToApp::LoginItemStatus { request, status });
            }
            ToShell::StatusItemSet { item, visible, title, symbol, menu } => {
                self.extras.set_status(slot, item, visible, title, symbol, menu);
                self.dirty = true;
            }
            ToShell::StatusItemRemove { item } => {
                self.extras.remove_status(slot, item);
                self.dirty = true;
            }
            _ => {}
        }
    }

    /// An app just started: it learns whether it is a login item.
    pub(super) fn extras_started(&mut self, slot: usize) {
        if let Some(app) = Self::extras_app_id(slot as u8) {
            let status = self.extras.login_status(&app);
            self.ext_send(slot, &ToApp::LoginItemStatus { request: 0, status });
        }
    }

    pub(super) fn extras_app_gone(&mut self, slot: u8) {
        self.extras.app_gone(slot);
        self.dirty = true;
    }

    /// Once per session, after the desktop is up and unlocked: starts the login items.
    pub(super) fn login_items_launch(&mut self) {
        if self.extras.launched || self.is_locked() || self.desk.is_none() {
            return;
        }
        self.extras.launched = true;
        let ids: Vec<String> = self.extras.login_ids().to_vec();
        let registered = super::apphost::installed();
        for id in ids {
            if let Some(slot) = registered.iter().position(|a| a.manifest.id == id) {
                self.ext_open(slot);
            }
        }
    }

    /// The extras the bar shows now (apps that are running), the order `Menu::Status(i)` indexes.
    pub(super) fn status_entries_now(&self) -> Vec<&StatusEntry> {
        self.extras.status_entries(|slot| self.ext.get(slot as usize).is_some_and(|h| h.is_some()))
    }

    /// Login Items page rows: (app id, display name) of every login item, installed or not.
    pub(super) fn login_item_views(&self) -> Vec<(String, String)> {
        Self::login_id_views(self.extras.login_ids())
    }

    /// Login Items page rows waiting for approval: (app id, display name).
    pub(super) fn login_pending_views(&self) -> Vec<(String, String)> {
        Self::login_id_views(self.extras.pending_ids())
    }

    fn login_id_views(ids: &[String]) -> Vec<(String, String)> {
        let registered = super::apphost::installed();
        ids.iter()
            .map(|id| (id.clone(), registered.iter().find(|a| &a.manifest.id == id).map(|a| a.manifest.name.clone()).unwrap_or_else(|| id.clone())))
            .collect()
    }

    /// The user allowed pending login items in Settings: they start at the next login, and a running app is told `Enabled`.
    pub(super) fn login_items_approve(&mut self, ids: Vec<String>) {
        let mut changed = false;
        for id in &ids {
            if self.extras.approve(id) {
                changed = true;
                if let Some(slot) = super::apphost::installed().iter().position(|a| &a.manifest.id == id) {
                    if self.ext.get(slot).is_some_and(|h| h.is_some()) {
                        self.ext_send(slot, &ToApp::LoginItemStatus { request: 0, status: LoginStatus::Enabled });
                    }
                }
            }
        }
        if changed {
            self.login_save();
            self.dirty = true;
        }
    }

    /// The user moved login items earlier in Settings > Login Items > Start Order.
    pub(super) fn login_items_move(&mut self, ids: Vec<String>) {
        let mut changed = false;
        for id in &ids {
            changed |= self.extras.move_earlier(id);
        }
        if changed {
            self.login_save();
            self.dirty = true;
        }
    }

    /// The user removed login items in Settings.
    pub(super) fn login_items_remove(&mut self, ids: Vec<String>) {
        let mut changed = false;
        for id in &ids {
            changed |= self.extras.unregister(id);
            if let Some(slot) = super::apphost::installed().iter().position(|a| &a.manifest.id == id) {
                if self.ext.get(slot).is_some_and(|h| h.is_some()) {
                    self.ext_send(slot, &ToApp::LoginItemStatus { request: 0, status: LoginStatus::NotRegistered });
                }
            }
        }
        if changed {
            self.login_save();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(id: u32, title: &str) -> StatusMenuItem {
        StatusMenuItem { id, title: title.into(), enabled: true, ..Default::default() }
    }

    #[test]
    fn login_items_register_unregister_and_persist() {
        let mut r = Registry::from_pref(Some("a, b,,a"));
        assert_eq!(r.login_ids(), ["a", "b"]);
        assert_eq!(r.login_op("c", LoginOp::Query), (LoginStatus::NotRegistered, false));
        assert_eq!(r.login_op("c", LoginOp::Register), (LoginStatus::RequiresApproval, true));
        assert_eq!(r.login_op("c", LoginOp::Register), (LoginStatus::RequiresApproval, false));
        assert_eq!(r.to_pref(), "a,b", "a pending request is not saved until the user allows it");
        assert!(r.approve("c"));
        assert_eq!(r.login_status("c"), LoginStatus::Enabled);
        assert_eq!(r.to_pref(), "a,b,c");
        assert_eq!(r.login_op("a", LoginOp::Unregister), (LoginStatus::NotRegistered, true));
        assert_eq!(Registry::from_pref(Some(&r.to_pref())).login_ids(), ["b", "c"]);
        assert_eq!(Registry::from_pref(None).login_ids().len(), 0);
    }

    #[test]
    fn login_items_move_earlier_and_keep_the_order_when_saved() {
        let mut r = Registry::from_pref(Some("a,b,c"));
        assert!(r.move_earlier("c"));
        assert_eq!(r.login_ids(), ["a", "c", "b"]);
        assert!(!r.move_earlier("a"), "the first item cannot move");
        assert_eq!(r.to_pref(), "a,c,b");
        assert_eq!(Registry::from_pref(Some(&r.to_pref())).login_ids(), ["a", "c", "b"]);
    }

    #[test]
    fn a_registration_requires_approval_until_the_user_allows_it() {
        let mut r = Registry::from_pref(Some("b"));
        assert_eq!(r.login_op("x", LoginOp::Query), (LoginStatus::NotRegistered, false));
        assert_eq!(r.login_op("x", LoginOp::Register), (LoginStatus::RequiresApproval, true));
        assert_eq!(r.pending_ids(), ["x"]);
        assert_eq!(r.take_changes(), vec![("x".to_string(), LoginStatus::RequiresApproval)]);
        assert!(r.take_changes().is_empty(), "changes are drained once");
        assert!(r.approve("x"));
        assert!(!r.approve("x"), "only a pending request can be approved");
        assert!(r.pending_ids().is_empty());
        assert_eq!(r.take_changes(), vec![("x".to_string(), LoginStatus::Enabled)]);
        assert_eq!(r.login_ids(), ["b", "x"]);
        assert_eq!(r.login_op("b", LoginOp::Register), (LoginStatus::Enabled, false), "an approved item is not asked again");
    }

    #[test]
    fn withdrawing_a_pending_request_reports_not_registered() {
        let mut r = Registry::default();
        r.request("p");
        assert_eq!(r.login_op("p", LoginOp::Unregister), (LoginStatus::NotRegistered, true));
        assert!(r.pending_ids().is_empty());
        assert!(!r.approve("p"));
        assert_eq!(r.take_changes(), vec![("p".to_string(), LoginStatus::RequiresApproval), ("p".to_string(), LoginStatus::NotRegistered)]);
    }

    #[test]
    fn status_items_are_per_slot_limited_and_go_with_the_app() {
        let mut r = Registry::default();
        r.set_status(1, 1, true, "42%".into(), String::new(), vec![item(1, "Open")]);
        r.set_status(1, 1, true, "43%".into(), String::new(), vec![]);
        r.set_status(2, 1, true, "x".into(), String::new(), vec![]);
        assert_eq!(r.status_entries(|_| true).len(), 2);
        assert_eq!(r.status_entries(|_| true)[0].title, "43%");
        for i in 2..=10 {
            r.set_status(1, i, true, "y".into(), String::new(), vec![]);
        }
        assert_eq!(r.status_entries(|s| s == 1).len(), MAX_STATUS_ITEMS_PER_APP);
        assert_eq!(r.status_entries(|s| s == 2).len(), 1);
        r.set_status(2, 1, false, String::new(), String::new(), vec![]);
        assert!(r.status_entries(|s| s == 2).is_empty());
        r.app_gone(1);
        assert!(r.status_entries(|_| true).is_empty());
    }

    #[test]
    fn status_entries_hide_apps_that_are_not_running() {
        let mut r = Registry::default();
        r.set_status(3, 1, true, "a".into(), String::new(), vec![]);
        assert!(r.status_entries(|_| false).is_empty());
        assert_eq!(r.status_entries(|s| s == 3).len(), 1);
    }
}
