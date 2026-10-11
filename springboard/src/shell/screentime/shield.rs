//! The Time Limit shield: a full-screen cover shown over an app whose daily
//! limit is reached (or that Downtime does not allow). It offers OK (back to
//! the Home Screen) and "Ignore Limit" for 15 minutes or the rest of the
//! day, each behind the Screen Time Passcode when one is set.
//!
//! It only ever takes touches. Hardware keys (power, volume, the screenshot
//! chord) never reach it, Settings is never shielded, and the shell drops
//! the shield the moment the app leaves the screen, so system recovery
//! cannot be blocked by it.

use super::super::icons::BLUE;
use super::super::passcode::{Lock, LockOut};
use super::super::ui::{centred, Ui, Weight, SECONDARY, WHITE};
use super::super::{Event, apps};
use super::config::{Ignore, Reason};
use crate::gfx::{pack, rgba, Surface};
use std::path::Path;

const BG: u32 = pack(18, 18, 20, 255);
const PLATE: u32 = pack(44, 44, 46, 255);
const BUTTON_W_PT: f32 = 320.0;
const BUTTON_H_PT: f32 = 50.0;
const BUTTON_GAP_PT: f32 = 12.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Btn {
    Fifteen,
    Today,
    Ok,
}

/// What a touch on the shield asks the shell to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Out {
    Open,
    /// close the app and go Home
    Home,
    /// the limit may be ignored this way (the passcode, if any, was entered)
    Ignore(Ignore),
}

pub struct Shield {
    pub app: super::super::AppId,
    pub reason: Reason,
    w: i32,
    h: i32,
    pressed: Option<Btn>,
    /// the passcode prompt, and what it will grant
    lock: Option<(Lock, Ignore)>,
}

impl Shield {
    pub fn new(app: super::super::AppId, reason: Reason, w: i32, h: i32) -> Self {
        Shield { app, reason, w, h, pressed: None, lock: None }
    }

    fn name(&self) -> &'static str {
        apps().iter().find(|a| a.id == self.app).map_or("This app", |a| a.name)
    }

    pub fn title(&self) -> &'static str {
        match self.reason {
            Reason::Limit => "Time Limit",
            Reason::Downtime => "Downtime",
        }
    }

    pub fn message(&self) -> String {
        match self.reason {
            Reason::Limit => format!("You've reached your daily limit for {}.", self.name()),
            Reason::Downtime => format!("{} is not available during Downtime.", self.name()),
        }
    }

    fn rect(&self, ui: &Ui, b: Btn) -> (f32, f32, f32, f32) {
        let (bw, bh, gap) = (ui.pt(BUTTON_W_PT).min(self.w as f32 - ui.pt(40.0)), ui.pt(BUTTON_H_PT), ui.pt(BUTTON_GAP_PT));
        let idx = match b {
            Btn::Fifteen => 0.0,
            Btn::Today => 1.0,
            Btn::Ok => 2.0,
        };
        let top = self.h as f32 * 0.58;
        ((self.w as f32 - bw) / 2.0, top + idx * (bh + gap), bw, bh)
    }

    fn hit(&self, ui: &Ui, x: f32, y: f32) -> Option<Btn> {
        [Btn::Fifteen, Btn::Today, Btn::Ok].into_iter().find(|&b| {
            let (rx, ry, rw, rh) = self.rect(ui, b);
            x >= rx && x < rx + rw && y >= ry && y < ry + rh
        })
    }

    /// Centre of a button's ignore/OK area, for tests: 0 = 15 min, 1 = today, 2 = OK.
    pub fn button_centre(&self, ui: &Ui, which: usize) -> (f32, f32) {
        let b = [Btn::Fifteen, Btn::Today, Btn::Ok][which];
        let (x, y, w, h) = self.rect(ui, b);
        (x + w / 2.0, y + h / 2.0)
    }

    /// Refreshes the passcode prompt's lockout text; true when a frame is due.
    pub fn tick(&mut self, unix: i64, t: f64) -> bool {
        self.lock.as_mut().is_some_and(|(l, _)| l.refresh(unix) | l.animating(t))
    }

    /// Whether the shield needs frames of its own (prompt animation or lockout countdown).
    pub fn next_wake(&self, unix: i64, t: f64) -> Option<f64> {
        let (l, _) = self.lock.as_ref()?;
        if l.animating(t) {
            Some(t)
        } else if l.locked(unix) {
            Some(t + 1000.0)
        } else {
            None
        }
    }

    /// Handles a touch; `pass` is the Screen Time passcode file.
    pub fn touch(&mut self, ui: &Ui, ev: Event, t: f64, pass: &Path) -> Out {
        if let Some((lock, how)) = &mut self.lock {
            return match lock.touch(ui, ev, t) {
                LockOut::Unlocked => Out::Ignore(*how),
                LockOut::Cancelled => {
                    self.lock = None;
                    Out::Open
                }
                LockOut::Open => Out::Open,
            };
        }
        match ev {
            Event::Down(x, y) => self.pressed = self.hit(ui, x, y),
            Event::Up(x, y) => {
                let was = self.pressed.take();
                if was.is_some() && was == self.hit(ui, x, y) {
                    return self.press(was.expect("pressed"), pass);
                }
            }
            _ => {}
        }
        Out::Open
    }

    fn press(&mut self, b: Btn, pass: &Path) -> Out {
        let how = match b {
            Btn::Ok => return Out::Home,
            Btn::Fifteen => Ignore::Fifteen,
            Btn::Today => Ignore::Today,
        };
        if Lock::required(pass) {
            self.lock = Some((Lock::new(pass, self.w, self.h), how));
            Out::Open
        } else {
            Out::Ignore(how)
        }
    }

    pub fn draw(&self, s: &mut Surface, ui: &Ui, t: f64) {
        s.fill_rrect(0.0, 0.0, self.w as f32, self.h as f32, 0.0, BG);
        let cx = self.w / 2;
        let title = ui.text(self.title(), 30.0, Weight::Semibold, WHITE);
        centred(s, &title, cx, (self.h as f32 * 0.30) as i32);
        let mut y = self.h as f32 * 0.30 + title.h as f32 + ui.pt(12.0);
        for line in ui.wrap(&self.message(), 17.0, Weight::Regular, ui.pt(BUTTON_W_PT).min(self.w as f32 - ui.pt(40.0))) {
            let img = ui.text(&line, 17.0, Weight::Regular, SECONDARY);
            centred(s, &img, cx, y as i32);
            y += img.h as f32 + ui.pt(4.0);
        }
        let radius = ui.pt(5.0).min(6.0);
        for (b, label, color) in [(Btn::Fifteen, "Ignore Limit for 15 min", BLUE), (Btn::Today, "Ignore Limit For Today", BLUE), (Btn::Ok, "OK", WHITE)] {
            let (x, by, w, h) = self.rect(ui, b);
            s.fill_rrect(x, by, w, h, radius, if self.pressed == Some(b) { rgba(70, 70, 74, 255) } else { PLATE });
            let img = ui.text(label, 17.0, Weight::Semibold, color);
            centred(s, &img, (x + w / 2.0) as i32, (by + (h - img.h as f32) / 2.0) as i32);
        }
        if let Some((lock, _)) = &self.lock {
            lock.draw(s, ui, t);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shell::AppId;

    fn tap(sh: &mut Shield, ui: &Ui, which: usize, pass: &Path) -> Out {
        let (x, y) = sh.button_centre(ui, which);
        sh.touch(ui, Event::Down(x, y), 0.0, pass);
        sh.touch(ui, Event::Up(x, y), 1.0, pass)
    }

    #[test]
    fn text_names_the_app_and_the_reason() {
        let a = Shield::new(AppId::Files, Reason::Limit, 1200, 800);
        assert_eq!((a.title(), a.message().as_str()), ("Time Limit", "You've reached your daily limit for Files."));
        let b = Shield::new(AppId::Photos, Reason::Downtime, 1200, 800);
        assert_eq!((b.title(), b.message().as_str()), ("Downtime", "Photos is not available during Downtime."));
    }

    #[test]
    fn ok_goes_home_and_ignore_needs_no_passcode_when_none_is_set() {
        let ui = crate::shell::tests::ui();
        let none = Path::new("/nonexistent/screentime_passcode.ini");
        let mut sh = Shield::new(AppId::Files, Reason::Limit, 1200, 800);
        assert_eq!(tap(&mut sh, &ui, 2, none), Out::Home);
        assert_eq!(tap(&mut sh, &ui, 0, none), Out::Ignore(Ignore::Fifteen));
        assert_eq!(tap(&mut sh, &ui, 1, none), Out::Ignore(Ignore::Today));
    }

    #[test]
    fn ignore_asks_for_the_passcode_when_one_is_set() {
        let ui = crate::shell::tests::ui();
        let dir = std::path::PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/target/test-shield")).join(std::process::id().to_string());
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("screentime_passcode.ini");
        let mut store = crate::shell::passcode::Store::load(&path);
        store.set("1234").unwrap();
        let mut sh = Shield::new(AppId::Files, Reason::Limit, 1200, 800);
        assert_eq!(tap(&mut sh, &ui, 1, &path), Out::Open, "the keypad is up, nothing granted yet");
        assert!(sh.lock.is_some());
        let mut buf = vec![0u32; 1200 * 800];
        sh.draw(&mut Surface { w: 1200, h: 800, stride: 1200, px: &mut buf, y0: 0 }, &ui, 0.0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn draw_paints_the_cover_and_buttons() {
        let ui = crate::shell::tests::ui();
        let sh = Shield::new(AppId::Files, Reason::Limit, 1200, 800);
        let mut buf = vec![0u32; 1200 * 800];
        sh.draw(&mut Surface { w: 1200, h: 800, stride: 1200, px: &mut buf, y0: 0 }, &ui, 0.0);
        assert_eq!(buf[0], BG);
        assert!(buf.contains(&PLATE));
    }
}
