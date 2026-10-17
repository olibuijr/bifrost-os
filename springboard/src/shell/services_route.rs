//! The Services menu on the desktop: the selection is the pasteboard's files or text (or the plain clipboard text).
//! `bifrost_ui::services_menu` models which services take it and where each one sends it. This module carries the
//! route out with the shell's own open paths: an app is opened by id and its intent runs over its control socket,
//! a link opens in the browser, a path is shown in Files, and Search opens with the text in its field.

use super::{Effect, Shell};
use bifrost_ui::app_services::{self as apps, ServiceDecl};
use bifrost_ui::services_menu::{self as menu, Route, Selection, Service};
use std::path::Path;
use std::time::Duration;

/// The Files app, which shows folders (Show in Files).
const FILES_APP: &str = "is.olibuijr.files.desktop";

/// How long an intent waits for its app's control socket after the app opens.
const INTENT_WAIT: Duration = Duration::from_secs(8);

/// The selection on the pasteboard, else the plain clipboard text. None when neither has anything.
pub fn selection() -> Option<Selection> {
    let (_, items) = super::pasteboard::snapshot();
    selection_in(&items).or_else(clipboard_text)
}

/// The selection carried by pasteboard `items`: file URLs first (as paths), then plain text.
fn selection_in(items: &[(String, Vec<u8>)]) -> Option<Selection> {
    let item = |uti: &str| items.iter().find(|(k, _)| k == uti).and_then(|(_, d)| String::from_utf8(d.clone()).ok());
    if let Some(uris) = item("public.file-url") {
        let files: Vec<String> = uris.lines().filter_map(menu::file_path_of).collect();
        if !files.is_empty() {
            return Some(Selection::Files(files));
        }
    }
    item("public.utf8-plain-text").filter(|t| !t.trim().is_empty()).map(Selection::Text)
}

/// The plain clipboard text, for a copy the pasteboard did not carry.
fn clipboard_text() -> Option<Selection> {
    let clip = bifrost_render::clipboard::global();
    let text = clip.as_ref()?.text()?;
    (!text.trim().is_empty()).then(|| Selection::Text(text.to_string()))
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
pub fn menu_entries() -> Vec<(u8, String)> {
    let Some(sel) = selection() else { return Vec::new() };
    let base = Service::ALL.len();
    let mut rows: Vec<(u8, String)> = Service::ALL.iter().enumerate().filter(|(_, s)| s.accepts(&sel)).map(|(i, s)| (i as u8, s.title().to_string())).collect();
    // the row index is a u8: at most the indexes that remain after the built-in ones
    let room = u8::MAX as usize - base;
    rows.extend(app_services_for(&sel).iter().take(room).enumerate().map(|(k, s)| ((base + k) as u8, s.decl.title.clone())));
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
    /// Carries out `service` on the current selection. Nothing happens when the selection does not take it.
    pub(super) fn run_service(&mut self, service: Service, t: f64) -> Vec<Effect> {
        let Some(sel) = selection() else { return Vec::new() };
        let Some(route) = menu::route(service, &sel) else { return Vec::new() };
        match route {
            Route::Intent { app, intent, param, text } => self.run_intent(app, intent, param, &text, t),
            Route::LookUp(term) => self.open_look_up(term),
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
        Vec::new()
    }

    /// Runs the app service at `index` (an index past the built-in services) on the current selection: opens the
    /// provider, then calls its `service.<method>` on a worker thread. A returned value goes on the pasteboard.
    pub(super) fn run_app_service(&mut self, index: usize, t: f64) {
        let Some(sel) = selection() else { return };
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
        if !self.desktop_open_app(app, t) {
            eprintln!("services: {app} is not installed");
            return;
        }
        let text = text.to_string();
        std::thread::spawn(move || {
            if let Err(e) = super::intents::run_with(app, intent, &[(param, text.as_str())], INTENT_WAIT) {
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

    fn item(uti: &str, text: &str) -> (String, Vec<u8>) {
        (uti.to_string(), text.as_bytes().to_vec())
    }

    #[test]
    fn plain_text_is_the_selection() {
        assert_eq!(selection_in(&[item("public.utf8-plain-text", "hello")]), Some(Selection::Text("hello".into())));
    }

    #[test]
    fn file_urls_win_over_text_and_decode_to_paths() {
        let items = [item("public.utf8-plain-text", "a.txt"), item("public.file-url", "file:///home/me/My%20Files/a.txt\n")];
        assert_eq!(selection_in(&items), Some(Selection::Files(vec!["/home/me/My Files/a.txt".into()])));
    }

    #[test]
    fn blank_or_missing_items_have_no_selection() {
        assert_eq!(selection_in(&[item("public.utf8-plain-text", "  \n")]), None);
        assert_eq!(selection_in(&[item("public.html", "<b>x</b>")]), None);
        assert_eq!(selection_in(&[]), None);
    }

    #[test]
    fn show_in_files_takes_the_folder_or_the_folder_of_a_file() {
        assert_eq!(reveal_folder("/"), "/");
        assert_eq!(reveal_folder("/nonexistent-bifrost-dir/file.txt"), "/nonexistent-bifrost-dir");
    }
}
