//! Recently deleted reminders, kept in a persistent store beside the reminders file. A deletion keeps the reminder (and
//! its subtasks) here, so it survives a restart and can be taken back out. The store holds each reminder as its own
//! `R` line (see `store::Codec`), in the order they were deleted. Delete Permanently (`take` without putting the reminder
//! back) and Delete All (`purge_all`) empty it for good.

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

/// The kept reminders, oldest deletion first.
pub fn kept(path: &Path) -> io::Result<Vec<Reminder>> {
    let context: ManagedContext<Reminder> = persistent_store::load(path)?;
    Ok(context.saved_state().into_iter().map(|(_, reminder)| reminder).collect())
}

/// Takes the first kept reminder with reminder id `id` out of the store and returns it, for restoring it.
pub fn take(path: &Path, id: u64) -> io::Result<Option<Reminder>> {
    let mut context: ManagedContext<Reminder> = persistent_store::load(path)?;
    let Some((object, reminder)) = context.saved_state().into_iter().find(|(_, r)| r.id == id) else {
        return Ok(None);
    };
    if context.delete(object).is_err() {
        return Ok(None);
    }
    context.save();
    persistent_store::write(path, &context)?;
    Ok(Some(reminder))
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
        let back = take(&path, 5).unwrap().expect("kept");
        assert_eq!(back.title, "Restore me");
        assert_eq!(kept(&path).unwrap().iter().map(|r| r.id).collect::<Vec<_>>(), vec![3]);
        assert_eq!(take(&path, 5).unwrap(), None, "already taken");
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
