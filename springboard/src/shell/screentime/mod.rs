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

use super::{AppId, Drawn, Effect, Event, Screen, Shell};
use config::Config;
use shield::{Out, Shield};
use std::path::PathBuf;

/// How often the shell wants a tick while an app is on screen, so seconds
/// are credited and limits noticed without any input.
pub const WAKE_MS: f64 = 5000.0;

impl Shell {
    pub(super) fn st_config_path(&self) -> PathBuf {
        self.data_dir.join("screentime.ini")
    }

    pub(super) fn st_passcode_path(&self) -> PathBuf {
        self.data_dir.join("screentime_passcode.ini")
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

    /// Records usage and raises or drops the shield. Called from `tick`.
    pub(super) fn screentime_tick(&mut self, t: f64) {
        let (unix, offset) = self.wall_now(t);
        let fg = self.st_foreground();
        if unix != self.st_checked {
            self.st_checked = unix;
            self.screentime.tick(unix, offset, fg, self.screen_on);
        }
        if let Some(sh) = &mut self.shield {
            if fg != Some(sh.app) || self.clock.alert().is_some() || self.low_alert.is_some() || self.power_menu.is_some() {
                self.shield = None;
                self.dirty = true;
            } else if sh.tick(unix, t) {
                self.dirty = true;
            }
            return;
        }
        let Some(app) = fg.filter(|_| !self.st_suspended()) else { return };
        if self.st_blocked_at == unix {
            return;
        }
        self.st_blocked_at = unix;
        let used = self.screentime.today().app_secs(app);
        if let Some(reason) = self.st_config().block(app, used, unix, offset) {
            self.shield = Some(Shield::new(app, reason, self.w, self.h));
            self.dirty = true;
            self.drawn = Drawn::Nothing;
        }
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

    /// Touches while the shield is up; hardware keys never get here.
    pub(super) fn shield_event(&mut self, ev: Event, t: f64) -> Option<Vec<Effect>> {
        if !matches!(ev, Event::Down(..) | Event::Move(..) | Event::Up(..)) || self.st_suspended() {
            return None;
        }
        let (unix, offset) = self.wall_now(t);
        let pass = self.st_passcode_path();
        let sh = self.shield.as_mut()?;
        let out = sh.touch(&self.ui, ev, t, &pass);
        let app = sh.app;
        self.dirty = true;
        match out {
            Out::Open => {}
            Out::Home => {
                self.shield = None;
                self.go_home(t);
                self.pager.home(t);
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
        Some(vec![])
    }
}
