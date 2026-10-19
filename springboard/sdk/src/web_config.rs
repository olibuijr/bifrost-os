//! Web view configuration, page scripts and messages, UI delegate panels and in-page find
//! (`WebViewConfiguration`, `Preferences`, `UserContentController`, `UserScript`, `ScriptMessageHandler`,
//! `WebUIDelegate`, `FindConfiguration`, `FindResult`).
//!
//! The configuration is read once when the engine starts a view: the user agent, the preferences and the user
//! scripts are applied then. The content controller routes a message a page posts under a handler name to the
//! handler the app registered for it. Everything here is pure state; the engine glue calls into it.

use std::collections::HashMap;
use std::path::PathBuf;

/// `Preferences`.
#[derive(Clone, Debug, PartialEq)]
pub struct WebPreferences {
    /// `javaScriptEnabled`
    pub javascript_enabled: bool,
    /// `javaScriptCanOpenWindowsAutomatically`
    pub javascript_can_open_windows_automatically: bool,
    /// `minimumFontSize`, in points; 0 means no minimum.
    pub minimum_font_size: f32,
    /// `isFraudulentWebsiteWarningEnabled`
    pub fraudulent_website_warning_enabled: bool,
}

impl Default for WebPreferences {
    fn default() -> Self {
        Self { javascript_enabled: true, javascript_can_open_windows_automatically: false, minimum_font_size: 0.0, fraudulent_website_warning_enabled: true }
    }
}

/// `UserScriptInjectionTime`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InjectionTime {
    AtDocumentStart,
    AtDocumentEnd,
}

/// `UserScript`: JavaScript the engine runs in each page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UserScript {
    pub source: String,
    pub injection_time: InjectionTime,
    /// `forMainFrameOnly`
    pub for_main_frame_only: bool,
}

impl UserScript {
    pub fn new(source: &str, injection_time: InjectionTime, for_main_frame_only: bool) -> Self {
        Self { source: source.into(), injection_time, for_main_frame_only }
    }
}

/// `ScriptMessage`: what a page posted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScriptMessage {
    /// The handler name the page posted to.
    pub name: String,
    /// `body`, as the JSON text the page sent.
    pub body: String,
    pub is_main_frame: bool,
}

/// `ScriptMessageHandler`.
pub trait ScriptMessageHandler {
    fn did_receive(&mut self, message: &ScriptMessage);
}

/// Why a handler could not be added.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HandlerError {
    /// A handler is already registered under this name.
    Duplicate,
    /// The name is empty or not usable as a JavaScript property name.
    InvalidName,
}

pub(crate) fn valid_handler_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_' || c == '$') && chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$')
}

/// `UserContentController`.
#[derive(Default)]
pub struct UserContentController {
    scripts: Vec<UserScript>,
    handlers: HashMap<String, Box<dyn ScriptMessageHandler>>,
}

impl UserContentController {
    pub fn new() -> Self {
        Self::default()
    }

    /// `addUserScript`
    pub fn add_user_script(&mut self, script: UserScript) {
        self.scripts.push(script);
    }

    /// `removeAllUserScripts`
    pub fn remove_all_user_scripts(&mut self) {
        self.scripts.clear();
    }

    /// `userScripts`
    pub fn user_scripts(&self) -> &[UserScript] {
        &self.scripts
    }

    /// `add(_:name:)`: a page reaches the handler as `window.bifrost.messageHandlers.<name>.postMessage(body)`.
    pub fn add(&mut self, handler: Box<dyn ScriptMessageHandler>, name: &str) -> Result<(), HandlerError> {
        if !valid_handler_name(name) {
            return Err(HandlerError::InvalidName);
        }
        if self.handlers.contains_key(name) {
            return Err(HandlerError::Duplicate);
        }
        self.handlers.insert(name.to_string(), handler);
        Ok(())
    }

    /// `removeScriptMessageHandler(forName:)`
    pub fn remove_script_message_handler(&mut self, name: &str) -> bool {
        self.handlers.remove(name).is_some()
    }

    /// Delivers a page's message to the handler registered under its name. Returns false when there is none.
    pub fn deliver(&mut self, message: &ScriptMessage) -> bool {
        match self.handlers.get_mut(&message.name) {
            Some(h) => {
                h.did_receive(message);
                true
            }
            None => false,
        }
    }

    /// The script that creates `window.bifrost.messageHandlers` for the registered names. `post` is the page-side
    /// function that hands `(name, JSON text)` to the engine. Runs at document start in every frame.
    pub fn bootstrap_script(&self, post: &str) -> UserScript {
        let mut names: Vec<&String> = self.handlers.keys().collect();
        names.sort();
        let mut js = String::from("(function(){var h={};");
        for n in names {
            js.push_str(&format!("h[\"{n}\"]={{postMessage:function(b){{{post}(\"{n}\",JSON.stringify(b));}}}};"));
        }
        js.push_str("window.bifrost=window.bifrost||{};window.bifrost.messageHandlers=h;})();");
        UserScript::new(&js, InjectionTime::AtDocumentStart, false)
    }
}

/// `WebViewConfiguration`.
#[derive(Default)]
pub struct WebViewConfiguration {
    pub preferences: WebPreferences,
    pub user_content_controller: UserContentController,
    /// `applicationNameForUserAgent`: appended to the default user agent.
    pub application_name_for_user_agent: Option<String>,
    /// `customUserAgent`: replaces the default user agent; applied when the engine starts.
    pub custom_user_agent: Option<String>,
}

impl WebViewConfiguration {
    pub fn new() -> Self {
        Self::default()
    }

    /// The user agent the engine starts with: the custom one if set, else `default` plus the application name.
    pub fn effective_user_agent(&self, default: &str) -> String {
        if let Some(ua) = self.custom_user_agent.as_deref().filter(|u| !u.is_empty()) {
            return ua.to_string();
        }
        match self.application_name_for_user_agent.as_deref().filter(|n| !n.is_empty()) {
            Some(app) => format!("{default} {app}"),
            None => default.to_string(),
        }
    }

    /// The scripts to inject, in order: the message bridge first, then the app's own.
    pub fn injected_scripts(&self, post: &str) -> Vec<UserScript> {
        let mut all = vec![self.user_content_controller.bootstrap_script(post)];
        all.extend(self.user_content_controller.user_scripts().iter().cloned());
        all
    }
}

/// `OpenPanelParameters`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct OpenPanelParameters {
    pub allows_multiple_selection: bool,
    pub allows_directories: bool,
}

/// `WebUIDelegate` panels. Without overrides a view behaves as with no delegate: alerts are dismissed, confirms
/// and prompts are refused, and no file is chosen.
pub trait UiDelegate {
    /// `runJavaScriptAlertPanelWithMessage`
    fn run_javascript_alert_panel(&mut self, _message: &str) {}
    /// `runJavaScriptConfirmPanelWithMessage`: true for OK.
    fn run_javascript_confirm_panel(&mut self, _message: &str) -> bool {
        false
    }
    /// `runJavaScriptTextInputPanelWithPrompt`: the entered text, or `None` for Cancel.
    fn run_javascript_text_input_panel(&mut self, _prompt: &str, _default: &str) -> Option<String> {
        None
    }
    /// `runOpenPanelWith`: the chosen files, or `None` for Cancel.
    fn run_open_panel(&mut self, _parameters: &OpenPanelParameters) -> Option<Vec<PathBuf>> {
        None
    }
}

/// Which script dialog is open (mirrors `webview::DialogKind`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PanelKind {
    Alert,
    Confirm,
    Prompt,
}

/// Runs the delegate panel for a script dialog and returns `(ok, text)` as `WebProcessPool::dialog_reply` takes it.
pub fn answer_script_dialog(delegate: &mut dyn UiDelegate, kind: PanelKind, message: &str, default: &str) -> (bool, String) {
    match kind {
        PanelKind::Alert => {
            delegate.run_javascript_alert_panel(message);
            (true, String::new())
        }
        PanelKind::Confirm => (delegate.run_javascript_confirm_panel(message), String::new()),
        PanelKind::Prompt => match delegate.run_javascript_text_input_panel(message, default) {
            Some(text) => (true, text),
            None => (false, String::new()),
        },
    }
}

/// `FindConfiguration`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FindConfiguration {
    pub backwards: bool,
    pub case_sensitive: bool,
    pub wraps: bool,
}

impl Default for FindConfiguration {
    fn default() -> Self {
        Self { backwards: false, case_sensitive: false, wraps: true }
    }
}

/// `FindResult`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FindResult {
    pub match_found: bool,
    /// Zero-based index of the selected match, when one is found.
    pub match_index: Option<usize>,
    pub match_count: usize,
}

/// Byte offsets of the non-overlapping matches of `query` in `text`.
pub fn find_matches(text: &str, query: &str, case_sensitive: bool) -> Vec<usize> {
    if query.is_empty() {
        return Vec::new();
    }
    let (hay, needle) = if case_sensitive { (text.to_string(), query.to_string()) } else { (text.to_lowercase(), query.to_lowercase()) };
    // lowercasing can change byte lengths, so offsets are only valid when it did not
    if hay.len() != text.len() {
        return Vec::new();
    }
    hay.match_indices(needle.as_str()).map(|(i, _)| i).collect()
}

/// Moves a selection through the matches of one find (`find(_:configuration:)` called again for next and previous).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FindSession {
    count: usize,
    selected: Option<usize>,
}

impl FindSession {
    /// Starts a find with `count` matches; the first (or, searching backwards, the last) is selected.
    pub fn begin(count: usize, config: &FindConfiguration) -> (Self, FindResult) {
        let selected = match (count, config.backwards) {
            (0, _) => None,
            (_, false) => Some(0),
            (n, true) => Some(n - 1),
        };
        let s = Self { count, selected };
        (s, s.result())
    }

    /// Selects the next match in the configured direction. Without `wraps`, stays on the last one and reports no
    /// match found past the end.
    pub fn step(&mut self, config: &FindConfiguration) -> FindResult {
        let Some(cur) = self.selected else { return self.result() };
        let next = if config.backwards { cur.checked_sub(1) } else { (cur + 1 < self.count).then_some(cur + 1) };
        match next {
            Some(i) => self.selected = Some(i),
            None if config.wraps => self.selected = Some(if config.backwards { self.count - 1 } else { 0 }),
            None => return FindResult { match_found: false, match_index: Some(cur), match_count: self.count },
        }
        self.result()
    }

    fn result(&self) -> FindResult {
        FindResult { match_found: self.selected.is_some(), match_index: self.selected, match_count: self.count }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    struct Recorder(Rc<RefCell<Vec<ScriptMessage>>>);
    impl ScriptMessageHandler for Recorder {
        fn did_receive(&mut self, m: &ScriptMessage) {
            self.0.borrow_mut().push(m.clone());
        }
    }

    fn message(name: &str, body: &str) -> ScriptMessage {
        ScriptMessage { name: name.into(), body: body.into(), is_main_frame: true }
    }

    #[test]
    fn a_posted_message_reaches_the_handler_with_that_name() {
        let seen = Rc::new(RefCell::new(Vec::new()));
        let other = Rc::new(RefCell::new(Vec::new()));
        let mut c = UserContentController::new();
        c.add(Box::new(Recorder(seen.clone())), "auth").unwrap();
        c.add(Box::new(Recorder(other.clone())), "log").unwrap();
        assert!(c.deliver(&message("auth", "{\"ok\":true}")));
        assert!(!c.deliver(&message("missing", "1")));
        assert_eq!(seen.borrow().len(), 1);
        assert_eq!(seen.borrow()[0].body, "{\"ok\":true}");
        assert!(other.borrow().is_empty());
    }

    #[test]
    fn handler_names_are_unique_valid_and_removable() {
        let seen = Rc::new(RefCell::new(Vec::new()));
        let mut c = UserContentController::new();
        c.add(Box::new(Recorder(seen.clone())), "a").unwrap();
        assert_eq!(c.add(Box::new(Recorder(seen.clone())), "a"), Err(HandlerError::Duplicate));
        for bad in ["", "1x", "a b", "a\"b"] {
            assert_eq!(c.add(Box::new(Recorder(seen.clone())), bad), Err(HandlerError::InvalidName));
        }
        assert!(c.remove_script_message_handler("a"));
        assert!(!c.remove_script_message_handler("a"));
        assert!(!c.deliver(&message("a", "1")));
    }

    #[test]
    fn user_scripts_keep_their_order_until_removed() {
        let mut c = UserContentController::new();
        c.add_user_script(UserScript::new("1", InjectionTime::AtDocumentStart, true));
        c.add_user_script(UserScript::new("2", InjectionTime::AtDocumentEnd, false));
        assert_eq!(c.user_scripts().iter().map(|s| s.source.as_str()).collect::<Vec<_>>(), vec!["1", "2"]);
        c.remove_all_user_scripts();
        assert!(c.user_scripts().is_empty());
    }

    #[test]
    fn the_bootstrap_defines_one_poster_per_handler() {
        let mut c = UserContentController::new();
        c.add(Box::new(Recorder(Rc::default())), "zeta").unwrap();
        c.add(Box::new(Recorder(Rc::default())), "alpha").unwrap();
        let s = c.bootstrap_script("__post");
        assert_eq!(s.injection_time, InjectionTime::AtDocumentStart);
        assert!(!s.for_main_frame_only);
        assert!(s.source.contains("window.bifrost.messageHandlers=h"));
        assert!(s.source.find("h[\"alpha\"]").unwrap() < s.source.find("h[\"zeta\"]").unwrap());
        assert!(s.source.contains("__post(\"alpha\",JSON.stringify(b))"));
    }

    #[test]
    fn the_configuration_puts_the_bridge_before_app_scripts() {
        let mut cfg = WebViewConfiguration::new();
        cfg.user_content_controller.add_user_script(UserScript::new("app()", InjectionTime::AtDocumentEnd, true));
        let all = cfg.injected_scripts("__post");
        assert_eq!(all.len(), 2);
        assert!(all[0].source.contains("messageHandlers") && all[1].source == "app()");
    }

    #[test]
    fn user_agent_prefers_the_custom_one_then_the_application_name() {
        let mut cfg = WebViewConfiguration::new();
        assert_eq!(cfg.effective_user_agent("Base/1"), "Base/1");
        cfg.application_name_for_user_agent = Some("Mail/2".into());
        assert_eq!(cfg.effective_user_agent("Base/1"), "Base/1 Mail/2");
        cfg.custom_user_agent = Some("Custom/3".into());
        assert_eq!(cfg.effective_user_agent("Base/1"), "Custom/3");
        cfg.custom_user_agent = Some(String::new());
        assert_eq!(cfg.effective_user_agent("Base/1"), "Base/1 Mail/2");
    }

    #[test]
    fn preferences_default_to_scripting_on_and_no_minimum_font() {
        let p = WebPreferences::default();
        assert!(p.javascript_enabled && p.fraudulent_website_warning_enabled);
        assert!(!p.javascript_can_open_windows_automatically);
        assert_eq!(p.minimum_font_size, 0.0);
    }

    #[derive(Default)]
    struct Panels {
        alerts: Vec<String>,
        confirm: bool,
        input: Option<String>,
    }
    impl UiDelegate for Panels {
        fn run_javascript_alert_panel(&mut self, m: &str) {
            self.alerts.push(m.into());
        }
        fn run_javascript_confirm_panel(&mut self, _m: &str) -> bool {
            self.confirm
        }
        fn run_javascript_text_input_panel(&mut self, _p: &str, default: &str) -> Option<String> {
            self.input.clone().map(|t| format!("{default}{t}"))
        }
    }

    #[test]
    fn dialog_replies_follow_the_delegate_answers() {
        let mut p = Panels { confirm: true, input: Some("!".into()), ..Default::default() };
        assert_eq!(answer_script_dialog(&mut p, PanelKind::Alert, "hi", ""), (true, String::new()));
        assert_eq!(p.alerts, vec!["hi"]);
        assert_eq!(answer_script_dialog(&mut p, PanelKind::Confirm, "sure?", ""), (true, String::new()));
        assert_eq!(answer_script_dialog(&mut p, PanelKind::Prompt, "name?", "bob"), (true, "bob!".to_string()));
        p.confirm = false;
        p.input = None;
        assert_eq!(answer_script_dialog(&mut p, PanelKind::Confirm, "sure?", ""), (false, String::new()));
        assert_eq!(answer_script_dialog(&mut p, PanelKind::Prompt, "name?", "bob"), (false, String::new()));
    }

    #[test]
    fn a_view_without_a_delegate_refuses_confirms_and_file_choosers() {
        struct None_;
        impl UiDelegate for None_ {}
        let mut d = None_;
        assert!(!d.run_javascript_confirm_panel("x"));
        assert_eq!(d.run_javascript_text_input_panel("p", "d"), None);
        assert_eq!(d.run_open_panel(&OpenPanelParameters::default()), None);
    }

    #[test]
    fn find_counts_matches_with_case_options() {
        assert_eq!(find_matches("Abc abc ABC", "abc", false), vec![0, 4, 8]);
        assert_eq!(find_matches("Abc abc ABC", "abc", true), vec![4]);
        assert!(find_matches("abc", "", false).is_empty());
        assert_eq!(find_matches("aaaa", "aa", true), vec![0, 2]);
    }

    #[test]
    fn find_steps_through_matches_and_wraps() {
        let fwd = FindConfiguration::default();
        let (mut s, first) = FindSession::begin(3, &fwd);
        assert_eq!(first, FindResult { match_found: true, match_index: Some(0), match_count: 3 });
        assert_eq!(s.step(&fwd).match_index, Some(1));
        assert_eq!(s.step(&fwd).match_index, Some(2));
        assert_eq!(s.step(&fwd).match_index, Some(0));
        let back = FindConfiguration { backwards: true, ..fwd };
        assert_eq!(s.step(&back).match_index, Some(2));
        assert_eq!(s.step(&back).match_index, Some(1));
    }

    #[test]
    fn find_without_wrap_stops_at_the_end_and_empty_finds_nothing() {
        let cfg = FindConfiguration { wraps: false, ..FindConfiguration::default() };
        let (mut s, _) = FindSession::begin(2, &cfg);
        s.step(&cfg);
        let end = s.step(&cfg);
        assert!(!end.match_found);
        assert_eq!(end.match_index, Some(1));
        let (mut none, r) = FindSession::begin(0, &cfg);
        assert_eq!(r, FindResult { match_found: false, match_index: None, match_count: 0 });
        assert!(!none.step(&cfg).match_found);
        let (_, last) = FindSession::begin(4, &FindConfiguration { backwards: true, ..cfg });
        assert_eq!(last.match_index, Some(3));
    }
}
