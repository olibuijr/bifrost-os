//! Recent Items: the apps the user opened last, listed in the leading system menu (wire6).
//!
//! The list is the shared `bifrost_ui::shared_list::SharedList` model. Every launch through `desk_open` moves the app to
//! the front; the system menu shows the newest ones and Clear Menu empties them. The list is kept in the settings
//! directory (`FILE`), so the recents and favourites survive a restart.
use super::apps::{apps, AppId};
use bifrost_ui::shared_list::SharedList;
use std::path::Path;

/// Recents kept in the list.
pub const CAPACITY: usize = 10;
/// Recents listed in the system menu.
#[cfg(feature = "desktop-wayland")]
pub const SHOWN: usize = 10;

/// The file in the settings directory that keeps the list across sessions.
pub const FILE: &str = "recent-items.txt";

pub fn new() -> SharedList {
    SharedList::new(CAPACITY)
}

/// The list the last session left in `dir`. An empty list when the file is missing or unreadable.
pub fn load(dir: &Path) -> SharedList {
    let text = std::fs::read_to_string(dir.join(FILE)).unwrap_or_default();
    bifrost_ui::shared_list_store::decode(&text, CAPACITY)
}

/// Writes the list to `dir` through a temporary file and a rename, so a crash never leaves half a list.
pub fn save(dir: &Path, list: &SharedList) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let tmp = dir.join(format!("{FILE}.tmp"));
    std::fs::write(&tmp, bifrost_ui::shared_list_store::encode(list))?;
    std::fs::rename(&tmp, dir.join(FILE))
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
#[cfg(test)]
pub fn set_favourite(list: &mut SharedList, app: AppId, on: bool) -> bool {
    let k = key(app);
    if !list.contains(&k) {
        note(list, app);
    }
    list.set_favourite(&k, on)
}

/// The favourite apps in list order. Favourites are not capped, so this can be longer than `CAPACITY`.
#[cfg(test)]
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
        self.recent_persist();
    }

    /// Writes the list to the settings directory once `set_data_dir` has named one (tests never write to disk).
    pub(super) fn recent_persist(&self) {
        if let Some(dir) = &self.recent_dir {
            if let Err(e) = save(dir, &self.recent_list) {
                eprintln!("springboard: recent items: {e}");
            }
        }
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
        self.recent_persist();
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
    fn recents_and_favourites_survive_a_save_and_a_load() {
        let dir = tempfile::tempdir().unwrap();
        assert!(load(dir.path()).is_empty(), "no file yet");
        let mut list = new();
        set_favourite(&mut list, AppId::Terminal, true);
        note(&mut list, AppId::Settings);
        note(&mut list, AppId::External(2));
        save(dir.path(), &list).unwrap();
        let again = load(dir.path());
        assert_eq!(again, list, "the same order, favourites and titles");
        assert_eq!(favourite_apps(&again), vec![AppId::Terminal]);
        let keys: Vec<&str> = again.entries().into_iter().map(|i| i.key.as_str()).collect();
        assert_eq!(keys, vec!["Terminal", "External(2)", "Settings"], "favourites first, then recents newest first");
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

}
