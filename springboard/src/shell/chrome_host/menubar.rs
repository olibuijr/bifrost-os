//! The shell side of the menu bar client (`bifrost-chrome`'s `menubar`): the `MenuBarModel` built from the shell's
//! state and the handler for a chosen item. Stateless: a menu's id is its index in `menu_buttons()`, and an item's
//! id is its path of row indices (`item_id`): the row in `menu_rows(menu)`, then for an item in a submenu its row in
//! `submenu_rows` of each submenu on the way, so a `MenuAction` is resolved against the shell as it is when it arrives.
//! Submenus (Recent Items, Services, an app's Font and Text) are nested `MenuItem::submenu`s, not flattened rows.
//! Owner: wave G2 agent `chrome-menubar`.

#[cfg(feature = "desktop-wayland")]
use super::super::desktop_bar::{Menu, Row};
#[cfg(feature = "desktop-wayland")]
use super::super::{Effect, Shell};
#[cfg(feature = "desktop-wayland")]
use bifrost_chrome::model::{Menu as BarMenu, MenuBarModel, MenuItem, StatusItem};

/// A row as the model reads it: the text the shell draws (a checked row starts with a check mark), whether it is a
/// separator, its shortcut, whether it has an action, and the rows of the submenu it opens.
#[cfg(feature = "desktop-wayland")]
#[derive(Debug, Clone, PartialEq, Default)]
pub(super) struct RowView {
    pub text: String,
    pub separator: bool,
    pub key: Option<String>,
    pub enabled: bool,
    pub children: Vec<RowView>,
}

/// Bits of an item id each nesting level takes (`item_id`): a menu level holds fewer rows than this allows.
#[cfg(feature = "desktop-wayland")]
const LEVEL_BITS: u32 = 10;
/// Nesting levels an item id has room for: the menu and two submenus (`proto::MAX_APP_MENU_DEPTH`).
#[cfg(feature = "desktop-wayland")]
const LEVELS: usize = 3;

/// The id of the item at row `row` of nesting level `level` under the item `parent` (the id of its submenu row; 0 at the
/// top). The top level's id is the row itself, as before submenus nested; a deeper level adds `row + 1` in its own bits,
/// so 0 there means "this level is not used". None when the row or the level does not fit the id.
#[cfg(feature = "desktop-wayland")]
pub(super) fn item_id(parent: u32, level: usize, row: usize) -> Option<u32> {
    if level >= LEVELS {
        return None;
    }
    if level == 0 {
        return (row < (1 << LEVEL_BITS)).then_some(row as u32);
    }
    ((row + 1) < (1 << LEVEL_BITS)).then(|| parent | ((row as u32 + 1) << (LEVEL_BITS * level as u32)))
}

/// The row path an item id stands for (`item_id`): the top row, then the row in each submenu.
#[cfg(feature = "desktop-wayland")]
pub(super) fn item_path(id: u32) -> Vec<usize> {
    let mask = (1u32 << LEVEL_BITS) - 1;
    let mut path = vec![(id & mask) as usize];
    for level in 1..LEVELS {
        match (id >> (LEVEL_BITS * level as u32)) & mask {
            0 => break,
            n => path.push(n as usize - 1),
        }
    }
    path
}

#[cfg(feature = "desktop-wayland")]
impl RowView {
    /// The view of `row`; a row that opens a submenu carries the submenu's rows (at most `LEVELS` deep).
    fn of(shell: &Shell, row: &Row, level: usize) -> RowView {
        let children = match row.opens() {
            Some(sub) if level + 1 < LEVELS => shell.submenu_rows(sub).iter().map(|r| RowView::of(shell, r, level + 1)).collect(),
            _ => Vec::new(),
        };
        RowView {
            text: row.text.clone(),
            separator: row.is_separator(),
            key: row.key_text().map(str::to_string),
            enabled: row.action.is_some(),
            children,
        }
    }
}

/// The model's items for one menu's rows. The id is the row's path (`item_id`); the check mark becomes `checked`, and a
/// row with children becomes a submenu item.
#[cfg(feature = "desktop-wayland")]
pub(super) fn items_from(rows: &[RowView]) -> Vec<MenuItem> {
    items_at(rows, 0, 0)
}

#[cfg(feature = "desktop-wayland")]
fn items_at(rows: &[RowView], parent: u32, level: usize) -> Vec<MenuItem> {
    rows.iter()
        .enumerate()
        .filter_map(|(i, r)| Some((item_id(parent, level, i)?, r)))
        .map(|(id, r)| {
            let checked = r.text.starts_with('\u{2713}');
            let label = if r.separator {
                String::new()
            } else {
                r.text.trim_start_matches(|c: char| c == '\u{2713}' || c == ' ').to_string()
            };
            MenuItem {
                id,
                label,
                shortcut: r.key.clone().unwrap_or_default(),
                enabled: r.enabled,
                checked,
                submenu: items_at(&r.children, id, level + 1),
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
        let rows: Vec<RowView> = shell.menu_rows(kind).iter().map(|r| RowView::of(shell, r, 0)).collect();
        let items = items_from(&rows);
        match kind {
            Menu::System => menus.push(BarMenu { id, title: String::new(), items }),
            Menu::App
            | Menu::Custom(_)
            | Menu::File
            | Menu::Edit
            | Menu::Image
            | Menu::View
            | Menu::Window
            | Menu::Help => menus.push(BarMenu { id, title, items }),
            // an app's extra drawn as a symbol carries the symbol's name (its title is empty); a text extra carries its text
            _ => {
                let (symbol, text) = if shell.status_item_is_icon(kind) { (title, String::new()) } else { (String::new(), title) };
                status.push(StatusItem { id, symbol, text, menu: items });
            }
        }
    }
    let app_name = menus.get(1).map(|m| m.title.clone()).unwrap_or_default();
    // the server stamps the generation when it publishes
    MenuBarModel { app_name, menus, status, generation: 0 }
}

// the shell menus it reads are built only with the Wayland desktop
#[cfg(feature = "desktop-wayland")]
/// The shell's answer to a `MenuAction`: runs the chosen row's action, following the item's path into the submenus it
/// is in (`item_path`). Unknown ids do nothing.
pub(super) fn menu_action(shell: &mut Shell, menu: u32, item: u32, t: f64) -> Vec<Effect> {
    let Some((kind, _, _, _)) = shell.menu_buttons().into_iter().nth(menu as usize) else {
        return Vec::new();
    };
    let path = item_path(item);
    let mut rows = shell.menu_rows(kind);
    for (k, &at) in path.iter().enumerate() {
        let Some(row) = rows.get(at) else { return Vec::new() };
        if k + 1 == path.len() {
            let Some(action) = row.action.clone() else { return Vec::new() };
            return shell.run_menu_action(action, t);
        }
        let Some(sub) = row.opens() else { return Vec::new() };
        rows = shell.submenu_rows(sub);
    }
    Vec::new()
}

#[cfg(all(test, feature = "desktop-wayland"))]
mod tests {
    use super::*;

    fn row(text: &str, enabled: bool) -> RowView {
        RowView { text: text.into(), separator: false, key: None, enabled, children: Vec::new() }
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
        let items = items_from(&[RowView { text: "ignored".into(), separator: true, key: None, enabled: false, children: Vec::new() }]);
        assert_eq!(items[0].label, "");
    }

    #[test]
    fn a_row_with_children_is_a_nested_submenu_with_path_ids() {
        let mut font = row("Font", true);
        let mut kern = row("Kern", true);
        kern.children = vec![row("Use Default", true), row("Tighten", true)];
        font.children = vec![row("Show Fonts", true), kern, row("Helvetica", true)];
        let items = items_from(&[row("Bold", true), font]);
        assert_eq!(items[1].label, "Font");
        assert_eq!(items[1].submenu.len(), 3, "the submenu's rows are nested, not flattened");
        let tighten = &items[1].submenu[1].submenu[1];
        assert_eq!(tighten.label, "Tighten");
        assert_eq!(item_path(tighten.id), vec![1, 1, 1], "Font, Kern, Tighten");
        assert_eq!(item_path(items[1].submenu[2].id), vec![1, 2]);
        assert_eq!(item_path(items[0].id), vec![0]);
        assert_eq!(items[0].id, 0, "a top-level item keeps its row index as id");
        let mut ids: Vec<u32> = items.iter().flat_map(|i| std::iter::once(i.id).chain(i.submenu.iter().map(|s| s.id))).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), 5, "every id is unique");
    }

    #[test]
    fn item_ids_round_trip_and_refuse_what_does_not_fit() {
        for (parent, level, row) in [(0, 0, 0), (0, 0, 900), (3, 1, 0), (3, 1, 700), (3 | (5 << 10), 2, 12)] {
            let id = item_id(parent, level, row).unwrap();
            assert_eq!(*item_path(id).last().unwrap(), row);
        }
        assert_eq!(item_id(0, 0, 1 << 10), None);
        assert_eq!(item_id(0, 1, (1 << 10) - 1), None);
        assert_eq!(item_id(0, LEVELS, 0), None);
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
