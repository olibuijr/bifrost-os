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

mod layout;
pub use layout::*;

mod draw;
use draw::*;
pub(super) use draw::{dim_rect, soft_shadow, BACKGROUND_DIM, SHADOW_REACH_PT};

/// Waits for (reaps) the started apps whose process has exited and forgets them. A quit app nobody waited for stays a
/// zombie, and a zombie still counts in its cgroup's `pids.current`, so the CPU and RAM menus listed quit apps as
/// running (E2E N14, W5-13). Running children are kept.
#[cfg_attr(not(feature = "desktop-wayland"), allow(dead_code))]
fn reap_exited(children: &mut std::collections::HashMap<String, std::process::Child>) {
    children.retain(|_, c| matches!(c.try_wait(), Ok(None)));
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
    pub(super) restore: Rect,
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
    /// `dy` px come off the y of every event: a press on the toolbar revealed under the menu bar over a full-screen window
    /// (`reveal_hit`) reaches the app at the row the band shows, and the drag it starts keeps that mapping until it lifts
    Content(AppId, f32),
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
    /// the look the pixels were drawn in (`window_look`): a Light/Dark switch, its fade or a new accent redraws the
    /// window at once instead of after the once-a-second refresh or the next hover (E2E13 finding 3)
    look:u64,
}

#[cfg(feature = "desktop-wayland")]
impl CachedWindow {
    /// Whether these pixels are at the size (`pw`, `ph`) the window is presented at now (`presented_size`): a client
    /// window whose app frame is smaller than the window is cached at that smaller size, so comparing the cache with the
    /// window's own frame never matched and the window was drawn afresh on every desk frame.
    fn sized(&self, (pw, ph): (i32, i32)) -> bool {
        self.width == pw.max(0) as usize && self.height == ph.max(0) as usize
    }
}

/// The appearance a window's cached pixels depend on: the scheme position (0 dark .. 1 light, moving during the fade)
/// and the accent colour. The position is quantised (`appearance::scheme_step`): keyed on the raw value every window
/// was redrawn in full on every frame of the 280 ms fade; now a quiet window redraws a few times and on the end value.
#[allow(dead_code)]
fn window_look() -> u64 {
    ((bifrost_render::appearance::scheme_step() as u64) << 32) | bifrost_render::appearance::accent().to_wire() as u64
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

/// The app the shell is bringing to the front (Search, the Apps grid, the Dock, `open-app`): opened at `since`
/// (shell ms). While it stands (`Shell::desk_front_intent`) a focus the window server gives another window on its own
/// (a hand-back as Search's exclusive keyboard goes, a keyboard enter, an activating configure) is not followed, and
/// the shell asks the window server again to activate the app's window once it is mapped (E2E6 Search).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct FrontIntent {
    pub app: AppId,
    pub since: f64,
}

#[derive(Default)]
pub struct Desk {
    /// the app the shell is bringing to the front, until it is in front for good (`FrontIntent`)
    pub(super) front_intent: Option<FrontIntent>,
    /// The active app that has no window: an SDK app that ordered its last window out while it was in front (Cmd+W on
    /// its last document). As on the reference it keeps the menu bar, the Dock dot and its place in Cmd+Tab, and the
    /// window under it does not take the front, until another window is raised or opened (`raise`, `open`) or the app
    /// gets a window back (`Shell::desk_windowless_front`). A `Cell` so that the read that finds it no longer holds (the
    /// process went, or a window came back) clears it there and then: it never comes back once a window shows again.
    pub(super) windowless_front: std::cell::Cell<Option<AppId>>,
    /// the window server's toplevel list changed since the front intent last looked at it (`Shell::desk_intent_tick`)
    #[cfg(feature = "desktop-wayland")]
    pub(super) intent_check: bool,
    /// back to front; the floating windows (`floating`) are the last ones
    pub wins: Vec<Win>,
    /// the displays the desktop spans; with fewer than two, every window uses `Desk::area` of the one screen
    pub(super) displays: super::displays::Arrangement,
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
    #[cfg(any(test, feature = "desktop-wayland"))]
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
    /// where a press on a Dock icon silenced the hover label: the label stays away until the pointer moves from here
    pub(super) dock_label_off:Option<(f32,f32)>,
    /// launch bounces: (`desk_dock` list entry, start in seconds) until the app's first window maps
    pub(super) dock_bounce:Vec<(usize,f64)>,
    /// the outer size each app's window had when it was last closed (the window comes back at it when reopened)
    pub(super) closed_sizes:std::collections::HashMap<AppId,(i32,i32)>,
    /// the windows Hide put away (Cmd+H, Hide Others): they are `minimized` but not in the Dock's minimized section, and
    /// they come back with Show All or when their app is activated (`raise`)
    pub(super) hidden: Vec<AppId>,
    /// the window server's toplevels (native and Linux apps) Hide minimized, so Show All can bring them back
    #[cfg(feature = "desktop-wayland")]
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
    /// the frames the last `take_wayland_windows` handed to the window server, without their pixels (the presenter
    /// holds those), in stacking order: `frames` is emptied by every present, so a window that starts closing finds
    /// its last frame here (`Shell::desk_keep_ghost`). Replaced on every present, so it never outlives the windows.
    #[cfg(feature = "desktop-wayland")]
    presented: Vec<crate::bare::wayland::WindowFrame>,
    /// an open or focus asked for the front window to be raised over native clients
    pub(super) raise_front: bool,
    /// windows brought forward together (Bring All to Front, a hidden app shown, the Apps grid closing): the window
    /// server is told to stack them in the shell's order over other clients' windows (`Shell::take_wayland_restack`)
    pub(super) restack: Vec<AppId>,
    /// the lock took the windows' toplevels down while one of the shell's windows was in front: when the unlock ends
    /// they come back in the shell's order over other clients' windows (`Shell::desk_restack_after_unlock`), not in the
    /// order the window server maps them again (E2E13 2)
    pub(super) restack_on_unlock: bool,
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

/// The most rects a borderless window's input region holds before `alpha_shape` merges them into boxes.
#[cfg(any(test, feature = "desktop-wayland"))]
pub const MAX_SHAPE_RECTS: usize = 512;

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

/// Whether frame `r` is a tile of the work area `a`: as tall or as wide as the work area (a half, the Fill frame,
/// `remembered_size`), or within 3 % of one of the tiles on both sides (a quarter, the Centre). The window server sizes
/// a tile; the shell keeps no flag for one, as the frame comes back from it (`desk_tile_window`).
#[cfg_attr(not(feature = "desktop-wayland"), allow(dead_code))]
fn tiled_frame(a: Rect, r: Rect) -> bool {
    use Tile::*;
    let near = |t: Rect| (t.w - r.w).abs() * 100 <= a.w * 3 && (t.h - r.h).abs() * 100 <= a.h * 3;
    !remembered_size(a, (r.w, r.h)) || [TopLeft, TopRight, BottomLeft, BottomRight, Center].into_iter().any(|t| near(tile_rect(a, t)))
}

/// The size (px) window `w` of a client app is drawn and presented at when the app's newest frame is `frame` px: the
/// window's own frame, or, while that frame is smaller than the window's body (the app was configured to a larger size and
/// has not drawn at it yet), only what the frame covers plus the title bar, as a compositor keeps a surface at its last
/// buffer's size until the client commits a new one. The window never paints its own black past the app's pixels
/// (0.10.39: Mail restored at 2560x1309 drew at 1200x860; the rest of the window was black over the wallpaper). A
/// full-screen window, a window with no frame yet and a frame that covers the body keep the window's frame, and so do a
/// zoomed (maximized) and a tiled window (`tiled_frame`): xdg-shell forbids
/// a maximized surface committing smaller than the size it was configured to, and the window server sized a tile.
#[cfg_attr(not(feature = "desktop-wayland"), allow(dead_code))]
pub fn presented_size(w: &Win, m: &Metrics, frame: Option<(usize, usize)>) -> (i32, i32) {
    let whole = (w.r.w.max(1), w.r.h.max(1));
    let held = w.full || w.maximized || tiled_frame(Desk::area(m), w.r);
    let Some((fw, fh)) = frame.filter(|&(fw, fh)| fw > 0 && fh > 0 && !held) else { return whole };
    let b = w.body(m);
    let top = (b.y - w.r.y).max(0);
    let fw = i32::try_from(fw).unwrap_or(i32::MAX);
    let fh = i32::try_from(fh).unwrap_or(i32::MAX);
    if fw >= b.w && fh >= b.h {
        return whole;
    }
    (fw.min(b.w).max(1), (top + fh.min(b.h)).max(1))
}

/// The menu bar shown over a full-screen window, with the window's title bar or toolbar revealed directly under it, as
/// the reference shows full screen: `top` is the bar's height in px, where the band starts, and `band` the band's height.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Reveal {
    pub top: i32,
    pub band: i32,
}

/// What a point in the revealed band of a full-screen window is (`reveal_hit`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RevealHit {
    /// a traffic light, shown in the revealed band (the window hides them while the bar is hidden)
    Light(Light),
    /// the app's toolbar, at this y in the window (px): the band shows the window's top rows moved down by `Reveal::top`
    App(f32),
    /// the title strip the shell draws for a titled window: it covers content, so the press goes nowhere
    Bar,
}

/// The height of the band a full-screen window reveals under the menu bar, in px: a full-size window's unified toolbar
/// (`UNIFIED_BAR_PT`), or a titled window's title bar.
pub fn reveal_band(w: &Win, m: &Metrics) -> i32 {
    if w.is_full_size() { (UNIFIED_BAR_PT * m.pt).round() as i32 } else { m.title }
}

/// Full-screen window `w` as its revealed band lays it out: the window's title bar or toolbar at `rv.top`, `rv.band` tall,
/// as a zoomed window with no edges, so `Win::hit` and `Win::light_rect` place the traffic lights in the band.
pub fn revealed_win(w: &Win, rv: Reveal) -> Win {
    let mut b = *w;
    b.full = false;
    b.maximized = true;
    b.r = Rect { x: w.r.x, y: rv.top, w: w.r.w, h: rv.band };
    b
}

/// What is at screen point (`x`, `y`) of the band full-screen window `w` reveals under the menu bar, or `None` outside the
/// band. The band is the window's top `rv.band` rows drawn `rv.top` px lower, so a point on it is the app's `rv.top` higher.
pub fn reveal_hit(w: &Win, m: &Metrics, rv: Reveal, x: f32, y: f32) -> Option<RevealHit> {
    if rv.band <= 0 || y < rv.top as f32 || y >= (rv.top + rv.band) as f32 || x < w.r.x as f32 || x >= (w.r.x + w.r.w) as f32 {
        return None;
    }
    Some(match revealed_win(w, rv).hit(m, x, y) {
        Some(Hit::Light(l)) => RevealHit::Light(l),
        _ if w.is_full_size() => RevealHit::App(y - rv.top as f32 + w.r.y as f32),
        _ => RevealHit::Bar,
    })
}

pub fn drag_zone(w: &Win, m: &Metrics, controls: &[bifrost_proto::Node]) -> (i32, Vec<[i32; 4]>) {
    // a full-screen window does not move: no press on its toolbar (or on the band revealed under the menu bar, which the
    // window server sees at the window's own rows) starts a window server move
    if w.full {
        return (0, Vec::new());
    }
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
    /// The press that began a drag session ends with the shell taking its release (`dnd_event`): the window must not go on
    /// hearing the pointer moves of the buttonless pointer as a held drag.
    pub(super) fn end_grab(&mut self){self.grab=None;}
    /// Whether the Dock keeps magnifying: with no grab, or while a Dock icon is pressed but not dragged, so the row does
    /// not jump back to rest under the pointer between the press and the release (W2-23).
    pub(super) fn dock_grab_magnifies(&self)->bool{matches!(self.grab,None|Some(Grab::DockDrag{moved:false,..}))}
    pub fn new() -> Self {
        Desk {dock_visible:true,..Default::default()}
    }

    pub fn index_of(&self, app: AppId) -> Option<usize> {
        self.wins.iter().position(|w| w.app == app)
    }

    /// Whether `app` has any window on the desk: its main window or one of its extra windows. An app whose main window
    /// was closed and that keeps a document window (Mail's compose window) still has one, though `index_of` finds none.
    pub fn has_window_of(&self, app: AppId) -> bool {
        self.wins.iter().any(|w| w.app.parent() == app.parent())
    }

    pub fn front(&self) -> Option<&Win> {
        self.wins.iter().rev().find(|w| !w.minimized)
    }

    /// Brings window `i` to the front (and shows it); returns its new index. A hidden app comes back whole: raising one
    /// of its windows (the Dock, Cmd+Tab, the Window menu) shows every window Hide put away, as activating a hidden app does.
    pub fn raise(&mut self, i: usize) -> usize {
        // a window coming to the front makes its app the active one
        self.windowless_front.set(None);
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
    #[cfg(any(test, feature = "desktop-wayland"))]
    pub(super) fn hide_app(&mut self, parent: AppId) -> bool {
        self.hide_where(|a| a.parent() == parent)
    }

    /// Hide Others (Option+Cmd+H): every app but `keep` (a parent id; None hides every app's windows) is hidden.
    #[cfg(any(test, feature = "desktop-wayland"))]
    pub(super) fn hide_others(&mut self, keep: Option<AppId>) -> bool {
        self.hide_where(|a| Some(a.parent()) != keep)
    }

    #[cfg(any(test, feature = "desktop-wayland"))]
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
    #[cfg(any(test, feature = "desktop-wayland"))]
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
    #[cfg(any(test, feature = "desktop-wayland"))]
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
    #[cfg(any(test, feature = "desktop-wayland"))]
    pub(super) fn any_hidden(&self) -> bool {
        self.wins.iter().any(|w| w.minimized && self.hidden.contains(&w.app))
    }

    /// Whether a window of an app other than `parent` is on screen (Hide Others has something to hide).
    #[cfg(any(test, feature = "desktop-wayland"))]
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

    /// The visible frame of the display `r` is on: the work area on the main display (below the menu bar, inside the
    /// Dock's edge), the whole frame on any other. One display is `Self::area`.
    pub fn area_for(&self, m: &Metrics, r: Rect) -> Rect {
        if self.displays.is_multi() { self.displays.visible_for(r, m) } else { Self::area(m) }
    }

    /// Opens (or raises) the window for `app`; `size` is the wanted outer size.
    pub fn open(&mut self, m: &Metrics, app: AppId, size: (i32, i32)) -> usize {
        self.windowless_front.set(None);
        if let Some(i) = self.index_of(app) {
            // opening a hidden app (the Dock, Cmd+Tab, Search) activates it: its windows come back in the order Hide
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
        let a = self.area_for(m, r);
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

    /// The app's own resize of window `i` to a content size in points (`ToShell::SetContentSize`, the reference window's set-content-size call).
    /// The top-left corner stays where it is (moved only as far as the work area needs); the size is held to the app's
    /// limits (a fixed-size window takes it as asked: only the person cannot resize it) and to the work area. A zoomed
    /// or full-screen window keeps its frame and takes the size as the one it goes back to. Returns whether the frame
    /// changed.
    pub fn set_content_size(&mut self, m: &Metrics, i: usize, size: (f32, f32)) -> bool {
        let Some(win) = self.wins.get(i).copied() else { return false };
        let a = self.area_for(m, win.r);
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
        let (screen, area) = (self.displays.frame_for(self.wins[i].r, m), self.area_for(m, self.wins[i].r));
        let screen = if self.displays.is_multi() { screen } else { Self::screen_rect(m) };
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
            w.r = screen;
            w.maximized = false;
        } else if let Some(k) = self.full_from_zoom.iter().position(|a| *a == w.app) {
            self.full_from_zoom.remove(k);
            w.r = area;
            w.maximized = true;
        } else {
            w.r = w.restore;
        }
        w.full = on;
        true
    }

    /// The frame window `i` is written down with (the session file, an update relaunch) and whether it comes back zoomed:
    /// a normal window's own frame; a zoomed one's frame from before the zoom, zoomed again on the way back; a full-screen
    /// one's way back from full screen (`set_full_screen`), never the screen it covers. Saving the full-screen frame
    /// brought the window back after a restart as a normal window held to the work area, a near-screen frame its app
    /// had never drawn at (0.10.39: Mail at 0,35,2560,1309 with black where its pixels ended).
    pub fn saved_frame(&self, i: usize) -> Option<(Rect, bool)> {
        let w = self.wins.get(i)?;
        Some(if w.full {
            (w.restore, self.full_from_zoom.contains(&w.app))
        } else if w.maximized {
            (w.restore, true)
        } else {
            (w.r, false)
        })
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
        let a = self.area_for(m, self.wins[i].r);
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
        let a = self.area_for(m, self.wins[i].r);
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
        // the frame the user left the window at (a zoomed or full-screen one goes back to the frame before it, never
        // the screen it covered: that opened the next window flush against the menu bar at its default size)
        let r = if w.maximized || w.full { w.restore } else { w.r };
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
        if self.displays.is_multi() {
            // the desktop is every display: the window may cross onto another, and its top is held inside the display
            // most of it lands on (under the menu bar on the main one, which alone has it)
            let at = Rect { x, y, w: w.r.w, h: w.r.h };
            let (b, k) = (self.displays.bounds().unwrap_or(Rect { x: 0, y: 0, w: m.w, h: m.h }), self.displays.index_for(at));
            let (f, top) = (self.displays.frame(k, m), if k == self.displays.main_index() { m.menubar } else { 0 });
            w.r.x = x.clamp(b.x + keep - w.r.w, (b.x + b.w - keep).max(b.x + keep - w.r.w));
            w.r.y = y.clamp(f.y + top, (f.y + f.h - m.title).max(f.y + top));
            return;
        }
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
pub(super) const CALENDAR_TILE: &str = "is.olibuijr.calendar.desktop";

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

/// Like `dock_rects` with a divider after each index in `seps` (pinned | recents | minimized and Trash).
/// One step through `n` items from `i`, wrapping: forwards, or backwards when `reverse`.
#[cfg(any(test, feature = "desktop-wayland"))]
pub(super) fn cycle_step(i: usize, n: usize, reverse: bool) -> usize {
    if n == 0 { 0 } else if reverse { (i + n - 1) % n } else { (i + 1) % n }
}

/// The next Cmd+` stop: the order to walk and the window to bring forward. `stack` is the front app's windows, front
/// first. A walk continues while the window in front is the one it raised last and the app has the same windows;
/// otherwise a new walk starts from the stack as it is. None with fewer than two windows.
#[cfg(any(test, feature = "desktop-wayland"))]
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
#[cfg(feature = "desktop-wayland")]
pub(super) fn frame_app(id: u32) -> Option<AppId> {
    if id & WINDOW_ID_FLAG != 0 {
        return Some(AppId::ExternalWindow(((id >> 16) & 0xff) as u8, (id & 0xffff) as u16));
    }
    apps().get((id as usize).checked_sub(1)?).map(|a| a.id)
}

/// The title a frame carries to the window server: the app's name (an extra window shows its parent's name here).
#[cfg(feature = "desktop-wayland")]
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
        if self.wayland {
            return self.retain_wayland_windows();
        }
        true
    }

    /// Whether a frame for an external app's new pixels may draw the windows alone: the settled desktop on the window
    /// server's path (the conditions of the bar-only frame), nothing animating and nothing floating over the windows.
    #[cfg(feature = "desktop-wayland")]
    pub(super) fn ext_windows_only_ok(&self, t: f64) -> bool {
        self.wayland
            && self.desk_active() && self.desk_drawn && !self.is_locked() && self.anim.is_none()
            && self.desktop_menu.is_none() && !self.cc_active() && !self.center_active() && !self.overlay_visible() && !self.quick_look.is_visible() && !self.utility_up()
            && !self.overview_active() && !self.desk_animating(t)
            // the band revealed under the menu bar copies the full-screen window's toolbar into the shell's frame: new
            // pixels from the app must redraw it too (`draw_desk_reveal`)
            && self.desk_reveal().is_none()
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
            d.presented = frames.iter().map(|f| crate::bare::wayland::WindowFrame { pixels: None, ..f.clone() }).collect();
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
            // Under the lock and while the unlock slides it away the toplevels map again in the order they draw and the
            // window server focuses them as they come: raising here would rewrite the shell's stack, so the order the unlock
            // restacks to (`desk_restack_after_unlock`) is the one from before the lock
            WindowEvent::ServerFocus(_) | WindowEvent::HandBack(_) if self.desk_lock_holds_order() => trace("window server focus ignored: locked or unlocking"),
            // The app the shell is bringing to the front (Search, the Apps grid, the Dock) stays in front against the
            // window server's own focus of another window: the keyboard it hands back to the window it knows in front as
            // the exclusive layer goes (Settings over a Calculator that drew its first frame but has no toplevel yet), the
            // topmost toplevel it picks (Mail while Calendar starts), or a late keyboard enter of the window that was in
            // front (Weather over Settings). Raising that window here put it over the app just opened in the shell's own
            // stack, so the app's toplevel then mapped behind it without the keyboard (E2E6 Search). Once the intended
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
            // server hands back as Search or the Apps grid closes went to the window it still knows in front (E2E6 1).
            // Only that hand-back: a press on the window below (a `Focus`) is the person's choice and raises it
            WindowEvent::HandBack(_) if self.desk_launching_above(i) => trace("hand-back ignored: the app above it is launching"),
            // the active app closed its last window and stays active with none (`Desk::windowless_front`): the focus the
            // window server gives the toplevel under the one that went is not an activation of that window's app (E2E13 1)
            WindowEvent::ServerFocus(_) | WindowEvent::HandBack(_) if self.desk_windowless_front().is_some() => trace("window server focus ignored: the active app has no window"),
            WindowEvent::ServerFocus(_) | WindowEvent::HandBack(_) => {
                trace(if matches!(ev, WindowEvent::HandBack(_)) { "hand-back: raised" } else { "window server focus: raised" });
                self.desk.as_mut().unwrap().raise(i); if !self.is_locked() { self.screen = Screen::App(app.parent()); } self.dirty = true;
            },
            WindowEvent::Close(_) => self.desk_close_requested(app),
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
        if self.wayland {
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
        if self.wayland {
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
    #[cfg(feature = "desktop-wayland")]
    pub(super) fn desk_bar_hidden(&self) -> bool {
        // a menu opened from the revealed bar keeps the bar while the pointer is down in the menu
        // a screen recording keeps the bar: its indicator must stay visible for the whole recording, even over a full-screen window
        self.desktop_menu.is_none() && !self.recording_active() && self.desk_bar_hidden_by_pointer()
    }

    /// The pointer rule alone of `desk_bar_hidden`: the front window is full screen and the pointer has not brought the
    /// bar back at the top edge (nor stayed on the toolbar revealed under it since). The band under the bar
    /// (`desk_reveal`) follows only this: a recording or an open menu keeps the bar but does not reveal the toolbar.
    fn desk_bar_hidden_by_pointer(&self) -> bool {
        let full = self.desk.as_ref().is_some_and(|d| d.front_full());
        // once the bar is back, the toolbar revealed under it keeps it while the pointer is on that toolbar
        let keep = if self.bar_latched { self.desk_full_band().map_or(0.0, |(_, b)| b as f32) } else { 0.0 };
        bar_hidden(full, self.pointer_y, self.desktop_bar_height() as f32, keep)
    }

    /// The front window when it is full screen: its index and the height of the band it reveals under the menu bar
    /// (`reveal_band`).
    pub(super) fn desk_full_band(&self) -> Option<(usize, i32)> {
        let d = self.desk.as_ref()?;
        let i = d.wins.iter().rposition(|w| !w.minimized)?;
        let w = d.wins.get(i).filter(|w| w.full)?;
        Some((i, reveal_band(w, &self.desk_metrics())))
    }

    /// The menu bar is shown over a full-screen front window: that window's index and the band its title bar or toolbar
    /// takes directly under the bar (`Reveal`). The window does not reflow: the band is its top rows drawn lower, over
    /// its content, with the traffic lights in it (`draw_desk_reveal`). `None` while the bar is hidden.
    pub(super) fn desk_reveal(&self) -> Option<(usize, Reveal)> {
        if !self.desk_active() || self.is_locked() {
            return None;
        }
        let (i, band) = self.desk_full_band()?;
        // the pointer brought the bar back: a bar kept by a recording or an open menu reveals no toolbar
        if self.desk_bar_hidden_by_pointer() {
            return None;
        }
        Some((i, Reveal { top: self.desktop_bar_height() as i32, band }))
    }

    /// What screen point (`x`, `y`) is in the band revealed under the menu bar over a full-screen window (`reveal_hit`),
    /// with the window's index; `None` outside the band or while the bar is hidden.
    pub(super) fn desk_reveal_at(&self, x: f32, y: f32) -> Option<(usize, RevealHit)> {
        let (i, rv) = self.desk_reveal()?;
        let w = self.desk.as_ref()?.wins.get(i)?;
        reveal_hit(w, &self.desk_metrics(), rv, x, y).map(|h| (i, h))
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

    #[cfg(feature = "desktop-wayland")]
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
        if !cfg!(feature = "desktop-wayland") || !self.wayland { return; }
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
        // `frames` holds this tick's frames only until the next present takes them; after that the last presented
        // frame is the one the window server still shows
        let (z, mut frame) = match d.frames.iter().position(|f| f.id == id) {
            Some(z) => (z, d.frames[z].clone()),
            None => match d.presented.iter().position(|f| f.id == id) { Some(z) => (z, d.presented[z].clone()), None => return },
        };
        frame.damage.all();
        frame.fx = bifrost_render::glass::Fx::IDENTITY;
        // the toplevel is already placed and mapped: nothing to move or map again
        frame.origin = None;
        frame.position = None;
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
        if self.wayland { list.push(apps().len()); }
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
        // a running app that ordered its main window out itself (its last document closed: `WindowClose` id 0) gets the
        // window back and is told (`ToApp::Reopen`), so a document app with no document shows a new one (E2E12 2). Only
        // an activation does this: the app ordering its window out never reopens it. The first show of an app started in
        // the background (a login item, a notification or share target) or of a direct app that fell back to shared
        // memory is not a reopen: the app made its window and expects no `Reopen`.
        let reopened = match app {
            AppId::External(slot) if !existed => self
                .ext
                .get_mut(slot as usize)
                .and_then(Option::as_mut)
                .filter(|h| h.running())
                .is_some_and(|h| std::mem::take(&mut h.main_ordered_out)),
            _ => false,
        };
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
        if let (true, AppId::External(slot)) = (reopened, app) {
            self.ext_send(slot as usize, &bifrost_proto::ToApp::Reopen);
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
    #[cfg(any(test, feature = "desktop-wayland"))]
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

    /// A lock begins (`sleep_screen`): when no other client's window is in front, the shell's windows are restacked in
    /// its order once the unlock ends. A lock while already locked keeps what the first one saw.
    pub(super) fn desk_note_lock(&mut self) {
        if self.is_locked() {
            return;
        }
        let foreign = self.foreign_front_window().is_some();
        if let Some(d) = self.desk.as_mut() {
            d.restack_on_unlock = !foreign;
        }
    }

    /// The screen is locked, or the lock is sliding in or out (`Anim::Unlock`): the window server's focus of a shell
    /// window then raises nothing (`wayland_window_event`).
    #[cfg(feature = "desktop-wayland")]
    pub(super) fn desk_lock_holds_order(&self) -> bool {
        self.is_locked() || matches!(self.anim, Some((super::Anim::Unlock { .. }, _)))
    }

    /// The unlock ended (the lock or login window is gone): the windows' toplevels map again in whatever order they draw,
    /// so the window server is told every shown window, bottom first, as the Apps grid does when it closes (E2E13 2).
    pub(super) fn desk_restack_after_unlock(&mut self) {
        if let Some(d) = self.desk.as_mut() {
            if std::mem::take(&mut d.restack_on_unlock) {
                let shown: Vec<AppId> = d.wins.iter().filter(|w| !w.minimized).map(|w| w.app).collect();
                d.note_restack(&shown);
            }
        }
    }

    pub(super) fn desk_refocus(&mut self) {
        // the lock screen stays up whatever the windows do: a per-tick refocus must never lift a window over it
        if self.is_locked() {
            self.dirty = true;
            return;
        }
        // the active app with no window keeps the front (`Desk::windowless_front`)
        self.screen = match self.desk_windowless_front().or_else(|| self.desk.as_ref().and_then(|d| d.front()).map(|w| w.app)) {
            Some(app) => Screen::App(app.parent()),
            None => Screen::Home,
        };
        self.dirty = true;
    }

    /// The window that draws as the active one (key traffic lights and title, the large shadow): the topmost shown window,
    /// unless the active app has no window (`desk_windowless_front`), when none does: the window under it belongs to
    /// another app, which is not the active one.
    pub(super) fn desk_active_window_index(&self) -> Option<usize> {
        if self.desk_windowless_front().is_some() {
            return None;
        }
        self.desk.as_ref().and_then(|d| d.wins.iter().rposition(|w| !w.minimized && !d.unmapped.contains(&w.app)))
    }

    /// The active app that has no window (`Desk::windowless_front`) while that still holds: its process runs and none
    /// of its windows (main or extra) is on the desk. The screen, the menu bar and the application menu follow it.
    pub(super) fn desk_windowless_front(&self) -> Option<AppId> {
        let d = self.desk.as_ref()?;
        let app = d.windowless_front.get()?;
        let valid = match app {
            AppId::External(slot) => {
                let alive = self.ext.get(slot as usize).and_then(Option::as_ref).is_some_and(|h| h.running());
                alive && !d.wins.iter().any(|w| w.app.parent() == app)
            }
            _ => false,
        };
        // stale (the process went, or a window came back): cleared, so it does not return when that window goes again
        if !valid {
            d.windowless_front.set(None);
        }
        valid.then_some(app)
    }

    /// The running SDK apps that ordered their main window out (`AppHost::main_ordered_out`): they keep their Dock dot
    /// and their place in Cmd+Tab with no window, and activating one sends `ToApp::Reopen` (`desk_open`).
    pub(super) fn ext_windowless_apps(&self) -> Vec<AppId> {
        self.ext
            .iter()
            .enumerate()
            .filter(|(_, h)| h.as_ref().is_some_and(|h| h.running() && h.main_ordered_out))
            .filter_map(|(slot, _)| u8::try_from(slot).ok().map(AppId::External))
            .collect()
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

    /// The person closes a window (the red traffic light, Cmd+W, the window server's close request). An installed SDK app
    /// that announced `ToShell::CloseRequests` is sent `ToApp::CloseRequest` for its main window (`WindowCloseRequest` for
    /// an extra window) and nothing else happens here: it asks "Save changes?" for an edited document and orders the window out itself (`ToShell::WindowClose` id 0,
    /// `desk_order_main_out`) when it is done, or keeps it on Cancel. Every other window and app closes as `desk_close_app`
    /// does. Quitting (Dock, Cmd+Q, shutdown) and the shell's own cleanups use `desk_close_app` and are never vetoed.
    pub(super) fn desk_close_requested(&mut self, app: AppId) {
        // an extra window's close control asks the app about that window, as Cmd+W in it does (a save sheet for an edited
        // document); closing it here took the window down before the app could answer, and the document came back in a
        // new window behind the main one
        if let AppId::ExternalWindow(slot, id) = app {
            let asks = self.ext.get(slot as usize).and_then(Option::as_ref).is_some_and(|h| h.running() && h.close_requests && h.window_open(u32::from(id)));
            if asks {
                self.ext_send(slot as usize, &bifrost_proto::ToApp::WindowCloseRequest { id: u32::from(id) });
                return;
            }
        }
        if let AppId::External(slot) = app {
            if self.ext.get(slot as usize).and_then(Option::as_ref).is_some_and(|h| h.running() && h.close_requests) {
                self.ext_send(slot as usize, &bifrost_proto::ToApp::CloseRequest);
                if let Some(h) = self.ext.get_mut(slot as usize).and_then(Option::as_mut) {
                    h.close_pending = true;
                }
                return;
            }
        }
        self.desk_close_app(app);
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

    /// The app in `slot` ordered its main window out (`ToShell::WindowClose` id 0: its last document closed): the main
    /// desk window closes with the close effect and the app keeps running with no window, its extra windows (if any)
    /// left as they are. Nothing reopens it until the person activates the app (`desk_open`, which sends `Reopen`) or
    /// the app asks for it back (`desk_order_main_in`). A direct app's window is the window server's: nothing here.
    pub(super) fn desk_order_main_out(&mut self, slot: u8) {
        let app = AppId::External(slot);
        // the next activation reopens the window and sends `Reopen`, whether or not the desk showed it yet
        if let Some(h) = self.ext.get_mut(slot as usize).and_then(Option::as_mut) {
            h.main_ordered_out = true;
        }
        if !self.desk.as_ref().is_some_and(|d| d.index_of(app).is_some()) {
            return;
        }
        // the app in front stays the active app with no window, its menu bar kept (File > New works), as on the
        // reference: the window under it does not take the front (E2E13 1)
        let was_front = self.screen == Screen::App(app);
        let t = self.last_input;
        self.desk_fx_start(app, FxCall::Close, t);
        #[cfg(feature = "desktop-wayland")]
        self.desk_keep_ghost(app);
        if let Some(d) = &mut self.desk {
            if let Some(i) = d.index_of(app) {
                d.close(i);
                self.heap_trim_due = true;
            }
            // only while none of the app's windows is left: with an extra window still shown that window is in front
            let has_windows = d.wins.iter().any(|w| w.app.parent() == app);
            if was_front && !has_windows {
                d.windowless_front.set(Some(app));
            } else if d.windowless_front.get() == Some(app) {
                d.windowless_front.set(None);
            }
        }
        self.desk_refocus();
        self.dirty = true;
    }

    /// The app in `slot` asked for its ordered-out main window back (`ToShell::WindowOpen` id 0: a document made while it
    /// had no window). It opens as an activation would, while the app is running and has no main desk window.
    pub(super) fn desk_order_main_in(&mut self, slot: u8, t: f64) {
        let app = AppId::External(slot);
        // the app ordered it in itself: it already has what the window shows, so this is no reopen (no `Reopen`)
        if let Some(h) = self.ext.get_mut(slot as usize).and_then(Option::as_mut) {
            h.main_ordered_out = false;
        }
        let running = self.ext.get(slot as usize).and_then(Option::as_ref).is_some_and(|h| h.running());
        if running && self.desk.as_ref().is_some_and(|d| d.index_of(app).is_none()) {
            self.desk_open(app, t);
        }
    }

    /// The person quits an app (Cmd+Q, the application menu's or the Dock's Quit). An installed SDK app that announced
    /// `ToShell::CloseRequests` is sent `ToApp::QuitRequest` and nothing else happens here: it asks about its unsaved
    /// documents and exits itself once it may, or stays on Cancel. Every other app quits as `desk_quit_app` does. Logout,
    /// shutdown, updates and the shell's own cleanups use `desk_quit_app` and are never vetoed.
    #[cfg_attr(not(feature = "desktop-wayland"), allow(dead_code))]
    pub(super) fn desk_quit_requested(&mut self, slot: u8) {
        if self.ext.get(slot as usize).and_then(Option::as_ref).is_some_and(|h| h.running() && h.close_requests) {
            self.ext_send(slot as usize, &bifrost_proto::ToApp::QuitRequest);
            return;
        }
        self.desk_quit_app(slot);
    }

    /// Quits the installed SDK app in `slot`: its desk windows go and the app is sent `Quit`, so it saves its state and exits
    /// (`AppHost::begin_quit` kills it after the grace period). Logout, shutdown and updates use it (the person's Quit goes through `desk_quit_requested`). A direct app's window is the
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
                        // from the app's newest other extra window, else from its main window (the reference toolkit's cascade)
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
        let mut main_orders = Vec::new();
        let mut raises = Vec::new();
        for (slot, host) in self.ext.iter_mut().enumerate() {
            let Some(h) = host else { continue };
            if let Some(window) = h.take_raise() {
                raises.push((slot as u8, window));
            }
            if let Some(p) = h.take_main_move() {
                main_moves.push((slot as u8, p));
            }
            if let Some(shown) = h.take_main_order() {
                // the order-out answers a close request on the direct route: that window can only go with the app
                let quits = !shown && apphost::order_out_quits(h.route(), h.close_pending);
                h.close_pending = false;
                main_orders.push((slot as u8, shown, quits));
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
        // a window the app asked to bring to the front (`WindowRaise`)
        for (slot, window) in raises {
            changed |= self.desk_raise_app_window(slot, window);
        }
        // the main window moves where its app asked (`WindowMove` with window 0); its collapse is the desk's own
        for (slot, p) in main_moves {
            let app = AppId::External(slot);
            let collapsed = self.desk.as_ref().is_some_and(|d| d.is_collapsed(app));
            changed |= self.ext_window_place(app, Some(p), collapsed);
        }
        // the main window ordered out (its app's last document closed) or back in (E2E12 2)
        for (slot, shown, quits) in main_orders {
            if shown {
                let t = self.last_input;
                self.desk_order_main_in(slot, t);
            } else if quits {
                self.desk_quit_app(slot);
            } else {
                self.desk_order_main_out(slot);
            }
            changed = true;
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
            let front = self.desk_front_intent().or_else(|| self.desk_windowless_front()).or_else(|| self.desk.as_ref().and_then(|d| d.front()).map(|w| w.app));
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

    /// Raises the window of an app that asked for it (`ToShell::WindowRaise`): an extra window by its id, 0 is the app's main
    /// window. False when the window is not on the desk.
    fn desk_raise_app_window(&mut self, slot: u8, window: u32) -> bool {
        if window == 0 {
            let Some(d) = self.desk.as_mut() else { return false };
            let Some(i) = d.index_of(AppId::External(slot)) else { return false };
            d.raise(i);
            d.raise_front = true;
            self.desk_refocus();
            self.dirty = true;
            return true;
        }
        u16::try_from(window).is_ok_and(|id| self.desk_focus_extra_window(slot, id))
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
        self.note_wheel_pan(t);
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
                    // a click puts the name away until the pointer moves (it would otherwise come back once the press ends,
                    // over the Applications grid or the window overview, with the pointer still on the icon)
                    self.desk.as_mut()?.dock_label_off = Some((x, y));
                    self.desk.as_mut()?.dock_drop = None;
                    // an app that already has a window comes forward on the press, not after the release
                    if let Some(a) = apps().get(list[k]).map(|a| Self::dock_app(a.id)) {
                        if self.desk.as_ref()?.index_of(a).is_some() && !self.desk_activate_shown_app(a, t) { self.desk_open(a, t); }
                    }
                    self.dirty = true;
                    return Some(vec![]);
                }
                // over a full-screen window with the menu bar back, the band under the bar shows the window's toolbar (or
                // title bar) and its traffic lights (`reveal_hit`): a press there is on those, at the row the band shows
                let revealed = self.desk_reveal_at(x, y);
                if matches!(revealed, Some((_, RevealHit::Bar))) {
                    return Some(vec![]);
                }
                let dy = match revealed { Some((_, RevealHit::App(ly))) => y - ly, _ => 0.0 };
                let picked = match revealed {
                    Some((i, RevealHit::Light(l))) => Some((i, Hit::Light(l))),
                    Some((i, RevealHit::App(_))) => Some((i, Hit::Content)),
                    _ => self.desk.as_ref()?.pick(&m, x, y),
                };
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
                        self.desk.as_mut()?.grab = Some(Grab::Content(app, dy));
                        let b = w.body(&m);
                        let fx = self.with_size(b.w, b.h, |sh| sh.app_touch_single(app, Event::Down(x - b.x as f32, y - dy - b.y as f32), t));
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
                    Grab::Content(app, dy) => {
                        let d = self.desk.as_ref()?;
                        let w = *d.wins.get(d.index_of(app)?)?;
                        let b = w.body(&m);
                        let fx = self.with_size(b.w, b.h, |sh| sh.app_touch_single(w.app, Event::Move(x - b.x as f32, y - dy - b.y as f32), t));
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
                    Some(Grab::Content(app, dy)) => {
                        let d = self.desk.as_ref()?;
                        let w = *d.wins.get(d.index_of(app)?)?;
                        let b = w.body(&m);
                        let fx = self.with_size(b.w, b.h, |sh| sh.app_touch_single(w.app, Event::Up(x - b.x as f32, y - dy - b.y as f32), t));
                        Some(fx)
                    }
                    Some(Grab::Light(app, l)) => {
                        let i = self.desk.as_ref()?.index_of(app)?;
                        let w = *self.desk.as_ref()?.wins.get(i)?;
                        // a full-screen window's lights are in the band revealed under the menu bar (`reveal_hit`)
                        let on_light = w.hit(&m, x, y) == Some(Hit::Light(l))
                            || self.desk_reveal_at(x, y).is_some_and(|(ri, h)| ri == i && h == RevealHit::Light(l));
                        if on_light {
                            match l {
                                Light::Close => self.desk_close_requested(w.app),
                                // a full-screen window does not minimize (its yellow light is dimmed, as the reference's is)
                                Light::Minimize if w.full => {}
                                Light::Minimize => {
                                    self.desk_minimize_window(i, t);
                                }
                                // the green button of an app window takes it full screen (its own space); in full screen any
                                // window's green button takes it back
                                Light::Zoom if w.full || matches!(w.app, AppId::External(_)) => {
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
                        if let (Some(tile), Some(i)) = (snap_zone(self.desk.as_ref()?.area_for(&m, Rect { x: x as i32, y: y as i32, w: 1, h: 1 }), x as i32, y as i32, band), self.desk.as_ref()?.index_of(moved)) {
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
    /// `i` (Search or the Apps grid closing, a menu closing) is not a choice of the person to put `i` over the app
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
        let front = self.desk_active_window_index();
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
            if self.wayland {
                let id = win_id(w.app).unwrap_or(0);
                let focused = Some(i) == front;
                let hover = self.desk.as_ref().unwrap().lights_hover == Some(w.app);
                let mut cached = self.desk.as_mut().unwrap().window_cache.remove(&id);
                let look = window_look();
                // an app's window is drawn at the size its newest frame covers while that frame is smaller than the
                // window (configured to a new size, not drawn at it yet): the rest was the body's black over the
                // wallpaper (0.10.39: Mail restored at 2560x1309 behind other windows, its frame 1200x860)
                let app_frame = match w.app {
                    AppId::External(slot) => self.ext.get(slot as usize).and_then(Option::as_ref).filter(|h| h.running()).and_then(|h| h.frame()).map(|(_, fw, fh)| (fw, fh)),
                    _ => None,
                };
                let (pw, ph) = presented_size(w, &m, app_frame);
                // the cache holds the pixels at the size they were presented at (`presented_size`), not the window's frame
                let unchanged = cached.as_ref().is_some_and(|c|
                    c.sized((pw, ph)) && c.focused == focused && c.maximized == w.maximized && c.hover == hover && c.look == look);
                // only the key state moved (Cmd-Tab, a click on a back window): a full-size app's window is its body plus
                // the lights over it, so the body is copied again from the app's last frame (dimmed when it lost focus)
                // instead of drawing the whole window afresh (a 31 ms frame on the lab PC, round 75)
                let refocus = !unchanged && w.is_full_size() && matches!(w.app, AppId::External(_)) && cached.as_ref().is_some_and(|c|
                    c.sized((pw, ph)) && c.focused != focused && c.maximized == w.maximized && c.hover == hover && c.look == look);
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
                    // dropped so that it repaints in full once it is uncovered. It keeps the size it was last presented
                    // at: a frame without pixels never names a size its pixels lack (the presenter holds it to them too,
                    // `hold_size_without_pixels`: a new size with the old pixels sliced past their end)
                    let (kw, kh) = cached.as_ref().map_or((pw as usize, ph as usize), |c| (c.width, c.height));
                    self.desk.as_mut().unwrap().hidden_ids.push(id);
                    self.desk.as_mut().unwrap().frames.push(crate::bare::wayland::WindowFrame {
                        id, title: frame_title(id), x:w.r.x, y:w.r.y,
                        width:kw, height:kh, pixels:None,
                        damage:crate::gfx::Damage::new(kw,kh), maximized:w.maximized, fullscreen:w.full, title_height:m.title,
                        drag_height, no_drag, fx:bifrost_render::glass::Fx::IDENTITY, opaque, floating:false, origin:None,position:None, min_size:(0,0),input:None,
                    });
                    continue;
                }
                if unchanged && quiet {
                    // the pixels held are at the presented size, which `unchanged` compared
                    let (kw, kh) = (pw as usize, ph as usize);
                    self.desk.as_mut().unwrap().frames.push(crate::bare::wayland::WindowFrame {
                        id, title: frame_title(id), x:w.r.x, y:w.r.y,
                        width:kw, height:kh, pixels:None,
                        damage:crate::gfx::Damage::new(kw,kh), maximized:w.maximized, fullscreen:w.full, title_height:m.title,
                        drag_height, no_drag, fx:bifrost_render::glass::Fx::IDENTITY, opaque, floating:false, origin:None,position:None, min_size:(0,0),input:None,
                    });
                    self.desk.as_mut().unwrap().window_cache.insert(id,cached.unwrap());
                    continue;
                }
                if let AppId::External(slot)=w.app {
                    // the retained pixels are at the presented size (the app's frame plus the title bar while that frame
                    // is smaller than the window), so the body the frame is copied into is too
                    let width=pw as usize; let height=ph as usize; let top=w.title_h(&m) as usize;
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
                        if w.is_full_size() && !bare && !w.full {
                            // the lights sit over the app: repaint them where the app just drew under them (a full-screen
                            // window hides them; the band revealed under the menu bar shows them, `draw_desk_reveal`)
                            let mut local=*w; local.r.x=0; local.r.y=0; local.r.w=pw; local.r.h=ph;
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
                    c.sized((pw, ph)) && c.focused != focused && c.maximized == w.maximized && c.look == look
                    && c.drawn > self.last_input && c.drawn > self.settings_changed && t - c.drawn < 1000.)
                    && (w.app != AppId::Settings || self.settings.as_ref().is_some_and(|st| st.next_wake(t).map_or(true, |x| x > t)));
                self.desk.as_mut().unwrap().reuse_body = quiet_body.then_some(w.app);
                // Reuse the shared scratch buffer instead of a fresh zeroed allocation per window per frame.
                let mut px = std::mem::take(&mut self.desk.as_mut().unwrap().window_buf);
                px.clear(); px.resize(pw as usize * ph as usize, 0);
                let mut image = Image { w: pw as usize, h: ph as usize, px };
                let mut local = *w; local.r.x = 0; local.r.y = 0; local.r.w = pw; local.r.h = ph;
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
                    width:image.w,height:image.h,pixels:pixels.clone(),focused,maximized:w.maximized,hover,drawn:t,look,
                });
                let pixels = (!damage.is_empty()).then_some(pixels);
                let desk = self.desk.as_mut().unwrap();
                desk.window_buf = image.px;
                desk.frames.push(crate::bare::wayland::WindowFrame {
                    id, title: frame_title(id), x:w.r.x, y:w.r.y,
                    width:pw as usize, height:ph as usize, pixels, damage, maximized:w.maximized, fullscreen:w.full, title_height:m.title,
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

    /// The band a full-screen window reveals under the menu bar while the bar is shown over it (`desk_reveal`), as the
    /// reference's full screen slides the title bar and toolbar down with the bar: a full-size window's toolbar (its top
    /// rows as last presented, `window_cache`) or a titled window's title bar, at `Reveal::top`, with the traffic lights.
    /// The window itself does not move or reflow; the band covers the top of its content. It is drawn into the shell's
    /// frame and shown above the window by its own chrome surface (`chrome_ids::REVEAL`).
    #[cfg(feature = "desktop-wayland")]
    pub(super) fn draw_desk_reveal(&self, s: &mut Surface) {
        let Some((i, rv)) = self.desk_reveal() else { return };
        let Some(d) = self.desk.as_ref() else { return };
        let Some(w) = d.wins.get(i).copied() else { return };
        let m = self.desk_metrics();
        let b = revealed_win(&w, rv);
        let (x0, width, band) = (b.r.x.max(0) as usize, b.r.w.max(0) as usize, rv.band.max(0) as usize);
        let frame = super::ui::WINDOW_FRAME_FOCUSED.now();
        if w.is_full_size() {
            let cached = win_id(w.app).and_then(|id| d.window_cache.get(&id)).filter(|c| c.width == w.r.w.max(0) as usize && c.height >= band);
            let top = b.r.y.max(0) as usize;
            // without the window server the windows are drawn into this frame: the toolbar's rows are moved down in place
            // (bottom row first, as the band overlaps them), before the bar is drawn over the top rows
            let in_frame = !self.wayland && s.y0 == 0 && s.h >= top + band && x0 < s.w;
            match cached {
                Some(c) => copy_rect(s, &c.pixels[..c.width * band], c.width, band, b.r.x, b.r.y),
                None if in_frame => {
                    let n = width.min(s.w - x0);
                    for row in (0..band).rev() {
                        let (from, to) = (row * s.stride + x0, (top + row) * s.stride + x0);
                        s.px.copy_within(from..from + n, to);
                    }
                }
                None => s.fill_rect(x0, top, width, band, frame),
            }
        } else {
            s.fill_rect(x0, b.r.y.max(0) as usize, width, band, frame);
        }
        // the hairline under a toolbar or title bar (the kit's `toolbar_band` draws it on the band's last row), across
        // the whole band: it now sits over content, so it also parts the band from the rows under it
        if band > 0 {
            s.fill_rect(x0, (b.r.y + rv.band - 1).max(0) as usize, width, 1, SEPARATOR_C.now());
        }
        if !w.is_full_size() {
            let doc = match w.app {
                AppId::External(slot) => self.ext.get(slot as usize).and_then(Option::as_ref).map(|h| h.doc_title.clone()).filter(|t| !t.is_empty()),
                _ => None,
            };
            let name = doc.unwrap_or_else(|| apps().iter().find(|a| a.id == w.app.parent()).map_or(String::new(), |a| a.name.to_string()));
            self.label(&name, MAC_BODY_PT, Weight::Semibold, LABEL.now(), |im| s.blit(im, b.r.x + (b.r.w - im.w as i32) / 2, b.r.y + (rv.band - im.h as i32) / 2, 255, 255));
        }
        let hover = d.lights_hover == Some(w.app);
        for (k, c) in [SYSTEM_RED, SYSTEM_YELLOW, SYSTEM_GREEN].into_iter().enumerate() {
            let lr = b.light_rect_px(&m, k);
            // a full-screen window does not minimize: its yellow light is dimmed, as the reference's is
            let off = k == 1;
            draw_light(s, &lr, if off { TRAFFIC_INACTIVE_C.now() } else { c }, !off, self.ui.pt(1.));
            if hover && !off {
                // the green light of a full-screen window leaves full screen: its glyph is the two inward arrowheads
                if k == 2 {
                    draw_exit_full_screen_glyph(s, &lr, self.ui.pt(1.));
                } else {
                    draw_light_glyph(s, &lr, k, false, self.ui.pt(1.));
                }
            }
        }
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
        let persistent = self.wayland && matches!(w.app, AppId::External(_)) && !self.keyboard.shown() && !self.keyboard.animating();
        #[cfg(not(feature = "desktop-wayland"))]
        let persistent = false;
        // a built-in on the Wayland path keeps its last (undimmed) body too, so a key-state change can reuse it
        #[cfg(feature = "desktop-wayland")]
        let builtin = self.wayland && !matches!(w.app, AppId::External(_));
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
    /// weekday (red) over the day number (black), as the Dock shows the date. `(x, y, w, h)` is the tile; `alpha`
    /// (0 to 255) fades the face with the icon under it, as the Applications grid animates in and out.
    pub(super) fn calendar_tile_face(&self, s: &mut Surface, scale: f32, (x, y, w, h): (f32, f32, f32, f32), (wday, mday): (i32, i32), alpha: u32) {
        let (day, num) = calendar_face_text(wday, mday);
        // the paper covers the baked picture between the squircle's rounded sides
        s.fill_rrect(x + w * 0.16, y, w * 0.68, h, 0.0, rgba(255, 255, 255, alpha.min(255) as u8));
        let cx = x + w / 2.0;
        self.label(day, 0.2 * h / scale, Weight::Semibold, SYSTEM_RED, |im| {
            s.blit(im, (cx - im.w as f32 / 2.0) as i32, (y + 0.06 * h) as i32, 255, alpha);
        });
        self.label(&num, 0.5 * h / scale, Weight::Regular, BLACK, |im| {
            s.blit(im, (cx - im.w as f32 / 2.0) as i32, (y + 0.34 * h) as i32, 255, alpha);
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
                self.calendar_tile_face(s, ui.s, (r.x as f32, r.y as f32, r.w as f32, r.h as f32), (self.info.wday, self.info.mday), 255);
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
        if let Some(k) = label_slot.filter(|_| desk.grab.is_none() && desk.dock_label_off.is_none() && !self.dock_label_hidden_by_menu()) {
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
impl Win {
    fn light_rect_px(&self, m: &Metrics, i: usize) -> Rect {
        self.light_rect(m, i)
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

#[cfg(test)]
mod tests;

#[cfg(test)]
mod retained_damage_tests;

#[cfg(test)]
mod rim_tests;

#[cfg(test)]
mod resize_grab_tests;

#[cfg(test)]
mod dim_tests;

#[cfg(test)]
mod window_shadow_tests;

impl Shell {
    /// Runs `f` on the rendered `text`, taken from the chrome text cache (rendered once per string and style).
    pub(super) fn label<R>(&self, text: &str, pt: f32, weight: Weight, color: u32, f: impl FnOnce(&Image) -> R) -> R {
        let mut cache = self.chrome_text.borrow_mut();
        f(cache.get(&self.ui, text, pt, weight, color))
    }
}

#[cfg(all(test, feature = "desktop-wayland"))]
mod material_rect_tests;

#[cfg(test)]
mod calendar_tile_tests;

#[cfg(test)]
mod keyboard_lift_tests;

/// The window server's frame of a toplevel moves the shell's rectangle (E2E N1).
#[cfg(all(test, feature = "desktop-wayland"))]
mod server_frame_tests;

#[cfg(test)]
mod cmdtab_material_tests;

#[cfg(test)]
mod w3_window_tests;

#[cfg(test)]
mod w4_window_tests;

#[cfg(all(test, feature = "desktop-wayland"))]
mod launch_front_tests;

/// E2E6 Search: whatever Search, the Apps grid, the Dock or `open-app` opens is the front window and keeps the
/// keyboard, whatever focus the window server gives other windows on its own while the app maps (`FrontIntent`).
#[cfg(all(test, feature = "desktop-wayland"))]
mod search_front_tests;

/// Full screen as the reference shows it: the menu bar slides in at the top edge with the window's toolbar directly
/// under it (never under the bar), the traffic lights in that band, and both go again once the pointer leaves them.
#[cfg(all(test, feature = "desktop-wayland"))]
mod fullscreen_reveal_tests;
