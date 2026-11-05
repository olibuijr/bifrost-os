//! Recently deleted reminders, kept in a persistent store beside the reminders file. A deletion keeps the reminder (and
//! its subtasks) here, so it survives a restart and can be taken back out. The store holds each reminder as its own
//! `R` line (see `store::Codec`), in the order they were deleted. Delete Permanently (`take` without putting the reminder
//! back) and Delete All (`purge_all`) empty it for good.
//!
//! A kept reminder is addressed by its store row (`Kept::row`), never by its reminder id: two kept reminders can share a
//! reminder id (one deleted, then a new reminder given the same id and deleted too), and the row is unique for the
//! store's life (E2E N3).

use crate::model::Reminder;
use bifrost_app::controls::managed_context::ManagedContext;
use bifrost_app::controls::persistent_store;
use std::io;
use std::path::{Path, PathBuf};

/// The store file's name, placed beside the reminders file.
pub const FILE: &str = "recently-deleted.store";

/// The store path for a reminders file at `reminders_path`.
pub fn path_beside(reminders_path: &Path) -> PathBuf {
    reminders_path.with_file_name(FILE)
}

/// Keeps the reminders a deletion removed.
pub fn keep(path: &Path, removed: &[Reminder]) -> io::Result<()> {
    let mut context: ManagedContext<Reminder> = persistent_store::load(path)?;
    for reminder in removed {
        context.insert(reminder.clone());
    }
    context.save();
    persistent_store::write(path, &context)
}

/// A kept reminder and its store row. The row is unique and never reused; the reminder's own id may repeat.
#[derive(Clone, Debug, PartialEq)]
pub struct Kept {
    pub row: u64,
    pub reminder: Reminder,
}

/// The kept reminders with their rows, oldest deletion first.
pub fn kept_rows(path: &Path) -> io::Result<Vec<Kept>> {
    let context: ManagedContext<Reminder> = persistent_store::load(path)?;
    Ok(context.saved_state().into_iter().map(|(row, reminder)| Kept { row, reminder }).collect())
}

/// The kept reminders, oldest deletion first.
pub fn kept(path: &Path) -> io::Result<Vec<Reminder>> {
    Ok(kept_rows(path)?.into_iter().map(|k| k.reminder).collect())
}

/// Takes the kept reminder in store row `row` out of the store and returns it, for restoring or purging it.
pub fn take(path: &Path, row: u64) -> io::Result<Option<Reminder>> {
    let mut context: ManagedContext<Reminder> = persistent_store::load(path)?;
    let Ok(reminder) = context.delete(row) else {
        return Ok(None);
    };
    context.save();
    persistent_store::write(path, &context)?;
    Ok(Some(reminder))
}

/// The undo of a deletion: takes back out of the store the reminders it kept, each the newest row holding exactly
/// that reminder. Returns how many rows went.
pub fn take_back(path: &Path, removed: &[Reminder]) -> io::Result<usize> {
    let mut context: ManagedContext<Reminder> = persistent_store::load(path)?;
    let mut rows = context.saved_state();
    let mut gone = 0;
    for reminder in removed {
        let Some(at) = rows.iter().rposition(|(_, r)| r == reminder) else { continue };
        let (row, _) = rows.remove(at);
        if context.delete(row).is_ok() {
            gone += 1;
        }
    }
    if gone > 0 {
        context.save();
        persistent_store::write(path, &context)?;
    }
    Ok(gone)
}

/// Delete All: empties the store and returns the reminders it held, so their pictures can go too.
pub fn purge_all(path: &Path) -> io::Result<Vec<Reminder>> {
    let gone = kept(path)?;
    if gone.is_empty() {
        return Ok(gone);
    }
    persistent_store::write(path, &ManagedContext::<Reminder>::new())?;
    Ok(gone)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("reminders-archive-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn the_store_sits_beside_the_reminders_file() {
        assert_eq!(path_beside(Path::new("/data/reminders/reminders.txt")), PathBuf::from("/data/reminders/recently-deleted.store"));
    }

    #[test]
    fn deleted_reminders_are_kept_in_deletion_order_across_a_reload() {
        let dir = scratch("keep");
        let path = dir.join(FILE);
        keep(&path, &[Reminder::new(3, "Parent"), Reminder::new(4, "Subtask\twith a tab")]).unwrap();
        keep(&path, &[Reminder::new(9, "Later")]).unwrap();
        let kept_now = kept(&path).unwrap();
        assert_eq!(kept_now.iter().map(|r| r.id).collect::<Vec<_>>(), vec![3, 4, 9]);
        assert_eq!(kept_now[1].title, "Subtask\twith a tab");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_taken_reminder_leaves_the_store() {
        let dir = scratch("take");
        let path = dir.join(FILE);
        keep(&path, &[Reminder::new(3, "Keep"), Reminder::new(5, "Restore me")]).unwrap();
        let row = kept_rows(&path).unwrap()[1].row;
        let back = take(&path, row).unwrap().expect("kept");
        assert_eq!(back.title, "Restore me");
        assert_eq!(kept(&path).unwrap().iter().map(|r| r.id).collect::<Vec<_>>(), vec![3]);
        assert_eq!(take(&path, row).unwrap(), None, "already taken");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn kept_reminders_sharing_a_reminder_id_are_told_apart_by_their_row() {
        // E2E N3: ids restart at 1 when the list is empty, so two kept reminders can both be reminder 1
        let dir = scratch("same-id");
        let path = dir.join(FILE);
        keep(&path, &[Reminder::new(1, "E2E test list")]).unwrap();
        keep(&path, &[Reminder::new(1, "E2E rem")]).unwrap();
        let rows = kept_rows(&path).unwrap();
        assert_ne!(rows[0].row, rows[1].row, "each kept reminder has its own row");
        assert_eq!(take(&path, rows[1].row).unwrap().map(|r| r.title), Some("E2E rem".to_string()));
        assert_eq!(kept(&path).unwrap().iter().map(|r| r.title.as_str()).collect::<Vec<_>>(), ["E2E test list"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn take_back_removes_the_newest_row_holding_each_reminder() {
        let dir = scratch("take-back");
        let path = dir.join(FILE);
        let older = Reminder::new(1, "Same");
        keep(&path, &[older.clone(), Reminder::new(2, "Other")]).unwrap();
        keep(&path, &[older.clone()]).unwrap();
        assert_eq!(take_back(&path, &[older.clone()]).unwrap(), 1);
        let rows = kept_rows(&path).unwrap();
        assert_eq!(rows.iter().map(|k| k.reminder.title.as_str()).collect::<Vec<_>>(), ["Same", "Other"], "the older keep stays");
        assert_eq!(take_back(&path, &[Reminder::new(9, "Never kept")]).unwrap(), 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn delete_all_empties_the_store_and_returns_what_it_held() {
        let dir = scratch("purge");
        let path = dir.join(FILE);
        keep(&path, &[Reminder::new(3, "One"), Reminder::new(5, "Two")]).unwrap();
        let gone = purge_all(&path).unwrap();
        assert_eq!(gone.iter().map(|r| r.id).collect::<Vec<_>>(), vec![3, 5]);
        assert!(kept(&path).unwrap().is_empty());
        assert!(purge_all(&path).unwrap().is_empty(), "an empty store stays empty");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_missing_store_is_empty() {
        let dir = scratch("missing");
        assert!(kept(&dir.join(FILE)).unwrap().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
