//! Passwords model: the items in the shared passwords group (logins, secure notes and certificates), search,
//! generated passwords and the CSV import and export. Pure logic (no window, no keychain), so it is tested alone;
//! `main.rs` does the calls and the drawing.
//!
//! Secrets only leave the model as the caller asks for them: a listing carries names, accounts and kinds, never a
//! password, note body or certificate. Nothing here logs or formats a secret for display except the generated
//! password the user asked for.

use bifrost_app::password_gen::{self, Rules};
use bifrost_app::passwords::{kind_of, name_of, Kind};

/// Length of a generated password.
pub const GENERATED_LENGTH: usize = 20;

/// One item as the list shows it. The secret is not held here; it is read from the keychain when asked for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    pub kind: Kind,
    /// The keychain service: the site of a login, or a kind prefix and the title or label (see `passwords::kind_of`).
    pub service: String,
    pub account: String,
}

impl Item {
    /// Builds the item for one listing row.
    pub fn new(service: String, account: String) -> Item {
        Item { kind: kind_of(&service), service, account }
    }

    /// The name the list shows: the site, the note title or the certificate label.
    pub fn name(&self) -> &str {
        name_of(&self.service)
    }
}

/// Sorts by name, then account, ignoring case, so the list matches the Settings page order.
pub fn sort(items: &mut [Item]) {
    items.sort_by(|a, b| a.name().to_lowercase().cmp(&b.name().to_lowercase()).then_with(|| a.account.to_lowercase().cmp(&b.account.to_lowercase())));
}

/// Whether an item matches the search text: every space-separated word must appear in the name or the account
/// (case-insensitive). An empty query matches everything.
pub fn matches(item: &Item, query: &str) -> bool {
    let name = item.name().to_lowercase();
    let account = item.account.to_lowercase();
    query.split_whitespace().all(|w| {
        let w = w.to_lowercase();
        name.contains(&w) || account.contains(&w)
    })
}

/// The items of `kind` that match `query`, in their current order.
pub fn filter<'a>(items: &'a [Item], kind: Kind, query: &str) -> Vec<&'a Item> {
    items.iter().filter(|i| i.kind == kind && matches(i, query)).collect()
}

/// A new password from the kernel random pool: upper and lower case letters, digits and symbols.
pub fn generate() -> Result<String, String> {
    password_gen::generate(&Rules::standard(GENERATED_LENGTH)).map_err(|e| e.to_string())
}

/// The same generator with a caller-supplied random source (tests fix every draw).
#[cfg_attr(not(test), allow(dead_code))]
pub fn generate_with<F>(length: usize, below: F) -> Result<String, String>
where
    F: FnMut(u64) -> Option<u64>,
{
    password_gen::generate_with(&Rules::standard(length), below).map_err(|e| e.to_string())
}

/// The one-time code a revealed login secret gives at a Unix time, when the secret is an `otpauth://totp/` URI: the
/// code and the seconds until it changes. `None` for any other secret.
pub fn verification_code(secret: &str, unix_time: u64) -> Option<(String, u64)> {
    let otp = bifrost_app::totp::parse_otpauth(secret)?;
    let code = otp.code_at(unix_time).ok()?;
    Some((code, otp.seconds_left(unix_time)))
}

/// One row of an import or export: a login with its password.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub site: String,
    pub account: String,
    pub password: String,
}

/// The result of reading a CSV: the usable rows, and how many rows were skipped (a missing field, or a site that
/// would be stored as a note or certificate).
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
/// a site, an account or a password is skipped and counted, and so is a site that starts with a note or certificate
/// prefix, since it would be stored as one of those kinds.
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
        if s.is_empty() || a.is_empty() || p.is_empty() || kind_of(s) != Kind::Login {
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

    fn login(site: &str, account: &str) -> Item {
        Item::new(site.into(), account.into())
    }

    #[test]
    fn items_take_their_kind_and_name_from_the_service() {
        let note = Item::new("note:Wi-Fi codes".into(), "secure-note".into());
        assert_eq!(note.kind, Kind::SecureNote);
        assert_eq!(note.name(), "Wi-Fi codes");
        let cert = Item::new("cert:Work CA".into(), "certificate".into());
        assert_eq!(cert.kind, Kind::Certificate);
        assert_eq!(cert.name(), "Work CA");
        assert_eq!(login("github.com", "me").name(), "github.com");
    }

    #[test]
    fn sort_is_by_name_then_account_ignoring_case() {
        let mut v = vec![login("b.example", "me"), login("A.example", "zed"), login("a.example", "amy")];
        sort(&mut v);
        let order: Vec<(&str, &str)> = v.iter().map(|l| (l.service.as_str(), l.account.as_str())).collect();
        assert_eq!(order, [("a.example", "amy"), ("A.example", "zed"), ("b.example", "me")]);
    }

    #[test]
    fn search_matches_every_word_in_name_or_account() {
        let l = login("github.com", "olibuijr");
        assert!(matches(&l, ""));
        assert!(matches(&l, "GitHub"));
        assert!(matches(&l, "git oli"), "words may come from the name and the account");
        assert!(!matches(&l, "git bank"));
        let note = Item::new("note:Wi-Fi codes".into(), "secure-note".into());
        assert!(matches(&note, "wi-fi"), "a note is searched by its title");
        let all = vec![login("github.com", "a"), login("bank.is", "b")];
        assert_eq!(filter(&all, Kind::Login, "bank").len(), 1);
    }

    #[test]
    fn filter_keeps_one_kind_at_a_time() {
        let all = vec![login("github.com", "a"), Item::new("note:github notes".into(), "secure-note".into())];
        assert_eq!(filter(&all, Kind::Login, "github").len(), 1);
        assert_eq!(filter(&all, Kind::SecureNote, "github").len(), 1);
        assert!(filter(&all, Kind::Certificate, "github").is_empty());
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
    fn import_skips_a_site_that_would_be_stored_as_a_note() {
        let got = parse_import("site,account,password\nnote:x,me,hunter2\nok.example,me,pw\n").unwrap();
        assert_eq!(got.skipped, 1);
        assert_eq!(got.entries.len(), 1);
        assert_eq!(got.entries[0].site, "ok.example");
    }

    #[test]
    fn export_quotes_what_needs_it_and_round_trips_through_import() {
        let entries = vec![Entry { site: "a.example".into(), account: "me".into(), password: "pa,\"ss\"".into() }];
        let text = render_export(&entries);
        assert_eq!(text, "site,account,password\r\na.example,me,\"pa,\"\"ss\"\"\"\r\n");
        assert_eq!(parse_import(&text).unwrap().entries, entries);
    }

    #[test]
    fn a_secret_that_is_an_otpauth_uri_gives_its_current_code() {
        let uri = "otpauth://totp/Example:me?secret=GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ&issuer=Example";
        assert_eq!(verification_code(uri, 59), Some(("287082".to_string(), 1)));
        assert_eq!(verification_code(uri, 60), Some(("359152".to_string(), 30)));
        assert_eq!(verification_code("hunter2", 59), None);
    }
}
