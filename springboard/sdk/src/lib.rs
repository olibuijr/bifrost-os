//! BifrOSt app SDK (docs/apps.md): the client side of the SpringBoard app
//! protocol, the event loop, and the UI kit apps share with the shell.
//!
//! ```ignore
//! struct Hello;
//! impl bifrost_app::App for Hello {
//!     fn id(&self) -> &str { "is.example.hello" }
//!     fn draw(&mut self, _: &Ctx, s: &mut Surface, _t: f64) -> Damage { /* ... */ }
//! }
//! fn main() { bifrost_app::run(Hello) }
//! ```

pub use bifrost_ui as controls;
pub use bifrost_ui::{Command, Commands};
pub use bifrost_proto as proto;
pub use bifrost_proto::{AppMenu, AppMenuItem, Configure, KeyInput, TouchKind};

/// The shell's drawing, text and Settings-style list kit, reused as is so apps
/// look like stock bifrost apps (see AGENTS.md design rules).
pub mod kit {
    pub use bifrost_render::font::Font;
    pub use bifrost_render::gfx::{self, pack, rgba, Area, Damage, Image, Surface};
    pub use bifrost_render::cache::TextCache;
    pub use bifrost_render::clipboard;
    pub use springboard::shell::list;
    pub use bifrost_render::appearance;
    pub use bifrost_render::ui::{self, Ui, Weight};
    /// the native toolkit-named semantic colours (`NativeColor.labelColor` etc.) as dynamic `Pair`s.
    pub use bifrost_render::ui::system as colors;
    pub use bifrost_ui::Event;
    /// The one scroll physics for every list in the system (bifrost drag, 0.998/ms deceleration,
    /// velocity over the last 100 ms, rubber band and a velocity-carrying bounce at the edges).
    /// Feed it `down`/`moved`/`up` with the touch time, call `tick(t)` each frame, draw at `offset()`.
    pub use bifrost_ui::scroll::Scroller;
}

pub mod widgets;
pub mod glyphs;
pub mod defaults;
pub mod screen_saver_defaults;
pub mod locale;
pub mod keychain;
pub mod gss_status;
pub mod gss_oid;
pub mod gss_buffer;
pub mod accounts;
pub mod testing;
pub mod usernotif;
pub mod service_management;
pub mod crash_report;
pub mod status_item;
pub mod share;
pub mod print;
pub mod pasteboard;
pub mod dnd;
pub mod review;
pub mod app_transaction;
pub mod transfer;
pub mod layout;
pub mod control;
pub mod intents;
pub mod intent_resolve;
pub mod donation;
pub mod donations;
pub mod extension;
pub mod quicksearch;
pub mod visual_search;
pub mod peek;
pub mod webview;
pub mod webview_dev;
pub mod script_value;
pub mod script_syntax;
pub mod view;
pub mod document;
pub mod log;
pub mod signpost;
pub mod dispatch;
pub mod synchronization;
pub mod fswatch;
pub mod file_coordination;
pub mod save_sync;
pub mod file_provider;
pub mod file_manager;
pub mod url;
pub mod multipart;
pub mod social;
pub mod url_components;
pub mod http_cookies;
pub mod http_auth;
pub mod stream;
pub mod external_accessory;
pub mod error;
pub mod coding;
pub mod peer_session;
pub mod peer_mesh;
pub mod restoration;
pub use restoration::RestorableState;
pub mod attributed;
pub mod spellcheck;
pub mod translation;
pub mod notification;
pub mod system_sound;
pub mod workspace;
pub mod distributed;
pub mod observable;
pub mod observable_object;
pub mod ndarray;
pub mod inference;
pub mod music_understanding;
pub use observable::{Cancellables, Observable, Subscription};
pub use notification::{DistributedNotificationCenter, Notification, NotificationCenter, ObserverToken};
/// `NativeCache` parity (see `docs/bifrost-ui.md`, "Cache").
pub mod cache {
    pub use bifrost_render::purgeable::{purge_all, Cache};
}
pub mod process_info;
pub mod process_events;
pub mod file_metadata;
pub mod auth_events;
pub mod identity_events;
pub mod route_events;
pub mod bluetooth_events;
pub use process_info::{Activity, ActivityOptions, ProcessInfo, ThermalState};
/// Mach-style host statistics (`host_statistics64`, `task_info`, `os_proc_available_memory`); see `docs/bifrost-ui.md`.
pub use bifrost_core::host;
mod runtime;
pub use kit::{Damage, Surface, Ui};
pub use runtime::{run, run_on, Ctx, Document, DocumentEvent, Input, OpenOptions};
pub use proto::ContextItem;

/// The audio service client (`audio::Client`): playback state and transport over `bifrost_proto::svc::audio`.
pub use springboard::svc::audio;

/// An app. Only `id` and `draw` are required.
pub trait App {
    /// The reverse-DNS id from the manifest.
    fn id(&self) -> &str;
    /// Size, scale, safe area or appearance changed (also called once before the first frame).
    fn configure(&mut self, _ctx: &Ctx) {}
    /// A touch, key or URL; true when the app needs a new frame.
    fn event(&mut self, _ctx: &Ctx, _ev: Input) -> bool {
        false
    }
    /// Semantic controls in logical points; sent only when they change.
    fn controls(&self, _ctx: &Ctx) -> Vec<controls::Node> {Vec::new()}
    /// Parent/child relations and focus/checked state for the nodes `controls` returns (`NativeAccessibility` hierarchy).
    /// Sent only when they change; the default (empty) leaves every node a direct child of the window.
    fn controls_tree(&self, _ctx: &Ctx) -> Vec<crate::proto::NodeExt> {Vec::new()}
    /// Caret, selection and visible range of the text elements `controls` returns (`AXSelectedTextRange`,
    /// `AXVisibleCharacterRange`); a secure field reports only its length. Sent only when they change.
    fn ax_text(&self, _ctx: &Ctx) -> Vec<bifrost_ui::ax::AxText> {Vec::new()}
    /// `accessibilityTitle` and `accessibilityHelp` of elements (`Node::label` is `accessibilityLabel`). Sent only when they change.
    fn ax_meta(&self, _ctx: &Ctx) -> Vec<bifrost_ui::ax::AxMeta> {Vec::new()}
    /// An assistive technology acts on node `id` (`ax set`: `AXPress`, an `AXValue` write, `AXFocused`). The default maps
    /// `Press` onto `Input::Activate(id)` and ignores the rest; apps with editable nodes override it. True when a frame is needed.
    fn ax_set(&mut self, ctx: &Ctx, id: u64, action: crate::proto::AxAction) -> bool {
        match action {
            crate::proto::AxAction::Press => self.event(ctx, Input::Activate(id)),
            _ => false,
        }
    }
    /// Available semantic commands for the focused control.
    fn commands(&self, _ctx: &Ctx) -> Commands { Commands::default() }
    /// The Edit menu titles for Undo and Redo (`undoMenuItemTitle`): `("Undo Typing", "Redo Move")`; an empty string
    /// keeps the plain "Undo" or "Redo". Sent to the shell only when they change.
    fn undo_titles(&self, _ctx: &Ctx) -> (String, String) { (String::new(), String::new()) }
    /// The app's own main-menu items (`NativeApp.mainMenu`), shown by the desktop bar after the app-name menu while the
    /// app is in front. Called after every event and frame; the runtime sends the menus only when they differ from
    /// the last ones, so enabled and checked state is recomputed here (`validateMenuItem:`).
    fn main_menu(&self, _ctx: &Ctx) -> Vec<AppMenu> { Vec::new() }
    /// The user chose menu item `id` (from the bar or its key equivalent); true when the app needs a new frame.
    fn menu_action(&mut self, _ctx: &Ctx, _id: u32) -> bool { false }
    /// Draws into `surface` (premultiplied pixels, stride = width) and returns what changed.
    /// The first frame after a (re)allocation is always sent in full.
    fn draw(&mut self, ctx: &Ctx, surface: &mut Surface, t: f64) -> Damage;
    /// The app left the foreground: save state. No frames are drawn until `resume`.
    fn suspend(&mut self, _ctx: &Ctx) {}
    fn resume(&mut self, _ctx: &Ctx) {}
    /// IME and keyboard composition (`NativeTextInputClient` / `NativeTextInput`): `SetMarkedText`, `UnmarkText`,
    /// `DeleteSurrounding` and accessory buttons, from Wayland `text-input-v3` or the on-screen keyboard. Apps with a
    /// `bifrost_ui::text::TextBuffer` map them onto `set_marked_text`, `unmark_text(commit)` and `delete_surrounding`. Return true when a frame is needed.
    fn text_input(&mut self, _ctx: &Ctx, _ev: crate::proto::ToApp) -> bool {
        false
    }
    /// Keyboard system app only (`provides = ["keyboard"]`): the shell configured the keyboard from the focused field
    /// (layout, return key, predictive text, shown). Other apps never receive it. Return true when a frame is needed.
    fn keyboard_config(&mut self, _ctx: &Ctx, _config: crate::proto::KeyboardConfig) -> bool {
        false
    }
    /// The answer to `ctx.open_document`, `save_document`, `commit_document` or `reopen_document` (`NativeOpenPanel`'s
    /// completion handler). Return true when a frame is needed.
    fn document(&mut self, _ctx: &Ctx, _ev: DocumentEvent) -> bool {
        false
    }
    /// The answer to a `keychain::add`, `get`, `update` or `delete` call (`SecItem*`). Return true when a frame is needed.
    fn keychain(&mut self, _ctx: &Ctx, _reply: keychain::Reply) -> bool {
        false
    }
    /// A user notification event (`UNUserNotificationCenterDelegate`): the user's response, or the answer to a
    /// `usernotif::add`, `delivered` or `pending` call. Return true when a frame is needed.
    fn notification(&mut self, _ctx: &Ctx, _ev: usernotif::Event) -> bool {
        false
    }
    /// The user opened an indexed item from Search (`NativeUserActivity` continuation, `CSSearchableItemActionType`). Show the
    /// item named by `activity.unique_identifier`. Return true when a frame is needed.
    fn continue_activity(&mut self, _ctx: &Ctx, _activity: quicksearch::UserActivity) -> bool {
        false
    }
    /// The answer to `share::show_picker`: the app id the user chose (`share::COPY` for Copy), or `None` when the menu was
    /// dismissed. Return true when a frame is needed.
    fn share_done(&mut self, _ctx: &Ctx, _request: u32, _target: Option<String>) -> bool {
        false
    }
    /// The answer to a `service_management::SMAppService` call (also sent once at start). Return true when a frame is needed.
    fn login_item_status(&mut self, _ctx: &Ctx, _request: u32, _status: service_management::Status) -> bool {
        false
    }
    /// The user chose entry `id` in the menu of status item `item` (`status_item::StatusItem::id`). Return true when a frame
    /// is needed.
    fn status_item_action(&mut self, _ctx: &Ctx, _item: u32, _id: u32) -> bool {
        false
    }
    /// A drag and drop event (`NativeDraggingDestination` for a window that called `dnd::register_types`,
    /// `NativeDraggingSource` ending for a session the app began). Return true when a frame is needed.
    fn drag(&mut self, _ctx: &Ctx, _ev: dnd::Event) -> bool {
        false
    }
    /// State restoration (`NativeWindowRestoration`): the window's restoration identifier. `None` (the default) means the
    /// window is not restorable; the other restoration hooks are then never called. See `restoration`.
    fn restoration_identifier(&self) -> Option<&str> {
        None
    }
    /// `NativeWindow.isRestorable`: true when the app has a restoration identifier.
    fn is_restorable(&self) -> bool {
        self.restoration_identifier().is_some()
    }
    /// `encodeRestorableState(with:)`: put what is needed to restore the window into `state`. Called by the runtime
    /// after `ctx.invalidate_restorable_state()` (coalesced), on suspend and on quit.
    fn encode_restorable_state(&self, _ctx: &Ctx, _state: &mut RestorableState) {}
    /// `restoreState(with:)`: called once, after the first `configure`, when the shell asked this launch to restore.
    /// Return true when a frame is needed.
    fn restore_state(&mut self, _ctx: &Ctx, _state: &RestorableState) -> bool {
        false
    }
    /// Called every loop turn while in the foreground; true when a frame is needed.
    fn tick(&mut self, _ctx: &Ctx, _t: f64) -> bool {
        false
    }
    /// When `tick` next has work (ms, same clock as `t`); `None` sleeps until an event.
    fn next_wake(&self, _t: f64) -> Option<f64> {
        None
    }
    /// A descriptor the event loop also waits on while in the foreground (for example
    /// `webview::WebProcessPool::fd`); when it becomes readable, `tick` runs.
    fn wake_fd(&self) -> Option<i32> {
        None
    }
    /// True when `draw` paints every pixel each frame: the runtime then skips bringing the buffer
    /// up to date with the previous frame first (a full-screen copy).
    fn full_redraw(&self) -> bool {
        false
    }
    /// App Intents (`AppIntent`): typed actions served as `intent.<id>`, `intent.list` and `entity.query` over the
    /// control socket; see `intents`. Declare the same ones as `[[intent]]` in `manifest.toml` for Search.
    fn intents(&self) -> Vec<intents::Intent> {
        Vec::new()
    }
    /// Runs the declared intent `id`; `params` is already validated against its declaration.
    fn perform_intent(&mut self, _ctx: Option<&Ctx>, _data_dir: &std::path::Path, id: &str, _params: &bifrost_core::json::Json) -> Result<intents::IntentResult, control::RpcError> {
        Err(control::RpcError::new(control::METHOD_NOT_FOUND, format!("no intent {id}")))
    }
    /// `EntityQuery`: the entities of type `ty` that are suggested, have these ids, or match a string.
    fn entities(&mut self, _data_dir: &std::path::Path, ty: &str, _query: &intents::EntityQuery) -> Result<Vec<intents::Entity>, control::RpcError> {
        Err(control::RpcError::new(control::INVALID_PARAMS, format!("unknown entity type {ty}")))
    }
    /// App-specific control methods (tasks/app-control.md), listed by `app.methods`. The SDK adds `app.*`.
    fn rpc_methods(&self) -> Vec<&'static str> {
        Vec::new()
    }
    /// Answers one control request on the app's main thread. `ctx` is `None` before the first `Configure`;
    /// `data_dir` is always set, so data-only methods can answer from the start.
    /// Unknown methods return `METHOD_NOT_FOUND`.
    fn rpc(&mut self, _ctx: Option<&Ctx>, _data_dir: &std::path::Path, method: &str, _params: &bifrost_core::json::Json) -> Result<bifrost_core::json::Json, control::RpcError> {
        Err(control::RpcError::new(control::METHOD_NOT_FOUND, format!("no method {method}")))
    }
}
pub mod definitions;
