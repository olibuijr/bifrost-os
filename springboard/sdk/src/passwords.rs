//! Website logins for apps: save a login once, fill it in later. A login is one item of the shared passwords group
//! ([`GROUP`], `is.bifrost.passwords`): the service is the site and the account is the user name, so the Settings
//! Passwords page lists every login an app saved, and an app that fills a form reads the same item back.
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

pub use keychain_groups::Error;

/// The shared group the Passwords page reads. Apps that save logins list it in `keychain_groups`.
pub const GROUP: &str = "is.bifrost.passwords";

/// Saves a new login for `site` and `account`. Answers `DuplicateItem` when the login already exists.
pub fn save(ctx: &Ctx, site: &str, account: &str, password: &str) -> u32 {
    keychain_groups::add(ctx, GROUP, site, account, password.as_bytes())
}

/// Replaces the password of an existing login. Answers `ItemNotFound` when there is none.
pub fn update(ctx: &Ctx, site: &str, account: &str, password: &str) -> u32 {
    keychain_groups::update(ctx, GROUP, site, account, password.as_bytes())
}

/// Asks for the password of `site` and `account`. Answer: [`decode_password`].
pub fn fill(ctx: &Ctx, site: &str, account: &str) -> u32 {
    keychain_groups::get(ctx, GROUP, site, account)
}

/// Deletes a login. Answers `ItemNotFound` when there is none.
pub fn remove(ctx: &Ctx, site: &str, account: &str) -> u32 {
    keychain_groups::delete(ctx, GROUP, site, account)
}

/// Decodes the answer to [`fill`] (or to [`save`] and [`update`], where the password is empty). A password that is
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
