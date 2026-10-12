//! Open With: the apps that can open the focused file, as the shell's launch service names them, the default marked.
//! The list is read-only: the default is chosen in Settings > General > Default Apps, and the shell has no call that
//! opens a file in a chosen app yet, so a row cannot open the file.
use crate::*;
use bifrost_app::launch_services::Candidate;

/// The Open With panel: the file it is for, and the launch service's answer once it arrives (None while it waits).
pub(crate) struct OpenWith {
    pub(crate) path: PathBuf,
    pub(crate) name: String,
    pub(crate) apps: Option<Result<Vec<Candidate>, String>>,
}

impl OpenWith {
    /// The lines the panel shows under the file name: the apps with the default marked, or the one message that
    /// explains why there are none.
    pub(crate) fn lines(&self) -> Vec<String> {
        match &self.apps {
            None => vec!["Looking up apps…".into()],
            Some(Err(e)) => vec![e.clone()],
            Some(Ok(apps)) if apps.is_empty() => vec!["No app opens this file.".into()],
            Some(Ok(apps)) => apps.iter().map(|a| if a.is_default { format!("{} (Default)", a.name) } else { a.name.clone() }).collect(),
        }
    }

    /// A number that changes with the answer, for the damage check.
    pub(crate) fn state(&self) -> usize {
        self.lines().len() + usize::from(self.apps.is_some())
    }
}

impl Files {
    /// The Open With menu item and the Open With panel: asks the launch service for the focused file's apps.
    pub(crate) fn open_with_panel(&mut self) {
        let Some(entry) = self.focus_entry().cloned() else {
            return;
        };
        if entry.is_dir {
            return;
        }
        self.preview = None;
        self.info = None;
        self.tag_panel = false;
        self.open_with = Some(OpenWith { path: entry.path.clone(), name: entry.name.clone(), apps: None });
        self.sys.effects.push(Effect::OpenWithApps(entry.path));
        self.revision += 1;
    }

    /// The Open With panel, under the Get Info position.
    pub(crate) fn paint_open_with(&self, ui: &Ui, s: &mut Surface) {
        let Some(panel) = &self.open_with else {
            return;
        };
        let (w, x, y) = (self.w, self.side() + 16., 72.);
        let lines = panel.lines();
        let card_h = 62. + lines.len() as f32 * 26. + 40.;
        rect(s, ui, x, y, w - x - 16., card_h, CARD_BG.now());
        text(s, ui, "Open With", 17., LABEL.now(), x + 20., y + 18., w - x - 56.);
        text(s, ui, &panel.name, 12., LABEL_2.now(), x + 20., y + 40., w - x - 56.);
        for (i, line) in lines.iter().enumerate() {
            text(s, ui, line, 13., LABEL.now(), x + 20., y + 62. + i as f32 * 26., w - x - 56.);
        }
        text(
            s,
            ui,
            "Choose the default app in Settings > General > Default Apps. Press Escape or click to close.",
            11.,
            LABEL_2.now(),
            x + 20.,
            y + 62. + lines.len() as f32 * 26. + 10.,
            w - x - 56.,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(id: &str, name: &str, is_default: bool) -> Candidate {
        Candidate { app: id.into(), name: name.into(), is_default }
    }

    fn panel(apps: Option<Result<Vec<Candidate>, String>>) -> OpenWith {
        OpenWith { path: PathBuf::from("/tmp/x/photo.jpg"), name: "photo.jpg".into(), apps }
    }

    #[test]
    fn the_default_app_is_marked_and_the_order_of_the_answer_is_kept() {
        let p = panel(Some(Ok(vec![app("bifrost.photos", "Photos", true), app("bifrost.files", "Files", false)])));
        assert_eq!(p.lines(), vec!["Photos (Default)".to_string(), "Files".to_string()]);
        assert_eq!(p.state(), 3);
    }

    #[test]
    fn while_the_answer_is_pending_the_panel_says_so() {
        let p = panel(None);
        assert_eq!(p.lines(), vec!["Looking up apps…".to_string()]);
        assert_eq!(p.state(), 1);
    }

    #[test]
    fn an_empty_answer_and_an_error_each_give_one_line() {
        assert_eq!(panel(Some(Ok(vec![]))).lines(), vec!["No app opens this file.".to_string()]);
        assert_eq!(panel(Some(Err("no service launch".into()))).lines(), vec!["no service launch".to_string()]);
    }

    #[test]
    fn a_candidate_without_a_default_flag_is_not_marked() {
        let p = panel(Some(Ok(vec![app("a.b", "A", false)])));
        assert_eq!(p.lines(), vec!["A".to_string()]);
    }
}
