//! Website logins, secure notes and certificates for apps: save once, read later. Every item is one entry of the shared
//! passwords group ([`GROUP`], `is.bifrost.passwords`), which the Settings Passwords page and the Passwords app both
//! list. A login's service is the site and its account is the user name, so an app that fills a form reads the same
//! item back. A note or certificate is kept under a kind prefix (see [`Kind`]); [`kind_of`] tells them apart in a
//! listing. The item model lives in the shell's `credentials` module, so the shell and the SDK share one definition.
//!
//! The app lists [`GROUP`] under `keychain_groups` in its manifest and asks for the `keychain` permission; without
//! both, every call answers `NotPermitted` (see [`keychain_groups`](crate::keychain_groups)). Each call returns a
//! request id. The answer arrives in `App::service_reply`: [`decode_password`] gives the password for [`fill`], and
//! for [`save`] an `Error::DuplicateItem` means the login exists, so the app calls [`update`] instead.
//!
//! ```ignore
//! let id = passwords::save(ctx, "example.com", "alice", "hunter2");
//! // later, in App::service_reply(ctx, request, result):
//! //   passwords::decode_password(result)  ->  Ok("hunter2") for fill
//! ```

use crate::keychain_groups;
use crate::Ctx;
use bifrost_core::json::Json;
use springboard::credentials::{certificate_service, note_service, CERTIFICATE_ACCOUNT, NOTE_ACCOUNT};

pub use keychain_groups::Error;
pub use springboard::credentials::{kind_of, name_of, Kind, GROUP};

/// Saves a new login for `site` and `account`. Answers `DuplicateItem` when the login already exists.
pub fn save(ctx: &Ctx, site: &str, account: &str, password: &str) -> u32 {
    keychain_groups::add(ctx, GROUP, site, account, password.as_bytes())
}

/// Saves a secure note titled `title` with `body`. Answers `DuplicateItem` when a note has that title.
pub fn save_note(ctx: &Ctx, title: &str, body: &str) -> u32 {
    keychain_groups::add(ctx, GROUP, &note_service(title), NOTE_ACCOUNT, body.as_bytes())
}

/// Saves a certificate labelled `label` as PEM text. Answers `DuplicateItem` when a certificate has that label.
pub fn save_certificate(ctx: &Ctx, label: &str, pem: &str) -> u32 {
    keychain_groups::add(ctx, GROUP, &certificate_service(label), CERTIFICATE_ACCOUNT, pem.as_bytes())
}

/// Replaces the password of an existing login. Answers `ItemNotFound` when there is none.
pub fn update(ctx: &Ctx, site: &str, account: &str, password: &str) -> u32 {
    keychain_groups::update(ctx, GROUP, site, account, password.as_bytes())
}

/// Asks for the password of `site` and `account`. Answer: [`decode_password`].
pub fn fill(ctx: &Ctx, site: &str, account: &str) -> u32 {
    keychain_groups::get(ctx, GROUP, site, account)
}

/// Deletes a login, note or certificate by its service and account. Answers `ItemNotFound` when there is none.
pub fn remove(ctx: &Ctx, site: &str, account: &str) -> u32 {
    keychain_groups::delete(ctx, GROUP, site, account)
}

/// Lists every item in the group (logins, notes and certificates), with no secrets. Answer: [`decode_list`].
pub fn list(ctx: &Ctx) -> u32 {
    keychain_groups::list(ctx, GROUP)
}

/// Decodes the answer to [`list`]; `None` when the answer is an error or not a list.
pub fn decode_list(result: &Result<Json, String>) -> Option<Vec<keychain_groups::Row>> {
    keychain_groups::decode_list(result)
}

/// Decodes the answer to [`fill`] (the password, the note body or the certificate's PEM text), or to [`save`] and
/// [`update`], where the value is empty. A password that is
/// not text is `Error::Param`.
pub fn decode_password(result: &Result<Json, String>) -> Result<String, Error> {
    let bytes = keychain_groups::decode(result)?;
    String::from_utf8(bytes).map_err(|_| Error::Param)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_refused_request_decodes_as_a_param_error() {
        assert_eq!(decode_password(&Err("no such method".into())), Err(Error::Param));
        assert_eq!(GROUP, "is.bifrost.passwords");
    }
}
