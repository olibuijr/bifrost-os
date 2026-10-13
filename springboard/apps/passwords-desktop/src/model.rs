//! Passwords model: the logins in the shared passwords group, search, generated passwords and the CSV import and
//! export. Pure logic (no window, no keychain), so it is tested alone; `main.rs` does the calls and the drawing.
//!
//! Secrets only leave the model as the caller asks for them: a listing carries sites, accounts and times, never a
//! password. Nothing here logs or formats a password for display except the generated one the user asked for.

use bifrost_app::password_gen::{self, Rules};

/// Length of a generated password.
pub const GENERATED_LENGTH: usize = 20;

/// One login as the list shows it. The password is not held here; it is read from the keychain when asked for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Login {
    pub site: String,
    pub account: String,
}

/// Sorts by site, then account, ignoring case, so the list matches the Settings page order.
pub fn sort(logins: &mut [Login]) {
    logins.sort_by(|a, b| a.site.to_lowercase().cmp(&b.site.to_lowercase()).then_with(|| a.account.to_lowercase().cmp(&b.account.to_lowercase())));
}

/// Whether a login matches the search text: every space-separated word must appear in the site or the account
/// (case-insensitive). An empty query matches everything.
pub fn matches(login: &Login, query: &str) -> bool {
    let site = login.site.to_lowercase();
    let account = login.account.to_lowercase();
    query.split_whitespace().all(|w| {
        let w = w.to_lowercase();
        site.contains(&w) || account.contains(&w)
    })
}

/// The logins that match `query`, in their current order.
pub fn filter<'a>(logins: &'a [Login], query: &str) -> Vec<&'a Login> {
    logins.iter().filter(|l| matches(l, query)).collect()
}

/// A new password from the kernel random pool: upper and lower case letters, digits and symbols.
pub fn generate() -> Result<String, String> {
    password_gen::generate(&Rules::standard(GENERATED_LENGTH)).map_err(|e| e.to_string())
}

/// The same generator with a caller-supplied random source (tests fix every draw).
pub fn generate_with<F>(length: usize, below: F) -> Result<String, String>
where
    F: FnMut(u64) -> Option<u64>,
{
    password_gen::generate_with(&Rules::standard(length), below).map_err(|e| e.to_string())
}

/// One row of an import: a login with its password.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub site: String,
    pub account: String,
    pub password: String,
}

/// The result of reading a CSV: the usable rows, and how many rows were skipped for a missing field.
#[derive(Debug, PartialEq, Eq)]
pub struct Import {
    pub entries: Vec<Entry>,
    pub skipped: usize,
}

/// Splits CSV text into rows of fields (RFC 4180: quoted fields, doubled quotes, CR or LF line ends). Blank lines are
/// dropped. An unterminated quote is an error.
pub fn parse_csv(text: &str) -> Result<Vec<Vec<String>>, String> {
    let mut rows = Vec::new();
    let mut row: Vec<String> = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if quoted {
            if c == '"' {
                if chars.peek() == Some(&'"') {
                    field.push('"');
                    chars.next();
                } else {
                    quoted = false;
                }
            } else {
                field.push(c);
            }
            continue;
        }
        match c {
            '"' if field.is_empty() => quoted = true,
            ',' => row.push(std::mem::take(&mut field)),
            '\r' | '\n' => {
                if c == '\r' && chars.peek() == Some(&'\n') {
                    chars.next();
                }
                row.push(std::mem::take(&mut field));
                if row.iter().any(|f| !f.is_empty()) {
                    rows.push(std::mem::take(&mut row));
                } else {
                    row.clear();
                }
            }
            _ => field.push(c),
        }
    }
    if quoted {
        return Err("a quoted field is not closed".into());
    }
    row.push(field);
    if row.iter().any(|f| !f.is_empty()) {
        rows.push(row);
    }
    Ok(rows)
}

/// Reads a logins CSV. The first row is the header; the columns are found by name: the site as `site`, `url`, `name`
/// or `website`; the account as `account`, `username`, `login` or `email`; the password as `password`. A row without
/// a site, an account or a password is skipped and counted.
pub fn parse_import(text: &str) -> Result<Import, String> {
    let rows = parse_csv(text)?;
    let Some((header, body)) = rows.split_first() else {
        return Err("the file is empty".into());
    };
    let find = |names: &[&str]| header.iter().position(|h| names.contains(&h.trim().to_lowercase().as_str()));
    let (Some(site), Some(account), Some(password)) = (find(&["site", "url", "name", "website"]), find(&["account", "username", "login", "email"]), find(&["password"])) else {
        return Err("the header needs site, account and password columns".into());
    };
    let mut entries = Vec::new();
    let mut skipped = 0;
    for row in body {
        let get = |i: usize| row.get(i).map(|v| v.trim()).unwrap_or("");
        let (s, a, p) = (get(site), get(account), row.get(password).map(String::as_str).unwrap_or(""));
        if s.is_empty() || a.is_empty() || p.is_empty() {
            skipped += 1;
        } else {
            entries.push(Entry { site: s.to_string(), account: a.to_string(), password: p.to_string() });
        }
    }
    Ok(Import { entries, skipped })
}

/// One CSV field, quoted when it holds a comma, a quote or a line end.
fn field(v: &str) -> String {
    if v.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", v.replace('"', "\"\""))
    } else {
        v.to_string()
    }
}

/// The export text: a header line, then `site,account,password` per entry, with CRLF line ends.
pub fn render_export(entries: &[Entry]) -> String {
    let mut out = String::from("site,account,password\r\n");
    for e in entries {
        out.push_str(&format!("{},{},{}\r\n", field(&e.site), field(&e.account), field(&e.password)));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn login(site: &str, account: &str) -> Login {
        Login { site: site.into(), account: account.into() }
    }

    #[test]
    fn sort_is_by_site_then_account_ignoring_case() {
        let mut v = vec![login("b.example", "me"), login("A.example", "zed"), login("a.example", "amy")];
        sort(&mut v);
        let order: Vec<(&str, &str)> = v.iter().map(|l| (l.site.as_str(), l.account.as_str())).collect();
        assert_eq!(order, [("a.example", "amy"), ("A.example", "zed"), ("b.example", "me")]);
    }

    #[test]
    fn search_matches_every_word_in_site_or_account() {
        let l = login("github.com", "olibuijr");
        assert!(matches(&l, ""));
        assert!(matches(&l, "GitHub"));
        assert!(matches(&l, "git oli"), "words may come from the site and the account");
        assert!(!matches(&l, "git bank"));
        let all = vec![login("github.com", "a"), login("bank.is", "b")];
        assert_eq!(filter(&all, "bank").len(), 1);
    }

    #[test]
    fn generated_password_uses_the_length_and_every_class() {
        let p = generate_with(GENERATED_LENGTH, |bound| Some(bound / 2)).unwrap();
        assert_eq!(p.chars().count(), GENERATED_LENGTH);
        assert!(p.chars().any(|c| c.is_ascii_lowercase()));
        assert!(p.chars().any(|c| c.is_ascii_uppercase()));
        assert!(p.chars().any(|c| c.is_ascii_digit()));
        assert!(p.chars().any(|c| "!#$%&*+-=?@^_".contains(c)));
    }

    #[test]
    fn generator_refuses_a_length_too_short_for_the_classes() {
        assert!(generate_with(3, |b| Some(b / 2)).is_err());
    }

    #[test]
    fn csv_reads_quoted_fields_doubled_quotes_and_crlf() {
        let rows = parse_csv("a,\"b,c\",\"say \"\"hi\"\"\"\r\n\r\nx,y,z").unwrap();
        assert_eq!(rows, vec![vec!["a".to_string(), "b,c".into(), "say \"hi\"".into()], vec!["x".to_string(), "y".into(), "z".into()]]);
        assert!(parse_csv("a,\"open").is_err());
    }

    #[test]
    fn import_finds_columns_by_name_and_counts_skipped_rows() {
        let text = "Name,Username,Password\nexample.com,me,hunter2\n,nobody,x\nbank.is,you,\"p,w\"\n";
        let got = parse_import(text).unwrap();
        assert_eq!(got.skipped, 1);
        assert_eq!(got.entries[0], Entry { site: "example.com".into(), account: "me".into(), password: "hunter2".into() });
        assert_eq!(got.entries[1].password, "p,w");
        assert!(parse_import("site,account\n").is_err(), "a header without a password column is refused");
        assert!(parse_import("").is_err());
    }

    #[test]
    fn export_quotes_what_needs_it_and_round_trips_through_import() {
        let entries = vec![Entry { site: "a.example".into(), account: "me".into(), password: "pa,\"ss\"".into() }];
        let text = render_export(&entries);
        assert_eq!(text, "site,account,password\r\na.example,me,\"pa,\"\"ss\"\"\"\r\n");
        assert_eq!(parse_import(&text).unwrap().entries, entries);
    }
}
