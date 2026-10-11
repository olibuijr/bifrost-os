//! Undo for file operations (Edit > Undo, Cmd+Z): move, rename, trash and new folder, as Files does.
use crate::*;

/// One undoable step.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Undo {
    /// Items moved or renamed: (original path, where it is now).
    Move(Vec<(PathBuf, PathBuf)>),
    /// Items sent to Recently Deleted: (original path, stored path).
    Trash(Vec<(PathBuf, PathBuf)>),
    /// A folder created (removed again only while it is empty).
    NewFolder(PathBuf),
}

impl Undo {
    /// The action name for the Edit menu ("Undo Move").
    pub(crate) fn name(&self) -> &'static str {
        match self {
            Undo::Move(pairs) if pairs.iter().all(|(a, b)| a.parent() == b.parent()) => "Rename",
            Undo::Move(_) => "Move",
            Undo::Trash(_) => "Trash",
            Undo::NewFolder(_) => "New Folder",
        }
    }
}

/// Steps kept; older ones fall off.
pub(crate) const UNDO_DEPTH: usize = 20;

/// Reverses `step`, returning a short description or why it could not be undone.
/// Nothing is overwritten: a name that is taken again gets a numbered name.
pub(crate) fn undo_step(step: &Undo, bin: &Path) -> Result<String, String> {
    match step {
        Undo::Move(pairs) => {
            let mut failed = None;
            for (from, now) in pairs {
                let Some(dir) = from.parent().filter(|d| d.is_dir()) else {
                    failed = Some("The original folder is gone".to_string());
                    continue;
                };
                if now.symlink_metadata().is_err() {
                    failed = Some("The item is no longer there".to_string());
                    continue;
                }
                let name = from.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                let back = fs::unique_name(dir, &name);
                let moved = std::fs::rename(now, &back).or_else(|_| fs::move_to(now, dir).and_then(|m| if m == back { Ok(()) } else { std::fs::rename(&m, &back) }));
                if let Err(e) = moved {
                    failed = Some(batch::friendly(&e));
                }
            }
            failed.map_or(Ok("Undid the move".into()), Err)
        }
        Undo::Trash(pairs) => {
            let mut failed = None;
            for (orig, stored) in pairs {
                let Some(dir) = orig.parent().filter(|d| d.is_dir()) else {
                    failed = Some("The original folder is gone".to_string());
                    continue;
                };
                match trash::restore(stored, dir) {
                    Ok(_) => {
                        if let Some(name) = stored.file_name() {
                            let _ = take_origin(bin, &name.to_string_lossy());
                        }
                    }
                    Err(e) => failed = Some(batch::friendly(&e)),
                }
            }
            failed.map_or(Ok("Put the items back".into()), Err)
        }
        Undo::NewFolder(path) => std::fs::remove_dir(path).map(|_| "Removed the new folder".into()).map_err(|_| "The new folder is no longer empty".into()),
    }
}

impl Files {
    pub(crate) fn push_undo(&mut self, step: Undo) {
        self.undo.push(step);
        if self.undo.len() > UNDO_DEPTH {
            self.undo.remove(0);
        }
    }
    /// The Edit menu titles: the newest step's name ("Undo Trash"); Redo is not offered for file operations.
    pub(crate) fn file_undo_titles(&self) -> (String, String) {
        (self.undo.last().map(|u| format!("Undo {}", u.name())).unwrap_or_default(), String::new())
    }
    /// Edit > Undo for file operations.
    pub(crate) fn undo_file_op(&mut self) {
        let Some(step) = self.undo.pop() else { return };
        let bin = self.bin();
        self.notice = undo_step(&step, &bin).err();
        let dir = self.path.clone();
        self.navigate(dir, false);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("files-undo-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn undo_steps_are_named_for_the_edit_menu() {
        let rename = Undo::Move(vec![(PathBuf::from("/d/old"), PathBuf::from("/d/new"))]);
        let moved = Undo::Move(vec![(PathBuf::from("/a/f"), PathBuf::from("/b/f"))]);
        assert_eq!((rename.name(), moved.name()), ("Rename", "Move"));
        assert_eq!(Undo::Trash(vec![]).name(), "Trash");
        assert_eq!(Undo::NewFolder(PathBuf::from("/d/n")).name(), "New Folder");
    }

    #[test]
    fn undo_move_returns_the_item_and_never_overwrites() {
        let d = scratch("move");
        let (a, b) = (d.join("a"), d.join("b"));
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        std::fs::write(b.join("f"), "moved").unwrap();
        std::fs::write(a.join("f"), "new occupant").unwrap();
        undo_step(&Undo::Move(vec![(a.join("f"), b.join("f"))]), &d).unwrap();
        assert_eq!(std::fs::read_to_string(a.join("f")).unwrap(), "new occupant");
        assert_eq!(std::fs::read_to_string(a.join("f 2")).unwrap(), "moved");
        assert!(!b.join("f").exists());
        assert!(undo_step(&Undo::Move(vec![(a.join("f"), b.join("f"))]), &d).is_err(), "already gone");
    }

    #[test]
    fn undo_rename_trash_and_new_folder() {
        let d = scratch("rest");
        std::fs::write(d.join("new name"), "x").unwrap();
        undo_step(&Undo::Move(vec![(d.join("old"), d.join("new name"))]), &d).unwrap();
        assert!(d.join("old").exists());
        let bin = trash::dir(&d);
        let stored = trash::discard(&d.join("old"), &bin, 1000).unwrap();
        undo_step(&Undo::Trash(vec![(d.join("old"), stored)]), &bin).unwrap();
        assert!(d.join("old").exists());
        std::fs::create_dir(d.join("New Folder")).unwrap();
        assert!(undo_step(&Undo::NewFolder(d.join("New Folder")), &bin).is_ok());
        std::fs::create_dir(d.join("New Folder")).unwrap();
        std::fs::write(d.join("New Folder/keep"), "").unwrap();
        assert!(undo_step(&Undo::NewFolder(d.join("New Folder")), &bin).is_err());
        assert!(d.join("New Folder/keep").exists());
    }

    #[test]
    fn trash_undo_with_missing_folder_reports() {
        let d = scratch("gone");
        assert!(undo_step(&Undo::Trash(vec![(d.join("nope/x"), d.join("y"))]), &d).is_err());
    }
}
