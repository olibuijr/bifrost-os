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
//!
//! Installing software needs the administrator's password. The confirmation names the package, its version and that a
//! package file is unsigned; Install then shows the password sheet (the one-field prompt of `auth_sheet.rs`), and the
//! password goes to privd's `auth.elevate`, which checks it through PAM and answers with a single-use grant. Only a
//! successful check starts the install, and the install request carries the grant: privd refuses
//! `package.install_file` and `package.install_repo` without one, so the shell alone cannot install. Cancel, Escape and
//! `SHEET_ATTEMPTS` wrong passwords end the sheet with nothing installed and no install request sent. A root shell
//! with no privd (the installer, host tests) has no password to check and installs after the confirmation alone.

use super::forms::{self, Purpose};
use super::keyboard::Out;
use super::pacman_update::Source;
use super::services::local_auth::SHEET_ATTEMPTS;
use super::ui::{AlertCard, Ui, Weight, LINK, MAC_BODY_PT};
use super::wifi_page::{Sheet as PasswordSheet, SheetHit};
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
    /// What the person is about to install: name, version and whether it is signed (`Source::describe`).
    note: String,
    /// Whether Install asks for the administrator's password first.
    password: bool,
    card: AlertCard,
}

impl Sheet {
    /// The confirmation for `packages`. None when there is nothing to install, since there is nothing to confirm.
    pub fn new(ui: &Ui, packages: Vec<String>) -> Option<Self> {
        if !pacman_progress::confirmation_needed(&packages) {
            return None;
        }
        let mut sheet = Sheet { packages, step: Step::Confirm, error: None, note: String::new(), password: false, card: AlertCard::new(ui, "", "", Vec::new()) };
        sheet.card = sheet.build_card(ui);
        Some(sheet)
    }

    /// Tells the confirmation what is being installed (`Source::describe`) and whether Install asks for a password.
    pub fn set_details(&mut self, ui: &Ui, note: String, password: bool) {
        self.note = note;
        self.password = password;
        self.card = self.build_card(ui);
    }

    /// The install cannot go ahead (the password was not accepted, privd could not be reached): the sheet says why and
    /// offers Done. Nothing was installed. Does nothing once the install is running or over.
    pub fn refuse(&mut self, ui: &Ui, why: String) {
        if self.step == Step::Confirm {
            self.step = Step::Finished { ok: false };
            self.error = Some(why);
            self.card = self.build_card(ui);
        }
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
            Step::Confirm => {
                let warning = "Installing runs the setup script of each package as the administrator, so only install software you trust.";
                let ask = if self.password { " You are asked for your password next." } else { "" };
                if self.note.is_empty() {
                    format!("{warning}{ask}")
                } else {
                    format!("{} {warning}{ask}", self.note)
                }
            }
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

/// What the real install runs; `grant` is the single-use proof of the administrator's password (empty for a root shell).
fn run_source(source: &Source, grant: &str) -> Result<(), String> {
    source.run(grant)
}

/// Runs an install with the grant of a fresh authentication and reports how it ended. A function pointer so tests can
/// stand in for pacman.
pub type Runner = fn(&Source, &str) -> Result<(), String>;

/// Why a password check did not give a grant.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AuthFail {
    /// The password was wrong; the person may try again.
    Wrong,
    /// Nothing can be installed now (locked out, no password set, privd not reachable); the text says why.
    Other(String),
}

/// Checks the administrator's password for installing `source` and returns the grant. Runs on a worker thread, since a
/// wrong password makes PAM sleep. A function pointer so tests can stand in for privd.
pub type Authenticator = fn(&Source, &str) -> Result<String, AuthFail>;

/// What the real password check does: privd's `auth.elevate` for installs.
fn real_authenticator(_: &Source, password: &str) -> Result<String, AuthFail> {
    match crate::privd_client::connect().elevate(crate::privd::SCOPE_PACKAGE_INSTALL, password) {
        Ok(grant) => Ok(grant),
        Err(e) => Err(match e.code.as_str() {
            "wrong_password" => AuthFail::Wrong,
            "locked_out" => AuthFail::Other("Too many wrong passwords. Try again later.".to_string()),
            "no_password" => AuthFail::Other("This account has no password yet. Set one in Settings, then install again.".to_string()),
            "locked" | "unauthorized" => AuthFail::Other("The session is locked. Unlock it and try again.".to_string()),
            _ => AuthFail::Other(format!("The password could not be checked: {}", e.message)),
        }),
    }
}

/// The one-field password prompt for installing `what`; `attempt` counts the wrong passwords so far.
pub fn auth_prompt(what: &str, attempt: u32) -> forms::Prompt {
    let mut message = format!("Enter your administrator password to install {what}.");
    if attempt > 0 {
        let left = SHEET_ATTEMPTS.saturating_sub(attempt);
        message = format!("{message}\nThat password is wrong. {left} {} left.", if left == 1 { "try" } else { "tries" });
    }
    forms::Prompt { purpose: Purpose::InstallAuthentication, title: "Enter Password".into(), message, label: "Password", ok: "Install", secure: true, max: 128, initial: String::new() }
}

/// The password sheet on screen: the wrong passwords so far and the check that is running, if any.
struct AuthForm {
    form: PasswordSheet,
    attempt: u32,
    checking: Option<Receiver<Result<String, AuthFail>>>,
}

/// The sheet on screen with what it installs, and the running install.
pub struct Session {
    sheet: Sheet,
    source: Source,
    runner: Runner,
    /// Checks the administrator's password; `None` when there is no password to check (a root shell without privd).
    authenticator: Option<Authenticator>,
    /// The password sheet, between pressing Install and the check's answer.
    auth: Option<AuthForm>,
    log: PathBuf,
    /// Where the log ended when the install started.
    log_start: u64,
    done: Option<Receiver<Result<(), String>>>,
    next_poll: f64,
}

/// What the answer of a password check does to the sheet.
enum AuthOutcome {
    Granted(String),
    Retry(u32),
    Refused(String),
}

impl super::Shell {
    /// Whether the install sheet is on screen. Touches and typing belong to it while it is.
    pub(super) fn package_showing(&self) -> bool {
        self.package_session.is_some()
    }

    /// Opens the install sheet for `source`: the action behind opening a package archive in Files and behind a store
    /// link. False when another install sheet is up or the source cannot be offered (not a package file, bad names).
    pub fn package_install_open(&mut self, source: Source) -> bool {
        // a root shell with no privd has no password to check; every user session asks the administrator
        let authenticator: Option<Authenticator> = if crate::privd_client::connect().is_local() { None } else { Some(real_authenticator) };
        self.package_install_open_with(source, run_source, PathBuf::from(PACMAN_LOG), authenticator)
    }

    fn package_install_open_with(&mut self, source: Source, runner: Runner, log: PathBuf, authenticator: Option<Authenticator>) -> bool {
        if self.package_session.is_some() {
            return false;
        }
        let Ok(names) = source.packages() else { return false };
        let Some(mut sheet) = Sheet::new(&self.ui, names) else { return false };
        sheet.set_details(&self.ui, source.describe(), authenticator.is_some());
        self.package_session = Some(Session { sheet, source, runner, authenticator, auth: None, log, log_start: 0, done: None, next_poll: 0.0 });
        self.dirty = true;
        self.drawn = Drawn::Nothing;
        true
    }

    /// Opens the install sheet for the first package archive among `files`. False when none is one.
    pub(super) fn package_open_files(&mut self, files: &[PathBuf]) -> bool {
        let Some(path) = files.iter().find(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(super::pacman_update::is_package_file)) else { return false };
        self.package_install_open(Source::File(path.clone()))
    }

    /// Follows the password check and a running install: starts the install when the check gave a grant, shows the
    /// result when the worker is done, and the progress from the pacman log between.
    pub(super) fn package_tick(&mut self, t: f64) {
        let Some(session) = self.package_session.as_mut() else { return };
        if let Some(auth) = session.auth.as_mut() {
            let answer = match auth.checking.as_ref().map(|rx| rx.try_recv()) {
                Some(Ok(result)) => Some(result),
                Some(Err(TryRecvError::Disconnected)) => Some(Err(AuthFail::Other("The password could not be checked.".to_string()))),
                Some(Err(TryRecvError::Empty)) | None => None,
            };
            let Some(answer) = answer else { return };
            let outcome = match answer {
                Ok(grant) => AuthOutcome::Granted(grant),
                Err(AuthFail::Wrong) if auth.attempt + 1 < SHEET_ATTEMPTS => AuthOutcome::Retry(auth.attempt + 1),
                Err(AuthFail::Wrong) => AuthOutcome::Refused("The password was not accepted. Nothing was installed.".to_string()),
                Err(AuthFail::Other(why)) => AuthOutcome::Refused(format!("{why} Nothing was installed.")),
            };
            match outcome {
                AuthOutcome::Granted(grant) => {
                    session.auth = None;
                    self.package_run(grant);
                }
                AuthOutcome::Retry(attempt) => {
                    let form = PasswordSheet::form(&self.ui, self.w, t, auth_prompt(&session.sheet.what(), attempt));
                    session.auth = Some(AuthForm { form, attempt, checking: None });
                }
                AuthOutcome::Refused(why) => {
                    session.auth = None;
                    session.sheet.refuse(&self.ui, why);
                }
            }
            self.dirty = true;
            self.drawn = Drawn::Nothing;
            return;
        }
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

    /// A touch on the sheet: Cancel or Done close it, Install asks for the password (or starts the install when there
    /// is none to ask for), and the password sheet's Cancel and Install buttons close it or check the password. Every
    /// touch belongs to the sheet. `t` is the shell clock.
    pub(super) fn package_touch(&mut self, x: f32, y: f32, up: bool, t: f64) {
        if up {
            if self.package_session.as_ref().is_some_and(|s| s.auth.is_some()) {
                let hit = self.package_session.as_ref().and_then(|s| s.auth.as_ref()).and_then(|a| a.form.hit(&self.ui, self.w, self.h, x, y));
                match hit {
                    Some(SheetHit::Cancel) => self.package_session = None,
                    Some(SheetHit::Join) => self.package_auth_submit(),
                    _ => {}
                }
            } else {
                let hit = self.package_session.as_ref().and_then(|s| s.sheet.hit(&self.ui, self.w, self.h, x, y));
                match hit {
                    Some(Button::Cancel) | Some(Button::Done) => self.package_session = None,
                    Some(Button::Install) => self.package_start(t),
                    None => {}
                }
            }
        }
        self.dirty = true;
        self.drawn = Drawn::Nothing;
    }

    /// Typing. Escape closes the sheet while it asks, while it asks for the password and after the install ended. In
    /// the password sheet Return checks the password and other keys edit the field. Everywhere else other keys do
    /// nothing, and Return never installs: only the Install button does.
    pub(super) fn package_type(&mut self, out: Out, t: f64) {
        let Some(session) = self.package_session.as_mut() else { return };
        if let Some(auth) = session.auth.as_mut() {
            if out == Out::Named("Escape") {
                self.package_session = None;
            } else if auth.checking.is_none() {
                if out == Out::Enter {
                    self.package_auth_submit();
                } else {
                    auth.form.edit(&self.ui, out, t);
                }
            }
            self.dirty = true;
            self.drawn = Drawn::Nothing;
            return;
        }
        if out == Out::Named("Escape") && !matches!(session.sheet.step(), Step::Installing { .. }) {
            self.package_session = None;
            self.dirty = true;
            self.drawn = Drawn::Nothing;
        }
    }

    /// The person pressed Install: with a password to check the password sheet comes up and nothing is sent to privd
    /// yet; without one the install starts at once.
    fn package_start(&mut self, t: f64) {
        let Some(session) = self.package_session.as_mut() else { return };
        if session.sheet.step() != Step::Confirm || session.auth.is_some() {
            return;
        }
        if session.authenticator.is_some() {
            let form = PasswordSheet::form(&self.ui, self.w, t, auth_prompt(&session.sheet.what(), 0));
            session.auth = Some(AuthForm { form, attempt: 0, checking: None });
            return;
        }
        self.package_run(String::new());
    }

    /// The typed password is acceptable: check it on a worker thread. The install starts when the check gives a grant
    /// (`package_tick`).
    fn package_auth_submit(&mut self) {
        let Some(session) = self.package_session.as_mut() else { return };
        let Some(authenticator) = session.authenticator else { return };
        let Some(auth) = session.auth.as_mut() else { return };
        if auth.checking.is_some() {
            return;
        }
        let Some((_, password)) = auth.form.form_result() else { return };
        let source = session.source.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        crate::bare::sys::spawn_named("package-auth", move || {
            let _ = tx.send(authenticator(&source, &password));
        });
        auth.checking = Some(rx);
    }

    /// Moves the sheet to the progress and runs the install on a worker thread with `grant`.
    fn package_run(&mut self, grant: String) {
        let Some(session) = self.package_session.as_mut() else { return };
        if !session.sheet.answer(&self.ui, Button::Install) {
            return;
        }
        session.log_start = std::fs::metadata(&session.log).map(|m| m.len()).unwrap_or(0);
        let (tx, rx) = std::sync::mpsc::channel();
        let (source, runner) = (session.source.clone(), session.runner);
        crate::bare::sys::spawn_named("package-install", move || {
            let _ = tx.send(runner(&source, &grant));
        });
        session.done = Some(rx);
    }

    /// Draws the card, or the password sheet, over a dimmed screen.
    pub(super) fn package_draw(&mut self, s: &mut Surface, t: f64) {
        let (w, h) = (self.w, self.h);
        if let Some(session) = self.package_session.as_mut() {
            match session.auth.as_mut() {
                Some(auth) => auth.form.draw(s, &self.ui, w, h, t),
                None => session.sheet.draw(s, &self.ui, w, h),
            }
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

    fn fake_ok(_: &Source, _: &str) -> Result<(), String> {
        Ok(())
    }

    fn fake_fail(_: &Source, _: &str) -> Result<(), String> {
        Err("pacman exited with 1: error: failed to commit transaction".into())
    }

    fn scratch_log(name: &str, text: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("package-sheet-{name}-{}.log", std::process::id()));
        std::fs::write(&path, text).unwrap();
        path
    }

    fn tap(sh: &mut crate::shell::Shell, index: usize) {
        let (x, y) = sh.package_session.as_ref().expect("the sheet is up").sheet.button(&sh.ui, sh.w, sh.h, index);
        sh.package_touch(x, y, true, 1.0);
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
        assert!(sh.package_install_open_with(Source::File(file), fake_ok, log.clone(), None));
        assert!(sh.package_showing());
        assert_eq!(sh.package_session.as_ref().unwrap().sheet.packages(), ["htop".to_string()]);
        assert_eq!(sh.package_session.as_ref().unwrap().sheet.step(), Step::Confirm);
        // a tick, a stray tap and Return all leave it asking; nothing runs
        sh.package_tick(0.0);
        sh.package_touch(2.0, 2.0, true, 1.0);
        sh.package_type(crate::shell::keyboard::Out::Enter, 1.0);
        assert_eq!(sh.package_session.as_ref().unwrap().sheet.step(), Step::Confirm);
        // another install cannot open over it
        assert!(!sh.package_install_open_with(Source::Repo(vec!["vim".into()]), fake_ok, log.clone(), None));
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
        assert!(sh.package_install_open_with(Source::Repo(vec!["htop".into()]), fake_fail, log.clone(), None));
        tap(&mut sh, 0);
        assert!(!sh.package_showing());
        assert!(sh.package_install_open_with(Source::Repo(vec!["htop".into()]), fake_fail, log.clone(), None));
        sh.package_type(crate::shell::keyboard::Out::Named("Escape"), 1.0);
        assert!(!sh.package_showing());
        let _ = std::fs::remove_file(&log);
    }

    #[test]
    fn a_failed_install_says_why_and_progress_reads_only_what_pacman_wrote_since() {
        let mut sh = crate::shell::tests::shell();
        let log = scratch_log("fail", "[2026-01-01T00:00:00+0000] [ALPM] transaction started\n[2026-01-01T00:00:01+0000] [ALPM] installed old (1-1)\n[2026-01-01T00:00:02+0000] [ALPM] transaction completed\n");
        assert!(sh.package_install_open_with(Source::Repo(vec!["a".into(), "b".into()]), fake_fail, log.clone(), None));
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

    // ---- administrator authentication

    use crate::shell::keyboard::Out;
    use std::sync::Mutex;

    /// What the stand-ins for privd were asked, as "auth:<package>" and "run:<package>:<grant>". The statics are shared
    /// by the test threads, so each test uses its own package name and looks only at those entries.
    static CALLS: Mutex<Vec<String>> = Mutex::new(Vec::new());

    fn calls_for(package: &str) -> Vec<String> {
        CALLS.lock().unwrap().iter().filter(|c| c.split(':').nth(1) == Some(package)).cloned().collect()
    }

    fn first_package(source: &Source) -> String {
        source.packages().unwrap().remove(0)
    }

    /// The right password is "right"; the grant names the package so a run can be matched to its check.
    fn fake_authenticator(source: &Source, password: &str) -> Result<String, AuthFail> {
        let package = first_package(source);
        CALLS.lock().unwrap().push(format!("auth:{package}"));
        match password {
            "right" => Ok(format!("grant-{package}")),
            "locked" => Err(AuthFail::Other("Too many wrong passwords. Try again later.".into())),
            _ => Err(AuthFail::Wrong),
        }
    }

    fn recording_runner(source: &Source, grant: &str) -> Result<(), String> {
        CALLS.lock().unwrap().push(format!("run:{}:{grant}", first_package(source)));
        Ok(())
    }

    fn type_password(sh: &mut crate::shell::Shell, text: &str) {
        sh.package_type(Out::Text(text.into()), 1.0);
        sh.package_type(Out::Enter, 1.0);
    }

    /// Ticks until the password check (if one runs) has been answered.
    fn wait_for_the_check(sh: &mut crate::shell::Shell) {
        for k in 0..400 {
            sh.package_tick(10_000.0 + k as f64 * LOG_POLL_MS);
            let Some(session) = sh.package_session.as_ref() else { return };
            if session.auth.as_ref().map_or(true, |a| a.checking.is_none()) {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        panic!("the password check did not end");
    }

    fn open_asking(sh: &mut crate::shell::Shell, package: &str, log: &Path) {
        assert!(sh.package_install_open_with(Source::Repo(vec![package.into()]), recording_runner, log.to_path_buf(), Some(fake_authenticator)));
    }

    #[test]
    fn the_confirmation_names_the_package_its_version_and_that_a_file_is_unsigned() {
        let mut sh = crate::shell::tests::shell();
        let log = scratch_log("describe", "");
        let file = PathBuf::from("/home/ola/Downloads/htop-3.3.0-1-x86_64.pkg.tar.zst");
        assert!(sh.package_install_open_with(Source::File(file), fake_ok, log.clone(), Some(fake_authenticator)));
        let detail = sh.package_session.as_ref().unwrap().sheet.detail();
        assert!(detail.contains("htop 3.3.0-1"), "{detail}");
        assert!(detail.contains("Unsigned"), "{detail}");
        assert!(detail.contains("password"), "the person is told the password comes next: {detail}");
        let repo = Source::Repo(vec!["vim".into()]).describe();
        assert!(repo.contains("vim") && !repo.contains("Unsigned"), "{repo}");
        let _ = std::fs::remove_file(&log);
    }

    #[test]
    fn nothing_reaches_privd_before_the_password_is_checked() {
        let mut sh = crate::shell::tests::shell();
        let log = scratch_log("noprivd", "");
        open_asking(&mut sh, "authzero", &log);
        // the confirmation, a tick, a stray tap and Return send nothing
        sh.package_tick(0.0);
        sh.package_touch(2.0, 2.0, true, 1.0);
        sh.package_type(Out::Enter, 1.0);
        assert!(calls_for("authzero").is_empty());
        // Install only brings up the password sheet: still nothing sent
        tap(&mut sh, 1);
        assert!(sh.package_session.as_ref().unwrap().auth.is_some());
        assert_eq!(sh.package_session.as_ref().unwrap().sheet.step(), Step::Confirm);
        sh.package_tick(2.0);
        assert!(calls_for("authzero").is_empty());
        // Return on an empty field is not an attempt either
        sh.package_type(Out::Enter, 1.0);
        wait_for_the_check(&mut sh);
        assert!(calls_for("authzero").is_empty());
        let _ = std::fs::remove_file(&log);
    }

    #[test]
    fn cancel_in_the_password_sheet_installs_nothing() {
        let mut sh = crate::shell::tests::shell();
        let log = scratch_log("authcancel", "");
        open_asking(&mut sh, "authcancel", &log);
        tap(&mut sh, 1);
        let (x, y) = sh.package_session.as_ref().unwrap().auth.as_ref().unwrap().form.cancel_button(&sh.ui, sh.w, sh.h);
        sh.package_touch(x, y, true, 1.0);
        assert!(!sh.package_showing());
        // Escape does the same, also while the check runs
        open_asking(&mut sh, "authcancel", &log);
        tap(&mut sh, 1);
        type_password(&mut sh, "right");
        sh.package_type(Out::Named("Escape"), 1.0);
        assert!(!sh.package_showing());
        sh.package_tick(3.0);
        std::thread::sleep(std::time::Duration::from_millis(50));
        assert!(!calls_for("authcancel").iter().any(|c| c.starts_with("run:")), "{:?}", calls_for("authcancel"));
        let _ = std::fs::remove_file(&log);
    }

    #[test]
    fn a_wrong_password_installs_nothing_and_three_end_the_sheet() {
        let mut sh = crate::shell::tests::shell();
        let log = scratch_log("authwrong", "");
        open_asking(&mut sh, "authwrong", &log);
        tap(&mut sh, 1);
        type_password(&mut sh, "nope");
        wait_for_the_check(&mut sh);
        assert!(sh.package_session.as_ref().unwrap().auth.is_some(), "a wrong password asks again");
        type_password(&mut sh, "nope");
        wait_for_the_check(&mut sh);
        type_password(&mut sh, "nope");
        wait_for_the_check(&mut sh);
        let session = sh.package_session.as_ref().unwrap();
        assert!(session.auth.is_none());
        assert_eq!(session.sheet.step(), Step::Finished { ok: false });
        assert!(session.sheet.detail().contains("Nothing was installed"));
        assert_eq!(calls_for("authwrong"), vec!["auth:authwrong"; 3], "three checks and no install");
        let _ = std::fs::remove_file(&log);
    }

    #[test]
    fn a_refused_check_says_why_and_installs_nothing() {
        let mut sh = crate::shell::tests::shell();
        let log = scratch_log("authlocked", "");
        open_asking(&mut sh, "authlocked", &log);
        tap(&mut sh, 1);
        type_password(&mut sh, "locked");
        wait_for_the_check(&mut sh);
        let session = sh.package_session.as_ref().unwrap();
        assert_eq!(session.sheet.step(), Step::Finished { ok: false });
        assert!(session.sheet.detail().starts_with("Too many wrong passwords"));
        assert_eq!(calls_for("authlocked"), vec!["auth:authlocked"]);
        let _ = std::fs::remove_file(&log);
    }

    #[test]
    fn the_right_password_sends_exactly_one_install_with_the_grant() {
        let mut sh = crate::shell::tests::shell();
        let log = scratch_log("authright", "");
        open_asking(&mut sh, "authright", &log);
        tap(&mut sh, 1);
        type_password(&mut sh, "wrong-first");
        wait_for_the_check(&mut sh);
        type_password(&mut sh, "right");
        wait_for_the_check(&mut sh);
        assert!(matches!(sh.package_session.as_ref().unwrap().sheet.step(), Step::Installing { .. }));
        wait_for_the_worker(&mut sh);
        assert_eq!(sh.package_session.as_ref().unwrap().sheet.step(), Step::Finished { ok: true });
        assert_eq!(calls_for("authright"), vec!["auth:authright", "auth:authright", "run:authright:grant-authright"]);
        // pressing Return or tapping on the finished sheet cannot install again
        sh.package_type(Out::Enter, 1.0);
        sh.package_tick(20_000.0);
        assert_eq!(calls_for("authright").iter().filter(|c| c.starts_with("run:")).count(), 1);
        let _ = std::fs::remove_file(&log);
    }

    #[test]
    fn a_root_shell_without_privd_installs_after_the_confirmation_alone() {
        let mut sh = crate::shell::tests::shell();
        let log = scratch_log("noauth", "");
        assert!(sh.package_install_open_with(Source::Repo(vec!["authnone".into()]), recording_runner, log.clone(), None));
        tap(&mut sh, 1);
        assert!(sh.package_session.as_ref().unwrap().auth.is_none());
        wait_for_the_worker(&mut sh);
        assert_eq!(calls_for("authnone"), vec!["run:authnone:"]);
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
