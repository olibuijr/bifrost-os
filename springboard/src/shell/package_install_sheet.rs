//! The package install sheet (`PackageUIKit`): asks the person before pacman installs software, since each package's
//! install script runs during the install, then shows how far the install has got and how it ended. The confirmation
//! rule is `bifrost_core::pacman_progress::confirmation_needed`, the progress is `pacman_progress::progress` over the
//! pacman log.
//!
//! The sheet is a model plus its card: `Sheet::new` asks, `answer` takes the person's choice, `update` reads the log
//! while the install runs and `finish` shows the result. The shell opens it from two places
//! (`Shell::package_install_open`): opening a package archive (`.pkg.tar.zst`) from Files (`workspace.rs`, the
//! `OpenFile` route), and a store link `bifrost-package:install/name,name` (`workspace_open_url`). Install runs
//! `pacman_update::Source::run` on a worker thread (privd does the root work), and the shell tick follows the pacman log
//! for the progress. Cancel, or Escape before the install starts, closes the sheet and nothing is installed.

use super::pacman_update::Source;
use super::ui::{AlertCard, Ui, Weight, LINK, MAC_BODY_PT};
use super::Drawn;
use crate::gfx::Surface;
use bifrost_core::pacman_progress;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, TryRecvError};

/// pacman's log, where an install writes its steps.
pub const PACMAN_LOG: &str = "/var/log/pacman.log";

/// How often the log is read while an install runs, in the shell clock's milliseconds.
pub const LOG_POLL_MS: f64 = 500.0;

/// The buttons of the sheet, left to right while it asks; `Done` is the one button after the install ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Button {
    Cancel,
    Install,
    Done,
}

/// Where the sheet is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Step {
    /// Waiting for the person to press Install or Cancel.
    Confirm,
    /// The install runs; `fraction` is how far it has got, from 0 to 1.
    Installing { fraction: f32 },
    /// The install ended; `ok` says whether it worked.
    Finished { ok: bool },
}

pub struct Sheet {
    packages: Vec<String>,
    step: Step,
    /// Why the install failed, when it did.
    error: Option<String>,
    card: AlertCard,
}

impl Sheet {
    /// The confirmation for `packages`. None when there is nothing to install, since there is nothing to confirm.
    pub fn new(ui: &Ui, packages: Vec<String>) -> Option<Self> {
        if !pacman_progress::confirmation_needed(&packages) {
            return None;
        }
        let mut sheet = Sheet { packages, step: Step::Confirm, error: None, card: AlertCard::new(ui, "", "", Vec::new()) };
        sheet.card = sheet.build_card(ui);
        Some(sheet)
    }

    pub fn step(&self) -> Step {
        self.step
    }

    /// The packages this sheet is about.
    pub fn packages(&self) -> &[String] {
        &self.packages
    }

    fn what(&self) -> String {
        match self.packages.len() {
            1 => self.packages[0].clone(),
            n => format!("{n} packages"),
        }
    }

    fn title(&self) -> String {
        match self.step {
            Step::Confirm => format!("Install {}?", self.what()),
            Step::Installing { .. } => "Installing".to_string(),
            Step::Finished { ok: true } => format!("Installed {}", self.what()),
            Step::Finished { ok: false } => format!("Could not install {}", self.what()),
        }
    }

    fn detail(&self) -> String {
        match self.step {
            Step::Confirm => "Installing runs the setup script of each package, so only install software you trust.".to_string(),
            Step::Installing { fraction } => format!("{} percent done.", (fraction * 100.0).round() as u32),
            Step::Finished { ok: true } => "The software is ready to use.".to_string(),
            Step::Finished { ok: false } => self.error.clone().unwrap_or_else(|| "The install stopped.".to_string()),
        }
    }

    fn build_card(&self, ui: &Ui) -> AlertCard {
        let buttons = match self.step {
            Step::Confirm => vec![
                ui.text("Cancel", MAC_BODY_PT, Weight::Regular, LINK),
                ui.text("Install", MAC_BODY_PT, Weight::Semibold, LINK),
            ],
            Step::Installing { .. } => Vec::new(),
            Step::Finished { .. } => vec![ui.text("Done", MAC_BODY_PT, Weight::Semibold, LINK)],
        };
        AlertCard::new(ui, &self.title(), &self.detail(), buttons)
    }

    /// The button under (x, y), if any.
    pub fn hit(&self, ui: &Ui, w: i32, h: i32, x: f32, y: f32) -> Option<Button> {
        let index = self.card.hit(ui, w, h, x, y)?;
        match self.step {
            Step::Confirm => Some(if index == 0 { Button::Cancel } else { Button::Install }),
            Step::Installing { .. } => None,
            Step::Finished { .. } => Some(Button::Done),
        }
    }

    /// Centre of button `index`, for tests.
    #[cfg(test)]
    pub fn button(&self, ui: &Ui, w: i32, h: i32, index: usize) -> (f32, f32) {
        self.card.button_centre(ui, w, h, index)
    }

    /// The person's choice. Install starts the install and returns true; Cancel returns false and the sheet should
    /// close. Only the confirmation takes an answer, so a second press does nothing.
    pub fn answer(&mut self, ui: &Ui, button: Button) -> bool {
        if self.step != Step::Confirm {
            return true;
        }
        match button {
            Button::Cancel | Button::Done => false,
            Button::Install => {
                self.step = Step::Installing { fraction: 0.0 };
                self.card = self.build_card(ui);
                true
            }
        }
    }

    /// Reads the progress of the install from the pacman log. Does nothing before the install starts.
    pub fn update(&mut self, ui: &Ui, log: &str) {
        if let Step::Installing { .. } = self.step {
            self.step = Step::Installing { fraction: pacman_progress::progress(log, self.packages.len()) };
            self.card = self.build_card(ui);
        }
    }

    /// The install ended with `result`: the sheet says so and offers Done. Does nothing before the install starts.
    pub fn finish(&mut self, ui: &Ui, result: Result<(), String>) {
        if let Step::Installing { .. } = self.step {
            self.step = Step::Finished { ok: result.is_ok() };
            self.error = result.err();
            self.card = self.build_card(ui);
        }
    }

    pub fn draw(&self, s: &mut Surface, ui: &Ui, w: i32, h: i32) {
        self.card.draw(s, ui, w, h);
    }
}

/// The part of pacman's log written after the install started: from byte `offset` on, empty when the log cannot be read.
pub fn read_log_since(log: &Path, offset: u64) -> String {
    use std::io::{Read, Seek, SeekFrom};
    let Ok(mut file) = std::fs::File::open(log) else { return String::new() };
    if file.seek(SeekFrom::Start(offset)).is_err() {
        return String::new();
    }
    let mut bytes = Vec::new();
    let _ = file.read_to_end(&mut bytes);
    String::from_utf8_lossy(&bytes).into_owned()
}

/// What the real install runs.
fn run_source(source: &Source) -> Result<(), String> {
    source.run()
}

/// Runs an install and reports how it ended. A function pointer so tests can stand in for pacman.
pub type Runner = fn(&Source) -> Result<(), String>;

/// The sheet on screen with what it installs, and the running install.
pub struct Session {
    sheet: Sheet,
    source: Source,
    runner: Runner,
    log: PathBuf,
    /// Where the log ended when the install started.
    log_start: u64,
    done: Option<Receiver<Result<(), String>>>,
    next_poll: f64,
}

impl super::Shell {
    /// Whether the install sheet is on screen. Touches and typing belong to it while it is.
    pub(super) fn package_showing(&self) -> bool {
        self.package_session.is_some()
    }

    /// Opens the install sheet for `source`: the action behind opening a package archive in Files and behind a store
    /// link. False when another install sheet is up or the source cannot be offered (not a package file, bad names).
    pub fn package_install_open(&mut self, source: Source) -> bool {
        self.package_install_open_with(source, run_source, PathBuf::from(PACMAN_LOG))
    }

    fn package_install_open_with(&mut self, source: Source, runner: Runner, log: PathBuf) -> bool {
        if self.package_session.is_some() {
            return false;
        }
        let Ok(names) = source.packages() else { return false };
        let Some(sheet) = Sheet::new(&self.ui, names) else { return false };
        self.package_session = Some(Session { sheet, source, runner, log, log_start: 0, done: None, next_poll: 0.0 });
        self.dirty = true;
        self.drawn = Drawn::Nothing;
        true
    }

    /// Opens the install sheet for the first package archive among `files`. False when none is one.
    pub(super) fn package_open_files(&mut self, files: &[PathBuf]) -> bool {
        let Some(path) = files.iter().find(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(super::pacman_update::is_package_file)) else { return false };
        self.package_install_open(Source::File(path.clone()))
    }

    /// Follows a running install: shows the result when the worker is done, and the progress from the pacman log between.
    pub(super) fn package_tick(&mut self, t: f64) {
        let Some(session) = self.package_session.as_mut() else { return };
        let Some(rx) = session.done.as_ref() else { return };
        let ended = match rx.try_recv() {
            Ok(result) => Some(result),
            Err(TryRecvError::Disconnected) => Some(Err("The install stopped before it finished.".to_string())),
            Err(TryRecvError::Empty) => None,
        };
        match ended {
            Some(result) => {
                session.done = None;
                session.sheet.finish(&self.ui, result);
            }
            None if t >= session.next_poll => {
                session.next_poll = t + LOG_POLL_MS;
                let log = read_log_since(&session.log, session.log_start);
                let fraction = pacman_progress::progress(&log, session.sheet.packages().len());
                if !matches!(session.sheet.step(), Step::Installing { fraction: f } if (f - fraction).abs() < 0.001) {
                    session.sheet.update(&self.ui, &log);
                }
            }
            None => return,
        }
        self.dirty = true;
        self.drawn = Drawn::Nothing;
    }

    /// A touch on the sheet: Cancel or Done close it, Install starts the install. Every touch belongs to the sheet.
    pub(super) fn package_touch(&mut self, x: f32, y: f32, up: bool) {
        if up {
            let hit = self.package_session.as_ref().and_then(|s| s.sheet.hit(&self.ui, self.w, self.h, x, y));
            match hit {
                Some(Button::Cancel) | Some(Button::Done) => self.package_session = None,
                Some(Button::Install) => self.package_start(),
                None => {}
            }
        }
        self.dirty = true;
        self.drawn = Drawn::Nothing;
    }

    /// Escape closes the sheet while it asks and after the install ended. Other keys do nothing, and Return never
    /// installs: only the Install button does.
    pub(super) fn package_type(&mut self, out: super::keyboard::Out) {
        if out == super::keyboard::Out::Named("Escape") {
            let closable = self.package_session.as_ref().is_some_and(|s| !matches!(s.sheet.step(), Step::Installing { .. }));
            if closable {
                self.package_session = None;
                self.dirty = true;
                self.drawn = Drawn::Nothing;
            }
        }
    }

    /// The person pressed Install: the sheet moves to the progress and the install runs on a worker thread.
    fn package_start(&mut self) {
        let Some(session) = self.package_session.as_mut() else { return };
        if !session.sheet.answer(&self.ui, Button::Install) {
            return;
        }
        session.log_start = std::fs::metadata(&session.log).map(|m| m.len()).unwrap_or(0);
        let (tx, rx) = std::sync::mpsc::channel();
        let (source, runner) = (session.source.clone(), session.runner);
        crate::bare::sys::spawn_named("package-install", move || {
            let _ = tx.send(runner(&source));
        });
        session.done = Some(rx);
    }

    /// Draws the card over a dimmed screen.
    pub(super) fn package_draw(&self, s: &mut Surface) {
        if let Some(session) = &self.package_session {
            session.sheet.draw(s, &self.ui, self.w, self.h);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOG: &str = "[2026-10-17T10:00:00+0000] [ALPM] transaction started\n[2026-10-17T10:00:01+0000] [ALPM] installed a (1-1)\n";

    #[test]
    fn nothing_to_install_means_no_sheet() {
        let ui = crate::shell::tests::ui();
        assert!(Sheet::new(&ui, Vec::new()).is_none());
    }

    #[test]
    fn install_waits_for_the_persons_confirmation() {
        let ui = crate::shell::tests::ui();
        let mut sheet = Sheet::new(&ui, vec!["a".into(), "b".into()]).unwrap();
        assert_eq!(sheet.step(), Step::Confirm);
        assert_eq!(sheet.packages().len(), 2);
        assert!(!sheet.answer(&ui, Button::Cancel), "cancel does not start the install");
        assert_eq!(sheet.step(), Step::Confirm);
        assert!(sheet.answer(&ui, Button::Install));
        assert_eq!(sheet.step(), Step::Installing { fraction: 0.0 });
    }

    #[test]
    fn progress_follows_the_pacman_log_once_the_install_runs() {
        let ui = crate::shell::tests::ui();
        let mut sheet = Sheet::new(&ui, vec!["a".into(), "b".into()]).unwrap();
        sheet.update(&ui, LOG);
        assert_eq!(sheet.step(), Step::Confirm, "no progress before the person says install");
        sheet.answer(&ui, Button::Install);
        sheet.update(&ui, LOG);
        assert_eq!(sheet.step(), Step::Installing { fraction: 0.5 });
    }

    #[test]
    fn a_single_package_is_named_in_the_question() {
        let ui = crate::shell::tests::ui();
        let sheet = Sheet::new(&ui, vec!["tool".into()]).unwrap();
        assert_eq!(sheet.title(), "Install tool?");
        let many = Sheet::new(&ui, vec!["a".into(), "b".into(), "c".into()]).unwrap();
        assert_eq!(many.title(), "Install 3 packages?");
    }

    #[test]
    fn the_result_replaces_the_progress_and_offers_done() {
        let ui = crate::shell::tests::ui();
        let mut sheet = Sheet::new(&ui, vec!["tool".into()]).unwrap();
        sheet.finish(&ui, Ok(()));
        assert_eq!(sheet.step(), Step::Confirm, "no result before the install starts");
        sheet.answer(&ui, Button::Install);
        sheet.finish(&ui, Err("error: target not found: tool".into()));
        assert_eq!(sheet.step(), Step::Finished { ok: false });
        assert_eq!(sheet.title(), "Could not install tool");
        assert_eq!(sheet.detail(), "error: target not found: tool");
        let (x, y) = sheet.button(&ui, 1200, 1920, 0);
        assert_eq!(sheet.hit(&ui, 1200, 1920, x, y), Some(Button::Done));
        let mut ok = Sheet::new(&ui, vec!["tool".into()]).unwrap();
        ok.answer(&ui, Button::Install);
        ok.finish(&ui, Ok(()));
        assert_eq!((ok.step(), ok.title().as_str()), (Step::Finished { ok: true }, "Installed tool"));
    }

    // ---- the shell action

    fn fake_ok(_: &Source) -> Result<(), String> {
        Ok(())
    }

    fn fake_fail(_: &Source) -> Result<(), String> {
        Err("pacman exited with 1: error: failed to commit transaction".into())
    }

    fn scratch_log(name: &str, text: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("package-sheet-{name}-{}.log", std::process::id()));
        std::fs::write(&path, text).unwrap();
        path
    }

    fn tap(sh: &mut crate::shell::Shell, index: usize) {
        let (x, y) = sh.package_session.as_ref().expect("the sheet is up").sheet.button(&sh.ui, sh.w, sh.h, index);
        sh.package_touch(x, y, true);
    }

    fn wait_for_the_worker(sh: &mut crate::shell::Shell) {
        for k in 0..400 {
            sh.package_tick(10_000.0 + k as f64 * LOG_POLL_MS);
            if !matches!(sh.package_session.as_ref().unwrap().sheet.step(), Step::Installing { .. }) {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        panic!("the install did not end");
    }

    #[test]
    fn a_package_file_opens_the_sheet_and_install_runs_only_after_the_button() {
        let mut sh = crate::shell::tests::shell();
        let log = scratch_log("file", "old line before\n");
        let file = PathBuf::from("/home/ola/Downloads/htop-3.3.0-1-x86_64.pkg.tar.zst");
        assert!(sh.package_install_open_with(Source::File(file), fake_ok, log.clone()));
        assert!(sh.package_showing());
        assert_eq!(sh.package_session.as_ref().unwrap().sheet.packages(), ["htop".to_string()]);
        assert_eq!(sh.package_session.as_ref().unwrap().sheet.step(), Step::Confirm);
        // a tick, a stray tap and Return all leave it asking; nothing runs
        sh.package_tick(0.0);
        sh.package_touch(2.0, 2.0, true);
        sh.package_type(crate::shell::keyboard::Out::Enter);
        assert_eq!(sh.package_session.as_ref().unwrap().sheet.step(), Step::Confirm);
        // another install cannot open over it
        assert!(!sh.package_install_open_with(Source::Repo(vec!["vim".into()]), fake_ok, log.clone()));
        tap(&mut sh, 1);
        assert!(matches!(sh.package_session.as_ref().unwrap().sheet.step(), Step::Installing { .. }));
        wait_for_the_worker(&mut sh);
        assert_eq!(sh.package_session.as_ref().unwrap().sheet.step(), Step::Finished { ok: true });
        tap(&mut sh, 0);
        assert!(!sh.package_showing(), "Done closes the sheet");
        let _ = std::fs::remove_file(&log);
    }

    #[test]
    fn cancel_and_escape_close_the_sheet_and_install_nothing() {
        let mut sh = crate::shell::tests::shell();
        let log = scratch_log("cancel", "");
        assert!(sh.package_install_open_with(Source::Repo(vec!["htop".into()]), fake_fail, log.clone()));
        tap(&mut sh, 0);
        assert!(!sh.package_showing());
        assert!(sh.package_install_open_with(Source::Repo(vec!["htop".into()]), fake_fail, log.clone()));
        sh.package_type(crate::shell::keyboard::Out::Named("Escape"));
        assert!(!sh.package_showing());
        let _ = std::fs::remove_file(&log);
    }

    #[test]
    fn a_failed_install_says_why_and_progress_reads_only_what_pacman_wrote_since() {
        let mut sh = crate::shell::tests::shell();
        let log = scratch_log("fail", "[2026-01-01T00:00:00+0000] [ALPM] transaction started\n[2026-01-01T00:00:01+0000] [ALPM] installed old (1-1)\n[2026-01-01T00:00:02+0000] [ALPM] transaction completed\n");
        assert!(sh.package_install_open_with(Source::Repo(vec!["a".into(), "b".into()]), fake_fail, log.clone()));
        tap(&mut sh, 1);
        // the old transaction is before the start offset: it does not count
        assert_eq!(read_log_since(&log, sh.package_session.as_ref().unwrap().log_start), "");
        wait_for_the_worker(&mut sh);
        let session = sh.package_session.as_ref().unwrap();
        assert_eq!(session.sheet.step(), Step::Finished { ok: false });
        assert!(session.sheet.detail().contains("failed to commit transaction"));
        let _ = std::fs::remove_file(&log);
    }

    #[test]
    fn the_log_since_the_start_drives_the_progress() {
        let ui = crate::shell::tests::ui();
        let log = scratch_log("progress", "before\n");
        let start = std::fs::metadata(&log).unwrap().len();
        let mut text = std::fs::read_to_string(&log).unwrap();
        text.push_str(LOG);
        std::fs::write(&log, text).unwrap();
        let since = read_log_since(&log, start);
        assert_eq!(since, LOG);
        let mut sheet = Sheet::new(&ui, vec!["a".into(), "b".into()]).unwrap();
        sheet.answer(&ui, Button::Install);
        sheet.update(&ui, &since);
        assert_eq!(sheet.step(), Step::Installing { fraction: 0.5 });
        assert_eq!(read_log_since(Path::new("/nonexistent/pacman.log"), 0), "");
        let _ = std::fs::remove_file(&log);
    }

    #[test]
    fn only_package_files_and_store_links_open_the_sheet() {
        let mut sh = crate::shell::tests::shell();
        let notes = vec![PathBuf::from("/home/ola/notes.txt"), PathBuf::from("/home/ola/pic.png")];
        assert!(!sh.package_open_files(&notes));
        assert!(!sh.package_showing());
        let mixed = vec![PathBuf::from("/home/ola/notes.txt"), PathBuf::from("/home/ola/Downloads/vim-9.1-1-x86_64.pkg.tar.zst")];
        assert!(sh.package_open_files(&mixed));
        assert_eq!(sh.package_session.as_ref().unwrap().sheet.packages(), ["vim".to_string()]);
        sh.package_session = None;
        let reply = sh.workspace_open_url("bifrost-package:install/htop,vim", 0.0);
        assert!(matches!(reply.0, bifrost_proto::WorkspaceStatus::Ok));
        assert_eq!(sh.package_session.as_ref().unwrap().sheet.packages().len(), 2);
        sh.package_session = None;
        let refused = sh.workspace_open_url("bifrost-package:install/--force", 0.0);
        assert!(!matches!(refused.0, bifrost_proto::WorkspaceStatus::Ok));
        assert!(!sh.package_showing());
    }
}
