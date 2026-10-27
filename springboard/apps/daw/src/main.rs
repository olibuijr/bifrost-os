//! DAW for BifrOSt PC: song list, session grid (tracks by scenes, mute/solo), piano roll for the selected clip,
//! and transport through the shell's audio service.
use bifrost_app::controls::{Node, Rect, Role};
use bifrost_app::kit::{ui, Damage, Surface, Ui, Weight};
use bifrost_app::layout::traffic_lights_trailing;
use bifrost_app::controls::media_trim::{TrimResult, TrimSession};
use bifrost_app::controls::plugin_host::PresetStore;
use bifrost_app::now_playing::Command;
use bifrost_app::proto::{DocErrorCode, DocMode};
use bifrost_app::{App, Ctx, DocumentEvent, Input, TouchKind};
use bifrost_core::json::Json;
use daw::ai;
use daw::model::{Clip, Extra, Instrument, Project, Scene, Track};
use daw::playback::Player;
use daw::rack;
use daw::render::{self, RenderOpts};
use daw::store::{self, Summary};
use daw::system::{self, Transport};
use daw::{export, midi_file, wav};
use std::io::Read;
use std::path::{Path, PathBuf};

const MARGIN: f32 = 20.0;

/// A toggle button's fill: the user's accent while it is on, the field colour while off.
fn toggle_fill(on: bool) -> u32 {
    if on { ui::accent_now() } else { ui::FIELD_BG.now() }
}
const HEADER: f32 = 96.0;
const ROW_H: f32 = 56.0;
const TRACK_W: f32 = 180.0;
const CELL_H: f32 = 44.0;
const SCENE_W: f32 = 120.0;
const ROLL_H: f32 = 260.0;
const PICK_ROWS: usize = 5;
const PICK_ROW_H: f32 = 40.0;
/// Rack inspector: the parameter rows start under the header, one slider row per parameter.
const RACK_TOP: f32 = HEADER + 16.0;
const RACK_ROW_H: f32 = 28.0;
/// Width of the parameter names before the slider, and of the preset column on the right.
const RACK_LABEL_W: f32 = 240.0;
const RACK_PANEL_W: f32 = 240.0;
/// Presets listed in the rack's preset column.
const RACK_PRESET_ROWS: usize = 8;

/// The preset column of the rack: its x and width in px.
fn rack_panel(w: f32) -> (f32, f32) {
    (w - RACK_PANEL_W, RACK_PANEL_W - MARGIN)
}

/// The slider of a rack row: its x and width in px. The value readout sits right of it.
fn rack_slider(w: f32) -> (f32, f32) {
    let sx = MARGIN + 8.0 + RACK_LABEL_W;
    let end = w - RACK_PANEL_W - 90.0;
    (sx, (end - sx).max(40.0))
}

#[derive(Clone, Copy, PartialEq)]
enum View {
    Songs,
    Session,
    Arrangement,
    Rack,
}

struct Daw {
    store_dir: PathBuf,
    songs: Vec<Summary>,
    view: View,
    project: Option<Project>,
    selected_clip: Option<(usize, usize)>,
    status: String,
    player: Option<Player>,
    playing: bool,
    now: f64,
    /// automation mode: roll clicks add points to `param` instead of notes
    auto_mode: bool,
    /// selected automation parameter key: "inst.<name>" or "<effect id>.<name>"
    param: Option<String>,
    /// track shown in the rack view
    rack_track: usize,
    /// instrument picker page while the picker is open
    picker: Option<usize>,
    /// instruments the desktop engine can build: (type, name, description), sorted by name
    instruments: Vec<(String, String, String)>,
    /// x of the header controls in px: right of the traffic lights in the full-size window band
    lead: f32,
    /// the shell services the app uses: output route, Now Playing session, Low Power Mode (system.rs)
    system: system::System,
    /// playback was paused from the Now Playing tile (the service keeps the position)
    paused: bool,
    /// the region being trimmed for export, while Trim is on (in and out points, one bar at least)
    trim: Option<TrimSession>,
    /// an Import MIDI tap asked for the open panel; `event` opens it, since `touch` has no context
    pending_import: bool,
    /// the user presets of the rack track's instrument kind, loaded when the rack opens or the track changes
    rack_presets: Option<PresetStore>,
}

#[derive(Clone, Copy)]
enum Hit {
    Song(usize),
    NewSong,
    OpenPicker,
    ClosePicker,
    PickPage(i32),
    PickInst(usize),
    AddScene,
    Back,
    Play,
    Stop,
    Mute(usize),
    Solo(usize),
    Clip(usize, usize),
    Arrange,
    Rack,
    Auto,
    ArrBars(usize, i32),
    ArrRemove(usize),
    ArrAdd(usize),
    ArrMute(usize),
    RackParam(usize),
    RackTrack(i32),
    RackSlider(usize, f32),
    RackSavePreset,
    RackLoadPreset(usize),
    ImportMidi,
    Trim,
    TrimStart,
    TrimEnd,
    Export,
    ExportCd,
    Nothing,
}

/// One bar in seconds at a song tempo (4 beats; the song's meter is not used for trimming).
fn bar_seconds(bpm: f64) -> f64 {
    240.0 / bpm.clamp(40.0, 240.0)
}

/// The song's length in seconds, from the same schedule the renderer uses, before the reverb tail.
fn song_seconds(p: &Project) -> f64 {
    let segs = if p.arrangement.is_empty() {
        render::scene_segments(p, RenderOpts::default().loops, None)
    } else {
        render::arrangement_segments(p)
    };
    render::step_times(p, &segs).1
}

fn rect_contains(r: (f32, f32, f32, f32), x: f32, y: f32) -> bool {
    x >= r.0 && x < r.0 + r.2 && y >= r.1 && y < r.1 + r.3
}

impl Daw {
    fn new(store_dir: PathBuf) -> Self {
        let songs = store::list(&store_dir);
        let plugins = std::env::var("DAW_PLUGINS").map(PathBuf::from).unwrap_or_else(|_| daw::assets::asset_root().join("plugins"));
        let root = PathBuf::from(std::env::var("BIFROST_SERVICE_ROOT").unwrap_or_else(|_| "/var/lib/springboard".into()));
        let instruments = load_instruments(&plugins);
        let player = std::env::var("BIFROST_SOCK").ok().map(|_| Player::new(&root, store_dir.join("render-cache"), plugins));
        Daw { store_dir, songs, view: View::Songs, project: None, selected_clip: None, status: String::new(), player, playing: false, now: 0.0, auto_mode: false, param: None, rack_track: 0, picker: None, instruments, lead: 0.0, system: Default::default(), paused: false, trim: None, pending_import: false, rack_presets: None }
    }

    /// Writes the project to `<store>/projects/<id>.json`.
    fn save_project(&self, p: &Project) -> Result<(), String> {
        let dir = self.store_dir.join("projects");
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let text = serde_json::to_string_pretty(p).map_err(|e| e.to_string())?;
        store::write_atomic(&dir.join(format!("{}.json", p.id)), &text)
    }

    /// A new empty song with one scene, saved and opened in the Session view.
    fn new_song(&mut self) {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
        let id = format!("song{now:x}");
        let project = Project {
            id: id.clone(),
            name: format!("Song {}", self.songs.len() + 1),
            bpm: 120.0,
            swing: 0.0,
            key: None,
            scale: None,
            master: None,
            scenes: vec![Scene { id: "scene1".into(), name: "Scene 1".into(), bpm: None, sig: None, color: None, extra: Default::default() }],
            tracks: vec![],
            arrangement: vec![],
            groups: None,
            extra: Default::default(),
        };
        match self.save_project(&project) {
            Ok(()) => {
                self.songs = store::list(&self.store_dir);
                if let Some(i) = self.songs.iter().position(|s| s.id == id) {
                    self.open(i);
                }
            }
            Err(e) => self.status = format!("Could not create the song: {e}"),
        }
    }

    /// Adds a track on the given instrument, named after it, with an empty slot per scene, and saves the song.
    fn add_track(&mut self, kind: &str, name: &str) {
        let Some(p) = self.project.as_mut() else { return };
        let n = p.tracks.len() + 1;
        p.tracks.push(Track {
            id: ai::new_id("t", n),
            name: name.to_string(),
            color: String::new(),
            group: None,
            instrument: Instrument { kind: kind.into(), params: ai::default_params(kind, None), sample: None, pads: None, extra: Default::default() },
            effects: vec![],
            volume: 0.8,
            pan: 0.0,
            mute: false,
            solo: false,
            frozen: None,
            clips: Default::default(),
            extra: Default::default(),
        });
        self.status = format!("Added {name}");
        self.save();
    }

    /// Adds an empty scene (up to 8, as the mobile app).
    fn add_scene(&mut self) {
        let Some(p) = self.project.as_mut() else { return };
        if p.scenes.len() >= 8 {
            return;
        }
        let n = p.scenes.len() + 1;
        p.scenes.push(Scene { id: ai::new_id("s", n), name: n.to_string(), bpm: None, sig: None, color: None, extra: Default::default() });
        self.save();
    }

    /// Tapping an empty slot makes a one-bar clip named after its track, as the mobile app does.
    fn new_clip(&mut self, r: usize, c: usize) {
        let Some(p) = self.project.as_mut() else { return };
        let scene = p.scenes[c].id.clone();
        let t = &mut p.tracks[r];
        let clip = daw::model::Clip { id: ai::new_id("c", r * 100 + c), name: t.name.clone(), length: 16, notes: vec![], automation: None, cc: None, extra: Default::default() };
        t.clips.insert(scene, clip);
        self.save();
    }

    fn open(&mut self, i: usize) {
        let Some(s) = self.songs.get(i) else { return };
        match store::load(&s.path) {
            Ok(p) => {
                self.status = format!("{} tracks, {} BPM", p.tracks.len(), p.bpm);
                self.project = Some(p);
                self.view = View::Session;
                self.selected_clip = None;
            }
            Err(e) => self.status = e,
        }
    }

    fn layout_session(&self, w: f32, h: f32) -> (f32, f32, f32, f32) {
        let roll_top = h - ROLL_H;
        (MARGIN, HEADER, w - 2.0 * MARGIN, roll_top - HEADER - 8.0)
    }

    fn hit(&self, w: f32, h: f32, x: f32, y: f32) -> Hit {
        match self.view {
            View::Songs => {
                if rect_contains((w - 150.0, 26.0, 130.0, 40.0), x, y) {
                    return Hit::NewSong;
                }
                if rect_contains((self.lead, 0.0, 120.0, HEADER), x, y) {
                    return Hit::Nothing;
                }
                if y < HEADER {
                    return Hit::Nothing;
                }
                let i = ((y - HEADER) / ROW_H) as usize;
                if i < self.songs.len() {
                    Hit::Song(i)
                } else {
                    Hit::Nothing
                }
            }
            View::Arrangement => {
                if rect_contains((self.lead, 8.0, 80.0, 40.0), x, y) {
                    return Hit::Back;
                }
                if rect_contains((w - 220.0, 16.0, 100.0, 32.0), x, y) {
                    return Hit::Play;
                }
                let Some(p) = &self.project else { return Hit::Nothing };
                let top = HEADER + 16.0;
                if y < top {
                    return Hit::Nothing;
                }
                let row = ((y - top) / ROW_H) as usize;
                if row < p.arrangement.len() {
                    if x > w - 260.0 && x < w - 220.0 {
                        return Hit::ArrBars(row, -1);
                    }
                    if x > w - 200.0 && x < w - 160.0 {
                        return Hit::ArrBars(row, 1);
                    }
                    if x > w - 140.0 && x < w - 100.0 {
                        return Hit::ArrRemove(row);
                    }
                    if x > w - 90.0 {
                        return Hit::ArrMute(row);
                    }
                    return Hit::Nothing;
                }
                let add_row = row - p.arrangement.len();
                if add_row < p.scenes.len() && x < w - MARGIN {
                    return Hit::ArrAdd(add_row);
                }
                Hit::Nothing
            }
            View::Rack => {
                if rect_contains((self.lead, 8.0, 80.0, 40.0), x, y) {
                    return Hit::Back;
                }
                if rect_contains((w - 220.0, 16.0, 100.0, 32.0), x, y) {
                    return Hit::RackTrack(-1);
                }
                if rect_contains((w - 110.0, 16.0, 100.0, 32.0), x, y) {
                    return Hit::RackTrack(1);
                }
                let Some(p) = &self.project else { return Hit::Nothing };
                let (px, pw) = rack_panel(w);
                if rect_contains((px, RACK_TOP, pw, 32.0), x, y) {
                    return Hit::RackSavePreset;
                }
                let presets = self.rack_presets.as_ref().map_or(0, |s| s.presets().len()).min(RACK_PRESET_ROWS);
                for k in 0..presets {
                    if rect_contains((px, RACK_TOP + 44.0 + k as f32 * 32.0, pw, 28.0), x, y) {
                        return Hit::RackLoadPreset(k);
                    }
                }
                if y < RACK_TOP || x < MARGIN {
                    return Hit::Nothing;
                }
                let row = ((y - RACK_TOP) / RACK_ROW_H).floor() as usize;
                if row >= self.rack_rows(p).len() {
                    return Hit::Nothing;
                }
                let (sx, sw) = rack_slider(w);
                if sw > 0.0 && x >= sx - 6.0 && x <= sx + sw + 6.0 {
                    return Hit::RackSlider(row, ((x - sx) / sw).clamp(0.0, 1.0));
                }
                if x < sx {
                    return Hit::RackParam(row);
                }
                Hit::Nothing
            }
            View::Session => {
                if self.picker.is_some() {
                    if let Some(hit) = self.picker_hit(w, h, x, y) {
                        return hit;
                    }
                }
                if rect_contains((w - 220.0, 16.0, 100.0, 32.0), x, y) {
                    return Hit::Play;
                }
                if rect_contains((w - 330.0, 16.0, 100.0, 32.0), x, y) {
                    return Hit::Arrange;
                }
                if rect_contains((w - 440.0, 16.0, 100.0, 32.0), x, y) {
                    return Hit::Rack;
                }
                if rect_contains((w - 550.0, 16.0, 100.0, 32.0), x, y) {
                    return Hit::Auto;
                }
                if rect_contains((w - 110.0, 16.0, 100.0, 32.0), x, y) {
                    return Hit::Stop;
                }
                if rect_contains((w - 660.0, 16.0, 100.0, 32.0), x, y) {
                    return Hit::Trim;
                }
                if rect_contains((w - 770.0, 16.0, 100.0, 32.0), x, y) {
                    return Hit::Export;
                }
                if self.trim.is_none() && rect_contains((w - 880.0, 16.0, 100.0, 32.0), x, y) {
                    return Hit::ExportCd;
                }
                if self.trim.is_some() {
                    if rect_contains((w - 880.0, 16.0, 100.0, 32.0), x, y) {
                        return Hit::TrimStart;
                    }
                    if rect_contains((w - 990.0, 16.0, 100.0, 32.0), x, y) {
                        return Hit::TrimEnd;
                    }
                }
                if rect_contains((self.lead, 8.0, 80.0, 40.0), x, y) {
                    return Hit::Back;
                }
                let Some(p) = &self.project else { return Hit::Nothing };
                let (gx, gy, _gw, _gh) = self.layout_session(w, h);
                let ty = gy + 32.0;
                if y < ty && x >= gx && x < gx + TRACK_W {
                    return Hit::Stop;
                }
                if y < ty || x < gx {
                    return Hit::Nothing;
                }
                let row = ((y - ty) / CELL_H) as usize;
                if row == p.tracks.len() {
                    if x >= gx && x < gx + TRACK_W {
                        return Hit::OpenPicker;
                    }
                    if x >= gx + TRACK_W && x < gx + TRACK_W + 120.0 && p.scenes.len() < 8 {
                        return Hit::AddScene;
                    }
                    if x >= gx + TRACK_W + 120.0 && x < gx + TRACK_W + 260.0 {
                        return Hit::ImportMidi;
                    }
                    return Hit::Nothing;
                }
                if row > p.tracks.len() {
                    return Hit::Nothing;
                }
                if x < gx + TRACK_W {
                    if x >= gx + TRACK_W - 36.0 {
                        return Hit::Solo(row);
                    }
                    if x >= gx + TRACK_W - 72.0 {
                        return Hit::Mute(row);
                    }
                    return Hit::Nothing;
                }
                let col = ((x - gx - TRACK_W) / SCENE_W) as usize;
                if col < p.scenes.len() {
                    Hit::Clip(row, col)
                } else {
                    Hit::Nothing
                }
            }
        }
    }

    /// Rack rows: (automation key, label, spec min, spec max) for the instrument and every effect of the rack track.
    fn rack_rows(&self, p: &Project) -> Vec<(String, String, f64, f64)> {
        let Some(track) = p.tracks.get(self.rack_track) else { return vec![] };
        rack::rows(track).into_iter().map(|r| (r.key, r.label, r.item.param().min, r.item.param().max)).collect()
    }

    /// The kind of the rack track's instrument, which names its preset file.
    fn rack_kind(&self) -> Option<String> {
        self.project.as_ref()?.tracks.get(self.rack_track).map(|t| t.instrument.kind.clone())
    }

    /// Loads the presets of the rack track's instrument kind from its file. A file that cannot be read is reported.
    fn refresh_rack_presets(&mut self) {
        let Some(kind) = self.rack_kind() else {
            self.rack_presets = None;
            return;
        };
        let path = rack::preset_path(&self.store_dir, &kind);
        self.rack_presets = match rack::load_presets(&path) {
            Ok(store) => Some(store),
            Err(e) => {
                self.status = format!("Presets could not be read: {e}");
                None
            }
        };
    }

    /// Moves the rack to the next or previous track, wrapping at the ends.
    fn step_rack_track(&mut self, d: i32) {
        let Some(n) = self.project.as_ref().map(|p| p.tracks.len()).filter(|n| *n > 0) else { return };
        self.rack_track = (self.rack_track as i32 + d).rem_euclid(n as i32) as usize;
        self.refresh_rack_presets();
    }

    /// A click on a slider: sets the parameter to the position across the slider, and saves the song.
    fn rack_slide(&mut self, row: usize, position: f32) {
        let Some(p) = self.project.as_mut() else { return };
        let Some(track) = p.tracks.get_mut(self.rack_track) else { return };
        let Some(item) = rack::rows(track).get(row).map(|r| (r.item, r.label.clone())) else { return };
        let Some(extra) = rack::owner_extra_mut(track, item.0.owner) else { return };
        let clamped = rack::set(&item.0, extra, rack::value_at(&item.0, position));
        self.save();
        self.status = format!("{}: {:.2}", item.1, clamped);
    }

    /// Saves the rack track's instrument values as a new preset, and writes the preset file.
    fn rack_save_preset(&mut self) {
        if self.rack_presets.is_none() {
            self.refresh_rack_presets();
        }
        let Some(kind) = self.rack_kind() else { return };
        let path = rack::preset_path(&self.store_dir, &kind);
        let Some(extra) = self.project.as_ref().and_then(|p| p.tracks.get(self.rack_track)).map(|t| t.instrument.params.clone()) else { return };
        let Some(store) = self.rack_presets.as_mut() else { return };
        let result = rack::add_preset(store, &kind, &extra).and_then(|name| rack::save_presets(&path, store).map(|()| name));
        self.status = match result {
            Ok(name) => format!("Saved {name}"),
            Err(e) => e,
        };
    }

    /// Sets the rack track's instrument values to a saved preset, and saves the song.
    fn rack_load_preset(&mut self, k: usize) {
        let Some(kind) = self.rack_kind() else { return };
        let Some(store) = self.rack_presets.as_mut() else { return };
        let Some(number) = store.presets().get(k).map(|p| p.number) else { return };
        let Some(track) = self.project.as_mut().and_then(|p| p.tracks.get_mut(self.rack_track)) else { return };
        let result = rack::load_preset(store, &kind, &mut track.instrument.params, number);
        self.save();
        self.status = match result {
            Ok(name) => format!("Loaded {name}"),
            Err(e) => e,
        };
    }

    /// Automation-mode click in the roll: adds a point on the selected parameter lane, value from the pointer height.
    fn auto_touch(&mut self, w: f32, h: f32, x: f32, y: f32) -> bool {
        let Some((r, c)) = self.selected_clip else { return false };
        let Some(key) = self.param.clone() else { return false };
        let (x0, y0, rw, rh) = self.roll_rect(w, h);
        if !rect_contains((x0, y0, rw, rh), x, y) {
            return false;
        }
        let (lo, hi) = self.param_range(&key).unwrap_or((0.0, 1.0));
        let Some(p) = self.project.as_mut() else { return false };
        let scene = p.scenes[c].id.clone();
        let Some(clip) = p.tracks[r].clips.get_mut(&scene) else { return false };
        let len = clip.length.max(1) as f32;
        let t = (((x - x0) / rw) * len).clamp(0.0, len) as f64;
        let v = (lo + (1.0 - ((y - y0) / rh).clamp(0.0, 1.0)) as f64 * (hi - lo)).clamp(lo, hi);
        let lane = clip.automation.get_or_insert_with(Default::default).entry(key).or_default();
        lane.retain(|pt| (pt.t - t).abs() > 0.5);
        lane.push(daw::model::Point { t, v });
        lane.sort_by(|a, b| a.t.partial_cmp(&b.t).unwrap_or(std::cmp::Ordering::Equal));
        self.save();
        true
    }

    fn param_range(&self, key: &str) -> Option<(f64, f64)> {
        let p = self.project.as_ref()?;
        self.rack_rows(p).into_iter().find(|r| r.0 == key).map(|r| (r.2, r.3))
    }

    /// Piano-roll grid rectangle for the selected clip (x, y, w, h).
    fn roll_rect(&self, w: f32, h: f32) -> (f32, f32, f32, f32) {
        (MARGIN, h - ROLL_H + 32.0, w - 2.0 * MARGIN, ROLL_H - 44.0)
    }

    /// Click in the piano roll: removes the note under the pointer, or adds a note at that step and pitch.
    fn roll_touch(&mut self, w: f32, h: f32, x: f32, y: f32) -> bool {
        let Some((r, c)) = self.selected_clip else { return false };
        let (x0, y0, rw, rh) = self.roll_rect(w, h);
        if !rect_contains((x0, y0, rw, rh), x, y) {
            return false;
        }
        let Some(p) = self.project.as_mut() else { return false };
        let scene = p.scenes[c].id.clone();
        let Some(clip) = p.tracks[r].clips.get_mut(&scene) else { return false };
        let len = clip.length.max(1) as f32;
        let (lo, hi) = clip.notes.iter().fold((127.0f64, 0.0f64), |(lo, hi), n| (lo.min(n.p), hi.max(n.p)));
        let (lo, hi) = if clip.notes.is_empty() { (48.0, 72.0) } else { (lo, hi.max(lo + 1.0)) };
        let step_at = |px: f32| ((px - x0) / rw * len).floor().max(0.0);
        let pitch_at = |py: f32| (lo + ((y0 + rh - 6.0 - py) / (rh - 8.0)) as f64 * (hi - lo)).round().clamp(0.0, 127.0);
        let hit_index = clip.notes.iter().position(|n| {
            let nx = x0 + (n.t as f32 / len) * rw;
            let nw = ((n.d as f32 / len) * rw).max(3.0);
            let ny = y0 + rh - ((n.p - lo) as f32 / (hi - lo) as f32 * (rh - 8.0)) - 6.0;
            rect_contains((nx, ny - 2.0, nw, 9.0), x, y)
        });
        match hit_index {
            Some(i) => {
                clip.notes.remove(i);
            }
            None => clip.notes.push(daw::model::Note {
                t: step_at(x) as f64,
                d: 1.0,
                p: pitch_at(y),
                v: 0.8,
                prob: None,
                extra: Default::default(),
            }),
        }
        clip.notes.sort_by(|a, b| a.t.partial_cmp(&b.t).unwrap_or(std::cmp::Ordering::Equal));
        self.save();
        true
    }

    /// Writes the open song back to the store, atomically.
    fn save(&mut self) {
        let Some(p) = self.project.as_ref() else { return };
        let Some(summary) = self.songs.iter().find(|s| s.id == p.id) else { return };
        match daw::store::write_atomic(&summary.path, &p.to_json()) {
            Ok(()) => self.status = "saved".into(),
            Err(e) => self.status = e,
        }
    }

    fn touch(&mut self, w: f32, h: f32, x: f32, y: f32) -> bool {
        let picking = self.view == View::Session && self.picker.is_some();
        if !picking && self.view == View::Session && self.auto_mode && self.auto_touch(w, h, x, y) {
            return true;
        }
        if !picking && self.view == View::Session && self.roll_touch(w, h, x, y) {
            return true;
        }
        let hit = self.hit(w, h, x, y);
        match hit {
            Hit::Song(i) => self.open(i),
            Hit::Back => {
                self.view = View::Songs;
                self.project = None;
            }
            Hit::NewSong => self.new_song(),
            Hit::OpenPicker => self.picker = Some(0),
            Hit::ClosePicker => self.picker = None,
            Hit::PickPage(d) => {
                let pages = self.instruments.len().div_ceil(PICK_ROWS).max(1) as i32;
                if let Some(page) = self.picker.as_mut() {
                    *page = (*page as i32 + d).clamp(0, pages - 1) as usize;
                }
            }
            Hit::PickInst(i) => {
                if let Some((kind, name, _)) = self.instruments.get(i).cloned() {
                    self.add_track(&kind, &name);
                }
                self.picker = None;
            }
            Hit::AddScene => self.add_scene(),
            Hit::Play => self.play(),
            Hit::Stop => self.stop(),
            Hit::Mute(i) => {
                if let Some(p) = self.project.as_mut() {
                    p.tracks[i].mute = !p.tracks[i].mute;
                }
            }
            Hit::Solo(i) => {
                if let Some(p) = self.project.as_mut() {
                    p.tracks[i].solo = !p.tracks[i].solo;
                }
            }
            Hit::Clip(r, c) => {
                let empty = self.project.as_ref().is_some_and(|p| !p.tracks[r].clips.contains_key(&p.scenes[c].id));
                if empty {
                    self.new_clip(r, c);
                }
                self.selected_clip = Some((r, c));
            }
            Hit::ImportMidi => self.pending_import = true,
            Hit::Trim => self.toggle_trim(),
            Hit::TrimStart => self.nudge_trim(true),
            Hit::TrimEnd => self.nudge_trim(false),
            Hit::Export => self.export(),
            Hit::ExportCd => self.export_cd(),
            Hit::Arrange => self.view = View::Arrangement,
            Hit::Rack => {
                if let Some((r, _)) = self.selected_clip {
                    self.rack_track = r;
                }
                self.view = View::Rack;
                self.refresh_rack_presets();
            }
            Hit::RackTrack(d) => self.step_rack_track(d),
            Hit::RackSlider(row, position) => self.rack_slide(row, position),
            Hit::RackSavePreset => self.rack_save_preset(),
            Hit::RackLoadPreset(k) => self.rack_load_preset(k),
            Hit::Auto => self.auto_mode = !self.auto_mode,
            Hit::ArrBars(i, d) => {
                if let Some(p) = self.project.as_mut() {
                    let b = p.arrangement[i].bars as i32 + d;
                    p.arrangement[i].bars = b.clamp(1, 512) as u32;
                    self.save();
                }
            }
            Hit::ArrRemove(i) => {
                if let Some(p) = self.project.as_mut() {
                    p.arrangement.remove(i);
                    self.save();
                }
            }
            Hit::ArrMute(i) => {
                if let Some(p) = self.project.as_mut() {
                    let m = &mut p.arrangement[i];
                    m.mutes = if m.mutes.is_some() { None } else { Some(vec![]) };
                    self.save();
                }
            }
            Hit::ArrAdd(i) => {
                if let Some(p) = self.project.as_mut() {
                    let id = p.scenes[i].id.clone();
                    p.arrangement.push(daw::model::Section { scene_id: id, bars: 4, bpm: None, sig: None, mutes: None, label: None, extra: Default::default() });
                    self.save();
                }
            }
            Hit::RackParam(i) => {
                if let Some(p) = self.project.as_ref() {
                    let key = self.rack_rows(p).get(i).map(|r| r.0.clone());
                    self.param = key;
                    self.auto_mode = true;
                    self.view = View::Session;
                }
            }
            Hit::Nothing => return false,
        }
        true
    }

    /// Imports a Standard MIDI File into a new track on the first instrument (the synth when there is one). The
    /// notes go into the clip of the selected clip's scene, or the first scene, and the clip grows to fit them.
    fn import_midi(&mut self, bytes: &[u8], name: &str) {
        let Some(p) = self.project.as_ref() else {
            self.status = "Open a song first".into();
            return;
        };
        if p.scenes.is_empty() {
            self.status = "Add a scene first".into();
            return;
        }
        let scene = self.selected_clip.map_or(0, |(_, c)| c).min(p.scenes.len() - 1);
        let scene_id = p.scenes[scene].id.clone();
        let bpm = p.bpm;
        let track_index = p.tracks.len();
        let kind = match self.instruments.iter().find(|i| i.0 == "synth").or(self.instruments.first()) {
            Some(i) => i.0.clone(),
            None => {
                self.status = "No instrument is available".into();
                return;
            }
        };
        let notes = match midi_file::import_notes(bytes, bpm) {
            Ok(notes) => notes,
            Err(e) => {
                self.status = format!("{name}: {e}");
                return;
            }
        };
        let count = notes.len();
        let track_name = name.to_string();
        self.add_track(&kind, &track_name);
        let Some(p) = self.project.as_mut() else { return };
        let Some(track) = p.tracks.get_mut(track_index) else { return };
        let mut clip = Clip {
            id: ai::new_id("c", track_index * 100 + scene),
            name: track_name.clone(),
            length: 16,
            notes: vec![],
            automation: None,
            cc: None,
            extra: Default::default(),
        };
        midi_file::fill_clip(&mut clip, notes);
        track.clips.insert(scene_id, clip);
        self.selected_clip = Some((track_index, scene));
        self.save();
        self.status = format!("Imported {count} notes from {track_name}");
    }

    /// Trim on: the whole song is selected, one bar at least. Trim again cancels.
    fn toggle_trim(&mut self) {
        if self.trim.take().is_some() {
            self.status = "Trim cancelled".into();
            return;
        }
        let Some(p) = self.project.as_ref() else { return };
        let mut session = TrimSession::new(song_seconds(p), bar_seconds(p.bpm));
        if session.begin_trimming() {
            self.trim = Some(session);
            self.status = "Trim: move the start and end a bar at a time, then Export".into();
        } else {
            self.status = "Nothing to trim yet: add some notes first".into();
        }
    }

    /// Moves the in-point later or the out-point earlier by one bar. The session keeps them in the song.
    fn nudge_trim(&mut self, start: bool) {
        let Some(p) = self.project.as_ref() else { return };
        let bar = bar_seconds(p.bpm);
        let Some(session) = self.trim.as_mut() else { return };
        let (s, e) = session.selection();
        if start {
            session.set_start(s + bar);
        } else {
            session.set_end(e - bar);
        }
    }

    /// Exports the trimmed region, or the whole song when not trimming, as a 32-bit float WAV in `exports/`. The
    /// render runs here, so a long song takes a moment.
    fn export(&mut self) {
        self.export_region(false);
    }

    /// Exports the same region as a 16-bit WAV at 44.1 kHz, the CD rate, through the PCM conversion model.
    fn export_cd(&mut self) {
        self.export_region(true);
    }

    fn export_region(&mut self, cd: bool) {
        let Some(p) = self.project.as_ref() else { return };
        let (start, end) = match self.trim.take() {
            Some(mut session) => session.finish(TrimResult::Ok).unwrap_or((0.0, song_seconds(p))),
            None => (0.0, song_seconds(p)),
        };
        let rendered = render::render(p, &RenderOpts::default());
        let (bytes, tag) = if cd {
            match export::converted_wav(&rendered, start, end, export::CD_RATE) {
                Ok(b) => (b, "-cd"),
                Err(e) => {
                    self.status = format!("Export failed: {e}");
                    return;
                }
            }
        } else {
            (export::region_wav(&rendered, start, end), "")
        };
        let dir = self.store_dir.join("exports");
        let file = dir.join(format!(
            "{}-{}-{}{tag}.wav",
            p.id,
            (start * 1000.0).round() as u64,
            (end * 1000.0).round() as u64
        ));
        let result = std::fs::create_dir_all(&dir).and_then(|()| wav::write_file(&file, &bytes));
        self.status = match result {
            Ok(()) => format!("Exported {}", file.display()),
            Err(e) => format!("Export failed: {e}"),
        };
    }

    fn play(&mut self) {
        let (Some(p), Some(player)) = (self.project.as_ref(), self.player.as_mut()) else {
            self.status = "audio service not available".into();
            return;
        };
        match player.play(p) {
            Ok(path) => {
                self.playing = true;
                self.paused = false;
                self.status = format!("playing {}", path.display());
            }
            Err(e) => self.status = e,
        }
    }

    fn stop(&mut self) {
        if let Some(player) = self.player.as_mut() {
            player.stop();
        }
        self.playing = false;
        self.paused = false;
    }

    /// Pauses the song in place; play resumes it from the same position.
    fn pause(&mut self) {
        if !self.playing {
            return;
        }
        if let Some(player) = self.player.as_mut() {
            player.pause();
        }
        self.playing = false;
        self.paused = true;
    }

    /// The transport the desktop should show.
    fn transport(&self) -> Transport {
        if self.playing {
            Transport::Playing
        } else if self.paused {
            Transport::Paused
        } else {
            Transport::Stopped
        }
    }

    /// A transport command from the Now Playing tile. Only play, pause and stop are declared to the desktop.
    fn command(&mut self, cmd: Command) {
        match cmd {
            Command::Play if !self.playing => self.play(),
            Command::Pause => self.pause(),
            Command::Stop => self.stop(),
            _ => {}
        }
    }

    fn draw_text(s: &mut Surface, ui: &Ui, text: &str, pt: f32, weight: Weight, color: u32, x: f32, y: f32) {
        let img = ui.text(text, pt, weight, color);
        s.blit(&img, x as i32, y as i32, 255, 255);
    }

    fn draw_songs(&self, s: &mut Surface, ui: &Ui) {
        Self::draw_text(s, ui, "DAW", ui.pt(26.0), Weight::Bold, ui::LABEL.now(), self.lead, 22.0);
        let w = s.w as f32;
        s.fill_rect((w - 150.0) as usize, 26, 130, 40, ui::accent_now());
        Self::draw_text(s, ui, "New Song", ui.pt(15.0), Weight::Semibold, ui::WHITE, w - 136.0, 35.0);
        if self.songs.is_empty() {
            Self::draw_text(s, ui, "No songs yet. Create one with New Song.", ui.pt(15.0), Weight::Regular, ui::LABEL_2.now(), MARGIN, HEADER + 4.0);
        }
        for (i, song) in self.songs.iter().enumerate() {
            let y = HEADER + i as f32 * ROW_H;
            s.fill_rect(MARGIN as usize, y as usize + 4, s.w.saturating_sub(2 * MARGIN as usize), ROW_H as usize - 8, ui::FIELD_BG.now());
            Self::draw_text(s, ui, &song.name, ui.pt(17.0), Weight::Semibold, ui::LABEL.now(), MARGIN + 16.0, y + 10.0);
            let meta = format!("{:.0} BPM  ·  {} tracks  ·  {} scenes", song.bpm, song.tracks, song.scenes);
            Self::draw_text(s, ui, &meta, ui.pt(13.0), Weight::Regular, ui::LABEL_2.now(), MARGIN + 16.0, y + 32.0);
        }
    }

    fn draw_session(&self, s: &mut Surface, ui: &Ui) {
        let (w, h) = (s.w as f32, s.h as f32);
        let Some(p) = &self.project else { return };
        // header
        s.fill_rect(self.lead as usize, 8, 80, 40, ui::FIELD_BG.now());
        Self::draw_text(s, ui, "< Songs", ui.pt(15.0), Weight::Semibold, ui::TINT, self.lead + 12.0, 18.0);
        Self::draw_text(s, ui, &p.name, ui.pt(ui::MAC_TITLE1_PT), Weight::Bold, ui::LABEL.now(), self.lead + 110.0, 16.0);
        let play_col = if self.playing { ui::LABEL_2.now() } else { ui::accent_now() };
        s.fill_rect((w - 220.0) as usize, 16, 100, 32, play_col);
        Self::draw_text(s, ui, "Play", ui.pt(15.0), Weight::Semibold, ui::WHITE, w - 190.0, 22.0);
        s.fill_rect((w - 110.0) as usize, 16, 100, 32, ui::FIELD_BG.now());
        Self::draw_text(s, ui, "Stop", ui.pt(15.0), Weight::Semibold, ui::LABEL.now(), w - 80.0, 22.0);
        for (x, label, on) in [(w - 330.0, "Arrange", false), (w - 440.0, "Rack", false), (w - 550.0, "Auto", self.auto_mode)] {
            s.fill_rect(x as usize, 16, 100, 32, toggle_fill(on));
            Self::draw_text(s, ui, label, ui.pt(14.0), Weight::Semibold, if on { ui::WHITE } else { ui::LABEL.now() }, x + 12.0, 22.0);
        }
        // Trim and Export; the in and out buttons only while trimming
        let trimming = self.trim.is_some();
        let trim_label = if trimming { "Cancel Trim" } else { "Trim" };
        let mut trim_buttons = vec![(w - 660.0, trim_label, trimming), (w - 770.0, "Export", false)];
        if trimming {
            trim_buttons.push((w - 880.0, "Start +1 Bar", false));
            trim_buttons.push((w - 990.0, "End -1 Bar", false));
        } else {
            trim_buttons.push((w - 880.0, "Export CD", false));
        }
        for (x, label, on) in trim_buttons {
            s.fill_rect(x as usize, 16, 100, 32, toggle_fill(on));
            Self::draw_text(s, ui, label, ui.pt(14.0), Weight::Semibold, if on { ui::WHITE } else { ui::LABEL.now() }, x + 10.0, 22.0);
        }
        let (gx, gy, _gw, _gh) = self.layout_session(s.w as f32, s.h as f32);
        // the scene row: Stop in the head column
        s.fill_rect(gx as usize, (gy + 4.0) as usize, (TRACK_W - 8.0) as usize, 24, ui::FIELD_BG.now());
        Self::draw_text(s, ui, "Stop", ui.pt(13.0), Weight::Semibold, ui::LABEL.now(), gx + 12.0, gy + 7.0);
        // the add row under the last track: + Track, and + Scene while under 8 scenes
        let add_y = gy + 32.0 + p.tracks.len() as f32 * CELL_H;
        Self::draw_text(s, ui, "+ Track", ui.pt(15.0), Weight::Semibold, ui::TINT, gx + 8.0, add_y + 12.0);
        Self::draw_text(s, ui, "Import MIDI", ui.pt(15.0), Weight::Semibold, ui::TINT, gx + TRACK_W + 132.0, add_y + 12.0);
        if p.scenes.len() < 8 {
            Self::draw_text(s, ui, "+ Scene", ui.pt(15.0), Weight::Semibold, ui::TINT, gx + TRACK_W + 8.0, add_y + 12.0);
        }
        // scene headers
        for (c, sc) in p.scenes.iter().enumerate() {
            let x = gx + TRACK_W + c as f32 * SCENE_W;
            Self::draw_text(s, ui, &sc.name, ui.pt(13.0), Weight::Semibold, ui::LABEL_2.now(), x + 6.0, gy + 6.0);
        }
        // tracks and clips
        for (r, t) in p.tracks.iter().enumerate() {
            let y = gy + 32.0 + r as f32 * CELL_H;
            let name_col = if t.mute { ui::LABEL_2.now() } else { ui::LABEL.now() };
            Self::draw_text(s, ui, &t.name, ui.pt(15.0), Weight::Semibold, name_col, gx + 8.0, y + 12.0);
            let mute_col = toggle_fill(t.mute);
            let solo_col = toggle_fill(t.solo);
            s.fill_rect((gx + TRACK_W - 72.0) as usize, (y + 8.0) as usize, 32, 28, mute_col);
            Self::draw_text(s, ui, "M", ui.pt(13.0), Weight::Bold, ui::LABEL.now(), gx + TRACK_W - 62.0, y + 13.0);
            s.fill_rect((gx + TRACK_W - 36.0) as usize, (y + 8.0) as usize, 32, 28, solo_col);
            Self::draw_text(s, ui, "S", ui.pt(13.0), Weight::Bold, ui::LABEL.now(), gx + TRACK_W - 26.0, y + 13.0);
            for (c, sc) in p.scenes.iter().enumerate() {
                let x = gx + TRACK_W + c as f32 * SCENE_W;
                let fill = match t.clips.get(&sc.id) {
                    Some(_) if self.selected_clip == Some((r, c)) => ui::accent_now(),
                    Some(_) => ui::FIELD_BG.now(),
                    None => ui::CARD_BG.now(),
                };
                s.fill_rect((x + 3.0) as usize, (y + 3.0) as usize, (SCENE_W - 6.0) as usize, (CELL_H - 6.0) as usize, fill);
                match t.clips.get(&sc.id) {
                    Some(clip) => {
                        // clip name, then a mini preview of its notes along the bottom of the cell
                        Self::draw_text(s, ui, &clip.name, ui.pt(ui::MAC_CALLOUT_PT), Weight::Semibold, if self.selected_clip == Some((r, c)) { ui::WHITE } else { ui::LABEL.now() }, x + 10.0, y + 6.0);
                        let (lo, hi) = clip.notes.iter().fold((127.0f64, 0.0f64), |(lo, hi), n| (lo.min(n.p), hi.max(n.p)));
                        let (lo, hi) = if clip.notes.is_empty() { (48.0, 72.0) } else { (lo, hi.max(lo + 1.0)) };
                        let len = clip.length.max(1) as f32;
                        let cw = SCENE_W - 20.0;
                        for n in &clip.notes {
                            let nx = x + 10.0 + (n.t as f32 / len) * cw;
                            let nw = ((n.d as f32 / len) * cw).max(2.0);
                            let ny = y + CELL_H - 12.0 - ((n.p - lo) as f32 / (hi - lo) as f32) * 14.0;
                            s.fill_rect(nx as usize, ny as usize, nw as usize, 2, ui::SYSTEM_TEAL);
                        }
                    }
                    None => Self::draw_text(s, ui, "+", ui.pt(15.0), Weight::Regular, ui::LABEL_2.now(), x + SCENE_W / 2.0 - 5.0, y + 10.0),
                }
            }
        }
        // piano roll for the selected clip
        if let Some((r, c)) = self.selected_clip {
            let top = h - ROLL_H;
            s.fill_rect(0, top as usize, s.w, ROLL_H as usize, ui::FIELD_BG.now());
            let track = &p.tracks[r];
            let scene = &p.scenes[c].id;
            if let Some(clip) = track.clips.get(scene) {
                Self::draw_text(s, ui, &format!("{} · {}", track.name, p.scenes[c].name), ui.pt(13.0), Weight::Semibold, ui::LABEL_2.now(), MARGIN, top + 8.0);
                let (x0, y0, rw, rh) = (MARGIN, top + 32.0, w - 2.0 * MARGIN, ROLL_H - 44.0);
                let len = clip.length.max(1) as f32;
                let (lo, hi) = clip.notes.iter().fold((127.0f64, 0.0f64), |(lo, hi), n| (lo.min(n.p), hi.max(n.p)));
                let (lo, hi) = if clip.notes.is_empty() { (48.0, 72.0) } else { (lo, hi.max(lo + 1.0)) };
                for n in &clip.notes {
                    let nx = x0 + (n.t as f32 / len) * rw;
                    let nw = ((n.d as f32 / len) * rw).max(3.0);
                    let ny = y0 + rh - ((n.p - lo) as f32 / (hi - lo) as f32 * (rh - 8.0)) - 6.0;
                    s.fill_rect(nx as usize, ny as usize, nw as usize, 5, ui::accent_now());
                }
            }
        }
        self.draw_picker(s, ui, w, h);
    }
}

/// Reads the instrument types from the plugin manifests, keeping only those the engine can build.
fn load_instruments(plugins: &Path) -> Vec<(String, String, String)> {
    let mut out = vec![];
    let Ok(dir) = std::fs::read_dir(plugins) else { return out };
    for e in dir.flatten() {
        let Ok(text) = std::fs::read_to_string(e.path().join("plugin.json")) else { continue };
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) else { continue };
        if v["kind"] != "instrument" {
            continue;
        }
        let Some(kind) = v["type"].as_str() else { continue };
        if daw::devices::instrument_spec(kind).is_none() {
            continue;
        }
        out.push((kind.to_string(), v["name"].as_str().unwrap_or(kind).to_string(), v["desc"].as_str().unwrap_or("").to_string()));
    }
    out.sort_by(|a, b| a.1.to_lowercase().cmp(&b.1.to_lowercase()));
    out
}

impl Daw {
    fn picker_top(h: f32) -> f32 {
        h - ROLL_H
    }

    /// Instrument picker taps, over the bottom panel while it is open. Some(Nothing) swallows taps in the panel.
    fn picker_hit(&self, w: f32, h: f32, x: f32, y: f32) -> Option<Hit> {
        let page = self.picker?;
        let top = Self::picker_top(h);
        if y < top {
            return None;
        }
        if rect_contains((w - 220.0, top + 6.0, 60.0, 28.0), x, y) {
            return Some(Hit::PickPage(-1));
        }
        if rect_contains((w - 150.0, top + 6.0, 60.0, 28.0), x, y) {
            return Some(Hit::PickPage(1));
        }
        if rect_contains((w - 80.0, top + 6.0, 60.0, 28.0), x, y) {
            return Some(Hit::ClosePicker);
        }
        if y >= top + 40.0 {
            let row = ((y - top - 40.0) / PICK_ROW_H) as usize;
            let i = page * PICK_ROWS + row;
            if row < PICK_ROWS && i < self.instruments.len() {
                return Some(Hit::PickInst(i));
            }
        }
        Some(Hit::Nothing)
    }

    fn draw_picker(&self, s: &mut Surface, ui: &Ui, w: f32, h: f32) {
        let Some(page) = self.picker else { return };
        let top = Self::picker_top(h);
        s.fill_rect(0, top as usize, w as usize, ROLL_H as usize, ui::FIELD_BG.now());
        Self::draw_text(s, ui, "Choose an instrument", ui.pt(13.0), Weight::Semibold, ui::LABEL_2.now(), MARGIN, top + 12.0);
        let pages = self.instruments.len().div_ceil(PICK_ROWS).max(1);
        Self::draw_text(s, ui, &format!("{} / {}", page + 1, pages), ui.pt(13.0), Weight::Regular, ui::LABEL_2.now(), w - 300.0, top + 12.0);
        for (label, x, on) in [("Prev", w - 220.0, false), ("Next", w - 150.0, false), ("Close", w - 80.0, false)] {
            s.fill_rect(x as usize, (top + 6.0) as usize, 60, 28, if on { ui::accent_now() } else { ui::SEGMENT_ON.now() });
            Self::draw_text(s, ui, label, ui.pt(13.0), Weight::Semibold, if on { ui::WHITE } else { ui::LABEL.now() }, x + 10.0, top + 12.0);
        }
        for (i, (_, name, desc)) in self.instruments.iter().skip(page * PICK_ROWS).take(PICK_ROWS).enumerate() {
            let y = top + 40.0 + i as f32 * PICK_ROW_H;
            s.fill_rect(MARGIN as usize, y as usize, (w - 2.0 * MARGIN) as usize, (PICK_ROW_H - 4.0) as usize, ui::SEGMENT_ON.now());
            Self::draw_text(s, ui, name, ui.pt(15.0), Weight::Semibold, ui::LABEL.now(), MARGIN + 8.0, y + 8.0);
            let max = ((w - 2.0 * MARGIN - 240.0) / 6.0) as usize;
            let short: String = if desc.chars().count() > max { desc.chars().take(max.saturating_sub(1)).chain(std::iter::once('…')).collect() } else { desc.clone() };
            Self::draw_text(s, ui, &short, ui.pt(ui::MAC_CALLOUT_PT), Weight::Regular, ui::LABEL_2.now(), MARGIN + 220.0, y + 11.0);
        }
    }
}

impl Daw {
    fn draw_arrangement(&self, s: &mut Surface, ui: &Ui) {
        let w = s.w as f32;
        let Some(p) = &self.project else { return };
        s.fill_rect(self.lead as usize, 8, 80, 40, ui::FIELD_BG.now());
        Self::draw_text(s, ui, "< Session", ui.pt(15.0), Weight::Semibold, ui::TINT, self.lead + 8.0, 18.0);
        Self::draw_text(s, ui, "Arrangement", ui.pt(ui::MAC_TITLE1_PT), Weight::Bold, ui::LABEL.now(), self.lead + 110.0, 16.0);
        for (i, a) in p.arrangement.iter().enumerate() {
            let y = HEADER + 16.0 + i as f32 * ROW_H;
            s.fill_rect(MARGIN as usize, (y + 4.0) as usize, (w - 2.0 * MARGIN) as usize, (ROW_H - 8.0) as usize, ui::FIELD_BG.now());
            let name = p.scenes.iter().find(|sc| sc.id == a.scene_id).map(|sc| sc.name.clone()).unwrap_or_else(|| a.scene_id.clone());
            Self::draw_text(s, ui, &name, ui.pt(ui::MAC_TITLE3_PT), Weight::Semibold, ui::LABEL.now(), MARGIN + 16.0, y + 16.0);
            let muted = a.mutes.is_some();
            Self::draw_text(s, ui, &format!("{} bars", a.bars), ui.pt(14.0), Weight::Regular, ui::LABEL_2.now(), w - 320.0, y + 18.0);
            for (x, label) in [(w - 260.0, "-"), (w - 200.0, "+"), (w - 140.0, "x"), (w - 90.0, if muted { "unmute" } else { "mute" })] {
                s.fill_rect(x as usize, (y + 12.0) as usize, 40, 28, ui::SEGMENT_ON.now());
                Self::draw_text(s, ui, label, ui.pt(14.0), Weight::Semibold, ui::TINT, x + 12.0, y + 17.0);
            }
        }
        let base = HEADER + 16.0 + p.arrangement.len() as f32 * ROW_H;
        Self::draw_text(s, ui, "Add a scene to the arrangement:", ui.pt(13.0), Weight::Semibold, ui::LABEL_2.now(), MARGIN, base + 4.0);
        for (i, sc) in p.scenes.iter().enumerate() {
            let y = base + 28.0 + i as f32 * 40.0;
            Self::draw_text(s, ui, &format!("+ {}", sc.name), ui.pt(15.0), Weight::Regular, ui::TINT, MARGIN + 8.0, y);
        }
    }

    /// The rack inspector: one slider per parameter of the track's instrument and effects (value in its own unit),
    /// the track's stream format under the title, and the instrument's presets in the column on the right.
    fn draw_rack(&self, s: &mut Surface, ui: &Ui) {
        let Some(p) = &self.project else { return };
        let w = s.w as f32;
        s.fill_rect(self.lead as usize, 8, 80, 40, ui::FIELD_BG.now());
        Self::draw_text(s, ui, "< Session", ui.pt(15.0), Weight::Semibold, ui::TINT, self.lead + 8.0, 18.0);
        let title = p.tracks.get(self.rack_track).map(|t| t.name.clone()).unwrap_or_default();
        Self::draw_text(s, ui, &format!("Rack · {title}"), ui.pt(ui::MAC_TITLE1_PT), Weight::Bold, ui::LABEL.now(), self.lead + 110.0, 16.0);
        let format = format!("Stream: {}", rack::describe(daw::render::DEFAULT_SR, 2));
        Self::draw_text(s, ui, &format, ui.pt(ui::MAC_CALLOUT_PT), Weight::Regular, ui::LABEL_2.now(), self.lead + 110.0, 58.0);
        for (label, x) in [("< Track", w - 220.0), ("Track >", w - 110.0)] {
            s.fill_rect(x as usize, 16, 100, 32, ui::FIELD_BG.now());
            Self::draw_text(s, ui, label, ui.pt(15.0), Weight::Semibold, ui::TINT, x + 22.0, 22.0);
        }

        let Some(track) = p.tracks.get(self.rack_track) else { return };
        let (sx, sw) = rack_slider(w);
        for (i, row) in rack::rows(track).iter().enumerate() {
            let y = RACK_TOP + i as f32 * RACK_ROW_H;
            let selected = self.param.as_deref() == Some(row.key.as_str());
            let col = if selected { ui::accent_now() } else { ui::LABEL_2.now() };
            Self::draw_text(s, ui, &row.label, ui.pt(13.0), Weight::Regular, col, MARGIN + 8.0, y + 4.0);
            let empty = Extra::new();
            let value = rack::value(&row.item, rack::owner_extra(track, row.item.owner).unwrap_or(&empty));
            let pos = rack::position(&row.item, value);
            let q = row.item.param();
            s.fill_rect(sx as usize, (y + 11.0) as usize, sw as usize, 6, ui::FIELD_BG.now());
            s.fill_rect(sx as usize, (y + 11.0) as usize, (sw * pos) as usize, 6, ui::accent_now());
            s.fill_rect((sx + sw * pos - 4.0).max(0.0) as usize, (y + 7.0) as usize, 8, 14, ui::LABEL.now());
            let text = match (q.curve, q.unit) {
                (daw::devices::Curve::Int, "") => format!("{value:.0}"),
                (daw::devices::Curve::Int, unit) => format!("{value:.0} {unit}"),
                (_, "") => format!("{value:.2}"),
                (_, unit) => format!("{value:.2} {unit}"),
            };
            Self::draw_text(s, ui, &text, ui.pt(13.0), Weight::Regular, ui::LABEL_2.now(), sx + sw + 12.0, y + 4.0);
        }

        let (px, pw) = rack_panel(w);
        s.fill_rect(px as usize, RACK_TOP as usize, pw as usize, 32, ui::FIELD_BG.now());
        Self::draw_text(s, ui, "Save Preset", ui.pt(15.0), Weight::Semibold, ui::TINT, px + 12.0, RACK_TOP + 8.0);
        match self.rack_presets.as_ref() {
            Some(store) if !store.presets().is_empty() => {
                for (k, preset) in store.presets().iter().take(RACK_PRESET_ROWS).enumerate() {
                    let y = RACK_TOP + 44.0 + k as f32 * 32.0;
                    let current = store.current() == Some(preset.number);
                    let col = if current { ui::accent_now() } else { ui::LABEL.now() };
                    s.fill_rect(px as usize, y as usize, pw as usize, 28, ui::FIELD_BG.now());
                    Self::draw_text(s, ui, &preset.name, ui.pt(13.0), Weight::Regular, col, px + 12.0, y + 6.0);
                }
            }
            Some(_) => Self::draw_text(s, ui, "No presets yet", ui.pt(13.0), Weight::Regular, ui::LABEL_2.now(), px, RACK_TOP + 50.0),
            None => Self::draw_text(s, ui, "Presets unavailable", ui.pt(13.0), Weight::Regular, ui::LABEL_2.now(), px, RACK_TOP + 50.0),
        }
    }
}

impl Daw {
    /// Sets the header x: right of the traffic lights in the full-size window band (points to px).
    fn set_lead(&mut self, ui: &Ui) {
        self.lead = ui.pt(traffic_lights_trailing() + ui::SPACE_S);
    }

    /// Header buttons of the current view: (control id, accessible label, px rect). Published as controls so the
    /// top band does not drag the window under them; `hit` uses the same rectangles.
    fn header_buttons(&self, w: f32) -> Vec<(u64, &'static str, (f32, f32, f32, f32))> {
        let back = (self.lead, 8.0, 80.0, 40.0);
        match self.view {
            View::Songs => vec![(1, "New Song", (w - 150.0, 26.0, 130.0, 40.0))],
            View::Session => {
                let mut buttons = vec![
                    (1, "Back", back),
                    (2, "Play", (w - 220.0, 16.0, 100.0, 32.0)),
                    (3, "Arrange", (w - 330.0, 16.0, 100.0, 32.0)),
                    (4, "Rack", (w - 440.0, 16.0, 100.0, 32.0)),
                    (5, "Auto", (w - 550.0, 16.0, 100.0, 32.0)),
                    (6, "Stop", (w - 110.0, 16.0, 100.0, 32.0)),
                    (7, if self.trim.is_some() { "Cancel Trim" } else { "Trim" }, (w - 660.0, 16.0, 100.0, 32.0)),
                    (8, "Export", (w - 770.0, 16.0, 100.0, 32.0)),
                ];
                if self.trim.is_none() {
                    buttons.push((11, "Export CD", (w - 880.0, 16.0, 100.0, 32.0)));
                }
                if self.trim.is_some() {
                    buttons.push((9, "Start +1 Bar", (w - 880.0, 16.0, 100.0, 32.0)));
                    buttons.push((10, "End -1 Bar", (w - 990.0, 16.0, 100.0, 32.0)));
                }
                buttons
            }
            View::Arrangement => vec![(1, "Back", back), (2, "Play", (w - 220.0, 16.0, 100.0, 32.0))],
            View::Rack => vec![
                (1, "Back", back),
                (2, "Previous Track", (w - 220.0, 16.0, 100.0, 32.0)),
                (3, "Next Track", (w - 110.0, 16.0, 100.0, 32.0)),
            ],
        }
    }
}

impl App for Daw {
    fn id(&self) -> &str {
        "is.olibuijr.daw.desktop"
    }

    fn configure(&mut self, ctx: &Ctx) {
        self.songs = store::list(&self.store_dir);
        self.set_lead(&ctx.ui);
        self.system.start(ctx);
        // every edit is written at once and atomically, so nothing is left unsaved when the shell kills the app
        ctx.enable_sudden_termination();
    }

    fn controls(&self, ctx: &Ctx) -> Vec<Node> {
        let k = ctx.ui.pt(1.0);
        self.header_buttons(ctx.cfg.w as f32)
            .into_iter()
            .map(|(id, label, (x, y, w, h))| Node {
                id,
                role: Role::Button,
                label: label.to_string(),
                value: String::new(),
                bounds: Rect { x: x / k, y: y / k, w: w / k, h: h / k },
                enabled: true,
            })
            .collect()
    }

    fn event(&mut self, ctx: &Ctx, ev: Input) -> bool {
        self.set_lead(&ctx.ui);
        let (w, h) = (ctx.cfg.w as f32, ctx.cfg.h as f32);
        match ev {
            Input::Touch { kind: TouchKind::Down, x, y, .. } => {
                let handled = self.touch(w, h, x, y);
                if std::mem::take(&mut self.pending_import) {
                    ctx.open_document(DocMode::Read, "Import MIDI File", &["mid", "midi"]);
                }
                handled
            }
            _ => false,
        }
    }

    /// The open panel's answer: a chosen .mid file is read and imported. Cancelling leaves the song alone.
    fn document(&mut self, _ctx: &Ctx, ev: DocumentEvent) -> bool {
        match ev {
            DocumentEvent::Opened(doc) => {
                let name = doc.name.trim_end_matches(".mid").trim_end_matches(".midi").to_string();
                let mut file = doc.file;
                let mut bytes = Vec::new();
                match file.read_to_end(&mut bytes) {
                    Ok(_) => self.import_midi(&bytes, &name),
                    Err(e) => self.status = format!("Could not read {name}: {e}"),
                }
                true
            }
            DocumentEvent::Failed { code: DocErrorCode::Cancelled, .. } => false,
            DocumentEvent::Failed { .. } => {
                self.status = "The MIDI file could not be opened".into();
                true
            }
            _ => false,
        }
    }

    fn service_reply(&mut self, _ctx: &Ctx, request: u32, result: &Result<Json, String>) -> bool {
        self.system.reply(request, result)
    }

    fn service_event(&mut self, ctx: &Ctx, service: &str, event: &str, body: &Json) -> bool {
        match self.system.event(ctx, service, event, body) {
            system::Event::Ignored => false,
            system::Event::Route(text) => {
                self.status = text;
                true
            }
            system::Event::Power => true,
            system::Event::Command(cmd) => {
                self.command(cmd);
                true
            }
        }
    }

    fn draw(&mut self, ctx: &Ctx, s: &mut Surface, t: f64) -> Damage {
        self.now = t;
        self.set_lead(&ctx.ui);
        if let Some(player) = self.player.as_mut() {
            player.tick(t);
            self.playing = player.playing();
        }
        let transport = self.transport();
        if self.system.shown() != Some(transport) {
            let position = self.player.as_ref().map_or(0, |p| p.position_ms().max(0.0) as u64);
            let title = self.project.as_ref().map(|p| p.name.clone()).unwrap_or_default();
            self.system.sync_session(ctx, transport, &title, position);
        }
        let ui: &Ui = &ctx.ui;
        s.fill_rect(0, 0, s.w, s.h, ui::CONTENT_BG.now());
        match self.view {
            View::Songs => self.draw_songs(s, ui),
            View::Session => self.draw_session(s, ui),
            View::Arrangement => self.draw_arrangement(s, ui),
            View::Rack => self.draw_rack(s, ui),
        }
        if !self.status.is_empty() {
            Self::draw_text(s, ui, &self.status, ui.pt(ui::MAC_CALLOUT_PT), Weight::Regular, ui::LABEL_2.now(), MARGIN, s.h as f32 - 16.0);
        }
        if self.system.low_power {
            let warning = "Low Power Mode is on. Playback and rendering may be slower.";
            Self::draw_text(s, ui, warning, ui.pt(ui::MAC_CALLOUT_PT), Weight::Regular, ui::SYSTEM_ORANGE, MARGIN, s.h as f32 - 40.0);
        }
        let mut d = Damage::new(s.w, s.h);
        d.all();
        d
    }
}

fn main() {
    let data = std::env::var("BIFROST_DATA").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from("."));
    bifrost_app::run(Daw::new(data))
}

#[cfg(test)]
mod accent_tests {
    use super::*;
    use bifrost_app::kit::appearance::{accent, set_accent, Accent};

    /// The accent is process-wide: this test changes it and restores it.
    static ACCENT_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn an_on_toggle_is_filled_with_the_chosen_accent_not_system_blue() {
        let _g = ACCENT_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let keep = accent();
        set_accent(Accent::Green);
        let (on, off, green) = (toggle_fill(true), toggle_fill(false), ui::accent_now());
        set_accent(keep);
        assert_eq!(on, green);
        assert_ne!(on, ui::TINT);
        assert_eq!(off, ui::FIELD_BG.now());
    }
}
