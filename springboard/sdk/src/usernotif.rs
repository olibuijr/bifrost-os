//! User notifications (`UNUserNotificationCenter`): categories with actions, requests with an identifier, title,
//! subtitle, body, thread and category, removal of delivered and pending ones, and the user's answer.
//!
//! Calls are asynchronous like the document panels: `add` returns a request id, the status arrives in
//! `App::notification` as [`Event::Status`], and a tap or an action button arrives as [`Event::Response`], also when the
//! app was not running (the shell starts it and delivers the response once it is up). Needs the `notifications`
//! permission in the manifest; without it every `add` answers `NotifStatus::NotPermitted`.

pub use crate::proto::{NotifAction, NotifCategory, NotifRequest, NotifStatus, TextInput, NOTIF_DEFAULT_ACTION, NOTIF_DISMISS_ACTION};
use crate::proto::ToShell;
use crate::Ctx;

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
}

/// What `App::notification` is called with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// the user acted on a notification (`userNotificationCenter(_:didReceive:)`)
    Response(Response),
    /// the result of an [`add`]
    Status { request: u32, status: NotifStatus },
    /// the answer to [`delivered`] or [`pending`]
    List { request: u32, delivered: bool, ids: Vec<String> },
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
}

impl Event {
    pub(crate) fn response(id: String, action: String, text: String, thread: String, category: String) -> Event {
        Event::Response(Response { id, action, text, thread, category })
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_builder_fills_every_field() {
        let r = Request::new("id", "T").subtitle("S").body("B").thread("th").category("C").after_ms(250).badge(4);
        assert_eq!(r.0, NotifRequest { id: "id".into(), title: "T".into(), subtitle: "S".into(), body: "B".into(), thread: "th".into(), category: "C".into(), delay_ms: 250, badge: Some(4) });
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
    fn response_event_carries_every_field() {
        let e = Event::response("id".into(), NOTIF_DEFAULT_ACTION.into(), "typed".into(), "th".into(), "cat".into());
        assert_eq!(
            e,
            Event::Response(Response { id: "id".into(), action: NOTIF_DEFAULT_ACTION.into(), text: "typed".into(), thread: "th".into(), category: "cat".into() })
        );
        assert_ne!(NOTIF_DEFAULT_ACTION, NOTIF_DISMISS_ACTION);
    }
}
