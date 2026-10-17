//! Hand text to Mail's compose window: `mail_compose::compose` opens a new message in the app that opens `mailto:` links
//! (the default for the scheme, Mail by default), with the recipients, copies, subject and body filled in.
//!
//! ```ignore
//! let mut draft = Compose::default();
//! draft.to.push("ada@example.org".into());
//! draft.subject = "Notes".into();
//! draft.body = "Attached below.".into();
//! mail_compose::compose(ctx, &draft)?;
//! ```
//!
//! The text travels as a `mailto:` URL (`controls::compose_handoff::Compose::to_url`) through the shell's URL routing,
//! so it reaches the default mail app as `Input::Open`. Attachments are not sent yet: a URL cannot carry files, and the
//! file grants that would hand Mail the files are not wired, so [`compose`] refuses a draft that has any.

use crate::controls::compose_handoff::Compose;
use crate::runtime::Ctx;

/// Why a draft was not handed to Mail.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ComposeError {
    /// The draft has no recipient, subject, body or attachment, so there is nothing to show.
    Empty,
    /// The draft names files. They cannot travel in a `mailto:` link, and the file grants are not wired yet.
    Attachments,
}

impl std::fmt::Display for ComposeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ComposeError::Empty => write!(f, "the message is empty"),
            ComposeError::Attachments => write!(f, "files cannot be handed to Mail yet"),
        }
    }
}

/// Opens a new message in Mail, prefilled from `draft`. Returns an error when nothing can be handed over.
pub fn compose(ctx: &Ctx, draft: &Compose) -> Result<(), ComposeError> {
    if draft.is_empty() {
        return Err(ComposeError::Empty);
    }
    if !draft.attachments.is_empty() {
        return Err(ComposeError::Attachments);
    }
    ctx.open_url(&draft.to_url());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_draft_has_nothing_to_hand_over() {
        assert!(Compose::default().is_empty());
        assert_eq!(ComposeError::Empty.to_string(), "the message is empty");
    }

    #[test]
    fn a_draft_with_a_file_is_refused_until_file_grants_are_wired() {
        let mut draft = Compose { subject: "Hi".into(), ..Compose::default() };
        assert!(draft.attach("/home/olibuijr/report.pdf"));
        assert!(!draft.attachments.is_empty());
        assert_eq!(ComposeError::Attachments.to_string(), "files cannot be handed to Mail yet");
    }

    #[test]
    fn the_handed_over_url_reads_back_as_the_same_draft() {
        let draft = Compose { to: vec!["ada@example.org".into()], subject: "Notes".into(), body: "Line one\nTwo".into(), ..Compose::default() };
        let back = Compose::from_url(&draft.to_url()).expect("a mailto URL");
        assert_eq!(back.to, draft.to);
        assert_eq!(back.subject, draft.subject);
        assert_eq!(back.body, draft.body);
    }
}
