//! `AVPlayerView` and the declarative `VideoPlayer(player:videoOverlay:)` as pure state: the controls style, the chrome that
//! style shows, the layout of that chrome, hit-testing and time labels. `PlayerViewController` owns a view and reports
//! its full-screen stages to a delegate. The view drives a `PlayerHandle` (the app's
//! existing player) and decodes nothing. Nothing is drawn here.
use crate::media_time::MediaTime;
use crate::Rect;

/// Height of the control bar, in points.
pub const BAR_HEIGHT: f32 = 44.0;
/// Side of the square buttons, in points.
pub const BUTTON_SIZE: f32 = 32.0;
/// Gap between chrome elements and the view edge, in points.
pub const PADDING: f32 = 8.0;
/// Width of the time labels, in points.
pub const LABEL_WIDTH: f32 = 52.0;

/// `AVPlayerViewControlsStyle`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ControlsStyle {
    None,
    Inline,
    Floating,
    Minimal,
}

/// The chrome elements a style can show.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Chrome {
    PlayPause,
    Scrubber,
    ElapsedLabel,
    RemainingLabel,
    FullScreenToggle,
}

/// What a point on the view hits.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Hit {
    PlayPause,
    /// Position the scrubber maps to.
    Scrubber(MediaTime),
    FullScreenToggle,
    Video,
}

/// The player the view drives. Position and duration are in seconds.
pub trait PlayerHandle {
    fn is_playing(&self) -> bool;
    fn play(&mut self);
    fn pause(&mut self);
    fn current_seconds(&self) -> f64;
    fn duration_seconds(&self) -> f64;
    fn seek_seconds(&mut self, seconds: f64);
}

/// `AVPlayerView`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlayerView {
    pub controls_style: ControlsStyle,
    pub shows_full_screen_toggle_button: bool,
    pub allows_picture_in_picture_playback: bool,
    pub updates_now_playing_info_center: bool,
    full_screen: bool,
}

impl Default for PlayerView {
    fn default() -> Self {
        PlayerView {
            controls_style: ControlsStyle::Inline,
            shows_full_screen_toggle_button: false,
            allows_picture_in_picture_playback: false,
            updates_now_playing_info_center: true,
            full_screen: false,
        }
    }
}

impl PlayerView {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_full_screen(&self) -> bool {
        self.full_screen
    }

    /// Flips full screen; does nothing while the toggle button is hidden.
    pub fn toggle_full_screen(&mut self) {
        if self.shows_full_screen_toggle_button {
            self.full_screen = !self.full_screen;
        }
    }

    /// The chrome that is visible for the style and flags.
    pub fn visible_chrome(&self) -> Vec<Chrome> {
        let mut out = match self.controls_style {
            ControlsStyle::None => return Vec::new(),
            ControlsStyle::Minimal => vec![Chrome::PlayPause],
            ControlsStyle::Floating => vec![Chrome::PlayPause, Chrome::Scrubber],
            ControlsStyle::Inline => vec![
                Chrome::PlayPause,
                Chrome::ElapsedLabel,
                Chrome::Scrubber,
                Chrome::RemainingLabel,
            ],
        };
        if self.shows_full_screen_toggle_button && self.controls_style != ControlsStyle::Minimal {
            out.push(Chrome::FullScreenToggle);
        }
        out
    }

    /// The control bar: along the bottom for inline, floating above the bottom for floating, centred for minimal.
    fn bar(&self, bounds: Rect) -> Rect {
        match self.controls_style {
            ControlsStyle::Inline => Rect { x: bounds.x, y: bounds.y + bounds.h - BAR_HEIGHT, w: bounds.w, h: BAR_HEIGHT },
            ControlsStyle::Floating => {
                let w = (bounds.w - 8.0 * PADDING).max(0.0);
                Rect { x: bounds.x + (bounds.w - w) / 2.0, y: bounds.y + bounds.h - BAR_HEIGHT - 4.0 * PADDING, w, h: BAR_HEIGHT }
            }
            _ => Rect { x: bounds.x + (bounds.w - BUTTON_SIZE) / 2.0, y: bounds.y + (bounds.h - BUTTON_SIZE) / 2.0, w: BUTTON_SIZE, h: BUTTON_SIZE },
        }
    }

    /// Where each visible element sits inside `bounds`.
    pub fn layout(&self, bounds: Rect) -> Vec<(Chrome, Rect)> {
        let chrome = self.visible_chrome();
        if chrome.is_empty() {
            return Vec::new();
        }
        let bar = self.bar(bounds);
        if self.controls_style == ControlsStyle::Minimal {
            return vec![(Chrome::PlayPause, bar)];
        }
        let width_of = |c: &Chrome, scrub: f32| match c {
            Chrome::Scrubber => scrub,
            Chrome::ElapsedLabel | Chrome::RemainingLabel => LABEL_WIDTH,
            _ => BUTTON_SIZE,
        };
        let fixed: f32 = chrome.iter().map(|c| width_of(c, 0.0)).sum();
        let gaps = PADDING * (chrome.len() as f32 + 1.0);
        let scrub_w = (bar.w - fixed - gaps).max(0.0);
        let y = bar.y + (bar.h - BUTTON_SIZE) / 2.0;
        let mut x = bar.x + PADDING;
        let mut out = Vec::new();
        for c in chrome {
            let w = width_of(&c, scrub_w);
            out.push((c, Rect { x, y, w, h: BUTTON_SIZE }));
            x += w + PADDING;
        }
        out
    }

    /// What the point hits. Scrubber hits carry the time the x position maps to (`duration` in seconds).
    pub fn hit_test(&self, bounds: Rect, x: f32, y: f32, duration: f64) -> Option<Hit> {
        if !bounds.contains(x, y) {
            return None;
        }
        for (c, r) in self.layout(bounds) {
            if !r.contains(x, y) {
                continue;
            }
            return match c {
                Chrome::PlayPause => Some(Hit::PlayPause),
                Chrome::FullScreenToggle => Some(Hit::FullScreenToggle),
                Chrome::Scrubber => Some(Hit::Scrubber(scrub_time(r, x, duration))),
                _ => Some(Hit::Video),
            };
        }
        Some(Hit::Video)
    }

    /// Applies a tap to the player and the view.
    pub fn tap(&mut self, bounds: Rect, x: f32, y: f32, player: &mut dyn PlayerHandle) -> Option<Hit> {
        let hit = self.hit_test(bounds, x, y, player.duration_seconds())?;
        match hit {
            Hit::PlayPause => toggle_play(player),
            Hit::Scrubber(t) => player.seek_seconds(t.seconds()),
            Hit::FullScreenToggle => self.toggle_full_screen(),
            Hit::Video => {}
        }
        Some(hit)
    }
}

/// Plays a paused player and pauses a playing one.
pub fn toggle_play(player: &mut dyn PlayerHandle) {
    if player.is_playing() {
        player.pause();
    } else {
        player.play();
    }
}

/// The time an x position on the scrubber track maps to, clamped to the track.
pub fn scrub_time(track: Rect, x: f32, duration: f64) -> MediaTime {
    if track.w <= 0.0 || !duration.is_finite() || duration <= 0.0 {
        return MediaTime::zero();
    }
    let f = ((x - track.x) / track.w).clamp(0.0, 1.0) as f64;
    MediaTime::from_seconds(f * duration, 600)
}

/// `m:ss`, or `h:mm:ss` from one hour. Negative or non-finite values show as zero.
pub fn format_time(seconds: f64) -> String {
    let s = if seconds.is_finite() && seconds > 0.0 { seconds as u64 } else { 0 };
    let (h, m, sec) = (s / 3600, (s / 60) % 60, s % 60);
    if h > 0 {
        format!("{h}:{m:02}:{sec:02}")
    } else {
        format!("{m}:{sec:02}")
    }
}

/// The remaining-time label: a minus sign and the time left.
pub fn format_remaining(current: f64, duration: f64) -> String {
    format!("-{}", format_time((duration - current).max(0.0)))
}

/// Declarative `VideoPlayer(player:videoOverlay:)`: the player view with inline controls and an overlay drawn over the video.
#[derive(Clone, Debug, PartialEq)]
pub struct VideoPlayer<O> {
    pub view: PlayerView,
    pub video_overlay: Option<O>,
}

impl<O> VideoPlayer<O> {
    pub fn new(video_overlay: Option<O>) -> Self {
        VideoPlayer { view: PlayerView::new(), video_overlay }
    }
}

/// The delegate of a `PlayerViewController`. Full-screen changes arrive as a will stage, the change, then a did stage.
pub trait PlayerViewDelegate {
    fn will_begin_full_screen(&mut self) {}
    fn did_begin_full_screen(&mut self) {}
    fn will_end_full_screen(&mut self) {}
    fn did_end_full_screen(&mut self) {}
}

/// The player controller: owns a `PlayerView` and reports its full-screen changes to a delegate.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PlayerViewController {
    pub view: PlayerView,
}

impl PlayerViewController {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_full_screen(&self) -> bool {
        self.view.is_full_screen()
    }

    /// Moves to full screen or back, reporting the stages around the change. Returns false and reports nothing when the
    /// view is already in the requested state.
    pub fn set_full_screen(&mut self, on: bool, delegate: &mut dyn PlayerViewDelegate) -> bool {
        if self.view.full_screen == on {
            return false;
        }
        if on {
            delegate.will_begin_full_screen();
        } else {
            delegate.will_end_full_screen();
        }
        self.view.full_screen = on;
        if on {
            delegate.did_begin_full_screen();
        } else {
            delegate.did_end_full_screen();
        }
        true
    }

    /// Applies a tap. The full-screen button goes through `set_full_screen`; every other hit is handled by the view.
    pub fn tap(
        &mut self,
        bounds: Rect,
        x: f32,
        y: f32,
        player: &mut dyn PlayerHandle,
        delegate: &mut dyn PlayerViewDelegate,
    ) -> Option<Hit> {
        match self.view.hit_test(bounds, x, y, player.duration_seconds())? {
            Hit::FullScreenToggle => {
                let on = !self.is_full_screen();
                self.set_full_screen(on, delegate);
                Some(Hit::FullScreenToggle)
            }
            _ => self.view.tap(bounds, x, y, player),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fake {
        playing: bool,
        pos: f64,
    }
    impl PlayerHandle for Fake {
        fn is_playing(&self) -> bool { self.playing }
        fn play(&mut self) { self.playing = true; }
        fn pause(&mut self) { self.playing = false; }
        fn current_seconds(&self) -> f64 { self.pos }
        fn duration_seconds(&self) -> f64 { 100.0 }
        fn seek_seconds(&mut self, s: f64) { self.pos = s; }
    }

    const B: Rect = Rect { x: 0.0, y: 0.0, w: 600.0, h: 400.0 };

    fn view(style: ControlsStyle) -> PlayerView {
        PlayerView { controls_style: style, ..PlayerView::new() }
    }

    fn find(v: &PlayerView, c: Chrome) -> Rect {
        v.layout(B).into_iter().find(|(k, _)| *k == c).unwrap().1
    }

    #[test]
    fn style_maps_to_chrome() {
        assert!(view(ControlsStyle::None).visible_chrome().is_empty());
        assert_eq!(view(ControlsStyle::Minimal).visible_chrome(), vec![Chrome::PlayPause]);
        assert_eq!(view(ControlsStyle::Floating).visible_chrome(), vec![Chrome::PlayPause, Chrome::Scrubber]);
        assert_eq!(view(ControlsStyle::Inline).visible_chrome().len(), 4);
    }

    #[test]
    fn full_screen_toggle_follows_flag() {
        let mut v = view(ControlsStyle::Inline);
        assert!(!v.visible_chrome().contains(&Chrome::FullScreenToggle));
        v.toggle_full_screen();
        assert!(!v.is_full_screen());
        v.shows_full_screen_toggle_button = true;
        assert!(v.visible_chrome().contains(&Chrome::FullScreenToggle));
        v.toggle_full_screen();
        assert!(v.is_full_screen());
    }

    #[test]
    fn scrubber_hit_converts_x_to_seconds() {
        let v = view(ControlsStyle::Inline);
        let r = find(&v, Chrome::Scrubber);
        match v.hit_test(B, r.x + r.w / 2.0, r.y + r.h / 2.0, 100.0) {
            Some(Hit::Scrubber(t)) => assert!((t.seconds() - 50.0).abs() < 0.01),
            other => panic!("{other:?}"),
        }
        assert_eq!(scrub_time(r, r.x - 50.0, 100.0).seconds(), 0.0);
        assert_eq!(scrub_time(r, r.x + r.w + 50.0, 100.0).seconds(), 100.0);
    }

    #[test]
    fn tap_toggles_play_and_seeks() {
        let mut v = view(ControlsStyle::Inline);
        let mut p = Fake { playing: false, pos: 0.0 };
        let r = find(&v, Chrome::PlayPause);
        assert_eq!(v.tap(B, r.x + 1.0, r.y + 1.0, &mut p), Some(Hit::PlayPause));
        assert!(p.is_playing());
        v.tap(B, r.x + 1.0, r.y + 1.0, &mut p);
        assert!(!p.is_playing());
        let s = find(&v, Chrome::Scrubber);
        v.tap(B, s.x + s.w / 4.0, s.y + 1.0, &mut p);
        assert!((p.current_seconds() - 25.0).abs() < 0.01);
    }

    #[test]
    fn video_area_and_outside() {
        let v = view(ControlsStyle::Inline);
        assert_eq!(v.hit_test(B, 10.0, 10.0, 100.0), Some(Hit::Video));
        assert_eq!(v.hit_test(B, -1.0, 10.0, 100.0), None);
        assert_eq!(view(ControlsStyle::None).hit_test(B, 300.0, 390.0, 100.0), Some(Hit::Video));
    }

    #[test]
    fn time_labels() {
        assert_eq!(format_time(0.0), "0:00");
        assert_eq!(format_time(65.9), "1:05");
        assert_eq!(format_time(3600.0), "1:00:00");
        assert_eq!(format_time(3725.0), "1:02:05");
        assert_eq!(format_time(-4.0), "0:00");
        assert_eq!(format_remaining(20.0, 80.0), "-1:00");
    }

    #[test]
    fn defaults_match_reference() {
        let v = PlayerView::new();
        assert_eq!(v.controls_style, ControlsStyle::Inline);
        assert!(!v.shows_full_screen_toggle_button);
        assert!(v.updates_now_playing_info_center);
        let vp: VideoPlayer<u8> = VideoPlayer::new(Some(1));
        assert_eq!(vp.video_overlay, Some(1));
    }
}
