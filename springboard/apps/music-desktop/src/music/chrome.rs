//! The Golden Gate window chrome's model: the source list (Library, Playlists) and the unified toolbar
//! (back, title, sort, search, playlist actions, Up Next). The window draws them with the SDK kit
//! (`widgets::source_list`, `widgets::toolbar`) beside and above the content pane `Music` draws.

use super::browse::Scope;
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
/// Edit Rules on a smart playlist, and History on the Up Next page.
pub const BAR_EDIT: u32 = 8;
pub const BAR_HISTORY: u32 = 9;

/// Sidebar row ids: the Library tabs, the lists under Songs (Recently Added, Recently Played, Favourite Songs), then one
/// row per playlist and one per smart playlist.
pub const SIDE_TAB: u32 = 100;
pub const SIDE_SCOPE: u32 = 150;
pub const SIDE_PLAYLIST: u32 = 200;
pub const SIDE_SMART: u32 = 1000;

/// The lists under Songs in the source list, with their symbols: every `Scope` but the whole library.
const SCOPES: [(Scope, &str); 3] = [(Scope::RecentlyAdded, "clock"), (Scope::RecentlyPlayed, "arrow.clockwise"), (Scope::Favourites, "star")];

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
                Tab::Songs => self.scope.label().into(),
                t => TABS.iter().find(|e| e.0 == t).map_or("", |e| e.1).into(),
            },
            Page::Artist(a) => self.library.artists[a].name.clone(),
            Page::Album(a) => self.library.albums[a].name.clone(),
            Page::Queue => "Up Next".into(),
            Page::History => "History".into(),
            Page::Playlist(p) => match &self.renaming {
                Some(typed) => format!("{typed}|"),
                None => self.playlists.lists().get(p).map_or(String::new(), |l| l.name.clone()),
            },
            Page::Smart(p) => match &self.renaming {
                Some(typed) => format!("{typed}|"),
                None => self.smart.lists().get(p).map_or(String::new(), |l| l.name.clone()),
            },
            Page::SmartEdit(p) => self.smart.lists().get(p).map_or(String::new(), |l| format!("{} Rules", l.name)),
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
        if let Page::Smart(_) = self.page() {
            items.push(ToolbarItem::button(BAR_EDIT, "Edit Rules"));
        }
        if let Page::Playlist(_) | Page::Smart(_) = self.page() {
            items.push(ToolbarItem::button(BAR_RENAME, if self.renaming.is_some() { "Done" } else { "Rename" }));
            items.push(ToolbarItem::icon(BAR_DELETE, "Delete Playlist", "trash"));
        }
        if self.page() == Page::Root && self.tab == Tab::Songs {
            items.push(ToolbarItem::icon(BAR_SORT, &format!("Sort By {}", self.sort.label()), "arrow.up.arrow.down"));
        }
        if self.page() == Page::Queue {
            items.push(ToolbarItem::icon(BAR_HISTORY, "History", "clock"));
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
        self.commit_rule_edit();
        let target = match id {
            BAR_BACK => Target::Back,
            BAR_SORT => Target::Sort,
            BAR_EDIT => Target::EditRules,
            BAR_HISTORY => {
                self.pages.push(Page::History);
                self.scroll.jump_top();
                self.dirty = true;
                return;
            }
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

    /// Esc: closes the search field, stops renaming or drops a rule value being typed.
    pub fn cancel(&mut self, t: f64) -> bool {
        if self.rule_edit.take().is_some() {
            self.dirty = true;
            return true;
        }
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
            if *tab == Tab::Songs {
                for (k, (scope, symbol)) in SCOPES.iter().enumerate() {
                    items.push(SidebarItem::row(SIDE_SCOPE + k as u32, scope.label()).with_symbol(symbol));
                }
            }
        }
        for (p, list) in self.playlists.lists().iter().enumerate() {
            items.push(SidebarItem::row(SIDE_PLAYLIST + p as u32, &list.name).with_symbol("music.note"));
        }
        for (p, list) in self.smart.lists().iter().enumerate() {
            items.push(SidebarItem::row(SIDE_SMART + p as u32, &list.name).with_symbol("music.note.list"));
        }
        let mut list = SourceList::new(items, 32., 30.);
        let on_scope = self.page() == Page::Root && self.tab == Tab::Songs;
        let selected = match self.page() {
            Page::Queue | Page::History => None,
            Page::Playlist(p) => Some(SIDE_PLAYLIST + p as u32),
            Page::Smart(p) | Page::SmartEdit(p) => Some(SIDE_SMART + p as u32),
            _ if on_scope && self.scope != Scope::All => SCOPES.iter().position(|e| e.0 == self.scope).map(|k| SIDE_SCOPE + k as u32),
            _ => TABS.iter().position(|e| e.0 == self.tab).map(|i| SIDE_TAB + i as u32),
        };
        list.set_selected(selected);
        list
    }

    /// A sidebar row was clicked.
    pub fn pick_sidebar(&mut self, id: u32, t: f64) {
        if (SIDE_TAB..SIDE_TAB + TABS.len() as u32).contains(&id) {
            self.activate(Target::Seg((id - SIDE_TAB) as usize), t);
        } else if let Some((scope, _)) = id.checked_sub(SIDE_SCOPE).and_then(|k| SCOPES.get(k as usize)) {
            // Seg(2) is the Songs tab; the scope is set after it, which leaves the whole library
            self.activate(Target::Seg(2), t);
            self.set_scope(*scope);
        } else if let Some(p) = id.checked_sub(SIDE_SMART).map(|p| p as usize).filter(|&p| p < self.smart.lists().len()) {
            self.activate(Target::Seg(4), t);
            self.pages.push(Page::Smart(p));
            self.scroll.jump_top();
        } else if let Some(p) = id.checked_sub(SIDE_PLAYLIST).map(|p| p as usize).filter(|&p| p < self.playlists.lists().len()) {
            self.activate(Target::Seg(4), t);
            self.pages.push(Page::Playlist(p));
            self.scroll.jump_top();
        }
    }
}
