//! Files's Go menu (Go to Folder with path completion, Connect to Server, Home, Desktop, Documents, Downloads, Recents), spring-loaded folders
//! and the Option-to-copy drag state.
use crate::*;

/// How long a drag must hover over a folder before it opens (Files's spring-loaded delay).
pub(crate) const SPRING_DELAY: Duration = Duration::from_millis(800);

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
    /// The decoded picture of the Preview panel (JPEG and PNG).
    pub(crate) preview_image: Option<springboard::gfx::Image>,
    /// Go > Connect to Server: the address sheet, the remote folder browser and the recent servers.
    pub(crate) server: crate::server::ServerState,
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
    /// Go > Go to Folder: opens the path sheet, pre-filled with the open folder.
    pub(crate) fn open_goto(&mut self) {
        let mut field = TextField::new();
        let open = if self.in_recents() { self.home.clone() } else { self.path.clone() };
        let shown = match open.strip_prefix(&self.home) {
            Ok(rest) if rest.as_os_str().is_empty() => "~/".to_string(),
            Ok(rest) => format!("~/{}/", rest.display()),
            Err(_) => format!("{}/", open.display()),
        };
        field.push_str(&shown);
        self.preview = None;
        self.info = None;
        self.tag_panel = false;
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
        match input {
            Input::Key(KeyInput::Text(s)) => field.push_str(s),
            Input::Key(KeyInput::Code(code)) => match *code {
                13 => {
                    let typed = field.to_string();
                    let dest = expand(&self.home, &self.path, &typed);
                    if dest.is_dir() {
                        self.fx.goto = None;
                        self.go_to(dest);
                    } else {
                        self.notice = Some(format!("The folder \"{}\" can't be found", typed.trim()));
                    }
                }
                27 => self.fx.goto = None,
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
        text(s, ui, "Tab completes the name. Return opens it, Escape cancels.", bifrost_app::kit::ui::MAC_SUBHEAD_PT, LABEL_2.now(), x + 20., y + 84., w - 40.);
    }
    /// Tracks the folder a drag hovers over; call when the drop target changes.
    pub(crate) fn spring_hover(&mut self, target: Option<&PathBuf>) {
        self.fx.spring = target.filter(|t| t.is_dir()).map(|t| (t.clone(), Instant::now()));
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
        self.navigate(dir, true);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
