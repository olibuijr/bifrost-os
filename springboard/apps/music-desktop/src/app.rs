//! Music in a Golden Gate full-size window: the source list at the left, the unified toolbar over the content pane,
//! and the pane itself (`music::Music`). Playback goes through the audio service (`is.olibuijr.audio`).

use crate::music;
use crate::music::chrome::{BAR_SEARCH, BAR_TITLE, SIDE_TAB};
use crate::music::menus::{self, Command};
use crate::music::miniplayer::{self, Layout, Part};
use crate::music::system;
use bifrost_app::app_windows::{is_close_window_key, WindowId, WindowSpec};
use bifrost_app::audio::Client;
use bifrost_app::audio_devices::{self, Change};
use bifrost_app::control::{RpcError, INVALID_PARAMS};
use bifrost_app::intents::{self, Intent, IntentResult};
use bifrost_app::controls::nav::ToolbarLayout;
use bifrost_app::controls::sidebar::SidebarItem;
use bifrost_app::controls::{Node, Rect, Role, TextField};
use bifrost_app::dock_tile;
use bifrost_app::kit::ui::{Ui, Weight, LABEL, MAC_TITLE3_PT, SEPARATOR_C};
use bifrost_app::kit::{Damage, Event, Surface};
use bifrost_app::layout::{SIDEBAR_FRACTION, SIDEBAR_MAX_PT, SIDEBAR_MIN_PT, UNIFIED_BAR_PT};
use bifrost_app::now_playing::{self, CommandState, PlaybackState, RemoteCommand};
use bifrost_app::widgets;
use bifrost_app::windows::{self, Level};
use bifrost_app::{App, AppMenu, Ctx, DocumentEvent, Input, KeyInput, OpenOptions, TouchKind};
use springboard::shell::music::nowplaying::Target as Transport;
use bifrost_core::json::Json;
use springboard::shell::music::radio::Http;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

/// The source list starts under the traffic lights, like Files's.
const SIDEBAR_SCROLL: f32 = -(UNIFIED_BAR_PT + 4.);
/// The open search field's width, in points.
const SEARCH_W: f32 = 180.;
/// Accessibility ids of the Play and Shuffle buttons on the album and playlist pages (sidebar rows use 100 up to these).
const NODE_PLAY: u64 = 9000;
const NODE_SHUFFLE: u64 = 9001;
/// The file types File > Import offers: the formats the audio service plays, and MIDI files.
const IMPORT_TYPES: [&str; 10] = ["mp3", "m4a", "m4b", "aac", "flac", "ogg", "oga", "wav", "mid", "midi"];

/// What a pointer went down on.
#[derive(Clone, Copy, PartialEq)]
enum Down {
    Sidebar,
    Bar,
    Pane,
}

/// Music in a Golden Gate full-size window: the source list at the left, the unified toolbar over the
/// content pane, and the pane itself (`music::Music`, in pane-local pixels).
pub struct MusicApp {
    app: Option<music::Music>,
    keyboard: bool,
    /// window size, points
    w: f32,
    h: f32,
    title_w: f32,
    down: Option<Down>,
    /// the toolbar as last drawn, and its buttons as controls (window points)
    placed: Option<ToolbarLayout>,
    nodes: Vec<Node>,
    /// the clock the publish rules use (milliseconds since the app started)
    epoch: Instant,
    /// the Now Playing session as last published to Control Center
    published: Option<Published>,
    /// the `audio-devices` list request, until its reply arrives
    devices_request: Option<u32>,
    /// the default output device id (`headphones`, `bluetooth`, `speaker`), as last reported
    output: Option<String>,
    /// the `audio-devices` list request made for the output picker, until its reply arrives
    picker_request: Option<u32>,
    /// the output devices (id, name) as last listed, for Controls > Output
    outputs: Vec<(String, String)>,
    /// the open panel of File > Import, until the files arrive
    import_request: Option<u32>,
    /// whether the Dock tile shows the library scan's progress
    dock_busy: bool,
    /// the MiniPlayer window, while it is open
    mini: Option<WindowId>,
    /// Window > Keep MiniPlayer on Top: the MiniPlayer floats above the other windows
    mini_on_top: bool,
    /// the MiniPlayer part a touch went down on, until it lifts
    mini_down: Option<Part>,
}

/// What the last Now Playing publish carried: the key without the position, the position, when it went out (app clock,
/// ms) and whether it was playing.
struct Published {
    key: String,
    position_ms: f64,
    at: f64,
    playing: bool,
}

/// The devices that can play, as (id, name), in the order the shell lists them.
fn output_devices(listing: &audio_devices::Listing) -> Vec<(String, String)> {
    listing.devices.iter().filter(|d| d.can_output && !d.hidden).map(|d| (d.id.clone(), d.name.clone())).collect()
}

impl MusicApp {
    pub fn new() -> Self {
        MusicApp {
            app: None,
            keyboard: false,
            w: 0.,
            h: 0.,
            title_w: 40.,
            down: None,
            placed: None,
            nodes: vec![],
            epoch: Instant::now(),
            published: None,
            devices_request: None,
            output: None,
            picker_request: None,
            outputs: Vec::new(),
            import_request: None,
            dock_busy: false,
            mini: None,
            mini_on_top: false,
            mini_down: None,
        }
    }

    /// The app clock in milliseconds.
    fn clock(&self) -> f64 {
        self.epoch.elapsed().as_secs_f64() * 1000.0
    }

    /// Keeps the Dock tile in step with the library scan. Called on each tick; the call goes out only when it changes.
    fn dock_step(&mut self, ctx: &Ctx, busy: bool) {
        if busy == self.dock_busy {
            return;
        }
        self.dock_busy = busy;
        if busy {
            dock_tile::set_indeterminate(ctx);
        } else {
            dock_tile::set_progress(ctx, None);
        }
    }

    /// Publishes the session to Control Center when its content, state or commands changed, or when a seek moved the
    /// position. A stopped session (no song) is withdrawn. Nothing is sent while playback runs normally.
    fn publish_step(
        published: &mut Option<Published>,
        ctx: &Ctx,
        session: Option<&now_playing::MediaSession>,
        queue: &[now_playing::QueueEntry],
        artwork: Option<&now_playing::ArtworkImage>,
        now: f64,
    ) {
        let body = session.and_then(|s| now_playing::publish_queue_body(s, queue));
        let (Some(session), Some(body)) = (session, body) else {
            if published.take().is_some() {
                now_playing::clear(ctx);
            }
            return;
        };
        let snap = session.snapshot();
        let position = snap.position_ms as f64;
        let playing = matches!(snap.state, PlaybackState::Playing);
        let seekable = matches!(session.command_state(RemoteCommand::SeekTo), CommandState::Enabled);
        // a new cover is a change the tile must show at once, like a new song
        let key = format!("{}|art:{:x}", system::publish_key(&body), artwork.map_or(0, now_playing::ArtworkImage::fingerprint));
        let due = match published.as_ref() {
            None => true,
            Some(p) => p.key != key || (seekable && system::seeked(p.position_ms, p.at, now, p.playing, position)),
        };
        if due {
            let _ = now_playing::publish_with_artwork(ctx, session, queue, artwork);
            *published = Some(Published { key, position_ms: position, at: now, playing });
        }
    }

    /// The service events Music acts on: a Control Center transport command, and a route that left headphones or
    /// Bluetooth for the speaker (Music pauses then).
    fn route_changed(&mut self, output: Option<String>) -> bool {
        let lost = system::output_lost(self.output.as_deref(), output.as_deref());
        self.output = output;
        lost && self.app.as_mut().is_some_and(|m| m.remote_command("pause"))
    }

    /// A transport command from Control Center. True when the app acted on it.
    fn system_command(&mut self, cmd: now_playing::Command) -> bool {
        let Some(m) = self.app.as_mut() else { return false };
        match cmd {
            now_playing::Command::Seek(ms) => m.remote_seek(ms),
            other => system::transport_name(&other).is_some_and(|name| m.remote_command(name)),
        }
    }

    /// Sidebar width in points: a fraction of the window within the system limits.
    fn side(&self) -> f32 {
        (self.w * SIDEBAR_FRACTION).clamp(SIDEBAR_MIN_PT, SIDEBAR_MAX_PT).round()
    }

    /// Sidebar width in pixels: where the content pane starts.
    fn x0(&self, ui: &Ui) -> usize {
        ui.pt(self.side()).round() as usize
    }

    fn bar_bounds(&self) -> Rect {
        let x = self.side() + 12.;
        let h = widgets::CAPSULE_PT;
        Rect { x, y: (UNIFIED_BAR_PT - h) / 2., w: (self.w - x - 16.).max(0.), h }
    }

    /// The search field is always open on the Songs page, as Music's toolbar field is; it widens only while typing.
    fn search_w(_: &music::Music) -> f32 {
        SEARCH_W
    }

    fn layout(&self) -> Option<ToolbarLayout> {
        let m = self.app.as_ref()?;
        Some(m.toolbar(self.title_w, Self::search_w(m)).layout_with(self.bar_bounds(), |_| widgets::CAPSULE_PT, widgets::CAPSULE_PT, widgets::TOOLBAR_GAP_PT))
    }

    /// A click in the toolbar band (points).
    fn bar_click(&mut self, x: f32, y: f32, t: f64) {
        let Some(placed) = self.placed.clone() else { return };
        let Some(m) = &mut self.app else { return };
        match placed.hit(x, y) {
            Some(BAR_SEARCH) if m.searching() => {
                if let Some(r) = placed.rect_of(BAR_SEARCH) {
                    if bifrost_app::controls::search::hits_clear(r, !m.search_text().is_empty(), x, y) {
                        m.clear_search();
                    }
                }
            }
            Some(id) => m.bar_press(id, t),
            None => {}
        }
    }

    fn sidebar_click(&mut self, x: f32, y: f32, t: f64) {
        let bounds = Rect { x: 0., y: 0., w: self.side(), h: self.h };
        let Some(m) = &mut self.app else { return };
        if let Some(id) = m.sidebar().hit(bounds, SIDEBAR_SCROLL, x, y) {
            m.pick_sidebar(id, t);
        }
    }

    /// An accessibility press on a node the window published (`App::controls`): Play or Shuffle, a sidebar row,
    /// or a toolbar button. True when a frame is needed.
    fn press_node(&mut self, id: u64) -> bool {
        match id {
            NODE_PLAY | NODE_SHUFFLE => {
                if let Some(m) = &mut self.app {
                    m.press_action(id == NODE_SHUFFLE, 0.);
                }
                true
            }
            id if (SIDE_TAB as u64..NODE_PLAY).contains(&id) => {
                if let Some(m) = &mut self.app {
                    m.pick_sidebar(id as u32, 0.);
                }
                true
            }
            id => {
                let Some(r) = self.layout().and_then(|l| l.rect_of(id as u32)) else { return false };
                self.bar_click(r.x + r.w / 2., r.y + r.h / 2., 0.);
                true
            }
        }
    }

    /// Draws the source list and the toolbar around the pane and records the controls the window publishes.
    fn chrome(&mut self, s: &mut Surface, ui: &Ui) {
        let Some(m) = &self.app else { return };
        let side = self.side();
        let list = m.sidebar();
        widgets::source_list(s, ui, Rect { x: 0., y: 0., w: side, h: self.h }, &list, SIDEBAR_SCROLL, None);
        let mut nodes: Vec<Node> = list
            .items
            .iter()
            .enumerate()
            .filter_map(|(i, item)| {
                let SidebarItem::Row { id, label, enabled, .. } = item else { return None };
                Some(Node {
                    id: *id as u64,
                    role: Role::Row,
                    label: label.clone(),
                    value: if list.selected() == Some(*id) { "Selected".to_string() } else { String::new() },
                    bounds: list.item_rect(i, Rect { x: 0., y: 0., w: side, h: self.h }, SIDEBAR_SCROLL),
                    enabled: *enabled,
                })
            })
            .collect();
        s.fill_rect(self.x0(ui).saturating_sub(1), 0, 1, s.h, SEPARATOR_C.now());
        if m.overlay() {
            self.placed = None;
            self.nodes.clear();
            return;
        }
        let title = m.title();
        let others = 6. * widgets::CAPSULE_PT + SEARCH_W + 4. * widgets::TOOLBAR_GAP_PT;
        let room = (self.bar_bounds().w - others).max(40.);
        self.title_w = (ui.measure(&title, MAC_TITLE3_PT, Weight::Bold) / ui.pt(1.) + 4.).clamp(40., room);
        widgets::toolbar_band(s, ui, Rect { x: side, y: 0., w: self.w - side, h: UNIFIED_BAR_PT });
        let bar = m.toolbar(self.title_w, Self::search_w(m));
        let placed = widgets::toolbar(s, ui, self.bar_bounds(), &bar, None);
        if let Some(r) = placed.rect_of(BAR_TITLE) {
            let im = ui.text(&ui.fit(&title, MAC_TITLE3_PT, ui.pt(r.w)), MAC_TITLE3_PT, Weight::Bold, LABEL.now());
            s.blit(&im, ui.pt(r.x) as i32, ui.pt(r.y + r.h / 2.) as i32 - im.h as i32 / 2, 255, 255);
        }
        let mut field = TextField::new();
        field.push_str(m.search_text());
        // The search field is a kit TextField that records its own accessibility node (`widgets::search_field`),
        // so the toolbar publishes only its buttons here.
        if let Some(r) = placed.rect_of(BAR_SEARCH) {
            let focused = m.searching();
            widgets::search_field(s, ui, &field, Rect { x: r.x, y: r.y + 4., w: r.w, h: r.h - 8. }, "Search Songs", focused);
        }
        nodes.extend(
            placed
                .visible
                .iter()
                .filter(|(id, _)| *id != BAR_TITLE && *id != BAR_SEARCH)
                .filter_map(|(id, r)| {
                    let item = bar.items.iter().find(|i| i.id == *id)?;
                    Some(Node { id: *id as u64, role: Role::Button, label: item.label.clone(), value: String::new(), bounds: *r, enabled: item.enabled })
                }),
        );
        let x0 = self.x0(ui) as f32;
        for (label, r) in m.action_buttons(ui) {
            nodes.push(Node {
                id: if label == "Play" { NODE_PLAY } else { NODE_SHUFFLE },
                role: Role::Button,
                label: label.to_string(),
                value: String::new(),
                bounds: Rect { x: (x0 + r.x) / ui.s, y: r.y / ui.s, w: r.w / ui.s, h: r.h / ui.s },
                enabled: true,
            });
        }
        self.nodes = nodes;
        self.placed = Some(placed);
    }

    fn sync_keyboard(&mut self, ctx: &Ctx) {
        let want = self.app.as_ref().is_some_and(|m| m.wants_keyboard());
        if want != self.keyboard {
            self.keyboard = want;
            ctx.wants_keyboard(want);
        }
    }

    /// Keeps Control Center's Now Playing tile and the Dock tile in step with the app. Each call goes out only when
    /// something it shows changed.
    fn sync_system(&mut self, ctx: &Ctx) {
        let busy = self.app.as_ref().is_some_and(|m| m.is_scanning());
        self.dock_step(ctx, busy);
        let now = self.clock();
        if let Some(m) = self.app.as_mut() {
            m.refresh_now_playing_artwork();
        }
        let session = self.app.as_ref().and_then(|m| m.session());
        let queue = if session.is_some() { self.app.as_ref().map(|m| m.up_next_entries()).unwrap_or_default() } else { Vec::new() };
        let artwork = self.app.as_ref().and_then(|m| m.now_playing_artwork());
        Self::publish_step(&mut self.published, ctx, session, &queue, artwork, now);
    }
}

fn shell_dir() -> PathBuf {
    PathBuf::from(std::env::var("BIFROST_SERVICE_ROOT").unwrap_or_else(|_| "/var/lib/springboard".into()))
}

impl MusicApp {
    /// Window > MiniPlayer: opens the MiniPlayer window, or closes it when it is open.
    fn toggle_mini(&mut self, ctx: &Ctx) {
        if self.mini.is_some() {
            self.close_mini(ctx);
            return;
        }
        if let Ok(id) = ctx.open_window(WindowSpec::new("MiniPlayer", miniplayer::WIDTH_PT, miniplayer::HEIGHT_PT)) {
            self.mini = Some(id);
            self.apply_mini_level(ctx);
        }
    }

    fn close_mini(&mut self, ctx: &Ctx) {
        if let Some(id) = self.mini.take() {
            let _ = ctx.close_window(id);
        }
        self.mini_down = None;
    }

    /// Floats the MiniPlayer above the other windows, or lets it sit in the normal stack, as the Window menu says.
    fn apply_mini_level(&self, ctx: &Ctx) {
        if let Some(id) = self.mini {
            let level = if self.mini_on_top { Level::Floating } else { Level::Normal };
            windows::set_window_level(ctx, id, level);
        }
    }

    /// A touch in the MiniPlayer. The scrubber and the volume bar follow the finger; a transport button acts when the
    /// finger lifts over the part it went down on.
    fn mini_touch(&mut self, ctx: &Ctx, kind: TouchKind, x: f32, y: f32) -> bool {
        let lay = miniplayer::layout(miniplayer::size_px(ctx.ui.s).0, ctx.ui.s);
        if kind == TouchKind::Down {
            self.mini_down = lay.hit(x, y);
        }
        let down = self.mini_down;
        if matches!(kind, TouchKind::Up | TouchKind::Cancel) {
            self.mini_down = None;
        }
        let Some(m) = &mut self.app else { return true };
        match (kind, down) {
            (TouchKind::Down | TouchKind::Move, Some(Part::Scrubber)) => m.seek_to_fraction(Layout::fraction(lay.scrub, x)),
            (TouchKind::Down | TouchKind::Move, Some(Part::Volume)) => m.set_volume_fraction(Layout::fraction(lay.volume, x)),
            (TouchKind::Up, Some(part)) if lay.hit(x, y) == Some(part) => match part {
                Part::PlayPause => m.control(Transport::Play),
                Part::Prev => m.control(Transport::Prev),
                Part::Next => m.control(Transport::Next),
                Part::Scrubber | Part::Volume => {}
            },
            _ => {}
        }
        true
    }
}

/// App Intents that drive playback (the audio intents of the desktop's Shortcuts and Search).
const AUDIO_PLAY_PAUSE: &str = "play-pause";
const AUDIO_NEXT: &str = "next-track";
const AUDIO_PREVIOUS: &str = "previous-track";

/// The transport control an audio intent id runs, with the dialog it answers with. `None` for any other id.
fn audio_transport(id: &str) -> Option<(Transport, &'static str)> {
    match id {
        AUDIO_PLAY_PAUSE => Some((Transport::Play, "Playback toggled")),
        AUDIO_NEXT => Some((Transport::Next, "Skipped to the next track")),
        AUDIO_PREVIOUS => Some((Transport::Prev, "Went back to the previous track")),
        _ => None,
    }
}

impl App for MusicApp {
    fn id(&self) -> &str {
        "is.olibuijr.music.desktop"
    }

    fn configure(&mut self, ctx: &Ctx) {
        ctx.enable_sudden_termination(); // the queue and playlists are written when they change
        self.w = ctx.cfg.w as f32 / ctx.ui.s;
        self.h = ctx.cfg.h as f32 / ctx.ui.s;
        let (w, h) = (ctx.cfg.w as i32 - self.x0(&ctx.ui) as i32, ctx.cfg.h as i32);
        if let Some(m) = &mut self.app {
            m.set_size(w, h);
            return;
        }
        let roots = vec![PathBuf::from(std::env::var("HOME").unwrap_or_default()).join("Music")];
        let load = music::library::library_load_options(&ctx.defaults());
        let Ok(mut m) = music::Music::new(w, h, roots, load, Client::for_app(&shell_dir())) else { return };
        m.enable_library_store(ctx.data_dir.join("Music"));
        m.enable_radio(ctx.data_dir.join("Radio"), Arc::new(Http::default()));
        self.app = Some(m);
        // the route as it stands now; later changes arrive as `audio-devices` events
        self.devices_request = Some(audio_devices::list(ctx));
    }

    fn service_reply(&mut self, _ctx: &Ctx, request: u32, result: &Result<Json, String>) -> bool {
        if self.picker_request == Some(request) {
            self.picker_request = None;
            // the output devices for the route button's menu; an error leaves the button without a menu
            if let Ok(listing) = result.as_ref().map_err(|_| ()).and_then(|v| audio_devices::decode_list(v).map_err(|_| ())) {
                let devices: Vec<(String, String)> = output_devices(&listing);
                self.outputs = devices.clone();
                self.output = listing.output.clone();
                if let Some(m) = &mut self.app {
                    m.show_outputs(devices, listing.output);
                }
            }
            return true;
        }
        if self.devices_request != Some(request) {
            return false;
        }
        self.devices_request = None;
        // an error (an older shell has no `audio-devices` service) leaves the route unknown: nothing pauses
        if let Ok(listing) = result.as_ref().map_err(|_| ()).and_then(|v| audio_devices::decode_list(v).map_err(|_| ())) {
            self.outputs = output_devices(&listing);
            self.output = listing.output;
        }
        true
    }

    fn intents(&self) -> Vec<Intent> {
        vec![
            Intent::new(AUDIO_PLAY_PAUSE, "Play or Pause", "Starts or pauses playback in Music.").shortcut(),
            Intent::new(AUDIO_NEXT, "Next Track", "Skips to the next track in the queue.").shortcut(),
            Intent::new(AUDIO_PREVIOUS, "Previous Track", "Goes back to the previous track in the queue.").shortcut(),
        ]
    }

    fn perform_intent(&mut self, _ctx: Option<&Ctx>, _data_dir: &Path, id: &str, _params: &Json) -> Result<IntentResult, RpcError> {
        let Some((target, done)) = audio_transport(id) else { return Err(RpcError::new(INVALID_PARAMS, format!("no intent {id}"))) };
        let Some(m) = self.app.as_mut() else { return Err(intents::failed("Music is not running yet")) };
        m.control(target);
        Ok(IntentResult::dialog(done))
    }

    fn service_event(&mut self, ctx: &Ctx, service: &str, event: &str, body: &Json) -> bool {
        if let Some(index) = now_playing::queue_pick(service, event, body) {
            return self.app.as_mut().is_some_and(|m| m.remote_queue_pick(index));
        }
        if let Some(cmd) = now_playing::command_from_event(service, event, body) {
            return self.system_command(cmd);
        }
        if service == audio_devices::SERVICE {
            match audio_devices::decode_event(event, body) {
                Some(Change::Route { output, .. }) => return self.route_changed(output),
                // a device came or went: list them again for Controls > Output
                Some(Change::Devices(_)) if self.devices_request.is_none() => self.devices_request = Some(audio_devices::list(ctx)),
                _ => {}
            }
        }
        false
    }

    fn event(&mut self, ctx: &Ctx, ev: Input) -> bool {
        let Some(overlay) = self.app.as_ref().map(|m| m.overlay()) else { return false };
        let changed = match ev {
            Input::Touch { kind, x, y, t, .. } => {
                let (xp, yp) = (x / ctx.ui.s, y / ctx.ui.s);
                if kind == TouchKind::Down {
                    self.down = Some(if overlay || xp >= self.side() && yp >= UNIFIED_BAR_PT {
                        Down::Pane
                    } else if xp < self.side() {
                        Down::Sidebar
                    } else {
                        Down::Bar
                    });
                }
                let down = self.down;
                if matches!(kind, TouchKind::Up | TouchKind::Cancel) {
                    self.down = None;
                }
                match down {
                    Some(Down::Pane) | None => {
                        let x = x - self.x0(&ctx.ui) as f32;
                        let e = match kind {
                            TouchKind::Down => Event::Down(x, y),
                            TouchKind::Move => Event::Move(x, y),
                            TouchKind::Up | TouchKind::Cancel => Event::Up(x, y),
                        };
                        if let Some(m) = &mut self.app {
                            m.touch(&ctx.ui, e, t);
                        }
                    }
                    Some(Down::Sidebar) if kind == TouchKind::Up => self.sidebar_click(xp, yp, t),
                    Some(Down::Bar) if kind == TouchKind::Up => self.bar_click(xp, yp, t),
                    _ => {}
                }
                true
            }
            Input::Activate(id) => self.press_node(id as u64),
            Input::Key(KeyInput::Code(1 | 27)) => self.app.as_mut().is_some_and(|m| m.cancel(0.)),
            Input::Key(KeyInput::Text(s)) => {
                self.app.as_mut().unwrap().type_text(&s);
                true
            }
            Input::Key(KeyInput::Code(8)) => {
                self.app.as_mut().unwrap().backspace();
                true
            }
            Input::Key(KeyInput::Code(13)) => {
                self.app.as_mut().unwrap().type_text("\n");
                true
            }
            // Cmd+A selects the whole playlist name being typed; any other chord is not the music app's
            Input::Key(k) if k.as_chord() == Some((30, bifrost_app::proto::CHORD_META)) => self.app.as_mut().is_some_and(|m| m.select_all_name()),
            Input::Key(KeyInput::Code(_)) => false,
            Input::Open(url) => {
                let path = url.strip_prefix("file://").unwrap_or(&url);
                self.app.as_mut().unwrap().play_path(Path::new(path));
                true
            }
            _ => false,
        };
        self.sync_keyboard(ctx);
        changed
    }

    fn main_menu(&self, _ctx: &Ctx) -> Vec<AppMenu> {
        // With no window there is nothing to act on, so there are no menus at all.
        let Some(m) = self.app.as_ref() else { return Vec::new() };
        let mut state = m.menu_state();
        state.outputs = self.outputs.iter().map(|(_, name)| name.clone()).collect();
        state.output = self.output.as_ref().and_then(|id| self.outputs.iter().position(|(d, _)| d == id));
        let mut menus = menus::build(&state);
        menus.push(menus::window_menu(self.mini.is_some(), self.mini_on_top));
        menus
    }

    fn window_event(&mut self, ctx: &Ctx, window: WindowId, ev: Input) -> bool {
        if window.is_main() {
            return self.event(ctx, ev);
        }
        if self.mini != Some(window) {
            return false;
        }
        match ev {
            // the window's touches arrive in window points (`WindowTouch`); the MiniPlayer layout is in px
            Input::Touch { kind, x, y, .. } => {
                let k = ctx.ui.pt(1.0);
                self.mini_touch(ctx, kind, x * k, y * k)
            }
            Input::Key(k) if is_close_window_key(&k) => {
                self.close_mini(ctx);
                true
            }
            _ => false,
        }
    }

    fn draw_window(&mut self, ctx: &Ctx, window: WindowId, s: &mut Surface, _t: f64) -> Damage {
        let mut d = Damage::new(s.w, s.h);
        if window.is_main() || self.mini != Some(window) {
            return d;
        }
        let pressed = self.mini_down;
        let lay = miniplayer::layout(s.w as f32, ctx.ui.s);
        if let Some(m) = &mut self.app {
            let info = m.mini_info();
            let art = m.mini_art();
            miniplayer::draw(s, &ctx.ui, &lay, &info, art.as_ref(), pressed);
        }
        d.all();
        d
    }

    fn window_closed(&mut self, _ctx: &Ctx, window: WindowId) -> bool {
        if self.mini == Some(window) {
            self.mini = None;
            self.mini_down = None;
            return true;
        }
        false
    }

    fn menu_action(&mut self, ctx: &Ctx, id: u32) -> bool {
        let Some(cmd) = Command::from_id(id) else { return false };
        match cmd {
            Command::MiniPlayer => {
                self.toggle_mini(ctx);
                return true;
            }
            Command::KeepMiniOnTop => {
                self.mini_on_top = !self.mini_on_top;
                self.apply_mini_level(ctx);
                return true;
            }
            Command::Output(i) => {
                // the shell answers with a route change, which updates `output`
                let Some((id, _)) = self.outputs.get(i) else { return false };
                let _ = audio_devices::set_default(ctx, audio_devices::Role::Output, id);
                self.output = Some(id.clone());
                return true;
            }
            _ => {}
        }
        if cmd == Command::Import {
            // the panel answers through `document`
            let options = OpenOptions { title: "Import to Music".into(), filters: IMPORT_TYPES.iter().map(|t| t.to_string()).collect(), multiple: true, ..OpenOptions::default() };
            self.import_request = Some(ctx.open_panel(&options));
            return false;
        }
        let acted = self.app.as_mut().is_some_and(|m| m.run_command(cmd));
        self.sync_keyboard(ctx);
        acted
    }

    fn document(&mut self, ctx: &Ctx, ev: DocumentEvent) -> bool {
        match ev {
            DocumentEvent::OpenedMany { request, docs } if self.import_request == Some(request) => {
                self.import_request = None;
                let mut copied = 0;
                for mut doc in docs {
                    if let Some(m) = &mut self.app {
                        if m.import_song(&doc.name, &mut doc.file) {
                            copied += 1;
                        }
                    }
                    // one import is one use of the file: the app keeps no grant to it
                    ctx.release_document(doc.grant);
                }
                if let Some(m) = &mut self.app {
                    m.imports_done(copied);
                }
                true
            }
            DocumentEvent::Failed { request, .. } if self.import_request == Some(request) => {
                self.import_request = None;
                false
            }
            _ => false,
        }
    }

    fn tick(&mut self, ctx: &Ctx, t: f64) -> bool {
        let Some(m) = &mut self.app else { return false };
        m.pump(&ctx.ui);
        let due = m.tick(&ctx.ui, t);
        // the route button asks for the output devices; a device the user picked becomes the default output
        if m.take_output_request() {
            self.picker_request = Some(audio_devices::list(ctx));
        }
        if let Some(id) = m.take_output_choice() {
            let _ = audio_devices::set_default(ctx, audio_devices::Role::Output, &id);
        }
        self.sync_keyboard(ctx);
        self.sync_system(ctx);
        // the MiniPlayer shows the position, so its window is redrawn with the main one while it is open
        due || self.mini.is_some()
    }

    fn controls(&self, _: &Ctx) -> Vec<Node> {
        self.nodes.clone()
    }

    fn next_wake(&self, t: f64) -> Option<f64> {
        self.app.as_ref().and_then(|m| m.next_wake(t))
    }

    fn draw(&mut self, ctx: &Ctx, s: &mut Surface, t: f64) -> Damage {
        let x0 = self.x0(&ctx.ui).min(s.w);
        if let Some(m) = &mut self.app {
            let mut pane = Surface { w: s.w - x0, h: s.h, stride: s.stride, px: &mut s.px[x0..], y0: s.y0 };
            m.draw(&mut pane, &ctx.ui, t);
        }
        self.chrome(s, &ctx.ui);
        let mut d = Damage::new(s.w, s.h);
        d.all();
        d
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::music::nowplay;
    use bifrost_app::kit::Font;
    use bifrost_app::now_playing::{CommandState, MediaContent, MediaDuration, MediaType, PlaybackState};
    use bifrost_app::proto::{Configure, ToShell};

    fn test_ctx() -> Ctx {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets");
        let data: &'static [u8] = Box::leak(std::fs::read(format!("{dir}/SFUI.ttf")).unwrap().into_boxed_slice());
        let cfg = Configure { w: 100, h: 100, scale: 1.0, safe_top: 0.0, safe_bottom: 0.0, dark: true, text_scale: 1.0, reduce_motion: false };
        Ctx::testing(bifrost_app::Ui::new(Font::parse(data).unwrap(), 1.0), cfg)
    }

    /// The service calls sent since the last call, as (service, method, body).
    fn calls(ctx: &Ctx) -> Vec<(String, String, String)> {
        ctx.take_sent()
            .into_iter()
            .filter_map(|m| match m {
                ToShell::ServiceCall { service, method, body, .. } => Some((service, method, body)),
                _ => None,
            })
            .collect()
    }

    fn song_session(position_ms: u64) -> now_playing::MediaSession {
        let mut s = nowplay::session().unwrap();
        let c = MediaContent::new("song:1", "Song", MediaType::Audio, MediaDuration::Finite(180.0)).ok().unwrap();
        s.set_content(Some(c));
        s.set_playback(PlaybackState::Playing, position_ms);
        s.set_command(RemoteCommand::SeekTo, CommandState::Enabled);
        s
    }

    #[test]
    fn the_dock_shows_progress_only_while_the_library_scan_runs() {
        let ctx = test_ctx();
        let mut app = MusicApp::new();
        app.dock_step(&ctx, true);
        app.dock_step(&ctx, true);
        app.dock_step(&ctx, false);
        assert_eq!(
            calls(&ctx),
            vec![
                (dock_tile::SERVICE.to_string(), "progress".to_string(), dock_tile::indeterminate_params().encode()),
                (dock_tile::SERVICE.to_string(), "progress".to_string(), dock_tile::progress_params(None).encode()),
            ]
        );
    }

    #[test]
    fn a_cover_rides_the_publish_and_a_new_cover_republishes() {
        let ctx = test_ctx();
        let mut published = None;
        let session = song_session(5_000);
        let red = now_playing::ArtworkImage::new(1, 1, vec![255, 0, 0, 255]).unwrap();
        let blue = now_playing::ArtworkImage::new(1, 1, vec![0, 0, 255, 255]).unwrap();
        MusicApp::publish_step(&mut published, &ctx, Some(&session), &[], None, 0.0);
        let sent = calls(&ctx);
        assert_eq!(sent.len(), 1);
        assert!(!sent[0].2.contains("\"artwork\""), "no cover, no artwork field: the shell keeps its glyph");
        // the cover loads: one more publish, now with the pixels
        MusicApp::publish_step(&mut published, &ctx, Some(&session), &[], Some(&red), 1_000.0);
        let sent = calls(&ctx);
        assert_eq!(sent.len(), 1);
        assert!(sent[0].2.contains("\"artwork\":{\"w\":1,\"h\":1,\"rgba\":\"/wAA/w==\"}"));
        // the same cover again sends nothing, a different one does
        MusicApp::publish_step(&mut published, &ctx, Some(&session), &[], Some(&red), 2_000.0);
        assert!(calls(&ctx).is_empty());
        MusicApp::publish_step(&mut published, &ctx, Some(&session), &[], Some(&blue), 3_000.0);
        assert_eq!(calls(&ctx).len(), 1);
    }

    #[test]
    fn the_now_playing_tile_is_published_on_change_and_on_a_seek_only() {
        let ctx = test_ctx();
        let mut published = None;
        let first = song_session(5_000);
        MusicApp::publish_step(&mut published, &ctx, Some(&first), &[], None, 0.0);
        // normal playback a second later: the position is where it should be, nothing is sent
        MusicApp::publish_step(&mut published, &ctx, Some(&first), &[], None, 1_000.0);
        let sent = calls(&ctx);
        assert_eq!(sent.len(), 1);
        assert_eq!((sent[0].0.as_str(), sent[0].1.as_str()), (now_playing::SERVICE, "publish"));
        assert!(sent[0].2.contains("\"title\":\"Song\""));
        // a scrub to 90 s
        let scrubbed = song_session(90_000);
        MusicApp::publish_step(&mut published, &ctx, Some(&scrubbed), &[], None, 1_000.0);
        let sent = calls(&ctx);
        assert_eq!(sent.len(), 1);
        assert!(sent[0].2.contains("90000"));
        // no song left: the session is withdrawn once
        MusicApp::publish_step(&mut published, &ctx, None, &[], None, 2_000.0);
        MusicApp::publish_step(&mut published, &ctx, None, &[], None, 3_000.0);
        assert_eq!(calls(&ctx), vec![(now_playing::SERVICE.to_string(), "clear".to_string(), "null".to_string())]);
    }

    #[test]
    fn a_failed_audio_devices_list_leaves_the_route_unknown_and_frees_the_request() {
        let ctx = test_ctx();
        let mut app = MusicApp::new();
        app.devices_request = Some(7);
        assert!(app.service_reply(&ctx, 7, &Err("no service audio-devices".into())));
        assert_eq!(app.devices_request, None);
        assert_eq!(app.output, None);
        assert!(!app.service_reply(&ctx, 8, &Ok(Json::Null)));
    }

    #[test]
    fn the_listed_output_devices_feed_controls_output_and_a_pick_becomes_the_default() {
        let ctx = test_ctx();
        let mut app = MusicApp::new();
        app.devices_request = Some(3);
        let listing = bifrost_core::json::parse(
            r#"{"devices":[{"id":"speaker","name":"Speakers","output":true},{"id":"mic","name":"Microphone","input":true},
                {"id":"headphones","name":"Headphones","output":true}],"defaults":{"output":"speaker"}}"#,
        )
        .unwrap();
        assert!(app.service_reply(&ctx, 3, &Ok(listing)));
        assert_eq!(app.outputs, [("speaker".to_string(), "Speakers".to_string()), ("headphones".to_string(), "Headphones".to_string())], "only devices that can play");
        assert!(app.menu_action(&ctx, Command::Output(1).id()));
        assert_eq!(app.output.as_deref(), Some("headphones"));
        let sent = calls(&ctx);
        assert!(sent.iter().any(|(svc, method, body)| svc == audio_devices::SERVICE && method == "set_default" && body.contains("headphones") && body.contains("output")), "{sent:?}");
        assert!(!app.menu_action(&ctx, Command::Output(9).id()), "a device that is gone does nothing");
        // a device list change asks for the list again
        let changed = bifrost_core::json::parse(r#"{"devices":["speaker"]}"#).unwrap();
        app.service_event(&ctx, audio_devices::SERVICE, "devices_changed", &changed);
        assert!(app.devices_request.is_some());
    }

    #[test]
    fn import_asks_for_several_songs_and_only_its_own_answer_is_taken() {
        use bifrost_app::proto::DocErrorCode;
        let ctx = test_ctx();
        let mut app = MusicApp::new();
        assert!(!app.menu_action(&ctx, Command::Import.id()), "the panel needs no frame");
        let request = app.import_request.expect("the open panel was asked for");
        let asked = ctx.take_sent().into_iter().any(|m| matches!(m, ToShell::DocOpenEx { title, options, .. } if title == "Import to Music" && options & bifrost_app::proto::docopt::MULTIPLE != 0));
        assert!(asked, "one panel that allows several files");
        assert!(!app.document(&ctx, DocumentEvent::Failed { request: request.wrapping_add(1), code: DocErrorCode::Cancelled }));
        assert_eq!(app.import_request, Some(request), "another request's answer is not ours");
        assert!(!app.document(&ctx, DocumentEvent::Failed { request, code: DocErrorCode::Cancelled }));
        assert_eq!(app.import_request, None, "a cancelled panel is forgotten");
    }

    #[test]
    fn without_a_window_there_are_no_menus_and_no_command_runs() {
        let ctx = test_ctx();
        let mut app = MusicApp::new();
        assert!(app.main_menu(&ctx).is_empty());
        assert!(!app.menu_action(&ctx, Command::PlayPause.id()));
        assert!(!app.menu_action(&ctx, 7), "an id no menu carries");
    }

    #[test]
    fn route_and_command_events_without_a_window_do_nothing() {
        let ctx = test_ctx();
        let mut app = MusicApp::new();
        assert!(!app.route_changed(Some("speaker".into())));
        assert_eq!(app.output.as_deref(), Some("speaker"));
        let pause = Json::Obj(vec![("command".into(), Json::Str("pause".into()))]);
        assert!(!app.service_event(&ctx, now_playing::SERVICE, "command", &pause));
        assert!(!app.service_event(&ctx, "some-other-service", "command", &pause));
        assert!(calls(&ctx).is_empty());
    }

    #[test]
    fn audio_intents_map_to_the_transport_controls() {
        assert_eq!(audio_transport(AUDIO_PLAY_PAUSE), Some((Transport::Play, "Playback toggled")));
        assert_eq!(audio_transport(AUDIO_NEXT), Some((Transport::Next, "Skipped to the next track")));
        assert_eq!(audio_transport(AUDIO_PREVIOUS), Some((Transport::Prev, "Went back to the previous track")));
        assert!(audio_transport("shuffle").is_none());
    }
}
