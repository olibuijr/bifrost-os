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
        let mut items = if hit.is_some() {
            let in_bin = self.in_bin();
            let single = self.selection.len() == 1;
            vec![
                ContextItem { id: MENU_OPEN, label: "Open".into(), enabled: single },
                ContextItem { id: MENU_QUICK_LOOK, label: "Preview".into(), enabled: single },
                ContextItem { id: MENU_COPY_NAME, label: "Copy Name".into(), enabled: single },
                ContextItem { id: MENU_COPY_PATH, label: "Copy Item Path".into(), enabled: single },
                ContextItem { id: MENU_INFO, label: "Get Info".into(), enabled: single },
                ContextItem { id: MENU_OPEN_WITH, label: "Open With…".into(), enabled: single && self.focus_entry().is_some_and(|e| !e.is_dir) },
                ContextItem { id: MENU_COMPRESS, label: "Compress".into(), enabled: !in_bin && !self.in_recents() },
                ContextItem { id: MENU_COMPRESS_TGZ, label: "Compress as TAR.GZ".into(), enabled: !in_bin && !self.in_recents() },
                ContextItem { id: MENU_COMPRESS_TXZ, label: "Compress as TAR.XZ".into(), enabled: !in_bin && !self.in_recents() },
                ContextItem { id: MENU_TAGS, label: "Tags…".into(), enabled: true },
                ContextItem { id: MENU_EXTRACT, label: "Expand".into(), enabled: single && self.focus_entry().is_some_and(Self::is_archive) },
                ContextItem { id: MENU_RENAME, label: "Rename".into(), enabled: single && !in_bin },
                ContextItem { id: MENU_COPY, label: "Copy".into(), enabled: true },
                ContextItem { id: MENU_CUT, label: "Cut".into(), enabled: true },
                ContextItem { id: MENU_PASTE, label: "Paste".into(), enabled: self.clip.is_some() },
                ContextItem { id: MENU_DUPLICATE, label: "Duplicate".into(), enabled: !in_bin },
                ContextItem { id: MENU_ALIAS, label: "Make Alias".into(), enabled: !in_bin && !self.in_recents() },
                if in_bin {
                    ContextItem { id: MENU_PUT_BACK, label: "Put Back".into(), enabled: true }
                } else {
                    ContextItem { id: MENU_TRASH, label: "Move to Trash".into(), enabled: true }
                },
            ]
        } else {
            let mut items = vec![
                ContextItem { id: MENU_NEW_WINDOW, label: "New Window".into(), enabled: true },
                ContextItem { id: MENU_NEW_FOLDER, label: "New Folder".into(), enabled: !self.in_bin() },
                ContextItem { id: MENU_PASTE, label: "Paste".into(), enabled: self.clip.is_some() },
                ContextItem { id: MENU_HIDDEN, label: if self.show_hidden { "Hide Hidden Files".into() } else { "Show Hidden Files".into() }, enabled: true },
                ContextItem { id: MENU_REFRESH, label: "Refresh".into(), enabled: true },
                ContextItem { id: MENU_SIDEBAR, label: if self.sidebar_hidden { "Show Sidebar".into() } else { "Hide Sidebar".into() }, enabled: true },
                ContextItem { id: MENU_GALLERY, label: "as Gallery".into(), enabled: true },
                ContextItem { id: MENU_VIEW_OPTIONS, label: "Show View Options".into(), enabled: true },
                ContextItem { id: MENU_GROUP_NONE, label: if self.vo.group == Group::None { "✓ Group By None".into() } else { "Group By None".into() }, enabled: true },
                ContextItem { id: MENU_GROUP_KIND, label: if self.vo.group == Group::Kind { "✓ Group By Kind".into() } else { "Group By Kind".into() }, enabled: true },
                ContextItem { id: MENU_GROUP_DATE, label: if self.vo.group == Group::Date { "✓ Group By Date".into() } else { "Group By Date".into() }, enabled: true },
                ContextItem { id: MENU_STATUS_BAR, label: if self.status_bar { "Hide Status Bar".into() } else { "Show Status Bar".into() }, enabled: true },
            ];
            if !self.query.trim().is_empty() && !self.in_recents() && !self.in_bin() {
                items.push(ContextItem { id: MENU_SAVE_SEARCH, label: "Save Search as Smart Folder".into(), enabled: true });
            }
            if self.in_bin() {
                items.push(ContextItem { id: MENU_EMPTY_TRASH, label: "Empty Trash".into(), enabled: true });
            }
            items
        };
        if hit.is_none() && self.in_bin() {
            items.push(ContextItem { id: MENU_EMPTY_TRASH, label: "Empty Trash…".into(), enabled: true });
        }
        ctx.context_menu(x, y, items);
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
            MENU_GROUP_NONE | MENU_GROUP_KIND | MENU_GROUP_DATE => {
                self.vo.group = match id { MENU_GROUP_KIND => Group::Kind, MENU_GROUP_DATE => Group::Date, _ => Group::None };
                self.sort_rows();
                self.revision += 1;
            }
            _ => return false,
        }
        true
    }
}
