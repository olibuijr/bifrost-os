//! Reminder list templates and grocery sorting: a template is a list saved without dates, times, completion or
//! flags, so a new list can be made from it; the grocery table assigns shopping items to sections by keyword; the
//! time zone override turns a due day and clock time in a chosen zone into a Unix time. Pure std over `Reminder`.
#![allow(dead_code)] // Save as Template, the Templates sheet, grocery sections and the zone picker wire these in

use crate::model::Reminder;

/// A saved template: a name and its items (title, notes, priority, parent index into the same list).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Template {
    pub name: String,
    pub items: Vec<TemplateItem>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TemplateItem {
    pub title: String,
    pub notes: String,
    pub priority: u8,
    /// Index of the parent item in `Template::items`, if this one is nested.
    pub parent: Option<usize>,
}

/// Saves the reminders of `list` (in order) as a template; dates, times, repeats, completion and flags are dropped.
pub fn save_template(name: &str, items: &[Reminder], list: u64) -> Template {
    let mine: Vec<&Reminder> = items.iter().filter(|r| r.list == list).collect();
    let items = mine
        .iter()
        .map(|r| TemplateItem {
            title: r.title.clone(),
            notes: r.notes.clone(),
            priority: r.priority,
            parent: r.parent.and_then(|p| mine.iter().position(|m| m.id == p)),
        })
        .collect();
    Template { name: name.trim().to_string(), items }
}

/// Makes reminders in `list` from a template, with ids from `first_id` upward. Parents keep their nesting.
pub fn instantiate(t: &Template, list: u64, first_id: u64) -> Vec<Reminder> {
    t.items
        .iter()
        .enumerate()
        .map(|(i, it)| {
            let mut r = Reminder::new(first_id + i as u64, it.title.clone());
            r.notes = it.notes.clone();
            r.priority = it.priority;
            r.list = list;
            r.parent = it.parent.filter(|p| *p < i).map(|p| first_id + p as u64);
            r
        })
        .collect()
}

/// Stored form: a header line `#name`, then `priority<TAB>parent<TAB>title<TAB>notes` per item (parent `-` for none).
/// Tabs and line breaks in text are stored as spaces.
pub fn to_text(t: &Template) -> String {
    let flat = |s: &str| s.replace(['\t', '\n', '\r'], " ");
    let mut out = format!("#{}\n", flat(&t.name));
    for it in &t.items {
        let parent = it.parent.map_or("-".to_string(), |p| p.to_string());
        out.push_str(&format!("{}\t{parent}\t{}\t{}\n", it.priority, flat(&it.title), flat(&it.notes)));
    }
    out
}

pub fn from_text(text: &str) -> Option<Template> {
    let mut lines = text.lines();
    let name = lines.next()?.strip_prefix('#')?.to_string();
    let mut items = vec![];
    for line in lines {
        let p: Vec<&str> = line.splitn(4, '\t').collect();
        if p.len() < 3 {
            continue;
        }
        items.push(TemplateItem {
            priority: p[0].parse::<u8>().ok()?.min(3),
            parent: if p[1] == "-" { None } else { Some(p[1].parse().ok()?) },
            title: p[2].to_string(),
            notes: p.get(3).unwrap_or(&"").to_string(),
        });
    }
    Some(Template { name, items })
}

/// The grocery sections, in shopping-route order.
pub const SECTIONS: [&str; 9] =
    ["Produce", "Bakery", "Dairy & Eggs", "Meat & Fish", "Frozen", "Pantry", "Drinks", "Household", "Other"];

const KEYWORDS: [(&str, &[&str]); 8] = [
    ("Produce", &["apple", "banana", "lettuce", "tomato", "onion", "potato", "carrot", "cucumber", "lemon", "berries", "spinach", "garlic", "pepper", "avocado", "fruit", "veg"]),
    ("Bakery", &["bread", "bagel", "bun", "croissant", "baguette", "cake", "roll"]),
    ("Dairy & Eggs", &["milk", "cheese", "butter", "yogurt", "yoghurt", "cream", "egg", "skyr"]),
    ("Meat & Fish", &["chicken", "beef", "pork", "ham", "sausage", "bacon", "salmon", "cod", "fish", "shrimp", "lamb"]),
    ("Frozen", &["frozen", "ice cream", "pizza"]),
    ("Pantry", &["rice", "pasta", "flour", "sugar", "salt", "oil", "beans", "cereal", "oats", "sauce", "spice", "coffee", "tea", "honey"]),
    ("Drinks", &["juice", "water", "soda", "beer", "wine", "cola"]),
    ("Household", &["soap", "detergent", "paper", "foil", "sponge", "toothpaste", "shampoo", "bin bag", "tissue", "batteries"]),
];

/// The section a shopping item belongs in, by the first keyword that appears in its title (whole words first, then
/// substrings). Items nothing matches go to Other.
pub fn grocery_section(title: &str) -> &'static str {
    let lower = title.to_lowercase();
    let words: Vec<&str> = lower.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).collect();
    for (section, keys) in KEYWORDS {
        if keys.iter().any(|k| words.iter().any(|w| w == k || w.strip_suffix('s') == Some(*k))) {
            return section;
        }
    }
    for (section, keys) in KEYWORDS {
        if keys.iter().any(|k| lower.contains(k)) {
            return section;
        }
    }
    "Other"
}

/// Groups reminder ids into sections in route order; empty sections are left out.
pub fn auto_categorize(items: &[Reminder]) -> Vec<(&'static str, Vec<u64>)> {
    SECTIONS
        .iter()
        .filter_map(|s| {
            let ids: Vec<u64> = items.iter().filter(|r| grocery_section(&r.title) == *s).map(|r| r.id).collect();
            (!ids.is_empty()).then_some((*s, ids))
        })
        .collect()
}

/// The Unix time of local `day` (days since 1970-01-01) at `minutes` after midnight in a zone `offset_secs` east of
/// UTC. This is the due moment of a reminder with a time zone override.
pub fn due_in_zone(day: i64, minutes: u16, offset_secs: i64) -> i64 {
    day * 86_400 + i64::from(minutes) * 60 - offset_secs
}

/// The same moment shown in another zone: `(day, minutes)` for `unix` at `offset_secs` east of UTC.
pub fn local_in_zone(unix: i64, offset_secs: i64) -> (i64, u16) {
    let local = unix + offset_secs;
    (local.div_euclid(86_400), (local.rem_euclid(86_400) / 60) as u16)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn template_drops_dates_and_keeps_nesting() {
        let mut a = Reminder::new(10, "Pack bags");
        a.list = 2;
        a.due = Some(100);
        a.time = Some(540);
        a.completed = true;
        a.flagged = true;
        a.priority = 2;
        let mut b = Reminder::new(11, "Passport");
        b.list = 2;
        b.parent = Some(10);
        b.notes = "in the drawer".into();
        let other = Reminder::new(12, "Elsewhere");
        let t = save_template(" Trip ", &[a, b, other], 2);
        assert_eq!(t.name, "Trip");
        assert_eq!(t.items.len(), 2);
        assert_eq!(t.items[0], TemplateItem { title: "Pack bags".into(), notes: String::new(), priority: 2, parent: None });
        assert_eq!(t.items[1].parent, Some(0));
        let made = instantiate(&t, 5, 100);
        assert_eq!(made[0].id, 100);
        assert_eq!(made[1].parent, Some(100));
        assert!(made.iter().all(|r| r.list == 5 && r.due.is_none() && !r.completed && !r.flagged));
        assert_eq!(made[1].notes, "in the drawer");
    }

    #[test]
    fn stored_form_round_trips() {
        let t = Template {
            name: "Trip".into(),
            items: vec![
                TemplateItem { title: "A".into(), notes: "n".into(), priority: 1, parent: None },
                TemplateItem { title: "B".into(), notes: String::new(), priority: 0, parent: Some(0) },
            ],
        };
        assert_eq!(from_text(&to_text(&t)), Some(t));
        assert_eq!(from_text("no header\n"), None);
    }

    #[test]
    fn grocery_sections() {
        assert_eq!(grocery_section("Bananas"), "Produce");
        assert_eq!(grocery_section("whole milk"), "Dairy & Eggs");
        assert_eq!(grocery_section("Sourdough bread"), "Bakery");
        assert_eq!(grocery_section("frozen pizza"), "Frozen");
        assert_eq!(grocery_section("Laundry detergent"), "Household");
        assert_eq!(grocery_section("birthday card"), "Other");
    }

    #[test]
    fn categorize_in_route_order() {
        let items = vec![Reminder::new(1, "Milk"), Reminder::new(2, "Apples"), Reminder::new(3, "Eggs"), Reminder::new(4, "Stamps")];
        let g = auto_categorize(&items);
        assert_eq!(g, vec![("Produce", vec![2]), ("Dairy & Eggs", vec![1, 3]), ("Other", vec![4])]);
    }

    #[test]
    fn zones() {
        // 09:00 on day 20000 in UTC+2 is 07:00 UTC
        let u = due_in_zone(20_000, 540, 7200);
        assert_eq!(u, 20_000 * 86_400 + 7 * 3600);
        assert_eq!(local_in_zone(u, 7200), (20_000, 540));
        assert_eq!(local_in_zone(u, -5 * 3600), (20_000, 120));
    }
}
