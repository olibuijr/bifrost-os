//! Files's Go menu (Go to Folder with path completion, Connect to Server, Home, Desktop, Documents, Downloads, Recents), spring-loaded folders
//! and the Option-to-copy drag state.
use crate::*;

/// How long a drag must hover over a folder before it opens (Files's spring-loaded delay).
/// A second: long enough that a drop aimed at the folder's icon lands before the folder opens under it.
pub(crate) const SPRING_DELAY: Duration = Duration::from_millis(1000);

/// Where a window was before a drag sprang a folder open: the folder shown, its history and the place in it.
pub(crate) type SpringOrigin = (PathBuf, Vec<PathBuf>, usize);

/// State of the Files features that do not belong to one view.
#[derive(Default)]
pub(crate) struct FileManagerState {
    /// The Go to Folder sheet's path field, when open.
    pub(crate) goto: Option<TextField>,
    /// The Empty Trash confirmation sheet is open (Recently Deleted only).
    pub(crate) empty_trash: bool,
    /// The folder a drag hovers over and since when; it opens after `SPRING_DELAY`.
    pub(crate) spring: Option<(PathBuf, Instant)>,
    /// The items being dragged. Kept apart from the selection because a spring-loaded folder changes the selection.
    pub(crate) drag_paths: Vec<PathBuf>,
    /// Option was held during the drag: drop copies instead of moves.
    pub(crate) copy_drag: bool,
    /// When a finger pressed an item and has rested on it since (ms, the touch clock): after `hold_ms` it lifts the item
    /// (`Files::lift_item`). None for a mouse press, and once the finger moved past the drag slop, which is a scroll.
    pub(crate) hold: Option<f64>,
    /// The decoded picture of the Preview panel (JPEG and PNG).
    pub(crate) preview_image: Option<springboard::gfx::Image>,
    /// Go > Connect to Server: the address sheet, the remote folder browser and the recent servers.
    pub(crate) server: crate::server::ServerState,
    /// The item to select (and maybe rename) when the open folder has been read again: a renamed item, or a new folder.
    pub(crate) pending: Option<crate::ops::Pending>,
    /// The folder a drag began in, kept from the first folder it sprang open: a drag that ends without a drop goes
    /// back there (`spring_return`).
    pub(crate) spring_from: Option<SpringOrigin>,
    /// The scroll and the selected items of a folder that is being listed again in place (`Files::relist`): the
    /// listing puts them back, so a relist never moves the view.
    pub(crate) relist: Option<(f32, Vec<PathBuf>)>,
    /// The watched folders changed (`rewatch`): the main window builds the one stream again (`sync_watch`).
    pub(crate) watch_stale: bool,
    /// This `Files` is a New Window or Get Info window: its folder is watched by the main window's stream.
    pub(crate) extra_window: bool,
    /// Counts this window's Copy and Cut: when it moves, the other windows take the copied items (`share_clip`).
    pub(crate) clip_rev: u64,
    /// The `clip_rev` the other windows have taken (`Files::share_clips`).
    pub(crate) clip_shared: u64,
    /// The folders the main window's stream watches now.
    pub(crate) watched: Vec<PathBuf>,
    /// Streams replaced while a batch of theirs was still maturing: polled until it is handed over.
    pub(crate) retiring: Vec<bifrost_app::fswatch::WatchStream>,
    /// The mount points of the volumes at the last look, when that was, and until when the mount table is looked at
    /// again every second (a disk or a media folder just changed, and its mount follows a moment later).
    pub(crate) mounts: Option<Vec<PathBuf>>,
    pub(crate) mounts_checked: Option<Instant>,
    pub(crate) mounts_until: Option<Instant>,
    /// The drag image at device pixels for the drag being asked for: its side and straight RGBA (`dragpreview`).
    pub(crate) drag_image: Option<(u32, Vec<u8>)>,
    /// This window is not the key window (another window of Files has the keys, or another app is in front): its
    /// selection is drawn on the unemphasised plate (`Files::selection_plate`). Set before each draw (`set_key`).
    pub(crate) inactive: bool,
    /// The main window only: the app left the foreground (`App::suspend`), so none of its windows is key.
    pub(crate) suspended: bool,
    /// The main window only: the app went to the background or came back, and the windows have not been drawn since
    /// (`tick` asks for the frame).
    pub(crate) emphasis_stale: bool,
    /// The items a paste, a drop, Duplicate or an undo just put into the open folder: selected and scrolled into
    /// view once the folder has been read again (`Files::apply_added`).
    pub(crate) added: Vec<PathBuf>,
    /// Type-to-select: the letters typed so far, and when the last one came (`Files::type_select`).
    pub(crate) typed: Option<(String, Instant)>,
    /// Go to Folder: why the path typed could not be opened, shown in the sheet until the path is edited.
    pub(crate) goto_error: Option<String>,
    /// The mouse press that opened the Tags panel is still down: its release over a colour row chooses the colour
    /// (`Files::tag_release`).
    pub(crate) tag_press: bool,
}

/// Expands a typed path: `~` is the home folder, a relative path is taken from `base`.
pub(crate) fn expand(home: &Path, base: &Path, typed: &str) -> PathBuf {
    let typed = typed.trim();
    if typed == "~" {
        home.to_path_buf()
    } else if let Some(rest) = typed.strip_prefix("~/") {
        home.join(rest)
    } else if typed.starts_with('/') {
        PathBuf::from(typed)
    } else {
        base.join(typed)
    }
}

/// What Return in the Go to Folder sheet opens.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum GotoTarget {
    /// A folder: it is shown.
    Folder(PathBuf),
    /// Anything else that exists: its folder is shown and the item selected (the folder, then the item).
    Item(PathBuf, PathBuf),
}

/// Where the path typed into the Go to Folder sheet leads: `~`, `~/sub`, an absolute path or one relative to the
/// folder shown (`expand`), with or without a slash at its end. None when nothing is there.
pub(crate) fn goto_target(home: &Path, base: &Path, typed: &str) -> Option<GotoTarget> {
    if typed.trim().is_empty() {
        return None;
    }
    // without the slash at the end, the `.` and the doubled slashes a typed path may have
    let dest: PathBuf = expand(home, base, typed).components().collect();
    if dest.is_dir() {
        return Some(GotoTarget::Folder(dest));
    }
    // a file, or a link that leads nowhere: something with that name is in its folder
    let folder = dest.parent().filter(|p| p.is_dir())?.to_path_buf();
    dest.symlink_metadata().is_ok().then(|| GotoTarget::Item(folder, dest))
}

/// The Edit command a Cmd or Ctrl chord means in a text field: A, C, X, V, Z and Shift+Z (evdev key codes). None
/// for any other chord, and with Option held.
pub(crate) fn field_chord(key: u32, mods: u32) -> Option<Command> {
    if mods & (CHORD_META | CHORD_CTRL) == 0 || mods & CHORD_ALT != 0 {
        return None;
    }
    match (key, mods & CHORD_SHIFT != 0) {
        (30, false) => Some(Command::SelectAll),
        (46, false) => Some(Command::Copy),
        (45, false) => Some(Command::Cut),
        (47, false) => Some(Command::Paste),
        (44, false) => Some(Command::Undo),
        (44, true) => Some(Command::Redo),
        _ => None,
    }
}

/// The sheet's line for a path that leads nowhere, in the sentence style of the other notices.
pub(crate) fn goto_missing_message(typed: &str) -> String {
    format!("The folder \u{201C}{}\u{201D} can\u{2019}t be found.", typed.trim())
}

/// Tab completion for the Go to Folder field: the longest common prefix of the folders that start with the
/// last component (case-insensitive), with a trailing `/` when exactly one matches. `None` when nothing matches.
pub(crate) fn complete_path(home: &Path, base: &Path, typed: &str) -> Option<String> {
    let (dir_part, prefix) = match typed.rfind('/') {
        Some(i) => (&typed[..=i], &typed[i + 1..]),
        None => ("", typed),
    };
    let dir = if dir_part.is_empty() { base.to_path_buf() } else { expand(home, base, dir_part) };
    let lower = prefix.to_lowercase();
    let mut names: Vec<String> = std::fs::read_dir(&dir)
        .ok()?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_dir())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.to_lowercase().starts_with(&lower) && (prefix.starts_with('.') || !n.starts_with('.')))
        .collect();
    names.sort();
    let first = names.first()?.clone();
    let mut common = first.clone();
    for n in &names[1..] {
        let keep = common.chars().zip(n.chars()).take_while(|(a, b)| a.to_lowercase().eq(b.to_lowercase())).count();
        common = common.chars().take(keep).collect();
    }
    let done = if names.len() == 1 { format!("{first}/") } else { common };
    Some(format!("{dir_part}{done}"))
}

impl Files {
    /// Go > Go to Folder: opens the path sheet, pre-filled with the open folder. The path is selected, so typing a
    /// path replaces it and an arrow key keeps it to add to. (The caret stood after it with nothing selected, so a
    /// path typed from `~` came after the `~/` already there.)
    pub(crate) fn open_goto(&mut self) {
        let mut field = TextField::new();
        let open = if self.in_recents() { self.home.clone() } else { self.path.clone() };
        let shown = match open.strip_prefix(&self.home) {
            Ok(rest) if rest.as_os_str().is_empty() => "~/".to_string(),
            Ok(rest) => format!("~/{}/", rest.display()),
            Err(_) => format!("{}/", open.display()),
        };
        field.push_str(&shown);
        field.select(0..shown.len());
        self.preview = None;
        self.info = None;
        self.tag_panel = false;
        self.fx.goto_error = None;
        self.fx.goto = Some(field);
        self.revision += 1;
    }
    /// Go > Home, Desktop, Documents, Downloads, Recents and any other place.
    pub(crate) fn go_to(&mut self, path: PathBuf) {
        if path != self.path {
            self.navigate(path, true);
        }
    }
    /// Go menu chords (Cmd held). `key` is the key code; returns false when the chord is not a Go command.
    pub(crate) fn go_chord(&mut self, key: u32, shift: bool, alt: bool) -> bool {
        match (key, shift, alt) {
            (34, true, false) => self.open_goto(),                       // Shift+G: Go to Folder
            (35, true, false) => self.go_to(self.home.clone()),          // Shift+H: Home
            (33, true, false) => self.go_to(PathBuf::from(fs::RECENTS)), // Shift+F: Recents
            (32, true, false) => self.go_to(self.home.join("Desktop")),  // Shift+D: Desktop
            (24, true, false) => self.go_to(self.home.join("Documents")), // Shift+O: Documents
            (38, false, true) => self.go_to(self.home.join("Downloads")), // Option+L: Downloads
            (37, false, false) => self.open_connect(),                   // K: Connect to Server
            _ => return false,
        }
        true
    }
    /// Keys for the Go to Folder sheet while it is open. Returns true when the key was used.
    pub(crate) fn goto_input(&mut self, input: &Input) -> bool {
        let Some(field) = self.fx.goto.as_mut() else {
            return false;
        };
        // an edit of the path takes the last answer away; Return gives a new one
        if matches!(input, Input::Key(_)) {
            self.fx.goto_error = None;
        }
        // the Edit commands act on the field: Select All (Cmd+A), Cut, Copy, Paste, Undo and Redo, as the shell sends
        // them and as a chord it left to the app. (They fell through to the folder behind the sheet: Select All
        // selected its items, and the chord was dropped.)
        let command = match input {
            Input::Command(command) => Some(*command),
            Input::Key(key) => key.as_chord().and_then(|(key, mods)| field_chord(key, mods)),
            _ => None,
        };
        if let Some(command) = command {
            let clipboard = bifrost_app::kit::clipboard::text().unwrap_or_default();
            if let Some(text) = field.command(command, &clipboard) {
                bifrost_app::kit::clipboard::set_text(&text);
            }
            if matches!(command, Command::Cut | Command::Paste | Command::Undo | Command::Redo) {
                self.fx.goto_error = None;
            }
            self.revision += 1;
            return true;
        }
        match input {
            Input::Key(KeyInput::Text(s)) => field.push_str(s),
            Input::Key(KeyInput::Code(code)) => match *code {
                13 => {
                    let typed = field.to_string();
                    match goto_target(&self.home, &self.path, &typed) {
                        Some(GotoTarget::Folder(dest)) => {
                            self.fx.goto = None;
                            self.go_to(dest);
                        }
                        // the path of a file: its folder opens with the file selected and scrolled into view, as
                        // Show in Files does (`apply_pending`)
                        Some(GotoTarget::Item(folder, item)) => {
                            self.fx.goto = None;
                            // a file of the folder shown is selected where it is; no second history entry
                            let shown = folder == self.path;
                            if !shown {
                                self.navigate(folder, true);
                            }
                            self.fx.pending = Some(crate::ops::Pending { path: item, rename: false });
                            if shown {
                                self.apply_pending();
                            }
                        }
                        // the sheet stays open and says so under the field, where the path can be corrected. (The
                        // message went to the status bar, behind the sheet's own hint.)
                        None => self.fx.goto_error = Some(goto_missing_message(&typed)),
                    }
                }
                KEY_ESCAPE | KEY_ESCAPE_ASCII => self.fx.goto = None,
                9 => {
                    let typed = field.to_string();
                    if let Some(done) = complete_path(&self.home, &self.path, &typed) {
                        field.clear();
                        field.push_str(&done);
                    }
                }
                8 => field.pop(),
                46 => field.delete(),
                37 | 39 => field.move_caret(*code == 39),
                36 => field.home(),
                35 => field.end(),
                _ => {}
            },
            _ => return false,
        }
        self.revision += 1;
        true
    }
    /// The Go to Folder sheet.
    pub(crate) fn paint_goto(&self, ui: &Ui, s: &mut Surface) {
        let Some(field) = &self.fx.goto else {
            return;
        };
        let w = (self.w - self.side() - 32.).clamp(240., 440.);
        let (x, y) = (self.side() + (self.w - self.side() - w) / 2., 72.);
        rect(s, ui, x, y, w, 118., CARD_BG.now());
        text(s, ui, "Go to the folder:", MAC_TITLE3_PT, LABEL.now(), x + 20., y + 16., w - 40.);
        bifrost_app::widgets::text_field(s, ui, field, Rect { x: x + 20., y: y + 48., w: w - 40., h: 24. }, "", true);
        // the line under the field: why the path could not be opened, else how the sheet works
        match &self.fx.goto_error {
            Some(error) => text(s, ui, error, bifrost_app::kit::ui::MAC_SUBHEAD_PT, LABEL.now(), x + 20., y + 84., w - 40.),
            None => text(s, ui, "Tab completes the name. Return opens it, Escape cancels.", bifrost_app::kit::ui::MAC_SUBHEAD_PT, LABEL_2.now(), x + 20., y + 84., w - 40.),
        }
    }
    /// Tracks the folder a drag hovers over; call when the drop target changes. The open folder itself (the view's
    /// background, a file's row) is a drop target but springs nothing open: opening it again would put the view back
    /// at its top under the drag.
    pub(crate) fn spring_hover(&mut self, target: Option<&PathBuf>) {
        self.fx.spring = target.filter(|t| t.is_dir() && **t != self.path).map(|t| (t.clone(), Instant::now()));
    }
    /// Opens the hovered folder once the delay has passed. Returns true when it navigated.
    pub(crate) fn spring_tick(&mut self) -> bool {
        if !self.dragging {
            self.fx.spring = None;
            return false;
        }
        let Some((dir, since)) = self.fx.spring.clone() else {
            return false;
        };
        if since.elapsed() < SPRING_DELAY {
            return false;
        }
        self.fx.spring = None;
        self.drop_target = None;
        // the folder the drag began in is where it goes back to when it ends without a drop
        let origin = self.fx.spring_from.take().unwrap_or_else(|| (self.path.clone(), self.history.clone(), self.at));
        self.navigate(dir, true);
        self.fx.spring_from = Some(origin);
        true
    }
    /// The drag is over without a drop: a window that sprang folders open shows the folder the drag began in again,
    /// with the history it had. True when it went back.
    pub(crate) fn spring_return(&mut self) -> bool {
        let Some((from, history, at)) = self.fx.spring_from.take() else { return false };
        if self.path == from {
            return false;
        }
        self.history = history;
        self.at = at;
        self.navigate(from, false);
        true
    }
    /// The drop went into a folder: the window stays where the drag took it.
    pub(crate) fn spring_keep(&mut self) {
        self.fx.spring_from = None;
    }
    /// The open folder as the target of a drop on the view's background at (`x`, `y`): a real folder that none of the
    /// dragged items is in already, and that is not inside one of them. A drag inside one folder has no such target
    /// (its items are there already); after a folder sprang open, and for files from another window, it is the
    /// folder shown.
    pub(crate) fn background_target(&self, x: f32, y: f32) -> Option<PathBuf> {
        let inside = x >= self.side() && y >= self.body_top() && y < self.bottom();
        if !inside || self.in_recents() || self.in_bin() || !self.path.is_dir() {
            return None;
        }
        let here = self.path.as_path();
        let blocked = self.fx.drag_paths.iter().any(|p| p.parent() == Some(here) || here.starts_with(p));
        (!blocked).then(|| self.path.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sprung_folder_takes_a_drop_on_its_background_and_an_abandoned_drag_goes_back() {
        let root = std::env::temp_dir().join(format!("files-spring-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("sub")).unwrap();
        std::fs::write(root.join("a.txt"), "x").unwrap();
        let mut files = crate::tests::fixture(vec![]);
        files.home = root.clone();
        files.navigate(root.clone(), true);
        files.dragging = true;
        files.fx.drag_paths = vec![root.join("a.txt")];
        let (x, y) = (files.side() + 200., files.bottom() - 40.);
        assert_eq!(files.background_target(x, y), None, "the items are in this folder already");
        // the drag has hovered over the folder for longer than the delay
        files.fx.spring = Some((root.join("sub"), Instant::now() - SPRING_DELAY - Duration::from_millis(10)));
        assert!(files.spring_tick());
        assert_eq!(files.path, root.join("sub"));
        assert_eq!(files.fx.drag_paths, vec![root.join("a.txt")], "the dragged items outlive the navigation");
        assert_eq!(files.background_target(x, y), Some(root.join("sub")), "the sprung folder takes the drop");
        assert_eq!(files.drop_target_at(x, y), Some(root.join("sub")));
        assert_eq!(files.background_target(1., y), None, "not over the sidebar");
        // the drag is given up: back to where it began, without the sprung folder in the history
        assert!(files.spring_return());
        assert_eq!(files.path, root);
        assert_eq!(files.history, vec![root.clone()]);
        assert!(!files.spring_return(), "once");
        // a folder is never dropped into itself or into a folder inside it
        files.fx.drag_paths = vec![root.clone()];
        files.navigate(root.join("sub"), true);
        assert_eq!(files.background_target(x, y), None);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_folder_springs_open_after_a_second() {
        assert_eq!(SPRING_DELAY, Duration::from_secs(1));
    }

    /// Go to Folder: `~`, `~/sub` and a slash at the end open the folder, the path of a file reveals the file, and a
    /// path that leads nowhere is answered in the sheet, which stays open.
    #[test]
    fn go_to_folder_opens_folders_reveals_files_and_says_when_nothing_is_there() {
        let home = std::env::temp_dir().join(format!("files-goto-return-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(home.join("sub/deep")).unwrap();
        std::fs::write(home.join("sub/notes.txt"), "n").unwrap();
        let base = home.join("sub");
        let folder = |p: PathBuf| Some(GotoTarget::Folder(p));
        assert_eq!(goto_target(&home, &base, "~"), folder(home.clone()));
        assert_eq!(goto_target(&home, &base, "~/"), folder(home.clone()));
        assert_eq!(goto_target(&home, &base, "~/sub"), folder(home.join("sub")));
        assert_eq!(goto_target(&home, &base, "~/sub/"), folder(home.join("sub")), "a slash at the end");
        assert_eq!(goto_target(&home, &base, " ~/sub/deep/ "), folder(home.join("sub/deep")), "spaces around the path");
        assert_eq!(goto_target(&home, &base, "deep"), folder(home.join("sub/deep")), "relative to the folder shown");
        assert_eq!(goto_target(&home, &base, &format!("{}/sub/", home.display())), folder(home.join("sub")), "an absolute path");
        assert_eq!(goto_target(&home, &base, "~/sub/notes.txt"), Some(GotoTarget::Item(home.join("sub"), home.join("sub/notes.txt"))), "a file: its folder, and the file to select");
        assert_eq!(goto_target(&home, &base, "~/nowhere"), None);
        assert_eq!(goto_target(&home, &base, "~/nowhere/notes.txt"), None);
        assert_eq!(goto_target(&home, &base, "  "), None, "an empty field opens nothing");
        // the sheet: Return on a path that leads nowhere keeps it open with the reason under the field
        let mut files = crate::tests::fixture(vec![]);
        files.home = home.clone();
        files.path = home.clone();
        let key = |code: u32| Input::Key(KeyInput::Code(code));
        let type_path = |files: &mut Files, path: &str| {
            files.open_goto();
            if let Some(field) = files.fx.goto.as_mut() {
                field.clear();
                field.push_str(path);
            }
        };
        type_path(&mut files, "~/nowhere");
        assert!(files.goto_input(&key(13)));
        assert!(files.fx.goto.is_some(), "the sheet stays open");
        assert_eq!(files.fx.goto_error.as_deref(), Some("The folder \u{201C}~/nowhere\u{201D} can\u{2019}t be found."));
        assert_eq!(files.path, home, "and the window stays where it was");
        // editing the path takes the message away
        assert!(files.goto_input(&Input::Key(KeyInput::Text("x".into()))));
        assert!(files.fx.goto_error.is_none());
        // a file's path: the sheet closes, the folder opens and the file waits to be selected
        type_path(&mut files, "~/sub/notes.txt");
        assert!(files.goto_input(&key(13)));
        assert!(files.fx.goto.is_none() && files.fx.goto_error.is_none());
        assert_eq!(files.path, home.join("sub"));
        assert_eq!(files.fx.pending, Some(crate::ops::Pending { path: home.join("sub/notes.txt"), rename: false }));
        // a folder with a slash at its end
        type_path(&mut files, "~/sub/deep/");
        assert!(files.goto_input(&key(13)));
        assert_eq!(files.path, home.join("sub/deep"));
        assert!(files.fx.goto.is_none());
        // a file of the folder that is shown already is selected at once, and the history does not grow
        files.path = home.join("sub");
        files.listing = fs::list(&home.join("sub")).unwrap();
        files.refresh_rows();
        let steps = files.history.len();
        type_path(&mut files, "~/sub/notes.txt");
        assert!(files.goto_input(&key(13)));
        assert_eq!(files.selected_paths(), vec![home.join("sub/notes.txt")]);
        assert_eq!(files.history.len(), steps);
        assert!(files.fx.pending.is_none());
        let _ = std::fs::remove_dir_all(&home);
    }

    /// E2E9: the sheet opened with `~/` and the caret after it, so a path typed from `~` gave `~/~/...`; Select All
    /// selected the items of the folder behind the sheet and left the field as it was.
    #[test]
    fn go_to_folder_starts_with_its_path_selected_and_select_all_acts_on_the_field() {
        let mut files = crate::tests::fixture(vec![crate::tests::folder("A"), crate::tests::folder("B")]);
        let text = |s: &str| Input::Key(KeyInput::Text(s.into()));
        let shown = |files: &Files| files.fx.goto.as_ref().map(|f| (f.to_string(), f.selection())).unwrap();
        files.open_goto();
        assert_eq!(shown(&files), ("~/".to_string(), 0..2), "the path of the folder shown, selected");
        for part in ["~", "/", "Documents"] {
            assert!(files.goto_input(&text(part)));
        }
        assert_eq!(shown(&files), ("~/Documents".to_string(), 11..11), "typing replaces what the sheet opened with");
        // Edit > Select All, as the shell sends it: the Edit menu is the field's while the sheet is open
        assert!(files.edit_commands().supports(Command::SelectAll));
        assert!(!files.edit_commands().supports(Command::Copy), "nothing is selected in the field");
        assert!(files.goto_input(&Input::Command(Command::SelectAll)));
        assert_eq!(shown(&files), ("~/Documents".to_string(), 0..11));
        assert!(files.selection.is_empty(), "the folder behind the sheet keeps its selection");
        assert!(files.edit_commands().supports(Command::Copy));
        assert!(files.goto_input(&text("/")));
        assert!(files.goto_input(&text("tmp")));
        assert_eq!(shown(&files), ("/tmp".to_string(), 4..4));
        // the chord, where the shell leaves the key to the app
        assert!(files.goto_input(&Input::Key(KeyInput::chord(30, CHORD_META))));
        assert_eq!(shown(&files), ("/tmp".to_string(), 0..4));
        assert!(files.selection.is_empty());
        assert!(files.goto_input(&text("~")));
        assert_eq!(shown(&files), ("~".to_string(), 1..1));
        // Right keeps the path the sheet opened with, to add to
        files.open_goto();
        assert!(files.goto_input(&Input::Key(KeyInput::Code(39))));
        assert!(files.goto_input(&text("A")));
        assert_eq!(shown(&files), ("~/A".to_string(), 3..3));
        // with the sheet closed the command is the window's again
        files.fx.goto = None;
        assert!(!files.goto_input(&Input::Command(Command::SelectAll)));
    }

    #[test]
    fn a_chord_in_a_field_is_its_edit_command() {
        assert_eq!(field_chord(30, CHORD_META), Some(Command::SelectAll));
        assert_eq!(field_chord(30, CHORD_CTRL), Some(Command::SelectAll));
        assert_eq!(field_chord(46, CHORD_META), Some(Command::Copy));
        assert_eq!(field_chord(45, CHORD_META), Some(Command::Cut));
        assert_eq!(field_chord(47, CHORD_META), Some(Command::Paste));
        assert_eq!(field_chord(44, CHORD_META), Some(Command::Undo));
        assert_eq!(field_chord(44, CHORD_META | CHORD_SHIFT), Some(Command::Redo));
        assert_eq!(field_chord(30, CHORD_META | CHORD_SHIFT), None);
        assert_eq!(field_chord(30, CHORD_META | CHORD_ALT), None, "Option makes another command");
        assert_eq!(field_chord(30, 0), None);
        assert_eq!(field_chord(34, CHORD_META | CHORD_SHIFT), None, "Go to Folder's own key");
    }

    #[test]
    fn expand_handles_home_absolute_and_relative() {
        let (home, base) = (Path::new("/home/u"), Path::new("/srv"));
        assert_eq!(expand(home, base, "~"), PathBuf::from("/home/u"));
        assert_eq!(expand(home, base, "~/Documents"), PathBuf::from("/home/u/Documents"));
        assert_eq!(expand(home, base, "/etc"), PathBuf::from("/etc"));
        assert_eq!(expand(home, base, "data"), PathBuf::from("/srv/data"));
    }

    #[test]
    fn complete_path_finishes_a_unique_folder_and_stops_at_the_common_prefix() {
        let root = std::env::temp_dir().join(format!("files-goto-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for d in ["Photos", "Pictures", "Music"] {
            std::fs::create_dir_all(root.join(d)).unwrap();
        }
        let typed = |p: &str| format!("{}/{p}", root.display());
        assert_eq!(complete_path(&root, &root, &typed("mu")), Some(typed("Music/")));
        assert_eq!(complete_path(&root, &root, &typed("P")), Some(typed("P")));
        assert_eq!(complete_path(&root, &root, &typed("x")), None);
        let _ = std::fs::remove_dir_all(&root);
    }
}
