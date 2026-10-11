//! Play and pause on a notification card: the button the card shows for a media notification, where it sits, its
//! tint, and whether it shows play or pause. Playback stays with the app. A tap asks the app for the opposite of
//! the current state, the app starts or stops its playback and calls `media_play` or `media_pause`, and the glyph
//! follows that state. Nothing here plays media.

use crate::Rect;

/// The glyph on the button.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Glyph {
    Play,
    Pause,
}

/// What a tap on the button asks the app to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    Play,
    Pause,
}

/// The play and pause button of one notification card (`mediaPlayPauseButtonFrame`, `...TintColor`, `mediaPlay()`,
/// `mediaPause()`). The frame is in card points.
#[derive(Clone, Debug, PartialEq)]
pub struct MediaControl {
    frame: Rect,
    tint: Option<[u8; 3]>,
    playing: bool,
    shown: bool,
}

impl MediaControl {
    /// A hidden control at `frame`, not yet playing.
    pub fn new(frame: Rect) -> Self {
        MediaControl { frame, tint: None, playing: false, shown: false }
    }

    /// Shows the button (`mediaPlayPauseButtonType` set to a visible type).
    pub fn show(&mut self) {
        self.shown = true;
    }

    pub fn hide(&mut self) {
        self.shown = false;
    }

    pub fn is_shown(&self) -> bool {
        self.shown
    }

    /// `mediaPlayPauseButtonTintColor`, as 8-bit RGB. `None` keeps the card's default tint.
    pub fn set_tint(&mut self, rgb: Option<[u8; 3]>) {
        self.tint = rgb;
    }

    pub fn tint(&self) -> Option<[u8; 3]> {
        self.tint
    }

    /// `mediaPlay()`: the app has started playback. The glyph becomes a pause.
    pub fn media_play(&mut self) {
        self.playing = true;
    }

    /// `mediaPause()`: the app has stopped playback. The glyph becomes a play.
    pub fn media_pause(&mut self) {
        self.playing = false;
    }

    pub fn is_playing(&self) -> bool {
        self.playing
    }

    /// The glyph shown now, or `None` while the button is hidden.
    pub fn glyph(&self) -> Option<Glyph> {
        if !self.shown {
            return None;
        }
        Some(if self.playing { Glyph::Pause } else { Glyph::Play })
    }

    /// The command a tap at (`x`, `y`) in card points asks for: the opposite of the current state. `None` when the
    /// button is hidden or the tap misses it. The state does not change until the app calls `media_play` or `media_pause`.
    pub fn tap(&self, x: f32, y: f32) -> Option<Command> {
        if !self.shown || !self.frame.contains(x, y) {
            return None;
        }
        Some(if self.playing { Command::Pause } else { Command::Play })
    }

    /// The frame moved inside `card`: the size is kept unless it is larger than the card, and the origin is pulled
    /// back so the whole button lies inside the card.
    pub fn fitted_frame(&self, card: Rect) -> Rect {
        let w = self.frame.w.min(card.w).max(0.0);
        let h = self.frame.h.min(card.h).max(0.0);
        let x = self.frame.x.max(card.x).min((card.right() - w).max(card.x));
        let y = self.frame.y.max(card.y).min((card.bottom() - h).max(card.y));
        Rect { x, y, w, h }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn button() -> MediaControl {
        MediaControl::new(Rect { x: 300.0, y: 20.0, w: 28.0, h: 28.0 })
    }

    #[test]
    fn hidden_control_has_no_glyph_and_takes_no_taps() {
        let m = button();
        assert!(!m.is_shown());
        assert_eq!(m.glyph(), None);
        assert_eq!(m.tap(310.0, 30.0), None);
    }

    #[test]
    fn tap_asks_for_the_opposite_of_the_current_state() {
        let mut m = button();
        m.show();
        assert_eq!(m.glyph(), Some(Glyph::Play));
        assert_eq!(m.tap(310.0, 30.0), Some(Command::Play));
        m.media_play();
        assert_eq!(m.glyph(), Some(Glyph::Pause));
        assert_eq!(m.tap(310.0, 30.0), Some(Command::Pause));
    }

    #[test]
    fn a_tap_outside_the_frame_is_ignored() {
        let mut m = button();
        m.show();
        assert_eq!(m.tap(299.9, 30.0), None);
        assert_eq!(m.tap(328.0, 30.0), None);
        assert_eq!(m.tap(310.0, 48.0), None);
    }

    #[test]
    fn state_changes_only_through_play_and_pause_calls() {
        let mut m = button();
        m.show();
        // a tap alone does not flip the glyph; the app must confirm it
        let _ = m.tap(310.0, 30.0);
        assert!(!m.is_playing());
        m.media_play();
        m.media_pause();
        assert_eq!(m.glyph(), Some(Glyph::Play));
    }

    #[test]
    fn tint_is_stored_and_can_be_cleared() {
        let mut m = button();
        assert_eq!(m.tint(), None);
        m.set_tint(Some([10, 132, 255]));
        assert_eq!(m.tint(), Some([10, 132, 255]));
        m.set_tint(None);
        assert_eq!(m.tint(), None);
    }

    #[test]
    fn fitted_frame_stays_inside_the_card() {
        let card = Rect { x: 0.0, y: 0.0, w: 420.0, h: 80.0 };
        // past the right and bottom edges: pulled back inside
        let m = MediaControl::new(Rect { x: 410.0, y: 70.0, w: 28.0, h: 28.0 });
        assert_eq!(m.fitted_frame(card), Rect { x: 392.0, y: 52.0, w: 28.0, h: 28.0 });
        // a frame already inside is unchanged
        let inside = MediaControl::new(Rect { x: 20.0, y: 20.0, w: 28.0, h: 28.0 });
        assert_eq!(inside.fitted_frame(card), Rect { x: 20.0, y: 20.0, w: 28.0, h: 28.0 });
        // larger than the card: shrunk to the card
        let big = MediaControl::new(Rect { x: 0.0, y: 0.0, w: 500.0, h: 100.0 });
        assert_eq!(big.fitted_frame(card), Rect { x: 0.0, y: 0.0, w: 420.0, h: 80.0 });
    }
}
