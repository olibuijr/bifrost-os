//! Recently Deleted's Delete Permanently and Delete All. A click on a kept reminder selects it (its Restore link restores
//! it); Delete Permanently comes from the row's context menu, the toolbar's "..." menu and Cmd+Backspace, Delete All from
//! both menus. Both ask first in the same alert sheet the completed strip's Clear uses (`completed.rs`): the delete
//! button is destructive and Return does not answer it, Escape and Cancel keep the reminders. A purged reminder leaves
//! the archive store for good, and so do its pictures unless another reminder still uses them.
use super::*;
use bifrost_app::controls::sheet::{ButtonRole, Sheet, SheetButton};
use bifrost_proto::ContextItem;

/// What the alert is about to delete for good: one kept reminder (by its store row, `archive::Kept::row`) or every
/// kept reminder.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Purge {
    One(u64),
    All,
}

/// The rows of the Recently Deleted menu (a right click on a kept reminder, and the toolbar's "...").
pub const CTX_RESTORE: u32 = 5;
pub const CTX_PURGE: u32 = 6;
pub const CTX_PURGE_ALL: u32 = 7;

/// The width of the Restore link's zone at a kept row's trailing edge (points); the title is fitted to the rest.
pub(super) const RESTORE_W: f32 = SPACE_HUGE * 2.0;

/// The question the alert asks before `purge`; `title` is the one reminder's title, `n` how many are kept.
pub(super) fn purge_question(purge: Purge, title: &str, n: usize) -> String {
    match purge {
        Purge::One(_) => format!("Delete \u{201c}{title}\u{201d} permanently?"),
        Purge::All if n == 1 => "Delete 1 reminder permanently?".to_string(),
        Purge::All => format!("Delete {n} reminders permanently?"),
    }
}

impl Reminders {
    /// The store row of the kept reminder selected on the Recently Deleted page, while the page is open and it is
    /// still kept.
    pub fn kept_selected(&self) -> Option<u64> {
        let kept = self.deleted.as_ref()?;
        self.kept_selected.filter(|row| kept.iter().any(|k| k.row == *row))
    }

    /// Whether the Recently Deleted page is open with something on it (Delete All applies).
    pub fn kept_any(&self) -> bool {
        self.deleted.as_ref().is_some_and(|kept| !kept.is_empty())
    }

    /// Whether an alert (Clear or a purge) is up; it blocks the window until it is answered.
    pub(super) fn asking(&self) -> bool {
        self.clear_ask || self.purge_ask.is_some()
    }

    /// The Recently Deleted menu: Restore and Delete Permanently for the selected kept reminder, then Delete All.
    pub(super) fn kept_menu(&self) -> Vec<ContextItem> {
        let one = self.kept_selected().is_some();
        vec![
            ContextItem::new(CTX_RESTORE, "Restore").enabled(one),
            ContextItem::new(CTX_PURGE, "Delete Permanently...").enabled(one),
            ContextItem::separator(),
            ContextItem::new(CTX_PURGE_ALL, "Delete All...").enabled(self.kept_any()),
        ]
    }

    /// Runs row `id` of the Recently Deleted menu. True when something changed.
    pub(super) fn kept_action(&mut self, ui: &Ui, id: u32) -> bool {
        match (id, self.kept_selected()) {
            (CTX_RESTORE, Some(kept)) => {
                self.restore_deleted(ui, kept);
                true
            }
            (CTX_PURGE, Some(kept)) => self.ask_purge(Purge::One(kept)),
            (CTX_PURGE_ALL, _) => self.ask_purge(Purge::All),
            _ => false,
        }
    }

    /// Puts up the alert for `purge` when there is something for it to delete. False when there is not.
    pub(super) fn ask_purge(&mut self, purge: Purge) -> bool {
        let Some(kept) = &self.deleted else { return false };
        let ok = match purge {
            Purge::One(row) => kept.iter().any(|k| k.row == row),
            Purge::All => !kept.is_empty(),
        };
        if ok {
            self.purge_ask = Some(purge);
            self.ask_focus = None;
            self.dirty = true;
        }
        ok
    }

    /// File > Delete Reminder and Cmd+Backspace on the Recently Deleted page: asks before the selected kept reminder
    /// goes for good. False when none is selected.
    pub fn purge_selected(&mut self) -> bool {
        let Some(row) = self.kept_selected() else { return false };
        self.ask_purge(Purge::One(row))
    }

    /// The purge alert while it is up.
    pub(super) fn purge_sheet(&self) -> Option<Sheet> {
        let purge = self.purge_ask?;
        let kept = self.deleted.as_deref().unwrap_or_default();
        let title = match purge {
            Purge::One(row) => kept.iter().find(|k| k.row == row).map(|k| clean(&k.reminder.title)).unwrap_or_default(),
            Purge::All => String::new(),
        };
        let button = match purge {
            Purge::One(_) => "Delete",
            Purge::All => "Delete All",
        };
        let buttons = vec![
            SheetButton::new(completed::ASK_CLEAR, button, ButtonRole::Destructive),
            SheetButton::new(completed::ASK_CANCEL, "Cancel", ButtonRole::Cancel),
        ];
        Some(Sheet::alert(&purge_question(purge, &title, kept.len()), "This cannot be undone.", buttons).with_focus(self.ask_focus))
    }

    /// The alert's delete button: the asked-for kept reminders leave the store, and their pictures with them unless
    /// a reminder (on the list or still kept) uses them.
    pub(super) fn purge_confirmed(&mut self, ui: &Ui) {
        let Some(purge) = self.purge_ask.take() else { return };
        let path = archive::path_beside(&self.path);
        let gone: Vec<Reminder> = match purge {
            Purge::One(row) => archive::take(&path, row).ok().flatten().into_iter().collect(),
            Purge::All => archive::purge_all(&path).unwrap_or_default(),
        };
        if gone.is_empty() {
            self.notice = Some("Could not delete the reminder".to_string());
        }
        let kept = archive::kept(&path).unwrap_or_default();
        for name in gone.iter().flat_map(|r| r.attachments.iter()) {
            let used = |r: &Reminder| r.attachments.contains(name);
            if !self.data.items.iter().any(used) && !kept.iter().any(used) {
                attach::remove(&self.attach_dir, name);
                self.thumbs.remove(name);
            }
        }
        self.refresh_kept();
        self.kept_selected = None;
        self.rebuild(ui);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_purge_alert_names_what_goes() {
        assert_eq!(purge_question(Purge::One(3), "Buy milk", 4), "Delete \u{201c}Buy milk\u{201d} permanently?");
        assert_eq!(purge_question(Purge::All, "", 1), "Delete 1 reminder permanently?");
        assert_eq!(purge_question(Purge::All, "", 3), "Delete 3 reminders permanently?");
    }
}
