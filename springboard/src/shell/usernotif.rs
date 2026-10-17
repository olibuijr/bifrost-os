//! `UNUserNotificationCenter` on the shell side: the categories each app registered, requests waiting for their
//! trigger, the delivered requests (mapped to the entries of the notification store) and the answers that wait for an
//! app that is not running. The registry is plain data; `Shell` feeds it from the app sockets (`apphost.rs`) and
//! shows what it hands back through the normal notification UI (`notify/`).
//!
//! Identity is always the app id of the connection, never anything in the message.

use bifrost_proto::{NotifAction, NotifCategory, NotifRequest, NotifSettings, NotifStatus, ToApp, NOTIF_DEFAULT_ACTION, NOTIF_DISMISS_ACTION};
use std::collections::HashMap;

/// Pending requests one app may hold (`UNUserNotificationCenter` allows 64 local notifications).
pub const MAX_PENDING: usize = 64;
/// Answers kept for an app that is not running.
const MAX_QUEUED: usize = 32;

#[derive(Clone, Debug, PartialEq)]
struct Pending {
    app: String,
    req: NotifRequest,
    /// shell clock (ms) when the trigger fires
    due: f64,
}

/// A delivered request and the notification store entry that shows it.
#[derive(Clone, Debug, PartialEq)]
pub struct Delivered {
    pub app: String,
    pub req: NotifRequest,
    pub store_id: u64,
}

/// What `add` decided.
#[derive(Debug, PartialEq)]
pub enum Added {
    /// show it now; `replaced` are store entries of an earlier delivery with the same identifier
    Deliver { replaced: Vec<u64> },
    /// it waits for its trigger
    Scheduled,
    Refused(NotifStatus),
}

#[derive(Default)]
pub struct Registry {
    categories: HashMap<String, Vec<NotifCategory>>,
    pending: Vec<Pending>,
    delivered: Vec<Delivered>,
    queued: Vec<(String, ToApp)>,
    /// requests that wait for the first-time alert of their app: app, slot, request id, request
    held: Vec<(String, u8, u32, NotifRequest)>,
}

impl Registry {
    /// `setNotificationCategories`: replaces the app's set.
    pub fn set_categories(&mut self, app: &str, list: Vec<NotifCategory>) {
        self.categories.insert(app.to_string(), list);
    }

    pub fn category(&self, app: &str, id: &str) -> Option<&NotifCategory> {
        self.categories.get(app)?.iter().find(|c| c.id == id)
    }

    /// Why a request is refused (empty identifier or content, unknown category), or None when it is valid.
    pub fn check(&self, app: &str, req: &NotifRequest) -> Option<NotifStatus> {
        if req.id.is_empty() || (req.title.is_empty() && req.body.is_empty() && req.subtitle.is_empty()) {
            return Some(NotifStatus::Param);
        }
        if !req.category.is_empty() && self.category(app, &req.category).is_none() {
            return Some(NotifStatus::Param);
        }
        None
    }

    /// `UNUserNotificationCenter.add`: validates the request and either delivers it now or schedules it. An earlier
    /// request with the same identifier, pending or delivered, is replaced.
    pub fn add(&mut self, app: &str, req: NotifRequest, now: f64) -> Added {
        if let Some(status) = self.check(app, &req) {
            return Added::Refused(status);
        }
        self.pending.retain(|p| !(p.app == app && p.req.id == req.id));
        let replaced_pending = self.pending.iter().filter(|p| p.app == app).count();
        if req.delay_ms > 0 && replaced_pending >= MAX_PENDING {
            return Added::Refused(NotifStatus::Full);
        }
        if req.delay_ms > 0 {
            let due = now + f64::from(req.delay_ms);
            self.pending.push(Pending { app: app.to_string(), req, due });
            return Added::Scheduled;
        }
        let replaced = self.take_delivered(app, &req.id);
        Added::Deliver { replaced }
    }

    /// The store entries of the app's delivered requests with `id`, which a new delivery with that id replaces.
    pub fn take_delivered(&mut self, app: &str, id: &str) -> Vec<u64> {
        let (gone, keep): (Vec<_>, Vec<_>) = std::mem::take(&mut self.delivered).into_iter().partition(|d| d.app == app && d.req.id == id);
        self.delivered = keep;
        gone.into_iter().map(|d| d.store_id).collect()
    }

    /// Requests whose trigger has fired by `now`, oldest first; the caller delivers them (`record_delivery`).
    pub fn due(&mut self, now: f64) -> Vec<(String, NotifRequest)> {
        let mut fired: Vec<Pending> = Vec::new();
        let mut kept: Vec<Pending> = Vec::with_capacity(self.pending.len());
        for mut p in std::mem::take(&mut self.pending) {
            if p.due > now {
                kept.push(p);
                continue;
            }
            fired.push(p.clone());
            if p.req.repeats && p.req.delay_ms > 0 {
                // a repeating trigger stays pending on its own grid: the first date after `now`
                let step = f64::from(p.req.delay_ms);
                p.due += ((now - p.due) / step).floor() * step + step;
                kept.push(p);
            }
        }
        self.pending = kept;
        fired.sort_by(|a, b| a.due.total_cmp(&b.due));
        fired.into_iter().map(|p| (p.app, p.req)).collect()
    }

    /// The soonest trigger (ms), for the shell's wake-up.
    pub fn next_due(&self) -> Option<f64> {
        self.pending.iter().map(|p| p.due).min_by(f64::total_cmp)
    }

    /// The store entry that shows a delivered request.
    pub fn record_delivery(&mut self, app: &str, req: NotifRequest, store_id: u64) {
        self.delivered.push(Delivered { app: app.to_string(), req, store_id });
    }

    /// `removeDeliveredNotifications` / `removePendingNotificationRequests`; empty `ids` removes every one of the
    /// app's. Returns the store entries to drop (delivered only).
    pub fn remove(&mut self, app: &str, delivered: bool, ids: &[String]) -> Vec<u64> {
        let hit = |id: &str| ids.is_empty() || ids.iter().any(|i| i == id);
        if !delivered {
            self.pending.retain(|p| !(p.app == app && hit(&p.req.id)));
            return Vec::new();
        }
        let (gone, keep): (Vec<_>, Vec<_>) = std::mem::take(&mut self.delivered).into_iter().partition(|d| d.app == app && hit(&d.req.id));
        self.delivered = keep;
        gone.into_iter().map(|d| d.store_id).collect()
    }

    /// `getDeliveredNotifications` / `getPendingNotificationRequests`: the identifiers, oldest first.
    pub fn list(&self, app: &str, delivered: bool) -> Vec<String> {
        if delivered {
            self.delivered.iter().filter(|d| d.app == app).map(|d| d.req.id.clone()).collect()
        } else {
            self.pending.iter().filter(|p| p.app == app).map(|p| p.req.id.clone()).collect()
        }
    }

    /// Forgets delivered requests whose store entry is gone (the user cleared it, or the store evicted it).
    pub fn prune(&mut self, exists: impl Fn(u64) -> bool) {
        self.delivered.retain(|d| exists(d.store_id));
    }

    pub fn store_ids(&self) -> Vec<u64> {
        self.delivered.iter().map(|d| d.store_id).collect()
    }

    pub fn delivered_for(&self, store_id: u64) -> Option<&Delivered> {
        self.delivered.iter().find(|d| d.store_id == store_id)
    }

    /// The newest delivered request of `app` (what opening the app from its Dock icon or a banner means).
    pub fn newest_for(&self, app: &str) -> Option<&Delivered> {
        self.delivered.iter().rev().find(|d| d.app == app)
    }

    /// The action buttons of a delivered notification, in order (empty without a category).
    pub fn actions(&self, store_id: u64) -> Vec<NotifAction> {
        let Some(d) = self.delivered_for(store_id) else { return Vec::new() };
        self.category(&d.app, &d.req.category).map(|c| c.actions.clone()).unwrap_or_default()
    }

    /// The user chose `action` on a delivered notification (with `text` for a text-input action). Returns the app and
    /// the message for it, or None when the action does not exist in the notification's category. The default action
    /// is always allowed; `Dismiss` only where... always, the app decides whether it cares.
    pub fn respond(&self, store_id: u64, action: &str, text: &str) -> Option<(String, ToApp)> {
        let d = self.delivered_for(store_id)?;
        let text = if action == NOTIF_DEFAULT_ACTION || action == NOTIF_DISMISS_ACTION {
            String::new()
        } else {
            let a = self.category(&d.app, &d.req.category)?.actions.iter().find(|a| a.id == action)?;
            if a.text_input.is_some() {
                text.to_string()
            } else {
                String::new()
            }
        };
        Some((d.app.clone(), ToApp::NotificationAction { id: d.req.id.clone(), action: action.to_string(), text, thread: d.req.thread.clone(), category: d.req.category.clone() }))
    }

    /// A request waits until the person answers the first-time alert of its app (`take_held` returns it then).
    pub fn hold(&mut self, app: &str, slot: u8, request: u32, req: NotifRequest) {
        self.held.push((app.to_string(), slot, request, req));
    }

    /// The app's requests that waited for its answer, oldest first, as (slot, request id, request).
    pub fn take_held(&mut self, app: &str) -> Vec<(u8, u32, NotifRequest)> {
        let (mine, rest): (Vec<_>, Vec<_>) = std::mem::take(&mut self.held).into_iter().partition(|(a, ..)| a == app);
        self.held = rest;
        mine.into_iter().map(|(_, slot, request, req)| (slot, request, req)).collect()
    }

    /// Holds an answer for an app that is not running; delivered by `take_queued` when it starts.
    pub fn queue(&mut self, app: &str, msg: ToApp) {
        let n = self.queued.iter().filter(|(a, _)| a == app).count();
        if n >= MAX_QUEUED {
            if let Some(i) = self.queued.iter().position(|(a, _)| a == app) {
                self.queued.remove(i);
            }
        }
        self.queued.push((app.to_string(), msg));
    }

    pub fn take_queued(&mut self, app: &str) -> Vec<ToApp> {
        let (mine, rest): (Vec<_>, Vec<_>) = std::mem::take(&mut self.queued).into_iter().partition(|(a, _)| a == app);
        self.queued = rest;
        mine.into_iter().map(|(_, m)| m).collect()
    }

    /// The app was removed: everything of it goes.
    pub fn forget_app(&mut self, app: &str) {
        self.categories.remove(app);
        self.pending.retain(|p| p.app != app);
        self.delivered.retain(|d| d.app != app);
        self.queued.retain(|(a, _)| a != app);
        self.held.retain(|(a, ..)| a != app);
    }
}

/// What the settings query answers. Without the permission nothing is allowed; a muted app is stored and badged
/// nowhere, so its alert, sound and badge are off; Focus is reported on its own.
pub fn settings_for(permitted: bool, muted: bool, focus: bool) -> NotifSettings {
    let on = permitted && !muted;
    NotifSettings { authorized: permitted, alert: on, sound: on, badge: on, focus }
}

/// The text a delivered request shows: the subtitle goes between title and body the way Golden Gate stacks them.
pub fn display(req: &NotifRequest) -> (String, String) {
    let body = if req.subtitle.is_empty() { req.body.clone() } else if req.body.is_empty() { req.subtitle.clone() } else { format!("{}\n{}", req.subtitle, req.body) };
    (req.title.clone(), body)
}

/// True when the app `app` (its id) has the `notifications` permission in its manifest.
pub(super) fn permitted(app: &str) -> bool {
    super::apphost::installed().iter().any(|p| p.manifest.id == app && p.manifest.permissions.iter().any(|x| x == "notifications"))
}

/// Shell side: the sockets in, the notification UI out.
impl super::Shell {
    /// A `Notify*` message from the app in `slot`.
    pub(super) fn usernotif_msg(&mut self, slot: u8, msg: bifrost_proto::ToShell, t: f64) {
        use bifrost_proto::ToShell;
        let registered = super::apphost::installed();
        let Some(package) = registered.get(slot as usize) else { return };
        let app = package.manifest.id.clone();
        let name = package.manifest.name.clone();
        let permitted = package.manifest.permissions.iter().any(|p| p == "notifications");
        match msg {
            ToShell::NotifyCategories(list) if permitted => self.usernotif.set_categories(&app, list),
            // the first request of an app that was never asked waits for the first-time alert (`permission.rs`)
            ToShell::NotifyRequest { request, req } if permitted && !self.notify.store().is_asked(super::AppId::External(slot)) => {
                self.usernotif_hold(slot, &app, &name, request, req);
            }
            ToShell::NotifyRequest { request, req } => {
                let status = if permitted { self.usernotif_submit(slot, &app, req, t) } else { NotifStatus::NotPermitted };
                self.ext_send(slot as usize, &ToApp::NotificationStatus { request, status });
            }
            ToShell::NotifyRemove { delivered, ids } if permitted => {
                for id in self.usernotif.remove(&app, delivered, &ids) {
                    self.notify_remove(id);
                }
            }
            ToShell::NotifyQuery { request, delivered } if permitted => {
                self.usernotif_prune();
                let ids = self.usernotif.list(&app, delivered);
                self.ext_send(slot as usize, &ToApp::NotificationList { request, delivered, ids });
            }
            ToShell::NotifySettings { request } => {
                let muted = self.notify.store().is_muted(super::AppId::External(slot));
                let settings = settings_for(permitted, muted, self.dnd());
                self.ext_send(slot as usize, &ToApp::NotificationSettings { request, settings });
            }
            _ => {}
        }
    }

    /// Adds a request of a permitted app: it is delivered now, scheduled, or refused. Returns the status for the app.
    fn usernotif_submit(&mut self, slot: u8, app: &str, req: NotifRequest, t: f64) -> NotifStatus {
        match self.usernotif.add(app, req.clone(), t) {
            Added::Refused(status) => status,
            Added::Scheduled => NotifStatus::Ok,
            Added::Deliver { replaced } => {
                self.usernotif_deliver(slot, app, req, replaced, t);
                NotifStatus::Ok
            }
        }
    }

    /// A request of an app that was never asked waits for the first-time alert. The alert is queued once per app.
    fn usernotif_hold(&mut self, slot: u8, app: &str, name: &str, request: u32, req: NotifRequest) {
        self.usernotif.hold(app, slot, request, req);
        let id = super::AppId::External(slot);
        let queued = self.notify.ask_queue.iter().any(|(a, _)| *a == id);
        let showing = self.notify.ask.as_ref().is_some_and(|s| s.app() == id);
        if !queued && !showing {
            self.notify.ask_queue.push((id, name.to_string()));
        }
    }

    /// Shows the next first-time alert when none is on screen (the shell's frame tick).
    pub(super) fn usernotif_ask_tick(&mut self, t: f64) {
        if self.notify.ask.is_some() || self.notify.ask_queue.is_empty() {
            return;
        }
        let (id, name) = self.notify.ask_queue.remove(0);
        match super::notify::permission::Sheet::new(&self.ui, id, &name, t.max(0.0) as u64) {
            Some(sheet) => self.notify.ask = Some(sheet),
            // the alert could not be made, so the question stays unanswered, which grants nothing
            None => self.usernotif_answer(id, false, t),
        }
        self.dirty = true;
        self.drawn = super::Drawn::Nothing;
    }

    /// Touches while the first-time alert is up: it takes every touch until the person answers it.
    pub(super) fn usernotif_ask_event(&mut self, ev: super::Event) -> Option<Vec<super::Effect>> {
        self.notify.ask.as_ref()?;
        if let super::Event::Up(x, y) = ev {
            let hit = self.notify.ask.as_ref().and_then(|s| s.hit(&self.ui, self.w, self.h, x, y));
            if let Some(button) = hit {
                if let Some(sheet) = self.notify.ask.take() {
                    let app = sheet.app();
                    let t = self.notify.t;
                    // the question has no expiry, so the clock does not change the answer
                    let allowed = sheet.finish(button, t.max(0.0) as u64).unwrap_or(false);
                    self.usernotif_answer(app, allowed, t);
                }
            }
        }
        self.drawn = super::Drawn::Nothing;
        self.dirty = true;
        Some(vec![])
    }

    /// The person answered the first-time alert of `id`: the answer is stored and the Allow Notifications switch
    /// follows it. The requests the app made while it waited get their status now (refused when not allowed).
    fn usernotif_answer(&mut self, id: super::AppId, allowed: bool, t: f64) {
        self.notify.answer_permission(id, allowed);
        let super::AppId::External(slot) = id else { return };
        let Some(app) = super::apphost::installed().get(slot as usize).map(|p| p.manifest.id.clone()) else { return };
        for (slot, request, req) in self.usernotif.take_held(&app) {
            let status = if allowed { self.usernotif_submit(slot, &app, req, t) } else { NotifStatus::NotPermitted };
            self.ext_send(slot as usize, &ToApp::NotificationStatus { request, status });
        }
    }

    fn usernotif_deliver(&mut self, slot: u8, app: &str, req: NotifRequest, replaced: Vec<u64>, t: f64) {
        for id in replaced {
            self.notify_remove(id);
        }
        let (title, body) = display(&req);
        let mut n = super::notify::Notification::new(super::AppId::External(slot), title, body, self.wall.now(t));
        n.thread = req.thread.clone();
        let id = self.notify_id(n);
        if let Some(count) = req.badge {
            if let Some(Some(h)) = self.ext.get_mut(slot as usize) {
                h.badge = if count == 0 { String::new() } else { super::apphost::clean_badge(&count.to_string()) };
                self.dirty = true;
            }
        }
        self.usernotif.record_delivery(app, req, id);
    }

    /// Delivers the requests whose trigger fired; called every tick.
    pub(super) fn usernotif_tick(&mut self, t: f64) {
        if self.usernotif.next_due().is_some_and(|due| due <= t) {
            for (app, req) in self.usernotif.due(t) {
                self.usernotif_fire(&app, req, t);
            }
        }
    }

    /// Delivers a request whose trigger fired now, from the registry or from the `notification-schedule` service. An
    /// earlier delivery with the same identifier is replaced, as for a request that was due at once.
    pub(super) fn usernotif_fire(&mut self, app: &str, req: NotifRequest, t: f64) {
        if let Some(slot) = super::apphost::slot_of(app) {
            let replaced = self.usernotif.take_delivered(app, &req.id);
            self.usernotif_deliver(slot as u8, app, req, replaced, t);
        }
    }

    /// Forgets delivered requests the user cleared from the notification UI.
    fn usernotif_prune(&mut self) {
        let gone: Vec<u64> = self.usernotif.store_ids().into_iter().filter(|&id| !self.notify_exists(id)).collect();
        if !gone.is_empty() {
            self.usernotif.prune(|id| !gone.contains(&id));
        }
    }

    /// The user chose `action` (with `text` for a reply) on notification `store_id`. The app is told; if it is not
    /// running it is started and told when it is up. False when the notification or action does not exist.
    pub fn usernotif_respond(&mut self, store_id: u64, action: &str, text: &str, t: f64) -> bool {
        let foreground = self.usernotif_foreground(store_id, action);
        let Some((app, msg)) = self.usernotif.respond(store_id, action, text) else { return false };
        let Some(slot) = super::apphost::slot_of(&app) else { return false };
        let running = self.ext.get(slot).and_then(Option::as_ref).is_some_and(|h| h.running());
        if running {
            self.ext_send(slot, &msg);
        } else {
            self.usernotif.queue(&app, msg);
            self.ext_open(slot);
        }
        if foreground && self.desk_active() {
            self.desk_open(super::AppId::External(slot as u8), t);
        }
        true
    }

    fn usernotif_foreground(&self, store_id: u64, action: &str) -> bool {
        action == NOTIF_DEFAULT_ACTION || self.usernotif.actions(store_id).iter().any(|a| a.id == action && a.foreground)
    }

    /// The user opened `app` from one of its notifications: the app hears the default action for its newest one.
    pub(super) fn usernotif_tapped(&mut self, app: super::AppId) {
        let super::AppId::External(slot) = app else { return };
        let registered = super::apphost::installed();
        let Some(package) = registered.get(slot as usize) else { return };
        self.usernotif_prune();
        let Some(store_id) = self.usernotif.newest_for(&package.manifest.id).map(|d| d.store_id) else { return };
        let Some((id, msg)) = self.usernotif.respond(store_id, NOTIF_DEFAULT_ACTION, "") else { return };
        let running = self.ext.get(slot as usize).and_then(Option::as_ref).is_some_and(|h| h.running());
        if running {
            self.ext_send(slot as usize, &msg);
        } else {
            self.usernotif.queue(&id, msg);
        }
    }

    /// An app just started: what it missed while it was not running.
    pub(super) fn usernotif_started(&mut self, slot: usize) {
        let registered = super::apphost::installed();
        let Some(package) = registered.get(slot) else { return };
        for msg in self.usernotif.take_queued(&package.manifest.id) {
            self.ext_send(slot, &msg);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_proto::TextInput;

    const APP: &str = "is.example.chat";

    #[test]
    fn a_denied_app_is_told_nothing_is_allowed_but_still_sees_focus() {
        assert_eq!(settings_for(false, false, false), NotifSettings::default());
        assert_eq!(settings_for(false, false, false).bits(), 0);
        assert_eq!(settings_for(false, false, true).bits(), 16);
        assert!(!settings_for(false, true, false).authorized);
    }

    #[test]
    fn a_granted_app_gets_alert_sound_and_badge_until_it_is_muted() {
        let open = settings_for(true, false, false);
        assert!(open.authorized && open.alert && open.sound && open.badge);
        assert!(!open.focus);
        assert_eq!(open.bits(), 15);
        let muted = settings_for(true, true, false);
        assert_eq!(muted, NotifSettings { authorized: true, ..Default::default() });
        assert_eq!(muted.bits(), 1);
    }

    #[test]
    fn focus_is_reported_for_a_granted_app_without_changing_its_grant() {
        assert_eq!(settings_for(true, false, true).bits(), 31);
        assert_eq!(settings_for(true, true, true).bits(), 17);
    }

    fn req(id: &str) -> NotifRequest {
        NotifRequest { id: id.into(), title: "T".into(), body: "B".into(), ..Default::default() }
    }

    fn with_category() -> Registry {
        let mut r = Registry::default();
        r.set_categories(
            APP,
            vec![NotifCategory {
                id: "MSG".into(),
                actions: vec![
                    NotifAction::button("archive", "Archive"),
                    NotifAction { id: "reply".into(), title: "Reply".into(), destructive: false, foreground: false, text_input: Some(TextInput { button_title: "Send".into(), placeholder: "".into() }) },
                    NotifAction { destructive: true, ..NotifAction::button("delete", "Delete") },
                ],
            }],
        );
        r
    }

    #[test]
    fn invalid_requests_are_refused() {
        let mut r = Registry::default();
        assert_eq!(r.add(APP, NotifRequest { id: String::new(), title: "T".into(), ..Default::default() }, 0.0), Added::Refused(NotifStatus::Param));
        assert_eq!(r.add(APP, NotifRequest { id: "a".into(), ..Default::default() }, 0.0), Added::Refused(NotifStatus::Param));
        assert_eq!(r.add(APP, NotifRequest { category: "NOPE".into(), ..req("a") }, 0.0), Added::Refused(NotifStatus::Param));
        assert_eq!(r.add(APP, req("a"), 0.0), Added::Deliver { replaced: vec![] });
    }

    #[test]
    fn same_identifier_replaces_delivered_and_pending() {
        let mut r = Registry::default();
        assert_eq!(r.add(APP, req("a"), 0.0), Added::Deliver { replaced: vec![] });
        r.record_delivery(APP, req("a"), 7);
        assert_eq!(r.add(APP, req("a"), 1.0), Added::Deliver { replaced: vec![7] });
        assert_eq!(r.add(APP, NotifRequest { delay_ms: 500, ..req("b") }, 0.0), Added::Scheduled);
        assert_eq!(r.add(APP, NotifRequest { delay_ms: 900, ..req("b") }, 0.0), Added::Scheduled);
        assert_eq!(r.list(APP, false), vec!["b".to_string()]);
        // another app's identifier is a different notification
        assert_eq!(r.add("other", req("a"), 0.0), Added::Deliver { replaced: vec![] });
    }

    #[test]
    fn repeating_trigger_fires_on_its_grid_until_removed() {
        let mut r = Registry::default();
        assert_eq!(r.add(APP, NotifRequest { delay_ms: 100, repeats: true, ..req("tick") }, 0.0), Added::Scheduled);
        assert!(r.due(99.0).is_empty());
        assert_eq!(r.due(100.0).len(), 1);
        assert_eq!(r.next_due(), Some(200.0));
        // the shell slept past two triggers: one delivery, and the next date is the first one after now
        let fired = r.due(350.0);
        assert_eq!(fired.iter().map(|(_, q)| q.id.as_str()).collect::<Vec<_>>(), ["tick"]);
        assert_eq!(r.next_due(), Some(400.0));
        assert_eq!(r.list(APP, false), vec!["tick".to_string()]);
        r.remove(APP, false, &[]);
        assert!(r.due(10_000.0).is_empty());
        assert_eq!(r.next_due(), None);
    }

    #[test]
    fn repeats_without_a_delay_deliver_once() {
        let mut r = Registry::default();
        assert_eq!(r.add(APP, NotifRequest { repeats: true, ..req("now") }, 0.0), Added::Deliver { replaced: vec![] });
        assert_eq!(r.next_due(), None);
        assert_eq!(r.add(APP, NotifRequest { delay_ms: 50, ..req("once") }, 0.0), Added::Scheduled);
        assert_eq!(r.due(50.0).len(), 1);
        assert_eq!(r.next_due(), None, "a one-off trigger is not re-armed");
    }

    #[test]
    fn triggers_fire_in_order() {
        let mut r = Registry::default();
        r.add(APP, NotifRequest { delay_ms: 2000, ..req("late") }, 100.0);
        r.add(APP, NotifRequest { delay_ms: 500, ..req("soon") }, 100.0);
        assert_eq!(r.next_due(), Some(600.0));
        assert!(r.due(599.0).is_empty());
        let fired = r.due(2200.0);
        assert_eq!(fired.iter().map(|(_, q)| q.id.as_str()).collect::<Vec<_>>(), ["soon", "late"]);
        assert_eq!(r.next_due(), None);
    }

    #[test]
    fn pending_is_capped_per_app() {
        let mut r = Registry::default();
        for i in 0..MAX_PENDING {
            assert_eq!(r.add(APP, NotifRequest { delay_ms: 10, ..req(&i.to_string()) }, 0.0), Added::Scheduled);
        }
        assert_eq!(r.add(APP, NotifRequest { delay_ms: 10, ..req("x") }, 0.0), Added::Refused(NotifStatus::Full));
        assert_eq!(r.add("other", NotifRequest { delay_ms: 10, ..req("x") }, 0.0), Added::Scheduled);
    }

    #[test]
    fn remove_and_list_cover_both_kinds() {
        let mut r = Registry::default();
        for (i, id) in ["a", "b", "c"].iter().enumerate() {
            r.record_delivery(APP, req(id), 10 + i as u64);
        }
        r.record_delivery("other", req("a"), 99);
        assert_eq!(r.list(APP, true), ["a", "b", "c"]);
        assert_eq!(r.remove(APP, true, &["b".to_string()]), vec![11]);
        assert_eq!(r.remove(APP, true, &[]), vec![10, 12]);
        assert_eq!(r.list("other", true), ["a"]);
        r.add(APP, NotifRequest { delay_ms: 100, ..req("p") }, 0.0);
        assert!(r.remove(APP, false, &["p".to_string()]).is_empty());
        assert!(r.list(APP, false).is_empty());
    }

    #[test]
    fn prune_forgets_cleared_entries() {
        let mut r = Registry::default();
        r.record_delivery(APP, req("a"), 1);
        r.record_delivery(APP, req("b"), 2);
        r.prune(|id| id == 2);
        assert_eq!(r.list(APP, true), ["b"]);
    }

    #[test]
    fn responses_check_the_category() {
        let mut r = with_category();
        r.record_delivery(APP, NotifRequest { category: "MSG".into(), thread: "room".into(), ..req("m1") }, 5);
        assert_eq!(r.actions(5).iter().map(|a| a.id.as_str()).collect::<Vec<_>>(), ["archive", "reply", "delete"]);
        let (app, msg) = r.respond(5, "reply", "on my way").unwrap();
        assert_eq!(app, APP);
        assert_eq!(msg, ToApp::NotificationAction { id: "m1".into(), action: "reply".into(), text: "on my way".into(), thread: "room".into(), category: "MSG".into() });
        // text is only passed to a text-input action
        let ToApp::NotificationAction { text, .. } = r.respond(5, "archive", "ignored").unwrap().1 else { panic!() };
        assert_eq!(text, "");
        // an action the category does not have, and an unknown notification, are refused
        assert!(r.respond(5, "launch-missiles", "").is_none());
        assert!(r.respond(6, "archive", "").is_none());
        // default and dismiss exist on every notification
        let ToApp::NotificationAction { action, .. } = r.respond(5, NOTIF_DEFAULT_ACTION, "x").unwrap().1 else { panic!() };
        assert_eq!(action, NOTIF_DEFAULT_ACTION);
        assert!(r.respond(5, NOTIF_DISMISS_ACTION, "").is_some());
    }

    #[test]
    fn answers_wait_for_a_stopped_app() {
        let mut r = Registry::default();
        let m = |n: &str| ToApp::NotificationAction { id: n.into(), action: NOTIF_DEFAULT_ACTION.into(), text: String::new(), thread: String::new(), category: String::new() };
        r.queue(APP, m("1"));
        r.queue("other", m("x"));
        r.queue(APP, m("2"));
        assert_eq!(r.take_queued(APP), vec![m("1"), m("2")]);
        assert!(r.take_queued(APP).is_empty());
        for i in 0..MAX_QUEUED + 3 {
            r.queue(APP, m(&i.to_string()));
        }
        assert_eq!(r.take_queued(APP).len(), MAX_QUEUED);
        assert_eq!(r.take_queued("other"), vec![m("x")]);
    }

    #[test]
    fn forgetting_an_app_clears_everything() {
        let mut r = with_category();
        r.record_delivery(APP, req("a"), 1);
        r.add(APP, NotifRequest { delay_ms: 10, ..req("p") }, 0.0);
        r.queue(APP, ToApp::Resume);
        r.forget_app(APP);
        assert!(r.list(APP, true).is_empty() && r.list(APP, false).is_empty() && r.take_queued(APP).is_empty());
        assert!(r.category(APP, "MSG").is_none());
    }

    #[test]
    fn requests_held_for_the_first_alert_come_back_with_the_answer_in_order() {
        let mut r = Registry::default();
        r.hold(APP, 2, 7, req("a"));
        r.hold("other", 3, 8, req("x"));
        r.hold(APP, 2, 9, req("b"));
        assert_eq!(r.take_held(APP), vec![(2, 7, req("a")), (2, 9, req("b"))]);
        assert_eq!(r.take_held("other"), vec![(3, 8, req("x"))]);
        assert!(r.take_held(APP).is_empty());
        r.hold(APP, 2, 1, req("gone"));
        r.forget_app(APP);
        assert!(r.take_held(APP).is_empty());
    }

    #[test]
    fn display_stacks_subtitle_over_body() {
        assert_eq!(display(&NotifRequest { title: "T".into(), subtitle: "S".into(), body: "B".into(), ..Default::default() }), ("T".to_string(), "S\nB".to_string()));
        assert_eq!(display(&NotifRequest { title: "T".into(), subtitle: "S".into(), ..Default::default() }).1, "S");
        assert_eq!(display(&NotifRequest { title: "T".into(), body: "B".into(), ..Default::default() }).1, "B");
    }
}
