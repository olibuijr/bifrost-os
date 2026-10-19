//! The Browser's use of the SDK web bridge (`bifrost_app::web_bridge`, `content_rule_lists`): the content blockers it
//! applies to every tab, and the `control` message handler pages can post to.
//!
//! * Content blockers: lists compiled with `browser.content_rules` are kept in `<profile>/content-rules/` and applied
//!   when the engine starts and whenever the set changes. The engine blocks requests and navigations itself;
//!   `css-display-none` runs as a page script.
//! * Page messages: a page calls `window.bifrost.messageHandlers.control.postMessage(value)`; the engine forwards it,
//!   `Bridge::on_event` hands it to the handler, and `browser.messages` reads the log. This is the path an app
//!   (or a test) uses to hear from a page without polling `browser.eval`.

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::path::Path;
use std::rc::Rc;

use bifrost_app::content_rule_lists::{ContentRuleList, ContentRuleListStore, ContentRuleLists, Request, ResourceType};
use bifrost_app::web_config::{ScriptMessage, ScriptMessageHandler, WebViewConfiguration};
use bifrost_app::webview::{WebProcessPool, WebViewEvent, WebViewId};
use bifrost_core::json::Json;

use crate::control::{num, obj, text};

/// Messages kept in the log.
const LOG_CAP: usize = 500;
/// The handler name pages post to.
pub const HANDLER: &str = "control";

/// One logged page message.
#[derive(Clone, Debug, PartialEq)]
pub struct Logged {
    pub seq: u64,
    pub view: WebViewId,
    pub message: ScriptMessage,
}

struct Recorder {
    log: Rc<RefCell<VecDeque<Logged>>>,
    view: Rc<Cell<WebViewId>>,
    seq: Rc<Cell<u64>>,
}

impl ScriptMessageHandler for Recorder {
    fn did_receive(&mut self, message: &ScriptMessage) {
        let seq = self.seq.get() + 1;
        self.seq.set(seq);
        let mut log = self.log.borrow_mut();
        if log.len() >= LOG_CAP {
            log.pop_front();
        }
        log.push_back(Logged { seq, view: self.view.get(), message: message.clone() });
    }
}

/// Content rule lists and the page message log of one Browser.
pub struct Bridge {
    pub config: WebViewConfiguration,
    pub lists: ContentRuleLists,
    log: Rc<RefCell<VecDeque<Logged>>>,
    view: Rc<Cell<WebViewId>>,
}

impl Default for Bridge {
    fn default() -> Self {
        let (log, view, seq) = (Rc::new(RefCell::new(VecDeque::new())), Rc::new(Cell::new(0)), Rc::new(Cell::new(0)));
        let mut config = WebViewConfiguration::new();
        let _ = config.user_content_controller.add(Box::new(Recorder { log: log.clone(), view: view.clone(), seq }), HANDLER);
        Bridge { config, lists: ContentRuleLists::new(), log, view }
    }
}

fn store_of(profile: &Path) -> ContentRuleListStore {
    ContentRuleListStore::in_profile(profile)
}

impl Bridge {
    /// Applies the message bridge and every stored rule list to a pool that just started.
    pub fn attach(&mut self, pool: &mut WebProcessPool, profile: &Path) {
        let _ = pool.install_configuration(&self.config);
        let store = store_of(profile);
        self.lists.remove_all();
        for id in store.available_identifiers() {
            if let Some(list) = store.look_up(&id) {
                self.lists.add(list);
            }
        }
        if !self.lists.identifiers().is_empty() {
            pool.set_content_rules(&self.lists);
        }
    }

    /// Routes a page message event to the handler. True when it was one.
    pub fn on_event(&mut self, ev: &WebViewEvent) -> bool {
        match ev {
            WebViewEvent::ScriptMessage { id, message } => {
                self.view.set(*id);
                self.config.user_content_controller.deliver(message)
            }
            _ => false,
        }
    }

    /// `browser.messages`: the logged messages after `since_seq`, optionally clearing the log.
    pub fn messages_json(&mut self, since_seq: Option<u64>, clear: bool) -> Json {
        let after = since_seq.unwrap_or(0);
        let rows: Vec<Json> = self
            .log
            .borrow()
            .iter()
            .filter(|m| m.seq > after)
            .map(|m| {
                obj(vec![
                    ("seq", num(m.seq as f64)),
                    ("tab", num(m.view as f64)),
                    ("name", text(&m.message.name)),
                    ("body", text(&m.message.body)),
                    ("main_frame", Json::Bool(m.message.is_main_frame)),
                ])
            })
            .collect();
        if clear {
            self.log.borrow_mut().clear();
        }
        obj(vec![("messages", Json::Arr(rows))])
    }

    fn lists_json(&self) -> Json {
        let rows = self.lists.rule_counts().into_iter().map(|(id, n)| obj(vec![("identifier", text(id)), ("rules", num(n as f64))])).collect();
        obj(vec![("lists", Json::Arr(rows)), ("rules", num(self.lists.rule_set().len() as f64))])
    }

    /// `browser.content_rules`. `pool` is the running engine, when there is one.
    #[allow(clippy::too_many_arguments)]
    pub fn content_rules(
        &mut self,
        pool: Option<&mut WebProcessPool>,
        profile: &Path,
        op: &str,
        identifier: Option<&str>,
        rules: Option<&str>,
        url: Option<&str>,
        resource: Option<&str>,
        top_url: Option<&str>,
    ) -> Result<Json, String> {
        let store = store_of(profile);
        let mut changed = false;
        match op {
            "list" => {}
            "compile" => {
                let list: ContentRuleList = store.compile(identifier.unwrap_or(""), rules.unwrap_or("")).map_err(|e| e.to_string())?;
                self.lists.add(list);
                changed = true;
            }
            "remove" => {
                let id = identifier.unwrap_or("");
                let stored = store.remove(id);
                let used = self.lists.remove(id);
                if !stored && !used {
                    return Err(format!("no content rule list named {id:?}"));
                }
                changed = true;
            }
            "clear" => {
                for id in store.available_identifiers() {
                    store.remove(&id);
                }
                self.lists.remove_all();
                changed = true;
            }
            "test" => {
                let rt = match resource {
                    None => ResourceType::Document,
                    Some(name) => ResourceType::from_name(name).ok_or_else(|| format!("unknown resource {name:?}"))?,
                };
                let u = url.unwrap_or("");
                let v = self.lists.evaluate(&Request { url: u, resource: rt, top_url });
                return Ok(obj(vec![
                    ("url", text(u)),
                    ("block", Json::Bool(v.block)),
                    ("block_cookies", Json::Bool(v.block_cookies)),
                    ("make_https", Json::Bool(v.make_https)),
                    ("css_selectors", Json::Arr(v.css_selectors.iter().map(|s| text(s)).collect())),
                    ("notifications", Json::Arr(v.notifications.iter().map(|s| text(s)).collect())),
                ]));
            }
            _ => return Err("unknown op".into()),
        }
        let mut dropped = 0;
        if changed {
            if let Some(p) = pool {
                dropped = p.set_content_rules(&self.lists);
            }
        }
        let mut out = self.lists_json();
        if let Json::Obj(fields) = &mut out {
            fields.push(("available".into(), Json::Arr(store.available_identifiers().iter().map(|s| text(s)).collect())));
            if dropped > 0 {
                fields.push(("dropped_rules".into(), num(dropped as f64)));
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fresh empty directory under the system temp directory, removed when dropped.
    struct TempDir(std::path::PathBuf);
    impl TempDir {
        fn new(tag: &str) -> TempDir {
            let dir = std::env::temp_dir().join(format!("bifrost-browser-{tag}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            TempDir(dir)
        }
        fn path(&self) -> &Path {
            &self.0
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn rules(pattern: &str) -> String {
        format!(r#"[{{"trigger":{{"url-filter":"{pattern}"}},"action":{{"type":"block"}}}}]"#)
    }

    #[test]
    fn a_page_message_is_logged_with_its_tab_and_read_back_once() {
        let mut b = Bridge::default();
        let post = |id, name: &str, body: &str| WebViewEvent::ScriptMessage { id, message: ScriptMessage { name: name.into(), body: body.into(), is_main_frame: true } };
        assert!(b.on_event(&post(4, HANDLER, "{\"hello\":1}")));
        assert!(!b.on_event(&post(4, "other", "1")), "nobody registered `other`");
        assert!(!b.on_event(&WebViewEvent::LoadFinished(4)));
        assert!(b.on_event(&post(9, HANDLER, "\"two\"")));
        let all = b.messages_json(None, false);
        let rows = all.get("messages").and_then(Json::as_array).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].get("tab").and_then(Json::number), Some(4.0));
        assert_eq!(rows[0].get("body").and_then(Json::as_str), Some("{\"hello\":1}"));
        assert_eq!(rows[1].get("tab").and_then(Json::number), Some(9.0));
        let after = b.messages_json(Some(1), true);
        assert_eq!(after.get("messages").and_then(Json::as_array).unwrap().len(), 1);
        assert!(b.messages_json(None, false).get("messages").and_then(Json::as_array).unwrap().is_empty(), "cleared");
    }

    #[test]
    fn the_log_keeps_the_newest_messages() {
        let mut b = Bridge::default();
        for i in 0..(LOG_CAP + 5) {
            b.on_event(&WebViewEvent::ScriptMessage { id: 1, message: ScriptMessage { name: HANDLER.into(), body: i.to_string(), is_main_frame: true } });
        }
        let rows = b.messages_json(None, false);
        let rows = rows.get("messages").and_then(Json::as_array).unwrap();
        assert_eq!(rows.len(), LOG_CAP);
        assert_eq!(rows[0].get("body").and_then(Json::as_str), Some("5"));
    }

    #[test]
    fn compiled_lists_are_stored_applied_tested_and_removed() {
        let dir = TempDir::new("rules");
        let mut b = Bridge::default();
        let out = b.content_rules(None, dir.path(), "compile", Some("ads"), Some(&rules("ads")), None, None, None).unwrap();
        assert_eq!(out.get("rules").and_then(Json::number), Some(1.0));
        assert_eq!(out.get("available").and_then(Json::as_array).map(|a| a.len()), Some(1));
        let hit = b.content_rules(None, dir.path(), "test", None, None, Some("https://x.example/ads/1.png"), Some("image"), Some("https://page.example/")).unwrap();
        assert_eq!(hit.get("block"), Some(&Json::Bool(true)));
        let miss = b.content_rules(None, dir.path(), "test", None, None, Some("https://x.example/ok.png"), Some("image"), None).unwrap();
        assert_eq!(miss.get("block"), Some(&Json::Bool(false)));
        assert!(b.content_rules(None, dir.path(), "test", None, None, Some("u"), Some("telepathy"), None).is_err());
        // a broken list is refused and changes nothing
        assert!(b.content_rules(None, dir.path(), "compile", Some("bad"), Some("[{}]"), None, None, None).is_err());
        assert_eq!(b.lists.identifiers(), vec!["ads"]);
        // the list survives a restart: a fresh bridge finds it in the profile store
        let store = ContentRuleListStore::in_profile(dir.path());
        assert!(store.look_up("ads").is_some());
        assert!(b.content_rules(None, dir.path(), "remove", Some("ads"), None, None, None, None).is_ok());
        assert!(b.content_rules(None, dir.path(), "remove", Some("ads"), None, None, None, None).is_err(), "already gone");
        assert!(store.look_up("ads").is_none());
        assert!(b.content_rules(None, dir.path(), "compile", Some("one"), Some(&rules("a")), None, None, None).is_ok());
        let cleared = b.content_rules(None, dir.path(), "clear", None, None, None, None, None).unwrap();
        assert_eq!(cleared.get("rules").and_then(Json::number), Some(0.0));
        assert!(store.available_identifiers().is_empty());
    }
}
