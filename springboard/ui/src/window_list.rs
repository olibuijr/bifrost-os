//! Window list: the windows on the desktop, front to back, as the window server reports them. The shell fills the
//! records from its real window state (see `springboard/src/shell/services/window_services.rs`); this module holds the
//! record, the list options an app can ask for (every window or only the ones on screen, the on-screen windows stacked
//! above or below one window, the exclusion of desktop elements), the window levels, and the wire form shared by the
//! shell and the SDK client. A window's title is private to apps that may record the screen: [`redact_names`] drops it
//! for the others. Records keep the stacking order they were reported in, front first.

use crate::wire_json::*;
use crate::Rect;
use bifrost_core::json::Json;

/// The layer a normal window sits on. A window on a layer below this one is a desktop element (the wallpaper and
/// the icons drawn on it).
pub const NORMAL_LAYER: i32 = 0;

/// How a window's contents may be read by other processes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SharingState {
    /// Other processes cannot read the contents.
    None,
    /// Other processes may read the contents.
    ReadOnly,
    /// Other processes may read and write the contents.
    ReadWrite,
}

impl SharingState {
    pub fn code(self) -> u32 {
        match self {
            SharingState::None => 0,
            SharingState::ReadOnly => 1,
            SharingState::ReadWrite => 2,
        }
    }

    pub fn from_code(c: u32) -> SharingState {
        match c {
            1 => SharingState::ReadOnly,
            2 => SharingState::ReadWrite,
            _ => SharingState::None,
        }
    }
}

/// One window record. `name` is `None` when the window has no title or the caller may not read titles.
#[derive(Clone, Debug, PartialEq)]
pub struct WindowInfo {
    pub number: u32,
    pub owner_pid: u32,
    pub owner_name: String,
    pub name: Option<String>,
    pub layer: i32,
    pub bounds: Rect,
    pub alpha: f32,
    pub on_screen: bool,
    pub sharing: SharingState,
}

impl WindowInfo {
    pub fn to_json(&self) -> Json {
        obj(vec![
            ("number", num(self.number)),
            ("owner_pid", num(self.owner_pid)),
            ("owner_name", text(&self.owner_name)),
            ("name", self.name.as_deref().map_or(Json::Null, text)),
            ("layer", num(self.layer)),
            ("bounds", rect_json(self.bounds)),
            ("alpha", num(self.alpha)),
            ("on_screen", Json::Bool(self.on_screen)),
            ("sharing", num(self.sharing.code())),
        ])
    }

    pub fn from_json(j: &Json) -> Option<WindowInfo> {
        Some(WindowInfo {
            number: u32_of(j, "number")?,
            owner_pid: u32_of(j, "owner_pid").unwrap_or(0),
            owner_name: str_of(j, "owner_name").unwrap_or_default(),
            name: str_of(j, "name"),
            layer: i32_of(j, "layer").unwrap_or(NORMAL_LAYER),
            bounds: rect_of(j, "bounds")?,
            alpha: f32_of(j, "alpha").unwrap_or(1.0),
            on_screen: bool_of(j, "on_screen").unwrap_or(false),
            sharing: SharingState::from_code(u32_of(j, "sharing").unwrap_or(0)),
        })
    }
}

/// The wire form of a window list.
pub fn list_to_json(list: &[WindowInfo]) -> Json {
    Json::Arr(list.iter().map(WindowInfo::to_json).collect())
}

/// Decodes a window list; records that do not parse are skipped.
pub fn list_from_json(j: &Json) -> Vec<WindowInfo> {
    j.as_array().map(|a| a.iter().filter_map(WindowInfo::from_json).collect()).unwrap_or_default()
}

/// Drops the title of every window that the caller does not own. An app always reads its own titles; the others need
/// the permission to record the screen.
pub fn redact_names(list: &mut [WindowInfo], caller_pid: u32) {
    for w in list.iter_mut() {
        if w.owner_pid != caller_pid {
            w.name = None;
        }
    }
}

/// Which windows a list includes. The on-screen above and below scopes compare stacking positions with the window
/// named by the number; that window is never in its own list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowScope {
    /// Every window, on screen or not.
    All,
    /// Only the windows that are on screen.
    OnScreenOnly,
    /// The on-screen windows stacked above the window with this number. An unknown number gives an empty list.
    OnScreenAbove(u32),
    /// The on-screen windows stacked below the window with this number. An unknown number gives an empty list.
    OnScreenBelow(u32),
}

/// The options of a window list: a scope and whether desktop elements are left out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WindowListOptions {
    pub scope: WindowScope,
    pub exclude_desktop_elements: bool,
}

impl WindowListOptions {
    /// Every window, desktop elements included.
    pub fn all() -> Self {
        WindowListOptions { scope: WindowScope::All, exclude_desktop_elements: false }
    }

    pub fn to_json(&self) -> Json {
        let (scope, reference) = match self.scope {
            WindowScope::All => ("all", 0),
            WindowScope::OnScreenOnly => ("on_screen", 0),
            WindowScope::OnScreenAbove(n) => ("above", n),
            WindowScope::OnScreenBelow(n) => ("below", n),
        };
        obj(vec![("scope", text(scope)), ("reference", num(reference)), ("exclude_desktop", Json::Bool(self.exclude_desktop_elements))])
    }

    /// `None` for an unknown scope name. A missing field means the default (every window).
    pub fn from_json(j: &Json) -> Option<WindowListOptions> {
        let reference = u32_of(j, "reference").unwrap_or(0);
        let scope = match str_of(j, "scope").as_deref().unwrap_or("all") {
            "all" => WindowScope::All,
            "on_screen" => WindowScope::OnScreenOnly,
            "above" => WindowScope::OnScreenAbove(reference),
            "below" => WindowScope::OnScreenBelow(reference),
            _ => return None,
        };
        Some(WindowListOptions { scope, exclude_desktop_elements: bool_of(j, "exclude_desktop").unwrap_or(false) })
    }
}

/// A desktop element: a window on a layer below the normal layer.
pub fn is_desktop_element(window: &WindowInfo) -> bool {
    window.layer < NORMAL_LAYER
}

/// The position of the window with this number in the stacking order (0 is the front-most), or `None` when no record
/// has the number.
pub fn stacking_index(windows: &[WindowInfo], number: u32) -> Option<usize> {
    windows.iter().position(|w| w.number == number)
}

/// Whether window `a` is stacked above window `b`. `None` when either number is unknown.
pub fn is_above(windows: &[WindowInfo], a: u32, b: u32) -> Option<bool> {
    let ia = stacking_index(windows, a)?;
    let ib = stacking_index(windows, b)?;
    Some(ia < ib)
}

/// The records the options select, in the same stacking order as `windows`.
pub fn window_list(windows: &[WindowInfo], options: WindowListOptions) -> Vec<WindowInfo> {
    let reference = match options.scope {
        WindowScope::OnScreenAbove(n) | WindowScope::OnScreenBelow(n) => stacking_index(windows, n),
        _ => None,
    };
    let mut out = Vec::new();
    for (i, w) in windows.iter().enumerate() {
        if options.exclude_desktop_elements && is_desktop_element(w) {
            continue;
        }
        let keep = match options.scope {
            WindowScope::All => true,
            WindowScope::OnScreenOnly => w.on_screen,
            WindowScope::OnScreenAbove(_) => w.on_screen && reference.map_or(false, |r| i < r),
            WindowScope::OnScreenBelow(_) => w.on_screen && reference.map_or(false, |r| i > r),
        };
        if keep {
            out.push(w.clone());
        }
    }
    out
}

// ---- window levels ----

/// The named window levels: a window on a higher level is drawn over the ones on lower levels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowLevelKey {
    Base,
    Minimum,
    Desktop,
    Backstop,
    Normal,
    Floating,
    TornOffMenu,
    Utility,
    ModalPanel,
    Dock,
    MainMenu,
    Status,
    PopUpMenu,
    Overlay,
    Help,
    Dragging,
    ScreenSaver,
    AssistiveTech,
    Cursor,
    Maximum,
}

impl WindowLevelKey {
    /// Every key, lowest level first.
    pub const ALL: [WindowLevelKey; 20] = [
        WindowLevelKey::Base,
        WindowLevelKey::Minimum,
        WindowLevelKey::Desktop,
        WindowLevelKey::Backstop,
        WindowLevelKey::Normal,
        WindowLevelKey::Floating,
        WindowLevelKey::TornOffMenu,
        WindowLevelKey::Utility,
        WindowLevelKey::ModalPanel,
        WindowLevelKey::Dock,
        WindowLevelKey::MainMenu,
        WindowLevelKey::Status,
        WindowLevelKey::PopUpMenu,
        WindowLevelKey::Overlay,
        WindowLevelKey::Help,
        WindowLevelKey::Dragging,
        WindowLevelKey::ScreenSaver,
        WindowLevelKey::AssistiveTech,
        WindowLevelKey::Cursor,
        WindowLevelKey::Maximum,
    ];

    /// The layer number of the level. Normal windows are on [`NORMAL_LAYER`]; the desktop and its backstop are below.
    pub fn level(self) -> i32 {
        match self {
            WindowLevelKey::Base => -1000,
            WindowLevelKey::Minimum => -999,
            WindowLevelKey::Desktop => -20,
            WindowLevelKey::Backstop => -10,
            WindowLevelKey::Normal => NORMAL_LAYER,
            WindowLevelKey::Floating => 3,
            WindowLevelKey::TornOffMenu => 3,
            WindowLevelKey::Utility => 5,
            WindowLevelKey::ModalPanel => 8,
            WindowLevelKey::Dock => 20,
            WindowLevelKey::MainMenu => 24,
            WindowLevelKey::Status => 25,
            WindowLevelKey::PopUpMenu => 101,
            WindowLevelKey::Overlay => 102,
            WindowLevelKey::Help => 200,
            WindowLevelKey::Dragging => 500,
            WindowLevelKey::ScreenSaver => 1000,
            WindowLevelKey::AssistiveTech => 1500,
            WindowLevelKey::Cursor => 2000,
            WindowLevelKey::Maximum => 3000,
        }
    }

    /// The name used on the wire and in settings files.
    pub fn name(self) -> &'static str {
        match self {
            WindowLevelKey::Base => "base",
            WindowLevelKey::Minimum => "minimum",
            WindowLevelKey::Desktop => "desktop",
            WindowLevelKey::Backstop => "backstop",
            WindowLevelKey::Normal => "normal",
            WindowLevelKey::Floating => "floating",
            WindowLevelKey::TornOffMenu => "torn_off_menu",
            WindowLevelKey::Utility => "utility",
            WindowLevelKey::ModalPanel => "modal_panel",
            WindowLevelKey::Dock => "dock",
            WindowLevelKey::MainMenu => "main_menu",
            WindowLevelKey::Status => "status",
            WindowLevelKey::PopUpMenu => "pop_up_menu",
            WindowLevelKey::Overlay => "overlay",
            WindowLevelKey::Help => "help",
            WindowLevelKey::Dragging => "dragging",
            WindowLevelKey::ScreenSaver => "screen_saver",
            WindowLevelKey::AssistiveTech => "assistive_tech",
            WindowLevelKey::Cursor => "cursor",
            WindowLevelKey::Maximum => "maximum",
        }
    }

    pub fn from_name(name: &str) -> Option<WindowLevelKey> {
        WindowLevelKey::ALL.iter().copied().find(|k| k.name() == name)
    }
}

/// The layer number of a level key.
pub fn window_level(key: WindowLevelKey) -> i32 {
    key.level()
}

/// The layer a window of the shell sits on, from what it is: a normal app window, or one of the shell's own panels.
pub fn level_of_shell_window(kind: ShellWindowKind) -> i32 {
    match kind {
        ShellWindowKind::Desktop => WindowLevelKey::Desktop.level(),
        ShellWindowKind::App => WindowLevelKey::Normal.level(),
        ShellWindowKind::Dock => WindowLevelKey::Dock.level(),
        ShellWindowKind::MenuBar => WindowLevelKey::MainMenu.level(),
    }
}

/// The kinds of window the shell itself puts on the desktop.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShellWindowKind {
    Desktop,
    App,
    Dock,
    MenuBar,
}

// ---- capture options ----

/// How a window image is cut from the window.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ImageOptions {
    /// Leave out the title bar and the frame around the content.
    pub ignore_framing: bool,
    /// Fill transparent pixels with opaque black.
    pub opaque: bool,
}

impl ImageOptions {
    pub fn to_json(&self) -> Json {
        obj(vec![("ignore_framing", Json::Bool(self.ignore_framing)), ("opaque", Json::Bool(self.opaque))])
    }

    pub fn from_json(j: &Json) -> ImageOptions {
        ImageOptions { ignore_framing: bool_of(j, "ignore_framing").unwrap_or(false), opaque: bool_of(j, "opaque").unwrap_or(false) }
    }
}

/// Sets the alpha byte of every pixel to opaque (0xAARRGGBB pixels).
pub fn make_opaque(px: &mut [u32]) {
    for p in px.iter_mut() {
        *p |= 0xff00_0000;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn win(number: u32, layer: i32, on_screen: bool) -> WindowInfo {
        WindowInfo {
            number,
            owner_pid: 100 + number,
            owner_name: format!("app{}", number),
            name: Some(format!("title{}", number)),
            layer,
            bounds: Rect { x: 0.0, y: 0.0, w: 100.0, h: 80.0 },
            alpha: 1.0,
            on_screen,
            sharing: SharingState::None,
        }
    }

    fn ids(list: &[WindowInfo]) -> Vec<u32> {
        list.iter().map(|w| w.number).collect()
    }

    /// Front to back: 1 (on screen), 2 (off screen), 3 (desktop element, on screen), 4 (on screen).
    fn stack() -> Vec<WindowInfo> {
        vec![win(1, 0, true), win(2, 0, false), win(3, -20, true), win(4, 0, true)]
    }

    #[test]
    fn all_scope_keeps_every_window_in_stacking_order() {
        assert_eq!(ids(&window_list(&stack(), WindowListOptions::all())), vec![1, 2, 3, 4]);
    }

    #[test]
    fn on_screen_only_drops_hidden_windows() {
        let o = WindowListOptions { scope: WindowScope::OnScreenOnly, exclude_desktop_elements: false };
        assert_eq!(ids(&window_list(&stack(), o)), vec![1, 3, 4]);
    }

    #[test]
    fn excluding_desktop_elements_drops_windows_below_the_normal_layer() {
        let o = WindowListOptions { scope: WindowScope::All, exclude_desktop_elements: true };
        assert_eq!(ids(&window_list(&stack(), o)), vec![1, 2, 4]);
        assert!(is_desktop_element(&stack()[2]));
        assert!(!is_desktop_element(&stack()[0]));
    }

    #[test]
    fn above_and_below_scopes_use_the_stacking_position_of_the_reference() {
        let above = WindowListOptions { scope: WindowScope::OnScreenAbove(4), exclude_desktop_elements: false };
        assert_eq!(ids(&window_list(&stack(), above)), vec![1, 3]);
        let below = WindowListOptions { scope: WindowScope::OnScreenBelow(1), exclude_desktop_elements: false };
        assert_eq!(ids(&window_list(&stack(), below)), vec![3, 4]);
    }

    #[test]
    fn unknown_reference_gives_an_empty_list() {
        let o = WindowListOptions { scope: WindowScope::OnScreenAbove(99), exclude_desktop_elements: false };
        assert!(window_list(&stack(), o).is_empty());
    }

    #[test]
    fn stacking_index_and_is_above_follow_the_reported_order() {
        let s = stack();
        assert_eq!(stacking_index(&s, 3), Some(2));
        assert_eq!(stacking_index(&s, 9), None);
        assert_eq!(is_above(&s, 1, 4), Some(true));
        assert_eq!(is_above(&s, 4, 1), Some(false));
        assert_eq!(is_above(&s, 4, 9), None);
    }

    #[test]
    fn a_record_and_the_options_survive_the_wire() {
        let mut w = win(7, 3, true);
        w.sharing = SharingState::ReadOnly;
        w.alpha = 0.5;
        assert_eq!(WindowInfo::from_json(&w.to_json()), Some(w.clone()));
        let mut unnamed = w.clone();
        unnamed.name = None;
        assert_eq!(WindowInfo::from_json(&unnamed.to_json()), Some(unnamed));
        assert_eq!(list_from_json(&list_to_json(&stack())), stack());
        for scope in [WindowScope::All, WindowScope::OnScreenOnly, WindowScope::OnScreenAbove(5), WindowScope::OnScreenBelow(6)] {
            let o = WindowListOptions { scope, exclude_desktop_elements: true };
            assert_eq!(WindowListOptions::from_json(&o.to_json()), Some(o));
        }
        assert_eq!(WindowListOptions::from_json(&Json::Null), Some(WindowListOptions::all()));
        assert_eq!(WindowListOptions::from_json(&obj(vec![("scope", text("sideways"))])), None);
    }

    #[test]
    fn redaction_keeps_only_the_callers_own_titles() {
        let mut s = stack();
        redact_names(&mut s, 101);
        assert_eq!(s[0].name.as_deref(), Some("title1"));
        assert!(s[1..].iter().all(|w| w.name.is_none()));
    }

    #[test]
    fn levels_are_ordered_and_named() {
        let levels: Vec<i32> = WindowLevelKey::ALL.iter().map(|k| k.level()).collect();
        assert!(levels.windows(2).all(|p| p[0] <= p[1]));
        assert_eq!(window_level(WindowLevelKey::Normal), NORMAL_LAYER);
        assert!(window_level(WindowLevelKey::Desktop) < NORMAL_LAYER);
        for k in WindowLevelKey::ALL {
            assert_eq!(WindowLevelKey::from_name(k.name()), Some(k));
        }
        assert_eq!(WindowLevelKey::from_name("nope"), None);
        assert_eq!(level_of_shell_window(ShellWindowKind::Dock), window_level(WindowLevelKey::Dock));
        assert_eq!(level_of_shell_window(ShellWindowKind::App), NORMAL_LAYER);
    }

    #[test]
    fn capture_options_and_opaque_fill() {
        let o = ImageOptions { ignore_framing: true, opaque: false };
        assert_eq!(ImageOptions::from_json(&o.to_json()), o);
        let mut px = [0x0011_2233u32, 0x8000_0000];
        make_opaque(&mut px);
        assert_eq!(px, [0xff11_2233, 0xff00_0000]);
    }
}
