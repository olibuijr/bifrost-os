//! Web content blockers (`ContentRuleList`, `ContentRuleListStore`): JSON rule lists that stop requests, hide page
//! elements, block cookies and upgrade links to HTTPS in a web view.
//!
//! The JSON is the reference's rule list format: an array of `{ "trigger": {...}, "action": {...} }` objects.
//!
//! ```json
//! [{"trigger": {"url-filter": "^https?://.*\\.tracker\\.example/", "resource-type": ["script", "image"],
//!               "load-type": ["third-party"], "unless-domain": ["*friend.example"]},
//!   "action": {"type": "block"}},
//!  {"trigger": {"url-filter": ".*"}, "action": {"type": "css-display-none", "selector": ".ad, #banner"}}]
//! ```
//!
//! Action types: `block`, `block-cookies`, `css-display-none` (needs `selector`), `ignore-previous-rules`,
//! `make-https`, `notify` (needs `notification`). Trigger keys: `url-filter` (required, the pattern language of
//! [`crate::rule_match`]), `url-filter-is-case-sensitive`, `if-domain` / `unless-domain` (lowercase, at most 150,
//! not both), `resource-type`, `load-type` (`first-party` or `third-party`), `if-top-url` / `unless-top-url` (filters,
//! not both). A domain covers its subdomains whether or not it has the `*` prefix.
//!
//! [`ContentRuleListStore`] keeps compiled lists as files in the profile directory (`content-rules/<id>.json`),
//! [`ContentRuleLists`] holds the lists a view uses, and `WebProcessPool::set_content_rules` (in `web_bridge`) sends
//! them to the engine. The engine blocks requests and navigations itself (it never asks the app per request);
//! `css-display-none` is applied by a script generated here ([`hide_script`]) that runs in every document.
//! `notify` is evaluated by [`ContentRuleLists::evaluate`] for apps that judge URLs themselves; the engine does not
//! report notifications.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use bifrost_core::json::{self, Json};

pub use crate::rule_match::{Action, LoadType, Request, ResourceType, Rule, RuleSet, Trigger, UrlFilter, Verdict};

/// Rules one list may hold (the reference content rule list store allows 150,000).
pub const MAX_RULES: usize = 150_000;
/// Entries of one domain list.
pub const MAX_DOMAINS: usize = 150;
/// Longest list JSON the store compiles, bytes.
pub const MAX_JSON_BYTES: usize = 50 * 1024 * 1024;

/// Why a list did not compile (the reference store's compile-failed error).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContentRuleError {
    /// Zero-based index of the rule at fault, when one is.
    pub rule: Option<usize>,
    pub message: String,
}

impl std::fmt::Display for ContentRuleError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.rule {
            Some(i) => write!(f, "rule {i}: {}", self.message),
            None => write!(f, "{}", self.message),
        }
    }
}

fn err(rule: usize, message: impl Into<String>) -> ContentRuleError {
    ContentRuleError { rule: Some(rule), message: message.into() }
}

/// One compiled list (`ContentRuleList`).
#[derive(Clone, Debug, PartialEq)]
pub struct ContentRuleList {
    pub identifier: String,
    pub rules: RuleSet,
}

fn domains(trigger: &Json, key: &str, i: usize) -> Result<Vec<String>, ContentRuleError> {
    let Some(v) = trigger.get(key) else { return Ok(Vec::new()) };
    let items = v.as_array().ok_or_else(|| err(i, format!("{key} must be an array")))?;
    if items.len() > MAX_DOMAINS {
        return Err(err(i, format!("{key} has more than {MAX_DOMAINS} entries")));
    }
    items
        .iter()
        .map(|d| {
            let s = d.as_str().ok_or_else(|| err(i, format!("{key} entries must be strings")))?;
            let bare = s.strip_prefix('*').unwrap_or(s);
            if bare.is_empty() || !bare.is_ascii() || bare.chars().any(|c| c.is_ascii_uppercase() || c == '/' || c == ':' || c == ' ') {
                return Err(err(i, format!("{key}: {s:?} must be a lowercase ASCII domain")));
            }
            Ok(s.to_string())
        })
        .collect()
}

fn top_filters(trigger: &Json, key: &str, case: bool, i: usize) -> Result<Vec<UrlFilter>, ContentRuleError> {
    let Some(v) = trigger.get(key) else { return Ok(Vec::new()) };
    let items = v.as_array().ok_or_else(|| err(i, format!("{key} must be an array")))?;
    items
        .iter()
        .map(|p| {
            let s = p.as_str().ok_or_else(|| err(i, format!("{key} entries must be strings")))?;
            UrlFilter::compile(s, case).map_err(|m| err(i, format!("{key}: {m}")))
        })
        .collect()
}

fn compile_rule(item: &Json, i: usize) -> Result<Rule, ContentRuleError> {
    let trigger = item.get("trigger").ok_or_else(|| err(i, "missing trigger"))?;
    let action = item.get("action").ok_or_else(|| err(i, "missing action"))?;
    let pattern = trigger.get("url-filter").and_then(Json::as_str).ok_or_else(|| err(i, "the trigger needs a url-filter string"))?;
    let case = match trigger.get("url-filter-is-case-sensitive") {
        None => false,
        Some(Json::Bool(b)) => *b,
        Some(_) => return Err(err(i, "url-filter-is-case-sensitive must be a boolean")),
    };
    let filter = UrlFilter::compile(pattern, case).map_err(|m| err(i, format!("url-filter: {m}")))?;
    let mut t = Trigger::new(filter);
    t.if_domain = domains(trigger, "if-domain", i)?;
    t.unless_domain = domains(trigger, "unless-domain", i)?;
    if !t.if_domain.is_empty() && !t.unless_domain.is_empty() {
        return Err(err(i, "if-domain and unless-domain cannot be used together"));
    }
    t.if_top_url = top_filters(trigger, "if-top-url", case, i)?;
    t.unless_top_url = top_filters(trigger, "unless-top-url", case, i)?;
    if !t.if_top_url.is_empty() && !t.unless_top_url.is_empty() {
        return Err(err(i, "if-top-url and unless-top-url cannot be used together"));
    }
    if let Some(v) = trigger.get("resource-type") {
        let items = v.as_array().ok_or_else(|| err(i, "resource-type must be an array"))?;
        let mut types = Vec::new();
        for n in items {
            let name = n.as_str().ok_or_else(|| err(i, "resource-type entries must be strings"))?;
            types.push(ResourceType::from_name(name).ok_or_else(|| err(i, format!("unknown resource-type {name:?}")))?);
        }
        if types.is_empty() {
            return Err(err(i, "resource-type is empty"));
        }
        t.resource_types = Some(types);
    }
    if let Some(v) = trigger.get("load-type") {
        let items = v.as_array().ok_or_else(|| err(i, "load-type must be an array"))?;
        t.load_type = match items.iter().map(Json::as_str).collect::<Vec<_>>().as_slice() {
            [Some("first-party")] => Some(LoadType::FirstParty),
            [Some("third-party")] => Some(LoadType::ThirdParty),
            [Some("first-party"), Some("third-party")] | [Some("third-party"), Some("first-party")] => None,
            _ => return Err(err(i, "load-type must name first-party, third-party or both")),
        };
    }
    let kind = action.get("type").and_then(Json::as_str).ok_or_else(|| err(i, "the action needs a type string"))?;
    let act = match kind {
        "block" => Action::Block,
        "block-cookies" => Action::BlockCookies,
        "ignore-previous-rules" => Action::IgnorePreviousRules,
        "make-https" => Action::MakeHttps,
        "css-display-none" => match action.get("selector").and_then(Json::as_str) {
            Some(s) if !s.trim().is_empty() => Action::CssDisplayNone(s.to_string()),
            _ => return Err(err(i, "css-display-none needs a selector")),
        },
        "notify" => match action.get("notification").and_then(Json::as_str) {
            Some(s) => Action::Notify(s.to_string()),
            None => return Err(err(i, "notify needs a notification string")),
        },
        other => return Err(err(i, format!("unknown action type {other:?}"))),
    };
    Ok(Rule { trigger: t, action: act })
}

impl ContentRuleList {
    /// `compileContentRuleList(forIdentifier:encodedContentRuleList:)`: validates `source` and compiles it.
    pub fn compile(identifier: &str, source: &str) -> Result<ContentRuleList, ContentRuleError> {
        if !valid_identifier(identifier) {
            return Err(ContentRuleError { rule: None, message: format!("{identifier:?} is not a usable list identifier") });
        }
        if source.len() > MAX_JSON_BYTES {
            return Err(ContentRuleError { rule: None, message: "the rule list is larger than 50 MB".into() });
        }
        let doc = json::parse(source).map_err(|m| ContentRuleError { rule: None, message: format!("the rule list is not JSON: {m}") })?;
        let items = doc.as_array().ok_or(ContentRuleError { rule: None, message: "a rule list is a JSON array".into() })?;
        if items.is_empty() {
            return Err(ContentRuleError { rule: None, message: "the rule list is empty".into() });
        }
        if items.len() > MAX_RULES {
            return Err(ContentRuleError { rule: None, message: format!("a rule list holds at most {MAX_RULES} rules") });
        }
        let rules = items.iter().enumerate().map(|(i, item)| compile_rule(item, i)).collect::<Result<Vec<_>, _>>()?;
        Ok(ContentRuleList { identifier: identifier.to_string(), rules: RuleSet::new(rules) })
    }
}

/// An identifier is 1 to 64 of letters, digits, `.`, `_` and `-`, and does not start with a dot.
pub fn valid_identifier(id: &str) -> bool {
    (1..=64).contains(&id.len()) && !id.starts_with('.') && id.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

/// `ContentRuleListStore`: compiled lists as files in a directory.
#[derive(Clone, Debug)]
pub struct ContentRuleListStore {
    dir: PathBuf,
}

impl ContentRuleListStore {
    /// The store of a web profile (`<profile>/content-rules`).
    pub fn in_profile(profile: &Path) -> ContentRuleListStore {
        ContentRuleListStore { dir: profile.join("content-rules") }
    }

    fn file(&self, identifier: &str) -> PathBuf {
        self.dir.join(format!("{identifier}.json"))
    }

    /// Compiles `source` and keeps it under `identifier`, replacing an earlier list with that name. A list that
    /// does not compile leaves the stored one alone.
    pub fn compile(&self, identifier: &str, source: &str) -> Result<ContentRuleList, ContentRuleError> {
        let list = ContentRuleList::compile(identifier, source)?;
        let io_err = |e: io::Error| ContentRuleError { rule: None, message: format!("cannot store the list: {e}") };
        fs::create_dir_all(&self.dir).map_err(io_err)?;
        let tmp = self.dir.join(format!(".{identifier}.tmp"));
        fs::write(&tmp, source).map_err(io_err)?;
        fs::rename(&tmp, self.file(identifier)).map_err(io_err)?;
        Ok(list)
    }

    /// `lookUpContentRuleList(forIdentifier:)`. None when there is no such list or it no longer compiles.
    pub fn look_up(&self, identifier: &str) -> Option<ContentRuleList> {
        if !valid_identifier(identifier) {
            return None;
        }
        let text = fs::read_to_string(self.file(identifier)).ok()?;
        ContentRuleList::compile(identifier, &text).ok()
    }

    /// `removeContentRuleList(forIdentifier:)`. True when a list was removed.
    pub fn remove(&self, identifier: &str) -> bool {
        valid_identifier(identifier) && fs::remove_file(self.file(identifier)).is_ok()
    }

    /// `getAvailableContentRuleListIdentifiers`, sorted.
    pub fn available_identifiers(&self) -> Vec<String> {
        let mut ids: Vec<String> = fs::read_dir(&self.dir)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| e.file_name().into_string().ok())
            .filter_map(|n| n.strip_suffix(".json").map(str::to_string))
            .filter(|n| valid_identifier(n))
            .collect();
        ids.sort();
        ids
    }
}

/// The lists one web view uses (`UserContentController.add(_ contentRuleList:)`).
#[derive(Clone, Debug, Default)]
pub struct ContentRuleLists {
    lists: Vec<ContentRuleList>,
}

impl ContentRuleLists {
    pub fn new() -> ContentRuleLists {
        ContentRuleLists::default()
    }

    /// `add(_:)`: a list with the same identifier is replaced.
    pub fn add(&mut self, list: ContentRuleList) {
        self.remove(&list.identifier);
        self.lists.push(list);
    }

    /// `remove(_:)`: true when the list was in use.
    pub fn remove(&mut self, identifier: &str) -> bool {
        let before = self.lists.len();
        self.lists.retain(|l| l.identifier != identifier);
        self.lists.len() != before
    }

    /// `removeAllContentRuleLists()`
    pub fn remove_all(&mut self) {
        self.lists.clear();
    }

    pub fn identifiers(&self) -> Vec<&str> {
        self.lists.iter().map(|l| l.identifier.as_str()).collect()
    }

    /// Each list with its number of rules, in the order the lists were added.
    pub fn rule_counts(&self) -> Vec<(&str, usize)> {
        self.lists.iter().map(|l| (l.identifier.as_str(), l.rules.len())).collect()
    }

    /// Every rule of every list, in the order the lists were added.
    pub fn rule_set(&self) -> RuleSet {
        RuleSet::new(self.lists.iter().flat_map(|l| l.rules.rules.iter().cloned()).collect())
    }

    /// The verdict for one request over all lists.
    pub fn evaluate(&self, req: &Request) -> Verdict {
        self.rule_set().evaluate(req)
    }
}

/// The script that applies `css-display-none` rules: runs at document start in every frame, and hides the
/// selectors of the matching rules once the document element exists. None when there is no such rule.
pub fn hide_script(rules: &RuleSet) -> Option<String> {
    let mut entries = Vec::new();
    for r in &rules.rules {
        let (kind, selector) = match &r.action {
            Action::CssDisplayNone(s) => ("h", s.as_str()),
            Action::IgnorePreviousRules => ("i", ""),
            _ => continue,
        };
        let names = |v: &[String]| Json::Arr(v.iter().map(|d| Json::Str(d.trim_start_matches('*').trim_start_matches('.').to_string())).collect());
        entries.push(Json::Arr(vec![
            Json::Str(kind.into()),
            Json::Str(r.trigger.filter.source().to_string()),
            Json::Str(if r.trigger.filter.is_case_sensitive() { "" } else { "i" }.into()),
            names(&r.trigger.if_domain),
            names(&r.trigger.unless_domain),
            Json::Str(selector.to_string()),
        ]));
    }
    if !entries.iter().any(|e| matches!(e, Json::Arr(a) if a.first() == Some(&Json::Str("h".into())))) {
        return None;
    }
    let data = Json::Arr(entries).encode();
    Some(format!(
        "(function(){{try{{var R={data},h=location.hostname.toLowerCase(),u=location.href,sel=[];\
function dm(l){{return l.some(function(d){{return h===d||h.slice(-d.length-1)==='.'+d}})}}\
for(var i=0;i<R.length;i++){{var r=R[i];try{{\
if(!new RegExp(r[1],r[2]).test(u))continue;\
if(r[3].length&&!dm(r[3]))continue;if(dm(r[4]))continue;\
if(r[0]==='i')sel=[];else sel.push(r[5])}}catch(e){{}}}}\
if(!sel.length)return;\
var s=document.createElement('style');s.setAttribute('data-bifrost-rules','1');\
s.textContent=sel.join(',')+'{{display:none!important}}';\
(document.head||document.documentElement).appendChild(s)}}catch(e){{}}}})()"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIST: &str = r#"[
      {"trigger": {"url-filter": "^https?://.*\\.tracker\\.example/", "resource-type": ["script", "image"],
                   "load-type": ["third-party"], "unless-domain": ["*friend.example"]},
       "action": {"type": "block"}},
      {"trigger": {"url-filter": ".*"}, "action": {"type": "css-display-none", "selector": ".ad, #banner"}},
      {"trigger": {"url-filter": "^http://shop\\.example/"}, "action": {"type": "make-https"}},
      {"trigger": {"url-filter": "metrics"}, "action": {"type": "block-cookies"}}
    ]"#;

    #[test]
    fn a_reference_style_list_compiles_and_judges_requests() {
        let list = ContentRuleList::compile("ads", LIST).unwrap();
        assert_eq!(list.rules.len(), 4);
        let ask = |url, rt, top| list.rules.evaluate(&Request { url, resource: rt, top_url: top });
        let v = ask("https://x.tracker.example/p.js", ResourceType::Script, Some("https://news.example/"));
        assert!(v.block);
        assert_eq!(v.css_selectors, vec![".ad, #banner".to_string()]);
        assert!(!ask("https://x.tracker.example/p.js", ResourceType::Script, Some("https://www.friend.example/")).block);
        assert!(!ask("https://x.tracker.example/p.css", ResourceType::StyleSheet, Some("https://news.example/")).block);
        assert!(ask("http://shop.example/cart", ResourceType::Document, None).make_https);
        assert!(ask("https://a.example/metrics", ResourceType::Raw, Some("https://news.example/")).block_cookies);
    }

    #[test]
    fn broken_lists_are_refused_with_the_rule_index() {
        let bad = |json: &str| ContentRuleList::compile("x", json).unwrap_err();
        assert_eq!(bad("{}").rule, None);
        assert_eq!(bad("[]").message, "the rule list is empty");
        assert_eq!(bad("not json").rule, None);
        let e = bad(r#"[{"trigger":{"url-filter":"a"},"action":{"type":"block"}},{"trigger":{"url-filter":"a|b"},"action":{"type":"block"}}]"#);
        assert_eq!(e.rule, Some(1));
        assert!(e.message.contains("alternation"));
        assert!(bad(r#"[{"trigger":{"url-filter":"a"},"action":{"type":"css-display-none"}}]"#).message.contains("selector"));
        assert!(bad(r#"[{"trigger":{"url-filter":"a"},"action":{"type":"explode"}}]"#).message.contains("unknown action"));
        assert!(bad(r#"[{"trigger":{"url-filter":"a","if-domain":["a.example"],"unless-domain":["b.example"]},"action":{"type":"block"}}]"#).message.contains("together"));
        assert!(bad(r#"[{"trigger":{"url-filter":"a","if-domain":["Upper.example"]},"action":{"type":"block"}}]"#).message.contains("lowercase"));
        assert!(bad(r#"[{"trigger":{"url-filter":"a","resource-type":["telepathy"]},"action":{"type":"block"}}]"#).message.contains("resource-type"));
        assert!(bad(r#"[{"trigger":{"url-filter":"a","load-type":["sideways"]},"action":{"type":"block"}}]"#).message.contains("load-type"));
        assert!(bad(r#"[{"trigger":{},"action":{"type":"block"}}]"#).message.contains("url-filter"));
        assert!(ContentRuleList::compile("../evil", LIST).is_err());
        assert!(ContentRuleList::compile("", LIST).is_err());
    }

    #[test]
    fn both_load_types_mean_no_restriction() {
        let l = ContentRuleList::compile("x", r#"[{"trigger":{"url-filter":"a","load-type":["first-party","third-party"]},"action":{"type":"block"}}]"#).unwrap();
        assert_eq!(l.rules.rules[0].trigger.load_type, None);
    }

    #[test]
    fn the_store_keeps_replaces_and_removes_lists() {
        let dir = tempfile::tempdir().unwrap();
        let store = ContentRuleListStore::in_profile(dir.path());
        assert!(store.available_identifiers().is_empty());
        assert!(store.look_up("ads").is_none());
        store.compile("ads", LIST).unwrap();
        store.compile("b-list", r#"[{"trigger":{"url-filter":"x"},"action":{"type":"block"}}]"#).unwrap();
        assert_eq!(store.available_identifiers(), vec!["ads".to_string(), "b-list".to_string()]);
        assert_eq!(store.look_up("ads").unwrap().rules.len(), 4);
        // a list that fails to compile leaves the stored one in place
        assert!(store.compile("ads", "[{\"trigger\":{}}]").is_err());
        assert_eq!(store.look_up("ads").unwrap().rules.len(), 4);
        store.compile("ads", r#"[{"trigger":{"url-filter":"y"},"action":{"type":"block"}}]"#).unwrap();
        assert_eq!(store.look_up("ads").unwrap().rules.len(), 1);
        assert!(store.remove("ads"));
        assert!(!store.remove("ads"));
        assert!(!store.remove("../ads"));
        assert_eq!(store.available_identifiers(), vec!["b-list".to_string()]);
    }

    #[test]
    fn lists_in_use_join_in_order_and_replace_by_identifier() {
        let block = |p: &str| ContentRuleList::compile("a", &format!(r#"[{{"trigger":{{"url-filter":"{p}"}},"action":{{"type":"block"}}}}]"#)).unwrap();
        let mut use_ = ContentRuleLists::new();
        use_.add(block("one"));
        let mut second = block("two");
        second.identifier = "b".into();
        use_.add(second);
        assert_eq!(use_.identifiers(), vec!["a", "b"]);
        use_.add(block("three")); // replaces "a", which moves to the end
        assert_eq!(use_.identifiers(), vec!["b", "a"]);
        assert_eq!(use_.rule_counts(), vec![("b", 1), ("a", 1)]);
        let ask = |u: &str| use_.evaluate(&Request { url: u, resource: ResourceType::Image, top_url: Some("https://p.example/") }).block;
        assert!(!ask("https://x.example/one") && ask("https://x.example/two") && ask("https://x.example/three"));
        assert!(use_.remove("b") && !use_.remove("b"));
        use_.remove_all();
        assert!(use_.identifiers().is_empty() && use_.rule_set().is_empty());
    }

    #[test]
    fn the_hide_script_carries_selectors_domains_and_ignore_rules() {
        let list = ContentRuleList::compile(
            "h",
            r#"[{"trigger":{"url-filter":".*","if-domain":["*news.example"]},"action":{"type":"css-display-none","selector":".ad"}},
                {"trigger":{"url-filter":"/video/"},"action":{"type":"ignore-previous-rules"}},
                {"trigger":{"url-filter":"x","url-filter-is-case-sensitive":true},"action":{"type":"block"}}]"#,
        )
        .unwrap();
        let js = hide_script(&list.rules).unwrap();
        assert!(js.contains("[\"h\",\".*\",\"i\",[\"news.example\"],[],\".ad\"]"), "{js}");
        assert!(js.contains("[\"i\",\"/video/\",\"i\",[],[],\"\"]"));
        assert!(js.contains("display:none!important"));
        assert!(!js.contains("\"x\""), "block rules do not belong in the page script");
        assert_eq!(js.matches('{').count(), js.matches('}').count());
        let only_block = ContentRuleList::compile("b", r#"[{"trigger":{"url-filter":"x"},"action":{"type":"block"}}]"#).unwrap();
        assert_eq!(hide_script(&only_block.rules), None);
        let only_ignore = ContentRuleList::compile("i", r#"[{"trigger":{"url-filter":"x"},"action":{"type":"ignore-previous-rules"}}]"#).unwrap();
        assert_eq!(hide_script(&only_ignore.rules), None);
    }
}
