//! The chrome model: what the shell tells the menu bar, Dock and panels to show, and what they tell the shell back.
//! Plain data with a compact binary codec (`encode`/`decode`, owner `chrome-core`, hostile-input safe like
//! `bifrost_layers::codec`). Add fields and variants only at the end (the codec carries a version byte).

/// One menu item. A separator has an empty `label`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct MenuItem {
    pub id: u32,
    pub label: String,
    /// Shortcut as the menu shows it ("cmd+s"), empty for none.
    pub shortcut: String,
    pub enabled: bool,
    pub checked: bool,
    pub submenu: Vec<MenuItem>,
}

/// A top-level menu of the menu bar.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Menu {
    pub id: u32,
    pub title: String,
    pub items: Vec<MenuItem>,
}

/// A status item on the right of the menu bar (clock, Wi-Fi, Now Playing, input source ...).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct StatusItem {
    pub id: u32,
    /// A symbol name from the shell's symbol set, or empty.
    pub symbol: String,
    pub text: String,
    pub menu: Vec<MenuItem>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct MenuBarModel {
    /// The front app's name (bold, after the system menu).
    pub app_name: String,
    pub menus: Vec<Menu>,
    pub status: Vec<StatusItem>,
}

/// One Dock item.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct DockItem {
    pub app_id: String,
    pub title: String,
    /// Path of the icon image the shell resolved (PNG or the shell's raw icon), read by the Dock client.
    pub icon_path: String,
    pub running: bool,
    pub active: bool,
    pub badge: String,
    /// Launch or download progress 0..1, negative for none.
    pub progress: f32,
    /// Kept in the Dock (the context menu offers Remove from Dock). Added after v1: the codec carries it at the end.
    pub pinned: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DockPosition {
    #[default]
    Bottom,
    Left,
    Right,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct DockModel {
    pub items: Vec<DockItem>,
    /// Icon size in points and the magnified size (0 disables magnification).
    pub size: f32,
    pub magnified: f32,
    pub position: DockPosition,
    pub autohide: bool,
}

/// A notification as Notification Center shows it.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Notification {
    pub id: u32,
    pub app_id: String,
    pub title: String,
    pub body: String,
    pub time: String,
    pub actions: Vec<(u32, String)>,
}

/// A Control Center module.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ControlModule {
    pub id: u32,
    pub title: String,
    pub symbol: String,
    pub on: bool,
    /// A slider value 0..1, negative for a toggle.
    pub value: f32,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct PanelsModel {
    pub notifications: Vec<Notification>,
    pub controls: Vec<ControlModule>,
}

/// Shell to chrome.
#[derive(Debug, Clone, PartialEq)]
pub enum ToChrome {
    MenuBar(MenuBarModel),
    Dock(DockModel),
    Panels(PanelsModel),
    Appearance { dark: bool, accent: u32, reduce_motion: bool, reduce_transparency: bool },
    /// Open a panel ("notifications", "control", "search") or close it ("").
    ShowPanel(String),
}

/// Chrome to shell.
#[derive(Debug, Clone, PartialEq)]
pub enum FromChrome {
    /// A menu bar item was chosen: the menu id and the item id (status items use their own id as the menu).
    MenuAction { menu: u32, item: u32 },
    DockActivate { app_id: String },
    /// The Dock's context menu asked for an action ("quit", "keep", "remove", "show").
    DockCommand { app_id: String, command: String },
    /// Where an app's Dock icon is, in output logical pixels, for the window server's minimize animation.
    MinimizeTarget { app_id: String, rect: [f32; 4] },
    ControlChanged { id: u32, on: bool, value: f32 },
    NotificationAction { id: u32, action: u32 },
    /// The client is up and wants the current models.
    Hello { client: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecodeError {
    Truncated,
    BadVersion,
    UnknownTag(u8),
    BadValue,
    TooLong,
}

/// The largest accepted message.
pub const MAX_LEN: usize = 4 << 20;

pub fn encode_to_chrome(m: &ToChrome) -> Vec<u8> {
    let _ = m;
    todo!("chrome-core")
}

pub fn decode_to_chrome(b: &[u8]) -> Result<ToChrome, DecodeError> {
    let _ = b;
    todo!("chrome-core")
}

pub fn encode_from_chrome(m: &FromChrome) -> Vec<u8> {
    let _ = m;
    todo!("chrome-core")
}

pub fn decode_from_chrome(b: &[u8]) -> Result<FromChrome, DecodeError> {
    let _ = b;
    todo!("chrome-core")
}
