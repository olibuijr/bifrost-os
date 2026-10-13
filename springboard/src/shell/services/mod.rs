//! System services that apps call by name: `ToShell::ServiceCall { service, method, body }` is answered with
//! `ToApp::ServiceReply`, and a service may push `ToApp::ServiceEvent`s. The SDK side is `bifrost_app::service`.
//!
//! Adding a service: write `services/<name>.rs` with a type that implements [`SystemService`], add `pub mod <name>;`
//! below and one line to [`all`]. Nothing in the wire protocol changes. A service gets the whole [`Shell`] on every
//! call and tick, so it can read and change any shell state; it must answer at once (no blocking work on the shell
//! thread: hand slow work to a thread and report it later as an event).
//!
//! # Reference launch agents and the shell module that fills each role
//!
//! The reference system runs these roles as separate launchd agents; BifrOSt runs them inside the one shell process
//! (the compositor is labwc, started by `packaging/arch/springboard.service`). Rows are tracked in
//! `docs/parity-matrix/os/daemons/` and guarded by `springboard/tests/daemon_map.rs`.
//!
//! | Reference agent | BifrOSt module (under `springboard/src/shell/`) |
//! |---|---|
//! | Dock | `desktop_dock.rs`, `dock_stack.rs` |
//! | Finder (desktop) | `desk.rs` |
//! | MenuBarAgent | `desktop_bar.rs` |
//! | controlcenter | `control_center.rs` |
//! | notificationcenterui | `notify/center.rs`, `notify/banner.rs`, `notify/lockwidgets.rs` |
//! | usernotificationsd, UserNotificationCenter | `usernotif.rs`, `notify/store.rs`, `notify/schedule.rs`, `notify/permission.rs` |
//! | WindowManager, WindowServer | `workspace.rs`, `window_fx.rs`, labwc |
//! | Spotlight | `quicksearch.rs` |
//! | corespotlightd, mdworker.shared | `spotindex.rs` |
//! | screencaptureui | `screenshot.rs`, `screenrec.rs` |
//! | wallpaper | `wallpaper_page.rs` |
//! | loginwindow | `login.rs` |
//! | TextInputSwitcher, TextInputMenuAgent | `input_source_hud.rs`, `kb_layouts.rs`, `fcitx_bridge.rs` |
//! | menuextra.battery.helper | `battery.rs`, `low_battery.rs` |
//! | xpc.loginitemregisterd | `login_items_page.rs` |
//! | coreservices.sharedfilelistd | `recent_items.rs` |
//! | screensharing | `sharing_page.rs` (toggle only) |
//! | SoftwareUpdateNotificationManager | `ota.rs` |
//! | ScreenReaderUIServer | `accessibility/reader.rs` |
//! | gamecontrolleragentd | `game_controllers_page.rs` |
//!
//! Siri, family controls, iCloud, search-party, Screen Time and the memory/expansion slot agents have no counterpart
//! and stay gap rows.
use super::Shell;
use bifrost_core::json::Json;

pub mod pasteboard_types;
pub mod keychain_groups;
pub mod search_ranking;
pub mod privacy_permissions;
pub mod contacts;
pub mod calendar;
pub mod location;
pub mod screen_sample;
pub mod power_service;
pub mod audio_devices;
pub mod quick_look;
pub mod gamepad_service;
pub mod bluetooth_service;
pub mod login_items_approval;
pub mod network_path;
pub mod dock_tile;
pub mod now_playing;
pub mod notification_schedule;
pub mod screens;
pub mod cursor_service;
pub mod system;
pub mod widgets_timeline;
pub mod accessibility_display;
pub mod text_services;
pub mod focus_filter;
pub mod notify_bus;
pub mod launch_services;
pub mod display_services;
pub mod window_services;
pub mod event_services;
pub mod thumbnails;
pub mod thumb_providers;

/// A named service in the shell.
pub trait SystemService {
    /// The name apps call it by, for example `"dock"`.
    fn name(&self) -> &'static str;
    /// Handles `method` called by the app `app_id` with JSON `params`. Returns the result or an error message.
    fn call(&mut self, shell: &mut Shell, app_id: &str, method: &str, params: &Json, t: f64) -> Result<Json, String>;
    /// Runs on every shell tick; returns events to deliver.
    fn tick(&mut self, _shell: &mut Shell, _t: f64) -> Vec<Event> {
        Vec::new()
    }
}

/// Whether the installed app `app_id` lists `permission` in its manifest.
pub fn app_has_permission(app_id: &str, permission: &str) -> bool {
    super::apphost::installed().iter().find(|a| a.manifest.id == app_id).is_some_and(|a| a.manifest.permissions.iter().any(|p| p == permission))
}

/// The process id of the running installed app `app_id`, when it runs as a child of the shell.
pub fn app_pid(shell: &Shell, app_id: &str) -> Option<u32> {
    let slot = super::apphost::slot_of(app_id)?;
    shell.ext.get(slot)?.as_ref()?.pid()
}

/// An event a service sends: to one app (`app` is its id) or to every running app (`None`).
#[derive(Clone, Debug, PartialEq)]
pub struct Event {
    pub app: Option<String>,
    pub event: String,
    pub body: Json,
}

/// Every service, in registration order. One line per service.
pub fn all() -> Vec<Box<dyn SystemService>> {
    vec![
        Box::new(dock_tile::DockService),
        Box::new(system::System),
        Box::new(pasteboard_types::Pasteboards::default()),
        Box::new(keychain_groups::KeychainGroups::default()),
        Box::new(search_ranking::SearchRanking::default()),
        Box::new(privacy_permissions::PrivacyPermissions),
        Box::new(contacts::Contacts),
        Box::new(calendar::Calendar),
        Box::new(location::Location::default()),
        Box::new(screen_sample::ScreenSample),
        Box::new(widgets_timeline::WidgetsTimeline),
        Box::new(power_service::PowerService::default()),
        Box::new(audio_devices::AudioDevices::new()),
        Box::new(quick_look::PreviewService::default()),
        Box::new(gamepad_service::GamepadService::default()),
        Box::new(bluetooth_service::BluetoothService::default()),
        Box::new(login_items_approval::LoginItemsApproval),
        Box::new(network_path::NetworkPath::new()),
        Box::new(now_playing::NowPlaying),
        Box::new(notification_schedule::NotificationSchedule::default()),
        Box::new(screens::Displays::default()),
        Box::new(cursor_service::CursorService),
        Box::new(accessibility_display::AccessibilityDisplay::default()),
        Box::new(text_services::TextServices),
        Box::new(focus_filter::FocusFilterService::default()),
        Box::new(notify_bus::NotifyBus::default()),
        Box::new(launch_services::LaunchServices),
        Box::new(display_services::DisplayServices::default()),
        Box::new(window_services::WindowServices),
        Box::new(event_services::EventServices),
        Box::new(thumbnails::Thumbnails::default()),
    ]
}

/// The names of the registered services.
pub fn names(list: &[Box<dyn SystemService>]) -> Vec<&'static str> {
    list.iter().map(|s| s.name()).collect()
}

impl Shell {
    /// Runs one service call for the app `app_id`. An unknown service or method is an error, never a panic.
    pub(super) fn service_call(&mut self, app_id: &str, service: &str, method: &str, body: &str, t: f64) -> Result<Json, String> {
        let params = if body.trim().is_empty() { Json::Null } else { bifrost_core::json::parse(body)? };
        let mut list = std::mem::take(&mut self.services);
        let r = match list.iter_mut().find(|s| s.name() == service) {
            Some(s) => s.call(self, app_id, method, &params, t),
            None => Err(format!("no service {service}")),
        };
        // a service registered during the call (none should) is kept after the original list
        list.append(&mut self.services);
        self.services = list;
        r
    }

    /// Ticks every service and sends the events they return to the running apps they name.
    pub(super) fn services_tick(&mut self, t: f64) {
        let mut list = std::mem::take(&mut self.services);
        let mut out = Vec::new();
        for s in list.iter_mut() {
            for e in s.tick(self, t) {
                out.push((s.name(), e));
            }
        }
        list.append(&mut self.services);
        self.services = list;
        if out.is_empty() {
            return;
        }
        let apps = super::apphost::installed();
        for (service, e) in out {
            let m = bifrost_proto::ToApp::ServiceEvent { service: service.to_string(), event: e.event.clone(), body: e.body.encode() };
            for slot in 0..self.ext.len() {
                let id = apps.get(slot).map(|a| a.manifest.id.as_str());
                if e.app.is_none() || e.app.as_deref() == id {
                    self.ext_send(slot, &m);
                }
            }
        }
    }
}
