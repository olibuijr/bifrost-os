//! Desktop mode: apps run in floating windows over the wallpaper, with a dock at the bottom.
//!
//! This file holds the window model and its geometry (pure, unit-tested) and the shell side that
//! draws it and routes touches. Every app is a singleton, so there is one window per app. Each
//! window's app is laid out for the window body (the shell's `w`/`h` are swapped to the body size
//! while it draws or takes a touch), drawn into an offscreen buffer and copied in.

use super::ui::{Weight, BLACK, SECONDARY, SYSTEM_GREEN, SYSTEM_RED, SYSTEM_YELLOW, TRAFFIC_INACTIVE_C, WHITE, TRAFFIC_GLYPH, MENU_TINT, MENU_RADIUS_PT, SEPARATOR_C, FIELD_BG, LABEL, MAC_BODY_PT};
use super::desk_occlusion::{self, grown, opaque_interior, Cover};
use super::{apphost, apps, window_fx, AppId, Effect, Event, Screen, Shell};
#[cfg(test)]
use crate::gfx::pack;
use crate::gfx::{rgba, Image, Surface};

/// Title bar height, traffic-light size and spacing, in pt.
const TITLE_PT: f32 = 30.0;
const LIGHT_PT: f32 = 14.0;
const LIGHT_GAP_PT: f32 = 9.0;
const LIGHT_X_PT: f32 = 13.0;
/// Resize handle thickness around a window, in pt.
const EDGE_PT: f32 = 6.0;
/// Smallest window body, in pt.
const MIN_W_PT: f32 = 320.0;
const MIN_H_PT: f32 = 220.0;
/// Dock metrics, in pt.
const DOCK_PAD_PT: f32 = 10.0;
const DOCK_GAP_PT: f32 = 10.0;
const DOCK_MARGIN_PT: f32 = 10.0;
const DOCK_SEP_PT: f32 = 14.0;
/// How far past the Dock a dragged icon must be released to leave it, in pt.
const DOCK_REMOVE_PT: f32 = 40.0;
/// The dock's Trash slot in the `desk_dock` list (after the divider, always last).
pub(super) const DOCK_TRASH: usize = usize::MAX;
/// The Files app the Trash opens, and the XDG trash it shows.
const FILES_APP: &str = "is.olibuijr.files.desktop";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Rect {
    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x as f32 && x < (self.x + self.w) as f32 && y >= self.y as f32 && y < (self.y + self.h) as f32
    }
}

/// Where a window can be tiled in the work area (Window menu, Move & Resize, and edge snapping).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tile {
    Left, Right, Top, Bottom, TopLeft, TopRight, BottomLeft, BottomRight, Fill, Center,
}

/// Whether the Wayland compositor owns window positions (the desktop session): the shell can maximize and restore a
/// window there, but cannot move it to a half or the centre.
#[cfg(feature = "desktop-wayland")]
pub fn compositor_places_windows() -> bool {
    cfg!(feature = "desktop-wayland") && std::env::var_os("SPRINGBOARD_WAYLAND").is_some()
}

/// The rectangle of `tile` inside the work area `a`; Centre keeps two thirds of each side.
pub fn tile_rect(a: Rect, tile: Tile) -> Rect {
    let (hw, hh) = (a.w / 2, a.h / 2);
    let (rw, rh) = (a.w - hw, a.h - hh);
    let r = |x, y, w, h| Rect { x: a.x + x, y: a.y + y, w, h };
    match tile {
        Tile::Left => r(0, 0, hw, a.h),
        Tile::Right => r(hw, 0, rw, a.h),
        Tile::Top => r(0, 0, a.w, hh),
        Tile::Bottom => r(0, hh, a.w, rh),
        Tile::TopLeft => r(0, 0, hw, hh),
        Tile::TopRight => r(hw, 0, rw, hh),
        Tile::BottomLeft => r(0, hh, hw, rh),
        Tile::BottomRight => r(hw, hh, rw, rh),
        Tile::Fill => a,
        Tile::Center => r(a.w / 6, a.h / 6, a.w * 2 / 3, a.h * 2 / 3),
    }
}

/// Split view: two windows share the work area side by side with a `divider` px gap between them. `lead_permille`
/// (0..=1000) is the leading window's share of the usable width; each side keeps at least `min_w` px, so the ratio
/// gives way on a narrow area. The rectangles always cover the area minus the divider.
#[cfg_attr(not(test), allow(dead_code))] // wired by the split view entry (next round)
pub fn split_view_rects(a: Rect, lead_permille: i32, divider: i32, min_w: i32) -> (Rect, Rect) {
    let usable = (a.w - divider).max(0);
    let lo = min_w.max(0).min(usable / 2);
    let lead = (usable * lead_permille.clamp(0, 1000) / 1000).clamp(lo, usable - lo);
    let trail = usable - lead;
    (Rect { x: a.x, y: a.y, w: lead, h: a.h }, Rect { x: a.x + lead + divider, y: a.y, w: trail, h: a.h })
}

/// The tile a pointer at (`px`,`py`) snaps to while dragging a title bar: left or right edge = half,
/// top edge = fill, corners = quarters. The band is `band` px wide (derive it from `ui.pt`).
pub fn snap_zone(a: Rect, px: i32, py: i32, band: i32) -> Option<Tile> {
    let (l, r, t) = (px <= a.x + band, px >= a.x + a.w - band, py <= a.y + band);
    let b = py >= a.y + a.h - band;
    match (l, r, t, b) {
        (true, _, true, _) => Some(Tile::TopLeft),
        (_, true, true, _) => Some(Tile::TopRight),
        (true, _, _, true) => Some(Tile::BottomLeft),
        (_, true, _, true) => Some(Tile::BottomRight),
        (true, ..) => Some(Tile::Left),
        (_, true, ..) => Some(Tile::Right),
        (_, _, true, _) => Some(Tile::Fill),
        _ => None,
    }
}

/// One window: `r` is the outer rectangle (title bar included).
#[derive(Clone, Copy, Debug)]
pub struct Win {
    pub app: AppId,
    pub r: Rect,
    pub maximized: bool,
    pub minimized: bool,
    /// the rectangle to go back to when un-maximized
    restore: Rect,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Light {
    Close,
    Minimize,
    Zoom,
}

/// Which part of a window a point is on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
    Light(Light),
    Title,
    Content,
    /// resize handles: bit 0 left, 1 right, 2 top, 3 bottom
    Edge(u8),
}

const L: u8 = 1;
const R: u8 = 2;
const T: u8 = 4;
const B: u8 = 8;

#[derive(Clone, Copy, Debug)]
enum Grab {
    Move { dx: f32, dy: f32 },
    Resize { edges: u8, start: Rect, x: f32, y: f32 },
    /// a press on a traffic light that acts on release
    Light(usize, Light),
    DockDrag{from:usize,x:f32,y:f32,at_x:f32,at_y:f32,moved:bool},
    /// the touch belongs to this window's app until it lifts
    Content(usize),
}

#[cfg(feature = "desktop-wayland")]
pub(super) struct LinuxDockApp { pub(super) app_id:String, pub(super) windows:Vec<crate::bare::wayland::ForeignWindow>, icon:Image }

#[cfg(feature = "desktop-wayland")]
struct CachedWindow {
    width:usize, height:usize, pixels:std::sync::Arc<[u32]>,
    focused:bool, maximized:bool, hover:bool, drawn:f64,
}

/// Two title-bar presses this close (ms) are a double-click, which zooms the window as Golden Gate does.
const DOUBLE_CLICK_MS: f64 = 400.0;

#[derive(Default)]
pub struct Desk {
    /// back to front
    pub wins: Vec<Win>,
    pub(super) dock_preferences:super::desktop_dock::Preferences,
    pub(super) dock_visible:bool,
    /// the window whose traffic lights the pointer is over; its lights show their glyphs
    pub(super) lights_hover:Option<AppId>,
    /// the open Cmd/Alt+Tab switcher: the windows most recent first, and the selected one
    pub(super) cmdtab: Option<(Vec<AppId>, usize)>,
    /// the dock slot under the pointer, whose name shows above it
    pub(super) dock_hover:Option<usize>,
    /// the pointer while it magnifies the dock (None when magnification is off or the pointer is away)
    pub(super) dock_ptr:Option<(f32,f32)>,
    /// magnification amount 0..1 easing in and out as the pointer enters and leaves the dock
    pub(super) dock_mag:Option<bifrost_render::motion::Anim>,
    /// autohide slide: dock visibility 0..1 (the dock stays `dock_visible` until a slide-out ends)
    pub(super) dock_vis_anim:Option<bifrost_render::motion::Anim>,
    /// hover label fade: when `dock_hover` last changed (ms) and the slot that was hovered before
    pub(super) dock_label_ms:f64,
    pub(super) dock_label_last:Option<usize>,
    /// launch bounces: (`desk_dock` list entry, start in seconds) until the app's first window maps
    pub(super) dock_bounce:Vec<(usize,f64)>,
    /// the outer size each app's window had when it was last closed (the window comes back at it when reopened)
    pub(super) closed_sizes:std::collections::HashMap<AppId,(i32,i32)>,
    /// the last title-bar press (window, time in ms): a second press within DOUBLE_CLICK_MS zooms
    last_title: Option<(AppId, f64)>,
    pub(super) dock_hide_at:Option<f64>,
    /// the newest files of Downloads, fanned out by the Dock's stack, and when the folder was last listed (seconds)
    pub(super) stack_files:Vec<String>,
    pub(super) stack_listed:Option<f64>,
    pub(super) dock_menu_target:Option<usize>,
    pub(super) dock_menu_anchor:(f32,f32),
    grab: Option<Grab>,
    /// neighbours sliding aside while a dock icon is lifted, and the slot it hovers
    pub(super) dock_slide: std::cell::RefCell<bifrost_ui::dockdrag::Slide>,
    dock_drop: Option<usize>,
    /// dock icons scaled to the dock size, keyed by (source pixels, width): resized once, not per frame
    icon_cache: std::cell::RefCell<std::collections::HashMap<(usize, usize), Image>>,
    buf: Vec<u32>,
    /// scratch for drawing one window in the Wayland path
    #[cfg(feature = "desktop-wayland")]
    window_buf: Vec<u32>,
    /// how many windows have been opened, to cascade them
    opened: usize,
    /// running open, minimize, restore and zoom effects (empty at rest: no frames, no timers)
    pub(super) fx: super::window_fx::FxTracker,
    #[cfg(feature = "desktop-wayland")]
    frames: Vec<crate::bare::wayland::WindowFrame>,
    /// last frames of windows that are closing: (depth in `frames`, frame), drawn until the close effect ends
    #[cfg(feature = "desktop-wayland")]
    ghosts: Vec<(usize, crate::bare::wayland::WindowFrame)>,
    /// an open or focus asked for the front window to be raised over native clients
    pub(super) raise_front: bool,
    #[cfg(feature = "desktop-wayland")]
    window_cache: std::collections::HashMap<u32, CachedWindow>,
    #[cfg(feature = "desktop-wayland")]
    body_cache: std::collections::HashMap<AppId, Vec<u32>>,
    #[cfg(feature = "desktop-wayland")]
    pub(super) linux_apps:Vec<LinuxDockApp>,
    #[cfg(feature = "desktop-wayland")]
    pub(super) native_windows:Vec<crate::bare::wayland::ForeignWindow>,
    #[cfg(feature = "desktop-wayland")]
    foreign_snapshot:Vec<crate::bare::wayland::ForeignWindow>,
    #[cfg(feature = "desktop-wayland")]
    native_children:std::collections::HashMap<String,std::process::Child>,
    #[cfg(feature = "desktop-wayland")]
    activate:Vec<u32>,
    /// Native app ids last saved to (or loaded from) `open-native-windows.json`; `None` before restore.
    #[cfg(feature = "desktop-wayland")]
    saved_native:Option<Vec<String>>,
    /// Restored native apps that have not shown a window yet; saving waits for them.
    #[cfg(feature = "desktop-wayland")]
    restore_pending:Vec<String>,
}

#[cfg(feature = "desktop-wayland")]
const OPEN_NATIVE_FILE: &str = "open-native-windows.json";

/// Reads `["app.id", ...]`; ids are reverse-DNS, so no escapes are needed.
#[cfg(feature = "desktop-wayland")]
fn read_open_native(path:&std::path::Path)->Vec<String> {
    let Ok(text)=std::fs::read_to_string(path) else {return vec![]};
    text.split('"').skip(1).step_by(2).filter(|id|!id.is_empty()&&id.len()<=128&&id.chars().all(|c|c.is_ascii_alphanumeric()||"._-".contains(c))).map(str::to_string).take(64).collect()
}

#[cfg(feature = "desktop-wayland")]
fn write_open_native(path:&std::path::Path,ids:&[String]) {
    let text=format!("[{}]\n",ids.iter().map(|id|format!("\"{id}\"")).collect::<Vec<_>>().join(","));
    let tmp=path.with_extension("json.tmp");
    if std::fs::write(&tmp,text).and_then(|_|std::fs::rename(&tmp,path)).is_err(){eprintln!("BifrOSt: could not save open windows");}
}

/// Geometry in px, derived from the shell's scale and size.
#[derive(Clone, Copy)]
pub struct Metrics {
    pub w: i32,
    pub h: i32,
    pub title: i32,
    pub menubar: i32,
    pub min_w: i32,
    pub min_h: i32,
    pub edge: i32,
    pub light: i32,
    pub light_gap: i32,
    pub light_x: i32,
    pub dock_bottom: i32,
    /// px per point
    pub pt: f32,
}

/// A full-size-content window (`NativeWindowStyleMaskFullSizeContentView`): no title bar, the app draws to the top edge
/// and the traffic lights sit inside its sidebar. Settings is one; an installed app asks with `window = "full-size"`
/// in its manifest.
pub fn full_size(app: AppId) -> bool {
    match app {
        AppId::Settings => true,
        AppId::External(slot) => apphost::installed().get(slot as usize).is_some_and(|a| a.manifest.full_size_content()),
        _ => false,
    }
}

/// The unified title and toolbar row of a full-size app window, in points (`bifrost_app::layout::UNIFIED_BAR_PT`).
pub const UNIFIED_BAR_PT: f32 = 52.0;

/// Where a press moves window `w` instead of reaching the app: above `drag_height` (window px), except in the
/// rectangles returned (window px). A full-size app's empty toolbar band drags; its traffic lights and the controls
/// it publishes (`App::controls`) do not, as the native toolkit lets the empty part of a unified toolbar move the window.
pub fn drag_zone(w: &Win, m: &Metrics, controls: &[bifrost_proto::Node]) -> (i32, Vec<[i32; 4]>) {
    zone_of(w, m, controls, full_size(w.app) && w.app != AppId::Settings)
}

/// `drag_zone` for a window that is (`unified`) or is not a full-size app window.
fn zone_of(w: &Win, m: &Metrics, controls: &[bifrost_proto::Node], unified: bool) -> (i32, Vec<[i32; 4]>) {
    if !unified {
        return (m.title, Vec::new());
    }
    let pt = m.pt;
    let band = (UNIFIED_BAR_PT * pt).round() as i32;
    let l2 = w.light_rect(m, 2);
    let mut out = vec![[0, 0, l2.x + l2.w - w.r.x + (6.0 * pt) as i32, band]];
    for n in controls.iter().filter(|n| n.bounds.y < UNIFIED_BAR_PT) {
        let b = n.bounds;
        out.push([(b.x * pt).floor() as i32, (b.y * pt).floor() as i32, (b.w * pt).ceil() as i32, (b.h * pt).ceil() as i32]);
    }
    (band, out)
}

/// Height of the title bar of a window of `app`, in px.
/// An installed app's window size rules from its manifest (`window_size`, `min_size`, `max_size`); built-in apps have
/// none (`NativeWindow.contentMinSize`, `contentMaxSize`, a fixed-size window without the resizable style).
pub fn size_limits(app: AppId) -> apphost::SizeLimits {
    match app {
        AppId::External(slot) => apphost::installed().get(slot as usize).map(|a| a.manifest.size_limits()).unwrap_or_default(),
        _ => apphost::SizeLimits::default(),
    }
}

/// A content size in points as an outer window size in px (adds the title bar of a titled window).
pub fn outer_px(app: AppId, m: &Metrics, (w, h): (f32, f32)) -> (i32, i32) {
    ((w * m.pt).round() as i32, (h * m.pt).round() as i32 + title_of(app, m))
}

/// The outer size range (px) a window of `app` may be resized within: `(min_w, min_h, max_w, max_h)`.
fn size_range(app: AppId, m: &Metrics) -> (i32, i32, i32, i32) {
    let lim = size_limits(app);
    let (min_w, min_h) = lim.min.map(|s| outer_px(app, m, s)).unwrap_or((m.min_w, m.min_h + title_of(app, m)));
    let (max_w, max_h) = lim.max.map(|s| outer_px(app, m, s)).unwrap_or((i32::MAX, i32::MAX));
    (min_w, min_h, max_w.max(min_w), max_h.max(min_h))
}

pub fn title_of(app: AppId, m: &Metrics) -> i32 {
    if full_size(app) { 0 } else { m.title }
}

impl Win {
    pub fn body(&self, m: &Metrics) -> Rect {
        let t = title_of(self.app, m);
        Rect { x: self.r.x, y: self.r.y + t, w: self.r.w, h: (self.r.h - t).max(1) }
    }

    fn light_rect(&self, m: &Metrics, i: usize) -> Rect {
        if full_size(self.app) {
            use bifrost_ui::prefs_layout::Frame;
            let d = (14.0 * m.pt).round() as i32;
            let (cx, cy) = Frame::traffic_light(i);
            return Rect { x: self.r.x + (cx * m.pt).round() as i32 - d / 2, y: self.r.y + (cy * m.pt).round() as i32 - d / 2, w: d, h: d };
        }
        let x = self.r.x + m.light_x + i as i32 * (m.light + m.light_gap);
        Rect { x, y: self.r.y + (m.title - m.light) / 2, w: m.light, h: m.light }
    }

    /// What is at (x, y) of this window, if anything.
    pub fn hit(&self, m: &Metrics, x: f32, y: f32) -> Option<Hit> {
        let e = m.edge;
        let outer = Rect { x: self.r.x - e / 2, y: self.r.y - e / 2, w: self.r.w + e, h: self.r.h + e };
        if !outer.contains(x, y) {
            return None;
        }
        if !self.maximized && !size_limits(self.app).fixed {
            let mut edges = 0;
            if x < (self.r.x + e / 2) as f32 {
                edges |= L;
            }
            if x >= (self.r.x + self.r.w - e / 2) as f32 {
                edges |= R;
            }
            if y < (self.r.y + e / 2) as f32 {
                edges |= T;
            }
            if y >= (self.r.y + self.r.h - e / 2) as f32 {
                edges |= B;
            }
            if edges != 0 {
                return Some(Hit::Edge(edges));
            }
        }
        if !self.r.contains(x, y) {
            return None;
        }
        if full_size(self.app) {
            let pt = m.pt;
            for (i, l) in [Light::Close, Light::Minimize, Light::Zoom].into_iter().enumerate() {
                let lr = self.light_rect(m, i);
                let pad = (4.0 * pt) as i32;
                if (Rect { x: lr.x - pad, y: lr.y - pad, w: lr.w + 2 * pad, h: lr.h + 2 * pad }).contains(x, y) {
                    return Some(Hit::Light(l));
                }
            }
            if self.app != AppId::Settings {
                // a full-size app: its unified toolbar band is the title bar; controls in it are sorted out on the press
                return Some(if y < self.r.y as f32 + UNIFIED_BAR_PT * pt { Hit::Title } else { Hit::Content });
            }
            // the drag strip: the top of the sidebar above the search field, and a thin band over the detail pane
            let side = (bifrost_ui::prefs_layout::m::SIDEBAR_W * pt) as i32;
            let strip = if (x as i32) < self.r.x + side { bifrost_ui::prefs_layout::m::SEARCH_Y - 8.0 } else { bifrost_ui::prefs_layout::m::TOOLBAR_Y };
            if y < self.r.y as f32 + strip * pt {
                return Some(Hit::Title);
            }
            return Some(Hit::Content);
        }
        if y < (self.r.y + m.title) as f32 {
            for (i, l) in [Light::Close, Light::Minimize, Light::Zoom].into_iter().enumerate() {
                let lr = self.light_rect(m, i);
                let pad = m.light_gap / 2;
                if (Rect { x: lr.x - pad, y: self.r.y, w: lr.w + 2 * pad, h: m.title }).contains(x, y) {
                    return Some(Hit::Light(l));
                }
            }
            return Some(Hit::Title);
        }
        Some(Hit::Content)
    }
}

impl Desk {
    pub(super) fn grab_idle(&self)->bool{self.grab.is_none()}
    pub fn new() -> Self {
        Desk {dock_visible:true,..Default::default()}
    }

    pub fn index_of(&self, app: AppId) -> Option<usize> {
        self.wins.iter().position(|w| w.app == app)
    }

    pub fn front(&self) -> Option<&Win> {
        self.wins.iter().rev().find(|w| !w.minimized)
    }

    /// Brings window `i` to the front (and shows it); returns its new index.
    pub fn raise(&mut self, i: usize) -> usize {
        let mut w = self.wins.remove(i);
        w.minimized = false;
        self.wins.push(w);
        self.wins.len() - 1
    }

    /// The frontmost visible window hit at (x, y).
    pub fn pick(&self, m: &Metrics, x: f32, y: f32) -> Option<(usize, Hit)> {
        self.wins.iter().enumerate().rev().filter(|(_, w)| !w.minimized).find_map(|(i, w)| w.hit(m, x, y).map(|h| (i, h)))
    }

    /// The work area windows live in: below the menu bar, above the dock.
    pub fn area(m: &Metrics) -> Rect {
        Rect { x: 0, y: m.menubar, w: m.w, h: (m.h - m.menubar - m.dock_bottom).max(1) }
    }

    /// Opens (or raises) the window for `app`; `size` is the wanted outer size.
    pub fn open(&mut self, m: &Metrics, app: AppId, size: (i32, i32)) -> usize {
        if let Some(i) = self.index_of(app) {
            return self.raise(i);
        }
        let a = Self::area(m);
        let (w, h) = (size.0.min(a.w), size.1.min(a.h));
        let step = (m.title as f32 * 1.1) as i32;
        let n = (self.opened % 8) as i32;
        self.opened += 1;
        let x = (a.x + (a.w - w) / 2 + (n - 3) * step).clamp(a.x, (a.x + a.w - w).max(a.x));
        let y = (a.y + (a.h - h) / 3 + n * step / 2).clamp(a.y, (a.y + a.h - h).max(a.y));
        let r = Rect { x, y, w, h };
        self.wins.push(Win { app, r, maximized: false, minimized: false, restore: r });
        self.wins.len() - 1
    }

    /// Puts window `i` at `r` (a restored frame): the size is held inside the app's limits and the work area, the
    /// position inside the work area. The window is no longer zoomed, and `r` is what un-zooming goes back to.
    pub fn place(&mut self, m: &Metrics, i: usize, r: Rect) {
        let a = Self::area(m);
        let (min_w, min_h, max_w, max_h) = size_range(self.wins[i].app, m);
        let w = r.w.clamp(min_w, max_w).min(a.w).max(1);
        let h = r.h.clamp(min_h, max_h).min(a.h).max(1);
        let x = r.x.clamp(a.x, (a.x + a.w - w).max(a.x));
        let y = r.y.clamp(a.y, (a.y + a.h - h).max(a.y));
        let win = &mut self.wins[i];
        win.r = Rect { x, y, w, h };
        win.restore = win.r;
        win.maximized = false;
    }

    pub fn toggle_zoom(&mut self, m: &Metrics, i: usize) {
        let a = Self::area(m);
        let w = &mut self.wins[i];
        if size_limits(w.app).fixed {
            return; // a fixed-size window does not zoom (its green button is disabled on Golden Gate)
        }
        if w.maximized {
            w.r = w.restore;
            w.maximized = false;
        } else {
            w.restore = w.r;
            w.r = a;
            w.maximized = true;
        }
    }

    /// Places window `i` on `tile` of the work area; the rectangle it had is kept for "Return to Previous Size".
    pub fn tile(&mut self, m: &Metrics, i: usize, tile: Tile) {
        let a = Self::area(m);
        let w = &mut self.wins[i];
        if size_limits(w.app).fixed {
            return;
        }
        if !w.maximized && tile_rect(a, tile) != w.r {
            w.restore = w.r;
        }
        w.r = tile_rect(a, tile);
        w.maximized = tile == Tile::Fill;
    }

    /// Goes back to the size before the last tiling or zoom.
    #[cfg(any(test, feature = "desktop-wayland"))]
    pub fn return_to_previous(&mut self, i: usize) {
        let w = &mut self.wins[i];
        w.r = w.restore;
        w.maximized = false;
    }

    pub fn close(&mut self, i: usize) -> AppId {
        let w = self.wins.remove(i);
        // the frame the user left the window at (the zoomed one goes back to the frame before zoom)
        let r = if w.maximized { w.restore } else { w.r };
        self.closed_sizes.insert(w.app, (r.w, r.h));
        self.release_app_buffers(w.app);
        w.app
    }

    /// Drops the per-window pixel caches and stale dock icon resizes; true when something was freed.
    pub fn release_app_buffers(&mut self, app: AppId) -> bool {
        let mut freed = false;
        #[cfg(feature = "desktop-wayland")]
        {
            freed |= self.body_cache.remove(&app).is_some();
            let before = self.window_cache.len();
            self.window_cache.retain(|id, _| apps().get((*id as usize).wrapping_sub(1)).map_or(false, |a| a.id != app));
            freed |= self.window_cache.len() != before;
        }
        let _ = app;
        let mut ic = self.icon_cache.borrow_mut();
        if !ic.is_empty() { ic.clear(); freed = true; }
        freed
    }

    /// The last frame drawn for `app`'s window (width, height, pixels), for the Overview tiles.
    #[cfg(feature = "desktop-wayland")]
    pub(super) fn window_pixels(&self, app: AppId) -> Option<(usize, usize, std::sync::Arc<[u32]>)> {
        let id = apps().iter().position(|a| a.id == app)? as u32 + 1;
        self.window_cache.get(&id).map(|c| (c.width, c.height, c.pixels.clone()))
    }
    #[cfg(not(feature = "desktop-wayland"))]
    pub(super) fn window_pixels(&self, _app: AppId) -> Option<(usize, usize, std::sync::Arc<[u32]>)> { None }

    /// Moves window `i` so its title bar stays reachable inside the screen.
    pub fn move_to(&mut self, m: &Metrics, i: usize, x: i32, y: i32) {
        let w = &mut self.wins[i];
        w.maximized = false;
        let keep = m.title * 2;
        w.r.x = x.clamp(keep - w.r.w, m.w - keep);
        w.r.y = y.clamp(m.menubar, (m.h - m.title).max(m.menubar));
    }

    /// Resizes window `i` from the rectangle it had when the drag began.
    pub fn resize_to(&mut self, m: &Metrics, i: usize, edges: u8, start: Rect, dx: i32, dy: i32) {
        let w = &mut self.wins[i];
        w.maximized = false;
        let (mut x, mut y, mut rw, mut rh) = (start.x, start.y, start.w, start.h);
        let (min_w, min_h, max_w, max_h) = size_range(w.app, m);
        if edges & R != 0 {
            rw = (start.w + dx).clamp(min_w, max_w);
        }
        if edges & B != 0 {
            rh = (start.h + dy).clamp(min_h, max_h);
        }
        if edges & L != 0 {
            rw = (start.w - dx).clamp(min_w, max_w);
            x = start.x + start.w - rw;
        }
        if edges & T != 0 {
            rh = (start.h - dy).clamp(min_h, max_h);
            y = (start.y + start.h - rh).max(m.menubar);
            rh = start.y + start.h - y;
        }
        w.r = Rect { x, y, w: rw, h: rh };
    }
}

/// Dock icon rectangles (px) for `n` icons and the capsule behind them.
/// The Calendar app's package id: its Dock tile shows today's weekday and date over the package icon.
const CALENDAR_TILE: &str = "is.olibuijr.calendar.desktop";

/// The weekday and day of the month on the Calendar tile: ("Mon", "12") for Monday the 12th.
pub(super) fn calendar_face_text(wday: i32, mday: i32) -> (&'static str, String) {
    const DAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
    (DAYS[wday.rem_euclid(7) as usize], mday.to_string())
}

#[cfg(test)]
pub fn dock_rects(m: &Metrics, icon: i32, pad: i32, gap: i32, n: usize, sep_after: Option<usize>, sep: i32) -> (Rect, Vec<Rect>) {
    let seps: Vec<usize> = sep_after.into_iter().collect();
    dock_rects_seps(m, icon, pad, gap, n, &seps, sep)
}

/// Like `dock_rects` with a divider after each index in `seps` (pinned | recents | minimized and Trash).
pub fn dock_rects_seps(m: &Metrics, icon: i32, pad: i32, gap: i32, n: usize, seps: &[usize], sep: i32) -> (Rect, Vec<Rect>) {
    let extra = seps.len() as i32 * sep;
    let total = n as i32 * icon + (n as i32 - 1).max(0) * gap + extra + 2 * pad;
    let x0 = (m.w - total) / 2;
    let y0 = m.h - icon - 2 * pad - m.dock_bottom / 4;
    let bar = Rect { x: x0, y: y0, w: total, h: icon + 2 * pad };
    let mut x = x0 + pad;
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        out.push(Rect { x, y: y0 + pad, w: icon, h: icon });
        x += icon + gap;
        if seps.contains(&i) {
            x += sep;
        }
    }
    (bar, out)
}

// ---- the shell side -------------------------------------------------------------------

/// Which window effect `Shell::desk_fx_start` begins.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum FxCall { Open, Minimize, Restore, Zoom, Close }

/// The id the frames and the presenter use for the window of `app` (its index in `apps()` plus one).
fn win_id(app: AppId) -> Option<u32> {
    apps().iter().position(|a| a.id == app).map(|i| i as u32 + 1)
}

impl Shell {
    #[cfg(feature = "desktop-wayland")]
    pub(super) fn retain_wayland_windows(&mut self) -> bool {
        let m=self.desk_metrics();
        let title_height=m.title;
        let ext=&self.ext;
        let Some(desk)=self.desk.as_mut() else{return false};
        if !desk.fx.is_empty() {return false;}
        let mut frames=Vec::new();
        for w in desk.wins.iter().filter(|w|!w.minimized) {
            let Some(id)=apps().iter().position(|a|a.id==w.app).map(|i|i as u32+1) else{return false};
            if !desk.window_cache.contains_key(&id) {return false;}
            frames.push(crate::bare::wayland::WindowFrame {id,title:apps()[(id-1) as usize].name,x:w.r.x,y:w.r.y,width:w.r.w as usize,height:w.r.h as usize,pixels:None,damage:crate::gfx::Damage::new(w.r.w as usize,w.r.h as usize),maximized:w.maximized,title_height,drag_height:0,no_drag:Vec::new(),fx:bifrost_render::glass::Fx::IDENTITY,opaque:!matches!(w.app,AppId::External(_))});
            let controls=match w.app {AppId::External(slot)=>ext.get(slot as usize).and_then(Option::as_ref).map_or(&[][..],|h|&h.controls[..]),_=>&[][..]};
            let (dh,nd)=drag_zone(w,&m,controls);
            if let Some(f)=frames.last_mut(){f.drag_height=dh;f.no_drag=nd;}
        }
        desk.frames=frames;
        true
    }
    /// True once after an open or focus that should raise the front window over native clients.
    pub fn take_raise_front(&mut self) -> bool { self.desk.as_mut().is_some_and(|d|std::mem::take(&mut d.raise_front)) }
    /// Each open window's behind-window materials (`NativeVisualEffectView` .sidebar / .titlebar) in window px, for
    /// `Wayland::window_materials`: an app's `ToShell::Materials` (window points, under the title bar of a titled
    /// window) and the Settings sidebar column. Empty without vibrancy, so the presenter draws windows opaque.
    /// Unrounded window corners would let the blur show past the window's rounded edge, so every region takes the
    /// window radius and grows by it on the sides that are inside the window: the extra blur lands under opaque
    /// (or other material) pixels, where it cannot be seen.
    /// Points to px on the desktop (`ui.pt(1.0)`), for the presenter's `Wayland::set_scale`.
    pub fn points_px(&self) -> f32 { self.ui.pt(1.0) }
    #[cfg(feature = "desktop-wayland")]
    pub fn take_wayland_materials(&self) -> Vec<(u32, Vec<bifrost_render::glass::Rect>)> {
        let Some(desk) = self.desk.as_ref() else { return Vec::new() };
        let vibrant = apphost::vibrancy_available();
        let m = self.desk_metrics();
        let corner = self.ui.pt(bifrost_ui::prefs_layout::m::WINDOW_RADIUS);
        let mut out = Vec::new();
        for w in desk.wins.iter().filter(|w| !w.minimized) {
            let Some(id) = apps().iter().position(|a| a.id == w.app).map(|i| i as u32 + 1) else { continue };
            let top = title_of(w.app, &m) as f32;
            let mut rects: Vec<(f32, f32, f32, f32, f32, bifrost_render::glass::Kind, Option<bool>, Option<bifrost_render::glass::Detail>)> = Vec::new();
            if vibrant {
                match w.app {
                    AppId::External(slot) => {
                        if let Some(h) = self.ext.get(slot as usize).and_then(Option::as_ref).filter(|h| h.running()) {
                            for mat in h.materials.iter().take(bifrost_proto::MAX_MATERIALS) {
                                let b = mat.rect;
                                let req = mat.request();
                                let c = req.corners();
                                let detail = bifrost_render::glass::Detail {
                                    kind: mat.kind.window_server_wire(),
                                    within_window: !req.for_window_server(),
                                    emphasized: mat.emphasized,
                                    radii: [c.tl * m.pt, c.tr * m.pt, c.br * m.pt, c.bl * m.pt],
                                };
                                let r = c.tl.max(c.tr).max(c.br).max(c.bl);
                                rects.push((b.x * m.pt, top + b.y * m.pt, b.w * m.pt, b.h * m.pt, r * m.pt, glass_kind(mat.kind), glass_active(mat.state), Some(detail)));
                            }
                        }
                    }
                    AppId::Settings => {
                        if let Some(side) = self.settings.as_ref().map(|st| st.sidebar_px()).filter(|&p| p > 0) {
                            rects.push((0.0, top, side as f32, (w.r.h as f32 - top).max(0.0), 0.0, bifrost_render::glass::Kind::Sidebar, None, None));
                        }
                    }
                    _ => {}
                }
            }
            let glass = rects.into_iter().filter(|r| r.2 > 0.0 && r.3 > 0.0).map(|(x, y, rw, rh, r, kind, active, detail)| {
                let mut g = expand_material((x, y, rw, rh, r, kind, active), (w.r.w as f32, w.r.h as f32), w.maximized, corner);
                // an expanded region takes the window radius on every corner; an untouched one keeps its own radii
                g.detail = detail.map(|mut d| {
                    if (g.x, g.y, g.w, g.h) != (x, y, rw, rh) {
                        d.radii = [g.r; 4];
                    }
                    d
                });
                g
            }).collect();
            out.push((id, glass));
        }
        out
    }
    #[cfg(feature = "desktop-wayland")]
    pub fn take_wayland_windows(&mut self) -> Vec<crate::bare::wayland::WindowFrame> {
        self.desk.as_mut().map(|d| {
            let frames = std::mem::take(&mut d.frames);
            // Lock/overlays destroy the backend's windows; rebuilding must send
            // complete pixels rather than a cached unchanged-window sentinel.
            if frames.is_empty() { d.window_cache.clear(); d.body_cache.clear(); }
            frames
        }).unwrap_or_default()
    }
    #[cfg(feature = "desktop-wayland")]
    pub fn take_native_activations(&mut self) -> Vec<u32> {
        self.desk.as_mut().map(|d|std::mem::take(&mut d.activate)).unwrap_or_default()
    }
    #[cfg(feature = "desktop-wayland")]
    pub fn wayland_foreign_windows(&mut self,windows:Vec<crate::bare::wayland::ForeignWindow>) {
        #[cfg(feature = "desktop-wayland")]
        if self.desk.as_ref().is_some_and(|d|d.saved_native.is_none()) { self.desk_restore_native(); }
        // apps the shell launched on the direct path: their window is the window server's, listed by manifest id
        let direct:Vec<String>=apphost::installed().iter().enumerate().filter(|(slot,_)|self.ext.get(*slot).and_then(Option::as_ref).is_some_and(|h|h.running()&&h.route()==apphost::Route::Direct)).map(|(_,a)|a.manifest.id.clone()).collect();
        let Some(desk)=self.desk.as_mut() else { return; };
        if desk.foreign_snapshot==windows {return;}
        desk.foreign_snapshot=windows.clone();
        for (slot, app) in apphost::installed().iter().enumerate() {
            let compositor_id=if direct.contains(&app.manifest.id) {app.manifest.id.clone()}
                else if app.manifest.window=="native" {app.manifest.wayland_app_id.clone()}
                else {let Some(index)=apps().iter().position(|a|a.id==AppId::External(slot as u8)) else {continue};format!("is.olibuijr.bifrost.{}",index+1)};
            crate::bare::app_manager::focus(&app.manifest.id, windows.iter().any(|w|w.app_id==compositor_id && w.active));
        }
        desk.native_windows=windows.iter().filter(|w|apphost::installed().iter().any(|a|(a.manifest.window=="native" && a.manifest.wayland_app_id==w.app_id) || (direct.contains(&a.manifest.id) && a.manifest.id==w.app_id))).cloned().collect();
        let windows:Vec<_>=windows.into_iter().filter(|w|!w.app_id.starts_with("is.olibuijr.bifrost.") && !desk.native_windows.iter().any(|n|n.id==w.id)).collect();
        desk.linux_apps.retain(|app|windows.iter().any(|w|w.app_id==app.app_id)||desk.dock_preferences.pinned.contains(&format!("linux:{}",app.app_id)));
        for key in &desk.dock_preferences.pinned{if let Some(id)=key.strip_prefix("linux:"){if !desk.linux_apps.iter().any(|a|a.app_id==id){desk.linux_apps.push(LinuxDockApp{app_id:id.into(),windows:vec![],icon:linux_icon(id)});}}}
        for w in &windows {
            if !desk.linux_apps.iter().any(|app|app.app_id==w.app_id) {
                desk.linux_apps.push(LinuxDockApp {app_id:w.app_id.clone(),windows:Vec::new(),icon:linux_icon(&w.app_id)});
            }
        }
        for app in &mut desk.linux_apps { app.windows=windows.iter().filter(|w|w.app_id==app.app_id).cloned().collect(); }
        #[cfg(feature = "desktop-wayland")]
        self.desk_save_native();
        self.dirty=true;
    }
    /// Reopens the native apps saved by the previous shell session (once, at the first snapshot).
    #[cfg(feature = "desktop-wayland")]
    fn desk_restore_native(&mut self) {
        let saved=read_open_native(&self.data_dir.join(OPEN_NATIVE_FILE));
        if let Some(desk)=self.desk.as_mut() {desk.saved_native=Some(saved.clone());}
        for id in &saved {
            let Some(slot)=apphost::installed().iter().position(|a|a.manifest.id==*id&&a.manifest.window=="native") else {continue};
            self.desk_native_open(AppId::External(slot as u8));
            if let Some(desk)=self.desk.as_mut() {if desk.native_children.contains_key(id) {desk.restore_pending.push(id.clone());}}
        }
    }
    /// Saves the native app ids with open windows, only when that set changes.
    #[cfg(feature = "desktop-wayland")]
    fn desk_save_native(&mut self) {
        let Some(desk)=self.desk.as_mut() else {return};
        if desk.saved_native.is_none() {return;}
        let shown=|id:&str|desk.native_windows.iter().any(|w|apphost::installed().iter().any(|a|a.manifest.id==id&&a.manifest.wayland_app_id==w.app_id));
        let mut pending=std::mem::take(&mut desk.restore_pending);
        pending.retain(|id|!shown(id)&&desk.native_children.get_mut(id).is_some_and(|c|matches!(c.try_wait(),Ok(None))));
        desk.restore_pending=pending;
        if !desk.restore_pending.is_empty() {return;}
        let mut now:Vec<String>=desk.native_windows.iter().filter_map(|w|apphost::installed().iter().find(|a|a.manifest.window=="native"&&a.manifest.wayland_app_id==w.app_id).map(|a|a.manifest.id.clone())).collect();
        now.sort();now.dedup();
        if desk.saved_native.as_ref()==Some(&now) {return;}
        write_open_native(&self.data_dir.join(OPEN_NATIVE_FILE),&now);
        if let Some(desk)=self.desk.as_mut() {desk.saved_native=Some(now);}
    }
    #[cfg(feature = "desktop-wayland")]
    pub fn wayland_window_event(&mut self, ev: crate::bare::wayland::WindowEvent) {
        use crate::bare::wayland::WindowEvent;
        let id = match ev { WindowEvent::Focus(id) | WindowEvent::Close(id) | WindowEvent::Resize(id,_,_) | WindowEvent::Maximized(id,_) => id };
        let Some(app) = apps().get(id.saturating_sub(1) as usize).map(|a| a.id) else { return };
        let Some(i) = self.desk.as_ref().and_then(|d| d.index_of(app)) else { return };
        match ev {
            WindowEvent::Focus(_) => { self.desk.as_mut().unwrap().raise(i); self.screen = Screen::App(app); self.dirty = true; },
            WindowEvent::Close(_) => self.desk_close_app(app),
            WindowEvent::Maximized(_,maximized) => {let win=&mut self.desk.as_mut().unwrap().wins[i]; if win.maximized!=maximized {if maximized {win.restore=win.r;}else{win.r=win.restore;}win.maximized=maximized;self.dirty=true;}},
            WindowEvent::Resize(_,w,h) if w > 0 && h > 0 => {
                let m = self.desk_metrics();
                let win = &mut self.desk.as_mut().unwrap().wins[i];
                win.r.w = w.max(m.min_w); win.r.h = h.max(m.min_h + title_of(win.app, &m)); self.desk_layout(i); self.dirty = true;
            },
            _ => {}
        }
    }

    pub(super) fn desk_active(&self) -> bool {
        self.desk.is_some()
    }

    /// Settings > Appearance without a restart: the pref is saved and `appearance_tick` fades to it.
    pub fn set_appearance(&mut self,mode:&str)->Result<(),String> {
        self.prefs.set(super::settings::APPEARANCE_PREF,mode).map_err(|e|e.to_string())?;
        self.dirty=true;
        Ok(())
    }

    pub fn desktop_open_app(&mut self,id:&str,t:f64)->bool {
        if !self.desk_active(){return false;}
        if let Some(app)=apps().iter().find(|a|a.icon==id).map(|a|a.id){self.desk_open(app,t);return true;}
        // installed store apps have no built-in entry: open them by their installed slot
        if let Some(slot)=apphost::slot_of(id){self.desk_open(AppId::External(slot as u8),t);return true;}
        false
    }
    /// Show in Files: opens the installed app `id` at `folder`. The request is written to the app's
    /// data folder first, so the app reads it on its next tick; a failed open removes it again.
    pub fn desktop_open_folder(&mut self,id:&str,folder:&str,t:f64)->Result<(),String> {
        if !self.desk_active(){return Err("The desktop is not active".into());}
        if id.is_empty()||!id.bytes().all(|c|c.is_ascii_alphanumeric()||b"._-".contains(&c)){return Err("Invalid app ID".into());}
        if !apphost::installed().iter().any(|p|p.manifest.id==id){return Err("App not installed".into());}
        let data=apphost::root().join("appdata").join(id);
        crate::files::handoff::write_request(&data,std::path::Path::new(folder)).map_err(|e|e.to_string())?;
        if self.desktop_open_app(id,t){Ok(())}else{let _=std::fs::remove_file(data.join(crate::files::handoff::REQUEST));Err("App not running on the desktop".into())}
    }
    pub fn desktop_export_apps(&self){if !self.desk_active(){return;}let root=std::path::Path::new("/run/bifrost/user/share/applications");if std::fs::create_dir_all(root).is_err(){return;}for app in apps(){if app.icon=="is.olibuijr.browser.desktop"{continue;}if !app.icon.bytes().all(|c|c.is_ascii_alphanumeric()||b"._-".contains(&c)){continue;}let text=format!("[Desktop Entry]\nType=Application\nName={}\nExec=/usr/bin/bifrost-open-url --app {}\nIcon=application-x-executable\nTerminal=false\nCategories=Utility;\n",app.name.replace('\n'," "),app.icon);let _=std::fs::write(root.join(format!("{}.desktop",app.icon)),text);}}
    pub fn desktop_open_url(&mut self,url:&str,t:f64)->std::io::Result<()> {
        let _ = t; // only the Wayland desktop opens the page with a time
        if !self.desk_active()||!(url.starts_with("http://")||url.starts_with("https://"))||url.chars().any(char::is_control){return Err(std::io::Error::other("Unsupported web URL"));}
        #[cfg(feature="desktop-wayland")]{let package=apphost::installed().iter().find(|p|p.manifest.id=="is.olibuijr.browser.desktop").cloned().ok_or_else(||std::io::Error::other("Install BifrOSt Browser from App Store"))?;
        // an SDK Browser (0.3+) opens the page in a tab of its window; only a native build takes the URL on its command line
        if package.manifest.window!="native" {let slot=apphost::slot_of(&package.manifest.id).ok_or_else(||std::io::Error::other("Browser not installed"))?;self.desk_open(AppId::External(slot as u8),t);self.ext_send(slot,&bifrost_proto::ToApp::Open{url:url.into()});self.dirty=true;return Ok(());}let policy=crate::bare::app_policy::Policy::from_permissions(&package.manifest.id,&package.manifest.permissions)?;let mut cmd=crate::bare::app_policy::command(&package.manifest.id,&package.exec_path(),&apphost::root().join("appdata").join(&package.manifest.id),policy,true)?;cmd.arg(url);let child=cmd.spawn()?;if let Some(mut old)=self.desk.as_mut().unwrap().native_children.insert(package.manifest.id.clone(),child){if matches!(old.try_wait(),Ok(None)){crate::bare::sys::spawn_named("reap-child", move||{let _=old.wait();});}}self.dirty=true;return Ok(());}
        #[cfg(not(feature="desktop-wayland"))]Err(std::io::Error::other("Desktop Wayland is unavailable"))
    }
    /// Index of the front desktop window, for the Window menu and keyboard tiling.
    /// Whether the window server lists a window for the app in `slot`: a direct app by its manifest id, a native app
    /// by its Wayland app id (the Dock's running dot).
    #[cfg(feature = "desktop-wayland")]
    pub(super) fn foreign_running(&self, slot: usize) -> bool {
        let (Some(desk), registered) = (self.desk.as_ref(), apphost::installed()) else { return false };
        let Some(app) = registered.get(slot) else { return false };
        let direct = self.ext.get(slot).and_then(Option::as_ref).is_some_and(|h| h.running() && h.route() == apphost::Route::Direct);
        let id = if direct { app.manifest.id.as_str() } else { app.manifest.wayland_app_id.as_str() };
        desk.native_windows.iter().any(|w| w.app_id == id)
    }

    /// Opens a direct app's window: the window server owns it, so the shell starts the app if needed and raises
    /// its toplevel, and makes no window of its own. True when handled; false sends the open down the window path
    /// (the app is on the shared-memory path, or the launch decision is not the direct one).
    #[cfg(feature = "desktop-wayland")]
    fn desk_direct_open(&mut self, slot: usize) -> bool {
        if apphost::launch_route() != apphost::Route::Direct { return false; }
        self.ext_open(slot);
        if let Some(h) = self.ext.get(slot).and_then(Option::as_ref) {
            if h.route() == apphost::Route::Shm { return false; }
        }
        let id = apphost::installed().get(slot).map(|a| a.manifest.id.clone()).unwrap_or_default();
        let toplevel = self.desk.as_ref().and_then(|d| apphost::foreign_window_of(d.native_windows.iter().map(|w| (w.id, w.app_id.as_str())), &id));
        if let (Some(toplevel), Some(d)) = (toplevel, self.desk.as_mut()) { d.activate.push(toplevel); }
        self.dirty = true;
        true
    }

    #[cfg(feature = "desktop-wayland")]
    fn desk_front_index(&self) -> Option<usize> {
        self.desk.as_ref().and_then(|d| d.front().and_then(|w| d.index_of(w.app)))
    }

    /// Tiles the front desktop window on `tile` (Window menu and Ctrl+Cmd arrow keys). In the Wayland session the
    /// compositor places windows, so only Fill (maximize) applies here; the halves and Center are compositor key
    /// bindings (packaging/arch/wayland/rc.xml).
    #[cfg(feature = "desktop-wayland")]
    pub(super) fn desk_tile_front(&mut self, tile: Tile) {
        let Some(i) = self.desk_front_index() else { return };
        if compositor_places_windows() {
            let maximized = self.desk.as_ref().is_some_and(|d| d.wins[i].maximized);
            if tile == Tile::Fill && !maximized { self.desk_zoom_window(i); }
            return;
        }
        let m = self.desk_metrics();
        if let Some(d) = self.desk.as_mut() { d.tile(&m, i, tile); }
        self.desk_layout(i);
        self.desk_frame_changed(i);
    }

    /// After a frame change outside a drag: the presenter takes the new frame through the zoom effect (as Zoom does),
    /// otherwise the app's surface stays where it was.
    fn desk_frame_changed(&mut self, i: usize) {
        if let Some(app) = self.desk.as_ref().and_then(|d| d.wins.get(i)).map(|w| w.app) {
            let t = self.last_input;
            self.desk_fx_start(app, FxCall::Zoom, t);
        }
        // the whole desk repaints: a menu that chose the action closes on this frame, and its area must not be kept
        self.drawn = super::Drawn::Nothing;
        self.dirty = true;
    }

    /// Returns the front desktop window to the size it had before its last tiling or zoom.
    #[cfg(feature = "desktop-wayland")]
    pub(super) fn desk_restore_front(&mut self) {
        let Some(i) = self.desk_front_index() else { return };
        if compositor_places_windows() {
            if self.desk.as_ref().is_some_and(|d| d.wins[i].maximized) { self.desk_zoom_window(i); }
            return;
        }
        if let Some(d) = self.desk.as_mut() { d.return_to_previous(i); }
        self.desk_layout(i);
        self.desk_frame_changed(i);
    }

    pub(super) fn desk_zoom_window(&mut self,i:usize) {let m=self.desk_metrics();self.desk.as_mut().unwrap().toggle_zoom(&m,i);self.desk_layout(i);if let Some(app)=self.desk.as_ref().and_then(|d|d.wins.get(i)).map(|w|w.app){let t=self.last_input;self.desk_fx_start(app,FxCall::Zoom,t);}self.dirty=true;}

    /// Minimizes window `i` to its Dock tile, playing the minimize effect first when it is allowed.
    pub(super) fn desk_minimize_window(&mut self, i: usize, t: f64) {
        let Some(app) = self.desk.as_ref().and_then(|d| d.wins.get(i)).map(|w| w.app) else { return };
        // the effect starts while the window is still mapped; the draw loop keeps it up until the effect ends
        self.desk_fx_start(app, FxCall::Minimize, t);
        if let Some(d) = self.desk.as_mut() { d.wins[i].minimized = true; }
        self.desk_refocus();
    }

    /// Dock tile (x, y, w, h) of `app`, or the dock's own rectangle when it has no tile.
    fn fx_dock_target(&self, m: &Metrics, app: AppId) -> (i32, i32, i32, i32) {
        let (list, bar, icons) = self.desk_dock(m);
        let want = Self::dock_app(app);
        let r = list.iter().position(|&e| apps().get(e).is_some_and(|a| Self::dock_app(a.id) == want))
            .and_then(|k| icons.get(k)).copied().unwrap_or(bar);
        (r.x, r.y, r.w, r.h)
    }

    /// Starts a window effect. Nothing starts without the Wayland presenter, for `AnimationBehavior::None`
    /// (and the kinds a behaviour excludes), or without a desk; Reduce Motion turns it into a cross-fade.
    pub(super) fn desk_fx_start(&mut self, app: AppId, call: FxCall, t: f64) {
        if !cfg!(feature = "desktop-wayland") || std::env::var_os("SPRINGBOARD_WAYLAND").is_none() { return; }
        let (Some(id), Some(i)) = (win_id(app), self.desk.as_ref().and_then(|d| d.index_of(app))) else { return };
        let m = self.desk_metrics();
        let win = self.desk.as_ref().unwrap().wins[i].r;
        let (dx, dy) = window_fx::minimize_target((win.x, win.y, win.w, win.h), self.fx_dock_target(&m, app));
        // Funnel needs a mesh warp the presenter cannot draw in the window's own drawable: both effects scale and slide
        let _effect = window_fx::MinimizeEffect::from_index(super::pages::DeskPrefs::load(&self.prefs).minimize_effect);
        let effect = match call {
            FxCall::Open => window_fx::Effect::Open,
            FxCall::Minimize => window_fx::Effect::Minimize { to_dx: dx, to_dy: dy },
            FxCall::Restore => window_fx::Effect::Restore { from_dx: dx, from_dy: dy },
            FxCall::Zoom => window_fx::Effect::Zoom,
            FxCall::Close => window_fx::Effect::Close,
        };
        let behavior = match app {
            AppId::External(slot) => apphost::installed().get(slot as usize).map_or_else(Default::default, |a| a.manifest.animation_behavior()),
            _ => window_fx::AnimationBehavior::Default,
        };
        let reduce = bifrost_render::appearance::reduce_motion();
        #[cfg(feature = "desktop-wayland")]
        if let Some(d) = self.desk.as_mut() { d.ghosts.retain(|g| g.1.id != id); }
        if self.desk.as_mut().unwrap().fx.begin(id, effect, behavior, reduce, t / 1000.0) { self.dirty = true; }
    }

    /// Keeps the last drawn frame of a window that is closing, at its depth, so the close effect can draw it after
    /// the window has left the window list. Only when the close effect actually started.
    #[cfg(feature = "desktop-wayland")]
    fn desk_keep_ghost(&mut self, app: AppId) {
        let Some(id) = win_id(app) else { return };
        let Some(d) = self.desk.as_mut() else { return };
        if !d.fx.get(id).is_some_and(|f| f.effect == window_fx::Effect::Close) { return; }
        let Some(z) = d.frames.iter().position(|f| f.id == id) else { return };
        let mut frame = d.frames[z].clone();
        frame.damage.all();
        frame.fx = bifrost_render::glass::Fx::IDENTITY;
        d.ghosts.push((z, frame));
    }

    /// Per tick: drops finished effects (one last frame is drawn) and keeps frames coming while any runs.
    pub(super) fn fx_tick(&mut self, t: f64) {
        let Some(d) = self.desk.as_mut() else { return };
        let now = t / 1000.0;
        if !d.fx.finish(now).is_empty() || d.fx.active(now) { self.dirty = true; }
    }

    /// True while a window effect needs frames; `next_wake` polls it.
    pub fn fx_animating(&self, t: f64) -> bool {
        self.desk.as_ref().is_some_and(|d| d.fx.active(t / 1000.0))
    }
    pub(super) fn desk_metrics(&self) -> Metrics {
        let ui = &self.ui;
        let icon = ui.pt(self.dock_preferences().size).round() as i32;
        let pad = ui.pt(DOCK_PAD_PT).round() as i32;
        Metrics {
            w: self.w,
            h: self.h,
            title: ui.pt(TITLE_PT).round() as i32,
            menubar: self.desktop_bar_height() as i32,
            min_w: ui.pt(MIN_W_PT).round() as i32,
            min_h: ui.pt(MIN_H_PT).round() as i32,
            edge: ui.pt(EDGE_PT).round() as i32,
            light: ui.pt(LIGHT_PT).round() as i32,
            light_gap: ui.pt(LIGHT_GAP_PT).round() as i32,
            light_x: ui.pt(LIGHT_X_PT).round() as i32,
            dock_bottom: icon + 2 * pad + ui.pt(DOCK_MARGIN_PT).round() as i32 * 2,
            pt: ui.pt(1.0),
        }
    }

    pub(super) fn desk_dock(&self, m: &Metrics) -> (Vec<usize>, Rect, Vec<Rect>) {
        let desk=self.desk.as_ref().unwrap();
        let mut list:Vec<usize>=desk.dock_preferences.pinned.iter().filter_map(|key|apps().iter().position(|a|a.icon==key)).collect();
        for (i,a) in apps().iter().enumerate(){#[cfg_attr(not(feature="desktop-wayland"),allow(unused_mut))] let mut running=desk.index_of(a.id).is_some();
        #[cfg(feature="desktop-wayland")]if let AppId::External(slot)=a.id {running|=self.foreign_running(slot as usize);}
        if running&&!list.contains(&i){list.push(i);}}
        #[cfg(feature = "desktop-wayland")]
        list.extend((0..self.desk.as_ref().unwrap().linux_apps.len()).map(|i|apps().len()+1+i));
        if std::env::var_os("SPRINGBOARD_WAYLAND").is_some() { list.push(apps().len()); }
        // sections: pinned, then recents (running, not kept), then minimized windows and the Trash
        list.sort_by_key(|&i| self.dock_section(i));
        list.extend(desk.wins.iter().enumerate().filter(|(_, w)| self.overview.user_minimized(w)).map(|(wi, _)| super::desktop_dock::DOCK_MIN_BASE - wi));
        let ui = &self.ui;
        let icon = ui.pt(self.dock_preferences().size).round() as i32;
        let pad = ui.pt(DOCK_PAD_PT).round() as i32;
        let gap = ui.pt(DOCK_GAP_PT).round() as i32;
        let sep = ui.pt(DOCK_SEP_PT).round() as i32;
        let available=if self.dock_preferences().position==super::desktop_dock::Position::Bottom {m.w}else{m.h-m.menubar};
        let icon=icon.min(((available-2*pad-2*ui.pt(DOCK_MARGIN_PT) as i32-2*sep)/(list.len().max(1) as i32)-gap).max(ui.pt(24.) as i32));
        // the Downloads stack and the Trash are last, after the last divider
        list.push(super::dock_stack::DOCK_STACK);
        list.push(DOCK_TRASH);
        let seps = self.dock_seps(&list);
        let (bar, rects) = dock_rects_seps(m, icon, pad, gap, list.len(), &seps, sep);
        // sit a margin above the bottom edge
        let lift = ui.pt(DOCK_MARGIN_PT).round() as i32 - m.dock_bottom / 4;
        let up = |r: Rect| Rect { y: r.y + lift, ..r };
        let bar=up(bar);let rects:Vec<Rect>=rects.into_iter().map(up).collect();
        let position=self.dock_preferences().position;
        if position==super::desktop_dock::Position::Bottom {return (list,bar,rects);}
        let left=position==super::desktop_dock::Position::Left;let x=if left {ui.pt(DOCK_MARGIN_PT) as i32}else{m.w-bar.h-ui.pt(DOCK_MARGIN_PT) as i32};let y=m.menubar+(m.h-m.menubar-bar.w)/2;
        let rotate=|r:Rect|Rect{x:x+(r.y-bar.y),y:y+(r.x-bar.x),w:r.h,h:r.w};
        (list,Rect{x,y,w:bar.h,h:bar.w},rects.into_iter().map(rotate).collect())
    }

    /// Runs `f` with the shell sized as a `w` x `h` px window body.
    fn with_size<R>(&mut self, w: i32, h: i32, f: impl FnOnce(&mut Self) -> R) -> R {
        let (ow, oh) = (self.w, self.h);
        self.w = w.max(1);
        self.h = h.max(1);
        let r = f(self);
        self.w = ow;
        self.h = oh;
        r
    }

    pub(super) fn desk_app_size(&self, app: AppId) -> Option<(i32, i32)> {
        let m = self.desk_metrics();
        let w = self.desk.as_ref()?.wins.iter().find(|w| w.app == app)?;
        let body = w.body(&m);
        Some((body.w, body.h))
    }

    /// The window's app gets the body size.
    pub(super) fn desk_layout(&mut self, i: usize) {
        let m = self.desk_metrics();
        let Some(w) = self.desk.as_ref().and_then(|d| d.wins.get(i).copied()) else { return };
        let b = w.body(&m);
        self.resize_app(w.app, b.w, b.h);
        if w.app == AppId::Terminal {
            self.with_size(b.w, b.h, |sh| {
                sh.resize_terminals();
            });
        }
        if w.app == AppId::Files {
            if let Some(f) = &mut self.files {
                f.set_size(b.w, b.h);
            }
        }
        self.dirty = true;
    }

    /// Opens `app` in a window (or raises it) and makes it the focused app.
    pub(super) fn desk_open(&mut self, app: AppId, t: f64) {
        self.recent_note(app);
        if let Some(d)=self.desk.as_mut(){d.raise_front=true;}
        // the Dock tile bounces while the app launches, whichever way it was opened (a running app starts none)
        if let Some(k)=apps().iter().position(|a|a.id==app){self.dock_start_bounce(k,t);}
        #[cfg(feature = "desktop-wayland")]
        if let AppId::External(slot)=app { if self.desk_direct_open(slot as usize) { return; } }
        #[cfg(feature = "desktop-wayland")]
        if app == AppId::Browser {
            if let Some(slot)=apphost::slot_of("is.olibuijr.browser.desktop") {self.desk_native_open(AppId::External(slot as u8));return;}
        }
        #[cfg(feature = "desktop-wayland")]
        if self.desk_native_open(app) { return; }
        let app = if app == AppId::Files {
            apphost::slot_of("is.olibuijr.files.desktop").map(|i| AppId::External(i as u8)).unwrap_or(AppId::AppStore)
        } else { app };
        let m = self.desk_metrics();
        let ui = &self.ui;
        let (w, h) = match app {
            AppId::Terminal => (ui.pt(820.0), ui.pt(540.0)),
            AppId::Settings => (ui.pt(740.0), ui.pt(690.0)),
            _ => (ui.pt(960.0), ui.pt(660.0)),
        };
        let (w, h) = ((w as i32).min(m.w * 9 / 10), (h as i32).min(m.h * 85 / 100));
        // the manifest's window_size (content points) replaces the default size
        let (w, h) = size_limits(app).initial.map(|s| outer_px(app, &m, s)).unwrap_or((w, h));
        // a window opened again comes back at the size it was closed with; a fixed manifest size still wins
        let (w, h) = match (size_limits(app).initial, self.desk.as_ref().and_then(|d| d.closed_sizes.get(&app).copied())) {
            (None, Some(closed)) => closed,
            _ => (w, h),
        };
        let existed = self.desk.as_ref().is_some_and(|d| d.index_of(app).is_some());
        let was_minimized = self.desk.as_ref().and_then(|d| d.index_of(app).map(|i| d.wins[i].minimized)).unwrap_or(false);
        if !existed {
            let (bw, bh) = (w, h - title_of(app, &m));
            self.with_size(bw, bh, |sh| sh.init_app(app, t));
        }
        let i = self.desk.as_mut().unwrap().open(&m, app, (w, h));
        if !existed {
            self.desk_layout(i);
            self.desk_fx_start(app, FxCall::Open, t);
        } else if was_minimized {
            self.desk_fx_start(app, FxCall::Restore, t);
        }
        self.quicksearch.note_launch(app);
        self.clear_notifications(app);
        self.fullscreen = false;
        self.screen = Screen::App(app);
        // the phone dock's recents are not kept on the desktop: Recent Items has its own list
        self.dirty = true;
    }

    /// Focus follows the frontmost visible window; with none, the desktop (Home) shows.
    pub(super) fn desk_refocus(&mut self) {
        self.screen = match self.desk.as_ref().and_then(|d| d.front()).map(|w| w.app) {
            Some(app) => Screen::App(app),
            None => Screen::Home,
        };
        self.dirty = true;
    }

    /// Alt/Ctrl/Meta+Tab on the desktop: raises the visible window behind the front one,
    /// so repeated presses toggle between the two most recent windows.
    #[cfg(any(test, feature = "desktop-wayland"))]
    pub(super) fn desk_cycle_window(&mut self) {
        if let Some(d) = &mut self.desk {
            match &mut d.cmdtab {
                Some((order, sel)) => *sel = (*sel + 1) % order.len(),
                None => {
                    let order: Vec<AppId> = d.wins.iter().rev().map(|w| w.app).collect();
                    if order.len() >= 2 {
                        d.cmdtab = Some((order, 1));
                    }
                }
            }
        }
        self.dirty = true;
    }

    /// The last modifier went up: the selected window comes to the front.
    pub fn cmdtab_release(&mut self, t: f64) {
        let Some((order, sel)) = self.desk.as_mut().and_then(|d| d.cmdtab.take()) else { return };
        self.desk_open(order[sel], t);
        self.dirty = true;
    }

    /// The Cmd+Tab strip: a glass capsule of app icons, the selection on a plate with its name below.
    fn draw_cmdtab(&self, s: &mut Surface) {
        let Some((order, sel)) = self.desk.as_ref().and_then(|d| d.cmdtab.as_ref()) else { return };
        let ui = &self.ui;
        let (icon, gap, pad) = (ui.pt(72.0) as usize, ui.pt(14.0), ui.pt(18.0));
        let n = order.len() as f32;
        let (w, h) = (n * icon as f32 + (n - 1.0) * gap + 2.0 * pad, icon as f32 + 2.0 * pad + ui.pt(26.0));
        let (x, y) = (((s.w as f32 - w) / 2.0).round(), ((s.h as f32 - h) / 2.0).round());
        s.fill_rrect(x, y, w, h, ui.pt(34.0), MENU_TINT.now());
        s.stroke_rrect(x, y, w, h, ui.pt(34.0), ui.pt(1.0).max(1.0), SEPARATOR_C.now());
        for (k, app) in order.iter().enumerate() {
            let ix = x + pad + k as f32 * (icon as f32 + gap);
            let Some(i) = apps().iter().position(|a| a.id == *app) else { continue };
            if k == *sel {
                let g = gap / 2.0;
                s.fill_rrect(ix - g, y + pad - g, icon as f32 + gap, icon as f32 + gap, ui.pt(18.0), FIELD_BG.now());
                self.label(apps()[i].name, MAC_BODY_PT, Weight::Regular, WHITE, |im| {
                    s.blit_colored(im, (ix + icon as f32 / 2.0 - im.w as f32 / 2.0) as i32, (y + pad + icon as f32 + ui.pt(10.0)) as i32, LABEL.now(), 255);
                });
            }
            let base = &self.base_icons[i];
            let img = if base.w == icon { base.clone() } else { base.resized(icon, icon) };
            s.blit(&img, ix as i32, (y + pad) as i32, 255, 255);
        }
    }

    /// Hides every window and shows the desktop.
    pub(super) fn desk_show_desktop(&mut self) {
        if let Some(d) = &mut self.desk {
            for w in &mut d.wins {
                w.minimized = true;
            }
        }
        self.desk_refocus();
    }

    pub(super) fn desk_close_app(&mut self, app: AppId) {
        #[cfg(feature = "desktop-wayland")]
        if let AppId::External(slot)=app { if self.ext.get(slot as usize).and_then(Option::as_ref).is_some_and(|h|h.route()==apphost::Route::Direct) { self.ext_close(slot as usize); return; } }
        // the close effect starts while the window is still listed; its last frame is kept for the effect
        let t = self.last_input;
        self.desk_fx_start(app, FxCall::Close, t);
        #[cfg(feature = "desktop-wayland")]
        self.desk_keep_ghost(app);
        if let Some(d) = &mut self.desk {
            if let Some(i) = d.index_of(app) {
                d.close(i);
                self.heap_trim_due = true;
            }
        }
        self.desk_refocus();
    }

    /// Touches over the desktop. `None` leaves the event to the normal path.
    pub(super) fn desk_event(&mut self, ev: Event, t: f64) -> Option<Vec<Effect>> {
        let m = self.desk_metrics();
        let (list, bar0, icons) = self.desk_dock(&m);
        let (dock_bar, icons) = self.dock_live(bar0, &icons, t / 1000.0);
        match ev {
            Event::Down(x, y) => {
                self.desk.as_mut()?.grab = None;
                // the dock sits over the windows
                if let Some(k) = icons.iter().position(|r| self.dock_is_visible()&&r.contains(x, y)) {
                    self.desk.as_mut()?.grab = Some(Grab::DockDrag{from:k,x,y,at_x:x,at_y:y,moved:false});
                    self.desk.as_mut()?.dock_drop = None;
                    // an app that already has a window comes forward on the press, not after the release
                    if let Some(a) = apps().get(list[k]).map(|a| Self::dock_app(a.id)) {
                        if self.desk.as_ref()?.index_of(a).is_some() { self.desk_open(a, t); }
                    }
                    self.dirty = true;
                    return Some(vec![]);
                }
                let picked = self.desk.as_ref()?.pick(&m, x, y);
                let Some((i, hit)) = picked else { return Some(vec![]) };
                let i = {
                    let d = self.desk.as_mut()?;
                    d.raise(i)
                };
                let app = self.desk.as_ref()?.wins[i].app;
                self.screen = Screen::App(app);
                self.fullscreen = false;
                let w = self.desk.as_ref()?.wins[i];
                let mut hit = hit;
                if hit == Hit::Title {
                    if let AppId::External(slot) = app {
                        let controls = self.ext.get(slot as usize).and_then(Option::as_ref).map_or(&[][..], |h| &h.controls[..]);
                        let (_, holes) = drag_zone(&w, &m, controls);
                        let (lx, ly) = ((x - w.r.x as f32) as i32, (y - w.r.y as f32) as i32);
                        if full_size(app) && holes.iter().skip(1).any(|r| lx >= r[0] && lx < r[0] + r[2] && ly >= r[1] && ly < r[1] + r[3]) {
                            hit = Hit::Content;
                        }
                    }
                }
                if hit == Hit::Title {
                    let d = self.desk.as_mut()?;
                    if d.last_title.is_some_and(|(a, t0)| a == app && t - t0 <= DOUBLE_CLICK_MS) {
                        d.last_title = None;
                        d.toggle_zoom(&m, i);
                        self.desk_layout(i);
                        self.dirty = true;
                        return Some(vec![]);
                    }
                    d.last_title = Some((app, t));
                }
                let grab = match hit {
                    Hit::Light(l) => Grab::Light(i, l),
                    Hit::Title => Grab::Move { dx: x - w.r.x as f32, dy: y - w.r.y as f32 },
                    Hit::Edge(edges) => Grab::Resize { edges, start: w.r, x, y },
                    Hit::Content => {
                        self.desk.as_mut()?.grab = Some(Grab::Content(i));
                        let b = w.body(&m);
                        let fx = self.with_size(b.w, b.h, |sh| sh.app_touch_single(app, Event::Down(x - b.x as f32, y - b.y as f32), t));
                        return Some(fx);
                    }
                };
                self.desk.as_mut()?.grab = Some(grab);
                self.dirty = true;
                Some(vec![])
            }
            Event::Move(x, y) => {
                // a press that landed on the empty desktop has no grab: swallow the drag like its Down and Up
                let Some(grab) = self.desk.as_ref()?.grab else { return Some(vec![]) };
                match grab {
                    Grab::Move { dx, dy } => {
                        let i = self.desk.as_ref()?.wins.len().checked_sub(1)?;
                        self.desk.as_mut()?.move_to(&m, i, (x - dx) as i32, (y - dy) as i32);
                        self.dirty = true;
                        Some(vec![])
                    }
                    Grab::Resize { edges, start, x: x0, y: y0 } => {
                        let i = self.desk.as_ref()?.wins.len().checked_sub(1)?;
                        self.desk.as_mut()?.resize_to(&m, i, edges, start, (x - x0) as i32, (y - y0) as i32);
                        self.desk_layout(i);
                        Some(vec![])
                    }
                    Grab::Content(i) => {
                        let w = *self.desk.as_ref()?.wins.get(i)?;
                        let b = w.body(&m);
                        let fx = self.with_size(b.w, b.h, |sh| sh.app_touch_single(w.app, Event::Move(x - b.x as f32, y - b.y as f32), t));
                        Some(fx)
                    }
                    Grab::DockDrag{from,x:x0,y:y0,..}=>{
                        let moved=(x-x0).abs()+(y-y0).abs()>self.ui.pt(8.);
                        let desk=self.desk.as_mut()?;
                        desk.grab=Some(Grab::DockDrag{from,x:x0,y:y0,at_x:x,at_y:y,moved});
                        if moved {
                            let centres:Vec<(f32,f32)>=icons.iter().map(|r|((r.x+r.w/2) as f32,(r.y+r.h/2) as f32)).collect();
                            if let Some(slot)=bifrost_ui::dockdrag::slot_at(&centres,x,y) {
                                desk.dock_drop=Some(slot);
                                let order=bifrost_ui::dockdrag::preview_order(icons.len(),from,slot);
                                // icon i shows where slot (its place in the preview) is
                                let targets:Vec<(f32,f32)>=(0..icons.len()).map(|i|{let at=order.iter().position(|&o|o==i).unwrap_or(i);((icons[at].x-icons[i].x) as f32,(icons[at].y-icons[i].y) as f32)}).collect();
                                desk.dock_slide.borrow_mut().set_targets(t/1000.,&targets);
                            }
                        }
                        self.dirty=true;Some(vec![])
                    },
                    Grab::Light(..) => Some(vec![]),
                }
            }
            Event::Up(x, y) => {
                let grab = self.desk.as_mut()?.grab.take();
                self.dirty = true;
                match grab {
                    Some(Grab::Content(i)) => {
                        let w = *self.desk.as_ref()?.wins.get(i)?;
                        let b = w.body(&m);
                        let fx = self.with_size(b.w, b.h, |sh| sh.app_touch_single(w.app, Event::Up(x - b.x as f32, y - b.y as f32), t));
                        Some(fx)
                    }
                    Some(Grab::Light(i, l)) => {
                        let w = *self.desk.as_ref()?.wins.get(i)?;
                        if w.hit(&m, x, y) == Some(Hit::Light(l)) {
                            match l {
                                Light::Close => self.desk_close_app(w.app),
                                Light::Minimize => {
                                    self.desk_minimize_window(i, t);
                                }
                                Light::Zoom => {
                                    self.desk.as_mut()?.toggle_zoom(&m, i);
                                    self.desk_layout(i);
                                    self.desk_fx_start(w.app, FxCall::Zoom, t);
                                }
                            }
                        }
                        Some(vec![])
                    }
                    Some(Grab::Move { .. }) => {
                        // a title bar dropped on a screen edge or corner tiles the window there (snap_zone)
                        let band = self.ui.pt(8.).round() as i32;
                        if let (Some(tile), Some(i)) = (snap_zone(Desk::area(&m), x as i32, y as i32, band), self.desk.as_ref()?.wins.len().checked_sub(1)) {
                            if let Some(d) = self.desk.as_mut() { d.tile(&m, i, tile); }
                            self.desk_layout(i);
                            self.desk_frame_changed(i);
                        }
                        Some(vec![])
                    }
                    Some(Grab::DockDrag{from:k,x:x0,y:y0,..}) if (x-x0).abs()+(y-y0).abs()>self.ui.pt(8.) => {
                        // the neighbours already show the final order, so dropping them in place is seamless
                        let slot=self.desk.as_mut()?.dock_drop.take();
                        self.desk.as_ref()?.dock_slide.borrow_mut().clear();
                        // an app pinned in the Dock and dragged well off it leaves the Dock (its Pin toggles off)
                        if slot.is_none() && self.dock_removable(list[k]) && super::desktop_dock::dock_drag_off(dock_bar, x, y, self.ui.pt(DOCK_REMOVE_PT)) {
                            self.dock_change(super::desktop_dock::Change::Pin(list[k]), t);
                        } else if let Some(target)=slot{self.dock_reorder(list[k],list[target],target>k);}
                        Some(vec![])
                    },
                    Some(Grab::DockDrag{from:k,..}) => {
                        if icons.get(k).is_some_and(|r| r.contains(x, y)) {
                            if list[k] == DOCK_TRASH {
                                // the account's Trash as Files keeps it; the shell runs as root, so not $HOME, and Files creates it
                                let trash=crate::files::trash::dir(&super::dock_stack::home_dir());
                                if let Err(e)=self.desktop_open_folder(FILES_APP,&trash.to_string_lossy(),t){eprintln!("BifrOSt Trash: {e}");}
                                return Some(vec![]);
                            }
                            if list[k] == super::dock_stack::DOCK_STACK {
                                // the Downloads stack opens its folder in Files
                                let folder = super::dock_stack::downloads_dir();
                                if let Err(e)=self.desktop_open_folder(FILES_APP,&folder.to_string_lossy(),t){eprintln!("BifrOSt Downloads: {e}");}
                                return Some(vec![]);
                            }
                            if let Some(wi) = super::desktop_dock::dock_min_index(list[k]) {
                                // a minimized window comes back from its slot
                                let app = self.desk.as_ref()?.wins.get(wi)?.app;
                                self.desk_open(app, t);
                                return Some(vec![]);
                            }
                            #[cfg(feature = "desktop-wayland")]
                            if list[k] > apps().len() {
                                let app=&self.desk.as_ref()?.linux_apps[list[k]-apps().len()-1];
                                if app.windows.is_empty() {
                                    let id = app.app_id.clone();
                                    self.dock_start_bounce(list[k], t);
                                    crate::bare::sys::spawn_named("launch-apps", move || { let _ = std::process::Command::new("/usr/bin/bifrost-apps").arg(id).status(); });
                                    return Some(vec![]);
                                }
                                let next=app.windows.iter().position(|w|w.active).map(|i|(i+1)%app.windows.len()).unwrap_or(0);
                                return Some(vec![Effect::ActivateForeign(app.windows[next].id)]);
                            }
                            if list[k] == apps().len() {
                                // the Apps icon opens the Apps grid
                                self.appgrid_toggle(t);
                                return Some(vec![]);
                            }
                            let app = apps()[list[k]].id;
                            self.dock_start_bounce(list[k], t);
                            self.desk_dock_tap(app, t);
                        }
                        Some(vec![])
                    }
                    _ => Some(vec![]),
                }
            }
            _ => None,
        }
    }

    #[cfg(feature = "desktop-wayland")]
    fn desk_native_open(&mut self,app:AppId) -> bool {
        let AppId::External(slot)=app else {return false};
        let registered=apphost::installed();
        let Some(package)=registered.get(slot as usize).filter(|a|a.manifest.window=="native") else {return false};
        let Some(desk)=self.desk.as_mut() else {return false};
        let windows:Vec<_>=desk.native_windows.iter().filter(|w|w.app_id==package.manifest.wayland_app_id).collect();
        if !windows.is_empty() {
            let next=windows.iter().position(|w|w.active).map(|i|(i+1)%windows.len()).unwrap_or(0);
            desk.activate.push(windows[next].id); return true;
        }
        if desk.native_children.get_mut(&package.manifest.id).is_some_and(|c|matches!(c.try_wait(),Ok(None))) {return true;}
        let launch=crate::bare::app_policy::Policy::from_permissions(&package.manifest.id,&package.manifest.permissions).and_then(|policy|crate::bare::app_policy::command(&package.manifest.id,&package.exec_path(),&apphost::root().join("appdata").join(&package.manifest.id),policy,true)).and_then(|mut cmd|cmd.spawn());
        match launch {
            Ok(child) => {desk.native_children.insert(package.manifest.id.clone(),child);}
            Err(e) => eprintln!("BifrOSt native app {}: {e}",package.manifest.id),
        }
        self.dirty=true;
        true
    }
    /// A dock icon: open it, bring it forward, or hide it when it already has the focus.
    fn desk_dock_tap(&mut self, app: AppId, t: f64) {
        #[cfg(feature = "desktop-wayland")]
        if app == AppId::Browser {
            if let Some(slot)=apphost::slot_of("is.olibuijr.browser.desktop") {self.desk_native_open(AppId::External(slot as u8));return;}
        }
        #[cfg(feature = "desktop-wayland")]
        if self.desk_native_open(app) { return; }
        let app = if app == AppId::Files {
            apphost::slot_of("is.olibuijr.files.desktop").map(|i| AppId::External(i as u8)).unwrap_or(AppId::AppStore)
        } else { app };
        // Golden Gate: clicking the frontmost app's icon does nothing; minimizing is the yellow light or Cmd+M
        self.desk_open(app, t);
    }

    /// The window app a dock icon stands for (the Files icon opens the installed Files app).
    pub(super) fn dock_app(app: AppId) -> AppId {
        if app == AppId::Files {
            apphost::slot_of("is.olibuijr.files.desktop").map(|i| AppId::External(i as u8)).unwrap_or(AppId::AppStore)
        } else { app }
    }

    /// Desktop backdrop for every overlay and transition; no bifrost grid.
    pub(super) fn draw_desktop_background(&self, s: &mut Surface) {
        self.draw_wallpaper_layer(s);
        self.draw_app_status(s);
        self.draw_desk_dock(s, &self.desk_metrics(), 1e15); // no clock here: slides show where they end
        #[cfg(feature="desktop-wayland")]
        self.draw_desktop_popup(s);
    }

    /// The wallpaper and where it sits on screen, for the GPU presenter.

    /// The wallpaper under the desktop: copied on the CPU, or left transparent when the GPU
    /// presenter draws it (and the glass blur) under the content layer.
    fn draw_wallpaper_layer(&self, s: &mut Surface) {
        if bifrost_render::glass::gpu() {
            // a partial desktop frame clears only the regions it redraws; the rest stays transparent
            match &self.desk_clear {
                Some(rects) => for &(x, y, w, h) in rects {
                    let (x0, y0) = (x.max(0), y.max(0));
                    let (x1, y1) = ((x + w).min(s.w as i32), (y + h).min(s.h as i32));
                    if x1 > x0 && y1 > y0 { s.fill_rect(x0 as usize, y0 as usize, (x1 - x0) as usize, (y1 - y0) as usize, 0); }
                },
                None => s.fill_rect(0, 0, s.w, s.h, 0),
            }
            return;
        }
        s.fill_rect(0, 0, s.w, s.h, BLACK);
        let wp = self.home_still();
        s.copy(wp, (self.w - wp.w as i32) / 2, 0);
    }

    /// A minimized window whose minimize effect still runs: it stays drawn until the effect ends.
    fn minimizing_at(&self, i: usize, now_s: f64) -> bool {
        let Some(d) = self.desk.as_ref() else { return false };
        let Some(w) = d.wins.get(i) else { return false };
        w.minimized && cfg!(feature = "desktop-wayland") && win_id(w.app).is_some_and(|id| d.fx.keeps_visible(id, now_s))
    }

    /// Each window's cover, back to front, for `desk_occlusion::hidden`. Minimized windows, windows with an effect
    /// running and external windows neither cover nor get covered (`None`).
    fn desk_covers(&self, now_s: f64) -> Vec<Option<Cover>> {
        let Some(d) = self.desk.as_ref() else { return Vec::new() };
        let reach = self.ui.pt(desk_occlusion::SHADOW_REACH_PT).ceil() as i32;
        d.wins.iter().map(|w| {
            let animating = win_id(w.app).is_some_and(|id| d.fx.keeps_visible(id, now_s));
            if w.minimized || animating || matches!(w.app, AppId::External(_)) {
                return None;
            }
            let radius = if w.maximized { 0 } else { self.ui.pt(bifrost_ui::prefs_layout::m::WINDOW_RADIUS).ceil() as i32 };
            Some(Cover { shown: grown(w.r, reach), opaque: opaque_interior(w.r, radius) })
        }).collect()
    }

    /// The wallpaper, the windows back to front, the menu bar and the dock.
    pub(super) fn draw_desk(&mut self, s: &mut Surface, t: f64) {
        let m = self.desk_metrics();
        self.draw_wallpaper_layer(s);
        // windows are read by index, so a frame copies no window list
        let count = self.desk.as_ref().map_or(0, |d| d.wins.len());
        // a window that is minimizing stays drawn until its effect ends
        let now_s = t / 1000.0;
        let fading: Vec<bool> = (0..count).map(|i| self.minimizing_at(i, now_s)).collect();
        // a built-in window that a window in front covers completely is not drawn (see `desk_occlusion`)
        let hidden = desk_occlusion::hidden(&self.desk_covers(now_s));
        #[cfg(feature = "desktop-wayland")]
        {
            let desk = self.desk.as_mut().unwrap();
            desk.window_cache.retain(|id,_| desk.wins.iter().zip(&fading).any(|(w, f)| (!w.minimized || *f) && apps()[(*id-1) as usize].id == w.app));
            desk.body_cache.retain(|app,_| desk.wins.iter().zip(&fading).any(|(w, f)| (!w.minimized || *f) && w.app == *app));
        }
        let front = self.desk.as_ref().and_then(|d| d.wins.iter().rposition(|w| !w.minimized));
        for i in 0..count {
            let Some(win) = self.desk.as_ref().and_then(|d| d.wins.get(i).copied()) else { break };
            let w = &win;
            if w.minimized && !fading[i] {
                continue;
            }
            #[cfg(feature = "desktop-wayland")]
            if std::env::var_os("SPRINGBOARD_WAYLAND").is_some() {
                let id = apps().iter().position(|a| a.id == w.app).unwrap() as u32 + 1;
                let focused = Some(i) == front;
                let hover = self.desk.as_ref().unwrap().lights_hover == Some(w.app);
                let mut cached = self.desk.as_mut().unwrap().window_cache.remove(&id);
                let unchanged = cached.as_ref().is_some_and(|c|
                    c.width == w.r.w as usize && c.height == w.r.h as usize && c.focused == focused && c.maximized == w.maximized && c.hover == hover);
                // only the key state moved (Cmd-Tab, a click on a back window): a full-size app's window is its body plus
                // the lights over it, so the body is copied again from the app's last frame (dimmed when it lost focus)
                // instead of drawing the whole window afresh (a 31 ms frame on the lab PC, round 75)
                let refocus = !unchanged && full_size(w.app) && matches!(w.app, AppId::External(_)) && cached.as_ref().is_some_and(|c|
                    c.width == w.r.w as usize && c.height == w.r.h as usize && c.focused != focused && c.maximized == w.maximized && c.hover == hover);
                if self.keyboard.shown() || self.keyboard.animating() {
                    self.desk.as_mut().unwrap().body_cache.remove(&w.app);
                }
                let quiet = match w.app {
                    AppId::External(slot) => self.ext.get(slot as usize).and_then(Option::as_ref)
                        .is_some_and(|h| h.running() && h.frame().is_some() && !h.has_pending()),
                    // A focused Settings window keeps its pixels until input, a reported change or an animation
                    // makes them stale (it was redrawn in full on every desk frame: ~23 ms at 2560x1440 on Midget,
                    // round 75); it still refreshes once a second like the background built-ins below.
                    AppId::Settings if focused => cached.as_ref().is_some_and(|c| t - c.drawn < 1000. && c.drawn > self.last_input && c.drawn > self.settings_changed)
                        && self.settings.as_ref().is_some_and(|st| st.next_wake(t).map_or(true, |w| w > t)),
                    // Background built-ins refresh once a second for status/jobs;
                    // input/focus changes always bypass this cache.
                    _ => !focused && cached.as_ref().is_some_and(|c| t - c.drawn < 1000.),
                } && !self.keyboard.shown() && !self.keyboard.animating();
                let (drag_height,no_drag)={
                    let controls=match w.app {AppId::External(slot)=>self.ext.get(slot as usize).and_then(Option::as_ref).map_or(&[][..],|h|&h.controls[..]),_=>&[][..]};
                    drag_zone(w,&m,controls)
                };
                let corner=self.ui.pt(bifrost_ui::prefs_layout::m::WINDOW_RADIUS);
                // shell-drawn content is opaque (no vibrancy, see `take_wayland_materials`); a client's may not be
                let opaque=!matches!(w.app,AppId::External(_));
                if hidden[i] {
                    // a window in front covers it: its surface keeps the last pixels presented, and its cache is
                    // dropped so that it repaints in full once it is uncovered
                    self.desk.as_mut().unwrap().frames.push(crate::bare::wayland::WindowFrame {
                        id, title: apps()[(id-1) as usize].name, x:w.r.x, y:w.r.y,
                        width:w.r.w as usize, height:w.r.h as usize, pixels:None,
                        damage:crate::gfx::Damage::new(w.r.w as usize,w.r.h as usize), maximized:w.maximized, title_height:m.title,
                        drag_height, no_drag, fx:bifrost_render::glass::Fx::IDENTITY, opaque,
                    });
                    continue;
                }
                if unchanged && quiet {
                    self.desk.as_mut().unwrap().frames.push(crate::bare::wayland::WindowFrame {
                        id, title: apps()[(id-1) as usize].name, x:w.r.x, y:w.r.y,
                        width:w.r.w as usize, height:w.r.h as usize, pixels:None,
                        damage:crate::gfx::Damage::new(w.r.w as usize,w.r.h as usize), maximized:w.maximized, title_height:m.title,
                        drag_height, no_drag, fx:bifrost_render::glass::Fx::IDENTITY, opaque,
                    });
                    self.desk.as_mut().unwrap().window_cache.insert(id,cached.unwrap());
                    continue;
                }
                if let AppId::External(slot)=w.app {
                    let width=w.r.w as usize; let height=w.r.h as usize; let top=title_of(w.app,&m) as usize;
                    let body_h=height.saturating_sub(top);
                    let mapped=self.ext.get(slot as usize).and_then(Option::as_ref).filter(|h|h.running()).and_then(|h|h.frame()).map(|(_,w,h)|(w,h));
                    if (unchanged || refocus) && !self.keyboard.shown() && !self.keyboard.animating() && mapped==Some((width,body_h)) {
                        let mut retained=cached.take().unwrap();
                        retained.focused=focused;
                        let pixels=std::sync::Arc::make_mut(&mut retained.pixels);
                        let mut body_damage=crate::gfx::Damage::new(width,body_h);
                        let mut body=Surface{w:width,h:body_h,stride:width,px:&mut pixels[top*width..],y0:0};
                        if refocus {
                            if let Some(Some(h))=self.ext.get_mut(slot as usize) { body_damage.union(&h.blit(&mut body,true)); }
                        } else {
                            self.ext_blit_damage(&mut body,slot as usize,&mut body_damage);
                        }
                        let mut damage=crate::gfx::Damage::new(width,height);
                        for a in body_damage.areas() {damage.add(a.x0 as i32,(a.y0+top) as i32,(a.x1-a.x0) as i32,(a.y1-a.y0) as i32);}
                        if !focused { for a in body_damage.areas() { dim_rect(pixels, width, a.x0, a.y0 + top, a.x1, a.y1 + top, BACKGROUND_DIM); } }
                        if full_size(w.app) {
                            // the lights sit over the app: repaint them where the app just drew under them
                            let mut local=*w; local.r.x=0; local.r.y=0;
                            let mut surf=Surface{w:width,h:height,stride:width,px:&mut pixels[..],y0:0};
                            for (k,c) in [SYSTEM_RED, SYSTEM_YELLOW, SYSTEM_GREEN].into_iter().enumerate() {
                                let lr=local.light_rect_px(&m,k);
                                let under=body_damage.areas().iter().any(|a| (a.x0 as i32) < lr.x+lr.w && (a.x1 as i32) > lr.x && ((a.y0+top) as i32) < lr.y+lr.h && ((a.y1+top) as i32) > lr.y);
                                if !under { continue; }
                                draw_light(&mut surf,&lr,if focused {c} else {TRAFFIC_INACTIVE_C.now()},focused,self.ui.pt(1.));
                                if focused && hover { draw_light_glyph(&mut surf,&lr,k,w.maximized,self.ui.pt(1.)); }
                                damage.add(lr.x-2,lr.y-2,lr.w+4,lr.h+4);
                            }
                        }
                        if !w.maximized {
                            // the app drew up to the edge: put the dark-mode inner rim back over it
                            if body_damage.areas().iter().any(|a|a.x0==0||a.x1>=width||a.y1>=body_h) {
                                window_rim(&mut Surface{w:width,h:height,stride:width,px:&mut pixels[..],y0:0},0.,0.,width as f32,height as f32,corner);
                                for (x,y,dw,dh) in [(0,0,2,height),(width-2,0,2,height),(0,height-2,width,2)] {damage.add(x as i32,y as i32,dw as i32,dh as i32);}
                            }
                            clip_window_corners(pixels,width,height,corner,Some(&damage));
                        }
                        retained.drawn=t;
                        let outgoing=(!damage.is_empty()).then(||retained.pixels.clone());
                        let desk=self.desk.as_mut().unwrap();
                        desk.body_cache.remove(&w.app);
                        desk.window_cache.insert(id,retained);
                        desk.frames.push(crate::bare::wayland::WindowFrame{id,title:apps()[(id-1) as usize].name,x:w.r.x,y:w.r.y,width,height,pixels:outgoing,damage,maximized:w.maximized,title_height:m.title,drag_height,no_drag,fx:bifrost_render::glass::Fx::IDENTITY,opaque});
                        continue;
                    }
                }
                // Reuse the shared scratch buffer instead of a fresh zeroed allocation per window per frame.
                let mut px = std::mem::take(&mut self.desk.as_mut().unwrap().window_buf);
                px.clear(); px.resize(w.r.w as usize * w.r.h as usize, 0);
                let mut image = Image { w: w.r.w as usize, h: w.r.h as usize, px };
                let mut local = *w; local.r.x = 0; local.r.y = 0;
                self.draw_window(&mut Surface { w:image.w,h:image.h,stride:image.w,px:&mut image.px,y0:0 }, &local, Some(i) == front, &m, t, false);
                if !w.maximized { clip_corners(&mut image,corner); }
                let previous = cached.as_ref().filter(|c| c.width == image.w && c.height == image.h).map_or(&[][..], |c| c.pixels.as_ref());
                let damage = crate::gfx::Damage::between(previous, &image.px, image.w, image.h);
                let pixels:std::sync::Arc<[u32]> = if damage.is_empty() {cached.unwrap().pixels} else {std::sync::Arc::from(&image.px[..])};
                self.desk.as_mut().unwrap().window_cache.insert(id,CachedWindow {
                    width:image.w,height:image.h,pixels:pixels.clone(),focused,maximized:w.maximized,hover,drawn:t,
                });
                let pixels = (!damage.is_empty()).then_some(pixels);
                let desk = self.desk.as_mut().unwrap();
                desk.window_buf = image.px;
                desk.frames.push(crate::bare::wayland::WindowFrame {
                    id, title: apps()[(id-1) as usize].name, x:w.r.x, y:w.r.y,
                    width:w.r.w as usize, height:w.r.h as usize, pixels, damage, maximized:w.maximized, title_height:m.title,
                    drag_height, no_drag, fx:bifrost_render::glass::Fx::IDENTITY, opaque,
                });
                continue;
            }
            // on the GPU presenter a partial frame redraws only some regions, so a hidden window is skipped only on the CPU
            if hidden[i] && !bifrost_render::glass::gpu() {
                continue;
            }
            self.draw_window(s, w, Some(i) == front, &m, t, true);
        }
        #[cfg(feature = "desktop-wayland")]
        if let Some(d) = self.desk.as_mut() {
            let fx = &d.fx;
            d.ghosts.retain(|g| fx.keeps_visible(g.1.id, now_s));
            for (z, frame) in d.ghosts.iter() {
                let at = (*z).min(d.frames.len());
                d.frames.insert(at, frame.clone());
            }
        }
        #[cfg(feature = "desktop-wayland")]
        {
            // stamp each window's effect transform; the presenter applies it inside the window's drawable
            let shift = self.ui.pt(40.0) as f32;
            if let Some(d) = self.desk.as_mut() {
                let fx = &d.fx;
                for f in &mut d.frames { f.fx = fx.fx(f.id, now_s, shift); }
            }
        }
        self.draw_app_status(s);
        self.draw_desk_dock(s, &m, t);
        #[cfg(feature="desktop-wayland")]
        self.draw_desktop_popup(s);
        self.draw_quick_look(s);
        self.draw_cmdtab(s);
        // Developer > Performance HUD: the desktop branch never reached `draw_assistive`, so it never showed
        self.draw_perf(s);
    }

    /// `shadow` is false for a window-local image: the presenter draws the shadow around it.
    fn draw_window(&mut self, s: &mut Surface, w: &Win, focused: bool, m: &Metrics, t: f64, shadow: bool) {
        let ui = &self.ui;
        let r = w.r;
        let rad = if w.maximized { 0.0 } else { ui.pt(bifrost_ui::prefs_layout::m::WINDOW_RADIUS) };
        // shadow (CPU path; the GPU presenter draws its own): Golden Gate's key window casts a
        // large soft shadow (~40 pt blur, ~16 pt down, ~30% under the bottom edge), others a light one
        if !w.maximized && shadow {
            let (layers, step, dy, a) = if focused { (8, 5.0, 12.0, 9) } else { (4, 4.0, 5.0, 8) };
            for k in (0..layers).rev() {
                let g = ui.pt(step) * (k + 1) as f32;
                s.fill_rrect(r.x as f32 - g, r.y as f32 - g + ui.pt(dy), r.w as f32 + 2.0 * g, r.h as f32 + 2.0 * g, rad + g, rgba(0, 0, 0, a));
            }
            // the outer hairline: dark ~16% just outside the frame
            s.fill_rrect(r.x as f32 - 1.0, r.y as f32 - 1.0, r.w as f32 + 2.0, r.h as f32 + 2.0, rad + 1.0, rgba(0, 0, 0, 40));
        }
        let bar = if focused { super::ui::WINDOW_FRAME_FOCUSED.now() } else { super::ui::WINDOW_FRAME_BLURRED.now() };
        s.fill_rrect(r.x as f32, r.y as f32, r.w as f32, r.h as f32, rad, bar);
        // the body: the app, drawn at the body size
        let b = w.body(m);
        let (bw, bh) = (b.w.max(1) as usize, b.h.max(1) as usize);
        #[cfg(feature = "desktop-wayland")]
        let persistent = std::env::var_os("SPRINGBOARD_WAYLAND").is_some() && matches!(w.app, AppId::External(_)) && !self.keyboard.shown() && !self.keyboard.animating();
        #[cfg(not(feature = "desktop-wayland"))]
        let persistent = false;
        #[cfg(feature = "desktop-wayland")]
        let mut buf = if persistent { self.desk.as_mut().unwrap().body_cache.remove(&w.app).unwrap_or_default() } else { std::mem::take(&mut self.desk.as_mut().unwrap().buf) };
        #[cfg(not(feature = "desktop-wayland"))]
        let mut buf = std::mem::take(&mut self.desk.as_mut().unwrap().buf);
        let fresh = buf.len() != bw * bh;
        if !persistent {buf.clear();}
        buf.resize(bw * bh, BLACK);
        {
            let mut sub = Surface { w: bw, h: bh, stride: bw, px: &mut buf, y0: 0 };
            let app = w.app;
            if persistent && !fresh && !self.keyboard.shown() && !self.keyboard.animating() {
                if let AppId::External(slot) = app { self.ext_blit_damage(&mut sub,slot as usize,&mut crate::gfx::Damage::new(bw,bh)); }
            } else {
                self.with_size(b.w, b.h, |sh| sh.draw_app_body(&mut sub, app, t));
            }
        }
        if full_size(w.app) && rad > 0.0 { copy_rect_rounded(s, &buf, bw, bh, b.x, b.y, rad); } else { copy_rect(s, &buf, bw, bh, b.x, b.y); }
        if !focused { dim_rect(s.px, s.stride, b.x.max(0) as usize, (b.y.max(0) as usize).saturating_sub(s.y0), ((b.x + b.w).max(0) as usize).min(s.w), ((b.y + b.h).max(0) as usize).saturating_sub(s.y0).min(s.h), BACKGROUND_DIM); }
        #[cfg(feature = "desktop-wayland")]
        if persistent { self.desk.as_mut().unwrap().body_cache.insert(w.app,buf); }
        else { self.desk.as_mut().unwrap().buf = buf; }
        #[cfg(not(feature = "desktop-wayland"))]
        { self.desk.as_mut().unwrap().buf = buf; }
        let ui = &self.ui;
        // hairline under the title bar and around the window
        if !w.maximized { window_rim(s, r.x as f32, r.y as f32, r.w as f32, r.h as f32, rad); }
        if !full_size(w.app) { s.fill_rect(r.x.max(0) as usize, (r.y + m.title - 1).max(0) as usize, r.w.max(0) as usize, 1, super::ui::SEPARATOR_C.now()); }
        // traffic lights
        let (doc_title, doc_edited): (&str, bool) = match w.app { AppId::External(slot) => self.ext.get(slot as usize).and_then(Option::as_ref).map_or(("", false), |h| (h.doc_title.as_str(), h.doc_edited)), _ => ("", false) };
        for (i, c) in [SYSTEM_RED, SYSTEM_YELLOW, SYSTEM_GREEN].into_iter().enumerate() {
            let lr = w.light_rect_px(m, i);
            let c = if focused { c } else { TRAFFIC_INACTIVE_C.now() };
            draw_light(s, &lr, c, focused, ui.pt(1.));
            if i == 0 && doc_edited {
                // NativeDocument isDocumentEdited: a dark dot in the close light
                let d = lr.w as f32 * 0.34;
                s.fill_rrect(lr.x as f32 + (lr.w as f32 - d) / 2.0, lr.y as f32 + (lr.h as f32 - d) / 2.0, d, d, d / 2.0, TRAFFIC_GLYPH);
            }
            if focused && self.desk.as_ref().unwrap().lights_hover == Some(w.app) {
                draw_light_glyph(s, &lr, i, w.maximized, ui.pt(1.));
            }
        }
        // title
        if full_size(w.app) { return; }
        let base: &str = if doc_title.is_empty() { apps().iter().find(|a| a.id == w.app).map_or("", |a| a.name) } else { doc_title };
        let name: std::borrow::Cow<str> = if doc_edited { format!("{base} \u{2013} Edited").into() } else { base.into() };
        self.label(&name, 13.0, Weight::Semibold, if focused { super::ui::LABEL.now() } else { super::ui::LABEL_2.now() }, |im| s.blit(im, r.x + (r.w - im.w as i32) / 2, r.y + (m.title - im.h as i32) / 2, 255, 255));
    }

    /// The Calendar tile's face. The package icon is a fixed picture, so the Dock repaints its paper and sets today's
    /// weekday (red) over the day number (black), as the Dock shows the date. `(x, y, w, h)` is the tile.
    fn calendar_tile_face(&self, s: &mut Surface, scale: f32, (x, y, w, h): (f32, f32, f32, f32), (wday, mday): (i32, i32)) {
        let (day, num) = calendar_face_text(wday, mday);
        // the paper covers the baked picture between the squircle's rounded sides
        s.fill_rrect(x + w * 0.16, y, w * 0.68, h, 0.0, WHITE);
        let cx = x + w / 2.0;
        self.label(day, 0.2 * h / scale, Weight::Semibold, SYSTEM_RED, |im| {
            s.blit(im, (cx - im.w as f32 / 2.0) as i32, (y + 0.06 * h) as i32, 255, 255);
        });
        self.label(&num, 0.5 * h / scale, Weight::Regular, BLACK, |im| {
            s.blit(im, (cx - im.w as f32 / 2.0) as i32, (y + 0.34 * h) as i32, 255, 255);
        });
    }

    fn draw_desk_dock(&self, s: &mut Surface, m: &Metrics, t: f64) {
        if !self.dock_is_visible(){return;}
        let (list, bar0, rects0) = self.desk_dock(m);
        // autohide slide, then magnification around the pointer (both pure functions of the clock)
        let (bar, slid) = self.dock_slid(bar0, &rects0, t / 1000.0);
        let rects = self.dock_magnified(&slid, t / 1000.0);
        let ui = &self.ui;
        // frosted material: blur what is behind the dock, tint it, and edge it with a hairline
        let (bx, by) = (bar.x.max(0) as usize, bar.y.max(0) as usize);
        let (bw, bh) = ((bar.w as usize).min(s.w.saturating_sub(bx)), (bar.h as usize).min(s.h.saturating_sub(by)));
        if bw > 0 && bh > 0 && bifrost_render::glass::gpu() {
            let rad = bar.w.min(bar.h) as f32 * DOCK_RADIUS_K;
            bifrost_render::glass::push(super::control_center::shell_glass_rect(&bifrost_ui::material_request::ShellSurface::Dock.request((bar.x as f32, bar.y as f32, bar.w as f32, bar.h as f32), rad)));
            soft_shadow(s, ui, bar.x as f32, bar.y as f32, bar.w as f32, bar.h as f32, rad);
            s.fill_rrect(bar.x as f32 - 1.0, bar.y as f32 - 1.0, bar.w as f32 + 2.0, bar.h as f32 + 2.0, rad + 1.0, rgba(255, 255, 255, 36));
            s.fill_rrect(bar.x as f32, bar.y as f32, bar.w as f32, bar.h as f32, rad, bifrost_render::glass::tint(super::ui::DOCK_TINT.now()));
            dock_rim(s, bar, rad);
        } else if bw > 0 && bh > 0 {
            let mut img = Image::new(bw, bh);
            for y in 0..bh {
                let from = (by + y) * s.stride + bx;
                img.px[y * bw..(y + 1) * bw].copy_from_slice(&s.px[from..from + bw]);
            }
            img.blur_material(ui.pt(10.0) as usize);
            let rad = bar.w.min(bar.h) as f32 * DOCK_RADIUS_K;
            // clip the blurred patch to the capsule by drawing it, then the tint over it
            let mut patch = Surface { w: s.w, h: s.h, stride: s.stride, px: s.px, y0: s.y0 };
            patch.blit(&img, bx as i32, by as i32, 255, 255);
            s.fill_rrect(bar.x as f32 - 1.0, bar.y as f32 - 1.0, bar.w as f32 + 2.0, bar.h as f32 + 2.0, rad + 1.0, rgba(255, 255, 255, 36));
            s.fill_rrect(bar.x as f32, bar.y as f32, bar.w as f32, bar.h as f32, rad, super::ui::DOCK_TINT.now());
            dock_rim(s, bar, rad);
        }
        // dividers: pinned | recents | minimized windows and the Trash
        for k in self.dock_seps(&list) {
            let (a, b) = (slid[k], slid[k + 1]);
            if a.y == b.y {
                let h = (a.h as f32 * 0.6).round() as usize;
                s.fill_rect(((a.x + a.w + b.x) / 2).max(0) as usize, (a.y + (a.h - h as i32) / 2).max(0) as usize, 1, h, SEPARATOR_C.now());
            } else {
                let w = (a.w as f32 * 0.6).round() as usize;
                s.fill_rect((a.x + (a.w - w as i32) / 2).max(0) as usize, ((a.y + a.h + b.y) / 2).max(0) as usize, w, 1, SEPARATOR_C.now());
            }
        }
        let desk = self.desk.as_ref().unwrap();
        let lifted = match desk.grab { Some(Grab::DockDrag { from, moved: true, .. }) => Some(from), _ => None };
        let slide = desk.dock_slide.borrow();
        for (k, r) in rects.iter().enumerate() {
            let i = list[k];
            if lifted == Some(k) { continue; } // its slot stays empty; the icon follows the pointer
            let (ox, oy) = slide.offset(k, t / 1000.0);
            let (bx, by) = self.dock_bounce_offset(i, t / 1000.0, r.w);
            let r = &Rect { x: r.x + ox.round() as i32 + bx, y: r.y + oy.round() as i32 + by, ..*r };
            if i == DOCK_TRASH { draw_trash(s, r); continue; }
            if i == super::dock_stack::DOCK_STACK { super::dock_stack::draw_stack(s, r, &desk.stack_files); continue; }
            if let Some(wi) = super::desktop_dock::dock_min_index(i) {
                // a minimized window: a plate with its app's icon, to the right of the last divider
                if let Some(ai) = desk.wins.get(wi).and_then(|w| apps().iter().position(|a| a.id == w.app)) {
                    s.fill_rrect(r.x as f32, r.y as f32, r.w as f32, r.h as f32, ui.pt(10.0), super::ui::CELL);
                    let inset = r.w / 6;
                    let side = (r.w - 2 * inset).max(1) as usize;
                    let base = &self.base_icons[ai];
                    let mut cache = desk.icon_cache.borrow_mut(); if cache.len() > 48 { cache.retain(|k, _| k.1 == side); if cache.len() > 48 { cache.clear(); } }
                    let icon = cache.entry((base.px.as_ptr() as usize, side)).or_insert_with(|| base.resized(side, side));
                    s.blit(icon, r.x + inset, r.y + inset, 255, 255);
                }
                continue;
            }
            if i == apps().len() {
                s.fill_rrect(r.x as f32,r.y as f32,r.w as f32,r.h as f32,ui.pt(10.0),super::ui::CELL);
                let dot = ui.pt(8.0); let gap = ui.pt(6.0);
                for y in 0..3 { for x in 0..3 {
                    s.fill_rrect(r.x as f32+ui.pt(6.0)+(dot+gap)*x as f32,r.y as f32+ui.pt(6.0)+(dot+gap)*y as f32,dot,dot,ui.pt(2.0),WHITE);
                }}
                continue;
            }
            #[cfg(feature = "desktop-wayland")]
            if i > apps().len() {
                let app=&desk.linux_apps[i-apps().len()-1];
                let pressed=matches!(desk.grab,Some(Grab::DockDrag{from:g,..}) if g==k);
                let mut cache=desk.icon_cache.borrow_mut();if cache.len()>48{cache.retain(|k,_|k.1==r.w as usize);if cache.len()>48{cache.clear();}}
                let icon=cache.entry((app.icon.px.as_ptr() as usize,r.w as usize)).or_insert_with(||app.icon.resized(r.w as usize,r.h as usize));
                s.blit(icon,r.x,r.y,255,if pressed {150} else {255});
                drop(cache);
                if !app.windows.is_empty() {
                    let d=ui.pt(4.0);
                    s.fill_rrect((r.x+r.w/2) as f32-d/2.0,(r.y+r.h) as f32+ui.pt(3.0),d,d,d/2.0,WHITE);
                }
                continue;
            }
            let base = &self.base_icons[i];
            let pressed = matches!(desk.grab, Some(Grab::DockDrag{from:g,..}) if g == k);
            if base.w as i32 == r.w {
                s.blit(base, r.x, r.y, 255, if pressed { 150 } else { 255 });
            } else {
                let mut cache = desk.icon_cache.borrow_mut(); if cache.len() > 48 { cache.retain(|k, _| k.1 == r.w as usize); if cache.len() > 48 { cache.clear(); } }
                let icon = cache.entry((base.px.as_ptr() as usize, r.w as usize)).or_insert_with(|| base.resized(r.w as usize, r.h as usize));
                s.blit(icon, r.x, r.y, 255, if pressed { 150 } else { 255 });
            }
            if apps()[i].icon == CALENDAR_TILE {
                self.calendar_tile_face(s, ui.s, (r.x as f32, r.y as f32, r.w as f32, r.h as f32), (self.info.wday, self.info.mday));
            }
            let id = apps()[i].id;
            #[cfg(feature = "desktop-wayland")]
            let native_running=if let AppId::External(slot)=id {self.foreign_running(slot as usize)} else {false};
            #[cfg(not(feature = "desktop-wayland"))]
            let native_running=false;
            let background = matches!(id, AppId::External(slot) if self.ext.get(slot as usize).and_then(Option::as_ref).is_some_and(|h| h.running()));
            if let Some(c) = dock_dot(desk.index_of(id).is_some() || native_running, background) {
                let d = ui.pt(4.0);
                s.fill_rrect((r.x + r.w / 2) as f32 - d / 2.0, (r.y + r.h) as f32 + ui.pt(3.0), d, d, d / 2.0, c);
            }
            if let AppId::External(slot) = id {
                if let Some(h) = self.ext.get(slot as usize).and_then(Option::as_ref).filter(|h| !h.badge.is_empty()) {
                    self.label(&h.badge, 11.0, Weight::Semibold, WHITE, |im| {
                        let (bh, pad) = (ui.pt(16.0), ui.pt(5.0));
                        let bw = (im.w as f32 + 2.0 * pad).max(bh);
                        let (bx, by) = ((r.x + r.w) as f32 + ui.pt(4.0) - bw, r.y as f32 - ui.pt(4.0));
                        s.fill_rrect(bx, by, bw, bh, bh / 2.0, SYSTEM_RED);
                        s.blit(im, (bx + (bw - im.w as f32) / 2.0) as i32, (by + (bh - im.h as f32) / 2.0) as i32, 255, 255);
                    });
                }
            }
            self.dock_tile_progress(s, i, r, t);
        }
        // the hover label fades in and out (Reduce Motion: it just appears)
        let (label_slot, label_alpha) = self.dock_label_state(t);
        if let Some(k) = label_slot.filter(|_| desk.grab.is_none()) {
            if let (Some(r), Some(&i)) = (rects.get(k), list.get(k)) {
                let name: String = if i < apps().len() { apps()[i].name.to_string() }
                    else if i == DOCK_TRASH { "Trash".into() }
                    else if i == super::dock_stack::DOCK_STACK { "Downloads".into() }
                    else if let Some(wi) = super::desktop_dock::dock_min_index(i) { desk.wins.get(wi).and_then(|w| apps().iter().find(|a| a.id == w.app)).map_or(String::new(), |a| a.name.to_string()) }
                    else { self.dock_linux_name(i) };
                if !name.is_empty() && label_alpha > 0.0 {
                    let a = (label_alpha * 255.0).round() as u32;
                    let pos = self.dock_preferences().position;
                    self.label(&name, 12.0, Weight::Regular, WHITE, |im| {
                        let (px, py) = (ui.pt(10.0), ui.pt(5.0));
                        let (w, h) = (im.w as f32 + 2.0 * px, im.h as f32 + 2.0 * py);
                        let (x, y) = match pos {
                            super::desktop_dock::Position::Bottom => (((r.x + r.w / 2) as f32 - w / 2.0).clamp(0.0, (s.w as f32 - w).max(0.0)), r.y as f32 - h - ui.pt(8.0)),
                            super::desktop_dock::Position::Left => ((r.x + r.w) as f32 + ui.pt(8.0), (r.y + r.h / 2) as f32 - h / 2.0),
                            super::desktop_dock::Position::Right => (r.x as f32 - w - ui.pt(8.0), (r.y + r.h / 2) as f32 - h / 2.0),
                        };
                        s.fill_rrect(x, y, w, h, ui.pt(MENU_RADIUS_PT), super::desktop_dock::fade(MENU_TINT.now(), label_alpha));
                        s.blit(im, (x + px) as i32, (y + py) as i32, 255, a);
                    });
                }
            }
        }
        if let Some(Grab::DockDrag{from,at_x,at_y,moved:true,..})=desk.grab {if let Some(&index)=list.get(from){if let Some(base)=self.base_icons.get(index){let size=rects[from].w as usize;let icon=base.resized(size,size);s.blit(&icon,at_x as i32-size as i32/2,at_y as i32-size as i32/2,255,210);}}}
    }
}

impl Win {
    fn light_rect_px(&self, m: &Metrics, i: usize) -> Rect {
        self.light_rect(m, i)
    }
}

/// A traffic light as a glossy sphere: a rim in a darker shade of its colour and a body that is lighter towards the bottom.
fn draw_light(s: &mut Surface, lr: &Rect, c: u32, focused: bool, px: f32) {
    let (x, y, d) = (lr.x as f32, lr.y as f32, lr.w as f32);
    let shade = |c: u32, to: u32, k: f32| bifrost_render::appearance::mix(c, to, k);
    let rim = shade(c, crate::gfx::pack(0, 0, 0, 255), if focused { 0.22 } else { 0.18 });
    s.fill_rrect(x, y, d, d, d / 2.0, rim);
    let r = d / 2.0 - px;
    let rows = (r * 2.0).ceil() as i32;
    let (top, bottom) = (shade(c, crate::gfx::pack(255, 255, 255, 255), 0.10), shade(c, crate::gfx::pack(255, 255, 255, 255), 0.38));
    for row in 0..rows {
        let dy = row as f32 + 0.5 - r;
        let half = (r * r - dy * dy).max(0.0).sqrt();
        if half <= 0.0 {
            continue;
        }
        let k = row as f32 / rows.max(1) as f32;
        s.fill_rrect(x + d / 2.0 - half, y + px + row as f32, half * 2.0, 1.0, 0.0, shade(top, bottom, k));
    }
}

/// The glyph a traffic light shows while the pointer is over the three: close x, minimise -, zoom +
/// (an inset square, hollow, when the window is already maximised).
fn draw_light_glyph(s: &mut Surface, lr: &Rect, i: usize, maximized: bool, p: f32) {
    let (cx, cy) = (lr.x as f32 + lr.w as f32 / 2.0, lr.y as f32 + lr.h as f32 / 2.0);
    let a = lr.w as f32 * 0.22;
    let th = (p * 2.0).max(2.2);
    let dot = |s: &mut Surface, x: f32, y: f32| s.fill_rrect(x - th / 2.0, y - th / 2.0, th, th, th / 2.0, TRAFFIC_GLYPH);
    let steps = (a * 2.0 / th * 2.0).ceil().max(2.0) as i32;
    for k in 0..=steps {
        let u = -a + 2.0 * a * k as f32 / steps as f32;
        match i {
            0 => { dot(s, cx + u, cy + u); dot(s, cx + u, cy - u); }
            1 => dot(s, cx + u, cy),
            _ if maximized => { dot(s, cx + u, cy - a); dot(s, cx + u, cy + a); dot(s, cx - a, cy + u); dot(s, cx + a, cy + u); }
            _ => { dot(s, cx + u, cy); dot(s, cx, cy + u); }
        }
    }
}

/// Copies a `w` x `h` image into `s` with its top-left at (x, y), clipped to the surface.
/// The dot under a dock icon: white for an app with a window, systemGray for an app whose process still runs
/// with no window (as Golden Gate shows a quit app that is still running), none for an app that is not running.
pub(super) fn dock_dot(has_window: bool, background: bool) -> Option<u32> {
    if has_window {
        Some(WHITE)
    } else if background {
        Some(SECONDARY)
    } else {
        None
    }
}

fn copy_rect(s: &mut Surface, src: &[u32], w: usize, h: usize, x: i32, y: i32) {
    let (x0, x1) = (x.max(0), (x + w as i32).min(s.w as i32));
    if x0 >= x1 {
        return;
    }
    let n = (x1 - x0) as usize;
    for row in 0..h as i32 {
        let sy = y + row;
        if sy < s.y0 as i32 || sy >= s.h as i32 {
            continue;
        }
        let from = row as usize * w + (x0 - x) as usize;
        let to = (sy as usize - s.y0) * s.stride + x0 as usize;
        s.px[to..to + n].copy_from_slice(&src[from..from + n]);
    }
}

/// `copy_rect` with the four corners of the rectangle blended over what is already there through a rounded-rect mask.
fn copy_rect_rounded(s: &mut Surface, src: &[u32], w: usize, h: usize, x: i32, y: i32, radius: f32) {
    let n = (radius.ceil() as usize).min(w / 2).min(h / 2);
    let mask = if n > 0 { crate::gfx::rrect_mask(n * 2, n * 2, radius) } else { Vec::new() };
    // the destination under each corner pixel, read before the body overwrites it
    let mut under: Vec<(usize, u32, u32)> = Vec::new();
    for cy in 0..n {
        for cx in 0..n {
            let cov = mask[cy * 2 * n + cx] as u32;
            if cov >= 255 {
                continue;
            }
            for (px, py) in [(cx, cy), (w - 1 - cx, cy), (cx, h - 1 - cy), (w - 1 - cx, h - 1 - cy)] {
                let (dx, dy) = (x + px as i32, y + py as i32);
                if dx < 0 || dx >= s.w as i32 || dy < s.y0 as i32 || dy >= s.h as i32 {
                    continue;
                }
                let at = (dy as usize - s.y0) * s.stride + dx as usize;
                under.push((at, s.px[at], cov));
                let _ = (px, py);
            }
        }
    }
    copy_rect(s, src, w, h, x, y);
    for (at, was, cov) in under {
        let now = s.px[at];
        let ch = |i: u32| ((((now >> i) & 255) * cov + ((was >> i) & 255) * (255 - cov)) / 255) << i;
        s.px[at] = ch(0) | ch(8) | ch(16) | ch(24);
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn dock_dot_states() {
        use super::{dock_dot, SECONDARY, WHITE};
        assert_eq!(dock_dot(true, false), Some(WHITE));
        assert_eq!(dock_dot(true, true), Some(WHITE));
        assert_eq!(dock_dot(false, true), Some(SECONDARY));
        assert_eq!(dock_dot(false, false), None);
    }

    use super::*;

    #[test]
    fn a_full_size_app_drags_from_its_empty_toolbar_band_but_not_its_controls_or_lights() {
        let m = Metrics { pt: 1.5, ..metrics() };
        let r = Rect { x: 100, y: 50, w: 900, h: 600 };
        let w = Win { app: AppId::External(3), r, maximized: false, minimized: false, restore: r };
        let node = |x: f32, y: f32| bifrost_proto::Node { id: 1, role: bifrost_proto::Role::Button, label: "Back".into(), value: String::new(), bounds: bifrost_ui::Rect { x, y, w: 64.0, h: 32.0 }, enabled: true };
        let (band, holes) = zone_of(&w, &m, &[node(220.0, 10.0), node(220.0, 300.0)], true);
        assert_eq!(band, (UNIFIED_BAR_PT * 1.5).round() as i32, "the unified bar is the drag band");
        assert_eq!(holes.len(), 2, "the lights and the one control inside the band");
        assert_eq!(holes[1], [330, 15, 96, 48], "control bounds scale from points to window px");
        assert!(holes[0][0] == 0 && holes[0][2] > 0, "the lights are not a drag area");
        let (title, none) = zone_of(&w, &m, &[node(220.0, 10.0)], false);
        assert_eq!((title, none.len()), (m.title, 0), "a normal window drags from its title bar only");
    }

    #[test]
    fn light_glyphs_draw_dark_marks_inside_the_light() {
        for i in 0..3 {
            let mut px = vec![WHITE; 20 * 20];
            let mut s = Surface { w: 20, h: 20, stride: 20, px: &mut px, y0: 0 };
            draw_light_glyph(&mut s, &Rect { x: 4, y: 4, w: 12, h: 12 }, i, false, 1.0);
            let dark = px.iter().filter(|p| **p != WHITE).count();
            assert!(dark > 4, "glyph {i} drew {dark} px");
            assert!(px.iter().any(|p| (p & 0xff) < 110), "glyph {i} has a clearly dark core");
            assert_eq!(px[0], WHITE, "nothing outside the light");
        }
    }

    fn metrics() -> Metrics {
        Metrics { w: 1920, h: 1080, title: 30, menubar: 28, min_w: 320, min_h: 220, edge: 6, light: 12, light_gap: 8, light_x: 12, dock_bottom: 100, pt: 1.0 }
    }

    #[test]
    fn closing_a_window_releases_its_buffers() {
        let m = metrics();
        let mut d = Desk::new();
        let i = d.open(&m, AppId::Settings, (900, 600));
        #[cfg(feature = "desktop-wayland")]
        d.body_cache.insert(AppId::Settings, vec![0u32; 1024]);
        d.icon_cache.borrow_mut().insert((1, 1), Image { w: 1, h: 1, px: vec![0; 1] });
        d.close(i);
        #[cfg(feature = "desktop-wayland")]
        assert!(d.body_cache.is_empty());
        assert!(d.icon_cache.borrow().is_empty());
    }

    #[test]
    fn opening_the_same_app_raises_instead_of_duplicating() {
        let m = metrics();
        let mut d = Desk::new();
        d.open(&m, AppId::Settings, (900, 600));
        d.open(&m, AppId::Photos, (900, 600));
        let i = d.open(&m, AppId::Settings, (900, 600));
        assert_eq!(d.wins.len(), 2);
        assert_eq!(i, 1);
        assert_eq!(d.wins[1].app, AppId::Settings);
    }

    #[test]
    fn windows_open_inside_the_work_area_and_cascade() {
        let m = metrics();
        let mut d = Desk::new();
        d.open(&m, AppId::Settings, (900, 600));
        d.open(&m, AppId::Photos, (900, 600));
        let a = Desk::area(&m);
        for w in &d.wins {
            assert!(w.r.x >= a.x && w.r.y >= a.y && w.r.x + w.r.w <= a.x + a.w && w.r.y + w.r.h <= a.y + a.h, "{:?}", w.r);
        }
        assert_ne!(d.wins[0].r, d.wins[1].r);
    }

    #[test]
    fn a_restored_frame_is_kept_inside_the_work_area() {
        let m = metrics();
        let mut d = Desk::new();
        let i = d.open(&m, AppId::Photos, (900, 600));
        let a = Desk::area(&m);
        d.place(&m, i, Rect { x: 200, y: 150, w: 800, h: 500 });
        assert_eq!(d.wins[i].r, Rect { x: 200, y: 150, w: 800, h: 500 }, "a sane frame comes back as saved");
        d.place(&m, i, Rect { x: 99_999, y: -500, w: 99_999, h: 99_999 });
        let r = d.wins[i].r;
        assert!(r.x >= a.x && r.y >= a.y && r.x + r.w <= a.x + a.w && r.y + r.h <= a.y + a.h, "{r:?}");
        assert!(!d.wins[i].maximized);
    }

    #[test]
    fn a_closed_window_remembers_its_frame_for_the_next_open() {
        let m = metrics();
        let mut d = Desk::new();
        let i = d.open(&m, AppId::Photos, (900, 600));
        d.place(&m, i, Rect { x: 200, y: 150, w: 800, h: 500 });
        d.close(0);
        assert_eq!(d.closed_sizes.get(&AppId::Photos), Some(&(800, 500)), "the size it was closed with");
        // a zoomed window is remembered at the frame it goes back to, not at the screen size
        let i = d.open(&m, AppId::Photos, (900, 600));
        d.place(&m, i, Rect { x: 200, y: 150, w: 800, h: 500 });
        d.toggle_zoom(&m, i);
        d.close(0);
        assert_eq!(d.closed_sizes.get(&AppId::Photos), Some(&(800, 500)));
    }

    #[test]
    fn oversized_windows_are_clamped_to_the_work_area() {
        let m = metrics();
        let mut d = Desk::new();
        d.open(&m, AppId::Settings, (5000, 5000));
        let a = Desk::area(&m);
        assert_eq!((d.wins[0].r.w, d.wins[0].r.h), (a.w, a.h));
    }

    #[test]
    fn hits_find_lights_title_content_and_edges() {
        let m = metrics();
        let w = Win { app: AppId::Files, r: Rect { x: 100, y: 100, w: 600, h: 400 }, maximized: false, minimized: false, restore: Rect { x: 0, y: 0, w: 1, h: 1 } };
        assert_eq!(w.hit(&m, 118.0, 115.0), Some(Hit::Light(Light::Close)));
        assert_eq!(w.hit(&m, 138.0, 115.0), Some(Hit::Light(Light::Minimize)));
        assert_eq!(w.hit(&m, 158.0, 115.0), Some(Hit::Light(Light::Zoom)));
        assert_eq!(w.hit(&m, 400.0, 115.0), Some(Hit::Title));
        assert_eq!(w.hit(&m, 400.0, 300.0), Some(Hit::Content));
        assert_eq!(w.hit(&m, 699.0, 499.0), Some(Hit::Edge(R | B)));
        assert_eq!(w.hit(&m, 101.0, 300.0), Some(Hit::Edge(L)));
        assert_eq!(w.hit(&m, 50.0, 50.0), None);
    }

    #[test]
    fn a_maximized_window_has_no_resize_edges() {
        let m = metrics();
        let w = Win { app: AppId::Settings, r: Desk::area(&m), maximized: true, minimized: false, restore: Rect { x: 0, y: 0, w: 1, h: 1 } };
        assert_eq!(w.hit(&m, 1.0, 600.0), Some(Hit::Content));
    }

    #[test]
    fn the_front_window_wins_and_minimized_ones_are_skipped() {
        let m = metrics();
        let mut d = Desk::new();
        d.open(&m, AppId::Settings, (900, 600));
        d.open(&m, AppId::Photos, (900, 600));
        let (x, y) = (d.wins[1].r.x as f32 + 300.0, d.wins[1].r.y as f32 + 300.0);
        assert_eq!(d.pick(&m, x, y).map(|p| p.0), Some(1));
        d.wins[1].minimized = true;
        assert!(d.pick(&m, x, y).map_or(true, |p| p.0 == 0));
        assert_eq!(d.front().map(|w| w.app), Some(AppId::Settings));
    }

    #[test]
    fn zoom_toggles_between_the_work_area_and_the_old_rect() {
        let m = metrics();
        let mut d = Desk::new();
        d.open(&m, AppId::Settings, (900, 600));
        let before = d.wins[0].r;
        d.toggle_zoom(&m, 0);
        assert_eq!(d.wins[0].r, Desk::area(&m));
        d.toggle_zoom(&m, 0);
        assert_eq!(d.wins[0].r, before);
    }

    #[test]
    fn tiling_moves_a_window_to_a_half_and_return_restores_its_size() {
        let m = metrics();
        let mut d = Desk::new();
        d.open(&m, AppId::Settings, (900, 600));
        let before = d.wins[0].r;
        d.tile(&m, 0, Tile::Left);
        assert_eq!(d.wins[0].r, tile_rect(Desk::area(&m), Tile::Left));
        assert!(!d.wins[0].maximized);
        d.return_to_previous(0);
        assert_eq!(d.wins[0].r, before);
        d.tile(&m, 0, Tile::Fill);
        assert!(d.wins[0].maximized);
        assert_eq!(d.wins[0].r, Desk::area(&m));
    }

    #[test]
    fn tiles_cover_the_work_area_and_snap_zones_pick_edges() {
        let a = Rect { x: 0, y: 30, w: 1001, h: 601 };
        let (l, r) = (tile_rect(a, Tile::Left), tile_rect(a, Tile::Right));
        assert_eq!(l.w + r.w, a.w);
        assert_eq!(r.x, l.w);
        assert_eq!(tile_rect(a, Tile::Fill), a);
        assert_eq!(snap_zone(a, 2, 300, 8), Some(Tile::Left));
        assert_eq!(snap_zone(a, 500, 31, 8), Some(Tile::Fill));
        assert_eq!(snap_zone(a, 1000, 31, 8), Some(Tile::TopRight));
        assert_eq!(snap_zone(a, 500, 300, 8), None);
        let m = metrics();
        let mut d = Desk::new();
        d.open(&m, AppId::Settings, (900, 600));
        let before = d.wins[0].r;
        d.tile(&m, 0, Tile::Left);
        d.return_to_previous(0);
        assert_eq!(d.wins[0].r, before);
    }

    #[test]
    fn split_view_shares_the_width_around_the_divider_and_keeps_both_sides_usable() {
        let a = Rect { x: 0, y: 30, w: 1001, h: 601 };
        let (l, r) = split_view_rects(a, 500, 8, 320);
        assert_eq!(l.w + 8 + r.w, a.w, "the two sides and the divider cover the area");
        assert_eq!((l.w, r.w), (496, 497), "an even split takes the usable width in halves");
        assert_eq!((l.x, r.x), (0, 504), "the trailing side starts after the divider");
        assert!(l.y == 30 && r.y == 30 && l.h == 601 && r.h == 601);
        // the ratio is clamped so neither side drops below the minimum width
        let (l, r) = split_view_rects(a, 50, 8, 320);
        assert_eq!((l.w, r.w), (320, 1001 - 8 - 320));
        let (l, r) = split_view_rects(a, 1000, 8, 320);
        assert_eq!((l.w, r.w), (1001 - 8 - 320, 320));
        // a narrow area splits in halves instead of honouring the minimum
        let (l, r) = split_view_rects(Rect { x: 0, y: 0, w: 300, h: 200 }, 900, 8, 320);
        assert_eq!((l.w, r.w), (146, 146));
        // degenerate width never goes negative
        let (l, r) = split_view_rects(Rect { x: 0, y: 0, w: 4, h: 10 }, 500, 8, 320);
        assert!(l.w >= 0 && r.w >= 0);
    }

    #[test]
    fn moving_keeps_the_title_bar_reachable() {
        let m = metrics();
        let mut d = Desk::new();
        d.open(&m, AppId::Settings, (900, 600));
        d.move_to(&m, 0, -5000, -5000);
        assert!(d.wins[0].r.y >= m.menubar);
        assert!(d.wins[0].r.x + d.wins[0].r.w >= m.title * 2);
        d.move_to(&m, 0, 9000, 9000);
        assert!(d.wins[0].r.x <= m.w - m.title * 2 && d.wins[0].r.y < m.h);
    }

    #[test]
    fn resizing_respects_the_minimum_and_the_menu_bar() {
        let m = metrics();
        let mut d = Desk::new();
        d.open(&m, AppId::Settings, (900, 600));
        let start = d.wins[0].r;
        d.resize_to(&m, 0, R | B, start, -5000, -5000);
        assert_eq!((d.wins[0].r.w, d.wins[0].r.h), (m.min_w, m.min_h + title_of(AppId::Settings, &m)), "Settings is full-size content: no title bar to add");
        d.resize_to(&m, 0, L | T, start, -50, -5000);
        assert_eq!(d.wins[0].r.x, start.x - 50);
        assert!(d.wins[0].r.y >= m.menubar);
        assert_eq!(d.wins[0].r.y + d.wins[0].r.h, start.y + start.h, "the bottom edge stays put");
    }

    #[test]
    fn the_dock_is_centred_and_its_icons_do_not_overlap() {
        let m = metrics();
        let (bar, icons) = dock_rects(&m, 48, 10, 10, 5, Some(1), 14);
        assert_eq!(bar.x * 2 + bar.w, m.w);
        for p in icons.windows(2) {
            assert!(p[0].x + p[0].w < p[1].x);
        }
        assert!(icons.iter().all(|i| bar.contains(i.x as f32, i.y as f32)));
    }
}

/// Dark appearance: a white ~15% hairline just inside the window edge (Golden Gate's window rim);
/// in Light it fades to nothing and the dark outer hairline (shadow pass) edges the window.
fn window_rim(s: &mut Surface, x: f32, y: f32, w: f32, h: f32, rad: f32) {
    let c = bifrost_render::appearance::Pair::new(rgba(255, 255, 255, 38), rgba(255, 255, 255, 0)).now();
    if c >> 24 != 0 { s.stroke_rrect(x, y, w, h, rad, 1.0, c); }
}

#[cfg(feature = "desktop-wayland")]
fn clip_corners(image:&mut Image,radius:f32) {clip_window_corners(&mut image.px,image.w,image.h,radius,None);}
#[allow(dead_code)]
fn clip_window_corners(pixels:&mut [u32],width:usize,height:usize,radius:f32,damage:Option<&crate::gfx::Damage>) {
    let n=(radius.ceil() as usize).min(width/2).min(height/2);
    let mask=crate::gfx::rrect_mask(n*2,n*2,radius);
    for y in 0..n {for x in 0..n {
        for (dx,dy,mx,my) in [(x,y,x,y),(width-1-x,y,2*n-1-x,y),(x,height-1-y,x,2*n-1-y),(width-1-x,height-1-y,2*n-1-x,2*n-1-y)] {
            if damage.is_some_and(|d|!d.areas().iter().any(|a|dx>=a.x0&&dx<a.x1&&dy>=a.y0&&dy<a.y1)) {continue;}
            let coverage=mask[my*2*n+mx] as u32; let p=&mut pixels[dy*width+dx];
            let channels=std::array::from_fn::<_,4,_>(|i|((*p>>(i*8))&255)*coverage/255);
            *p=channels.iter().enumerate().fold(0,|v,(i,c)|v|(c<<(i*8)));
        }
    }}
}

#[cfg(feature = "desktop-wayland")]
fn linux_icon(app_id:&str) -> Image {
    let mut names=vec![app_id.to_owned(),app_id.to_ascii_lowercase()];
    let dirs=["/usr/share/applications", "/usr/local/share/applications"];
    for dir in dirs {
        if let Ok(entries)=std::fs::read_dir(dir) { for entry in entries.flatten() {
            if entry.path().extension().and_then(|s|s.to_str())!=Some("desktop") {continue;}
            if let Ok(text)=std::fs::read_to_string(entry.path()) {
                let stem=entry.path().file_stem().and_then(|s|s.to_str()).unwrap_or("").to_owned();
                if stem.eq_ignore_ascii_case(app_id) || text.lines().any(|l|l.strip_prefix("StartupWMClass=").is_some_and(|v|v.eq_ignore_ascii_case(app_id))) {
                    if let Some(icon)=text.lines().find_map(|l|l.strip_prefix("Icon=")) {names.insert(0,icon.to_owned());}
                }
            }
        }}
    }
    let mut paths=Vec::new();
    if app_id.to_ascii_lowercase().contains("firefox") {paths.push("/usr/lib/firefox/browser/chrome/icons/default/default128.png".to_owned());}
    for name in names {
        if name.starts_with('/') {paths.push(name.clone());}
        // Only desktop-entry basenames are accepted for theme lookup.
        if name.contains('/') {continue;}
        for size in [128,256,64,48,32] {paths.push(format!("/usr/share/icons/hicolor/{size}x{size}/apps/{name}.png"));}
        paths.push(format!("/usr/share/pixmaps/{name}.png"));
    }
    for path in paths {if let Ok(bytes)=std::fs::read(path) {if let Some(img)=crate::cg::imageio::decode(&bytes) {return img;}}}
    let mut img=Image::new(64,64);
    let mut surface=Surface {w:64,h:64,stride:64,px:&mut img.px,y0:0};
    surface.fill_rrect(0.0,0.0,64.0,64.0,12.0,super::ui::CELL);
    surface.fill_rrect(12.0,16.0,40.0,32.0,4.0,super::ui::SYSTEM_BLUE);
    img
}

#[cfg(test)]
mod retained_damage_tests {
    use super::*;
    #[test]
    fn review_retained_corner_damage_matches_full_repaint() {
        let (w,h)=(40,30); let color=pack(80,120,170,255);
        let mut retained=vec![color;w*h]; clip_window_corners(&mut retained,w,h,10.,None);
        let mut raw=vec![color;w*h]; let mut damage=crate::gfx::Damage::new(w,h);damage.add(30,20,10,10);
        for y in 20..30 {for x in 30..40 {raw[y*w+x]=pack(200,40,80,255);retained[y*w+x]=raw[y*w+x];}}
        clip_window_corners(&mut retained,w,h,10.,Some(&damage));
        clip_window_corners(&mut raw,w,h,10.,None);
        assert_eq!(retained,raw);
    }
}

/// Dock corner radius as a share of the dock's thickness (Golden Gate: ~24 pt on a 72 pt dock).
const DOCK_RADIUS_K: f32 = 0.33;

/// A lighter glass edge: a bright 1 px inner rim over the tint (white ~35% in Light, ~18% in Dark).
/// Straight edges only; the rounded corners keep the outer hairline.
fn dock_rim(s: &mut Surface, bar: Rect, rad: f32) {
    let c = bifrost_render::appearance::Pair::new(rgba(255, 255, 255, 46), rgba(255, 255, 255, 90)).now();
    let (x, y, w, h) = (bar.x as f32, bar.y as f32, bar.w as f32, bar.h as f32);
    let r = rad.ceil();
    s.fill_rrect(x + r, y + 1.0, (w - 2.0 * r).max(0.0), 1.0, 0.0, c);
    s.fill_rrect(x + r, y + h - 2.0, (w - 2.0 * r).max(0.0), 1.0, 0.0, c);
    s.fill_rrect(x + 1.0, y + r, 1.0, (h - 2.0 * r).max(0.0), 0.0, c);
    s.fill_rrect(x + w - 2.0, y + r, 1.0, (h - 2.0 * r).max(0.0), 0.0, c);
}

/// The dock's Trash, drawn procedurally: a light grey translucent bin, slightly tapered, with vertical ribs and a darker rim.
fn draw_trash(s: &mut Surface, r: &Rect) {
    let (x, y, w, h) = (r.x as f32, r.y as f32, r.w as f32, r.h as f32);
    let (body, rib, rim) = (rgba(222, 226, 232, 200), rgba(160, 166, 176, 200), rgba(130, 136, 146, 230));
    let (top, bottom) = (y + h * 0.08, y + h * 0.98);
    let (tw, bw) = (w * 0.80, w * 0.66);
    let rows = (bottom - top).max(1.0) as i32;
    for k in 0..rows {
        let f = k as f32 / rows as f32;
        let row = tw + (bw - tw) * f;
        let (x0, yk) = (x + (w - row) / 2.0, top + k as f32);
        s.fill_rrect(x0, yk, row, 1.0, 0.0, body);
        for j in 1..5 { s.fill_rrect(x0 + row * j as f32 / 5.0 - 0.5, yk, 1.0, 1.0, 0.0, rib); }
        s.fill_rrect(x0, yk, 1.0, 1.0, 0.0, rim);
        s.fill_rrect(x0 + row - 1.0, yk, 1.0, 1.0, 0.0, rim);
    }
    s.fill_rrect(x + (w - bw) / 2.0, bottom - 1.0, bw, 1.5, 0.75, rim);
    let lip = (h * 0.06).max(2.0);
    s.fill_rrect(x + (w - tw * 1.06) / 2.0, top - lip / 2.0, tw * 1.06, lip, lip / 2.0, rim);
}

/// A soft drop shadow made of concentric rings just outside a rounded rectangle, so the glass tint
/// inside it is not darkened. It reaches `SHADOW_REACH_PT` points beyond the shape. The rings are
/// drawn once per size into a cached image (stroking them every frame cost 8 ms for the dock).
pub(super) fn soft_shadow(s: &mut Surface, ui: &super::ui::Ui, x: f32, y: f32, w: f32, h: f32, rad: f32) {
    use std::cell::RefCell;
    type Key = (i32, i32, u32, u32);
    thread_local! { static CACHE: RefCell<Vec<(Key, Image)>> = const { RefCell::new(Vec::new()) }; }
    let step = ui.pt(SHADOW_REACH_PT / 4.0);
    let margin = (step * 4.0).ceil() as usize + ui.pt(2.0).ceil() as usize;
    let key: Key = (w.round() as i32, h.round() as i32, rad.to_bits(), step.to_bits());
    CACHE.with(|c| {
        let mut c = c.borrow_mut();
        let i = match c.iter().position(|(k, _)| *k == key) {
            Some(i) => i,
            None => {
                let mut img = Image::new(w.round() as usize + 2 * margin, h.round() as usize + 2 * margin);
                let mut t = Surface { w: img.w, h: img.h, stride: img.w, px: &mut img.px, y0: 0 };
                for (k, a) in [30u8, 21, 13, 7].into_iter().enumerate() {
                    let g = step * (k + 1) as f32;
                    t.stroke_rrect(margin as f32 - g, margin as f32 - g + ui.pt(2.0), w.round() + 2.0 * g, h.round() + 2.0 * g, rad + g, step, rgba(0, 0, 0, a));
                }
                if c.len() >= 8 { c.remove(0); }
                c.push((key, img));
                c.len() - 1
            }
        };
        s.blit(&c[i].1, x.round() as i32 - margin as i32, y.round() as i32 - margin as i32, 255, 255);
    });
}

/// How far the dock and menu shadows reach beyond their shape, in points.
pub(super) const SHADOW_REACH_PT: f32 = 10.0;

/// How much the body of a window that is not in front is darkened (of 255): inactive windows recede.
pub(super) const BACKGROUND_DIM: u32 = 36;

/// Darkens the opaque pixels in [x0, x1) x [y0, y1) of `px` (row length `stride`) by `amount` of 255.
pub(super) fn dim_rect(px: &mut [u32], stride: usize, x0: usize, y0: usize, x1: usize, y1: usize, amount: u32) {
    let keep = 255 - amount.min(255);
    for y in y0..y1 {
        let Some(row) = px.get_mut(y * stride + x0..y * stride + x1.min(stride)) else { continue };
        for p in row {
            let ch = |sh: u32| (((*p >> sh) & 0xff) * keep / 255) << sh;
            *p = (*p & 0xff00_0000) | ch(16) | ch(8) | ch(0);
        }
    }
}

#[cfg(test)]
mod dim_tests {
    use super::dim_rect;

    #[test]
    fn dims_only_the_rectangle_and_keeps_alpha() {
        let mut px = vec![0xff80_80ffu32; 16];
        dim_rect(&mut px, 4, 1, 1, 3, 3, 51); // 20 %
        assert_eq!(px[0], 0xff80_80ff, "outside is untouched");
        assert_eq!(px[1 * 4 + 1], 0xff66_66cc, "inside is 80 % of the channels, alpha stays");
        assert_eq!(px[2 * 4 + 2], 0xff66_66cc);
        assert_eq!(px[3 * 4 + 3], 0xff80_80ff);
    }

    #[test]
    fn a_rectangle_past_the_buffer_is_clipped_not_a_panic() {
        let mut px = vec![0xffff_ffffu32; 8];
        dim_rect(&mut px, 4, 0, 0, 9, 9, 255);
        assert!(px.iter().all(|&p| p == 0xff00_0000));
    }
}

impl Shell {
    /// Runs `f` on the rendered `text`, taken from the chrome text cache (rendered once per string and style).
    pub(super) fn label<R>(&self, text: &str, pt: f32, weight: Weight, color: u32, f: impl FnOnce(&Image) -> R) -> R {
        let mut cache = self.chrome_text.borrow_mut();
        f(cache.get(&self.ui, text, pt, weight, color))
    }
}

/// One material region (window px) as the blur region of its window: unless maximized, each side that lies inside the
/// window grows by the window radius (the extra blur sits under opaque pixels) and the corner takes the window radius.
#[cfg(feature = "desktop-wayland")]
fn expand_material((x, y, rw, rh, r, kind, active): (f32, f32, f32, f32, f32, bifrost_render::glass::Kind, Option<bool>), (ww, wh): (f32, f32), maximized: bool, corner: f32) -> bifrost_render::glass::Rect {
    if maximized {
        return bifrost_render::glass::Rect { x, y, w: rw, h: rh, r, kind, active, detail: None };
    }
    let e = 0.5;
    let x0 = if x <= e { 0.0 } else { (x - corner).max(0.0) };
    let y0 = if y <= e { 0.0 } else { (y - corner).max(0.0) };
    let x1 = if x + rw >= ww - e { ww } else { (x + rw + corner).min(ww) };
    let y1 = if y + rh >= wh - e { wh } else { (y + rh + corner).min(wh) };
    bifrost_render::glass::Rect { x: x0, y: y0, w: x1 - x0, h: y1 - y0, r: corner.max(r), kind, active, detail: None }
}

#[cfg(all(test, feature = "desktop-wayland"))]
mod material_rect_tests {
    use super::*;
    use bifrost_render::glass::Kind;
    #[test]
    fn an_app_sidebar_region_reaches_the_window_edge_and_grows_inside() {
        // sidebar at the window's left edge under a 28 px title bar, window 400x300, radius 10
        let g = expand_material((0.0, 28.0, 120.0, 272.0, 0.0, Kind::Sidebar, None), (400.0, 300.0), false, 10.0);
        assert_eq!((g.x, g.y, g.w, g.h), (0.0, 18.0, 130.0, 282.0));
        assert_eq!(g.r, 10.0);
    }
    #[test]
    fn a_maximized_window_keeps_the_declared_rect() {
        let g = expand_material((0.0, 28.0, 120.0, 272.0, 6.0, Kind::Sidebar, None), (400.0, 300.0), true, 10.0);
        assert_eq!((g.x, g.y, g.w, g.h, g.r), (0.0, 28.0, 120.0, 272.0, 6.0));
    }
}

/// The presenter's glass kind for a window material an app declared (`NativeVisualEffectView.Material`).
#[cfg(feature = "desktop-wayland")]
fn glass_kind(k: bifrost_proto::MaterialKind) -> bifrost_render::glass::Kind {
    use bifrost_proto::MaterialKind as M;
    use bifrost_render::glass::Kind;
    match k {
        M::Sidebar => Kind::Sidebar,
        M::Titlebar | M::HeaderView => Kind::Titlebar,
        M::WindowBackground | M::ContentBackground | M::UnderWindowBackground | M::UnderPageBackground => Kind::Content,
        M::Menu | M::Popover | M::Sheet | M::HudWindow | M::FullScreenUi | M::ToolTip | M::Selection => Kind::Menu,
    }
}

/// `NativeVisualEffectView.State` for the presenter: `None` follows the window's active state.
#[cfg(feature = "desktop-wayland")]
fn glass_active(s: bifrost_proto::MaterialState) -> Option<bool> {
    match s {
        bifrost_proto::MaterialState::FollowsWindowActiveState => None,
        bifrost_proto::MaterialState::Active => Some(true),
        bifrost_proto::MaterialState::Inactive => Some(false),
    }
}

#[cfg(test)]
mod calendar_tile_tests {
    use super::calendar_face_text;

    #[test]
    fn the_calendar_tile_shows_the_day_it_is_not_a_fixed_picture() {
        // Monday 12 October 2026: the tile reads "Mon" over "12", never the baked "Tue 1"
        assert_eq!(calendar_face_text(1, 12), ("Mon", "12".to_string()));
        assert_eq!(calendar_face_text(2, 1), ("Tue", "1".to_string()));
        // weekday 0 is Sunday; a negative index wraps to Saturday rather than panicking
        assert_eq!(calendar_face_text(0, 4), ("Sun", "4".to_string()));
        assert_eq!(calendar_face_text(-1, 31), ("Sat", "31".to_string()));
    }
}
