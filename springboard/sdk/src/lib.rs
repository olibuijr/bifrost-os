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
/// The shared remote command centre (`CommandCenter`): app-side handlers for the transport actions the system sends.
pub use bifrost_ui::remote_command;
pub use bifrost_proto as proto;
pub use bifrost_proto::{AppMenu, AppMenuItem, Configure, KeyInput, TouchKind};
pub use bifrost_ui::touch::{TouchEvent, TouchPhase, TouchType};

/// The shell's drawing, text and Settings-style list kit, reused as is so apps
/// look like stock bifrost apps (see AGENTS.md design rules).
pub mod kit {
    pub use bifrost_render::font::Font;
    pub use bifrost_render::gfx::{self, pack, rgba, Area, Damage, Image, Surface};
    pub use bifrost_render::cache::TextCache;
    pub use bifrost_render::clipboard;
    /// The 2D drawing stack: the context (`DrawContext`, `Bitmap`, paths, clipping, blend modes) that a view reaches for
    /// its frame through `view::with_context`; colours and colour spaces (`cg::color`); gradients, shadings, functions
    /// and patterns (`cg::paint`).
    pub use bifrost_render::cg;
    /// The drawing vocabulary `draw_canvas` takes (`Canvas`, `RectF`, `ImageRef`, ...).
    pub use bifrost_render::canvas;
    /// Pixel buffer layouts and the 8-bit CMYK, grey and RGB conversions, plane splitting and interleaving.
    pub use bifrost_render::pixel_layout;
    pub use springboard::shell::list;
    pub use bifrost_render::appearance;
    /// The shared motion contract: durations per kind of animation (0 under Reduce Motion) and the curves.
    pub use bifrost_render::motion;
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
pub mod widgets_timeline;
pub mod glyphs;
pub mod glyph_draw;
pub mod dock_tile;
pub mod quick_actions;
pub mod cursor_service;
pub mod text_services;
pub mod accessibility_display;
pub mod accessibility_rotor;
pub mod guided_access;
pub mod defaults;
pub mod font_descriptor;
pub mod font_substitution;
/// App-facing font objects, specs, the font manager, collections and fallback cascade.
pub use bifrost_render::{fontinfo, fontobj};
pub mod screen_saver_defaults;
pub mod screen_saver;
pub mod idle_watch;
pub mod screen_time;
pub mod screens;
pub mod displays;
pub mod windows;
pub mod input_events;
pub mod touch_events;
pub mod launch_services;
pub mod notify_bus;
pub mod locale;
pub mod time_zone;
pub mod formatters;
pub mod pref_domains;
pub mod calendar_system;
pub mod bundle_resources;
pub mod container_layout;
pub mod package_layout;
pub mod keychain;
pub mod keychain_groups;
pub mod keychain_query;
pub mod passwords;
pub mod entropy;
pub mod passkey;
pub mod password_gen;
pub mod digest;
pub mod crypto_key;
pub mod keychain_protection;
pub mod autofill;
pub mod trust;
pub mod gss_status;
pub mod gss_oid;
pub mod gss_buffer;
pub mod gss_name;
pub mod gss_token;
pub mod gss_credential;
pub mod gss_mechanism;
pub mod gss_context;
pub mod gss_message;
pub mod gss_krb5;
pub mod archive;
pub mod dispatch_data;
pub mod dispatch_io;
pub mod predicate_validating;
pub mod cf_bundle;
pub mod app_events;
pub mod dispatch_time;
pub mod timer_source;
pub mod background_tasks;
pub mod keyed_archive;
pub mod value_types;
pub mod colorsync;
pub mod decimal;
pub mod csv_writer;
pub mod accounts;
pub mod testing;
pub mod usernotif;
pub mod usernotif_model;
pub mod contacts;
pub mod calendar;
pub mod core_location;
pub mod notification_schedule;
pub mod event_recurrence;
pub mod messaging;
pub mod file_identity;
pub mod service_management;
pub mod service_bundle;
pub mod login_items_approval;
pub mod crash_report;
pub mod background_download;
pub mod sample_buffer;
pub mod diagnostic_report;
pub mod bug_report;
pub mod exception_handler;
pub mod call_stack_tree;
pub mod hitch_ratio;
pub mod daily_metric_report;
pub mod status_item;
pub mod share;
pub use bifrost_ui::print;
pub use bifrost_ui::print_pages;
pub use bifrost_ui::{cups, ipp};
pub mod print_panel;
pub mod drawing_model;
pub mod usb_printer;
pub mod usb_device;
pub mod pasteboard;
pub mod pasteboard_item;
pub mod pasteboard_types;
pub mod pasteboard_promise;
pub mod item_provider;
pub mod dnd;
pub mod spring_loaded;
pub mod paste_control;
pub mod review;
pub mod app_transaction;
pub mod transfer;
pub mod transfer_drag;
pub mod layout;
pub mod control;
pub mod intents;
pub mod intent_resolve;
pub mod donation;
pub mod donations;
pub mod extension;
pub mod quicksearch;
pub mod indexed_entity;
pub mod search_terms;
pub mod search_index_batch;
pub mod visual_search;
pub mod data_scanner;
pub mod sign_in;
pub mod sensors;
pub mod image_analysis;
pub mod peek;
pub mod quick_look;
pub mod permission_kit;
pub mod authorization;
pub mod audio_session;
pub mod audio_application;
pub mod audio_player_control;
pub mod now_playing;
pub mod thumbnails;
pub mod thumbnail_error;
pub mod peek_pdf;
pub mod web_download;
pub mod web_config;
pub mod web_data;
pub mod web_nav;
pub mod webview;
pub mod webview_dev;
pub mod power_sources;
pub mod hid_devices;
pub mod serial_ports;
pub mod ble_midi;
pub mod midi;
pub mod midi_ci;
pub mod midi_ci_property;
pub mod midi_network;
pub mod motion_raw;
pub mod device_registry;
pub mod script_value;
pub mod script_syntax;
pub mod script_export;
pub mod view;
pub mod tabs;
pub mod wl;
pub mod layers;
/// The retained layer tree (`Layer`, delegates, layout managers, constraints); see `ViewBase::layer`.
pub mod layer_tree {
    pub use bifrost_ui::layer_layout::*;
    pub use bifrost_ui::layer_tree::*;
}
pub mod document;
pub mod markup;
pub mod log;
pub mod signpost;
pub mod dispatch;
pub mod actor;
pub mod synchronization;
pub mod fswatch;
pub mod file_coordination;
pub mod file_version;
pub mod save_sync;
pub mod file_provider;
pub mod file_manager;
pub mod url;
pub mod url_background;
pub mod multipart;
pub mod social;
pub mod social_account;
pub mod url_components;
pub mod network_path;
pub mod network_connection;
pub mod network_listener;
pub mod network_browser;
pub mod http_cookies;
pub mod passkey_client_data;
pub mod credential_exchange;
pub mod web_extension;
pub mod web_auth;
pub mod bookmark_import;
pub mod browser_settings;
pub mod app_links;
pub mod reading_list;
pub mod content_rules;
pub mod browser_sheet;
pub mod http_auth;
pub mod dav_client;
pub mod dav_sync;
pub mod host_addressing;
pub mod ftp;
pub mod dns_service;
pub mod net_diagnostic;
pub mod stream;
pub mod websocket;
pub mod url_cache;
pub mod url_stream_task;
pub mod net_service;
pub mod service_discovery;
pub mod local_auth;
pub mod privacy_permissions;
pub mod screen_sample;
pub mod external_accessory;
pub mod accessory_access;
pub mod error;
pub mod coding;
pub mod peer_session;
pub mod peer_mesh;
pub mod peer_identity;
pub mod nearby_service;
pub mod nearby_browsing;
pub mod restoration;
pub mod scene_session;
pub mod image_source;
pub mod image_metadata;
pub mod image_animation;
pub mod picture_writer;
pub mod photo_library;
pub mod image_blur;
pub mod image_filter;
pub mod image_effects;
pub use restoration::RestorableState;
pub mod attributed;
pub mod text_formatting;
pub mod layer_kinds;
pub mod richlayout;
pub mod spellcheck;
pub mod translation;
pub mod wlan;
pub mod wlan_events;
pub mod wlan_keychain;
pub mod wifi_status;
pub mod system_prefs;
pub mod wifi_secrets;
pub mod net_totals;
pub mod notification;
pub mod notification_queue;
pub mod post_delivery;
pub mod system_sound;
pub mod workspace;
pub mod service;
pub mod search_ranking;
pub mod audio_devices;
pub mod disk_arbitration;
/// Aggregate devices (`AudioHardwareCreateAggregateDevice` keys) and clock domains, as a pure model.
pub mod audio_aggregate;
/// Process objects and process taps (`CATapDescription`), as pure models with their wire shapes.
pub mod audio_process;
/// Audio boxes (`AudioHardwareBox`) as a pure model: enclosures, their devices, enable and acquire state.
pub mod audio_box;
/// Virtual audio devices a driver plug-in would publish, as a pure model, and the driver status (no loader here).
pub mod audio_virtual;
pub mod speech_recognizer;
/// Stream formats (`StreamFormat`), the voice-take format (`pcm16`) and buffer durations (`audio_format::duration_ms`).
pub mod audio_format;
pub mod focus_filter;
pub mod pointer_focus;
pub mod gamepad_service;
pub mod distributed;
pub mod observable;
pub mod observable_object;
pub mod async_publisher;
pub mod object_graph;
pub mod ndarray;
pub mod inference;
pub mod music_understanding;
pub mod hotspot_configuration;
pub mod accessory_hotspot;
pub mod hotspot_helper;
pub mod vpn_configuration;
pub mod vpn_intents;
pub mod wifi_accessory_browser;
pub mod compression;
pub mod video_compression;
pub mod pixel_buffer;
pub mod display_link;
pub mod texture_cache;
pub use bifrost_ui::frame_clock;
pub mod gpu_layer;
pub mod offscreen;
/// Frame-rate ranges and transitions, shared with the shell.
pub use bifrost_ui::{frame_rate, transition};
/// The form factor the app runs on (desktop or touch tablet), shared with the shell through `bifrost_ui`.
pub use bifrost_ui::form_factor;
pub use bifrost_ui::form_factor::FormFactor;

/// Whether this app runs on a touch tablet: touch is the primary input and the on-screen keyboard can appear.
pub fn is_touch_tablet() -> bool {
    FormFactor::current().is_tablet()
}
pub use observable::{Cancellables, Observable, Subscription};
pub use notification::{DistributedNotificationCenter, Notification, NotificationCenter, ObserverToken};
/// `NativeCache` parity (see `docs/bifrost-ui.md`, "Cache").
pub mod cache {
    pub use bifrost_render::purgeable::{purge_all, Cache};
}
pub mod process_info;
pub mod power_service;
pub mod process_events;
pub mod event_client;
pub mod file_metadata;
pub mod quarantine;
pub mod binary_plist;
pub mod file_wrapper;
pub mod bookmark;
pub mod metadata_query;
pub mod file_stat;
pub mod volume;
pub mod identity;
pub mod reachability;
pub mod auth_events;
pub mod identity_events;
pub mod memory_events;
pub mod route_events;
pub mod bluetooth_events;
pub mod bluetooth_controller;
pub mod bluetooth_picker;
pub mod wireless_accessory_browser;
pub mod bluetooth_devices;
pub mod bluetooth_inquiry;
pub mod bluetooth_hid;
pub mod bluetooth_service;
pub mod ble_error;
pub mod central;
pub use process_info::{Activity, ActivityOptions, ProcessInfo, ThermalState};
/// Mach-style host statistics (`host_statistics64`, `task_info`, `os_proc_available_memory`); see `docs/bifrost-ui.md`.
pub use bifrost_core::host;
/// Raw accelerometer and gyroscope updates (the motion manager model) and the kernel sensor reader; see `docs/core-motion.md`.
pub use bifrost_core::{inertial_sensors, motion_manager};
mod runtime;
pub use kit::{Damage, Surface, Ui};
pub use runtime::{run, run_on, Ctx, Document, DocumentEvent, Input, OpenOptions};
pub use proto::ContextItem;

/// The audio service client (`audio::Client`): playback state and transport over `bifrost_proto::svc::audio`.
pub use springboard::svc::audio;

/// The AVAudioPlayer shape (`audio_player::AudioPlayer`): play, pause and stop, volume with fade, stereo pan, playback
/// rate, loops, seeking and rendered stereo blocks. Shared with the shell through `bifrost_ui`.
pub use bifrost_ui::audio_player;

/// The AVAudioEngine shape (`audio_engine::Engine`): the node graph of players, mixers, units and taps, offline or on
/// the audio device through the HAL.
pub use bifrost_audio_engine as audio_engine;

/// Microphone consent, the mute switch and the one recording owner (`microphone::Microphone`). The shell answers the
/// prompt; an app asks `begin` before it sends `RecordStart` to the audio service.
pub use bifrost_ui::mic_consent as microphone;

/// Speech model assets: installed locales, downloads and reservations (`speech_models::AssetInventory`).
pub use bifrost_ui::speech_assets as speech_models;

/// Speech recognition requests, tasks and authorization (`speech_recognition::Recognizer`, `Request`, `Task`).
pub use bifrost_ui::recognition_task as speech_recognition;

/// Calls: the call controller, transactions and observation (`calling::CallCenter`, `calling::Handle`). The shell
/// owns ringing and audio; an app drives calls through this model.
pub use bifrost_ui::call as calling;

/// The incoming-call card and its update (`calling_ui::CallUpdate`, `calling_ui::IncomingCard`).
pub use bifrost_ui::call_ui as calling_ui;

/// The default calling app and the apps registered for calling (`calling_apps::CallingApps`).
pub use bifrost_ui::call_apps as calling_apps;
/// Drawing models for the reference pencil-canvas framework: drawings, strokes, the lasso and the tool picker,
/// shared by the shell and apps through `bifrost_ui`.
pub use bifrost_ui::{drawing, lasso, tool_picker};

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
    /// True when the app takes touch events (`touches`) instead of the plain `Input::Touch` stream. The default keeps
    /// the plain stream, so existing apps are unchanged.
    fn wants_touch_events(&self) -> bool {
        false
    }
    /// One frame of touches (`touchesBegan`, `touchesMoved`, `touchesEnded`, `touchesCancelled`): every touch with its
    /// phase, location in surface pixels, previous location, tap count and optional force and radius. Called only when
    /// `wants_touch_events` is true. A plain pointer arrives as one indirect touch. True when the app needs a new frame.
    fn touches(&mut self, _ctx: &Ctx, _ev: &touch_events::TouchEvent) -> bool {
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
    /// Guided Access started or ended, or the user changed one of the app's restrictions (`UIGuidedAccessRestrictionDelegate`).
    /// True when the app needs a new frame. The default ignores it.
    fn guided_access(&mut self, _ctx: &Ctx, _change: guided_access::GuidedAccessChange) -> bool {
        false
    }
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
    /// Whether the app draws through `draw_canvas` (the GPU path, docs/graphics-architecture.md). Apps that return
    /// `true` are drawn with `draw_canvas` on the direct path and with `draw_canvas` on a CPU `Surface` (which
    /// implements `Canvas`) on the shared-memory path, so one drawing routine serves both. The default `false` keeps
    /// `draw`.
    fn draws_canvas(&self) -> bool {
        false
    }
    /// Draws through the drawing vocabulary of `bifrost_render::canvas::Canvas` and returns what changed. Called
    /// instead of `draw` when `draws_canvas` is `true`.
    fn draw_canvas(&mut self, ctx: &Ctx, canvas: &mut dyn bifrost_render::canvas::Canvas, t: f64) -> Damage {
        let _ = (ctx, t);
        let (w, h) = canvas.size();
        Damage::new(w as usize, h as usize)
    }
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
    /// The reply to a `service::call` with id `request`. Return true when the app took it (and a frame is needed);
    /// false keeps it for `service::take_reply`.
    fn service_reply(&mut self, _ctx: &Ctx, _request: u32, _result: &Result<bifrost_core::json::Json, String>) -> bool {
        false
    }
    /// An event pushed by the shell service `service`. Return true when a frame is needed.
    fn service_event(&mut self, _ctx: &Ctx, _service: &str, _event: &str, _body: &bifrost_core::json::Json) -> bool {
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

pub mod encryption_preference;
pub mod graphics_context;
pub mod graphics_stroke;
pub mod graphics_gradient;
pub mod graphics_color;
pub mod graphics_pdf;
pub mod graphics_imageio;
pub mod color_list;
pub mod image_object;
pub mod gpu_device;
pub mod drawable_pool;
pub mod gpu_pass;
pub mod biquad;
pub mod extent_map;
