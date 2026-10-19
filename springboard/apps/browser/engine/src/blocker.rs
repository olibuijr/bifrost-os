//! Content blockers, navigation policy and script messages of the engine (app side: `bifrost_app::web_bridge`,
//! `bifrost_app::content_rule_lists`).
//!
//! * Content blockers: the app sends its rules as text (`rules-reset`, `rules-add TEXT`, `rules-commit`; the text is
//!   `rule_match::RuleSet::to_wire`, the same source file the SDK compiles). Every web resource load
//!   (`WebViewDelegate::load_web_resource`) and every navigation (`request_navigation`) is judged against them:
//!   `block` cancels the load, `make-https` turns an `http` navigation into the `https` one. `css-display-none` is a
//!   page script the app installs; `block-cookies` and `notify` have no hook in Servo 0.7.0 and are not applied here.
//! * Navigation policy (`navpolicy on`): a navigation the rules let through is held and announced as
//!   `navreq ID TOKEN URL`; the app answers `nav-reply ID TOKEN allow|cancel|download`. A held request that is dropped
//!   (too many pending, the view closed) is allowed by Servo's `Drop`.
//! * Script messages: a console line `@@bifrost-msg NAME FLAG BODY` becomes `scriptmsg ID FLAG NAME BODY`.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;

use servo::{NavigationRequest, WebResourceLoad, WebResourceResponse};
use url::Url;

use crate::rule_match::{Request, ResourceType, RuleSet};

/// Held navigation requests at most.
const MAX_HELD: usize = 64;
/// Largest script message body forwarded, bytes.
const MAX_MESSAGE: usize = 60 * 1024;
/// Rules the engine keeps.
const MAX_RULES: usize = 150_000;
/// What the page-side poster writes before a message.
pub const MESSAGE_PREFIX: &str = "@@bifrost-msg ";

thread_local! {
    static RULES: RefCell<RuleSet> = RefCell::new(RuleSet::default());
    static PENDING_RULES: RefCell<String> = const { RefCell::new(String::new()) };
    static ASK_APP: Cell<bool> = const { Cell::new(false) };
    static HELD: RefCell<HashMap<u32, (u32, NavigationRequest)>> = RefCell::new(HashMap::new());
}

pub fn rules_reset() {
    PENDING_RULES.with(|p| p.borrow_mut().clear());
}

/// One chunk of rule lines (already unescaped).
pub fn rules_add(text: &str) {
    PENDING_RULES.with(|p| {
        let mut p = p.borrow_mut();
        if p.len() < 64 * 1024 * 1024 {
            p.push_str(text);
            p.push('\n');
        }
    });
}

/// Compiles the chunks received since `rules_reset` and makes them the active rules. Returns how many rules.
pub fn rules_commit() -> usize {
    let text = PENDING_RULES.with(|p| std::mem::take(&mut *p.borrow_mut()));
    let mut set = RuleSet::from_wire(&text);
    set.rules.truncate(MAX_RULES);
    let n = set.len();
    RULES.with(|r| *r.borrow_mut() = set);
    n
}

pub fn set_ask_app(on: bool) {
    ASK_APP.with(|a| a.set(on));
}

/// Judges a web resource load; a blocked one is cancelled (a network error for the page), the rest continue.
/// `top` is the URL of the page the load belongs to.
pub fn filter_load(load: WebResourceLoad, top: Option<String>) {
    let blocked = RULES.with(|r| {
        let rules = r.borrow();
        if rules.is_empty() {
            return false;
        }
        let req = load.request();
        let destination = format!("{:?}", req.destination);
        let (resource, top_url) = if req.is_for_main_frame { (ResourceType::Document, None) } else { (ResourceType::from_destination(&destination), top.as_deref()) };
        rules.evaluate(&Request { url: req.url.as_str(), resource, top_url }).block
    });
    if blocked {
        let url = load.request().url.clone();
        load.intercept(WebResourceResponse::new(url)).cancel();
    }
}

/// What to do with a navigation, by the rules alone.
pub enum Verdict {
    Pass,
    Block,
    /// Cancel this navigation and load the `https` URL instead.
    Https(Url),
}

pub fn judge_navigation(url: &Url) -> Verdict {
    RULES.with(|r| {
        let rules = r.borrow();
        if rules.is_empty() {
            return Verdict::Pass;
        }
        let v = rules.evaluate(&Request { url: url.as_str(), resource: ResourceType::Document, top_url: None });
        if v.block {
            return Verdict::Block;
        }
        if v.make_https {
            if let Some(Ok(upgraded)) = crate::rule_match::upgrade_to_https(url.as_str()).map(|u| Url::parse(&u)) {
                return Verdict::Https(upgraded);
            }
        }
        Verdict::Pass
    })
}

pub fn ask_app() -> bool {
    ASK_APP.with(|a| a.get())
}

/// Keeps a request until the app answers. The oldest is dropped (and so allowed) when too many wait.
pub fn hold(token: u32, view: u32, request: NavigationRequest) {
    HELD.with(|h| {
        let mut h = h.borrow_mut();
        if h.len() >= MAX_HELD {
            if let Some(oldest) = h.keys().min().copied() {
                if let Some((_, request)) = h.remove(&oldest) { request.deny(); }
            }
        }
        h.insert(token, (view, request));
    });
}

/// The app's answer. Returns the URL when the request became a download.
pub fn answer(token: u32, word: &str) -> Option<String> {
    let (_, request) = HELD.with(|h| h.borrow_mut().remove(&token))?;
    match word {
        "cancel" => {
            request.deny();
            None
        }
        "download" => {
            let url = request.url.to_string();
            request.deny();
            Some(url)
        }
        _ => {
            request.allow();
            None
        }
    }
}

/// Allows every held request (the policy was switched off).
pub fn release_all() {
    let all: Vec<NavigationRequest> = HELD.with(|h| h.borrow_mut().drain().map(|(_, (_, request))| request).collect());
    for r in all {
        r.allow();
    }
}

/// Shutdown cancels pending navigations instead of starting new loads while tearing down Servo.
pub fn cancel_all() {
    let all: Vec<NavigationRequest> = HELD.with(|held| held.borrow_mut().drain().map(|(_, (_, request))| request).collect());
    for request in all {
        request.deny();
    }
}

/// Closing one view must not leave its responders alive or affect another tab's policy decision.
pub fn cancel_view(view: u32) {
    let requests: Vec<NavigationRequest> = HELD.with(|held| {
        let mut held = held.borrow_mut();
        let tokens: Vec<u32> = held.iter().filter(|(_, (owner, _))| *owner == view).map(|(token, _)| *token).collect();
        tokens.into_iter().filter_map(|token| held.remove(&token).map(|(_, request)| request)).collect()
    });
    for request in requests { request.deny(); }
}

/// Escapes `\n`, `\r` and `\\`, with no length cap (the line protocol of the SDK's `escape_full`).
fn escape_all(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 16);
    for c in s.chars() {
        match c {
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\\' => out.push_str("\\\\"),
            c => out.push(c),
        }
    }
    out
}

/// The `scriptmsg` line for the console text after [`MESSAGE_PREFIX`] (`NAME FLAG BODY`). None for a malformed
/// message or a body over 60 KB.
pub fn script_message_line(view: u32, rest: &str) -> Option<String> {
    let mut f = rest.splitn(3, ' ');
    let (name, flag, body) = (f.next()?, f.next()?, f.next().unwrap_or("null"));
    let valid = name.chars().next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_' || c == '$') && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$');
    if !valid || body.len() > MAX_MESSAGE || !matches!(flag, "0" | "1") {
        return None;
    }
    Some(format!("scriptmsg {view} {flag} {name} {}", escape_all(body)))
}

/// A cookie records line for the SDK: `name U+1 value U+1 domain U+1 path U+1 expires|- U+1 secure U+1 httponly`.
pub fn cookie_record(name: &str, value: &str, domain: &str, path: &str, expires: Option<i64>, secure: bool, http_only: bool) -> Option<String> {
    let clean = |s: &str| !s.contains(['\u{1}', '\u{2}', '\n', '\r']);
    if ![name, value, domain, path].iter().all(|s| clean(s)) {
        return None;
    }
    let exp = expires.map_or("-".to_string(), |e| e.to_string());
    Some(format!("{name}\u{1}{value}\u{1}{domain}\u{1}{path}\u{1}{exp}\u{1}{}\u{1}{}", secure as u8, http_only as u8))
}

/// The `cookies` reply line.
pub fn cookies_line(view: u32, token: &str, records: &[String]) -> String {
    format!("cookies {view} {token} {}", escape_all(&records.join("\u{2}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rules_apply_after_commit_and_clear_with_an_empty_commit() {
        rules_reset();
        rules_add("block\t0\tads\t\t\t\t\t\t");
        rules_add("https\t0\t^http://shop\\.example/\t\t\t\t\t\t");
        assert_eq!(rules_commit(), 2);
        assert!(matches!(judge_navigation(&Url::parse("http://x.example/ads/1").unwrap()), Verdict::Block));
        match judge_navigation(&Url::parse("http://shop.example/cart").unwrap()) {
            Verdict::Https(u) => assert_eq!(u.as_str(), "https://shop.example/cart"),
            _ => panic!("expected an https upgrade"),
        }
        assert!(matches!(judge_navigation(&Url::parse("https://fine.example/").unwrap()), Verdict::Pass));
        rules_reset();
        assert_eq!(rules_commit(), 0);
        assert!(matches!(judge_navigation(&Url::parse("http://x.example/ads/1").unwrap()), Verdict::Pass));
    }

    #[test]
    fn script_messages_become_protocol_lines() {
        assert_eq!(script_message_line(3, "auth 1 {\"a\":\"x\\ny\"}").as_deref(), Some("scriptmsg 3 1 auth {\"a\":\"x\\\\ny\"}"));
        assert_eq!(script_message_line(3, "log 0").as_deref(), Some("scriptmsg 3 0 log null"));
        assert_eq!(script_message_line(3, "bad-name 1 x"), None);
        assert_eq!(script_message_line(3, "log 2 x"), None);
        assert_eq!(script_message_line(3, &format!("log 1 {}", "x".repeat(MAX_MESSAGE + 1))), None);
    }

    #[test]
    fn cookie_records_refuse_separators() {
        assert_eq!(cookie_record("a", "b", "h.example", "/", Some(5), true, false).as_deref(), Some("a\u{1}b\u{1}h.example\u{1}/\u{1}5\u{1}1\u{1}0"));
        assert_eq!(cookie_record("a", "b", "h.example", "/", None, false, true).as_deref(), Some("a\u{1}b\u{1}h.example\u{1}/\u{1}-\u{1}0\u{1}1"));
        assert_eq!(cookie_record("a\u{1}", "b", "h", "/", None, false, false), None);
        let line = cookies_line(2, "9", &["x".to_string(), "y".to_string()]);
        assert_eq!(line, "cookies 2 9 x\u{2}y");
    }
}
