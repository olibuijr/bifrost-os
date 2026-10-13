//! Voice Memos: record from the microphone, list, rename, delete and play
//! back WAV files. Recording (`RecordStart`/`RecordStop`) and playback go through
//! the audio service (`bifrost_app::audio`); nothing opens the microphone or the
//! speaker until the user taps Record or Play, and both stop when the app is left.

use bifrost_app::kit::ui::{SPACE_HUGE, SPACE_M, SPACE_XL, SPACE_XS, SPACE_XXL, SPACE_XXS};
use bifrost_app::kit::ui::{MAC_BODY_PT, MAC_FOOTNOTE_PT, MAC_SUBHEAD_PT, MAC_TITLE1_PT};
use crate::store::{self, Memo};
use bifrost_app::audio::{self, Track};
use bifrost_app::kit::ui::{home_zone, CARD_BG, CONTROL_PRESSED, CONTENT_BG, FIELD_BG, LABEL, LABEL_2, LABEL_3, SEPARATOR_C, SIDEBAR_BG};
use bifrost_app::controls::values::Slider;
use bifrost_app::controls::sidebar::{SidebarItem, SourceList};
use bifrost_app::layout::{Layout, SIDEBAR_FRACTION, SIDEBAR_MAX_PT, SIDEBAR_MIN_PT};
use bifrost_app::kit::ui::{DISPLAY_SMALL_PT, MAC_CALLOUT_PT, MAC_TITLE3_PT};
use bifrost_app::kit::{Event, Surface, TextCache, Ui, Weight};
use bifrost_app::controls::nav::{Toolbar, ToolbarItem, ToolbarLayout};
use bifrost_app::controls::prefs_layout::m as gg;
use bifrost_app::controls::recording::Trim;
use bifrost_app::controls::textview::TextView;
use bifrost_app::controls::{Node, Rect, Role};
use bifrost_app::layout::UNIFIED_BAR_PT;
use bifrost_app::widgets;
use crate::services::{self, Ask, Want};
use bifrost_app::audio_devices;
use bifrost_app::privacy_permissions;
use bifrost_app::controls::capture_access::Status;
use bifrost_app::now_playing::{self, CommandState, MediaContent, MediaDuration, MediaSession, MediaType, PlaybackState, RemoteCommand};
use bifrost_core::json::Json;
use springboard::deildu::scroll::Scroller;
use springboard::shell::clock::Wall;
use springboard::shell::keyboard::Out;
use springboard::shell::photos::library;
use springboard::shell::textfield::TextField;
use std::path::{Path, PathBuf};
use bifrost_app::kit::ui::{SPACE_L, SPACE_S};
use bifrost_app::kit::ui::{SYSTEM_RED, TINT};

const RED: u32 = SYSTEM_RED;
const BLUE: u32 = TINT;
/// Round button plates (transport, editor buttons) and their pressed state, in the current appearance.
fn plate() -> u32 { FIELD_BG.now() }
fn plate_down() -> u32 { CONTROL_PRESSED.now() }
/// A rectangle in points from a pixel rectangle.
fn pts(ui: &Ui, (x, y, w, h): (f32, f32, f32, f32)) -> bifrost_app::controls::Rect {
    let k = ui.pt(1.0);
    bifrost_app::controls::Rect { x: x / k, y: y / k, w: w / k, h: h / k }
}
/// The level in dB relative to full scale. The service reports the power of the block, so the amplitude is its
/// square root (the same reading the waveform uses). Silence and anything below the meter floor read as the floor.
fn level_db(level: f32) -> f32 {
    bifrost_core::meter::to_db(level.max(0.0).sqrt()).max(METER_FLOOR_DB)
}

/// The input meter's bar fraction, 0 at the floor and 1 at full scale.
fn meter_fraction(level: f32) -> f32 {
    ((level_db(level) - METER_FLOOR_DB) / -METER_FLOOR_DB).clamp(0.0, 1.0)
}

/// The meter's readout: "Silent" at the floor, otherwise the level in whole dB ("-6 dB").
fn meter_title(level: f32) -> String {
    let db = level_db(level);
    if db <= METER_FLOOR_DB {
        "Silent".into()
    } else {
        format!("{:.0} dB", db)
    }
}

/// Toolbar item ids (also the semantic control ids).
const BAR_RENAME: u32 = 1;
const BAR_DELETE: u32 = 2;
const BAR_TRIM: u32 = 3;
const BAR_NOTES: u32 = 4;
/// Menu item ids of the "Recording" app menu (`main.rs` builds it, `VoiceMemos::menu` runs it).
pub const MENU_TRIM: u32 = 1;
pub const MENU_NOTES: u32 = 2;
pub const MENU_RENAME: u32 = 3;
pub const MENU_EXPORT: u32 = 4;
pub const MENU_SPLIT: u32 = 5;
pub const MENU_SKIP: u32 = 6;
/// Enhance Recording: saves a noise-gated copy of the selected take.
pub const MENU_ENHANCE: u32 = 7;
/// Line height of the notes text (points), about 1.3 times the body size.
const NOTES_LINE_PT: f32 = 17.0;
/// Corner radius of the rename field card (prefs layout card radius, points).
const RADIUS: f32 = gg::CARD_RADIUS;
/// Height of one recording row in the sidebar source list (points, two lines: title and date).
const ROW: f32 = 72.0;
/// Top of the recordings list, below the large title (points).
const LIST_TOP: f32 = 100.0;
/// Rename and transport button sizes (points).
const BUTTON_W: f32 = 100.0;
const BUTTON_H: f32 = 44.0;
const TRANSPORT_PT: f32 = 64.0;
const RECORD_PT: f32 = 72.0;
/// Waveform top and height as shares of the detail pane's height, and the gap below it to the transport.
const WAVE_TOP_SHARE: f32 = 0.17;
const WAVE_H_SHARE: f32 = 0.12;
const TRANSPORT_GAP: f32 = 70.0;
/// Detail title and subtitle baselines, and the empty-state height (points).
const TITLE_Y: f32 = 90.0;
const SUBTITLE_Y: f32 = 130.0;
const EMPTY_H: f32 = 120.0;
/// Node ids for the controls published to accessibility (the toolbar uses 1 and 2).
const CTL_RECORD: u64 = 10;
const CTL_BACK: u64 = 11;
const CTL_PLAY: u64 = 12;
const CTL_FORWARD: u64 = 13;
const CTL_CANCEL: u64 = 14;
const CTL_DONE: u64 = 15;
const CTL_VOLUME: u64 = 17;
/// The input meter's scale in dB: silence at the floor, full scale at 0 dB.
const METER_FLOOR_DB: f32 = -60.0;
/// The input meter's bar height and the gap from the timer to the bar (points).
const METER_H_PT: f32 = 6.0;
const METER_TOP_PT: f32 = 56.0;
/// The playback volume slider: track length, track height and knob diameter (points).
const VOLUME_W_PT: f32 = 220.0;
const VOLUME_H_PT: f32 = 24.0;
const VOLUME_TRACK_PT: f32 = 4.0;
const VOLUME_KNOB_PT: f32 = 16.0;
/// The recordings sidebar as a list (rows are published by the kit's source list when there are any).
const CTL_SIDEBAR: u64 = 16;
/// Frame poll period while recording or playing (ms).
const POLL_MS: f64 = 100.0;
/// how long a started recording or playback may take to show up in the service state
const START_TIMEOUT_MS: f64 = 4_000.0;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Target {
    Row(usize),
    Record,
    Play,
    Back,
    Forward,
    Scrub,
    Rename,
    Delete,
    Trim,
    Notes,
    /// the playback volume slider under the transport (a press or drag sets the output level)
    Volume,
    /// the notes text card (a press places the caret, a drag selects)
    NotesText,
    Cancel,
    Done,
    List,
}

/// The notes of a recording open for editing: the sidecar path and the text view over its text.
struct NotesEdit {
    path: PathBuf,
    view: TextView,
}

struct Editor {
    memo: usize,
    field: TextField,
}

/// A trim in progress on the selected memo: the kept range in seconds, and which handle a drag moves
/// (`Some(true)` the start handle, `Some(false)` the end handle).
struct TrimEdit {
    memo: usize,
    range: Trim,
    drag: Option<bool>,
}

/// The media session: the commands this app supports. Next and previous are left unsupported.
fn new_session() -> Option<MediaSession> {
    let mut session = MediaSession::new("is.olibuijr.voicememos.desktop").ok()?;
    for command in [RemoteCommand::Play, RemoteCommand::Pause, RemoteCommand::Stop, RemoteCommand::TogglePlayPause, RemoteCommand::SkipForward, RemoteCommand::SkipBackward, RemoteCommand::SeekTo] {
        session.set_command(command, CommandState::Enabled);
    }
    Some(session)
}

/// The Now Playing content for a recording: its title and its length (a recording with no length is continuous).
fn content_for(memo: &Memo) -> Option<MediaContent> {
    let title = if memo.title.trim().is_empty() { "Voice Memo" } else { memo.title.as_str() };
    let duration = MediaDuration::finite_seconds(memo.duration_ms as f64 / 1000.0).unwrap_or(MediaDuration::Continuous);
    MediaContent::new(&memo.path.to_string_lossy(), title, MediaType::Audio, duration).ok()
}

pub struct VoiceMemos {
    w: i32,
    h: i32,
    dir: PathBuf,
    memos: Vec<Memo>,
    selected: Option<usize>,
    confirm_delete: bool,
    audio: audio::Client,
    /// a recording was started (`RecordStart` sent) and has not finished yet
    recording: bool,
    /// where the service writes it; the memo appears when it reports `RecordDone`
    recording_path: Option<PathBuf>,
    /// the service reported the recording active
    rec_seen: bool,
    rec_elapsed_ms: u64,
    /// the memo (by path) playback was started on
    playing: Option<PathBuf>,
    /// the service reported it playing; false while the command is in flight
    play_seen: bool,
    started_at: f64,
    now: f64,
    position_ms: f64,
    editor: Option<Editor>,
    trim: Option<TrimEdit>,
    notes: Option<NotesEdit>,
    /// a press on the notes card is dragging the selection
    notes_drag: bool,
    field_touch: bool,
    scroll: Scroller,
    scrolling: bool,
    pressed: Option<Target>,
    scrub: bool,
    peaks: Vec<f32>,
    peaks_for: Option<PathBuf>,
    live: Vec<f32>,
    live_t: f64,
    error: String,
    wall: Wall,
    dirty: bool,
    cache: TextCache,
    cache_style: (u32, u32, bool),
    /// microphone permission as the privacy service last answered it (None until an answer)
    mic: Option<Status>,
    /// the default input device, from the audio-devices service
    input_id: Option<String>,
    input_name: Option<String>,
    /// the input device when the current recording started
    record_input: Option<String>,
    /// a line about the input that replaces the microphone caption (an input change during a recording)
    notice: String,
    /// service calls the model wants made; the app drains them with `take_wants`
    wants: Vec<Want>,
    /// the media session for the playing recording
    session: Option<MediaSession>,
    /// the input level the service reported on the last `Record` event (power, 0 to 1)
    input_level: f32,
    /// the output volume (0 to 1) the slider shows: the service's level, or the value being dragged
    volume: f32,
    /// a press on the volume slider is dragging it
    volume_drag: bool,
    /// playback jumps over silence (the Recording menu's Skip Silence)
    skip_silence: bool,
    /// the speech spans of the playing memo, in milliseconds, sorted (empty when not skipping)
    spans: Vec<(f64, f64)>,
    /// playback does not look for silence before this position: a skip is in flight
    skip_guard: f64,
}

impl VoiceMemos {
    /// Lists the folder; opens no device.
    pub fn new(ui: &Ui, w: i32, h: i32, dir: PathBuf, wall: Wall, shell_dir: &Path) -> Self {
        let memos = store::scan(&dir);
        Self {
            w,
            h,
            dir,
            memos,
            selected: None,
            confirm_delete: false,
            audio: audio::Client::for_root(shell_dir),
            recording: false,
            recording_path: None,
            rec_seen: false,
            rec_elapsed_ms: 0,
            playing: None,
            play_seen: false,
            started_at: 0.0,
            now: 0.0,
            position_ms: 0.0,
            editor: None,
            trim: None,
            notes: None,
            notes_drag: false,
            field_touch: false,
            scroll: Scroller::new(),
            scrolling: false,
            pressed: None,
            scrub: false,
            peaks: Vec::new(),
            peaks_for: None,
            live: Vec::new(),
            live_t: 0.0,
            error: String::new(),
            wall,
            dirty: true,
            cache: TextCache::default(),
            cache_style: (ui.s.to_bits(), ui.text_scale.to_bits(), ui.bold),
            mic: None,
            input_id: None,
            input_name: None,
            record_input: None,
            notice: String::new(),
            wants: vec![Want::MicStatus, Want::ListDevices],
            session: new_session(),
            input_level: 0.0,
            volume: 1.0,
            volume_drag: false,
            skip_silence: false,
            spans: Vec::new(),
            skip_guard: 0.0,
        }
    }

    /// The service calls queued since the last call. The app sends them with its `services::Bus`.
    pub fn take_wants(&mut self) -> Vec<Want> {
        std::mem::take(&mut self.wants)
    }

    /// A service reply to a question this model asked. A missing service leaves the state unknown.
    pub fn reply(&mut self, ask: Ask, result: &Result<Json, String>) {
        match ask {
            Ask::Mic => {
                self.mic = result.as_ref().ok().and_then(privacy_permissions::decode_status);
                // the permission was turned off while recording: stop, so nothing is taken without it
                if self.recording && matches!(self.mic, Some(Status::Denied | Status::Restricted)) {
                    self.finish_recording();
                    self.error = "Recording stopped: microphone access was turned off".into();
                }
            }
            Ask::Devices => match result.as_ref().ok().map(audio_devices::decode_list) {
                Some(Ok(listing)) => self.set_input(&listing),
                _ => {
                    self.input_id = None;
                    self.input_name = None;
                }
            },
        }
        self.dirty = true;
    }

    /// The output route or the device list changed: ask again for the defaults.
    pub fn devices_changed(&mut self) {
        self.wants.push(Want::ListDevices);
    }

    fn set_input(&mut self, listing: &audio_devices::Listing) {
        let id = listing.default_for(audio_devices::Role::Input).map(str::to_string);
        let name = id.as_deref().and_then(|id| listing.devices.iter().find(|d| d.id == id)).map(|d| d.name.clone());
        if self.recording && id.is_some() && id != self.record_input {
            self.notice = format!("Input changed to {}", name.clone().unwrap_or_else(|| "another device".into()));
        }
        self.input_id = id;
        self.input_name = name;
    }

    /// A transport command from the shell (the media tile, a keyboard media key). Only the commands declared on the
    /// session reach here; next and previous are not declared.
    pub fn command(&mut self, command: now_playing::Command) {
        use now_playing::Command as C;
        match command {
            C::Play => {
                if self.playing.is_none() {
                    self.play();
                }
            }
            C::Pause | C::Stop => self.stop_playback(),
            C::Toggle => {
                if self.playing.is_some() {
                    self.stop_playback();
                } else {
                    self.play();
                }
            }
            C::SkipForward => self.skip(15_000.0),
            C::SkipBackward => self.skip(-15_000.0),
            C::Seek(ms) => self.seek_to(ms),
            C::Next | C::Previous => {}
        }
        self.dirty = true;
    }

    pub fn set_wall(&mut self, wall: Wall) {
        self.wall = wall;
    }

    pub fn set_size(&mut self, w: i32, h: i32) {
        if (self.w, self.h) != (w, h) {
            self.w = w;
            self.h = h;
            self.dirty = true;
        }
    }

    /// The app left the foreground: finish a recording (so nothing is lost and the microphone
    /// closes) and stop playback.
    pub fn suspend(&mut self) {
        self.end_playback();
        if self.recording {
            self.finish_recording();
        }
        self.end_notes(true);
        self.editor = None;
        self.trim = None;
        self.audio.pump();
    }

    fn reload(&mut self) {
        self.memos = store::scan(&self.dir);
        self.selected = self.selected.filter(|&i| i < self.memos.len());
    }

    fn start_recording(&mut self) {
        self.end_playback();
        self.error.clear();
        self.notice.clear();
        self.input_level = 0.0;
        // the microphone permission is checked on the last answer; this press asks again for the next one
        self.wants.push(Want::MicStatus);
        if let Err(message) = services::record_gate(self.mic) {
            self.error = message.into();
            return;
        }
        self.live.clear();
        let unix = self.wall.now(self.now);
        let stamp = library::stamp(&library::civil(unix, self.wall.offset));
        let path = store::free_path(&self.dir, &format!("Voice Memo {stamp}"), None);
        if self.audio.record_start(&path.to_string_lossy()) {
            self.recording = true;
            self.recording_path = Some(path);
            self.record_input = self.input_id.clone();
            self.rec_seen = false;
            self.rec_elapsed_ms = 0;
            self.started_at = self.now;
        } else {
            self.error = "Cannot record: the audio service is not running".into();
        }
    }

    /// Asks the service to stop; the memo is listed when it reports `RecordDone`.
    fn finish_recording(&mut self) {
        self.audio.record_stop();
    }

    fn play(&mut self) {
        let Some(memo) = self.selected.and_then(|i| self.memos.get(i)).cloned() else { return };
        let start = self.position_ms;
        self.stop_playback();
        let track = Track { id: 0, path: memo.path.to_string_lossy().into_owned(), url: None };
        if !self.audio.play_queue(vec![track], 0, false, 0) {
            self.error = "Cannot play: the audio service is not running".into();
            return;
        }
        self.playing = Some(memo.path.clone());
        self.load_spans();
        self.skip_guard = 0.0;
        self.play_seen = false;
        self.started_at = self.now;
        self.position_ms = 0.0;
        if start > 0.0 && start + 250.0 < memo.duration_ms as f64 {
            self.audio.seek_ms(start);
            self.position_ms = start;
        }
        if let Some(session) = self.session.as_mut() {
            session.set_content(content_for(&memo));
        }
        self.publish_playback();
    }

    /// Silences the service when it is playing this app's memo. The media tile stays, paused.
    fn stop_playback(&mut self) {
        if self.playing.take().is_some() {
            self.audio.stop();
            self.publish_playback();
        }
        self.spans.clear();
        self.position_ms = 0.0;
    }

    /// The speech spans of the playing memo, when Skip Silence is on; none otherwise.
    fn load_spans(&mut self) {
        self.spans = match &self.playing {
            Some(path) if self.skip_silence => store::speech_spans(path),
            _ => Vec::new(),
        };
    }

    /// Flips Skip Silence. Playback that is running picks up the spans of its memo at once.
    fn toggle_skip_silence(&mut self) {
        self.skip_silence = !self.skip_silence;
        self.load_spans();
    }

    /// While the memo plays in silence, jumps the service to the next speech. Waits out a jump still in flight.
    fn skip_silence_from(&mut self, position: f64) {
        if !self.skip_silence || position < self.skip_guard {
            return;
        }
        let Some(to) = store::next_speech_ms(&self.spans, position) else { return };
        self.audio.seek_ms(to);
        self.skip_guard = to;
        self.position_ms = to;
    }

    /// Playback is over, or its memo went away: stop it and take the session off the media tile.
    fn end_playback(&mut self) {
        self.stop_playback();
        if let Some(session) = self.session.as_mut() {
            if session.content().is_some() {
                session.set_content(None);
                self.wants.push(Want::Withdraw);
            }
        }
    }

    /// Publishes the playback state and the playhead of the session (only on a real change: play, stop, seek, skip).
    fn publish_playback(&mut self) {
        let Some(session) = self.session.as_mut() else { return };
        let state = if self.playing.is_some() { PlaybackState::Playing } else { PlaybackState::Paused };
        session.set_playback(state, self.position_ms.max(0.0) as u64);
        if let Some(body) = now_playing::publish_body(session) {
            self.wants.push(Want::Publish(body));
        }
    }

    /// Follows the service: recordings that finished, the meter, and the playing memo.
    fn sync_audio(&mut self, t: f64) {
        self.dirty |= self.audio.pump();
        self.dirty |= self.audio.tick(t);
        // the slider follows the service's output level, except while the user is dragging it
        if !self.volume_drag {
            let level = self.audio.snapshot().volume.clamp(0.0, 1.0);
            if level != self.volume {
                self.volume = level;
                self.dirty = true;
            }
        }
        for (path, _ms) in self.audio.take_recordings() {
            if self.recording_path.as_deref().is_some_and(|p| p.to_string_lossy() == path) {
                self.recording = false;
                self.recording_path = None;
                self.reload();
                self.selected = self.memos.iter().position(|m| m.path.to_string_lossy() == path);
                self.dirty = true;
            }
        }
        if self.recording {
            match self.audio.recording().cloned() {
                Some(r) if r.active => {
                    self.rec_seen = true;
                    self.rec_elapsed_ms = r.elapsed_ms;
                    self.input_level = r.level;
                    if t - self.live_t >= POLL_MS {
                        self.live_t = t;
                        self.live.push(r.level.sqrt().min(1.0));
                    }
                    if let Some(e) = r.error {
                        self.error = format!("Recording stopped: {e}");
                        self.finish_recording();
                    }
                }
                Some(r) => {
                    // inactive without a RecordDone: the microphone could not be opened, or the take failed
                    if let Some(e) = r.error {
                        self.error = format!("Cannot record: {e}");
                        self.recording = false;
                        self.recording_path = None;
                    }
                }
                None if self.rec_seen => {
                    self.error = "Recording interrupted: the audio service restarted".into();
                    self.recording = false;
                    self.recording_path = None;
                }
                None => {}
            }
            if self.recording && !self.rec_seen && t - self.started_at >= START_TIMEOUT_MS {
                self.error = "Cannot record: the audio service did not respond".into();
                self.recording = false;
                self.recording_path = None;
            }
            self.dirty = true;
        }
        if let Some(path) = self.playing.clone() {
            let snap = self.audio.snapshot();
            let ours = snap.track.as_ref().is_some_and(|tr| Path::new(&tr.path) == path);
            let (playing, position, error) = (snap.playing, snap.position_ms, snap.error.clone());
            if ours && snap.error.is_some() {
                self.error = format!("Cannot play: {}", error.unwrap_or_default());
                self.end_playback();
            } else if ours && playing {
                self.play_seen = true;
                self.position_ms = position;
                self.skip_silence_from(position);
            } else if self.play_seen {
                // finished, paused elsewhere, or another app took the speaker
                self.end_playback();
            } else if t - self.started_at >= START_TIMEOUT_MS {
                self.error = "Cannot play: the audio service did not respond".into();
                self.end_playback();
            }
            self.dirty = true;
        }
    }

    fn begin_rename(&mut self, ui: &Ui) {
        let Some(i) = self.selected else { return };
        let mut field = TextField::new(ui, "Title", MAC_TITLE3_PT, false, store::MAX_TITLE);
        field.set_text(ui, &self.memos[i].title);
        self.end_playback();
        self.editor = Some(Editor { memo: i, field });
    }

    fn end_rename(&mut self, commit: bool) {
        let Some(ed) = self.editor.take() else { return };
        if !commit {
            return;
        }
        if let Some(memo) = self.memos.get(ed.memo).cloned() {
            match store::rename(&memo, ed.field.text()) {
                Ok(path) => {
                    self.reload();
                    self.selected = self.memos.iter().position(|m| m.path == path);
                }
                Err(e) => self.error = format!("Cannot rename: {e}"),
            }
        }
    }

    /// Opens the trim of the selected memo, starting with the whole recording. Playback stops first.
    fn begin_trim(&mut self) {
        if self.notes.is_some() {
            return;
        }
        let Some(i) = self.selected else { return };
        let Some(memo) = self.memos.get(i) else { return };
        match Trim::full(memo.duration_ms as f64 / 1000.0) {
            Some(range) => {
                self.end_playback();
                self.error.clear();
                self.trim = Some(TrimEdit { memo: i, range, drag: None });
            }
            None => self.error = "This recording is too short to trim".into(),
        }
    }

    /// Closes the trim. Done writes the kept part as a new memo (`<title> trimmed`); the original stays.
    fn end_trim(&mut self, commit: bool) {
        let Some(edit) = self.trim.take() else { return };
        let whole = edit.range.start() <= 0.0 && edit.range.end() >= edit.range.length();
        if !commit || whole {
            return;
        }
        let Some(memo) = self.memos.get(edit.memo).cloned() else { return };
        match store::save_trim(&memo, edit.range) {
            Ok(path) => {
                self.reload();
                self.selected = self.memos.iter().position(|m| m.path == path);
                self.position_ms = 0.0;
            }
            Err(e) => self.error = format!("Cannot save the trimmed recording: {e}"),
        }
    }

    /// Seconds at pixel x on the trim waveform.
    fn trim_secs(&self, ui: &Ui, x: f32) -> f64 {
        let Some(edit) = &self.trim else { return 0.0 };
        let r = self.wave_rect(ui);
        ((x - r.0) / r.2).clamp(0.0, 1.0) as f64 * edit.range.length()
    }

    /// Pixel x of `secs` on the trim waveform of a recording `length` seconds long.
    fn trim_x(&self, ui: &Ui, secs: f64, length: f64) -> f32 {
        let r = self.wave_rect(ui);
        r.0 + r.2 * (secs / length).clamp(0.0, 1.0) as f32
    }

    /// A press on the waveform in trim mode grabs the handle nearer to it.
    fn trim_grab(&mut self, ui: &Ui, x: f32) {
        let secs = self.trim_secs(ui, x);
        if let Some(edit) = self.trim.as_mut() {
            edit.drag = Some((secs - edit.range.start()).abs() <= (secs - edit.range.end()).abs());
        }
        self.trim_move(ui, x);
    }

    /// Drags the grabbed handle to x. The model keeps it inside the recording and at least `MIN_TRIM` from the other.
    fn trim_move(&mut self, ui: &Ui, x: f32) {
        let secs = self.trim_secs(ui, x);
        let Some(edit) = self.trim.as_mut() else { return };
        let Some(start) = edit.drag else { return };
        if start {
            edit.range.set_start(secs);
        } else {
            edit.range.set_end(secs);
        }
        self.dirty = true;
    }

    fn delete_selected(&mut self) {
        let Some(memo) = self.selected.and_then(|i| self.memos.get(i)).cloned() else { return };
        self.end_playback();
        match std::fs::remove_file(&memo.path) {
            Ok(()) => {
                let _ = std::fs::remove_file(store::notes_path(&memo.path));
                self.selected = None;
                self.reload();
            }
            Err(e) => self.error = format!("Cannot delete: {e}"),
        }
    }

    /// True when the menu's recording commands apply: a recording is selected and nothing is being recorded or edited.
    pub fn has_selection(&self) -> bool {
        self.selected.is_some() && !self.recording && self.editor.is_none() && self.trim.is_none() && self.notes.is_none()
    }

    /// Whether the selected recording is the one playing, so a split can be made at the playhead.
    pub fn can_split(&self) -> bool {
        let selected = self.selected.and_then(|i| self.memos.get(i));
        self.has_selection() && selected.is_some_and(|m| self.playing.as_deref() == Some(m.path.as_path()))
    }

    /// Whether Skip Silence is on (the menu's check mark).
    pub fn skip_silence_on(&self) -> bool {
        self.skip_silence
    }

    /// Runs a "Recording" menu item. False for an id this app does not declare.
    pub fn menu(&mut self, ui: &Ui, id: u32) -> bool {
        if !matches!(id, MENU_TRIM | MENU_NOTES | MENU_RENAME | MENU_EXPORT | MENU_SPLIT | MENU_SKIP | MENU_ENHANCE) {
            return false;
        }
        if id == MENU_SKIP {
            self.toggle_skip_silence();
        } else if self.has_selection() {
            match id {
                MENU_TRIM => self.begin_trim(),
                MENU_NOTES => self.begin_notes(ui),
                MENU_RENAME => self.begin_rename(ui),
                MENU_SPLIT => self.split_selected(),
                MENU_ENHANCE => self.enhance_selected(),
                _ => self.export_selected(),
            }
        }
        self.dirty = true;
        true
    }

    /// Splits the playing recording at the playhead into two recordings, and selects the first part. The original
    /// stays. Playback stops, as a split changes the list.
    fn split_selected(&mut self) {
        if !self.can_split() {
            return;
        }
        let Some(memo) = self.selected.and_then(|i| self.memos.get(i)).cloned() else { return };
        let at = self.position_ms / 1000.0;
        self.stop_playback();
        match store::save_split(&memo, at) {
            Ok((first, second)) => {
                self.error.clear();
                self.reload();
                self.selected = self.memos.iter().position(|m| m.path == first);
                let name = |p: &Path| p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                self.notice = format!("Split into {} and {}", name(&first), name(&second));
            }
            Err(e) => {
                self.reload();
                self.error = format!("Cannot split: {e}");
            }
        }
    }

    /// Saves a noise-gated copy of the selected recording as `<title> enhanced` and selects it. Playback stops, as the
    /// list changes. The original stays.
    fn enhance_selected(&mut self) {
        let Some(memo) = self.selected.and_then(|i| self.memos.get(i)).cloned() else { return };
        self.stop_playback();
        match store::save_enhanced(&memo) {
            Ok(path) => {
                self.error.clear();
                self.reload();
                self.selected = self.memos.iter().position(|m| m.path == path);
                self.position_ms = 0.0;
                let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                self.notice = format!("Enhanced copy saved as {name}");
            }
            Err(e) => {
                self.reload();
                self.error = format!("Cannot enhance: {e}");
            }
        }
    }

    /// Copies the selected recording to the Downloads folder; the original stays.
    fn export_selected(&mut self) {
        let Some(memo) = self.selected.and_then(|i| self.memos.get(i)).cloned() else { return };
        let Some(home) = std::env::var_os("HOME") else {
            self.error = "Cannot export: there is no home folder".into();
            return;
        };
        let dir = PathBuf::from(home).join("Downloads");
        match store::export_copy(&memo, &dir) {
            Ok(path) => {
                self.error.clear();
                self.notice = format!("Exported to {}", path.display());
            }
            Err(e) => self.error = format!("Cannot export: {e}"),
        }
    }

    /// Opens the notes of the selected recording in the notes card. Playback stops first.
    fn begin_notes(&mut self, ui: &Ui) {
        if self.recording || self.editor.is_some() || self.trim.is_some() {
            return;
        }
        let Some(memo) = self.selected.and_then(|i| self.memos.get(i)).cloned() else { return };
        self.end_playback();
        self.error.clear();
        let (width, viewport) = self.notes_metrics(ui);
        let view = TextView::new(&store::read_notes(&memo.path), width, NOTES_LINE_PT, viewport);
        self.notes = Some(NotesEdit { path: memo.path.clone(), view });
    }

    /// Closes the notes. Done saves them next to the recording; Cancel discards the edits.
    fn end_notes(&mut self, commit: bool) {
        let Some(edit) = self.notes.take() else { return };
        if commit {
            if let Err(e) = store::write_notes(&edit.path, edit.view.text()) {
                self.error = format!("Cannot save the notes: {e}");
            }
        }
        self.dirty = true;
    }

    /// Keys in the notes card: typing, Enter for a new line, Backspace, and the arrows.
    fn notes_key(&mut self, ui: &Ui, out: &Out) {
        let k = ui.pt(1.0);
        let measure = |t: &str| ui.measure(t, MAC_BODY_PT, Weight::Regular) / k;
        let Some(edit) = self.notes.as_mut() else { return };
        let view = &mut edit.view;
        match out {
            Out::Text(text) => {
                view.insert(text, &measure);
            }
            Out::Enter => {
                view.insert("\n", &measure);
            }
            Out::Backspace => {
                view.delete_backward(&measure);
            }
            Out::Named(name) => match *name {
                "Left" => view.move_horizontal(false, false, &measure),
                "Right" => view.move_horizontal(true, false, &measure),
                "Up" => {
                    view.move_vertical(false, false, &measure);
                }
                "Down" => {
                    view.move_vertical(true, false, &measure);
                }
                _ => {}
            },
            _ => {}
        }
    }

    /// Puts the notes caret at the point (pixels); `extend` drags the selection from the anchor.
    fn notes_place(&mut self, ui: &Ui, x: f32, y: f32, extend: bool) {
        let (b, pad, k) = (self.notes_box(ui), ui.pt(SPACE_M), ui.pt(1.0));
        let measure = |t: &str| ui.measure(t, MAC_BODY_PT, Weight::Regular) / k;
        if let Some(edit) = self.notes.as_mut() {
            let byte = edit.view.hit_test((x - b.0 - pad) / k, (y - b.1 - pad) / k, &measure);
            edit.view.place_caret(byte, extend);
            edit.view.scroll_to_caret(&measure);
        }
        self.dirty = true;
    }

    pub fn key(&mut self, ui: &Ui, out: Out, t: f64) {
        if self.notes.is_some() {
            self.notes_key(ui, &out);
        } else if self.editor.is_some() {
            if matches!(out, Out::Hide | Out::Enter) {
                self.end_rename(matches!(out, Out::Enter));
            } else if let Some(ed) = &mut self.editor {
                ed.field.edit(ui, &out, t);
            }
        }
        self.dirty = true;
    }

    // layout ------------------------------------------------------------

    fn list_bottom(&self, ui: &Ui) -> f32 {
        self.h as f32 - home_zone(ui)
    }

    fn margin(&self, ui: &Ui) -> f32 {
        ui.pt(SPACE_XL)
    }

    /// Width of the recordings sidebar in points: the kit sidebar share of the window, within the kit bounds.
    fn side_pt(&self, ui: &Ui) -> f32 {
        let w = self.w as f32 / ui.pt(1.0);
        (w * SIDEBAR_FRACTION).clamp(SIDEBAR_MIN_PT, SIDEBAR_MAX_PT).min(w)
    }

    /// Width of the recordings sidebar in pixels.
    fn side_w(&self, ui: &Ui) -> f32 {
        ui.pt(self.side_pt(ui))
    }

    /// The sidebar's list area in points: below the large title, down to the home zone.
    fn column(&self, ui: &Ui) -> bifrost_app::controls::Rect {
        let p = ui.pt(1.0);
        bifrost_app::controls::Rect { x: 0.0, y: LIST_TOP, w: self.side_pt(ui), h: ((self.h as f32 - home_zone(ui)) / p - LIST_TOP).max(0.0) }
    }

    /// The recordings as a Golden Gate source list: title and date, the duration (or position while playing) trailing.
    fn sidebar(&self) -> SourceList {
        let items = self.memos.iter().enumerate().map(|(i, memo)| {
            let day = library::day_title(&library::civil(memo.mtime, self.wall.offset));
            let shown = if self.playing.as_deref() == Some(memo.path.as_path()) {
                format!("{} / {}", store::clock(self.position_ms as u64), store::clock(memo.duration_ms))
            } else {
                store::clock(memo.duration_ms)
            };
            SidebarItem::row(i as u32, &memo.title).with_subtitle(&day).with_trailing(&shown)
        }).collect();
        let mut list = SourceList::new(items, ROW, ROW);
        list.two_line_h = ROW;
        list.set_selected(self.selected.map(|i| i as u32));
        list
    }

    /// The kit layout of the sidebar column (for its large title), in points.
    fn sidebar_layout(&self, ui: &Ui) -> Layout {
        Layout { sidebar: 0.0, ..Layout::from_points(self.side_pt(ui), self.h as f32 / ui.pt(1.0), 0.0, 0.0) }
    }

    /// The detail pane's x origin and width.
    fn pane(&self, ui: &Ui) -> (f32, f32) {
        let x = self.side_w(ui);
        (x, self.w as f32 - x)
    }

    fn record_rect(&self, ui: &Ui) -> (f32, f32, f32, f32) {
        let size = ui.pt(RECORD_PT);
        let (x, w) = self.pane(ui);
        (x + (w - size) / 2.0, self.list_bottom(ui) - ui.pt(SPACE_HUGE) - size, size, size)
    }

    fn wave_rect(&self, ui: &Ui) -> (f32, f32, f32, f32) {
        let (x, w) = self.pane(ui);
        let m = self.margin(ui);
        let h = self.h as f32 / ui.pt(1.0);
        (x + m, ui.pt(h * WAVE_TOP_SHARE), w - 2.0 * m, ui.pt(h * WAVE_H_SHARE))
    }

    /// Skip back 15 s (0), play/pause (1), skip forward 15 s (2).
    fn transport_rect(&self, ui: &Ui, n: usize) -> (f32, f32, f32, f32) {
        let (x, w) = self.pane(ui);
        let (size, gap) = (ui.pt(TRANSPORT_PT), ui.pt(SPACE_HUGE));
        let left = x + (w - 3.0 * size - 2.0 * gap) / 2.0;
        let wave = self.wave_rect(ui);
        (left + (size + gap) * n as f32, wave.1 + wave.3 + ui.pt(TRANSPORT_GAP), size, size)
    }

    /// The playback volume track in pixels: centred in the pane, under the transport.
    fn volume_rect(&self, ui: &Ui) -> (f32, f32, f32, f32) {
        let (x, w) = self.pane(ui);
        let track_w = ui.pt(VOLUME_W_PT);
        let play = self.transport_rect(ui, 1);
        (x + (w - track_w) / 2.0, play.1 + play.3 + ui.pt(SPACE_XL), track_w, ui.pt(VOLUME_H_PT))
    }

    /// Sets the output level from a pointer at `x` (pixels) on the volume track. The track's value mapping is the
    /// slider model's (0 at the left end, 1 at the right end).
    fn volume_at(&mut self, ui: &Ui, x: f32) {
        let mut slider = Slider::new(0.0, 1.0, self.volume, None);
        slider.drag(pts(ui, self.volume_rect(ui)), x / ui.pt(1.0));
        self.set_volume(slider.value());
    }

    /// Sends the output level to the audio service, which is the level Control Center shows as well.
    fn set_volume(&mut self, level: f32) {
        self.volume = level.clamp(0.0, 1.0);
        self.audio.set_volume(self.volume);
        self.dirty = true;
    }

    /// The detail pane's unified toolbar: Rename and Delete (Confirm Delete while asking).
    fn bar(&self) -> Toolbar {
        let on = self.selected.is_some() && !self.recording;
        let delete = if self.confirm_delete { ToolbarItem::button(BAR_DELETE, "Confirm Delete") } else { ToolbarItem::icon(BAR_DELETE, "Delete", "trash") };
        Toolbar::new(vec![
            ToolbarItem::flexible_space(),
            ToolbarItem::button(BAR_TRIM, "Trim").enabled(on),
            ToolbarItem::button(BAR_NOTES, "Notes").enabled(on),
            ToolbarItem::button(BAR_RENAME, "Rename").enabled(on),
            delete.enabled(on),
        ])
    }

    /// The toolbar's rectangle in points: the top band of the detail pane.
    fn bar_bounds(&self, ui: &Ui) -> Rect {
        let p = ui.pt(1.0);
        let x = self.side_w(ui) / p + SPACE_M;
        Rect { x, y: 0.0, w: (self.w as f32 / p - x - SPACE_M).max(0.0), h: UNIFIED_BAR_PT }
    }

    /// The toolbar laid out as `widgets::toolbar` draws it (titled buttons measured at body size).
    fn bar_layout(&self, ui: &Ui) -> ToolbarLayout {
        let width = |it: &ToolbarItem| {
            if it.symbol.is_some() { widgets::CAPSULE_PT } else { (ui.measure(&it.label, MAC_BODY_PT, Weight::Regular) / ui.pt(1.0) + 3.0 * SPACE_S).max(widgets::CAPSULE_PT) }
        };
        let h = widgets::CAPSULE_PT;
        let b = self.bar_bounds(ui);
        self.bar().layout_with(Rect { y: b.y + (b.h - h) / 2.0, h, ..b }, width, widgets::CAPSULE_PT, widgets::TOOLBAR_GAP_PT)
    }

    /// Toolbar item `id` in pixels (empty when it overflowed).
    fn bar_rect(&self, ui: &Ui, id: u32) -> (f32, f32, f32, f32) {
        self.bar_layout(ui).rect_of(id).map_or((0.0, 0.0, 0.0, 0.0), |r| (ui.pt(r.x), ui.pt(r.y), ui.pt(r.w), ui.pt(r.h)))
    }

    /// Toolbar buttons as semantic controls (points), so the shell does not drag the window over them.
    pub fn controls(&self, ui: &Ui) -> Vec<Node> {
        let mut nodes = Vec::new();
        if self.editor.is_some() || self.trim.is_some() || self.notes.is_some() {
            let button =|id: u64, label: &str, r: (f32, f32, f32, f32)| Node { id, role: Role::Button, label: label.into(), value: String::new(), bounds: pts(ui, r), enabled: true };
            nodes.push(button(CTL_CANCEL, "Cancel", self.cancel_rect(ui)));
            nodes.push(button(CTL_DONE, "Done", self.done_rect(ui)));
            return nodes;
        }
        // the recordings sidebar is a list even while it has no rows (the empty state), so it is published as one
        nodes.push(Node { id: CTL_SIDEBAR, role: Role::List, label: "Recordings".into(), value: self.memos.len().to_string(), bounds: self.column(ui), enabled: true });
        let bar = self.bar();
        nodes.extend(self.bar_layout(ui).visible.iter().filter_map(|(id, r)| {
            let item = bar.items.iter().find(|i| i.id == *id)?;
            Some(Node { id: *id as u64, role: Role::Button, label: item.label.clone(), value: String::new(), bounds: *r, enabled: item.enabled })
        }));
        // the record button, and the transport while a memo is open
        let button = |id: u64, label: &str, r: (f32, f32, f32, f32), enabled: bool| Node { id, role: Role::Button, label: label.into(), value: String::new(), bounds: pts(ui, r), enabled };
        let record = if self.recording { "Stop Recording" } else { "Record" };
        nodes.push(button(CTL_RECORD, record, self.record_rect(ui), true));
        if !self.recording && self.selected.is_some() {
            nodes.push(button(CTL_BACK, "Skip Back 15 Seconds", self.transport_rect(ui, 0), true));
            let play = if self.playing.is_some() { "Pause" } else { "Play" };
            nodes.push(button(CTL_PLAY, play, self.transport_rect(ui, 1), true));
            nodes.push(button(CTL_FORWARD, "Skip Forward 15 Seconds", self.transport_rect(ui, 2), true));
            let percent = (self.volume * 100.0).round() as u32;
            nodes.push(Node { id: CTL_VOLUME, role: Role::Slider, label: "Volume".into(), value: format!("{percent} percent"), bounds: pts(ui, self.volume_rect(ui)), enabled: true });
        }
        nodes
    }

    fn rename_rect(&self, ui: &Ui) -> (f32, f32, f32, f32) {
        self.bar_rect(ui, BAR_RENAME)
    }

    fn delete_rect(&self, ui: &Ui) -> (f32, f32, f32, f32) {
        self.bar_rect(ui, BAR_DELETE)
    }

    /// The rename sheet's buttons sit below the unified bar, clear of the traffic lights.
    fn cancel_rect(&self, ui: &Ui) -> (f32, f32, f32, f32) {
        (self.margin(ui), ui.pt(UNIFIED_BAR_PT), ui.pt(BUTTON_W), ui.pt(BUTTON_H))
    }

    fn done_rect(&self, ui: &Ui) -> (f32, f32, f32, f32) {
        (self.w as f32 - self.margin(ui) - ui.pt(BUTTON_W), ui.pt(UNIFIED_BAR_PT), ui.pt(BUTTON_W), ui.pt(BUTTON_H))
    }

    fn field_rect(&self, ui: &Ui) -> (f32, f32, f32, f32) {
        let m = self.margin(ui);
        (m + ui.pt(SPACE_L), ui.pt(UNIFIED_BAR_PT + 68.0), self.w as f32 - 2.0 * m - ui.pt(28.0), ui.pt(52.0))
    }

    fn inside((x, y, w, h): (f32, f32, f32, f32), px: f32, py: f32) -> bool {
        px >= x && px < x + w && py >= y && py < y + h
    }

    fn hit(&self, ui: &Ui, x: f32, y: f32) -> Option<Target> {
        if y >= self.h as f32 - home_zone(ui) {
            return None;
        }
        if self.notes.is_some() {
            if Self::inside(self.cancel_rect(ui), x, y) {
                return Some(Target::Cancel);
            }
            if Self::inside(self.done_rect(ui), x, y) {
                return Some(Target::Done);
            }
            return Self::inside(self.notes_box(ui), x, y).then_some(Target::NotesText);
        }
        if self.trim.is_some() {
            if Self::inside(self.cancel_rect(ui), x, y) {
                return Some(Target::Cancel);
            }
            if Self::inside(self.done_rect(ui), x, y) {
                return Some(Target::Done);
            }
            let w = self.wave_rect(ui);
            if Self::inside((w.0, w.1 - ui.pt(SPACE_XL), w.2, w.3 + ui.pt(SPACE_HUGE)), x, y) {
                return Some(Target::Scrub);
            }
            return None;
        }
        if self.editor.is_some() {
            if Self::inside(self.cancel_rect(ui), x, y) {
                return Some(Target::Cancel);
            }
            return Self::inside(self.done_rect(ui), x, y).then_some(Target::Done);
        }
        if Self::inside(self.record_rect(ui), x, y) {
            return Some(Target::Record);
        }
        if self.recording {
            return None;
        }
        if self.selected.is_some() {
            for (target, r) in [
                (Target::Trim, self.bar_rect(ui, BAR_TRIM)),
                (Target::Notes, self.bar_rect(ui, BAR_NOTES)),
                (Target::Rename, self.rename_rect(ui)),
                (Target::Delete, self.delete_rect(ui)),
                (Target::Back, self.transport_rect(ui, 0)),
                (Target::Play, self.transport_rect(ui, 1)),
                (Target::Forward, self.transport_rect(ui, 2)),
                (Target::Volume, self.volume_rect(ui)),
            ] {
                if Self::inside(r, x, y) {
                    return Some(target);
                }
            }
            let w = self.wave_rect(ui);
            if Self::inside((w.0, w.1 - ui.pt(SPACE_XL), w.2, w.3 + ui.pt(SPACE_HUGE)), x, y) {
                return Some(Target::Scrub);
            }
        }
        if x < self.side_w(ui) && y >= ui.pt(LIST_TOP) && y < self.list_bottom(ui) {
            let p = ui.pt(1.0);
            if let Some(id) = self.sidebar().hit(self.column(ui), self.scroll.offset() / p, x / p, y / p) {
                return Some(Target::Row(id as usize));
            }
            return Some(Target::List);
        }
        None
    }

    /// Moves the playhead to the x position on the waveform (seeks when a player is open).
    fn scrub_to(&mut self, ui: &Ui, x: f32) {
        let Some(memo) = self.selected.and_then(|i| self.memos.get(i)) else { return };
        let r = self.wave_rect(ui);
        let ms = ((x - r.0) / r.2).clamp(0.0, 1.0) as f64 * memo.duration_ms as f64;
        self.position_ms = ms;
        if self.playing.is_some() {
            self.audio.seek_ms(ms);
        }
        self.dirty = true;
    }

    fn skip(&mut self, delta_ms: f64) {
        self.seek_to(self.position_ms + delta_ms);
    }

    /// Moves the playhead to `ms` within the selected recording, seeking the speaker when it plays, and publishes it.
    fn seek_to(&mut self, ms: f64) {
        let Some(dur) = self.selected.and_then(|i| self.memos.get(i)).map(|m| m.duration_ms as f64) else { return };
        self.position_ms = ms.clamp(0.0, dur);
        if self.playing.is_some() {
            self.audio.seek_ms(self.position_ms);
        }
        self.publish_playback();
    }

    fn activate(&mut self, ui: &Ui, target: Target) {
        if target != Target::Delete {
            self.confirm_delete = false;
        }
        match target {
            Target::Record => {
                if self.recording {
                    self.finish_recording();
                } else {
                    self.start_recording();
                }
            }
            Target::Row(i) => {
                if self.selected != Some(i) {
                    self.end_playback();
                }
                self.selected = Some(i);
                self.position_ms = if self.playing.is_some() { self.position_ms } else { 0.0 };
                self.error.clear();
            }
            Target::Play => {
                if self.playing.is_some() {
                    self.stop_playback();
                } else {
                    self.play();
                }
            }
            Target::Back => self.skip(-15_000.0),
            Target::Forward => self.skip(15_000.0),
            Target::Scrub | Target::Volume => {}
            Target::Trim => self.begin_trim(),
            Target::Notes => self.begin_notes(ui),
            Target::NotesText => {}
            Target::Cancel if self.notes.is_some() => self.end_notes(false),
            Target::Done if self.notes.is_some() => self.end_notes(true),
            Target::Rename => self.begin_rename(ui),
            Target::Delete => {
                if self.confirm_delete {
                    self.confirm_delete = false;
                    self.delete_selected();
                } else {
                    self.confirm_delete = true;
                }
            }
            Target::Cancel if self.trim.is_some() => self.end_trim(false),
            Target::Done if self.trim.is_some() => self.end_trim(true),
            Target::Cancel => self.end_rename(false),
            Target::Done => self.end_rename(true),
            Target::List => {}
        }
        self.dirty = true;
    }

    pub fn touch(&mut self, ui: &Ui, event: Event, t: f64) {
        let (x, y) = match event {
            Event::Down(x, y) | Event::Move(x, y) | Event::Up(x, y) => (x, y),
            _ => return,
        };
        match event {
            Event::Down(..) => {
                self.dirty = true;
                if self.editor.is_some() {
                    let frame = self.field_rect(ui);
                    let w = self.w as f32;
                    if let Some(ed) = &mut self.editor {
                        if ed.field.touch(ui, event, frame, w, t) {
                            self.field_touch = true;
                            return;
                        }
                    }
                }
                self.pressed = self.hit(ui, x, y);
                if self.pressed == Some(Target::Volume) {
                    self.volume_drag = true;
                    self.volume_at(ui, x);
                    return;
                }
                if self.pressed == Some(Target::NotesText) {
                    self.notes_drag = true;
                    self.notes_place(ui, x, y, false);
                    return;
                }
                if self.pressed == Some(Target::Scrub) {
                    self.scrub = true;
                    if self.trim.is_some() {
                        self.trim_grab(ui, x);
                    } else {
                        self.scrub_to(ui, x);
                    }
                    return;
                }
                if matches!(self.pressed, Some(Target::Row(_) | Target::List)) {
                    self.scrolling = true;
                    self.scroll.down(y, t);
                }
            }
            Event::Move(..) => {
                if self.notes_drag {
                    self.notes_place(ui, x, y, true);
                } else if self.field_touch {
                    let (frame, w) = (self.field_rect(ui), self.w as f32);
                    if let Some(ed) = &mut self.editor {
                        ed.field.touch(ui, event, frame, w, t);
                    }
                    self.dirty = true;
                } else if self.volume_drag {
                    self.volume_at(ui, x);
                } else if self.scrub {
                    if self.trim.is_some() {
                        self.trim_move(ui, x);
                    } else {
                        self.scrub_to(ui, x);
                    }
                } else if self.scrolling {
                    self.scroll.moved(y, t);
                    if self.scroll.dragging() {
                        self.pressed = None;
                    }
                    self.dirty = true;
                } else if self.pressed.is_some() && self.hit(ui, x, y) != self.pressed {
                    self.pressed = None;
                    self.dirty = true;
                }
            }
            Event::Up(..) => {
                self.dirty = true;
                if std::mem::take(&mut self.notes_drag) {
                    self.pressed = None;
                    return;
                }
                if std::mem::take(&mut self.field_touch) {
                    let (frame, w) = (self.field_rect(ui), self.w as f32);
                    if let Some(ed) = &mut self.editor {
                        ed.field.touch(ui, event, frame, w, t);
                    }
                    return;
                }
                if std::mem::take(&mut self.volume_drag) {
                    self.pressed = None;
                    return;
                }
                if std::mem::take(&mut self.scrub) {
                    self.pressed = None;
                    if let Some(edit) = self.trim.as_mut() {
                        edit.drag = None;
                    }
                    return;
                }
                let tap = if std::mem::take(&mut self.scrolling) { self.scroll.up(y, t) } else { true };
                let pressed = self.pressed.take();
                if tap {
                    if let Some(target) = pressed.filter(|&p| self.hit(ui, x, y) == Some(p)) {
                        self.activate(ui, target);
                    }
                }
            }
            _ => {}
        }
    }

    pub fn tick(&mut self, ui: &Ui, t: f64) -> bool {
        self.now = t;
        self.scroll.set_max((ui.pt(LIST_TOP + ROW * self.memos.len() as f32) - self.list_bottom(ui)).max(0.0));
        self.dirty |= self.scroll.tick(t);
        if let Some(ed) = &mut self.editor {
            self.dirty |= ed.field.tick(ui, t);
            if ed.field.focused {
                self.dirty = true;
            }
        }
        self.sync_audio(t);
        self.dirty
    }

    pub fn next_wake(&self, t: f64) -> Option<f64> {
        let mut due = (self.recording || self.playing.is_some()).then_some(t + POLL_MS);
        if due.is_none() {
            due = self.audio.next_wake(t);
        }
        for wake in [self.editor.as_ref().and_then(|e| e.field.next_wake(t)), self.scroll.animating().then_some(t)] {
            if let Some(w) = wake {
                due = Some(due.map_or(w, |d| d.min(w)));
            }
        }
        due
    }

    // drawing -------------------------------------------------------------

    fn text(&mut self, s: &mut Surface, ui: &Ui, text: &str, pt: f32, weight: Weight, color: u32, x: f32, y: f32) {
        let image = self.cache.get(ui, text, pt, weight, color);
        s.blit(image, x as i32, y as i32, 255, 255);
    }

    fn centred(&mut self, s: &mut Surface, ui: &Ui, text: &str, pt: f32, color: u32, (x, y, w, h): (f32, f32, f32, f32)) {
        let image = self.cache.get(ui, text, pt, Weight::Regular, color);
        s.blit(image, (x + (w - image.w as f32) / 2.0) as i32, (y + (h - image.h as f32) / 2.0) as i32, 255, 255);
    }

    fn button(&mut self, s: &mut Surface, ui: &Ui, target: Target, label: &str, color: u32, r: (f32, f32, f32, f32)) {
        let fill = if self.pressed == Some(target) { plate_down() } else { plate() };
        s.fill_rrect(r.0, r.1, r.2, r.3, ui.pt(RADIUS), fill);
        self.centred(s, ui, label, MAC_BODY_PT, color, r);
    }

    /// Waveform bars across `r` on a grey band (Golden Gate Voice Memos); bars before `played` (0..=1) are the
    /// label colour, the rest tertiary. A live recording (`newest_right`) is red.
    fn bars(s: &mut Surface, ui: &Ui, r: (f32, f32, f32, f32), levels: &[f32], played: f32, newest_right: bool) {
        let (step, bw) = (ui.pt(SPACE_XS), ui.pt(SPACE_XXS));
        let n = ((r.2 / step) as usize).max(1);
        let mid = r.1 + r.3 / 2.0;
        s.fill_rect(r.0 as usize, r.1 as usize, r.2 as usize, r.3 as usize, CARD_BG.now());
        for i in 0..n {
            let v = if newest_right {
                // the last `n` live levels, newest at the right edge
                let k = (levels.len() + i).checked_sub(n);
                k.and_then(|k| levels.get(k)).copied().unwrap_or(0.0)
            } else if levels.is_empty() {
                0.0
            } else {
                levels[i * levels.len() / n]
            };
            let h = (v * r.3).max(ui.pt(SPACE_XS));
            let color = if newest_right { RED } else if (i as f32 / n as f32) < played { LABEL.now() } else { LABEL_3.now() };
            s.fill_rrect(r.0 + i as f32 * step, mid - h / 2.0, bw, h, bw / 2.0, color);
        }
    }

    pub fn draw(&mut self, s: &mut Surface, ui: &Ui, t: f64) {
        self.dirty = false;
        let style = (ui.s.to_bits(), ui.text_scale.to_bits(), ui.bold);
        if style != self.cache_style {
            self.cache.clear();
            self.cache_style = style;
        }
        if self.cache.len() > 256 {
            self.cache.clear();
        }
        s.fill_rect(0, 0, self.w as usize, self.h as usize, CONTENT_BG.now());
        if self.editor.is_some() {
            self.draw_editor(s, ui, t);
            return;
        }
        if self.trim.is_some() {
            self.draw_trim(s, ui);
            return;
        }
        if self.notes.is_some() {
            self.draw_notes(s, ui);
            return;
        }
        let (px, pw) = self.pane(ui);
        let m = self.margin(ui);
        let side = self.side_w(ui);
        let bottom = self.list_bottom(ui);
        let memo = self.selected.and_then(|i| self.memos.get(i)).cloned();
        let wave = self.wave_rect(ui);
        let pane_centre = (px, 0.0, pw, 0.0);
        if self.recording {
            // live recording: timer and a scrolling waveform
            let text = store::clock(self.rec_elapsed_ms);
            let live = self.live.clone();
            self.text(s, ui, "New Recording", MAC_TITLE1_PT, Weight::Bold, LABEL.now(), px + m, ui.pt(TITLE_Y));
            Self::bars(s, ui, wave, &live, 0.0, true);
            s.fill_rect(wave.0 as usize, (wave.1 + wave.3 / 2.0) as usize, wave.2 as usize, 1, SEPARATOR_C.now());
            self.centred(s, ui, &text, DISPLAY_SMALL_PT, LABEL.now(), (pane_centre.0, wave.1 + wave.3 + ui.pt(SPACE_XL), pane_centre.2, ui.pt(48.0)));
            // the input meter under the timer: a bar on the dB scale and the level read out
            let (my, h, mw) = (wave.1 + wave.3 + ui.pt(METER_TOP_PT), ui.pt(METER_H_PT), ui.pt(VOLUME_W_PT));
            let mx = px + (pw - mw) / 2.0;
            s.fill_rrect(mx, my, mw, h, h / 2.0, LABEL_3.now());
            let fill = meter_fraction(self.input_level) * mw;
            if fill > 0.0 {
                s.fill_rrect(mx, my, fill.max(h), h, h / 2.0, BLUE);
            }
            let readout = meter_title(self.input_level);
            self.centred(s, ui, &readout, MAC_FOOTNOTE_PT, LABEL_2.now(), (px, my + h + ui.pt(SPACE_XS), pw, ui.pt(20.0)));
        } else if let Some(memo) = memo {
            if self.peaks_for.as_deref() != Some(memo.path.as_path()) {
                self.peaks = store::peaks(&memo.path, 256);
                self.peaks_for = Some(memo.path.clone());
            }
            let d = library::civil(memo.mtime, self.wall.offset);
            let title = ui.fit(&memo.title, MAC_TITLE1_PT, pw - 2.0 * m);
            self.text(s, ui, &title, MAC_TITLE1_PT, Weight::Bold, LABEL.now(), px + m, ui.pt(TITLE_Y));
            let mut sub = format!("{}  {}", library::day_title(&d), library::time_title(&d));
            let format = store::format_title(memo.rate, memo.channels);
            if !format.is_empty() {
                sub = format!("{sub}  ·  {format}");
            }
            self.text(s, ui, &sub, MAC_SUBHEAD_PT, Weight::Regular, LABEL_2.now(), px + m, ui.pt(SUBTITLE_Y));
            let dur = memo.duration_ms.max(1) as f64;
            let frac = (self.position_ms / dur).clamp(0.0, 1.0) as f32;
            let peaks = self.peaks.clone();
            Self::bars(s, ui, wave, &peaks, frac, false);
            let x = wave.0 + wave.2 * frac;
            s.fill_rrect(x - ui.pt(1.0), wave.1 - ui.pt(SPACE_M), ui.pt(SPACE_XXS), wave.3 + ui.pt(SPACE_XXL), ui.pt(SPACE_XXS) / 2.0, RED);
            let left = store::clock(self.position_ms as u64);
            let right = format!("-{}", store::clock(memo.duration_ms.saturating_sub(self.position_ms as u64)));
            let ty = wave.1 + wave.3 + ui.pt(SPACE_XXL);
            self.text(s, ui, &left, MAC_FOOTNOTE_PT, Weight::Regular, LABEL_2.now(), wave.0, ty);
            let img = self.cache.get(ui, &right, MAC_FOOTNOTE_PT, Weight::Regular, LABEL_2.now());
            s.blit(img, (wave.0 + wave.2) as i32 - img.w as i32, ty as i32, 255, 255);
            // transport: -15, play/pause, +15
            let back = self.transport_rect(ui, 0);
            let play = self.transport_rect(ui, 1);
            let fwd = self.transport_rect(ui, 2);
            for (target, r, label) in [(Target::Back, back, "-15"), (Target::Forward, fwd, "+15")] {
                let fill = if self.pressed == Some(target) { plate_down() } else { plate() };
                s.fill_rrect(r.0, r.1, r.2, r.3, r.2 / 2.0, fill);
                self.centred(s, ui, label, MAC_BODY_PT, LABEL.now(), r);
            }
            let fill = if self.pressed == Some(Target::Play) { plate_down() } else { plate() };
            s.fill_rrect(play.0, play.1, play.2, play.3, play.2 / 2.0, fill);
            let (cx, cy, g) = (play.0 + play.2 / 2.0, play.1 + play.3 / 2.0, ui.pt(26.0));
            if self.playing.is_some() {
                for dx in [-0.32f32, 0.08] {
                    s.fill_rrect(cx + g * dx, cy - g / 2.0, g * 0.24, g, g * 0.12, LABEL.now());
                }
            } else {
                let cols = g as usize;
                for c in 0..cols {
                    let h = g * (1.0 - c as f32 / cols as f32);
                    s.fill_rect((cx - g * 0.35) as usize + c, (cy - h / 2.0) as usize, 1, h as usize, LABEL.now());
                }
            }
            self.draw_volume(s, ui);
        } else {
            self.centred(s, ui, "No Recording Selected", MAC_TITLE3_PT, LABEL_2.now(), (px, wave.1, pw, wave.3));
        }
        // one line above the record button: the error in red, else the microphone caption (state and input)
        let slot = self.record_rect(ui);
        if !self.error.is_empty() {
            let msg = ui.fit(&self.error, MAC_BODY_PT, pw - 2.0 * m);
            self.centred(s, ui, &msg, MAC_CALLOUT_PT, RED, (px, slot.1 - ui.pt(44.0), pw, ui.pt(28.0)));
        } else {
            let caption = services::caption(self.mic, self.input_name.as_deref(), &self.notice);
            if !caption.is_empty() {
                let msg = ui.fit(&caption, MAC_CALLOUT_PT, pw - 2.0 * m);
                self.centred(s, ui, &msg, MAC_CALLOUT_PT, LABEL_2.now(), (px, slot.1 - ui.pt(44.0), pw, ui.pt(28.0)));
            }
        }
        // red record button: a grey ring, red disc (rounded square while recording)
        let r = self.record_rect(ui);
        let (cx, cy) = (r.0 + r.2 / 2.0, r.1 + r.3 / 2.0);
        s.fill_rrect(r.0, r.1, r.2, r.3, r.2 / 2.0, LABEL_3.now());
        let inset = ui.pt(SPACE_XS);
        s.fill_rrect(r.0 + inset, r.1 + inset, r.2 - 2.0 * inset, r.3 - 2.0 * inset, r.2 / 2.0, CONTENT_BG.now());
        let d = if self.recording { r.2 * 0.38 } else { r.2 - 2.0 * (inset + ui.pt(SPACE_XS)) };
        let corner = if self.recording { ui.pt(SPACE_S) } else { d / 2.0 };
        s.fill_rrect(cx - d / 2.0, cy - d / 2.0, d, d, corner, RED);

        // the detail pane's unified toolbar
        widgets::toolbar_band(s, ui, Rect { x: side / ui.pt(1.0), y: 0.0, w: pw / ui.pt(1.0), h: UNIFIED_BAR_PT });
        let pressed = match self.pressed {
            Some(Target::Trim) => Some(BAR_TRIM),
            Some(Target::Notes) => Some(BAR_NOTES),
            Some(Target::Rename) => Some(BAR_RENAME),
            Some(Target::Delete) => Some(BAR_DELETE),
            _ => None,
        };
        widgets::toolbar(s, ui, self.bar_bounds(ui), &self.bar(), pressed);

        // sidebar: the Golden Gate source list over the full-height shaded column, with the large title in the top
        // band (beside the traffic lights) and the home zone below. Rows scrolled under the title repaint it.
        s.fill_rect(0, 0, side as usize, self.h as usize, SIDEBAR_BG.now());
        let pressed = match self.pressed { Some(Target::Row(i)) => Some(i as u32), _ => None };
        widgets::source_list(s, ui, self.column(ui), &self.sidebar(), self.scroll.offset() / ui.pt(1.0), pressed);
        let list_top = ui.pt(LIST_TOP);
        s.fill_rect(0, 0, side as usize, list_top as usize - 1, SIDEBAR_BG.now());
        s.fill_rect(0, bottom as usize, side as usize, (self.h as f32 - bottom) as usize, SIDEBAR_BG.now());
        widgets::large_title(s, ui, &self.sidebar_layout(ui), "All Recordings", UNIFIED_BAR_PT);
        if self.memos.is_empty() {
            self.centred(s, ui, "No Recordings", MAC_TITLE3_PT, LABEL_2.now(), (0.0, list_top, side, ui.pt(EMPTY_H)));
        }
        s.fill_rect(side as usize, 0, 1, bottom as usize, SEPARATOR_C.now());
    }

    /// The playback volume slider: a track with the filled part in the tint, and a knob at the level.
    fn draw_volume(&mut self, s: &mut Surface, ui: &Ui) {
        let r = self.volume_rect(ui);
        let track = ui.pt(VOLUME_TRACK_PT);
        let ty = r.1 + (r.3 - track) / 2.0;
        s.fill_rrect(r.0, ty, r.2, track, track / 2.0, LABEL_3.now());
        let filled = r.2 * self.volume;
        if filled > 0.0 {
            s.fill_rrect(r.0, ty, filled.max(track), track, track / 2.0, BLUE);
        }
        let knob = ui.pt(VOLUME_KNOB_PT);
        let kx = r.0 + filled - knob / 2.0;
        let ky = r.1 + (r.3 - knob) / 2.0;
        s.fill_rrect(kx, ky, knob, knob, knob / 2.0, SEPARATOR_C.now());
        let inner = ui.pt(1.0);
        s.fill_rrect(kx + inner, ky + inner, knob - 2.0 * inner, knob - 2.0 * inner, (knob - 2.0 * inner) / 2.0, CARD_BG.now());
    }

    /// Trim mode: the waveform with the start and end handles, and the kept length. Cancel and Done sit where the
    /// rename sheet's do.
    fn draw_trim(&mut self, s: &mut Surface, ui: &Ui) {
        let Some(edit) = self.trim.as_ref() else { return };
        let (range, index) = (edit.range, edit.memo);
        let Some(memo) = self.memos.get(index).cloned() else { return };
        self.button(s, ui, Target::Cancel, "Cancel", BLUE, self.cancel_rect(ui));
        self.button(s, ui, Target::Done, "Done", BLUE, self.done_rect(ui));
        self.centred(s, ui, "Trim", MAC_TITLE3_PT, LABEL.now(), (0.0, ui.pt(UNIFIED_BAR_PT), self.w as f32, ui.pt(44.0)));
        let wave = self.wave_rect(ui);
        if self.peaks_for.as_deref() != Some(memo.path.as_path()) {
            self.peaks = store::peaks(&memo.path, 256);
            self.peaks_for = Some(memo.path.clone());
        }
        let peaks = self.peaks.clone();
        Self::bars(s, ui, wave, &peaks, 0.0, false);
        let length = range.length();
        let (hw, top, hh) = (ui.pt(SPACE_XXS), wave.1 - ui.pt(SPACE_M), wave.3 + ui.pt(SPACE_XXL));
        for secs in [range.start(), range.end()] {
            let x = self.trim_x(ui, secs, length);
            s.fill_rrect(x - hw / 2.0, top, hw, hh, hw / 2.0, BLUE);
        }
        let keep = format!("Keeps {} of {}", store::clock((range.duration() * 1000.0) as u64), store::clock(memo.duration_ms));
        let (px, pw) = self.pane(ui);
        self.centred(s, ui, &keep, MAC_FOOTNOTE_PT, LABEL_2.now(), (px, wave.1 + wave.3 + ui.pt(SPACE_XXL), pw, ui.pt(SPACE_XL)));
    }

    /// The notes card: the sheet's Cancel and Done, a grey card with the wrapped notes, the selection and the caret.
    fn draw_notes(&mut self, s: &mut Surface, ui: &Ui) {
        self.button(s, ui, Target::Cancel, "Cancel", BLUE, self.cancel_rect(ui));
        self.button(s, ui, Target::Done, "Done", BLUE, self.done_rect(ui));
        self.centred(s, ui, "Notes", MAC_TITLE3_PT, LABEL.now(), (0.0, ui.pt(UNIFIED_BAR_PT), self.w as f32, ui.pt(44.0)));
        let b = self.notes_box(ui);
        s.fill_rrect(b.0, b.1, b.2, b.3, ui.pt(RADIUS), CARD_BG.now());
        let (width, viewport) = self.notes_metrics(ui);
        let (k, pad, lh) = (ui.pt(1.0), ui.pt(SPACE_M), ui.pt(NOTES_LINE_PT));
        let measure = |t: &str| ui.measure(t, MAC_BODY_PT, Weight::Regular) / k;
        let Some(edit) = self.notes.as_mut() else { return };
        edit.view.resize(width, viewport, &measure);
        let lines = edit.view.lines(&measure);
        let text = edit.view.text().to_string();
        let rows: Vec<(usize, String)> = edit.view.visible_lines(&measure).map(|l| (l, text[lines[l].clone()].to_string())).collect();
        let highlights = edit.view.highlights(&measure);
        let (cx, cy) = edit.view.caret_point(&measure);
        let scroll = edit.view.scroll_y();
        let (ox, oy) = (b.0 + pad, b.1 + pad);
        // content line n sits at oy + n * line height, less the scroll
        let y_of = |line: usize| oy + ui.pt(line as f32 * NOTES_LINE_PT - scroll);
        for h in &highlights {
            let (x0, w) = (ox + ui.pt(h.x0), ui.pt(h.x1 - h.x0).max(0.0));
            s.fill_rect(x0.max(0.0) as usize, y_of(h.line).max(0.0) as usize, w as usize, lh as usize, bifrost_app::kit::ui::SELECTION.now());
        }
        for (line, row) in &rows {
            if !row.is_empty() {
                self.text(s, ui, row, MAC_BODY_PT, Weight::Regular, LABEL.now(), ox, y_of(*line));
            }
        }
        let (caret_x, caret_y) = (ox + ui.pt(cx), oy + ui.pt(cy - scroll));
        if caret_y >= oy && caret_y + lh <= b.1 + b.3 {
            s.fill_rect(caret_x.max(0.0) as usize, caret_y.max(0.0) as usize, k.max(1.0) as usize, lh as usize, LABEL.now());
        }
    }

    /// The notes card below the sheet's buttons, inset from the pane edges (pixels).
    fn notes_box(&self, ui: &Ui) -> (f32, f32, f32, f32) {
        let m = self.margin(ui);
        let top = ui.pt(UNIFIED_BAR_PT + 68.0);
        let bottom = self.h as f32 - home_zone(ui) - ui.pt(SPACE_L);
        (m + ui.pt(SPACE_L), top, self.w as f32 - 2.0 * m - ui.pt(28.0), (bottom - top).max(0.0))
    }

    /// The text width and visible height of the notes card in points (the text sits inside a margin).
    fn notes_metrics(&self, ui: &Ui) -> (f32, f32) {
        let (_, _, w, h) = self.notes_box(ui);
        let (k, pad) = (ui.pt(1.0), ui.pt(SPACE_M));
        (((w - 2.0 * pad) / k).max(1.0), ((h - 2.0 * pad) / k).max(0.0))
    }

    fn draw_editor(&mut self, s: &mut Surface, ui: &Ui, t: f64) {
        self.button(s, ui, Target::Cancel, "Cancel", BLUE, self.cancel_rect(ui));
        self.button(s, ui, Target::Done, "Done", BLUE, self.done_rect(ui));
        self.centred(s, ui, "Rename", MAC_TITLE3_PT, LABEL.now(), (0.0, ui.pt(UNIFIED_BAR_PT), self.w as f32, ui.pt(44.0)));
        let f = self.field_rect(ui);
        let pad = ui.pt(SPACE_L);
        s.fill_rrect(f.0 - pad, f.1, f.2 + 2.0 * pad, f.3, ui.pt(RADIUS), CARD_BG.now());
        if let Some(ed) = &self.editor {
            ed.field.draw(s, ui, f, t);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(name: &str) -> (VoiceMemos, Ui) {
        let ui = springboard::shell::tests::ui();
        let dir = std::env::temp_dir().join(format!("voicememos-ui-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        (VoiceMemos::new(&ui, 1200, 1920, dir.clone(), Wall::default(), &dir), ui)
    }

    #[test]
    fn opening_the_app_opens_no_device() {
        let (app, _) = app("idle");
        assert!(!app.recording && app.playing.is_none());
        assert!(app.next_wake(0.0).is_none(), "nothing polls until the user acts");
    }

    #[test]
    fn rename_and_delete_follow_the_selection() {
        let (mut app, ui) = app("flow");
        let wav = springboard::media::capture::wav_bytes(&springboard::media::capture::pcm16(8000, 1), &vec![0; 4000]);
        store::save(&app.dir, "20261004-120000", &wav, 100).unwrap();
        app.reload();
        assert_eq!(app.memos.len(), 1);
        let r = app.sidebar().item_rect(0, app.column(&ui), 0.0);
        let (x, y) = (ui.pt(r.x + 10.0), ui.pt(r.y + 10.0));
        app.touch(&ui, Event::Down(x, y), 0.0);
        app.touch(&ui, Event::Up(x, y), 10.0);
        assert_eq!(app.selected, Some(0));
        app.begin_rename(&ui);
        app.editor.as_mut().unwrap().field.set_text(&ui, "Standup");
        app.end_rename(true);
        assert_eq!(app.memos[0].title, "Standup");
        app.activate(&ui, Target::Delete);
        assert!(app.confirm_delete && app.memos.len() == 1, "the first tap only asks");
        app.activate(&ui, Target::Delete);
        assert!(app.memos.is_empty());
        let _ = std::fs::remove_dir_all(&app.dir);
    }

    #[test]
    fn trim_saves_the_kept_part_as_a_new_memo() {
        let (mut app, ui) = app("trim");
        let samples: Vec<i16> = (0..8000).map(|i| i as i16).collect();
        store::save(&app.dir, "20261004-120000", &springboard::media::capture::wav_bytes(&springboard::media::capture::pcm16(8000, 1), &samples), 100).unwrap();
        app.reload();
        app.selected = Some(0);
        app.activate(&ui, Target::Trim);
        assert!(app.trim.is_some(), "Trim opens the trim view");
        if let Some(edit) = app.trim.as_mut() {
            edit.range.set_start(0.25);
            edit.range.set_end(0.5);
        }
        app.activate(&ui, Target::Done);
        assert!(app.trim.is_none());
        assert_eq!(app.memos.len(), 2, "the original stays");
        let kept = &app.memos[app.selected.unwrap()];
        assert_eq!(kept.duration_ms, 250);
        assert!(kept.title.ends_with(" trimmed"), "{}", kept.title);
        let _ = std::fs::remove_dir_all(&app.dir);
    }

    #[test]
    fn enhance_saves_a_gated_copy_and_selects_it() {
        let (mut app, ui) = app("enhance");
        let samples: Vec<i16> = (0..8000).map(|i| if i < 4000 { 40 } else { 12000 }).collect();
        store::save(&app.dir, "20261004-120000", &springboard::media::capture::wav_bytes(&springboard::media::capture::pcm16(8000, 1), &samples), 100).unwrap();
        app.reload();
        app.selected = Some(0);
        assert!(app.menu(&ui, MENU_ENHANCE));
        assert_eq!(app.memos.len(), 2, "the original and the enhanced copy");
        let copy = &app.memos[app.selected.unwrap()];
        assert!(copy.title.ends_with(" enhanced"), "{}", copy.title);
        assert_eq!(copy.duration_ms, 1000, "the length is kept");
        assert!(app.error.is_empty());
        assert!(app.notice.starts_with("Enhanced copy saved as"), "{}", app.notice);
        let _ = std::fs::remove_dir_all(&app.dir);
    }

    #[test]
    fn split_at_the_playhead_writes_two_parts_and_selects_the_first() {
        let (mut app, ui) = app("split");
        let samples: Vec<i16> = (0..8000).map(|i| i as i16).collect();
        store::save(&app.dir, "20261004-120000", &springboard::media::capture::wav_bytes(&springboard::media::capture::pcm16(8000, 1), &samples), 100).unwrap();
        app.reload();
        app.selected = Some(0);
        assert!(!app.can_split(), "a split needs the selected take to be playing");
        app.playing = Some(app.memos[0].path.clone());
        app.position_ms = 250.0;
        assert!(app.can_split());
        assert!(app.menu(&ui, MENU_SPLIT));
        assert!(app.playing.is_none(), "a split stops playback");
        let mut durations: Vec<u64> = app.memos.iter().map(|m| m.duration_ms).collect();
        durations.sort();
        assert_eq!(durations, vec![250, 750, 1000], "two parts and the original");
        let first = &app.memos[app.selected.unwrap()];
        assert!(first.title.ends_with(" part 1"), "{}", first.title);
        let _ = std::fs::remove_dir_all(&app.dir);
    }

    #[test]
    fn skip_silence_is_a_menu_check_and_jumps_over_the_quiet_part() {
        let (mut app, ui) = app("skip");
        assert!(!app.skip_silence_on());
        assert!(app.menu(&ui, MENU_SKIP));
        assert!(app.skip_silence_on(), "the first pick turns it on");
        assert!(app.menu(&ui, MENU_SKIP));
        assert!(!app.skip_silence_on(), "the second pick turns it off");

        app.skip_silence = true;
        app.spans = vec![(500.0, 1000.0)];
        app.skip_silence_from(0.0);
        assert_eq!(app.position_ms, 500.0, "silence before the speech is skipped");
        assert_eq!(app.skip_guard, 500.0);
        app.skip_silence_from(100.0);
        assert_eq!(app.position_ms, 500.0, "a jump still in flight is waited for");
        app.skip_silence_from(600.0);
        assert_eq!(app.position_ms, 500.0, "inside speech nothing moves");
        app.skip_silence = false;
        app.skip_silence_from(1200.0);
        assert_eq!(app.position_ms, 500.0, "off means off");
    }

    #[test]
    fn notes_save_beside_the_recording_and_the_menu_opens_them() {
        let (mut app, ui) = app("notes");
        store::save(&app.dir, "20261012-100000", &springboard::media::capture::wav_bytes(&springboard::media::capture::pcm16(8000, 1), &vec![0; 800]), 100).unwrap();
        app.reload();
        app.selected = Some(0);
        assert!(app.has_selection(), "a selected recording enables the menu");
        assert!(app.menu(&ui, MENU_NOTES));
        assert!(app.notes.is_some(), "Notes opens the notes card");
        app.key(&ui, Out::Text("Milk".into()), 0.0);
        app.key(&ui, Out::Enter, 0.0);
        app.key(&ui, Out::Text("Eggs".into()), 0.0);
        app.key(&ui, Out::Backspace, 0.0);
        app.key(&ui, Out::Text("s".into()), 0.0);
        app.activate(&ui, Target::Done);
        assert!(app.notes.is_none());
        assert_eq!(store::read_notes(&app.memos[0].path), "Milk\nEggs");
        assert_eq!(app.memos.len(), 1, "the notes file is not listed as a recording");
        app.menu(&ui, MENU_NOTES);
        assert_eq!(app.notes.as_ref().unwrap().view.text(), "Milk\nEggs");
        app.activate(&ui, Target::Cancel);
        assert!(app.notes.is_none());
        let _ = std::fs::remove_dir_all(&app.dir);
    }

    fn parse(text: &str) -> Json {
        bifrost_core::json::parse(text).unwrap()
    }

    fn one_recording(app: &mut VoiceMemos, samples: usize) {
        let wav = springboard::media::capture::wav_bytes(&springboard::media::capture::pcm16(8000, 1), &vec![0; samples]);
        store::save(&app.dir, "20261004-120000", &wav, 100).unwrap();
        app.reload();
        app.selected = Some(0);
    }

    #[test]
    fn a_new_app_asks_for_the_microphone_and_the_devices_and_opens_no_device() {
        let (mut app, _) = app("ask");
        assert!(matches!(app.take_wants().as_slice(), [Want::MicStatus, Want::ListDevices]));
        assert!(app.next_wake(0.0).is_none());
    }

    #[test]
    fn a_denied_microphone_refuses_to_record_and_the_app_asks_again() {
        let (mut app, _) = app("denied");
        app.take_wants();
        app.reply(Ask::Mic, &Ok(parse(r#"{"media":"microphone","status":"denied"}"#)));
        app.start_recording();
        assert!(!app.recording, "no recording without the permission");
        assert!(app.error.contains("Microphone access is off"), "{}", app.error);
        assert!(matches!(app.take_wants().as_slice(), [Want::MicStatus]));
    }

    #[test]
    fn a_missing_privacy_service_leaves_the_microphone_unknown_and_recording_allowed() {
        let (mut app, _) = app("noservice");
        app.reply(Ask::Mic, &Err("no service privacy".into()));
        assert!(app.mic.is_none());
        assert!(services::record_gate(app.mic).is_ok());
    }

    #[test]
    fn the_input_device_is_named_and_a_change_during_a_recording_is_noted() {
        let (mut app, _) = app("input");
        let listing = |input: &str| {
            parse(&format!(
                r#"{{"devices":[{{"id":"a","name":"Desk mic","input":true,"output":false,"sound_effects":false}},{{"id":"b","name":"USB mic","input":true,"output":false,"sound_effects":false}}],"defaults":{{"input":"{input}"}}}}"#
            ))
        };
        app.reply(Ask::Devices, &Ok(listing("a")));
        assert_eq!(app.input_name.as_deref(), Some("Desk mic"));
        app.recording = true;
        app.record_input = Some("a".into());
        app.reply(Ask::Devices, &Ok(listing("b")));
        assert_eq!(app.notice, "Input changed to USB mic");
        assert_eq!(app.input_name.as_deref(), Some("USB mic"));
        assert!(services::caption(app.mic, app.input_name.as_deref(), &app.notice).starts_with("Input changed"));
    }

    #[test]
    fn a_route_or_device_change_asks_for_the_device_list_again() {
        let (mut app, _) = app("route");
        app.take_wants();
        app.devices_changed();
        assert!(matches!(app.take_wants().as_slice(), [Want::ListDevices]));
    }

    #[test]
    fn a_playing_recording_is_published_and_withdrawn_when_playback_ends() {
        let (mut app, _) = app("now");
        one_recording(&mut app, 8000);
        app.take_wants();
        // the state play() sets, without a speaker
        let memo = app.memos[0].clone();
        app.playing = Some(memo.path.clone());
        if let Some(session) = app.session.as_mut() {
            session.set_content(content_for(&memo));
        }
        app.publish_playback();
        let body = app
            .take_wants()
            .into_iter()
            .find_map(|w| match w {
                Want::Publish(body) => Some(body),
                _ => None,
            })
            .expect("a publish");
        assert_eq!(body.str_field("state").as_deref(), Some("playing"));
        assert_eq!(body.str_field("title").as_deref(), Some(memo.title.as_str()));
        app.playing = None;
        app.end_playback();
        assert!(app.take_wants().iter().any(|w| matches!(w, Want::Withdraw)));
        let _ = std::fs::remove_dir_all(&app.dir);
    }

    #[test]
    fn a_shell_seek_or_skip_moves_the_playhead_within_the_recording() {
        let (mut app, _) = app("seek");
        one_recording(&mut app, 4000); // 500 ms
        app.command(now_playing::Command::Seek(900.0));
        assert_eq!(app.position_ms, 500.0, "clamped to the length");
        app.command(now_playing::Command::SkipBackward);
        assert_eq!(app.position_ms, 0.0, "clamped at the start");
        app.command(now_playing::Command::Next);
        assert_eq!(app.position_ms, 0.0, "next is not declared, so it does nothing");
        let _ = std::fs::remove_dir_all(&app.dir);
    }

    #[test]
    fn the_input_meter_reads_silence_as_the_floor_and_full_scale_at_the_top() {
        assert_eq!(meter_fraction(0.0), 0.0);
        assert_eq!(meter_title(0.0), "Silent");
        assert!((meter_fraction(1.0) - 1.0).abs() < 1e-6);
        assert_eq!(meter_title(1.0), "0 dB");
        // power 0.25 is amplitude 0.5, about -6.02 dB: (60 - 6.02) / 60 = 0.8997
        assert!((meter_fraction(0.25) - 0.8997).abs() < 0.001, "{}", meter_fraction(0.25));
        assert_eq!(meter_title(0.25), "-6 dB");
        // power 1e-7 is amplitude 3.2e-4, about -70 dB: below the floor, so the bar is empty and it reads Silent
        assert_eq!(meter_fraction(1e-7), 0.0);
        assert_eq!(meter_title(1e-7), "Silent");
    }

    #[test]
    fn dragging_the_volume_slider_sets_the_level_and_clamps_at_the_ends() {
        let (mut app, ui) = app("volume");
        one_recording(&mut app, 8000);
        let r = app.volume_rect(&ui);
        let y = r.1 + r.3 / 2.0;
        // a press a quarter of the way along the track sets the level to 0.25
        app.touch(&ui, Event::Down(r.0 + r.2 * 0.25, y), 0.0);
        app.touch(&ui, Event::Up(r.0 + r.2 * 0.25, y), 0.0);
        assert!((app.volume - 0.25).abs() < 1e-3, "{}", app.volume);
        // a drag past the right end clamps to full
        app.touch(&ui, Event::Down(r.0 + r.2 * 0.5, y), 0.0);
        app.touch(&ui, Event::Move(r.0 + r.2 * 2.0, y), 0.0);
        app.touch(&ui, Event::Up(r.0 + r.2 * 2.0, y), 0.0);
        assert_eq!(app.volume, 1.0);
        assert!(!app.volume_drag, "the drag ends on release");
        let _ = std::fs::remove_dir_all(&app.dir);
    }
}
