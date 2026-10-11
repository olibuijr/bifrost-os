//! Settings pages for the notification features: the Do Not Disturb
//! schedule (appended to Focus) and Notifications, one Muted switch per app
//! that can post. Settings keeps a copy of the state (`NotifyPrefs`) and
//! reports edits as `NotifyChange`s that the shell applies to its store.

use super::list::{Group, Item, Spec};
use super::notify::schedule::Schedule;
use super::pages::{Action, PageId};
use super::{AppId, apps};

/// The notification state the pages show.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct NotifyPrefs {
    pub schedule: Schedule,
    pub muted: Vec<AppId>,
    /// 24-hour clock for the From and To rows
    pub clock_24h: bool,
    /// per-app options other than Allow Notifications (that is `muted`)
    pub apps: Vec<(AppId, AppNotif)>,
}

/// One app's page of Settings > Notifications (bifrost defaults: everything on, Temporary banners).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AppNotif {
    pub lock_screen: bool,
    pub center: bool,
    pub banners: bool,
    pub persistent: bool,
    pub sounds: bool,
    pub badges: bool,
    /// 0 Always, 1 When Unlocked, 2 Never
    pub previews: u8,
    /// 0 Automatic, 1 By App, 2 Off
    pub grouping: u8,
}

impl Default for AppNotif {
    fn default() -> Self {
        AppNotif { lock_screen: true, center: true, banners: true, persistent: false, sounds: true, badges: true, previews: 0, grouping: 0 }
    }
}

/// A row of an app's page.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum AppField {
    LockScreen,
    Center,
    Banners,
    Persistent(bool),
    Sounds,
    Badges,
    Previews,
    Grouping,
}

pub const PREVIEWS: [&str; 3] = ["Always", "When Unlocked", "Never"];
pub const GROUPING: [&str; 3] = ["Automatic", "By App", "Off"];

impl AppNotif {
    /// `prefs` value: eight digits, one per field; anything malformed is the default.
    pub fn encode(&self) -> String {
        let b = |v: bool| if v { '1' } else { '0' };
        format!("{}{}{}{}{}{}{}{}", b(self.lock_screen), b(self.center), b(self.banners), b(self.persistent), b(self.sounds), b(self.badges), self.previews, self.grouping)
    }

    pub fn decode(s: &str) -> AppNotif {
        let d: Vec<u8> = s.bytes().map(|c| c.wrapping_sub(b'0')).collect();
        if d.len() != 8 || d[..6].iter().any(|&x| x > 1) || d[6] > 2 || d[7] > 2 {
            return AppNotif::default();
        }
        AppNotif { lock_screen: d[0] == 1, center: d[1] == 1, banners: d[2] == 1, persistent: d[3] == 1, sounds: d[4] == 1, badges: d[5] == 1, previews: d[6], grouping: d[7] }
    }

    pub fn apply(&mut self, f: AppField) {
        match f {
            AppField::LockScreen => self.lock_screen = !self.lock_screen,
            AppField::Center => self.center = !self.center,
            AppField::Banners => self.banners = !self.banners,
            AppField::Persistent(p) => self.persistent = p,
            AppField::Sounds => self.sounds = !self.sounds,
            AppField::Badges => self.badges = !self.badges,
            AppField::Previews => self.previews = (self.previews + 1) % 3,
            AppField::Grouping => self.grouping = (self.grouping + 1) % 3,
        }
    }
}

/// The `prefs` key of an app's options.
pub fn pref_key(app: AppId) -> String {
    format!("notif.{}", apps().iter().find(|a| a.id == app).map_or("", |a| a.icon))
}

impl NotifyPrefs {
    pub fn app(&self, app: AppId) -> AppNotif {
        self.apps.iter().find(|(a, _)| *a == app).map_or_else(AppNotif::default, |(_, n)| *n)
    }

    pub fn is_muted(&self, app: AppId) -> bool {
        self.muted.contains(&app)
    }
}

/// What a row of these pages does.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum NotifyEdit {
    /// the Scheduled switch
    Scheduled,
    /// open the time picker for the start of the window
    From,
    /// open the time picker for the end of the window
    To,
    /// the Muted switch of one app
    Mute(AppId),
    /// a row of one app's page
    App(AppId, AppField),
}

/// An edit the shell must apply to its notification store.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum NotifyChange {
    Schedule(Schedule),
    ToggleMute(AppId),
    /// an app's options changed (persisted by the shell)
    App(AppId, AppNotif),
}

fn clock(minute: u16, h24: bool) -> String {
    crate::status::clock_in((minute / 60) as i32, (minute % 60) as i32, h24)
}

/// The Scheduled switch with its From and To rows, for the Focus page.
pub fn schedule_group(p: &NotifyPrefs) -> Group {
    Group::new(vec![
        Item::new("Scheduled").switch(p.schedule.enabled, Action::Notify(NotifyEdit::Scheduled)),
        Item::new("From").value(clock(p.schedule.start, p.clock_24h)).action(Action::Notify(NotifyEdit::From)),
        Item::new("To").value(clock(p.schedule.end, p.clock_24h)).action(Action::Notify(NotifyEdit::To)),
    ])
    .header("SCHEDULE")
    .footer("Turns Do Not Disturb on and off by itself every day. A window can run past midnight.")
}

/// Apps that can post notifications, in launcher order.
pub fn posting_apps() -> impl Iterator<Item = (AppId, &'static str)> {
    apps().iter().filter(|a| a.id != AppId::Settings).map(|a| (a.id, a.name))
}

/// Settings › Notifications: every app, opening its page.
pub fn spec(p: &NotifyPrefs) -> Spec {
    let rows = posting_apps()
        .map(|(app, name)| Item::new(name).value(if p.is_muted(app) { "Off" } else { "On" }).link(PageId::NotificationApp))
        .collect();
    Spec { id: PageId::Notifications, title: "Notifications".into(), large: false, groups: vec![Group::new(rows).header("NOTIFICATION STYLE")] }
}

/// Settings › Notifications › one app. Rows are the bifrost ones; the phone mockups of the
/// alert styles are not drawn.
pub fn app_spec(p: &NotifyPrefs, name: &str) -> Spec {
    let app = posting_apps().find(|(_, n)| *n == name).map(|(a, _)| a);
    let Some(app) = app else { return Spec { id: PageId::NotificationApp, title: name.into(), large: false, groups: vec![] } };
    let n = p.app(app);
    let allowed = !p.is_muted(app);
    let f = |f| Action::Notify(NotifyEdit::App(app, f));
    let mut groups = vec![Group::new(vec![Item::new("Allow Notifications").switch(allowed, Action::Notify(NotifyEdit::Mute(app)))])];
    if allowed {
        groups.push(
            Group::new(vec![
                Item::new("Lock Screen").check(n.lock_screen, f(AppField::LockScreen)),
                Item::new("Notification Centre").check(n.center, f(AppField::Center)),
                Item::new("Banners").check(n.banners, f(AppField::Banners)),
            ])
            .header("ALERTS"),
        );
        if n.banners {
            groups.push(Group::new(vec![
                Item::new("Temporary").check(!n.persistent, f(AppField::Persistent(false))),
                Item::new("Persistent").check(n.persistent, f(AppField::Persistent(true))),
            ]).header("BANNER STYLE"));
        }
        groups.push(Group::new(vec![
            Item::new("Sounds").switch(n.sounds, f(AppField::Sounds)),
            Item::new("Badges").switch(n.badges, f(AppField::Badges)),
        ]));
        groups.push(
            Group::new(vec![
                Item::new("Show Previews").value(PREVIEWS[n.previews as usize % 3]).action(f(AppField::Previews)),
                Item::new("Notification Grouping").value(GROUPING[n.grouping as usize % 3]).action(f(AppField::Grouping)),
            ])
            .header("LOCK SCREEN APPEARANCE"),
        );
    }
    Spec { id: PageId::NotificationApp, title: name.into(), large: false, groups }
}

/// Applies a time picked for the From or To row; false when `edit` is not one of them.
pub fn set_edge(s: &mut Schedule, edit: NotifyEdit, minute: u16) -> bool {
    match edit {
        NotifyEdit::From => s.start = minute % 1440,
        NotifyEdit::To => s.end = minute % 1440,
        _ => return false,
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schedule_rows_show_the_window() {
        let p = NotifyPrefs { schedule: Schedule { enabled: true, start: 22 * 60, end: 7 * 60 }, muted: vec![], clock_24h: true, apps: vec![] };
        let g = schedule_group(&p);
        assert_eq!(g.items[0].accessory, super::super::list::Accessory::Switch(true));
        assert_eq!((g.items[1].value.as_str(), g.items[2].value.as_str()), ("22:00", "7:00"));
        assert_eq!(g.items[1].action, Action::Notify(NotifyEdit::From));
    }

    #[test]
    fn every_posting_app_gets_a_row_and_a_page() {
        let p = NotifyPrefs { muted: vec![AppId::Photos], ..Default::default() };
        let s = spec(&p);
        assert_eq!(s.groups[0].items.len(), posting_apps().count());
        assert!(s.groups[0].items.iter().all(|i| i.label != "Settings"));
        assert_eq!(app_spec(&p, "Photos").groups.len(), 1);
        assert!(app_spec(&NotifyPrefs::default(), "Photos").groups.len() > 4);
    }

    #[test]
    fn app_options_round_trip() {
        let mut n = AppNotif::default();
        n.apply(AppField::Persistent(true));
        n.apply(AppField::Previews);
        assert_eq!(AppNotif::decode(&n.encode()), n);
        assert_eq!(AppNotif::decode("zz"), AppNotif::default());
    }

    #[test]
    fn edges_wrap_into_a_day() {
        let mut s = Schedule::default();
        assert!(set_edge(&mut s, NotifyEdit::From, 1500));
        assert_eq!(s.start, 60);
        assert!(!set_edge(&mut s, NotifyEdit::Scheduled, 5));
    }
}
