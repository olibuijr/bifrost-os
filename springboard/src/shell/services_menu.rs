//! The Services menu and the Look Up popover: what the text (or link, or file) the user picked can be sent to.
//!
//! Services: Look Up in Dictionary, New Note With Selection, Open URL, Search With quick search, Make New Sticky Note and
//! Show in Files. Each one names a target: a web link goes through the workspace router, a text goes to an app as an
//! `Open` with a `data:` URL (the share sheet's delivery, `sharing::text_url`), a query opens quick search, and a file goes
//! to Files as a document grant. The Look Up popover is the desktop context menu with the term and the same services.
//!
//! The model is plain functions (`for_text`, `for_payloads`, `target`, `look_up_items`) and is tested without a running
//! shell. The shell glue (`impl Shell` below) runs a chosen service. The selection is the text on the shell clipboard,
//! which an SDK app fills with Copy: apps do not publish their selection yet, so a file selection never reaches this menu.

use bifrost_proto::{ContextItem, GrantId, Payload};

/// Context-menu ids of the services start here (`Service::id`). Below the share menu's base, so they never collide.
pub const MENU_BASE: u32 = 0x5345_0000;

/// The dictionary app, which Look Up in Dictionary opens with the word.
pub const DICTIONARY: &str = "is.olibuijr.dictionary.desktop";
/// The notes app, which New Note With Selection opens with the text as a new note.
pub const NOTES: &str = "is.olibuijr.notes.desktop";
/// The sticky notes app, which Make New Sticky Note opens with the text as a new note.
pub const STICKIES: &str = "is.olibuijr.stickies.desktop";

/// The Look Up popover shows a term up to this many characters; a longer one is cut with an ellipsis.
const LOOK_UP_CHARS: usize = 40;

/// One entry of the Services menu, in menu order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Service {
    LookUp,
    NewNote,
    OpenUrl,
    Search,
    StickyNote,
    ShowInFiles,
}

impl Service {
    /// Every service, in the order the menu lists them.
    pub const ALL: [Service; 6] = [Service::LookUp, Service::NewNote, Service::OpenUrl, Service::Search, Service::StickyNote, Service::ShowInFiles];

    /// The menu title.
    pub fn title(self) -> &'static str {
        match self {
            Service::LookUp => "Look Up in Dictionary",
            Service::NewNote => "New Note With Selection",
            Service::OpenUrl => "Open URL",
            Service::Search => "Search With quick search",
            Service::StickyNote => "Make New Sticky Note",
            Service::ShowInFiles => "Show in Files",
        }
    }

    /// The context-menu id of this service.
    pub fn id(self) -> u32 {
        MENU_BASE + self as u32
    }

    /// The service a context-menu id stands for, when it is one of ours.
    pub fn from_id(id: u32) -> Option<Service> {
        Service::ALL.iter().copied().find(|s| s.id() == id)
    }
}

/// Whether `text` is one web link: an http or https address with no spaces or line breaks.
pub fn is_web_link(text: &str) -> bool {
    let t = text.trim();
    t.strip_prefix("https://").or_else(|| t.strip_prefix("http://")).is_some_and(|rest| !rest.is_empty() && !t.chars().any(char::is_whitespace))
}

/// The services for a text: a web link opens in the browser, and other text is looked up when it is one line.
pub fn for_text(text: &str) -> Vec<Service> {
    let text = text.trim();
    if text.is_empty() {
        return Vec::new();
    }
    if is_web_link(text) {
        return vec![Service::OpenUrl, Service::NewNote, Service::Search, Service::StickyNote];
    }
    let mut list = Vec::new();
    if !text.contains('\n') {
        list.push(Service::LookUp);
    }
    list.extend([Service::NewNote, Service::Search, Service::StickyNote]);
    list
}

/// The services for a selection of items: a single text or link as `for_text`, a single file in Files, nothing else.
pub fn for_payloads(items: &[Payload]) -> Vec<Service> {
    match items {
        [Payload::File { .. }] => vec![Service::ShowInFiles],
        [Payload::Text(t)] | [Payload::Url(t)] => for_text(t),
        _ => Vec::new(),
    }
}

/// Where a service sends the selection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    /// a web link, opened in the default browser through the workspace router
    Web(String),
    /// the text goes to `app` as an open (`sharing::text_url`)
    App { app: &'static str, text: String },
    /// quick search opens with the text as its query
    Search(String),
    /// the file goes to Files as a read-only document grant
    Files { grant: GrantId, name: String },
}

/// The target of `service` for `items`, or `None` when the service does not apply to them.
pub fn target(service: Service, items: &[Payload]) -> Option<Target> {
    match (service, items) {
        (Service::ShowInFiles, [Payload::File { grant, name }]) => Some(Target::Files { grant: *grant, name: name.clone() }),
        (_, [Payload::Text(t)]) | (_, [Payload::Url(t)]) => {
            let text = t.trim();
            if text.is_empty() || !for_payloads(items).contains(&service) {
                return None;
            }
            match service {
                Service::LookUp => Some(Target::App { app: DICTIONARY, text: text.to_string() }),
                Service::NewNote => Some(Target::App { app: NOTES, text: text.to_string() }),
                Service::StickyNote => Some(Target::App { app: STICKIES, text: text.to_string() }),
                Service::Search => Some(Target::Search(text.to_string())),
                Service::OpenUrl => Some(Target::Web(text.to_string())),
                Service::ShowInFiles => None,
            }
        }
        _ => None,
    }
}

/// The text cut to the popover's length, with an ellipsis when it is cut.
fn shorten(text: &str) -> String {
    if text.chars().count() <= LOOK_UP_CHARS {
        return text.to_string();
    }
    let head: String = text.chars().take(LOOK_UP_CHARS).collect();
    format!("{head}\u{2026}")
}

/// The Look Up popover for a term: the term (not selectable), then Look Up in Dictionary, Search With quick search and
/// Make New Sticky Note. `None` for an empty text or one with line breaks, which is not a term.
pub fn look_up_items(text: &str) -> Option<Vec<ContextItem>> {
    let term = text.trim();
    if term.is_empty() || term.contains('\n') {
        return None;
    }
    let mut items = vec![ContextItem { id: 0, label: format!("\u{201c}{}\u{201d}", shorten(term)), enabled: false }];
    for service in [Service::LookUp, Service::Search, Service::StickyNote] {
        items.push(ContextItem { id: service.id(), label: service.title().into(), enabled: true });
    }
    Some(items)
}

/// Shell side: runs a Services entry and opens the Look Up popover.
#[cfg(feature = "desktop-wayland")]
impl super::Shell {
    /// Runs `service` on the clipboard text. Nothing happens when the text is not one the service applies to.
    pub(super) fn run_service(&mut self, service: Service, t: f64) {
        let Some(text) = super::clipboard::text() else { return };
        let items = [Payload::Text(text)];
        match target(service, &items) {
            Some(Target::Web(url)) => {
                self.workspace_open_url(&url, t);
            }
            Some(Target::App { app, text }) => self.deliver_text(app, &text, t),
            Some(Target::Search(query)) => self.search_for(&query, t),
            // a file selection comes from a sharing app, so it is never on the clipboard: nothing to show here yet
            Some(Target::Files { .. }) | None => {}
        }
        self.dirty = true;
    }

    /// Brings `app` to the front and sends it `text` as an open, as the share sheet delivers shared text.
    fn deliver_text(&mut self, app: &str, text: &str, t: f64) {
        let Some(to) = super::apphost::slot_of(app) else { return };
        self.open_for_inbound(to, t);
        self.ext_send(to, &bifrost_proto::ToApp::Open { url: super::sharing::text_url(text) });
    }

    /// Opens quick search on the desktop with `query` in its field and its results shown.
    fn search_for(&mut self, query: &str, t: f64) {
        self.open_quicksearch_hw(t);
        if self.quicksearch.is_shown() {
            self.quicksearch.set_query(&self.ui, self.h, query);
        }
    }

    /// Ctrl+Cmd+D: the Look Up popover for the clipboard text, as the desktop context menu under the top of the screen.
    /// False when there is no term to look up, so the key falls through.
    pub(super) fn look_up_popover(&mut self) -> bool {
        let Some(text) = super::clipboard::text() else { return false };
        let Some(items) = look_up_items(&text) else { return false };
        let x = self.w as f32 / 2.0;
        let y = self.desktop_bar_height() as f32;
        self.desktop_context = Some(super::desktop_bar::ContextMenu { slot: 0, x, y, items });
        self.desktop_menu = Some(super::desktop_bar::Menu::Context);
        self.dirty = true;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(s: &str) -> Vec<Payload> {
        vec![Payload::Text(s.into())]
    }

    #[test]
    fn every_service_has_a_title_and_a_distinct_id_below_the_share_menu() {
        let mut titles: Vec<&str> = Service::ALL.iter().map(|s| s.title()).collect();
        titles.sort_unstable();
        titles.dedup();
        assert_eq!(titles.len(), Service::ALL.len());
        assert!(Service::ALL.iter().all(|s| !s.title().is_empty()));
        let mut ids: Vec<u32> = Service::ALL.iter().map(|s| s.id()).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), Service::ALL.len());
        assert!(ids.iter().all(|&id| id < super::super::sharing::MENU_BASE));
        for s in Service::ALL {
            assert_eq!(Service::from_id(s.id()), Some(s));
        }
        assert_eq!(Service::from_id(MENU_BASE + 99), None);
        assert_eq!(Service::from_id(0), None);
    }

    #[test]
    fn a_web_link_offers_open_url_first_and_no_look_up() {
        assert_eq!(for_text("  https://example.com/a?b=c  "), vec![Service::OpenUrl, Service::NewNote, Service::Search, Service::StickyNote]);
        assert!(!is_web_link("https://"), "a scheme with nothing after it is not a link");
        assert!(!is_web_link("see https://example.com"), "a link with other words is text");
        assert!(!is_web_link("ftp://example.com"));
    }

    #[test]
    fn one_line_text_can_be_looked_up_and_several_lines_cannot() {
        assert_eq!(for_text("serendipity"), vec![Service::LookUp, Service::NewNote, Service::Search, Service::StickyNote]);
        assert_eq!(for_text("one\ntwo"), vec![Service::NewNote, Service::Search, Service::StickyNote]);
        assert!(for_text("   \n ").is_empty(), "blank text offers nothing");
    }

    #[test]
    fn a_single_file_is_shown_in_files_and_mixed_items_offer_nothing() {
        let file = Payload::File { grant: [3; 16], name: "a.png".into() };
        assert_eq!(for_payloads(&[file.clone()]), vec![Service::ShowInFiles]);
        assert!(for_payloads(&[file, Payload::Text("x".into())]).is_empty());
        assert!(for_payloads(&[]).is_empty());
    }

    #[test]
    fn look_up_sends_the_trimmed_word_to_the_dictionary() {
        assert_eq!(target(Service::LookUp, &text("  cat ")), Some(Target::App { app: DICTIONARY, text: "cat".into() }));
        assert_eq!(target(Service::LookUp, &text("two words")), Some(Target::App { app: DICTIONARY, text: "two words".into() }));
    }

    #[test]
    fn new_note_and_sticky_go_to_their_apps_with_the_text() {
        assert_eq!(target(Service::NewNote, &text("a note")), Some(Target::App { app: NOTES, text: "a note".into() }));
        assert_eq!(target(Service::StickyNote, &text("a note")), Some(Target::App { app: STICKIES, text: "a note".into() }));
    }

    #[test]
    fn search_takes_the_text_as_its_query() {
        assert_eq!(target(Service::Search, &text("  quarterly report ")), Some(Target::Search("quarterly report".into())));
    }

    #[test]
    fn open_url_applies_only_to_a_web_link() {
        assert_eq!(target(Service::OpenUrl, &text("https://example.com")), Some(Target::Web("https://example.com".into())));
        assert_eq!(target(Service::OpenUrl, &text("plain words")), None);
        assert_eq!(target(Service::OpenUrl, &[Payload::Url("https://example.com/x".into())]), Some(Target::Web("https://example.com/x".into())));
    }

    #[test]
    fn show_in_files_is_for_a_file_and_never_for_text() {
        let file = Payload::File { grant: [9; 16], name: "b.txt".into() };
        assert_eq!(target(Service::ShowInFiles, &[file]), Some(Target::Files { grant: [9; 16], name: "b.txt".into() }));
        assert_eq!(target(Service::ShowInFiles, &text("words")), None);
    }

    #[test]
    fn a_service_that_does_not_apply_has_no_target() {
        assert_eq!(target(Service::LookUp, &text("two\nlines")), None, "several lines are not looked up");
        assert_eq!(target(Service::LookUp, &text("   ")), None, "blank text has no target");
        assert_eq!(target(Service::LookUp, &[]), None);
    }

    #[test]
    fn look_up_popover_lists_the_term_then_the_services() {
        let items = look_up_items("  cat ").unwrap();
        assert_eq!(items[0].label, "\u{201c}cat\u{201d}");
        assert!(!items[0].enabled, "the term is a label, not a choice");
        let ids: Vec<u32> = items[1..].iter().map(|i| i.id).collect();
        assert_eq!(ids, [Service::LookUp.id(), Service::Search.id(), Service::StickyNote.id()]);
        assert!(items[1..].iter().all(|i| i.enabled));
        assert_eq!(items[1].label, "Look Up in Dictionary");
    }

    #[test]
    fn look_up_popover_needs_one_term() {
        assert!(look_up_items("").is_none());
        assert!(look_up_items("   ").is_none());
        assert!(look_up_items("one\ntwo").is_none());
    }

    #[test]
    fn a_long_term_is_cut_with_an_ellipsis() {
        let long = "a".repeat(LOOK_UP_CHARS + 10);
        let items = look_up_items(&long).unwrap();
        assert_eq!(items[0].label, format!("\u{201c}{}\u{2026}\u{201d}", "a".repeat(LOOK_UP_CHARS)));
        let exact = "b".repeat(LOOK_UP_CHARS);
        assert_eq!(look_up_items(&exact).unwrap()[0].label, format!("\u{201c}{exact}\u{201d}"));
    }
}
