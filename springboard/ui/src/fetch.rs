//! Fetch requests over in-memory records (`NativeFetchRequest`): a predicate, sort descriptors, an offset and a limit.
//! A record exposes its fields by name. Predicates are a tree of comparisons, case-insensitive text containment and the
//! boolean combinators. Matching and sorting are plain Rust over slices, so a list and the query behind it share one
//! definition.

use std::cmp::Ordering;

/// One field value a record exposes to predicates and sorting.
#[derive(Clone, Debug, PartialEq)]
pub enum Field {
    Null,
    Bool(bool),
    Int(i64),
    Text(String),
}

impl Field {
    /// A total order. Values of one kind compare by value; text ignores case first and then compares exactly.
    /// Values of different kinds order by kind, null first.
    pub fn order(&self, other: &Field) -> Ordering {
        match (self, other) {
            (Field::Bool(a), Field::Bool(b)) => a.cmp(b),
            (Field::Int(a), Field::Int(b)) => a.cmp(b),
            (Field::Text(a), Field::Text(b)) => a.to_lowercase().cmp(&b.to_lowercase()).then_with(|| a.cmp(b)),
            _ => self.rank().cmp(&other.rank()),
        }
    }

    fn rank(&self) -> u8 {
        match self {
            Field::Null => 0,
            Field::Bool(_) => 1,
            Field::Int(_) => 2,
            Field::Text(_) => 3,
        }
    }
}

/// A record that exposes named fields to fetch requests. A field the record does not have is `None`.
pub trait Record {
    fn field(&self, key: &str) -> Option<Field>;
}

/// A comparison operator in a predicate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

impl Op {
    fn holds(self, ordering: Ordering) -> bool {
        match self {
            Op::Eq => ordering == Ordering::Equal,
            Op::Ne => ordering != Ordering::Equal,
            Op::Lt => ordering == Ordering::Less,
            Op::Le => ordering != Ordering::Greater,
            Op::Gt => ordering == Ordering::Greater,
            Op::Ge => ordering != Ordering::Less,
        }
    }
}

/// The filter of a request (`NativePredicate`). `All` matches every record.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum Predicate {
    #[default]
    All,
    /// A record matches when its field exists and the comparison holds. A missing field never matches.
    Compare { key: String, op: Op, value: Field },
    /// Case-insensitive substring match on a text field.
    Contains { key: String, text: String },
    And(Vec<Predicate>),
    Or(Vec<Predicate>),
    Not(Box<Predicate>),
}

impl Predicate {
    pub fn compare(key: &str, op: Op, value: Field) -> Self {
        Predicate::Compare { key: key.into(), op, value }
    }

    pub fn contains(key: &str, text: &str) -> Self {
        Predicate::Contains { key: key.into(), text: text.into() }
    }

    pub fn and(self, other: Predicate) -> Self {
        Predicate::And(vec![self, other])
    }

    pub fn or(self, other: Predicate) -> Self {
        Predicate::Or(vec![self, other])
    }

    pub fn negate(self) -> Self {
        Predicate::Not(Box::new(self))
    }

    pub fn matches<R: Record + ?Sized>(&self, record: &R) -> bool {
        match self {
            Predicate::All => true,
            Predicate::Compare { key, op, value } => {
                record.field(key).is_some_and(|field| op.holds(field.order(value)))
            }
            Predicate::Contains { key, text } => match record.field(key) {
                Some(Field::Text(s)) => s.to_lowercase().contains(&text.to_lowercase()),
                _ => false,
            },
            Predicate::And(list) => list.iter().all(|p| p.matches(record)),
            Predicate::Or(list) => list.iter().any(|p| p.matches(record)),
            Predicate::Not(p) => !p.matches(record),
        }
    }
}

/// One sort key of a request (`NativeSortDescriptor`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SortDescriptor {
    pub key: String,
    pub ascending: bool,
}

impl SortDescriptor {
    pub fn new(key: &str, ascending: bool) -> Self {
        SortDescriptor { key: key.into(), ascending }
    }
}

/// What to fetch: the records that match `predicate`, ordered by `sort`, skipping `offset`, at most `limit` of them.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FetchRequest {
    pub predicate: Predicate,
    pub sort: Vec<SortDescriptor>,
    pub offset: usize,
    pub limit: Option<usize>,
}

impl FetchRequest {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn filter(mut self, predicate: Predicate) -> Self {
        self.predicate = predicate;
        self
    }

    /// Adds a sort key. Keys added first take precedence.
    pub fn sorted_by(mut self, key: &str, ascending: bool) -> Self {
        self.sort.push(SortDescriptor::new(key, ascending));
        self
    }

    pub fn offset(mut self, offset: usize) -> Self {
        self.offset = offset;
        self
    }

    pub fn limit(mut self, limit: usize) -> Self {
        self.limit = Some(limit);
        self
    }
}

/// The positions in `records` that `request` selects, in result order. Sorting is stable, so records that tie keep
/// their original order. Offset and limit apply after sorting.
pub fn fetch_indices<R: Record>(records: &[R], request: &FetchRequest) -> Vec<usize> {
    let mut hits: Vec<usize> = (0..records.len()).filter(|&i| request.predicate.matches(&records[i])).collect();
    if !request.sort.is_empty() {
        hits.sort_by(|&a, &b| compare_by(&records[a], &records[b], &request.sort));
    }
    hits.into_iter().skip(request.offset).take(request.limit.unwrap_or(usize::MAX)).collect()
}

/// The records that `request` selects, in result order.
pub fn fetch<'a, R: Record>(records: &'a [R], request: &FetchRequest) -> Vec<&'a R> {
    fetch_indices(records, request).into_iter().map(|i| &records[i]).collect()
}

fn compare_by<R: Record>(a: &R, b: &R, sort: &[SortDescriptor]) -> Ordering {
    for d in sort {
        let fa = a.field(&d.key).unwrap_or(Field::Null);
        let fb = b.field(&d.key).unwrap_or(Field::Null);
        let ordering = fa.order(&fb);
        let ordering = if d.ascending { ordering } else { ordering.reverse() };
        if ordering != Ordering::Equal {
            return ordering;
        }
    }
    Ordering::Equal
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Item {
        name: &'static str,
        rank: i64,
        done: bool,
    }

    impl Record for Item {
        fn field(&self, key: &str) -> Option<Field> {
            match key {
                "name" => Some(Field::Text(self.name.into())),
                "rank" => Some(Field::Int(self.rank)),
                "done" => Some(Field::Bool(self.done)),
                _ => None,
            }
        }
    }

    fn items() -> Vec<Item> {
        vec![
            Item { name: "Birch", rank: 2, done: false },
            Item { name: "alder", rank: 5, done: true },
            Item { name: "Cedar", rank: 2, done: true },
            Item { name: "Aspen", rank: 9, done: false },
        ]
    }

    fn names(found: &[&Item]) -> Vec<&'static str> {
        found.iter().map(|i| i.name).collect()
    }

    #[test]
    fn equality_selects_matching_records_and_missing_fields_never_match() {
        let all = items();
        let done = fetch(&all, &FetchRequest::new().filter(Predicate::compare("done", Op::Eq, Field::Bool(true))));
        assert_eq!(names(&done), ["alder", "Cedar"]);
        let unknown = fetch(&all, &FetchRequest::new().filter(Predicate::compare("colour", Op::Eq, Field::Null)));
        assert!(unknown.is_empty());
    }

    #[test]
    fn text_containment_ignores_case() {
        let all = items();
        let found = fetch(&all, &FetchRequest::new().filter(Predicate::contains("name", "ED")));
        assert_eq!(names(&found), ["Cedar"]);
    }

    #[test]
    fn combinators_and_ordering_comparisons_compose() {
        let all = items();
        let p = Predicate::compare("rank", Op::Ge, Field::Int(2))
            .and(Predicate::compare("rank", Op::Lt, Field::Int(9)))
            .and(Predicate::compare("done", Op::Eq, Field::Bool(false)).negate());
        assert_eq!(names(&fetch(&all, &FetchRequest::new().filter(p))), ["alder", "Cedar"]);
        let either = Predicate::contains("name", "aspen").or(Predicate::compare("rank", Op::Eq, Field::Int(5)));
        assert_eq!(names(&fetch(&all, &FetchRequest::new().filter(either))), ["alder", "Aspen"]);
    }

    #[test]
    fn sorts_by_several_keys_with_direction() {
        let all = items();
        let req = FetchRequest::new().sorted_by("rank", false).sorted_by("name", true);
        assert_eq!(names(&fetch(&all, &req)), ["Aspen", "alder", "Birch", "Cedar"]);
    }

    #[test]
    fn ties_keep_their_original_order() {
        let all = items();
        let req = FetchRequest::new().sorted_by("done", true);
        assert_eq!(names(&fetch(&all, &req)), ["Birch", "Aspen", "alder", "Cedar"]);
    }

    #[test]
    fn offset_and_limit_apply_after_sorting() {
        let all = items();
        let req = FetchRequest::new().sorted_by("name", true).offset(1).limit(2);
        assert_eq!(names(&fetch(&all, &req)), ["alder", "Birch"]);
        assert_eq!(fetch_indices(&all, &req), vec![1, 0]);
    }

    #[test]
    fn mixed_kinds_order_by_kind_and_null_sorts_first() {
        assert_eq!(Field::Null.order(&Field::Int(0)), Ordering::Less);
        assert_eq!(Field::Int(7).order(&Field::Text("a".into())), Ordering::Less);
        assert_eq!(Field::Text("cherry".into()).order(&Field::Text("Banana".into())), Ordering::Greater);
        assert_eq!(Field::Text("apricot".into()).order(&Field::Text("Banana".into())), Ordering::Less);
        assert_eq!(Field::Text("a".into()).order(&Field::Text("A".into())), Ordering::Greater);
    }
}
