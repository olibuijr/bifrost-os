//! The package install sheet: asks the person before pacman installs a list of packages, since each package's install
//! script runs during the install, and then shows how far the install has got. The confirmation rule is
//! `bifrost_core::pacman_progress::confirmation_needed`, the progress is `pacman_progress::progress` over the pacman
//! log.
//!
//! The sheet is a model plus its card: `Sheet::new` asks, `answer` takes the person's choice, and `update` reads the log
//! while the install runs. The shell has no install action to start yet, so nothing opens this sheet.

use super::ui::{AlertCard, Ui, Weight, LINK, MAC_BODY_PT};
use crate::gfx::Surface;
use bifrost_core::pacman_progress;

/// The two buttons of the confirmation, left to right.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Button {
    Cancel,
    Install,
}

/// Where the sheet is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Step {
    /// Waiting for the person to press Install or Cancel.
    Confirm,
    /// The install runs; `fraction` is how far it has got, from 0 to 1.
    Installing { fraction: f32 },
}

pub struct Sheet {
    packages: Vec<String>,
    step: Step,
    card: AlertCard,
}

impl Sheet {
    /// The confirmation for `packages`. None when there is nothing to install, since there is nothing to confirm.
    pub fn new(ui: &Ui, packages: Vec<String>) -> Option<Self> {
        if !pacman_progress::confirmation_needed(&packages) {
            return None;
        }
        let mut sheet = Sheet { packages, step: Step::Confirm, card: AlertCard::new(ui, "", "", Vec::new()) };
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

    fn title(&self) -> String {
        match self.step {
            Step::Confirm => match self.packages.len() {
                1 => format!("Install {}?", self.packages[0]),
                n => format!("Install {n} packages?"),
            },
            Step::Installing { .. } => "Installing".to_string(),
        }
    }

    fn detail(&self) -> String {
        match self.step {
            Step::Confirm => "Installing runs the setup script of each package, so only install software you trust.".to_string(),
            Step::Installing { fraction } => format!("{} percent done.", (fraction * 100.0).round() as u32),
        }
    }

    fn build_card(&self, ui: &Ui) -> AlertCard {
        let buttons = match self.step {
            Step::Confirm => vec![
                ui.text("Cancel", MAC_BODY_PT, Weight::Regular, LINK),
                ui.text("Install", MAC_BODY_PT, Weight::Semibold, LINK),
            ],
            Step::Installing { .. } => Vec::new(),
        };
        AlertCard::new(ui, &self.title(), &self.detail(), buttons)
    }

    /// The button under (x, y) while the person is asked, if any.
    pub fn hit(&self, ui: &Ui, w: i32, h: i32, x: f32, y: f32) -> Option<Button> {
        if self.step != Step::Confirm {
            return None;
        }
        self.card.hit(ui, w, h, x, y).map(|i| if i == 0 { Button::Cancel } else { Button::Install })
    }

    /// The person's choice. Install starts the install and returns true; Cancel returns false and the sheet should
    /// close. Only the confirmation takes an answer, so a second press does nothing.
    pub fn answer(&mut self, ui: &Ui, button: Button) -> bool {
        if self.step != Step::Confirm {
            return true;
        }
        match button {
            Button::Cancel => false,
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

    pub fn draw(&self, s: &mut Surface, ui: &Ui, w: i32, h: i32) {
        self.card.draw(s, ui, w, h);
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
}
