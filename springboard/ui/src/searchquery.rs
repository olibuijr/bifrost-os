//! App-side search queries: a query string run over an app's own searchable records, with the query context
//! (fetch attributes, keyboard language, filter queries) and the searched fields chosen by the caller.
//!
//! Query grammar: whitespace separates words and every word must match (AND). A word matches a field when one of
//! the field's words starts with it, so "mil" finds "Milk". Text in double quotes is a phrase: its words must appear
//! next to each other in one field, and the last word of a phrase may be a prefix. Matching ignores case and punctuation.
//! Filter queries are parsed the same way and every one of them must match too, but they do not add to the score.
//! Results are ordered best first: a word in the title weighs more than one in the keywords, which weighs more than one
//! in the description. Ties keep the order of the records.
//!
//! Pure model: the caller supplies the records (an app's own index, flattened); the shell's on-device index is not
//! queried from here. Gaps: no streamed batches or cancellation (the caller gets all hits at once), no protection
//! classes, no source options for mail, and `fetch_attributes` is kept but does not trim the result.

/// The fields a query may search, `CSSearchQuery`'s attribute names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Field {
    Title,
    Keywords,
    Description,
}

impl Field {
    /// Every searchable field, the default for a query.
    pub const ALL: [Field; 3] = [Field::Title, Field::Keywords, Field::Description];

    /// Weight of a match in this field. A title match ranks above a keyword match, above a description match.
    fn weight(self) -> u32 {
        match self {
            Field::Title => 3,
            Field::Keywords => 2,
            Field::Description => 1,
        }
    }
}

/// Why a query could not run. Stable codes within [`DOMAIN`]: logs and tests may rely on them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// The query string holds no word or phrase, so there is nothing to match.
    EmptyQueryString,
    /// The query searches no field.
    NoFields,
}

impl Error {
    /// The error domain name, for logs.
    pub const DOMAIN: &'static str = "bifrost.quicksearch.query";

    pub fn code(self) -> u32 {
        match self {
            Error::EmptyQueryString => 1,
            Error::NoFields => 2,
        }
    }
}

/// `CSSearchQueryContext`: the behaviour the query runs with.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Context {
    /// `fetchAttributes`: the attributes the caller wants back. Recorded for the caller; hits always carry the index.
    pub fetch_attributes: Vec<String>,
    /// `keyboardLanguage`: a BCP 47 tag for the query, recorded for the caller.
    pub keyboard_language: Option<String>,
    /// `filterQueries`: further query strings, each one required of every hit.
    pub filter_queries: Vec<String>,
}

/// One searchable record, flattened from an item's attributes by the caller.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Record {
    pub title: String,
    pub description: String,
    pub keywords: Vec<String>,
}

/// A query: the query string, the fields it searches and its context.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Query {
    pub query_string: String,
    pub fields: Vec<Field>,
    pub context: Context,
}

impl Query {
    /// A query that searches every field with the default context.
    pub fn new(query_string: &str) -> Self {
        Query { query_string: query_string.into(), fields: Field::ALL.to_vec(), context: Context::default() }
    }

    pub fn with_context(query_string: &str, context: Context) -> Self {
        Query { context, ..Query::new(query_string) }
    }

    /// Restricts the query to `fields`.
    pub fn fields(mut self, fields: &[Field]) -> Self {
        self.fields = fields.to_vec();
        self
    }
}

/// A matching record: its position in the slice passed to [`run`], and its score (higher ranks first).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Hit {
    pub index: usize,
    pub score: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Term {
    Word(String),
    Phrase(Vec<String>),
}

/// The lowercase words in `text`: runs of letters and digits, punctuation removed.
fn words(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).map(|w| w.to_lowercase()).collect()
}

/// Splits a query string into terms. Text outside double quotes gives one term per word; text inside them gives one
/// phrase. An unclosed quote takes the rest of the string as a phrase.
fn parse(text: &str) -> Vec<Term> {
    let mut terms = Vec::new();
    for (i, part) in text.split('"').enumerate() {
        let w = words(part);
        if i % 2 == 1 && w.len() > 1 {
            terms.push(Term::Phrase(w));
        } else {
            terms.extend(w.into_iter().map(Term::Word));
        }
    }
    terms
}

/// The best field weight at which `term` matches the fields, or `None` when it matches none of them.
fn best_weight(term: &Term, fields: &[(Field, Vec<String>)]) -> Option<u32> {
    fields
        .iter()
        .filter(|(_, ws)| match term {
            Term::Word(w) => ws.iter().any(|x| x.starts_with(w.as_str())),
            Term::Phrase(p) => ws.len() >= p.len() && ws.windows(p.len()).any(|win| {
                let last = p.len() - 1;
                win.iter().zip(p).enumerate().all(|(i, (a, b))| if i == last { a.starts_with(b.as_str()) } else { a == b })
            }),
        })
        .map(|(f, _)| f.weight())
        .max()
}

/// Runs `query` over `records`. Returns the matching records best first.
pub fn run(query: &Query, records: &[Record]) -> Result<Vec<Hit>, Error> {
    if query.fields.is_empty() {
        return Err(Error::NoFields);
    }
    let terms = parse(&query.query_string);
    if terms.is_empty() {
        return Err(Error::EmptyQueryString);
    }
    let filters: Vec<Term> = query.context.filter_queries.iter().flat_map(|f| parse(f)).collect();
    let mut hits = Vec::new();
    for (index, r) in records.iter().enumerate() {
        let mut fields: Vec<(Field, Vec<String>)> = Vec::new();
        for f in &query.fields {
            let ws = match f {
                Field::Title => words(&r.title),
                Field::Description => words(&r.description),
                Field::Keywords => r.keywords.iter().flat_map(|k| words(k)).collect(),
            };
            fields.push((*f, ws));
        }
        if filters.iter().any(|t| best_weight(t, &fields).is_none()) {
            continue;
        }
        let mut score = 0;
        let mut all = true;
        for t in &terms {
            match best_weight(t, &fields) {
                Some(w) => score += w,
                None => {
                    all = false;
                    break;
                }
            }
        }
        if all {
            hits.push(Hit { index, score });
        }
    }
    hits.sort_by(|a, b| b.score.cmp(&a.score).then(a.index.cmp(&b.index)));
    Ok(hits)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(title: &str, description: &str, keywords: &[&str]) -> Record {
        Record { title: title.into(), description: description.into(), keywords: keywords.iter().map(|s| s.to_string()).collect() }
    }

    fn indexes(hits: &[Hit]) -> Vec<usize> {
        hits.iter().map(|h| h.index).collect()
    }

    #[test]
    fn every_word_must_match_and_a_word_matches_a_prefix() {
        let records = [rec("Milk", "oat", &[]), rec("Milk", "dairy", &[]), rec("Bread", "oat", &[])];
        let hits = run(&Query::new("mil oa"), &records).unwrap();
        assert_eq!(indexes(&hits), [0]);
        assert_eq!(indexes(&run(&Query::new("MILK"), &records).unwrap()), [0, 1]);
    }

    #[test]
    fn title_matches_rank_above_keywords_above_description() {
        let records = [rec("Notes", "buy milk", &[]), rec("Shopping", "", &["milk"]), rec("Milk", "", &[])];
        let hits = run(&Query::new("milk"), &records).unwrap();
        assert_eq!(indexes(&hits), [2, 1, 0]);
        assert_eq!(hits[0].score, 3);
        assert_eq!(hits[2].score, 1);
    }

    #[test]
    fn ties_keep_the_order_of_the_records() {
        let records = [rec("Alpha", "", &[]), rec("Beta", "", &[]), rec("Alpha", "", &[])];
        assert_eq!(indexes(&run(&Query::new("alpha"), &records).unwrap()), [0, 2]);
    }

    #[test]
    fn quoted_phrases_need_adjacent_words_and_the_last_word_may_be_a_prefix() {
        let records = [rec("Oat milk", "", &[]), rec("Milk oat", "", &[])];
        assert_eq!(indexes(&run(&Query::new("\"oat milk\""), &records).unwrap()), [0]);
        assert_eq!(indexes(&run(&Query::new("\"oat mi\""), &records).unwrap()), [0]);
        assert_eq!(indexes(&run(&Query::new("\"milk\" oat"), &records).unwrap()), [0, 1]);
    }

    #[test]
    fn filter_queries_narrow_the_hits_without_adding_to_the_score() {
        let records = [rec("Milk", "", &["shop"]), rec("Milk", "", &["home"])];
        let ctx = Context { filter_queries: vec!["shop".into()], ..Context::default() };
        let hits = run(&Query::with_context("milk", ctx), &records).unwrap();
        assert_eq!(hits, [Hit { index: 0, score: 3 }]);
    }

    #[test]
    fn restricting_the_fields_excludes_the_others() {
        let records = [rec("Milk", "", &[]), rec("Bread", "milk", &[])];
        let only_description = Query::new("milk").fields(&[Field::Description]);
        assert_eq!(indexes(&run(&only_description, &records).unwrap()), [1]);
        assert_eq!(run(&Query::new("milk").fields(&[]), &records), Err(Error::NoFields));
    }

    #[test]
    fn empty_query_strings_and_unicode_are_handled() {
        let records = [rec("Mjólk", "", &[])];
        assert_eq!(run(&Query::new("  \" \" "), &records), Err(Error::EmptyQueryString));
        assert_eq!(indexes(&run(&Query::new("MJÓL"), &records).unwrap()), [0]);
        assert_eq!(Error::EmptyQueryString.code(), 1);
        assert_eq!(Error::NoFields.code(), 2);
    }
}
