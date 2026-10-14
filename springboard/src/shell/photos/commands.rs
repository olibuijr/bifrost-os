//! The Photos menu commands. Each one is a row of the File, Edit, Image and View menus of the desktop bar
//! (`desktop_bar.rs`), and the commands with a key equivalent are chords too (`hwkeys.rs`). A command acts on the
//! photo open in the viewer or, on the grid, on the selection. It is greyed out when there is nothing for it to act
//! on, and the writes it makes are one step for Undo.

use super::*;

/// What a menu row or a key equivalent asks Photos to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PhotoCommand {
    Undo,
    Redo,
    Copy,
    SelectAll,
    DeselectAll,
    Find,
    /// the search card, for a date typed as the filter
    FilterDate,
    /// moves the open photo (or the selection) to Recently Deleted, or deletes it for good in Recently Deleted
    Delete,
    NewAlbum,
    RenameAlbum,
    DeleteAlbum,
    /// the folder prompt of the album picker's More page
    Import,
    Export,
    Print,
    PlaySlideshow,
    ZoomIn,
    ZoomOut,
    NewestFirst,
    OldestFirst,
    AlbumsByName,
    ShowAll,
    FavouritesOnly,
    EditedOnly,
    ScreenshotsOnly,
    ShowHidden,
    AspectGrid,
    Favourite,
    Hide,
    /// 0 clears the rating
    Rate(u8),
    Info,
    RotateLeft,
    RotateRight,
    FlipHorizontal,
    FlipVertical,
    AutoEnhance,
    RevertToOriginal,
    Duplicate,
    CopyEdits,
    PasteEdits,
    MergeDuplicates,
}

/// One key equivalent: the evdev key code, whether Shift and Option are held, the command it runs and its text for
/// the menus.
struct Chord {
    cmd: PhotoCommand,
    code: u16,
    shift: bool,
    alt: bool,
    text: &'static str,
}

const fn chord(cmd: PhotoCommand, code: u16, shift: bool, alt: bool, text: &'static str) -> Chord {
    Chord { cmd, code, shift, alt, text }
}

/// The key equivalents, all with Cmd held. Cmd+Z and Cmd+Y are the shell's Undo and Redo keys, and Cmd+V
/// is left to the text card.
const CHORDS: &[Chord] = &[
    chord(PhotoCommand::SelectAll, 30, false, false, "cmd+a"),
    chord(PhotoCommand::DeselectAll, 30, true, false, "cmd+shift+a"),
    chord(PhotoCommand::Copy, 46, false, false, "cmd+c"),
    chord(PhotoCommand::CopyEdits, 46, true, false, "cmd+shift+c"),
    chord(PhotoCommand::PasteEdits, 47, true, false, "cmd+shift+v"),
    chord(PhotoCommand::Find, 33, false, false, "cmd+f"),
    chord(PhotoCommand::Delete, 14, false, false, "cmd+backspace"),
    chord(PhotoCommand::NewAlbum, 49, false, false, "cmd+n"),
    chord(PhotoCommand::Import, 23, true, false, "cmd+shift+i"),
    chord(PhotoCommand::Export, 18, true, false, "cmd+shift+e"),
    chord(PhotoCommand::Print, 25, false, false, "cmd+p"),
    chord(PhotoCommand::ZoomIn, 13, false, false, "cmd+="),
    chord(PhotoCommand::ZoomIn, 13, true, false, "cmd++"),
    chord(PhotoCommand::ZoomOut, 12, false, false, "cmd+-"),
    chord(PhotoCommand::Favourite, 38, false, false, "cmd+l"),
    chord(PhotoCommand::Hide, 38, true, false, "cmd+shift+l"),
    chord(PhotoCommand::Info, 23, false, false, "cmd+i"),
    chord(PhotoCommand::Duplicate, 32, false, false, "cmd+d"),
    chord(PhotoCommand::RotateLeft, 19, false, false, "cmd+r"),
    chord(PhotoCommand::RotateRight, 19, false, true, "cmd+alt+r"),
    chord(PhotoCommand::Rate(0), 11, false, false, "cmd+0"),
    chord(PhotoCommand::Rate(1), 2, false, false, "cmd+1"),
    chord(PhotoCommand::Rate(2), 3, false, false, "cmd+2"),
    chord(PhotoCommand::Rate(3), 4, false, false, "cmd+3"),
    chord(PhotoCommand::Rate(4), 5, false, false, "cmd+4"),
    chord(PhotoCommand::Rate(5), 6, false, false, "cmd+5"),
];

impl PhotoCommand {
    /// The command of a Cmd chord: the evdev key code, and whether Shift and Option are held.
    pub fn from_chord(code: u16, shift: bool, alt: bool) -> Option<PhotoCommand> {
        CHORDS.iter().find(|c| c.code == code && c.shift == shift && c.alt == alt).map(|c| c.cmd)
    }

    /// The first key equivalent of the command, as the menus name it ("cmd+shift+a").
    pub fn key_text(self) -> Option<&'static str> {
        CHORDS.iter().find(|c| c.cmd == self).map(|c| c.text)
    }

    /// Whether the menus put a check mark on the command: a setting that is on or off, or the choice in force.
    pub fn checkable(self) -> bool {
        use PhotoCommand::*;
        matches!(self, NewestFirst | OldestFirst | AlbumsByName | ShowAll | FavouritesOnly | EditedOnly | ScreenshotsOnly | ShowHidden | AspectGrid | AutoEnhance | Rate(_))
    }

    /// The edit tool a command turns on or off, if it is one.
    fn tool(self) -> Option<Tool> {
        match self {
            PhotoCommand::FlipHorizontal => Some(Tool::FlipHorizontal),
            PhotoCommand::FlipVertical => Some(Tool::FlipVertical),
            PhotoCommand::AutoEnhance => Some(Tool::AutoEnhance),
            _ => None,
        }
    }
}

/// The on or off value a boolean setting is stored as.
fn flag(on: bool) -> String {
    if on {
        "1".to_string()
    } else {
        "0".to_string()
    }
}

/// The write that keeps `edit` as the saved edit of `p` (an unedited photo stores an empty value).
fn edit_write(p: &Photo, edit: &Edit) -> (String, String) {
    let value = if edit.is_identity() { String::new() } else { recipe::encode(edit) };
    (format!("edit:{}", p.path.display()), value)
}

impl Photos {
    /// The photos a command acts on: the photo open in the viewer, else the grid's selection.
    fn command_targets(&self) -> Vec<Photo> {
        if self.editor.is_some() {
            return Vec::new();
        }
        match self.viewing() {
            Some(p) => vec![p.clone()],
            None if self.selection.active => self.selected_photos(),
            None => Vec::new(),
        }
    }

    /// Whether Photos shows its All Photos grid with the hidden photos listed (View > Show Hidden Photo Album).
    pub(super) fn shows_hidden(&self) -> bool {
        self.prefs.get("show_hidden") == Some("1")
    }

    /// Whether the command can run now.
    pub fn command_enabled(&self, cmd: PhotoCommand) -> bool {
        use PhotoCommand::*;
        if self.editor.is_some() || self.slideshow.is_some() || self.entry.is_some() {
            return false;
        }
        let viewing = self.viewer.is_some();
        let grid = !viewing;
        let has = !self.command_targets().is_empty();
        let editable = has && self.album != Album::Trash;
        match cmd {
            Undo => self.stack.undo_label().is_some(),
            Redo => self.stack.redo_label().is_some(),
            Copy | Info | Print | CopyEdits => viewing,
            Duplicate | Export => viewing && self.album != Album::Trash,
            SelectAll => grid && !self.view.is_empty(),
            DeselectAll => self.selection.active && !self.selection.is_empty(),
            Find | FilterDate | Import | ShowHidden | AspectGrid | AlbumsByName => true,
            NewestFirst | OldestFirst | ShowAll | FavouritesOnly | EditedOnly | ScreenshotsOnly => grid,
            ZoomIn => grid && self.cols > ZOOMS[ZOOMS.len() - 1],
            ZoomOut => grid && self.cols < ZOOMS[0],
            PlaySlideshow => !self.view.is_empty() && self.album != Album::Trash,
            Delete => has,
            NewAlbum => self.user_albums.len() < MAX_USER_ALBUMS,
            RenameAlbum | DeleteAlbum => matches!(self.album, Album::User(_)),
            MergeDuplicates => grid && self.album == Album::Duplicates,
            Favourite | Hide | Rate(_) | RotateLeft | RotateRight | FlipHorizontal | FlipVertical | AutoEnhance | RevertToOriginal => editable,
            PasteEdits => editable && self.edit_clip.is_some(),
        }
    }

    /// The check mark of a checkable command: whether its setting is on, or its choice is the one in force.
    pub fn command_checked(&self, cmd: PhotoCommand) -> bool {
        use PhotoCommand::*;
        let search = self.search.as_str();
        match cmd {
            NewestFirst => self.newest_first,
            OldestFirst => !self.newest_first,
            AlbumsByName => self.albums_by_name(),
            ShowAll => search.is_empty(),
            FavouritesOnly => search == "is:favorite",
            EditedOnly => search == "is:edited",
            ScreenshotsOnly => search == "is:screenshot",
            ShowHidden => self.shows_hidden(),
            AspectGrid => self.fit_grid(),
            AutoEnhance => self.command_targets().first().is_some_and(|p| self.recipe(p).is_some_and(|e| e.auto)),
            Rate(n) => self.command_targets().first().is_some_and(|p| self.rating(p) == n),
            _ => false,
        }
    }

    /// The title of a command in the menus. Undo and Redo name the change they take back, and the toggles name what
    /// they do next.
    pub fn command_title(&self, cmd: PhotoCommand) -> String {
        use PhotoCommand::*;
        let first = self.command_targets().into_iter().next();
        let favourite = first.as_ref().is_some_and(|p| self.is_favourite(p));
        let hidden = first.as_ref().is_some_and(|p| self.is_hidden(p));
        match cmd {
            Undo => match self.stack.undo_label() {
                Some(label) if !label.is_empty() => format!("Undo {label}"),
                _ => "Undo".to_string(),
            },
            Redo => match self.stack.redo_label() {
                Some(label) if !label.is_empty() => format!("Redo {label}"),
                _ => "Redo".to_string(),
            },
            Copy => "Copy".into(),
            SelectAll => "Select All".into(),
            DeselectAll => "Deselect All".into(),
            Find => "Find\u{2026}".into(),
            FilterDate => "Filter by Date\u{2026}".into(),
            Delete if self.album == Album::Trash => "Delete Permanently".into(),
            Delete => "Delete Photo".into(),
            NewAlbum => "New Album".into(),
            RenameAlbum => "Rename Album\u{2026}".into(),
            DeleteAlbum => "Delete Album".into(),
            Import => "Import from Folder\u{2026}".into(),
            Export => "Export Photo\u{2026}".into(),
            Print => "Print\u{2026}".into(),
            PlaySlideshow => "Play Slideshow".into(),
            ZoomIn => "Zoom In".into(),
            ZoomOut => "Zoom Out".into(),
            NewestFirst => "Newest First".into(),
            OldestFirst => "Oldest First".into(),
            AlbumsByName => "Sort Albums by Name".into(),
            ShowAll => "All Photos".into(),
            FavouritesOnly => "Favorites".into(),
            EditedOnly => "Edited".into(),
            ScreenshotsOnly => "Screenshots".into(),
            ShowHidden => "Show Hidden Photo Album".into(),
            AspectGrid => "Aspect Ratio Grid".into(),
            Favourite if favourite => "Unfavorite".into(),
            Favourite => "Favorite".into(),
            Hide if hidden => "Unhide Photo".into(),
            Hide => "Hide Photo".into(),
            Rate(0) => "Clear Rating".into(),
            Rate(1) => "1 Star".into(),
            Rate(n) => format!("{n} Stars"),
            Info => "Info".into(),
            RotateLeft => "Rotate Left".into(),
            RotateRight => "Rotate Right".into(),
            FlipHorizontal => "Flip Horizontal".into(),
            FlipVertical => "Flip Vertical".into(),
            AutoEnhance => "Auto Enhance".into(),
            RevertToOriginal => "Revert to Original".into(),
            Duplicate => "Duplicate".into(),
            CopyEdits => "Copy Edits".into(),
            PasteEdits => "Paste Edits".into(),
            MergeDuplicates => "Merge Duplicates".into(),
        }
    }

    /// Runs a command. A command that is not enabled does nothing.
    pub fn command(&mut self, ui: &Ui, cmd: PhotoCommand, t: f64) {
        if !self.command_enabled(cmd) {
            return;
        }
        use PhotoCommand::*;
        self.now = t;
        self.dirty = true;
        self.album_menu = false;
        self.more_menu = false;
        let targets = self.command_targets();
        match cmd {
            Undo => self.undo(ui),
            Redo => self.redo(ui),
            Copy => self.copy_current(),
            SelectAll => {
                if !self.selection.active {
                    self.selection.begin();
                }
                let all = self.view.clone();
                self.selection.select_all(&all);
                self.confirm_purge = false;
            }
            DeselectAll => {
                self.selection.deselect_all();
                self.confirm_purge = false;
            }
            Find => {
                let text = self.search.clone();
                self.open_entry(ui, Purpose::Search, &text, None, t);
            }
            FilterDate => self.open_entry(ui, Purpose::Search, "", None, t),
            Delete if self.viewer.is_some() => self.sheet = Some(Sheet::Delete),
            Delete => self.activate(ui, Target::SelDelete, t),
            NewAlbum => self.activate(ui, Target::NewAlbum, t),
            RenameAlbum => self.activate(ui, Target::RenameAlbum, t),
            DeleteAlbum => self.activate(ui, Target::DeleteAlbum, t),
            Import => self.activate(ui, Target::ImportOpen, t),
            Export => self.sheet = Some(Sheet::Export),
            Print => self.print_req = self.print_image(),
            PlaySlideshow => {
                let at = self.viewer.as_ref().map_or(0, |v| v.index);
                self.start_slideshow(self.photos().to_vec(), at, t);
            }
            ZoomIn => self.activate(ui, Target::ZoomIn, t),
            ZoomOut => self.activate(ui, Target::ZoomOut, t),
            NewestFirst | OldestFirst => {
                if self.newest_first != (cmd == NewestFirst) {
                    self.activate(ui, Target::SortOrder, t);
                }
            }
            AlbumsByName => self.activate(ui, Target::AlbumSort, t),
            ShowAll => self.set_search(ui, ""),
            FavouritesOnly => self.set_search(ui, "is:favorite"),
            EditedOnly => self.set_search(ui, "is:edited"),
            ScreenshotsOnly => self.set_search(ui, "is:screenshot"),
            ShowHidden => {
                let on = !self.shows_hidden();
                let _ = self.pref_set("show_hidden", if on { "1" } else { "0" });
                self.rebuild(ui);
            }
            AspectGrid => {
                let on = !self.fit_grid();
                let _ = self.pref_set("grid", if on { "fit" } else { "square" });
                // the thumbnails of the other shape are not kept: the grid decodes its own
                self.cells = MemCache::new(thumbs::MEM_BUDGET);
                self.cell_px = 0;
                self.rebuild(ui);
            }
            Favourite => {
                let on = targets.iter().any(|p| !self.is_favourite(p));
                let writes: Vec<(String, String)> = targets.iter().map(|p| (format!("fav:{}", p.path.display()), flag(on))).collect();
                self.command_writes(ui, "Favorite", writes, &[]);
            }
            Hide => {
                let on = targets.iter().any(|p| !self.is_hidden(p));
                let writes: Vec<(String, String)> = targets.iter().map(|p| (format!("hide:{}", p.path.display()), if on { "1".to_string() } else { String::new() })).collect();
                self.command_writes(ui, if on { "Hide Photo" } else { "Unhide Photo" }, writes, &[]);
            }
            Rate(n) => {
                let writes: Vec<(String, String)> = targets.iter().map(|p| (format!("rate:{}", p.path.display()), if n == 0 { String::new() } else { n.to_string() })).collect();
                self.command_writes(ui, "Rating", writes, &[]);
            }
            Info => self.activate(ui, Target::Info, t),
            RotateLeft | RotateRight => {
                let right = cmd == RotateRight;
                let writes: Vec<(String, String)> = targets
                    .iter()
                    .map(|p| {
                        let mut edit = self.recipe(p).unwrap_or_default();
                        edit.rotate(right);
                        edit_write(p, &edit)
                    })
                    .collect();
                self.command_writes(ui, "Rotate", writes, &targets);
            }
            FlipHorizontal | FlipVertical | AutoEnhance => {
                let Some(tool) = cmd.tool() else { return };
                // every photo takes the state the first one does not have, so a mixed selection ends up the same
                let on = !targets.iter().all(|p| tool.is_on(&self.recipe(p).unwrap_or_default()));
                let writes: Vec<(String, String)> = targets
                    .iter()
                    .map(|p| {
                        let mut edit = self.recipe(p).unwrap_or_default();
                        if tool.is_on(&edit) != on {
                            tool.apply(&mut edit);
                        }
                        edit_write(p, &edit)
                    })
                    .collect();
                self.command_writes(ui, "Edit", writes, &targets);
            }
            RevertToOriginal => {
                let writes: Vec<(String, String)> = targets.iter().map(|p| (format!("edit:{}", p.path.display()), String::new())).collect();
                self.info_revert = false;
                self.command_writes(ui, "Revert to Original", writes, &targets);
            }
            Duplicate => self.duplicate_current(),
            CopyEdits => {
                let clip = self.viewing().cloned().map(|p| self.recipe(&p).unwrap_or_default());
                if clip.is_some() {
                    self.edit_clip = clip;
                }
            }
            PasteEdits => self.paste_edits_to(ui, targets),
            MergeDuplicates => self.merge_duplicates(ui),
        }
        self.dirty = true;
    }

    /// Writes `writes` as one step for Undo, then rebuilds the lists and keeps the viewer on its photo. The photos
    /// given are edited ones: their cached pictures go, so they are decoded again with the new edit.
    fn command_writes(&mut self, ui: &Ui, label: &str, writes: Vec<(String, String)>, edited: &[Photo]) {
        let anchor = self.viewer_anchor();
        if !self.write_many(label, writes) {
            return;
        }
        for p in edited {
            self.invalidate(p);
        }
        self.rebuild(ui);
        self.restore_viewer(anchor);
    }

    /// The open photo with its edit applied, for Print. None when the file cannot be read.
    fn print_image(&self) -> Option<Image> {
        let p = self.viewing()?;
        let bytes = std::fs::read(&p.path).ok()?;
        decode_edited(&bytes, self.recipe(p))
    }

    /// The picture a Print command asked to print, once. The shell opens the print sheet over it.
    pub fn take_print(&mut self) -> Option<Image> {
        self.print_req.take()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_key_chord_names_one_command() {
        for (i, a) in CHORDS.iter().enumerate() {
            for b in &CHORDS[i + 1..] {
                if a.code == b.code && a.shift == b.shift && a.alt == b.alt {
                    assert_eq!(a.cmd, b.cmd, "{} and {} are the same key", a.text, b.text);
                }
            }
        }
    }

    #[test]
    fn chords_read_back_from_their_keys() {
        assert_eq!(PhotoCommand::from_chord(30, false, false), Some(PhotoCommand::SelectAll));
        assert_eq!(PhotoCommand::from_chord(30, true, false), Some(PhotoCommand::DeselectAll));
        assert_eq!(PhotoCommand::from_chord(19, false, true), Some(PhotoCommand::RotateRight));
        assert_eq!(PhotoCommand::from_chord(6, false, false), Some(PhotoCommand::Rate(5)));
        assert_eq!(PhotoCommand::from_chord(13, true, false), Some(PhotoCommand::ZoomIn));
        assert_eq!(PhotoCommand::from_chord(5, false, false), None, "Cmd+4 is bound, Cmd+5 is not the key of 4");
    }

    #[test]
    fn menu_key_texts_name_their_chord_and_parse() {
        assert_eq!(PhotoCommand::DeselectAll.key_text(), Some("cmd+shift+a"));
        assert_eq!(PhotoCommand::Rate(0).key_text(), Some("cmd+0"));
        assert_eq!(PhotoCommand::Hide.key_text(), Some("cmd+shift+l"));
        assert_eq!(PhotoCommand::RevertToOriginal.key_text(), None);
        for c in CHORDS {
            assert!(bifrost_ui::keys::KeyCombo::parse(c.text).is_some(), "{} parses", c.text);
        }
    }

    #[test]
    fn a_fitted_thumbnail_keeps_its_shape_inside_its_cell() {
        let cell = Rect { x: 10, y: 20, w: 100, h: 100 };
        // a 2:1 picture is as wide as the cell and centred vertically
        assert_eq!(fit_rect(cell, 200, 100), Rect { x: 10, y: 45, w: 100, h: 50 });
        // a tall picture fills the height and is centred across the width
        assert_eq!(fit_rect(cell, 50, 100), Rect { x: 35, y: 20, w: 50, h: 100 });
        assert_eq!(fit_rect(cell, 0, 0), cell, "an empty picture keeps the cell");
    }

    #[test]
    fn only_settings_and_choices_get_check_marks() {
        assert!(PhotoCommand::NewestFirst.checkable());
        assert!(PhotoCommand::Rate(3).checkable());
        assert!(!PhotoCommand::Favourite.checkable());
        assert!(!PhotoCommand::RotateLeft.checkable());
    }
}
