//! The shell side of the menu bar client (`bifrost-chrome`'s `menubar`): the `MenuBarModel` built from the shell's
//! state and the handler for a chosen item. Stateless: a menu's id is its index in `menu_buttons()`, and an item's
//! id is its row index in `menu_rows(menu)`, so a `MenuAction` is resolved against the shell as it is when it arrives.
//! Owner: wave G2 agent `chrome-menubar`.

use super::super::desktop_bar::{Menu, Row};
use super::super::{Effect, Shell};
use bifrost_chrome::model::{Menu as BarMenu, MenuBarModel, MenuItem, StatusItem};

/// A row as the model reads it: the text the shell draws (a checked row starts with a check mark), whether it is a
/// separator, its shortcut and whether it has an action.
#[derive(Debug, Clone, PartialEq, Default)]
pub(super) struct RowView {
    pub text: String,
    pub separator: bool,
    pub key: Option<String>,
    pub enabled: bool,
}

impl RowView {
    fn of(row: &Row) -> RowView {
        RowView {
            text: row.text.clone(),
            separator: row.is_separator(),
            key: row.key_text().map(str::to_string),
            enabled: row.action.is_some(),
        }
    }
}

/// The model's items for one menu's rows. The id is the row index; the check mark becomes `checked`.
pub(super) fn items_from(rows: &[RowView]) -> Vec<MenuItem> {
    rows.iter()
        .enumerate()
        .map(|(i, r)| {
            let checked = r.text.starts_with('\u{2713}');
            let label = if r.separator {
                String::new()
            } else {
                r.text.trim_start_matches(|c: char| c == '\u{2713}' || c == ' ').to_string()
            };
            MenuItem {
                id: i as u32,
                label,
                shortcut: r.key.clone().unwrap_or_default(),
                enabled: r.enabled,
                checked,
                submenu: Vec::new(),
            }
        })
        .collect()
}

// the shell menus it reads are built only with the Wayland desktop
#[cfg(feature = "desktop-wayland")]
/// The menu bar model for the shell's current state. Index 0 is the system menu (its title is empty: the chrome
/// draws the mark), the rest of the menus follow the bar order, and everything else on the bar is a status item.
pub(super) fn model(shell: &Shell) -> MenuBarModel {
    let mut menus = Vec::new();
    let mut status = Vec::new();
    for (id, (kind, _x, _w, title)) in shell.menu_buttons().into_iter().enumerate() {
        let id = id as u32;
        let rows: Vec<RowView> = shell.menu_rows(kind).iter().map(RowView::of).collect();
        let items = items_from(&rows);
        match kind {
            Menu::System => menus.push(BarMenu { id, title: String::new(), items }),
            Menu::App
            | Menu::Custom(_)
            | Menu::File
            | Menu::Edit
            | Menu::View
            | Menu::Window
            | Menu::Help => menus.push(BarMenu { id, title, items }),
            _ => status.push(StatusItem { id, symbol: String::new(), text: title, menu: items }),
        }
    }
    let app_name = menus.get(1).map(|m| m.title.clone()).unwrap_or_default();
    MenuBarModel { app_name, menus, status }
}

// the shell menus it reads are built only with the Wayland desktop
#[cfg(feature = "desktop-wayland")]
/// The shell's answer to a `MenuAction`: runs the chosen row's action. Unknown ids do nothing.
pub(super) fn menu_action(shell: &mut Shell, menu: u32, item: u32, t: f64) -> Vec<Effect> {
    let Some((kind, _, _, _)) = shell.menu_buttons().into_iter().nth(menu as usize) else {
        return Vec::new();
    };
    let rows = shell.menu_rows(kind);
    let Some(action) = rows.get(item as usize).and_then(|r| r.action.clone()) else {
        return Vec::new();
    };
    shell.run_menu_action(action, t)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(text: &str, enabled: bool) -> RowView {
        RowView { text: text.into(), separator: false, key: None, enabled }
    }

    #[test]
    fn a_separator_row_has_an_empty_label_and_keeps_its_index_as_id() {
        let rows = vec![row("Open", true), RowView { separator: true, ..RowView::default() }, row("Quit", true)];
        let items = items_from(&rows);
        assert_eq!(items.len(), 3);
        assert_eq!(items[1].label, "");
        assert_eq!(items[1].id, 1);
        assert_eq!(items[2].id, 2);
        assert_eq!(items[2].label, "Quit");
    }

    #[test]
    fn a_check_mark_becomes_checked_and_is_not_part_of_the_label() {
        let items = items_from(&[row("\u{2713} Left", true), row("  Right", true)]);
        assert!(items[0].checked);
        assert_eq!(items[0].label, "Left");
        assert!(!items[1].checked);
        assert_eq!(items[1].label, "Right");
    }

    #[test]
    fn a_row_without_an_action_is_a_disabled_label() {
        let items = items_from(&[row("Recent Items", false), row("Save", true)]);
        assert!(!items[0].enabled);
        assert_eq!(items[0].label, "Recent Items");
        assert!(items[1].enabled);
    }

    #[test]
    fn the_shortcut_text_is_carried_over() {
        let mut r = row("Quit", true);
        r.key = Some("cmd+q".into());
        let items = items_from(&[r, row("Hide", true)]);
        assert_eq!(items[0].shortcut, "cmd+q");
        assert_eq!(items[1].shortcut, "");
    }

    #[test]
    fn a_separator_stays_a_separator_even_with_text() {
        let items = items_from(&[RowView { text: "ignored".into(), separator: true, key: None, enabled: false }]);
        assert_eq!(items[0].label, "");
    }

    #[test]
    fn an_empty_menu_gives_no_items() {
        assert!(items_from(&[]).is_empty());
    }

    #[test]
    fn a_checked_disabled_row_keeps_both_flags() {
        let items = items_from(&[row("\u{2713} Wi-Fi", false)]);
        assert!(items[0].checked);
        assert!(!items[0].enabled);
        assert_eq!(items[0].label, "Wi-Fi");
    }
}
