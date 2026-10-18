//! One movie open in the window: the shell's media player behind the fullscreen player view (the same view Files and
//! Deildu use), the captions of a sidecar subtitle file drawn over the picture, and trim marks that loop playback.
//!
//! The view owns the HUD, scrubbing and the double-tap skips; this type adds what the player view does not know:
//! captions from `springboard::media::subtitles` and the trim range from the shared recording model.

use bifrost_app::controls::recording::Trim;
use bifrost_app::kit::ui::{CELL_RADIUS_PT, SPACE_L};
use bifrost_app::kit::{Event, Image, Surface, Ui, Weight};
use bifrost_app::kit::ui;
use springboard::deildu::play::{mime_for, PlayState, Player};
use springboard::deildu::player_view::{Outcome, PlayerView};
use springboard::media::pickers::Subtitles;
use springboard::media::subtitles::{sidecars_in, Track};
use springboard::shell::video::MediaPlayer;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Subtitle files larger than this are not read (a caption file is a few hundred kilobytes at most).
const MAX_SUBTITLE_BYTES: u64 = 2 * 1024 * 1024;
const CAPTION_PT: f32 = 17.0;

pub struct Movie {
    path: PathBuf,
    player: MediaPlayer,
    view: PlayerView,
    /// the parsed subtitle files, in the order of the picker's rows after Off
    tracks: Vec<Track>,
    /// the choice: Off or one of `tracks`
    subtitles: Subtitles,
    trim: Option<Trim>,
    /// the cue text being shown, and its lines rendered once per change
    caption: Option<String>,
    caption_lines: Vec<Image>,
}

/// The clock text of a time in seconds, `m:ss`.
pub fn clock(seconds: f64) -> String {
    let s = seconds.max(0.0) as u64;
    format!("{}:{:02}", s / 60, s % 60)
}

/// The subtitle files next to `movie` (`name.srt`, `name.vtt` and one per language, `name.en.srt`): the label of each
/// and its parsed track. A file that is too large, unreadable or without a cue is left out.
fn load_subtitles(movie: &Path) -> (Vec<String>, Vec<Track>) {
    let dir = match movie.parent() {
        Some(d) if !d.as_os_str().is_empty() => d.to_path_buf(),
        _ => PathBuf::from("."),
    };
    let names: Vec<String> = match fs::read_dir(&dir) {
        Ok(entries) => entries.flatten().filter_map(|e| e.file_name().into_string().ok()).collect(),
        Err(_) => Vec::new(),
    };
    let mut labels = Vec::new();
    let mut tracks = Vec::new();
    for sidecar in sidecars_in(movie, &names) {
        let path = dir.join(&sidecar.file_name);
        let Ok(meta) = fs::metadata(&path) else { continue };
        if !meta.is_file() || meta.len() > MAX_SUBTITLE_BYTES {
            continue;
        }
        let Ok(text) = fs::read_to_string(&path) else { continue };
        let track = Track::parse(&text);
        if !track.is_empty() {
            labels.push(sidecar.label);
            tracks.push(track);
        }
    }
    (labels, tracks)
}

impl Movie {
    /// Opens `path` at a window of `w` by `h` pixels and starts it playing. `title` names it in the player view.
    pub fn open(ui: &Ui, w: i32, h: i32, path: &Path, title: &str) -> io::Result<Movie> {
        let mut player = MediaPlayer::new(w, h);
        player.open(path, mime_for(&path.to_string_lossy()))?;
        let view = PlayerView::new(ui, w, h, title, "");
        let (labels, tracks) = load_subtitles(path);
        let mut subtitles = Subtitles::new(labels);
        if !tracks.is_empty() {
            // the first file shows at once, as a sidecar always did
            subtitles.choose(1);
        }
        Ok(Movie {
            path: path.to_path_buf(),
            player,
            view,
            tracks,
            subtitles,
            trim: None,
            caption: None,
            caption_lines: Vec::new(),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn set_size(&mut self, w: i32, h: i32) {
        self.view.set_size(w, h);
    }

    /// Pauses playback (the window left the foreground).
    pub fn pause(&mut self) {
        self.player.pause();
    }

    pub fn has_subtitles(&self) -> bool {
        !self.tracks.is_empty()
    }

    /// The subtitle picker's rows: Off, then one per subtitle file.
    pub fn subtitle_rows(&self) -> Vec<String> {
        self.subtitles.rows()
    }

    /// The picker row that is chosen (0 is Off).
    pub fn subtitle_row(&self) -> usize {
        self.subtitles.selected_row()
    }

    /// Chooses a subtitle row (0 is Off); the caption on screen is dropped so the new track draws from the next tick.
    /// False for a row that does not exist.
    pub fn choose_subtitles(&mut self, row: usize) -> bool {
        let ok = self.subtitles.choose(row);
        if ok {
            self.caption = None;
            self.caption_lines.clear();
        }
        ok
    }

    /// The playback speed in force (1.0 is normal).
    pub fn speed(&self) -> f32 {
        self.player.rate()
    }

    /// Plays at `rate` times normal speed (0.5 to 3.0); the sound is time-stretched so its pitch holds.
    pub fn set_speed(&mut self, rate: f32) {
        self.player.set_rate(rate);
    }

    /// The trim range, if one is set.
    pub fn trim(&self) -> Option<Trim> {
        self.trim
    }

    /// Moves the trim start to the playhead, starting a whole-movie trim when none is set. False when the length is not
    /// known yet or the playhead is too close to the end for a trim to keep `MIN_TRIM` of movie.
    pub fn set_trim_start(&mut self) -> bool {
        let Some(length) = self.length_s() else { return false };
        let mut trim = match self.trim.or_else(|| Trim::full(length)) {
            Some(t) => t,
            None => return false,
        };
        if !trim.set_start(self.playhead_s()) {
            return false;
        }
        self.trim = Some(trim);
        true
    }

    /// Moves the trim end to the playhead, starting a whole-movie trim when none is set.
    pub fn set_trim_end(&mut self) -> bool {
        let Some(length) = self.length_s() else { return false };
        let mut trim = match self.trim.or_else(|| Trim::full(length)) {
            Some(t) => t,
            None => return false,
        };
        if !trim.set_end(self.playhead_s()) {
            return false;
        }
        self.trim = Some(trim);
        true
    }

    pub fn clear_trim(&mut self) {
        self.trim = None;
    }

    /// True when the trim keeps less than the whole movie, so playback loops inside it.
    fn partial_trim(&self) -> Option<Trim> {
        self.trim.filter(|t| t.start() > 0.0 || t.end() < t.length())
    }

    fn length_s(&self) -> Option<f64> {
        self.player.duration_ms().map(|d| d / 1000.0)
    }

    fn playhead_s(&self) -> f64 {
        self.player.position_ms() / 1000.0
    }

    pub fn touch(&mut self, ui: &Ui, ev: Event, t: f64) -> Outcome {
        self.view.touch(ui, &mut self.player, ev, t)
    }

    /// A hardware key for the player (space, arrows). False when the view does not use it.
    pub fn key(&mut self, code: u16, t: f64) -> bool {
        self.view.key(&mut self.player, code, t)
    }

    /// Advances the player view, loops a partial trim and refreshes the caption. True when the picture needs a redraw.
    pub fn tick(&mut self, ui: &Ui, t: f64) -> bool {
        let looped = self.wrap_trim();
        let captioned = self.update_caption(ui);
        let drawn = self.view.tick(ui, &mut self.player, t);
        looped || captioned || drawn
    }

    /// Plays from the trim start again once the playhead reaches the trim end. True when it seeked.
    fn wrap_trim(&mut self) -> bool {
        let Some(trim) = self.partial_trim() else { return false };
        if self.player.state() != PlayState::Playing || self.playhead_s() < trim.end() {
            return false;
        }
        self.player.seek_ms(trim.start() * 1000.0);
        true
    }

    /// Picks the cue for the playhead and renders its lines when it changed. True when the caption changed.
    fn update_caption(&mut self, ui: &Ui) -> bool {
        let ms = self.player.position_ms().max(0.0) as u64;
        let text = self.subtitles.track().and_then(|i| self.tracks.get(i)).and_then(|t| t.text_at(ms)).map(str::to_string);
        if text == self.caption {
            return false;
        }
        let max = (self.view.size().0 as f32 - 2.0 * ui.pt(SPACE_L)).max(0.0);
        self.caption_lines = match text.as_deref() {
            Some(t) => t.lines().map(|line| ui.text(&ui.fit(line, CAPTION_PT, max), CAPTION_PT, Weight::Semibold, ui::WHITE)).collect(),
            None => Vec::new(),
        };
        self.caption = text;
        true
    }

    pub fn draw(&self, s: &mut Surface, ui: &Ui, t: f64) {
        self.view.draw(s, ui, t);
        self.draw_caption(s, ui);
    }

    /// The caption lines on a dark plate, centred above the HUD's scrubber.
    fn draw_caption(&self, s: &mut Surface, ui: &Ui) {
        if self.caption_lines.is_empty() {
            return;
        }
        let pad = ui.pt(CELL_RADIUS_PT);
        let width = self.caption_lines.iter().map(|img| img.w as f32).fold(0.0, f32::max) + 2.0 * pad;
        let height = self.caption_lines.iter().map(|img| img.h as f32).sum::<f32>() + 2.0 * pad;
        let (w, h) = self.view.size();
        let x = (w as f32 - width) / 2.0;
        let top = h as f32 * 0.88 - height;
        s.fill_rrect(x, top, width, height, ui.pt(CELL_RADIUS_PT), ui::MATERIAL_HUD.now());
        let mut y = top + pad;
        for img in &self.caption_lines {
            let ix = x + (width - img.w as f32) / 2.0;
            s.blit(img, ix as i32, y as i32, 255, 255);
            y += img.h as f32;
        }
    }

    /// The trim range as text for the window's notice, e.g. "Trim 0:12 to 1:03".
    pub fn trim_text(&self) -> Option<String> {
        self.trim.map(|t| format!("Trim {} to {}", clock(t.start()), clock(t.end())))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clock_reads_minutes_and_seconds() {
        assert_eq!(clock(0.0), "0:00");
        assert_eq!(clock(9.7), "0:09");
        assert_eq!(clock(63.0), "1:03");
        assert_eq!(clock(-4.0), "0:00");
    }

    #[test]
    fn a_whole_movie_trim_does_not_loop() {
        let whole = Trim::full(120.0);
        let partial = Trim::new(10.0, 60.0, 120.0);
        let keep = |t: Option<Trim>| t.filter(|t| t.start() > 0.0 || t.end() < t.length());
        assert_eq!(keep(whole), None);
        assert_eq!(keep(partial), partial);
    }

    #[test]
    fn sidecar_subtitles_load_when_present() {
        let dir = std::env::temp_dir().join(format!("videoplayer-test-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let movie = dir.join("clip.mp4");
        assert!(load_subtitles(&movie).1.is_empty());
        fs::write(dir.join("clip.srt"), "1\n00:00:00,500 --> 00:00:02,000\nHi\n").unwrap();
        let (labels, tracks) = load_subtitles(&movie);
        assert_eq!(labels, vec!["Subtitles".to_string()]);
        assert_eq!(tracks[0].text_at(1000), Some("Hi"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn every_language_file_is_loaded_in_name_order_and_the_empty_ones_are_skipped() {
        let dir = std::env::temp_dir().join(format!("videoplayer-test-lang-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let movie = dir.join("clip.mp4");
        fs::write(dir.join("clip.is.srt"), "1\n00:00:00,500 --> 00:00:02,000\nHalo\n").unwrap();
        fs::write(dir.join("clip.en.srt"), "1\n00:00:00,500 --> 00:00:02,000\nHello\n").unwrap();
        fs::write(dir.join("clip.de.srt"), "no cues here").unwrap();
        let (labels, tracks) = load_subtitles(&movie);
        assert_eq!(labels, vec!["en".to_string(), "is".to_string()]);
        assert_eq!(tracks[0].text_at(1000), Some("Hello"));
        assert_eq!(tracks[1].text_at(1000), Some("Halo"));
        let mut picker = Subtitles::new(labels);
        assert_eq!(picker.rows(), vec!["Off", "en", "is"]);
        assert!(picker.choose(2));
        assert_eq!(picker.track(), Some(1));
        let _ = fs::remove_dir_all(&dir);
    }
}
