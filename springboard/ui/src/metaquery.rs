//! Metadata queries over file records (the system-wide search query model, `NativeMetadataQuery` parity): predicates on
//! name, extension, kind, size and modification time, combined with all, any and not, then sorted and limited.
//! Pure model: the caller gathers the `Metadata` records (a directory walk or an app's own index) and draws the
//! results. Text comparisons ignore case.
//!
//! Attributes: an item can carry named, typed attributes (display name, keywords, tags and so on, see `keys`). The
//! attribute predicates test them: presence, equality, containment in a text or a list, and numeric bounds on numbers
//! and dates. A predicate on a missing attribute, or on an attribute of another type, does not hold.
//!
//! Gaps: no live updates while a query runs, no query over the whole file system (the caller supplies the records),
//! no importers that fill attributes from file content, no label colours or label sets, no sort by attribute, and no
//! grouping or result counts per group.

/// Attribute names that the file records and the predicates use. The names are plain and lower-case.
pub mod keys {
    pub const DISPLAY_NAME: &str = "display-name";
    pub const CONTENT_TYPE: &str = "content-type";
    pub const KEYWORDS: &str = "keywords";
    /// User tags, such as the labels a file is marked with.
    pub const TAGS: &str = "tags";
}

/// The value of one attribute.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Value {
    Text(String),
    /// Integer quantities (counts, sizes, bit depths).
    Number(i64),
    /// Seconds since the Unix epoch.
    Date(i64),
    Flag(bool),
    /// Several values, such as keywords or tags. Stored in the order given.
    List(Vec<String>),
}

impl Value {
    /// The number a numeric comparison uses: a number as it is, a date as its time. Other types have none.
    fn as_number(&self) -> Option<i64> {
        match self {
            Value::Number(n) | Value::Date(n) => Some(*n),
            _ => None,
        }
    }
}

/// The facts a predicate may test about one item.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Metadata {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    pub size: u64,
    /// Seconds since the Unix epoch.
    pub modified: i64,
    /// Typed attributes by name, sorted by name.
    pub attributes: std::collections::BTreeMap<String, Value>,
}

impl Metadata {
    /// The same record with one more attribute, replacing any earlier value of that name.
    pub fn with_attribute(mut self, key: &str, value: Value) -> Self {
        self.attributes.insert(key.to_string(), value);
        self
    }

    /// The attribute of that name, when the item has it.
    pub fn attribute(&self, key: &str) -> Option<&Value> {
        self.attributes.get(key)
    }

    /// The lowercase extension without its dot, or "" when there is none. A name that starts with a dot has no
    /// extension (".rc" is not an extension of "rc"), and "archive.tar.gz" has "gz".
    pub fn extension(&self) -> String {
        match self.name.rfind('.') {
            Some(i) if i > 0 => self.name[i + 1..].to_lowercase(),
            _ => String::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Predicate {
    /// The name contains the text.
    NameContains(String),
    /// The name starts with the text.
    NameBeginsWith(String),
    /// The extension equals the text, with or without a leading dot.
    Extension(String),
    IsDirectory(bool),
    /// Inclusive bounds on the size in bytes.
    SizeAtLeast(u64),
    SizeAtMost(u64),
    /// Modified at or after the time (seconds since the epoch).
    ModifiedAtOrAfter(i64),
    /// Modified strictly before the time.
    ModifiedBefore(i64),
    /// The item has an attribute of that name, of any type.
    HasAttribute(String),
    /// The attribute equals the value. Text ignores case; other types compare exactly.
    AttributeEquals(String, Value),
    /// Text contains the needle, ignoring case; a list has an item equal to the needle, ignoring case.
    AttributeContains(String, String),
    /// A number or a date is at least the bound (inclusive).
    AttributeAtLeast(String, i64),
    /// A number or a date is at most the bound (inclusive).
    AttributeAtMost(String, i64),
    /// Every sub-predicate holds. An empty list holds for every item.
    All(Vec<Predicate>),
    /// At least one sub-predicate holds. An empty list holds for no item.
    Any(Vec<Predicate>),
    Not(Box<Predicate>),
}

impl Default for Predicate {
    fn default() -> Self {
        Predicate::All(Vec::new())
    }
}

impl Predicate {
    pub fn matches(&self, m: &Metadata) -> bool {
        match self {
            Predicate::NameContains(t) => m.name.to_lowercase().contains(&t.to_lowercase()),
            Predicate::NameBeginsWith(t) => m.name.to_lowercase().starts_with(&t.to_lowercase()),
            Predicate::Extension(e) => m.extension() == e.trim_start_matches('.').to_lowercase(),
            Predicate::IsDirectory(d) => m.is_dir == *d,
            Predicate::SizeAtLeast(n) => m.size >= *n,
            Predicate::SizeAtMost(n) => m.size <= *n,
            Predicate::ModifiedAtOrAfter(t) => m.modified >= *t,
            Predicate::ModifiedBefore(t) => m.modified < *t,
            Predicate::HasAttribute(key) => m.attribute(key).is_some(),
            Predicate::AttributeEquals(key, want) => match (m.attribute(key), want) {
                (Some(Value::Text(have)), Value::Text(want)) => have.to_lowercase() == want.to_lowercase(),
                (Some(have), want) => have == want,
                (None, _) => false,
            },
            Predicate::AttributeContains(key, needle) => {
                let needle = needle.to_lowercase();
                match m.attribute(key) {
                    Some(Value::Text(text)) => text.to_lowercase().contains(&needle),
                    Some(Value::List(items)) => items.iter().any(|item| item.to_lowercase() == needle),
                    _ => false,
                }
            }
            Predicate::AttributeAtLeast(key, n) => m.attribute(key).and_then(Value::as_number).is_some_and(|v| v >= *n),
            Predicate::AttributeAtMost(key, n) => m.attribute(key).and_then(Value::as_number).is_some_and(|v| v <= *n),
            Predicate::All(ps) => ps.iter().all(|p| p.matches(m)),
            Predicate::Any(ps) => ps.iter().any(|p| p.matches(m)),
            Predicate::Not(p) => !p.matches(m),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SortKey {
    /// Case-insensitive name, exact spelling breaks ties.
    Name,
    Size,
    Modified,
}

#[derive(Clone, Debug, Default)]
pub struct Query {
    pub predicate: Predicate,
    /// The sort key and whether it runs descending. `None` keeps the input order.
    pub sort: Option<(SortKey, bool)>,
    /// At most this many results.
    pub limit: Option<usize>,
}

impl Query {
    /// Indices into `items` of the matching records, in result order.
    pub fn run(&self, items: &[Metadata]) -> Vec<usize> {
        let mut hits: Vec<usize> = (0..items.len()).filter(|&i| self.predicate.matches(&items[i])).collect();
        if let Some((key, descending)) = self.sort {
            hits.sort_by(|&a, &b| {
                let (x, y) = (&items[a], &items[b]);
                let ord = match key {
                    SortKey::Name => x
                        .name
                        .to_lowercase()
                        .cmp(&y.name.to_lowercase())
                        .then_with(|| x.name.cmp(&y.name)),
                    SortKey::Size => x.size.cmp(&y.size),
                    SortKey::Modified => x.modified.cmp(&y.modified),
                };
                if descending {
                    ord.reverse()
                } else {
                    ord
                }
            });
        }
        if let Some(n) = self.limit {
            hits.truncate(n);
        }
        hits
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(name: &str, size: u64, modified: i64) -> Metadata {
        Metadata {
            name: name.into(),
            path: format!("/home/u/{name}"),
            is_dir: false,
            size,
            modified,
            attributes: Default::default(),
        }
    }

    fn dir(name: &str, modified: i64) -> Metadata {
        Metadata {
            name: name.into(),
            path: format!("/home/u/{name}"),
            is_dir: true,
            size: 0,
            modified,
            attributes: Default::default(),
        }
    }

    /// Index 0..=5 in this order: Invoice-2026.PDF, notes.txt, .rc, archive.tar.gz, Photos, readme.
    fn sample() -> Vec<Metadata> {
        vec![
            file("Invoice-2026.PDF", 4_000, 300),
            file("notes.txt", 120, 500),
            file(".rc", 10, 100),
            file("archive.tar.gz", 90_000, 200),
            dir("Photos", 400),
            file("readme", 0, 50),
        ]
    }

    fn names(items: &[Metadata], hits: &[usize]) -> Vec<String> {
        hits.iter().map(|&i| items[i].name.clone()).collect()
    }

    fn only(p: Predicate) -> Query {
        Query { predicate: p, ..Default::default() }
    }

    #[test]
    fn name_predicates_ignore_case_and_differ_in_position() {
        let items = sample();
        let contains = only(Predicate::NameContains("VOICE".into())).run(&items);
        assert_eq!(names(&items, &contains), ["Invoice-2026.PDF"]);
        assert!(only(Predicate::NameBeginsWith("voice".into())).run(&items).is_empty(), "voice is not at the start");
        let begins = only(Predicate::NameBeginsWith("PH".into())).run(&items);
        assert_eq!(names(&items, &begins), ["Photos"]);
    }

    #[test]
    fn extension_is_the_last_dotted_part_and_a_leading_dot_is_not_one() {
        let items = sample();
        assert_eq!(items[0].extension(), "pdf");
        assert_eq!(items[3].extension(), "gz", "only the last part counts");
        assert_eq!(items[2].extension(), "", "a dot file has no extension");
        assert_eq!(items[5].extension(), "", "no dot, no extension");
        let hits = only(Predicate::Extension(".Pdf".into())).run(&items);
        assert_eq!(names(&items, &hits), ["Invoice-2026.PDF"]);
    }

    #[test]
    fn all_any_and_not_combine_and_empty_lists_have_the_usual_meaning() {
        let items = sample();
        let big_documents = Predicate::All(vec![
            Predicate::IsDirectory(false),
            Predicate::SizeAtLeast(100),
            Predicate::Not(Box::new(Predicate::Extension("gz".into()))),
        ]);
        assert_eq!(names(&items, &only(big_documents).run(&items)), ["Invoice-2026.PDF", "notes.txt"]);

        let either = Predicate::Any(vec![Predicate::Extension("txt".into()), Predicate::IsDirectory(true)]);
        assert_eq!(names(&items, &only(either).run(&items)), ["notes.txt", "Photos"]);

        assert_eq!(only(Predicate::All(vec![])).run(&items).len(), items.len());
        assert!(only(Predicate::Any(vec![])).run(&items).is_empty());
    }

    #[test]
    fn size_bounds_are_inclusive_and_time_bounds_are_half_open() {
        let items = sample();
        let sized = Predicate::All(vec![Predicate::SizeAtLeast(10), Predicate::SizeAtMost(120)]);
        assert_eq!(names(&items, &only(sized).run(&items)), ["notes.txt", ".rc"]);

        let since = only(Predicate::ModifiedAtOrAfter(300)).run(&items);
        assert_eq!(names(&items, &since), ["Invoice-2026.PDF", "notes.txt", "Photos"], "300 itself is included");
        let before = only(Predicate::ModifiedBefore(200)).run(&items);
        assert_eq!(names(&items, &before), [".rc", "readme"], "200 itself is excluded");
    }

    #[test]
    fn run_sorts_then_limits_and_keeps_input_order_without_a_sort() {
        let items = sample();
        let newest_first = Query { sort: Some((SortKey::Modified, true)), limit: Some(3), ..Default::default() };
        assert_eq!(names(&items, &newest_first.run(&items)), ["notes.txt", "Photos", "Invoice-2026.PDF"]);

        let by_name = Query { sort: Some((SortKey::Name, false)), ..Default::default() };
        assert_eq!(
            names(&items, &by_name.run(&items)),
            [".rc", "archive.tar.gz", "Invoice-2026.PDF", "notes.txt", "Photos", "readme"],
            "names compare without case"
        );

        let unsorted = Query { limit: Some(2), ..Default::default() };
        assert_eq!(unsorted.run(&items), vec![0, 1]);

        let biggest = Query { sort: Some((SortKey::Size, true)), ..Default::default() };
        assert_eq!(biggest.run(&items)[0], 3);
    }

    #[test]
    fn a_query_over_no_records_is_empty() {
        assert!(Query::default().run(&[]).is_empty());
    }

    /// Index 0: 2025-tax.pdf (tags Work, Tax; display name; added at 300). Index 1: beach.jpg (tag Holiday).
    /// Index 2: notes.txt (no attributes).
    fn tagged() -> Vec<Metadata> {
        vec![
            file("2025-tax.pdf", 4_000, 300)
                .with_attribute(keys::TAGS, Value::List(vec!["Work".into(), "Tax".into()]))
                .with_attribute(keys::DISPLAY_NAME, Value::Text("Tax return 2025".into()))
                .with_attribute("added", Value::Date(300)),
            file("beach.jpg", 900, 500).with_attribute(keys::TAGS, Value::List(vec!["Holiday".into()])),
            file("notes.txt", 120, 200),
        ]
    }

    #[test]
    fn equality_ignores_case_for_text_and_never_matches_another_type() {
        let items = tagged();
        let same = only(Predicate::AttributeEquals(keys::DISPLAY_NAME.into(), Value::Text("TAX return 2025".into())));
        assert_eq!(names(&items, &same.run(&items)), ["2025-tax.pdf"]);
        let number = only(Predicate::AttributeEquals(keys::DISPLAY_NAME.into(), Value::Number(5)));
        assert!(number.run(&items).is_empty(), "a number never equals a text");
    }

    #[test]
    fn contains_matches_a_whole_list_item_but_a_substring_of_text() {
        let items = tagged();
        let tax = only(Predicate::AttributeContains(keys::TAGS.into(), "tax".into())).run(&items);
        assert_eq!(names(&items, &tax), ["2025-tax.pdf"], "Tax is a whole item of the list");
        let partial = only(Predicate::AttributeContains(keys::TAGS.into(), "ta".into())).run(&items);
        assert!(partial.is_empty(), "a part of a list item is not a match");
        let text = only(Predicate::AttributeContains(keys::DISPLAY_NAME.into(), "RETURN".into())).run(&items);
        assert_eq!(names(&items, &text), ["2025-tax.pdf"], "text matches a substring");
    }

    #[test]
    fn numeric_bounds_apply_to_numbers_and_dates_and_are_inclusive() {
        let items = tagged();
        let at_least = only(Predicate::AttributeAtLeast("added".into(), 300)).run(&items);
        assert_eq!(names(&items, &at_least), ["2025-tax.pdf"], "300 itself is included");
        let at_most = only(Predicate::AttributeAtMost("added".into(), 299)).run(&items);
        assert!(at_most.is_empty());
        let text_bound = only(Predicate::AttributeAtLeast(keys::DISPLAY_NAME.into(), 0)).run(&items);
        assert!(text_bound.is_empty(), "a text has no number to compare");
    }

    #[test]
    fn has_attribute_and_its_negation_split_the_tagged_items() {
        let items = tagged();
        let untagged = only(Predicate::Not(Box::new(Predicate::HasAttribute(keys::TAGS.into())))).run(&items);
        assert_eq!(names(&items, &untagged), ["notes.txt"]);
        let tagged_count = only(Predicate::HasAttribute(keys::TAGS.into())).run(&items);
        assert_eq!(tagged_count, vec![0, 1]);
    }

    #[test]
    fn attribute_predicates_combine_with_name_predicates() {
        let items = tagged();
        let work_pdf = Predicate::All(vec![
            Predicate::Extension("pdf".into()),
            Predicate::AttributeContains(keys::TAGS.into(), "work".into()),
        ]);
        assert_eq!(names(&items, &only(work_pdf).run(&items)), ["2025-tax.pdf"]);

        let either = Predicate::Any(vec![
            Predicate::AttributeContains(keys::TAGS.into(), "holiday".into()),
            Predicate::Extension("txt".into()),
        ]);
        assert_eq!(names(&items, &only(either).run(&items)), ["beach.jpg", "notes.txt"]);
    }

    #[test]
    fn with_attribute_replaces_an_earlier_value_of_the_same_name() {
        let item = file("a", 1, 1).with_attribute(keys::KEYWORDS, Value::Number(1)).with_attribute(keys::KEYWORDS, Value::Number(2));
        assert_eq!(item.attribute(keys::KEYWORDS), Some(&Value::Number(2)));
        assert_eq!(item.attributes.len(), 1);
    }
}
