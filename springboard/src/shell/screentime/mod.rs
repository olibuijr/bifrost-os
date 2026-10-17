//! Screen Time: foreground-app usage per day, per-app daily limits,
//! Downtime, and the Time Limit shield. `usage` records, `config` decides,
//! `chart` and `page` show it in Settings, `shield` covers a blocked app.
//! The `impl Shell` below is the shell's side: it feeds the tracker from
//! `tick`, raises and drops the shield, and applies an ignore grant.

pub mod chart;
pub mod config;
pub mod page;
pub mod shield;
pub mod usage;

use super::notify::Notification;
use super::{Anim, AppId, Drawn, Effect, Event, Screen, Shell, apps};
use config::Config;
use shield::{Out, Shield};
use std::path::PathBuf;

/// The Screen Time passcode: a keychain item under the shell data dir. The file it replaced
/// (`screentime_passcode.ini`) is migrated once and removed.
pub fn passcode(data_dir: &std::path::Path) -> super::passcode::Store {
    super::passcode::Store::load_keychain(data_dir, &data_dir.join("screentime_passcode.ini"))
}

/// How often the shell wants a tick while an app is on screen, so seconds
/// are credited and limits noticed without any input.
pub const WAKE_MS: f64 = 5000.0;

impl Shell {
    pub(super) fn st_config_path(&self) -> PathBuf {
        self.data_dir.join("screentime.ini")
    }

    /// Points Screen Time at the shell's data directory.
    pub(super) fn st_load(&mut self) {
        self.screentime = usage::Tracker::new(self.data_dir.join("screentime"));
    }

    /// The Screen Time config, re-read only when `screentime.ini` changed (Settings edits it).
    fn st_config(&mut self) -> &Config {
        let path = self.st_config_path();
        let stamp = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
        if self.st_cfg.as_ref().is_none_or(|(_, seen)| *seen != stamp) {
            self.st_cfg = Some((Config::load(&path), stamp));
        }
        &self.st_cfg.as_ref().expect("just loaded").0
    }

    /// The app Screen Time credits right now: the one fully on screen.
    fn st_foreground(&self) -> Option<AppId> {
        match self.screen {
            Screen::App(app) if self.screen_on && self.anim.is_none() => Some(app),
            _ => None,
        }
    }

    /// System modals that must stay reachable: an alarm or timer alert, the
    /// low-battery alert and the power menu. The shield steps aside for them.
    pub(super) fn st_suspended(&self) -> bool {
        self.clock.alert().is_some() || self.low_alert.is_some() || self.power_menu.is_some()
    }

    /// Records the state now (see `Shell::event`). Does not touch the tick gate.
    pub(super) fn st_mark(&mut self, t: f64) {
        let (unix, offset) = self.wall_now(t);
        let fg = self.st_foreground();
        self.screentime.tick(unix, offset, fg, self.screen_on);
    }

    /// Records usage and raises or drops the shield. Called from `tick`.
    pub(super) fn screentime_tick(&mut self, t: f64) {
        let (unix, offset) = self.wall_now(t);
        let fg = self.st_foreground();
        if unix != self.st_checked {
            self.st_checked = unix;
            self.screentime.tick(unix, offset, fg, self.screen_on);
        }
        if let Some(sh) = &self.shield {
            // the cover stays up while its app is still zooming open
            let opening = matches!(self.anim, Some((Anim::Open { app, .. }, _)) if app == sh.app);
            let kept = fg == Some(sh.app) || opening;
            if !kept || self.st_suspended() {
                self.shield = None;
                self.desk_cover_changed();
            } else if let Some(sh) = &mut self.shield {
                if sh.tick(unix, t) {
                    self.dirty = true;
                }
            }
            return;
        }
        let Some(app) = fg.filter(|_| !self.st_suspended()) else { return };
        if self.st_blocked_at == unix {
            return;
        }
        self.st_blocked_at = unix;
        let used = self.screentime.today().app_secs(app);
        self.st_warn(app, used, unix, offset);
        if let Some(reason) = self.st_config().block(app, used, unix, offset) {
            self.shield = Some(Shield::new(app, reason, self.w, self.h));
            self.desk_cover_changed();
            self.drawn = Drawn::Nothing;
        }
    }

    /// Posts the five-minutes-left notification once per app per day.
    fn st_warn(&mut self, app: AppId, used: u32, unix: i64, offset: i32) {
        let day = usage::day_of(unix, offset);
        if !self.st_config().warn_due(app, used, unix, offset) || self.st_warned.contains(&(day, app)) {
            return;
        }
        self.st_warned.retain(|(d, _)| *d == day);
        self.st_warned.push((day, app));
        let name = apps().iter().find(|a| a.id == app).map_or("this app", |a| a.name);
        self.notify(Notification::new(app, "Screen Time", format!("5 minutes left today for {name}."), unix));
    }

    /// Checks a launch before the app's frame zooms open (phone) or its window
    /// is shown (desktop): a limit that is already reached (or Downtime) covers
    /// the app at once, so it never shows its own screen. True when covered.
    pub(super) fn st_launch_check(&mut self, app: AppId, t: f64) -> bool {
        if self.st_suspended() {
            return false;
        }
        let (unix, offset) = self.wall_now(t);
        let used = self.screentime.today().app_secs(app);
        let Some(reason) = self.st_config().block(app, used, unix, offset) else { return false };
        self.shield = Some(Shield::new(app, reason, self.w, self.h));
        self.desk_cover_changed();
        self.drawn = Drawn::Nothing;
        true
    }

    /// When Screen Time next needs a tick on its own.
    pub(super) fn screentime_wake(&self, t: f64) -> Option<f64> {
        if !self.screen_on {
            return None;
        }
        if let Some(sh) = &self.shield {
            return sh.next_wake(self.wall_now(t).0, t).or(Some(t + WAKE_MS));
        }
        self.st_foreground().map(|_| t + WAKE_MS)
    }

    /// Touches while the shield is up; hardware keys never get here. On the
    /// desktop the cover is a window body: touches outside it (the title bar,
    /// the lights, other windows) pass through, so the window can still be
    /// moved or closed.
    pub(super) fn shield_event(&mut self, ev: Event, t: f64) -> Option<Vec<Effect>> {
        let (x, y) = match ev {
            Event::Down(x, y) | Event::Move(x, y) | Event::Up(x, y) => (x, y),
            _ => return None,
        };
        if self.st_suspended() {
            return None;
        }
        let (dx, dy) = if self.desk_active() {
            let app = self.shield.as_ref()?.app;
            let (rx, ry, rw, rh) = self.desk_cover_rect(app)?;
            let inside = x >= rx as f32 && x < (rx + rw) as f32 && y >= ry as f32 && y < (ry + rh) as f32;
            if !inside {
                return None;
            }
            (rx as f32, ry as f32)
        } else {
            (0.0, 0.0)
        };
        let local = match ev {
            Event::Down(..) => Event::Down(x - dx, y - dy),
            Event::Move(..) => Event::Move(x - dx, y - dy),
            _ => Event::Up(x - dx, y - dy),
        };
        let (unix, offset) = self.wall_now(t);
        let pass = passcode(&self.data_dir);
        let desk = self.desk_active();
        let sh = self.shield.as_mut()?;
        let out = sh.touch(&self.ui, local, t, &pass);
        let app = sh.app;
        self.dirty = true;
        match out {
            Out::Open => {}
            Out::Home => {
                self.shield = None;
                if desk {
                    // on the desktop, OK closes the window the cover was over
                    self.desk_close_app(app);
                } else {
                    self.go_home(t);
                    self.pager.home(t);
                }
            }
            Out::Ignore(how) => {
                let path = self.st_config_path();
                let mut cfg = Config::load(&path);
                cfg.grant(app, how, unix, offset);
                self.st_cfg = None;
                self.shield = None;
                self.st_blocked_at = 0;
                self.drawn = Drawn::Nothing;
            }
        }
        self.desk_cover_changed();
        Some(vec![])
    }
}
