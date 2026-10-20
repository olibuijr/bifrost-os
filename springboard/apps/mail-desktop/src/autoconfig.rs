//! The autoconfig file a mail domain publishes (the `config-v1.1.xml` format): where to fetch it for an address and how to
//! read the incoming and outgoing servers out of it.

use bifrost_app::controls::internet_accounts::{Security, Server};

/// The servers a file describes and the user name pattern of the incoming one, already filled in for the address.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Config {
    pub imap: Option<Server>,
    pub smtp: Option<Server>,
    /// The login name the file asks for. None when the file does not say; the address is used then.
    pub username: Option<String>,
}

/// The addresses to try for `email`, in order: the file the domain's `autoconfig` host serves, then the one under
/// `.well-known`. Both are https only: the person's address goes in the first, and what comes back decides where the
/// password is sent.
pub fn urls(email: &str) -> Vec<String> {
    let domain = email.rsplit_once('@').map_or("", |(_, d)| d).to_ascii_lowercase();
    let query = escape_query(email);
    vec![
        format!("https://autoconfig.{domain}/mail/config-v1.1.xml?emailaddress={query}"),
        format!("https://{domain}/.well-known/autoconfig/mail/config-v1.1.xml?emailaddress={query}"),
    ]
}

/// Percent-encodes what would end or change a query value. `@` stays: it is allowed in a query and servers expect it.
fn escape_query(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for b in text.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'@' | b'.' | b'-' | b'_' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// Reads an autoconfig file for `email`: the first IMAP server and the first SMTP server that use TLS. A server without
/// encryption is not taken, because the password would cross the network in the clear. A file that is not autoconfig
/// gives an empty [`Config`].
pub fn parse(xml: &str, email: &str) -> Config {
    let xml = strip_comments(xml);
    let mut config = Config::default();
    for (attrs, inner) in blocks(&xml, "incomingServer") {
        if config.imap.is_some() || attribute(&attrs, "type").as_deref() != Some("imap") {
            continue;
        }
        if let Some(server) = server_of(&inner, email, 993, 143) {
            config.imap = Some(server);
            config.username = child(&inner, "username").map(|u| fill(&u, email)).filter(|u| valid_username(u));
        }
    }
    for (attrs, inner) in blocks(&xml, "outgoingServer") {
        if config.smtp.is_some() || attribute(&attrs, "type").as_deref() != Some("smtp") {
            continue;
        }
        config.smtp = server_of(&inner, email, 465, 587);
    }
    config
}

fn valid_username(name: &str) -> bool {
    !name.is_empty() && !name.contains(['%', '\r', '\n', '\0'])
}

/// The server a `<incomingServer>` or `<outgoingServer>` body describes. `tls_port` and `start_port` are the ports used
/// when the file leaves the port out.
fn server_of(inner: &str, email: &str, tls_port: u16, start_port: u16) -> Option<Server> {
    let security = match child(inner, "socketType")?.to_ascii_uppercase().as_str() {
        "SSL" | "TLS" => Security::Tls,
        "STARTTLS" => Security::StartTls,
        _ => return None,
    };
    let host = fill(&child(inner, "hostname")?, email).to_ascii_lowercase();
    if !valid_host(&host) {
        return None;
    }
    let port = match child(inner, "port") {
        Some(p) => p.trim().parse::<u16>().ok().filter(|&p| p != 0)?,
        None if security == Security::Tls => tls_port,
        None => start_port,
    };
    Some(Server { host, port, security })
}

/// A host name: letters, digits, dots, hyphens and underscores, in labels that are not empty.
pub fn valid_host(host: &str) -> bool {
    !host.is_empty()
        && host.len() <= 253
        && host.split('.').all(|label| !label.is_empty() && label.len() <= 63)
        && host.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'))
}

/// Replaces the placeholders of the format (in any case) with the parts of the address. Text put in is not read again.
fn fill(text: &str, email: &str) -> String {
    let (local, domain) = email.rsplit_once('@').unwrap_or((email, ""));
    let keys = [("%EMAILADDRESS%", email), ("%EMAILLOCALPART%", local), ("%EMAILDOMAIN%", domain)];
    let mut out = String::with_capacity(text.len());
    let mut rest = text.trim();
    while let Some(at) = rest.find('%') {
        out.push_str(&rest[..at]);
        let tail = &rest[at..];
        match keys.iter().find(|(key, _)| tail.get(..key.len()).is_some_and(|head| head.eq_ignore_ascii_case(key))) {
            Some((key, value)) => {
                out.push_str(value);
                rest = &tail[key.len()..];
            }
            None => {
                out.push('%');
                rest = &tail[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

fn strip_comments(xml: &str) -> String {
    let mut out = String::with_capacity(xml.len());
    let mut rest = xml;
    while let Some(start) = rest.find("<!--") {
        out.push_str(&rest[..start]);
        match rest[start..].find("-->") {
            Some(end) => rest = &rest[start + end + 3..],
            None => return out,
        }
    }
    out.push_str(rest);
    out
}

/// The elements named `tag`: each one's attribute text and body. Elements do not nest in the format, so a body ends at
/// the first closing tag.
fn blocks(xml: &str, tag: &str) -> Vec<(String, String)> {
    let open = format!("<{tag}");
    let close = format!("</{tag}>");
    let mut out = Vec::new();
    let mut rest = xml;
    while let Some(at) = rest.find(&open) {
        let after = &rest[at + open.len()..];
        // `<incomingServerX` is another element
        if !after.starts_with(|c: char| c.is_whitespace() || c == '>') {
            rest = after;
            continue;
        }
        let Some(head_end) = after.find('>') else { break };
        let attrs = after[..head_end].to_string();
        let body = &after[head_end + 1..];
        let Some(end) = body.find(&close) else { break };
        out.push((attrs, body[..end].to_string()));
        rest = &body[end + close.len()..];
    }
    out
}

/// The value of attribute `name` in the text of a start tag.
fn attribute(attrs: &str, name: &str) -> Option<String> {
    let mut rest = attrs;
    while let Some(at) = rest.find(name) {
        let before_ok = rest[..at].ends_with(|c: char| c.is_whitespace()) || at == 0;
        let after = rest[at + name.len()..].trim_start();
        if before_ok {
            if let Some(value) = after.strip_prefix('=') {
                let value = value.trim_start();
                let quote = value.chars().next().filter(|c| *c == '"' || *c == '\'')?;
                let inner = &value[1..];
                return inner.find(quote).map(|end| inner[..end].to_string());
            }
        }
        rest = &rest[at + name.len()..];
    }
    None
}

/// The text of the first `<tag>` inside `inner`, with the XML escapes and CDATA undone.
fn child(inner: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = inner.find(&open)? + open.len();
    let end = inner[start..].find(&close)? + start;
    Some(unescape(inner[start..end].trim()))
}

fn unescape(text: &str) -> String {
    let text = text.strip_prefix("<![CDATA[").and_then(|t| t.strip_suffix("]]>")).unwrap_or(text);
    text.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'").replace("&amp;", "&")
}

#[cfg(test)]
mod tests {
    use super::*;

    const FILE: &str = r#"<?xml version="1.0"?>
<clientConfig version="1.1">
  <!-- the pop3 server comes first on purpose -->
  <emailProvider id="example.net">
    <domain>example.net</domain>
    <incomingServer type="pop3">
      <hostname>pop.example.net</hostname><port>995</port><socketType>SSL</socketType>
      <username>%EMAILADDRESS%</username>
    </incomingServer>
    <incomingServer type="imap">
      <hostname>mail.example.net</hostname>
      <port>993</port>
      <socketType>SSL</socketType>
      <authentication>password-cleartext</authentication>
      <username>%EMAILLOCALPART%</username>
    </incomingServer>
    <incomingServer type="imap">
      <hostname>mail.example.net</hostname><port>143</port><socketType>STARTTLS</socketType>
    </incomingServer>
    <outgoingServer type="smtp">
      <hostname>mail.example.net</hostname>
      <port>587</port>
      <socketType>STARTTLS</socketType>
      <username>%EMAILADDRESS%</username>
    </outgoingServer>
  </emailProvider>
</clientConfig>"#;

    fn server(host: &str, port: u16, security: Security) -> Server {
        Server { host: host.into(), port, security }
    }

    #[test]
    fn the_domains_file_gives_both_servers_and_the_login() {
        let config = parse(FILE, "ana@example.net");
        assert_eq!(config.imap, Some(server("mail.example.net", 993, Security::Tls)));
        assert_eq!(config.smtp, Some(server("mail.example.net", 587, Security::StartTls)));
        assert_eq!(config.username.as_deref(), Some("ana"));
    }

    #[test]
    fn the_address_placeholders_are_filled_in() {
        let xml = r#"<incomingServer type="imap"><hostname>imap.%EMAILDOMAIN%</hostname><socketType>SSL</socketType>
            <username>%EMAILADDRESS%</username></incomingServer>
            <outgoingServer type="smtp"><hostname>smtp.%emaildomain%</hostname><socketType>SSL</socketType></outgoingServer>"#;
        let config = parse(xml, "ana@Example.net");
        assert_eq!(config.imap, Some(server("imap.example.net", 993, Security::Tls)), "the port follows the encryption");
        assert_eq!(config.smtp, Some(server("smtp.example.net", 465, Security::Tls)));
        assert_eq!(config.username.as_deref(), Some("ana@Example.net"));
    }

    #[test]
    fn a_server_without_encryption_is_not_taken() {
        let xml = r#"<incomingServer type="imap"><hostname>mail.example.net</hostname><port>143</port><socketType>plain</socketType></incomingServer>
            <outgoingServer type="smtp"><hostname>mail.example.net</hostname><port>25</port><socketType>plain</socketType></outgoingServer>"#;
        assert_eq!(parse(xml, "ana@example.net"), Config::default());
    }

    #[test]
    fn a_bad_host_or_port_is_skipped_for_the_next_server() {
        let xml = r#"<incomingServer type="imap"><hostname>bad host</hostname><port>993</port><socketType>SSL</socketType></incomingServer>
            <incomingServer type="imap"><hostname>x.example.net</hostname><port>0</port><socketType>SSL</socketType></incomingServer>
            <incomingServer type="imap"><hostname>ok.example.net</hostname><port>993</port><socketType>SSL</socketType></incomingServer>"#;
        assert_eq!(parse(xml, "ana@example.net").imap, Some(server("ok.example.net", 993, Security::Tls)));
    }

    #[test]
    fn text_that_is_not_autoconfig_gives_nothing() {
        assert_eq!(parse("<html><body>Not found</body></html>", "ana@example.net"), Config::default());
        assert_eq!(parse("", "ana@example.net"), Config::default());
        assert_eq!(parse("<incomingServer type=\"imap\"><hostname>x", "ana@example.net"), Config::default());
    }

    #[test]
    fn escapes_and_cdata_in_values_are_undone() {
        let xml = "<incomingServer type='imap'><hostname><![CDATA[mail.example.net]]></hostname><socketType>SSL</socketType>\
                   <username>a&amp;b</username></incomingServer>";
        let config = parse(xml, "ana@example.net");
        assert_eq!(config.imap.map(|s| s.host), Some("mail.example.net".to_string()));
        assert_eq!(config.username.as_deref(), Some("a&b"));
    }

    #[test]
    fn the_file_is_asked_for_over_https_in_two_places() {
        assert_eq!(
            urls("ana+news@Example.net"),
            [
                "https://autoconfig.example.net/mail/config-v1.1.xml?emailaddress=ana%2Bnews@Example.net",
                "https://example.net/.well-known/autoconfig/mail/config-v1.1.xml?emailaddress=ana%2Bnews@Example.net",
            ]
        );
    }

    #[test]
    fn host_names_are_checked() {
        assert!(valid_host("mail.example.net") && valid_host("a-b_c.example"));
        assert!(!valid_host("") && !valid_host("a..b") && !valid_host("a b") && !valid_host("a/b") && !valid_host(".example"));
    }
}
