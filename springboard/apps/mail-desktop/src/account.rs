//! Mail accounts: the IMAP and SMTP servers an account talks to. The account list is saved as plain text in the
//! app data directory (`accounts.ini`). A password is never written there; it lives in the keychain under
//! [`KEYCHAIN_SERVICE`] with [`Account::keychain_account`] as the account name.

use std::fmt;

pub const KEYCHAIN_SERVICE: &str = "is.olibuijr.mail.desktop";

/// How a connection is secured.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Security {
    /// TLS from the first byte (IMAPS 993, SMTPS 465).
    Tls,
    /// Plain connection upgraded with STARTTLS (143, 587).
    StartTls,
    /// No encryption. Only for a server on a trusted local network.
    Plain,
}

impl Security {
    fn as_str(self) -> &'static str {
        match self {
            Security::Tls => "tls",
            Security::StartTls => "starttls",
            Security::Plain => "plain",
        }
    }

    fn parse(s: &str) -> Option<Security> {
        match s {
            "tls" => Some(Security::Tls),
            "starttls" => Some(Security::StartTls),
            "plain" => Some(Security::Plain),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Server {
    pub host: String,
    pub port: u16,
    pub security: Security,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Account {
    /// Stable id, used for the file section and the keychain account name. Letters, digits, `-` and `_` only.
    pub id: String,
    /// The name shown in the sidebar.
    pub name: String,
    /// The sender address.
    pub email: String,
    /// The login name, often the same as the address.
    pub username: String,
    pub imap: Server,
    pub smtp: Server,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AccountError {
    /// A required field is empty.
    Missing(&'static str),
    /// A field has a value that cannot be used (bad id, address, port, or a line break inside a value).
    Invalid(&'static str),
    /// The saved file has a bad line at this 1-based line number.
    Parse(usize),
}

impl fmt::Display for AccountError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AccountError::Missing(field) => write!(f, "{field} is required"),
            AccountError::Invalid(field) => write!(f, "{field} is not valid"),
            AccountError::Parse(line) => write!(f, "accounts file line {line} is not valid"),
        }
    }
}

impl Account {
    /// The keychain account name for this account's password.
    pub fn keychain_account(&self) -> String {
        format!("account:{}", self.id)
    }

    /// Checks every field the saved file and the connection code rely on.
    pub fn validate(&self) -> Result<(), AccountError> {
        if self.id.is_empty() {
            return Err(AccountError::Missing("id"));
        }
        if !self.id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_') {
            return Err(AccountError::Invalid("id"));
        }
        if self.email.is_empty() {
            return Err(AccountError::Missing("email"));
        }
        if !valid_address(&self.email) {
            return Err(AccountError::Invalid("email"));
        }
        for (field, value) in [("name", &self.name), ("username", &self.username)] {
            if value.contains(['\r', '\n']) {
                return Err(AccountError::Invalid(field));
            }
        }
        if self.username.is_empty() {
            return Err(AccountError::Missing("username"));
        }
        validate_server("imap", &self.imap)?;
        validate_server("smtp", &self.smtp)
    }
}

fn validate_server(which: &'static str, server: &Server) -> Result<(), AccountError> {
    if server.host.is_empty() {
        return Err(AccountError::Missing(if which == "imap" { "imap host" } else { "smtp host" }));
    }
    if server.host.contains(|c: char| c.is_whitespace() || c == '/' || c == '[' || c == ']') {
        return Err(AccountError::Invalid(if which == "imap" { "imap host" } else { "smtp host" }));
    }
    if server.port == 0 {
        return Err(AccountError::Invalid(if which == "imap" { "imap port" } else { "smtp port" }));
    }
    Ok(())
}

/// A plain `local@domain` address: one `@`, no spaces or angle brackets, no line breaks.
pub fn valid_address(addr: &str) -> bool {
    let Some((local, domain)) = addr.split_once('@') else { return false };
    !local.is_empty()
        && !domain.is_empty()
        && !domain.contains('@')
        && !addr.contains(|c: char| c.is_whitespace() || c == '<' || c == '>' || c == ',' || c == ';')
}

/// Serialises accounts. Sections are `[account <id>]` with `key = value` lines.
pub fn encode(accounts: &[Account]) -> String {
    let mut out = String::new();
    for a in accounts {
        out.push_str(&format!("[account {}]\n", a.id));
        out.push_str(&format!("name = {}\n", a.name));
        out.push_str(&format!("email = {}\n", a.email));
        out.push_str(&format!("username = {}\n", a.username));
        out.push_str(&format!("imap.host = {}\nimap.port = {}\nimap.security = {}\n", a.imap.host, a.imap.port, a.imap.security.as_str()));
        out.push_str(&format!("smtp.host = {}\nsmtp.port = {}\nsmtp.security = {}\n\n", a.smtp.host, a.smtp.port, a.smtp.security.as_str()));
    }
    out
}

fn blank(id: &str) -> Account {
    let server = Server { host: String::new(), port: 0, security: Security::Tls };
    Account { id: id.to_string(), name: String::new(), email: String::new(), username: String::new(), imap: server.clone(), smtp: server }
}

/// Reads accounts written by [`encode`]. Unknown keys are ignored; each account is validated.
pub fn decode(text: &str) -> Result<Vec<Account>, AccountError> {
    let mut accounts: Vec<Account> = Vec::new();
    for (index, raw) in text.lines().enumerate() {
        let line_no = index + 1;
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(inner) = line.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
            let id = inner.strip_prefix("account ").ok_or(AccountError::Parse(line_no))?.trim();
            accounts.push(blank(id));
            continue;
        }
        let (key, value) = line.split_once('=').ok_or(AccountError::Parse(line_no))?;
        let (key, value) = (key.trim(), value.trim());
        let current = accounts.last_mut().ok_or(AccountError::Parse(line_no))?;
        match key {
            "name" => current.name = value.to_string(),
            "email" => current.email = value.to_string(),
            "username" => current.username = value.to_string(),
            "imap.host" => current.imap.host = value.to_string(),
            "imap.port" => current.imap.port = value.parse().map_err(|_| AccountError::Parse(line_no))?,
            "imap.security" => current.imap.security = Security::parse(value).ok_or(AccountError::Parse(line_no))?,
            "smtp.host" => current.smtp.host = value.to_string(),
            "smtp.port" => current.smtp.port = value.parse().map_err(|_| AccountError::Parse(line_no))?,
            "smtp.security" => current.smtp.security = Security::parse(value).ok_or(AccountError::Parse(line_no))?,
            _ => {}
        }
    }
    for a in &accounts {
        a.validate()?;
    }
    Ok(accounts)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Account {
        Account {
            id: "work-1".into(),
            name: "Work".into(),
            email: "olibuijr@example.com".into(),
            username: "olibuijr".into(),
            imap: Server { host: "imap.example.com".into(), port: 993, security: Security::Tls },
            smtp: Server { host: "smtp.example.com".into(), port: 587, security: Security::StartTls },
        }
    }

    #[test]
    fn accounts_round_trip_through_the_file_format() {
        let accounts = vec![sample(), Account { id: "home".into(), ..sample() }];
        assert_eq!(decode(&encode(&accounts)).unwrap(), accounts);
    }

    #[test]
    fn keychain_name_never_holds_the_password_itself() {
        assert_eq!(sample().keychain_account(), "account:work-1");
        assert!(!encode(&[sample()]).contains("password"));
    }

    #[test]
    fn validation_rejects_bad_fields() {
        let mut a = sample();
        a.email = "no-at-sign".into();
        assert_eq!(a.validate(), Err(AccountError::Invalid("email")));
        let mut a = sample();
        a.imap.port = 0;
        assert_eq!(a.validate(), Err(AccountError::Invalid("imap port")));
        let mut a = sample();
        a.name = "Evil\r\nBcc: x".into();
        assert_eq!(a.validate(), Err(AccountError::Invalid("name")));
        let mut a = sample();
        a.id = "a b".into();
        assert_eq!(a.validate(), Err(AccountError::Invalid("id")));
    }

    #[test]
    fn decode_reports_the_bad_line() {
        assert_eq!(decode("name = x\n"), Err(AccountError::Parse(1)));
        assert_eq!(decode("[account a]\nimap.port = abc\n"), Err(AccountError::Parse(2)));
    }

    #[test]
    fn addresses() {
        assert!(valid_address("a@b.is"));
        assert!(!valid_address("a@b@c"));
        assert!(!valid_address("<a@b.is>"));
        assert!(!valid_address("a b@c"));
    }
}
