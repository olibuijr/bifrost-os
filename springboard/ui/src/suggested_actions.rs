//! The suggested-actions panel model: for a piece of text, every detected item (link, date, phone, address and so on)
//! with the actions its kind offers, in the order the text reads.
//!
//! The panel host draws these rows and runs the one tapped. The detector finds the items (`detect::find`) and the
//! action titles come from `detect::menu_actions`, so the panel and the context menu always offer the same actions.
//!
//! [`route`] says what running an action does, as data the shell carries out: copy the item, open a link in the
//! browser, start a mail, or add an event to the calendar. An action with no route (a call, a message, a map, a
//! reading list, a flight status: the apps or services behind them are not built) is not offered, so a row on the
//! panel always does something. [`offers`] lists the rows the panel draws.

use crate::detect::{self, Kind, Match};
use std::ops::Range;

/// One detected item and the actions it offers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Suggestion {
    pub kind: Kind,
    /// byte range of the item in the source text
    pub range: Range<usize>,
    /// the item as written in the text
    pub text: String,
    /// the action titles for this kind, first action first
    pub actions: Vec<&'static str>,
}

/// Every item of `kinds` found in `text`, in reading order. Items that no kind offers an action for are skipped.
pub fn suggest(text: &str, kinds: &[Kind]) -> Vec<Suggestion> {
    detect::find(text, kinds).into_iter().filter_map(|m| suggestion(&m, text)).collect()
}

fn suggestion(m: &Match, text: &str) -> Option<Suggestion> {
    let actions = detect::menu_actions(m.kind);
    if actions.is_empty() {
        return None;
    }
    Some(Suggestion { kind: m.kind, range: m.range.clone(), text: m.text(text).to_string(), actions: actions.to_vec() })
}

/// The action the panel runs first when an item is tapped: its first title, or `None` when the item offers nothing.
pub fn primary_action(s: &Suggestion) -> Option<&'static str> {
    s.actions.first().copied()
}

/// Every kind the panel looks for.
pub const ALL_KINDS: [Kind; 7] = [Kind::Link, Kind::Email, Kind::Date, Kind::Address, Kind::Money, Kind::Flight, Kind::Phone];

/// What running an action does. The shell carries it out with its own open paths.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Route {
    /// Put this text on the clipboard.
    Copy(String),
    /// Open this web address in the browser.
    OpenUrl(String),
    /// Start a new message to this address: a `mailto:` link for Mail.
    ComposeMail(String),
    /// Add an event to the calendar: its title, and its day (`yyyy-mm-dd`) when the text names one.
    CreateEvent { title: String, day: Option<String> },
}

/// The web address a link item opens: `http://` and `https://` addresses as written, a bare `www.` or host address on
/// `https://`. `None` for any other scheme or for text with a space or a control character.
pub fn web_address(text: &str) -> Option<String> {
    let t = text.trim();
    if t.is_empty() || t.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return None;
    }
    let lower = t.to_ascii_lowercase();
    if lower.starts_with("http://") || lower.starts_with("https://") {
        return Some(t.to_string());
    }
    // another scheme (`ftp://`, `file://`, `javascript:`) is not opened from here
    if lower.contains("://") || lower.starts_with("javascript:") || lower.starts_with("data:") || lower.starts_with("mailto:") {
        return None;
    }
    Some(format!("https://{t}"))
}

/// The ISO day (`yyyy-mm-dd`) of a detected date written `yyyy-mm-dd` or `d.m.yyyy`. `None` for anything else.
pub fn iso_day(text: &str) -> Option<String> {
    let t = text.trim();
    let num = |s: &str| -> Option<u32> { (!s.is_empty() && s.len() <= 4 && s.bytes().all(|b| b.is_ascii_digit())).then(|| s.parse().ok()).flatten() };
    let (y, m, d) = if let Some((y, rest)) = t.split_once('-') {
        let (m, d) = rest.split_once('-')?;
        (num(y)?, num(m)?, num(d)?)
    } else {
        let mut parts = t.split('.');
        let (d, m, y) = (parts.next()?, parts.next()?, parts.next()?);
        if parts.next().is_some() {
            return None;
        }
        (num(y)?, num(m)?, num(d)?)
    };
    ((1..=12).contains(&m) && (1..=31).contains(&d) && y >= 1000).then(|| format!("{y:04}-{m:02}-{d:02}"))
}

/// What the action titled `action` does for an item of `kind` written `text`. `None` when nothing in BifrOSt does it.
pub fn route(kind: Kind, action: &str, text: &str) -> Option<Route> {
    if action.starts_with("Copy ") {
        return Some(Route::Copy(text.to_string()));
    }
    match (kind, action) {
        (Kind::Link, "Open Link") => web_address(text).map(Route::OpenUrl),
        (Kind::Email, "Compose Email") => {
            let ok = !text.is_empty() && text.contains('@') && !text.chars().any(|c| c.is_control() || c.is_whitespace() || c == '?' || c == '&');
            ok.then(|| Route::ComposeMail(format!("mailto:{text}")))
        }
        (Kind::Date, "Add to Calendar") => Some(Route::CreateEvent { title: text.to_string(), day: iso_day(text) }),
        _ => None,
    }
}

/// One row of the panel: the action's title, the item it acts on, and what it does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Offer {
    pub title: &'static str,
    pub item: String,
    pub route: Route,
}

/// The rows for `text`: every detected item's actions that have a route, in reading order, at most `max` rows. The
/// same item and action never appear twice.
pub fn offers(text: &str, kinds: &[Kind], max: usize) -> Vec<Offer> {
    let mut out: Vec<Offer> = Vec::new();
    for s in suggest(text, kinds) {
        for title in &s.actions {
            if out.len() >= max {
                return out;
            }
            let Some(route) = route(s.kind, title, &s.text) else { continue };
            if !out.iter().any(|o| o.title == *title && o.item == s.text) {
                out.push(Offer { title, item: s.text.clone(), route });
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_a_link_and_offers_the_link_actions() {
        let out = suggest("see https://bifrost.olibuijr.com/docs now", &[Kind::Link]);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].kind, Kind::Link);
        assert_eq!(out[0].text, "https://bifrost.olibuijr.com/docs");
        assert!(out[0].actions.contains(&"Copy Link"));
    }

    #[test]
    fn items_come_in_reading_order() {
        let text = "call 555 1234 or mail someone@example.com";
        let out = suggest(text, &[Kind::Phone, Kind::Email]);
        let kinds: Vec<Kind> = out.iter().map(|s| s.kind).collect();
        assert_eq!(kinds, [Kind::Phone, Kind::Email]);
        assert!(out[0].range.start < out[1].range.start);
    }

    #[test]
    fn only_the_requested_kinds_are_suggested() {
        let text = "https://example.com on 2026-10-09";
        let out = suggest(text, &[Kind::Date]);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].kind, Kind::Date);
    }

    #[test]
    fn ranges_index_the_source_text() {
        let text = "pay $12.50 today";
        let out = suggest(text, &[Kind::Money]);
        assert_eq!(out.len(), 1);
        assert_eq!(&text[out[0].range.clone()], out[0].text);
    }

    #[test]
    fn primary_action_is_the_first_title_of_the_kind() {
        let out = suggest("call 555 1234", &[Kind::Phone]);
        assert_eq!(primary_action(&out[0]), Some("Call"));
    }

    #[test]
    fn a_link_opens_on_the_web_and_copies() {
        let text = "see www.bifrost.olibuijr.com/docs and https://example.com/a";
        let rows = offers(text, &[Kind::Link], 8);
        let titles: Vec<&str> = rows.iter().map(|o| o.title).collect();
        assert_eq!(titles, ["Open Link", "Copy Link", "Open Link", "Copy Link"], "Add to Reading List has no route");
        assert_eq!(rows[0].route, Route::OpenUrl("https://www.bifrost.olibuijr.com/docs".into()));
        assert_eq!(rows[2].route, Route::OpenUrl("https://example.com/a".into()));
        assert_eq!(rows[1].route, Route::Copy("www.bifrost.olibuijr.com/docs".into()));
    }

    #[test]
    fn only_web_schemes_are_opened() {
        assert_eq!(web_address("HTTP://x.is"), Some("HTTP://x.is".into()));
        assert_eq!(web_address("ftp://x.is"), None);
        assert_eq!(web_address("javascript:alert(1)"), None);
        assert_eq!(web_address("a b"), None);
        assert_eq!(web_address(""), None);
        assert_eq!(route(Kind::Link, "Open Link", "file:///etc/passwd"), None);
    }

    #[test]
    fn an_email_starts_a_message_and_a_date_adds_an_event() {
        let mail = route(Kind::Email, "Compose Email", "ada@example.com");
        assert_eq!(mail, Some(Route::ComposeMail("mailto:ada@example.com".into())));
        assert_eq!(route(Kind::Email, "Compose Email", "a@b.c?bcc=x@y.z"), None, "no header injection");
        assert_eq!(
            route(Kind::Date, "Add to Calendar", "9.10.2026"),
            Some(Route::CreateEvent { title: "9.10.2026".into(), day: Some("2026-10-09".into()) })
        );
        assert_eq!(iso_day("2026-10-09").as_deref(), Some("2026-10-09"));
        assert_eq!(iso_day("9.10.26"), None, "a two-digit year is not a date");
        assert_eq!(iso_day("31.13.2026"), None);
    }

    #[test]
    fn actions_nothing_performs_are_not_offered() {
        for (kind, text) in [(Kind::Phone, "555 1234"), (Kind::Flight, "FI 552"), (Kind::Address, "Laugavegur 1")] {
            let rows = offers(text, &[kind], 8);
            assert!(rows.iter().all(|o| o.title.starts_with("Copy ")), "{kind:?}: {rows:?}");
        }
        assert!(offers("FI 552", &[Kind::Flight], 8).is_empty(), "Show Flight Status has no route and no copy action");
        assert_eq!(route(Kind::Phone, "Call", "555 1234"), None);
    }

    #[test]
    fn the_panel_lists_at_most_max_rows() {
        let text = "https://a.is https://b.is https://c.is";
        assert_eq!(offers(text, &ALL_KINDS, 3).len(), 3);
        assert!(offers(text, &ALL_KINDS, 0).is_empty());
    }

    #[test]
    fn no_text_gives_no_suggestions() {
        assert!(suggest("nothing to see here", &[Kind::Link, Kind::Email]).is_empty());
        assert!(suggest("", &[Kind::Link]).is_empty());
    }
}
