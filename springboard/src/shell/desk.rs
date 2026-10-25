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
/// how long after a wheel event the press the Wayland pointer emulates for it may come (ms)
const WHEEL_DRAG_MS: f64 = 250.0;
/// Smallest window body, in pt.
const MIN_W_PT: f32 = 320.0;
const MIN_H_PT: f32 = 220.0;
/// Dock metrics, in pt.
pub(super) const DOCK_PAD_PT: f32 = 10.0;
const DOCK_GAP_PT: f32 = 10.0;
/// Small screen-edge inset calibrated to the desktop reference, independent of plate depth.
pub(super) const DOCK_MARGIN_PT: f32 = 4.0;
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

/// Waits for (reaps) the started apps whose process has exited and forgets them. A quit app nobody waited for stays a
/// zombie, and a zombie still counts in its cgroup's `pids.current`, so the CPU and RAM menus listed quit apps as
/// running (E2E N14, W5-13). Running children are kept.
#[cfg_attr(not(feature = "desktop-wayland"), allow(dead_code))]
fn reap_exited(children: &mut std::collections::HashMap<String, std::process::Child>) {
    children.retain(|_, c| matches!(c.try_wait(), Ok(None)));
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

/// The position a window's toplevel asks to map at: where the window is (`x`, `y`) when the window server placed it
/// before (the toplevel was destroyed while the window was hidden and comes back), none for a window never placed, which
/// the window server cascades as a new window.
#[cfg_attr(not(feature = "desktop-wayland"), allow(dead_code))]
pub fn origin_hint(placed: &std::collections::HashSet<AppId>, app: AppId, x: i32, y: i32) -> Option<(i32, i32)> {
    placed.contains(&app).then_some((x, y))
}

/// The top-left corner (px) of a window of `size` cascaded from `from`, another window of the same app
/// (`window_flags::CASCADE`, AppKit's `cascadeTopLeftFromPoint:`), kept inside the work area `a`. From the main window
/// (`from_main`) it goes beside the main window's trailing edge when it fits there, and otherwise over the main window's
/// trailing side one `step` down, so the leading columns (a sidebar, a message list) stay in view. A window about as
/// wide as the main window (less than a `step` narrower) has no trailing side to sit on: that put a second document
/// straight over the first, one step down (E2E11 5), so it cascades one `step` down and right instead. From another
/// cascaded window it goes one `step` down and right, as each new document window does.
pub fn cascade_origin(a: Rect, from: Rect, from_main: bool, size: (i32, i32), step: i32) -> (i32, i32) {
    let (w, h) = size;
    let (x, y) = if !from_main || from.w - w < step {
        (from.x + step, from.y + step)
    } else if from.x + from.w + w <= a.x + a.w {
        (from.x + from.w, from.y)
    } else {
        (from.x + from.w - w, from.y + step)
    };
    (x.clamp(a.x, (a.x + a.w - w).max(a.x)), y.clamp(a.y, (a.y + a.h - h).max(a.y)))
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
    /// an extra window whose app asked for full-size content (`window_flags::FULL_SIZE`, kept in its app host's window
    /// table): it has no shell title row and draws its own, as a second document window of an editor does (E2E9 4)
    pub full_size: bool,
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
    /// `app` is the window that was pressed: the drag follows it, not whichever window is in front now (a focus
    /// change during the drag resized Settings to another window's frame, E2E-12)
    Resize { edges: u8, start: Rect, x: f32, y: f32, app: AppId },
    /// a press on a traffic light of `app`'s window that acts on release (by app, not stack index: a focus change between
    /// press and release reorders the stack, W3-1)
    Light(AppId, Light),
    /// a press on Dock slot `from`: `entry` is the dock list entry and `rect` the icon as hit-tested at the press, so the
    /// release opens the icon that was pressed even when the row re-centres in between (W2-23)
    DockDrag{from:usize,entry:usize,rect:Rect,x:f32,y:f32,at_x:f32,at_y:f32,moved:bool},
    /// the touch belongs to this window's app until it lifts
    Content(AppId),
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

/// How long (ms) the app the shell opened stays the front app against the window server's own focus changes
/// (`Desk::front_intent`): long enough for a fresh process to draw its first frame and for its toplevel to map, short
/// enough that a window the window server focuses later (a dialog, an app's own activation) is followed again.
pub(super) const FRONT_INTENT_MS: f64 = 3000.0;

/// The app the shell is bringing to the front (Spotlight, the Apps grid, the Dock, `open-app`): opened at `since`
/// (shell ms). While it stands (`Shell::desk_front_intent`) a focus the window server gives another window on its own
/// (a hand-back as Spotlight's exclusive keyboard goes, a keyboard enter, an activating configure) is not followed, and
/// the shell asks the window server again to activate the app's window once it is mapped (E2E6 Spotlight).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct FrontIntent {
    pub app: AppId,
    pub since: f64,
}

#[derive(Default)]
pub struct Desk {
    /// the app the shell is bringing to the front, until it is in front for good (`FrontIntent`)
    pub(super) front_intent: Option<FrontIntent>,
    /// the window server's toplevel list changed since the front intent last looked at it (`Shell::desk_intent_tick`)
    #[cfg(feature = "desktop-wayland")]
    pub(super) intent_check: bool,
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
    /// the front app's windows as Cmd+` found them (front first) and the one it brought forward last: consecutive
    /// presses walk this order, so raising one window at a time still visits every window (W2-25)
    pub(super) window_cycle: Option<(Vec<AppId>, usize)>,
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
    /// the windows Hide put away (Cmd+H, Hide Others): they are `minimized` but not in the Dock's minimized section, and
    /// they come back with Show All or when their app is activated (`raise`)
    pub(super) hidden: Vec<AppId>,
    /// the window server's toplevels (native and Linux apps) Hide minimized, so Show All can bring them back
    pub(super) hidden_foreign: Vec<u32>,
    /// the windows Show Desktop put aside: `minimized`, but not in the Dock's minimized section; Show Desktop again or
    /// Bring All to Front brings them back (N13)
    pub(super) put_aside: Vec<AppId>,
    /// the window server's toplevels (native and Linux apps) Show Desktop minimized, in the order they come back (the
    /// active one last, so it is in front again); Show Desktop again or Bring All to Front activates them (R3)
    #[cfg(feature = "desktop-wayland")]
    pub(super) foreign_aside: Vec<u32>,
    /// the ids in `foreign_aside` the window server has reported minimized since they were put aside: a report of one of
    /// these shown again means the person (the Dock) or the app brought it back, so it is no longer aside. One not yet
    /// reported minimized is kept, since the snapshot may predate the minimize (`prune_foreign_aside`)
    #[cfg(feature = "desktop-wayland")]
    foreign_aside_seen: Vec<u32>,
    /// the app (window server app id) whose toplevel was active when Show Desktop put the toplevels aside: the active
    /// app Bring All to Front acts on while no toplevel is active
    #[cfg(feature = "desktop-wayland")]
    foreign_aside_front: Option<String>,
    /// the window server's toplevels to minimize this round (`take_native_minimizes`)
    #[cfg(feature = "desktop-wayland")]
    minimize: Vec<u32>,
    /// where each app's window was when it was last closed: a window opened again comes back there (W2-26), as an app's
    /// autosaved frame does, instead of at the window server's next cascade position
    pub(super) closed_origins:std::collections::HashMap<AppId,(i32,i32)>,
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
    /// the open stack (`Menu::Stack`): the items it lists, newest first, and the Dock icon it opened from (px)
    #[cfg_attr(not(feature = "desktop-wayland"), allow(dead_code))]
    pub(super) stack_items:Vec<super::dock_stack::Item>,
    #[cfg_attr(not(feature = "desktop-wayland"), allow(dead_code))]
    pub(super) stack_icon:Option<(f32,f32,f32,f32)>,
    /// the open stack's type-select prefix (`dock_stack::type_select`), and its items' pictures, drawn off the frame by
    /// the thumbnail providers and kept on the shared disk cache (`search_thumbs`)
    #[cfg_attr(not(feature = "desktop-wayland"), allow(dead_code))]
    pub(super) stack_typed:bifrost_ui::menu::Menu,
    #[cfg_attr(not(feature = "desktop-wayland"), allow(dead_code))]
    pub(super) stack_thumbs:super::search_thumbs::FileThumbs,
    pub(super) dock_menu_target:Option<usize>,
    pub(super) dock_menu_anchor:(f32,f32),
    grab: Option<Grab>,
    /// when the last wheel event came (ms): the press the Wayland pointer emulates right after it (`bare::wayland`)
    /// is the wheel, never a click on a title bar, an edge, a traffic light or a Dock icon (`Shell::desk_wheel`)
    wheel_at: Option<f64>,
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
    /// the apps whose window the window server has placed (it reported a frame): when the shell hides such a window
    /// (Overview, a minimize) its toplevel is destroyed, and the new toplevel must map where the window was, not at the
    /// window server's cascade
    pub(super) placed: std::collections::HashSet<AppId>,
    /// the windows the shell moved where their app asked (`WindowMove`) since their last frame went out: the next frame
    /// asks the window server to move the mapped toplevel there (`set_position`), once (E2E11 3)
    pub(super) moved: std::collections::HashSet<AppId>,
    /// the input shape of each borderless window (window px, `alpha_shape` of its last frame): the window server gives
    /// it the pointer only there, and `pick` hits it only there
    pub(super) shapes: std::collections::HashMap<AppId, std::sync::Arc<[[i32; 4]]>>,
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
    /// windows brought forward together (Bring All to Front, a hidden app shown, the Apps grid closing): the window
    /// server is told to stack them in the shell's order over other clients' windows (`Shell::take_wayland_restack`)
    pub(super) restack: Vec<AppId>,
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
    pub dock_left: i32,
    pub dock_right: i32,
    /// px per point
    pub pt: f32,
}

/// A full-size-content window (`NativeWindowStyleMaskFullSizeContentView`): no title bar, the app draws to the top edge
/// and the traffic lights sit inside its sidebar. Settings is one; an installed app asks with `window = "full-size"`
/// in its manifest.
/// Whether `app`'s window is borderless (`window = "borderless"`): a full-size window without shell chrome (no traffic
/// lights, title band, rim, shadow, dim or corner clip) whose transparent pixels let the pointer through
/// (`alpha_shape`), for an app that draws free-floating pieces such as Sticky Notes' notes (W2-22).
pub fn borderless(app: AppId) -> bool {
    match app {
        AppId::External(slot) => apphost::installed().get(slot as usize).is_some_and(|a| a.manifest.borderless()),
        _ => false,
    }
}

/// The parts of a window of `width` x `height` px that its pixels cover (alpha above zero), as rects in window px:
/// a run of covered pixels per row, and rows with the same runs merged into one rect. A borderless window takes the
/// pointer only there (`WindowFrame::input`). More than `MAX_SHAPE_RECTS` rects (a noisy picture) falls back to one
/// bounding box per run of rows, so the region stays small.
pub fn alpha_shape(px: &[u32], width: usize, height: usize) -> Vec<[i32; 4]> {
    let mut out: Vec<[i32; 4]> = Vec::new();
    // the rects of the row band still growing: (x0, x1) runs and the row they started on
    let mut open: Vec<(usize, usize)> = Vec::new();
    let mut start = 0usize;
    let flush = |out: &mut Vec<[i32; 4]>, runs: &[(usize, usize)], y0: usize, y1: usize| {
        for &(a, b) in runs { out.push([a as i32, y0 as i32, (b - a) as i32, (y1 - y0) as i32]); }
    };
    for y in 0..=height {
        let runs: Vec<(usize, usize)> = if y == height { Vec::new() } else {
            let row = &px[y * width..(y + 1) * width];
            let mut v = Vec::new();
            let mut x = 0;
            while x < width {
                if row[x] >> 24 == 0 { x += 1; continue; }
                let a = x;
                while x < width && row[x] >> 24 != 0 { x += 1; }
                v.push((a, x));
            }
            v
        };
        if runs != open {
            flush(&mut out, &open, start, y);
            open = runs;
            start = y;
        }
    }
    if out.len() > MAX_SHAPE_RECTS {
        // one box per band of rows that has any pixel
        let mut boxes: Vec<[i32; 4]> = Vec::new();
        for r in out {
            match boxes.last_mut() {
                Some(b) if b[1] + b[3] == r[1] || (b[1] == r[1]) => {
                    let (x0, x1) = (b[0].min(r[0]), (b[0] + b[2]).max(r[0] + r[2]));
                    let (y0, y1) = (b[1].min(r[1]), (b[1] + b[3]).max(r[1] + r[3]));
                    *b = [x0, y0, x1 - x0, y1 - y0];
                }
                _ => boxes.push(r),
            }
        }
        return boxes;
    }
    out
}

/// The most rects a borderless window's input region holds before `alpha_shape` merges them into boxes.
pub const MAX_SHAPE_RECTS: usize = 512;

/// Whether window-local (`x`, `y`) is inside one of `shape`'s rects.
pub fn shape_contains(shape: &[[i32; 4]], x: f32, y: f32) -> bool {
    shape.iter().any(|r| x >= r[0] as f32 && x < (r[0] + r[2]) as f32 && y >= r[1] as f32 && y < (r[1] + r[3]) as f32)
}

/// Whether the Photos window is full-size: on the desktop form factor, read from the process's form factor (fixed for
/// the life of the shell) rather than from Photos' own state, so it holds before Photos is first opened and never goes
/// stale. Tests switch Photos' layout on and off themselves (`Photos::set_desktop`), so they read that.
fn photos_full_size() -> bool {
    #[cfg(test)]
    {
        super::photos::sidebar::sidebar_layout()
    }
    #[cfg(not(test))]
    {
        bifrost_ui::form_factor::FormFactor::current().is_desktop()
    }
}

pub fn full_size(app: AppId) -> bool {
    match app {
        // the built-in sidebar apps: the sidebar runs to the top edge under the traffic lights (no title bar)
        AppId::Settings | AppId::AppStore => true,
        // Photos on the desktop form factor: its source list runs to the top edge (the tablet keeps the title bar)
        AppId::Photos => photos_full_size(),
        AppId::External(slot) => apphost::installed().get(slot as usize).is_some_and(|a| a.manifest.full_size_content()),
        // an extra window keeps its title bar unless its app asked for full-size content (`window_flags::FULL_SIZE`):
        // that is per window (`Win::full_size`, from the app host's window table), so ask the window (`Win::is_full_size`)
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
    // a borderless window has no title band: every press is the app's (a sticky note moves itself)
    if borderless(w.app) {
        return (0, Vec::new());
    }
    // Settings is a built-in full-size window: it reports its toolbar controls the way an SDK app publishes them, so its
    // empty toolbar drags and the back/forward pill (and the search field) take the press (W3-3)
    // so is the App Store: its sidebar search field and the pane's back control take the press
    let own = match w.app {
        AppId::Settings => Some(settings_toolbar_controls(w, m)),
        AppId::AppStore => Some(super::store::toolbar_controls(w.r.w, m.pt, super::store::back_control_pt())),
        // and Photos: the toolbar's title, search capsule and buttons it last drew
        AppId::Photos if w.is_full_size() => Some(super::photos::sidebar::toolbar_controls()),
        _ => None,
    };
    zone_of(w, m, own.as_deref().unwrap_or(controls), w.is_full_size())
}

/// The Settings window's toolbar controls as semantic nodes in window points, laid out by `prefs_layout::Frame` as the
/// window draws them: the two halves of the back/forward pill over the detail pane and the sidebar search field.
pub fn settings_toolbar_controls(w: &Win, m: &Metrics) -> Vec<bifrost_proto::Node> {
    use bifrost_proto::{Node, Role};
    let pt = m.pt.max(0.01);
    let f = bifrost_ui::prefs_layout::Frame::new(w.r.w as f32 / pt, w.r.h as f32 / pt);
    let p = f.toolbar_pill;
    let node = |id, role, label: &str, bounds| Node { id, role, label: label.into(), value: String::new(), bounds, enabled: true };
    vec![
        node(1, Role::Button, "Back", bifrost_ui::Rect { w: p.w / 2.0, ..p }),
        node(2, Role::Button, "Forward", bifrost_ui::Rect { x: p.x + p.w / 2.0, w: p.w / 2.0, ..p }),
        node(3, Role::TextField, "Search", f.search),
    ]
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
    let pt = m.pt;
    let l2 = w.light_rect(m, 2);
    let lights_w = l2.x + l2.w - w.r.x + (6.0 * pt) as i32;
    // a titled window: the traffic lights (and a margin of 6 pt past the last) take the press, the rest of the title bar moves
    if !unified {
        return (m.title, vec![[0, 0, lights_w, m.title]]);
    }
    let band = (UNIFIED_BAR_PT * pt).round() as i32;
    let mut out = vec![[0, 0, lights_w, band]];
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
        AppId::External(slot) => {
            let manifest = apphost::installed().get(slot as usize).map(|a| a.manifest.size_limits()).unwrap_or_default();
            match content_limits(slot) {
                Some((min, max)) => manifest.with_content_limits(min, max),
                None => manifest,
            }
        }
        // Settings on the desktop is always the sidebar + detail layout: narrower than the split minimum it fell back to
        // the tablet root list and stayed there (E2E-12: a 400x549 window)
        // (one point over it, so the rounding to px never lands under the split test)
        AppId::Settings => apphost::SizeLimits { min: Some((super::settings::SPLIT_MIN_PT + 1.0, MIN_H_PT)), ..Default::default() },
        _ => apphost::SizeLimits::default(),
    }
}

/// The content limits each running app set for its main window (`ToShell::SetContentLimits`): (slot, min, max) in
/// points. They last until the app exits (`forget_content_limits`).
static CONTENT_LIMITS: std::sync::Mutex<Vec<(u8, Option<(f32, f32)>, Option<(f32, f32)>)>> = std::sync::Mutex::new(Vec::new());

/// Keeps the main-window content limits app `slot` asked for; both `None` drops them (the manifest's apply again).
pub(super) fn set_content_limits(slot: u8, min: Option<(f32, f32)>, max: Option<(f32, f32)>) {
    let mut all = CONTENT_LIMITS.lock().unwrap_or_else(|e| e.into_inner());
    all.retain(|(s, _, _)| *s != slot);
    if min.is_some() || max.is_some() {
        all.push((slot, min, max));
    }
}

/// Drops the content limits of app `slot` (it exited).
pub(super) fn forget_content_limits(slot: u8) {
    set_content_limits(slot, None, None);
}

fn content_limits(slot: u8) -> Option<(Option<(f32, f32)>, Option<(f32, f32)>)> {
    CONTENT_LIMITS.lock().unwrap_or_else(|e| e.into_inner()).iter().find(|(s, _, _)| *s == slot).map(|&(_, min, max)| (min, max))
}

/// A content size in points as an outer window size in px (adds the title bar of a titled window).
pub fn outer_px(app: AppId, m: &Metrics, (w, h): (f32, f32)) -> (i32, i32) {
    ((w * m.pt).round() as i32, (h * m.pt).round() as i32 + title_of(app, m))
}

/// The outer px size of an extra window from the size its app asked for in points: never under the window minimum, so
/// an app cannot open a window with no width (a zero-width toplevel is a protocol error for the window server). A
/// `full_size` window (`window_flags::FULL_SIZE`) has no shell title bar to add.
fn extra_window_px(app: AppId, own_title_row: bool, (w, h): (f32, f32), k: f32, m: &Metrics) -> (i32, i32) {
    let (min_w, min_h, _, _) = size_range(app, m);
    let title = if own_title_row { 0 } else { title_of(app, m) };
    (((w * k) as i32).max(min_w), ((h * k) as i32 + title).max(min_h - title_of(app, m) + title))
}

/// The outer size range (px) a window of `app` may be resized within: `(min_w, min_h, max_w, max_h)`.
fn size_range(app: AppId, m: &Metrics) -> (i32, i32, i32, i32) {
    let lim = size_limits(app);
    let (min_w, min_h) = lim.min.map(|s| outer_px(app, m, s)).unwrap_or((m.min_w, m.min_h + title_of(app, m)));
    // a minimum is never larger than the screen below the menu bar (Settings' split width on a small display)
    let (min_w, min_h) = (min_w.min(m.w.max(1)), min_h.min((m.h - m.menubar).max(1)));
    let (max_w, max_h) = lim.max.map(|s| outer_px(app, m, s)).unwrap_or((i32::MAX, i32::MAX));
    (min_w, min_h, max_w.max(min_w), max_h.max(min_h))
}

/// Whether a window closed at `size` (outer px) comes back at that size in the work area `a`. A frame that spans the
/// work area's height or width (within 3 %) is a tile or the Fill frame, which the window server sizes, not a size the
/// person chose by hand: the next window opens at its default size, never filling the screen (W4-3, W4-8).
pub(super) fn remembered_size(a: Rect, (w, h): (i32, i32)) -> bool {
    w * 100 < a.w * 97 && h * 100 < a.h * 97
}

/// `size` (outer px) held to the range `app`'s window may be resized within (`size_range`).
fn clamp_size(app: AppId, m: &Metrics, (w, h): (i32, i32)) -> (i32, i32) {
    let (min_w, min_h, max_w, max_h) = size_range(app, m);
    (w.clamp(min_w, max_w), h.clamp(min_h, max_h))
}

pub fn title_of(app: AppId, m: &Metrics) -> i32 {
    if full_size(app) { 0 } else { m.title }
}

impl Win {
    /// True for a full-size window: its app's content runs to the top edge under the traffic lights (no shell title
    /// bar). An app's kind decides it (`full_size`), or for an extra window the flag its app opened it with.
    pub fn is_full_size(&self) -> bool {
        self.full_size || full_size(self.app)
    }

    /// Height of this window's shell title bar in px (0 for a full-size window).
    pub fn title_h(&self, m: &Metrics) -> i32 {
        if self.is_full_size() { 0 } else { m.title }
    }

    pub fn body(&self, m: &Metrics) -> Rect {
        // a full-screen window has no title bar: the app is the whole window
        let t = if self.full { 0 } else { self.title_h(m) };
        Rect { x: self.r.x, y: self.r.y + t, w: self.r.w, h: (self.r.h - t).max(1) }
    }

    pub(super) fn light_rect(&self, m: &Metrics, i: usize) -> Rect {
        if self.is_full_size() {
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
        // a full-screen window is the app alone: no resize edges, no title bar, no traffic lights; so is a borderless one
        if self.full || borderless(self.app) {
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
        if self.is_full_size() {
            let pt = m.pt;
            for (i, l) in [Light::Close, Light::Minimize, Light::Zoom].into_iter().enumerate() {
                let lr = self.light_rect(m, i);
                let pad = (4.0 * pt) as i32;
                if (Rect { x: lr.x - pad, y: lr.y - pad, w: lr.w + 2 * pad, h: lr.h + 2 * pad }).contains(x, y) {
                    return Some(Hit::Light(l));
                }
            }
            if !matches!(self.app, AppId::Settings | AppId::AppStore | AppId::Photos) {
                // a full-size app: its unified toolbar band is the title bar; controls in it are sorted out on the press
                return Some(if y < self.r.y as f32 + UNIFIED_BAR_PT * pt { Hit::Title } else { Hit::Content });
            }
            // Settings, the App Store and Photos: the same band and holes as the window server's drag zone (`drag_zone`), so
            // the empty toolbar moves the window and the pill, back control and search field get the press
            let (band, holes) = drag_zone(self, m, &[]);
            let (lx, ly) = (x as i32 - self.r.x, y as i32 - self.r.y);
            let on_control = holes.iter().skip(1).any(|r| lx >= r[0] && lx < r[0] + r[2] && ly >= r[1] && ly < r[1] + r[3]);
            return Some(if ly < band && !on_control { Hit::Title } else { Hit::Content });
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

    /// Waits for the native apps that have quit (see `reap_exited`).
    #[cfg(feature = "desktop-wayland")]
    pub(super) fn reap_native_children(&mut self) {
        reap_exited(&mut self.native_children);
    }

    pub(super) fn grab_idle(&self)->bool{self.grab.is_none()}
    /// Whether the Dock keeps magnifying: with no grab, or while a Dock icon is pressed but not dragged, so the row does
    /// not jump back to rest under the pointer between the press and the release (W2-23).
    pub(super) fn dock_grab_magnifies(&self)->bool{matches!(self.grab,None|Some(Grab::DockDrag{moved:false,..}))}
    pub fn new() -> Self {
        Desk {dock_visible:true,..Default::default()}
    }

    pub fn index_of(&self, app: AppId) -> Option<usize> {
        self.wins.iter().position(|w| w.app == app)
    }

    pub fn front(&self) -> Option<&Win> {
        self.wins.iter().rev().find(|w| !w.minimized)
    }

    /// Brings window `i` to the front (and shows it); returns its new index. A hidden app comes back whole: raising one
    /// of its windows (the Dock, Cmd+Tab, the Window menu) shows every window Hide put away, as activating a hidden app does.
    pub fn raise(&mut self, i: usize) -> usize {
        let app = self.wins[i].app;
        let parent = app.parent();
        let i = if self.hidden.iter().any(|a| a.parent() == parent) {
            self.show_hidden_app(parent);
            self.index_of(app).unwrap_or(i)
        } else {
            i
        };
        let mut w = self.wins.remove(i);
        w.minimized = false;
        let at = self.band_top(w.app);
        self.wins.insert(at, w);
        at
    }

    /// A hidden app comes back whole (E2E N2): every window Hide put away is shown, and the app's shown windows come
    /// forward over the other apps' in the order they had among themselves, so the one in front when the app was hidden
    /// (a Mail viewer over its main window) is in front again. Returns the index of the app's front window.
    fn show_hidden_app(&mut self, parent: AppId) -> Option<usize> {
        self.unhide(|a| a.parent() == parent);
        let mine: Vec<AppId> = self.wins.iter().filter(|w| !w.minimized && w.app.parent() == parent).map(|w| w.app).collect();
        let mut front = None;
        for &app in &mine {
            if let Some(i) = self.index_of(app) {
                let mut w = self.wins.remove(i);
                w.minimized = false;
                let at = self.band_top(w.app);
                self.wins.insert(at, w);
            }
        }
        // their toplevels map again, each when it has drawn: the window server is told their order, or the ones that map
        // before the front one go under the window it activated meanwhile (E2E5 3: Mail's main window under App Store)
        self.note_restack(&mine);
        // the band order may put a floating window of the app above a normal one: the front is the topmost of the app
        for (i, w) in self.wins.iter().enumerate() {
            if !w.minimized && w.app.parent() == parent {
                front = Some(i);
            }
        }
        front
    }

    /// Hide (Cmd+H, the App menu's Hide): every shown window of the app `parent` goes away at once, with no minimize
    /// effect and no Dock thumbnail; the app stays running and comes back with `unhide` or when one of its windows is raised.
    /// Returns whether a window was hidden.
    pub(super) fn hide_app(&mut self, parent: AppId) -> bool {
        self.hide_where(|a| a.parent() == parent)
    }

    /// Hide Others (Option+Cmd+H): every app but `keep` (a parent id; None hides every app's windows) is hidden.
    pub(super) fn hide_others(&mut self, keep: Option<AppId>) -> bool {
        self.hide_where(|a| Some(a.parent()) != keep)
    }

    fn hide_where(&mut self, pick: impl Fn(AppId) -> bool) -> bool {
        let mut any = false;
        for w in self.wins.iter_mut().filter(|w| !w.minimized && pick(w.app)) {
            w.minimized = true;
            if !self.hidden.contains(&w.app) {
                self.hidden.push(w.app);
            }
            any = true;
        }
        any
    }

    /// Window > Bring All to Front: every shown window of the app `parent` comes forward over the other apps' windows,
    /// keeping the order among them (the one in front stays in front). Minimized and hidden windows stay where they are,
    /// as on Golden Gate, where the command never takes a window out of the Dock (W3-6).
    pub(super) fn bring_all_to_front(&mut self, parent: AppId) {
        let mine: Vec<AppId> = self.wins.iter().filter(|w| !w.minimized && w.app.parent() == parent).map(|w| w.app).collect();
        for &app in &mine {
            if let Some(i) = self.index_of(app) {
                self.raise(i);
            }
        }
        // the window server raises only the front window it is asked to activate: it is told the app's whole set (E2E5 3)
        self.note_restack(&mine);
    }

    /// Windows brought forward together: the window server stacks them in the shell's order over other clients'
    /// windows with the next frame (`Shell::take_wayland_restack`), mapped already or still to map.
    pub(super) fn note_restack(&mut self, apps: &[AppId]) {
        for &app in apps {
            if !self.restack.contains(&app) {
                self.restack.push(app);
            }
        }
    }

    /// Show Desktop: every shown window is put aside (minimized without a Dock slot) and remembered, so the desktop
    /// shows; returns whether a window went. Windows put aside earlier stay aside. The window server's toplevels go
    /// too (R3): each shown one is minimized and remembered; one already minimized (by the person or Hide) stays as it is.
    pub(super) fn put_windows_aside(&mut self) -> bool {
        let mut any = false;
        for w in self.wins.iter_mut().filter(|w| !w.minimized) {
            w.minimized = true;
            if !self.put_aside.contains(&w.app) {
                self.put_aside.push(w.app);
            }
            any = true;
        }
        #[cfg(feature = "desktop-wayland")]
        {
            let mut shown: Vec<(bool, u32)> = self.foreign_shown().map(|w| (w.active, w.id)).collect();
            if !shown.is_empty() {
                let front = self.foreign_shown().find(|w| w.active).map(|w| w.app_id.clone());
                self.foreign_aside_front = front;
            }
            // stable: the active window is put aside last, so it is activated last and comes back in front
            shown.sort_by_key(|(active, _)| *active);
            for (_, id) in shown {
                self.foreign_aside.push(id);
                self.foreign_aside_seen.retain(|s| *s != id);
                self.minimize.push(id);
                any = true;
            }
        }
        any
    }

    /// The window server's toplevels that are on screen: not minimized (one put aside counts as gone at once, before
    /// the window server reports it minimized).
    #[cfg(feature = "desktop-wayland")]
    pub(super) fn foreign_shown(&self) -> impl Iterator<Item = &crate::bare::wayland::ForeignWindow> {
        self.foreign_all().filter(move |w| !w.minimized && !self.foreign_aside.contains(&w.id))
    }

    /// Every toplevel of the window server the shell knows: the native and direct apps' windows, then the Linux apps'.
    #[cfg(feature = "desktop-wayland")]
    fn foreign_all(&self) -> impl Iterator<Item = &crate::bare::wayland::ForeignWindow> {
        self.native_windows.iter().chain(self.linux_apps.iter().flat_map(|a| a.windows.iter()))
    }

    /// The window server reported its toplevels `windows`: a toplevel put aside that has closed, or that is shown again
    /// after being reported minimized (the Dock brought it back, or the app activated itself), is no longer aside, so
    /// the next Show Desktop hides it instead of taking the bring-back path.
    #[cfg(feature = "desktop-wayland")]
    pub(super) fn prune_foreign_aside(&mut self, windows: &[crate::bare::wayland::ForeignWindow]) {
        let seen = &mut self.foreign_aside_seen;
        self.foreign_aside.retain(|id| match windows.iter().find(|w| w.id == *id) {
            None => false,
            Some(w) if w.minimized => {
                if !seen.contains(id) {
                    seen.push(*id);
                }
                true
            }
            Some(_) => !seen.contains(id),
        });
        let aside = &self.foreign_aside;
        self.foreign_aside_seen.retain(|id| aside.contains(id));
    }

    /// The shell activates toplevel `id` (the Dock, Overview): it is shown again, so no longer aside.
    #[cfg(feature = "desktop-wayland")]
    pub(super) fn foreign_brought_back(&mut self, id: u32) {
        self.foreign_aside.retain(|a| *a != id);
        self.foreign_aside_seen.retain(|a| *a != id);
    }

    /// Whether any window is on screen: a shown built-in window or a shown toplevel of the window server.
    pub(super) fn any_shown(&self) -> bool {
        #[cfg(feature = "desktop-wayland")]
        if self.foreign_shown().next().is_some() {
            return true;
        }
        self.wins.iter().any(|w| !w.minimized)
    }

    /// The windows Show Desktop put aside come back where they were in the stack; returns whether one came back. A
    /// window minimized or closed since is not brought back. Show Desktop's toggle and Overview bring back everything.
    pub(super) fn bring_back_put_aside(&mut self) -> bool {
        #[allow(unused_mut)]
        let mut any = self.bring_back_shell_aside();
        #[cfg(feature = "desktop-wayland")]
        {
            any |= self.bring_back_foreign_aside(|_| true);
        }
        any
    }

    /// Bring All to Front's share of what Show Desktop put aside: the shell's windows, and of the window server's
    /// toplevels only the active app's (Window > Bring All to Front acts on the active app). The active app is the one
    /// whose toplevel is active, or, when none is and `shell_app_front` is false, the one that was active when Show
    /// Desktop put the toplevels aside. Returns whether a window came back.
    pub(super) fn bring_back_front_app_aside(&mut self, shell_app_front: bool) -> bool {
        #[allow(unused_mut)]
        let mut any = self.bring_back_shell_aside();
        #[cfg(feature = "desktop-wayland")]
        {
            let front = self
                .foreign_all()
                .find(|w| w.active)
                .map(|w| w.app_id.clone())
                .or_else(|| if shell_app_front { None } else { self.foreign_aside_front.clone() });
            if let Some(front) = front {
                any |= self.bring_back_foreign_aside(|w| w.app_id == front);
            }
        }
        let _ = shell_app_front;
        any
    }

    /// The shell's windows Show Desktop put aside come back.
    fn bring_back_shell_aside(&mut self) -> bool {
        let back = std::mem::take(&mut self.put_aside);
        let mut any = false;
        for w in self.wins.iter_mut().filter(|w| w.minimized && back.contains(&w.app)) {
            w.minimized = false;
            any = true;
        }
        any
    }

    /// The window server's toplevels put aside that `pick` chooses come back by activation (it unminimizes and raises
    /// them), in the order they were put aside; one closed since is dropped, the others stay aside.
    #[cfg(feature = "desktop-wayland")]
    fn bring_back_foreign_aside(&mut self, pick: impl Fn(&crate::bare::wayland::ForeignWindow) -> bool) -> bool {
        let aside = std::mem::take(&mut self.foreign_aside);
        let (mut back, mut keep) = (Vec::new(), Vec::new());
        for id in aside {
            match self.foreign_all().find(|w| w.id == id) {
                None => {}
                Some(w) if pick(w) => back.push(id),
                Some(_) => keep.push(id),
            }
        }
        self.foreign_aside = keep;
        let aside = &self.foreign_aside;
        self.foreign_aside_seen.retain(|id| aside.contains(id));
        let any = !back.is_empty();
        self.activate.extend(back);
        any
    }

    /// Whether the window of `app` is aside because of Show Desktop (not minimized to the Dock by the person).
    pub(super) fn is_put_aside(&self, app: AppId) -> bool {
        self.put_aside.contains(&app) && self.index_of(app).is_some_and(|i| self.wins[i].minimized)
    }

    /// Show All: every window Hide put away is shown again, where it was in the stack.
    pub(super) fn unhide_all(&mut self) {
        self.unhide(|_| true);
    }

    fn unhide(&mut self, pick: impl Fn(AppId) -> bool) {
        let (back, keep): (Vec<AppId>, Vec<AppId>) = self.hidden.iter().partition(|a| pick(**a));
        self.hidden = keep;
        for w in self.wins.iter_mut().filter(|w| back.contains(&w.app)) {
            w.minimized = false;
        }
    }

    /// Whether the window of `app` is hidden (put away by Hide, not minimized to the Dock).
    pub(super) fn is_hidden(&self, app: AppId) -> bool {
        self.hidden.contains(&app) && self.index_of(app).is_some_and(|i| self.wins[i].minimized)
    }

    /// Whether any window is hidden now (Show All has something to show).
    pub(super) fn any_hidden(&self) -> bool {
        self.wins.iter().any(|w| w.minimized && self.hidden.contains(&w.app))
    }

    /// Whether a window of an app other than `parent` is on screen (Hide Others has something to hide).
    pub(super) fn others_shown(&self, parent: Option<AppId>) -> bool {
        self.wins.iter().any(|w| !w.minimized && Some(w.app.parent()) != parent)
    }

    /// Whether the window of `app` floats: an app's main window and its extra windows float with the app
    /// (`ToShell::WindowLevel`), and one extra window floats on its own (`ToShell::WindowLevelOf`). A built-in window
    /// never floats, so Settings, Terminal and the other built-ins always come in front of the normal windows (W4-1).
    pub(super) fn is_floating(&self, app: AppId) -> bool {
        matches!(app, AppId::External(_) | AppId::ExternalWindow(..)) && (self.floating.contains(&app) || self.floating.contains(&app.parent()))
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
        // the bands follow `is_floating`, the test `band_top` places a raised or opened window with: sorting by another
        // rule (the exact id) left a window `band_top` counted as floating inside the normal band, and a window raised
        // later went in under it, behind the front (W4-1). The sort is stable, so each band keeps its order.
        let level: Vec<bool> = self.wins.iter().map(|w| self.is_floating(w.app)).collect();
        // true when the stack moved: a settled desktop asks for no frame
        let moved = level.windows(2).any(|p| p[0] && !p[1]);
        if moved {
            let mut wins: Vec<(bool, Win)> = level.into_iter().zip(std::mem::take(&mut self.wins)).collect();
            wins.sort_by_key(|(f, _)| *f);
            self.wins = wins.into_iter().map(|(_, w)| w).collect();
        }
        moved
    }

    /// The frontmost visible window hit at (x, y).
    pub fn pick(&self, m: &Metrics, x: f32, y: f32) -> Option<(usize, Hit)> {
        self.wins.iter().enumerate().rev().filter(|(_, w)| !w.minimized && !self.unmapped.contains(&w.app))
            // a shaped (borderless) window is hit only where it drew: the rest belongs to the window below (W2-22)
            .filter(|(_, w)| self.shapes.get(&w.app).map_or(true, |s| shape_contains(s, x - w.r.x as f32, y - w.r.y as f32)))
            .find_map(|(i, w)| w.hit(m, x, y).map(|h| (i, h)))
    }

    /// The work area windows live in: below the menu bar and inside the Dock's chosen edge.
    pub fn area(m: &Metrics) -> Rect {
        Rect { x: m.dock_left, y: m.menubar, w: (m.w - m.dock_left - m.dock_right).max(1), h: (m.h - m.menubar - m.dock_bottom).max(1) }
    }

    /// Opens (or raises) the window for `app`; `size` is the wanted outer size.
    pub fn open(&mut self, m: &Metrics, app: AppId, size: (i32, i32)) -> usize {
        if let Some(i) = self.index_of(app) {
            // opening a hidden app (the Dock, Cmd+Tab, Spotlight) activates it: its windows come back in the order Hide
            // left them, the one that was in front in front, not the main window over its viewer (E2E N2)
            if app == app.parent() && self.hidden.iter().any(|a| a.parent() == app) {
                if let Some(front) = self.show_hidden_app(app) {
                    return front;
                }
            }
            return self.raise(i);
        }
        let a = Self::area(m);
        let (w, h) = (size.0.min(a.w), size.1.min(a.h));
        let step = (m.title as f32 * 1.1) as i32;
        let n = (self.opened % 8) as i32;
        let (x, y) = match self.closed_origins.get(&app) {
            // a window opened again comes back where it was closed (W2-26); its toplevel asks to map there (`placed`)
            Some(&(x, y)) => {
                self.placed.insert(app);
                (x, y)
            }
            None => {
                self.opened += 1;
                (a.x + (a.w - w) / 2 + (n - 3) * step, a.y + (a.h - h) / 3 + n * step / 2)
            }
        };
        let x = x.clamp(a.x, (a.x + a.w - w).max(a.x));
        let y = y.clamp(a.y, (a.y + a.h - h).max(a.y));
        let r = Rect { x, y, w, h };
        let at = self.band_top(app);
        self.wins.insert(at, Win { app, r, maximized: false, minimized: false, full: false, restore: r, full_size: false });
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
        // the window was put somewhere on purpose (a restored session): its toplevel maps there, not at the window
        // server's cascade, so the frame the shell keeps is the one on screen and a later raise moves nothing (E2E N7)
        let app = win.app;
        self.placed.insert(app);
    }

    /// The space a full-screen window takes: the whole screen, under the menu bar and over the Dock.
    pub fn screen_rect(m: &Metrics) -> Rect {
        Rect { x: 0, y: 0, w: m.w, h: m.h }
    }

    /// The app's own resize of window `i` to a content size in points (`ToShell::SetContentSize`, `NSWindow.setContentSize`).
    /// The top-left corner stays where it is (moved only as far as the work area needs); the size is held to the app's
    /// limits (a fixed-size window takes it as asked: only the person cannot resize it) and to the work area. A zoomed
    /// or full-screen window keeps its frame and takes the size as the one it goes back to. Returns whether the frame
    /// changed.
    pub fn set_content_size(&mut self, m: &Metrics, i: usize, size: (f32, f32)) -> bool {
        let Some(win) = self.wins.get(i).copied() else { return false };
        let a = Self::area(m);
        let (w, h) = outer_px(win.app, m, size);
        let (w, h) = if size_limits(win.app).fixed {
            (w, h)
        } else {
            let (min_w, min_h, max_w, max_h) = size_range(win.app, m);
            (w.clamp(min_w, max_w), h.clamp(min_h, max_h))
        };
        let (w, h) = (w.min(a.w).max(1), h.min(a.h).max(1));
        let fit = |r: Rect| {
            let x = r.x.clamp(a.x, (a.x + a.w - w).max(a.x));
            let y = r.y.clamp(a.y, (a.y + a.h - h).max(a.y));
            Rect { x, y, w, h }
        };
        let win = &mut self.wins[i];
        if win.full || win.maximized {
            win.restore = fit(win.restore);
            return false;
        }
        let r = fit(win.r);
        if r == win.r {
            return false;
        }
        win.r = r;
        win.restore = r;
        true
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
                if !self.full_from_zoom.contains(&w.app) {
                    self.full_from_zoom.push(w.app);
                }
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
        self.closed_origins.insert(w.app, (r.x, r.y));
        // the toplevel is gone; `open` asks the next one to map where this one was (`closed_origins`)
        self.placed.remove(&w.app);
        self.moved.remove(&w.app);
        self.shapes.remove(&w.app);
        self.release_app_buffers(w.app);
        self.collapsed.remove(&w.app);
        // a window closed while full screen from a zoom must not leave its way back for the window opened next
        self.full_from_zoom.retain(|a| *a != w.app);
        // nor a window Show Desktop had put aside (the next window of the app is not brought back with the others)
        self.put_aside.retain(|a| *a != w.app);
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
                let title = self.wins[i].title_h(m).max(1);
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
/// One step through `n` items from `i`, wrapping: forwards, or backwards when `reverse`.
pub(super) fn cycle_step(i: usize, n: usize, reverse: bool) -> usize {
    if n == 0 { 0 } else if reverse { (i + n - 1) % n } else { (i + 1) % n }
}

/// The next Cmd+` stop: the order to walk and the window to bring forward. `stack` is the front app's windows, front
/// first. A walk continues while the window in front is the one it raised last and the app has the same windows;
/// otherwise a new walk starts from the stack as it is. None with fewer than two windows.
pub(super) fn window_cycle_next(prev: Option<(Vec<AppId>, usize)>, stack: &[AppId], reverse: bool) -> Option<(Vec<AppId>, usize)> {
    if stack.len() < 2 { return None; }
    let same = |order: &Vec<AppId>| order.len() == stack.len() && stack.iter().all(|a| order.contains(a));
    let (order, at) = match prev {
        Some((order, at)) if same(&order) && order.get(at) == stack.first() => (order, at),
        _ => (stack.to_vec(), 0),
    };
    let next = cycle_step(at, order.len(), reverse);
    Some((order, next))
}

/// The toplevels of the app `app_id` to activate, in order, among the window server's (`id`, app id, active): every
/// one of the app's, the one reported active last, so activating them in turn leaves the app's front window in front.
/// An empty id matches nothing (a window without an app id is no app's).
#[cfg(any(feature = "desktop-wayland", test))]
pub(super) fn direct_activation_order<'a>(windows: impl IntoIterator<Item = (u32, &'a str, bool)>, app_id: &str) -> Vec<u32> {
    if app_id.is_empty() {
        return Vec::new();
    }
    let mine: Vec<(u32, bool)> = windows.into_iter().filter(|(_, id, _)| *id == app_id).map(|(w, _, active)| (w, active)).collect();
    mine.iter().filter(|(_, a)| !a).chain(mine.iter().filter(|(_, a)| *a)).map(|(w, _)| *w).collect()
}

/// The press areas of the Dock's tiles (`rects`, laid out along the Dock's axis, `vertical` for a side Dock): each
/// icon grown along the axis by half the gap between two icons, so a press between two tiles opens the nearer one.
/// The pointer resting in that gap magnifies both neighbours alike, and a press there opened nothing (E2E N5: the Files
/// icon "did nothing" twice). The wider gap at a divider keeps a dead strip in its middle.
pub(super) fn dock_press_rects(rects: &[Rect], vertical: bool) -> Vec<Rect> {
    let gap = rects
        .windows(2)
        .map(|p| if vertical { p[1].y - (p[0].y + p[0].h) } else { p[1].x - (p[0].x + p[0].w) })
        .filter(|g| *g > 0)
        .min()
        .unwrap_or(0);
    let half = (gap + 1) / 2;
    rects.iter().map(|r| if vertical { Rect { y: r.y - half, h: r.h + 2 * half, ..*r } } else { Rect { x: r.x - half, w: r.w + 2 * half, ..*r } }).collect()
}

/// Whether the release of a Dock press at (`x`, `y`) opens the pressed icon: the icon as it was hit at the press
/// (`pressed`) decides, so a row that re-centred under a still pointer cannot drop the click or move it to a neighbour
/// (W2-23); the slot's current rect counts too while it still holds the same entry (`now`, `now_entry`).
pub(super) fn dock_release_hits(pressed: Rect, now: Option<Rect>, now_entry: Option<usize>, entry: usize, x: f32, y: f32) -> bool {
    pressed.contains(x, y) || (now_entry == Some(entry) && now.is_some_and(|r| r.contains(x, y)))
}

fn dock_span(icon: i32, pad: i32, gap: i32, n: usize, seps: &[usize], sep: i32) -> i32 {
    n as i32 * icon + (n as i32 - 1).max(0) * gap + seps.len() as i32 * sep + 2 * pad
}

pub fn dock_rects_seps(m: &Metrics, icon: i32, pad: i32, gap: i32, n: usize, seps: &[usize], sep: i32) -> (Rect, Vec<Rect>) {
    let total = dock_span(icon, pad, gap, n, seps, sep);
    let x0 = (m.w - total) / 2;
    let y0 = m.h - icon - 2 * pad - (m.pt * DOCK_MARGIN_PT).round() as i32;
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

    /// The settled desktop frame draws nothing of its own, but the window server is still handed every window's frame
    /// (`take_wayland_windows`): an empty list destroys the toplevels. True when they are kept as the last frame left
    /// them; false when only a full desktop frame can publish them (W5-1). Off the window server's path there is
    /// nothing to keep.
    pub(super) fn desk_windows_kept(&mut self) -> bool {
        #[cfg(feature = "desktop-wayland")]
        if std::env::var_os("SPRINGBOARD_WAYLAND").is_some() {
            return self.retain_wayland_windows();
        }
        true
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
            frames.push(crate::bare::wayland::WindowFrame {id,title:frame_title(id),x:w.r.x,y:w.r.y,width:w.r.w as usize,height:w.r.h as usize,pixels:None,damage:crate::gfx::Damage::new(w.r.w as usize,w.r.h as usize),maximized:w.maximized,fullscreen:w.full,title_height,drag_height:0,no_drag:Vec::new(),floating:false,fx:bifrost_render::glass::Fx::IDENTITY,opaque:!matches!(w.app,AppId::External(_)),origin:None,position:None,min_size:(0,0),input:None});
            let controls=match w.app {AppId::External(slot)=>ext.get(slot as usize).and_then(Option::as_ref).map_or(&[][..],|h|&h.controls[..]),_=>&[][..]};
            let (dh,nd)=drag_zone(w,&m,controls);
            if let Some(f)=frames.last_mut(){f.drag_height=dh;f.no_drag=nd;}
        }
        desk.frames=frames;
        true
    }
    /// True once after an open or focus that should raise the front window over native clients.
    pub fn take_raise_front(&mut self) -> bool { self.desk.as_mut().is_some_and(|d|std::mem::take(&mut d.raise_front)) }
    /// The windows (window server ids) the shell brought forward together since the last call, for
    /// `Wayland::restack`: the window server stacks them in the shell's order over other clients' windows (E2E5 1, 3).
    pub fn take_wayland_restack(&mut self) -> Vec<u32> {
        self.desk.as_mut().map(|d| std::mem::take(&mut d.restack).into_iter().filter_map(win_id).collect()).unwrap_or_default()
    }
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
            let top = w.title_h(&m) as f32;
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
                    // the App Store's sidebar is the same `.sidebar` material (its column is painted translucent)
                    AppId::AppStore => {
                        if let Some(side) = self.store.as_ref().map(|st| st.sidebar_px()).filter(|&p| p > 0) {
                            rects.push((0.0, top, side as f32, (w.r.h as f32 - top).max(0.0), 0.0, bifrost_render::glass::Kind::Sidebar, None, None));
                        }
                    }
                    // Photos' source list on the desktop, the same `.sidebar` material
                    AppId::Photos => {
                        if let Some(side) = self.photos.as_ref().map(|p| p.sidebar_px()).filter(|&p| p > 0) {
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
        let m = self.desk_metrics();
        self.desk.as_mut().map(|d| {
            d.sync_levels(floating);
            let mut frames = std::mem::take(&mut d.frames);
            // a floating window's frame goes last, so the window server stacks it above the normal ones
            for f in frames.iter_mut() {
                f.floating = frame_app(f.id).is_some_and(|a| d.is_floating(a));
            }
            frames.sort_by_key(|f| f.floating);
            for f in frames.iter_mut() {
                // a borderless window takes the pointer only where its newest pixels cover (W2-22)
                if let Some(a) = frame_app(f.id).filter(|a| borderless(*a)) {
                    if let Some(px) = f.pixels.as_ref().filter(|p| p.len() == f.width * f.height) {
                        let shape: std::sync::Arc<[[i32; 4]]> = alpha_shape(px, f.width, f.height).into();
                        d.shapes.insert(a, shape);
                    }
                    f.input = Some(d.shapes.get(&a).cloned().unwrap_or_else(|| std::sync::Arc::from(Vec::new())));
                }
                f.origin = frame_app(f.id).and_then(|a| origin_hint(&d.placed, a, f.x, f.y));
                f.position = frame_app(f.id).filter(|a| d.moved.remove(a)).map(|_| (f.x, f.y));
                // the toplevel advertises the app's own minimum (outer px, as its window geometry), not a fixed one
                f.min_size = frame_app(f.id).map_or((0, 0), |a| { let (w, h, _, _) = size_range(a, &m); (w, h) });
            }
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
        if let Some(d) = self.desk.as_mut() { d.foreign_brought_back(id); d.activate.push(id); }
        let _ = id;
        self.dirty = true;
    }

    #[cfg(feature = "desktop-wayland")]
    pub fn take_native_activations(&mut self) -> Vec<u32> {
        self.desk.as_mut().map(|d|std::mem::take(&mut d.activate)).unwrap_or_default()
    }
    /// The window server's toplevels to minimize (Show Desktop puts them aside, R3).
    #[cfg(feature = "desktop-wayland")]
    pub fn take_native_minimizes(&mut self) -> Vec<u32> {
        self.desk.as_mut().map(|d|std::mem::take(&mut d.minimize)).unwrap_or_default()
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
        desk.intent_check=true;
        // a toplevel put aside that closed or is shown again (the Dock, the app itself) is no longer aside
        desk.prune_foreign_aside(&windows);
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
        // a window list change is often an app quitting: wait for the native apps that have exited
        #[cfg(feature = "desktop-wayland")]
        desk.reap_native_children();
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
        let id = match ev { WindowEvent::Focus(id) | WindowEvent::ServerFocus(id) | WindowEvent::HandBack(id) | WindowEvent::Close(id) | WindowEvent::Resize(id,_,_) | WindowEvent::Maximized(id,_) | WindowEvent::Frame(id,..) | WindowEvent::Fullscreen(id,_) => id };
        // every event is reported for every toplevel of the shell, an app's extra windows (flagged ids) included
        let app = frame_app(id);
        let Some(app) = app else { return };
        let Some(i) = self.desk.as_ref().and_then(|d| d.index_of(app)) else { return };
        let trace = |what: &str| crate::bare::focus_trace::log(format_args!("event window={id} app={app:?}: {what}"));
        match ev {
            // a press is the person's choice: it ends any front intent and raises the window
            WindowEvent::Focus(_) => {
                if let Some(intent) = self.desk.as_mut().and_then(|d| d.front_intent.take()) { trace(&format!("press ends the front intent of {:?}", intent.app)); }
                trace("press: raised");
                self.desk.as_mut().unwrap().raise(i); if !self.is_locked() { self.screen = Screen::App(app.parent()); } self.dirty = true;
            },
            // The app the shell is bringing to the front (Spotlight, the Apps grid, the Dock) stays in front against the
            // window server's own focus of another window: the keyboard it hands back to the window it knows in front as
            // the exclusive layer goes (Settings over a Calculator that drew its first frame but has no toplevel yet), the
            // topmost toplevel it picks (Mail while Calendar starts), or a late keyboard enter of the window that was in
            // front (Weather over Settings). Raising that window here put it over the app just opened in the shell's own
            // stack, so the app's toplevel then mapped behind it without the keyboard (E2E6 Spotlight). Once the intended
            // window is drawn the window server is asked to activate it again.
            WindowEvent::ServerFocus(_) | WindowEvent::HandBack(_) if self.desk_front_intent().is_some_and(|want| want.parent() != app.parent()) => {
                let want = self.desk_front_intent().unwrap_or(app);
                let front = self.desk.as_ref().and_then(|d| d.front()).map_or(want, |w| w.app);
                let drawn = !self.desk_window_unmapped(front);
                // the next frame's `windows` call activates the front window again: the keyboard went elsewhere
                if drawn { self.desk.as_mut().unwrap().raise_front = true; }
                self.dirty = true;
                let kind = if matches!(ev, WindowEvent::HandBack(_)) { "hand-back" } else { "window server focus" };
                trace(&format!("{kind} ignored: {want:?} is being brought to the front ({})", if drawn { "activated again" } else { "activated when it maps" }));
            },
            // the compositor hands focus back to a window when a menu or sheet closes; on a locked screen that must not
            // lift the window over the lock (Lock Screen from the System menu closes the menu in the same turn); an extra window
            // focuses the app that opened it, because the routing sites match the parent (`desk_refocus` does the same)
            // the app just launched above it (no frame yet, so no toplevel) stays the front app: the focus the window
            // server hands back as Spotlight or the Apps grid closes went to the window it still knows in front (E2E6 1).
            // Only that hand-back: a press on the window below (a `Focus`) is the person's choice and raises it
            WindowEvent::HandBack(_) if self.desk_launching_above(i) => trace("hand-back ignored: the app above it is launching"),
            WindowEvent::ServerFocus(_) | WindowEvent::HandBack(_) => {
                trace(if matches!(ev, WindowEvent::HandBack(_)) { "hand-back: raised" } else { "window server focus: raised" });
                self.desk.as_mut().unwrap().raise(i); if !self.is_locked() { self.screen = Screen::App(app.parent()); } self.dirty = true;
            },
            WindowEvent::Close(_) => self.desk_close_app(app),
            // a full-screen window keeps the zoom it goes back to (`set_full_screen`): the window server's maximize state
            // while it is full screen is not a zoom or an un-zoom of it (W3-6)
            WindowEvent::Maximized(_,maximized) => {let win=&mut self.desk.as_mut().unwrap().wins[i]; if win.maximized!=maximized && !win.full {if maximized {win.restore=win.r;}else{win.r=win.restore;}win.maximized=maximized;self.dirty=true;}},
            WindowEvent::Resize(_,w,h) if w > 0 && h > 0 => {
                let m = self.desk_metrics();
                let win = &mut self.desk.as_mut().unwrap().wins[i];
                // never under the app's own minimum, as a drag is not (`resize_to`): Settings' is its split layout width
                let (min_w, min_h, _, _) = size_range(win.app, &m);
                win.r.w = w.max(min_w); win.r.h = h.max(min_h); self.desk_layout(i); self.dirty = true;
            },
            // The window server places, moves and resizes toplevels and xdg-shell never says where, so the shell's own
            // idea of the position drifted (E2E N1: `state` said 852,339 for a window drawn at 800,360). The frame it
            // reports is the one source of truth: everything the shell puts on the screen from this rectangle (app
            // context menus, the share menu, taps) lands on the window. The size stays with the configure (`Resize`).
            WindowEvent::Frame(_,x,y,_,_) => { self.desk.as_mut().unwrap().placed.insert(app); let win = &mut self.desk.as_mut().unwrap().wins[i]; if frame_follows(&mut win.r, x, y) { self.dirty = true; } },
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
                    // the keys go where a hardware keyboard's would (`hw_type`); Hide lowers the keyboard
                    if out == super::keyboard::Out::Hide {
                        self.set_keyboard(false, t);
                        return Some(vec![]);
                    }
                    return Some(self.hw_type(out, t));
                }
            }
            _ => return None,
        }
        Some(vec![])
    }

    /// Ends the press in progress without its lift (a long press became a secondary click): a Dock icon held down is
    /// not left dragging, where the next tap would drop it off the Dock.
    pub(super) fn desk_cancel_press(&mut self) {
        if let Some(d) = self.desk.as_mut() {
            if d.grab.take().is_some() {
                d.dock_drop = None;
                d.dock_slide.borrow_mut().clear();
                self.dirty = true;
            }
        }
    }

    /// What the tablet desktop's on-screen keyboard should do for the text field in focus: `Some(true)` when typing
    /// reaches a field, `Some(false)` when it reaches none, and `None` to leave the keyboard alone (no desktop, a
    /// locked screen, a Wayland client's field that the input method serves, or a terminal, which raises and lowers
    /// its own keyboard). The order follows `hw_type`.
    pub(super) fn desk_text_focus(&self) -> Option<bool> {
        if !self.desk_active() || self.is_locked() || self.im.active {
            return None;
        }
        if self.auth_showing() || self.volume_showing() || self.package_showing() || self.net_auth_showing() {
            return Some(true);
        }
        if self.quicksearch.is_shown() || self.assistant.is_shown() || self.char_viewer.is_some() {
            return Some(true);
        }
        #[cfg(feature = "desktop-wayland")]
        if self.doc_panel.is_some() || self.compose.is_some() {
            return Some(true);
        }
        match self.screen {
            Screen::App(AppId::Terminal | AppId::External(_)) => None,
            Screen::App(AppId::AppStore) => Some(self.store.as_ref().is_some_and(|c| c.text_focused())),
            Screen::App(AppId::Settings) => Some(self.settings.as_ref().is_some_and(|s| s.text_focused())),
            Screen::App(AppId::Browser) => Some(self.browser.as_ref().is_some_and(|b| b.text_focused())),
            Screen::App(AppId::Files) => Some(self.files.as_ref().is_some_and(|f| f.text_focused())),
            Screen::App(AppId::Photos) => Some(self.photos.as_ref().is_some_and(|p| p.text_focused())),
            Screen::App(AppId::Deildu) => Some(self.deildu.as_ref().is_some_and(|d| d.text_focused())),
            _ => Some(false),
        }
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
        self.desk_direct_activate(slot, false)
    }

    /// `desk_direct_open`'s work: starts the direct app in `slot` if needed (restarting an outdated process, `ext_open`)
    /// and asks the window server to activate its toplevel, or with `all` every toplevel it shows, the one it reports
    /// active last so the app's front window stays in front (the Dock activating an app with extra windows, E2E N2).
    #[cfg(feature = "desktop-wayland")]
    fn desk_direct_activate(&mut self, slot: usize, all: bool) -> bool {
        if apphost::launch_route() != apphost::Route::Direct { return false; }
        self.ext_open(slot);
        if let Some(h) = self.ext.get(slot).and_then(Option::as_ref) {
            if h.route() == apphost::Route::Shm { return false; }
        }
        let id = apphost::installed().get(slot).map(|a| a.manifest.id.clone()).unwrap_or_default();
        let toplevels: Vec<u32> = match self.desk.as_ref() {
            Some(d) if all => direct_activation_order(d.native_windows.iter().map(|w| (w.id, w.app_id.as_str(), w.active)), &id),
            Some(d) => apphost::foreign_window_of(d.native_windows.iter().map(|w| (w.id, w.app_id.as_str())), &id).into_iter().collect(),
            None => Vec::new(),
        };
        if let Some(d) = self.desk.as_mut() {
            for toplevel in toplevels {
                d.foreign_brought_back(toplevel);
                d.activate.push(toplevel);
            }
        }
        self.dirty = true;
        true
    }

    #[cfg(feature = "desktop-wayland")]
    fn desk_front_index(&self) -> Option<usize> {
        self.desk.as_ref().and_then(|d| d.front().and_then(|w| d.index_of(w.app)))
    }

    /// Tiles the front window on `tile` (Window menu and Ctrl+Cmd arrow keys). The front window is the one the menu bar
    /// stands for: another client's window the window server reports active (a native app's), else the shell's top
    /// window. A native window is placed by the window server through its foreign toplevel handle (the effect), never by
    /// whichever window the window server's focus is on (E2E N2).
    #[cfg(feature = "desktop-wayland")]
    pub(super) fn desk_tile_front(&mut self, tile: Tile) -> Vec<Effect> {
        if let Some(id) = self.foreign_front_window() {
            return vec![Effect::ForeignWindowTile(id, placement_of(tile))];
        }
        let Some(i) = self.desk_front_index() else { return Vec::new() };
        self.desk_tile_window(i, tile);
        Vec::new()
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

    /// Returns the front window (as `desk_tile_front` picks it) to the size it had before its last tiling or zoom.
    #[cfg(feature = "desktop-wayland")]
    pub(super) fn desk_restore_front(&mut self) -> Vec<Effect> {
        if let Some(id) = self.foreign_front_window() {
            return vec![Effect::ForeignWindowTile(id, crate::bare::window_level_client::Placement::Previous)];
        }
        let Some(i) = self.desk_front_index() else { return Vec::new() };
        self.desk_restore_window(i);
        Vec::new()
    }

    /// Returns window `i` to the size it had before its last tiling or zoom (the window server does it in the Wayland
    /// session, as it does for the zoom's way back).
    #[cfg(feature = "desktop-wayland")]
    pub(super) fn desk_restore_window(&mut self, i: usize) {
        let Some(w) = self.desk.as_ref().and_then(|d| d.wins.get(i)).copied() else { return };
        if w.full { return; }
        if compositor_places_windows() {
            // a zoom (Fill, the green button, a full-screen exit that came back filled) is undone as a zoom: the shell's
            // frame goes back to the one it kept and the window server drops its maximize to the same frame, so neither
            // side is left waiting for the other (W3-6); a tile goes back through the window server, which made it
            if w.maximized {
                self.desk_zoom_window(i);
            } else {
                self.desk_request_placement(w.app, crate::bare::window_level_client::Placement::Previous);
            }
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
            // the pointer left a surface: the window server says so when the pointer crosses from the window onto the
            // menu's own surface, before the menu's first motion, so a Leave is not a move away from the menu (W3-5);
            // the next real motion outside the button, bridge and menu closes it
            if Self::pointer_left_surface(x, y) { return; }
            if !self.arrange_keeps(x, y) {
                self.desktop_menu = None;
                self.dirty = true;
                if let Some(d) = self.desk.as_mut() { d.arrange_at = None; d.arrange = None; }
            }
            return;
        }
        let Some(d) = self.desk.as_mut() else { return };
        match (over, d.arrange_at) {
            // a menu that closed some other way (a click, a key, a choice) latched the rest; the pointer moving on the
            // button again is a new rest, so the menu opens once more after the dwell (W3-5)
            (Some(a), Some((b, since))) if a == b && since.is_infinite() => d.arrange_at = Some((a, t)),
            (Some(a), Some((b, _))) if a == b => {}
            (Some(a), _) => d.arrange_at = Some((a, t)),
            (None, _) => d.arrange_at = None,
        }
    }

    /// Whether (x, y) is the position a pointer Leave reports (`bare::wayland` sends `Hover(-100, -100)` when the
    /// pointer leaves the shell's surface): no point of the screen is at negative coordinates on both axes.
    #[cfg(feature = "desktop-wayland")]
    fn pointer_left_surface(x: f32, y: f32) -> bool { x < 0.0 && y < 0.0 }

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

    /// An app resized its main window (`ToShell::SetContentSize`): the window takes the new frame through the zoom
    /// effect, and the app gets the new size as a configure (`desk_layout`).
    pub(super) fn desk_set_content_size(&mut self, app: AppId, size: (f32, f32)) {
        let m = self.desk_metrics();
        let Some(d) = self.desk.as_mut() else { return };
        let Some(i) = d.index_of(app) else { return };
        if !d.set_content_size(&m, i, size) {
            return;
        }
        self.desk_layout(i);
        self.desk_frame_changed(i);
    }

    pub(super) fn desk_zoom_window(&mut self,i:usize) {let m=self.desk_metrics();self.desk.as_mut().unwrap().toggle_zoom(&m,i);self.desk_layout(i);if let Some(app)=self.desk.as_ref().and_then(|d|d.wins.get(i)).map(|w|w.app){let t=self.last_input;self.desk_fx_start(app,FxCall::Zoom,t);}self.dirty=true;}

    /// Minimizes window `i` to its Dock tile, playing the minimize effect first when it is allowed.
    pub(super) fn desk_minimize_window(&mut self, i: usize, t: f64) {
        let Some(app) = self.desk.as_ref().and_then(|d| d.wins.get(i)).map(|w| w.app) else { return };
        // the effect starts while the window is still mapped; the draw loop keeps it up until the effect ends
        self.desk_fx_start(app, FxCall::Minimize, t);
        // its Dock thumbnail: one picture, asked for while the window server still has it on screen
        self.dock_shot(app, t);
        if let Some(d) = self.desk.as_mut() { d.wins[i].minimized = true; d.hidden.retain(|a| *a != app); d.put_aside.retain(|a| *a != app); }
        // the desktop it uncovers is drawn whole, not only the bar and Dock regions (W4-4)
        self.force_full_redraw();
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
        let position = self.dock_preferences().position;
        let mut m = Metrics {
            w: self.w, h: self.h,
            title: ui.pt(TITLE_PT).round() as i32,
            menubar: self.desktop_bar_height() as i32,
            min_w: ui.pt(MIN_W_PT).round() as i32,
            min_h: ui.pt(MIN_H_PT).round() as i32,
            edge: ui.pt(EDGE_PT).round() as i32,
            light: ui.pt(LIGHT_PT).round() as i32,
            light_gap: ui.pt(LIGHT_GAP_PT).round() as i32,
            light_x: ui.pt(LIGHT_X_PT).round() as i32,
            dock_bottom: 0, dock_left: 0, dock_right: 0,
            pt: ui.pt(1.0),
        };
        let bar = self.desk_dock(&m).1;
        let depth = if position == super::desktop_dock::Position::Bottom { bar.h } else { bar.w };
        let reserve = if self.desk.as_ref().is_some_and(|d| d.dock_autohides()) { 0 } else {
            depth + 2 * ui.pt(DOCK_MARGIN_PT).round() as i32
        };
        match position {
            super::desktop_dock::Position::Bottom => m.dock_bottom = reserve,
            super::desktop_dock::Position::Left => m.dock_left = reserve,
            super::desktop_dock::Position::Right => m.dock_right = reserve,
        }
        m
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
        list.extend(desk.wins.iter().enumerate().filter(|(_, w)| self.overview.user_minimized(w) && !desk.is_hidden(w.app) && !desk.is_put_aside(w.app)).map(|(wi, _)| super::desktop_dock::DOCK_MIN_BASE - wi));
        let ui = &self.ui;
        // the Downloads stack and the Trash are last, after the last divider
        list.push(super::dock_stack::DOCK_STACK);
        list.push(DOCK_TRASH);
        let seps = self.dock_seps(&list);
        let icon = ui.pt(self.dock_preferences().size).round() as i32;
        let pad = ui.pt(DOCK_PAD_PT).round() as i32;
        let gap = ui.pt(DOCK_GAP_PT).round() as i32;
        let sep = ui.pt(DOCK_SEP_PT).round() as i32;
        let available = if self.dock_preferences().position == super::desktop_dock::Position::Bottom { m.w } else { m.h - m.menubar };
        // Fit the whole row, including the stack, Trash and every divider. Padding and gaps shrink with the icons.
        let wanted = dock_span(icon, pad, gap, list.len(), &seps, sep).max(1);
        let factor = ((available - 2 * ui.pt(DOCK_MARGIN_PT).round() as i32).max(1) as f32 / wanted as f32).min(1.0);
        let scaled = |v: i32| (v as f32 * factor).floor() as i32;
        let (icon, pad, gap, sep) = (scaled(icon).max(1), scaled(pad), scaled(gap), scaled(sep));
        let (bar, rects) = dock_rects_seps(m, icon, pad, gap, list.len(), &seps, sep);
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
        // activating an app that shows windows besides its main one (from Search, Cmd+Tab, a menu or the Dock) brings
        // all of them forward in their own order, not only the main window (E2E9 5). Only while its main window is
        // open: a closed (or minimized) main window is reopened below, as before.
        if self.desk_main_window_shown(app) && self.desk_activate_shown_app(app, t) {
            self.quicksearch.note_launch(app);
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
        // a borderless window is a transparent layer over the whole work area, so what it draws can go anywhere on it
        let (w, h) = if borderless(app) { let a = Desk::area(&m); (a.w, a.h) } else { (w, h) };
        // a window opened again comes back at the size it was closed with; a fixed manifest size still wins
        let (w, h) = match (size_limits(app).initial, self.desk.as_ref().and_then(|d| d.closed_sizes.get(&app).copied())) {
            // held to the window's range: a size below the minimum must not come back with the window (E2E-12)
            // a frame as tall or as wide as the work area (a tile or the Fill frame the window server gave it) is not a
            // size the person chose: the window opens at its default size instead of filling the screen (W4-3, W4-8)
            (None, Some(closed)) if !borderless(app) && remembered_size(Desk::area(&m), closed) => clamp_size(app, &m, closed),
            _ => (w, h),
        };
        let existed = self.desk.as_ref().is_some_and(|d| d.index_of(app).is_some());
        let was_minimized = self.desk.as_ref().and_then(|d| d.index_of(app).map(|i| d.wins[i].minimized)).unwrap_or(false);
        if !existed {
            let (bw, bh) = (w, h - title_of(app, &m));
            self.with_size(bw, bh, |sh| sh.init_app(app, t));
            // a new App Store window opens on the front page, not where the last one was closed (W5-20)
            if app == AppId::AppStore {
                if let Some(s) = self.store.as_mut() {
                    s.show_front_page();
                }
            }
        }
        let i = self.desk.as_mut().unwrap().open(&m, app, (w, h));
        // the app opened is the front app until it is in front for good (`FrontIntent`), whatever the window server
        // focuses meanwhile on its own
        if let Some(d) = self.desk.as_mut() {
            d.front_intent = Some(FrontIntent { app, since: t });
            crate::bare::focus_trace::log(format_args!("intent app={app:?} at={t:.0}: opened"));
        }
        if !existed {
            self.desk_layout(i);
            self.desk_fx_start(app, FxCall::Open, t);
        } else if was_minimized {
            self.desk_fx_start(app, FxCall::Restore, t);
        }
        self.quicksearch.note_launch(app);
        // Opening an app on the desktop leaves its notifications in Notification Center, as on the reference: a click on
        // one card or banner clears that one, a stack clears from its X or Clear All (E2E W5-6). The tablet's `open`
        // still clears them.
        self.fullscreen = false;
        if !self.is_locked() {
            self.screen = Screen::App(app);
        }
        // the phone dock's recents are not kept on the desktop: Recent Items has its own list
        self.dirty = true;
    }

    /// Focus follows the frontmost visible window; with none, the desktop (Home) shows.
    /// Window > Bring All to Front for the active app (the app of the front window when none is active). The windows
    /// Show Desktop put aside come back first (N13): the shell's, and of the window server's toplevels only the active
    /// app's (Show Desktop's toggle brings back the rest); windows minimized to the Dock stay there.
    pub(super) fn desk_bring_all_to_front(&mut self) {
        let shell_app_front = matches!(self.screen, Screen::App(_));
        if self.desk.as_mut().is_some_and(|d| d.bring_back_front_app_aside(shell_app_front)) {
            self.force_full_redraw();
        }
        let parent = match self.screen {
            Screen::App(app) => Some(app.parent()),
            _ => self.desk.as_ref().and_then(|d| d.front()).map(|w| w.app.parent()),
        };
        if let (Some(parent), Some(d)) = (parent, self.desk.as_mut()) {
            d.bring_all_to_front(parent);
            d.raise_front = true;
        }
        self.desk_refocus();
    }

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
    /// Cmd+` (`reverse`: Shift+Cmd+`) brings the front app's next (previous) window forward, its extra windows
    /// included; minimized windows are skipped. While the Cmd+Tab strip is open it moves the selection instead.
    pub(super) fn desk_cycle_window(&mut self, reverse: bool) {
        if let Some(d) = &mut self.desk {
            match &mut d.cmdtab {
                Some((order, sel)) => *sel = cycle_step(*sel, order.len(), reverse),
                None => {
                    let front = d.front().map(|w| w.app.parent());
                    let stack: Vec<AppId> = d.wins.iter().rev().filter(|w| !w.minimized && Some(w.app.parent()) == front).map(|w| w.app).collect();
                    if let Some((order, sel)) = window_cycle_next(d.window_cycle.take(), &stack, reverse) {
                        d.window_cycle = Some((order.clone(), sel));
                        d.cmdtab = Some((order, sel));
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

    /// Show Desktop (Window menu, hot corner, gesture): puts every shown window aside to show the desktop; with none
    /// shown and windows aside, it is the opposite command and brings them back (N13).
    pub(super) fn desk_show_desktop(&mut self) {
        let any_shown = self.desk.as_ref().is_some_and(|d| d.any_shown());
        if !any_shown && self.desk.as_mut().is_some_and(|d| d.bring_back_put_aside()) {
            if let Some(d) = self.desk.as_mut() {
                d.raise_front = true;
            }
            self.force_full_redraw();
            self.desk_refocus();
            return;
        }
        self.desk_put_windows_aside();
    }

    /// Puts every shown window aside and shows the desktop (`go_home` on the desktop never toggles them back). The
    /// windows get no Dock slot: they are aside, not minimized by the person.
    pub(super) fn desk_put_windows_aside(&mut self) {
        if let Some(d) = &mut self.desk {
            d.put_windows_aside();
        }
        // the whole desktop surface is drawn afresh: a partial frame clears only the bar, Dock and menu regions, so
        // anything left on the surface under the windows (a stray drag image) stayed in the middle of the bare
        // desktop until the next full frame (W4-4)
        self.force_full_redraw();
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
            // only a running app's windows float: the level an exited app asked for is not kept for its windows
            let Some(h) = host.as_ref().filter(|h| h.running()) else { continue };
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
        // which extra windows draw their own title row (the app host's window table), known before any of them opens:
        // their size and chrome follow it, and each desk window keeps it (`Win::full_size`)
        let mut full: Vec<(u8, u16)> = Vec::new();
        for (slot, host) in self.ext.iter().enumerate() {
            let Some(h) = host else { continue };
            for x in h.windows.of_app(&h.id) {
                if let (true, Ok(id)) = (x.full_size, u16::try_from(x.id)) {
                    full.push((slot as u8, id));
                }
            }
        }
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
                let sizes: Vec<(u32, f32, f32, bool, bool)> = h.windows.of_app(&h.id).iter().map(|x| (x.id, x.w, x.h, x.cascade, x.position.is_some())).collect();
                plan.push((slot as u8, open, close, sizes));
            }
        }
        // a window that changed its level moves within the stack even when the table's windows did not change
        let floating = self.floating_apps();
        if let Some(d) = self.desk.as_mut() {
            changed |= d.sync_levels(floating);
            // a flag the table changed reaches the window already on the desk
            for w in d.wins.iter_mut() {
                if let AppId::ExternalWindow(s, id) = w.app {
                    let on = full.contains(&(s, id));
                    if w.full_size != on {
                        w.full_size = on;
                        changed = true;
                    }
                }
            }
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
                let (w, h, cascade, positioned) = sizes.iter().find(|s| s.0 == id).map_or((480.0, 560.0, false, false), |s| (s.1, s.2, s.3, s.4));
                let app = AppId::ExternalWindow(slot, id16);
                let own_title_row = full.contains(&(slot, id16));
                let size = extra_window_px(app, own_title_row, (w, h), k, &m);
                if let Some(d) = self.desk.as_mut() {
                    let i = d.open(&m, app, size);
                    d.wins[i].full_size = own_title_row;
                    if cascade {
                        // from the app's newest other extra window, else from its main window (AppKit's cascade)
                        let newest = d.wins.iter().filter_map(|o| match o.app {
                            AppId::ExternalWindow(s, other) if s == slot && other != id16 => Some((other, o.r)),
                            _ => None,
                        }).max_by_key(|o| o.0).map(|o| (o.1, false));
                        let from = newest.or_else(|| d.index_of(AppId::External(slot)).map(|j| (d.wins[j].r, true)));
                        if let Some((from, from_main)) = from {
                            let r = d.wins[i].r;
                            let (x, y) = cascade_origin(Desk::area(&m), from, from_main, (r.w, r.h), m.title);
                            d.wins[i].r.x = x;
                            d.wins[i].r.y = y;
                            d.wins[i].restore = d.wins[i].r;
                            // the new toplevel asks the window server to map it there, not at the server's own cascade
                            d.placed.insert(app);
                        }
                    }
                    self.desk_layout(i);
                }
                // the cascade places a window once, when it first opens: shown again (after the desk dropped it while
                // the table kept it) it comes back where the person left it, not cascaded anew
                if cascade {
                    if let Some(Some(h)) = self.ext.get_mut(slot as usize) {
                        h.windows.set_cascade(&h.id, id, false);
                    }
                }
                // a window the app gave no position: the app is told where the desk put it (`WindowMoved`), so it knows
                // each window's frame (a document app moving a document into its main window keeps that frame)
                if !positioned {
                    self.ext_window_dropped(app);
                }
            }
        }
        // the placement and collapse the table asks for (`WindowMove`, `WindowCollapse`), on every window the desk shows
        let mut asks = Vec::new();
        let mut main_moves = Vec::new();
        for (slot, host) in self.ext.iter_mut().enumerate() {
            let Some(h) = host else { continue };
            if let Some(p) = h.take_main_move() {
                main_moves.push((slot as u8, p));
            }
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
        // the main window moves where its app asked (`WindowMove` with window 0); its collapse is the desk's own
        for (slot, p) in main_moves {
            let app = AppId::External(slot);
            let collapsed = self.desk.as_ref().is_some_and(|d| d.is_collapsed(app));
            changed |= self.ext_window_place(app, Some(p), collapsed);
        }
        if changed {
            self.desk_refocus();
            self.dirty = true;
        } else if !self.is_locked() && self.bar_held_front().is_none() {
            // the front window decides the screen on every tick; only a different answer is something to draw. Under the
            // Apps grid every window is hidden, and the screen stays the app that was in front, whose menus the bar
            // keeps (Lead (a)); the grid's close refocuses
            // the app being brought to the front is the screen while the intent stands, whatever the window server's
            // focus did meanwhile (`desk_front_intent`)
            let front = self.desk_front_intent().or_else(|| self.desk.as_ref().and_then(|d| d.front()).map(|w| w.app));
            let screen = match front {
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
                // put there on purpose: its toplevel maps there (`bifrost_window_level_v1` `set_origin`), not at the
                // window server's cascade, and a toplevel already mapped is moved there (`set_position`, E2E11 3: a
                // document app's main window taking over a closed document's frame stayed at its old place on screen)
                d.placed.insert(app);
                d.moved.insert(app);
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
        // the window server raises it and gives it the keyboard as well, as an open does (W4-1)
        d.raise_front = true;
        self.desk_refocus();
        self.dirty = true;
        true
    }

    /// A wheel event at `t` ms. The Wayland pointer emulates a drag (a press, a move and a release) after each one, so
    /// lists scroll; that press must not grab window chrome: on an edge it resized the window by the notch (N1), on a
    /// title bar it moved it, on a traffic light or a Dock icon it clicked it. Only content takes the emulated drag.
    pub fn desk_wheel(&mut self, t: f64) {
        if let Some(d) = self.desk.as_mut() {
            d.wheel_at = Some(t);
        }
    }

    /// The wheel's mark (`desk_wheel`) is spent: a shell surface took the notch and drops its whole emulated drag
    /// (`Shell::swallow_wheel_drag`), or a press was routed already. A mark left behind made the next real press on
    /// chrome within `WHEEL_DRAG_MS` count as the wheel's, so a title-bar drag right after a notch over Settings did nothing.
    pub(super) fn desk_wheel_spent(&mut self) {
        if let Some(d) = self.desk.as_mut() {
            d.wheel_at = None;
        }
    }

    /// True (once) for the press the Wayland pointer emulates for the wheel event noted at most `WHEEL_DRAG_MS` before.
    fn desk_wheel_press(&mut self, t: f64) -> bool {
        matches!(self.desk.as_mut().and_then(|d| d.wheel_at.take()), Some(at) if t >= at && t - at <= WHEEL_DRAG_MS)
    }

    /// Touches over the desktop. `None` leaves the event to the normal path.
    pub(super) fn desk_event(&mut self, ev: Event, t: f64) -> Option<Vec<Effect>> {
        let fx = self.desk_event_inner(ev, t);
        // Settings ran at its window's size. Apply desktop changes only after with_size restored the output size.
        let changes = self.settings.as_mut().map(|st| st.take_dock_changes()).unwrap_or_default();
        for change in changes { let _ = self.dock_change(change, t); }
        fx
    }

    fn desk_event_inner(&mut self, ev: Event, t: f64) -> Option<Vec<Effect>> {
        // a press is also a pointer position: the row is magnified about it before the hit test, so the press
        // resolves the icon drawn under the pointer, not one laid out around where the pointer was a moment ago (W2-23)
        if let Event::Down(x, y) = ev { let _ = self.dock_pointer(super::desktop_dock::Pointer::Hover(x, y), t); }
        let m = self.desk_metrics();
        let (list, bar0, icons) = self.desk_dock(&m);
        let (dock_bar, icons) = self.dock_live(bar0, &icons, t / 1000.0);
        let press = self.dock_press(&icons);
        match ev {
            Event::Down(x, y) => {
                self.desk.as_mut()?.grab = None;
                let wheel = self.desk_wheel_press(t);
                // the dock sits over the windows
                if let Some(k) = press.iter().position(|r| self.dock_is_visible()&&r.contains(x, y)) {
                    // the wheel over the Dock opens nothing
                    if wheel {
                        return Some(vec![]);
                    }
                    self.desk.as_mut()?.grab = Some(Grab::DockDrag{from:k,entry:list[k],rect:press[k],x,y,at_x:x,at_y:y,moved:false});
                    self.desk.as_mut()?.dock_drop = None;
                    // an app that already has a window comes forward on the press, not after the release
                    if let Some(a) = apps().get(list[k]).map(|a| Self::dock_app(a.id)) {
                        if self.desk.as_ref()?.index_of(a).is_some() && !self.desk_activate_shown_app(a, t) { self.desk_open(a, t); }
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
                        if w.is_full_size() && holes.iter().skip(1).any(|r| lx >= r[0] && lx < r[0] + r[2] && ly >= r[1] && ly < r[1] + r[3]) {
                            hit = Hit::Content;
                        }
                    }
                }
                // the wheel's emulated drag scrolls content only: with no grab its move and release are swallowed (N1)
                if wheel && hit != Hit::Content {
                    self.dirty = true;
                    return Some(vec![]);
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
                    Hit::Light(l) => Grab::Light(app, l),
                    Hit::Title => Grab::Move { dx: x - w.r.x as f32, dy: y - w.r.y as f32, app },
                    Hit::Edge(edges) => Grab::Resize { edges, start: w.r, x, y, app },
                    Hit::Content => {
                        self.desk.as_mut()?.grab = Some(Grab::Content(app));
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
                    Grab::Resize { edges, start, x: x0, y: y0, app } => {
                        let i = self.desk.as_ref()?.index_of(app)?;
                        self.desk.as_mut()?.resize_to(&m, i, edges, start, (x - x0) as i32, (y - y0) as i32);
                        self.desk_layout(i);
                        Some(vec![])
                    }
                    Grab::Content(app) => {
                        let d = self.desk.as_ref()?;
                        let w = *d.wins.get(d.index_of(app)?)?;
                        let b = w.body(&m);
                        let fx = self.with_size(b.w, b.h, |sh| sh.app_touch_single(w.app, Event::Move(x - b.x as f32, y - b.y as f32), t));
                        Some(fx)
                    }
                    Grab::DockDrag{from,entry,rect,x:x0,y:y0,..}=>{
                        let moved=(x-x0).abs()+(y-y0).abs()>self.ui.pt(8.);
                        let desk=self.desk.as_mut()?;
                        desk.grab=Some(Grab::DockDrag{from,entry,rect,x:x0,y:y0,at_x:x,at_y:y,moved});
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
                    Some(Grab::Content(app)) => {
                        let d = self.desk.as_ref()?;
                        let w = *d.wins.get(d.index_of(app)?)?;
                        let b = w.body(&m);
                        let fx = self.with_size(b.w, b.h, |sh| sh.app_touch_single(w.app, Event::Up(x - b.x as f32, y - b.y as f32), t));
                        Some(fx)
                    }
                    Some(Grab::Light(app, l)) => {
                        let i = self.desk.as_ref()?.index_of(app)?;
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
                    Some(Grab::DockDrag{from:k,entry,x:x0,y:y0,..}) if (x-x0).abs()+(y-y0).abs()>self.ui.pt(8.) => {
                        // the neighbours already show the final order, so dropping them in place is seamless
                        let slot=self.desk.as_mut()?.dock_drop.take();
                        self.desk.as_ref()?.dock_slide.borrow_mut().clear();
                        // an app pinned in the Dock and dragged well off it leaves the Dock (its Pin toggles off)
                        if slot.is_none() && self.dock_removable(entry) && super::desktop_dock::dock_drag_off(dock_bar, x, y, self.ui.pt(DOCK_REMOVE_PT)) {
                            self.dock_change(super::desktop_dock::Change::Pin(entry), t);
                        } else if let Some(&to)=slot.and_then(|target|list.get(target)){self.dock_reorder(entry,to,slot.unwrap_or(0)>k);}
                        Some(vec![])
                    },
                    Some(Grab::DockDrag{from:k,entry,rect,..}) => {
                        // the icon latched at the press decides: the row may have re-centred since (W2-23)
                        if dock_release_hits(rect, press.get(k).copied(), list.get(k).copied(), entry, x, y) {
                            if entry == DOCK_TRASH {
                                // the account's Trash as Files keeps it; the shell runs as root, so not $HOME, and Files creates it
                                let trash=crate::files::trash::dir(&super::dock_stack::home_dir());
                                if let Err(e)=self.desktop_open_folder(FILES_APP,&trash.to_string_lossy(),t){eprintln!("BifrOSt Trash: {e}");}
                                return Some(vec![]);
                            }
                            if entry == super::dock_stack::DOCK_STACK {
                                // the Downloads stack opens above its icon, a fan or a grid of its newest items
                                // (`dock_stack::layout`); without the desktop menus it opens its folder in Files
                                #[cfg(feature = "desktop-wayland")]
                                self.stack_open(rect);
                                #[cfg(not(feature = "desktop-wayland"))]
                                {
                                    let folder = super::dock_stack::downloads_dir();
                                    if let Err(e)=self.desktop_open_folder(FILES_APP,&folder.to_string_lossy(),t){eprintln!("BifrOSt Downloads: {e}");}
                                }
                                return Some(vec![]);
                            }
                            if let Some(wi) = super::desktop_dock::dock_min_index(entry) {
                                // a minimized window comes back from its slot
                                let app = self.desk.as_ref()?.wins.get(wi)?.app;
                                self.desk_open(app, t);
                                return Some(vec![]);
                            }
                            #[cfg(feature = "desktop-wayland")]
                            if entry > apps().len() {
                                let app=self.desk.as_ref()?.linux_apps.get(entry-apps().len()-1)?;
                                if app.windows.is_empty() {
                                    let id = app.app_id.clone();
                                    self.dock_start_bounce(entry, t);
                                    crate::bare::sys::spawn_named("launch-apps", move || { let _ = std::process::Command::new("/usr/bin/bifrost-apps").arg(id).status(); });
                                    return Some(vec![]);
                                }
                                let next=app.windows.iter().position(|w|w.active).map(|i|(i+1)%app.windows.len()).unwrap_or(0);
                                return Some(vec![Effect::ActivateForeign(app.windows[next].id)]);
                            }
                            if entry == apps().len() {
                                // the Apps icon opens the Apps grid
                                self.appgrid_toggle(t);
                                return Some(vec![]);
                            }
                            let Some(app) = apps().get(entry).map(|a| a.id) else { return Some(vec![]) };
                            self.dock_start_bounce(entry, t);
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
            let id=windows[next].id; desk.foreign_brought_back(id); desk.activate.push(id); return true;
        }
        if desk.native_children.get_mut(&package.manifest.id).is_some_and(|c|matches!(c.try_wait(),Ok(None))) {return true;}
        let launch=crate::bare::account_policy::check_launch(&package.manifest.id).and_then(|()|crate::bare::app_policy::Policy::from_permissions(&package.manifest.id,&package.manifest.permissions)).and_then(|policy|{crate::bare::app_policy::ask_microphone_at_launch(&package.manifest.id,&package.manifest.name,policy,true,package.manifest.records_audio);crate::bare::app_policy::command(&package.manifest.id,&package.exec_path(),&apphost::root().join("appdata").join(&package.manifest.id),policy,true)}).and_then(|mut cmd|cmd.spawn());
        match launch {
            Ok(child) => {desk.native_children.insert(package.manifest.id.clone(),child);}
            Err(e) => eprintln!("BifrOSt native app {}: {e}",package.manifest.id),
        }
        self.dirty=true;
        true
    }
    /// True when `app`'s main window is on the desk and not minimized: `desk_open` then only activates the app's shown
    /// windows; otherwise it reopens (or restores) the main window.
    fn desk_main_window_shown(&self, app: AppId) -> bool {
        self.desk.as_ref().is_some_and(|d| d.wins.iter().any(|w| w.app == app && !w.minimized))
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
        if self.desk_activate_shown_app(app, t) {
            return;
        }
        // Golden Gate: clicking the frontmost app's icon does nothing; minimizing is the yellow light or Cmd+M
        self.desk_open(app, t);
    }

    /// The Dock activates an app that shows a window besides its main one: the app's shown windows come forward in the
    /// order they have among themselves, so its front window (a Mail viewer over the main window) stays in front and
    /// keeps the keyboard, as activating an app does (E2E N2). False leaves the click to `desk_open` (an app that is
    /// hidden, not running, or that shows its main window alone). Everything else an open does still happens: the Screen
    /// Time launch check, the Recent Items entry and, for a direct app, the restart of an outdated process and the
    /// activation of its toplevels (`desk_direct_open`), only the shell's stacking is this one's.
    pub(super) fn desk_activate_shown_app(&mut self, app: AppId, t: f64) -> bool {
        {
            let Some(d) = self.desk.as_ref() else { return false };
            if app != app.parent() || d.hidden.iter().any(|a| a.parent() == app) {
                return false;
            }
            if !d.wins.iter().any(|w| !w.minimized && w.app != app && w.app.parent() == app) {
                return false;
            }
        }
        let blocked = self.st_launch_check(app, t);
        self.recent_note(app);
        if let Some(d) = self.desk.as_mut() {
            d.bring_all_to_front(app);
            d.raise_front = true;
        }
        #[cfg(feature = "desktop-wayland")]
        if let AppId::External(slot) = app {
            if !blocked {
                self.desk_direct_activate(slot as usize, true);
            }
        }
        #[cfg(not(feature = "desktop-wayland"))]
        let _ = blocked;
        if !self.is_locked() {
            self.screen = Screen::App(app);
        }
        self.fullscreen = false;
        self.dirty = true;
        true
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
        let top = w.title_h(&self.desk_metrics());
        let h = w.r.h - top;
        (h > 0 && w.r.w > 0).then_some((w.r.x, w.r.y + top, w.r.w, h))
    }

    /// Draws the Time Limit cover over its window's body. The frame, title bar and lights are drawn
    /// before this, so the window can still be moved or closed. Clipped to the surface.
    fn desk_cover(&mut self, s: &mut Surface, w: &Win, m: &Metrics, t: f64) {
        let top = w.title_h(m);
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
            // an app drawing its first frame into a new buffer set (a live resize) stays mapped (`between_buffers`)
            AppId::External(slot) => self.ext.get(slot as usize).and_then(Option::as_ref).is_some_and(|h| !h.incompatible && (!h.running() || (h.frame().is_none() && !h.between_buffers()))),
            AppId::ExternalWindow(slot, id) => self.ext.get(slot as usize).and_then(Option::as_ref).map_or(true, |h| !h.window_mapped(u32::from(id))),
            _ => false,
        }
    }

    /// True when `app`'s window waits for the first frame of an app that is running: a launch (or a new extra window) in
    /// progress. A window whose process is gone is not launching.
    #[cfg(feature = "desktop-wayland")]
    pub(super) fn desk_window_launching(&self, app: AppId) -> bool {
        match app {
            AppId::External(slot) => self.ext.get(slot as usize).and_then(Option::as_ref).is_some_and(|h| h.running() && !h.incompatible && h.frame().is_none() && !h.between_buffers()),
            AppId::ExternalWindow(slot, id) => self.ext.get(slot as usize).and_then(Option::as_ref).is_some_and(|h| h.running() && !h.window_mapped(u32::from(id))),
            _ => false,
        }
    }

    /// True when the shown windows above window `i` are all still launching (`desk_window_launching`), at least one of
    /// them: the window server has no toplevel for them yet, so `i` is the front it knows, and a focus it hands back to
    /// `i` (Spotlight or the Apps grid closing, a menu closing) is not a choice of the person to put `i` over the app
    /// that was just opened (E2E6 1).
    #[cfg(feature = "desktop-wayland")]
    pub(super) fn desk_launching_above(&self, i: usize) -> bool {
        let Some(d) = self.desk.as_ref() else { return false };
        let mut above = d.wins.iter().skip(i + 1).filter(|w| !w.minimized).peekable();
        above.peek().is_some() && above.all(|w| self.desk_window_launching(w.app))
    }

    /// The app the shell is bringing to the front (`Desk::front_intent`) while the intent stands: its window (or one of
    /// its extra windows) is still the shell's front window. Anything else that put another window in front (a press,
    /// another open, a close) ended it.
    pub(super) fn desk_front_intent(&self) -> Option<AppId> {
        let d = self.desk.as_ref()?;
        let intent = d.front_intent?;
        // an app whose process is gone cannot be brought to the front: the focus goes to the window under it
        let alive = match intent.app.parent() {
            AppId::External(slot) => self.ext.get(slot as usize).and_then(Option::as_ref).is_some_and(|h| h.running()),
            _ => true,
        };
        (alive && d.front().map(|w| w.app.parent()) == Some(intent.app.parent())).then_some(intent.app)
    }

    /// Per tick: ends the front intent once it no longer stands or after `FRONT_INTENT_MS`; while it stands and the
    /// window server's toplevel list changed, a drawn front window of the app that the list does not show active is
    /// activated again (`raise_front`: xdg-activation and the foreign-toplevel activate with the next frame).
    pub(super) fn desk_intent_tick(&mut self, t: f64) {
        let Some(intent) = self.desk.as_ref().and_then(|d| d.front_intent) else { return };
        let ended = if self.desk_front_intent().is_none() {
            Some("another window is in front")
        } else if t - intent.since > FRONT_INTENT_MS {
            Some("the bound ran out")
        } else {
            None
        };
        if let Some(why) = ended {
            if let Some(d) = self.desk.as_mut() { d.front_intent = None; }
            crate::bare::focus_trace::log(format_args!("intent app={:?} ends: {why}", intent.app));
            return;
        }
        #[cfg(feature = "desktop-wayland")]
        {
            let Some(front) = self.desk.as_ref().and_then(|d| d.front()).map(|w| w.app) else { return };
            if self.desk_window_unmapped(front) { return; }
            let Some(id) = win_id(front) else { return };
            let app_id = format!("is.olibuijr.bifrost.{id}");
            let Some(d) = self.desk.as_mut() else { return };
            if !std::mem::take(&mut d.intent_check) { return; }
            if let Some(toplevel) = d.foreign_snapshot.iter().find(|w| w.app_id == app_id && !w.active).map(|w| w.id) {
                crate::bare::focus_trace::log(format_args!("intent app={:?} window={id} (toplevel {toplevel}) is mapped but not active: activated again", intent.app));
                d.raise_front = true;
                self.dirty = true;
            }
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
            // keyed exactly as the windows are drawn below (`win_id(app).unwrap_or(0)`): an app with no catalogue id is
            // drawn as 0, and `frame_app(0)` names no app, so matching on it dropped that window's cache every frame
            desk.window_cache.retain(|id,_| desk.wins.iter().zip(&fading).any(|(w, f)| (!w.minimized || *f) && win_id(w.app).unwrap_or(0) == *id));
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
                let refocus = !unchanged && w.is_full_size() && matches!(w.app, AppId::External(_)) && cached.as_ref().is_some_and(|c|
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
                        drag_height, no_drag, fx:bifrost_render::glass::Fx::IDENTITY, opaque, floating:false, origin:None,position:None, min_size:(0,0),input:None,
                    });
                    continue;
                }
                if unchanged && quiet {
                    self.desk.as_mut().unwrap().frames.push(crate::bare::wayland::WindowFrame {
                        id, title: frame_title(id), x:w.r.x, y:w.r.y,
                        width:w.r.w as usize, height:w.r.h as usize, pixels:None,
                        damage:crate::gfx::Damage::new(w.r.w as usize,w.r.h as usize), maximized:w.maximized, fullscreen:w.full, title_height:m.title,
                        drag_height, no_drag, fx:bifrost_render::glass::Fx::IDENTITY, opaque, floating:false, origin:None,position:None, min_size:(0,0),input:None,
                    });
                    self.desk.as_mut().unwrap().window_cache.insert(id,cached.unwrap());
                    continue;
                }
                if let AppId::External(slot)=w.app {
                    let width=w.r.w as usize; let height=w.r.h as usize; let top=w.title_h(&m) as usize;
                    let body_h=height.saturating_sub(top);
                    let mapped=self.ext.get(slot as usize).and_then(Option::as_ref).filter(|h|h.running()).and_then(|h|h.frame()).map(|(_,w,h)|(w,h));
                    // on a key-state change the app's last frame may still be a few pixels larger than the window (the
                    // size it first drew at); `blit` clips it to the body exactly as the full draw would, so it is taken
                    let fits = mapped==Some((width,body_h)) || (refocus && mapped.is_some_and(|(mw,mh)| mw>=width && mh>=body_h));
                    // the app reallocated its buffers (each step of a live resize does) and has not drawn into them yet:
                    // the window keeps the pixels it last presented, at their size, until the app's frame arrives. It used
                    // to be dropped from the frames, which destroyed its toplevel: the pointer left it, the resize grab
                    // ended after a few pixels (E2E-11) and the window behind took the focus (E2E-12)
                    if mapped.is_none() && self.ext.get(slot as usize).and_then(Option::as_ref).is_some_and(|h| h.between_buffers()) {
                        if let Some(c) = cached.take() {
                            let (cw, ch) = (c.width, c.height);
                            let desk = self.desk.as_mut().unwrap();
                            desk.window_cache.insert(id, c);
                            desk.frames.push(crate::bare::wayland::WindowFrame {
                                id, title: frame_title(id), x:w.r.x, y:w.r.y, width:cw, height:ch, pixels:None,
                                damage:crate::gfx::Damage::new(cw,ch), maximized:w.maximized, fullscreen:w.full, title_height:m.title,
                                drag_height, no_drag, fx:bifrost_render::glass::Fx::IDENTITY, opaque, floating:false, origin:None,position:None, min_size:(0,0),input:None,
                            });
                            continue;
                        }
                    }
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
                        // a borderless window is the app's pixels alone: no dim, rim, corner clip or lights (its notes, or
                        // whatever it draws, carry their own edges)
                        let bare = borderless(w.app);
                        if !focused && !bare { for a in body_damage.areas() { dim_rect(pixels, width, a.x0, a.y0 + top, a.x1, a.y1 + top, BACKGROUND_DIM); } }
                        let src=self.ext.get(slot as usize).and_then(Option::as_ref).and_then(|h|h.frame());
                        let dim=(!focused && !bare).then_some(BACKGROUND_DIM);
                        // the app drew into the window's edge band: the band is taken again from the app's frame and the
                        // dark-mode inner rim and the corner clip are applied once over it (stroking the retained buffer
                        // stacked 15% white per repaint into a solid white border, E2E-10)
                        let mut edge:Vec<(i32,i32,i32,i32)>=Vec::new();
                        if !w.maximized && !bare && body_damage.areas().iter().any(|a| edge_band_rects(width,height,corner).iter().any(|r| overlaps(r,a,top))) {
                            if let Some(src)=src { edge=restore_window_edge(pixels,width,height,top,src,dim,corner,window_rim_color()); }
                            for r in &edge {damage.add(r.0,r.1,r.2,r.3);}
                        }
                        if w.is_full_size() && !bare {
                            // the lights sit over the app: repaint them where the app just drew under them
                            let mut local=*w; local.r.x=0; local.r.y=0;
                            let mut surf=Surface{w:width,h:height,stride:width,px:&mut pixels[..],y0:0};
                            for (k,c) in [SYSTEM_RED, SYSTEM_YELLOW, SYSTEM_GREEN].into_iter().enumerate() {
                                let lr=local.light_rect_px(&m,k);
                                let under=body_damage.areas().iter().any(|a| (a.x0 as i32) < lr.x+lr.w && (a.x1 as i32) > lr.x && ((a.y0+top) as i32) < lr.y+lr.h && ((a.y1+top) as i32) > lr.y);
                                let in_edge=edge.iter().any(|r| r.0 < lr.x+lr.w+2 && r.0+r.2 > lr.x-2 && r.1 < lr.y+lr.h+2 && r.1+r.3 > lr.y-2);
                                if !under && !in_edge { continue; }
                                // the light is drawn over the app's own pixels, never over the light of an earlier frame
                                // (its antialiased edge would darken a little more on every repaint)
                                if let Some(src)=src { restore_from_frame(surf.px,width,height,top,src,dim,lr.x-2,lr.y-2,lr.x+lr.w+2,lr.y+lr.h+2); }
                                draw_light(&mut surf,&lr,if focused {c} else {TRAFFIC_INACTIVE_C.now()},focused,self.ui.pt(1.));
                                if focused && hover { draw_light_glyph(&mut surf,&lr,k,w.maximized,self.ui.pt(1.)); }
                                damage.add(lr.x-2,lr.y-2,lr.w+4,lr.h+4);
                            }
                        }
                        retained.drawn=t;
                        let outgoing=(!damage.is_empty()).then(||retained.pixels.clone());
                        let desk=self.desk.as_mut().unwrap();
                        desk.body_cache.remove(&w.app);
                        desk.window_cache.insert(id,retained);
                        desk.frames.push(crate::bare::wayland::WindowFrame{id,title:frame_title(id),x:w.r.x,y:w.r.y,width,height,pixels:outgoing,damage,maximized:w.maximized,fullscreen:w.full,title_height:m.title,drag_height,no_drag,fx:bifrost_render::glass::Fx::IDENTITY,opaque,floating:false,origin:None,position:None,min_size:(0,0),input:None});
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
                if !w.maximized && !borderless(w.app) { clip_corners(&mut image,corner); }
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
                    drag_height, no_drag, fx:bifrost_render::glass::Fx::IDENTITY, opaque, floating:false, origin:None,position:None, min_size:(0,0),input:None,
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
        let bare = borderless(w.app);
        let rad = if w.maximized || w.full || bare { 0.0 } else { ui.pt(bifrost_ui::prefs_layout::m::WINDOW_RADIUS) };
        // shadow (CPU path; the GPU presenter draws its own): Golden Gate's key window casts a
        // large soft shadow (~40 pt blur, ~16 pt down, ~30% under the bottom edge), others a light one
        if !w.maximized && shadow && !bare {
            cached_window_shadow(s, ui, r.x as f32, r.y as f32, r.w as f32, r.h as f32, rad, focused);
        }
        let bar = if focused { super::ui::WINDOW_FRAME_FOCUSED.now() } else { super::ui::WINDOW_FRAME_BLURRED.now() };
        if !bare { s.fill_rrect(r.x as f32, r.y as f32, r.w as f32, r.h as f32, rad, bar); }
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
        if w.is_full_size() && rad > 0.0 { copy_rect_rounded(s, &buf, bw, bh, b.x, b.y, rad); } else { copy_rect(s, &buf, bw, bh, b.x, b.y); }
        if !focused && !bare { dim_rect(s.px, s.stride, b.x.max(0) as usize, (b.y.max(0) as usize).saturating_sub(s.y0), ((b.x + b.w).max(0) as usize).min(s.w), ((b.y + b.h).max(0) as usize).saturating_sub(s.y0).min(s.h), BACKGROUND_DIM); }
        #[cfg(feature = "desktop-wayland")]
        if persistent || builtin { self.desk.as_mut().unwrap().body_cache.insert(w.app,buf); }
        else { self.desk.as_mut().unwrap().buf = buf; }
        #[cfg(not(feature = "desktop-wayland"))]
        { self.desk.as_mut().unwrap().buf = buf; }
        let ui = &self.ui;
        // a full-screen window is the app alone: no rim, title or traffic lights; so is a borderless one
        if w.full || bare { return; }
        // hairline under the title bar and around the window
        if !w.maximized { window_rim(s, r.x as f32, r.y as f32, r.w as f32, r.h as f32, rad); }
        if !w.is_full_size() { s.fill_rect(r.x.max(0) as usize, (r.y + m.title - 1).max(0) as usize, r.w.max(0) as usize, 1, super::ui::SEPARATOR_C.now()); }
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
        if w.is_full_size() { return; }
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

    pub(super) fn draw_desk_dock(&self, s: &mut Surface, m: &Metrics, t: f64) {
        if !self.dock_is_visible(){self.note_dock_glass(None);return;}
        let (list, bar0, rects0) = self.desk_dock(m);
        // autohide slide, then magnification around the pointer (both pure functions of the clock)
        let (bar, slid) = self.dock_slid(bar0, &rects0, t / 1000.0);
        let rects = self.dock_magnified(&slid, t / 1000.0);
        // the plate grows along the axis with the magnified row, so the end icons stay on it (W2-24)
        let bar = super::desktop_dock::dock_plate(bar, &slid, &rects, self.dock_preferences().position != super::desktop_dock::Position::Bottom);
        self.note_dock_glass(bifrost_render::glass::gpu().then_some(bar));
        let ui = &self.ui;
        // frosted material: blur what is behind the dock, tint it, and edge it with a hairline
        let (bx, by) = (bar.x.max(0) as usize, bar.y.max(0) as usize);
        let (bw, bh) = ((bar.w as usize).min(s.w.saturating_sub(bx)), (bar.h as usize).min(s.h.saturating_sub(by)));
        if bw > 0 && bh > 0 && bifrost_render::appearance::reduce_transparency() {
            // Reduce Transparency: an opaque plate, nothing blurred or seen through it; Increase Contrast outlines it
            let rad = bar.w.min(bar.h) as f32 * DOCK_RADIUS_K;
            soft_shadow(s, ui, bar.x as f32, bar.y as f32, bar.w as f32, bar.h as f32, rad);
            s.fill_rrect(bar.x as f32, bar.y as f32, bar.w as f32, bar.h as f32, rad, super::ui::DOCK_SOLID.now());
            match bifrost_render::ui::material_border() {
                Some(b) => s.stroke_rrect(bar.x as f32, bar.y as f32, bar.w as f32, bar.h as f32, rad, ui.pt(1.0).max(1.0), b),
                None => dock_rim(s, bar, rad),
            }
        } else if bw > 0 && bh > 0 && bifrost_render::glass::gpu() {
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
            // placed between the icons as drawn (magnified), at the resting icons' depth
            let (a, b) = if slid[k].y == slid[k + 1].y {
                (Rect { x: rects[k].x, w: rects[k].w, ..slid[k] }, Rect { x: rects[k + 1].x, ..slid[k + 1] })
            } else {
                (Rect { y: rects[k].y, h: rects[k].h, ..slid[k] }, Rect { y: rects[k + 1].y, ..slid[k + 1] })
            };
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
        // the hover label fades in and out (Reduce Motion: it just appears); an open Dock menu or stack hides it, as the
        // reference does, so it never shows through the stack (E2E8 7)
        let (label_slot, label_alpha) = self.dock_label_state(t);
        if let Some(k) = label_slot.filter(|_| desk.grab.is_none() && !self.dock_label_hidden_by_menu()) {
            if let (Some(r), Some(&i)) = (rects.get(k), list.get(k)) {
                let name = self.dock_label_text(i);
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
        let px = ui.pt(super::desktop_dock::DOCK_LABEL_PAD_X_PT);
        let gap = ui.pt(super::desktop_dock::DOCK_LABEL_GAP_PT);
        let room = match pos {
            super::desktop_dock::Position::Bottom => s.w as f32 - 2.0 * px,
            super::desktop_dock::Position::Left => s.w as f32 - (r.x + r.w) as f32 - gap - 2.0 * px,
            super::desktop_dock::Position::Right => r.x as f32 - gap - 2.0 * px,
        };
        let name = ui.fit(name, MAC_CALLOUT_PT, ui.pt(super::desktop_dock::DOCK_LABEL_MAX_WIDTH_PT).min(room.max(1.0)));
        self.label(&name, MAC_CALLOUT_PT, Weight::Regular, WHITE, |im| {
            let (px, py) = (ui.pt(super::desktop_dock::DOCK_LABEL_PAD_X_PT), ui.pt(super::desktop_dock::DOCK_LABEL_PAD_Y_PT));
            let (w, h) = (im.w as f32 + 2.0 * px, im.h as f32 + 2.0 * py);
            let (x, y) = match pos {
                super::desktop_dock::Position::Bottom => (((r.x + r.w / 2) as f32 - w / 2.0).clamp(0.0, (s.w as f32 - w).max(0.0)), r.y as f32 - h - ui.pt(super::desktop_dock::DOCK_LABEL_GAP_PT)),
                super::desktop_dock::Position::Left => ((r.x + r.w) as f32 + ui.pt(super::desktop_dock::DOCK_LABEL_GAP_PT), (r.y + r.h / 2) as f32 - h / 2.0),
                super::desktop_dock::Position::Right => (r.x as f32 - w - ui.pt(super::desktop_dock::DOCK_LABEL_GAP_PT), (r.y + r.h / 2) as f32 - h / 2.0),
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
    fn quit_children_are_reaped_and_running_ones_kept() {
        let mut children = std::collections::HashMap::new();
        children.insert("quit".to_string(), std::process::Command::new("true").spawn().unwrap());
        children.insert("running".to_string(), std::process::Command::new("sleep").arg("30").spawn().unwrap());
        // `true` exits at once; give it time to become a zombie
        std::thread::sleep(std::time::Duration::from_millis(500));
        reap_exited(&mut children);
        assert_eq!(children.keys().collect::<Vec<_>>(), vec!["running"], "the zombie is waited for and dropped");
        let mut running = children.remove("running").unwrap();
        let _ = running.kill();
        let _ = running.wait();
    }

    #[test]
    fn a_full_size_app_drags_from_its_empty_toolbar_band_but_not_its_controls_or_lights() {
        let m = Metrics { pt: 1.5, ..metrics() };
        let r = Rect { x: 100, y: 50, w: 900, h: 600 };
        let w = Win { app: AppId::External(3), r, maximized: false, minimized: false, full: false, restore: r, full_size: false };
        let node = |x: f32, y: f32| bifrost_proto::Node { id: 1, role: bifrost_proto::Role::Button, label: "Back".into(), value: String::new(), bounds: bifrost_ui::Rect { x, y, w: 64.0, h: 32.0 }, enabled: true };
        let (band, holes) = zone_of(&w, &m, &[node(220.0, 10.0), node(220.0, 300.0)], true);
        assert_eq!(band, (UNIFIED_BAR_PT * 1.5).round() as i32, "the unified bar is the drag band");
        assert_eq!(holes.len(), 2, "the lights and the one control inside the band");
        assert_eq!(holes[1], [330, 15, 96, 48], "control bounds scale from points to window px");
        assert!(holes[0][0] == 0 && holes[0][2] > 0, "the lights are not a drag area");
        // a normal window drags from its title bar, except over the three lights: a press there must reach the button
        // (the window-move threshold used to swallow a press on the green one)
        let (title, lights) = zone_of(&w, &m, &[node(220.0, 10.0)], false);
        assert_eq!(title, m.title, "a normal window drags from its title bar only");
        assert_eq!(lights.len(), 1, "the one hole is the traffic lights; the toolbar controls of a normal window are not holes");
        let hole = lights[0];
        for i in 0..3 {
            let l = w.light_rect(&m, i);
            let (x, y) = (l.x - w.r.x, l.y - w.r.y);
            assert!(x >= hole[0] && x + l.w <= hole[0] + hole[2] && y >= hole[1] && y + l.h <= hole[1] + hole[3], "light {i} at {x},{y} is inside the hole {hole:?}");
        }
    }

    #[test]
    fn settings_drags_from_its_empty_toolbar_but_not_from_the_pill_or_search_field() {
        use bifrost_ui::prefs_layout::m as pm;
        let m = Metrics { pt: 1.5, ..metrics() };
        let r = Rect { x: 100, y: 50, w: (740.0 * 1.5) as i32, h: (690.0 * 1.5) as i32 };
        let w = Win { app: AppId::Settings, r, maximized: false, minimized: false, full: false, restore: r, full_size: false };
        let (band, holes) = drag_zone(&w, &m, &[]);
        assert_eq!(band, (UNIFIED_BAR_PT * 1.5).round() as i32, "Settings has the full-size toolbar band, not the title-bar one");
        let px = |v: f32| (v * 1.5) as i32;
        let pill_x = pm::SIDEBAR_W + pm::TOOLBAR_X;
        let pill_mid_y = pm::TOOLBAR_Y + pm::TOOLBAR_PILL_H / 2.0;
        // both halves of the pill, including their upper part that the old title band covered
        for (x, y) in [(pill_x + 10.0, pill_mid_y), (pill_x + pm::TOOLBAR_PILL_W - 10.0, pill_mid_y), (pill_x + 10.0, pm::TOOLBAR_Y + 2.0), (pill_x + 50.0, pm::TOOLBAR_Y + 4.0)] {
            assert!(!drags(band, &holes, px(x), px(y)), "({x},{y}) is on the pill");
            let hit = w.hit(&m, (r.x + px(x)) as f32, (r.y + px(y)) as f32);
            assert_eq!(hit, Some(Hit::Content), "({x},{y}) reaches Settings");
        }
        // empty toolbar space right of the pill and above the sidebar search field moves the window
        for (x, y) in [(pill_x + pm::TOOLBAR_PILL_W + 200.0, pill_mid_y), (pill_x + pm::TOOLBAR_PILL_W + 200.0, 4.0), (150.0, 45.0)] {
            assert!(drags(band, &holes, px(x), px(y)), "({x},{y}) drags");
            assert_eq!(w.hit(&m, (r.x + px(x)) as f32, (r.y + px(y)) as f32), Some(Hit::Title), "({x},{y}) is title");
        }
        // the search field and the content below the band are Settings'
        let s = pm::SEARCH_Y + pm::SEARCH_H / 2.0;
        assert_eq!(w.hit(&m, (r.x + px(100.0)) as f32, (r.y + px(s)) as f32), Some(Hit::Content));
        assert_eq!(w.hit(&m, (r.x + px(500.0)) as f32, (r.y + px(200.0)) as f32), Some(Hit::Content));
        // the traffic lights stay lights
        let (cx, cy) = bifrost_ui::prefs_layout::Frame::traffic_light(0);
        assert_eq!(w.hit(&m, (r.x + px(cx)) as f32, (r.y + px(cy)) as f32), Some(Hit::Light(Light::Close)));
        assert!(holes[0][0] == 0 && holes[0][2] > 0, "the lights are a hole");
    }

    /// E2E9 4: an extra window opened with `window_flags::FULL_SIZE` (a second TextEdit document) has no shell title row:
    /// its body is the whole window under the lights, and its outer size is the size the app asked for. A plain extra
    /// window keeps the shell's title bar.
    #[test]
    fn a_full_size_extra_window_has_no_shell_title_row() {
        let m = Metrics { pt: 1.5, ..metrics() };
        let (doc, plain) = (AppId::ExternalWindow(3, 2), AppId::ExternalWindow(3, 4));
        assert!(!full_size(doc) && !full_size(plain), "an extra window's id alone does not say: the window does");
        let r = Rect { x: 100, y: 50, w: 900, h: 700 };
        let w = Win { app: doc, r, maximized: false, minimized: false, full: false, restore: r, full_size: true };
        let p = Win { app: plain, full_size: false, ..w };
        assert!(w.is_full_size());
        assert!(!p.is_full_size(), "a window that did not ask keeps its title bar");
        assert_eq!(w.title_h(&m), 0);
        assert_eq!(p.title_h(&m), m.title);
        assert_eq!(w.body(&m), r, "the app draws the whole window, its own title row included");
        assert_eq!(extra_window_px(doc, true, (600.0, 400.0), 1.5, &m).1, 600, "no title bar is added to the asked height");
        assert_eq!(extra_window_px(plain, false, (600.0, 400.0), 1.5, &m).1, 600 + m.title);
    }

    /// E2E10: a full-size extra window takes its flag from its app host's window table when the desk opens it, so its
    /// unified title band (no shell title bar) is hit right away, before any later sync; a plain one keeps the title bar.
    #[test]
    fn a_full_size_extra_window_hits_its_title_band_right_after_it_opens() {
        let mut sh = crate::shell::tests::desktop_shell();
        sh.anim = None;
        let (mine, _theirs) = bifrost_proto::sys::socketpair_pair().unwrap();
        let cfg = bifrost_proto::Configure { w: 100, h: 100, scale: 1., safe_top: 0., safe_bottom: 0., dark: true, text_scale: 1., reduce_motion: false };
        let mut host = apphost::AppHost::with_channel("fixture", mine, None, cfg);
        host.windows.open("fixture", 2, "Untitled 2", 600.0, 400.0).unwrap();
        assert!(host.windows.set_full_size("fixture", 2, true));
        host.windows.open("fixture", 4, "Plain", 300.0, 200.0).unwrap();
        sh.ext.push(Some(host));
        let slot = (sh.ext.len() - 1) as u8;
        let (doc, plain) = (AppId::ExternalWindow(slot, 2), AppId::ExternalWindow(slot, 4));
        sh.desk.as_mut().unwrap().wins.clear();
        sh.desk_sync_extra_windows();
        let m = sh.desk_metrics();
        let d = sh.desk.as_ref().unwrap();
        let w = d.wins[d.index_of(doc).expect("the desk opened the document window")];
        let p = d.wins[d.index_of(plain).expect("and the plain one")];
        assert!(w.full_size && w.is_full_size() && !p.is_full_size());
        assert_eq!(w.title_h(&m), 0, "no shell title row");
        // below the shell's title bar height but inside the unified band (52 pt), past the traffic lights
        let (x, y) = ((w.r.x + w.r.w / 2) as f32, w.r.y as f32 + m.title as f32 + 2.0);
        assert!(y < w.r.y as f32 + UNIFIED_BAR_PT * m.pt);
        assert_eq!(w.hit(&m, x, y), Some(Hit::Title), "the unified band moves the window");
        assert_eq!(w.hit(&m, x, w.r.y as f32 + UNIFIED_BAR_PT * m.pt + 4.0), Some(Hit::Content));
        let (px_, py) = ((p.r.x + p.r.w / 2) as f32, p.r.y as f32 + m.title as f32 + 2.0);
        assert_eq!(p.hit(&m, px_, py), Some(Hit::Content), "a titled window's body starts under its title bar");
    }

    #[test]
    fn the_app_store_is_a_full_size_window_whose_search_field_and_back_control_take_the_press() {
        // a sidebar app: no title bar, the sidebar runs to the top under the traffic lights (N8)
        assert!(full_size(AppId::AppStore));
        let m = Metrics { pt: 1.5, ..metrics() };
        assert_eq!(title_of(AppId::AppStore, &m), 0, "no title bar above the sidebar");
        let r = Rect { x: 100, y: 50, w: 1500, h: 1000 };
        let w = Win { app: AppId::AppStore, r, maximized: false, minimized: false, full: false, restore: r, full_size: false };
        assert_eq!(w.body(&m), r, "the store draws the whole window");
        let px = |v: f32| (v * 1.5) as i32;
        let side = ((200.0 * 1.5) as i32).min(r.w / 3);
        // no page open: no back control, so the whole band over the pane drags
        super::super::store::set_back_control_pt(0.0);
        let (band, holes) = drag_zone(&w, &m, &[]);
        assert_eq!(holes.len(), 2, "the lights and the sidebar search field");
        assert!(drags(band, &holes, side + px(20.0), px(26.0)), "with no back control shown its place moves the window");
        // a page is open: its back control (90 pt drawn) takes the press, and only it
        super::super::store::set_back_control_pt(90.0);
        let (band, holes) = drag_zone(&w, &m, &[]);
        assert_eq!(band, (UNIFIED_BAR_PT * 1.5).round() as i32, "the unified toolbar band drags");
        assert_eq!(holes.len(), 3, "the lights, the sidebar search field and the back control");
        assert!(drags(band, &holes, side + px(100.0), px(26.0)), "past the back control the band drags");
        // the search capsule's top edge is inside the band and takes the press; the empty band over the pane drags
        assert!(!drags(band, &holes, px(60.0), px(48.0)), "the search field is not a drag area");
        assert!(!drags(band, &holes, side + px(20.0), px(26.0)), "the back control is not a drag area");
        assert!(drags(band, &holes, side + px(400.0), px(26.0)), "the empty band over the pane moves the window");
        assert_eq!(w.hit(&m, (r.x + side + px(20.0)) as f32, (r.y + px(26.0)) as f32), Some(Hit::Content));
        assert_eq!(w.hit(&m, (r.x + side + px(400.0)) as f32, (r.y + px(26.0)) as f32), Some(Hit::Title));
        let (cx, cy) = bifrost_ui::prefs_layout::Frame::traffic_light(0);
        assert_eq!(w.hit(&m, (r.x + px(cx)) as f32, (r.y + px(cy)) as f32), Some(Hit::Light(Light::Close)));
        super::super::store::set_back_control_pt(0.0);
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
        let w = Win { app: AppId::External(3), r, maximized: false, minimized: false, full: false, restore: r, full_size: false };
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
        let w = Win { app: AppId::External(18), r, maximized: false, minimized: false, full: false, restore: r, full_size: false };
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
        assert_eq!((title, none.len()), (m.title, 1), "a window without the style drags from its title bar, but not from the lights");
        assert!(!drags(title, &none, 12 + 2 * (m.light + m.light_gap) + m.light / 2, m.title / 2), "the green light is not a drag");
        assert!(drags(title, &none, 600, m.title / 2), "past the lights the title bar drags");
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
        Metrics { w: 1920, h: 1080, title: 30, menubar: 28, min_w: 320, min_h: 220, edge: 6, light: 12, light_gap: 8, light_x: 12, dock_left: 0, dock_right: 0, dock_bottom: 100, pt: 1.0 }
    }

    #[test]
    fn closing_a_full_screen_window_forgets_that_it_came_from_a_zoom() {
        let m = metrics();
        let app = AppId::ExternalWindow(0, 4);
        let mut d = Desk::new();
        let i = d.open(&m, app, (500, 400));
        d.toggle_zoom(&m, i);
        assert!(d.wins[i].maximized);
        assert!(d.set_full_screen(&m, i, true));
        assert_eq!(d.full_from_zoom, vec![app]);
        d.close(i);
        assert!(d.full_from_zoom.is_empty(), "the entry goes with the window");
        let i = d.open(&m, app, (500, 400));
        let before = d.wins[i].r;
        assert!(d.set_full_screen(&m, i, true));
        assert!(d.set_full_screen(&m, i, false));
        assert!(!d.wins[i].maximized, "the new window returns to its own frame, not a zoom");
        assert_eq!(d.wins[i].r, before);
    }

    #[test]
    fn an_extra_window_is_never_opened_under_the_window_minimum() {
        let m = metrics();
        let app = AppId::ExternalWindow(0, 4);
        assert_eq!(extra_window_px(app, false, (0.1, 0.1), 1.0, &m), (m.min_w, m.min_h + m.title));
        assert_eq!(extra_window_px(app, false, (f32::NAN, -5.0), 1.0, &m), (m.min_w, m.min_h + m.title));
        assert_eq!(extra_window_px(app, false, (600.0, 400.0), 1.5, &m), (900, 600 + m.title));
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

    /// F4: a viewer opens beside the main window, or over its trailing side, never over the leading list column; each
    /// later viewer cascades down and right of the one before it, and every one stays inside the work area.
    #[test]
    fn a_cascaded_window_leaves_the_main_windows_leading_side_in_view() {
        let a = Rect { x: 0, y: 28, w: 1920, h: 952 };
        let step = 30;
        let narrow = Rect { x: 100, y: 100, w: 800, h: 600 };
        assert_eq!(cascade_origin(a, narrow, true, (560, 640), step), (900, 100), "beside the main window when it fits");
        let wide = Rect { x: 200, y: 80, w: 1500, h: 800 };
        let (x, y) = cascade_origin(a, wide, true, (560, 640), step);
        assert_eq!((x, y), (200 + 1500 - 560, 80 + step), "over the trailing side, one title bar down");
        assert!(x > wide.x + wide.w / 2, "the leading half (sidebar and list) stays in view");
        let first = Rect { x, y, w: 560, h: 640 };
        assert_eq!(cascade_origin(a, first, false, (560, 640), step), (x + step, y + step), "the next one cascades from the first");
        let corner = Rect { x: 1800, y: 900, w: 560, h: 640 };
        let (cx, cy) = cascade_origin(a, corner, false, (560, 640), step);
        assert!(cx + 560 <= a.x + a.w && cy + 640 <= a.y + a.h, "kept inside the work area: {cx},{cy}");
    }

    /// E2E11 5: a second document window as large as the main window (Cmd+N in a text editor) cascades one title bar
    /// down and right of it, not straight below it, whether or not it would fit beside it.
    #[test]
    fn a_document_window_as_wide_as_the_main_window_cascades_down_and_right() {
        let a = Rect { x: 0, y: 28, w: 1920, h: 952 };
        let step = 30;
        let main = Rect { x: 400, y: 200, w: 900, h: 600 };
        assert_eq!(cascade_origin(a, main, true, (900, 600), step), (430, 230), "down and right by one title bar");
        assert_eq!(cascade_origin(a, main, true, (890, 600), step), (430, 230), "a few px narrower is the same size");
        let narrow = Rect { x: 100, y: 100, w: 600, h: 500 };
        assert_eq!(cascade_origin(a, narrow, true, (600, 500), step), (130, 130), "it fits beside, but cascades as a document");
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
    fn hide_puts_away_a_whole_app_and_raising_it_brings_it_back() {
        let m = metrics();
        let mut d = Desk::new();
        d.open(&m, AppId::Photos, (900, 600));
        d.open(&m, AppId::External(3), (900, 600));
        d.open(&m, AppId::ExternalWindow(3, 7), (400, 300));
        d.open(&m, AppId::Terminal, (900, 600));
        assert!(d.hide_app(AppId::External(3)), "the app had windows to hide");
        assert!(d.is_hidden(AppId::External(3)) && d.is_hidden(AppId::ExternalWindow(3, 7)), "its extra window goes with it");
        assert!(!d.is_hidden(AppId::Photos) && !d.is_hidden(AppId::Terminal));
        assert!(!d.hide_app(AppId::External(3)), "nothing left to hide");
        assert!(d.others_shown(Some(AppId::Terminal)), "Photos is still shown");
        // raising one of its windows shows the whole app again
        let i = d.index_of(AppId::External(3)).unwrap();
        d.raise(i);
        assert!(!d.any_hidden(), "the extra window came back too");
        assert!(d.hidden.is_empty());
    }

    /// E2E N2: after Cmd+H the Dock shows Mail again with its viewer over the main window, as it was when hidden, so the
    /// viewer keeps the keyboard and Cmd+W closes it; the Window menu choosing the main window still brings that one up.
    #[test]
    fn a_hidden_app_comes_back_with_its_front_window_in_front() {
        let m = metrics();
        let mut d = Desk::new();
        let (main, viewer) = (AppId::External(3), AppId::ExternalWindow(3, 2));
        d.open(&m, main, (900, 600));
        d.open(&m, viewer, (500, 400));
        d.open(&m, AppId::Terminal, (900, 600));
        assert!(d.hide_app(main));
        let i = d.open(&m, main, (900, 600));
        assert_eq!(d.wins[i].app, viewer, "the window that was in front comes back in front");
        assert_eq!(d.front().map(|w| w.app), Some(viewer));
        assert_eq!(d.wins.iter().map(|w| w.app).collect::<Vec<_>>(), vec![AppId::Terminal, main, viewer], "over the other apps, in their own order");
        assert!(d.hidden.is_empty() && !d.any_hidden());
        assert!(d.hide_app(main));
        let i = d.index_of(main).unwrap();
        let at = d.raise(i);
        assert_eq!(d.wins[at].app, main, "a raise of one window of a hidden app brings that window up");
        assert_eq!(d.wins.iter().map(|w| w.app).collect::<Vec<_>>(), vec![AppId::Terminal, viewer, main]);
    }

    /// E2E N7: a window the restored session put somewhere maps there, so the frame the shell keeps is the one on
    /// screen and the window server never cascades it later (a Dock raise moved Terminal from 768,352 to 880,465).
    #[test]
    fn a_window_placed_on_purpose_maps_where_it_was_placed() {
        let m = metrics();
        let mut d = Desk::new();
        let i = d.open(&m, AppId::Terminal, (900, 600));
        assert!(!d.placed.contains(&AppId::Terminal), "a new window is cascaded by the window server");
        let a = Desk::area(&m);
        d.place(&m, i, Rect { x: a.x + 100, y: a.y + 50, w: 900, h: 600 });
        assert_eq!(origin_hint(&d.placed, AppId::Terminal, d.wins[i].r.x, d.wins[i].r.y), Some((a.x + 100, a.y + 50)));
        let i = d.raise(i);
        assert_eq!((d.wins[i].r.x, d.wins[i].r.y), (a.x + 100, a.y + 50), "a raise moves nothing");
    }

    /// E2E N2: the Dock activating an app whose viewer is in front keeps the viewer in front (it does not raise the
    /// main window over it); an app showing only its main window goes the usual open path.
    #[test]
    fn the_dock_activates_an_app_with_its_front_window_in_front() {
        let mut sh = crate::shell::tests::desktop_shell();
        let m = sh.desk_metrics();
        let (main, viewer) = (AppId::External(3), AppId::ExternalWindow(3, 2));
        {
            let d = sh.desk.as_mut().unwrap();
            d.wins.clear();
            d.open(&m, main, (900, 600));
            d.open(&m, viewer, (500, 400));
            d.open(&m, AppId::Terminal, (900, 600));
        }
        assert!(sh.desk_activate_shown_app(main, 0.0));
        let d = sh.desk.as_ref().unwrap();
        assert_eq!(d.wins.iter().map(|w| w.app).collect::<Vec<_>>(), vec![AppId::Terminal, main, viewer]);
        assert!(sh.take_raise_front(), "the window server raises and focuses the viewer");
        assert!(!sh.desk_activate_shown_app(AppId::Terminal, 0.0), "one window: the open path");
        assert!(!sh.desk_activate_shown_app(viewer, 0.0), "only an app (its main id) is activated");
    }

    /// E2E9 5: opening an app that already shows two windows (Search's Return, Cmd+Tab) brings both forward, in their
    /// own order, instead of only its main window.
    #[test]
    fn opening_a_running_app_brings_all_of_its_windows_forward() {
        let mut sh = crate::shell::tests::desktop_shell();
        sh.screen = crate::shell::Screen::Home;
        let m = sh.desk_metrics();
        let (main, second) = (AppId::External(3), AppId::ExternalWindow(3, 1));
        {
            let d = sh.desk.as_mut().unwrap();
            d.wins.clear();
            d.open(&m, second, (500, 400));
            d.open(&m, AppId::Terminal, (900, 600));
            d.open(&m, main, (900, 600));
            d.open(&m, AppId::Settings, (700, 500));
        }
        sh.desk_open(main, 0.0);
        let d = sh.desk.as_ref().unwrap();
        assert_eq!(d.wins.iter().map(|w| w.app).collect::<Vec<_>>(), vec![AppId::Terminal, AppId::Settings, second, main]);
        assert_eq!(sh.screen, crate::shell::Screen::App(main));
    }

    /// E2E10: opening an app whose main window is closed (only a second window is shown) reopens the main window
    /// instead of only activating the second one; a minimized main window is restored the same way.
    #[test]
    fn opening_an_app_with_its_main_window_closed_reopens_it() {
        let mut sh = crate::shell::tests::desktop_shell();
        let m = sh.desk_metrics();
        let (main, second) = (AppId::External(3), AppId::ExternalWindow(3, 1));
        {
            let d = sh.desk.as_mut().unwrap();
            d.wins.clear();
            d.open(&m, second, (500, 400));
            d.open(&m, AppId::Terminal, (900, 600));
        }
        assert!(!sh.desk_main_window_shown(main), "only the second window is open");
        sh.desk.as_mut().unwrap().open(&m, main, (900, 600));
        assert!(sh.desk_main_window_shown(main));
        let i = sh.desk.as_ref().unwrap().index_of(main).unwrap();
        sh.desk.as_mut().unwrap().wins[i].minimized = true;
        assert!(!sh.desk_main_window_shown(main), "a minimized main window is restored by the open path");
        sh.desk.as_mut().unwrap().wins.retain(|w| w.app != main);
        // without the window server the shell opens the main window itself (a direct app's window comes from the app)
        #[cfg(not(feature = "desktop-wayland"))]
        {
            sh.desk_open(main, 0.0);
            assert!(sh.desk.as_ref().unwrap().index_of(main).is_some(), "the main window is open again");
        }
    }

    /// E2E5 3: the window server raises only the window it is asked to activate, so a Dock click on an app showing a
    /// viewer tells it the app's whole set: the main window comes over the other apps too, under the viewer.
    #[test]
    fn activating_an_app_tells_the_window_server_all_of_its_windows_in_order() {
        let mut sh = crate::shell::tests::desktop_shell();
        let m = sh.desk_metrics();
        let (main, viewer) = (AppId::External(3), AppId::ExternalWindow(3, 2));
        {
            let d = sh.desk.as_mut().unwrap();
            d.wins.clear();
            d.open(&m, main, (900, 600));
            d.open(&m, viewer, (500, 400));
            d.open(&m, AppId::Terminal, (900, 600));
        }
        sh.take_wayland_restack();
        assert!(sh.desk_activate_shown_app(main, 0.0));
        // the window server ids of the app's windows (the main window's needs the app in the catalogue)
        let ids: Vec<u32> = [main, viewer].into_iter().filter_map(win_id).collect();
        assert!(!ids.is_empty());
        assert_eq!(sh.take_wayland_restack(), ids, "the app's windows, bottom first; not the other app's");
        assert!(sh.take_wayland_restack().is_empty(), "told once");
        // Window > Bring All to Front says the same
        sh.screen = crate::shell::Screen::App(main);
        sh.desk_bring_all_to_front();
        let mut told = sh.take_wayland_restack();
        told.sort_unstable();
        let mut want = ids.clone();
        want.sort_unstable();
        assert_eq!(told, want);
    }

    /// E2E5 3: a hidden app shown again brings all of its windows back, and the window server is told the set, so the
    /// toplevels that map before the front one do not go under the window it activated meanwhile.
    #[test]
    fn a_hidden_app_shown_again_tells_the_window_server_all_of_its_windows() {
        let m = metrics();
        let mut d = Desk::new();
        let (main, viewer) = (AppId::External(3), AppId::ExternalWindow(3, 2));
        d.open(&m, main, (900, 600));
        d.open(&m, viewer, (500, 400));
        d.open(&m, AppId::Terminal, (900, 600));
        assert!(d.hide_app(main));
        d.restack.clear();
        d.open(&m, main, (900, 600));
        assert!(d.restack.contains(&main) && d.restack.contains(&viewer), "{:?}", d.restack);
        assert!(!d.restack.contains(&AppId::Terminal), "the other app stays where it is");
    }

    /// The Dock activating an app with extra windows still counts as opening it: it is in Recent Items, as every open is.
    #[test]
    fn activating_an_app_with_extra_windows_notes_it_in_recent_items() {
        let mut sh = crate::shell::tests::desktop_shell();
        let m = sh.desk_metrics();
        let (main, viewer) = (AppId::External(3), AppId::ExternalWindow(3, 2));
        {
            let d = sh.desk.as_mut().unwrap();
            d.wins.clear();
            d.open(&m, main, (900, 600));
            d.open(&m, viewer, (500, 400));
        }
        assert!(sh.desk_activate_shown_app(main, 0.0));
        let newest = sh.recent_list.entries().first().map(|e| e.key.clone());
        assert_eq!(newest, Some(super::super::recent_items::key(main)), "the newest Recent Items entry is the app activated");
    }

    #[test]
    fn a_direct_app_activated_from_the_dock_raises_all_its_toplevels_front_last() {
        let windows = [(7, "x.mail", false), (8, "x.other", true), (9, "x.mail", true), (10, "x.mail", false)];
        assert_eq!(direct_activation_order(windows, "x.mail"), vec![7, 10, 9], "the active one last, so it ends in front");
        assert_eq!(direct_activation_order(windows, "x.none"), Vec::<u32>::new());
        assert_eq!(direct_activation_order([(1, "", false)], ""), Vec::<u32>::new(), "an empty id is no app's");
    }

    /// E2E N5: a press in the gap between two Dock icons (where the pointer magnifies both alike) opens the nearer one;
    /// the wider gap at a divider keeps a dead strip in its middle.
    #[test]
    fn a_dock_press_between_two_icons_opens_the_nearer_one() {
        let row: Vec<Rect> = (0..3).map(|i| Rect { x: 100 + i * 58, y: 700, w: 48, h: 48 }).collect();
        let press = dock_press_rects(&row, false);
        let hit = |press: &[Rect], x: f32, y: f32| press.iter().position(|r| r.contains(x, y));
        assert_eq!(hit(&press, 120.0, 720.0), Some(0));
        assert_eq!(hit(&press, 150.0, 720.0), Some(0), "just past the first icon");
        assert_eq!(hit(&press, 154.0, 720.0), Some(1), "just before the second");
        assert_eq!(hit(&press, 154.0, 760.0), None, "below the row is not the Dock's icons");
        let divided = vec![Rect { x: 100, y: 700, w: 48, h: 48 }, Rect { x: 158, y: 700, w: 48, h: 48 }, Rect { x: 240, y: 700, w: 48, h: 48 }];
        assert_eq!(hit(&dock_press_rects(&divided, false), 223.0, 720.0), None, "the divider's middle opens nothing");
        let side = dock_press_rects(&[Rect { x: 10, y: 100, w: 48, h: 48 }, Rect { x: 10, y: 158, w: 48, h: 48 }], true);
        assert_eq!(hit(&side, 30.0, 152.0), Some(0));
        assert_eq!(hit(&side, 30.0, 153.0), Some(1));
        assert!(dock_press_rects(&[], false).is_empty());
    }

    #[test]
    fn hide_others_keeps_the_front_app_and_unhide_all_restores_the_rest() {
        let m = metrics();
        let mut d = Desk::new();
        d.open(&m, AppId::Photos, (900, 600));
        d.open(&m, AppId::Terminal, (900, 600));
        d.open(&m, AppId::Settings, (900, 600));
        let i = d.index_of(AppId::Photos).unwrap();
        d.wins[i].minimized = true; // minimized by the user: not hidden, and Show All leaves it in the Dock
        assert!(d.hide_others(Some(AppId::Settings)));
        assert!(d.is_hidden(AppId::Terminal) && !d.is_hidden(AppId::Settings));
        assert!(!d.is_hidden(AppId::Photos), "a minimized window is not hidden");
        assert!(!d.others_shown(Some(AppId::Settings)));
        d.unhide_all();
        assert!(!d.wins[d.index_of(AppId::Terminal).unwrap()].minimized, "Terminal is back");
        assert!(d.wins[d.index_of(AppId::Photos).unwrap()].minimized, "the minimized window stays in the Dock");
        assert!(!d.any_hidden());
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

    /// W2-22: a borderless window's shape is where its pixels cover, row runs merged into rects.
    #[test]
    fn the_alpha_shape_covers_the_drawn_pixels_only() {
        let (w, h) = (10, 6);
        let mut px = vec![0u32; w * h];
        // a 3x2 block at (1,1) and a 2x3 block at (6,2)
        for (x0, y0, bw, bh) in [(1, 1, 3, 2), (6, 2, 2, 3)] {
            for y in y0..y0 + bh { for x in x0..x0 + bw { px[y * w + x] = 0x80ff_ff00; } }
        }
        let shape = alpha_shape(&px, w, h);
        let covered = |x: usize, y: usize| shape_contains(&shape, x as f32 + 0.5, y as f32 + 0.5);
        for y in 0..h { for x in 0..w {
            assert_eq!(covered(x, y), px[y * w + x] >> 24 != 0, "({x},{y})");
        }}
        assert!(shape.len() <= 4, "rows with the same runs merge: {shape:?}");
        assert!(alpha_shape(&vec![0u32; w * h], w, h).is_empty(), "nothing drawn, nothing hit");
        assert_eq!(alpha_shape(&vec![0xff00_0000u32; w * h], w, h), vec![[0, 0, w as i32, h as i32]], "an opaque window is one rect");
        // a noisy picture falls back to boxes that still cover every drawn pixel
        let (w, h) = (2 * MAX_SHAPE_RECTS, 4);
        let noisy: Vec<u32> = (0..w * h).map(|i| if (i + i / w) % 2 == 0 { 0xff00_0000 } else { 0 }).collect();
        let shape = alpha_shape(&noisy, w, h);
        assert!(shape.len() <= MAX_SHAPE_RECTS);
        assert!((0..w * h).filter(|i| noisy[*i] != 0).all(|i| shape_contains(&shape, (i % w) as f32 + 0.5, (i / w) as f32 + 0.5)));
    }

    /// W2-22: a click on the transparent part of a shaped window picks the window behind it.
    #[test]
    fn a_shaped_window_lets_a_click_through_where_it_drew_nothing() {
        let m = metrics();
        let mut d = Desk::new();
        let back = d.open(&m, AppId::Photos, (900, 600));
        let back_r = d.wins[back].r;
        let front = d.open(&m, AppId::External(3), (1600, 900));
        let r = d.wins[front].r;
        let note = [100, 100, 200, 150];
        d.shapes.insert(AppId::External(3), std::sync::Arc::from(vec![note]));
        let on_note = ((r.x + 150) as f32, (r.y + 150) as f32);
        assert_eq!(d.pick(&m, on_note.0, on_note.1).map(|(i, _)| d.wins[i].app), Some(AppId::External(3)), "the note takes the click");
        // a point inside both windows but off the note reaches the window behind
        let (x, y) = ((back_r.x + back_r.w / 2) as f32, (back_r.y + back_r.h / 2) as f32);
        assert!(r.contains(x, y) && !shape_contains(&[note], x - r.x as f32, y - r.y as f32));
        assert_eq!(d.pick(&m, x, y).map(|(i, _)| d.wins[i].app), Some(AppId::Photos));
        d.close(d.index_of(AppId::External(3)).unwrap());
        assert!(d.shapes.is_empty(), "closing forgets the shape");
    }

    /// W2-26: a window closed and opened again comes back at the frame it was closed with, and its toplevel asks the
    /// window server to map it there; a window never closed is cascaded as before.
    #[test]
    fn a_reopened_window_comes_back_where_it_was_closed() {
        let m = metrics();
        let mut d = Desk::new();
        let i = d.open(&m, AppId::Settings, (700, 600));
        let a = Desk::area(&m);
        let at = Rect { x: a.x + 40, y: a.y + 30, w: 700, h: 600 };
        d.place(&m, i, at);
        d.close(i);
        assert!(!d.placed.contains(&AppId::Settings), "the closed toplevel is gone");
        let i = d.open(&m, AppId::Settings, (700, 600));
        assert_eq!((d.wins[i].r.x, d.wins[i].r.y), (at.x, at.y), "back where it was");
        assert!(d.placed.contains(&AppId::Settings), "its new toplevel maps there (origin_hint)");
        assert_eq!(origin_hint(&d.placed, AppId::Settings, d.wins[i].r.x, d.wins[i].r.y), Some((at.x, at.y)));
        // a remembered spot off the work area (a smaller screen now) is held inside it
        d.close(i);
        d.closed_origins.insert(AppId::Settings, (a.x + a.w + 500, a.y - 200));
        let i = d.open(&m, AppId::Settings, (700, 600));
        let r = d.wins[i].r;
        assert!(r.x >= a.x && r.x + r.w <= a.x + a.w && r.y >= a.y, "inside the work area");
        // a window never closed is cascaded and asks nothing
        let j = d.open(&m, AppId::Photos, (500, 400));
        let p = d.wins[j].app;
        assert!(!d.placed.contains(&p));
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
        Metrics { w, h, title: px(30.0), menubar: px(28.0), min_w: px(320.0), min_h: px(220.0), edge: px(6.0), light: px(12.0), light_gap: px(8.0), light_x: px(12.0), dock_left: 0, dock_right: 0, dock_bottom: px(100.0), pt }
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

    /// A window the window server placed asks its toplevel to map where it is (`origin`); a new one asks nothing.
    #[cfg(feature = "desktop-wayland")]
    #[test]
    fn the_frames_of_placed_windows_carry_their_origin_and_new_windows_do_not() {
        let mut sh = crate::shell::tests::desktop_shell();
        let m = sh.desk_metrics();
        let (placed, fresh) = (AppId::Settings, AppId::AppStore);
        {
            let d = sh.desk.as_mut().unwrap();
            d.wins.clear();
            d.open(&m, placed, (600, 400));
            d.open(&m, fresh, (600, 400));
        }
        // the window server reported where the person dragged the first window
        let id = win_id(placed).unwrap();
        sh.wayland_window_event(crate::bare::wayland::WindowEvent::Frame(id, 1065, 524, 600, 400));
        let rect = |sh: &Shell, app| sh.desk.as_ref().unwrap().wins.iter().find(|w| w.app == app).map(|w| w.r).unwrap();
        assert_eq!((rect(&sh, placed).x, rect(&sh, placed).y), (1065, 524), "the shell follows the reported frame");
        let frames: Vec<_> = [placed, fresh].iter().map(|&app| {
            let r = rect(&sh, app);
            crate::bare::wayland::WindowFrame { id: win_id(app).unwrap(), title: "", x: r.x, y: r.y, width: r.w as usize, height: r.h as usize, pixels: None, damage: crate::gfx::Damage::new(1, 1), maximized: false, fullscreen: false, title_height: 0, drag_height: 0, no_drag: Vec::new(), floating: false, fx: bifrost_render::glass::Fx::IDENTITY, opaque: true, origin: None, position: None, min_size: (0, 0), input: None }
        }).collect();
        sh.desk.as_mut().unwrap().frames = frames;
        let out = sh.take_wayland_windows();
        let of = |app| out.iter().find(|f| Some(f.id) == win_id(app)).unwrap();
        assert_eq!(of(placed).origin, Some((1065, 524)));
        assert_eq!(of(fresh).origin, None, "a window the window server never placed is cascaded by it");
        // each toplevel advertises its app's own minimum: Settings' split width, the desk minimum for the others
        let (sw, sh_min, _, _) = size_range(placed, &m);
        assert_eq!(of(placed).min_size, (sw, sh_min));
        assert!(sw >= (crate::shell::settings::SPLIT_MIN_PT * m.pt) as i32 || sw == m.w, "Settings' minimum is its split layout width");
        assert_eq!(of(fresh).min_size, (m.min_w, m.min_h + title_of(fresh, &m)));
        // closing forgets the place: the window opened again is a new one
        let d = sh.desk.as_mut().unwrap();
        let i = d.index_of(placed).unwrap();
        d.close(i);
        assert!(!d.placed.contains(&placed));
    }

    /// E2E11 3: an app moving its mapped window (`WindowMove`) makes the next frame ask the window server to move the
    /// toplevel there (`set_position`), once; a frame after that asks nothing, so a later drag is never pulled back.
    #[cfg(feature = "desktop-wayland")]
    #[test]
    fn a_window_its_app_moved_asks_the_window_server_to_move_its_toplevel_once() {
        let mut sh = crate::shell::tests::desktop_shell();
        let m = sh.desk_metrics();
        let app = AppId::Settings;
        {
            let d = sh.desk.as_mut().unwrap();
            d.wins.clear();
            d.open(&m, app, (600, 400));
        }
        let id = win_id(app).unwrap();
        // the window server mapped it and said where
        sh.wayland_window_event(crate::bare::wayland::WindowEvent::Frame(id, 300, 200, 600, 400));
        assert!(sh.ext_window_place(app, Some((140.0, 120.0)), false));
        let r = sh.desk.as_ref().unwrap().wins.iter().find(|w| w.app == app).map(|w| w.r).unwrap();
        let frame = |r: Rect| crate::bare::wayland::WindowFrame { id, title: "", x: r.x, y: r.y, width: r.w as usize, height: r.h as usize, pixels: None, damage: crate::gfx::Damage::new(1, 1), maximized: false, fullscreen: false, title_height: 0, drag_height: 0, no_drag: Vec::new(), floating: false, fx: bifrost_render::glass::Fx::IDENTITY, opaque: true, origin: None, position: None, min_size: (0, 0), input: None };
        sh.desk.as_mut().unwrap().frames = vec![frame(r)];
        let out = sh.take_wayland_windows();
        assert_eq!(out[0].position, Some((r.x, r.y)), "the mapped toplevel moves where the shell put the window");
        assert_ne!((r.x, r.y), (300, 200));
        sh.desk.as_mut().unwrap().frames = vec![frame(r)];
        let out = sh.take_wayland_windows();
        assert_eq!(out[0].position, None, "asked once");
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
        let w = Win { app: AppId::Files, r: Rect { x: 100, y: 100, w: 600, h: 400 }, maximized: false, minimized: false, full: false, restore: Rect { x: 0, y: 0, w: 1, h: 1 }, full_size: false };
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
        let w = Win { app: AppId::Settings, r: Desk::area(&m), maximized: true, minimized: false, full: false, restore: Rect { x: 0, y: 0, w: 1, h: 1 }, full_size: false };
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
    fn an_app_resizes_its_window_from_the_top_left_inside_the_work_area() {
        let m = metrics();
        let app = AppId::External(250);
        let mut d = Desk::new();
        d.open(&m, app, (900, 600));
        let before = d.wins[0].r;
        // a size inside the window's range (the desktop minimum is m.min_w wide; smaller is held to it, below)
        assert!(d.set_content_size(&m, 0, (430.0, 510.0)));
        let (w, h) = outer_px(app, &m, (430.0, 510.0));
        assert_eq!(d.wins[0].r, Rect { x: before.x, y: before.y, w, h }, "the top-left corner stays");
        assert_eq!(d.wins[0].restore, d.wins[0].r);
        assert!(!d.set_content_size(&m, 0, (430.0, 510.0)), "the same size again changes nothing");
        // larger than the screen: held to the work area, and moved back onto it
        assert!(d.set_content_size(&m, 0, (100_000.0, 100_000.0)));
        assert_eq!(d.wins[0].r, Desk::area(&m));
        // under the window minimum: held to it
        assert!(d.set_content_size(&m, 0, (1.0, 1.0)));
        let (min_w, min_h, _, _) = size_range(app, &m);
        assert_eq!((d.wins[0].r.w, d.wins[0].r.h), (min_w, min_h));
    }

    #[test]
    fn a_zoomed_window_keeps_its_frame_and_returns_at_the_size_the_app_asked_for() {
        let m = metrics();
        let app = AppId::External(251);
        let mut d = Desk::new();
        d.open(&m, app, (900, 600));
        d.toggle_zoom(&m, 0);
        assert!(!d.set_content_size(&m, 0, (560.0, 522.0)));
        assert_eq!(d.wins[0].r, Desk::area(&m));
        d.toggle_zoom(&m, 0);
        let (w, h) = outer_px(app, &m, (560.0, 522.0));
        assert_eq!((d.wins[0].r.w, d.wins[0].r.h), (w, h));
    }

    #[test]
    fn content_limits_hold_the_apps_own_resize_and_go_with_the_app() {
        let m = metrics();
        let app = AppId::External(252);
        set_content_limits(252, Some((600.0, 500.0)), None);
        let mut d = Desk::new();
        d.open(&m, app, (2000, 1200));
        assert!(d.set_content_size(&m, 0, (230.0, 300.0)));
        let (w, h) = outer_px(app, &m, (600.0, 500.0));
        assert_eq!((d.wins[0].r.w, d.wins[0].r.h), (w.min(Desk::area(&m).w), h.min(Desk::area(&m).h)));
        forget_content_limits(252);
        assert_eq!(size_limits(app).min, None, "the manifest's limits (none here) apply again");
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
        d.open(&m, AppId::Photos, (900, 600));
        let start = d.wins[0].r;
        d.resize_to(&m, 0, R | B, start, -5000, -5000);
        assert_eq!((d.wins[0].r.w, d.wins[0].r.h), (m.min_w, m.min_h + title_of(AppId::Photos, &m)), "the desktop minimum plus the title bar");
        d.resize_to(&m, 0, L | T, start, -50, -5000);
        assert_eq!(d.wins[0].r.x, start.x - 50);
        assert!(d.wins[0].r.y >= m.menubar);
        assert_eq!(d.wins[0].r.y + d.wins[0].r.h, start.y + start.h, "the bottom edge stays put");
    }

    /// W2-23: a press latches the icon it hit, so the release opens it even when the row has re-centred under the
    /// pointer since (or the layout fell back to rest); a different entry now in that slot does not count.
    #[test]
    fn a_dock_release_opens_the_icon_latched_at_the_press() {
        let row: Vec<Rect> = (0..5).map(|i| Rect { x: 100 + i * 58, y: 700, w: 48, h: 48 }).collect();
        // the icon under the pointer in the row magnified around it, as the press hit-tests it
        let live = super::super::desktop_dock::magnified_rects(&row, false, true, 175.0, 96, 150.0, 1.0);
        let k = live.iter().position(|r| r.contains(175.0, 740.0)).expect("an icon under the pointer");
        assert!(dock_release_hits(live[k], Some(row[k]), Some(7), 7, 175.0, 740.0), "the latched rect decides");
        assert!(!dock_release_hits(live[k], Some(live[k]), Some(8), 7, 400.0, 740.0), "a release elsewhere opens nothing");
        assert!(dock_release_hits(Rect { x: 0, y: 0, w: 1, h: 1 }, Some(row[2]), Some(7), 7, 230.0, 720.0), "the slot's current rect counts for the same entry");
        assert!(!dock_release_hits(Rect { x: 0, y: 0, w: 1, h: 1 }, Some(row[2]), Some(8), 7, 230.0, 720.0), "but not once another entry is there");
    }

    /// W2-23: pressing a Dock icon keeps the row magnified (it used to drop to the resting layout under the pointer);
    /// a drag that moved, and any other grab, lay the row out at rest.
    #[test]
    fn a_pressed_dock_icon_keeps_the_dock_magnified_until_it_is_dragged() {
        let mut d = Desk::new();
        assert!(d.dock_grab_magnifies());
        let r = Rect { x: 0, y: 0, w: 48, h: 48 };
        d.grab = Some(Grab::DockDrag { from: 1, entry: 3, rect: r, x: 0.0, y: 0.0, at_x: 0.0, at_y: 0.0, moved: false });
        assert!(d.dock_grab_magnifies());
        d.grab = Some(Grab::DockDrag { from: 1, entry: 3, rect: r, x: 0.0, y: 0.0, at_x: 40.0, at_y: 0.0, moved: true });
        assert!(!d.dock_grab_magnifies());
        d.grab = Some(Grab::Content(AppId::Settings));
        assert!(!d.dock_grab_magnifies());
    }

    /// W2-25: Cmd+` visits every window of the app in turn although each press only raises one, and Shift+Cmd+`
    /// walks the other way; a window brought forward some other way starts a new walk.
    #[test]
    fn cmd_grave_walks_every_window_of_the_app_both_ways() {
        let (a, b, c) = (AppId::External(3), AppId::ExternalWindow(3, 1), AppId::ExternalWindow(3, 2));
        // raising only: the window raised goes in front, the rest keep their order
        let raise = |stack: &[AppId], w: AppId| { let mut s: Vec<AppId> = stack.iter().copied().filter(|x| *x != w).collect(); s.insert(0, w); s };
        let mut stack = vec![a, b, c];
        let mut cycle = None;
        let mut seen = vec![];
        for _ in 0..3 {
            let (order, sel) = window_cycle_next(cycle.take(), &stack, false).expect("three windows cycle");
            stack = raise(&stack, order[sel]);
            seen.push(order[sel]);
            cycle = Some((order, sel));
        }
        assert_eq!(seen, vec![b, c, a], "forwards: every window once, then back to the first");
        let (order, sel) = window_cycle_next(None, &[a, b, c], true).unwrap();
        assert_eq!(order[sel], c, "Shift+Cmd+` brings the last window forward");
        // the person clicked b: a new walk from the stack as it is now
        let (order, sel) = window_cycle_next(Some((vec![a, b, c], 0)), &[b, a, c], false).unwrap();
        assert_eq!(order[sel], a);
        assert!(window_cycle_next(None, &[a], false).is_none(), "one window: nothing to cycle");
        assert_eq!(cycle_step(0, 3, true), 2);
        assert_eq!(cycle_step(2, 3, false), 0);
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
    stroke_window_rim(s, x, y, w, h, rad, window_rim_color());
}

fn window_rim_color() -> u32 {
    bifrost_render::appearance::Pair::new(rgba(255, 255, 255, 38), rgba(255, 255, 255, 0)).now()
}

fn stroke_window_rim(s: &mut Surface, x: f32, y: f32, w: f32, h: f32, rad: f32, c: u32) {
    if c >> 24 != 0 { s.stroke_rrect(x, y, w, h, rad, 1.0, c); }
}

/// Width of the band along each window edge that the rim stroke and the corner clip can change.
const EDGE_BAND: usize = 3;

/// The parts of a `width` x `height` window that the rim stroke and the corner clip touch, as (x, y, w, h): a band
/// `EDGE_BAND` wide along each edge and a square over each rounded corner.
#[allow(dead_code)]
fn edge_band_rects(width: usize, height: usize, radius: f32) -> [(i32, i32, i32, i32); 8] {
    let n = (radius.ceil() as usize).min(width / 2).min(height / 2);
    let (w, h) = (width as i32, height as i32);
    let b = EDGE_BAND.min(width).min(height) as i32;
    let c = (n + EDGE_BAND).min(width).min(height) as i32;
    [(0, 0, w, b), (0, h - b, w, b), (0, 0, b, h), (w - b, 0, b, h),
     (0, 0, c, c), (w - c, 0, c, c), (0, h - c, c, c), (w - c, h - c, c, c)]
}

/// Whether window rect `r` overlaps body damage area `a` (body coordinates, `top` rows below the window's top).
#[allow(dead_code)]
fn overlaps(r: &(i32, i32, i32, i32), a: &crate::gfx::Area, top: usize) -> bool {
    (a.x0 as i32) < r.0 + r.2 && (a.x1 as i32) > r.0 && ((a.y0 + top) as i32) < r.1 + r.3 && ((a.y1 + top) as i32) > r.1
}

/// Copies [x0, x1) x [y0, y1) of a retained window (window coordinates) from the app's frame `src` (pixels, width,
/// height; the frame starts `top` rows down), dimmed by `dim` for a background window as the fast path dims a fresh
/// blit. Pixels outside the frame (the title bar) are left as they are.
#[allow(dead_code, clippy::too_many_arguments)]
fn restore_from_frame(pixels: &mut [u32], width: usize, height: usize, top: usize, src: (&[u32], usize, usize), dim: Option<u32>, x0: i32, y0: i32, x1: i32, y1: i32) {
    let (spx, sw, sh) = src;
    let cx = |v: i32| (v.max(0) as usize).min(width).min(sw);
    let (x0, x1) = (cx(x0), cx(x1));
    let y0 = (y0.max(0) as usize).max(top);
    let y1 = (y1.max(0) as usize).min(height).min(top + sh);
    if x0 >= x1 { return; }
    for y in y0..y1 {
        let sy = y - top;
        let (Some(d), Some(s)) = (pixels.get_mut(y * width + x0..y * width + x1), spx.get(sy * sw + x0..sy * sw + x1)) else { continue };
        d.copy_from_slice(s);
        if let Some(amount) = dim { dim_rect(pixels, width, x0, y, x1, y + 1, amount); }
    }
}

/// Puts the inner rim and the rounded corners back on a retained external window whose app drew into the edge band.
/// Every band pixel inside the app's frame is first taken again from the frame (dimmed by `dim`), and every band pixel
/// outside it (the title bar, which already carries the rim from the full draw) is kept as it was; then the rim is
/// stroked once and the corners clipped once. So an edge pixel always holds exactly one stroke over the app's pixel,
/// however often the app repaints (stroking the retained buffer again blended 15% white per repaint, E2E-10).
/// Returns the window rects it changed (for the frame's damage).
#[allow(dead_code, clippy::too_many_arguments)]
fn restore_window_edge(pixels: &mut [u32], width: usize, height: usize, top: usize, src: (&[u32], usize, usize), dim: Option<u32>, radius: f32, rim: u32) -> Vec<(i32, i32, i32, i32)> {
    let rects = edge_band_rects(width, height, radius);
    let (_, sw, sh) = src;
    let in_frame = |x: usize, y: usize| y >= top && y - top < sh && x < sw;
    // pixels the frame does not cover keep their value: saved here, put back after the stroke and the clip
    let mut kept: Vec<(usize, u32)> = Vec::new();
    for &(x, y, w, h) in &rects {
        for py in y.max(0) as usize..((y + h).max(0) as usize).min(height) {
            for px in x.max(0) as usize..((x + w).max(0) as usize).min(width) {
                if !in_frame(px, py) { kept.push((py * width + px, pixels[py * width + px])); }
            }
        }
    }
    for &(x, y, w, h) in &rects { restore_from_frame(pixels, width, height, top, src, dim, x, y, x + w, y + h); }
    stroke_window_rim(&mut Surface { w: width, h: height, stride: width, px: &mut pixels[..], y0: 0 }, 0., 0., width as f32, height as f32, radius, rim);
    clip_window_corners(pixels, width, height, radius, None);
    for (i, v) in kept { pixels[i] = v; }
    rects.to_vec()
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
mod rim_tests {
    use super::*;

    const RIM: u32 = 0x2626_2626; // white at 15%, as `window_rim_color` gives in Dark
    const TITLE: u32 = 0xff30_3030;
    const APP: u32 = 0xff20_2020;

    /// The window as the full draw makes it: title bar, the app's frame below it (dimmed behind), one rim stroke, the
    /// corners clipped.
    fn full_draw(width: usize, height: usize, top: usize, src: &[u32], dim: Option<u32>, radius: f32) -> Vec<u32> {
        let mut px = vec![TITLE; width * height];
        px[top * width..].copy_from_slice(src);
        if let Some(a) = dim { dim_rect(&mut px, width, 0, top, width, height, a); }
        stroke_window_rim(&mut Surface { w: width, h: height, stride: width, px: &mut px[..], y0: 0 }, 0., 0., width as f32, height as f32, radius, RIM);
        clip_window_corners(&mut px, width, height, radius, None);
        px
    }

    /// The app repainted [x0, x1) x [y0, y1) of its body: the fast path copies that part of its frame in again.
    fn app_repaint(px: &mut [u32], width: usize, top: usize, src: &[u32], dim: Option<u32>, (x0, y0, x1, y1): (usize, usize, usize, usize)) {
        for y in y0..y1 {
            px[(y + top) * width + x0..(y + top) * width + x1].copy_from_slice(&src[y * width + x0..y * width + x1]);
            if let Some(a) = dim { dim_rect(px, width, x0, y + top, x1, y + top + 1, a); }
        }
    }

    /// E2E-10: repeated retained-path repaints that touch an edge leave every edge pixel at the single-stroke value
    /// (the Dark rim stacked 15% white per repaint into a solid white border before).
    #[test]
    fn repeated_edge_repaints_keep_the_rim_at_one_stroke() {
        for (top, dim, radius) in [(0usize, None, 8.0f32), (10, None, 8.0), (0, Some(BACKGROUND_DIM), 6.0), (12, Some(BACKGROUND_DIM), 10.0)] {
            let (width, height) = (64usize, 48usize);
            let body_h = height - top;
            let src = vec![APP; width * body_h];
            let want = full_draw(width, height, top, &src, dim, radius);
            let mut px = want.clone();
            // list click (left and bottom), body scroll (right), a full repaint, a corner
            let repaints = [(0, 0, 20, body_h), (width - 20, 0, width, body_h), (0, 0, width, body_h), (0, body_h - 5, 30, body_h), (width - 4, 0, width, 4)];
            for round in 0..8 {
                let r = repaints[round % repaints.len()];
                app_repaint(&mut px, width, top, &src, dim, r);
                restore_window_edge(&mut px, width, height, top, (&src, width, body_h), dim, radius, RIM);
                assert_eq!(px, want, "round {round}: top {top}, dim {dim:?}: the edge holds exactly one rim stroke");
            }
            // the left edge pixel half way down is the app's colour with one 15% white stroke over it, not white
            let edge = px[(top + body_h / 2) * width];
            assert!(edge & 0xff < 0x50, "left edge {edge:08x} stays dark");
        }
    }

    /// The edge band covers the 1 px rim and every pixel the corner clip changes.
    #[test]
    fn the_edge_band_covers_the_rim_and_the_corners() {
        let (width, height, radius) = (40usize, 30usize, 9.0f32);
        let mut a = vec![APP; width * height];
        let b = a.clone();
        stroke_window_rim(&mut Surface { w: width, h: height, stride: width, px: &mut a[..], y0: 0 }, 0., 0., width as f32, height as f32, radius, RIM);
        clip_window_corners(&mut a, width, height, radius, None);
        let rects = edge_band_rects(width, height, radius);
        for y in 0..height { for x in 0..width {
            if a[y * width + x] != b[y * width + x] {
                let (x, y) = (x as i32, y as i32);
                assert!(rects.iter().any(|r| x >= r.0 && x < r.0 + r.2 && y >= r.1 && y < r.1 + r.3), "({x},{y}) changed outside the band");
            }
        }}
    }
}

#[cfg(test)]
mod resize_grab_tests {
    use super::*;

    /// E2E-12: a corner drag resizes the window that was pressed, also when another window comes forward during the
    /// drag (the window server handed the focus to Settings behind, and Settings took the drag's frame).
    #[test]
    fn a_resize_follows_the_pressed_window_when_another_comes_forward() {
        let mut sh = crate::shell::tests::desktop_shell();
        sh.anim = None;
        sh.screen = crate::shell::Screen::Home;
        if sh.desk.is_none() { sh.desk = Some(Desk::new()); }
        let m = sh.desk_metrics();
        let d = sh.desk.as_mut().unwrap();
        d.open(&m, AppId::Settings, (1000, 900));
        d.open(&m, AppId::Photos, (900, 700));
        let settings_before = { let r = d.wins[d.index_of(AppId::Settings).unwrap()].r; (r.x, r.y, r.w, r.h) };
        let p = d.wins[d.index_of(AppId::Photos).unwrap()].r;
        let (x, y) = ((p.x + p.w - 1) as f32, (p.y + p.h - 1) as f32);
        sh.desk_event(Event::Down(x, y), 0.);
        let d = sh.desk.as_mut().unwrap();
        let s = d.index_of(AppId::Settings).unwrap();
        d.raise(s);
        sh.desk_event(Event::Move(x - 40., y - 30.), 16.);
        sh.desk_event(Event::Up(x - 40., y - 30.), 32.);
        let d = sh.desk.as_ref().unwrap();
        let r = d.wins[d.index_of(AppId::Settings).unwrap()].r;
        assert_eq!((r.x, r.y, r.w, r.h), settings_before, "the window behind is not resized");
        let r = d.wins[d.index_of(AppId::Photos).unwrap()].r;
        assert_eq!((r.w, r.h), (p.w - 40, p.h - 30), "the pressed window follows the pointer");
    }

    /// N1: the drag the Wayland pointer emulates for a wheel notch over a window's edge (the bottom one on Midget; every
    /// edge alike) leaves the window's size alone; a real press there, after the wheel, still resizes it.
    #[test]
    fn the_wheel_never_starts_or_drives_a_resize() {
        let mut sh = crate::shell::tests::desktop_shell();
        sh.anim = None;
        sh.screen = crate::shell::Screen::Home;
        if sh.desk.is_none() { sh.desk = Some(Desk::new()); }
        let m = sh.desk_metrics();
        let d = sh.desk.as_mut().unwrap();
        d.open(&m, AppId::Photos, (900, 700));
        let p = d.wins[d.index_of(AppId::Photos).unwrap()].r;
        let (x, y) = ((p.x + p.w - 1) as f32, (p.y + p.h / 2) as f32);
        assert!(matches!(sh.desk.as_ref().unwrap().pick(&m, x, y), Some((_, Hit::Edge(_)))), "the press is on the right edge");
        for n in 0..15 {
            let t = n as f64 * 100.;
            sh.desk_wheel(t);
            sh.desk_event(Event::Down(x, y), t);
            sh.desk_event(Event::Move(x - 9., y), t + 16.);
            sh.desk_event(Event::Up(x - 9., y), t + 32.);
        }
        let d = sh.desk.as_ref().unwrap();
        let r = d.wins[d.index_of(AppId::Photos).unwrap()].r;
        assert_eq!((r.w, r.h), (p.w, p.h), "fifteen wheel notches leave the window's size alone");
        sh.desk_event(Event::Down(x, y), 5000.);
        sh.desk_event(Event::Move(x - 30., y), 5016.);
        sh.desk_event(Event::Up(x - 30., y), 5032.);
        let d = sh.desk.as_ref().unwrap();
        let r = d.wins[d.index_of(AppId::Photos).unwrap()].r;
        assert_eq!(r.w, p.w - 30, "a press long after the wheel still resizes");
    }

    /// A wheel notch Settings took drops its emulated drag in `Shell::event`, so the desk never sees that press: the
    /// mark must not outlive it, or the real title-bar press that follows within 250 ms is taken for the wheel's.
    #[test]
    fn a_title_bar_press_right_after_a_notch_over_settings_drags_the_window() {
        let mut sh = crate::shell::tests::desktop_shell();
        sh.anim = None;
        sh.screen = crate::shell::Screen::Home;
        if sh.desk.is_none() { sh.desk = Some(Desk::new()); }
        let m = sh.desk_metrics();
        let d = sh.desk.as_mut().unwrap();
        d.open(&m, AppId::Settings, (1000, 800));
        let s = d.wins[d.index_of(AppId::Settings).unwrap()].r;
        let (x, y) = ((s.x + s.w / 2) as f32, (s.y + m.title / 2) as f32);
        assert!(matches!(sh.desk.as_ref().unwrap().pick(&m, x, y), Some((_, Hit::Title))), "the press is on the title bar");
        // the notch, as bin/shell.rs routes it: the desk is told, then Settings takes it (`settings_wheel`)
        sh.desk_wheel(1000.);
        sh.swallow_wheel_drag(1000.);
        let (cx, cy) = ((s.x + s.w / 2) as f32, (s.y + s.h / 2) as f32);
        assert!(sh.event(Event::Down(cx, cy), 1000.).is_empty());
        assert!(sh.event(Event::Move(cx, cy - 9.), 1016.).is_empty());
        assert!(sh.event(Event::Up(cx, cy - 9.), 1032.).is_empty());
        // a real press 100 ms later, well inside the wheel's 250 ms
        sh.desk_event(Event::Down(x, y), 1100.);
        sh.desk_event(Event::Move(x + 40., y + 30.), 1116.);
        sh.desk_event(Event::Up(x + 40., y + 30.), 1132.);
        let d = sh.desk.as_ref().unwrap();
        let r = d.wins[d.index_of(AppId::Settings).unwrap()].r;
        assert_eq!((r.x, r.y), (s.x + 40, s.y + 30), "the title-bar press drags the window");
        assert_eq!((r.w, r.h), (s.w, s.h));
    }

    /// E2E-12: Settings never goes below the width of its sidebar layout, whether by a drag, the window server's size or
    /// the size it is reopened at.
    #[test]
    fn settings_keeps_its_split_layout_width() {
        let m = Metrics { w: 2560, h: 1440, title: 38, menubar: 35, min_w: 400, min_h: 275, edge: 8, light: 18, light_gap: 10, light_x: 15, dock_left: 0, dock_right: 0, dock_bottom: 120, pt: 1.25 };
        let (min_w, _, _, _) = size_range(AppId::Settings, &m);
        assert!(min_w as f32 >= super::super::settings::SPLIT_MIN_PT * m.pt, "{min_w} px is the split layout");
        assert_eq!(clamp_size(AppId::Settings, &m, (400, 549)).0, min_w, "the 400x549 of E2E-12 is held to the minimum");
        let mut d = Desk::new();
        d.open(&m, AppId::Settings, (923, 860));
        let start = d.wins[0].r;
        d.resize_to(&m, 0, R | B, start, -800, -500);
        assert_eq!(d.wins[0].r.w, min_w);
        assert!(clamp_size(AppId::Photos, &m, (100, 100)) == (m.min_w, m.min_h + m.title), "other windows keep the desktop minimum");
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
        return bifrost_render::glass::Rect { x, y, w: rw, h: rh, r, kind, active, detail: None, tint: [0.0; 4] };
    }
    let e = 0.5;
    let x0 = if x <= e { 0.0 } else { (x - corner).max(0.0) };
    let y0 = if y <= e { 0.0 } else { (y - corner).max(0.0) };
    let x1 = if x + rw >= ww - e { ww } else { (x + rw + corner).min(ww) };
    let y1 = if y + rh >= wh - e { wh } else { (y + rh + corner).min(wh) };
    bifrost_render::glass::Rect { x: x0, y: y0, w: x1 - x0, h: y1 - y0, r: corner.max(r), kind, active, detail: None, tint: [0.0; 4] }
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

#[cfg(test)]
mod w3_window_tests {
    use super::*;

    fn two_windows() -> (crate::shell::Shell, Metrics) {
        let mut sh = crate::shell::tests::desktop_shell();
        sh.anim = None;
        sh.screen = crate::shell::Screen::Home;
        if sh.desk.is_none() { sh.desk = Some(Desk::new()); }
        let m = sh.desk_metrics();
        let d = sh.desk.as_mut().unwrap();
        d.open(&m, AppId::Settings, (1000, 900));
        d.open(&m, AppId::Photos, (900, 700));
        (sh, m)
    }

    /// A point on `light` of `app`'s window that the desk hit-tests as that light.
    fn light_at(d: &Desk, m: &Metrics, app: AppId, light: Light) -> (f32, f32) {
        let i = d.index_of(app).unwrap();
        let r = d.wins[i].r;
        for y in r.y..r.y + 80 {
            for x in r.x..r.x + 200 {
                if d.pick(m, x as f32, y as f32) == Some((i, Hit::Light(light))) {
                    return (x as f32, y as f32);
                }
            }
        }
        panic!("{app:?} has no {light:?} button in front");
    }

    /// W3-1: a traffic light acts on the window that was pressed, also when the stack changes before the release (the
    /// window server handing the focus to another window), as the move and resize grabs do.
    #[test]
    fn a_traffic_light_acts_on_the_pressed_window_when_another_comes_forward() {
        let (mut sh, m) = two_windows();
        let (x, y) = light_at(sh.desk.as_ref().unwrap(), &m, AppId::Photos, Light::Zoom);
        sh.desk_event(Event::Down(x, y), 0.);
        let d = sh.desk.as_mut().unwrap();
        let s = d.index_of(AppId::Settings).unwrap();
        d.raise(s);
        sh.desk_event(Event::Up(x, y), 16.);
        let d = sh.desk.as_ref().unwrap();
        assert!(d.wins[d.index_of(AppId::Photos).unwrap()].maximized, "the pressed window zooms");
        assert!(!d.wins[d.index_of(AppId::Settings).unwrap()].maximized, "the window that came forward does not");
    }

    /// W3-6: Bring All to Front raises the active app's shown windows and leaves minimized ones in the Dock.
    #[test]
    fn bring_all_to_front_raises_the_app_and_never_unminimizes() {
        let (mut sh, m) = two_windows();
        let d = sh.desk.as_mut().unwrap();
        d.open(&m, AppId::Files, (700, 500));
        let f = d.index_of(AppId::Files).unwrap();
        d.wins[f].minimized = true;
        sh.screen = crate::shell::Screen::App(AppId::Settings);
        sh.desk_bring_all_to_front();
        let d = sh.desk.as_ref().unwrap();
        assert!(d.wins[d.index_of(AppId::Files).unwrap()].minimized, "a minimized window stays in the Dock");
        assert_eq!(d.front().map(|w| w.app), Some(AppId::Settings), "the active app's window comes to the front");
        assert!(!d.wins[d.index_of(AppId::Photos).unwrap()].minimized);
    }

    /// W3-6: a window filled, taken full screen and back keeps the frame from before the Fill as its way back.
    #[test]
    fn full_screen_from_a_fill_keeps_the_frame_from_before_the_fill() {
        let m = Metrics { w: 2560, h: 1440, title: 38, menubar: 35, min_w: 400, min_h: 275, edge: 8, light: 18, light_gap: 10, light_x: 15, dock_left: 0, dock_right: 0, dock_bottom: 120, pt: 1.25 };
        let mut d = Desk::new();
        d.open(&m, AppId::Photos, (923, 860));
        let before = d.wins[0].r;
        d.toggle_zoom(&m, 0);
        assert!(d.set_full_screen(&m, 0, true));
        assert!(d.set_full_screen(&m, 0, false));
        assert!(d.wins[0].maximized, "it comes back filled");
        assert_eq!(d.wins[0].r, Desk::area(&m));
        d.return_to_previous(0);
        assert_eq!(d.wins[0].r, before, "Return to Previous Size goes back to the frame from before the Fill");
    }

    /// W3-6: the window server's own full screen (Ctrl+Cmd+F) on a filled window: it reports the full-screen state
    /// before the maximize state (`bare::wayland` configure), so the shell keeps its zoom and the frame from before it,
    /// and the un-maximized configure of a full-screen window is not taken as an un-zoom.
    #[cfg(feature = "desktop-wayland")]
    #[test]
    fn the_window_servers_full_screen_keeps_the_zoom_and_its_way_back() {
        use crate::bare::wayland::WindowEvent;
        let mut sh = crate::shell::tests::desktop_shell();
        sh.anim = None;
        sh.screen = crate::shell::Screen::Home;
        sh.desk_open(AppId::Settings, 0.);
        let id = win_id(AppId::Settings).unwrap();
        let m = sh.desk_metrics();
        let i = sh.desk.as_ref().unwrap().index_of(AppId::Settings).unwrap();
        let before = sh.desk.as_ref().unwrap().wins[i].r;
        sh.desk.as_mut().unwrap().toggle_zoom(&m, i);
        sh.wayland_window_event(WindowEvent::Maximized(id, true));
        sh.wayland_window_event(WindowEvent::Fullscreen(id, true));
        sh.wayland_window_event(WindowEvent::Maximized(id, false));
        assert!(sh.desk.as_ref().unwrap().wins[i].full);
        sh.wayland_window_event(WindowEvent::Fullscreen(id, false));
        sh.wayland_window_event(WindowEvent::Maximized(id, true));
        let w = sh.desk.as_ref().unwrap().wins[i];
        assert!(!w.full && w.maximized, "back filled");
        sh.desk.as_mut().unwrap().return_to_previous(i);
        assert_eq!(sh.desk.as_ref().unwrap().wins[i].r, before, "the frame from before the Fill, not the full-screen one");
    }

    #[cfg(feature = "desktop-wayland")]
    fn arrange_menu_open(sh: &mut crate::shell::Shell) -> (f32, f32) {
        let m = sh.desk_metrics();
        let (gx, gy) = light_at(sh.desk.as_ref().unwrap(), &m, AppId::Settings, Light::Zoom);
        sh.desktop_pointer(gx, gy, false, 1000.);
        sh.arrange_tick(1000. + ARRANGE_DWELL_MS);
        assert_eq!(sh.desktop_menu, Some(super::super::desktop_bar::Menu::Arrange));
        (gx, gy)
    }

    /// W3-5: the pointer crossing from the window onto the menu's own surface makes the window server send a Leave
    /// (`Hover(-100, -100)`) before the menu's first motion; the menu stays open for it and closes only on a motion
    /// that is outside the button, the bridge and the menu.
    #[cfg(feature = "desktop-wayland")]
    #[test]
    fn the_green_button_menu_survives_the_pointer_crossing_onto_its_surface() {
        let mut sh = crate::shell::tests::desktop_shell();
        sh.screen = crate::shell::Screen::Home;
        sh.anim = None;
        sh.desk_open(AppId::Settings, 0.);
        arrange_menu_open(&mut sh);
        sh.desktop_pointer(-100., -100., false, 1600.);
        assert_eq!(sh.desktop_menu, Some(super::super::desktop_bar::Menu::Arrange), "a Leave is not a move away");
        let (px, py, pw, ph) = sh.desktop_popup_rect().unwrap();
        sh.desktop_pointer((px + pw / 2) as f32, (py + ph / 2) as f32, false, 1650.);
        assert_eq!(sh.desktop_menu, Some(super::super::desktop_bar::Menu::Arrange), "the pointer is on the menu");
        sh.desktop_pointer(sh.w as f32 - 4., sh.h as f32 - 4., false, 1700.);
        assert!(sh.desktop_menu.is_none(), "a motion away from it closes it");
    }

    /// W3-5: a menu closed by a click on its button opens again when the pointer moves on the button and rests there.
    #[cfg(feature = "desktop-wayland")]
    #[test]
    fn resting_on_the_green_button_again_reopens_a_menu_closed_by_a_click() {
        let mut sh = crate::shell::tests::desktop_shell();
        sh.screen = crate::shell::Screen::Home;
        sh.anim = None;
        sh.desk_open(AppId::Settings, 0.);
        let (gx, gy) = arrange_menu_open(&mut sh);
        assert!(sh.desktop_bar_event(Event::Down(gx, gy), 2000.).is_none());
        assert!(sh.desktop_menu.is_none());
        sh.arrange_tick(2000. + 2. * ARRANGE_DWELL_MS);
        assert!(sh.desktop_menu.is_none(), "not again without the pointer moving");
        sh.desktop_pointer(gx, gy, false, 3000.);
        sh.arrange_tick(3000. + ARRANGE_DWELL_MS);
        assert_eq!(sh.desktop_menu, Some(super::super::desktop_bar::Menu::Arrange), "a new rest opens it again");
    }
}

#[cfg(test)]
mod w4_window_tests {
    use super::*;

    fn metrics() -> Metrics {
        Metrics { w: 2560, h: 1440, title: 52, menubar: 35, min_w: 320, min_h: 220, edge: 6, light: 12, light_gap: 8, light_x: 12, dock_left: 0, dock_right: 0, dock_bottom: 172, pt: 1.25 }
    }

    fn desktop() -> crate::shell::Shell {
        let mut sh = crate::shell::tests::desktop_shell();
        sh.anim = None;
        sh.screen = crate::shell::Screen::Home;
        if sh.desk.is_none() { sh.desk = Some(Desk::new()); }
        sh
    }

    /// W4-1: a built-in window never floats, whatever the list says, so raising it always puts it in front of the
    /// normal windows of every app.
    #[test]
    fn a_built_in_window_never_floats() {
        let mut d = Desk::new();
        d.floating = vec![AppId::Terminal, AppId::Settings];
        assert!(!d.is_floating(AppId::Terminal) && !d.is_floating(AppId::Settings));
        d.floating = vec![AppId::External(4)];
        assert!(d.is_floating(AppId::External(4)) && d.is_floating(AppId::ExternalWindow(4, 2)), "an app's windows float with it");
        d.floating = vec![AppId::ExternalWindow(4, 2)];
        assert!(d.is_floating(AppId::ExternalWindow(4, 2)) && !d.is_floating(AppId::External(4)), "one extra window floats alone");
    }

    /// W4-1: the bands `sync_levels` sorts are the ones `band_top` reads: an extra window of a floating app goes up with
    /// it, so a built-in window raised afterwards lands above every normal window, not under a stray floating one.
    #[test]
    fn a_raised_built_in_window_goes_above_every_normal_window() {
        let m = metrics();
        let mut d = Desk::new();
        d.open(&m, AppId::Terminal, (800, 500));
        d.open(&m, AppId::ExternalWindow(5, 1), (300, 200));
        d.open(&m, AppId::Settings, (900, 700));
        d.open(&m, AppId::External(25), (1000, 700));
        d.open(&m, AppId::External(5), (400, 300));
        d.sync_levels(vec![AppId::External(5)]);
        let order: Vec<AppId> = d.wins.iter().map(|w| w.app).collect();
        assert_eq!(&order[3..], &[AppId::ExternalWindow(5, 1), AppId::External(5)], "both windows of the floating app are on top");
        let t = d.index_of(AppId::Terminal).unwrap();
        let at = d.raise(t);
        assert_eq!(at, 2, "Terminal goes right under the floating band");
        assert!(d.wins[..at].iter().all(|w| !d.is_floating(w.app)), "no floating window is left under it");
        d.sync_levels(Vec::new());
        let t = d.index_of(AppId::Terminal).unwrap();
        assert_eq!(d.raise(t), d.wins.len() - 1, "with nothing floating, a raise is the very front");
        assert_eq!(d.front().map(|w| w.app), Some(AppId::Terminal));
    }

    /// W4-3, W4-8: a window closed at a tile or the Fill frame is not remembered at that size; a size the person chose is.
    #[test]
    fn a_tile_or_fill_frame_is_not_remembered_as_the_closed_size() {
        let a = Desk::area(&metrics());
        assert!(!remembered_size(a, (a.w, a.h)), "the Fill frame");
        assert!(!remembered_size(a, (a.w / 2, a.h)), "a half tile is as tall as the work area");
        assert!(!remembered_size(a, (a.w, a.h / 2)), "a top or bottom tile is as wide");
        assert!(remembered_size(a, (1200, 825)), "a default-sized window");
        assert!(remembered_size(a, (a.w * 2 / 3, a.h * 2 / 3)), "the Centre tile is a size of its own");
    }

    /// W4-8: an app without a manifest `window_size` opens at its default size, even when its last window was closed
    /// while it filled the work area.
    #[test]
    fn a_window_closed_filling_the_screen_opens_again_at_the_default_size() {
        let mut sh = desktop();
        sh.desk_open(AppId::Photos, 0.0);
        let m = sh.desk_metrics();
        let default = { let d = sh.desk.as_ref().unwrap(); let r = d.wins[d.index_of(AppId::Photos).unwrap()].r; (r.w, r.h) };
        {
            let d = sh.desk.as_mut().unwrap();
            let i = d.index_of(AppId::Photos).unwrap();
            d.wins[i].r = Desk::area(&m);
            d.close(i);
        }
        sh.desk_open(AppId::Photos, 10.0);
        let d = sh.desk.as_ref().unwrap();
        let r = d.wins[d.index_of(AppId::Photos).unwrap()].r;
        assert_eq!((r.w, r.h), default, "the default size, not the work area");
    }

    /// W4-2: a click on the pinned Terminal icon in the Dock opens Terminal in front, with the focus.
    #[test]
    fn a_dock_click_on_terminal_opens_it_in_front() {
        let mut sh = desktop();
        sh.desk_open(AppId::Settings, 0.0);
        sh.desk_open(AppId::Photos, 1.0);
        {
            let p = &mut sh.desk.as_mut().unwrap().dock_preferences.pinned;
            if !p.iter().any(|k| k == "is.olibuijr.term") { p.push("is.olibuijr.term".into()); }
        }
        let m = sh.desk_metrics();
        let t = 5000.0;
        let (list, bar, icons) = sh.desk_dock(&m);
        let (_, icons) = sh.dock_live(bar, &icons, t / 1000.0);
        let k = list.iter().position(|&e| apps().get(e).is_some_and(|a| a.id == AppId::Terminal)).expect("Terminal is in the Dock");
        let r = icons[k];
        let (x, y) = ((r.x + r.w / 2) as f32, (r.y + r.h / 2) as f32);
        sh.desk_event(Event::Down(x, y), t);
        sh.desk_event(Event::Up(x, y), t + 60.0);
        let d = sh.desk.as_ref().unwrap();
        assert_eq!(d.front().map(|w| w.app), Some(AppId::Terminal), "Terminal is the front window");
        assert_eq!(sh.screen, crate::shell::Screen::App(AppId::Terminal), "and has the focus");
    }

    /// W4-5: opening a window that sits behind others (an Overview pick, the Dock, the Window menu) brings it to the very
    /// front with the focus, and the window server is asked to raise it.
    #[test]
    fn opening_a_window_behind_others_brings_it_to_the_very_front() {
        let mut sh = desktop();
        sh.desk_open(AppId::Terminal, 0.0);
        sh.desk_open(AppId::Settings, 1.0);
        sh.desk_open(AppId::Photos, 2.0);
        sh.take_raise_front();
        sh.desk_open(AppId::Terminal, 3.0);
        assert_eq!(sh.desk.as_ref().unwrap().wins.last().map(|w| w.app), Some(AppId::Terminal), "last in the stack");
        assert_eq!(sh.screen, crate::shell::Screen::App(AppId::Terminal));
        assert!(sh.take_raise_front(), "the window server raises and focuses it");
    }

    /// W4-1: raising an app's extra window (the Window menu, the Dock's app menu) asks the window server to raise it too.
    #[test]
    fn focusing_an_extra_window_asks_the_window_server_to_raise_it() {
        let mut sh = desktop();
        let m = sh.desk_metrics();
        let d = sh.desk.as_mut().unwrap();
        d.open(&m, AppId::ExternalWindow(3, 1), (300, 200));
        d.open(&m, AppId::Settings, (900, 700));
        sh.take_raise_front();
        assert!(sh.desk_focus_extra_window(3, 1));
        assert_eq!(sh.desk.as_ref().unwrap().front().map(|w| w.app), Some(AppId::ExternalWindow(3, 1)));
        assert!(sh.take_raise_front());
    }

    /// W4-4: Show Desktop (Esc, the hot corner, the Window menu) draws the whole desktop surface afresh, so nothing left
    /// under the windows stays on the bare desktop.
    #[test]
    fn show_desktop_redraws_the_whole_desktop() {
        let mut sh = desktop();
        sh.desk_open(AppId::Settings, 0.0);
        sh.desk_regions = Some(vec![(0, 0, 10, 10)]);
        sh.desk_show_desktop();
        assert!(sh.desk_regions.is_none(), "no partial frame: the regions of the last one are dropped");
        assert!(matches!(sh.drawn, crate::shell::Drawn::Nothing));
        assert!(sh.desk.as_ref().unwrap().wins.iter().all(|w| w.minimized));
    }

    fn minimized(sh: &crate::shell::Shell, app: AppId) -> bool {
        let d = sh.desk.as_ref().unwrap();
        d.wins[d.index_of(app).unwrap()].minimized
    }

    /// N13: Show Desktop is its own opposite: a second Show Desktop brings back the windows it put aside, in their
    /// order, and a window the person minimized stays in the Dock. The windows aside have no Dock slot.
    #[test]
    fn show_desktop_again_brings_the_windows_back() {
        let mut sh = desktop();
        sh.desk_open(AppId::Settings, 0.0);
        sh.desk_open(AppId::Photos, 1.0);
        sh.desk_open(AppId::Terminal, 2.0);
        let i = sh.desk.as_ref().unwrap().index_of(AppId::Terminal).unwrap();
        sh.desk_minimize_window(i, 3.0);
        sh.desk_show_desktop();
        assert!(minimized(&sh, AppId::Settings) && minimized(&sh, AppId::Photos));
        let m = sh.desk_metrics();
        let (list, _, _) = sh.desk_dock(&m);
        let slots = list.iter().filter(|&&e| crate::shell::desktop_dock::dock_min_index(e).is_some()).count();
        assert_eq!(slots, 1, "only the window the person minimized has a Dock slot");
        sh.desk_show_desktop();
        assert!(!minimized(&sh, AppId::Settings) && !minimized(&sh, AppId::Photos), "the windows came back");
        assert!(minimized(&sh, AppId::Terminal), "the minimized window stays in the Dock");
        assert_eq!(sh.desk.as_ref().unwrap().front().map(|w| w.app), Some(AppId::Photos), "the front window is in front again");
    }

    /// N13: after Show Desktop, Window > Bring All to Front brings the windows back.
    #[test]
    fn bring_all_to_front_brings_back_what_show_desktop_put_aside() {
        let mut sh = desktop();
        sh.desk_open(AppId::Settings, 0.0);
        sh.desk_open(AppId::Photos, 1.0);
        sh.desk_show_desktop();
        sh.desk_bring_all_to_front();
        assert!(!minimized(&sh, AppId::Settings) && !minimized(&sh, AppId::Photos));
        assert_eq!(sh.desk.as_ref().unwrap().front().map(|w| w.app), Some(AppId::Photos));
    }

    #[cfg(feature = "desktop-wayland")]
    fn with_native(sh: &mut crate::shell::Shell, windows: &[(u32, bool)]) {
        sh.desk.as_mut().unwrap().native_windows = windows
            .iter()
            .map(|&(id, active)| crate::bare::wayland::ForeignWindow { id, title: "Film".into(), app_id: format!("x.app{id}"), active, maximized: false, minimized: false })
            .collect();
    }

    /// The window server reports the toplevels `ids` minimized (or shown again).
    #[cfg(feature = "desktop-wayland")]
    fn report_minimized(sh: &mut crate::shell::Shell, ids: &[u32], minimized: bool) {
        for w in sh.desk.as_mut().unwrap().native_windows.iter_mut().filter(|w| ids.contains(&w.id)) {
            w.minimized = minimized;
        }
    }

    /// R3: Show Desktop puts the window server's toplevels (a native app's window) aside with the shell's windows, and
    /// a second Show Desktop brings them back, the window that was active last so it is in front again.
    #[cfg(feature = "desktop-wayland")]
    #[test]
    fn show_desktop_puts_native_windows_aside_and_brings_them_back() {
        let mut sh = desktop();
        sh.desk_open(AppId::Settings, 0.0);
        with_native(&mut sh, &[(7, true), (8, false)]);
        sh.desk_show_desktop();
        assert!(minimized(&sh, AppId::Settings));
        assert_eq!(sh.take_native_minimizes(), vec![8, 7], "both native windows are minimized, the active one last");
        report_minimized(&mut sh, &[7, 8], true);
        sh.desk_show_desktop();
        assert!(!minimized(&sh, AppId::Settings), "the shell's window came back");
        assert_eq!(sh.take_native_activations(), vec![8, 7], "the native windows come back, the active one in front");
        assert!(sh.take_native_minimizes().is_empty());
    }

    /// R3: with only a native window on screen, Show Desktop is not taken for an empty desktop: it puts the window
    /// aside, and a native window the person minimized is left alone. A window put aside counts as gone at once, so
    /// the toggle works before the window server reports it minimized, and going home twice minimizes it once.
    #[cfg(feature = "desktop-wayland")]
    #[test]
    fn show_desktop_with_only_a_native_window_toggles_it() {
        let mut sh = desktop();
        with_native(&mut sh, &[(7, true), (9, false)]);
        report_minimized(&mut sh, &[9], true);
        sh.desk_show_desktop();
        assert_eq!(sh.take_native_minimizes(), vec![7], "the minimized window is left alone");
        sh.desk_show_desktop();
        assert_eq!(sh.take_native_activations(), vec![7], "only the window Show Desktop put aside comes back");
        sh.desk_put_windows_aside();
        sh.desk_put_windows_aside();
        assert_eq!(sh.take_native_minimizes(), vec![7], "put aside once");
    }

    /// R3: after Show Desktop, Bring All to Front brings back the shell's windows and the active app's native windows
    /// (Window > Bring All to Front acts on the active app); another app's native window stays aside until Show
    /// Desktop's toggle brings everything back. The active app is remembered from when the windows went aside, since
    /// the window server reports no toplevel active once they are minimized.
    #[cfg(feature = "desktop-wayland")]
    #[test]
    fn bring_all_to_front_brings_back_only_the_active_apps_native_windows() {
        let mut sh = desktop();
        sh.desk_open(AppId::Settings, 0.0);
        with_native(&mut sh, &[(7, true), (8, false)]);
        sh.desk_show_desktop();
        assert_eq!(sh.take_native_minimizes(), vec![8, 7]);
        report_minimized(&mut sh, &[7, 8], true);
        for w in sh.desk.as_mut().unwrap().native_windows.iter_mut() {
            w.active = false;
        }
        sh.desk_bring_all_to_front();
        assert!(!minimized(&sh, AppId::Settings), "the shell's window came back");
        assert_eq!(sh.take_native_activations(), vec![7], "only the active app's native window came back");
        assert_eq!(sh.desk.as_ref().unwrap().foreign_aside, vec![8], "the other app's window stays aside");
        assert!(sh.desk.as_mut().unwrap().bring_back_put_aside());
        assert_eq!(sh.take_native_activations(), vec![8], "the toggle's bring-back restores everything");
    }

    /// R3: with a shell window's app active (no toplevel active), Bring All to Front brings back the shell's windows
    /// and leaves the native windows aside.
    #[cfg(feature = "desktop-wayland")]
    #[test]
    fn bring_all_to_front_for_a_shell_app_leaves_native_windows_aside() {
        let mut sh = desktop();
        sh.desk_open(AppId::Settings, 0.0);
        with_native(&mut sh, &[(7, false)]);
        sh.desk_show_desktop();
        assert_eq!(sh.take_native_minimizes(), vec![7]);
        report_minimized(&mut sh, &[7], true);
        sh.screen = crate::shell::Screen::App(AppId::Settings);
        sh.desk_bring_all_to_front();
        assert!(!minimized(&sh, AppId::Settings));
        assert!(sh.take_native_activations().is_empty());
        assert_eq!(sh.desk.as_ref().unwrap().foreign_aside, vec![7]);
    }

    /// The window server reports the shell's current native windows (the foreign toplevel state update).
    #[cfg(feature = "desktop-wayland")]
    fn report_windows(sh: &mut crate::shell::Shell) {
        let d = sh.desk.as_mut().unwrap();
        let windows = d.native_windows.clone();
        d.prune_foreign_aside(&windows);
    }

    /// R3: a native window put aside that comes back by itself (the app activates it, or the Dock unminimizes it) is
    /// no longer aside once the window server reports it shown: the next Show Desktop hides it again instead of taking
    /// the bring-back path.
    #[cfg(feature = "desktop-wayland")]
    #[test]
    fn a_native_window_shown_again_is_no_longer_aside() {
        let mut sh = desktop();
        with_native(&mut sh, &[(7, true)]);
        sh.desk_show_desktop();
        assert_eq!(sh.take_native_minimizes(), vec![7]);
        report_minimized(&mut sh, &[7], true);
        report_windows(&mut sh);
        assert_eq!(sh.desk.as_ref().unwrap().foreign_aside, vec![7], "still aside while minimized");
        report_minimized(&mut sh, &[7], false);
        report_windows(&mut sh);
        assert!(sh.desk.as_ref().unwrap().foreign_aside.is_empty(), "shown again: no longer aside");
        sh.desk_show_desktop();
        assert!(sh.take_native_activations().is_empty(), "no bring-back");
        assert_eq!(sh.take_native_minimizes(), vec![7], "the window is put aside again");
    }

    /// R3: a report that predates the minimize (the window still shown) does not end Show Desktop: the window stays
    /// aside and the toggle brings it back.
    #[cfg(feature = "desktop-wayland")]
    #[test]
    fn a_report_before_the_minimize_keeps_the_window_aside() {
        let mut sh = desktop();
        with_native(&mut sh, &[(7, true)]);
        sh.desk_show_desktop();
        assert_eq!(sh.take_native_minimizes(), vec![7]);
        report_windows(&mut sh);
        assert_eq!(sh.desk.as_ref().unwrap().foreign_aside, vec![7]);
        sh.desk_show_desktop();
        assert_eq!(sh.take_native_activations(), vec![7]);
    }

    /// R3: a native window put aside that closes is dropped from the windows aside, and one the shell activates (the
    /// Dock, Overview) is shown again at once.
    #[cfg(feature = "desktop-wayland")]
    #[test]
    fn a_native_window_closed_or_activated_is_no_longer_aside() {
        let mut sh = desktop();
        with_native(&mut sh, &[(7, true), (8, false)]);
        sh.desk_show_desktop();
        assert_eq!(sh.take_native_minimizes(), vec![8, 7]);
        report_minimized(&mut sh, &[7, 8], true);
        sh.desk.as_mut().unwrap().native_windows.retain(|w| w.id != 8);
        report_windows(&mut sh);
        assert_eq!(sh.desk.as_ref().unwrap().foreign_aside, vec![7], "the closed window is dropped");
        sh.desk_activate_foreign(7);
        assert!(sh.desk.as_ref().unwrap().foreign_aside.is_empty(), "the activated window is no longer aside");
        assert_eq!(sh.take_native_activations(), vec![7]);
    }

    /// N13: going home on the desktop (the assistive Home, a Screen Time block) only ever puts the windows aside; it
    /// never brings them back the way a second Show Desktop does.
    #[test]
    fn putting_the_windows_aside_twice_keeps_them_aside() {
        let mut sh = desktop();
        sh.desk_open(AppId::Settings, 0.0);
        sh.desk_put_windows_aside();
        sh.desk_put_windows_aside();
        assert!(minimized(&sh, AppId::Settings));
    }

    /// N8: when the Terminal's shell exits, only the Terminal's window closes; the other windows stay on the desktop.
    #[test]
    fn a_terminal_whose_shell_exits_closes_only_its_window() {
        let mut sh = desktop();
        sh.desk_open(AppId::Settings, 0.0);
        sh.desk_open(AppId::Terminal, 1.0);
        sh.term.clear();
        sh.anim = None;
        sh.tick(100.0);
        let d = sh.desk.as_ref().unwrap();
        assert!(d.index_of(AppId::Terminal).is_none(), "the Terminal's window closed");
        assert!(!minimized(&sh, AppId::Settings), "Settings stays on the desktop");
        assert_eq!(sh.screen, crate::shell::Screen::App(AppId::Settings));
    }
}

#[cfg(all(test, feature = "desktop-wayland"))]
mod launch_front_tests {
    use crate::shell::tests::{desktop_shell, fixture_app, hand_back_after_launch};
    use crate::shell::{AppId, Screen};

    /// E2E6 1: a Dock click on an app that is not running yet opens it in front, and the keyboard the window server
    /// hands back to the window it still knows in front does not put that window over the launching app.
    #[test]
    fn a_dock_launch_stays_in_front_through_a_keyboard_hand_back() {
        let mut sh = desktop_shell();
        sh.screen = Screen::Home;
        sh.anim = None;
        let _peer = fixture_app(&mut sh);
        sh.desk_open(AppId::Settings, 0.0);
        sh.desk_dock_tap(AppId::External(0), 1.0);
        assert_eq!(hand_back_after_launch(&mut sh), (Some(AppId::External(0)), Screen::App(AppId::External(0))));
    }

    /// The guard holds only while every window above is still launching: a drawn window above (a built-in) leaves the
    /// hand-back as it was, a raise of the window it names.
    #[test]
    fn a_hand_back_with_a_drawn_window_above_still_raises() {
        let mut sh = desktop_shell();
        sh.screen = Screen::Home;
        sh.anim = None;
        let _peer = fixture_app(&mut sh);
        sh.desk_open(AppId::Settings, 0.0);
        sh.desk_open(AppId::External(0), 1.0);
        let m = sh.desk_metrics();
        sh.desk.as_mut().unwrap().open(&m, AppId::Magnifier, (600, 400));
        let i = sh.desk.as_ref().unwrap().index_of(AppId::Settings).unwrap();
        assert!(!sh.desk_launching_above(i), "the built-in above draws its own content");
        assert_eq!(hand_back_after_launch(&mut sh), (Some(AppId::Settings), Screen::App(AppId::Settings)));
    }
}

/// E2E6 Spotlight: whatever Spotlight, the Apps grid, the Dock or `open-app` opens is the front window and keeps the
/// keyboard, whatever focus the window server gives other windows on its own while the app maps (`FrontIntent`).
#[cfg(all(test, feature = "desktop-wayland"))]
mod spotlight_front_tests {
    use super::{win_id, FRONT_INTENT_MS};
    use crate::bare::wayland::WindowEvent;
    use crate::shell::tests::{desktop_shell, fixture_app};
    use crate::shell::{AppId, Screen, Shell};

    fn front(sh: &Shell) -> (Option<AppId>, Screen) {
        (sh.desk.as_ref().unwrap().front().map(|w| w.app), sh.screen)
    }

    fn id(app: AppId) -> u32 {
        win_id(app).unwrap()
    }

    /// Settings in front, Calculator launched: the window server's own focus of Settings (its keyboard enter or an
    /// activating configure, not only the hand-back as Spotlight goes) does not lift Settings over the launch.
    #[test]
    fn a_launch_over_a_built_in_stays_in_front_through_the_window_servers_focus() {
        let mut sh = desktop_shell();
        sh.screen = Screen::Home;
        sh.anim = None;
        let _peer = fixture_app(&mut sh);
        sh.desk_open(AppId::Settings, 0.0);
        sh.desk_open(AppId::External(0), 10.0);
        let want = (Some(AppId::External(0)), Screen::App(AppId::External(0)));
        sh.wayland_window_event(WindowEvent::HandBack(id(AppId::Settings)));
        assert_eq!(front(&sh), want, "the hand-back as Spotlight's keyboard goes");
        sh.wayland_window_event(WindowEvent::ServerFocus(id(AppId::Settings)));
        assert_eq!(front(&sh), want, "a later keyboard enter of the window that was in front");
        sh.desk_intent_tick(10.0 + FRONT_INTENT_MS / 2.0);
        assert_eq!(sh.desk_front_intent(), Some(AppId::External(0)), "the intent holds within its bound");
        sh.desk_intent_tick(10.0 + FRONT_INTENT_MS + 1.0);
        assert_eq!(sh.desk_front_intent(), None, "and ends after it");
        sh.wayland_window_event(WindowEvent::ServerFocus(id(AppId::Settings)));
        assert_eq!(front(&sh), (Some(AppId::Settings), Screen::App(AppId::Settings)), "the window server's later focus is followed again");
    }

    /// Weather (an external app's window) in front, Settings opened from Spotlight: the keyboard the window server hands
    /// back to Weather does not put Weather over Settings, and Settings, drawn already, is activated again.
    #[test]
    fn a_built_in_opened_over_an_external_app_stays_in_front_and_is_activated_again() {
        let mut sh = desktop_shell();
        sh.screen = Screen::Home;
        sh.anim = None;
        let _peer = fixture_app(&mut sh);
        let m = sh.desk_metrics();
        let weather = AppId::ExternalWindow(0, 3);
        sh.desk.as_mut().unwrap().open(&m, AppId::Settings, (900, 700));
        sh.desk.as_mut().unwrap().open(&m, weather, (600, 400));
        sh.screen = Screen::App(AppId::External(0));
        sh.desk_open(AppId::Settings, 5.0);
        assert!(sh.take_raise_front(), "the open asks for the activation");
        let want = (Some(AppId::Settings), Screen::App(AppId::Settings));
        sh.wayland_window_event(WindowEvent::HandBack(id(weather)));
        assert_eq!(front(&sh), want);
        assert!(sh.take_raise_front(), "the keyboard went to Weather: Settings is activated again");
        sh.wayland_window_event(WindowEvent::ServerFocus(id(weather)));
        assert_eq!(front(&sh), want);
        assert!(sh.take_raise_front(), "each focus the window server gives elsewhere asks again");
        sh.wayland_window_event(WindowEvent::ServerFocus(id(AppId::Settings)));
        assert_eq!(front(&sh), want, "the focus of the intended window itself is followed");
        assert!(!sh.take_raise_front(), "and asks for nothing more");
    }

    /// Weather in front, Calendar launched: Mail, a third app's window the window server picks as its next toplevel
    /// while Calendar has none yet, does not come to the front, nor does Weather after it.
    #[test]
    fn a_launch_over_an_external_app_ignores_the_window_servers_next_toplevel() {
        let mut sh = desktop_shell();
        sh.screen = Screen::Home;
        sh.anim = None;
        let _weather_peer = fixture_app(&mut sh);
        let _calendar_peer = fixture_app(&mut sh);
        let m = sh.desk_metrics();
        let (mail, weather) = (AppId::Terminal, AppId::ExternalWindow(0, 3));
        sh.desk.as_mut().unwrap().open(&m, mail, (900, 600));
        sh.desk.as_mut().unwrap().open(&m, weather, (600, 400));
        sh.desk_open(AppId::External(1), 20.0);
        let want = (Some(AppId::External(1)), Screen::App(AppId::External(1)));
        sh.wayland_window_event(WindowEvent::ServerFocus(id(mail)));
        assert_eq!(front(&sh), want, "the window server's topmost pick");
        sh.wayland_window_event(WindowEvent::HandBack(id(weather)));
        assert_eq!(front(&sh), want, "the hand-back to the window that was in front");
        let d = sh.desk.as_ref().unwrap();
        assert_eq!(d.wins.iter().rev().nth(1).map(|w| w.app), Some(weather), "the stack below is left as it was");
    }

    /// The per-tick refocus never takes the screen from the app being brought to the front, and a press (the person's
    /// choice) ends the intent at once.
    #[test]
    fn the_per_tick_refocus_keeps_the_intended_app_and_a_press_ends_it() {
        let mut sh = desktop_shell();
        sh.screen = Screen::Home;
        sh.anim = None;
        let _peer = fixture_app(&mut sh);
        sh.desk_open(AppId::Settings, 0.0);
        sh.desk_open(AppId::External(0), 1.0);
        sh.screen = Screen::App(AppId::Settings);
        sh.desk_sync_extra_windows();
        assert_eq!(sh.screen, Screen::App(AppId::External(0)), "the tick puts the intended app back");
        sh.wayland_window_event(WindowEvent::ServerFocus(id(AppId::Settings)));
        sh.desk_intent_tick(2.0);
        sh.desk_sync_extra_windows();
        assert_eq!(front(&sh), (Some(AppId::External(0)), Screen::App(AppId::External(0))));
        sh.wayland_window_event(WindowEvent::Focus(id(AppId::Settings)));
        assert_eq!(front(&sh), (Some(AppId::Settings), Screen::App(AppId::Settings)), "a press raises the window below");
        assert_eq!(sh.desk_front_intent(), None);
        assert!(sh.desk.as_ref().unwrap().front_intent.is_none());
    }

    /// The window server reports the intended window's toplevel but another one active: the shell asks again, once per
    /// report, and stops once it is active.
    #[test]
    fn an_inactive_intended_toplevel_is_activated_again() {
        let mut sh = desktop_shell();
        sh.screen = Screen::Home;
        sh.anim = None;
        sh.desk_open(AppId::Settings, 0.0);
        sh.take_raise_front();
        let toplevel = |active| crate::bare::wayland::ForeignWindow { id: 40, title: "Settings".into(), app_id: format!("is.olibuijr.bifrost.{}", id(AppId::Settings)), active, maximized: false, minimized: false };
        let d = sh.desk.as_mut().unwrap();
        d.foreign_snapshot = vec![toplevel(false)];
        d.intent_check = true;
        sh.desk_intent_tick(100.0);
        assert!(sh.take_raise_front(), "mapped but not active: activated again");
        sh.desk_intent_tick(110.0);
        assert!(!sh.take_raise_front(), "once per report");
        let d = sh.desk.as_mut().unwrap();
        d.foreign_snapshot = vec![toplevel(true)];
        d.intent_check = true;
        sh.desk_intent_tick(120.0);
        assert!(!sh.take_raise_front(), "active: nothing to ask");
    }
}
