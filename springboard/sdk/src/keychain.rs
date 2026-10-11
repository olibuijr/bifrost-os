//! Keychain (`SecItemAdd`, `SecItemCopyMatching`, `SecItemUpdate`, `SecItemDelete`, `kSecClassGenericPassword`):
//! secrets the system keeps for the app, out of reach of `defaults.txt` and the app's data directory.
//!
//! The calls go to the shell, which stores the items under this app's id (taken from the connection, so an app
//! cannot name another app's items). They are asynchronous like the document panels: each returns a request id and the
//! answer arrives in `App::keychain` as a [`Reply`]. Needs the `keychain` permission in the manifest; without it every
//! call answers `Error::NotPermitted`. Errors mirror's: `DuplicateItem` (errSecDuplicateItem), `ItemNotFound`
//! (errSecItemNotFound), `Param` (errSecParam). Limits: names up to 255 bytes, secrets up to 8 KiB, 256 items.

pub use springboard::keychain::Error;

use crate::proto::{KeychainOp, KeychainStatus, ToShell};
use crate::Ctx;

/// The answer to one keychain call.
#[derive(Debug, PartialEq)]
pub struct Reply {
    pub request: u32,
    /// `Ok(secret)` for [`get`]; `Ok(empty)` for the other calls
    pub result: Result<Vec<u8>, Error>,
}

impl Reply {
    pub(crate) fn from_wire(request: u32, status: KeychainStatus, value: Vec<u8>) -> Reply {
        Reply { request, result: match Error::from_status(status) { None => Ok(value), Some(e) => Err(e) } }
    }
}

fn call(ctx: &Ctx, op: KeychainOp) -> u32 {
    let request = ctx.next_request();
    ctx.send_to_shell(ToShell::Keychain { request, op });
    request
}

/// `SecItemAdd`: stores a new secret; the reply is `DuplicateItem` when `(service, account)` exists.
pub fn add(ctx: &Ctx, service: &str, account: &str, secret: &[u8]) -> u32 {
    call(ctx, KeychainOp::Add { service: service.into(), account: account.into(), value: secret.to_vec() })
}

/// `SecItemCopyMatching` with `kSecReturnData`: the reply carries the secret, or `ItemNotFound`.
pub fn get(ctx: &Ctx, service: &str, account: &str) -> u32 {
    call(ctx, KeychainOp::Get { service: service.into(), account: account.into() })
}

/// `SecItemUpdate`: replaces the secret; the reply is `ItemNotFound` when there is no such item.
pub fn update(ctx: &Ctx, service: &str, account: &str, secret: &[u8]) -> u32 {
    call(ctx, KeychainOp::Update { service: service.into(), account: account.into(), value: secret.to_vec() })
}

/// `SecItemDelete`: the reply is `ItemNotFound` when there is no such item.
pub fn delete(ctx: &Ctx, service: &str, account: &str) -> u32 {
    call(ctx, KeychainOp::Delete { service: service.into(), account: account.into() })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replies_map_statuses_to_style_errors() {
        assert_eq!(Reply::from_wire(1, KeychainStatus::Ok, b"s".to_vec()).result, Ok(b"s".to_vec()));
        assert_eq!(Reply::from_wire(2, KeychainStatus::Duplicate, vec![]).result, Err(Error::DuplicateItem));
        assert_eq!(Reply::from_wire(3, KeychainStatus::NotFound, vec![]).result, Err(Error::ItemNotFound));
        assert_eq!(Reply::from_wire(4, KeychainStatus::NotPermitted, vec![]).result, Err(Error::NotPermitted));
    }

    #[test]
    fn io_and_param_statuses_map_and_ok_carries_the_value() {
        assert_eq!(Reply::from_wire(5, KeychainStatus::Param, vec![]).result, Err(Error::Param));
        assert_eq!(Reply::from_wire(6, KeychainStatus::Io, vec![]).result, Err(Error::Io(String::new())));
        assert_eq!(Reply::from_wire(7, KeychainStatus::Ok, vec![]).result, Ok(vec![]));
    }

    #[test]
    fn reply_keeps_the_request_id() {
        assert_eq!(Reply::from_wire(99, KeychainStatus::Ok, vec![1]).request, 99);
        assert_eq!(Reply::from_wire(100, KeychainStatus::NotFound, vec![]).request, 100);
    }

    #[test]
    fn errors_ignore_the_value_payload() {
        assert_eq!(Reply::from_wire(1, KeychainStatus::NotFound, b"leak".to_vec()).result, Err(Error::ItemNotFound));
    }
}
