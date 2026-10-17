//! User notifications (`UNUserNotificationCenter`): categories with actions, requests with an identifier, title,
//! subtitle, body, thread and category, removal of delivered and pending ones, and the user's answer.
//!
//! Calls are asynchronous like the document panels: `add` returns a request id, the status arrives in
//! `App::notification` as [`Event::Status`], and a tap or an action button arrives as [`Event::Response`], also when the
//! app was not running (the shell starts it and delivers the response once it is up). Needs the `notifications`
//! permission in the manifest; without it every `add` answers `NotifStatus::NotPermitted`.

pub use crate::proto::{NotifAction, NotifCategory, NotifRequest, NotifSettings, NotifStatus, TextInput, NOTIF_AUTH_MASK, NOTIF_DEFAULT_ACTION, NOTIF_DISMISS_ACTION, MAX_NOTIF_ATTACHMENTS, MAX_NOTIF_ATTACHMENT_PATH, NOTIF_CATEGORY_CUSTOM_DISMISS, NOTIF_CATEGORY_HIDDEN_SHOW_SUBTITLE, NOTIF_CATEGORY_HIDDEN_SHOW_TITLE, NOTIF_PRESENT_MASK};
use crate::proto::ToShell;
use crate::Ctx;

pub use crate::usernotif_model::{
    Attachment, AuthorizationOptions, InterruptionLevel, AuthorizationStatus, AuthorizationStore, CalendarTrigger, DateComponents, Decision, IntervalTrigger, PresentationOptions, Setting, Settings, Sound,
};

/// The play and pause button of a media notification card: its frame, tint and visibility, and the play or pause glyph.
/// [`MediaControl`](media::MediaControl) holds that state; the app starts or stops its playback on a tap and calls
/// `media_play` or `media_pause`, and the glyph follows that state.
pub mod media {
    pub use bifrost_ui::notifmedia::{Command, Glyph, MediaControl};
}

/// `UNNotificationResponse`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Response {
    /// the identifier of the request
    pub id: String,
    /// the action identifier: one of the category's, `NOTIF_DEFAULT_ACTION` or `NOTIF_DISMISS_ACTION`
    pub action: String,
    /// the text of a `UNTextInputNotificationResponse`; empty for other actions
    pub text: String,
    pub thread: String,
    pub category: String,
    /// the request's `userInfo` (`UNNotificationContent.userInfo`), as the app set it
    pub user_info: Vec<(String, String)>,
}

/// What `App::notification` is called with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// the user acted on a notification (`userNotificationCenter(_:didReceive:)`)
    Response(Response),
    /// a notification of this app arrives while the app is in front, and the app opted in with
    /// [`set_presentation_delegate`]. Answer with [`will_present`] using this `token`; without an answer in time the
    /// notification is not shown.
    WillPresent { token: u32, id: String, thread: String, category: String, title: String, body: String },
    /// the result of an [`add`]
    Status { request: u32, status: NotifStatus },
    /// the answer to [`delivered`] or [`pending`]
    List { request: u32, delivered: bool, ids: Vec<String> },
    /// the answer to [`settings`]
    Settings { request: u32, settings: NotifSettings },
    /// the answer to [`request_authorization`]: `granted` is whether the app may post notifications
    Authorization { request: u32, granted: bool },
}

/// A request under construction (`UNMutableNotificationContent` plus a trigger).
#[derive(Clone, Debug, Default)]
pub struct Request(NotifRequest);

impl Request {
    pub fn new(id: &str, title: &str) -> Self {
        Request(NotifRequest { id: id.into(), title: title.into(), ..Default::default() })
    }
    pub fn subtitle(mut self, v: &str) -> Self {
        self.0.subtitle = v.into();
        self
    }
    pub fn body(mut self, v: &str) -> Self {
        self.0.body = v.into();
        self
    }
    pub fn thread(mut self, v: &str) -> Self {
        self.0.thread = v.into();
        self
    }
    /// `categoryIdentifier`; the category must have been registered with [`set_categories`].
    pub fn category(mut self, v: &str) -> Self {
        self.0.category = v.into();
        self
    }
    /// `UNTimeIntervalNotificationTrigger`: deliver after `ms` milliseconds instead of now.
    pub fn after_ms(mut self, ms: u32) -> Self {
        self.0.delay_ms = ms;
        self
    }
    /// `content.badge`: sets the Dock badge to `n` (0 clears it).
    pub fn badge(mut self, n: u32) -> Self {
        self.0.badge = Some(n);
        self
    }
    /// `UNTimeIntervalNotificationTrigger(timeInterval:repeats:)`: with [`after_ms`] the request fires every `ms` until
    /// it is removed with [`remove_pending`]. Without a delay the request is delivered once, now.
    pub fn repeats(mut self, v: bool) -> Self {
        self.0.repeats = v;
        self
    }
}

impl Request {
    /// `content.sound`. A critical sound is only played when the app holds the critical-alert grant.
    pub fn sound(mut self, sound: &Sound) -> Self {
        let (kind, name, volume) = match sound {
            Sound::Default => (1, String::new(), 0),
            Sound::Named(n) => (2, n.clone(), 0),
            Sound::DefaultCritical => (3, String::new(), 0),
            Sound::DefaultCriticalVolume(v) => (4, String::new(), (v * 100.0).round() as u8),
        };
        self.0.sound_kind = kind;
        self.0.sound_name = name;
        self.0.sound_volume = volume;
        self
    }
    /// `content.userInfo`: at most 16 entries, each value at most 512 characters; extra entries are an error.
    pub fn user_info(mut self, info: &[(&str, &str)]) -> Result<Self, String> {
        if info.len() > crate::proto::rich::MAX_NOTIF_USER_INFO {
            return Err("userInfo takes at most 16 entries".into());
        }
        if info.iter().any(|(k, v)| k.is_empty() || k.chars().count() > crate::proto::rich::MAX_NOTIF_ID || v.chars().count() > crate::proto::rich::MAX_NOTIF_USER_INFO_VALUE) {
            return Err("a userInfo key is 1 to 128 characters and a value at most 512".into());
        }
        self.0.user_info = info.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        Ok(self)
    }
    /// `content.interruptionLevel`.
    pub fn interruption_level(mut self, level: InterruptionLevel) -> Self {
        self.0.interruption_level = level.code();
        self
    }
    /// `content.relevanceScore`, clamped to 0...1.
    /// `UNNotificationAttachment`: a PNG at an absolute `path` inside the app's data directory. The first attachment
    /// shows on the card as a thumbnail. Up to `MAX_NOTIF_ATTACHMENTS` paths; anything else is refused here.
    pub fn attachment(mut self, path: &str) -> Result<Self, String> {
        if self.0.attachments.len() >= MAX_NOTIF_ATTACHMENTS {
            return Err(format!("at most {MAX_NOTIF_ATTACHMENTS} attachments"));
        }
        if !path.starts_with('/') || path.len() > MAX_NOTIF_ATTACHMENT_PATH {
            return Err("an attachment needs an absolute path".into());
        }
        if !path.to_ascii_lowercase().ends_with(".png") {
            return Err("only PNG attachments are shown".into());
        }
        self.0.attachments.push(path.to_string());
        Ok(self)
    }

    pub fn relevance_score(mut self, score: f32) -> Self {
        self.0.relevance_milli = (score.clamp(0.0, 1.0) * 1000.0).round() as u16;
        self
    }
}

impl Event {
    pub(crate) fn response(id: String, action: String, text: String, thread: String, category: String, user_info: Vec<(String, String)>) -> Event {
        Event::Response(Response { id, action, text, thread, category, user_info })
    }
}

/// `setNotificationCategories`: replaces the registered set. Register before posting requests that name them.
pub fn set_categories(ctx: &Ctx, categories: &[NotifCategory]) {
    ctx.send_to_shell(ToShell::NotifyCategories(categories.to_vec()));
}

/// `add(_:)`: the answer is [`Event::Status`] with the returned request id. A request with an identifier that is
/// already pending or delivered replaces it.
pub fn add(ctx: &Ctx, request: Request) -> u32 {
    let id = ctx.next_request();
    ctx.send_to_shell(ToShell::NotifyRequest { request: id, req: request.0 });
    id
}

/// `removeDeliveredNotifications(withIdentifiers:)`; no ids removes all of the app's.
pub fn remove_delivered(ctx: &Ctx, ids: &[&str]) {
    ctx.send_to_shell(ToShell::NotifyRemove { delivered: true, ids: ids.iter().map(|s| s.to_string()).collect() });
}

/// `removePendingNotificationRequests(withIdentifiers:)`; no ids removes all of the app's.
pub fn remove_pending(ctx: &Ctx, ids: &[&str]) {
    ctx.send_to_shell(ToShell::NotifyRemove { delivered: false, ids: ids.iter().map(|s| s.to_string()).collect() });
}

/// `getDeliveredNotifications`: the identifiers arrive as [`Event::List`].
pub fn delivered(ctx: &Ctx) -> u32 {
    let request = ctx.next_request();
    ctx.send_to_shell(ToShell::NotifyQuery { request, delivered: true });
    request
}

/// `getPendingNotificationRequests`: the identifiers arrive as [`Event::List`].
pub fn pending(ctx: &Ctx) -> u32 {
    let request = ctx.next_request();
    ctx.send_to_shell(ToShell::NotifyQuery { request, delivered: false });
    request
}

/// The notification settings query: the answer arrives as [`Event::Settings`]. Also answered for an app without the
/// permission, with `authorized` false and no alert, sound or badge.
pub fn settings(ctx: &Ctx) -> u32 {
    let request = ctx.next_request();
    ctx.send_to_shell(ToShell::NotifySettings { request });
    request
}

/// `requestAuthorization(options:)`: asks the person to allow notifications for the app. An app asked before gets its
/// stored answer without a prompt; otherwise the shell shows the first-time alert and answers after it. The answer
/// arrives as [`Event::Authorization`]. `options` is the [`AuthorizationOptions`] bit set; a bit outside the
/// [`NOTIF_AUTH_MASK`] is refused by the shell, so only the known bits are passed on.
pub fn request_authorization(ctx: &Ctx, options: AuthorizationOptions) -> u32 {
    let request = ctx.next_request();
    ctx.send_to_shell(ToShell::NotifyAuthorize { request, options: options.0 & NOTIF_AUTH_MASK });
    request
}

/// `UNUserNotificationCenterDelegate.userNotificationCenter(_:willPresent:)`: opts the app in (`true`) or out. Only
/// an app that opted in is asked, and only for notifications that arrive while it is in front.
pub fn set_presentation_delegate(ctx: &Ctx, enabled: bool) {
    ctx.send_to_shell(ToShell::NotifyPresentDelegate { enabled });
}

/// The completion handler of `willPresent`: `options` says how the notification shows while the app is in front.
/// [`PresentationOptions::NONE`] hides it. Bits the shell does not know are dropped.
pub fn will_present(ctx: &Ctx, token: u32, options: PresentationOptions) {
    ctx.send_to_shell(ToShell::NotifyPresentReply { token, options: options.0 & NOTIF_PRESENT_MASK });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_builder_fills_every_field() {
        let r = Request::new("id", "T").subtitle("S").body("B").thread("th").category("C").after_ms(250).badge(4).repeats(true);
        assert_eq!(r.0, NotifRequest { id: "id".into(), title: "T".into(), subtitle: "S".into(), body: "B".into(), thread: "th".into(), category: "C".into(), delay_ms: 250, badge: Some(4), repeats: true, ..Default::default() });
    }

    #[test]
    fn sound_selection_maps_to_the_wire_kind() {
        let r = Request::new("a", "T").sound(&Sound::Default);
        assert_eq!(r.0.sound_kind, 1);
        let r = r.sound(&Sound::named("chime.wav").unwrap());
        assert_eq!((r.0.sound_kind, r.0.sound_name.as_str()), (2, "chime.wav"));
        let r = r.sound(&Sound::critical_with_volume(0.5).unwrap());
        assert_eq!((r.0.sound_kind, r.0.sound_volume, r.0.sound_name.as_str()), (4, 50, ""));
        assert_eq!(r.sound(&Sound::DefaultCritical).0.sound_kind, 3);
    }

    #[test]
    fn user_info_is_bounded() {
        let r = Request::new("a", "T").user_info(&[("room", "7")]).unwrap();
        assert_eq!(r.0.user_info, vec![("room".to_string(), "7".to_string())]);
        let many: Vec<(&str, &str)> = (0..17).map(|_| ("k", "v")).collect();
        assert!(Request::new("a", "T").user_info(&many).is_err());
        assert!(Request::new("a", "T").user_info(&[("", "v")]).is_err());
        assert!(Request::new("a", "T").user_info(&[("k", &"x".repeat(513))]).is_err());
    }

    #[test]
    fn interruption_levels_order_and_relevance_clamps() {
        assert!(InterruptionLevel::Passive < InterruptionLevel::Active);
        assert!(InterruptionLevel::Active < InterruptionLevel::TimeSensitive);
        assert!(InterruptionLevel::TimeSensitive < InterruptionLevel::Critical);
        assert_eq!(Request::new("a", "T").interruption_level(InterruptionLevel::TimeSensitive).0.interruption_level, 2);
        assert_eq!(Request::new("a", "T").relevance_score(2.0).0.relevance_milli, 1000);
        assert_eq!(Request::new("a", "T").relevance_score(-1.0).0.relevance_milli, 0);
        assert_eq!(Request::new("a", "T").relevance_score(0.25).0.relevance_milli, 250);
    }

    #[test]
    fn attachments_are_absolute_png_paths_and_bounded() {
        let r = Request::new("a", "T").attachment("/data/pic.PNG").unwrap();
        assert_eq!(r.0.attachments, vec!["/data/pic.PNG".to_string()]);
        assert!(Request::new("a", "T").attachment("pic.png").is_err(), "relative");
        assert!(Request::new("a", "T").attachment("/data/pic.jpg").is_err(), "not PNG");
        let mut many = Request::new("a", "T");
        for _ in 0..MAX_NOTIF_ATTACHMENTS {
            many = many.attachment("/p.png").unwrap();
        }
        assert!(many.attachment("/p.png").is_err(), "too many");
    }

    #[test]
    fn repeats_is_off_until_asked_and_keeps_the_interval() {
        assert!(!Request::new("a", "T").after_ms(60_000).0.repeats);
        let r = Request::new("a", "T").after_ms(60_000).repeats(true);
        assert!(r.0.repeats);
        assert_eq!(r.0.delay_ms, 60_000);
        assert!(!r.repeats(false).0.repeats);
    }

    #[test]
    fn new_request_has_identifier_and_title_only() {
        let r = Request::new("a", "T");
        assert_eq!(r.0, NotifRequest { id: "a".into(), title: "T".into(), ..Default::default() });
        assert_eq!(r.0.badge, None);
        assert_eq!(r.0.delay_ms, 0);
    }

    #[test]
    fn badge_zero_is_distinct_from_no_badge() {
        assert_eq!(Request::new("a", "T").badge(0).0.badge, Some(0));
    }

    #[test]
    fn builder_last_write_wins() {
        let r = Request::new("a", "T").body("one").body("two").after_ms(5).after_ms(9);
        assert_eq!(r.0.body, "two");
        assert_eq!(r.0.delay_ms, 9);
    }

    #[test]
    fn same_identifier_requests_compare_by_content() {
        let a = Request::new("same", "T").body("x");
        let b = Request::new("same", "T").body("y");
        assert_eq!(a.0.id, b.0.id);
        assert_ne!(a.0, b.0);
    }

    #[test]
    fn settings_event_carries_flags_and_request() {
        let flags = NotifSettings { authorized: true, alert: true, sound: true, badge: true, focus: false };
        assert_eq!(flags.bits(), 15);
        let e = Event::Settings { request: 3, settings: flags };
        assert_eq!(e, Event::Settings { request: 3, settings: NotifSettings { focus: false, ..flags } });
        assert_ne!(e, Event::Settings { request: 3, settings: NotifSettings::default() });
    }

    #[test]
    fn response_event_carries_every_field() {
        let info = vec![("room".to_string(), "7".to_string())];
        let e = Event::response("id".into(), NOTIF_DEFAULT_ACTION.into(), "typed".into(), "th".into(), "cat".into(), info.clone());
        assert_eq!(
            e,
            Event::Response(Response { id: "id".into(), action: NOTIF_DEFAULT_ACTION.into(), text: "typed".into(), thread: "th".into(), category: "cat".into(), user_info: info })
        );
        assert_ne!(NOTIF_DEFAULT_ACTION, NOTIF_DISMISS_ACTION);
    }

    #[test]
    fn media_button_asks_for_the_opposite_of_the_playing_state() {
        use super::media::{Command, Glyph, MediaControl};
        let frame = bifrost_ui::Rect { x: 10.0, y: 10.0, w: 20.0, h: 20.0 };
        let mut control = MediaControl::new(frame);
        assert_eq!(control.tap(15.0, 15.0), None, "a hidden button takes no tap");
        control.show();
        assert_eq!(control.tap(15.0, 15.0), Some(Command::Play));
        assert_eq!(control.tap(0.0, 0.0), None, "a tap outside the frame is not the button");
        control.media_play();
        assert_eq!(control.glyph(), Some(Glyph::Pause));
        assert_eq!(control.tap(15.0, 15.0), Some(Command::Pause));
    }
}
