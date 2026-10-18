//! The `mail_compose` service: hands a message to Mail's compose window (the SDK client is `bifrost_app::mail_compose`).
//!
//! Methods (JSON params):
//! - `compose` `{url, files}`: `url` is a `mailto:` URL with the recipients, copies, subject and body (no files);
//!   `files` are the read grants the calling app holds, each as 32 hex digits. Mail opens the link as a new message, and
//!   each file is granted to Mail read-only and attached to that message. Answer: `{}`.
use super::SystemService;
use crate::shell::Shell;
use bifrost_core::json::Json;
use bifrost_proto::GrantId;

pub struct MailCompose;

const BAD_GRANT: &str = "a grant is 32 hex digits";

/// One grant, from its 32 hex digits.
pub fn parse_grant(s: &str) -> Result<GrantId, String> {
    if s.len() != 32 || !s.is_ascii() {
        return Err(BAD_GRANT.into());
    }
    let mut out = [0u8; 16];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&s[2 * i..2 * i + 2], 16).map_err(|_| BAD_GRANT.to_string())?;
    }
    Ok(out)
}

/// The grants in the `files` parameter. Missing or empty is no files.
fn grants_param(params: &Json) -> Result<Vec<GrantId>, String> {
    let Some(items) = params.get("files").and_then(Json::as_array) else { return Ok(Vec::new()) };
    items.iter().map(|item| parse_grant(item.as_str().ok_or(BAD_GRANT)?)).collect()
}

impl SystemService for MailCompose {
    fn name(&self) -> &'static str {
        "mail_compose"
    }

    fn call(&mut self, shell: &mut Shell, app_id: &str, method: &str, params: &Json, t: f64) -> Result<Json, String> {
        match method {
            "compose" => {
                let url = params.get("url").and_then(Json::as_str).ok_or("needs a url string")?;
                if !url.to_ascii_lowercase().starts_with("mailto:") {
                    return Err("compose needs a mailto: URL".into());
                }
                let files = grants_param(params)?;
                shell.mail_compose_deliver(app_id, url, &files, t)?;
                Ok(Json::Obj(Vec::new()))
            }
            _ => Err(format!("mail_compose has no method {method}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_grant_reads_back_from_its_hex_digits() {
        let grant = parse_grant("000102030405060708090a0b0c0d0eff").expect("32 hex digits");
        assert_eq!(grant[0], 0x00);
        assert_eq!(grant[1], 0x01);
        assert_eq!(grant[15], 0xff);
    }

    #[test]
    fn a_grant_that_is_not_32_hex_digits_is_refused() {
        assert!(parse_grant("abc").is_err());
        assert!(parse_grant("zz0102030405060708090a0b0c0d0eff").is_err());
        assert!(parse_grant("é0102030405060708090a0b0c0d0eff").is_err());
    }

    #[test]
    fn the_files_parameter_is_optional() {
        let params = Json::Obj(vec![("url".to_string(), Json::Str("mailto:a@b.c".into()))]);
        assert_eq!(grants_param(&params), Ok(Vec::new()));
    }
}
