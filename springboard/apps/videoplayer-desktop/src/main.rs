//! Video Player as an installed BifrOSt app. It plays one movie at a time in the shell's fullscreen player view, over
//! the shell's media stack (the same view and decoder Files and Deildu use). A sidecar subtitle file (`.srt` or `.vtt`
//! next to the movie) shows as captions, trim marks loop a part of the movie, and the Recording menu makes an audio
//! take through the audio service into the app's data folder.
//!
//! Movies come from Movie > Open, from a file handed to the app, or from the command line. New Screen Recording stays
//! off until the shell has a screen capture service, and a trimmed movie cannot be exported yet: there is no movie
//! writer. The take and the recording state use the shared `Recorder` model; the window only shows its answers.

mod movie;

use bifrost_app::audio;
use bifrost_app::controls::recording::{RecordError, Recorder};
use bifrost_app::kit::{pack, rgba, Damage, Event, Image, Surface, Ui, Weight};
use bifrost_app::intents::{self, Intent, IntentResult, Param, ParamType};
use bifrost_app::proto::DocMode;
use bifrost_app::{App, AppMenu, AppMenuItem, Ctx, DocumentEvent, Input, KeyInput, OpenOptions, TouchKind};
use bifrost_core::json::Json;
use springboard::deildu::player_view::Outcome;
use springboard::shell::clock::Wall;
use springboard::shell::photos::library;
use std::fs::File;
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};

const MENU_OPEN: u32 = 1;
const MENU_TRIM_START: u32 = 2;
const MENU_TRIM_END: u32 = 3;
const MENU_TRIM_CLEAR: u32 = 4;
const MENU_SUBTITLES: u32 = 5;
const MENU_AUDIO: u32 = 6;
const MENU_SCREEN: u32 = 7;

/// The extensions the open panel offers, as the media stack demuxes them.
const MOVIE_TYPES: &[&str] = &["mp4", "m4v", "mov", "webm"];
const BACKGROUND: u32 = pack(28, 28, 30, 255);
const WHITE: u32 = pack(255, 255, 255, 255);
const EMPTY_PT: f32 = 17.0;
const NOTICE_PT: f32 = 15.0;

struct VideoPlayerApp {
    w: i32,
    h: i32,
    movie: Option<movie::Movie>,
    /// the file a panel handed over; kept open while its movie plays
    held: Option<File>,
    /// a path from the command line, opened on the first configure
    pending: Option<PathBuf>,
    audio: audio::Client,
    /// the audio take in progress (the shared recording model)
    take: Recorder,
    take_path: Option<PathBuf>,
    /// screen recording: the shell has no capture service yet, so this recorder stays unavailable
    screen: Recorder,
    notice: String,
    notice_img: Option<(String, Image)>,
    empty: Option<Image>,
    /// the last frame time, for the calls that come from menus and keys
    last_t: f64,
}

fn shell_dir() -> PathBuf {
    PathBuf::from(std::env::var("BIFROST_SERVICE_ROOT").unwrap_or_else(|_| "/var/lib/springboard".into()))
}

/// The window title of a movie: its file name without the extension.
fn title_of(path: &Path) -> String {
    path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
}

/// The sentence the notice shows for a refused recorder call.
fn take_error(e: RecordError) -> String {
    match e {
        RecordError::Unavailable => "Recording is not available.".into(),
        RecordError::AlreadyRecording => "A recording is already running.".into(),
        RecordError::NotRecording => "No recording is running.".into(),
        RecordError::Busy => "The recording is busy.".into(),
        RecordError::InvalidTime => "The recording time is invalid.".into(),
    }
}

impl VideoPlayerApp {
    fn new(pending: Option<PathBuf>) -> Self {
        VideoPlayerApp {
            w: 0,
            h: 0,
            movie: None,
            held: None,
            pending,
            audio: audio::Client::for_root(&shell_dir()),
            take: Recorder::new(true),
            take_path: None,
            screen: Recorder::new(false),
            notice: String::new(),
            notice_img: None,
            empty: None,
            last_t: 0.0,
        }
    }

    /// Opens `path` as the movie in the window. A file that cannot be played closes the movie and says why.
    fn open_path(&mut self, ctx: &Ctx, path: &Path, title: &str) -> bool {
        match movie::Movie::open(&ctx.ui, self.w, self.h, path, title) {
            Ok(m) => {
                self.movie = Some(m);
                self.notice.clear();
                true
            }
            Err(e) => {
                self.movie = None;
                self.held = None;
                self.notice = format!("Cannot play {title}: {e}");
                false
            }
        }
    }

    fn close(&mut self) {
        self.movie = None;
        self.held = None;
    }

    /// Reopens the movie after a playback error.
    fn retry(&mut self, ctx: &Ctx) {
        if let Some(path) = self.movie.as_ref().map(|m| m.path().to_path_buf()) {
            let title = title_of(&path);
            self.open_path(ctx, &path, &title);
        }
    }

    fn open_panel(&self, ctx: &Ctx) {
        let options = OpenOptions {
            mode: DocMode::Read,
            title: "Open Movie".into(),
            filters: MOVIE_TYPES.iter().map(|s| s.to_string()).collect(),
            ..Default::default()
        };
        ctx.open_panel(&options);
    }

    /// Trim start or end at the playhead, from the Movie menu.
    fn set_trim(&mut self, start: bool) {
        let Some(m) = self.movie.as_mut() else { return };
        let ok = if start { m.set_trim_start() } else { m.set_trim_end() };
        self.notice = if ok { m.trim_text().unwrap_or_default() } else { "A trim needs the movie's length: play it first.".into() };
    }

    /// Starts an audio take through the audio service, or finishes the one running.
    fn toggle_take(&mut self, ctx: &Ctx, t: f64) {
        if self.take.is_recording() {
            self.finish_take(t);
            return;
        }
        let wall = Wall::sample(t);
        let stamp = library::stamp(&library::civil(wall.now(t), wall.offset));
        let path = ctx.data_dir.join(format!("Audio Recording {stamp}.wav"));
        if let Err(e) = self.take.start(t / 1000.0, true) {
            self.notice = take_error(e);
            return;
        }
        if !self.audio.record_start(&path.to_string_lossy()) {
            self.take.fail("the audio service is not running");
            self.notice = "Cannot record: the audio service is not running.".into();
            return;
        }
        self.take_path = Some(path);
        self.notice = "Recording audio.".into();
    }

    fn finish_take(&mut self, t: f64) {
        self.audio.record_stop();
        match self.take.stop(t / 1000.0) {
            Ok(done) => {
                let name = self.take_path.take().and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned())).unwrap_or_default();
                self.notice = format!("Recorded {name} ({}).", movie::clock(done.duration));
            }
            Err(e) => self.notice = take_error(e),
        }
    }

    fn input(&mut self, ctx: &Ctx, ev: Input) -> bool {
        match ev {
            Input::Open(path) => {
                let path = PathBuf::from(path);
                let title = title_of(&path);
                self.open_path(ctx, &path, &title);
                true
            }
            Input::Key(KeyInput::Code(code)) => {
                let t = self.last_t;
                self.movie.as_mut().is_some_and(|m| m.key(code as u16, t))
            }
            Input::Touch { kind, x, y, t, .. } => {
                self.last_t = t;
                let e = match kind {
                    TouchKind::Down => Event::Down(x, y),
                    TouchKind::Move => Event::Move(x, y),
                    TouchKind::Up | TouchKind::Cancel => Event::Up(x, y),
                };
                let outcome = match &mut self.movie {
                    Some(m) => m.touch(&ctx.ui, e, t),
                    None => return false,
                };
                match outcome {
                    Outcome::None => {}
                    Outcome::Close => self.close(),
                    Outcome::Retry => self.retry(ctx),
                }
                true
            }
            _ => false,
        }
    }

    /// The notice line, rendered once per change.
    fn refresh_notice(&mut self, ui: &Ui) {
        if self.notice.is_empty() {
            self.notice_img = None;
        } else if self.notice_img.as_ref().map_or(true, |(text, _)| *text != self.notice) {
            let max = (self.w as f32 - ui.pt(64.0)).max(0.0);
            let img = ui.text(&ui.fit(&self.notice, NOTICE_PT, max), NOTICE_PT, Weight::Regular, WHITE);
            self.notice_img = Some((self.notice.clone(), img));
        }
    }

    fn draw_notice(&self, s: &mut Surface, ui: &Ui) {
        let Some((_, img)) = &self.notice_img else { return };
        let pad = ui.pt(8.0);
        let (width, height) = (img.w as f32 + 2.0 * pad, img.h as f32 + 2.0 * pad);
        let x = (s.w as f32 - width) / 2.0;
        let y = s.h as f32 - height - ui.pt(12.0);
        s.fill_rrect(x, y, width, height, height / 2.0, rgba(0, 0, 0, 160));
        s.blit(img, (x + pad) as i32, (y + pad) as i32, 255, 255);
    }

    fn draw_empty(&self, s: &mut Surface) {
        let (w, h) = (s.w, s.h);
        s.fill_rect(0, 0, w, h, BACKGROUND);
        if let Some(img) = &self.empty {
            s.blit(img, (w as i32 - img.w as i32) / 2, (h as i32 - img.h as i32) / 2, 255, 255);
        }
    }
}

impl App for VideoPlayerApp {
    fn id(&self) -> &str {
        "is.olibuijr.videoplayer.desktop"
    }

    fn configure(&mut self, ctx: &Ctx) {
        self.w = ctx.cfg.w as i32;
        self.h = ctx.cfg.h as i32;
        self.empty = Some(ctx.ui.text("Open a movie from the Movie menu.", EMPTY_PT, Weight::Regular, WHITE));
        if let Some(m) = &mut self.movie {
            m.set_size(self.w, self.h);
        }
        if let Some(path) = self.pending.take() {
            let title = title_of(&path);
            self.open_path(ctx, &path, &title);
        }
    }

    fn event(&mut self, ctx: &Ctx, ev: Input) -> bool {
        self.input(ctx, ev)
    }

    fn suspend(&mut self, _ctx: &Ctx) {
        // leaving the foreground ends an audio take, so nothing is lost and the microphone is released
        if self.take.is_recording() {
            self.finish_take(self.last_t);
        }
        if let Some(m) = &mut self.movie {
            m.pause();
        }
    }

    fn tick(&mut self, ctx: &Ctx, t: f64) -> bool {
        self.last_t = t;
        self.movie.as_mut().is_some_and(|m| m.tick(&ctx.ui, t))
    }

    /// One menu for the movie and one for recording, between App and the shell's File, Edit, View, Window and Help.
    fn main_menu(&self, _ctx: &Ctx) -> Vec<AppMenu> {
        let movie = self.movie.as_ref();
        let open = movie.is_some();
        let trimmed = movie.is_some_and(|m| m.trim().is_some());
        let subtitles = movie.is_some_and(|m| m.has_subtitles());
        let subtitles_on = movie.is_some_and(|m| m.subtitles_on());
        let taking = self.take.is_recording();
        vec![
            AppMenu {
                title: "Movie".into(),
                items: vec![
                    AppMenuItem::new(MENU_OPEN, "Open\u{2026}").key("cmd+o"),
                    AppMenuItem::separator(),
                    AppMenuItem::new(MENU_TRIM_START, "Set Trim Start").enabled(open),
                    AppMenuItem::new(MENU_TRIM_END, "Set Trim End").enabled(open),
                    AppMenuItem::new(MENU_TRIM_CLEAR, "Clear Trim").enabled(trimmed),
                    AppMenuItem::separator(),
                    AppMenuItem::new(MENU_SUBTITLES, "Subtitles").checked(subtitles_on).enabled(subtitles),
                ],
            },
            AppMenu {
                title: "Recording".into(),
                items: vec![
                    AppMenuItem::new(MENU_AUDIO, if taking { "Stop Audio Recording" } else { "New Audio Recording" }),
                    AppMenuItem::new(MENU_SCREEN, "New Screen Recording").enabled(self.screen.is_available()),
                ],
            },
        ]
    }

    fn menu_action(&mut self, ctx: &Ctx, id: u32) -> bool {
        let t = self.last_t;
        match id {
            MENU_OPEN => self.open_panel(ctx),
            MENU_TRIM_START => self.set_trim(true),
            MENU_TRIM_END => self.set_trim(false),
            MENU_TRIM_CLEAR => {
                if let Some(m) = &mut self.movie {
                    m.clear_trim();
                }
                self.notice.clear();
            }
            MENU_SUBTITLES => {
                if let Some(m) = &mut self.movie {
                    m.toggle_subtitles();
                }
            }
            MENU_AUDIO => self.toggle_take(ctx, t),
            MENU_SCREEN => self.notice = "Screen recording needs the screen capture service, which is not available yet.".into(),
            _ => return false,
        }
        true
    }

    /// The answer to the open panel: the chosen movie. Its file is kept open while it plays.
    fn document(&mut self, ctx: &Ctx, ev: DocumentEvent) -> bool {
        match ev {
            DocumentEvent::Opened(doc) => {
                let path = PathBuf::from(format!("/proc/self/fd/{}", doc.file.as_raw_fd()));
                let title = title_of(Path::new(&doc.name));
                if self.open_path(ctx, &path, &title) {
                    self.held = Some(doc.file);
                }
                true
            }
            _ => false,
        }
    }

    /// `open-movie`: the shell hands over a movie file by path (a finished screen recording). It plays like a movie from
    /// the open panel, but the file is opened by path, not through a document grant.
    fn intents(&self) -> Vec<Intent> {
        vec![Intent::new("open-movie", "Open Movie", "Opens a movie file in the player.").param(Param::new("path", "Path", ParamType::String))]
    }

    fn perform_intent(&mut self, ctx: Option<&Ctx>, _data_dir: &Path, id: &str, params: &Json) -> Result<IntentResult, bifrost_app::control::RpcError> {
        match id {
            "open-movie" => {
                let Some(ctx) = ctx else { return Err(intents::failed("Video Player is not running yet")) };
                let path = PathBuf::from(intents::str_param(params, "path")?);
                let title = title_of(&path);
                if self.open_path(ctx, &path, &title) {
                    Ok(IntentResult::dialog(&format!("Playing {title}")))
                } else {
                    Err(intents::failed(self.notice.clone()))
                }
            }
            _ => Err(bifrost_app::control::RpcError::new(bifrost_app::control::INVALID_PARAMS, format!("no intent {id}"))),
        }
    }

    fn draw(&mut self, ctx: &Ctx, s: &mut Surface, t: f64) -> Damage {
        self.refresh_notice(&ctx.ui);
        match &self.movie {
            Some(m) => m.draw(s, &ctx.ui, t),
            None => self.draw_empty(s),
        }
        self.draw_notice(s, &ctx.ui);
        let mut d = Damage::new(s.w, s.h);
        d.all();
        d
    }
}

fn main() {
    let pending = std::env::args().nth(1).map(PathBuf::from);
    bifrost_app::run(VideoPlayerApp::new(pending));
}
