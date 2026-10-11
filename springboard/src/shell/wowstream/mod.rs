//! WoW: a low-latency game-streaming client. The protocol driver and the in-stream controls
//! come from `wowstream-client`; everything you look at outside the stream is SpringBoard's.
//!
//! * `launcher`: the game cards page (state from the core's `launcher::Nav`)
//! * `settings`: login and stream options as an bifrost grouped list with the system keyboard
//! * `controls`: the core's `Ui` (sticks, buttons, menu) with a touch dp and the shell keyboard
//! * `video`: hardware H.264 decode; the picture goes to the GPU as is (`video_layer`) or is
//!   converted NV12 -> RGBA at the letterboxed size when the CPU draws it
//! * `audio`: the Opus sink (still a no-op, see the TODO there)
//! * `render`, `keys`, `icon`: painting helpers, evdev -> character, the app icon
//!
//! WoW is a landscape app: the shell lays out landscape while it is in front (see
//! `rotate::prefers_landscape`), turning with the sensor between the two landscape sides, so
//! everything here draws and takes touches in a plain landscape frame.
//!
//! The client runs only while the app is on screen: it connects when the app comes to the front
//! and is closed (keys released, `Bye` sent) as soon as it is not.

mod audio;
mod controls;
mod icon;
pub(crate) mod keys;
mod launcher;
mod render;
mod settings;
mod video;

pub use icon::icon;

use super::keyboard::Out;
use super::ui::{home_indicator, Ui, BLACK};
use super::{AppId, Event, Screen, Shell};
use crate::gfx::{Image, Surface};
use crate::media::{convert, Picture, VideoLayer};
use controls::Controls;
use launcher::{Chip, Hit, Launcher, Link};
use settings::{Outcome, SettingsPage};
use std::ffi::c_int;
use std::io::{self, Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
use std::sync::mpsc::{channel, sync_channel, Receiver, RecvTimeoutError, Sender, SyncSender, TrySendError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use wowstream_client::client::{Client, ClientEvent};
use wowstream_client::config::Settings;
use wowstream_client::launcher::{Out as NavOut, Screen as NavScreen};
use wowstream_client::net::Event as NetEvent;
use wowstream_client::profile::Shape;
use wowstream_client::proto::Control;
use wowstream_client::stats::Rates;
use wowstream_client::ui::Effect;

/// evdev `KEY_BACK`: the one key that means "back" rather than going to the game.
const KEY_BACK: u16 = 158;
const KEY_LEFTSHIFT: u16 = 42;
const KEY_RIGHTSHIFT: u16 = 54;
const KEY_BACKSPACE: u16 = 14;
const KEY_ENTER: u16 = 28;

/// One connection: the client, the threads around it and what they hand back.
struct Session {
    client: Arc<Client>,
    stop: Arc<AtomicBool>,
    frame: video::Latest,
    video: SyncSender<video::Msg>,
    video_thread: Option<std::thread::JoinHandle<()>>,
    events: Receiver<NetEvent>,
    /// the shell draws video on the GPU: the video thread stops converting
    gpu: Arc<AtomicBool>,
}

/// Access units queued for the decoder before the router starts dropping them (to the next IDR).
const VIDEO_QUEUE: usize = 32;

impl Session {
    fn start(settings: &Settings, shape: Shape, wake: video::Wake, gpu: bool) -> std::io::Result<Session> {
        let (client, rx) = Client::connect(settings.clone(), shape)?;
        let client = Arc::new(client);
        let (video, video_rx) = sync_channel(VIDEO_QUEUE);
        let (events_tx, events) = channel();
        let frame: video::Latest = Arc::new(Mutex::new(None));
        let stop = Arc::new(AtomicBool::new(false));
        let gpu = Arc::new(AtomicBool::new(gpu));
        let video_thread = video::spawn(video_rx, client.clone(), frame.clone(), wake.clone(), stop.clone(), gpu.clone());
        let router = Router { rx, video: video.clone(), client: client.clone(), skipping: false, events: events_tx, wake, stop: stop.clone(), sink: audio::open() };
        if let Err(e) = std::thread::Builder::new().name("wow-route".into()).spawn(move || router.run()) {
            eprintln!("springboard: wow: router thread: {e}");
        }
        Ok(Session { client, stop, frame, video, video_thread, events, gpu })
    }
}

impl Drop for Session {
    /// Releases input, says Bye and stops the network thread; the other threads follow.
    fn drop(&mut self) {
        self.stop.store(true, Relaxed);
        self.client.close();
        // The video thread owns the hardware decoder: a reopened game must not open a second one
        // while this one is still up. It sees `stop` within 100 ms (or after the frame in hand).
        if let Some(h) = self.video_thread.take() {
            crate::media::audioplayer::join_within(h, Duration::from_millis(1500));
        }
    }
}

/// Splits the client's single queue: video to the decoder, audio to the sink, the rest to the UI.
struct Router {
    rx: Receiver<ClientEvent>,
    video: SyncSender<video::Msg>,
    client: Arc<Client>,
    /// the queue overflowed: drop frames until the next keyframe fits
    skipping: bool,
    events: Sender<NetEvent>,
    wake: video::Wake,
    stop: Arc<AtomicBool>,
    sink: Box<dyn audio::Sink>,
}

impl Router {
    fn run(mut self) {
        while !self.stop.load(Relaxed) {
            match self.rx.recv_timeout(Duration::from_millis(100)) {
                Ok(ClientEvent::Frame(f)) => {
                    if self.skipping && !f.keyframe {
                        continue;
                    }
                    let keyframe = f.keyframe;
                    match self.video.try_send(video::Msg::Frame(f)) {
                        Ok(()) => self.skipping = false,
                        Err(TrySendError::Full(_)) => {
                            // the decoder fell behind: a gap breaks the stream, so wait for an IDR
                            if !self.skipping || keyframe {
                                self.client.request_keyframe();
                            }
                            self.skipping = true;
                        }
                        Err(TrySendError::Disconnected(_)) => break,
                    }
                }
                Ok(ClientEvent::Audio { opus, pts_us }) => self.sink.packet(&opus, pts_us),
                Ok(ClientEvent::Net(e)) => {
                    // In order with the frames: everything after this belongs to the new stream.
                    let restart = match &e {
                        NetEvent::Connected { width, height, .. } => Some((*width, *height)),
                        NetEvent::StreamChanged { shape, resize: true } => Some((shape.width, shape.height)),
                        _ => None,
                    };
                    if let Some((width, height)) = restart {
                        let _ = self.video.send(video::Msg::Restart { width, height });
                        self.skipping = false;
                    }
                    let _ = self.events.send(e);
                    (self.wake)();
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break,
            }
        }
    }
}

/// What the app shows.
#[derive(Clone, Copy, PartialEq, Eq)]
enum View {
    Settings,
    Launcher,
    Stream,
}

pub struct WowStream {
    w: i32,
    h: i32,
    controls: Controls,
    launcher: Launcher,
    page: Option<SettingsPage>,
    /// the launcher button under the finger
    pressed: Hit,
    settings: Settings,
    settings_path: PathBuf,
    session: Option<Session>,
    /// On screen: the client is wanted (it stays down after a failed login until Save).
    active: bool,
    connected: bool,
    /// The host the current connection reached.
    host: String,
    status: String,
    /// The newest decoded picture: RGBA at the view size, or still in the decoder's buffer.
    frame: Option<Picture>,
    /// A `Picture::Gpu` converted for a frame the CPU draws: ((seq, w, h), picture).
    cpu_copy: Option<((u64, usize, usize), Image)>,
    /// The shell can draw video on the GPU (tells new sessions' video threads).
    gpu_video: bool,
    /// The next `draw` is an overlay over a GPU video layer (set by `video_layer`).
    layer_frame: bool,
    rates: Rates,
    wake_rx: UnixStream,
    wake_tx: Arc<UnixStream>,
    /// The picture size last sent to the video thread.
    view_sent: (usize, usize),
    shift: u8,
    dirty: bool,
}

impl WowStream {
    /// `dir` holds the settings file (`wowstream.conf`).
    pub fn new(ui: &Ui, w: i32, h: i32, dir: PathBuf) -> io::Result<Self> {
        std::fs::create_dir_all(&dir)?;
        let settings_path = dir.join("wowstream.conf");
        let first_run = !settings_path.exists();
        let mut settings = Settings::load(&settings_path);
        if first_run {
            // the core pre-fills the owner's name; a first run starts with the user empty
            settings.user.clear();
        }
        let (wake_rx, wake_tx) = UnixStream::pair()?;
        wake_rx.set_nonblocking(true)?;
        wake_tx.set_nonblocking(true)?;
        let mut controls = Controls::new(ui, w, h);
        controls.core.show_settings(&settings);
        Ok(WowStream {
            w,
            h,
            controls,
            launcher: Launcher::new(),
            page: None,
            pressed: Hit::None,
            settings,
            settings_path,
            session: None,
            active: false,
            connected: false,
            host: String::new(),
            status: String::new(),
            frame: None,
            cpu_copy: None,
            gpu_video: false,
            layer_frame: false,
            rates: Rates::new(Instant::now()),
            wake_rx,
            wake_tx: Arc::new(wake_tx),
            view_sent: (0, 0),
            shift: 0,
            dirty: true,
        })
    }

    fn view(&self) -> View {
        if self.page.is_some() {
            View::Settings
        } else if self.controls.core.nav.screen == NavScreen::Stream {
            View::Stream
        } else {
            View::Launcher
        }
    }

    /// The stream owns the whole screen (no status bar); the other pages sit under it.
    pub fn fullscreen(&self) -> bool {
        self.view() == View::Stream
    }

    /// A stream is live: the screen and the device stay awake.
    pub fn streaming(&self) -> bool {
        self.active && self.connected
    }

    /// The app came to the front: connect (or ask for the login first).
    fn resume(&mut self, ui: &Ui, t: f64) {
        if self.active {
            return;
        }
        self.active = true;
        let mut fx = vec![];
        self.controls.sync(ui, &mut fx);
        self.apply(ui, t, fx);
        if self.settings.is_complete() {
            self.connect();
        } else {
            self.open_settings(ui, String::new());
        }
        self.dirty = true;
    }

    /// The app left the front: release the keys and close the connection.
    pub fn stop(&mut self) {
        if !self.active {
            return;
        }
        self.active = false;
        if let Some(s) = &self.session {
            s.client.release_all();
        }
        let mut fx = vec![];
        self.controls.core.pause(&mut fx);
        drop(fx); // the session is closing anyway: Drop sends the ReleaseAll
        self.session = None;
        self.connected = false;
        self.frame = None;
        self.cpu_copy = None;
        self.status.clear();
        self.page = None;
        self.dirty = true;
    }

    fn size(&self) -> (f32, f32) {
        (self.w as f32, self.h as f32)
    }

    fn shape(&self) -> Shape {
        self.settings.profile.shape(self.settings.fill, self.size())
    }

    fn connect(&mut self) {
        self.session = None;
        self.connected = false;
        let tx = self.wake_tx.clone();
        let wake: video::Wake = Arc::new(move || drop((&*tx).write(&[1])));
        self.session = match Session::start(&self.settings, self.shape(), wake, self.gpu_video) {
            Ok(session) => Some(session),
            Err(e) => {
                eprintln!("springboard: wow: connect: {e}");
                None
            }
        };
        self.view_sent = (0, 0);
    }

    /// Asks the server for the shape the current profile and Fill screen setting want.
    fn request_stream(&self) {
        if let Some(s) = &self.session {
            s.client.stream_config(self.shape());
        }
    }

    fn save_settings(&self) {
        if let Err(e) = self.settings.save(&self.settings_path) {
            eprintln!("springboard: wow: saving settings: {e}");
        }
    }

    fn open_settings(&mut self, ui: &Ui, status: String) {
        self.page = Some(SettingsPage::new(ui, self.w, self.h, self.size(), &self.settings, status));
        self.dirty = true;
    }

    /// Carries out what the core asked for.
    fn apply(&mut self, ui: &Ui, _t: f64, fx: Vec<Effect>) {
        for e in fx {
            match e {
                Effect::Control(msg) => {
                    if let Some(s) = &self.session {
                        match msg {
                            Control::GamesRequest => s.client.request_games(),
                            Control::Launch { id } => s.client.launch(&id),
                            Control::StopGame => s.client.stop_game(),
                            _ => {}
                        }
                    }
                }
                Effect::Send(input) => {
                    if let Some(s) = &self.session {
                        s.client.send(input);
                    }
                }
                Effect::ToggleHud => {
                    self.settings.show_hud = !self.settings.show_hud;
                    self.save_settings();
                }
                Effect::Profile(p) => {
                    self.settings.profile = p;
                    self.save_settings();
                    self.request_stream();
                }
                Effect::OpenSettings => self.open_settings(ui, String::new()),
                // Controls::touch turns this into the shell keyboard before effects get here.
                Effect::SystemKeyboard => {}
                Effect::Save(s) => {
                    if let Err(e) = s.save(&self.settings_path) {
                        eprintln!("springboard: wow: saving settings: {e}");
                    }
                    let login = |s: &Settings| (s.host.clone(), s.user.clone(), s.password.clone());
                    let reconnect = login(&s) != login(&self.settings) || self.session.is_none();
                    self.settings = s;
                    if reconnect && self.active && self.settings.is_complete() {
                        self.connect();
                    } else {
                        self.request_stream();
                    }
                }
            }
            self.dirty = true;
        }
        self.controls.core.show_settings(&self.settings);
    }

    /// Carries out launcher navigation steps.
    fn nav_out(&mut self, ui: &Ui, t: f64, outs: Vec<NavOut>) {
        for o in outs {
            let mut fx = vec![];
            match o {
                NavOut::RequestGames => fx.push(Effect::Control(Control::GamesRequest)),
                NavOut::Launch(id) => fx.push(Effect::Control(Control::Launch { id })),
                NavOut::Stop => fx.push(Effect::Control(Control::StopGame)),
                NavOut::ReleaseAll => self.controls.core.pause(&mut fx),
            }
            self.apply(ui, t, fx);
        }
    }

    /// A launcher button was tapped.
    fn launcher_act(&mut self, ui: &Ui, t: f64, hit: Hit) {
        let now = t as u64;
        let mut outs = vec![];
        let nav = &mut self.controls.core.nav;
        match hit {
            Hit::Settings => return self.open_settings(ui, String::new()),
            Hit::Play(id) => nav.play(&id, now, &mut outs),
            Hit::Close => nav.ask_close(),
            Hit::Yes => nav.answer(true, now, &mut outs),
            Hit::No => nav.answer(false, now, &mut outs),
            Hit::None => {}
        }
        self.nav_out(ui, t, outs);
    }

    /// Touches from the shell: one finger.
    pub fn touch(&mut self, ui: &Ui, ev: Event, t: f64) {
        self.dirty = true;
        match self.view() {
            View::Settings => {
                let out = self.page.as_mut().and_then(|p| p.touch(ui, ev, t));
                match out {
                    Some(Outcome::Save(s)) => {
                        self.page = None;
                        self.apply(ui, t, vec![Effect::Save(s)]);
                    }
                    Some(Outcome::Cancel) => self.page = None,
                    None => {}
                }
            }
            View::Launcher => {
                let (w, h) = (self.w as f32, self.h as f32);
                match ev {
                    Event::Down(x, y) => self.pressed = self.launcher.hit(ui, w, h, &self.controls.core.nav, x, y),
                    Event::Up(x, y) => {
                        let hit = self.launcher.hit(ui, w, h, &self.controls.core.nav, x, y);
                        if hit == std::mem::replace(&mut self.pressed, Hit::None) {
                            self.launcher_act(ui, t, hit);
                        }
                    }
                    _ => {}
                }
            }
            View::Stream => {
                let mut fx = vec![];
                self.controls.touch(ui, ev, t, &mut fx);
                self.apply(ui, t, fx);
            }
        }
    }

    /// A physical key (evdev code): the settings page types it, the game gets it.
    fn hw_key(&mut self, ui: &Ui, code: u16, down: bool, t: f64) {
        if code == KEY_LEFTSHIFT || code == KEY_RIGHTSHIFT {
            self.shift = if down { self.shift.saturating_add(1) } else { self.shift.saturating_sub(1) };
        }
        let ch = if down { keys::char_for(code, self.shift > 0) } else { None };
        if let Some(page) = &mut self.page {
            if down {
                match (code, ch) {
                    (KEY_BACKSPACE, _) => page.key(ui, Out::Backspace, t),
                    (KEY_ENTER, _) => page.key(ui, Out::Enter, t),
                    (_, Some(c)) => page.key(ui, Out::Text(c.to_string()), t),
                    _ => {}
                }
            }
        } else if self.view() == View::Stream {
            let mut fx = vec![];
            self.controls.hardware_key(code, down, ch, &mut fx);
            self.apply(ui, t, fx);
        }
        self.dirty = true;
    }

    /// A hardware keyboard's text goes to the settings page's focused field; nothing else takes it.
    pub fn hw_text(&mut self, ui: &Ui, out: Out, t: f64) {
        if let Some(page) = &mut self.page {
            page.key(ui, out, t);
            self.dirty = true;
        }
    }

    /// Back: closes the settings page, steps out of the stream; false means leave the app.
    fn back(&mut self, ui: &Ui, t: f64) -> bool {
        self.dirty = true;
        if self.page.take().is_some() {
            return true;
        }
        let mut fx = vec![];
        let handled = self.controls.core.back(&mut fx);
        self.apply(ui, t, fx);
        handled
    }

    /// Frame work for the shell's tick; true when the screen needs a redraw.
    pub fn tick(&mut self, ui: &Ui, t: f64) -> bool {
        let mut fx = vec![];
        self.dirty |= self.controls.sync(ui, &mut fx);
        match self.view() {
            View::Stream => self.dirty |= self.controls.tick(t, &mut fx),
            _ => self.controls.core.tick(t as u64, &mut fx),
        }
        self.apply(ui, t, fx);
        if let Some(p) = &mut self.page {
            self.dirty |= p.tick(ui, t);
        }
        self.send_view();
        if let Some(s) = &self.session {
            if self.rates.update(s.client.stats(), Instant::now()) && self.settings.show_hud {
                self.dirty = true;
            }
        }
        std::mem::take(&mut self.dirty)
    }

    /// Tells the video thread how big the picture is on screen (it changes with the keyboard).
    fn send_view(&mut self) {
        let (_, _, w, h) = self.controls.view();
        let size = (w.round().max(2.0) as usize, h.round().max(2.0) as usize);
        if size == self.view_sent {
            return;
        }
        if let Some(s) = &self.session {
            if let Err(TrySendError::Full(_)) = s.video.try_send(video::Msg::View { width: size.0, height: size.1 }) {
                return; // retried on the next tick
            }
        }
        self.view_sent = size;
    }

    pub fn next_wake(&self, t: f64) -> Option<f64> {
        let hud = (self.settings.show_hud && self.session.is_some()).then_some(t + 1000.0);
        let page = self.page.as_ref().and_then(|p| p.next_wake(t));
        [self.controls.next_wake(t), hud, page].into_iter().flatten().reduce(f64::min)
    }

    /// Readable when the client or the decoder has something for the UI.
    pub fn fds(&self) -> Vec<c_int> {
        vec![self.wake_rx.as_raw_fd()]
    }

    /// Takes the newest picture and the connection events; true when the screen changed.
    pub fn pump(&mut self, ui: &Ui, t: f64) -> bool {
        let mut sink = [0u8; 64];
        while matches!(self.wake_rx.read(&mut sink), Ok(n) if n > 0) {}
        let changed = self.take_frame() | self.take_events(ui, t);
        self.dirty |= changed;
        changed
    }

    /// Only the newest decoded picture is ever shown; older ones were replaced in the slot.
    fn take_frame(&mut self) -> bool {
        let Some(s) = &self.session else { return false };
        // the picture shown before is dropped here: the GPU finished with it at the last present
        let Some(img) = s.frame.lock().unwrap().take() else { return false };
        s.client.stats().frames_shown.fetch_add(1, Relaxed);
        self.frame = Some(img);
        true
    }

    fn take_events(&mut self, ui: &Ui, t: f64) -> bool {
        let mut any = false;
        while let Some(e) = self.session.as_ref().and_then(|s| s.events.try_recv().ok()) {
            any = true;
            match e {
                NetEvent::Connected { host, width, height, fps } => {
                    eprintln!("springboard: wow: streaming {width}x{height}@{fps} from {host}");
                    self.connected = true;
                    self.host = host;
                    self.controls.core.connected();
                    self.status.clear();
                    self.set_stream(width, height);
                }
                NetEvent::StreamChanged { shape, resize } => {
                    if resize {
                        self.set_stream(shape.width, shape.height);
                    }
                }
                NetEvent::Games { running, games } => {
                    let mut fx = vec![];
                    self.controls.core.on_games(running, games, &mut fx);
                    self.apply(ui, t, fx);
                }
                NetEvent::Status(s) => {
                    self.connected = false;
                    self.status = s;
                }
                NetEvent::AuthFailed(why) => {
                    self.connected = false;
                    self.session = None;
                    self.status = format!("Login failed: {why}");
                    self.open_settings(ui, self.status.clone());
                }
            }
        }
        any
    }

    /// A new stream size: the old picture no longer fits.
    fn set_stream(&mut self, width: u16, height: u16) {
        self.frame = None;
        self.cpu_copy = None;
        self.controls.set_stream(width, height);
    }

    /// The chip under the launcher's title.
    fn chip(&self) -> Chip {
        let Some(s) = &self.session else { return Chip { link: Link::Offline, text: "Not connected".into() } };
        if !self.connected {
            let text = if self.status.is_empty() { "Connecting...".to_string() } else { self.status.clone() };
            return Chip { link: Link::Connecting, text };
        }
        let st = s.client.stats();
        let rtt = st.rtt_us.load(Relaxed) as f32 / 1000.0;
        Chip { link: Link::Online, text: format!("{}  \u{b7}  {}  \u{b7}  {rtt:.0} ms", self.host, *st.path.lock().unwrap()) }
    }

    pub fn draw(&mut self, s: &mut Surface, ui: &Ui, t: f64) {
        match self.view() {
            View::Settings => {
                if let Some(p) = &mut self.page {
                    p.draw(s, ui, t);
                }
            }
            View::Launcher => {
                let chip = self.chip();
                self.launcher.draw(s, ui, &self.controls.core.nav, &chip, &self.pressed, t);
            }
            View::Stream => self.draw_stream(s, ui, t),
        }
    }

    /// The picture for the GPU, when the shell can compose this frame (`eligible`) and the
    /// stream has one in a decoder buffer; the next `draw` then paints only the controls,
    /// transparent where the picture shows. `gpu` says whether the GPU path exists at all: the
    /// video thread converts pictures only while it does not.
    pub fn video_layer(&mut self, gpu: bool, eligible: bool) -> Option<VideoLayer> {
        self.gpu_video = gpu;
        if let Some(s) = &self.session {
            s.gpu.store(gpu, Relaxed);
        }
        self.layer_frame = false;
        if !(gpu && eligible && self.view() == View::Stream) {
            return None;
        }
        let Some(Picture::Gpu(p)) = &self.frame else { return None };
        let dst = crate::gpu::quad::fit_rect(p.width, p.height, self.controls.view());
        let layer = VideoLayer { frame: p.clone(), dst };
        self.layer_frame = true;
        Some(layer)
    }

    /// The picture as RGBA for a frame the CPU draws (a `Picture::Gpu` is converted once per
    /// picture and view size).
    fn cpu_picture(&mut self) -> Option<&Image> {
        let p = match self.frame.as_ref()? {
            Picture::Rgba(_) => None,
            Picture::Gpu(p) => Some(p),
        };
        if let Some(p) = p {
            let (_, _, vw, vh) = self.controls.view();
            let (ow, oh) = convert::fit(p.width, p.height, vw.round().max(2.0) as u32, vh.round().max(2.0) as u32);
            let key = (p.seq, ow, oh);
            if self.cpu_copy.as_ref().map(|c| c.0) != Some(key) {
                self.cpu_copy = Some((key, convert::dma_to_rgba(p, ow, oh, convert::matrix_for(p.height), 3)));
            }
            return self.cpu_copy.as_ref().map(|c| &c.1);
        }
        match self.frame.as_ref()? {
            Picture::Rgba(img) => Some(img),
            Picture::Gpu(_) => None,
        }
    }

    /// The picture, then the controls over it, all turned for a portrait surface. Over a GPU
    /// video layer only the controls are drawn, on transparent.
    fn draw_stream(&mut self, s: &mut Surface, ui: &Ui, t: f64) {
        let (w, h) = (s.w, s.h);
        let stats = self.session.as_ref().map(|s| s.client.stats());
        let hud = stats.filter(|_| self.settings.show_hud).map(|st| self.rates.line(st));
        let waiting = !self.connected || self.frame.is_none();
        let banner = waiting.then(|| if self.status.is_empty() { "Waiting for video..." } else { self.status.as_str() }).map(str::to_owned);
        let lists = self.controls.overlay(hud.as_deref(), banner.as_deref());
        if std::mem::take(&mut self.layer_frame) {
            // The GPU draws the picture under this frame: when the controls are exactly what was
            // painted last time, the overlay pixels are still in place and nothing needs drawing.
            if self.controls.overlay_unchanged(&lists, (w, h)) {
                return;
            }
            s.fill_rect(0, 0, w, h, 0);
        } else {
            self.controls.overlay_invalidate();
            let (vx, vy, vw, vh) = self.controls.view();
            let f = self.cpu_picture();
            if !f.is_some_and(|f| f.w >= w && f.h >= h) {
                s.fill_rect(0, 0, w, h, BLACK);
            }
            if let Some(f) = f {
                let (x, y) = ((vx + (vw - f.w as f32) / 2.0).round() as i32, (vy + (vh - f.h as f32) / 2.0).round() as i32);
                s.copy(f.view(), x, y);
            }
        }
        self.controls.paint(s, ui, &lists, t);
        home_indicator(s, ui, w as i32, h as i32, 70);
    }
}

impl Shell {
    /// Runs the client while the app is on screen and shuts it down as soon as it is not.
    pub(super) fn tick_wowstream(&mut self, t: f64) {
        let on = self.screen_on && self.screen == Screen::App(AppId::WowStream);
        let Some(w) = self.wowstream.as_mut() else { return };
        if !on {
            w.stop();
            return;
        }
        w.resume(&self.ui, t);
        self.dirty |= w.tick(&self.ui, t);
        if w.streaming() {
            // a game is not idle because nobody touched the screen
            self.last_input = t;
        }
    }

    /// A key from a physical keyboard goes to the app while WoW is on screen; true if taken.
    /// Power, volume and wake stay with the shell.
    pub fn hw_key(&mut self, code: u16, down: bool, t: f64) -> bool {
        const KEEP: [u16; 4] = [114, 115, 116, 143];
        if down && self.screen_on && self.anim.is_none() && !KEEP.contains(&code) {
            let used = match self.screen {
                Screen::App(AppId::Deildu) => self.deildu.as_mut().is_some_and(|d| d.hw_key(code, t)),
                Screen::App(AppId::Files) => self.files.as_mut().is_some_and(|f| f.hw_key(code, t)),
                _ => false,
            };
            if used {
                self.last_input = t;
                self.dirty = true;
                return true;
            }
        }
        if !self.screen_on || self.anim.is_some() || self.screen != Screen::App(AppId::WowStream) || KEEP.contains(&code) {
            return false;
        }
        let Some(w) = self.wowstream.as_mut() else { return false };
        self.last_input = t;
        self.dirty = true;
        if code == KEY_BACK {
            if down && !w.back(&self.ui, t) {
                self.go_home(t);
            }
            return true;
        }
        w.hw_key(&self.ui, code, down, t);
        true
    }
}
