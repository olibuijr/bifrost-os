//! Photos: the pictures on the PC (our Pictures folder and $HOME/Pictures)
//! as All Photos, Recents, Favorites and Screenshots
//! grids with month headers and a month filter, a full-screen viewer with
//! paging, pinch/double-tap zoom with rubber-band edges and an EXIF info
//! sheet, multi-select, a Recently Deleted album (purged after 30 days), a
//! simple editor (rotate, straighten, crop, filters, blur, vignette, brightness, contrast, exposure, saturation, warmth, revert, saved as a copy) and
//! Share › Set as Wallpaper. Scanning and decoding run on worker threads
//! whose results arrive through a pipe the shell polls, like Deildu.

pub mod albums;
pub mod edit;
pub mod exif;
pub mod grid;
pub mod import;
pub mod library;
pub mod models;
pub mod recipe;
pub mod slideshow;
pub mod stars;
pub mod thumbs;
pub mod viewer;
pub mod zoom;
#[cfg(test)]
mod favorites_tests;
#[cfg(test)]
mod sort_tests;

use crate::layout::in_rect as within;
use super::cache::TextCache;
use super::glyphs;
use super::prefs::Prefs;
use super::clock::tz::unix_now;
use super::ui::{home_zone, Ui, Weight, BLACK, CELL, EMPTY_STATE_GRAY, SECONDARY, SEPARATOR, WHITE};
use super::{Effect, Event};
use crate::deildu::jobs::Pool;
use crate::deildu::scroll::Scroller;
use crate::gfx::{pack, rgba, Image, Surface};
use crate::imgdec;
use crate::layout::Rect;
use crate::scene::STATUS_PT;
use albums::Selection;
use bifrost_ui::photo_caching::{Key as PreKey, Manager as PreheatCache};
use bifrost_ui::photo_smart_albums::SmartAlbum;
use edit::{Aspect, Edit, Filter};
use grid::Grid;
use library::{civil, sections, Photo, Section};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use thumbs::{DiskCache, MemCache};
use slideshow::Slideshow;
use viewer::{Action, Viewer};

pub const BLUE: u32 = pack(10, 132, 255, 255);
pub const RED: u32 = pack(255, 69, 58, 255);
/// The translucent bars over a photo.
const BAR: u32 = pack(0, 0, 0, 200);
const SHEET: u32 = pack(44, 44, 46, 255);
/// Hairline between grid cells, in px.
const GAP_PX: f32 = 2.0;
const NAV_PT: f32 = 44.0;
const HEADER_PT: f32 = 44.0;
const BAR_PT: f32 = 50.0;
const MARGIN_PT: f32 = 16.0;
const EMPTY_BUTTON_W_PT: f32 = 137.0;
const EMPTY_BUTTON_H_PT: f32 = 28.5;
const SHEET_ROW_PT: f32 = 56.0;
const SHEET_W_PT: f32 = 260.0;
/// Columns the zoom buttons step through (fewer = bigger).
const ZOOMS: [usize; 4] = [8, 5, 3, 2];
/// The level the grid starts at (index into `ZOOMS`).
const ZOOM_DEFAULT: usize = 1;
const WORKERS: usize = 2;
/// Screen-fitted pictures kept decoded: the current one and its neighbours,
/// never more than `FULL_MAX` whatever their size (keeps memory use bounded).
const FULL_BUDGET: usize = 4 * 1200 * 1920 * 4;
const FULL_MAX: usize = 3;
const EDIT_ROW_PT: f32 = 48.0;
const INFO_ROW_PT: f32 = 44.0;
const INFO_W_PT: f32 = 420.0;
/// How much of a picture's file the info sheet reads for EXIF.
const EXIF_HEAD: u64 = 256 * 1024;
/// The corner radius of the new controls, in px at most.
fn flat(ui: &Ui) -> f32 {
    ui.pt(4.0).min(6.0)
}

/// Where the thumbnails live: `<data dir>/cache/thumbs`.
pub const THUMB_DIR: &str = "cache/thumbs";
/// Favourites and other settings: `<data dir>/photos.ini`.
pub const PREFS_FILE: &str = "photos.ini";
/// Wallpapers made from photos are `wallpapers/custom-<n>.raw`.
pub const CUSTOM_PREFIX: &str = "custom-";

/// Something a touch can land on.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Target {
    /// the grid with nothing to activate (scrolls)
    Content,
    Cell(usize),
    /// Opens the album picker.
    Album,
    /// the empty library's Open Camera button
    OpenCamera,
    ChooseAlbum(Album),
    /// the menu's New Album row
    NewAlbum,
    /// the viewer's Album button: add to (or remove from) a user album
    AddToAlbum,
    AlbumCancel,
    /// the picker's sort row: flips oldest first and newest first
    SortOrder,
    ZoomIn,
    ZoomOut,
    /// Select All / Deselect All while selecting
    SelectAll,
    Back,
    /// the picture in the viewer
    Photo,
    Share,
    Favourite,
    Delete,
    Recover,
    /// the sheet's action row
    SheetAction,
    /// the share sheet's Send Nearby row
    SheetNearby,
    /// the share sheet's Slideshow, Copy and Save to Files rows
    SheetSlideshow,
    SheetCopy,
    SheetFiles,
    /// the multi-select bar's Slideshow
    SelSlideshow,
    /// a tap on the running slideshow's picture
    ShowTap,
    ShowDone,
    ShowPlay,
    ShowPrev,
    ShowNext,
    SheetCancel,
    /// the month filter (cycles through the months present)
    Filter,
    /// starts or ends multi-select
    Select,
    SelShare,
    SelDelete,
    Info,
    /// a star of the Info sheet's rating row
    Rate(u8),
    /// the Info sheet's Hide Photo / Unhide Photo row
    InfoHide,
    /// the share sheet's Duplicate row
    SheetDuplicate,
    /// the Info sheet's Revert to Original row
    InfoRevert,
    Edit,
    EditCancel,
    EditSave,
    EditRotate,
    EditAspect(Aspect),
    EditFilter(Filter),
    EditRevert,
    EditPane(EditPane),
    /// a button of the Tools page
    EditTool(Tool),
    Slide(Slider),
}

/// The edit mode's tool pages: a tab row picks one, and its sliders fill the rows below.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditPane {
    Adjust,
    Effects,
    Crop,
    /// buttons rather than sliders: rotate left, flips, Auto Enhance and the histogram
    Tools,
}

/// The buttons of the Tools page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    RotateLeft,
    FlipHorizontal,
    FlipVertical,
    AutoEnhance,
}

impl Tool {
    pub fn label(self) -> &'static str {
        match self {
            Tool::RotateLeft => "Rotate Left",
            Tool::FlipHorizontal => "Flip Horizontal",
            Tool::FlipVertical => "Flip Vertical",
            Tool::AutoEnhance => "Auto Enhance",
        }
    }

    /// Does the tool in `edit` show as on (the flips and Auto Enhance toggle).
    pub fn is_on(self, edit: &Edit) -> bool {
        match self {
            Tool::RotateLeft => false,
            Tool::FlipHorizontal => edit.flip_h,
            Tool::FlipVertical => edit.flip_v,
            Tool::AutoEnhance => edit.auto,
        }
    }

    /// Runs the tool on `edit`.
    pub fn apply(self, edit: &mut Edit) {
        match self {
            Tool::RotateLeft => edit.rotate(false),
            Tool::FlipHorizontal => edit.flip_h = !edit.flip_h,
            Tool::FlipVertical => edit.flip_v = !edit.flip_v,
            Tool::AutoEnhance => edit.auto = !edit.auto,
        }
    }
}

impl EditPane {
    pub const ALL: [EditPane; 4] = [EditPane::Adjust, EditPane::Effects, EditPane::Crop, EditPane::Tools];

    pub fn label(self) -> &'static str {
        match self {
            EditPane::Adjust => "Adjust",
            EditPane::Effects => "Effects",
            EditPane::Crop => "Crop",
            EditPane::Tools => "Tools",
        }
    }

    /// The sliders of the page, top to bottom. No page has more than `EDIT_SLIDER_ROWS`.
    pub fn sliders(self) -> &'static [Slider] {
        match self {
            EditPane::Adjust => &[Slider::Brightness, Slider::Contrast, Slider::Exposure, Slider::Saturation, Slider::Warmth],
            EditPane::Effects => &[Slider::Blur, Slider::Vignette],
            EditPane::Crop => &[Slider::Straighten, Slider::Crop],
            EditPane::Tools => &[],
        }
    }
}

/// Rows of sliders under the tab row.
const EDIT_SLIDER_ROWS: usize = 5;

/// The editor panel's height in rows: aspect, filters, tabs, then the sliders.
const EDIT_PANEL_ROWS: f32 = 3.0 + EDIT_SLIDER_ROWS as f32;

/// The edit mode's sliders.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Slider {
    Brightness,
    Contrast,
    Exposure,
    Saturation,
    Warmth,
    Blur,
    Vignette,
    Crop,
    Straighten,
}

impl Slider {
    pub fn label(self) -> &'static str {
        match self {
            Slider::Brightness => "Brightness",
            Slider::Contrast => "Contrast",
            Slider::Exposure => "Exposure",
            Slider::Saturation => "Saturation",
            Slider::Warmth => "Warmth",
            Slider::Blur => "Blur",
            Slider::Vignette => "Vignette",
            Slider::Crop => "Crop",
            Slider::Straighten => "Straighten",
        }
    }

    /// The slider's current value in `edit`.
    pub fn value(self, edit: &Edit) -> f32 {
        match self {
            Slider::Brightness => edit.brightness,
            Slider::Contrast => edit.contrast,
            Slider::Exposure => edit.exposure,
            Slider::Saturation => edit.saturation,
            Slider::Warmth => edit.warmth,
            Slider::Blur => edit.blur,
            Slider::Vignette => edit.vignette,
            Slider::Crop => edit.crop_zoom,
            Slider::Straighten => edit.straighten,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Album {
    All,
    Recents,
    Favorites,
    Screenshots,
    Trash,
    /// photos hidden from All Photos; listed in the picker once there is one
    Hidden,
    /// a user album: index into the album list
    User(u8),
}

/// At most this many user albums.
const MAX_USER_ALBUMS: usize = 8;

impl Album {

    fn title(self) -> &'static str {
        match self {
            Self::All => "All Photos",
            Self::Recents => "Recents",
            Self::Favorites => "Favorites",
            Self::Screenshots => "Screenshots",
            Self::Trash => "Recently Deleted",
            Self::Hidden => "Hidden",
            Self::User(_) => "Album",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Sheet {
    Share,
    Delete,
    Info,
}

/// The edit mode: the screen-fitted source, the settings and the preview.
struct Editor {
    photo: Photo,
    src: Image,
    edit: Edit,
    preview: Image,
    pane: EditPane,
    /// the luminance histogram of the preview
    hist: [u32; edit::HISTOGRAM_BINS],
}

impl Editor {
    fn refresh(&mut self) {
        self.preview = self.edit.apply(&self.src);
        self.hist = edit::histogram(&self.preview);
    }
}

enum Done {
    Scan { gen: u64, all: Vec<Photo>, trash: Vec<Photo> },
    Thumb { key: String, image: Option<Image> },
    /// a thumbnail job that found its cell off screen and did nothing
    ThumbSkipped { key: String },
    Full { key: String, image: Option<Image> },
    Wallpaper { id: String, ok: bool },
}

pub struct Photos {
    w: i32,
    h: i32,
    data: PathBuf,
    pictures: PathBuf,
    dirs: Vec<PathBuf>,
    pool: Pool<Done>,
    gen: u64,
    scanning: bool,
    all: Vec<Photo>,
    /// `all` without the hidden photos: what All Photos and the smart albums draw from
    visible: Vec<Photo>,
    /// the hidden photos (the Hidden album)
    hidden: Vec<Photo>,
    favorites: Vec<Photo>,
    trash: Vec<Photo>,
    recents: Vec<Photo>,
    screenshots: Vec<Photo>,
    /// user albums: name and members, by `ua.<i>.` prefs
    user_albums: Vec<(String, Vec<Photo>)>,
    /// the current album after the date filter: what the grid and viewer index
    view: Vec<Photo>,
    /// the active month filter: its title and `[start, end)` in Unix seconds
    filter: Option<(String, (i64, i64))>,
    selection: Selection,
    /// the first tap on a permanent multi-delete asks to confirm
    confirm_purge: bool,
    /// rows of the info sheet
    info: Vec<(String, String)>,
    editor: Option<Editor>,
    album: Album,
    album_menu: bool,
    /// the grid order: newest first (true) or oldest first, the library's own order (false)
    newest_first: bool,
    sections: Vec<Section>,
    grid: Grid,
    scroll: Scroller,
    cols: usize,
    /// local zone offset from UTC in seconds
    offset: i32,
    disk: DiskCache,
    thumbs: MemCache,
    thumb_pending: HashSet<String>,
    thumb_failed: HashSet<String>,
    /// keys of the thumbnails worth decoding now; queued jobs for others are dropped
    thumb_wanted: Arc<Mutex<HashSet<String>>>,
    /// scroll offset at the last thumbnail request, to tell which way it moves
    thumb_scroll: f32,
    /// the thumbnail preparations started for the grid's window (bifrost_ui photo_caching)
    preheat: PreheatCache,
    preheat_keys: Vec<PreKey>,
    /// the Info sheet offers Revert to Original: the open photo has a saved edit
    info_revert: bool,
    /// thumbnails resampled to the grid's cell size, so drawing is a plain copy
    cells: MemCache,
    cell_px: usize,
    /// favourite photos' paths, for the grid's hearts
    fav_paths: HashSet<PathBuf>,
    full: MemCache,
    full_pending: HashSet<String>,
    viewer: Option<Viewer>,
    slideshow: Option<Slideshow>,
    sheet: Option<Sheet>,
    prefs: Prefs,
    text: TextCache,
    pressed: Option<Target>,
    scrolling: bool,
    /// the first scan has arrived (the grid then starts at the bottom)
    started: bool,
    /// pictures added while their files were still being written; a scan
    /// that has not seen them yet keeps them
    recent: Vec<Photo>,
    effects: Vec<Effect>,
    /// a picture the user chose to send; the shell opens the share sheet
    share_req: Option<PathBuf>,
    camera_req: bool,
    dirty: bool,
}

/// Top of the scrolling grid: below the status and navigation bars.
pub fn content_top(ui: &Ui) -> f32 {
    ui.pt(STATUS_PT + NAV_PT)
}

/// Vertical centre of the navigation bar.
fn nav_mid(ui: &Ui) -> f32 {
    ui.pt(STATUS_PT + NAV_PT / 2.0)
}

impl Photos {
    /// Lays the app out for a pane of `w` x `h` px (Split View, Slide Over).
    pub fn set_size(&mut self, w: i32, h: i32) {
        self.w = w;
        self.h = h;
    }

    /// `data` holds the caches and settings, `dirs` are scanned for pictures
    /// (the first is ours and receives screenshots and the trash), `offset`
    /// is the local zone's offset from UTC in seconds.
    pub fn new(ui: &Ui, w: i32, h: i32, data: PathBuf, dirs: Vec<PathBuf>, offset: i32) -> std::io::Result<Self> {
        let pictures = dirs.first().cloned().unwrap_or_else(|| data.join("Pictures"));
        std::fs::create_dir_all(&pictures)?;
        let mut p = Photos {
            w,
            h,
            pictures,
            dirs,
            pool: Pool::new(WORKERS)?,
            gen: 0,
            scanning: false,
            all: vec![],
            visible: vec![],
            hidden: vec![],
            favorites: vec![],
            trash: vec![],
            recents: vec![],
            screenshots: vec![],
            user_albums: vec![],
            view: vec![],
            filter: None,
            selection: Selection::default(),
            confirm_purge: false,
            info: vec![],
            editor: None,
            album: Album::All,
            album_menu: false,
            newest_first: false,
            sections: vec![],
            grid: Grid::build(&[], ZOOMS[ZOOM_DEFAULT], w as f32, GAP_PX, ui.pt(HEADER_PT)),
            scroll: Scroller::new(),
            cols: ZOOMS[ZOOM_DEFAULT],
            offset,
            disk: DiskCache::new(data.join(THUMB_DIR), thumbs::MAX_FILES),
            thumbs: MemCache::new(thumbs::MEM_BUDGET),
            thumb_pending: HashSet::new(),
            thumb_failed: HashSet::new(),
            thumb_wanted: Arc::new(Mutex::new(HashSet::new())),
            thumb_scroll: 0.0,
            preheat: PreheatCache::new(),
            preheat_keys: Vec::new(),
            info_revert: false,
            cells: MemCache::new(thumbs::MEM_BUDGET),
            cell_px: 0,
            fav_paths: HashSet::new(),
            full: MemCache::new(FULL_BUDGET).with_max_items(FULL_MAX),
            full_pending: HashSet::new(),
            viewer: None,
            slideshow: None,
            sheet: None,
            prefs: Prefs::load(data.join(PREFS_FILE)),
            text: TextCache::default(),
            pressed: None,
            scrolling: false,
            started: false,
            recent: vec![],
            effects: vec![],
            share_req: None,
            camera_req: false,
            dirty: true,
            data,
        };
        p.newest_first = p.prefs.get("sort") == Some("newest");
        if let Some(c) = p.prefs.get_parsed::<usize>("cols").filter(|c| ZOOMS.contains(c)) {
            p.cols = c;
        }
        p.rescan();
        Ok(p)
    }

    /// The fds that become readable when a job finished; call `pump` then.
    pub fn fds(&self) -> Vec<std::ffi::c_int> {
        vec![self.pool.fd()]
    }

    /// Effects produced since the last call (a wallpaper to apply).
    pub fn take_effects(&mut self) -> Vec<Effect> {
        std::mem::take(&mut self.effects)
    }

    /// True once when the empty library's Open Camera button was tapped.
    pub fn take_camera_request(&mut self) -> bool {
        std::mem::take(&mut self.camera_req)
    }

    /// The picture to share, if Send Nearby was just chosen.
    pub fn take_share(&mut self) -> Option<PathBuf> {
        self.share_req.take()
    }

    /// True while the viewer hides its bars (no status bar or home indicator).
    pub fn fullscreen(&self) -> bool {
        self.slideshow.as_ref().is_some_and(|s| !s.controls) || self.viewer.as_ref().is_some_and(|v| !v.chrome)
    }

    /// When `tick` next needs to run without input.
    pub fn next_wake(&self) -> Option<f64> {
        self.viewer.as_ref().and_then(Viewer::next_wake)
    }

    /// The photos of the current album, in grid order.
    pub fn photos(&self) -> &[Photo] {
        &self.view
    }

    /// The photos of `album` before the date filter.
    fn album_list(&self, album: Album) -> &[Photo] {
        match album {
            Album::All => &self.visible,
            Album::Hidden => &self.hidden,
            Album::Recents => &self.recents,
            Album::Favorites => &self.favorites,
            Album::Screenshots => &self.screenshots,
            Album::Trash => &self.trash,
            Album::User(i) => self.user_albums.get(i as usize).map_or(&[][..], |u| &u.1),
        }
    }

    /// The albums the picker lists: the fixed ones, then the user's.
    fn menu_albums(&self) -> Vec<Album> {
        let mut v = vec![Album::All, Album::Recents, Album::Favorites, Album::Screenshots, Album::Trash];
        if !self.hidden.is_empty() {
            v.push(Album::Hidden);
        }
        v.extend((0..self.user_albums.len()).map(|i| Album::User(i as u8)));
        v
    }

    /// Is `p` hidden from All Photos.
    pub fn is_hidden(&self, p: &Photo) -> bool {
        self.prefs.get(&format!("hide:{}", p.path.display())) == Some("1")
    }

    fn set_hidden(&mut self, p: &Photo, on: bool) {
        let _ = self.pref_set(&format!("hide:{}", p.path.display()), if on { "1" } else { "" });
    }

    fn album_title(&self, album: Album) -> String {
        match album {
            Album::User(i) => self.user_albums.get(i as usize).map_or_else(|| album.title().to_string(), |u| u.0.clone()),
            _ => album.title().to_string(),
        }
    }

    fn in_user_album(&self, i: usize, p: &Photo) -> bool {
        self.prefs.get(&format!("ua.{i}.p:{}", p.path.display())) == Some("1")
    }

    /// Creates an empty user album named "Album N"; its index.
    fn new_album(&mut self) -> Option<usize> {
        let i = (0..MAX_USER_ALBUMS).find(|i| self.prefs.get(&format!("ua.{i}.name")).is_none_or(str::is_empty))?;
        let name = format!("Album {}", i + 1);
        self.pref_set(&format!("ua.{i}.name"), &name).then_some(i)
    }

    /// Adds the photo to user album `i`, or removes it when it is in it.
    fn toggle_in_album(&mut self, i: usize, p: &Photo) {
        let on = !self.in_user_album(i, p);
        self.pref_set(&format!("ua.{i}.p:{}", p.path.display()), if on { "1" } else { "0" });
        self.pref_set("ua.last", &i.to_string());
    }

    pub fn album(&self) -> Album {
        self.album
    }

    /// Switches albums and returns to that album's grid.
    pub fn select_album(&mut self, ui: &Ui, album: Album) {
        self.album = album;
        self.album_menu = false;
        self.selection.end();
        self.confirm_purge = false;
        self.editor = None;
        self.viewer = None;
        self.sheet = None;
        self.pressed = None;
        self.scrolling = false;
        self.rebuild(ui);
        self.scroll.jump_to(if album == Album::Trash { 0.0 } else { f32::MAX });
    }

    pub fn cols(&self) -> usize {
        self.cols
    }

    /// Filters the album to the date `query` names ("2026-10", "oct 2026",
    /// "4 October 2026"); false when it is not a date.
    pub fn set_date_query(&mut self, ui: &Ui, query: &str) -> bool {
        let Some(range) = albums::parse_date_query(query, self.offset, unix_now()) else { return false };
        self.set_filter(ui, Some((query.trim().to_string(), range)));
        true
    }

    /// Removes the date filter.
    pub fn clear_filter(&mut self, ui: &Ui) {
        self.set_filter(ui, None);
    }

    fn set_filter(&mut self, ui: &Ui, filter: Option<(String, (i64, i64))>) {
        self.filter = filter;
        self.selection.end();
        self.viewer = None;
        self.sheet = None;
        self.rebuild(ui);
        self.scroll.jump_to(if self.album == Album::Trash { 0.0 } else { f32::MAX });
    }

    /// The month filter's next state: the newest month present, then older
    /// ones, then none.
    fn cycle_filter(&mut self, ui: &Ui) {
        let months = albums::month_filters(self.album_list(self.album), self.offset);
        let next = match &self.filter {
            None => months.into_iter().next(),
            Some((title, _)) => {
                let at = months.iter().position(|(t, _)| t == title);
                at.and_then(|i| months.into_iter().nth(i + 1))
            }
        };
        self.set_filter(ui, next);
    }

    /// The photo open in the viewer.
    pub fn viewing(&self) -> Option<&Photo> {
        self.viewer.as_ref().and_then(|v| self.photos().get(v.index))
    }

    /// key=value lines for remote inspection.
    pub fn describe(&self) -> String {
        let screen = match (&self.viewer, self.sheet) {
            _ if self.editor.is_some() => "editor",
            (Some(_), Some(_)) => "sheet",
            (Some(_), None) => "viewer",
            (None, _) => "grid",
        };
        format!("photos_screen={screen}\nphotos_album={:?}\nphotos_count={}\nphotos_trash={}\nphotos_cols={}\nphotos_thumbs={}\nphotos_pending={}\nphotos_favorites={}\nphotos_album_count={}\nphotos_scanning={}\nphotos_filter={}\nphotos_selected={}\n", self.album, self.all.len(), self.trash.len(), self.cols, self.thumbs.len(), self.thumb_pending.len(), self.favorites.len(), self.photos().len(), u8::from(self.scanning), self.filter.as_ref().map_or("", |f| f.0.as_str()), self.selection.len())
    }

    pub fn is_favourite(&self, p: &Photo) -> bool {
        self.prefs.get(&format!("fav:{}", p.path.display())) == Some("1")
    }

    // --- library ---

    /// Rescans the directories on a worker thread.
    pub fn rescan(&mut self) {
        self.gen += 1;
        let (gen, dirs, pictures, offset) = (self.gen, self.dirs.clone(), self.pictures.clone(), self.offset);
        self.scanning = true;
        self.pool.spawn(move || Done::Scan { gen, all: library::scan_at(&dirs, offset), trash: library::scan_trash(&pictures, offset) });
    }

    /// Sets the local zone offset (section titles follow it).
    pub fn set_offset(&mut self, ui: &Ui, offset: i32) {
        if offset != self.offset {
            self.offset = offset;
            self.rebuild(ui);
            // capture times were read in the old zone
            self.rescan();
        }
    }

    /// Recomputes the albums, the filtered view, the sections and the grid.
    fn rebuild(&mut self, ui: &Ui) {
        let now = unix_now();
        let (hidden, visible): (Vec<Photo>, Vec<Photo>) = self.all.iter().cloned().partition(|p| self.is_hidden(p));
        self.hidden = hidden;
        self.visible = visible;
        if self.album == Album::Hidden && self.hidden.is_empty() {
            self.album = Album::All;
        }
        self.favorites = models::smart_album(SmartAlbum::Favourites, &self.visible, |p| self.is_favourite(p), now);
        self.fav_paths = self.all.iter().chain(&self.trash).filter(|p| self.is_favourite(p)).map(|p| p.path.clone()).collect();
        self.user_albums = (0..MAX_USER_ALBUMS)
            .filter_map(|i| {
                let name = self.prefs.get(&format!("ua.{i}.name")).filter(|n| !n.is_empty())?.to_string();
                Some((i, name))
            })
            .map(|(i, name)| (name, self.visible.iter().filter(|p| self.in_user_album(i, p)).cloned().collect()))
            .collect();
        self.recents = models::smart_album(SmartAlbum::RecentlyAdded, &self.visible, |p| self.is_favourite(p), now);
        self.screenshots = models::smart_album(SmartAlbum::Screenshots, &self.visible, |p| self.is_favourite(p), now);
        let view = {
            let base = self.album_list(self.album);
            let filtered = match &self.filter {
                Some((_, range)) => albums::in_range(base, *range),
                None => base.to_vec(),
            };
            models::by_date(&filtered, self.newest_first)
        };
        self.view = view;
        self.selection.retain_in(&self.view);
        self.sections = sections(&self.view, self.offset);
        self.grid = Grid::build(&self.sections, self.cols, self.w as f32, GAP_PX, ui.pt(HEADER_PT));
        self.fit_scroll(ui);
        self.dirty = true;
    }

    fn viewer_anchor(&self) -> Option<(PathBuf, usize)> {
        self.viewer.as_ref().and_then(|v| self.photos().get(v.index).map(|p| (p.path.clone(), v.index)))
    }

    /// Keep the same photo across insertions/reordering; after its removal,
    /// use the next surviving photo, or the previous one at the album's end.
    fn restore_viewer(&mut self, anchor: Option<(PathBuf, usize)>) {
        let Some((path, old_index)) = anchor else { return };
        let count = self.photos().len();
        if count == 0 {
            self.viewer = None;
            self.sheet = None;
            return;
        }
        let found = self.photos().iter().position(|p| p.path == path);
        let index = found.unwrap_or(old_index.min(count - 1));
        if found.is_some() && self.viewer.as_ref().is_some_and(|v| v.index == index && v.count == count) {
            return;
        }
        let chrome = self.viewer.as_ref().is_none_or(|v| v.chrome);
        self.viewer = Some(Viewer::new(index, count, self.w as f32, self.h as f32));
        self.viewer.as_mut().unwrap().chrome = chrome;
        // A confirmation sheet belongs to the photo that originally opened it.
        if found.is_none() {
            self.sheet = None;
        }
    }

    fn move_favourite(&mut self, from: &Photo, to: &Photo) {
        for i in 0..self.user_albums.len() {
            if self.in_user_album(i, from) {
                self.pref_set(&format!("ua.{i}.p:{}", to.path.display()), "1");
                self.pref_set(&format!("ua.{i}.p:{}", from.path.display()), "0");
            }
        }
        let on = self.is_favourite(from);
        // Keep the source marker if the destination could not be persisted.
        if self.write_favourite(to, on) {
            self.write_favourite(from, false);
        }
    }

    fn write_favourite(&mut self, photo: &Photo, on: bool) -> bool {
        let saved = self.pref_set(&format!("fav:{}", photo.path.display()), if on { "1" } else { "0" });
        if self.is_favourite(photo) {
            self.fav_paths.insert(photo.path.clone());
        } else {
            self.fav_paths.remove(&photo.path);
        }
        saved
    }

    /// When a trashed photo was deleted (Unix seconds), if recorded.
    fn deleted_at(&self, photo: &Photo) -> Option<i64> {
        self.prefs.get(&format!("del:{}", photo.path.display())).and_then(|v| v.parse().ok())
    }

    fn forget_deleted(&mut self, photo: &Photo) {
        self.pref_set(&format!("del:{}", photo.path.display()), "");
    }

    /// Photos in Recently Deleted for 30 days are removed for good; ones with
    /// no recorded deletion time start their 30 days now.
    fn purge_expired(&mut self) {
        let now = unix_now();
        let mut gone = Vec::new();
        for p in self.trash.clone() {
            match self.deleted_at(&p) {
                Some(at) if albums::purge_due(at, now) => {
                    if std::fs::remove_file(&p.path).is_ok() {
                        self.forget_deleted(&p);
                        self.write_favourite(&p, false);
                        gone.push(p.path.clone());
                    }
                }
                Some(_) => {}
                None => {
                    self.pref_set(&format!("del:{}", p.path.display()), &now.to_string());
                }
            }
        }
        self.trash.retain(|p| !gone.contains(&p.path));
    }

    /// Writes one setting; false (and the old value kept) if it cannot be saved.
    fn pref_set(&mut self, key: &str, value: &str) -> bool {
        let mut next = self.prefs.clone();
        match next.set(key, value) {
            Ok(()) => {
                self.prefs = next;
                true
            }
            Err(e) => {
                eprintln!("photos: {PREFS_FILE}: {e}");
                false
            }
        }
    }

    fn content_bottom(&self, ui: &Ui) -> f32 {
        self.h as f32 - home_zone(ui)
    }

    fn fit_scroll(&mut self, ui: &Ui) {
        let view = self.content_bottom(ui) - content_top(ui);
        self.scroll.set_max(self.grid.height - view);
    }

    /// Adds a picture that was just written (a screenshot) with its
    /// screen-fitted image, so it shows before any rescan.
    pub fn add_photo(&mut self, ui: &Ui, photo: Photo, image: Image) {
        let anchor = self.viewer_anchor();
        let key = photo.key();
        self.thumbs.insert(&key, imgdec::cover(&image, thumbs::SIZE, thumbs::SIZE));
        self.full.insert(&key, imgdec::contain(&image, self.w as usize, self.h as usize));
        self.all.retain(|p| p.path != photo.path);
        let at = self.all.partition_point(|p| (p.mtime, &p.path) <= (photo.mtime, &photo.path));
        self.all.insert(at, photo.clone());
        self.recent.push(photo);
        self.rebuild(ui);
        self.restore_viewer(anchor);
    }

    /// Opens the viewer on the picture at `path`, if it is in the library.
    pub fn show(&mut self, ui: &Ui, path: &Path) -> bool {
        if !self.all.iter().any(|p| p.path == path) {
            return false;
        }
        if self.album != Album::All || self.filter.is_some() {
            self.filter = None;
            self.select_album(ui, Album::All);
        }
        let Some(i) = self.view.iter().position(|p| p.path == path) else { return false };
        self.open_viewer(i);
        true
    }

    fn open_viewer(&mut self, i: usize) {
        if i >= self.photos().len() {
            return;
        }
        self.album_menu = false;
        self.sheet = None;
        self.editor = None;
        self.viewer = Some(Viewer::new(i, self.photos().len(), self.w as f32, self.h as f32));
        self.dirty = true;
    }

    // --- jobs ---

    fn request_thumbs(&mut self, ui: &Ui) {
        let top = self.scroll.offset();
        let view = self.content_bottom(ui) - content_top(ui);
        // while moving, also the screenful the scroll is heading to
        let ahead = if top > self.thumb_scroll { (top + view, top + 2.0 * view) } else if top < self.thumb_scroll { (top - view, top) } else { (top, top) };
        self.thumb_scroll = top;
        let near = self.grid.visible(top, top + view);
        let more: Vec<usize> = self.grid.visible(ahead.0, ahead.1).into_iter().filter(|i| !near.contains(i)).collect();
        let order = [near, more].concat();
        let wanted: Vec<Photo> = order.into_iter().filter_map(|i| self.photos().get(i).cloned()).collect();
        let keys: Vec<String> = wanted.iter().map(Photo::key).collect();
        // jobs still queued for cells that scrolled away give way
        *self.thumb_wanted.lock().unwrap_or_else(|p| p.into_inner()) = keys.iter().cloned().collect();
        // the caching manager keeps the same window as its preparations; the ones that scrolled away stop
        let window: Vec<PreKey> = keys.iter().map(|k| PreKey::new(k, thumbs::SIZE as u32, thumbs::SIZE as u32)).collect();
        let stale: Vec<PreKey> = self.preheat_keys.iter().filter(|k| !window.contains(*k)).cloned().collect();
        self.preheat.stop(&stale);
        self.preheat.start(&window);
        self.preheat_keys = window;
        for (photo, key) in wanted.into_iter().zip(keys) {
            if self.thumbs.contains(&key) || self.thumb_pending.contains(&key) || self.thumb_failed.contains(&key) {
                continue;
            }
            self.thumb_pending.insert(key.clone());
            let recipe = self.recipe(&photo);
            let (disk, still_wanted) = (self.disk.clone(), self.thumb_wanted.clone());
            let failed = key.clone();
            self.pool.spawn_or(
                move || {
                    if !still_wanted.lock().unwrap_or_else(|p| p.into_inner()).contains(&key) {
                        return Done::ThumbSkipped { key };
                    }
                    let image = disk.read(&key).or_else(|| {
                        let img = decode_edited(&std::fs::read(&photo.path).ok()?, recipe)?;
                        let thumb = imgdec::cover(&img, thumbs::SIZE, thumbs::SIZE);
                        disk.write(&key, &thumb);
                        Some(thumb)
                    });
                    Done::Thumb { key, image }
                },
                move || Done::Thumb { key: failed, image: None },
            );
        }
    }

    fn request_full(&mut self) {
        let (w, h) = (self.w as usize, self.h as usize);
        let around: Vec<Photo> = if let Some(sh) = &self.slideshow {
            vec![sh.current().clone(), sh.photos[sh.next_index()].clone()]
        } else {
            let Some(v) = &self.viewer else { return };
            (v.index.saturating_sub(1)..=v.index + 1).filter_map(|i| self.photos().get(i).cloned()).collect()
        };
        for photo in around {
            let key = photo.key();
            if self.full.contains(&key) || self.full_pending.contains(&key) {
                continue;
            }
            self.full_pending.insert(key.clone());
            let recipe = self.recipe(&photo);
            let failed = key.clone();
            // the picture on screen jumps the queue of grid thumbnails
            self.pool.spawn_front_or(
                move || {
                    let image = std::fs::read(&photo.path).ok().and_then(|b| decode_edited(&b, recipe)).map(|img| imgdec::contain(&img, w, h));
                    Done::Full { key, image }
                },
                move || Done::Full { key: failed, image: None },
            );
        }
    }

    /// Next free `custom-<n>` wallpaper id.
    fn next_wallpaper_id(&self) -> String {
        let dir = self.data.join("wallpapers");
        (1..).map(|n| format!("{CUSTOM_PREFIX}{n}")).find(|id| !dir.join(format!("{id}.raw")).exists()).unwrap()
    }

    /// Set as Wallpaper: covers the screen with the photo, packs it next to
    /// the stock stills (with a thumbnail and an index entry) and, once
    /// written, emits `Effect::Wallpaper`.
    fn set_wallpaper(&mut self, photo: &Photo) {
        let id = self.next_wallpaper_id();
        let dir = self.data.join("wallpapers");
        let (w, h, path, n) = (self.w as usize, self.h as usize, photo.path.clone(), id.trim_start_matches(CUSTOM_PREFIX).to_string());
        let job_id = id.clone();
        let recipe = self.recipe(photo);
        self.pool.spawn(move || {
            let ok = (|| {
                let img = decode_edited(&std::fs::read(&path).ok()?, recipe)?;
                let full = imgdec::cover(&img, w, h);
                std::fs::create_dir_all(&dir).ok()?;
                let write = |name: &str, bytes: Vec<u8>| {
                    let p = dir.join(name);
                    let tmp = p.with_extension("tmp");
                    std::fs::write(&tmp, bytes).ok()?;
                    std::fs::rename(tmp, p).ok()
                };
                write(&format!("{job_id}.thumb.raw"), thumbs::to_raw(&imgdec::cover(&full, 320, 180)))?;
                write(&format!("{job_id}.raw"), thumbs::to_raw(&full))?;
                Prefs::load(dir.join("index.ini")).set(&job_id, &format!("Photo {n}")).ok()
            })()
            .is_some();
            Done::Wallpaper { id: job_id, ok }
        });
        self.dirty = true;
    }

    /// Applies finished jobs; true if the screen changed.
    pub fn pump(&mut self, ui: &Ui) -> bool {
        let mut changed = false;
        for done in self.pool.drain() {
            changed = true;
            match done {
                Done::Scan { gen, all, trash } => {
                    if gen != self.gen {
                        continue;
                    }
                    self.scanning = false;
                    let anchor = self.viewer_anchor();
                    self.all = all;
                    self.trash = trash;
                    self.purge_expired();
                    self.recent.retain(|r| !self.all.iter().any(|p| p.path == r.path));
                    for r in self.recent.clone() {
                        let at = self.all.partition_point(|p| (p.mtime, &p.path) <= (r.mtime, &r.path));
                        self.all.insert(at, r);
                    }
                    self.rebuild(ui);
                    self.restore_viewer(anchor);
                    if !self.started {
                        self.started = true;
                        self.scroll.jump_to(f32::MAX);
                    }
                }
                Done::Thumb { key, image } => {
                    self.thumb_pending.remove(&key);
                    match image {
                        Some(img) => self.thumbs.insert(&key, img),
                        None => {
                            self.thumb_failed.insert(key);
                        }
                    }
                }
                Done::ThumbSkipped { key } => {
                    self.thumb_pending.remove(&key);
                }
                Done::Full { key, image } => {
                    self.full_pending.remove(&key);
                    if let Some(img) = image {
                        self.full.insert(&key, img);
                    }
                }
                Done::Wallpaper { id, ok } => {
                    if ok {
                        self.effects.push(Effect::Wallpaper(id));
                    } else {
                        eprintln!("photos: wallpaper {id} not written");
                    }
                }
            }
        }
        self.dirty |= changed;
        changed
    }

    /// Advances scrolling, paging and the tap timer; asks for pictures that
    /// came into view. True when the screen changed.
    pub fn tick(&mut self, ui: &Ui, t: f64) -> bool {
        if let Some(sh) = &mut self.slideshow {
            sh.tick(t);
            self.request_full();
            self.dirty = true;
            return true;
        }
        let moving = match &mut self.viewer {
            Some(v) => {
                let was = v.animating();
                if v.tick(t) != Action::None {
                    self.dirty = true;
                }
                was || v.animating()
            }
            None => self.scroll.tick(t),
        };
        if self.viewer.is_some() {
            self.request_full();
        } else {
            self.request_thumbs(ui);
        }
        self.dirty |= moving;
        self.dirty
    }

    // --- touch ---

    /// Nothing to show in All Photos: bifrost 15's "Library" with its empty state.
    fn blank_library(&self) -> bool {
        self.album == Album::All && self.filter.is_none() && !self.selection.active && !self.scanning && self.photos().is_empty()
    }

    /// The empty library's Open Camera button (x, y, w, h), centred on the screen.
    fn camera_button(&self, ui: &Ui) -> (f32, f32, f32, f32) {
        let (w, h) = (ui.pt(EMPTY_BUTTON_W_PT), ui.pt(EMPTY_BUTTON_H_PT));
        ((self.w as f32 - w) / 2.0, self.h as f32 / 2.0 + ui.pt(34.0), w, h)
    }

    fn nav_hit(&self, ui: &Ui, x: f32, y: f32) -> Option<Target> {
        if y >= ui.pt(STATUS_PT) && y < content_top(ui) {
            let m = ui.pt(MARGIN_PT);
            if x < m + ui.pt(150.0) {
                return Some(Target::Album);
            }
            if self.blank_library() {
                return None;
            }
            if x >= self.w as f32 - m - ui.pt(44.0) {
                return Some(Target::ZoomIn);
            }
            if x >= self.w as f32 - m - ui.pt(96.0) {
                return Some(Target::ZoomOut);
            }
            if x >= self.w as f32 - m - ui.pt(96.0 + 76.0) {
                return Some(Target::Select);
            }
            if (x - self.w as f32 / 2.0).abs() <= ui.pt(100.0) {
                return Some(if self.selection.active { Target::SelectAll } else { Target::Filter });
            }
        }
        None
    }

    /// Bottom bar buttons of the viewer as (target, centre x).
    fn bar_buttons(&self) -> Vec<(Target, f32)> {
        let w = self.w as f32;
        match self.album {
            Album::Trash => vec![(Target::Recover, w * 0.15), (Target::Delete, w * 0.85)],
            _ => vec![(Target::Share, w * 0.12), (Target::Info, w * 0.31), (Target::Favourite, w * 0.5), (Target::Edit, w * 0.69), (Target::Delete, w * 0.88)],
        }
    }

    fn bar_top(&self, ui: &Ui) -> f32 {
        self.h as f32 - home_zone(ui) - ui.pt(BAR_PT)
    }

    /// The sheet box (x, y, w, h) above the button that opened it.
    fn sheet_rect(&self, ui: &Ui, sheet: Sheet) -> (f32, f32, f32, f32) {
        let anchor = match sheet {
            Sheet::Share => Target::Share,
            Sheet::Delete => Target::Delete,
            Sheet::Info => Target::Info,
        };
        let cx = self.bar_buttons().iter().find(|(t, _)| *t == anchor).map_or(self.w as f32 / 2.0, |(_, x)| *x);
        let rows = sheet_rows(sheet) as f32;
        if sheet == Sheet::Info {
            let (iw, ih) = self.info_size(ui);
            return ((self.w as f32 - iw) / 2.0, self.bar_top(ui) - ih - ui.pt(12.0), iw, ih);
        }
        let (sw, sh) = (ui.pt(SHEET_W_PT), ui.pt(SHEET_ROW_PT * rows));
        let x = (cx - sw / 2.0).clamp(ui.pt(8.0), self.w as f32 - sw - ui.pt(8.0));
        (x, self.bar_top(ui) - sh - ui.pt(12.0), sw, sh)
    }

    fn hit(&self, ui: &Ui, x: f32, y: f32) -> Option<Target> {
        if self.album_menu {
            for album in self.menu_albums() {
                if within(x, y, self.album_row(ui, album)) {
                    return Some(Target::ChooseAlbum(album));
                }
            }
            if self.user_albums.len() < MAX_USER_ALBUMS && within(x, y, self.new_album_row(ui)) {
                return Some(Target::NewAlbum);
            }
            if within(x, y, self.sort_row(ui)) {
                return Some(Target::SortOrder);
            }
            return Some(Target::AlbumCancel);
        }
        if self.editor.is_some() {
            return self.edit_hit(ui, x, y);
        }
        if let Some(sh) = &self.slideshow {
            if sh.controls {
                let (cy, r) = (self.show_bar_y(ui), ui.pt(32.0));
                let cx = self.w as f32 / 2.0;
                for (t, bx) in [(Target::ShowPrev, cx - ui.pt(96.0)), (Target::ShowPlay, cx), (Target::ShowNext, cx + ui.pt(96.0))] {
                    if (x - bx).abs() <= r && (y - cy).abs() <= r {
                        return Some(t);
                    }
                }
                if y < content_top(ui) && x < ui.pt(120.0) {
                    return Some(Target::ShowDone);
                }
            }
            return Some(Target::ShowTap);
        }
        let Some(v) = &self.viewer else {
            if let Some(t) = self.nav_hit(ui, x, y) {
                return Some(t);
            }
            if self.blank_library() && within(x, y, self.camera_button(ui)) {
                return Some(Target::OpenCamera);
            }
            if self.selection.active && y >= self.bar_top(ui) && y < self.content_bottom(ui) {
                let w = self.w as f32;
                return Some(if x < w / 3.0 {
                    Target::SelShare
                } else if x >= w * 2.0 / 3.0 {
                    Target::SelDelete
                } else if self.album != Album::Trash {
                    Target::SelSlideshow
                } else {
                    Target::Content
                });
            }
            if y < content_top(ui) || y >= self.content_bottom(ui) {
                return None;
            }
            return Some(self.grid.hit(x, y - content_top(ui) + self.scroll.offset()).map_or(Target::Content, Target::Cell));
        };
        if let Some(sheet) = self.sheet {
            if sheet == Sheet::Info {
                let rated = 2 * usize::from(self.info_has_rating());
                if self.info_has_rating() {
                    let (rect, (x0, x1)) = self.info_rating_row(ui);
                    if within(x, y, rect) {
                        return Some(stars::star_at(x, x0, x1).map_or(Target::SheetCancel, Target::Rate));
                    }
                    if within(x, y, self.info_hide_row(ui)) {
                        return Some(Target::InfoHide);
                    }
                }
                if self.info_revert {
                    let (sx, sy, sw, _) = self.sheet_rect(ui, Sheet::Info);
                    let row = ui.pt(INFO_ROW_PT);
                    if within(x, y, (sx, sy + row * (self.info.len() + 1 + rated) as f32, sw, row)) {
                        return Some(Target::InfoRevert);
                    }
                }
                return Some(Target::SheetCancel);
            }
            let (sx, sy, sw, sh) = self.sheet_rect(ui, sheet);
            let rows = sheet_rows(sheet) as f32;
            let row = ((y - sy) / (sh / rows)).floor();
            if !within(x, y, (sx, sy, sw, sh)) {
                return Some(Target::SheetCancel);
            }
            return Some(match (sheet, row as i32) {
                (Sheet::Share, 0) => Target::SheetSlideshow,
                (Sheet::Share, 1) => Target::SheetCopy,
                (Sheet::Share, 2) => Target::SheetFiles,
                (Sheet::Share, 3) => Target::SheetNearby,
                (Sheet::Share, 4) => Target::SheetDuplicate,
                (Sheet::Share, 5) | (Sheet::Delete, 0) => Target::SheetAction,
                _ => Target::SheetCancel,
            });
        }
        if v.chrome {
            if y < content_top(ui) {
                return Some(if x < ui.pt(160.0) {
                    Target::Back
                } else if x >= self.w as f32 - ui.pt(100.0) && self.album != Album::Trash {
                    Target::AddToAlbum
                } else {
                    Target::Photo
                });
            }
            if y >= self.bar_top(ui) {
                let r = ui.pt(40.0);
                return self.bar_buttons().into_iter().find(|(_, cx)| (x - cx).abs() <= r).map(|(t, _)| t);
            }
        }
        Some(Target::Photo)
    }

    pub fn touch(&mut self, ui: &Ui, ev: Event, t: f64) {
        self.dirty = true;
        let (x, y) = match ev {
            Event::Down(x, y) | Event::Move(x, y) | Event::Up(x, y) => (x, y),
            Event::Power | Event::Wake | Event::VolumeUp | Event::VolumeDown | Event::Screenshot => return,
        };
        match ev {
            Event::Down(..) => {
                self.pressed = self.hit(ui, x, y);
                match self.pressed {
                    Some(Target::Photo) => {
                        if let Some(v) = &mut self.viewer {
                            v.down(x, y, t);
                        }
                    }
                    Some(Target::Content | Target::Cell(_)) => {
                        self.scroll.down(y, t);
                        self.scrolling = true;
                    }
                    Some(Target::Slide(slider)) => self.set_slider(ui, slider, x),
                    _ => {}
                }
            }
            Event::Move(..) => {
                if self.pressed == Some(Target::Photo) {
                    if let Some(v) = &mut self.viewer {
                        v.moved(x, y, t);
                    }
                } else if let Some(Target::Slide(slider)) = self.pressed {
                    self.set_slider(ui, slider, x);
                } else if self.scrolling {
                    self.scroll.moved(y, t);
                    if self.scroll.dragging() {
                        self.pressed = None;
                    }
                }
            }
            Event::Up(..) | Event::Power | Event::Wake | Event::VolumeUp | Event::VolumeDown | Event::Screenshot => {
                let pressed = self.pressed.take();
                if pressed == Some(Target::Photo) {
                    if let Some(v) = &mut self.viewer {
                        v.up(x, y, t);
                    }
                    return;
                }
                let tap = if std::mem::take(&mut self.scrolling) { self.scroll.up(y, t) } else { true };
                if tap {
                    if let Some(p) = pressed.filter(|&p| Some(p) == self.hit(ui, x, y)) {
                        self.activate(ui, p, t);
                    }
                }
            }
        }
    }

    fn activate(&mut self, ui: &Ui, target: Target, t: f64) {
        match target {
            Target::Cell(i) => {
                if self.selection.active {
                    if let Some(p) = self.photos().get(i).cloned() {
                        self.selection.toggle(&p.path);
                        self.confirm_purge = false;
                    }
                } else {
                    self.open_viewer(i);
                }
            }
            Target::OpenCamera => self.camera_req = true,
            Target::Filter => self.cycle_filter(ui),
            Target::SelectAll => {
                let all = self.view.clone();
                self.selection.toggle_all(&all);
                self.confirm_purge = false;
            }
            Target::Select => {
                if self.selection.active {
                    self.selection.end();
                } else {
                    self.selection.begin();
                }
                self.confirm_purge = false;
            }
            Target::SelShare => {
                if self.album == Album::Trash {
                    self.recover_selected(ui);
                } else {
                    // the share sheet sends one file; the first selected goes
                    self.share_req = self.selection.paths().into_iter().next();
                    self.selection.end();
                }
            }
            Target::SelDelete => {
                if self.selection.is_empty() {
                    return;
                }
                if self.album == Album::Trash && !self.confirm_purge {
                    self.confirm_purge = true;
                } else {
                    self.delete_selected(ui);
                }
            }
            Target::Info => {
                if let Some(p) = self.viewing().cloned() {
                    let head = read_head(&p.path);
                    self.info = exif::describe(&p.name(), p.size, p.mtime, self.offset, &head);
                    // Recently Deleted photos have no revert: their edit left with them
                    self.info_revert = self.album != Album::Trash && self.recipe(&p).is_some();
                    self.sheet = Some(Sheet::Info);
                }
            }
            Target::Rate(n) => {
                if let Some(p) = self.viewing().cloned() {
                    let next = stars::after_tap(self.rating(&p), n);
                    self.set_rating(&p, next);
                }
            }
            Target::InfoHide => {
                if let Some(p) = self.viewing().cloned() {
                    let anchor = self.viewer_anchor();
                    let on = !self.is_hidden(&p);
                    self.set_hidden(&p, on);
                    self.sheet = None;
                    self.rebuild(ui);
                    // the photo has left this album's grid (or come back to it): the viewer lands on its neighbour
                    self.restore_viewer(anchor);
                }
            }
            Target::SheetDuplicate => {
                self.sheet = None;
                self.duplicate_current();
            }
            Target::InfoRevert => {
                self.info_revert = false;
                self.sheet = None;
                if let Some(p) = self.viewing().cloned() {
                    self.set_recipe(&p, Edit::default());
                    self.invalidate(&p);
                }
            }
            Target::Edit => {
                // the shown picture already has its edit applied, so the editor decodes the original
                if let Some(p) = self.viewing().cloned() {
                    if self.full.contains(&p.key()) {
                        let (w, h) = (self.w as usize, self.h as usize);
                        let src = std::fs::read(&p.path).ok().and_then(|b| decode_upright(&b)).map(|img| imgdec::contain(&img, w, h));
                        if let Some(src) = src {
                            self.sheet = None;
                            let edit = self.recipe(&p).unwrap_or_default();
                            let mut ed = Editor { photo: p, preview: src.clone(), src, edit, pane: EditPane::Adjust, hist: [0; edit::HISTOGRAM_BINS] };
                            ed.refresh();
                            self.editor = Some(ed);
                        }
                    }
                }
            }
            Target::EditCancel => self.editor = None,
            Target::EditRotate => {
                if let Some(ed) = &mut self.editor {
                    ed.edit.rotate(true);
                    ed.refresh();
                }
            }
            Target::EditAspect(aspect) => {
                if let Some(ed) = &mut self.editor {
                    ed.edit.aspect = aspect;
                    ed.refresh();
                }
            }
            Target::EditFilter(filter) => {
                if let Some(ed) = &mut self.editor {
                    ed.edit.filter = filter;
                    ed.refresh();
                }
            }
            Target::EditRevert => {
                if let Some(ed) = &mut self.editor {
                    ed.edit = Edit::default();
                    ed.refresh();
                }
            }
            Target::EditTool(tool) => {
                if let Some(ed) = &mut self.editor {
                    tool.apply(&mut ed.edit);
                    ed.refresh();
                }
            }
            Target::EditPane(pane) => {
                if let Some(ed) = &mut self.editor {
                    ed.pane = pane;
                }
            }
            Target::EditSave => {
                if let Some(ed) = self.editor.take() {
                    self.set_recipe(&ed.photo, ed.edit);
                    self.invalidate(&ed.photo);
                    self.dirty = true;
                }
            }
            Target::Slide(_) => {}
            Target::Album => {
                self.album_menu = true;
            }
            Target::ChooseAlbum(album) => self.select_album(ui, album),
            Target::AlbumCancel => self.album_menu = false,
            Target::SortOrder => {
                self.newest_first = !self.newest_first;
                let _ = self.pref_set("sort", if self.newest_first { "newest" } else { "oldest" });
                self.album_menu = false;
                self.rebuild(ui);
                self.scroll.jump_to(if self.newest_first { 0.0 } else { f32::MAX });
            }
            Target::NewAlbum => {
                if let Some(i) = self.new_album() {
                    self.rebuild(ui);
                    self.select_album(ui, Album::User(i as u8));
                }
            }
            Target::AddToAlbum => {
                if let Some(p) = self.viewing().cloned() {
                    let anchor = self.viewer_anchor();
                    let target = match self.album {
                        Album::User(i) => Some(i as usize),
                        _ => self.prefs.get("ua.last").and_then(|v| v.parse::<usize>().ok()).filter(|&i| i < self.user_albums.len()).or(if self.user_albums.is_empty() { self.new_album() } else { Some(0) }),
                    };
                    if let Some(i) = target {
                        self.rebuild(ui);
                        self.toggle_in_album(i, &p);
                        self.rebuild(ui);
                        self.restore_viewer(anchor);
                    }
                }
            }
            Target::ZoomIn | Target::ZoomOut => {
                let i = ZOOMS.iter().position(|&c| c == self.cols).unwrap_or(ZOOM_DEFAULT);
                let next = if target == Target::ZoomIn { (i + 1).min(ZOOMS.len() - 1) } else { i.saturating_sub(1) };
                if ZOOMS[next] != self.cols {
                    let k = if self.grid.height > 0.0 { self.scroll.offset() / self.grid.height } else { 0.0 };
                    self.cols = ZOOMS[next];
                    let _ = self.pref_set("cols", &self.cols.to_string());
                    self.rebuild(ui);
                    self.scroll.jump_to(k * self.grid.height);
                }
            }
            Target::Back => self.viewer = None,
            Target::Share => self.sheet = Some(Sheet::Share),
            Target::Delete => self.sheet = Some(Sheet::Delete),
            Target::SheetCancel => self.sheet = None,
            Target::SheetSlideshow => {
                self.sheet = None;
                let at = self.viewer.as_ref().map_or(0, |v| v.index);
                self.start_slideshow(self.photos().to_vec(), at, t);
            }
            Target::SheetCopy => {
                self.sheet = None;
                self.copy_current();
            }
            Target::SheetFiles => {
                self.sheet = None;
                self.save_to_files();
            }
            Target::SelSlideshow => {
                let chosen: Vec<Photo> = self.photos().iter().filter(|p| self.selection.contains(&p.path)).cloned().collect();
                self.start_slideshow(chosen, 0, t);
            }
            Target::ShowTap => {
                if let Some(sh) = &mut self.slideshow {
                    sh.toggle_controls(t);
                }
            }
            Target::ShowDone => self.slideshow = None,
            Target::ShowPlay => {
                if let Some(sh) = &mut self.slideshow {
                    sh.toggle_pause();
                    sh.show_controls(t);
                }
            }
            Target::ShowPrev | Target::ShowNext => {
                if let Some(sh) = &mut self.slideshow {
                    sh.step(if target == Target::ShowNext { 1 } else { -1 }, t);
                }
            }
            Target::SheetNearby => {
                self.sheet = None;
                self.share_req = self.viewing().map(|p| p.path.clone());
            }
            Target::SheetAction => {
                let sheet = self.sheet.take();
                if let Some(photo) = self.viewing().cloned() {
                    match sheet {
                        Some(Sheet::Share) => self.set_wallpaper(&photo),
                        Some(Sheet::Delete) => self.delete(ui, &photo),
                        Some(Sheet::Info) | None => {}
                    }
                }
            }
            Target::Favourite => {
                if let Some(p) = self.viewing().cloned() {
                    if self.album == Album::Trash {
                        return;
                    }
                    let anchor = self.viewer_anchor();
                    let on = !self.is_favourite(&p);
                    self.write_favourite(&p, on);
                    self.rebuild(ui);
                    self.restore_viewer(anchor);
                }
            }
            Target::Recover => {
                if self.album != Album::Trash {
                    return;
                }
                if let Some(p) = self.viewing().cloned() {
                    if self.recover_one(&p) {
                        self.after_removal(ui);
                        self.rescan();
                    }
                }
            }
            Target::Content | Target::Photo => {}
        }
        self.dirty = true;
    }

    /// Moves a photo to the trash, or deletes a trashed one for good. True
    /// when the library changed (the caller rebuilds and rescans).
    fn remove_one(&mut self, p: &Photo) -> bool {
        match self.album {
            Album::Trash => {
                let ok = std::fs::remove_file(&p.path).is_ok();
                if ok {
                    self.trash.retain(|q| q.path != p.path);
                    self.write_favourite(p, false);
                    self.forget_deleted(p);
                }
                ok
            }
            _ => match library::move_to(&p.path, &self.pictures.join(library::TRASH)).ok().and_then(|t| Photo::from_path_at(t, self.offset)) {
                Some(t) => {
                    self.move_favourite(p, &t);
                    self.pref_set(&format!("del:{}", t.path.display()), &unix_now().to_string());
                    self.all.retain(|q| q.path != p.path);
                    self.recent.retain(|q| q.path != p.path);
                    self.trash.push(t);
                    true
                }
                None => false,
            },
        }
    }

    fn delete(&mut self, ui: &Ui, p: &Photo) {
        if self.remove_one(p) {
            self.after_removal(ui);
            self.rescan();
        } else {
            eprintln!("photos: could not delete {}", p.path.display());
        }
    }

    /// The selected photos of the grid, in grid order.
    fn selected_photos(&self) -> Vec<Photo> {
        self.view.iter().filter(|p| self.selection.contains(&p.path)).cloned().collect()
    }

    /// Deletes the selection (to Recently Deleted, or for good in it).
    fn delete_selected(&mut self, ui: &Ui) {
        let mut removed = 0;
        for p in self.selected_photos() {
            if self.remove_one(&p) {
                removed += 1;
            } else {
                eprintln!("photos: could not delete {}", p.path.display());
            }
        }
        self.selection.end();
        self.confirm_purge = false;
        self.rebuild(ui);
        if removed > 0 {
            self.rescan();
        }
    }

    /// Moves a trashed photo back to the library. True when it moved.
    fn recover_one(&mut self, p: &Photo) -> bool {
        match library::move_to(&p.path, &self.pictures).ok().and_then(|t| Photo::from_path_at(t, self.offset)) {
            Some(back) => {
                self.move_favourite(p, &back);
                self.forget_deleted(p);
                self.trash.retain(|q| q.path != p.path);
                let at = self.all.partition_point(|q| (q.mtime, &q.path) <= (back.mtime, &back.path));
                self.all.insert(at, back.clone());
                self.recent.push(back);
                true
            }
            None => {
                eprintln!("photos: could not recover {}", p.path.display());
                false
            }
        }
    }

    fn recover_selected(&mut self, ui: &Ui) {
        let mut moved = 0;
        for p in self.selected_photos() {
            if self.recover_one(&p) {
                moved += 1;
            }
        }
        self.selection.end();
        self.rebuild(ui);
        if moved > 0 {
            self.rescan();
        }
    }

    /// The viewer moves on to the next photo (or closes) after the current one left the album.
    fn after_removal(&mut self, ui: &Ui) {
        let index = self.viewer.as_ref().map(|v| v.index);
        self.rebuild(ui);
        self.viewer = None;
        let n = self.photos().len();
        if let Some(i) = index.filter(|_| n > 0) {
            self.open_viewer(i.min(n - 1));
        }
    }

    // --- editor ---

    fn edit_panel_top(&self, ui: &Ui) -> f32 {
        self.h as f32 - home_zone(ui) - ui.pt(EDIT_ROW_PT * EDIT_PANEL_ROWS)
    }

    /// The slider track's left and right ends in px.
    fn slider_track(&self, ui: &Ui) -> (f32, f32) {
        (ui.pt(120.0), self.w as f32 - ui.pt(MARGIN_PT + 8.0))
    }

    fn edit_hit(&self, ui: &Ui, x: f32, y: f32) -> Option<Target> {
        let w = self.w as f32;
        if y >= ui.pt(STATUS_PT) && y < content_top(ui) {
            return Some(if x < w / 3.0 {
                Target::EditCancel
            } else if x < w * 2.0 / 3.0 {
                Target::EditRevert
            } else {
                Target::EditSave
            });
        }
        let top = self.edit_panel_top(ui);
        if y < top || y >= top + ui.pt(EDIT_ROW_PT * EDIT_PANEL_ROWS) {
            return None;
        }
        let pane = self.editor.as_ref().map_or(EditPane::Adjust, |e| e.pane);
        match ((y - top) / ui.pt(EDIT_ROW_PT)) as usize {
            0 => match (x / (w / 7.0)) as usize {
                0 => Some(Target::EditRotate),
                col => Aspect::ALL.get(col - 1).map(|&a| Target::EditAspect(a)),
            },
            1 => Filter::ALL.get((x / (w / Filter::ALL.len() as f32)) as usize).map(|&f| Target::EditFilter(f)),
            2 => EditPane::ALL.get((x / (w / EditPane::ALL.len() as f32)) as usize).map(|&p| Target::EditPane(p)),
            row if pane == EditPane::Tools => match row {
                3 => [Tool::RotateLeft, Tool::FlipHorizontal, Tool::FlipVertical].get((x / (w / 3.0)) as usize).map(|&t| Target::EditTool(t)),
                4 => Some(Target::EditTool(Tool::AutoEnhance)),
                _ => None,
            },
            row => pane.sliders().get(row - 3).map(|&s| Target::Slide(s)),
        }
    }

    /// The slider's range in the editor.
    fn slider_range(slider: Slider) -> (f32, f32) {
        match slider {
            Slider::Brightness | Slider::Contrast | Slider::Exposure | Slider::Saturation | Slider::Warmth => (-1.0, 1.0),
            Slider::Blur | Slider::Vignette => (0.0, 1.0),
            Slider::Crop => (1.0, 4.0),
            Slider::Straighten => (-edit::STRAIGHTEN_MAX, edit::STRAIGHTEN_MAX),
        }
    }

    fn set_slider(&mut self, ui: &Ui, slider: Slider, x: f32) {
        let (x0, x1) = self.slider_track(ui);
        let (lo, hi) = Self::slider_range(slider);
        let mut v = edit::slider_value(x, x0, x1, lo, hi);
        let snap = if slider == Slider::Straighten { 0.5 } else { 0.04 };
        if slider != Slider::Crop && v.abs() < snap {
            v = 0.0;
        }
        if let Some(ed) = &mut self.editor {
            match slider {
                Slider::Brightness => ed.edit.brightness = v,
                Slider::Contrast => ed.edit.contrast = v,
                Slider::Exposure => ed.edit.exposure = v,
                Slider::Saturation => ed.edit.saturation = v,
                Slider::Warmth => ed.edit.warmth = v,
                Slider::Blur => ed.edit.blur = v,
                Slider::Vignette => ed.edit.vignette = v,
                Slider::Crop => ed.edit.crop_zoom = v,
                Slider::Straighten => ed.edit.straighten = v,
            }
            ed.refresh();
        }
        self.dirty = true;
    }

    // --- info sheet ---

    fn info_size(&self, ui: &Ui) -> (f32, f32) {
        let revert = usize::from(self.info_revert);
        (ui.pt(INFO_W_PT).min(self.w as f32 - 2.0 * ui.pt(MARGIN_PT)), ui.pt(INFO_ROW_PT) * (self.info.len() + 1 + 2 * usize::from(self.info_has_rating()) + revert) as f32)
    }

    /// The Info sheet's Hide Photo row (x, y, w, h), under the rating row.
    fn info_hide_row(&self, ui: &Ui) -> (f32, f32, f32, f32) {
        let ((x, y, w, h), _) = self.info_rating_row(ui);
        (x, y + h, w, h)
    }

    /// The Info sheet has a rating row and a Hide Photo row for every photo that is not in Recently Deleted.
    fn info_has_rating(&self) -> bool {
        self.album != Album::Trash
    }

    /// The Info sheet's rating row (x, y, w, h) and the span (x0, x1) its five stars share.
    fn info_rating_row(&self, ui: &Ui) -> ((f32, f32, f32, f32), (f32, f32)) {
        let (sx, sy, sw, _) = self.sheet_rect(ui, Sheet::Info);
        let row = ui.pt(INFO_ROW_PT);
        let rect = (sx, sy + row * (self.info.len() + 1) as f32, sw, row);
        (rect, (sx + sw - ui.pt(16.0) - ui.pt(32.0) * stars::MAX_STARS as f32, sx + sw - ui.pt(16.0)))
    }

    /// The star rating of `p` (0 = none).
    pub fn rating(&self, p: &Photo) -> u8 {
        self.prefs.get(&format!("rate:{}", p.path.display())).map_or(0, stars::parse)
    }

    fn set_rating(&mut self, p: &Photo, rating: u8) {
        let value = if rating == 0 { String::new() } else { rating.to_string() };
        let _ = self.pref_set(&format!("rate:{}", p.path.display()), &value);
    }

    /// The saved edit of `p`, when it has one (an unedited photo has none).
    fn recipe(&self, p: &Photo) -> Option<Edit> {
        self.prefs.get(&format!("edit:{}", p.path.display())).and_then(recipe::decode).filter(|e| !e.is_identity())
    }

    /// Keeps `edit` as the recipe of `p`; an unedited photo stores an empty value, which reads as no edit.
    fn set_recipe(&mut self, p: &Photo, edit: Edit) {
        let value = if edit.is_identity() { String::new() } else { recipe::encode(&edit) };
        let _ = self.pref_set(&format!("edit:{}", p.path.display()), &value);
    }

    /// Drops the cached pictures of `p` so the next request decodes it with its current edit.
    fn invalidate(&mut self, p: &Photo) {
        let key = p.key();
        self.thumbs.remove(&key);
        self.cells.remove(&key);
        self.full.remove(&key);
        self.thumb_failed.remove(&key);
        let _ = std::fs::remove_file(self.disk.path(&key));
    }

    /// "12 days left" for a photo in Recently Deleted.
    fn trash_note(&self, p: &Photo) -> String {
        let left = self.deleted_at(p).map_or(albums::TRASH_DAYS, |at| albums::days_left(at, unix_now()));
        format!("{left} day{} left", if left == 1 { "" } else { "s" })
    }

    // --- drawing ---

    fn label(&mut self, s: &mut Surface, ui: &Ui, text: &str, pt: f32, weight: Weight, color: u32, cx: f32, cy: f32) {
        let img = self.text.get(ui, text, pt, weight, color);
        let (w, h) = (img.w as i32, img.h as i32);
        s.blit(img, cx as i32 - w / 2, cy as i32 - h / 2, 255, 255);
    }

    fn label_left(&mut self, s: &mut Surface, ui: &Ui, text: &str, pt: f32, weight: Weight, color: u32, x: f32, cy: f32) {
        let img = self.text.get(ui, text, pt, weight, color);
        let h = img.h as i32;
        s.blit(img, x as i32, cy as i32 - h / 2, 255, 255);
    }

    pub fn draw(&mut self, s: &mut Surface, ui: &Ui, _t: f64) {
        self.dirty = false;
        s.fill_rect(0, 0, self.w as usize, self.h as usize, BLACK);
        if self.slideshow.is_some() {
            self.draw_slideshow(s, ui);
        } else if self.editor.is_some() {
            self.draw_editor(s, ui);
        } else if self.viewer.is_some() {
            self.draw_viewer(s, ui);
        } else {
            self.draw_grid(s, ui);
        }
    }

    /// Resamples the thumbnail of `key` to `size` px with a smooth filter (the
    /// drawing blit is nearest-neighbour, so it must not have to scale).
    fn prepare_cell(&mut self, key: &str, size: usize) {
        if self.cell_px != size {
            self.cells = MemCache::new(thumbs::MEM_BUDGET);
            self.cell_px = size;
        }
        if self.cells.contains(key) {
            return;
        }
        let Some(thumb) = self.thumbs.get(key) else { return };
        if thumb.w != size || thumb.h != size {
            let scaled = imgdec::cover(thumb, size, size);
            self.cells.insert(key, scaled);
        }
    }

    fn draw_grid(&mut self, s: &mut Surface, ui: &Ui) {
        let (top, bottom) = (content_top(ui), self.content_bottom(ui));
        let off = self.scroll.offset();
        let pressed = self.pressed;
        let empty = self.photos().is_empty();
        let selecting = self.selection.active;
        let keys: Vec<(usize, String, bool, bool)> = self.grid.visible(off, off + bottom - top).into_iter().filter_map(|i| self.photos().get(i).map(|p| (i, p.key(), self.album != Album::Trash && self.fav_paths.contains(&p.path), self.selection.contains(&p.path)))).collect();
        {
            let mut band = s.rows(top as usize, bottom as usize);
            for (si, y) in self.grid.headers(off, off + bottom - top) {
                let title = self.sections[si].title.clone();
                let hy = top + y - off + ui.pt(HEADER_PT) / 2.0 + ui.pt(4.0);
                self.label_left(&mut band, ui, &title, 22.0, Weight::Bold, WHITE, ui.pt(MARGIN_PT), hy);
            }
            for (i, key, favorite, picked) in keys {
                let (x, y, cw, ch) = self.grid.cell_rect(i);
                let r = Rect { x: x as i32, y: (top + y - off) as i32, w: cw as i32, h: ch as i32 };
                self.prepare_cell(&key, cw as usize);
                match self.cells.get(&key).or_else(|| self.thumbs.get(&key)) {
                    Some(img) => super::switcher::draw_scaled(&mut band, img, r, 0.0, 255),
                    None => band.fill_rect(r.x as usize, r.y as usize, r.w as usize, r.h as usize, CELL),
                }
                if pressed == Some(Target::Cell(i)) {
                    band.fill_rrect(r.x as f32, r.y as f32, r.w as f32, r.h as f32, 0.0, pack(0, 0, 0, 100));
                }
                if favorite {
                    let (cx, cy) = (r.x as f32 + ui.pt(13.0), (r.y + r.h) as f32 - ui.pt(13.0));
                    band.fill_rrect(cx - ui.pt(10.0), cy - ui.pt(10.0), ui.pt(20.0), ui.pt(20.0), ui.pt(10.0), rgba(0, 0, 0, 110));
                    heart_icon(&mut band, cx, cy, ui.pt(7.0), ui.pt(1.0), WHITE, true);
                }
                if selecting {
                    let (bs, flat) = (ui.pt(22.0), flat(ui));
                    let (bx, by) = ((r.x + r.w) as f32 - bs - ui.pt(5.0), (r.y + r.h) as f32 - bs - ui.pt(5.0));
                    if picked {
                        band.fill_rrect(r.x as f32, r.y as f32, r.w as f32, r.h as f32, 0.0, rgba(10, 132, 255, 70));
                        band.fill_rrect(bx, by, bs, bs, flat, BLUE);
                        let t = ui.pt(2.0);
                        for (a, b) in [((bx + bs * 0.24, by + bs * 0.52), (bx + bs * 0.43, by + bs * 0.72)), ((bx + bs * 0.43, by + bs * 0.72), (bx + bs * 0.78, by + bs * 0.3))] {
                            glyphs::paint(&mut band, (bx, by, bs, bs), WHITE, glyphs::segment(a, b, t));
                        }
                    } else {
                        band.fill_rrect(bx, by, bs, bs, flat, rgba(0, 0, 0, 120));
                        band.fill_rrect(bx + ui.pt(2.0), by + ui.pt(2.0), bs - ui.pt(4.0), bs - ui.pt(4.0), (flat - ui.pt(1.0)).max(0.0), rgba(255, 255, 255, 40));
                    }
                }
            }
        }
        if empty && !self.scanning {
            let cx = self.w as f32 / 2.0;
            if self.blank_library() {
                let cy = self.h as f32 / 2.0;
                self.label(s, ui, "No Photos or Videos", 22.0, Weight::Bold, EMPTY_STATE_GRAY, cx, cy - ui.pt(50.5));
                self.label(s, ui, "You can take photos and videos using the camera, or", 17.0, Weight::Regular, EMPTY_STATE_GRAY, cx, cy - ui.pt(22.0));
                self.label(s, ui, "copy photos and videos into Pictures using Files.", 17.0, Weight::Regular, EMPTY_STATE_GRAY, cx, cy);
                let (bx, by, bw, bh) = self.camera_button(ui);
                let (r, line) = (ui.pt(2.0), ui.pt(1.0));
                s.fill_rrect(bx, by, bw, bh, r, EMPTY_STATE_GRAY);
                s.fill_rrect(bx + line, by + line, bw - 2.0 * line, bh - 2.0 * line, (r - line).max(0.0), if pressed == Some(Target::OpenCamera) { rgba(255, 255, 255, 40) } else { BLACK });
                self.label(s, ui, "Open Camera", 17.0, Weight::Regular, EMPTY_STATE_GRAY, cx, by + bh / 2.0);
            } else {
                let cy = (top + bottom) / 2.0;
                let (title, hint) = match (self.album, &self.filter) {
                    (_, Some(_)) => ("No Photos", "Nothing matches the date filter. Tap the title to change it."),
                    (Album::All, _) => ("No Photos", "Screenshots and pictures you save appear here."),
                    (Album::Recents, _) => ("No Recent Photos", "Pictures from the last 30 days appear here."),
                    (Album::Favorites, _) => ("No Favorites", "Tap the heart on a photo to add it to Favorites."),
                    (Album::Screenshots, _) => ("No Screenshots", "Screenshots you take appear here."),
                    (Album::Trash, _) => ("No Recently Deleted Photos", "Deleted photos wait here for 30 days."),
                    (Album::Hidden, _) => ("No Hidden Photos", "Photos you hide from All Photos appear here."),
                    (Album::User(_), _) => ("No Photos", "Open a photo and tap Album to add it here."),
                };
                self.label(s, ui, title, 22.0, Weight::Bold, WHITE, cx, cy - ui.pt(14.0));
                self.label(s, ui, hint, 15.0, Weight::Regular, SECONDARY, cx, cy + ui.pt(14.0));
            }
        }
        // navigation bar over the content
        s.fill_rect(0, 0, self.w as usize, top as usize, BLACK);
        let mid = nav_mid(ui);
        let m = ui.pt(MARGIN_PT);
        let blank = self.blank_library();
        let title = if self.selection.active {
            match self.selection.len() {
                0 => "Select Items".to_string(),
                n => format!("{n} Selected"),
            }
        } else {
            self.filter.as_ref().map_or_else(|| if blank { "Library".to_string() } else { self.album_title(self.album) }, |f| f.0.clone())
        };
        let title_color = if self.selection.active { dim_blue(self.pressed, Target::SelectAll) } else if self.filter.is_some() { BLUE } else { WHITE };
        self.label(s, ui, &title, 17.0, Weight::Semibold, title_color, self.w as f32 / 2.0, mid);
        if blank {
            let tint = if pressed == Some(Target::Album) { rgba(10, 132, 255, 140) } else { BLUE };
            let (x, t) = (ui.pt(22.5), ui.pt(2.2));
            for (a, b) in [((x + ui.pt(10.5), mid - ui.pt(8.5)), (x, mid)), ((x, mid), (x + ui.pt(10.5), mid + ui.pt(8.5)))] {
                glyphs::paint(s, (x - ui.pt(2.0), mid - ui.pt(11.0), ui.pt(15.0), ui.pt(22.0)), tint, glyphs::segment(a, b, t));
            }
            self.label_left(s, ui, "Photos", 17.0, Weight::Regular, tint, ui.pt(41.5), mid);
            return self.finish_grid(s, ui);
        }
        let select = if self.selection.active { "Cancel" } else { "Select" };
        self.label(s, ui, select, 17.0, Weight::Regular, if pressed == Some(Target::Select) { rgba(10, 132, 255, 140) } else { BLUE }, self.w as f32 - ui.pt(MARGIN_PT + 96.0 + 38.0), mid);
        let dim = |p: Option<Target>, t: Target| if p == Some(t) { rgba(10, 132, 255, 140) } else { BLUE };
        self.label_left(s, ui, "Albums", 17.0, Weight::Regular, dim(pressed, Target::Album), m, mid);
        let cx = m + ui.pt(66.0);
        let cy = mid;
        for (a, b) in [((cx - ui.pt(4.0), cy - ui.pt(2.0)), (cx, cy + ui.pt(2.0))), ((cx, cy + ui.pt(2.0)), (cx + ui.pt(4.0), cy - ui.pt(2.0)))] {
            glyphs::paint(s, (cx - ui.pt(6.0), cy - ui.pt(6.0), ui.pt(12.0), ui.pt(12.0)), BLUE, glyphs::segment(a, b, ui.pt(1.5)));
        }
        let (zi, zo) = (self.cols > ZOOMS[ZOOMS.len() - 1], self.cols < ZOOMS[0]);
        let right = self.w as f32 - m;
        self.zoom_button(s, ui, right - ui.pt(22.0), mid, true, if zi { dim(pressed, Target::ZoomIn) } else { SEPARATOR });
        self.zoom_button(s, ui, right - ui.pt(74.0), mid, false, if zo { dim(pressed, Target::ZoomOut) } else { SEPARATOR });
        self.finish_grid(s, ui);
    }

    fn finish_grid(&mut self, s: &mut Surface, ui: &Ui) {
        if self.selection.active {
            self.draw_selection_bar(s, ui);
        }
        if self.album_menu {
            self.draw_albums(s, ui);
        }
    }

    /// The bottom bar of multi-select: share (or recover) and delete.
    fn draw_selection_bar(&mut self, s: &mut Surface, ui: &Ui) {
        let (w, bar_top) = (self.w as usize, self.bar_top(ui));
        s.fill_rect(0, bar_top as usize, w, self.content_bottom(ui) as usize - bar_top as usize, BAR);
        let cy = bar_top + ui.pt(BAR_PT) / 2.0;
        let pressed = self.pressed;
        let on = !self.selection.is_empty();
        let tint = |t: Target, base: u32| if !on { SEPARATOR } else if pressed == Some(t) { crate::gfx::scale(base, 140) } else { base };
        let wf = self.w as f32;
        if self.album == Album::Trash {
            self.label(s, ui, "Recover", 17.0, Weight::Regular, tint(Target::SelShare, BLUE), wf * 0.15, cy);
            let (text, color) = if self.confirm_purge { ("Delete Permanently?", RED) } else { ("Delete", RED) };
            self.label(s, ui, text, 17.0, Weight::Regular, tint(Target::SelDelete, color), wf * 0.8, cy);
        } else {
            share_icon(s, wf * 0.12, cy, ui.pt(22.0), ui.pt(2.0), tint(Target::SelShare, BLUE));
            self.label(s, ui, "Slideshow", 17.0, Weight::Regular, tint(Target::SelSlideshow, BLUE), wf * 0.5, cy);
            trash_icon(s, wf * 0.88, cy, ui.pt(22.0), ui.pt(2.0), tint(Target::SelDelete, BLUE));
        }
    }

    fn album_row(&self, ui: &Ui, album: Album) -> (f32, f32, f32, f32) {
        let row = self.menu_albums().iter().position(|&a| a == album).unwrap_or(0);
        let margin = ui.pt(MARGIN_PT);
        (margin, content_top(ui) + ui.pt(8.0 + row as f32 * 56.0), ui.pt(280.0).min(self.w as f32 - 2.0 * margin), ui.pt(56.0))
    }

    fn new_album_row(&self, ui: &Ui) -> (f32, f32, f32, f32) {
        let (x, y, w, h) = self.album_row(ui, Album::All);
        (x, y + h * self.menu_albums().len() as f32, w, h)
    }

    /// The picker's sort row, below the albums and the New Album row.
    fn sort_row(&self, ui: &Ui) -> (f32, f32, f32, f32) {
        let (x, y, w, h) = self.album_row(ui, Album::All);
        let rows = self.menu_albums().len() + usize::from(self.user_albums.len() < MAX_USER_ALBUMS);
        (x, y + h * rows as f32, w, h)
    }

    fn draw_albums(&mut self, s: &mut Surface, ui: &Ui) {
        s.fill_rrect(0.0, 0.0, self.w as f32, self.h as f32, 0.0, rgba(0, 0, 0, 80));
        let (x, y, w, row_h) = self.album_row(ui, Album::All);
        let menu = self.menu_albums();
        let can_add = self.user_albums.len() < MAX_USER_ALBUMS;
        s.fill_rrect(x, y, w, row_h * (menu.len() + usize::from(can_add) + 1) as f32, flat(ui), SHEET);
        if can_add {
            let (_, ny, _, _) = self.new_album_row(ui);
            if self.pressed == Some(Target::NewAlbum) {
                s.fill_rrect(x, ny, w, row_h, flat(ui), rgba(255, 255, 255, 20));
            }
            s.fill_rect((x + ui.pt(14.0)) as usize, ny as usize, (w - ui.pt(28.0)) as usize, 1, SEPARATOR);
            self.label_left(s, ui, "New Album", 17.0, Weight::Regular, BLUE, x + ui.pt(16.0), ny + row_h / 2.0);
        }
        for (index, album) in menu.into_iter().enumerate() {
            let (_, y, _, _) = self.album_row(ui, album);
            if self.pressed == Some(Target::ChooseAlbum(album)) {
                s.fill_rrect(x, y, w, row_h, flat(ui), rgba(255, 255, 255, 20));
            }
            if index > 0 {
                s.fill_rect((x + ui.pt(14.0)) as usize, y as usize, (w - ui.pt(28.0)) as usize, 1, SEPARATOR);
            }
            let cy = y + row_h / 2.0;
            self.label_left(s, ui, &self.album_title(album), 17.0, Weight::Regular, if album == self.album { BLUE } else { WHITE }, x + ui.pt(16.0), cy);
            let count = self.album_list(album).len();
            self.label(s, ui, &count.to_string(), 14.0, Weight::Regular, SECONDARY, x + w - ui.pt(22.0), cy);
        }
        let (_, sy, _, _) = self.sort_row(ui);
        if self.pressed == Some(Target::SortOrder) {
            s.fill_rrect(x, sy, w, row_h, flat(ui), rgba(255, 255, 255, 20));
        }
        s.fill_rect((x + ui.pt(14.0)) as usize, sy as usize, (w - ui.pt(28.0)) as usize, 1, SEPARATOR);
        let cy = sy + row_h / 2.0;
        self.label_left(s, ui, "Sort by Date", 17.0, Weight::Regular, WHITE, x + ui.pt(16.0), cy);
        let order = if self.newest_first { "Newest" } else { "Oldest" };
        self.label(s, ui, order, 14.0, Weight::Regular, SECONDARY, x + w - ui.pt(60.0), cy);
    }

    /// A round zoom button with a plus or minus.
    fn zoom_button(&self, s: &mut Surface, ui: &Ui, cx: f32, cy: f32, plus: bool, color: u32) {
        let r = ui.pt(16.0);
        glyphs::paint(s, (cx - r - 1.0, cy - r - 1.0, 2.0 * r + 2.0, 2.0 * r + 2.0), CELL, glyphs::disc((cx, cy), r));
        let (a, t) = (ui.pt(7.0), ui.pt(2.0));
        glyphs::paint(s, (cx - a - 2.0, cy - a - 2.0, 2.0 * a + 4.0, 2.0 * a + 4.0), color, glyphs::segment((cx - a, cy), (cx + a, cy), t));
        if plus {
            glyphs::paint(s, (cx - a - 2.0, cy - a - 2.0, 2.0 * a + 4.0, 2.0 * a + 4.0), color, glyphs::segment((cx, cy - a), (cx, cy + a), t));
        }
    }

    fn draw_viewer(&mut self, s: &mut Surface, ui: &Ui) {
        let (w, h) = (self.w as usize, self.h as usize);
        let Some(v) = &self.viewer else { return };
        let index = v.index;
        let around: Vec<(usize, Photo)> = (index.saturating_sub(1)..=index + 1).filter_map(|i| self.photos().get(i).map(|p| (i, p.clone()))).collect();
        let Some(v) = &mut self.viewer else { return };
        for (page, p) in &around {
            let (page, key) = (*page, p.key());
            let (img, fit) = match self.full.get(&key) {
                Some(img) => {
                    let fit = imgdec::fit_size(img, w, h);
                    (img, fit)
                }
                None => match self.thumbs.get(&key) {
                    Some(t) => {
                        let side = w.min(h);
                        (t, (side, side))
                    }
                    None => continue,
                },
            };
            if page == index {
                v.set_fit(fit.0 as f32, fit.1 as f32);
            }
            let (x, y, fw, fh) = v.frame(page, fit.0 as f32, fit.1 as f32);
            let r = Rect { x: x.round() as i32, y: y.round() as i32, w: fw.round() as i32, h: fh.round() as i32 };
            if (r.w as usize, r.h as usize) == (img.w, img.h) {
                s.blit(img, r.x, r.y, 255, 255);
            } else {
                super::switcher::draw_scaled(s, img, r, 0.0, 255);
            }
        }
        if !v.chrome {
            return;
        }
        let current = around.into_iter().find(|(i, _)| *i == index).map(|(_, p)| p);
        let top = content_top(ui);
        s.fill_rect(0, 0, w, top as usize, BAR);
        let bar_top = self.bar_top(ui);
        s.fill_rect(0, bar_top as usize, w, h - bar_top as usize, BAR);
        let pressed = self.pressed;
        let mid = nav_mid(ui);
        let m = ui.pt(MARGIN_PT);
        let back = if pressed == Some(Target::Back) { rgba(10, 132, 255, 140) } else { BLUE };
        self.label_left(s, ui, "‹", 34.0, Weight::Medium, back, m - ui.pt(6.0), mid - ui.pt(2.0));
        let back_label = self.album_title(self.album);
        self.label_left(s, ui, &back_label, 17.0, Weight::Regular, back, m + ui.pt(14.0), mid);
        if self.album != Album::Trash {
            let c = if pressed == Some(Target::AddToAlbum) { rgba(10, 132, 255, 140) } else { BLUE };
            let cur_in = current.as_ref().is_some_and(|p| matches!(self.album, Album::User(_)) || self.user_albums.iter().enumerate().any(|(i, _)| self.in_user_album(i, p)));
            self.label(s, ui, if cur_in { "Added" } else { "Album" }, 17.0, Weight::Regular, c, self.w as f32 - m - ui.pt(36.0), mid);
        }
        if let Some(p) = current {
            let d = civil(p.mtime, self.offset);
            let cx = self.w as f32 / 2.0;
            self.label(s, ui, &library::day_title(&d), 17.0, Weight::Semibold, WHITE, cx, mid - ui.pt(8.0));
            let sub = if self.album == Album::Trash { self.trash_note(&p) } else { library::time_title(&d) };
            self.label(s, ui, &sub, 13.0, Weight::Regular, SECONDARY, cx, mid + ui.pt(10.0));
            let fav = self.is_favourite(&p);
            let cy = bar_top + ui.pt(BAR_PT) / 2.0;
            for (target, cx) in self.bar_buttons() {
                let color = if pressed == Some(target) { rgba(10, 132, 255, 140) } else { BLUE };
                match target {
                    Target::Share => share_icon(s, cx, cy, ui.pt(22.0), ui.pt(2.0), color),
                    Target::Favourite => heart_icon(s, cx, cy, ui.pt(12.0), ui.pt(2.0), color, fav),
                    Target::Delete if self.album != Album::Trash => trash_icon(s, cx, cy, ui.pt(22.0), ui.pt(2.0), color),
                    Target::Delete => self.label(s, ui, "Delete", 17.0, Weight::Regular, if pressed == Some(target) { rgba(255, 69, 58, 140) } else { RED }, cx, cy),
                    Target::Recover => self.label(s, ui, "Recover", 17.0, Weight::Regular, color, cx, cy),
                    Target::Info => self.label(s, ui, "Info", 17.0, Weight::Regular, color, cx, cy),
                    Target::Edit => self.label(s, ui, "Edit", 17.0, Weight::Regular, color, cx, cy),
                    _ => {}
                }
            }
        }
        if let Some(sheet) = self.sheet {
            self.draw_sheet(s, ui, sheet);
        }
    }

    /// The EXIF card: a title row, then label and value rows.
    fn draw_info(&mut self, s: &mut Surface, ui: &Ui) {
        let (x, y, w, h) = self.sheet_rect(ui, Sheet::Info);
        s.fill_rrect(x, y, w, h, flat(ui), SHEET);
        let row = ui.pt(INFO_ROW_PT);
        self.label(s, ui, "Info", 17.0, Weight::Semibold, WHITE, x + w / 2.0, y + row / 2.0);
        let rows = self.info.clone();
        for (i, (label, value)) in rows.iter().enumerate() {
            let cy = y + row * (i + 1) as f32 + row / 2.0;
            s.fill_rect((x + ui.pt(14.0)) as usize, (cy - row / 2.0) as usize, (w - ui.pt(28.0)) as usize, 1, SEPARATOR);
            self.label_left(s, ui, label, 14.0, Weight::Regular, SECONDARY, x + ui.pt(16.0), cy);
            let text = self.text.get(ui, value, 15.0, Weight::Regular, WHITE);
            let (tw, th) = (text.w as i32, text.h as i32);
            let tx = (x + w - ui.pt(16.0)) as i32 - tw;
            s.blit(text, tx.max((x + ui.pt(100.0)) as i32), cy as i32 - th / 2, 255, 255);
        }
        let rated = 2 * usize::from(self.info_has_rating());
        if rated == 2 {
            let cy = y + row * (rows.len() + 1) as f32 + row / 2.0;
            s.fill_rect((x + ui.pt(14.0)) as usize, (cy - row / 2.0) as usize, (w - ui.pt(28.0)) as usize, 1, SEPARATOR);
            self.label_left(s, ui, "Rating", 14.0, Weight::Regular, SECONDARY, x + ui.pt(16.0), cy);
            let have = self.viewing().map_or(0, |p| self.rating(p));
            let (_, (x0, x1)) = self.info_rating_row(ui);
            let slot = (x1 - x0) / stars::MAX_STARS as f32;
            let r = ui.pt(11.0);
            for i in 0..stars::MAX_STARS {
                let (scx, on) = (x0 + slot * (i as f32 + 0.5), i < have);
                let (inner, t) = (0.5, ui.pt(1.4));
                let pad = r + 2.0;
                glyphs::paint(s, (scx - pad, cy - pad, 2.0 * pad, 2.0 * pad), if on { BLUE } else { SECONDARY }, move |px, py| {
                    let d = stars::star_distance(px - scx, py - cy, r, inner);
                    if on { d } else { d.abs() - t / 2.0 }
                });
            }
        }
        if rated == 2 {
            let cy = y + row * (rows.len() + 2) as f32 + row / 2.0;
            s.fill_rect((x + ui.pt(14.0)) as usize, (cy - row / 2.0) as usize, (w - ui.pt(28.0)) as usize, 1, SEPARATOR);
            let hidden = self.viewing().is_some_and(|p| self.is_hidden(p));
            let color = if self.pressed == Some(Target::InfoHide) { crate::gfx::scale(BLUE, 140) } else { BLUE };
            self.label(s, ui, if hidden { "Unhide Photo" } else { "Hide Photo" }, 15.0, Weight::Regular, color, x + w / 2.0, cy);
        }
        if self.info_revert {
            let cy = y + row * (rows.len() + 1 + rated) as f32 + row / 2.0;
            s.fill_rect((x + ui.pt(14.0)) as usize, (cy - row / 2.0) as usize, (w - ui.pt(28.0)) as usize, 1, SEPARATOR);
            let color = if self.pressed == Some(Target::InfoRevert) { crate::gfx::scale(BLUE, 140) } else { BLUE };
            self.label(s, ui, "Revert to Original", 15.0, Weight::Regular, color, x + w / 2.0, cy);
        }
    }

    /// The edit screen: the live preview and the tool panel.
    fn draw_editor(&mut self, s: &mut Surface, ui: &Ui) {
        let (w, top, panel) = (self.w as f32, content_top(ui), self.edit_panel_top(ui));
        let Some((cur, preview)) = self.editor.as_ref().map(|e| (e.edit, (e.preview.w as f32, e.preview.h as f32))) else { return };
        if let Some(ed) = &self.editor {
            let (aw, ah) = (w, panel - top);
            let k = (aw / preview.0.max(1.0)).min(ah / preview.1.max(1.0));
            let (dw, dh) = ((preview.0 * k).round(), (preview.1 * k).round());
            let r = Rect { x: ((w - dw) / 2.0) as i32, y: (top + (ah - dh) / 2.0) as i32, w: dw as i32, h: dh as i32 };
            super::switcher::draw_scaled(s, &ed.preview, r, 0.0, 255);
        }
        s.fill_rect(0, 0, w as usize, top as usize, BAR);
        s.fill_rect(0, panel as usize, w as usize, self.h as usize - panel as usize, BAR);
        let (mid, m, pressed) = (nav_mid(ui), ui.pt(MARGIN_PT), self.pressed);
        let tint = |t: Target, c: u32| if pressed == Some(t) { crate::gfx::scale(c, 140) } else { c };
        if cur.is_identity() {
            self.label(s, ui, "Edit", 17.0, Weight::Semibold, WHITE, w / 2.0, mid);
        } else {
            self.label(s, ui, "Revert", 17.0, Weight::Regular, tint(Target::EditRevert, RED), w / 2.0, mid);
        }
        self.label(s, ui, "Cancel", 17.0, Weight::Regular, tint(Target::EditCancel, BLUE), m + ui.pt(28.0), mid);
        self.label(s, ui, "Save", 17.0, Weight::Semibold, tint(Target::EditSave, BLUE), w - m - ui.pt(20.0), mid);
        let row = ui.pt(EDIT_ROW_PT);
        let col = w / 7.0;
        self.label(s, ui, "Rotate", 15.0, Weight::Regular, tint(Target::EditRotate, BLUE), col / 2.0, panel + row / 2.0);
        for (i, a) in Aspect::ALL.into_iter().enumerate() {
            let color = if cur.aspect == a { BLUE } else { WHITE };
            self.label(s, ui, a.label(), 15.0, if cur.aspect == a { Weight::Semibold } else { Weight::Regular }, tint(Target::EditAspect(a), color), col * (i as f32 + 1.5), panel + row / 2.0);
        }
        let (x0, x1) = self.slider_track(ui);
        let chips = Filter::ALL.len() as f32;
        for (i, f) in Filter::ALL.into_iter().enumerate() {
            let color = if cur.filter == f { BLUE } else { WHITE };
            self.label(s, ui, f.label(), 15.0, if cur.filter == f { Weight::Semibold } else { Weight::Regular }, tint(Target::EditFilter(f), color), w / chips * (i as f32 + 0.5), panel + row * 1.5);
        }
        let pane = self.editor.as_ref().map_or(EditPane::Adjust, |e| e.pane);
        let pane_count = EditPane::ALL.len() as f32;
        for (i, p) in EditPane::ALL.into_iter().enumerate() {
            let color = if pane == p { BLUE } else { WHITE };
            self.label(s, ui, p.label(), 15.0, if pane == p { Weight::Semibold } else { Weight::Regular }, tint(Target::EditPane(p), color), w / pane_count * (i as f32 + 0.5), panel + row * 2.5);
        }
        for (i, &slider) in pane.sliders().iter().enumerate().take(EDIT_SLIDER_ROWS) {
            let cy = panel + row * (i as f32 + 3.5);
            self.label_left(s, ui, slider.label(), 15.0, Weight::Regular, WHITE, m, cy);
            s.fill_rect(x0 as usize, cy as usize - 1, (x1 - x0) as usize, 2, SEPARATOR);
            let (lo, hi) = Self::slider_range(slider);
            let kx = edit::slider_pos(slider.value(&cur), x0, x1, lo, hi);
            s.fill_rrect(kx - ui.pt(5.0), cy - ui.pt(11.0), ui.pt(10.0), ui.pt(22.0), flat(ui), WHITE);
        }
        if pane == EditPane::Tools {
            let third = w / 3.0;
            for (i, t) in [Tool::RotateLeft, Tool::FlipHorizontal, Tool::FlipVertical].into_iter().enumerate() {
                let color = if t.is_on(&cur) { BLUE } else { WHITE };
                self.label(s, ui, t.label(), 15.0, if t.is_on(&cur) { Weight::Semibold } else { Weight::Regular }, tint(Target::EditTool(t), color), third * (i as f32 + 0.5), panel + row * 3.5);
            }
            let auto = Tool::AutoEnhance;
            let color = if auto.is_on(&cur) { BLUE } else { WHITE };
            self.label(s, ui, auto.label(), 15.0, if auto.is_on(&cur) { Weight::Semibold } else { Weight::Regular }, tint(Target::EditTool(auto), color), w / 2.0, panel + row * 4.5);
            // the histogram fills the rows below the buttons
            if let Some(ed) = &self.editor {
                let (hx, hy, hw, hh) = (m, panel + row * 5.0 + ui.pt(6.0), w - 2.0 * m, row * 3.0 - ui.pt(12.0));
                s.fill_rrect(hx, hy, hw, hh, flat(ui), rgba(255, 255, 255, 20));
                for (bx, h) in histogram_columns(&ed.hist, hw as usize / 2).into_iter().enumerate() {
                    let bar = (h * hh) as usize;
                    if bar > 0 {
                        s.fill_rect(hx as usize + bx * 2, (hy + hh) as usize - bar, 2, bar, WHITE);
                    }
                }
            }
        }
    }

    /// Vertical centre of the slideshow's play/pause row.
    fn show_bar_y(&self, ui: &Ui) -> f32 {
        self.h as f32 - home_zone(ui) - ui.pt(BAR_PT)
    }

    fn start_slideshow(&mut self, photos: Vec<Photo>, index: usize, t: f64) {
        self.sheet = None;
        self.slideshow = Slideshow::new(photos, index);
        if let Some(sh) = &mut self.slideshow {
            sh.show_controls(t);
        }
        self.request_full();
    }

    /// Share › Copy: the open picture goes on the shared clipboard.
    fn copy_current(&mut self) {
        let Some(p) = self.viewing().cloned() else { return };
        let recipe = self.recipe(&p);
        let img = self.full.get(&p.key()).cloned().or_else(|| std::fs::read(&p.path).ok().and_then(|b| decode_edited(&b, recipe)));
        if let (Some(img), Some(c)) = (img, super::clipboard::global().as_mut()) {
            c.set_image(img);
        }
    }

    /// Share › Save to Files: a copy of the picture in the Files folder.
    /// Duplicate: copies the open photo next to the original under a numbered name (its edit and rating come
    /// along) and rescans so the copy shows.
    fn duplicate_current(&mut self) {
        let Some(p) = self.viewing().cloned() else { return };
        match library::duplicate(&p.path) {
            Ok(copy) => {
                let (from, to) = (p.path.display().to_string(), copy.display().to_string());
                for key in ["edit", "rate"] {
                    if let Some(v) = self.prefs.get(&format!("{key}:{from}")).map(str::to_string).filter(|v| !v.is_empty()) {
                        let _ = self.pref_set(&format!("{key}:{to}"), &v);
                    }
                }
                self.rescan();
            }
            Err(e) => eprintln!("photos: could not duplicate {}: {e}", p.name()),
        }
    }

    fn save_to_files(&mut self) {
        let Some(p) = self.viewing().cloned() else { return };
        let dir = self.data.join("Files");
        let name = p.name();
        // like the share sheet's Save to Files: a second copy gets a numbered name, nothing is overwritten
        let ok = std::fs::create_dir_all(&dir).is_ok() && crate::net::share::create_unique(&dir, &name).and_then(|(_, mut out)| std::io::copy(&mut std::fs::File::open(&p.path)?, &mut out)).is_ok();
        if !ok {
            eprintln!("photos: could not save {name} to Files");
        }
    }

    fn draw_slideshow(&mut self, s: &mut Surface, ui: &Ui) {
        let (w, h) = (self.w as usize, self.h as usize);
        let Some(sh) = &self.slideshow else { return };
        let layers: Vec<(Photo, f32, u32, usize)> = sh.layers().into_iter().map(|(i, p, a)| (sh.photos[i].clone(), p, a, i)).collect();
        let (paused, controls) = (sh.paused, sh.controls);
        for (photo, progress, alpha, i) in layers {
            let Some(img) = self.full.get(&photo.key()) else { continue };
            let fit = imgdec::fit_size(img, w, h);
            let (x, y, fw, fh) = slideshow::ken_burns(i, progress, (fit.0 as f32, fit.1 as f32), (w as f32, h as f32));
            let r = Rect { x: x.round() as i32, y: y.round() as i32, w: fw.round() as i32, h: fh.round() as i32 };
            super::switcher::draw_scaled(s, img, r, 0.0, alpha);
        }
        if !controls {
            return;
        }
        let top = content_top(ui);
        s.fill_rect(0, 0, w, top as usize, BAR);
        let bar_top = self.bar_top(ui);
        s.fill_rect(0, bar_top as usize, w, h - bar_top as usize, BAR);
        let pressed = self.pressed;
        let col = |t: Target| if pressed == Some(t) { rgba(10, 132, 255, 140) } else { BLUE };
        self.label_left(s, ui, "Done", 17.0, Weight::Semibold, col(Target::ShowDone), ui.pt(MARGIN_PT), nav_mid(ui));
        let (cx, cy, k) = (w as f32 / 2.0, self.show_bar_y(ui) + ui.pt(BAR_PT) / 2.0, ui.pt(10.0));
        let step = ui.pt(96.0);
        for (t, bx, dir) in [(Target::ShowPrev, cx - step, -1.0f32), (Target::ShowNext, cx + step, 1.0)] {
            // a double triangle pointing `dir`
            for off in [-0.6f32, 0.6] {
                let tip = bx + dir * k * (0.5 + off);
                for row in 0..(2.0 * k) as i32 {
                    let d = (row as f32 - k).abs() / k;
                    let len = k * (1.0 - d);
                    s.fill_rect((tip - if dir > 0.0 { len } else { 0.0 }).max(0.0) as usize, (cy - k + row as f32) as usize, len.max(1.0) as usize, 1, col(t));
                }
            }
        }
        if paused {
            for row in 0..(2.0 * k) as i32 {
                let len = k * 1.6 * (1.0 - (row as f32 - k).abs() / k);
                s.fill_rect((cx - k * 0.6) as usize, (cy - k + row as f32) as usize, len.max(1.0) as usize, 1, col(Target::ShowPlay));
            }
        } else {
            for dx in [-0.6f32, 0.6] {
                s.fill_rect((cx + dx * k - k * 0.2) as usize, (cy - k) as usize, (k * 0.4).max(2.0) as usize, (2.0 * k) as usize, col(Target::ShowPlay));
            }
        }
    }

    fn draw_sheet(&mut self, s: &mut Surface, ui: &Ui, sheet: Sheet) {
        s.fill_rrect(0.0, 0.0, self.w as f32, self.h as f32, 0.0, rgba(0, 0, 0, 90));
        if sheet == Sheet::Info {
            return self.draw_info(s, ui);
        }
        let (x, y, w, h) = self.sheet_rect(ui, sheet);
        let r = ui.pt(13.0);
        s.fill_rrect(x, y, w, h, r, SHEET);
        let rows = sheet_rows(sheet);
        for i in 1..rows {
            s.fill_rect(x as usize, (y + h * i as f32 / rows as f32) as usize, w as usize, 1, SEPARATOR);
        }
        let (action, color) = match (sheet, self.album) {
            (Sheet::Share, _) => ("Set as Wallpaper", BLUE),
            (Sheet::Delete, Album::Trash) => ("Delete Permanently", RED),
            (Sheet::Delete | Sheet::Info, _) => ("Delete Photo", RED),
        };
        let pressed = self.pressed;
        let dim = |c: u32, t: Target| if pressed == Some(t) { crate::gfx::scale(c, 140) } else { c };
        let cx = x + w / 2.0;
        let row = h / rows as f32;
        let mut at = y + row / 2.0;
        if sheet == Sheet::Share {
            for (text, t) in [("Slideshow", Target::SheetSlideshow), ("Copy", Target::SheetCopy), ("Save to Files", Target::SheetFiles), ("Send Nearby", Target::SheetNearby), ("Duplicate", Target::SheetDuplicate)] {
                self.label(s, ui, text, 20.0, Weight::Regular, dim(BLUE, t), cx, at);
                at += row;
            }
        }
        self.label(s, ui, action, 20.0, Weight::Regular, dim(color, Target::SheetAction), cx, at);
        self.label(s, ui, "Cancel", 20.0, Weight::Semibold, dim(BLUE, Target::SheetCancel), cx, at + row);
    }
}

/// Blue, dimmed while `target` is the pressed one.
fn dim_blue(pressed: Option<Target>, target: Target) -> u32 {
    if pressed == Some(target) { rgba(10, 132, 255, 140) } else { BLUE }
}

/// The histogram squeezed into `columns` bars with heights 0..=1 (the tallest bar is 1). `columns` of 0 gives none.
pub fn histogram_columns(bins: &[u32; edit::HISTOGRAM_BINS], columns: usize) -> Vec<f32> {
    if columns == 0 {
        return Vec::new();
    }
    let sums: Vec<u64> = (0..columns).map(|c| bins[c * bins.len() / columns..((c + 1) * bins.len() / columns).max(c * bins.len() / columns + 1).min(bins.len())].iter().map(|&b| b as u64).sum()).collect();
    let max = sums.iter().copied().max().unwrap_or(0).max(1) as f32;
    sums.into_iter().map(|v| v as f32 / max).collect()
}

/// Rows in a sheet: the share sheet lists Slideshow, Copy, Save to Files,
/// Send Nearby, Duplicate, Set as Wallpaper and Cancel.
fn sheet_rows(sheet: Sheet) -> usize {
    if sheet == Sheet::Share {
        7
    } else {
        2
    }
}

/// The share symbol: an open box with an arrow rising out of it.
fn share_icon(s: &mut Surface, cx: f32, cy: f32, size: f32, t: f32, color: u32) {
    let (half, top) = (size / 2.0, cy - size * 0.45);
    let box_top = cy - size * 0.1;
    // box: left, bottom, right (the top edge is open around the arrow)
    let bx = cx - half * 0.75;
    let by = cy + half;
    for (a, b) in [((bx, box_top), (bx, by)), ((bx, by), (cx + half * 0.75, by)), ((cx + half * 0.75, by), (cx + half * 0.75, box_top)), ((bx, box_top), (bx + half * 0.4, box_top)), ((cx + half * 0.75 - half * 0.4, box_top), (cx + half * 0.75, box_top))] {
        glyphs::paint(s, (cx - half - t, top - t, size + 2.0 * t, size + 2.0 * t), color, glyphs::segment(a, b, t));
    }
    // arrow
    let shaft = size * 0.72;
    for (a, b) in [((cx, top), (cx, top + shaft)), ((cx, top), (cx - size * 0.22, top + size * 0.22)), ((cx, top), (cx + size * 0.22, top + size * 0.22))] {
        glyphs::paint(s, (cx - half - t, top - t, size + 2.0 * t, size + 2.0 * t), color, glyphs::segment(a, b, t));
    }
}

/// A heart of radius `r` (filled when `fill`, else stroked `t` wide), from
/// the implicit curve (x² + y² − 1)³ − x² y³ = 0 with a gradient-normalised
/// distance for the anti-aliasing.
fn heart_icon(s: &mut Surface, cx: f32, cy: f32, r: f32, t: f32, color: u32, fill: bool) {
    let k = 1.25 / r;
    let dist = move |px: f32, py: f32| {
        let (x, y) = ((px - cx) * k, -(py - cy - r * 0.15) * k);
        let q = x * x + y * y - 1.0;
        let f = q * q * q - x * x * y * y * y;
        let gx = 6.0 * x * q * q - 2.0 * x * y * y * y;
        let gy = 6.0 * y * q * q - 3.0 * x * x * y * y;
        let d = f / (gx * gx + gy * gy).sqrt().max(1e-3) / k;
        if fill {
            d
        } else {
            d.abs() - t / 2.0
        }
    };
    let pad = r * 1.6;
    glyphs::paint(s, (cx - pad, cy - pad, 2.0 * pad, 2.0 * pad), color, dist);
}

/// A waste bin: lid with a handle over a tapering body with two slats.
fn trash_icon(s: &mut Surface, cx: f32, cy: f32, size: f32, t: f32, color: u32) {
    let half = size / 2.0;
    let top = cy - half;
    let lid = top + size * 0.18;
    let bottom = cy + half;
    let lines = [
        ((cx - half * 0.9, lid), (cx + half * 0.9, lid)),
        ((cx - half * 0.3, top), (cx + half * 0.3, top)),
        ((cx - half * 0.3, top), (cx - half * 0.3, lid)),
        ((cx + half * 0.3, top), (cx + half * 0.3, lid)),
        ((cx - half * 0.72, lid), (cx - half * 0.6, bottom)),
        ((cx + half * 0.72, lid), (cx + half * 0.6, bottom)),
        ((cx - half * 0.6, bottom), (cx + half * 0.6, bottom)),
        ((cx - half * 0.25, lid + size * 0.2), (cx - half * 0.2, bottom - size * 0.15)),
        ((cx + half * 0.25, lid + size * 0.2), (cx + half * 0.2, bottom - size * 0.15)),
    ];
    for (a, b) in lines {
        glyphs::paint(s, (cx - half - t, top - t, size + 2.0 * t, size + 2.0 * t), color, glyphs::segment(a, b, t));
    }
}

/// Decodes a picture and turns it upright by its EXIF orientation.
fn decode_upright(bytes: &[u8]) -> Option<Image> {
    crate::cg::imageio::decode(bytes).map(|img| edit::orient(img, exif::orientation(bytes)))
}

/// Decodes a picture upright and applies its saved edit, when it has one.
fn decode_edited(bytes: &[u8], recipe: Option<Edit>) -> Option<Image> {
    let img = decode_upright(bytes)?;
    Some(match recipe {
        Some(edit) => edit.apply(&img),
        None => img,
    })
}

/// The first bytes of a file (where a JPEG keeps its EXIF).
fn read_head(path: &Path) -> Vec<u8> {
    use std::io::Read;
    let mut out = Vec::new();
    if let Ok(f) = std::fs::File::open(path) {
        let _ = f.take(EXIF_HEAD).read_to_end(&mut out);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::library::tests::{picture, scratch};
    use super::*;
    use crate::shell::tests::ui;

    /// The embedded 8x8 fixture with an EXIF block of the given orientation spliced in.
    fn oriented_jpeg(orientation: u16) -> Vec<u8> {
        use super::exif::tests::{short, tiff};
        let dot = include_bytes!("../../deildu/fixtures/dot.jpg");
        let t = tiff(true, vec![short(true, 0x0112, orientation)], vec![], vec![]);
        let mut out = dot[..2].to_vec();
        out.extend([0xFF, 0xE1]);
        out.extend(((t.len() + 8) as u16).to_be_bytes());
        out.extend(b"Exif\0\0");
        out.extend(&t);
        out.extend(&dot[2..]);
        out
    }

    #[test]
    fn decoding_applies_the_exif_orientation() {
        let plain = crate::cg::imageio::decode(include_bytes!("../../deildu/fixtures/dot.jpg")).unwrap();
        assert_eq!(decode_upright(&oriented_jpeg(1)).unwrap(), plain);
        assert_eq!(decode_upright(&oriented_jpeg(3)).unwrap(), edit::rotate(&plain, 2));
        assert_eq!(decode_upright(&oriented_jpeg(6)).unwrap(), edit::rotate(&plain, 1));
    }

    /// A Photos app over a scratch library of `n` pictures a day apart.
    pub(super) fn photos(name: &str, n: usize) -> (Photos, PathBuf, Ui) {
        let ui = ui();
        let dir = scratch(name);
        let pics = dir.join("Pictures");
        for i in 0..n {
            picture(&pics.join(format!("IMG_{i:04}.png")), 1_790_000_000 + i as i64 * 86_400);
        }
        let mut p = Photos::new(&ui, 1200, 1920, dir.clone(), vec![pics, dir.join("missing")], 0).unwrap();
        pump_until(&mut p, &ui, |p| !p.scanning);
        (p, dir, ui)
    }

    pub(super) fn pump_until(p: &mut Photos, ui: &Ui, done: impl Fn(&Photos) -> bool) {
        let start = std::time::Instant::now();
        while !done(p) && start.elapsed().as_secs() < 30 {
            crate::bare::sys::wait(&p.fds(), 50);
            p.pump(ui);
        }
        assert!(done(p), "timed out: {}", p.describe());
    }

    pub(super) fn tap(p: &mut Photos, ui: &Ui, x: f32, y: f32, t: f64) {
        p.touch(ui, Event::Down(x, y), t);
        p.touch(ui, Event::Up(x, y), t + 30.0);
    }

    pub(super) fn choose_album(p: &mut Photos, ui: &Ui, album: Album, t: f64) {
        tap(p, ui, ui.pt(40.0), nav_mid(ui), t);
        assert!(p.album_menu);
        let (x, y, w, h) = p.album_row(ui, album);
        tap(p, ui, x + w / 2.0, y + h / 2.0, t + 100.0);
        assert_eq!(p.album(), album);
        assert!(!p.album_menu);
    }

    pub(super) fn frame(p: &mut Photos, ui: &Ui) -> Vec<u32> {
        let mut buf = vec![0u32; 1200 * 1920];
        p.draw(&mut Surface { w: 1200, h: 1920, stride: 1200, px: &mut buf, y0: 0 }, ui, 0.0);
        buf
    }

    #[test]
    fn scans_into_month_sections_and_starts_at_the_bottom() {
        let (mut p, dir, ui) = photos("sections", 12);
        assert_eq!(p.photos().len(), 12);
        assert_eq!(p.sections.iter().map(|s| s.title.as_str()).collect::<Vec<_>>(), ["September 2026", "October 2026"]);
        assert_eq!(p.cols(), 5);
        assert_eq!(p.scroll.offset(), 0.0, "twelve photos fit on one screen");
        assert!(p.describe().contains("photos_screen=grid"));
        // thumbnails are decoded for the visible cells and cached on disk
        p.tick(&ui, 0.0);
        pump_until(&mut p, &ui, |p| p.thumbs.len() == 12);
        assert_eq!(p.disk.len(), 12);
        assert!(p.thumb_pending.is_empty());
        let buf = frame(&mut p, &ui);
        let (x, y, cw, ch) = p.grid.cell_rect(0);
        let px = buf[(content_top(&ui) + y + ch / 2.0) as usize * 1200 + (x + cw / 2.0) as usize];
        assert_eq!(px, pack(10, 20, 30, 255), "the thumbnail's colour fills its cell");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn thumbnail_requests_cover_the_visible_cells_and_drop_the_rest() {
        let (mut p, dir, ui) = photos("wanted", 12);
        p.tick(&ui, 0.0);
        let visible: HashSet<String> = p.grid.visible(0.0, p.content_bottom(&ui) - content_top(&ui)).into_iter().map(|i| p.photos()[i].key()).collect();
        assert_eq!(*p.thumb_wanted.lock().unwrap_or_else(|p| p.into_inner()), visible);
        assert_eq!(visible.len(), 12);
        // the caching manager holds one preparation per visible thumbnail, at the grid's thumbnail size
        assert_eq!(p.preheat.len(), 12);
        assert!(p.preheat.is_started(&PreKey::new(&p.photos()[0].key(), thumbs::SIZE as u32, thumbs::SIZE as u32)));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn revert_to_original_drops_the_saved_edit_and_never_touches_the_file() {
        let (mut p, dir, ui) = photos("revert", 1);
        let original = dir.join("Pictures/IMG_0000.png");
        p.open_viewer(0);
        frame(&mut p, &ui);
        let photo = p.photos()[0].clone();
        p.set_recipe(&photo, Edit { turns: 1, ..Edit::default() });
        assert!(p.recipe(&photo).is_some());
        let cy = p.bar_top(&ui) + ui.pt(BAR_PT) / 2.0;
        tap(&mut p, &ui, 1200.0 * 0.31, cy, 1000.0);
        assert_eq!(p.sheet, Some(Sheet::Info));
        assert!(p.info_revert, "an edited photo offers Revert to Original");
        // the Revert row sits under the detail rows
        let (sx, sy, sw, _) = p.sheet_rect(&ui, Sheet::Info);
        let row = ui.pt(INFO_ROW_PT);
        let y = sy + row * (p.info.len() + 1) as f32 + row / 2.0;
        tap(&mut p, &ui, sx + sw / 2.0, y, 2000.0);
        assert!(p.recipe(&photo).is_none(), "the saved edit is dropped");
        assert!(original.exists(), "the original is never changed");
        assert_eq!(p.photos().len(), 1, "nothing leaves the library");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_long_library_opens_scrolled_to_the_newest_row() {
        let (mut p, dir, ui) = photos("long", 60);
        assert!(p.grid.height > 1920.0);
        assert_eq!(p.scroll.offset(), p.grid.height - (p.content_bottom(&ui) - content_top(&ui)));
        // zoom in: three columns, position kept proportionally
        let k = p.scroll.offset() / p.grid.height;
        tap(&mut p, &ui, 1200.0 - ui.pt(MARGIN_PT + 22.0), nav_mid(&ui), 100.0);
        assert_eq!(p.cols(), 3);
        assert!((p.scroll.offset() / p.grid.height - k).abs() < 0.05);
        tap(&mut p, &ui, 1200.0 - ui.pt(MARGIN_PT + 22.0), nav_mid(&ui), 200.0);
        assert_eq!(p.cols(), 2);
        tap(&mut p, &ui, 1200.0 - ui.pt(MARGIN_PT + 22.0), nav_mid(&ui), 250.0);
        assert_eq!(p.cols(), 2, "no further zoom level");
        assert_eq!(p.prefs.get("cols"), Some("2"), "the level is remembered");
        tap(&mut p, &ui, 1200.0 - ui.pt(MARGIN_PT + 74.0), nav_mid(&ui), 300.0);
        tap(&mut p, &ui, 1200.0 - ui.pt(MARGIN_PT + 74.0), nav_mid(&ui), 350.0);
        assert_eq!(p.cols(), 5);
        tap(&mut p, &ui, 1200.0 - ui.pt(MARGIN_PT + 74.0), nav_mid(&ui), 400.0);
        tap(&mut p, &ui, 1200.0 - ui.pt(MARGIN_PT + 74.0), nav_mid(&ui), 450.0);
        assert_eq!(p.cols(), 8, "more, smaller thumbnails");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn tapping_a_cell_opens_the_viewer_and_back_returns() {
        let (mut p, dir, ui) = photos("viewer", 3);
        let (x, y, cw, ch) = p.grid.cell_rect(1);
        tap(&mut p, &ui, x + cw / 2.0, content_top(&ui) + y + ch / 2.0, 1000.0);
        assert_eq!(p.viewing().map(Photo::name).as_deref(), Some("IMG_0001.png"));
        assert!(p.describe().contains("photos_screen=viewer"));
        p.tick(&ui, 1100.0);
        pump_until(&mut p, &ui, |p| p.full.len() == 3);
        let buf = frame(&mut p, &ui);
        assert_eq!(buf[960 * 1200 + 600], pack(10, 20, 30, 255), "the picture is centred");
        assert!(!p.fullscreen());
        // a single tap hides the chrome once the double-tap window passes
        tap(&mut p, &ui, 600.0, 900.0, 2000.0);
        p.tick(&ui, 2400.0);
        assert!(p.fullscreen());
        assert_eq!(p.next_wake(), None);
        tap(&mut p, &ui, 600.0, 900.0, 3000.0);
        p.tick(&ui, 3400.0);
        assert!(!p.fullscreen());
        tap(&mut p, &ui, ui.pt(40.0), nav_mid(&ui), 4000.0);
        assert!(p.viewer.is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn favourite_persists_in_the_prefs_file() {
        let (mut p, dir, ui) = photos("fav", 2);
        p.open_viewer(0);
        let photo = p.viewing().cloned().unwrap();
        assert!(!p.is_favourite(&photo));
        let cy = p.bar_top(&ui) + ui.pt(BAR_PT) / 2.0;
        tap(&mut p, &ui, 600.0, cy, 1000.0);
        assert!(p.is_favourite(&photo));
        let again = Prefs::load(dir.join(PREFS_FILE));
        assert_eq!(again.get(&format!("fav:{}", photo.path.display())), Some("1"));
        tap(&mut p, &ui, 600.0, cy, 2000.0);
        assert!(!p.is_favourite(&photo));
        frame(&mut p, &ui);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn delete_moves_to_recently_deleted_where_it_can_be_recovered_or_purged() {
        let (mut p, dir, ui) = photos("delete", 3);
        p.open_viewer(2);
        let cy = p.bar_top(&ui) + ui.pt(BAR_PT) / 2.0;
        tap(&mut p, &ui, 1200.0 * 0.88, cy, 1000.0);
        assert_eq!(p.sheet, Some(Sheet::Delete));
        assert!(p.describe().contains("photos_screen=sheet"));
        frame(&mut p, &ui);
        let (sx, sy, sw, sh) = p.sheet_rect(&ui, Sheet::Delete);
        tap(&mut p, &ui, sx + sw / 2.0, sy + 3.0 * sh / 4.0, 1100.0);
        assert_eq!(p.sheet, None, "cancelled");
        assert_eq!(p.photos().len(), 3);
        tap(&mut p, &ui, 1200.0 * 0.88, cy, 1200.0);
        tap(&mut p, &ui, sx + sw / 2.0, sy + sh / 4.0, 1300.0);
        assert_eq!(p.photos().len(), 2);
        assert_eq!(p.viewing().map(Photo::name).as_deref(), Some("IMG_0001.png"), "the viewer moves to the last remaining");
        assert!(dir.join("Pictures/.trash/IMG_0002.png").exists());
        // Recently Deleted
        p.viewer = None;
        choose_album(&mut p, &ui, Album::Trash, 2000.0);
        assert_eq!(p.album(), Album::Trash);
        assert_eq!(p.photos().len(), 1);
        frame(&mut p, &ui);
        p.open_viewer(0);
        frame(&mut p, &ui);
        tap(&mut p, &ui, 1200.0 * 0.15, cy, 3000.0);
        assert!(dir.join("Pictures/IMG_0002.png").exists(), "recovered");
        assert_eq!((p.all.len(), p.trash.len()), (3, 0));
        assert!(p.viewer.is_none(), "nothing left to view in the trash");
        frame(&mut p, &ui);
        // delete for good
        choose_album(&mut p, &ui, Album::All, 4000.0);
        p.open_viewer(0);
        tap(&mut p, &ui, 1200.0 * 0.88, cy, 5000.0);
        tap(&mut p, &ui, sx + sw / 2.0, sy + sh / 4.0, 5100.0);
        assert_eq!(p.all.len(), 2, "trashed again");
        p.viewer = None;
        choose_album(&mut p, &ui, Album::Trash, 6000.0);
        assert_eq!(p.album(), Album::Trash);
        p.open_viewer(0);
        tap(&mut p, &ui, 1200.0 * 0.85, cy, 7000.0);
        let (sx, sy, sw, sh) = p.sheet_rect(&ui, Sheet::Delete);
        tap(&mut p, &ui, sx + sw / 2.0, sy + sh / 4.0, 7100.0);
        assert!(p.trash.is_empty() && !dir.join("Pictures/.trash/IMG_0000.png").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn set_as_wallpaper_packs_the_photo_and_emits_the_effect() {
        let (mut p, dir, ui) = photos("wallpaper", 1);
        p.open_viewer(0);
        let cy = p.bar_top(&ui) + ui.pt(BAR_PT) / 2.0;
        tap(&mut p, &ui, 1200.0 * 0.12, cy, 1000.0);
        assert_eq!(p.sheet, Some(Sheet::Share));
        frame(&mut p, &ui);
        let (sx, sy, sw, sh) = p.sheet_rect(&ui, Sheet::Share);
        tap(&mut p, &ui, sx + sw / 2.0, sy + sh * 5.5 / 7.0, 1100.0);
        pump_until(&mut p, &ui, |p| !p.effects.is_empty());
        assert_eq!(p.take_effects(), vec![Effect::Wallpaper("custom-1".into())]);
        let raw = std::fs::read(dir.join("wallpapers/custom-1.raw")).unwrap();
        let img = Image::from_raw(&raw).unwrap();
        assert_eq!((img.w, img.h), (1200, 1920));
        assert_eq!(img.px[0], pack(10, 20, 30, 255));
        assert!(Image::from_raw(&std::fs::read(dir.join("wallpapers/custom-1.thumb.raw")).unwrap()).is_some());
        assert_eq!(Prefs::load(dir.join("wallpapers/index.ini")).get("custom-1"), Some("Photo 1"));
        assert_eq!(p.next_wallpaper_id(), "custom-2");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn duplicate_copies_the_photo_with_its_edit_and_rating() {
        let (mut p, dir, ui) = photos("duplicate", 1);
        p.open_viewer(0);
        let photo = p.photos()[0].clone();
        p.set_recipe(&photo, Edit { turns: 1, ..Edit::default() });
        p.set_rating(&photo, 4);
        let cy = p.bar_top(&ui) + ui.pt(BAR_PT) / 2.0;
        tap(&mut p, &ui, 1200.0 * 0.12, cy, 1000.0);
        frame(&mut p, &ui);
        let (sx, sy, sw, sh) = p.sheet_rect(&ui, Sheet::Share);
        tap(&mut p, &ui, sx + sw / 2.0, sy + sh * 4.5 / 7.0, 1100.0);
        assert_eq!(p.sheet, None);
        pump_until(&mut p, &ui, |p| !p.scanning && p.all.len() == 2);
        let copy = dir.join("Pictures/IMG_0000 2.png");
        assert!(copy.exists() && dir.join("Pictures/IMG_0000.png").exists());
        let dup = p.all.iter().find(|q| q.path == copy).cloned().expect("the copy is in the library");
        assert_eq!(p.rating(&dup), 4);
        assert_eq!(p.recipe(&dup).map(|e| e.turns), Some(1));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_info_sheet_rates_with_stars() {
        let (mut p, dir, ui) = photos("rating", 1);
        p.open_viewer(0);
        let cy = p.bar_top(&ui) + ui.pt(BAR_PT) / 2.0;
        tap(&mut p, &ui, 1200.0 * 0.31, cy, 1000.0);
        frame(&mut p, &ui);
        let photo = p.photos()[0].clone();
        assert_eq!(p.rating(&photo), 0);
        let ((rx, ry, _, rh), (x0, x1)) = p.info_rating_row(&ui);
        let star = |n: f32| x0 + (x1 - x0) * (n - 0.5) / 5.0;
        tap(&mut p, &ui, star(4.0), ry + rh / 2.0, 1100.0);
        assert_eq!(p.sheet, Some(Sheet::Info), "the sheet stays open");
        assert_eq!(p.rating(&photo), 4);
        frame(&mut p, &ui);
        tap(&mut p, &ui, star(4.0), ry + rh / 2.0, 1200.0);
        assert_eq!(p.rating(&photo), 0, "the same star clears it");
        tap(&mut p, &ui, star(1.0), ry + rh / 2.0, 1300.0);
        assert_eq!(p.rating(&photo), 1);
        assert!(rx < x0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn hiding_a_photo_moves_it_from_all_photos_to_the_hidden_album() {
        let (mut p, dir, ui) = photos("hidden", 3);
        p.open_viewer(0);
        let photo = p.photos()[0].clone();
        let cy = p.bar_top(&ui) + ui.pt(BAR_PT) / 2.0;
        tap(&mut p, &ui, 1200.0 * 0.31, cy, 1000.0);
        frame(&mut p, &ui);
        let (hx, hy, hw, hh) = p.info_hide_row(&ui);
        tap(&mut p, &ui, hx + hw / 2.0, hy + hh / 2.0, 1100.0);
        assert!(p.is_hidden(&photo));
        assert_eq!((p.visible.len(), p.hidden.len(), p.all.len()), (2, 1, 3));
        assert!(!p.album_list(Album::All).iter().any(|q| q.path == photo.path));
        assert!(p.menu_albums().contains(&Album::Hidden));
        assert!(!p.favorites.iter().chain(&p.recents).any(|q| q.path == photo.path));
        p.viewer = None;
        p.select_album(&ui, Album::Hidden);
        assert_eq!(p.photos().len(), 1);
        p.open_viewer(0);
        tap(&mut p, &ui, 1200.0 * 0.31, cy, 1200.0);
        frame(&mut p, &ui);
        let (hx, hy, hw, hh) = p.info_hide_row(&ui);
        tap(&mut p, &ui, hx + hw / 2.0, hy + hh / 2.0, 1300.0);
        assert_eq!((p.visible.len(), p.hidden.len()), (3, 0));
        assert_eq!(p.album(), Album::All, "an empty Hidden album closes");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn send_nearby_hands_the_photo_path_to_the_shell() {
        let (mut p, dir, ui) = photos("nearby", 1);
        p.open_viewer(0);
        let cy = p.bar_top(&ui) + ui.pt(BAR_PT) / 2.0;
        tap(&mut p, &ui, 1200.0 * 0.12, cy, 1000.0);
        frame(&mut p, &ui);
        let (sx, sy, sw, sh) = p.sheet_rect(&ui, Sheet::Share);
        tap(&mut p, &ui, sx + sw / 2.0, sy + sh * 3.5 / 7.0, 1100.0);
        assert_eq!(p.sheet, None);
        assert_eq!(p.take_share(), p.photos().first().map(|q| q.path.clone()));
        assert_eq!(p.take_share(), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn share_sheet_slideshow_plays_and_pauses() {
        let (mut p, dir, ui) = photos("slideshow", 2);
        p.open_viewer(0);
        let cy = p.bar_top(&ui) + ui.pt(BAR_PT) / 2.0;
        tap(&mut p, &ui, 1200.0 * 0.12, cy, 1000.0);
        frame(&mut p, &ui);
        let (sx, sy, sw, sh) = p.sheet_rect(&ui, Sheet::Share);
        tap(&mut p, &ui, sx + sw / 2.0, sy + sh / 14.0, 1100.0);
        assert!(p.slideshow.is_some());
        p.activate(&ui, Target::ShowPlay, 1200.0);
        assert!(p.slideshow.as_ref().unwrap().paused);
        p.activate(&ui, Target::ShowDone, 1300.0);
        assert!(p.slideshow.is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_new_screenshot_shows_at_once_and_opens_by_path() {
        let (mut p, dir, ui) = photos("shot", 2);
        let path = dir.join("Pictures/Screenshot-20261004-002315.png");
        let shot = Photo { path: path.clone(), mtime: 1_800_000_000, size: 9 };
        let img = Image { w: 1200, h: 1920, px: vec![pack(1, 2, 3, 255); 1200 * 1920] };
        p.add_photo(&ui, shot, img);
        assert_eq!(p.photos().len(), 3);
        assert!(p.thumbs.contains(&p.photos()[2].key()) && p.full.len() == 1);
        p.rescan();
        pump_until(&mut p, &ui, |p| !p.scanning);
        assert_eq!(p.photos().len(), 3, "a scan that cannot see the file yet keeps it");
        assert!(p.show(&ui, &path));
        assert_eq!(p.viewing().map(|q| q.path.clone()), Some(path));
        let buf = frame(&mut p, &ui);
        assert_eq!(buf[960 * 1200 + 600], pack(1, 2, 3, 255));
        assert!(!p.show(&ui, Path::new("/nowhere.png")));
        let empty = Photos::new(&ui, 1200, 1920, dir.join("empty"), vec![dir.join("empty/Pictures")], 0).unwrap();
        let mut empty = empty;
        pump_until(&mut empty, &ui, |p| !p.scanning);
        frame(&mut empty, &ui);
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn select_button(ui: &Ui) -> f32 {
        1200.0 - ui.pt(MARGIN_PT + 96.0 + 38.0)
    }

    fn cell_centre(p: &Photos, ui: &Ui, i: usize) -> (f32, f32) {
        let (x, y, cw, ch) = p.grid.cell_rect(i);
        (x + cw / 2.0, content_top(ui) + y - p.scroll.offset() + ch / 2.0)
    }

    #[test]
    fn screenshots_and_recents_are_albums_of_their_own() {
        let (mut p, dir, ui) = photos("albums", 3);
        picture(&dir.join("Pictures/Screenshot-20261004-002315.png"), 1_791_000_000);
        p.rescan();
        pump_until(&mut p, &ui, |p| !p.scanning);
        assert_eq!(p.album_list(Album::All).len(), 4);
        choose_album(&mut p, &ui, Album::Screenshots, 1000.0);
        assert_eq!(p.photos().iter().map(Photo::name).collect::<Vec<_>>(), ["Screenshot-20261004-002315.png"]);
        frame(&mut p, &ui);
        choose_album(&mut p, &ui, Album::Recents, 2000.0);
        assert!(p.photos().len() <= 4, "only what is within 30 days of now");
        choose_album(&mut p, &ui, Album::All, 3000.0);
        assert_eq!(p.photos().len(), 4);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_title_cycles_through_months_and_dates_can_be_typed() {
        let (mut p, dir, ui) = photos("filter", 12);
        assert_eq!(p.photos().len(), 12);
        tap(&mut p, &ui, 600.0, nav_mid(&ui), 1000.0);
        assert_eq!(p.filter.as_ref().map(|f| f.0.as_str()), Some("October 2026"));
        assert_eq!(p.photos().len(), 2);
        assert!(p.describe().contains("photos_filter=October 2026"));
        frame(&mut p, &ui);
        tap(&mut p, &ui, 600.0, nav_mid(&ui), 2000.0);
        assert_eq!(p.photos().len(), 10, "September");
        tap(&mut p, &ui, 600.0, nav_mid(&ui), 3000.0);
        assert!(p.filter.is_none());
        assert_eq!(p.photos().len(), 12);
        assert!(p.set_date_query(&ui, "2026-09-25"));
        assert_eq!(p.photos().len(), 1);
        assert!(!p.set_date_query(&ui, "not a date"));
        assert_eq!(p.photos().len(), 1, "a bad query leaves the filter");
        p.clear_filter(&ui);
        assert_eq!(p.photos().len(), 12);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn multi_select_deletes_to_recently_deleted_with_a_purge_clock() {
        let (mut p, dir, ui) = photos("select", 4);
        tap(&mut p, &ui, select_button(&ui), nav_mid(&ui), 1000.0);
        assert!(p.selection.active);
        let (x, y) = cell_centre(&p, &ui, 0);
        tap(&mut p, &ui, x, y, 1100.0);
        assert!(p.viewer.is_none(), "taps select while selecting");
        let (x, y) = cell_centre(&p, &ui, 2);
        tap(&mut p, &ui, x, y, 1200.0);
        assert_eq!(p.selection.len(), 2);
        assert!(p.describe().contains("photos_selected=2"));
        frame(&mut p, &ui);
        let bar_y = p.bar_top(&ui) + ui.pt(BAR_PT) / 2.0;
        tap(&mut p, &ui, 1200.0 * 0.88, bar_y, 1300.0);
        pump_until(&mut p, &ui, |p| !p.scanning && p.all.len() == 2);
        assert_eq!(p.trash.len(), 2);
        assert!(!p.selection.active);
        assert!(dir.join("Pictures/.trash/IMG_0000.png").exists() && dir.join("Pictures/.trash/IMG_0002.png").exists());
        let t = p.trash[0].clone();
        assert!(p.deleted_at(&t).is_some_and(|at| (unix_now() - at).abs() < 60));
        assert_eq!(p.trash_note(&t), "30 days left");
        // an old deletion is purged at the next scan; a recent one stays
        let old = p.trash[0].clone();
        p.pref_set(&format!("del:{}", old.path.display()), &(unix_now() - 31 * 86_400).to_string());
        p.rescan();
        pump_until(&mut p, &ui, |p| !p.scanning);
        assert_eq!(p.trash.len(), 1);
        assert!(!old.path.exists());
        // Recently Deleted: select and recover
        choose_album(&mut p, &ui, Album::Trash, 5000.0);
        tap(&mut p, &ui, select_button(&ui), nav_mid(&ui), 5100.0);
        let (x, y) = cell_centre(&p, &ui, 0);
        tap(&mut p, &ui, x, y, 5200.0);
        frame(&mut p, &ui);
        tap(&mut p, &ui, 1200.0 * 0.15, bar_y, 5300.0);
        pump_until(&mut p, &ui, |p| !p.scanning && p.all.len() == 3);
        assert!(p.trash.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn tapping_the_selection_title_selects_and_deselects_everything() {
        let (mut p, dir, ui) = photos("selectall", 4);
        tap(&mut p, &ui, select_button(&ui), nav_mid(&ui), 1000.0);
        tap(&mut p, &ui, 600.0, nav_mid(&ui), 1100.0);
        assert_eq!(p.selection.len(), 4);
        tap(&mut p, &ui, 600.0, nav_mid(&ui), 1200.0);
        assert!(p.selection.is_empty() && p.selection.active, "Deselect All keeps selecting");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn permanent_multi_delete_asks_twice() {
        let (mut p, dir, ui) = photos("purge2", 2);
        p.delete(&ui, &p.photos()[0].clone());
        pump_until(&mut p, &ui, |p| !p.scanning && p.trash.len() == 1);
        choose_album(&mut p, &ui, Album::Trash, 1000.0);
        tap(&mut p, &ui, select_button(&ui), nav_mid(&ui), 1100.0);
        let (x, y) = cell_centre(&p, &ui, 0);
        tap(&mut p, &ui, x, y, 1200.0);
        let bar_y = p.bar_top(&ui) + ui.pt(BAR_PT) / 2.0;
        tap(&mut p, &ui, 1200.0 * 0.8, bar_y, 1300.0);
        assert!(p.confirm_purge && p.trash.len() == 1, "the first tap only asks");
        frame(&mut p, &ui);
        tap(&mut p, &ui, 1200.0 * 0.8, bar_y, 1400.0);
        pump_until(&mut p, &ui, |p| !p.scanning && p.trash.is_empty());
        assert!(!dir.join("Pictures/.trash/IMG_0000.png").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_info_sheet_lists_the_file_details_and_closes_on_a_tap() {
        let (mut p, dir, ui) = photos("info", 1);
        p.open_viewer(0);
        let cy = p.bar_top(&ui) + ui.pt(BAR_PT) / 2.0;
        tap(&mut p, &ui, 1200.0 * 0.31, cy, 1000.0);
        assert_eq!(p.sheet, Some(Sheet::Info));
        let labels: Vec<&str> = p.info.iter().map(|(l, _)| l.as_str()).collect();
        assert_eq!(labels, ["Name", "Date", "File Size"], "a PNG has no EXIF");
        assert_eq!(p.info[0].1, "IMG_0000.png");
        frame(&mut p, &ui);
        tap(&mut p, &ui, 100.0, 100.0, 2000.0);
        assert_eq!(p.sheet, None);
        assert!(p.viewer.is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_editor_previews_edits_and_saves_a_recipe() {
        let (mut p, dir, ui) = photos("editor", 1);
        p.open_viewer(0);
        p.tick(&ui, 1000.0);
        pump_until(&mut p, &ui, |p| p.full.len() == 1);
        let cy = p.bar_top(&ui) + ui.pt(BAR_PT) / 2.0;
        tap(&mut p, &ui, 1200.0 * 0.69, cy, 2000.0);
        assert!(p.editor.is_some() && p.describe().contains("photos_screen=editor"));
        frame(&mut p, &ui);
        let row = ui.pt(EDIT_ROW_PT);
        let top = p.edit_panel_top(&ui);
        let col = 1200.0 / 7.0;
        // 16:9 is the sixth preset: the 4x4 picture crops to 4x2
        tap(&mut p, &ui, col * 6.5, top + row / 2.0, 2100.0);
        assert_eq!(p.editor.as_ref().map(|e| (e.preview.w, e.preview.h)), Some((4, 2)));
        tap(&mut p, &ui, col / 2.0, top + row / 2.0, 2200.0);
        assert_eq!(p.editor.as_ref().map(|e| e.edit.turns), Some(1));
        assert_eq!(p.editor.as_ref().map(|e| (e.preview.w, e.preview.h)), Some((4, 2)), "a square rotates into itself and keeps the 16:9 crop");
        // brightness slider: the right end is +1
        let (_, x1) = p.slider_track(&ui);
        tap(&mut p, &ui, x1, top + row * 3.5, 2300.0);
        assert_eq!(p.editor.as_ref().map(|e| e.edit.brightness), Some(1.0));
        // the Effects tab: its first slider is blur, and the right end is the full radius
        tap(&mut p, &ui, 1200.0 / 4.0 * 1.5, top + row * 2.5, 2350.0);
        assert_eq!(p.editor.as_ref().map(|e| e.pane), Some(EditPane::Effects));
        tap(&mut p, &ui, x1, top + row * 3.5, 2360.0);
        assert_eq!(p.editor.as_ref().map(|e| (e.edit.blur, e.edit.brightness)), Some((1.0, 1.0)));
        frame(&mut p, &ui);
        // Save
        tap(&mut p, &ui, 1100.0, ui.pt(STATUS_PT) + 5.0, 2400.0);
        assert!(p.editor.is_none());
        let photo = p.photos()[0].clone();
        assert!(p.recipe(&photo).is_some_and(|e| e.turns == 1 && e.aspect == Aspect::R16x9), "the edit is saved as a recipe");
        assert!(dir.join("Pictures/IMG_0000.png").exists(), "the original stays");
        assert_eq!(p.all.len(), 1, "no copy is written");
        // Cancel leaves nothing behind
        p.open_viewer(0);
        p.tick(&ui, 3000.0);
        pump_until(&mut p, &ui, |p| p.full.len() >= 1);
        tap(&mut p, &ui, 1200.0 * 0.69, cy, 3100.0);
        tap(&mut p, &ui, 100.0, ui.pt(STATUS_PT) + 5.0, 3200.0);
        assert!(p.editor.is_none() && p.viewer.is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_tools_page_flips_rotates_and_enhances() {
        let (mut p, dir, ui) = photos("tools", 1);
        p.open_viewer(0);
        p.tick(&ui, 1000.0);
        pump_until(&mut p, &ui, |p| p.full.len() == 1);
        let cy = p.bar_top(&ui) + ui.pt(BAR_PT) / 2.0;
        tap(&mut p, &ui, 1200.0 * 0.69, cy, 2000.0);
        let (row, top) = (ui.pt(EDIT_ROW_PT), p.edit_panel_top(&ui));
        tap(&mut p, &ui, 1200.0 / 4.0 * 3.5, top + row * 2.5, 2100.0);
        assert_eq!(p.editor.as_ref().map(|e| e.pane), Some(EditPane::Tools));
        tap(&mut p, &ui, 1200.0 / 6.0, top + row * 3.5, 2200.0);
        assert_eq!(p.editor.as_ref().map(|e| e.edit.turns), Some(3), "Rotate Left is a quarter back");
        tap(&mut p, &ui, 600.0, top + row * 3.5, 2300.0);
        tap(&mut p, &ui, 1000.0, top + row * 3.5, 2400.0);
        tap(&mut p, &ui, 600.0, top + row * 4.5, 2500.0);
        let e = p.editor.as_ref().map(|e| e.edit).unwrap();
        assert!(e.flip_h && e.flip_v && e.auto);
        tap(&mut p, &ui, 600.0, top + row * 3.5, 2600.0);
        assert_eq!(p.editor.as_ref().map(|e| e.edit.flip_h), Some(false), "a second tap turns the flip off");
        assert!(p.editor.as_ref().is_some_and(|e| e.hist.iter().sum::<u32>() > 0), "the histogram follows the preview");
        frame(&mut p, &ui);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn histogram_columns_scale_to_the_tallest() {
        let mut bins = [0u32; edit::HISTOGRAM_BINS];
        bins[0] = 10;
        bins[255] = 5;
        let cols = histogram_columns(&bins, 128);
        assert_eq!((cols.len(), cols[0], cols[127]), (128, 1.0, 0.5));
        assert!(histogram_columns(&bins, 0).is_empty());
        assert_eq!(histogram_columns(&bins, 256).len(), 256);
        assert_eq!(histogram_columns(&[0; edit::HISTOGRAM_BINS], 4), vec![0.0; 4]);
    }

    #[test]
    fn at_most_three_full_size_pictures_stay_decoded() {
        let (mut p, dir, ui) = photos("budget", 7);
        p.open_viewer(2);
        p.tick(&ui, 1000.0);
        pump_until(&mut p, &ui, |p| p.full.len() == 3);
        p.open_viewer(5);
        p.tick(&ui, 2000.0);
        let last = p.photos()[6].key();
        pump_until(&mut p, &ui, |p| p.full.contains(&last));
        assert_eq!(p.full.len(), 3, "the oldest were evicted");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
