//! Open With: the apps that declare the focused file's type, as the shell's launch service names them, the default
<<<<<<< HEAD
//! marked and first. A click on an app opens the file in it (`workspace::open_with`, `NSWorkspace.open(_:withApplicationAt:)`);
//! the default itself is chosen in Settings > General > Default Apps. "Other…" at the end lists every installed app,
//! as the reference's Open With > Other… does, for a file no installed app declares (an RTF only TextEdit names).
||||||| f62164531
//! marked and first. A click on an app opens the file in it (`workspace::open_with`, `NSWorkspace.open(_:withApplicationAt:)`);
//! the default itself is chosen in Settings > General > Default Apps.
=======
//! marked and first. A click on an app opens the file in it (`workspace::open_with`, the reference workspace's open-with call);
//! the default itself is chosen in Settings > General > Default Apps.
>>>>>>> handoff/fix-names
use crate::*;
use bifrost_app::launch_services::Candidate;

/// The panel's top in window points (the Get Info position), the first app row's top below it, and the row pitch.
const PANEL_TOP: f32 = 72.;
const ROWS_TOP: f32 = 62.;
const ROW_H: f32 = 26.;
/// The last row of the declared apps: it switches the panel to every installed app.
const OTHER: &str = "Other…";
/// The last row of a page of the installed apps when they do not all fit the window: it shows the next page.
const MORE: &str = "More…";
/// Fewest rows a page of the installed apps shows, however short the window is (including the More row).
const MIN_FIT: usize = 4;

/// How many rows fit in a window `h` points tall, between the panel's top and the hint under the rows.
pub(crate) fn fit_rows(h: f32) -> usize {
    (((h - 16. - PANEL_TOP - ROWS_TOP - 40.) / ROW_H).max(0.) as usize).max(MIN_FIT)
}

/// Every installed app under the shell data folder `root` as an Open With candidate, by name. These are the apps the
/// shell opens a file with (`workspace::open_with`): packages that are apps, not system packages or services.
pub(crate) fn installed_apps(root: &Path) -> Vec<Candidate> {
    let mut apps: Vec<Candidate> = springboard::shell::apphost::scan(root)
        .into_iter()
        .map(|a| Candidate { app: a.manifest.id, name: a.manifest.name, is_default: false })
        .collect();
    apps.sort_by_key(|c| c.name.to_lowercase());
    apps
}

/// The Open With panel: the file it is for, and the launch service's answer once it arrives (None while it waits).
pub(crate) struct OpenWith {
    pub(crate) path: PathBuf,
    pub(crate) name: String,
    pub(crate) apps: Option<Result<Vec<Candidate>, String>>,
    /// Every installed app, once "Other…" was chosen (the panel then lists these instead of the declared apps).
    pub(crate) all: Option<Vec<Candidate>>,
    /// The first of `all` shown, when they take more than one page.
    pub(crate) first: usize,
}

impl OpenWith {
    /// Every row of the panel's list: the declared apps with the default marked and "Other…" last, or (after "Other…")
    /// every installed app; or the one message that explains why there are none.
    fn rows(&self) -> Vec<String> {
        if let Some(all) = &self.all {
            return if all.is_empty() { vec!["No apps are installed.".into()] } else { all.iter().map(|a| a.name.clone()).collect() };
        }
        match &self.apps {
            None => vec!["Looking up apps…".into()],
            Some(Err(e)) => vec![e.clone()],
            Some(Ok(apps)) => {
                let mut rows: Vec<String> = if apps.is_empty() {
                    vec!["No app opens this file.".into()]
                } else {
                    apps.iter().map(|a| if a.is_default { format!("{} (Default)", a.name) } else { a.name.clone() }).collect()
                };
                rows.push(OTHER.into());
                rows
            }
        }
    }

    /// Whether the installed apps take more than the `fit` rows the window has room for.
    fn paged(&self, fit: usize) -> bool {
        self.all.as_ref().is_some_and(|a| a.len() > fit)
    }

    /// The lines the panel shows under the file name when `fit` rows fit: all of them, or one page of the installed
    /// apps closed by a "More…" row.
    pub(crate) fn lines(&self, fit: usize) -> Vec<String> {
        let mut rows = self.rows();
        if self.paged(fit) {
            let start = self.first.min(rows.len());
            rows = rows[start..(start + fit - 1).min(rows.len())].to_vec();
            rows.push(MORE.into());
        }
        rows
    }

    /// The app of row `i` when the answer has one there (the rows are the apps in the answer's order).
    pub(crate) fn app_at(&self, i: usize, fit: usize) -> Option<&Candidate> {
        match (&self.all, &self.apps) {
            (Some(_), _) if self.more_row(fit) == Some(i) => None,
            (Some(all), _) => all.get(self.first + i),
            (None, Some(Ok(apps))) => apps.get(i),
            _ => None,
        }
    }

    /// The row that switches to every installed app, when the declared apps are showing.
    pub(crate) fn other_row(&self) -> Option<usize> {
        (self.all.is_none() && matches!(self.apps, Some(Ok(_)))).then(|| self.rows().len() - 1)
    }

    /// The row that shows the next page of the installed apps, when they take more than one page.
    pub(crate) fn more_row(&self, fit: usize) -> Option<usize> {
        self.paged(fit).then(|| self.lines(fit).len() - 1)
    }

    /// The next page of the installed apps, from the first one again after the last page.
    pub(crate) fn next_page(&mut self, fit: usize) {
        let len = self.all.as_ref().map_or(0, Vec::len);
        self.first += fit.saturating_sub(1).max(1);
        if self.first >= len {
            self.first = 0;
        }
    }

    /// The row under the point `dy` below the panel's top, if it is one of the lines.
    pub(crate) fn row_at(&self, dy: f32, fit: usize) -> Option<usize> {
        let first = dy - ROWS_TOP;
        if first < 0. {
            return None;
        }
        let i = (first / ROW_H) as usize;
        (i < self.lines(fit).len()).then_some(i)
    }

    /// A number that changes with the answer, for the damage check.
    pub(crate) fn state(&self) -> usize {
        self.rows().len() + usize::from(self.apps.is_some()) + self.first * 1000 + self.all.as_ref().map_or(0, |_| 1_000_000)
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
        self.open_with = Some(OpenWith { path: entry.path.clone(), name: entry.name.clone(), apps: None, all: None, first: 0 });
        self.sys.effects.push(Effect::OpenWithApps(entry.path));
        self.revision += 1;
    }

    /// A click while the panel is up: an app row opens the file in that app and closes the panel; "Other…" lists every
    /// installed app, and "More…" shows the next page of them. False for a click outside the rows (the caller closes the
    /// panel then).
    pub(crate) fn open_with_click(&mut self, x: f32, y: f32) -> bool {
        let Some(panel) = &self.open_with else { return false };
        let left = self.side() + 16.;
        if x < left || x > self.w - 16. {
            return false;
        }
        let fit = fit_rows(self.h);
        let Some(i) = panel.row_at(y - PANEL_TOP, fit) else { return false };
        if panel.other_row() == Some(i) {
            let all = installed_apps(&springboard::shell::apphost::root());
            if let Some(panel) = self.open_with.as_mut() {
                panel.all = Some(all);
                panel.first = 0;
            }
            self.revision += 1;
            return true;
        }
        if panel.more_row(fit) == Some(i) {
            if let Some(panel) = self.open_with.as_mut() {
                panel.next_page(fit);
            }
            self.revision += 1;
            return true;
        }
        let Some(app) = panel.app_at(i, fit).map(|c| c.app.clone()) else { return false };
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
        let lines = panel.lines(fit_rows(self.h));
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
            if panel.all.is_some() { "Click an app to open the file with it." } else { "Click an app to open the file with it. Choose the default app in Settings > General > Default Apps." },
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

    const FIT: usize = 16;

    fn app(id: &str, name: &str, is_default: bool) -> Candidate {
        Candidate { app: id.into(), name: name.into(), is_default }
    }

    fn panel(apps: Option<Result<Vec<Candidate>, String>>) -> OpenWith {
        OpenWith { path: PathBuf::from("/tmp/x/photo.jpg"), name: "photo.jpg".into(), apps, all: None, first: 0 }
    }

    fn every(n: usize) -> Vec<Candidate> {
        (0..n).map(|i| app(&format!("is.x.app{i}"), &format!("App {i}"), false)).collect()
    }

    #[test]
    fn the_default_app_is_marked_and_the_order_of_the_answer_is_kept() {
        let p = panel(Some(Ok(vec![app("bifrost.photos", "Photos", true), app("bifrost.files", "Files", false)])));
        assert_eq!(p.lines(FIT), vec!["Photos (Default)".to_string(), "Files".to_string(), "Other…".to_string()]);
        assert_eq!(p.state(), 4);
    }

    #[test]
    fn while_the_answer_is_pending_the_panel_says_so() {
        let p = panel(None);
        assert_eq!(p.lines(FIT), vec!["Looking up apps…".to_string()]);
        assert_eq!(p.state(), 1);
        assert_eq!(p.other_row(), None, "no Other… before the answer");
    }

    #[test]
    fn an_empty_answer_still_offers_other_and_an_error_gives_one_line() {
        let p = panel(Some(Ok(vec![])));
        assert_eq!(p.lines(FIT), vec!["No app opens this file.".to_string(), "Other…".to_string()]);
        assert_eq!(p.other_row(), Some(1), "an RTF nobody declares can still be opened with any app");
        assert!(p.app_at(0, FIT).is_none());
        let e = panel(Some(Err("no service launch".into())));
        assert_eq!(e.lines(FIT), vec!["no service launch".to_string()]);
        assert_eq!(e.other_row(), None);
    }

    #[test]
    fn rows_map_to_the_apps_of_the_answer_and_other() {
        let p = panel(Some(Ok(vec![app("is.olibuijr.preview.desktop", "Preview", true), app("bifrost.photos", "Photos", false)])));
        assert_eq!(p.row_at(ROWS_TOP + 1., FIT), Some(0));
        assert_eq!(p.row_at(ROWS_TOP + ROW_H + 1., FIT), Some(1));
        assert_eq!(p.row_at(ROWS_TOP - 1., FIT), None, "the title is not a row");
        assert_eq!(p.row_at(ROWS_TOP + 2. * ROW_H + 1., FIT), Some(2), "Other… follows the last app");
        assert_eq!(p.row_at(ROWS_TOP + 3. * ROW_H + 1., FIT), None, "below Other…");
        assert_eq!(p.app_at(1, FIT).map(|c| c.app.as_str()), Some("bifrost.photos"));
        assert!(p.app_at(2, FIT).is_none(), "Other… is not an app");
        assert_eq!(p.other_row(), Some(2));
        // a message line is not an app
        let waiting = panel(None);
        assert_eq!(waiting.row_at(ROWS_TOP + 1., FIT), Some(0));
        assert!(waiting.app_at(0, FIT).is_none());
    }

    #[test]
    fn a_candidate_without_a_default_flag_is_not_marked() {
        let p = panel(Some(Ok(vec![app("a.b", "A", false)])));
        assert_eq!(p.lines(FIT), vec!["A".to_string(), "Other…".to_string()]);
    }

    #[test]
    fn other_lists_every_installed_app_and_each_row_opens_that_app() {
        let mut p = panel(Some(Ok(vec![app("is.olibuijr.textedit.desktop", "TextEdit", true)])));
        p.all = Some(vec![app("is.olibuijr.calculator.desktop", "Calculator", false), app("is.olibuijr.textedit.desktop", "TextEdit", false)]);
        assert_eq!(p.lines(FIT), vec!["Calculator".to_string(), "TextEdit".to_string()]);
        assert_eq!(p.other_row(), None, "Other… is gone once it was chosen");
        assert_eq!(p.more_row(FIT), None, "everything fits");
        assert_eq!(p.app_at(0, FIT).map(|c| c.app.as_str()), Some("is.olibuijr.calculator.desktop"));
        assert_eq!(p.app_at(1, FIT).map(|c| c.app.as_str()), Some("is.olibuijr.textedit.desktop"));
        assert!(p.app_at(2, FIT).is_none());
        p.all = Some(vec![]);
        assert_eq!(p.lines(FIT), vec!["No apps are installed.".to_string()]);
        assert!(p.app_at(0, FIT).is_none());
    }

    #[test]
    fn more_installed_apps_than_rows_are_paged_and_wrap_round() {
        let mut p = panel(Some(Ok(vec![])));
        p.all = Some(every(7));
        let fit = 4;
        // three apps and More…, then three, then the last one and More… (which wraps to the first page)
        assert_eq!(p.lines(fit), vec!["App 0".to_string(), "App 1".to_string(), "App 2".to_string(), "More…".to_string()]);
        assert_eq!(p.more_row(fit), Some(3));
        assert!(p.app_at(3, fit).is_none(), "More… is not an app");
        p.next_page(fit);
        assert_eq!(p.lines(fit), vec!["App 3".to_string(), "App 4".to_string(), "App 5".to_string(), "More…".to_string()]);
        assert_eq!(p.app_at(0, fit).map(|c| c.name.as_str()), Some("App 3"));
        p.next_page(fit);
        assert_eq!(p.lines(fit), vec!["App 6".to_string(), "More…".to_string()]);
        assert_eq!(p.more_row(fit), Some(1));
        assert!(p.app_at(1, fit).is_none());
        let before = p.state();
        p.next_page(fit);
        assert_eq!(p.first, 0);
        assert_eq!(p.lines(fit)[0], "App 0");
        assert_ne!(p.state(), before, "a page turn changes the damage state");
    }

    #[test]
    fn the_rows_that_fit_follow_the_window_height_and_never_drop_below_a_floor() {
        assert!(fit_rows(900.) > fit_rows(630.));
        assert_eq!(fit_rows(10.), MIN_FIT);
        let rows = fit_rows(630.) as f32;
        assert!(PANEL_TOP + ROWS_TOP + rows * ROW_H + 40. <= 630., "the card ends inside the window");
    }

    fn install(root: &Path, id: &str, name: &str, kind: &str) {
        let dir = root.join("apps").join(id).join("current");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("manifest.toml"), format!("id = \"{id}\"\nname = \"{name}\"\nversion = \"1.0.0\"\nexec = \"run\"\nkind = \"{kind}\"\n")).unwrap();
        std::fs::write(dir.join("run"), "#!/bin/sh\n").unwrap();
    }

    #[test]
    fn installed_apps_are_every_app_by_name_without_system_packages() {
        let root = std::env::temp_dir().join(format!("files-openwith-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        install(&root, "is.x.zed", "Zed Writer", "app");
        install(&root, "is.x.alpha", "alpha Viewer", "system-app");
        install(&root, "is.x.daemon", "Daemon", "service");
        let apps = installed_apps(&root);
        let names: Vec<&str> = apps.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["alpha Viewer", "Zed Writer"], "by name, ignoring case; a service is no app to open a file with");
        assert_eq!(apps[1].app, "is.x.zed");
        assert!(apps.iter().all(|c| !c.is_default));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn clicking_other_keeps_the_panel_open_on_the_installed_apps() {
        let mut files = crate::tests::fixture(vec![]);
        files.open_with = Some(panel(Some(Ok(vec![app("is.olibuijr.preview.desktop", "Preview", true)]))));
        let x = files.side() + 40.;
        // row 0 is Preview, row 1 is Other…
        let y = PANEL_TOP + ROWS_TOP + ROW_H + 1.;
        assert!(files.open_with_click(x, y));
        let p = files.open_with.as_ref().expect("the panel stays up");
        assert!(p.all.is_some(), "now listing every installed app");
        assert!(files.sys.effects.is_empty(), "nothing was opened yet");
    }

    #[test]
    fn clicking_an_app_row_opens_the_file_in_it() {
        let mut files = crate::tests::fixture(vec![]);
        let mut p = panel(Some(Ok(vec![])));
        p.all = Some(vec![app("is.olibuijr.x", "X", false), app("is.olibuijr.textedit.desktop", "TextEdit", false)]);
        files.open_with = Some(p);
        let x = files.side() + 40.;
        assert!(files.open_with_click(x, PANEL_TOP + ROWS_TOP + ROW_H + 1.));
        assert!(files.open_with.is_none(), "choosing an app closes the panel");
        assert!(matches!(files.sys.effects.last(), Some(Effect::OpenFiles(paths, Some(app))) if paths == &[PathBuf::from("/tmp/x/photo.jpg")] && app == "is.olibuijr.textedit.desktop"));
    }
}
