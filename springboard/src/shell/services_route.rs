//! The Services menu and Look Up on the desktop. The selection is the front window's, as the reference's services act
//! on the front app's selection (the reference services requestor): an external app is asked for it (`ToApp::SelectionRequest`,
//! answered by the SDK from the focused text field or by the app's own `App::selected_text`), and when it says nothing
//! is selected, or does not answer in time, the word at its caret is taken. The clipboard is never the selection: a
//! stale copy is not what the person pointed at. `bifrost_ui::services_menu` models which services take the selection
//! and where each one sends it. This module carries the route out with the shell's own open paths: an app is opened
//! by id and its intent runs over its control socket, a link opens in the browser, a path is shown in Files, and
//! Search opens with the text in its field.

use super::apps::AppId;
use super::{Effect, Screen, Shell};
use bifrost_proto::TextState;
use bifrost_ui::app_services::{self as apps, ServiceDecl};
use bifrost_ui::services_menu::{self as menu, Route, Selection, Service};
use std::path::Path;
use std::time::Duration;

/// The Files app, which shows folders (Show in Files).
const FILES_APP: &str = "is.olibuijr.files.desktop";

/// How long an intent waits for its app's control socket after the app opens.
const INTENT_WAIT: Duration = Duration::from_secs(8);

/// How long the shell waits for the front app's answer to a `SelectionRequest` before it takes the word at the app's
/// caret instead (ms). Short: the chord's popover must not lag, and an app built before the request never answers.
pub const SELECTION_WAIT_MS: f64 = 250.0;

/// What runs on the selection once the front app has said what it is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelectionUse {
    /// a built-in service (Look Up, New Note With Selection, ...)
    Service(Service),
    /// an app's service, by its Services row index (past the built-in ones)
    AppService(usize),
    /// the Look Up chord (Ctrl+Cmd+D): the in-place popover on the term. The Services item opens Dictionary instead.
    LookUpPopover,
}

/// A `SelectionRequest` the shell sent to the app in `slot` and waits on until `deadline` (ms).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SelectionAsk {
    pub request: u32,
    pub slot: u8,
    pub then: SelectionUse,
    pub deadline: f64,
}

/// The selection of a field: the text the app answered when it is not blank, else the selection of the field it last
/// reported (`state`), else the word at that field's caret. None when all of them are blank. Never the clipboard.
pub fn selection_from(answer: &str, state: &TextState) -> Option<Selection> {
    let text = [answer, state.selected_text(), state.word_at_cursor()].into_iter().find(|t| !t.trim().is_empty())?;
    Some(Selection::Text(text.to_string()))
}

/// A service an installed app declares (`[[service]]`) that takes the current selection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppService {
    pub app_id: String,
    pub app_name: String,
    pub decl: ServiceDecl,
    /// The send type the selection carries that the service takes; the call names it.
    pub uti: String,
}

/// The services of the installed apps that take `sel`, in install order.
pub fn app_services_for(sel: &Selection) -> Vec<AppService> {
    let types = apps::selection_types(sel, |t| Path::new(t).is_file());
    let mut out = Vec::new();
    for app in super::apphost::installed().iter() {
        for decl in app.manifest.services.iter().filter(|d| d.accepts(&types)) {
            let Some(uti) = decl.send_type_for(&types) else { continue };
            out.push(AppService { app_id: app.manifest.id.clone(), app_name: app.manifest.name.clone(), decl: decl.clone(), uti: uti.to_string() });
        }
    }
    out
}

/// The App menu's Services rows: the index and the title of each service that takes the selection. Built-in services
/// index `Service::ALL`; an app's service follows them at `Service::ALL.len()` plus its place in [`app_services_for`].
#[cfg_attr(not(feature = "desktop-wayland"), allow(dead_code))]
pub fn menu_entries(sel: Option<Selection>) -> Vec<(u8, String)> {
    let Some(sel) = sel else { return Vec::new() };
    let base = Service::ALL.len();
    let shown: Vec<Service> = Service::ALL.iter().copied().filter(|s| s.accepts(&sel)).collect();
    let mut rows: Vec<(u8, String)> = Service::ALL.iter().enumerate().filter(|(_, s)| shown.contains(s)).map(|(i, s)| (i as u8, s.title().to_string())).collect();
    // the row index is a u8: at most the indexes that remain after the built-in ones. An app's service named like a
    // built-in row (the Dictionary app's "Look Up in Dictionary") is listed once, as that row; the others keep their
    // index in `app_services_for`, which `run_app_service_on` reads.
    let room = u8::MAX as usize - base;
    rows.extend(
        app_services_for(&sel)
            .iter()
            .take(room)
            .enumerate()
            .filter(|(_, s)| !menu::duplicates_built_in(&s.decl.title, &shown))
            .map(|(k, s)| ((base + k) as u8, s.decl.title.clone())),
    );
    rows
}

/// Show in Files shows a folder itself, or the folder that holds a file.
pub fn reveal_folder(path: &str) -> String {
    let p = Path::new(path);
    if p.is_dir() {
        return path.to_string();
    }
    p.parent().map_or_else(|| path.to_string(), |f| f.to_string_lossy().into_owned())
}

impl Shell {
    /// The external app in front, by slot, while its process runs.
    pub(super) fn front_ext_slot(&self) -> Option<u8> {
        if !matches!(self.screen, Screen::App(AppId::External(_))) {
            return None;
        }
        let slot = self.ext_fg?;
        self.ext.get(slot as usize)?.as_ref().filter(|h| h.running()).map(|_| slot)
    }

    /// The front window's selection as the shell holds it now, without asking the app: the built-in Terminal's selected
    /// text, or what the external app in front last reported of its focused field (its selection, else the word at its
    /// caret). The Services menu lists its rows from this; running a service asks the app (`with_selection`).
    pub(super) fn front_selection(&self) -> Option<Selection> {
        if matches!(self.screen, Screen::App(AppId::Terminal)) {
            let text = self.term.as_ref().and_then(|term| term.selected_text())?;
            return (!text.trim().is_empty()).then_some(Selection::Text(text));
        }
        let host = self.ext.get(self.front_ext_slot()? as usize)?.as_ref()?;
        selection_from("", &host.text_state)
    }

    /// Runs `then` on the front window's selection. An external app in front is asked for its selection first
    /// (`ToApp::SelectionRequest`); its answer (`selection_reply`) or the wait running out (`selection_tick`) carries
    /// `then` out. Any other window's selection is used at once.
    pub(super) fn with_selection(&mut self, then: SelectionUse, t: f64) {
        if let Some(slot) = self.front_ext_slot() {
            self.selection_request = self.selection_request.wrapping_add(1).max(1);
            let request = self.selection_request;
            self.ext_send(slot as usize, &bifrost_proto::ToApp::SelectionRequest { request });
            self.selection_ask = Some(SelectionAsk { request, slot, then, deadline: t + SELECTION_WAIT_MS });
            return;
        }
        let sel = self.front_selection();
        self.use_selection(then, sel, t);
    }

    /// The app in `slot` answered `SelectionRequest` `request` with `text`. An answer to an older request, or from
    /// another app than the one asked, is dropped.
    pub(super) fn selection_reply(&mut self, slot: u8, request: u32, text: &str, t: f64) {
        let Some(ask) = self.selection_ask.take_if(|a| a.request == request && a.slot == slot) else { return };
        let sel = self.ext.get(slot as usize).and_then(Option::as_ref).and_then(|h| selection_from(text, &h.text_state));
        self.use_selection(ask.then, sel, t);
    }

    /// The app asked did not answer by the deadline: the word at its caret (or the selection it last reported) is taken.
    pub(super) fn selection_tick(&mut self, t: f64) {
        let Some(ask) = self.selection_ask.take_if(|a| t >= a.deadline) else { return };
        let sel = self.ext.get(ask.slot as usize).and_then(Option::as_ref).and_then(|h| selection_from("", &h.text_state));
        self.use_selection(ask.then, sel, t);
    }

    /// When the shell must wake to stop waiting for an app's selection (`selection_tick`).
    pub(super) fn selection_deadline(&self) -> Option<f64> {
        self.selection_ask.map(|a| a.deadline)
    }

    fn use_selection(&mut self, then: SelectionUse, sel: Option<Selection>, t: f64) {
        let Some(sel) = sel else { return };
        match then {
            SelectionUse::Service(service) => self.run_service_on(service, sel, t),
            SelectionUse::AppService(index) => self.run_app_service_on(index, sel, t),
            SelectionUse::LookUpPopover => {
                let Selection::Text(text) = &sel else { return };
                if let Some(term) = bifrost_ui::look_up::term_of(text) {
                    self.open_look_up(term, Some(sel));
                }
            }
        }
    }

    /// Carries out `service` on the front window's selection (`with_selection`). Nothing happens when the selection
    /// does not take it. Look Up in Dictionary opens the Dictionary app on the term; [`Self::look_up_selection`] is
    /// the chord's in-place popover.
    pub(super) fn run_service(&mut self, service: Service, t: f64) -> Vec<Effect> {
        self.with_selection(SelectionUse::Service(service), t);
        Vec::new()
    }

    /// Ctrl+Cmd+D: the Look Up popover on the front window's selection (`with_selection`), near the selection.
    pub(super) fn look_up_selection(&mut self, t: f64) -> Vec<Effect> {
        self.with_selection(SelectionUse::LookUpPopover, t);
        Vec::new()
    }

    fn run_service_on(&mut self, service: Service, sel: Selection, t: f64) {
        let Some(route) = menu::route(service, &sel) else { return };
        match route {
            Route::Intent { app, intent, param, text } => self.run_intent(app, intent, param, &text, t),
            Route::Search(text) => self.search_for(&text, t),
            Route::OpenUrl(url) => {
                if let Err(e) = self.desktop_open_url(&url, t) {
                    eprintln!("services: {e}");
                }
            }
            // a selected file opens its folder: `cd` cannot enter a file
            Route::Terminal(path) => {
                let folder = reveal_folder(&path);
                self.open_terminal_at(Path::new(&folder), t)
            }
            Route::Reveal(path) => {
                let folder = reveal_folder(&path);
                if let Err(e) = self.desktop_open_folder(FILES_APP, &folder, t) {
                    eprintln!("services: {e}");
                }
            }
        }
    }

    /// Runs the app service at `index` (an index past the built-in services) on the front window's selection
    /// (`with_selection`): opens the provider, then calls its `service.<method>` on a worker thread. A returned value
    /// goes on the pasteboard.
    pub(super) fn run_app_service(&mut self, index: usize, t: f64) {
        self.with_selection(SelectionUse::AppService(index), t);
    }

    fn run_app_service_on(&mut self, index: usize, sel: Selection, t: f64) {
        let Some(svc) = index.checked_sub(Service::ALL.len()).and_then(|k| app_services_for(&sel).into_iter().nth(k)) else { return };
        if !self.desktop_open_app(&svc.app_id, t) {
            eprintln!("services: {} is not installed", svc.app_name);
            return;
        }
        let payload = apps::payload_for(&svc.uti, &sel);
        let line = apps::request_line(&svc.decl.method, &svc.uti, &payload);
        let returns = !svc.decl.return_types.is_empty();
        std::thread::spawn(move || {
            let reply = match super::intents::exchange(&svc.app_id, &line, INTENT_WAIT) {
                Ok(reply) => reply,
                Err(e) => return eprintln!("services: {}: {e}", svc.app_id),
            };
            match apps::parse_reply(&reply) {
                Ok(Some(text)) if returns => {
                    if let Some(plain) = super::pasteboard::set(vec![(apps::PLAIN_TEXT.to_string(), text.into_bytes())]) {
                        bifrost_render::clipboard::set_text(&plain);
                    }
                }
                Ok(_) => {}
                Err(e) => eprintln!("services: {}: {e}", svc.app_id),
            }
        });
    }

    /// Opens `app` and runs its `intent` with one string parameter over its control socket, on a worker thread.
    /// Shared by the Services routes and the Look Up popover's Open in Dictionary row.
    pub(super) fn run_intent(&mut self, app: &'static str, intent: &'static str, param: &'static str, text: &str, t: f64) {
        self.run_intent_with(app, intent, vec![(param, text.to_string())], t);
    }

    /// Like `run_intent`, with several string parameters (`name`, `value`). Used by the Look Up popover's Add to Calendar.
    pub(super) fn run_intent_with(&mut self, app: &'static str, intent: &'static str, params: Vec<(&'static str, String)>, t: f64) {
        if !self.desktop_open_app(app, t) {
            eprintln!("services: {app} is not installed");
            return;
        }
        std::thread::spawn(move || {
            let refs: Vec<(&str, &str)> = params.iter().map(|(k, v)| (*k, v.as_str())).collect();
            if let Err(e) = super::intents::run_with(app, intent, &refs, INTENT_WAIT) {
                eprintln!("services: {app} {intent}: {e}");
            }
        });
    }

    /// Opens Quick Search with `text` in its field.
    pub(super) fn search_for(&mut self, text: &str, t: f64) {
        self.quicksearch_show(t);
        self.quicksearch.field_mut().set_text(&self.ui, text);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn field(text: &str, cursor: u32, anchor: u32) -> TextState {
        TextState { text: text.into(), cursor, anchor, marked: None }
    }

    /// E2E8 1: the app's answer is the selection; with none, the field's own selection, then the word at the caret.
    #[test]
    fn the_answer_wins_then_the_field_selection_then_the_word_at_the_caret() {
        let state = field("the second line", 6, 6);
        assert_eq!(selection_from("second", &TextState::default()), Some(Selection::Text("second".into())));
        assert_eq!(selection_from("first", &field("the second line", 10, 4)), Some(Selection::Text("first".into())), "the app's answer wins");
        assert_eq!(selection_from(" ", &field("the second line", 10, 4)), Some(Selection::Text("second".into())), "a blank answer: the field's selection");
        assert_eq!(selection_from("", &state), Some(Selection::Text("second".into())), "nothing selected: the word at the caret");
        assert_eq!(selection_from("", &field("a  b", 2, 2)), None, "no word at the caret, and never the clipboard");
        assert_eq!(selection_from("", &TextState::default()), None);
    }

    /// E2E8 1: Look Up and the services ask the app in front for its selection instead of reading the clipboard; its
    /// answer runs the service, a late or foreign answer is dropped, and with no answer the word at its caret is taken.
    #[test]
    fn a_service_asks_the_front_app_and_runs_on_its_answer_or_the_word_at_its_caret() {
        let _lock = crate::test_support::global_lock();
        let (mine, theirs) = bifrost_proto::sys::socketpair_pair().unwrap();
        let cfg = bifrost_proto::Configure { w: 100, h: 100, scale: 1., safe_top: 0., safe_bottom: 0., dark: true, text_scale: 1., reduce_motion: false };
        let host = super::super::apphost::AppHost::with_channel("fixture", mine, None, cfg);
        let mut sh = crate::shell::tests::desktop_shell();
        sh.anim = None;
        sh.ext.push(Some(host));
        let slot = 0u8;
        sh.ext_fg = Some(slot);
        sh.screen = Screen::App(AppId::External(slot));
        bifrost_render::clipboard::set_text("e2e-test.pdf");
        sh.ext[slot as usize].as_mut().unwrap().text_state = field("the second line", 6, 6);
        // the chord asks; nothing opens until the app answers
        sh.look_up_selection(1000.0);
        let ask = sh.selection_ask.expect("the front app is asked for its selection");
        let mut asked = Vec::new();
        while let bifrost_proto::sys::Recv::Packet(packet, _) = theirs.recv(false).unwrap() {
            asked.push(bifrost_proto::ToApp::decode(&packet).unwrap());
        }
        assert!(asked.contains(&bifrost_proto::ToApp::SelectionRequest { request: ask.request }), "{asked:?}");
        assert_eq!((ask.slot, ask.then), (slot, SelectionUse::LookUpPopover));
        assert!(sh.look_up.is_none());
        assert_eq!(sh.selection_deadline(), Some(1000.0 + SELECTION_WAIT_MS));
        // an answer to another request, or from another app, is not the one waited on
        sh.selection_reply(slot, ask.request.wrapping_add(1), "other", 1010.0);
        sh.selection_reply(slot.wrapping_add(1), ask.request, "other", 1010.0);
        assert!(sh.selection_ask.is_some() && sh.look_up.is_none());
        sh.selection_reply(slot, ask.request, "second", 1020.0);
        assert!(sh.selection_ask.is_none());
        assert_eq!(sh.look_up.as_ref().map(|p| p.look.term.as_str()), Some("second"), "the selected word, not the clipboard's text");
        sh.look_up = None;
        // no answer by the deadline: the word at the caret
        sh.ext[slot as usize].as_mut().unwrap().text_state = field("look up quokka now", 10, 10);
        sh.look_up_selection(2000.0);
        sh.selection_tick(2000.0 + SELECTION_WAIT_MS - 1.0);
        assert!(sh.look_up.is_none(), "still waiting");
        sh.selection_tick(2000.0 + SELECTION_WAIT_MS);
        assert_eq!(sh.look_up.as_ref().map(|p| p.look.term.as_str()), Some("quokka"));
        sh.look_up = None;
        // nothing selected and no word at the caret: nothing opens, the clipboard is not used
        sh.ext[slot as usize].as_mut().unwrap().text_state = TextState::default();
        sh.look_up_selection(3000.0);
        let ask = sh.selection_ask.unwrap();
        sh.selection_reply(slot, ask.request, "", 3010.0);
        assert!(sh.look_up.is_none());
    }

    /// E2E11 (c): Services > Look Up in Dictionary hands the term to the Dictionary app (its look-up intent), as the
    /// reference's Services item opens Dictionary; only the chord shows the in-place popover.
    #[test]
    fn the_services_look_up_item_goes_to_dictionary_not_the_popover() {
        let _lock = crate::test_support::global_lock();
        let (mine, theirs) = bifrost_proto::sys::socketpair_pair().unwrap();
        let cfg = bifrost_proto::Configure { w: 100, h: 100, scale: 1., safe_top: 0., safe_bottom: 0., dark: true, text_scale: 1., reduce_motion: false };
        let host = super::super::apphost::AppHost::with_channel("fixture", mine, None, cfg);
        let mut sh = crate::shell::tests::desktop_shell();
        sh.anim = None;
        sh.ext.push(Some(host));
        sh.ext_fg = Some(0);
        sh.screen = Screen::App(AppId::External(0));
        sh.run_service(Service::LookUp, 1000.0);
        let ask = sh.selection_ask.expect("the front app is asked for its selection");
        assert_eq!(ask.then, SelectionUse::Service(Service::LookUp));
        sh.selection_reply(0, ask.request, "harbor", 1010.0);
        assert!(sh.look_up.is_none(), "the Services item does not open the popover");
        assert_eq!(
            menu::route(Service::LookUp, &Selection::Text("harbor".into())),
            Some(Route::Intent { app: menu::DICTIONARY_APP, intent: "look-up", param: "word", text: "harbor".into() })
        );
        drop(theirs);
    }

    #[test]
    fn show_in_files_takes_the_folder_or_the_folder_of_a_file() {
        assert_eq!(reveal_folder("/"), "/");
        assert_eq!(reveal_folder("/nonexistent-bifrost-dir/file.txt"), "/nonexistent-bifrost-dir");
    }
}
