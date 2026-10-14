//! The Services menu: which services take the current selection (text or files), and where each one sends it.
//! A pure model. The shell reads the selection from the pasteboard, lists [`menu_for`] in the App menu, and carries
//! out the [`Route`] that [`route`] returns with the desktop's own open paths (app intents, links, Files, Search).

/// What is selected: plain text, or files and folders as paths.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Selection {
    Text(String),
    Files(Vec<String>),
}

/// The app ids the services target (the ids in each app's manifest).
pub const DICTIONARY_APP: &str = "is.olibuijr.dictionary.desktop";
pub const NOTES_APP: &str = "is.olibuijr.notes.desktop";
pub const STICKIES_APP: &str = "is.olibuijr.stickies.desktop";

/// The services, in menu order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Service {
    LookUp,
    NewNoteWithSelection,
    MakeStickyNote,
    OpenUrl,
    SearchWith,
    ShowInFiles,
    NewTerminalAtFolder,
}

impl Service {
    /// Every service, in menu order. The App menu's Services rows index this list.
    pub const ALL: [Service; 7] = [
        Service::LookUp,
        Service::NewNoteWithSelection,
        Service::MakeStickyNote,
        Service::OpenUrl,
        Service::SearchWith,
        Service::ShowInFiles,
        Service::NewTerminalAtFolder,
    ];

    /// The menu title.
    pub fn title(self) -> &'static str {
        match self {
            Service::LookUp => "Look Up in Dictionary",
            Service::NewNoteWithSelection => "New Note With Selection",
            Service::MakeStickyNote => "Make New Sticky Note",
            Service::OpenUrl => "Open URL",
            Service::SearchWith => "Search With Quick Search",
            Service::ShowInFiles => "Show in Files",
            Service::NewTerminalAtFolder => "New Terminal at Folder",
        }
    }

    /// True when the service takes this selection: text for the text services, a web link for Open URL, paths for
    /// Show in Files. Look Up needs a term (see [`crate::look_up::term_of`]).
    pub fn accepts(self, selection: &Selection) -> bool {
        match (self, selection) {
            (Service::LookUp, Selection::Text(t)) => crate::look_up::term_of(t).is_some(),
            (Service::NewNoteWithSelection | Service::MakeStickyNote | Service::SearchWith, Selection::Text(t)) => !t.trim().is_empty(),
            (Service::OpenUrl, Selection::Text(t)) => web_url(t).is_some(),
            (Service::ShowInFiles | Service::NewTerminalAtFolder, Selection::Files(files)) => !files.is_empty(),
            _ => false,
        }
    }
}

/// Where a service sends the selection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Route {
    /// Opens the app and runs its intent with one string parameter over the app's control socket.
    Intent { app: &'static str, intent: &'static str, param: &'static str, text: String },
    /// Opens the Look Up popover for the term, near the selection (see `look_up`).
    LookUp(String),
    /// Opens Search with the text in its field.
    Search(String),
    /// Opens the link in the browser.
    OpenUrl(String),
    /// Shows the path in Files: the folder itself, or the folder that holds a file (the shell checks which).
    Reveal(String),
    /// Opens a Terminal in the folder of the path: the folder itself, or the folder that holds a file (the shell checks).
    Terminal(String),
}

/// The services that take `selection`, in menu order.
pub fn menu_for(selection: &Selection) -> Vec<Service> {
    Service::ALL.iter().copied().filter(|s| s.accepts(selection)).collect()
}

/// Where `service` sends `selection`, or None when the service does not take it.
pub fn route(service: Service, selection: &Selection) -> Option<Route> {
    if !service.accepts(selection) {
        return None;
    }
    Some(match (service, selection) {
        (Service::LookUp, Selection::Text(t)) => Route::LookUp(crate::look_up::term_of(t)?),
        (Service::NewNoteWithSelection, Selection::Text(t)) => Route::Intent { app: NOTES_APP, intent: "create-note", param: "text", text: t.clone() },
        (Service::MakeStickyNote, Selection::Text(t)) => Route::Intent { app: STICKIES_APP, intent: "create-sticky", param: "text", text: t.clone() },
        (Service::OpenUrl, Selection::Text(t)) => Route::OpenUrl(web_url(t)?),
        (Service::SearchWith, Selection::Text(t)) => Route::Search(t.trim().to_string()),
        (Service::ShowInFiles, Selection::Files(files)) => Route::Reveal(files.first()?.clone()),
        (Service::NewTerminalAtFolder, Selection::Files(files)) => Route::Terminal(files.first()?.clone()),
        _ => return None,
    })
}

/// The link in `text` when it is one web address (http or https, no spaces or control characters).
pub fn web_url(text: &str) -> Option<String> {
    let t = text.trim();
    if !(t.starts_with("http://") || t.starts_with("https://")) || t.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return None;
    }
    Some(t.to_string())
}

/// The local path of a `file://` URL as pasteboards carry it (percent-encoded). None for other schemes or a remote host.
pub fn file_path_of(uri: &str) -> Option<String> {
    let rest = uri.trim().strip_prefix("file://")?;
    let path = rest.strip_prefix("localhost").unwrap_or(rest);
    if !path.starts_with('/') {
        return None;
    }
    percent_decode(path)
}

fn percent_decode(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = s.get(i + 1..i + 3)?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(t: &str) -> Selection {
        Selection::Text(t.to_string())
    }

    #[test]
    fn plain_text_gets_the_text_services_in_menu_order() {
        assert_eq!(menu_for(&text("hello")), vec![Service::LookUp, Service::NewNoteWithSelection, Service::MakeStickyNote, Service::SearchWith]);
    }

    #[test]
    fn a_web_link_also_gets_open_url() {
        assert!(menu_for(&text("https://example.com/a?b=1")).contains(&Service::OpenUrl));
        assert!(!menu_for(&text("ftp://example.com")).contains(&Service::OpenUrl));
        assert!(!menu_for(&text("https://example.com/a b")).contains(&Service::OpenUrl));
    }

    #[test]
    fn files_get_only_show_in_files() {
        assert_eq!(
            menu_for(&Selection::Files(vec!["/home/me/a.txt".into()])),
            vec![Service::ShowInFiles, Service::NewTerminalAtFolder]
        );
        assert!(menu_for(&Selection::Files(Vec::new())).is_empty());
    }

    #[test]
    fn blank_text_gets_no_services() {
        assert!(menu_for(&text("   \n\t")).is_empty());
    }

    #[test]
    fn look_up_takes_the_first_line_as_the_term() {
        assert_eq!(route(Service::LookUp, &text("  cat \nthe second line")), Some(Route::LookUp("cat".into())));
    }

    #[test]
    fn new_note_and_sticky_send_the_whole_text_to_their_intents() {
        assert_eq!(route(Service::NewNoteWithSelection, &text("x\ny")), Some(Route::Intent { app: NOTES_APP, intent: "create-note", param: "text", text: "x\ny".into() }));
        assert_eq!(route(Service::MakeStickyNote, &text("note")), Some(Route::Intent { app: STICKIES_APP, intent: "create-sticky", param: "text", text: "note".into() }));
    }

    #[test]
    fn open_url_and_search_routes() {
        assert_eq!(route(Service::OpenUrl, &text(" https://olibuijr.com ")), Some(Route::OpenUrl("https://olibuijr.com".into())));
        assert_eq!(route(Service::SearchWith, &text(" lava flow ")), Some(Route::Search("lava flow".into())));
    }

    #[test]
    fn a_service_that_does_not_take_the_selection_has_no_route() {
        assert_eq!(route(Service::OpenUrl, &text("ftp://x")), None);
        assert_eq!(route(Service::ShowInFiles, &text("hello")), None);
        assert_eq!(route(Service::NewTerminalAtFolder, &text("hello")), None);
        assert_eq!(
            route(Service::NewTerminalAtFolder, &Selection::Files(vec!["/a/b c".into()])),
            Some(Route::Terminal("/a/b c".into()))
        );
    }

    #[test]
    fn show_in_files_reveals_the_first_path() {
        assert_eq!(route(Service::ShowInFiles, &Selection::Files(vec!["/a/b c.txt".into(), "/z".into()])), Some(Route::Reveal("/a/b c.txt".into())));
    }

    #[test]
    fn file_urls_decode_to_paths() {
        assert_eq!(file_path_of("file:///home/me/My%20Files/a.txt").as_deref(), Some("/home/me/My Files/a.txt"));
        assert_eq!(file_path_of("file://localhost/tmp/x").as_deref(), Some("/tmp/x"));
        assert_eq!(file_path_of("https://example.com"), None);
        assert_eq!(file_path_of("file://otherhost/tmp/x"), None);
        assert_eq!(file_path_of("file:///bad%zz"), None);
    }

    #[test]
    fn titles_match_the_menu_names() {
        assert_eq!(Service::LookUp.title(), "Look Up in Dictionary");
        assert_eq!(Service::ShowInFiles.title(), "Show in Files");
        assert_eq!(Service::ALL.len(), 7);
        assert_eq!(Service::NewTerminalAtFolder.title(), "New Terminal at Folder");
    }
}
