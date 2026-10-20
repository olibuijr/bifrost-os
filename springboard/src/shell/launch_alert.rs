//! The alert for a downloaded program opened from Files (see `launch_gate`). A program the policy blocks gets
//! "cannot be opened" with OK and Open Settings (Settings > Privacy has Open Anyway); one the policy only wants
//! confirmed gets the question with Cancel and Open, and Open remembers the answer for that code.

use super::launch_gate;
use super::ui::{AlertCard, Ui, Weight, LINK, MAC_BODY_PT};
use super::{AppId, Shell};
use crate::files::fs::Entry;
use crate::gfx::Surface;
use bifrost_core::codereq::Identity;
use bifrost_core::launch_policy::Verdict;
use std::path::{Path, PathBuf};

/// What the alert asks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// The policy refuses; Settings can lift it.
    Blocked,
    /// The policy asks; Open says yes.
    Confirm,
}

/// The alert's buttons: the dismissing one on the left, the main one on the right.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Button {
    Dismiss,
    Main,
}

const BUTTONS: [Button; 2] = [Button::Dismiss, Button::Main];

/// The title: `"Tool" cannot be opened` or `Open "Tool"?`.
pub fn title(kind: Kind, name: &str) -> String {
    match kind {
        Kind::Blocked => format!("\"{name}\" cannot be opened"),
        Kind::Confirm => format!("Open \"{name}\"?"),
    }
}

/// The alert on screen.
pub struct Alert {
    pub kind: Kind,
    pub path: PathBuf,
    /// The sentence under the title (the policy's own words).
    pub sentence: String,
    /// The identity (the hash of the code) the person was shown when the alert was raised: Open remembers and runs
    /// exactly this code, and refuses a file that was replaced since.
    pub assessed: Option<Identity>,
    card: AlertCard,
}

impl Alert {
    pub fn new(ui: &Ui, kind: Kind, path: &Path, sentence: &str) -> Self {
        let name = path.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
        let (dismiss, main) = match kind {
            Kind::Blocked => ("OK", "Open Settings"),
            Kind::Confirm => ("Cancel", "Open"),
        };
        let labels = vec![ui.text(dismiss, MAC_BODY_PT, Weight::Regular, LINK), ui.text(main, MAC_BODY_PT, Weight::Semibold, LINK)];
        Alert { kind, path: path.to_path_buf(), sentence: sentence.to_string(), assessed: None, card: AlertCard::new(ui, &title(kind, &name), sentence, labels) }
    }

    /// The alert for the code whose identity was `assessed` when the question was asked.
    pub fn assessed(mut self, assessed: Identity) -> Self {
        self.assessed = Some(assessed);
        self
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

    /// The centre of the button a key presses: Esc dismisses. Return takes the main (bold) button only on the "cannot be
    /// opened" alert (it opens Settings); it never says Open to a downloaded program, which only a click may run, so a
    /// second Return (or a held one) that is meant for something else cannot start it and remember the grant.
    pub fn key_button(&self, ui: &Ui, w: i32, h: i32, cancel: bool) -> Option<(f32, f32)> {
        if !cancel && self.kind == Kind::Confirm {
            return None;
        }
        self.button(ui, w, h, if cancel { Button::Dismiss } else { Button::Main })
    }

    pub fn draw(&self, s: &mut Surface, ui: &Ui, w: i32, h: i32) {
        self.card.draw(s, ui, w, h);
    }
}

impl Shell {
    /// Files asked to open a program. One with a quarantine record is assessed: allowed ones start, the rest raise the
    /// alert. One without a record (made here) is shown in Files' preview as before.
    pub(super) fn launch_program(&mut self, entry: &Entry, t: f64) {
        let Some(record) = (self.launch_hooks.record)(&entry.path) else {
            self.preview_in_files(entry, t);
            return;
        };
        let path = entry.path.as_path();
        let identity = match launch_gate::identity_of(path) {
            Ok(id) => id,
            Err(e) => {
                eprintln!("springboard: launch {}: {e}", path.display());
                return;
            }
        };
        let policy = launch_gate::load_policy(&self.prefs, &self.data_dir);
        let name = entry.name.as_str();
        match launch_gate::assess(&policy, path, record, &identity) {
            Verdict::Allow => self.run_downloaded(path),
            Verdict::Confirm(reason) => self.raise_launch_alert(Kind::Confirm, path, &reason.explain(name), identity),
            Verdict::Block(reason) => {
                let sentence = reason.explain(name);
                launch_gate::write_blocked_hashed(&self.data_dir, path, &sentence, identity.code_directory_hash.as_deref());
                self.raise_launch_alert(Kind::Blocked, path, &sentence, identity);
            }
        }
    }

    fn run_downloaded(&mut self, path: &Path) {
        if let Err(e) = (self.launch_hooks.run)(path) {
            eprintln!("springboard: launch {}: {e}", path.display());
        }
    }

    fn raise_launch_alert(&mut self, kind: Kind, path: &Path, sentence: &str, assessed: Identity) {
        self.launch_alert = Some(Alert::new(&self.ui, kind, path, sentence).assessed(assessed));
        self.dirty = true;
        self.drawn = super::Drawn::Nothing;
    }

    /// A tap while the alert is on screen: one of its buttons answers it. Taps elsewhere are ignored.
    pub(super) fn launch_alert_touch(&mut self, x: f32, y: f32) {
        let Some(button) = self.launch_alert.as_ref().and_then(|a| a.hit(&self.ui, self.w, self.h, x, y)) else { return };
        let Some(alert) = self.launch_alert.take() else { return };
        let t = self.last_input;
        match (alert.kind, button) {
            (_, Button::Dismiss) => {}
            (Kind::Blocked, Button::Main) => {
                // Settings > Privacy has Open Anyway for the program that was blocked
                self.open(AppId::Settings, t);
                let ui = &self.ui;
                if let Some(st) = self.settings.as_mut() {
                    let _ = st.open_page_id(ui, super::pages::PageId::Privacy, t);
                }
            }
            (Kind::Confirm, Button::Main) => {
                // Open: remembered for the code that was assessed, then started. A file replaced since the question was
                // asked is another program: nothing is remembered or run for it, and opening it again asks again.
                match (alert.assessed.as_ref(), launch_gate::identity_of(&alert.path)) {
                    (Some(assessed), Ok(now)) if now.code_directory_hash == assessed.code_directory_hash => {
                        let mut policy = launch_gate::load_policy(&self.prefs, &self.data_dir);
                        launch_gate::grant(&mut policy, &alert.path, assessed);
                        let _ = launch_gate::save_overrides(&policy, &self.data_dir);
                        self.run_downloaded(&alert.path);
                    }
                    _ => eprintln!("springboard: launch {}: the file changed since it was assessed; not opened", alert.path.display()),
                }
            }
        }
        self.dirty = true;
        self.drawn = super::Drawn::Nothing;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::files::fs::Kind as FileKind;
    use bifrost_core::quarantine::{Quarantine, QuarantineKind};
    use std::os::unix::fs::PermissionsExt;
    use std::sync::Mutex;

    static RAN: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());

    fn received(_: &Path) -> Option<Quarantine> {
        Some(Quarantine::new(QuarantineKind::WebDownload, "Browser", "Browser", 100))
    }

    fn local(_: &Path) -> Option<Quarantine> {
        None
    }

    fn record_run(p: &Path) -> std::io::Result<()> {
        RAN.lock().unwrap().push(p.to_path_buf());
        Ok(())
    }

    fn setup(tag: &str) -> (Shell, PathBuf, Entry) {
        let dir = std::env::temp_dir().join(format!("launch-alert-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("tool");
        std::fs::write(&path, "#!/bin/sh\necho hi\n").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        let mut sh = crate::shell::tests::shell();
        sh.data_dir = dir.clone();
        sh.launch_hooks = launch_gate::Hooks { record: received, run: record_run };
        let entry = Entry { kind: FileKind::Other, name: "tool".into(), path, is_dir: false, size: 18, mtime: 0, items: None };
        (sh, dir, entry)
    }

    fn press(sh: &mut Shell, which: Button) {
        let (x, y) = sh.launch_alert.as_ref().unwrap().button(&sh.ui, sh.w, sh.h, which).unwrap();
        sh.launch_alert_touch(x, y);
    }

    fn ran(path: &Path) -> usize {
        RAN.lock().unwrap().iter().filter(|p| p.as_path() == path).count()
    }

    #[test]
    fn a_blocked_program_raises_the_alert_and_open_settings_shows_open_anyway() {
        let (mut sh, dir, entry) = setup("blocked");
        sh.launch_program(&entry, 1.0);
        let alert = sh.launch_alert.as_ref().expect("the alert is up");
        assert_eq!(alert.kind, Kind::Blocked);
        assert!(alert.sentence.contains("cannot be opened"), "{}", alert.sentence);
        assert_eq!(ran(&entry.path), 0, "a blocked program does not start");
        assert!(launch_gate::read_blocked(&dir).is_some(), "Settings is told which program waits");
        press(&mut sh, Button::Main);
        assert!(sh.launch_alert.is_none());
        let st = sh.settings.as_ref().expect("Settings opened");
        assert_eq!(st.page(), crate::shell::pages::PageId::Privacy);
        assert!(st.row_centre(&sh.ui, "Open Anyway").is_some(), "the Privacy page offers Open Anyway");
        // Open Anyway lifts the block for this code; opening it again starts it
        assert!(launch_gate::open_anyway(&dir, &sh.prefs));
        sh.launch_program(&entry, 2.0);
        assert!(sh.launch_alert.is_none());
        assert_eq!(ran(&entry.path), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn anywhere_asks_and_open_remembers_the_answer() {
        let (mut sh, dir, entry) = setup("confirm");
        sh.prefs.set(launch_gate::SETTING_PREF, "anywhere").unwrap();
        sh.launch_program(&entry, 1.0);
        assert_eq!(sh.launch_alert.as_ref().map(|a| a.kind), Some(Kind::Confirm));
        press(&mut sh, Button::Dismiss);
        assert_eq!(ran(&entry.path), 0, "Cancel starts nothing");
        sh.launch_program(&entry, 2.0);
        press(&mut sh, Button::Main);
        assert_eq!(ran(&entry.path), 1, "Open starts it");
        sh.launch_program(&entry, 3.0);
        assert!(sh.launch_alert.is_none(), "asked once for this code");
        assert_eq!(ran(&entry.path), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn return_never_opens_a_downloaded_program_but_escape_cancels() {
        let (mut sh, dir, entry) = setup("keys");
        sh.prefs.set(launch_gate::SETTING_PREF, "anywhere").unwrap();
        sh.launch_program(&entry, 1.0);
        let alert = sh.launch_alert.as_ref().unwrap();
        assert_eq!(alert.kind, Kind::Confirm);
        assert!(alert.key_button(&sh.ui, sh.w, sh.h, false).is_none(), "no key says Open");
        assert!(alert.key_button(&sh.ui, sh.w, sh.h, true).is_some(), "Escape is Cancel");
        assert!(sh.card_alert_key(false, 2.0).is_empty());
        assert!(sh.launch_alert.is_some(), "Return left the alert up");
        assert_eq!(ran(&entry.path), 0);
        sh.card_alert_key(true, 3.0);
        assert!(sh.launch_alert.is_none(), "Escape answered it");
        assert_eq!(ran(&entry.path), 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn open_runs_and_remembers_the_code_that_was_assessed_not_a_replacement() {
        let (mut sh, dir, entry) = setup("replaced");
        sh.prefs.set(launch_gate::SETTING_PREF, "anywhere").unwrap();
        sh.launch_program(&entry, 1.0);
        assert_eq!(sh.launch_alert.as_ref().map(|a| a.kind), Some(Kind::Confirm));
        // another app swaps the file while the question is up
        std::fs::write(&entry.path, "#!/bin/sh\necho swapped\n").unwrap();
        press(&mut sh, Button::Main);
        assert!(sh.launch_alert.is_none());
        assert_eq!(ran(&entry.path), 0, "the replacement does not run");
        assert!(!dir.join(launch_gate::OVERRIDES_FILE).exists(), "nothing is remembered for it");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn open_anyway_lifts_the_block_only_for_the_code_that_was_blocked() {
        let (mut sh, dir, entry) = setup("anyway-replaced");
        sh.launch_program(&entry, 1.0);
        assert_eq!(sh.launch_alert.as_ref().map(|a| a.kind), Some(Kind::Blocked));
        std::fs::write(&entry.path, "#!/bin/sh\necho swapped\n").unwrap();
        assert!(!launch_gate::open_anyway(&dir, &sh.prefs), "the file is not the one that was blocked");
        assert!(!dir.join(launch_gate::OVERRIDES_FILE).exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_program_made_here_is_not_assessed() {
        let (mut sh, dir, entry) = setup("local");
        sh.launch_hooks.record = local;
        sh.launch_program(&entry, 1.0);
        assert!(sh.launch_alert.is_none());
        assert_eq!(ran(&entry.path), 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_alert_hits_its_buttons() {
        let ui = crate::shell::tests::ui();
        let alert = Alert::new(&ui, Kind::Blocked, Path::new("/home/a/tool"), "\"tool\" cannot be opened because its developer cannot be identified.");
        for which in BUTTONS {
            let (x, y) = alert.button(&ui, 1200, 1920, which).unwrap();
            assert_eq!(alert.hit(&ui, 1200, 1920, x, y), Some(which));
        }
        assert_eq!(title(Kind::Confirm, "tool"), "Open \"tool\"?");
    }
}
