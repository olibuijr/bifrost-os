//! Context menus: the item menu, the folder menu and the choices they send back.
use crate::*;

/// Context menu ids; the shell echoes them back as `ContextAction`.
pub(crate) const MENU_OPEN: u32 = 1;

pub(crate) const MENU_QUICK_LOOK: u32 = 2;

pub(crate) const MENU_COPY_NAME: u32 = 3;

pub(crate) const MENU_REFRESH: u32 = 4;

pub(crate) const MENU_RENAME: u32 = 5;

pub(crate) const MENU_COPY: u32 = 6;

pub(crate) const MENU_CUT: u32 = 7;

pub(crate) const MENU_PASTE: u32 = 8;

pub(crate) const MENU_DUPLICATE: u32 = 9;

pub(crate) const MENU_TRASH: u32 = 10;

pub(crate) const MENU_NEW_FOLDER: u32 = 11;

pub(crate) const MENU_HIDDEN: u32 = 12;

pub(crate) const MENU_PUT_BACK: u32 = 13;

pub(crate) const MENU_INFO: u32 = 14;

pub(crate) const MENU_COMPRESS: u32 = 15;

pub(crate) const MENU_EXTRACT: u32 = 16;

pub(crate) const MENU_TAGS: u32 = 17;

pub(crate) const MENU_SIDEBAR: u32 = 18;

pub(crate) const MENU_STATUS_BAR: u32 = 19;

pub(crate) const MENU_GALLERY: u32 = 20;

pub(crate) const MENU_VIEW_OPTIONS: u32 = 21;

pub(crate) const MENU_GROUP_NONE: u32 = 22;

pub(crate) const MENU_GROUP_KIND: u32 = 23;

pub(crate) const MENU_GROUP_DATE: u32 = 24;

pub(crate) const MENU_OPEN_WITH: u32 = 25;

pub(crate) const MENU_COMPRESS_TGZ: u32 = 26;

pub(crate) const MENU_COMPRESS_TXZ: u32 = 27;

pub(crate) const MENU_EMPTY_TRASH: u32 = 28;
pub(crate) const MENU_ALIAS: u32 = 29;
pub(crate) const MENU_COPY_PATH: u32 = 30;
pub(crate) const MENU_SAVE_SEARCH: u32 = 31;
/// File > New Window (⌘N): a second Files window at the home folder.
pub(crate) const MENU_NEW_WINDOW: u32 = 32;
/// View > Group By Name and Group By Size (the panel offers None, Kind and Date).
pub(crate) const MENU_GROUP_NAME: u32 = 33;
pub(crate) const MENU_GROUP_SIZE: u32 = 34;

/// What the menu of the selected items depends on, so its layout can be checked without a window.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct ItemMenu {
    /// exactly one item is selected
    pub(crate) single: bool,
    pub(crate) in_bin: bool,
    pub(crate) in_recents: bool,
    /// the focused item is a file Open With can offer apps for
    pub(crate) open_with: bool,
    /// the focused item is an archive
    pub(crate) expand: bool,
    pub(crate) can_paste: bool,
}

/// What the menu of the folder background depends on.
#[derive(Clone, Copy, Debug)]
pub(crate) struct FolderMenu {
    pub(crate) in_bin: bool,
    pub(crate) can_paste: bool,
    pub(crate) show_hidden: bool,
    pub(crate) sidebar_hidden: bool,
    pub(crate) status_bar: bool,
    pub(crate) group: Group,
    /// a search is typed in a folder that can hold a smart folder
    pub(crate) can_save_search: bool,
}

/// The groups in one menu, a separator between neighbouring groups that have rows.
pub(crate) fn grouped(groups: Vec<Vec<ContextItem>>) -> Vec<ContextItem> {
    let mut out = Vec::new();
    for group in groups.into_iter().filter(|g| !g.is_empty()) {
        if !out.is_empty() {
            out.push(ContextItem::separator());
        }
        out.extend(group);
    }
    out
}

/// The menu of the selected items, grouped as in the reference: Open | Move to Trash | file actions | clipboard | names |
/// Tags. The key equivalents are the chords Files handles itself (`keys.rs`).
pub(crate) fn item_menu(m: ItemMenu) -> Vec<ContextItem> {
    let files_ok = !m.in_bin && !m.in_recents;
    grouped(vec![
        vec![
            ContextItem::new(MENU_OPEN, "Open").enabled(m.single),
            ContextItem::new(MENU_OPEN_WITH, "Open With…").enabled(m.open_with),
        ],
        vec![if m.in_bin { ContextItem::new(MENU_PUT_BACK, "Put Back") } else { ContextItem::new(MENU_TRASH, "Move to Trash").key("cmd+backspace") }],
        vec![
            ContextItem::new(MENU_INFO, "Get Info").enabled(m.single).key("cmd+i"),
            ContextItem::new(MENU_RENAME, "Rename").enabled(m.single && !m.in_bin).key("return"),
            ContextItem::new(MENU_COMPRESS, "Compress").enabled(files_ok),
            ContextItem::new(MENU_COMPRESS_TGZ, "Compress as TAR.GZ").enabled(files_ok),
            ContextItem::new(MENU_COMPRESS_TXZ, "Compress as TAR.XZ").enabled(files_ok),
            ContextItem::new(MENU_EXTRACT, "Expand").enabled(m.expand),
            ContextItem::new(MENU_DUPLICATE, "Duplicate").enabled(!m.in_bin).key("cmd+d"),
            ContextItem::new(MENU_ALIAS, "Make Alias").enabled(files_ok).key("ctrl+cmd+a"),
            ContextItem::new(MENU_QUICK_LOOK, "Preview").enabled(m.single),
        ],
        vec![
            ContextItem::new(MENU_COPY, "Copy").key("cmd+c"),
            ContextItem::new(MENU_CUT, "Cut").key("cmd+x"),
            ContextItem::new(MENU_PASTE, "Paste").enabled(m.can_paste).key("cmd+v"),
        ],
        vec![
            ContextItem::new(MENU_COPY_NAME, "Copy Name").enabled(m.single),
            ContextItem::new(MENU_COPY_PATH, "Copy Item Path").enabled(m.single).key("alt+cmd+c"),
        ],
        vec![ContextItem::new(MENU_TAGS, "Tags…")],
    ])
}

/// The menu of the folder background: new items | Paste | the folder's view and contents | Group By | search and Trash.
pub(crate) fn folder_menu(m: FolderMenu) -> Vec<ContextItem> {
    // the checked Group By row names its mark in the title, as the other Files choices do
    let group = |id: u32, label: &str, this: Group| {
        ContextItem::new(id, &if m.group == this { format!("✓ {label}") } else { label.to_string() })
    };
    // a row whose title names what choosing it does: `when_on` while the setting is on, else `when_off`
    let toggle = |id: u32, on: bool, when_on: &str, when_off: &str| ContextItem::new(id, if on { when_on } else { when_off });
    grouped(vec![
        vec![
            ContextItem::new(MENU_NEW_WINDOW, "New Window"),
            ContextItem::new(MENU_NEW_FOLDER, "New Folder").enabled(!m.in_bin).key("shift+cmd+n"),
        ],
        vec![ContextItem::new(MENU_PASTE, "Paste").enabled(m.can_paste).key("cmd+v")],
        vec![
            toggle(MENU_HIDDEN, m.show_hidden, "Hide Hidden Files", "Show Hidden Files").key("shift+cmd+."),
            ContextItem::new(MENU_REFRESH, "Refresh"),
        ],
        vec![
            toggle(MENU_SIDEBAR, m.sidebar_hidden, "Show Sidebar", "Hide Sidebar").key("ctrl+cmd+s"),
            toggle(MENU_STATUS_BAR, m.status_bar, "Hide Status Bar", "Show Status Bar").key("cmd+/"),
            ContextItem::new(MENU_GALLERY, "as Gallery").key("cmd+4"),
            ContextItem::new(MENU_VIEW_OPTIONS, "Show View Options").key("cmd+j"),
        ],
        vec![
            group(MENU_GROUP_NONE, "Group By None", Group::None),
            group(MENU_GROUP_KIND, "Group By Kind", Group::Kind),
            group(MENU_GROUP_DATE, "Group By Date", Group::Date),
            group(MENU_GROUP_NAME, "Group By Name", Group::Name),
            group(MENU_GROUP_SIZE, "Group By Size", Group::Size),
        ],
        if m.can_save_search { vec![ContextItem::new(MENU_SAVE_SEARCH, "Save Search as Smart Folder")] } else { Vec::new() },
        if m.in_bin { vec![ContextItem::new(MENU_EMPTY_TRASH, "Empty Trash…").key("shift+cmd+backspace")] } else { Vec::new() },
    ])
}

impl Files {
    /// Right-click: selects the item under the pointer and opens the item or folder menu.
    pub(crate) fn context_menu_at(&mut self, ctx: &Ctx, x: f32, y: f32) {
        let hit = self.item_at(x/ctx.ui.s,y/ctx.ui.s);
        if let Some(i) = hit {
            // a right-click outside the selection selects just that item
            if !self.selection.contains(&i) {
                self.select_only(i);
            }
        } else {
            self.selection.clear();
        }
        self.last_click=None;
        let items = if hit.is_some() {
            let single = self.selection.len() == 1;
            item_menu(ItemMenu {
                single,
                in_bin: self.in_bin(),
                in_recents: self.in_recents(),
                open_with: single && self.focus_entry().is_some_and(|e| !e.is_dir),
                expand: single && self.focus_entry().is_some_and(Self::is_archive),
                can_paste: self.clip.is_some(),
            })
        } else {
            folder_menu(FolderMenu {
                in_bin: self.in_bin(),
                can_paste: self.clip.is_some(),
                show_hidden: self.show_hidden,
                sidebar_hidden: self.sidebar_hidden,
                status_bar: self.status_bar,
                group: self.vo.group,
                can_save_search: !self.query.trim().is_empty() && !self.in_recents() && !self.in_bin(),
            })
        };
        // the rows file-manager extensions add for the selection, or for the folder, are a group of their own
        ctx.context_menu(x, y, grouped(vec![items, self.extension_menu_items(hit.is_some())]));
    }
    /// Copy Item Path (Option+Cmd+C): the focused item's full path as text on the clipboard.
    pub(crate) fn copy_item_path(&mut self) {
        if let Some(entry) = self.focus_entry() {
            bifrost_app::kit::clipboard::set_text(&entry.path.to_string_lossy());
        }
    }
    /// Runs a context-menu choice. Returns false for an id that is not ours.
    pub(crate) fn context_action(&mut self, id: u32) -> bool {
        match id {
            MENU_OPEN => if let Some(i) = self.focus() { self.open(i) },
            MENU_QUICK_LOOK => self.quick_look(),
            MENU_INFO => self.get_info(),
            MENU_OPEN_WITH => self.open_with_panel(),
            MENU_COMPRESS => self.compress(archive::Format::Zip),
            MENU_COMPRESS_TGZ => self.compress(archive::Format::TarGz),
            MENU_COMPRESS_TXZ => self.compress(archive::Format::TarXz),
            MENU_TAGS => self.open_tags(),
            MENU_EXTRACT => self.extract(),
            MENU_COPY_NAME => if let Some(entry) = self.focus_entry() { bifrost_app::kit::clipboard::set_text(&entry.name); },
            MENU_COPY_PATH => self.copy_item_path(),
            MENU_ALIAS => self.make_alias(),
            MENU_SAVE_SEARCH => self.save_search(),
            MENU_REFRESH => { let dir = self.path.clone(); self.navigate(dir, false) }
            MENU_RENAME => self.start_rename(),
            MENU_COPY => self.copy_selected(false),
            MENU_CUT => self.copy_selected(true),
            MENU_PASTE => self.paste(),
            MENU_DUPLICATE => self.duplicate(),
            MENU_TRASH => self.trash_selected(),
            MENU_NEW_FOLDER => self.new_folder(),
            MENU_HIDDEN => self.toggle_hidden(),
            MENU_PUT_BACK => self.put_back(),
            MENU_EMPTY_TRASH => self.ask_empty_trash(),
            MENU_SIDEBAR => self.toggle_sidebar(),
            MENU_STATUS_BAR => self.toggle_status_bar(),
            MENU_GALLERY => self.set_gallery(),
            MENU_VIEW_OPTIONS => { self.vo.show = !self.vo.show; self.revision += 1 }
            MENU_GROUP_NONE | MENU_GROUP_KIND | MENU_GROUP_DATE | MENU_GROUP_NAME | MENU_GROUP_SIZE => {
                self.vo.group = match id { MENU_GROUP_KIND => Group::Kind, MENU_GROUP_DATE => Group::Date, MENU_GROUP_NAME => Group::Name, MENU_GROUP_SIZE => Group::Size, _ => Group::None };
                self.sort_rows();
                self.revision += 1;
            }
            // the toolbar menus' own choices (barmenu.rs), then the rows extensions add
            _ => return self.bar_menu_action(id) || self.extension_action(id),
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The menu as one line per row: the title, its key in brackets, `-` for a disabled row, `|` for a separator.
    fn shape(items: &[ContextItem]) -> Vec<String> {
        items
            .iter()
            .map(|i| {
                if i.is_separator() {
                    return "|".to_string();
                }
                let key = i.key_combo().map(|k| format!(" [{k}]")).unwrap_or_default();
                format!("{}{}{}", if i.enabled { "" } else { "-" }, i.title(), key)
            })
            .collect()
    }

    fn one_item() -> ItemMenu {
        ItemMenu { single: true, open_with: true, can_paste: true, ..ItemMenu::default() }
    }

    #[test]
    fn the_item_menu_is_grouped_as_in_the_reference_with_its_key_equivalents() {
        let menu = item_menu(one_item());
        assert_eq!(
            shape(&menu),
            vec![
                "Open", "Open With…", "|", "Move to Trash [cmd+backspace]", "|",
                "Get Info [cmd+i]", "Rename [return]", "Compress", "Compress as TAR.GZ", "Compress as TAR.XZ", "-Expand",
                "Duplicate [cmd+d]", "Make Alias [ctrl+cmd+a]", "Preview", "|",
                "Copy [cmd+c]", "Cut [cmd+x]", "Paste [cmd+v]", "|",
                "Copy Name", "Copy Item Path [alt+cmd+c]", "|", "Tags…",
            ]
        );
        assert_eq!(menu.len(), 23, "18 rows and 5 separators: inside the 32 rows a context menu carries");
        assert!(!menu.first().unwrap().is_separator() && !menu.last().unwrap().is_separator());
    }

    #[test]
    fn the_item_menu_keeps_its_ids_and_enabled_states() {
        let id_of = |menu: &[ContextItem], title: &str| menu.iter().find(|i| i.title() == title).map(|i| (i.id, i.enabled));
        let menu = item_menu(one_item());
        assert_eq!(id_of(&menu, "Open"), Some((MENU_OPEN, true)));
        assert_eq!(id_of(&menu, "Preview"), Some((MENU_QUICK_LOOK, true)));
        assert_eq!(id_of(&menu, "Expand"), Some((MENU_EXTRACT, false)), "only an archive can be expanded");
        assert_eq!(id_of(&menu, "Move to Trash"), Some((MENU_TRASH, true)));
        // several items: the commands for one item are off, the clipboard ones stay
        let many = item_menu(ItemMenu { single: false, can_paste: false, ..ItemMenu::default() });
        assert_eq!(id_of(&many, "Get Info"), Some((MENU_INFO, false)));
        assert_eq!(id_of(&many, "Copy"), Some((MENU_COPY, true)));
        assert_eq!(id_of(&many, "Paste"), Some((MENU_PASTE, false)), "nothing to paste");
        // an archive
        let archive = item_menu(ItemMenu { expand: true, ..one_item() });
        assert_eq!(id_of(&archive, "Expand"), Some((MENU_EXTRACT, true)));
    }

    #[test]
    fn in_the_bin_the_menu_puts_back_and_cannot_rename_compress_or_duplicate() {
        let menu = item_menu(ItemMenu { in_bin: true, ..one_item() });
        let row = |title: &str| menu.iter().find(|i| i.title() == title).map(|i| (i.id, i.enabled));
        assert_eq!(row("Put Back"), Some((MENU_PUT_BACK, true)));
        assert_eq!(row("Move to Trash"), None);
        for title in ["Rename", "Compress", "Compress as TAR.GZ", "Compress as TAR.XZ", "Duplicate", "Make Alias"] {
            assert_eq!(row(title).map(|r| r.1), Some(false), "{title} is off in the bin");
        }
        let recents = item_menu(ItemMenu { in_recents: true, ..one_item() });
        let recent = |title: &str| recents.iter().find(|i| i.title() == title).map(|i| i.enabled);
        assert_eq!((recent("Compress"), recent("Make Alias"), recent("Duplicate")), (Some(false), Some(false), Some(true)));
    }

    fn folder() -> FolderMenu {
        FolderMenu { in_bin: false, can_paste: false, show_hidden: false, sidebar_hidden: false, status_bar: true, group: Group::None, can_save_search: false }
    }

    #[test]
    fn the_folder_menu_is_grouped_with_separators_and_marks_the_group_by_choice() {
        let menu = folder_menu(FolderMenu { group: Group::Kind, ..folder() });
        assert_eq!(
            shape(&menu),
            vec![
                "New Window", "New Folder [shift+cmd+n]", "|",
                "-Paste [cmd+v]", "|",
                "Show Hidden Files [shift+cmd+.]", "Refresh", "|",
                "Hide Sidebar [ctrl+cmd+s]", "Hide Status Bar [cmd+/]", "as Gallery [cmd+4]", "Show View Options [cmd+j]", "|",
                "Group By None", "✓ Group By Kind", "Group By Date", "Group By Name", "Group By Size",
            ]
        );
    }

    #[test]
    fn the_folder_menu_names_the_opposite_of_each_setting_and_ends_with_search_and_trash() {
        let on = folder_menu(FolderMenu { show_hidden: true, sidebar_hidden: true, status_bar: false, can_save_search: true, ..folder() });
        let titles: Vec<&str> = on.iter().filter(|i| !i.is_separator()).map(|i| i.title()).collect();
        assert!(titles.contains(&"Hide Hidden Files") && titles.contains(&"Show Sidebar") && titles.contains(&"Show Status Bar"));
        assert_eq!(on[on.len() - 2].is_separator(), true, "the search row is a group of its own");
        assert_eq!(on.last().unwrap().id, MENU_SAVE_SEARCH);
        let bin = folder_menu(FolderMenu { in_bin: true, ..folder() });
        let empty: Vec<&ContextItem> = bin.iter().filter(|i| i.id == MENU_EMPTY_TRASH).collect();
        assert_eq!(empty.len(), 1, "one Empty Trash row, not two");
        assert_eq!(bin.last().unwrap().id, MENU_EMPTY_TRASH);
        assert_eq!(bin.iter().find(|i| i.id == MENU_NEW_FOLDER).map(|i| i.enabled), Some(false), "no new folder in the bin");
    }

    #[test]
    fn grouped_puts_a_separator_only_between_groups_that_have_rows() {
        let a = || vec![ContextItem::new(1, "A")];
        let b = || vec![ContextItem::new(2, "B"), ContextItem::new(3, "C")];
        assert_eq!(shape(&grouped(vec![a(), Vec::new(), b()])), vec!["A", "|", "B", "C"]);
        assert_eq!(shape(&grouped(vec![Vec::new(), a(), Vec::new()])), vec!["A"], "no separator at either end");
        assert!(grouped(vec![Vec::new()]).is_empty());
    }
}
