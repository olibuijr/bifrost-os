//! Hand text and files to Mail's compose window: `mail_compose::compose` opens a new message in the app that opens
//! `mailto:` links (Mail by default), with the recipients, copies, subject and body filled in, and attaches the files.
//!
//! ```ignore
//! let mut draft = Compose::default();
//! draft.to.push("ada@example.org".into());
//! draft.subject = "Notes".into();
//! draft.body = "The report is attached.".into();
//! mail_compose::compose(ctx, &draft, &[report.grant])?;
//! ```
//!
//! The text travels as a `mailto:` URL through the `mail_compose` shell service (`springboard/src/shell/services/mail_compose.rs`),
//! which opens Mail with it. Files travel as grants: pass the `grant` of each `Document` you hold (an open panel's file
//! or a file someone handed you). The shell grants each one to Mail read-only and Mail attaches it to the new message.
//! A file named by path in `draft.attachments` is refused, because a path is not a grant.

use crate::controls::compose_handoff::Compose;
use crate::proto::GrantId;
use crate::runtime::Ctx;
use crate::service;
use bifrost_core::json::Json;

/// The shell service that takes the hand-off, and its method.
pub const SERVICE: &str = "mail_compose";
pub const METHOD_COMPOSE: &str = "compose";

/// Why a draft was not handed to Mail.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ComposeError {
    /// The draft has no recipient, subject, body or file, so there is nothing to show.
    Empty,
    /// The draft names files by path. A path is not a grant, so pass the files' grants instead.
    PathAttachments,
}

impl std::fmt::Display for ComposeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ComposeError::Empty => write!(f, "the message is empty"),
            ComposeError::PathAttachments => write!(f, "files must be passed as grants, not paths"),
        }
    }
}

/// One grant as 32 hex digits, the form the shell service reads.
fn hex(grant: &GrantId) -> String {
    grant.iter().map(|b| format!("{b:02x}")).collect()
}

/// The parameters of the `compose` call: the `mailto:` URL and the files' grants.
pub fn compose_params(draft: &Compose, files: &[GrantId]) -> Json {
    Json::Obj(vec![
        ("url".to_string(), Json::Str(draft.to_url())),
        ("files".to_string(), Json::Arr(files.iter().map(|g| Json::Str(hex(g))).collect())),
    ])
}

/// Opens a new message in Mail, prefilled from `draft`, with each of `files` attached. Returns the request id of the
/// call. Errors are checked here; the shell's answer arrives as a service reply.
pub fn compose(ctx: &Ctx, draft: &Compose, files: &[GrantId]) -> Result<u32, ComposeError> {
    if !draft.attachments.is_empty() {
        return Err(ComposeError::PathAttachments);
    }
    if draft.is_empty() && files.is_empty() {
        return Err(ComposeError::Empty);
    }
    Ok(service::call(ctx, SERVICE, METHOD_COMPOSE, &compose_params(draft, files)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_error_and_a_path_error_say_what_is_wrong() {
        assert_eq!(ComposeError::Empty.to_string(), "the message is empty");
        assert_eq!(ComposeError::PathAttachments.to_string(), "files must be passed as grants, not paths");
    }

    #[test]
    fn a_grant_is_sent_as_its_32_hex_digits() {
        let mut grant = [0u8; 16];
        grant[0] = 0xab;
        grant[15] = 0x01;
        assert_eq!(hex(&grant), "ab000000000000000000000000000001");
    }

    #[test]
    fn the_params_carry_the_mailto_url_and_the_files() {
        let draft = Compose { to: vec!["ada@example.org".into()], subject: "Notes".into(), ..Compose::default() };
        let params = compose_params(&draft, &[[1u8; 16]]);
        assert_eq!(params.get("url").and_then(Json::as_str), Some("mailto:ada@example.org?subject=Notes"));
        let files = params.get("files").and_then(Json::as_array).expect("a files array");
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].as_str(), Some("01010101010101010101010101010101"));
    }
}
