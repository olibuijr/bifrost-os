//! Metadata queries over file records (the system-wide search query model, `NativeMetadataQuery` parity): predicates on
//! name, extension, kind, size and modification time, combined with all, any and not, then sorted and limited.
//! Pure model: the caller gathers the `Metadata` records (a directory walk or an app's own index) and draws the
//! results. Text comparisons ignore case.
//!
//! Gaps: no live updates while a query runs, no query over the whole file system (the caller supplies the records),
//! no content or tag predicates, and no grouping or result counts per group.

/// The facts a predicate may test about one item.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Metadata {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    pub size: u64,
    /// Seconds since the Unix epoch.
    pub modified: i64,
}

impl Metadata {
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
        Metadata { name: name.into(), path: format!("/home/u/{name}"), is_dir: false, size, modified }
    }

    fn dir(name: &str, modified: i64) -> Metadata {
        Metadata { name: name.into(), path: format!("/home/u/{name}"), is_dir: true, size: 0, modified }
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
}
