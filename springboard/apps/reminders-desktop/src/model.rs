//! Reminder data model and every pure rule on it: smart lists, subtask tree,
//! sorting and grouping, search, completion with repeat, undoable deletion,
//! reordering and alert times. Nothing here touches the UI or the disk.
use crate::repeat::{next_occurrence, Repeat};
use bifrost_app::controls::fetch::{fetch_indices, Field, FetchRequest, Op, Predicate, Record};
use springboard::shell::clock::tz::civil_from_days;
use std::collections::HashSet;

pub const MAX_TITLE: usize = 240;
pub const MAX_NOTES: usize = 2000;
pub const MAX_URL: usize = 500;
pub const MAX_NAME: usize = 40;
pub const DEFAULT_LIST: u64 = 1;
/// Limits the store enforces on load and save.
pub const MAX_ITEMS: usize = 10_000;
pub const MAX_LISTS: usize = 100;
pub const MAX_GROUPS: usize = 50;
/// Sections one list may hold.
pub const MAX_SECTIONS: usize = 20;
pub const MAX_TEMPLATES: usize = 30;
pub const MAX_SMARTS: usize = 30;
/// Reminders one template may hold.
pub const MAX_TEMPLATE_ITEMS: usize = 200;
/// Alert time of a reminder that has a date but no time of day.
pub const DEFAULT_ALERT_MIN: u16 = 9 * 60;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reminder {
    pub id: u64,
    pub title: String,
    pub notes: String,
    pub url: String,
    pub list: u64,
    /// One level of nesting: the id of the parent reminder.
    pub parent: Option<u64>,
    /// Local calendar day since 1970-01-01.
    pub due: Option<i64>,
    /// Minutes after local midnight.
    pub time: Option<u16>,
    pub completed: bool,
    pub flagged: bool,
    /// 0 none, 1 low, 2 medium, 3 high
    pub priority: u8,
    pub repeat: Option<Repeat>,
    /// Extra alert this many minutes before the due moment.
    pub early: Option<u32>,
    /// Unix second the reminder was created; `None` for one made before the app kept it.
    pub created: Option<i64>,
    /// The id of a section of its list (`ListDef::sections`); only a top-level reminder is placed in one.
    pub section: Option<u64>,
    /// A time zone name or POSIX rule (`zones`): the due time is read on that clock. `None` is the device's zone.
    pub tz: Option<String>,
    /// Once the due moment has passed, the alert repeats every minute until the reminder is completed.
    pub urgent: bool,
    /// File names of the pictures attached to the reminder (`attach`), in the `attachments` folder beside the store.
    pub attachments: Vec<String>,
}

impl Reminder {
    pub fn new(id: u64, title: impl Into<String>) -> Self {
        Self {
            id,
            title: title.into(),
            notes: String::new(),
            url: String::new(),
            list: DEFAULT_LIST,
            parent: None,
            due: None,
            time: None,
            completed: false,
            flagged: false,
            priority: 0,
            repeat: None,
            early: None,
            created: None,
            section: None,
            tz: None,
            urgent: false,
            attachments: vec![],
        }
    }

    /// The `#tags` of the title and the notes, each once, in order.
    pub fn tags(&self) -> Vec<String> {
        let mut out = tags_in(&self.title);
        for t in tags_in(&self.notes) {
            if out.iter().all(|o| tag_key(o) != tag_key(&t)) {
                out.push(t);
            }
        }
        out
    }
}

/// A tag key: the FNV-1a hash of the lowercase tag. A `View::Tag` holds it, so the view stays `Copy` and the key
/// does not change when the tag list does.
pub fn tag_key(tag: &str) -> u64 {
    tag.to_lowercase()
        .bytes()
        .fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3))
}

/// The `#tags` in `text`, in order, each once. Matching ignores case and the first spelling is kept. A tag is a `#`
/// at the start or after a space or punctuation (so `C#` is not one), then a letter, then letters, digits, `_` or
/// `-` (a trailing `-` is left out).
pub fn tags_in(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut out: Vec<String> = vec![];
    for i in 0..chars.len() {
        if chars[i] != '#' {
            continue;
        }
        if i > 0 && (chars[i - 1].is_alphanumeric() || chars[i - 1] == '#' || chars[i - 1] == '&') {
            continue;
        }
        if !chars.get(i + 1).is_some_and(|c| c.is_alphabetic()) {
            continue;
        }
        let word: String = chars[i + 1..].iter().take_while(|c| c.is_alphanumeric() || **c == '_' || **c == '-').collect();
        let word = word.trim_end_matches('-').to_string();
        if out.iter().all(|t| tag_key(t) != tag_key(&word)) {
            out.push(word);
        }
    }
    out
}

/// Every tag the reminders carry, sorted by name without case, with the number of open reminders that carry it.
/// A tag used only by completed reminders is listed with 0.
pub fn tag_counts(items: &[Reminder]) -> Vec<(String, usize)> {
    let mut out: Vec<(String, usize)> = vec![];
    for r in items {
        for t in r.tags() {
            let key = tag_key(&t);
            let pos = match out.iter().position(|(name, _)| tag_key(name) == key) {
                Some(p) => p,
                None => {
                    out.push((t, 0));
                    out.len() - 1
                }
            };
            if !r.completed {
                out[pos].1 += 1;
            }
        }
    }
    out.sort_by_cached_key(|(name, _)| name.to_lowercase());
    out
}

/// The name (first spelling) of the tag with `key`, when any reminder carries it.
pub fn tag_name(items: &[Reminder], key: u64) -> Option<String> {
    tag_counts(items).into_iter().find(|(name, _)| tag_key(name) == key).map(|(name, _)| name)
}

/// The fields a smart list's predicate reads (`fetch::Record`): completion, the flag, whether a day is set and the day.
impl Record for Reminder {
    fn field(&self, key: &str) -> Option<Field> {
        match key {
            "completed" => Some(Field::Bool(self.completed)),
            "flagged" => Some(Field::Bool(self.flagged)),
            "scheduled" => Some(Field::Bool(self.due.is_some())),
            "due" => self.due.map(Field::Int),
            _ => None,
        }
    }
}

/// A named part of a list; reminders of the list are placed in one (`Reminder::section`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Section {
    pub id: u64,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListDef {
    pub id: u64,
    pub name: String,
    /// Index into the colour palette.
    pub color: u8,
    /// Index into the icon set.
    pub icon: u8,
    /// A pinned list has a tile beside the smart lists.
    pub pinned: bool,
    /// The list group the list sits in, in the sidebar.
    pub group: Option<u64>,
    pub sections: Vec<Section>,
}

impl ListDef {
    pub fn new(id: u64, name: impl Into<String>, color: u8, icon: u8) -> Self {
        Self { id, name: name.into(), color, icon, pinned: false, group: None, sections: vec![] }
    }
}

/// A named, collapsible group of lists in the sidebar.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListGroup {
    pub id: u64,
    pub name: String,
}

/// One reminder of a template: the content without dates, repeat or completion.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TemplateItem {
    pub title: String,
    pub notes: String,
    pub url: String,
    pub flagged: bool,
    pub priority: u8,
    /// A subtask of the nearest earlier item that is not one.
    pub sub: bool,
}

/// A saved list that new lists can be made from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Template {
    pub id: u64,
    pub name: String,
    pub color: u8,
    pub icon: u8,
    pub items: Vec<TemplateItem>,
}

/// Which due days a custom smart list takes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DueRule {
    #[default]
    Any,
    Overdue,
    Today,
    /// Today and the six days after it.
    Next7,
    /// A week from today or later.
    Later,
    NoDate,
}

impl DueRule {
    pub const ALL: [Self; 6] = [Self::Any, Self::Overdue, Self::Today, Self::Next7, Self::Later, Self::NoDate];
    pub fn label(self) -> &'static str {
        match self {
            Self::Any => "Any Date",
            Self::Overdue => "Overdue",
            Self::Today => "Due Today",
            Self::Next7 => "Next 7 Days",
            Self::Later => "Later",
            Self::NoDate => "No Date",
        }
    }
    /// The position in [`DueRule::ALL`], which the store writes.
    pub fn code(self) -> u8 {
        Self::ALL.iter().position(|d| *d == self).unwrap_or(0) as u8
    }
    pub fn from_code(code: u8) -> Option<Self> {
        Self::ALL.get(code as usize).copied()
    }
    /// The next rule in the list, from the last back to the first.
    pub fn next(self) -> Self {
        Self::ALL[(self.code() as usize + 1) % Self::ALL.len()]
    }
}

/// Which times of day a custom smart list takes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TimeRule {
    #[default]
    Any,
    /// Before noon.
    Morning,
    /// Noon up to 5 pm.
    Afternoon,
    /// 5 pm on.
    Evening,
    NoTime,
}

impl TimeRule {
    pub const ALL: [Self; 5] = [Self::Any, Self::Morning, Self::Afternoon, Self::Evening, Self::NoTime];
    pub fn label(self) -> &'static str {
        match self {
            Self::Any => "Any Time",
            Self::Morning => "Morning",
            Self::Afternoon => "Afternoon",
            Self::Evening => "Evening",
            Self::NoTime => "No Time",
        }
    }
    pub fn code(self) -> u8 {
        Self::ALL.iter().position(|d| *d == self).unwrap_or(0) as u8
    }
    pub fn from_code(code: u8) -> Option<Self> {
        Self::ALL.get(code as usize).copied()
    }
    pub fn next(self) -> Self {
        Self::ALL[(self.code() as usize + 1) % Self::ALL.len()]
    }
}

/// Whether a custom smart list takes flagged reminders, unflagged ones or both.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FlagRule {
    #[default]
    Any,
    Flagged,
    Unflagged,
}

impl FlagRule {
    pub const ALL: [Self; 3] = [Self::Any, Self::Flagged, Self::Unflagged];
    pub fn label(self) -> &'static str {
        match self {
            Self::Any => "Any",
            Self::Flagged => "Flagged",
            Self::Unflagged => "Not Flagged",
        }
    }
    pub fn code(self) -> u8 {
        Self::ALL.iter().position(|d| *d == self).unwrap_or(0) as u8
    }
    pub fn from_code(code: u8) -> Option<Self> {
        Self::ALL.get(code as usize).copied()
    }
    pub fn next(self) -> Self {
        Self::ALL[(self.code() as usize + 1) % Self::ALL.len()]
    }
}

/// What a custom smart list collects: the open reminders that satisfy every rule that is set.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SmartRules {
    pub due: DueRule,
    pub time: TimeRule,
    pub flagged: FlagRule,
    /// The lowest priority taken: 0 any, 1 low and up, 2 medium and up, 3 high.
    pub priority: u8,
    /// A tag every reminder must carry, by `tag_key`.
    pub tag: Option<u64>,
    /// The one list the reminders must be in.
    pub list: Option<u64>,
}

impl SmartRules {
    /// Whether open reminder `r` belongs to the smart list on `today`. A completed reminder never does.
    pub fn matches(&self, r: &Reminder, today: i64) -> bool {
        if r.completed {
            return false;
        }
        let due = match self.due {
            DueRule::Any => true,
            DueRule::Overdue => r.due.is_some_and(|d| d < today),
            DueRule::Today => r.due == Some(today),
            DueRule::Next7 => r.due.is_some_and(|d| (today..today + 7).contains(&d)),
            DueRule::Later => r.due.is_some_and(|d| d >= today + 7),
            DueRule::NoDate => r.due.is_none(),
        };
        let time = match self.time {
            TimeRule::Any => true,
            TimeRule::Morning => r.time.is_some_and(|t| t < 12 * 60),
            TimeRule::Afternoon => r.time.is_some_and(|t| (12 * 60..17 * 60).contains(&t)),
            TimeRule::Evening => r.time.is_some_and(|t| t >= 17 * 60),
            TimeRule::NoTime => r.time.is_none(),
        };
        let flagged = match self.flagged {
            FlagRule::Any => true,
            FlagRule::Flagged => r.flagged,
            FlagRule::Unflagged => !r.flagged,
        };
        due && time
            && flagged
            && r.priority >= self.priority
            && self.list.is_none_or(|l| r.list == l)
            && self.tag.is_none_or(|key| r.tags().iter().any(|t| tag_key(t) == key))
    }
}

/// A smart list the user defined.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SmartDef {
    pub id: u64,
    pub name: String,
    pub color: u8,
    pub rules: SmartRules,
}

/// The label of a minimum priority in the smart list form.
pub fn priority_rule_label(min: u8) -> &'static str {
    match min {
        1 => "Low or Higher",
        2 => "Medium or Higher",
        3 => "High",
        _ => "Any",
    }
}

/// The tag after `current` among the tags the reminders carry (by name, without case); none after the last, and none
/// when `current` is no longer used. From none it is the first tag, if there is one.
pub fn next_tag(current: Option<u64>, items: &[Reminder]) -> Option<u64> {
    let keys: Vec<u64> = tag_counts(items).iter().map(|(name, _)| tag_key(name)).collect();
    match current {
        None => keys.first().copied(),
        Some(key) => keys.iter().position(|k| *k == key).and_then(|i| keys.get(i + 1)).copied(),
    }
}

/// The list after `current` in `lists`; none after the last or when `current` is gone. From none it is the first list.
pub fn next_list(current: Option<u64>, lists: &[ListDef]) -> Option<u64> {
    match current {
        None => lists.first().map(|l| l.id),
        Some(id) => lists.iter().position(|l| l.id == id).and_then(|i| lists.get(i + 1)).map(|l| l.id),
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Data {
    pub lists: Vec<ListDef>,
    pub items: Vec<Reminder>,
    pub groups: Vec<ListGroup>,
    pub templates: Vec<Template>,
    pub smarts: Vec<SmartDef>,
}

impl Default for Data {
    fn default() -> Self {
        Self {
            lists: vec![ListDef::new(DEFAULT_LIST, "Reminders", 0, 0)],
            items: vec![],
            groups: vec![],
            templates: vec![],
            smarts: vec![],
        }
    }
}

impl Data {
    pub fn next_item_id(&self) -> Option<u64> {
        self.items.iter().map(|r| r.id).max().unwrap_or(0).checked_add(1)
    }
    pub fn next_list_id(&self) -> Option<u64> {
        self.lists.iter().map(|l| l.id).max().unwrap_or(0).checked_add(1)
    }
    pub fn next_group_id(&self) -> Option<u64> {
        self.groups.iter().map(|g| g.id).max().unwrap_or(0).checked_add(1)
    }
    pub fn next_smart_id(&self) -> Option<u64> {
        self.smarts.iter().map(|s| s.id).max().unwrap_or(0).checked_add(1)
    }
    pub fn next_template_id(&self) -> Option<u64> {
        self.templates.iter().map(|t| t.id).max().unwrap_or(0).checked_add(1)
    }
    /// Section ids are unique across all lists.
    pub fn next_section_id(&self) -> Option<u64> {
        self.lists.iter().flat_map(|l| l.sections.iter()).map(|s| s.id).max().unwrap_or(0).checked_add(1)
    }
    /// Deleting a list removes its reminders; the last list cannot go.
    pub fn delete_list(&mut self, id: u64) -> bool {
        if self.lists.len() <= 1 || !self.lists.iter().any(|l| l.id == id) {
            return false;
        }
        self.lists.retain(|l| l.id != id);
        self.items.retain(|r| r.list != id);
        // a smart list that asked for this list takes every list instead
        for s in self.smarts.iter_mut().filter(|s| s.rules.list == Some(id)) {
            s.rules.list = None;
        }
        true
    }
    pub fn move_list(&mut self, id: u64, delta: i32) -> bool {
        let Some(i) = self.lists.iter().position(|l| l.id == id) else {
            return false;
        };
        let j = i as i64 + delta as i64;
        if j < 0 || j >= self.lists.len() as i64 || j == i as i64 {
            return false;
        }
        self.lists.swap(i, j as usize);
        true
    }
    /// Deleting a group keeps its lists; they move out of any group.
    pub fn delete_group(&mut self, id: u64) -> bool {
        if !self.groups.iter().any(|g| g.id == id) {
            return false;
        }
        self.groups.retain(|g| g.id != id);
        for l in self.lists.iter_mut().filter(|l| l.group == Some(id)) {
            l.group = None;
        }
        true
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Smart {
    Today,
    Scheduled,
    Flagged,
    All,
    Completed,
}

impl Smart {
    pub const ALL: [Self; 5] = [Self::Today, Self::Scheduled, Self::All, Self::Flagged, Self::Completed];
    /// The position in [`Smart::ALL`].
    pub fn index(self) -> usize {
        Self::ALL.iter().position(|s| *s == self).unwrap_or(0)
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Today => "Today",
            Self::Scheduled => "Scheduled",
            Self::Flagged => "Flagged",
            Self::All => "All",
            Self::Completed => "Completed",
        }
    }
    /// The smart list as a fetch predicate over the reminders. A reminder without a day never matches a day test.
    pub fn predicate(self, today: i64) -> Predicate {
        let open = Predicate::compare("completed", Op::Eq, Field::Bool(false));
        match self {
            Self::Today => open.and(Predicate::compare("due", Op::Le, Field::Int(today))),
            Self::Scheduled => open.and(Predicate::compare("scheduled", Op::Eq, Field::Bool(true))),
            Self::Flagged => open.and(Predicate::compare("flagged", Op::Eq, Field::Bool(true))),
            Self::All => open,
            Self::Completed => Predicate::compare("completed", Op::Eq, Field::Bool(true)),
        }
    }
    pub fn matches(self, r: &Reminder, today: i64) -> bool {
        self.predicate(today).matches(r)
    }
    /// The number of reminders the smart list shows, from a fetch request over `items`.
    pub fn count(self, items: &[Reminder], today: i64) -> usize {
        let request = FetchRequest { predicate: self.predicate(today), ..FetchRequest::default() };
        fetch_indices(items, &request).len()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum View {
    Smart(Smart),
    List(u64),
    /// Every reminder carrying the tag with this key (`tag_key`).
    Tag(u64),
    /// The custom smart list with this id.
    Custom(u64),
}

pub fn list_count(items: &[Reminder], list: u64) -> usize {
    items.iter().filter(|r| r.list == list && !r.completed).count()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sort {
    Manual,
    Due,
    Priority,
    Title,
    /// Oldest first; a reminder from before creation dates were kept counts as the oldest.
    Created,
}

impl Sort {
    pub const ALL: [Self; 5] = [Self::Manual, Self::Due, Self::Priority, Self::Title, Self::Created];
    pub fn label(self) -> &'static str {
        match self {
            Self::Manual => "Manual",
            Self::Due => "Due Date",
            Self::Priority => "Priority",
            Self::Title => "Title",
            Self::Created => "Creation Date",
        }
    }
    /// The word the settings file stores this order as.
    pub fn key(self) -> &'static str {
        match self {
            Self::Manual => "manual",
            Self::Due => "due",
            Self::Priority => "priority",
            Self::Title => "title",
            Self::Created => "created",
        }
    }
    pub fn from_key(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|s| s.key() == text)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Group {
    None,
    Due,
    Priority,
    List,
}

impl Group {
    pub const ALL: [Self; 4] = [Self::None, Self::Due, Self::Priority, Self::List];
    pub fn label(self) -> &'static str {
        match self {
            Self::None => "None",
            Self::Due => "Due Date",
            Self::Priority => "Priority",
            Self::List => "List",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Options {
    pub sort: Sort,
    pub group: Group,
    pub show_completed: bool,
    /// The list new reminders from a smart list go to; None is the first list.
    pub default_list: Option<u64>,
    /// Bit `Smart::index` is set for a smart list whose tile is hidden in the sidebar.
    pub hidden_smart: u8,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            sort: Sort::Manual,
            group: Group::None,
            show_completed: false,
            default_list: None,
            hidden_smart: 0,
        }
    }
}

impl Options {
    /// Whether the sidebar shows the tile of smart list `s`.
    pub fn smart_shown(&self, s: Smart) -> bool {
        self.hidden_smart & (1 << s.index()) == 0
    }
    /// Shows a hidden smart list tile, or hides a shown one.
    pub fn toggle_smart(&mut self, s: Smart) {
        self.hidden_smart ^= 1 << s.index();
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Row {
    Header(String),
    /// The header of a section of the open list; choosing it edits the section.
    Section { id: u64, name: String },
    Item { idx: usize, depth: u8 },
}

pub fn priority_label(p: u8) -> &'static str {
    match p {
        1 => "Low",
        2 => "Medium",
        3 => "High",
        _ => "None",
    }
}

fn due_bucket(due: Option<i64>, today: i64) -> (u8, &'static str) {
    match due {
        None => (5, "No Date"),
        Some(d) if d < today => (0, "Overdue"),
        Some(d) if d == today => (1, "Today"),
        Some(d) if d == today + 1 => (2, "Tomorrow"),
        Some(d) if d <= today + 7 => (3, "This Week"),
        Some(_) => (4, "Later"),
    }
}

fn contains_ci(hay: &str, needle_lower: &str) -> bool {
    hay.to_lowercase().contains(needle_lower)
}

pub fn matches_query(r: &Reminder, query: &str) -> bool {
    let q = query.trim().to_lowercase();
    q.is_empty() || contains_ci(&r.title, &q) || contains_ci(&r.notes, &q)
}

fn sort_key(r: &Reminder, sort: Sort) -> (i64, i64, String) {
    match sort {
        Sort::Manual => (0, 0, String::new()),
        Sort::Due => (
            r.due.unwrap_or(i64::MAX),
            r.time.map_or(i64::MAX, i64::from),
            String::new(),
        ),
        Sort::Priority => (if r.priority == 0 { 9 } else { 4 - r.priority as i64 }, 0, String::new()),
        Sort::Title => (0, 0, r.title.to_lowercase()),
        Sort::Created => (r.created.unwrap_or(i64::MIN), 0, String::new()),
    }
}

/// The rows to draw for `view`. A non-empty `query` searches every
/// reminder (flat); lists nest subtasks one level deep under their parent,
/// smart lists are flat.
pub fn visible(
    data: &Data,
    view: View,
    opts: Options,
    today: i64,
    query: &str,
    collapsed: &HashSet<u64>,
) -> Vec<Row> {
    let items = &data.items;
    let searching = !query.trim().is_empty();
    let pass = |r: &Reminder| -> bool {
        if searching {
            return matches_query(r, query);
        }
        match view {
            View::Smart(Smart::Completed) => r.completed,
            View::Smart(s) => s.matches(r, today) || (opts.show_completed && r.completed && s == Smart::All),
            View::List(l) => r.list == l && (opts.show_completed || !r.completed),
            View::Tag(key) => r.tags().iter().any(|t| tag_key(t) == key) && (opts.show_completed || !r.completed),
            View::Custom(id) => data.smarts.iter().find(|s| s.id == id).is_some_and(|s| s.rules.matches(r, today)),
        }
    };
    let nested = !searching && matches!(view, View::List(_));
    let passing: Vec<usize> = (0..items.len()).filter(|&i| pass(&items[i])).collect();
    let passing_ids: HashSet<u64> = passing.iter().map(|&i| items[i].id).collect();
    let is_child = |r: &Reminder| nested && r.parent.is_some_and(|p| passing_ids.contains(&p));
    let mut tops: Vec<usize> = passing.iter().copied().filter(|&i| !is_child(&items[i])).collect();
    // Stable sorts keep manual (storage) order inside equal keys.
    tops.sort_by_cached_key(|&i| sort_key(&items[i], opts.sort));
    let list_pos = |id: u64| data.lists.iter().position(|l| l.id == id).unwrap_or(usize::MAX);
    let group_key = |r: &Reminder| -> (usize, String) {
        match opts.group {
            Group::None => (0, String::new()),
            Group::Due => {
                let (k, label) = due_bucket(r.due, today);
                (k as usize, label.to_string())
            }
            Group::Priority => {
                let k = if r.priority == 0 { 9 } else { 4 - r.priority as usize };
                (k, format!("{} Priority", priority_label(r.priority)))
            }
            Group::List => (
                list_pos(r.list),
                data.lists
                    .iter()
                    .find(|l| l.id == r.list)
                    .map_or(String::new(), |l| l.name.clone()),
            ),
        }
    };
    if opts.group != Group::None {
        tops.sort_by_cached_key(|&i| group_key(&items[i]).0);
    }
    let mut rows = vec![];
    // The open list's sections: unplaced reminders first, then each section under its header (empty ones too).
    let sections: &[Section] = match view {
        View::List(l) if !searching && opts.group == Group::None => {
            data.lists.iter().find(|d| d.id == l).map_or(&[][..], |d| d.sections.as_slice())
        }
        _ => &[],
    };
    if !sections.is_empty() {
        let known = |r: &Reminder| r.section.is_some_and(|s| sections.iter().any(|x| x.id == s));
        for &i in tops.iter().filter(|&&i| !known(&items[i])) {
            push_item(&mut rows, items, &passing, nested, collapsed, i);
        }
        for sec in sections {
            rows.push(Row::Section { id: sec.id, name: sec.name.clone() });
            for &i in tops.iter().filter(|&&i| items[i].section == Some(sec.id)) {
                push_item(&mut rows, items, &passing, nested, collapsed, i);
            }
        }
        return rows;
    }
    let mut last: Option<(usize, String)> = None;
    for i in tops {
        if opts.group != Group::None {
            let key = group_key(&items[i]);
            if last.as_ref() != Some(&key) {
                rows.push(Row::Header(key.1.clone()));
                last = Some(key);
            }
        }
        push_item(&mut rows, items, &passing, nested, collapsed, i);
    }
    rows
}

/// A top-level reminder's row, and its subtasks' rows under it unless it is collapsed.
fn push_item(rows: &mut Vec<Row>, items: &[Reminder], passing: &[usize], nested: bool, collapsed: &HashSet<u64>, i: usize) {
    rows.push(Row::Item { idx: i, depth: 0 });
    if nested && !collapsed.contains(&items[i].id) {
        for &c in passing {
            if items[c].parent == Some(items[i].id) {
                rows.push(Row::Item { idx: c, depth: 1 });
            }
        }
    }
}

pub fn has_children(items: &[Reminder], id: u64) -> bool {
    items.iter().any(|r| r.parent == Some(id))
}

/// (completed, total) subtasks of `id`.
pub fn progress(items: &[Reminder], id: u64) -> (usize, usize) {
    let kids = items.iter().filter(|r| r.parent == Some(id));
    (kids.clone().filter(|r| r.completed).count(), kids.count())
}

/// Makes `id` a subtask of the previous top-level reminder of its list.
pub fn indent(items: &mut [Reminder], id: u64) -> bool {
    let Some(pos) = items.iter().position(|r| r.id == id) else {
        return false;
    };
    if items[pos].parent.is_some() || has_children(items, id) {
        return false;
    }
    let list = items[pos].list;
    let Some(parent) = items[..pos]
        .iter()
        .rev()
        .find(|r| r.list == list && r.parent.is_none())
        .map(|r| r.id)
    else {
        return false;
    };
    items[pos].parent = Some(parent);
    true
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Completion {
    Done,
    Undone,
    /// A repeating reminder moved to its next occurrence instead.
    Advanced(i64),
}

pub fn toggle_complete(items: &mut [Reminder], id: u64, today: i64) -> Option<Completion> {
    let r = items.iter_mut().find(|r| r.id == id)?;
    if r.completed {
        r.completed = false;
        return Some(Completion::Undone);
    }
    if let (Some(rule), Some(due)) = (r.repeat, r.due) {
        let next = next_occurrence(due, rule, today);
        r.due = Some(next);
        return Some(Completion::Advanced(next));
    }
    r.completed = true;
    Some(Completion::Done)
}

/// Removes `id` and its subtasks, returning (index, reminder) pairs for undo.
pub fn remove_with_children(items: &mut Vec<Reminder>, id: u64) -> Vec<(usize, Reminder)> {
    let mut removed = vec![];
    let mut i = 0;
    let mut offset = 0;
    while i < items.len() {
        if items[i].id == id || items[i].parent == Some(id) {
            removed.push((i + offset, items.remove(i)));
            offset += 1;
        } else {
            i += 1;
        }
    }
    // Indices above are positions in the original vector, ascending.
    removed
}

pub fn restore(items: &mut Vec<Reminder>, removed: Vec<(usize, Reminder)>) {
    for (i, r) in removed {
        items.insert(i.min(items.len()), r);
    }
}

/// Removes every completed reminder in `list`, and the subtasks of those, for undo.
pub fn remove_completed(items: &mut Vec<Reminder>, list: u64) -> Vec<(usize, Reminder)> {
    let done: std::collections::HashSet<u64> = items.iter().filter(|r| r.completed && r.list == list).map(|r| r.id).collect();
    let mut removed = vec![];
    let mut i = 0;
    let mut offset = 0;
    while i < items.len() {
        let gone = done.contains(&items[i].id) || items[i].parent.is_some_and(|p| done.contains(&p));
        if gone {
            removed.push((i + offset, items.remove(i)));
            offset += 1;
        } else {
            i += 1;
        }
    }
    removed
}

/// Moves `id` (with its subtasks) before top-level reminder `before`, or to
/// the end when `None`.
pub fn move_before(items: &mut Vec<Reminder>, id: u64, before: Option<u64>) -> bool {
    if Some(id) == before || !items.iter().any(|r| r.id == id) {
        return false;
    }
    if items.iter().find(|r| r.id == id).is_some_and(|r| r.parent.is_some()) {
        return false;
    }
    let mut block = vec![];
    let mut rest = vec![];
    for r in items.drain(..) {
        if r.id == id || r.parent == Some(id) {
            block.push(r);
        } else {
            rest.push(r);
        }
    }
    let at = before
        .and_then(|b| rest.iter().position(|r| r.id == b))
        .unwrap_or(rest.len());
    for (k, r) in block.into_iter().enumerate() {
        rest.insert(at + k, r);
    }
    *items = rest;
    true
}

/// Zone offset in seconds east of UTC at a Unix time.
pub type OffsetAt<'a> = &'a dyn Fn(i64) -> i32;

/// One Markdown checklist line for `r`: `- [ ]` or `- [x]`, the title on one line, the due day (and time) in brackets.
fn checklist_line(out: &mut String, r: &Reminder, indent: &str) {
    let mark = if r.completed { 'x' } else { ' ' };
    let title = r.title.split_whitespace().collect::<Vec<_>>().join(" ");
    out.push_str(&format!("{indent}- [{mark}] {title}"));
    if let Some(day) = r.due {
        let (y, m, d) = civil_from_days(day);
        out.push_str(&format!(" ({y:04}-{m:02}-{d:02}"));
        if let Some(t) = r.time {
            out.push_str(&format!(" {:02}:{:02}", t / 60, t % 60));
        }
        out.push(')');
    }
    out.push('\n');
}

/// The reminders of a smart list (or any flat selection) as a Markdown checklist under `title`, one line each in the
/// order given, the way File > Print List prints a smart list.
pub fn flat_markdown(title: &str, items: &[&Reminder]) -> String {
    let mut out = format!("# {title}\n\n");
    for r in items {
        checklist_line(&mut out, r, "");
    }
    out
}

/// A list as Markdown checklist lines: `- [ ]` and `- [x]`, subtasks indented, the
/// due day (and time) in brackets. Each section of the list is a `## heading` over its reminders.
pub fn export_markdown(data: &Data, list: u64) -> String {
    // the top-level reminders of one section (or of none), each followed by its subtasks
    fn block(out: &mut String, data: &Data, list: u64, known: &[Section], section: Option<u64>) {
        let placed = |r: &Reminder| r.section.filter(|s| known.iter().any(|k| k.id == *s));
        for r in data.items.iter().filter(|r| r.list == list && r.parent.is_none() && placed(r) == section) {
            checklist_line(out, r, "");
            for c in data.items.iter().filter(|c| c.parent == Some(r.id)) {
                checklist_line(out, c, "  ");
            }
        }
    }
    let name = data.lists.iter().find(|l| l.id == list).map_or("Reminders", |l| l.name.as_str());
    let known: &[Section] = data.lists.iter().find(|l| l.id == list).map_or(&[][..], |l| l.sections.as_slice());
    let mut out = format!("# {name}\n\n");
    block(&mut out, data, list, known, None);
    for s in known {
        if !out.ends_with("\n\n") {
            out.push('\n');
        }
        out.push_str(&format!("## {}\n\n", s.name));
        block(&mut out, data, list, known, Some(s.id));
    }
    out
}

/// Unix seconds of the due moment: the wall time on the due day in the zone
/// as it is then, not as it is today (daylight saving may differ). A reminder with a day but no time is due at
/// `all_day_min` minutes after midnight (the all-day alert time in Settings).
pub fn due_unix(r: &Reminder, all_day_min: u16, offset_at: OffsetAt) -> Option<i64> {
    let day = r.due?;
    let secs = r.time.unwrap_or(all_day_min) as i64 * 60;
    Some(match r.tz.as_deref().and_then(crate::zones::resolve) {
        Some(zone) => springboard::shell::clock::alarm::local_to_unix(day, secs, &|t| zone.at(t).0),
        None => springboard::shell::clock::alarm::local_to_unix(day, secs, offset_at),
    })
}

/// "9:05 AM" for minute 545.
pub fn clock_label(min: u16) -> String {
    let (h, m) = (min / 60 % 24, min % 60);
    format!("{}:{:02} {}", if h % 12 == 0 { 12 } else { h % 12 }, m, if h < 12 { "AM" } else { "PM" })
}

/// Notification body of an alert for `r`.
pub fn alert_text(r: &Reminder, all_day_min: u16, early: bool) -> String {
    match (early, r.early) {
        (true, Some(m)) => format!("Due in {}", early_label(m).trim_end_matches(" before")),
        _ => format!("Due {}", clock_label(r.time.unwrap_or(all_day_min))),
    }
}

/// Notification body of the repeating alarm of an urgent reminder.
pub fn urgent_text(r: &Reminder, all_day_min: u16) -> String {
    format!("Urgent: {}", alert_text(r, all_day_min, false))
}

pub fn early_label(minutes: u32) -> String {
    match minutes {
        m if m % 1440 == 0 => format!("{} day{} before", m / 1440, if m == 1440 { "" } else { "s" }),
        m if m % 60 == 0 => format!("{} hour{} before", m / 60, if m == 60 { "" } else { "s" }),
        m => format!("{m} minutes before"),
    }
}

/// Early-reminder presets, cycled in the editor.
pub fn cycle_early(current: Option<u32>) -> Option<u32> {
    const STEPS: [u32; 5] = [5, 15, 60, 1440, 2880];
    match current {
        None => Some(STEPS[0]),
        Some(m) => STEPS.iter().position(|&s| s == m).and_then(|i| STEPS.get(i + 1).copied()),
    }
}

/// Open reminders due today or earlier: the number on the dock badge (as in Reminders).
pub fn due_count(items: &[Reminder], today: i64) -> usize {
    items.iter().filter(|r| !r.completed && r.due.is_some_and(|d| d <= today)).count()
}

/// The dock badge text for `count`: nothing for zero, "99+" past 99.
pub fn badge_label(count: usize) -> String {
    match count {
        0 => String::new(),
        1..=99 => count.to_string(),
        _ => "99+".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repeat::Unit;

    #[test]
    fn the_badge_counts_open_reminders_due_today_or_earlier() {
        let mut overdue = Reminder::new(1, "overdue");
        overdue.due = Some(19_999);
        let mut today = Reminder::new(2, "today");
        today.due = Some(20_000);
        let mut later = Reminder::new(3, "later");
        later.due = Some(20_001);
        let undated = Reminder::new(4, "undated");
        let mut done = Reminder::new(5, "done");
        done.due = Some(19_990);
        done.completed = true;
        assert_eq!(due_count(&[overdue, today, later, undated, done], 20_000), 2);
        assert_eq!(due_count(&[], 20_000), 0);
    }
    #[test]
    fn smart_lists_are_fetch_predicates_over_the_reminders() {
        // today is 20_000: overdue 19_999, today 20_000, later 20_001, flagged without a day, plain without a day,
        // and a completed one that was due yesterday.
        let mut overdue = Reminder::new(1, "overdue");
        overdue.due = Some(19_999);
        let mut today = Reminder::new(2, "today");
        today.due = Some(20_000);
        let mut later = Reminder::new(3, "later");
        later.due = Some(20_001);
        let mut flagged = Reminder::new(4, "flagged");
        flagged.flagged = true;
        let plain = Reminder::new(5, "plain");
        let mut done = Reminder::new(6, "done");
        done.due = Some(19_990);
        done.completed = true;
        let items = [overdue, today, later, flagged, plain, done];
        assert_eq!(Smart::Today.count(&items, TODAY), 2, "open and due on or before today");
        assert_eq!(Smart::Scheduled.count(&items, TODAY), 3, "open with a day, any day");
        assert_eq!(Smart::Flagged.count(&items, TODAY), 1);
        assert_eq!(Smart::All.count(&items, TODAY), 5, "every open reminder");
        assert_eq!(Smart::Completed.count(&items, TODAY), 1);
        assert!(!Smart::Today.matches(&items[3], TODAY), "no day never matches a day test");
        assert!(Smart::Flagged.matches(&items[3], TODAY));
        // The count follows the day: tomorrow the reminder due 20_001 is due today too; yesterday only the overdue one.
        assert_eq!(Smart::Today.count(&items, TODAY + 1), 3);
        assert_eq!(Smart::Today.count(&items, TODAY - 1), 1, "only the one due before yesterday");
    }

    #[test]
    fn the_badge_label_is_empty_for_zero_and_caps_at_99() {
        assert_eq!(badge_label(0), "");
        assert_eq!(badge_label(7), "7");
        assert_eq!(badge_label(99), "99");
        assert_eq!(badge_label(100), "99+");
    }
    const TODAY: i64 = 20_000;
    fn r(id: u64, title: &str) -> Reminder {
        Reminder::new(id, title)
    }
    fn data(items: Vec<Reminder>) -> Data {
        Data { items, ..Data::default() }
    }
    fn ids(rows: &[Row], items: &[Reminder]) -> Vec<(u64, u8)> {
        rows.iter()
            .filter_map(|row| match row {
                Row::Item { idx, depth } => Some((items[*idx].id, *depth)),
                _ => None,
            })
            .collect()
    }
    #[test]
    fn smart_list_filters_and_counts() {
        let mut a = r(1, "a");
        a.due = Some(TODAY - 1);
        let mut b = r(2, "b");
        b.due = Some(TODAY + 3);
        b.flagged = true;
        let mut c = r(3, "c");
        c.completed = true;
        c.flagged = true;
        let d = r(4, "d");
        let items = vec![a, b, c, d];
        assert_eq!(Smart::Today.count(&items, TODAY), 1);
        assert_eq!(Smart::Scheduled.count(&items, TODAY), 2);
        assert_eq!(Smart::Flagged.count(&items, TODAY), 1);
        assert_eq!(Smart::All.count(&items, TODAY), 3);
        assert_eq!(Smart::Completed.count(&items, TODAY), 1);
        assert_eq!(list_count(&items, DEFAULT_LIST), 3);
    }
    #[test]
    fn tags_are_hashtags_in_title_and_notes() {
        assert_eq!(tags_in("Buy #Milk, #milk and #Bread-1 #2024 C#sharp # x"), vec!["Milk", "Bread-1"]);
        assert_eq!(tags_in("##a"), Vec::<String>::new(), "a tag starts with a letter after one #");
        assert_eq!(tags_in("#work-"), vec!["work"], "a trailing hyphen is not part of the tag");
        let mut call = r(1, "Call #mom");
        call.notes = "#mom and #Family".into();
        assert_eq!(call.tags(), vec!["mom", "Family"]);
        assert_ne!(tag_key("Home"), tag_key("Work"));
        assert_eq!(tag_key("HOME"), tag_key("home"));
    }
    #[test]
    fn tag_counts_count_open_reminders_and_sort_by_name() {
        let mut done = r(2, "#home #work");
        done.completed = true;
        let items = vec![r(1, "#Home chores"), done, r(3, "Work #Work")];
        assert_eq!(tag_counts(&items), vec![("Home".to_string(), 1), ("work".to_string(), 1)]);
        assert_eq!(tag_name(&items, tag_key("WORK")), Some("work".to_string()));
        assert_eq!(tag_name(&items, tag_key("gym")), None);
    }
    #[test]
    fn tag_view_lists_the_reminders_with_that_tag() {
        let mut done = r(2, "#home done");
        done.completed = true;
        let items = vec![r(1, "#Home"), done, r(3, "no tag")];
        let d = data(items.clone());
        let none = HashSet::new();
        let key = tag_key("home");
        let rows = visible(&d, View::Tag(key), Options::default(), TODAY, "", &none);
        assert_eq!(ids(&rows, &items), vec![(1, 0)], "completed ones are hidden as in a list");
        let opts = Options { show_completed: true, ..Options::default() };
        let rows = visible(&d, View::Tag(key), opts, TODAY, "", &none);
        assert_eq!(ids(&rows, &items), vec![(1, 0), (2, 0)]);
    }
    #[test]
    fn list_view_nests_subtasks_and_collapses() {
        let mut kid = r(2, "kid");
        kid.parent = Some(1);
        let items = vec![r(1, "parent"), kid, r(3, "other")];
        let d = data(items);
        let none = HashSet::new();
        let rows = visible(&d, View::List(1), Options::default(), TODAY, "", &none);
        assert_eq!(ids(&rows, &d.items), vec![(1, 0), (2, 1), (3, 0)]);
        let collapsed: HashSet<u64> = [1].into();
        let rows = visible(&d, View::List(1), Options::default(), TODAY, "", &collapsed);
        assert_eq!(ids(&rows, &d.items), vec![(1, 0), (3, 0)]);
        let rows = visible(&d, View::Smart(Smart::All), Options::default(), TODAY, "", &none);
        assert_eq!(ids(&rows, &d.items), vec![(1, 0), (2, 0), (3, 0)]);
    }
    #[test]
    fn completed_parent_hidden_promotes_open_subtasks() {
        let mut p = r(1, "p");
        p.completed = true;
        let mut kid = r(2, "kid");
        kid.parent = Some(1);
        let d = data(vec![p, kid]);
        let none = HashSet::new();
        let rows = visible(&d, View::List(1), Options::default(), TODAY, "", &none);
        assert_eq!(ids(&rows, &d.items), vec![(2, 0)]);
        let opts = Options { show_completed: true, ..Options::default() };
        let rows = visible(&d, View::List(1), opts, TODAY, "", &none);
        assert_eq!(ids(&rows, &d.items), vec![(1, 0), (2, 1)]);
    }
    #[test]
    fn sort_and_group() {
        let mut a = r(1, "banana");
        a.due = Some(TODAY + 20);
        a.priority = 1;
        let mut b = r(2, "apricot");
        b.due = Some(TODAY);
        b.priority = 3;
        let c = r(3, "cherry");
        let d = data(vec![a, b, c]);
        let none = HashSet::new();
        let o = |sort, group| Options { sort, group, ..Options::default() };
        let rows = visible(&d, View::List(1), o(Sort::Title, Group::None), TODAY, "", &none);
        assert_eq!(ids(&rows, &d.items), vec![(2, 0), (1, 0), (3, 0)]);
        let rows = visible(&d, View::List(1), o(Sort::Due, Group::None), TODAY, "", &none);
        assert_eq!(ids(&rows, &d.items), vec![(2, 0), (1, 0), (3, 0)]);
        let rows = visible(&d, View::List(1), o(Sort::Priority, Group::None), TODAY, "", &none);
        assert_eq!(ids(&rows, &d.items), vec![(2, 0), (1, 0), (3, 0)]);
        let rows = visible(&d, View::List(1), o(Sort::Manual, Group::Due), TODAY, "", &none);
        assert_eq!(
            rows,
            vec![
                Row::Header("Today".into()),
                Row::Item { idx: 1, depth: 0 },
                Row::Header("Later".into()),
                Row::Item { idx: 0, depth: 0 },
                Row::Header("No Date".into()),
                Row::Item { idx: 2, depth: 0 },
            ]
        );
        let rows = visible(&d, View::List(1), o(Sort::Manual, Group::Priority), TODAY, "", &none);
        assert!(matches!(&rows[0], Row::Header(h) if h == "High Priority"));
    }
    #[test]
    fn search_spans_lists_and_completed_items() {
        let mut a = r(1, "Buy Milk");
        a.completed = true;
        let mut b = r(2, "call");
        b.notes = "ask about MILK".into();
        b.list = 2;
        let d = data(vec![a, b, r(3, "other")]);
        let rows = visible(&d, View::List(1), Options::default(), TODAY, "milk", &HashSet::new());
        assert_eq!(ids(&rows, &d.items), vec![(1, 0), (2, 0)]);
    }
    #[test]
    fn indent_outdent_rules() {
        let mut items = vec![r(1, "a"), r(2, "b"), r(3, "c")];
        assert!(!indent(&mut items, 1), "first row has nothing to nest under");
        assert!(indent(&mut items, 2));
        assert_eq!(items[1].parent, Some(1));
        assert!(!indent(&mut items, 1), "a parent with children cannot nest");
        assert!(indent(&mut items, 3));
        assert_eq!(items[2].parent, Some(1), "nests under the previous top-level");
        assert_eq!(progress(&items, 1), (0, 2));
        items[1].completed = true;
        assert_eq!(progress(&items, 1), (1, 2));
    }
    #[test]
    fn completion_advances_repeats() {
        let mut items = vec![r(1, "x"), r(2, "y")];
        assert_eq!(toggle_complete(&mut items, 1, TODAY), Some(Completion::Done));
        assert_eq!(toggle_complete(&mut items, 1, TODAY), Some(Completion::Undone));
        items[1].due = Some(TODAY);
        items[1].repeat = Some(Repeat::new(Unit::Week, 1));
        assert_eq!(toggle_complete(&mut items, 2, TODAY), Some(Completion::Advanced(TODAY + 7)));
        assert!(!items[1].completed);
        assert_eq!(items[1].due, Some(TODAY + 7));
        assert_eq!(toggle_complete(&mut items, 9, TODAY), None);
    }
    #[test]
    fn delete_and_undo_restore_positions() {
        let mut kid = r(2, "kid");
        kid.parent = Some(1);
        let original = vec![r(0, "z"), r(1, "p"), kid, r(3, "q")];
        let mut items = original.clone();
        let removed = remove_with_children(&mut items, 1);
        assert_eq!(items.len(), 2);
        assert_eq!(removed.len(), 2);
        restore(&mut items, removed);
        assert_eq!(items, original);
    }
    #[test]
    fn reorder_moves_blocks() {
        let mut kid = r(2, "kid");
        kid.parent = Some(1);
        let mut items = vec![r(1, "p"), kid, r(3, "q"), r(4, "s")];
        assert!(move_before(&mut items, 1, None));
        let order: Vec<u64> = items.iter().map(|r| r.id).collect();
        assert_eq!(order, vec![3, 4, 1, 2]);
        assert!(move_before(&mut items, 4, Some(3)));
        let order: Vec<u64> = items.iter().map(|r| r.id).collect();
        assert_eq!(order, vec![4, 3, 1, 2]);
        assert!(!move_before(&mut items, 2, Some(3)), "subtasks do not drag alone");
        assert!(!move_before(&mut items, 3, Some(3)));
    }
    #[test]
    fn list_management() {
        let mut d = Data::default();
        d.lists.push(ListDef::new(2, "Work", 1, 1));
        let mut x = r(1, "w");
        x.list = 2;
        d.items = vec![x, r(2, "h")];
        assert!(d.move_list(2, -1));
        assert_eq!(d.lists[0].id, 2);
        assert!(!d.move_list(2, -1));
        assert_eq!(d.next_list_id(), Some(3));
        assert!(d.delete_list(2));
        assert_eq!(d.items.len(), 1);
        assert!(!d.delete_list(1), "the last list stays");
    }
    #[test]
    fn smart_rules_take_open_reminders_that_satisfy_every_rule() {
        let mut overdue = r(1, "overdue #home");
        overdue.due = Some(TODAY - 2);
        overdue.time = Some(9 * 60);
        overdue.flagged = true;
        overdue.priority = 3;
        let mut tonight = r(2, "tonight");
        tonight.due = Some(TODAY);
        tonight.time = Some(19 * 60);
        tonight.list = 2;
        let mut later = r(3, "later");
        later.due = Some(TODAY + 7);
        later.time = Some(13 * 60);
        let undated = r(4, "undated");
        let mut done = r(5, "done");
        done.due = Some(TODAY);
        done.completed = true;
        let items = [overdue, tonight, later, undated, done];
        let ids = |rules: SmartRules| items.iter().filter(|x| rules.matches(x, TODAY)).map(|x| x.id).collect::<Vec<_>>();
        let any = SmartRules::default();
        assert_eq!(ids(any), [1, 2, 3, 4], "no rule takes every open reminder");
        assert_eq!(ids(SmartRules { due: DueRule::Overdue, ..any }), [1]);
        assert_eq!(ids(SmartRules { due: DueRule::Today, ..any }), [2]);
        assert_eq!(ids(SmartRules { due: DueRule::Next7, ..any }), [2], "today up to six days ahead");
        assert_eq!(ids(SmartRules { due: DueRule::Later, ..any }), [3], "a week from today is later");
        assert_eq!(ids(SmartRules { due: DueRule::NoDate, ..any }), [4]);
        assert_eq!(ids(SmartRules { time: TimeRule::Morning, ..any }), [1]);
        assert_eq!(ids(SmartRules { time: TimeRule::Afternoon, ..any }), [3]);
        assert_eq!(ids(SmartRules { time: TimeRule::Evening, ..any }), [2]);
        assert_eq!(ids(SmartRules { time: TimeRule::NoTime, ..any }), [4]);
        assert_eq!(ids(SmartRules { flagged: FlagRule::Flagged, ..any }), [1]);
        assert_eq!(ids(SmartRules { flagged: FlagRule::Unflagged, ..any }), [2, 3, 4]);
        assert_eq!(ids(SmartRules { priority: 1, ..any }), [1]);
        assert_eq!(ids(SmartRules { priority: 3, ..any }), [1]);
        assert_eq!(ids(SmartRules { list: Some(2), ..any }), [2]);
        assert_eq!(ids(SmartRules { tag: Some(tag_key("HOME")), ..any }), [1], "tags match without case");
        let both = SmartRules { due: DueRule::Overdue, flagged: FlagRule::Flagged, priority: 2, tag: Some(tag_key("home")), ..any };
        assert_eq!(ids(both), [1]);
        assert_eq!(ids(SmartRules { due: DueRule::Overdue, list: Some(2), ..any }), Vec::<u64>::new(), "every rule must hold");
    }
    #[test]
    fn smart_rule_choices_cycle_and_round_trip_their_codes() {
        let mut due = DueRule::Any;
        let mut seen = vec![];
        for _ in 0..DueRule::ALL.len() {
            seen.push(due);
            due = due.next();
        }
        assert_eq!(seen, DueRule::ALL);
        assert_eq!(due, DueRule::Any, "the last choice wraps to the first");
        for d in DueRule::ALL {
            assert_eq!(DueRule::from_code(d.code()), Some(d));
        }
        for t in TimeRule::ALL {
            assert_eq!(TimeRule::from_code(t.code()), Some(t));
        }
        for f in FlagRule::ALL {
            assert_eq!(FlagRule::from_code(f.code()), Some(f));
        }
        assert_eq!(DueRule::from_code(DueRule::ALL.len() as u8), None);
        assert_eq!(TimeRule::NoTime.next(), TimeRule::Any);
        assert_eq!(FlagRule::Unflagged.next(), FlagRule::Any);
        assert_eq!(priority_rule_label(0), "Any");
        assert_eq!(priority_rule_label(3), "High");
    }
    #[test]
    fn the_tag_and_list_choices_walk_what_exists_then_return_to_any() {
        let items = vec![r(1, "#Work"), r(2, "#home and #Work")];
        let (home, work) = (tag_key("home"), tag_key("work"));
        assert_eq!(next_tag(None, &items), Some(home), "tags are in name order");
        assert_eq!(next_tag(Some(home), &items), Some(work));
        assert_eq!(next_tag(Some(work), &items), None);
        assert_eq!(next_tag(Some(tag_key("gone")), &items), None, "a tag nobody uses any more");
        assert_eq!(next_tag(None, &[]), None);
        let mut d = Data::default();
        d.lists.push(ListDef::new(2, "Work", 1, 1));
        assert_eq!(next_list(None, &d.lists), Some(1));
        assert_eq!(next_list(Some(1), &d.lists), Some(2));
        assert_eq!(next_list(Some(2), &d.lists), None);
        assert_eq!(next_list(Some(9), &d.lists), None);
    }
    #[test]
    fn a_custom_smart_list_view_shows_its_matches_flat() {
        let mut a = r(1, "a");
        a.flagged = true;
        let mut kid = r(2, "kid");
        kid.parent = Some(1);
        kid.flagged = true;
        let mut d = data(vec![a, kid, r(3, "c")]);
        d.smarts.push(SmartDef { id: 4, name: "Flagged things".into(), color: 0, rules: SmartRules { flagged: FlagRule::Flagged, ..SmartRules::default() } });
        let rows = visible(&d, View::Custom(4), Options::default(), TODAY, "", &HashSet::new());
        assert_eq!(ids(&rows, &d.items), vec![(1, 0), (2, 0)], "a subtask is its own row, as in the other smart lists");
        let missing = visible(&d, View::Custom(9), Options::default(), TODAY, "", &HashSet::new());
        assert!(missing.is_empty());
    }
    #[test]
    fn deleting_a_list_frees_the_smart_lists_that_asked_for_it() {
        let mut d = Data::default();
        d.lists.push(ListDef::new(2, "Work", 1, 1));
        d.smarts.push(SmartDef { id: 1, name: "Work things".into(), color: 0, rules: SmartRules { list: Some(2), ..SmartRules::default() } });
        assert_eq!(d.next_smart_id(), Some(2));
        assert!(d.delete_list(2));
        assert_eq!(d.smarts[0].rules.list, None);
    }
    #[test]
    fn early_presets_cycle() {
        let mut e = None;
        let mut seen = vec![];
        for _ in 0..6 {
            e = cycle_early(e);
            seen.push(e);
        }
        assert_eq!(seen, vec![Some(5), Some(15), Some(60), Some(1440), Some(2880), None]);
        assert_eq!(early_label(60), "1 hour before");
        assert_eq!(early_label(2880), "2 days before");
        assert_eq!(early_label(15), "15 minutes before");
    }
    #[test]
    fn due_moment_uses_the_offset_in_force_on_the_due_day() {
        let mut r = Reminder::new(1, "x".to_string());
        r.due = Some(20_000);
        r.time = Some(600);
        let midnight = 20_000 * 86_400;
        // summer time (+1h) from the day before the due day on
        let zone = |t: i64| if t >= midnight - 86_400 { 3600 } else { 0 };
        assert_eq!(due_unix(&r, DEFAULT_ALERT_MIN, &zone), Some(midnight + 600 * 60 - 3600));
        assert_eq!(due_unix(&r, DEFAULT_ALERT_MIN, &|_| 0), Some(midnight + 600 * 60));
    }
    #[test]
    fn a_reminder_with_a_zone_is_due_on_that_zones_clock() {
        use springboard::shell::clock::tz::days_from_civil;
        let ny = "EST5EDT,M3.2.0,M11.1.0";
        let summer = days_from_civil(2026, 10, 8);
        let winter = days_from_civil(2026, 1, 15);
        let mut r = Reminder::new(1, "call".to_string());
        r.due = Some(summer);
        r.time = Some(9 * 60);
        // the device is on UTC+1 all year
        let device = |_: i64| 3600;
        assert_eq!(due_unix(&r, DEFAULT_ALERT_MIN, &device), Some(summer * 86_400 + 8 * 3600), "no zone: the device clock");
        r.tz = Some(ny.to_string());
        assert_eq!(due_unix(&r, DEFAULT_ALERT_MIN, &device), Some(summer * 86_400 + 13 * 3600), "9:00 in New York in summer is 13:00 UTC");
        r.due = Some(winter);
        assert_eq!(due_unix(&r, DEFAULT_ALERT_MIN, &device), Some(winter * 86_400 + 14 * 3600), "and 14:00 UTC in winter");
        r.time = None;
        assert_eq!(due_unix(&r, 7 * 60, &device), Some(winter * 86_400 + 12 * 3600), "the all-day time is read there too");
        r.tz = Some("Not/AZone".to_string());
        assert_eq!(due_unix(&r, 7 * 60, &device), Some(winter * 86_400 + 6 * 3600), "an unknown zone falls back to the device clock");
        let mut urgent = r.clone();
        urgent.urgent = true;
        assert_eq!(urgent_text(&urgent, 9 * 60), "Urgent: Due 9:00 AM");
    }
    #[test]
    fn date_only_reminders_alert_at_the_all_day_time_from_the_settings() {
        let mut r = Reminder::new(1, "x".to_string());
        r.due = Some(20_000);
        let midnight = 20_000 * 86_400;
        assert_eq!(due_unix(&r, 7 * 60 + 30, &|_| 0), Some(midnight + 7 * 3600 + 1800));
        assert_eq!(alert_text(&r, 17 * 60, false), "Due 5:00 PM");
        r.time = Some(600);
        assert_eq!(due_unix(&r, 7 * 60, &|_| 0), Some(midnight + 600 * 60), "a reminder with a time ignores the all-day time");
        assert_eq!(alert_text(&r, 17 * 60, false), "Due 10:00 AM");
    }
    #[test]
    fn date_only_reminders_alert_at_nine_and_say_so() {
        // RF18: no time means the alert uses DEFAULT_ALERT_MIN, and the text names it.
        let mut r = Reminder::new(1, "x".to_string());
        r.due = Some(20_000);
        let midnight = 20_000 * 86_400;
        assert_eq!(due_unix(&r, DEFAULT_ALERT_MIN, &|_| 0), Some(midnight + 9 * 3600));
        assert_eq!(alert_text(&r, DEFAULT_ALERT_MIN, false), "Due 9:00 AM");
        r.time = Some(9 * 60 + 30);
        assert_eq!(due_unix(&r, DEFAULT_ALERT_MIN, &|_| 0), Some(midnight + 9 * 3600 + 1800));
    }
    #[test]
    fn next_alert_and_texts() {
        let mut a = r(1, "a");
        a.due = Some(TODAY);
        a.time = Some(600);
        a.early = Some(30);
        assert_eq!(clock_label(545), "9:05 AM");
        assert_eq!(clock_label(0), "12:00 AM");
        assert_eq!(clock_label(13 * 60), "1:00 PM");
        assert_eq!(alert_text(&a, DEFAULT_ALERT_MIN, false), "Due 10:00 AM");
        assert_eq!(alert_text(&a, DEFAULT_ALERT_MIN, true), "Due in 30 minutes");
    }
}
