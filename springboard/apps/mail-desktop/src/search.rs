//! Search over the messages of a mailbox. A query is words separated by spaces; a quoted run is one phrase; a word
//! can be limited to a field (`from:`, `to:`, `subject:`, `body:`). Every term must match (case-insensitive).

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Field {
    From,
    To,
    Subject,
    Body,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Term {
    pub field: Option<Field>,
    /// Lowercased text to find.
    pub text: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Query {
    pub terms: Vec<Term>,
}

/// The searchable text of one message.
#[derive(Clone, Debug, Default)]
pub struct Doc<'a> {
    pub from: &'a str,
    pub to: &'a str,
    pub subject: &'a str,
    pub body: &'a str,
}

fn field_of(word: &str) -> Option<(Field, &str)> {
    let (name, rest) = word.split_once(':')?;
    let field = match name.to_ascii_lowercase().as_str() {
        "from" => Field::From,
        "to" => Field::To,
        "subject" => Field::Subject,
        "body" => Field::Body,
        _ => return None,
    };
    Some((field, rest))
}

/// Splits a query into words, keeping quoted runs together. Quotes are removed.
fn words(input: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut in_quote = false;
    for c in input.chars() {
        match c {
            '"' => in_quote = !in_quote,
            c if c.is_whitespace() && !in_quote => {
                if !current.is_empty() {
                    out.push(std::mem::take(&mut current));
                }
            }
            _ => current.push(c),
        }
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

impl Query {
    pub fn parse(input: &str) -> Query {
        let terms = words(input)
            .into_iter()
            .filter_map(|w| {
                let parsed = field_of(&w).map(|(f, rest)| (f, rest.to_string()));
                let (field, text) = match parsed {
                    Some((f, rest)) => (Some(f), rest),
                    None => (None, w),
                };
                let text = text.trim().to_lowercase();
                (!text.is_empty()).then_some(Term { field, text })
            })
            .collect();
        Query { terms }
    }

    /// The server-side form of the query: IMAP search keys, every term ANDed. A term with no field matches any of the
    /// four fields. None for an empty query. Text that is not ASCII is sent under `CHARSET UTF-8`.
    pub fn imap_criteria(&self) -> Option<String> {
        if self.terms.is_empty() {
            return None;
        }
        let charset = if self.terms.iter().all(|t| t.text.is_ascii()) { "" } else { "CHARSET UTF-8 " };
        let keys: Vec<String> = self.terms.iter().map(term_criteria).collect();
        Some(format!("{charset}{}", keys.join(" ")))
    }

    /// An empty query matches every message.
    pub fn matches(&self, doc: &Doc) -> bool {
        self.terms.iter().all(|term| {
            let hit = |s: &str| s.to_lowercase().contains(&term.text);
            match term.field {
                Some(Field::From) => hit(doc.from),
                Some(Field::To) => hit(doc.to),
                Some(Field::Subject) => hit(doc.subject),
                Some(Field::Body) => hit(doc.body),
                None => hit(doc.from) || hit(doc.to) || hit(doc.subject) || hit(doc.body),
            }
        })
    }
}

/// One term as an IMAP search key. The text is quoted, so a space or a quote in it cannot add keys.
fn term_criteria(term: &Term) -> String {
    let text = crate::imap::quote(&term.text);
    match term.field {
        Some(Field::From) => format!("FROM {text}"),
        Some(Field::To) => format!("TO {text}"),
        Some(Field::Subject) => format!("SUBJECT {text}"),
        Some(Field::Body) => format!("BODY {text}"),
        None => any_of(&[format!("FROM {text}"), format!("TO {text}"), format!("SUBJECT {text}"), format!("BODY {text}")]),
    }
}

/// IMAP's `OR` takes exactly two keys, so n keys nest to the right: `OR a OR b c` matches a, b or c.
fn any_of(keys: &[String]) -> String {
    match keys {
        [] => String::new(),
        [one] => one.clone(),
        [first, rest @ ..] => format!("OR {first} {}", any_of(rest)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_server_criteria_follow_the_fields() {
        assert_eq!(Query::parse("").imap_criteria(), None);
        assert_eq!(Query::parse("from:ana subject:\"fundur á\"").imap_criteria().as_deref(), Some("CHARSET UTF-8 FROM \"ana\" SUBJECT \"fundur á\""));
        assert_eq!(Query::parse("from:ana subject:fundur").imap_criteria().as_deref(), Some("FROM \"ana\" SUBJECT \"fundur\""));
        assert_eq!(
            Query::parse("plain").imap_criteria().as_deref(),
            Some("OR FROM \"plain\" OR TO \"plain\" OR SUBJECT \"plain\" BODY \"plain\"")
        );
        assert_eq!(Query::parse("body:x").imap_criteria().as_deref(), Some("BODY \"x\""));
    }

    fn doc() -> Doc<'static> {
        Doc { from: "Ana <ana@example.com>", to: "bo@example.com", subject: "Fundur á morgun", body: "Sjáumst kl. 10" }
    }

    #[test]
    fn parses_fields_and_phrases() {
        let q = Query::parse("from:ANA \"fundur á\" plain");
        assert_eq!(q.terms.len(), 3);
        assert_eq!(q.terms[0], Term { field: Some(Field::From), text: "ana".into() });
        assert_eq!(q.terms[1], Term { field: None, text: "fundur á".into() });
    }

    #[test]
    fn every_term_must_match() {
        assert!(Query::parse("").matches(&doc()));
        assert!(Query::parse("FUNDUR").matches(&doc()));
        assert!(Query::parse("subject:morgun from:ana").matches(&doc()));
        assert!(!Query::parse("subject:ana").matches(&doc()));
        assert!(!Query::parse("fundur missing").matches(&doc()));
        assert!(Query::parse("body:10").matches(&doc()));
    }
}
