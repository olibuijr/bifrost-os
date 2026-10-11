//! Certificate chain and trust evaluation.
//!
//! A chain of [`Certificate`]s, leaf first, is checked against a set of trust anchors under a [`Policy`]. The
//! checks are the ones a TLS client or a signing tool needs: each certificate is valid at the given time, every
//! certificate above the leaf is a CA, path lengths hold, each certificate is issued by the next one, the root is
//! an anchor, the leaf names the host, and the leaf may sign when the policy asks for it. The model reads no
//! certificate bytes and does no cryptography. The caller parses the certificates and passes the signature check
//! as a closure; the model decides the rest from the fields it is given and the time it is given (Unix seconds).
//!
//! [`Verdict`] mirrors the outcome categories of a trust evaluation. The model never returns "proceed": that
//! outcome means the user chose to trust a certificate, and the caller records that choice itself.
//!
//! [`CertificateView`] is the display model of one certificate: a summary that is always shown, and details that
//! the user can disclose.

/// One certificate, as far as trust evaluation needs it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Certificate {
    /// Subject name, as the issuer writes it in the issuer field of the certificates it signs.
    pub subject: String,
    /// Issuer name.
    pub issuer: String,
    /// Start of validity, in Unix seconds.
    pub not_before: u64,
    /// End of validity, in Unix seconds.
    pub not_after: u64,
    /// Basic constraints: the key may sign other certificates.
    pub is_ca: bool,
    /// Basic constraints path length: how many intermediate CAs may sit below this one. `None` is no limit.
    pub path_len: Option<u8>,
    /// Key usage: the key may create digital signatures.
    pub digital_signature: bool,
    /// Subject alternative DNS names. A name may start with a wildcard label, such as `*.example.com`.
    pub dns_names: Vec<String>,
}

impl Certificate {
    /// Whether the subject and the issuer are the same name. The check is by name only.
    pub fn is_self_signed(&self) -> bool {
        self.subject == self.issuer
    }

    /// Whether `now` lies inside the validity period, both ends included.
    pub fn valid_at(&self, now: u64) -> bool {
        self.not_before <= now && now <= self.not_after
    }
}

/// What a chain is evaluated for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Policy {
    /// Validity, chain, CA and path rules only. No name or key usage check.
    Basic,
    /// Everything in `Basic`, plus the leaf must name `host` and must be allowed to sign.
    Ssl { host: String },
}

/// One reason a chain fails. Indexes count from the leaf, which is index 0.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Failure {
    /// The chain has no certificate.
    EmptyChain,
    /// `now` is after the certificate's end of validity.
    Expired { index: usize },
    /// `now` is before the certificate's start of validity.
    NotYetValid { index: usize },
    /// The certificate's issuer name is not the next certificate's subject.
    BrokenChain { index: usize },
    /// The certificate's signature does not verify under its issuer's key.
    SignatureInvalid { index: usize },
    /// A certificate above the leaf is not a CA.
    NotCa { index: usize },
    /// A CA's path length is smaller than the number of CAs below it.
    PathTooLong { index: usize },
    /// The leaf may not create digital signatures, which the policy needs.
    KeyUsage { index: usize },
    /// The leaf names no host that matches the policy's host.
    NameMismatch,
    /// The root is neither an anchor nor issued by one.
    UntrustedRoot,
}

/// The outcome of a trust evaluation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// The chain is empty.
    Invalid,
    /// No failures: the chain is valid and the user has not yet chosen to trust it.
    Unspecified,
    /// The only failure is an untrusted root. The user may choose to trust it.
    RecoverableTrustFailure,
    /// At least one failure that the user cannot override.
    FatalTrustFailure,
}

/// The result of [`evaluate`]: a verdict and every failure found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Evaluation {
    pub verdict: Verdict,
    pub failures: Vec<Failure>,
}

impl Evaluation {
    /// Whether the chain passes without any failure.
    pub fn passes(&self) -> bool {
        self.verdict == Verdict::Unspecified
    }
}

/// Whether a certificate name matches a host. A wildcard name `*.example.com` matches exactly one label in front
/// of `.example.com`, so `www.example.com` matches and `example.com` and `a.b.example.com` do not. The match
/// ignores ASCII case and a trailing dot on the host.
pub fn host_matches(name: &str, host: &str) -> bool {
    let name = name.trim_end_matches('.').to_ascii_lowercase();
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    if name.is_empty() || host.is_empty() {
        return false;
    }
    if let Some(suffix) = name.strip_prefix("*") {
        // `suffix` starts with '.', so the cut below falls on a character boundary.
        if !suffix.starts_with('.') || !host.ends_with(suffix) || host.len() <= suffix.len() {
            return false;
        }
        let label = &host[..host.len() - suffix.len()];
        return !label.contains('.');
    }
    name == host
}

/// Evaluates a chain, leaf first, against the anchors under a policy at time `now` (Unix seconds).
///
/// `verify(child, issuer)` answers whether `child` is signed by `issuer`'s key. It is called for each link of the
/// chain and for the root's issuer among the anchors. The model never calls it for a link whose names do not
/// match, so a broken link is reported as [`Failure::BrokenChain`] only.
pub fn evaluate(
    chain: &[Certificate],
    anchors: &[Certificate],
    policy: &Policy,
    now: u64,
    verify: &dyn Fn(&Certificate, &Certificate) -> bool,
) -> Evaluation {
    if chain.is_empty() {
        return Evaluation { verdict: Verdict::Invalid, failures: vec![Failure::EmptyChain] };
    }
    let mut failures = Vec::new();
    let last = chain.len() - 1;

    for (i, cert) in chain.iter().enumerate() {
        if now < cert.not_before {
            failures.push(Failure::NotYetValid { index: i });
        } else if now > cert.not_after {
            failures.push(Failure::Expired { index: i });
        }
        if i > 0 {
            if !cert.is_ca {
                failures.push(Failure::NotCa { index: i });
            }
            // The CAs below this one, excluding the leaf, are chain[1..i]: there are i - 1 of them.
            if let Some(limit) = cert.path_len {
                if i - 1 > limit as usize {
                    failures.push(Failure::PathTooLong { index: i });
                }
            }
        }
        if i < last {
            let next = &chain[i + 1];
            if cert.issuer != next.subject {
                failures.push(Failure::BrokenChain { index: i });
            } else if !verify(cert, next) {
                failures.push(Failure::SignatureInvalid { index: i });
            }
        }
    }

    let leaf = &chain[0];
    if let Policy::Ssl { host } = policy {
        if !leaf.dns_names.iter().any(|name| host_matches(name, host)) {
            failures.push(Failure::NameMismatch);
        }
        if !leaf.digital_signature {
            failures.push(Failure::KeyUsage { index: 0 });
        }
    }

    let root = &chain[last];
    let issuer_anchors: Vec<&Certificate> = if root.is_self_signed() {
        Vec::new()
    } else {
        anchors.iter().filter(|a| a.is_ca && a.subject == root.issuer).collect()
    };
    let trusted = anchors.iter().any(|a| a == root) || issuer_anchors.iter().any(|&a| verify(root, a));
    if !trusted {
        if issuer_anchors.is_empty() {
            failures.push(Failure::UntrustedRoot);
        } else {
            failures.push(Failure::SignatureInvalid { index: last });
        }
    }

    let verdict = if failures.is_empty() {
        Verdict::Unspecified
    } else if failures.iter().all(|f| *f == Failure::UntrustedRoot) {
        Verdict::RecoverableTrustFailure
    } else {
        Verdict::FatalTrustFailure
    };
    Evaluation { verdict, failures }
}

/// One labelled line of a certificate view.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub label: &'static str,
    pub value: String,
}

/// The display model of one certificate. The summary is always shown. The details are shown only when the user
/// discloses them, the way a disclosure triangle opens a section.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CertificateView {
    cert: Certificate,
    disclosed: bool,
}

impl CertificateView {
    /// A view of a certificate with the details hidden.
    pub fn new(cert: Certificate) -> CertificateView {
        CertificateView { cert, disclosed: false }
    }

    pub fn is_disclosed(&self) -> bool {
        self.disclosed
    }

    /// Sets the disclosure state. Returns true only when the state changed, so a caller can post one change
    /// notification per real change.
    pub fn set_disclosed(&mut self, on: bool) -> bool {
        let changed = self.disclosed != on;
        self.disclosed = on;
        changed
    }

    /// The rows shown with the details hidden: subject, issuer and the end of validity.
    pub fn summary(&self) -> Vec<Row> {
        vec![
            Row { label: "Subject", value: self.cert.subject.clone() },
            Row { label: "Issuer", value: self.cert.issuer.clone() },
            Row { label: "Expires", value: self.cert.not_after.to_string() },
        ]
    }

    /// The rows shown when the details are disclosed. Empty while they are hidden.
    pub fn details(&self) -> Vec<Row> {
        if !self.disclosed {
            return Vec::new();
        }
        let yes_no = |b: bool| (if b { "Yes" } else { "No" }).to_string();
        let names = if self.cert.dns_names.is_empty() {
            "None".to_string()
        } else {
            self.cert.dns_names.join(", ")
        };
        vec![
            Row { label: "Valid from", value: self.cert.not_before.to_string() },
            Row { label: "Certificate authority", value: yes_no(self.cert.is_ca) },
            Row {
                label: "Path length",
                value: match self.cert.path_len {
                    Some(n) => n.to_string(),
                    None => "No limit".to_string(),
                },
            },
            Row { label: "Digital signature", value: yes_no(self.cert.digital_signature) },
            Row { label: "DNS names", value: names },
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A CA or leaf valid from 0 to 5000 unless the test changes it.
    fn cert(subject: &str, issuer: &str, is_ca: bool) -> Certificate {
        Certificate {
            subject: subject.to_string(),
            issuer: issuer.to_string(),
            not_before: 0,
            not_after: 5000,
            is_ca,
            path_len: None,
            digital_signature: true,
            dns_names: Vec::new(),
        }
    }

    /// A leaf that names `host`, valid from 0 to 2000, issued by `issuer`.
    fn leaf(host: &str, issuer: &str) -> Certificate {
        let mut c = cert("leaf", issuer, false);
        c.not_after = 2000;
        c.dns_names = vec![host.to_string()];
        c
    }

    fn root() -> Certificate {
        cert("root", "root", true)
    }

    fn always(_: &Certificate, _: &Certificate) -> bool {
        true
    }

    fn never(_: &Certificate, _: &Certificate) -> bool {
        false
    }

    fn ssl(host: &str) -> Policy {
        Policy::Ssl { host: host.to_string() }
    }

    #[test]
    fn valid_chain_with_anchored_root_passes() {
        let chain = vec![leaf("www.example.com", "inter"), cert("inter", "root", true), root()];
        let anchors = vec![root()];
        let e = evaluate(&chain, &anchors, &ssl("www.example.com"), 1000, &always);
        assert_eq!(e.verdict, Verdict::Unspecified);
        assert!(e.failures.is_empty());
        assert!(e.passes());
    }

    #[test]
    fn expired_and_not_yet_valid_leaves_are_fatal() {
        // The leaf ends at 2000 and the root at 5000, so only the leaf is out of date at 2001.
        let chain = vec![leaf("a.example.com", "root"), root()];
        let anchors = vec![root()];
        let late = evaluate(&chain, &anchors, &ssl("a.example.com"), 2001, &always);
        assert_eq!(late.verdict, Verdict::FatalTrustFailure);
        assert_eq!(late.failures, vec![Failure::Expired { index: 0 }]);

        let mut early_leaf = leaf("a.example.com", "root");
        early_leaf.not_before = 500;
        let chain = vec![early_leaf, root()];
        let early = evaluate(&chain, &anchors, &ssl("a.example.com"), 400, &always);
        assert_eq!(early.failures, vec![Failure::NotYetValid { index: 0 }]);
    }

    #[test]
    fn untrusted_root_is_recoverable() {
        let chain = vec![leaf("a.example.com", "root"), root()];
        let e = evaluate(&chain, &[], &ssl("a.example.com"), 1000, &always);
        assert_eq!(e.failures, vec![Failure::UntrustedRoot]);
        assert_eq!(e.verdict, Verdict::RecoverableTrustFailure);
        assert!(!e.passes());
    }

    #[test]
    fn broken_link_and_bad_signature_are_reported_by_index() {
        // The leaf names "other" as its issuer, but the next certificate is "inter".
        let chain = vec![leaf("a.example.com", "other"), cert("inter", "root", true), root()];
        let anchors = vec![root()];
        let broken = evaluate(&chain, &anchors, &ssl("a.example.com"), 1000, &always);
        assert_eq!(broken.failures, vec![Failure::BrokenChain { index: 0 }]);

        // Names agree, but no signature verifies: both links fail, and the root is still an anchor.
        let chain = vec![leaf("a.example.com", "inter"), cert("inter", "root", true), root()];
        let forged = evaluate(&chain, &anchors, &ssl("a.example.com"), 1000, &never);
        assert_eq!(
            forged.failures,
            vec![Failure::SignatureInvalid { index: 0 }, Failure::SignatureInvalid { index: 1 }]
        );
    }

    #[test]
    fn host_names_match_with_one_label_wildcards() {
        assert!(host_matches("*.example.com", "www.example.com"));
        assert!(host_matches("*.Example.COM.", "WWW.example.com"));
        assert!(!host_matches("*.example.com", "example.com"));
        assert!(!host_matches("*.example.com", "a.b.example.com"));
        assert!(!host_matches("*.example.com", "www.example.org"));
        assert!(host_matches("www.example.com", "www.example.com"));
        assert!(!host_matches("www.example.com", "mail.example.com"));

        let chain = vec![leaf("*.example.com", "root"), root()];
        let anchors = vec![root()];
        let deep = evaluate(&chain, &anchors, &ssl("a.b.example.com"), 1000, &always);
        assert_eq!(deep.failures, vec![Failure::NameMismatch]);
        assert_eq!(deep.verdict, Verdict::FatalTrustFailure);
    }

    #[test]
    fn path_length_limit_counts_cas_below_the_limited_one() {
        // inter2 allows zero CAs below it, but inter1 sits below it, so the limit is exceeded at index 2.
        let mut limited = cert("inter2", "root", true);
        limited.path_len = Some(0);
        let chain = vec![
            leaf("a.example.com", "inter1"),
            cert("inter1", "inter2", true),
            limited,
            root(),
        ];
        let anchors = vec![root()];
        let e = evaluate(&chain, &anchors, &ssl("a.example.com"), 1000, &always);
        assert_eq!(e.failures, vec![Failure::PathTooLong { index: 2 }]);
    }

    #[test]
    fn certificate_above_the_leaf_must_be_a_ca() {
        let chain = vec![leaf("a.example.com", "inter"), cert("inter", "root", false), root()];
        let anchors = vec![root()];
        let e = evaluate(&chain, &anchors, &ssl("a.example.com"), 1000, &always);
        assert_eq!(e.failures, vec![Failure::NotCa { index: 1 }]);
    }

    #[test]
    fn leaf_without_digital_signature_fails_ssl_only() {
        let mut no_sign = leaf("a.example.com", "root");
        no_sign.digital_signature = false;
        let chain = vec![no_sign, root()];
        let anchors = vec![root()];
        let ssl_eval = evaluate(&chain, &anchors, &ssl("a.example.com"), 1000, &always);
        assert_eq!(ssl_eval.failures, vec![Failure::KeyUsage { index: 0 }]);
        let basic = evaluate(&chain, &anchors, &Policy::Basic, 1000, &always);
        assert!(basic.passes());
    }

    #[test]
    fn empty_chain_is_invalid() {
        let e = evaluate(&[], &[], &Policy::Basic, 0, &always);
        assert_eq!(e.verdict, Verdict::Invalid);
        assert_eq!(e.failures, vec![Failure::EmptyChain]);
    }

    #[test]
    fn certificate_view_discloses_details_once_per_change() {
        let mut c = leaf("a.example.com", "root");
        c.dns_names.push("*.example.com".to_string());
        let mut view = CertificateView::new(c);
        assert!(!view.is_disclosed());
        assert!(view.details().is_empty());
        assert_eq!(view.summary().len(), 3);
        assert_eq!(view.summary()[2], Row { label: "Expires", value: "2000".to_string() });

        assert!(view.set_disclosed(true), "a real change reports true");
        assert!(!view.set_disclosed(true), "the same state reports false");
        let details = view.details();
        assert_eq!(details.len(), 5);
        assert_eq!(details[2], Row { label: "Path length", value: "No limit".to_string() });
        assert_eq!(details[4], Row { label: "DNS names", value: "a.example.com, *.example.com".to_string() });
    }
}
