//! Finds a mail account's servers from its address, so Add Account asks for a name, an address and a password only.
//!
//! The order, each step filling only what the earlier ones left open:
//! 1. the domain's autoconfig file (`autoconfig.<domain>`, then `.well-known`), fetched over https;
//! 2. the service records of RFC 6186 (`_imaps._tcp`, `_imap._tcp`, `_submissions._tcp`, `_submission._tcp`);
//! 3. the domain's mail exchanger, as a last guess;
//! 4. `imap.<domain>` and `smtp.<domain>`, only when they exist.
//!
//! DNS answers are not signed, so a server that they name outside the address's own domain is not trusted on its own: the
//! result says so (`trusted`) and Add Account shows it for the person to confirm before the password is sent there.

use crate::autoconfig;
use crate::dns::{self, Mx, Record, Resolver, Srv};
use bifrost_app::controls::internet_accounts::{Security, Server};
use springboard::deildu::http::{Client, Jar, Transport};
use std::sync::Arc;

/// Largest autoconfig file read.
const FILE_LIMIT: usize = 256 * 1024;

/// Fetches a document by its address. The real one speaks https; tests give canned files.
pub trait Fetch {
    /// The body of a 200 reply, or None for any failure.
    fn get(&self, url: &str) -> Option<String>;

    /// Like [`Fetch::get`], and also the address the body really came from after any redirects. The default is for
    /// fetchers that never redirect.
    fn get_from(&self, url: &str) -> Option<(String, String)> {
        self.get(url).map(|body| (body, url.to_string()))
    }
}

/// The host of an address, lower case, or None when there is none.
fn url_host(url: &str) -> Option<String> {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    let host = host.rsplit_once(':').filter(|(_, p)| p.bytes().all(|b| b.is_ascii_digit())).map_or(host, |(h, _)| h);
    (!host.is_empty()).then(|| host.to_ascii_lowercase())
}

/// The DNS questions discovery asks.
pub trait Lookup {
    /// The service records of `name` (`_imaps._tcp.example.net`), in the order to try them.
    fn srv(&self, name: &str) -> Vec<Srv>;
    /// The mail exchangers of `domain`, the most preferred first.
    fn mx(&self, domain: &str) -> Vec<String>;
    /// True when `host` has an address.
    fn resolves(&self, host: &str) -> bool;
}

/// What discovery found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Found {
    pub imap: Server,
    pub smtp: Server,
    /// The login the domain asks for, when it says (the address is the login otherwise).
    pub username: Option<String>,
    /// False when a server came from DNS alone and is not in the address's own domain.
    pub trusted: bool,
}

/// One server and whether it can be used without asking.
struct Part {
    server: Server,
    trusted: bool,
}

/// True when `host` is `domain` or a name under it.
pub fn in_domain(host: &str, domain: &str) -> bool {
    let (host, domain) = (host.trim_end_matches('.').to_ascii_lowercase(), domain.to_ascii_lowercase());
    host == domain || host.ends_with(&format!(".{domain}"))
}

fn imap_security(port: u16) -> Security {
    if port == 993 {
        Security::Tls
    } else {
        Security::StartTls
    }
}

fn smtp_security(port: u16) -> Security {
    if port == 465 {
        Security::Tls
    } else {
        Security::StartTls
    }
}

/// The first usable service record of `name` as a server.
fn from_srv(lookup: &dyn Lookup, name: &str, domain: &str, security: fn(u16) -> Security) -> Option<Part> {
    lookup.srv(name).into_iter().find_map(|r| {
        let host = r.target.trim_end_matches('.').to_ascii_lowercase();
        autoconfig::valid_host(&host).then(|| {
            let trusted = in_domain(&host, domain);
            Part { server: Server { host, port: r.port, security: security(r.port) }, trusted }
        })
    })
}

/// Looks up the servers for `email`.
pub fn discover(email: &str, fetch: &dyn Fetch, lookup: &dyn Lookup) -> Result<Found, String> {
    let domain = email.rsplit_once('@').map_or("", |(_, d)| d).trim().to_ascii_lowercase();
    if !autoconfig::valid_host(&domain) || !domain.contains('.') {
        return Err(format!("Mail cannot look up servers for {email}. Check the address, or choose Configure Manually"));
    }
    let (mut imap, mut smtp): (Option<Part>, Option<Part>) = (None, None);
    let mut username = None;

    // 1. the domain's own file
    for url in autoconfig::urls(email) {
        if imap.is_some() && smtp.is_some() {
            break;
        }
        let Some((xml, from)) = fetch.get_from(&url) else { continue };
        // The file is trusted only when it came from the address's own domain (a redirect that leaves it does not
        // count) and names a server in that domain. Anything else is shown to the person before the password is sent.
        let vouched = url_host(&from).is_some_and(|h| in_domain(&h, &domain));
        let config = autoconfig::parse(&xml, email);
        if imap.is_none() {
            if let Some(server) = config.imap {
                let trusted = vouched && in_domain(&server.host, &domain);
                imap = Some(Part { server, trusted });
                username = config.username;
            }
        }
        if smtp.is_none() {
            smtp = config.smtp.map(|server| {
                let trusted = vouched && in_domain(&server.host, &domain);
                Part { server, trusted }
            });
        }
    }

    // 2. service records
    if imap.is_none() {
        imap = from_srv(lookup, &format!("_imaps._tcp.{domain}"), &domain, imap_security)
            .or_else(|| from_srv(lookup, &format!("_imap._tcp.{domain}"), &domain, imap_security));
    }
    if smtp.is_none() {
        smtp = from_srv(lookup, &format!("_submissions._tcp.{domain}"), &domain, smtp_security)
            .or_else(|| from_srv(lookup, &format!("_submission._tcp.{domain}"), &domain, smtp_security));
    }

    // 3. the mail exchanger. A filtering service or a large provider as exchanger is not an IMAP server, so only a host in
    // the domain itself is taken.
    if imap.is_none() || smtp.is_none() {
        let own = lookup.mx(&domain).into_iter().map(|h| h.trim_end_matches('.').to_ascii_lowercase()).find(|h| in_domain(h, &domain) && lookup.resolves(h));
        if let Some(host) = own {
            if imap.is_none() {
                imap = Some(Part { server: Server { host: host.clone(), port: 993, security: Security::Tls }, trusted: true });
            }
            if smtp.is_none() {
                smtp = Some(Part { server: Server { host, port: 587, security: Security::StartTls }, trusted: true });
            }
        }
    }

    // 4. the conventional names
    if imap.is_none() {
        let host = format!("imap.{domain}");
        if lookup.resolves(&host) {
            imap = Some(Part { server: Server { host, port: 993, security: Security::Tls }, trusted: true });
        }
    }
    if smtp.is_none() {
        let host = format!("smtp.{domain}");
        if lookup.resolves(&host) {
            smtp = Some(Part { server: Server { host, port: 587, security: Security::StartTls }, trusted: true });
        }
    }

    match (imap, smtp) {
        (Some(imap), Some(smtp)) => Ok(Found { trusted: imap.trusted && smtp.trusted, imap: imap.server, smtp: smtp.server, username }),
        (None, _) => Err(format!("Mail could not find the incoming mail server for {domain}")),
        (_, None) => Err(format!("Mail could not find the outgoing mail server for {domain}")),
    }
}

/// Fetches https documents with the shell's client. Redirects must stay on https.
pub struct HttpFetch {
    client: Client,
}

impl HttpFetch {
    pub fn new(transport: Arc<dyn Transport>) -> HttpFetch {
        HttpFetch { client: Client::new(transport, Jar::default()).require_https() }
    }
}

impl Fetch for HttpFetch {
    fn get(&self, url: &str) -> Option<String> {
        self.get_from(url).map(|(body, _)| body)
    }

    fn get_from(&self, url: &str) -> Option<(String, String)> {
        let reply = self.client.get(url, None, FILE_LIMIT).ok()?;
        let from = if reply.url.is_empty() { url.to_string() } else { reply.url.clone() };
        (reply.status == 200).then(|| (String::from_utf8_lossy(&reply.body).into_owned(), from))
    }
}

/// Answers the DNS questions from the name servers of this computer.
pub struct SystemLookup {
    resolver: Resolver,
}

impl SystemLookup {
    pub fn new() -> SystemLookup {
        SystemLookup { resolver: Resolver::system() }
    }
}

impl Default for SystemLookup {
    fn default() -> Self {
        SystemLookup::new()
    }
}

impl Lookup for SystemLookup {
    fn srv(&self, name: &str) -> Vec<Srv> {
        let records = self.resolver.lookup(name, dns::TYPE_SRV).unwrap_or_default();
        dns::order_srv(records.into_iter().filter_map(|r| if let Record::Srv(s) = r { Some(s) } else { None }).collect())
    }

    fn mx(&self, domain: &str) -> Vec<String> {
        let records = self.resolver.lookup(domain, dns::TYPE_MX).unwrap_or_default();
        dns::order_mx(records.into_iter().filter_map(|r| if let Record::Mx(m) = r { Some(m) } else { None }).collect::<Vec<Mx>>())
    }

    fn resolves(&self, host: &str) -> bool {
        self.resolver.resolves(host)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::HashMap;

    #[derive(Default)]
    struct Fake {
        files: HashMap<String, String>,
        srv: HashMap<String, Vec<Srv>>,
        mx: Vec<String>,
        hosts: Vec<String>,
        /// Every question asked, in order.
        asked: RefCell<Vec<String>>,
    }

    impl Fetch for Fake {
        fn get(&self, url: &str) -> Option<String> {
            self.asked.borrow_mut().push(url.to_string());
            self.files.get(url).cloned()
        }
    }

    impl Lookup for Fake {
        fn srv(&self, name: &str) -> Vec<Srv> {
            self.asked.borrow_mut().push(name.to_string());
            self.srv.get(name).cloned().unwrap_or_default()
        }
        fn mx(&self, domain: &str) -> Vec<String> {
            self.asked.borrow_mut().push(format!("MX {domain}"));
            self.mx.clone()
        }
        fn resolves(&self, host: &str) -> bool {
            self.asked.borrow_mut().push(format!("A {host}"));
            self.hosts.iter().any(|h| h == host)
        }
    }

    fn srv(port: u16, target: &str) -> Vec<Srv> {
        vec![Srv { priority: 0, weight: 1, port, target: target.into() }]
    }

    fn server(host: &str, port: u16, security: Security) -> Server {
        Server { host: host.into(), port, security }
    }

    const FILE: &str = "<clientConfig><emailProvider>\
        <incomingServer type=\"imap\"><hostname>mail.example.net</hostname><port>993</port><socketType>SSL</socketType><username>%EMAILADDRESS%</username></incomingServer>\
        <outgoingServer type=\"smtp\"><hostname>mail.example.net</hostname><port>587</port><socketType>STARTTLS</socketType></outgoingServer>\
        </emailProvider></clientConfig>";

    #[test]
    fn the_autoconfig_file_decides_before_dns_is_asked() {
        let mut fake = Fake::default();
        fake.files.insert("https://autoconfig.example.net/mail/config-v1.1.xml?emailaddress=ana@example.net".into(), FILE.into());
        fake.srv.insert("_imaps._tcp.example.net".into(), srv(993, "elsewhere.example.org"));
        let found = discover("ana@example.net", &fake, &fake).unwrap();
        assert_eq!(found.imap, server("mail.example.net", 993, Security::Tls));
        assert_eq!(found.smtp, server("mail.example.net", 587, Security::StartTls));
        assert_eq!(found.username.as_deref(), Some("ana@example.net"));
        assert!(found.trusted);
        assert_eq!(fake.asked.borrow().len(), 1, "one fetch, nothing else");
    }

    #[test]
    fn the_well_known_file_follows_when_the_autoconfig_host_has_none() {
        let mut fake = Fake::default();
        fake.files.insert("https://example.net/.well-known/autoconfig/mail/config-v1.1.xml?emailaddress=ana@example.net".into(), FILE.into());
        let found = discover("ana@example.net", &fake, &fake).unwrap();
        assert_eq!(found.imap.host, "mail.example.net");
        assert_eq!(fake.asked.borrow().len(), 2);
    }

    #[test]
    fn service_records_fill_in_when_there_is_no_file() {
        let mut fake = Fake::default();
        fake.srv.insert("_imaps._tcp.example.net".into(), srv(993, "mail.example.net."));
        fake.srv.insert("_submission._tcp.example.net".into(), srv(587, "mail.example.net."));
        let found = discover("ana@example.net", &fake, &fake).unwrap();
        assert_eq!(found.imap, server("mail.example.net", 993, Security::Tls));
        assert_eq!(found.smtp, server("mail.example.net", 587, Security::StartTls));
        assert_eq!(found.username, None);
        assert!(found.trusted);
        let asked = fake.asked.borrow();
        let at = |what: &str| asked.iter().position(|a| a == what).unwrap_or_else(|| panic!("{what} was not asked"));
        assert!(at("_imaps._tcp.example.net") < at("_submissions._tcp.example.net"));
        assert!(at("_submissions._tcp.example.net") < at("_submission._tcp.example.net"));
        assert!(!asked.iter().any(|a| a.starts_with("MX ") || a.starts_with("A ")), "nothing later is asked once the servers are known");
    }

    #[test]
    fn the_plain_records_are_used_when_the_tls_ones_are_missing() {
        let mut fake = Fake::default();
        fake.srv.insert("_imap._tcp.example.net".into(), srv(143, "mail.example.net"));
        fake.srv.insert("_submissions._tcp.example.net".into(), srv(465, "mail.example.net"));
        let found = discover("ana@example.net", &fake, &fake).unwrap();
        assert_eq!(found.imap, server("mail.example.net", 143, Security::StartTls));
        assert_eq!(found.smtp, server("mail.example.net", 465, Security::Tls));
    }

    #[test]
    fn a_file_with_one_server_is_completed_from_the_records() {
        let mut fake = Fake::default();
        let only_imap = "<incomingServer type=\"imap\"><hostname>mail.example.net</hostname><port>993</port><socketType>SSL</socketType></incomingServer>";
        fake.files.insert("https://autoconfig.example.net/mail/config-v1.1.xml?emailaddress=ana@example.net".into(), only_imap.into());
        fake.srv.insert("_submission._tcp.example.net".into(), srv(587, "smtp.example.net"));
        let found = discover("ana@example.net", &fake, &fake).unwrap();
        assert_eq!(found.smtp, server("smtp.example.net", 587, Security::StartTls));
    }

    #[test]
    fn the_mail_exchanger_is_a_last_guess_and_must_be_in_the_domain() {
        let mut fake = Fake::default();
        fake.mx = vec!["aspmx.l.example.org".into(), "mx1.example.net.".into()];
        fake.hosts = vec!["mx1.example.net".into()];
        let found = discover("ana@example.net", &fake, &fake).unwrap();
        assert_eq!(found.imap, server("mx1.example.net", 993, Security::Tls));
        assert_eq!(found.smtp, server("mx1.example.net", 587, Security::StartTls));
        assert!(found.trusted);
    }

    #[test]
    fn the_conventional_names_come_last_and_only_when_they_exist() {
        let mut fake = Fake::default();
        fake.hosts = vec!["imap.example.net".into(), "smtp.example.net".into()];
        let found = discover("ana@example.net", &fake, &fake).unwrap();
        assert_eq!(found.imap.host, "imap.example.net");
        assert_eq!(found.smtp.host, "smtp.example.net");
        let asked = fake.asked.borrow();
        assert!(asked.iter().position(|a| a == "MX example.net") < asked.iter().position(|a| a == "A imap.example.net"));
    }

    #[test]
    fn a_server_that_dns_names_outside_the_domain_is_not_trusted() {
        let mut fake = Fake::default();
        fake.srv.insert("_imaps._tcp.example.net".into(), srv(993, "imap.host.example.org"));
        fake.srv.insert("_submission._tcp.example.net".into(), srv(587, "smtp.host.example.org"));
        let found = discover("ana@example.net", &fake, &fake).unwrap();
        assert_eq!(found.imap.host, "imap.host.example.org");
        assert!(!found.trusted, "the person confirms it before the password goes there");
    }

    #[test]
    fn an_autoconfig_server_outside_the_domain_is_not_trusted() {
        let mut fake = Fake::default();
        let off = FILE.replace("mail.example.net", "mail.example.org");
        fake.files.insert("https://autoconfig.example.net/mail/config-v1.1.xml?emailaddress=ana@example.net".into(), off);
        let found = discover("ana@example.net", &fake, &fake).unwrap();
        assert_eq!(found.imap.host, "mail.example.org");
        assert!(!found.trusted, "the person confirms it before the password goes there");
    }

    /// A fetcher that answers every address from another domain, as after a redirect.
    struct Redirected;

    impl Fetch for Redirected {
        fn get(&self, _: &str) -> Option<String> {
            Some(FILE.to_string())
        }
        fn get_from(&self, _: &str) -> Option<(String, String)> {
            Some((FILE.to_string(), "https://config.example.org/config.xml".to_string()))
        }
    }

    #[test]
    fn a_file_reached_through_a_cross_domain_redirect_is_not_trusted() {
        let none = Fake::default();
        let found = discover("ana@example.net", &Redirected, &none).unwrap();
        assert_eq!(found.imap.host, "mail.example.net");
        assert!(!found.trusted);
    }

    #[test]
    fn the_host_of_an_address_is_read() {
        assert_eq!(url_host("https://Autoconfig.Example.net:443/a?b=c").as_deref(), Some("autoconfig.example.net"));
        assert_eq!(url_host("https://example.net@evil.org/x").as_deref(), Some("evil.org"));
        assert_eq!(url_host("https://"), None);
    }

    #[test]
    fn nothing_found_is_a_plain_error() {
        let fake = Fake::default();
        let err = discover("ana@example.net", &fake, &fake).unwrap_err();
        assert_eq!(err, "Mail could not find the incoming mail server for example.net");
        let mut half = Fake::default();
        half.srv.insert("_imaps._tcp.example.net".into(), srv(993, "mail.example.net"));
        assert_eq!(discover("ana@example.net", &half, &half).unwrap_err(), "Mail could not find the outgoing mail server for example.net");
    }

    #[test]
    fn an_address_that_cannot_be_a_mail_domain_asks_for_nothing() {
        let fake = Fake::default();
        for email in ["ana@localhost", "ana@exa mple.net", "ana@exa/mple.net", "ana"] {
            assert!(discover(email, &fake, &fake).is_err(), "{email}");
        }
        assert!(fake.asked.borrow().is_empty(), "no request is made for a bad domain");
    }

    #[test]
    fn domains_match_whole_labels() {
        assert!(in_domain("mail.example.net", "example.net") && in_domain("EXAMPLE.net.", "example.net"));
        assert!(!in_domain("badexample.net", "example.net") && !in_domain("example.net.evil.org", "example.net"));
    }
}
