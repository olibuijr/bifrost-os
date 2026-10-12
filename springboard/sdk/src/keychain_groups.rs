//! Keychain access groups (`kSecAttrAccessGroup`): secrets shared by the apps that list the same group in
//! `keychain_groups = [...]` in their manifests. A client of the `keychain-groups` shell service
//! (`springboard/src/shell/services/keychain_groups.rs`), over `service::call`.
//!
//! Each call returns a request id. The answer arrives in `App::service_reply` and goes through [`decode`] (or
//! [`decode_groups`] for [`groups`]). When another member changes an item, the app gets the `changed` event in
//! `App::service_event`; [`changed`] reads it. Errors are the keychain's own [`Error`]. The call needs the `keychain`
//! permission and the group in the manifest, or it answers `NotPermitted`. A group must be named on every call:
//! there is no search across groups, unlike the app's own [`keychain`](crate::keychain) items.
//!
//! ```ignore
//! let id = keychain_groups::add(ctx, "is.example.keys", "login", "me", b"secret");
//! // later, in App::service_reply(ctx, request, result):
//! //   keychain_groups::decode(result)  ->  Ok(empty) for add
//! ```

use crate::service;
use crate::Ctx;
use bifrost_core::json::Json;
use springboard::keychain::{hex, unhex};

pub use springboard::keychain::Error;

/// The shell service this client calls.
pub const SERVICE: &str = "keychain-groups";
/// The event a member receives after another member changed an item.
pub const CHANGED: &str = "changed";

/// The `(group, account)` of a `changed` event.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Change {
    pub group: String,
    pub account: String,
}

fn call(ctx: &Ctx, method: &str, params: &Json) -> u32 {
    service::call(ctx, SERVICE, method, params)
}

/// The request body: the group, the item's service and account, and the secret as hex when there is one.
fn params(group: &str, item_service: &str, account: &str, value: Option<&[u8]>) -> Json {
    let mut fields = vec![
        ("group".to_string(), Json::Str(group.to_string())),
        ("service".to_string(), Json::Str(item_service.to_string())),
        ("account".to_string(), Json::Str(account.to_string())),
    ];
    if let Some(v) = value {
        fields.push(("value".to_string(), Json::Str(hex(v))));
    }
    Json::Obj(fields)
}

/// Asks for the caller's own groups. Answer: [`decode_groups`].
pub fn groups(ctx: &Ctx) -> u32 {
    call(ctx, "groups", &Json::Null)
}

/// `SecItemAdd` in `group`: the answer is `DuplicateItem` when the item exists.
pub fn add(ctx: &Ctx, group: &str, item_service: &str, account: &str, secret: &[u8]) -> u32 {
    call(ctx, "add", &params(group, item_service, account, Some(secret)))
}

/// `SecItemCopyMatching` in `group`: the answer carries the secret, or `ItemNotFound`.
pub fn get(ctx: &Ctx, group: &str, item_service: &str, account: &str) -> u32 {
    call(ctx, "get", &params(group, item_service, account, None))
}

/// `SecItemUpdate` in `group`: the answer is `ItemNotFound` when there is no such item.
pub fn update(ctx: &Ctx, group: &str, item_service: &str, account: &str, secret: &[u8]) -> u32 {
    call(ctx, "update", &params(group, item_service, account, Some(secret)))
}

/// `SecItemDelete` in `group`: the answer is `ItemNotFound` when there is no such item.
pub fn delete(ctx: &Ctx, group: &str, item_service: &str, account: &str) -> u32 {
    call(ctx, "delete", &params(group, item_service, account, None))
}

/// Decodes the answer to one call. `Ok` holds the secret for [`get`] and is empty for the others. A refusal of the
/// request itself (the shell's error text, or an answer without a status) is `Error::Param`.
pub fn decode(result: &Result<Json, String>) -> Result<Vec<u8>, Error> {
    let body = result.as_ref().map_err(|_| Error::Param)?;
    let code = body.get("status").and_then(|s| s.number()).ok_or(Error::Param)? as i32;
    if let Some(e) = Error::from_code(code) {
        return Err(e);
    }
    let value = body.get("value").and_then(|v| v.as_str()).ok_or(Error::Param)?;
    unhex(value).ok_or(Error::Param)
}

/// Decodes the answer to [`groups`]; `None` when the answer is an error or not a list.
pub fn decode_groups(result: &Result<Json, String>) -> Option<Vec<String>> {
    let items = result.as_ref().ok()?.as_array()?;
    Some(items.iter().filter_map(|g| g.as_str().map(str::to_string)).collect())
}

/// The change behind an event, when `service` and `event` are this client's `changed` event.
pub fn changed(service: &str, event: &str, body: &Json) -> Option<Change> {
    if service != SERVICE || event != CHANGED {
        return None;
    }
    Some(Change {
        group: body.get("group")?.as_str()?.to_string(),
        account: body.get("account")?.as_str()?.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn answer(status: f64, value: &str) -> Result<Json, String> {
        Ok(Json::Obj(vec![("status".into(), Json::Num(status)), ("value".into(), Json::Str(value.into()))]))
    }

    #[test]
    fn the_request_carries_group_service_account_and_hex_value() {
        let add = params("is.example.keys", "login", "me", Some(&[0x01, 0xab]));
        assert_eq!(add.get("group"), Some(&Json::Str("is.example.keys".into())));
        assert_eq!(add.get("service"), Some(&Json::Str("login".into())));
        assert_eq!(add.get("account"), Some(&Json::Str("me".into())));
        assert_eq!(add.get("value"), Some(&Json::Str("01ab".into())));
        let get = params("g", "s", "a", None);
        assert_eq!(get.get("value"), None, "a get sends no secret");
    }

    #[test]
    fn a_successful_answer_carries_the_secret() {
        assert_eq!(decode(&answer(0.0, "6869")), Ok(b"hi".to_vec()));
        assert_eq!(decode(&answer(0.0, "")), Ok(Vec::new()));
    }

    #[test]
    fn a_status_becomes_the_keychain_error() {
        assert_eq!(decode(&answer(-25300.0, "")), Err(Error::ItemNotFound));
        assert_eq!(decode(&answer(-25299.0, "")), Err(Error::DuplicateItem));
        assert_eq!(decode(&answer(-25291.0, "")), Err(Error::NotPermitted));
    }

    #[test]
    fn a_refused_request_or_a_malformed_answer_is_a_param_error() {
        assert_eq!(decode(&Err("keychain-groups has no method fly".into())), Err(Error::Param));
        assert_eq!(decode(&Ok(Json::Null)), Err(Error::Param));
        assert_eq!(decode(&answer(0.0, "zz")), Err(Error::Param));
    }

    #[test]
    fn the_groups_answer_lists_names() {
        let list = Ok(Json::Arr(vec![Json::Str("is.a".into()), Json::Str("is.b".into())]));
        assert_eq!(decode_groups(&list), Some(vec!["is.a".to_string(), "is.b".to_string()]));
        assert_eq!(decode_groups(&Err("no".into())), None);
    }

    #[test]
    fn a_changed_event_names_the_group_and_account() {
        let body = Json::Obj(vec![("group".into(), Json::Str("is.a".into())), ("account".into(), Json::Str("me".into()))]);
        assert_eq!(changed(SERVICE, CHANGED, &body), Some(Change { group: "is.a".into(), account: "me".into() }));
        assert_eq!(changed("system", CHANGED, &body), None, "another service's event is not ours");
        assert_eq!(changed(SERVICE, "other", &body), None);
    }
}
