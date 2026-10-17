//! `UNUserNotificationCenter` on the shell side: the categories each app registered, requests waiting for their
//! trigger, the delivered requests (mapped to the entries of the notification store) and the answers that wait for an
//! app that is not running. The registry is plain data; `Shell` feeds it from the app sockets (`apphost.rs`) and
//! shows what it hands back through the normal notification UI (`notify/`).
//!
//! Identity is always the app id of the connection, never anything in the message.

use bifrost_proto::{NotifAction, NotifCategory, NotifRequest, NotifSettings, NotifStatus, ToApp, NOTIF_AUTH_ALERT, NOTIF_AUTH_BADGE, NOTIF_AUTH_MASK, NOTIF_AUTH_PROVISIONAL, NOTIF_AUTH_SOUND, NOTIF_DEFAULT_ACTION, NOTIF_DISMISS_ACTION, NOTIF_PRESENT_BADGE, NOTIF_PRESENT_BANNER, NOTIF_PRESENT_LIST, NOTIF_PRESENT_SOUND, NotifTrigger, NOTIF_CATEGORY_CUSTOM_DISMISS, NOTIF_CATEGORY_HIDDEN_SHOW_SUBTITLE, NOTIF_CATEGORY_HIDDEN_SHOW_TITLE};
use crate::shell::clock::alarm::Alarm;
use super::notify_page::AppNotif;
use crate::gfx::Image;
use std::path::PathBuf;
use std::collections::{HashMap, HashSet};

/// Pending requests one app may hold (`UNUserNotificationCenter` allows 64 local notifications).
pub const MAX_PENDING: usize = 64;
/// Answers kept for an app that is not running.
const MAX_QUEUED: usize = 32;
/// The largest attachment file the shell reads (bytes), and the side of its thumbnail (pixels).
const MAX_ATTACHMENT_BYTES: u64 = 4 * 1024 * 1024;
const ATTACHMENT_PX: usize = 96;
/// How long a frontmost app has to answer `willPresent` (shell clock, ms). No answer hides the notification, which is
/// what an app without the delegate gets.
pub const PRESENT_TIMEOUT_MS: f64 = 1000.0;

/// A notification waiting for the `willPresent` answer of the frontmost app that it arrived for.
#[derive(Clone, Debug, PartialEq)]
pub struct Presenting {
    pub token: u32,
    pub app: String,
    pub slot: u8,
    pub req: NotifRequest,
    /// store entries of an earlier delivery with the same identifier, replaced once this one shows
    pub replaced: Vec<u64>,
    /// shell clock (ms) at which the answer is given up on
    pub deadline: f64,
}

/// The `UNNotificationPresentationOptions` an app gets while it is in front: what it asked for, cut to what the grant
/// and the mute state allow (`foreground_presentation` in the SDK model). `grant` None means the app was never decided.
pub fn presentation_plan(asked: u8, grant: Option<u8>, muted: bool) -> u8 {
    if grant == Some(0) || muted {
        return 0;
    }
    let bits = grant.unwrap_or(NOTIF_AUTH_ALERT | NOTIF_AUTH_SOUND | NOTIF_AUTH_BADGE);
    let mut out = 0;
    if bits & NOTIF_AUTH_ALERT != 0 {
        out |= asked & (NOTIF_PRESENT_BANNER | NOTIF_PRESENT_LIST);
    }
    if bits & NOTIF_AUTH_SOUND != 0 {
        out |= asked & NOTIF_PRESENT_SOUND;
    }
    if bits & NOTIF_AUTH_BADGE != 0 {
        out |= asked & NOTIF_PRESENT_BADGE;
    }
    out
}

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

/// A calendar-trigger request waiting for its next time. It is kept here, with the interval requests, so the wire path
/// and the `notification-schedule` service share one pending list; the service saves and restores it.
#[derive(Clone, Debug, PartialEq)]
pub struct Calendar {
    pub app: String,
    pub req: NotifRequest,
    pub alarm: Alarm,
    pub repeats: bool,
    /// Unix time (s) of the next delivery
    pub due: i64,
}

#[derive(Default)]
pub struct Registry {
    categories: HashMap<String, Vec<NotifCategory>>,
    pending: Vec<Pending>,
    /// the calendar-trigger requests of every app (see `Calendar`)
    pub calendars: Vec<Calendar>,
    delivered: Vec<Delivered>,
    queued: Vec<(String, ToApp)>,
    /// requests that wait for the first-time alert of their app: app, slot, request id, request
    held: Vec<(String, u8, u32, NotifRequest)>,
    /// scheduled requests that wait for the first-time alert of their app: app, slot, request id, request, trigger
    held_schedules: Vec<(String, u8, u32, NotifRequest, NotifTrigger)>,
    /// `requestAuthorization` calls that wait for the first-time alert of their app: app, slot, request id, options
    auth_held: Vec<(String, u8, u32, u8)>,
    /// What each app was granted, as `NOTIF_AUTH_*` bits (provisional included); 0 is a denial. Saved to `granted_path`.
    granted: HashMap<String, u8>,
    granted_path: Option<PathBuf>,
    /// apps that opted in to `willPresent` (`NotifyPresentDelegate`)
    presenters: HashSet<String>,
    /// notifications that wait for their app's `willPresent` answer
    presenting: Vec<Presenting>,
    next_token: u32,
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

    /// The pending interval requests as `(app, request, due)`, with `due` in Unix milliseconds: `t` is the shell
    /// clock now and `now_ms` the Unix time now. This is what `notification-schedule` saves across a restart.
    pub fn pending_rows(&self, t: f64, now_ms: i64) -> Vec<(String, NotifRequest, i64)> {
        self.pending.iter().map(|p| (p.app.clone(), p.req.clone(), now_ms + (p.due - t).round() as i64)).collect()
    }

    /// Puts back a pending request saved by `pending_rows`; `due` is its trigger on the shell clock.
    pub fn restore_pending(&mut self, app: &str, req: NotifRequest, due: f64) {
        self.pending.retain(|p| !(p.app == app && p.req.id == req.id));
        self.pending.push(Pending { app: app.to_string(), req, due });
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
    /// Forgets the delivered requests whose store entry is gone, and returns them (oldest first).
    pub fn prune(&mut self, exists: impl Fn(u64) -> bool) -> Vec<Delivered> {
        let (gone, keep): (Vec<_>, Vec<_>) = std::mem::take(&mut self.delivered).into_iter().partition(|d| !exists(d.store_id));
        self.delivered = keep;
        gone
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
        Some((d.app.clone(), ToApp::NotificationAction { id: d.req.id.clone(), action: action.to_string(), text, thread: d.req.thread.clone(), category: d.req.category.clone(), user_info: d.req.user_info.clone() }))
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

    pub fn hold_schedule(&mut self, app: &str, slot: u8, request: u32, req: NotifRequest, trigger: NotifTrigger) {
        self.held_schedules.push((app.to_string(), slot, request, req, trigger));
    }

    /// The app's scheduled requests that waited for its answer, oldest first, as (slot, request id, request, trigger).
    pub fn take_held_schedules(&mut self, app: &str) -> Vec<(u8, u32, NotifRequest, NotifTrigger)> {
        let (mine, rest): (Vec<_>, Vec<_>) = std::mem::take(&mut self.held_schedules).into_iter().partition(|(a, ..)| a == app);
        self.held_schedules = rest;
        mine.into_iter().map(|(_, slot, request, req, trigger)| (slot, request, req, trigger)).collect()
    }

    pub fn hold_authorization(&mut self, app: &str, slot: u8, request: u32, options: u8) {
        self.auth_held.push((app.to_string(), slot, request, options));
    }

    /// The app's authorization requests that waited for its answer, oldest first, as (slot, request id, options).
    pub fn take_authorization(&mut self, app: &str) -> Vec<(u8, u32, u8)> {
        let (mine, rest): (Vec<_>, Vec<_>) = std::mem::take(&mut self.auth_held).into_iter().partition(|(a, ..)| a == app);
        self.auth_held = rest;
        mine.into_iter().map(|(_, slot, request, options)| (slot, request, options)).collect()
    }

    /// What the app was granted (`NOTIF_AUTH_*` bits), or None when it was never decided by `requestAuthorization`.
    pub fn granted(&self, app: &str) -> Option<u8> {
        self.granted.get(app).copied()
    }

    /// `NotifyPresentDelegate`: the app is asked (or not) how to present a notification that arrives while it is in front.
    pub fn set_presenter(&mut self, app: &str, on: bool) {
        if on {
            self.presenters.insert(app.to_string());
        } else {
            self.presenters.remove(app);
        }
    }

    pub fn presents(&self, app: &str) -> bool {
        self.presenters.contains(app)
    }

    /// Holds a notification until its app answers `willPresent`; returns the token the app must answer with.
    pub fn hold_presentation(&mut self, app: &str, slot: u8, req: NotifRequest, replaced: Vec<u64>, deadline: f64) -> u32 {
        self.next_token = self.next_token.wrapping_add(1).max(1);
        let token = self.next_token;
        self.presenting.push(Presenting { token, app: app.to_string(), slot, req, replaced, deadline });
        token
    }

    /// The held notification of `token`, once; None for an unknown or expired token.
    pub fn take_presentation(&mut self, token: u32) -> Option<Presenting> {
        let at = self.presenting.iter().position(|p| p.token == token)?;
        Some(self.presenting.remove(at))
    }

    /// Drops the held notifications whose app did not answer in time: they are not shown.
    pub fn expire_presentations(&mut self, now: f64) {
        self.presenting.retain(|p| p.deadline > now);
    }

    /// Records what the app was granted and saves it.
    pub fn set_granted(&mut self, app: &str, bits: u8) {
        self.granted.insert(app.to_string(), bits);
        self.save_granted();
    }

    /// Reads the grants saved at `path` and saves there from now on; malformed lines are skipped.
    pub fn load_granted(&mut self, path: PathBuf) {
        self.granted = std::fs::read_to_string(&path)
            .unwrap_or_default()
            .lines()
            .filter_map(|line| {
                let (app, bits) = line.split_once('\t')?;
                let bits: u8 = bits.parse().ok()?;
                (!app.is_empty() && bits & !NOTIF_AUTH_MASK == 0).then(|| (app.to_string(), bits))
            })
            .collect();
        self.granted_path = Some(path);
    }

    fn save_granted(&self) {
        let Some(path) = &self.granted_path else { return };
        let mut lines: Vec<String> = self.granted.iter().map(|(app, bits)| format!("{app}\t{bits}\n")).collect();
        lines.sort();
        if let Err(e) = crate::shell::prefs::write_atomic(path, &lines.concat()) {
            eprintln!("springboard: authorization.ini: {e}");
        }
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
        self.auth_held.retain(|(a, ..)| a != app);
        if self.granted.remove(app).is_some() {
            self.save_granted();
        }
    }
}

/// What the settings query answers. Without the permission nothing is allowed; a muted app is stored and badged
/// nowhere, so its alert, sound and badge are off; Focus is reported on its own.
pub fn settings_for(permitted: bool, muted: bool, focus: bool) -> NotifSettings {
    let on = permitted && !muted;
    NotifSettings { authorized: permitted, alert: on, sound: on, badge: on, focus }
}

/// The banner, sound and badge of a delivery after the app's page: an alert needs the Banners row, a sound the Sounds
/// row and a badge the Badges row.
pub fn gate_by_page(banner: bool, sound: bool, badge: bool, page: &AppNotif) -> (bool, bool, bool) {
    (banner && page.banners, sound && page.sounds, badge && page.badges)
}

/// The settings an app's query answers, with the app's page applied: its alert, sound and badge are off when the
/// page turns them off.
pub fn apply_page(mut settings: NotifSettings, page: &AppNotif) -> NotifSettings {
    settings.alert &= page.banners;
    settings.sound &= page.sounds;
    settings.badge &= page.badges;
    settings
}

/// The text a delivered request shows: the subtitle goes between title and body the way Golden Gate stacks them.
pub fn display(req: &NotifRequest) -> (String, String) {
    let body = if req.subtitle.is_empty() { req.body.clone() } else if req.body.is_empty() { req.subtitle.clone() } else { format!("{}\n{}", req.subtitle, req.body) };
    (req.title.clone(), body)
}

/// The text shown while previews are hidden (the app's Show Previews setting, or the lock screen): the body never shows,
/// and the title and subtitle show only when the category's hidden-preview options ask for them. The subtitle then shows
/// alone in the body.
pub fn hidden_text(options: u8, req: &NotifRequest) -> (String, String) {
    let title = if options & NOTIF_CATEGORY_HIDDEN_SHOW_TITLE != 0 { req.title.clone() } else { String::new() };
    let body = if options & NOTIF_CATEGORY_HIDDEN_SHOW_SUBTITLE != 0 { req.subtitle.clone() } else { String::new() };
    (title, body)
}

/// What a delivery shows for a grant (None when the app was never decided): (banner, sound). The banner needs the alert
/// option and the sound needs the sound option and an unmuted app; a request without a sound (kind 0) plays none. A
/// critical sound has no grant here, so it plays as the default sound.
pub fn delivery_plan(grant: Option<u8>, muted: bool, sound_kind: u8) -> (bool, bool) {
    let banner = grant.is_none_or(|bits| bits & NOTIF_AUTH_ALERT != 0);
    let sound = sound_kind != 0 && !muted && grant.is_none_or(|bits| bits & NOTIF_AUTH_SOUND != 0);
    (banner, sound)
}

/// The sound a delivery plays, by the request's `content.sound` kind: none for 0, the request's own bundled sound for
/// 2 (when it names one), and otherwise the person's alert sound from Settings > Sound. Critical sounds (3 and 4) have
/// no sound of their own here, so they play the alert sound too.
pub fn request_sound(sound_kind: u8, sound_name: &str, alert: &str) -> Option<String> {
    match sound_kind {
        0 => None,
        2 if !sound_name.is_empty() => Some(sound_name.to_string()),
        _ => Some(alert.to_string()),
    }
}

#[cfg(test)]
mod sound_tests {
    use super::*;

    #[test]
    fn the_apps_page_turns_off_its_banners_sounds_and_badges() {
        let page = AppNotif { banners: false, sounds: true, badges: false, ..AppNotif::default() };
        assert_eq!(gate_by_page(true, true, true, &page), (false, true, false));
        let all = settings_for(true, false, false);
        let s = apply_page(all, &page);
        assert!(!s.alert && s.sound && !s.badge && s.authorized, "the page clears only what it turns off");
        assert_eq!(apply_page(all, &AppNotif::default()), all, "the default page changes nothing");
    }

    #[test]
    fn a_request_without_a_sound_plays_none() {
        assert_eq!(request_sound(0, "Glass", "Chime"), None);
    }

    #[test]
    fn a_named_sound_plays_its_own_file_and_the_default_plays_the_alert_sound() {
        assert_eq!(request_sound(2, "Glass", "Chime").as_deref(), Some("Glass"));
        assert_eq!(request_sound(1, "Glass", "Chime").as_deref(), Some("Chime"), "the default follows Settings > Sound");
        assert_eq!(request_sound(2, "", "Chime").as_deref(), Some("Chime"), "a named sound with no name falls back");
        assert_eq!(request_sound(3, "", "Tink").as_deref(), Some("Tink"));
        assert_eq!(request_sound(4, "", "Tink").as_deref(), Some("Tink"));
    }
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
            ToShell::NotifyRequest { request, req } if permitted && !self.usernotif_decided(slot, &app) => {
                self.usernotif_hold(slot, &app, &name, request, req);
            }
            ToShell::NotifyRequest { request, req } => {
                let posting = permitted && self.usernotif.granted(&app).is_none_or(|bits| bits != 0);
                let status = if posting { self.usernotif_submit(slot, &app, req, t) } else { NotifStatus::NotPermitted };
                self.ext_send(slot as usize, &ToApp::NotificationStatus { request, status });
            }
            ToShell::NotifyRemove { delivered: false, ids } if permitted => self.usernotif_remove_pending(&app, &ids),
            ToShell::NotifyRemove { delivered, ids } if permitted => {
                for id in self.usernotif.remove(&app, delivered, &ids) {
                    self.notify_remove(id);
                }
            }
            ToShell::NotifyQuery { request, delivered } if permitted => {
                self.usernotif_prune();
                let ids = if delivered { self.usernotif.list(&app, true) } else { self.usernotif_pending(&app) };
                self.ext_send(slot as usize, &ToApp::NotificationList { request, delivered, ids });
            }
            // the first scheduled request of an app that was never asked waits for the first-time alert, as a request does
            ToShell::NotifySchedule { request, req, trigger } if permitted && !self.usernotif_decided(slot, &app) => {
                self.usernotif.hold_schedule(&app, slot, request, req, trigger);
                self.usernotif_ask_first_time(super::AppId::External(slot), &name);
            }
            ToShell::NotifySchedule { request, req, trigger } => {
                let posting = permitted && self.usernotif.granted(&app).is_none_or(|bits| bits != 0);
                let status = if posting { self.usernotif_schedule(&app, req, trigger, t) } else { NotifStatus::NotPermitted };
                self.ext_send(slot as usize, &ToApp::NotificationStatus { request, status });
            }
            ToShell::NotifyAuthorize { request, options } => self.usernotif_authorize(slot, &app, &name, request, options, permitted),
            ToShell::NotifyPresentDelegate { enabled } => self.usernotif.set_presenter(&app, enabled),
            ToShell::NotifyPresentReply { token, options } => self.usernotif_presented(token, options, t),
            ToShell::NotifySettings { request } => {
                let muted = self.notify.store().is_muted(super::AppId::External(slot));
                let page = self.app_notif(super::AppId::External(slot));
                let mut settings = apply_page(settings_for(permitted, muted, self.dnd()), &page);
                if let Some(bits) = self.usernotif.granted(&app) {
                    settings.authorized = permitted && bits != 0;
                    settings.alert &= bits & NOTIF_AUTH_ALERT != 0;
                    settings.sound &= bits & NOTIF_AUTH_SOUND != 0;
                    settings.badge &= bits & NOTIF_AUTH_BADGE != 0;
                }
                self.ext_send(slot as usize, &ToApp::NotificationSettings { request, settings });
            }
            _ => {}
        }
    }

    /// Schedules a request of a permitted app on a calendar or interval trigger (`ToShell::NotifySchedule`, and the
    /// `add` method of `notification-schedule`). An identifier replaces the pending request of either kind. The
    /// returned status is the one the app gets.
    pub(super) fn usernotif_schedule(&mut self, app: &str, req: NotifRequest, trigger: NotifTrigger, t: f64) -> NotifStatus {
        if let Some(status) = self.usernotif.check(app, &req) {
            return status;
        }
        let id = req.id.clone();
        self.usernotif.calendars.retain(|c| !(c.app == app && c.req.id == id));
        self.usernotif.remove(app, false, std::slice::from_ref(&id));
        if self.usernotif_pending(app).len() >= MAX_PENDING {
            return NotifStatus::Full;
        }
        match trigger {
            NotifTrigger::Interval { seconds, repeats } => {
                let delay_ms = seconds.saturating_mul(1000);
                match self.usernotif.add(app, NotifRequest { delay_ms, repeats, ..req }, t) {
                    Added::Scheduled => NotifStatus::Ok,
                    Added::Refused(status) => status,
                    Added::Deliver { .. } => NotifStatus::Param,
                }
            }
            NotifTrigger::Calendar { hour, minute, weekdays, repeats } => {
                let now = self.wall.now(t);
                let offset = super::clock::offset_fn(None, self.wall.offset);
                let alarm = Alarm { repeat: weekdays, ..Alarm::new(u32::from(hour), u32::from(minute)) };
                match alarm.regular_fire(now, &offset) {
                    Some(due) => {
                        self.usernotif.calendars.push(Calendar { app: app.to_string(), req, alarm, repeats, due });
                        NotifStatus::Ok
                    }
                    None => NotifStatus::Param,
                }
            }
        }
    }

    /// The pending identifiers of both kinds: the interval ones, then the calendar ones, in the order they were added.
    pub(super) fn usernotif_pending(&self, app: &str) -> Vec<String> {
        let mut ids = self.usernotif.list(app, false);
        ids.extend(self.usernotif.calendars.iter().filter(|c| c.app == app).map(|c| c.req.id.clone()));
        ids
    }

    /// `removePendingNotificationRequests`: removes the pending requests of both kinds named by `ids`; empty removes all.
    pub(super) fn usernotif_remove_pending(&mut self, app: &str, ids: &[String]) {
        let hit = |id: &str| ids.is_empty() || ids.iter().any(|i| i == id);
        self.usernotif.calendars.retain(|c| !(c.app == app && hit(&c.req.id)));
        self.usernotif.remove(app, false, ids);
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
        self.usernotif_ask_first_time(super::AppId::External(slot), name);
    }

    /// Whether the app's first-time question is settled: asked on the alert, or granted provisionally.
    fn usernotif_decided(&self, slot: u8, app: &str) -> bool {
        self.notify.store().is_asked(super::AppId::External(slot)) || self.usernotif.granted(app).is_some()
    }

    /// `requestAuthorization`. An app already decided gets its stored grant without a prompt. A provisional request is
    /// granted at once, quietly, with no prompt. Otherwise the app waits for the first-time alert and is answered after
    /// the person chose, with the options it asked for (critical alerts are never granted).
    fn usernotif_authorize(&mut self, slot: u8, app: &str, name: &str, request: u32, options: u8, permitted: bool) {
        let id = super::AppId::External(slot);
        if !permitted {
            self.ext_send(slot as usize, &ToApp::NotificationAuthorized { request, granted: false });
        } else if let Some(bits) = self.usernotif.granted(app) {
            self.ext_send(slot as usize, &ToApp::NotificationAuthorized { request, granted: bits != 0 });
        } else if self.notify.store().is_asked(id) {
            // answered before the per-option grants existed: the single alert switch decides
            let granted = !self.notify.store().is_muted(id);
            self.ext_send(slot as usize, &ToApp::NotificationAuthorized { request, granted });
        } else if options & NOTIF_AUTH_PROVISIONAL != 0 {
            self.usernotif.set_granted(app, (options & (NOTIF_AUTH_BADGE | NOTIF_AUTH_SOUND | NOTIF_AUTH_ALERT)) | NOTIF_AUTH_PROVISIONAL);
            self.ext_send(slot as usize, &ToApp::NotificationAuthorized { request, granted: true });
        } else {
            self.usernotif.hold_authorization(app, slot, request, options);
            self.usernotif_ask_first_time(id, name);
        }
    }

    /// Queues the first-time alert of `id` once, unless it is already queued or on screen.
    fn usernotif_ask_first_time(&mut self, id: super::AppId, name: &str) {
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
        for (slot, request, req, trigger) in self.usernotif.take_held_schedules(&app) {
            let status = if allowed { self.usernotif_schedule(&app, req, trigger, t) } else { NotifStatus::NotPermitted };
            self.ext_send(slot as usize, &ToApp::NotificationStatus { request, status });
        }
        // the grant is the union of what the waiting requests asked for; a request held for a notification alone
        // asks for all three alert options
        let waiting = self.usernotif.take_authorization(&app);
        let asked = waiting.iter().fold(0, |bits, (_, _, options)| bits | options) & (NOTIF_AUTH_BADGE | NOTIF_AUTH_SOUND | NOTIF_AUTH_ALERT);
        let bits = match (allowed, waiting.is_empty()) {
            (false, _) => 0,
            (true, true) => NOTIF_AUTH_BADGE | NOTIF_AUTH_SOUND | NOTIF_AUTH_ALERT,
            (true, false) => asked,
        };
        self.usernotif.set_granted(&app, bits);
        for (slot, request, _) in waiting {
            self.ext_send(slot as usize, &ToApp::NotificationAuthorized { request, granted: bits != 0 });
        }
    }

    /// Delivers a request. An app in front that asked for `willPresent` is asked first; the notification shows when it
    /// answers (`usernotif_presented`), or never when it does not answer in time.
    fn usernotif_deliver(&mut self, slot: u8, app: &str, req: NotifRequest, replaced: Vec<u64>, t: f64) {
        let in_front = self.desk.as_ref().and_then(|d| d.front()).map(|w| w.app.parent()) == Some(super::AppId::External(slot));
        if in_front && self.usernotif.presents(app) {
            let (title, body) = display(&req);
            let (id, thread, category) = (req.id.clone(), req.thread.clone(), req.category.clone());
            let token = self.usernotif.hold_presentation(app, slot, req, replaced, t + PRESENT_TIMEOUT_MS);
            self.ext_send(slot as usize, &ToApp::NotificationWillPresent { token, id, thread, category, title, body });
            return;
        }
        self.usernotif_show(slot, app, req, replaced, t, None);
    }

    /// The app in front answered `willPresent` with `options`: shows the notification as it asked, cut to the grant.
    fn usernotif_presented(&mut self, token: u32, options: u8, t: f64) {
        let Some(p) = self.usernotif.take_presentation(token) else { return };
        let muted = self.notify.store().is_muted(super::AppId::External(p.slot));
        let plan = presentation_plan(options, self.usernotif.granted(&p.app), muted);
        if plan != 0 {
            self.usernotif_show(p.slot, &p.app, p.req, p.replaced, t, Some(plan));
        }
    }

    /// What a delivery of `req` shows for the app's Show Previews setting: hidden when set to Never, or when set to When
    /// Unlocked and the device is locked. The shell decides at delivery, so the stored text stays hidden after unlocking.
    fn usernotif_text(&self, slot: u8, app: &str, req: &NotifRequest) -> (String, String) {
        let previews = self.app_notif(super::AppId::External(slot)).previews;
        let hidden = previews == 2 || (previews == 1 && self.is_locked());
        if !hidden {
            return display(req);
        }
        let options = self.usernotif.category(app, &req.category).map_or(0, |c| c.options);
        hidden_text(options, req)
    }

    /// The thumbnail of the request's first attachment. Only a PNG inside the app's own data directory is read (a link or a
    /// path elsewhere is refused), and a file over 4 MiB is not read at all.
    fn usernotif_thumbnail(&self, app: &str, req: &NotifRequest) -> Option<Image> {
        let path = std::path::Path::new(req.attachments.first()?);
        let base = std::fs::canonicalize(super::apphost::root().join("appdata").join(app)).ok()?;
        let file = std::fs::canonicalize(path).ok()?;
        let png = file.extension().is_some_and(|e| e.eq_ignore_ascii_case("png"));
        if !png || !file.starts_with(&base) || std::fs::metadata(&file).ok()?.len() > MAX_ATTACHMENT_BYTES {
            return None;
        }
        super::spotindex::decode_thumbnail(&std::fs::read(&file).ok()?, ATTACHMENT_PX)
    }

    /// Shows a delivered request. `presented` is the `willPresent` plan of a request that arrived in front; None applies
    /// the grant and mute state alone.
    fn usernotif_show(&mut self, slot: u8, app: &str, req: NotifRequest, replaced: Vec<u64>, t: f64, presented: Option<u8>) {
        for id in replaced {
            self.notify_remove(id);
        }
        let (title, body) = self.usernotif_text(slot, app, &req);
        let mut n = super::notify::Notification::new(super::AppId::External(slot), title, body, self.wall.now(t));
        n.thread = req.thread.clone();
        n.image = self.usernotif_thumbnail(app, &req);
        let (banner, sound, badge_allowed) = match presented {
            None => {
                let (banner, sound) = delivery_plan(self.usernotif.granted(app), self.notify.store().is_muted(super::AppId::External(slot)), req.sound_kind);
                (banner, sound, self.usernotif.granted(app).is_none_or(|bits| bits & NOTIF_AUTH_BADGE != 0))
            }
            Some(plan) => (plan & NOTIF_PRESENT_BANNER != 0, plan & NOTIF_PRESENT_SOUND != 0 && req.sound_kind != 0, plan & NOTIF_PRESENT_BADGE != 0),
        };
        // the app's page in Settings > Notifications has the last word on banners, sounds and badges
        let page = self.app_notif(super::AppId::External(slot));
        let (banner, sound, badge_allowed) = gate_by_page(banner, sound, badge_allowed, &page);
        let id = self.notify_id_banner(n, banner);
        if sound {
            let alert = super::options::ALERT_SOUNDS[self.options().alert_sound.min(super::options::ALERT_SOUNDS.len() - 1)];
            if let Some(name) = request_sound(req.sound_kind, &req.sound_name, alert) {
                super::apphost::queue_request_sound(&name);
            }
        }
        if let Some(count) = req.badge.filter(|_| badge_allowed) {
            if let Some(Some(h)) = self.ext.get_mut(slot as usize) {
                h.badge = if count == 0 { String::new() } else { super::apphost::clean_badge(&count.to_string()) };
                self.dirty = true;
            }
        }
        self.usernotif.record_delivery(app, req, id);
        // the banner was built before the delivery was recorded, so its category's buttons are added now
        let actions = self.usernotif.actions(id);
        self.usernotif_banner_actions(id, &actions);
    }

    /// Delivers the requests whose trigger fired; called every tick.
    pub(super) fn usernotif_tick(&mut self, t: f64) {
        self.usernotif.expire_presentations(t);
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
    pub(super) fn usernotif_prune(&mut self) {
        let gone: Vec<u64> = self.usernotif.store_ids().into_iter().filter(|&id| !self.notify_exists(id)).collect();
        if !gone.is_empty() {
            self.usernotif.prune(|id| !gone.contains(&id));
        }
    }

    /// The user cleared notifications from the notification UI (swipe, group or Clear All). Call `usernotif_prune` before
    /// the change, so only entries this change removed are found here. An app whose category asked for a custom dismissal
    /// hears `NOTIF_DISMISS_ACTION` for each of them, once it is running or when it next starts.
    pub(super) fn usernotif_dismiss_gone(&mut self) {
        let gone: Vec<u64> = self.usernotif.store_ids().into_iter().filter(|&id| !self.notify_exists(id)).collect();
        if gone.is_empty() {
            return;
        }
        for d in self.usernotif.prune(|id| !gone.contains(&id)) {
            let asks = self.usernotif.category(&d.app, &d.req.category).is_some_and(|c| c.options & NOTIF_CATEGORY_CUSTOM_DISMISS != 0);
            if !asks {
                continue;
            }
            let msg = ToApp::NotificationAction {
                id: d.req.id.clone(),
                action: NOTIF_DISMISS_ACTION.to_string(),
                text: String::new(),
                thread: d.req.thread.clone(),
                category: d.req.category.clone(),
                user_info: d.req.user_info.clone(),
            };
            let Some(slot) = super::apphost::slot_of(&d.app) else { continue };
            let running = self.ext.get(slot).and_then(Option::as_ref).is_some_and(|h| h.running());
            if running {
                self.ext_send(slot, &msg);
            } else {
                self.usernotif.queue(&d.app, msg);
            }
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
    fn an_authorization_waits_for_its_own_app_only() {
        let mut r = Registry::default();
        r.hold_authorization(APP, 2, 7, 4);
        r.hold_authorization("is.other", 3, 8, 2);
        assert_eq!(r.take_authorization(APP), vec![(2, 7, 4)]);
        assert!(r.take_authorization(APP).is_empty());
        assert_eq!(r.take_authorization("is.other"), vec![(3, 8, 2)]);
        r.hold_authorization(APP, 2, 9, 0);
        r.forget_app(APP);
        assert!(r.take_authorization(APP).is_empty());
    }

    #[test]
    fn delivery_follows_the_alert_and_sound_options() {
        assert_eq!(delivery_plan(None, false, 1), (true, true));
        assert_eq!(delivery_plan(Some(NOTIF_AUTH_ALERT), false, 1), (true, false));
        assert_eq!(delivery_plan(Some(NOTIF_AUTH_SOUND), false, 1), (false, true));
        assert_eq!(delivery_plan(Some(NOTIF_AUTH_BADGE), false, 2), (false, false));
        assert_eq!(delivery_plan(Some(NOTIF_AUTH_ALERT | NOTIF_AUTH_SOUND), true, 1), (true, false));
        assert_eq!(delivery_plan(Some(NOTIF_AUTH_ALERT | NOTIF_AUTH_SOUND), false, 0), (true, false));
    }

    #[test]
    fn grants_are_kept_per_option_and_survive_a_reload() {
        let dir = std::env::temp_dir().join(format!("usernotif-grants-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("authorization.ini");
        let _ = std::fs::remove_file(&path);
        let mut r = Registry::default();
        r.load_granted(path.clone());
        assert_eq!(r.granted(APP), None);
        r.set_granted(APP, NOTIF_AUTH_ALERT | NOTIF_AUTH_PROVISIONAL);
        r.set_granted("is.other", 0);
        let mut again = Registry::default();
        again.load_granted(path.clone());
        assert_eq!(again.granted(APP), Some(NOTIF_AUTH_ALERT | NOTIF_AUTH_PROVISIONAL));
        assert_eq!(again.granted("is.other"), Some(0));
        again.forget_app(APP);
        assert_eq!(Registry::default().granted(APP), None);
        let mut third = Registry::default();
        third.load_granted(path);
        assert_eq!(third.granted(APP), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

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
                options: 0,
                id: "MSG".into(),
                actions: vec![
                    NotifAction::button("archive", "Archive"),
                    NotifAction { id: "reply".into(), title: "Reply".into(), destructive: false, foreground: false, text_input: Some(TextInput { button_title: "Send".into(), placeholder: "".into() }), icon: String::new() },
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
    fn prune_returns_what_it_forgot_with_its_request() {
        let mut r = Registry::default();
        r.record_delivery(APP, req("a"), 1);
        r.record_delivery(APP, req("b"), 2);
        let gone = r.prune(|id| id == 2);
        assert_eq!(gone.iter().map(|d| (d.req.id.as_str(), d.store_id)).collect::<Vec<_>>(), [("a", 1)]);
        assert_eq!(r.list(APP, true), ["b"]);
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
        assert_eq!(msg, ToApp::NotificationAction { id: "m1".into(), action: "reply".into(), text: "on my way".into(), thread: "room".into(), category: "MSG".into(), user_info: vec![] });
        // the request's userInfo comes back with the response
        r.record_delivery(APP, NotifRequest { category: "MSG".into(), user_info: vec![("room".into(), "7".into())], ..req("m2") }, 6);
        let ToApp::NotificationAction { user_info, .. } = r.respond(6, "archive", "").unwrap().1 else { panic!() };
        assert_eq!(user_info, vec![("room".to_string(), "7".to_string())]);
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
        let m = |n: &str| ToApp::NotificationAction { id: n.into(), action: NOTIF_DEFAULT_ACTION.into(), text: String::new(), thread: String::new(), category: String::new(), user_info: vec![] };
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
    fn hidden_previews_keep_only_what_the_category_asks_for() {
        let r = NotifRequest { title: "T".into(), subtitle: "S".into(), body: "B".into(), ..Default::default() };
        assert_eq!(hidden_text(0, &r), (String::new(), String::new()), "no option: nothing of the content shows");
        assert_eq!(hidden_text(NOTIF_CATEGORY_HIDDEN_SHOW_TITLE, &r), ("T".to_string(), String::new()));
        assert_eq!(hidden_text(NOTIF_CATEGORY_HIDDEN_SHOW_SUBTITLE, &r), (String::new(), "S".to_string()));
        assert_eq!(hidden_text(NOTIF_CATEGORY_HIDDEN_SHOW_TITLE | NOTIF_CATEGORY_HIDDEN_SHOW_SUBTITLE, &r), ("T".to_string(), "S".to_string()));
        assert_eq!(hidden_text(NOTIF_CATEGORY_CUSTOM_DISMISS, &r), (String::new(), String::new()), "the dismiss option does not show text");
    }

    #[test]
    fn presentation_is_cut_to_what_the_grant_and_mute_allow() {
        use bifrost_proto::{NOTIF_PRESENT_BADGE as BADGE, NOTIF_PRESENT_BANNER as BANNER, NOTIF_PRESENT_LIST as LIST, NOTIF_PRESENT_SOUND as SOUND};
        let all = BANNER | LIST | SOUND | BADGE;
        assert_eq!(presentation_plan(all, Some(NOTIF_AUTH_ALERT | NOTIF_AUTH_SOUND | NOTIF_AUTH_BADGE), false), all);
        assert_eq!(presentation_plan(all, Some(NOTIF_AUTH_SOUND | NOTIF_AUTH_BADGE), false), SOUND | BADGE, "no alert grant, no banner or list");
        assert_eq!(presentation_plan(all, Some(0), false), 0, "a denied app shows nothing");
        assert_eq!(presentation_plan(all, Some(NOTIF_AUTH_ALERT), true), 0, "a muted app shows nothing");
        assert_eq!(presentation_plan(0, Some(NOTIF_AUTH_ALERT), false), 0, "an empty answer hides it");
        assert_eq!(presentation_plan(BANNER, None, false), BANNER, "a never-decided app is not cut");
    }

    #[test]
    fn a_held_presentation_is_taken_once_and_expires_unanswered() {
        let mut r = Registry::default();
        assert!(!r.presents("chat"));
        r.set_presenter("chat", true);
        assert!(r.presents("chat"));
        let a = r.hold_presentation("chat", 2, req("a"), vec![5], 1500.0);
        let b = r.hold_presentation("chat", 2, req("b"), vec![], 1800.0);
        assert_ne!(a, b, "each hold gets its own token");
        let got = r.take_presentation(a).expect("the held notification");
        assert_eq!((got.app.as_str(), got.slot, got.req.id.as_str(), got.replaced.clone()), ("chat", 2, "a", vec![5]));
        assert!(r.take_presentation(a).is_none(), "an answer is taken once");
        r.expire_presentations(1900.0);
        assert!(r.take_presentation(b).is_none(), "no answer in time hides it");
        r.set_presenter("chat", false);
        assert!(!r.presents("chat"));
    }

    #[test]
    fn display_stacks_subtitle_over_body() {
        assert_eq!(display(&NotifRequest { title: "T".into(), subtitle: "S".into(), body: "B".into(), ..Default::default() }), ("T".to_string(), "S\nB".to_string()));
        assert_eq!(display(&NotifRequest { title: "T".into(), subtitle: "S".into(), ..Default::default() }).1, "S");
        assert_eq!(display(&NotifRequest { title: "T".into(), body: "B".into(), ..Default::default() }).1, "B");
    }
}
