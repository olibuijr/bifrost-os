//! Internet Accounts: the one list of mail, calendar and contact accounts that Settings > Internet Accounts keeps.
//! Mail, Calendar, Contacts and Reminders read the same list ([`load`] from [`store_path`]).
//!
//! The list is a plain text file in the user's data directory: one `[account <id>]` section per account, with
//! `key = value` lines. A password is never written there. It lives in the passwords group ([`crate::passwords`]),
//! under the site [`Account::password_site`] and the account name [`Account::username`].
//!
//! An account is one of three [`Kind`]s: mail over IMAP and SMTP, or a CalDAV or CardDAV server. Each account
//! switches its [`Service`]s on and off; a service only applies to the kinds that can feed it (see
//! [`Kind::services`]). The DAV requests and answers are in [`dav`].

pub mod dav;

use std::fmt;
use std::path::{Path, PathBuf};

/// The file name of the shared list, inside the `bifrost` directory of the user's data directory.
pub const STORE_FILE: &str = "internet-accounts.ini";
/// The largest list file read.
pub const MAX_STORE_BYTES: u64 = 1024 * 1024;

/// What an account connects to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Mail over an IMAP incoming server and an SMTP outgoing server.
    Mail,
    /// A CalDAV calendar server.
    CalDav,
    /// A CardDAV address book server.
    CardDav,
}

impl Kind {
    pub const ALL: [Kind; 3] = [Kind::Mail, Kind::CalDav, Kind::CardDav];

    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Mail => "mail",
            Kind::CalDav => "caldav",
            Kind::CardDav => "carddav",
        }
    }

    pub fn parse(s: &str) -> Option<Kind> {
        Kind::ALL.into_iter().find(|k| k.as_str() == s)
    }

    /// The name shown when adding an account.
    pub fn label(self) -> &'static str {
        match self {
            Kind::Mail => "Mail (IMAP and SMTP)",
            Kind::CalDav => "Calendar (CalDAV)",
            Kind::CardDav => "Contacts (CardDAV)",
        }
    }

    /// The account type this kind is saved under in the accounts store. The identifiers are the system types the SDK
    /// registers (`bifrost_app::accounts::BUILTIN_TYPES`), so an app that reads the store finds the same type names.
    pub fn account_type(self) -> &'static str {
        match self {
            Kind::Mail => "bifrost.mail",
            Kind::CalDav => "bifrost.caldav",
            Kind::CardDav => "bifrost.carddav",
        }
    }

    /// The services an account of this kind can feed, in the order the pane lists them.
    pub fn services(self) -> &'static [Service] {
        match self {
            Kind::Mail => &[Service::Mail],
            Kind::CalDav => &[Service::Calendars, Service::Reminders],
            Kind::CardDav => &[Service::Contacts],
        }
    }
}

/// An app that reads the shared list.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Service {
    Mail,
    Calendars,
    Contacts,
    Reminders,
}

impl Service {
    pub const ALL: [Service; 4] = [Service::Mail, Service::Calendars, Service::Contacts, Service::Reminders];

    pub fn as_str(self) -> &'static str {
        match self {
            Service::Mail => "mail",
            Service::Calendars => "calendars",
            Service::Contacts => "contacts",
            Service::Reminders => "reminders",
        }
    }

    pub fn parse(s: &str) -> Option<Service> {
        Service::ALL.into_iter().find(|v| v.as_str() == s)
    }

    /// The row label in the account's service list.
    pub fn label(self) -> &'static str {
        match self {
            Service::Mail => "Mail",
            Service::Calendars => "Calendars",
            Service::Contacts => "Contacts",
            Service::Reminders => "Reminders",
        }
    }
}

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

/// One account in the shared list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Account {
    /// Stable id, used for the file section. Letters, digits, `-` and `_` only.
    pub id: String,
    pub kind: Kind,
    /// The name shown in the account list.
    pub name: String,
    /// The login name. The password is stored under this name in the passwords group.
    pub username: String,
    /// The sender address. Mail only; empty for the DAV kinds.
    pub email: String,
    /// Mail: the IMAP incoming server.
    pub imap: Option<Server>,
    /// Mail: the SMTP outgoing server.
    pub smtp: Option<Server>,
    /// CalDAV and CardDAV: the server URL (`https://...`). Empty for mail.
    pub url: String,
    /// The services switched on for this account. Always a subset of [`Kind::services`].
    pub services: Vec<Service>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AccountError {
    /// A required field is empty.
    Missing(&'static str),
    /// A field has a value that cannot be used.
    Invalid(&'static str),
    /// The saved file has a bad line at this 1-based line number.
    Parse(usize),
    /// The file is larger than [`MAX_STORE_BYTES`].
    TooLarge,
}

impl fmt::Display for AccountError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AccountError::Missing(field) => write!(f, "{field} is required"),
            AccountError::Invalid(field) => write!(f, "{field} is not valid"),
            AccountError::Parse(line) => write!(f, "accounts file line {line} is not valid"),
            AccountError::TooLarge => write!(f, "accounts file is too large"),
        }
    }
}

impl Account {
    /// A new account of `kind` with every service that kind can feed switched on.
    pub fn new(kind: Kind, id: &str, name: &str, username: &str) -> Account {
        Account {
            id: id.to_string(),
            kind,
            name: name.to_string(),
            username: username.to_string(),
            email: String::new(),
            imap: None,
            smtp: None,
            url: String::new(),
            services: kind.services().to_vec(),
        }
    }

    pub fn has(&self, service: Service) -> bool {
        self.services.contains(&service)
    }

    /// Switches a service on or off. Returns false, and changes nothing, when this kind cannot feed the service.
    pub fn set_service(&mut self, service: Service, on: bool) -> bool {
        if !self.kind.services().contains(&service) {
            return false;
        }
        self.services.retain(|s| *s != service);
        if on {
            self.services.push(service);
        }
        true
    }

    /// The site the password is stored under in the passwords group: the incoming mail host, or the DAV server host.
    pub fn password_site(&self) -> String {
        match self.kind {
            Kind::Mail => self.imap.as_ref().map(|s| s.host.clone()).unwrap_or_default(),
            Kind::CalDav | Kind::CardDav => url_host(&self.url).unwrap_or_default().to_string(),
        }
    }

    /// Checks every field the saved file and the network code rely on.
    pub fn validate(&self) -> Result<(), AccountError> {
        if self.id.is_empty() {
            return Err(AccountError::Missing("id"));
        }
        if !self.id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_') {
            return Err(AccountError::Invalid("id"));
        }
        if self.name.contains(['\r', '\n']) {
            return Err(AccountError::Invalid("name"));
        }
        if self.username.is_empty() {
            return Err(AccountError::Missing("username"));
        }
        if self.username.contains(['\r', '\n']) {
            return Err(AccountError::Invalid("username"));
        }
        for s in &self.services {
            if !self.kind.services().contains(s) {
                return Err(AccountError::Invalid("services"));
            }
        }
        match self.kind {
            Kind::Mail => {
                if self.email.is_empty() {
                    return Err(AccountError::Missing("email"));
                }
                if !valid_address(&self.email) {
                    return Err(AccountError::Invalid("email"));
                }
                let imap = self.imap.as_ref().ok_or(AccountError::Missing("imap host"))?;
                let smtp = self.smtp.as_ref().ok_or(AccountError::Missing("smtp host"))?;
                validate_server("imap", imap)?;
                validate_server("smtp", smtp)
            }
            Kind::CalDav | Kind::CardDav => {
                if self.url.is_empty() {
                    return Err(AccountError::Missing("url"));
                }
                if !valid_dav_url(&self.url) {
                    return Err(AccountError::Invalid("url"));
                }
                Ok(())
            }
        }
    }
}

fn validate_server(which: &'static str, server: &Server) -> Result<(), AccountError> {
    let host_field = if which == "imap" { "imap host" } else { "smtp host" };
    let port_field = if which == "imap" { "imap port" } else { "smtp port" };
    if server.host.is_empty() {
        return Err(AccountError::Missing(host_field));
    }
    if server.host.contains(|c: char| c.is_whitespace() || c == '/' || c == '[' || c == ']') {
        return Err(AccountError::Invalid(host_field));
    }
    if server.port == 0 {
        return Err(AccountError::Invalid(port_field));
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

/// An `http://` or `https://` URL with a host and no spaces, control characters or empty authority.
pub fn valid_dav_url(url: &str) -> bool {
    let rest = url.strip_prefix("https://").or_else(|| url.strip_prefix("http://"));
    match rest {
        Some(rest) => {
            !url_host(url).unwrap_or("").is_empty()
                && !url.contains(|c: char| c.is_whitespace() || c.is_control())
                && !rest.starts_with('/')
        }
        None => false,
    }
}

/// The host of an `http://` or `https://` URL, without port, path or user information.
pub fn url_host(url: &str) -> Option<&str> {
    let rest = url.strip_prefix("https://").or_else(|| url.strip_prefix("http://"))?;
    let authority = rest.split(['/', '?', '#']).next()?;
    let host_port = authority.rsplit('@').next()?;
    let host = if let Some(stripped) = host_port.strip_prefix('[') {
        stripped.split(']').next()?
    } else {
        host_port.split(':').next()?
    };
    Some(host)
}

/// Where the shared list lives: `$XDG_DATA_HOME/bifrost/internet-accounts.ini`, or under `~/.local/share`.
pub fn store_path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_DATA_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))?;
    Some(base.join("bifrost").join(STORE_FILE))
}

/// Serialises accounts. Sections are `[account <id>]` with `key = value` lines.
pub fn encode(accounts: &[Account]) -> String {
    let mut out = String::new();
    for a in accounts {
        out.push_str(&format!("[account {}]\n", a.id));
        out.push_str(&format!("kind = {}\n", a.kind.as_str()));
        out.push_str(&format!("name = {}\n", a.name));
        out.push_str(&format!("username = {}\n", a.username));
        if a.kind == Kind::Mail {
            out.push_str(&format!("email = {}\n", a.email));
            if let Some(s) = &a.imap {
                out.push_str(&format!("imap.host = {}\nimap.port = {}\nimap.security = {}\n", s.host, s.port, s.security.as_str()));
            }
            if let Some(s) = &a.smtp {
                out.push_str(&format!("smtp.host = {}\nsmtp.port = {}\nsmtp.security = {}\n", s.host, s.port, s.security.as_str()));
            }
        } else {
            out.push_str(&format!("url = {}\n", a.url));
        }
        let names: Vec<&str> = a.services.iter().map(|s| s.as_str()).collect();
        out.push_str(&format!("services = {}\n\n", names.join(",")));
    }
    out
}

struct Draft {
    account: Account,
    has_kind: bool,
}

/// Reads accounts written by [`encode`]. Unknown keys are ignored; each account is validated.
pub fn decode(text: &str) -> Result<Vec<Account>, AccountError> {
    let mut drafts: Vec<Draft> = Vec::new();
    for (index, raw) in text.lines().enumerate() {
        let line_no = index + 1;
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(inner) = line.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
            let id = inner.strip_prefix("account ").ok_or(AccountError::Parse(line_no))?.trim();
            drafts.push(Draft { account: Account::new(Kind::Mail, id, "", ""), has_kind: false });
            continue;
        }
        let (key, value) = line.split_once('=').ok_or(AccountError::Parse(line_no))?;
        let (key, value) = (key.trim(), value.trim());
        let current = drafts.last_mut().ok_or(AccountError::Parse(line_no))?;
        let a = &mut current.account;
        match key {
            "kind" => {
                a.kind = Kind::parse(value).ok_or(AccountError::Parse(line_no))?;
                current.has_kind = true;
            }
            "name" => a.name = value.to_string(),
            "username" => a.username = value.to_string(),
            "email" => a.email = value.to_string(),
            "url" => a.url = value.to_string(),
            "services" => {
                a.services = Vec::new();
                for part in value.split(',').map(str::trim).filter(|p| !p.is_empty()) {
                    let s = Service::parse(part).ok_or(AccountError::Parse(line_no))?;
                    if !a.services.contains(&s) {
                        a.services.push(s);
                    }
                }
            }
            "imap.host" | "imap.port" | "imap.security" => {
                let s = a.imap.get_or_insert(Server { host: String::new(), port: 0, security: Security::Tls });
                set_server_field(s, key, value, line_no)?;
            }
            "smtp.host" | "smtp.port" | "smtp.security" => {
                let s = a.smtp.get_or_insert(Server { host: String::new(), port: 0, security: Security::Tls });
                set_server_field(s, key, value, line_no)?;
            }
            _ => {}
        }
    }
    let mut accounts = Vec::with_capacity(drafts.len());
    for d in drafts {
        if !d.has_kind {
            return Err(AccountError::Missing("kind"));
        }
        d.account.validate()?;
        accounts.push(d.account);
    }
    Ok(accounts)
}

fn set_server_field(s: &mut Server, key: &str, value: &str, line_no: usize) -> Result<(), AccountError> {
    let field = key.rsplit('.').next().unwrap_or("");
    match field {
        "host" => s.host = value.to_string(),
        "port" => s.port = value.parse().map_err(|_| AccountError::Parse(line_no))?,
        _ => s.security = Security::parse(value).ok_or(AccountError::Parse(line_no))?,
    }
    Ok(())
}

/// Reads the shared list. A missing file is an empty list.
pub fn load(path: &Path) -> Result<Vec<Account>, AccountError> {
    match std::fs::metadata(path) {
        Err(_) => return Ok(Vec::new()),
        Ok(m) if m.len() > MAX_STORE_BYTES => return Err(AccountError::TooLarge),
        Ok(_) => {}
    }
    let text = std::fs::read_to_string(path).map_err(|_| AccountError::Parse(0))?;
    decode(&text)
}

/// Writes the shared list through a temporary file and a rename. Refuses accounts that do not validate.
pub fn save(path: &Path, accounts: &[Account]) -> std::io::Result<()> {
    for a in accounts {
        a.validate().map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("ini.tmp");
    std::fs::write(&tmp, encode(accounts))?;
    std::fs::rename(&tmp, path)
}

/// The accounts that switch `service` on, in list order. Every app that reads the list uses this.
pub fn serving(accounts: &[Account], service: Service) -> Vec<&Account> {
    accounts.iter().filter(|a| a.has(service)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_kind_has_its_own_account_type_name() {
        let names: Vec<&str> = Kind::ALL.iter().map(|k| k.account_type()).collect();
        assert_eq!(names, vec!["bifrost.mail", "bifrost.caldav", "bifrost.carddav"]);
        assert!(names.iter().all(|n| n.starts_with("bifrost.")));
    }

    fn mail() -> Account {
        Account {
            email: "olibuijr@example.com".into(),
            imap: Some(Server { host: "imap.example.com".into(), port: 993, security: Security::Tls }),
            smtp: Some(Server { host: "smtp.example.com".into(), port: 587, security: Security::StartTls }),
            ..Account::new(Kind::Mail, "work-1", "Work", "olibuijr")
        }
    }

    fn dav(kind: Kind) -> Account {
        Account { url: "https://dav.example.com/dav/".into(), ..Account::new(kind, "dav-1", "Home", "olibuijr") }
    }

    #[test]
    fn accounts_round_trip_through_the_file_format() {
        let accounts = vec![mail(), dav(Kind::CalDav), dav(Kind::CardDav)];
        assert_eq!(decode(&encode(&accounts)).unwrap(), accounts);
    }

    #[test]
    fn the_file_never_holds_a_password() {
        assert!(!encode(&[mail(), dav(Kind::CalDav)]).contains("password"));
    }

    #[test]
    fn a_service_only_applies_to_the_kinds_that_feed_it() {
        let mut a = dav(Kind::CalDav);
        assert!(a.has(Service::Reminders));
        assert!(a.set_service(Service::Reminders, false));
        assert!(!a.has(Service::Reminders));
        assert!(a.has(Service::Calendars));
        assert!(!a.set_service(Service::Mail, true));
        assert!(!a.has(Service::Mail));
        assert!(a.set_service(Service::Reminders, true));
        assert!(a.has(Service::Reminders));
    }

    #[test]
    fn serving_lists_only_the_accounts_switched_on() {
        let mut off = dav(Kind::CardDav);
        off.id = "off".into();
        off.set_service(Service::Contacts, false);
        let all = vec![off, dav(Kind::CardDav)];
        let got: Vec<&str> = serving(&all, Service::Contacts).iter().map(|a| a.id.as_str()).collect();
        assert_eq!(got, vec!["dav-1"]);
        assert!(serving(&all, Service::Mail).is_empty());
    }

    #[test]
    fn the_password_site_is_the_server_host() {
        assert_eq!(mail().password_site(), "imap.example.com");
        assert_eq!(dav(Kind::CalDav).password_site(), "dav.example.com");
        let mut a = dav(Kind::CalDav);
        a.url = "https://user@[::1]:8443/x".into();
        assert_eq!(a.password_site(), "::1");
    }

    #[test]
    fn validation_rejects_bad_fields() {
        let mut a = mail();
        a.email = "no-at-sign".into();
        assert_eq!(a.validate(), Err(AccountError::Invalid("email")));
        let mut a = mail();
        a.imap.as_mut().unwrap().port = 0;
        assert_eq!(a.validate(), Err(AccountError::Invalid("imap port")));
        let mut a = mail();
        a.name = "Evil\r\nBcc: x".into();
        assert_eq!(a.validate(), Err(AccountError::Invalid("name")));
        let mut a = mail();
        a.id = "a b".into();
        assert_eq!(a.validate(), Err(AccountError::Invalid("id")));
        let mut a = dav(Kind::CardDav);
        a.url = "ftp://example.com/".into();
        assert_eq!(a.validate(), Err(AccountError::Invalid("url")));
        let mut a = dav(Kind::CardDav);
        a.services.push(Service::Calendars);
        assert_eq!(a.validate(), Err(AccountError::Invalid("services")));
    }

    #[test]
    fn decode_reports_the_bad_line() {
        assert_eq!(decode("name = x\n"), Err(AccountError::Parse(1)));
        assert_eq!(decode("[account a]\nkind = caldav\nservices = bogus\n"), Err(AccountError::Parse(3)));
        assert_eq!(decode("[account a]\nname = x\n"), Err(AccountError::Missing("kind")));
    }

    #[test]
    fn url_hosts() {
        assert_eq!(url_host("https://a.example.com:8443/x?y#z"), Some("a.example.com"));
        assert_eq!(url_host("http://user:pw@h.example/"), Some("h.example"));
        assert_eq!(url_host("mailto:x"), None);
        assert!(valid_dav_url("https://dav.example.com/"));
        assert!(!valid_dav_url("https:///path"));
    }

    #[test]
    fn addresses() {
        assert!(valid_address("a@b.is"));
        assert!(!valid_address("a@b@c"));
        assert!(!valid_address("<a@b.is>"));
        assert!(!valid_address("a b@c"));
    }

    #[test]
    fn save_then_load_round_trips_through_a_file() {
        let dir = std::env::temp_dir().join(format!("ia-test-{}", std::process::id()));
        let path = dir.join(STORE_FILE);
        let accounts = vec![mail(), dav(Kind::CalDav)];
        save(&path, &accounts).unwrap();
        assert_eq!(load(&path).unwrap(), accounts);
        assert_eq!(load(&dir.join("missing.ini")).unwrap(), Vec::new());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
