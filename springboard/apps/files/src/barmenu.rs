//! The toolbar's pull-downs and actions: Group and Sort, Share, Tags and the action menu ("..."). A click only queues
//! the menu or the share sheet as an [`Effect`]; `SystemState::flush` sends it with the ctx once the input is handled,
//! and a menu choice comes back as `Input::ContextAction(id)` into `Files::context_action` (the Group By, Tags and item
//! ids of `menus.rs`, and the ids below).
use crate::*;

/// Where the toolbar menus' own ids start: above the ids of `menus.rs`, below the extension rows (`MENU_EXT_BASE`).
pub(crate) const BAR_MENU_BASE: u32 = 200;

/// Sort By: the criterion, the same as the View Options panel's buttons.
pub(crate) const MENU_SORT_NAME: u32 = BAR_MENU_BASE;

pub(crate) const MENU_SORT_KIND: u32 = BAR_MENU_BASE + 1;

pub(crate) const MENU_SORT_DATE: u32 = BAR_MENU_BASE + 2;

pub(crate) const MENU_SORT_SIZE: u32 = BAR_MENU_BASE + 3;

/// Sort By: the opposite of the natural order (Z to A, oldest first, smallest first).
pub(crate) const MENU_SORT_REVERSE: u32 = BAR_MENU_BASE + 4;

/// The action menu's Select Items: taps add or remove items from the selection (the old Select button).
pub(crate) const MENU_SELECT_MODE: u32 = BAR_MENU_BASE + 5;

/// The view pop-up's choices, in the order of `VIEWS`: as Icons, as List, as Columns, as Gallery.
pub(crate) const MENU_VIEW_ICONS: u32 = BAR_MENU_BASE + 6;

pub(crate) const MENU_VIEW_LIST: u32 = BAR_MENU_BASE + 7;

pub(crate) const MENU_VIEW_COLUMNS: u32 = BAR_MENU_BASE + 8;

pub(crate) const MENU_VIEW_GALLERY: u32 = BAR_MENU_BASE + 9;

/// Points between a pull-down button's bottom edge and its menu.
const BAR_MENU_GAP: f32 = 4.;

/// The most rows a context menu may have; the shell drops a longer one.
const MAX_MENU_ITEMS: usize = 32;

/// One menu row.
fn row(id: u32, label: &str, enabled: bool) -> ContextItem {
    ContextItem { id, label: label.into(), enabled }
}

/// `label`, with the check mark the other menus of Files put in front of the choice that is on.
fn marked(on: bool, label: &str) -> String {
    if on { format!("✓ {label}") } else { label.to_string() }
}

impl Files {
    /// The Group and Sort pop-up: Group By (None, Name, Kind, Date Modified, Size), Sort By (Name, Kind, Date Modified,
    /// Size) with Reverse Sort Order, and the View Options panel. The choice that is on has a check mark.
    pub(crate) fn group_sort_items(&self) -> Vec<ContextItem> {
        let groups = [
            (MENU_GROUP_NONE, Group::None, "None"),
            (MENU_GROUP_NAME, Group::Name, "Name"),
            (MENU_GROUP_KIND, Group::Kind, "Kind"),
            (MENU_GROUP_DATE, Group::Date, "Date Modified"),
            (MENU_GROUP_SIZE, Group::Size, "Size"),
        ];
        let sorts = [
            (MENU_SORT_NAME, Sort::Name, "Name"),
            (MENU_SORT_KIND, Sort::Kind, "Kind"),
            (MENU_SORT_DATE, Sort::Date, "Date Modified"),
            (MENU_SORT_SIZE, Sort::Size, "Size"),
        ];
        let mut items: Vec<ContextItem> = groups.iter().map(|(id, g, label)| row(*id, &marked(self.vo.group == *g, &format!("Group By {label}")), true)).collect();
        items.extend(sorts.iter().map(|(id, by, label)| row(*id, &marked(self.sort == *by, &format!("Sort By {label}")), true)));
        items.push(row(MENU_SORT_REVERSE, &marked(!self.vo.asc, "Reverse Sort Order"), true));
        items.push(row(MENU_VIEW_OPTIONS, "Show View Options", true));
        items
    }

    /// The view pop-up that stands in for the four view buttons while a query is active: as Icons, as List, as Columns and
    /// as Gallery with the keys of the View menu (Cmd+1 to Cmd+4, which keep working), the view showing checked.
    pub(crate) fn view_items(&self) -> Vec<ContextItem> {
        VIEWS.iter().enumerate().map(|(i, (_, _, name))| ContextItem::new(MENU_VIEW_ICONS + i as u32, &marked(self.view_on(i), name)).key(&format!("cmd+{}", i + 1))).collect()
    }

    /// The action menu ("..."): what can be done with the selection, or with the folder when nothing is selected, then
    /// Select Items and the rows file-manager extensions add.
    pub(crate) fn action_menu_items(&self) -> Vec<ContextItem> {
        let (in_bin, in_recents) = (self.in_bin(), self.in_recents());
        let mut items = if self.selection.is_empty() {
            vec![
                row(MENU_NEW_FOLDER, "New Folder", !in_bin && !in_recents),
                row(MENU_PASTE, "Paste", self.clip.is_some() && !in_recents),
                row(MENU_HIDDEN, if self.show_hidden { "Hide Hidden Files" } else { "Show Hidden Files" }, true),
                row(MENU_REFRESH, "Refresh", true),
                row(MENU_VIEW_OPTIONS, "Show View Options", true),
            ]
        } else {
            let single = self.selection.len() == 1;
            let file = single && self.focus_entry().is_some_and(|e| !e.is_dir);
            vec![
                row(MENU_OPEN, "Open", single),
                row(MENU_QUICK_LOOK, "Preview", single),
                row(MENU_INFO, "Get Info", single),
                row(MENU_OPEN_WITH, "Open With…", file),
                row(MENU_COPY, "Copy", true),
                row(MENU_CUT, "Cut", true),
                row(MENU_DUPLICATE, "Duplicate", !in_bin),
                row(MENU_ALIAS, "Make Alias", !in_bin && !in_recents),
                row(MENU_COMPRESS, "Compress", !in_bin && !in_recents),
                row(MENU_TAGS, "Tags…", true),
                row(MENU_RENAME, "Rename", single && !in_bin),
                if in_bin { row(MENU_PUT_BACK, "Put Back", true) } else { row(MENU_TRASH, "Move to Trash", true) },
            ]
        };
        items.push(row(MENU_SELECT_MODE, &marked(self.select_mode, "Select Items"), true));
        items.extend(self.extension_menu_items(!self.selection.is_empty()));
        items.truncate(MAX_MENU_ITEMS);
        items
    }

    /// A click on one of the toolbar's pull-down or action buttons (any other id does nothing).
    pub(crate) fn bar_action(&mut self, id: u32) {
        match id {
            BAR_VIEW => self.pop_bar_menu(id, self.view_items()),
            BAR_SORT => self.pop_bar_menu(id, self.group_sort_items()),
            BAR_MORE => self.pop_bar_menu(id, self.action_menu_items()),
            BAR_TAGS => self.open_tags(),
            BAR_SHARE => self.share_selection(),
            _ => {}
        }
    }

    /// Queues the menu of toolbar button `id` to open under that button.
    fn pop_bar_menu(&mut self, id: u32, items: Vec<ContextItem>) {
        let Some((x, y)) = self.bar_layout().menu_anchor(id, BAR_MENU_GAP) else { return };
        self.sys.effects.push(Effect::PopUp(x, y, items));
    }

    /// Queues the share menu for the selected files under the Share button.
    fn share_selection(&mut self) {
        let paths = self.selected_paths();
        let Some(r) = self.bar_layout().rect_of(BAR_SHARE) else { return };
        if paths.is_empty() || provider::is_path(&self.path) {
            return;
        }
        self.sys.effects.push(Effect::Share(paths, (r.x, r.y, r.w, r.h)));
    }

    /// Runs a choice of the toolbar menus that `context_action` does not know. False for any other id.
    pub(crate) fn bar_menu_action(&mut self, id: u32) -> bool {
        match id {
            MENU_SORT_NAME | MENU_SORT_KIND | MENU_SORT_DATE | MENU_SORT_SIZE => {
                self.sort = match id {
                    MENU_SORT_KIND => Sort::Kind,
                    MENU_SORT_DATE => Sort::Date,
                    MENU_SORT_SIZE => Sort::Size,
                    _ => Sort::Name,
                };
                // a sort starts in its natural order, as the View Options panel's buttons do
                self.vo.asc = true;
                self.sort_rows();
            }
            MENU_SORT_REVERSE => {
                self.vo.asc = !self.vo.asc;
                self.sort_rows();
            }
            MENU_SELECT_MODE => self.select_mode = !self.select_mode,
            // the view pop-up: the same switches as the four buttons and Cmd+1 to Cmd+4
            MENU_VIEW_ICONS => self.set_view(true, false),
            MENU_VIEW_LIST => self.set_view(false, false),
            MENU_VIEW_COLUMNS => self.set_view(false, true),
            MENU_VIEW_GALLERY => self.set_gallery(),
            _ => return false,
        }
        self.scroll = 0.;
        self.revision += 1;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::{fixture, folder};

    fn labels(items: &[ContextItem]) -> Vec<&str> {
        items.iter().map(|i| i.label.as_str()).collect()
    }

    #[test]
    fn the_group_and_sort_menu_marks_the_choices_that_are_on() {
        let mut files = fixture(vec![]);
        let items = files.group_sort_items();
        assert_eq!(labels(&items)[..5], ["✓ Group By None", "Group By Name", "Group By Kind", "Group By Date Modified", "Group By Size"]);
        assert_eq!(labels(&items)[5..9], ["✓ Sort By Name", "Sort By Kind", "Sort By Date Modified", "Sort By Size"]);
        assert_eq!(items[9].label, "Reverse Sort Order");
        assert_eq!(items.last().map(|i| i.id), Some(MENU_VIEW_OPTIONS));
        files.vo.group = Group::Kind;
        files.sort = Sort::Size;
        files.vo.asc = false;
        let items = files.group_sort_items();
        assert_eq!(items[2].label, "✓ Group By Kind");
        assert_eq!(items[0].label, "Group By None");
        assert_eq!(items[8].label, "✓ Sort By Size");
        assert_eq!(items[9].label, "✓ Reverse Sort Order");
        // every id is distinct, so a choice is never taken for another
        let mut ids: Vec<u32> = items.iter().map(|i| i.id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), items.len());
    }

    #[test]
    fn choosing_a_sort_sets_it_in_its_natural_order_and_reverse_flips_it() {
        let mut files = fixture(vec![folder("B"), folder("A")]);
        files.vo.asc = false;
        assert!(files.bar_menu_action(MENU_SORT_SIZE));
        assert_eq!((files.sort, files.vo.asc), (Sort::Size, true));
        assert!(files.bar_menu_action(MENU_SORT_REVERSE));
        assert!(!files.vo.asc);
        assert!(files.bar_menu_action(MENU_SORT_NAME));
        assert_eq!((files.sort, files.vo.asc), (Sort::Name, true));
        assert!(!files.select_mode);
        assert!(files.bar_menu_action(MENU_SELECT_MODE));
        assert!(files.select_mode);
        assert!(!files.bar_menu_action(MENU_OPEN), "a Files id is not a toolbar menu choice");
    }

    #[test]
    fn the_context_action_route_reaches_the_toolbar_choices() {
        let mut files = fixture(vec![]);
        assert!(files.context_action(MENU_SORT_DATE));
        assert_eq!(files.sort, Sort::Date);
        assert!(files.context_action(MENU_GROUP_NAME));
        assert_eq!(files.vo.group, Group::Name);
    }

    #[test]
    fn the_action_menu_acts_on_the_folder_until_something_is_selected() {
        let mut files = fixture(vec![folder("A"), folder("B")]);
        let folder_items = files.action_menu_items();
        assert!(folder_items.iter().any(|i| i.id == MENU_NEW_FOLDER && i.enabled), "no selection: the folder's menu");
        assert!(!folder_items.iter().any(|i| i.id == MENU_TRASH));
        assert_eq!(folder_items.last().map(|i| i.id), Some(MENU_SELECT_MODE));
        files.select_only(0);
        let item_items = files.action_menu_items();
        assert!(item_items.iter().any(|i| i.id == MENU_TRASH && i.enabled), "a selection: the item's menu");
        assert!(item_items.iter().any(|i| i.id == MENU_OPEN && i.enabled), "one item can be opened");
        files.select_all();
        let many = files.action_menu_items();
        assert!(many.iter().any(|i| i.id == MENU_OPEN && !i.enabled), "two items cannot be opened at once");
        assert!(many.len() <= MAX_MENU_ITEMS);
    }

    #[test]
    fn the_buttons_queue_their_menus_and_the_share_sheet_and_tags_need_a_selection() {
        let mut files = fixture(vec![folder("A")]);
        files.update_controls();
        let enabled = |files: &Files, id: u32| files.bar().items.iter().find(|i| i.id == id).map(|i| i.enabled);
        assert_eq!(enabled(&files, BAR_SHARE), Some(false), "nothing to share");
        assert_eq!(enabled(&files, BAR_TAGS), Some(false), "nothing to tag");
        assert_eq!(enabled(&files, BAR_MORE), Some(true));
        files.bar_action(BAR_SORT);
        files.bar_action(BAR_MORE);
        files.bar_action(BAR_SHARE);
        let queued = std::mem::take(&mut files.sys.effects);
        assert_eq!(queued.len(), 2, "two menus, and no share sheet without a selection");
        let sort = files.bar_layout().rect_of(BAR_SORT).unwrap();
        match &queued[0] {
            Effect::PopUp(x, y, items) => {
                assert_eq!((*x, *y), (sort.x, sort.y + sort.h + BAR_MENU_GAP), "the menu opens under its button");
                assert_eq!(items.len(), files.group_sort_items().len());
            }
            other => panic!("the first effect is the pop-up, got {other:?}"),
        }
        files.select_only(0);
        assert_eq!(enabled(&files, BAR_SHARE), Some(true));
        assert_eq!(enabled(&files, BAR_TAGS), Some(true));
        files.bar_action(BAR_SHARE);
        files.bar_action(BAR_TAGS);
        assert!(matches!(files.sys.effects.last(), Some(Effect::Share(paths, _)) if paths == &vec![PathBuf::from("/fixture/A")]));
        assert!(files.tag_panel, "Tags opens the tag panel for the selection");
    }

    #[test]
    fn the_view_pop_up_checks_the_current_view_and_a_choice_switches_to_it() {
        let mut files = fixture(vec![folder("A")]);
        let items = files.view_items();
        assert_eq!(items.iter().map(|i| i.title()).collect::<Vec<_>>(), ["✓ as Icons", "as List", "as Columns", "as Gallery"]);
        assert_eq!(items.iter().map(|i| i.key_combo()).collect::<Vec<_>>(), [Some("cmd+1"), Some("cmd+2"), Some("cmd+3"), Some("cmd+4")], "the keys of the View menu, which keep working");
        assert_eq!(items.iter().map(|i| i.id).collect::<Vec<_>>(), [MENU_VIEW_ICONS, MENU_VIEW_LIST, MENU_VIEW_COLUMNS, MENU_VIEW_GALLERY]);
        assert!(MENU_VIEW_GALLERY < crate::extensions::MENU_EXT_BASE, "below the extension rows");
        assert!(files.bar_menu_action(MENU_VIEW_COLUMNS));
        assert!(files.columns && !files.grid && !files.vo.gallery);
        assert_eq!(files.view_items()[2].title(), "✓ as Columns");
        assert_eq!(files.view_items()[0].title(), "as Icons");
        assert!(files.context_action(MENU_VIEW_GALLERY), "the context route reaches the pop-up's choices");
        assert!(files.vo.gallery && !files.columns);
        assert!(files.context_action(MENU_VIEW_LIST));
        assert!(files.is_list());
        assert!(files.context_action(MENU_VIEW_ICONS));
        assert!(files.grid && !files.vo.gallery);
    }

    #[test]
    fn the_view_pop_up_opens_under_its_button_only_while_a_query_is_active() {
        let mut files = fixture(vec![folder("A")]);
        files.bar_action(BAR_VIEW);
        assert!(files.sys.effects.is_empty(), "no query: the four buttons are on the bar and there is no pop-up");
        files.query.push_str("a");
        files.update_controls();
        let view = files.bar_layout().rect_of(BAR_VIEW).unwrap();
        files.click(view.x + view.w / 2., view.y + view.h / 2., 0.);
        let queued = std::mem::take(&mut files.sys.effects);
        match queued.last() {
            Some(Effect::PopUp(x, y, items)) => {
                assert_eq!((*x, *y), (view.x, view.y + view.h + BAR_MENU_GAP), "the menu opens under its button");
                assert_eq!(items.len(), 4);
            }
            other => panic!("a click on the view pop-up queues its menu, got {other:?}"),
        }
    }
}
