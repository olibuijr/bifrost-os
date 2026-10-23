//! Search: the query running on its own thread and streaming its hits, and the scope bar under the toolbar that says
//! where it looks (This Computer, or the open folder).
use crate::*;
use springboard::shell::ui::SIDEBAR_PLATE;

/// The most hits a search returns; the walk stops there and the status line says so.
pub(crate) const RESULT_CAP: usize = 1000;

/// The scope bar's height under the toolbar (and the tab strip), in points.
pub(crate) const SCOPE_BAR_H: f32 = 32.;
/// One scope plate: its height, the room either side of its text, and the gap between plates (points).
const SCOPE_PLATE_H: f32 = 22.;
const SCOPE_PAD: f32 = SPACE_S + SPACE_XXS;
const SCOPE_GAP: f32 = SPACE_XS;
/// The widest a folder name is drawn in its plate (points).
const SCOPE_NAME_MAX: f32 = 240.;

/// A search under a scope, streaming its hits.
pub(crate) struct Search {
    pub(crate) rx: Receiver<(Vec<Entry>, bool)>,
    pub(crate) cancel: Arc<AtomicBool>,
    pub(crate) query: String,
    /// The `tag:` term: only hits carrying this tag are shown.
    pub(crate) tag: Option<Tag>,
    /// The `date:` term: only hits changed in that span.
    pub(crate) date: Option<DateTerm>,
}

/// What a search field asks for: the name words, a kind, a tag, a date span and a start folder. `kind:images tag:red
/// date:week in:home holiday` searches "holiday" among the images tagged Red, changed this week, from the home folder.
/// A `kind:`, `tag:`, `date:` or `in:` with a value nobody knows stays one of the name words.
#[derive(Debug, PartialEq)]
pub(crate) struct Terms {
    pub(crate) words: String,
    pub(crate) kind: TypeFilter,
    pub(crate) tag: Option<Tag>,
    /// The `date:` term: only items changed in that span.
    pub(crate) date: Option<DateTerm>,
    /// The `in:` term: where the search starts. None leaves it to the scope bar.
    pub(crate) scope: Option<Scope>,
}

/// What a `date:` term names (`date:today`, `date:yesterday`, `date:week`, `date:month`, `date:year`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DateTerm {
    Today,
    Yesterday,
    Week,
    Month,
    Year,
}

impl DateTerm {
    fn named(value: &str) -> Option<DateTerm> {
        Some(match value {
            "today" => DateTerm::Today,
            "yesterday" => DateTerm::Yesterday,
            "week" | "7days" => DateTerm::Week,
            "month" | "30days" => DateTerm::Month,
            "year" => DateTerm::Year,
            _ => return None,
        })
    }
    /// True when an item modified at `mtime` (Unix seconds) falls in the span, counting whole days up to `now`.
    /// An item without a date matches nothing; one from the future counts as today.
    pub(crate) fn matches(self, mtime: i64, now: i64) -> bool {
        if mtime <= 0 {
            return false;
        }
        let age = now.div_euclid(86400) - mtime.div_euclid(86400);
        match self {
            DateTerm::Today => age <= 0,
            DateTerm::Yesterday => age == 1,
            DateTerm::Week => age <= 6,
            DateTerm::Month => age <= 29,
            DateTerm::Year => age <= 364,
        }
    }
}

/// Where a search starts. The scope bar picks `Computer` (the default) or `Folder`; an `in:` term in the query
/// overrides it (`in:folder`, `in:home`, `in:disk`, `in:computer`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Scope {
    /// The open folder and everything under it.
    Folder,
    /// The home folder and everything under it.
    Home,
    /// The whole disk from `/`, without the kernel's virtual trees.
    Disk,
    /// This Computer: the home folder first, then the volumes mounted now, then the rest of the disk.
    #[default]
    Computer,
}

/// What a search walks: the folders it starts from, in order, and the folders it lists but does not enter.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct SearchPlan {
    pub(crate) roots: Vec<PathBuf>,
    pub(crate) skip: Vec<PathBuf>,
}

/// Trees that hold nothing a person searches for: the kernel's views, device nodes and the runtime folder, and the
/// snapshot folders, which repeat every file of the disk many times. They are endless or meaningless as results, so
/// the disk walk does not enter them (mounted volumes are read as roots of their own).
const VIRTUAL_TREES: [&str; 6] = ["/proc", "/sys", "/dev", "/run", "/.snapshots", "/home/.snapshots"];

impl Scope {
    fn named(value: &str) -> Option<Scope> {
        Some(match value {
            "folder" | "here" | "this" => Scope::Folder,
            "home" => Scope::Home,
            "disk" => Scope::Disk,
            "computer" | "everywhere" => Scope::Computer,
            _ => return None,
        })
    }
    /// What the search walks: `open` is the open folder, `home` the home folder and `volumes` the mount points of the
    /// removable volumes mounted now. The home folder comes first for This Computer, where the files are, so its hits
    /// stream in before the rest of the disk is read; the disk walk then leaves out what the earlier roots read.
    pub(crate) fn plan(self, open: &Path, home: &Path, volumes: &[PathBuf]) -> SearchPlan {
        let disk = SearchPlan { roots: vec![PathBuf::from("/")], skip: VIRTUAL_TREES.iter().map(|p| PathBuf::from(*p)).collect() };
        match self {
            Scope::Folder => SearchPlan { roots: vec![open.to_path_buf()], skip: vec![] },
            Scope::Home => SearchPlan { roots: vec![home.to_path_buf()], skip: vec![] },
            Scope::Disk => disk,
            // a home folder of "/" would be walked twice
            Scope::Computer if home == Path::new("/") => disk,
            Scope::Computer => {
                // a volume mounted inside the home folder is read with it
                let volumes: Vec<PathBuf> = volumes.iter().filter(|v| !v.starts_with(home)).cloned().collect();
                let mut roots = vec![home.to_path_buf()];
                roots.extend(volumes.iter().cloned());
                roots.push(PathBuf::from("/"));
                let mut skip = disk.skip;
                skip.push(home.to_path_buf());
                skip.extend(volumes);
                SearchPlan { roots, skip }
            }
        }
    }
}

/// The scope bar's state: the scope the bar picked, and where its plates were drawn, for clicks.
#[derive(Default)]
pub(crate) struct ScopeBar {
    pub(crate) scope: Scope,
    pub(crate) plates: Vec<(Scope, Rect)>,
}

/// In a place with no tree of its own (Recents, a tag, a smart folder) whether a search only filters the listing: under
/// the place's own plate, or when the bar is not offered there; This Computer, Home and Disk walk their trees.
pub(crate) fn filters_in_place(scope: Scope, bar_shown: bool) -> bool {
    scope == Scope::Folder || !bar_shown
}

/// Where each scope plate sits: `(x, width)` pairs left to right, the first after the "Search:" label that starts at
/// `left` and is `label_w` wide. Each plate is as wide as its text (`texts`, in points) plus padding.
pub(crate) fn scope_plates(left: f32, label_w: f32, texts: &[f32]) -> Vec<(f32, f32)> {
    let mut x = left + label_w + SCOPE_GAP;
    texts
        .iter()
        .map(|t| {
            let at = (x, t + 2. * SCOPE_PAD);
            x += at.1 + SCOPE_GAP;
            at
        })
        .collect()
}

pub(crate) fn parse_terms(query: &str) -> Terms {
    let mut words = vec![];
    let mut kind = TypeFilter::All;
    let mut tag = None;
    let mut date = None;
    let mut scope = None;
    for word in query.split_whitespace() {
        let lower = word.to_lowercase();
        if let Some(value) = lower.strip_prefix("date:") {
            if let Some(d) = DateTerm::named(value) {
                date = Some(d);
                continue;
            }
        } else if let Some(value) = lower.strip_prefix("in:") {
            if let Some(s) = Scope::named(value) {
                scope = Some(s);
                continue;
            }
        } else if let Some(value) = lower.strip_prefix("kind:") {
            if let Some(k) = kind_named(value) {
                kind = k;
                continue;
            }
        } else if let Some(value) = lower.strip_prefix("tag:") {
            if let Some(t) = Tag::ALL.into_iter().find(|t| t.label().to_lowercase() == value) {
                tag = Some(t);
                continue;
            }
        }
        words.push(word);
    }
    Terms { words: words.join(" "), kind, tag, date, scope }
}

/// The type filter a `kind:` value names (lower case), or None.
fn kind_named(value: &str) -> Option<TypeFilter> {
    Some(match value {
        "folder" | "folders" => TypeFilter::Folders,
        "document" | "documents" | "doc" | "docs" => TypeFilter::Documents,
        "image" | "images" | "picture" | "pictures" => TypeFilter::Images,
        "video" | "videos" => TypeFilter::Video,
        "audio" | "music" | "song" | "songs" => TypeFilter::Audio,
        "archive" | "archives" => TypeFilter::Archives,
        _ => return None,
    })
}

impl Files {
    pub(crate) fn stop_search(&mut self) {
        if let Some(search) = self.search.take() {
            search.cancel.store(true, Ordering::Relaxed);
        }
        self.hits = None;
    }
    /// The typed query changed: search for it under the scope, or show the folder again.
    pub(crate) fn query_changed(&mut self) {
        // a search that starts from nothing begins at the default scope, not at the one picked last time
        if self.hits.is_none() && self.search.is_none() {
            self.scope_bar.scope = Scope::default();
        }
        self.restart_search();
    }
    /// Searches again for the typed query under the current scope (a new query, or another scope plate), or shows the
    /// folder again when the query is empty. In Recents, Tags and the Trash there is no tree to walk: the listing is
    /// only filtered by the typed name.
    pub(crate) fn restart_search(&mut self) {
        self.stop_search();
        self.scroll = 0.;
        self.selection.clear();
        let q = self.query.trim().to_string();
        let terms = parse_terms(&q);
        let scope = terms.scope.unwrap_or(self.scope_bar.scope);
        if q.is_empty() || self.in_bin() || (self.in_recents() && filters_in_place(scope, self.scope_bar_shown())) {
            self.refresh_rows();
            return;
        }
        let (tx, rx) = feed();
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = cancel.clone();
        let (open, home) = (self.path.clone(), self.home.clone());
        let (search_for, kind) = (terms.words.clone(), terms.kind);
        DispatchQueue::global(Qos::Utility).async_(move || {
            // the mounted volumes are read here, off the UI thread
            let volumes: Vec<PathBuf> = if scope == Scope::Computer {
                springboard::files::volumes::mounted().into_iter().map(|v| v.mount).collect()
            } else {
                vec![]
            };
            let plan = scope.plan(&open, &home, &volumes);
            let mut walk = FileManager::over(&plan.roots, &plan.skip, &search_for, kind, RESULT_CAP);
            let mut out = vec![];
            let mut first = true;
            while !walk.done() && !flag.load(Ordering::Relaxed) {
                let found = walk.step(16);
                // the first update goes out at once, so the folder's own listing stops standing in for the hits; after
                // that only a step that found something is worth a repaint
                if found.is_empty() && !first {
                    continue;
                }
                first = false;
                out.extend(found);
                let _ = tx.send((out.clone(), false));
            }
            let _ = tx.send((out, true));
        });
        self.search = Some(Search { rx, cancel, query: q, tag: terms.tag, date: terms.date });
        self.revision += 1;
    }
    /// A click on a scope plate: search again from that scope.
    pub(crate) fn pick_scope(&mut self, scope: Scope) {
        if self.scope_bar.scope == scope {
            return;
        }
        self.scope_bar.scope = scope;
        self.restart_search();
    }
    /// The scope the search runs under: the query's `in:` term when it has one, else the scope bar's choice.
    pub(crate) fn effective_scope(&self) -> Scope {
        parse_terms(self.query.trim()).scope.unwrap_or(self.scope_bar.scope)
    }
    /// The scope bar shows while there is a query, as in the reference ("Search: This Computer | the place"), also in
    /// Recents, a tag and a smart folder, where the place's plate filters the listing in place. Not in the Trash, the
    /// Applications place or a provider's folder, which are only filtered.
    pub(crate) fn scope_bar_shown(&self) -> bool {
        !self.query.trim().is_empty() && !self.in_bin() && !applications::is_store_dir(&self.path) && !provider::is_path(&self.path)
    }
    /// The scope bar's height now: 0 while it is not shown.
    pub(crate) fn scope_bar_h(&self) -> f32 {
        if self.scope_bar_shown() { SCOPE_BAR_H } else { 0. }
    }
    /// The place a scope names, as the title and the status line put it.
    pub(crate) fn scope_label(&self, scope: Scope) -> String {
        match scope {
            Scope::Computer => "This Computer".into(),
            Scope::Folder => self.folder_title(),
            Scope::Home => self.home_name(),
            Scope::Disk => "Computer".into(),
        }
    }
    /// The scope plate under a point, while the scope bar is shown.
    pub(crate) fn scope_hit(&self, x: f32, y: f32) -> Option<Scope> {
        if !self.scope_bar_shown() {
            return None;
        }
        self.scope_bar.plates.iter().find(|(_, r)| r.contains(x, y)).map(|(scope, _)| *scope)
    }
    /// Draws the scope bar under the toolbar and the tab strip: "Search:" and one plate per scope, the searched one
    /// raised on the neutral plate. The plates' places are kept for `scope_hit`.
    pub(crate) fn paint_scope_bar(&mut self, ui: &Ui, s: &mut Surface) {
        self.scope_bar.plates.clear();
        if !self.scope_bar_shown() {
            return;
        }
        let (x0, y0) = (self.side(), HEADER_H + self.tab_strip_h());
        rect(s, ui, x0, y0 + SCOPE_BAR_H - 1., self.w - x0, 1., SEPARATOR_C.now());
        let pt = MAC_CALLOUT_PT;
        let mid = ui.pt(y0 + (SCOPE_BAR_H - 1.) / 2.) as i32;
        let label = ui.text("Search:", pt, Weight::Semibold, LABEL_2.now());
        let left = x0 + SPACE_L;
        s.blit(&label, ui.pt(left) as i32, mid - label.h as i32 / 2, 255, 255);
        let searched = self.effective_scope();
        let places = [
            (Scope::Computer, self.scope_label(Scope::Computer)),
            (Scope::Folder, format!("\u{201C}{}\u{201D}", self.scope_label(Scope::Folder))),
        ];
        let names: Vec<String> = places.iter().map(|(_, name)| ui.fit(name, pt, ui.pt(SCOPE_NAME_MAX))).collect();
        let widths: Vec<f32> = names.iter().map(|name| ui.measure(name, pt, Weight::Semibold) / ui.pt(1.)).collect();
        let cells = scope_plates(left, label.w as f32 / ui.pt(1.), &widths);
        let plate_y = y0 + (SCOPE_BAR_H - 1. - SCOPE_PLATE_H) / 2.;
        for (((scope, _), name), (x, w)) in places.iter().zip(&names).zip(cells) {
            let on = *scope == searched;
            if on {
                s.fill_rrect(ui.pt(x), ui.pt(plate_y), ui.pt(w), ui.pt(SCOPE_PLATE_H), ui.pt(CELL_RADIUS_PT), SIDEBAR_PLATE.now());
            }
            let im = ui.text(name, pt, Weight::Semibold, if on { LABEL.now() } else { LABEL_2.now() });
            s.blit(&im, ui.pt(x + SCOPE_PAD) as i32, mid - im.h as i32 / 2, 255, 255);
            self.scope_bar.plates.push((*scope, Rect { x, y: plate_y, w, h: SCOPE_PLATE_H }));
        }
    }
    /// Takes the newest hits from the search thread. Returns true when the screen changed.
    pub(crate) fn poll_search(&mut self) -> bool {
        let Some(search) = self.search.as_ref() else { return false };
        let mut latest = None;
        let mut done = false;
        while let Ok((hits, finished)) = search.rx.try_recv() {
            latest = Some(hits);
            done = finished;
            if finished {
                break;
            }
        }
        let Some(mut hits) = latest else { return false };
        let for_query = search.query.clone();
        if for_query != self.query.trim() {
            return false;
        }
        let (tag, date) = (search.tag, search.date);
        if let Some(tag) = tag {
            hits.retain(|h| self.tag_index.has(&h.path, tag));
        }
        if let Some(date) = date {
            let now = std::time::SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
            hits.retain(|h| date.matches(h.mtime, now));
        }
        self.order(&mut hits);
        self.hits = Some(hits);
        self.refresh_rows();
        if done {
            self.search = None;
        }
        self.revision += 1;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use springboard::files::{find::TypeFilter, tags::Tag};

    #[test]
    fn kind_and_tag_terms_are_taken_out_of_the_name_words() {
        assert_eq!(parse_terms("kind:images tag:red holiday"), Terms { words: "holiday".into(), kind: TypeFilter::Images, tag: Some(Tag::ALL[0]), date: None, scope: None });
        assert_eq!(parse_terms("Kind:Docs  report 2026").kind, TypeFilter::Documents, "keys and values ignore case");
        assert_eq!(parse_terms("kind:folders"), Terms { words: String::new(), kind: TypeFilter::Folders, tag: None, date: None, scope: None });
    }

    #[test]
    fn unknown_terms_stay_as_name_words() {
        let t = parse_terms("kind:spreadsheet tag:nope plain");
        assert_eq!(t.words, "kind:spreadsheet tag:nope plain");
        assert_eq!(t.kind, TypeFilter::All);
        assert_eq!(t.tag, None);
        let t = parse_terms("date:someday in:mars plain");
        assert_eq!(t.words, "date:someday in:mars plain");
        assert_eq!((t.date, t.scope), (None, None), "an unknown scope leaves the choice to the scope bar");
    }

    #[test]
    fn date_and_scope_terms_are_taken_out_of_the_name_words() {
        let t = parse_terms("Date:Week in:home report");
        assert_eq!((t.words.as_str(), t.date, t.scope), ("report", Some(DateTerm::Week), Some(Scope::Home)));
        assert_eq!(parse_terms("in:disk").scope, Some(Scope::Disk));
        assert_eq!(parse_terms("in:computer").scope, Some(Scope::Computer));
        assert_eq!(parse_terms("in:here x").scope, Some(Scope::Folder));
        assert_eq!(parse_terms("date:yesterday").date, Some(DateTerm::Yesterday));
    }

    #[test]
    fn date_spans_count_whole_days() {
        let now = 100 * 86400 + 3600; // 01:00 on day 100
        let day = |d: i64| d * 86400 + 80000;
        assert!(DateTerm::Today.matches(day(100), now));
        assert!(DateTerm::Today.matches(day(101), now), "the future counts as today");
        assert!(!DateTerm::Today.matches(day(99), now));
        assert!(DateTerm::Yesterday.matches(day(99), now) && !DateTerm::Yesterday.matches(day(98), now));
        assert!(DateTerm::Week.matches(day(94), now) && !DateTerm::Week.matches(day(93), now));
        assert!(DateTerm::Month.matches(day(71), now) && !DateTerm::Month.matches(day(70), now));
        let year = 400 * 86400 + 3600;
        assert!(DateTerm::Year.matches(day(36), year) && !DateTerm::Year.matches(day(35), year));
        assert!(!DateTerm::Year.matches(0, now), "no date matches nothing");
    }

    fn font_ui() -> Ui {
        let font = springboard::font::Font::parse(Box::leak(std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/SFUI.ttf")).unwrap().into_boxed_slice())).unwrap();
        Ui::new(font, 1.25)
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("files-search-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Walks `roots` to the end, as the search thread does.
    fn walk(roots: &[PathBuf], skip: &[PathBuf], query: &str, cap: usize) -> Vec<Entry> {
        let mut walk = FileManager::over(roots, skip, query, TypeFilter::All, cap);
        let mut out = vec![];
        while !walk.done() {
            out.extend(walk.step(16));
        }
        out
    }

    /// Takes the search thread's hits until it has finished.
    fn finish(files: &mut Files) {
        for _ in 0..500 {
            files.poll_search();
            if files.search.is_none() {
                return;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        panic!("the search did not finish");
    }

    #[test]
    fn scopes_pick_the_start_folders() {
        let (open, home) = (Path::new("/a/b"), Path::new("/h"));
        assert_eq!(Scope::Folder.plan(open, home, &[]).roots, vec![PathBuf::from("/a/b")]);
        assert_eq!(Scope::Home.plan(open, home, &[]).roots, vec![PathBuf::from("/h")]);
        let disk = Scope::Disk.plan(open, home, &[]);
        assert_eq!(disk.roots, vec![PathBuf::from("/")]);
        assert!(disk.skip.contains(&PathBuf::from("/proc")) && disk.skip.contains(&PathBuf::from("/sys")), "the virtual trees are not entered");
        assert!(disk.skip.contains(&PathBuf::from("/.snapshots")), "nor the snapshots, which repeat the disk");
    }

    #[test]
    fn this_computer_reads_home_first_then_volumes_then_the_rest_of_the_disk() {
        let (open, home) = (Path::new("/a/b"), Path::new("/h"));
        let usb = PathBuf::from("/run/media/u/usb");
        let inside = PathBuf::from("/h/mnt");
        let plan = Scope::Computer.plan(open, home, &[usb.clone(), inside.clone()]);
        assert_eq!(plan.roots, vec![PathBuf::from("/h"), usb.clone(), PathBuf::from("/")], "a volume inside home is read with it");
        for read_already in [PathBuf::from("/h"), usb, PathBuf::from("/proc"), PathBuf::from("/run")] {
            assert!(plan.skip.contains(&read_already), "{read_already:?} is not read twice");
        }
        assert!(!plan.skip.contains(&inside));
        // a home folder of "/" is the disk walk alone
        assert_eq!(Scope::Computer.plan(open, Path::new("/"), &[]).roots, vec![PathBuf::from("/")]);
        // the scope bar starts at This Computer
        assert_eq!(Scope::default(), Scope::Computer);
    }

    #[test]
    fn this_computer_finds_files_deep_in_home_and_on_a_volume_and_stops_at_the_cap() {
        let dir = scratch("computer");
        let (home, volume) = (dir.join("home"), dir.join("usb"));
        std::fs::create_dir_all(home.join("Documents/2026")).unwrap();
        std::fs::create_dir_all(volume.join("backup")).unwrap();
        std::fs::write(home.join("Documents/2026/needle report.txt"), "x").unwrap();
        std::fs::write(home.join("needle.txt"), "x").unwrap();
        std::fs::write(volume.join("backup/needle copy.txt"), "x").unwrap();
        std::fs::write(volume.join("unrelated.txt"), "x").unwrap();
        let plan = Scope::Computer.plan(&home, &home, &[volume.clone()]);
        // the real disk root is not walked in a test: the plan's first two roots are the home folder and the volume
        assert_eq!(plan.roots[..2].to_vec(), vec![home.clone(), volume.clone()]);
        let hits = walk(&plan.roots[..2], &plan.skip, "needle", usize::MAX);
        let mut names: Vec<&str> = hits.iter().map(|e| e.name.as_str()).collect();
        names.sort();
        assert_eq!(names, ["needle copy.txt", "needle report.txt", "needle.txt"]);
        // the home folder is read first, so a cap that is reached there never gets to the volume
        let capped = walk(&plan.roots[..2], &plan.skip, "needle", 2);
        assert_eq!(capped.len(), 2);
        assert!(capped.iter().all(|e| e.path.starts_with(&home)));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_scope_bar_shows_with_a_query_and_names_the_searched_place_in_the_title() {
        let mut files = crate::tests::fixture(vec![]);
        files.path = PathBuf::from("/fixture/Music");
        assert!(!files.scope_bar_shown(), "no query, no bar");
        assert_eq!(files.scope_bar_h(), 0.);
        assert_eq!(files.title(), "Music");
        // typed, not searched: a search would start the walk of the whole computer
        files.query.push_str("aqua");
        assert!(files.scope_bar_shown());
        assert_eq!(files.scope_bar_h(), SCOPE_BAR_H);
        assert_eq!(files.body_top(), HEADER_H + SCOPE_BAR_H, "the content sits under the bar");
        assert_eq!(files.effective_scope(), Scope::Computer, "This Computer is the default");
        assert_eq!(files.title(), "Searching \u{201C}This Computer\u{201D}");
        files.scope_bar.scope = Scope::Folder;
        assert_eq!(files.title(), "Searching \u{201C}Music\u{201D}", "the folder scope names the open folder");
        // an in: term wins over the bar
        files.query.clear();
        files.query.push_str("in:home aqua");
        assert_eq!(files.effective_scope(), Scope::Home);
        assert_eq!(files.title(), "Searching \u{201C}fixture\u{201D}");
        // clearing the query restores the folder's title and the content moves back up
        files.query.clear();
        assert_eq!(files.title(), "Music");
        assert_eq!(files.body_top(), HEADER_H);
        // Recents, a tag or a smart folder offers the bar too; its own plate filters in place, This Computer walks
        files.query.push_str("aqua");
        files.path = PathBuf::from(fs::RECENTS);
        assert!(files.scope_bar_shown(), "the reference offers This Computer from Recents and the tags");
        assert!(filters_in_place(Scope::Folder, true));
        assert!(!filters_in_place(Scope::Computer, true));
        assert!(filters_in_place(Scope::Computer, false), "no bar: only the listing is filtered");
        // the Trash is only filtered
        files.path = files.bin();
        assert!(!files.scope_bar_shown());
    }

    #[test]
    fn scope_plates_follow_the_label_and_each_other() {
        let cells = scope_plates(100., 50., &[70., 40.]);
        assert_eq!(cells[0].0, 100. + 50. + SCOPE_GAP);
        assert_eq!(cells[0].1, 70. + 2. * SCOPE_PAD);
        assert_eq!(cells[1].0, cells[0].0 + cells[0].1 + SCOPE_GAP);
        assert_eq!(cells[1].1, 40. + 2. * SCOPE_PAD);
        assert!(scope_plates(0., 10., &[]).is_empty());
    }

    #[test]
    fn a_click_on_a_scope_plate_names_that_scope() {
        let mut files = crate::tests::fixture(vec![]);
        files.scope_bar.plates = vec![(Scope::Computer, Rect { x: 200., y: 66., w: 90., h: 22. }), (Scope::Folder, Rect { x: 294., y: 66., w: 60., h: 22. })];
        assert_eq!(files.scope_hit(210., 70.), None, "no query: the bar is not shown");
        files.query.push_str("a");
        assert_eq!(files.scope_hit(210., 70.), Some(Scope::Computer));
        assert_eq!(files.scope_hit(300., 70.), Some(Scope::Folder));
        assert_eq!(files.scope_hit(292., 70.), None, "between the plates");
        assert_eq!(files.scope_hit(210., 100.), None, "below the bar");
    }

    #[test]
    fn the_scope_bar_is_drawn_with_the_searched_plate_raised() {
        let ui = font_ui();
        let mut files = crate::tests::fixture(vec![]);
        files.path = PathBuf::from("/fixture/Music");
        files.query.push_str("aqua");
        let (w, h) = (1200usize, 800usize);
        let mut px = vec![0u32; w * h];
        files.paint_scope_bar(&ui, &mut Surface { w, h, stride: w, px: &mut px, y0: 0 });
        assert_eq!(files.scope_bar.plates.len(), 2);
        let (first, r0) = files.scope_bar.plates[0];
        let (second, r1) = files.scope_bar.plates[1];
        assert_eq!((first, second), (Scope::Computer, Scope::Folder));
        assert!(r0.x > files.side() && r1.x > r0.x + r0.w, "the plates sit side by side after the label");
        assert!(r0.y >= HEADER_H && r0.y + r0.h <= HEADER_H + SCOPE_BAR_H, "inside the bar under the toolbar");
        // the searched plate (This Computer) is on the neutral plate; the other is bare
        let at = |r: Rect| px[ui.pt(r.y + r.h / 2.) as usize * w + ui.pt(r.x + 3.) as usize];
        assert_eq!(at(r0), SIDEBAR_PLATE.now());
        assert_eq!(at(r1), 0);
        // a click where the second plate was drawn picks it
        assert_eq!(files.scope_hit(r1.x + 4., r1.y + 4.), Some(Scope::Folder));
        // without a query nothing is drawn and the plates are forgotten
        files.query.clear();
        files.paint_scope_bar(&ui, &mut Surface { w, h, stride: w, px: &mut px, y0: 0 });
        assert!(files.scope_bar.plates.is_empty());
    }

    #[test]
    fn picking_a_scope_searches_again_and_a_new_search_starts_at_the_default() {
        let dir = scratch("pick");
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("sub/needle.txt"), "x").unwrap();
        let mut files = crate::tests::fixture(vec![]);
        files.home = dir.clone();
        files.path = dir.clone();
        files.query.push_str("needle");
        files.scope_bar.scope = Scope::Folder;
        files.restart_search();
        finish(&mut files);
        assert_eq!(files.hits.as_ref().map(|h| h.len()), Some(1), "the folder tree is searched recursively");
        assert_eq!(files.visible().len(), 1, "the hits show in the view");
        // the plate that is already picked does not search again
        files.pick_scope(Scope::Folder);
        assert!(files.search.is_none() && files.hits.is_some());
        // clearing the query shows the folder again
        files.query.clear();
        files.query_changed();
        assert!(files.hits.is_none() && !files.scope_bar_shown());
        // the next search starts at the default scope, not at the folder picked before; stop it at once, since
        // This Computer reads the whole disk
        files.scope_bar.scope = Scope::Folder;
        files.query.push_str("needle");
        files.query_changed();
        assert_eq!(files.scope_bar.scope, Scope::Computer);
        files.stop_search();
        let _ = std::fs::remove_dir_all(&dir);
    }
}
