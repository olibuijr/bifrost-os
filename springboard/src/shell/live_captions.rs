//! Live Captions: the words in the sound the computer plays, drawn as captions over everything.
//!
//! Settings > Accessibility > Live Captions (`a11y_live_captions`, off by default) turns it on. While it is on and the
//! screen is lit, the shell asks the `speech_recognizer` service for a caption pipeline (`captions_start`): the same
//! on-device whisper model dictation uses, fed from the monitor of the default output instead of the microphone, so it
//! hears what the computer plays and not the room. Nothing leaves the machine, and the service refuses without the
//! consent and the model that Keyboard > Dictation manages; the caption plate then says so once.
//!
//! The shell polls the pipeline (`captions_poll`) every [`POLL_MS`]. A hypothesis replaces the line being spoken; a
//! final transcript is kept as a finished line. The plate shows the newest two lines of text at the bottom centre,
//! stays [`IDLE_MS`] after the last words and fades out, and is drawn in the caption look of
//! `bifrost_ui::captions` (text colour and size). Stored caption profiles are not wired to Settings yet, so the look is
//! the model's default.

use super::services::recognizer::{SERVICE, SHELL_APP};
use super::ui::{Ui, Weight, MAC_TITLE3_PT};
use super::Shell;
use crate::gfx::{rgba, scale, Surface};
use bifrost_core::json::Json;
use bifrost_ui::captions::{CaptionAppearance, Domain, Rgb, Settings};

/// The pref Settings > Accessibility > Live Captions writes (`AccessibilitySettings::live_captions`).
pub const SWITCH: &str = "a11y_live_captions";

/// How often the pipeline is polled while captions run, in milliseconds.
pub const POLL_MS: f64 = 250.0;
/// The plate stays this long after the last words, then fades.
pub const IDLE_MS: f64 = 5000.0;
/// The fade-out.
pub const FADE_MS: f64 = 300.0;
/// A pipeline that ended on its own is started again after this long.
pub const RESTART_MS: f64 = 5000.0;
/// A notice (captions could not start) stays this long.
pub const NOTICE_MS: f64 = 7000.0;
/// Lines of text on the plate.
const LINES: usize = 2;
/// Finished lines kept behind the one being spoken.
const KEPT: usize = 6;
/// The plate's width as a share of the screen, within these bounds in points.
const WIDTH_SHARE: f32 = 0.6;
const WIDTH_MIN_PT: f32 = 320.0;
const WIDTH_MAX_PT: f32 = 760.0;
/// The gap between the plate and the bottom edge, clear of the dock.
const BOTTOM_PT: f32 = 96.0;
const PAD_PT: f32 = 12.0;
/// Line height as a multiple of the text size.
const LEADING: f32 = 1.35;
/// Grouped-card radius (design-golden-gate.md): the plate is a card.
const RADIUS_PT: f32 = 13.0;

/// What the person sees when captions cannot start.
pub fn notice_for(error: &str) -> &'static str {
    if error.contains("consent") || error.contains("model is not installed") {
        "Live Captions needs the speech model. Turn on Dictation in Settings > Keyboard."
    } else {
        "Live Captions could not start."
    }
}

/// The caption plate and the pipeline state behind it.
pub struct Captions {
    /// finished lines, oldest first
    done: Vec<String>,
    /// the line being spoken, as the latest hypothesis has it
    live: String,
    /// when words last arrived
    last_text: Option<f64>,
    /// a message in place of words, and when it was set
    notice: Option<(String, f64)>,
    running: bool,
    last_poll: f64,
    /// no start before this time; infinity after a refusal, until the switch is turned off and on
    retry_at: f64,
    look: Settings,
}

impl Default for Captions {
    fn default() -> Self {
        Captions { done: Vec::new(), live: String::new(), last_text: None, notice: None, running: false, last_poll: f64::NEG_INFINITY, retry_at: 0.0, look: CaptionAppearance::new().settings(Domain::User) }
    }
}

impl Captions {
    /// Takes the transcripts of one poll: `(text, is_final)` in order.
    pub fn apply(&mut self, transcripts: &[(String, bool)], t: f64) {
        for (text, is_final) in transcripts {
            if *is_final {
                self.done.push(text.clone());
                self.live.clear();
            } else {
                self.live = text.clone();
            }
        }
        if self.done.len() > KEPT {
            self.done.drain(..self.done.len() - KEPT);
        }
        if !transcripts.is_empty() {
            self.last_text = Some(t);
            self.notice = None;
        }
    }

    /// The text on the plate: the finished lines and the one being spoken, run together.
    fn text(&self) -> String {
        let mut parts: Vec<&str> = self.done.iter().map(String::as_str).collect();
        if !self.live.is_empty() {
            parts.push(&self.live);
        }
        parts.join(" ")
    }

    /// The text or the notice on show, and when it appeared.
    fn shown(&self) -> Option<(String, f64)> {
        if let Some(at) = self.last_text.filter(|_| !self.text().is_empty()) {
            return Some((self.text(), at));
        }
        self.notice.clone().map(|(m, at)| (m, at))
    }

    /// 1 while holding, falling to 0 through the fade, 0 with nothing to show.
    pub fn alpha(&self, t: f64) -> f32 {
        let hold = if self.last_text.is_some() && !self.text().is_empty() { IDLE_MS } else { NOTICE_MS };
        match self.shown() {
            Some((_, at)) => (1.0 - (t - at - hold) / FADE_MS).clamp(0.0, 1.0) as f32,
            None => 0.0,
        }
    }

    pub fn visible(&self, t: f64) -> bool {
        self.alpha(t) > 0.0
    }

    /// Whether the pipeline runs.
    #[cfg(test)]
    pub fn running(&self) -> bool {
        self.running
    }

    /// The text on the plate now, empty when there is none.
    #[cfg(test)]
    pub fn current_text(&self) -> String {
        self.shown().map(|(m, _)| m).unwrap_or_default()
    }

    /// When the loop must wake: the next poll while the pipeline runs, and the end of the plate's life.
    pub fn next_wake(&self, t: f64) -> Option<f64> {
        let poll = self.running.then(|| (self.last_poll + POLL_MS).max(t));
        let fade = self.shown().map(|(_, at)| at + if self.last_text.is_some() && !self.text().is_empty() { IDLE_MS } else { NOTICE_MS }).filter(|&end| end > t);
        match (poll, fade) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }

    /// Forgets words and notices that have faded; true when something was removed (one frame to repaint).
    fn expire(&mut self, t: f64) -> bool {
        if self.shown().is_some() && !self.visible(t) {
            self.done.clear();
            self.live.clear();
            self.last_text = None;
            self.notice = None;
            return true;
        }
        false
    }

    fn size_pt(&self) -> f32 {
        MAC_TITLE3_PT * self.look.relative_size.max(0.5)
    }

    /// (x, y, w, h) in px of the largest plate for a screen `w` by `h`: the box to repaint.
    pub fn rect(&self, ui: &Ui, w: i32, h: i32) -> (f32, f32, f32, f32) {
        let pw = (w as f32 * WIDTH_SHARE).clamp(ui.pt(WIDTH_MIN_PT), ui.pt(WIDTH_MAX_PT));
        let ph = ui.pt(2.0 * PAD_PT + LINES as f32 * self.size_pt() * LEADING);
        ((w as f32 - pw) / 2.0, h as f32 - ui.pt(BOTTOM_PT) - ph, pw, ph)
    }

    /// The lines the plate shows: the newest [`LINES`] that fit its width.
    pub fn lines(&self, ui: &Ui, plate_w: f32) -> Vec<String> {
        let Some((text, _)) = self.shown() else { return Vec::new() };
        let wrapped = ui.wrap(&text, self.size_pt(), Weight::Semibold, plate_w - ui.pt(2.0 * PAD_PT));
        let skip = wrapped.len().saturating_sub(LINES);
        wrapped.into_iter().skip(skip).collect()
    }

    pub fn draw(&self, s: &mut Surface, ui: &Ui, w: i32, h: i32, t: f64) {
        let alpha = (self.alpha(t) * 255.0) as u32;
        if alpha == 0 {
            return;
        }
        let lines = self.lines(ui, self.rect(ui, w, h).2);
        if lines.is_empty() {
            return;
        }
        let (x, y, pw, ph) = self.rect(ui, w, h);
        let line_h = ui.pt(self.size_pt() * LEADING);
        // the plate hugs its lines and sits on the bottom of the box, so one line is not a tall plate
        let plate_h = ui.pt(2.0 * PAD_PT) + line_h * lines.len() as f32;
        let py = y + ph - plate_h;
        s.fill_rrect(x, py, pw, plate_h, ui.pt(RADIUS_PT), scale(rgba(28, 28, 30, 235), alpha));
        let c = |v: f32| (v.clamp(0.0, 1.0) * 255.0) as u8;
        let Rgb { r, g, b } = self.look.text;
        let color = rgba(c(r), c(g), c(b), 255);
        for (i, line) in lines.iter().enumerate() {
            let img = ui.text(line, self.size_pt(), Weight::Semibold, super::ui::WHITE);
            let tx = x + (pw - img.w as f32) / 2.0;
            let ty = py + ui.pt(PAD_PT) + line_h * i as f32 + (line_h - img.h as f32) / 2.0;
            s.blit_colored(&img, tx as i32, ty as i32, color, alpha);
        }
    }
}

impl Shell {
    /// Starts, polls and stops the caption pipeline to match the Accessibility switch and the screen. Called every tick.
    pub(super) fn captions_tick(&mut self, t: f64) {
        let on = self.prefs.get_bool(SWITCH, false);
        let wanted = on && self.screen_on;
        if !on {
            // switched off: the next switch-on may ask again
            self.captions.retry_at = 0.0;
        }
        if wanted && !self.captions.running && t >= self.captions.retry_at {
            match self.service_call(SHELL_APP, SERVICE, "captions_start", "", t) {
                Ok(_) => {
                    self.captions.running = true;
                    self.captions.last_poll = t;
                }
                Err(e) => {
                    self.captions.notice = Some((notice_for(&e).to_string(), t));
                    self.captions.retry_at = f64::INFINITY;
                    self.dirty = true;
                }
            }
        } else if !wanted && self.captions.running {
            let _ = self.service_call(SHELL_APP, SERVICE, "captions_stop", "", t);
            self.captions.running = false;
            self.captions.done.clear();
            self.captions.live.clear();
            self.captions.last_text = None;
            self.dirty = true;
        }
        if self.captions.running && t - self.captions.last_poll >= POLL_MS {
            self.captions.last_poll = t;
            match self.service_call(SHELL_APP, SERVICE, "captions_poll", "", t) {
                Ok(reply) => {
                    let heard: Vec<(String, bool)> = reply
                        .get("transcripts")
                        .and_then(Json::as_array)
                        .unwrap_or(&[])
                        .iter()
                        .filter_map(|r| Some((r.str_field("text")?, matches!(r.get("final"), Some(Json::Bool(true))))))
                        .collect();
                    if !heard.is_empty() {
                        self.captions.apply(&heard, t);
                        self.dirty = true;
                    }
                    if !matches!(reply.get("running"), Some(Json::Bool(true))) {
                        // the pipeline ended on its own: start it again shortly
                        self.captions.running = false;
                        self.captions.retry_at = t + RESTART_MS;
                    }
                }
                Err(_) => {
                    self.captions.running = false;
                    self.captions.retry_at = t + RESTART_MS;
                }
            }
        }
        // the fade needs a frame each tick; the one after it removes the plate
        if self.captions.expire(t) || (self.captions.visible(t) && self.captions.alpha(t) < 1.0) {
            self.dirty = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shell::services::recognizer::{Fetcher, Pipeline, PipelineSpawner, Recognizer};
    use crate::shell::tests::{shell, ui};
    use std::path::Path;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};

    struct Fake {
        lines: Arc<Mutex<Vec<String>>>,
        argvs: Arc<Mutex<Vec<Vec<String>>>>,
        kills: Arc<Mutex<usize>>,
        done: Arc<AtomicBool>,
    }

    struct FakePipeline(Arc<Mutex<Vec<String>>>, Arc<Mutex<usize>>, Arc<AtomicBool>);

    impl PipelineSpawner for Fake {
        fn start(&mut self, argv: &[String]) -> Result<Box<dyn Pipeline>, String> {
            self.argvs.lock().unwrap().push(argv.to_vec());
            Ok(Box::new(FakePipeline(Arc::clone(&self.lines), Arc::clone(&self.kills), Arc::clone(&self.done))))
        }
    }

    impl Pipeline for FakePipeline {
        fn take_lines(&mut self) -> Vec<String> {
            std::mem::take(&mut *self.0.lock().unwrap())
        }
        fn interrupt(&mut self) {}
        fn finished(&mut self) -> bool {
            self.2.load(Ordering::SeqCst)
        }
        fn kill(&mut self) {
            *self.1.lock().unwrap() += 1;
        }
    }

    struct NoFetch;
    impl Fetcher for NoFetch {
        fn fetch(&self, _: &str, _: &Path) -> Result<(), String> {
            Err("no network in tests".into())
        }
    }

    struct Rig {
        sh: Shell,
        lines: Arc<Mutex<Vec<String>>>,
        argvs: Arc<Mutex<Vec<Vec<String>>>>,
        kills: Arc<Mutex<usize>>,
        done: Arc<AtomicBool>,
    }

    /// A shell whose speech service has a scratch data dir (with the model and consent, or not) and a fake pipeline.
    fn rig(tag: &str, model_and_consent: bool) -> Rig {
        let dir = std::env::temp_dir().join(format!("captions-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let speech = dir.join("speech");
        std::fs::create_dir_all(&speech).unwrap();
        if model_and_consent {
            std::fs::write(speech.join("ggml-base.en.bin"), b"model").unwrap();
            std::fs::write(speech.join("consent"), "granted\n").unwrap();
        }
        let (lines, argvs, kills, done) = (Arc::new(Mutex::new(Vec::new())), Arc::new(Mutex::new(Vec::new())), Arc::new(Mutex::new(0)), Arc::new(AtomicBool::new(false)));
        let fake = Fake { lines: Arc::clone(&lines), argvs: Arc::clone(&argvs), kills: Arc::clone(&kills), done: Arc::clone(&done) };
        let mut sh = shell();
        sh.services.retain(|s| s.name() != SERVICE);
        sh.services.push(Box::new(Recognizer::with_parts(dir, Box::new(fake), Arc::new(NoFetch))));
        Rig { sh, lines, argvs, kills, done }
    }

    fn said(text: &str, is_final: bool) -> String {
        format!("/GstPipeline:pipeline0/whisper:whisper0: element message {{text=\"{text}\", is-final=(boolean){is_final}}}")
    }

    /// Turns the Accessibility switch on or off the way Settings does: in the prefs.
    fn switch(sh: &mut Shell, on: bool) {
        sh.prefs.set(SWITCH, if on { "1" } else { "0" }).unwrap();
    }

    #[test]
    fn the_plate_keeps_the_newest_words_and_fades_after_the_silence() {
        let mut c = Captions::default();
        assert!(!c.visible(0.0) && c.current_text().is_empty());
        c.apply(&[("hello".into(), false)], 1000.0);
        assert_eq!(c.current_text(), "hello");
        c.apply(&[("hello there".into(), true), ("how are".into(), false)], 1500.0);
        assert_eq!(c.current_text(), "hello there how are", "a final line is kept and the live one follows it");
        assert_eq!(c.alpha(1500.0 + IDLE_MS - 1.0), 1.0);
        assert!(c.alpha(1500.0 + IDLE_MS + FADE_MS / 2.0) < 1.0);
        assert!(!c.visible(1500.0 + IDLE_MS + FADE_MS + 1.0));
        assert!(c.expire(1500.0 + IDLE_MS + FADE_MS + 1.0));
        assert!(c.current_text().is_empty() && c.next_wake(1e9).is_none());
    }

    #[test]
    fn the_plate_is_drawn_at_the_bottom_centre_in_its_box() {
        let ui = ui();
        let mut c = Captions::default();
        c.apply(&[("the quick brown fox".into(), true)], 0.0);
        let (x, y, w, h) = c.rect(&ui, 1200, 1920);
        assert_eq!(x + w / 2.0, 600.0, "centred");
        assert_eq!(y + h, 1920.0 - ui.pt(BOTTOM_PT), "above the dock");
        let mut buf = vec![0u32; 1200 * 1920];
        c.draw(&mut Surface { w: 1200, h: 1920, stride: 1200, px: &mut buf, y0: 0 }, &ui, 1200, 1920, 100.0);
        let inside = buf[(y + h - 4.0) as usize * 1200 + (x + w / 2.0) as usize];
        assert_ne!(inside, 0, "the plate is painted");
        let outside = buf[(y - 20.0) as usize * 1200 + (x + w / 2.0) as usize];
        assert_eq!(outside, 0, "nothing above the box");
        // a long text wraps and shows only the newest two lines
        let long = "word ".repeat(120);
        c.apply(&[(long.trim().into(), true)], 200.0);
        assert!(c.lines(&ui, w).len() <= LINES);
    }

    #[test]
    fn switching_the_setting_on_starts_the_system_audio_pipeline_and_words_reach_the_plate() {
        {
            let mut r = rig("on", true);
            switch(&mut r.sh, true);
            r.sh.captions_tick(1000.0);
            assert!(r.sh.captions.running());
            let argv = r.argvs.lock().unwrap()[0].clone();
            assert!(argv.iter().any(|a| a == "device=@DEFAULT_MONITOR@" || a.starts_with("stream-properties=props,stream.capture.sink")), "{argv:?}");
            r.lines.lock().unwrap().push(said("good evening", false));
            r.sh.captions_tick(1100.0);
            assert!(r.sh.captions.current_text().is_empty(), "before the poll interval nothing is read");
            r.sh.captions_tick(1000.0 + POLL_MS + 10.0);
            assert_eq!(r.sh.captions.current_text(), "good evening");
            assert!(r.sh.captions.visible(1300.0));
            r.lines.lock().unwrap().push(said("good evening everyone", true));
            r.sh.captions_tick(1600.0);
            assert_eq!(r.sh.captions.current_text(), "good evening everyone");
            assert_eq!(r.sh.captions.next_wake(1600.0), Some(1600.0 + POLL_MS), "the loop wakes to poll");
            // the frame draws the plate and damages its box
            let mut buf = vec![0u32; 1200 * 1920];
            let d = r.sh.draw(&mut Surface { w: 1200, h: 1920, stride: 1200, px: &mut buf, y0: 0 }, 1650.0).clone();
            assert!(!d.is_empty());
            let (x, y, w, h) = r.sh.captions.rect(&r.sh.ui, r.sh.w, r.sh.h);
            assert_ne!(buf[(y + h - 3.0) as usize * 1200 + (x + w / 2.0) as usize], rgba(20, 20, 30, 255), "the plate is over the wallpaper");
        }
    }

    #[test]
    fn switching_it_off_stops_the_pipeline_and_clears_the_plate() {
        {
            let mut r = rig("off", true);
            switch(&mut r.sh, true);
            r.sh.captions_tick(1000.0);
            r.lines.lock().unwrap().push(said("hello", true));
            r.sh.captions_tick(1300.0);
            assert_eq!(r.sh.captions.current_text(), "hello");
            switch(&mut r.sh, false);
            r.sh.captions_tick(1400.0);
            assert!(!r.sh.captions.running());
            assert_eq!(*r.kills.lock().unwrap(), 1);
            assert!(r.sh.captions.current_text().is_empty());
            // it stays off: no new pipeline
            r.sh.captions_tick(9000.0);
            assert_eq!(r.argvs.lock().unwrap().len(), 1);
        }
    }

    #[test]
    fn without_the_model_and_consent_the_plate_says_so_once_and_nothing_is_captured() {
        {
            let mut r = rig("refused", false);
            switch(&mut r.sh, true);
            r.sh.captions_tick(1000.0);
            assert!(!r.sh.captions.running());
            assert!(r.argvs.lock().unwrap().is_empty());
            assert!(r.sh.captions.current_text().contains("Dictation"), "{}", r.sh.captions.current_text());
            assert!(r.sh.captions.visible(1500.0));
            r.sh.captions_tick(60_000.0);
            assert!(!r.sh.captions.visible(60_000.0), "the notice faded");
            assert!(r.argvs.lock().unwrap().is_empty(), "no retry until the switch is turned off and on");
            // turning it off and on asks again
            switch(&mut r.sh, false);
            r.sh.captions_tick(61_000.0);
            switch(&mut r.sh, true);
            r.sh.captions_tick(62_000.0);
            assert!(r.sh.captions.current_text().contains("Dictation"), "refused again, and said so again");
        }
    }

    #[test]
    fn a_dark_screen_captures_nothing_and_a_pipeline_that_ends_is_started_again() {
        {
            let mut r = rig("dark", true);
            switch(&mut r.sh, true);
            r.sh.screen_on = false;
            r.sh.captions_tick(1000.0);
            assert!(!r.sh.captions.running() && r.argvs.lock().unwrap().is_empty());
            r.sh.screen_on = true;
            r.sh.captions_tick(2000.0);
            assert!(r.sh.captions.running());
            r.done.store(true, Ordering::SeqCst);
            r.sh.captions_tick(2400.0);
            assert!(!r.sh.captions.running(), "the pipeline ended");
            r.done.store(false, Ordering::SeqCst);
            r.sh.captions_tick(2400.0 + RESTART_MS + 10.0);
            assert!(r.sh.captions.running());
            assert_eq!(r.argvs.lock().unwrap().len(), 2);
        }
    }

    #[test]
    fn the_setting_is_off_by_default_and_nothing_runs() {
        {
            let mut r = rig("default", true);
            r.sh.captions_tick(1000.0);
            assert!(!r.sh.captions.running() && r.argvs.lock().unwrap().is_empty());
            assert!(!crate::shell::accessibility::AccessibilitySettings::default().live_captions);
        }
    }

    #[test]
    fn notices_name_the_cause() {
        assert!(notice_for("speech recognizer: grant consent in Settings before recognition starts").contains("Dictation"));
        assert!(notice_for("speech recognizer: the speech model is not installed; download it in Settings").contains("Dictation"));
        assert_eq!(notice_for("speech recognizer: gst-launch-1.0 is not installed"), "Live Captions could not start.");
    }
}
