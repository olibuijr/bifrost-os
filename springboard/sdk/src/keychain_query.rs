//! SecItem query API over the keychain transport: item classes (`kSecClassGenericPassword`,
//! `kSecClassInternetPassword`), attributes (`kSecAttrService`, `kSecAttrAccount`, `kSecAttrServer`,
//! `kSecAttrProtocol`, `kSecAttrPort`, `kSecAttrLabel`, `kSecAttrComment`, `kSecAttrCreationDate`,
//! `kSecAttrModificationDate`, `kSecAttrAccessGroup`), `kSecMatchLimitOne` / `kSecMatchLimitAll` and `OSStatus` values.
//!
//! The shell store keys items by `(service, account)` and keeps one secret per item, so the SDK side maps a query to that
//! wire: an internet password is keyed by `("inet:<server>:<port>:<protocol>", account)`, and the attributes travel in
//! the stored value as a `SIQ1` record (class, attributes, secret). Values without the `SIQ1` prefix are legacy items
//! from [`crate::keychain`]; they decode as generic passwords with only the secret. Listing (`kSecMatchLimitAll`) needs
//! a shell list operation that does not exist yet: [`select`] filters the records once the shell returns them.
//! Calls are asynchronous like [`crate::keychain`]; replies arrive in `App::keychain` and go through [`decode_item`].

use crate::keychain::{self, Error, Reply};
use crate::proto::{KeychainOp, ToShell};
use crate::Ctx;

const MAGIC: &[u8; 4] = b"SIQ1";

/// `kSecClass`
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ItemClass {
    /// kSecClassGenericPassword
    GenericPassword,
    /// kSecClassInternetPassword
    InternetPassword,
}

/// `kSecMatchLimit`
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MatchLimit {
    /// kSecMatchLimitOne
    One,
    /// kSecMatchLimitAll
    All,
}

/// The `kSecAttr*` values of an item or a query; `None` is "not set" (matches anything in a query).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Attributes {
    pub service: Option<String>,
    pub account: Option<String>,
    pub server: Option<String>,
    pub protocol: Option<String>,
    pub port: Option<u16>,
    pub label: Option<String>,
    pub comment: Option<String>,
    /// seconds since the Unix epoch
    pub creation_date: Option<u64>,
    /// seconds since the Unix epoch
    pub modification_date: Option<u64>,
    pub access_group: Option<String>,
}

/// One stored item: class, attributes and secret.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    pub class: ItemClass,
    pub attributes: Attributes,
    pub data: Vec<u8>,
}

/// A `SecItemCopyMatching` / `SecItemUpdate` / `SecItemDelete` query.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Query {
    pub class: ItemClass,
    pub attributes: Attributes,
    pub limit: MatchLimit,
    pub return_data: bool,
    pub return_attributes: bool,
}

impl Query {
    pub fn new(class: ItemClass) -> Query {
        Query { class, attributes: Attributes::default(), limit: MatchLimit::One, return_data: false, return_attributes: false }
    }
}

/// `OSStatus` of a SecItem call.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Status {
    /// errSecSuccess
    Success,
    /// errSecDuplicateItem
    DuplicateItem,
    /// errSecItemNotFound
    ItemNotFound,
    /// errSecNotAvailable
    NotAvailable,
    /// errSecParam
    Param,
    /// errSecIO
    Io,
    /// errSecDecode: the stored value is not a readable record
    Decode,
}

impl Status {
    pub fn code(&self) -> i32 {
        match self {
            Status::Success => 0,
            Status::DuplicateItem => -25299,
            Status::ItemNotFound => -25300,
            Status::NotAvailable => -25291,
            Status::Param => -50,
            Status::Io => -36,
            Status::Decode => -26275,
        }
    }

    pub fn from_error(e: &Error) -> Status {
        match e {
            Error::DuplicateItem => Status::DuplicateItem,
            Error::ItemNotFound => Status::ItemNotFound,
            Error::NotPermitted => Status::NotAvailable,
            Error::Param => Status::Param,
            Error::Io(_) => Status::Io,
        }
    }
}

/// The `(service, account)` the shell store keys this item by, or `None` when the key attributes are missing.
pub fn wire_key(class: ItemClass, a: &Attributes) -> Option<(String, String)> {
    let account = a.account.clone()?;
    let service = match class {
        ItemClass::GenericPassword => a.service.clone()?,
        ItemClass::InternetPassword => format!(
            "inet:{}:{}:{}",
            a.server.as_deref()?,
            a.port.map(|p| p.to_string()).unwrap_or_default(),
            a.protocol.as_deref().unwrap_or("")
        ),
    };
    Some((service, account))
}

fn put(out: &mut Vec<u8>, bytes: &[u8]) {
    out.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
    out.extend_from_slice(bytes);
}

fn put_opt(out: &mut Vec<u8>, v: Option<&str>) {
    match v {
        Some(s) => {
            out.push(1);
            put(out, s.as_bytes());
        }
        None => out.push(0),
    }
}

/// The value stored for `item`: `SIQ1`, class byte, the attributes, then the secret.
pub fn encode(item: &Item) -> Vec<u8> {
    let a = &item.attributes;
    let mut out = MAGIC.to_vec();
    out.push(if item.class == ItemClass::InternetPassword { 1 } else { 0 });
    for s in [&a.service, &a.account, &a.server, &a.protocol, &a.label, &a.comment, &a.access_group] {
        put_opt(&mut out, s.as_deref());
    }
    for n in [a.port.map(|p| p.to_string()), a.creation_date.map(|d| d.to_string()), a.modification_date.map(|d| d.to_string())] {
        put_opt(&mut out, n.as_deref());
    }
    put(&mut out, &item.data);
    out
}

struct Reader<'a>(&'a [u8]);

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        if self.0.len() < n {
            return None;
        }
        let (head, rest) = self.0.split_at(n);
        self.0 = rest;
        Some(head)
    }
    fn bytes(&mut self) -> Option<&'a [u8]> {
        let n = u32::from_be_bytes(self.take(4)?.try_into().ok()?) as usize;
        self.take(n)
    }
    fn opt(&mut self) -> Option<Option<String>> {
        match self.take(1)?[0] {
            0 => Some(None),
            1 => Some(Some(String::from_utf8(self.bytes()?.to_vec()).ok()?)),
            _ => None,
        }
    }
}

/// Reads a stored value. A value without the `SIQ1` prefix is a legacy generic password: only the secret is known.
pub fn decode(value: &[u8]) -> Result<Item, Status> {
    if !value.starts_with(MAGIC) {
        return Ok(Item { class: ItemClass::GenericPassword, attributes: Attributes::default(), data: value.to_vec() });
    }
    parse(&value[MAGIC.len()..]).ok_or(Status::Decode)
}

fn parse(rest: &[u8]) -> Option<Item> {
    let mut r = Reader(rest);
    let class = if r.take(1)?[0] == 1 { ItemClass::InternetPassword } else { ItemClass::GenericPassword };
    let mut a = Attributes {
        service: r.opt()?,
        account: r.opt()?,
        server: r.opt()?,
        protocol: r.opt()?,
        label: r.opt()?,
        comment: r.opt()?,
        access_group: r.opt()?,
        ..Attributes::default()
    };
    a.port = r.opt()?.and_then(|s| s.parse().ok());
    a.creation_date = r.opt()?.and_then(|s| s.parse().ok());
    a.modification_date = r.opt()?.and_then(|s| s.parse().ok());
    let data = r.bytes()?.to_vec();
    Some(Item { class, attributes: a, data })
}

fn want<T: PartialEq>(q: &Option<T>, v: &Option<T>) -> bool {
    q.is_none() || q == v
}

/// Whether `item` satisfies the query's class and set attributes.
pub fn matches(q: &Query, item: &Item) -> bool {
    let (a, b) = (&q.attributes, &item.attributes);
    q.class == item.class
        && want(&a.service, &b.service)
        && want(&a.account, &b.account)
        && want(&a.server, &b.server)
        && want(&a.protocol, &b.protocol)
        && want(&a.port, &b.port)
        && want(&a.label, &b.label)
        && want(&a.comment, &b.comment)
        && want(&a.access_group, &b.access_group)
}

/// The items a `kSecMatchLimitAll` / `kSecMatchLimitOne` query selects from `items`; secrets are dropped unless
/// `return_data`, attributes unless `return_attributes`.
pub fn select(q: &Query, items: &[Item]) -> Vec<Item> {
    let max = if q.limit == MatchLimit::One { 1 } else { usize::MAX };
    items
        .iter()
        .filter(|i| matches(q, i))
        .take(max)
        .map(|i| Item {
            class: i.class,
            attributes: if q.return_attributes { i.attributes.clone() } else { Attributes::default() },
            data: if q.return_data { i.data.clone() } else { Vec::new() },
        })
        .collect()
}

/// Turns a `get` reply into the item, or the `OSStatus` it failed with.
pub fn decode_item(reply: &Reply) -> Result<Item, Status> {
    match &reply.result {
        Ok(v) => decode(v),
        Err(e) => Err(Status::from_error(e)),
    }
}

fn call(ctx: &Ctx, op: KeychainOp) -> u32 {
    let request = ctx.next_request();
    ctx.send_to_shell(ToShell::Keychain { request, op });
    request
}

/// The creation date is `now` unless set; the modification date is `now`.
fn stamped(mut item: Item, now: u64) -> Item {
    item.attributes.creation_date.get_or_insert(now);
    item.attributes.modification_date = Some(now);
    item
}

/// `SecItemAdd`; `now` is seconds since the epoch. Returns `None` (no request sent) when the key attributes are missing.
pub fn add_item(ctx: &Ctx, item: Item, now: u64) -> Option<u32> {
    let (service, account) = wire_key(item.class, &item.attributes)?;
    let value = encode(&stamped(item, now));
    Some(call(ctx, KeychainOp::Add { service, account, value }))
}

/// `SecItemCopyMatching` for one item named by its key attributes; decode the reply with [`decode_item`].
pub fn copy_matching(ctx: &Ctx, q: &Query) -> Option<u32> {
    let (service, account) = wire_key(q.class, &q.attributes)?;
    Some(keychain::get(ctx, &service, &account))
}

/// `SecItemUpdate`: replaces the item named by `q`'s key attributes with `item` (modification date set to `now`).
pub fn update_items(ctx: &Ctx, q: &Query, item: Item, now: u64) -> Option<u32> {
    let (service, account) = wire_key(q.class, &q.attributes)?;
    let value = encode(&stamped(item, now));
    Some(call(ctx, KeychainOp::Update { service, account, value }))
}

/// `SecItemDelete` for the item named by `q`'s key attributes.
pub fn delete_items(ctx: &Ctx, q: &Query) -> Option<u32> {
    let (service, account) = wire_key(q.class, &q.attributes)?;
    Some(keychain::delete(ctx, &service, &account))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &str) -> Option<String> {
        Some(v.to_string())
    }

    fn inet() -> Item {
        Item {
            class: ItemClass::InternetPassword,
            attributes: Attributes {
                server: s("mail.example"), protocol: s("imaps"), port: Some(993), account: s("me"),
                label: s("Mail"), comment: s("work"), creation_date: Some(5), modification_date: Some(6),
                access_group: s("g"), ..Attributes::default()
            },
            data: b"pw".to_vec(),
        }
    }

    fn generic(service: &str, account: &str) -> Item {
        Item {
            class: ItemClass::GenericPassword,
            attributes: Attributes { service: s(service), account: s(account), ..Attributes::default() },
            data: b"x".to_vec(),
        }
    }

    #[test]
    fn generic_key_is_service_and_account_like_the_legacy_calls() {
        let it = generic("svc", "acc");
        assert_eq!(wire_key(it.class, &it.attributes), Some(("svc".into(), "acc".into())));
    }

    #[test]
    fn internet_key_combines_server_port_protocol() {
        let it = inet();
        assert_eq!(wire_key(it.class, &it.attributes), Some(("inet:mail.example:993:imaps".into(), "me".into())));
    }

    #[test]
    fn missing_key_attributes_give_no_key() {
        assert_eq!(wire_key(ItemClass::GenericPassword, &Attributes { account: s("a"), ..Attributes::default() }), None);
        assert_eq!(wire_key(ItemClass::InternetPassword, &Attributes { server: s("h"), ..Attributes::default() }), None);
    }

    #[test]
    fn record_round_trips_with_every_attribute() {
        let it = inet();
        assert_eq!(decode(&encode(&it)), Ok(it));
    }

    #[test]
    fn legacy_value_decodes_as_generic_secret_only() {
        let it = decode(b"plain-secret").unwrap();
        assert_eq!(it.class, ItemClass::GenericPassword);
        assert_eq!(it.attributes, Attributes::default());
        assert_eq!(it.data, b"plain-secret");
    }

    #[test]
    fn truncated_record_is_a_decode_error() {
        let mut v = encode(&inet());
        v.truncate(v.len() - 3);
        assert_eq!(decode(&v), Err(Status::Decode));
    }

    #[test]
    fn match_all_selects_by_class_and_attributes() {
        let items = vec![generic("a", "1"), generic("a", "2"), generic("b", "1"), inet()];
        let mut q = Query::new(ItemClass::GenericPassword);
        q.limit = MatchLimit::All;
        q.attributes.service = s("a");
        q.return_attributes = true;
        let got = select(&q, &items);
        assert_eq!(got.len(), 2);
        assert_eq!(got[1].attributes.account, s("2"));
        assert!(got[0].data.is_empty());
    }

    #[test]
    fn match_one_returns_first_with_data_on_request() {
        let items = vec![generic("a", "1"), generic("a", "2")];
        let mut q = Query::new(ItemClass::GenericPassword);
        q.return_data = true;
        let got = select(&q, &items);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].data, b"x");
        assert_eq!(got[0].attributes, Attributes::default());
    }

    #[test]
    fn status_codes_follow_osstatus() {
        assert_eq!(Status::from_error(&Error::ItemNotFound).code(), -25300);
        assert_eq!(Status::from_error(&Error::DuplicateItem).code(), -25299);
        assert_eq!(Status::Success.code(), 0);
    }

    #[test]
    fn replies_decode_to_items_or_statuses() {
        let ok = Reply { request: 1, result: Ok(encode(&inet())) };
        assert_eq!(decode_item(&ok), Ok(inet()));
        let nf = Reply { request: 2, result: Err(Error::ItemNotFound) };
        assert_eq!(decode_item(&nf), Err(Status::ItemNotFound));
    }

    #[test]
    fn stamping_keeps_creation_and_sets_modification() {
        let it = stamped(inet(), 99);
        assert_eq!(it.attributes.creation_date, Some(5));
        assert_eq!(it.attributes.modification_date, Some(99));
        let fresh = stamped(generic("a", "b"), 7);
        assert_eq!(fresh.attributes.creation_date, Some(7));
    }
}
