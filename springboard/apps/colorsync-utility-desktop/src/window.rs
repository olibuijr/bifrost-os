//! The state and the geometry of Colour Utility: the list of profile files, the selected profile with its parse and its
//! checks, the panel shown, and the scroll of the list and of the panel. Layout is in points; the drawing and the hit
//! tests read the same rectangles, so a tap lands on what is drawn.

use crate::library::{self, Found};
use crate::model::{Gamut, Issue, Profile};
use bifrost_app::controls::sidebar::{SidebarItem, SourceList};
use bifrost_app::controls::Rect;
use bifrost_app::kit::TextCache;
use bifrost_app::layout::UNIFIED_BAR_PT;
use std::path::{Path, PathBuf};

/// Width of the profile list, in points.
pub const SIDE_PT: f32 = 248.0;
/// Height of one profile row in the list, in points.
pub const ROW_PT: f32 = 44.0;
/// Space above the first row of the list.
const LIST_TOP_PT: f32 = 8.0;
/// Margin of the detail column.
pub const PAD_PT: f32 = 16.0;
/// Distance from the top of the unified bar to the panel tabs, and from it to the panel body.
const TABS_DY_PT: f32 = 66.0;
const BODY_DY_PT: f32 = 104.0;
/// Size and gap of the panel tabs.
pub const TAB_W_PT: f32 = 104.0;
pub const TAB_H_PT: f32 = 26.0;
const TAB_GAP_PT: f32 = 6.0;
/// Height of one line of the header, tag and verification lists.
pub const LINE_PT: f32 = 22.0;
/// Width of the key column of the header and tag lists.
pub const KEY_PT: f32 = 170.0;
/// Largest file the window reads, in bytes. A profile is normally under a megabyte; a larger file is refused.
pub const MAX_BYTES: u64 = 64 * 1024 * 1024;

/// The panels of the detail column.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Panel {
    Header,
    Tags,
    Verify,
    Gamut,
}

impl Panel {
    pub const ALL: [Panel; 4] = [Panel::Header, Panel::Tags, Panel::Verify, Panel::Gamut];

    pub fn title(self) -> &'static str {
        match self {
            Panel::Header => "Header",
            Panel::Tags => "Tags",
            Panel::Verify => "Verify",
            Panel::Gamut => "Gamut",
        }
    }

    pub fn index(self) -> usize {
        Panel::ALL.iter().position(|p| *p == self).unwrap_or(0)
    }
}

/// What a point on the window belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    Nothing,
    /// A profile row of the list, by its index in `found`.
    Row(usize),
    /// A panel tab.
    Tab(Panel),
    /// The list below or beside the rows; a drag scrolls the list.
    Side,
    /// The panel body; a drag scrolls the panel.
    Body,
}

/// The selected profile, read and checked.
pub struct Current {
    /// The file the profile was read from.
    pub path: PathBuf,
    /// The profile description, or the file name when the profile has none.
    pub title: String,
    /// The parsed profile; None when the file could not be read or has no header.
    pub profile: Option<Profile>,
    /// Why the file could not be read, when it could not.
    pub error: Option<String>,
    /// The findings of `Profile::verify`.
    pub issues: Vec<Issue>,
    /// The gamut of an RGB matrix profile.
    pub gamut: Option<Gamut>,
}

pub struct ColourUtility {
    /// The home folder the user's profile folders are under.
    pub home: PathBuf,
    /// Every profile file found, system folders first.
    pub found: Vec<Found>,
    /// The index in `found` of the selected profile.
    pub selected: Option<usize>,
    /// The selected profile, read and checked.
    pub current: Option<Current>,
    pub panel: Panel,
    /// How far the list is scrolled, in points.
    pub side_scroll: f32,
    /// How far the panel body is scrolled, in points.
    pub scroll: f32,
    /// The line under the toolbar: the count of profiles, or a failure.
    pub status: String,
    /// The window size in points.
    pub size: (f32, f32),
    pub labels: TextCache,
    /// The target the press began on.
    pub press: Target,
    /// The last pointer position of a drag, and whether the drag has moved.
    pub drag_y: Option<f32>,
    pub dragged: bool,
    /// False until the profile folders have been searched the first time.
    pub loaded: bool,
}

impl ColourUtility {
    pub fn new() -> ColourUtility {
        let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
        ColourUtility::with_home(home)
    }

    pub fn with_home(home: PathBuf) -> ColourUtility {
        ColourUtility {
            home,
            found: Vec::new(),
            selected: None,
            current: None,
            panel: Panel::Header,
            side_scroll: 0.0,
            scroll: 0.0,
            status: String::new(),
            size: (0.0, 0.0),
            labels: TextCache::default(),
            press: Target::Nothing,
            drag_y: None,
            dragged: false,
            loaded: false,
        }
    }

    /// Looks for the profile files again. The selection stays on the same file when it is still there.
    pub fn reload(&mut self) {
        let previous = self.selected.and_then(|i| self.found.get(i)).map(|f| f.path.clone());
        self.found = library::scan_all(&self.home);
        self.selected = previous.and_then(|p| self.found.iter().position(|f| f.path == p));
        self.side_scroll = self.side_scroll.clamp(0.0, self.max_side_scroll());
        self.status = match self.found.len() {
            0 => "No colour profiles were found in the system or user folders.".to_string(),
            1 => "1 colour profile.".to_string(),
            n => format!("{n} colour profiles."),
        };
        self.load_selected();
    }

    /// Selects the profile at `index`. True when the selection changed.
    pub fn select(&mut self, index: usize) -> bool {
        if index >= self.found.len() {
            return false;
        }
        self.selected = Some(index);
        self.scroll = 0.0;
        self.load_selected();
        true
    }

    /// Moves the selection by `step` rows, stopping at the ends of the list.
    pub fn step(&mut self, step: i32) -> bool {
        if self.found.is_empty() {
            return false;
        }
        let last = self.found.len() as i32 - 1;
        let from = self.selected.map_or(-1, |i| i as i32);
        let to = (from + step).clamp(0, last) as usize;
        if Some(to) == self.selected {
            return false;
        }
        self.select(to)
    }

    /// Shows the panel `panel`. True when it changed.
    pub fn show(&mut self, panel: Panel) -> bool {
        if self.panel == panel {
            return false;
        }
        self.panel = panel;
        self.scroll = 0.0;
        true
    }

    /// Reads and checks the selected file, or clears the detail when nothing is selected.
    fn load_selected(&mut self) {
        let Some(found) = self.selected.and_then(|i| self.found.get(i)) else {
            self.current = None;
            return;
        };
        let path = found.path.clone();
        let fallback = path.file_stem().and_then(|s| s.to_str()).unwrap_or("Profile").to_string();
        self.current = Some(match read_bounded(&path).and_then(Profile::parse) {
            Ok(profile) => Current {
                title: profile.title().unwrap_or(fallback),
                issues: profile.verify(),
                gamut: profile.gamut(),
                path,
                profile: Some(profile),
                error: None,
            },
            Err(error) => Current {
                title: fallback,
                issues: Vec::new(),
                gamut: None,
                path,
                profile: None,
                error: Some(error),
            },
        });
    }

    pub fn content_x(&self) -> f32 {
        SIDE_PT + PAD_PT
    }

    pub fn content_w(&self) -> f32 {
        (self.size.0 - SIDE_PT - 2.0 * PAD_PT).max(0.0)
    }

    pub fn title_y(&self) -> f32 {
        UNIFIED_BAR_PT + 12.0
    }

    pub fn sidebar_rect(&self) -> Rect {
        Rect { x: 0.0, y: UNIFIED_BAR_PT, w: SIDE_PT, h: (self.size.1 - UNIFIED_BAR_PT).max(0.0) }
    }

    pub fn sidebar(&self) -> SourceList {
        let mut items = vec![SidebarItem::header("")];
        for (index, found) in self.found.iter().enumerate() {
            let name = found.path.file_name().and_then(|n| n.to_str()).unwrap_or("profile");
            items.push(SidebarItem::row(index as u32 + 1, name).with_symbol("folder").with_subtitle(found.group.title()));
        }
        let mut list = SourceList::new(items, ROW_PT, LIST_TOP_PT);
        list.two_line_h = ROW_PT;
        list.set_selected(self.selected.map(|i| i as u32 + 1));
        list
    }

    /// The rectangle of profile row `index`, moved by the list's scroll.
    pub fn row_rect(&self, index: usize) -> Rect {
        Rect { x: 0.0, y: UNIFIED_BAR_PT + LIST_TOP_PT + index as f32 * ROW_PT - self.side_scroll, w: SIDE_PT, h: ROW_PT }
    }

    pub fn tab_rect(&self, panel: Panel) -> Rect {
        Rect {
            x: self.content_x() + panel.index() as f32 * (TAB_W_PT + TAB_GAP_PT),
            y: UNIFIED_BAR_PT + TABS_DY_PT,
            w: TAB_W_PT,
            h: TAB_H_PT,
        }
    }

    pub fn body_rect(&self) -> Rect {
        let top = UNIFIED_BAR_PT + BODY_DY_PT;
        Rect { x: self.content_x(), y: top, w: self.content_w(), h: (self.size.1 - top - PAD_PT).max(0.0) }
    }

    /// The target under the point (x, y), in points.
    pub fn hit(&self, x: f32, y: f32) -> Target {
        if y < UNIFIED_BAR_PT {
            return Target::Nothing;
        }
        if x < SIDE_PT {
            let bounds = self.sidebar_rect();
            if let Some(id) = self.sidebar().hit(bounds, self.side_scroll, x, y) {
                return Target::Row(id.saturating_sub(1) as usize);
            }
            return Target::Side;
        }
        for panel in Panel::ALL {
            if contains(self.tab_rect(panel), x, y) {
                return Target::Tab(panel);
            }
        }
        if contains(self.body_rect(), x, y) {
            return Target::Body;
        }
        Target::Nothing
    }

    /// Scrolls the list by `d` points. True when the scroll moved.
    pub fn scroll_side_by(&mut self, d: f32) -> bool {
        let next = (self.side_scroll + d).clamp(0.0, self.max_side_scroll());
        let moved = next != self.side_scroll;
        self.side_scroll = next;
        moved
    }

    /// Scrolls the panel body by `d` points. True when the scroll moved.
    pub fn scroll_body_by(&mut self, d: f32) -> bool {
        let next = (self.scroll + d).clamp(0.0, self.max_body_scroll());
        let moved = next != self.scroll;
        self.scroll = next;
        moved
    }

    fn max_side_scroll(&self) -> f32 {
        let list = LIST_TOP_PT + self.found.len() as f32 * ROW_PT;
        (list - self.sidebar_rect().h).max(0.0)
    }

    /// The lines the panel body holds, for the scroll limit.
    pub fn body_lines(&self) -> usize {
        let Some(c) = &self.current else { return 1 };
        match self.panel {
            Panel::Header => c.profile.as_ref().map_or(1, |p| crate::model::header_rows(p).len()),
            Panel::Tags => c.profile.as_ref().map_or(1, |p| p.tags.len() + 1),
            Panel::Verify => c.issues.len().max(1) + 1,
            Panel::Gamut => 0,
        }
    }

    fn max_body_scroll(&self) -> f32 {
        let lines = self.body_lines() as f32 * LINE_PT;
        (lines - self.body_rect().h).max(0.0)
    }
}

impl Default for ColourUtility {
    fn default() -> Self {
        ColourUtility::new()
    }
}

/// True when the point is inside the rectangle.
pub fn contains(r: Rect, x: f32, y: f32) -> bool {
    x >= r.x && x < r.x + r.w && y >= r.y && y < r.y + r.h
}

/// Reads a file, refusing one larger than `MAX_BYTES`.
pub fn read_bounded(path: &Path) -> Result<Vec<u8>, String> {
    let len = std::fs::metadata(path).map_err(|e| e.to_string())?.len();
    if len > MAX_BYTES {
        return Err(format!("the file is {} MB; the limit is {} MB", len >> 20, MAX_BYTES >> 20));
    }
    std::fs::read(path).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::Group;

    fn window_with(n: usize) -> ColourUtility {
        let mut w = ColourUtility::with_home(PathBuf::from("/nonexistent-home"));
        w.size = (1000.0, 700.0);
        w.found = (0..n)
            .map(|i| Found { path: PathBuf::from(format!("/nowhere/p{i}.icc")), group: Group::System })
            .collect();
        w
    }

    #[test]
    fn rows_and_tabs_are_hit() {
        let w = window_with(3);
        let r1 = w.row_rect(1);
        assert_eq!(w.hit(r1.x + 10.0, r1.y + 5.0), Target::Row(1));
        let tab = w.tab_rect(Panel::Gamut);
        assert_eq!(w.hit(tab.x + 2.0, tab.y + 2.0), Target::Tab(Panel::Gamut));
        assert_eq!(w.hit(w.body_rect().x + 4.0, w.body_rect().y + 4.0), Target::Body);
        assert_eq!(w.hit(10.0, 4.0), Target::Nothing);
    }

    #[test]
    fn blank_list_area_is_side() {
        let w = window_with(1);
        assert_eq!(w.hit(10.0, 600.0), Target::Side);
    }

    #[test]
    fn list_scroll_is_limited() {
        let mut w = window_with(40);
        assert!(w.scroll_side_by(100000.0));
        assert_eq!(w.side_scroll, w.max_side_scroll());
        assert!(w.scroll_side_by(-100000.0));
        assert_eq!(w.side_scroll, 0.0);
        assert!(!w.scroll_side_by(-5.0));
    }

    #[test]
    fn step_stops_at_the_ends() {
        let mut w = window_with(2);
        assert!(w.step(1));
        assert_eq!(w.selected, Some(0));
        assert!(!w.step(-1));
    }

    #[test]
    fn selecting_an_unreadable_file_reports_it() {
        let mut w = window_with(1);
        assert!(w.select(0));
        let c = w.current.as_ref().expect("a current profile");
        assert!(c.profile.is_none());
        assert!(c.error.is_some());
        assert_eq!(c.title, "p0");
    }

    #[test]
    fn panels_switch_once() {
        let mut w = window_with(0);
        assert!(w.show(Panel::Verify));
        assert!(!w.show(Panel::Verify));
        assert_eq!(w.panel.title(), "Verify");
    }

    #[test]
    fn read_refuses_a_missing_file() {
        assert!(read_bounded(Path::new("/nonexistent/none.icc")).is_err());
    }
}
