//! File-manager sync extensions: the extension point that lets an app add badges, shortcut-menu items and a toolbar
//! button to the Files app, and watch the folders it cares about (the job of the reference OS's file-manager sync
//! extension and its private host framework).
//!
//! This is the in-process model. An extension implements [`SyncExtension`]; the file manager owns one [`SyncHost`] and
//! asks it three questions: which folders are being observed (so the extension is told when a window opens or closes
//! one), which badge an item shows, and which menu items an extension adds for a context. The extension answers
//! through a [`SyncController`], which holds its observed folders, its registered badges and the badge set on each
//! item, and which also carries the host's view of the current selection (`targeted`, `selected`).
//!
//! Rules kept from the reference behaviour: an extension only sees and badges paths inside the folders it asked to
//! observe; a badge must be registered (id, label, image name) before an item can carry it; setting the badge id to
//! `None` clears it; an extension that asks for a badge on a path outside its folders is refused. Nothing is queried
//! remotely and no extension code runs in the file manager's process: a later extension host speaks this same API
//! over a socket.
//!
//! Gaps: no extension process and no IPC yet, no enable screen in Settings, no service-name list, and badge images
//! are names for the shell's icon set (there is no image data path).

use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

/// Most badges one extension may register.
pub const MAX_BADGES: usize = 16;
/// Most items one menu may hold.
pub const MAX_MENU_ITEMS: usize = 32;

/// Why a call was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SyncError {
    /// The path is not absolute, or holds `..`.
    BadPath,
    /// The path is outside every folder the extension observes.
    NotObserved,
    /// The badge id is blank, already registered, or the extension has [`MAX_BADGES`].
    BadBadge,
    /// The badge id was never registered.
    UnknownBadge,
    /// No extension with that id is registered.
    UnknownExtension,
}

/// A badge an extension registered: the id the extension uses, the label the person sees, the icon name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Badge {
    pub id: String,
    pub label: String,
    pub image: String,
}

/// Where a menu is shown.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuKind {
    /// the shortcut menu of the selected items
    ContextualItems,
    /// the shortcut menu of the folder background
    ContextualContainer,
    /// the shortcut menu of a sidebar entry
    Sidebar,
    /// the menu behind the extension's toolbar button
    Toolbar,
}

/// One row of a menu an extension adds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MenuItem {
    pub id: u32,
    pub title: String,
    pub enabled: bool,
}

impl MenuItem {
    pub fn new(id: u32, title: &str) -> MenuItem {
        MenuItem { id, title: title.to_string(), enabled: true }
    }
}

/// The toolbar button an extension adds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolbarButton {
    pub name: String,
    pub image: String,
    pub tool_tip: String,
}

/// What an extension implements. All methods have do-nothing defaults.
pub trait SyncExtension {
    /// A folder the extension observes came into view.
    fn begin_observing(&mut self, _dir: &Path) {}
    /// A folder the extension observes left every window.
    fn end_observing(&mut self, _dir: &Path) {}
    /// The file manager needs the badge for `path`. The extension answers with `controller.set_badge`; the host also
    /// keeps any badge set earlier.
    fn request_badge(&mut self, _controller: &mut SyncController, _path: &Path) {}
    /// The items this extension adds to a menu.
    fn menu(&mut self, _kind: MenuKind, _controller: &SyncController) -> Vec<MenuItem> {
        Vec::new()
    }
    /// The toolbar button, if the extension has one.
    fn toolbar(&self) -> Option<ToolbarButton> {
        None
    }
}

fn clean(path: &Path) -> Result<PathBuf, SyncError> {
    if !path.is_absolute() {
        return Err(SyncError::BadPath);
    }
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            Component::ParentDir => return Err(SyncError::BadPath),
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    Ok(out)
}

/// One extension's state: its folders, its badges, the badge on each item, and the host's selection.
#[derive(Clone, Debug, Default)]
pub struct SyncController {
    directories: Vec<PathBuf>,
    badges: Vec<Badge>,
    item_badges: BTreeMap<PathBuf, String>,
    targeted: Option<PathBuf>,
    selected: Vec<PathBuf>,
}

impl SyncController {
    /// The folders the extension observes.
    pub fn directory_urls(&self) -> &[PathBuf] {
        &self.directories
    }

    /// Replaces the folder set. Badges on items that fell outside the new set are dropped.
    pub fn set_directory_urls(&mut self, dirs: &[PathBuf]) -> Result<(), SyncError> {
        let mut cleaned = Vec::new();
        for d in dirs {
            let d = clean(d)?;
            if !cleaned.contains(&d) {
                cleaned.push(d);
            }
        }
        self.directories = cleaned;
        let dirs = self.directories.clone();
        self.item_badges.retain(|p, _| dirs.iter().any(|d| p.starts_with(d)));
        Ok(())
    }

    fn observes(&self, path: &Path) -> bool {
        self.directories.iter().any(|d| path.starts_with(d))
    }

    /// Registers a badge. The id is unique per extension.
    pub fn set_badge_image(&mut self, id: &str, label: &str, image: &str) -> Result<(), SyncError> {
        let id = id.trim();
        if id.is_empty() || self.badges.len() >= MAX_BADGES || self.badges.iter().any(|b| b.id == id) {
            return Err(SyncError::BadBadge);
        }
        self.badges.push(Badge { id: id.to_string(), label: label.trim().to_string(), image: image.to_string() });
        Ok(())
    }

    /// The registered badges, in registration order.
    pub fn badges(&self) -> &[Badge] {
        &self.badges
    }

    /// Puts the badge `id` on `path`, or clears it with `None`.
    pub fn set_badge(&mut self, path: &Path, id: Option<&str>) -> Result<(), SyncError> {
        let path = clean(path)?;
        if !self.observes(&path) {
            return Err(SyncError::NotObserved);
        }
        match id {
            None => {
                self.item_badges.remove(&path);
            }
            Some(id) => {
                if !self.badges.iter().any(|b| b.id == id) {
                    return Err(SyncError::UnknownBadge);
                }
                self.item_badges.insert(path, id.to_string());
            }
        }
        Ok(())
    }

    /// The badge on `path`.
    pub fn badge_for(&self, path: &Path) -> Option<&Badge> {
        let id = self.item_badges.get(path)?;
        self.badges.iter().find(|b| &b.id == id)
    }

    /// The folder the person is looking at, as the host last reported it.
    pub fn targeted_url(&self) -> Option<&Path> {
        self.targeted.as_deref()
    }

    /// The selected items, as the host last reported them. Empty outside the extension's folders.
    pub fn selected_item_urls(&self) -> &[PathBuf] {
        &self.selected
    }

    fn set_context(&mut self, targeted: Option<&Path>, selected: &[PathBuf]) {
        self.targeted = targeted.and_then(|t| clean(t).ok()).filter(|t| self.observes(t));
        let selected: Vec<PathBuf> = selected.iter().filter_map(|p| clean(p).ok()).filter(|p| self.observes(p)).collect();
        self.selected = selected;
    }
}

struct Registered {
    id: String,
    controller: SyncController,
    extension: Box<dyn SyncExtension>,
    /// folders currently reported as begun, so begin and end calls pair up
    open: Vec<PathBuf>,
}

/// The file manager's side: every enabled extension and what it asked for.
#[derive(Default)]
pub struct SyncHost {
    extensions: Vec<Registered>,
}

/// A menu item with the extension that added it, so a tap goes back to the right extension.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostMenuItem {
    pub extension: String,
    pub item: MenuItem,
}

impl SyncHost {
    pub fn new() -> SyncHost {
        SyncHost::default()
    }

    /// Enables an extension. An id already in use replaces the older extension.
    pub fn register(&mut self, id: &str, controller: SyncController, extension: Box<dyn SyncExtension>) {
        self.extensions.retain(|e| e.id != id);
        self.extensions.push(Registered { id: id.to_string(), controller, extension, open: Vec::new() });
    }

    /// Disables an extension, ending the folders it still had open.
    pub fn unregister(&mut self, id: &str) -> bool {
        let Some(i) = self.extensions.iter().position(|e| e.id == id) else { return false };
        let mut gone = self.extensions.remove(i);
        for dir in std::mem::take(&mut gone.open) {
            gone.extension.end_observing(&dir);
        }
        true
    }

    /// The controller of one extension.
    pub fn controller(&self, id: &str) -> Option<&SyncController> {
        self.extensions.iter().find(|e| e.id == id).map(|e| &e.controller)
    }

    /// Ids of the enabled extensions, in the order they were enabled.
    pub fn extension_ids(&self) -> Vec<&str> {
        self.extensions.iter().map(|e| e.id.as_str()).collect()
    }

    /// A window now shows `dir`. Every extension whose folders contain it hears `begin_observing` once, until the
    /// matching [`SyncHost::folder_closed`].
    pub fn folder_opened(&mut self, dir: &Path) -> Result<usize, SyncError> {
        let dir = clean(dir)?;
        let mut told = 0;
        for e in &mut self.extensions {
            if e.controller.observes(&dir) && !e.open.contains(&dir) {
                e.open.push(dir.clone());
                e.extension.begin_observing(&dir);
                told += 1;
            }
        }
        Ok(told)
    }

    /// No window shows `dir` any more.
    pub fn folder_closed(&mut self, dir: &Path) -> Result<usize, SyncError> {
        let dir = clean(dir)?;
        let mut told = 0;
        for e in &mut self.extensions {
            if let Some(i) = e.open.iter().position(|d| *d == dir) {
                e.open.remove(i);
                e.extension.end_observing(&dir);
                told += 1;
            }
        }
        Ok(told)
    }

    /// The badges that show on `path`: one per extension that badged it, in the order the extensions were enabled.
    /// Each extension is asked first, so a badge it has not set yet appears on the next draw.
    pub fn badges_for(&mut self, path: &Path) -> Vec<Badge> {
        let Ok(path) = clean(path) else { return Vec::new() };
        let mut out = Vec::new();
        for e in &mut self.extensions {
            if !e.controller.observes(&path) {
                continue;
            }
            e.extension.request_badge(&mut e.controller, &path);
            if let Some(b) = e.controller.badge_for(&path) {
                out.push(b.clone());
            }
        }
        out
    }

    /// The items the extensions add to a menu. The host's selection is handed to each extension first, narrowed to its
    /// own folders; an extension with no folder covering the targeted folder adds nothing.
    pub fn menu(&mut self, kind: MenuKind, targeted: &Path, selected: &[PathBuf]) -> Vec<HostMenuItem> {
        let mut out = Vec::new();
        for e in &mut self.extensions {
            e.controller.set_context(Some(targeted), selected);
            if e.controller.targeted_url().is_none() {
                continue;
            }
            let mut items = e.extension.menu(kind, &e.controller);
            items.truncate(MAX_MENU_ITEMS);
            out.extend(items.into_iter().map(|item| HostMenuItem { extension: e.id.clone(), item }));
        }
        out
    }

    /// The toolbar buttons of the extensions that have one, with the extension that owns each.
    pub fn toolbar_buttons(&self) -> Vec<(String, ToolbarButton)> {
        self.extensions.iter().filter_map(|e| e.extension.toolbar().map(|b| (e.id.clone(), b))).collect()
    }

    /// Whether `extension` is enabled (for a tap that arrives after it was disabled).
    pub fn has(&self, extension: &str) -> Result<(), SyncError> {
        if self.extensions.iter().any(|e| e.id == extension) {
            Ok(())
        } else {
            Err(SyncError::UnknownExtension)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    #[derive(Default)]
    struct Log(Vec<String>);

    struct Cloud {
        log: Rc<RefCell<Log>>,
    }

    impl SyncExtension for Cloud {
        fn begin_observing(&mut self, dir: &Path) {
            self.log.borrow_mut().0.push(format!("begin {}", dir.display()));
        }
        fn end_observing(&mut self, dir: &Path) {
            self.log.borrow_mut().0.push(format!("end {}", dir.display()));
        }
        fn request_badge(&mut self, c: &mut SyncController, path: &Path) {
            if path.extension().is_some_and(|e| e == "tmp") {
                c.set_badge(path, Some("pending")).unwrap();
            }
        }
        fn menu(&mut self, kind: MenuKind, c: &SyncController) -> Vec<MenuItem> {
            match kind {
                MenuKind::ContextualItems if !c.selected_item_urls().is_empty() => vec![MenuItem::new(1, "Share Link")],
                MenuKind::ContextualContainer => vec![MenuItem::new(2, "Sync This Folder")],
                MenuKind::Toolbar => vec![MenuItem { id: 3, title: "Pause Sync".into(), enabled: false }],
                _ => Vec::new(),
            }
        }
        fn toolbar(&self) -> Option<ToolbarButton> {
            Some(ToolbarButton { name: "Cloud".into(), image: "cloud".into(), tool_tip: "Sync status".into() })
        }
    }

    fn controller(dirs: &[&str]) -> SyncController {
        let mut c = SyncController::default();
        c.set_directory_urls(&dirs.iter().map(PathBuf::from).collect::<Vec<_>>()).unwrap();
        c.set_badge_image("pending", "Waiting to sync", "arrow.clockwise").unwrap();
        c.set_badge_image("done", "Up to date", "checkmark").unwrap();
        c
    }

    fn host(dirs: &[&str]) -> (SyncHost, Rc<RefCell<Log>>) {
        let log = Rc::new(RefCell::new(Log::default()));
        let mut h = SyncHost::new();
        h.register("cloud", controller(dirs), Box::new(Cloud { log: log.clone() }));
        (h, log)
    }

    #[test]
    fn badges_are_registered_once_and_must_exist_before_use() {
        let mut c = controller(&["/home/a/Sync"]);
        assert_eq!(c.set_badge_image("done", "x", "y"), Err(SyncError::BadBadge));
        assert_eq!(c.set_badge_image("  ", "x", "y"), Err(SyncError::BadBadge));
        let item = Path::new("/home/a/Sync/report.txt");
        assert_eq!(c.set_badge(item, Some("missing")), Err(SyncError::UnknownBadge));
        c.set_badge(item, Some("done")).unwrap();
        assert_eq!(c.badge_for(item).map(|b| b.label.as_str()), Some("Up to date"));
        c.set_badge(item, None).unwrap();
        assert!(c.badge_for(item).is_none());
    }

    #[test]
    fn at_most_sixteen_badges_per_extension() {
        let mut c = SyncController::default();
        for i in 0..MAX_BADGES {
            c.set_badge_image(&format!("b{i}"), "l", "i").unwrap();
        }
        assert_eq!(c.set_badge_image("extra", "l", "i"), Err(SyncError::BadBadge));
    }

    #[test]
    fn an_extension_only_badges_paths_inside_its_folders() {
        let mut c = controller(&["/home/a/Sync"]);
        assert_eq!(c.set_badge(Path::new("/home/a/Other/x"), Some("done")), Err(SyncError::NotObserved));
        assert_eq!(c.set_badge(Path::new("/home/a/Sync/../Other/x"), Some("done")), Err(SyncError::BadPath));
        assert_eq!(c.set_badge(Path::new("Sync/x"), Some("done")), Err(SyncError::BadPath));
        // a sibling that only shares a name prefix is outside the folder
        assert_eq!(c.set_badge(Path::new("/home/a/SyncX/x"), Some("done")), Err(SyncError::NotObserved));
    }

    #[test]
    fn shrinking_the_folder_set_drops_badges_that_fell_outside() {
        let mut c = controller(&["/home/a/One", "/home/a/Two"]);
        c.set_badge(Path::new("/home/a/One/f"), Some("done")).unwrap();
        c.set_badge(Path::new("/home/a/Two/f"), Some("done")).unwrap();
        c.set_directory_urls(&[PathBuf::from("/home/a/Two")]).unwrap();
        assert!(c.badge_for(Path::new("/home/a/One/f")).is_none());
        assert!(c.badge_for(Path::new("/home/a/Two/f")).is_some());
        assert_eq!(c.set_directory_urls(&[PathBuf::from("rel")]), Err(SyncError::BadPath));
        assert_eq!(c.directory_urls().len(), 1, "a refused set leaves the folders alone");
    }

    #[test]
    fn begin_and_end_pair_up_per_folder() {
        let (mut h, log) = host(&["/home/a/Sync"]);
        assert_eq!(h.folder_opened(Path::new("/home/a/Sync/docs")), Ok(1));
        assert_eq!(h.folder_opened(Path::new("/home/a/Sync/docs")), Ok(0), "a second window on the same folder is not told again");
        assert_eq!(h.folder_opened(Path::new("/home/a/Elsewhere")), Ok(0));
        assert_eq!(h.folder_closed(Path::new("/home/a/Sync/docs")), Ok(1));
        assert_eq!(h.folder_closed(Path::new("/home/a/Sync/docs")), Ok(0));
        assert_eq!(log.borrow().0, ["begin /home/a/Sync/docs", "end /home/a/Sync/docs"]);
    }

    #[test]
    fn disabling_an_extension_ends_its_open_folders() {
        let (mut h, log) = host(&["/home/a/Sync"]);
        h.folder_opened(Path::new("/home/a/Sync")).unwrap();
        assert!(h.unregister("cloud"));
        assert!(!h.unregister("cloud"));
        assert_eq!(log.borrow().0.last().map(String::as_str), Some("end /home/a/Sync"));
        assert_eq!(h.has("cloud"), Err(SyncError::UnknownExtension));
    }

    #[test]
    fn the_host_asks_for_a_badge_and_returns_it() {
        let (mut h, _) = host(&["/home/a/Sync"]);
        let got = h.badges_for(Path::new("/home/a/Sync/draft.tmp"));
        assert_eq!(got.iter().map(|b| b.id.as_str()).collect::<Vec<_>>(), ["pending"]);
        assert!(h.badges_for(Path::new("/home/a/Sync/final.txt")).is_empty());
        assert!(h.badges_for(Path::new("/home/a/Elsewhere/draft.tmp")).is_empty(), "outside the extension's folders");
        assert_eq!(h.controller("cloud").unwrap().badges().len(), 2);
    }

    #[test]
    fn menus_follow_the_kind_and_the_selection_is_narrowed_to_the_extension() {
        let (mut h, _) = host(&["/home/a/Sync"]);
        let dir = Path::new("/home/a/Sync");
        let none = h.menu(MenuKind::ContextualItems, dir, &[]);
        assert!(none.is_empty(), "no selection, no item menu");
        let outside = h.menu(MenuKind::ContextualItems, dir, &[PathBuf::from("/home/a/Elsewhere/x")]);
        assert!(outside.is_empty(), "selected items outside the folders are not shown to the extension");
        let some = h.menu(MenuKind::ContextualItems, dir, &[PathBuf::from("/home/a/Sync/x")]);
        assert_eq!(some, vec![HostMenuItem { extension: "cloud".into(), item: MenuItem::new(1, "Share Link") }]);
        assert_eq!(h.controller("cloud").unwrap().selected_item_urls(), [PathBuf::from("/home/a/Sync/x")]);
        let bg = h.menu(MenuKind::ContextualContainer, dir, &[]);
        assert_eq!(bg[0].item.title, "Sync This Folder");
        let tool = h.menu(MenuKind::Toolbar, dir, &[]);
        assert!(!tool[0].item.enabled);
    }

    #[test]
    fn a_folder_outside_the_extension_gets_no_menu_and_the_toolbar_button_is_listed() {
        let (mut h, _) = host(&["/home/a/Sync"]);
        assert!(h.menu(MenuKind::ContextualContainer, Path::new("/home/a/Elsewhere"), &[]).is_empty());
        let buttons = h.toolbar_buttons();
        assert_eq!(buttons.len(), 1);
        assert_eq!((buttons[0].0.as_str(), buttons[0].1.name.as_str()), ("cloud", "Cloud"));
    }

    #[test]
    fn registering_an_id_again_replaces_the_extension() {
        let (mut h, log) = host(&["/home/a/Sync"]);
        h.register("cloud", controller(&["/home/a/Other"]), Box::new(Cloud { log: log.clone() }));
        assert_eq!(h.extension_ids(), ["cloud"]);
        assert_eq!(h.folder_opened(Path::new("/home/a/Sync")), Ok(0));
        assert_eq!(h.folder_opened(Path::new("/home/a/Other")), Ok(1));
    }

    #[test]
    fn menus_are_capped() {
        struct Many;
        impl SyncExtension for Many {
            fn menu(&mut self, _: MenuKind, _: &SyncController) -> Vec<MenuItem> {
                (0..100).map(|i| MenuItem::new(i, "x")).collect()
            }
        }
        let mut h = SyncHost::new();
        h.register("many", controller(&["/d"]), Box::new(Many));
        assert_eq!(h.menu(MenuKind::Sidebar, Path::new("/d"), &[]).len(), MAX_MENU_ITEMS);
    }
}
