//! The source list: Favorites, Locations and Tags sections (Files's sidebar).
use crate::*;

/// The sidebar column's width, its rows' start below the traffic lights, and the header band's height (points).
pub(crate) const SIDEBAR_W: f32 = 160.;

pub(crate) const SIDEBAR_SCROLL: f32 = -(UNIFIED_BAR_PT + 4.);

/// Stand-in folder of the Tags sidebar row: every tagged item.
pub(crate) const TAGGED: &str = "/.tagged";

/// The stand-in folder of one tag's sidebar row.
pub(crate) fn tag_path(tag: Tag) -> PathBuf {
    Path::new(TAGGED).join(tag.label())
}

/// The tag a stand-in tag folder lists (`None` for every tagged item), or `None` outside them.
pub(crate) fn tag_filter(path: &Path) -> Option<Option<Tag>> {
    let rest = path.strip_prefix(TAGGED).ok()?;
    Some(Tag::ALL.into_iter().find(|t| rest == Path::new(t.label())))
}

/// One sidebar row: its label, glyph or tag dot, and the place it opens.
pub(crate) struct Place {
    pub(crate) label: String,
    pub(crate) symbol: &'static str,
    pub(crate) dot: Option<u32>,
    pub(crate) path: PathBuf,
    /// The device a volume row ejects (`/dev/sdb1`, or `/dev/loop0` for a disk image); None for every other row.
    pub(crate) eject: Option<String>,
}

/// Width of the row's right end where the eject mark is (points): a click there ejects the volume, not opens it.
/// The mark is `TRAILING_GLYPH_PT` wide and ends 18 points before the column's edge; the zone starts where the
/// label's room ends.
const EJECT_W: f32 = 40.;

/// The glyph of a row that is a volume or a provider's storage, and the mark a removable volume ejects with.
const VOLUME_SYMBOL: &str = "externaldrive";
const EJECT_SYMBOL: &str = "eject";

/// The source-list row of `place` with this id, under the section `header`. A removable volume has the drive glyph
/// and a small eject mark at its end instead of the word, so its name has the row's width up to the mark.
fn place_row(id: u32, place: &Place, header: Option<&str>) -> SidebarItem {
    let row = SidebarItem::row(id, &place.label);
    let row = if place.eject.is_some() { row.with_trailing_symbol(EJECT_SYMBOL) } else { row };
    match place.dot {
        Some(c) => row.with_dot(c),
        // Favorites and the top rows take the accent; Locations (home, computer, trash, volumes) the secondary grey
        None if header == Some("Locations") => row.with_symbol(place.symbol).neutral(),
        None => row.with_symbol(place.symbol),
    }
}

/// Whether `path` is a folder of a volume that is not mounted any more (ejected here or elsewhere, or pulled out).
/// Only a path under the media folders reads the mount table.
pub(crate) fn lost_volume(path: &Path) -> bool {
    use springboard::files::volumes;
    if !volumes::under_media(path) {
        return false;
    }
    let mounts: Vec<PathBuf> = volumes::mounted().into_iter().map(|v| v.mount).collect();
    volumes::on_lost_volume(path, path.is_dir(), &mounts)
}

/// Drops the entries `lost` names from a navigation history and returns the position that replaces `at`: the nearest
/// kept entry before it, else the first kept one. Equal neighbours the removal leaves become one entry. A history
/// with no such entry is left as it is.
pub(crate) fn forget_lost(history: &mut Vec<PathBuf>, at: usize, lost: &dyn Fn(&Path) -> bool) -> usize {
    if !history.iter().any(|p| lost(p)) {
        return at;
    }
    let mut kept: Vec<PathBuf> = Vec::with_capacity(history.len());
    let mut new_at = 0;
    for (i, p) in std::mem::take(history).into_iter().enumerate() {
        if !lost(&p) && kept.last() != Some(&p) {
            kept.push(p);
        }
        if i <= at {
            new_at = kept.len().saturating_sub(1);
        }
    }
    *history = kept;
    new_at
}

impl Files {
    /// A volume went away (this app ejected it, or the shell says the disks changed): every tab of this window and of
    /// the New Window ones leaves its folders, and they leave every history, so Back and Forward never open a folder
    /// that is gone. The window showing one opens the history entry that replaces it, or the home folder
    /// (`navigate`). True when anything changed.
    pub(crate) fn leave_lost_volumes(&mut self) -> bool {
        let home = self.home.clone();
        let mut changed = false;
        for tab in self.tabs.items_mut() {
            changed |= tab.leave_lost(&home, &lost_volume);
        }
        if lost_volume(&self.path) {
            let gone = self.path.clone();
            self.navigate(gone, false);
            changed = true;
        } else {
            let before = self.history.len();
            self.at = forget_lost(&mut self.history, self.at, &lost_volume);
            changed |= self.history.len() != before;
        }
        for e in &mut self.windows {
            changed |= e.files.leave_lost_volumes();
        }
        if changed {
            self.revision += 1;
        }
        changed
    }
    pub(crate) fn sidebar_bounds(&self) -> Rect {
        Rect { x: 0., y: 0., w: self.side(), h: self.h }
    }
    /// The home folder's name in the sidebar and title (Files: the user's short name).
    pub(crate) fn home_name(&self) -> String {
        self.home.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or("Home".into())
    }
    /// The folder the Applications row opens: the user's or the system's Applications folder when there is one, else
    /// where BifrOSt keeps the installed store apps (`<shell data>/apps/<id>/`).
    pub(crate) fn applications_dir(&self) -> Option<PathBuf> {
        [self.home.join("Applications"), PathBuf::from("/Applications"), springboard::shell::apphost::root().join("apps")].into_iter().find(|p| p.is_dir())
    }
    /// The sidebar's sections in Files's order: Recents without a header, Favorites, Locations, Tags.
    pub(crate) fn sections(&self) -> Vec<(Option<&'static str>, Vec<Place>)> {
        let place = |label: &str, symbol: &'static str, path: PathBuf| Place { label: label.into(), symbol, dot: None, path, eject: None };
        let mut favorites = vec![];
        if let Some(apps) = self.applications_dir() {
            favorites.push(place("Applications", "square.grid.2x2", apps));
        }
        favorites.push(place("Desktop", "menubar.dock.rectangle", self.home.join("Desktop")));
        favorites.push(place("Documents", "doc", self.home.join("Documents")));
        favorites.push(place("Downloads", "arrow.down.circle", self.home.join("Downloads")));
        let locations = vec![
            place(&self.home_name(), "house", self.home.clone()),
            place("Computer", "desktopcomputer", PathBuf::from("/")),
            place("Trash", "trash", self.bin()),
        ];
        // removable volumes mounted now follow Trash, each with an eject mark; under the test pin none are listed, so a
        // snapshot does not depend on the disks plugged into the machine that runs it
        let mounted = if bifrost_app::testing::pinned() { Vec::new() } else { springboard::files::volumes::mounted() };
        let volumes = mounted.into_iter().map(|v| Place { label: v.name, symbol: VOLUME_SYMBOL, dot: None, path: v.mount, eject: Some(v.device) });
        // file provider domains (installed apps that serve one) follow the volumes
        let providers = provider::shared().rows().into_iter().map(|(label, path)| Place { label, symbol: VOLUME_SYMBOL, dot: None, path, eject: None });
        let locations: Vec<Place> = locations.into_iter().chain(volumes).chain(providers).collect();
        let tags = Tag::ALL.into_iter().map(|t| Place { label: t.label().into(), symbol: "tag", dot: Some(tag_colour(t)), path: tag_path(t), eject: None }).collect();
        vec![
            (None, vec![place("Recents", "clock", PathBuf::from(fs::RECENTS))]),
            (Some("Favorites"), favorites),
            (Some("Locations"), locations),
            (Some("Tags"), tags),
        ]
        .into_iter()
        .chain(self.smart_section())
        .collect()
    }
    /// The Smart Folders section: one row per saved search, none when there are none.
    pub(crate) fn smart_section(&self) -> Vec<(Option<&'static str>, Vec<Place>)> {
        if self.smart.is_empty() {
            return vec![];
        }
        let rows = self.smart.iter().enumerate().map(|(i, f)| Place { label: f.name.clone(), symbol: "magnifyingglass", dot: None, path: smart::smart_path(i), eject: None }).collect();
        vec![(Some("Smart Folders"), rows)]
    }
    /// Every sidebar row in order; row `i` has id `100 + i`.
    pub(crate) fn places(&self) -> Vec<Place> {
        self.sections().into_iter().flat_map(|(_, rows)| rows).collect()
    }
    /// The source list, each row with its Files glyph or tag dot; the open place is selected.
    pub(crate) fn sidebar(&self) -> SourceList {
        let mut items = vec![];
        let mut id = 100;
        for (header, rows) in self.sections() {
            if let Some(h) = header {
                items.push(SidebarItem::header(h));
            }
            for p in rows {
                items.push(place_row(id, &p, header));
                id += 1;
            }
        }
        let mut list = SourceList::new(items, 32., 30.);
        list.set_selected(self.sidebar_id(&self.path));
        list
    }
    /// A click in the sidebar: a row opens its place, and the eject mark of a removable volume ejects it.
    pub(crate) fn sidebar_click(&mut self, x: f32, y: f32) {
        let Some(id) = self.sidebar().hit(self.sidebar_bounds(), self.sidebar_scroll(), x, y) else { return };
        let Some(place) = self.places().into_iter().nth(id.saturating_sub(100) as usize) else { return };
        match place.eject {
            Some(device) if x > self.side() - EJECT_W => self.eject_volume(&place.label, &device, &place.path),
            _ => self.navigate(place.path, true),
        }
    }
    /// Ejects a volume. A folder open inside it is left first (`leave_volume`). The shell does the work
    /// (`system::eject_effect` picks the request: a disk image's volume, or a removable disk's); its event brings a
    /// failure, which the status bar shows. The row goes away on the next sidebar build once the volume is gone, and
    /// the event of an eject that worked takes the volume's folders out of every tab, window and history
    /// (`leave_lost_volumes`).
    fn eject_volume(&mut self, name: &str, device: &str, mount: &Path) {
        self.leave_volume(mount);
        self.sys.effects.push(system::eject_effect(device, name));
        self.revision += 1;
    }
    /// Leaves the volume mounted at `mount` before it is ejected, when the window shows a folder of it: the window
    /// opens the last folder shown before the volume, by the rule `leave_lost_volumes` uses once the volume is gone,
    /// and the volume's folders leave the history. Only a window that never showed another folder goes home.
    pub(crate) fn leave_volume(&mut self, mount: &Path) {
        if !self.path.starts_with(mount) {
            return;
        }
        self.at = forget_lost(&mut self.history, self.at, &|p: &Path| p.starts_with(mount));
        match self.history.get(self.at).cloned() {
            // the entry is in the history already
            Some(before) => self.navigate(before, false),
            None => {
                let home = self.home.clone();
                self.navigate(home, true);
            }
        }
    }
    /// How far the source list can scroll: its rows past the bottom of the column (`SIDEBAR_SCROLL` is the header band).
    pub(crate) fn side_scroll_max(&self) -> f32 {
        (self.sidebar().content_height() - SIDEBAR_SCROLL - self.sidebar_bounds().h).max(0.)
    }
    /// The scroll the source list draws and hits with: the header band plus the user's scroll, kept within the list.
    pub(crate) fn sidebar_scroll(&self) -> f32 {
        SIDEBAR_SCROLL + self.side_scroll.clamp(0., self.side_scroll_max())
    }
    pub(crate) fn sidebar_id(&self, path: &Path) -> Option<u32> {
        self.places().iter().position(|p| p.path == path).map(|i| 100 + i as u32)
    }
    pub(crate) fn sidebar_path(&self, id: u32) -> Option<PathBuf> {
        self.places().into_iter().nth(id.checked_sub(100)? as usize).map(|p| p.path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::fixture;

    #[test]
    fn a_history_forgets_an_ejected_volume_and_keeps_its_place() {
        let lost = |p: &Path| p.starts_with("/run/media/me/V");
        let paths = |list: &[&str]| list.iter().map(PathBuf::from).collect::<Vec<_>>();
        // on the volume, with a folder before it and one after: the place is the folder before it
        let mut h = paths(&["/home/me", "/run/media/me/V", "/run/media/me/V/a", "/tmp"]);
        assert_eq!(forget_lost(&mut h, 2, &lost), 0);
        assert_eq!(h, paths(&["/home/me", "/tmp"]));
        // nothing before the volume: the first folder that is kept
        let mut h = paths(&["/run/media/me/V", "/tmp"]);
        assert_eq!(forget_lost(&mut h, 0, &lost), 0);
        assert_eq!(h, paths(&["/tmp"]));
        // home, the volume, home again (Eject moved the window home): one entry, not two of the same
        let mut h = paths(&["/home/me", "/run/media/me/V", "/home/me", "/tmp"]);
        assert_eq!(forget_lost(&mut h, 3, &lost), 1);
        assert_eq!(h, paths(&["/home/me", "/tmp"]));
        // only the volume: nothing is left, and the caller opens the home folder
        let mut h = paths(&["/run/media/me/V"]);
        assert_eq!(forget_lost(&mut h, 0, &lost), 0);
        assert!(h.is_empty());
        // a history that never saw the volume is not touched, repeats included
        let mut h = paths(&["/home/me", "/home/me", "/tmp"]);
        assert_eq!(forget_lost(&mut h, 2, &lost), 2);
        assert_eq!(h.len(), 3);
    }

    #[test]
    fn an_eject_goes_back_to_the_folder_shown_before_the_volume() {
        let root = std::env::temp_dir().join(format!("files-eject-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for d in ["home", "home/Docs", "vol/a"] {
            std::fs::create_dir_all(root.join(d)).unwrap();
        }
        let (home, docs, vol) = (root.join("home"), root.join("home/Docs"), root.join("vol"));
        let mut files = fixture(vec![]);
        files.home = home.clone();
        for p in [docs.clone(), vol.clone(), vol.join("a")] {
            files.navigate(p, true);
        }
        files.leave_volume(&vol);
        assert_eq!(files.path, docs, "the last folder shown before the volume, not the home folder");
        assert_eq!((files.history.clone(), files.at), (vec![docs.clone()], 0), "no entry of the volume, none added");
        // a window on another folder is left alone
        files.leave_volume(&vol);
        assert_eq!((files.path.clone(), files.history.len()), (docs.clone(), 1));
        // a window that only ever showed the volume goes home
        let mut files = fixture(vec![]);
        files.home = home.clone();
        files.navigate(vol.clone(), true);
        files.leave_volume(&vol);
        assert_eq!((files.path.clone(), files.history.clone(), files.at), (home.clone(), vec![home.clone()], 0));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn folders_outside_the_media_folders_are_never_a_lost_volume() {
        assert!(!lost_volume(Path::new("/fixture/gone")));
        assert!(!lost_volume(Path::new("/home/me/removed")));
        // a window with no volume in sight changes nothing
        let mut files = fixture(vec![]);
        let before = (files.path.clone(), files.history.clone(), files.at, files.revision);
        assert!(!files.leave_lost_volumes());
        assert_eq!((files.path.clone(), files.history.clone(), files.at, files.revision), before);
    }

    #[test]
    fn locations_glyphs_are_grey_while_favorites_recents_and_tags_keep_their_look() {
        let files = fixture(vec![]);
        let list = files.sidebar();
        let neutral_of = |label: &str| {
            list.items.iter().find_map(|i| match i {
                SidebarItem::Row { label: l, neutral, .. } if l == label => Some(*neutral),
                _ => None,
            })
        };
        assert_eq!(neutral_of("Recents"), Some(false));
        assert_eq!(neutral_of("Desktop"), Some(false));
        assert_eq!(neutral_of("fixture"), Some(true), "the home row is named after the folder");
        assert_eq!(neutral_of("Computer"), Some(true));
        assert_eq!(neutral_of("Trash"), Some(true));
        assert_eq!(neutral_of("Red"), Some(false), "a tag keeps its colour dot");
    }

    #[test]
    fn a_mounted_volume_row_has_a_drawn_glyph_and_an_eject_mark_that_leaves_its_name_the_width() {
        use bifrost_app::controls::sidebar::{label_room, TRAILING_GLYPH_PT};
        let volume = Place { label: "TESTVOL".into(), symbol: VOLUME_SYMBOL, dot: None, path: PathBuf::from("/run/media/me/TESTVOL"), eject: Some("/dev/loop0".into()) };
        let SidebarItem::Row { label, symbol, trailing, trailing_symbol, neutral, .. } = place_row(107, &volume, Some("Locations")) else { panic!("a row") };
        assert_eq!(label, "TESTVOL");
        assert_eq!(symbol.as_deref(), Some(VOLUME_SYMBOL));
        assert!(neutral, "grey like the other Locations rows");
        assert_eq!(trailing, None, "no word at the end of the row");
        assert_eq!(trailing_symbol.as_deref(), Some(EJECT_SYMBOL));
        // both names have a drawing: a name the glyph set does not know draws nothing and leaves the row without an icon
        assert!(bifrost_app::glyphs::has(VOLUME_SYMBOL));
        assert!(bifrost_app::glyphs::has(EJECT_SYMBOL));
        // the row's geometry in the 160 pt column: the label starts after the 20 pt glyph and ends at the mark
        let (text_x, right, gap) = (10. + 8. + 20. + 8., SIDEBAR_W - 10. - 8., 8.);
        let room = label_room(text_x, right, &[TRAILING_GLYPH_PT], gap);
        assert_eq!(room, 74.);
        assert!(room >= label_room(text_x, right, &[32.], gap) + 18., "more than beside a word");
        // the click zone covers the mark and stays clear of the label
        assert!(SIDEBAR_W - EJECT_W <= right - TRAILING_GLYPH_PT, "the mark is inside the eject zone");
        assert!(SIDEBAR_W - EJECT_W >= text_x + room, "the label is outside it");
    }

    #[test]
    fn every_sidebar_place_has_a_glyph_the_set_draws_and_only_volumes_eject() {
        let files = fixture(vec![]);
        for (header, rows) in files.sections() {
            for p in &rows {
                assert!(p.dot.is_some() || bifrost_app::glyphs::has(p.symbol), "{} has no drawing for {}", p.label, p.symbol);
                let marked = matches!(place_row(100, p, header), SidebarItem::Row { trailing_symbol: Some(_), .. });
                assert_eq!(marked, p.eject.is_some(), "{}: only a row that ejects has the mark", p.label);
            }
        }
        let provider = Place { label: "Cloud".into(), symbol: VOLUME_SYMBOL, dot: None, path: PathBuf::from("/provider"), eject: None };
        assert!(matches!(place_row(100, &provider, Some("Locations")), SidebarItem::Row { trailing_symbol: None, trailing: None, .. }));
    }

    #[test]
    fn a_short_window_scrolls_the_sidebar_down_to_the_last_tag() {
        let mut files = fixture(vec![]);
        files.h = 420.;
        let gray = files.sidebar_id(&tag_path(Tag::Gray)).expect("the Gray tag row");
        // the bottom edge of the Gray row under the current scroll
        let bottom = |files: &Files| {
            let list = files.sidebar();
            let index = list.items.iter().position(|i| matches!(i, SidebarItem::Row { id, .. } if *id == gray)).unwrap();
            let r = list.item_rect(index, files.sidebar_bounds(), files.sidebar_scroll());
            r.y + r.h
        };
        assert!(bottom(&files) > files.h, "the last tag is cut off at this height");
        // an offset past the end is held at the end of the list
        files.side_scroll = f32::MAX;
        assert!(bottom(&files) <= files.h, "scrolled to the end, the last tag is in view");
    }

    #[test]
    fn the_applications_row_opens_the_applications_folder_when_the_home_has_one() {
        let dir = std::env::temp_dir().join(format!("files-apps-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("Applications")).unwrap();
        let mut files = fixture(vec![]);
        files.home = dir.clone();
        assert_eq!(files.applications_dir(), Some(dir.join("Applications")));
        let favorites = files.sections().into_iter().find(|(header, _)| *header == Some("Favorites")).map(|(_, rows)| rows).unwrap();
        assert_eq!(favorites[0].label, "Applications");
        assert_eq!(favorites[0].path, dir.join("Applications"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
