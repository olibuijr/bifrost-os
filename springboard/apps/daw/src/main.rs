//! DAW for BifrOSt PC: song list, session grid (tracks by scenes, mute/solo), piano roll for the selected clip,
//! and transport through the shell's audio service.
use bifrost_app::controls::{Node, Rect, Role};
use bifrost_app::kit::{ui, Damage, Surface, Ui, Weight};
use bifrost_app::layout::traffic_lights_trailing;
use bifrost_app::{App, Ctx, Input, TouchKind};
use daw::ai;
use daw::model::{Instrument, Project, Scene, Track};
use daw::playback::Player;
use daw::store::{self, Summary};
use std::path::{Path, PathBuf};

const MARGIN: f32 = 20.0;
const HEADER: f32 = 96.0;
const ROW_H: f32 = 56.0;
const TRACK_W: f32 = 180.0;
const CELL_H: f32 = 44.0;
const SCENE_W: f32 = 120.0;
const ROLL_H: f32 = 260.0;
const PICK_ROWS: usize = 5;
const PICK_ROW_H: f32 = 40.0;

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
    Nothing,
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
        Daw { store_dir, songs, view: View::Songs, project: None, selected_clip: None, status: String::new(), player, playing: false, now: 0.0, auto_mode: false, param: None, rack_track: 0, picker: None, instruments, lead: 0.0 }
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

    fn scenes(&self) -> Vec<String> {
        self.project.as_ref().map(|p| p.scenes.iter().map(|s| s.id.clone()).collect()).unwrap_or_default()
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
                let Some(p) = &self.project else { return Hit::Nothing };
                let row = ((y - HEADER - 16.0) / 28.0).floor();
                if row < 0.0 || x < MARGIN {
                    return Hit::Nothing;
                }
                let i = row as usize;
                if i < self.rack_rows(p).len() {
                    Hit::RackParam(i)
                } else {
                    Hit::Nothing
                }
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

    /// Rack rows: (automation key, label, spec min, spec max) for the instrument and every effect.
    fn rack_rows(&self, p: &Project) -> Vec<(String, String, f64, f64)> {
        let mut out = vec![];
        if let Some(spec) = daw::devices::instrument_spec(&p.tracks.get(self.rack_track).map(|t| t.instrument.kind.clone()).unwrap_or_default()) {
            for q in spec {
                out.push((format!("inst.{}", q.name), format!("instrument · {}", q.name), q.min, q.max));
            }
        }
        if let Some(t) = p.tracks.get(self.rack_track) {
            for e in &t.effects {
                if let Some(spec) = daw::devices::effect_spec(&e.kind) {
                    for q in spec {
                        out.push((format!("{}.{}", e.id, q.name), format!("{} · {}", e.kind, q.name), q.min, q.max));
                    }
                }
            }
        }
        out
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
            Hit::Arrange => self.view = View::Arrangement,
            Hit::Rack => {
                if let Some((r, _)) = self.selected_clip {
                    self.rack_track = r;
                }
                self.view = View::Rack;
            }
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

    fn play(&mut self) {
        let (Some(p), Some(player)) = (self.project.as_ref(), self.player.as_mut()) else {
            self.status = "audio service not available".into();
            return;
        };
        match player.play(p) {
            Ok(path) => {
                self.playing = true;
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
    }

    fn draw_text(s: &mut Surface, ui: &Ui, text: &str, pt: f32, weight: Weight, color: u32, x: f32, y: f32) {
        let img = ui.text(text, pt, weight, color);
        s.blit(&img, x as i32, y as i32, 255, 255);
    }

    fn draw_songs(&self, s: &mut Surface, ui: &Ui) {
        Self::draw_text(s, ui, "DAW", ui.pt(26.0), Weight::Bold, ui::LABEL.now(), self.lead, 22.0);
        let w = s.w as f32;
        s.fill_rect((w - 150.0) as usize, 26, 130, 40, ui::TINT);
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
        let play_col = if self.playing { ui::LABEL_2.now() } else { ui::TINT };
        s.fill_rect((w - 220.0) as usize, 16, 100, 32, play_col);
        Self::draw_text(s, ui, "Play", ui.pt(15.0), Weight::Semibold, ui::WHITE, w - 190.0, 22.0);
        s.fill_rect((w - 110.0) as usize, 16, 100, 32, ui::FIELD_BG.now());
        Self::draw_text(s, ui, "Stop", ui.pt(15.0), Weight::Semibold, ui::LABEL.now(), w - 80.0, 22.0);
        for (x, label, on) in [(w - 330.0, "Arrange", false), (w - 440.0, "Rack", false), (w - 550.0, "Auto", self.auto_mode)] {
            s.fill_rect(x as usize, 16, 100, 32, if on { ui::TINT } else { ui::FIELD_BG.now() });
            Self::draw_text(s, ui, label, ui.pt(14.0), Weight::Semibold, if on { ui::WHITE } else { ui::LABEL.now() }, x + 12.0, 22.0);
        }
        let (gx, gy, _gw, _gh) = self.layout_session(s.w as f32, s.h as f32);
        // the scene row: Stop in the head column
        s.fill_rect(gx as usize, (gy + 4.0) as usize, (TRACK_W - 8.0) as usize, 24, ui::FIELD_BG.now());
        Self::draw_text(s, ui, "Stop", ui.pt(13.0), Weight::Semibold, ui::LABEL.now(), gx + 12.0, gy + 7.0);
        // the add row under the last track: + Track, and + Scene while under 8 scenes
        let add_y = gy + 32.0 + p.tracks.len() as f32 * CELL_H;
        Self::draw_text(s, ui, "+ Track", ui.pt(15.0), Weight::Semibold, ui::TINT, gx + 8.0, add_y + 12.0);
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
            let mute_col = if t.mute { ui::TINT } else { ui::FIELD_BG.now() };
            let solo_col = if t.solo { ui::TINT } else { ui::FIELD_BG.now() };
            s.fill_rect((gx + TRACK_W - 72.0) as usize, (y + 8.0) as usize, 32, 28, mute_col);
            Self::draw_text(s, ui, "M", ui.pt(13.0), Weight::Bold, ui::LABEL.now(), gx + TRACK_W - 62.0, y + 13.0);
            s.fill_rect((gx + TRACK_W - 36.0) as usize, (y + 8.0) as usize, 32, 28, solo_col);
            Self::draw_text(s, ui, "S", ui.pt(13.0), Weight::Bold, ui::LABEL.now(), gx + TRACK_W - 26.0, y + 13.0);
            for (c, sc) in p.scenes.iter().enumerate() {
                let x = gx + TRACK_W + c as f32 * SCENE_W;
                let fill = match t.clips.get(&sc.id) {
                    Some(_) if self.selected_clip == Some((r, c)) => ui::TINT,
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
                    s.fill_rect(nx as usize, ny as usize, nw as usize, 5, ui::TINT);
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
            s.fill_rect(x as usize, (top + 6.0) as usize, 60, 28, if on { ui::TINT } else { ui::SEGMENT_ON.now() });
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

    fn draw_rack(&self, s: &mut Surface, ui: &Ui) {
        let Some(p) = &self.project else { return };
        s.fill_rect(self.lead as usize, 8, 80, 40, ui::FIELD_BG.now());
        Self::draw_text(s, ui, "< Session", ui.pt(15.0), Weight::Semibold, ui::TINT, self.lead + 8.0, 18.0);
        let title = p.tracks.get(self.rack_track).map(|t| t.name.clone()).unwrap_or_default();
        Self::draw_text(s, ui, &format!("Rack · {title}"), ui.pt(ui::MAC_TITLE1_PT), Weight::Bold, ui::LABEL.now(), self.lead + 110.0, 16.0);
        for (i, (key, label, lo, hi)) in self.rack_rows(p).iter().enumerate() {
            let y = HEADER + 16.0 + i as f32 * 28.0;
            let col = if self.param.as_deref() == Some(key.as_str()) { ui::TINT } else { ui::LABEL_2.now() };
            Self::draw_text(s, ui, &format!("{label}  [{lo:.2} .. {hi:.2}]"), ui.pt(13.0), Weight::Regular, col, MARGIN + 8.0, y + 4.0);
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
            View::Session => vec![
                (1, "Back", back),
                (2, "Play", (w - 220.0, 16.0, 100.0, 32.0)),
                (3, "Arrange", (w - 330.0, 16.0, 100.0, 32.0)),
                (4, "Rack", (w - 440.0, 16.0, 100.0, 32.0)),
                (5, "Auto", (w - 550.0, 16.0, 100.0, 32.0)),
                (6, "Stop", (w - 110.0, 16.0, 100.0, 32.0)),
            ],
            View::Arrangement => vec![(1, "Back", back), (2, "Play", (w - 220.0, 16.0, 100.0, 32.0))],
            View::Rack => vec![(1, "Back", back)],
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
            Input::Touch { kind: TouchKind::Down, x, y, .. } => self.touch(w, h, x, y),
            _ => false,
        }
    }

    fn draw(&mut self, ctx: &Ctx, s: &mut Surface, t: f64) -> Damage {
        self.now = t;
        self.set_lead(&ctx.ui);
        if let Some(player) = self.player.as_mut() {
            player.tick(t);
            self.playing = player.playing();
        }
        let ui: &Ui = &ctx.ui;
        s.fill_rect(0, 0, s.w, s.h, ui::DETAIL_BG.now());
        match self.view {
            View::Songs => self.draw_songs(s, ui),
            View::Session => self.draw_session(s, ui),
            View::Arrangement => self.draw_arrangement(s, ui),
            View::Rack => self.draw_rack(s, ui),
        }
        if !self.status.is_empty() {
            Self::draw_text(s, ui, &self.status, ui.pt(ui::MAC_CALLOUT_PT), Weight::Regular, ui::LABEL_2.now(), MARGIN, s.h as f32 - 16.0);
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
