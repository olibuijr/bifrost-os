//! The photo pick: what an app asked the person to choose (the photos service `picker` call) and what the person has
//! chosen so far. The sheet that shows it is `photo_pick_sheet`; the service answers the app when the pick ends.
//!
//! A pick is per app. `start` stores it as waiting; the shell's tick hands it to the sheet (`take_waiting`), which owns
//! it while the person chooses. Ending the pick (`finish` or `cancel`) queues the result (`take_finished`), and the
//! photos service pushes it to the app as `picker_did_finish`, granting the app the chosen assets only. The app never
//! sees the library: it learns the identifiers of the pictures the person picked, in the order they were picked.
//!
//! Filters follow the SDK's `PHPickerFilter`: an asset passes when it matches any filter of the pick (none: all). An
//! image filter matches pictures, Live Photo stills included; the live filter matches Live Photo stills only; the video
//! filter matches movies. The selection limit is the SDK's `selection_limit`: 0 means no limit, 1 (the default) makes a
//! new tap replace the earlier choice, and a larger limit refuses a tap on another asset once it is reached.
use bifrost_ui::photo_media::Kind;
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::PathBuf;

/// One asset the picker lists.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    /// The asset identifier the app receives (`services::photos::local_identifier`).
    pub id: String,
    pub path: PathBuf,
    /// The thumbnail cache key (`Photo::key`): changes when the file does.
    pub key: String,
    pub kind: Kind,
    /// A Live Photo still.
    pub live: bool,
    /// When the picture was taken, Unix seconds.
    pub mtime: i64,
}

/// `PHPickerFilter`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Filter {
    Images,
    Videos,
    LivePhotos,
}

impl Filter {
    pub fn from_name(name: &str) -> Option<Filter> {
        Some(match name {
            "images" => Filter::Images,
            "videos" => Filter::Videos,
            "live_photos" => Filter::LivePhotos,
            _ => return None,
        })
    }

    /// The label of the sheet's tab for this filter.
    pub fn label(self) -> &'static str {
        match self {
            Filter::Images => "Photos",
            Filter::Videos => "Videos",
            Filter::LivePhotos => "Live",
        }
    }

    pub fn matches(self, e: &Entry) -> bool {
        match self {
            Filter::Images => e.kind == Kind::Image,
            Filter::Videos => e.kind == Kind::Video,
            Filter::LivePhotos => e.kind == Kind::Image && e.live,
        }
    }
}

/// What a tap on an asset did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Toggle {
    Selected,
    Deselected,
    /// The selection limit is reached: nothing changed.
    Full,
    /// The asset is not listed under the current filter.
    Unknown,
}

/// A pick being made.
#[derive(Clone, Debug)]
pub struct Pick {
    pub app_id: String,
    pub app_name: String,
    /// The filters the app asked for; empty shows everything.
    pub allowed: Vec<Filter>,
    /// 0: no limit.
    pub limit: usize,
    /// Every listed asset, newest first.
    pub entries: Vec<Entry>,
    selected: Vec<String>,
    /// The tab the person chose within `allowed` (`None`: all of them).
    tab: Option<Filter>,
}

impl Pick {
    pub fn new(app_id: &str, app_name: &str, allowed: Vec<Filter>, limit: usize, mut entries: Vec<Entry>) -> Pick {
        // newest first, as the person looks for a recent picture; the sort is stable, so ties keep the library order
        entries.sort_by(|a, b| b.mtime.cmp(&a.mtime));
        Pick { app_id: app_id.to_string(), app_name: app_name.to_string(), allowed, limit, entries, selected: Vec::new(), tab: None }
    }

    fn passes_allowed(&self, e: &Entry) -> bool {
        self.allowed.is_empty() || self.allowed.iter().any(|f| f.matches(e))
    }

    /// The tabs the sheet offers: `None` is "All". With a single kind of asset allowed there is nothing to switch.
    pub fn tabs(&self) -> Vec<Option<Filter>> {
        let kinds: Vec<Filter> = if self.allowed.is_empty() { vec![Filter::Images, Filter::Videos] } else { self.allowed.clone() };
        if kinds.len() < 2 {
            return Vec::new();
        }
        let mut out = vec![None];
        out.extend(kinds.into_iter().map(Some));
        out
    }

    pub fn tab(&self) -> Option<Filter> {
        self.tab
    }

    /// Switches the tab; a tab the pick does not offer is ignored. The selection is kept.
    pub fn set_tab(&mut self, tab: Option<Filter>) {
        if tab.is_none() || self.tabs().contains(&tab) {
            self.tab = tab;
        }
    }

    /// The indexes into `entries` of the assets on show: allowed by the app and by the tab.
    pub fn visible(&self) -> Vec<usize> {
        (0..self.entries.len()).filter(|&i| self.passes_allowed(&self.entries[i]) && self.tab.is_none_or(|t| t.matches(&self.entries[i]))).collect()
    }

    /// Taps the asset `id`: selects it, or deselects it when it is chosen already.
    pub fn toggle(&mut self, id: &str) -> Toggle {
        let Some(entry) = self.entries.iter().find(|e| e.id == id) else { return Toggle::Unknown };
        if !self.passes_allowed(entry) {
            return Toggle::Unknown;
        }
        if let Some(at) = self.selected.iter().position(|s| s == id) {
            self.selected.remove(at);
            return Toggle::Deselected;
        }
        if self.limit == 1 {
            self.selected.clear();
        } else if self.limit > 1 && self.selected.len() >= self.limit {
            return Toggle::Full;
        }
        self.selected.push(id.to_string());
        Toggle::Selected
    }

    /// The chosen identifiers, in the order they were picked.
    pub fn selected(&self) -> &[String] {
        &self.selected
    }

    /// The 1-based place of `id` in the selection.
    pub fn order_of(&self, id: &str) -> Option<usize> {
        self.selected.iter().position(|s| s == id).map(|i| i + 1)
    }

    /// The text of the sheet's count: "2 of 3 selected", "2 selected", or "Select a photo" with none chosen.
    pub fn summary(&self) -> String {
        match (self.selected.len(), self.limit) {
            (0, _) => "Select a photo".to_string(),
            (n, 0) | (n, 1) => format!("{n} selected"),
            (n, limit) => format!("{n} of {limit} selected"),
        }
    }
}

enum Slot {
    /// Stored by `start`, not on screen yet.
    Waiting(Box<Pick>),
    /// The sheet owns the pick.
    Showing,
}

/// The id the shell itself picks under, for the Today View's Photos widget. Its result stays in the shell
/// (`take_widget_result`): no app is told, and the id is not an installable app id.
pub const WIDGET_APP: &str = "shell.widget.photos";

thread_local! {
    /// The thumbnail keys the person chose for the widget, until the shell takes them.
    static WIDGET_RESULT: RefCell<Option<Vec<String>>> = const { RefCell::new(None) };
    static PICKS: RefCell<BTreeMap<String, Slot>> = RefCell::new(BTreeMap::new());
    /// Ended picks not yet delivered: (app id, chosen identifiers; empty when cancelled), oldest first.
    static FINISHED: RefCell<Vec<(String, Vec<String>)>> = RefCell::new(Vec::new());
}

/// Why a pick could not start.
#[derive(Debug, PartialEq, Eq)]
pub enum StartError {
    /// The app's earlier pick is on screen.
    Busy,
}

/// Starts the pick of an app. A pick that has not been shown yet is replaced; one on screen is not.
pub fn start(pick: Pick) -> Result<(), StartError> {
    PICKS.with(|p| {
        let mut picks = p.borrow_mut();
        if matches!(picks.get(&pick.app_id), Some(Slot::Showing)) {
            return Err(StartError::Busy);
        }
        picks.insert(pick.app_id.clone(), Slot::Waiting(Box::new(pick)));
        Ok(())
    })
}

/// True while the app has a pick, waiting or on screen.
pub fn is_open(app_id: &str) -> bool {
    PICKS.with(|p| p.borrow().contains_key(app_id))
}

/// Hands the oldest waiting pick (by app id) to the sheet, which shows it.
pub fn take_waiting() -> Option<Pick> {
    PICKS.with(|p| {
        let mut picks = p.borrow_mut();
        let app = picks.iter().find(|(_, s)| matches!(s, Slot::Waiting(_))).map(|(a, _)| a.clone())?;
        match picks.insert(app, Slot::Showing) {
            Some(Slot::Waiting(pick)) => Some(*pick),
            _ => None,
        }
    })
}

/// Ends the pick with the person's choice (in pick order). The result is queued for the photos service.
pub fn finish(app_id: &str, chosen: Vec<String>) {
    PICKS.with(|p| p.borrow_mut().remove(app_id));
    // the shell's own widget pick has no app to tell (a choice comes by `finish_widget`)
    if app_id != WIDGET_APP {
        FINISHED.with(|f| f.borrow_mut().push((app_id.to_string(), chosen)));
    }
}

/// Ends the widget pick with the thumbnail keys of the chosen assets (`Entry::key`).
pub fn finish_widget(keys: Vec<String>) {
    PICKS.with(|p| p.borrow_mut().remove(WIDGET_APP));
    WIDGET_RESULT.with(|r| *r.borrow_mut() = Some(keys));
}

/// What the person chose for the widget, once; `None` while the sheet is open or after a cancel.
pub fn take_widget_result() -> Option<Vec<String>> {
    WIDGET_RESULT.with(|r| r.borrow_mut().take())
}

/// Ends the pick without a choice.
pub fn cancel(app_id: &str) {
    finish(app_id, Vec::new());
}

/// The ended picks to deliver, oldest first. Each is returned once.
pub fn take_finished() -> Vec<(String, Vec<String>)> {
    FINISHED.with(|f| std::mem::take(&mut *f.borrow_mut()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: &str, kind: Kind, live: bool, mtime: i64) -> Entry {
        Entry { id: id.into(), path: PathBuf::from(format!("/nonexistent/{id}")), key: format!("k-{id}"), kind, live, mtime }
    }

    fn library() -> Vec<Entry> {
        vec![entry("a", Kind::Image, false, 100), entry("b", Kind::Image, true, 300), entry("c", Kind::Video, false, 200), entry("d", Kind::Image, false, 50)]
    }

    fn ids(p: &Pick) -> Vec<&str> {
        p.visible().into_iter().map(|i| p.entries[i].id.as_str()).collect()
    }

    #[test]
    fn the_newest_pictures_come_first() {
        let p = Pick::new("app", "App", vec![], 1, library());
        assert_eq!(ids(&p), ["b", "c", "a", "d"]);
    }

    #[test]
    fn the_apps_filters_limit_what_is_listed() {
        let images = Pick::new("app", "App", vec![Filter::Images], 1, library());
        assert_eq!(ids(&images), ["b", "a", "d"], "a live still is a picture too");
        let videos = Pick::new("app", "App", vec![Filter::Videos], 1, library());
        assert_eq!(ids(&videos), ["c"]);
        let live = Pick::new("app", "App", vec![Filter::LivePhotos], 1, library());
        assert_eq!(ids(&live), ["b"]);
        let both = Pick::new("app", "App", vec![Filter::Videos, Filter::LivePhotos], 1, library());
        assert_eq!(ids(&both), ["b", "c"], "an asset passes when it matches any filter");
    }

    #[test]
    fn tabs_switch_within_the_allowed_kinds() {
        let mut p = Pick::new("app", "App", vec![], 0, library());
        assert_eq!(p.tabs(), [None, Some(Filter::Images), Some(Filter::Videos)]);
        p.set_tab(Some(Filter::Videos));
        assert_eq!(ids(&p), ["c"]);
        p.set_tab(None);
        assert_eq!(ids(&p).len(), 4);
        let mut one = Pick::new("app", "App", vec![Filter::Videos], 0, library());
        assert!(one.tabs().is_empty(), "one kind allowed: nothing to switch");
        one.set_tab(Some(Filter::Images));
        assert_eq!(ids(&one), ["c"], "a tab the app did not allow is ignored");
        let mut two = Pick::new("app", "App", vec![Filter::Videos, Filter::Images], 0, library());
        two.set_tab(Some(Filter::LivePhotos));
        assert_eq!(two.tab(), None);
    }

    #[test]
    fn a_limit_of_one_replaces_the_choice() {
        let mut p = Pick::new("app", "App", vec![], 1, library());
        assert_eq!(p.toggle("a"), Toggle::Selected);
        assert_eq!(p.toggle("b"), Toggle::Selected);
        assert_eq!(p.selected(), ["b"]);
        assert_eq!(p.toggle("b"), Toggle::Deselected);
        assert!(p.selected().is_empty());
    }

    #[test]
    fn a_larger_limit_refuses_the_choice_past_it_and_keeps_pick_order() {
        let mut p = Pick::new("app", "App", vec![], 2, library());
        assert_eq!(p.toggle("d"), Toggle::Selected);
        assert_eq!(p.toggle("a"), Toggle::Selected);
        assert_eq!(p.toggle("b"), Toggle::Full);
        assert_eq!(p.selected(), ["d", "a"]);
        assert_eq!(p.order_of("a"), Some(2));
        assert_eq!(p.summary(), "2 of 2 selected");
        assert_eq!(p.toggle("d"), Toggle::Deselected);
        assert_eq!(p.toggle("b"), Toggle::Selected);
        assert_eq!(p.selected(), ["a", "b"]);
    }

    #[test]
    fn no_limit_takes_every_tap_and_unknown_or_filtered_assets_are_refused() {
        let mut p = Pick::new("app", "App", vec![Filter::Images], 0, library());
        for id in ["a", "b", "d"] {
            assert_eq!(p.toggle(id), Toggle::Selected);
        }
        assert_eq!(p.toggle("c"), Toggle::Unknown, "a movie the app did not ask for cannot be chosen");
        assert_eq!(p.toggle("zzz"), Toggle::Unknown);
        assert_eq!(p.summary(), "3 selected");
    }

    #[test]
    fn a_pick_goes_from_waiting_to_the_sheet_to_the_apps_result() {
        let app = "is.example.pick-model-flow";
        assert!(!is_open(app));
        start(Pick::new(app, "App", vec![], 3, library())).unwrap();
        assert!(is_open(app));
        let mut pick = take_waiting().expect("the pick waits for the sheet");
        assert_eq!(pick.app_id, app);
        assert!(take_waiting().is_none(), "it is on screen now");
        assert_eq!(start(Pick::new(app, "App", vec![], 3, library())), Err(StartError::Busy), "no second pick over the first");
        pick.toggle("a");
        pick.toggle("b");
        finish(app, pick.selected().to_vec());
        assert!(!is_open(app));
        assert_eq!(take_finished(), vec![(app.to_string(), vec!["a".to_string(), "b".to_string()])]);
        assert!(take_finished().is_empty(), "each result is delivered once");
    }

    #[test]
    fn the_widget_pick_answers_the_shell_and_tells_no_app() {
        start(Pick::new(WIDGET_APP, "Photos Widget", vec![Filter::Images], 1, library())).unwrap();
        let mut pick = take_waiting().unwrap();
        pick.toggle("b");
        let keys: Vec<String> = pick.selected().iter().filter_map(|id| pick.entries.iter().find(|e| &e.id == id)).map(|e| e.key.clone()).collect();
        finish_widget(keys);
        assert!(!is_open(WIDGET_APP));
        assert!(take_finished().is_empty(), "no app hears of the widget pick");
        assert_eq!(take_widget_result(), Some(vec!["k-b".to_string()]));
        assert_eq!(take_widget_result(), None, "the result is taken once");
        start(Pick::new(WIDGET_APP, "Photos Widget", vec![], 1, library())).unwrap();
        take_waiting().unwrap();
        cancel(WIDGET_APP);
        assert!(take_finished().is_empty() && take_widget_result().is_none(), "a cancel leaves the widget as it was");
    }

    #[test]
    fn a_waiting_pick_is_replaced_and_cancel_reports_no_choice() {
        let app = "is.example.pick-model-cancel";
        start(Pick::new(app, "App", vec![], 1, library())).unwrap();
        start(Pick::new(app, "App", vec![Filter::Videos], 5, library())).unwrap();
        let pick = take_waiting().unwrap();
        assert_eq!((pick.limit, pick.allowed.clone()), (5, vec![Filter::Videos]));
        cancel(app);
        assert_eq!(take_finished(), vec![(app.to_string(), Vec::new())]);
    }
}
