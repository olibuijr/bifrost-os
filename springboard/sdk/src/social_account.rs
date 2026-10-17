//! Signed-in social accounts (`SLRequest.account` parity, see `docs/parity/social.md`).
//!
//! An app sends a service request as the user's account for that service. This module has three parts:
//! - [`AccountStore`]: the accounts the user has added, as metadata only (service type, user name, how the account
//!   signs). The store answers whether an account exists for a service, which is what `isAvailable` needs.
//! - [`Credentials`]: the secret material for one account. It is kept in the keychain, under
//!   [`KEYCHAIN_SERVICE`] and [`SocialAccount::keychain_account`], in the form [`Credentials::to_secret_bytes`] writes.
//! - [`sign`]: signs a [`URLRequest`] in place. A bearer token becomes `Authorization: Bearer`. OAuth 1.0a (RFC 5849)
//!   with `HMAC-SHA1` becomes an `Authorization: OAuth` header whose signature covers the method, the URL without its
//!   query or fragment, and every parameter from the query, a form body and the `oauth_*` set.
//!
//! The caller supplies the timestamp and the nonce, so signing is repeatable and testable. Not mirrored: the sign-in
//! flow (no account provider is registered in the shell) and OAuth 2 flows. The signature is checked against the
//! RFC 2202 HMAC-SHA1 vectors and the base string rules, not yet against a live service.

use crate::digest::insecure;
use crate::http_auth::base64_encode;
use crate::social::{encode, ServiceRequest};
use crate::url::URLRequest;
use std::fmt;

/// The keychain service name under which account secrets are stored.
pub const KEYCHAIN_SERVICE: &str = "com.bifrost.social.accounts";

/// How an account signs its requests.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuthMethod {
    /// `Authorization: Bearer <token>`.
    Bearer,
    /// OAuth 1.0a with `HMAC-SHA1`.
    OAuth1,
}

/// An account's metadata. It holds no secret.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SocialAccount {
    pub service_type: String,
    pub username: String,
    pub method: AuthMethod,
}

impl SocialAccount {
    pub fn new(service_type: &str, username: &str, method: AuthMethod) -> Self {
        SocialAccount { service_type: service_type.to_string(), username: username.to_string(), method }
    }

    /// The keychain account name that holds this account's secret: `service:username`.
    pub fn keychain_account(&self) -> String {
        format!("{}:{}", self.service_type, self.username)
    }
}

/// The secret material for one account. The `Debug` output hides the secrets.
#[derive(Clone, PartialEq, Eq)]
pub enum Credentials {
    OAuth1 { consumer_key: String, consumer_secret: String, token: String, token_secret: String },
    Bearer { token: String },
}

impl fmt::Debug for Credentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Credentials::OAuth1 { consumer_key, .. } => {
                write!(f, "Credentials::OAuth1 {{ consumer_key: {consumer_key:?}, secrets: <hidden> }}")
            }
            Credentials::Bearer { .. } => write!(f, "Credentials::Bearer {{ token: <hidden> }}"),
        }
    }
}

/// Why an account, its secret or a signature was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AccountError {
    /// A required field is empty; the name says which one.
    EmptyField(&'static str),
    /// A field contains a line break, which the keychain form cannot hold.
    LineBreak(&'static str),
    /// An account with the same service type and user name is already stored.
    Duplicate,
    /// The stored secret does not have the fields its method needs.
    BadSecret,
    /// The URL has no `scheme://host` part, so it cannot be signed.
    BadUrl,
    /// The service request cannot be prepared (uploads on GET or DELETE).
    BadRequest,
}

impl fmt::Display for AccountError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AccountError::EmptyField(name) => write!(f, "{name} is empty"),
            AccountError::LineBreak(name) => write!(f, "{name} contains a line break"),
            AccountError::Duplicate => write!(f, "an account with this name already exists"),
            AccountError::BadSecret => write!(f, "the stored account secret is malformed"),
            AccountError::BadUrl => write!(f, "the request URL has no scheme and host"),
            AccountError::BadRequest => write!(f, "the service request cannot be prepared"),
        }
    }
}

impl Credentials {
    /// The method these credentials sign with.
    pub fn method(&self) -> AuthMethod {
        match self {
            Credentials::OAuth1 { .. } => AuthMethod::OAuth1,
            Credentials::Bearer { .. } => AuthMethod::Bearer,
        }
    }

    /// Encodes the secret for the keychain: the fields in order, one per line. The consumer key and secret, and a
    /// bearer token, must not be empty. The OAuth token fields may be empty before authorization.
    pub fn to_secret_bytes(&self) -> Result<Vec<u8>, AccountError> {
        let fields: Vec<(&'static str, &str)> = match self {
            Credentials::OAuth1 { consumer_key, consumer_secret, token, token_secret } => {
                require("consumer key", consumer_key)?;
                require("consumer secret", consumer_secret)?;
                vec![
                    ("consumer key", consumer_key.as_str()),
                    ("consumer secret", consumer_secret.as_str()),
                    ("token", token.as_str()),
                    ("token secret", token_secret.as_str()),
                ]
            }
            Credentials::Bearer { token } => {
                require("token", token)?;
                vec![("token", token.as_str())]
            }
        };
        let mut lines = Vec::with_capacity(fields.len());
        for (name, value) in fields {
            if value.contains(['\n', '\r']) {
                return Err(AccountError::LineBreak(name));
            }
            lines.push(value);
        }
        Ok(lines.join("\n").into_bytes())
    }

    /// Reads the keychain form written by [`Credentials::to_secret_bytes`] for an account with `method`.
    pub fn from_secret_bytes(method: AuthMethod, bytes: &[u8]) -> Result<Credentials, AccountError> {
        let text = std::str::from_utf8(bytes).map_err(|_| AccountError::BadSecret)?;
        let parts: Vec<&str> = text.split('\n').collect();
        match (method, parts.as_slice()) {
            (AuthMethod::OAuth1, [key, secret, token, token_secret]) => Ok(Credentials::OAuth1 {
                consumer_key: key.to_string(),
                consumer_secret: secret.to_string(),
                token: token.to_string(),
                token_secret: token_secret.to_string(),
            }),
            (AuthMethod::Bearer, [token]) if !token.is_empty() => Ok(Credentials::Bearer { token: token.to_string() }),
            _ => Err(AccountError::BadSecret),
        }
    }
}

fn require(name: &'static str, value: &str) -> Result<(), AccountError> {
    if value.is_empty() {
        Err(AccountError::EmptyField(name))
    } else {
        Ok(())
    }
}

/// The accounts the user has added. The store holds no secrets; see [`Credentials`].
#[derive(Clone, Debug, Default)]
pub struct AccountStore {
    accounts: Vec<SocialAccount>,
}

impl AccountStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds an account. The service type and user name must not be empty, and the pair must be new.
    pub fn add(&mut self, account: SocialAccount) -> Result<(), AccountError> {
        require("service type", &account.service_type)?;
        require("user name", &account.username)?;
        if self.find(&account.service_type, &account.username).is_some() {
            return Err(AccountError::Duplicate);
        }
        self.accounts.push(account);
        Ok(())
    }

    /// Removes an account. Returns false when there was none. The caller deletes its keychain item.
    pub fn remove(&mut self, service_type: &str, username: &str) -> bool {
        match self.accounts.iter().position(|a| a.service_type == service_type && a.username == username) {
            Some(i) => {
                self.accounts.remove(i);
                true
            }
            None => false,
        }
    }

    /// The accounts for one service type, in the order they were added.
    pub fn accounts_for(&self, service_type: &str) -> Vec<&SocialAccount> {
        self.accounts.iter().filter(|a| a.service_type == service_type).collect()
    }

    /// Whether a request for the service type can be signed now: at least one account exists for it.
    pub fn has_service(&self, service_type: &str) -> bool {
        self.accounts.iter().any(|a| a.service_type == service_type)
    }

    pub fn find(&self, service_type: &str, username: &str) -> Option<&SocialAccount> {
        self.accounts.iter().find(|a| a.service_type == service_type && a.username == username)
    }

    pub fn accounts(&self) -> &[SocialAccount] {
        &self.accounts
    }

    pub fn len(&self) -> usize {
        self.accounts.len()
    }

    pub fn is_empty(&self) -> bool {
        self.accounts.is_empty()
    }
}

/// Signs `req` in place with `creds`. `timestamp` is seconds since the epoch and `nonce` a string unique to this
/// request. A bearer token sets `Authorization: Bearer`. OAuth 1.0a sets `Authorization: OAuth`, with the signature
/// over the base string from [`signature_base_string`].
pub fn sign(req: &mut URLRequest, creds: &Credentials, timestamp: u64, nonce: &str) -> Result<(), AccountError> {
    match creds {
        Credentials::Bearer { token } => {
            require("token", token)?;
            req.set_header("Authorization", &format!("Bearer {token}"));
            Ok(())
        }
        Credentials::OAuth1 { consumer_key, consumer_secret, token, token_secret } => {
            require("consumer key", consumer_key)?;
            require("consumer secret", consumer_secret)?;
            require("nonce", nonce)?;
            let mut oauth = oauth_params(consumer_key, token, timestamp, nonce);
            let base = signature_base_string(req, &oauth)?;
            // The signing key is the two secrets, each percent-encoded, joined with '&' (RFC 5849 3.4.2).
            let key = format!("{}&{}", encode(consumer_secret, false), encode(token_secret, false));
            oauth.push(("oauth_signature".to_string(), base64_encode(&hmac_sha1(key.as_bytes(), base.as_bytes()))));
            let header = oauth
                .iter()
                .map(|(k, v)| format!("{}=\"{}\"", encode(k, false), encode(v, false)))
                .collect::<Vec<_>>()
                .join(", ");
            req.set_header("Authorization", &format!("OAuth {header}"));
            Ok(())
        }
    }
}

/// Prepares a service request and signs it. This is the request to send with `URLSession`.
pub fn signed_service_request(
    service: &ServiceRequest,
    creds: &Credentials,
    timestamp: u64,
    nonce: &str,
) -> Result<URLRequest, AccountError> {
    let mut req = service.prepared_url_request().map_err(|_| AccountError::BadRequest)?;
    sign(&mut req, creds, timestamp, nonce)?;
    Ok(req)
}

/// The `oauth_*` parameters of a request, without the signature. `oauth_token` is left out while the token is empty.
fn oauth_params(consumer_key: &str, token: &str, timestamp: u64, nonce: &str) -> Vec<(String, String)> {
    let mut params = vec![
        ("oauth_consumer_key".to_string(), consumer_key.to_string()),
        ("oauth_nonce".to_string(), nonce.to_string()),
        ("oauth_signature_method".to_string(), "HMAC-SHA1".to_string()),
        ("oauth_timestamp".to_string(), timestamp.to_string()),
    ];
    if !token.is_empty() {
        params.push(("oauth_token".to_string(), token.to_string()));
    }
    params.push(("oauth_version".to_string(), "1.0".to_string()));
    params
}

/// The OAuth 1.0a signature base string (RFC 5849 3.4.1): the upper-case method, the normalized URL and the sorted,
/// encoded parameters, each part percent-encoded and joined with `&`.
pub fn signature_base_string(req: &URLRequest, oauth: &[(String, String)]) -> Result<String, AccountError> {
    let (base_url, query) = split_url(&req.url)?;
    let mut params: Vec<(String, String)> = Vec::new();
    collect_pairs(&query, &mut params);
    if let Some(body) = &req.body {
        if is_form_body(req) {
            collect_pairs(&String::from_utf8_lossy(body), &mut params);
        }
    }
    params.extend(oauth.iter().cloned());
    let mut encoded: Vec<(String, String)> = params.iter().map(|(k, v)| (encode(k, false), encode(v, false))).collect();
    encoded.sort();
    let normalized = encoded.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join("&");
    Ok(format!("{}&{}&{}", req.method.to_ascii_uppercase(), encode(&base_url, false), encode(&normalized, false)))
}

fn is_form_body(req: &URLRequest) -> bool {
    req.headers.iter().any(|(k, v)| {
        k.eq_ignore_ascii_case("Content-Type")
            && v.trim().to_ascii_lowercase().starts_with("application/x-www-form-urlencoded")
    })
}

/// Splits a URL into its normalized base (lower-case scheme and host, default port removed, `/` for an empty path)
/// and its raw query. The fragment is dropped.
fn split_url(url: &str) -> Result<(String, String), AccountError> {
    let without_fragment = url.split('#').next().unwrap_or("");
    let (rest, query) = match without_fragment.split_once('?') {
        Some((rest, query)) => (rest, query),
        None => (without_fragment, ""),
    };
    let (scheme, after_scheme) = rest.split_once("://").ok_or(AccountError::BadUrl)?;
    let scheme = scheme.to_ascii_lowercase();
    let (authority, path) = match after_scheme.find('/') {
        Some(i) => (&after_scheme[..i], &after_scheme[i..]),
        None => (after_scheme, ""),
    };
    let mut authority = authority.to_ascii_lowercase();
    let default_port = match scheme.as_str() {
        "http" => ":80",
        "https" => ":443",
        _ => "",
    };
    if !default_port.is_empty() && authority.ends_with(default_port) {
        authority.truncate(authority.len() - default_port.len());
    }
    if authority.is_empty() {
        return Err(AccountError::BadUrl);
    }
    let path = if path.is_empty() { "/" } else { path };
    Ok((format!("{scheme}://{authority}{path}"), query.to_string()))
}

/// Parses `k=v&k=v` into decoded pairs. A key without `=` has an empty value.
fn collect_pairs(s: &str, out: &mut Vec<(String, String)>) {
    for part in s.split('&').filter(|p| !p.is_empty()) {
        let (k, v) = part.split_once('=').unwrap_or((part, ""));
        out.push((decode(k), decode(v)));
    }
}

/// Percent-decodes a form or query component. `+` is a space.
fn decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok());
                match hex {
                    Some(byte) => {
                        out.push(byte);
                        i += 2;
                    }
                    None => out.push(b'%'),
                }
            }
            b => out.push(b),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// HMAC-SHA1 (RFC 2104) with the 64-byte SHA-1 block size. Used only for OAuth 1.0a, which requires it.
pub(crate) fn hmac_sha1(key: &[u8], message: &[u8]) -> [u8; 20] {
    const BLOCK: usize = 64;
    let mut block = [0u8; BLOCK];
    if key.len() > BLOCK {
        block[..20].copy_from_slice(&insecure::sha1(key));
    } else {
        block[..key.len()].copy_from_slice(key);
    }
    let inner: Vec<u8> = block.iter().map(|b| b ^ 0x36).chain(message.iter().copied()).collect();
    let inner_digest = insecure::sha1(&inner);
    let outer: Vec<u8> = block.iter().map(|b| b ^ 0x5c).chain(inner_digest.iter().copied()).collect();
    insecure::sha1(&outer)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    fn oauth1(token: &str, token_secret: &str) -> Credentials {
        Credentials::OAuth1 {
            consumer_key: "k".into(),
            consumer_secret: "s".into(),
            token: token.into(),
            token_secret: token_secret.into(),
        }
    }

    #[test]
    fn hmac_sha1_matches_the_rfc_2202_vectors() {
        assert_eq!(hex(&hmac_sha1(&[0x0b; 20], b"Hi There")), "b617318655057264e28bc0b6fb378c8ef146be00");
        assert_eq!(hex(&hmac_sha1(b"Jefe", b"what do ya want for nothing?")), "effcdf6ae5eb2fa2d27416d5f184df9c259a7c79");
    }

    #[test]
    fn base_string_normalizes_the_url_and_sorts_every_parameter() {
        let mut req = URLRequest::post("https://Example.COM:443/resource?b=2&a=1", b"c=3".to_vec());
        req.set_header("Content-Type", "application/x-www-form-urlencoded");
        let oauth = oauth_params("k", "t", 100, "n1");
        let base = signature_base_string(&req, &oauth).unwrap();
        assert_eq!(
            base,
            "POST&https%3A%2F%2Fexample.com%2Fresource&a%3D1%26b%3D2%26c%3D3%26oauth_consumer_key%3Dk%26oauth_nonce%3Dn1%26oauth_signature_method%3DHMAC-SHA1%26oauth_timestamp%3D100%26oauth_token%3Dt%26oauth_version%3D1.0"
        );
    }

    #[test]
    fn a_multipart_body_is_not_part_of_the_base_string() {
        let mut req = URLRequest::post("https://example.test/up", b"x=1".to_vec());
        req.set_header("Content-Type", "multipart/form-data; boundary=b");
        let base = signature_base_string(&req, &oauth_params("k", "", 1, "n")).unwrap();
        assert!(!base.contains("x%3D1"));
    }

    #[test]
    fn the_url_drops_fragment_and_default_port_and_gets_a_root_path() {
        assert_eq!(split_url("HTTP://Host:80#frag").unwrap(), ("http://host/".to_string(), String::new()));
        assert_eq!(split_url("https://h.test:8443/a?x=1").unwrap(), ("https://h.test:8443/a".to_string(), "x=1".to_string()));
        assert_eq!(split_url("example.test/x"), Err(AccountError::BadUrl));
        assert_eq!(split_url("https:///x"), Err(AccountError::BadUrl));
    }

    #[test]
    fn decode_handles_plus_percent_and_bad_escapes() {
        assert_eq!(decode("a+b%20c%2B"), "a b c+");
        assert_eq!(decode("100%"), "100%");
        assert_eq!(decode("%zz"), "%zz");
    }

    #[test]
    fn oauth_header_carries_the_params_and_a_signature() {
        let mut req = URLRequest::new("https://example.test/feed?q=1");
        sign(&mut req, &oauth1("t", "ts"), 100, "n1").unwrap();
        let header = req.headers.iter().find(|(k, _)| k == "Authorization").map(|(_, v)| v.clone()).unwrap();
        assert!(header.starts_with("OAuth "));
        assert!(header.contains("oauth_consumer_key=\"k\""));
        assert!(header.contains("oauth_token=\"t\""));
        assert!(header.contains("oauth_signature_method=\"HMAC-SHA1\""));
        assert!(header.contains("oauth_signature=\""));
        assert!(!req.url.contains("oauth_"));
    }

    #[test]
    fn the_signature_depends_on_the_token_secret_and_the_nonce() {
        let sig = |creds: &Credentials, nonce: &str| {
            let mut req = URLRequest::new("https://example.test/feed");
            sign(&mut req, creds, 100, nonce).unwrap();
            req.headers.iter().find(|(k, _)| k == "Authorization").unwrap().1.clone()
        };
        assert_eq!(sig(&oauth1("t", "a"), "n"), sig(&oauth1("t", "a"), "n"));
        assert_ne!(sig(&oauth1("t", "a"), "n"), sig(&oauth1("t", "b"), "n"));
        assert_ne!(sig(&oauth1("t", "a"), "n"), sig(&oauth1("t", "a"), "m"));
    }

    #[test]
    fn a_bearer_token_sets_the_authorization_header() {
        let mut req = URLRequest::new("https://example.test/feed");
        sign(&mut req, &Credentials::Bearer { token: "abc".into() }, 0, "").unwrap();
        assert!(req.headers.contains(&("Authorization".to_string(), "Bearer abc".to_string())));
    }

    #[test]
    fn empty_secrets_and_nonces_are_refused() {
        let mut req = URLRequest::new("https://example.test/feed");
        assert_eq!(sign(&mut req, &Credentials::Bearer { token: String::new() }, 0, ""), Err(AccountError::EmptyField("token")));
        assert_eq!(sign(&mut req, &oauth1("t", "s"), 0, ""), Err(AccountError::EmptyField("nonce")));
    }

    #[test]
    fn credentials_round_trip_through_the_keychain_form() {
        let creds = oauth1("", "");
        let bytes = creds.to_secret_bytes().unwrap();
        assert_eq!(Credentials::from_secret_bytes(AuthMethod::OAuth1, &bytes).unwrap(), creds);
        let bearer = Credentials::Bearer { token: "tok".into() };
        let bytes = bearer.to_secret_bytes().unwrap();
        assert_eq!(bytes, b"tok");
        assert_eq!(Credentials::from_secret_bytes(AuthMethod::Bearer, &bytes).unwrap(), bearer);
    }

    #[test]
    fn malformed_secrets_are_refused() {
        assert_eq!(Credentials::from_secret_bytes(AuthMethod::OAuth1, b"a\nb"), Err(AccountError::BadSecret));
        assert_eq!(Credentials::from_secret_bytes(AuthMethod::Bearer, b""), Err(AccountError::BadSecret));
        let bad = Credentials::Bearer { token: "a\nb".into() };
        assert_eq!(bad.to_secret_bytes(), Err(AccountError::LineBreak("token")));
        let empty_key = Credentials::OAuth1 { consumer_key: String::new(), consumer_secret: "s".into(), token: String::new(), token_secret: String::new() };
        assert_eq!(empty_key.to_secret_bytes(), Err(AccountError::EmptyField("consumer key")));
    }

    #[test]
    fn debug_output_hides_the_secrets() {
        let text = format!("{:?}", oauth1("tok", "tsecret"));
        assert!(!text.contains("tsecret"));
        assert!(!text.contains("\"s\""));
    }

    #[test]
    fn the_store_refuses_duplicates_and_empty_names() {
        let mut store = AccountStore::new();
        store.add(SocialAccount::new("feed", "ann", AuthMethod::Bearer)).unwrap();
        assert_eq!(store.add(SocialAccount::new("feed", "ann", AuthMethod::OAuth1)), Err(AccountError::Duplicate));
        assert_eq!(store.add(SocialAccount::new("", "bo", AuthMethod::Bearer)), Err(AccountError::EmptyField("service type")));
        assert_eq!(store.add(SocialAccount::new("feed", "", AuthMethod::Bearer)), Err(AccountError::EmptyField("user name")));
        store.add(SocialAccount::new("feed", "bo", AuthMethod::Bearer)).unwrap();
        assert_eq!(store.len(), 2);
    }

    #[test]
    fn the_store_answers_availability_and_removes_accounts() {
        let mut store = AccountStore::new();
        assert!(!store.has_service("feed"));
        store.add(SocialAccount::new("feed", "ann", AuthMethod::Bearer)).unwrap();
        store.add(SocialAccount::new("photos", "ann", AuthMethod::OAuth1)).unwrap();
        assert!(store.has_service("feed"));
        assert_eq!(store.accounts_for("feed").len(), 1);
        assert!(store.remove("feed", "ann"));
        assert!(!store.remove("feed", "ann"));
        assert!(!store.has_service("feed"));
        assert!(store.has_service("photos"));
    }

    #[test]
    fn the_keychain_account_name_joins_service_and_user() {
        assert_eq!(SocialAccount::new("feed", "ann", AuthMethod::Bearer).keychain_account(), "feed:ann");
    }

    #[test]
    fn a_service_request_is_prepared_then_signed() {
        let svc = ServiceRequest::new(crate::social::RequestMethod::Get, "https://example.test/feed", &[("q", "a b")]);
        let req = signed_service_request(&svc, &Credentials::Bearer { token: "abc".into() }, 0, "").unwrap();
        assert_eq!(req.url, "https://example.test/feed?q=a%20b");
        assert!(req.headers.contains(&("Authorization".to_string(), "Bearer abc".to_string())));
        let mut up = ServiceRequest::new(crate::social::RequestMethod::Get, "https://example.test/up", &[]);
        up.add_multipart_data(b"x", "f", "text/plain", None);
        assert_eq!(signed_service_request(&up, &Credentials::Bearer { token: "abc".into() }, 0, "").unwrap_err(), AccountError::BadRequest);
    }
}
