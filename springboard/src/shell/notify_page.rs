//! Settings pages for the notification features: the Do Not Disturb
//! schedule (appended to Focus) and Notifications, one Muted switch per app
//! that can post. Settings keeps a copy of the state (`NotifyPrefs`) and
//! reports edits as `NotifyChange`s that the shell applies to its store.

use super::icons::Icon;
use super::list::{Group, Item, Spec};
use super::row_glyphs::Glyph;
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
    /// apps whose notifications still show while Focus is on
    pub focus_allowed: Vec<AppId>,
    /// the names of the Focus modes, in the order of `notify::modes::Modes::list`
    pub modes: Vec<String>,
    /// the index into `modes` of the mode that is on by hand
    pub mode_on: Option<usize>,
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

impl AppNotif {
    /// The Alert Style index (see `ALERT_STYLES`): 0 None when banners are off, 2 Alerts for a persistent banner,
    /// 1 Banners otherwise.
    pub fn style(&self) -> u8 {
        match (self.banners, self.persistent) {
            (false, _) => 0,
            (true, false) => 1,
            (true, true) => 2,
        }
    }
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
    /// the Alert Style row chosen: 0 None, 1 Banners, 2 Alerts
    Style(u8),
}

pub const PREVIEWS: [&str; 3] = ["Always", "When Unlocked", "Never"];
pub const GROUPING: [&str; 3] = ["Automatic", "By App", "Off"];
/// The Alert Style choices, in the order of `AppField::Style`: None (no banner), Banners (a banner that goes away on
/// its own), Alerts (a banner that stays until it is dismissed). These are the `UNAlertStyle` cases none, banner and alert.
pub const ALERT_STYLES: [&str; 3] = ["None", "Banners", "Alerts"];

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
            AppField::Style(s) => match s {
                0 => self.banners = false,
                1 => {
                    self.banners = true;
                    self.persistent = false;
                }
                _ => {
                    self.banners = true;
                    self.persistent = true;
                }
            },
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

    pub fn focus_allows(&self, app: AppId) -> bool {
        self.focus_allowed.contains(&app)
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
    /// the Allow During Focus switch of one app
    FocusAllowed(AppId),
    /// a row of the Focus mode list (an index into `NotifyPrefs::modes`)
    Mode(usize),
}

/// An edit the shell must apply to its notification store.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum NotifyChange {
    Schedule(Schedule),
    ToggleMute(AppId),
    /// an app's options changed (persisted by the shell)
    App(AppId, AppNotif),
    /// an app was allowed through Focus, or held back (persisted by the shell)
    FocusAllowed(AppId, bool),
    /// a Focus mode was switched on by hand (an index into the mode list), or Focus was switched off
    Mode(Option<usize>),
}

fn clock(minute: u16, h24: bool) -> String {
    crate::status::clock_in((minute / 60) as i32, (minute % 60) as i32, h24)
}

/// The symbol tile of a Focus mode: Do Not Disturb has the Focus moon, the built-in modes their own, and a mode the
/// person made a ring and a dot.
fn with_mode_symbol(item: Item, name: &str) -> Item {
    match name.trim().to_lowercase().as_str() {
        "do not disturb" => item.icon(Icon::Focus),
        "work" => item.glyph(Glyph::ModeWork),
        "personal" => item.glyph(Glyph::ModePersonal),
        "sleep" => item.glyph(Glyph::ModeSleep),
        _ => item.glyph(Glyph::ModeOther),
    }
}

/// The Focus modes, one row each; the mode that is on carries the check. Tapping it again switches Focus off.
pub fn modes_group(p: &NotifyPrefs) -> Group {
    Group::new(p.modes.iter().enumerate().map(|(i, name)| with_mode_symbol(Item::new(name.as_str()), name).check(p.mode_on == Some(i), Action::Notify(NotifyEdit::Mode(i)))).collect())
        .header("FOCUS MODES")
        .footer(super::pages::by_form("A mode holds back the notifications of every app and person it does not allow. Tap the mode that is on to turn Focus off.", "A mode holds back the notifications of every app and person it does not allow. Click the mode that is on to turn Focus off."))
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

/// The id of the app's icon (`apphost::icon_image` or the built-in drawn icon), for the leading slot of its row.
fn app_icon_id(app: AppId) -> Option<&'static str> {
    apps().iter().find(|a| a.id == app).map(|a| a.icon)
}

/// What an app's row says about its notifications: "Banners, Sounds, Badges" for what is on, "Off" when the app is
/// muted.
pub fn summary(p: &NotifyPrefs, app: AppId) -> String {
    if p.is_muted(app) {
        return "Off".into();
    }
    let n = p.app(app);
    let on: Vec<&str> = [(n.style() == 1, "Banners"), (n.style() == 2, "Alerts"), (n.sounds, "Sounds"), (n.badges, "Badges")].into_iter().filter(|(on, _)| *on).map(|(_, label)| label).collect();
    if on.is_empty() { "On".into() } else { on.join(", ") }
}

/// Settings › Notifications: every app, opening its page.
pub fn spec(p: &NotifyPrefs) -> Spec {
    let rows = posting_apps()
        .map(|(app, name)| {
            let row = Item::new(name).value(summary(p, app)).link(PageId::NotificationApp);
            match app_icon_id(app) {
                Some(id) => row.app_icon(id),
                None => row,
            }
        })
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
                Item::new("Notification Center").check(n.center, f(AppField::Center)),
            ])
            .header("ALERTS"),
        );
        let style = n.style();
        groups.push(
            Group::new(
                ALERT_STYLES
                    .iter()
                    .enumerate()
                    .map(|(i, label)| Item::new(*label).check(style == i as u8, f(AppField::Style(i as u8))))
                    .collect(),
            )
            .header("ALERT STYLE"),
        );
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
        groups.push(Group::new(vec![Item::new("Allow During Focus").switch(p.focus_allows(app), Action::Notify(NotifyEdit::FocusAllowed(app)))]).footer("Focus still holds back the other apps."));
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
        let p = NotifyPrefs { schedule: Schedule { enabled: true, start: 22 * 60, end: 7 * 60 }, clock_24h: true, ..Default::default() };
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
    fn each_app_row_has_its_icon_and_says_what_is_on() {
        let p = NotifyPrefs { muted: vec![AppId::Photos], apps: vec![(AppId::Camera, AppNotif { sounds: false, persistent: true, ..AppNotif::default() })], ..Default::default() };
        let s = spec(&p);
        assert!(s.groups[0].items.iter().all(|i| i.app_icon.is_some()), "every row has an icon slot");
        let row = |name: &str| s.groups[0].items.iter().find(|i| i.label == name).expect("a row").value.clone();
        assert_eq!(row("Photos"), "Off", "a muted app");
        assert_eq!(row("Camera"), "Alerts, Badges");
        assert_eq!(summary(&NotifyPrefs::default(), AppId::Files), "Banners, Sounds, Badges");
        let quiet = NotifyPrefs { apps: vec![(AppId::Files, AppNotif { banners: false, sounds: false, badges: false, ..AppNotif::default() })], ..Default::default() };
        assert_eq!(summary(&quiet, AppId::Files), "On", "allowed, but nothing shows or sounds");
    }

    #[test]
    fn focus_modes_have_their_symbols() {
        let p = NotifyPrefs { modes: vec!["Do Not Disturb".into(), "Work".into(), "Personal".into(), "Sleep".into(), "Reading".into()], ..Default::default() };
        let g = modes_group(&p);
        assert_eq!(g.items[0].icon, Some(Icon::Focus));
        let glyphs: Vec<_> = g.items[1..].iter().map(|i| i.glyph).collect();
        assert_eq!(glyphs, [Some(Glyph::ModeWork), Some(Glyph::ModePersonal), Some(Glyph::ModeSleep), Some(Glyph::ModeOther)]);
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
    fn the_alert_style_is_none_banners_or_alerts() {
        let mut n = AppNotif::default();
        assert_eq!(n.style(), 1, "banners is the default");
        n.apply(AppField::Style(0));
        assert!(!n.banners && n.style() == 0);
        n.apply(AppField::Style(2));
        assert!(n.banners && n.persistent && n.style() == 2);
        n.apply(AppField::Style(1));
        assert!(n.banners && !n.persistent && n.style() == 1);
        assert_eq!(AppNotif::decode(&n.encode()), n);
    }

    #[test]
    fn the_alert_style_rows_mark_the_current_style() {
        let p = NotifyPrefs { apps: vec![(AppId::Photos, AppNotif { persistent: true, ..AppNotif::default() })], ..Default::default() };
        let s = app_spec(&p, "Photos");
        let style = &s.groups[2];
        assert_eq!(style.header.as_deref(), Some("ALERT STYLE"));
        let checks: Vec<bool> = style.items.iter().map(|i| i.accessory == super::super::list::Accessory::Check(true)).collect();
        assert_eq!(checks, [false, false, true]);
        assert_eq!(style.items[0].action, Action::Notify(NotifyEdit::App(AppId::Photos, AppField::Style(0))));
        let off = app_spec(&NotifyPrefs { muted: vec![AppId::Photos], ..Default::default() }, "Photos");
        assert_eq!(off.groups.len(), 1, "a muted app shows no alert style");
    }

    #[test]
    fn allow_during_focus_is_a_switch_on_an_allowed_app_page() {
        let p = NotifyPrefs { focus_allowed: vec![AppId::Photos], ..Default::default() };
        let s = app_spec(&p, "Photos");
        let last = s.groups.last().unwrap();
        assert_eq!(last.items[0].label, "Allow During Focus");
        assert_eq!(last.items[0].accessory, super::super::list::Accessory::Switch(true));
        assert_eq!(last.items[0].action, Action::Notify(NotifyEdit::FocusAllowed(AppId::Photos)));
        assert_eq!(app_spec(&NotifyPrefs { muted: vec![AppId::Photos], ..Default::default() }, "Photos").groups.len(), 1);
    }

    #[test]
    fn the_focus_page_lists_the_modes_and_checks_the_one_that_is_on() {
        let p = NotifyPrefs { modes: vec!["Do Not Disturb".into(), "Work".into(), "Sleep".into()], mode_on: Some(1), ..Default::default() };
        let g = modes_group(&p);
        assert_eq!(g.header.as_deref(), Some("FOCUS MODES"));
        let labels: Vec<&str> = g.items.iter().map(|i| i.label.as_str()).collect();
        assert_eq!(labels, ["Do Not Disturb", "Work", "Sleep"]);
        let checks: Vec<bool> = g.items.iter().map(|i| i.accessory == super::super::list::Accessory::Check(true)).collect();
        assert_eq!(checks, [false, true, false]);
        assert_eq!(g.items[2].action, Action::Notify(NotifyEdit::Mode(2)));
        let none = modes_group(&NotifyPrefs { modes: vec!["Work".into()], ..Default::default() });
        assert!(none.items.iter().all(|i| i.accessory == super::super::list::Accessory::Check(false)));
    }

    #[test]
    fn edges_wrap_into_a_day() {
        let mut s = Schedule::default();
        assert!(set_edge(&mut s, NotifyEdit::From, 1500));
        assert_eq!(s.start, 60);
        assert!(!set_edge(&mut s, NotifyEdit::Scheduled, 5));
    }
}
