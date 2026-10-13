//! IMAP (RFC 3501) on the wire: a parser for the server lines this app reads, and a builder for the commands it sends.
//! Responses with literals (`{n}` followed by data) are not parsed here; the connection layer reads those itself.

use crate::flags::Flags;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Ok,
    No,
    Bad,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reply {
    /// `+ ...`: the server wants more data (for AUTHENTICATE or literals).
    Continuation(String),
    /// `* ...`: untagged data, without the leading `* `.
    Untagged(String),
    /// `A001 OK ...`: the final answer to a command.
    Tagged { tag: String, status: Status, text: String },
    /// A line that is none of the above.
    Unknown(String),
}

/// Classifies one server line (CRLF already removed).
pub fn parse_line(line: &str) -> Reply {
    if let Some(rest) = line.strip_prefix("+") {
        return Reply::Continuation(rest.trim_start().to_string());
    }
    if let Some(rest) = line.strip_prefix("* ") {
        return Reply::Untagged(rest.to_string());
    }
    let Some((tag, rest)) = line.split_once(' ') else { return Reply::Unknown(line.to_string()) };
    let (word, text) = rest.split_once(' ').unwrap_or((rest, ""));
    let status = match word {
        "OK" => Status::Ok,
        "NO" => Status::No,
        "BAD" => Status::Bad,
        _ => return Reply::Unknown(line.to_string()),
    };
    Reply::Tagged { tag: tag.to_string(), status, text: text.to_string() }
}

/// A parsed IMAP list value: an atom, a quoted string, `NIL`, or a parenthesised list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Value {
    Atom(String),
    Quoted(String),
    Nil,
    List(Vec<Value>),
}

impl Value {
    /// The text of an atom or quoted string.
    pub fn text(&self) -> Option<&str> {
        match self {
            Value::Atom(s) | Value::Quoted(s) => Some(s),
            _ => None,
        }
    }
}

/// Parses one value from the start of `s`, returning it and the unparsed rest.
pub fn parse_value(s: &str) -> Option<(Value, &str)> {
    let s = s.trim_start();
    let first = s.chars().next()?;
    match first {
        '(' => {
            let mut rest = &s[1..];
            let mut items = Vec::new();
            loop {
                rest = rest.trim_start();
                if let Some(after) = rest.strip_prefix(')') {
                    return Some((Value::List(items), after));
                }
                let (item, after) = parse_value(rest)?;
                items.push(item);
                rest = after;
            }
        }
        '"' => {
            let mut out = String::new();
            let mut chars = s[1..].char_indices();
            while let Some((i, c)) = chars.next() {
                match c {
                    '\\' => out.push(chars.next()?.1),
                    '"' => return Some((Value::Quoted(out), &s[1 + i + 1..])),
                    _ => out.push(c),
                }
            }
            None
        }
        _ => {
            let end = s.find(|c: char| c.is_whitespace() || c == '(' || c == ')').unwrap_or(s.len());
            let (atom, rest) = s.split_at(end);
            if atom.is_empty() {
                return None;
            }
            let value = if atom.eq_ignore_ascii_case("NIL") { Value::Nil } else { Value::Atom(atom.to_string()) };
            Some((value, rest))
        }
    }
}

/// A mailbox from a `LIST` response.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mailbox {
    pub name: String,
    pub delimiter: Option<char>,
    pub attributes: Vec<String>,
}

impl Mailbox {
    /// `\Noselect` mailboxes only hold children and cannot be opened.
    pub fn selectable(&self) -> bool {
        !self.attributes.iter().any(|a| a.eq_ignore_ascii_case("\\Noselect"))
    }
}

/// Parses the untagged text of a `LIST` response: `LIST (\HasNoChildren) "/" "INBOX"`.
pub fn parse_list(untagged: &str) -> Option<Mailbox> {
    let rest = untagged.strip_prefix("LIST ").or_else(|| untagged.strip_prefix("LSUB "))?;
    let (attrs, rest) = parse_value(rest)?;
    let (delim, rest) = parse_value(rest)?;
    let (name, _) = parse_value(rest)?;
    let attributes = match attrs {
        Value::List(items) => items.iter().filter_map(|v| v.text().map(str::to_string)).collect(),
        _ => return None,
    };
    let delimiter = delim.text().and_then(|d| d.chars().next());
    Some(Mailbox { name: name.text()?.to_string(), delimiter, attributes })
}

/// A `FETCH` answer with the fields this app asks for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fetch {
    pub seq: u32,
    pub uid: Option<u32>,
    pub flags: Option<Flags>,
}

/// Parses untagged `n FETCH (UID u FLAGS (...))`. Other fields in the list are skipped.
pub fn parse_fetch(untagged: &str) -> Option<Fetch> {
    let (seq, rest) = untagged.split_once(' ')?;
    let seq: u32 = seq.parse().ok()?;
    let rest = rest.strip_prefix("FETCH ")?;
    let (list, _) = parse_value(rest)?;
    let Value::List(items) = list else { return None };
    let mut fetch = Fetch { seq, uid: None, flags: None };
    let mut i = 0;
    while i + 1 < items.len() {
        let key = items[i].text().unwrap_or("");
        if key.eq_ignore_ascii_case("UID") {
            fetch.uid = items[i + 1].text().and_then(|t| t.parse().ok());
        } else if key.eq_ignore_ascii_case("FLAGS") {
            if let Value::List(names) = &items[i + 1] {
                let names: Vec<&str> = names.iter().filter_map(Value::text).collect();
                fetch.flags = Some(Flags::from_names(names));
            }
        }
        i += 2;
    }
    Some(fetch)
}

/// Parses untagged `SEARCH n n n` into message numbers (UIDs when the search was a `UID SEARCH`).
pub fn parse_search(untagged: &str) -> Option<Vec<u32>> {
    let rest = untagged.strip_prefix("SEARCH")?;
    rest.split_whitespace().map(|n| n.parse().ok()).collect()
}

/// Quotes an IMAP string argument. Line breaks are dropped so a value cannot inject a second command.
pub fn quote(text: &str) -> String {
    let mut out = String::from("\"");
    for c in text.chars() {
        match c {
            '\r' | '\n' => {}
            '"' | '\\' => {
                out.push('\\');
                out.push(c);
            }
            _ => out.push(c),
        }
    }
    out.push('"');
    out
}

/// The tag for the n-th command of a session: `A001`, `A002`, ...
pub fn tag(n: u32) -> String {
    format!("A{n:03}")
}

/// A full command line, CRLF terminated.
pub fn command(tag: &str, body: &str) -> String {
    format!("{tag} {body}\r\n")
}

pub fn login(user: &str, password: &str) -> String {
    format!("LOGIN {} {}", quote(user), quote(password))
}

pub fn list_all() -> String {
    "LIST \"\" \"*\"".to_string()
}

pub fn select(mailbox: &str) -> String {
    format!("SELECT {}", quote(mailbox))
}

/// Flags and UIDs for every message in the selected mailbox.
pub fn fetch_flags_all() -> String {
    "UID FETCH 1:* (UID FLAGS)".to_string()
}

pub fn uid_search_all() -> String {
    "UID SEARCH ALL".to_string()
}

/// Adds (`+`), removes (`-`) or sets (no sign) flags on one message, silently.
pub fn store_flags(uid: u32, op: char, flags: Flags) -> String {
    format!("UID STORE {uid} {op}FLAGS.SILENT {}", flags.to_imap_list())
}

pub fn logout() -> String {
    "LOGOUT".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flags::Flag;

    #[test]
    fn classifies_server_lines() {
        assert_eq!(parse_line("+ go ahead"), Reply::Continuation("go ahead".into()));
        assert_eq!(parse_line("* OK ready"), Reply::Untagged("OK ready".into()));
        assert_eq!(
            parse_line("A002 NO [AUTHENTICATIONFAILED] bad"),
            Reply::Tagged { tag: "A002".into(), status: Status::No, text: "[AUTHENTICATIONFAILED] bad".into() }
        );
        assert_eq!(parse_line("A003 OK"), Reply::Tagged { tag: "A003".into(), status: Status::Ok, text: String::new() });
        assert_eq!(parse_line("garbage"), Reply::Unknown("garbage".into()));
    }

    #[test]
    fn parses_list_responses() {
        let m = parse_list("LIST (\\HasNoChildren) \"/\" \"INBOX\"").unwrap();
        assert_eq!(m.name, "INBOX");
        assert_eq!(m.delimiter, Some('/'));
        assert!(m.selectable());
        let folder = parse_list("LIST (\\Noselect \\HasChildren) NIL \"Archive \\\"old\\\"\"").unwrap();
        assert_eq!(folder.delimiter, None);
        assert_eq!(folder.name, "Archive \"old\"");
        assert!(!folder.selectable());
    }

    #[test]
    fn parses_fetch_and_search() {
        let f = parse_fetch("3 FETCH (UID 42 FLAGS (\\Seen \\Flagged) RFC822.SIZE 900)").unwrap();
        assert_eq!(f.seq, 3);
        assert_eq!(f.uid, Some(42));
        let flags = f.flags.unwrap();
        assert!(flags.contains(Flag::Seen) && flags.contains(Flag::Flagged));
        assert_eq!(parse_search("SEARCH 4 9 12"), Some(vec![4, 9, 12]));
        assert_eq!(parse_search("SEARCH"), Some(vec![]));
        assert_eq!(parse_search("SEARCH x"), None);
    }

    #[test]
    fn quotes_and_commands() {
        assert_eq!(quote("a\"b\\c"), "\"a\\\"b\\\\c\"");
        assert_eq!(quote("x\r\nA999 LOGOUT"), "\"xA999 LOGOUT\"");
        assert_eq!(login("me", "pa ss"), "LOGIN \"me\" \"pa ss\"");
        assert_eq!(command(&tag(7), &select("INBOX")), "A007 SELECT \"INBOX\"\r\n");
        assert_eq!(store_flags(5, '+', Flags::empty().with(Flag::Seen, true)), "UID STORE 5 +FLAGS.SILENT (\\Seen)");
    }
}
