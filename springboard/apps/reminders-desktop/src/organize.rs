//! Rules that reshape the reminders: marking due dates in bulk, subtask levels, sections, grocery
//! categories, templates and Markdown checklist import. Pure: nothing here touches the UI or the disk.
use crate::model::{
    Data, ListDef, Reminder, Section, Template, TemplateItem, MAX_ITEMS, MAX_LISTS, MAX_NAME, MAX_SECTIONS, MAX_TEMPLATES,
    MAX_TEMPLATE_ITEMS, MAX_TITLE,
};
use springboard::shell::clock::tz::{civil_from_days, days_from_civil};
use std::collections::HashSet;

// ---- due dates ---------------------------------------------------------------------------------------------------

/// The days Reminders > Mark Due Date As offers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MarkDue {
    Today,
    Tomorrow,
    ThisWeekend,
    NextWeek,
}

impl MarkDue {
    pub const ALL: [Self; 4] = [Self::Today, Self::Tomorrow, Self::ThisWeekend, Self::NextWeek];
    pub fn label(self) -> &'static str {
        match self {
            Self::Today => "Today",
            Self::Tomorrow => "Tomorrow",
            Self::ThisWeekend => "This Weekend",
            Self::NextWeek => "Next Week",
        }
    }
}

/// Weekday of a local day number, Monday 0 to Sunday 6 (day 0, 1970-01-01, was a Thursday).
pub fn weekday(day: i64) -> i64 {
    (day + 3).rem_euclid(7)
}

/// The day `mark` means on `today`. This Weekend is the coming Saturday, or today on a Saturday or Sunday; Next Week
/// is the coming Monday.
pub fn mark_day(mark: MarkDue, today: i64) -> i64 {
    let wd = weekday(today);
    match mark {
        MarkDue::Today => today,
        MarkDue::Tomorrow => today + 1,
        MarkDue::ThisWeekend if wd == 6 => today,
        MarkDue::ThisWeekend => today + 5 - wd,
        MarkDue::NextWeek => today + 7 - wd,
    }
}

/// Gives the open reminder `id` the due day `day`, keeping its time. False when it is done, missing or already due then.
pub fn mark_due(items: &mut [Reminder], id: u64, day: i64) -> bool {
    match items.iter_mut().find(|r| r.id == id && !r.completed) {
        Some(r) if r.due != Some(day) => {
            r.due = Some(day);
            true
        }
        _ => false,
    }
}

/// Moves every open reminder due before `today` to `today`; returns how many moved.
pub fn overdue_to_today(items: &mut [Reminder], today: i64) -> usize {
    let mut moved = 0;
    for r in items.iter_mut().filter(|r| !r.completed && r.due.is_some_and(|d| d < today)) {
        r.due = Some(today);
        moved += 1;
    }
    moved
}

pub fn has_overdue(items: &[Reminder], today: i64) -> bool {
    items.iter().any(|r| !r.completed && r.due.is_some_and(|d| d < today))
}

// ---- subtasks ----------------------------------------------------------------------------------------------------

/// Whether `crate::model::indent` would nest `id`: it is a top-level reminder without subtasks and an earlier
/// top-level reminder of its list exists.
pub fn can_indent(items: &[Reminder], id: u64) -> bool {
    let Some(pos) = items.iter().position(|r| r.id == id) else { return false };
    let r = &items[pos];
    r.parent.is_none()
        && !crate::model::has_children(items, id)
        && items[..pos].iter().any(|p| p.list == r.list && p.parent.is_none())
}

/// Makes subtask `id` a top-level reminder again; false when it is not a subtask.
pub fn outdent(items: &mut [Reminder], id: u64) -> bool {
    match items.iter_mut().find(|r| r.id == id) {
        Some(r) if r.parent.is_some() => {
            r.parent = None;
            true
        }
        _ => false,
    }
}

/// The ids of every reminder that has subtasks (what Hide All Subtasks collapses).
pub fn parent_ids(items: &[Reminder]) -> HashSet<u64> {
    items.iter().filter_map(|r| r.parent).collect()
}

// ---- sections ----------------------------------------------------------------------------------------------------

/// A new section at the end of `list`; its id, or `None` for an empty name, a full list or an unknown list.
pub fn add_section(data: &mut Data, list: u64, name: &str) -> Option<u64> {
    let name = clean_name(name)?;
    let id = data.next_section_id()?;
    let l = data.lists.iter_mut().find(|l| l.id == list)?;
    if l.sections.len() >= MAX_SECTIONS {
        return None;
    }
    l.sections.push(Section { id, name });
    Some(id)
}

pub fn rename_section(data: &mut Data, id: u64, name: &str) -> bool {
    let Some(name) = clean_name(name) else { return false };
    match data.lists.iter_mut().flat_map(|l| l.sections.iter_mut()).find(|s| s.id == id) {
        Some(s) => {
            s.name = name;
            true
        }
        None => false,
    }
}

/// Removes section `id`; its reminders stay in the list, out of any section.
pub fn delete_section(data: &mut Data, id: u64) -> bool {
    let Some(l) = data.lists.iter_mut().find(|l| l.sections.iter().any(|s| s.id == id)) else { return false };
    l.sections.retain(|s| s.id != id);
    for r in data.items.iter_mut().filter(|r| r.section == Some(id)) {
        r.section = None;
    }
    true
}

/// Puts top-level reminder `id` in `section` (or none) and, when `before` is a top-level reminder of that section,
/// directly ahead of it; otherwise it goes last. Subtasks follow their parent. False when `id` is missing or a subtask.
pub fn place_in_section(items: &mut Vec<Reminder>, id: u64, section: Option<u64>, before: Option<u64>) -> bool {
    match items.iter().find(|r| r.id == id) {
        Some(r) if r.parent.is_none() => {}
        _ => return false,
    }
    for r in items.iter_mut().filter(|r| r.id == id || r.parent == Some(id)) {
        r.section = section;
    }
    crate::model::move_before(items, id, before)
}

// ---- grocery categories ------------------------------------------------------------------------------------------

/// Grocery sections and the words that put a reminder in them, tried in this order (so "frozen pizza" is Frozen and
/// "orange juice" is Drinks). A single word matches a whole word of the title, with a plural; a phrase matches whole
/// words in a row.
static GROCERY: [(&str, &[&str]); 8] = [
    ("Frozen", &["frozen", "ice cream", "popsicle", "fries"]),
    ("Drinks", &["water", "juice", "soda", "cola", "beer", "wine", "lemonade", "sparkling", "cider"]),
    (
        "Pantry",
        &[
            "rice", "pasta", "flour", "sugar", "salt", "oil", "vinegar", "sauce", "beans", "lentil", "cereal", "oats", "honey", "jam",
            "spice", "soup", "noodle", "canned", "nut", "peanut", "coffee", "tea", "ketchup", "mustard", "chip", "cracker", "chocolate",
        ],
    ),
    (
        "Produce",
        &[
            "apple", "banana", "lettuce", "spinach", "tomato", "potato", "onion", "garlic", "carrot", "cucumber", "pepper", "broccoli",
            "lemon", "lime", "orange", "avocado", "grape", "berry", "strawberry", "blueberry", "raspberry", "mushroom", "celery", "cabbage", "pear", "peach", "basil", "parsley",
            "salad", "fruit", "vegetable", "ginger", "zucchini", "melon", "mango", "kale",
        ],
    ),
    ("Dairy and Eggs", &["milk", "cheese", "yogurt", "yoghurt", "butter", "cream", "egg", "skyr", "cottage"]),
    (
        "Meat and Fish",
        &["chicken", "beef", "pork", "bacon", "sausage", "ham", "steak", "mince", "lamb", "turkey", "fish", "salmon", "cod", "tuna", "shrimp", "haddock"],
    ),
    ("Bakery", &["bread", "bagel", "bun", "croissant", "roll", "baguette", "tortilla", "cake", "muffin", "pastry", "pita"]),
    (
        "Household",
        &[
            "soap", "detergent", "shampoo", "toothpaste", "tissue", "toilet paper", "paper towel", "sponge", "trash bag", "foil", "cleaner", "laundry",
            "battery", "bulb", "candle", "napkin", "dish soap",
        ],
    ),
];

/// The grocery section a reminder title belongs in, if any word of it is known.
pub fn grocery_category(title: &str) -> Option<&'static str> {
    let words: Vec<String> = title.to_lowercase().split(|c: char| !c.is_alphabetic()).filter(|w| !w.is_empty()).map(str::to_string).collect();
    if words.is_empty() {
        return None;
    }
    let phrase = format!(" {} ", words.join(" "));
    let single = |kw: &str| {
        words.iter().any(|w| {
            w == kw || w.strip_suffix('s') == Some(kw) || w.strip_suffix("es") == Some(kw) || (w.ends_with("ies") && w.len() > 3 && format!("{}y", &w[..w.len() - 3]) == kw)
        })
    };
    GROCERY
        .iter()
        .find(|(_, kws)| kws.iter().any(|kw| if kw.contains(' ') { phrase.contains(&format!(" {kw} ")) } else { single(*kw) }))
        .map(|(name, _)| *name)
}

/// Auto-categorize: puts each top-level reminder of `list` that is in no section into a grocery section, adding the
/// sections it needs (a section of the same name, in any case, is reused). Returns how many reminders were placed.
pub fn auto_categorize(data: &mut Data, list: u64) -> usize {
    let wanted: Vec<(usize, &'static str)> = data
        .items
        .iter()
        .enumerate()
        .filter(|(_, r)| r.list == list && r.parent.is_none() && !section_known(data, list, r))
        .filter_map(|(i, r)| grocery_category(&r.title).map(|c| (i, c)))
        .collect();
    let mut placed = 0;
    for (name, _) in GROCERY.iter() {
        let members: Vec<usize> = wanted.iter().filter(|(_, c)| c == name).map(|(i, _)| *i).collect();
        if members.is_empty() {
            continue;
        }
        let existing = data
            .lists
            .iter()
            .find(|l| l.id == list)
            .and_then(|l| l.sections.iter().find(|s| s.name.eq_ignore_ascii_case(name)))
            .map(|s| s.id);
        let Some(section) = existing.or_else(|| add_section(data, list, name)) else { continue };
        for i in members {
            let id = data.items[i].id;
            for r in data.items.iter_mut().filter(|r| r.id == id || r.parent == Some(id)) {
                r.section = Some(section);
            }
            placed += 1;
        }
    }
    placed
}

fn section_known(data: &Data, list: u64, r: &Reminder) -> bool {
    r.section.is_some_and(|s| data.lists.iter().any(|l| l.id == list && l.sections.iter().any(|x| x.id == s)))
}

// ---- templates ---------------------------------------------------------------------------------------------------

/// Saves `list` as a template named `name`: its reminders without dates, repeats, alerts or completion. A template of
/// the same name (in any case) is replaced. Returns the template id; `None` when the list is unknown, the name is
/// empty, or the limits are reached.
pub fn save_template(data: &mut Data, list: u64, name: &str) -> Option<u64> {
    let name = clean_name(name)?;
    let def = data.lists.iter().find(|l| l.id == list)?;
    let (color, icon) = (def.color, def.icon);
    let mut items = vec![];
    for r in data.items.iter().filter(|r| r.list == list && r.parent.is_none()) {
        let mut push = |r: &Reminder, sub: bool| {
            items.push(TemplateItem { title: r.title.clone(), notes: r.notes.clone(), url: r.url.clone(), flagged: r.flagged, priority: r.priority, sub });
        };
        push(r, false);
        for kid in data.items.iter().filter(|k| k.parent == Some(r.id)) {
            push(kid, true);
        }
    }
    items.truncate(MAX_TEMPLATE_ITEMS);
    if let Some(t) = data.templates.iter_mut().find(|t| t.name.eq_ignore_ascii_case(&name)) {
        (t.name, t.color, t.icon, t.items) = (name, color, icon, items);
        return Some(t.id);
    }
    if data.templates.len() >= MAX_TEMPLATES {
        return None;
    }
    let id = data.next_template_id()?;
    data.templates.push(Template { id, name, color, icon, items });
    Some(id)
}

/// A new list made from template `tid`, its reminders open and undated. Returns the list id; `None` when the template
/// is unknown or the list or reminder limits are reached.
pub fn list_from_template(data: &mut Data, tid: u64, created: Option<i64>) -> Option<u64> {
    let t = data.templates.iter().find(|t| t.id == tid)?.clone();
    if data.lists.len() >= MAX_LISTS || data.items.len() + t.items.len() > MAX_ITEMS {
        return None;
    }
    let list = data.next_list_id()?;
    data.lists.push(ListDef::new(list, t.name.clone(), t.color, t.icon));
    let mut next = data.next_item_id()?;
    let mut parent: Option<u64> = None;
    for item in &t.items {
        let mut r = Reminder::new(next, item.title.clone());
        r.list = list;
        r.notes = item.notes.clone();
        r.url = item.url.clone();
        r.flagged = item.flagged;
        r.priority = item.priority;
        r.created = created;
        if item.sub {
            r.parent = parent;
        } else {
            parent = Some(next);
        }
        data.items.push(r);
        next += 1;
    }
    Some(list)
}

pub fn delete_template(data: &mut Data, tid: u64) -> bool {
    let before = data.templates.len();
    data.templates.retain(|t| t.id != tid);
    data.templates.len() != before
}

// ---- Markdown checklist import -----------------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChecklistItem {
    pub title: String,
    pub done: bool,
    pub sub: bool,
    pub due: Option<i64>,
    pub time: Option<u16>,
    /// Index into [`Checklist::sections`]: the `## heading` the line sits under.
    pub section: Option<usize>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Checklist {
    /// The first `# heading`.
    pub name: Option<String>,
    /// The `## headings`, in order, each once.
    pub sections: Vec<String>,
    pub items: Vec<ChecklistItem>,
}

/// Reads a Markdown checklist, the form `model::export_markdown` writes: `- [ ] title` and `- [x] title` lines (a
/// plain `-`, `*` or `+` bullet is an open reminder), two or more spaces of indent for a subtask, and a trailing
/// `(YYYY-MM-DD)` or `(YYYY-MM-DD HH:MM)` for the due day. A `## heading` starts a section and the lines below it
/// belong to it. Other lines are ignored.
pub fn parse_checklist(text: &str) -> Checklist {
    let mut out = Checklist::default();
    let mut section: Option<usize> = None;
    for line in text.trim_start_matches('\u{feff}').lines() {
        let line = line.trim_end();
        if out.name.is_none() {
            if let Some(h) = line.strip_prefix("# ") {
                out.name = clean_name(h);
                continue;
            }
        }
        if let Some(h) = line.strip_prefix("## ") {
            if let Some(name) = clean_name(h) {
                section = match out.sections.iter().position(|s| s.eq_ignore_ascii_case(&name)) {
                    Some(i) => Some(i),
                    None if out.sections.len() < MAX_SECTIONS => {
                        out.sections.push(name);
                        Some(out.sections.len() - 1)
                    }
                    None => None,
                };
            }
            continue;
        }
        let indent: usize = line.chars().take_while(|c| *c == ' ' || *c == '\t').map(|c| if c == '\t' { 2 } else { 1 }).sum();
        let body = line.trim_start();
        let Some(rest) = ["- ", "* ", "+ "].iter().find_map(|b| body.strip_prefix(*b)) else { continue };
        if matches!(rest, "[ ]" | "[x]" | "[X]") {
            continue;
        }
        let (done, rest) = match rest {
            r if r.starts_with("[ ] ") => (false, &r[4..]),
            r if r.starts_with("[x] ") || r.starts_with("[X] ") => (true, &r[4..]),
            r => (false, r),
        };
        let (title, due, time) = split_due(rest);
        let title: String = title.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(MAX_TITLE).collect();
        if title.is_empty() {
            continue;
        }
        // a subtask nests under the last top-level reminder, which must sit in the same section
        let sub = indent >= 2 && out.items.iter().rev().find(|i| !i.sub).is_some_and(|p| p.section == section);
        out.items.push(ChecklistItem { title, done, sub, due, time, section });
    }
    out
}

/// Splits a trailing `(YYYY-MM-DD)` or `(YYYY-MM-DD HH:MM)` off `text`.
fn split_due(text: &str) -> (&str, Option<i64>, Option<u16>) {
    let none = (text, None, None);
    let Some(inner) = text.trim_end().strip_suffix(')') else { return none };
    let Some(open) = inner.rfind('(') else { return none };
    let (date, time) = match inner[open + 1..].split_once(' ') {
        Some((d, t)) => (d, Some(t)),
        None => (&inner[open + 1..], None),
    };
    let mut parts = date.split('-');
    let (Some(y), Some(m), Some(d), None) = (parts.next(), parts.next(), parts.next(), parts.next()) else { return none };
    let (Ok(y), Ok(m), Ok(d)) = (y.parse::<i64>(), m.parse::<u32>(), d.parse::<u32>()) else { return none };
    if !(1900..=2200).contains(&y) || y.to_string().len() != 4 || date.len() != 10 {
        return none;
    }
    let day = days_from_civil(y, m, d);
    if civil_from_days(day) != (y, m, d) {
        return none;
    }
    let minute = match time {
        Some(t) => {
            let (h, mi) = match t.split_once(':') {
                Some((h, mi)) if h.len() == 2 && mi.len() == 2 => (h.parse::<u16>(), mi.parse::<u16>()),
                _ => return none,
            };
            match (h, mi) {
                (Ok(h), Ok(mi)) if h < 24 && mi < 60 => Some(h * 60 + mi),
                _ => return none,
            }
        }
        None => None,
    };
    (inner[..open].trim_end(), Some(day), minute)
}

/// Adds a list made from `checklist` named by its heading (or `fallback`). Returns the new list id; `None` when it has
/// no reminders or the list or reminder limits are reached.
pub fn import_checklist(data: &mut Data, checklist: &Checklist, fallback: &str, created: Option<i64>) -> Option<u64> {
    if checklist.items.is_empty() || data.lists.len() >= MAX_LISTS {
        return None;
    }
    let name = checklist.name.clone().or_else(|| clean_name(fallback)).unwrap_or_else(|| "Imported".to_string());
    let room = MAX_ITEMS.saturating_sub(data.items.len());
    let list = data.next_list_id()?;
    let color = (data.lists.len() % 8) as u8;
    data.lists.push(ListDef::new(list, name, color, 0));
    let sections: Vec<Option<u64>> = checklist.sections.iter().map(|s| add_section(data, list, s)).collect();
    let mut next = data.next_item_id()?;
    let mut parent: Option<u64> = None;
    for item in checklist.items.iter().take(room) {
        let mut r = Reminder::new(next, item.title.clone());
        r.list = list;
        r.section = item.section.and_then(|i| sections.get(i).copied().flatten());
        r.completed = item.done;
        r.due = item.due;
        r.time = item.time.filter(|_| item.due.is_some());
        r.created = created;
        if item.sub && parent.is_some() {
            r.parent = parent;
        } else {
            parent = Some(next);
        }
        data.items.push(r);
        next += 1;
    }
    Some(list)
}

/// A list, group, section or template name: whitespace collapsed, at most `MAX_NAME` characters; `None` when empty.
pub fn clean_name(text: &str) -> Option<String> {
    let name: String = text.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(MAX_NAME).collect();
    let name = name.trim().to_string();
    (!name.is_empty()).then_some(name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{export_markdown, visible, Group, Options, Row, Sort, View};

    /// 2024-10-04, a Friday.
    const FRIDAY: i64 = 20_000;

    fn r(id: u64, title: &str) -> Reminder {
        Reminder::new(id, title)
    }

    #[test]
    fn weekdays_count_from_monday() {
        assert_eq!(weekday(0), 3, "1970-01-01 was a Thursday");
        assert_eq!(weekday(FRIDAY), 4);
        assert_eq!(weekday(FRIDAY + 3), 0);
    }

    #[test]
    fn mark_due_days_follow_the_calendar() {
        assert_eq!(mark_day(MarkDue::Today, FRIDAY), FRIDAY);
        assert_eq!(mark_day(MarkDue::Tomorrow, FRIDAY), FRIDAY + 1);
        assert_eq!(mark_day(MarkDue::ThisWeekend, FRIDAY), FRIDAY + 1, "Friday to Saturday");
        assert_eq!(mark_day(MarkDue::ThisWeekend, FRIDAY + 1), FRIDAY + 1, "on Saturday it is today");
        assert_eq!(mark_day(MarkDue::ThisWeekend, FRIDAY + 2), FRIDAY + 2, "on Sunday it is today");
        assert_eq!(mark_day(MarkDue::ThisWeekend, FRIDAY + 3), FRIDAY + 8, "on Monday the Saturday after");
        assert_eq!(mark_day(MarkDue::NextWeek, FRIDAY), FRIDAY + 3, "Friday to Monday");
        assert_eq!(mark_day(MarkDue::NextWeek, FRIDAY + 3), FRIDAY + 10, "on Monday the Monday after");
        assert_eq!(MarkDue::ALL.map(MarkDue::label), ["Today", "Tomorrow", "This Weekend", "Next Week"]);
    }

    #[test]
    fn marking_a_due_date_keeps_the_time_and_skips_done_reminders() {
        let mut timed = r(1, "timed");
        timed.due = Some(FRIDAY - 3);
        timed.time = Some(615);
        let mut done = r(2, "done");
        done.completed = true;
        let mut items = vec![timed, done, r(3, "plain")];
        assert!(mark_due(&mut items, 1, FRIDAY));
        assert_eq!((items[0].due, items[0].time), (Some(FRIDAY), Some(615)));
        assert!(!mark_due(&mut items, 1, FRIDAY), "already on that day");
        assert!(!mark_due(&mut items, 2, FRIDAY), "a completed reminder keeps its day");
        assert!(!mark_due(&mut items, 9, FRIDAY), "unknown id");
        assert!(mark_due(&mut items, 3, FRIDAY + 1));
        assert_eq!(items[2].due, Some(FRIDAY + 1));
    }

    #[test]
    fn overdue_reminders_move_to_today_and_nothing_else_does() {
        let mut late = r(1, "late");
        late.due = Some(FRIDAY - 5);
        late.time = Some(60);
        let mut today = r(2, "today");
        today.due = Some(FRIDAY);
        let mut later = r(3, "later");
        later.due = Some(FRIDAY + 2);
        let mut old_done = r(4, "old done");
        old_done.due = Some(FRIDAY - 9);
        old_done.completed = true;
        let mut items = vec![late, today, later, old_done, r(5, "undated")];
        assert!(has_overdue(&items, FRIDAY));
        assert_eq!(overdue_to_today(&mut items, FRIDAY), 1);
        assert_eq!(items[0].due, Some(FRIDAY));
        assert_eq!(items[0].time, Some(60), "the time of day stays");
        assert_eq!(items[3].due, Some(FRIDAY - 9));
        assert!(!has_overdue(&items, FRIDAY));
        assert_eq!(overdue_to_today(&mut items, FRIDAY), 0);
    }

    #[test]
    fn indent_and_outdent_are_the_menu_rules() {
        let mut items = vec![r(1, "a"), r(2, "b"), r(3, "c")];
        assert!(!can_indent(&items, 1), "the first reminder has nothing above it");
        assert!(can_indent(&items, 2));
        assert!(crate::model::indent(&mut items, 2));
        assert!(!can_indent(&items, 2), "already a subtask");
        assert!(!can_indent(&items, 1), "a reminder with subtasks does not nest");
        assert!(can_indent(&items, 3));
        assert!(!outdent(&mut items, 1), "a top-level reminder cannot outdent");
        assert!(outdent(&mut items, 2));
        assert_eq!(items[1].parent, None);
        assert!(!outdent(&mut items, 9));
        let mut other = vec![r(1, "a"), r(2, "b")];
        other[1].list = 2;
        assert!(!can_indent(&other, 2), "the earlier reminder is in another list");
    }

    #[test]
    fn parents_are_the_reminders_with_subtasks() {
        let mut kid = r(2, "kid");
        kid.parent = Some(1);
        let mut kid2 = r(4, "kid2");
        kid2.parent = Some(3);
        let items = vec![r(1, "p"), kid, r(3, "q"), kid2, r(5, "alone")];
        assert_eq!(parent_ids(&items), HashSet::from([1, 3]));
    }

    fn data_with(items: Vec<Reminder>) -> Data {
        Data { items, ..Data::default() }
    }

    #[test]
    fn sections_are_added_renamed_and_deleted_with_their_members_kept() {
        let mut d = data_with(vec![r(1, "milk"), r(2, "bread")]);
        let a = add_section(&mut d, 1, "  Cold   food ").unwrap();
        let b = add_section(&mut d, 1, "Dry").unwrap();
        assert_ne!(a, b);
        assert_eq!(d.lists[0].sections[0].name, "Cold food");
        assert_eq!(add_section(&mut d, 1, "   "), None, "empty name");
        assert_eq!(add_section(&mut d, 7, "x"), None, "unknown list");
        assert!(rename_section(&mut d, b, "Pantry"));
        assert!(!rename_section(&mut d, 99, "x"));
        d.items[0].section = Some(a);
        assert!(delete_section(&mut d, a));
        assert_eq!(d.items[0].section, None, "the reminder stays, out of any section");
        assert_eq!(d.lists[0].sections.len(), 1);
        assert!(!delete_section(&mut d, a));
    }

    #[test]
    fn a_list_holds_at_most_twenty_sections() {
        let mut d = Data::default();
        for i in 0..MAX_SECTIONS {
            assert!(add_section(&mut d, 1, &format!("S{i}")).is_some());
        }
        assert_eq!(add_section(&mut d, 1, "one more"), None);
    }

    #[test]
    fn the_list_view_shows_unplaced_reminders_then_each_section_under_its_header() {
        let mut d = data_with(vec![r(1, "loose"), r(2, "in b"), r(3, "in a"), r(4, "in a too")]);
        let a = add_section(&mut d, 1, "A").unwrap();
        let b = add_section(&mut d, 1, "B").unwrap();
        let empty = add_section(&mut d, 1, "Empty").unwrap();
        d.items[1].section = Some(b);
        d.items[2].section = Some(a);
        d.items[3].section = Some(a);
        let rows = visible(&d, View::List(1), Options::default(), 20_000, "", &HashSet::new());
        let shape: Vec<String> = rows
            .iter()
            .map(|row| match row {
                Row::Item { idx, .. } => d.items[*idx].title.clone(),
                Row::Section { name, .. } => format!("[{name}]"),
                Row::Header(h) => format!("<{h}>"),
            })
            .collect();
        assert_eq!(shape, ["loose", "[A]", "in a", "in a too", "[B]", "in b", "[Empty]"]);
        assert!(rows.contains(&Row::Section { id: empty, name: "Empty".into() }));
        // searching, grouping and the smart lists show no section headers
        let flat = visible(&d, View::List(1), Options::default(), 20_000, "in", &HashSet::new());
        assert!(flat.iter().all(|row| matches!(row, Row::Item { .. })));
        let grouped = visible(&d, View::List(1), Options { group: Group::Priority, ..Options::default() }, 20_000, "", &HashSet::new());
        assert!(grouped.iter().all(|row| !matches!(row, Row::Section { .. })));
        let smart = visible(&d, View::Smart(crate::model::Smart::All), Options::default(), 20_000, "", &HashSet::new());
        assert!(smart.iter().all(|row| matches!(row, Row::Item { .. })));
    }

    #[test]
    fn a_section_with_a_deleted_id_falls_back_to_unplaced() {
        let mut d = data_with(vec![r(1, "orphan")]);
        d.items[0].section = Some(42);
        add_section(&mut d, 1, "A").unwrap();
        let rows = visible(&d, View::List(1), Options::default(), 20_000, "", &HashSet::new());
        assert!(matches!(rows[0], Row::Item { idx: 0, depth: 0 }), "listed before the first header");
    }

    #[test]
    fn subtasks_stay_under_their_parent_inside_a_section() {
        let mut kid = r(2, "kid");
        kid.parent = Some(1);
        let mut d = data_with(vec![r(1, "parent"), kid]);
        let a = add_section(&mut d, 1, "A").unwrap();
        assert!(place_in_section(&mut d.items, 1, Some(a), None));
        assert_eq!(d.items[1].section, Some(a), "the subtask takes its parent's section");
        let rows = visible(&d, View::List(1), Options::default(), 20_000, "", &HashSet::new());
        assert_eq!(rows.len(), 3);
        assert!(matches!(rows[0], Row::Section { .. }));
        assert!(matches!(rows[2], Row::Item { depth: 1, .. }));
        assert!(!place_in_section(&mut d.items, 2, None, None), "a subtask is not placed alone");
        assert!(!place_in_section(&mut d.items, 9, None, None));
    }

    #[test]
    fn placing_in_a_section_before_a_reminder_orders_that_section() {
        let mut d = data_with(vec![r(1, "one"), r(2, "two"), r(3, "three")]);
        let a = add_section(&mut d, 1, "A").unwrap();
        for id in [1, 2] {
            assert!(place_in_section(&mut d.items, id, Some(a), None));
        }
        assert!(place_in_section(&mut d.items, 3, Some(a), Some(1)), "three goes ahead of one");
        let rows = visible(&d, View::List(1), Options::default(), 20_000, "", &HashSet::new());
        let titles: Vec<&str> = rows
            .iter()
            .filter_map(|row| match row {
                Row::Item { idx, .. } => Some(d.items[*idx].title.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(titles, ["three", "one", "two"]);
        assert!(place_in_section(&mut d.items, 3, None, None), "out of the section again");
        assert_eq!(d.items.iter().find(|x| x.id == 3).unwrap().section, None);
    }

    #[test]
    fn sorting_applies_inside_each_section() {
        let mut d = data_with(vec![r(1, "pear"), r(2, "apple"), r(3, "zucchini"), r(4, "beet")]);
        let a = add_section(&mut d, 1, "A").unwrap();
        d.items[0].section = Some(a);
        d.items[1].section = Some(a);
        let opts = Options { sort: Sort::Title, ..Options::default() };
        let rows = visible(&d, View::List(1), opts, 20_000, "", &HashSet::new());
        let titles: Vec<&str> = rows
            .iter()
            .filter_map(|row| match row {
                Row::Item { idx, .. } => Some(d.items[*idx].title.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(titles, ["beet", "zucchini", "apple", "pear"]);
    }

    #[test]
    fn grocery_words_pick_the_section() {
        assert_eq!(grocery_category("Milk"), Some("Dairy and Eggs"));
        assert_eq!(grocery_category("2 dozen eggs"), Some("Dairy and Eggs"));
        assert_eq!(grocery_category("Tomatoes"), Some("Produce"));
        assert_eq!(grocery_category("Strawberries and berries"), Some("Produce"), "ies plural");
        assert_eq!(grocery_category("Orange juice"), Some("Drinks"), "juice wins over orange");
        assert_eq!(grocery_category("Frozen pizza"), Some("Frozen"));
        assert_eq!(grocery_category("Ice cream"), Some("Frozen"), "a phrase");
        assert_eq!(grocery_category("tomato sauce"), Some("Pantry"));
        assert_eq!(grocery_category("Toilet paper"), Some("Household"));
        assert_eq!(grocery_category("Eggplant"), None, "whole words only");
        assert_eq!(grocery_category("Call the dentist"), None);
        assert_eq!(grocery_category(""), None);
    }

    #[test]
    fn auto_categorize_creates_the_sections_it_needs_and_leaves_the_rest() {
        let mut kid = r(5, "kid of milk");
        kid.parent = Some(1);
        let mut other = r(6, "bananas");
        other.list = 2;
        let mut placed = r(7, "cheese");
        placed.section = Some(1);
        let mut d = data_with(vec![r(1, "milk"), r(2, "bananas"), r(3, "dentist"), r(4, "bread"), kid, other, placed]);
        d.lists.push(ListDef::new(2, "Other", 1, 0));
        d.lists[0].sections.push(Section { id: 1, name: "dairy and eggs".into() });
        d.lists[0].sections.push(Section { id: 2, name: "Hand made".into() });
        d.items[6].section = Some(2);
        let n = auto_categorize(&mut d, 1);
        assert_eq!(n, 3, "milk, bananas and bread");
        let names: Vec<&str> = d.lists[0].sections.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["dairy and eggs", "Hand made", "Produce", "Bakery"], "the existing Dairy section is reused");
        assert_eq!(d.items[0].section, Some(1));
        assert_eq!(d.items[4].section, Some(1), "the subtask follows");
        assert_eq!(d.items[2].section, None, "unknown words stay out");
        assert_eq!(d.items[5].section, None, "another list is untouched");
        assert_eq!(d.items[6].section, Some(2), "a reminder already in a section stays there");
        assert_eq!(auto_categorize(&mut d, 1), 0, "a second run changes nothing");
    }

    #[test]
    fn a_template_drops_dates_and_completion_and_makes_open_lists() {
        let mut d = data_with(vec![]);
        let mut a = r(1, "Pack bags");
        a.due = Some(FRIDAY);
        a.time = Some(600);
        a.completed = true;
        a.flagged = true;
        a.priority = 2;
        a.repeat = Some(crate::repeat::Repeat::new(crate::repeat::Unit::Week, 1));
        a.early = Some(30);
        a.notes = "check list".into();
        let mut kid = r(2, "Passports");
        kid.parent = Some(1);
        d.items = vec![a, kid, r(3, "Lock doors")];
        d.lists[0].color = 3;
        let t = save_template(&mut d, 1, " Trip ").unwrap();
        assert_eq!(d.templates[0].name, "Trip");
        assert_eq!(d.templates[0].items.len(), 3);
        assert!(d.templates[0].items[1].sub && !d.templates[0].items[2].sub);
        assert_eq!(save_template(&mut d, 1, "trip"), Some(t), "the same name replaces the template");
        assert_eq!(d.templates.len(), 1);
        assert_eq!(save_template(&mut d, 1, ""), None);
        assert_eq!(save_template(&mut d, 9, "x"), None);

        let list = list_from_template(&mut d, t, Some(1234)).unwrap();
        assert_eq!(d.lists.last().unwrap().name, "trip", "the replacement template's casing is preserved");
        assert_eq!(d.lists.last().unwrap().color, 3);
        let made: Vec<&Reminder> = d.items.iter().filter(|r| r.list == list).collect();
        assert_eq!(made.len(), 3);
        assert!(made.iter().all(|r| !r.completed && r.due.is_none() && r.time.is_none() && r.repeat.is_none() && r.early.is_none()));
        assert!(made[0].flagged && made[0].priority == 2 && made[0].notes == "check list");
        assert_eq!(made[1].parent, Some(made[0].id), "the subtask is re-linked");
        assert_eq!(made[2].parent, None);
        assert_eq!(made[0].created, Some(1234));
        let ids: HashSet<u64> = d.items.iter().map(|r| r.id).collect();
        assert_eq!(ids.len(), d.items.len(), "ids stay unique");
        assert_eq!(list_from_template(&mut d, 99, None), None);
        assert!(delete_template(&mut d, t));
        assert!(!delete_template(&mut d, t));
    }

    #[test]
    fn the_template_count_is_capped() {
        let mut d = Data::default();
        for i in 0..MAX_TEMPLATES {
            assert!(save_template(&mut d, 1, &format!("T{i}")).is_some());
        }
        assert_eq!(save_template(&mut d, 1, "extra"), None);
        assert!(save_template(&mut d, 1, "t3").is_some(), "replacing one is still allowed");
    }

    #[test]
    fn a_checklist_reads_what_the_export_writes() {
        let mut d = Data::default();
        d.lists[0].name = "Weekend".into();
        let mut a = r(1, "Buy   milk");
        a.due = Some(FRIDAY);
        a.time = Some(9 * 60 + 5);
        let mut kid = r(2, "Oat milk");
        kid.parent = Some(1);
        kid.completed = true;
        let mut c = r(3, "Call mum");
        c.due = Some(FRIDAY + 1);
        d.items = vec![a, kid, c, r(4, "Plain")];
        let text = export_markdown(&d, 1);
        let parsed = parse_checklist(&text);
        assert_eq!(parsed.name.as_deref(), Some("Weekend"));
        assert_eq!(
            parsed.items,
            vec![
                ChecklistItem { title: "Buy milk".into(), done: false, sub: false, due: Some(FRIDAY), time: Some(545), section: None },
                ChecklistItem { title: "Oat milk".into(), done: true, sub: true, due: None, time: None, section: None },
                ChecklistItem { title: "Call mum".into(), done: false, sub: false, due: Some(FRIDAY + 1), time: None, section: None },
                ChecklistItem { title: "Plain".into(), done: false, sub: false, due: None, time: None, section: None },
            ]
        );
    }

    #[test]
    fn checklist_parsing_is_forgiving_about_bullets_and_strict_about_dates() {
        let text = "\u{feff}# Trip\nsome prose\n* [X] Done one\n+ bare bullet\n- [ ]   \n\t- [ ] tab subtask\n- [ ] Dated (2024-02-30)\n- [ ] Later (2024-10-04 25:00)\n- [ ] Ok (2024-10-04 07:30)\n  - [ ] orphan indent\n";
        let c = parse_checklist(text);
        let titles: Vec<&str> = c.items.iter().map(|i| i.title.as_str()).collect();
        assert_eq!(c.name.as_deref(), Some("Trip"));
        assert_eq!(
            titles,
            ["Done one", "bare bullet", "tab subtask", "Dated (2024-02-30)", "Later (2024-10-04 25:00)", "Ok", "orphan indent"],
            "an impossible date or time stays in the title"
        );
        assert!(c.items[0].done && !c.items[1].done);
        assert!(c.items[2].sub, "a tab is a subtask level");
        assert_eq!((c.items[5].due, c.items[5].time), (Some(FRIDAY), Some(450)));
        assert!(c.items[6].sub);
        assert!(parse_checklist("- [ ] first\n  - [ ] sub").items[1].sub);
        assert!(!parse_checklist("  - [ ] indented first").items[0].sub, "nothing to nest under");
        assert_eq!(parse_checklist("no list here"), Checklist::default());
    }

    #[test]
    fn sections_survive_an_export_and_an_import() {
        let mut d = data_with(vec![r(1, "loose"), r(2, "milk"), r(3, "bread"), r(4, "whole")]);
        let a = add_section(&mut d, 1, "Cold food").unwrap();
        let b = add_section(&mut d, 1, "Bakery").unwrap();
        add_section(&mut d, 1, "Empty").unwrap();
        d.items[1].section = Some(a);
        d.items[2].section = Some(b);
        d.items[3].section = Some(b);
        d.items[3].parent = Some(3);
        d.items[3].title = "whole grain".into();
        let text = export_markdown(&d, 1);
        assert_eq!(
            text,
            "# Reminders\n\n- [ ] loose\n\n## Cold food\n\n- [ ] milk\n\n## Bakery\n\n- [ ] bread\n  - [ ] whole grain\n\n## Empty\n\n"
        );
        let parsed = parse_checklist(&text);
        assert_eq!(parsed.sections, ["Cold food", "Bakery", "Empty"]);
        let sections: Vec<Option<usize>> = parsed.items.iter().map(|i| i.section).collect();
        assert_eq!(sections, [None, Some(0), Some(1), Some(1)]);
        assert!(parsed.items[3].sub);

        let mut imported = Data::default();
        let list = import_checklist(&mut imported, &parsed, "x", None).unwrap();
        let l = imported.lists.iter().find(|l| l.id == list).unwrap();
        assert_eq!(l.sections.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), ["Cold food", "Bakery", "Empty"]);
        let section_of = |title: &str| imported.items.iter().find(|r| r.title == title).unwrap().section;
        assert_eq!(section_of("loose"), None);
        assert_eq!(section_of("milk"), Some(l.sections[0].id));
        assert_eq!(section_of("bread"), Some(l.sections[1].id));
        assert_eq!(section_of("whole grain"), Some(l.sections[1].id), "a subtask follows its parent");
    }

    #[test]
    fn a_subtask_under_a_new_heading_does_not_nest_under_the_last_section() {
        let c = parse_checklist("- [ ] one\n## Next\n  - [ ] indented first in the section\n- [ ] two\n## Next\n- [ ] three");
        assert_eq!(c.sections, ["Next"], "a repeated heading is one section");
        let shape: Vec<(bool, Option<usize>)> = c.items.iter().map(|i| (i.sub, i.section)).collect();
        assert_eq!(shape, [(false, None), (false, Some(0)), (false, Some(0)), (false, Some(0))]);
    }

    #[test]
    fn importing_a_checklist_makes_a_new_list() {
        let mut d = data_with(vec![r(7, "existing")]);
        let c = parse_checklist("# Packing\n- [x] Socks\n  - [ ] Wool (2024-10-04 08:00)\n- [ ] Shoes\n");
        let list = import_checklist(&mut d, &c, "ignored", Some(99)).unwrap();
        assert_eq!(d.lists.len(), 2);
        let l = d.lists.iter().find(|l| l.id == list).unwrap();
        assert_eq!(l.name, "Packing");
        let made: Vec<&Reminder> = d.items.iter().filter(|r| r.list == list).collect();
        assert_eq!(made.len(), 3);
        assert!(made[0].completed && made[0].created == Some(99));
        assert_eq!(made[1].parent, Some(made[0].id));
        assert_eq!((made[1].due, made[1].time), (Some(FRIDAY), Some(480)));
        assert!(made[2].parent.is_none() && made[2].id > 7);
        // without a heading the file name stands in, and an empty file imports nothing
        let named = import_checklist(&mut d, &parse_checklist("- groceries"), "  shopping   list ", None).unwrap();
        assert_eq!(d.lists.iter().find(|l| l.id == named).unwrap().name, "shopping list");
        assert_eq!(import_checklist(&mut d, &Checklist::default(), "x", None), None);
        assert_eq!(d.lists.len(), 3);
    }

    #[test]
    fn names_are_cleaned_and_capped() {
        assert_eq!(clean_name("  a \t b  "), Some("a b".into()));
        assert_eq!(clean_name(" \n"), None);
        assert_eq!(clean_name(&"x".repeat(100)).unwrap().chars().count(), MAX_NAME);
    }
}
