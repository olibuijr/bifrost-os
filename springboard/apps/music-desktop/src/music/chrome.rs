//! The Golden Gate window chrome's model: the source list (Library, Playlists) and the unified toolbar
//! (back, title, sort, search, playlist actions, Up Next). The window draws them with the SDK kit
//! (`widgets::source_list`, `widgets::toolbar`) beside and above the content pane `Music` draws.

use super::{Music, Page, Tab, Target};
use bifrost_app::controls::nav::{Toolbar, ToolbarItem};
use bifrost_app::controls::sidebar::{SidebarItem, SourceList};

/// Toolbar item ids; they are also the control ids the shell sees (and does not drag the window over).
pub const BAR_BACK: u32 = 1;
pub const BAR_TITLE: u32 = 2;
pub const BAR_SORT: u32 = 3;
pub const BAR_SEARCH: u32 = 4;
pub const BAR_RENAME: u32 = 5;
pub const BAR_DELETE: u32 = 6;
pub const BAR_QUEUE: u32 = 7;

/// Sidebar row ids: the Library tabs, then one row per playlist.
pub const SIDE_TAB: u32 = 100;
pub const SIDE_PLAYLIST: u32 = 200;

const TABS: [(Tab, &str, &str); 5] = [
    (Tab::Artists, "Artists", "music.note"),
    (Tab::Albums, "Albums", "square.grid.2x2"),
    (Tab::Songs, "Songs", "list.bullet"),
    (Tab::Radio, "Radio", "globe"),
    (Tab::Playlists, "All Playlists", "rectangle.split.3x1"),
];

impl Music {
    /// What the toolbar title says: the shown list, artist, album or playlist.
    pub fn title(&self) -> String {
        match self.page() {
            Page::Root => match self.tab {
                Tab::Radio if self.radio.error.is_some() && !self.radio.stations().is_empty() => "Radio (Offline)".into(),
                Tab::Playlists => "Playlists".into(),
                t => TABS.iter().find(|e| e.0 == t).map_or("", |e| e.1).into(),
            },
            Page::Artist(a) => self.library.artists[a].name.clone(),
            Page::Album(a) => self.library.albums[a].name.clone(),
            Page::Queue => "Up Next".into(),
            Page::Playlist(p) => match &self.renaming {
                Some(typed) => format!("{typed}|"),
                None => self.playlists.lists().get(p).map_or(String::new(), |l| l.name.clone()),
            },
        }
    }

    pub fn searching(&self) -> bool {
        self.searching
    }

    pub fn search_text(&self) -> &str {
        &self.search
    }

    /// The Now Playing sheet or the song menu covers the window.
    pub fn overlay(&self) -> bool {
        self.np || self.np_anim.is_some() || self.menu.is_some()
    }

    /// The unified toolbar: back (inside a list), the title, then what the shown list offers.
    pub fn toolbar(&self, title_w: f32, search_w: f32) -> Toolbar {
        let mut items = vec![];
        if self.pages.len() > 1 {
            items.push(ToolbarItem::icon(BAR_BACK, "Back", "chevron.left"));
        }
        items.push(ToolbarItem::custom(BAR_TITLE, "Title", title_w));
        items.push(ToolbarItem::flexible_space());
        if let Page::Playlist(_) = self.page() {
            items.push(ToolbarItem::button(BAR_RENAME, if self.renaming.is_some() { "Done" } else { "Rename" }));
            items.push(ToolbarItem::icon(BAR_DELETE, "Delete Playlist", "trash"));
        }
        if self.page() == Page::Root && self.tab == Tab::Songs {
            items.push(ToolbarItem::icon(BAR_SORT, &format!("Sort By {}", self.sort.label()), "arrow.up.arrow.down"));
        }
        if !self.queue.items().is_empty() {
            items.push(ToolbarItem::icon(BAR_QUEUE, "Up Next", "list.bullet").selected(self.page() == Page::Queue));
        }
        if self.page() == Page::Root && self.tab == Tab::Songs {
            items.push(ToolbarItem::custom(BAR_SEARCH, "Search", search_w));
        }
        Toolbar::new(items)
    }

    /// A toolbar button was pressed.
    pub fn bar_press(&mut self, id: u32, t: f64) {
        let target = match id {
            BAR_BACK => Target::Back,
            BAR_SORT => Target::Sort,
            BAR_SEARCH if !self.searching => Target::Search,
            BAR_RENAME if self.renaming.is_some() => {
                self.type_text("\n");
                return;
            }
            BAR_RENAME => Target::Rename,
            BAR_DELETE => Target::DeletePlaylist,
            BAR_QUEUE if self.page() == Page::Queue => Target::Back,
            BAR_QUEUE => {
                self.pages.push(Page::Queue);
                self.scroll.jump_top();
                self.dirty = true;
                return;
            }
            _ => return,
        };
        self.activate(target, t);
    }

    /// Esc: closes the search field or stops renaming.
    pub fn cancel(&mut self, t: f64) -> bool {
        if self.renaming.take().is_some() {
            self.dirty = true;
            return true;
        }
        if self.searching {
            self.activate(Target::Search, t);
            return true;
        }
        false
    }

    pub fn clear_search(&mut self) {
        self.search.clear();
        self.refilter();
        self.dirty = true;
    }

    /// The source list: the Library, then Playlists with each playlist; the shown one is selected.
    pub fn sidebar(&self) -> SourceList {
        let mut items = vec![SidebarItem::header("Library")];
        for (i, (tab, label, symbol)) in TABS.iter().enumerate() {
            if *tab == Tab::Radio {
                items.push(SidebarItem::header("Radio"));
            }
            if *tab == Tab::Playlists {
                items.push(SidebarItem::header("Playlists"));
            }
            items.push(SidebarItem::row(SIDE_TAB + i as u32, label).with_symbol(symbol));
        }
        for (p, list) in self.playlists.lists().iter().enumerate() {
            items.push(SidebarItem::row(SIDE_PLAYLIST + p as u32, &list.name).with_symbol("music.note"));
        }
        let mut list = SourceList::new(items, 32., 30.);
        let selected = match self.page() {
            Page::Queue => None,
            Page::Playlist(p) => Some(SIDE_PLAYLIST + p as u32),
            _ => TABS.iter().position(|e| e.0 == self.tab).map(|i| SIDE_TAB + i as u32),
        };
        list.set_selected(selected);
        list
    }

    /// A sidebar row was clicked.
    pub fn pick_sidebar(&mut self, id: u32, t: f64) {
        if (SIDE_TAB..SIDE_TAB + TABS.len() as u32).contains(&id) {
            self.activate(Target::Seg((id - SIDE_TAB) as usize), t);
        } else if let Some(p) = id.checked_sub(SIDE_PLAYLIST).map(|p| p as usize).filter(|&p| p < self.playlists.lists().len()) {
            self.activate(Target::Seg(4), t);
            self.pages.push(Page::Playlist(p));
            self.scroll.jump_top();
        }
    }
}
