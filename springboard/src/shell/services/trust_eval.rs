//! The `trust` service: certificate chain evaluation for apps (`SecTrustEvaluateWithError`). The model is
//! `bifrost_ui::trust` (validity, names, CA and path rules, the chain and the anchors). The model asks a verify
//! callback whether each link is signed; this service answers that with Ed25519 (`bifrost_ui::ed25519`, the same
//! primitive the SDK's `crypto_kit` uses). It does not parse certificates from DER: the app sends the fields it parsed,
//! and the bytes that were signed.
//!
//! A link is trusted only when the child certificate carries `signed` and `signature` and the issuer carries
//! `public_key`, `signed` is exactly the canonical encoding of the child's fields ([`to_be_signed`]), and the signature
//! verifies over it. Binding the fields to the signed bytes means a certificate altered after signing fails. A certificate without those fields fails its link, so the
//! service stays fail-closed for anything it cannot check. Network connections are verified by the TLS stack (rustls
//! with the webpki roots), not by this service.
//!
//! Methods (JSON parameters):
//! - `evaluate {chain, anchors, host?, now?}`: `chain` is a list of certificates, leaf first; `anchors` the trusted
//!   roots; `host` selects the SSL policy (a leaf that names it), and without it the basic policy applies; `now` is
//!   unix seconds (default: the clock). A certificate is `{subject, issuer, not_before, not_after, is_ca,
//!   path_len?, digital_signature, dns_names, public_key?, signed?, signature?}`. `public_key` is 32 bytes of Ed25519
//!   key, `signed` the signed bytes and `signature` the 64-byte signature, all as hex text.
//!   Answers `{verdict, passes, failures}`: `verdict` is `invalid`, `unspecified`, `recoverable` or `fatal`, and
//!   `failures` lists each failure as text.
use super::location::now_unix;
use super::{Shell, SystemService};
use bifrost_core::json::Json;
use bifrost_core::plist::hex_decode;
use bifrost_ui::ed25519::PublicKey;
use bifrost_ui::trust::{evaluate, Certificate, Policy, Verdict};

/// The signature material of one certificate: what it holds, and what it signed. `None` where the app sent none.
#[derive(Default)]
struct Signing {
    public_key: Option<Vec<u8>>,
    signed: Option<Vec<u8>>,
    signature: Option<Vec<u8>>,
    /// The canonical encoding of the certificate's own fields, which `signed` must equal.
    tbs: Vec<u8>,
}

/// The canonical bytes a certificate's issuer signs: a version line, then one line per field in a fixed order (DNS
/// names comma separated, the public key as hex, empty when absent).
pub fn to_be_signed(cert: &Certificate, public_key: Option<&[u8]>) -> Vec<u8> {
    let path_len = cert.path_len.map(|n| n.to_string()).unwrap_or_default();
    let key = public_key.map(bifrost_core::plist::hex_encode).unwrap_or_default();
    format!(
        "bifrost-cert-1\nsubject={}\nissuer={}\nnot_before={}\nnot_after={}\nis_ca={}\npath_len={}\ndigital_signature={}\ndns_names={}\npublic_key={}\n",
        cert.subject.replace('\n', " "),
        cert.issuer.replace('\n', " "),
        cert.not_before,
        cert.not_after,
        cert.is_ca,
        path_len,
        cert.digital_signature,
        cert.dns_names.join(",").replace('\n', " "),
        key,
    )
    .into_bytes()
}

/// The signing material of `cert`, found by its address in one of the `(certificates, material)` lists.
fn material<'a>(lists: &[(&[Certificate], &'a [Signing])], cert: &Certificate) -> Option<&'a Signing> {
    lists.iter().find_map(|(certs, signing)| certs.iter().position(|c| std::ptr::eq(c, cert)).map(|i| &signing[i]))
}

/// Whether the certificate `child` is signed by the key of `issuer`. Missing material is a failed link.
fn link_signed(child: &Signing, issuer: &Signing) -> bool {
    let (Some(signed), Some(signature), Some(key)) = (&child.signed, &child.signature, &issuer.public_key) else {
        return false;
    };
    if *signed != child.tbs {
        return false;
    }
    match PublicKey::from_raw_representation(key) {
        Ok(key) => key.is_valid_signature(signature, signed),
        Err(_) => false,
    }
}

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
    let (chain, chain_signing) = certificates(params, "chain")?;
    let (anchors, anchor_signing) = certificates(params, "anchors")?;
    let policy = match params.str_field("host") {
        Some(host) => Policy::Ssl { host },
        None => Policy::Basic,
    };
    let now = params.get("now").and_then(Json::as_u64).unwrap_or_else(|| now_unix() as u64);
    // Each certificate is found by its address in the request lists, so the callback reads the material the app
    // sent for exactly that certificate.
    let verify = |child: &Certificate, issuer: &Certificate| {
        let lists = [(&chain[..], &chain_signing[..]), (&anchors[..], &anchor_signing[..])];
        match (material(&lists, child), material(&lists, issuer)) {
            (Some(c), Some(i)) => link_signed(c, i),
            _ => false,
        }
    };
    let evaluation = evaluate(&chain, &anchors, &policy, now, &verify);
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

/// The certificates named by `key`, with their signing material in the same order.
fn certificates(params: &Json, key: &str) -> Result<(Vec<Certificate>, Vec<Signing>), String> {
    match params.get(key) {
        None | Some(Json::Null) => Ok((Vec::new(), Vec::new())),
        Some(Json::Arr(items)) => {
            let certs = items.iter().map(certificate).collect::<Result<Vec<_>, _>>()?;
            let signing = items.iter().zip(&certs).map(|(j, c)| signing(j, c)).collect::<Result<Vec<_>, _>>()?;
            Ok((certs, signing))
        }
        Some(_) => Err(format!("{key} must be a list of certificates")),
    }
}

/// The signing fields of one certificate. Each is hex text; a field that is absent stays `None`.
fn signing(j: &Json, cert: &Certificate) -> Result<Signing, String> {
    let field = |k: &str| -> Result<Option<Vec<u8>>, String> {
        match j.str_field(k) {
            None => Ok(None),
            Some(hex) => hex_decode(&hex).map(Some).ok_or_else(|| format!("certificate {k} must be hex text")),
        }
    };
    let public_key = field("public_key")?;
    let tbs = to_be_signed(cert, public_key.as_deref());
    Ok(Signing { public_key, signed: field("signed")?, signature: field("signature")?, tbs })
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
    use bifrost_core::plist::hex_encode;
    use bifrost_ui::ed25519::PrivateKey;

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
    fn unsigned_links_still_fail() {
        let chain = vec![cert("example.com", "Root", false), cert("Root", "Root", true)];
        let r = answer("evaluate", &request(chain, Some("example.com"))).unwrap();
        assert_eq!(r.get("passes"), Some(&Json::Bool(false)));
        let failures = r.get("failures").and_then(Json::as_array).unwrap();
        assert!(!failures.is_empty());
    }

    /// The certificate `subject` issued by `issuer`, holding `own`'s public key and signed by `signer` over its fields.
    fn signed(subject: &str, issuer: &str, is_ca: bool, own: &PrivateKey, signer: &PrivateKey) -> Json {
        let mut c = cert(subject, issuer, is_ca);
        let own_key = own.public_key().raw_representation();
        let tbs = to_be_signed(&certificate(&c).unwrap(), Some(&own_key));
        let signature = signer.signature_for(&tbs);
        if let Json::Obj(pairs) = &mut c {
            pairs.push(("public_key".into(), Json::Str(hex_encode(&own.public_key().raw_representation()))));
            pairs.push(("signed".into(), Json::Str(hex_encode(&tbs))));
            pairs.push(("signature".into(), Json::Str(hex_encode(&signature))));
        }
        c
    }

    fn chain_with_anchor(leaf: Json, root: Json) -> Json {
        Json::Obj(vec![
            ("chain".into(), Json::Arr(vec![leaf, root.clone()])),
            ("anchors".into(), Json::Arr(vec![root])),
            ("now".into(), Json::Num(1_000.0)),
            ("host".into(), Json::Str("example.com".into())),
        ])
    }

    #[test]
    fn a_chain_signed_by_its_anchor_passes() {
        let root_key = PrivateKey::generate().unwrap();
        let leaf_key = PrivateKey::generate().unwrap();
        let root = signed("Root", "Root", true, &root_key, &root_key);
        let leaf = signed("example.com", "Root", false, &leaf_key, &root_key);
        let r = answer("evaluate", &chain_with_anchor(leaf, root)).unwrap();
        assert_eq!(r.get("passes"), Some(&Json::Bool(true)), "{r:?}");
    }

    #[test]
    fn a_leaf_signed_by_another_key_or_altered_after_signing_fails() {
        let root_key = PrivateKey::generate().unwrap();
        let other_key = PrivateKey::generate().unwrap();
        let leaf_key = PrivateKey::generate().unwrap();
        let root = signed("Root", "Root", true, &root_key, &root_key);
        let forged = signed("example.com", "Root", false, &leaf_key, &other_key);
        let r = answer("evaluate", &chain_with_anchor(forged, root.clone())).unwrap();
        assert_eq!(r.get("passes"), Some(&Json::Bool(false)));
        assert!(format!("{:?}", r.get("failures")).contains("SignatureInvalid"));

        let mut altered = signed("example.com", "Root", false, &leaf_key, &root_key);
        if let Json::Obj(pairs) = &mut altered {
            for (k, v) in pairs.iter_mut() {
                if k == "subject" {
                    *v = Json::Str("evil.example".into());
                }
            }
        }
        let r = answer("evaluate", &chain_with_anchor(altered, root)).unwrap();
        assert_eq!(r.get("passes"), Some(&Json::Bool(false)));
    }

    #[test]
    fn a_hex_field_that_is_not_hex_is_an_error() {
        let mut c = cert("example.com", "Root", false);
        if let Json::Obj(pairs) = &mut c {
            pairs.push(("signature".into(), Json::Str("zz".into())));
        }
        assert!(answer("evaluate", &request(vec![c], None)).is_err());
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
