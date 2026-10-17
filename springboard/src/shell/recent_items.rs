//! Recent Items: the apps the user opened last, listed in the leading system menu (wire6).
//!
//! The list is the shared `bifrost_ui::shared_list::SharedList` model. Every launch through `desk_open` moves the app to
//! the front; the system menu shows the newest ones and Clear Menu empties them. The list is kept across restarts in
//! `<root>/recent-apps.txt` (see [`encode`] and [`save`]): each launch and each Clear Menu rewrites the file.
use super::apps::{apps, AppId};
use bifrost_ui::shared_list::{Item, SharedList};
use std::io;
use std::path::{Path, PathBuf};

/// Recents kept in the list.
pub const CAPACITY: usize = 10;
/// Recents listed in the system menu.
#[cfg(feature = "desktop-wayland")]
pub const SHOWN: usize = 10;

pub fn new() -> SharedList {
    SharedList::new(CAPACITY)
}

/// The file the recent apps are kept in.
pub fn path() -> PathBuf {
    super::apphost::root().join("recent-apps.txt")
}

/// The list the shell starts with: the saved one, or an empty list when nothing is saved or the file cannot be read.
/// Tests start empty and keep no file, so they never depend on the machine's saved list.
pub fn initial() -> SharedList {
    if cfg!(test) {
        new()
    } else {
        load(&path())
    }
}

/// Writes the list to the shell's file. Tests write nothing.
fn persist(list: &SharedList) {
    if !cfg!(test) {
        let _ = save(&path(), list);
    }
}

/// Encodes a list, one line per entry in recency order: `F` for a favourite or `R` for a recent, a tab, the key, a tab
/// and the title. Tabs and line breaks inside a key or title become spaces.
pub fn encode(list: &SharedList) -> String {
    let mut out = String::new();
    for item in list.in_recency_order() {
        let mark = if item.favourite { 'F' } else { 'R' };
        out.push_str(&format!("{mark}\t{}\t{}\n", clean(&item.key), clean(&item.title)));
    }
    out
}

/// Decodes a list written by [`encode`]. A line that does not parse is skipped, a repeated key is dropped, and recents
/// over `CAPACITY` are trimmed.
pub fn decode(text: &str) -> SharedList {
    let items: Vec<Item> = text
        .lines()
        .filter_map(|line| {
            let mut parts = line.splitn(3, '\t');
            let favourite = match parts.next()? {
                "F" => true,
                "R" => false,
                _ => return None,
            };
            let key = parts.next()?;
            let title = parts.next()?;
            Some(Item { key: key.to_string(), title: title.to_string(), favourite })
        })
        .collect();
    SharedList::from_recency_order(CAPACITY, items)
}

fn clean(text: &str) -> String {
    text.chars().map(|c| if matches!(c, '\t' | '\n' | '\r') { ' ' } else { c }).collect()
}

/// Writes the list to `path` atomically, creating the folder when it is missing.
pub fn save(path: &Path, list: &SharedList) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    bifrost_core::fsutil::write_atomic(path, encode(list))
}

/// Reads the list at `path`. A missing or unreadable file gives an empty list.
pub fn load(path: &Path) -> SharedList {
    match std::fs::read_to_string(path) {
        Ok(text) => decode(&text),
        Err(_) => new(),
    }
}

/// The key an app is stored under: the debug form of its `AppId`, unique per catalogue entry.
pub fn key(app: AppId) -> String {
    format!("{app:?}")
}

/// Records a launch of `app` at the front of the list.
pub fn note(list: &mut SharedList, app: AppId) {
    let title = apps().iter().find(|a| a.id == app).map_or_else(|| key(app), |a| a.name.to_string());
    list.add(&key(app), &title);
}

/// Marks or unmarks `app` as a favourite. An app not yet in the list is recorded first. False only when the list
/// refuses the key.
#[cfg(any(feature = "desktop-wayland", test))]
pub fn set_favourite(list: &mut SharedList, app: AppId, on: bool) -> bool {
    let k = key(app);
    if !list.contains(&k) {
        note(list, app);
    }
    list.set_favourite(&k, on)
}

/// The favourite apps in list order. Favourites are not capped, so this can be longer than `CAPACITY`.
#[cfg(any(feature = "desktop-wayland", test))]
pub fn favourite_apps(list: &SharedList) -> Vec<AppId> {
    list.entries()
        .into_iter()
        .filter(|item| item.favourite)
        .filter_map(|item| apps().iter().find(|a| key(a.id) == item.key).map(|a| a.id))
        .collect()
}

/// The app behind the `n`-th entry, newest first. None past the end or when the app is no longer in the catalogue.
#[cfg(any(feature = "desktop-wayland", test))]
pub fn app_at(list: &SharedList, n: usize) -> Option<AppId> {
    let items = list.entries();
    let item = items.get(n)?;
    apps().iter().find(|a| key(a.id) == item.key).map(|a| a.id)
}

impl super::Shell {
    /// Records a launch. Called from `desk_open`, so every way of opening an app counts.
    pub(super) fn recent_note(&mut self, app: AppId) {
        note(&mut self.recent_list, app);
        persist(&self.recent_list);
    }

    /// The titles listed in the system menu, newest first, at most `SHOWN`.
    #[cfg(feature = "desktop-wayland")]
    pub(super) fn recent_titles(&self) -> Vec<String> {
        self.recent_list.entries().into_iter().take(SHOWN).map(|i| i.title.clone()).collect()
    }

    /// The app behind the `n`-th listed entry.
    #[cfg(feature = "desktop-wayland")]
    pub(super) fn recent_app_at(&self, n: usize) -> Option<AppId> {
        app_at(&self.recent_list, n)
    }

    /// Clear Menu: empties the recent apps.
    #[cfg(feature = "desktop-wayland")]
    pub(super) fn recent_clear(&mut self) {
        self.recent_list.clear_recents();
        persist(&self.recent_list);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_launch_moves_the_app_to_the_front_and_the_title_is_the_catalogue_name() {
        let mut list = new();
        note(&mut list, AppId::Settings);
        note(&mut list, AppId::Terminal);
        note(&mut list, AppId::Settings);
        assert_eq!(list.len(), 2, "a reopened app is not listed twice");
        assert_eq!(app_at(&list, 0), Some(AppId::Settings));
        assert_eq!(app_at(&list, 1), Some(AppId::Terminal));
        assert_eq!(app_at(&list, 2), None);
        let titles: Vec<String> = list.entries().into_iter().map(|i| i.title.clone()).collect();
        assert_eq!(titles, vec!["Settings".to_string(), "Term".to_string()]);
    }

    #[test]
    fn reopening_moves_apps_to_the_front_and_clear_empties_the_list() {
        let mut list = new();
        // Distinct apps in order of first launch: Settings, Terminal, Files, Browser. Then Settings and Files again.
        for app in [AppId::Settings, AppId::Terminal, AppId::Files, AppId::Browser, AppId::Settings, AppId::Files] {
            note(&mut list, app);
        }
        // Newest first: Files, Settings, Browser, Terminal.
        assert_eq!(list.len(), 4);
        assert_eq!(app_at(&list, 0), Some(AppId::Files));
        assert_eq!(app_at(&list, 1), Some(AppId::Settings));
        assert_eq!(app_at(&list, 2), Some(AppId::Browser));
        assert_eq!(app_at(&list, 3), Some(AppId::Terminal));
        list.clear_recents();
        assert!(list.is_empty());
        assert_eq!(app_at(&list, 0), None);
    }

    #[test]
    fn favourites_are_marked_listed_in_order_and_unmarked() {
        let mut list = new();
        assert!(set_favourite(&mut list, AppId::Terminal, true));
        assert!(set_favourite(&mut list, AppId::Settings, true));
        assert_eq!(favourite_apps(&list), vec![AppId::Settings, AppId::Terminal]);
        assert!(set_favourite(&mut list, AppId::Terminal, false));
        assert_eq!(favourite_apps(&list), vec![AppId::Settings]);
    }

    #[test]
    fn a_favourite_is_kept_when_recents_are_capped() {
        let mut list = new();
        set_favourite(&mut list, AppId::Terminal, true);
        // More distinct recents than CAPACITY, so the cap drops the oldest recents.
        for n in 0..(CAPACITY as u8 + 3) {
            note(&mut list, AppId::External(n));
        }
        assert_eq!(list.len(), CAPACITY + 1, "the favourite does not count against the cap");
        assert_eq!(favourite_apps(&list), vec![AppId::Terminal]);
        assert!(list.contains(&key(AppId::Terminal)));
        assert!(!list.contains(&key(AppId::External(0))), "the oldest recent is dropped");
    }

    #[test]
    fn the_key_is_the_debug_form_of_the_app_id() {
        assert_eq!(key(AppId::External(3)), "External(3)");
        assert_eq!(key(AppId::AppStore), "AppStore");
    }

    #[cfg(feature = "desktop-wayland")]
    #[test]
    fn the_system_menu_lists_launches_newest_first_and_clear_menu_empties_it() {
        use super::super::desktop_bar::{Action, Menu};
        let mut sh = crate::shell::tests::shell();
        sh.desk = Some(crate::shell::desk::Desk::new());
        sh.screen = crate::shell::Screen::Home;
        sh.anim = None;
        sh.desk_open(AppId::Terminal, 0.);
        sh.desk_open(AppId::Settings, 0.);
        let rows = sh.menu_row_texts(Menu::System);
        let at = rows.iter().position(|t| t == "Recent Items").expect("the Recent Items header is in the system menu");
        assert_eq!(rows[at..at + 4].to_vec(), vec!["Recent Items", "Settings", "Term", "Clear Menu"]);
        // Choosing the second entry opens Term again, which moves it to the front.
        let _ = sh.run_menu_action(Action::Recent(1), 0.);
        let rows = sh.menu_row_texts(Menu::System);
        assert_eq!(rows[at..at + 4].to_vec(), vec!["Recent Items", "Term", "Settings", "Clear Menu"]);
        let _ = sh.run_menu_action(Action::ClearRecent, 0.);
        let rows = sh.menu_row_texts(Menu::System);
        assert_eq!(rows[at..at + 2].to_vec(), vec!["Recent Items", "Clear Menu"]);
        assert!(sh.recent_app_at(0).is_none());
    }

    #[test]
    fn the_list_round_trips_through_its_file_format_in_recency_order() {
        let mut list = new();
        note(&mut list, AppId::Settings);
        note(&mut list, AppId::Terminal);
        set_favourite(&mut list, AppId::Settings, true);
        let text = encode(&list);
        assert_eq!(text.lines().count(), 2);
        assert!(text.starts_with("R\t"), "the newest recent comes first");
        let back = decode(&text);
        assert_eq!(back.in_recency_order(), list.in_recency_order());
        assert_eq!(favourite_apps(&back), vec![AppId::Settings]);
        assert_eq!(app_at(&back, 1), Some(AppId::Terminal), "favourites are listed before recents");
    }

    #[test]
    fn a_damaged_line_is_skipped_and_a_repeated_key_is_dropped() {
        let text = "R\tSettings\tSettings\nX\tbroken\tline\nR\tSettings\tAgain\nF\tTerminal\tTerm\nR\tcut\n";
        let list = decode(text);
        let keys: Vec<&str> = list.in_recency_order().iter().map(|i| i.key.as_str()).collect();
        assert_eq!(keys, vec!["Settings", "Terminal"]);
        assert!(list.in_recency_order()[1].favourite);
    }

    #[test]
    fn tabs_and_line_breaks_in_a_title_cannot_split_a_record() {
        let mut list = new();
        list.add("Notes", "Two\tline\ntitle");
        let text = encode(&list);
        assert_eq!(text.lines().count(), 1);
        assert_eq!(decode(&text).in_recency_order()[0].title, "Two line title");
    }

    #[test]
    fn save_writes_the_file_and_load_reads_it_back_or_starts_empty() {
        let dir = std::env::temp_dir().join(format!("bifrost-recent-apps-{}", std::process::id()));
        let path = dir.join("recent-apps.txt");
        assert!(load(&path).is_empty(), "a missing file starts empty");
        let mut list = new();
        note(&mut list, AppId::Settings);
        save(&path, &list).unwrap();
        assert_eq!(load(&path).in_recency_order(), list.in_recency_order());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
