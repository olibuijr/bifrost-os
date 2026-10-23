//! The tablet desktop's touch gestures: what each decision of `touch_gestures` does on the desk.
//!
//! The decisions come from `SystemGestures::feed_desk` in the shell's contact path (`bin/shell.rs`). Here they run
//! only on the tablet desktop (`desk_active`, the tablet form factor, unlocked): a phone or any other shell is unchanged.

use super::touch_gestures::DeskGesture;
use super::{AppId, Effect, Screen, Shell};

impl Shell {
    /// Runs one decision of the system gestures. Nothing happens off the tablet desktop.
    pub fn desk_gesture(&mut self, g: DeskGesture, t: f64) -> Vec<Effect> {
        if !(self.desk_active() && self.form.is_tablet() && !self.is_locked()) {
            return vec![];
        }
        match g {
            DeskGesture::ControlCenter => self.open_control_center(t),
            DeskGesture::NotificationCenter => self.open_notification_center(t),
            DeskGesture::Overview => {
                if !self.overview_active() {
                    self.overview_toggle(t);
                }
            }
            DeskGesture::RevealDock => self.desk_reveal_dock(t),
            DeskGesture::ShowDesktop => self.desk_show_desktop(),
            // the window cycle exists in the desktop build (the tablet desktop is one); a build without it ignores the gesture
            DeskGesture::CycleWindow => {
                #[cfg(any(test, feature = "desktop-wayland"))]
                self.desk_cycle_window(false);
            }
            DeskGesture::Back => self.desk_back(t),
            DeskGesture::Pinch { scale, focal_px } => self.desk_pinch(scale, focal_px),
            DeskGesture::PinchEnd => self.desk_pinch_end(),
            DeskGesture::Taken => self.gesture_take_touch(),
        }
        self.dirty = true;
        vec![]
    }

    /// Whether the four-finger swipes are on (Settings > Gestures > Four & Five Finger Swipe).
    pub fn four_finger_swipes(&self) -> bool {
        self.options().four_finger
    }

    /// The in-app back step of the app in front (the same steps as Escape): Settings, Files and Photos. Other apps
    /// have none, and nothing else steps back.
    fn desk_back(&mut self, t: f64) {
        match self.screen {
            Screen::App(AppId::Settings) => {
                self.settings.as_mut().is_some_and(|s| s.back_page(t));
            }
            Screen::App(AppId::Files) => {
                self.files.as_mut().is_some_and(|f| f.hw_back(t));
            }
            Screen::App(AppId::Photos) => {
                if let Some(p) = self.photos.as_mut() {
                    p.hw_back(&self.ui, t);
                }
            }
            _ => {}
        }
    }

    /// A system gesture took the touches in progress: the press on the desk ends without its action, a long press
    /// ends, and a menu that the same touch opened closes. The rest of that touch belongs to the gesture (`event`).
    fn gesture_take_touch(&mut self) {
        self.desk_cancel_press();
        self.desk_hold = None;
        self.desktop_menu = None;
        self.desktop_context = None;
        #[cfg(feature = "desktop-wayland")]
        {
            self.desktop_menu_row = None;
        }
        self.touch_taken = true;
    }

    /// One pinch step. Photos zooms its open picture about the midpoint of the fingers; the recognizer's scale is
    /// cumulative, so the step is this scale over the last one.
    fn desk_pinch(&mut self, scale: f32, focal_px: (f32, f32)) {
        let step = match self.pinch_last {
            Some(last) if last > 0.0 => scale / last,
            _ => scale,
        };
        self.pinch_last = Some(scale);
        if self.screen != Screen::App(AppId::Photos) {
            return;
        }
        let m = self.desk_metrics();
        let Some(d) = self.desk.as_ref() else { return };
        let Some(i) = d.index_of(AppId::Photos) else { return };
        let body = d.wins[i].body(&m);
        let focal = (focal_px.0 - body.x as f32, focal_px.1 - body.y as f32);
        if let Some(p) = self.photos.as_mut() {
            p.pinch(step, focal);
        }
    }

    /// The pinch ended: the next one starts from scale 1 again, and Photos settles its zoom.
    fn desk_pinch_end(&mut self) {
        self.pinch_last = None;
        if self.screen == Screen::App(AppId::Photos) {
            if let Some(p) = self.photos.as_mut() {
                p.end_pinch();
            }
        }
    }
}
