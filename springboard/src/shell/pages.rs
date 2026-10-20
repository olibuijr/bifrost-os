//! What every Settings page lists, as data: `spec` builds a page's groups
//! from the device info and the user's options, and `Action` says what a
//! row does. Rendering and hit testing live in `list`.

use super::accessibility::shortcut::Feature;
use super::accessibility::{AccessibilitySettings, HOLD_MS, SPEECH_PITCHES, SPEECH_PUNCTUATION, SPEECH_RATES, SPEECH_VOICES, ZOOMS};
use super::accessibility::dwell::DELAYS_MS as DWELL_MS;
use super::slow_keys::DELAYS_MS as SLOW_MS;
use super::battery::{self, Graph};
use super::desktop_dock::Position;
use super::icons::Icon;
use super::row_glyphs::Glyph;
use super::low_battery::last_charged;
use super::list::{Accessory, Ends, Group, Item, Spec, Tint};
use super::options::{speed_slider, Options, AUTO_LOCK, GAME_MODES, REQUIRE_AFTER, SYSTEM_SLEEP, ICON_SIZES, POWER_MODES, TEXT_SIZES, TRACKPAD_SECONDARY};
use super::{Info, WifiAction};
use bifrost_render::fontprefs::{FontRole, Installed};
use bifrost_ui::capture_access::Media;
use super::services::privacy_permissions::{self, Row};
use super::services::display_services::{rate_label, resolution_label, RefreshChoice};
use crate::bare::bluetooth::{Cmd, State as BtState};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PageId {
    Root,
    Network,
    Ethernet,
    Wifi,
    WifiDetail,
    KnownNetworks,
    Bluetooth,
    BluetoothDetail,
    Wallpaper,
    /// Settings > Screen Saver: the saver, Preview and Start After (see `screensaver`)
    ScreenSaver,
    Sounds,
    Focus,
    Notifications,
    NotificationApp,
    General,
    About,
    /// About › Legal & Regulatory
    Legal,
    Nearby,
    AirPlayHandoff,
    Pip,
    BackgroundRefresh,
    Gestures,
    Fonts,
    /// General > Fonts > one role: its default or a font from the library
    FontPick(FontRole),
    Dictionary,
    Vpn,
    /// Network > Firewall (desktop): the incoming connection choice for each app (see `firewall_sheet`)
    Firewall,
    SoftwareUpdate,
    Backup,
    DateTime,
    Keyboard,
    /// Keyboard > Keyboards
    Keyboards,
    /// Keyboard > Dictation: the on-device speech model, its consent and its removal
    Dictation,
    /// Keyboard > Text Replacements: shortcuts that expand to a phrase
    TextReplacements,
    Language,
    /// General > Language & Region > Region: the list of regions (see `bifrost_ui::region`)
    Region,
    Reset,
    Display,
    /// Displays > Refresh Rate: Highest Available or one of the panel's rates (see `refresh_rate_page`)
    RefreshRate,
    /// Displays > Resolution: Native or one of the sizes the panel offers (see `resolution_page`)
    Resolution,
    NightShift,
    AutoLock,
    /// Lock Screen > Require Password: the grace after the display goes off (see `options::REQUIRE_AFTER`)
    RequirePassword,
    Energy,
    /// Battery > Schedule: the repeating wake and power-on times (see `wake_page`)
    WakeSchedule,
    HomeScreen,
    Battery,
    ControlCentre,
    MenuBar,
    Accessibility,
    DisplayZoom,
    TouchHold,
    /// Accessibility > Touch > Dwell Control: the delay before a resting pointer clicks (`a11y.dwell`)
    DwellControl,
    /// Accessibility > Keyboard > Slow Keys: how long a key must be held before it counts (`a11y.slow`)
    SlowKeys,
    /// Accessibility › Accessibility Shortcut: the feature Option+Cmd+F5 switches
    AccessibilityShortcut,
    /// Accessibility › Screen Reader › Speech: rate, pitch and voice
    ScreenReaderSpeech,
    /// Display & Brightness › Interface Scale (desktop only)
    InterfaceScale,
    Appearance,
    Privacy,
    Spotlight,
    /// Privacy › Analytics & Diagnostics
    Diagnostics,
    /// Privacy › App Permissions: which installed apps may use the camera, microphone or screen
    PrivacyPermissions,
    /// Privacy › Location Services: the global switch, System location and each app (see `location_page::spec`)
    LocationServices,
    Storage,
    /// Display & Brightness > Graphics (desktop): adapters and displays, read-only
    Graphics,
    Passcode,
    /// Settings > Passwords: saved website logins, revealed after the passcode (see `passwords_page`)
    Passwords,
    /// Settings > Internet Accounts: the mail, calendar and contact accounts (see `internet_accounts_page`)
    InternetAccounts,
    /// Settings > BifrOSt Account: the signed-in account, its devices and Sign Out (see `account_page`)
    Account,
    Developer,
    ScreenTime,
    StDowntime,
    StLimits,
    StAllowed,
    /// General > Default Apps (desktop): which app opens each kind of file and link
    DefaultApps,
    /// General > Login Items (desktop): the apps that start with the session (`SMAppService`)
    LoginItems,
    /// General > Game Controllers (desktop): the connected controllers and their button remapping
    GameControllers,
    /// Game Controllers > one controller: its battery, a live test and the button remap
    GameController(u32),
    /// Desktop & Dock > Hot Corners (desktop)
    HotCorners,
    /// Keyboard Shortcuts (desktop): the shortcuts the shell answers to
    Shortcuts,
    /// Settings > Assistant: the assistant's switch, backend, history and per-app access to intents (`assistant_page`)
    Assistant,
    /// Overview (desktop): how windows are grouped and when desktops switch
    Overview,
    /// Sharing (desktop): the computer name, the sharing services, Nearby Share and AirPlay & Handoff
    Sharing,
    /// General > Startup Disk (desktop): the systems the firmware can start and the one it starts (see `startup_disk_page`)
    StartupDisk,
    /// Print & Scan (desktop): the printers the printing service knows, the default one, and adding one (`print_scan_page`)
    PrintScan,
    /// Lock Screen: the passcode, inactivity lock and screen saver in one place (links to their own pages)
    LockScreen,
    /// Settings > Trackpad (desktop): tracking speed, tap to click, secondary click, natural scrolling and gestures
    Trackpad,
    /// Settings > Mouse (desktop): tracking and scrolling speed, natural scrolling, swapped buttons
    Mouse,
    /// Settings > Users & Groups (desktop): the local account, login options and password
    Users,
    /// Users & Groups > one account (the detail is its account name)
    UserDetail,
    /// Users & Groups > Groups
    Groups,
    /// Groups > one group (the detail is its name)
    GroupDetail,
    /// Keyboard > Modifier Keys: what Caps Lock, Control, Option and Command do (see `kb_input`)
    ModifierKeys,
    /// Settings > one app's panes: the installed app at this slot (see `settings_panes::spec`)
    AppPanes(usize),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Toggle {
    /// Settings › Menu Bar: shows or hides one menu extra
    MenuExtra(super::options::MenuExtra),
    /// Settings › App Permissions › Contacts: the n-th installed app may read contacts
    Contacts(usize),
    /// Settings › App Permissions › Calendars: the n-th installed app may use the calendar
    Calendars(usize),
    /// Settings > Screen Saver: flips the n-th (on or off) option of the chosen saver
    SaverOption(usize),
    /// Settings › App Permissions › USB and HID devices: takes back every device the n-th installed app may use
    Usb(usize),
    AutoUpdate,
    BackupBeforeUpdate,
    Airplane,
    Wifi,
    Bluetooth,
    KeyboardClicks,
    KeyboardHaptics,
    LockSound,
    Clock24,
    /// Date & Time: the menu bar clock shows the date
    ClockDate,
    /// Date & Time: the menu bar clock's date starts with the weekday
    ClockDay,
    AutoCaps,
    /// enables or removes a keyboard layout (index into `kb_layouts::Layout::ALL`)
    Layout(usize),
    /// each app gets back the input source it had when it was last in front (`kb_layouts::set_remember`)
    RememberInputSource,
    EnableCapsLock,
    PeriodShortcut,
    BoldText,
    AutoJoin,
    Dnd,
    AssistiveTouch,
    ReduceMotion,
    IncreaseContrast,
    ReduceTransparency,
    ButtonShapes,
    /// Accessibility > Dim Flashing Lights (`a11y_dim_flashing_lights`)
    DimFlashingLights,
    /// Settings > Accessibility > Guided Access (`a11y_guided_access`)
    GuidedAccess,
    /// Accessibility > Screen Reader (`a11y_screen_reader`)
    ScreenReader,
    /// Accessibility > Switch Scanning (`a11y_switch_scan`)
    SwitchScan,
    /// Accessibility > Keyboard > Sticky Keys (`a11y_sticky_keys`)
    StickyKeys,
    /// Accessibility > Keyboard > Accessibility Keyboard: the on-screen keyboard (`a11y_accessibility_keyboard`)
    AccessibilityKeyboard,
    /// Keyboard > Input Sources > Show Input menu in menu bar (`show_input_menu`)
    InputMenu,
    /// Accessibility > Keyboard > Mouse Keys: the keypad moves and clicks the pointer (`a11y.mouse_keys`)
    MouseKeys,
    /// Accessibility > Touch > Switch Control: the keyboard steps a highlight through the screen and selects, and the switch
    /// keys scan a panel of controls (`a11y_switch_control`)
    SwitchControl,
    /// Accessibility > Voice Control: spoken commands press what they name (`a11y_voice_control`)
    VoiceControl,
    /// Accessibility > Motion > Show Vehicle Motion Cues: dots at the screen edge follow the accelerometer (`a11y.motion_cues`)
    MotionCues,
    /// Accessibility > Hearing > Live Captions: captions for the sound the computer plays (`a11y_live_captions`)
    LiveCaptions,
    AutoTime,
    LowPower,
    BatteryPercent,
    DockRecents,
    /// Desktop & Dock › Automatically Hide and Show
    DockAutohide,
    /// Desktop & Dock > Magnification
    DockMagnify,
    /// Overview > Group Windows by Application
    OverviewGroup,
    /// Overview > Switch to a Desktop with Open Windows
    OverviewSwitch,
    /// General: Close windows when quitting an application
    CloseOnQuit,
    /// Developer: root adbd at boot, adb over Wi-Fi
    RootedDebug,
    WifiAdb,
    /// Developer: the frame time / temperature / memory overlay
    PerfHud,
    /// Developer: GPU frames flip straight to the display (next start)
    GpuScanout,
    Handoff,
    /// Continuity: share copies between the account's devices (sealed; off by default)
    UniversalClipboard,
    PipAuto,
    /// Displays > Automatically Adjust Brightness: the ambient light sensor drives the panel (needs a sensor)
    AutoBrightness,
    BgRefresh,
    SuspendBackground,
    FourFinger,
    CornerSwipe,
    /// Settings > Search: a kind of result Search lists, switched on or off
    SpotlightCategory(super::spotlight_scope::Category),
    /// App Permissions: the app at this index (of `apphost::installed()`) may use this media type
    Permission(Media, usize),
    /// App Permissions > Location: the app at this index may use location while it is in use
    Location(usize),
    /// Mouse and Trackpad: natural scrolling (the wheel and fingers move the content the way a touch screen does)
    NaturalScroll,
    /// Trackpad > Tap to Click
    TapToClick,
    /// Mouse > Pointer Acceleration
    MouseAcceleration,
    /// Users & Groups > Login Options: name and password fields at sign-in
    LoginNamePassword,
    /// Users & Groups > Login Options: password hints at sign-in
    LoginHints,
    /// Keyboard > Function Keys: F1 to F12 type their standard keys
    FunctionStandard,
    /// Keyboard > Keyboard Navigation: Tab moves focus between controls
    TabNavigation,
    /// Keyboard Shortcuts: switches one shell shortcut on or off (index into `kb_input::BINDINGS`)
    Shortcut(usize),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pick {
    /// Privacy > Allow Apps From: the index into `launch_gate::SETTINGS`
    AllowAppsFrom(usize),
    /// Mouse > Secondary Click: index into `mouse_page::SECONDARY_BUTTON`
    MouseSecondary(usize),
    /// Settings > Search > Privacy: stop excluding the folder at this index
    SpotlightExclude(usize),
    /// Settings > Backup > Backup Destination: the index into `backup_page::destination_choices`
    BackupDestination(usize),
    /// Keyboard > Modifier Keys: the modifier key (index into `kb_input::REMAPPABLE`) sends the choice (index into `kb_input::REMAP_TARGETS`)
    Remap(usize, usize),
    /// Settings > Sound: the n-th listed output / input becomes the default
    SoundOutput(usize),
    SoundInput(usize),
    AutoLock(usize),
    RequirePassword(usize),
    SystemSleep(usize),
    IconSize(usize),
    PowerMode(usize),
    GameMode(usize),
    DisplayZoom(usize),
    TouchHold(usize),
    /// Accessibility > Touch > Dwell Control: the n-th of `dwell::DELAYS_MS`
    Dwell(usize),
    /// Accessibility > Keyboard > Slow Keys: the n-th of `slow_keys::DELAYS_MS`
    Slow(usize),
    /// Accessibility > Accessibility Shortcut: the n-th of `accessibility::SHORTCUT_CHOICES` (0 is None)
    AccessibilityShortcut(usize),
    /// Screen Reader > Speech: the n-th of `accessibility::SPEECH_RATES`, `SPEECH_PITCHES`, `SPEECH_VOICES`
    SpeechRate(usize),
    SpeechPitch(usize),
    SpeechVoice(usize),
    /// Screen Reader > Speech > Punctuation: an index into `SPEECH_PUNCTUATION`
    SpeechPunctuation(usize),
    /// 0 is Auto, n the n-th of `layout::SCALE_CHOICES`
    InterfaceScale(usize),
    /// Displays > Appearance: 0 Light, 1 Dark, 2 Auto
    Appearance(usize),
    /// Appearance > Accent colour: the index into `Accent::ALL`
    Accent(usize),
    /// Appearance > Liquid Glass style: 0 Clear, 1 Tinted
    GlassStyle(usize),
    /// Appearance > Icon & widget style: the index into `ICON_STYLES` (0 Default, 1 Dark, 2 Tinted)
    IconStyle(usize),
    Nearby(usize),
    /// Language & Region > Preferred Language: the index into `bifrost_ui::region::LANGUAGES`
    Language(usize),
    /// Language & Region > Region: the index into `bifrost_ui::region::REGIONS`
    Region(usize),
    /// Language & Region > First Day of Week: 0 Automatic, 1 Sunday, 2 Monday
    FirstWeekday(usize),
    /// Desktop & Dock › Position: index into Bottom, Left, Right
    DockPosition(usize),
    /// Desktop & Dock > Minimize Windows Using: index into `MINIMIZE_EFFECTS`
    MinimizeEffect(usize),
    /// Hot Corners: (corner 0..4, action index into `CORNER_ACTIONS`)
    HotCorner(usize, usize),
    /// Appearance > Highlight colour: 0 follows the accent, n is `Accent::ALL[n]`
    Highlight(usize),
    /// Appearance > Sidebar icon size: 0 Small, 1 Medium, 2 Large
    SidebarIcons(usize),
    /// Appearance > Show scroll bars: index into `SCROLL_BAR_MODES`
    ScrollBars(usize),
    /// Settings > Screen Saver: the index into `screensaver::SAVERS`
    ScreenSaverModule(usize),
    /// Settings > Sound > Alert Sound: the n-th `options::ALERT_SOUNDS` entry
    AlertSound(usize),
    /// Settings > Screen Saver > Start After: the index into `screensaver::START_AFTER`
    ScreenSaverStart(usize),
    /// Settings > Screen Saver: the n-th option of the chosen saver, set to its c-th choice (see `screensaver::load_sheet`)
    ScreenSaverOption(usize, usize),
    /// Settings > Screen Saver > Preview: the saver starts full screen (the shell does it, see `Settings::take_screensaver_preview`)
    ScreenSaverPreview,
    /// Trackpad > Secondary Click: index into `TRACKPAD_SECONDARY`
    TrackpadSecondary(usize),
    /// Displays > Refresh Rate: 0 is Highest Available, n the (n-1)th rate the panel offers (see `services::display_services::choice_at`)
    RefreshRate(usize),
    /// Displays > Resolution: 0 is Native, n the (n-1)th size the panel offers (see `services::display_services::resolution_at`)
    Resolution(usize),
}

impl Pick {
    /// The same pick with `k` as its choice, for a pop-up row's menu. None for a pick that is not a single choice among
    /// listed values.
    pub fn with_choice(self, k: usize) -> Option<Pick> {
        Some(match self {
            Pick::AllowAppsFrom(_) => Pick::AllowAppsFrom(k),
            Pick::MouseSecondary(_) => Pick::MouseSecondary(k),
            Pick::TrackpadSecondary(_) => Pick::TrackpadSecondary(k),
            Pick::GameMode(_) => Pick::GameMode(k),
            Pick::SystemSleep(_) => Pick::SystemSleep(k),
            Pick::PowerMode(_) => Pick::PowerMode(k),
            Pick::DockPosition(_) => Pick::DockPosition(k),
            Pick::MinimizeEffect(_) => Pick::MinimizeEffect(k),
            Pick::ScreenSaverModule(_) => Pick::ScreenSaverModule(k),
            Pick::ScreenSaverStart(_) => Pick::ScreenSaverStart(k),
            Pick::ScreenSaverOption(n, _) => Pick::ScreenSaverOption(n, k),
            _ => return None,
        })
    }
}

/// The labels of a list of choices as a pop-up row or a segmented row takes them.
pub fn choice_labels<S: AsRef<str>>(labels: &[S]) -> Vec<String> {
    labels.iter().map(|l| l.as_ref().to_string()).collect()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Slider {
    /// Keyboard > Key Repeat > Delay Until Repeat (`options::repeat_delay_from_slider`)
    KeyRepeatDelay,
    /// Keyboard > Key Repeat > Key Repeat (`options::repeat_rate_from_slider`)
    KeyRepeatRate,
    Brightness,
    Volume,
    TextSize,
    Warmth,
    /// Desktop & Dock › Size (the dock height, 32..=80 pt)
    DockSize,
    /// Displays › Liquid Glass: 0 clear .. 1 tinted (Golden Gate Golden Gate)
    LiquidGlass,
    /// Desktop & Dock > Magnification size, 0..1
    DockMagnify,
    /// Mouse > Tracking Speed (see `options::speed_from_slider`)
    MouseSpeed,
    /// Mouse > Scrolling Speed
    MouseScroll,
    /// Mouse > Double-Click Speed (see `mouse_page::double_click_from_slider`)
    MouseDoubleClick,
    /// Trackpad > Tracking Speed
    TrackpadSpeed,
    /// Accessibility > Cursor Size, 0..1 (`accessibility::AccessibilitySettings::cursor_size`)
    CursorSize,
}

/// Something that asks for confirmation first.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    ResetSettings,
    /// Transfer or Reset › Erase All Content and Settings: staged, done at the next start (`bifrost_core::erase_content`)
    EraseContent,
    Restart,
    /// power the computer off (General › Shut Down)
    ShutDown,
    /// reboot the computer (General › Restart)
    Reboot,
    Forget(String),
    /// unpairs a Bluetooth device (by address)
    ForgetBluetooth(String),
    /// Keyboard > Text Replacements: removes the replacement for this shortcut
    RemoveTextReplacement(String),
    /// empties one disposable cache (bifrost Storage)
    ClearCache(super::storage::Clear),
    /// turning Rooted Debugging off, after the warning
    RootedOff,
    BackUpNow,
    RestoreBackup,
    /// delete every backup archive (Backup page); the data itself is kept
    DeleteBackups,
    CheckUpdate,
    DownloadUpdate,
    /// restarts the shell so boot.sh promotes the staged binary
    InstallUpdate,
    /// Software Update: roll back to the previous system (pacman profile, through the guard unit)
    RollbackSystem,
    /// Analytics & Diagnostics: upload the bundle to the update server
    ShareDiagnostics,
    DeleteCrashLogs,
    /// Keyboard > Dictation: the person agrees to the model download (consent is recorded first)
    DownloadSpeechModel,
    /// Keyboard > Dictation: removes the model and withdraws consent
    RemoveSpeechModel,
    /// Storage > Applications: removes one package with pacman, after the confirmation alert
    RemovePackage(String),
    /// Users & Groups: delete an account or a group
    Account(super::users_page::Danger),
    /// a failed change, shown as an alert with OK (its text is the reason)
    Notice(String),
}

/// The Reduce Transparency switch: Increase Contrast turns it on and locks it (the reference desktop's
/// Accessibility > Display does the same), so with contrast on it shows on and does nothing when tapped.
fn reduce_transparency_item(reduce: bool, contrast: bool) -> Item {
    let action = if contrast { Action::None } else { Action::Toggle(Toggle::ReduceTransparency) };
    Item::new("Reduce Transparency").switch(reduce || contrast, action)
}

#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    None,
    Push(PageId),
    Toggle(Toggle),
    /// opens the passcode keypad for this purpose; true picks a custom-length code
    Passcode(super::passcode::Purpose, bool),
    Pick(Pick),
    Slider(Slider),
    Run(Command),
    /// join, select or ask for the password of a scanned network
    Network(String),
    /// open the Time Zone picker
    TimeZone,
    SetTime,
    /// a notification setting: DND schedule rows or a per-app mute
    Notify(super::notify_page::NotifyEdit),
    Night(super::night_shift::NightEdit),
    /// a Battery > Schedule row (see `wake_page::WakeEdit`)
    Wake(super::wake_page::WakeEdit),
    /// Network > Diagnose: run the connection probes on a worker thread (see `net_diagnose_page`)
    NetDiagnose,
    /// Privacy > Security: let the program that was blocked open (see `launch_gate`)
    OpenAnyway,
    /// a Screen Time row: range, limits, Downtime, allowed apps, its passcode
    ScreenTime(super::screentime::page::StEdit),
    /// opens the "Other Network" sheet
    JoinOther,
    /// a Wi-Fi change straight from a row
    Wifi(WifiAction),
    /// a Bluetooth change straight from a row (pair, connect, disconnect)
    Bluetooth(crate::bare::bluetooth::Cmd),
    /// make the connected Bluetooth audio device the sound output (audio-devices `set_default`)
    AudioOutput,
    /// shows a text file in Files' viewer (a crash log)
    OpenFile(std::path::PathBuf),
    /// Default Apps: make this app (key) the default for the category (index)
    DefaultApp(usize, String),
    /// Login Items: stop starting this app (id) at login
    LoginItemRemove(String),
    /// Network > Firewall: allow or block this app's (id) incoming connections, or forget the choice
    Firewall(String, super::firewall_sheet::Edit),
    /// a tap on a Settings > Passwords row (see `passwords_page::Act`)
    Passwords(super::passwords_page::Act),
    /// a tap on a Settings > Internet Accounts row (see `internet_accounts_page::Act`)
    InternetAccounts(super::internet_accounts_page::Act),
    /// a tap on the Settings > BifrOSt Account page (see `account_page::Act`)
    Account(super::account_page::Act),
    /// Bluetooth: type the PIN or passkey the device asked for while pairing (`addr`)
    BluetoothPrompt { addr: String, passkey: bool },
    /// VPN: bring the NetworkManager connection `name` up (`up`) or down
    Vpn { name: String, up: bool },
    /// Login Items: allow this pending app (id) to open at login
    LoginItemApprove(String),
    /// Game Controllers: change one button's output, or restore a controller's defaults
    GameController(super::game_controllers_page::Edit),
    /// Login Items: start this app (id) one place earlier
    LoginItemMove(String),
    /// Login Items & Extensions: turn an app's background agent on or off (the switch in ALLOW IN THE BACKGROUND)
    BackgroundAgentToggle(String),
    /// Login Items > Extensions: turn the extension with this key (`login_items_page::extension_key`) on or off
    ExtensionToggle(String),
    /// Background App Refresh: turn one app's refresh on or off (the APPS switches under General > Background App Refresh)
    RefreshToggle(String),
    /// Fonts: draw `role` with this font file from now on (None is the default)
    Font(FontRole, Option<String>),
    /// Users & Groups: add, promote, set a password, edit a group (see `users_page`)
    Users(super::users_page::Edit),
    /// Sharing: change the computer name or switch a sharing service (see `sharing_page`)
    Sharing(super::sharing_page::Edit),
    /// Startup Disk: choose the system to start from, after an administrator approves (see `startup_disk_page`)
    StartupDisk(super::startup_disk_page::Edit),
    /// Print & Scan: make a printer the default, or add one (see `print_scan_page`)
    PrintScan(super::print_scan_page::Edit),
    /// Location Services: the switches, the place and the apps' access (see `location_page::Edit`)
    Location(super::location_page::Edit),
    /// Settings > Assistant: a switch, the backend, or an app's access to its intents (see `assistant_page::Edit`)
    Assistant(super::assistant_page::Edit),
    /// Settings > an app's pane: ask the app in this slot to open the pane with this id (`settings_panes::open`)
    AppPane(usize, String),
}

/// Compile-time identity of this build (build.rs supplies the git fields).
pub struct Build {
    pub version: &'static str,
    pub git: &'static str,
    pub date: &'static str,
}

pub const BUILD: Build = Build { version: env!("CARGO_PKG_VERSION"), git: match option_env!("SPRINGBOARD_GIT") { Some(g) => g, None => "unknown" }, date: match option_env!("SPRINGBOARD_BUILD_DATE") { Some(d) => d, None => "unknown" } };

/// Live state a page is built from.
pub struct Model<'a> {
    pub info: &'a Info,
    pub opts: &'a Options,
    pub a11y: &'a AccessibilitySettings,
    pub airplane: bool,
    /// the device has a vibrator (Keyboard Haptics is only offered then)
    pub haptics: bool,
    /// the switch as the user left it, before the device confirms
    pub wifi_on: bool,
    pub brightness: f32,
    /// display name of the current wallpaper
    pub wallpaper: String,
    /// the network whose detail page is open
    pub detail: &'a str,
    /// Unix time now, for the battery graph
    pub now: i64,
    /// an OS update is on offer: red badges on General and Software Update
    pub update_badge: bool,
    /// the desktop dock as the shell holds it (Desktop & Dock page)
    pub dock: super::desktop_dock::Preferences,
    /// the value of the BifrOSt Account row at the top of Settings (`account_page::label`)
    pub account: &'a str,
    /// the Default Apps page: categories, candidates and the current default
    pub defaults: &'a [super::defaultapps::CategoryView],
    /// the Login Items page: (app id, name) of each login item
    pub login_items: &'a [(String, String)],
    /// the Login Items page: (app id, name) of each registration waiting for approval
    pub login_pending: &'a [(String, String)],
    /// the Firewall page: each app with a remembered incoming connection choice
    pub firewall: &'a [super::firewall_sheet::FirewallView],
    /// the background agents and app extensions of the Login Items & Extensions page
    pub ext: &'a super::login_items_page::LoginExtensions,
    /// the Fonts pages: each role's default and override, and the font library
    pub fonts: &'a FontsView,
    /// Displays > Refresh Rate: the choice kept in the display-refresh file (Highest Available when missing)
    pub refresh: RefreshChoice,
    /// Displays > Resolution: the size chosen (None is Native), kept in the display-resolution file
    pub resolution: Option<(u32, u32)>,
    /// Settings > Sound: the device lists from the audio object model
    pub sound: &'a super::sound_page::SoundView,
    /// the Sharing page: the computer name and the service states (read off the UI thread, see `settings_load`)
    /// Sharing: None until the worker's read lands (the page shows a placeholder)
    pub sharing: Option<&'a super::sharing_page::View>,
    /// the Print & Scan page: the printers and scanners
    /// Print & Scan: None until the worker's read lands
    pub printers: Option<&'a super::print_scan_page::View>,
    /// the Users & Groups pages: the accounts and groups
    pub users: &'a super::users_page::View,
}

/// The Sharing page before its first read lands.
/// The Print & Scan page before its first read lands.
/// The Users & Groups pages before their first read lands.
pub static NO_USERS: super::users_page::View = super::users_page::View { users: Vec::new(), groups: Vec::new(), me: String::new(), limits: bifrost_ui::account_policy::PolicyTable::empty(), limits_damaged: false };

/// Settings > Fonts as the page shows it, indexed by `FontRole::index`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FontsView {
    /// the family of each role's packed default
    pub defaults: [String; 2],
    /// the font file saved for each role, when it is not the default
    pub overrides: [Option<String>; 2],
    /// the family name of each saved override, read with it (None when the file has no name)
    pub names: [Option<String>; 2],
    /// BifrOSt's font library (`fontprefs::library`)
    pub library: Vec<Installed>,
}

/// A `FontsView` with nothing in it, for models that never show the Fonts pages.
pub static NO_FONTS: FontsView = FontsView { defaults: [String::new(), String::new()], overrides: [None, None], names: [None, None], library: Vec::new() };

/// "117.9 GB" style size.
pub fn gb(bytes: u64) -> String {
    let gb = bytes as f64 / 1e9;
    if gb >= 100.0 {
        format!("{gb:.0} GB")
    } else {
        format!("{gb:.1} GB")
    }
}

/// "3d 2h 7m" / "2h 7m" / "7m".
pub fn uptime(minutes: i64) -> String {
    let (d, h, m) = (minutes / 1440, minutes % 1440 / 60, minutes % 60);
    match (d, h) {
        (0, 0) => format!("{m}m"),
        (0, _) => format!("{h}h {m}m"),
        _ => format!("{d}d {h}h {m}m"),
    }
}

/// Whether this computer has a battery to show. A desktop (it has the network snapshot) without one has no Battery
/// pane, and its menu bar shows no battery or plug.
pub fn has_battery(info: &Info) -> bool {
    info.desktop_network.is_none() || info.battery_present
}

/// The form factor Settings words itself for. Tests start as a tablet (what the pages assumed before the wording
/// followed the form) and switch with `set_test_form`.
fn form() -> bifrost_ui::form_factor::FormFactor {
    #[cfg(test)]
    {
        TEST_FORM.with(|f| f.get())
    }
    #[cfg(not(test))]
    {
        bifrost_ui::form_factor::FormFactor::current()
    }
}

#[cfg(test)]
thread_local! {
    static TEST_FORM: std::cell::Cell<bifrost_ui::form_factor::FormFactor> = const { std::cell::Cell::new(bifrost_ui::form_factor::FormFactor::Tablet) };
}

#[cfg(test)]
pub(super) fn set_test_form(f: bifrost_ui::form_factor::FormFactor) {
    TEST_FORM.with(|c| c.set(f));
}

/// `touch` on a tablet and `click` on a desktop: a pane's instructions name the input the person has.
pub fn by_form(touch: &'static str, click: &'static str) -> &'static str {
    if form().is_tablet() { touch } else { click }
}

/// Whether the lock is a numeric passcode (a tablet's) rather than the account password (a desktop's).
fn numeric_passcode() -> bool {
    form().is_tablet()
}

/// The battery level as text. A desktop without a battery has no level to show.
fn battery_level(info: &Info) -> String {
    if info.desktop_network.is_some() && !info.battery_present {
        return "None".into();
    }
    format!("{}%", info.battery)
}

/// The About page's rows: (label, value) in page order, grouped.
pub fn about(info: &Info) -> Vec<Vec<(&'static str, String)>> {
    let dash = |s: &str| if s.is_empty() { "—".to_string() } else { s.to_string() };
    vec![
        vec![
            ("Name", dash(&info.name)),
            ("BifrOSt Version", BUILD.version.to_string()),
            ("Model", dash(&info.model)),
            ("Processor", dash(&info.cpu)),
            ("Memory", dash(&info.memory)),
            ("Serial Number", dash(&info.serial)),
        ],
        vec![("Capacity", gb(info.storage_total)), ("Available", gb(info.storage_free))],
        vec![("Wi-Fi Address", dash(&info.mac)), ("IP Address", dash(&info.ip)), ("Kernel", dash(&info.kernel)), ("Uptime", uptime(info.uptime_min))],
        // this machine's own facts (read once from /proc and /sys, see `bifrost_core::sysinfo`)
        {
            let this = bifrost_core::sysinfo::Summary::this_machine();
            vec![
                ("Graphics", if this.graphics.is_empty() { dash("") } else { this.graphics.join("; ") }),
                ("Operating System", dash(&format!("{} {}", this.os_name, this.os_version))),
            ]
        },
    ]
}

fn rows(pairs: Vec<(&'static str, String)>) -> Group {
    Group::new(pairs.into_iter().map(|(l, v)| Item::new(l).value(v)).collect())
}

/// Why `ssid` is not connected ("Incorrect Password"), or "".
fn note_of<'a>(info: &'a Info, ssid: &str) -> &'a str {
    info.saved.iter().find(|n| n.ssid == ssid).map_or("", |n| n.note.as_str())
}

fn on_off(b: bool) -> &'static str {
    if b {
        "On"
    } else {
        "Off"
    }
}

/// Settings › Bluetooth: the switch, my devices, the devices found nearby.
fn bluetooth_spec(m: &Model, page: impl Fn(&str, bool, Vec<Group>) -> Spec) -> Spec {
    let bt = &m.info.bt;
    let mut groups = vec![Group::new(vec![Item::new("Bluetooth").switch(bt.on(), Action::Toggle(Toggle::Bluetooth))])];
    if bt.state == BtState::Unavailable {
        groups[0] = groups[0].clone().footer("Bluetooth is not available on this computer right now.");
        return page("Bluetooth", false, groups);
    }
    if bt.on() {
        if let Some(p) = &bt.prompt {
            let label = bt.device(&p.addr).map_or(p.addr.as_str(), |d| d.label());
            let passkey = p.method == bifrost_core::bt_pairing::PairingMethod::Passkey;
            let ask = Item::new(format!("Enter the {} for {label}", if passkey { "passkey" } else { "PIN" }))
                .tint(Tint::Blue)
                .action(Action::BluetoothPrompt { addr: p.addr.clone(), passkey });
            groups.push(Group::new(vec![ask]).footer("The device is waiting for this code to finish pairing."));
        }
        let note = |addr: &str, connected: bool| match (bt.busy.as_deref() == Some(addr), connected) {
            (true, _) => "Connecting…",
            (_, true) => "Connected",
            _ => "Not Connected",
        };
        let mine: Vec<Item> = bt
            .devices
            .iter()
            .filter(|d| d.paired)
            .map(|d| Item { accessory: Accessory::Info, action: Action::Push(PageId::BluetoothDetail), ..Item::new(d.label()).value(note(&d.addr, d.connected)) })
            .collect();
        if !mine.is_empty() {
            groups.push(Group::new(mine).header("MY DEVICES"));
        }
        let nearby: Vec<Item> = bt
            .devices
            .iter()
            .filter(|d| !d.paired)
            .map(|d| {
                let value = if bt.busy.as_deref() == Some(d.addr.as_str()) { "Connecting…" } else { "" };
                Item::new(d.label()).value(value).action(Action::Bluetooth(Cmd::Pair(d.addr.clone())))
            })
            .collect();
        let mut nearby = if nearby.is_empty() { vec![Item::new(if bt.scanning { "Searching…" } else { "No Devices Found" }).tint(Tint::Dim)] } else { nearby };
        // Search again: the page scans when it opens, this lets the user repeat the search without leaving it.
        if !bt.scanning {
            nearby.push(Item::new("Scan for Devices").tint(Tint::Blue).action(Action::Bluetooth(Cmd::Scan)));
        }
        let mut other = Group::new(nearby).header("OTHER DEVICES");
        if let Some(e) = &bt.error {
            other = other.footer(e);
        }
        groups.push(other);
    } else if let Some(e) = &bt.error {
        groups[0] = groups[0].clone().footer(e);
    } else {
        groups[0] = groups[0].clone().footer("Turn on Bluetooth to pair headphones, speakers and keyboards.");
    }
    page("Bluetooth", false, groups)
}

/// Words a pane or a setting row is also found by, besides its label and the page it sits on (Settings search).
/// Keys are the labels lowercased.
const KEYWORDS: &[(&str, &str)] = &[
    ("network", "internet connection ethernet wifi lan"),
    ("wi-fi", "wireless wlan wifi internet ssid join network"),
    ("ethernet", "wired cable lan network"),
    ("bluetooth", "wireless devices pair headphones speaker mouse keyboard"),
    ("vpn", "tunnel private network"),
    ("firewall", "incoming connections block allow ports listening network security"),
    ("airplane mode", "flight offline wireless radio"),
    ("display & brightness", "screen monitor brightness resolution scale night shift text size"),
    ("displays", "screen monitor brightness resolution scale appearance"),
    ("appearance", "dark light mode theme accent color"),
    ("wallpaper", "background desktop picture image animated live dynamic time of day drift waves dither"),
    ("screen saver", "idle screen saver slideshow"),
    ("sounds", "sound volume alert audio output input speaker microphone"),
    ("sound", "volume alert audio output input speaker microphone"),
    ("focus", "do not disturb schedule notifications silence"),
    ("do not disturb", "focus silence notifications schedule"),
    ("notifications", "alerts banners badges sounds do not disturb"),
    ("accessibility", "zoom magnifier voiceover text size contrast reduce motion hearing screen reader speech spoken transparency animation shortcut pointer cursor switch scanning flashing lights vision touch"),
    ("assistivetouch", "accessibility touch pointer hold"),
    ("dwellcontrol", "accessibility dwell click pointer rest hover"),
    ("slowkeys", "accessibility keyboard acceptance hold delay"),
    ("mousekeys", "accessibility keypad numbers pointer move click"),
    ("battery", "power charge energy low power"),
    ("energy", "power battery sleep display"),
    ("about", "name version serial model build computer"),
    ("software update", "update version upgrade install backup rollback"),
    ("general", "about software update storage date time language fonts"),
    ("date & time", "clock timezone time zone ntp 24 hour"),
    ("keyboard", "typing layout input source shortcuts text replacements"),
    ("language & region", "language region locale format"),
    ("fonts", "typeface text font family"),
    ("privacy", "permissions camera microphone location tracking analytics"),
    ("passcode", "pin password lock unlock"),
    ("passwords", "logins saved website accounts"),
    ("users & groups", "accounts user admin password login"),
    ("trackpad", "touchpad pointer click scroll gestures tap"),
    ("mouse", "pointer wheel scroll speed buttons double-click secondary click acceleration tracking"),
    ("night shift", "warm color temperature schedule evening"),
    ("print & scan", "printer scanner printing scanning paper"),
    ("sharing", "computer name hostname file sharing remote"),
    ("storage", "disk space free used cache"),
    ("backup", "restore copy save files"),
    ("screen time", "limits downtime usage apps children"),
    ("login items", "startup start at login launch background"),
    ("default apps", "browser mail open files default"),
    ("desktop & dock", "dock icons hot corners home screen"),
    ("home screen & dock", "dock icons home screen"),
    ("hot corners", "screen corners actions"),
    ("lock screen", "password passcode lock idle"),
    ("developer", "debug adb root performance hud"),
    ("graphics", "gpu graphics driver intel nvidia amd"),
    ("game controllers", "gamepad controller joystick buttons remap"),
    ("keyboard shortcuts", "keys hotkeys commands"),
    ("overview", "windows desktops group switch"),
    ("menu bar", "top bar status items clock"),
    ("control center", "control centre quick settings"),
    ("legal", "license regulatory terms"),
    ("dictionary", "spelling words"),
    ("text replacements", "shortcut expand autocorrect"),
    ("modifier keys", "caps lock control option command"),
    ("interface scale", "size scale zoom display"),
    ("liquid glass", "transparency clarity glass tint"),
];

/// The words a row or pane is found by besides its label (`KEYWORDS`); empty when it has none.
pub fn keywords(label: &str) -> &'static str {
    let key = label.trim().to_lowercase();
    KEYWORDS.iter().find(|(k, _)| *k == key).map_or("", |(_, words)| words)
}

/// Settings > Sharing: the computer name and services (`None` while they are read), then Nearby Share and AirPlay & Handoff.
pub fn sharing_spec(o: &Options, view: Option<&super::sharing_page::View>) -> Spec {
    let mut groups = match view {
        Some(v) => super::sharing_page::groups(v),
        None => vec![Group::new(vec![Item::new("Computer Name").value("Loading…").tint(Tint::Dim)])],
    };
    groups.push(Group::new(vec![
        Item::new("Nearby Share").glyph(Glyph::Nearby).value(NEARBY_CHOICES.get(o.nearby).copied().unwrap_or(NEARBY_CHOICES[0])).link(PageId::Nearby),
        Item::new("AirPlay & Handoff").value(if o.handoff { "Handoff On" } else { "Handoff Off" }).link(PageId::AirPlayHandoff),
    ])
    .footer("Choose who can share files with this computer nearby, and whether your other devices can pick up work from it."));
    Spec { id: PageId::Sharing, title: "Sharing".into(), large: false, groups }
}

/// The page `id` as the model shows it now.
pub fn spec(id: PageId, m: &Model) -> Spec {
    let (info, o) = (m.info, m.opts);
    let page = |title: &str, large: bool, groups: Vec<Group>| Spec { id, title: title.into(), large, groups };
    match id {
        PageId::Root => {
            let wifi_row = Item::new("Wi-Fi").icon(Icon::Wifi).value(if m.wifi_on { info.wifi.as_str() } else { "Off" }).link(PageId::Wifi);
            // a desktop lists Wi-Fi above Network when it has a wireless adapter; Network keeps the wired rows
            let mut connections = vec![Item::new("Airplane Mode").icon(Icon::Airplane).switch(m.airplane, Action::Toggle(Toggle::Airplane))];
            match info.desktop_network.as_ref() {
                Some(n) => {
                    if n.wifi_available {
                        connections.push(wifi_row);
                    }
                    connections.push(Item::new("Network").icon(Icon::Network).value(if info.online { "Connected" } else { "Not Connected" }).link(PageId::Network));
                }
                None => connections.push(wifi_row),
            }
            connections.push(Item::new("Bluetooth").icon(Icon::Bluetooth).value(info.bt.summary()).link(PageId::Bluetooth));
            // no battery, no Battery pane: a desktop offers its sleep and wake settings as Energy instead
            let power = if has_battery(info) {
                Item::new("Battery").icon(Icon::Battery).value(battery_level(info)).link(PageId::Battery)
            } else {
                Item::new("Energy").glyph(Glyph::Energy).value(o.system_sleep_label()).link(PageId::Energy)
            };
            let mut general = vec![
                Item::new("General").icon(Icon::General).badge(usize::from(m.update_badge)).link(PageId::General),
                Item::new("Control Center").icon(Icon::ControlCentre).link(PageId::ControlCentre),
                Item::new("Menu Bar").icon(Icon::ControlCentre).link(PageId::MenuBar),
                Item::new("Display & Brightness").icon(Icon::Display).link(PageId::Display),
                Item::new("Home Screen & Dock").icon(Icon::HomeScreen).link(PageId::HomeScreen),
                Item::new("Overview").icon(Icon::HomeScreen).link(PageId::Overview),
                Item::new("Sharing").icon(Icon::Wifi).link(PageId::Sharing),
                Item::new("Print & Scan").icon(Icon::Printer).link(PageId::PrintScan),
                Item::new("Keyboard Shortcuts").icon(Icon::Keyboard).link(PageId::Shortcuts),
                Item::new("Assistant").icon(Icon::Assistant).link(PageId::Assistant),
                Item::new("Trackpad").icon(Icon::Trackpad).link(PageId::Trackpad),
                Item::new("Mouse").icon(Icon::Mouse).link(PageId::Mouse),
                Item::new("Users & Groups").icon(Icon::Passcode).link(PageId::Users),
                Item::new("Accessibility").icon(Icon::Accessibility).link(PageId::Accessibility),
                Item::new("Wallpaper").icon(Icon::Wallpaper).value(m.wallpaper.as_str()).link(PageId::Wallpaper),
                Item::new("Screen Saver").icon(Icon::Wallpaper).value(if o.screensaver_minutes == 0 { "Never" } else { super::screensaver::saver_name(o.screensaver_module) }).link(PageId::ScreenSaver),
                power,
                Item::new("Privacy").icon(Icon::Privacy).link(PageId::Privacy),
                Item::new("Search").icon(Icon::Search).link(PageId::Spotlight),
                Item::new("Lock Screen").icon(Icon::Passcode).link(PageId::LockScreen),
            ];
            // the numeric passcode is a tablet's lock; a desktop signs in with its account password, which Lock Screen
            // and Users & Groups cover
            if numeric_passcode() {
                general.push(Item::new("Passcode").icon(Icon::Passcode).link(PageId::Passcode));
            }
            general.push(Item::new("Passwords").icon(Icon::Passcode).link(PageId::Passwords));
            general.push(Item::new("Internet Accounts").icon(Icon::General).link(PageId::InternetAccounts));
            page(
                "Settings",
                true,
                vec![
                    Group::new(vec![Item::new("BifrOSt Account").icon(Icon::Account).value(m.account).link(PageId::Account)]),
                    Group::new(connections),
                    Group::new(vec![
                        Item::new("Notifications").icon(Icon::Notifications).link(PageId::Notifications),
                        Item::new("Sounds").icon(Icon::Sounds).link(PageId::Sounds),
                        Item::new("Focus").icon(Icon::Focus).value(on_off(o.dnd)).link(PageId::Focus),
                        Item::new("Screen Time").icon(Icon::ScreenTime).link(PageId::ScreenTime),
                    ]),
                    Group::new(general),
                    Group::new(vec![Item::new("Developer").icon(Icon::General).link(PageId::Developer)]),
                ],
            )
        }
        PageId::Network => desktop_network_page(info),
        PageId::Ethernet => desktop_ethernet_page(info),
        PageId::Vpn => desktop_vpn_page(info),
        PageId::Firewall => super::firewall_sheet::page(m.firewall),
        PageId::Wifi if info.desktop_network.as_ref().is_some_and(|n|!n.wifi_available) => page("Wi-Fi",false,vec![Group::new(vec![Item::new("No Wi-Fi adapter").tint(Tint::Dim)]).footer("Connect a supported Wi-Fi adapter to browse and join wireless networks. Ethernet is managed separately in Network settings.")]),
        PageId::Wifi => {
            let current = info.current_ssid.as_deref().filter(|_| m.wifi_on);
            let mut first = vec![Item::new("Wi-Fi").switch(m.wifi_on, Action::Toggle(Toggle::Wifi))];
            if let Some(ssid) = current {
                first.push(Item { leading_check: true, accessory: Accessory::Info, action: Action::Push(PageId::WifiDetail), ..Item::new(ssid) });
            }
            let mut groups = vec![match info.wifi_notice.as_deref().filter(|_| m.wifi_on) {
                Some(n) => Group::new(first).footer(n),
                None => Group::new(first),
            }];
            if m.wifi_on {
                let nets: Vec<Item> = info
                    .networks
                    .iter()
                    .filter(|n| Some(n.ssid.as_str()) != current)
                    .map(|n| Item { accessory: Accessory::Wifi { secure: n.secure, bars: n.bars }, action: Action::Network(n.ssid.clone()), ..Item::new(&n.ssid).value(note_of(info, &n.ssid)) })
                    .collect();
                let mut nets = if nets.is_empty() { vec![Item::new("Searching…").tint(Tint::Dim)] } else { nets };
                nets.push(Item::new("Other…").action(Action::JoinOther));
                groups.push(Group::new(nets).header("NETWORKS"));
                if !info.saved.is_empty() {
                    groups.push(Group::new(vec![Item::new("Known Networks").value(info.saved.len().to_string()).link(PageId::KnownNetworks)]).footer("Known networks are joined automatically when they are in range."));
                }
                // the country whose radio rules the Wi-Fi chip follows; netd applies it from the same region setting
                groups.push(Group::new(vec![Item::new("Country or Region").value(bifrost_ui::region::current().region_name())]).footer("The Wi-Fi chip uses the channels and power levels allowed in this country. It follows the Region in Language & Region."));
            }
            page("Wi-Fi", false, groups)
        }
        PageId::WifiDetail => {
            let place = info.saved.iter().position(|n| n.ssid == m.detail);
            let auto_join = place.is_none_or(|i| info.saved[i].auto_join);
            let mut groups = vec![
                Group::new(vec![Item::new("Forget This Network").tint(Tint::Blue).action(Action::Run(Command::Forget(m.detail.into())))]),
                Group::new(vec![Item::new("Auto-Join").switch(auto_join, Action::Wifi(WifiAction::SetAutoJoin { ssid: m.detail.into(), on: !auto_join }))]),
            ];
            if info.current_ssid.as_deref() == Some(m.detail) {
                groups.push(rows(vec![("IP Address", info.ip.clone()), ("Router", info.router.clone())]).header("IPV4 ADDRESS"));
                // the link's details: security from the saved entry, the Wi-Fi interface from the desktop network state
                let mut details: Vec<(&'static str, String)> = vec![("Security", place.and_then(|i| info.saved[i].security).map_or("—", |s| s.label()).to_string())];
                if let Some(i) = info.desktop_network.as_ref().and_then(|n| n.interfaces.iter().find(|i| i.kind == "wifi")) {
                    details.push(("Interface", i.name.clone()));
                    details.push(("Configure IPv4", match i.method.as_str() { "auto" => "Automatic (DHCP)", "manual" => "Manually", _ => "—" }.to_string()));
                    details.push(("DNS Servers", if i.dns.is_empty() { "—".to_string() } else { i.dns.join(", ") }));
                }
                groups.push(rows(details).header("DETAILS"));
            }
            groups.push(rows(vec![("Wi-Fi Address", info.mac.clone())]));
            // why the last join failed (the saved note), so a network that will not connect says so here
            if let Some(problem) = place.map(|i| info.saved[i].note.as_str()).filter(|n| !n.is_empty()) {
                groups.push(rows(vec![("Problem", problem.to_string())]).header("DIAGNOSTICS"));
            }
            if let Some(i) = place {
                let step = |label: &str, up: bool, possible: bool| {
                    let row = Item::new(label).tint(if possible { Tint::Blue } else { Tint::Dim });
                    row.action(Action::Wifi(WifiAction::Move { ssid: m.detail.into(), up }))
                };
                groups.push(
                    Group::new(vec![step("Move Up", true, i > 0), step("Move Down", false, i + 1 < info.saved.len())])
                        .header(&format!("PRIORITY {} OF {}", i + 1, info.saved.len()))
                        .footer("Networks higher in the list are joined first when several are in range."),
                );
            }
            page(m.detail, false, groups)
        }
        PageId::Bluetooth => bluetooth_spec(m, page),
        PageId::BluetoothDetail => {
            let Some(d) = info.bt.devices.iter().find(|d| d.label() == m.detail) else {
                return page(m.detail, false, vec![Group::new(vec![Item::new("Device Not Found").tint(Tint::Dim)])]);
            };
            let busy = info.bt.busy.is_some();
            let link = match (d.connected, busy) {
                (_, true) => Item::new("Connecting…").tint(Tint::Dim),
                (true, _) => Item::new("Disconnect").tint(Tint::Blue).action(Action::Bluetooth(Cmd::Disconnect(d.addr.clone()))),
                (false, _) => Item::new("Connect").tint(Tint::Blue).action(Action::Bluetooth(Cmd::Connect(d.addr.clone()))),
            };
            // A connected headset, speaker or headphones can take the sound: the audio-devices service makes it the output.
            let mut head = vec![link];
            if d.connected && d.audio && !busy {
                head.push(Item::new("Use for Sound Output").tint(Tint::Blue).action(Action::AudioOutput));
            }
            page(
                d.label(),
                false,
                vec![
                    Group::new(head),
                    Group::new(vec![Item::new("Forget This Device").tint(Tint::Blue).action(Action::Run(Command::ForgetBluetooth(d.addr.clone())))]),
                    rows(vec![
                        ("Address", d.addr.clone()),
                        ("Type", d.type_label().to_string()),
                        ("Status", if d.connected { "Connected" } else { "Not Connected" }.to_string()),
                        // the properties rows of the SDK's device model (`RemoteDevice::properties`) end with the signal
                        ("Signal", d.rssi.map_or("Not reported".to_string(), |r| format!("{r} dBm"))),
                    ]),
                    // The service browser model: no service records are read yet, so the list is empty and says so.
                    Group::new(vec![Item::new("Services").value("None listed")]).header("SERVICES").footer(super::services::bluetooth_service::SERVICE_NOTE),
                ],
            )
        }
        PageId::KnownNetworks => {
            let rows: Vec<Item> = info
                .saved
                .iter()
                .map(|n| {
                    // security first, then the problem or "Auto-Join Off": "WPA2 Personal, Incorrect Password"
                    let state = if !n.note.is_empty() { n.note.clone() } else if n.auto_join { String::new() } else { "Auto-Join Off".to_string() };
                    let value = match (n.security, state.is_empty()) {
                        (Some(s), true) => s.label().to_string(),
                        (Some(s), false) => format!("{}, {}", s.label(), state),
                        (None, _) => state,
                    };
                    Item { accessory: Accessory::Info, action: Action::Push(PageId::WifiDetail), ..Item::new(&n.ssid).value(value) }
                })
                .collect();
            let rows = if rows.is_empty() { vec![Item::new("No Known Networks").tint(Tint::Dim)] } else { rows };
            page("Known Networks", false, vec![Group::new(rows).header("MY NETWORKS").footer("Networks higher in the list are joined first. Open a network to change its priority, turn off Auto-Join or forget it.")])
        }
        PageId::Wallpaper => page("Wallpaper", false, vec![]),
        PageId::ScreenSaver => screen_saver_page(o, &super::screensaver::manifest(o.screensaver_module).map(|m| m.sheet().clone()).unwrap_or_default()),
        PageId::Sounds => page(
            "Sounds",
            false,
            {
                let mut groups = vec![
                    Group::new(vec![Item::slider(Ends::Volume, o.volume, Action::Slider(Slider::Volume))]).header("RINGER AND ALERTS"),
                    Group::new(super::options::ALERT_SOUNDS.iter().enumerate().map(|(i, l)| Item::new(*l).check(i == o.alert_sound, Action::Pick(Pick::AlertSound(i)))).collect())
                        .header("ALERT SOUND")
                        .footer("The sound that plays when an alert arrives."),
                    Group::new(sound_switches(o, m.haptics)),
                ];
                groups.extend(super::sound_page::groups(m.sound));
                groups
            },
        ),
        PageId::Focus => page(
            "Focus",
            false,
            vec![Group::new(vec![Item::new("Do Not Disturb").icon(Icon::Focus).switch(o.dnd, Action::Toggle(Toggle::Dnd))])
                .footer("Silences notification banners and keeps the screen asleep. Notifications remain in Notification Center and on app badges. Alarms and timers still ring.")],
        ),
        PageId::General => {
            page(
                "General",
                false,
                vec![
                    Group::new(vec![Item::new("About").glyph(Glyph::About).link(PageId::About), Item::new("Software Update").glyph(Glyph::Update).badge(usize::from(m.update_badge)).link(PageId::SoftwareUpdate)]),
                    Group::new(vec![Item::new("Nearby Share").glyph(Glyph::Nearby).link(PageId::Nearby)]),
                    Group::new(vec![{
                        let now = GAME_MODES.iter().position(|(_, p)| *p == o.game_mode).unwrap_or(0);
                        Item::new("Game Mode").popup(GAME_MODES.iter().map(|(l, _)| l.to_string()).collect(), now, Action::Pick(Pick::GameMode(now)))
                    }])
                        .footer("Automatic gives a full-screen game the whole processor while it is in front, and lets go a few seconds after the game leaves the front."),
                    Group::new(vec![Item::new("Storage").glyph(Glyph::Storage).link(PageId::Storage), Item::new("Startup Disk").glyph(Glyph::Storage).link(PageId::StartupDisk), Item::new("Background App Refresh").glyph(Glyph::Refresh).link(PageId::BackgroundRefresh)]),
                    Group::new(vec![
                        Item::new("Date & Time").glyph(Glyph::DateTime).link(PageId::DateTime),
                        Item::new("Keyboard").glyph(Glyph::Keyboard).link(PageId::Keyboard),
                        Item::new("Fonts").glyph(Glyph::Fonts).link(PageId::Fonts),
                        Item::new("Language & Region").glyph(Glyph::Language).link(PageId::Language),
                        Item::new("Dictionary").glyph(Glyph::Dictionary).link(PageId::Dictionary),
                        Item::new("Default Apps").glyph(Glyph::DefaultApps).link(PageId::DefaultApps),
                        Item::new("Login Items & Extensions").glyph(Glyph::DefaultApps).link(PageId::LoginItems),
                        Item::new("Game Controllers").link(PageId::GameControllers),
                    ]),
                    Group::new(vec![Item::new("Legal & Regulatory").glyph(Glyph::Info).link(PageId::Legal)]),
                    Group::new(vec![Item::new("Backup").glyph(Glyph::Backup).link(PageId::Backup)]),
                    Group::new(vec![
                        Item::new("Restart").tint(Tint::Blue).action(Action::Run(Command::Reboot)),
                        Item::new("Shut Down").tint(Tint::Blue).action(Action::Run(Command::ShutDown)),
                    ]),
                ],
            )
        }
        PageId::About => {
            let mut groups: Vec<Group> = about(info).into_iter().map(rows).collect();
            groups.push(Group::new(vec![Item::new("Legal & Regulatory").link(PageId::Legal)]));
            page("About", false, groups)
        }
        PageId::Nearby => page(
            "Nearby Share",
            false,
            vec![Group::new(NEARBY_CHOICES.iter().enumerate().map(|(i, l)| Item::new(*l).check(i == o.nearby, Action::Pick(Pick::Nearby(i)))).collect())
                .footer("Nearby Share lets you share instantly with devices nearby. You can be discoverable to receive from everyone or only from devices you have already seen nearby.")],
        ),
        // the Settings pane reads the computer name and services on a worker and passes them in (`settings.rs`)
        PageId::Sharing => sharing_spec(o, m.sharing),
        // the Settings pane reads the printers on a worker and passes them in (`settings.rs`)
        PageId::PrintScan => super::print_scan_page::spec(m.printers),
        PageId::LockScreen => page(
            "Lock Screen",
            false,
            vec![
                if numeric_passcode() {
                    Group::new(vec![Item::new("Passcode").icon(Icon::Passcode).link(PageId::Passcode)])
                        .footer("A passcode is asked for before the screen opens again after it locks.")
                } else {
                    Group::new(vec![Item::new("Login Password").link(PageId::Users)])
                        .footer("The password you sign in with is asked for before the screen opens again after it locks. Change it in Users & Groups.")
                },
                Group::new(vec![Item::new("Lock After Inactivity").value(o.auto_lock_label()).link(PageId::AutoLock)]),
                Group::new(vec![Item::new("Require Password").value(o.lock_grace_label()).link(PageId::RequirePassword)])
                    .footer("After the display has been off longer than this, the screen asks for the password before it opens again."),
                Group::new(vec![Item::new("Screen Saver").value(if o.screensaver_minutes == 0 { "Never" } else { super::screensaver::saver_name(o.screensaver_module) }).link(PageId::ScreenSaver)])
                    .footer("The screen saver can start after a while without input, and then the lock applies as usual."),
            ],
        ),
        PageId::AirPlayHandoff => page(
            "AirPlay & Handoff",
            false,
            vec![
                Group::new(vec![Item::new("Automatically AirPlay to TVs").value("Never")]),
                Group::new(vec![Item::new("Handoff").switch(o.handoff, Action::Toggle(Toggle::Handoff))])
                    .footer("Handoff lets you start something on one device and instantly pick it up on other devices nearby."),
                Group::new(vec![Item::new("Share Clipboard Between Your Devices").switch(o.universal_clipboard, Action::Toggle(Toggle::UniversalClipboard))])
                    .footer("Copies of text, links and images are encrypted on this device before they leave it, and are kept in the cloud for two minutes at most. Passwords from password managers are never shared."),
            ],
        ),
        PageId::DefaultApps => page(
            "Default Apps",
            false,
            m.defaults
                .iter()
                .enumerate()
                .map(|(c, v)| {
                    let rows: Vec<Item> = if v.apps.is_empty() { vec![Item::new("No app available")] } else { v.apps.iter().map(|(key, name)| Item::new(name.as_str()).check(v.current.as_deref() == Some(key.as_str()), Action::DefaultApp(c, key.clone()))).collect() };
                    Group::new(rows).header(&v.label.to_uppercase())
                })
                .chain(std::iter::once(Group::new(vec![]).footer("Choosing an app opens every file of that kind with it, as Change All does in Files's Get Info. Apps announce what they can open in their manifest.")))
                .collect(),
        ),
        PageId::LoginItems => page(
            "Login Items & Extensions",
            false,
            {
                // an app that asked to open at login waits here until the user allows it (SMAppService: requiresApproval)
                let mut groups = Vec::new();
                if !m.login_pending.is_empty() {
                    groups.push(
                        Group::new(m.login_pending.iter().map(|(id, name)| Item::new(name.as_str()).tint(Tint::Blue).action(Action::LoginItemApprove(id.clone()))).collect())
                            .header("REQUIRES APPROVAL")
                            .footer("These apps asked to open when you log in. Choose one to allow it."),
                    );
                }
                groups.push(
                    Group::new(if m.login_items.is_empty() {
                        vec![Item::new("No login items")]
                    } else {
                        m.login_items.iter().map(|(id, name)| Item::new(name.as_str()).tint(Tint::Blue).action(Action::LoginItemRemove(id.clone()))).collect()
                    })
                    .header("OPEN AT LOGIN")
                    .footer("These apps start when you log in. Choose one to remove it. An app adds itself with SMAppService and can be removed here at any time."),
                );
                groups.extend(super::login_items_page::order_group(m.login_items));
                groups.extend(super::login_items_page::background_group(m.ext));
                groups.extend(super::login_items_page::extension_groups(m.ext));
                groups
            },
        ),
        PageId::Pip => page(
            "Picture in Picture",
            false,
            vec![Group::new(vec![Item::new("Start PiP Automatically").switch(o.pip_auto, Action::Toggle(Toggle::PipAuto))])
                .footer("When this is on, videos will continue playing in an overlay, even when you press the Home button.")],
        ),
        PageId::BackgroundRefresh => page(
            "Background App Refresh",
            false,
            {
                let mut groups = vec![Group::new(vec![Item::new("Background App Refresh").switch(o.bg_refresh, Action::Toggle(Toggle::BgRefresh))])
                    .footer("Allow apps to refresh their content in the background. Turning off apps may help preserve battery life. Refresh is paused in Low Power Mode.")];
                groups.push(Group::new(vec![Item::new("Suspend Background Apps").switch(o.suspend_background, Action::Toggle(Toggle::SuspendBackground))])
                    .footer("Stop apps that nothing shows after a short wait, and resume them when you return to them. A stopped app does no work, so it also stops playing audio in the background."));
                groups.extend(super::login_items_page::refresh_group(m.ext));
                groups
            },
        ),
        PageId::Gestures => page(
            "Gestures",
            false,
            vec![
                Group::new(vec![Item::new("Four & Five Finger Swipe").switch(o.four_finger, Action::Toggle(Toggle::FourFinger))]).header("MULTITASKING GESTURES").footer("Use four or five fingers to swipe between apps."),
                Group::new(vec![Item::new("Allow Finger to Swipe From Corner").switch(o.corner_swipe, Action::Toggle(Toggle::CornerSwipe))]).header("CORNER GESTURES"),
                Group::new(vec![Item::new("Left Corner Swipe").value(if o.corner_swipe { "Screenshot" } else { "Off" }), Item::new("Right Corner Swipe").value("Off")])
                    .footer("Actions when you swipe your finger from a bottom corner."),
            ],
        ),
        PageId::Trackpad => page(
            "Trackpad",
            false,
            vec![
                Group::new(vec![Item::slider(Ends::Speed, speed_slider(o.trackpad_speed), Action::Slider(Slider::TrackpadSpeed))]).header("TRACKING SPEED"),
                Group::new(vec![Item::new("Tap to Click").switch(o.tap_to_click, Action::Toggle(Toggle::TapToClick))]).footer("A light tap on the trackpad clicks."),
                Group::new(vec![Item::new("Secondary Click").popup(choice_labels(&TRACKPAD_SECONDARY), o.trackpad_secondary, Action::Pick(Pick::TrackpadSecondary(o.trackpad_secondary)))]),
                Group::new(vec![Item::new("Natural Scrolling").switch(o.natural_scroll, Action::Toggle(Toggle::NaturalScroll))]).footer("The content moves with your fingers, as it does on a touch screen."),
                Group::new(vec![
                    Item::new("Four & Five Finger Swipe").value(on_off(o.four_finger)).link(PageId::Gestures),
                    Item::new("Swipe From Corner").value(on_off(o.corner_swipe)).link(PageId::Gestures),
                ])
                .header("GESTURES")
                .footer("Gestures are set in General › Gestures."),
            ],
        ),
        PageId::Mouse => page("Mouse", false, super::mouse_page::groups(o)),
        PageId::Users | PageId::UserDetail | PageId::Groups | PageId::GroupDetail => super::users_page::spec(id, m.users, m.detail, "", o),
        PageId::Fonts => fonts_page(m),
        PageId::FontPick(role) => font_pick_page(m, role),
        PageId::RefreshRate => refresh_rate_page(m),
        PageId::Resolution => resolution_page(m),
        PageId::Dictionary => page(
            "Dictionary",
            false,
            vec![Group::new(dictionaries().into_iter().map(|d| Item::new(d).check(true, Action::None)).collect())],
        ),
        PageId::Legal => page(
            "Legal & Regulatory",
            false,
            vec![
                rows(vec![("Model", info.model.clone()), ("Regulatory Model", info.model.clone())]).footer("BifrOSt is open software; third-party components remain under their own licenses."),
            ],
        ),
        PageId::SoftwareUpdate => page(
            "Software Update",
            false,
            vec![rows(vec![("BifrOSt", BUILD.version.into()), ("Build", BUILD.git.into()), ("Built", BUILD.date.into())])
                .footer("BifrOSt updates arrive over the air from the BifrOSt repository; the build below is what is installed now.")],
        ),
        PageId::DateTime => page(
            "Date & Time",
            false,
            vec![
                Group::new(vec![Item::new("Set Automatically").switch(!info.time_manual, Action::Toggle(Toggle::AutoTime))]).footer(&info.time_sync),
                Group::new(vec![Item::new("24-Hour Time").switch(o.clock_24h, Action::Toggle(Toggle::Clock24))]),
                Group::new(vec![Item::new("Show Date in Menu Bar").switch(o.clock_date, Action::Toggle(Toggle::ClockDate)), Item::new("Show Day of the Week").switch(o.clock_day, Action::Toggle(Toggle::ClockDay))]),
                Group::new(vec![Item::new("Time Zone").value(info.timezone.as_str()).link(PageId::DateTime).action(Action::TimeZone), { let t = Item::new("Time").value(crate::status::clock_in(info.hour, info.min, o.clock_24h)); if info.time_manual { t.link(PageId::DateTime).action(Action::SetTime) } else { t } }]),
            ],
        ),
        PageId::Keyboard => page(
            "Keyboard",
            false,
            vec![
                Group::new(vec![
                    Item::new("Keyboards").value(super::kb_layouts::summary()).link(PageId::Keyboards),
                    Item::new("Text Replacements").value(super::services::text_services::replacements_summary()).link(PageId::TextReplacements),
                    Item::new("Dictation").link(PageId::Dictation),
                ]),
                Group::new(vec![
                    Item::new("Auto-Capitalisation").switch(o.auto_caps, Action::Toggle(Toggle::AutoCaps)),
                    Item::new("Enable Caps Lock").switch(o.enable_caps_lock, Action::Toggle(Toggle::EnableCapsLock)),
                    Item::new("\".\" Shortcut").switch(o.period_shortcut, Action::Toggle(Toggle::PeriodShortcut)),
                ])
                .header("ALL KEYBOARDS")
                .footer("Double-tapping the space bar will insert a period followed by a space."),
                Group::new(vec![
                    Item::slider(Ends::Delay, super::options::repeat_delay_slider(o.key_repeat_delay_ms), Action::Slider(Slider::KeyRepeatDelay)),
                    Item::slider(Ends::Rate, super::options::repeat_rate_slider(o.key_repeat_rate), Action::Slider(Slider::KeyRepeatRate)),
                ])
                .header("KEY REPEAT")
                .footer("Delay Until Repeat is how long a key is held before it repeats. Key Repeat is how fast it repeats after that."),
                Group::new(vec![Item::new("Modifier Keys").link(PageId::ModifierKeys)])
                    .footer("Choose what Caps Lock, Control, Option and Command do."),
                Group::new(vec![Item::new("Use F1, F2, etc. as Standard Function Keys").switch(o.function_standard, Action::Toggle(Toggle::FunctionStandard))])
                    .footer("When on, the function keys type F1 to F12 into apps and the terminal. When off, they keep the shell's own actions."),
                Group::new(vec![Item::new("Tab Moves Focus Between Controls").switch(o.tab_navigation, Action::Toggle(Toggle::TabNavigation))])
                    .header("KEYBOARD NAVIGATION")
                    .footer("When off, Tab does not move focus between controls. The terminal still receives Tab for completion."),
            ],
        ),
        PageId::ModifierKeys => page(
            "Modifier Keys",
            false,
            MODIFIER_HEADERS
                .iter()
                .enumerate()
                .map(|(slot, header)| {
                    let current = o.key_prefs().remap.slot(slot);
                    Group::new(
                        super::kb_input::REMAP_TARGETS
                            .iter()
                            .enumerate()
                            .map(|(t, (label, _))| Item::new(*label).check(t == current, Action::Pick(Pick::Remap(slot, t))))
                            .collect(),
                    )
                    .header(header)
                })
                .collect(),
        ),
        PageId::Keyboards => page(
            "Keyboards",
            false,
            vec![Group::new(super::kb_layouts::Layout::ALL.iter().enumerate().map(|(i, l)| Item::new(l.name()).switch(super::kb_layouts::is_enabled(*l), Action::Toggle(Toggle::Layout(i)))).collect()).footer("The globe key switches between enabled keyboards; touch and hold it to choose one."), Group::new(vec![Item::new("Remember Input Source for Each App").switch(super::kb_layouts::remembers(), Action::Toggle(Toggle::RememberInputSource))]).footer("Each app gets back the keyboard it had when it was last in front."), Group::new(vec![Item::new("Show Input menu in menu bar").switch(o.input_menu, Action::Toggle(Toggle::InputMenu))]).footer("The Input menu always shows while more than one keyboard is enabled. It lists the keyboards and opens the Keyboard Viewer and the on-screen keyboard.")]),
        PageId::Dictation => dictation_page(&super::services::recognizer::model_state(&super::services::recognizer::user_data_dir())),
        PageId::TextReplacements => {
            let mut items: Vec<Item> = super::services::text_services::replacement_rows()
                .iter()
                .map(|(shortcut, phrase)| {
                    Item::new(shortcut.as_str()).value(phrase.as_str()).action(Action::Run(Command::RemoveTextReplacement(shortcut.clone())))
                })
                .collect();
            if items.is_empty() {
                items.push(Item::new("No Text Replacements").tint(Tint::Dim));
            }
            page(
                "Text Replacements",
                false,
                vec![Group::new(items).footer(by_form("Tap an entry to remove it. An app expands a shortcut when it asks the text service.", "Click an entry to remove it. An app expands a shortcut when it asks the text service."))],
            )
        }
        PageId::Language => {
            let r = bifrost_ui::region::current();
            let lang = r.language().to_string();
            let first = r.first_weekday;
            page(
                "Language & Region",
                false,
                vec![
                    Group::new(
                        bifrost_ui::region::LANGUAGES
                            .iter()
                            .enumerate()
                            .map(|(i, (code, name))| Item::new(*name).check(*code == lang.as_str(), Action::Pick(Pick::Language(i))))
                            .collect(),
                    )
                    .header("PREFERRED LANGUAGE")
                    .footer("The first language is the one the system and apps use."),
                    Group::new(vec![Item::new("Region").value(r.region_name()).link(PageId::Region)])
                        .footer("The region sets how dates, numbers and currency are shown."),
                    Group::new(vec![
                        Item::new("Automatic").check(first.is_none(), Action::Pick(Pick::FirstWeekday(0))),
                        Item::new("Sunday").check(first == Some(0), Action::Pick(Pick::FirstWeekday(1))),
                        Item::new("Monday").check(first == Some(1), Action::Pick(Pick::FirstWeekday(2))),
                    ])
                    .header("FIRST DAY OF WEEK")
                    .footer("Calendars start each week on this day. Automatic follows the region."),
                    Group::new(vec![
                        Item::new("Date").value(bifrost_ui::region::short_date(&r, info.wday, info.mday, info.mon)),
                        Item::new("Currency").value(r.currency()),
                    ])
                    .header("FORMATS"),
                ],
            )
        }
        PageId::Region => {
            let current = bifrost_ui::region::current().region_code();
            page(
                "Region",
                false,
                vec![Group::new(
                    bifrost_ui::region::REGIONS
                        .iter()
                        .enumerate()
                        .map(|(i, (code, name))| Item::new(*name).check(*code == current.as_str(), Action::Pick(Pick::Region(i))))
                        .collect(),
                )
                .footer("The region sets the date, number, currency and first day of the week formats that apps use.")],
            )
        }
        PageId::Reset => page(
            "Reset",
            false,
            vec![
                Group::new(vec![Item::new("Reset All Settings").tint(Tint::Blue).action(Action::Run(Command::ResetSettings))]).footer("Returns every setting, including the wallpaper, to its default. Saved Wi-Fi networks are kept."),
                Group::new(vec![Item::new("Restart BifrOSt").tint(Tint::Blue).action(Action::Run(Command::Restart))]).footer("Quits the shell and starts it again."),
                Group::new(vec![Item::new("Erase All Content and Settings").tint(Tint::Red).action(Action::Run(Command::EraseContent))]).footer("Removes your files, accounts, passwords and settings from this computer and starts again as new. The system stays installed."),
            ],
        ),
        PageId::Display => page(
            "Display & Brightness",
            false,
            {
            let mut groups = vec![
                Group::new(vec![Item::slider(Ends::Brightness, m.brightness, Action::Slider(Slider::Brightness))]).header("BRIGHTNESS"),
                Group::new(vec![Item::new("Night Shift").value(super::night_shift::published().label()).link(PageId::NightShift), Item::new("True Tone").value("Off")]),
                Group::new(vec![Item::new("Auto-Lock").value(o.auto_lock_label()).link(PageId::AutoLock)]),
                Group::new(vec![
                    Item::slider(Ends::TextSize, o.text_size_step() as f32 / (TEXT_SIZES.len() - 1) as f32, Action::Slider(Slider::TextSize)),
                    Item::new("Bold Text").switch(o.bold_text, Action::Toggle(Toggle::BoldText)),
                ])
                .header("TEXT SIZE")
                .footer("The text in the shell scales as you drag the slider. Bold Text makes every weight heavier."),
            ];
            // a desktop panel also picks the size of everything
            if crate::layout::adaptive() {
                groups.insert(3, Group::new(vec![Item::new("Interface Scale").value(scale_label(crate::layout::user_scale())).link(PageId::InterfaceScale)]));
            }
            groups
            },
        ),
        PageId::AutoLock => page("Auto-Lock", false, vec![Group::new(AUTO_LOCK.iter().enumerate().map(|(i, (l, ms))| Item::new(*l).check(*ms == o.auto_lock_ms, Action::Pick(Pick::AutoLock(i)))).collect())]),
        PageId::RequirePassword => page(
            "Require Password",
            false,
            vec![Group::new(REQUIRE_AFTER.iter().enumerate().map(|(i, (l, s))| Item::new(*l).check(*s == o.lock_grace_s, Action::Pick(Pick::RequirePassword(i)))).collect())
                .footer("Choose how long the screen can stay off before a password is asked. Immediately asks for it every time the screen comes back on.")],
        ),
        PageId::Energy => page(
            "Energy",
            false,
            vec![Group::new(vec![system_sleep_row(o)])
                .header("PUT THE COMPUTER TO SLEEP WHEN THE DISPLAY IS OFF")
                .footer("Turning the display off or locking the screen never sleeps the computer by itself; this timeout starts once the display is off. The change takes effect at once.")]
                .into_iter()
                // a computer without a battery has no Battery pane: its power modes live here
                .chain(if has_battery(info) {
                    Vec::new()
                } else {
                    vec![
                        Group::new(vec![Item::new("Low Power Mode").switch(o.low_power, Action::Toggle(Toggle::LowPower))])
                            .footer("Caps the brightness at 50%, locks the screen after 30 seconds and limits animations to 30 frames a second until you turn it off."),
                        Group::new(vec![power_mode_row(o)])
                            .footer(&format!(
                                "In effect: {}. Low Power Mode caps the clock speeds to save energy.",
                                if info.power_profile.is_empty() { "Balanced" } else { &info.power_profile }
                            )),
                    ]
                })
                .collect(),
        ),
        PageId::HomeScreen => page(
            "Home Screen & Dock",
            false,
            vec![Group::new(ICON_SIZES.iter().enumerate().map(|(i, l)| Item::new(*l).check(i == o.icon_size, Action::Pick(Pick::IconSize(i)))).collect()).header("APP ICONS"),
                Group::new(vec![Item::new("Show Suggested and Recent Apps in Dock").switch(o.dock_recents, Action::Toggle(Toggle::DockRecents))]).header("DOCK")],
        ),
        PageId::Battery => page(
            "Battery",
            false,
            vec![
                rows(vec![
                    ("Battery Level", battery_level(info)),
                    ("Status", if info.charging { "Charging" } else { "Not Charging" }.into()),
                    ("Last Charged", last_charged(&info.battery_history, m.now, info.charging)),
                ]),
                Group::new(vec![Item::graph(Graph { bins: battery::bins(&info.battery_history, m.now), end_hour: info.hour, end_min: info.min })]).header("LAST 24 HOURS"),
                Group::new(info.battery_health.iter().map(|(k, v)| Item::new(k.clone()).value(v.clone())).collect()).header("BATTERY HEALTH"),
                Group::new(vec![Item::new("Energy").value(o.system_sleep_label()).link(PageId::Energy)]),
                Group::new(vec![Item::new("Battery Percentage").switch(o.battery_percent, Action::Toggle(Toggle::BatteryPercent))]),
                Group::new(vec![Item::new("Low Power Mode").switch(o.low_power, Action::Toggle(Toggle::LowPower))])
                    .footer("Caps the brightness at 50%, locks the screen after 30 seconds and limits animations to 30 frames a second until you turn it off."),
                Group::new(vec![power_mode_row(o)])
                    .footer(&format!(
                        "In effect: {}. Automatic runs Performance while charging and Battery Saver at 30% or less; Low Power Mode always means Battery Saver, which caps the clock speeds to save energy.",
                        if info.power_profile.is_empty() { "Balanced" } else { &info.power_profile }
                    )),
            ],
        ),
        PageId::MenuBar => page(
            "Menu Bar",
            false,
            vec![Group::new(super::options::MenuExtra::ALL.iter().filter(|&&e| e != super::options::MenuExtra::Battery || has_battery(info)).map(|&e| Item::new(e.label()).switch(o.menu_extras & e.bit() != 0, Action::Toggle(Toggle::MenuExtra(e)))).collect())
                .header("MENU BAR CONTROLS")
                .footer("Choose the controls and readouts shown at the right of the menu bar, beside the clock. Controls that do not fit are left out from the left.")],
        ),
        PageId::ControlCentre => page(
            "Control Center",
            false,
            vec![rows(vec![("Brightness", String::new()), ("Volume", String::new()), ("Wi-Fi", String::new()), ("Screen Lock", String::new()), ("Focus", on_off(o.dnd).into())])
                .header("INCLUDED CONTROLS")
                .footer(by_form("Swipe down from the top-right corner of the screen to open Control Center.", "Click the Control Center icon in the menu bar to open Control Center."))],
        ),
        PageId::Accessibility => {
            let a = m.a11y;
            page(
                "Accessibility",
                false,
                vec![
                    Group::new(vec![
                        Item::new("Display Zoom").value(ZOOMS[a.zoom.min(ZOOMS.len() - 1)].0).link(PageId::DisplayZoom),
                        Item::new("Larger Text").value(format!("{:.0}%", o.text_scale * 100.0)).link(PageId::Display),
                        Item::new("Bold Text").switch(o.bold_text, Action::Toggle(Toggle::BoldText)),
                        Item::new("Increase Contrast").switch(a.increase_contrast, Action::Toggle(Toggle::IncreaseContrast)),
                        reduce_transparency_item(a.reduce_transparency, a.increase_contrast),
                        Item::new("Show Borders").switch(a.button_shapes, Action::Toggle(Toggle::ButtonShapes)),
                        Item::new("Dim Flashing Lights").switch(a.dim_flashing_lights, Action::Toggle(Toggle::DimFlashingLights)),
                        Item::new("Guided Access").switch(a.guided_access, Action::Toggle(Toggle::GuidedAccess)),
                        Item::new("Screen Reader").switch(a.screen_reader, Action::Toggle(Toggle::ScreenReader)),
                        Item::new("Speech").value(SPEECH_VOICES[speech_voice_index(&a.speech_voice)].0).link(PageId::ScreenReaderSpeech),
                    ])
                    .header("VISION")
                    .footer("Screen Reader speaks the element under the keyboard cursor. Ctrl+Alt+Right and Ctrl+Alt+Left move it."),
                    Group::new(vec![
                        Item::new("Sticky Keys").switch(a.sticky_keys, Action::Toggle(Toggle::StickyKeys)),
                        Item::new("Mouse Keys").switch(a.mouse_keys, Action::Toggle(Toggle::MouseKeys)),
                        Item::new("Accessibility Keyboard").switch(a.accessibility_keyboard, Action::Toggle(Toggle::AccessibilityKeyboard)),
                        Item::new("Slow Keys").value(SLOW_MS[a.slow.min(SLOW_MS.len() - 1)].0).link(PageId::SlowKeys),
                    ])
                        .header("KEYBOARD")
                        .footer("Press a modifier key once to hold it for the next key you press, or twice to hold it until you press it again. Slow Keys ignores a key that is not held long enough. The Accessibility Keyboard opens the on-screen keyboard until you close it with its hide key or turn this off."),
                    Group::new({
                        let mut rows = vec![Item::new("Reduce Motion").switch(a.reduce_motion, Action::Toggle(Toggle::ReduceMotion))];
                        if m.info.motion_sensor_supported {
                            rows.push(Item::new("Show Vehicle Motion Cues").switch(a.motion_cues, Action::Toggle(Toggle::MotionCues)));
                        }
                        rows
                    })
                    .header("MOTION")
                    .footer(if m.info.motion_sensor_supported {
                        "Opening, closing and switching apps cross-fade instead of zooming. Vehicle Motion Cues show dots at the edge of the screen that follow the motion of a moving vehicle."
                    } else {
                        "Opening, closing and switching apps cross-fade instead of zooming."
                    }),
                    Group::new(vec![Item::new("Live Captions").switch(a.live_captions, Action::Toggle(Toggle::LiveCaptions))])
                        .header("HEARING")
                        .footer("Shows what is said in the sound this computer plays as captions at the bottom of the screen. It uses the on-device speech model from Keyboard > Dictation, English only, and nothing leaves the computer."),
                    Group::new(vec![Item::slider(Ends::Cursor, a.cursor_size, Action::Slider(Slider::CursorSize))]).header("POINTER").footer("The pointer grows as you drag the slider."),
                    Group::new(vec![Item::new("Switch Scanning").switch(a.switch_scan, Action::Toggle(Toggle::SwitchScan))])
                        .header("SWITCH")
                        .footer("A highlight steps through the screen by itself. Press Ctrl+Alt+Space to press the highlighted element, or Ctrl+Alt+Right to step by hand."),
                    Group::new(vec![Item::new("Accessibility Shortcut").value(shortcut_label(&a.shortcut)).link(PageId::AccessibilityShortcut)])
                        .header("SHORTCUT")
                        .footer("Press Option+Cmd+F5 to open a panel where you switch the chosen features on or off."),
                    Group::new(vec![
                        Item::new("AssistiveTouch").switch(o.assistive_touch, Action::Toggle(Toggle::AssistiveTouch)),
                        Item::new("Touch Accommodations").value(HOLD_MS[a.hold.min(HOLD_MS.len() - 1)].0).link(PageId::TouchHold),
                        Item::new("Dwell Control").value(DWELL_MS[a.dwell.min(DWELL_MS.len() - 1)].0).link(PageId::DwellControl),
                        Item::new("Switch Control").switch(a.switch_control, Action::Toggle(Toggle::SwitchControl)),
                        Item::new("Voice Control").switch(a.voice_control, Action::Toggle(Toggle::VoiceControl)),
                    ])
                    .header("TOUCH")
                    .footer("Use the floating button to reach Home, Control Center, notifications and other controls. Switch Control highlights one item at a time: Ctrl+Alt+Right and Left move the highlight and Ctrl+Alt+Space selects it. The switch keys also move through a panel of controls (Space) and choose from it (Enter); Escape closes the panel. Voice Control listens through the microphone on this computer, with the dictation model from Keyboard settings, and presses what you name: say \"press Save\", or \"show numbers\", \"show names\" or \"show grid\" to label what is on screen. Your speech is never uploaded."),
                ],
            )
        }
        PageId::ScreenReaderSpeech => page(
            "Speech",
            false,
            vec![
                Group::new(SPEECH_RATES.iter().enumerate().map(|(i, (l, wpm))| Item::new(*l).check(*wpm == m.a11y.speech_rate, Action::Pick(Pick::SpeechRate(i)))).collect())
                    .header("SPEAKING RATE")
                    .footer("How fast the screen reader speaks."),
                Group::new(SPEECH_PITCHES.iter().enumerate().map(|(i, (l, p))| Item::new(*l).check(*p == m.a11y.speech_pitch, Action::Pick(Pick::SpeechPitch(i)))).collect())
                    .header("PITCH")
                    .footer("A higher pitch sounds lighter."),
                Group::new(SPEECH_VOICES.iter().enumerate().map(|(i, (l, v))| Item::new(*l).check(*v == m.a11y.speech_voice, Action::Pick(Pick::SpeechVoice(i)))).collect())
                    .header("VOICE")
                    .footer("Needs the espeak-ng speech engine. Without it the screen reader stays silent and shows the text."),
                Group::new(SPEECH_PUNCTUATION.iter().enumerate().map(|(i, (l, _))| Item::new(*l).check(i == m.a11y.speech_punctuation.min(SPEECH_PUNCTUATION.len() - 1), Action::Pick(Pick::SpeechPunctuation(i)))).collect())
                    .header("PUNCTUATION")
                    .footer("None only pauses at punctuation. Some names symbols such as @ and #. All names every mark."),
            ],
        ),
        PageId::DisplayZoom => page("Display Zoom", false, vec![Group::new(ZOOMS.iter().enumerate().map(|(i, (l, _))| Item::new(*l).check(i == m.a11y.zoom, Action::Pick(Pick::DisplayZoom(i)))).collect()).header("VIEW")]),
        PageId::Appearance => appearance_page(m, &DeskPrefs::default()),
        PageId::HotCorners => hot_corners_page(&DeskPrefs::default()),
        PageId::Shortcuts => shortcuts_page(o),
        // the Settings window builds the live page from the saved settings (`settings::spec_for`)
        PageId::Assistant => super::assistant_page::spec(&super::assistant::Settings::default(), &[]),
        PageId::Overview => overview_page(&DeskPrefs::default()),
        PageId::InterfaceScale => {
            let now = crate::layout::user_scale();
            let rows = std::iter::once(None).chain(crate::layout::SCALE_CHOICES.iter().map(|&s| Some(s))).enumerate();
            page(
                "Interface Scale",
                false,
                vec![Group::new(rows.map(|(i, s)| Item::new(scale_label(s)).check(s == now, Action::Pick(Pick::InterfaceScale(i)))).collect())
                    .header("SIZE OF EVERYTHING")
                    .footer("Auto picks a size from the screen's resolution and physical size.")],
            )
        }
        PageId::TouchHold => page(
            "Touch Accommodations",
            false,
            vec![Group::new(HOLD_MS.iter().enumerate().map(|(i, (l, _))| Item::new(*l).check(i == m.a11y.hold, Action::Pick(Pick::TouchHold(i)))).collect())
                .header("HOLD DURATION")
                .footer("A press must last this long before it counts as a touch and hold.")],
        ),
        PageId::DwellControl => page(
            "Dwell Control",
            false,
            vec![Group::new(DWELL_MS.iter().enumerate().map(|(i, (l, _))| Item::new(*l).check(i == m.a11y.dwell, Action::Pick(Pick::Dwell(i)))).collect())
                .header("DWELL DELAY")
                .footer("Rest the pointer in one place for this long and it clicks there, without a button press. Off turns Dwell Control off.")],
        ),
        PageId::SlowKeys => page(
            "Slow Keys",
            false,
            vec![Group::new(SLOW_MS.iter().enumerate().map(|(i, (l, _))| Item::new(*l).check(i == m.a11y.slow, Action::Pick(Pick::Slow(i)))).collect())
                .header("ACCEPTANCE DELAY")
                .footer("A key must be held this long before it types. Off turns Slow Keys off.")],
        ),
        PageId::AccessibilityShortcut => page(
            "Accessibility Shortcut",
            false,
            vec![Group::new(Feature::ALL.iter().enumerate().map(|(i, f)| Item::new(f.name()).check(m.a11y.shortcut.contains(f), Action::Pick(Pick::AccessibilityShortcut(i)))).collect())
                .header("FEATURES")
                .footer("Option+Cmd+F5 opens a panel with a checkbox for each feature checked here. Choose a feature in the panel to switch it.")],
        ),
        // built by `game_controllers_page::spec`, which reads the live controllers
        PageId::GameControllers | PageId::GameController(_) => page("", false, vec![]),
        // built by `storage_page::spec`, which also knows the live state
        PageId::Storage | PageId::Graphics | PageId::StartupDisk | PageId::Passcode | PageId::Passwords | PageId::InternetAccounts | PageId::Account | PageId::Developer | PageId::Backup | PageId::ScreenTime | PageId::StDowntime | PageId::StLimits | PageId::StAllowed => page("", false, vec![]),
        // built by `notify_page::spec`
        PageId::Notifications | PageId::NotificationApp | PageId::NightShift | PageId::WakeSchedule => page("Notifications", false, vec![]),
        PageId::Spotlight => {
            use super::spotlight_scope::Category;
            let scope = &o.spotlight;
            let kinds = Category::ALL.into_iter().map(|c| Item::new(c.title()).switch(scope.allows(c), Action::Toggle(Toggle::SpotlightCategory(c)))).collect();
            let folders: Vec<Item> = scope.excluded().iter().enumerate().map(|(i, f)| Item::new(f.display().to_string()).action(Action::Pick(Pick::SpotlightExclude(i)))).collect();
            let folders = if folders.is_empty() { vec![Item::new("No Excluded Folders").tint(Tint::Dim)] } else { folders };
            page(
                "Search",
                false,
                vec![
                    Group::new(kinds).header("RESULTS").footer("Turn off a kind of result to keep it out of Search."),
                    Group::new(folders).header("PRIVACY").footer(by_form("Search never lists files in an excluded folder or inside it. Tap a folder to include it again.", "Search never lists files in an excluded folder or inside it. Click a folder to include it again.")),
                ],
            )
        }
        PageId::Privacy => page("Privacy", false, vec![Group::new(vec![Item::new("App Permissions").link(PageId::PrivacyPermissions), Item::new("Location Services").link(PageId::LocationServices), Item::new("Analytics & Diagnostics").link(PageId::Diagnostics)]).footer("BifrOSt keeps everything on this computer and sends nothing anywhere unless you share a diagnostics report.")]),
        // built by `diagnostics_page::spec`, which knows the crash logs
        PageId::Diagnostics => page("Analytics & Diagnostics", false, vec![]),
        // built by `permissions_page`, which reads the installed apps and their answers
        PageId::PrivacyPermissions => page("App Permissions", false, vec![]),
        PageId::AppPanes(slot) => super::settings_panes::spec(slot),
        // built by `location_page::spec`, which reads the prefs and the location answers
        PageId::LocationServices => page("Location Services", false, vec![]),
    }
}

/// Keyboard > Dictation: the speech model state, what it needs from the person, and the download or removal.
pub fn dictation_page(v: &super::services::recognizer::ModelView) -> Spec {
    let state = if v.downloading {
        "Downloading…"
    } else if v.installed {
        "Installed"
    } else {
        "Not Downloaded"
    };
    let size = format!("{} MB", bifrost_ui::whisper::MODEL_SIZE_BYTES / 1_000_000);
    let mut groups = vec![Group::new(vec![Item::new("Speech Model").value(state), Item::new("Size").value(size)])
        .header("ON-DEVICE DICTATION")
        .footer("The microphone key dictates text into the focused field. The model runs on this computer, and your speech is never uploaded.")];
    if v.installed {
        groups.push(Group::new(vec![Item::new("Remove Speech Model").tint(Tint::Red).action(Action::Run(Command::RemoveSpeechModel))])
            .footer("Removing it turns dictation off until the model is downloaded again."));
    } else if !v.downloading {
        let footer = match &v.failed {
            Some(e) => e.clone(),
            None => "Downloading the model needs your consent. It is about 148 MB and comes from the model's public download address; no audio is sent anywhere.".to_string(),
        };
        groups.push(Group::new(vec![Item::new("Download Speech Model").tint(Tint::Blue).action(Action::Run(Command::DownloadSpeechModel))]).footer(&footer));
    }
    Spec { id: PageId::Dictation, title: "Dictation".into(), large: false, groups }
}

/// The index of the `SPEECH_VOICES` choice for an engine voice name; an unknown name shows as the default voice (0).
pub fn speech_voice_index(voice: &str) -> usize {
    SPEECH_VOICES.iter().position(|(_, v)| *v == voice).unwrap_or(0)
}

/// The value the Accessibility Shortcut row shows: the features it switches, or None.
pub fn shortcut_label(features: &[Feature]) -> String {
    match features {
        [] => "None".into(),
        [one] => one.name().into(),
        many => format!("{} Features", many.len()),
    }
}

/// Energy: how long the display stays off before the computer sleeps, one pop-up row over `SYSTEM_SLEEP`.
fn system_sleep_row(o: &Options) -> Item {
    let now = SYSTEM_SLEEP.iter().position(|(_, m)| *m == o.system_sleep_min).unwrap_or(SYSTEM_SLEEP.len() - 1);
    Item::new("Sleep After").popup(SYSTEM_SLEEP.iter().map(|(l, _)| l.to_string()).collect(), now, Action::Pick(Pick::SystemSleep(now)))
}

/// Energy and Battery: Automatic, Performance, Balanced or Battery Saver, one pop-up row over `POWER_MODES`.
fn power_mode_row(o: &Options) -> Item {
    let now = POWER_MODES.iter().position(|(_, p)| *p == o.power_profile).unwrap_or(0);
    Item::new("Power Mode").popup(POWER_MODES.iter().map(|(l, _)| l.to_string()).collect(), now, Action::Pick(Pick::PowerMode(now)))
}

/// The setting a pop-up row sends for its `choice` index: the row's action with the choice in place of its own.
/// None when the row's action has no choices or the index is out of range.
pub fn chosen(row: &Action, index: usize) -> Option<Action> {
    match row {
        Action::Location(super::location_page::Edit::Access { app, .. }) => {
            let choice = *super::services::privacy_permissions::LocationAccess::ALL.get(index)?;
            Some(Action::Location(super::location_page::Edit::Access { app: app.clone(), choice }))
        }
        // a value choice among listed values: the pick with the chosen index
        Action::Pick(pick) => pick.with_choice(index).map(Action::Pick),
        // Screen Time's Day or Week switch: segment 1 is the week
        Action::ScreenTime(super::screentime::page::StEdit::Range(_)) => (index < 2).then(|| Action::ScreenTime(super::screentime::page::StEdit::Range(index == 1))),
        Action::Assistant(super::assistant_page::Edit::Backend(_)) => super::assistant::Backend::ALL.get(index).map(|b| Action::Assistant(super::assistant_page::Edit::Backend(*b))),
        _ => None,
    }
}

/// Privacy › App Permissions: one group per media type (camera, microphone, screen recording), each listing the
/// installed apps with a switch. A switch is on only when the person authorized that app for that media type; apps
/// that have not asked yet show off. `rows` come from `services::privacy_permissions::rows()`, in the order of the
/// installed apps, so a switch's index is the app's index.
pub fn permissions_page(rows: &[Row]) -> Spec {
    let mut groups: Vec<Group> = privacy_permissions::MEDIA
        .iter()
        .map(|media| {
            let (header, footer) = match media {
                Media::Video => ("CAMERA", "Apps that may use the camera. A switch is on only when you allow it."),
                Media::Audio => ("MICROPHONE", "Apps that may record from the microphone. A switch is on only when you allow it."),
                Media::Screen => ("SCREEN RECORDING", "Apps that may record the screen or capture other windows."),
            };
            let items: Vec<Item> = if rows.is_empty() {
                vec![Item::new("No apps installed")]
            } else {
                rows.iter()
                    .enumerate()
                    .map(|(i, r)| Item::new(r.name.as_str()).app_icon(r.app_id.as_str()).switch(r.access.can_capture(*media), Action::Toggle(Toggle::Permission(*media, i))))
                    .collect()
            };
            Group::new(items).header(header).footer(footer)
        })
        .collect();
    // Location (wire `location_status`): on only when the person allowed the app while in use; a restricted app shows off.
    let location: Vec<Item> = if rows.is_empty() {
        vec![Item::new("No apps installed")]
    } else {
        rows.iter()
            .enumerate()
            .map(|(i, r)| Item::new(r.name.as_str()).app_icon(r.app_id.as_str()).switch(r.location.is_authorized(), Action::Toggle(Toggle::Location(i))))
            .collect()
    };
    groups.push(Group::new(location).header("LOCATION").footer("Apps that may use your location while you use them. A switch is on only when you allow it."));
    // Contacts (last, so the group indexes above stay the same): on only when the person lets the app read contacts.
    let contacts: Vec<Item> = if rows.is_empty() {
        vec![Item::new("No apps installed")]
    } else {
        rows.iter()
            .enumerate()
            .map(|(i, r)| Item::new(r.name.as_str()).app_icon(r.app_id.as_str()).switch(r.contacts, Action::Toggle(Toggle::Contacts(i))))
            .collect()
    };
    groups.push(Group::new(contacts).header("CONTACTS").footer("Apps that may read your contacts. A switch is on only when you allow it."));
    // USB and HID devices (after Contacts, so the indexes above stay the same): the apps the person allowed to use a device
    // directly. A switch is on while the app holds a grant; turning it off takes every device back. It cannot grant, as a
    // grant names one device and is made on the sheet the app asks with.
    let devices: Vec<Item> = rows
        .iter()
        .enumerate()
        .filter(|(_, r)| r.usb > 0)
        .map(|(i, r)| Item::new(r.name.as_str()).app_icon(r.app_id.as_str()).switch(true, Action::Toggle(Toggle::Usb(i))))
        .collect();
    let devices = if devices.is_empty() { vec![Item::new("No apps use USB or HID devices")] } else { devices };
    groups.push(Group::new(devices).header("USB AND HID DEVICES").footer("Apps you allowed to talk to a device directly. Turn a switch off to take back the devices that app may use."));
    // Calendars (last, so the indexes above stay the same): on while the person lets the app add events to the calendar
    // (an app that asked only to add shows on too). Turning it on gives full access, off denies it.
    let calendars: Vec<Item> = if rows.is_empty() {
        vec![Item::new("No apps installed")]
    } else {
        rows.iter()
            .enumerate()
            .map(|(i, r)| Item::new(r.name.as_str()).app_icon(r.app_id.as_str()).switch(r.calendars, Action::Toggle(Toggle::Calendars(i))))
            .collect()
    };
    groups.push(Group::new(calendars).header("CALENDARS").footer("Apps that may use your calendar. A switch is on only when you allow it."));
    Spec { id: PageId::PrivacyPermissions, title: "App Permissions".into(), large: false, groups }
}

/// The switches under Sounds: Keyboard Haptics only exists with a vibrator.
/// "Auto" or the scale as a percentage ("125%").
pub fn scale_label(s: Option<f32>) -> String {
    s.map_or_else(|| "Auto".to_string(), |s| format!("{:.0}%", s * 100.0))
}

fn sound_switches(o: &Options, haptics: bool) -> Vec<Item> {
    let mut rows = vec![Item::new("Keyboard Clicks").switch(o.keyboard_clicks, Action::Toggle(Toggle::KeyboardClicks))];
    if haptics {
        rows.push(Item::new("Keyboard Haptics").switch(o.keyboard_haptics, Action::Toggle(Toggle::KeyboardHaptics)));
    }
    rows.push(Item::new("Lock Sound").switch(o.lock_sound, Action::Toggle(Toggle::LockSound)));
    rows
}

/// General › Dictionary: the word lists built in (src/shell/words), used by
/// the keyboard's suggestions and Look Up.
fn dictionaries() -> Vec<&'static str> {
    vec!["English", "Íslenska"]
}

/// Desktop & Dock (desktop profile only): the same dock state the bar's Dock menu edits.
pub fn desktop_dock_page(m: &Model<'_>, d: &DeskPrefs) -> Spec {
    let positions = [("Bottom", Position::Bottom), ("Left", Position::Left), ("Right", Position::Right)];
    let position = positions.iter().position(|(_, p)| *p == m.dock.position).unwrap_or(0);
    let minimize = d.minimize_effect.min(MINIMIZE_EFFECTS.len() - 1);
    Spec { id: PageId::HomeScreen, title: "Desktop & Dock".into(), large: false, groups: vec![
        Group::new(vec![Item::slider(Ends::TextSize, (m.dock.size - 32.) / 48., Action::Slider(Slider::DockSize))]).header("SIZE"),
        Group::new(vec![
            Item::new("Magnification").switch(d.magnify, Action::Toggle(Toggle::DockMagnify)),
            Item::slider(Ends::TextSize, d.magnify_size, Action::Slider(Slider::DockMagnify)),
        ]).footer("Icons under the pointer grow. The slider sets how large they get."),
        // value choices are pop-up buttons on their rows
        Group::new(vec![
            Item::new("Position on Screen").popup(positions.iter().map(|(l, _)| l.to_string()).collect(), position, Action::Pick(Pick::DockPosition(position))),
            Item::new("Minimize Windows Using").popup(choice_labels(&MINIMIZE_EFFECTS), minimize, Action::Pick(Pick::MinimizeEffect(minimize))),
        ]),
        Group::new(vec![
            Item::new("Automatically Hide and Show").switch(m.dock.autohide, Action::Toggle(Toggle::DockAutohide)),
            Item::new("Show Suggested and Recent Apps").switch(m.opts.dock_recents, Action::Toggle(Toggle::DockRecents)),
        ]).footer("The dock slides away until you move the pointer to its edge."),
        Group::new(vec![Item::new("Hot Corners").link(PageId::HotCorners)]),
    ] }
}

/// Where a window goes when it is minimized.
pub const MINIMIZE_EFFECTS: [&str; 2] = ["Funnel Effect", "Scale Effect"];
/// What a hot corner does; index 0 is nothing.
pub const CORNER_ACTIONS: [&str; 7] = ["None", "Overview", "Apps Grid", "Show Desktop", "Lock Screen", "Search", "Start Screen Saver"];
pub const CORNER_NAMES: [&str; 4] = ["Top Left", "Top Right", "Bottom Left", "Bottom Right"];
pub const SIDEBAR_ICON_SIZES: [&str; 3] = ["Small", "Medium", "Large"];
/// Nearby Share: who may send to this computer (index is `Options::nearby`).
pub const NEARBY_CHOICES: [&str; 3] = ["Receiving Off", "Contacts Only", "Everyone"];
pub const SCROLL_BAR_MODES: [&str; 3] = ["Automatically Based on Mouse or Trackpad", "When Scrolling", "Always"];

pub const DOCK_MAGNIFY_PREF: &str = "dock_magnify";
pub const DOCK_MAGNIFY_SIZE_PREF: &str = "dock_magnify_size";
pub const DOCK_MINIMIZE_PREF: &str = "dock_minimize_effect";
pub const HOT_CORNER_PREFS: [&str; 4] = ["hot_corner_tl", "hot_corner_tr", "hot_corner_bl", "hot_corner_br"];
pub const OVERVIEW_GROUP_PREF: &str = "overview_group_by_app";
pub const OVERVIEW_SWITCH_PREF: &str = "overview_switch_desktop";
pub const HIGHLIGHT_PREF: &str = "highlight_color";
pub const SIDEBAR_ICON_PREF: &str = "sidebar_icon_size";
pub const SCROLL_BARS_PREF: &str = "show_scroll_bars";

/// The desktop preferences Settings owns that live in `settings.ini` (the dock's position, size and auto-hide
/// stay in `desktop_dock::Preferences`). Other tracks read the same keys; `apply` publishes the appearance ones.
#[derive(Clone, Debug, PartialEq)]
pub struct DeskPrefs {
    pub magnify: bool,
    /// 0..1 across the enlarged size range
    pub magnify_size: f32,
    pub minimize_effect: usize,
    /// per corner (top left, top right, bottom left, bottom right): index into `CORNER_ACTIONS`
    pub corners: [usize; 4],
    pub overview_group: bool,
    pub overview_switch: bool,
    /// General: "Close windows when quitting an application" (`session::CLOSE_ON_QUIT_PREF`)
    pub close_on_quit: bool,
    /// 0 follows the accent colour
    pub highlight: usize,
    pub sidebar_icons: usize,
    pub scroll_bars: usize,
}

impl Default for DeskPrefs {
    fn default() -> Self {
        DeskPrefs { magnify: true, magnify_size: 0.4, minimize_effect: 0, corners: [0; 4], overview_group: true, overview_switch: true, close_on_quit: false, highlight: 0, sidebar_icons: 1, scroll_bars: 0 }
    }
}

impl DeskPrefs {
    pub fn load(prefs: &super::prefs::Prefs) -> Self {
        let d = DeskPrefs::default();
        let flag = |key: &str, default: bool| prefs.get(key).map_or(default, |v| v == "1");
        let index = |key: &str, default: usize, len: usize| prefs.get_parsed::<usize>(key).filter(|&v| v < len).unwrap_or(default);
        let mut corners = d.corners;
        for (c, key) in corners.iter_mut().zip(HOT_CORNER_PREFS) {
            *c = index(key, 0, CORNER_ACTIONS.len());
        }
        DeskPrefs {
            magnify: flag(DOCK_MAGNIFY_PREF, d.magnify),
            magnify_size: prefs.get_parsed::<f32>(DOCK_MAGNIFY_SIZE_PREF).filter(|v| v.is_finite()).map_or(d.magnify_size, |v| v.clamp(0., 1.)),
            minimize_effect: index(DOCK_MINIMIZE_PREF, 0, MINIMIZE_EFFECTS.len()),
            corners,
            overview_group: flag(OVERVIEW_GROUP_PREF, d.overview_group),
            overview_switch: flag(OVERVIEW_SWITCH_PREF, d.overview_switch),
            close_on_quit: flag(super::session::CLOSE_ON_QUIT_PREF, d.close_on_quit),
            highlight: index(HIGHLIGHT_PREF, 0, bifrost_render::appearance::Accent::ALL.len()),
            sidebar_icons: index(SIDEBAR_ICON_PREF, 1, SIDEBAR_ICON_SIZES.len()),
            scroll_bars: index(SCROLL_BARS_PREF, 0, SCROLL_BAR_MODES.len()),
        }
    }

    /// Publishes the appearance preferences (`bifrost_render::ui::highlight_now` and friends).
    pub fn apply(&self) {
        bifrost_render::ui::set_highlight(self.highlight as u32);
        bifrost_render::ui::set_sidebar_icon_size(self.sidebar_icons as u32);
        bifrost_render::ui::set_scroll_bars(self.scroll_bars as u32);
    }
}

/// Appearance > Icon & widget style, in the order of `icon_doc::Variant` (Default 0, Dark 1, Tinted 2).
pub const ICON_STYLES: [&str; 3] = ["Default", "Dark", "Tinted"];

/// The row that is checked: the variant the icon renderer draws with.
fn icon_style_index() -> usize {
    bifrost_render::icon_doc::current_style().0 as usize
}

/// Appearance: the scheme, accent and highlight colour, sidebar icon size, scroll bars and transparency.
pub fn appearance_page(m: &Model<'_>, d: &DeskPrefs) -> Spec {
    use bifrost_render::appearance::{accent, mode, mode_label, Accent, Mode};
    let now = mode();
    let highlights = std::iter::once(Item::new("Accent Colour").check(d.highlight == 0, Action::Pick(Pick::Highlight(0))))
        .chain(Accent::ALL.into_iter().enumerate().skip(1).map(|(i, a)| Item::new(a.label()).glyph(Glyph::Swatch(i as u8)).check(d.highlight == i, Action::Pick(Pick::Highlight(i)))))
        .collect();
    Spec { id: PageId::Appearance, title: "Appearance".into(), large: false, groups: vec![
        Group::new([Mode::Light, Mode::Dark, Mode::Auto].into_iter().enumerate().map(|(i, md)| Item::new(mode_label(md)).check(md == now, Action::Pick(Pick::Appearance(i)))).collect())
            .header("APPEARANCE")
            .footer("Light and Dark change the look of the whole desktop and fade between them. Auto is light from 07:00 to 19:00 and dark at night."),
        Group::new(Accent::ALL.into_iter().enumerate().map(|(i, a)| Item::new(a.label()).glyph(Glyph::Swatch(i as u8)).check(a == accent(), Action::Pick(Pick::Accent(i)))).collect())
            .header("ACCENT COLOUR")
            .footer("The colour of buttons, links, switches, sliders and the selected sidebar row. Multicolour uses blue for controls."),
        Group::new(highlights).header("HIGHLIGHT COLOUR").footer("The colour of selected text."),
        Group::new(SIDEBAR_ICON_SIZES.iter().enumerate().map(|(i, l)| Item::new(*l).check(i == d.sidebar_icons, Action::Pick(Pick::SidebarIcons(i)))).collect())
            .header("SIDEBAR ICON SIZE"),
        Group::new(SCROLL_BAR_MODES.iter().enumerate().map(|(i, l)| Item::new(*l).check(i == d.scroll_bars, Action::Pick(Pick::ScrollBars(i)))).collect())
            .header("SHOW SCROLL BARS"),
        Group::new(ICON_STYLES.iter().enumerate().map(|(i, l)| Item::new(*l).check(i == icon_style_index(), Action::Pick(Pick::IconStyle(i)))).collect())
            .header("ICON & WIDGET STYLE")
            .footer("Dark draws app icons on a dark tile. Tinted draws them in the accent colour over grey."),
        Group::new(vec![
            Item::slider(Ends::Glass, bifrost_render::glass::clarity(), Action::Slider(Slider::LiquidGlass)),
            reduce_transparency_item(m.a11y.reduce_transparency, m.a11y.increase_contrast),
        ]).header("TRANSPARENCY")
            .footer("Move toward the filled pane for more opaque menus, the dock and Search. Reduce Transparency replaces every translucent material with a solid one."),
    ] }
}

/// General > Fonts: the font each role draws with; a row opens that role's choices.
pub fn fonts_page(m: &Model<'_>) -> Spec {
    let f = m.fonts;
    let rows = FontRole::ALL
        .into_iter()
        .map(|role| {
            let current = match &f.overrides[role.index()] {
                Some(path) => f.names[role.index()].clone().unwrap_or_else(|| path.rsplit('/').next().unwrap_or(path).to_string()),
                None => f.defaults[role.index()].clone(),
            };
            Item::new(role.label()).value(current).link(PageId::FontPick(role))
        })
        .collect();
    Spec { id: PageId::Fonts, title: "Fonts".into(), large: false, groups: vec![
        Group::new(rows).footer("The system font sets labels, menus and window titles. The monospaced font sets terminals and code. Changes take effect when BifrOSt restarts."),
        Group::new(vec![Item::new("Restart BifrOSt").tint(Tint::Blue).action(Action::Run(Command::Restart))]),
    ] }
}

/// General > Fonts > a role: its default, then the library fonts that suit it (fixed pitch for Monospaced).
pub fn font_pick_page(m: &Model<'_>, role: FontRole) -> Spec {
    let f = m.fonts;
    let current = f.overrides[role.index()].as_deref();
    let default = if f.defaults[role.index()].is_empty() { "Default".to_string() } else { format!("{} (Default)", f.defaults[role.index()]) };
    let library: Vec<Item> = f
        .library
        .iter()
        .filter(|font| font.mono == (role == FontRole::Mono))
        .map(|font| Item::new(font.family.as_str()).check(current == Some(font.path.as_str()), Action::Font(role, Some(font.path.clone()))))
        .collect();
    let library = if library.is_empty() { vec![Item::new("No Other Fonts").tint(Tint::Dim)] } else { library };
    Spec { id: PageId::FontPick(role), title: role.label().into(), large: false, groups: vec![
        Group::new(vec![Item::new(default).check(current.is_none(), Action::Font(role, None))]),
        Group::new(library).header("FONT LIBRARY").footer("Open-source fonts that come with BifrOSt, and fonts that apps install."),
    ] }
}

/// Desktop & Dock > Hot Corners: one group per corner.
/// Settings > Screen Saver: the saver choice, Preview, the chosen saver's own options (its configuration sheet, with the
/// values saved for it) and Start After. A switch is an on or off option; a choice option is a list of checks.
pub fn screen_saver_page(o: &Options, sheet: &bifrost_ui::screensaver::ConfigureSheet) -> Spec {
    let mut groups = vec![
        Group::new(vec![Item::new("Screen Saver").popup(choice_labels(&super::screensaver::SAVERS), o.screensaver_module.min(super::screensaver::SAVERS.len() - 1), Action::Pick(Pick::ScreenSaverModule(o.screensaver_module)))]),
        Group::new(vec![Item::new("Preview").tint(Tint::Blue).action(Action::Pick(Pick::ScreenSaverPreview))])
            .footer(by_form("Shows the screen saver full screen. Touch the screen or press a key to return.", "Shows the screen saver full screen. Click or press a key to return.")),
    ];
    for (n, key) in sheet.keys().into_iter().enumerate() {
        let title = super::screensaver::option_title(key);
        if sheet.is_bool(key) {
            groups.push(Group::new(vec![Item::new(&title).switch(sheet.bool_or(key, false), Action::Toggle(Toggle::SaverOption(n)))]));
        } else if let Some(choices) = sheet.choices(key) {
            let current = sheet.choice_of(key).unwrap_or("");
            let now = choices.iter().position(|name| name == current).unwrap_or(0);
            let titles: Vec<String> = choices.iter().map(|name| super::screensaver::choice_title(name)).collect();
            groups.push(Group::new(vec![Item::new(&title).popup(titles, now, Action::Pick(Pick::ScreenSaverOption(n, now)))]));
        }
    }
    groups.push(
        Group::new(vec![{
            let now = super::screensaver::START_AFTER.iter().position(|(_, min)| *min == o.screensaver_minutes).unwrap_or(0);
            Item::new("Start After").popup(super::screensaver::START_AFTER.iter().map(|(l, _)| l.to_string()).collect(), now, Action::Pick(Pick::ScreenSaverStart(now)))
        }])
            .footer("The screen saver starts once the computer has been idle for this long. The display still turns off at the Auto-Lock time. A hot corner can start it at once (Desktop & Dock > Hot Corners)."),
    );
    Spec { id: PageId::ScreenSaver, title: "Screen Saver".into(), large: false, groups }
}

pub fn hot_corners_page(d: &DeskPrefs) -> Spec {
    let groups = CORNER_NAMES.iter().enumerate().map(|(c, name)| {
        Group::new(CORNER_ACTIONS.iter().enumerate().map(|(a, l)| Item::new(*l).check(d.corners[c] == a, Action::Pick(Pick::HotCorner(c, a)))).collect()).header(&name.to_uppercase())
    }).collect();
    Spec { id: PageId::HotCorners, title: "Hot Corners".into(), large: false, groups }
}

/// Overview: grouping and desktop switching.
/// General > "Close windows when quitting an application": off keeps an app's windows for its next launch, on starts
/// it fresh.
pub fn restoration_group(d: &DeskPrefs) -> Group {
    Group::new(vec![Item::new("Close windows when quitting an application").switch(d.close_on_quit, Action::Toggle(Toggle::CloseOnQuit))])
        .footer("When on, an application starts without its old windows when you open it again. Windows that were open when you restarted or logged out come back if you leave \u{201c}Reopen windows when logging back in\u{201d} checked.")
}

pub fn overview_page(d: &DeskPrefs) -> Spec {
    Spec { id: PageId::Overview, title: "Overview".into(), large: false, groups: vec![
        Group::new(vec![Item::new("Group Windows by Application").switch(d.overview_group, Action::Toggle(Toggle::OverviewGroup))])
            .footer("Overview shows the windows of one application stacked together instead of side by side."),
        Group::new(vec![Item::new("When Switching to an Application, Switch to a Desktop with Open Windows for the Application").switch(d.overview_switch, Action::Toggle(Toggle::OverviewSwitch))]),
    ] }
}

/// The shortcuts the shell answers to, as (heading, [(action, keys)]).
pub const SHORTCUTS: [(&str, &[(&str, &str)]); 4] = [
    ("DESKTOP", &[("Back", "Escape")]),
    ("WINDOWS", &[("Quit Application", "Cmd+Q")]),
    ("EDITING", &[("Undo", "Cmd+Z"), ("Redo", "Cmd+Shift+Z"), ("Cut", "Cmd+X"), ("Copy", "Cmd+C"), ("Paste", "Cmd+V"), ("Select All", "Cmd+A")]),
    ("FIND AND TEXT SIZE", &[("Find", "Cmd+F"), ("Find Next", "Cmd+G"), ("Find Previous", "Cmd+Shift+G"), ("Actual Size", "Cmd+0"), ("Bigger", "Cmd+="), ("Smaller", "Cmd+-")]),
];

/// The headers of the Modifier Keys groups, in `kb_input::REMAPPABLE` order.
const MODIFIER_HEADERS: [&str; 4] = ["CAPS LOCK KEY", "CONTROL KEY", "OPTION KEY", "COMMAND KEY"];

/// Keyboard Shortcuts: the shell's own shortcuts as switches (they can be turned off to give the keys back to the
/// app in front), then the reference lists of the other shortcuts.
pub fn shortcuts_page(o: &Options) -> Spec {
    let mut groups = vec![Group::new(
        super::kb_input::BINDINGS
            .iter()
            .enumerate()
            // a lone Cmd goes Home on a tablet's keyboard; on a desktop it does nothing, so the row is left out (its
            // switch keeps its place in `BINDINGS`, so the other rows still address their own bits)
            .filter(|(_, (action, _))| *action != "Home" || !form().is_desktop())
            .map(|(i, (action, keys))| Item::new(*action).value(*keys).switch(o.shortcuts_off & (1 << i) == 0, Action::Toggle(Toggle::Shortcut(i))))
            .collect(),
    )
    .header("SYSTEM")
    .footer("A switched-off shortcut does nothing, and its keys go to the app in front.")];
    groups.extend(SHORTCUTS.iter().map(|(h, rows)| Group::new(rows.iter().map(|(a, k)| Item::new(*a).value(*k)).collect()).header(*h)));
    Spec { id: PageId::Shortcuts, title: "Keyboard Shortcuts".into(), large: false, groups }
}

/// Desktop controls describe the active physical display, independent of the Settings window.
pub fn desktop_display(m: &Model<'_>) -> Spec {
    let page = |title: &str, large, groups| Spec { id: PageId::Display, title: title.into(), large, groups };
    let mut display = vec![];
    if m.info.display_mode.is_some() {
        display.push(Item::new("Resolution").value(resolution_label(m.resolution)).link(PageId::Resolution));
        display.push(Item::new("Refresh Rate").value(m.refresh.label()).link(PageId::RefreshRate));
    }
    display.push(Item::new("Appearance").value(bifrost_render::appearance::mode_label(bifrost_render::appearance::mode())).link(PageId::Appearance));
    display.push(Item::new("Interface Scale").value(scale_label(crate::layout::user_scale())).link(PageId::InterfaceScale));
    display.push(Item::new("Graphics").link(PageId::Graphics));
    let mut groups = vec![Group::new(display).header("CONNECTED DISPLAY")
        .footer("Native is the panel's own resolution unless another is chosen. The highest refresh rate is used unless another is chosen. Interface Scale changes the size of windows, menus and controls.")];
    if m.info.brightness_supported {
        let mut rows = vec![Item::slider(Ends::Brightness, m.brightness, Action::Slider(Slider::Brightness))];
        if m.info.ambient_light_supported {
            rows.push(Item::new("Automatically Adjust Brightness").switch(m.opts.auto_brightness, Action::Toggle(Toggle::AutoBrightness)));
        }
        groups.push(Group::new(rows).header("BRIGHTNESS"));
    }
    groups.push(Group::new(vec![Item::new("Night Shift").value(super::night_shift::published().label()).link(PageId::NightShift)]));
    groups.push(Group::new(vec![Item::slider(Ends::Glass, bifrost_render::glass::clarity(), Action::Slider(Slider::LiquidGlass))]).header("LIQUID GLASS")
        .footer("Clear shows more of the desktop through the menu bar, menus, the dock and Search. Tinted makes them more opaque."));
    groups.push(Group::new(vec![
        Item::slider(Ends::TextSize, m.opts.text_size_step() as f32 / (TEXT_SIZES.len() - 1) as f32, Action::Slider(Slider::TextSize)),
        Item::new("Bold Text").switch(m.opts.bold_text, Action::Toggle(Toggle::BoldText)),
    ]).header("TEXT SIZE"));
    groups.push(Group::new(vec![Item::new("Lock After Inactivity").value(m.opts.auto_lock_label()).link(PageId::AutoLock)]));
    page("Displays", false, groups)
}

/// Displays > Refresh Rate: Highest Available (the default) and each rate the panel offers at its size, the chosen one checked.
pub fn refresh_rate_page(m: &Model<'_>) -> Spec {
    let mut rows = vec![Item::new(RefreshChoice::Max.label()).check(m.refresh == RefreshChoice::Max, Action::Pick(Pick::RefreshRate(0)))];
    rows.extend(m.info.display_rates.iter().enumerate().map(|(i, &hz)| {
        let rate = RefreshChoice::Hz(hz);
        Item::new(rate_label(hz)).check(m.refresh.same(rate), Action::Pick(Pick::RefreshRate(i + 1)))
    }));
    Spec { id: PageId::RefreshRate, title: "Refresh Rate".into(), large: false, groups: vec![
        Group::new(rows).footer("The highest refresh rate is used unless another is chosen. Rates are the ones the display offers at its resolution."),
    ] }
}

/// Displays > Resolution: Native (the default) and each size the panel offers, the chosen one checked.
pub fn resolution_page(m: &Model<'_>) -> Spec {
    let mut rows = vec![Item::new(resolution_label(None)).check(m.resolution.is_none(), Action::Pick(Pick::Resolution(0)))];
    rows.extend(m.info.display_sizes.iter().enumerate().map(|(i, &(w, h))| {
        Item::new(resolution_label(Some((w, h)))).check(m.resolution == Some((w, h)), Action::Pick(Pick::Resolution(i + 1)))
    }));
    Spec { id: PageId::Resolution, title: "Resolution".into(), large: false, groups: vec![
        Group::new(rows).footer("Native is the panel's own size. Choosing another size changes the whole screen until it is changed back."),
    ] }
}

#[cfg(test)]
mod tests {
    use super::super::desktop_dock::Preferences as DockPrefs;
    use super::*;

    #[test]
    fn a_desktop_without_a_battery_shows_none_not_a_made_up_level() {
        let mut info = Info { battery: 100, ..Info::default() };
        assert_eq!(battery_level(&info), "100%", "a bifrost keeps its level");
        info.desktop_network = Some(Default::default());
        info.battery_present = false;
        assert_eq!(battery_level(&info), "None");
        info.battery_present = true;
        info.battery = 64;
        assert_eq!(battery_level(&info), "64%", "a desktop with a battery shows it");
    }
    use crate::shell::{Network, SavedNetwork};

    #[test]
    fn the_speech_voice_row_names_the_saved_voice_and_falls_back_to_the_default() {
        assert_eq!(speech_voice_index(""), 0, "the engine's default voice");
        assert_eq!(super::super::accessibility::SPEECH_VOICES[speech_voice_index("is")].0, "Icelandic");
        assert_eq!(speech_voice_index("no-such-voice"), 0, "an unknown saved voice shows as the default");
    }

    fn info() -> Info {
        Info {
            battery: 63,
            charging: true,
            wifi: "WIFI-5G".into(),
            wifi_on: true,
            current_ssid: Some("WIFI-5G".into()),
            networks: vec![Network { ssid: "WIFI-5G".into(), bars: 3, secure: true, saved: true }, Network { ssid: "Cafe".into(), bars: 1, secure: false, saved: false }],
            ip: "192.168.1.40".into(),
            router: "192.168.1.1".into(),
            mac: "AC:E4:B5:35:20:F0".into(),
            model: "Midget".into(),
            kernel: "4.14.186".into(),
            memory: "3.6 GB".into(),
            cpu: "Unisoc T618, 8 cores".into(),
            storage_total: 32_000_000_000,
            storage_free: 17_900_000_000,
            uptime_min: 1_500,
            name: "Midget".into(),
            serial: "R9XR40ABC".into(),
            timezone: "Atlantic/Reykjavik".into(),
            hour: 14,
            min: 5,
            ..Info::default()
        }
    }

    #[test]
    fn fonts_pages_show_each_role_and_offer_the_library_by_kind() {
        let (info, opts) = (info(), Options::default());
        let lib = |family: &str, path: &str, mono: bool| Installed { family: family.into(), path: path.into(), mono, variable: true };
        let fonts = FontsView {
            defaults: ["Inter".into(), "JetBrains Mono".into()],
            overrides: [None, Some("/lib/GeistMono.ttf".into())],
            names: [None, Some("Geist Mono".into())],
            library: vec![lib("Geist", "/lib/Geist.ttf", false), lib("Geist Mono", "/lib/GeistMono.ttf", true)],
        };
        let m = Model { fonts: &fonts, ..model(&info, &opts) };
        let root = spec(PageId::Fonts, &m);
        let rows: Vec<_> = root.groups[0].items.iter().map(|i| (i.label.as_str(), i.action.clone())).collect();
        assert_eq!(rows, [("System Font", Action::Push(PageId::FontPick(FontRole::System))), ("Monospaced Font", Action::Push(PageId::FontPick(FontRole::Mono)))]);
        assert_eq!(root.groups[0].items[0].value, "Inter", "no override: the default's family");
        assert_eq!(root.groups[1].items[0].action, Action::Run(Command::Restart), "overrides apply after a restart");

        let system = spec(PageId::FontPick(FontRole::System), &m);
        assert_eq!(system.groups[0].items[0].label, "Inter (Default)");
        assert_eq!(system.groups[0].items[0].accessory, Accessory::Check(true));
        let offered: Vec<_> = system.groups[1].items.iter().map(|i| i.label.as_str()).collect();
        assert_eq!(offered, ["Geist"], "proportional fonts only");
        assert_eq!(system.groups[1].items[0].action, Action::Font(FontRole::System, Some("/lib/Geist.ttf".into())));

        let mono = spec(PageId::FontPick(FontRole::Mono), &m);
        assert_eq!(mono.groups[0].items[0].accessory, Accessory::Check(false));
        assert_eq!((mono.groups[1].items[0].label.as_str(), &mono.groups[1].items[0].accessory), ("Geist Mono", &Accessory::Check(true)));
        assert_eq!(mono.groups[0].items[0].action, Action::Font(FontRole::Mono, None), "Default clears the override");
    }

    // a static: the settings hold a Vec, so a const would be a temporary here
    static A11Y_DEFAULT: AccessibilitySettings = AccessibilitySettings::DEFAULT;

    fn model<'a>(info: &'a Info, opts: &'a Options) -> Model<'a> {
        Model { info, opts, a11y: &A11Y_DEFAULT, airplane: false, haptics: false, wifi_on: true, brightness: 0.5, wallpaper: "Lake Day".into(), detail: "WIFI-5G", now: 1_000_000, update_badge: false, dock: DockPrefs::default(), defaults: &[], login_items: &[], login_pending: &[], firewall: &[], ext: &crate::shell::login_items_page::NO_EXTENSIONS, fonts: &NO_FONTS, refresh: RefreshChoice::Max, resolution: None, sound: &crate::shell::sound_page::NO_SOUND, sharing: None, printers: None, users: &NO_USERS, account: "" }
    }

    /// The row labelled `label` anywhere on the spec.
    fn popup_row<'a>(spec: &'a Spec, label: &str) -> &'a Item {
        spec.groups.iter().flat_map(|g| &g.items).find(|i| i.label == label).unwrap_or_else(|| panic!("no row {label:?}"))
    }

    #[test]
    fn value_choices_are_pop_up_rows_that_send_the_same_picks() {
        let (info, opts) = (info(), Options::default());
        let m = model(&info, &opts);
        // Desktop & Dock: position and minimise effect
        let dock = desktop_dock_page(&m, &DeskPrefs::default());
        let position = popup_row(&dock, "Position on Screen");
        assert_eq!(position.accessory, Accessory::Popup);
        assert_eq!(position.popup.as_ref().map(|p| (p.choices.clone(), p.selected)), Some((vec!["Bottom".to_string(), "Left".into(), "Right".into()], 0)));
        assert_eq!(chosen(&position.action, 2), Some(Action::Pick(Pick::DockPosition(2))), "the menu's choice replaces the index");
        let minimize = popup_row(&dock, "Minimize Windows Using");
        assert_eq!(minimize.popup.as_ref().map(|p| p.choices.len()), Some(MINIMIZE_EFFECTS.len()));
        assert_eq!(chosen(&minimize.action, 1), Some(Action::Pick(Pick::MinimizeEffect(1))));
        assert!(dock.groups.iter().all(|g| g.items.iter().all(|i| !matches!(i.accessory, Accessory::Check(_)))), "no checklist left on the page");
        // General: Game Mode
        let general = spec(PageId::General, &m);
        let game = popup_row(&general, "Game Mode");
        assert_eq!(game.accessory, Accessory::Popup);
        assert_eq!(chosen(&game.action, 1), Some(Action::Pick(Pick::GameMode(1))));
        // Trackpad: secondary click
        let trackpad = spec(PageId::Trackpad, &m);
        let secondary = popup_row(&trackpad, "Secondary Click");
        assert_eq!(secondary.popup.as_ref().map(|p| p.choices.len()), Some(TRACKPAD_SECONDARY.len()));
        assert_eq!(chosen(&secondary.action, 1), Some(Action::Pick(Pick::TrackpadSecondary(1))));
        // Screen Saver: kind and Start After; a choice option of the saver keeps its option number
        let saver = screen_saver_page(&opts, &Default::default());
        assert_eq!(popup_row(&saver, "Screen Saver").popup.as_ref().map(|p| p.choices.clone()), Some(super::super::screensaver::SAVERS.iter().map(|s| s.to_string()).collect()));
        let start = popup_row(&saver, "Start After");
        assert_eq!(chosen(&start.action, 3), Some(Action::Pick(Pick::ScreenSaverStart(3))));
        assert_eq!(chosen(&Action::Pick(Pick::ScreenSaverOption(2, 0)), 1), Some(Action::Pick(Pick::ScreenSaverOption(2, 1))));
        // a pick that is not a value choice has no menu
        assert_eq!(chosen(&Action::Pick(Pick::ScreenSaverPreview), 1), None);
        assert_eq!(chosen(&Action::Pick(Pick::Remap(0, 0)), 1), None);
    }

    #[test]
    fn the_screen_time_day_and_week_switch_is_a_segmented_control() {
        use super::super::screentime::page::StEdit;
        let row = Item::new("Range").segmented(vec!["Day".into(), "Week".into()], 1, Action::ScreenTime(StEdit::Range(true)));
        assert_eq!(chosen(&row.action, 0), Some(Action::ScreenTime(StEdit::Range(false))));
        assert_eq!(chosen(&row.action, 1), Some(Action::ScreenTime(StEdit::Range(true))));
        assert_eq!(chosen(&row.action, 2), None);
    }

    #[test]
    fn app_permissions_lists_each_media_type_with_one_switch_per_app() {
        use bifrost_ui::capture_access::CaptureAccess;
        use bifrost_ui::location_auth::AuthorizationStatus;
        let rows = vec![Row { app_id: "is.example.a".into(), name: "Alpha".into(), access: CaptureAccess::default(), location: AuthorizationStatus::AuthorizedWhenInUse, contacts: false, calendars: true, usb: 2 }];
        let s = permissions_page(&rows);
        assert_eq!(s.groups.len(), 7, "camera, microphone, screen recording, location, contacts, devices, calendars");
        assert_eq!(s.groups[6].header.as_deref(), Some("CALENDARS"));
        assert_eq!(s.groups[6].items[0].accessory, Accessory::Switch(true), "an app the person let use the calendar is on");
        assert_eq!(s.groups[6].items[0].action, Action::Toggle(Toggle::Calendars(0)));
        assert_eq!(s.groups[5].header.as_deref(), Some("USB AND HID DEVICES"));
        assert_eq!(s.groups[5].items[0].action, Action::Toggle(Toggle::Usb(0)), "an app that holds device grants is listed");
        assert_eq!(permissions_page(&[]).groups[5].items[0].label, "No apps use USB or HID devices");
        assert_eq!(s.groups[3].header.as_deref(), Some("LOCATION"));
        assert_eq!(s.groups[3].items[0].accessory, Accessory::Switch(true), "an app the person allowed while in use is on");
        assert_eq!(s.groups[3].items[0].action, Action::Toggle(Toggle::Location(0)));
        assert_eq!(s.groups[0].header.as_deref(), Some("CAMERA"));
        let row = &s.groups[0].items[0];
        assert_eq!(row.label, "Alpha");
        assert_eq!(row.accessory, Accessory::Switch(false), "an app that has not asked is off");
        assert_eq!(row.action, Action::Toggle(Toggle::Permission(Media::Video, 0)));
        assert_eq!(s.groups[2].items[0].action, Action::Toggle(Toggle::Permission(Media::Screen, 0)));
        let empty = permissions_page(&[]);
        assert_eq!(empty.groups[1].items[0].label, "No apps installed");
        assert_eq!(empty.groups[1].items[0].action, Action::None);
    }

    #[test]
    fn the_settings_map_is_generated_from_the_page_specs() {
        let (info, opts) = (info(), Options::default());
        let mut map = bifrost_ui::ax::Map::new();
        crate::shell::axmap::settings(&mut map, &|id| spec(id, &model(&info, &opts)));
        assert!(map.len() > 100, "{} elements", map.len());
        let su = map.get("os/settings/general/software-update").expect("General > Software Update is addressable");
        assert_eq!((su.role.as_str(), su.handle.as_str()), ("page", "General>Software Update"));
        assert!(map.get("os/settings/general").is_some_and(|e| e.role == "page"));
        let mut addresses: Vec<_> = map.elements().iter().map(|e| e.address.as_str()).collect();
        let n = addresses.len();
        addresses.sort();
        addresses.dedup();
        assert_eq!(addresses.len(), n, "addresses are unique");
        assert!(map.elements().iter().any(|e| e.role == "toggle" && (e.value == "on" || e.value == "off")), "switches carry their state");
        eprintln!("{}", map.listing("os/settings/general").lines().take(14).collect::<Vec<_>>().join("\n"));
    }

    #[test]
    fn about_values_are_formatted_like_mobile() {
        assert_eq!(gb(117_870_000_000), "118 GB");
        assert_eq!(gb(17_900_000_000), "17.9 GB");
        assert_eq!(gb(0), "0.0 GB");
        assert_eq!(uptime(7), "7m");
        assert_eq!(uptime(127), "2h 7m");
        assert_eq!(uptime(1_500), "1d 1h 0m");
        let a = about(&info());
        assert_eq!(a[0][0], ("Name", "Midget".into()));
        assert_eq!(a[0][2], ("Model", "Midget".into()));
        assert_eq!(a[1][0], ("Capacity", "32.0 GB".into()));
        assert_eq!(a[2][0], ("Wi-Fi Address", "AC:E4:B5:35:20:F0".into()));
        assert_eq!(a[2][3], ("Uptime", "1d 1h 0m".into()));
        let blank = about(&Info::default());
        assert_eq!(blank[0][0].1, "—", "missing values show a dash");
        assert_eq!(blank[2][1].1, "—");
    }

    #[test]
    fn the_menu_bar_page_has_one_switch_per_control_and_follows_the_saved_choice() {
        use crate::shell::options::MenuExtra;
        let info = info();
        let mut opts = Options::default();
        let s = spec(PageId::MenuBar, &model(&info, &opts));
        let labels: Vec<&str> = s.groups[0].items.iter().map(|i| i.label.as_str()).collect();
        assert_eq!(labels, ["CPU Usage", "Memory Usage", "Battery", "Wi-Fi and Network", "Control Center"]);
        assert!(s.groups[0].items.iter().all(|i| i.accessory == Accessory::Switch(true)), "every control shows by default");
        assert_eq!(s.groups[0].items[2].action, Action::Toggle(Toggle::MenuExtra(MenuExtra::Battery)));
        opts.menu_extras = MenuExtra::Battery.bit() | MenuExtra::ControlCenter.bit();
        let s = spec(PageId::MenuBar, &model(&info, &opts));
        let on: Vec<bool> = s.groups[0].items.iter().map(|i| i.accessory == Accessory::Switch(true)).collect();
        assert_eq!(on, [false, false, true, false, true]);
    }

    #[test]
    fn the_control_centre_page_lists_the_controls_it_holds() {
        let (info, opts) = (info(), Options::default());
        let s = spec(PageId::ControlCentre, &model(&info, &opts));
        let labels: Vec<&str> = s.groups[0].items.iter().map(|i| i.label.as_str()).collect();
        assert_eq!(labels, ["Brightness", "Volume", "Wi-Fi", "Screen Lock", "Focus"]);
        assert_eq!(s.groups[0].items[4].value, "Off");
    }

    #[test]
    fn the_vpn_page_lists_the_held_connections_and_a_tap_flips_one() {
        use crate::shell::desktop_network::{Snapshot, VpnConnection};
        let (mut info, opts) = (info(), Options::default());
        let empty = spec(PageId::Vpn, &model(&info, &opts));
        assert_eq!(empty.groups[0].items[0].label, "No VPN Configurations");
        info.desktop_network = Some(Snapshot {
            vpns: vec![
                VpnConnection { name: "Office".into(), kind: "vpn".into(), active: false },
                VpnConnection { name: "wg0".into(), kind: "wireguard".into(), active: true },
            ],
            ..Snapshot::default()
        });
        let s = spec(PageId::Vpn, &model(&info, &opts));
        let items = &s.groups[0].items;
        assert_eq!((items[0].label.as_str(), items[0].value.as_str()), ("Office", "Not Connected"));
        assert_eq!((items[1].label.as_str(), items[1].value.as_str()), ("wg0", "Connected"));
        assert_eq!(items[0].action, Action::Vpn { name: "Office".into(), up: true }, "a tap on a disconnected VPN connects it");
        assert_eq!(items[1].action, Action::Vpn { name: "wg0".into(), up: false }, "a tap on a connected one disconnects it");
    }

    #[test]
    fn root_lists_the_bifrost_sections_with_live_values() {
        let (info, opts) = (info(), Options::default());
        let s = spec(PageId::Root, &model(&info, &opts));
        assert!(s.large);
        let labels: Vec<&str> = s.groups.iter().flat_map(|g| g.items.iter().map(|i| i.label.as_str())).collect();
        assert_eq!(labels[..4], ["BifrOSt Account", "Airplane Mode", "Wi-Fi", "Bluetooth"]);
        assert_eq!(labels[4..8], ["Notifications", "Sounds", "Focus", "Screen Time"]);
        assert_eq!(labels[8..], ["General", "Control Center", "Menu Bar", "Display & Brightness", "Home Screen & Dock", "Overview", "Sharing", "Print & Scan", "Keyboard Shortcuts", "Assistant", "Trackpad", "Mouse", "Users & Groups", "Accessibility", "Wallpaper", "Screen Saver", "Battery", "Privacy", "Search", "Lock Screen", "Passcode", "Passwords", "Internet Accounts", "Developer"]);
        assert_eq!(s.groups[1].items[1].value, "WIFI-5G");
        // Sharing, Trackpad, Mouse and Users & Groups sit before Wallpaper
        assert_eq!(s.groups[3].items[14].value, "Lake Day");
        assert_eq!(s.groups[3].items[15].value, "Never", "the screen saver is off by default");
        assert_eq!(s.groups[3].items[16].value, "63%");
        assert_eq!(s.groups[1].items[0].accessory, Accessory::Switch(false));
        assert_eq!(s.groups[2].items[0].action, Action::Push(PageId::Notifications));
        let off = Model { wifi_on: false, airplane: true, ..model(&info, &opts) };
        let s = spec(PageId::Root, &off);
        assert_eq!(s.groups[1].items[1].value, "Off");
        assert_eq!(s.groups[1].items[0].accessory, Accessory::Switch(true));
    }

    fn desktop_info(battery: bool, wifi: bool) -> Info {
        let mut info = Info { desktop_network: Some(Default::default()), battery_present: battery, ..Info::default() };
        if let Some(n) = info.desktop_network.as_mut() {
            n.wifi_available = wifi;
        }
        info
    }

    fn root_labels(info: &Info, form: bifrost_ui::form_factor::FormFactor) -> Vec<String> {
        set_test_form(form);
        let opts = Options::default();
        let s = spec(PageId::Root, &model(info, &opts));
        set_test_form(bifrost_ui::form_factor::FormFactor::Tablet);
        s.groups.iter().flat_map(|g| g.items.iter().map(|i| i.label.clone())).collect()
    }

    #[test]
    fn a_desktop_without_a_battery_has_no_battery_pane_but_offers_energy() {
        use bifrost_ui::form_factor::FormFactor::Desktop;
        let labels = root_labels(&desktop_info(false, false), Desktop);
        assert!(!labels.iter().any(|l| l == "Battery"), "no Battery pane: {labels:?}");
        assert!(labels.iter().any(|l| l == "Energy"), "sleep and wake settings stay, as Energy");
        assert!(root_labels(&desktop_info(true, false), Desktop).iter().any(|l| l == "Battery"), "a desktop with a battery keeps it");
        assert!(!root_labels(&desktop_info(true, false), Desktop).iter().any(|l| l == "Energy"));
        assert!(has_battery(&Info::default()), "a machine that is not a desktop always has the pane");
        // the power modes moved with it: Energy carries Low Power Mode and the Power Mode pop-up
        let (info, opts) = (desktop_info(false, false), Options::default());
        let energy = spec(PageId::Energy, &model(&info, &opts));
        let items: Vec<&str> = energy.groups.iter().flat_map(|g| g.items.iter().map(|i| i.label.as_str())).collect();
        assert!(items.contains(&"Low Power Mode"), "Energy has Low Power Mode: {items:?}");
        let mode = popup_row(&energy, "Power Mode");
        assert_eq!(mode.accessory, Accessory::Popup, "the power modes are one pop-up row");
        assert_eq!(mode.popup.as_ref().map(|p| p.choices.clone()), Some(vec!["Automatic".to_string(), "Performance".into(), "Balanced".into(), "Battery Saver".into()]));
        assert_eq!(chosen(&mode.action, 3), Some(Action::Pick(Pick::PowerMode(3))), "the menu's choice replaces the index");
        // a computer with a battery keeps them in Battery, not in Energy
        let with = spec(PageId::Energy, &model(&desktop_info(true, false), &opts));
        assert!(!with.groups.iter().any(|g| g.items.iter().any(|i| i.label == "Low Power Mode")));
        // and its menu bar has no Battery control to switch
        let bar = spec(PageId::MenuBar, &model(&info, &opts));
        assert!(!bar.groups[0].items.iter().any(|i| i.label == "Battery"), "no Battery menu extra without a battery");
    }

    #[test]
    fn wifi_has_its_own_row_above_network_when_the_computer_has_a_wireless_adapter() {
        use bifrost_ui::form_factor::FormFactor::Desktop;
        let labels = root_labels(&desktop_info(false, true), Desktop);
        let at = |name: &str| labels.iter().position(|l| l == name).unwrap_or(usize::MAX);
        assert_eq!(at("Wi-Fi") + 1, at("Network"), "Wi-Fi sits right above Network: {labels:?}");
        assert_eq!(labels.iter().filter(|l| *l == "Wi-Fi").count(), 1);
        assert!(!root_labels(&desktop_info(false, false), Desktop).iter().any(|l| l == "Wi-Fi"), "no adapter, no Wi-Fi row");
    }

    #[test]
    fn the_numeric_passcode_is_a_tablet_pane_and_a_desktop_has_a_login_password() {
        use bifrost_ui::form_factor::FormFactor::{Desktop, Tablet};
        assert!(!root_labels(&desktop_info(false, false), Desktop).iter().any(|l| l == "Passcode"));
        assert!(root_labels(&desktop_info(false, false), Tablet).iter().any(|l| l == "Passcode"), "a tablet keeps it");
        let (info, opts) = (desktop_info(false, false), Options::default());
        set_test_form(Desktop);
        let lock = spec(PageId::LockScreen, &model(&info, &opts));
        set_test_form(Tablet);
        assert_eq!(lock.groups[0].items[0].label, "Login Password");
        assert_eq!(lock.groups[0].items[0].action, Action::Push(PageId::Users));
    }

    #[test]
    fn touch_wording_gives_way_to_click_wording_on_a_desktop() {
        use bifrost_ui::form_factor::FormFactor::{Desktop, Tablet};
        let (info, opts) = (desktop_info(false, false), Options::default());
        let footer = |form, id| {
            set_test_form(form);
            let s = spec(id, &model(&info, &opts));
            set_test_form(Tablet);
            s.groups.iter().filter_map(|g| g.footer.clone()).collect::<Vec<_>>().join(" ")
        };
        assert!(footer(Tablet, PageId::ControlCentre).contains("Swipe down from the top-right corner"));
        assert!(!footer(Desktop, PageId::ControlCentre).contains("Swipe"));
        assert!(footer(Desktop, PageId::ControlCentre).contains("Click"));
        assert!(footer(Tablet, PageId::ScreenSaver).contains("Touch the screen"));
        let desktop_saver = footer(Desktop, PageId::ScreenSaver);
        assert!(!desktop_saver.contains("Touch") && desktop_saver.contains("Click or press a key"), "{desktop_saver}");
    }

    #[test]
    fn a_lone_command_key_row_is_left_out_of_the_desktop_shortcuts() {
        use bifrost_ui::form_factor::FormFactor::{Desktop, Tablet};
        let opts = Options::default();
        let home = |form| {
            set_test_form(form);
            let s = shortcuts_page(&opts);
            set_test_form(Tablet);
            s.groups[0].items.iter().any(|i| i.label == "Home")
        };
        assert!(home(Tablet));
        assert!(!home(Desktop));
        // the other rows still address their own bits
        set_test_form(Desktop);
        let s = shortcuts_page(&opts);
        set_test_form(Tablet);
        let assistant = s.groups[0].items.iter().find(|i| i.label == "Assistant").expect("the Assistant row");
        assert_eq!(assistant.action, Action::Toggle(Toggle::Shortcut(11)));
    }

    #[test]
    fn the_network_and_sidebar_rows_have_their_own_symbols() {
        let (info, opts) = (desktop_info(false, true), Options::default());
        let net = spec(PageId::Network, &model(&info, &opts));
        let icon = |label: &str| net.groups[0].items.iter().find(|i| i.label == label).and_then(|i| i.icon);
        assert_eq!((icon("Ethernet"), icon("VPN")), (Some(Icon::Ethernet), Some(Icon::Vpn)));
        let root = spec(PageId::Root, &model(&info, &opts));
        let icon = |label: &str| root.groups.iter().flat_map(|g| g.items.iter()).find(|i| i.label == label).and_then(|i| i.icon);
        for (label, want) in [("BifrOSt Account", Icon::Account), ("Print & Scan", Icon::Printer), ("Keyboard Shortcuts", Icon::Keyboard), ("Assistant", Icon::Assistant), ("Trackpad", Icon::Trackpad), ("Mouse", Icon::Mouse), ("Search", Icon::Search), ("Network", Icon::Network)] {
            assert_eq!(icon(label), Some(want), "{label}");
        }
        // every pane row has a tile of its own kind: none of the listed ones is the gear any more
        for label in ["Print & Scan", "Keyboard Shortcuts", "Assistant", "Trackpad", "Mouse", "Search"] {
            assert_ne!(icon(label), Some(Icon::General), "{label}");
        }
    }

    #[test]
    fn the_desktop_network_page_links_to_the_firewall() {
        let mut info = Info { desktop_network: Some(Default::default()), ..Info::default() };
        info.battery_present = true;
        let opts = Options::default();
        let s = spec(PageId::Network, &model(&info, &opts));
        let firewall = s.groups[0].items.iter().find(|i| i.label == "Firewall").expect("a Firewall row");
        assert_eq!(firewall.action, Action::Push(PageId::Firewall));
        assert!(spec(PageId::Firewall, &model(&info, &opts)).groups[0].items[0].label.starts_with("No Apps"));
    }

    #[test]
    fn wifi_page_lists_the_joined_network_first_and_the_rest_under_networks() {
        let (info, opts) = (info(), Options::default());
        let s = spec(PageId::Wifi, &model(&info, &opts));
        assert_eq!(s.groups[0].items[1].label, "WIFI-5G");
        assert!(s.groups[0].items[1].leading_check);
        assert_eq!(s.groups[0].items[1].action, Action::Push(PageId::WifiDetail));
        assert_eq!(s.groups[1].header.as_deref(), Some("NETWORKS"));
        assert_eq!(s.groups[1].items[0].action, Action::Network("Cafe".into()));
        assert_eq!(s.groups[1].items[0].accessory, Accessory::Wifi { secure: false, bars: 1 });
        assert_eq!(s.groups.last().unwrap().items[0].label, "Country or Region", "the regulatory domain shows under the networks");
        let off = spec(PageId::Wifi, &Model { wifi_on: false, ..model(&info, &opts) });
        assert_eq!(off.groups.len(), 1, "nothing but the switch while off");
        assert_eq!(off.groups[0].items.len(), 1);
        let none = Info { networks: vec![], current_ssid: None, ..info.clone() };
        let searching = spec(PageId::Wifi, &model(&none, &opts));
        assert_eq!(searching.groups[1].items[0].label, "Searching…");
        assert!(!searching.groups[1].items[0].tappable());
        let d = spec(PageId::WifiDetail, &model(&info, &opts));
        assert_eq!(d.title, "WIFI-5G");
        assert_eq!(d.groups[0].items[0].action, Action::Run(Command::Forget("WIFI-5G".into())));
        assert_eq!(d.groups[2].items[1].value, "192.168.1.1");
    }

    #[test]
    fn saved_networks_show_notes_priority_and_the_other_row() {
        let (mut info, opts) = (info(), Options::default());
        let saved = |ssid: &str, auto_join, note: &str| SavedNetwork { ssid: ssid.into(), auto_join, note: note.into(), security: None };
        info.saved = vec![saved("WIFI-5G", true, ""), saved("Cafe", false, "Incorrect Password")];
        let s = spec(PageId::Wifi, &model(&info, &opts));
        assert_eq!(s.groups[1].items[0].value, "Incorrect Password", "the failure shows in the row");
        assert_eq!(s.groups[1].items.last().unwrap().action, Action::JoinOther);
        assert_eq!(s.groups[2].items[0].action, Action::Push(PageId::KnownNetworks));
        let k = spec(PageId::KnownNetworks, &model(&info, &opts));
        assert_eq!(k.groups[0].items.len(), 2);
        assert_eq!(k.groups[0].items[0].value, "");
        assert_eq!(k.groups[0].items[1].value, "Incorrect Password");
        let d = spec(PageId::WifiDetail, &model(&info, &opts));
        assert_eq!(d.groups[1].items[0].action, Action::Wifi(WifiAction::SetAutoJoin { ssid: "WIFI-5G".into(), on: false }));
        let priority = d.groups.last().unwrap();
        assert_eq!(priority.header.as_deref(), Some("PRIORITY 1 OF 2"));
        assert!(!priority.items[0].tappable(), "already first");
        assert_eq!(priority.items[1].action, Action::Wifi(WifiAction::Move { ssid: "WIFI-5G".into(), up: false }));
        let off = Model { detail: "Cafe", ..model(&info, &opts) };
        assert_eq!(spec(PageId::WifiDetail, &off).groups[1].items[0].accessory, Accessory::Switch(false));
    }

    #[test]
    fn known_networks_name_the_security_and_the_details_show_the_link() {
        use crate::net::wifi_config::Security;
        let (mut info, opts) = (info(), Options::default());
        let saved = |ssid: &str, auto_join, note: &str, security| SavedNetwork { ssid: ssid.into(), auto_join, note: note.into(), security };
        info.saved = vec![saved("WIFI-5G", true, "", Some(Security::Wpa3)), saved("Cafe", false, "Incorrect Password", Some(Security::Open))];
        let k = spec(PageId::KnownNetworks, &model(&info, &opts));
        assert_eq!(k.groups[0].items[0].value, "WPA3 Personal");
        assert_eq!(k.groups[0].items[1].value, "None, Incorrect Password", "the problem follows the type");
        info.desktop_network = Some(Default::default());
        if let Some(n) = info.desktop_network.as_mut() {
            n.interfaces = vec![crate::shell::desktop_network::Interface { name: "wlan0".into(), kind: "wifi".into(), method: "auto".into(), dns: vec!["10.0.0.1".into()], ..Default::default() }];
        }
        let d = spec(PageId::WifiDetail, &model(&info, &opts));
        assert_eq!(d.title, "WIFI-5G");
        let details = d.groups.iter().find(|g| g.header.as_deref() == Some("DETAILS")).unwrap();
        assert_eq!(details.items[0].value, "WPA3 Personal");
        assert_eq!(details.items[1].value, "wlan0");
        assert_eq!(details.items[2].value, "Automatic (DHCP)");
        assert_eq!(details.items[3].value, "10.0.0.1");
        assert!(d.groups.iter().all(|g| g.header.as_deref() != Some("DIAGNOSTICS")), "WIFI-5G has no problem");
        let cafe = spec(PageId::WifiDetail, &Model { detail: "Cafe", ..model(&info, &opts) });
        let problem = cafe.groups.iter().find(|g| g.header.as_deref() == Some("DIAGNOSTICS")).unwrap();
        assert_eq!(problem.items[0].value, "Incorrect Password");
    }

    #[test]
    fn the_reset_page_offers_a_confirmed_erase_of_all_content() {
        let info = info();
        let opts = Options::default();
        let s = spec(PageId::Reset, &model(&info, &opts));
        let erase = s.groups.iter().flat_map(|g| &g.items).find(|i| i.label == "Erase All Content and Settings").expect("the Reset page has the erase row");
        assert_eq!(erase.action, Action::Run(Command::EraseContent), "the row asks for confirmation through the command");
    }

    #[test]
    fn desktop_display_shows_panel_mode_and_only_working_controls() {
        let mut info = info();
        info.display_mode = Some((2048, 1152, 60));
        let opts = Options::default();
        let s = desktop_display(&model(&info, &opts));
        assert_eq!(s.title, "Displays");
        assert_eq!(s.groups[0].items[0].value, "Native", "the Resolution row shows the choice, Native by default");
        assert_eq!(s.groups[0].items[1].value, "Highest Available", "the row shows the choice, Highest Available by default");
        assert_eq!(s.groups[0].items[1].action, Action::Push(PageId::RefreshRate));
        assert!(s.groups[0].items.iter().any(|i| i.label == "Graphics" && i.action == Action::Push(PageId::Graphics)), "the Graphics page is reachable");
        assert!(!s.groups.iter().flat_map(|g| &g.items).any(|i| i.label == "Use as" || i.label == "Brightness"));
        assert_eq!(s.groups[1].items[0].label, "Night Shift");
        assert!(!s.groups.iter().flat_map(|g| &g.items).any(|i| i.label == "True Tone"));
        info.brightness_supported = true;
        let s = desktop_display(&model(&info, &opts));
        assert!(matches!(s.groups[1].items[0].body, super::super::list::Body::Slider { ends: Ends::Brightness, .. }));
        assert_eq!(s.groups[1].items.len(), 1, "no light sensor, no automatic brightness switch");
    }

    #[test]
    fn the_speech_page_offers_the_punctuation_levels_and_checks_the_chosen_one() {
        let opts = Options::default();
        let info = info();
        let groups = |a: &AccessibilitySettings| {
            let m = Model { a11y: a, ..model(&info, &opts) };
            spec(PageId::ScreenReaderSpeech, &m).groups
        };
        let off = groups(&AccessibilitySettings::default());
        let punct = off.iter().find(|g| g.header.as_deref() == Some("PUNCTUATION")).expect("the Punctuation group");
        let labels: Vec<&str> = punct.items.iter().map(|i| i.label.as_str()).collect();
        assert_eq!(labels, ["None", "Some", "All"]);
        assert_eq!(punct.items[0].accessory, Accessory::Check(true));
        assert_eq!(punct.items[2].action, Action::Pick(Pick::SpeechPunctuation(2)));
        let all = groups(&AccessibilitySettings { speech_punctuation: 2, ..Default::default() });
        let punct = all.iter().find(|g| g.header.as_deref() == Some("PUNCTUATION")).unwrap();
        assert_eq!((punct.items[0].accessory.clone(), punct.items[2].accessory.clone()), (Accessory::Check(false), Accessory::Check(true)));
    }

    #[test]
    fn the_touch_group_has_the_switch_control_and_voice_control_switches() {
        let opts = Options::default();
        let info = info();
        let touch_rows = |a: &AccessibilitySettings| -> Vec<(String, Accessory, Action)> {
            let m = Model { a11y: a, ..model(&info, &opts) };
            let s = spec(PageId::Accessibility, &m);
            let group = s.groups.iter().find(|g| g.header.as_deref() == Some("TOUCH")).expect("the Touch group");
            group.items.iter().map(|i| (i.label.clone(), i.accessory.clone(), i.action.clone())).collect()
        };
        let off = touch_rows(&AccessibilitySettings::default());
        let find = |rows: &[(String, Accessory, Action)], name: &str| rows.iter().find(|r| r.0 == name).cloned().expect(name);
        assert_eq!(find(&off, "Switch Control").1, Accessory::Switch(false));
        assert_eq!(find(&off, "Voice Control").2, Action::Toggle(Toggle::VoiceControl));
        let on = touch_rows(&AccessibilitySettings { switch_control: true, voice_control: true, ..Default::default() });
        assert_eq!(find(&on, "Switch Control").1, Accessory::Switch(true));
        assert_eq!(find(&on, "Voice Control").1, Accessory::Switch(true));
        assert_eq!(find(&on, "Switch Control").2, Action::Toggle(Toggle::SwitchControl));
    }

    #[test]
    fn the_keyboard_group_has_the_accessibility_keyboard_switch() {
        let opts = Options::default();
        let info = info();
        let row = |a: &AccessibilitySettings| -> (Accessory, Action) {
            let m = Model { a11y: a, ..model(&info, &opts) };
            let s = spec(PageId::Accessibility, &m);
            let group = s.groups.iter().find(|g| g.header.as_deref() == Some("KEYBOARD")).expect("the Keyboard group");
            let item = group.items.iter().find(|i| i.label == "Accessibility Keyboard").expect("the Accessibility Keyboard row");
            (item.accessory.clone(), item.action.clone())
        };
        assert_eq!(row(&AccessibilitySettings::default()), (Accessory::Switch(false), Action::Toggle(Toggle::AccessibilityKeyboard)));
        assert_eq!(row(&AccessibilitySettings { accessibility_keyboard: true, ..Default::default() }).0, Accessory::Switch(true));
    }

    #[test]
    fn input_sources_has_a_show_input_menu_switch_that_is_off_by_default() {
        let info = info();
        let row = |opts: &Options| -> (Accessory, Action) {
            let s = spec(PageId::Keyboards, &model(&info, opts));
            let item = s.groups.iter().flat_map(|g| g.items.iter()).find(|i| i.label == "Show Input menu in menu bar").expect("the Show Input menu row");
            (item.accessory.clone(), item.action.clone())
        };
        assert_eq!(row(&Options::default()), (Accessory::Switch(false), Action::Toggle(Toggle::InputMenu)));
        assert_eq!(row(&Options { input_menu: true, ..Options::default() }).0, Accessory::Switch(true));
    }

    #[test]
    fn the_motion_group_offers_vehicle_motion_cues_only_with_an_accelerometer() {
        let opts = Options::default();
        let mut info = info();
        let motion_rows = |info: &Info, a: &AccessibilitySettings| -> Vec<(String, Accessory)> {
            let m = Model { a11y: a, ..model(info, &opts) };
            let s = spec(PageId::Accessibility, &m);
            let group = s.groups.iter().find(|g| g.header.as_deref() == Some("MOTION")).expect("the Motion group");
            group.items.iter().map(|i| (i.label.clone(), i.accessory.clone())).collect()
        };
        let off = AccessibilitySettings::default();
        let rows = motion_rows(&info, &off);
        assert_eq!(rows.len(), 1, "no accelerometer, no cues switch: {rows:?}");
        info.motion_sensor_supported = true;
        let rows = motion_rows(&info, &off);
        assert_eq!(rows[1], ("Show Vehicle Motion Cues".to_string(), Accessory::Switch(false)));
        let on = AccessibilitySettings { motion_cues: true, ..Default::default() };
        assert_eq!(motion_rows(&info, &on)[1].1, Accessory::Switch(true), "the switch follows the setting");
    }

    #[test]
    fn a_light_sensor_adds_the_automatic_brightness_switch_under_the_slider() {
        let mut info = info();
        info.display_mode = Some((2048, 1152, 60));
        info.brightness_supported = true;
        info.ambient_light_supported = true;
        let mut opts = Options::default();
        let rows = |opts: &Options| desktop_display(&model(&info, opts)).groups[1].items.clone();
        let off = rows(&opts);
        assert_eq!(off.len(), 2);
        assert_eq!(off[1].label, "Automatically Adjust Brightness");
        assert_eq!(off[1].action, Action::Toggle(Toggle::AutoBrightness));
        assert_eq!(off[1].accessory, super::super::list::Accessory::Switch(false));
        opts.auto_brightness = true;
        let on = rows(&opts);
        assert_eq!(on[1].accessory, super::super::list::Accessory::Switch(true), "the switch follows the option");
        // a panel that cannot be dimmed shows neither the slider nor the switch
        info.brightness_supported = false;
        let none = desktop_display(&model(&info, &opts));
        assert!(!none.groups.iter().flat_map(|g| &g.items).any(|i| i.label == "Automatically Adjust Brightness"));
    }

    #[test]
    fn the_refresh_row_shows_the_chosen_rate_and_opens_the_picker() {
        let mut info = info();
        info.display_mode = Some((2560, 1600, 165));
        info.display_rates = vec![164.85, 144.0, 60.0];
        let opts = Options::default();
        let m = Model { refresh: RefreshChoice::Hz(144.0), ..model(&info, &opts) };
        let s = desktop_display(&m);
        let row = &s.groups[0].items[1];
        assert_eq!((row.label.as_str(), row.value.as_str()), ("Refresh Rate", "144 Hz"));
        assert_eq!(row.action, Action::Push(PageId::RefreshRate));
        assert!(s.groups[0].footer.as_deref().unwrap_or("").contains("highest refresh rate is used unless another is chosen"));
    }

    #[test]
    fn the_resolution_row_shows_native_or_the_chosen_size_and_opens_the_picker() {
        let mut info = info();
        info.display_mode = Some((2560, 1600, 165));
        info.display_sizes = vec![(2560, 1600), (1920, 1200)];
        let opts = Options::default();
        let s = desktop_display(&Model { resolution: Some((1920, 1200)), ..model(&info, &opts) });
        let row = &s.groups[0].items[0];
        assert_eq!((row.label.as_str(), row.value.as_str()), ("Resolution", "1920 × 1200"));
        assert_eq!(row.action, Action::Push(PageId::Resolution));
        let native = desktop_display(&model(&info, &opts));
        assert_eq!(native.groups[0].items[0].value, "Native");
    }

    #[test]
    fn the_resolution_picker_lists_native_then_each_size_with_the_choice_checked() {
        let mut info = info();
        info.display_sizes = vec![(2560, 1600), (1920, 1200), (1280, 800)];
        let opts = Options::default();
        let p = spec(PageId::Resolution, &Model { resolution: Some((1920, 1200)), ..model(&info, &opts) });
        assert_eq!(p.title, "Resolution");
        let rows = &p.groups[0].items;
        let labels: Vec<&str> = rows.iter().map(|i| i.label.as_str()).collect();
        assert_eq!(labels, ["Native", "2560 × 1600", "1920 × 1200", "1280 × 800"]);
        let checked: Vec<bool> = rows.iter().map(|i| i.accessory == Accessory::Check(true)).collect();
        assert_eq!(checked, [false, false, true, false], "only the chosen size is checked");
        assert_eq!(rows[0].action, Action::Pick(Pick::Resolution(0)));
        assert_eq!(rows[3].action, Action::Pick(Pick::Resolution(3)));
        let native = spec(PageId::Resolution, &model(&info, &opts));
        assert!(native.groups[0].items[0].accessory == Accessory::Check(true), "Native is checked when nothing is chosen");
    }

    #[test]
    fn the_refresh_picker_lists_highest_available_and_each_rate_with_the_choice_checked() {
        let mut info = info();
        info.display_mode = Some((2560, 1600, 165));
        info.display_rates = vec![164.85, 144.0, 120.0, 100.0, 60.0];
        let opts = Options::default();
        let p = spec(PageId::RefreshRate, &Model { refresh: RefreshChoice::Hz(120.0), ..model(&info, &opts) });
        assert_eq!(p.title, "Refresh Rate");
        let rows = &p.groups[0].items;
        let labels: Vec<&str> = rows.iter().map(|i| i.label.as_str()).collect();
        assert_eq!(labels, ["Highest Available", "164.85 Hz", "144 Hz", "120 Hz", "100 Hz", "60 Hz"]);
        let checked: Vec<bool> = rows.iter().map(|i| i.accessory == Accessory::Check(true)).collect();
        assert_eq!(checked, [false, false, false, true, false, false], "only the chosen rate is checked");
        assert_eq!(rows[0].action, Action::Pick(Pick::RefreshRate(0)));
        assert_eq!(rows[2].action, Action::Pick(Pick::RefreshRate(2)));
        let max = spec(PageId::RefreshRate, &Model { refresh: RefreshChoice::Max, ..model(&info, &opts) });
        assert_eq!(max.groups[0].items[0].accessory, Accessory::Check(true), "Highest Available is checked by default");
    }

    #[test]
    fn sharing_and_lock_screen_are_reachable_from_the_root_and_link_their_controls() {
        let info = info();
        let mut opts = Options::default();
        let root = spec(PageId::Root, &model(&info, &opts));
        let items: Vec<&Item> = root.groups.iter().flat_map(|g| &g.items).collect();
        assert!(items.iter().any(|i| i.action == Action::Push(PageId::Sharing)), "Sharing is in the root list");
        assert!(items.iter().any(|i| i.action == Action::Push(PageId::LockScreen)), "Lock Screen is in the root list");

        // the computer name and the sharing services come first; Nearby Share and AirPlay & Handoff are the last group
        let view = super::super::sharing_page::View { computer: "Midget".into(), services: Vec::new(), screen_user: None };
        let sharing = sharing_spec(&opts, Some(&view));
        assert_eq!(sharing.groups[0].items[0].action, Action::Sharing(super::super::sharing_page::Edit::ComputerName));
        assert_eq!(sharing.groups[3].items[0].action, Action::Push(PageId::Nearby));
        assert_eq!(sharing.groups[3].items[0].value, NEARBY_CHOICES[opts.nearby.min(NEARBY_CHOICES.len() - 1)]);
        assert_eq!(sharing.groups[3].items[1].action, Action::Push(PageId::AirPlayHandoff), "AirPlay & Handoff is reachable, not an orphan page");
        opts.handoff = true;
        assert_eq!(sharing_spec(&opts, Some(&view)).groups[3].items[1].value, "Handoff On");
        assert_eq!(sharing_spec(&opts, None).groups[0].items[0].value, "Loading…", "the page shows a placeholder until the read lands");

        let lock = spec(PageId::LockScreen, &model(&info, &opts));
        assert_eq!(lock.groups[0].items[0].action, Action::Push(PageId::Passcode));
        assert_eq!(lock.groups[1].items[0].action, Action::Push(PageId::AutoLock));
        assert_eq!(lock.groups[1].items[0].value, opts.auto_lock_label());
        // Require Password has its own group now, so Screen Saver moved from group 2 to group 3
        assert_eq!(lock.groups[2].items[0].action, Action::Push(PageId::RequirePassword));
        assert_eq!(lock.groups[3].items[0].action, Action::Push(PageId::ScreenSaver));
    }

    #[test]
    fn option_pages_reflect_the_options() {
        let info = info();
        let opts = Options { auto_lock_ms: 0, text_scale: 1.3, bold_text: true, icon_size: 1, volume: 0.25, ..Options::default() };
        let m = model(&info, &opts);
        let d = spec(PageId::Display, &m);
        assert_eq!(d.groups[2].items[0].value, "Never");
        assert_eq!(d.groups[3].items[0].body, super::super::list::Body::Slider { ends: Ends::TextSize, value: 1.0 });
        assert_eq!(d.groups[3].items[1].accessory, Accessory::Switch(true));
        let e_opts = Options { system_sleep_min: 15, ..Options::default() };
        let e = spec(PageId::Energy, &Model { opts: &e_opts, ..model(&info, &opts) });
        assert_eq!(e.groups[0].items.len(), 1, "the sleep time is one pop-up row");
        assert_eq!(e.groups[0].items[0].accessory, Accessory::Popup);
        let sleep = e.groups[0].items[0].popup.as_ref().expect("the sleep time choices");
        assert_eq!(sleep.choices.len(), SYSTEM_SLEEP.len());
        assert_eq!(sleep.selected, 3, "15 minutes");
        assert_eq!(chosen(&e.groups[0].items[0].action, 6), Some(Action::Pick(Pick::SystemSleep(6))));
        let saver_opts = Options { screensaver_module: 1, screensaver_minutes: 5, ..Options::default() };
        let ss = spec(PageId::ScreenSaver, &Model { opts: &saver_opts, ..model(&info, &opts) });
        // Colours has its own Speed choice group between Preview and Start After
        assert_eq!(ss.groups.len(), 4);
        let saver = ss.groups[0].items[0].popup.as_ref().expect("the saver choice is one pop-up row");
        assert_eq!(ss.groups[0].items[0].accessory, Accessory::Popup);
        assert_eq!(saver.choices.len(), super::super::screensaver::SAVERS.len());
        assert_eq!(saver.selected, 1);
        assert_eq!(ss.groups[1].items[0].label, "Preview");
        let speed = ss.groups[2].items[0].popup.as_ref().expect("Colours: one Speed pop-up");
        assert_eq!(speed.choices.len(), 3, "Colours: slow, normal, fast");
        let start = ss.groups[3].items[0].popup.as_ref().expect("Start After is one pop-up row");
        assert_eq!(ss.groups[3].items[0].accessory, Accessory::Popup);
        assert_eq!(start.choices.len(), super::super::screensaver::START_AFTER.len());
        assert_eq!(start.selected, super::super::screensaver::START_AFTER.iter().position(|(_, min)| *min == 5).unwrap());
        let a = spec(PageId::AutoLock, &m);
        assert_eq!(a.groups[0].items.len(), AUTO_LOCK.len());
        assert_eq!(a.groups[0].items[4].accessory, Accessory::Check(true));
        assert_eq!(a.groups[0].items[0].accessory, Accessory::Check(false));
        let h = spec(PageId::HomeScreen, &m);
        assert_eq!(h.groups[0].items[1].accessory, Accessory::Check(true));
        let s = spec(PageId::Sounds, &m);
        assert_eq!(s.groups[0].items[0].body, super::super::list::Body::Slider { ends: Ends::Volume, value: 0.25 });
        let b = spec(PageId::Battery, &m);
        assert_eq!(b.groups[0].items[0].value, "63%");
        assert_eq!(b.groups[0].items[1].value, "Charging");
        assert!(matches!(b.groups[1].items[0].body, super::super::list::Body::Graph(_)));
        let u = spec(PageId::SoftwareUpdate, &m);
        assert_eq!(u.groups[0].items[0].value, env!("CARGO_PKG_VERSION"));
        assert!(!BUILD.git.is_empty() && !BUILD.date.is_empty());
        let t = spec(PageId::DateTime, &m);
        // the clock options (24-hour, date and day in the menu bar) take groups 1 and 2, so Time Zone and Time are group 3
        assert_eq!(t.groups[3].items[1].value, "14:05");
        assert_eq!(t.groups[3].items[0].action, Action::TimeZone);
        let acc = spec(PageId::Accessibility, &m);
        assert_eq!(acc.groups[0].items[1].value, "130%");
        assert_eq!(acc.groups[0].items[6].accessory, super::super::list::Accessory::Switch(false));
        for id in [PageId::General, PageId::About, PageId::Keyboard, PageId::Language, PageId::Region, PageId::Reset, PageId::ControlCentre, PageId::Privacy] {
            assert!(!spec(id, &m).groups.is_empty(), "{id:?}");
        }
    }

    #[test]
    fn language_and_region_mark_the_fresh_install_defaults() {
        // nothing is published in this test, so the page shows the defaults: English, Iceland, automatic weeks
        use super::super::list::Accessory;
        let opts = Options::default();
        let info = info();
        let lang = spec(PageId::Language, &model(&info, &opts));
        assert_eq!(lang.groups[0].items[0].accessory, Accessory::Check(true));
        assert_eq!(lang.groups[0].items[1].accessory, Accessory::Check(false));
        assert_eq!(lang.groups[1].items[0].value, "Iceland");
        assert_eq!(lang.groups[1].items[0].action, Action::Push(PageId::Region));
        assert_eq!(lang.groups[2].items[0].accessory, Accessory::Check(true));
        assert_eq!(lang.groups[2].items[2].accessory, Accessory::Check(false));
        let region = spec(PageId::Region, &model(&info, &opts));
        assert_eq!(region.groups[0].items[0].accessory, Accessory::Check(true));
        assert_eq!(region.groups[0].items[1].action, Action::Pick(Pick::Region(1)));
    }

    fn bt_device(addr: &str, name: &str, paired: bool, connected: bool) -> crate::bare::bluetooth::Device {
        crate::bare::bluetooth::Device { addr: addr.into(), kind: 0, name: name.into(), paired, connected, audio: true, rssi: None, class: None }
    }

    #[test]
    fn bluetooth_page_lists_my_devices_then_the_ones_nearby() {
        use crate::bare::bluetooth::{Cmd, State, Status};
        let opts = Options::default();
        let mut info = info();
        info.bt = Status { state: State::On, scanning: true, devices: vec![bt_device("A", "Buds", true, true), bt_device("B", "Speaker", false, false)], ..Status::default() };
        let s = spec(PageId::Bluetooth, &model(&info, &opts));
        assert_eq!(s.groups[0].items[0].action, Action::Toggle(Toggle::Bluetooth));
        assert_eq!(s.groups[1].header.as_deref(), Some("MY DEVICES"));
        assert_eq!((s.groups[1].items[0].label.as_str(), s.groups[1].items[0].value.as_str()), ("Buds", "Connected"));
        assert_eq!(s.groups[1].items[0].action, Action::Push(PageId::BluetoothDetail));
        assert_eq!(s.groups[2].items[0].action, Action::Bluetooth(Cmd::Pair("B".into())));
        let root = spec(PageId::Root, &model(&info, &opts));
        assert_eq!(root.groups[1].items[2].value, "Connected");
        info.bt.state = State::Off;
        assert_eq!(spec(PageId::Bluetooth, &model(&info, &opts)).groups.len(), 1, "off: just the switch");
        info.bt.state = State::Unavailable;
        let none = spec(PageId::Bluetooth, &model(&info, &opts));
        assert!(none.groups[0].footer.as_deref().unwrap().contains("not available"));
    }

    #[test]
    fn bluetooth_page_offers_a_scan_row_only_when_not_scanning() {
        use crate::bare::bluetooth::{Cmd, State, Status};
        let opts = Options::default();
        let mut info = info();
        info.bt = Status { state: State::On, scanning: false, devices: vec![bt_device("B", "Speaker", false, false)], ..Status::default() };
        let s = spec(PageId::Bluetooth, &model(&info, &opts));
        let nearby = &s.groups[1].items;
        assert_eq!(nearby.len(), 2);
        assert_eq!(nearby[0].action, Action::Bluetooth(Cmd::Pair("B".into())));
        assert_eq!(nearby[1].label.as_str(), "Scan for Devices");
        assert!(nearby[1].action == Action::Bluetooth(Cmd::Scan));
        info.bt.scanning = true;
        let s = spec(PageId::Bluetooth, &model(&info, &opts));
        assert_eq!(s.groups[1].items.len(), 1, "no scan row while a scan runs");
    }

    #[test]
    fn bluetooth_detail_offers_connect_or_disconnect_and_forget() {
        use crate::bare::bluetooth::{Cmd, State, Status};
        let opts = Options::default();
        let mut info = info();
        info.bt = Status { state: State::On, devices: vec![bt_device("A", "Buds", true, false)], ..Status::default() };
        let m = Model { detail: "Buds", ..model(&info, &opts) };
        let s = spec(PageId::BluetoothDetail, &m);
        assert_eq!(s.title, "Buds");
        assert_eq!(s.groups[0].items[0].action, Action::Bluetooth(Cmd::Connect("A".into())));
        assert_eq!(s.groups[1].items[0].action, Action::Run(Command::ForgetBluetooth("A".into())));
        info.bt.devices[0].connected = true;
        let m = Model { detail: "Buds", ..model(&info, &opts) };
        let detail = spec(PageId::BluetoothDetail, &m);
        assert_eq!(detail.groups[0].items[0].action, Action::Bluetooth(Cmd::Disconnect("A".into())));
        // a connected audio device can take the sound; the type row says what the class says, and "Unknown" with no class
        assert_eq!(detail.groups[0].items[1].label.as_str(), "Use for Sound Output");
        assert_eq!(detail.groups[0].items[1].action, Action::AudioOutput);
        let kind = &detail.groups[2].items[1];
        assert_eq!((kind.label.as_str(), kind.value.as_str()), ("Type", "Unknown"));
        // the signal row reads "Not reported" until a scan heard the device, then its strength
        let signal = &detail.groups[2].items[3];
        assert_eq!((signal.label.as_str(), signal.value.as_str()), ("Signal", "Not reported"));
        info.bt.devices[0].rssi = Some(-61);
        let heard = spec(PageId::BluetoothDetail, &Model { detail: "Buds", ..model(&info, &opts) });
        assert_eq!(heard.groups[2].items[3].value.as_str(), "-61 dBm");
        let services = &detail.groups[3];
        assert_eq!((services.items[0].label.as_str(), services.items[0].value.as_str()), ("Services", "None listed"));
        assert_eq!(services.header.as_deref(), Some("SERVICES"));
    }

    #[test]
    fn keyboard_haptics_switch_needs_a_vibrator() {
        let (info, opts) = (info(), Options::default());
        let labels = |haptics| {
            let s = spec(PageId::Sounds, &Model { haptics, ..model(&info, &opts) });
            format!("{s:?}")
        };
        assert!(!labels(false).contains("Keyboard Haptics"));
        assert!(labels(true).contains("Keyboard Haptics"));
        assert!(labels(false).contains("Keyboard Clicks"));
    }

    #[test]
    fn keyboard_links_to_text_replacements_and_the_empty_list_says_so() {
        let (info, opts) = (info(), Options::default());
        let keyboard = spec(PageId::Keyboard, &model(&info, &opts));
        let row = keyboard.groups[0].items.iter().find(|i| i.label == "Text Replacements").expect("the Keyboard page has the row");
        assert_eq!(row.action, Action::Push(PageId::TextReplacements));
        let page = spec(PageId::TextReplacements, &model(&info, &opts));
        assert_eq!(page.groups[0].items[0].label, "No Text Replacements", "the list is empty in a fresh shell");
        assert_eq!(page.groups[0].items[0].action, Action::None);
    }
}

fn desktop_network_page(info:&Info)->Spec {
    let network=info.desktop_network.as_ref();
    let ethernet=network.and_then(|n|n.interfaces.iter().find(|i|i.kind=="ethernet"));
    let wifi=network.is_some_and(|n|n.wifi_available);
    let eth_status=ethernet.map(|i|if i.connected(){"Connected"}else{"Not Connected"}).unwrap_or("No adapter");
    Spec{id:PageId::Network,title:"Network".into(),large:false,groups:vec![
        Group::new(vec![Item::new("Ethernet").icon(Icon::Ethernet).value(eth_status).link(PageId::Ethernet), Item::new("Wi-Fi").icon(Icon::Wifi).value(if wifi {info.wifi.as_str()}else{"No adapter"}).link(PageId::Wifi), Item::new("VPN").icon(Icon::Vpn).value(match network.map_or(&[][..], |n|n.vpns.as_slice()){[]=>"None", v if v.iter().any(|c|c.active)=>"Connected", _=>"Not Connected"}).link(PageId::Vpn), Item::new("Firewall").icon(Icon::Passcode).link(PageId::Firewall)]).header("CONNECTIONS"),
        rows(vec![("Network Manager", "NetworkManager".into())]).footer("Ethernet and Wi-Fi use separate connection settings. Changes are applied by the system network service."),
    ]}
}
fn desktop_ethernet_page(info:&Info)->Spec {
    let interfaces:Vec<_>=info.desktop_network.as_ref().map(|n|n.interfaces.iter().filter(|i|i.kind=="ethernet").collect()).unwrap_or_default();
    let groups=if interfaces.is_empty(){vec![Group::new(vec![Item::new("No Ethernet adapter").tint(Tint::Dim)])]}else{interfaces.into_iter().map(|i|rows(vec![
        ("Status",if i.connected(){"Connected".into()}else{"Not Connected".into()}),("Interface",i.name.clone()),("Connection",i.connection.clone()),
        ("Configure IPv4",match i.method.as_str(){"auto"=>"Automatic (DHCP)","manual"=>"Manually",_=>"—"}.into()),
        ("IP Address",if i.ipv4.is_empty(){"—".into()}else{i.ipv4.join(", ")}),("Router",if i.gateway.is_empty(){"—".into()}else{i.gateway.clone()}),("DNS Servers",if i.dns.is_empty(){"—".into()}else{i.dns.join(", ")}),
        ("Ethernet Address",if i.mac.is_empty(){"—".into()}else{i.mac.clone()}),("IPv6 Address",if i.ipv6.is_empty(){"—".into()}else{i.ipv6.join(", ")}),
    ]).header(&i.name).footer("Connection details update automatically. Unplugging this cable disconnects remote access over Ethernet.")).collect()};
    Spec{id:PageId::Ethernet,title:"Ethernet".into(),large:false,groups}
}
fn desktop_vpn_page(info:&Info)->Spec {
    let vpns:&[super::desktop_network::VpnConnection]=info.desktop_network.as_ref().map_or(&[],|n|n.vpns.as_slice());
    let items:Vec<Item>=if vpns.is_empty(){vec![Item::new("No VPN Configurations").tint(Tint::Dim)]}else{vpns.iter().map(|v|Item::new(v.name.clone()).value(v.status_label()).action(Action::Vpn{name:v.name.clone(),up:!v.active})).collect()};
    Spec{id:PageId::Vpn,title:"VPN".into(),large:false,groups:vec![Group::new(items).header("CONFIGURATIONS").footer("These are the VPN and WireGuard connections the system network service holds. Choose one to connect or disconnect it. Adding a configuration is done with the network service (nmcli connection import) and is not built into this page yet.")]}
}
