//! Certificate Assistant: make a certificate, and read one back and judge it.
//!
//! - **Create** a self-signed certificate (or a certificate authority, with `is_ca`) or a certificate signing request
//!   from a [`Request`]: a common name, optional organisation, country and e-mail, host names for the subject
//!   alternative name, a lifetime in days and a key kind. The request is checked first ([`Request::validate`]) and then
//!   turned into the arguments of the system's `openssl` ([`Request::self_signed_args`], [`Request::csr_args`]); the
//!   keys are made by OpenSSL, never by this module, and a key file is never overwritten.
//! - **Read** a certificate ([`read_certificate`]): the fields trust evaluation uses ([`trust::Certificate`]), parsed from
//!   `openssl x509` output ([`parse_description`]).
//! - **Evaluate** a certificate against anchors and a host ([`evaluate_files`]): the rules of [`trust::evaluate`]
//!   (validity window, host name, chain and authority) plus the signature, which `trust` leaves to its caller and which
//!   `openssl verify` checks here.
//!
//! Nothing is read from the network and no key leaves the machine. Private keys are written by `openssl` with owner-only
//! permissions, and this module narrows them again to `0600` after the run.

use crate::calendar_system::days_from_civil;
use crate::trust::{self, Certificate, Policy, Verdict};
use std::fmt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// The program that makes and reads certificates.
pub const OPENSSL: &str = "openssl";
/// The longest lifetime a request may ask for, in days (ten years).
pub const MAX_DAYS: u32 = 3650;

/// The kind of key made for a certificate.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum KeyKind {
    /// elliptic curve P-256: small and fast, the default
    #[default]
    EcP256,
    Rsa2048,
    Rsa4096,
}

impl KeyKind {
    /// The `-newkey` arguments for this kind.
    pub fn newkey_args(self) -> Vec<String> {
        match self {
            KeyKind::EcP256 => vec!["-newkey".into(), "ec".into(), "-pkeyopt".into(), "ec_paramgen_curve:P-256".into()],
            KeyKind::Rsa2048 => vec!["-newkey".into(), "rsa:2048".into()],
            KeyKind::Rsa4096 => vec!["-newkey".into(), "rsa:4096".into()],
        }
    }

    /// The name a person picks it by: `ec`, `rsa2048` or `rsa4096`.
    pub fn name(self) -> &'static str {
        match self {
            KeyKind::EcP256 => "ec",
            KeyKind::Rsa2048 => "rsa2048",
            KeyKind::Rsa4096 => "rsa4096",
        }
    }

    /// The kind called `name`, if there is one.
    pub fn parse(name: &str) -> Option<KeyKind> {
        [KeyKind::EcP256, KeyKind::Rsa2048, KeyKind::Rsa4096].into_iter().find(|k| k.name() == name)
    }
}

/// What a certificate or request is made from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    /// the subject's common name: a person, a host or an authority name (required)
    pub common_name: String,
    pub organization: String,
    pub email: String,
    /// two capital letters, or empty
    pub country: String,
    /// host names for the subject alternative name; `*.example.com` covers one label
    pub dns_names: Vec<String>,
    /// how long the certificate is valid, 1 to [`MAX_DAYS`]
    pub days: u32,
    /// make a certificate authority (it may issue other certificates)
    pub is_ca: bool,
    pub key: KeyKind,
}

/// Why a request was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Problem {
    NoCommonName,
    /// a field holds a control character, `/` or `\` (the subject line cannot carry them)
    BadCharacter(&'static str),
    BadCountry,
    BadEmail,
    BadDays,
    BadDnsName(String),
}

impl fmt::Display for Problem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Problem::NoCommonName => write!(f, "A common name is needed"),
            Problem::BadCharacter(field) => write!(f, "The {field} holds a character that cannot be used (a control character, / or \\)"),
            Problem::BadCountry => write!(f, "The country is two capital letters, such as IS"),
            Problem::BadEmail => write!(f, "The e-mail address needs a name, an @ and a domain"),
            Problem::BadDays => write!(f, "The lifetime is 1 to {MAX_DAYS} days"),
            Problem::BadDnsName(name) => write!(f, "{name} is not a host name"),
        }
    }
}

/// Why making or reading a certificate failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    Invalid(Problem),
    /// a file the run would write is already there; nothing was changed
    Exists(PathBuf),
    Io(String),
    /// `openssl` ran and refused; the first line of what it said
    Openssl(String),
    /// the `openssl` output did not hold what a certificate has
    Unreadable(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Invalid(p) => write!(f, "{p}"),
            Error::Exists(p) => write!(f, "{} already exists and is not overwritten", p.display()),
            Error::Io(e) => write!(f, "{e}"),
            Error::Openssl(e) => write!(f, "openssl: {e}"),
            Error::Unreadable(e) => write!(f, "not a certificate: {e}"),
        }
    }
}

impl From<Problem> for Error {
    fn from(p: Problem) -> Error {
        Error::Invalid(p)
    }
}

fn clean(text: &str) -> bool {
    !text.chars().any(|c| c.is_control() || c == '/' || c == '\\')
}

/// Whether `name` is a host name: dot-separated labels of letters, digits and `-`, optionally starting with `*.`.
pub fn valid_dns_name(name: &str) -> bool {
    let body = name.strip_prefix("*.").unwrap_or(name);
    !body.is_empty()
        && body.len() <= 253
        && body.split('.').all(|label| {
            !label.is_empty() && label.len() <= 63 && !label.starts_with('-') && !label.ends_with('-') && label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        })
}

impl Request {
    /// A request for a certificate valid for a year, with an elliptic-curve key, for `common_name`.
    pub fn new(common_name: &str) -> Request {
        Request {
            common_name: common_name.trim().to_string(),
            organization: String::new(),
            email: String::new(),
            country: String::new(),
            dns_names: Vec::new(),
            days: 365,
            is_ca: false,
            key: KeyKind::default(),
        }
    }

    /// Checks every field. The first problem found is returned.
    pub fn validate(&self) -> Result<(), Problem> {
        if self.common_name.trim().is_empty() {
            return Err(Problem::NoCommonName);
        }
        for (field, text) in [("common name", &self.common_name), ("organisation", &self.organization), ("e-mail address", &self.email)] {
            if !clean(text) {
                return Err(Problem::BadCharacter(field));
            }
        }
        if !self.country.is_empty() && !(self.country.len() == 2 && self.country.chars().all(|c| c.is_ascii_uppercase())) {
            return Err(Problem::BadCountry);
        }
        if !self.email.is_empty() {
            let ok = match self.email.split_once('@') {
                Some((name, domain)) => !name.is_empty() && !domain.contains('@') && valid_dns_name(domain) && !self.email.contains(char::is_whitespace),
                None => false,
            };
            if !ok {
                return Err(Problem::BadEmail);
            }
        }
        if self.days == 0 || self.days > MAX_DAYS {
            return Err(Problem::BadDays);
        }
        if let Some(bad) = self.dns_names.iter().find(|n| !valid_dns_name(n)) {
            return Err(Problem::BadDnsName(bad.clone()));
        }
        Ok(())
    }

    /// The `-subj` text: `/C=IS/O=Org/CN=name/emailAddress=a@b`, with the fields that are set.
    pub fn subject(&self) -> String {
        let mut s = String::new();
        for (key, value) in [("C", &self.country), ("O", &self.organization), ("CN", &self.common_name), ("emailAddress", &self.email)] {
            if !value.trim().is_empty() {
                s.push_str(&format!("/{key}={}", value.trim()));
            }
        }
        s
    }

    /// The `-addext` values: the basic constraints, and the subject alternative name when host names are given.
    pub fn extensions(&self) -> Vec<String> {
        let mut out = vec![if self.is_ca { "basicConstraints=critical,CA:TRUE".to_string() } else { "basicConstraints=critical,CA:FALSE".to_string() }];
        if !self.dns_names.is_empty() {
            out.push(format!("subjectAltName={}", self.dns_names.iter().map(|n| format!("DNS:{n}")).collect::<Vec<_>>().join(",")));
        }
        out
    }

    fn req_args(&self, key_out: &Path, out: &Path, self_signed: bool) -> Result<Vec<String>, Problem> {
        self.validate()?;
        let mut args: Vec<String> = vec!["req".into()];
        if self_signed {
            args.push("-x509".into());
        }
        args.extend(self.key.newkey_args());
        args.extend(["-nodes".into(), "-keyout".into(), key_out.display().to_string(), "-out".into(), out.display().to_string()]);
        if self_signed {
            args.extend(["-days".into(), self.days.to_string()]);
        }
        args.extend(["-subj".into(), self.subject()]);
        for ext in self.extensions() {
            args.extend(["-addext".into(), ext]);
        }
        Ok(args)
    }

    /// The arguments that make a self-signed certificate and its key.
    pub fn self_signed_args(&self, key_out: &Path, cert_out: &Path) -> Result<Vec<String>, Problem> {
        self.req_args(key_out, cert_out, true)
    }

    /// The arguments that make a signing request and its key.
    pub fn csr_args(&self, key_out: &Path, csr_out: &Path) -> Result<Vec<String>, Problem> {
        self.req_args(key_out, csr_out, false)
    }
}

/// A file name stem for a common name: letters, digits, `.`, `-` and `_` stay, anything else becomes `_`.
pub fn file_stem(common_name: &str) -> String {
    let stem: String = common_name
        .trim()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') { c.to_ascii_lowercase() } else { '_' })
        .collect();
    let stem = stem.trim_matches('.').to_string();
    if stem.is_empty() {
        "certificate".to_string()
    } else {
        stem
    }
}

/// The files a run made.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Created {
    pub key: PathBuf,
    /// the certificate, or the signing request
    pub output: PathBuf,
}

fn run_openssl(args: &[String]) -> Result<String, Error> {
    let out = Command::new(OPENSSL)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| Error::Io(format!("cannot run {OPENSSL}: {e}")))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        let said = String::from_utf8_lossy(&out.stderr);
        Err(Error::Openssl(said.lines().find(|l| !l.trim().is_empty()).unwrap_or("failed").trim().to_string()))
    }
}

fn make(req: &Request, dir: &Path, self_signed: bool) -> Result<Created, Error> {
    req.validate()?;
    let stem = file_stem(&req.common_name);
    let key = dir.join(format!("{stem}.key.pem"));
    let output = dir.join(if self_signed { format!("{stem}.cert.pem") } else { format!("{stem}.csr.pem") });
    for path in [&key, &output] {
        if path.exists() {
            return Err(Error::Exists(path.clone()));
        }
    }
    std::fs::create_dir_all(dir).map_err(|e| Error::Io(format!("cannot make {}: {e}", dir.display())))?;
    let args = req.req_args(&key, &output, self_signed)?;
    run_openssl(&args)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&key, std::fs::Permissions::from_mode(0o600)).map_err(|e| Error::Io(format!("cannot protect {}: {e}", key.display())))?;
    }
    Ok(Created { key, output })
}

/// Makes a self-signed certificate and its key in `dir` (`<name>.cert.pem`, `<name>.key.pem`). Refuses to overwrite.
pub fn create_self_signed(req: &Request, dir: &Path) -> Result<Created, Error> {
    make(req, dir, true)
}

/// Makes a certificate signing request and its key in `dir` (`<name>.csr.pem`, `<name>.key.pem`). Refuses to overwrite.
pub fn create_request(req: &Request, dir: &Path) -> Result<Created, Error> {
    make(req, dir, false)
}

/// The `openssl` arguments that print what [`parse_description`] reads.
pub fn describe_args(cert: &Path) -> Vec<String> {
    ["x509", "-in", &cert.display().to_string(), "-noout", "-subject", "-issuer", "-startdate", "-enddate", "-ext", "subjectAltName,basicConstraints", "-nameopt", "RFC2253"]
        .iter()
        .map(|s| s.to_string())
        .collect()
}

/// Seconds since the Unix epoch of an `openssl` time such as `Oct  1 12:00:00 2026 GMT`.
pub fn parse_openssl_time(text: &str) -> Option<u64> {
    const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    let mut parts = text.split_whitespace();
    let name = parts.next()?;
    let month = MONTHS.iter().position(|m| *m == name)? as i64 + 1;
    let day: i64 = parts.next()?.parse().ok()?;
    let mut clock = parts.next()?.split(':');
    let (h, m, s): (i64, i64, i64) = (clock.next()?.parse().ok()?, clock.next()?.parse().ok()?, clock.next()?.parse().ok()?);
    let year: i64 = parts.next()?.parse().ok()?;
    if !(1..=31).contains(&day) || h > 23 || m > 59 || s > 60 || parts.next() != Some("GMT") {
        return None;
    }
    let secs = days_from_civil(year, month, day) * 86_400 + h * 3600 + m * 60 + s;
    u64::try_from(secs).ok()
}

/// The certificate `openssl x509 ... -subject -issuer -startdate -enddate -ext ...` describes (see [`describe_args`]).
pub fn parse_description(text: &str) -> Result<Certificate, Error> {
    let mut subject = None;
    let mut issuer = None;
    let (mut not_before, mut not_after) = (None, None);
    let mut dns_names = Vec::new();
    let mut is_ca = false;
    let mut lines = text.lines();
    while let Some(line) = lines.next() {
        let t = line.trim();
        if let Some(v) = t.strip_prefix("subject=") {
            subject = Some(v.trim().to_string());
        } else if let Some(v) = t.strip_prefix("issuer=") {
            issuer = Some(v.trim().to_string());
        } else if let Some(v) = t.strip_prefix("notBefore=") {
            not_before = parse_openssl_time(v);
        } else if let Some(v) = t.strip_prefix("notAfter=") {
            not_after = parse_openssl_time(v);
        } else if t.starts_with("X509v3 Subject Alternative Name") {
            if let Some(names) = lines.next() {
                dns_names.extend(names.split(',').filter_map(|n| n.trim().strip_prefix("DNS:")).map(str::to_string));
            }
        } else if t.starts_with("X509v3 Basic Constraints") {
            if let Some(value) = lines.next() {
                is_ca = value.split(',').any(|v| v.trim() == "CA:TRUE");
            }
        }
    }
    match (subject, issuer, not_before, not_after) {
        (Some(subject), Some(issuer), Some(not_before), Some(not_after)) => Ok(Certificate { subject, issuer, dns_names, not_before, not_after, is_ca }),
        _ => Err(Error::Unreadable("subject, issuer or validity dates are missing".into())),
    }
}

/// Reads a PEM or DER certificate file through `openssl`.
pub fn read_certificate(path: &Path) -> Result<Certificate, Error> {
    parse_description(&run_openssl(&describe_args(path))?)
}

/// What [`evaluate_files`] found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Evaluation {
    pub certificate: Certificate,
    /// the verdict of the trust rules (validity, host, chain, authority) against the anchors
    pub verdict: Verdict,
    /// whether one of the anchors signed the certificate (or it signed itself and is an anchor)
    pub signature_ok: bool,
}

impl Evaluation {
    /// Trusted only when both the rules and the signature hold.
    pub fn trusted(&self) -> bool {
        self.verdict.is_trusted() && self.signature_ok
    }
}

/// The words for a refusal.
pub fn reason_text(reason: trust::Reason) -> &'static str {
    match reason {
        trust::Reason::NoCertificates => "there is no certificate",
        trust::Reason::Expired => "the certificate has expired",
        trust::Reason::NotYetValid => "the certificate is not valid yet",
        trust::Reason::HostMismatch => "the certificate does not name this host",
        trust::Reason::BrokenChain => "the certificate was not issued by the next one in the chain",
        trust::Reason::NotCertificateAuthority => "an issuer in the chain is not a certificate authority",
        trust::Reason::UntrustedRoot => "no trusted anchor issued the certificate",
    }
}

/// Judges the certificate in `leaf` against the certificates in `anchors` at Unix time `now`, for `host` when given.
/// Lower-level than a chain walk: the leaf stands alone, so it is trusted when it is itself an anchor or an anchor
/// issued it, and the signature is checked with `openssl verify`.
pub fn evaluate_files(leaf: &Path, anchors: &[PathBuf], host: Option<&str>, now: u64) -> Result<Evaluation, Error> {
    let certificate = read_certificate(leaf)?;
    let anchor_certs = anchors.iter().map(|a| read_certificate(a)).collect::<Result<Vec<_>, _>>()?;
    let policy = match host {
        Some(h) => Policy::ssl_server(h),
        None => Policy::basic(),
    };
    let verdict = trust::evaluate(std::slice::from_ref(&certificate), &anchor_certs, &policy, now);
    let signature_ok = anchors.iter().any(|anchor| {
        run_openssl(&["verify".into(), "-no_check_time".into(), "-CAfile".into(), anchor.display().to_string(), leaf.display().to_string()]).is_ok()
    });
    Ok(Evaluation { certificate, verdict, signature_ok })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "subject=CN=shop.example,O=Shop Ltd\n\
issuer=CN=Test Root\n\
notBefore=Oct  1 12:00:00 2026 GMT\n\
notAfter=Oct  1 12:00:00 2027 GMT\n\
X509v3 Subject Alternative Name: \n\
    DNS:shop.example, DNS:*.shop.example, IP Address:10.0.0.1\n\
X509v3 Basic Constraints: critical\n\
    CA:FALSE\n";

    #[test]
    fn a_description_is_read_into_the_fields_trust_uses() {
        let c = parse_description(SAMPLE).unwrap();
        assert_eq!(c.subject, "CN=shop.example,O=Shop Ltd");
        assert_eq!(c.issuer, "CN=Test Root");
        assert_eq!(c.dns_names, vec!["shop.example", "*.shop.example"], "the IP address is not a DNS name");
        assert!(!c.is_ca);
        assert_eq!(c.not_after - c.not_before, 365 * 86_400);
        let ca = parse_description(&SAMPLE.replace("CA:FALSE", "CA:TRUE, pathlen:0")).unwrap();
        assert!(ca.is_ca);
    }

    #[test]
    fn a_description_without_the_main_fields_is_refused() {
        assert!(matches!(parse_description(""), Err(Error::Unreadable(_))));
        assert!(matches!(parse_description("subject=CN=a\nissuer=CN=a\n"), Err(Error::Unreadable(_))));
        // no extensions at all is fine: a certificate may carry none
        let bare = "subject=CN=a\nissuer=CN=a\nnotBefore=Jan  1 00:00:00 2026 GMT\nnotAfter=Jan  2 00:00:00 2026 GMT\n";
        let c = parse_description(bare).unwrap();
        assert!(c.dns_names.is_empty() && !c.is_ca);
    }

    #[test]
    fn times_are_read_as_utc() {
        assert_eq!(parse_openssl_time("Jan  1 00:00:00 1970 GMT"), Some(0));
        assert_eq!(parse_openssl_time("Jan  2 00:00:01 1970 GMT"), Some(86_401));
        assert_eq!(parse_openssl_time("Mar  1 00:00:00 2000 GMT"), Some(951_868_800));
        assert_eq!(parse_openssl_time("Feb 29 23:59:59 2028 GMT"), Some(1_835_481_599));
        assert_eq!(parse_openssl_time("Oct 17 12:00:00 2026 PST"), None, "only GMT is written");
        assert_eq!(parse_openssl_time("Foo 17 12:00:00 2026 GMT"), None);
        assert_eq!(parse_openssl_time("Oct 32 12:00:00 2026 GMT"), None);
        assert_eq!(parse_openssl_time("Oct 17 25:00:00 2026 GMT"), None);
        assert_eq!(parse_openssl_time(""), None);
    }

    #[test]
    fn a_description_judged_by_the_trust_rules() {
        let c = parse_description(SAMPLE).unwrap();
        let during = c.not_before + 86_400;
        let root = Certificate { subject: "CN=Test Root".into(), issuer: "CN=Test Root".into(), dns_names: vec![], not_before: 0, not_after: u64::MAX / 2, is_ca: true };
        let ok = trust::evaluate(std::slice::from_ref(&c), std::slice::from_ref(&root), &Policy::ssl_server("www.shop.example"), during);
        assert_eq!(ok, Verdict::Trusted, "the wildcard covers www");
        let wrong = trust::evaluate(std::slice::from_ref(&c), std::slice::from_ref(&root), &Policy::ssl_server("other.example"), during);
        assert_eq!(wrong, Verdict::Refused(trust::Reason::HostMismatch));
        let late = trust::evaluate(std::slice::from_ref(&c), std::slice::from_ref(&root), &Policy::basic(), c.not_after + 1);
        assert_eq!(late, Verdict::Refused(trust::Reason::Expired));
        assert_eq!(reason_text(trust::Reason::Expired), "the certificate has expired");
    }

    #[test]
    fn a_request_is_checked_before_anything_runs() {
        let mut r = Request::new("shop.example");
        assert_eq!(r.validate(), Ok(()));
        assert_eq!(Request::new("  ").validate(), Err(Problem::NoCommonName));
        r.country = "is".into();
        assert_eq!(r.validate(), Err(Problem::BadCountry));
        r.country = "IS".into();
        r.email = "ada@".into();
        assert_eq!(r.validate(), Err(Problem::BadEmail));
        r.email = "ada@shop.example".into();
        r.days = 0;
        assert_eq!(r.validate(), Err(Problem::BadDays));
        r.days = MAX_DAYS + 1;
        assert_eq!(r.validate(), Err(Problem::BadDays));
        r.days = 30;
        r.dns_names = vec!["ok.example".into(), "bad_name.example".into()];
        assert_eq!(r.validate(), Err(Problem::BadDnsName("bad_name.example".into())));
        r.dns_names = vec!["*.shop.example".into()];
        r.organization = "Shop/Ltd".into();
        assert_eq!(r.validate(), Err(Problem::BadCharacter("organisation")));
        r.organization = "Shop Ltd".into();
        assert_eq!(r.validate(), Ok(()));
        assert_eq!(Problem::BadDays.to_string(), "The lifetime is 1 to 3650 days");
    }

    #[test]
    fn host_names_accept_labels_and_one_leading_wildcard() {
        for ok in ["a", "a.b", "*.example.com", "x-1.example"] {
            assert!(valid_dns_name(ok), "{ok}");
        }
        for bad in ["", "*", "a..b", "-a.b", "a-.b", "a b", "a_b", "*.*.a", ".a"] {
            assert!(!valid_dns_name(bad), "{bad}");
        }
        assert!(!valid_dns_name(&"x".repeat(64)), "a label is at most 63 characters");
        assert!(valid_dns_name(&"x".repeat(63)));
    }

    #[test]
    fn the_subject_and_extensions_follow_the_request() {
        let mut r = Request::new(" shop.example ");
        assert_eq!(r.subject(), "/CN=shop.example");
        assert_eq!(r.extensions(), vec!["basicConstraints=critical,CA:FALSE"]);
        r.country = "IS".into();
        r.organization = "Shop Ltd".into();
        r.email = "ada@shop.example".into();
        r.dns_names = vec!["shop.example".into(), "*.shop.example".into()];
        r.is_ca = true;
        assert_eq!(r.subject(), "/C=IS/O=Shop Ltd/CN=shop.example/emailAddress=ada@shop.example");
        assert_eq!(r.extensions(), vec!["basicConstraints=critical,CA:TRUE", "subjectAltName=DNS:shop.example,DNS:*.shop.example"]);
    }

    #[test]
    fn the_arguments_make_a_self_signed_certificate_or_a_request() {
        let mut r = Request::new("shop.example");
        r.days = 90;
        r.key = KeyKind::Rsa2048;
        let cert = r.self_signed_args(Path::new("k.pem"), Path::new("c.pem")).unwrap();
        assert_eq!(&cert[..2], ["req", "-x509"]);
        assert!(cert.windows(2).any(|w| w == ["-newkey", "rsa:2048"]));
        assert!(cert.windows(2).any(|w| w == ["-days", "90"]));
        assert!(cert.windows(2).any(|w| w == ["-keyout", "k.pem"]));
        assert!(cert.windows(2).any(|w| w == ["-subj", "/CN=shop.example"]));
        assert!(cert.contains(&"-nodes".to_string()));
        let csr = r.csr_args(Path::new("k.pem"), Path::new("r.pem")).unwrap();
        assert_eq!(csr[0], "req");
        assert!(!csr.contains(&"-x509".to_string()) && !csr.contains(&"-days".to_string()));
        assert!(csr.windows(2).any(|w| w == ["-out", "r.pem"]));
        // a bad request makes no arguments
        assert_eq!(Request::new("").self_signed_args(Path::new("k"), Path::new("c")), Err(Problem::NoCommonName));
        assert_eq!(KeyKind::EcP256.newkey_args()[3], "ec_paramgen_curve:P-256");
        assert_eq!(KeyKind::parse("rsa4096"), Some(KeyKind::Rsa4096));
        assert_eq!(KeyKind::parse("dsa"), None);
    }

    #[test]
    fn file_stems_keep_safe_characters_only() {
        assert_eq!(file_stem("Shop Example"), "shop_example");
        assert_eq!(file_stem("*.shop.example"), "_.shop.example");
        assert_eq!(file_stem("../etc"), "_etc", "no path can be built from a name");
        assert_eq!(file_stem("..."), "certificate");
        assert_eq!(file_stem(""), "certificate");
    }

    fn openssl_present() -> bool {
        Command::new(OPENSSL).arg("version").stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok_and(|s| s.success())
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("bifrost-certassist-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn a_made_certificate_reads_back_and_is_trusted_as_its_own_anchor() {
        if !openssl_present() {
            return;
        }
        let dir = scratch("self");
        let mut r = Request::new("Lab Root");
        r.dns_names = vec!["lab.example".into()];
        r.is_ca = true;
        r.days = 30;
        let made = create_self_signed(&r, &dir).expect("made");
        assert!(made.output.ends_with("lab_root.cert.pem") && made.key.ends_with("lab_root.key.pem"));
        let cert = read_certificate(&made.output).expect("read back");
        assert!(cert.subject.contains("CN=Lab Root"), "{}", cert.subject);
        assert_eq!(cert.dns_names, vec!["lab.example"]);
        assert!(cert.is_ca);
        assert_eq!(cert.not_after - cert.not_before, 30 * 86_400);
        let now = cert.not_before + 3600;
        let seen = evaluate_files(&made.output, std::slice::from_ref(&made.output), Some("lab.example"), now).expect("evaluated");
        assert!(seen.trusted(), "{seen:?}");
        let other = evaluate_files(&made.output, std::slice::from_ref(&made.output), Some("other.example"), now).expect("evaluated");
        assert_eq!(other.verdict, Verdict::Refused(trust::Reason::HostMismatch));
        assert!(!other.trusted());
        let late = evaluate_files(&made.output, std::slice::from_ref(&made.output), None, cert.not_after + 1).expect("evaluated");
        assert_eq!(late.verdict, Verdict::Refused(trust::Reason::Expired));
        // the key is the owner's alone, and a second run never overwrites it
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&made.key).unwrap().permissions().mode() & 0o777, 0o600);
        }
        assert_eq!(create_self_signed(&r, &dir), Err(Error::Exists(made.key.clone())));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_certificate_from_another_authority_is_not_trusted() {
        if !openssl_present() {
            return;
        }
        let dir = scratch("other");
        let mut a = Request::new("Authority A");
        a.is_ca = true;
        let mut b = Request::new("Authority B");
        b.is_ca = true;
        let a = create_self_signed(&a, &dir).expect("a");
        let b = create_self_signed(&b, &dir).expect("b");
        let now = read_certificate(&a.output).unwrap().not_before + 60;
        let seen = evaluate_files(&a.output, std::slice::from_ref(&b.output), None, now).expect("evaluated");
        assert_eq!(seen.verdict, Verdict::Refused(trust::Reason::UntrustedRoot));
        assert!(!seen.signature_ok && !seen.trusted());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_signing_request_is_made_with_its_key_and_a_bad_request_makes_nothing() {
        if !openssl_present() {
            return;
        }
        let dir = scratch("csr");
        let made = create_request(&Request::new("shop.example"), &dir).expect("made");
        assert!(made.output.ends_with("shop.example.csr.pem"));
        assert!(std::fs::read_to_string(&made.output).unwrap().contains("CERTIFICATE REQUEST"));
        assert!(std::fs::read_to_string(&made.key).unwrap().contains("PRIVATE KEY"));
        let bad = scratch("bad");
        assert_eq!(create_self_signed(&Request::new(""), &bad), Err(Error::Invalid(Problem::NoCommonName)));
        assert!(!bad.exists(), "a refused request makes no directory");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
