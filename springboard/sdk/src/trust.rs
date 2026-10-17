//! Certificate trust evaluation for apps (`SecTrustEvaluateWithError`, `SecPolicyCreateSSL`, `SecCertificate`).
//!
//! This is the pure model: an app hands over a certificate chain it received, the anchors it trusts and a policy,
//! and gets a [`Verdict`]. The checks are the ones a TLS client needs: every certificate is inside its validity
//! window, the leaf names the host (exact or one-label wildcard), each certificate is issued by the next one and
//! the issuers are certificate authorities, and the chain ends at a trusted anchor. The model does not parse DER
//! and does not verify signatures; the caller fills [`Certificate`] from parsed fields and must check signatures
//! before trusting a verdict. Nothing is read from disk or the network.

/// One certificate, reduced to the fields trust evaluation uses. Times are seconds since the Unix epoch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Certificate {
    /// Distinguished name of the subject, as one string.
    pub subject: String,
    /// Distinguished name of the issuer, as one string.
    pub issuer: String,
    /// The subject alternative DNS names (for example `example.com` or `*.example.com`).
    pub dns_names: Vec<String>,
    pub not_before: u64,
    pub not_after: u64,
    /// True when the basic constraints mark the certificate as a certificate authority.
    pub is_ca: bool,
}

impl Certificate {
    /// The trust fields of a certificate parsed from its DER (`bifrost_core::x509::parse`): names as RFC 4514 strings,
    /// the DNS subject alternative names, the validity window and the authority flag from basic constraints. Signatures
    /// are not checked here.
    pub fn from_x509(cert: &bifrost_core::x509::Certificate) -> Result<Certificate, bifrost_core::x509::X509Error> {
        Ok(Certificate {
            subject: cert.subject.to_string(),
            issuer: cert.issuer.to_string(),
            dns_names: cert.dns_names()?,
            not_before: cert.not_before,
            not_after: cert.not_after,
            is_ca: cert.can_sign_certificates()?,
        })
    }
}

/// What the chain is checked for. `ssl_server` names a host the leaf must cover.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Policy {
    pub host: Option<String>,
}

impl Policy {
    /// `SecPolicyCreateSSL(true, host)`: a TLS server certificate for `host`.
    pub fn ssl_server(host: &str) -> Policy {
        Policy { host: Some(host.to_string()) }
    }

    /// A policy that checks the chain and trust but no host name.
    pub fn basic() -> Policy {
        Policy { host: None }
    }
}

/// Why a chain was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reason {
    /// The chain is empty.
    NoCertificates,
    /// `now` is after the `not_after` of a certificate in the chain.
    Expired,
    /// `now` is before the `not_before` of a certificate in the chain.
    NotYetValid,
    /// The leaf does not cover the policy host.
    HostMismatch,
    /// A certificate is not issued by the next certificate in the chain.
    BrokenChain,
    /// A certificate above the leaf is not marked as a certificate authority.
    NotCertificateAuthority,
    /// The chain does not end at a trusted anchor.
    UntrustedRoot,
}

/// The result of a trust evaluation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    Trusted,
    Refused(Reason),
}

impl Verdict {
    pub fn is_trusted(&self) -> bool {
        matches!(self, Verdict::Trusted)
    }
}

/// True when `host` is covered by `pattern`. A pattern is a host name, or `*.` followed by a name: the wildcard
/// stands for exactly one leading label, so `*.example.com` covers `a.example.com` but not `example.com` or
/// `a.b.example.com`. Comparison ignores ASCII case.
pub fn host_matches(pattern: &str, host: &str) -> bool {
    let pattern = pattern.to_ascii_lowercase();
    let host = host.to_ascii_lowercase();
    if host.is_empty() {
        return false;
    }
    match pattern.strip_prefix("*.") {
        Some(suffix) => match host.split_once('.') {
            Some((label, rest)) => !label.is_empty() && rest == suffix,
            None => false,
        },
        None => pattern == host,
    }
}

fn valid_at(cert: &Certificate, now: u64) -> Result<(), Reason> {
    if now < cert.not_before {
        Err(Reason::NotYetValid)
    } else if now > cert.not_after {
        Err(Reason::Expired)
    } else {
        Ok(())
    }
}

/// Evaluates `chain` (leaf first) against `anchors` under `policy` at time `now`.
pub fn evaluate(chain: &[Certificate], anchors: &[Certificate], policy: &Policy, now: u64) -> Verdict {
    match check(chain, anchors, policy, now) {
        Ok(()) => Verdict::Trusted,
        Err(reason) => Verdict::Refused(reason),
    }
}

fn check(chain: &[Certificate], anchors: &[Certificate], policy: &Policy, now: u64) -> Result<(), Reason> {
    let leaf = chain.first().ok_or(Reason::NoCertificates)?;
    for cert in chain {
        valid_at(cert, now)?;
    }
    if let Some(host) = &policy.host {
        if !leaf.dns_names.iter().any(|name| host_matches(name, host)) {
            return Err(Reason::HostMismatch);
        }
    }
    for pair in chain.windows(2) {
        let (child, issuer) = (&pair[0], &pair[1]);
        if child.issuer != issuer.subject {
            return Err(Reason::BrokenChain);
        }
        if !issuer.is_ca {
            return Err(Reason::NotCertificateAuthority);
        }
    }
    // The top of the chain is trusted when it is itself an anchor, or when a trusted anchor issued it.
    let top = chain.last().ok_or(Reason::NoCertificates)?;
    let trusted = anchors.iter().any(|anchor| {
        anchor == top || (anchor.is_ca && anchor.subject == top.issuer && valid_at(anchor, now).is_ok())
    });
    if trusted {
        Ok(())
    } else {
        Err(Reason::UntrustedRoot)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: u64 = 1_800_000_000;
    const DAY: u64 = 86_400;

    fn leaf(host: &str) -> Certificate {
        Certificate {
            subject: format!("CN={host}"),
            issuer: "CN=Test Intermediate".into(),
            dns_names: vec![host.into()],
            not_before: NOW - DAY,
            not_after: NOW + DAY,
            is_ca: false,
        }
    }

    fn intermediate() -> Certificate {
        Certificate {
            subject: "CN=Test Intermediate".into(),
            issuer: "CN=Test Root".into(),
            dns_names: vec![],
            not_before: NOW - 10 * DAY,
            not_after: NOW + 10 * DAY,
            is_ca: true,
        }
    }

    fn root() -> Certificate {
        Certificate {
            subject: "CN=Test Root".into(),
            issuer: "CN=Test Root".into(),
            dns_names: vec![],
            not_before: NOW - 100 * DAY,
            not_after: NOW + 100 * DAY,
            is_ca: true,
        }
    }

    #[test]
    fn a_chain_to_a_trusted_root_is_trusted_for_its_host() {
        let chain = [leaf("shop.example"), intermediate()];
        let verdict = evaluate(&chain, &[root()], &Policy::ssl_server("shop.example"), NOW);
        assert_eq!(verdict, Verdict::Trusted);
        assert!(verdict.is_trusted());
    }

    #[test]
    fn an_anchor_that_is_the_leaf_itself_is_trusted() {
        let cert = leaf("pinned.example");
        assert_eq!(evaluate(&[cert.clone()], &[cert], &Policy::ssl_server("pinned.example"), NOW), Verdict::Trusted);
    }

    #[test]
    fn an_empty_chain_has_no_certificates() {
        assert_eq!(evaluate(&[], &[root()], &Policy::basic(), NOW), Verdict::Refused(Reason::NoCertificates));
    }

    #[test]
    fn an_expired_or_future_certificate_is_refused() {
        let mut old = leaf("shop.example");
        old.not_after = NOW - 1;
        let chain = [old, intermediate()];
        assert_eq!(evaluate(&chain, &[root()], &Policy::basic(), NOW), Verdict::Refused(Reason::Expired));
        let mut future = leaf("shop.example");
        future.not_before = NOW + 1;
        let chain = [future, intermediate()];
        assert_eq!(evaluate(&chain, &[root()], &Policy::basic(), NOW), Verdict::Refused(Reason::NotYetValid));
    }

    #[test]
    fn a_leaf_for_another_host_is_refused() {
        let chain = [leaf("shop.example"), intermediate()];
        assert_eq!(
            evaluate(&chain, &[root()], &Policy::ssl_server("bank.example"), NOW),
            Verdict::Refused(Reason::HostMismatch)
        );
    }

    #[test]
    fn a_broken_link_is_refused() {
        let mut other = intermediate();
        other.subject = "CN=Someone Else".into();
        let chain = [leaf("shop.example"), other];
        assert_eq!(evaluate(&chain, &[root()], &Policy::basic(), NOW), Verdict::Refused(Reason::BrokenChain));
    }

    #[test]
    fn an_issuer_that_is_not_a_ca_is_refused() {
        let mut issuer = intermediate();
        issuer.is_ca = false;
        let chain = [leaf("shop.example"), issuer];
        assert_eq!(
            evaluate(&chain, &[root()], &Policy::basic(), NOW),
            Verdict::Refused(Reason::NotCertificateAuthority)
        );
    }

    #[test]
    fn a_chain_without_a_trusted_anchor_is_refused() {
        let chain = [leaf("shop.example"), intermediate()];
        assert_eq!(evaluate(&chain, &[], &Policy::basic(), NOW), Verdict::Refused(Reason::UntrustedRoot));
        let mut other_root = root();
        other_root.subject = "CN=Other Root".into();
        assert_eq!(
            evaluate(&chain, &[other_root], &Policy::basic(), NOW),
            Verdict::Refused(Reason::UntrustedRoot)
        );
    }

    #[test]
    fn wildcards_cover_exactly_one_label() {
        assert!(host_matches("*.example.com", "a.example.com"));
        assert!(host_matches("*.Example.COM", "a.example.com"));
        assert!(!host_matches("*.example.com", "example.com"));
        assert!(!host_matches("*.example.com", "a.b.example.com"));
        assert!(!host_matches("*.example.com", ".example.com"));
        assert!(host_matches("example.com", "EXAMPLE.com"));
        assert!(!host_matches("example.com", ""));
    }

    #[test]
    fn a_wildcard_leaf_covers_its_hosts() {
        let mut cert = leaf("*.shop.example");
        cert.dns_names = vec!["*.shop.example".into()];
        let chain = [cert, intermediate()];
        assert_eq!(
            evaluate(&chain, &[root()], &Policy::ssl_server("cart.shop.example"), NOW),
            Verdict::Trusted
        );
    }

    #[test]
    fn a_chain_parsed_from_der_is_evaluated_by_the_same_rules() {
        use bifrost_core::der;
        use bifrost_core::oid::Oid;
        use bifrost_core::x509::{encode, parse, Template};
        let alg = Oid::parse("1.2.840.10045.4.3.2").unwrap();
        let spki = der::sequence(&[der::sequence(&[der::oid(&Oid::parse("1.2.840.10045.2.1").unwrap()).unwrap()]), der::bit_string(0, &[4u8; 65])]);
        let make = |issuer: &str, subject: &str, dns: &[&str], authority: bool| {
            let (i, s) = ([("CN", issuer)], [("CN", subject)]);
            let t = Template { serial: &[1], signature_algorithm: &alg, issuer: &i, subject: &s, not_before: NOW - DAY, not_after: NOW + DAY, spki: &spki, dns_names: dns, authority };
            Certificate::from_x509(&parse(&encode(&t, &[1]).unwrap()).unwrap()).unwrap()
        };
        let chain = [make("Test Root", "www.example.is", &["www.example.is"], false), make("Test Root", "Test Root", &[], true)];
        assert_eq!(chain[0].subject, "CN=www.example.is");
        assert_eq!(chain[0].dns_names, vec!["www.example.is".to_string()]);
        assert!(!chain[0].is_ca && chain[1].is_ca);
        let anchors = [chain[1].clone()];
        assert_eq!(evaluate(&chain, &anchors, &Policy::ssl_server("www.example.is"), NOW), Verdict::Trusted);
        assert_eq!(evaluate(&chain, &anchors, &Policy::ssl_server("other.example.is"), NOW), Verdict::Refused(Reason::HostMismatch));
    }
}
