//! The "quit unexpectedly" alert. When an app process ends on its own with a failure (a signal such as a segfault or an
//! abort, or a non-zero exit), the shell writes one line to crash.log, closes that app's window and keeps running. It
//! then shows an alert card with three buttons: Close dismisses it, Report saves the report next to the data and opens
//! Console (its Crash Reports list holds the stacks that systemd-coredump kept), and Reopen starts the app again.
//!
//! The reason, the crash-log line and the report text are pure ([`reason`], [`log_line`], [`report_text`]), so they
//! test without a screen. [`Alert`] only draws its card, as `low_battery::Alert` does. A quit the shell asked for, and
//! an app that exits with 0, are never a crash: `apphost` reports only exits it did not start itself.

use super::apphost;
use super::diag::{self, CrashEntry};
use super::ui::{AlertCard, Ui, Weight, LINK, MAC_BODY_PT};
use super::{AppId, Shell};
use crate::gfx::Surface;

/// The Console app's manifest id (`apps/console-desktop`), which Report opens.
pub const CONSOLE_ID: &str = "is.olibuijr.console.desktop";
/// The most crash-log lines one report quotes for the app.
pub const REPORT_LINES: usize = 8;
/// The file in the data dir that Report writes the report to.
pub const REPORT_FILE: &str = "last_crash_report.txt";

/// Whether an exit status (see `apphost::Status::Exited`) is a failure: any code but 0, except the signals something
/// outside sends to stop an app (SIGHUP, SIGINT, SIGKILL, SIGTERM: a quit, a force quit, the memory killer), which the
/// reference platform does not report as "quit unexpectedly" either.
pub fn is_crash(code: i32) -> bool {
    match signal_of(code) {
        Some(sig) => !matches!(sig, 1 | 2 | 9 | 15),
        None => code != 0,
    }
}

/// The signal a status stands for: `128 + n` for the signal numbers 1 to 64.
pub fn signal_of(code: i32) -> Option<i32> {
    (129..=192).contains(&code).then(|| code - 128)
}

/// The usual POSIX name of a signal, when it has one.
pub fn signal_name(sig: i32) -> Option<&'static str> {
    Some(match sig {
        1 => "SIGHUP",
        2 => "SIGINT",
        3 => "SIGQUIT",
        4 => "SIGILL",
        5 => "SIGTRAP",
        6 => "SIGABRT",
        7 => "SIGBUS",
        8 => "SIGFPE",
        9 => "SIGKILL",
        11 => "SIGSEGV",
        13 => "SIGPIPE",
        15 => "SIGTERM",
        _ => return None,
    })
}

/// "signal 11 (SIGSEGV)", "signal 40", or "exit code 1".
pub fn reason(code: i32) -> String {
    match signal_of(code) {
        Some(sig) => match signal_name(sig) {
            Some(name) => format!("signal {sig} ({name})"),
            None => format!("signal {sig}"),
        },
        None => format!("exit code {code}"),
    }
}

/// The crash.log message for an unexpected exit: `app <id> quit unexpectedly: <reason>`.
pub fn log_line(id: &str, code: i32) -> String {
    format!("app {id} quit unexpectedly: {}", reason(code))
}

/// The alert's title, as the system's own alerts word it: `"Notes" quit unexpectedly`.
pub fn title(name: &str) -> String {
    format!("\"{name}\" quit unexpectedly")
}

/// The text under the title: what happened, and what the buttons do.
pub fn detail(reason: &str) -> String {
    format!("The app stopped because of {reason}. The rest of the desktop is unaffected. Reopen starts it again; Report saves the crash report and opens Console.")
}

/// The report the person can read: the app, the reason, and this app's crash-log lines, oldest first.
pub fn report_text(name: &str, id: &str, reason: &str, lines: &[String]) -> String {
    let mut out = format!("Process: {name} ({id})\nReason: {reason}\n\nCrash log:\n");
    for line in lines {
        out.push_str(line);
        out.push('\n');
    }
    out
}

/// This app's crash-log lines for the report: at most [`REPORT_LINES`] of the newest, oldest first. `entries` are
/// newest first, as `diag::load` returns them.
pub fn app_entries(entries: &[CrashEntry], id: &str) -> Vec<String> {
    let mut lines: Vec<String> = entries.iter().filter(|e| e.signature.contains(id)).take(REPORT_LINES).map(|e| format!("{} {}", e.unix, e.signature)).collect();
    lines.reverse();
    lines
}

/// The alert's buttons, left to right.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Button {
    Close,
    Report,
    Reopen,
}

const BUTTONS: [Button; 3] = [Button::Close, Button::Report, Button::Reopen];

/// The alert on screen: the app's slot, its name, the report Report saves, and the card that draws it.
pub struct Alert {
    pub slot: usize,
    pub report: String,
    card: AlertCard,
}

impl Alert {
    pub fn new(ui: &Ui, slot: usize, name: &str, reason: &str, report: String) -> Self {
        let labels = vec![
            ui.text("Close", MAC_BODY_PT, Weight::Regular, LINK),
            ui.text("Report...", MAC_BODY_PT, Weight::Regular, LINK),
            ui.text("Reopen", MAC_BODY_PT, Weight::Semibold, LINK),
        ];
        Alert { slot, report, card: AlertCard::new(ui, &title(name), &detail(reason), labels) }
    }

    /// The button under (x, y), if any.
    pub fn hit(&self, ui: &Ui, w: i32, h: i32, x: f32, y: f32) -> Option<Button> {
        self.card.hit(ui, w, h, x, y).map(|i| BUTTONS[i])
    }

    /// Centre of a button, for tests.
    pub fn button(&self, ui: &Ui, w: i32, h: i32, which: Button) -> Option<(f32, f32)> {
        let i = BUTTONS.iter().position(|b| *b == which)?;
        Some(self.card.button_centre(ui, w, h, i))
    }

    /// The centre of the button a key presses: Esc closes, Return reopens (the bold button).
    pub fn key_button(&self, ui: &Ui, w: i32, h: i32, cancel: bool) -> Option<(f32, f32)> {
        self.button(ui, w, h, if cancel { Button::Close } else { Button::Reopen })
    }

    pub fn draw(&self, s: &mut Surface, ui: &Ui, w: i32, h: i32) {
        self.card.draw(s, ui, w, h);
    }
}

impl Shell {
    /// An app process ended on its own. A failure is logged to crash.log and gets the alert; any other exit is left
    /// alone. The caller closes the app's window; the shell itself keeps running.
    pub(super) fn app_crashed(&mut self, slot: usize) {
        let code = match self.ext.get(slot).and_then(Option::as_ref).and_then(|h| h.exit_code()) {
            Some(code) if is_crash(code) => code,
            _ => return,
        };
        let registered = apphost::installed();
        let Some(app) = registered.get(slot) else { return };
        let (id, name) = (app.manifest.id.clone(), app.manifest.name.clone());
        let why = reason(code);
        crate::bare::crashlog::append(&self.data_dir.display().to_string(), &log_line(&id, code));
        let entries = diag::load(&self.data_dir);
        let report = report_text(&name, &id, &why, &app_entries(&entries, &id));
        if self.desk_active() {
            // the desktop draws the shell under the app windows, so a modal card there would sit unseen (and
            // repaint the whole screen every frame) until answered: post a notification and keep the report
            let _ = std::fs::write(self.data_dir.join(REPORT_FILE), &report);
            let body = format!("The app stopped because of {why}. The crash report is in Console.");
            self.notify(super::notify::store::Notification::new(AppId::Settings, title(&name), body, crate::bare::sys::wall_time().0));
            return;
        }
        self.crash_alert = Some(Alert::new(&self.ui, slot, &name, &why, report));
        self.dirty = true;
        self.drawn = super::Drawn::Nothing;
    }

    /// A crash `apphost::late_crashes` found after the app's window had already gone: logged, reported and posted as
    /// a notification (there is no window or slot left for the card).
    pub(super) fn app_crashed_late(&mut self, id: &str, code: i32) {
        let name = apphost::installed().iter().find(|a| a.manifest.id == id).map_or_else(|| id.to_string(), |a| a.manifest.name.clone());
        let why = reason(code);
        crate::bare::crashlog::append(&self.data_dir.display().to_string(), &log_line(id, code));
        let entries = diag::load(&self.data_dir);
        let _ = std::fs::write(self.data_dir.join(REPORT_FILE), report_text(&name, id, &why, &app_entries(&entries, id)));
        let body = format!("The app stopped because of {why}. The crash report is in Console.");
        self.notify(super::notify::store::Notification::new(AppId::Settings, title(&name), body, crate::bare::sys::wall_time().0));
        self.dirty = true;
    }

    /// A tap while the alert is on screen: one of its buttons answers it. Taps elsewhere are ignored.
    pub(super) fn crash_alert_touch(&mut self, x: f32, y: f32) {
        let Some(button) = self.crash_alert.as_ref().and_then(|a| a.hit(&self.ui, self.w, self.h, x, y)) else { return };
        let Some(alert) = self.crash_alert.take() else { return };
        let t = self.last_input;
        match button {
            Button::Close => {}
            Button::Report => {
                let _ = std::fs::write(self.data_dir.join(REPORT_FILE), &alert.report);
                if let Some(slot) = apphost::slot_of(CONSOLE_ID) {
                    self.open(AppId::External(slot as u8), t);
                }
            }
            Button::Reopen => self.open(AppId::External(alert.slot as u8), t),
        }
        self.dirty = true;
        self.drawn = super::Drawn::Nothing;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(unix: i64, signature: &str) -> CrashEntry {
        CrashEntry { unix, signature: signature.into(), file: std::path::PathBuf::from("/d/crash.log") }
    }

    #[test]
    fn a_signal_status_names_its_signal() {
        assert!(is_crash(139), "a segfault");
        assert!(is_crash(134), "an abort");
        assert!(!is_crash(0), "a clean exit is not a crash");
        assert!(is_crash(1), "a failing exit code is");
        assert!(!is_crash(143), "SIGTERM is a quit from outside, not a crash");
        assert!(!is_crash(137), "SIGKILL (force quit, the memory killer) is not a crash");
        assert_eq!(signal_of(139), Some(11));
        assert_eq!(signal_of(1), None, "an ordinary exit code is not a signal");
        assert_eq!(reason(139), "signal 11 (SIGSEGV)");
        assert_eq!(reason(134), "signal 6 (SIGABRT)");
        assert_eq!(reason(170), "signal 42", "a signal without a usual name");
        assert_eq!(reason(3), "exit code 3");
    }

    #[test]
    fn the_crash_log_line_names_the_app_and_the_reason() {
        let line = log_line("is.example.notes", 139);
        assert_eq!(line, "app is.example.notes quit unexpectedly: signal 11 (SIGSEGV)");
        let parsed = diag::parse_line(&format!("[1700000000 12:00:00] thread 'shell': {line}")).map(|(_, m)| m);
        assert_eq!(parsed.as_deref(), Some(line.as_str()), "the crash-log reader reads it back");
    }

    #[test]
    fn the_title_and_detail_say_what_happened() {
        assert_eq!(title("Notes"), "\"Notes\" quit unexpectedly");
        let d = detail("signal 11 (SIGSEGV)");
        assert!(d.contains("signal 11 (SIGSEGV)") && d.contains("Reopen") && d.contains("Console"));
    }

    #[test]
    fn the_report_lists_this_apps_newest_lines_oldest_first() {
        let entries = vec![
            entry(300, "app is.example.notes quit unexpectedly: signal 11 (SIGSEGV)"),
            entry(200, "app is.example.mail quit unexpectedly: signal 6 (SIGABRT)"),
            entry(100, "app is.example.notes quit unexpectedly: exit code 2"),
        ];
        let lines = app_entries(&entries, "is.example.notes");
        assert_eq!(lines, vec!["100 app is.example.notes quit unexpectedly: exit code 2", "300 app is.example.notes quit unexpectedly: signal 11 (SIGSEGV)"]);
        let text = report_text("Notes", "is.example.notes", "signal 11 (SIGSEGV)", &lines);
        assert!(text.starts_with("Process: Notes (is.example.notes)\nReason: signal 11 (SIGSEGV)\n"));
        assert!(text.ends_with("300 app is.example.notes quit unexpectedly: signal 11 (SIGSEGV)\n"));
    }

    #[test]
    fn the_report_is_capped() {
        let entries: Vec<CrashEntry> = (0..20).map(|i| entry(i, "app a.b quit unexpectedly: exit code 1")).collect();
        assert_eq!(app_entries(&entries, "a.b").len(), REPORT_LINES);
        assert!(app_entries(&entries, "other.app").is_empty());
    }

    #[test]
    fn the_alert_hits_its_buttons() {
        let ui = crate::shell::tests::ui();
        let alert = Alert::new(&ui, 3, "Notes", "signal 11 (SIGSEGV)", String::new());
        assert_eq!(alert.slot, 3);
        for which in BUTTONS.iter().copied() {
            let (x, y) = alert.button(&ui, 1200, 1920, which).unwrap();
            assert_eq!(alert.hit(&ui, 1200, 1920, x, y), Some(which));
        }
        assert_eq!(alert.hit(&ui, 1200, 1920, 2.0, 2.0), None);
    }
}
