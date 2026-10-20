//! Desktop mode: apps run in floating windows over the wallpaper, with a dock at the bottom.
//!
//! This file holds the window model and its geometry (pure, unit-tested) and the shell side that
//! draws it and routes touches. Every app is a singleton, so there is one window per app. Each
//! window's app is laid out for the window body (the shell's `w`/`h` are swapped to the body size
//! while it draws or takes a touch), drawn into an offscreen buffer and copied in.

use super::ui::{Weight, BLACK, SECONDARY, SYSTEM_GREEN, SYSTEM_RED, SYSTEM_YELLOW, TRAFFIC_INACTIVE_C, WHITE, TRAFFIC_GLYPH, MENU_TINT, MENU_RADIUS_PT, SEPARATOR_C, FIELD_BG, LABEL, MAC_BODY_PT, MAC_CALLOUT_PT};
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
/// Corner radius of a minimised window's Dock thumbnail, and the width of its app badge as a share of the Dock icon.
const MINIMIZED_THUMB_RADIUS_PT: f32 = 6.0;
const MINIMIZED_BADGE_K: f32 = 0.4;
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

/// A choice of the green-button menu (hover on the green traffic light), for one window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Arrange {
    /// Fill, Centre and the halves and quarters of Move & Resize.
    Tile(Tile),
    /// Return to Previous Size.
    Previous,
    /// Enter (`true`) or Exit Full Screen.
    FullScreen(bool),
    /// This window on the left half, the window behind it on the right half.
    SideBySide,
}

/// The open green-button menu: the window it is for, and where it opens (screen px, under the button).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ArrangeMenu {
    pub app: AppId,
    pub x: f32,
    pub y: f32,
}

/// How long (ms) the pointer rests on a green button before its menu opens.
pub const ARRANGE_DWELL_MS: f64 = 500.0;
/// The gap (pt) between a green button and the menu under it, and how far above the menu's top (pt, measured from the
/// anchor) the pointer may stray on the way in before the menu closes.
pub const ARRANGE_GAP_PT: f32 = 4.0;
pub const ARRANGE_BRIDGE_PT: f32 = 24.0;

/// One line of the green-button menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArrangeLine {
    Item(&'static str, Arrange),
    Header(&'static str),
    Separator,
}

/// The lines of the green-button menu for a window: `full` when it is full screen (the only choice then is to leave
/// it), `others` when another window could share the screen.
pub fn arrange_lines(full: bool, others: bool) -> Vec<ArrangeLine> {
    use ArrangeLine::*;
    if full {
        return vec![Item("Exit Full Screen", Arrange::FullScreen(false))];
    }
    let mut v = vec![
        Item("Fill", Arrange::Tile(Tile::Fill)),
        Item("Center", Arrange::Tile(Tile::Center)),
        Item("Return to Previous Size", Arrange::Previous),
        Separator,
        Header("Move & Resize"),
        Item("Left", Arrange::Tile(Tile::Left)),
        Item("Right", Arrange::Tile(Tile::Right)),
        Item("Top", Arrange::Tile(Tile::Top)),
        Item("Bottom", Arrange::Tile(Tile::Bottom)),
        Item("Top Left", Arrange::Tile(Tile::TopLeft)),
        Item("Top Right", Arrange::Tile(Tile::TopRight)),
        Item("Bottom Left", Arrange::Tile(Tile::BottomLeft)),
        Item("Bottom Right", Arrange::Tile(Tile::BottomRight)),
    ];
    if others {
        v.extend([Separator, Item("Tile with Next Window", Arrange::SideBySide)]);
    }
    v.extend([Separator, Item("Enter Full Screen", Arrange::FullScreen(true))]);
    v
}

/// What the window server is asked for a tile (`bifrost_window_level_v1` `tile`): the same actions the window
/// server's Ctrl+Super arrow keys perform.
#[cfg(feature = "desktop-wayland")]
pub fn placement_of(tile: Tile) -> crate::bare::window_level_client::Placement {
    use crate::bare::window_level_client::Placement as P;
    match tile {
        Tile::Fill => P::Fill,
        Tile::Left => P::Left,
        Tile::Right => P::Right,
        Tile::Top => P::Top,
        Tile::Bottom => P::Bottom,
        Tile::TopLeft => P::TopLeft,
        Tile::TopRight => P::TopRight,
        Tile::BottomLeft => P::BottomLeft,
        Tile::BottomRight => P::BottomRight,
        Tile::Center => P::Centre,
    }
}

/// Whether the Wayland compositor owns window positions (the desktop session): the shell can maximize and restore a
/// window there, but cannot move it to a half or the centre.
#[cfg(feature = "desktop-wayland")]
pub fn compositor_places_windows() -> bool {
    cfg!(feature = "desktop-wayland") && std::env::var_os("SPRINGBOARD_WAYLAND").is_some()
}

/// The rectangle of `tile` inside the work area `a`; Centre keeps two thirds of each side.
/// How far a window moves up to clear the on-screen keyboard whose top edge is `kb_top`: its bottom edge stops
/// at the keyboard, and its top never goes above the menu bar. Zero when it already clears the keyboard.
pub fn kb_lift(r: Rect, kb_top: i32, menubar: i32) -> i32 {
    let over = r.y + r.h - kb_top;
    if over <= 0 {
        return 0;
    }
    over.min((r.y - menubar).max(0))
}

/// Moves `r` to the position (`x`, `y`) the window server reports for its toplevel; the size is left alone (the
/// configure decides it). True when the position changed, so the frame the presenter holds must be sent again.
#[cfg(feature = "desktop-wayland")]
pub fn frame_follows(r: &mut Rect, x: i32, y: i32) -> bool {
    if (r.x, r.y) == (x, y) {
        return false;
    }
    r.x = x;
    r.y = y;
    true
}

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
    /// full screen: the window is in its own space over the whole screen, with no menu bar, Dock or title bar
    pub full: bool,
    /// the rectangle to go back to when un-maximized or left full screen
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
    /// a title bar drag; `app` is the window moved (the stack can put other windows after it)
    Move { dx: f32, dy: f32, app: AppId },
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
impl LinuxDockApp {
    /// The app's icon, as the Dock draws it.
    pub(super) fn icon(&self) -> &Image {
        &self.icon
    }
}
#[cfg(all(test, feature = "desktop-wayland"))]
impl LinuxDockApp {
    /// A Linux app entry with the given windows, for tests that need windows of other clients.
    pub(super) fn for_test(app_id: &str, windows: Vec<crate::bare::wayland::ForeignWindow>) -> Self {
        LinuxDockApp { app_id: app_id.into(), windows, icon: Image::new(1, 1) }
    }
}

#[cfg(feature = "desktop-wayland")]
struct CachedWindow {
    width:usize, height:usize, pixels:std::sync::Arc<[u32]>,
    focused:bool, maximized:bool, hover:bool, drawn:f64,
}

/// Two title-bar presses this close (ms) are a double-click, which zooms the window as Golden Gate does. The gap is
/// the Settings > Mouse > Double-Click Speed choice (`pointer_prefs`).
fn double_click_ms() -> f64 {
    crate::bare::pointer_prefs::get().double_click_ms as f64
}

#[derive(Default)]
pub struct Desk {
    /// back to front; the floating windows (`floating`) are the last ones
    pub wins: Vec<Win>,
    /// the apps whose windows float (their host asked for `ToShell::WindowLevel`), as of the last `sync_levels`
    pub(super) floating: Vec<AppId>,
    /// the extra windows collapsed to their title bars, with the height each had before it collapsed
    pub(super) collapsed: std::collections::HashMap<AppId, i32>,
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
    /// the last title-bar press (window, time in ms): a second press within the Double-Click Speed gap zooms
    last_title: Option<(AppId, f64)>,
    pub(super) dock_hide_at:Option<f64>,
    /// the windows that went full screen while zoomed: they come back zoomed (`set_full_screen`)
    full_from_zoom: Vec<AppId>,
    /// the green-button menu that is open, and the window and time (ms) the pointer started resting on a green button
    pub(super) arrange: Option<ArrangeMenu>,
    pub(super) arrange_at: Option<(AppId, f64)>,
    /// placements asked of the window server and not sent yet: (window id, placement), see `take_wayland_tiles`
    #[cfg(feature = "desktop-wayland")]
    tile_requests: Vec<(u32, crate::bare::window_level_client::Placement)>,
    /// the newest files of Downloads, fanned out by the Dock's stack, and when the folder was last listed (seconds)
    pub(super) stack_files:Vec<String>,
    pub(super) stack_listed:Option<f64>,
    pub(super) dock_menu_target:Option<usize>,
    pub(super) dock_menu_anchor:(f32,f32),
    grab: Option<Grab>,
    /// neighbours sliding aside while a dock icon is lifted, and the slot it hovers
    pub(super) dock_slide: std::cell::RefCell<bifrost_ui::dockdrag::Slide>,
    dock_drop: Option<usize>,
    /// dock icons scaled to the dock size, keyed by the app they belong to, the icon generation and the size (never by
    /// where the source pixels live in memory): resized once, not per frame
    icon_cache: std::cell::RefCell<std::collections::HashMap<DockIconKey, Image>>,
    buf: Vec<u32>,
    /// scratch for drawing one window in the Wayland path
    #[cfg(feature = "desktop-wayland")]
    window_buf: Vec<u32>,
    /// how many windows have been opened, to cascade them
    opened: usize,
    /// windows of apps that have not committed a frame yet: not drawn and not hit until the first frame arrives
    /// (`Shell::desk_refresh_unmapped`), so an app that opened a window it never drew leaves no black shell behind
    pub(super) unmapped: Vec<AppId>,
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
    /// the windows the last desktop frame did not draw because windows in front cover them completely: they keep no cache
    /// entry (they repaint in full once uncovered), and `retain_wayland_windows` must not take that for a missing frame
    #[cfg(feature = "desktop-wayland")]
    pub(super) hidden_ids: Vec<u32>,
    /// a built-in window whose body may be reused this frame (only the key state moved; see `draw_window`)
    reuse_body: Option<AppId>,
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

/// The Cmd+Tab strip in points: the icons, the gap between them, the padding round them, the room for the selected
/// name under the icons and the capsule's corner radius.
const CMDTAB_ICON_PT: f32 = 72.0;
const CMDTAB_GAP_PT: f32 = 14.0;
const CMDTAB_PAD_PT: f32 = 18.0;
const CMDTAB_LABEL_PT: f32 = 26.0;
const CMDTAB_RADIUS_PT: f32 = 34.0;

/// The Cmd+Tab strip placed on a screen, in px (`Shell::cmdtab_layout`).
struct CmdtabLayout {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    icon: usize,
    gap: f32,
    pad: f32,
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
        // an extra window keeps its title bar: the app's full-size content is for its main window
        AppId::ExternalWindow(..) => false,
        _ => false,
    }
}

/// The unified title and toolbar row of a full-size app window, in points (`bifrost_app::layout::UNIFIED_BAR_PT`).
pub const UNIFIED_BAR_PT: f32 = 52.0;

/// Where a press moves window `w` instead of reaching the app: above `drag_height` (window px), except in the
/// rectangles returned (window px). A full-size app's empty toolbar band drags; its traffic lights and the controls
/// it publishes (`App::controls`) do not, as the native toolkit lets the empty part of a unified toolbar move the window.
/// A full-screen window covers the menu bar; the bar comes back only while the pointer is in its band at the top.
pub fn bar_hidden(full: bool, pointer_y: Option<f32>, bar_h: f32) -> bool {
    full && pointer_y.map_or(true, |y| y >= bar_h)
}

pub fn drag_zone(w: &Win, m: &Metrics, controls: &[bifrost_proto::Node]) -> (i32, Vec<[i32; 4]>) {
    zone_of(w, m, controls, full_size(w.app) && w.app != AppId::Settings)
}

/// Whether a published node of this role handles a press itself, so a press on it must reach the app instead of moving
/// the window. The kit also publishes containers (the toolbar and sidebar `List` that span a whole row or column),
/// rows, static text and sheet windows: those are not controls, and the empty toolbar space they cover must drag.
pub fn takes_press(role: bifrost_proto::Role) -> bool {
    use bifrost_proto::Role;
    matches!(role, Role::Button | Role::TextField | Role::Toggle | Role::Checkbox | Role::Slider | Role::SegmentedControl | Role::MenuItem)
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
    for n in controls.iter().filter(|n| n.bounds.y < UNIFIED_BAR_PT && takes_press(n.role)) {
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
        // a full-screen window has no title bar: the app is the whole window
        let t = if self.full { 0 } else { title_of(self.app, m) };
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
        // a full-screen window is the app alone: no resize edges, no title bar, no traffic lights
        if self.full {
            return self.r.contains(x, y).then_some(Hit::Content);
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
    /// The ids of the native apps this desk started (they may still be running). Used by the low-space purge, which
    /// never clears the cache of a running app.
    #[cfg(feature = "desktop-wayland")]
    pub(super) fn native_running_ids(&self) -> Vec<String> {
        self.native_children.keys().cloned().collect()
    }

    #[cfg(not(feature = "desktop-wayland"))]
    pub(super) fn native_running_ids(&self) -> Vec<String> {
        Vec::new()
    }

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
        let at = self.band_top(w.app);
        self.wins.insert(at, w);
        at
    }

    /// Whether the windows of `app` float.
    pub(super) fn is_floating(&self, app: AppId) -> bool {
        // a window floats with its app (the window level is per app, `ToShell::WindowLevel`)
        self.floating.contains(&app.parent())
    }

    /// The index a window of `app` takes on the top of its band: after every window at its level or below, so a normal
    /// window goes in front of the floating ones and a floating one stays above them.
    pub(super) fn band_top(&self, app: AppId) -> usize {
        let floating = self.is_floating(app);
        self.wins.iter().position(|o| self.is_floating(o.app) && !floating).unwrap_or(self.wins.len())
    }

    /// Sets the apps whose windows float and moves their windows after the normal ones; each band keeps its order.
    pub(super) fn sync_levels(&mut self, floating: Vec<AppId>) -> bool {
        self.floating = floating;
        let list = std::mem::take(&mut self.floating);
        // true when the stack moved: a settled desktop asks for no frame
        let moved = self.wins.windows(2).any(|p| list.contains(&p[0].app) && !list.contains(&p[1].app));
        if moved {
            self.wins.sort_by_key(|w| list.contains(&w.app));
        }
        self.floating = list;
        moved
    }

    /// The frontmost visible window hit at (x, y).
    pub fn pick(&self, m: &Metrics, x: f32, y: f32) -> Option<(usize, Hit)> {
        self.wins.iter().enumerate().rev().filter(|(_, w)| !w.minimized && !self.unmapped.contains(&w.app)).find_map(|(i, w)| w.hit(m, x, y).map(|h| (i, h)))
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
        let at = self.band_top(app);
        self.wins.insert(at, Win { app, r, maximized: false, minimized: false, full: false, restore: r });
        at
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

    /// The space a full-screen window takes: the whole screen, under the menu bar and over the Dock.
    pub fn screen_rect(m: &Metrics) -> Rect {
        Rect { x: 0, y: 0, w: m.w, h: m.h }
    }

    /// Enters (`on`) or leaves full screen for window `i`. The window keeps the size it had to go back to. A
    /// fixed-size window does not go full screen; returns whether the window changed state.
    pub fn set_full_screen(&mut self, m: &Metrics, i: usize, on: bool) -> bool {
        let w = &mut self.wins[i];
        if size_limits(w.app).fixed || w.full == on {
            return false;
        }
        if on {
            // a zoomed window keeps the frame from before the zoom as its way back, and returns zoomed
            if w.maximized {
                self.full_from_zoom.push(w.app);
            } else {
                w.restore = w.r;
            }
            w.r = Self::screen_rect(m);
            w.maximized = false;
        } else if let Some(k) = self.full_from_zoom.iter().position(|a| *a == w.app) {
            self.full_from_zoom.remove(k);
            w.r = Self::area(m);
            w.maximized = true;
        } else {
            w.r = w.restore;
        }
        w.full = on;
        true
    }

    /// Whether the front window is full screen (Game Mode reads it).
    pub fn front_full(&self) -> bool {
        self.front().is_some_and(|w| w.full)
    }

    /// Whether the Dock slides away when the pointer leaves it: the Dock's own choice, or a full-screen front window
    /// (which covers the screen; the Dock comes back when the pointer reaches its edge).
    pub(super) fn dock_autohides(&self) -> bool {
        self.dock_preferences.autohide || self.front_full()
    }

    pub fn toggle_zoom(&mut self, m: &Metrics, i: usize) {
        let a = Self::area(m);
        let w = &mut self.wins[i];
        if size_limits(w.app).fixed || w.full {
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
        self.collapsed.remove(&w.app);
        w.app
    }

    /// Whether an extra window is collapsed to its title bar.
    pub(super) fn is_collapsed(&self, app: AppId) -> bool {
        self.collapsed.contains_key(&app)
    }

    /// Collapses window `i` to its title bar, or expands it to the height it had. The app's body is not resized here: the
    /// caller relays the size (`desk_layout`).
    pub(super) fn set_collapsed(&mut self, m: &Metrics, i: usize, on: bool) {
        let app = self.wins[i].app;
        if on {
            if !self.is_collapsed(app) {
                let title = title_of(app, m).max(1);
                self.collapsed.insert(app, self.wins[i].r.h);
                self.wins[i].r.h = title;
            }
        } else if let Some(h) = self.collapsed.remove(&app) {
            self.wins[i].r.h = h;
        }
    }

    /// Runs `f` on `src` scaled to `w` x `h`, resized once per (owner, generation, size). Only the current generation
    /// is kept when the cache grows, so icons drawn before a rebuild never come back.
    pub(super) fn with_scaled_icon<R>(&self, key: DockIconKey, src: &Image, w: usize, h: usize, f: impl FnOnce(&Image) -> R) -> R {
        let mut cache = self.icon_cache.borrow_mut();
        if cache.len() > 48 {
            cache.retain(|k, _| k.generation == key.generation && k.size == key.size);
            if cache.len() > 48 {
                cache.clear();
            }
        }
        let icon = cache.entry(key).or_insert_with(|| src.resized(w, h));
        f(icon)
    }

    /// Forgets every scaled Dock icon (the icons were drawn again).
    pub(super) fn clear_icon_cache(&self) {
        self.icon_cache.borrow_mut().clear();
    }

    /// Drops the per-window pixel caches and stale dock icon resizes; true when something was freed.
    pub fn release_app_buffers(&mut self, app: AppId) -> bool {
        let mut freed = false;
        #[cfg(feature = "desktop-wayland")]
        {
            freed |= self.body_cache.remove(&app).is_some();
            let before = self.window_cache.len();
            self.window_cache.retain(|id, _| frame_app(*id).is_some_and(|a| a != app));
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

/// Whose icon a cached Dock resize is: an entry of `apps()` or a Linux client by Wayland app id.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(super) enum DockIconOwner {
    App(usize),
    #[cfg_attr(not(feature = "desktop-wayland"), allow(dead_code))]
    Linux(String),
}

/// Key of a cached, scaled Dock icon. It names the app (not the address of its pixel buffer: a freed buffer's address
/// is handed to the next icon drawn, so an address key showed one app's picture on another's slot after the icons
/// were rebuilt), the icon generation (`Shell::icon_rebuilds`) and the side in px.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(super) struct DockIconKey {
    pub owner: DockIconOwner,
    pub generation: u32,
    pub size: usize,
}

/// The largest picture asked of the window server for a minimized window's Dock thumbnail: twice the biggest the slot
/// gets when the Dock magnifies, so it stays sharp.
const DOCK_SHOT_BOUND: (u32, u32) = (256, 256);

/// The frame of a minimised window's Dock thumbnail inside its square `slot`: the window's aspect (`w` / `h`, clamped
/// so a very wide or tall window still reads), fitted to the slot and centred on it.
pub(super) fn thumbnail_frame(slot: Rect, window_w: i32, window_h: i32) -> Rect {
    let aspect = if window_w > 0 && window_h > 0 { (window_w as f32 / window_h as f32).clamp(0.5, 2.0) } else { 1.0 };
    let (w, h) = if aspect >= 1.0 { (slot.w as f32, slot.w as f32 / aspect) } else { (slot.h as f32 * aspect, slot.h as f32) };
    let (w, h) = ((w.round() as i32).clamp(1, slot.w.max(1)), (h.round() as i32).clamp(1, slot.h.max(1)));
    Rect { x: slot.x + (slot.w - w) / 2, y: slot.y + (slot.h - h) / 2, w, h }
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
pub(super) fn win_id(app: AppId) -> Option<u32> {
    match app {
        AppId::ExternalWindow(slot, id) => Some(WINDOW_ID_FLAG | ((slot as u32) << 16) | id as u32),
        _ => apps().iter().position(|a| a.id == app).map(|i| i as u32 + 1),
    }
}

/// Set in the window-server id of an extra window: `WINDOW_ID_FLAG | slot << 16 | window id`. The catalogue ids (1 to
/// `apps().len()`) never reach it.
const WINDOW_ID_FLAG: u32 = 0x0100_0000;

/// The desk window a window-server id names (the reverse of `win_id`).
pub(super) fn frame_app(id: u32) -> Option<AppId> {
    if id & WINDOW_ID_FLAG != 0 {
        return Some(AppId::ExternalWindow(((id >> 16) & 0xff) as u8, (id & 0xffff) as u16));
    }
    apps().get((id as usize).checked_sub(1)?).map(|a| a.id)
}

/// The title a frame carries to the window server: the app's name (an extra window shows its parent's name here).
fn frame_title(id: u32) -> &'static str {
    let Some(app) = frame_app(id) else { return "" };
    apps().iter().find(|a| a.id == app.parent()).map_or("", |a| a.name)
}

impl Shell {
    #[cfg(feature = "desktop-wayland")]
    /// Gets the windows' frames ready for a partial desktop frame: drawn afresh when an external app sent pixels (`ext`),
    /// else kept as the last frame left them. False when only a full desktop frame can do it.
    pub(super) fn windows_ready(&mut self, s: &mut Surface, ext: bool, t: f64) -> bool {
        if ext {
            self.draw_desk_windows(s, t);
            true
        } else {
            self.retain_wayland_windows()
        }
    }

    /// Whether a frame for an external app's new pixels may draw the windows alone: the settled desktop on the window
    /// server's path (the conditions of the bar-only frame), nothing animating and nothing floating over the windows.
    #[cfg(feature = "desktop-wayland")]
    pub(super) fn ext_windows_only_ok(&self, t: f64) -> bool {
        std::env::var_os("SPRINGBOARD_WAYLAND").is_some()
            && self.desk_active() && self.desk_drawn && !self.is_locked() && self.anim.is_none()
            && self.desktop_menu.is_none() && !self.cc_active() && !self.center_active() && !self.overlay_visible() && !self.quick_look.is_visible() && !self.utility_up()
            && !self.overview_active() && !self.desk_animating(t)
    }

    #[cfg(not(feature = "desktop-wayland"))]
    pub(super) fn ext_windows_only_ok(&self, _t: f64) -> bool {
        false
    }

    #[cfg(feature = "desktop-wayland")]
    pub(super) fn retain_wayland_windows(&mut self) -> bool {
        let m=self.desk_metrics();
        let title_height=m.title;
        let ext=&self.ext;
        let Some(desk)=self.desk.as_mut() else{return false};
        if !desk.fx.is_empty() {return false;}
        let mut frames=Vec::new();
        for w in desk.wins.iter().filter(|w|!w.minimized && !desk.unmapped.contains(&w.app)) {
            let Some(id)=win_id(w.app) else{return false};
            // a covered window has no cache entry by design (see `hidden_ids`); anything else without one needs a full draw
            if !desk.window_cache.contains_key(&id) && !desk.hidden_ids.contains(&id) {return false;}
            frames.push(crate::bare::wayland::WindowFrame {id,title:frame_title(id),x:w.r.x,y:w.r.y,width:w.r.w as usize,height:w.r.h as usize,pixels:None,damage:crate::gfx::Damage::new(w.r.w as usize,w.r.h as usize),maximized:w.maximized,fullscreen:w.full,title_height,drag_height:0,no_drag:Vec::new(),floating:false,fx:bifrost_render::glass::Fx::IDENTITY,opaque:!matches!(w.app,AppId::External(_))});
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
        let floating = self.floating_apps();
        self.desk.as_mut().map(|d| {
            d.sync_levels(floating);
            let mut frames = std::mem::take(&mut d.frames);
            // a floating window's frame goes last, so the window server stacks it above the normal ones
            for f in frames.iter_mut() {
                f.floating = frame_app(f.id).is_some_and(|a| d.is_floating(a));
            }
            frames.sort_by_key(|f| f.floating);
            // Lock/overlays destroy the backend's windows; rebuilding must send
            // complete pixels rather than a cached unchanged-window sentinel.
            if frames.is_empty() { d.window_cache.clear(); d.body_cache.clear(); }
            frames
        }).unwrap_or_default()
    }
    /// A window is being minimized: asks the window server once for its picture, which its Dock slot shows in the
    /// thumbnail frame. Without the capture global nothing is asked and the slot keeps the app icon in its frame.
    pub(super) fn dock_shot(&mut self, app: AppId, t: f64) {
        let Some(id) = win_id(app) else { return };
        self.shots.request_now(super::window_shots::Source::Surface(id), DOCK_SHOT_BOUND, t);
    }

    /// The picture a minimized window's Dock slot shows, when the window server gave one.
    pub(super) fn dock_picture(&self, app: AppId) -> Option<&Image> {
        let shot = self.shots.shot(super::window_shots::Source::Surface(win_id(app)?))?;
        (shot.image.w > 0 && shot.image.h > 0).then_some(&shot.image)
    }

    /// The windows of other clients the window server lists, once each: the native and direct apps' windows, then the
    /// Linux apps' (Overview gives each a tile).
    #[cfg(feature = "desktop-wayland")]
    pub(super) fn mission_foreign_windows(&self) -> Vec<(u32, String)> {
        let Some(d) = self.desk.as_ref() else { return Vec::new() };
        let mut seen = std::collections::HashSet::new();
        d.native_windows.iter().chain(d.linux_apps.iter().flat_map(|a| a.windows.iter())).filter(|w| seen.insert(w.id)).map(|w| (w.id, w.title.clone())).collect()
    }
    #[cfg(not(feature = "desktop-wayland"))]
    pub(super) fn mission_foreign_windows(&self) -> Vec<(u32, String)> { Vec::new() }

    /// The window of another client the window server reports as active (the one with the focus), if any.
    #[cfg(feature = "desktop-wayland")]
    pub(super) fn foreign_front_window(&self) -> Option<u32> {
        let d = self.desk.as_ref()?;
        d.native_windows.iter().chain(d.linux_apps.iter().flat_map(|a| a.windows.iter())).find(|w| w.active).map(|w| w.id)
    }
    #[cfg(not(feature = "desktop-wayland"))]
    pub(super) fn foreign_front_window(&self) -> Option<u32> { None }

    /// Activates window `id` of another client: the window server raises and focuses it.
    pub(super) fn desk_activate_foreign(&mut self, id: u32) {
        #[cfg(feature = "desktop-wayland")]
        if let Some(d) = self.desk.as_mut() { d.activate.push(id); }
        let _ = id;
        self.dirty = true;
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
        // a window of an installed BifrOSt app is never a Linux app of its own (the app's tile stands for it, also while its process winds down)
        let installed=apphost::installed();
        let windows:Vec<_>=windows.into_iter().filter(|w|!desk.native_windows.iter().any(|n|n.id==w.id) && super::dock_tiles::is_linux_app(&installed,apps(),&w.app_id)).collect();
        // a Linux tile kept in the Dock for an installed app (or a stale key of one) becomes that app's own key
        if super::dock_tiles::normalize_pinned(&installed,apps(),&mut desk.dock_preferences.pinned,true) {desk.dock_preferences.save(&self.data_dir);}
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
        let id = match ev { WindowEvent::Focus(id) | WindowEvent::Close(id) | WindowEvent::Resize(id,_,_) | WindowEvent::Maximized(id,_) | WindowEvent::Frame(id,..) | WindowEvent::Fullscreen(id,_) => id };
        // a frame is reported for every toplevel of the shell, an app's extra windows (flagged ids) included
        let app = if matches!(ev, WindowEvent::Frame(..) | WindowEvent::Fullscreen(..)) { frame_app(id) } else { apps().get(id.saturating_sub(1) as usize).map(|a| a.id) };
        let Some(app) = app else { return };
        let Some(i) = self.desk.as_ref().and_then(|d| d.index_of(app)) else { return };
        match ev {
            // the compositor hands focus back to a window when a menu or sheet closes; on a locked screen that must not
            // lift the window over the lock (Lock Screen from the System menu closes the menu in the same turn)
            WindowEvent::Focus(_) => { self.desk.as_mut().unwrap().raise(i); if !self.is_locked() { self.screen = Screen::App(app); } self.dirty = true; },
            WindowEvent::Close(_) => self.desk_close_app(app),
            WindowEvent::Maximized(_,maximized) => {let win=&mut self.desk.as_mut().unwrap().wins[i]; if win.maximized!=maximized {if maximized {win.restore=win.r;}else{win.r=win.restore;}win.maximized=maximized;self.dirty=true;}},
            WindowEvent::Resize(_,w,h) if w > 0 && h > 0 => {
                let m = self.desk_metrics();
                let win = &mut self.desk.as_mut().unwrap().wins[i];
                win.r.w = w.max(m.min_w); win.r.h = h.max(m.min_h + title_of(win.app, &m)); self.desk_layout(i); self.dirty = true;
            },
            // The window server places, moves and resizes toplevels and xdg-shell never says where, so the shell's own
            // idea of the position drifted (E2E N1: `state` said 852,339 for a window drawn at 800,360). The frame it
            // reports is the one source of truth: everything the shell puts on the screen from this rectangle (app
            // context menus, the share menu, taps) lands on the window. The size stays with the configure (`Resize`).
            WindowEvent::Frame(_,x,y,_,_) => { let win = &mut self.desk.as_mut().unwrap().wins[i]; if frame_follows(&mut win.r, x, y) { self.dirty = true; } },
            // The window server put the toplevel on the whole output or took it back (its own keys, or an answer to a
            // request the shell made): the shell's window follows, so its menu bar, Dock and layout agree.
            WindowEvent::Fullscreen(_,on) => { if self.desk.as_ref().is_some_and(|d| d.wins[i].full != on) { self.desk_set_full_screen(app, on); } },
            _ => {}
        }
    }

    pub(super) fn desk_active(&self) -> bool {
        self.desk.is_some()
    }

    /// The on-screen keyboard is drawn over the desktop: a touch tablet or a keyboard the user opened on purpose, not
    /// locked, and the keyboard is up or sliding. A desktop otherwise never gets one (`set_keyboard`).
    pub(super) fn desk_keyboard_visible(&self) -> bool {
        self.osk_over_desk() && !self.is_locked() && (self.keyboard.shown() || self.keyboard.animating())
    }

    /// Lifts the front window clear of the keyboard while it is up, and puts a lifted window back once it is down.
    /// A window the user moved since the lift is left where it is.
    pub(super) fn desk_keyboard_lift(&mut self) {
        if let Some((app, y, lifted_y)) = self.desk_kb_lift.take() {
            if let Some(w) = self.desk.as_mut().and_then(|d| d.wins.iter_mut().find(|w| w.app == app)) {
                if w.r.y == lifted_y {
                    w.r.y = y;
                    self.dirty = true;
                }
            }
        }
        if !(self.osk_over_desk() && self.keyboard.shown() && !self.is_locked()) {
            return;
        }
        let kb_top = self.kb_top() as i32;
        let menubar = self.desktop_bar_height() as i32;
        let Some(d) = self.desk.as_mut() else { return };
        let Some(i) = d.wins.iter().rposition(|w| !w.minimized) else { return };
        let w = &mut d.wins[i];
        let lift = kb_lift(w.r, kb_top, menubar);
        if lift > 0 {
            let (app, y) = (w.app, w.r.y);
            w.r.y -= lift;
            self.desk_kb_lift = Some((app, y, w.r.y));
            self.dirty = true;
        }
    }

    /// Touches on the keyboard while the desktop is up: the same routing as `ext_touch` for a foreground app.
    /// `None` leaves the touch to the desktop.
    pub(super) fn desk_keyboard_touch(&mut self, ev: Event, t: f64) -> Option<Vec<Effect>> {
        if !self.desk_keyboard_visible() {
            return None;
        }
        let top = self.kb_top();
        match ev {
            Event::Down(x, y) if self.keyboard.shown() && y >= top => {
                self.kb_touch = true;
                self.keyboard.down(x, y - top, t);
            }
            Event::Move(x, y) if self.kb_touch => self.keyboard.moved(x, y - top),
            Event::Up(x, y) if self.kb_touch => {
                self.kb_touch = false;
                if let Some(out) = self.keyboard.up(x, y - top, t) {
                    self.type_out(out, t);
                }
            }
            _ => return None,
        }
        Some(vec![])
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
        if package.manifest.window!="native" {let slot=apphost::slot_of(&package.manifest.id).ok_or_else(||std::io::Error::other("Browser not installed"))?;self.desk_open(AppId::External(slot as u8),t);self.ext_send(slot,&bifrost_proto::ToApp::Open{url:url.into()});self.dirty=true;return Ok(());}crate::bare::account_policy::check_launch(&package.manifest.id)?;let policy=crate::bare::app_policy::Policy::from_permissions(&package.manifest.id,&package.manifest.permissions)?;let mut cmd=crate::bare::app_policy::command(&package.manifest.id,&package.exec_path(),&apphost::root().join("appdata").join(&package.manifest.id),policy,true)?;cmd.arg(url);let child=cmd.spawn()?;if let Some(mut old)=self.desk.as_mut().unwrap().native_children.insert(package.manifest.id.clone(),child){if matches!(old.try_wait(),Ok(None)){crate::bare::sys::spawn_named("reap-child", move||{let _=old.wait();});}}self.dirty=true;return Ok(());}
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
        // an app without a Wayland id has no window to find: an empty id must not match every unnamed window
        !id.is_empty() && desk.native_windows.iter().any(|w| w.app_id == id)
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

    /// Tiles the front desktop window on `tile` (Window menu and Ctrl+Cmd arrow keys).
    #[cfg(feature = "desktop-wayland")]
    pub(super) fn desk_tile_front(&mut self, tile: Tile) {
        let Some(i) = self.desk_front_index() else { return };
        self.desk_tile_window(i, tile);
    }

    /// Tiles window `i` on `tile`. In the Wayland session the window server places windows, so the shell asks it
    /// (`bifrost_window_level_v1` `tile`, the same actions as its Ctrl+Super arrow keys) and the frame comes back
    /// from it; Fill stays the zoom the green button has always done.
    #[cfg(feature = "desktop-wayland")]
    pub(super) fn desk_tile_window(&mut self, i: usize, tile: Tile) {
        let Some(w) = self.desk.as_ref().and_then(|d| d.wins.get(i)).copied() else { return };
        if w.full { return; }
        if compositor_places_windows() {
            if tile == Tile::Fill {
                if !w.maximized { self.desk_zoom_window(i); }
            } else {
                self.desk_request_placement(w.app, placement_of(tile));
            }
            return;
        }
        let m = self.desk_metrics();
        if let Some(d) = self.desk.as_mut() { d.tile(&m, i, tile); }
        self.desk_layout(i);
        self.desk_frame_changed(i);
    }

    /// Queues a placement for the window server (sent with the next frame, `take_wayland_tiles`).
    #[cfg(feature = "desktop-wayland")]
    fn desk_request_placement(&mut self, app: AppId, placement: crate::bare::window_level_client::Placement) {
        let (Some(id), Some(d)) = (win_id(app), self.desk.as_mut()) else { return };
        d.tile_requests.push((id, placement));
        self.drawn = super::Drawn::Nothing;
        self.dirty = true;
    }

    /// The placements to send to the window server, oldest first.
    #[cfg(feature = "desktop-wayland")]
    pub fn take_wayland_tiles(&mut self) -> Vec<(u32, crate::bare::window_level_client::Placement)> {
        self.desk.as_mut().map(|d| std::mem::take(&mut d.tile_requests)).unwrap_or_default()
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
        self.desk_restore_window(i);
    }

    /// Returns window `i` to the size it had before its last tiling or zoom (the window server does it in the Wayland
    /// session, as it does for the zoom's way back).
    #[cfg(feature = "desktop-wayland")]
    pub(super) fn desk_restore_window(&mut self, i: usize) {
        let Some(w) = self.desk.as_ref().and_then(|d| d.wins.get(i)).copied() else { return };
        if w.full { return; }
        if compositor_places_windows() {
            self.desk_request_placement(w.app, crate::bare::window_level_client::Placement::Previous);
            return;
        }
        if let Some(d) = self.desk.as_mut() { d.return_to_previous(i); }
        self.desk_layout(i);
        self.desk_frame_changed(i);
    }

    /// The pointer moved: tracks a rest on a green button (the menu opens after `ARRANGE_DWELL_MS`, in `arrange_tick`)
    /// and closes the open menu once the pointer has left both the button and the menu.
    #[cfg(feature = "desktop-wayland")]
    pub(super) fn arrange_hover(&mut self, x: f32, y: f32, t: f64) {
        let over = self.zoom_button_at(x, y);
        if self.desktop_menu == Some(super::desktop_bar::Menu::Arrange) {
            if !self.arrange_keeps(x, y) {
                self.desktop_menu = None;
                self.dirty = true;
                if let Some(d) = self.desk.as_mut() { d.arrange_at = None; d.arrange = None; }
            }
            return;
        }
        let Some(d) = self.desk.as_mut() else { return };
        match (over, d.arrange_at) {
            (Some(a), Some((b, _))) if a == b => {}
            (Some(a), _) => d.arrange_at = Some((a, t)),
            (None, _) => d.arrange_at = None,
        }
    }

    /// The app whose green button is at (x, y), when the front-most window there has one under the pointer.
    #[cfg(feature = "desktop-wayland")]
    fn zoom_button_at(&self, x: f32, y: f32) -> Option<AppId> {
        let m = self.desk_metrics();
        let d = self.desk.as_ref()?;
        match d.pick(&m, x, y) {
            Some((i, Hit::Light(Light::Zoom))) => Some(d.wins[i].app),
            _ => None,
        }
    }

    /// Whether a press at (x, y) is on the green button the open menu hangs from.
    #[cfg(feature = "desktop-wayland")]
    pub(super) fn arrange_on_button(&self, x: f32, y: f32) -> bool {
        let Some(a) = self.desk.as_ref().and_then(|d| d.arrange) else { return false };
        self.zoom_button_at(x, y) == Some(a.app)
    }

    /// Whether the pointer at (x, y) keeps the open menu up: on its green button, inside the menu, or in the strip
    /// between them (the menu opens under the button, so the pointer crosses it on the way in).
    #[cfg(feature = "desktop-wayland")]
    fn arrange_keeps(&self, x: f32, y: f32) -> bool {
        if self.arrange_on_button(x, y) { return true; }
        let (Some(a), Some((px, py, pw, ph))) = (self.desk.as_ref().and_then(|d| d.arrange), self.desktop_popup_rect()) else { return false };
        let slack = self.ui.pt(ARRANGE_GAP_PT * 2.0);
        let (left, top, right, bottom) = (px as f32 - slack, a.y - self.ui.pt(ARRANGE_BRIDGE_PT), (px + pw) as f32, (py + ph) as f32);
        x >= left && x < right && y >= top && y < bottom
    }

    /// Per tick: opens the green-button menu once the pointer has rested on the button for `ARRANGE_DWELL_MS`. A rest
    /// that cannot open it (a menu or a drag is under way, the screen is locked) is dropped, so nothing keeps waking the shell.
    #[cfg(feature = "desktop-wayland")]
    pub(super) fn arrange_tick(&mut self, t: f64) {
        let Some((app, since)) = self.desk.as_ref().and_then(|d| d.arrange_at) else { return };
        if t - since < ARRANGE_DWELL_MS { return; }
        // a full-screen window has no traffic lights to rest on: a rest that began before it went full screen is dropped
        if self.desk.as_ref().is_some_and(|d| d.index_of(app).and_then(|i| d.wins.get(i)).is_some_and(|w| w.full)) {
            if let Some(d) = self.desk.as_mut() { d.arrange_at = None; }
            return;
        }
        let m = self.desk_metrics();
        let button = self.desk.as_ref().and_then(|d| d.index_of(app).and_then(|i| d.wins.get(i))).map(|w| w.light_rect(&m, 2));
        let idle = self.desk.as_ref().is_some_and(|d| d.grab_idle());
        let anchor = button.filter(|_| idle && self.desktop_menu.is_none() && !self.is_locked())
            .map(|b| (b.x as f32, (b.y + b.h) as f32 + self.ui.pt(ARRANGE_GAP_PT)));
        let Some(d) = self.desk.as_mut() else { return };
        match anchor {
            Some((x, y)) => {
                d.arrange = Some(ArrangeMenu { app, x, y });
                // resting on the button keeps the menu from opening again once it has closed
                d.arrange_at = Some((app, f64::INFINITY));
                self.desktop_menu = Some(super::desktop_bar::Menu::Arrange);
                self.desktop_menu_row = None;
            }
            None => d.arrange_at = None,
        }
        self.dirty = true;
    }

    /// Carries out a green-button menu choice for the window of `app`: the window comes to the front, then the
    /// choice is made (the window server places it in the Wayland session).
    #[cfg(feature = "desktop-wayland")]
    pub(super) fn desk_arrange(&mut self, app: AppId, what: Arrange, _t: f64) {
        let Some(d) = self.desk.as_mut() else { return };
        d.arrange = None;
        d.arrange_at = None;
        let Some(i) = d.index_of(app) else { return };
        // the window that shares the screen with it: the top-most other visible one, found before this one is raised
        let other = d.wins.iter().rev().find(|o| o.app.parent() != app.parent() && !o.minimized).map(|o| o.app);
        d.raise_front = true;
        let i = d.raise(i);
        if !self.is_locked() { self.screen = Screen::App(app); }
        match what {
            Arrange::Tile(tile) => self.desk_tile_window(i, tile),
            Arrange::Previous => self.desk_restore_window(i),
            Arrange::FullScreen(on) => self.desk_set_full_screen(app, on),
            Arrange::SideBySide => {
                self.desk_tile_window(i, Tile::Left);
                if let Some(j) = other.and_then(|o| self.desk.as_ref().and_then(|d| d.index_of(o))) {
                    self.desk_tile_window(j, Tile::Right);
                }
            }
        }
        self.dirty = true;
    }

    /// Whether the menu bar is hidden now: the front window is full screen and the pointer is not at the top.
    pub(super) fn desk_bar_hidden(&self) -> bool {
        // a menu opened from the revealed bar keeps the bar while the pointer is down in the menu
        // a screen recording keeps the bar: its indicator must stay visible for the whole recording, even over a full-screen window
        let full = self.desk.as_ref().is_some_and(|d| d.front_full()) && self.desktop_menu.is_none() && !self.recording_active();
        bar_hidden(full, self.pointer_y, self.desktop_bar_height() as f32)
    }

    /// Full screen for an app's window: the green button, or the app's own request (`ToShell::SetFullScreen`). The app
    /// hears it as `ToApp::FullScreen`. Game Mode reads the front window's state (`Desk::front_full`).
    pub(super) fn desk_set_full_screen(&mut self, app: AppId, on: bool) {
        let m = self.desk_metrics();
        let now = self.last_input;
        let Some(d) = self.desk.as_mut() else { return };
        let Some(i) = d.index_of(app) else { return };
        if !d.set_full_screen(&m, i, on) {
            return;
        }
        // the Dock slides away with the menu bar and comes back when the window leaves full screen; either edge reveals it
        d.arrange = None;
        // a rest on the green button that began before the click must not open a menu on the window the click changed
        d.arrange_at = None;
        if on {
            d.dock_hide_at = Some(now);
        } else {
            d.dock_hide_at = None;
            d.dock_visible = !d.dock_preferences.autohide;
            d.dock_vis_anim = None;
        }
        if self.desktop_menu == Some(super::desktop_bar::Menu::Arrange) { self.desktop_menu = None; }
        self.desk_layout(i);
        if let AppId::External(slot) = app {
            if let Some(Some(h)) = self.ext.get_mut(slot as usize) {
                h.send(&bifrost_proto::ToApp::FullScreen { on });
            }
        }
        self.dirty = true;
    }

    pub(super) fn desk_zoom_window(&mut self,i:usize) {let m=self.desk_metrics();self.desk.as_mut().unwrap().toggle_zoom(&m,i);self.desk_layout(i);if let Some(app)=self.desk.as_ref().and_then(|d|d.wins.get(i)).map(|w|w.app){let t=self.last_input;self.desk_fx_start(app,FxCall::Zoom,t);}self.dirty=true;}

    /// Minimizes window `i` to its Dock tile, playing the minimize effect first when it is allowed.
    pub(super) fn desk_minimize_window(&mut self, i: usize, t: f64) {
        let Some(app) = self.desk.as_ref().and_then(|d| d.wins.get(i)).map(|w| w.app) else { return };
        // the effect starts while the window is still mapped; the draw loop keeps it up until the effect ends
        self.desk_fx_start(app, FxCall::Minimize, t);
        // its Dock thumbnail: one picture, asked for while the window server still has it on screen
        self.dock_shot(app, t);
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
        let minimize_effect = window_fx::MinimizeEffect::from_index(super::pages::DeskPrefs::load(&self.prefs).minimize_effect);
        let effect = match call {
            FxCall::Open => window_fx::Effect::Open,
            FxCall::Minimize => window_fx::Effect::Minimize { to_dx: dx, to_dy: dy, how: minimize_effect },
            FxCall::Restore => window_fx::Effect::Restore { from_dx: dx, from_dy: dy, how: minimize_effect },
            FxCall::Zoom => window_fx::Effect::Zoom,
            FxCall::Close => window_fx::Effect::Close,
        };
        let behavior = match app.parent() {
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
        // one tile per app id: a key pinned twice, two entries of one app, or an instance whose id differs from the
        // pinned key never make a second tile (the pinned tile shows the running dot)
        #[cfg_attr(not(feature = "desktop-wayland"),allow(unused_mut))]
        let mut list:Vec<usize>=super::dock_tiles::dock_app_tiles(apps(),&desk.dock_preferences.pinned,|i|self.dock_entry_running(i));
        #[cfg(feature = "desktop-wayland")]
        {let installed=apphost::installed();
        let ids:Vec<&str>=desk.linux_apps.iter().map(|a|a.app_id.as_str()).collect();
        list.extend(super::dock_tiles::linux_tiles(&installed,apps(),&ids).into_iter().map(|i|apps().len()+1+i));}
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
        // a collapsed window keeps the size its app had: the title bar alone is not a size to give the app
        if !self.desk.as_ref().is_some_and(|d| d.is_collapsed(w.app)) {
            self.resize_app(w.app, b.w, b.h);
        }
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
        // an extra window is raised (it is opened by its app, not by the shell)
        if let AppId::ExternalWindow(slot, id) = app {
            self.desk_focus_extra_window(slot, id);
            return;
        }

        // a limit already reached covers the window this opens (see `st_launch_check`); a blocked app's own
        // process (native route) is then never started
        let blocked = self.st_launch_check(app, t);
        self.recent_note(app);
        if let Some(d)=self.desk.as_mut(){d.raise_front=true;}
        // the Dock tile bounces while the app launches, whichever way it was opened (a running app starts none)
        if let Some(k)=apps().iter().position(|a|a.id==app){self.dock_start_bounce(k,t);}
        #[cfg(feature = "desktop-wayland")]
        if let AppId::External(slot)=app { if !blocked && self.desk_direct_open(slot as usize) { return; } }
        #[cfg(feature = "desktop-wayland")]
        if app == AppId::Browser && !blocked {
            if let Some(slot)=apphost::slot_of("is.olibuijr.browser.desktop") {self.desk_native_open(AppId::External(slot as u8));return;}
        }
        #[cfg(feature = "desktop-wayland")]
        if !blocked && self.desk_native_open(app) { return; }
        #[cfg(not(feature = "desktop-wayland"))]
        let _ = blocked;
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
        if !self.is_locked() {
            self.screen = Screen::App(app);
        }
        // the phone dock's recents are not kept on the desktop: Recent Items has its own list
        self.dirty = true;
    }

    /// Focus follows the frontmost visible window; with none, the desktop (Home) shows.
    pub(super) fn desk_refocus(&mut self) {
        // the lock screen stays up whatever the windows do: a per-tick refocus must never lift a window over it
        if self.is_locked() {
            self.dirty = true;
            return;
        }
        self.screen = match self.desk.as_ref().and_then(|d| d.front()).map(|w| w.app) {
            Some(app) => Screen::App(app.parent()),
            None => Screen::Home,
        };
        self.dirty = true;
    }

    /// Alt/Ctrl/Meta+Tab on the desktop: raises the visible window behind the front one,
    /// so repeated presses toggle between the two most recent windows.
    #[cfg(any(test, feature = "desktop-wayland"))]
    pub(super) fn desk_cycle_window(&mut self) {
        if let Some(d) = &mut self.desk {
            // Cmd+` walks the windows of the front app, its extra windows included
            let front = d.front().map(|w| w.app.parent());
            match &mut d.cmdtab {
                Some((order, sel)) => *sel = (*sel + 1) % order.len(),
                None => {
                    let order: Vec<AppId> = d.wins.iter().rev().map(|w| w.app).filter(|a| Some(a.parent()) == front).collect();
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

    /// True while the Cmd+Tab strip is open.
    pub(super) fn cmdtab_open(&self) -> bool {
        self.desk.as_ref().is_some_and(|d| d.cmdtab.is_some())
    }

    /// Where the Cmd+Tab strip sits on a `sw` by `sh` screen: the capsule's rectangle and the size of its parts, in px.
    /// Centred; with many windows the icons shrink so the strip stays inside 90% of the width. None while it is closed.
    fn cmdtab_layout(&self, sw: f32, sh: f32) -> Option<CmdtabLayout> {
        let order = &self.desk.as_ref()?.cmdtab.as_ref()?.0;
        let ui = &self.ui;
        let (gap, pad) = (ui.pt(CMDTAB_GAP_PT), ui.pt(CMDTAB_PAD_PT));
        let n = order.len().max(1) as f32;
        let icon = ui.pt(CMDTAB_ICON_PT).min(((sw * 0.9 - 2.0 * pad - (n - 1.0) * gap) / n).floor()).max(1.0);
        let (w, h) = (n * icon + (n - 1.0) * gap + 2.0 * pad, icon + 2.0 * pad + ui.pt(CMDTAB_LABEL_PT));
        let (x, y) = (((sw - w) / 2.0).round(), ((sh - h) / 2.0).round());
        Some(CmdtabLayout { x, y, w, h, icon: icon as usize, gap, pad })
    }

    /// The strip's rectangle on the screen in px (x, y, w, h), clipped to the screen; None while it is closed. On the
    /// desktop the strip is its own overlay surface above the windows (`desktop_chrome_frames`, id 13) and a region the
    /// GPU presenter keeps (`desk_regions_now`): drawn only into the desktop frame it would sit under the windows.
    #[cfg(feature = "desktop-wayland")]
    pub(super) fn cmdtab_rect(&self) -> Option<(usize, usize, usize, usize)> {
        let l = self.cmdtab_layout(self.w as f32, self.h as f32)?;
        let (x, y) = (l.x.max(0.0) as usize, l.y.max(0.0) as usize);
        let w = (l.w.ceil() as usize).min((self.w as usize).saturating_sub(x));
        let h = (l.h.ceil() as usize).min((self.h as usize).saturating_sub(y));
        (w > 0 && h > 0).then_some((x, y, w, h))
    }

    /// The strip as an overlay surface: its rectangle and the alpha mask of its rounded ends.
    #[cfg(feature = "desktop-wayland")]
    pub(super) fn cmdtab_overlay(&self) -> Option<((usize, usize, usize, usize), Vec<u8>)> {
        let (x, y, w, h) = self.cmdtab_rect()?;
        Some(((x, y, w, h), crate::gfx::rrect_mask(w, h, self.ui.pt(CMDTAB_RADIUS_PT))))
    }

    /// The Cmd+Tab strip: a glass capsule of app icons, the selection on a plate with its name below.
    fn draw_cmdtab(&self, s: &mut Surface) {
        let Some((order, sel)) = self.desk.as_ref().and_then(|d| d.cmdtab.as_ref()) else { return };
        let Some(CmdtabLayout { x, y, w, h, icon, gap, pad }) = self.cmdtab_layout(s.w as f32, s.h as f32) else { return };
        let ui = &self.ui;
        // frosted like Search: the presenter's blur under the capsule (a flat fill pushes no glass shape)
        super::quicksearch::mac_glass(s, x, y, w, h, ui.pt(CMDTAB_RADIUS_PT), 255);
        s.stroke_rrect(x, y, w, h, ui.pt(CMDTAB_RADIUS_PT), ui.pt(1.0).max(1.0), SEPARATOR_C.now());
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
        let t = self.last_input;
        let shown: Vec<AppId> = self.desk.as_ref().map(|d| d.wins.iter().filter(|w| !w.minimized).map(|w| w.app).collect()).unwrap_or_default();
        for app in shown {
            self.dock_shot(app, t);
        }
        if let Some(d) = &mut self.desk {
            for w in &mut d.wins {
                w.minimized = true;
            }
        }
        self.desk_refocus();
    }

    pub(super) fn desk_close_app(&mut self, app: AppId) {
        // an extra window's traffic light closes that window through its app; the app keeps running
        if let AppId::ExternalWindow(slot, id) = app {
            self.desk_close_extra_window(slot, id);
            return;
        }
        // the app's extra windows go with it (a closed or dead app keeps none on the desk)
        if let AppId::External(slot) = app {
            self.desk_drop_app_extras(slot);
        }
        // a live direct app's window is the window server's: quitting the app closes it. A dead app's host stays
        // listed, and its desk windows (if any) are removed below.
        #[cfg(feature = "desktop-wayland")]
        if let AppId::External(slot)=app { if self.ext.get(slot as usize).and_then(Option::as_ref).is_some_and(|h|h.running()&&h.route()==apphost::Route::Direct) { self.ext_close(slot as usize); return; } }
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

    /// Quits the installed SDK app in `slot`: its desk windows go and the app is sent `Quit`, so it saves its state and exits
    /// (`AppHost::begin_quit` kills it after the grace period). The Dock's Quit and Cmd+Q use it. A direct app's window is the
    /// window server's, so `desk_close_app` already quits it; an app on the shared-memory route only loses its desk window there
    /// and its process is quit here. Cmd+W and the traffic light close the window and leave the app running.
    pub(super) fn desk_quit_app(&mut self, slot: u8) {
        self.desk_close_app(AppId::External(slot));
        if self.ext.get(slot as usize).and_then(Option::as_ref).is_some() {
            self.ext_close(slot as usize);
        }
    }

    /// The desk windows that float: each app whose host asked for `ToShell::WindowLevel`, and each extra window its app
    /// set to `ToShell::WindowLevelOf`.
    pub(super) fn floating_apps(&self) -> Vec<AppId> {
        let mut out = Vec::new();
        for (slot, host) in self.ext.iter().enumerate() {
            let Some(h) = host else { continue };
            if h.window_level == bifrost_proto::WindowLevel::Floating {
                out.push(AppId::External(slot as u8));
            }
            out.extend(h.windows.floating_ids(&h.id).into_iter().filter_map(|id| u16::try_from(id).ok()).map(|id| AppId::ExternalWindow(slot as u8, id)));
        }
        out
    }

    /// The extra windows follow each app host's table (`app_window_table::desk_changes`): a window the app opened gets a
    /// desk window of its own, one the app closed (or the table dropped) leaves the desk. Runs from the app host tick.
    pub(super) fn desk_sync_extra_windows(&mut self) {
        // an extra window whose app has no live process (never started, exited, or its host dropped) has no window
        let orphans: Vec<(u8, u16)> = self.desk.as_ref().map_or_else(Vec::new, |d| {
            d.wins.iter().filter_map(|w| match w.app {
                AppId::ExternalWindow(s, id) if !self.ext.get(s as usize).and_then(Option::as_ref).is_some_and(|h| h.running()) => Some((s, id)),
                _ => None,
            }).collect()
        });
        // this runs every tick: only a change to the desk asks for a frame (a settled desktop draws nothing)
        let mut changed = !orphans.is_empty();
        for (s, id) in orphans {
            self.desk_drop_extra_window(s, id);
        }
        // a main window whose installed app has no host at all (a restored window whose app never came back) is no window
        let registered = apphost::installed();
        let ghosts: Vec<u8> = self.desk.as_ref().map_or_else(Vec::new, |d| {
            d.wins.iter().filter_map(|w| match w.app {
                AppId::External(s) if registered.get(s as usize).is_some() && self.ext.get(s as usize).and_then(Option::as_ref).is_none() => Some(s),
                _ => None,
            }).collect()
        });
        changed |= !ghosts.is_empty();
        for s in ghosts {
            self.desk_close_app(AppId::External(s));
        }
        let Some(d) = self.desk.as_ref() else { return };
        let k = self.ui.pt(1.0) as f32;
        let mut plan = Vec::new();
        for (slot, host) in self.ext.iter().enumerate() {
            let Some(h) = host else { continue };
            let shown: Vec<u32> = d
                .wins
                .iter()
                .filter_map(|w| match w.app {
                    AppId::ExternalWindow(s, id) if s as usize == slot => Some(u32::from(id)),
                    _ => None,
                })
                .collect();
            let (open, close) = h.windows.desk_changes(&h.id, &shown);
            if !open.is_empty() || !close.is_empty() {
                let sizes: Vec<(u32, f32, f32)> = h.windows.of_app(&h.id).iter().map(|x| (x.id, x.w, x.h)).collect();
                plan.push((slot as u8, open, close, sizes));
            }
        }
        // a window that changed its level moves within the stack even when the table's windows did not change
        let floating = self.floating_apps();
        if let Some(d) = self.desk.as_mut() {
            changed |= d.sync_levels(floating);
        }
        changed |= !plan.is_empty();
        let m = self.desk_metrics();
        for (slot, open, close, sizes) in plan {
            for id in &close {
                if let Ok(id) = u16::try_from(*id) {
                    self.desk_drop_extra_window(slot, id);
                }
            }
            for id in open {
                let Ok(id16) = u16::try_from(id) else { continue };
                // the table's size is in points; the desk window is outer px (body plus title bar)
                let (w, h) = sizes.iter().find(|s| s.0 == id).map_or((480.0, 560.0), |s| (s.1, s.2));
                let size = ((w * k) as i32, (h * k) as i32 + m.title);
                if let Some(d) = self.desk.as_mut() {
                    let i = d.open(&m, AppId::ExternalWindow(slot, id16), size);
                    self.desk_layout(i);
                }
            }
        }
        // the placement and collapse the table asks for (`WindowMove`, `WindowCollapse`), on every window the desk shows
        let mut asks = Vec::new();
        for (slot, host) in self.ext.iter_mut().enumerate() {
            let Some(h) = host else { continue };
            let ids: Vec<u32> = h.windows.of_app(&h.id).iter().map(|w| w.id).collect();
            for id in ids {
                let pending = h.windows.take_pending_move(&h.id, id);
                let collapsed = h.windows.of_app(&h.id).iter().any(|w| w.id == id && w.collapsed);
                asks.push((slot as u8, id, pending, collapsed));
            }
        }
        for (slot, id, pending, collapsed) in asks {
            if let Ok(id16) = u16::try_from(id) {
                changed |= self.ext_window_place(AppId::ExternalWindow(slot, id16), pending, collapsed);
            }
        }
        if changed {
            self.desk_refocus();
            self.dirty = true;
        } else if !self.is_locked() {
            // the front window decides the screen on every tick; only a different answer is something to draw
            let screen = match self.desk.as_ref().and_then(|d| d.front()).map(|w| w.app) {
                Some(app) => Screen::App(app.parent()),
                None => Screen::Home,
            };
            if screen != self.screen {
                self.screen = screen;
                self.dirty = true;
            }
        }
    }

    /// Applies the placement and collapse the table asks for to one extra window's desk window. A requested position is
    /// clamped to the visible area (points, relative to the work area) and the window moves there.
    pub(super) fn ext_window_place(&mut self, app: AppId, pending: Option<(f32, f32)>, collapsed: bool) -> bool {
        let Some(d) = self.desk.as_ref() else { return false };
        let Some(i) = d.index_of(app) else { return false };
        // nothing asked and nothing to change (the common case, once per window per tick): no layout work, no frame
        if pending.is_none() && d.is_collapsed(app) == collapsed {
            return false;
        }
        let m = self.desk_metrics();
        let k = self.ui.pt(1.0) as f32;
        let a = Desk::area(&m);
        let Some(d) = self.desk.as_mut() else { return false };
        if let Some((x, y)) = pending {
            let win_w = d.wins[i].r.w as f32 / k;
            if let Some((cx, cy)) = super::app_window_table::clamp_to_screen(x, y, a.w as f32 / k, a.h as f32 / k, win_w) {
                d.move_to(&m, i, a.x + (cx * k).round() as i32, a.y + (cy * k).round() as i32);
            }
        }
        if d.is_collapsed(app) != collapsed {
            d.set_collapsed(&m, i, collapsed);
            self.desk_layout(i);
        }
        self.dirty = true;
        true
    }

    /// A drag of an extra window ended: its app is told where the window is now (`WindowMoved`), in points relative to the
    /// visible desktop area.
    pub(super) fn ext_window_dropped(&mut self, app: AppId) {
        let m = self.desk_metrics();
        let AppId::ExternalWindow(slot, id) = app else { return };
        let Some(r) = self.desk.as_ref().and_then(|d| d.index_of(app).map(|i| d.wins[i].r)) else { return };
        let k = self.ui.pt(1.0) as f32;
        let a = Desk::area(&m);
        let (x, y) = ((r.x - a.x) as f32 / k, (r.y - a.y) as f32 / k);
        if let Some(Some(host)) = self.ext.get_mut(slot as usize) {
            host.window_dragged(u32::from(id), x, y);
        }
    }

    /// The title bar of extra window `id` was double-clicked: true when its app opted in and was told.
    pub(super) fn ext_window_title_double_click(&mut self, slot: usize, id: u32) -> bool {
        match self.ext.get_mut(slot) {
            Some(Some(host)) => host.window_title_double_clicked(id),
            _ => false,
        }
    }

    /// Each app's key window follows the desk's front window: an extra window in front is key for its app, and every
    /// other app's key window is its main window. Runs from the app host tick and before the input is routed.
    pub(super) fn sync_key_windows(&mut self) {
        let front = self.desk.as_ref().and_then(|d| d.front()).map(|w| w.app);
        for (slot, host) in self.ext.iter_mut().enumerate() {
            let Some(h) = host else { continue };
            let key = match front {
                Some(AppId::ExternalWindow(s, id)) if s as usize == slot => u32::from(id),
                _ => 0,
            };
            h.set_key_window(key);
        }
    }

    /// Removes every extra window of the app in `slot` from the desk (the app exited or is quitting).
    pub(super) fn desk_drop_app_extras(&mut self, slot: u8) {
        if let Some(d) = self.desk.as_mut() {
            while let Some(i) = d.wins.iter().position(|w| matches!(w.app, AppId::ExternalWindow(s, _) if s == slot)) {
                d.close(i);
            }
        }
    }

    /// Removes an extra window's desk window without telling its app (the app's table already dropped it).
    fn desk_drop_extra_window(&mut self, slot: u8, id: u16) {
        if let Some(d) = self.desk.as_mut() {
            if let Some(i) = d.index_of(AppId::ExternalWindow(slot, id)) {
                d.close(i);
            }
        }
    }

    /// The traffic light (or the app switcher) closes an extra window: the app is told (`WindowClosed`) and keeps running.
    pub(super) fn desk_close_extra_window(&mut self, slot: u8, id: u16) {
        if let Some(Some(h)) = self.ext.get_mut(slot as usize) {
            h.close_window_from_shell(u32::from(id));
        }
        self.desk_drop_extra_window(slot, id);
        self.desk_refocus();
        self.dirty = true;
    }

    /// Raises an extra window (Window menu, Dock app menu). False when the window is not on the desk.
    pub(super) fn desk_focus_extra_window(&mut self, slot: u8, id: u16) -> bool {
        let Some(d) = self.desk.as_mut() else { return false };
        let Some(i) = d.index_of(AppId::ExternalWindow(slot, id)) else { return false };
        d.raise(i);
        self.desk_refocus();
        self.dirty = true;
        true
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
                self.screen = Screen::App(app.parent());
                self.fullscreen = false;
                // the window now in front takes over the keyboard's lift
                self.desk_keyboard_lift();
                let w = self.desk.as_ref()?.wins[i];
                let mut hit = hit;
                // a collapsed window has no edges to resize: its title bar is all there is
                if matches!(hit, Hit::Edge(_)) && self.desk.as_ref()?.is_collapsed(app) {
                    hit = Hit::Title;
                }
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
                    let double = self.desk.as_ref()?.last_title.is_some_and(|(a, t0)| a == app && t - t0 <= double_click_ms());
                    if double {
                        self.desk.as_mut()?.last_title = None;
                        // an app that opted in gets the double-click itself; any other window zooms
                        let told = match app {
                            AppId::ExternalWindow(slot, id) => self.ext_window_title_double_click(slot as usize, u32::from(id)),
                            _ => false,
                        };
                        if !told {
                            self.desk.as_mut()?.toggle_zoom(&m, i);
                            self.desk_layout(i);
                        }
                        self.dirty = true;
                        return Some(vec![]);
                    }
                    self.desk.as_mut()?.last_title = Some((app, t));
                }
                let grab = match hit {
                    Hit::Light(l) => Grab::Light(i, l),
                    Hit::Title => Grab::Move { dx: x - w.r.x as f32, dy: y - w.r.y as f32, app },
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
                    Grab::Move { dx, dy, app } => {
                        let i = self.desk.as_ref()?.index_of(app)?;
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
                                Light::Zoom if matches!(w.app, AppId::External(_)) => {
                                    // the green button of an app window takes it full screen (its own space)
                                    self.desk_set_full_screen(w.app, !w.full);
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
                    Some(Grab::Move { app: moved, .. }) => {
                        // a title bar dropped on a screen edge or corner tiles the window there (snap_zone)
                        let band = self.ui.pt(8.).round() as i32;
                        if let (Some(tile), Some(i)) = (snap_zone(Desk::area(&m), x as i32, y as i32, band), self.desk.as_ref()?.index_of(moved)) {
                            if let Some(d) = self.desk.as_mut() { d.tile(&m, i, tile); }
                            self.desk_layout(i);
                            self.desk_frame_changed(i);
                        }
                        // the person dropped a window: its app learns where it is now (`WindowMoved`, in points)
                        self.ext_window_dropped(moved);
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
        let launch=crate::bare::account_policy::check_launch(&package.manifest.id).and_then(|()|crate::bare::app_policy::Policy::from_permissions(&package.manifest.id,&package.manifest.permissions)).and_then(|policy|crate::bare::app_policy::command(&package.manifest.id,&package.exec_path(),&apphost::root().join("appdata").join(&package.manifest.id),policy,true)).and_then(|mut cmd|cmd.spawn());
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
        // the live frame while an animated or time-of-day look is chosen (cached: this only copies), else the still
        let wp = self.wallpaper_frame();
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
            if w.minimized || animating || matches!(w.app.parent(), AppId::External(_)) {
                return None;
            }
            let radius = if w.maximized { 0 } else { self.ui.pt(bifrost_ui::prefs_layout::m::WINDOW_RADIUS).ceil() as i32 };
            Some(Cover { shown: grown(w.r, reach), opaque: opaque_interior(w.r, radius) })
        }).collect()
    }

    /// The body rectangle of `app`'s window (below its title bar) that a Time Limit cover fills.
    pub(super) fn desk_cover_rect(&self, app: AppId) -> Option<(i32, i32, i32, i32)> {
        let d = self.desk.as_ref()?;
        let w = d.wins.iter().find(|w| w.app == app && !w.minimized)?;
        let top = title_of(app, &self.desk_metrics());
        let h = w.r.h - top;
        (h > 0 && w.r.w > 0).then_some((w.r.x, w.r.y + top, w.r.w, h))
    }

    /// Draws the Time Limit cover over its window's body. The frame, title bar and lights are drawn
    /// before this, so the window can still be moved or closed. Clipped to the surface.
    fn desk_cover(&mut self, s: &mut Surface, w: &Win, m: &Metrics, t: f64) {
        let top = title_of(w.app, m);
        let (cw, ch) = (w.r.w, w.r.h - top);
        let Some(sh) = self.shield.as_mut() else { return };
        if sh.app != w.app || cw <= 0 || ch <= 0 { return; }
        sh.resize(cw, ch);
        let (cx, cy) = (w.r.x, w.r.y + top);
        let mut img = vec![0u32; cw as usize * ch as usize];
        sh.draw(&mut Surface { w: cw as usize, h: ch as usize, stride: cw as usize, px: &mut img, y0: 0 }, &self.ui, t);
        let (x_lo, x_hi) = (cx.max(0), (cx + cw).min(s.w as i32));
        let (y_lo, y_hi) = (cy.max(s.y0 as i32), (cy + ch).min(s.h as i32));
        for y in y_lo..y_hi {
            for x in x_lo..x_hi {
                let src = img[((y - cy) * cw + (x - cx)) as usize];
                s.px[(y - s.y0 as i32) as usize * s.stride + x as usize] = src;
            }
        }
    }

    /// A Time Limit cover was raised or dropped: the desktop's window caches are stale.
    pub(super) fn desk_cover_changed(&mut self) {
        #[cfg(feature = "desktop-wayland")]
        if let Some(d) = self.desk.as_mut() {
            d.window_cache.clear();
            d.body_cache.clear();
        }
        self.dirty = true;
    }

    /// True when `app`'s window has no content to show yet: an app's window (or one of its extra windows) the app has
    /// not committed a frame to. A window server maps a window on its first committed buffer, not on creation.
    pub(super) fn desk_window_unmapped(&self, app: AppId) -> bool {
        match app {
            // a host whose process is gone keeps its last frame, but the window has nothing live to show
            AppId::External(slot) => self.ext.get(slot as usize).and_then(Option::as_ref).is_some_and(|h| !h.incompatible && (!h.running() || h.frame().is_none())),
            AppId::ExternalWindow(slot, id) => self.ext.get(slot as usize).and_then(Option::as_ref).map_or(true, |h| !h.window_mapped(u32::from(id))),
            _ => false,
        }
    }

    /// Recomputes `Desk::unmapped` from the app hosts; a window that just gained its first frame is repainted.
    pub(super) fn desk_refresh_unmapped(&mut self) {
        let Some(d) = self.desk.as_ref() else { return };
        let now: Vec<AppId> = d.wins.iter().map(|w| w.app).filter(|a| self.desk_window_unmapped(*a)).collect();
        if let Some(d) = self.desk.as_mut() {
            if d.unmapped != now {
                d.unmapped = now;
                self.dirty = true;
            }
        }
    }

    /// The wallpaper, the windows back to front, the menu bar and the dock.
    pub(super) fn draw_desk(&mut self, s: &mut Surface, t: f64) {
        self.draw_wallpaper_layer(s);
        self.draw_desk_windows(s, t);
        self.draw_desk_chrome(s, t);
    }

    /// The desktop's windows, back to front. On the window server's path each window becomes a frame for the presenter
    /// (`Desk::frames`) and `s` is not touched; a frame that only an external app's new pixels asked for runs just this
    /// (`Shell::draw`, the windows-only path), so the bar, the Dock and the wallpaper are not drawn again.
    pub(super) fn draw_desk_windows(&mut self, s: &mut Surface, t: f64) {
        let m = self.desk_metrics();
        #[cfg(feature = "desktop-wayland")]
        if let Some(d) = self.desk.as_mut() { d.hidden_ids.clear(); }
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
            desk.window_cache.retain(|id,_| desk.wins.iter().zip(&fading).any(|(w, f)| (!w.minimized || *f) && frame_app(*id) == Some(w.app)));
            desk.body_cache.retain(|app,_| desk.wins.iter().zip(&fading).any(|(w, f)| (!w.minimized || *f) && w.app == *app));
        }
        self.desk_refresh_unmapped();
        let front = self.desk.as_ref().and_then(|d| d.wins.iter().rposition(|w| !w.minimized && !d.unmapped.contains(&w.app)));
        for i in 0..count {
            let Some(win) = self.desk.as_ref().and_then(|d| d.wins.get(i).copied()) else { break };
            let w = &win;
            if w.minimized && !fading[i] {
                continue;
            }
            // a window whose app has committed no frame has nothing to show: no frame is drawn for it
            if self.desk.as_ref().is_some_and(|d| d.unmapped.contains(&w.app)) {
                continue;
            }
            #[cfg(feature = "desktop-wayland")]
            if std::env::var_os("SPRINGBOARD_WAYLAND").is_some() {
                let id = win_id(w.app).unwrap_or(0);
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
                    // an extra window always takes its app's newest frame (it is blitted each frame)
                    AppId::ExternalWindow(..) => false,
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
                let opaque=!matches!(w.app.parent(),AppId::External(_));
                if hidden[i] {
                    // a window in front covers it: its surface keeps the last pixels presented, and its cache is
                    // dropped so that it repaints in full once it is uncovered
                    self.desk.as_mut().unwrap().hidden_ids.push(id);
                    self.desk.as_mut().unwrap().frames.push(crate::bare::wayland::WindowFrame {
                        id, title: frame_title(id), x:w.r.x, y:w.r.y,
                        width:w.r.w as usize, height:w.r.h as usize, pixels:None,
                        damage:crate::gfx::Damage::new(w.r.w as usize,w.r.h as usize), maximized:w.maximized, fullscreen:w.full, title_height:m.title,
                        drag_height, no_drag, fx:bifrost_render::glass::Fx::IDENTITY, opaque, floating:false,
                    });
                    continue;
                }
                if unchanged && quiet {
                    self.desk.as_mut().unwrap().frames.push(crate::bare::wayland::WindowFrame {
                        id, title: frame_title(id), x:w.r.x, y:w.r.y,
                        width:w.r.w as usize, height:w.r.h as usize, pixels:None,
                        damage:crate::gfx::Damage::new(w.r.w as usize,w.r.h as usize), maximized:w.maximized, fullscreen:w.full, title_height:m.title,
                        drag_height, no_drag, fx:bifrost_render::glass::Fx::IDENTITY, opaque, floating:false,
                    });
                    self.desk.as_mut().unwrap().window_cache.insert(id,cached.unwrap());
                    continue;
                }
                if let AppId::External(slot)=w.app {
                    let width=w.r.w as usize; let height=w.r.h as usize; let top=title_of(w.app,&m) as usize;
                    let body_h=height.saturating_sub(top);
                    let mapped=self.ext.get(slot as usize).and_then(Option::as_ref).filter(|h|h.running()).and_then(|h|h.frame()).map(|(_,w,h)|(w,h));
                    // on a key-state change the app's last frame may still be a few pixels larger than the window (the
                    // size it first drew at); `blit` clips it to the body exactly as the full draw would, so it is taken
                    let fits = mapped==Some((width,body_h)) || (refocus && mapped.is_some_and(|(mw,mh)| mw>=width && mh>=body_h));
                    if (unchanged || refocus) && !self.keyboard.shown() && !self.keyboard.animating() && fits {
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
                        desk.frames.push(crate::bare::wayland::WindowFrame{id,title:frame_title(id),x:w.r.x,y:w.r.y,width,height,pixels:outgoing,damage,maximized:w.maximized,fullscreen:w.full,title_height:m.title,drag_height,no_drag,fx:bifrost_render::glass::Fx::IDENTITY,opaque,floating:false});
                        continue;
                    }
                }
                // only the key state moved on a built-in whose content is quiet (drawn after the last input and the
                // last change Settings reported): its body is reused and only the frame, dim and lights are drawn
                // (bringing another window forward cost a 31 ms frame on the lab PC, round 75)
                let quiet_body = !matches!(w.app, AppId::External(_)) && cached.as_ref().is_some_and(|c|
                    c.width == w.r.w as usize && c.height == w.r.h as usize && c.focused != focused && c.maximized == w.maximized
                    && c.drawn > self.last_input && c.drawn > self.settings_changed && t - c.drawn < 1000.)
                    && (w.app != AppId::Settings || self.settings.as_ref().is_some_and(|st| st.next_wake(t).map_or(true, |x| x > t)));
                self.desk.as_mut().unwrap().reuse_body = quiet_body.then_some(w.app);
                // Reuse the shared scratch buffer instead of a fresh zeroed allocation per window per frame.
                let mut px = std::mem::take(&mut self.desk.as_mut().unwrap().window_buf);
                px.clear(); px.resize(w.r.w as usize * w.r.h as usize, 0);
                let mut image = Image { w: w.r.w as usize, h: w.r.h as usize, px };
                let mut local = *w; local.r.x = 0; local.r.y = 0;
                {
                    let mut surf = Surface { w:image.w,h:image.h,stride:image.w,px:&mut image.px,y0:0 };
                    self.draw_window(&mut surf, &local, Some(i) == front, &m, t, false);
                    self.desk_cover(&mut surf, &local, &m, t);
                }
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
                    id, title: frame_title(id), x:w.r.x, y:w.r.y,
                    width:w.r.w as usize, height:w.r.h as usize, pixels, damage, maximized:w.maximized, fullscreen:w.full, title_height:m.title,
                    drag_height, no_drag, fx:bifrost_render::glass::Fx::IDENTITY, opaque, floating:false,
                });
                continue;
            }
            // on the GPU presenter a partial frame redraws only some regions, so a hidden window is skipped only on the CPU
            if hidden[i] && !bifrost_render::glass::gpu() {
                continue;
            }
            self.draw_window(s, w, Some(i) == front, &m, t, true);
            self.desk_cover(s, w, &m, t);
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
    }

    /// What the shell itself draws over the desktop: the bar, the Dock, menus, the keyboard, the file preview and the HUD.
    fn draw_desk_chrome(&mut self, s: &mut Surface, t: f64) {
        let m = self.desk_metrics();
        self.draw_app_status(s);
        self.draw_desk_dock(s, &m, t);
        // a tablet's on-screen keyboard sits over the desktop, above the windows and the dock
        if self.desk_keyboard_visible() {
            let kb_top = self.kb_top();
            self.keyboard.draw(s, &self.ui, kb_top, t);
            let below = (kb_top + self.keyboard.h() + self.keyboard.offset(t)) as usize;
            s.fill_rect(0, below, self.w as usize, self.h as usize, super::keyboard::BG);
        }
        #[cfg(feature="desktop-wayland")]
        self.draw_desktop_popup(s);
        self.quick_look_prepare(s.w as f32, s.h as f32);
        self.draw_quick_look(s);
        #[cfg(feature = "desktop-wayland")]
        self.draw_utility(s);
        self.draw_cmdtab(s);
        // Developer > Performance HUD: the desktop branch never reached `draw_assistive`, so it never showed
        self.draw_perf(s);
    }

    /// `shadow` is false for a window-local image: the presenter draws the shadow around it.
    fn draw_window(&mut self, s: &mut Surface, w: &Win, focused: bool, m: &Metrics, t: f64, shadow: bool) {
        let ui = &self.ui;
        let r = w.r;
        let rad = if w.maximized || w.full { 0.0 } else { ui.pt(bifrost_ui::prefs_layout::m::WINDOW_RADIUS) };
        // shadow (CPU path; the GPU presenter draws its own): Golden Gate's key window casts a
        // large soft shadow (~40 pt blur, ~16 pt down, ~30% under the bottom edge), others a light one
        if !w.maximized && shadow {
            cached_window_shadow(s, ui, r.x as f32, r.y as f32, r.w as f32, r.h as f32, rad, focused);
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
        // a built-in on the Wayland path keeps its last (undimmed) body too, so a key-state change can reuse it
        #[cfg(feature = "desktop-wayland")]
        let builtin = std::env::var_os("SPRINGBOARD_WAYLAND").is_some() && !matches!(w.app, AppId::External(_));
        #[cfg(not(feature = "desktop-wayland"))]
        let builtin = false;
        let reuse = builtin && self.desk.as_mut().unwrap().reuse_body.take() == Some(w.app) && !self.keyboard.shown() && !self.keyboard.animating();
        #[cfg(feature = "desktop-wayland")]
        let mut buf = if persistent || builtin { self.desk.as_mut().unwrap().body_cache.remove(&w.app).unwrap_or_default() } else { std::mem::take(&mut self.desk.as_mut().unwrap().buf) };
        #[cfg(not(feature = "desktop-wayland"))]
        let mut buf = std::mem::take(&mut self.desk.as_mut().unwrap().buf);
        let fresh = buf.len() != bw * bh;
        if !persistent && !(reuse && !fresh) {buf.clear();}
        buf.resize(bw * bh, BLACK);
        {
            let mut sub = Surface { w: bw, h: bh, stride: bw, px: &mut buf, y0: 0 };
            let app = w.app;
            if let AppId::ExternalWindow(slot, id) = app {
                // an extra window's body is its app's newest frame for that window
                self.ext_window_blit(&mut sub, slot as usize, u32::from(id));
            } else if reuse && !fresh {
                // the body is the one drawn last time: nothing in it changed
            } else if persistent && !fresh && !self.keyboard.shown() && !self.keyboard.animating() {
                if let AppId::External(slot) = app { self.ext_blit_damage(&mut sub,slot as usize,&mut crate::gfx::Damage::new(bw,bh)); }
            } else {
                self.with_size(b.w, b.h, |sh| sh.draw_app_body(&mut sub, app, t));
            }
        }
        if full_size(w.app) && rad > 0.0 { copy_rect_rounded(s, &buf, bw, bh, b.x, b.y, rad); } else { copy_rect(s, &buf, bw, bh, b.x, b.y); }
        if !focused { dim_rect(s.px, s.stride, b.x.max(0) as usize, (b.y.max(0) as usize).saturating_sub(s.y0), ((b.x + b.w).max(0) as usize).min(s.w), ((b.y + b.h).max(0) as usize).saturating_sub(s.y0).min(s.h), BACKGROUND_DIM); }
        #[cfg(feature = "desktop-wayland")]
        if persistent || builtin { self.desk.as_mut().unwrap().body_cache.insert(w.app,buf); }
        else { self.desk.as_mut().unwrap().buf = buf; }
        #[cfg(not(feature = "desktop-wayland"))]
        { self.desk.as_mut().unwrap().buf = buf; }
        let ui = &self.ui;
        // a full-screen window is the app alone: no rim, title or traffic lights
        if w.full { return; }
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
        // an extra window carries the title its app gave it in the window table; the catalogue knows only whole apps
        let table_title: String = match w.app {
            AppId::ExternalWindow(slot, id) => self.ext.get(slot as usize).and_then(Option::as_ref).and_then(|h| h.windows.of_app(&h.id).iter().find(|x| x.id == u32::from(id)).map(|x| x.title.clone())).unwrap_or_default(),
            _ => String::new(),
        };
        let base: &str = if !table_title.is_empty() { &table_title } else if doc_title.is_empty() { apps().iter().find(|a| a.id == w.app.parent()).map_or("", |a| a.name) } else { doc_title };
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

    /// Records the Dock capsule this frame pushed as a glass shape (`None`: none was). A capsule that differs from the
    /// last one is kept in `dock_glass_vacated`: the presenter drops a stored glass shape only when redrawn pixels wholly
    /// cover it, and a Dock that lost an icon or hid changes just a few rows of its old area (the middle of the plate
    /// comes out the same), so the old, wider shape stayed as a pale slab at both ends.
    pub(super) fn note_dock_glass(&self, bar: Option<Rect>) {
        let now = bar.map(|b| (b.x, b.y, b.w, b.h));
        if let Some(prev) = self.dock_glass_bar.replace(now).filter(|p| Some(*p) != now) {
            self.dock_glass_vacated.borrow_mut().push(prev);
        }
    }

    fn draw_desk_dock(&self, s: &mut Surface, m: &Metrics, t: f64) {
        if !self.dock_is_visible(){self.note_dock_glass(None);return;}
        let (list, bar0, rects0) = self.desk_dock(m);
        // autohide slide, then magnification around the pointer (both pure functions of the clock)
        let (bar, slid) = self.dock_slid(bar0, &rects0, t / 1000.0);
        self.note_dock_glass(bifrost_render::glass::gpu().then_some(bar));
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
            // a hairline of the Dock's own divider colour: the separator grey vanished into the dark glass
            let t = ui.pt(1.0).round().max(1.0);
            if a.y == b.y {
                let h = (a.h as f32 * 0.6).round();
                s.fill_rrect((((a.x + a.w + b.x) / 2) as f32 - t / 2.0).round().max(0.0), a.y as f32 + (a.h as f32 - h) / 2.0, t, h, 0.0, super::ui::DOCK_DIVIDER.now());
            } else {
                let w = (a.w as f32 * 0.6).round();
                s.fill_rrect(a.x as f32 + (a.w as f32 - w) / 2.0, (((a.y + a.h + b.y) / 2) as f32 - t / 2.0).round().max(0.0), w, t, 0.0, super::ui::DOCK_DIVIDER.now());
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
                // a minimized window, to the right of the last divider: a thumbnail frame at the window's aspect with
                // the window's own picture in it (a capture taken as it was minimized) or, without one, its app's icon, and a small badge of the app
                if let Some(win) = desk.wins.get(wi) {
                    let frame = thumbnail_frame(*r, win.r.w, win.r.h);
                    let (fx, fy, fw, fh) = (frame.x as f32, frame.y as f32, frame.w as f32, frame.h as f32);
                    let radius = ui.pt(MINIMIZED_THUMB_RADIUS_PT).min(fw.min(fh) / 2.0);
                    s.fill_rrect(fx, fy, fw, fh, radius, SEPARATOR_C.now());
                    let hair = ui.pt(1.0).max(1.0);
                    s.fill_rrect(fx + hair, fy + hair, (fw - 2.0 * hair).max(1.0), (fh - 2.0 * hair).max(1.0), (radius - hair).max(0.0), FIELD_BG.now());
                    // the window's own picture fills the frame when the window server gave one; else the app's icon is in it
                    let picture = self.dock_picture(win.app);
                    if let Some(img) = picture {
                        let inset = hair.round() as i32;
                        let inner = Rect { x: frame.x + inset, y: frame.y + inset, w: (frame.w - 2 * inset).max(1), h: (frame.h - 2 * inset).max(1) };
                        let inner = crate::layout::Rect { x: inner.x, y: inner.y, w: inner.w, h: inner.h };
                        super::switcher::draw_scaled(s, img, inner, (radius - hair).max(0.0), 255);
                    }
                    if let Some(ai) = apps().iter().position(|a| a.id == win.app) {
                        let base = &self.base_icons[ai];
                        let key = |size: usize| DockIconKey { owner: DockIconOwner::App(ai), generation: self.icon_rebuilds, size };
                        if picture.is_none() {
                            let body = ((frame.w.min(frame.h) as f32) * 0.6).round().max(1.0) as usize;
                            desk.with_scaled_icon(key(body), base, body, body, |icon| {
                                s.blit(icon, frame.x + (frame.w - body as i32) / 2, frame.y + (frame.h - body as i32) / 2, 255, 255);
                            });
                        }
                        let badge = ((r.w as f32) * MINIMIZED_BADGE_K).round().max(1.0) as usize;
                        desk.with_scaled_icon(key(badge), base, badge, badge, |icon| {
                            s.blit(icon, frame.x + frame.w - badge as i32 + badge as i32 / 4, frame.y + frame.h - badge as i32 + badge as i32 / 4, 255, 255);
                        });
                    }
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
                let key=DockIconKey{owner:DockIconOwner::Linux(app.app_id.clone()),generation:self.icon_rebuilds,size:r.w as usize};
                desk.with_scaled_icon(key,&app.icon,r.w as usize,r.h as usize,|icon|s.blit(icon,r.x,r.y,255,if pressed {150} else {255}));
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
                let key = DockIconKey { owner: DockIconOwner::App(i), generation: self.icon_rebuilds, size: r.w as usize };
                desk.with_scaled_icon(key, base, r.w as usize, r.h as usize, |icon| s.blit(icon, r.x, r.y, 255, if pressed { 150 } else { 255 }));
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
                    self.draw_dock_label(s, &name, *r, self.dock_preferences().position, label_alpha, MENU_TINT.now(), LABEL.now());
                }
            }
        }
        if let Some(Grab::DockDrag{from,at_x,at_y,moved:true,..})=desk.grab {if let Some(&index)=list.get(from){if let Some(base)=self.base_icons.get(index){let size=rects[from].w as usize;let icon=base.resized(size,size);s.blit(&icon,at_x as i32-size as i32/2,at_y as i32-size as i32/2,255,210);}}}
    }

    /// The Dock's hover label: `name` on the menu material beside the icon slot `r`, faded by `alpha` (0..1). The text is
    /// rendered white and tinted with `ink` by `blit_colored`, so it follows Light and Dark: a plain white blit read on Dark's
    /// plate but left an empty pill on Light's pale one. `plate` and `ink` are the resolved `MENU_TINT` and `LABEL`.
    pub(super) fn draw_dock_label(&self, s: &mut Surface, name: &str, r: Rect, pos: super::desktop_dock::Position, alpha: f32, plate: u32, ink: u32) {
        let ui = &self.ui;
        let a = (alpha * 255.0).round() as u32;
        self.label(name, MAC_CALLOUT_PT, Weight::Regular, WHITE, |im| {
            let (px, py) = (ui.pt(10.0), ui.pt(5.0));
            let (w, h) = (im.w as f32 + 2.0 * px, im.h as f32 + 2.0 * py);
            let (x, y) = match pos {
                super::desktop_dock::Position::Bottom => (((r.x + r.w / 2) as f32 - w / 2.0).clamp(0.0, (s.w as f32 - w).max(0.0)), r.y as f32 - h - ui.pt(8.0)),
                super::desktop_dock::Position::Left => ((r.x + r.w) as f32 + ui.pt(8.0), (r.y + r.h / 2) as f32 - h / 2.0),
                super::desktop_dock::Position::Right => (r.x as f32 - w - ui.pt(8.0), (r.y + r.h / 2) as f32 - h / 2.0),
            };
            s.fill_rrect(x, y, w, h, ui.pt(MENU_RADIUS_PT), super::desktop_dock::fade(plate, alpha));
            s.blit_colored(im, (x + px) as i32, (y + py) as i32, ink, a);
        });
    }
}

/// Cache the CPU window shadow's rounded-rectangle raster. The size and appearance stay identical;
/// only the expensive per-frame SDF coverage walks are replaced by a clipped source-over blit.
fn cached_window_shadow(s: &mut Surface, ui: &super::ui::Ui, x: f32, y: f32, w: f32, h: f32, rad: f32, focused: bool) {
    use std::cell::RefCell;
    type Key = (usize, usize, u32, u32, u32, bool);
    const MAX_CACHE_BYTES: usize = 16 * 1024 * 1024;
    const MAX_CACHE_ENTRIES: usize = 8;
    thread_local! { static CACHE: RefCell<Vec<(Key, usize, usize, Image)>> = const { RefCell::new(Vec::new()) }; }
    let (layers, step_pt, dy_pt, alpha) = if focused { (8, 5.0, 12.0, 9) } else { (4, 4.0, 5.0, 8) };
    let step = ui.pt(step_pt);
    let dy = ui.pt(dy_pt);
    let margin = (step * layers as f32 + dy + 2.0).ceil() as usize;
    let (width, height) = (w.round() as usize, h.round() as usize);
    let key = (width, height, rad.to_bits(), step.to_bits(), dy.to_bits(), focused);
    let Some(image_w) = width.checked_add(margin.saturating_mul(2)) else {
        draw_window_shadow_layers(s, ui, x, y, w, h, rad, focused);
        return;
    };
    let Some(image_h) = height.checked_add(margin.saturating_mul(2)) else {
        draw_window_shadow_layers(s, ui, x, y, w, h, rad, focused);
        return;
    };
    let Some(image_bytes) = image_w.checked_mul(image_h).and_then(|pixels| pixels.checked_mul(std::mem::size_of::<u32>())) else {
        draw_window_shadow_layers(s, ui, x, y, w, h, rad, focused);
        return;
    };
    if image_bytes > MAX_CACHE_BYTES {
        draw_window_shadow_layers(s, ui, x, y, w, h, rad, focused);
        return;
    }
    CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        let index = match cache.iter().position(|(candidate, _, _, _)| *candidate == key) {
            Some(index) => index,
            None => {
                while cache.len() >= MAX_CACHE_ENTRIES || cache.iter().map(|(_, _, bytes, _)| *bytes).sum::<usize>() + image_bytes > MAX_CACHE_BYTES {
                    if cache.is_empty() { break; }
                    cache.remove(0);
                }
                let mut image = Image::new(image_w, image_h);
                let mut shadow = Surface { w: image.w, h: image.h, stride: image.w, px: &mut image.px, y0: 0 };
                for k in (0..layers).rev() {
                    let g = step * (k + 1) as f32;
                    shadow.fill_rrect(margin as f32 - g, margin as f32 - g + dy, width as f32 + 2.0 * g, height as f32 + 2.0 * g, rad + g, rgba(0, 0, 0, alpha));
                }
                shadow.fill_rrect(margin as f32 - 1.0, margin as f32 - 1.0, width as f32 + 2.0, height as f32 + 2.0, rad + 1.0, rgba(0, 0, 0, 40));
                cache.push((key, margin, image_bytes, image));
                cache.len() - 1
            }
        };
        let (_, margin, _, image) = &cache[index];
        s.blit(image, x.round() as i32 - *margin as i32, y.round() as i32 - *margin as i32, 255, 255);
    });
}

fn draw_window_shadow_layers(s: &mut Surface, ui: &super::ui::Ui, x: f32, y: f32, w: f32, h: f32, rad: f32, focused: bool) {
    let (layers, step_pt, dy_pt, alpha) = if focused { (8, 5.0, 12.0, 9) } else { (4, 4.0, 5.0, 8) };
    let step = ui.pt(step_pt);
    let dy = ui.pt(dy_pt);
    for k in (0..layers).rev() {
        let g = step * (k + 1) as f32;
        s.fill_rrect(x - g, y - g + dy, w + 2.0 * g, h + 2.0 * g, rad + g, rgba(0, 0, 0, alpha));
    }
    s.fill_rrect(x - 1.0, y - 1.0, w + 2.0, h + 2.0, rad + 1.0, rgba(0, 0, 0, 40));
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
        let w = Win { app: AppId::External(3), r, maximized: false, minimized: false, full: false, restore: r };
        let node = |x: f32, y: f32| bifrost_proto::Node { id: 1, role: bifrost_proto::Role::Button, label: "Back".into(), value: String::new(), bounds: bifrost_ui::Rect { x, y, w: 64.0, h: 32.0 }, enabled: true };
        let (band, holes) = zone_of(&w, &m, &[node(220.0, 10.0), node(220.0, 300.0)], true);
        assert_eq!(band, (UNIFIED_BAR_PT * 1.5).round() as i32, "the unified bar is the drag band");
        assert_eq!(holes.len(), 2, "the lights and the one control inside the band");
        assert_eq!(holes[1], [330, 15, 96, 48], "control bounds scale from points to window px");
        assert!(holes[0][0] == 0 && holes[0][2] > 0, "the lights are not a drag area");
        let (title, none) = zone_of(&w, &m, &[node(220.0, 10.0)], false);
        assert_eq!((title, none.len()), (m.title, 0), "a normal window drags from its title bar only");
    }

    fn kit_node(role: bifrost_proto::Role, label: &str, x: f32, y: f32, w: f32, h: f32) -> bifrost_proto::Node {
        bifrost_proto::Node { id: 1, role, label: label.into(), value: String::new(), bounds: bifrost_ui::Rect { x, y, w, h }, enabled: true }
    }

    /// The press the shell tests on a full-size window: inside the band, right of the lights, outside every hole.
    fn drags(band: i32, holes: &[[i32; 4]], x: i32, y: i32) -> bool {
        y < band && !holes.iter().any(|r| x >= r[0] && x < r[0] + r[2] && y >= r[1] && y < r[1] + r[3])
    }

    #[test]
    fn empty_toolbar_space_drags_under_a_toolbar_container_and_only_interactive_nodes_block() {
        use bifrost_proto::Role;
        let m = Metrics { pt: 1.5, ..metrics() };
        let r = Rect { x: 100, y: 50, w: 900, h: 600 };
        let w = Win { app: AppId::External(3), r, maximized: false, minimized: false, full: false, restore: r };
        // what the kit publishes for a unified toolbar: the "Toolbar" and "Sidebar" containers, the title labels, two
        // buttons and a search field, in points
        let nodes = [
            kit_node(Role::List, "Toolbar", 220.0, 10.0, 520.0, 32.0),
            kit_node(Role::List, "Sidebar", 0.0, 0.0, 200.0, 600.0),
            kit_node(Role::StaticText, "Notes", 230.0, 12.0, 120.0, 28.0),
            kit_node(Role::Row, "All Notes", 0.0, 40.0, 200.0, 30.0),
            kit_node(Role::Window, "Sheet", 0.0, 0.0, 900.0, 600.0),
            kit_node(Role::Button, "Grid", 600.0, 10.0, 32.0, 32.0),
            kit_node(Role::Button, "More", 640.0, 10.0, 32.0, 32.0),
            kit_node(Role::TextField, "Search", 680.0, 10.0, 100.0, 32.0),
        ];
        let (band, holes) = zone_of(&w, &m, &nodes, true);
        assert_eq!(holes.len(), 4, "the lights plus the two buttons and the search field, nothing else");
        // empty toolbar space between the title block and the buttons, at the height a capsule would be hit
        assert!(drags(band, &holes, (400.0 * 1.5) as i32, (20.0 * 1.5) as i32), "empty space drags");
        assert!(drags(band, &holes, (400.0 * 1.5) as i32, (2.0 * 1.5) as i32), "the top strip drags");
        assert!(drags(band, &holes, (100.0 * 1.5) as i32, (20.0 * 1.5) as i32), "the sidebar's top drags past the lights");
        assert!(drags(band, &holes, (260.0 * 1.5) as i32, (20.0 * 1.5) as i32), "the title label drags");
        for (x, y) in [(616.0, 26.0), (656.0, 26.0), (730.0, 26.0)] {
            assert!(!drags(band, &holes, (x * 1.5) as i32, (y * 1.5) as i32), "({x},{y}) is a control");
        }
        assert!(!drags(band, &holes, (400.0 * 1.5) as i32, band + 1), "below the band is the app");
        assert!(takes_press(Role::SegmentedControl) && takes_press(Role::MenuItem) && !takes_press(Role::ProgressIndicator));
    }

    #[test]
    fn a_store_app_with_a_full_size_manifest_gets_the_52_pt_band() {
        let m = Metrics { pt: 2.0, ..metrics() };
        let r = Rect { x: 0, y: 0, w: 1200, h: 800 };
        let w = Win { app: AppId::External(18), r, maximized: false, minimized: false, full: false, restore: r };
        // `drag_zone` asks `full_size`, which is true for an installed app whose manifest says `window = "full-size"`
        // (the shipped Notes and Mail manifests do); `zone_of` is what it computes for such a window
        for path in ["apps/notes-desktop/manifest.toml", "apps/mail-desktop/manifest.toml"] {
            let text = std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(path)).unwrap();
            assert!(apphost::Manifest::parse(&text).unwrap().full_size_content(), "{path} asks for a full-size window");
        }
        let plain = apphost::Manifest::parse("id = \"is.x.app\"\nname = \"X\"\nexec = \"x\"\n").unwrap();
        assert!(!plain.full_size_content(), "a manifest without the style keeps its title bar");
        let nodes = [kit_node(bifrost_proto::Role::List, "Toolbar", 100.0, 8.0, 400.0, 36.0), kit_node(bifrost_proto::Role::Button, "New", 560.0, 8.0, 36.0, 36.0)];
        let (band, holes) = zone_of(&w, &m, &nodes, true);
        assert_eq!(band, 104, "52 pt at 2 px per point");
        assert!(drags(band, &holes, 600, 40), "empty toolbar space moves the window");
        assert!(!drags(band, &holes, 1130, 40), "the button does not");
        let (title, none) = zone_of(&w, &m, &nodes, false);
        assert_eq!((title, none.len()), (m.title, 0), "a window without the style drags from its title bar");
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
    fn a_window_collapses_to_its_title_bar_and_expands_to_the_height_it_had() {
        let m = metrics();
        let app = AppId::ExternalWindow(0, 4);
        let mut d = Desk::new();
        let i = d.open(&m, app, (300, 400));
        let h = d.wins[i].r.h;
        d.set_collapsed(&m, i, true);
        assert_eq!(d.wins[i].r.h, title_of(app, &m));
        assert!(d.is_collapsed(app));
        d.set_collapsed(&m, i, true);
        assert_eq!(d.wins[i].r.h, title_of(app, &m), "collapsing twice keeps the height it had");
        d.set_collapsed(&m, i, false);
        assert_eq!(d.wins[i].r.h, h);
        assert!(!d.is_collapsed(app));
    }

    #[test]
    fn a_floating_extra_window_is_stacked_last_while_its_app_keeps_its_other_windows_normal() {
        let m = metrics();
        let (a, b) = (AppId::ExternalWindow(0, 1), AppId::ExternalWindow(0, 2));
        let mut d = Desk::new();
        d.open(&m, a, (200, 200));
        d.open(&m, b, (200, 200));
        d.sync_levels(vec![a]);
        assert_eq!(d.wins.last().map(|w| w.app), Some(a), "the floating window is above the normal ones");
        d.sync_levels(vec![]);
        assert!(!d.is_floating(a), "dropping the level puts the window back in the normal band");
    }

    #[test]
    fn closing_a_window_releases_its_buffers() {
        let m = metrics();
        let mut d = Desk::new();
        let i = d.open(&m, AppId::Settings, (900, 600));
        #[cfg(feature = "desktop-wayland")]
        d.body_cache.insert(AppId::Settings, vec![0u32; 1024]);
        d.icon_cache.borrow_mut().insert(DockIconKey { owner: DockIconOwner::App(0), generation: 0, size: 1 }, Image { w: 1, h: 1, px: vec![0; 1] });
        d.close(i);
        #[cfg(feature = "desktop-wayland")]
        assert!(d.body_cache.is_empty());
        assert!(d.icon_cache.borrow().is_empty());
    }

    /// A desktop shell with window capture offered and Settings open; its window index.
    #[cfg(feature = "desktop-wayland")]
    fn desk_with_one_window() -> (Shell, usize) {
        let mut sh = crate::shell::tests::desktop_shell();
        sh.anim = None;
        sh.screen = crate::shell::Screen::Home;
        sh.shots.set_available(true);
        sh.desk_open(AppId::Settings, 100.0);
        let i = sh.desk.as_ref().unwrap().index_of(AppId::Settings).expect("the window is open");
        (sh, i)
    }

    /// The centre pixel of the minimized window's thumbnail frame in the Dock, as the Dock draws it at `t` ms.
    #[cfg(feature = "desktop-wayland")]
    fn minimized_slot_centre(sh: &Shell, wi: usize, t: f64) -> u32 {
        let m = sh.desk_metrics();
        let (list, bar0, rects0) = sh.desk_dock(&m);
        let (_, slid) = sh.dock_slid(bar0, &rects0, t / 1000.0);
        let rects = sh.dock_magnified(&slid, t / 1000.0);
        let k = list.iter().position(|&e| crate::shell::desktop_dock::dock_min_index(e) == Some(wi)).expect("the minimized window has a slot");
        let win = &sh.desk.as_ref().unwrap().wins[wi];
        let frame = thumbnail_frame(rects[k], win.r.w, win.r.h);
        let (w, h) = (m.w as usize, m.h as usize);
        let mut px = vec![0u32; w * h];
        sh.draw_desk_dock(&mut Surface { w, h, stride: w, px: &mut px, y0: 0 }, &m, t);
        px[(frame.y + frame.h / 2) as usize * w + (frame.x + frame.w / 2) as usize]
    }

    #[cfg(feature = "desktop-wayland")]
    #[test]
    fn minimizing_a_window_asks_once_for_its_picture() {
        let (mut sh, i) = desk_with_one_window();
        sh.desk_minimize_window(i, 200.0);
        let reqs = sh.take_capture_requests();
        let id = win_id(AppId::Settings).unwrap();
        assert_eq!(reqs.len(), 1, "one request");
        assert_eq!(reqs[0].source, crate::shell::window_shots::Source::Surface(id), "named by the shell's own window");
        assert_eq!(reqs[0].max, DOCK_SHOT_BOUND);
        assert_eq!(reqs[0].since, 0, "a whole picture, not an unchanged answer");
        assert!(sh.take_capture_requests().is_empty(), "and no more while it stays minimized");
        sh.shots_tick(1000.0);
        sh.shots_tick(3000.0);
        assert!(sh.take_capture_requests().is_empty(), "the Dock never refreshes it");
    }

    #[cfg(feature = "desktop-wayland")]
    #[test]
    fn the_minimized_slot_shows_the_windows_picture_inside_the_thumbnail_frame() {
        let (mut sh, i) = desk_with_one_window();
        sh.desk_minimize_window(i, 200.0);
        // the fallback first: the icon in the frame, no picture
        assert!(sh.dock_picture(AppId::Settings).is_none());
        let icon_centre = minimized_slot_centre(&sh, i, 5000.0);
        let token = sh.take_capture_requests()[0].token;
        let red = crate::gfx::pack(250, 10, 10, 255);
        sh.window_capture_results(vec![(token, crate::shell::window_shots::Outcome::Ready { image: Image { w: 64, h: 40, px: vec![red; 64 * 40] }, serial: 1 })], 300.0);
        assert!(sh.dock_picture(AppId::Settings).is_some());
        let picture_centre = minimized_slot_centre(&sh, i, 5000.0);
        assert_ne!(picture_centre, icon_centre, "the slot no longer shows the icon");
        assert_eq!(picture_centre & 255, 250, "it shows the window's own pixels");
    }

    #[cfg(feature = "desktop-wayland")]
    #[test]
    fn a_failed_capture_leaves_the_icon_in_the_frame() {
        let (mut sh, i) = desk_with_one_window();
        sh.desk_minimize_window(i, 200.0);
        let before = minimized_slot_centre(&sh, i, 5000.0);
        let token = sh.take_capture_requests()[0].token;
        sh.window_capture_results(vec![(token, crate::shell::window_shots::Outcome::Failed)], 300.0);
        assert!(sh.dock_picture(AppId::Settings).is_none());
        assert_eq!(minimized_slot_centre(&sh, i, 5000.0), before, "the same icon-in-frame slot as without window capture");
    }

    #[cfg(feature = "desktop-wayland")]
    #[test]
    fn without_window_capture_minimizing_asks_for_nothing() {
        let (mut sh, i) = desk_with_one_window();
        sh.shots.set_available(false);
        sh.desk_minimize_window(i, 200.0);
        assert!(sh.take_capture_requests().is_empty());
        assert!(sh.dock_picture(AppId::Settings).is_none());
    }

    #[cfg(feature = "desktop-wayland")]
    #[test]
    fn a_closed_windows_picture_is_freed() {
        let (mut sh, i) = desk_with_one_window();
        sh.desk_minimize_window(i, 200.0);
        let token = sh.take_capture_requests()[0].token;
        sh.window_capture_results(vec![(token, crate::shell::window_shots::Outcome::Ready { image: Image { w: 8, h: 8, px: vec![0xff00_0000; 64] }, serial: 1 })], 300.0);
        assert_eq!(sh.shots.held(), 1);
        sh.desk.as_mut().unwrap().close(i);
        sh.shots_tick(10_000.0);
        assert_eq!(sh.shots.held(), 0, "nothing is kept for a window that is gone");
    }

    #[test]
    fn the_dock_icon_cache_is_keyed_by_app_and_generation_not_by_pixel_address() {
        let d = Desk::new();
        let solid = |c: u32| Image { w: 2, h: 2, px: vec![c; 4] };
        let (red, blue) = (solid(0xffff_0000), solid(0xff00_00ff));
        let key = |i: usize, generation: u32| DockIconKey { owner: DockIconOwner::App(i), generation, size: 2 };
        let pick = |k: DockIconKey, src: &Image| d.with_scaled_icon(k, src, 2, 2, |im| im.px[0]);
        assert_eq!(pick(key(1, 0), &red), 0xffff_0000);
        assert_eq!(pick(key(2, 0), &blue), 0xff00_00ff);
        // one buffer standing in for two apps (a freed buffer's address reused) never shares a slot
        assert_eq!(pick(key(3, 0), &blue), 0xff00_00ff);
        assert_eq!(pick(key(1, 0), &red), 0xffff_0000, "slot 1 keeps its own picture");
        // the icons were rebuilt: the new generation draws the new picture, whatever the old one was
        assert_eq!(pick(key(1, 1), &blue), 0xff00_00ff);
        // a Linux client is its own owner, never an `apps()` entry
        let linux = DockIconKey { owner: DockIconOwner::Linux("firefox".into()), generation: 0, size: 2 };
        assert_eq!(pick(linux, &red), 0xffff_0000);
    }

    #[test]
    fn a_full_dock_icon_cache_keeps_only_the_current_generation() {
        let d = Desk::new();
        let img = Image { w: 2, h: 2, px: vec![0xff11_2233; 4] };
        for i in 0..48 {
            d.with_scaled_icon(DockIconKey { owner: DockIconOwner::App(i), generation: 0, size: 2 }, &img, 2, 2, |_| ());
        }
        // 48 entries do not evict. The first generation-1 insert makes 49; the second finds 49, keeps only the
        // generation-1 entry and adds its own.
        d.with_scaled_icon(DockIconKey { owner: DockIconOwner::App(0), generation: 1, size: 2 }, &img, 2, 2, |_| ());
        d.with_scaled_icon(DockIconKey { owner: DockIconOwner::App(1), generation: 1, size: 2 }, &img, 2, 2, |_| ());
        let cache = d.icon_cache.borrow();
        assert_eq!(cache.len(), 2);
        assert!(cache.keys().all(|k| k.generation == 1));
    }

    #[test]
    fn a_dock_capsule_that_changed_or_hid_vacates_its_old_glass_shape() {
        let sh = crate::shell::tests::desktop_shell();
        let capsule = |x: i32, w: i32| Rect { x, y: 900, w, h: 80 };
        let vacated = || sh.dock_glass_vacated.take();
        sh.note_dock_glass(Some(capsule(100, 800)));
        assert!(vacated().is_empty(), "the first capsule leaves nothing behind");
        sh.note_dock_glass(Some(capsule(100, 800)));
        assert!(vacated().is_empty(), "an unchanged capsule leaves nothing behind");
        sh.note_dock_glass(Some(capsule(150, 700)));
        assert_eq!(vacated(), vec![(100, 900, 800, 80)], "a Dock that lost icons vacates its wider capsule");
        sh.note_dock_glass(None);
        assert_eq!(vacated(), vec![(150, 900, 700, 80)], "a hidden Dock vacates its capsule");
        sh.note_dock_glass(None);
        assert!(vacated().is_empty());
    }

    #[test]
    fn a_minimised_thumbnail_takes_the_windows_aspect_inside_its_slot() {
        let slot = Rect { x: 10, y: 20, w: 48, h: 48 };
        let inside = |f: Rect| f.x >= slot.x && f.y >= slot.y && f.x + f.w <= slot.x + slot.w && f.y + f.h <= slot.y + slot.h;
        let wide = thumbnail_frame(slot, 1600, 900);
        assert_eq!((wide.w, wide.h), (48, 27));
        assert_eq!(wide.x + wide.w / 2, slot.x + slot.w / 2, "centred across");
        assert!((wide.y + wide.h / 2 - (slot.y + slot.h / 2)).abs() <= 1, "centred down");
        let tall = thumbnail_frame(slot, 500, 1000);
        assert_eq!((tall.w, tall.h), (24, 48));
        let extreme = thumbnail_frame(slot, 4000, 100);
        assert_eq!((extreme.w, extreme.h), (48, 24), "the aspect is clamped");
        let none = thumbnail_frame(slot, 0, 0);
        assert_eq!((none.w, none.h), (48, 48), "a window without a size gets a square");
        assert!([wide, tall, extreme, none].into_iter().all(inside));
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
    fn full_screen_takes_the_screen_and_goes_back_to_its_size() {
        let m = metrics();
        let mut d = Desk::new();
        d.open(&m, AppId::External(0), (500, 400));
        let before = d.wins[0].r;
        assert!(d.set_full_screen(&m, 0, true));
        assert!(d.wins[0].full && d.front_full());
        assert_eq!(d.wins[0].r, Desk::screen_rect(&m), "the whole screen, under the menu bar");
        assert!(!d.set_full_screen(&m, 0, true), "already full: no change");
        d.toggle_zoom(&m, 0);
        assert_eq!(d.wins[0].r, Desk::screen_rect(&m), "a full window does not zoom");
        assert!(d.set_full_screen(&m, 0, false));
        assert_eq!(d.wins[0].r, before);
        assert!(!d.front_full());
    }

    /// Metrics of an output of `w` x `h` px at `pt` px per point: the sizes scale with it, as the shell's do.
    fn metrics_for(w: i32, h: i32, pt: f32) -> Metrics {
        let px = |v: f32| (v * pt).round() as i32;
        Metrics { w, h, title: px(30.0), menubar: px(28.0), min_w: px(320.0), min_h: px(220.0), edge: px(6.0), light: px(12.0), light_gap: px(8.0), light_x: px(12.0), dock_bottom: px(100.0), pt }
    }

    #[test]
    fn full_screen_covers_the_whole_output_at_its_origin_for_any_size_and_scale() {
        for (w, h, pt) in [(1024, 768, 1.0), (1280, 720, 1.0), (1920, 1080, 1.0), (2560, 1440, 1.5625), (3840, 2160, 2.0), (1440, 3200, 3.0)] {
            let m = metrics_for(w, h, pt);
            let mut d = Desk::new();
            d.open(&m, AppId::ExternalWindow(0, 4), (m.w / 3, m.h / 3));
            let before = d.wins[0].r;
            assert!(d.set_full_screen(&m, 0, true));
            let r = d.wins[0].r;
            assert_eq!((r.x, r.y, r.w, r.h), (0, 0, w, h), "{w}x{h} at {pt}x: the output, not the old origin");
            let body = d.wins[0].body(&m);
            assert_eq!((body.x, body.y, body.w, body.h), (0, 0, w, h), "no title bar in full screen");
            assert!(d.set_full_screen(&m, 0, false));
            assert_eq!(d.wins[0].r, before, "{w}x{h}: the previous frame comes back");
        }
    }

    #[test]
    fn a_zoomed_window_returns_zoomed_from_full_screen_and_keeps_the_frame_before_the_zoom() {
        let m = metrics();
        let mut d = Desk::new();
        d.open(&m, AppId::ExternalWindow(0, 4), (500, 400));
        let before = d.wins[0].r;
        d.toggle_zoom(&m, 0);
        assert_eq!(d.wins[0].r, Desk::area(&m));
        assert!(d.set_full_screen(&m, 0, true));
        assert_eq!(d.wins[0].r, Desk::screen_rect(&m));
        assert!(!d.wins[0].maximized, "full screen is not the zoom");
        assert!(d.set_full_screen(&m, 0, false));
        assert!(d.wins[0].maximized && d.wins[0].r == Desk::area(&m), "back where it was: zoomed");
        d.toggle_zoom(&m, 0);
        assert_eq!(d.wins[0].r, before, "un-zooming still goes to the frame before the zoom");
    }

    #[test]
    fn a_full_screen_window_is_all_content_with_no_lights_or_edges() {
        let m = metrics();
        let mut d = Desk::new();
        d.open(&m, AppId::ExternalWindow(0, 4), (500, 400));
        let lights = (d.wins[0].r.x + m.light_x + 2 * (m.light + m.light_gap) + m.light / 2, d.wins[0].r.y + m.title / 2);
        assert!(matches!(d.wins[0].hit(&m, lights.0 as f32, lights.1 as f32), Some(Hit::Light(_))), "the lights are there before");
        d.set_full_screen(&m, 0, true);
        for (x, y) in [(lights.0, lights.1), (0, 0), (m.w / 2, m.h - 1), (m.w - 1, m.h / 2)] {
            assert_eq!(d.wins[0].hit(&m, x as f32, y as f32), Some(Hit::Content), "({x},{y})");
        }
        assert_eq!(d.wins[0].hit(&m, -50.0, -50.0), None);
    }

    #[test]
    fn a_full_screen_front_window_makes_the_dock_slide_away() {
        let m = metrics();
        let mut d = Desk::new();
        d.open(&m, AppId::ExternalWindow(0, 4), (500, 400));
        assert!(!d.dock_autohides(), "an ordinary window leaves the Dock as the Dock's own setting has it");
        d.set_full_screen(&m, 0, true);
        assert!(d.dock_autohides());
        d.set_full_screen(&m, 0, false);
        assert!(!d.dock_autohides());
    }

    #[test]
    fn the_green_button_menu_has_fill_centre_the_halves_and_quarters_and_full_screen() {
        let lines = arrange_lines(false, false);
        let items: Vec<(&str, Arrange)> = lines.iter().filter_map(|l| if let ArrangeLine::Item(t, a) = l { Some((*t, *a)) } else { None }).collect();
        for tile in [Tile::Fill, Tile::Center, Tile::Left, Tile::Right, Tile::Top, Tile::Bottom, Tile::TopLeft, Tile::TopRight, Tile::BottomLeft, Tile::BottomRight] {
            assert!(items.iter().any(|(_, a)| *a == Arrange::Tile(tile)), "{tile:?} is in the menu");
        }
        assert!(items.iter().any(|(_, a)| *a == Arrange::FullScreen(true)));
        assert!(items.iter().any(|(_, a)| *a == Arrange::Previous));
        assert!(!items.iter().any(|(_, a)| *a == Arrange::SideBySide), "no other window to share the screen with");
        assert!(lines.contains(&ArrangeLine::Header("Move & Resize")));
        for (i, (t, a)) in items.iter().enumerate() {
            assert!(items.iter().skip(i + 1).all(|(u, b)| u != t && b != a), "{t} appears once");
        }
        assert_eq!(lines.first(), Some(&ArrangeLine::Item("Fill", Arrange::Tile(Tile::Fill))));
        assert_ne!(lines.last(), Some(&ArrangeLine::Separator));
    }

    #[test]
    fn the_green_button_menu_offers_a_side_by_side_tile_only_with_a_second_window() {
        let has = |others| arrange_lines(false, others).contains(&ArrangeLine::Item("Tile with Next Window", Arrange::SideBySide));
        assert!(!has(false));
        assert!(has(true));
    }

    #[test]
    fn a_full_screen_window_can_only_leave_full_screen() {
        assert_eq!(arrange_lines(true, true), vec![ArrangeLine::Item("Exit Full Screen", Arrange::FullScreen(false))]);
    }

    #[cfg(feature = "desktop-wayland")]
    #[test]
    fn every_tile_is_a_distinct_placement_for_the_window_server() {
        let tiles = [Tile::Fill, Tile::Center, Tile::Left, Tile::Right, Tile::Top, Tile::Bottom, Tile::TopLeft, Tile::TopRight, Tile::BottomLeft, Tile::BottomRight];
        let wires: Vec<u32> = tiles.iter().map(|t| crate::bare::window_level_client::placement_wire(placement_of(*t))).collect();
        for (i, a) in wires.iter().enumerate() {
            assert!(wires.iter().skip(i + 1).all(|b| a != b), "{:?} shares its wire value", tiles[i]);
        }
        assert_eq!(placement_of(Tile::Center), crate::bare::window_level_client::Placement::Centre, "Centre is the window server's centre key");
    }

    #[test]
    fn the_menu_bar_comes_back_only_at_the_top_edge() {
        assert!(!bar_hidden(false, None, 24.0), "an ordinary window never hides it");
        assert!(bar_hidden(true, None, 24.0), "no pointer seen yet: hidden");
        assert!(bar_hidden(true, Some(400.0), 24.0));
        assert!(!bar_hidden(true, Some(3.0), 24.0), "pointer at the top: shown");
    }

    #[test]
    fn a_frame_id_names_the_app_whose_level_it_takes() {
        // the wayland frame of an app is `win_id(app)`, and take_wayland_windows finds the app again from that id
        for (i, a) in apps().iter().enumerate() {
            assert_eq!(win_id(a.id), Some(i as u32 + 1));
            assert_eq!(win_id(a.id).and_then(|id| apps().get(id as usize - 1)).map(|b| b.id), Some(a.id));
        }
    }

    #[test]
    fn a_floating_extra_window_stays_above_the_normal_windows_of_the_desk() {
        let m = metrics();
        let mut d = Desk::default();
        d.open(&m, AppId::ExternalWindow(3, 2), (300, 200));
        d.open(&m, AppId::Files, (400, 300));
        d.sync_levels(vec![AppId::ExternalWindow(3, 2)]);
        assert_eq!(d.wins.last().map(|w| w.app), Some(AppId::ExternalWindow(3, 2)), "the floating extra window is last");
        d.sync_levels(Vec::new());
        assert_eq!(d.wins.iter().map(|w| w.app).collect::<Vec<_>>(), vec![AppId::Files, AppId::ExternalWindow(3, 2)], "a window that stops floating keeps its place in the normal band");
    }

    #[test]
    fn a_floating_window_stays_above_the_normal_ones_when_raised_or_opened() {
        let m = metrics();
        let mut d = Desk::default();
        d.open(&m, AppId::External(3), (400, 300));
        d.open(&m, AppId::Files, (400, 300));
        d.sync_levels(vec![AppId::External(3)]);
        assert_eq!(d.wins.iter().map(|w| w.app).collect::<Vec<_>>(), vec![AppId::Files, AppId::External(3)], "the floating window is last");
        assert_eq!(d.raise(0), 0, "a normal window raised goes in front of the normal ones, under the floating one");
        assert_eq!(d.wins.iter().map(|w| w.app).collect::<Vec<_>>(), vec![AppId::Files, AppId::External(3)]);
        d.open(&m, AppId::Settings, (400, 300));
        assert_eq!(d.wins.iter().map(|w| w.app).collect::<Vec<_>>(), vec![AppId::Files, AppId::Settings, AppId::External(3)], "a new normal window opens under the floating one");
        let i = d.index_of(AppId::External(3)).unwrap();
        d.raise(i);
        assert_eq!(d.wins.last().map(|w| w.app), Some(AppId::External(3)), "raising the floating window keeps it on top");
        d.sync_levels(Vec::new());
        assert_eq!(d.wins.iter().map(|w| w.app).collect::<Vec<_>>(), vec![AppId::Files, AppId::Settings, AppId::External(3)], "dropping the level keeps the order");
    }

    #[test]
    fn hits_find_lights_title_content_and_edges() {
        let m = metrics();
        let w = Win { app: AppId::Files, r: Rect { x: 100, y: 100, w: 600, h: 400 }, maximized: false, minimized: false, full: false, restore: Rect { x: 0, y: 0, w: 1, h: 1 } };
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
        let w = Win { app: AppId::Settings, r: Desk::area(&m), maximized: true, minimized: false, full: false, restore: Rect { x: 0, y: 0, w: 1, h: 1 } };
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
    use super::{dim_rect, BACKGROUND_DIM};

    /// A window that is not in front recedes by darkening its colour, never by becoming see-through: the alpha of every
    /// pixel (clear where an app leaves a material region for the window server, a faint tint, opaque) is untouched.
    #[test]
    fn dimming_a_background_window_never_changes_how_opaque_a_pixel_is() {
        let mut px = vec![0x0000_0000u32, 0x2010_2030, 0x80ff_ffff, 0xff80_80ff];
        let before: Vec<u32> = px.iter().map(|p| p >> 24).collect();
        dim_rect(&mut px, 4, 0, 0, 4, 1, BACKGROUND_DIM);
        let after: Vec<u32> = px.iter().map(|p| p >> 24).collect();
        assert_eq!(before, after);
        assert_eq!(after, vec![0x00, 0x20, 0x80, 0xff]);
    }

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

#[cfg(test)]
mod window_shadow_tests {
    use super::*;

    #[test]
    fn cached_shadow_matches_cpu_layers_across_focus_clipping_and_alpha() {
        let ui = super::super::tests::ui();
        let (width, height) = (500usize, 380usize);
        for focused in [false, true] {
            for background_color in [0xff34_5678, 0] {
                let y0 = 12usize;
                let (x, y, w, h, rad) = (-8.0, 8.0, 280.0, 180.0, ui.pt(17.0));
                let background = vec![background_color; width * (height - y0)];
                let mut expected = background.clone();
                let mut actual = background;
                let mut expected_surface = Surface { w: width, h: height, stride: width, px: &mut expected, y0 };
                let mut actual_surface = Surface { w: width, h: height, stride: width, px: &mut actual, y0 };
                draw_window_shadow_layers(&mut expected_surface, &ui, x, y, w, h, rad, focused);
                cached_window_shadow(&mut actual_surface, &ui, x, y, w, h, rad, focused);
                let max_delta = expected.iter().zip(&actual).flat_map(|(a, b)| (0..4).map(move |shift| {
                    ((((a >> (shift * 8)) & 0xff) as i16) - (((b >> (shift * 8)) & 0xff) as i16)).abs()
                })).max().unwrap_or(0);
                assert!(max_delta <= 2, "focus={focused}, background={background_color:#010x}: channel delta {max_delta}");
            }
        }
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

#[cfg(test)]
mod keyboard_lift_tests {
    use super::{kb_lift, Rect};

    fn rect(y: i32, h: i32) -> Rect {
        Rect { x: 0, y, w: 600, h }
    }

    #[test]
    fn a_window_that_clears_the_keyboard_does_not_move() {
        assert_eq!(kb_lift(rect(200, 600), 1200, 40), 0);
        // the bottom edge exactly on the keyboard's top is clear too
        assert_eq!(kb_lift(rect(200, 1000), 1200, 40), 0);
    }

    #[test]
    fn a_window_under_the_keyboard_moves_up_by_the_overlap() {
        assert_eq!(kb_lift(rect(900, 600), 1200, 40), 300);
    }

    #[test]
    fn the_lift_stops_at_the_menu_bar() {
        // the overlap is 900 but the top is only 60 px below the menu bar
        assert_eq!(kb_lift(rect(100, 1800), 1000, 40), 60);
        // a window already at the menu bar cannot move
        assert_eq!(kb_lift(rect(40, 1800), 1000, 40), 0);
    }
}

/// The window server's frame of a toplevel moves the shell's rectangle (E2E N1).
#[cfg(all(test, feature = "desktop-wayland"))]
mod server_frame_tests {
    use super::{frame_follows, Rect};

    #[test]
    fn a_new_position_is_taken_and_reported_as_a_change() {
        let mut r = Rect { x: 852, y: 339, w: 1184, h: 810 };
        assert!(frame_follows(&mut r, 800, 360));
        assert_eq!(r, Rect { x: 800, y: 360, w: 1184, h: 810 }, "the size is not touched");
    }

    #[test]
    fn the_same_position_is_not_a_change() {
        let mut r = Rect { x: 800, y: 360, w: 1184, h: 810 };
        assert!(!frame_follows(&mut r, 800, 360));
        assert_eq!(r, Rect { x: 800, y: 360, w: 1184, h: 810 });
    }

    #[test]
    fn a_window_dragged_partly_off_screen_keeps_the_reported_negative_position() {
        let mut r = Rect { x: 100, y: 100, w: 600, h: 400 };
        assert!(frame_follows(&mut r, -80, 28));
        assert_eq!((r.x, r.y), (-80, 28));
    }
}

#[cfg(test)]
mod cmdtab_material_tests {
    use super::*;

    /// The Cmd+Tab capsule is the Search material (a translucent tint over the presenter's blur), not the menus'
    /// nearly opaque flat plate: the wallpaper must show through it.
    #[test]
    fn the_cmd_tab_capsule_is_translucent_glass_not_a_flat_plate() {
        let mut sh = super::super::tests::desktop_shell();
        sh.screen = Screen::Home;
        sh.anim = None;
        sh.desk_open(AppId::Settings, 0.0);
        sh.desk_open(AppId::Files, 1.0);
        sh.switcher_cycle(false);
        let (w, h) = (sh.w as usize, sh.h as usize);
        let mut buf = vec![0u32; w * h];
        sh.draw_cmdtab(&mut Surface { w, h, stride: w, px: &mut buf, y0: 0 });
        let l = sh.cmdtab_layout(w as f32, h as f32).expect("the strip is open");
        let alpha = buf[(l.y + l.h / 2.0) as usize * w + (l.x + l.pad / 2.0) as usize] >> 24;
        assert!(alpha > 100 && alpha < 200, "the capsule fill is translucent (alpha {alpha}), not the menus' 240");
    }
}
