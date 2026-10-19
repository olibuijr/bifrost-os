//! The glue between the web view pool and the app-side models: page scripts and messages, navigation decisions,
//! content blockers, cookies.
//!
//! The models (`web_config`, `web_nav`, `content_rule_lists`, `web_data`) are pure state. This module speaks the engine
//! protocol for them (`apps/browser/engine`, see its header for the verbs):
//!
//! app -> engine
//!   userscript NAME JS            keep JS (escaped) as the document-start script NAME in every web view; empty JS removes it
//!   rules-reset | rules-add TEXT | rules-commit    content blocker rules, `RuleSet::to_wire` text in chunks
//!   navpolicy on|off              ask the app before every navigation (`navreq`)
//!   nav-reply ID TOKEN allow|cancel|download
//!   cookies ID TOKEN              the cookies the engine holds for the page URL of view ID
//!   clear-cookies                 drop every cookie of the engine
//! engine -> app
//!   scriptmsg ID FLAG NAME BODY   a page posted BODY (JSON text, escaped) to the handler NAME; FLAG 1 for the main frame
//!   navreq ID TOKEN URL           a navigation is about to start; answer with `nav-reply` (an unanswered one is allowed)
//!   cookies ID TOKEN RECORDS      records `name U+1 value U+1 domain U+1 path U+1 expires|- U+1 secure U+1 httponly` split by U+2
//!   download ID URL               the navigation was turned into a download (answer `download`)
//!
//! A page reaches an app handler as `window.bifrost.messageHandlers.<name>.postMessage(body)`: the bootstrap script
//! of [`crate::web_config::UserContentController`] writes a line to the console that starts with
//! [`SCRIPT_MESSAGE_PREFIX`]; the engine turns that line into `scriptmsg` (and does not forward it as console text),
//! and [`route_script_messages`] hands it to the handler registered under the name.

use std::collections::HashMap;

use crate::content_rule_lists::{hide_script, ContentRuleLists};
use crate::web_config::{valid_handler_name, InjectionTime, ScriptMessage, UserContentController, UserScript, WebViewConfiguration};
use crate::web_data::{Cookie, CookieStore};
use crate::web_nav::{NavigationActionInfo, NavigationActionPolicy, NavigationDelegate, NavigationType, PolicyBroker, DECISION_TIMEOUT_MS};
use crate::webview::{unescape_line, WebProcessPool, WebViewEvent, WebViewId};
use crate::webview_dev::escape_full;

/// What the page-side poster writes in front of a message: `@@bifrost-msg NAME FLAG BODY`.
pub const SCRIPT_MESSAGE_PREFIX: &str = "@@bifrost-msg ";

/// The page-side function `(name, jsonText)` the bootstrap script calls. Pass it as `post` to
/// `UserContentController::bootstrap_script` (`WebViewConfiguration::injected_scripts`).
pub const POST_EXPRESSION: &str = "(function(n,j){console.debug(\"@@bifrost-msg \"+n+\" \"+(window.top===window?1:0)+\" \"+(j===undefined?\"null\":j))})";

/// Longest script the engine takes in one line, bytes (it cuts commands at 256 KB).
pub const MAX_SCRIPT_BYTES: usize = 120 * 1024;
/// Longest text of one `rules-add` chunk before escaping, bytes.
pub const RULES_CHUNK_BYTES: usize = 100 * 1024;

/// Reads the tail of `scriptmsg ID FLAG NAME BODY` (after the id). None when the name is not a valid handler name.
pub fn parse_script_message(tail: &str) -> Option<ScriptMessage> {
    let mut f = tail.splitn(3, ' ');
    let main = f.next()? == "1";
    let name = f.next()?;
    if !valid_handler_name(name) {
        return None;
    }
    Some(ScriptMessage { name: name.to_string(), body: unescape_line(f.next().unwrap_or("null")), is_main_frame: main })
}

/// Reads a console text that carries a script message (for an engine that forwards it as console output). The
/// console form is `NAME FLAG BODY` with the body as written, not escaped.
pub fn script_message_from_console(text: &str) -> Option<ScriptMessage> {
    let rest = text.strip_prefix(SCRIPT_MESSAGE_PREFIX)?;
    let mut f = rest.splitn(3, ' ');
    let (name, flag, body) = (f.next()?, f.next()?, f.next().unwrap_or("null"));
    if !valid_handler_name(name) {
        return None;
    }
    Some(ScriptMessage { name: name.to_string(), body: body.to_string(), is_main_frame: flag == "1" })
}

/// Hands every `ScriptMessage` event to the handler registered under its name. Returns how many had a handler.
pub fn route_script_messages(controller: &mut UserContentController, events: &[WebViewEvent]) -> usize {
    events.iter().filter(|e| matches!(e, WebViewEvent::ScriptMessage { message, .. } if controller.deliver(message))).count()
}

/// A user script as the engine runs it: in its own function scope, main frame only when asked, and for
/// `AtDocumentEnd` after the document is parsed.
pub fn wrap_user_script(script: &UserScript) -> String {
    let guard = if script.for_main_frame_only { "if(window.top!==window)return;" } else { "" };
    match script.injection_time {
        InjectionTime::AtDocumentStart => format!("(function(){{{guard}\n{}\n}})();", script.source),
        InjectionTime::AtDocumentEnd => format!(
            "(function(){{{guard}var run=function(){{\n{}\n}};if(document.readyState==='loading'){{document.addEventListener('DOMContentLoaded',run)}}else{{run()}}}})();",
            script.source
        ),
    }
}

/// All scripts of a configuration as one engine script, in order.
pub fn bundle_scripts(scripts: &[UserScript]) -> String {
    scripts.iter().map(wrap_user_script).collect::<Vec<_>>().join("\n")
}

/// The engine verb for a navigation policy.
pub fn policy_word(p: NavigationActionPolicy) -> &'static str {
    match p {
        NavigationActionPolicy::Allow => "allow",
        NavigationActionPolicy::Cancel => "cancel",
        NavigationActionPolicy::Download => "download",
    }
}

/// Reads the tail of `navreq ID TOKEN URL` (after the id).
pub fn parse_navigation_request(tail: &str) -> Option<(u32, String)> {
    let (token, url) = tail.split_once(' ')?;
    Some((token.parse().ok()?, url.to_string()))
}

/// The file name a download of `url` suggests: the last path segment, or `download`.
pub fn suggested_filename(url: &str) -> String {
    let path = url.split(['?', '#']).next().unwrap_or("");
    let path = path.split_once("://").map_or(path, |(_, r)| r.split_once('/').map_or("", |(_, p)| p));
    match path.rsplit('/').find(|s| !s.is_empty()) {
        Some(s) => s.to_string(),
        None => "download".to_string(),
    }
}

/// The decision of a delegate for one `NavigationRequested` event. The engine reports no frame or cause, so the
/// request is described as a main-frame navigation of unknown type.
pub fn decide_navigation(delegate: &mut dyn NavigationDelegate, url: &str) -> NavigationActionPolicy {
    delegate.decide_policy_for_action(&NavigationActionInfo { url: url.to_string(), navigation_type: NavigationType::Other, target_is_main_frame: true })
}

/// Asks `delegate` about every `NavigationRequested` event and returns the answers to send, in order.
pub fn navigation_decisions(delegate: &mut dyn NavigationDelegate, events: &[WebViewEvent]) -> Vec<(WebViewId, u32, NavigationActionPolicy)> {
    events
        .iter()
        .filter_map(|e| match e {
            WebViewEvent::NavigationRequested { id, token, url } => Some((*id, *token, decide_navigation(delegate, url))),
            _ => None,
        })
        .collect()
}

/// Navigation decisions the app answers later (a sheet, a lookup). Each waits at most
/// [`DECISION_TIMEOUT_MS`]; one that is not answered then resolves to allow, so a slow app never stalls a page.
#[derive(Debug, Default)]
pub struct NavigationRouter {
    broker: PolicyBroker,
    views: HashMap<u32, WebViewId>,
}

impl NavigationRouter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Records the `NavigationRequested` events of `events` as open decisions at time `now` (milliseconds).
    pub fn open(&mut self, events: &[WebViewEvent], now: u64) {
        for e in events {
            if let WebViewEvent::NavigationRequested { id, token, .. } = e {
                self.broker.request(*token, now, DECISION_TIMEOUT_MS);
                self.views.insert(*token, *id);
            }
        }
    }

    /// The app's answer. Returns the view and the policy to send, or None when the decision is unknown, answered
    /// already or expired.
    pub fn answer(&mut self, token: u32, policy: NavigationActionPolicy) -> Option<(WebViewId, NavigationActionPolicy)> {
        if !self.broker.answer(token, policy) {
            return None;
        }
        let view = self.views.remove(&token)?;
        Some((view, self.broker.take(token)?))
    }

    /// Decisions past their deadline at `now`, resolved to allow: `(view, token, policy)`.
    pub fn expire(&mut self, now: u64) -> Vec<(WebViewId, u32, NavigationActionPolicy)> {
        self.broker
            .expire(now)
            .into_iter()
            .filter_map(|t| Some((self.views.remove(&t)?, t, self.broker.take(t)?)))
            .collect()
    }

    pub fn pending(&self) -> usize {
        self.views.len()
    }
}

/// Splits `text` into chunks of whole lines, each at most `max` bytes; a line longer than `max` is dropped and
/// counted. Returns `(chunks, dropped_lines)`.
pub fn chunk_lines(text: &str, max: usize) -> (Vec<String>, usize) {
    let mut chunks = Vec::new();
    let mut cur = String::new();
    let mut dropped = 0;
    for line in text.lines() {
        if line.len() > max {
            dropped += 1;
            continue;
        }
        if !cur.is_empty() && cur.len() + 1 + line.len() > max {
            chunks.push(std::mem::take(&mut cur));
        }
        if !cur.is_empty() {
            cur.push('\n');
        }
        cur.push_str(line);
    }
    if !cur.is_empty() {
        chunks.push(cur);
    }
    (chunks, dropped)
}

/// Reads the tail of `cookies ID TOKEN RECORDS` (after the id).
pub fn parse_cookies(tail: &str) -> Option<(u32, Vec<Cookie>)> {
    let (token, records) = tail.split_once(' ').unwrap_or((tail, ""));
    let token = token.parse().ok()?;
    let cookies = records
        .split('\u{2}')
        .filter(|r| !r.is_empty())
        .filter_map(|r| {
            let f: Vec<&str> = r.split('\u{1}').collect();
            if f.len() != 7 {
                return None;
            }
            let mut c = Cookie::new(f[0], f[1], f[2]);
            c.path = if f[3].starts_with('/') { f[3].to_string() } else { "/".to_string() };
            c.expires = f[4].parse().ok();
            c.secure = f[5] == "1";
            c.http_only = f[6] == "1";
            Some(c)
        })
        .collect();
    Some((token, cookies))
}

/// Makes `store` agree with the engine's cookies for `url`: engine cookies are added or updated, and cookies the
/// store would send to `url` that the engine no longer has are removed. Returns `(set, removed)`.
pub fn reconcile_cookies(store: &mut CookieStore, url: &str, engine: &[Cookie], now: u64) -> (usize, usize) {
    let same = |a: &Cookie, b: &Cookie| a.name == b.name && a.domain.trim_start_matches('.').eq_ignore_ascii_case(b.domain.trim_start_matches('.')) && a.path == b.path;
    let stale: Vec<Cookie> = store.cookies_for(url, now).into_iter().filter(|c| !engine.iter().any(|e| same(c, e))).collect();
    let removed = stale.iter().filter(|c| store.delete(c)).count();
    let set = engine.iter().filter(|c| store.set_cookie((*c).clone(), now)).count();
    (set, removed)
}

/// The page script that sets `cookie` for the page it runs in (`document.cookie`). None for a cookie a script may
/// not set (`HttpOnly`) or one that cannot be written as a cookie string. A dot-prefixed domain is sent as a
/// domain cookie; an expiry in the past deletes.
pub fn cookie_set_script(cookie: &Cookie, now: u64) -> Option<String> {
    if cookie.http_only {
        return None;
    }
    let bad = |s: &str| s.chars().any(|c| c.is_control() || c == ';');
    if cookie.name.is_empty() || bad(&cookie.name) || bad(&cookie.value) || cookie.name.contains(['=', ' ']) || bad(&cookie.path) || bad(&cookie.domain) {
        return None;
    }
    let mut s = format!("{}={}; path={}", cookie.name, cookie.value, cookie.path);
    if cookie.domain.starts_with('.') {
        s.push_str(&format!("; domain={}", cookie.domain));
    }
    if let Some(e) = cookie.expires {
        s.push_str(&format!("; max-age={}", e.saturating_sub(now)));
    }
    if cookie.secure {
        s.push_str("; secure");
    }
    Some(format!("document.cookie={}", bifrost_core::json::quote(&s)))
}

impl WebProcessPool {
    /// Applies a configuration's page scripts (the message bridge first, then the app's own) to every web view,
    /// replacing what an earlier call installed. Pages loaded afterwards run them. `Err` when the scripts are too
    /// large for one engine command.
    pub fn install_configuration(&mut self, config: &WebViewConfiguration) -> Result<(), String> {
        let js = bundle_scripts(&config.injected_scripts(POST_EXPRESSION));
        if js.len() > MAX_SCRIPT_BYTES {
            return Err(format!("the user scripts are {} bytes; the limit is {MAX_SCRIPT_BYTES}", js.len()));
        }
        self.send_raw(&format!("userscript config {}", escape_full(&js)));
        Ok(())
    }

    /// Removes the scripts `install_configuration` installed.
    pub fn remove_configuration_scripts(&mut self) {
        self.send_raw("userscript config ");
    }

    /// Uses `lists` as the content blockers of every web view, replacing the earlier ones: the engine blocks
    /// requests and navigations itself, and the `css-display-none` rules run as a page script. An empty set clears
    /// them. Returns how many rule lines were too long to send (they are not applied).
    pub fn set_content_rules(&mut self, lists: &ContentRuleLists) -> usize {
        let rules = lists.rule_set();
        let (chunks, dropped) = chunk_lines(&rules.to_wire(), RULES_CHUNK_BYTES);
        self.send_raw("rules-reset");
        for c in chunks {
            self.send_raw(&format!("rules-add {}", escape_full(&c)));
        }
        self.send_raw("rules-commit");
        let hide = hide_script(&rules).filter(|s| s.len() <= MAX_SCRIPT_BYTES).unwrap_or_default();
        self.send_raw(&format!("userscript rules {}", escape_full(&hide)));
        dropped
    }

    /// Whether the engine asks the app before every navigation (`NavigationRequested` events, answered with
    /// `navigation_reply`). Off by default: navigations proceed without a round trip.
    pub fn set_navigation_policy_enabled(&mut self, on: bool) {
        self.send_raw(if on { "navpolicy on" } else { "navpolicy off" });
    }

    /// The answer to a `NavigationRequested` event. An unanswered request is allowed by the engine when the pool
    /// ends the policy (`set_navigation_policy_enabled(false)`) or the request is dropped.
    pub fn navigation_reply(&mut self, id: WebViewId, token: u32, policy: NavigationActionPolicy) {
        self.send_raw(&format!("nav-reply {id} {token} {}", policy_word(policy)));
    }

    /// Asks `delegate` about each `NavigationRequested` event of `events` and sends the answers. Returns how many.
    pub fn answer_navigation_requests(&mut self, delegate: &mut dyn NavigationDelegate, events: &[WebViewEvent]) -> usize {
        let answers = navigation_decisions(delegate, events);
        for (id, token, policy) in &answers {
            self.navigation_reply(*id, *token, *policy);
        }
        answers.len()
    }

    /// Asks for the cookies the engine holds for the page URL of view `id`; they arrive as `WebViewEvent::Cookies`
    /// with the same `token`. Feed them to `reconcile_cookies`.
    pub fn request_cookies(&mut self, id: WebViewId, token: u32) {
        self.send_raw(&format!("cookies {id} {token}"));
    }

    /// Deletes every cookie the engine holds (`removeData` of cookies for all sites).
    pub fn clear_engine_cookies(&mut self) {
        self.send_raw("clear-cookies");
    }

    /// Sets a cookie for the page of view `id` through the page's own cookie jar. False when a page script may not
    /// set it (`HttpOnly`) or it cannot be written; the cookie applies only to the site of that page.
    pub fn set_page_cookie(&mut self, id: WebViewId, cookie: &Cookie, now: u64) -> bool {
        match cookie_set_script(cookie, now) {
            Some(js) => {
                self.eval(id, &js);
                true
            }
            None => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::web_config::ScriptMessageHandler;
    use std::cell::RefCell;
    use std::rc::Rc;

    #[test]
    fn a_posted_message_survives_the_trip_from_the_page_to_the_handler() {
        let cfg = WebViewConfiguration::new();
        let mut c = cfg.user_content_controller;
        struct Rec(Rc<RefCell<Vec<ScriptMessage>>>);
        impl ScriptMessageHandler for Rec {
            fn did_receive(&mut self, m: &ScriptMessage) {
                self.0.borrow_mut().push(m.clone());
            }
        }
        let seen = Rc::new(RefCell::new(Vec::new()));
        c.add(Box::new(Rec(seen.clone())), "auth").unwrap();
        // the bootstrap names the poster and the poster writes the prefix
        let boot = c.bootstrap_script(POST_EXPRESSION);
        assert!(boot.source.contains("h[\"auth\"]={postMessage:function(b){(function(n,j){console.debug(\"@@bifrost-msg \"+n"));
        // what the engine sends for `postMessage({"k":"a b\nc"})` from the main frame
        let line = "1 auth {\"k\":\"a b\\\\nc\"}";
        let m = parse_script_message(line).unwrap();
        assert_eq!(m, ScriptMessage { name: "auth".into(), body: "{\"k\":\"a b\\nc\"}".into(), is_main_frame: true });
        let events = vec![
            WebViewEvent::ScriptMessage { id: 3, message: m },
            WebViewEvent::ScriptMessage { id: 3, message: ScriptMessage { name: "nobody".into(), body: "1".into(), is_main_frame: false } },
            WebViewEvent::LoadFinished(3),
        ];
        assert_eq!(route_script_messages(&mut c, &events), 1);
        assert_eq!(seen.borrow().len(), 1);
        assert_eq!(seen.borrow()[0].name, "auth");
    }

    #[test]
    fn script_message_lines_are_validated() {
        assert_eq!(parse_script_message("0 log \"x\"").map(|m| (m.name, m.is_main_frame, m.body)), Some(("log".into(), false, "\"x\"".into())));
        assert_eq!(parse_script_message("1 bad-name 1"), None);
        assert_eq!(parse_script_message("1"), None);
        assert_eq!(parse_script_message("1 name").map(|m| m.body), Some("null".to_string()));
        // the console form is NAME FLAG BODY
        let m = script_message_from_console("@@bifrost-msg auth 1 {\"a\":1}").unwrap();
        assert_eq!((m.name.as_str(), m.is_main_frame, m.body.as_str()), ("auth", true, "{\"a\":1}"));
        assert_eq!(script_message_from_console("hello world"), None);
        assert_eq!(script_message_from_console("@@bifrost-msg no-good 1 x"), None);
    }

    #[test]
    fn user_scripts_are_scoped_and_timed() {
        let start = wrap_user_script(&UserScript::new("a()", InjectionTime::AtDocumentStart, false));
        assert_eq!(start, "(function(){\na()\n})();");
        let main_only = wrap_user_script(&UserScript::new("b()", InjectionTime::AtDocumentStart, true));
        assert!(main_only.contains("if(window.top!==window)return;"));
        let end = wrap_user_script(&UserScript::new("c() // trailing comment", InjectionTime::AtDocumentEnd, true));
        assert!(end.contains("DOMContentLoaded") && end.contains("document.readyState==='loading'"));
        assert!(end.contains("// trailing comment\n}"), "a line comment cannot swallow the closing brace");
        for js in [&start, &main_only, &end] {
            assert_eq!(js.matches('{').count(), js.matches('}').count(), "{js}");
        }
        let bundle = bundle_scripts(&[UserScript::new("one", InjectionTime::AtDocumentStart, false), UserScript::new("two", InjectionTime::AtDocumentStart, false)]);
        assert!(bundle.find("one").unwrap() < bundle.find("two").unwrap());
    }

    struct Policy;
    impl NavigationDelegate for Policy {
        fn decide_policy_for_action(&mut self, a: &NavigationActionInfo) -> NavigationActionPolicy {
            if a.url.contains("evil") {
                NavigationActionPolicy::Cancel
            } else if a.url.ends_with(".zip") {
                NavigationActionPolicy::Download
            } else {
                NavigationActionPolicy::Allow
            }
        }
    }

    #[test]
    fn the_delegate_decides_each_requested_navigation() {
        let events = vec![
            WebViewEvent::NavigationRequested { id: 1, token: 10, url: "https://ok.example/".into() },
            WebViewEvent::LoadStarted(1),
            WebViewEvent::NavigationRequested { id: 1, token: 11, url: "https://evil.example/".into() },
            WebViewEvent::NavigationRequested { id: 2, token: 12, url: "https://files.example/a.zip".into() },
        ];
        let answers = navigation_decisions(&mut Policy, &events);
        assert_eq!(
            answers,
            vec![(1, 10, NavigationActionPolicy::Allow), (1, 11, NavigationActionPolicy::Cancel), (2, 12, NavigationActionPolicy::Download)]
        );
        assert_eq!(policy_word(NavigationActionPolicy::Download), "download");
        assert_eq!(parse_navigation_request("12 https://a.example/x y"), Some((12, "https://a.example/x y".to_string())));
        assert_eq!(parse_navigation_request("x https://a"), None);
        assert_eq!(parse_navigation_request("12"), None);
    }

    #[test]
    fn slow_decisions_resolve_to_allow_at_the_deadline() {
        let mut r = NavigationRouter::new();
        let events = vec![
            WebViewEvent::NavigationRequested { id: 1, token: 5, url: "https://a/".into() },
            WebViewEvent::NavigationRequested { id: 2, token: 6, url: "https://b/".into() },
        ];
        r.open(&events, 1_000);
        assert_eq!(r.pending(), 2);
        assert_eq!(r.answer(5, NavigationActionPolicy::Cancel), Some((1, NavigationActionPolicy::Cancel)));
        assert_eq!(r.answer(5, NavigationActionPolicy::Allow), None, "answered once");
        assert!(r.expire(1_000 + DECISION_TIMEOUT_MS - 1).is_empty());
        assert_eq!(r.expire(1_000 + DECISION_TIMEOUT_MS), vec![(2, 6, NavigationActionPolicy::Allow)]);
        assert_eq!(r.pending(), 0);
        assert_eq!(r.answer(6, NavigationActionPolicy::Cancel), None, "too late");
    }

    #[test]
    fn download_names_come_from_the_url_path() {
        assert_eq!(suggested_filename("https://h.example/files/report.pdf?x=1#f"), "report.pdf");
        assert_eq!(suggested_filename("https://h.example/dir/"), "dir");
        assert_eq!(suggested_filename("https://h.example/"), "download");
        assert_eq!(suggested_filename("https://h.example"), "download");
    }

    #[test]
    fn rules_are_chunked_by_whole_lines() {
        let (chunks, dropped) = chunk_lines("aaaa\nbbbb\ncc\n", 9);
        assert_eq!((chunks, dropped), (vec!["aaaa\nbbbb".to_string(), "cc".to_string()], 0));
        let (chunks, dropped) = chunk_lines("a\ntoolongline\nb", 5);
        assert_eq!((chunks, dropped), (vec!["a\nb".to_string()], 1));
        assert_eq!(chunk_lines("", 5), (Vec::new(), 0));
    }

    #[test]
    fn engine_cookies_parse_and_reconcile_with_the_store() {
        let rec = |n: &str, v: &str, d: &str, e: &str, s: &str, h: &str| format!("{n}\u{1}{v}\u{1}{d}\u{1}/\u{1}{e}\u{1}{s}\u{1}{h}");
        let tail = format!("42 {}\u{2}{}\u{2}broken", rec("sid", "abc", "shop.example", "-", "1", "1"), rec("theme", "dark", ".shop.example", "5000", "0", "0"));
        let (token, cookies) = parse_cookies(&tail).unwrap();
        assert_eq!(token, 42);
        assert_eq!(cookies.len(), 2, "the broken record is skipped");
        assert!(cookies[0].secure && cookies[0].http_only && cookies[0].expires.is_none());
        assert_eq!(cookies[1].expires, Some(5000));
        assert_eq!(parse_cookies("7").map(|c| c.1.len()), Some(0));
        assert_eq!(parse_cookies("x a"), None);

        let mut store = CookieStore::new();
        let mut old = Cookie::new("gone", "1", "shop.example");
        old.expires = Some(9_999);
        store.set_cookie(old, 100);
        store.set_cookie(Cookie::new("other", "1", "elsewhere.example"), 100);
        let (set, removed) = reconcile_cookies(&mut store, "https://shop.example/cart", &cookies, 100);
        assert_eq!((set, removed), (2, 1));
        let names: Vec<String> = store.get_all_cookies(100).into_iter().map(|c| c.name).collect();
        assert!(names.contains(&"sid".to_string()) && names.contains(&"theme".to_string()));
        assert!(!names.contains(&"gone".to_string()), "the engine dropped it");
        assert!(names.contains(&"other".to_string()), "other sites are untouched");
        // a second pass with the same cookies changes nothing
        assert_eq!(reconcile_cookies(&mut store, "https://shop.example/cart", &cookies, 100), (2, 0));
    }

    #[test]
    fn page_cookies_are_written_as_cookie_strings() {
        let mut c = Cookie::new("theme", "dark", ".shop.example");
        c.expires = Some(1_100);
        c.secure = true;
        c.path = "/app".into();
        assert_eq!(
            cookie_set_script(&c, 1_000).as_deref(),
            Some("document.cookie=\"theme=dark; path=/app; domain=.shop.example; max-age=100; secure\"")
        );
        // host-only and expired
        let mut d = Cookie::new("a", "b", "shop.example");
        d.expires = Some(10);
        assert_eq!(cookie_set_script(&d, 1_000).as_deref(), Some("document.cookie=\"a=b; path=/; max-age=0\""));
        // refused: HttpOnly cannot be set from a script, and unsafe characters cannot be written
        let mut h = Cookie::new("sid", "x", "shop.example");
        h.http_only = true;
        assert_eq!(cookie_set_script(&h, 0), None);
        assert_eq!(cookie_set_script(&Cookie::new("a", "b;c", "x.example"), 0), None);
        assert_eq!(cookie_set_script(&Cookie::new("a=b", "c", "x.example"), 0), None);
        assert_eq!(cookie_set_script(&Cookie::new("", "c", "x.example"), 0), None);
    }

    #[test]
    fn a_configuration_installs_its_bridge_before_its_scripts() {
        let mut cfg = WebViewConfiguration::new();
        cfg.user_content_controller.add_user_script(UserScript::new("app()", InjectionTime::AtDocumentEnd, true));
        let js = bundle_scripts(&cfg.injected_scripts(POST_EXPRESSION));
        assert!(js.find("messageHandlers").unwrap() < js.find("app()").unwrap());
    }
}
