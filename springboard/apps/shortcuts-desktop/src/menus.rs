//! The app's menus and the window commands they send. File holds New, Duplicate, Import, Export and Delete; Edit holds
//! Rename and Find; Shortcuts holds Run, the Gallery and one item per shortcut, each with its menu key (Control and
//! Command with a digit) when it has one, which runs it while the app is in front. Window holds All Shortcuts and, in the
//! editor, Previous and Next Shortcut.

use crate::widget::Command;
use bifrost_app::{AppMenu, AppMenuItem};
use springboard::shortcuts as model;
use springboard::shortcuts::Shortcut;

const NEW: u32 = 1;
const DUPLICATE: u32 = 2;
const IMPORT: u32 = 3;
const EXPORT: u32 = 4;
const DELETE: u32 = 5;
const RENAME: u32 = 6;
const FIND: u32 = 7;
const RUN: u32 = 8;
const GALLERY: u32 = 9;
const HELP: u32 = 10;
const WINDOW_ALL: u32 = 11;
const WINDOW_PREVIOUS: u32 = 12;
const WINDOW_NEXT: u32 = 13;
/// The menu id of "Run" for shortcut number `i` is `RUN_BASE + i`.
const RUN_BASE: u32 = 100;
/// The most shortcuts the Shortcuts menu lists.
pub const MENU_RUNS: usize = 12;
/// The key that runs a shortcut from the menu: Control and Command with its menu key (1 to 9), if it has one.
pub fn run_key(key: u8) -> Option<String> {
    (1..=model::MAX_KEY).contains(&key).then(|| format!("ctrl+cmd+{key}"))
}

/// The menus for a list, with the items that need an open shortcut enabled only when `open` is true.
pub fn build(list: &[Shortcut], open: bool) -> Vec<AppMenu> {
    let mut shortcuts = vec![
        AppMenuItem::new(RUN, "Run").key("cmd+r").enabled(open),
        AppMenuItem::new(GALLERY, "Gallery").key("cmd+g"),
    ];
    if !list.is_empty() {
        shortcuts.push(AppMenuItem::separator());
    }
    for (i, s) in list.iter().take(MENU_RUNS).enumerate() {
        let item = AppMenuItem::new(RUN_BASE + i as u32, &format!("Run {}", s.name));
        shortcuts.push(match run_key(s.key) {
            Some(key) => item.key(&key),
            None => item,
        });
    }
    vec![
        AppMenu {
            title: "File".into(),
            items: vec![
                AppMenuItem::new(NEW, "New Shortcut").key("cmd+n"),
                AppMenuItem::new(DUPLICATE, "Duplicate").key("cmd+d").enabled(open),
                AppMenuItem::separator(),
                AppMenuItem::new(IMPORT, "Import Shortcuts...").key("cmd+o"),
                AppMenuItem::new(EXPORT, "Export...").key("cmd+e"),
                AppMenuItem::separator(),
                AppMenuItem::new(DELETE, "Delete Shortcut").enabled(open),
            ],
        },
        AppMenu {
            title: "Edit".into(),
            items: vec![
                AppMenuItem::new(RENAME, "Rename").enabled(open),
                AppMenuItem::separator(),
                AppMenuItem::new(FIND, "Find").key("cmd+f"),
            ],
        },
        AppMenu { title: "Shortcuts".into(), items: shortcuts },
        AppMenu {
            title: "Window".into(),
            items: vec![
                AppMenuItem::new(WINDOW_ALL, "All Shortcuts").key("cmd+1"),
                AppMenuItem::separator(),
                AppMenuItem::new(WINDOW_PREVIOUS, "Previous Shortcut").key("cmd+[").enabled(open),
                AppMenuItem::new(WINDOW_NEXT, "Next Shortcut").key("cmd+]").enabled(open),
            ],
        },
        AppMenu { title: "Help".into(), items: vec![AppMenuItem::new(HELP, "Shortcuts Help")] },
    ]
}

/// The window command a menu item asks for.
pub fn command(id: u32) -> Option<Command> {
    Some(match id {
        NEW => Command::New,
        DUPLICATE => Command::Duplicate,
        IMPORT => Command::Import,
        EXPORT => Command::Export,
        DELETE => Command::Delete,
        RENAME => Command::Rename,
        FIND => Command::Find,
        RUN => Command::RunOpen,
        GALLERY => Command::Gallery,
        HELP => Command::Help,
        WINDOW_ALL => Command::ShowAll,
        WINDOW_PREVIOUS => Command::Adjacent(false),
        WINDOW_NEXT => Command::Adjacent(true),
        id if id >= RUN_BASE && ((id - RUN_BASE) as usize) < MENU_RUNS => Command::Run((id - RUN_BASE) as usize),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_app::controls::keys::KeyCombo;
    use springboard::shortcuts::{defaults, Action};

    fn all_items(menus: &[AppMenu]) -> Vec<&AppMenuItem> {
        menus.iter().flat_map(|m| m.items.iter()).filter(|i| !i.separator).collect()
    }

    #[test]
    fn the_menus_are_file_edit_shortcuts_window_and_help() {
        let titles: Vec<String> = build(&defaults(), false).into_iter().map(|m| m.title).collect();
        assert_eq!(titles, vec!["File", "Edit", "Shortcuts", "Window", "Help"]);
    }

    #[test]
    fn the_window_items_move_between_shortcuts_only_in_the_editor() {
        for open in [false, true] {
            let menus = build(&defaults(), open);
            let window = &menus[3].items;
            assert_eq!(window[0].title, "All Shortcuts");
            assert!(window[0].enabled);
            assert_eq!(window[0].key, "cmd+1");
            assert_eq!(window[2].title, "Previous Shortcut");
            assert_eq!(window[2].enabled, open);
            assert_eq!(window[3].title, "Next Shortcut");
            assert_eq!(window[3].enabled, open);
        }
        assert_eq!(command(WINDOW_ALL), Some(Command::ShowAll));
        assert_eq!(command(WINDOW_PREVIOUS), Some(Command::Adjacent(false)));
        assert_eq!(command(WINDOW_NEXT), Some(Command::Adjacent(true)));
    }

    #[test]
    fn the_help_item_opens_the_help_panel_and_is_always_enabled() {
        let menus = build(&[], false);
        let help = &menus[4].items;
        assert_eq!(help.len(), 1);
        assert_eq!(help[0].title, "Shortcuts Help");
        assert!(help[0].enabled);
        assert_eq!(command(help[0].id), Some(Command::Help));
    }

    #[test]
    fn ids_are_unique_and_every_item_maps_to_a_command() {
        let menus = build(&defaults(), true);
        let items = all_items(&menus);
        let mut ids: Vec<u32> = items.iter().map(|i| i.id).collect();
        ids.sort_unstable();
        let n = ids.len();
        ids.dedup();
        assert_eq!(ids.len(), n, "a menu id is used twice");
        for i in items {
            assert!(command(i.id).is_some(), "{} has no command", i.title);
        }
    }

    #[test]
    fn the_items_that_need_an_open_shortcut_follow_the_editor() {
        let needs = ["Duplicate", "Delete Shortcut", "Rename", "Run"];
        for open in [false, true] {
            let menus = build(&defaults(), open);
            for item in all_items(&menus).into_iter().filter(|i| needs.contains(&i.title.as_str())) {
                assert_eq!(item.enabled, open, "{}", item.title);
            }
            let free = all_items(&menus).into_iter().find(|i| i.title == "New Shortcut").unwrap();
            assert!(free.enabled);
        }
    }

    /// Twenty shortcuts, the first nine with the menu keys 1 to 9.
    fn keyed() -> Vec<Shortcut> {
        (0..20)
            .map(|n| {
                let mut s = Shortcut::new(&format!("S{n}"), 0, vec![Action::Screenshot]);
                if n < 9 {
                    s.key = n as u8 + 1;
                }
                s
            })
            .collect()
    }

    #[test]
    fn every_key_is_a_valid_combination_and_none_is_used_twice() {
        let list = keyed();
        let menus = build(&list, true);
        let keys: Vec<&str> = all_items(&menus).into_iter().map(|i| i.key.as_str()).filter(|k| !k.is_empty()).collect();
        for k in &keys {
            assert!(KeyCombo::parse(k).is_some(), "{k}");
        }
        let mut sorted = keys.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), keys.len(), "two items share a key");
    }

    #[test]
    fn the_run_items_list_the_shortcuts_and_stop_at_the_menu_limit() {
        let list = keyed();
        let menus = build(&list, false);
        let runs: Vec<&AppMenuItem> = menus[2].items.iter().filter(|i| i.title.starts_with("Run S")).collect();
        assert_eq!(runs.len(), MENU_RUNS);
        assert_eq!(runs[0].title, "Run S0");
        assert_eq!(runs[0].key, "ctrl+cmd+1");
        assert_eq!(runs[8].key, "ctrl+cmd+9");
        assert_eq!(runs[9].key, "", "only the first nine have a key");
        assert_eq!(command(runs[3].id), Some(Command::Run(3)));
        assert_eq!(command(RUN_BASE + MENU_RUNS as u32), None);
        assert_eq!(command(0), None);
    }

    #[test]
    fn an_empty_list_has_no_separator_before_the_run_items() {
        let menus = build(&[], false);
        assert_eq!(menus[2].items.len(), 2);
        assert!(menus[2].items.iter().all(|i| !i.separator));
    }

    #[test]
    fn the_file_and_edit_commands_map_to_the_window_commands() {
        assert_eq!(command(NEW), Some(Command::New));
        assert_eq!(command(IMPORT), Some(Command::Import));
        assert_eq!(command(EXPORT), Some(Command::Export));
        assert_eq!(command(RUN), Some(Command::RunOpen));
        assert_eq!(command(FIND), Some(Command::Find));
        assert_eq!(command(GALLERY), Some(Command::Gallery));
    }
}
