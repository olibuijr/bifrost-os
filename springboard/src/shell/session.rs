//! Session restoration (state restoration for the whole desktop): which app windows were open, in which order and
//! where, so that after a restart, a log out, an update or a crash the desktop comes back as it was.
//!
//! The shell keeps `session.txt` in its data dir. It is rewritten (only when it changed, at most every
//! [`SYNC_MS`]) while the desktop runs, so a crash leaves the last layout behind, and once more when the
//! power menu or Settings restarts or shuts the machine down: with an empty list when the user unchecked "Reopen
//! windows when logging back in". Frames are stored as parts of the screen ([`SCALE`] = the whole screen), so a layout
//! saved at one resolution or scale comes back sensibly on another.
//!
//! What goes *inside* an app's window (the open note, the folder) is the app's own business: the SDK's
//! `restoration` module. The shell only tells a launched app whether to restore it ([`restore_flag`], the
//! `BIFROST_RESTORE` variable).
use super::{apps, AppId, Shell};
use std::path::Path;

/// The file in the shell's data dir.
pub const FILE: &str = "session.txt";
/// Settings pref (`1`/`0`, default on): the "Reopen windows when logging back in" checkbox of the restart and shut down alerts.
pub const REOPEN_PREF: &str = "reopen_windows_on_login";
/// Settings pref (`1`/`0`, default off): General > "Close windows when quitting an application".
pub const CLOSE_ON_QUIT_PREF: &str = "close_windows_on_quit";
/// The most windows one session file restores.
pub const MAX_WINDOWS: usize = 64;
/// How often the layout is written at most (ms) while the desktop runs.
pub const SYNC_MS: f64 = 2000.0;
/// A frame coordinate of `SCALE` is the whole screen width or height.
pub const SCALE: i64 = 10_000;
const HEADER: &str = "bifrost-session 1";

/// One window: the app's stable id, its frame in parts of the screen, and the desktop (Overview) it is on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WindowRec {
    pub app: String,
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    pub maximized: bool,
    /// the person's own minimized state (a window of another desktop is hidden, not minimized)
    pub minimized: bool,
    /// the desktop it is on, from 0 (`overview::Spaces`); a line written before desktops were kept has none: 0
    pub desktop: usize,
}
/// The highest desktop a session line may name (Overview holds fewer; a larger number is junk).
const MAX_DESKTOP: usize = 63;

fn rel(v: i32, whole: i32) -> i32 {
    (v as i64 * SCALE / whole.max(1) as i64) as i32
}
fn abs(v: i32, whole: i32) -> i32 {
    (v as i64 * whole.max(1) as i64 / SCALE) as i32
}

impl WindowRec {
    /// A record from a frame in px on a screen of `screen` px.
    pub fn from_px(app: &str, (x, y, w, h): (i32, i32, i32, i32), screen: (i32, i32), maximized: bool, minimized: bool) -> Self {
        WindowRec { app: app.to_string(), x: rel(x, screen.0), y: rel(y, screen.1), w: rel(w, screen.0), h: rel(h, screen.1), maximized, minimized, desktop: 0 }
    }
    /// The frame in px on a screen of `screen` px.
    pub fn to_px(&self, screen: (i32, i32)) -> (i32, i32, i32, i32) {
        (abs(self.x, screen.0), abs(self.y, screen.1), abs(self.w, screen.0), abs(self.h, screen.1))
    }
}

/// Reverse-DNS ids and the built-in `bifrost.*` ids only: no separators or control characters can reach the file.
fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 128 && id.chars().all(|c| c.is_ascii_alphanumeric() || "._-".contains(c))
}

/// The windows of the last desktop, back to front.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Session {
    pub windows: Vec<WindowRec>,
}

impl Session {
    pub fn to_text(&self) -> String {
        let mut out = format!("{HEADER}\n");
        for w in &self.windows {
            let flags = u8::from(w.maximized) | u8::from(w.minimized) << 1;
            out.push_str(&format!("window\t{}\t{}\t{}\t{}\t{}\t{}\t{}\n", w.app, w.x, w.y, w.w, w.h, flags, w.desktop));
        }
        out
    }

    /// Reads a file's text. Anything unknown (header, a bad line, an app id with odd characters, a size that is not
    /// positive) is skipped; a repeated app keeps its first window; at most [`MAX_WINDOWS`]. The desktop column is
    /// optional: a line without it, or with a desktop that is not a small number, is on the first desktop.
    pub fn parse(text: &str) -> Session {
        let mut lines = text.lines();
        if lines.next().map(str::trim) != Some(HEADER) {
            return Session::default();
        }
        let mut windows: Vec<WindowRec> = Vec::new();
        for line in lines {
            let f: Vec<&str> = line.split('\t').collect();
            if !(7..=8).contains(&f.len()) || f[0] != "window" || !valid_id(f[1]) {
                continue;
            }
            let n = |i: usize| f[i].parse::<i32>().ok();
            let (Some(x), Some(y), Some(w), Some(h), Some(flags)) = (n(2), n(3), n(4), n(5), n(6)) else { continue };
            let sane = |v: i32| (-SCALE as i32..=2 * SCALE as i32).contains(&v);
            if w <= 0 || h <= 0 || !sane(x) || !sane(y) || w > 2 * SCALE as i32 || h > 2 * SCALE as i32 {
                continue;
            }
            if windows.iter().any(|r| r.app == f[1]) {
                continue;
            }
            let desktop = f.get(7).and_then(|d| d.parse::<usize>().ok()).filter(|d| *d <= MAX_DESKTOP).unwrap_or(0);
            windows.push(WindowRec { app: f[1].to_string(), x, y, w, h, maximized: flags & 1 != 0, minimized: flags & 2 != 0, desktop });
            if windows.len() == MAX_WINDOWS {
                break;
            }
        }
        Session { windows }
    }

    /// The windows to reopen: only the apps `known` still has (an app uninstalled since is dropped). If every window
    /// was minimized the last one is shown, so a restored desktop is never blank with windows hidden.
    pub fn plan(&self, known: impl Fn(&str) -> bool) -> Vec<WindowRec> {
        let mut out: Vec<WindowRec> = self.windows.iter().filter(|w| known(&w.app)).cloned().collect();
        if !out.is_empty() && out.iter().all(|w| w.minimized) {
            if let Some(last) = out.last_mut() {
                last.minimized = false;
            }
        }
        out
    }
}

/// Reads the session; a missing or unreadable file is an empty session.
pub fn load(dir: &Path) -> Session {
    std::fs::read_to_string(dir.join(FILE)).map(|t| Session::parse(&t)).unwrap_or_default()
}

/// Writes the session with an atomic replace; `Ok(false)` when the file already holds exactly this.
pub fn save(dir: &Path, session: &Session) -> std::io::Result<bool> {
    let text = session.to_text();
    if std::fs::read_to_string(dir.join(FILE)).is_ok_and(|old| old == text) {
        return Ok(false);
    }
    let tmp = dir.join(format!("{FILE}.tmp"));
    std::fs::write(&tmp, &text)?;
    std::fs::rename(&tmp, dir.join(FILE))?;
    Ok(true)
}

/// Whether a launched app is told to restore its own state (`BIFROST_RESTORE=1`): always for the windows the login
/// restore brings back, and for any other launch unless "Close windows when quitting an application" is on.
pub fn restore_flag(close_on_quit: bool, login_restore: bool) -> bool {
    login_restore || !close_on_quit
}

/// What the shell remembers between ticks.
#[derive(Debug, Default)]
pub struct Runtime {
    /// the saved session has been replayed (once per shell start, after the desktop is up and unlocked)
    pub restored: bool,
    /// true while the replay launches apps, so they restore their own state
    pub restoring: bool,
    /// the shell is going down: the final layout is written, later ticks leave it alone
    pub frozen: bool,
    /// the next time the layout may be written (ms)
    pub next_at: f64,
    /// when the login restore ran (shell ms), and the installed apps (slots) it launched
    pub restored_at: f64,
    pub restored_slots: Vec<u8>,
    /// apps quit because their package was updated while they ran, waiting for the old process to go to be reopened
    pub relaunch: Vec<Relaunch>,
}

/// An app quit to be reopened at its new version: its slot and its main window as it was.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Relaunch {
    pub slot: u8,
    pub frame: Option<WindowRec>,
}

/// Whether an app whose package was updated while it ran is quit and reopened now: when it is not the app in front, or
/// when it is but the login restore started it and the person has not used the desktop since. The app in front that the
/// person is using keeps running until they switch away, so a relaunch never happens under their hands. An app with
/// unsaved changes (`edited`, its document's "Edited" state) is never quit for an update: it waits until the document is
/// saved or closed, so the relaunch cannot lose work or stop on a "Save changes?" sheet.
pub fn relaunch_now(front: bool, restored_untouched: bool, edited: bool) -> bool {
    !edited && (!front || restored_untouched)
}

/// The stable id of a built-in or installed app (what `apps()` calls its icon name), or `None` for an unknown app.
fn key_of(app: AppId) -> Option<String> {
    apps().iter().find(|a| a.id == app).map(|a| a.icon.to_string())
}

fn app_of(key: &str) -> Option<AppId> {
    apps().iter().find(|a| a.icon == key).map(|a| a.id)
}

impl Shell {
    /// The layout now, back to front, with the desktop each window is on; empty when `reopen` is false.
    pub(super) fn session_snapshot(&self, reopen: bool) -> Session {
        let Some(desk) = self.desk.as_ref().filter(|_| reopen) else { return Session::default() };
        let m = self.desk_metrics();
        let windows = desk
            .wins
            .iter()
            .filter_map(|w| {
                let (desktop, minimized) = self.overview.spaces.saved_state(w.app, w.minimized);
                let rec = WindowRec::from_px(&key_of(w.app)?, (w.r.x, w.r.y, w.r.w, w.r.h), (m.w, m.h), w.maximized, minimized);
                Some(WindowRec { desktop, ..rec })
            })
            .take(MAX_WINDOWS)
            .collect();
        Session { windows }
    }

    /// Replays the saved session once, as soon as the desktop is up and unlocked: each window opens in its old
    /// place and its app is started with `BIFROST_RESTORE=1`.
    pub(super) fn session_restore(&mut self, t: f64) {
        if self.session.restored || self.desk.is_none() || self.is_locked() {
            return;
        }
        self.session.restored = true;
        // the keys held at login went to the login window or another client: no modifier stays counted into the session
        self.key_reset = true;
        // a login after Log Out (`utility_window::log_out` froze the layout when it wrote it down) writes the layout again
        self.session.frozen = false;
        let plan = load(&self.data_dir).plan(|key| app_of(key).is_some());
        if plan.is_empty() {
            return;
        }
        self.session.restoring = true;
        self.session.restored_at = t;
        let m = self.desk_metrics();
        let mut placed = Vec::new();
        for rec in &plan {
            let Some(app) = app_of(&rec.app) else { continue };
            if let AppId::External(slot) = app {
                self.session.restored_slots.push(slot);
            }
            self.desk_open(app, t);
            let Some(i) = self.desk.as_ref().and_then(|d| d.index_of(app)) else { continue };
            let (x, y, w, h) = rec.to_px((m.w, m.h));
            if let Some(d) = self.desk.as_mut() {
                d.place(&m, i, super::desk::Rect { x, y, w, h });
                if rec.maximized {
                    d.toggle_zoom(&m, i);
                }
                d.wins[i].minimized = rec.minimized;
            }
            placed.push((app, rec.desktop));
            self.desk_layout(i);
        }
        // each window goes back to the desktop it was on; the other desktops' windows stay hidden until switched to
        if placed.iter().any(|(_, d)| *d > 0) {
            self.overview_restore_desktops(&placed);
        }
        self.session.restoring = false;
        self.desk_refocus();
        self.dirty = true;
    }

    /// While the desktop runs: keeps the file in step with the windows (at most every [`SYNC_MS`]; nothing is written
    /// when the layout did not change).
    pub(super) fn session_tick(&mut self, t: f64) {
        if !self.session.restored || self.session.frozen || t < self.session.next_at {
            return;
        }
        self.session.next_at = t + SYNC_MS;
        let snap = self.session_snapshot(true);
        if let Err(e) = save(&self.data_dir, &snap) {
            bifrost_core::log::shell_logger("session").error(format!("cannot save the session: {e}"));
        }
    }

    /// Before a restart, shut down or update restart: writes the final layout, empty when the user unchecked
    /// "Reopen windows when logging back in", and stops the periodic writes. Apps that allowed sudden termination are
    /// then killed without a Quit round trip by `stop_apps`; their state is already on disk.
    pub fn session_save_for_exit(&mut self) {
        self.session.frozen = true;
        if self.desk.is_none() {
            return;
        }
        let reopen = self.prefs.get_bool(REOPEN_PREF, true);
        let snap = self.session_snapshot(reopen);
        if let Err(e) = save(&self.data_dir, &snap) {
            bifrost_core::log::shell_logger("session").error(format!("cannot save the session: {e}"));
        }
    }

    /// Apps updated while they ran start again at the new version. The login restore can start an app before the
    /// background update of its package lands (the check runs while the old version is already open), and an App Store
    /// update can replace a running app's package; the process keeps the old binary either way. Each such app is quit
    /// (`Quit`, so it saves its state) once [`relaunch_now`] allows it, and reopened at the new version with its windows
    /// restored, as apps updated during an OS update come back after login. Runs about once a second (`ota_tick`).
    pub(super) fn app_update_relaunch(&mut self, t: f64) {
        if self.desk.is_none() || self.is_locked() {
            return;
        }
        let registered = super::apphost::installed();
        // the apps quit earlier whose old process is gone are reopened
        let mut i = 0;
        while i < self.session.relaunch.len() {
            let slot = self.session.relaunch[i].slot;
            let id = registered.get(slot as usize).map(|a| a.manifest.id.as_str()).unwrap_or_default();
            // the person opened it again meanwhile: nothing left to do
            let reopened = self.ext.get(slot as usize).and_then(Option::as_ref).is_some_and(|h| h.running());
            let quitting = self.dying.iter().any(|h| h.id == id && h.running());
            if quitting && !reopened {
                i += 1;
                continue;
            }
            let r = self.session.relaunch.remove(i);
            if reopened || registered.get(slot as usize).is_none_or(|a| a.is_removed()) {
                continue;
            }
            self.relaunch_reopen(r, t);
        }
        let front = self.ext_fg;
        let untouched = self.last_input <= self.session.restored_at;
        let due: Vec<u8> = self
            .ext
            .iter()
            .enumerate()
            .filter_map(|(slot, h)| {
                let h = h.as_ref().filter(|h| h.running())?;
                let app = registered.get(slot).filter(|a| !a.is_removed())?;
                let slot = u8::try_from(slot).ok()?;
                if self.session.relaunch.iter().any(|r| r.slot == slot) || !h.is_outdated(&app.exec_path()) {
                    return None;
                }
                relaunch_now(front == Some(slot), untouched && self.session.restored_slots.contains(&slot), h.doc_edited).then_some(slot)
            })
            .collect();
        for slot in due {
            let app = AppId::External(slot);
            let m = self.desk_metrics();
            let frame = self
                .desk
                .as_ref()
                .and_then(|d| d.index_of(app).map(|i| &d.wins[i]))
                .map(|w| WindowRec::from_px(&key_of(app).unwrap_or_default(), (w.r.x, w.r.y, w.r.w, w.r.h), (m.w, m.h), w.maximized, w.minimized));
            let id = registered.get(slot as usize).map(|a| a.manifest.id.as_str()).unwrap_or_default();
            bifrost_core::log::shell_logger("session").info(format!("{id} was updated while it ran: relaunching it at the new version"));
            self.session.relaunch.push(Relaunch { slot, frame });
            self.desk_quit_app(slot);
        }
    }

    /// Opens an app quit by [`Shell::app_update_relaunch`] again, telling it to restore its state, with its main window
    /// where it was; the app in front before stays in front.
    fn relaunch_reopen(&mut self, r: Relaunch, t: f64) {
        let app = AppId::External(r.slot);
        let before = self.desk.as_ref().and_then(|d| d.front()).map(|w| w.app).filter(|a| a.parent() != app);
        self.session.restoring = true;
        self.desk_open(app, t);
        self.session.restoring = false;
        let m = self.desk_metrics();
        if let (Some(rec), Some(i)) = (r.frame, self.desk.as_ref().and_then(|d| d.index_of(app))) {
            let (x, y, w, h) = rec.to_px((m.w, m.h));
            if let Some(d) = self.desk.as_mut() {
                d.place(&m, i, super::desk::Rect { x, y, w, h });
                if rec.maximized && !d.wins[i].maximized {
                    d.toggle_zoom(&m, i);
                }
                d.wins[i].minimized = rec.minimized;
            }
            self.desk_layout(i);
        }
        if let Some(front) = before {
            if let Some(d) = self.desk.as_mut() {
                if let Some(k) = d.index_of(front) {
                    d.raise(k);
                }
            }
        }
        self.desk_refocus();
    }

    /// True when an app launched now should restore its own state.
    pub(super) fn session_restore_flag(&self) -> bool {
        restore_flag(self.prefs.get_bool(CLOSE_ON_QUIT_PREF, false), self.session.restoring)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(app: &str, x: i32) -> WindowRec {
        WindowRec { app: app.into(), x, y: 500, w: 6000, h: 5000, maximized: false, minimized: false, desktop: 0 }
    }

    #[test]
    fn an_updated_app_relaunches_unless_the_person_is_using_it() {
        assert!(relaunch_now(false, false, false), "in the background: quit and reopened now");
        assert!(relaunch_now(true, true, false), "in front, but only the login restore opened it");
        assert!(!relaunch_now(true, false, false), "in front and in use: waits until the person switches away");
    }

    #[test]
    fn an_updated_app_with_unsaved_changes_waits_until_they_are_saved() {
        for (front, untouched) in [(false, false), (true, true), (true, false)] {
            assert!(!relaunch_now(front, untouched, true), "edited document: never quit for an update ({front}, {untouched})");
        }
    }

    #[test]
    fn the_desktop_round_trips_and_a_line_without_it_is_on_the_first() {
        let mut s = Session { windows: vec![rec("a.one", 0), rec("a.two", 0)] };
        s.windows[1].desktop = 3;
        assert_eq!(Session::parse(&s.to_text()), s);
        let text = format!("{HEADER}\nwindow\ta.old\t0\t0\t5000\t5000\t0\nwindow\ta.bad\t0\t0\t5000\t5000\t0\tx\nwindow\ta.far\t0\t0\t5000\t5000\t0\t9999\n");
        let s = Session::parse(&text);
        assert_eq!(s.windows.iter().map(|w| (w.app.as_str(), w.desktop)).collect::<Vec<_>>(), [("a.old", 0), ("a.bad", 0), ("a.far", 0)]);
        assert!(Session::parse(&format!("{HEADER}\nwindow\ta.b\t0\t0\t5000\t5000\t0\t1\textra\n")).windows.is_empty(), "a line with more columns is junk");
    }

    #[test]
    fn a_session_round_trips_in_order() {
        let mut s = Session { windows: vec![rec("is.olibuijr.notes.desktop", 100), rec("bifrost.settings", 2500)] };
        s.windows[1].maximized = true;
        s.windows[0].minimized = true;
        assert_eq!(Session::parse(&s.to_text()), s);
    }

    #[test]
    fn frames_are_parts_of_the_screen_and_survive_a_resolution_change() {
        let r = WindowRec::from_px("a.b", (480, 270, 960, 540), (1920, 1080), false, false);
        assert_eq!((r.x, r.y, r.w, r.h), (2500, 2500, 5000, 5000));
        assert_eq!(r.to_px((1920, 1080)), (480, 270, 960, 540));
        assert_eq!(r.to_px((3840, 2160)), (960, 540, 1920, 1080), "the same place on a bigger screen");
    }

    #[test]
    fn junk_in_the_file_is_skipped() {
        assert_eq!(Session::parse("not a session\nwindow\ta\t0\t0\t1\t1\t0\n"), Session::default(), "wrong header");
        let text = format!(
            "{HEADER}\nwindow\tgood.app\t0\t0\t5000\t5000\t0\nwindow\tbad id\t0\t0\t5000\t5000\t0\nwindow\tzero.size\t0\t0\t0\t5000\t0\nwindow\toff.screen\t999999\t0\t5000\t5000\t0\nwindow\tgood.app\t1\t1\t5000\t5000\t0\nwindow\tshort\t1\nwindow\tx.y\ta\t0\t5000\t5000\t0\n"
        );
        let s = Session::parse(&text);
        assert_eq!(s.windows.len(), 1, "{s:?}");
        assert_eq!(s.windows[0].app, "good.app");
        assert_eq!(s.windows[0].x, 0, "a repeated app keeps its first window");
    }

    #[test]
    fn at_most_max_windows_come_back() {
        let s = Session { windows: (0..100).map(|i| rec(&format!("app.n{i}"), 0)).collect() };
        assert_eq!(Session::parse(&s.to_text()).windows.len(), MAX_WINDOWS);
    }

    #[test]
    fn the_plan_drops_uninstalled_apps_and_never_hides_every_window() {
        let mut s = Session { windows: vec![rec("gone.app", 0), rec("a.one", 0), rec("a.two", 0)] };
        s.windows[1].minimized = true;
        s.windows[2].minimized = true;
        let plan = s.plan(|id| id != "gone.app");
        assert_eq!(plan.iter().map(|w| w.app.as_str()).collect::<Vec<_>>(), ["a.one", "a.two"]);
        assert!(plan[0].minimized && !plan[1].minimized, "the last one is shown");
        assert!(Session::default().plan(|_| true).is_empty());
    }

    #[test]
    fn save_writes_once_and_load_reads_it_back() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(load(dir.path()), Session::default(), "no file is an empty session");
        let s = Session { windows: vec![rec("a.one", 10)] };
        assert!(save(dir.path(), &s).unwrap());
        assert!(!save(dir.path(), &s).unwrap(), "an unchanged layout is not rewritten");
        assert_eq!(load(dir.path()), s);
        assert!(save(dir.path(), &Session::default()).unwrap(), "unchecking Reopen windows empties it");
        assert!(load(dir.path()).windows.is_empty());
        assert!(!dir.path().join(format!("{FILE}.tmp")).exists());
    }

    #[test]
    fn apps_restore_unless_windows_close_on_quit() {
        assert!(restore_flag(false, false), "the default keeps windows across a quit");
        assert!(!restore_flag(true, false), "Close windows when quitting starts the app fresh");
        assert!(restore_flag(true, true), "windows reopened at login restore even then");
    }
}
