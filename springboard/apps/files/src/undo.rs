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
    /// A link made by Make Alias (removed again only while it is still a link).
    Alias(PathBuf),
    /// Permissions changed in Get Info: (path, the mode bits it had before).
    Mode(Vec<(PathBuf, u32)>),
    /// Colour tags changed: (path, the tags it had before). Restored in the tag index by `Files::undo_file_op`.
    Tags(Vec<(PathBuf, Vec<Tag>)>),
    /// Copies made by a paste, a copying drop or Duplicate (`duplicate`): where each one is. Undo moves them to the
    /// Trash instead of deleting them, so a copy that replaced an item of the same name can still be put back.
    Copy { made: Vec<PathBuf>, duplicate: bool },
}

impl Undo {
    /// The action name for the Edit menu ("Undo Move").
    pub(crate) fn name(&self) -> &'static str {
        match self {
            Undo::Move(pairs) if pairs.iter().all(|(a, b)| a.parent() == b.parent()) => "Rename",
            Undo::Move(_) => "Move",
            Undo::Trash(_) => "Trash",
            Undo::NewFolder(_) => "New Folder",
            Undo::Alias(_) => "Make Alias",
            Undo::Mode(_) => "Permissions",
            Undo::Tags(_) => "Tags",
            Undo::Copy { duplicate: true, .. } => "Duplicate",
            Undo::Copy { .. } => "Copy",
        }
    }
    /// The items the step acted on, as they are named now (a trashed item by the name it had): what the Edit menu
    /// names after the action.
    fn items(&self) -> Vec<&Path> {
        match self {
            Undo::Move(pairs) => pairs.iter().map(|(_, now)| now.as_path()).collect(),
            Undo::Trash(pairs) => pairs.iter().map(|(from, _)| from.as_path()).collect(),
            Undo::NewFolder(path) | Undo::Alias(path) => vec![path.as_path()],
            Undo::Mode(items) => items.iter().map(|(p, _)| p.as_path()).collect(),
            Undo::Tags(items) => items.iter().map(|(p, _)| p.as_path()).collect(),
            Undo::Copy { made, .. } => made.iter().map(PathBuf::as_path).collect(),
        }
    }
    /// The Edit menu's Undo title: the action and what it acted on, as the reference file manager writes it
    /// ("Undo Copy of “d4 2.txt”", "Undo Move of 3 Items").
    pub(crate) fn title(&self) -> String {
        undo_title(self.name(), &self.items())
    }
}

/// "Undo `action` of “name”" for one item, "Undo `action` of N Items" for several, and "Undo `action`" when the step
/// names none. A long name is cut in the middle so the menu stays a menu.
pub(crate) fn undo_title(action: &str, items: &[&Path]) -> String {
    const NAME_MAX: usize = 32;
    match items {
        [] => format!("Undo {action}"),
        [one] => {
            let name = one.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            if name.is_empty() {
                return format!("Undo {action}");
            }
            let chars: Vec<char> = name.chars().collect();
            let name = if chars.len() > NAME_MAX {
                let (head, tail) = (NAME_MAX / 2, NAME_MAX / 2 - 1);
                format!("{}\u{2026}{}", chars[..head].iter().collect::<String>(), chars[chars.len() - tail..].iter().collect::<String>())
            } else {
                name
            };
            format!("Undo {action} of \u{201C}{name}\u{201D}")
        }
        many => format!("Undo {action} of {} Items", many.len()),
    }
}

/// Sets each path back to the tags it had (`Undo::Tags`): the colours it has now are cleared first.
pub(crate) fn restore_tags(index: &mut TagIndex, items: &[(PathBuf, Vec<Tag>)]) {
    for (path, tags) in items {
        index.clear(path);
        for tag in tags {
            index.toggle(path, *tag);
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
        Undo::Mode(items) => {
            let mut failed = None;
            for (path, mode) in items {
                if let Err(e) = perms::set_mode(path, *mode) {
                    failed = Some(batch::friendly(&e));
                }
            }
            failed.map_or(Ok("Restored the permissions".into()), Err)
        }
        // the tag index belongs to `Files`, which restores it (`restore_tags`) before this is reached
        Undo::Tags(_) => Ok("Restored the tags".into()),
        Undo::Copy { made, .. } => {
            let mut failed = None;
            for copy in made {
                // a copy that is gone already (moved, renamed or deleted since) is left alone
                if copy.symlink_metadata().is_err() {
                    failed = Some("The copy is no longer there".to_string());
                    continue;
                }
                match trash::discard(copy, bin, trash::now_secs()) {
                    Ok(stored) => {
                        if let (Some(name), Some(parent)) = (stored.file_name(), copy.parent()) {
                            record_origin(bin, &name.to_string_lossy(), parent);
                        }
                    }
                    Err(e) => failed = Some(batch::friendly(&e)),
                }
            }
            failed.map_or(Ok("Moved the copies to the Trash".into()), Err)
        }
        Undo::NewFolder(path) => std::fs::remove_dir(path).map(|_| "Removed the new folder".into()).map_err(|_| "The new folder is no longer empty".into()),
        Undo::Alias(link) => match link.symlink_metadata() {
            Ok(md) if md.file_type().is_symlink() => std::fs::remove_file(link).map(|_| "Removed the alias".to_string()).map_err(|e| batch::friendly(&e)),
            Ok(_) => Err("The item is no longer an alias".into()),
            Err(_) => Err("The alias is no longer there".into()),
        },
    }
}

impl Files {
    pub(crate) fn push_undo(&mut self, step: Undo) {
        self.undo.push(step);
        if self.undo.len() > UNDO_DEPTH {
            self.undo.remove(0);
        }
    }
    /// The Edit menu titles: the newest step's action and item ("Undo Trash of “a.txt”"); Redo is not offered for
    /// file operations.
    pub(crate) fn file_undo_titles(&self) -> (String, String) {
        (self.undo.last().map(Undo::title).unwrap_or_default(), String::new())
    }
    /// Edit > Undo for file operations.
    pub(crate) fn undo_file_op(&mut self) {
        let Some(step) = self.undo.pop() else { return };
        if let Undo::Tags(items) = &step {
            // only the sidecar changes: the listing stays as it is, with its scroll position and selection
            self.with_tags(|index| restore_tags(index, items));
            self.notice = None;
            return;
        }
        let bin = self.bin();
        self.notice = undo_step(&step, &bin).err();
        if let Undo::Mode(items) = &step {
            for (path, _) in items {
                self.refresh_info_perms(path);
            }
        }
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
        assert_eq!(Undo::Tags(vec![]).name(), "Tags");
    }

    #[test]
    fn the_undo_title_names_the_action_and_its_items() {
        let copy = Undo::Copy { made: vec![PathBuf::from("/d/d4 2.txt")], duplicate: false };
        assert_eq!(copy.title(), "Undo Copy of \u{201C}d4 2.txt\u{201D}");
        let duplicates = Undo::Copy { made: vec![PathBuf::from("/d/a 2"), PathBuf::from("/d/b 2")], duplicate: true };
        assert_eq!(duplicates.title(), "Undo Duplicate of 2 Items");
        // a move names the item where it is now, a rename the new name, a trash the name it had
        assert_eq!(Undo::Move(vec![(PathBuf::from("/a/f"), PathBuf::from("/b/f"))]).title(), "Undo Move of \u{201C}f\u{201D}");
        assert_eq!(Undo::Move(vec![(PathBuf::from("/d/old"), PathBuf::from("/d/new"))]).title(), "Undo Rename of \u{201C}new\u{201D}");
        assert_eq!(Undo::Trash(vec![(PathBuf::from("/d/gone.txt"), PathBuf::from("/bin/gone.txt.1"))]).title(), "Undo Trash of \u{201C}gone.txt\u{201D}");
        assert_eq!(Undo::NewFolder(PathBuf::from("/d/New Folder")).title(), "Undo New Folder of \u{201C}New Folder\u{201D}");
        assert_eq!(Undo::Trash(vec![]).title(), "Undo Trash", "a step that names nothing");
        let long = Undo::Copy { made: vec![PathBuf::from(format!("/d/{}", "x".repeat(80)))], duplicate: false };
        assert!(long.title().chars().count() < 60 && long.title().contains('\u{2026}'), "{}", long.title());
    }

    #[test]
    fn undo_copy_moves_the_copies_to_the_trash_and_leaves_the_originals() {
        let d = scratch("copy");
        std::fs::write(d.join("d4.txt"), "original").unwrap();
        std::fs::write(d.join("d4 2.txt"), "copy").unwrap();
        let bin = trash::dir(&d);
        let step = Undo::Copy { made: vec![d.join("d4 2.txt")], duplicate: false };
        undo_step(&step, &bin).unwrap();
        assert!(!d.join("d4 2.txt").exists(), "the copy left the folder");
        assert_eq!(std::fs::read_to_string(d.join("d4.txt")).unwrap(), "original");
        assert!(undo_step(&step, &bin).is_err(), "a second undo finds nothing");
    }

    #[test]
    fn a_window_offers_undo_after_a_paste() {
        let mut files = crate::tests::fixture(vec![]);
        assert_eq!(files.file_undo_titles().0, "");
        assert!(!files.edit_commands().supports(Command::Undo));
        // what `finish_job` records for a paste that copied one file
        let step = copy_undo(JobKind::Transfer(Op::Copy), &[PathBuf::from("/fixture/d4 2.txt")]).expect("a copy is undoable");
        files.push_undo(step);
        assert!(files.edit_commands().supports(Command::Undo));
        assert_eq!(files.edit_undo_titles(), ("Undo Copy of \u{201C}d4 2.txt\u{201D}".to_string(), String::new()));
        // Duplicate and files dropped by another app too; a move, a trash and a copy of nothing leave it to their own steps
        assert!(matches!(copy_undo(JobKind::Duplicate, &[PathBuf::from("/d/a 2")]), Some(Undo::Copy { duplicate: true, .. })));
        assert!(matches!(copy_undo(JobKind::Receive, &[PathBuf::from("/d/a")]), Some(Undo::Copy { duplicate: false, .. })));
        assert!(copy_undo(JobKind::Transfer(Op::Move), &[PathBuf::from("/d/a")]).is_none());
        assert!(copy_undo(JobKind::Trash, &[PathBuf::from("/d/a")]).is_none());
        assert!(copy_undo(JobKind::Transfer(Op::Copy), &[]).is_none());
    }

    #[test]
    fn restore_tags_puts_back_exactly_the_old_colours() {
        let (a, b) = (PathBuf::from("/d/a"), PathBuf::from("/d/b"));
        let mut index = TagIndex::default();
        index.toggle(&a, Tag::Red);
        index.toggle(&a, Tag::Blue);
        let before = vec![(a.clone(), index.tags(&a)), (b.clone(), index.tags(&b))];
        // a gains a colour and loses one; b, untagged, gains one
        index.toggle(&a, Tag::Red);
        index.toggle(&a, Tag::Green);
        index.toggle(&b, Tag::Purple);
        restore_tags(&mut index, &before);
        assert_eq!(index.tags(&a), before[0].1);
        assert!(index.tags(&b).is_empty(), "an item that had no tags has none again");
        assert_eq!(index.len(), 1);
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
    fn undo_alias_removes_only_the_link() {
        let d = scratch("alias");
        std::fs::write(d.join("file"), "kept").unwrap();
        let link = fs::make_alias(&d.join("file")).unwrap();
        assert_eq!(Undo::Alias(link.clone()).name(), "Make Alias");
        undo_step(&Undo::Alias(link.clone()), &d).unwrap();
        assert!(link.symlink_metadata().is_err(), "the link is removed");
        assert_eq!(std::fs::read_to_string(d.join("file")).unwrap(), "kept", "the original is untouched");
        assert!(undo_step(&Undo::Alias(link), &d).is_err(), "a second undo finds nothing");
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
