//! What Books remembers: which books are in the library, where the reader
//! stopped in each, its bookmarks, and the reading settings. One small
//! tab-separated file, written whenever it changes.

use super::engine;
use std::path::{Path, PathBuf};

const MAX_DEPTH: usize = 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Theme {
    White,
    Sepia,
    Dark,
}

impl Theme {
    pub const ALL: [Theme; 3] = [Theme::White, Theme::Sepia, Theme::Dark];

    pub fn name(self) -> &'static str {
        match self {
            Theme::White => "White",
            Theme::Sepia => "Sepia",
            Theme::Dark => "Dark",
        }
    }

    fn id(self) -> u8 {
        self as u8
    }

    fn from_id(i: u8) -> Theme {
        Theme::ALL.get(i as usize).copied().unwrap_or(Theme::White)
    }
}

/// Body text sizes in points, smallest to largest.
pub const SIZES: [f32; 7] = [14.0, 16.0, 18.0, 20.0, 23.0, 27.0, 32.0];
pub const DEFAULT_SIZE: usize = 2;

/// Line spacing choices of the Display panel: a name and the line height as a multiple of the font size.
pub const SPACINGS: [(&str, f32); 3] = [("Tight", 1.25), ("Normal", 1.42), ("Loose", 1.7)];
pub const DEFAULT_SPACING: usize = 1;
/// Side margin choices of the Display panel: a name and the margin in points.
pub const MARGINS: [(&str, f32); 3] = [("Narrow", 24.0), ("Normal", 44.0), ("Wide", 76.0)];
pub const DEFAULT_MARGIN: usize = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Settings {
    pub size: usize,
    pub theme: Theme,
    /// index into `SPACINGS`
    pub spacing: usize,
    /// index into `MARGINS`
    pub margin: usize,
    /// body text is set flush on both sides
    pub justify: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings { size: DEFAULT_SIZE, theme: Theme::White, spacing: DEFAULT_SPACING, margin: DEFAULT_MARGIN, justify: false }
    }
}

impl Settings {
    /// The line height as a multiple of the font size.
    pub fn line_height(&self) -> f32 {
        SPACINGS[self.spacing.min(SPACINGS.len() - 1)].1
    }

    /// The side margin in points.
    pub fn margin_pt(&self) -> f32 {
        MARGINS[self.margin.min(MARGINS.len() - 1)].1
    }
}

/// Where a book stands for its reader (the Want to Read and Finished collections).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Status {
    #[default]
    None,
    WantToRead,
    Finished,
}

impl Status {
    fn id(self) -> u8 {
        self as u8
    }

    fn from_id(i: u8) -> Status {
        match i {
            1 => Status::WantToRead,
            2 => Status::Finished,
            _ => Status::None,
        }
    }
}

/// The order of the library.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum SortBy {
    /// last opened first
    #[default]
    Recent,
    Title,
    Author,
    /// furthest along first
    Progress,
}

impl SortBy {
    pub const ALL: [SortBy; 4] = [SortBy::Recent, SortBy::Title, SortBy::Author, SortBy::Progress];

    pub fn name(self) -> &'static str {
        match self {
            SortBy::Recent => "Recent",
            SortBy::Title => "Title",
            SortBy::Author => "Author",
            SortBy::Progress => "Progress",
        }
    }

    fn id(self) -> u8 {
        self as u8
    }

    fn from_id(i: u8) -> SortBy {
        SortBy::ALL.get(i as usize).copied().unwrap_or_default()
    }
}

/// The sort key of a book: books without an author come last when sorting by author, and the books furthest along come
/// first when sorting by progress (`progress` is the reader's place in 1/1000 of the book). `Recent` keeps the order the
/// shelf already has, so its key is the same for every book.
pub fn sort_key(sort: SortBy, title: &str, author: &str, progress: u16) -> (bool, std::cmp::Reverse<u16>, String, String) {
    use std::cmp::Reverse;
    match sort {
        SortBy::Recent => (false, Reverse(0), String::new(), String::new()),
        SortBy::Title => (false, Reverse(0), title.to_lowercase(), String::new()),
        SortBy::Author => (author.trim().is_empty(), Reverse(0), author.trim().to_lowercase(), title.to_lowercase()),
        SortBy::Progress => (false, Reverse(progress), title.to_lowercase(), String::new()),
    }
}

/// How the library lists its books.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ViewMode {
    #[default]
    Grid,
    List,
}

impl ViewMode {
    fn id(self) -> u8 {
        self as u8
    }

    fn from_id(i: u8) -> ViewMode {
        if i == 1 { ViewMode::List } else { ViewMode::Grid }
    }
}

/// Library preferences, kept with the reading state.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct LibraryPrefs {
    pub sort: SortBy,
    pub view: ViewMode,
    /// the reading goal in minutes a day; 0 = no goal
    pub goal: u32,
}

/// The goals the Library menu offers, in minutes a day (0 = off).
pub const GOALS: [u32; 6] = [0, 10, 15, 30, 60, 120];

/// How many days of reading time are kept.
const KEEP_DAYS: usize = 90;

/// The day number (days since 1970, UTC) a Unix time in seconds falls on.
pub fn day_of(unix_secs: u64) -> u32 {
    (unix_secs / 86_400) as u32
}

/// Today's day number.
pub fn today() -> u32 {
    day_of(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs()))
}

/// The colour names of a highlight, in the order of the Highlight Color menu. The last one is drawn as an underline.
pub const HIGHLIGHT_STYLES: [&str; 6] = ["Yellow", "Green", "Blue", "Pink", "Purple", "Underline"];
/// The style index of the underline.
pub const UNDERLINE: u8 = 5;

/// A reader's own group of books.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Collection {
    pub name: String,
    pub books: Vec<PathBuf>,
}

/// A place in a book: a block and a char offset inside it. Independent of
/// font size and page geometry, so it survives re-pagination.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Pos {
    pub block: usize,
    pub offset: usize,
}

/// A run of text the reader marked: `len` chars from `offset` inside `block`. Like `Pos`, it does not depend on the
/// font size, so a highlight survives re-pagination.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Span {
    pub block: usize,
    pub offset: usize,
    pub len: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub path: PathBuf,
    pub pos: Pos,
    pub bookmarks: Vec<Pos>,
    /// highlighted text, sorted by place
    pub highlights: Vec<Span>,
    /// the note written on a highlight, sorted by place; every note's span is also in `highlights`
    pub notes: Vec<(Span, String)>,
    /// order of last opening (higher = more recent); 0 = never opened
    pub opened: u64,
    /// how far through the book, in thousandths
    pub progress: u16,
    pub status: Status,
    /// the style of the highlights that are not yellow, by place; a highlight missing here is yellow
    pub styles: Vec<(Span, u8)>,
}

#[derive(Debug, Default)]
pub struct Store {
    file: PathBuf,
    pub entries: Vec<Entry>,
    pub settings: Settings,
    pub library: LibraryPrefs,
    pub collections: Vec<Collection>,
    /// seconds read per day (day number, seconds), oldest first
    pub reading: Vec<(u32, u32)>,
    /// library folders the user added in Settings, after the built-in ones
    pub folders: Vec<PathBuf>,
    /// the font family EPUB body text is set in, from the library; empty = the system face
    pub family: String,
    /// Settings > Open Books In: a book opens in continuous scrolling rather than paged
    pub open_continuous: bool,
    clock: u64,
}

/// The family after stepping `delta` places through the Display panel's font row. The row runs over the system face
/// (an empty name) and then `families` in order, and wraps round at both ends. An unknown `current` counts as the
/// system face.
pub fn family_step(families: &[String], current: &str, delta: i32) -> String {
    let n = families.len() as i32 + 1;
    let at = if current.is_empty() { 0 } else { families.iter().position(|f| f == current).map_or(0, |i| i as i32 + 1) };
    let next = (at + delta).rem_euclid(n) as usize;
    if next == 0 { String::new() } else { families[next - 1].clone() }
}

/// Whether the font family choice applies to a book at `path`. EPUB text is set in the chosen face; a PDF page draws
/// as the document set it, so the Display panel's font row is disabled for it.
pub fn family_applies(path: &Path) -> bool {
    !matches!(engine::format_of(path), Some(engine::Format::Pdf))
}

fn clean(path: &Path) -> String {
    path.to_string_lossy().replace(['\t', '\n'], " ")
}

/// A collection name as it is written to the file: one line, no tabs.
fn clean_name(name: &str) -> String {
    name.replace(['\t', '\n', '\r'], " ").trim().to_string()
}

impl Store {
    pub fn load(file: PathBuf) -> Store {
        let mut s = Store { file, ..Default::default() };
        let Ok(text) = std::fs::read_to_string(&s.file) else { return s };
        for line in text.lines() {
            let f: Vec<&str> = line.split('\t').collect();
            match f.as_slice() {
                ["S", size, theme, rest @ ..] => {
                    // files written before the spacing, margin and justification settings end after the theme
                    let at = |i: usize| rest.get(i).and_then(|v| v.parse::<usize>().ok());
                    s.settings = Settings {
                        size: size.parse::<usize>().unwrap_or(DEFAULT_SIZE).min(SIZES.len() - 1),
                        theme: Theme::from_id(theme.parse().unwrap_or(0)),
                        spacing: at(0).unwrap_or(DEFAULT_SPACING).min(SPACINGS.len() - 1),
                        margin: at(1).unwrap_or(DEFAULT_MARGIN).min(MARGINS.len() - 1),
                        justify: at(2) == Some(1),
                    };
                }
                ["F", name] => s.family = clean_name(name),
                ["W", open] => s.open_continuous = *open == "1",
                ["D", path] => {
                    let path = PathBuf::from(path);
                    if !s.folders.contains(&path) {
                        s.folders.push(path);
                    }
                }
                ["V", sort, view, goal] => {
                    s.library = LibraryPrefs { sort: SortBy::from_id(sort.parse().unwrap_or(0)), view: ViewMode::from_id(view.parse().unwrap_or(0)), goal: goal.parse().unwrap_or(0) };
                }
                ["T", day, secs] => {
                    if let (Ok(day), Ok(secs)) = (day.parse::<u32>(), secs.parse::<u32>()) {
                        s.add_reading(day, secs);
                    }
                }
                ["C", name] => {
                    s.create_collection(name);
                }
                ["M", name, path] => {
                    if let Some(c) = s.collections.iter_mut().find(|c| c.name == *name) {
                        let path = PathBuf::from(path);
                        if !c.books.contains(&path) {
                            c.books.push(path);
                        }
                    }
                }
                ["K", path, block, offset, len, style] => {
                    let span = Span { block: block.parse().unwrap_or(0), offset: offset.parse().unwrap_or(0), len: len.parse().unwrap_or(0) };
                    let style = style.parse::<u8>().unwrap_or(0);
                    if span.len > 0 && style != 0 && (style as usize) < HIGHLIGHT_STYLES.len() {
                        let e = s.entry_mut(Path::new(path));
                        e.styles.retain(|(x, _)| *x != span);
                        e.styles.push((span, style));
                        e.styles.sort();
                    }
                }
                ["B", path, block, offset, opened, marks, ..] => {
                    let opened = opened.parse().unwrap_or(0);
                    s.clock = s.clock.max(opened);
                    // a highlight line may come before its book line
                    let e = s.entry_mut(Path::new(path));
                    e.pos = Pos { block: block.parse().unwrap_or(0), offset: offset.parse().unwrap_or(0) };
                    e.bookmarks = marks.split(',').filter_map(parse_pos).collect();
                    e.opened = opened;
                    e.progress = f.get(6).and_then(|p| p.parse::<u16>().ok()).unwrap_or(0).min(1000);
                    e.status = Status::from_id(f.get(7).and_then(|p| p.parse::<u8>().ok()).unwrap_or(0));
                }
                ["N", path, block, offset, len, ..] => {
                    let span = Span { block: block.parse().unwrap_or(0), offset: offset.parse().unwrap_or(0), len: len.parse().unwrap_or(0) };
                    // the note text is written with tabs and line breaks removed, so it is the field after `len`
                    let text = f.get(5).copied().unwrap_or("").trim().to_string();
                    if span.len > 0 && !text.is_empty() {
                        let e = s.entry_mut(Path::new(path));
                        e.notes.retain(|(x, _)| *x != span);
                        e.notes.push((span, text));
                        e.notes.sort_by_key(|(x, _)| *x);
                    }
                }
                ["H", path, block, offset, len] => {
                    let span = Span { block: block.parse().unwrap_or(0), offset: offset.parse().unwrap_or(0), len: len.parse().unwrap_or(0) };
                    if span.len > 0 {
                        let e = s.entry_mut(Path::new(path));
                        if !e.highlights.contains(&span) {
                            e.highlights.push(span);
                            e.highlights.sort();
                        }
                    }
                }
                _ => {}
            }
        }
        s
    }

    pub fn save(&self) {
        let st = &self.settings;
        let mut out = format!("S\t{}\t{}\t{}\t{}\t{}\n", st.size, st.theme.id(), st.spacing, st.margin, st.justify as u8);
        out.push_str(&format!("V\t{}\t{}\t{}\n", self.library.sort.id(), self.library.view.id(), self.library.goal));
        out.push_str(&format!("F\t{}\n", clean_name(&self.family)));
        out.push_str(&format!("W\t{}\n", self.open_continuous as u8));
        for f in &self.folders {
            out.push_str(&format!("D\t{}\n", clean(f)));
        }
        for (day, secs) in &self.reading {
            out.push_str(&format!("T\t{day}\t{secs}\n"));
        }
        for c in &self.collections {
            out.push_str(&format!("C\t{}\n", clean_name(&c.name)));
            for b in &c.books {
                out.push_str(&format!("M\t{}\t{}\n", clean_name(&c.name), clean(b)));
            }
        }
        for e in &self.entries {
            let marks: Vec<String> = e.bookmarks.iter().map(|p| format!("{}.{}", p.block, p.offset)).collect();
            out.push_str(&format!("B\t{}\t{}\t{}\t{}\t{}\t{}\t{}\n", clean(&e.path), e.pos.block, e.pos.offset, e.opened, marks.join(","), e.progress, e.status.id()));
            for h in &e.highlights {
                out.push_str(&format!("H\t{}\t{}\t{}\t{}\n", clean(&e.path), h.block, h.offset, h.len));
            }
            for (span, style) in &e.styles {
                out.push_str(&format!("K\t{}\t{}\t{}\t{}\t{}\n", clean(&e.path), span.block, span.offset, span.len, style));
            }
            for (span, text) in &e.notes {
                let text = text.replace(['\t', '\n', '\r'], " ");
                out.push_str(&format!("N\t{}\t{}\t{}\t{}\t{}\n", clean(&e.path), span.block, span.offset, span.len, text));
            }
        }
        if let Some(dir) = self.file.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let tmp = self.file.with_extension("tmp");
        if std::fs::write(&tmp, out).is_ok() {
            let _ = std::fs::rename(&tmp, &self.file);
        }
    }

    pub fn entry(&self, path: &Path) -> Option<&Entry> {
        self.entries.iter().find(|e| e.path == path)
    }

    /// The entry for `path`, created when it is new.
    pub fn entry_mut(&mut self, path: &Path) -> &mut Entry {
        if let Some(i) = self.entries.iter().position(|e| e.path == path) {
            return &mut self.entries[i];
        }
        self.entries.push(Entry { path: path.to_path_buf(), pos: Pos::default(), bookmarks: vec![], highlights: vec![], notes: vec![], opened: 0, progress: 0, status: Status::None, styles: vec![] });
        self.entries.last_mut().expect("just pushed")
    }

    /// Marks `path` as just opened.
    pub fn touch(&mut self, path: &Path) {
        self.clock += 1;
        let c = self.clock;
        self.entry_mut(path).opened = c;
    }

    pub fn set_pos(&mut self, path: &Path, pos: Pos, progress: u16) {
        let e = self.entry_mut(path);
        e.pos = pos;
        e.progress = progress.min(1000);
    }

    /// Adds or removes a bookmark; true when it now exists.
    pub fn toggle_bookmark(&mut self, path: &Path, pos: Pos) -> bool {
        let e = self.entry_mut(path);
        if let Some(i) = e.bookmarks.iter().position(|&b| b == pos) {
            e.bookmarks.remove(i);
            false
        } else {
            e.bookmarks.push(pos);
            e.bookmarks.sort_by_key(|p| (p.block, p.offset));
            true
        }
    }

    /// Adds or removes a highlight; true when it now exists. An empty span is ignored (false).
    pub fn toggle_highlight(&mut self, path: &Path, span: Span) -> bool {
        if span.len == 0 {
            return false;
        }
        let e = self.entry_mut(path);
        if let Some(i) = e.highlights.iter().position(|&h| h == span) {
            e.highlights.remove(i);
            e.styles.retain(|(s, _)| *s != span);
            false
        } else {
            e.highlights.push(span);
            e.highlights.sort();
            true
        }
    }

    /// The note on a highlight, if it has one.
    pub fn note_of(&self, path: &Path, span: Span) -> Option<String> {
        self.entry(path)?.notes.iter().find(|(s, _)| *s == span).map(|(_, t)| t.clone())
    }

    /// Sets the note on a highlight. The span is highlighted too. Line breaks and tabs become spaces, and the text is
    /// trimmed; an empty note removes the note and keeps the highlight. A zero-length span is ignored.
    pub fn set_note(&mut self, path: &Path, span: Span, text: &str) {
        if span.len == 0 {
            return;
        }
        let text = text.replace(['\t', '\n', '\r'], " ").trim().to_string();
        let e = self.entry_mut(path);
        if !e.highlights.contains(&span) {
            e.highlights.push(span);
            e.highlights.sort();
        }
        e.notes.retain(|(s, _)| *s != span);
        if !text.is_empty() {
            e.notes.push((span, text));
            e.notes.sort_by_key(|(s, _)| *s);
        }
    }

    /// The style of a highlight (an index into `HIGHLIGHT_STYLES`); yellow (0) unless it was given another.
    pub fn highlight_style(&self, path: &Path, span: Span) -> u8 {
        self.entry(path).and_then(|e| e.styles.iter().find(|(s, _)| *s == span)).map_or(0, |(_, style)| *style)
    }

    /// Gives a highlight a colour or the underline. The span is highlighted too. An unknown style, or a zero-length
    /// span, is ignored.
    pub fn set_highlight_style(&mut self, path: &Path, span: Span, style: u8) {
        if span.len == 0 || style as usize >= HIGHLIGHT_STYLES.len() {
            return;
        }
        let e = self.entry_mut(path);
        if !e.highlights.contains(&span) {
            e.highlights.push(span);
            e.highlights.sort();
        }
        e.styles.retain(|(s, _)| *s != span);
        if style != 0 {
            e.styles.push((span, style));
            e.styles.sort();
        }
    }

    // --- Want to Read and Finished ---

    pub fn status_of(&self, path: &Path) -> Status {
        self.entry(path).map_or(Status::None, |e| e.status)
    }

    pub fn set_status(&mut self, path: &Path, status: Status) {
        self.entry_mut(path).status = status;
    }

    /// Chooses `status` for a book, or clears it when the book already has it (a menu item that toggles).
    pub fn toggle_status(&mut self, path: &Path, status: Status) -> Status {
        let now = if self.status_of(path) == status { Status::None } else { status };
        self.set_status(path, now);
        now
    }

    // --- collections ---

    /// `base` made unique among the collection names: "New Collection", "New Collection 2", ...
    pub fn unique_collection_name(&self, base: &str) -> String {
        let base = clean_name(base);
        let base = if base.is_empty() { "New Collection".to_string() } else { base };
        let taken = |n: &str| self.collections.iter().any(|c| c.name.eq_ignore_ascii_case(n));
        if !taken(&base) {
            return base;
        }
        (2..).map(|k| format!("{base} {k}")).find(|n| !taken(n)).expect("an unused number exists")
    }

    /// Adds an empty collection and returns its index; None when the name is empty or taken.
    pub fn create_collection(&mut self, name: &str) -> Option<usize> {
        let name = clean_name(name);
        if name.is_empty() || self.collections.iter().any(|c| c.name.eq_ignore_ascii_case(&name)) {
            return None;
        }
        self.collections.push(Collection { name, books: vec![] });
        Some(self.collections.len() - 1)
    }

    /// Renames collection `i`; false when the name is empty or another collection has it.
    pub fn rename_collection(&mut self, i: usize, name: &str) -> bool {
        let name = clean_name(name);
        if name.is_empty() || i >= self.collections.len() || self.collections.iter().enumerate().any(|(k, c)| k != i && c.name.eq_ignore_ascii_case(&name)) {
            return false;
        }
        self.collections[i].name = name;
        true
    }

    /// Removes collection `i`; its books stay in the library.
    pub fn delete_collection(&mut self, i: usize) {
        if i < self.collections.len() {
            self.collections.remove(i);
        }
    }

    pub fn in_collection(&self, i: usize, path: &Path) -> bool {
        self.collections.get(i).is_some_and(|c| c.books.iter().any(|b| b == path))
    }

    /// Adds the book to collection `i`, or takes it out when it is in it; true when it is in the collection now.
    pub fn toggle_in_collection(&mut self, i: usize, path: &Path) -> bool {
        let Some(c) = self.collections.get_mut(i) else { return false };
        if let Some(k) = c.books.iter().position(|b| b == path) {
            c.books.remove(k);
            false
        } else {
            c.books.push(path.to_path_buf());
            true
        }
    }

    // --- reading time ---

    /// Adds `secs` of reading to `day`. Only the last `KEEP_DAYS` days are kept.
    pub fn add_reading(&mut self, day: u32, secs: u32) {
        if secs == 0 {
            return;
        }
        match self.reading.iter_mut().find(|(d, _)| *d == day) {
            Some((_, total)) => *total = total.saturating_add(secs),
            None => {
                self.reading.push((day, secs));
                self.reading.sort_by_key(|(d, _)| *d);
                if self.reading.len() > KEEP_DAYS {
                    let extra = self.reading.len() - KEEP_DAYS;
                    self.reading.drain(..extra);
                }
            }
        }
    }

    /// Seconds read on `day`.
    pub fn seconds_on(&self, day: u32) -> u32 {
        self.reading.iter().find(|(d, _)| *d == day).map_or(0, |(_, s)| *s)
    }

    /// The days in a row, ending today or yesterday, on which the reading goal was met; 0 without a goal.
    pub fn goal_streak(&self, today: u32) -> u32 {
        let goal = self.library.goal * 60;
        if goal == 0 {
            return 0;
        }
        let met = |d: u32| self.seconds_on(d) >= goal;
        // today may still be in progress: start from yesterday when today's goal is not met yet
        let mut day = if met(today) { today } else { today.saturating_sub(1) };
        let mut n = 0;
        while met(day) {
            n += 1;
            if day == 0 {
                break;
            }
            day -= 1;
        }
        n
    }
}

/// The text beside a library cover: how far the reader is, or where the book stands.
pub fn caption(status: Status, progress: u16) -> String {
    match status {
        Status::Finished => "Finished".to_string(),
        Status::WantToRead if progress == 0 => "Want to Read".to_string(),
        _ if progress == 0 => "New".to_string(),
        _ => format!("{}%", progress / 10),
    }
}

/// The reading goal line for the Reading Now section: minutes read today against the goal. None without a goal.
pub fn goal_label(seconds_today: u32, goal_minutes: u32) -> Option<String> {
    if goal_minutes == 0 {
        return None;
    }
    let minutes = seconds_today / 60;
    Some(if minutes >= goal_minutes { format!("Goal reached: {minutes} min today") } else { format!("Today: {minutes} of {goal_minutes} min") })
}

/// Counts reading time. Every call tells it the time and whether a book is open; the time since the last call counts as
/// reading, but at most `MAX_GAP_MS` of it, so a book left open on a page does not count for hours.
#[derive(Debug, Default)]
pub struct ReadingClock {
    last: Option<f64>,
    /// milliseconds that have not yet made a whole second
    carry: f64,
}

/// The longest stretch between two calls that still counts as reading.
const MAX_GAP_MS: f64 = 120_000.0;

impl ReadingClock {
    /// Whole seconds of reading to add now.
    pub fn advance(&mut self, t: f64, reading: bool) -> u32 {
        let last = if reading { self.last.replace(t) } else { self.last.take() };
        let Some(last) = last.filter(|_| reading) else { return 0 };
        self.carry += (t - last).clamp(0.0, MAX_GAP_MS);
        let secs = (self.carry / 1000.0).floor();
        self.carry -= secs * 1000.0;
        secs as u32
    }
}

/// Copies a book that arrived as a stream (an Add to Library choice, a shared file) into `dir` and returns the new path.
/// Only the file name of `name` is used, and only `.epub` and `.pdf` are taken. A name that is taken gets a number
/// ("Moby Dick 2.epub"). The copy is written beside its place under a hidden name, then renamed, so a failed copy
/// leaves no half file in the library.
pub fn import_into(dir: &Path, name: &str, source: &mut dyn std::io::Read) -> Result<PathBuf, String> {
    let file = Path::new(name).file_name().map(|n| n.to_string_lossy().replace(['\t', '\n', '\r'], " ")).filter(|n| !n.trim().is_empty() && !n.starts_with('.')).ok_or_else(|| "That file has no usable name.".to_string())?;
    if !engine::is_book(Path::new(&file)) {
        return Err(format!("Books opens .epub and .pdf files, not {file}."));
    }
    std::fs::create_dir_all(dir).map_err(|e| format!("Cannot make the Books folder: {e}"))?;
    let stem = Path::new(&file).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let ext = Path::new(&file).extension().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let dest = (1..)
        .map(|k| if k == 1 { dir.join(&file) } else { dir.join(format!("{stem} {k}.{ext}")) })
        .find(|p| !p.exists())
        .expect("an unused name exists");
    let tmp = dir.join(format!(".{}.part", dest.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()));
    let copied = std::fs::File::create(&tmp).and_then(|mut out| std::io::copy(source, &mut out).map(|_| ()));
    if let Err(e) = copied {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("Cannot copy {file}: {e}"));
    }
    std::fs::rename(&tmp, &dest).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("Cannot add {file}: {e}")
    })?;
    Ok(dest)
}

fn parse_pos(s: &str) -> Option<Pos> {
    let (b, o) = s.split_once('.')?;
    Some(Pos { block: b.parse().ok()?, offset: o.parse().ok()? })
}

/// Every book under `dirs`, plus the explicitly added `extra` paths that still
/// exist, most recently opened first, then by name.
pub fn library(store: &Store, dirs: &[PathBuf]) -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = vec![];
    for d in dirs {
        walk(d, 0, &mut found);
    }
    for e in &store.entries {
        if e.path.is_file() && !found.contains(&e.path) {
            found.push(e.path.clone());
        }
    }
    let opened = |p: &PathBuf| store.entry(p).map_or(0, |e| e.opened);
    found.sort_by(|a, b| opened(b).cmp(&opened(a)).then_with(|| a.file_name().cmp(&b.file_name())));
    found
}

fn walk(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let path = e.path();
        if e.file_name().to_string_lossy().starts_with('.') {
            continue;
        }
        if path.is_dir() {
            if depth < MAX_DEPTH {
                walk(&path, depth + 1, out);
            }
        } else if engine::is_book(&path) && !out.contains(&path) {
            out.push(path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/target/test-books-store")).join(format!("{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn state_round_trips() {
        let dir = scratch("roundtrip");
        let file = dir.join("state.tsv");
        let mut s = Store::load(file.clone());
        s.settings = Settings { size: 4, theme: Theme::Dark, spacing: 2, margin: 0, justify: true };
        s.touch(Path::new("/a/one.epub"));
        s.set_pos(Path::new("/a/one.epub"), Pos { block: 12, offset: 34 }, 250);
        assert!(s.toggle_bookmark(Path::new("/a/one.epub"), Pos { block: 9, offset: 0 }));
        assert!(s.toggle_bookmark(Path::new("/a/one.epub"), Pos { block: 2, offset: 5 }));
        s.touch(Path::new("/a/two.pdf"));
        s.save();
        let t = Store::load(file);
        assert_eq!(t.settings, Settings { size: 4, theme: Theme::Dark, spacing: 2, margin: 0, justify: true });
        let one = t.entry(Path::new("/a/one.epub")).unwrap();
        assert_eq!(one.pos, Pos { block: 12, offset: 34 });
        assert_eq!(one.progress, 250);
        assert_eq!(one.bookmarks, vec![Pos { block: 2, offset: 5 }, Pos { block: 9, offset: 0 }]);
        assert!(t.entry(Path::new("/a/two.pdf")).unwrap().opened > one.opened);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn bookmark_toggle_removes() {
        let mut s = Store::default();
        let p = Path::new("x.pdf");
        assert!(s.toggle_bookmark(p, Pos { block: 1, offset: 1 }));
        assert!(!s.toggle_bookmark(p, Pos { block: 1, offset: 1 }));
        assert!(s.entry(p).unwrap().bookmarks.is_empty());
    }

    #[test]
    fn library_scans_sorts_and_includes_added_files() {
        let dir = scratch("lib");
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::create_dir_all(dir.join(".hidden")).unwrap();
        for f in ["b.epub", "a.pdf", "notes.txt", "sub/c.PDF", ".hidden/d.pdf"] {
            std::fs::write(dir.join(f), b"x").unwrap();
        }
        let elsewhere = dir.join("e.pdf");
        std::fs::write(&elsewhere, b"x").unwrap();
        let mut s = Store::default();
        s.touch(&dir.join("sub/c.PDF"));
        s.entry_mut(&elsewhere);
        s.entry_mut(Path::new("/gone/f.pdf"));
        let names = |v: Vec<PathBuf>| v.iter().map(|p| p.file_name().unwrap().to_string_lossy().into_owned()).collect::<Vec<_>>();
        assert_eq!(names(library(&s, &[dir.clone()])), ["c.PDF", "a.pdf", "b.epub", "e.pdf"]);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn bad_settings_fall_back() {
        let dir = scratch("bad");
        let file = dir.join("state.tsv");
        std::fs::write(&file, "S\t99\t7\nB\tp\tx\ty\tz\t\ngarbage\n").unwrap();
        let s = Store::load(file);
        assert_eq!(s.settings, Settings { size: SIZES.len() - 1, theme: Theme::White, ..Settings::default() });
        assert_eq!(s.entries[0].pos, Pos::default());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn old_state_files_without_the_newer_fields_load_with_defaults() {
        let dir = scratch("old");
        let file = dir.join("state.tsv");
        std::fs::write(&file, "S\t3\t1\nB\t/a/x.epub\t4\t5\t7\t\t300\n").unwrap();
        let s = Store::load(file);
        assert_eq!(s.settings, Settings { size: 3, theme: Theme::Sepia, ..Settings::default() });
        assert_eq!(s.library, LibraryPrefs::default());
        let e = s.entry(Path::new("/a/x.epub")).unwrap();
        assert_eq!((e.progress, e.status), (300, Status::None));
        assert!(s.collections.is_empty() && s.reading.is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn status_collections_prefs_styles_and_reading_round_trip() {
        let dir = scratch("newer");
        let file = dir.join("state.tsv");
        let (a, b) = (Path::new("/a/one.epub"), Path::new("/a/two.pdf"));
        let mut s = Store::load(file.clone());
        s.set_status(a, Status::Finished);
        s.set_status(b, Status::WantToRead);
        let reading = s.create_collection("Holiday reads").unwrap();
        assert!(s.toggle_in_collection(reading, a));
        s.library = LibraryPrefs { sort: SortBy::Author, view: ViewMode::List, goal: 30 };
        s.add_reading(20_000, 600);
        s.add_reading(20_001, 90);
        let span = Span { block: 3, offset: 4, len: 5 };
        s.set_highlight_style(a, span, 2);
        s.save();
        let t = Store::load(file);
        assert_eq!(t.status_of(a), Status::Finished);
        assert_eq!(t.status_of(b), Status::WantToRead);
        assert_eq!(t.collections, vec![Collection { name: "Holiday reads".into(), books: vec![a.to_path_buf()] }]);
        assert_eq!(t.library, LibraryPrefs { sort: SortBy::Author, view: ViewMode::List, goal: 30 });
        assert_eq!(t.seconds_on(20_000), 600);
        assert_eq!(t.seconds_on(20_001), 90);
        assert_eq!(t.highlight_style(a, span), 2);
        assert!(t.entry(a).unwrap().highlights.contains(&span));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn toggling_a_status_sets_it_and_a_second_choice_clears_it() {
        let mut s = Store::default();
        let p = Path::new("x.epub");
        assert_eq!(s.toggle_status(p, Status::Finished), Status::Finished);
        assert_eq!(s.toggle_status(p, Status::WantToRead), Status::WantToRead);
        assert_eq!(s.toggle_status(p, Status::WantToRead), Status::None);
        assert_eq!(s.status_of(p), Status::None);
    }

    #[test]
    fn collections_get_unique_names_and_hold_books() {
        let mut s = Store::default();
        assert_eq!(s.unique_collection_name(""), "New Collection");
        let first = s.create_collection(" New Collection ").unwrap();
        assert_eq!(s.collections[first].name, "New Collection");
        assert_eq!(s.unique_collection_name("new collection"), "new collection 2");
        assert_eq!(s.create_collection("NEW COLLECTION"), None, "a name that differs only in case is taken");
        assert_eq!(s.create_collection("   "), None);
        let second = s.create_collection("Poetry").unwrap();
        assert!(!s.rename_collection(second, "New collection"), "taken by the first");
        assert!(s.rename_collection(second, "Verse"));
        let book = Path::new("p.pdf");
        assert!(s.toggle_in_collection(second, book));
        assert!(s.in_collection(second, book) && !s.in_collection(first, book));
        assert!(!s.toggle_in_collection(second, book));
        assert!(s.toggle_in_collection(second, book));
        s.delete_collection(first);
        assert_eq!(s.collections.len(), 1);
        assert_eq!(s.collections[0].name, "Verse");
        assert!(!s.toggle_in_collection(9, book), "no such collection");
    }

    #[test]
    fn highlight_styles_follow_their_highlight() {
        let mut s = Store::default();
        let p = Path::new("x.epub");
        let span = Span { block: 1, offset: 2, len: 3 };
        assert_eq!(s.highlight_style(p, span), 0);
        s.set_highlight_style(p, span, UNDERLINE);
        assert_eq!(s.highlight_style(p, span), UNDERLINE);
        assert!(s.entry(p).unwrap().highlights.contains(&span), "styling highlights the text");
        s.set_highlight_style(p, span, 9);
        assert_eq!(s.highlight_style(p, span), UNDERLINE, "an unknown style is ignored");
        s.set_highlight_style(p, span, 0);
        assert_eq!(s.highlight_style(p, span), 0);
        assert!(s.entry(p).unwrap().styles.is_empty(), "yellow is the default and is not stored");
        s.set_highlight_style(p, span, 3);
        assert!(!s.toggle_highlight(p, span));
        assert!(s.entry(p).unwrap().styles.is_empty(), "removing the highlight drops its style");
        s.set_highlight_style(p, Span { block: 0, offset: 0, len: 0 }, 1);
        assert!(s.entry(p).unwrap().styles.is_empty());
    }

    #[test]
    fn sort_keys_order_by_title_and_author() {
        let mut books = vec![("Zebra", "Adams"), ("avocado", ""), ("Mango", "Brown"), ("Berry", "adams")];
        books.sort_by_key(|(t, a)| sort_key(SortBy::Title, t, a, 0));
        assert_eq!(books.iter().map(|b| b.0).collect::<Vec<_>>(), ["avocado", "Berry", "Mango", "Zebra"]);
        books.sort_by_key(|(t, a)| sort_key(SortBy::Author, t, a, 0));
        assert_eq!(books.iter().map(|b| b.0).collect::<Vec<_>>(), ["Berry", "Zebra", "Mango", "avocado"], "same author by title, no author last");
        books.sort_by_key(|(t, a)| sort_key(SortBy::Recent, t, a, 0));
        assert_eq!(books[0].0, "Berry", "Recent leaves a stable sort alone");
    }

    #[test]
    fn captions_show_status_before_progress() {
        assert_eq!(caption(Status::None, 0), "New");
        assert_eq!(caption(Status::None, 456), "45%");
        assert_eq!(caption(Status::WantToRead, 0), "Want to Read");
        assert_eq!(caption(Status::WantToRead, 120), "12%");
        assert_eq!(caption(Status::Finished, 1000), "Finished");
    }

    #[test]
    fn reading_time_accumulates_per_day_and_old_days_drop() {
        let mut s = Store::default();
        s.add_reading(100, 30);
        s.add_reading(100, 45);
        s.add_reading(100, 0);
        assert_eq!(s.seconds_on(100), 75);
        assert_eq!(s.seconds_on(99), 0);
        for d in 0..(KEEP_DAYS as u32 + 5) {
            s.add_reading(1000 + d, 1);
        }
        assert_eq!(s.reading.len(), KEEP_DAYS);
        assert_eq!(s.seconds_on(100), 0, "the oldest days were dropped");
        assert_eq!(s.reading.last().unwrap().0, 1000 + KEEP_DAYS as u32 + 4);
        assert_eq!(day_of(86_399), 0);
        assert_eq!(day_of(86_400), 1);
    }

    #[test]
    fn the_goal_streak_counts_days_in_a_row() {
        let mut s = Store::default();
        s.library.goal = 10;
        assert_eq!(s.goal_streak(50), 0);
        s.add_reading(48, 600);
        s.add_reading(49, 900);
        assert_eq!(s.goal_streak(50), 2, "today is still open, so the run ends yesterday");
        s.add_reading(50, 599);
        assert_eq!(s.goal_streak(50), 2);
        s.add_reading(50, 1);
        assert_eq!(s.goal_streak(50), 3);
        s.add_reading(46, 600);
        assert_eq!(s.goal_streak(50), 3, "a gap on day 47 ends the run");
        s.library.goal = 0;
        assert_eq!(s.goal_streak(50), 0);
    }

    #[test]
    fn the_goal_label_shows_progress_then_success() {
        assert_eq!(goal_label(500, 0), None);
        assert_eq!(goal_label(125, 30), Some("Today: 2 of 30 min".to_string()));
        assert_eq!(goal_label(1800, 30), Some("Goal reached: 30 min today".to_string()));
        assert_eq!(goal_label(5000, 30), Some("Goal reached: 83 min today".to_string()));
    }

    #[test]
    fn the_reading_clock_counts_whole_seconds_and_caps_idle_gaps() {
        let mut c = ReadingClock::default();
        assert_eq!(c.advance(1000.0, true), 0, "the first call only starts the clock");
        assert_eq!(c.advance(2500.0, true), 1, "1.5 s: one whole second, half a second carried");
        assert_eq!(c.advance(3600.0, true), 1, "the carried half second and 1.1 s make 1.6 s");
        assert_eq!(c.advance(3700.0, false), 0, "a closed book stops it");
        assert_eq!(c.advance(9_000_000.0, true), 0, "and starts it afresh");
        assert_eq!(c.advance(9_000_000.0 + 3_600_000.0, true), 120, "an hour on one page counts two minutes");
    }

    #[test]
    fn importing_copies_a_book_under_a_safe_unused_name() {
        let dir = scratch("import");
        let lib = dir.join("Books");
        let mut src: &[u8] = b"%PDF-1.4 one";
        let first = import_into(&lib, "../../etc/Report.PDF", &mut src).unwrap();
        assert_eq!(first, lib.join("Report.PDF"), "only the file name is used");
        assert_eq!(std::fs::read(&first).unwrap(), b"%PDF-1.4 one");
        let mut again: &[u8] = b"%PDF-1.4 two";
        let second = import_into(&lib, "Report.PDF", &mut again).unwrap();
        assert_eq!(second, lib.join("Report 2.PDF"));
        assert_eq!(std::fs::read(&first).unwrap(), b"%PDF-1.4 one", "the first copy is untouched");
        let mut text: &[u8] = b"hello";
        assert!(import_into(&lib, "notes.txt", &mut text).unwrap_err().contains(".epub and .pdf"));
        let mut hidden: &[u8] = b"x";
        assert!(import_into(&lib, ".secret.epub", &mut hidden).is_err());
        let names: Vec<String> = std::fs::read_dir(&lib).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        assert_eq!(names.len(), 2, "no part file is left behind: {names:?}");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn spacing_and_margin_settings_resolve_to_numbers() {
        let s = Settings::default();
        assert_eq!(s.line_height(), crate::books::layout::LINE_HEIGHT);
        assert_eq!(s.margin_pt(), 44.0);
        let wide = Settings { spacing: 2, margin: 2, ..s };
        assert!(wide.line_height() > s.line_height() && wide.margin_pt() > s.margin_pt());
        let bad = Settings { spacing: 99, margin: 99, ..s };
        assert_eq!((bad.line_height(), bad.margin_pt()), (SPACINGS[2].1, MARGINS[2].1), "out of range clamps");
    }
}
