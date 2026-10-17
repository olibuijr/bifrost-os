//! Account suggestions from the person's mail address: which account type fits the address, and the mail servers to
//! suggest for it. It works on the device only. The answers come from a built-in table of well-known providers, or
//! from the address's own domain (`imap.` and `smtp.` in front of it, as Settings > Internet Accounts already does).
//! Nothing is looked up over the network, so the address never leaves the machine to find a suggestion.
//!
//! The suggestion is shown as text next to the account's user-name step; the person still decides what to save.

use crate::internet_accounts::{Kind, Security, Server};

/// One well-known provider: the domains it serves and its servers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Provider {
    pub domains: &'static [&'static str],
    pub name: &'static str,
    pub imap: (&'static str, u16, Security),
    pub smtp: (&'static str, u16, Security),
}

/// The built-in provider table. A domain not listed here gets the `imap.` and `smtp.` guess.
pub const PROVIDERS: [Provider; 5] = [
    Provider {
        domains: &["gmail.com", "googlemail.com"],
        name: "Gmail",
        imap: ("imap.gmail.com", 993, Security::Tls),
        smtp: ("smtp.gmail.com", 465, Security::Tls),
    },
    Provider {
        domains: &["outlook.com", "hotmail.com", "live.com"],
        name: "Outlook",
        imap: ("outlook.office365.com", 993, Security::Tls),
        smtp: ("smtp.office365.com", 587, Security::StartTls),
    },
    Provider {
        domains: &["icloud.com", "me.com"],
        name: "iCloud Mail",
        imap: ("imap.mail.me.com", 993, Security::Tls),
        smtp: ("smtp.mail.me.com", 587, Security::StartTls),
    },
    Provider {
        domains: &["yahoo.com"],
        name: "Yahoo Mail",
        imap: ("imap.mail.yahoo.com", 993, Security::Tls),
        smtp: ("smtp.mail.yahoo.com", 465, Security::Tls),
    },
    Provider {
        domains: &["fastmail.com"],
        name: "Fastmail",
        imap: ("imap.fastmail.com", 993, Security::Tls),
        smtp: ("smtp.fastmail.com", 465, Security::Tls),
    },
];

/// What to suggest for one address.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Suggestion {
    /// The account type the address fits (`bifrost.mail` for every address).
    pub account_type: &'static str,
    /// The provider's name when the domain is in the built-in table; `None` for the guess.
    pub provider: Option<&'static str>,
    pub domain: String,
    pub imap: Server,
    pub smtp: Server,
}

/// The domain of a well-formed address, lower-cased.
pub fn domain_of(address: &str) -> Option<String> {
    let (local, domain) = address.trim().split_once('@')?;
    (!local.is_empty() && !domain.is_empty() && !domain.contains('@') && domain.contains('.')).then(|| domain.to_ascii_lowercase())
}

/// The suggestion for an address, or `None` when the address is not well formed.
pub fn for_address(address: &str) -> Option<Suggestion> {
    let domain = domain_of(address)?;
    let known = PROVIDERS.iter().find(|p| p.domains.contains(&domain.as_str()));
    let (imap, smtp, provider) = match known {
        Some(p) => (server(p.imap.0, p.imap.1, p.imap.2), server(p.smtp.0, p.smtp.1, p.smtp.2), Some(p.name)),
        None => (server(&format!("imap.{domain}"), 993, Security::Tls), server(&format!("smtp.{domain}"), 587, Security::StartTls), None),
    };
    Some(Suggestion { account_type: Kind::Mail.account_type(), provider, domain, imap, smtp })
}

fn server(host: &str, port: u16, security: Security) -> Server {
    Server { host: host.to_string(), port, security }
}

/// The suggestion as one line of text for the person to read.
pub fn describe(s: &Suggestion) -> String {
    let source = match s.provider {
        Some(name) => format!("{name} settings"),
        None => "Guessed from the domain".to_string(),
    };
    format!(
        "Suggested for {}: IMAP {}:{}, SMTP {}:{}. {} on this device; nothing is looked up online.",
        s.domain, s.imap.host, s.imap.port, s.smtp.host, s.smtp.port, source
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_mail_address_fits_the_mail_account_type() {
        let s = for_address("ola@example.is").unwrap();
        assert_eq!(s.account_type, "bifrost.mail");
        assert_eq!(s.domain, "example.is");
    }

    #[test]
    fn a_known_provider_gets_its_own_servers() {
        let s = for_address("Ola@GMail.com").unwrap();
        assert_eq!(s.provider, Some("Gmail"));
        assert_eq!(s.imap, Server { host: "imap.gmail.com".into(), port: 993, security: Security::Tls });
        assert_eq!(s.smtp.port, 465);
    }

    #[test]
    fn an_unknown_domain_gets_the_guess_the_shell_already_uses() {
        let s = for_address("ola@example.is").unwrap();
        assert_eq!(s.provider, None);
        assert_eq!(s.imap.host, "imap.example.is");
        assert_eq!(s.smtp, Server { host: "smtp.example.is".into(), port: 587, security: Security::StartTls });
    }

    #[test]
    fn a_malformed_address_gets_no_suggestion() {
        assert_eq!(for_address("no-at-sign"), None);
        assert_eq!(for_address("@example.is"), None);
        assert_eq!(for_address("ola@"), None);
        assert_eq!(for_address("ola@localhost"), None, "a domain needs a dot");
        assert_eq!(for_address("a@b@example.is"), None);
    }

    #[test]
    fn the_description_says_where_the_settings_came_from() {
        let text = describe(&for_address("ola@gmail.com").unwrap());
        assert!(text.contains("imap.gmail.com:993"));
        assert!(text.contains("Gmail settings on this device"));
        assert!(describe(&for_address("ola@example.is").unwrap()).contains("Guessed from the domain"));
    }

    #[test]
    fn every_provider_domain_is_listed_once() {
        let mut seen: Vec<&str> = PROVIDERS.iter().flat_map(|p| p.domains.iter().copied()).collect();
        let n = seen.len();
        seen.sort();
        seen.dedup();
        assert_eq!(seen.len(), n);
    }
}
