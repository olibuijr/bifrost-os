//! Core Spotlight query side: `CSSearchQuery`, `CSSearchQueryContext`, `CSUserQuery`, `CSSuggestion`,
//! `CSSearchableIndexDelegate` and acknowledged index writes with client state.
//!
//! Query language (as Apple's `queryString`): `attr == "text"` or `!=`, `<` `>` `<=` `>=` for numbers and times,
//! `*` wildcard inside text, modifiers after the closing quote (`c` ignore case, `d` ignore diacritics, `w` match any
//! whole word of the value), `&&` binds tighter than `||`, parentheses group, and `$time.now` or `$time.now(-3600)`
//! (seconds from now) as a time value. Pure model, std only; callers pass the clock.
//! Gaps: `CSSearchQueryContext.sourceOptions` is recorded but has no effect, no SpotlightSearchTool, no disk store.

use std::cell::Cell;
use std::collections::BTreeMap;
use std::rc::Rc;

/// Index and query errors. `RemoteConnection` is `CSIndexErrorCode.remoteConnectionError`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// The query string does not parse.
    InvalidQuery,
    /// The query was cancelled before it finished.
    Cancelled,
    /// The indexer cannot be reached (`CSIndexErrorCode.remoteConnectionError`).
    RemoteConnection,
    /// A batch call was made out of order.
    BatchState,
}

impl Error {
    pub const DOMAIN: &'static str = "bifrost.spotlight";
    pub fn code(self) -> u32 {
        match self {
            Error::InvalidQuery => 1,
            Error::Cancelled => 2,
            Error::RemoteConnection => 3,
            Error::BatchState => 4,
        }
    }
}

/// An indexed item: identifier, domain, text attributes and numeric (time) attributes, keyed by attribute name.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Item {
    pub id: String,
    pub domain: String,
    pub text: BTreeMap<String, Vec<String>>,
    pub numbers: BTreeMap<String, f64>,
}

impl Item {
    pub fn new(id: &str, domain: &str) -> Self {
        Item { id: id.into(), domain: domain.into(), ..Item::default() }
    }
    pub fn text(mut self, key: &str, value: &str) -> Self {
        self.text.entry(key.into()).or_default().push(value.into());
        self
    }
    pub fn number(mut self, key: &str, value: f64) -> Self {
        self.numbers.insert(key.into(), value);
        self
    }
}

#[derive(Clone, Debug, PartialEq)]
enum Value {
    Text { pattern: String, fold_case: bool, fold_diacritics: bool, word: bool },
    Number(f64),
    /// Seconds relative to now.
    Now(f64),
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Op {
    Eq,
    Ne,
    Lt,
    Gt,
    Le,
    Ge,
}

#[derive(Clone, Debug, PartialEq)]
enum Expr {
    Cmp(String, Op, Value),
    And(Box<Expr>, Box<Expr>),
    Or(Box<Expr>, Box<Expr>),
}

struct Parser<'a> {
    s: &'a [char],
    i: usize,
}

impl<'a> Parser<'a> {
    fn ws(&mut self) {
        while self.i < self.s.len() && self.s[self.i].is_whitespace() {
            self.i += 1;
        }
    }
    fn eat(&mut self, t: &str) -> bool {
        self.ws();
        let t: Vec<char> = t.chars().collect();
        if self.s[self.i..].starts_with(&t) {
            self.i += t.len();
            true
        } else {
            false
        }
    }
    fn or(&mut self) -> Result<Expr, Error> {
        let mut l = self.and()?;
        while self.eat("||") {
            l = Expr::Or(Box::new(l), Box::new(self.and()?));
        }
        Ok(l)
    }
    fn and(&mut self) -> Result<Expr, Error> {
        let mut l = self.atom()?;
        while self.eat("&&") {
            l = Expr::And(Box::new(l), Box::new(self.atom()?));
        }
        Ok(l)
    }
    fn atom(&mut self) -> Result<Expr, Error> {
        if self.eat("(") {
            let e = self.or()?;
            return if self.eat(")") { Ok(e) } else { Err(Error::InvalidQuery) };
        }
        self.ws();
        let start = self.i;
        while self.i < self.s.len() && (self.s[self.i].is_alphanumeric() || self.s[self.i] == '_' || self.s[self.i] == '.') {
            self.i += 1;
        }
        if start == self.i {
            return Err(Error::InvalidQuery);
        }
        let key: String = self.s[start..self.i].iter().collect();
        let ops = [("==", Op::Eq), ("!=", Op::Ne), ("<=", Op::Le), (">=", Op::Ge), ("<", Op::Lt), (">", Op::Gt)];
        let mut found = None;
        for (t, o) in ops {
            if self.eat(t) {
                found = Some(o);
                break;
            }
        }
        let op = found.ok_or(Error::InvalidQuery)?;
        let v = self.value()?;
        Ok(Expr::Cmp(key, op, v))
    }
    fn value(&mut self) -> Result<Value, Error> {
        self.ws();
        if self.eat("\"") {
            let mut p = String::new();
            loop {
                match self.s.get(self.i) {
                    None => return Err(Error::InvalidQuery),
                    Some('"') => {
                        self.i += 1;
                        break;
                    }
                    Some('\\') if self.i + 1 < self.s.len() => {
                        p.push(self.s[self.i + 1]);
                        self.i += 2;
                    }
                    Some(c) => {
                        p.push(*c);
                        self.i += 1;
                    }
                }
            }
            let (mut c, mut d, mut w) = (false, false, false);
            while let Some(m) = self.s.get(self.i) {
                match m {
                    'c' => c = true,
                    'd' => d = true,
                    'w' => w = true,
                    _ => break,
                }
                self.i += 1;
            }
            return Ok(Value::Text { pattern: p, fold_case: c, fold_diacritics: d, word: w });
        }
        if self.eat("$time.now") {
            if self.eat("(") {
                let n = self.number()?;
                if !self.eat(")") {
                    return Err(Error::InvalidQuery);
                }
                return Ok(Value::Now(n));
            }
            return Ok(Value::Now(0.0));
        }
        Ok(Value::Number(self.number()?))
    }
    fn number(&mut self) -> Result<f64, Error> {
        self.ws();
        let start = self.i;
        while self.i < self.s.len() && (self.s[self.i].is_ascii_digit() || matches!(self.s[self.i], '-' | '+' | '.')) {
            self.i += 1;
        }
        self.s[start..self.i].iter().collect::<String>().parse().map_err(|_| Error::InvalidQuery)
    }
}

fn parse(q: &str) -> Result<Expr, Error> {
    let chars: Vec<char> = q.chars().collect();
    let mut p = Parser { s: &chars, i: 0 };
    let e = p.or()?;
    p.ws();
    if p.i == chars.len() {
        Ok(e)
    } else {
        Err(Error::InvalidQuery)
    }
}

fn strip(c: char) -> char {
    match c {
        'á' | 'à' | 'ä' | 'â' | 'å' => 'a',
        'é' | 'è' | 'ë' | 'ê' => 'e',
        'í' | 'ì' | 'ï' | 'î' => 'i',
        'ó' | 'ò' | 'ö' | 'ô' => 'o',
        'ú' | 'ù' | 'ü' | 'û' => 'u',
        'ý' | 'ÿ' => 'y',
        'ç' => 'c',
        'ñ' => 'n',
        other => other,
    }
}

fn norm(s: &str, case: bool, diac: bool) -> Vec<char> {
    let lowered: Vec<char> = if case { s.chars().flat_map(|c| c.to_lowercase()).collect() } else { s.chars().collect() };
    lowered.into_iter().map(|c| if diac { strip(c) } else { c }).collect()
}

fn glob(p: &[char], t: &[char]) -> bool {
    match p.split_first() {
        None => t.is_empty(),
        Some(('*', rest)) => (0..=t.len()).any(|i| glob(rest, &t[i..])),
        Some((c, rest)) => t.first() == Some(c) && glob(rest, &t[1..]),
    }
}

fn text_matches(pattern: &str, c: bool, d: bool, word: bool, value: &str) -> bool {
    let p = norm(pattern, c, d);
    let v = norm(value, c, d);
    if word {
        v.split(|ch| !ch.is_alphanumeric()).any(|w| glob(&p, w))
    } else {
        glob(&p, &v)
    }
}

fn cmp(have: Option<&f64>, op: Op, want: f64) -> bool {
    match have {
        None => op == Op::Ne,
        Some(h) => match op {
            Op::Eq => *h == want,
            Op::Ne => *h != want,
            Op::Lt => *h < want,
            Op::Gt => *h > want,
            Op::Le => *h <= want,
            Op::Ge => *h >= want,
        },
    }
}

fn eval(e: &Expr, item: &Item, now: f64) -> bool {
    match e {
        Expr::And(a, b) => eval(a, item, now) && eval(b, item, now),
        Expr::Or(a, b) => eval(a, item, now) || eval(b, item, now),
        Expr::Cmp(key, op, v) => match v {
            Value::Text { pattern, fold_case, fold_diacritics, word } => {
                let hit = item
                    .text
                    .get(key)
                    .map_or(false, |vs| vs.iter().any(|x| text_matches(pattern, *fold_case, *fold_diacritics, *word, x)));
                match op {
                    Op::Eq => hit,
                    Op::Ne => !hit,
                    _ => false,
                }
            }
            Value::Number(n) => cmp(item.numbers.get(key), *op, *n),
            Value::Now(off) => cmp(item.numbers.get(key), *op, now + off),
        },
    }
}

/// `CSSearchQueryContext`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct QueryContext {
    pub fetch_attributes: Vec<String>,
    pub filter_queries: Vec<String>,
    pub source_options: u32,
}

/// `CSSearchQuery(queryString:queryContext:)`. Results arrive in batches of [`SearchQuery::BATCH`].
pub struct SearchQuery {
    query_string: String,
    context: QueryContext,
    cancelled: Rc<Cell<bool>>,
    found_count: usize,
}

/// Cancels a running query from inside a handler.
#[derive(Clone)]
pub struct CancelHandle(Rc<Cell<bool>>);

impl CancelHandle {
    pub fn cancel(&self) {
        self.0.set(true);
    }
}

impl SearchQuery {
    pub const BATCH: usize = 8;

    pub fn new(query_string: &str, context: QueryContext) -> Self {
        SearchQuery { query_string: query_string.into(), context, cancelled: Rc::new(Cell::new(false)), found_count: 0 }
    }
    pub fn cancel_handle(&self) -> CancelHandle {
        CancelHandle(self.cancelled.clone())
    }
    /// `cancel()`.
    pub fn cancel(&self) {
        self.cancelled.set(true);
    }
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.get()
    }
    /// `foundItemCount`.
    pub fn found_item_count(&self) -> usize {
        self.found_count
    }
    pub fn context(&self) -> &QueryContext {
        &self.context
    }

    /// `start()`: calls `found_items` with each batch of matches, then `completion` once (`None` on success).
    pub fn start(&mut self, items: &[Item], now: f64, mut found_items: impl FnMut(&[Item]), completion: impl FnOnce(Option<Error>)) {
        let q = match parse(&self.query_string) {
            Ok(q) => q,
            Err(e) => return completion(Some(e)),
        };
        let mut filters = Vec::new();
        for f in &self.context.filter_queries {
            match parse(f) {
                Ok(p) => filters.push(p),
                Err(e) => return completion(Some(e)),
            }
        }
        let mut batch: Vec<Item> = Vec::new();
        for it in items {
            if self.cancelled.get() {
                return completion(Some(Error::Cancelled));
            }
            if eval(&q, it, now) && filters.iter().all(|f| eval(f, it, now)) {
                batch.push(it.clone());
                self.found_count += 1;
                if batch.len() == Self::BATCH {
                    found_items(&batch);
                    batch.clear();
                }
            }
        }
        if self.cancelled.get() {
            return completion(Some(Error::Cancelled));
        }
        if !batch.is_empty() {
            found_items(&batch);
        }
        completion(None)
    }
}

/// `CSSuggestion`: a completion of what the user typed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Suggestion {
    pub localized_attribute_set_title: String,
}

/// `CSUserQueryContext`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UserQueryContext {
    pub max_result_count: usize,
    pub max_suggestion_count: usize,
}

impl Default for UserQueryContext {
    fn default() -> Self {
        UserQueryContext { max_result_count: 20, max_suggestion_count: 5 }
    }
}

/// `CSUserQuery`: free text over titles, keywords and descriptions. Every typed word must start a word in some
/// field; a word scores 3 in the title, 2 in keywords, 1 in the description (best field), summed over words, ties
/// keep index order. Suggestions are titles that continue what was typed.
pub struct UserQuery {
    pub user_query_string: String,
    pub context: UserQueryContext,
}

impl UserQuery {
    pub fn new(user_query_string: &str, context: UserQueryContext) -> Self {
        UserQuery { user_query_string: user_query_string.into(), context }
    }

    fn words(s: &str) -> Vec<Vec<char>> {
        s.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).map(|w| norm(w, true, true)).collect()
    }

    fn weight(word: &[char], item: &Item) -> u32 {
        [("title", 3), ("keywords", 2), ("description", 1)]
            .iter()
            .filter(|(k, _)| item.text.get(*k).into_iter().flatten().flat_map(|s| Self::words(s)).any(|x| x.starts_with(word)))
            .map(|(_, w)| *w)
            .max()
            .unwrap_or(0)
    }

    /// Ranked items and suggestions drawn from `items`.
    pub fn run(&self, items: &[Item]) -> (Vec<Item>, Vec<Suggestion>) {
        let words = Self::words(&self.user_query_string);
        if words.is_empty() {
            return (vec![], vec![]);
        }
        let mut scored: Vec<(u32, usize)> = Vec::new();
        for (i, it) in items.iter().enumerate() {
            let ws: Vec<u32> = words.iter().map(|w| Self::weight(w, it)).collect();
            if ws.iter().all(|w| *w > 0) {
                scored.push((ws.iter().sum(), i));
            }
        }
        scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        let results = scored.iter().take(self.context.max_result_count).map(|(_, i)| items[*i].clone()).collect();
        let typed = norm(self.user_query_string.trim(), true, true);
        let mut sugg: Vec<Suggestion> = Vec::new();
        for it in items {
            for t in it.text.get("title").into_iter().flatten() {
                let continues = norm(t, true, true).starts_with(&typed) && t.chars().count() > typed.len();
                if continues && !sugg.iter().any(|s| &s.localized_attribute_set_title == t) {
                    sugg.push(Suggestion { localized_attribute_set_title: t.clone() });
                }
            }
        }
        sugg.truncate(self.context.max_suggestion_count);
        (results, sugg)
    }
}

/// `CSSearchableIndexDelegate`. Each callback is acknowledged by the delegate returning.
pub trait IndexDelegate {
    fn reindex_all_searchable_items(&mut self, index: &mut SearchableIndex);
    fn reindex_searchable_items(&mut self, index: &mut SearchableIndex, identifiers: &[String]);
}

/// `CSSearchableIndex(name:protectionClass:)` with acknowledged writes, batches and client state.
pub struct SearchableIndex {
    pub name: String,
    pub protection_class: Option<String>,
    capacity: usize,
    items: Vec<Item>,
    connected: bool,
    pending: Option<Vec<Item>>,
    client_state: Vec<u8>,
    evicted: Vec<String>,
}

impl SearchableIndex {
    pub fn new(name: &str, protection_class: Option<&str>, capacity: usize) -> Self {
        SearchableIndex {
            name: name.into(),
            protection_class: protection_class.map(String::from),
            capacity,
            items: vec![],
            connected: true,
            pending: None,
            client_state: vec![],
            evicted: vec![],
        }
    }
    pub fn items(&self) -> &[Item] {
        &self.items
    }
    pub fn set_connected(&mut self, connected: bool) {
        self.connected = connected;
    }
    /// Identifiers dropped to stay within the capacity (oldest first); the call drains them.
    pub fn take_evicted(&mut self) -> Vec<String> {
        std::mem::take(&mut self.evicted)
    }

    fn commit(&mut self, new: Vec<Item>) {
        for it in new {
            self.items.retain(|x| x.id != it.id);
            self.items.push(it);
        }
        while self.items.len() > self.capacity {
            self.evicted.push(self.items.remove(0).id);
        }
    }

    /// `indexSearchableItems` with completion: inside a batch the items wait for `end_batch`.
    pub fn index_searchable_items(&mut self, items: Vec<Item>) -> Result<(), Error> {
        if !self.connected {
            return Err(Error::RemoteConnection);
        }
        match &mut self.pending {
            Some(p) => p.extend(items),
            None => self.commit(items),
        }
        Ok(())
    }
    pub fn delete_searchable_items(&mut self, ids: &[String]) -> Result<(), Error> {
        if !self.connected {
            return Err(Error::RemoteConnection);
        }
        self.items.retain(|x| !ids.contains(&x.id));
        Ok(())
    }
    pub fn begin_batch(&mut self) -> Result<(), Error> {
        if self.pending.is_some() {
            return Err(Error::BatchState);
        }
        self.pending = Some(vec![]);
        Ok(())
    }
    /// `endBatch(withClientState:)`: commits the batch and records the state. When not connected the batch stays
    /// pending and the state is unchanged, so the caller can retry.
    pub fn end_batch(&mut self, client_state: &[u8]) -> Result<(), Error> {
        if self.pending.is_none() {
            return Err(Error::BatchState);
        }
        if !self.connected {
            return Err(Error::RemoteConnection);
        }
        let p = self.pending.take().unwrap_or_default();
        self.commit(p);
        self.client_state = client_state.to_vec();
        Ok(())
    }
    pub fn fetch_last_client_state(&self) -> Result<Vec<u8>, Error> {
        if self.connected {
            Ok(self.client_state.clone())
        } else {
            Err(Error::RemoteConnection)
        }
    }

    /// The index was lost or found corrupt: it empties, forgets its state and asks the delegate to rebuild.
    pub fn index_lost(&mut self, delegate: &mut dyn IndexDelegate) {
        self.items.clear();
        self.client_state.clear();
        self.pending = None;
        delegate.reindex_all_searchable_items(self);
    }
    /// Some items are unreadable: they are removed and the delegate re-indexes those identifiers.
    pub fn items_corrupt(&mut self, ids: &[String], delegate: &mut dyn IndexDelegate) {
        self.items.retain(|x| !ids.contains(&x.id));
        delegate.reindex_searchable_items(self, ids);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn items() -> Vec<Item> {
        vec![
            Item::new("1", "mail").text("title", "Milk Run").text("keywords", "shop").number("date", 100.0),
            Item::new("2", "mail").text("title", "Mjólk").number("date", 5000.0),
            Item::new("3", "notes").text("title", "Bread").text("description", "buy milk too"),
        ]
    }

    fn ids(q: &str, now: f64) -> Result<Vec<String>, Error> {
        let mut out = vec![];
        let mut res = None;
        SearchQuery::new(q, QueryContext::default()).start(&items(), now, |b| out.extend(b.iter().map(|i| i.id.clone())), |e| res = Some(e));
        match res.unwrap() {
            Some(e) => Err(e),
            None => Ok(out),
        }
    }

    #[test]
    fn query_language_table() {
        let cases: &[(&str, &[&str])] = &[
            (r#"title == "Bread""#, &["3"]),
            (r#"title == "bread""#, &[]),
            (r#"title == "bread"c"#, &["3"]),
            (r#"title == "Mil*""#, &["1"]),
            (r#"title == "mjolk"cd"#, &["2"]),
            (r#"title == "run"cw"#, &["1"]),
            (r#"title != "Bread""#, &["1", "2"]),
            (r#"title == "Bread" || title == "Mjólk""#, &["2", "3"]),
            (r#"title == "Milk*" && keywords == "shop""#, &["1"]),
            (r#"title == "Bread" || title == "Mjólk" && keywords == "x""#, &["3"]),
            (r#"(title == "Bread" || title == "Mjólk") && description == "*milk*""#, &["3"]),
            (r#"date < 1000"#, &["1"]),
            (r#"date >= $time.now"#, &["2"]),
            (r#"date > $time.now(-5000)"#, &["2"]),
        ];
        for (q, want) in cases {
            assert_eq!(ids(q, 1000.0).unwrap(), *want, "{q}");
        }
    }

    #[test]
    fn bad_queries_complete_with_an_error() {
        for q in ["", "title ==", r#"title == "x"#, "title = 1", r#"(title == "x""#, r#"title == "x" junk"#] {
            assert_eq!(ids(q, 0), Err(Error::InvalidQuery), "{q}");
        }
    }

    #[test]
    fn found_items_arrive_in_batches_and_filters_narrow() {
        let many: Vec<Item> = (0..20).map(|i| Item::new(&i.to_string(), "d").text("title", "a")).collect();
        let mut sizes = vec![];
        let mut q = SearchQuery::new(r#"title == "a""#, QueryContext::default());
        q.start(&many, 0.0, |b| sizes.push(b.len()), |e| assert!(e.is_none()));
        assert_eq!(sizes, [8, 8, 4]);
        assert_eq!(q.found_item_count(), 20);
        let ctx = QueryContext { filter_queries: vec![r#"title == "x""#.into()], ..Default::default() };
        let mut n = 0;
        SearchQuery::new(r#"title == "a""#, ctx).start(&many, 0.0, |b| n += b.len(), |_| {});
        assert_eq!(n, 0);
    }

    #[test]
    fn cancel_stops_after_the_current_batch_with_cancelled() {
        let many: Vec<Item> = (0..30).map(|i| Item::new(&i.to_string(), "d").text("title", "a")).collect();
        let mut q = SearchQuery::new(r#"title == "a""#, QueryContext::default());
        let h = q.cancel_handle();
        let (mut got, mut done) = (0, None);
        q.start(
            &many,
            0.0,
            |b| {
                got += b.len();
                h.cancel();
            },
            |e| done = Some(e),
        );
        assert_eq!(got, 8);
        assert_eq!(done, Some(Some(Error::Cancelled)));
        assert!(q.is_cancelled());
    }

    #[test]
    fn user_query_ranks_title_first_and_suggests_titles() {
        let (res, _) = UserQuery::new("milk", UserQueryContext::default()).run(&items());
        assert_eq!(res.iter().map(|i| i.id.as_str()).collect::<Vec<_>>(), ["1", "3"]);
        let (_, s) = UserQuery::new("br", UserQueryContext::default()).run(&items());
        assert_eq!(s.iter().map(|x| x.localized_attribute_set_title.as_str()).collect::<Vec<_>>(), ["Bread"]);
        let ctx = UserQueryContext { max_result_count: 1, max_suggestion_count: 0 };
        assert_eq!(UserQuery::new("milk", ctx).run(&items()).0.len(), 1);
        assert!(UserQuery::new("  ", UserQueryContext::default()).run(&items()).0.is_empty());
    }

    #[derive(Default)]
    struct Del {
        all: u32,
        some: Vec<String>,
    }

    impl IndexDelegate for Del {
        fn reindex_all_searchable_items(&mut self, index: &mut SearchableIndex) {
            self.all += 1;
            index.index_searchable_items(vec![Item::new("r", "d")]).unwrap();
        }
        fn reindex_searchable_items(&mut self, _: &mut SearchableIndex, ids: &[String]) {
            self.some.extend_from_slice(ids);
        }
    }

    #[test]
    fn lost_index_asks_the_delegate_to_rebuild() {
        let mut ix = SearchableIndex::new("n", Some("complete"), 10);
        ix.index_searchable_items(items()).unwrap();
        let mut d = Del::default();
        ix.index_lost(&mut d);
        assert_eq!(d.all, 1);
        assert_eq!(ix.items().len(), 1);
        ix.items_corrupt(&["r".into()], &mut d);
        assert_eq!(d.some, ["r"]);
        assert!(ix.items().is_empty());
    }

    #[test]
    fn batches_commit_with_client_state_and_survive_a_dropped_connection() {
        let mut ix = SearchableIndex::new("n", None, 10);
        assert_eq!(ix.end_batch(b"x"), Err(Error::BatchState));
        ix.begin_batch().unwrap();
        assert_eq!(ix.begin_batch(), Err(Error::BatchState));
        ix.index_searchable_items(items()).unwrap();
        assert!(ix.items().is_empty());
        ix.set_connected(false);
        assert_eq!(ix.end_batch(b"v1"), Err(Error::RemoteConnection));
        assert_eq!(ix.fetch_last_client_state(), Err(Error::RemoteConnection));
        ix.set_connected(true);
        ix.end_batch(b"v1").unwrap();
        assert_eq!(ix.items().len(), 3);
        assert_eq!(ix.fetch_last_client_state().unwrap(), b"v1");
    }

    #[test]
    fn acknowledged_writes_fail_offline_and_evict_the_oldest() {
        let mut ix = SearchableIndex::new("n", None, 2);
        ix.set_connected(false);
        assert_eq!(ix.index_searchable_items(items()), Err(Error::RemoteConnection));
        assert_eq!(ix.delete_searchable_items(&["1".into()]), Err(Error::RemoteConnection));
        assert_eq!(Error::RemoteConnection.code(), 3);
        ix.set_connected(true);
        ix.index_searchable_items(items()).unwrap();
        assert_eq!(ix.take_evicted(), ["1"]);
        assert_eq!(ix.items().iter().map(|i| i.id.as_str()).collect::<Vec<_>>(), ["2", "3"]);
        ix.delete_searchable_items(&["2".into()]).unwrap();
        assert_eq!(ix.items().len(), 1);
    }
}
