//! Notifications, bifrost 15 style: apps post a `Notification` through
//! `Shell::notify`; it lands in the persisted store (`notifications.ini`),
//! shows as a banner sliding down from the top for five seconds, sits on
//! the lock screen under the clock (one stack per app, swipe left to clear,
//! tap to open after the unlock), in Notification Center (a swipe down
//! from the left two thirds of the status bar) and as a red badge on the
//! app's home-screen icon. Sources: Clock alarms and timers, the battery
//! at 20 % and 10 % and when charged, Wi-Fi joined or lost, a Deildu
//! download ready to play and the Terminal's shell exiting.
//!
//! Stacks expand on a tap, a swipe left clears a card or stack, Clear All
//! ends every list; Do Not Disturb (by hand or on a daily schedule) and a
//! per-app Mute keep banners and screen wakes away while the notifications
//! stay stored and badged. The lock screen shows a widget strip under the
//! clock (next calendar event, battery); with a passcode set it hides the
//! notification text and a tap on the strip or a card asks for the passcode
//! before opening the app, never producing a sound.

pub mod banner;
pub mod center;
pub mod list;
pub mod lockwidgets;
pub mod modes;
pub mod permission;
pub mod schedule;
pub mod store;

use super::clock::Wall;
use super::notify_page::{pref_key, AppNotif};
use super::ui::Ui;
use super::{AppId, Effect, Event, Info, Screen, Shell, apps, IN_DOCK};
use crate::deildu::DeilduEvent;
use crate::gfx::{Image, Surface};
use crate::layout::{Layout, PER_PAGE};
use banner::Banner;
use center::Center;
use list::{badge_image, badge_pos, listed_in, Card, Hit, ListView, Mode, Opts, Slide, CARD_H_PT, CARD_W_PT, ICON_PT};
use lockwidgets::Strip;
pub use schedule::Schedule;
pub use store::{Notification, Store};

/// The lock screen's list starts here: under the clock and the widget strip
/// (further down when the Now Playing card sits between them).
pub const LOCK_LIST_TOP_PT: f32 = 250.0;
/// between the Now Playing card and the list under it
const MUSIC_GAP_PT: f32 = 12.0;
/// The banner's resting top edge, under the status bar.
const BANNER_TOP_PT: f32 = 28.0;
/// A touch that moves less than this is a tap.
const TAP_PT: f32 = 8.0;
/// A banner pulled up further than this leaves.
const BANNER_FAR_PT: f32 = 30.0;
/// A lock-screen card dragged left further than this fraction of its width clears.
const CLEAR_FRACTION: f32 = 0.4;
/// A card dragged right further than this fraction of its width opens its app.
const OPEN_FRACTION: f32 = 0.2;

/// What a finger on the lock screen landed on.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Aim {
    /// a card; `whole` when it stands for its stacked group
    Card { group: usize, id: u64, app: AppId, whole: bool },
    /// a button of the list
    Button(Hit),
    /// button 0 Options, 1 View, 2 Clear under the card resting swiped open
    Revealed(usize),
}

/// A finger on the lock screen's notifications or widgets.
#[derive(Clone, Copy, Debug)]
struct LockTouch {
    aim: Aim,
    x0: f32,
    y0: f32,
    x: f32,
    /// decided once the finger moved past the slop
    horizontal: Option<bool>,
    /// where the card rested when the finger landed (px, 0 or minus the reveal width)
    base: f32,
}

impl LockTouch {
    /// The card's offset: free to the left, resisting to the right (a swipe right opens).
    fn dx(&self) -> f32 {
        let d = self.base + self.x - self.x0;
        if d < 0.0 {
            d
        } else {
            d * 0.35
        }
    }

    /// The card or stack the finger drags sideways, if it is on one.
    fn slide(&self) -> Option<Slide> {
        match self.aim {
            Aim::Card { group, id, whole, .. } => Some(Slide { group, id: (!whole).then_some(id), dx: self.dx() }),
            _ => None,
        }
    }
}

/// Everything the shell keeps about notifications.
pub struct Notify {
    store: Store,
    wall: Wall,
    /// the last tick's time
    pub(super) t: f64,
    /// bumped whenever the store changes
    gen: u32,
    banner: Option<Banner>,
    banner_card: Option<Card>,
    /// the banner was on the last frame: its strip needs repainting
    banner_drawn: bool,
    lock: Option<ListView>,
    /// the strip above the list
    strip: Option<Strip>,
    /// store generation, minute, private, battery (level * 2 + charging)
    lock_key: (u32, i64, bool, u16),
    /// lock-screen stacks shown expanded
    expanded: Vec<AppId>,
    lock_touch: Option<LockTouch>,
    /// the lock-screen card resting swiped open (an `Aim::Card`)
    revealed: Option<Aim>,
    pub(super) center: Center,
    /// a notification arrived with the screen off: turn it on
    wake: bool,
    /// an app to open once the unlock animation lands
    pending_open: Option<AppId>,
    icons: Vec<(AppId, Image)>,
    badges: Vec<(AppId, usize, Image)>,
    /// badge counts that are not notifications (pending updates), added to `badges`
    pub(in crate::shell) extra: Vec<(AppId, usize)>,
    /// set after the first `set_info`, so the boot state posts nothing
    booted: bool,
    /// the Focus modes (`modes`) and the one on by hand; Do Not Disturb's apps and schedule are read from the store
    pub(in crate::shell) modes: modes::Modes,
    /// where `modes` is saved (`focus_modes.ini`); None for a shell that never loaded its data directory
    modes_path: Option<std::path::PathBuf>,
    /// the scheduled mode whose window covers now
    sched_mode: Option<String>,
    /// Focus was switched off inside the window: it stays off until the window ends
    sched_skip: bool,
    /// first-time permission alerts waiting for their turn: the app and the name the alert shows
    pub(super) ask_queue: Vec<(AppId, String)>,
    /// the first-time permission alert on screen (`permission.rs`)
    pub(super) ask: Option<permission::Sheet>,
}

impl Default for Notify {
    fn default() -> Self {
        Notify {
            store: Store::memory(),
            wall: Wall::default(),
            t: 0.0,
            gen: 0,
            banner: None,
            banner_card: None,
            banner_drawn: false,
            lock: None,
            strip: None,
            lock_key: (u32::MAX, 0, false, 0),
            expanded: Vec::new(),
            lock_touch: None,
            revealed: None,
            center: Center::default(),
            wake: false,
            pending_open: None,
            icons: Vec::new(),
            badges: Vec::new(),
            extra: Vec::new(),
            booted: false,
            modes: modes::Modes::builtin(),
            modes_path: None,
            sched_mode: None,
            sched_skip: false,
            ask_queue: Vec::new(),
            ask: None,
        }
    }
}

impl Notify {
    /// Focus affects delivery only; notification history and badges are kept. Do Not Disturb on or off.
    pub fn set_dnd(&mut self, on: bool) {
        self.set_mode(on.then_some(modes::DND_ID));
    }

    /// Switches the Focus mode `id` on by hand, or Focus off (None). Returns false for a mode that does not exist.
    pub fn set_mode(&mut self, id: Option<&str>) -> bool {
        if id.is_none() {
            // switching Focus off inside a schedule's window holds until the window ends
            self.end_scheduled();
        }
        if self.modes.manual.as_deref() == id {
            return true;
        }
        if !self.modes.set_manual(id) {
            return false;
        }
        if id.is_some() {
            self.sched_skip = false;
            self.silence();
        }
        true
    }

    /// Focus is on: a mode switched on by hand, or a schedule's window is open
    /// and nobody switched Focus off since it opened.
    pub fn dnd(&self) -> bool {
        self.active_mode().is_some()
    }

    /// The Focus mode on now, if any. A manual choice wins over the schedule.
    pub fn active_mode(&self) -> Option<modes::FocusMode> {
        if let Some(mode) = self.modes.manual_mode() {
            return Some(self.as_stored(mode));
        }
        if self.sched_skip {
            return None;
        }
        let id = self.sched_mode.as_deref()?;
        self.modes.get(id).map(|mode| self.as_stored(mode))
    }

    /// A mode as it applies now. Do Not Disturb's allowed apps and schedule live in the notification store
    /// (`notifications.ini`); every other mode keeps its own in `focus_modes.ini`.
    fn as_stored(&self, mode: &modes::FocusMode) -> modes::FocusMode {
        let mut m = mode.clone();
        if m.id == modes::DND_ID {
            m.apps = apps().iter().filter(|a| self.store.is_focus_allowed(a.id)).map(|a| store::app_key(a.id).to_string()).collect();
            let s = self.store.schedule();
            m.schedule = s.enabled.then_some(modes::ModeSchedule { days: modes::ALL_DAYS, start: s.start, end: s.end });
        }
        m
    }

    /// The user turned Focus off while a schedule had it on: it stays off
    /// until the window ends.
    pub fn end_scheduled(&mut self) {
        if self.sched_mode.is_some() {
            self.sched_skip = true;
        }
    }

    /// Whether a notification of `app` may show a banner or wake the screen: not muted, and the Focus mode on now
    /// lets it through.
    pub fn delivers(&self, app: AppId) -> bool {
        !self.store.is_muted(app) && self.shows(self.active_mode().as_ref(), app)
    }

    /// Whether a notification of `app` shows under `mode` (no mode shows everything).
    fn shows(&self, mode: Option<&modes::FocusMode>, app: AppId) -> bool {
        modes::decide(mode, store::app_key(app), "", false) == modes::Delivery::Show
    }

    /// Saves the Focus modes to their file, when the shell has a data directory.
    pub fn save_modes(&self) {
        if let Some(path) = &self.modes_path {
            if let Err(e) = self.modes.save(path) {
                eprintln!("springboard: focus_modes.ini: {e}");
            }
        }
    }

    /// Reads the Focus modes saved next to the notification store.
    pub(in crate::shell) fn load_modes(&mut self, notifications: &str) {
        let path = std::path::Path::new(notifications).with_file_name("focus_modes.ini");
        self.modes = modes::Modes::load(&path);
        self.modes_path = Some(path);
    }

    /// The person answered the first-time alert for `app`: the answer is stored (the app is not asked again) and the
    /// Allow Notifications switch follows it.
    pub fn answer_permission(&mut self, app: AppId, allowed: bool) {
        permission::apply_answer(&mut self.store, app, allowed);
        self.gen = self.gen.wrapping_add(1);
    }

    /// Drops what Focus silences: the banner and a pending wake. A banner of an app allowed through Focus stays.
    fn silence(&mut self) {
        let mode = self.active_mode();
        if self.banner_card.as_ref().is_some_and(|c| self.shows(mode.as_ref(), c.app)) {
            return;
        }
        self.banner = None;
        self.banner_card = None;
        self.wake = false;
    }

    /// Re-reads the schedule against the clock; true when the window opened or closed.
    fn refresh_schedule(&mut self) -> bool {
        let (unix, offset) = (self.now(), self.wall.offset);
        let (day, minute) = (modes::weekday_of(unix, offset), Schedule::minute_of_day(unix, offset));
        let on = self.modes.list.iter().map(|m| self.as_stored(m)).find(|m| m.schedule.as_ref().is_some_and(|s| s.covers(day, minute))).map(|m| m.id);
        if on == self.sched_mode {
            return false;
        }
        self.sched_mode = on;
        if self.sched_mode.is_none() {
            self.sched_skip = false;
        } else if self.dnd() {
            self.silence();
        }
        true
    }

    pub fn set_wall(&mut self, unix: i64, offset: i32, t: f64) {
        self.wall = Wall { unix, offset, at: t };
    }

    /// Unix seconds as of the last tick.
    pub fn now(&self) -> i64 {
        self.wall.now(self.t)
    }

    pub fn store(&self) -> &Store {
        &self.store
    }

    /// The banner on screen, if any.
    pub fn banner(&self) -> Option<&Banner> {
        self.banner.as_ref()
    }

    /// Every frame matters: the banner sliding or the sheet settling.
    pub fn animating(&self) -> bool {
        self.banner.as_ref().is_some_and(Banner::animating) || self.center.animating()
    }

    /// When a tick is next needed without input: the banner's rest ending,
    /// or at once when the screen must wake for a notification.
    pub fn next_wake(&self, t: f64) -> Option<f64> {
        if self.wake {
            return Some(t);
        }
        self.banner.as_ref().and_then(Banner::next_wake)
    }

    /// The banner is on screen or just left: the frame must repaint under it.
    fn banner_damage(&self) -> bool {
        self.banner.is_some() || self.banner_drawn
    }

    /// What the lock screen's list looks like: changes when the store, the
    /// minute or a card drag changes.
    pub fn lock_state(&self) -> u32 {
        let drag = self.lock_touch.and_then(|tc| tc.slide()).map_or(0, |sl| sl.dx as i32) ^ i32::from(self.revealed.is_some()) << 20;
        let (gen, minute, private, battery) = self.lock_key;
        gen.wrapping_mul(0x9e37_79b9) ^ (minute as u32).wrapping_mul(0x85eb_ca6b) ^ (drag as u32) ^ u32::from(private).wrapping_mul(0x27d4_eb2f) ^ u32::from(battery).wrapping_mul(0x1656_67b1)
    }

    /// Generation of the badge counts.
    pub fn badge_gen(&self) -> u32 {
        self.gen
    }

    /// Drops the overlays (screen off, lock); the store stays.
    fn sleep(&mut self) {
        self.center.hide();
        self.banner = None;
        self.banner_card = None;
        self.lock_touch = None;
    }

    pub fn describe(&self) -> String {
        let banner = match &self.banner {
            Some(b) => format!("{:?}", b.phase()).to_lowercase(),
            None => "none".into(),
        };
        format!("notifications={}\nbanner={banner}\ncenter={}", self.store.len(), self.center.describe())
    }
}

/// The lock screen's notifications, borrowed for the row workers.
pub struct LockDraw<'a> {
    list: Option<&'a ListView>,
    strip: Option<&'a Strip>,
    drag: Option<Slide>,
    /// top of the list in px
    top: f32,
}

impl LockDraw<'_> {
    /// Draws the widget strip and the stacks under the clock, lifted with the lock screen by `dy`.
    pub fn draw(&self, s: &mut Surface, ui: &Ui, w: i32, dy: i32) {
        if let Some(strip) = self.strip {
            strip.draw(s, ui, w, dy);
        }
        if let Some(l) = self.list {
            l.draw(s, ui, w, self.top + dy as f32, self.drag, 255);
        }
    }
}

/// The home screen's badges, borrowed for the row workers.
pub struct BadgeView<'a> {
    at: Vec<(i32, i32, &'a Image)>,
}

impl BadgeView<'_> {
    pub fn is_empty(&self) -> bool {
        self.at.is_empty()
    }

    pub fn draw(&self, s: &mut Surface) {
        for &(x, y, img) in &self.at {
            s.blit(img, x, y, 255, 255);
        }
    }
}

/// How a notification of an app with the page settings `app` is presented, from the shared delivery model.
/// `delivers` is `Notify::delivers` (Focus and mute already folded in): when it is false the notification is only stored.
/// Posts are ordinary interruptions (`Level::Active`), so the app's own banner or alert style decides the rest.
fn presentation_of(app: &AppNotif, delivers: bool) -> bifrost_ui::notifdelivery::Decision {
    use bifrost_ui::notifdelivery::{decide, AppPolicy, Context, Level};
    let policy = AppPolicy { banners: delivers && app.banners, alerts: app.persistent, sounds: app.sounds, time_sensitive: false };
    decide(Level::Active, policy, Context { focus_on: false, focus_allows_app: false })
}

impl Shell {
    /// Posts a notification: it is always stored, persisted and badged.
    /// Unless Do Not Disturb is enabled, it shows a banner when unlocked
    /// and wakes a dark screen onto the lock screen.
    pub fn notify(&mut self, n: Notification) {
        self.notify_id(n);
    }

    /// `notify`, returning the id the store gave the notification.
    pub fn notify_id(&mut self, n: Notification) -> u64 {
        self.notify_id_banner(n, true)
    }

    /// Like `notify_id`; with `banner` false the notification is only stored and listed (an app whose alert option
    /// was not granted), so no banner or wake is shown for it.
    pub fn notify_id_banner(&mut self, n: Notification, banner: bool) -> u64 {
        let app = n.app;
        let id = self.notify.store.push(n);
        self.screentime.note_notification(app);
        self.store_changed();
        self.notify.refresh_schedule();
        if !banner {
            return id;
        }
        // the delivery model decides: Stored (Alert Style None, muted, or Focus) is listed and badged only
        let decision = presentation_of(&self.app_notif(app), self.notify.delivers(app));
        if decision.presentation == bifrost_ui::notifdelivery::Presentation::Stored {
            return id;
        }
        if !self.screen_on {
            self.notify.wake = true;
            return id;
        }
        let covered = self.screen == Screen::Lock || self.cc_active() || self.center_active() || self.clock.alert().is_some();
        if !covered {
            self.start_banner(id, app);
        }
        id
    }

    /// Removes notification `id` from the store (an app withdrew it) and refreshes what shows it.
    pub(in crate::shell) fn notify_remove(&mut self, id: u64) {
        if self.notify.store.remove(id) {
            self.store_changed();
        }
    }

    pub(in crate::shell) fn notify_exists(&self, id: u64) -> bool {
        self.notify.store.get(id).is_some()
    }

    fn start_banner(&mut self, id: u64, app: AppId) {
        let now = self.notify.now();
        let Some(n) = self.notify.store.get(id).cloned() else { return };
        let icon = self.notification_icon(app);
        let page = self.app_notif(app);
        // a banner shows once unlocked, so only Never hides its text
        self.notify.banner_card = Some(Card::new(&self.ui, &n, &icon, now, page.previews == 2));
        let alerts = page.persistent;
        self.notify.banner = Some(Banner::new(id, self.notify.t).persistent(alerts));
    }

    /// The app's options from its page of Settings > Notifications (the defaults when it has none).
    pub(super) fn app_notif(&self, app: AppId) -> AppNotif {
        self.prefs.get(&pref_key(app)).map_or_else(AppNotif::default, AppNotif::decode)
    }

    /// The app's icon at the card size, rendered once per app.
    fn notification_icon(&mut self, app: AppId) -> Image {
        if let Some((_, img)) = self.notify.icons.iter().find(|(a, _)| *a == app) {
            return img.clone();
        }
        let i = apps().iter().position(|a| a.id == app).unwrap_or(0);
        let src = if i < IN_DOCK { &self.dock[i].icon } else { &self.grid[i - IN_DOCK].icon };
        let d = self.ui.pt(ICON_PT).round() as usize;
        let img = src.resized(d, d);
        self.notify.icons.push((app, img.clone()));
        img
    }

    /// Drops an app's notifications (it was opened, or its group cleared).
    pub(super) fn clear_notifications(&mut self, app: AppId) {
        if self.notify.store.clear_app(app) {
            self.store_changed();
        }
        if self.notify.banner_card.as_ref().is_some_and(|c| c.app == app) {
            if let Some(b) = &mut self.notify.banner {
                b.dismiss(self.notify.t);
            }
        }
    }

    /// The thread of lock-screen group `group` (empty when there is none).
    fn lock_thread(&self, group: usize) -> String {
        self.notify.lock.as_ref().and_then(|l| l.groups.get(group)).map(|g| g.thread.clone()).unwrap_or_default()
    }

    /// Clears one stack of an app: the notifications of `app` in `thread` (the Clear button of a group).
    pub(super) fn clear_group(&mut self, app: AppId, thread: &str) {
        self.usernotif_prune();
        self.clear_group_entries(app, thread);
        self.usernotif_dismiss_gone();
    }

    fn clear_group_entries(&mut self, app: AppId, thread: &str) {
        if self.notify.store.clear_thread(app, thread) {
            self.store_changed();
        }
        if self.notify.banner_card.as_ref().is_some_and(|c| c.app == app) {
            if let Some(b) = &mut self.notify.banner {
                b.dismiss(self.notify.t);
            }
        }
    }

    /// Drops one notification (a swipe on a card of an expanded stack).
    fn clear_notification(&mut self, id: u64) {
        self.usernotif_prune();
        self.clear_notification_entry(id);
        self.usernotif_dismiss_gone();
    }

    fn clear_notification_entry(&mut self, id: u64) {
        let banner = self.notify.banner.as_ref().is_some_and(|b| b.id == id);
        if self.notify.store.remove(id) {
            self.store_changed();
        }
        if banner {
            if let Some(b) = &mut self.notify.banner {
                b.dismiss(self.notify.t);
            }
        }
    }

    /// Drops every notification (Clear All).
    pub(super) fn clear_all_notifications(&mut self) {
        self.usernotif_prune();
        self.clear_all_entries();
        self.usernotif_dismiss_gone();
    }

    fn clear_all_entries(&mut self) {
        self.notify.store.clear();
        self.store_changed();
        if let Some(b) = &mut self.notify.banner {
            b.dismiss(self.notify.t);
        }
    }

    /// Mutes `app`, or unmutes it: its notifications are still stored and
    /// badged but no longer show a banner or wake the screen.
    pub fn toggle_mute(&mut self, app: AppId) {
        let mute = !self.notify.store.is_muted(app);
        self.notify.store.set_muted(app, mute);
        if mute && self.notify.banner_card.as_ref().is_some_and(|c| c.app == app) {
            if let Some(b) = &mut self.notify.banner {
                b.dismiss(self.notify.t);
            }
        }
        self.store_changed();
    }

    pub fn app_muted(&self, app: AppId) -> bool {
        self.notify.store.is_muted(app)
    }

    /// Lets `app` show its notifications while Focus is on, or holds it back (kept in the notification store).
    pub fn set_focus_allowed(&mut self, app: AppId, allowed: bool) {
        self.notify.store.set_focus_allowed(app, allowed);
        self.dirty = true;
    }

    /// Replaces the Do Not Disturb schedule (kept in the notification store).
    pub fn set_dnd_schedule(&mut self, schedule: Schedule) {
        self.notify.store.set_schedule(schedule);
        self.notify.refresh_schedule();
        self.dirty = true;
    }

    pub fn dnd_schedule(&self) -> Schedule {
        self.notify.store.schedule()
    }

    /// Opens `app` from a banner, a lock-screen card or Notification Center:
    /// whatever is on screen steps aside first.
    pub(super) fn open_from_notification(&mut self, app: AppId, t: f64) {
        self.usernotif_tapped(app);
        let link = app == AppId::Settings && self.notify.store.groups().iter().find(|g| g.app == app).is_some_and(|g| g.latest().title == "Software Update");
        self.open_from_notification_inner(app, t);
        if link && self.screen != Screen::Lock {
            let ui = &self.ui;
            if let Some(st) = self.settings.as_mut() {
                st.open_page(ui, "Software Update", t);
            }
        }
    }

    fn open_from_notification_inner(&mut self, app: AppId, t: f64) {
        match self.screen {
            Screen::Lock => {
                self.notify.pending_open = Some(app);
                self.lock_drag = None;
                self.begin_unlock(0.0, t);
                return;
            }
            Screen::Switcher => {
                self.switcher_sleep();
                self.screen = Screen::Home;
            }
            Screen::App(cur) => {
                if cur == app {
                    self.clear_notifications(app);
                    return;
                }
                let frame = self.render_app_frame(cur);
                self.store_card(cur, &frame);
                self.screen = Screen::Home;
            }
            Screen::Home => {}
        }
        self.open(app, t);
    }

    /// Whether the lock screen hides what notifications and events say: a passcode is set.
    /// Whether the app's Show Previews setting hides its text in a list of `mode`: Never hides it everywhere,
    /// When Unlocked hides it on the lock screen and shows it once unlocked (`notify_page::PREVIEWS`).
    fn previews_hidden(&self, app: AppId, mode: Mode) -> bool {
        match self.app_notif(app).previews {
            2 => true,
            1 => mode == Mode::Lock,
            _ => false,
        }
    }

    fn lock_private(&self) -> bool {
        self.screen == Screen::Lock && super::passcode::Lock::required(&self.passcode_path())
    }

    /// The groups a list of `mode` shows: the apps whose page lets them into it.
    pub(super) fn visible_groups(&self, mode: Mode) -> Vec<store::Group> {
        self.notify.store.groups().into_iter().filter(|g| listed_in(&self.app_notif(g.app), mode)).collect()
    }

    fn render_list(&mut self, mode: Mode) -> ListView {
        let private = mode == Mode::Lock && self.lock_private();
        let groups = self.visible_groups(mode);
        for g in &groups {
            self.notification_icon(g.app);
        }
        let icons = &self.notify.icons;
        let icon = |app: AppId| icons.iter().find(|(a, _)| *a == app).map(|(_, i)| i.clone()).unwrap_or_else(|| Image::new(1, 1));
        let muted: Vec<AppId> = apps().iter().map(|a| a.id).filter(|&a| self.notify.store.is_muted(a)).collect();
        let hidden: Vec<AppId> = apps().iter().map(|a| a.id).filter(|&a| self.previews_hidden(a, mode)).collect();
        let opts = Opts { expanded: &self.notify.expanded, muted: &muted, hidden: &hidden, private };
        ListView::new(&self.ui, mode, &groups, &icon, self.notify.now(), opts)
    }

    /// Where the lock screen's list starts, in px: under the widget strip,
    /// and under the Now Playing card while one shows.
    fn lock_list_top(&self) -> f32 {
        let playing = self.svc_active();
        let pt = if playing { super::music::nowplaying::LOCK_CARD_Y_PT + super::music::nowplaying::LOCK_CARD_H_PT + MUSIC_GAP_PT } else { LOCK_LIST_TOP_PT };
        self.ui.pt(pt)
    }

    /// Where the pending requests of `notification-schedule` are saved: beside the notification store, once the
    /// data directory is set (None before that, so a test shell writes nothing).
    pub(in crate::shell) fn pending_path(&self) -> Option<std::path::PathBuf> {
        self.notify.store.path().and_then(|p| p.parent()).map(|dir| dir.join("pending-notifications.ini"))
    }

    /// Loads the persisted notifications from `path` and saves there from now on.
    pub(super) fn notify_load(&mut self, path: &str) {
        self.notify.store = Store::load(path);
        self.notify.load_modes(path);
        self.usernotif.load_granted(std::path::Path::new(path).with_file_name("authorization.ini"));
        self.store_changed();
    }

    /// The store changed: new generation, fresh badges, a frame.
    pub(super) fn store_changed(&mut self) {
        self.notify.gen += 1;
        self.notify.revealed = None;
        self.dirty = true;
        // an app whose page turns Badges off gets none
        let badged: Vec<AppId> = apps().iter().map(|a| a.id).filter(|&a| self.app_notif(a).badges).collect();
        let store = &self.notify.store;
        self.notify.expanded.retain(|&a| store.count(a) > 1);
        let counts: Vec<(AppId, usize)> = apps().iter().map(|a| (a.id, self.notify.store.count(a.id) + self.notify.extra.iter().filter(|e| e.0 == a.id).map(|e| e.1).sum::<usize>())).filter(|&(_, n)| n > 0).filter(|(a, _)| badged.contains(a)).collect();
        let same = self.notify.badges.len() == counts.len() && self.notify.badges.iter().zip(&counts).all(|((a, n, _), (b, m))| a == b && n == m);
        if !same {
            self.notify.badges = counts.into_iter().map(|(app, n)| (app, n, badge_image(&self.ui, n))).collect();
        }
    }

    /// Rebuilds the lock screen's list when the store or the minute
    /// changed; called at the top of every frame.
    pub(super) fn notify_prepare(&mut self) {
        if self.screen != Screen::Lock {
            return;
        }
        let private = self.lock_private();
        let battery = u16::from(self.info.battery) * 2 + u16::from(self.info.charging);
        let key = (self.notify.gen, self.notify.now() / 60, private, battery);
        if self.notify.lock_key != key {
            let list = self.render_list(Mode::Lock);
            self.notify.lock = Some(list);
            self.notify.lock_key = key;
        }
    }

    /// Where each badge lands on the home screen as it is now.
    pub(super) fn badge_view(&self) -> BadgeView<'_> {
        let l: &Layout = &self.layout;
        let offset = self.pager.offset().round() as i32;
        let at = self
            .notify
            .badges
            .iter()
            .filter_map(|(app, _, img)| {
                let i = apps().iter().position(|a| a.id == *app)?;
                let (x, y, w) = if i < IN_DOCK {
                    let r = l.dock_icons[i];
                    (r.x, r.y, r.w)
                } else {
                    let g = i - IN_DOCK;
                    let r = l.cells[g % PER_PAGE];
                    (r.x + (g / PER_PAGE) as i32 * l.w - offset, r.y, r.w)
                };
                let (bx, by) = badge_pos(&self.ui, x, y, w, img);
                Some((bx, by, img))
            })
            .collect();
        BadgeView { at }
    }

    /// The card the finger drags, or the one resting swiped open.
    fn lock_slide(&self) -> Option<Slide> {
        if let Some(sl) = self.notify.lock_touch.and_then(|tc| tc.slide()) {
            return Some(sl);
        }
        match self.notify.revealed {
            Some(Aim::Card { group, id, whole, .. }) => Some(Slide { group, id: (!whole).then_some(id), dx: -ListView::reveal_w(&self.ui) }),
            _ => None,
        }
    }

    pub(super) fn lock_draw(&self) -> LockDraw<'_> {
        LockDraw { list: self.notify.lock.as_ref(), strip: self.notify.strip.as_ref(), drag: self.lock_slide(), top: self.lock_list_top() }
    }

    /// Forgets the app a lock-screen card wanted to open (the passcode prompt was cancelled).
    pub(super) fn notify_cancel_open(&mut self) {
        self.notify.pending_open = None;
    }

    /// Drops the overlays when the screen goes dark.
    pub(super) fn notify_sleep(&mut self) {
        self.notify.sleep();
    }

    /// Battery and Wi-Fi transitions between the shown `Info` and `next`.
    pub(super) fn notify_info(&mut self, next: &Info) {
        if !std::mem::replace(&mut self.notify.booted, true) {
            return;
        }
        let now = self.notify.now();
        let prev = &self.info;
        let low = |b: u8| if b <= 10 { 2 } else if b <= 20 { 1 } else { 0 };
        let mut posts = Vec::new();
        if !next.charging && low(next.battery) > low(prev.battery) {
            posts.push(Notification::new(AppId::Settings, "Low Battery", format!("{}% battery remaining.", next.battery), now));
        }
        let full = |i: &Info| i.charging && i.battery >= 100;
        if full(next) && !full(prev) {
            posts.push(Notification::new(AppId::Settings, "Battery Charged", "Your battery is fully charged.", now));
        }
        if next.online != prev.online {
            let ssid = |i: &Info| i.current_ssid.clone().unwrap_or_else(|| i.wifi.clone());
            let body = if next.online { format!("Joined {}", ssid(next)) } else { format!("Disconnected from {}", ssid(prev)) };
            posts.push(Notification::new(AppId::Settings, "Wi-Fi", body, now));
        }
        for n in posts {
            self.notify(n);
        }
    }

    /// The Clock's alert just fired: note it under the Clock.
    pub(super) fn notify_alarm(&mut self) {
        // filed under the installed Clock app by its manifest id, so it lands whether or not the app is open
        let app = super::apphost::slot_of(super::clock::MANIFEST_ID).map(|n| AppId::External(n as u8));
        for a in self.clock.drain_fired() {
            let Some(app) = app else { continue };
            let now = self.notify.now();
            let body = if a.is_timer { "Your timer is done.".to_string() } else { a.time.clone() };
            self.notify(Notification::new(app, a.title, body, now));
        }
    }

    /// The Terminal's shell exited.
    pub(super) fn notify_term_exit(&mut self) {
        let now = self.notify.now();
        self.notify(Notification::new(AppId::Terminal, "Term", "The shell exited.", now));
    }

    /// Advances the banner and the sheet, wakes the screen for a new
    /// notification and opens the app a lock-screen tap asked for.
    pub(super) fn notify_tick(&mut self, t: f64) -> Vec<Effect> {
        self.notify.t = t;
        if self.notify.refresh_schedule() {
            self.dirty = true;
        }
        let mut effects = Vec::new();
        if std::mem::take(&mut self.notify.wake) && !self.screen_on {
            self.screen_on = true;
            self.last_input = t;
            self.dirty = true;
            effects.push(Effect::ScreenOn);
        }
        let events = self.deildu.as_mut().map(|d| d.take_events()).unwrap_or_default();
        for e in events {
            let now = self.notify.now();
            match e {
                DeilduEvent::Ready { title } => self.notify(Notification::new(AppId::Deildu, "Ready to play", title, now)),
            }
        }
        if let Some(b) = &mut self.notify.banner {
            if b.tick(t) {
                self.dirty = true;
            }
            if b.gone() {
                self.notify.banner = None;
                self.notify.banner_card = None;
            }
        }
        self.center_tick(t);
        if self.screen == Screen::Home && self.anim.is_none() {
            if let Some(app) = self.notify.pending_open.take() {
                self.open(app, t);
            }
        }
        effects
    }

    /// The banner's box at `t`: (x, y, w, h) in px.
    fn banner_rect(&self, t: f64) -> Option<(f32, f32, f32, f32)> {
        let b = self.notify.banner.as_ref()?;
        let ui = &self.ui;
        let (w, h) = (ui.pt(CARD_W_PT), ui.pt(CARD_H_PT));
        let rest = ui.pt(BANNER_TOP_PT);
        let y = rest - (1.0 - b.shown(t)) * (rest + h) - b.pull();
        Some((((self.w as f32 - w) / 2.0).round(), y, w, h))
    }

    /// Touches on the banner and the sheet; None leaves them to the screen.
    pub(super) fn notify_event(&mut self, ev: Event, t: f64) -> Option<Vec<Effect>> {
        if let Some(fx) = self.banner_event(ev, t) {
            return Some(fx);
        }
        self.center_event(ev, t)
    }

    fn banner_event(&mut self, ev: Event, t: f64) -> Option<Vec<Effect>> {
        let pulling = self.notify.banner.as_ref().is_some_and(Banner::pulling);
        match ev {
            Event::Down(x, y) if !pulling => {
                let (bx, by, bw, bh) = self.banner_rect(t)?;
                let b = self.notify.banner.as_mut()?;
                if b.animating() || x < bx || x >= bx + bw || y < by || y >= by + bh {
                    return None;
                }
                b.down(y, t);
                Some(vec![])
            }
            Event::Move(_, y) if pulling => {
                if let Some(b) = &mut self.notify.banner {
                    b.moved(y, t);
                }
                self.dirty = true;
                Some(vec![])
            }
            Event::Up(_, y) if pulling => {
                let (slop, far) = (self.ui.pt(TAP_PT), self.ui.pt(BANNER_FAR_PT));
                let b = self.notify.banner.as_mut()?;
                let tap = b.up(y, t, slop, far);
                self.dirty = true;
                if tap {
                    b.dismiss(t);
                    if let Some(app) = self.notify.banner_card.as_ref().map(|c| c.app) {
                        self.open_from_notification(app, t);
                    }
                }
                Some(vec![])
            }
            _ => None,
        }
    }

    /// Touches on the lock screen's stacks, buttons and widget strip. A tap
    /// on a stack of several expands it; on a single card or on the strip it
    /// opens the app after the unlock (the passcode prompt first when one is
    /// set); a drag left clears the card or stack; Clear, Mute, Show Less
    /// and Clear All act at once (they need no unlock); a vertical drag
    /// unlocks as usual and a sideways one that began on a button or the
    /// strip slides the Today View in.
    pub(super) fn lock_notes_event(&mut self, ev: Event, t: f64) -> bool {
        let slop = self.ui.pt(TAP_PT);
        match (ev, self.notify.lock_touch) {
            (Event::Down(x, y), None) => {
                let top = self.lock_list_top();
                let hit = self.notify.lock.as_ref().and_then(|l| l.hit(&self.ui, self.w, top, x, y));
                let mut base = 0.0;
                if let Some(Aim::Card { group, id, whole, .. }) = self.notify.revealed {
                    let slot = self.notify.lock.as_ref().and_then(|l| l.reveal_hit(&self.ui, self.w, top, group, (!whole).then_some(id), x, y));
                    if let Some(slot) = slot {
                        self.notify.lock_touch = Some(LockTouch { aim: Aim::Revealed(slot), x0: x, y0: y, x, horizontal: None, base: 0.0 });
                        return true;
                    }
                    if matches!(hit, Some(Hit::Card { id: hid, .. }) if hid == id) {
                        base = -ListView::reveal_w(&self.ui);
                    } else {
                        // a touch elsewhere closes it and then does its usual thing
                        self.notify.revealed = None;
                        self.dirty = true;
                    }
                }
                let aim = match hit {
                    Some(Hit::Card { group, id, app }) => {
                        let whole = self.notify.lock.as_ref().is_some_and(|l| !l.groups[group].expanded);
                        Aim::Card { group, id, app, whole }
                    }
                    Some(button) => Aim::Button(button),
                    None => return false,
                };
                self.notify.lock_touch = Some(LockTouch { aim, x0: x, y0: y, x, horizontal: None, base });
                true
            }
            (Event::Move(x, y), Some(mut tc)) => {
                tc.x = x;
                if tc.horizontal.is_none() && ((x - tc.x0).abs() > slop || (y - tc.y0).abs() > slop) {
                    tc.horizontal = Some((x - tc.x0).abs() >= (y - tc.y0).abs());
                }
                if tc.horizontal == Some(false) {
                    // hand the finger to the unlock drag (vertical axis decided)
                    self.notify.lock_touch = None;
                    self.lock_drag = Some((tc.y0, (tc.y0 - y).max(0.0)));
                    self.lock_x0 = tc.x0;
                    self.lock_axis = Some(false);
                    return true;
                }
                if tc.horizontal == Some(true) && !matches!(tc.aim, Aim::Card { .. }) {
                    // a button or the strip has no sideways meaning: pager takes over
                    self.notify.lock_touch = None;
                    self.lock_drag = Some((tc.y0, 0.0));
                    self.lock_x0 = tc.x0;
                    self.lock_axis = Some(true);
                    self.lock_pager.down(tc.x0, tc.y0, t, None);
                    self.lock_pager.moved(x, y, t);
                    return true;
                }
                self.notify.lock_touch = Some(tc);
                self.dirty = true;
                true
            }
            (Event::Up(x, _), Some(mut tc)) => {
                tc.x = x;
                self.notify.lock_touch = None;
                self.dirty = true;
                let (cw, rw) = (self.ui.pt(CARD_W_PT), ListView::reveal_w(&self.ui));
                let total = tc.base + tc.x - tc.x0;
                match (tc.horizontal, tc.aim) {
                    (None, Aim::Card { .. }) if tc.base < 0.0 => self.notify.revealed = None,
                    (None, aim) => self.lock_tap(aim, t),
                    (Some(_), Aim::Card { id, app, whole, group, .. }) => {
                        if -total > cw * CLEAR_FRACTION {
                            if whole {
                                let thread = self.lock_thread(group);
                                self.clear_group(app, &thread);
                            } else {
                                self.clear_notification(id);
                            }
                        } else if -total > rw * 0.5 {
                            self.notify.revealed = Some(tc.aim);
                        } else if total > cw * OPEN_FRACTION {
                            self.notify.revealed = None;
                            self.open_from_notification(app, t);
                        } else {
                            self.notify.revealed = None;
                        }
                    }
                    _ => {}
                }
                true
            }
            _ => false,
        }
    }

    /// A tap that stayed put on `aim`.
    fn lock_tap(&mut self, aim: Aim, t: f64) {
        let group_app = |sh: &Self, i: usize| sh.notify.lock.as_ref().and_then(|l| l.groups.get(i)).map(|g| g.app);
        let group_stack = |sh: &Self, i: usize| sh.notify.lock.as_ref().and_then(|l| l.groups.get(i)).map(|g| (g.app, g.thread.clone()));
        match aim {
            Aim::Card { group, app, .. } => {
                let stacked = self.notify.lock.as_ref().and_then(|l| l.groups.get(group)).is_some_and(|g| g.stacked());
                if stacked {
                    self.notify.expanded.push(app);
                    self.store_changed();
                } else {
                    self.open_from_notification(app, t);
                }
            }
            Aim::Button(Hit::Clear(i)) => {
                if let Some((app, thread)) = group_stack(self, i) {
                    self.clear_group(app, &thread);
                }
            }
            Aim::Button(Hit::Mute(i)) => {
                if let Some(app) = group_app(self, i) {
                    self.toggle_mute(app);
                }
            }
            Aim::Button(Hit::Collapse(i)) => {
                if let Some(app) = group_app(self, i) {
                    self.notify.expanded.retain(|&a| a != app);
                    self.store_changed();
                }
            }
            Aim::Button(Hit::ClearAll) => self.clear_all_notifications(),
            Aim::Button(Hit::Card { .. }) => {}
            Aim::Revealed(slot) => {
                if let Some(Aim::Card { id, app, whole, group, .. }) = self.notify.revealed.take() {
                    self.dirty = true;
                    match slot {
                        0 => self.toggle_mute(app),
                        1 => self.open_from_notification(app, t),
                        _ if whole => {
                            let thread = self.lock_thread(group);
                            self.clear_group(app, &thread);
                        }
                        _ => self.clear_notification(id),
                    }
                }
            }
        }
    }

    /// Paints the banner over the frame and remembers that it did.
    pub(super) fn draw_banner(&mut self, s: &mut Surface, t: f64) {
        let rect = self.banner_rect(t);
        self.notify.banner_drawn = rect.is_some();
        let (Some((x, y, _, _)), Some(card)) = (rect, &self.notify.banner_card) else { return };
        card.draw(s, &self.ui, x, y, 255);
    }

    /// Whether this frame must repaint under the banner.
    pub(super) fn notify_damage_all(&self) -> bool {
        self.notify.banner_damage()
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::{settle, shell, swipe, unlock};
    use super::super::{Anim, Network};
    use super::banner::{Phase, HOLD_MS, SLIDE_MS};
    use super::*;
    use crate::gfx::Surface;

    #[test]
    fn a_page_switch_leaves_the_app_out_of_that_list_only() {
        let lock_off = AppNotif { lock_screen: false, ..AppNotif::default() };
        assert!(!listed_in(&lock_off, Mode::Lock) && listed_in(&lock_off, Mode::Center));
        let center_off = AppNotif { center: false, ..AppNotif::default() };
        assert!(listed_in(&center_off, Mode::Lock) && !listed_in(&center_off, Mode::Center));
        assert!(listed_in(&AppNotif::default(), Mode::Lock) && listed_in(&AppNotif::default(), Mode::Center));
    }

    #[test]
    fn focus_lets_only_its_allowed_apps_deliver() {
        let mut n = Notify::default();
        assert!(n.delivers(AppId::Files));
        n.set_dnd(true);
        assert!(!n.delivers(AppId::Files), "Focus holds back an app that is not allowed");
        n.store.set_focus_allowed(AppId::Files, true);
        assert!(n.delivers(AppId::Files), "an allowed app still delivers during Focus");
        assert!(!n.delivers(AppId::Terminal));
        n.store.set_muted(AppId::Files, true);
        assert!(!n.delivers(AppId::Files), "a mute beats the Focus allowance");
    }

    #[test]
    fn delivery_model_keeps_muted_or_off_apps_stored_and_picks_banner_or_alert() {
        use bifrost_ui::notifdelivery::Presentation;
        let banner = AppNotif { banners: true, ..AppNotif::default() };
        assert_eq!(presentation_of(&banner, true).presentation, Presentation::Banner);
        assert_eq!(presentation_of(&banner, false).presentation, Presentation::Stored, "muted or Focus: stored only");
        let alert = AppNotif { banners: true, persistent: true, ..AppNotif::default() };
        assert_eq!(presentation_of(&alert, true).presentation, Presentation::Alert);
        let off = AppNotif { banners: false, ..AppNotif::default() };
        assert_eq!(presentation_of(&off, true).presentation, Presentation::Stored);
        assert!(presentation_of(&banner, true).wakes_screen);
    }

    fn tap(sh: &mut Shell, x: f32, y: f32, t: f64) -> Vec<Effect> {
        let mut fx = sh.event(Event::Down(x, y), t);
        fx.extend(sh.event(Event::Up(x, y), t + 30.0));
        fx
    }

    fn draw(sh: &mut Shell, t: f64) -> Vec<u32> {
        let mut buf = vec![0u32; 1200 * 1920];
        sh.draw(&mut Surface { w: 1200, h: 1920, stride: 1200, px: &mut buf, y0: 0 }, t);
        buf
    }

    fn note(app: AppId, title: &str) -> Notification {
        Notification::new(app, title, "Something happened", 0)
    }

    #[test]
    fn focus_notifications_update_badges_and_remain_available_in_the_center() {
        let mut sh = shell();
        unlock(&mut sh);
        sh.set_dnd(true);
        sh.notify(note(AppId::Settings, "Low Battery"));
        sh.notify(note(AppId::Settings, "Wi-Fi"));
        assert!(sh.notify.banner().is_none());
        assert_eq!(sh.notify.badges.len(), 1);
        assert_eq!(sh.notify.badges[0].1, 2);
        assert_eq!(sh.badge_view().at.len(), 1);
        sh.open_notification_center(1000.0);
        settle(&mut sh, 1000.0);
        let list = sh.notify.center.list.as_ref().unwrap();
        assert_eq!(list.groups.len(), 1);
        assert_eq!(list.groups[0].count, 2);
        assert!(sh.notify.center.is_shown());
    }

    #[test]
    fn a_notification_at_home_shows_a_banner_that_leaves_after_five_seconds() {
        let mut sh = shell();
        unlock(&mut sh);
        sh.tick(1000.0);
        sh.notify(note(AppId::Files, "Timer"));
        assert_eq!(sh.notify.store().len(), 1);
        let b = sh.notify.banner().expect("banner");
        assert_eq!(b.phase(), Phase::In);
        assert!(sh.needs_frame());
        let (_, y0, _, _) = sh.banner_rect(1000.0).unwrap();
        assert!(y0 < 0.0, "starts above the screen");
        sh.tick(1000.0 + SLIDE_MS);
        assert_eq!(sh.notify.banner().unwrap().phase(), Phase::Hold);
        let (_, y, _, _) = sh.banner_rect(1500.0).unwrap();
        assert_eq!(y, sh.ui.pt(BANNER_TOP_PT));
        assert_eq!(sh.next_wake(1500.0), Some(1000.0 + SLIDE_MS + HOLD_MS));
        assert!(!sh.needs_frame() || sh.dirty, "resting costs no frames");
        let buf = draw(&mut sh, 1500.0);
        let (x, y, w, h) = sh.banner_rect(1500.0).unwrap();
        assert_ne!(buf[(y + h / 2.0) as usize * 1200 + (x + w / 2.0) as usize], buf[(y + h + 40.0) as usize * 1200 + (x + w / 2.0) as usize], "the banner shows over the home screen");
        sh.tick(1000.0 + SLIDE_MS + HOLD_MS + 1.0);
        assert_eq!(sh.notify.banner().unwrap().phase(), Phase::Out);
        sh.tick(1000.0 + 2.0 * SLIDE_MS + HOLD_MS + 1.0);
        assert!(sh.notify.banner().is_none());
        assert!(sh.describe().contains("notifications=1"));
    }

    #[test]
    fn tapping_the_banner_opens_the_app_and_clears_its_notifications() {
        let mut sh = shell();
        unlock(&mut sh);
        sh.tick(1000.0);
        sh.notify(note(AppId::Files, "Alarm"));
        sh.tick(1000.0 + SLIDE_MS);
        let (x, y, w, h) = sh.banner_rect(2000.0).unwrap();
        tap(&mut sh, x + w / 2.0, y + h / 2.0, 2000.0);
        settle(&mut sh, 2030.0);
        assert_eq!(sh.screen, Screen::App(AppId::Files));
        assert_eq!(sh.notify.store().count(AppId::Files), 0, "opening the app takes its notifications");
    }

    #[test]
    fn pulling_the_banner_up_dismisses_it() {
        let mut sh = shell();
        unlock(&mut sh);
        sh.tick(1000.0);
        sh.notify(note(AppId::Deildu, "Ready to play"));
        sh.tick(1000.0 + SLIDE_MS);
        let (x, y, w, h) = sh.banner_rect(2000.0).unwrap();
        swipe(&mut sh, x + w / 2.0, y + h / 2.0, y - 200.0, 2000.0);
        assert_eq!(sh.notify.banner().unwrap().phase(), Phase::Out);
        settle(&mut sh, 2110.0);
        assert!(sh.notify.banner().is_none());
        assert_eq!(sh.screen, Screen::Home);
        assert_eq!(sh.notify.store().len(), 1, "dismissing a banner keeps the notification");
        // a touch beside the banner is the home screen's
        sh.notify(note(AppId::Deildu, "Again"));
        sh.tick(3000.0 + SLIDE_MS);
        assert!(sh.event(Event::Down(10.0, 900.0), 4000.0).is_empty());
        assert!(!sh.notify.banner().unwrap().pulling());
    }

    #[test]
    fn locked_or_dark_screens_get_no_banner_but_the_dark_screen_wakes() {
        let mut sh = shell();
        sh.tick(1000.0);
        sh.notify(note(AppId::Files, "Timer"));
        assert!(sh.notify.banner().is_none(), "the lock screen lists it instead");
        sh.event(Event::Power, 2000.0);
        assert!(!sh.screen_on());
        sh.notify(note(AppId::Files, "Alarm"));
        assert_eq!(sh.next_wake(2500.0), Some(2500.0));
        assert_eq!(sh.tick(2500.0), vec![Effect::ScreenOn]);
        assert!(sh.screen_on() && sh.is_locked());
        assert!(sh.notify.banner().is_none());
    }

    #[test]
    fn the_lock_screen_stacks_notifications_per_app_and_a_swipe_left_clears() {
        let mut sh = shell();
        sh.tick(1000.0);
        sh.notify(note(AppId::Files, "Timer"));
        sh.notify(note(AppId::Settings, "Low Battery"));
        sh.notify(note(AppId::Files, "Alarm"));
        draw(&mut sh, 1000.0);
        let list = sh.notify.lock.as_ref().expect("lock list");
        assert_eq!(list.groups.len(), 2);
        assert_eq!((list.groups[0].app, list.groups[0].count), (AppId::Files, 2));
        let top = sh.ui.pt(LOCK_LIST_TOP_PT);
        let x = ListView::x(&sh.ui, sh.w);
        let y = top + sh.ui.pt(CARD_H_PT) / 2.0;
        let before = sh.notify.lock_state();
        sh.event(Event::Down(x + 100.0, y), 2000.0);
        sh.event(Event::Move(x + 50.0, y + 2.0), 2050.0);
        sh.event(Event::Move(x - 200.0, y + 4.0), 2100.0);
        assert_ne!(sh.notify.lock_state(), before, "the drag repaints");
        assert!(sh.lock_drag.is_none(), "a sideways drag never lifts the lock screen");
        let mid = draw(&mut sh, 2100.0);
        assert_ne!(mid[y as usize * 1200 + (x - 150.0) as usize], mid[(y - 150.0) as usize * 1200 + (x - 150.0) as usize]);
        sh.event(Event::Up(x - 200.0, y + 4.0), 2150.0);
        assert_eq!(sh.notify.store().count(AppId::Files), 0, "the whole Clock group is gone");
        assert_eq!(sh.notify.store().len(), 1);
        assert!(sh.is_locked());
        // a short drag springs back
        draw(&mut sh, 2200.0);
        sh.event(Event::Down(x + 100.0, y), 3000.0);
        sh.event(Event::Move(x + 60.0, y), 3050.0);
        sh.event(Event::Up(x + 60.0, y), 3100.0);
        assert_eq!(sh.notify.store().len(), 1);
    }

    #[test]
    fn a_half_swipe_on_a_lock_card_reveals_buttons_and_clear_removes_it() {
        let mut sh = shell();
        sh.tick(1000.0);
        sh.notify(note(AppId::Settings, "Wi-Fi"));
        draw(&mut sh, 1000.0);
        let x = ListView::x(&sh.ui, sh.w);
        let y = sh.ui.pt(LOCK_LIST_TOP_PT) + sh.ui.pt(CARD_H_PT) / 2.0;
        let rw = ListView::reveal_w(&sh.ui);
        sh.event(Event::Down(x + 300.0, y), 2000.0);
        sh.event(Event::Move(x + 300.0 - rw * 0.6, y), 2050.0);
        sh.event(Event::Up(x + 300.0 - rw * 0.6, y), 2100.0);
        assert!(sh.notify.revealed.is_some(), "the card rests open");
        assert_eq!(sh.notify.store().len(), 1);
        // Clear is the rightmost button
        let cx = x + sh.ui.pt(CARD_W_PT) - 10.0;
        sh.event(Event::Down(cx, y), 3000.0);
        sh.event(Event::Up(cx, y), 3050.0);
        assert_eq!(sh.notify.store().len(), 0);
        assert!(sh.notify.revealed.is_none());
    }

    #[test]
    fn a_vertical_drag_on_a_lock_card_still_unlocks_and_a_tap_opens_after_the_unlock() {
        let mut sh = shell();
        sh.tick(1000.0);
        sh.notify(note(AppId::Settings, "Wi-Fi"));
        draw(&mut sh, 1000.0);
        let x = ListView::x(&sh.ui, sh.w) + 100.0;
        let y = sh.ui.pt(LOCK_LIST_TOP_PT) + 20.0;
        swipe(&mut sh, x, y, y - 900.0, 2000.0);
        assert!(matches!(sh.anim, Some((Anim::Unlock { to, .. }, _)) if to >= 1.0), "{:?}", sh.anim);
        settle(&mut sh, 2200.0);
        assert_eq!(sh.screen, Screen::Home);
        assert_eq!(sh.notify.store().len(), 1, "unlocking keeps it");
        sh.event(Event::Power, 3000.0);
        sh.event(Event::Power, 3100.0);
        assert!(sh.is_locked());
        draw(&mut sh, 3100.0);
        tap(&mut sh, x, y, 4000.0);
        assert!(matches!(sh.anim, Some((Anim::Unlock { .. }, _))));
        settle(&mut sh, 4100.0);
        sh.tick(5100.0);
        settle(&mut sh, 5100.0);
        assert_eq!(sh.screen, Screen::App(AppId::Settings));
        assert_eq!(sh.notify.store().len(), 0, "opened: cleared from the lock screen");
    }

    #[test]
    fn badges_count_each_apps_notifications_on_the_home_screen() {
        let mut sh = shell();
        unlock(&mut sh);
        let clean = draw(&mut sh, 1000.0);
        sh.tick(1000.0);
        sh.notify(note(AppId::Settings, "a"));
        sh.notify(note(AppId::Settings, "b"));
        sh.notify(note(AppId::Deildu, "c"));
        settle(&mut sh, 1000.0 + 2.0 * SLIDE_MS + HOLD_MS);
        assert!(sh.notify.banner().is_none());
        let at: Vec<(i32, i32, usize, usize)> = sh.badge_view().at.iter().map(|&(x, y, img)| (x, y, img.w, img.h)).collect();
        assert_eq!(at.len(), 2);
        let r = sh.layout.dock_icons[0];
        let (bx, by, bw, bh) = at[0];
        assert!(bx + bw as i32 > r.x + r.w && by < r.y, "the badge hangs over the icon's top-right corner");
        let buf = draw(&mut sh, 9000.0);
        let (cx, cy) = (bx as usize + 5, by as usize + bh / 2);
        assert_eq!(buf[cy * 1200 + cx], list::BADGE_RED, "red at the badge's rim, left of the count");
        assert_ne!(buf[cy * 1200 + cx], clean[cy * 1200 + cx]);
        let g = sh.layout.cells[0];
        let (gx, gy, gw, _) = at[1];
        assert!(gx + gw as i32 > g.x + g.w && gy < g.y, "the grid icon has one too");
        sh.open(AppId::Settings, 9500.0);
        settle(&mut sh, 9500.0);
        assert_eq!(sh.badge_view().at.len(), 1, "opening Settings clears its badge");
        assert!(sh.describe().contains("notifications=1"));
    }

    #[test]
    fn swipe_down_from_the_left_of_the_status_bar_opens_notification_center() {
        let mut sh = shell();
        unlock(&mut sh);
        sh.tick(1000.0);
        sh.notify(note(AppId::Files, "Timer"));
        sh.notify(note(AppId::Files, "Alarm"));
        settle(&mut sh, 1000.0 + 2.0 * SLIDE_MS + HOLD_MS);
        assert!(sh.event(Event::Down(400.0, 5.0), 10_000.0).is_empty());
        assert_eq!(sh.notify.center.describe(), "dragging");
        assert!(sh.cc.is_hidden(), "the right third stays Control Center's");
        sh.event(Event::Move(400.0, 5.0 + sh.ui.pt(150.0)), 10_050.0);
        assert!((sh.notify.center.progress(10_050.0) - 0.5).abs() < 0.01);
        sh.event(Event::Up(400.0, 5.0 + sh.ui.pt(150.0)), 10_400.0);
        assert_eq!(sh.notify.center.describe(), "opening");
        assert!(sh.needs_frame());
        settle(&mut sh, 10_400.0);
        assert!(sh.notify.center.is_shown());
        assert!(sh.describe().contains("center=shown"));
        let buf = draw(&mut sh, 11_000.0);
        let list = sh.notify.center.list.as_ref().unwrap();
        assert_eq!(list.mode, Mode::Center);
        assert_eq!(list.groups[0].cards_len(), 2);
        let x = ListView::x(&sh.ui, sh.w) as usize;
        let y = sh.ui.pt(center::LIST_TOP_PT + list::HEADER_PT) as usize + 20;
        assert_ne!(buf[y * 1200 + x + 20], buf[y * 1200 + x - 20], "a card shows over the frost");
        // Clear drops the group; a tap on nothing puts the sheet away
        let w = sh.ui.pt(CARD_W_PT);
        let header_mid = sh.ui.pt(center::LIST_TOP_PT + list::HEADER_PT / 2.0);
        tap(&mut sh, x as f32 + w - 10.0, header_mid, 12_000.0);
        assert_eq!(sh.notify.store().len(), 0);
        assert!(sh.notify.center.is_shown());
        draw(&mut sh, 12_100.0);
        tap(&mut sh, 600.0, 1700.0, 13_000.0);
        assert_eq!(sh.notify.center.describe(), "closing");
        settle(&mut sh, 13_030.0);
        assert!(sh.notify.center.is_hidden());
        assert_eq!(sh.screen, Screen::Home);
    }

    #[test]
    fn notification_center_card_opens_the_app_and_swipe_up_or_power_dismisses() {
        let mut sh = shell();
        unlock(&mut sh);
        sh.tick(1000.0);
        sh.notify(note(AppId::Deildu, "Ready to play"));
        settle(&mut sh, 1000.0 + 2.0 * SLIDE_MS + HOLD_MS);
        swipe(&mut sh, 100.0, 5.0, 600.0, 10_000.0);
        settle(&mut sh, 10_110.0);
        assert!(sh.notify.center.is_shown());
        swipe(&mut sh, 600.0, 1500.0, 900.0, 11_000.0);
        assert_eq!(sh.notify.center.describe(), "closing");
        settle(&mut sh, 11_110.0);
        assert!(sh.notify.center.is_hidden());
        swipe(&mut sh, 100.0, 5.0, 600.0, 12_000.0);
        settle(&mut sh, 12_110.0);
        draw(&mut sh, 12_200.0);
        let x = ListView::x(&sh.ui, sh.w) + 100.0;
        let y = sh.ui.pt(center::LIST_TOP_PT + list::HEADER_PT) + 20.0;
        tap(&mut sh, x, y, 13_000.0);
        assert!(sh.notify.center.is_hidden());
        settle(&mut sh, 13_030.0);
        assert_eq!(sh.screen, Screen::App(AppId::Deildu));
        assert_eq!(sh.notify.store().len(), 0);
        swipe(&mut sh, 100.0, 5.0, 600.0, 14_000.0);
        settle(&mut sh, 14_110.0);
        assert!(sh.notify.center.is_shown(), "works over an app");
        draw(&mut sh, 14_200.0);
        assert_eq!(sh.event(Event::Power, 15_000.0), vec![Effect::ScreenOff]);
        assert!(sh.notify.center.is_hidden());
    }

    #[test]
    fn a_long_list_scrolls_instead_of_dismissing() {
        let mut sh = shell();
        unlock(&mut sh);
        sh.tick(1000.0);
        for app in [AppId::Files, AppId::Settings, AppId::Terminal, AppId::Deildu] {
            for i in 0..5 {
                sh.notify(note(app, &format!("{i}")));
            }
        }
        sh.notify.banner = None;
        swipe(&mut sh, 100.0, 5.0, 600.0, 10_000.0);
        settle(&mut sh, 10_110.0);
        draw(&mut sh, 10_200.0);
        assert!(sh.center_scroll_max() > 0.0);
        swipe(&mut sh, 600.0, 1500.0, 900.0, 11_000.0);
        assert!(sh.notify.center.is_shown(), "a swipe up scrolls");
        assert!(sh.notify.center.scroll_offset() > 0.0);
        settle(&mut sh, 11_110.0);
        draw(&mut sh, 12_000.0);
    }

    #[test]
    fn battery_and_wifi_changes_post_under_settings() {
        let mut sh = shell();
        unlock(&mut sh);
        sh.tick(1000.0);
        let mut next = sh.info.clone();
        next.battery = 21;
        sh.set_info(next.clone());
        assert_eq!(sh.notify.store().len(), 0);
        next.battery = 20;
        sh.set_info(next.clone());
        assert_eq!(sh.notify.store().groups()[0].latest().body, "20% battery remaining.");
        next.battery = 15;
        sh.set_info(next.clone());
        assert_eq!(sh.notify.store().len(), 1, "no second warning until 10%");
        next.battery = 9;
        sh.set_info(next.clone());
        assert_eq!(sh.notify.store().len(), 2);
        next.charging = true;
        next.battery = 100;
        sh.set_info(next.clone());
        assert_eq!(sh.notify.store().groups()[0].latest().title, "Battery Charged");
        next.min += 1;
        sh.set_info(next.clone());
        assert_eq!(sh.notify.store().len(), 3, "staying charged posts once");
        next.online = true;
        next.current_ssid = Some("Home".into());
        next.networks = vec![Network { ssid: "Home".into(), bars: 3, secure: true, saved: true }];
        sh.set_info(next.clone());
        assert_eq!(sh.notify.store().groups()[0].latest().body, "Joined Home");
        next.online = false;
        next.current_ssid = None;
        sh.set_info(next.clone());
        assert_eq!(sh.notify.store().groups()[0].latest().body, "Disconnected from Home");
        assert!(sh.notify.store().groups().iter().all(|g| g.app == AppId::Settings));
        assert_eq!(sh.notify.store().len(), 5);
        // a cable coming up is not a Wi-Fi join
        next.wired = true;
        sh.set_info(next.clone());
        assert_eq!(sh.notify.store().len(), 5, "wired links post no Wi-Fi notice");
    }

    #[test]
    fn boot_state_posts_nothing() {
        let info = Info { battery: 9, online: true, ..Info::default() };
        let data: &'static [u8] = Box::leak(std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/SFUI.ttf")).unwrap().into_boxed_slice());
        let px: &'static [u32] = Box::leak(vec![0u32; 1200 * 1920].into_boxed_slice());
        let mono: &'static [u8] = Box::leak(std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/SFUIMono.ttf")).unwrap().into_boxed_slice());
        let sh = Shell::new(1200, 1920, crate::font::Font::parse(data).unwrap(), mono, crate::gfx::View { w: 1200, h: 1920, px }, |_, _| None, info);
        assert!(sh.notify.store().is_empty());
    }

    #[test]
    fn persists_through_the_data_dir() {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/target/test-notify-shell");
        let _ = std::fs::remove_dir_all(dir);
        let mut sh = shell();
        sh.set_data_dir(dir, 0.0);
        sh.notify(note(AppId::Files, "Timer"));
        assert!(std::path::Path::new(&format!("{dir}/notifications.ini")).exists());
        let mut again = shell();
        again.set_data_dir(dir, 0.0);
        assert_eq!(again.notify.store().len(), 1);
        assert_eq!(again.notify.store().groups()[0].latest().title, "Timer");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn ages_use_the_wall_clock() {
        let mut sh = shell();
        sh.set_wall(1_000_000, 0, 1000.0);
        sh.tick(1000.0);
        assert_eq!(sh.notify.now(), 1_000_000);
        sh.notify(note(AppId::Files, "Timer"));
        sh.tick(601_000.0);
        assert_eq!(sh.notify.now(), 1_000_600);
        draw(&mut sh, 601_000.0);
        assert_eq!(sh.notify.lock_key.1, 1_000_600 / 60);
        assert_eq!(store::age_text(0, sh.notify.now()), format!("{}d ago", 1_000_600 / 86_400));
    }

    fn lock_top(sh: &Shell) -> f32 {
        sh.lock_list_top()
    }

    /// x of a point `back` pt left of the cards' right edge.
    fn right_of(sh: &Shell, back: f32) -> f32 {
        ListView::x(&sh.ui, sh.w) + sh.ui.pt(CARD_W_PT) - sh.ui.pt(back)
    }

    #[test]
    fn a_tap_on_a_stack_expands_it_and_show_less_folds_it_again() {
        let mut sh = shell();
        sh.tick(1000.0);
        sh.notify(note(AppId::Settings, "Wi-Fi"));
        sh.notify(note(AppId::Files, "Timer"));
        sh.notify(note(AppId::Files, "Alarm"));
        draw(&mut sh, 1000.0);
        let x = ListView::x(&sh.ui, sh.w) + 100.0;
        let y = lock_top(&sh) + 20.0;
        tap(&mut sh, x, y, 2000.0);
        assert!(sh.is_locked() && sh.anim.is_none(), "a tap on a stack of several opens nothing");
        assert_eq!(sh.notify.expanded, vec![AppId::Files]);
        draw(&mut sh, 2100.0);
        let list = sh.notify.lock.as_ref().unwrap();
        assert!(list.groups[0].expanded && !list.groups[1].expanded);
        assert_eq!(list.groups[0].cards_len(), 2);
        let mid = lock_top(&sh) + sh.ui.pt(list::HEADER_PT) / 2.0;
        // Show Less sits left of Mute and Clear
        {
            let (tx, ty) = (right_of(&sh, 60.0 + 6.0 + 66.0 + 6.0 + 20.0), mid);
            tap(&mut sh, tx, ty, 3000.0);
        }
        assert!(sh.notify.expanded.is_empty());
        draw(&mut sh, 3100.0);
        assert!(!sh.notify.lock.as_ref().unwrap().groups[0].expanded);
        assert_eq!(sh.notify.store().len(), 3, "folding keeps everything");
    }

    #[test]
    fn a_stack_folds_by_itself_once_it_is_down_to_one() {
        let mut sh = shell();
        sh.tick(1000.0);
        sh.notify(note(AppId::Files, "Timer"));
        sh.notify(note(AppId::Files, "Alarm"));
        draw(&mut sh, 1000.0);
        {
            let (tx, ty) = (ListView::x(&sh.ui, sh.w) + 100.0, lock_top(&sh) + 20.0);
            tap(&mut sh, tx, ty, 2000.0);
        }
        assert_eq!(sh.notify.expanded.len(), 1);
        sh.clear_notification(2);
        assert!(sh.notify.expanded.is_empty());
    }

    #[test]
    fn swiping_one_card_of_an_expanded_stack_clears_just_that_notification() {
        let mut sh = shell();
        sh.tick(1000.0);
        sh.notify(note(AppId::Files, "Timer"));
        sh.notify(note(AppId::Files, "Alarm"));
        draw(&mut sh, 1000.0);
        let x = ListView::x(&sh.ui, sh.w);
        {
            let (tx, ty) = (x + 100.0, lock_top(&sh) + 20.0);
            tap(&mut sh, tx, ty, 2000.0);
        }
        draw(&mut sh, 2100.0);
        let y = lock_top(&sh) + sh.ui.pt(list::HEADER_PT) + 20.0;
        sh.event(Event::Down(x + 100.0, y), 3000.0);
        sh.event(Event::Move(x + 50.0, y + 2.0), 3050.0);
        sh.event(Event::Move(x - 200.0, y + 4.0), 3100.0);
        assert!(sh.lock_drag.is_none());
        sh.event(Event::Up(x - 200.0, y + 4.0), 3150.0);
        assert_eq!(sh.notify.store().len(), 1);
        assert!(sh.notify.store().get(2).is_none() && sh.notify.store().get(1).is_some(), "the newest card was the one");
    }

    #[test]
    fn the_group_buttons_work_without_unlocking_and_the_lock_screen_has_no_clear_all() {
        let mut sh = shell();
        sh.tick(1000.0);
        sh.notify(note(AppId::Files, "Timer"));
        sh.notify(note(AppId::Settings, "Wi-Fi"));
        sh.notify(note(AppId::Terminal, "Term"));
        draw(&mut sh, 1000.0);
        let top = lock_top(&sh);
        // bifrost offers Clear All only in Notification Center: the lock screen keeps the cards
        let (bx, by, bw, bh) = sh.notify.lock.as_ref().unwrap().clear_all_rect(&sh.ui, sh.w, top);
        assert_eq!(sh.notify.lock.as_ref().unwrap().hit(&sh.ui, sh.w, top, bx + bw / 2.0, by + bh / 2.0), None);
        sh.clear_all_notifications();
        // Clear on one expanded stack, Mute on another
        sh.notify(note(AppId::Settings, "c"));
        sh.notify(note(AppId::Settings, "d"));
        sh.notify(note(AppId::Files, "a"));
        sh.notify(note(AppId::Files, "b"));
        sh.notify.expanded = vec![AppId::Files, AppId::Settings];
        sh.store_changed();
        draw(&mut sh, 3000.0);
        let mid = top + sh.ui.pt(list::HEADER_PT) / 2.0;
        {
            let (tx, ty) = (right_of(&sh, 40.0), mid);
            tap(&mut sh, tx, ty, 3500.0);
        }
        assert_eq!(sh.notify.store().count(AppId::Files), 0, "Clear drops the stack");
        draw(&mut sh, 3600.0);
        let mid = lock_top(&sh) + sh.ui.pt(list::HEADER_PT) / 2.0;
        {
            let (tx, ty) = (right_of(&sh, 60.0 + 6.0 + 30.0), mid);
            tap(&mut sh, tx, ty, 4000.0);
        }
        assert!(sh.app_muted(AppId::Settings));
        assert_eq!(sh.notify.store().count(AppId::Settings), 2, "muting keeps what is there");
        draw(&mut sh, 4100.0);
        let name = sh.notify.lock.as_ref().unwrap().groups[0].muted;
        assert!(name, "the stack says it is muted");
        {
            let (tx, ty) = (right_of(&sh, 60.0 + 6.0 + 30.0), mid);
            tap(&mut sh, tx, ty, 5000.0);
        }
        assert!(!sh.app_muted(AppId::Settings), "a second tap unmutes");
    }

    #[test]
    fn show_previews_decides_what_banners_and_lists_reveal() {
        let mut sh = shell();
        unlock(&mut sh);
        sh.tick(1000.0);
        let set = |sh: &mut Shell, previews: u8| {
            let page = AppNotif { previews, ..AppNotif::default() };
            let _ = sh.prefs.set(&pref_key(AppId::Files), &page.encode());
        };
        set(&mut sh, 0);
        sh.notify(note(AppId::Files, "Always"));
        assert!(!sh.notify.banner_card.as_ref().unwrap().private && !sh.previews_hidden(AppId::Files, Mode::Lock));
        set(&mut sh, 1);
        assert!(sh.previews_hidden(AppId::Files, Mode::Lock), "When Unlocked hides the text on the lock screen");
        assert!(!sh.previews_hidden(AppId::Files, Mode::Center), "and shows it once unlocked");
        sh.notify(note(AppId::Files, "When unlocked"));
        assert!(!sh.notify.banner_card.as_ref().unwrap().private);
        set(&mut sh, 2);
        assert!(sh.previews_hidden(AppId::Files, Mode::Center));
        sh.notify.banner = None;
        sh.notify(note(AppId::Files, "Never"));
        assert!(sh.notify.banner_card.as_ref().unwrap().private, "Never hides the banner text too");
    }

    #[test]
    fn clearing_a_stack_removes_only_its_thread_of_the_app() {
        let mut sh = shell();
        unlock(&mut sh);
        sh.tick(1000.0);
        let threaded = |thread: &str, title: &str| Notification { image: None, thread: thread.into(), ..note(AppId::Files, title) };
        sh.notify(threaded("chat:1", "hi"));
        sh.notify(threaded("chat:2", "yo"));
        sh.notify(threaded("chat:1", "again"));
        let groups = sh.visible_groups(Mode::Center);
        assert_eq!(groups.len(), 2, "one stack per thread");
        assert_eq!(groups[0].thread, "chat:1", "the newest stack first");
        sh.clear_group(AppId::Files, "chat:1");
        let groups = sh.notify.store.groups();
        let left: Vec<&str> = groups.iter().flat_map(|g| g.items.iter().map(|n| n.title.as_str())).collect();
        assert_eq!(left, vec!["yo"]);
    }

    #[test]
    fn a_muted_app_never_shows_a_banner_or_wakes_the_screen_but_keeps_its_badge() {
        let mut sh = shell();
        unlock(&mut sh);
        sh.tick(1000.0);
        sh.toggle_mute(AppId::Files);
        sh.notify(note(AppId::Files, "Timer"));
        assert!(sh.notify.banner().is_none());
        assert_eq!(sh.notify.badges.iter().map(|b| (b.0, b.1)).collect::<Vec<_>>(), vec![(AppId::Files, 1)]);
        sh.notify(note(AppId::Settings, "Wi-Fi"));
        assert!(sh.notify.banner().is_some(), "other apps still deliver");
        sh.notify.banner = None;
        sh.event(Event::Power, 2000.0);
        sh.notify(note(AppId::Files, "Alarm"));
        assert_eq!(sh.notify.next_wake(2100.0), None, "no wake for a muted app");
        sh.toggle_mute(AppId::Files);
        sh.notify(note(AppId::Files, "Again"));
        assert_eq!(sh.notify.next_wake(2100.0), Some(2100.0));
    }

    #[test]
    fn mutes_and_the_schedule_survive_a_restart_through_the_data_dir() {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/target/test-notify-shell-settings");
        let _ = std::fs::remove_dir_all(dir);
        let mut sh = shell();
        sh.set_data_dir(dir, 0.0);
        sh.toggle_mute(AppId::Terminal);
        sh.set_dnd_schedule(Schedule { enabled: true, start: 21 * 60, end: 6 * 60 });
        let mut again = shell();
        again.set_data_dir(dir, 0.0);
        assert!(again.app_muted(AppId::Terminal) && !again.app_muted(AppId::Files));
        assert_eq!(again.dnd_schedule(), Schedule { enabled: true, start: 21 * 60, end: 6 * 60 });
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_schedule_turns_focus_on_and_off_with_the_clock_and_a_manual_off_holds_until_it_ends() {
        let mut sh = shell();
        unlock(&mut sh);
        sh.set_wall(23 * 3600, 0, 1000.0);
        sh.tick(1000.0);
        assert!(!sh.dnd());
        sh.set_dnd_schedule(Schedule { enabled: true, start: 22 * 60, end: 7 * 60 });
        assert!(sh.dnd(), "23:00 is inside 22:00 to 07:00");
        sh.notify(note(AppId::Files, "Timer"));
        assert!(sh.notify.banner().is_none());
        assert_eq!(sh.notify.store().len(), 1, "stored all the same");
        // switching Focus off by hand inside the window
        sh.set_dnd(false);
        assert!(!sh.dnd());
        sh.notify(note(AppId::Files, "Alarm"));
        assert!(sh.notify.banner().is_some());
        sh.notify.banner = None;
        // the window ends at 07:00 and opens again at 22:00
        sh.notify.t = 2000.0;
        sh.notify.set_wall(7 * 3600 + 1800, 0, 2000.0);
        assert!(sh.notify.refresh_schedule());
        assert!(!sh.dnd() && !sh.notify.sched_skip, "the hold ended with the window");
        sh.notify.t = 3000.0;
        sh.notify.set_wall(22 * 3600 + 1800, 0, 3000.0);
        assert!(sh.notify.refresh_schedule());
        assert!(sh.dnd(), "on again for the next night");
    }

    #[test]
    fn the_schedule_follows_the_local_offset_and_opening_it_drops_the_banner() {
        let mut sh = shell();
        unlock(&mut sh);
        // 21:30 UTC is 22:30 at +1 h
        sh.set_wall(21 * 3600 + 1800, 3600, 1000.0);
        sh.tick(1000.0);
        sh.set_dnd_schedule(Schedule { enabled: false, start: 22 * 60, end: 7 * 60 });
        sh.notify(note(AppId::Files, "Timer"));
        assert!(sh.notify.banner().is_some());
        sh.set_dnd_schedule(Schedule { enabled: true, start: 22 * 60, end: 7 * 60 });
        assert!(sh.dnd());
        assert!(sh.notify.banner().is_none(), "the window opening silences what is on screen");
    }

    #[test]
    fn with_a_passcode_a_card_waits_for_it_and_the_lock_screen_hides_the_text() {
        let dir = std::path::PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/target/test-notify-lock-private"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        super::super::passcode::Store::load(dir.join("passcode.ini")).set("1234").unwrap();
        let mut sh = shell();
        sh.data_dir = dir.clone();
        sh.tick(1000.0);
        sh.notify(note(AppId::Files, "Secret title"));
        draw(&mut sh, 1000.0);
        assert!(sh.notify.lock_key.2, "private while a passcode is set");
        // the lock screen has no widget strip (OLI-729): tapping the card opens its app
        let (tx, ty) = (ListView::x(&sh.ui, sh.w) + 10.0, lock_top(&sh) + 10.0);
        tap(&mut sh, tx, ty, 2000.0);
        assert!(sh.lock_pad.is_some(), "the keypad asks first");
        assert!(sh.anim.is_none());
        assert_eq!(sh.notify.pending_open, Some(AppId::Files));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_list_moves_under_the_now_playing_card() {
        let sh = shell();
        let plain = sh.lock_list_top();
        assert_eq!(plain, sh.ui.pt(LOCK_LIST_TOP_PT));
        assert_eq!(super::super::music::nowplaying::LOCK_CARD_Y_PT, LOCK_LIST_TOP_PT, "the card takes the list's place");
    }
}
