//! Announcements on screen. An app posts text with its SDK `announce` call (the announcement-requested accessibility
//! notification); the screen reader speaks it through the AT-SPI bridge, and this banner shows the same text at the top of
//! the screen for a few seconds, so a sighted user sees what is spoken too.
//!
//! The queue is `bifrost_ui::ax::Announcements`: an assertive announcement (`Priority::High`) is shown before waiting polite
//! ones, the same text twice in a row is shown once, and blank text is refused. One announcement is on screen at a time.

use crate::gfx::Surface;
use crate::shell::ui::{Weight, MAC_CALLOUT_PT, WHITE};
use super::AccessibilitySettings;
use crate::shell::Shell;
use bifrost_proto::AnnouncePriority;
use bifrost_render::gfx::pack;
use bifrost_ui::ax::{Announcements, Politeness};

/// How long one announcement stays on screen, in ms.
pub const SHOW_MS: f64 = 4000.0;
/// The banner's top edge, in points: below the menu bar.
const TOP_PT: f32 = 56.0;
/// Inner padding of the banner, in points.
const PAD_X_PT: f32 = 16.0;
const PAD_Y_PT: f32 = 8.0;
/// The banner is a dark pill whatever the appearance, so its white text always reads.
const FILL: u32 = pack(28, 28, 30, 235);

#[derive(Debug, Default)]
pub struct Banner {
    queue: Announcements,
    /// the text on screen and the time (ms) it leaves
    current: Option<(String, f64)>,
    /// the last frame drew the banner, so the frame that clears it repaints the area
    drawn: bool,
    /// the screen reader was on at the last tick (its switch going off clears the banner)
    reader_was_on: bool,
}

impl Banner {
    /// Queues `text`. Returns false when it is blank or repeats the text queued last.
    pub fn post(&mut self, text: &str, priority: AnnouncePriority) -> bool {
        let politeness = if priority == AnnouncePriority::High { Politeness::Assertive } else { Politeness::Polite };
        self.queue.post(text, politeness)
    }

    /// Takes the next announcement once the one on screen has been up for `SHOW_MS`. Returns true when a new text
    /// came on screen this tick (the screen reader speaks it then).
    pub fn tick(&mut self, t: f64) -> bool {
        if self.current.as_ref().is_some_and(|(_, until)| t >= *until) {
            self.current = None;
        }
        if self.current.is_none() {
            if let Some(a) = self.queue.take_next() {
                self.current = Some((a.text, t + SHOW_MS));
                return true;
            }
        }
        false
    }

    /// The text on screen now.
    pub fn shown(&self) -> Option<&str> {
        self.current.as_ref().map(|(s, _)| s.as_str())
    }

    /// Whether the next frame must repaint the banner's area: it shows something, or the last frame did.
    pub fn busy(&self) -> bool {
        self.current.is_some() || self.drawn
    }

    /// Drops the text on screen and everything queued (the screen reader was switched off: its captions go with it).
    pub fn clear(&mut self) {
        self.current = None;
        while self.queue.take_next().is_some() {}
    }
}

impl Shell {
    /// The Screen Reader's speech settings: the open Settings window's values, else the saved ones.
    pub(in crate::shell) fn speech_config(&self) -> super::speech::Config {
        self.settings.as_ref().map_or_else(|| AccessibilitySettings::load(&self.prefs), |st| st.accessibility().clone()).speech_config()
    }

    /// Moves the announcements the apps posted since the last tick into the banner queue, then advances the banner.
    pub(in crate::shell) fn announce_tick(&mut self, t: f64) {
        for host in self.ext.iter_mut().flatten() {
            for (text, priority) in std::mem::take(&mut host.announce_banner) {
                self.announce.post(&text, priority);
            }
        }
        let reader_on = self.screen_reader_on();
        // the screen reader going off takes its caption (and what it still had queued) off the screen at once
        if std::mem::replace(&mut self.announce.reader_was_on, reader_on) && !reader_on {
            self.announce.clear();
            self.dirty = true;
        }
        let started = self.announce.tick(t);
        // Spoken only while the screen reader is on; the engine is silent when none is installed.
        if reader_on {
            if started {
                if let Some(text) = self.announce.shown().map(str::to_string) {
                    // rate, pitch and voice follow Settings > Accessibility > Screen Reader
                    let config = self.speech_config();
                    self.speech.configure(config);
                    self.speech.say(&text);
                }
            }
        } else {
            self.speech.stop();
        }
    }

    /// Whether the frame must repaint everything under the banner.
    pub(in crate::shell) fn announce_damage(&self) -> bool {
        self.announce.busy()
    }

    /// Draws the banner at the top centre of the screen, if an announcement is on screen.
    pub(in crate::shell) fn draw_announce(&mut self, s: &mut Surface) {
        let Some(text) = self.announce.shown().map(str::to_string) else {
            self.announce.drawn = false;
            self.a11y_layers[0] = None;
            return;
        };
        let img = self.ui.text(&text, MAC_CALLOUT_PT, Weight::Regular, WHITE);
        let (pad_x, pad_y) = (self.ui.pt(PAD_X_PT), self.ui.pt(PAD_Y_PT));
        let w = img.w as f32 + 2.0 * pad_x;
        let h = img.h as f32 + 2.0 * pad_y;
        let x = ((self.w as f32 - w) / 2.0).round();
        let y = self.ui.pt(TOP_PT);
        // on the desktop the caption is a surface of its own above the windows and the menu bar
        if self.overlays_on_desk() && !self.is_locked() {
            let (iw, ih) = (w.ceil().max(1.0) as usize, h.ceil().max(1.0) as usize);
            let mut image = crate::gfx::Image::new(iw, ih);
            let mut ls = Surface { w: iw, h: ih, stride: iw, px: &mut image.px, y0: 0 };
            ls.fill_rrect(0.0, 0.0, w, h, h / 2.0, FILL);
            ls.blit_colored(&img, pad_x as i32, pad_y as i32, WHITE, 255);
            self.a11y_layers[0] = Some(crate::shell::capture_flow::Layer { x: x as i32, y: y as i32, image, input: Some([0, 0, 0, 0]) });
        } else {
            self.a11y_layers[0] = None;
            s.fill_rrect(x, y, w, h, h / 2.0, FILL);
            s.blit_colored(&img, (x + pad_x) as i32, (y + pad_y) as i32, WHITE, 255);
        }
        self.announce.drawn = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clear_takes_the_caption_and_the_queue_off_the_screen() {
        let mut b = Banner::default();
        b.post("menu, system", AnnouncePriority::High);
        b.post("menu, Settings", AnnouncePriority::Medium);
        b.tick(0.0);
        assert_eq!(b.shown(), Some("menu, system"));
        b.clear();
        assert_eq!(b.shown(), None, "the caption goes at once");
        b.tick(SHOW_MS);
        assert_eq!(b.shown(), None, "and nothing queued follows it");
    }

    #[test]
    fn blank_and_repeated_announcements_are_refused() {
        let mut b = Banner::default();
        assert!(!b.post("   ", AnnouncePriority::Low), "blank text is refused");
        assert!(b.post("Saved", AnnouncePriority::Low));
        assert!(!b.post("Saved", AnnouncePriority::Low), "the same text as the last one is dropped");
        assert!(b.post("Lost", AnnouncePriority::High));
    }

    #[test]
    fn an_assertive_announcement_shows_first_and_each_stays_for_show_ms() {
        let mut b = Banner::default();
        b.post("Saved", AnnouncePriority::Medium);
        b.post("Connection lost", AnnouncePriority::High);
        b.tick(0.0);
        assert_eq!(b.shown(), Some("Connection lost"));
        b.tick(SHOW_MS - 1.0);
        assert_eq!(b.shown(), Some("Connection lost"), "still up just before the time is over");
        b.tick(SHOW_MS);
        assert_eq!(b.shown(), Some("Saved"), "the polite one follows when the first leaves");
        b.tick(2.0 * SHOW_MS);
        assert_eq!(b.shown(), None, "nothing waits any more");
    }

    #[test]
    fn the_frame_that_clears_the_banner_still_repaints_its_area() {
        let mut b = Banner::default();
        b.post("Done", AnnouncePriority::Low);
        b.tick(0.0);
        assert!(b.busy());
        b.drawn = true;
        b.tick(SHOW_MS);
        assert_eq!(b.shown(), None);
        assert!(b.busy(), "drawn last frame: this frame repaints the clearing");
        b.drawn = false;
        assert!(!b.busy(), "then the shell goes back to quiet frames");
    }
}
