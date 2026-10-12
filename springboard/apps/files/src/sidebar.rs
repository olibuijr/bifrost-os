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
}

impl Files {
    pub(crate) fn sidebar_bounds(&self) -> Rect {
        Rect { x: 0., y: 0., w: self.side(), h: self.h }
    }
    /// The home folder's name in the sidebar and title (Files: the user's short name).
    pub(crate) fn home_name(&self) -> String {
        self.home.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or("Home".into())
    }
    /// The sidebar's sections in Files's order: Recents without a header, Favorites, Locations, Tags.
    pub(crate) fn sections(&self) -> Vec<(Option<&'static str>, Vec<Place>)> {
        let place = |label: &str, symbol: &'static str, path: PathBuf| Place { label: label.into(), symbol, dot: None, path };
        let mut favorites = vec![];
        if let Some(apps) = [self.home.join("Applications"), PathBuf::from("/Applications")].into_iter().find(|p| p.is_dir()) {
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
        let tags = Tag::ALL.into_iter().map(|t| Place { label: t.label().into(), symbol: "tag", dot: Some(tag_colour(t)), path: tag_path(t) }).collect();
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
        let rows = self.smart.iter().enumerate().map(|(i, f)| Place { label: f.name.clone(), symbol: "magnifyingglass", dot: None, path: smart::smart_path(i) }).collect();
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
                items.push(if let Some(c) = p.dot { row.with_dot(c) } else { row.with_symbol(p.symbol) });
                id += 1;
            }
        }
        let mut list = SourceList::new(items, 32., 30.);
        list.set_selected(self.sidebar_id(&self.path));
        list
    }
    pub(crate) fn sidebar_id(&self, path: &Path) -> Option<u32> {
        self.places().iter().position(|p| p.path == path).map(|i| 100 + i as u32)
    }
    pub(crate) fn sidebar_path(&self, id: u32) -> Option<PathBuf> {
        self.places().into_iter().nth(id.checked_sub(100)? as usize).map(|p| p.path)
    }
}
