//! The `trust` service: certificate chain evaluation for apps (`SecTrustEvaluateWithError`). The model is
//! `bifrost_ui::trust` (validity, names, CA and path rules, the chain and the anchors). The model never reads a
//! signature, and this service does not parse certificates from DER either: the app sends the fields it parsed.
//!
//! Signature checking is not built here. The verify callback answers `false` for every link, so a chain that passes
//! the other rules still fails with `SignatureInvalid`. That keeps the service fail-closed: it can say a chain is
//! malformed, expired or for another host, but it never says a chain is trusted. Network connections are verified by
//! the TLS stack (rustls with the webpki roots), not by this service.
//!
//! Methods (JSON parameters):
//! - `evaluate {chain, anchors, host?, now?}`: `chain` is a list of certificates, leaf first; `anchors` the trusted
//!   roots; `host` selects the SSL policy (a leaf that names it), and without it the basic policy applies; `now` is
//!   unix seconds (default: the clock). A certificate is `{subject, issuer, not_before, not_after, is_ca,
//!   path_len?, digital_signature, dns_names}`.
//!   Answers `{verdict, passes, failures}`: `verdict` is `invalid`, `unspecified`, `recoverable` or `fatal`, and
//!   `failures` lists each failure as text.
use super::location::now_unix;
use super::{Shell, SystemService};
use bifrost_core::json::Json;
use bifrost_ui::trust::{evaluate, Certificate, Policy, Verdict};

/// The service name.
pub const SERVICE: &str = "trust";

/// The `trust` service.
pub struct TrustEval;

impl SystemService for TrustEval {
    fn name(&self) -> &'static str {
        SERVICE
    }

    fn call(&mut self, _shell: &mut Shell, _app_id: &str, method: &str, params: &Json, _t: f64) -> Result<Json, String> {
        answer(method, params)
    }
}

/// Runs one method. Pure apart from reading the clock when `now` is absent.
pub fn answer(method: &str, params: &Json) -> Result<Json, String> {
    if method != "evaluate" {
        return Err(format!("trust has no method {method}"));
    }
    let chain = certificates(params, "chain")?;
    let anchors = certificates(params, "anchors")?;
    let policy = match params.str_field("host") {
        Some(host) => Policy::Ssl { host },
        None => Policy::Basic,
    };
    let now = params.get("now").and_then(Json::as_u64).unwrap_or_else(|| now_unix() as u64);
    // Fail-closed: no signature is ever accepted by this service (see the module doc).
    let evaluation = evaluate(&chain, &anchors, &policy, now, &|_child: &Certificate, _issuer: &Certificate| false);
    let verdict = match evaluation.verdict {
        Verdict::Invalid => "invalid",
        Verdict::Unspecified => "unspecified",
        Verdict::RecoverableTrustFailure => "recoverable",
        Verdict::FatalTrustFailure => "fatal",
    };
    let failures = evaluation.failures.iter().map(|f| Json::Str(format!("{f:?}"))).collect();
    Ok(Json::Obj(vec![
        ("verdict".into(), Json::Str(verdict.into())),
        ("passes".into(), Json::Bool(evaluation.passes())),
        ("failures".into(), Json::Arr(failures)),
    ]))
}

fn certificates(params: &Json, key: &str) -> Result<Vec<Certificate>, String> {
    match params.get(key) {
        None | Some(Json::Null) => Ok(Vec::new()),
        Some(Json::Arr(items)) => items.iter().map(certificate).collect(),
        Some(_) => Err(format!("{key} must be a list of certificates")),
    }
}

fn certificate(j: &Json) -> Result<Certificate, String> {
    let text = |k: &str| j.str_field(k).ok_or_else(|| format!("certificate {k} must be text"));
    let time = |k: &str| j.get(k).and_then(Json::as_u64).ok_or_else(|| format!("certificate {k} must be a unix time"));
    let flag = |k: &str| matches!(j.get(k), Some(Json::Bool(true)));
    let path_len = match j.get("path_len") {
        None | Some(Json::Null) => None,
        Some(n) => Some(n.as_u64().and_then(|v| u8::try_from(v).ok()).ok_or_else(|| "certificate path_len must be 0 to 255".to_string())?),
    };
    let dns_names = match j.get("dns_names") {
        Some(Json::Arr(list)) => list.iter().filter_map(|v| v.as_str().map(str::to_string)).collect(),
        _ => Vec::new(),
    };
    Ok(Certificate {
        subject: text("subject")?,
        issuer: text("issuer")?,
        not_before: time("not_before")?,
        not_after: time("not_after")?,
        is_ca: flag("is_ca"),
        path_len,
        digital_signature: flag("digital_signature"),
        dns_names,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cert(subject: &str, issuer: &str, is_ca: bool) -> Json {
        Json::Obj(vec![
            ("subject".into(), Json::Str(subject.into())),
            ("issuer".into(), Json::Str(issuer.into())),
            ("not_before".into(), Json::Num(0.0)),
            ("not_after".into(), Json::Num(4_000_000_000.0)),
            ("is_ca".into(), Json::Bool(is_ca)),
            ("digital_signature".into(), Json::Bool(!is_ca)),
            ("dns_names".into(), Json::Arr(vec![Json::Str("example.com".into())])),
        ])
    }

    fn request(chain: Vec<Json>, host: Option<&str>) -> Json {
        let mut pairs = vec![("chain".to_string(), Json::Arr(chain)), ("anchors".to_string(), Json::Arr(Vec::new())), ("now".to_string(), Json::Num(1_000.0))];
        if let Some(h) = host {
            pairs.push(("host".to_string(), Json::Str(h.into())));
        }
        Json::Obj(pairs)
    }

    #[test]
    fn an_empty_chain_is_invalid() {
        let r = answer("evaluate", &request(Vec::new(), None)).unwrap();
        assert_eq!(r.get("verdict").and_then(Json::as_str), Some("invalid"));
        assert_eq!(r.get("passes"), Some(&Json::Bool(false)));
    }

    #[test]
    fn no_chain_passes_without_signature_checks() {
        let chain = vec![cert("example.com", "Root", false), cert("Root", "Root", true)];
        let r = answer("evaluate", &request(chain, Some("example.com"))).unwrap();
        assert_eq!(r.get("passes"), Some(&Json::Bool(false)));
        let failures = r.get("failures").and_then(Json::as_array).unwrap();
        assert!(!failures.is_empty());
    }

    #[test]
    fn a_leaf_for_another_host_is_refused() {
        let chain = vec![cert("example.com", "Root", false), cert("Root", "Root", true)];
        let r = answer("evaluate", &request(chain, Some("other.org"))).unwrap();
        assert_eq!(r.get("passes"), Some(&Json::Bool(false)));
        let text = format!("{:?}", r.get("failures"));
        assert!(text.contains("NameMismatch") || text.contains("SignatureInvalid"));
    }

    #[test]
    fn malformed_requests_are_errors() {
        assert!(answer("evaluate", &request(vec![Json::Num(1.0)], None)).is_err());
        assert!(answer("verify", &request(Vec::new(), None)).is_err());
    }
}
