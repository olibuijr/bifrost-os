//! What Mail remembers for itself between runs, in one JSON file in the app's data directory: the signature and the other
//! send-from addresses of each account, the undo-send delay, which address fields a new message shows, the favourites
//! bar, the smart mailboxes and the rules. A missing or damaged file gives the defaults.

use crate::rules::Rule;
use bifrost_core::json::{self, Json};
use std::collections::HashMap;
use std::path::Path;

/// File name in the app's data directory.
pub const FILE: &str = "mail-prefs.json";
/// Longest undo-send delay, in seconds.
pub const MAX_UNDO_DELAY: u32 = 30;
/// Most smart mailboxes and favourites kept.
pub const MAX_ENTRIES: usize = 24;

/// A saved search that shows as a mailbox in the sidebar (Mailbox > New Smart Mailbox).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SmartMailbox {
    pub name: String,
    /// The search text, as the Find sheet takes it (`from:ana subject:fundur`).
    pub query: String,
}

/// A mailbox pinned to the favourites bar.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Favorite {
    pub account: String,
    pub mailbox: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Prefs {
    /// Seconds a sent message waits before it goes out, so Message > Undo Send can take it back. 0 sends at once.
    pub undo_delay: u32,
    /// The signature appended to new messages, by account id.
    pub signatures: HashMap<String, String>,
    /// Other addresses an account may send from, by account id.
    pub aliases: HashMap<String, Vec<String>>,
    pub favorites: Vec<Favorite>,
    pub show_favorites: bool,
    pub smart: Vec<SmartMailbox>,
    pub rules: Vec<Rule>,
    /// Which optional address fields a new message shows (View menu).
    pub show_cc: bool,
    pub show_bcc: bool,
    pub show_reply_to: bool,
    /// Replies start with the original quoted under an attribution line (Mailbox > Mail Settings).
    pub quote_replies: bool,
    /// New messages are sent as plain text only, without the HTML alternative.
    pub plain_text: bool,
    /// The reader shows every header of the open message (View > All Headers).
    pub all_headers: bool,
    /// Unread messages show their sender in bold, and read ones in regular type (Mail Settings).
    pub bold_unread: bool,
    /// Remote content (images, tracking pixels included) may load in every message. Off: it loads only in the messages the
    /// person allows with Load Remote Content.
    pub remote_content: bool,
    /// A reply's signature goes above the quoted text (before the "On ..., wrote:" line) instead of at the end.
    pub signature_above_quote: bool,
    /// The newest messages' subjects and senders are indexed for Core Search (never the bodies). Off removes them.
    pub show_in_search: bool,
}

impl Default for Prefs {
    fn default() -> Prefs {
        Prefs {
            undo_delay: 0,
            signatures: HashMap::new(),
            aliases: HashMap::new(),
            favorites: Vec::new(),
            show_favorites: false,
            smart: Vec::new(),
            rules: Vec::new(),
            show_cc: true,
            show_bcc: false,
            show_reply_to: false,
            quote_replies: true,
            plain_text: false,
            all_headers: false,
            bold_unread: true,
            remote_content: false,
            signature_above_quote: false,
            show_in_search: true,
        }
    }
}

fn text_of(json: &Json, key: &str) -> String {
    json.get(key).and_then(Json::as_str).unwrap_or("").to_string()
}

fn flag_of(json: &Json, key: &str, default: bool) -> bool {
    match json.get(key) {
        Some(Json::Bool(b)) => *b,
        _ => default,
    }
}

impl Prefs {
    /// The signature of an account, empty when it has none.
    pub fn signature(&self, account: &str) -> &str {
        self.signatures.get(account).map_or("", String::as_str)
    }

    /// The other send-from addresses of an account.
    pub fn aliases_of(&self, account: &str) -> &[String] {
        self.aliases.get(account).map(Vec::as_slice).unwrap_or(&[])
    }

    /// Sets (or, when blank, clears) the signature of an account.
    pub fn set_signature(&mut self, account: &str, signature: &str) {
        let signature = signature.trim_end();
        if signature.trim().is_empty() {
            self.signatures.remove(account);
        } else {
            self.signatures.insert(account.to_string(), signature.to_string());
        }
    }

    /// Sets (or, when empty, clears) the other send-from addresses of an account.
    pub fn set_aliases(&mut self, account: &str, aliases: Vec<String>) {
        if aliases.is_empty() {
            self.aliases.remove(account);
        } else {
            self.aliases.insert(account.to_string(), aliases);
        }
    }

    pub fn is_favorite(&self, account: &str, mailbox: &str) -> bool {
        self.favorites.iter().any(|f| f.account == account && f.mailbox == mailbox)
    }

    /// Pins or unpins a mailbox. Returns false when the bar is full and nothing was pinned.
    pub fn toggle_favorite(&mut self, account: &str, mailbox: &str) -> bool {
        if let Some(at) = self.favorites.iter().position(|f| f.account == account && f.mailbox == mailbox) {
            self.favorites.remove(at);
            return true;
        }
        if self.favorites.len() >= MAX_ENTRIES {
            return false;
        }
        self.favorites.push(Favorite { account: account.to_string(), mailbox: mailbox.to_string() });
        true
    }

    pub fn to_json(&self) -> Json {
        let mut signatures: Vec<(&String, &String)> = self.signatures.iter().collect();
        signatures.sort();
        let mut aliases: Vec<(&String, &Vec<String>)> = self.aliases.iter().collect();
        aliases.sort();
        Json::Obj(vec![
            ("undo_delay".into(), Json::Num(f64::from(self.undo_delay))),
            ("signatures".into(), Json::Obj(signatures.into_iter().map(|(k, v)| (k.clone(), Json::Str(v.clone()))).collect())),
            (
                "aliases".into(),
                Json::Obj(aliases.into_iter().map(|(k, v)| (k.clone(), Json::Arr(v.iter().map(|a| Json::Str(a.clone())).collect()))).collect()),
            ),
            (
                "favorites".into(),
                Json::Arr(
                    self.favorites
                        .iter()
                        .map(|f| Json::Obj(vec![("account".into(), Json::Str(f.account.clone())), ("mailbox".into(), Json::Str(f.mailbox.clone()))]))
                        .collect(),
                ),
            ),
            ("show_favorites".into(), Json::Bool(self.show_favorites)),
            (
                "smart".into(),
                Json::Arr(
                    self.smart
                        .iter()
                        .map(|s| Json::Obj(vec![("name".into(), Json::Str(s.name.clone())), ("query".into(), Json::Str(s.query.clone()))]))
                        .collect(),
                ),
            ),
            ("rules".into(), Json::Arr(self.rules.iter().map(Rule::to_json).collect())),
            ("show_cc".into(), Json::Bool(self.show_cc)),
            ("show_bcc".into(), Json::Bool(self.show_bcc)),
            ("show_reply_to".into(), Json::Bool(self.show_reply_to)),
            ("quote_replies".into(), Json::Bool(self.quote_replies)),
            ("plain_text".into(), Json::Bool(self.plain_text)),
            ("all_headers".into(), Json::Bool(self.all_headers)),
            ("bold_unread".into(), Json::Bool(self.bold_unread)),
            ("remote_content".into(), Json::Bool(self.remote_content)),
            ("signature_above_quote".into(), Json::Bool(self.signature_above_quote)),
            ("show_in_search".into(), Json::Bool(self.show_in_search)),
        ])
    }

    /// Reads the settings back; anything missing or of the wrong shape keeps its default.
    pub fn from_json(json: &Json) -> Prefs {
        let defaults = Prefs::default();
        let mut prefs = Prefs {
            undo_delay: json.get("undo_delay").and_then(Json::as_u64).map_or(0, |n| n.min(u64::from(MAX_UNDO_DELAY)) as u32),
            show_favorites: flag_of(json, "show_favorites", defaults.show_favorites),
            show_cc: flag_of(json, "show_cc", defaults.show_cc),
            show_bcc: flag_of(json, "show_bcc", defaults.show_bcc),
            show_reply_to: flag_of(json, "show_reply_to", defaults.show_reply_to),
            quote_replies: flag_of(json, "quote_replies", defaults.quote_replies),
            plain_text: flag_of(json, "plain_text", defaults.plain_text),
            all_headers: flag_of(json, "all_headers", defaults.all_headers),
            bold_unread: flag_of(json, "bold_unread", defaults.bold_unread),
            remote_content: flag_of(json, "remote_content", defaults.remote_content),
            signature_above_quote: flag_of(json, "signature_above_quote", defaults.signature_above_quote),
            show_in_search: flag_of(json, "show_in_search", defaults.show_in_search),
            ..defaults
        };
        if let Some(Json::Obj(fields)) = json.get("signatures") {
            for (account, value) in fields {
                if let Some(text) = value.as_str().filter(|t| !t.trim().is_empty()) {
                    prefs.signatures.insert(account.clone(), text.to_string());
                }
            }
        }
        if let Some(Json::Obj(fields)) = json.get("aliases") {
            for (account, value) in fields {
                let list: Vec<String> = value.as_array().unwrap_or(&[]).iter().filter_map(Json::as_str).map(str::to_string).collect();
                if !list.is_empty() {
                    prefs.aliases.insert(account.clone(), list);
                }
            }
        }
        for item in json.get("favorites").and_then(Json::as_array).unwrap_or(&[]).iter().take(MAX_ENTRIES) {
            let (account, mailbox) = (text_of(item, "account"), text_of(item, "mailbox"));
            if !account.is_empty() && !mailbox.is_empty() {
                prefs.favorites.push(Favorite { account, mailbox });
            }
        }
        for item in json.get("smart").and_then(Json::as_array).unwrap_or(&[]).iter().take(MAX_ENTRIES) {
            let (name, query) = (text_of(item, "name"), text_of(item, "query"));
            if !name.trim().is_empty() && !query.trim().is_empty() {
                prefs.smart.push(SmartMailbox { name, query });
            }
        }
        prefs.rules = json.get("rules").and_then(Json::as_array).unwrap_or(&[]).iter().filter_map(Rule::from_json).collect();
        prefs
    }
}

/// Reads the settings file. A missing or damaged file gives the defaults.
pub fn load(path: &Path) -> Prefs {
    std::fs::read_to_string(path).ok().and_then(|text| json::parse(&text).ok()).map_or_else(Prefs::default, |j| Prefs::from_json(&j))
}

/// Writes the settings file, creating the directory.
pub fn save(path: &Path, prefs: &Prefs) -> Result<(), String> {
    write_json(path, &prefs.to_json())
}

/// Writes a JSON value to `path` through a temporary file, so a crash never leaves half a file.
pub fn write_json(path: &Path, value: &Json) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let temporary = path.with_extension("tmp");
    std::fs::write(&temporary, value.encode()).map_err(|e| e.to_string())?;
    std::fs::rename(&temporary, path).map_err(|e| e.to_string())
}

/// Reads a JSON file; None when it is missing or damaged.
pub fn read_json(path: &Path) -> Option<Json> {
    json::parse(&std::fs::read_to_string(path).ok()?).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flags::FlagColor;
    use crate::rules::Action;

    fn sample() -> Prefs {
        let mut prefs = Prefs {
            undo_delay: 10,
            show_favorites: true,
            show_cc: false,
            show_bcc: true,
            quote_replies: false,
            plain_text: true,
            all_headers: true,
            ..Prefs::default()
        };
        prefs.set_signature("work", "Ana\nOlafsson ehf.\n");
        prefs.set_aliases("work", vec!["info@example.com".into(), "ana@example.org".into()]);
        prefs.toggle_favorite("work", "INBOX");
        prefs.smart.push(SmartMailbox { name: "From Ana".into(), query: "from:ana".into() });
        prefs.rules.push(Rule { name: "Shops".into(), from: "shop".into(), actions: vec![Action::Flag(FlagColor::Blue)], ..Rule::default() });
        prefs
    }

    #[test]
    fn the_settings_survive_a_round_trip_through_the_file() {
        let dir = std::env::temp_dir().join(format!("bifrost-mail-prefs-{}", std::process::id()));
        let path = dir.join(FILE);
        assert_eq!(load(&path), Prefs::default(), "a missing file gives the defaults");
        let prefs = sample();
        save(&path, &prefs).unwrap();
        assert_eq!(load(&path), prefs);
        std::fs::write(&path, "{ not json").unwrap();
        assert_eq!(load(&path), Prefs::default(), "a damaged file gives the defaults");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_blank_signature_or_alias_list_clears_the_entry() {
        let mut prefs = sample();
        assert_eq!(prefs.signature("work"), "Ana\nOlafsson ehf.");
        prefs.set_signature("work", "  \n");
        prefs.set_aliases("work", Vec::new());
        assert_eq!(prefs.signature("work"), "");
        assert!(prefs.aliases_of("work").is_empty() && prefs.aliases_of("other").is_empty());
    }

    #[test]
    fn favourites_toggle_and_the_bar_has_a_limit() {
        let mut prefs = Prefs::default();
        assert!(prefs.toggle_favorite("a", "Sent") && prefs.is_favorite("a", "Sent"));
        assert!(prefs.toggle_favorite("a", "Sent") && !prefs.is_favorite("a", "Sent"));
        for n in 0..MAX_ENTRIES {
            assert!(prefs.toggle_favorite("a", &format!("box{n}")));
        }
        assert!(!prefs.toggle_favorite("a", "one too many"));
    }

    #[test]
    fn replies_quote_and_messages_keep_html_unless_asked_otherwise() {
        let prefs = Prefs::default();
        assert!(prefs.quote_replies && !prefs.plain_text && !prefs.all_headers);
        let older = Prefs::from_json(&json::parse(r#"{"undo_delay":3}"#).unwrap());
        assert!(older.quote_replies, "a file from before the setting keeps the default");
        let off = Prefs::from_json(&json::parse(r#"{"quote_replies":false,"plain_text":true}"#).unwrap());
        assert!(!off.quote_replies && off.plain_text);
    }

    #[test]
    fn unread_senders_are_bold_unless_the_setting_is_off() {
        assert!(Prefs::default().bold_unread);
        assert!(Prefs::from_json(&json::parse("{}").unwrap()).bold_unread, "a file from before the setting keeps bold");
        let off = Prefs::from_json(&json::parse(r#"{"bold_unread":false}"#).unwrap());
        assert!(!off.bold_unread);
        assert!(!Prefs::default().remote_content, "remote content stays blocked by default");
        assert!(!Prefs::default().signature_above_quote, "the signature goes at the end by default");
        assert_eq!(Prefs::from_json(&off.to_json()), off);
    }

    #[test]
    fn values_out_of_range_or_of_the_wrong_type_fall_back() {
        let json = json::parse(r#"{"undo_delay":9999,"show_cc":"yes","signatures":{"a":"  ","b":"hi"},"favorites":[{"account":"a"}],"smart":[{"name":"x","query":""}]}"#).unwrap();
        let prefs = Prefs::from_json(&json);
        assert_eq!(prefs.undo_delay, MAX_UNDO_DELAY);
        assert!(prefs.show_cc, "a wrong type keeps the default");
        assert_eq!(prefs.signature("a"), "");
        assert_eq!(prefs.signature("b"), "hi");
        assert!(prefs.favorites.is_empty() && prefs.smart.is_empty());
    }
}
