//! Open With: the apps that declare the focused file's type, as the shell's launch service names them, the default
//! marked and first. A click on an app opens the file in it (`workspace::open_with`, `NSWorkspace.open(_:withApplicationAt:)`);
//! the default itself is chosen in Settings > General > Default Apps.
use crate::*;
use bifrost_app::launch_services::Candidate;

/// The panel's top in window points (the Get Info position), the first app row's top below it, and the row pitch.
const PANEL_TOP: f32 = 72.;
const ROWS_TOP: f32 = 62.;
const ROW_H: f32 = 26.;

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

    /// The app of row `i` when the answer has one there (the rows are the apps in the answer's order).
    pub(crate) fn app_at(&self, i: usize) -> Option<&Candidate> {
        match &self.apps {
            Some(Ok(apps)) => apps.get(i),
            _ => None,
        }
    }

    /// The row under the point `dy` below the panel's top, if it is one of the lines.
    pub(crate) fn row_at(&self, dy: f32) -> Option<usize> {
        let first = dy - ROWS_TOP;
        if first < 0. {
            return None;
        }
        let i = (first / ROW_H) as usize;
        (i < self.lines().len()).then_some(i)
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

    /// A click while the panel is up: an app row opens the file in that app and closes the panel. False for a click
    /// outside the rows (the caller closes the panel then).
    pub(crate) fn open_with_click(&mut self, x: f32, y: f32) -> bool {
        let Some(panel) = &self.open_with else { return false };
        let left = self.side() + 16.;
        if x < left || x > self.w - 16. {
            return false;
        }
        let Some(app) = panel.row_at(y - PANEL_TOP).and_then(|i| panel.app_at(i)).map(|c| c.app.clone()) else { return false };
        let path = panel.path.clone();
        self.open_with = None;
        self.sys.effects.push(Effect::OpenFiles(vec![path], Some(app)));
        self.revision += 1;
        true
    }

    /// The Open With panel, under the Get Info position.
    pub(crate) fn paint_open_with(&self, ui: &Ui, s: &mut Surface) {
        let Some(panel) = &self.open_with else {
            return;
        };
        let (w, x, y) = (self.w, self.side() + 16., PANEL_TOP);
        let lines = panel.lines();
        let card_h = ROWS_TOP + lines.len() as f32 * ROW_H + 40.;
        rect(s, ui, x, y, w - x - 16., card_h, CARD_BG.now());
        text(s, ui, "Open With", 17., LABEL.now(), x + 20., y + 18., w - x - 56.);
        text(s, ui, &panel.name, 12., LABEL_2.now(), x + 20., y + 40., w - x - 56.);
        for (i, line) in lines.iter().enumerate() {
            text(s, ui, line, 13., LABEL.now(), x + 20., y + ROWS_TOP + i as f32 * ROW_H, w - x - 56.);
        }
        text(
            s,
            ui,
            "Click an app to open the file with it. Choose the default app in Settings > General > Default Apps.",
            11.,
            LABEL_2.now(),
            x + 20.,
            y + ROWS_TOP + lines.len() as f32 * ROW_H + 10.,
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
    fn rows_map_to_the_apps_of_the_answer_and_nothing_else() {
        let p = panel(Some(Ok(vec![app("is.olibuijr.preview.desktop", "Preview", true), app("bifrost.photos", "Photos", false)])));
        assert_eq!(p.row_at(ROWS_TOP + 1.), Some(0));
        assert_eq!(p.row_at(ROWS_TOP + ROW_H + 1.), Some(1));
        assert_eq!(p.row_at(ROWS_TOP - 1.), None, "the title is not a row");
        assert_eq!(p.row_at(ROWS_TOP + 2. * ROW_H + 1.), None, "below the last app");
        assert_eq!(p.app_at(1).map(|c| c.app.as_str()), Some("bifrost.photos"));
        // a message line is not an app
        let waiting = panel(None);
        assert_eq!(waiting.row_at(ROWS_TOP + 1.), Some(0));
        assert!(waiting.app_at(0).is_none());
    }

    #[test]
    fn a_candidate_without_a_default_flag_is_not_marked() {
        let p = panel(Some(Ok(vec![app("a.b", "A", false)])));
        assert_eq!(p.lines(), vec!["A".to_string()]);
    }
}
