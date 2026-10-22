//! The source list: Favorites, Locations and Tags sections (Files's sidebar).
use crate::*;
use std::sync::Mutex;

/// Why the last eject failed, until the app loop shows it (a worker thread leaves it here).
static EJECT_NOTICE: Mutex<Option<String>> = Mutex::new(None);

fn post_eject_notice(notice: String) {
    if let Ok(mut slot) = EJECT_NOTICE.lock() {
        *slot = Some(notice);
    }
}

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
    /// The partition a removable volume row ejects (`/dev/sdb1`); None for every other row.
    pub(crate) eject: Option<String>,
}

/// Width of the row's right end where the Eject text is (points): a click there ejects the volume, not opens it.
const EJECT_W: f32 = 56.;

impl Files {
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
        // removable volumes mounted now follow Trash, each with an Eject text; under the test pin none are listed, so a
        // snapshot does not depend on the disks plugged into the machine that runs it
        let mounted = if bifrost_app::testing::pinned() { Vec::new() } else { springboard::files::volumes::mounted() };
        let volumes = mounted.into_iter().map(|v| Place { label: v.name, symbol: "externaldrive", dot: None, path: v.mount, eject: Some(v.device) });
        // file provider domains (installed apps that serve one) follow the volumes
        let providers = provider::shared().rows().into_iter().map(|(label, path)| Place { label, symbol: "externaldrive", dot: None, path, eject: None });
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
                let row = SidebarItem::row(id, &p.label);
                let row = if p.eject.is_some() { row.with_trailing("Eject") } else { row };
                items.push(match p.dot {
                    Some(c) => row.with_dot(c),
                    // Favorites and the top rows take the accent; Locations (home, computer, trash, volumes) the secondary grey
                    None if header == Some("Locations") => row.with_symbol(p.symbol).neutral(),
                    None => row.with_symbol(p.symbol),
                });
                id += 1;
            }
        }
        let mut list = SourceList::new(items, 32., 30.);
        list.set_selected(self.sidebar_id(&self.path));
        list
    }
    /// A click in the sidebar: a row opens its place, and the Eject text of a removable volume ejects it.
    pub(crate) fn sidebar_click(&mut self, x: f32, y: f32) {
        let Some(id) = self.sidebar().hit(self.sidebar_bounds(), SIDEBAR_SCROLL, x, y) else { return };
        let Some(place) = self.places().into_iter().nth(id.saturating_sub(100) as usize) else { return };
        match place.eject {
            Some(device) if x > self.side() - EJECT_W => self.eject_volume(&place.label, &device, &place.path),
            _ => self.navigate(place.path, true),
        }
    }
    /// Ejects a removable volume in the background (unmount, then power off). A folder open inside it moves home first.
    /// A failure is logged and shown in the status bar, naming the programs that still hold the volume open
    /// (`volumes::eject_with_notice`); the row goes away on the next sidebar build once the volume is gone.
    fn eject_volume(&mut self, name: &str, device: &str, mount: &Path) {
        if self.path.starts_with(mount) {
            let home = self.home.clone();
            self.navigate(home, true);
        }
        let (name, device, mount) = (name.to_string(), device.to_string(), mount.to_path_buf());
        DispatchQueue::global(Qos::Utility).async_(move || {
            if let Some(notice) = springboard::files::volumes::eject_with_notice(&name, &device, &mount) {
                eprintln!("eject {device}: {notice}");
                post_eject_notice(notice);
                // wake the app loop so `tick` shows it
                DispatchQueue::main().async_(|| {});
            }
        });
        self.revision += 1;
    }
    /// The notice a failed eject left, once: the status bar shows it.
    pub(crate) fn take_eject_notice(&mut self) -> bool {
        let Some(notice) = EJECT_NOTICE.lock().ok().and_then(|mut n| n.take()) else { return false };
        self.notice = Some(notice);
        true
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
